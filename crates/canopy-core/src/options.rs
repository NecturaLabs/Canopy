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
            Self::Zdiff3 if git.is_some_and(|v| v >= (2, 35)) => "zdiff3",
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
        }
    }
}

fn flag(on: bool, yes: &'static str, no: &'static str) -> &'static str {
    if on { yes } else { no }
}

impl Repo {
    /// Whether the git configuration decides the choice its keys match (`pattern`, a regular
    /// expression over lower-case key names): Canopy follows it and it sets one of them.
    fn config_decides(&self, pattern: &str) -> bool {
        self.options().follow_config
            && cli::run_unchecked(self.workdir(), &["config", "--get-regexp", pattern])
                .is_ok_and(|o| o.code == Some(0))
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
                cli::version_number().is_some_and(|v| v >= (2, 27)),
            )
        };
        (known && !self.config_decides(key))
            .then(|| flag(self.options().autostash, "--autostash", "--no-autostash"))
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

    pub(crate) fn cleanup_flag(&self) -> Option<String> {
        (!self.config_decides(r"^commit\.cleanup$"))
            .then(|| format!("--cleanup={}", self.options().cleanup.value()))
    }

    /// A boolean the git configuration sets, as git reads it, while Canopy follows it.
    fn configured_bool(&self, key: &str) -> Option<bool> {
        if !self.options().follow_config {
            return None;
        }
        let out =
            cli::run_unchecked(self.workdir(), &["config", "--type=bool", "--get", key]).ok()?;
        match (out.code, String::from_utf8_lossy(&out.stdout).trim()) {
            (Some(0), "true") => Some(true),
            (Some(0), "false") => Some(false),
            _ => None,
        }
    }

    /// A fetch's pruning flags for `remote` (every remote when `None`): of remote-tracking
    /// branches as `prune` says, left to `fetch.prune` / `remote.<name>.prune` where the
    /// configuration decides; and of local tags as [`GitOptions::prune_tags`] says, or as
    /// `remote.<name>.pruneTags`, else `fetch.pruneTags`, says while Canopy follows them (for
    /// every remote, only `fetch.pruneTags`). The tags' value is passed even then: the `--tags`
    /// a fetch always has would keep git from reading it.
    pub(crate) fn prune_flags(&self, prune: bool, remote: Option<&str>) -> Vec<&'static str> {
        let mut flags = Vec::new();
        if !self.config_decides(r"^(fetch|remote\..*)\.prune$") {
            flags.push(flag(prune, "--prune", "--no-prune"));
        }
        let prune_tags = remote
            .and_then(|r| self.configured_bool(&format!("remote.{r}.pruneTags")))
            .or_else(|| self.configured_bool("fetch.pruneTags"))
            .unwrap_or(self.options().prune_tags);
        flags.push(flag(prune_tags, "--prune-tags", "--no-prune-tags"));
        flags
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
