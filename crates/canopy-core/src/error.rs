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
}

impl Error {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }
}
