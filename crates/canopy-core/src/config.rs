//! Git configuration values with their scope and origin, read without quoting file names.
//!
//! File paths retain their original bytes; the display origin is separate and always absolute.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Error, Result, cli, identity::Scope, paths::os_path};

/// Git settings Canopy can edit; executable programs and credentials are never accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Setting {
    PullRebase,
    PullFastForward,
    PullAutostash,
    FetchPrune,
    FetchPruneTags,
    RebaseAutostash,
    MergeAutostash,
    PushFollowTags,
    SubmoduleRecurse,
    PushDefault,
    ConflictStyle,
    InitialBranch,
    CommitCleanup,
    Autocrlf,
    CommitSigning,
    SigningFormat,
}

impl Setting {
    pub const ALL: [Self; 16] = [
        Self::PullRebase,
        Self::PullFastForward,
        Self::PullAutostash,
        Self::FetchPrune,
        Self::FetchPruneTags,
        Self::RebaseAutostash,
        Self::MergeAutostash,
        Self::PushFollowTags,
        Self::SubmoduleRecurse,
        Self::PushDefault,
        Self::ConflictStyle,
        Self::InitialBranch,
        Self::CommitCleanup,
        Self::Autocrlf,
        Self::CommitSigning,
        Self::SigningFormat,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::PullRebase => "pull.rebase",
            Self::PullFastForward => "pull.ff",
            Self::PullAutostash => "pull.autoStash",
            Self::FetchPrune => "fetch.prune",
            Self::FetchPruneTags => "fetch.pruneTags",
            Self::RebaseAutostash => "rebase.autoStash",
            Self::MergeAutostash => "merge.autoStash",
            Self::PushFollowTags => "push.followTags",
            Self::SubmoduleRecurse => "submodule.recurse",
            Self::PushDefault => "push.default",
            Self::ConflictStyle => "merge.conflictStyle",
            Self::InitialBranch => "init.defaultBranch",
            Self::CommitCleanup => "commit.cleanup",
            Self::Autocrlf => "core.autocrlf",
            Self::CommitSigning => "commit.gpgsign",
            Self::SigningFormat => "gpg.format",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PullRebase => "Rebase when pulling",
            Self::PullFastForward => "Fast-forward when pulling",
            Self::PullAutostash => "Autostash for all pulls",
            Self::FetchPrune => "Prune remote branches",
            Self::FetchPruneTags => "Prune local tags",
            Self::RebaseAutostash => "Autostash before rebase",
            Self::MergeAutostash => "Autostash before merge",
            Self::PushFollowTags => "Push annotated tags",
            Self::SubmoduleRecurse => "Recurse into submodules",
            Self::PushDefault => "Push destination",
            Self::ConflictStyle => "Conflict markers",
            Self::InitialBranch => "First branch name",
            Self::CommitCleanup => "Commit message cleanup",
            Self::Autocrlf => "Line ending conversion",
            Self::CommitSigning => "Sign commits",
            Self::SigningFormat => "Signing format",
        }
    }

    /// Canonical editor choices, the only values Canopy writes; an initial branch accepts a
    /// validated branch name instead. `push.default` lists the three that send one branch, the
    /// ones Canopy's own push understands; see [`Setting::shown_only`].
    pub fn values(self) -> &'static [&'static str] {
        match self {
            Self::PullRebase => &["false", "true", "merges", "interactive"],
            Self::PullFastForward => &["only", "true", "false"],
            Self::PushDefault => &["simple", "upstream", "current"],
            Self::ConflictStyle => &["zdiff3", "diff3", "merge"],
            Self::InitialBranch => &[],
            Self::CommitCleanup => &["whitespace", "strip", "verbatim", "scissors", "default"],
            Self::Autocrlf => &["false", "input", "true"],
            Self::SigningFormat => &["openpgp", "ssh", "x509"],
            _ => &["true", "false"],
        }
    }

    /// Values git accepts that the editor shows where the user's own configuration holds them but
    /// never offers or writes. `push.default = matching` (git's default before 2.0) pushes every
    /// branch that has a same-named branch on the remote, so one click would publish many
    /// branches; `nothing` refuses every push that names no branch. Canopy's own push
    /// ([`PushDefault`](crate::PushDefault)) knows neither, so it never chooses them.
    pub fn shown_only(self) -> &'static [&'static str] {
        match self {
            Self::PushDefault => &["matching", "nothing"],
            _ => &[],
        }
    }

    /// The recommended value of every setting, each with its source in ADR 0012 ("Audit of the
    /// recommendations"). Line endings follow GitHub's guidance (convert on Windows, only on
    /// commit elsewhere), signing stays off until the user sets up a key, and an SSH key signs
    /// once it is on. `pull.ff = only` makes a diverged pull stop, whatever `pull.rebase` says
    /// since git 2.34, and `pull.rebase = false` keeps older git merging instead of rebasing;
    /// Canopy then asks Rebase (recommended) or Merge. Commit cleanup is git's own `default`: a
    /// message written in an editor loses its `#` lines, Canopy's own commits (never edited) keep
    /// them. `submodule.recurse` stays off: a recursive switch drops uncommitted edits inside a
    /// submodule, and Canopy updates submodules itself, keeping them.
    pub fn recommendation(self) -> &'static str {
        self.recommendation_for(cli::version_number())
    }

    /// What Use recommended writes: every recommendation except line endings and signing, which
    /// stay as the user chose them, and `submodule.recurse`, which it removes where the global
    /// file turns it on ([`apply_recommended`]): a written `false` would decide that Canopy's own
    /// Update submodules does nothing while it follows the git configuration.
    pub fn recommended_write(self) -> Option<&'static str> {
        match self {
            Self::Autocrlf | Self::CommitSigning | Self::SigningFormat | Self::SubmoduleRecurse => {
                None
            }
            _ => Some(self.recommendation()),
        }
    }

    /// Whether the installed git (`git`, from the worker; `None` when unknown) reads this key at
    /// all: git before 2.51 ignores `pull.autoStash`.
    #[must_use]
    pub fn read_by(self, git: Option<(u32, u32)>) -> bool {
        self != Self::PullAutostash || git.is_some_and(|v| v >= cli::PULL_AUTOSTASH_SINCE)
    }

    /// Pure metadata for a UI that already received the installed Git version from its worker.
    pub fn recommendation_for(self, git: Option<(u32, u32)>) -> &'static str {
        match self {
            Self::FetchPruneTags
            | Self::PullRebase
            | Self::CommitSigning
            | Self::SubmoduleRecurse => "false",
            Self::PullFastForward => "only",
            Self::PullAutostash
            | Self::FetchPrune
            | Self::RebaseAutostash
            | Self::MergeAutostash
            | Self::PushFollowTags => "true",
            Self::PushDefault => "simple",
            Self::ConflictStyle => crate::ConflictStyle::default().value(git),
            Self::InitialBranch => "main",
            Self::CommitCleanup => "default",
            Self::Autocrlf => {
                if cfg!(windows) {
                    "true"
                } else {
                    "input"
                }
            }
            Self::SigningFormat if git.is_some_and(|v| v >= cli::SSH_SIGNING_SINCE) => "ssh",
            Self::SigningFormat => "openpgp",
        }
    }
}

/// Set or remove one supported setting in the selected direct file, leaving included files intact.
/// Blocking: callers with a UI must run this off its thread.
pub fn set(dir: &Path, scope: Scope, setting: Setting, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        if setting == Setting::InitialBranch {
            validate_branch(dir, value)?;
        } else if setting.shown_only().contains(&value) {
            return Err(Error::invalid(format!(
                "Canopy does not write {} = {value}, though git accepts it.",
                setting.key()
            )));
        } else if !setting.values().contains(&value) {
            return Err(Error::invalid(format!(
                "Invalid value for {}.",
                setting.key()
            )));
        }
        if setting == Setting::ConflictStyle
            && value == "zdiff3"
            && cli::version_number().is_none_or(|version| version < cli::ZDIFF3_SINCE)
        {
            return Err(Error::invalid(
                "This Git version needs diff3 instead of zdiff3.",
            ));
        }
    }
    let mut args = vec!["config", scope.flag(), "--no-includes"];
    args.push(if value.is_some() {
        "--replace-all"
    } else {
        "--unset-all"
    });
    args.extend(["--", setting.key()]);
    args.extend(value);
    let out = cli::output_within(cli::command(Some(dir)), &args, WRITE_LIMIT)?;
    if out.code == Some(0) || (value.is_none() && out.code == Some(cli::CONFIG_KEY_ABSENT)) {
        Ok(())
    } else {
        Err(out.error(&args))
    }
}

/// Validate a literal branch name without expanding checkout history or upstream syntax.
pub(crate) fn validate_branch(dir: &Path, branch: &str) -> Result<()> {
    let name = format!("refs/heads/{branch}");
    // check-ref-format has no -- separator; the refs/heads/ prefix keeps names out of options.
    if branch.starts_with('-')
        || branch == "HEAD"
        || cli::run_unchecked(dir, &["check-ref-format", &name])?
            .code
            .is_none_or(|code| code != 0)
    {
        return Err(Error::invalid(format!(
            "'{branch}' is not a valid branch name"
        )));
    }
    Ok(())
}

/// Write the recommended non-sensitive choices globally; repository overrides stay in place.
///
/// A `submodule.recurse` the main global file turns on is removed. A failed write reports how
/// many preceding settings were saved.
pub fn apply_recommended(dir: &Path) -> Result<()> {
    let recurse = Setting::SubmoduleRecurse.key();
    let direct = read_scoped(dir, Some(Scope::Global), false, Query::Get(recurse), true)
        .map_err(|err| Error::invalid(format!("Could not read {recurse}: {err}")))?;
    if direct
        .iter()
        .any(|entry| entry.value.as_deref() == Some("true"))
    {
        set(dir, Scope::Global, Setting::SubmoduleRecurse, None)
            .map_err(|err| Error::invalid(format!("Could not remove {recurse}: {err}")))?;
    }
    for (saved, (setting, value)) in Setting::ALL
        .into_iter()
        .filter_map(|setting| setting.recommended_write().map(|value| (setting, value)))
        .enumerate()
    {
        if let Err(err) = set(dir, Scope::Global, setting, Some(value)) {
            return Err(Error::invalid(format!(
                "Saved {saved} recommended Git settings before {} failed: {err}",
                setting.key(),
            )));
        }
    }
    for setting in Setting::ALL {
        let Some(expected) = setting.recommended_write() else {
            continue;
        };
        let query = Query::Get(setting.key());
        let raw = read(dir, true, query)
            .map_err(|err| {
                Error::invalid(format!(
                    "Saved recommended Git settings, but could not verify {}: {err}",
                    setting.key(),
                ))
            })?
            .pop();
        let canonical = if setting.values() == ["true", "false"] || setting == Setting::PullRebase {
            let bool_value = read_bool(dir, true, setting.key());
            if setting == Setting::PullRebase && bool_value.is_err() {
                // pull.rebase also takes merges/interactive, which are not booleans.
                raw.as_ref().and_then(|entry| entry.value.clone())
            } else {
                bool_value
                    .map_err(|err| {
                        Error::invalid(format!(
                            "Saved recommended Git settings, but could not verify {}: {err}",
                            setting.key(),
                        ))
                    })?
                    .and_then(|entry| entry.value)
            }
        } else {
            raw.as_ref().and_then(|entry| entry.value.clone())
        };
        if canonical.as_deref() != Some(expected) {
            let origin = raw
                .as_ref()
                .map_or("unknown configuration", |entry| entry.origin.as_str());
            return Err(Error::invalid(format!(
                "Saved recommended Git settings, but {} from {origin} still overrides the recommendation. Included files were left unchanged.",
                setting.key(),
            )));
        }
    }
    Ok(())
}

/// Which values to ask Git for, in its configuration reading order.
#[derive(Debug, Clone, Copy)]
pub enum Query<'a> {
    /// The effective value of one key.
    Get(&'a str),
    /// Every key matching a Git regular expression, including repeated values.
    Matching(&'a str),
}

/// One configuration entry and where Git read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    /// `None` for an implicit value in a matching query; `Some("")` for an explicit empty value.
    /// A single-key query cannot distinguish them: both are returned as `Some("")`.
    pub value: Option<String>,
    /// Empty on Git versions without `--show-scope`.
    pub scope: String,
    /// Display text; file origins are absolute and unquoted.
    pub origin: String,
    /// The actual file path, preserving non-UTF-8 names.
    pub file: Option<PathBuf>,
}

/// One read must not hold up the background configuration reader indefinitely.
const READ_LIMIT: Duration = Duration::from_secs(15);
/// Allow atomic Git config writes to finish before timing out.
const WRITE_LIMIT: Duration = Duration::from_secs(60);
/// Scope, origin and entry in current Git's NUL-delimited output.
const SCOPED_FIELDS: usize = 3;
/// Origin and entry before Git supported `--show-scope`.
const ORIGIN_FIELDS: usize = 2;

/// Read repository configuration, or only the global configuration when `global` is true.
/// Blocking: callers with a UI must run this off its thread.
pub fn read(dir: &Path, global: bool, query: Query<'_>) -> Result<Vec<Entry>> {
    read_scoped(dir, global.then_some(Scope::Global), true, query, false)
}

/// Read a Git boolean with its canonical value, scope and origin; invalid values are errors.
pub fn read_bool(dir: &Path, global: bool, key: &str) -> Result<Option<Entry>> {
    Ok(read_scoped(
        dir,
        global.then_some(Scope::Global),
        true,
        Query::Get(key),
        true,
    )?
    .pop())
}

/// Read only entries written directly at a selected editable scope, excluding included files.
pub fn read_direct(dir: &Path, scope: Scope, query: Query<'_>) -> Result<Vec<Entry>> {
    read_scoped(dir, Some(scope), false, query, false)
}

fn read_scoped(
    dir: &Path,
    scope: Option<Scope>,
    includes: bool,
    query: Query<'_>,
    boolean: bool,
) -> Result<Vec<Entry>> {
    let mut args = vec![
        "config",
        "-z",
        if includes {
            "--includes"
        } else {
            "--no-includes"
        },
    ];
    if let Some(scope) = scope {
        args.push(scope.flag());
    }
    if boolean {
        args.push("--type=bool");
    }
    args.extend(["--show-origin", "--show-scope"]);
    match query {
        Query::Get(key) => args.extend(["--get", "--", key]),
        Query::Matching(pattern) => args.extend(["--get-regexp", "--", pattern]),
    }
    let mut out = cli::output_within(cli::command(Some(dir)), &args, READ_LIMIT)?;
    let with_scope = if !matches!(out.code, Some(0 | 1))
        && String::from_utf8_lossy(&out.stderr).contains("show-scope")
    {
        args.retain(|arg| *arg != "--show-scope");
        out = cli::output_within(cli::command(Some(dir)), &args, READ_LIMIT)?;
        false
    } else {
        true
    };
    match out.code {
        Some(1) => Ok(Vec::new()),
        Some(0) => {
            // Git prints repository-local file origins relative to the worktree root even when
            // invoked from a subdirectory. Global-only reads use the supplied working directory.
            let root = if scope == Some(Scope::Global) {
                None
            } else {
                git2::Repository::discover(dir)
                    .ok()
                    .and_then(|repo| repo.workdir().map(Path::to_path_buf))
            };
            let root = root.unwrap_or(std::path::absolute(dir)?);
            Ok(parse(&out.stdout, query, with_scope, &root))
        }
        _ => Err(out.error(&args)),
    }
}

/// Parse `git config -z --show-origin [--show-scope]` output for `query`.
/// `root` is the absolute worktree root, or the directory used for a global-only read.
pub fn parse(out: &[u8], query: Query<'_>, with_scope: bool, root: &Path) -> Vec<Entry> {
    let Some(out) = out.strip_suffix(&[0]) else {
        return Vec::new();
    };
    let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    let width = if with_scope {
        SCOPED_FIELDS
    } else {
        ORIGIN_FIELDS
    };
    fields
        .chunks_exact(width)
        .map(|record| {
            let (scope, rest) = if with_scope {
                (record[0], &record[1..])
            } else {
                (b"".as_slice(), record)
            };
            let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
            let (key, value) = match query {
                Query::Get(key) => (key.to_owned(), Some(text(rest[1]))),
                Query::Matching(_) => match rest[1].iter().position(|&b| b == b'\n') {
                    Some(at) => (text(&rest[1][..at]), Some(text(&rest[1][at + 1..]))),
                    None => (text(rest[1]), None),
                },
            };
            let file = rest[0].strip_prefix(b"file:").map(|bytes| {
                let path = os_path(bytes);
                if path.is_relative() {
                    root.join(path)
                } else {
                    path
                }
            });
            let origin = file
                .as_ref()
                .map_or_else(|| text(rest[0]), |path| format!("file:{}", path.display()));
            Entry {
                key,
                value,
                scope: text(scope),
                origin,
                file,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_values_keep_order_empty_implicit_and_multiline_values() {
        let entries = parse(
            b"local\0file:.git/config\0a.b\0local\0file:.git/config\0a.b\n\0command\0command line:\0a.b\nfirst\nsecond\0",
            Query::Matching("a.b"), true, Path::new("/repo"),
        );
        assert_eq!(
            entries
                .iter()
                .map(|e| e.value.as_deref())
                .collect::<Vec<_>>(),
            [None, Some(""), Some("first\nsecond")]
        );
        assert_eq!(
            entries[0].origin,
            format!("file:{}", Path::new("/repo").join(".git/config").display())
        );
        assert_eq!(entries[2].file, None);
        assert_eq!(entries[2].origin, "command line:");
        assert_eq!(
            parse(b"", Query::Get("a.b"), true, Path::new("/repo")),
            Vec::<Entry>::new()
        );
    }

    #[test]
    fn a_single_value_without_scope_keeps_an_empty_value() {
        let entries = parse(
            b"file:.git/config\0\0",
            Query::Get("a.b"),
            false,
            Path::new("/repo"),
        );
        assert_eq!(entries[0].scope, "");
        assert_eq!(entries[0].value.as_deref(), Some(""));
    }

    #[test]
    fn reading_from_a_subdirectory_uses_the_repository_root() {
        let root = tempfile::tempdir().expect("tempdir");
        cli::run(root.path(), &["init", "-q", "-b", "main"]).expect("init");
        cli::run(root.path(), &["config", "a.b", "value"]).expect("config");
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).expect("directory");
        let entries = read(&sub, false, Query::Get("a.b")).expect("read");
        // Git names the file by its real path, which on Windows may be spelled differently from
        // the temporary folder's (a short 8.3 name, `/`): the same file is what matters.
        let file = entries[0].file.as_deref().expect("a file");
        assert!(file.is_absolute(), "{file:?}");
        assert_eq!(
            std::fs::canonicalize(file).ok(),
            std::fs::canonicalize(root.path().join(".git/config")).ok()
        );
    }
}
