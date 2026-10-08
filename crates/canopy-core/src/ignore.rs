//! Ignore rules: adding a rule to a `.gitignore` or exclude file, previewing what a rule would
//! ignore, asking git why a path is ignored, and untracking a path.
//!
//! Rules are written as bytes with LF line endings, skipped when the file already has the same
//! line (compared case-insensitively under `core.ignorecase`), and never through a symbolic link:
//! a cloned repository's `.gitignore` could point at any file of the user's.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::paths::{check_relative, os_path, refuse_link, safe_join, write_atomic};
use crate::{Error, Repo, Result, cli};

/// Small built-in starter rules, added only when explicitly selected during repository creation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IgnoreTemplate {
    #[default]
    None,
    Rust,
    Node,
    Python,
}

impl IgnoreTemplate {
    pub const ALL: [Self; 4] = [Self::None, Self::Rust, Self::Node, Self::Python];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Rust => "Rust",
            Self::Node => "Node / JavaScript",
            Self::Python => "Python",
        }
    }

    pub fn patterns(self) -> &'static [&'static str] {
        match self {
            Self::None => &[],
            Self::Rust => &["/target/", "**/*.rs.bk"],
            Self::Node => &["/node_modules/", "/dist/", "*.tsbuildinfo"],
            Self::Python => &[
                "__pycache__/",
                "*.py[cod]",
                ".venv/",
                ".pytest_cache/",
                ".mypy_cache/",
            ],
        }
    }
}

/// A bounded, non-recursive list of ignored working-tree paths (ignored folders stay collapsed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredPaths {
    pub paths: Vec<String>,
    pub truncated: bool,
}

/// Which file a new ignore rule goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IgnoreTarget {
    /// `.gitignore` at the root of the working tree, shared through commits.
    Root,
    /// The `.gitignore` closest to the path, in its folder or a parent; the root one when no
    /// other exists.
    Nearest,
    /// The repository's own exclude file (`info/exclude` in the Git directory), never committed.
    RepositoryExclude,
    /// The user's global excludes file (`core.excludesFile`, else `$XDG_CONFIG_HOME/git/ignore`
    /// or `~/.config/git/ignore`), which applies to every repository.
    Global,
}

impl IgnoreTarget {
    /// Every target, in menu order.
    pub const ALL: [Self; 4] = [
        Self::Root,
        Self::Nearest,
        Self::RepositoryExclude,
        Self::Global,
    ];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Root => ".gitignore at the root",
            Self::Nearest => "Nearest .gitignore",
            Self::RepositoryExclude => "This repository only (info/exclude)",
            Self::Global => "Every repository (global excludes file)",
        }
    }
}

/// What to ignore. Paths are relative to the working tree, `/`-separated as Git stores them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IgnoreRule {
    /// This one file, anchored: `/path/to/file`.
    File(String),
    /// Every file with the extension of this path, at any depth: `*.ext`.
    Extension(String),
    /// This folder and everything in it, anchored: `/path/to/dir/`.
    Folder(String),
    /// A pattern written by the user, stored as given. `near` is the path it was made for; it
    /// picks the [`IgnoreTarget::Nearest`] file and is the path checked after writing.
    Custom {
        /// The gitignore pattern.
        pattern: String,
        /// The path the user started from.
        near: String,
    },
}

impl IgnoreRule {
    /// The path the rule was made for.
    pub fn path(&self) -> &str {
        match self {
            Self::File(p) | Self::Extension(p) | Self::Folder(p) => p,
            Self::Custom { near, .. } => near,
        }
    }
}

/// The file a rule would be written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreDestination {
    /// The file (it may not exist yet).
    pub file: PathBuf,
    /// The folder its patterns are relative to, relative to the working tree, `/`-separated
    /// without a trailing `/`; empty for the root, the exclude file and the global file.
    pub base: String,
}

/// The rule git applies to a path, from `git check-ignore -v`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreMatch {
    /// The file holding the rule, as git names it (relative to the working tree or absolute).
    pub source: String,
    /// Line number of the rule in that file, from 1.
    pub line: usize,
    /// The pattern as written.
    pub pattern: String,
}

impl IgnoreMatch {
    /// A `!pattern`: the deciding rule re-includes the path, so it is not ignored.
    pub fn negated(&self) -> bool {
        self.pattern.starts_with('!')
    }
}

/// The result of [`Repo::add_ignore_rule`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreOutcome {
    /// The file the rule went into.
    pub file: PathBuf,
    /// The line written (or found).
    pub pattern: String,
    /// `false` when the file already had this rule and nothing was written.
    pub added: bool,
    /// The rule that now decides the rule's path, as [`Repo::why_ignored`] reports it. `None`, or
    /// a [`IgnoreMatch::negated`] match, means the path is still not ignored.
    pub check: Option<IgnoreMatch>,
}

/// Paths whose ignored state a rule would change, from [`Repo::preview_ignore_rule`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IgnorePreview {
    /// Paths that would become ignored.
    pub ignored: Vec<String>,
    /// Paths that would stop being ignored (a `!` pattern).
    pub unignored: Vec<String>,
}

/// Precedence of an ignore file for one path, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    /// The global excludes file.
    Global,
    /// The repository's `info/exclude`.
    Exclude,
    /// A `.gitignore` this many folders below the root (deeper ones are closer to the path).
    Gitignore(usize),
}

/// Folders in `base` (`/`-separated, empty for the root).
fn depth(base: &str) -> usize {
    if base.is_empty() {
        0
    } else {
        base.split('/').count()
    }
}

/// Escapes `text` so a gitignore pattern matches it literally: `\`, `*`, `?`, `[` and `]`
/// anywhere, a leading `#` or `!`, and trailing spaces.
pub fn escape_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    let trailing = text.len() - text.trim_end_matches(' ').len();
    let keep = text.len() - trailing;
    for (i, ch) in text.char_indices() {
        let escape = matches!(ch, '\\' | '*' | '?' | '[' | ']')
            || (i == 0 && matches!(ch, '#' | '!'))
            || (ch == ' ' && i >= keep);
        if escape {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// `path` relative to `base` (both `/`-separated, `base` without a trailing `/`, empty for the
/// root).
fn relative_to<'a>(path: &'a str, base: &str) -> Result<&'a str> {
    if base.is_empty() {
        return Ok(path);
    }
    path.strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| Error::invalid(format!("{path} is not inside {base}.")))
}

fn validate_custom(pattern: &str) -> Result<()> {
    if pattern.trim().is_empty() {
        return Err(Error::invalid("Write a pattern first."));
    }
    if pattern.contains(['\n', '\r', '\0']) {
        return Err(Error::invalid("A pattern is a single line."));
    }
    if pattern.starts_with('#') {
        return Err(Error::invalid(
            "A pattern starting with # is a comment. Write \\# to match a leading #.",
        ));
    }
    Ok(())
}

/// The line to write for `rule` into a file whose patterns are relative to `base`.
fn pattern_for(rule: &IgnoreRule, base: &str) -> Result<String> {
    // A gitignore pattern cannot contain a line break; a file name with one would otherwise add
    // a second, unintended rule.
    if let IgnoreRule::File(path) | IgnoreRule::Folder(path) | IgnoreRule::Extension(path) = rule
        && path.contains(['\n', '\r'])
    {
        return Err(Error::invalid(
            "A name with a line break cannot be ignored by an ignore rule.",
        ));
    }
    match rule {
        IgnoreRule::File(path) => {
            check_relative(path)?;
            Ok(format!("/{}", escape_pattern(relative_to(path, base)?)))
        }
        IgnoreRule::Folder(path) => {
            check_relative(path)?;
            let rel = relative_to(path.trim_end_matches('/'), base)?;
            Ok(format!("/{}/", escape_pattern(rel)))
        }
        IgnoreRule::Extension(path) => {
            check_relative(path)?;
            let name = path.rsplit('/').next().unwrap_or(path);
            match name.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
                    Ok(format!("*.{}", escape_pattern(ext)))
                }
                _ => Err(Error::invalid(format!("{name} has no file extension."))),
            }
        }
        IgnoreRule::Custom { pattern, .. } => {
            validate_custom(pattern)?;
            Ok(pattern.clone())
        }
    }
}

/// `pattern`, written in a `.gitignore` in folder `base`, as the equivalent root-level pattern.
fn root_relative(pattern: &str, base: &str) -> String {
    if base.is_empty() {
        return pattern.to_owned();
    }
    let (neg, body) = match pattern.strip_prefix('!') {
        Some(rest) => ("!", rest),
        None => ("", pattern),
    };
    // A slash anywhere but at the end anchors the pattern to the file's folder.
    if body.trim_end_matches('/').contains('/') {
        format!(
            "{neg}{}/{}",
            escape_pattern(base),
            body.trim_start_matches('/')
        )
    } else {
        format!("{neg}{}/**/{body}", escape_pattern(base))
    }
}

/// `pattern` with its last literal character wrapped in a bracket expression when it is a `!`
/// pattern, which matches the same paths. libgit2 does not apply an in-memory negation that has
/// no wildcard (`!keep.log`) over a matching rule of a `.gitignore`, so the preview would miss
/// the paths such a rule makes visible.
fn with_wildcard(pattern: &str) -> String {
    if !pattern.starts_with('!') {
        return pattern.to_owned();
    }
    let body = pattern.trim_end_matches('/');
    let slashes = pattern.len() - body.len();
    let chars: Vec<char> = body.chars().collect();
    let mut last = None;
    let mut i = 0;
    while i < chars.len() {
        let escaped = chars[i] == '\\';
        last = Some((i + usize::from(escaped), escaped));
        i += if escaped { 2 } else { 1 };
    }
    match last {
        Some((at, false))
            if chars
                .get(at)
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')) =>
        {
            let head: String = chars[..at].iter().collect();
            format!("{head}[{}]{}", chars[at], "/".repeat(slashes))
        }
        _ => pattern.to_owned(),
    }
}

fn trim_newline(mut bytes: &[u8]) -> &[u8] {
    while let [rest @ .., b'\n' | b'\r'] = bytes {
        bytes = rest;
    }
    bytes
}

/// The user's global excludes file when `core.excludesFile` is not set, as git finds it.
fn default_global_excludes() -> Result<PathBuf> {
    let user_profile = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        None
    };
    global_excludes_in(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
        user_profile,
    )
    .ok_or_else(|| Error::invalid("No home folder is set, so there is no global excludes file."))
}

/// `$XDG_CONFIG_HOME/git/ignore`, else `<home>/.config/git/ignore` with `HOME` (or, where given,
/// `USERPROFILE`). Empty values count as unset.
fn global_excludes_in(
    xdg: Option<OsString>,
    home: Option<OsString>,
    user_profile: Option<OsString>,
) -> Option<PathBuf> {
    let set = |v: Option<OsString>| v.filter(|v| !v.is_empty());
    if let Some(xdg) = set(xdg) {
        return Some(PathBuf::from(xdg).join("git").join("ignore"));
    }
    let home = set(home).or_else(|| set(user_profile))?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("git")
            .join("ignore"),
    )
}

/// Appends `line` to the bytes of an ignore file unless an equal line is already there. Returns
/// `None` when it is.
fn append_line(existing: &[u8], line: &str, ignore_case: bool) -> Option<Vec<u8>> {
    let same = |l: &[u8]| {
        let l = l.strip_suffix(b"\r").unwrap_or(l);
        if ignore_case {
            l.eq_ignore_ascii_case(line.as_bytes())
        } else {
            l == line.as_bytes()
        }
    };
    if existing.split(|b| *b == b'\n').any(same) {
        return None;
    }
    let mut out = existing.to_vec();
    if !out.is_empty() && !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    out.extend_from_slice(line.as_bytes());
    out.push(b'\n');
    Some(out)
}

impl Repo {
    /// Ignored paths for the management UI, without opening ignored directories or downloading.
    pub fn ignored_paths(&self, limit: usize) -> Result<IgnoredPaths> {
        let mut options = git2::StatusOptions::new();
        options
            .include_ignored(true)
            .include_untracked(false)
            .recurse_ignored_dirs(false)
            .recurse_untracked_dirs(true)
            .show(git2::StatusShow::Workdir)
            .renames_head_to_index(false)
            .renames_index_to_workdir(false);
        let statuses = self.git().statuses(Some(&mut options))?;
        let mut paths = statuses
            .iter()
            .filter(|entry| entry.status().contains(git2::Status::IGNORED))
            .map(|entry| String::from_utf8_lossy(entry.path_bytes()).into_owned())
            .take(limit.saturating_add(1))
            .collect::<Vec<_>>();
        paths.sort();
        let truncated = paths.len() > limit;
        paths.truncate(limit);
        Ok(IgnoredPaths { paths, truncated })
    }

    /// Appends a starter template atomically, retaining existing rules and refusing symlinks.
    /// Returns whether the file changed; None does not create a file.
    pub fn add_ignore_template(&self, template: IgnoreTemplate) -> Result<bool> {
        if template == IgnoreTemplate::None {
            return Ok(false);
        }
        let file = self
            .ignore_destination(IgnoreTarget::Root, ".gitignore")?
            .file;
        refuse_link(&file)?;
        let mut bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let mut changed = false;
        for pattern in template.patterns() {
            if let Some(next) = append_line(&bytes, pattern, false) {
                bytes = next;
                changed = true;
            }
        }
        if changed {
            write_atomic(&file, &bytes)?;
        }
        Ok(changed)
    }

    /// Where a rule for `path` aimed at `target` would be written, and the folder its patterns
    /// are relative to.
    pub fn ignore_destination(
        &self,
        target: IgnoreTarget,
        path: &str,
    ) -> Result<IgnoreDestination> {
        match target {
            IgnoreTarget::Root => Ok(IgnoreDestination {
                file: safe_join(self.workdir(), ".gitignore")?,
                base: String::new(),
            }),
            IgnoreTarget::Nearest => {
                check_relative(path)?;
                let mut dir = path.trim_end_matches('/');
                while let Some((parent, _)) = dir.rsplit_once('/') {
                    dir = parent;
                    let file = safe_join(self.workdir(), &format!("{dir}/.gitignore"))?;
                    if file.is_file() {
                        return Ok(IgnoreDestination {
                            file,
                            base: dir.to_owned(),
                        });
                    }
                }
                self.ignore_destination(IgnoreTarget::Root, path)
            }
            IgnoreTarget::RepositoryExclude => {
                let out = cli::run(self.workdir(), &["rev-parse", "--git-path", "info/exclude"])?;
                let file = os_path(trim_newline(out.as_bytes()));
                let file = if file.is_relative() {
                    self.workdir().join(file)
                } else {
                    file
                };
                Ok(IgnoreDestination {
                    file,
                    base: String::new(),
                })
            }
            IgnoreTarget::Global => {
                let (code, out) = cli::run_answer(
                    self.workdir(),
                    &["config", "--path", "--get", "core.excludesFile"],
                    b"",
                    &[0, 1],
                )?;
                let file = match trim_newline(&out) {
                    value if code == 0 && !value.is_empty() => os_path(value),
                    _ => default_global_excludes()?,
                };
                // git reads a relative value from the directory it runs in.
                let file = if file.is_relative() {
                    self.workdir().join(file)
                } else {
                    file
                };
                Ok(IgnoreDestination {
                    file,
                    base: String::new(),
                })
            }
        }
    }

    /// Adds `rule` to the file `target` names (see [`Self::ignore_destination`]), creating it if
    /// needed: one LF-terminated line, skipped when the file already has it. Refuses a file that
    /// is a symbolic link, and a working-tree `.gitignore` reached through one. Afterwards asks git
    /// which rule decides the rule's path ([`IgnoreOutcome::check`]); a later `!` rule or a
    /// closer `.gitignore` can still win.
    pub fn add_ignore_rule(
        &self,
        rule: &IgnoreRule,
        target: IgnoreTarget,
    ) -> Result<IgnoreOutcome> {
        check_relative(rule.path())?;
        let dest = self.ignore_destination(target, rule.path())?;
        let pattern = pattern_for(rule, &dest.base)?;
        refuse_link(&dest.file)?;
        let existing = match std::fs::read(&dest.file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let ignore_case = self
            .git()
            .config()
            .and_then(|c| c.get_bool("core.ignorecase"))
            .unwrap_or(false);
        let added = match append_line(&existing, &pattern, ignore_case) {
            Some(bytes) => {
                write_atomic(&dest.file, &bytes)?;
                true
            }
            None => false,
        };
        let check = self.why_ignored(rule.path())?;
        Ok(IgnoreOutcome {
            file: dest.file,
            pattern,
            added,
            check,
        })
    }

    /// Which of `paths` adding `rule` to `target` would ignore or un-ignore, without writing
    /// anything. Tracked paths are tested too (git keeps tracking them; see
    /// [`Self::stop_tracking`]).
    ///
    /// Each path's current deciding rule comes from git (as in [`Self::why_ignored`]). Whether the
    /// new pattern matches it is tested with libgit2's in-memory rules on a separate handle, so
    /// this repository's state is untouched. The new rule only takes effect where its file
    /// outranks the current rule's file, as in git: a closer `.gitignore` beats a parent one,
    /// every `.gitignore` beats the exclude file, which beats the global file, and within one
    /// file the appended (last) line wins.
    pub fn preview_ignore_rule(
        &self,
        rule: &IgnoreRule,
        target: IgnoreTarget,
        paths: &[String],
    ) -> Result<IgnorePreview> {
        check_relative(rule.path())?;
        let dest = self.ignore_destination(target, rule.path())?;
        let pattern = pattern_for(rule, &dest.base)?;
        let exclude = self
            .ignore_destination(IgnoreTarget::RepositoryExclude, rule.path())?
            .file;
        let dest_rank = match target {
            IgnoreTarget::Root | IgnoreTarget::Nearest => Rank::Gitignore(depth(&dest.base)),
            IgnoreTarget::RepositoryExclude => Rank::Exclude,
            IgnoreTarget::Global => Rank::Global,
        };
        let deciding = self.deciding_rules(paths)?;

        let scratch = git2::Repository::open(self.git().path())?;
        let without: Vec<bool> = paths
            .iter()
            .map(|p| scratch.is_path_ignored(p))
            .collect::<std::result::Result<_, _>>()?;
        scratch.add_ignore_rule(&with_wildcard(&root_relative(&pattern, &dest.base)))?;
        let mut preview = IgnorePreview::default();
        for ((path, rule_now), was_without) in paths.iter().zip(deciding).zip(without) {
            let with = scratch.is_path_ignored(path)?;
            if with == was_without {
                continue; // The new pattern does not decide this path.
            }
            let was = rule_now.as_ref().is_some_and(|m| !m.negated());
            let outranked = rule_now
                .as_ref()
                .is_some_and(|m| self.rank_of(&m.source, &exclude) > dest_rank);
            if outranked || with == was {
                continue;
            }
            if with {
                preview.ignored.push(path.clone());
            } else {
                preview.unignored.push(path.clone());
            }
        }
        Ok(preview)
    }

    /// How strongly the rule file `source` (as `git check-ignore -v` names it) wins.
    fn rank_of(&self, source: &str, exclude: &Path) -> Rank {
        let p = Path::new(source);
        if p.is_relative()
            && p.file_name().is_some_and(|n| n == ".gitignore")
            && check_relative(source).is_ok()
        {
            return Rank::Gitignore(p.components().count() - 1);
        }
        if self.workdir().join(p) == exclude {
            Rank::Exclude
        } else {
            Rank::Global
        }
    }

    /// The rule deciding each of `paths` (`git check-ignore -v -n --no-index`, one process).
    fn deciding_rules(&self, paths: &[String]) -> Result<Vec<Option<IgnoreMatch>>> {
        let mut input = Vec::new();
        for path in paths {
            check_relative(path)?;
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let (_, out) = cli::run_answer(
            self.workdir(),
            &["check-ignore", "-v", "-n", "--no-index", "-z", "--stdin"],
            &input,
            &[0, 1],
        )?;
        let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
        // Four NUL-terminated fields per path; the trailing NUL leaves one empty piece over.
        let (records, _) = fields.as_chunks::<4>();
        if records.len() != paths.len() {
            return Err(Error::invalid(
                "git check-ignore gave an answer Canopy cannot read.",
            ));
        }
        Ok(records
            .iter()
            .map(|r| {
                let text = |f: &[u8]| String::from_utf8_lossy(f).into_owned();
                (!r[0].is_empty()).then(|| IgnoreMatch {
                    source: text(r[0]),
                    line: text(r[1]).parse().unwrap_or(0),
                    pattern: text(r[2]),
                })
            })
            .collect())
    }

    /// The rule that decides whether `path` is ignored, tracked or not
    /// (`git check-ignore -v --no-index`); `None` when no rule matches. A
    /// [`IgnoreMatch::negated`] match means the path is not ignored.
    pub fn why_ignored(&self, path: &str) -> Result<Option<IgnoreMatch>> {
        let mut rules = self.deciding_rules(&[path.to_owned()])?;
        Ok(rules.pop().flatten())
    }

    /// Removes `path` (a file or folder) from the index but keeps it in the working tree
    /// (`git rm --cached -r`), so an ignore rule can take effect. Collaborators lose the file
    /// from their working trees when they pull the commit that removes it.
    pub fn stop_tracking(&self, path: &str) -> Result<()> {
        check_relative(path)?;
        let path = path.trim_end_matches('/');
        self.git_cmd(&[
            "--literal-pathspecs",
            "rm",
            "--cached",
            "-r",
            "-q",
            "--",
            path,
        ])
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_special_characters() {
        assert_eq!(escape_pattern("plain.txt"), "plain.txt");
        assert_eq!(escape_pattern("a*b?[c]\\d"), "a\\*b\\?\\[c\\]\\\\d");
        assert_eq!(escape_pattern("#hash"), "\\#hash");
        assert_eq!(escape_pattern("!bang"), "\\!bang");
        assert_eq!(escape_pattern("mid#!"), "mid#!");
        assert_eq!(escape_pattern("in side  "), "in side\\ \\ ");
    }

    #[test]
    fn patterns_for_each_rule() {
        let file = IgnoreRule::File("dir/a b.txt".into());
        assert_eq!(pattern_for(&file, "").unwrap(), "/dir/a b.txt");
        assert_eq!(pattern_for(&file, "dir").unwrap(), "/a b.txt");
        pattern_for(&file, "other").unwrap_err();
        let folder = IgnoreRule::Folder("dir/sub/".into());
        assert_eq!(pattern_for(&folder, "").unwrap(), "/dir/sub/");
        assert_eq!(pattern_for(&folder, "dir").unwrap(), "/sub/");
        let ext = IgnoreRule::Extension("x/debug.log".into());
        assert_eq!(pattern_for(&ext, "").unwrap(), "*.log");
        pattern_for(&IgnoreRule::Extension(".bashrc".into()), "").unwrap_err();
        pattern_for(&IgnoreRule::Extension("Makefile".into()), "").unwrap_err();
        let custom = |p: &str| IgnoreRule::Custom {
            pattern: p.into(),
            near: "a".into(),
        };
        assert_eq!(pattern_for(&custom("build/"), "x").unwrap(), "build/");
        for bad in ["", "  ", "a\nb", "a\rb", "# comment"] {
            assert!(pattern_for(&custom(bad), "").is_err(), "{bad:?}");
        }
        pattern_for(&IgnoreRule::File("../x".into()), "").unwrap_err();
        for name in ["a\n*", "a\r\n!b", "dir\n/"] {
            assert!(
                pattern_for(&IgnoreRule::File(name.into()), "").is_err(),
                "{name:?}"
            );
            assert!(
                pattern_for(&IgnoreRule::Folder(name.into()), "").is_err(),
                "{name:?}"
            );
        }
        pattern_for(&IgnoreRule::Extension("x.l\nog".into()), "").unwrap_err();
    }

    #[test]
    fn root_relative_patterns() {
        assert_eq!(root_relative("*.log", ""), "*.log");
        assert_eq!(root_relative("*.log", "sub"), "sub/**/*.log");
        assert_eq!(root_relative("/a.txt", "sub"), "sub/a.txt");
        assert_eq!(root_relative("build/", "sub"), "sub/**/build/");
        assert_eq!(root_relative("x/y", "sub"), "sub/x/y");
        assert_eq!(root_relative("!keep.log", "sub"), "!sub/**/keep.log");
        assert_eq!(root_relative("*.log", "app[1]"), "app\\[1\\]/**/*.log");
    }

    #[test]
    fn negations_get_a_wildcard_for_the_preview() {
        assert_eq!(with_wildcard("!keep.log"), "!keep.lo[g]");
        assert_eq!(with_wildcard("!/sub/keep.log"), "!/sub/keep.lo[g]");
        assert_eq!(with_wildcard("!build/"), "!buil[d]/");
        assert_eq!(with_wildcard("!*.log"), "!*.lo[g]");
        assert_eq!(with_wildcard("!a\\*"), "!a\\*");
        assert_eq!(with_wildcard("keep.log"), "keep.log");
        assert_eq!(with_wildcard("!"), "!");
    }

    #[test]
    fn global_excludes_default_follows_git() {
        let os = |s: &str| Some(OsString::from(s));
        let at = |parts: &[&str]| parts.iter().collect::<PathBuf>();
        assert_eq!(
            global_excludes_in(os("/x"), os("/h"), None),
            Some(at(&["/x", "git", "ignore"]))
        );
        assert_eq!(
            global_excludes_in(os(""), os("/h"), None),
            Some(at(&["/h", ".config", "git", "ignore"]))
        );
        assert_eq!(
            global_excludes_in(None, None, os("/u")),
            Some(at(&["/u", ".config", "git", "ignore"]))
        );
        assert_eq!(global_excludes_in(None, os(""), None), None);
    }

    #[test]
    fn appends_with_lf_and_skips_duplicates() {
        assert_eq!(append_line(b"", "a", false).unwrap(), b"a\n");
        assert_eq!(append_line(b"x", "a", false).unwrap(), b"x\na\n");
        assert_eq!(append_line(b"x\r\n", "a", false).unwrap(), b"x\r\na\n");
        assert_eq!(append_line(b"x\r\na\r\n", "a", false), None);
        assert_eq!(append_line(b"A\n", "a", true), None);
        assert!(append_line(b"A\n", "a", false).is_some());
        assert!(append_line(b"\xff\n", "a", false).is_some());
    }
}
