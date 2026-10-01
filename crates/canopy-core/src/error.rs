//! Error type shared by every backend operation.

use std::path::PathBuf;

/// Result alias used throughout `canopy-core`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong while reading or changing a repository.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// libgit2 reported an error.
    #[error("{}", .0.message())]
    Git(#[from] git2::Error),

    /// A `git` command line invocation failed. `stderr` holds git's own explanation.
    #[error("git {command} failed: {stderr}")]
    Cli {
        /// The sub-command, e.g. `push`.
        command: String,
        /// Trimmed standard error of the process.
        stderr: String,
    },

    /// The `git` executable could not be started.
    #[error("could not run git ({0}); is git installed and on PATH?")]
    GitMissing(std::io::Error),

    /// The path is not inside a Git repository.
    #[error("{} is not a Git repository", .0.display())]
    NotARepository(PathBuf),

    /// The repository has no working directory (bare repository).
    #[error("bare repositories are not supported yet")]
    Bare,

    /// A file system operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// A precondition the caller should have checked did not hold.
    #[error("{0}")]
    Invalid(String),

    /// A file changed on disk after the caller read it, so writing would discard those changes.
    /// Holds the path relative to the working tree. Nothing was written; reload the file and
    /// retry.
    #[error("{0} changed on disk since it was loaded. Reload it before saving.")]
    ChangedOnDisk(String),

    /// A path was refused before any file was read or written: it is empty, absolute, contains
    /// `..` or a `.git` component, or goes through a symbolic link. Nothing was read or written.
    #[error("{path} {reason}.")]
    UnsafePath {
        /// The path as given (relative to the working tree, or the file Canopy refused).
        path: String,
        /// Why it was refused, e.g. "goes through a symbolic link".
        reason: &'static str,
    },

    /// The repository is a partial clone (`git clone --filter=...`) and the read needs objects
    /// that were not downloaded. Retrying with [`crate::MissingObjects::Download`] fetches them
    /// when `remote` is set and reachable.
    #[error("{}", not_downloaded_message(.remote.as_deref(), .reason.as_deref()))]
    NotDownloaded {
        /// The promisor remote the objects can come from; `None` when no remote is configured
        /// (for example after `git remote remove`), so downloading is impossible.
        remote: Option<String>,
        /// Why downloading failed (git's stderr); `None` when no download was attempted.
        reason: Option<String>,
    },

    /// A partial clone made by Git before 2.24 records its promisor remote in
    /// `extensions.partialClone`, a repository extension libgit2 refuses to open.
    #[error(
        "This partial clone uses the old extensions.partialClone setting, which Canopy cannot \
         open. Git 2.24 and newer record the same thing as remote.{remote}.promisor; to convert \
         it, run `git config remote.{remote}.promisor true` and then \
         `git config --unset extensions.partialClone` in the repository."
    )]
    LegacyPartialClone {
        /// The promisor remote named by `extensions.partialClone`.
        remote: String,
    },
}

fn not_downloaded_message(remote: Option<&str>, reason: Option<&str>) -> String {
    let what = "This content hasn't been downloaded yet (partial clone).";
    match (remote, reason) {
        (Some(r), None) => format!("{what} It can be fetched from {r}."),
        (Some(r), Some(why)) => format!("{what} Fetching it from {r} failed: {why}"),
        (None, Some(why)) => format!("{what} {why}"),
        (None, None) => format!("{what} No promisor remote is configured to fetch it from."),
    }
}

impl Error {
    /// The read failed only because a partial clone has not downloaded what it needs, and
    /// retrying with [`crate::MissingObjects::Download`] can fetch it (a remote is configured).
    pub fn can_download(&self) -> bool {
        matches!(
            self,
            Self::NotDownloaded {
                remote: Some(_),
                ..
            }
        )
    }

    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }
}
