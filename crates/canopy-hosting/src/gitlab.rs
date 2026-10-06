//! GitLab REST API v4 (gitlab.com and self-hosted): merge requests.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{PER_PAGE, optional, unix};
use crate::error::{Error, Result, head_changed, map_head_conflict};
use crate::http::{Http, Method, encode, shorten};
use crate::model::{
    Check, CheckState, Comment, MergeMethod, NewRequest, RequestDetails, RequestState,
    RequestSummary, Review, ReviewDecision, Rollup, StateFilter,
};

/// How often a rebase is polled for completion, and how long Canopy waits for it.
const REBASE_POLL: Duration = Duration::from_secs(1);
const REBASE_WAIT: Duration = Duration::from_secs(30);

/// `/projects/<url-encoded path>`: GitLab takes the full path as the project id with every `/`
/// encoded as `%2F`.
pub(crate) fn project_path(path: &str) -> String {
    format!("/projects/{}", encode(path))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct User {
    username: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Pipeline {
    id: u64,
    status: String,
    web_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Mr {
    iid: u64,
    title: String,
    author: Option<User>,
    state: String,
    draft: bool,
    work_in_progress: bool,
    source_branch: String,
    target_branch: String,
    source_project_id: Option<u64>,
    target_project_id: Option<u64>,
    created_at: Option<String>,
    updated_at: Option<String>,
    web_url: String,
    /// Names, or objects with a `name` when `with_labels_details` is set.
    labels: Vec<Value>,
    // Single-MR fields only.
    description: Option<String>,
    sha: Option<String>,
    has_conflicts: bool,
    detailed_merge_status: Option<String>,
    merge_status: Option<String>,
    changes_count: Option<String>,
    head_pipeline: Option<Pipeline>,
    rebase_in_progress: bool,
    merge_error: Option<String>,
}

fn username(user: Option<&User>) -> String {
    user.map(|u| u.username.clone()).unwrap_or_default()
}

impl Mr {
    fn summary(&self) -> RequestSummary {
        let state = match self.state.as_str() {
            "merged" => RequestState::Merged,
            "closed" => RequestState::Closed,
            _ => RequestState::Open, // opened, locked
        };
        let from_fork = match (self.source_project_id, self.target_project_id) {
            (Some(s), Some(t)) => s != t,
            _ => false,
        };
        RequestSummary {
            number: self.iid,
            title: self.title.clone(),
            author: username(self.author.as_ref()),
            state,
            draft: self.draft || self.work_in_progress,
            source_branch: self.source_branch.clone(),
            target_branch: self.target_branch.clone(),
            from_fork,
            created_at: unix(self.created_at.as_deref()),
            updated_at: unix(self.updated_at.as_deref()),
            web_url: self.web_url.clone(),
            labels: self
                .labels
                .iter()
                .filter_map(|l| match l {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(o) => o.get("name").and_then(Value::as_str).map(str::to_owned),
                    _ => None,
                })
                .collect(),
            rollup: None,
        }
    }

    /// Conflict-free, like GitHub's `mergeable`; `None` while GitLab is still checking.
    fn mergeable(&self) -> Option<bool> {
        if self.has_conflicts {
            return Some(false);
        }
        let status = self
            .detailed_merge_status
            .as_deref()
            .or(self.merge_status.as_deref())
            .unwrap_or_default();
        match status {
            "" | "unchecked" | "checking" | "preparing" | "cannot_be_merged_recheck" => None,
            "conflict" | "broken_status" | "cannot_be_merged" => Some(false),
            _ => Some(true),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Approvals {
    approved_by: Vec<Approver>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Approver {
    user: Option<User>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Note {
    author: Option<User>,
    body: String,
    created_at: Option<String>,
    system: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Job {
    name: String,
    stage: String,
    status: String,
    web_url: Option<String>,
    allow_failure: bool,
}

fn pipeline_state(status: &str, allow_failure: bool) -> CheckState {
    match status {
        "success" => CheckState::Success,
        "failed" if allow_failure => CheckState::Neutral,
        "failed" | "canceled" | "canceling" => CheckState::Failure,
        "skipped" | "manual" => CheckState::Neutral,
        // created, waiting_for_resource, preparing, pending, running, scheduled, ...
        _ => CheckState::Pending,
    }
}

pub(crate) fn viewer(http: &Http) -> Result<String> {
    let user: User = http.json(Method::Get, "/user", None)?;
    Ok(user.username)
}

fn list_state(
    http: &Http,
    project: &str,
    state: &str,
    limit: usize,
) -> Result<Vec<RequestSummary>> {
    let per_page = limit.clamp(1, PER_PAGE);
    let mut out = Vec::new();
    let mut page = 1;
    while out.len() < limit {
        let path = format!(
            "{project}/merge_requests?state={state}&order_by=updated_at&sort=desc&per_page={per_page}&page={page}"
        );
        let mrs: Vec<Mr> = http.json(Method::Get, &path, None)?;
        let n = mrs.len();
        out.extend(mrs.iter().map(Mr::summary));
        if n < per_page {
            break;
        }
        page += 1;
    }
    out.truncate(limit);
    Ok(out)
}

pub(crate) fn list(
    http: &Http,
    project: &str,
    filter: StateFilter,
    limit: usize,
) -> Result<Vec<RequestSummary>> {
    match filter {
        StateFilter::Open => list_state(http, project, "opened", limit),
        StateFilter::All => list_state(http, project, "all", limit),
        StateFilter::Closed => {
            // GitLab keeps "closed" and "merged" apart; fetch both and interleave by update time.
            let mut out = list_state(http, project, "closed", limit)?;
            out.extend(list_state(http, project, "merged", limit)?);
            out.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
            out.truncate(limit);
            Ok(out)
        }
    }
}

/// What [`rollup`] asks for: each merge request's head pipeline status and whether anyone
/// approved it. Only fields every GitLab edition has (`approvalsLeft` is Premium only, and
/// `approved` means "has an approval" without approval rules, so neither can say a review is
/// still required).
const ROLLUP_QUERY: &str = "query($path: ID!, $iids: [String!]) { project(fullPath: $path) { \
     mergeRequests(iids: $iids, first: 100) { nodes { iid headPipeline { status } \
     approvedBy(first: 1) { nodes { username } } } } } }";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RollupData {
    project: Option<RollupProject>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct RollupProject {
    merge_requests: Option<RollupNodes<MrRollup>>,
}

#[derive(Debug, Deserialize)]
struct RollupNodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

impl<T> Default for RollupNodes<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MrRollup {
    /// A string in GitLab's GraphQL API.
    iid: String,
    head_pipeline: Option<PipelineStatus>,
    approved_by: Option<RollupNodes<Value>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PipelineStatus {
    /// `SUCCESS`, `FAILED`, `RUNNING`, …: the REST status, upper-cased.
    status: String,
}

impl MrRollup {
    fn rollup(&self) -> Rollup {
        let approved = self
            .approved_by
            .as_ref()
            .is_some_and(|a| a.nodes.iter().any(Option::is_some));
        Rollup {
            checks: self
                .head_pipeline
                .as_ref()
                .map(|p| pipeline_state(&p.status.to_ascii_lowercase(), false)),
            review: approved.then_some(ReviewDecision::Approved),
        }
    }
}

/// The checks and review standing of merge requests `iids` (at most [`PER_PAGE`], the first
/// ones taken), in one GraphQL query to the same host's `/api/graphql`: each head pipeline's
/// status and whether it has an approval. An iid the project does not have is left out.
pub(crate) fn rollup(http: &Http, path: &str, iids: &[u64]) -> Result<Vec<(u64, Rollup)>> {
    let mut wanted: Vec<u64> = Vec::new();
    for &n in iids {
        if !wanted.contains(&n) && wanted.len() < PER_PAGE {
            wanted.push(n);
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let variables = json!({
        "path": path,
        "iids": wanted.iter().map(u64::to_string).collect::<Vec<_>>(),
    });
    let data: RollupData = http.graphql(ROLLUP_QUERY, &variables)?;
    let found: Vec<(u64, Rollup)> = data
        .project
        .and_then(|p| p.merge_requests)
        .unwrap_or_default()
        .nodes
        .into_iter()
        .flatten()
        .filter_map(|mr| Some((mr.iid.parse::<u64>().ok()?, mr.rollup())))
        .collect();
    Ok(wanted
        .into_iter()
        .filter_map(|n| found.iter().find(|(iid, _)| *iid == n).copied())
        .collect())
}

pub(crate) fn details(http: &Http, project: &str, iid: u64) -> Result<RequestDetails> {
    let mr_path = format!("{project}/merge_requests/{iid}");
    let mr: Mr = http.json(Method::Get, &mr_path, None)?;
    let summary = mr.summary();

    let reviews = optional(
        "approvals",
        http.json::<Approvals>(Method::Get, &format!("{mr_path}/approvals"), None),
    )
    .map(|a| {
        a.approved_by
            .iter()
            .map(|x| Review {
                author: username(x.user.as_ref()),
                state: "Approved".to_owned(),
            })
            .collect()
    })
    .unwrap_or_default();

    let comments = optional(
        "notes",
        http.json::<Vec<Note>>(
            Method::Get,
            &format!("{mr_path}/notes?sort=asc&order_by=created_at&per_page={PER_PAGE}"),
            None,
        ),
    )
    .unwrap_or_default()
    .into_iter()
    .filter(|n| !n.system)
    .map(|n| Comment {
        author: username(n.author.as_ref()),
        body: n.body,
        created_at: unix(n.created_at.as_deref()),
    })
    .collect();

    let mut checks = Vec::new();
    if let Some(p) = &mr.head_pipeline {
        checks.push(Check {
            name: format!("Pipeline #{}", p.id),
            state: pipeline_state(&p.status, false),
            url: p.web_url.clone().filter(|u| !u.is_empty()),
        });
        if let Some(jobs) = optional(
            "pipeline jobs",
            http.json::<Vec<Job>>(
                Method::Get,
                &format!("{project}/pipelines/{}/jobs?per_page={PER_PAGE}", p.id),
                None,
            ),
        ) {
            checks.extend(jobs.into_iter().map(|j| Check {
                state: pipeline_state(&j.status, j.allow_failure),
                url: j.web_url.filter(|u| !u.is_empty()),
                name: if j.stage.is_empty() {
                    j.name
                } else {
                    format!("{}: {}", j.stage, j.name)
                },
            }));
        }
    }

    let changed_files = mr
        .changes_count
        .as_deref()
        .and_then(|c| c.trim_end_matches('+').parse::<u32>().ok());
    Ok(RequestDetails {
        summary,
        body: mr.description.clone().unwrap_or_default(),
        head_sha: mr.sha.clone().filter(|s| !s.is_empty()),
        mergeable: mr.mergeable(),
        commits: None,
        changed_files,
        additions: None,
        deletions: None,
        reviews,
        checks,
        comments,
    })
}

/// GitLab marks drafts with a title prefix.
fn draft_title(title: &str, draft: bool) -> String {
    let lower = title.trim_start().to_ascii_lowercase();
    let marked = ["draft:", "[draft]", "(draft)"]
        .iter()
        .any(|p| lower.starts_with(p));
    if draft && !marked {
        format!("Draft: {title}")
    } else {
        title.to_owned()
    }
}

pub(crate) fn create(http: &Http, project: &str, req: &NewRequest) -> Result<RequestSummary> {
    let body = json!({
        "source_branch": req.source,
        "target_branch": req.target,
        "title": draft_title(&req.title, req.draft),
        "description": req.body,
    });
    let mr: Mr = http.json(
        Method::Post,
        &format!("{project}/merge_requests"),
        Some(&body),
    )?;
    Ok(mr.summary())
}

/// Merges `iid`. `Squash` sets `squash`; `Merge` and `Rebase` use the project's configured merge
/// method (merge commit, semi-linear or fast-forward). `Rebase` first rebases the source branch
/// onto the target (GitLab does this asynchronously, so Canopy polls for up to 30 seconds) and
/// then merges exactly the rebased head.
///
/// With `head_sha`, the merge happens only while the source branch head is still that commit:
/// it is sent as `sha`, which GitLab checks and answers 409 ("SHA does not match HEAD of source
/// branch") when it does not match, mapped to [`crate::error::head_changed`]. For `Rebase` the
/// head is compared before GitLab is asked to rebase, and the merge then names the rebased head.
pub(crate) fn merge(
    http: &Http,
    project: &str,
    iid: u64,
    method: MergeMethod,
    head_sha: Option<&str>,
) -> Result<()> {
    let mr_path = format!("{project}/merge_requests/{iid}");
    let mut body = json!({ "squash": method == MergeMethod::Squash });
    let sha = if method == MergeMethod::Rebase {
        rebase(http, &mr_path, head_sha)?
    } else {
        head_sha.map(str::to_owned)
    };
    let sha_sent = sha.is_some();
    if let Some(sha) = sha {
        body["sha"] = Value::String(sha);
    }
    let result = http.call(Method::Put, &format!("{mr_path}/merge"), Some(&body));
    map_head_conflict(result, sha_sent, "merge request")
}

/// Rebases the source branch and waits for it; returns the new head. With `expected`, refuses
/// (before rebasing) when the head is no longer that commit.
fn rebase(http: &Http, mr_path: &str, expected: Option<&str>) -> Result<Option<String>> {
    let poll_path = format!("{mr_path}?include_rebase_in_progress=true");
    let before: Mr = http.json(Method::Get, &poll_path, None)?;
    if let Some(expected) = expected
        && before.sha.as_deref() != Some(expected)
    {
        return Err(head_changed("merge request"));
    }
    http.call(Method::Put, &format!("{mr_path}/rebase"), None)?;
    let deadline = Instant::now() + REBASE_WAIT;
    loop {
        let now: Mr = http.json(Method::Get, &poll_path, None)?;
        if !now.rebase_in_progress {
            let error = now.merge_error.as_deref().unwrap_or_default().trim();
            // A merge_error left over from an earlier attempt is stale once the head moved.
            if !error.is_empty() && now.sha == before.sha {
                return Err(Error::Api {
                    status: 409,
                    message: shorten(&format!(
                        "GitLab could not rebase the merge request: {error}"
                    )),
                });
            }
            return Ok(now.sha.filter(|s| !s.is_empty()));
        }
        if Instant::now() >= deadline {
            return Err(Error::Api {
                status: 202,
                message: "GitLab is still rebasing the merge request; merge it again once the \
                          rebase has finished"
                    .to_owned(),
            });
        }
        std::thread::sleep(REBASE_POLL);
    }
}

pub(crate) fn set_open(http: &Http, project: &str, iid: u64, open: bool) -> Result<()> {
    let body = json!({ "state_event": if open { "reopen" } else { "close" } });
    http.call(
        Method::Put,
        &format!("{project}/merge_requests/{iid}"),
        Some(&body),
    )
}

pub(crate) fn comment(http: &Http, project: &str, iid: u64, text: &str) -> Result<()> {
    let body = json!({ "body": text });
    http.call(
        Method::Post,
        &format!("{project}/merge_requests/{iid}/notes"),
        Some(&body),
    )
}

pub(crate) fn approve(http: &Http, project: &str, iid: u64) -> Result<()> {
    http.call(
        Method::Post,
        &format!("{project}/merge_requests/{iid}/approve"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_ids_are_url_encoded() {
        assert_eq!(project_path("group/repo"), "/projects/group%2Frepo");
        assert_eq!(
            project_path("group/sub.group/my-project"),
            "/projects/group%2Fsub.group%2Fmy-project"
        );
    }

    #[test]
    fn draft_titles() {
        assert_eq!(draft_title("Fix it", true), "Draft: Fix it");
        assert_eq!(draft_title("Draft: Fix it", true), "Draft: Fix it");
        assert_eq!(draft_title("[Draft] Fix it", true), "[Draft] Fix it");
        assert_eq!(draft_title("Fix it", false), "Fix it");
    }

    #[test]
    fn pipeline_states() {
        assert_eq!(pipeline_state("running", false), CheckState::Pending);
        assert_eq!(pipeline_state("failed", true), CheckState::Neutral);
        assert_eq!(pipeline_state("failed", false), CheckState::Failure);
        assert_eq!(pipeline_state("manual", false), CheckState::Neutral);
        assert_eq!(pipeline_state("success", false), CheckState::Success);
    }

    #[test]
    fn rollup_from_pipeline_and_approvals() {
        let mr = |status: Option<&str>, approvers: usize| MrRollup {
            iid: "4".into(),
            head_pipeline: status.map(|s| PipelineStatus { status: s.into() }),
            approved_by: Some(RollupNodes {
                nodes: (0..approvers)
                    .map(|_| Some(json!({"username": "x"})))
                    .collect(),
            }),
        };
        let checks = |status| mr(status, 0).rollup().checks;
        assert_eq!(checks(Some("SUCCESS")), Some(CheckState::Success));
        assert_eq!(checks(Some("FAILED")), Some(CheckState::Failure));
        assert_eq!(checks(Some("CANCELED")), Some(CheckState::Failure));
        assert_eq!(checks(Some("RUNNING")), Some(CheckState::Pending));
        assert_eq!(
            checks(Some("WAITING_FOR_CALLBACK")),
            Some(CheckState::Pending)
        );
        assert_eq!(checks(Some("MANUAL")), Some(CheckState::Neutral));
        assert_eq!(checks(None), None);
        assert_eq!(mr(None, 0).rollup().review, None);
        assert_eq!(mr(None, 1).rollup().review, Some(ReviewDecision::Approved));
        let unseen = MrRollup {
            approved_by: None,
            ..mr(None, 0)
        };
        assert_eq!(unseen.rollup(), Rollup::default());
    }

    #[test]
    fn mergeable_from_status() {
        let mut mr = Mr {
            detailed_merge_status: Some("mergeable".into()),
            ..Mr::default()
        };
        assert_eq!(mr.mergeable(), Some(true));
        mr.detailed_merge_status = Some("not_approved".into());
        assert_eq!(mr.mergeable(), Some(true));
        mr.detailed_merge_status = Some("checking".into());
        assert_eq!(mr.mergeable(), None);
        mr.has_conflicts = true;
        assert_eq!(mr.mergeable(), Some(false));
    }
}
