//! End-to-end tests of the GitHub and GitLab clients against a canned local HTTP server.
//! Nothing here touches the network: the server listens on 127.0.0.1 and answers from a table.

#![allow(clippy::expect_used, clippy::unwrap_used)] // Test helpers may panic: that is a failed test.

use canopy_core::forge::{Forge, ForgeKind};
use canopy_hosting::{
    CheckState, Client, Error, MergeMethod, NewRequest, RequestState, StateFilter,
};
use serde_json::{Value, json};

mod server {
    //! A tiny HTTP/1.1 server: one request per connection, responses keyed by method and target.

    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    pub struct Canned {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    #[derive(Debug, Clone)]
    pub struct Recorded {
        pub method: String,
        /// Path and query, as sent.
        pub target: String,
        /// Header names lower-cased.
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Recorded {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_str(&self.body).expect("request body is JSON")
        }
    }

    type Routes = Arc<Mutex<HashMap<(String, String), Canned>>>;

    #[derive(Debug)]
    pub struct Server {
        pub base: String,
        routes: Routes,
        log: Arc<Mutex<Vec<Recorded>>>,
    }

    impl Server {
        pub fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let base = format!("http://{}", listener.local_addr().expect("addr"));
            let routes: Routes = Arc::default();
            let log: Arc<Mutex<Vec<Recorded>>> = Arc::default();
            let (r, l) = (routes.clone(), log.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    handle(stream, &r, &l);
                }
            });
            Self { base, routes, log }
        }

        /// Answers `method target` with `status` and a JSON body. A target without `?` matches
        /// any query; one with `?` matches exactly (and wins).
        pub fn route(&self, method: &str, target: &str, status: u16, body: &str) {
            self.route_with_headers(method, target, status, &[], body);
        }

        pub fn route_with_headers(
            &self,
            method: &str,
            target: &str,
            status: u16,
            headers: &[(&str, &str)],
            body: &str,
        ) {
            self.routes.lock().expect("routes").insert(
                (method.to_owned(), target.to_owned()),
                Canned {
                    status,
                    headers: headers
                        .iter()
                        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                        .collect(),
                    body: body.to_owned(),
                },
            );
        }

        pub fn requests(&self) -> Vec<Recorded> {
            self.log.lock().expect("log").clone()
        }

        /// Requests other than GETs.
        pub fn writes(&self) -> Vec<Recorded> {
            self.requests()
                .into_iter()
                .filter(|r| r.method != "GET")
                .collect()
        }
    }

    fn handle(stream: TcpStream, routes: &Routes, log: &Arc<Mutex<Vec<Recorded>>>) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let target = parts.next().unwrap_or_default().to_owned();
        let mut headers = Vec::new();
        let mut length = 0usize;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 {
                break;
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
                if k == "content-length" {
                    length = v.parse().unwrap_or(0);
                }
                headers.push((k, v));
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).expect("read body");
        log.lock().expect("log").push(Recorded {
            method: method.clone(),
            target: target.clone(),
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        });

        let path = target.split('?').next().unwrap_or_default().to_owned();
        let canned = {
            let routes = routes.lock().expect("routes");
            routes
                .get(&(method.clone(), target.clone()))
                .or_else(|| routes.get(&(method, path)))
                .cloned()
        }
        .unwrap_or(Canned {
            status: 404,
            headers: Vec::new(),
            body: r#"{"message":"Not Found"}"#.to_owned(),
        });
        let mut out = format!(
            "HTTP/1.1 {} Canned\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            canned.status,
            canned.body.len()
        );
        for (k, v) in &canned.headers {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        out.push_str("\r\n");
        out.push_str(&canned.body);
        let mut stream = stream;
        let _ = stream.write_all(out.as_bytes());
        let _ = stream.flush();
    }
}

use server::Server;

const TOKEN: &str = "tok-SECRET-1234567890";

fn github(server: &Server, token: Option<&str>) -> Client {
    let forge = Forge {
        kind: ForgeKind::GitHub,
        host: "github.com".into(),
        port: None,
        path: "octo/hello".into(),
    };
    Client::with_api_base(&forge, token.map(str::to_owned), &server.base)
}

fn gitlab(server: &Server, token: Option<&str>) -> Client {
    let forge = Forge {
        kind: ForgeKind::GitLab,
        host: "gitlab.example.com".into(),
        port: None,
        path: "group/sub/proj".into(),
    };
    Client::with_api_base(
        &forge,
        token.map(str::to_owned),
        &format!("{}/api/v4", server.base),
    )
}

const GL: &str = "/api/v4/projects/group%2Fsub%2Fproj";

fn gh_pull(number: u64, state: &str, merged: bool, updated: &str) -> Value {
    json!({
        "number": number,
        "title": format!("PR {number}"),
        "user": { "login": "ann" },
        "state": state,
        "draft": number == 2,
        "merged_at": if merged { json!("2024-01-03T00:00:00Z") } else { Value::Null },
        "head": { "ref": format!("feature-{number}"), "sha": "abc123", "repo": { "full_name": if number == 3 { "fork/hello" } else { "octo/hello" } } },
        "base": { "ref": "main", "sha": "def456", "repo": { "full_name": "octo/hello" } },
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": updated,
        "html_url": format!("https://github.com/octo/hello/pull/{number}"),
        "labels": [{ "name": "bug" }, { "name": "ui" }],
        "unknown_field": { "ignored": true },
    })
}

fn gl_mr(iid: u64, state: &str, updated: &str) -> Value {
    json!({
        "iid": iid,
        "id": 1000 + iid,
        "title": format!("MR {iid}"),
        "author": { "username": "bob" },
        "state": state,
        "draft": false,
        "work_in_progress": iid == 2,
        "source_branch": format!("topic-{iid}"),
        "target_branch": "main",
        "source_project_id": if iid == 3 { 99 } else { 7 },
        "target_project_id": 7,
        "created_at": "2024-02-01T10:00:00.000+01:00",
        "updated_at": updated,
        "web_url": format!("https://gitlab.example.com/group/sub/proj/-/merge_requests/{iid}"),
        "labels": ["backend"],
    })
}

fn assert_no_token_leak(client: &Client) {
    let dbg = format!("{client:?}");
    assert!(!dbg.contains(TOKEN), "token in Debug output: {dbg}");
    assert!(!dbg.contains("SECRET"), "token in Debug output: {dbg}");
}

// ----------------------------------------------------------------------------------------------
// GitHub
// ----------------------------------------------------------------------------------------------

#[test]
fn github_list_maps_fields() {
    let server = Server::start();
    let page: Vec<Value> = (1..=3)
        .map(|n| gh_pull(n, "open", false, "2024-01-05T00:00:00Z"))
        .collect();
    server.route(
        "GET",
        "/repos/octo/hello/pulls?state=all&sort=updated&direction=desc&per_page=3&page=1",
        200,
        &Value::Array(page).to_string(),
    );
    let client = github(&server, Some(TOKEN));
    let all = client.list(StateFilter::All, 3).expect("list");
    assert_eq!(
        server.requests().len(),
        1,
        "a full first page satisfies the limit"
    );
    assert_eq!(all.iter().map(|s| s.number).collect::<Vec<_>>(), [1, 2, 3]);

    let first = &all[0];
    assert_eq!(first.title, "PR 1");
    assert_eq!(first.author, "ann");
    assert_eq!(first.state, RequestState::Open);
    assert!(!first.draft);
    assert!(all[1].draft);
    assert_eq!(first.source_branch, "feature-1");
    assert_eq!(first.target_branch, "main");
    assert!(!first.from_fork);
    assert!(all[2].from_fork);
    assert_eq!(first.created_at, 1_704_067_200);
    assert_eq!(first.updated_at, 1_704_412_800);
    assert_eq!(first.web_url, "https://github.com/octo/hello/pull/1");
    assert_eq!(first.labels, ["bug", "ui"]);

    let req = &server.requests()[0];
    assert_eq!(
        req.header("authorization"),
        Some(&*format!("Bearer {TOKEN}"))
    );
    assert_eq!(req.header("accept"), Some("application/vnd.github+json"));
    assert_eq!(req.header("x-github-api-version"), Some("2022-11-28"));
    assert!(
        req.header("user-agent")
            .is_some_and(|ua| ua.starts_with("Canopy/")),
        "{:?}",
        req.header("user-agent")
    );
    assert!(!req.target.contains(TOKEN));
    assert_no_token_leak(&client);
}

#[test]
fn github_list_follows_pages_until_short_page() {
    let server = Server::start();
    // limit 250 → per_page 100; page 1 full, page 2 short.
    let full: Vec<Value> = (1..=100)
        .map(|n| gh_pull(n, "closed", n % 2 == 0, "2024-01-05T00:00:00Z"))
        .collect();
    let short: Vec<Value> = (101..=110)
        .map(|n| gh_pull(n, "closed", false, "2024-01-04T00:00:00Z"))
        .collect();
    let base = "/repos/octo/hello/pulls?state=closed&sort=updated&direction=desc&per_page=100";
    server.route(
        "GET",
        &format!("{base}&page=1"),
        200,
        &Value::Array(full).to_string(),
    );
    server.route(
        "GET",
        &format!("{base}&page=2"),
        200,
        &Value::Array(short).to_string(),
    );
    let list = github(&server, None)
        .list(StateFilter::Closed, 250)
        .expect("list");
    assert_eq!(list.len(), 110);
    assert_eq!(server.requests().len(), 2);
    assert_eq!(list[0].state, RequestState::Closed);
    assert_eq!(list[1].state, RequestState::Merged);
    assert!(
        server.requests()[0].header("authorization").is_none(),
        "no token, no Authorization header"
    );

    // A limit below a page size truncates.
    let server = Server::start();
    let items: Vec<Value> = (1..=5)
        .map(|n| gh_pull(n, "open", false, "2024-01-05T00:00:00Z"))
        .collect();
    server.route(
        "GET",
        "/repos/octo/hello/pulls",
        200,
        &Value::Array(items).to_string(),
    );
    let list = github(&server, None)
        .list(StateFilter::Open, 2)
        .expect("list");
    assert_eq!(list.len(), 2);
    assert!(server.requests()[0].target.contains("state=open"));
    assert!(server.requests()[0].target.contains("per_page=2"));
}

#[test]
fn github_details_collects_parts_and_tolerates_failures() {
    let server = Server::start();
    let mut pull = gh_pull(5, "open", false, "2024-01-05T00:00:00Z");
    pull["body"] = json!("Fixes the thing.");
    pull["mergeable"] = json!(true);
    pull["commits"] = json!(3);
    pull["changed_files"] = json!(4);
    pull["additions"] = json!(10);
    pull["deletions"] = json!(2);
    server.route("GET", "/repos/octo/hello/pulls/5", 200, &pull.to_string());
    server.route(
        "GET",
        "/repos/octo/hello/pulls/5/reviews",
        200,
        &json!([
            { "user": { "login": "cy" }, "state": "COMMENTED" },
            { "user": { "login": "cy" }, "state": "APPROVED" },
            { "user": { "login": "dee" }, "state": "CHANGES_REQUESTED" },
        ])
        .to_string(),
    );
    server.route(
        "GET",
        "/repos/octo/hello/issues/5/comments",
        200,
        &json!([
            { "user": { "login": "dee" }, "body": "Looks good", "created_at": "2024-01-02T00:00:00Z" },
            { "user": null, "body": "From a deleted account", "created_at": "2024-01-02T01:00:00Z" },
        ])
        .to_string(),
    );
    // The token lacks the checks scope.
    server.route(
        "GET",
        "/repos/octo/hello/commits/abc123/check-runs",
        403,
        r#"{"message":"Resource not accessible by personal access token"}"#,
    );
    server.route(
        "GET",
        "/repos/octo/hello/commits/abc123/status",
        200,
        &json!({ "state": "failure", "statuses": [
            { "context": "ci/lint", "state": "success", "target_url": "https://ci.example/1" },
            { "context": "ci/test", "state": "error", "target_url": "" },
            { "context": "ci/slow", "state": "pending" },
        ]})
        .to_string(),
    );

    let d = github(&server, Some(TOKEN)).details(5).expect("details");
    assert_eq!(d.summary.number, 5);
    assert_eq!(d.body, "Fixes the thing.");
    assert_eq!(d.head_sha.as_deref(), Some("abc123"));
    assert_eq!(d.mergeable, Some(true));
    assert_eq!(
        (d.commits, d.changed_files, d.additions, d.deletions),
        (Some(3), Some(4), Some(10), Some(2))
    );
    assert_eq!(d.reviews.len(), 2);
    assert_eq!(
        (d.reviews[0].author.as_str(), d.reviews[0].state.as_str()),
        ("cy", "Approved")
    );
    assert_eq!(d.reviews[1].state, "Changes requested");
    assert_eq!(d.comments.len(), 2);
    assert_eq!(d.comments[0].author, "dee");
    assert_eq!(d.comments[0].body, "Looks good");
    assert_eq!(d.comments[0].created_at, 1_704_153_600);
    assert_eq!(d.comments[1].author, "ghost");
    let checks: Vec<(&str, CheckState, Option<&str>)> = d
        .checks
        .iter()
        .map(|c| (c.name.as_str(), c.state, c.url.as_deref()))
        .collect();
    assert_eq!(
        checks,
        [
            ("ci/lint", CheckState::Success, Some("https://ci.example/1")),
            ("ci/test", CheckState::Failure, None),
            ("ci/slow", CheckState::Pending, None),
        ]
    );

    // Check runs when they are readable.
    server.route(
        "GET",
        "/repos/octo/hello/commits/abc123/check-runs",
        200,
        &json!({ "total_count": 2, "check_runs": [
            { "name": "build", "status": "completed", "conclusion": "success", "html_url": "https://github.com/octo/hello/runs/1" },
            { "name": "test", "status": "in_progress", "conclusion": null, "details_url": "https://ci.example/t" },
        ]})
        .to_string(),
    );
    let d = github(&server, Some(TOKEN)).details(5).expect("details");
    assert_eq!(d.checks.len(), 5);
    assert_eq!(d.checks[0].name, "build");
    assert_eq!(d.checks[0].state, CheckState::Success);
    assert_eq!(d.checks[1].state, CheckState::Pending);
    assert_eq!(d.checks[1].url.as_deref(), Some("https://ci.example/t"));
}

#[test]
fn github_details_main_request_failure_is_an_error() {
    let server = Server::start();
    let err = github(&server, Some(TOKEN)).details(77).expect_err("404");
    assert_eq!(err, Error::NotFound("Not Found".into()));
    assert!(err.to_string().contains("token cannot see this repository"));
}

#[test]
fn github_writes() {
    let server = Server::start();
    server.route(
        "POST",
        "/repos/octo/hello/pulls",
        201,
        &gh_pull(9, "open", false, "2024-01-05T00:00:00Z").to_string(),
    );
    server.route(
        "PUT",
        "/repos/octo/hello/pulls/9/merge",
        200,
        r#"{"merged":true,"sha":"x"}"#,
    );
    server.route("PATCH", "/repos/octo/hello/pulls/9", 200, "{}");
    server.route("POST", "/repos/octo/hello/issues/9/comments", 201, "{}");
    server.route("POST", "/repos/octo/hello/pulls/9/reviews", 200, "{}");
    let client = github(&server, Some(TOKEN));

    let created = client
        .create(&NewRequest {
            title: "Add feature".into(),
            body: "Details".into(),
            source: "feature-9".into(),
            target: "main".into(),
            draft: true,
        })
        .expect("create");
    assert_eq!(created.number, 9);
    client
        .merge(9, MergeMethod::Squash, Some("abc123"))
        .expect("merge");
    client.set_open(9, false).expect("close");
    client.set_open(9, true).expect("reopen");
    client.comment(9, "Thanks!").expect("comment");
    client.approve(9).expect("approve");

    let writes = server.writes();
    let calls: Vec<(&str, &str)> = writes
        .iter()
        .map(|r| (r.method.as_str(), r.target.as_str()))
        .collect();
    assert_eq!(
        calls,
        [
            ("POST", "/repos/octo/hello/pulls"),
            ("PUT", "/repos/octo/hello/pulls/9/merge"),
            ("PATCH", "/repos/octo/hello/pulls/9"),
            ("PATCH", "/repos/octo/hello/pulls/9"),
            ("POST", "/repos/octo/hello/issues/9/comments"),
            ("POST", "/repos/octo/hello/pulls/9/reviews"),
        ]
    );
    assert_eq!(
        writes[0].json(),
        json!({ "title": "Add feature", "body": "Details", "head": "feature-9", "base": "main", "draft": true })
    );
    assert_eq!(
        writes[1].json(),
        json!({ "merge_method": "squash", "sha": "abc123" })
    );
    assert_eq!(writes[2].json(), json!({ "state": "closed" }));
    assert_eq!(writes[3].json(), json!({ "state": "open" }));
    assert_eq!(writes[4].json(), json!({ "body": "Thanks!" }));
    assert_eq!(writes[5].json(), json!({ "event": "APPROVE" }));
    for w in &writes {
        assert_eq!(w.header("content-type"), Some("application/json"));
        assert_eq!(w.header("authorization"), Some(&*format!("Bearer {TOKEN}")));
        assert!(!w.target.contains(TOKEN) && !w.body.contains(TOKEN));
    }
}

#[test]
fn github_error_mapping() {
    let server = Server::start();
    server.route(
        "GET",
        "/user",
        401,
        r#"{"message":"Bad credentials","documentation_url":"https://docs.github.com/rest"}"#,
    );
    server.route_with_headers(
        "GET",
        "/repos/octo/hello/pulls",
        403,
        &[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1800000000"),
        ],
        r#"{"message":"API rate limit exceeded for user ID 1."}"#,
    );
    server.route(
        "PUT",
        "/repos/octo/hello/pulls/3/merge",
        405,
        r#"{"message":"Pull Request is not mergeable"}"#,
    );
    server.route(
        "POST",
        "/repos/octo/hello/pulls",
        422,
        r#"{"message":"Validation Failed","errors":[{"resource":"PullRequest","code":"custom","message":"A pull request already exists for octo:feature."}]}"#,
    );
    server.route(
        "POST",
        "/repos/octo/hello/pulls/3/reviews",
        403,
        r#"{"message":"Must have admin rights to Repository."}"#,
    );
    server.route("PATCH", "/repos/octo/hello/pulls/3", 200, "not json at all");
    server.route("GET", "/repos/octo/hello/pulls/3", 200, "not json at all");
    let client = github(&server, Some(TOKEN));

    let err = client.viewer().expect_err("401");
    assert_eq!(err, Error::Unauthorized("Bad credentials".into()));
    assert!(err.to_string().contains("Bad credentials"));
    assert_eq!(
        client
            .list(StateFilter::Open, 10)
            .expect_err("rate limited"),
        Error::RateLimited {
            reset: Some(1_800_000_000)
        }
    );
    let err = client.merge(3, MergeMethod::Merge, None).expect_err("405");
    assert_eq!(
        err,
        Error::Api {
            status: 405,
            message: "Pull Request is not mergeable".into()
        }
    );
    assert!(err.to_string().contains("Pull Request is not mergeable"));
    let err = client
        .create(&NewRequest {
            title: "t".into(),
            body: String::new(),
            source: "feature".into(),
            target: "main".into(),
            draft: false,
        })
        .expect_err("422");
    assert_eq!(
        err,
        Error::Api {
            status: 422,
            message: "Validation Failed; A pull request already exists for octo:feature.".into()
        }
    );
    assert_eq!(
        client.approve(3).expect_err("403"),
        Error::Unauthorized("Must have admin rights to Repository.".into())
    );
    // A write whose response body is ignored does not need JSON.
    client.set_open(3, false).expect("close ignores the body");
    assert!(matches!(
        client.details(3).expect_err("bad json"),
        Error::Parse(_)
    ));

    // Nothing listening.
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = l.local_addr().expect("addr").port();
        drop(l);
        port
    };
    let forge = Forge {
        kind: ForgeKind::GitHub,
        host: "github.com".into(),
        port: None,
        path: "octo/hello".into(),
    };
    let offline = Client::with_api_base(
        &forge,
        Some(TOKEN.into()),
        &format!("http://127.0.0.1:{dead}"),
    );
    let err = offline.viewer().expect_err("connection refused");
    assert!(matches!(err, Error::Network(_)), "{err:?}");
    assert!(!err.to_string().contains(TOKEN));
    assert!(!format!("{err:?}").contains(TOKEN));
}

#[test]
fn github_viewer() {
    let server = Server::start();
    server.route("GET", "/user", 200, r#"{"login":"octocat","id":1}"#);
    let client = github(&server, Some(TOKEN));
    assert!(client.has_token());
    assert_eq!(client.viewer().expect("viewer"), "octocat");
    assert_no_token_leak(&client);
}

// ----------------------------------------------------------------------------------------------
// GitLab
// ----------------------------------------------------------------------------------------------

#[test]
fn gitlab_list_maps_fields_and_merges_closed_states() {
    let server = Server::start();
    server.route(
        "GET",
        &format!("{GL}/merge_requests"),
        200,
        &json!([
            gl_mr(1, "opened", "2024-02-03T00:00:00Z"),
            gl_mr(2, "locked", "2024-02-02T00:00:00Z"),
            gl_mr(3, "opened", "2024-02-01T00:00:00Z"),
        ])
        .to_string(),
    );
    let client = gitlab(&server, Some(TOKEN));
    let list = client.list(StateFilter::Open, 20).expect("list");
    assert_eq!(list.len(), 3);
    let req = &server.requests()[0];
    assert_eq!(
        req.target,
        format!(
            "{GL}/merge_requests?state=opened&order_by=updated_at&sort=desc&per_page=20&page=1"
        )
    );
    assert_eq!(
        req.header("authorization"),
        Some(&*format!("Bearer {TOKEN}"))
    );
    assert!(req.header("x-github-api-version").is_none());
    let mr = &list[0];
    assert_eq!(mr.number, 1);
    assert_eq!(mr.title, "MR 1");
    assert_eq!(mr.author, "bob");
    assert_eq!(mr.state, RequestState::Open);
    assert_eq!(list[1].state, RequestState::Open, "locked counts as open");
    assert!(list[1].draft, "work_in_progress is a draft");
    assert!(!mr.from_fork);
    assert!(list[2].from_fork);
    assert_eq!(mr.source_branch, "topic-1");
    assert_eq!(mr.target_branch, "main");
    assert_eq!(mr.created_at, 1_706_778_000);
    assert_eq!(mr.labels, ["backend"]);

    // Closed = closed + merged, interleaved by update time.
    let server = Server::start();
    server.route(
        "GET",
        &format!(
            "{GL}/merge_requests?state=closed&order_by=updated_at&sort=desc&per_page=3&page=1"
        ),
        200,
        &json!([
            gl_mr(10, "closed", "2024-03-05T00:00:00Z"),
            gl_mr(11, "closed", "2024-03-01T00:00:00Z"),
        ])
        .to_string(),
    );
    server.route(
        "GET",
        &format!(
            "{GL}/merge_requests?state=merged&order_by=updated_at&sort=desc&per_page=3&page=1"
        ),
        200,
        &json!([
            gl_mr(20, "merged", "2024-03-06T00:00:00Z"),
            gl_mr(21, "merged", "2024-03-02T00:00:00Z"),
        ])
        .to_string(),
    );
    let list = gitlab(&server, None)
        .list(StateFilter::Closed, 3)
        .expect("list");
    assert_eq!(
        list.iter().map(|m| m.number).collect::<Vec<_>>(),
        [20, 10, 21]
    );
    assert_eq!(list[0].state, RequestState::Merged);
    assert_eq!(list[1].state, RequestState::Closed);
}

#[test]
fn gitlab_details_collects_parts_and_tolerates_failures() {
    let server = Server::start();
    let mut mr = gl_mr(4, "opened", "2024-02-03T00:00:00Z");
    mr["description"] = json!("Implements it.");
    mr["sha"] = json!("cafe01");
    mr["has_conflicts"] = json!(false);
    mr["detailed_merge_status"] = json!("mergeable");
    mr["changes_count"] = json!("1000+");
    mr["head_pipeline"] =
        json!({ "id": 55, "status": "running", "web_url": "https://gitlab.example.com/p/55" });
    server.route(
        "GET",
        &format!("{GL}/merge_requests/4"),
        200,
        &mr.to_string(),
    );
    server.route(
        "GET",
        &format!("{GL}/merge_requests/4/approvals"),
        200,
        &json!({ "approved": true, "approved_by": [{ "user": { "username": "eve" } }] })
            .to_string(),
    );
    server.route(
        "GET",
        &format!("{GL}/merge_requests/4/notes"),
        200,
        &json!([
            { "author": { "username": "eve" }, "body": "LGTM", "created_at": "2024-02-02T00:00:00Z", "system": false },
            { "author": { "username": "eve" }, "body": "approved this merge request", "created_at": "2024-02-02T00:01:00Z", "system": true },
        ])
        .to_string(),
    );
    // Jobs are forbidden for this token: tolerated.
    server.route(
        "GET",
        &format!("{GL}/pipelines/55/jobs"),
        403,
        r#"{"message":"403 Forbidden"}"#,
    );
    let d = gitlab(&server, Some(TOKEN)).details(4).expect("details");
    assert_eq!(d.summary.number, 4);
    assert_eq!(d.body, "Implements it.");
    assert_eq!(d.head_sha.as_deref(), Some("cafe01"));
    assert_eq!(d.mergeable, Some(true));
    assert_eq!(d.changed_files, Some(1000));
    assert_eq!((d.commits, d.additions, d.deletions), (None, None, None));
    assert_eq!(d.reviews.len(), 1);
    assert_eq!(
        (d.reviews[0].author.as_str(), d.reviews[0].state.as_str()),
        ("eve", "Approved")
    );
    assert_eq!(d.comments.len(), 1, "system notes are dropped");
    assert_eq!(d.comments[0].body, "LGTM");
    assert_eq!(d.checks.len(), 1);
    assert_eq!(d.checks[0].name, "Pipeline #55");
    assert_eq!(d.checks[0].state, CheckState::Pending);
    assert_eq!(
        d.checks[0].url.as_deref(),
        Some("https://gitlab.example.com/p/55")
    );
    let notes = server
        .requests()
        .into_iter()
        .find(|r| r.target.contains("/notes"))
        .expect("notes requested");
    assert!(notes.target.contains("sort=asc"), "{}", notes.target);

    // Jobs readable; approvals failing.
    server.route(
        "GET",
        &format!("{GL}/pipelines/55/jobs"),
        200,
        &json!([
            { "name": "build", "stage": "build", "status": "success", "web_url": "https://gitlab.example.com/j/1", "allow_failure": false },
            { "name": "lint", "stage": "test", "status": "failed", "allow_failure": true },
            { "name": "deploy", "stage": "deploy", "status": "manual" },
        ])
        .to_string(),
    );
    server.route(
        "GET",
        &format!("{GL}/merge_requests/4/approvals"),
        500,
        r#"{"message":"500 Internal Server Error"}"#,
    );
    let d = gitlab(&server, Some(TOKEN)).details(4).expect("details");
    assert!(d.reviews.is_empty());
    let checks: Vec<(&str, CheckState)> = d
        .checks
        .iter()
        .map(|c| (c.name.as_str(), c.state))
        .collect();
    assert_eq!(
        checks,
        [
            ("Pipeline #55", CheckState::Pending),
            ("build: build", CheckState::Success),
            ("test: lint", CheckState::Neutral),
            ("deploy: deploy", CheckState::Neutral),
        ]
    );
}

#[test]
fn gitlab_writes() {
    let server = Server::start();
    server.route(
        "POST",
        &format!("{GL}/merge_requests"),
        201,
        &gl_mr(12, "opened", "2024-02-03T00:00:00Z").to_string(),
    );
    server.route("PUT", &format!("{GL}/merge_requests/12/merge"), 200, "{}");
    server.route("PUT", &format!("{GL}/merge_requests/12"), 200, "{}");
    server.route("POST", &format!("{GL}/merge_requests/12/notes"), 201, "{}");
    server.route(
        "POST",
        &format!("{GL}/merge_requests/12/approve"),
        201,
        "{}",
    );
    let client = gitlab(&server, Some(TOKEN));

    let created = client
        .create(&NewRequest {
            title: "Add feature".into(),
            body: "Details".into(),
            source: "topic-12".into(),
            target: "main".into(),
            draft: true,
        })
        .expect("create");
    assert_eq!(created.number, 12);
    client
        .merge(12, MergeMethod::Squash, Some("def456"))
        .expect("squash");
    client.merge(12, MergeMethod::Merge, None).expect("merge");
    client.set_open(12, false).expect("close");
    client.set_open(12, true).expect("reopen");
    client.comment(12, "Thanks!").expect("comment");
    client.approve(12).expect("approve");

    let writes = server.writes();
    let calls: Vec<(&str, String)> = writes
        .iter()
        .map(|r| (r.method.as_str(), r.target.replace(GL, "")))
        .collect();
    assert_eq!(
        calls,
        [
            ("POST", "/merge_requests".to_owned()),
            ("PUT", "/merge_requests/12/merge".to_owned()),
            ("PUT", "/merge_requests/12/merge".to_owned()),
            ("PUT", "/merge_requests/12".to_owned()),
            ("PUT", "/merge_requests/12".to_owned()),
            ("POST", "/merge_requests/12/notes".to_owned()),
            ("POST", "/merge_requests/12/approve".to_owned()),
        ]
    );
    assert_eq!(
        writes[0].json(),
        json!({ "source_branch": "topic-12", "target_branch": "main", "title": "Draft: Add feature", "description": "Details" })
    );
    assert_eq!(writes[1].json(), json!({ "squash": true, "sha": "def456" }));
    assert_eq!(writes[2].json(), json!({ "squash": false }));
    assert_eq!(writes[3].json(), json!({ "state_event": "close" }));
    assert_eq!(writes[4].json(), json!({ "state_event": "reopen" }));
    assert_eq!(writes[5].json(), json!({ "body": "Thanks!" }));
    assert!(writes[6].body.is_empty());
    for w in &writes {
        assert_eq!(w.header("authorization"), Some(&*format!("Bearer {TOKEN}")));
    }
}

#[test]
fn gitlab_rebase_merge_rebases_then_merges_the_new_head() {
    let server = Server::start();
    let mut mr = gl_mr(8, "opened", "2024-02-03T00:00:00Z");
    mr["sha"] = json!("rebased1");
    mr["rebase_in_progress"] = json!(false);
    server.route(
        "GET",
        &format!("{GL}/merge_requests/8"),
        200,
        &mr.to_string(),
    );
    server.route(
        "PUT",
        &format!("{GL}/merge_requests/8/rebase"),
        202,
        r#"{"rebase_in_progress":true}"#,
    );
    server.route("PUT", &format!("{GL}/merge_requests/8/merge"), 200, "{}");
    gitlab(&server, Some(TOKEN))
        .merge(8, MergeMethod::Rebase, None)
        .expect("rebase and merge");
    let reqs = server.requests();
    let calls: Vec<(&str, String)> = reqs
        .iter()
        .map(|r| (r.method.as_str(), r.target.replace(GL, "")))
        .collect();
    assert_eq!(
        calls,
        [
            (
                "GET",
                "/merge_requests/8?include_rebase_in_progress=true".to_owned()
            ),
            ("PUT", "/merge_requests/8/rebase".to_owned()),
            (
                "GET",
                "/merge_requests/8?include_rebase_in_progress=true".to_owned()
            ),
            ("PUT", "/merge_requests/8/merge".to_owned()),
        ]
    );
    assert_eq!(
        reqs[3].json(),
        json!({ "squash": false, "sha": "rebased1" })
    );

    // A failed rebase stops before merging, with GitLab's reason.
    let server = Server::start();
    mr["merge_error"] = json!("Rebase failed: conflicts");
    server.route(
        "GET",
        &format!("{GL}/merge_requests/8"),
        200,
        &mr.to_string(),
    );
    server.route(
        "PUT",
        &format!("{GL}/merge_requests/8/rebase"),
        202,
        r#"{"rebase_in_progress":true}"#,
    );
    let err = gitlab(&server, Some(TOKEN))
        .merge(8, MergeMethod::Rebase, None)
        .expect_err("rebase failed");
    assert!(
        err.to_string().contains("Rebase failed: conflicts"),
        "{err}"
    );
    assert!(!server.writes().iter().any(|r| r.target.ends_with("/merge")));
}

/// The message for a merge refused because the head moved after the review.
fn head_changed(noun: &str) -> Error {
    Error::Api {
        status: 409,
        message: format!(
            "The {noun} changed since you loaded it. Refresh and review before merging."
        ),
    }
}

#[test]
fn github_merge_refuses_a_head_that_moved() {
    let server = Server::start();
    server.route(
        "PUT",
        "/repos/octo/hello/pulls/9/merge",
        409,
        r#"{"message":"Head branch was modified. Review and try the merge again.","documentation_url":"https://docs.github.com/rest"}"#,
    );
    let client = github(&server, Some(TOKEN));
    let err = client
        .merge(9, MergeMethod::Merge, Some(" reviewed1 "))
        .expect_err("409");
    assert_eq!(err, head_changed("pull request"));
    assert!(
        err.to_string()
            .contains("changed since you loaded it. Refresh and review before merging."),
        "{err}"
    );
    let writes = server.writes();
    assert_eq!(
        writes[0].json(),
        json!({ "merge_method": "merge", "sha": "reviewed1" })
    );

    // Without an expected head (or a blank one) no sha is sent and a 409 keeps its own message.
    let err = client
        .merge(9, MergeMethod::Merge, Some("  "))
        .expect_err("409");
    assert_eq!(
        err,
        Error::Api {
            status: 409,
            message: "Head branch was modified. Review and try the merge again.".into()
        }
    );
    assert_eq!(
        server.writes()[1].json(),
        json!({ "merge_method": "merge" })
    );
}

#[test]
fn gitlab_merge_refuses_a_head_that_moved() {
    let server = Server::start();
    server.route(
        "PUT",
        &format!("{GL}/merge_requests/12/merge"),
        409,
        r#"{"message":"SHA does not match HEAD of source branch: 0123abc"}"#,
    );
    let client = gitlab(&server, Some(TOKEN));
    let err = client
        .merge(12, MergeMethod::Squash, Some("reviewed1"))
        .expect_err("409");
    assert_eq!(err, head_changed("merge request"));
    assert_eq!(
        server.writes()[0].json(),
        json!({ "squash": true, "sha": "reviewed1" })
    );
}

#[test]
fn gitlab_rebase_merge_checks_the_head_before_rebasing() {
    let server = Server::start();
    let mut mr = gl_mr(8, "opened", "2024-02-03T00:00:00Z");
    mr["sha"] = json!("pushed-after-review");
    mr["rebase_in_progress"] = json!(false);
    server.route(
        "GET",
        &format!("{GL}/merge_requests/8"),
        200,
        &mr.to_string(),
    );
    server.route(
        "PUT",
        &format!("{GL}/merge_requests/8/rebase"),
        202,
        r#"{"rebase_in_progress":true}"#,
    );
    server.route("PUT", &format!("{GL}/merge_requests/8/merge"), 200, "{}");
    let client = gitlab(&server, Some(TOKEN));

    // The head moved: nothing is rebased or merged.
    let err = client
        .merge(8, MergeMethod::Rebase, Some("reviewed1"))
        .expect_err("stale head");
    assert_eq!(err, head_changed("merge request"));
    assert!(server.writes().is_empty(), "{:?}", server.writes());

    // The head is the reviewed one: rebased, then the rebased head is merged.
    client
        .merge(8, MergeMethod::Rebase, Some("pushed-after-review"))
        .expect("rebase and merge");
    let writes = server.writes();
    let targets: Vec<String> = writes.iter().map(|r| r.target.replace(GL, "")).collect();
    assert_eq!(
        targets,
        ["/merge_requests/8/rebase", "/merge_requests/8/merge"]
    );
    assert_eq!(
        writes[1].json(),
        json!({ "squash": false, "sha": "pushed-after-review" })
    );
}

#[test]
fn gitlab_error_mapping() {
    let server = Server::start();
    server.route(
        "GET",
        "/api/v4/user",
        401,
        r#"{"message":"401 Unauthorized"}"#,
    );
    server.route_with_headers(
        "GET",
        &format!("{GL}/merge_requests"),
        429,
        &[("ratelimit-reset", "1800000123")],
        "Retry later",
    );
    server.route(
        "POST",
        &format!("{GL}/merge_requests"),
        409,
        r#"{"message":["Another open merge request already exists for this source branch: !3"]}"#,
    );
    server.route(
        "PUT",
        &format!("{GL}/merge_requests/3/merge"),
        405,
        r#"{"message":"405 Method Not Allowed"}"#,
    );
    server.route(
        "POST",
        &format!("{GL}/merge_requests/3/approve"),
        403,
        r#"{"message":"403 Forbidden"}"#,
    );
    let client = gitlab(&server, Some(TOKEN));
    assert_eq!(
        client.viewer().expect_err("401"),
        Error::Unauthorized("401 Unauthorized".into())
    );
    assert_eq!(
        client.list(StateFilter::All, 5).expect_err("429"),
        Error::RateLimited {
            reset: Some(1_800_000_123)
        }
    );
    let err = client
        .create(&NewRequest {
            title: "t".into(),
            body: String::new(),
            source: "topic".into(),
            target: "main".into(),
            draft: false,
        })
        .expect_err("409");
    assert_eq!(
        err,
        Error::Api {
            status: 409,
            message: "Another open merge request already exists for this source branch: !3".into()
        }
    );
    assert_eq!(
        client.merge(3, MergeMethod::Squash, None).expect_err("405"),
        Error::Api {
            status: 405,
            message: "405 Method Not Allowed".into()
        }
    );
    assert_eq!(
        client.approve(3).expect_err("403"),
        Error::Unauthorized("403 Forbidden".into())
    );
    let err = client.details(404).expect_err("404");
    assert_eq!(err, Error::NotFound("Not Found".into()));
    assert!(err.to_string().contains("Not Found"));
}

#[test]
fn gitlab_viewer_and_debug() {
    let server = Server::start();
    server.route("GET", "/api/v4/user", 200, r#"{"username":"bob","id":3}"#);
    let client = gitlab(&server, Some(TOKEN));
    assert_eq!(client.viewer().expect("viewer"), "bob");
    assert_eq!(client.kind(), ForgeKind::GitLab);
    assert_no_token_leak(&client);
    let dbg = format!("{client:?}");
    assert!(dbg.contains("group%2Fsub%2Fproj"), "{dbg}");
}

#[test]
fn checkout_refs() {
    assert_eq!(
        canopy_hosting::checkout_ref(ForgeKind::GitHub, 12),
        "refs/pull/12/head"
    );
    assert_eq!(
        canopy_hosting::checkout_ref(ForgeKind::GitLab, 12),
        "refs/merge-requests/12/head"
    );
}

#[test]
fn author_pictures_resolve_both_providers_and_download_without_tokens() {
    let server = Server::start();
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let picture = format!("{}/picture.png", server.base);
    // The list endpoint answers with the commit and its author account but without the changed
    // files, which for a large commit would not fit the lookup's size limit.
    server.route(
        "GET",
        "/repos/octo/hello/commits",
        200,
        &json!([{"sha": sha, "author": {"avatar_url": picture}, "unknown": true}]).to_string(),
    );
    server.route(
        "GET",
        "/api/v4/avatar?email=author%2Bgit%40example.com&size=64",
        200,
        &json!({"avatar_url": picture}).to_string(),
    );
    server.route("GET", "/picture.png", 200, "image fixture bytes");
    for client in [github(&server, Some(TOKEN)), gitlab(&server, Some(TOKEN))] {
        let avatar = client
            .author_avatar(sha, "author+git@example.com")
            .unwrap()
            .unwrap();
        assert!(!format!("{avatar:?}").contains(TOKEN));
        assert_eq!(avatar.download().unwrap(), b"image fixture bytes");
    }
    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests[0].target,
        format!("/repos/octo/hello/commits?sha={sha}&per_page=1")
    );
    assert_eq!(
        requests[0].header("authorization"),
        Some(&format!("Bearer {TOKEN}")[..])
    );
    assert_eq!(
        requests[2].header("authorization"),
        Some(&format!("Bearer {TOKEN}")[..])
    );
    for request in [&requests[1], &requests[3]] {
        assert!(request.header("authorization").is_none());
        assert!(request.header("cookie").is_none());
        assert_eq!(request.target, "/picture.png");
    }
}

#[test]
fn author_picture_redirects_and_off_host_urls_are_never_followed() {
    let server = Server::start();
    let other = Server::start();
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let path = "/repos/octo/hello/commits".to_owned();
    let external = format!("{}/stolen", other.base);
    let client = github(&server, Some(TOKEN));
    server.route_with_headers("GET", &path, 302, &[("Location", &external)], "");
    assert!(client.author_avatar(sha, "a@example.com").is_err());
    assert!(other.requests().is_empty());
    server.route(
        "GET",
        &path,
        200,
        &json!([{"sha":sha,"author":{"avatar_url":external}}]).to_string(),
    );
    assert!(
        client
            .author_avatar(sha, "a@example.com")
            .unwrap()
            .is_none()
    );
    let own = format!("{}/picture", server.base);
    server.route(
        "GET",
        &path,
        200,
        &json!([{"sha":sha,"author":{"avatar_url":own}}]).to_string(),
    );
    server.route_with_headers("GET", "/picture", 302, &[("Location", &external)], "");
    assert!(
        client
            .author_avatar(sha, "a@example.com")
            .unwrap()
            .unwrap()
            .download()
            .is_err()
    );
    assert!(other.requests().is_empty());
}

#[test]
fn author_picture_missing_accounts_errors_and_limits_keep_the_fallback() {
    let server = Server::start();
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let path = "/repos/octo/hello/commits".to_owned();
    let client = github(&server, None);
    server.route(
        "GET",
        &path,
        200,
        &json!([{"sha":sha,"author":null}]).to_string(),
    );
    assert!(
        client
            .author_avatar(sha, "a@example.com")
            .unwrap()
            .is_none()
    );
    assert!(server.requests()[0].header("authorization").is_none());
    let count = server.requests().len();
    assert!(
        client
            .author_avatar("../../users", "a@example.com")
            .is_err()
    );
    assert_eq!(server.requests().len(), count);
    for status in [401, 403, 404, 429, 500] {
        server.route("GET", &path, status, r#"{"message":"Unavailable"}"#);
        assert!(client.author_avatar(sha, "a@example.com").is_err());
        server.route(
            "GET",
            "/api/v4/avatar",
            status,
            r#"{"message":"Unavailable"}"#,
        );
        assert!(
            gitlab(&server, None)
                .author_avatar(sha, "a@example.com")
                .is_err()
        );
    }
    // No commit, or a different one than asked for, never yields someone else's picture.
    server.route("GET", &path, 200, "[]");
    assert!(
        client
            .author_avatar(sha, "a@example.com")
            .unwrap()
            .is_none()
    );
    server.route(
        "GET",
        &path,
        200,
        &json!([{"sha":"f".repeat(40),"author":{"avatar_url":format!("{}/picture",server.base)}}])
            .to_string(),
    );
    assert!(
        client
            .author_avatar(sha, "a@example.com")
            .unwrap()
            .is_none()
    );
    server.route("GET", &path, 200, "malformed");
    assert!(matches!(
        client.author_avatar(sha, "a@example.com"),
        Err(Error::Parse(_))
    ));
    server.route("GET", "/api/v4/avatar", 200, "malformed");
    assert!(matches!(
        gitlab(&server, None).author_avatar(sha, "a@example.com"),
        Err(Error::Parse(_))
    ));
    server.route(
        "GET",
        &path,
        200,
        &json!([{"sha":sha,"author":{"avatar_url":format!("{}/oversized",server.base)}}])
            .to_string(),
    );
    server.route("GET", "/oversized", 200, &"x".repeat(2 * 1024 * 1024 + 1));
    assert!(
        client
            .author_avatar(sha, "a@example.com")
            .unwrap()
            .unwrap()
            .download()
            .is_err()
    );
}
