//! What the user's git configuration says about the choices Canopy makes for a repository (how
//! a pull integrates, whether a fetch prunes, whether commits are signed) and where each value is
//! set, so Settings can show them beside Canopy's own choice and say where they differ (AGENTS.md,
//! "Git settings"). Read through the same `git` that runs the operations, on a thread of its own:
//! the UI thread never runs git.

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
    /// Where it is set, as git prints it (`file:/home/ada/.gitconfig`).
    pub origin: String,
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
    pub remotes: Vec<Remote>,
    /// Why the configuration could not be read (git missing, not a repository).
    pub error: Option<String>,
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
    let mut facts = GitFacts::default();
    match get("fetch.prune") {
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
        facts.remote_prune = facts
            .remotes
            .iter()
            .filter_map(|r| get(&format!("remote.{}.prune", r.name)).ok().flatten())
            .collect();
    }
    let pull = match workdir {
        Some(w) => canopy_core::Repo::open(w).and_then(|r| r.pull_config()),
        None => canopy_core::global_pull_config(dir),
    };
    facts.pull = pull
        .inspect_err(|e| tracing::debug!(error = %e, "pull settings not read"))
        .ok();
    facts.gpg_sign = get("commit.gpgsign").ok().flatten();
    facts.gpg_format = get("gpg.format").ok().flatten();
    facts
}

/// `git config --show-scope --show-origin --get <key>`: `Ok(None)` when it is not set.
fn get_value(dir: &Path, scope: &[&str], key: &str) -> Result<Option<ConfigValue>, String> {
    let mut args = vec!["config"];
    args.extend_from_slice(scope);
    args.extend(["--show-scope", "--show-origin", "--get", key]);
    let out = canopy_core::cli::run_unchecked(dir, &args).map_err(|e| e.to_string())?;
    match out.code {
        Some(0) => Ok(parse_value(
            key,
            &String::from_utf8_lossy(&out.stdout),
            true,
        )),
        Some(1) => Ok(None),
        // An older git without --show-scope (before 2.26): ask again without it.
        _ => {
            let mut args = vec!["config"];
            args.extend_from_slice(scope);
            args.extend(["--show-origin", "--get", key]);
            let out = canopy_core::cli::run_unchecked(dir, &args).map_err(|e| e.to_string())?;
            match out.code {
                Some(0) => Ok(parse_value(
                    key,
                    &String::from_utf8_lossy(&out.stdout),
                    false,
                )),
                Some(1) => Ok(None),
                _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
            }
        }
    }
}

/// The last line of `git config --show-origin [--show-scope] --get` output.
fn parse_value(key: &str, out: &str, with_scope: bool) -> Option<ConfigValue> {
    let line = out.lines().rev().find(|l| !l.trim().is_empty())?;
    let mut parts = line.splitn(if with_scope { 3 } else { 2 }, '\t');
    let scope = if with_scope { parts.next()? } else { "" };
    let origin = parts.next()?;
    let value = parts.next()?;
    Some(ConfigValue {
        key: key.to_owned(),
        value: value.to_owned(),
        scope: scope.to_owned(),
        origin: origin.to_owned(),
    })
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
        let v = parse_value(
            "pull.rebase",
            "global\tfile:/home/ada/.gitconfig\ttrue\n",
            true,
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
            "system\tfile:/etc/gitconfig\ttrue\nlocal\tfile:.git/config\tonly\n",
            true,
        )
        .expect("value");
        assert_eq!(
            (last.scope.as_str(), last.value.as_str()),
            ("local", "only")
        );
        assert_eq!(last.as_bool(), None, "only is not a boolean");
        let old = parse_value("fetch.prune", "file:.git/config\tfalse\n", false).expect("value");
        assert_eq!((old.scope.as_str(), old.as_bool()), ("", Some(false)));
        assert_eq!(parse_value("x", "", true), None);
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
        Ok(())
    }
}
