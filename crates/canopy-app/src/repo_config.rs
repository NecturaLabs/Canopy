//! What the user's git configuration says about the choices Canopy makes for a repository (how
//! a pull integrates, whether a fetch prunes branches and tags, whether commits are signed,
//! autostash, tags on push, where a push goes, the conflict style, message cleanup and a new
//! repository's first branch) and where each value is set, so Settings can show them beside
//! Canopy's own choice and say which one applies (AGENTS.md, "Git settings"). Read through the
//! same `git` that runs the operations, on a thread of its own: the UI thread never runs git.

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, bounded};

use canopy_core::PullConfig;
use canopy_core::identity::Source;

/// One configuration value and where git found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValue {
    /// The configuration key, such as `pull.rebase`.
    pub key: String,
    pub value: String,
    /// `global`, `local`, `system`, `worktree` or `command`; empty when this git cannot say.
    pub scope: String,
    /// Where it is set, as git prints it (`file:/home/ada/.gitconfig`), a file always absolute.
    pub origin: String,
    /// The configuration file as an absolute path, preserving non-UTF-8 names.
    pub file: Option<PathBuf>,
}

impl ConfigValue {
    /// Where the value is set, for [`crate::views::settings_screen::source_label`].
    pub fn source(&self) -> Source {
        Source::Config {
            scope: self.scope.clone(),
            origin: self.origin.clone(),
        }
    }

    /// The value read as a git boolean (`true`, `yes`, `on`, `1`; `false`, `no`, `off`, `0`).
    pub fn as_bool(&self) -> Option<bool> {
        match self.value.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" | "" => Some(false),
            _ => None,
        }
    }
}

/// A remote and the URL it fetches from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

/// What git's configuration says for one repository (or, without one, globally).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitFacts {
    /// The checked-out branch, when there is one.
    pub branch: Option<String>,
    /// What the configuration asks a pull to do (`branch.<name>.rebase`, `pull.rebase`,
    /// `pull.ff`, `merge.ff`), resolved in git's order by `canopy_core::Repo::pull_config`
    /// (globally `canopy_core::global_pull_config`); `None` when it could not be read.
    pub pull: Option<PullConfig>,
    pub fetch_prune: Option<ConfigValue>,
    /// `remote.<name>.prune` for each remote of the repository that sets it, in the order of
    /// [`Self::remotes`]; git lets it override `fetch.prune` for fetches from that remote.
    pub remote_prune: Vec<ConfigValue>,
    pub gpg_sign: Option<ConfigValue>,
    pub gpg_format: Option<ConfigValue>,
    /// How commits would be signed and what would stop it; read only while `commit.gpgsign` is
    /// on and for a repository.
    pub signing: Option<Signing>,
    /// `fetch.pruneTags`, and `remote.<name>.pruneTags` for each remote that sets it.
    pub prune_tags: Vec<ConfigValue>,
    /// `rebase.autoStash` (rebases) and `merge.autoStash` (merges and merging pulls).
    pub autostash: Vec<ConfigValue>,
    /// `push.followTags`.
    pub follow_tags: Option<ConfigValue>,
    /// `submodule.recurse`.
    pub submodule_recurse: Option<ConfigValue>,
    /// `push.default`.
    pub push_default: Option<ConfigValue>,
    /// `merge.conflictStyle`.
    pub conflict_style: Option<ConfigValue>,
    /// `commit.cleanup`.
    pub cleanup: Option<ConfigValue>,
    /// `init.defaultBranch`.
    pub initial_branch: Option<ConfigValue>,
    pub remotes: Vec<Remote>,
    /// The installed git's major and minor version, for the values Canopy would pass.
    pub git: Option<(u32, u32)>,
    /// Why the configuration could not be read (git missing, not a repository).
    pub error: Option<String>,
}

/// How git would sign a commit here, from configuration and what is installed; nothing is run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signing {
    /// `gpg.format` as set; `openpgp` when unset.
    pub format: String,
    /// `user.signingkey` shortened for display ([`key_label`]); the key itself is never kept.
    pub key: Option<String>,
    /// The program git runs: `gpg.<format>.program` (`gpg.program` for `OpenPGP`) or its default.
    pub program: String,
    /// What would make a commit fail, if anything.
    pub problem: Option<SigningProblem>,
}

/// Why signing would fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningProblem {
    /// `gpg.format` is none of `openpgp`, `ssh` and `x509`.
    UnknownFormat,
    /// The signing program is not installed.
    ProgramMissing,
    /// SSH signing needs a key and neither `user.signingkey` nor `gpg.ssh.defaultKeyCommand` is set.
    KeyNotSet,
    /// The key file `user.signingkey` names does not exist.
    KeyFileMissing,
}

impl Signing {
    /// The format as people name it.
    pub fn format_label(&self) -> &str {
        match self.format.as_str() {
            "openpgp" => "OpenPGP",
            "ssh" => "SSH",
            "x509" => "X.509",
            other => other,
        }
    }
}

/// `user.signingkey` as far as it is safe to show: a file's name, the end of a long key id or
/// fingerprint, or a generic label for SSH key material. `None` when unset or without a name.
fn key_label(key: &str) -> Option<String> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    if inline_ssh_key(key)
        || ((key.starts_with("ecdsa-") || key.starts_with("sk-"))
            && key.contains(char::is_whitespace))
    {
        return Some("an SSH key".to_owned());
    }
    if key.contains(['/', '\\']) || key.starts_with('~') {
        let name = key
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()?;
        return (!name.is_empty()).then(|| name.to_owned());
    }
    let chars: Vec<char> = key.chars().collect();
    Some(
        if chars.len() > KEY_ID_SHOWN && key.chars().all(|c| c.is_ascii_hexdigit()) {
            let tail: String = chars[chars.len() - KEY_ID_SHOWN..].iter().collect();
            format!("…{tail}")
        } else {
            chars.into_iter().take(KEY_LABEL_MAX).collect()
        },
    )
}

/// Characters of a long key id or fingerprint shown (the usual long key id).
const KEY_ID_SHOWN: usize = 16;
/// Characters of any other key description shown.
const KEY_LABEL_MAX: usize = 32;

/// Whether `user.signingkey` holds the key itself rather than naming a file: git takes only a
/// `key::` prefix or an `ssh-` start as a literal key and anything else as a path.
fn inline_ssh_key(key: &str) -> bool {
    key.starts_with("key::") || key.starts_with("ssh-")
}

/// How git signs with this configuration. `get` reads a configuration value, `program_missing`
/// says the signing program is not installed and `file_exists` is a plain metadata check, which
/// resolves a relative path where git runs (the repository's working directory).
fn signing_from(
    get: &dyn Fn(&str) -> Option<ConfigValue>,
    program_missing: &dyn Fn(&str) -> bool,
    file_exists: &dyn Fn(&Path) -> bool,
) -> Signing {
    // Git compares the value exactly (`SSH` is an error, so is an empty one).
    let format =
        get("gpg.format").map_or_else(|| "openpgp".to_owned(), |v| v.value.trim().to_owned());
    let (program_key, default_program) = match format.as_str() {
        "ssh" => ("gpg.ssh.program", "ssh-keygen"),
        "x509" => ("gpg.x509.program", "gpgsm"),
        _ => ("gpg.openpgp.program", "gpg"),
    };
    let program = get(program_key)
        .or_else(|| get("gpg.program").filter(|_| program_key == "gpg.openpgp.program"))
        .map(|v| v.value)
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| default_program.to_owned());
    let key = get("user.signingkey").map(|v| v.value);
    let problem = if !matches!(format.as_str(), "openpgp" | "ssh" | "x509") {
        Some(SigningProblem::UnknownFormat)
    } else if program_missing(&program) {
        Some(SigningProblem::ProgramMissing)
    } else if format == "ssh" {
        match key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            None => get("gpg.ssh.defaultKeyCommand")
                .is_none()
                .then_some(SigningProblem::KeyNotSet),
            Some(k) if inline_ssh_key(k) => None,
            Some(k) => {
                let path = crate::settings::expand_home(k);
                // `~user` is git's to expand: not judged here.
                (!path.to_string_lossy().starts_with('~') && !file_exists(&path))
                    .then_some(SigningProblem::KeyFileMissing)
            }
        }
    } else {
        None
    };
    Signing {
        format,
        key: key.as_deref().and_then(key_label),
        program,
        problem,
    }
}

impl GitFacts {
    /// Whether git signs commits (`commit.gpgsign`).
    pub fn signs_commits(&self) -> bool {
        self.gpg_sign
            .as_ref()
            .and_then(ConfigValue::as_bool)
            .unwrap_or(false)
    }
}

/// Reads the facts for `workdir`, or the global configuration when it is `None`, on a new
/// thread, and wakes the UI when they arrive.
pub fn read_async(ctx: &egui::Context, workdir: Option<PathBuf>) -> Receiver<GitFacts> {
    let (tx, rx) = bounded(1);
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("git-config".into())
        .spawn(move || {
            let facts = read(workdir.as_deref());
            let _ = tx.send(facts);
            ctx.request_repaint();
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the git configuration reader");
    }
    rx
}

/// Reads the facts now (blocking: never on the UI thread).
pub fn read(workdir: Option<&Path>) -> GitFacts {
    let home = crate::settings::home_dir();
    // Global values are read from the home folder (or the current one), outside any repository.
    let dir = workdir.unwrap_or_else(|| {
        if home.as_os_str().is_empty() {
            Path::new(".")
        } else {
            home.as_path()
        }
    });
    let scope: &[&str] = if workdir.is_some() {
        &[]
    } else {
        &["--global"]
    };
    let get = |key: &str| get_value(dir, scope, key);
    let mut facts = GitFacts {
        git: canopy_core::cli::version_number(),
        ..GitFacts::default()
    };
    match get_bool(dir, scope, "fetch.prune") {
        Ok(v) => facts.fetch_prune = v,
        Err(e) => {
            facts.error = Some(e);
            return facts;
        }
    }
    if workdir.is_some() {
        facts.branch =
            canopy_core::cli::run_unchecked(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
                .ok()
                .filter(|o| o.code == Some(0))
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|b| !b.is_empty());
        facts.remotes = remotes(dir);
        match remote_prune_values(&facts.remotes, |key| get_bool(dir, scope, key)) {
            Ok(values) => facts.remote_prune = values,
            Err(e) => {
                facts.error = Some(e);
                return facts;
            }
        }
    }
    facts.prune_tags = get_bool(dir, scope, "fetch.pruneTags")
        .ok()
        .flatten()
        .into_iter()
        .chain(facts.remotes.iter().filter_map(|r| {
            get_bool(dir, scope, &format!("remote.{}.pruneTags", r.name))
                .ok()
                .flatten()
        }))
        .collect();
    let pull = match workdir {
        Some(w) => canopy_core::Repo::open(w).and_then(|r| r.pull_config()),
        None => canopy_core::global_pull_config(dir),
    };
    facts.pull = pull
        .inspect_err(|e| tracing::debug!(error = %e, "pull settings not read"))
        .ok();
    facts.gpg_sign = get_bool(dir, scope, "commit.gpgsign").ok().flatten();
    facts.gpg_format = get("gpg.format").ok().flatten();
    if let Some(w) = workdir
        && facts.signs_commits()
    {
        // The tool check reads configuration and `PATH` only; it never runs the program.
        let missing = canopy_core::Repo::open(w)
            .map(|r| r.missing_tools())
            .unwrap_or_default();
        facts.signing = Some(signing_from(
            &|key| get(key).ok().flatten(),
            &|program| missing.iter().any(|m| m.program == program),
            &|path| w.join(path).metadata().is_ok(),
        ));
    }
    facts.autostash = ["rebase.autoStash", "merge.autoStash"]
        .into_iter()
        .filter_map(|key| get_bool(dir, scope, key).ok().flatten())
        .collect();
    facts.follow_tags = get_bool(dir, scope, "push.followTags").ok().flatten();
    facts.submodule_recurse = get_bool(dir, scope, "submodule.recurse").ok().flatten();
    facts.push_default = get("push.default").ok().flatten();
    facts.conflict_style = get("merge.conflictStyle").ok().flatten();
    facts.cleanup = get("commit.cleanup").ok().flatten();
    facts.initial_branch = get("init.defaultBranch").ok().flatten();
    facts
}

/// `git config -z --show-scope --show-origin --get <key>`: `Ok(None)` when it is not set. `-z`
/// keeps git from C-quoting a path with non-ASCII characters; the origin is made absolute here,
/// since git prints a repository's own file relative to its worktree root.
fn get_value(dir: &Path, scope: &[&str], key: &str) -> Result<Option<ConfigValue>, String> {
    canopy_core::config::read(
        dir,
        scope.contains(&"--global"),
        canopy_core::config::Query::Get(key),
    )
    .map(|entries| entries.into_iter().next_back().map(ConfigValue::from))
    .map_err(|e| e.to_string())
}

/// Reads each remote's pruning preference without silently dropping a failed git config read.
fn remote_prune_values(
    remotes: &[Remote],
    mut get: impl FnMut(&str) -> Result<Option<ConfigValue>, String>,
) -> Result<Vec<ConfigValue>, String> {
    let mut values = Vec::new();
    for remote in remotes {
        let key = format!("remote.{}.prune", remote.name);
        if let Some(value) = get(&key).map_err(|e| format!("{key}: {e}"))? {
            values.push(value);
        }
    }
    Ok(values)
}

/// [`get_value`] for a boolean key, with the value as git reads it: a key written without `=`
/// is true, one with an empty value false, and both print as an empty value.
fn get_bool(dir: &Path, scope: &[&str], key: &str) -> Result<Option<ConfigValue>, String> {
    let mut found = get_value(dir, scope, key)?;
    if let Some(v) = found.as_mut().filter(|v| v.value.is_empty()) {
        let mut args = vec!["config"];
        args.extend_from_slice(scope);
        args.extend(["--bool", "--get", key]);
        if let Ok(out) = canopy_core::cli::run_unchecked(dir, &args)
            && out.code == Some(0)
        {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .clone_into(&mut v.value);
        }
    }
    Ok(found)
}

impl From<canopy_core::config::Entry> for ConfigValue {
    fn from(entry: canopy_core::config::Entry) -> Self {
        Self {
            key: entry.key,
            value: entry.value.unwrap_or_default(),
            scope: entry.scope,
            origin: entry.origin,
            file: entry.file,
        }
    }
}

#[cfg(test)]
fn parse_value(
    key: &str,
    out: impl AsRef<[u8]>,
    with_scope: bool,
    dir: &Path,
) -> Option<ConfigValue> {
    canopy_core::config::parse(
        out.as_ref(),
        canopy_core::config::Query::Get(key),
        with_scope,
        dir,
    )
    .into_iter()
    .next_back()
    .map(ConfigValue::from)
}

/// The repository's remotes and their URLs, in configuration order.
fn remotes(dir: &Path) -> Vec<Remote> {
    canopy_core::cli::run_unchecked(dir, &["config", "--get-regexp", r"^remote\..*\.url$"])
        .ok()
        .filter(|o| o.code == Some(0))
        .map(|o| parse_remotes(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// `remote.<name>.url <url>` lines; a name may itself contain dots.
fn parse_remotes(out: &str) -> Vec<Remote> {
    out.lines()
        .filter_map(|line| {
            let (key, url) = line.split_once(' ')?;
            let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
            Some(Remote {
                name: name.to_owned(),
                url: url.trim().to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_where_they_are_set_are_read() {
        let here = Path::new("/work/repo");
        let v = parse_value(
            "pull.rebase",
            "global\0file:/home/ada/.gitconfig\0true\0",
            true,
            here,
        )
        .expect("value");
        assert_eq!(
            (v.scope.as_str(), v.origin.as_str(), v.value.as_str()),
            ("global", "file:/home/ada/.gitconfig", "true")
        );
        assert_eq!(v.as_bool(), Some(true));
        // The last of several values wins, as for git itself.
        let last = parse_value(
            "pull.ff",
            "system\0file:/etc/gitconfig\0true\0local\0file:/work/repo/.git/config\0only\0",
            true,
            here,
        )
        .expect("value");
        assert_eq!(
            (last.scope.as_str(), last.value.as_str()),
            ("local", "only")
        );
        assert_eq!(last.as_bool(), None, "only is not a boolean");
        let old =
            parse_value("fetch.prune", "file:/r/.git/config\0false\0", false, here).expect("value");
        assert_eq!((old.scope.as_str(), old.as_bool()), ("", Some(false)));
        let empty = parse_value("core.x", "local\0file:/r/.git/config\0\0", true, here)
            .expect("an empty value");
        assert_eq!(empty.value, "");
        assert_eq!(parse_value("x", "", true, here), None);
    }

    /// Git prints the repository's own file relative to where it ran; the origin must name the
    /// file wherever Canopy's working directory is.
    #[test]
    fn a_relative_origin_is_resolved_against_the_repository() {
        let here = Path::new("/work/repo");
        let v = parse_value("a.b", "local\0file:.git/config\0x\0", true, here).expect("value");
        assert_eq!(
            Path::new(v.origin.strip_prefix("file:").expect("file origin")),
            here.join(".git/config")
        );
        let command = parse_value("a.b", "command\0command line:\0x\0", true, here).expect("value");
        assert_eq!(command.origin, "command line:");
        let absolute =
            parse_value("a.b", "global\0file:/etc/gitconfig\0x\0", true, here).expect("value");
        assert_eq!(absolute.origin, "file:/etc/gitconfig");
    }

    /// Git prints the local file relative to the worktree root; the reader joins that root.
    #[test]
    fn the_origin_of_a_non_ascii_path_is_the_real_path() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let dir = root.path().join("höme répo");
        std::fs::create_dir(&dir)?;
        canopy_core::cli::run(&dir, &["init", "-q"]).expect("git");
        canopy_core::cli::run(&dir, &["config", "user.signingkey", "ABCDEF12"]).expect("git");
        let found = get_value(&dir, &[], "user.signingkey")
            .expect("read")
            .expect("set");
        assert_eq!(found.scope, "local");
        assert_eq!(
            found.origin,
            format!("file:{}", dir.join(".git/config").display())
        );
        Ok(())
    }

    /// An included file has its non-ASCII origin unquoted by `-z`.
    #[test]
    fn an_included_non_ascii_config_file_is_unquoted() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        canopy_core::cli::run(dir.path(), &["init", "-q", "-b", "main"]).expect("git");
        std::fs::write(dir.path().join(".git/ü.inc"), "[commit]\n gpgsign = true\n")?;
        canopy_core::cli::run(dir.path(), &["config", "include.path", "ü.inc"]).expect("git");
        let found = get_value(dir.path(), &[], "commit.gpgsign")
            .expect("read")
            .expect("set");
        assert_eq!(found.file, Some(dir.path().join(".git/ü.inc")));
        assert!(!found.origin.contains('"'), "{}", found.origin);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_config_file_keeps_non_utf8_path_bytes() -> std::io::Result<()> {
        use std::os::unix::ffi::OsStringExt;

        let root = tempfile::tempdir()?;
        let dir = root
            .path()
            .join(std::ffi::OsString::from_vec(b"bad\xffdir".to_vec()));
        std::fs::create_dir(&dir)?;
        canopy_core::cli::run(&dir, &["init", "-q", "-b", "main"]).expect("git");
        canopy_core::cli::run(&dir, &["config", "commit.gpgsign", "true"]).expect("git");
        let found = get_value(&dir, &[], "commit.gpgsign")
            .expect("read")
            .expect("set");
        assert_eq!(found.file, Some(dir.join(".git/config")));
        Ok(())
    }

    #[test]
    fn signing_key_labels_hide_key_material_and_empty_names() {
        assert_eq!(key_label("/"), None);
        assert_eq!(key_label("\\"), None);
        let label = key_label("ecdsa-sha2-nistp256 AAAAprivate-public-key-data").expect("label");
        assert!(!label.contains("AAAA"), "{label}");
    }

    #[test]
    fn remote_prune_reader_exposes_git_config_failures() {
        let remotes = [
            Remote {
                name: "origin".into(),
                url: "https://example.com/o".into(),
            },
            Remote {
                name: "mirror".into(),
                url: "https://example.com/m".into(),
            },
        ];
        let error = remote_prune_values(&remotes, |key| {
            if key == "remote.mirror.prune" {
                Err("invalid configuration".into())
            } else {
                Ok(None)
            }
        })
        .expect_err("a failed config read must reach the user");
        assert!(error.contains("remote.mirror.prune"), "{error}");
        assert!(error.contains("invalid configuration"), "{error}");
    }

    #[test]
    fn remote_names_may_contain_dots() {
        let remotes = parse_remotes(
            "remote.origin.url git@github.com:o/r.git\nremote.team.mirror.url https://example.com/r.git\n",
        );
        assert_eq!(
            remotes,
            [
                Remote {
                    name: "origin".into(),
                    url: "git@github.com:o/r.git".into()
                },
                Remote {
                    name: "team.mirror".into(),
                    url: "https://example.com/r.git".into()
                }
            ]
        );
    }

    /// Runs git in a temporary repository whose configuration this test owns.
    #[test]
    fn a_repository_s_configuration_is_read_with_its_branch() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let git = |args: &[&str]| {
            canopy_core::cli::run(dir.path(), args).expect("git");
        };
        git(&["init", "-q", "-b", "trunk"]);
        git(&["config", "pull.rebase", "false"]);
        git(&["config", "branch.trunk.rebase", "true"]);
        // Read after any global pull.ff=only, so the developer's configuration cannot decide.
        git(&["config", "pull.ff", "true"]);
        git(&["config", "fetch.prune", "true"]);
        git(&["remote", "add", "origin", "https://example.com/o/r.git"]);
        git(&["remote", "add", "mirror", "https://example.com/m/r.git"]);
        git(&["config", "remote.mirror.prune", "false"]);
        git(&["config", "remote.mirror.pruneTags", "true"]);
        git(&["config", "merge.autoStash", "false"]);
        git(&["config", "push.default", "current"]);
        git(&["config", "commit.cleanup", "strip"]);
        let facts = read(Some(dir.path()));
        assert_eq!(facts.error, None);
        assert_eq!(facts.branch.as_deref(), Some("trunk"));
        let pull = facts.pull.as_ref().expect("read");
        assert_eq!(
            pull.strategy,
            Some(canopy_core::PullStrategy::Rebase),
            "the branch's own setting comes before pull.rebase"
        );
        assert_eq!(pull.sources[0].key, "branch.trunk.rebase");
        assert_eq!(
            facts.fetch_prune.as_ref().and_then(ConfigValue::as_bool),
            Some(true)
        );
        assert_eq!(facts.remotes.len(), 2);
        assert_eq!(facts.remotes[0].url, "https://example.com/o/r.git");
        let [mirror] = facts.remote_prune.as_slice() else {
            panic!(
                "only the mirror sets its own pruning: {:?}",
                facts.remote_prune
            );
        };
        assert_eq!(
            (mirror.key.as_str(), mirror.as_bool()),
            ("remote.mirror.prune", Some(false))
        );
        let value = |values: &[ConfigValue], key: &str| {
            values
                .iter()
                .find(|v| v.key == key)
                .map(|v| v.value.clone())
        };
        assert_eq!(
            value(&facts.prune_tags, "remote.mirror.pruneTags").as_deref(),
            Some("true")
        );
        assert_eq!(
            value(&facts.autostash, "merge.autoStash").as_deref(),
            Some("false")
        );
        assert_eq!(
            facts.push_default.as_ref().map(|v| v.value.as_str()),
            Some("current")
        );
        assert_eq!(
            facts.cleanup.as_ref().map(|v| v.value.as_str()),
            Some("strip")
        );
        Ok(())
    }

    /// A key with no `=` is true to git, one with an empty value false; read as git reads them.
    #[test]
    fn a_key_without_a_value_is_true_as_git_reads_it() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let git = |args: &[&str]| canopy_core::cli::run(dir.path(), args).expect("git");
        git(&["init", "-q"]);
        let config = dir.path().join(".git").join("config");
        let mut text = std::fs::read_to_string(&config)?;
        text.push_str("[fetch]\n\tprune\n[commit]\n\tgpgsign\n");
        std::fs::write(&config, &text)?;
        let facts = read(Some(dir.path()));
        assert_eq!(
            git(&["config", "--bool", "--get", "fetch.prune"]).trim(),
            "true"
        );
        assert_eq!(
            facts.fetch_prune.as_ref().and_then(ConfigValue::as_bool),
            Some(true)
        );
        assert!(facts.signs_commits());

        text.push_str("[fetch]\n\tprune =\n");
        std::fs::write(&config, &text)?;
        let facts = read(Some(dir.path()));
        assert_eq!(
            git(&["config", "--bool", "--get", "fetch.prune"]).trim(),
            "false"
        );
        assert_eq!(
            facts.fetch_prune.as_ref().and_then(ConfigValue::as_bool),
            Some(false)
        );
        Ok(())
    }

    fn values(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<ConfigValue> + use<> {
        let map: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| {
            map.get(key).map(|value| ConfigValue {
                key: key.to_owned(),
                value: value.clone(),
                scope: "global".into(),
                origin: "file:/home/ada/.gitconfig".into(),
                file: Some(PathBuf::from("/home/ada/.gitconfig")),
            })
        }
    }

    #[test]
    fn openpgp_signing_names_its_program_and_needs_no_key() {
        let signing = signing_from(&values(&[]), &|_| false, &|_| false);
        assert_eq!(signing.format_label(), "OpenPGP");
        assert_eq!(signing.program, "gpg");
        assert_eq!((signing.key.as_deref(), signing.problem), (None, None));

        let configured = values(&[
            ("user.signingkey", "3AA5C34371567BD2"),
            ("gpg.program", "gpg2"),
        ]);
        let signing = signing_from(&configured, &|_| false, &|_| false);
        assert_eq!(signing.program, "gpg2");
        assert_eq!(signing.key.as_deref(), Some("3AA5C34371567BD2"));

        let own = values(&[("gpg.program", "gpg2"), ("gpg.openpgp.program", "gpg-new")]);
        assert_eq!(
            signing_from(&own, &|_| false, &|_| false).program,
            "gpg-new"
        );
    }

    #[test]
    fn a_long_fingerprint_is_shortened() {
        let fingerprint = "0123456789ABCDEF0123456789ABCDEF3AA5C34371567BD2";
        let signing = signing_from(
            &values(&[("user.signingkey", fingerprint)]),
            &|_| false,
            &|_| false,
        );
        assert_eq!(signing.key.as_deref(), Some("…3AA5C34371567BD2"));
    }

    #[test]
    fn ssh_signing_checks_the_key_file_and_never_shows_an_inline_key() {
        let ssh = |key: &'static str| values(&[("gpg.format", "ssh"), ("user.signingkey", key)]);
        let signing = signing_from(&ssh("/home/ada/.ssh/id_ed25519.pub"), &|_| false, &|_| true);
        assert_eq!(signing.format_label(), "SSH");
        assert_eq!(signing.program, "ssh-keygen");
        assert_eq!(signing.key.as_deref(), Some("id_ed25519.pub"));
        assert_eq!(signing.problem, None);

        let signing = signing_from(&ssh("/home/ada/.ssh/id_ed25519.pub"), &|_| false, &|_| {
            false
        });
        assert_eq!(signing.problem, Some(SigningProblem::KeyFileMissing));

        let inline = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIsecret";
        let signing = signing_from(&ssh(inline), &|_| false, &|_| false);
        assert_eq!(signing.problem, None, "an inline key is not a file");
        let shown = signing.key.as_deref().expect("label");
        assert!(!shown.contains("AAAA"), "{shown}");
        assert!(
            !format!("{signing:?}").contains("AAAA"),
            "the key is not kept at all"
        );

        let none = values(&[("gpg.format", "ssh")]);
        assert_eq!(
            signing_from(&none, &|_| false, &|_| false).problem,
            Some(SigningProblem::KeyNotSet)
        );
        let command = values(&[
            ("gpg.format", "ssh"),
            ("gpg.ssh.defaultKeyCommand", "ssh-add -L"),
        ]);
        assert_eq!(signing_from(&command, &|_| false, &|_| false).problem, None);
    }

    /// Git takes only `key::` and `ssh-` as a literal key; any other value is a path, so a
    /// relative one is looked for where git runs and a Windows one names its file.
    #[test]
    fn a_key_is_inline_only_as_git_reads_it() {
        assert!(inline_ssh_key("key::ssh-ed25519 AAAA") && inline_ssh_key("ssh-ed25519 AAAA"));
        for path in [
            "ecdsa-sha2-nistp256 AAAA",
            "sk-ssh-ed25519@openssh.com AAAA",
            "id.pub",
        ] {
            assert!(!inline_ssh_key(path), "{path}");
        }
        assert_eq!(
            key_label(r"C:\Users\ada\.ssh\id_ed25519.pub").as_deref(),
            Some("id_ed25519.pub")
        );
        assert_eq!(key_label("~/.ssh/id.pub").as_deref(), Some("id.pub"));

        let ssh = values(&[("gpg.format", "ssh"), ("user.signingkey", "keys/id.pub")]);
        let seen = std::cell::RefCell::new(None);
        let signing = signing_from(&ssh, &|_| false, &|p| {
            *seen.borrow_mut() = Some(p.to_path_buf());
            false
        });
        assert_eq!(seen.into_inner().as_deref(), Some(Path::new("keys/id.pub")));
        assert_eq!(signing.problem, Some(SigningProblem::KeyFileMissing));
        let user = values(&[("gpg.format", "ssh"), ("user.signingkey", "~bob/id.pub")]);
        assert_eq!(signing_from(&user, &|_| false, &|_| false).problem, None);
    }

    /// A relative key path is checked in the repository, not where Canopy happens to run.
    #[test]
    fn a_relative_key_file_is_looked_for_in_the_repository() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let git = |args: &[&str]| {
            canopy_core::cli::run(dir.path(), args).expect("git");
        };
        git(&["init", "-q"]);
        git(&["config", "commit.gpgsign", "true"]);
        git(&["config", "gpg.format", "ssh"]);
        git(&["config", "gpg.ssh.program", "sh"]);
        git(&["config", "user.signingkey", "keys/id.pub"]);
        let signing = read(Some(dir.path())).signing.expect("signing is on");
        assert_eq!(signing.problem, Some(SigningProblem::KeyFileMissing));
        std::fs::create_dir(dir.path().join("keys"))?;
        std::fs::write(dir.path().join("keys/id.pub"), "ssh-ed25519 AAAA\n")?;
        let signing = read(Some(dir.path())).signing.expect("signing is on");
        assert_eq!(signing.problem, None);
        assert_eq!(signing.key.as_deref(), Some("id.pub"));
        Ok(())
    }

    #[test]
    fn x509_signing_uses_gpgsm_and_an_unknown_format_is_a_problem() {
        let x509 = values(&[("gpg.format", "x509")]);
        let signing = signing_from(&x509, &|_| false, &|_| false);
        assert_eq!(signing.format_label(), "X.509");
        assert_eq!(signing.program, "gpgsm");

        // Git compares the format exactly: `X509` and an empty value are errors in git itself.
        for odd in ["pgp", "X509", "SSH", "OpenPGP", ""] {
            let config = values(&[("gpg.format", odd)]);
            let signing = signing_from(&config, &|_| false, &|_| false);
            assert_eq!(
                signing.problem,
                Some(SigningProblem::UnknownFormat),
                "{odd:?}"
            );
            assert_eq!(signing.format, odd);
        }
    }

    #[test]
    fn a_missing_program_is_the_first_problem() {
        let config = values(&[("gpg.format", "ssh")]);
        let signing = signing_from(&config, &|p| p == "ssh-keygen", &|_| false);
        assert_eq!(signing.problem, Some(SigningProblem::ProgramMissing));
    }

    #[test]
    fn signing_is_read_only_while_commits_are_signed() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let git = |args: &[&str]| {
            canopy_core::cli::run(dir.path(), args).expect("git");
        };
        git(&["init", "-q"]);
        // Read after any global signing configuration, so the developer's cannot decide.
        git(&["config", "commit.gpgsign", "false"]);
        assert_eq!(read(Some(dir.path())).signing, None);

        git(&["config", "commit.gpgsign", "true"]);
        git(&["config", "gpg.format", "ssh"]);
        git(&["config", "gpg.ssh.program", "canopy-no-such-signer"]);
        git(&["config", "user.signingkey", "key::ssh-ed25519 AAAA"]);
        let facts = read(Some(dir.path()));
        let signing = facts.signing.expect("signing is on");
        assert_eq!(signing.format_label(), "SSH");
        assert_eq!(signing.program, "canopy-no-such-signer");
        assert_eq!(signing.problem, Some(SigningProblem::ProgramMissing));
        Ok(())
    }
}
