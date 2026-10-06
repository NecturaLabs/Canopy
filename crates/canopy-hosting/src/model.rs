//! Provider-neutral pull/merge request types.

use canopy_core::forge::ForgeKind;

/// Lifecycle state of a pull or merge request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestState {
    /// Open (GitLab's "opened" and "locked").
    Open,
    /// Closed without merging.
    Closed,
    /// Merged.
    Merged,
}

impl RequestState {
    /// Display name: "Open", "Closed" or "Merged".
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Closed => "Closed",
            Self::Merged => "Merged",
        }
    }
}

/// Which requests [`crate::Client::list`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateFilter {
    /// Open requests only.
    Open,
    /// Closed requests, merged ones included.
    Closed,
    /// Everything.
    All,
}

impl StateFilter {
    /// Every filter, in display order.
    pub const ALL: [Self; 3] = [Self::Open, Self::Closed, Self::All];

    /// Display name: "Open", "Closed" or "All".
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Closed => "Closed",
            Self::All => "All",
        }
    }
}

/// How [`crate::Client::merge`] merges a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MergeMethod {
    /// A merge commit (on GitLab: the project's configured merge method).
    #[default]
    Merge,
    /// Squash the commits into one.
    Squash,
    /// Rebase the commits onto the target branch. On GitLab the source branch is rebased first
    /// and then merged with the project's configured merge method.
    Rebase,
}

impl MergeMethod {
    /// Every method, in display order.
    pub const ALL: [Self; 3] = [Self::Merge, Self::Squash, Self::Rebase];

    /// Display name, as the providers word it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Create a merge commit",
            Self::Squash => "Squash and merge",
            Self::Rebase => "Rebase and merge",
        }
    }

    /// GitHub's `merge_method` value.
    pub(crate) fn github_name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
}

/// Outcome of a CI check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckState {
    /// Queued or running.
    Pending,
    /// Passed.
    Success,
    /// Failed, errored, timed out, was cancelled or needs action.
    Failure,
    /// Neutral, skipped, manual or allowed to fail.
    Neutral,
}

/// One row of a request list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSummary {
    /// GitHub's pull request number, GitLab's merge request iid.
    pub number: u64,
    /// Title.
    pub title: String,
    /// Author's login (GitHub) or username (GitLab).
    pub author: String,
    /// Open, closed or merged.
    pub state: RequestState,
    /// Draft (GitLab: "Draft:" title prefix).
    pub draft: bool,
    /// Branch with the changes.
    pub source_branch: String,
    /// Branch the changes go into.
    pub target_branch: String,
    /// The source branch lives in another repository (a fork). On GitHub a deleted fork also
    /// counts.
    pub from_fork: bool,
    /// Creation time, unix seconds (0 when the server sent none).
    pub created_at: i64,
    /// Last update, unix seconds (0 when the server sent none).
    pub updated_at: i64,
    /// The request's page in the browser.
    pub web_url: String,
    /// Label names.
    pub labels: Vec<String>,
    /// Where the checks and the reviews stand, when the list came with them (see [`Rollup`]);
    /// `None` when they are not known.
    pub rollup: Option<Rollup>,
}

/// What a reviewer scans a request list for: where the request's CI checks and reviews stand.
///
/// [`crate::Client::rollup`] reads it for a whole page of requests in one query; the request
/// list itself carries neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rollup {
    /// The combined state of the checks on the head commit (GitHub's status check rollup,
    /// GitLab's head pipeline); `None` when the head has none.
    pub checks: Option<CheckState>,
    /// The review standing; `None` when nobody has approved or asked for changes and no review
    /// is required.
    pub review: Option<ReviewDecision>,
}

/// Where a request's reviews stand, as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewDecision {
    /// Approved (GitHub: the review decision, or an approval when no review is required;
    /// GitLab: at least one approval).
    Approved,
    /// A reviewer asked for changes (GitHub only).
    ChangesRequested,
    /// The branch rules require a review that has not been given yet (GitHub only).
    Required,
}

/// A CI check on the request's head commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Check run, status context, pipeline or job name.
    pub name: String,
    /// Outcome.
    pub state: CheckState,
    /// Page with details, when the provider gave one.
    pub url: Option<String>,
}

/// A reviewer's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// Reviewer's login or username.
    pub author: String,
    /// "Approved", "Changes requested", "Commented" or "Dismissed".
    pub state: String,
}

/// A conversation comment (not an inline review comment, not a system note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Author's login or username.
    pub author: String,
    /// Markdown body.
    pub body: String,
    /// Creation time, unix seconds.
    pub created_at: i64,
}

/// Everything the request detail view shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestDetails {
    /// The list row.
    pub summary: RequestSummary,
    /// Description, Markdown.
    pub body: String,
    /// Head commit of the source branch.
    pub head_sha: Option<String>,
    /// Whether the request merges without conflicts; `None` while the provider is still
    /// computing it.
    pub mergeable: Option<bool>,
    /// Number of commits (GitHub only).
    pub commits: Option<u32>,
    /// Number of changed files (GitLab reports at least this many when it caps the count).
    pub changed_files: Option<u32>,
    /// Added lines (GitHub only).
    pub additions: Option<u32>,
    /// Deleted lines (GitHub only).
    pub deletions: Option<u32>,
    /// One verdict per reviewer, in order of first review.
    pub reviews: Vec<Review>,
    /// CI checks on the head commit.
    pub checks: Vec<Check>,
    /// Conversation comments, oldest first.
    pub comments: Vec<Comment>,
}

/// What [`crate::Client::create`] opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRequest {
    /// Title.
    pub title: String,
    /// Description, Markdown.
    pub body: String,
    /// Branch with the changes (it must already be pushed to the project).
    pub source: String,
    /// Branch the changes go into.
    pub target: String,
    /// Open as a draft.
    pub draft: bool,
}

/// The full remote ref that holds a request's head commit, for `git fetch <remote> <ref>`:
/// `refs/pull/<n>/head` on GitHub, `refs/merge-requests/<n>/head` on GitLab.
pub fn checkout_ref(kind: ForgeKind, number: u64) -> String {
    match kind {
        ForgeKind::GitHub => format!("refs/pull/{number}/head"),
        ForgeKind::GitLab => format!("refs/merge-requests/{number}/head"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_refs() {
        assert_eq!(checkout_ref(ForgeKind::GitHub, 42), "refs/pull/42/head");
        assert_eq!(
            checkout_ref(ForgeKind::GitLab, 7),
            "refs/merge-requests/7/head"
        );
    }

    #[test]
    fn merge_method_defaults_and_labels() {
        assert_eq!(MergeMethod::default(), MergeMethod::Merge);
        assert_eq!(MergeMethod::ALL.len(), 3);
        assert_eq!(MergeMethod::Squash.label(), "Squash and merge");
        assert_eq!(MergeMethod::Rebase.github_name(), "rebase");
    }
}
