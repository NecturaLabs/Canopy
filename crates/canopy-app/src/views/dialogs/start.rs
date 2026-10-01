//! Where a new branch starts.

use super::*;

/// Where a new branch starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// The current `HEAD`.
    Head,
    /// A commit, e.g. from the history's context menu.
    Commit(Oid),
    /// A branch (or another revision) by name.
    Branch(String),
}

impl Start {
    /// The revision to hand to git; `None` for `HEAD`.
    pub fn rev(&self) -> Option<String> {
        match self {
            Start::Head => None,
            Start::Commit(oid) => Some(oid.to_string()),
            Start::Branch(name) => Some(name.clone()),
        }
    }

    /// How the dialog names it: `commit 1a2b3c4`, the branch name, or `head()` for `HEAD`.
    pub(super) fn label(&self, head: impl FnOnce() -> String) -> String {
        match self {
            Start::Head => head(),
            Start::Commit(oid) => format!("commit {}", short_id(*oid)),
            Start::Branch(name) => name.clone(),
        }
    }
}
