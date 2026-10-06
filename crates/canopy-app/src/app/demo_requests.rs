//! Demo pull and merge requests for screenshots: `--demo-requests` (with `--screenshot`).
//!
//! The requests view needs a forge remote, an API thread and a server to fill it, and a
//! screenshot has none of the three (tools/screenshot.sh renders a local fixture, offline). This
//! module puts a fixed, invented set of requests straight into the active tab instead:
//!
//! - nothing touches the network, even if someone clicks in a hand-run screenshot. The view only
//!   expects a handle to an API thread, so the tab gets [`HostWorker::offline`]: a handle with no
//!   thread behind it, which drops every job and never answers. The list and the details are
//!   written into the tab's state;
//! - it exists only under `--screenshot`: `main.rs` refuses the flag without it, and the one
//!   caller of [`Scenario::show`] is the screenshot driver;
//! - it is deterministic: the data is a constant table, times are fixed offsets from the moment
//!   it is installed (or fixed past dates), and the authors, branches and project are invented.

use std::sync::{Arc, OnceLock};

use canopy_core::forge::{Forge, ForgeKind};
use canopy_hosting::{
    Check, CheckState, Comment, MergeMethod, RequestDetails, RequestState, RequestSummary, Review,
    StateFilter,
};

use crate::hosting::{HostWorker, TokenSource};
use crate::tab::RepoTab;

/// The scenario `--demo-requests` chose. Set once by `main.rs`, read by the screenshot driver.
static CHOSEN: OnceLock<Scenario> = OnceLock::new();

/// What `--demo-requests` accepts, for `--help` and the error message.
pub const SPECS: &str = "list, list-all, list-closed, <number>, busy, loading, details-loading, \
details-error, error, no-token or empty; prefix gitlab: for merge requests";

/// Which part of the requests view to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scene {
    /// The list under this filter, nothing selected.
    List(StateFilter),
    /// The list with this request selected and its details loaded.
    Request(u64),
    /// The request in flight: its merge box shows the busy line.
    Busy,
    /// The list is still loading.
    Loading,
    /// The list is loaded and the selected request's details are still loading.
    DetailsLoading,
    /// The list is loaded and the selected request's details failed.
    DetailsError,
    /// The list failed with a server error.
    Error,
    /// The list failed because Canopy has no token.
    NoToken,
    /// A project without requests.
    Empty,
}

/// A demo scenario: the provider and the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scenario {
    kind: ForgeKind,
    scene: Scene,
}

/// Records the scenario for the screenshot driver. Refused without `--screenshot`, so a normal
/// run can never show demo data.
pub fn enable(scenario: Scenario, screenshot: bool) -> Result<(), String> {
    if !screenshot {
        return Err("--demo-requests only works together with --screenshot".into());
    }
    // `main.rs` rejects a repeated flag; a second call can only be a programming error.
    if CHOSEN.set(scenario).is_err() {
        return Err("the demo scenario was already chosen".into());
    }
    Ok(())
}

/// The scenario for the screenshot in progress, if `--demo-requests` was given.
pub(super) fn chosen() -> Option<Scenario> {
    CHOSEN.get().copied()
}

impl Scenario {
    /// Parses the value of `--demo-requests`; see [`SPECS`].
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (kind, rest) = match spec.strip_prefix("gitlab:") {
            Some(rest) => (ForgeKind::GitLab, rest),
            None => (ForgeKind::GitHub, spec),
        };
        let scene = match rest {
            "list" => Scene::List(StateFilter::Open),
            "list-all" => Scene::List(StateFilter::All),
            "list-closed" => Scene::List(StateFilter::Closed),
            "busy" => Scene::Busy,
            "loading" => Scene::Loading,
            "details-loading" => Scene::DetailsLoading,
            "details-error" => Scene::DetailsError,
            "error" => Scene::Error,
            "no-token" => Scene::NoToken,
            "empty" => Scene::Empty,
            number => {
                let Ok(number) = number.parse::<u64>() else {
                    return Err(format!("unknown --demo-requests value {spec}; use {SPECS}"));
                };
                if !ROWS.iter().any(|row| row.number == number) {
                    let known: Vec<String> = ROWS.iter().map(|r| r.number.to_string()).collect();
                    return Err(format!(
                        "no demo request #{number}; the demo has {}",
                        known.join(", ")
                    ));
                }
                Scene::Request(number)
            }
        };
        Ok(Self { kind, scene })
    }

    /// The invented project the requests belong to.
    fn forge(self) -> Forge {
        match self.kind {
            ForgeKind::GitHub => Forge {
                kind: ForgeKind::GitHub,
                host: "github.com".into(),
                port: None,
                path: "example-org/harbor".into(),
            },
            ForgeKind::GitLab => Forge {
                kind: ForgeKind::GitLab,
                host: "gitlab.com".into(),
                port: None,
                path: "example-org/platform/harbor".into(),
            },
        }
    }

    /// Makes `tab` show this scenario. Safe to call every frame: it installs once and again only
    /// when something replaced the forge (a repository refresh re-reads the remotes). Returns
    /// whether the tab shows the scenario: it needs the repository to have loaded, and not to have
    /// a forge remote of its own.
    pub(super) fn show(self, ctx: &egui::Context, tab: &mut RepoTab, now: i64) -> bool {
        if tab.snap.is_none() {
            return false;
        }
        let forge = self.forge();
        // A repository with a real GitHub or GitLab remote would get a real API thread from the
        // app's own start-up path. Refuse it; the fixtures have no such remote.
        if tab.forges.iter().any(|(_, f)| *f != forge) {
            tracing::error!("--demo-requests needs a repository without a GitHub or GitLab remote");
            return false;
        }
        let installed = tab.host.as_ref().is_some_and(|host| host.forge == forge)
            && tab
                .forges
                .first()
                .is_some_and(|(remote, f)| remote == "origin" && *f == forge);
        if !installed {
            self.install(ctx, tab, now);
        }
        // No thread reports a token: the scenario decides what is known.
        tab.requests.token = Some((self.scene != Scene::NoToken).then_some(TokenSource::Saved));
        true
    }

    /// Gives `tab` the demo project and an offline handle in place of an API thread, and shows
    /// the scene.
    fn install(self, ctx: &egui::Context, tab: &mut RepoTab, now: i64) {
        let forge = self.forge();
        tab.forges = vec![("origin".to_owned(), forge.clone())];
        tab.host = Some(HostWorker::offline(forge, ctx.clone()));
        self.fill(tab, now);
        self.seed_signals(ctx, tab, now);
    }

    /// Lets the list rows show every request's checks and reviews, as they do in a session where
    /// the reviewer has already opened each request (the list API sends neither). The request
    /// whose details are still loading or failed has not been seen yet.
    fn seed_signals(self, ctx: &egui::Context, tab: &RepoTab, now: i64) {
        let unseen = matches!(self.scene, Scene::DetailsLoading | Scene::DetailsError);
        for row in ROWS {
            if unseen && row.number == BUSY_NUMBER {
                continue;
            }
            if let Some(d) = details(self.kind, row.number, now) {
                crate::views::requests::remember_signals(ctx, tab, &d);
            }
        }
    }

    /// Writes the scene into the tab's request state.
    fn fill(self, tab: &mut RepoTab, now: i64) {
        let kind = self.kind;
        let requests = &mut tab.requests;
        requests.merge_method = MergeMethod::Merge;
        requests.selected = None;
        requests.details = None;
        requests.list = None;
        requests.loading = false;
        requests.details_loading = false;
        requests.busy = None;
        requests.filter = StateFilter::Open;
        let list = |filter| Some(Ok(summaries(kind, filter, now)));
        match self.scene {
            Scene::List(filter) => {
                requests.filter = filter;
                requests.list = list(filter);
            }
            Scene::Request(number) => {
                let open = ROWS
                    .iter()
                    .find(|row| row.number == number)
                    .is_some_and(|row| row.state == RequestState::Open);
                requests.filter = if open {
                    StateFilter::Open
                } else {
                    StateFilter::All
                };
                requests.list = list(requests.filter);
                requests.selected = Some(number);
                requests.details = details(kind, number, now).map(|d| Ok(Arc::new(d)));
            }
            Scene::Busy => {
                requests.list = list(StateFilter::Open);
                requests.selected = Some(BUSY_NUMBER);
                requests.details = details(kind, BUSY_NUMBER, now).map(|d| Ok(Arc::new(d)));
                requests.busy = Some(format!("Merging {} #{BUSY_NUMBER}", kind.request_abbrev()));
            }
            Scene::Loading => requests.loading = true,
            Scene::DetailsLoading => {
                requests.list = list(StateFilter::Open);
                requests.selected = Some(BUSY_NUMBER);
                requests.details_loading = true;
            }
            Scene::DetailsError => {
                requests.list = list(StateFilter::Open);
                requests.selected = Some(BUSY_NUMBER);
                requests.details = Some(Err(canopy_hosting::Error::Network(
                    "connection timed out".into(),
                )
                .to_string()));
            }
            Scene::Error => {
                requests.list = Some(Err(canopy_hosting::Error::Api {
                    status: 502,
                    message: "Bad Gateway".into(),
                }
                .to_string()));
            }
            Scene::NoToken => {
                requests.list = Some(Err(
                    canopy_hosting::Error::NotFound("Not Found".into()).to_string()
                ));
            }
            Scene::Empty => requests.list = Some(Ok(Arc::from([]))),
        }
    }
}

/// The request the merge box scenes use: open, approved, every check passing.
const BUSY_NUMBER: u64 = 428;

/// A time: a fixed distance before the scenario is installed, or a fixed date. Dates are for
/// requests older than a week, which Canopy shows as a calendar date: a fixed date reads the
/// same whenever the screenshot is taken.
#[derive(Debug, Clone, Copy)]
enum When {
    Ago(i64),
    At(i64),
}

impl When {
    const fn minutes(n: i64) -> Self {
        Self::Ago(n * 60 + 20)
    }

    const fn hours(n: i64) -> Self {
        Self::Ago(n * 3600 + 1200)
    }

    const fn days(n: i64) -> Self {
        Self::Ago(n * 86_400 + 3600)
    }

    fn at(self, now: i64) -> i64 {
        match self {
            Self::Ago(seconds) => now - seconds,
            Self::At(unix) => unix,
        }
    }
}

/// 2026-09-14 09:30 UTC, 2026-09-09 16:05 UTC, 2026-09-02 11:45 UTC, 2026-08-27 08:10 UTC.
const SEP_14: When = When::At(1_789_378_200);
const SEP_09: When = When::At(1_788_969_900);
const SEP_02: When = When::At(1_788_349_500);
const AUG_27: When = When::At(1_787_818_200);

/// Which CI checks a request has, and how they ended.
#[derive(Debug, Clone, Copy)]
enum Ci {
    None,
    Passing,
    Failing,
    Running,
    Mixed,
}

impl Ci {
    fn checks(self, kind: ForgeKind) -> Vec<Check> {
        let names: &[&str] = match kind {
            ForgeKind::GitHub => &[
                "build (linux)",
                "build (macos)",
                "build (windows)",
                "clippy",
                "test (linux)",
                "docs",
            ],
            ForgeKind::GitLab => &[
                "build:linux",
                "build:macos",
                "build:windows",
                "lint:clippy",
                "test:linux",
                "docs",
            ],
        };
        let states: &[CheckState] = match self {
            Self::None => &[],
            Self::Passing => &[CheckState::Success; 6],
            Self::Failing => &[
                CheckState::Success,
                CheckState::Success,
                CheckState::Failure,
                CheckState::Success,
                CheckState::Failure,
                CheckState::Neutral,
            ],
            Self::Running => &[
                CheckState::Success,
                CheckState::Pending,
                CheckState::Pending,
                CheckState::Success,
                CheckState::Pending,
                CheckState::Neutral,
            ],
            Self::Mixed => &[
                CheckState::Success,
                CheckState::Success,
                CheckState::Success,
                CheckState::Success,
                CheckState::Pending,
                CheckState::Neutral,
            ],
        };
        names
            .iter()
            .zip(states)
            .enumerate()
            .map(|(i, (name, state))| Check {
                name: (*name).to_owned(),
                state: *state,
                url: (i % 2 == 0).then(|| format!("https://ci.example.invalid/runs/{i}")),
            })
            .collect()
    }
}

/// One demo request: everything the list row and the detail view show.
struct Row {
    number: u64,
    title: &'static str,
    author: &'static str,
    state: RequestState,
    draft: bool,
    source: &'static str,
    target: &'static str,
    fork: bool,
    created: When,
    updated: When,
    labels: &'static [&'static str],
    ci: Ci,
    /// Reviewer and verdict.
    reviews: &'static [(&'static str, &'static str)],
    /// `None` while the provider is still working it out.
    mergeable: Option<bool>,
    /// Commits, changed files, added and deleted lines.
    stats: (u32, u32, u32, u32),
    body: &'static str,
    /// Author, text and when.
    comments: &'static [(&'static str, &'static str, When)],
}

const APPROVED: &str = "Approved";
const CHANGES: &str = "Changes requested";
const COMMENTED: &str = "Commented";

const NO_COMMENTS: &[(&str, &str, When)] = &[];

/// The demo project's requests, newest first.
const ROWS: &[Row] = &[
    Row {
        number: 428,
        title: "Stream large diffs through the virtual list without materialising every hunk",
        author: "lucia-ortega",
        state: RequestState::Open,
        draft: false,
        source: "perf/stream-diff-hunks",
        target: "main",
        fork: false,
        created: When::hours(30),
        updated: When::minutes(18),
        labels: &["performance", "diff", "ready for review"],
        ci: Ci::Passing,
        reviews: &[("maya-chen", APPROVED), ("sam-okafor", COMMENTED)],
        mergeable: Some(true),
        stats: (6, 9, 412, 138),
        body: "Large diffs no longer allocate a row per line up front.\n\n\
### What changed\n\
- Hunks are laid out lazily, as the virtual list asks for them\n\
- The side-by-side view shares the same layout cache\n\
- A 40 000 line diff opens in about a tenth of the time\n\n\
### How it was checked\n\
- Scrolled the 40 000 line fixture end to end at 144 Hz\n\
- Added a layout test for the hunk boundary rows\n\n\
Closes the long-standing scrolling stutter on generated files.",
        comments: &[
            (
                "sam-okafor",
                "Nice. Does the cache survive switching between unified and side by side?",
                When::hours(5),
            ),
            (
                "lucia-ortega",
                "Yes, both views read the same lines. I added a test that flips the mode on a loaded diff.",
                When::hours(3),
            ),
            (
                "maya-chen",
                "Approved. Could you squash the two fixup commits when you merge?",
                When::minutes(40),
            ),
        ],
    },
    Row {
        number: 427,
        title: "Rewrite the commit graph lane allocator for octopus merges and very long-lived release branches",
        author: "priya-raman",
        state: RequestState::Open,
        draft: true,
        source: "feature/graph-lane-allocator",
        target: "main",
        fork: false,
        created: When::days(2),
        updated: When::hours(7),
        labels: &["graph", "wip", "needs design"],
        ci: Ci::Running,
        reviews: &[],
        mergeable: Some(true),
        stats: (14, 7, 640, 291),
        body: "Work in progress. The current allocator reuses a lane as soon as it is free, which \
makes long-lived branches hop columns.\n\n\
- [x] Keep a branch in its lane while it has children below\n\
- [x] Octopus merges fan out in stable order\n\
- [ ] Re-balance lanes after a collapsed section\n\
- [ ] Benchmarks on the 200 000 commit fixture",
        comments: &[(
            "jonas-weber",
            "Happy to pair on the benchmark harness when you are ready.",
            When::hours(9),
        )],
    },
    Row {
        number: 426,
        title: "Remember the diff view mode per repository",
        author: "sam-okafor",
        state: RequestState::Merged,
        draft: false,
        source: "feature/diff-mode-per-repo",
        target: "main",
        fork: false,
        created: When::days(3),
        updated: When::days(1),
        labels: &["diff", "settings"],
        ci: Ci::Passing,
        reviews: &[("lucia-ortega", APPROVED), ("maya-chen", APPROVED)],
        mergeable: None,
        stats: (3, 4, 96, 12),
        body: "Unified or side by side is now stored with the repository, not globally.",
        comments: &[(
            "maya-chen",
            "Merged. Thanks for adding the migration test.",
            When::days(1),
        )],
    },
    Row {
        number: 425,
        title: "Fix crash when a worktree path contains a non-UTF-8 byte",
        author: "jonas-weber",
        state: RequestState::Open,
        draft: false,
        source: "fix/non-utf8-worktree",
        target: "main",
        fork: false,
        created: When::hours(20),
        updated: When::hours(2),
        labels: &["bug", "linux", "crash"],
        ci: Ci::Failing,
        reviews: &[("maya-chen", CHANGES)],
        mergeable: Some(true),
        stats: (2, 3, 58, 17),
        body: "Paths are `OsString` end to end now; the lossy conversion in the tab title was \
the last place that assumed UTF-8.\n\n\
Reproduce with `mkdir $'bad\\xff'` and open it as a repository.",
        comments: &[
            (
                "maya-chen",
                "The Windows build fails on the new test: `OsStr` has no `as_bytes` there. Gate it with `cfg(unix)` or build the name another way.",
                When::hours(3),
            ),
            (
                "jonas-weber",
                "Right, I will build the name from a wide string on Windows.",
                When::hours(2),
            ),
        ],
    },
    Row {
        number: 424,
        title: "Add keyboard navigation to the branch picker",
        author: "tomas-lindqvist",
        state: RequestState::Open,
        draft: false,
        source: "tomas/branch-picker-keys",
        target: "main",
        fork: true,
        created: When::days(4),
        updated: When::days(1),
        labels: &["accessibility", "keyboard", "good first issue"],
        ci: Ci::Mixed,
        reviews: &[("lucia-ortega", COMMENTED)],
        mergeable: Some(true),
        stats: (4, 5, 187, 24),
        body: "Up and Down move through the list, Enter checks out, Escape closes. The focused \
row is announced to screen readers.",
        comments: &[(
            "lucia-ortega",
            "Works well. One thing: Home and End should jump to the first and last branch.",
            When::days(1),
        )],
    },
    Row {
        number: 423,
        title: "Show the author's picture in the blame gutter",
        author: "priya-raman",
        state: RequestState::Merged,
        draft: false,
        source: "feature/blame-avatars",
        target: "main",
        fork: false,
        created: When::days(5),
        updated: When::days(3),
        labels: &["blame", "avatars"],
        ci: Ci::Passing,
        reviews: &[("sam-okafor", APPROVED)],
        mergeable: None,
        stats: (5, 6, 244, 51),
        body: "",
        comments: NO_COMMENTS,
    },
    Row {
        number: 422,
        title: "Fix off-by-one when staging the last hunk of a file without a trailing newline",
        author: "maya-chen",
        state: RequestState::Merged,
        draft: false,
        source: "fix/last-hunk-newline",
        target: "main",
        fork: false,
        created: SEP_14,
        updated: SEP_14,
        labels: &["bug", "staging"],
        ci: Ci::Passing,
        reviews: &[("jonas-weber", APPROVED)],
        mergeable: None,
        stats: (1, 2, 31, 6),
        body: "Staging the final hunk dropped the `\\ No newline at end of file` marker.",
        comments: NO_COMMENTS,
    },
    Row {
        number: 421,
        title: "Bump rustls from 0.23 to 0.24",
        author: "dep-updater",
        state: RequestState::Open,
        draft: false,
        source: "deps/rustls-0.24",
        target: "main",
        fork: false,
        created: When::days(6),
        updated: When::days(5),
        labels: &["dependencies"],
        ci: Ci::Passing,
        reviews: &[],
        mergeable: Some(true),
        stats: (1, 2, 14, 14),
        body: "Bumps `rustls` from 0.23.12 to 0.24.0.\n\n\
Release notes: the server name API now takes owned names; Canopy's client is unaffected.",
        comments: NO_COMMENTS,
    },
    Row {
        number: 420,
        title: "Replace libgit2 reads with gitoxide",
        author: "tomas-lindqvist",
        state: RequestState::Closed,
        draft: false,
        source: "experiment/gitoxide-reads",
        target: "main",
        fork: true,
        created: SEP_09,
        updated: SEP_09,
        labels: &["experiment", "wontfix"],
        ci: Ci::Failing,
        reviews: &[("maya-chen", CHANGES)],
        mergeable: Some(false),
        stats: (23, 41, 2210, 1894),
        body: "A spike to see how much of the read path gitoxide covers today.",
        comments: &[(
            "maya-chen",
            "Closing for now: the status and blame paths are not there yet. Worth revisiting next quarter.",
            SEP_09,
        )],
    },
    Row {
        number: 419,
        title: "Migrate settings storage to a versioned schema",
        author: "maya-chen",
        state: RequestState::Open,
        draft: false,
        source: "feature/settings-schema-v2",
        target: "release/2.x",
        fork: false,
        created: When::days(3),
        updated: When::hours(11),
        labels: &[],
        ci: Ci::Passing,
        reviews: &[],
        mergeable: Some(false),
        stats: (8, 12, 530, 217),
        body: "Settings gain a `version` field and an ordered list of migrations.\n\n\
This conflicts with #425 in `settings.rs`; I will rebase once that lands.",
        comments: NO_COMMENTS,
    },
    Row {
        number: 418,
        title: "Add a Portuguese translation of the welcome tour",
        author: "helena-duarte",
        state: RequestState::Merged,
        draft: false,
        source: "helena/pt-welcome",
        target: "main",
        fork: true,
        created: SEP_02,
        updated: SEP_02,
        labels: &["i18n", "docs"],
        ci: Ci::Passing,
        reviews: &[("sam-okafor", APPROVED)],
        mergeable: None,
        stats: (2, 1, 64, 0),
        body: "",
        comments: NO_COMMENTS,
    },
    Row {
        number: 417,
        title: "Remove the legacy diff cache",
        author: "jonas-weber",
        state: RequestState::Closed,
        draft: false,
        source: "chore/remove-diff-cache",
        target: "main",
        fork: false,
        created: AUG_27,
        updated: AUG_27,
        labels: &["chore"],
        ci: Ci::None,
        reviews: &[],
        mergeable: Some(true),
        stats: (1, 3, 0, 188),
        body: "Superseded by #428.",
        comments: NO_COMMENTS,
    },
    Row {
        number: 416,
        title: "Experiment with a frecency-ranked command palette",
        author: "tomas-lindqvist",
        state: RequestState::Open,
        draft: true,
        source: "experiment/frecency-palette",
        target: "main",
        fork: true,
        created: When::days(5),
        updated: When::days(4),
        labels: &["experiment"],
        ci: Ci::None,
        reviews: &[],
        mergeable: Some(true),
        stats: (2, 3, 120, 9),
        body: "Ranks palette commands by how recently and how often they run.",
        comments: NO_COMMENTS,
    },
];

fn summary(kind: ForgeKind, row: &Row, now: i64) -> RequestSummary {
    let path = match kind {
        ForgeKind::GitHub => format!("https://github.com/example-org/harbor/pull/{}", row.number),
        ForgeKind::GitLab => format!(
            "https://gitlab.com/example-org/platform/harbor/-/merge_requests/{}",
            row.number
        ),
    };
    RequestSummary {
        number: row.number,
        // GitLab keeps the draft mark in the title, GitHub in a flag.
        title: if kind == ForgeKind::GitLab && row.draft {
            format!("Draft: {}", row.title)
        } else {
            row.title.to_owned()
        },
        author: row.author.to_owned(),
        state: row.state,
        draft: row.draft,
        source_branch: row.source.to_owned(),
        target_branch: row.target.to_owned(),
        from_fork: row.fork,
        created_at: row.created.at(now),
        updated_at: row.updated.at(now),
        web_url: path,
        labels: row.labels.iter().map(|l| (*l).to_owned()).collect(),
    }
}

/// The list rows `filter` shows, as the provider would return them.
fn summaries(kind: ForgeKind, filter: StateFilter, now: i64) -> Arc<[RequestSummary]> {
    ROWS.iter()
        .filter(|row| match filter {
            StateFilter::Open => row.state == RequestState::Open,
            StateFilter::Closed => row.state != RequestState::Open,
            StateFilter::All => true,
        })
        .map(|row| summary(kind, row, now))
        .collect()
}

/// The detail view's data for request `number`.
fn details(kind: ForgeKind, number: u64, now: i64) -> Option<RequestDetails> {
    let row = ROWS.iter().find(|row| row.number == number)?;
    // GitHub reports line counts and commits; GitLab only caps the changed files.
    let github = kind == ForgeKind::GitHub;
    let (commits, files, added, deleted) = row.stats;
    Some(RequestDetails {
        summary: summary(kind, row, now),
        body: row.body.to_owned(),
        head_sha: Some(format!("{:07x}{:033x}", row.number * 7919, row.number)),
        mergeable: row.mergeable,
        commits: github.then_some(commits),
        changed_files: Some(files),
        additions: github.then_some(added),
        deletions: github.then_some(deleted),
        reviews: row
            .reviews
            .iter()
            .map(|(author, state)| Review {
                author: (*author).to_owned(),
                state: (*state).to_owned(),
            })
            .collect(),
        checks: row.ci.checks(kind),
        comments: row
            .comments
            .iter()
            .map(|(author, body, when)| Comment {
                author: (*author).to_owned(),
                body: (*body).to_owned(),
                created_at: when.at(now),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_000_000;

    fn rows(filter: StateFilter) -> Vec<RequestSummary> {
        summaries(ForgeKind::GitHub, filter, NOW).to_vec()
    }

    #[test]
    fn every_documented_value_parses() {
        for spec in [
            "list",
            "list-all",
            "list-closed",
            "428",
            "busy",
            "loading",
            "details-loading",
            "details-error",
            "error",
            "no-token",
            "empty",
            "gitlab:list",
            "gitlab:428",
        ] {
            assert!(Scenario::parse(spec).is_ok(), "{spec}");
        }
        assert_eq!(
            Scenario::parse("gitlab:list"),
            Ok(Scenario {
                kind: ForgeKind::GitLab,
                scene: Scene::List(StateFilter::Open),
            })
        );
    }

    #[test]
    fn unknown_values_are_rejected_with_the_choices() {
        for spec in ["", "lists", "gitlab:", "github:list", "99999", "-1"] {
            assert!(Scenario::parse(spec).is_err(), "{spec}");
        }
        let message = Scenario::parse("nope").unwrap_err();
        assert!(message.contains("no-token") && message.contains("gitlab:"));
        assert!(Scenario::parse("99999").unwrap_err().contains("428"));
    }

    #[test]
    fn the_flag_needs_a_screenshot() {
        let scenario = Scenario::parse("list").unwrap();
        assert!(enable(scenario, false).is_err());
        assert!(
            chosen().is_none(),
            "a refused flag must not leave demo data enabled"
        );
    }

    #[test]
    fn request_numbers_are_unique_and_newest_first() {
        let numbers: Vec<u64> = ROWS.iter().map(|r| r.number).collect();
        assert!(numbers.windows(2).all(|w| w[0] > w[1]), "{numbers:?}");
    }

    #[test]
    fn the_demo_covers_every_state_a_row_can_show() {
        let all = rows(StateFilter::All);
        let has = |f: &dyn Fn(&RequestSummary) -> bool| all.iter().any(f);
        assert!(has(&|r| r.state == RequestState::Open && !r.draft));
        assert!(has(&|r| r.state == RequestState::Open && r.draft));
        assert!(has(&|r| r.state == RequestState::Merged));
        assert!(has(&|r| r.state == RequestState::Closed));
        assert!(has(&|r| r.from_fork));
        assert!(has(&|r| r.labels.is_empty()));
        assert!(has(&|r| r.labels.len() >= 3));
        assert!(has(&|r| r.title.chars().count() > 80), "a title that wraps");
    }

    #[test]
    fn filters_split_the_list_like_the_provider_does() {
        let open = rows(StateFilter::Open);
        let closed = rows(StateFilter::Closed);
        let all = rows(StateFilter::All);
        assert!(open.iter().all(|r| r.state == RequestState::Open));
        assert!(closed.iter().all(|r| r.state != RequestState::Open));
        assert_eq!(open.len() + closed.len(), all.len());
        assert!(
            closed.iter().any(|r| r.state == RequestState::Merged)
                && closed.iter().any(|r| r.state == RequestState::Closed)
        );
    }

    #[test]
    fn details_cover_checks_reviews_and_comments() {
        let kinds = |ci: Ci| ci.checks(ForgeKind::GitHub);
        let states = |checks: &[Check], state| checks.iter().any(|c| c.state == state);
        assert_eq!(kinds(Ci::None), vec![]);
        assert!(
            kinds(Ci::Passing)
                .iter()
                .all(|c| c.state == CheckState::Success)
        );
        assert!(states(&kinds(Ci::Failing), CheckState::Failure));
        assert!(states(&kinds(Ci::Running), CheckState::Pending));
        assert!(states(&kinds(Ci::Mixed), CheckState::Neutral));
        let busy = details(ForgeKind::GitHub, BUSY_NUMBER, NOW).unwrap();
        assert_eq!(busy.summary.state, RequestState::Open);
        assert!(!busy.summary.draft && busy.mergeable == Some(true));
        assert!(busy.head_sha.is_some_and(|sha| sha.len() == 40));
        assert!(
            busy.checks.iter().all(|c| c.state == CheckState::Success),
            "the merge scenes show an enabled merge box"
        );
        assert!(!busy.reviews.is_empty() && !busy.comments.is_empty());
        assert!(details(ForgeKind::GitHub, 427, NOW).unwrap().summary.draft);
        assert_eq!(
            details(ForgeKind::GitHub, 420, NOW).unwrap().mergeable,
            Some(false)
        );
        assert!(details(ForgeKind::GitHub, 1, NOW).is_none());
    }

    #[test]
    fn gitlab_reports_what_gitlab_reports() {
        let github = details(ForgeKind::GitHub, 428, NOW).unwrap();
        let gitlab = details(ForgeKind::GitLab, 428, NOW).unwrap();
        assert!(github.commits.is_some() && github.additions.is_some());
        assert!(gitlab.commits.is_none() && gitlab.additions.is_none());
        assert!(gitlab.summary.web_url.contains("/-/merge_requests/428"));
        assert!(github.summary.web_url.ends_with("/pull/428"));
        let draft = |kind| details(kind, 427, NOW).unwrap().summary.title;
        assert!(draft(ForgeKind::GitLab).starts_with("Draft: Rewrite"));
        assert!(draft(ForgeKind::GitHub).starts_with("Rewrite"));
    }

    #[test]
    fn the_data_does_not_depend_on_the_clock() {
        let a = details(ForgeKind::GitHub, 428, NOW).unwrap();
        assert_eq!(a, details(ForgeKind::GitHub, 428, NOW).unwrap());
        // The same distance from "now", whenever now is.
        let b = details(ForgeKind::GitHub, 428, NOW + 12_345).unwrap();
        assert_eq!(
            NOW - a.summary.updated_at,
            NOW + 12_345 - b.summary.updated_at
        );
        // Requests older than a week keep their fixed dates.
        let old = details(ForgeKind::GitHub, 422, NOW).unwrap();
        assert_eq!(
            old.summary.updated_at,
            details(ForgeKind::GitHub, 422, NOW + 999)
                .unwrap()
                .summary
                .updated_at
        );
    }

    #[test]
    fn times_read_as_the_comments_intend() {
        let at = |w: When| w.at(NOW);
        assert!(NOW - at(When::minutes(18)) > 18 * 60 && NOW - at(When::minutes(18)) < 19 * 60);
        assert!(NOW - at(When::hours(2)) > 2 * 3600 && NOW - at(When::hours(2)) < 3 * 3600);
        assert!(NOW - at(When::days(3)) > 3 * 86_400 && NOW - at(When::days(3)) < 4 * 86_400);
        // Every relative time stays inside the week Canopy words as "3 days ago".
        for row in ROWS {
            for when in [row.created, row.updated] {
                if let When::Ago(seconds) = when {
                    assert!(seconds < 7 * 86_400, "#{}", row.number);
                }
            }
        }
    }

    #[test]
    fn fixed_dates_are_the_ones_named() {
        use chrono::{TimeZone, Utc};
        let unix = |y, m, d, h, min| {
            Utc.with_ymd_and_hms(y, m, d, h, min, 0)
                .unwrap()
                .timestamp()
        };
        assert_eq!(SEP_14.at(0), unix(2026, 9, 14, 9, 30));
        assert_eq!(SEP_09.at(0), unix(2026, 9, 9, 16, 5));
        assert_eq!(SEP_02.at(0), unix(2026, 9, 2, 11, 45));
        assert_eq!(AUG_27.at(0), unix(2026, 8, 27, 8, 10));
    }

    fn demo_tab(spec: &str) -> (RepoTab, egui::Context) {
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            "fixture".into(),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let ctx = egui::Context::default();
        Scenario::parse(spec)
            .expect("a documented value")
            .install(&ctx, &mut tab, NOW);
        (tab, ctx)
    }

    /// Whether the tab's hosting handle has a thread behind it: only a running thread keeps the
    /// events channel open, so a closed one means nothing can answer, or reach the network.
    fn has_thread(tab: &RepoTab) -> bool {
        let host = tab.host.as_ref().expect("the demo installs a handle");
        !matches!(
            host.events.try_recv(),
            Err(crossbeam_channel::TryRecvError::Disconnected)
        )
    }

    /// Security pass N3: a click on Reload or a row in a hand-run `--screenshot` must not send a
    /// request to the real host. The demo's handle has no thread, so none of its jobs can run.
    #[test]
    fn the_demo_has_no_api_thread_to_reach_the_network_with() {
        for spec in ["list", "428", "gitlab:427", "loading", "error", "no-token"] {
            let (mut tab, ctx) = demo_tab(spec);
            assert!(!has_thread(&tab), "{spec} started an API thread");
            // What the view does on a click, and on a repository refresh.
            tab.load_requests();
            tab.select_request(425);
            tab.host_job(crate::hosting::HostJob::List(StateFilter::All));
            tab.host_job(crate::hosting::HostJob::Details(425));
            tab.ensure_host(&ctx, Some("not-a-token".into()), true);
            assert!(!has_thread(&tab), "{spec}: a click started an API thread");
            assert!(
                tab.host
                    .as_ref()
                    .expect("still installed")
                    .events
                    .try_recv()
                    .is_err(),
                "{spec}: something answered"
            );
        }
    }

    #[test]
    fn updated_never_precedes_created() {
        for row in ROWS {
            assert!(
                row.updated.at(NOW) >= row.created.at(NOW),
                "#{}",
                row.number
            );
        }
    }
}
