//! Canopy's choices for how git carries out what Canopy asks.
//!
//! Each is passed to git explicitly (a flag, or a `-c name=value` override), so the user's git
//! configuration cannot change the outcome; Settings says where that configuration differs.

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

/// How Canopy has git pull, push, merge, rebase, cherry-pick, revert and apply stashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitOptions {
    /// Pull and rebase with uncommitted changes: git stashes them first and puts them back
    /// afterwards (`--autostash`).
    pub autostash: bool,
    /// A push also sends the annotated tags on the commits it pushes (`--follow-tags`).
    pub follow_tags: bool,
    /// How conflicts are written into files.
    pub conflict_style: ConflictStyle,
}

impl Default for GitOptions {
    /// Today's recommended practice (`docs/adr/0012-git-defaults.md`).
    fn default() -> Self {
        Self {
            autostash: true,
            follow_tags: true,
            conflict_style: ConflictStyle::Zdiff3,
        }
    }
}

impl GitOptions {
    /// The `-c merge.conflictStyle=…` override that goes before the sub-command of anything that
    /// can write conflicts.
    pub(crate) fn conflict_override(self) -> [String; 2] {
        [
            "-c".to_owned(),
            format!(
                "merge.conflictStyle={}",
                self.conflict_style.value(crate::cli::version_number())
            ),
        ]
    }

    pub(crate) fn autostash_flag(self) -> &'static str {
        if self.autostash {
            "--autostash"
        } else {
            "--no-autostash"
        }
    }

    pub(crate) fn follow_tags_flag(self) -> &'static str {
        if self.follow_tags {
            "--follow-tags"
        } else {
            "--no-follow-tags"
        }
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
