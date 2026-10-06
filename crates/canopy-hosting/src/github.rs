//! GitHub REST API v3 (`https://api.github.com`): pull requests.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{PER_PAGE, optional, unix};
use crate::error::{Error, Result, map_head_conflict};
use crate::http::{Http, Method, encode};
use crate::model::{
    Check, CheckState, Comment, MergeMethod, NewRequest, RequestDetails, RequestState,
    RequestSummary, Review, ReviewDecision, Rollup, StateFilter,
};

/// `/repos/owner/repo`.
pub(crate) fn repo_path(path: &str) -> String {
    let encoded: Vec<String> = path.split('/').map(encode).collect();
    format!("/repos/{}", encoded.join("/"))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct User {
    login: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Repo {
    full_name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Branch {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
    repo: Option<Repo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Label {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Pull {
    number: u64,
    title: String,
    user: Option<User>,
    state: String,
    draft: bool,
    merged_at: Option<String>,
    head: Branch,
    base: Branch,
    created_at: Option<String>,
    updated_at: Option<String>,
    html_url: String,
    labels: Vec<Label>,
    // Single-pull fields only.
    body: Option<String>,
    mergeable: Option<bool>,
    commits: Option<u32>,
    changed_files: Option<u32>,
    additions: Option<u32>,
    deletions: Option<u32>,
}

fn login(user: Option<&User>) -> String {
    user.map(|u| u.login.clone())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "ghost".to_owned())
}

impl Pull {
    fn summary(&self) -> RequestSummary {
        let state = if self.merged_at.is_some() {
            RequestState::Merged
        } else if self.state == "closed" {
            RequestState::Closed
        } else {
            RequestState::Open
        };
        let from_fork = match (&self.head.repo, &self.base.repo) {
            (Some(head), Some(base)) => !head.full_name.eq_ignore_ascii_case(&base.full_name),
            // The fork was deleted.
            (None, _) => true,
            (Some(_), None) => false,
        };
        RequestSummary {
            number: self.number,
            title: self.title.clone(),
            author: login(self.user.as_ref()),
            state,
            draft: self.draft,
            source_branch: self.head.name.clone(),
            target_branch: self.base.name.clone(),
            from_fork,
            created_at: unix(self.created_at.as_deref()),
            updated_at: unix(self.updated_at.as_deref()),
            web_url: self.html_url.clone(),
            labels: self.labels.iter().map(|l| l.name.clone()).collect(),
            rollup: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ReviewJson {
    user: Option<User>,
    state: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CommentJson {
    user: Option<User>,
    body: Option<String>,
    created_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CheckRuns {
    check_runs: Vec<CheckRun>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CheckRun {
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: Option<String>,
    details_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Combined {
    statuses: Vec<Status>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Status {
    context: String,
    state: String,
    target_url: Option<String>,
}

fn check_run_state(status: &str, conclusion: Option<&str>) -> CheckState {
    if status != "completed" {
        return CheckState::Pending;
    }
    match conclusion.unwrap_or_default() {
        "success" => CheckState::Success,
        "neutral" | "skipped" | "stale" => CheckState::Neutral,
        "" => CheckState::Pending,
        // failure, cancelled, timed_out, action_required, startup_failure.
        _ => CheckState::Failure,
    }
}

fn status_state(state: &str) -> CheckState {
    match state {
        "success" => CheckState::Success,
        "failure" | "error" => CheckState::Failure,
        _ => CheckState::Pending,
    }
}

/// One verdict per reviewer, in order of their first review. A later approval, change request
/// or dismissal replaces an earlier verdict; a plain comment never replaces one of those.
fn fold_reviews(raw: Vec<ReviewJson>) -> Vec<Review> {
    let mut out: Vec<Review> = Vec::new();
    for r in raw {
        let state = match r.state.as_str() {
            "APPROVED" => "Approved",
            "CHANGES_REQUESTED" => "Changes requested",
            "COMMENTED" => "Commented",
            "DISMISSED" => "Dismissed",
            _ => continue, // PENDING: the viewer's own unsubmitted review.
        };
        let author = login(r.user.as_ref());
        match out.iter_mut().find(|x| x.author == author) {
            Some(existing) => {
                if state != "Commented" || existing.state == "Commented" {
                    state.clone_into(&mut existing.state);
                }
            }
            None => out.push(Review {
                author,
                state: state.to_owned(),
            }),
        }
    }
    out
}

pub(crate) fn viewer(http: &Http) -> Result<String> {
    let user: User = http.json(Method::Get, "/user", None)?;
    Ok(user.login)
}

pub(crate) fn list(
    http: &Http,
    repo: &str,
    filter: StateFilter,
    limit: usize,
) -> Result<Vec<RequestSummary>> {
    let state = match filter {
        StateFilter::Open => "open",
        StateFilter::Closed => "closed",
        StateFilter::All => "all",
    };
    let per_page = limit.clamp(1, PER_PAGE);
    let mut out = Vec::new();
    let mut page = 1;
    while out.len() < limit {
        let path = format!(
            "{repo}/pulls?state={state}&sort=updated&direction=desc&per_page={per_page}&page={page}"
        );
        let pulls: Vec<Pull> = http.json(Method::Get, &path, None)?;
        let n = pulls.len();
        out.extend(pulls.iter().map(Pull::summary));
        if n < per_page {
            break;
        }
        page += 1;
    }
    out.truncate(limit);
    Ok(out)
}

/// What [`rollup`] reads for each pull request.
const ROLLUP_FRAGMENT: &str = "fragment Rollup on PullRequest { number reviewDecision \
     statusCheckRollup { state } latestOpinionatedReviews(first: 50) { nodes { state } } }";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RollupData {
    /// Pull requests by alias (`p<number>`); a number the repository does not have is `null`.
    repository: Option<HashMap<String, Option<PullRollup>>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PullRollup {
    number: u64,
    review_decision: Option<String>,
    status_check_rollup: Option<StateJson>,
    latest_opinionated_reviews: Option<ReviewNodes>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StateJson {
    state: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ReviewNodes {
    nodes: Vec<Option<StateJson>>,
}

/// The review standing: GitHub's review decision where the branch rules make one, else the
/// reviewers' latest verdicts, where one change request outweighs any number of approvals.
fn review_decision(decision: Option<&str>, opinions: &[&str]) -> Option<ReviewDecision> {
    match decision {
        Some("APPROVED") => Some(ReviewDecision::Approved),
        Some("CHANGES_REQUESTED") => Some(ReviewDecision::ChangesRequested),
        Some("REVIEW_REQUIRED") => Some(ReviewDecision::Required),
        _ if opinions.contains(&"CHANGES_REQUESTED") => Some(ReviewDecision::ChangesRequested),
        _ if opinions.contains(&"APPROVED") => Some(ReviewDecision::Approved),
        _ => None,
    }
}

impl PullRollup {
    fn rollup(&self) -> Rollup {
        let opinions: Vec<&str> = self
            .latest_opinionated_reviews
            .iter()
            .flat_map(|r| r.nodes.iter().flatten())
            .map(|n| n.state.as_str())
            .collect();
        Rollup {
            // SUCCESS, FAILURE, ERROR, PENDING or EXPECTED, as a commit status's state.
            checks: self
                .status_check_rollup
                .as_ref()
                .map(|s| status_state(&s.state.to_ascii_lowercase())),
            review: review_decision(self.review_decision.as_deref(), &opinions),
        }
    }
}

/// The checks and review standing of pull requests `numbers` (at most [`PER_PAGE`], the first
/// ones taken), in one GraphQL query to the same host: each pull request's status check rollup
/// state, review decision and latest reviews. A number the repository does not have is left
/// out.
pub(crate) fn rollup(http: &Http, path: &str, numbers: &[u64]) -> Result<Vec<(u64, Rollup)>> {
    let Some((owner, name)) = path.split_once('/') else {
        return Err(Error::NotFound(format!(
            "not an owner/name repository: {path}"
        )));
    };
    let mut wanted: Vec<u64> = Vec::new();
    // GraphQL's Int is 32 bits; no pull request number is larger.
    for &n in numbers.iter().filter(|&&n| i32::try_from(n).is_ok()) {
        if !wanted.contains(&n) && wanted.len() < PER_PAGE {
            wanted.push(n);
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = String::from(
        "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) {",
    );
    for n in &wanted {
        let _ = write!(query, " p{n}: pullRequest(number: {n}) {{ ...Rollup }}");
    }
    query.push_str(" } } ");
    query.push_str(ROLLUP_FRAGMENT);
    let data: RollupData = http.graphql(&query, &json!({ "owner": owner, "name": name }))?;
    let found = data.repository.unwrap_or_default();
    Ok(wanted
        .into_iter()
        .filter_map(|n| {
            let pull = found.get(&format!("p{n}"))?.as_ref()?;
            (pull.number == n).then(|| (n, pull.rollup()))
        })
        .collect())
}

pub(crate) fn details(http: &Http, repo: &str, number: u64) -> Result<RequestDetails> {
    let pull: Pull = http.json(Method::Get, &format!("{repo}/pulls/{number}"), None)?;
    let summary = pull.summary();
    let head_sha = Some(pull.head.sha.clone()).filter(|s| !s.is_empty());

    let reviews = optional(
        "reviews",
        http.json::<Vec<ReviewJson>>(
            Method::Get,
            &format!("{repo}/pulls/{number}/reviews?per_page={PER_PAGE}"),
            None,
        ),
    )
    .map(fold_reviews)
    .unwrap_or_default();

    let comments = optional(
        "comments",
        http.json::<Vec<CommentJson>>(
            Method::Get,
            &format!("{repo}/issues/{number}/comments?per_page={PER_PAGE}"),
            None,
        ),
    )
    .unwrap_or_default()
    .into_iter()
    .map(|c| Comment {
        author: login(c.user.as_ref()),
        body: c.body.unwrap_or_default(),
        created_at: unix(c.created_at.as_deref()),
    })
    .collect();

    let mut checks = Vec::new();
    if let Some(sha) = &head_sha {
        let sha = encode(sha);
        if let Some(runs) = optional(
            "check runs",
            http.json::<CheckRuns>(
                Method::Get,
                &format!("{repo}/commits/{sha}/check-runs?per_page={PER_PAGE}"),
                None,
            ),
        ) {
            checks.extend(runs.check_runs.into_iter().map(|r| Check {
                state: check_run_state(&r.status, r.conclusion.as_deref()),
                url: r.html_url.or(r.details_url).filter(|u| !u.is_empty()),
                name: r.name,
            }));
        }
        if let Some(combined) = optional(
            "commit statuses",
            http.json::<Combined>(
                Method::Get,
                &format!("{repo}/commits/{sha}/status?per_page={PER_PAGE}"),
                None,
            ),
        ) {
            checks.extend(combined.statuses.into_iter().map(|s| Check {
                state: status_state(&s.state),
                url: s.target_url.filter(|u| !u.is_empty()),
                name: s.context,
            }));
        }
    }

    Ok(RequestDetails {
        summary,
        body: pull.body.unwrap_or_default(),
        head_sha,
        mergeable: pull.mergeable,
        commits: pull.commits,
        changed_files: pull.changed_files,
        additions: pull.additions,
        deletions: pull.deletions,
        reviews,
        checks,
        comments,
    })
}

pub(crate) fn create(http: &Http, repo: &str, req: &NewRequest) -> Result<RequestSummary> {
    let body = json!({
        "title": req.title,
        "body": req.body,
        "head": req.source,
        "base": req.target,
        "draft": req.draft,
    });
    let pull: Pull = http.json(Method::Post, &format!("{repo}/pulls"), Some(&body))?;
    Ok(pull.summary())
}

/// Merges pull request `number`. With `head_sha`, GitHub merges only while the head is still
/// that commit (`sha` in the body) and answers 409 otherwise, which becomes
/// [`crate::error::head_changed`].
pub(crate) fn merge(
    http: &Http,
    repo: &str,
    number: u64,
    method: MergeMethod,
    head_sha: Option<&str>,
) -> Result<()> {
    let mut body = json!({ "merge_method": method.github_name() });
    if let Some(sha) = head_sha {
        body["sha"] = Value::String(sha.to_owned());
    }
    let result = http.call(
        Method::Put,
        &format!("{repo}/pulls/{number}/merge"),
        Some(&body),
    );
    map_head_conflict(result, head_sha.is_some(), "pull request")
}

pub(crate) fn set_open(http: &Http, repo: &str, number: u64, open: bool) -> Result<()> {
    let body = json!({ "state": if open { "open" } else { "closed" } });
    http.call(
        Method::Patch,
        &format!("{repo}/pulls/{number}"),
        Some(&body),
    )
}

pub(crate) fn comment(http: &Http, repo: &str, number: u64, text: &str) -> Result<()> {
    let body = json!({ "body": text });
    http.call(
        Method::Post,
        &format!("{repo}/issues/{number}/comments"),
        Some(&body),
    )
}

pub(crate) fn approve(http: &Http, repo: &str, number: u64) -> Result<()> {
    let body = json!({ "event": "APPROVE" });
    http.call(
        Method::Post,
        &format!("{repo}/pulls/{number}/reviews"),
        Some(&body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(user: &str, state: &str) -> ReviewJson {
        ReviewJson {
            user: Some(User { login: user.into() }),
            state: state.into(),
        }
    }

    #[test]
    fn reviews_fold_per_author() {
        let folded = fold_reviews(vec![
            review("ann", "COMMENTED"),
            review("bob", "CHANGES_REQUESTED"),
            review("ann", "APPROVED"),
            review("ann", "COMMENTED"),
            review("bob", "APPROVED"),
            review("cy", "PENDING"),
        ]);
        assert_eq!(
            folded,
            vec![
                Review {
                    author: "ann".into(),
                    state: "Approved".into()
                },
                Review {
                    author: "bob".into(),
                    state: "Approved".into()
                },
            ]
        );
    }

    #[test]
    fn check_states() {
        assert_eq!(check_run_state("in_progress", None), CheckState::Pending);
        assert_eq!(
            check_run_state("completed", Some("success")),
            CheckState::Success
        );
        assert_eq!(
            check_run_state("completed", Some("timed_out")),
            CheckState::Failure
        );
        assert_eq!(
            check_run_state("completed", Some("skipped")),
            CheckState::Neutral
        );
        assert_eq!(status_state("error"), CheckState::Failure);
        assert_eq!(status_state("pending"), CheckState::Pending);
    }

    #[test]
    fn review_decisions() {
        use ReviewDecision::{Approved, ChangesRequested, Required};
        assert_eq!(review_decision(Some("APPROVED"), &[]), Some(Approved));
        assert_eq!(
            review_decision(Some("CHANGES_REQUESTED"), &["APPROVED"]),
            Some(ChangesRequested)
        );
        // The branch rules speak first: an approval that is not enough still leaves it required.
        assert_eq!(
            review_decision(Some("REVIEW_REQUIRED"), &["APPROVED"]),
            Some(Required)
        );
        // Without rules there is no decision; the reviewers' latest verdicts stand in for it.
        assert_eq!(
            review_decision(None, &["APPROVED", "CHANGES_REQUESTED"]),
            Some(ChangesRequested)
        );
        assert_eq!(
            review_decision(None, &["COMMENTED", "APPROVED"]),
            Some(Approved)
        );
        assert_eq!(review_decision(None, &["COMMENTED", "DISMISSED"]), None);
        assert_eq!(review_decision(Some("SOMETHING_NEW"), &[]), None);
    }

    #[test]
    fn rollup_states() {
        let pull = |state: Option<&str>| PullRollup {
            number: 1,
            status_check_rollup: state.map(|s| StateJson { state: s.into() }),
            ..PullRollup::default()
        };
        let checks = |state| pull(state).rollup().checks;
        assert_eq!(checks(Some("SUCCESS")), Some(CheckState::Success));
        assert_eq!(checks(Some("FAILURE")), Some(CheckState::Failure));
        assert_eq!(checks(Some("ERROR")), Some(CheckState::Failure));
        assert_eq!(checks(Some("PENDING")), Some(CheckState::Pending));
        assert_eq!(checks(Some("EXPECTED")), Some(CheckState::Pending));
        assert_eq!(checks(None), None, "a head without checks has no state");
        let reviewed = PullRollup {
            latest_opinionated_reviews: Some(ReviewNodes {
                nodes: vec![
                    None,
                    Some(StateJson {
                        state: "APPROVED".into(),
                    }),
                ],
            }),
            ..pull(None)
        };
        assert_eq!(
            reviewed.rollup(),
            Rollup {
                checks: None,
                review: Some(ReviewDecision::Approved)
            }
        );
    }

    #[test]
    fn repo_paths() {
        assert_eq!(repo_path("octo/hello.rs"), "/repos/octo/hello.rs");
    }
}
