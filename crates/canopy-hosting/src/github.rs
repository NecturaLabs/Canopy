//! GitHub REST API v3 (`https://api.github.com`): pull requests.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{PER_PAGE, is_login, optional, search_words, unix};
use crate::error::{Error, Result, map_head_conflict};
use crate::http::{Http, Method, encode};
use crate::model::{
    Check, CheckState, Comment, ListPage, MergeMethod, NewRequest, RequestDetails, RequestQuery,
    RequestState, RequestSummary, Review, ReviewDecision, Rollup, SearchOrder, StateFilter,
};
use crate::preview::description_preview;

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
    /// The description; the list carries it too.
    body: Option<String>,
    // Single-pull fields only.
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
            description: description_preview(self.body.as_deref().unwrap_or_default()),
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

fn state_name(filter: StateFilter) -> &'static str {
    match filter {
        StateFilter::Open => "open",
        StateFilter::Closed => "closed",
        StateFilter::All => "all",
    }
}

/// One page (1-based) of `per_page` pull requests, most recently updated first.
fn page_of(
    http: &Http,
    repo: &str,
    filter: StateFilter,
    per_page: usize,
    page: usize,
) -> Result<Vec<RequestSummary>> {
    let state = state_name(filter);
    let path = format!(
        "{repo}/pulls?state={state}&sort=updated&direction=desc&per_page={per_page}&page={page}"
    );
    let pulls: Vec<Pull> = http.json(Method::Get, &path, None)?;
    Ok(pulls.iter().map(Pull::summary).collect())
}

pub(crate) fn list(
    http: &Http,
    repo: &str,
    filter: StateFilter,
    limit: usize,
) -> Result<Vec<RequestSummary>> {
    let per_page = limit.clamp(1, PER_PAGE);
    let mut out = Vec::new();
    let mut page = 1;
    while out.len() < limit {
        let rows = page_of(http, repo, filter, per_page, page)?;
        let n = rows.len();
        out.extend(rows);
        if n < per_page {
            break;
        }
        page += 1;
    }
    out.truncate(limit);
    Ok(out)
}

/// Page `page` (1-based) of [`PER_PAGE`] pull requests, and whether a full page means more follow.
pub(crate) fn list_page(
    http: &Http,
    repo: &str,
    filter: StateFilter,
    page: usize,
) -> Result<ListPage> {
    let rows = page_of(http, repo, filter, PER_PAGE, page)?;
    let more = rows.len() == PER_PAGE;
    Ok(ListPage {
        rows,
        more,
        cursor: None,
    })
}

/// The search a page of pull requests comes from: the fields of a row, as a pull request's own
/// listing has them.
const SEARCH_QUERY: &str = "query($q: String!, $first: Int!, $after: String) { \
     search(query: $q, type: ISSUE, first: $first, after: $after) { \
     pageInfo { hasNextPage endCursor } nodes { ... on PullRequest { number title isDraft state \
     createdAt updatedAt url body headRefName baseRefName isCrossRepository \
     author { login } labels(first: 20) { nodes { name } } } } } }";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SearchData {
    search: SearchResult,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SearchResult {
    page_info: PageInfo,
    /// Other kinds of result (an issue) come as empty objects.
    nodes: Vec<Option<SearchPull>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SearchPull {
    number: u64,
    title: String,
    is_draft: bool,
    /// `OPEN`, `CLOSED` or `MERGED`.
    state: String,
    created_at: Option<String>,
    updated_at: Option<String>,
    url: String,
    body: Option<String>,
    head_ref_name: String,
    base_ref_name: String,
    is_cross_repository: bool,
    author: Option<User>,
    labels: LabelNodes,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LabelNodes {
    nodes: Vec<Label>,
}

impl SearchPull {
    fn summary(&self) -> Option<RequestSummary> {
        (self.number != 0).then(|| RequestSummary {
            number: self.number,
            title: self.title.clone(),
            author: login(self.author.as_ref()),
            state: match self.state.as_str() {
                "MERGED" => RequestState::Merged,
                "CLOSED" => RequestState::Closed,
                _ => RequestState::Open,
            },
            draft: self.is_draft,
            source_branch: self.head_ref_name.clone(),
            target_branch: self.base_ref_name.clone(),
            from_fork: self.is_cross_repository,
            created_at: unix(self.created_at.as_deref()),
            updated_at: unix(self.updated_at.as_deref()),
            web_url: self.url.clone(),
            labels: self.labels.nodes.iter().map(|l| l.name.clone()).collect(),
            description: description_preview(self.body.as_deref().unwrap_or_default()),
            rollup: None,
        })
    }
}

/// GitHub's search string for `query` in `repo` (`owner/name`): every word quoted, so it is text
/// and not a qualifier; the author, which can only be a login, as a qualifier.
fn search_string(repo: &str, filter: StateFilter, query: &RequestQuery) -> String {
    let mut q = format!("repo:{repo} is:pr");
    match filter {
        StateFilter::Open => q.push_str(" is:open"),
        StateFilter::Closed => q.push_str(" is:closed"),
        StateFilter::All => {}
    }
    match query.draft {
        Some(true) => q.push_str(" draft:true"),
        Some(false) => q.push_str(" draft:false"),
        None => {}
    }
    if let Some(author) = query.author.as_deref().filter(|a| is_login(a)) {
        let _ = write!(q, " author:{author}");
    }
    let words = search_words(&query.text);
    for word in &words {
        let _ = write!(q, " \"{word}\"");
    }
    if !words.is_empty() {
        q.push_str(" in:title,body");
    }
    q.push_str(match query.order {
        SearchOrder::Updated => " sort:updated-desc",
        SearchOrder::Newest => " sort:created-desc",
        SearchOrder::Oldest => " sort:created-asc",
    });
    q
}

/// A page of the pull requests of `repo` (`owner/name`) matching `query`, by GitHub's search
/// (GraphQL; the REST search lacks the branches). `cursor` continues the page before; a later page
/// without one has nothing to continue from and fails. Search holds at most 1000 results.
pub(crate) fn search_page(
    http: &Http,
    path: &str,
    filter: StateFilter,
    query: &RequestQuery,
    page: usize,
    cursor: Option<&str>,
) -> Result<ListPage> {
    if page > 1 && cursor.is_none() {
        return Err(Error::Parse(
            "a search page needs the cursor of the page before".into(),
        ));
    }
    let variables = json!({
        "q": search_string(path, filter, query),
        "first": PER_PAGE,
        "after": cursor,
    });
    let data: SearchData = http.graphql(SEARCH_QUERY, &variables)?;
    let result = data.search;
    let rows = result
        .nodes
        .iter()
        .flatten()
        .filter_map(SearchPull::summary)
        .collect();
    let more = result.page_info.has_next_page;
    Ok(ListPage {
        rows,
        more,
        cursor: result.page_info.end_cursor.filter(|_| more),
    })
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
