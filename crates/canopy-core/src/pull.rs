//! The user's git configuration that decides what a bare `git pull` does, and where each value
//! comes from.
//!
//! Canopy always passes the strategy it chose to `git pull` ([`Repo::pull`]); this module reads
//! what the user's configuration would have made git do, in git's own order, so that the choice
//! can start from it and a pull that goes against it can say which setting it went against. The
//! values are read through the `git` that pulls, with `--show-origin --show-scope`: libgit2 does
//! not honour `GIT_CONFIG_GLOBAL` and cannot say which file a value comes from.
//!
//! Git's order (`builtin/pull.c`, checked against git 2.55 in the tests):
//!
//! 1. `pull.ff=only` ends every pull that cannot fast-forward, even where a rebase is configured.
//! 2. Otherwise the rebase setting decides: `branch.<name>.rebase` of the current branch, and only
//!    without it `pull.rebase`. `true` and `interactive` rebase, `merges` rebases keeping merge
//!    commits, `false` merges. A rebase ignores `pull.ff` and `merge.ff`.
//! 3. A merge fast-forwards or not as `pull.ff` says, else as `merge.ff` says.
//! 4. With neither a rebase setting nor `pull.ff`, git has no preference: it fast-forwards when it
//!    can and refuses diverged branches ("Need to specify how to reconcile divergent branches").
//!    `merge.ff` alone does not change that, so no strategy comes out of it.

use std::path::PathBuf;

use crate::ops::PullStrategy;
use crate::paths::os_path;
use crate::{Repo, Result, cli};

/// Which configuration file layer a value comes from (`git config --show-scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    /// The system-wide configuration (`/etc/gitconfig`).
    System,
    /// The user's configuration (`~/.gitconfig`, `$XDG_CONFIG_HOME/git/config`, or the file
    /// `GIT_CONFIG_GLOBAL` names).
    Global,
    /// The repository's own `.git/config`, or a file it includes.
    Local,
    /// The configuration of one linked worktree.
    Worktree,
    /// A `-c` option or `GIT_CONFIG_*` environment variables.
    Command,
    /// Anything else git reports.
    Other,
}

impl ConfigScope {
    fn parse(scope: &str) -> Self {
        match scope {
            "system" => Self::System,
            "global" => Self::Global,
            "local" => Self::Local,
            "worktree" => Self::Worktree,
            "command" => Self::Command,
            _ => Self::Other,
        }
    }

    /// Where the user sets it, in words for a message.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "system git config",
            Self::Global => "your global git config",
            Self::Local => "repository",
            Self::Worktree => "this worktree",
            Self::Command => "command line or environment",
            Self::Other => "git config",
        }
    }
}

/// One configuration value and where git found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSource {
    /// The key as git spells it: section and variable in lower case, the subsection (a branch
    /// name) as written, for example `branch.main.rebase`.
    pub key: String,
    /// The value as written; `true` for a key written without a value.
    pub value: String,
    /// The configuration layer.
    pub scope: ConfigScope,
    /// What git prints for the origin, for example `file:/home/ada/.gitconfig`, `file:.git/config`
    /// or `command line:`.
    pub origin: String,
    /// The configuration file, when the origin is one. A relative path git prints (the
    /// repository's own `.git/config`) is joined to the working tree.
    pub file: Option<PathBuf>,
}

impl ConfigSource {
    /// `branch.main.rebase=true (repository)`.
    pub fn describe(&self) -> String {
        format!("{}={} ({})", self.key, self.value, self.scope.label())
    }
}

/// What the user's git configuration says `git pull` should do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullConfig {
    /// The strategy the configuration asks for; `None` when it asks for nothing (git itself then
    /// fast-forwards if it can and refuses to reconcile diverged branches).
    pub strategy: Option<PullStrategy>,
    /// The settings behind `strategy`, the deciding one first. Empty when `strategy` is `None`.
    pub sources: Vec<ConfigSource>,
    /// Values git would reject (`pull.rebase=sometimes`, the removed `preserve`): they are
    /// ignored here, and every pull Canopy runs passes its strategy, so it does not fail on them.
    pub invalid: Vec<ConfigSource>,
}

impl PullConfig {
    /// The settings behind the strategy for a message: `branch.main.rebase=true (repository)`, or
    /// several separated by commas. `None` when the configuration asks for nothing.
    pub fn describe(&self) -> Option<String> {
        if self.sources.is_empty() {
            return None;
        }
        let parts: Vec<String> = self.sources.iter().map(ConfigSource::describe).collect();
        Some(parts.join(", "))
    }

    /// The deciding setting when `chosen` is not what the configuration asks for: a pull with
    /// `chosen` goes against it. `None` when they agree or the configuration asks for nothing.
    pub fn contradicted_by(&self, chosen: PullStrategy) -> Option<&ConfigSource> {
        match self.strategy {
            Some(configured) if configured != chosen => self.sources.first(),
            _ => None,
        }
    }
}

/// `branch.<name>.rebase`, `pull.rebase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rebase {
    No,
    Yes,
    Merges,
}

/// `pull.ff`, `merge.ff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FastForward {
    /// `false`: always create a merge commit.
    Never,
    /// `true`: fast-forward when possible.
    Allow,
    /// `only`: fail unless it can fast-forward.
    Only,
}

/// A configuration value with its meaning; `None` when git rejects the value.
type Setting<T> = (ConfigSource, Option<T>);

impl Repo {
    /// What the user's git configuration asks `git pull` to do on the current branch (none when
    /// HEAD is detached: only `pull.rebase` then applies).
    pub fn pull_config(&self) -> Result<PullConfig> {
        let head = self.head()?;
        self.pull_config_for(head.branch.as_deref())
    }

    /// [`Repo::pull_config`] for the local branch `branch`, whether or not it is checked out.
    pub fn pull_config_for(&self, branch: Option<&str>) -> Result<PullConfig> {
        let entries = self.read_pull_entries()?;
        let last = |key: &str| entries.iter().rfind(|e| e.key == key).cloned();
        // Git reads `pull.rebase` only when the branch has no setting of its own.
        let rebase = branch
            .and_then(|b| last(&format!("branch.{b}.rebase")))
            .or_else(|| last("pull.rebase"));
        let rebase = rebase.map(|s| self.parse_rebase(s)).transpose()?;
        let pull_ff = last("pull.ff").map(|s| self.parse_ff(s)).transpose()?;
        let merge_ff = last("merge.ff").map(|s| self.parse_ff(s)).transpose()?;
        Ok(decide(rebase, pull_ff, merge_ff))
    }

    /// Every `pull.rebase`, `pull.ff`, `merge.ff` and `branch.*.rebase` entry in git's reading
    /// order, so the last one of a key is the one in force.
    fn read_pull_entries(&self) -> Result<Vec<ConfigSource>> {
        let (_, out) = cli::run_answer(
            self.workdir(),
            &[
                "config",
                "--show-origin",
                "--show-scope",
                "-z",
                "--get-regexp",
                r"^(pull\.(rebase|ff)|merge\.ff|branch\..*\.rebase)$",
            ],
            b"",
            &[0, 1],
        )?;
        let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
        let (entries, _) = fields.as_chunks::<3>();
        Ok(entries
            .iter()
            .map(|&[scope, origin, entry]| {
                // `key\nvalue`; a key written without a value has no newline and means true.
                let (key, value) = match entry.iter().position(|&b| b == b'\n') {
                    Some(at) => (&entry[..at], &entry[at + 1..]),
                    None => (entry, b"true".as_slice()),
                };
                let file = origin.strip_prefix(b"file:").map(|path| {
                    let path = os_path(path);
                    if path.is_relative() {
                        self.workdir().join(path)
                    } else {
                        path
                    }
                });
                let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
                ConfigSource {
                    key: text(key),
                    value: text(value),
                    scope: ConfigScope::parse(&text(scope)),
                    origin: text(origin),
                    file,
                }
            })
            .collect())
    }

    fn parse_rebase(&self, source: ConfigSource) -> Result<Setting<Rebase>> {
        let parsed = match source.value.as_str() {
            "merges" | "m" => Some(Rebase::Merges),
            "interactive" | "i" => Some(Rebase::Yes),
            value => self
                .parse_bool(&source.key, value)?
                .map(|yes| if yes { Rebase::Yes } else { Rebase::No }),
        };
        Ok((source, parsed))
    }

    fn parse_ff(&self, source: ConfigSource) -> Result<Setting<FastForward>> {
        let parsed = match source.value.as_str() {
            "only" => Some(FastForward::Only),
            value => self.parse_bool(&source.key, value)?.map(|yes| {
                if yes {
                    FastForward::Allow
                } else {
                    FastForward::Never
                }
            }),
        };
        Ok((source, parsed))
    }

    /// Git's boolean reading of `value` (`true`, `yes`, `on`, `1`, any non-zero number, the empty
    /// string for false), asking git itself for the spellings that are not the obvious ones.
    fn parse_bool(&self, key: &str, value: &str) -> Result<Option<bool>> {
        match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => return Ok(Some(true)),
            "false" | "no" | "off" | "0" | "" => return Ok(Some(false)),
            _ => {}
        }
        let (code, out) = cli::run_answer(
            self.workdir(),
            &["config", "--type=bool", "--get", key],
            b"",
            &[0, 1, 128],
        )?;
        Ok((code == 0).then(|| out.starts_with(b"true")))
    }
}

/// The setting with its meaning, or `None` after noting a value git rejects in `invalid`.
fn usable<T>(
    setting: Option<Setting<T>>,
    invalid: &mut Vec<ConfigSource>,
) -> Option<(ConfigSource, T)> {
    match setting? {
        (source, Some(value)) => Some((source, value)),
        (source, None) => {
            invalid.push(source);
            None
        }
    }
}

/// Turns the settings into the strategy git would follow (the module documentation lists the
/// order). A value git rejects is recorded in `invalid` and counts as not set.
fn decide(
    rebase: Option<Setting<Rebase>>,
    pull_ff: Option<Setting<FastForward>>,
    merge_ff: Option<Setting<FastForward>>,
) -> PullConfig {
    let mut invalid = Vec::new();
    let rebase = usable(rebase, &mut invalid);
    let pull_ff = usable(pull_ff, &mut invalid);
    let merge_ff = usable(merge_ff, &mut invalid);

    let (strategy, sources) = match (rebase, pull_ff, merge_ff) {
        (_, Some((ff, FastForward::Only)), _) => (Some(PullStrategy::FastForwardOnly), vec![ff]),
        (Some((rebase, Rebase::Yes)), _, _) => (Some(PullStrategy::Rebase), vec![rebase]),
        (Some((rebase, Rebase::Merges)), _, _) => (Some(PullStrategy::RebaseMerges), vec![rebase]),
        (Some((rebase, Rebase::No)), pull_ff, merge_ff) => {
            let (strategy, ff) = match (pull_ff, merge_ff) {
                (Some((ff, FastForward::Never)), _) => (PullStrategy::MergeNoFastForward, Some(ff)),
                (Some((ff, _)), _) => (PullStrategy::Merge, Some(ff)),
                (None, Some((ff, FastForward::Never))) => {
                    (PullStrategy::MergeNoFastForward, Some(ff))
                }
                (None, Some((ff, FastForward::Only))) => (PullStrategy::FastForwardOnly, Some(ff)),
                (None, Some((_, FastForward::Allow)) | None) => (PullStrategy::Merge, None),
            };
            (Some(strategy), std::iter::once(rebase).chain(ff).collect())
        }
        (None, Some((ff, FastForward::Never)), _) => {
            (Some(PullStrategy::MergeNoFastForward), vec![ff])
        }
        (None, Some((ff, FastForward::Allow)), _) => (Some(PullStrategy::Merge), vec![ff]),
        (None, None, _) => (None, Vec::new()),
    };
    PullConfig {
        strategy,
        sources,
        invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(key: &str, value: &str) -> ConfigSource {
        ConfigSource {
            key: key.to_owned(),
            value: value.to_owned(),
            scope: ConfigScope::Local,
            origin: "file:.git/config".to_owned(),
            file: None,
        }
    }

    fn rebase(value: &str, parsed: Option<Rebase>) -> Setting<Rebase> {
        (source("pull.rebase", value), parsed)
    }

    fn ff(key: &str, value: &str, parsed: Option<FastForward>) -> Setting<FastForward> {
        (source(key, value), parsed)
    }

    fn keys(config: &PullConfig) -> Vec<&str> {
        config.sources.iter().map(|s| s.key.as_str()).collect()
    }

    #[test]
    fn nothing_set_asks_for_nothing() {
        let config = decide(None, None, None);
        assert_eq!(config, PullConfig::default());
        assert_eq!(config.describe(), None);
        assert_eq!(config.contradicted_by(PullStrategy::Rebase), None);
    }

    #[test]
    fn merge_ff_alone_asks_for_nothing() {
        // git still refuses diverged branches, so there is no preference to carry over.
        let config = decide(
            None,
            None,
            Some(ff("merge.ff", "false", Some(FastForward::Never))),
        );
        assert_eq!(config.strategy, None);
        assert_eq!(config.sources, Vec::new());
    }

    #[test]
    fn rebase_values_pick_the_rebase_strategies() {
        let yes = decide(Some(rebase("true", Some(Rebase::Yes))), None, None);
        assert_eq!(yes.strategy, Some(PullStrategy::Rebase));
        let merges = decide(Some(rebase("merges", Some(Rebase::Merges))), None, None);
        assert_eq!(merges.strategy, Some(PullStrategy::RebaseMerges));
        let no = decide(Some(rebase("false", Some(Rebase::No))), None, None);
        assert_eq!(no.strategy, Some(PullStrategy::Merge));
        assert_eq!(keys(&no), ["pull.rebase"]);
    }

    #[test]
    fn a_rebase_ignores_pull_ff_false_and_merge_ff() {
        let config = decide(
            Some(rebase("true", Some(Rebase::Yes))),
            Some(ff("pull.ff", "false", Some(FastForward::Never))),
            Some(ff("merge.ff", "only", Some(FastForward::Only))),
        );
        assert_eq!(config.strategy, Some(PullStrategy::Rebase));
        assert_eq!(keys(&config), ["pull.rebase"]);
    }

    #[test]
    fn pull_ff_only_beats_a_rebase() {
        let config = decide(
            Some(rebase("true", Some(Rebase::Yes))),
            Some(ff("pull.ff", "only", Some(FastForward::Only))),
            None,
        );
        assert_eq!(config.strategy, Some(PullStrategy::FastForwardOnly));
        assert_eq!(keys(&config), ["pull.ff"]);
    }

    #[test]
    fn a_merge_follows_pull_ff_then_merge_ff() {
        let never = decide(
            Some(rebase("false", Some(Rebase::No))),
            Some(ff("pull.ff", "false", Some(FastForward::Never))),
            Some(ff("merge.ff", "only", Some(FastForward::Only))),
        );
        assert_eq!(never.strategy, Some(PullStrategy::MergeNoFastForward));
        assert_eq!(keys(&never), ["pull.rebase", "pull.ff"]);

        let allow = decide(
            Some(rebase("false", Some(Rebase::No))),
            Some(ff("pull.ff", "true", Some(FastForward::Allow))),
            Some(ff("merge.ff", "false", Some(FastForward::Never))),
        );
        assert_eq!(allow.strategy, Some(PullStrategy::Merge));

        let from_merge = decide(
            Some(rebase("false", Some(Rebase::No))),
            None,
            Some(ff("merge.ff", "false", Some(FastForward::Never))),
        );
        assert_eq!(from_merge.strategy, Some(PullStrategy::MergeNoFastForward));
        assert_eq!(keys(&from_merge), ["pull.rebase", "merge.ff"]);

        let only = decide(
            Some(rebase("false", Some(Rebase::No))),
            None,
            Some(ff("merge.ff", "only", Some(FastForward::Only))),
        );
        assert_eq!(only.strategy, Some(PullStrategy::FastForwardOnly));
    }

    #[test]
    fn pull_ff_alone_picks_a_merge() {
        let never = decide(
            None,
            Some(ff("pull.ff", "false", Some(FastForward::Never))),
            None,
        );
        assert_eq!(never.strategy, Some(PullStrategy::MergeNoFastForward));
        let allow = decide(
            None,
            Some(ff("pull.ff", "true", Some(FastForward::Allow))),
            None,
        );
        assert_eq!(allow.strategy, Some(PullStrategy::Merge));
    }

    #[test]
    fn rejected_values_are_listed_and_ignored() {
        let config = decide(
            Some(rebase("sometimes", None)),
            Some(ff("pull.ff", "false", Some(FastForward::Never))),
            None,
        );
        assert_eq!(config.strategy, Some(PullStrategy::MergeNoFastForward));
        assert_eq!(config.invalid.len(), 1);
        assert_eq!(config.invalid[0].value, "sometimes");
    }

    #[test]
    fn describe_names_the_setting_its_value_and_where() {
        let config = decide(Some(rebase("true", Some(Rebase::Yes))), None, None);
        assert_eq!(
            config.describe().as_deref(),
            Some("pull.rebase=true (repository)")
        );
        assert_eq!(
            config
                .contradicted_by(PullStrategy::Merge)
                .map(|s| s.key.as_str()),
            Some("pull.rebase")
        );
        assert_eq!(config.contradicted_by(PullStrategy::Rebase), None);
    }
}
