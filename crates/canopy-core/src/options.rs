//! Canopy's choices for how git carries out what Canopy asks.
//!
//! Where the user's git configuration sets one of them and [`GitOptions::follow_config`] is on,
//! git's own configuration decides and Canopy passes nothing for it; otherwise Canopy's choice is
//! passed explicitly (a flag, or a `-c name=value` override), so no configuration value changes
//! the outcome. Settings shows both, with Canopy's recommendation.

use crate::{Repo, cli};

/// How a conflicted file shows the two sides (`merge.conflictStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictStyle {
    /// Both sides and the original text, trimmed to what really differs (`zdiff3`, git 2.35+).
    #[default]
    Zdiff3,
    /// Both sides and the original text (`diff3`).
    Diff3,
    /// Both sides only (`merge`, git's own default).
    Merge,
}

impl ConflictStyle {
    pub const ALL: [Self; 3] = [Self::Zdiff3, Self::Diff3, Self::Merge];

    /// The `merge.conflictStyle` value for a git of version `git`: `zdiff3` becomes `diff3` on a
    /// git older than 2.35, which refuses it, or one whose version is unknown.
    pub fn value(self, git: Option<(u32, u32)>) -> &'static str {
        match self {
            Self::Zdiff3 if git.is_some_and(|v| v >= cli::ZDIFF3_SINCE) => "zdiff3",
            Self::Zdiff3 | Self::Diff3 => "diff3",
            Self::Merge => "merge",
        }
    }

    /// The style a `merge.conflictStyle` value names.
    pub fn from_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "zdiff3" => Some(Self::Zdiff3),
            "diff3" => Some(Self::Diff3),
            "merge" => Some(Self::Merge),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Zdiff3 => "Both sides and the original, trimmed",
            Self::Diff3 => "Both sides and the original",
            Self::Merge => "Both sides only",
        }
    }
}

/// Where a push without a destination sends the current branch (`push.default`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PushDefault {
    /// To its upstream, only when that has the same name (`simple`).
    #[default]
    Simple,
    /// To its upstream, whatever its name (`upstream`).
    Upstream,
    /// To the branch of the same name on the remote (`current`).
    Current,
}

impl PushDefault {
    pub const ALL: [Self; 3] = [Self::Simple, Self::Upstream, Self::Current];

    pub fn value(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Upstream => "upstream",
            Self::Current => "current",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Simple => "Its upstream, when the names match",
            Self::Upstream => "Its upstream, whatever its name",
            Self::Current => "The branch of the same name",
        }
    }
}

/// What a commit removes from its message (`commit.cleanup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cleanup {
    /// Trailing spaces and surrounding blank lines only: a line starting with `#` stays
    /// (`whitespace`).
    #[default]
    Whitespace,
    /// Also lines starting with `#` (`strip`, git's default for an edited message).
    Strip,
    /// Nothing (`verbatim`).
    Verbatim,
}

impl Cleanup {
    pub const ALL: [Self; 3] = [Self::Whitespace, Self::Strip, Self::Verbatim];

    pub fn value(self) -> &'static str {
        match self {
            Self::Whitespace => "whitespace",
            Self::Strip => "strip",
            Self::Verbatim => "verbatim",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Whitespace => "Spaces and blank lines",
            Self::Strip => "Also lines starting with #",
            Self::Verbatim => "Nothing",
        }
    }
}

/// How Canopy has git pull, fetch, push, commit, merge, rebase, cherry-pick, revert and apply
/// stashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitOptions {
    /// Where the user's git configuration sets one of the choices below, it decides.
    pub follow_config: bool,
    /// Pull and rebase with uncommitted changes: git stashes them first and puts them back
    /// afterwards (`--autostash`).
    pub autostash: bool,
    /// A push also sends the annotated tags on the commits it pushes (`--follow-tags`).
    pub follow_tags: bool,
    /// How conflicts are written into files.
    pub conflict_style: ConflictStyle,
    /// Where a push without a destination goes.
    pub push_default: PushDefault,
    /// What a commit removes from its message.
    pub cleanup: Cleanup,
    /// A pruning fetch also deletes local tags the remote does not have (`--prune-tags`), tags
    /// never pushed included.
    pub prune_tags: bool,
    /// After a pull or a checkout, and in a new clone, the submodules are brought to the
    /// commits the superproject records (`git submodule update --init --recursive`).
    pub recurse_submodules: bool,
    /// Discard on a submodule also deletes the untracked files inside it (`git clean -f -d`,
    /// ignored files kept); off, they stay.
    pub discard_untracked_in_submodules: bool,
}

impl Default for GitOptions {
    /// Today's recommended practice (`docs/adr/0012-git-defaults.md`), with the user's git
    /// configuration followed where it sets something.
    fn default() -> Self {
        Self {
            follow_config: true,
            autostash: true,
            follow_tags: true,
            conflict_style: ConflictStyle::Zdiff3,
            push_default: PushDefault::Simple,
            cleanup: Cleanup::Whitespace,
            prune_tags: false,
            recurse_submodules: true,
            discard_untracked_in_submodules: false,
        }
    }
}

fn flag(on: bool, yes: &'static str, no: &'static str) -> &'static str {
    if on { yes } else { no }
}

impl Repo {
    /// Whether the git configuration decides the choice its keys match (`pattern`, a regular
    /// expression over lower-case key names): Canopy follows it and it sets one of them.
    pub(crate) fn config_decides(&self, pattern: &str) -> bool {
        self.options().follow_config
            && crate::config::read(
                self.workdir(),
                false,
                crate::config::Query::Matching(pattern),
            )
            .is_ok_and(|entries| !entries.is_empty())
    }

    /// Whether submodules follow a pull or a checkout: what the git configuration says in
    /// `submodule.recurse` where it decides, Canopy's choice otherwise.
    pub(crate) fn recurses_into_submodules(&self) -> crate::Result<bool> {
        if self.options().follow_config
            && let Some(entry) =
                crate::config::read_bool(self.workdir(), false, "submodule.recurse")?
        {
            return Ok(entry.value.as_deref() == Some("true"));
        }
        Ok(self.options().recurse_submodules)
    }

    /// The `-c submodule.recurse=false` that goes before a pull, switch or rebase: Canopy brings
    /// the submodules along itself afterwards ([`Repo::follow_submodules`], which reads a
    /// followed `submodule.recurse`), always with `git submodule update`. A recursive switch
    /// drops uncommitted edits inside a submodule to files both submodule commits share; the
    /// update keeps them.
    pub(crate) fn submodule_override() -> Vec<String> {
        vec!["-c".to_owned(), "submodule.recurse=false".to_owned()]
    }

    /// Runs `git switch args…` with [`Self::submodule_override`].
    pub(crate) fn switch_cmd(&self, args: &[&str]) -> crate::Result<String> {
        let o = Self::submodule_override();
        let mut all: Vec<&str> = o.iter().map(String::as_str).collect();
        all.push("switch");
        all.extend(args);
        self.git_cmd(&all)
    }

    /// Runs `git rebase args…` with recursive updates off and the chosen conflict style. Canopy
    /// follows submodules explicitly after the rebase or its continuation finishes.
    pub(crate) fn rebase_cmd(&self, args: &[&str]) -> crate::Result<String> {
        let mut overrides = Self::submodule_override();
        overrides.extend(self.conflict_override());
        let mut all: Vec<&str> = overrides.iter().map(String::as_str).collect();
        all.push("rebase");
        all.extend(args);
        self.git_cmd(&all)
    }

    /// The `-c merge.conflictStyle=…` override that goes before the sub-command of anything that
    /// can write conflicts.
    pub(crate) fn conflict_override(&self) -> Vec<String> {
        if self.config_decides(r"^merge\.conflictstyle$") {
            return Vec::new();
        }
        let style = self.options().conflict_style.value(cli::version_number());
        vec!["-c".to_owned(), format!("merge.conflictStyle={style}")]
    }

    /// The autostash flag for a rebase (`rebase`, read from `rebase.autoStash`) or a merge (read
    /// from `merge.autoStash`). `git merge` and `git pull` without `--rebase` learned it in git
    /// 2.27, so an older or unknown git gets none for a merge and keeps refusing to merge over
    /// changed files, as it always did.
    pub(crate) fn autostash_flag(&self, rebase: bool) -> Option<&'static str> {
        let (key, known) = if rebase {
            (r"^rebase\.autostash$", true)
        } else {
            (
                r"^merge\.autostash$",
                cli::version_number().is_some_and(|v| v >= cli::MERGE_AUTOSTASH_SINCE),
            )
        };
        (known && !self.config_decides(key))
            .then(|| flag(self.options().autostash, "--autostash", "--no-autostash"))
    }

    /// A fetched pull uses merge or rebase directly, so carry pull.autoStash's precedence over
    /// their strategy-specific configuration into an explicit flag. Git before 2.51 does not read
    /// `pull.autoStash`, so neither does Canopy there.
    pub(crate) fn pull_autostash_flag(&self, rebase: bool) -> crate::Result<Option<&'static str>> {
        if !rebase && cli::version_number().is_none_or(|v| v < cli::MERGE_AUTOSTASH_SINCE) {
            return Ok(None);
        }
        if self.options().follow_config
            && cli::version_number().is_some_and(|v| v >= cli::PULL_AUTOSTASH_SINCE)
            && let Some(entry) = crate::config::read_bool(self.workdir(), false, "pull.autoStash")?
        {
            return Ok(Some(flag(
                entry.value.as_deref() == Some("true"),
                "--autostash",
                "--no-autostash",
            )));
        }
        Ok(self.autostash_flag(rebase))
    }

    pub(crate) fn follow_tags_flag(&self) -> Option<&'static str> {
        (!self.config_decides(r"^push\.followtags$")).then(|| {
            flag(
                self.options().follow_tags,
                "--follow-tags",
                "--no-follow-tags",
            )
        })
    }

    /// `--recurse-submodules=check` for a push that publishes commits: git refuses, naming the
    /// submodules whose recorded commits are on no remote, before anything is sent. Without it a
    /// repository with `submodule.recurse` on pushes on demand, which fails where the submodule
    /// has no branch named like the superproject's, and one with it off publishes a commit nobody
    /// can fetch. Nothing where the configuration decides: `push.recurseSubmodules`.
    pub(crate) fn push_submodules_flag(&self) -> Option<&'static str> {
        (!self.config_decides(r"^push\.recursesubmodules$")).then_some("--recurse-submodules=check")
    }

    /// The `-c push.default=…` override for a push without a destination.
    pub(crate) fn push_default_override(&self) -> Vec<String> {
        if self.config_decides(r"^push\.default$") {
            return Vec::new();
        }
        vec![
            "-c".to_owned(),
            format!("push.default={}", self.options().push_default.value()),
        ]
    }

    /// Where a push without a destination sends the current branch: the configuration's
    /// `push.default` while Canopy follows it and it names one branch (`simple`, `upstream` or
    /// `current`), else Canopy's choice.
    pub(crate) fn push_default(&self) -> PushDefault {
        self.configured_value("push.default")
            .and_then(|v| {
                PushDefault::ALL
                    .into_iter()
                    .find(|d| v.eq_ignore_ascii_case(d.value()))
            })
            .unwrap_or(self.options().push_default)
    }

    pub(crate) fn cleanup_flag(&self) -> Option<String> {
        (!self.config_decides(r"^commit\.cleanup$"))
            .then(|| format!("--cleanup={}", self.options().cleanup.value()))
    }

    /// The value the git configuration sets for `key`, while Canopy follows it.
    fn configured_value(&self, key: &str) -> Option<String> {
        self.options()
            .follow_config
            .then(|| self.config_value(key))
            .flatten()
    }

    /// The value the git configuration sets for `key`, as git reads it.
    pub(crate) fn config_value(&self, key: &str) -> Option<String> {
        crate::config::read(self.workdir(), false, crate::config::Query::Get(key))
            .ok()?
            .pop()?
            .value
            .map(|value| value.trim().to_owned())
    }

    /// How a fetch prunes remote-tracking branches (`prune`) and local tags
    /// ([`GitOptions::prune_tags`]), as the arguments that go before the sub-command and after
    /// it. While Canopy follows the configuration its choices only stand in for an unset
    /// `fetch.prune` / `fetch.pruneTags` (`-c`), so each remote's own `remote.<name>.prune` /
    /// `pruneTags` still decides for that remote; otherwise flags decide for every remote.
    pub(crate) fn prune_args(&self, prune: bool) -> (Vec<String>, Vec<&'static str>) {
        let prune_tags = self.options().prune_tags;
        if !self.options().follow_config {
            return (
                Vec::new(),
                vec![
                    flag(prune, "--prune", "--no-prune"),
                    flag(prune_tags, "--prune-tags", "--no-prune-tags"),
                ],
            );
        }
        let mut config = Vec::new();
        for (key, pattern, on) in [
            ("fetch.prune", r"^fetch\.prune$", prune),
            ("fetch.pruneTags", r"^fetch\.prunetags$", prune_tags),
        ] {
            if !self.config_decides(pattern) {
                config.extend(["-c".to_owned(), format!("{key}={on}")]);
            }
        }
        (config, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zdiff3_falls_back_to_diff3_where_git_lacks_it() {
        let cases = [
            (ConflictStyle::Zdiff3, Some((2, 55)), "zdiff3"),
            (ConflictStyle::Zdiff3, Some((2, 35)), "zdiff3"),
            (ConflictStyle::Zdiff3, Some((2, 34)), "diff3"),
            (ConflictStyle::Zdiff3, None, "diff3"),
            (ConflictStyle::Diff3, Some((2, 55)), "diff3"),
            (ConflictStyle::Merge, Some((2, 55)), "merge"),
        ];
        for (style, git, expected) in cases {
            assert_eq!(style.value(git), expected, "{style:?} on {git:?}");
        }
    }

    #[test]
    fn every_style_reads_back_from_its_value() {
        for style in ConflictStyle::ALL {
            assert_eq!(
                ConflictStyle::from_value(style.value(Some((2, 55)))),
                Some(style)
            );
        }
        assert_eq!(
            ConflictStyle::from_value(" ZDiff3 "),
            Some(ConflictStyle::Zdiff3)
        );
        assert_eq!(ConflictStyle::from_value("fancy"), None);
    }
}
