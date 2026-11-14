//! State of one open repository: the latest snapshot, selections and in-flight work.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use canopy_core::forge::{Forge, ForgeKind};
use canopy_core::{CommitDetails, DiffSource, FileDiff, FileEntry, Oid, PullConfig, RepoSnapshot};
use canopy_hosting::{MergeMethod, RequestDetails, RequestQuery, RequestSummary, StateFilter};

use crate::highlight::Highlights;
use crate::hosting::{
    CliState, HostAction, HostEvent, HostJob, HostWorker, ListKey, rate_limit_pause,
};
use crate::request_view::{RequestDensity, RequestView};
use crate::settings::{DiffPrefs, PullChoice};
use crate::views::requests::row_standing;
use crate::worker::{DiffKey, Event, Job, Op, OpKind, Worker};

/// The main views of a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Commit graph and details.
    History,
    /// Staging area and commit composer.
    Changes,
    /// Pull requests (GitHub) or merge requests (GitLab).
    Requests,
}

/// Whether Canopy trusts a repository (`trust.rs`). An untrusted one is read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoTrust {
    /// The repository thread has not answered yet: nothing is asked or shown, and the thread
    /// refuses what an untrusted repository may not do all the same.
    Unknown,
    Trusted,
    Untrusted,
}

/// A sidebar row picked by the user (or, on opening, the checked-out branch): it shows selected
/// while History shows the commit it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarItem {
    Branch(String),
    RemoteBranch { remote: String, branch: String },
    Tag(String),
    Stash(Oid),
    Worktree(PathBuf),
}

/// Past this many local branches the merged markers skip the squash check, which runs one git
/// process per branch that is not plainly merged; the cheaper checks still run.
const SQUASH_CHECK_LIMIT: usize = 150;

/// Which local branches are merged into `base`, by name.
#[derive(Debug, Clone)]
pub struct MergedMarks {
    /// What the branches were compared with: the base branch and its remote-tracking branch.
    pub bases: Vec<String>,
    /// Every local branch but the base, by name, with the check that matched.
    pub rows: Arc<[canopy_core::MergedBranch]>,
    reasons: std::collections::HashMap<String, canopy_core::MergedReason>,
}

impl MergedMarks {
    pub fn new(bases: Vec<String>, rows: Arc<[canopy_core::MergedBranch]>) -> Self {
        let reasons = rows
            .iter()
            .filter_map(|b| Some((b.name.clone(), b.reason?)))
            .collect();
        Self {
            bases,
            rows,
            reasons,
        }
    }

    /// The base as the user sees it (`develop`, `main`, `origin/main`).
    pub fn shown(&self) -> &str {
        self.bases.first().map_or("", String::as_str)
    }

    /// The check that found `branch` merged; `None` when it is not or is unknown.
    pub fn reason(&self, branch: &str) -> Option<canopy_core::MergedReason> {
        self.reasons.get(branch).copied()
    }
}

/// What the merged markers depend on: the base, every branch's tip and every remote-tracking
/// branch's (an upstream that went away shows there).
fn merged_key(snap: &RepoSnapshot, bases: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bases.hash(&mut hasher);
    for b in &snap.branches {
        (&b.name, b.target, b.is_head).hash(&mut hasher);
    }
    for b in &snap.remote_branches {
        (&b.remote, &b.branch, b.target).hash(&mut hasher);
    }
    hasher.finish()
}

/// The sidebar sections a new tab starts with closed: only Branches is the daily surface. The
/// user's own opening or closing of a section replaces this in [`RepoTab::collapsed`]. A saved
/// set lists the closed sections, so a section added here opens in repositories that have one.
const SIDEBAR_CLOSED_AT_START: [&str; 5] =
    ["worktrees", "remotes", "tags", "stashes", "submodules"];

/// What a merge, rebase, pull, cherry-pick or revert that stopped on conflicts asks of the user.
/// When git holds their uncommitted changes aside (`--autostash`), they are not in the stash
/// list: it says where they are and when they come back. A stash that was applied or popped
/// (`stash`) leaves nothing to continue or abort and is kept in the stash list: it says so.
fn stopped_body(autostashed: bool, stash: bool) -> String {
    if stash {
        return String::from(
            "Resolve each conflicted file in Changes. The stash stays in the stash list until you drop it.",
        );
    }
    let mut body = String::from(
        "Resolve each conflicted file in Changes, then continue, or abort to go back to where you were.",
    );
    if autostashed {
        body.push_str(
            " Your uncommitted changes are held by git meanwhile and come back when you finish or abort.",
        );
    }
    body
}

/// Whether `message`, a hosting error's text, is the host refusing the sign-in
/// (`canopy_hosting::Error::Unauthorized`).
pub(crate) fn credential_refused(message: &str) -> bool {
    message.starts_with("The server refused the request (check the sign-in")
}

/// Whether `message`, a hosting error's text, is the server not being reached
/// (`canopy_hosting::Error::Network`): no sign-in would change that.
pub(crate) fn host_unreachable(message: &str) -> bool {
    message.starts_with("Could not reach the server: ")
}

/// How the history list and selected commit share the content area.
#[derive(Debug, Clone, Copy)]
pub struct HistoryLayout {
    pub expanded: bool,
    pub compact_details: bool,
    /// The current viewport shows either the list or details at a time.
    pub compact: bool,
}

impl Default for HistoryLayout {
    fn default() -> Self {
        Self {
            expanded: false,
            compact_details: true,
            compact: false,
        }
    }
}

impl HistoryLayout {
    pub fn show_list(&mut self) {
        self.expanded = false;
        self.compact_details = false;
    }
}

/// Why the details of a commit could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailsError {
    pub oid: Oid,
    /// The message to show: git's own explanation.
    pub message: String,
    /// A partial clone that has not downloaded the commit and can fetch it, so trying again can
    /// work (the details pane offers a Retry button only then).
    pub can_download: bool,
}

/// A file's blame, as asked for and as read.
#[derive(Debug, Clone)]
pub struct BlameRead {
    pub path: String,
    pub rev: Option<Oid>,
    /// `None` while the worker reads it.
    pub result: Option<Result<Arc<canopy_core::Blame>, String>>,
    /// The characters of the longest line of `result`, for the width of the view.
    pub longest: usize,
}

/// A file whose diff is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSelection {
    pub source: DiffSource,
    pub entry: FileEntry,
    pub conflicted: bool,
}

impl FileSelection {
    /// The commit to blame the file at: the one whose diff is shown, else the working tree.
    pub fn blame_rev(&self) -> Option<Oid> {
        match self.source {
            DiffSource::Commit(oid) => Some(oid),
            _ => None,
        }
    }

    /// Whether git has history to blame the file by: not a file the commit deleted, and in the
    /// working tree not one that is new, untracked, deleted or conflicted.
    pub fn can_blame(&self) -> bool {
        match self.source {
            DiffSource::Commit(_) => self.entry.kind != canopy_core::ChangeKind::Deleted,
            _ => {
                !self.conflicted
                    && !matches!(
                        self.entry.kind,
                        canopy_core::ChangeKind::Added
                            | canopy_core::ChangeKind::Untracked
                            | canopy_core::ChangeKind::Deleted
                            | canopy_core::ChangeKind::Conflicted
                    )
            }
        }
    }
}

/// Lines picked in one hunk of the shown diff, for staging, unstaging or discarding just them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineSelection {
    pub hunk: usize,
    /// Indices into the hunk's lines (added or removed lines only).
    pub lines: BTreeSet<usize>,
    /// Where a Shift-click range starts.
    pub anchor: usize,
}

/// The conflict resolver's state for one conflicted file.
#[derive(Debug, Clone)]
pub struct Resolver {
    pub path: String,
    pub kind: Option<canopy_core::ConflictKind>,
    /// The parsed working-tree file, or why it cannot be resolved line by line.
    pub file: Option<Result<Arc<canopy_core::ConflictFile>, String>>,
    /// The choice for each conflict, in order.
    pub choices: Vec<Option<canopy_core::Choice>>,
    /// The conflict keyboard actions apply to.
    pub focus: usize,
    /// Scroll the focused conflict into view next frame.
    pub reveal: bool,
    /// The conflict being edited by hand, with its text.
    pub editing: Option<(usize, String)>,
    /// Long unchanged stretches the user expanded (segment indices).
    pub expanded: HashSet<usize>,
    /// Show the common ancestor of each conflict (diff3 files).
    pub show_base: bool,
    /// Measured heights of the resolver's blocks, so off-screen ones are skipped.
    pub layout: crate::views::resolve::Layout,
    /// What the current side is called when git labels it only `HEAD`: the checked-out branch.
    pub current: Option<String>,
}

impl Resolver {
    pub(crate) fn new(path: String) -> Self {
        Self {
            path,
            kind: None,
            file: None,
            choices: Vec::new(),
            focus: 0,
            reveal: false,
            editing: None,
            expanded: HashSet::new(),
            show_base: false,
            layout: crate::views::resolve::Layout::default(),
            current: None,
        }
    }

    /// Whether leaving this file would discard unsaved resolution work.
    pub fn has_draft(&self) -> bool {
        self.editing.is_some() || self.choices.iter().any(Option::is_some)
    }

    /// Conflicts in the file (0 while loading or for whole-file conflicts).
    pub fn conflicts(&self) -> usize {
        match &self.file {
            Some(Ok(f)) => f.conflicts(),
            _ => 0,
        }
    }

    /// Conflicts with a choice.
    pub fn resolved(&self) -> usize {
        self.choices.iter().filter(|c| c.is_some()).count()
    }

    /// Moves the focus to the next (`delta` 1) or previous (-1) conflict, preferring unresolved
    /// ones.
    pub fn move_focus(&mut self, delta: isize) {
        let n = self.conflicts();
        if n == 0 {
            return;
        }
        let start = self.focus.min(n - 1);
        let shift = delta.unsigned_abs() % n;
        let step = |i: usize| {
            if delta < 0 {
                (i + n - shift) % n
            } else {
                (i + shift) % n
            }
        };
        let mut i = step(start);
        for _ in 0..n {
            if self.choices.get(i).is_some_and(Option::is_none) {
                break;
            }
            i = step(i);
        }
        if self.choices.iter().all(Option::is_some) {
            i = step(start);
        }
        self.focus = i;
        self.reveal = true;
    }

    /// Takes a fresh parse of the file. A choice survives only if its conflict is still there
    /// with the same content at the same position; edits in an editor reset the rest.
    pub fn reload(
        &mut self,
        kind: Option<canopy_core::ConflictKind>,
        file: Result<Arc<canopy_core::ConflictFile>, String>,
    ) {
        let old: Vec<canopy_core::ConflictRegion> = match &self.file {
            Some(Ok(f)) => f.regions().cloned().collect(),
            _ => Vec::new(),
        };
        let new: Vec<&canopy_core::ConflictRegion> = match &file {
            Ok(f) => f.regions().collect(),
            Err(_) => Vec::new(),
        };
        let choices: Vec<Option<canopy_core::Choice>> = new
            .iter()
            .enumerate()
            .map(|(i, region)| {
                (old.get(i) == Some(*region))
                    .then(|| self.choices.get(i).cloned().flatten())
                    .flatten()
            })
            .collect();
        let edited_region_changed = self
            .editing
            .as_ref()
            .is_some_and(|(i, _)| old.get(*i).is_none() || old.get(*i) != new.get(*i).copied());
        if edited_region_changed
            || choices.len() != self.choices.len()
            || choices.iter().zip(&self.choices).any(|(a, b)| a != b)
        {
            self.editing = None;
        }
        self.focus = self.focus.min(choices.len().saturating_sub(1));
        self.choices = choices;
        self.kind = kind;
        self.file = Some(file);
        self.layout = crate::views::resolve::Layout::default();
    }

    /// Clears the focused conflict's choice (and any hand edit of it).
    pub fn clear_focused(&mut self) {
        if let Some(slot) = self.choices.get_mut(self.focus) {
            *slot = None;
            if self.editing.as_ref().is_some_and(|(i, _)| *i == self.focus) {
                self.editing = None;
            }
            self.reveal = true;
        }
    }

    /// Starts editing the focused conflict by hand, from its current result or our side.
    pub fn edit_focused(&mut self) {
        let Some(Ok(file)) = &self.file else { return };
        let Some(region) = file.regions().nth(self.focus) else {
            return;
        };
        // The same starting text as the Edit button: the current result, or both sides.
        let choice = self
            .choices
            .get(self.focus)
            .cloned()
            .flatten()
            .unwrap_or(canopy_core::Choice::OursThenTheirs);
        self.editing = Some((self.focus, region.resolve(&choice).join("\n")));
        self.reveal = true;
    }

    /// Sets the focused conflict's choice and moves on to the next unresolved one.
    pub fn choose(&mut self, choice: canopy_core::Choice) {
        if let Some(slot) = self.choices.get_mut(self.focus) {
            *slot = Some(choice);
            if self.choices.iter().any(Option::is_none) {
                self.move_focus(1);
            }
        }
    }
}

/// Whether `program` is reported missing for the first time in this run: one toast for it, not
/// one for every repository that needs it.
fn first_report(program: &str) -> bool {
    static TOLD: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut told = TOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if told.iter().any(|p| p == program) {
        return false;
    }
    told.push(program.to_owned());
    true
}

/// How a [`Notice`] reaches the user; with `ok` it decides the notification's topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// A card for the outcome: Finished when it worked, Error when it failed.
    Card,
    /// Something waits on the user (conflicts to resolve, a program to install), so it is an
    /// attention notice and not an error.
    Attention,
    /// Work the user did not start failed: the center and the bell hold it, no card.
    Quiet,
}

/// Outcome of a finished operation, surfaced as a toast by the app.
#[derive(Debug)]
pub struct Notice {
    pub ok: bool,
    pub delivery: Delivery,
    pub title: String,
    pub body: String,
}

/// The leading verbs of operation labels (`Op::label`, `HostJob`'s): as the label says it, as a
/// finished title says it and as a failed one does.
const VERBS: &[(&str, &str, &str)] = &[
    ("Force pushing", "Force pushed", "force push"),
    ("Aborting", "Aborted", "abort"),
    ("Adding", "Added", "add"),
    ("Amending", "Amended", "amend"),
    ("Applying", "Applied", "apply"),
    ("Approving", "Approved", "approve"),
    ("Changing", "Changed", "change"),
    ("Checking", "Checked", "check"),
    ("Cherry-picking", "Cherry-picked", "cherry-pick"),
    ("Closing", "Closed", "close"),
    ("Commenting", "Commented", "comment"),
    ("Committing", "Committed", "commit"),
    ("Continuing", "Continued", "continue"),
    ("Creating", "Created", "create"),
    ("Deleting", "Deleted", "delete"),
    ("Discarding", "Discarded", "discard"),
    ("Dropping", "Dropped", "drop"),
    ("Fetching", "Fetched", "fetch"),
    ("Merging", "Merged", "merge"),
    ("Popping", "Popped", "pop"),
    ("Pulling", "Pulled", "pull"),
    ("Pushing", "Pushed", "push"),
    ("Rebasing", "Rebased", "rebase"),
    ("Removing", "Removed", "remove"),
    ("Renaming", "Renamed", "rename"),
    ("Reopening", "Reopened", "reopen"),
    ("Resolving", "Resolved", "resolve"),
    ("Reverting", "Reverted", "revert"),
    ("Saving", "Saved", "save"),
    ("Squashing", "Squashed", "squash"),
    ("Staging", "Staged", "stage"),
    ("Stashing", "Stashed", "stash"),
    ("Tagging", "Tagged", "tag"),
    ("Taking", "Took", "take"),
    ("Syncing", "Synced", "sync"),
    ("Unstaging", "Unstaged", "unstage"),
    ("Updating", "Updated", "update"),
    ("Untracking", "Stopped tracking", "stop tracking"),
];

/// A finished operation's notification title from its progress label: "Creating branch x"
/// becomes "Created branch x", or "Could not create branch x" when it failed.
pub fn outcome_title(label: &str, ok: bool) -> String {
    let found = VERBS.iter().find_map(|&(doing, done, verb)| {
        let rest = label.strip_prefix(doing)?;
        (rest.is_empty() || rest.starts_with(' ')).then_some((done, verb, rest))
    });
    match (found, ok) {
        (Some((done, _, rest)), true) => format!("{done}{rest}"),
        (Some((_, verb, rest)), false) => format!("Could not {verb}{rest}"),
        (None, true) => format!("{label} — done"),
        (None, false) => format!("{label} failed"),
    }
}

/// How many names a notification lists before "and N more".
const NAMES_SHOWN: usize = 3;

/// What git's `->` lines (fetch and push summaries) say happened to refs, by the name of the ref
/// on the far side.
#[derive(Debug, Default, PartialEq, Eq)]
struct RefChanges {
    new_branches: Vec<String>,
    updated: Vec<String>,
    forced: Vec<String>,
    deleted: Vec<String>,
    new_tags: Vec<String>,
    moved_tags: Vec<String>,
}

impl RefChanges {
    fn parse(out: &str) -> Self {
        let mut changes = Self::default();
        for line in out.lines() {
            let mut chars = line.chars();
            if chars.next() != Some(' ') {
                continue;
            }
            let Some(flag) = chars.next() else { continue };
            let rest = line.get(2..).unwrap_or_default().trim_start();
            let name = match rest.split_once("->") {
                Some((_, to)) => to.split_whitespace().next(),
                // `git push --delete` names the ref alone.
                None => rest
                    .strip_prefix("[deleted]")
                    .and_then(|r| r.split_whitespace().next()),
            };
            let Some(name) = name.map(str::to_owned) else {
                continue;
            };
            let slot = match flag {
                '*' if rest.starts_with("[new tag]") => &mut changes.new_tags,
                '*' => &mut changes.new_branches,
                't' => &mut changes.moved_tags,
                '+' => &mut changes.forced,
                '-' => &mut changes.deleted,
                ' ' if rest
                    .split_whitespace()
                    .next()
                    .is_some_and(|r| r.contains("..")) =>
                {
                    &mut changes.updated
                }
                _ => continue,
            };
            slot.push(name);
        }
        changes
    }

    /// "New branch: origin/x. Updated: origin/main.", or `None` when nothing changed.
    fn describe(&self) -> Option<String> {
        let groups = [
            ("New branch", "New branches", &self.new_branches),
            ("Updated", "Updated", &self.updated),
            ("Force-updated", "Force-updated", &self.forced),
            ("Removed", "Removed", &self.deleted),
            ("New tag", "New tags", &self.new_tags),
            ("Moved tag", "Moved tags", &self.moved_tags),
        ];
        let sentences: Vec<String> = groups
            .iter()
            .filter(|(_, _, names)| !names.is_empty())
            .map(|(one, many, names)| {
                let label = if names.len() == 1 { one } else { many };
                let mut list = names
                    .iter()
                    .take(NAMES_SHOWN)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if names.len() > NAMES_SHOWN {
                    list = format!("{list} and {} more", names.len() - NAMES_SHOWN);
                }
                format!("{label}: {list}.")
            })
            .collect();
        (!sentences.is_empty()).then(|| sentences.join(" "))
    }
}

/// Whether a line of git's output is transfer chatter rather than a result.
fn is_transfer_chatter(line: &str) -> bool {
    const PROGRESS: &[&str] = &[
        "remote:",
        "From ",
        "To ",
        "Enumerating objects",
        "Counting objects",
        "Compressing objects",
        "Writing objects",
        "Receiving objects",
        "Resolving deltas",
        "Unpacking objects",
        "Delta compression",
        "Total ",
    ];
    PROGRESS.iter().any(|p| line.starts_with(p))
}

/// What a successfully finished operation says in words, for its notification. Git's own text is
/// kept only for failures; a success is told from the ref lines and results in `out`, never as
/// the transfer chatter around them. `subject` is the message the commit composer held and
/// `branch` the checked-out branch, both from before the operation.
fn success_body(kind: &OpKind, out: &str, subject: &str, branch: Option<&str>) -> String {
    let changes = RefChanges::parse(out);
    match kind {
        OpKind::Commit => match (subject.is_empty(), out.trim()) {
            (false, id) if !id.is_empty() => format!("{subject} ({id})"),
            (false, _) => subject.to_owned(),
            (true, id) => id.to_owned(),
        },
        OpKind::Fetch => changes
            .describe()
            .unwrap_or_else(|| "Nothing new on any remote.".to_owned()),
        OpKind::Pull => {
            let branch = branch.unwrap_or("The branch");
            let mut body = if out.lines().any(|l| l.starts_with("Successfully rebased")) {
                format!("Rebased {branch} onto its upstream.")
            } else if out.lines().any(|l| l.starts_with("Merge made by")) {
                format!("{branch} now has everything from its upstream.")
            } else {
                format!("{branch} is up to date with its upstream.")
            };
            if out.contains("Applied autostash") {
                body.push_str(" Your uncommitted changes are back.");
            }
            body
        }
        OpKind::Push
        | OpKind::Other
        | OpKind::Worktree
        | OpKind::Squash(_)
        | OpKind::SafeDelete(_)
        | OpKind::Merge
        | OpKind::Stash
        | OpKind::Pop
        | OpKind::Branch
        | OpKind::Lfs
        | OpKind::Publish => {
            if let Some(told) = changes.describe() {
                told
            } else if out.lines().any(|l| l.trim() == "Everything up-to-date") {
                "Everything was already up to date on the remote.".to_owned()
            } else {
                let kept: Vec<&str> = out
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !is_transfer_chatter(l))
                    .collect();
                kept.join("\n").trim().to_owned()
            }
        }
    }
}

/// How the last announced operation ended, for the action bar to show on its tool for a moment.
#[derive(Debug, Clone)]
pub struct Finished {
    /// The operation's label (`Op::label`).
    pub label: String,
    /// Stable operation identity; labels are display text, not control state.
    pub kind: OpKind,
    pub ok: bool,
    pub at: Instant,
}

/// How old the request list may be when its view opens before it is loaded again.
const REQUESTS_STALE_AFTER: Duration = Duration::from_secs(60);

/// How long the filter field must rest before its text is put to the host as a search: a key
/// per call would spend the host's search quota (GitHub: 30 a minute) on half-typed words.
const REQUEST_SEARCH_DELAY: Duration = Duration::from_millis(400);

/// How often the open Requests view refreshes its list while automatic fetching is off.
pub const REQUESTS_REFRESH_EVERY: Duration = Duration::from_secs(60);

/// The longest wait between timed refreshes after repeated failures (never below the interval).
const REQUESTS_REFRESH_BACKOFF_CAP: Duration = Duration::from_mins(15);

/// What the timed refresh of the open Requests view may do right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStep {
    /// A condition is not met (another view, an unfocused window, a load under way, ...).
    Paused,
    /// Nothing to do yet; the next refresh falls due after this long.
    Wait(Duration),
    /// Refresh now.
    Due,
}

/// The conditions under which the open Requests view refreshes itself.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts of the frame, each a plain yes or no"
)]
pub struct RefreshGate {
    pub view: View,
    pub focused: bool,
    /// The tab has a hosting thread (hosting on, consent given, a forge).
    pub hosting: bool,
    /// The host reported a token. Without one a refresh would spend the anonymous rate limit
    /// (60 requests an hour on GitHub) that Reload, actions and picture lookups share.
    pub token: bool,
    /// The list on screen loaded without error.
    pub loaded: bool,
    /// A list load is under way, or a fetch, pull or push that refreshes the list when it ends.
    pub busy: bool,
}

/// The timed refresh's bookkeeping, apart from the clock.
#[derive(Debug, Default)]
pub struct RefreshTimer {
    /// The configured cadence, shared by timed and post-sync retries (zero before configured).
    pub interval: Duration,
    /// A quiet refresh was sent and its list has not come back.
    pub in_flight: bool,
    /// Quiet refreshes in a row that failed; each doubles the wait.
    pub failures: u32,
    /// When the newest quiet refresh failed.
    pub failed_at: Option<Instant>,
    /// A rate limit asked the host to be left alone until then.
    pub held_until: Option<Instant>,
}

/// `interval` doubled per failed refresh in a row, up to [`REQUESTS_REFRESH_BACKOFF_CAP`] (which
/// never shortens it).
fn backoff_wait(interval: Duration, failures: u32) -> Duration {
    interval
        .saturating_mul(1_u32.checked_shl(failures).unwrap_or(u32::MAX))
        .min(REQUESTS_REFRESH_BACKOFF_CAP.max(interval))
}

/// How long a rate limit that lifts at `reset` (unix seconds, when the host said) holds the
/// timer at `now`: until then, an hour at most, else the longest backoff.
fn rate_limit_hold(reset: Option<i64>, now: i64) -> Duration {
    u64::try_from(rate_limit_pause(reset, now, 0) - now)
        .ok()
        .filter(|&secs| secs > 0)
        .map_or(REQUESTS_REFRESH_BACKOFF_CAP, Duration::from_secs)
}

/// Decides, from instants and flags alone, whether the open Requests view refreshes now: the
/// wait is `interval`, doubled per failure up to [`REQUESTS_REFRESH_BACKOFF_CAP`], counted from
/// `last` (the newest load or failed attempt).
pub fn request_refresh_step(
    now: Instant,
    last: Instant,
    interval: Duration,
    failures: u32,
    gate: RefreshGate,
) -> RefreshStep {
    let ready = gate.view == View::Requests
        && gate.focused
        && gate.hosting
        && gate.token
        && gate.loaded
        && !gate.busy;
    if !ready {
        return RefreshStep::Paused;
    }
    let due = last + backoff_wait(interval, failures);
    if now >= due {
        RefreshStep::Due
    } else {
        RefreshStep::Wait(due - now)
    }
}

/// How long a request's loaded details are shown again, on returning to it, without asking the
/// host: browsing A, B and back to A is not three loads of A.
const REQUEST_DETAILS_CACHE_TTL: Duration = Duration::from_secs(60);

/// How many requests' details the cache keeps; the oldest is dropped first.
const REQUEST_DETAILS_CACHE_MAX: usize = 16;

/// One request's details as loaded, with the project and time they came from. Keyed by forge so
/// equal numbers in different projects never share an entry.
#[derive(Debug)]
struct CachedDetails {
    forge: Forge,
    number: u64,
    loaded_at: Instant,
    details: Arc<RequestDetails>,
}

/// An unsent comment, isolated by forge project and request number.
#[derive(Debug, Clone)]
pub struct RequestCommentDraft {
    forge: Forge,
    number: u64,
    body: String,
}

/// Pull or merge requests of the tab's forge.
#[derive(Debug)]
pub struct Requests {
    pub filter: StateFilter,
    /// Shared so a frame can hold the list while it mutates the tab, without copying it.
    pub list: Option<Result<Arc<[RequestSummary]>, String>>,
    /// The request filter field takes the caret on the next frame (the filter command).
    pub focus_filter: bool,
    /// The numbers of the requests the list showed on its last frame, in the order it showed
    /// them (the view's filters and sort applied); `None` before it was drawn. The arrow keys
    /// walk this order.
    pub shown: Option<Vec<u64>>,
    pub loading: bool,
    pub selected: Option<u64>,
    pub details: Option<Result<Arc<RequestDetails>, String>>,
    pub details_loading: bool,
    /// Rejects responses from an earlier selection or reload, even for the same request.
    pub details_generation: u64,
    /// Failure policy of the current details generation, independent of the list response.
    pub details_quiet_refresh: bool,
    /// Recently loaded details, oldest first, so returning to a request needs no new load.
    /// Emptied by a reload, an action's result and a restart of the hosting thread (`restart_host`).
    details_cache: Vec<CachedDetails>,
    /// An action in flight ("Merging PR #12").
    pub busy: Option<String>,
    pub comment: String,
    /// Exact submitted body; a late result must not discard text edited since submission.
    pub pending_comment: Option<(u64, String)>,
    pub merge_method: MergeMethod,
    /// What the provider's CLI said about its sign-in when the hosting thread started: the account
    /// it gave a credential for, or why it gave none. `None` while unknown.
    pub cli: Option<CliState>,
    /// The host refused the credential on the newest read (a list or a request's details); cleared by
    /// the next read or action it accepted. An action's refusal does not set it: the host also
    /// answers 403 to a locked conversation or an archived repository.
    pub token_rejected: bool,
    /// When the list was last loaded.
    pub loaded_at: Option<Instant>,
    /// The newest load was a quiet refresh (no spinner): if it fails, the list on screen stays.
    /// Any load the user asked for clears it.
    pub quiet_refresh: bool,
    /// The timer that refreshes the list while the view stays open.
    pub refresh: RefreshTimer,
    /// The host holds requests after the pages loaded.
    pub more: bool,
    /// How many pages of the list are loaded (the first, then one per page the end of the list
    /// asked for).
    pub pages: usize,
    /// A page after the first is on its way.
    pub loading_more: bool,
    /// The newest page after the first failed: the end of the list stops asking until the next
    /// load of the first page.
    pub more_failed: bool,
    /// The rows of the pages after the first, as they arrived; a reload of the first page keeps
    /// them, a reload the user asked for drops them.
    tail: Vec<RequestSummary>,
    /// What the host was asked to search for in the list on screen (or on its way): nothing
    /// until the plain list proved longer than what is loaded.
    pub query: RequestQuery,
    /// The state whose plain list the host holds more of than a page, so filters and sorts must
    /// be put to the host to reach every request.
    truncated: Option<StateFilter>,
    /// Where the next page of a search starts (GitHub pages a search by cursor).
    cursor: Option<String>,
    /// When the search for the view's text and filters is sent, once the typing has paused.
    search_due: Option<Instant>,
}

impl Default for Requests {
    fn default() -> Self {
        Self {
            filter: StateFilter::Open,
            list: None,
            shown: None,
            focus_filter: false,
            loading: false,
            selected: None,
            details: None,
            details_loading: false,
            details_generation: 0,
            details_quiet_refresh: false,
            details_cache: Vec::new(),
            busy: None,
            comment: String::new(),
            pending_comment: None,
            merge_method: MergeMethod::Merge,
            cli: None,
            token_rejected: false,
            loaded_at: None,
            quiet_refresh: false,
            refresh: RefreshTimer::default(),
            more: false,
            pages: 1,
            loading_more: false,
            more_failed: false,
            tail: Vec::new(),
            query: RequestQuery::default(),
            truncated: None,
            cursor: None,
            search_due: None,
        }
    }
}

impl Requests {
    /// Whether the CLI gave a credential; `None` until the hosting thread has asked.
    pub fn has_credential(&self) -> Option<bool> {
        self.cli.as_ref().map(Result::is_ok)
    }

    /// `head`, the first page, followed by the later pages' rows it does not repeat.
    fn with_tail(&self, mut head: Vec<RequestSummary>) -> Vec<RequestSummary> {
        let tail = self
            .tail
            .iter()
            .filter(|t| !head.iter().any(|h| h.number == t.number))
            .cloned()
            .collect::<Vec<_>>();
        head.extend(tail);
        head
    }

    /// Puts a later page's rows into the tail: a row already there is replaced (the page came
    /// again with its checks and reviews), the rest are added.
    fn add_page(&mut self, rows: Vec<RequestSummary>) {
        for row in rows {
            match self.tail.iter_mut().find(|t| t.number == row.number) {
                Some(existing) => *existing = row,
                None => self.tail.push(row),
            }
        }
    }

    /// The details of `number` in `forge` loaded less than [`REQUEST_DETAILS_CACHE_TTL`] ago.
    fn cached_details(&self, forge: &Forge, number: u64) -> Option<Arc<RequestDetails>> {
        self.details_cache
            .iter()
            .find(|c| c.number == number && &c.forge == forge)
            .filter(|c| c.loaded_at.elapsed() < REQUEST_DETAILS_CACHE_TTL)
            .map(|c| Arc::clone(&c.details))
    }

    /// Remembers a load that just arrived, replacing the request's older entry and dropping
    /// the oldest entries beyond [`REQUEST_DETAILS_CACHE_MAX`].
    fn cache_details(&mut self, forge: &Forge, number: u64, details: &Arc<RequestDetails>) {
        self.forget_details(forge, number);
        self.details_cache
            .retain(|c| c.loaded_at.elapsed() < REQUEST_DETAILS_CACHE_TTL);
        while self.details_cache.len() >= REQUEST_DETAILS_CACHE_MAX {
            self.details_cache.remove(0);
        }
        self.details_cache.push(CachedDetails {
            forge: forge.clone(),
            number,
            loaded_at: Instant::now(),
            details: Arc::clone(details),
        });
    }

    fn forget_details(&mut self, forge: &Forge, number: u64) {
        self.details_cache
            .retain(|c| c.number != number || &c.forge != forge);
    }
}

/// Facts derived from a snapshot once, when it arrives, so frames do not recompute them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SnapIndex {
    /// Indices into `remote_branches` for each remote, in `remotes` order.
    pub remote_branches: Vec<Vec<usize>>,
    /// The widest graph row, in lanes.
    pub graph_lanes: u16,
    /// Visible submodule indices and their status, including unstaged content changes.
    pub submodules: Vec<(usize, canopy_core::SubmoduleState)>,
    /// The local branches grouped into folders by their prefixes.
    pub branch_tree: Vec<crate::branch_tree::Node>,
}

impl SnapIndex {
    pub fn new(s: &RepoSnapshot) -> Self {
        let mut remote_branches = vec![Vec::new(); s.remotes.len()];
        let position: std::collections::HashMap<&str, usize> = s
            .remotes
            .iter()
            .enumerate()
            .map(|(i, r)| (r.name.as_str(), i))
            .collect();
        for (i, b) in s.remote_branches.iter().enumerate() {
            if let Some(&r) = position.get(b.remote.as_str()) {
                remote_branches[r].push(i);
            }
        }
        let mut index = Self {
            remote_branches,
            graph_lanes: s.graph.iter().map(|g| g.width).max().unwrap_or(1),
            submodules: Vec::new(),
            branch_tree: crate::branch_tree::build(&s.branches),
        };
        index.update_submodules(s);
        index
    }

    fn update_submodules(&mut self, s: &RepoSnapshot) {
        let modified: HashSet<&str> = s.status.unstaged.iter().map(|e| e.path.as_str()).collect();
        self.submodules = s
            .submodules
            .iter()
            .enumerate()
            .filter(|(_, sm)| !sm.is_removal_staged())
            .map(|(i, sm)| {
                let state = match sm.state() {
                    canopy_core::SubmoduleState::Clean if modified.contains(sm.path.as_str()) => {
                        canopy_core::SubmoduleState::Dirty
                    }
                    state => state,
                };
                (i, state)
            })
            .collect();
    }
}

/// The icons a repository's tab can show (Settings → This repository): a stored key, the glyph
/// and its name. The first is the default.
pub const TAB_ICONS: &[(&str, &str, &str)] = &[
    ("folder", egui_phosphor::regular::FOLDER_SIMPLE, "Folder"),
    ("code", egui_phosphor::regular::CODE, "Code"),
    (
        "terminal",
        egui_phosphor::regular::TERMINAL_WINDOW,
        "Terminal",
    ),
    ("globe", egui_phosphor::regular::GLOBE, "Web"),
    ("mobile", egui_phosphor::regular::DEVICE_MOBILE, "Mobile"),
    ("database", egui_phosphor::regular::DATABASE, "Data"),
    ("cube", egui_phosphor::regular::CUBE, "Package"),
    ("book", egui_phosphor::regular::BOOK_OPEN, "Docs"),
    ("palette", egui_phosphor::regular::PALETTE, "Design"),
    ("game", egui_phosphor::regular::GAME_CONTROLLER, "Game"),
    ("flask", egui_phosphor::regular::FLASK, "Experiment"),
    ("rocket", egui_phosphor::regular::ROCKET_LAUNCH, "Launch"),
    ("briefcase", egui_phosphor::regular::BRIEFCASE, "Work"),
    ("house", egui_phosphor::regular::HOUSE, "Personal"),
    ("star", egui_phosphor::regular::STAR, "Favourite"),
    ("lightning", egui_phosphor::regular::LIGHTNING, "Fast"),
];

/// The glyph for an icon key; the folder for none or an unknown one.
pub fn tab_icon(key: Option<&str>) -> &'static str {
    TAB_ICONS
        .iter()
        .find(|(k, _, _)| Some(*k) == key)
        .map_or(TAB_ICONS[0].1, |(_, glyph, _)| glyph)
}

/// The folder's name, which a tab shows unless the repository has a name of its own.
pub fn folder_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// One repository tab.
///
/// Large data from the worker (snapshot, commit details, diff, conflict file) is held in `Arc`s:
/// a frame clones the pointer, never the data, so its cost does not grow with the repository.
#[derive(Debug)]
pub struct RepoTab {
    /// Which branches are merged into the base: the newest answer to [`Self::read_merged`],
    /// `None` before the first.
    pub merged: Option<Result<MergedMarks, String>>,
    /// The newest request for [`Self::merged`]; an older reply is dropped.
    pub merged_serial: u64,
    /// The serial of the reply [`Self::merged`] holds (a form waits for one at least as new as
    /// its own request).
    pub merged_applied: u64,
    /// What the last automatic request was made for ([`merged_key`]), so a refresh that moved
    /// no branch does not run the checks again.
    merged_for: Option<u64>,
    /// The newest [`Self::read_base_compare`] request; an older reply is dropped.
    pub base_serial: u64,
    /// Its answer, `None` while it is on its way.
    pub base_result: Option<Result<canopy_core::PullPreview, String>>,
    /// Git LFS: the files stored with it, whether it works here, and the dialog's lists.
    pub lfs: crate::lfs::LfsState,
    pub stale_serial: u64,
    pub stale_result: Option<Result<Arc<[canopy_core::StaleCandidate]>, String>>,
    /// The newest [`Self::read_worktree_removal`] request; an older reply is dropped.
    pub worktree_removal_serial: u64,
    /// What removing that working folder would delete, `None` while it is on its way.
    pub worktree_removal: Option<Result<Box<canopy_core::WorktreeRemoval>, String>>,
    /// The newest [`Self::read_worktree_forget`] request; an older reply is dropped.
    pub worktree_forget_serial: u64,
    /// What git would forget, `None` while it is on its way.
    pub worktree_forget: Option<Result<Box<canopy_core::WorktreeForget>, String>>,
    /// The other tabs' working folders and their titles, set each frame by the app: a folder
    /// open in another tab cannot be removed from here.
    pub open_elsewhere: Vec<(PathBuf, String)>,
    file_history_serial: u64,
    change_search_serial: u64,
    /// The newest [`Self::search_changes`] answer, `None` while it is on its way or before a search.
    pub change_search: Option<Result<Arc<canopy_core::ChangeSearch>, String>>,
    /// The newest [`Self::read_file_history`] answer, `None` while it is on its way.
    pub file_history: Option<Result<Arc<canopy_core::FileHistory>, String>>,
    insights_serial: u64,
    pub insights: Option<Result<Arc<crate::worker::Insights>, String>>,
    blame_serial: u64,
    /// The blame being shown: of this file, as of this commit (`None`: the working tree), and
    /// what the worker read; the result is `None` while it is read.
    pub blame: Option<BlameRead>,
    pub ignore_query: Option<crate::worker::IgnoreQuery>,
    pub ignore_result: Option<Result<Arc<crate::worker::IgnoreData>, String>>,
    ignore_serial: u64,
    pub path: PathBuf,
    pub identity: Option<canopy_core::identity::Identity>,
    pub identity_error: Option<String>,
    /// What the tab says: the repository's own name from Settings, else its folder's.
    pub name: String,
    /// The tab's icon ([`TAB_ICONS`]).
    pub icon: &'static str,
    pub worker: Worker,
    pub snap: Option<Arc<RepoSnapshot>>,
    /// Derived from `snap` when it arrives.
    pub snap_index: SnapIndex,
    /// Bumped whenever the snapshot or the request list is replaced (the command palette
    /// rebuilds its entries when it changes).
    pub generation: u64,
    pub load_error: Option<String>,
    pub view: View,
    pub selected_commit: Option<Oid>,
    pub history_layout: HistoryLayout,
    /// Whether the on-demand history search field is visible.
    pub history_filter_open: bool,
    /// Focus the history filter when the list next renders.
    pub focus_history_filter: bool,
    pub details: Option<Arc<CommitDetails>>,
    /// How the selected commit's signature verified, once the worker has said.
    pub signature: Option<(Oid, canopy_core::SignatureStatus)>,
    /// Set when loading the details of `selected_commit` failed.
    pub details_error: Option<DetailsError>,
    pub file: Option<FileSelection>,
    /// Unstaged files picked with Ctrl-click or Shift-click besides the open one, for
    /// "Stage selected" (see [`Self::stage_targets`]).
    pub marked: BTreeSet<String>,
    pub diff: Option<Arc<FileDiff>>,
    /// Row layout of `diff` for the diff view, rebuilt only when the diff or layout changes.
    pub diff_layout: Option<crate::views::diff::DiffLayout>,
    pub diff_error: Option<String>,
    /// The diff whose download failed in a partial clone: it is not asked for again (the file
    /// watcher refreshes the status often) until the user chooses Retry or the file again.
    pub failed_diff: Option<DiffKey>,
    /// The failure of the last automatic fetch that was shown, so the same one is not shown
    /// again at every interval; a fetch that works, or a change of trusted folders, forgets it.
    auto_fetch_failure: Option<String>,
    /// Files whose diff a partial clone could not download, for the "not downloaded" marker on
    /// their rows.
    pub not_downloaded: HashSet<(DiffSource, String)>,
    pub pending_diff: Option<DiffKey>,
    /// A requested diff has not answered yet, including refreshes that keep the old diff visible.
    awaiting_diff: bool,
    /// Diff context and whitespace for this repository.
    pub diff_prefs: DiffPrefs,
    /// Every repository's diff context and whitespace (Settings), kept current by the app.
    pub diff_default: DiffPrefs,
    /// Syntax highlighting of `diff`.
    pub highlights: Option<Arc<Highlights>>,
    /// Lines picked in the diff for line-level staging.
    pub line_sel: Option<LineSelection>,
    /// The selected conflict resolver, retained while hidden if it has authored work.
    pub resolver: Option<Resolver>,
    /// Resolvers with authored work for the other conflicted files, kept until their file leaves
    /// the conflicted list.
    pub parked_resolvers: Vec<Resolver>,
    /// Side-by-side diff and syntax highlighting (from the settings).
    pub split_diff: bool,
    pub highlight_diff: bool,
    /// Comfortable or dense request list items (from the settings).
    pub request_density: RequestDensity,
    /// Hint at a commit message that is too wide (from the settings; set each frame the
    /// Changes view is drawn).
    pub check_message: bool,
    /// Settings' calendar date format for absolute dates drawn from the tab (blame); set each
    /// frame.
    pub iso_dates: bool,
    pub filter: String,
    pub filtered: Option<Vec<usize>>,
    pub filter_for: String,
    pub commit_summary: String,
    pub commit_body: String,
    /// The merge message put into the commit boxes when a merge began, to drop it again if the
    /// merge ends without a commit and nobody edited it.
    pub(crate) merge_prefill: Option<(String, String)>,
    /// The summary and description the commit template filled in, while they stay as filled:
    /// such a message still counts as unwritten for the amend, merge and squash prefills.
    pub(crate) template_prefill: Option<(String, String)>,
    pub amend: bool,
    /// Move keyboard focus to the commit summary next frame.
    pub focus_summary: bool,
    /// Take keyboard focus out of the commit boxes next frame (a commit just cleared them).
    pub blur_composer: bool,
    pub running: Option<String>,
    pub running_kind: Option<OpKind>,
    pub finished: Option<Finished>,
    /// How this repository pulls by Canopy's settings (kept current by the app every frame).
    pub pull: PullChoice,
    /// The git options last sent to the worker (`None`: none yet).
    pub git_options: Option<canopy_core::GitOptions>,
    /// What the repository's configuration asks a pull to do on the current branch; `None` until
    /// it is read, or when it cannot be.
    pub pull_config: Option<PullConfig>,
    /// The branch `pull_config` was last asked for (`None`: detached or unborn), once asked.
    pull_config_asked: bool,
    tools_asked: bool,
    /// The commit template was asked for; a skipped read may retry after trust changes.
    template_asked: bool,
    /// A template was received, even if an existing draft prevented it from being applied.
    template_loaded: bool,
    pull_config_branch: Option<String>,
    /// A branch `git branch -d` refused as not fully merged; the app asks before forcing it.
    pub force_delete_offer: Option<String>,
    /// How the last `git worktree add` went, for the New worktree dialog (git's words when it
    /// refused); the app shows a refusal there, or in a toast once the dialog is gone.
    pub worktree_result: Option<Result<(), String>>,
    /// Where a force push would go, once the repository thread has worked it out: the app asks
    /// before pushing.
    pub force_push_offer: Option<canopy_core::PushTarget>,
    /// A pull waits for the user's decision (diverged branches, or likely conflicts).
    pub pull_decision: Option<crate::worker::PullDecision>,
    pub progress: Option<String>,
    pub commit_limit: usize,
    /// Centre the selected commit next frame.
    pub scroll_to_selected: bool,
    /// Scroll just enough to show the selected commit next frame.
    pub reveal_selected: bool,
    /// The sidebar sections and remotes the user (or the tab's start, [`SIDEBAR_CLOSED_AT_START`])
    /// closed, by key.
    pub collapsed: HashSet<String>,
    /// The saved sidebar state has been applied after the first repository snapshot.
    pub sidebar_restored: bool,
    /// A section was toggled this frame, awaiting persistence by the app.
    pub sidebar_changed: bool,
    /// The request list's filters and sort for this repository.
    pub request_view: RequestView,
    /// The saved request view has been applied after the first repository snapshot.
    pub request_view_restored: bool,
    /// The sidebar row last clicked ([`RepoTab::sidebar_shows`]).
    pub sidebar_pick: Option<SidebarItem>,
    /// GitHub and GitLab remotes: (remote name, forge).
    pub forges: Vec<(String, Forge)>,
    /// The API thread for the main forge remote.
    pub host: Option<HostWorker>,
    /// Pull/merge request state for the main forge remote.
    pub requests: Requests,
    /// Drafts survive hosting initialization and worktree tab replacement for this run.
    pub request_comments: Vec<RequestCommentDraft>,
    /// The promisor remote a read needed but Canopy did not download from, because the
    /// repository is outside every trusted folder; the status bar offers to trust it.
    pub downloads_refused: Option<String>,
    /// Folders the worker found behind trusting this repository, including external Git
    /// directories.
    pub trust_folders: Vec<PathBuf>,
    /// Whether Canopy trusts the repository, as the worker last said.
    pub trust: RepoTrust,
    /// Whether the tab has asked the user about trust (they may have answered Don't trust, which
    /// leaves the repository read-only until the banner's Trust).
    pub trust_asked: bool,
    /// The newest version of the trusted list this tab has been told about: a refusal from an
    /// older one is out of date.
    trust_version: u64,
    pub avatars: crate::avatars::Avatars,
    /// When the next automatic fetch is due (`None`: not scheduled yet). Set by the app's
    /// auto-fetch pass, with a stagger so tabs do not fall due together, and again after every
    /// fetch.
    pub next_fetch: Option<Instant>,
    /// The auto-fetch interval `next_fetch` was scheduled for; a different one reschedules.
    pub fetch_every: Option<Duration>,
    /// An automatic fetch was sent to the worker and has not finished, so it counts against the
    /// limit on background fetches (`MAX_BACKGROUND_FETCHES`). Cleared by the worker's answer to
    /// that fetch (also when it could not run it, as when the repository would not open); a tab whose
    /// worker thread has ended is not counted ([`Worker::is_alive`]).
    pub auto_fetch_pending: bool,
    /// The quiet fetches sent and not yet answered, oldest first, each `true` when it was the
    /// automatic one. The worker runs a tab's operations in the order sent, and a quiet fetch
    /// always ends in exactly one `OpFinished` (it cannot stop to ask or on conflicts, as a pull
    /// or a merge can, and a batch that cannot run answers it with an error), so the first quiet
    /// `OpFinished` belongs to the first of these.
    quiet_fetches: std::collections::VecDeque<bool>,
}

impl RepoTab {
    /// Opens a tab and asks for the first snapshot at once (the app opens tabs with
    /// [`Self::unloaded`] and loads the shown one).
    #[cfg(test)]
    pub fn new(
        path: PathBuf,
        ctx: &egui::Context,
        commit_limit: usize,
        diff_prefs: DiffPrefs,
    ) -> Self {
        let worker = Worker::spawn(path.clone(), ctx);
        worker.send(Job::Refresh(commit_limit));
        Self::with_worker(path, commit_limit, diff_prefs, worker)
    }

    /// Sends an automatic fetch (the timer's, never the user's) and counts it against the limit on
    /// background fetches until it finishes.
    pub fn auto_fetch(&mut self, prune: bool) {
        self.auto_fetch_pending = self.send_op(Op::Fetch { prune, quiet: true }, true);
    }

    /// Opens a tab that loads nothing until it is first shown ([`Self::load`]): no thread, no
    /// file watcher, no snapshot. The tab strip shows its name meanwhile.
    pub fn unloaded(
        path: PathBuf,
        ctx: &egui::Context,
        commit_limit: usize,
        diff_prefs: DiffPrefs,
    ) -> Self {
        let worker = Worker::lazy(path.clone(), ctx);
        Self::with_worker(path, commit_limit, diff_prefs, worker)
    }

    /// Whether the tab's repository thread and file watcher run.
    pub fn is_loaded(&self) -> bool {
        self.worker.is_started()
    }

    /// Starts the thread and watcher of a tab opened with [`Self::unloaded`] and asks for its
    /// first snapshot. Returns whether it did so now, `false` for a tab that was loaded already.
    pub fn load(&mut self) -> bool {
        if !self.worker.start() {
            return false;
        }
        self.worker.send(Job::Refresh(self.commit_limit));
        // The background fetch is scheduled afresh from now, not from when the tab was opened.
        self.next_fetch = None;
        true
    }

    pub(crate) fn with_worker(
        path: PathBuf,
        commit_limit: usize,
        diff_prefs: DiffPrefs,
        worker: Worker,
    ) -> Self {
        let name = folder_name(&path);
        Self {
            path,
            lfs: crate::lfs::LfsState::default(),
            file_history_serial: 0,
            change_search_serial: 0,
            change_search: None,
            file_history: None,
            insights_serial: 0,
            insights: None,
            blame_serial: 0,
            blame: None,
            ignore_query: None,
            merged: None,
            merged_serial: 0,
            merged_applied: 0,
            merged_for: None,
            base_serial: 0,
            base_result: None,
            stale_serial: 0,
            stale_result: None,
            worktree_removal_serial: 0,
            worktree_removal: None,
            worktree_forget_serial: 0,
            worktree_forget: None,
            open_elsewhere: Vec::new(),
            ignore_result: None,
            ignore_serial: 0,
            identity: None,
            identity_error: None,
            name,
            icon: tab_icon(None),
            worker,
            snap: None,
            snap_index: SnapIndex::default(),
            generation: 0,
            load_error: None,
            view: View::History,
            selected_commit: None,
            history_layout: HistoryLayout::default(),
            history_filter_open: false,
            focus_history_filter: false,
            details: None,
            signature: None,
            details_error: None,
            file: None,
            diff: None,
            diff_layout: None,
            diff_error: None,
            failed_diff: None,
            auto_fetch_failure: None,
            not_downloaded: HashSet::new(),
            pending_diff: None,
            awaiting_diff: false,
            diff_prefs,
            diff_default: diff_prefs,
            highlights: None,
            line_sel: None,
            marked: BTreeSet::new(),
            resolver: None,
            parked_resolvers: Vec::new(),
            split_diff: false,
            request_density: RequestDensity::default(),
            highlight_diff: true,
            check_message: true,
            iso_dates: false,
            filter: String::new(),
            filtered: None,
            filter_for: String::new(),
            commit_summary: String::new(),
            commit_body: String::new(),
            merge_prefill: None,
            template_prefill: None,
            amend: false,
            focus_summary: false,
            blur_composer: false,
            running: None,
            running_kind: None,
            finished: None,
            pull: PullChoice::default(),
            git_options: None,
            pull_config: None,
            pull_config_asked: false,
            tools_asked: false,
            template_asked: false,
            template_loaded: false,
            pull_config_branch: None,
            force_delete_offer: None,
            worktree_result: None,
            force_push_offer: None,
            pull_decision: None,
            progress: None,
            commit_limit,
            scroll_to_selected: false,
            reveal_selected: false,
            collapsed: SIDEBAR_CLOSED_AT_START.map(str::to_owned).into(),
            sidebar_restored: false,
            sidebar_changed: false,
            request_view: RequestView::default(),
            request_view_restored: false,
            sidebar_pick: None,
            forges: Vec::new(),
            host: None,
            requests: Requests::default(),
            request_comments: Vec::new(),
            downloads_refused: None,
            trust_folders: Vec::new(),
            trust: RepoTrust::Unknown,
            trust_asked: false,
            trust_version: 0,
            avatars: crate::avatars::Avatars::default(),
            next_fetch: None,
            fetch_every: None,
            auto_fetch_pending: false,
            quiet_fetches: std::collections::VecDeque::new(),
        }
    }

    /// Whether the message boxes hold nothing the user wrote: empty, or still exactly what the
    /// commit template filled in. The amend, merge and squash prefills may replace such a message.
    pub fn message_is_unwritten(&self) -> bool {
        (self.commit_summary.trim().is_empty() && self.commit_body.trim().is_empty())
            || self
                .template_prefill
                .as_ref()
                .is_some_and(|(summary, body)| {
                    *summary == self.commit_summary && *body == self.commit_body
                })
    }

    /// Whether the composer can submit a commit through any input path.
    pub fn can_commit(&self) -> bool {
        self.snap.as_ref().is_some_and(|snap| {
            (!snap.status.staged.is_empty()
                || self.amend
                || snap.state == canopy_core::RepoState::Merge)
                && !self.commit_summary.trim().is_empty()
                && self.running.is_none()
                && snap.status.conflicted.is_empty()
                && (!self.amend || snap.state == canopy_core::RepoState::Clean)
        })
    }

    /// Queues a mutation.
    pub fn op(&mut self, op: Op) {
        self.send_op(op, false);
    }

    /// Queues `op` and says whether the worker took it (a tab that has not loaded drops it). A
    /// quiet fetch is remembered, to tell its end from the automatic one's.
    fn send_op(&mut self, op: Op, automatic: bool) -> bool {
        let sent = self.worker.is_started();
        if sent && matches!(op, Op::Fetch { quiet: true, .. }) {
            self.quiet_fetches.push_back(automatic);
        }
        self.worker.send(Job::Op(op));
        sent
    }

    /// Asks for the Git LFS dialog's lists: the tracked patterns (needs git-lfs) and the locks
    /// (the LFS server).
    pub fn read_lfs_lists(&mut self) {
        self.lfs.dialog_seen = true;
        let patterns = self.lfs.next_patterns_serial();
        self.worker.send(Job::LfsPatterns(patterns));
        let locks = self.lfs.next_locks_serial();
        self.worker.send(Job::LfsLocks(locks));
    }

    /// Asks for the history of `path` (relative to the repository root), through renames, from
    /// the commit `from` (`HEAD` when `None`).
    pub fn read_file_history(&mut self, from: Option<Oid>, path: String) {
        self.file_history_serial = self.file_history_serial.wrapping_add(1);
        self.file_history = None;
        self.worker.send(Job::FileHistory {
            serial: self.file_history_serial,
            from,
            path,
        });
    }

    /// Searches the history for commits that added or removed `query` (or, with `regex`, changed
    /// lines matching it); the answer replaces [`Self::change_search`].
    pub fn search_changes(&mut self, query: String, regex: bool) {
        self.change_search_serial = self.change_search_serial.wrapping_add(1);
        self.change_search = None;
        self.worker.send(Job::SearchChanges {
            serial: self.change_search_serial,
            query,
            regex,
        });
    }

    /// Opens `entry`'s commit in History with the file's diff at that commit: the working tree
    /// is only read, never changed.
    pub fn show_file_version(&mut self, entry: &canopy_core::FileHistoryEntry) {
        self.view = View::History;
        self.history_layout.show_list();
        self.filter.clear();
        self.select_commit(entry.oid);
        self.scroll_to_selected = true;
        self.select_file(FileSelection {
            source: DiffSource::Commit(entry.oid),
            entry: FileEntry {
                path: entry.path.clone(),
                old_path: entry.old_path.clone(),
                kind: entry.kind.unwrap_or(canopy_core::ChangeKind::Modified),
                additions: 0,
                deletions: 0,
                lfs: false,
                old_mode: None,
                new_mode: None,
            },
            conflicted: false,
        });
    }

    /// Asks for the blame of `path` as of `rev` (the working tree when `None`).
    pub fn read_blame(&mut self, path: String, rev: Option<Oid>) {
        self.send_blame(path.clone(), rev);
        self.blame = Some(BlameRead {
            path,
            rev,
            result: None,
            longest: 0,
        });
    }

    fn send_blame(&mut self, path: String, rev: Option<Oid>) {
        self.blame_serial = self.blame_serial.wrapping_add(1);
        self.worker.send(Job::Blame {
            serial: self.blame_serial,
            path,
            rev,
        });
    }

    /// Back to the file's diff; a reply still on its way is ignored.
    pub fn close_blame(&mut self) {
        self.blame = None;
        self.blame_serial = self.blame_serial.wrapping_add(1);
    }

    /// Shows the blame of the selected file in place of its diff, when it has one.
    pub fn blame_file(&mut self) {
        if let Some(sel) = self.file.as_ref().filter(|sel| sel.can_blame()) {
            let (path, rev) = (sel.entry.path.clone(), sel.blame_rev());
            self.read_blame(path, rev);
        }
    }

    pub fn read_insights(&mut self) {
        self.insights_serial = self.insights_serial.wrapping_add(1);
        self.insights = None;
        self.worker.send(Job::Insights(self.insights_serial));
    }

    pub fn read_ignore(&mut self, query: crate::worker::IgnoreQuery) {
        self.ignore_serial = self.ignore_serial.wrapping_add(1);
        self.ignore_query = Some(query.clone());
        self.ignore_result = None;
        self.worker.send(Job::Ignore {
            serial: self.ignore_serial,
            query,
        });
    }

    /// Asks which local branches are merged into the base ([`MergedMarks`]). The branches' names
    /// and tips decide whether it is needed again: a refresh that moved none leaves the markers
    /// as they are, since the squash check runs one git process per branch. `force` asks anyway
    /// (a cleanup form wants a reading made now). Returns the request's serial.
    pub fn read_merged(&mut self, force: bool) -> u64 {
        let Some(snap) = self.snap.as_ref() else {
            return self.merged_serial;
        };
        let Some(base) = snap.workflow.base.as_ref() else {
            self.merged = None;
            self.merged_for = None;
            return self.merged_serial;
        };
        let key = merged_key(snap, &base.references);
        if !force && self.merged_for == Some(key) {
            return self.merged_serial;
        }
        self.merged_for = Some(key);
        self.merged_serial = self.merged_serial.wrapping_add(1);
        self.worker.send(Job::MergedBranches {
            serial: self.merged_serial,
            bases: base.references.clone(),
            squash: snap.branches.len() <= SQUASH_CHECK_LIMIT,
        });
        self.merged_serial
    }

    /// Asks what bringing `base` into the checked-out branch would do; the answer lands in
    /// [`Self::base_result`]. Returns the request's serial.
    pub fn read_base_compare(&mut self, base: &str) -> u64 {
        self.base_serial = self.base_serial.wrapping_add(1);
        self.base_result = None;
        self.worker.send(Job::BaseCompare {
            serial: self.base_serial,
            base: base.to_owned(),
        });
        self.base_serial
    }

    /// A new cleanup form always gets a fresh preview; replies for older forms are discarded.
    pub fn read_stale(&mut self, older_than_days: u32) -> u64 {
        self.stale_serial = self.stale_serial.wrapping_add(1);
        self.stale_result = None;
        self.worker.send(Job::StaleCandidates {
            serial: self.stale_serial,
            older_than_days,
        });
        self.stale_serial
    }

    /// The other tabs' folders, as core wants them.
    pub fn open_folders(&self) -> Vec<canopy_core::OpenFolder> {
        self.open_elsewhere
            .iter()
            .map(|(path, name)| canopy_core::OpenFolder {
                path: path.clone(),
                name: name.clone(),
            })
            .collect()
    }

    /// Asks what removing the linked working folder `path` would delete; the answer lands in
    /// [`Self::worktree_removal`]. Returns the request's serial.
    pub fn read_worktree_removal(&mut self, path: PathBuf) -> u64 {
        self.worktree_removal_serial = self.worktree_removal_serial.wrapping_add(1);
        self.worktree_removal = None;
        self.worker.send(Job::WorktreeRemoval {
            serial: self.worktree_removal_serial,
            path,
            open_in: self.open_folders(),
        });
        self.worktree_removal_serial
    }

    /// Asks what `git worktree prune` would forget; the answer lands in
    /// [`Self::worktree_forget`]. Returns the request's serial.
    pub fn read_worktree_forget(&mut self) -> u64 {
        self.worktree_forget_serial = self.worktree_forget_serial.wrapping_add(1);
        self.worktree_forget = None;
        self.worker.send(Job::WorktreeForget {
            serial: self.worktree_forget_serial,
        });
        self.worktree_forget_serial
    }

    /// Requests a full refresh.
    pub fn refresh(&self) {
        // A tab that has not loaded has nothing to refresh: its first load reads everything.
        if !self.is_loaded() {
            return;
        }
        self.worker.send(Job::Refresh(self.commit_limit));
    }

    /// A refresh the user asked for (the Refresh command, the Retry on the empty Changes list): it
    /// also tries again a download of the working tree status that failed, which the refreshes
    /// the file watcher makes leave alone.
    pub fn refresh_by_user(&self) {
        let retry = self
            .snap
            .as_ref()
            .and_then(|s| s.status_unavailable.as_ref())
            .is_some_and(|u| u.can_download);
        if retry {
            self.worker.send(Job::RetryDownloads);
        } else {
            self.refresh();
        }
    }

    /// The strategy the Pull tool uses, and the configuration it came from when Canopy follows
    /// a strategy the user's git configuration sets.
    pub fn pull_default(&self) -> (canopy_core::PullStrategy, Option<&PullConfig>) {
        let configured = self.pull_config.as_ref().filter(|c| c.strategy.is_some());
        let strategy = self.pull.resolve(configured.and_then(|c| c.strategy));
        let from_config = configured.filter(|_| self.pull.follow_config);
        (strategy, from_config)
    }

    /// The repository has no remote at all.
    pub fn is_local_only(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.remotes.is_empty())
    }

    /// Recomputes which remotes are GitHub or GitLab.
    pub fn update_forges(&mut self, gitlab_hosts: &[String]) {
        self.forges = self
            .snap
            .as_ref()
            .map(|s| {
                s.remotes
                    .iter()
                    .filter_map(|r| {
                        let url = r.url.as_deref()?;
                        Some((r.name.clone(), Forge::parse(url, gitlab_hosts)?))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    /// The forge of `remote`.
    pub fn forge_of(&self, remote: &str) -> Option<&Forge> {
        self.forges
            .iter()
            .find(|(r, _)| r == remote)
            .map(|(_, f)| f)
    }

    /// The main forge remote: the current branch's upstream remote, else `origin`, else the
    /// first GitHub or GitLab remote.
    pub fn forge(&self) -> Option<&(String, Forge)> {
        let upstream_remote = self
            .snap
            .as_ref()
            .and_then(|s| s.current_branch())
            .and_then(|b| b.upstream.as_deref())
            .and_then(|u| u.split('/').next());
        upstream_remote
            .and_then(|r| self.forges.iter().find(|(n, _)| n == r))
            .or_else(|| self.forges.iter().find(|(n, _)| n == "origin"))
            .or_else(|| self.forges.first())
    }

    /// `branch` is the main forge remote's default branch (`refs/remotes/<r>/HEAD`).
    pub fn is_default_branch(&self, branch: &str) -> bool {
        let Some((remote, _)) = self.forge() else {
            return false;
        };
        self.snap
            .as_ref()
            .and_then(|s| s.remotes.iter().find(|r| &r.name == remote))
            .and_then(|r| r.default_branch.as_deref())
            .is_some_and(|d| d == branch)
    }

    /// Starts (or restarts, when the forge changed) the API thread.
    pub fn ensure_host(&mut self, ctx: &egui::Context, enabled: bool) {
        let want = if enabled {
            self.forge().map(|(_, f)| f.clone())
        } else {
            None
        };
        if self.host.as_ref().map(|h| &h.forge) == want.as_ref() {
            return;
        }
        self.request_comments = self.request_comment_drafts();
        self.requests = Requests::default();
        self.avatars = crate::avatars::Avatars::default();
        self.host = want.map(|f| HostWorker::spawn(f, ctx.clone()));
        self.load_requests();
    }

    /// Requests one visible author picture, without I/O on the UI thread.
    pub fn request_avatar(&mut self, oid: Oid, email: &str) {
        if let Some(host) = &self.host
            && host.avatars_enabled()
            && self.avatars.request(oid, email)
        {
            host.send(HostJob::Avatar(oid, email.to_owned()));
        }
    }

    /// Restarts the API thread, e.g. after the CLI's sign-in changed.
    pub fn restart_host(&mut self, ctx: &egui::Context, enabled: bool) {
        self.request_comments = self.request_comment_drafts();
        self.requests.comment.clear();
        // Details loaded with the old sign-in may differ from what the new one is allowed to see.
        self.requests.details_cache.clear();
        self.host = None;
        self.ensure_host(ctx, enabled);
    }

    /// Enqueue a fresh details load with an identity that invalidates older responses.
    pub fn request_details(&mut self, number: u64) {
        self.request_details_with_policy(number, false);
    }

    fn request_details_with_policy(&mut self, number: u64, quiet: bool) {
        self.requests.details_quiet_refresh = quiet;
        self.requests.details_generation = self.requests.details_generation.wrapping_add(1);
        self.host_job(HostJob::Details(number, self.requests.details_generation));
    }

    /// The list on screen: its state tab and what the host was asked to search for. A reply for
    /// another key is out of date.
    fn list_key(&self) -> ListKey {
        ListKey {
            state: self.requests.filter,
            query: self.requests.query.clone(),
        }
    }

    /// Whether the host can search this repository's requests: GitLab always, GitHub with a token.
    fn can_search(&self) -> bool {
        self.host
            .as_ref()
            .is_some_and(|h| h.forge.kind == ForgeKind::GitLab)
            || self.requests.has_credential() == Some(true)
    }

    /// What the host should be asked for, given the view: nothing while everything the host holds
    /// is loaded (the loaded rows are filtered and sorted here), the view's text, draft choice,
    /// author and order once the host holds more.
    fn wanted_query(&self) -> RequestQuery {
        if self.requests.truncated == Some(self.requests.filter) && self.can_search() {
            self.request_view.host_query()
        } else {
            RequestQuery::default()
        }
    }

    /// The request view changed (or what the host can do about it did): plans a search for it
    /// once the typing has paused. The rows on screen stay until the answer comes.
    pub fn request_view_changed(&mut self) {
        let due = (self.host.is_some() && self.wanted_query() != self.requests.query)
            .then(|| Instant::now() + REQUEST_SEARCH_DELAY);
        self.requests.search_due = due;
    }

    /// Sends the planned search when its time has come; returns how long until it does.
    pub fn run_search_if_due(&mut self) -> Option<Duration> {
        let due = self.requests.search_due?;
        let now = Instant::now();
        if let Some(wait) = due.checked_duration_since(now) {
            return Some(wait);
        }
        if self.requests.loading {
            // The list on its way answers first; the search follows it.
            return Some(REQUEST_SEARCH_DELAY);
        }
        self.load_requests();
        None
    }

    /// The view the loaded rows are put through: what the host's answer already covers is left
    /// out.
    pub fn local_request_view(&self) -> RequestView {
        self.request_view.remaining(&self.requests.query)
    }

    /// Reloads the request list.
    pub fn load_requests(&mut self) {
        if self.host.is_some() {
            // A reload the user asked for (or an action's aftermath) must not be answered from
            // what was loaded before it.
            self.requests.details_cache.clear();
            self.requests.quiet_refresh = false;
            self.requests.loading = true;
            self.requests.tail.clear();
            self.requests.pages = 1;
            self.requests.more = false;
            self.requests.cursor = None;
            self.requests.loading_more = false;
            self.requests.search_due = None;
            self.requests.query = self.wanted_query();
            self.requests.more_failed = false;
            self.host_job(HostJob::List(self.list_key()));
            // Refreshing the rows must refresh the open request's body and action state too.
            // Otherwise a manual refresh leaves its details stale until another selection.
            if let Some(n) = self.requests.selected
                && self.requests.details.is_some()
            {
                self.requests.details_loading = true;
                self.request_details(n);
            }
        }
    }

    /// Asks for the next page of the list. Nothing happens while one is on its way, when the host
    /// has no more, or without a hosting thread.
    pub fn load_more_requests(&mut self) {
        if self.host.is_none() || !self.requests.more || self.requests.loading_more {
            return;
        }
        self.requests.loading_more = true;
        self.requests.more_failed = false;
        self.host_job(HostJob::ListMore(
            self.list_key(),
            self.requests.pages + 1,
            self.requests.cursor.clone(),
        ));
    }

    /// Loads the request list again without a spinner, keeping the list, the selection and the
    /// open request on screen until the new ones arrive. Does nothing without a hosting thread
    /// (hosting off, no consent, no forge) or while the list was never loaded: a tab nobody
    /// opened asks its forge for nothing.
    pub fn refresh_requests_quietly(&mut self) {
        // One at a time: a refresh in flight (the timer's, say) already answers for this one.
        if self.host.is_none() || self.requests.list.is_none() || self.requests.refresh.in_flight {
            return;
        }
        self.requests.quiet_refresh = true;
        self.requests.refresh.in_flight = true;
        // The list is being revalidated, so what was cached before it may disagree with its rows.
        self.requests.details_cache.clear();
        self.host_job(HostJob::List(self.list_key()));
        if let Some(n) = self.requests.selected
            && self.requests.details.is_some()
        {
            self.request_details_with_policy(n, true);
        }
    }

    /// When the list was last loaded or a quiet refresh of it last failed.
    fn refresh_last(&self) -> Option<Instant> {
        match (self.requests.loaded_at, self.requests.refresh.failed_at) {
            (Some(loaded), Some(failed)) => Some(loaded.max(failed)),
            (loaded, failed) => loaded.or(failed),
        }
    }

    /// Refreshes the list quietly when it was loaded more than [`REQUESTS_STALE_AFTER`] ago:
    /// the Requests view opens on a list as current as the last sync, not as old as the tab. After
    /// failed refreshes it waits as long as the timer would, and it leaves a rate-limited host be.
    pub fn refresh_requests_if_stale(&mut self) {
        let now = Instant::now();
        let timer = &self.requests.refresh;
        let held = timer.held_until.is_some_and(|until| now < until);
        let interval = if timer.failures == 0 {
            REQUESTS_STALE_AFTER
        } else {
            timer.interval.max(REQUESTS_STALE_AFTER)
        };
        let wait = backoff_wait(interval, timer.failures);
        if !self.requests.loading
            && !held
            && self.requests.loaded_at.is_some()
            && self
                .refresh_last()
                .is_some_and(|last| now.saturating_duration_since(last) >= wait)
        {
            self.refresh_requests_quietly();
        }
    }

    /// Refreshes the list quietly when the timer is due (see [`request_refresh_step`]); returns
    /// how long until it next falls due so the frame loop can wake then, or `None` when paused.
    pub fn refresh_requests_on_timer(
        &mut self,
        now: Instant,
        interval: Duration,
        focused: bool,
    ) -> Option<Duration> {
        self.requests.refresh.interval = interval;
        let last = self.refresh_last()?;
        let timer = &self.requests.refresh;
        let syncing = self.running_kind.as_ref().is_some_and(OpKind::syncs_remote);
        let gate = RefreshGate {
            view: self.view,
            focused,
            hosting: self.host.is_some(),
            token: self.requests.has_credential() == Some(true),
            loaded: matches!(self.requests.list, Some(Ok(_))),
            busy: self.requests.loading || timer.in_flight || syncing,
        };
        let held = timer
            .held_until
            .and_then(|until| until.checked_duration_since(now))
            .unwrap_or_default();
        match request_refresh_step(now, last, interval, timer.failures, gate) {
            RefreshStep::Paused => None,
            RefreshStep::Wait(d) => Some(d.max(held)),
            RefreshStep::Due if !held.is_zero() => Some(held),
            RefreshStep::Due => {
                self.refresh_requests_quietly();
                None
            }
        }
    }

    /// Includes the currently edited comment without changing its displayed text.
    pub fn request_comment_drafts(&self) -> Vec<RequestCommentDraft> {
        let mut drafts = self.request_comments.clone();
        if !self.requests.comment.is_empty()
            && let Some(number) = self.requests.selected
            && let Some(host) = &self.host
        {
            drafts.retain(|draft| draft.forge != host.forge || draft.number != number);
            drafts.push(RequestCommentDraft {
                forge: host.forge.clone(),
                number,
                body: self.requests.comment.clone(),
            });
        }
        drafts
    }

    /// Shows one request.
    pub fn select_request(&mut self, number: u64) {
        if self.requests.selected == Some(number) && self.requests.details.is_some() {
            return;
        }
        self.request_comments = self.request_comment_drafts();
        self.requests.selected = Some(number);
        self.requests.details = None;
        self.requests.comment = self
            .host
            .as_ref()
            .and_then(|host| {
                self.request_comments
                    .iter()
                    .position(|draft| draft.forge == host.forge && draft.number == number)
            })
            .map(|index| self.request_comments.remove(index).body)
            .unwrap_or_default();
        let cached = self
            .host
            .as_ref()
            .and_then(|host| self.requests.cached_details(&host.forge, number));
        if let Some(details) = cached {
            self.requests.details = Some(Ok(details));
            self.requests.details_loading = false;
            // An older load still in flight must not land on what is now shown.
            self.requests.details_generation = self.requests.details_generation.wrapping_add(1);
        } else if self.host.is_some() {
            self.requests.details_loading = true;
            self.request_details(number);
        } else {
            // Invalidate an in-flight result from an old host even if no new one is sent.
            self.requests.details_generation = self.requests.details_generation.wrapping_add(1);
        }
    }

    /// Runs an API action.
    pub fn host_job(&self, job: HostJob) {
        if let Some(h) = &self.host {
            h.send(job);
        }
    }

    /// Starts a user action and records the comment whose completion may clear its draft.
    pub fn host_action(&mut self, job: HostJob) {
        let Some(host) = &self.host else { return };
        self.requests.busy = Some(match &job {
            HostJob::Create(_) => format!("Creating {}", host.forge.kind.request_noun()),
            _ => "Working…".into(),
        });
        if let HostJob::Comment(number, body) = &job {
            self.requests.pending_comment = Some((*number, body.clone()));
        }
        self.host_job(job);
    }

    /// Toggles the on-demand history filter.
    pub fn filter_history(&mut self, ctx: &egui::Context) {
        let list_visible = !(self.history_layout.expanded
            || (self.history_layout.compact && self.history_layout.compact_details));
        if self.history_filter_open && self.view == View::History && list_visible {
            self.close_history_filter(ctx);
            return;
        }
        self.history_layout.show_list();
        self.history_filter_open = true;
        self.focus_history_filter = true;
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("history-filter")));
    }

    /// Hides history search and restores the unfiltered graph.
    pub fn close_history_filter(&mut self, ctx: &egui::Context) {
        self.history_filter_open = false;
        self.focus_history_filter = false;
        self.filter.clear();
        ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::new("history-filter")));
    }

    /// Whether the sidebar row `item`, pointing at `target`, shows selected: it is the row last
    /// clicked and History still shows its commit (selecting another commit, or a branch moving,
    /// lets go of it).
    pub fn sidebar_shows(
        &self,
        item: impl FnOnce(&SidebarItem) -> bool,
        target: Option<Oid>,
    ) -> bool {
        self.view == View::History
            && target.is_some()
            && self.selected_commit == target
            && self.sidebar_pick.as_ref().is_some_and(item)
    }

    /// Selects a commit and loads its details.
    pub fn select_commit(&mut self, oid: Oid) {
        if self.selected_commit == Some(oid) {
            return;
        }
        self.selected_commit = Some(oid);
        self.details = None;
        self.signature = None;
        self.details_error = None;
        self.deselect_file();
        self.worker.send(Job::Details(oid));
        self.worker.send(Job::Signature(oid));
    }

    /// Closes the open file and its diff, with any error that was shown on it. A diff still on its
    /// way for that file is no longer wanted.
    pub fn deselect_file(&mut self) {
        self.file = None;
        self.close_blame();
        self.marked.clear();
        self.diff = None;
        self.diff_error = None;
        self.pending_diff = None;
        self.awaiting_diff = false;
        self.clear_obsolete_refusal();
    }

    /// Asks again for the details that failed to load (the Retry button of the details pane).
    pub fn retry_details(&mut self) {
        if let Some(error) = self.details_error.take() {
            self.worker.send(Job::Details(error.oid));
        }
    }

    /// The trusted folders changed, to `version` (`trust::changes()`). A tab whose downloads were
    /// refused asks again for what it was showing, which may now download. A refusal still on its
    /// way from the worker carries an older version and is then found out of date.
    /// A commit template skipped before trust is also requested again, until one is received.
    pub fn trust_changed(&mut self, version: u64) {
        self.trust_version = version;
        self.auto_fetch_failure = None;
        // A tab that has not loaded has no worker to ask; its first load reads the trust anew.
        if !self.is_loaded() {
            return;
        }
        // The worker tells the tab how the change left this repository.
        self.worker.send(Job::RefreshStatus);
        if self.template_asked && !self.template_loaded {
            self.worker.send(Job::CommitTemplate);
        }
        if self.downloads_refused.take().is_some() {
            self.reread();
        }
    }

    fn reread(&mut self) {
        self.failed_diff = None;
        self.not_downloaded.clear();
        self.refresh();
        self.details_error = None;
        if let Some(oid) = self.selected_commit {
            self.worker.send(Job::Details(oid));
            self.worker.send(Job::Signature(oid));
        }
        self.reload_diff();
    }

    /// Hides the "Downloads off" chip once nothing it explains is left on screen. It explains a
    /// refusal's own message in the status, the details or the diff, line counts left undone, and
    /// a commit whose details are still on their way (they may be refused too); an error of
    /// another kind is not its to explain.
    fn clear_obsolete_refusal(&mut self) {
        let refusal = |message: &str| message.contains(crate::worker::NOT_TRUSTED);
        // No snapshot yet: the status is still to come unless loading already failed.
        let status = self.snap.as_ref().is_none_or(|s| {
            s.status_unavailable
                .as_ref()
                .is_some_and(|u| refusal(&u.message))
        }) && self.load_error.as_deref().is_none_or(refusal);
        let details_refused = self.view == View::History
            && self
                .details_error
                .as_ref()
                .is_some_and(|e| refusal(&e.message));
        // With no file open there is no diff to show the error on.
        let diff_refused = self.file.is_some()
            && (self.awaiting_diff || self.diff_error.as_deref().is_some_and(refusal));
        // The worker asks for line counts only of a commit with up to `MAX_STAT_FILES` files.
        let details_open = self.view == View::History
            && self.selected_commit.is_some_and(|oid| {
                self.details_error.is_none()
                    && self.details.as_ref().is_none_or(|d| {
                        d.oid != oid
                            || (!d.stats_known
                                && d.files.len() <= canopy_core::repo::MAX_STAT_FILES)
                    })
            });
        if !(status || details_refused || diff_refused || details_open) {
            self.downloads_refused = None;
        }
    }

    fn diff_key(&self, sel: &FileSelection) -> DiffKey {
        DiffKey {
            source: sel.source,
            path: sel.entry.path.clone(),
            old_path: sel.entry.old_path.clone(),
            conflicted: sel.conflicted,
            context: self.diff_prefs.context,
            whitespace: self.diff_prefs.whitespace.core(),
            highlight: self.highlight_diff,
        }
    }

    /// The unstaged files "Stage selected" stages, in list order: the marked ones and the open
    /// file when it is an unstaged, unconflicted one.
    pub fn stage_targets(&self) -> Vec<String> {
        let Some(snap) = &self.snap else {
            return Vec::new();
        };
        let open = self
            .file
            .as_ref()
            .filter(|f| f.source == DiffSource::Unstaged && !f.conflicted)
            .map(|f| f.entry.path.as_str());
        snap.status
            .unstaged
            .iter()
            .filter(|f| open == Some(f.path.as_str()) || self.marked.contains(&f.path))
            .map(|f| f.path.clone())
            .collect()
    }

    /// Forgets marked files that are no longer unstaged.
    fn prune_marked(&mut self) {
        if let Some(snap) = &self.snap {
            self.marked
                .retain(|p| snap.status.unstaged.iter().any(|f| f.path == *p));
        }
    }

    /// Forgets parked resolvers of files that are no longer conflicted. An unreadable status says
    /// nothing about the conflicts, so it keeps them.
    fn prune_parked(&mut self) {
        if let Some(snap) = self
            .snap
            .as_ref()
            .filter(|s| s.status_unavailable.is_none())
        {
            self.parked_resolvers
                .retain(|r| snap.status.conflicted.iter().any(|f| f.path == r.path));
        }
    }

    /// Shows the diff of a file.
    pub fn select_file(&mut self, sel: FileSelection) {
        let key = self.diff_key(&sel);
        if self.file.as_ref() == Some(&sel)
            && self.diff.is_some()
            && self.pending_diff.as_ref() == Some(&key)
        {
            return;
        }
        if self.file.as_ref() != Some(&sel) {
            self.line_sel = None;
            self.close_blame();
        }
        if !self.file.as_ref().is_some_and(|f| {
            f.source == sel.source
                && f.entry.path == sel.entry.path
                && f.conflicted == sel.conflicted
        }) {
            self.marked.clear();
        }
        if sel.conflicted {
            if self
                .resolver
                .as_ref()
                .is_none_or(|r| r.path != sel.entry.path)
            {
                let next = self
                    .parked_resolvers
                    .iter()
                    .position(|r| r.path == sel.entry.path)
                    .map_or_else(
                        || Resolver::new(sel.entry.path.clone()),
                        |i| self.parked_resolvers.remove(i),
                    );
                if let Some(left) = self.resolver.replace(next).filter(Resolver::has_draft) {
                    self.parked_resolvers.push(left);
                }
            }
            self.worker.send(Job::Conflict(sel.entry.path.clone()));
        } else if self.resolver.as_ref().is_none_or(|r| !r.has_draft()) {
            self.resolver = None;
        }
        self.file = Some(sel);
        self.diff_error = None;
        self.failed_diff = None;
        self.pending_diff = Some(key.clone());
        self.awaiting_diff = true;
        self.worker.send(Job::Diff(key));
    }

    /// Reloads the open diff, keeping the old one on screen until the new one arrives.
    pub fn reload_diff(&mut self) {
        // A blame of the working tree goes stale with the file; the old one stays until the new
        // one arrives.
        if let Some(read) = self.blame.as_ref().filter(|b| b.rev.is_none()) {
            self.send_blame(read.path.clone(), None);
        }
        if let Some(sel) = self.file.clone() {
            if sel.conflicted {
                self.worker.send(Job::Conflict(sel.entry.path.clone()));
            }
            let key = self.diff_key(&sel);
            if self.failed_diff.as_ref() == Some(&key) {
                return;
            }
            self.diff_error = None;
            self.pending_diff = Some(key.clone());
            self.awaiting_diff = true;
            self.worker.send(Job::Diff(key));
        }
    }

    /// Asks again for the diff that could not be downloaded (the Retry button of the diff pane).
    pub fn retry_diff(&mut self) {
        self.failed_diff = None;
        self.reload_diff();
    }

    /// Commits in display order (filtered or all).
    pub fn visible_rows(&mut self) -> Vec<usize> {
        let filtered = self.filtered_rows().map(<[usize]>::to_vec);
        filtered.unwrap_or_else(|| (0..self.snap.as_ref().map_or(0, |s| s.commits.len())).collect())
    }

    /// Moves the commit selection by `delta` rows (clamped); `isize::MIN` jumps to the first row
    /// and `isize::MAX` to the last. With nothing selected, any other move selects the first row.
    pub fn move_commit(&mut self, delta: isize) {
        let rows = self.visible_rows();
        let Some(snap) = &self.snap else { return };
        if rows.is_empty() {
            return;
        }
        let pos = self
            .selected_commit
            .and_then(|s| rows.iter().position(|&i| snap.commits[i].oid == s));
        let last = rows.len() - 1;
        let next = match (pos, delta) {
            (_, isize::MAX) => last,
            (_, isize::MIN) | (None, _) => 0,
            (Some(p), d) => p.saturating_add_signed(d).min(last),
        };
        let oid = snap.commits[rows[next]].oid;
        self.select_commit(oid);
        self.reveal_selected = true;
    }

    /// Working-copy files in display order: conflicts, unstaged, staged.
    pub fn change_rows(&self) -> Vec<FileSelection> {
        let Some(s) = &self.snap else {
            return Vec::new();
        };
        let st = &s.status;
        let mut v: Vec<FileSelection> = st
            .conflicted
            .iter()
            .map(|e| FileSelection {
                source: DiffSource::Unstaged,
                entry: e.clone(),
                conflicted: true,
            })
            .collect();
        v.extend(st.unstaged.iter().map(|e| FileSelection {
            source: DiffSource::Unstaged,
            entry: e.clone(),
            conflicted: false,
        }));
        v.extend(st.staged.iter().map(|e| FileSelection {
            source: DiffSource::Staged,
            entry: e.clone(),
            conflicted: false,
        }));
        v
    }

    /// Moves the file selection in Changes.
    pub fn move_file(&mut self, delta: isize) {
        let rows = self.change_rows();
        if rows.is_empty() {
            return;
        }
        let pos = self
            .file
            .as_ref()
            .and_then(|f| rows.iter().position(|r| r == f));
        let last = rows.len() - 1;
        let next = match pos {
            None => 0,
            Some(p) => p.saturating_add_signed(delta).min(last),
        };
        self.select_file(rows[next].clone());
    }

    /// Moves the request selection.
    pub fn move_request(&mut self, delta: isize) {
        let numbers: Vec<u64> = match (&self.requests.shown, &self.requests.list) {
            (Some(shown), Some(Ok(_))) => shown.clone(),
            (None, Some(Ok(l))) => l.iter().map(|r| r.number).collect(),
            _ => return,
        };
        if numbers.is_empty() {
            return;
        }
        let pos = self
            .requests
            .selected
            .and_then(|n| numbers.iter().position(|x| *x == n));
        let last = numbers.len() - 1;
        let next = match pos {
            None => 0,
            Some(p) => p.saturating_add_signed(delta).min(last),
        };
        self.select_request(numbers[next]);
    }

    /// Keeps the selected working-copy file valid after the status changed.
    fn reconcile_file_selection(&mut self) {
        let Some(snap) = &self.snap else { return };
        let Some(sel) = &self.file else { return };
        if matches!(sel.source, DiffSource::Commit(_)) {
            // The recorded pointers are immutable, but fetching a submodule changes which
            // commits and log entries are available locally.
            if sel.entry.old_mode == Some(0o160_000) || sel.entry.new_mode == Some(0o160_000) {
                self.reload_diff();
            }
            return;
        }
        let st = &snap.status;
        let list = if sel.conflicted {
            &st.conflicted
        } else if sel.source == DiffSource::Staged {
            &st.staged
        } else {
            &st.unstaged
        };
        if list.iter().any(|f| f.path == sel.entry.path) {
            self.reload_diff();
        } else {
            // The file moved to the other list (staged/unstaged) or is gone: follow it if possible.
            let other = if sel.source == DiffSource::Staged {
                (&st.unstaged, DiffSource::Unstaged)
            } else {
                (&st.staged, DiffSource::Staged)
            };
            if let Some(e) = other.0.iter().find(|f| f.path == sel.entry.path).cloned() {
                self.diff = None;
                self.select_file(FileSelection {
                    source: other.1,
                    entry: e,
                    conflicted: false,
                });
            } else {
                self.deselect_file();
            }
        }
    }

    /// The repository identity from the worker; no path resolution happens on the UI thread.
    pub fn sidebar_key(&self) -> Option<&std::path::Path> {
        self.snap.as_ref().map(|s| {
            if s.common_dir.as_os_str().is_empty() {
                s.workdir.as_path()
            } else {
                s.common_dir.as_path()
            }
        })
    }

    /// Whether the sidebar is as a new tab opens it.
    pub fn sidebar_is_default(&self) -> bool {
        self.collapsed.len() == SIDEBAR_CLOSED_AT_START.len()
            && SIDEBAR_CLOSED_AT_START
                .iter()
                .all(|k| self.collapsed.contains(*k))
    }

    /// Changes a sidebar section only on a toggle, keeping steady frames allocation-free.
    pub fn set_sidebar_open(&mut self, key: &str, open: bool) {
        if open == self.collapsed.contains(key) {
            if open {
                self.collapsed.remove(key);
            } else {
                self.collapsed.insert(key.to_owned());
            }
            self.sidebar_changed = true;
        }
    }

    /// Applies events from the workers. Returns notices for the app to show.
    pub fn poll(&mut self, gitlab_hosts: &[String]) -> Vec<Notice> {
        let mut notices = Vec::new();
        while let Ok(ev) = self.worker.events.try_recv() {
            match ev {
                Event::MergedBranches {
                    serial,
                    bases,
                    result,
                } => {
                    if serial == self.merged_serial {
                        self.merged_applied = serial;
                        self.merged = Some(result.map(|rows| MergedMarks::new(bases, rows)));
                    }
                }
                Event::BaseCompare { serial, result } => {
                    if serial == self.base_serial {
                        self.base_result = Some(result);
                    }
                }
                Event::StaleCandidates { serial, result } => {
                    if serial == self.stale_serial {
                        self.stale_result = Some(result);
                    }
                }
                Event::WorktreeRemoval { serial, result } => {
                    if serial == self.worktree_removal_serial {
                        self.worktree_removal = Some(result);
                    }
                }
                Event::WorktreeForget { serial, result } => {
                    if serial == self.worktree_forget_serial {
                        self.worktree_forget = Some(result);
                    }
                }
                Event::Ignore { serial, result } => {
                    if serial == self.ignore_serial {
                        self.ignore_result = Some(result);
                    }
                }
                Event::Snapshot(s) => {
                    self.ignore_serial = self.ignore_serial.wrapping_add(1);
                    self.ignore_query = None;
                    self.ignore_result = None;
                    let entering_merge = s.state == canopy_core::RepoState::Merge
                        && self
                            .snap
                            .as_ref()
                            .is_none_or(|old| old.state != canopy_core::RepoState::Merge);
                    if s.state != canopy_core::RepoState::Clean {
                        self.amend = false;
                    }
                    if s.state != canopy_core::RepoState::Merge
                        && let Some((summary, body)) = self.merge_prefill.take()
                        && self.commit_summary == summary
                        && self.commit_body == body
                    {
                        self.commit_summary.clear();
                        self.commit_body.clear();
                    }
                    if entering_merge && self.message_is_unwritten() {
                        self.worker.send(Job::MergeMessage);
                    }
                    // The pull settings depend on the current branch: read them again when it
                    // changes (and for the first snapshot).
                    if !self.tools_asked {
                        self.tools_asked = true;
                        self.worker.send(Job::Tools);
                    }
                    let lfs_seen = s
                        .status
                        .staged
                        .iter()
                        .chain(&s.status.unstaged)
                        .any(|f| f.lfs);
                    self.worker.send(Job::Lfs {
                        known: self.lfs.in_use() || lfs_seen,
                    });
                    if !self.template_asked {
                        self.template_asked = true;
                        self.worker.send(Job::CommitTemplate);
                    }
                    if !self.pull_config_asked || self.pull_config_branch != s.head.branch {
                        self.pull_config_asked = true;
                        self.pull_config_branch.clone_from(&s.head.branch);
                        self.worker.send(Job::PullConfig);
                    }
                    self.name.clone_from(&s.name);
                    let head = s.head.oid;
                    let had = self.snap.is_some();
                    self.snap_index = SnapIndex::new(&s);
                    self.snap = Some(s);
                    self.read_merged(false);
                    self.prune_marked();
                    self.prune_parked();
                    self.generation += 1;
                    self.update_forges(gitlab_hosts);
                    self.load_error = None;
                    self.filter_for = String::from("\u{0}");
                    if !had || self.selected_commit.is_none() {
                        // A restored working-copy draft must survive initial history setup.
                        if self.file.is_none()
                            && let Some(h) = head.or_else(|| {
                                self.snap
                                    .as_ref()
                                    .and_then(|s| s.commits.first().map(|c| c.oid))
                            })
                        {
                            self.select_commit(h);
                            self.scroll_to_selected = true;
                        }
                        // Opening shows the checked-out branch's commit: mark the branch too.
                        if !had && self.sidebar_pick.is_none() {
                            self.sidebar_pick = self
                                .snap
                                .as_ref()
                                .and_then(|s| s.head.branch.clone())
                                .map(SidebarItem::Branch);
                        }
                    } else if let Some(sel) = self.selected_commit
                        && self
                            .snap
                            .as_ref()
                            .is_some_and(|s| !s.commits.iter().any(|c| c.oid == sel))
                    {
                        self.selected_commit = None;
                        self.details = None;
                        self.details_error = None;
                    }
                    self.reconcile_file_selection();
                    self.clear_obsolete_refusal();
                }
                Event::Status(st) => {
                    self.ignore_serial = self.ignore_serial.wrapping_add(1);
                    self.ignore_query = None;
                    self.ignore_result = None;
                    if let Some(s) = &mut self.snap {
                        // Copies the snapshot only if a frame still shares it (it does not
                        // between frames).
                        let s = Arc::make_mut(s);
                        s.status = st;
                        s.status_unavailable = None;
                        self.snap_index.update_submodules(s);
                    }
                    self.prune_parked();
                    self.reconcile_file_selection();
                    self.clear_obsolete_refusal();
                }
                Event::DetailsFailed {
                    oid,
                    message,
                    can_download,
                } => {
                    if Some(oid) == self.selected_commit {
                        self.details_error = Some(DetailsError {
                            oid,
                            message,
                            can_download,
                        });
                    }
                    self.clear_obsolete_refusal();
                }
                Event::DownloadsRefused { remote, trust } => {
                    if trust < self.trust_version {
                        self.reread();
                    } else {
                        self.downloads_refused = Some(remote);
                    }
                }
                Event::TrustFolders(folders) => self.trust_folders = folders,
                Event::Trust { trusted, version } => {
                    // An answer older than a change this tab knows of is out of date; the worker
                    // answers the change next.
                    if version >= self.trust_version {
                        let was = self.trust;
                        self.trust = if trusted {
                            RepoTrust::Trusted
                        } else {
                            RepoTrust::Untrusted
                        };
                        // The squash check is skipped until the folder is trusted.
                        if trusted && was != RepoTrust::Trusted {
                            self.read_merged(true);
                        }
                    }
                }
                Event::Signature { oid, status } => {
                    if Some(oid) == self.selected_commit {
                        self.signature = status.map(|s| (oid, s));
                    }
                }
                Event::Details(d) => {
                    if Some(d.oid) == self.selected_commit {
                        let first = d.files.first().cloned();
                        let oid = d.oid;
                        self.details_error = None;
                        // A second reply for the same commit (the line counts arriving after the
                        // file list) replaces the first and leaves the open diff alone.
                        self.details = Some(d);
                        if self.view == View::History
                            && self.file.is_none()
                            && let Some(entry) = first
                        {
                            self.select_file(FileSelection {
                                source: DiffSource::Commit(oid),
                                entry,
                                conflicted: false,
                            });
                        }
                    }
                    self.clear_obsolete_refusal();
                }
                Event::Diff(key, result) => {
                    if self.pending_diff.as_ref() == Some(&key) {
                        self.awaiting_diff = false;
                        match result {
                            Ok(d) => {
                                self.not_downloaded.remove(&(key.source, key.path));
                                // The worker resends the same `Arc` when nothing changed.
                                if !self.diff.as_ref().is_some_and(|old| Arc::ptr_eq(old, &d)) {
                                    self.line_sel = None;
                                    self.highlights = None;
                                }
                                self.diff = Some(d);
                                self.diff_error = None;
                            }
                            Err(e) => {
                                self.diff = None;
                                self.highlights = None;
                                self.line_sel = None;
                                self.diff_error = Some(e);
                            }
                        }
                    }
                    // The refusal that came with this result, if any, explains it only while it
                    // is on screen.
                    self.clear_obsolete_refusal();
                }
                Event::DiffNotDownloaded(key) => {
                    self.not_downloaded.insert((key.source, key.path.clone()));
                    if self.pending_diff.as_ref() == Some(&key) {
                        self.failed_diff = Some(key);
                    }
                }
                Event::Highlight(key, diff, h) => {
                    // Only for the diff on screen: a highlight of an older version of the same
                    // file can arrive after the newer diff.
                    if self.pending_diff.as_ref() == Some(&key)
                        && self.diff.as_ref().is_some_and(|d| Arc::ptr_eq(d, &diff))
                    {
                        self.highlights = Some(h);
                    }
                }
                Event::Conflict { path, kind, file } => {
                    if let Some(r) = self.resolver.as_mut().filter(|r| r.path == path) {
                        r.reload(kind, file);
                    }
                }
                Event::MergeMessage(message) => {
                    if self
                        .snap
                        .as_ref()
                        .is_some_and(|snap| snap.state == canopy_core::RepoState::Merge)
                        && !self.amend
                        && self.message_is_unwritten()
                    {
                        let mut parts = message.splitn(2, '\n');
                        parts
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .clone_into(&mut self.commit_summary);
                        parts
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .clone_into(&mut self.commit_body);
                        self.merge_prefill =
                            Some((self.commit_summary.clone(), self.commit_body.clone()));
                    }
                }
                Event::CommitTemplate(result) => {
                    let previously_loaded = self.template_loaded;
                    self.template_loaded |= result.as_ref().is_ok_and(Option::is_some);
                    match result {
                        Ok(Some(template))
                            if !previously_loaded
                                && !self.amend
                                && self.snap.as_ref().is_some_and(|snap| {
                                    snap.state == canopy_core::RepoState::Clean
                                })
                                && self.commit_summary.trim().is_empty()
                                && self.commit_body.trim().is_empty() =>
                        {
                            let mut parts = template.trim_start().splitn(2, '\n');
                            parts
                                .next()
                                .unwrap_or_default()
                                .trim_end()
                                .clone_into(&mut self.commit_summary);
                            parts
                                .next()
                                .unwrap_or_default()
                                .trim()
                                .clone_into(&mut self.commit_body);
                            self.template_prefill =
                                Some((self.commit_summary.clone(), self.commit_body.clone()));
                        }
                        Ok(_) => {}
                        Err(body) => notices.push(Notice {
                            ok: false,
                            delivery: Delivery::Card,
                            title: "Could not read the commit template".into(),
                            body,
                        }),
                    }
                }
                Event::PullConfig(result) => match result {
                    Ok(config) => self.pull_config = Some(config),
                    Err(e) => {
                        tracing::debug!(error = %e, "pull settings not read");
                        self.pull_config = None;
                    }
                },
                Event::FileHistory { serial, result } => {
                    if serial != self.file_history_serial {
                        continue;
                    }
                    if let Err(error) = &result {
                        notices.push(Notice {
                            ok: false,
                            delivery: Delivery::Card,
                            title: "Could not read the file's history".into(),
                            body: error.clone(),
                        });
                    }
                    self.file_history = Some(result);
                }
                Event::ChangeSearch { serial, result } => {
                    if serial != self.change_search_serial {
                        continue;
                    }
                    self.change_search = Some(result);
                }
                Event::Blame { serial, result } => {
                    if serial != self.blame_serial {
                        continue;
                    }
                    if let Err(error) = &result {
                        notices.push(Notice {
                            ok: false,
                            delivery: Delivery::Card,
                            title: "Could not blame the file".into(),
                            body: error.clone(),
                        });
                    }
                    if let Some(read) = &mut self.blame {
                        read.longest = result.as_ref().map_or(0, |blame| {
                            blame
                                .lines
                                .iter()
                                .map(|l| l.text.chars().count())
                                .max()
                                .unwrap_or(0)
                        });
                        read.result = Some(result);
                    }
                }
                Event::Insights { serial, result } => {
                    if serial != self.insights_serial {
                        continue;
                    }
                    if let Err(error) = &result {
                        notices.push(Notice {
                            ok: false,
                            delivery: Delivery::Card,
                            title: "Could not load insights".into(),
                            body: error.clone(),
                        });
                    }
                    self.insights = Some(result);
                }
                Event::ToolsMissing(tools) => {
                    for tool in tools.into_iter().filter(|t| first_report(&t.program)) {
                        notices.push(Notice {
                            ok: false,
                            delivery: Delivery::Attention,
                            title: format!("{} is not installed", tool.program),
                            body: format!("{}. {}", tool.needed_for, tool.install.join(" ")),
                        });
                    }
                }
                Event::Lfs(info) => self.lfs.set(info),
                Event::LfsLocks { serial, result } => self.lfs.accept_locks(serial, result),
                Event::LfsPatterns { serial, result } => self.lfs.accept_patterns(serial, result),
                Event::PushTarget(result) => match result {
                    Ok(target) => self.force_push_offer = Some(target),
                    Err(e) => notices.push(Notice {
                        ok: false,
                        delivery: Delivery::Card,
                        title: "Could not force push".into(),
                        body: e,
                    }),
                },
                Event::HeadMessage(m) => {
                    if !self.amend
                        || self
                            .snap
                            .as_ref()
                            .is_none_or(|snap| snap.state != canopy_core::RepoState::Clean)
                        || !self.message_is_unwritten()
                    {
                        continue;
                    }
                    let mut parts = m.splitn(2, '\n');
                    parts
                        .next()
                        .unwrap_or("")
                        .trim()
                        .clone_into(&mut self.commit_summary);
                    parts
                        .next()
                        .unwrap_or("")
                        .trim()
                        .clone_into(&mut self.commit_body);
                }
                Event::SquashMessage(message) => {
                    if !self.amend
                        && self
                            .snap
                            .as_ref()
                            .is_some_and(|snap| snap.state == canopy_core::RepoState::Clean)
                        && self.message_is_unwritten()
                    {
                        let mut parts = message.splitn(2, '\n');
                        parts
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .clone_into(&mut self.commit_summary);
                        parts
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .clone_into(&mut self.commit_body);
                    }
                }
                Event::OpStarted { label, kind } => {
                    self.running = Some(label);
                    self.running_kind = Some(kind);
                    self.progress = None;
                }
                // An empty line ends the progress of a download the worker ran outside an
                // operation.
                Event::Progress(line) => self.progress = Some(line).filter(|l| !l.is_empty()),
                Event::OpFinished {
                    label,
                    kind,
                    announce,
                    result,
                } => {
                    self.running = None;
                    self.running_kind = None;
                    self.progress = None;
                    // The end of a fetch nobody asked for gives its slot back, whatever the result.
                    if kind == OpKind::Fetch
                        && !announce
                        && self.quiet_fetches.pop_front() == Some(true)
                    {
                        self.auto_fetch_pending = false;
                    }
                    // Any operation can change the file being resolved (a save, a stash, a
                    // checkout): parse it again so choices and the save guard match the disk.
                    if let Some(r) = &self.resolver {
                        self.worker.send(Job::Conflict(r.path.clone()));
                    }
                    match result {
                        Ok(out) => {
                            let subject = self.commit_summary.trim().to_owned();
                            let branch =
                                self.snap.as_ref().and_then(|snap| snap.head.branch.clone());
                            if kind == OpKind::Commit {
                                self.commit_summary.clear();
                                self.commit_body.clear();
                                self.blur_composer = true;
                                self.amend = false;
                                self.selected_commit = None;
                            }
                            if let OpKind::Squash(target) = &kind {
                                // Nothing is committed yet: prefill the message and show the
                                // staged result for review.
                                self.amend = false;
                                self.worker.send(Job::SquashMessage(target.clone()));
                                self.view = View::Changes;
                            }
                            if kind == OpKind::Worktree {
                                self.worktree_result = Some(Ok(()));
                            }
                            if kind == OpKind::Lfs && self.lfs.dialog_seen {
                                self.read_lfs_lists();
                            }
                            if kind == OpKind::Fetch {
                                self.next_fetch = self
                                    .fetch_every
                                    .and_then(|every| Instant::now().checked_add(every));
                                self.auto_fetch_failure = None;
                            }
                            // The remote moved, or Canopy moved it: its requests may have too.
                            // A successful Git sync does not lift the API's retry hold.
                            let now = Instant::now();
                            let timer = &self.requests.refresh;
                            let held = timer.held_until.is_some_and(|until| now < until);
                            let backing_off = timer.failed_at.is_some_and(|failed| {
                                now.saturating_duration_since(failed)
                                    < backoff_wait(
                                        timer.interval.max(REQUESTS_REFRESH_EVERY),
                                        timer.failures,
                                    )
                            });
                            if kind.syncs_remote() && !held && !backing_off {
                                self.refresh_requests_quietly();
                            }
                            if announce {
                                self.finished = Some(Finished {
                                    label: label.clone(),
                                    kind: kind.clone(),
                                    ok: true,
                                    at: Instant::now(),
                                });
                                notices.push(Notice {
                                    ok: true,
                                    delivery: Delivery::Card,
                                    title: outcome_title(&label, true),
                                    body: success_body(&kind, &out, &subject, branch.as_deref()),
                                });
                            }
                        }
                        Err(e) => match &kind {
                            // Git's own safety check, not a failure: ask before forcing.
                            OpKind::SafeDelete(name) if e.contains("not fully merged") => {
                                self.force_delete_offer = Some(name.clone());
                            }
                            OpKind::Worktree => self.worktree_result = Some(Err(e)),
                            // An automatic fetch that fails the way it failed last time (the
                            // folder is not trusted, the server is down) says so once.
                            OpKind::Fetch
                                if !announce
                                    && self.auto_fetch_failure.as_deref() == Some(e.as_str()) => {}
                            _ => {
                                if kind == OpKind::Fetch && !announce {
                                    self.auto_fetch_failure = Some(e.clone());
                                }
                                self.finished = Some(Finished {
                                    label: label.clone(),
                                    kind: kind.clone(),
                                    ok: false,
                                    at: Instant::now(),
                                });
                                notices.push(Notice {
                                    ok: false,
                                    // Only a fetch nobody asked for is quiet; a failed
                                    // stage or resolve is the user's own action.
                                    delivery: if announce || kind != OpKind::Fetch {
                                        Delivery::Card
                                    } else {
                                        Delivery::Quiet
                                    },
                                    title: outcome_title(&label, false),
                                    body: e,
                                });
                            }
                        },
                    }
                }
                Event::PullDecision(decision) => {
                    self.running = None;
                    self.running_kind = None;
                    self.progress = None;
                    self.pull_decision = Some(*decision);
                }
                Event::Conflicted {
                    label,
                    autostashed,
                    stash,
                } => {
                    self.running = None;
                    self.running_kind = None;
                    self.progress = None;
                    // The resolver lives in Changes: take the user to the conflicted files.
                    self.view = View::Changes;
                    notices.push(Notice {
                        ok: false,
                        delivery: Delivery::Attention,
                        title: format!("{label} stopped on conflicts"),
                        body: stopped_body(autostashed, stash),
                    });
                }
                Event::AutostashConflict => {
                    // The resolver lives in Changes: take the user to the conflicted files.
                    self.view = View::Changes;
                    notices.push(Notice {
                        ok: false,
                        delivery: Delivery::Attention,
                        title: "Your changes conflict with what came in".into(),
                        body: "Canopy set your uncommitted changes aside for this, and putting them back conflicts with the new commits. Resolve the conflicted files in Changes. Your changes also stay in the stash list until you drop that stash.".into(),
                    });
                }
                Event::WatchFailed(e) => notices.push(Notice {
                    ok: false,
                    delivery: Delivery::Quiet,
                    title: "Changes on disk are not picked up".into(),
                    body: format!("{e} {}", crate::watcher::WATCH_FAILED_HINT),
                }),
                Event::Failed(e) => {
                    if self.snap.is_none() {
                        self.load_error = Some(e.clone());
                    }
                    self.clear_obsolete_refusal();
                    notices.push(Notice {
                        ok: false,
                        delivery: Delivery::Card,
                        title: "Git error".into(),
                        body: e,
                    });
                }
            }
        }
        // A line-count refusal can arrive after Details with no terminal error event. Only
        // reconcile once the batch is drained, so a paired refusal/error can explain its view.
        self.clear_obsolete_refusal();
        self.poll_host(&mut notices);
        notices
    }

    /// Remembers whether the host refused the token in `reply` to a read: an accepted reply
    /// clears the note, a refusal sets it, and any other failure says nothing about the token.
    fn note_token<T>(&mut self, reply: &Result<T, String>) {
        match reply {
            Ok(_) => self.requests.token_rejected = false,
            Err(message) if credential_refused(message) => self.requests.token_rejected = true,
            Err(_) => {}
        }
    }

    /// Rebuilds the list from its first page and the later pages, leaving the selection where it is.
    fn show_pages(&mut self, ctx: &egui::Context) {
        let Some(Ok(list)) = &self.requests.list else {
            return;
        };
        let loaded: HashSet<u64> = self.requests.tail.iter().map(|r| r.number).collect();
        let mut all: Vec<RequestSummary> = list
            .iter()
            .filter(|r| !loaded.contains(&r.number))
            .cloned()
            .collect();
        all.extend(self.requests.tail.iter().cloned());
        let all: Arc<[RequestSummary]> = Arc::from(all);
        let standing = |r: &RequestSummary| row_standing(ctx, self, r);
        let order: Vec<u64> = self
            .local_request_view()
            .visible(&all, standing)
            .into_iter()
            .map(|i| all[i].number)
            .collect();
        self.requests.list = Some(Ok(all));
        self.requests.shown = Some(order);
    }

    fn poll_host(&mut self, notices: &mut Vec<Notice>) {
        let Some(host) = &self.host else { return };
        let kind = host.forge.kind;
        let forge = host.forge.clone();
        let ctx = host.context().clone();
        let events: Vec<HostEvent> = host.events.try_iter().collect();
        for ev in events {
            match ev {
                HostEvent::Avatar { oid, email, result } => {
                    self.avatars.finish(&ctx, oid, email, result);
                }
                HostEvent::Cli(state) => {
                    self.requests.cli = Some(state);
                    self.request_view_changed();
                }
                HostEvent::RateLimited { reset } => {
                    let now = chrono::Utc::now().timestamp();
                    self.requests.refresh.held_until =
                        Some(Instant::now() + rate_limit_hold(reset, now));
                }
                HostEvent::More { key, more, cursor } => {
                    if key == self.list_key() && self.requests.pages <= 1 {
                        self.requests.more = more;
                        self.requests.cursor = cursor;
                        if key.query.is_empty() {
                            self.requests.truncated = more.then_some(key.state);
                            // The host holds more than this: the view's filters belong to it.
                            self.request_view_changed();
                        }
                    }
                }
                HostEvent::Page {
                    key,
                    page,
                    result,
                    cursor,
                } => {
                    self.note_token(&result);
                    if key == self.list_key() {
                        match result {
                            Ok((rows, more)) => {
                                let asked =
                                    page == self.requests.pages + 1 && self.requests.loading_more;
                                if asked {
                                    self.requests.pages = page;
                                    self.requests.more = more;
                                    self.requests.cursor = cursor;
                                    // Only the page that was asked for ends the wait: the same
                                    // page again with its checks must not enable a second click
                                    // while the next page is on its way.
                                    self.requests.loading_more = false;
                                }
                                if page <= self.requests.pages {
                                    self.requests.add_page(rows);
                                    self.show_pages(&ctx);
                                }
                            }
                            Err(e) => {
                                self.requests.loading_more = false;
                                self.requests.more_failed = true;
                                notices.push(Notice {
                                    ok: false,
                                    delivery: Delivery::Card,
                                    title: format!(
                                        "Could not load more {}s",
                                        forge.kind.request_noun()
                                    ),
                                    body: e,
                                });
                            }
                        }
                    }
                }
                HostEvent::List(key, result) => {
                    let current = key == self.list_key();
                    self.note_token(&result);
                    let result = result.map(|head| self.requests.with_tail(head));
                    let kept_through_failure = self.requests.quiet_refresh
                        && result.is_err()
                        && matches!(self.requests.list, Some(Ok(_)));
                    if kept_through_failure && current {
                        // A quiet refresh failed (a network blip, a rate limit): the list on
                        // screen is still the best there is. The next sync tries again.
                        tracing::debug!("a quiet request refresh failed; keeping the list");
                        self.requests.quiet_refresh = false;
                        let timer = &mut self.requests.refresh;
                        timer.in_flight = false;
                        timer.failures = timer.failures.saturating_add(1);
                        timer.failed_at = Some(Instant::now());
                    } else if current {
                        self.requests.loading = false;
                        self.requests.loaded_at = Some(Instant::now());
                        self.requests.refresh.in_flight = false;
                        if result.is_ok() {
                            self.requests.more_failed = false;
                            self.requests.refresh.failures = 0;
                            self.requests.refresh.failed_at = None;
                            self.requests.refresh.held_until = None;
                        }
                        // The order the list will show, standing as its rows will show it: the
                        // arrow keys walk it from now, and the first of it opens.
                        let order: Option<Vec<u64>> = result.as_ref().ok().map(|l| {
                            let standing = |r: &RequestSummary| row_standing(&ctx, self, r);
                            let visible = self.local_request_view().visible(l, standing);
                            visible.into_iter().map(|i| l[i].number).collect()
                        });
                        let first = order.as_ref().and_then(|o| o.first().copied());
                        self.generation += 1;
                        let keep = self.requests.selected.is_some_and(|n| {
                            result
                                .as_ref()
                                .is_ok_and(|l| l.iter().any(|r| r.number == n))
                        });
                        self.requests.list = Some(result.map(Arc::from));
                        self.requests.shown = order;
                        if !keep {
                            self.request_comments = self.request_comment_drafts();
                            self.requests.comment.clear();
                            self.requests.selected = None;
                            self.requests.details = None;
                            if let Some(n) = first {
                                self.select_request(n);
                            }
                        }
                    }
                }
                HostEvent::Details(n, generation, result) => {
                    self.note_token(&result);
                    if self.requests.selected == Some(n)
                        && self.requests.details_generation == generation
                    {
                        self.requests.details_loading = false;
                        // A quiet refresh of the open request that fails leaves it shown.
                        let kept = self.requests.details_quiet_refresh
                            && result.is_err()
                            && matches!(self.requests.details, Some(Ok(_)));
                        if !kept {
                            let result = result.map(Arc::new);
                            match &result {
                                Ok(details) => self.requests.cache_details(&forge, n, details),
                                Err(_) => self.requests.forget_details(&forge, n),
                            }
                            self.requests.details = Some(result);
                        }
                    }
                }
                HostEvent::Created(result) => {
                    self.requests.busy = None;
                    match result {
                        Ok(r) => {
                            notices.push(Notice {
                                ok: true,
                                delivery: Delivery::Card,
                                title: format!("{} #{} created", kind.request_abbrev(), r.number),
                                body: r.title.clone(),
                            });
                            self.requests.filter = StateFilter::Open;
                            self.load_requests();
                            self.select_request(r.number);
                        }
                        Err(e) => notices.push(Notice {
                            ok: false,
                            delivery: Delivery::Card,
                            title: format!("Could not create the {}", kind.request_noun()),
                            body: e,
                        }),
                    }
                }
                HostEvent::Done {
                    label,
                    action,
                    number,
                    result,
                } => {
                    if result.is_ok() {
                        self.requests.token_rejected = false;
                    }
                    self.requests.busy = None;
                    let submitted = if action == HostAction::Comment {
                        self.requests.pending_comment.take()
                    } else {
                        None
                    };
                    match result {
                        Ok(()) => {
                            if let Some((posted_number, body)) = submitted
                                && number == Some(posted_number)
                            {
                                if self.requests.selected == number
                                    && self.requests.comment.trim() == body
                                {
                                    self.requests.comment.clear();
                                }
                                self.request_comments.retain(|draft| {
                                    draft.forge != forge
                                        || draft.number != posted_number
                                        || draft.body.trim() != body
                                });
                            }
                            notices.push(Notice {
                                ok: true,
                                delivery: Delivery::Card,
                                title: outcome_title(&label, true),
                                body: String::new(),
                            });
                            if action == HostAction::Merge {
                                // The target branch moved on the remote.
                                self.op(Op::Fetch {
                                    prune: false,
                                    quiet: true,
                                });
                            }
                        }
                        Err(e) => notices.push(Notice {
                            ok: false,
                            delivery: Delivery::Card,
                            title: outcome_title(&label, false),
                            body: e,
                        }),
                    }
                    // Refresh the request currently shown, without stealing a later selection.
                    self.load_requests();
                }
            }
        }
    }

    /// Indices of commits matching the filter (recomputed only when the filter changes).
    pub fn filtered_rows(&mut self) -> Option<&[usize]> {
        let q = self.filter.trim().to_lowercase();
        if q.is_empty() {
            self.filtered = None;
            return None;
        }
        if self.filter_for != q {
            let rows = self.snap.as_ref().map(|s| {
                s.commits
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        c.summary.to_lowercase().contains(&q)
                            || c.author.name.to_lowercase().contains(&q)
                            || c.author.email.to_lowercase().contains(&q)
                            || c.oid.to_string().starts_with(&q)
                    })
                    .map(|(i, _)| i)
                    .collect()
            });
            self.filtered = rows;
            self.filter_for = q;
        }
        self.filtered.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canopy_core::{Choice, ConflictFile};

    #[test]
    fn ignore_replies_cannot_replace_a_newer_path_or_survive_a_status_change() {
        use crate::worker::{IgnoreData, IgnoreQuery};
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.read_ignore(IgnoreQuery::Why("old.log".into()));
        let Job::Ignore { serial: old, .. } = jobs.recv().expect("old read") else {
            panic!("ignore read")
        };
        tab.read_ignore(IgnoreQuery::Why("new.log".into()));
        let Job::Ignore { serial: new, .. } = jobs.recv().expect("new read") else {
            panic!("ignore read")
        };
        events
            .send(Event::Ignore {
                serial: old,
                result: Err("old failure".into()),
            })
            .expect("old response");
        tab.poll(&[]);
        assert!(tab.ignore_result.is_none());
        events
            .send(Event::Ignore {
                serial: new,
                result: Ok(Arc::new(IgnoreData::Why {
                    matched: None,
                    tracked: false,
                })),
            })
            .expect("new response");
        tab.poll(&[]);
        assert!(matches!(tab.ignore_result, Some(Ok(_))));
        events
            .send(Event::Status(canopy_core::WorkingStatus::default()))
            .expect("status");
        events
            .send(Event::Ignore {
                serial: new,
                result: Err("late failure".into()),
            })
            .expect("late response");
        tab.poll(&[]);
        assert!(tab.ignore_result.is_none() && tab.ignore_query.is_none());
    }

    /// #142: the host refusing the token is noted until it accepts one; a network failure or
    /// another refusal says nothing about the token.
    #[test]
    fn a_refused_token_is_remembered_until_the_host_accepts_one() {
        use canopy_hosting::Error;
        let refused = Error::Unauthorized("Bad credentials".into()).to_string();
        assert!(credential_refused(&refused), "{refused}");
        for other in [
            Error::Network("timed out".into()),
            Error::RateLimited { reset: None },
            Error::NotFound("no such project".into()),
            Error::Api {
                status: 500,
                message: "oops".into(),
            },
        ] {
            assert!(!credential_refused(&other.to_string()), "{other}");
        }
        let (worker, _, _) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        assert!(!tab.requests.token_rejected);
        tab.note_token::<()>(&Err(refused));
        assert!(tab.requests.token_rejected);
        tab.note_token::<()>(&Err("Could not reach the server: timed out".into()));
        assert!(
            tab.requests.token_rejected,
            "a network failure changes nothing"
        );
        tab.note_token(&Ok(()));
        assert!(!tab.requests.token_rejected);
    }

    /// An action refused for another reason (a locked conversation answers 403) must not turn a
    /// token that reads fine into a rejected one; one the host accepted still clears the note.
    #[test]
    fn a_refused_action_does_not_flag_the_token() {
        let refused = canopy_hosting::Error::Unauthorized("Conversation is locked".into());
        let done = |result| HostEvent::Done {
            label: "Commenting".into(),
            action: HostAction::Comment,
            number: Some(7),
            result,
        };
        let (mut tab, _, _) = tab_showing_requests();
        let forge = tab.host.as_ref().expect("host").forge.clone();
        let (host, events, _) = HostWorker::detached(forge);
        tab.host = Some(host);
        events.send(done(Err(refused.to_string()))).expect("event");
        tab.poll(&[]);
        assert!(!tab.requests.token_rejected);

        tab.requests.token_rejected = true;
        events.send(done(Ok(()))).expect("event");
        tab.poll(&[]);
        assert!(!tab.requests.token_rejected, "an accepted action clears it");
    }

    /// A notification says what happened in the past tense, or what could not be done.
    #[test]
    fn a_finished_operation_is_titled_by_what_happened() {
        let cases = [
            ("Pulling", "Pulled", "Could not pull"),
            (
                "Creating branch topic",
                "Created branch topic",
                "Could not create branch topic",
            ),
            ("Force pushing", "Force pushed", "Could not force push"),
            (
                "Checking out main",
                "Checked out main",
                "Could not check out main",
            ),
            (
                "Commenting on PR #12",
                "Commented on PR #12",
                "Could not comment on PR #12",
            ),
            ("Pushing", "Pushed", "Could not push"),
            ("Hard reset", "Hard reset — done", "Hard reset failed"),
        ];
        for (label, done, failed) in cases {
            assert_eq!(outcome_title(label, true), done, "{label}");
            assert_eq!(outcome_title(label, false), failed, "{label}");
        }
        // A verb counts only as a whole word.
        assert_eq!(outcome_title("Pushingly", true), "Pushingly — done");
    }

    #[test]
    fn a_success_is_told_in_words_not_in_git_s_transfer_output() {
        let fetch = "remote: Total 3 (delta 2), reused 0 (delta 0), pack-reused 0 (from 0)\n\
            From /srv/origin\n\
            \x20  a1c2bac..6620397  main       -> origin/main\n\
            \x20* [new branch]      feat       -> origin/feat\n\
            \x20* [new branch]      two        -> origin/two\n\
            \x20* [new branch]      three      -> origin/three\n\
            \x20* [new branch]      four       -> origin/four\n\
            \x20- [deleted]         (none)     -> origin/old\n\
            \x20* [new tag]         v1         -> v1";
        let push = "To /srv/origin\n   a1c2bac..6620397  main -> main\n + 1234567...89abcde feat -> feat (forced update)";
        let cases: [(OpKind, &str, &str, &str); 10] = [
            (
                OpKind::Commit,
                "a1b2c3d",
                "Fix the typo",
                "Fix the typo (a1b2c3d)",
            ),
            (
                OpKind::Fetch,
                fetch,
                "",
                "New branches: origin/feat, origin/two, origin/three and 1 more. Updated: origin/main. Removed: origin/old. New tag: v1.",
            ),
            (OpKind::Fetch, "", "", "Nothing new on any remote."),
            (
                OpKind::Pull,
                "From /srv/origin\n   a..b  main -> origin/main\nSuccessfully rebased and updated refs/heads/main.",
                "",
                "Rebased main onto its upstream.",
            ),
            (
                OpKind::Pull,
                "remote: Total 3",
                "",
                "main is up to date with its upstream.",
            ),
            (
                OpKind::Pull,
                "Merge made by the 'ort' strategy.\n a.txt | 1 +\n 1 file changed, 1 insertion(+)",
                "",
                "main now has everything from its upstream.",
            ),
            (
                OpKind::Push,
                push,
                "",
                "Updated: main. Force-updated: feat.",
            ),
            (
                OpKind::Push,
                "To /srv/origin\nEverything up-to-date",
                "",
                "Everything was already up to date on the remote.",
            ),
            (
                OpKind::Other,
                "To /srv/origin\n - [deleted]         gone",
                "",
                "Removed: gone.",
            ),
            (
                OpKind::Other,
                "Merge made by the 'ort' strategy.",
                "",
                "Merge made by the 'ort' strategy.",
            ),
        ];
        for (kind, out, subject, told) in cases {
            assert_eq!(
                success_body(&kind, out, subject, Some("main")),
                told,
                "{kind:?}: {out}"
            );
        }
    }

    /// The unstaged change to `a.rs`, open in the diff pane.
    fn open_unstaged_file() -> FileSelection {
        FileSelection {
            source: DiffSource::Unstaged,
            entry: FileEntry {
                path: "a.rs".into(),
                old_path: None,
                kind: canopy_core::ChangeKind::Modified,
                additions: 0,
                deletions: 0,
                lfs: false,
                old_mode: None,
                new_mode: None,
            },
            conflicted: false,
        }
    }

    fn unstaged_entry(path: &str) -> FileEntry {
        let mut entry = open_unstaged_file().entry;
        entry.path = path.into();
        entry
    }

    #[test]
    fn stage_targets_are_the_open_unstaged_file_and_the_marked_ones_in_list_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, _events, _jobs) = Worker::detached();
        tab.worker = worker;
        let mut snap = RepoSnapshot::default();
        snap.status.unstaged = ["a.rs", "b.rs", "c.rs", "d.rs"]
            .map(unstaged_entry)
            .to_vec();
        snap.status.staged = vec![unstaged_entry("e.rs")];
        tab.snap = Some(Arc::new(snap));
        assert!(tab.stage_targets().is_empty(), "nothing is open");

        tab.select_file(open_unstaged_file());
        tab.marked
            .extend(["d.rs".to_owned(), "b.rs".to_owned(), "gone.rs".to_owned()]);
        assert_eq!(tab.stage_targets(), ["a.rs", "b.rs", "d.rs"]);

        // A staged file is open: only the marked files are staged.
        tab.file = Some(FileSelection {
            source: DiffSource::Staged,
            entry: unstaged_entry("e.rs"),
            conflicted: false,
        });
        assert_eq!(tab.stage_targets(), ["b.rs", "d.rs"]);
        Ok(())
    }

    #[test]
    fn marks_follow_the_selection_and_drop_files_that_left_the_unstaged_list()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, _events, _jobs) = Worker::detached();
        tab.worker = worker;
        let mut snap = RepoSnapshot::default();
        snap.status.unstaged = ["a.rs", "b.rs"].map(unstaged_entry).to_vec();
        tab.snap = Some(Arc::new(snap));
        tab.select_file(open_unstaged_file());
        tab.marked.insert("b.rs".into());

        // The same file arriving again with new counts keeps the marks...
        let mut again = open_unstaged_file();
        again.entry.additions = 3;
        tab.select_file(again);
        assert_eq!(tab.marked.len(), 1);
        // ...another file replaces the selection.
        tab.select_file(FileSelection {
            entry: unstaged_entry("b.rs"),
            ..open_unstaged_file()
        });
        assert!(tab.marked.is_empty());

        tab.marked.insert("a.rs".into());
        tab.deselect_file();
        assert!(tab.marked.is_empty());
        Ok(())
    }

    #[test]
    fn refresh_reloads_a_historical_gitlink_without_losing_its_selection() {
        for full in [false, true] {
            let (worker, events, jobs) = Worker::detached();
            let mut tab =
                RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
            tab.snap = Some(Arc::new(RepoSnapshot::default()));
            tab.pull_config_asked = true;
            tab.tools_asked = true;
            tab.template_asked = true;
            let mut selection = open_unstaged_file();
            selection.source = DiffSource::Commit(Oid::from_bytes(&[1; 20]).expect("oid"));
            selection.entry.path = "deps/lib".into();
            selection.entry.old_mode = Some(0o160_000);
            selection.entry.new_mode = Some(0o160_000);
            tab.select_file(selection.clone());
            let Ok(Job::Diff(key)) = jobs.try_recv() else {
                panic!("initial diff request");
            };
            let old = Arc::new(FileDiff::default());
            tab.diff = Some(Arc::clone(&old));
            events
                .send(if full {
                    Event::Snapshot(Arc::new(RepoSnapshot::default()))
                } else {
                    Event::Status(canopy_core::WorkingStatus::default())
                })
                .expect("refresh");
            tab.poll(&[]);
            assert_eq!(tab.file.as_ref(), Some(&selection));
            assert!(Arc::ptr_eq(tab.diff.as_ref().expect("retained diff"), &old));
            if full {
                // A snapshot also asks for the repository's LFS files (`Job::Lfs`).
                assert!(matches!(jobs.try_recv(), Ok(Job::Lfs { known: false })));
            }
            assert!(
                matches!(jobs.try_recv(), Ok(Job::Diff(refreshed)) if refreshed == key),
                "historical gitlink availability can change on refresh, full={full}"
            );
            let current = Arc::new(FileDiff {
                submodule: Some(canopy_core::SubmoduleChange {
                    old: Some(Oid::from_bytes(&[1; 20]).expect("oid")),
                    new: Some(Oid::from_bytes(&[2; 20]).expect("oid")),
                    old_present: true,
                    new_present: true,
                    log: canopy_core::SubmoduleLog::default(),
                }),
                ..FileDiff::default()
            });
            events
                .send(Event::Diff(key, Ok(Arc::clone(&current))))
                .expect("loaded diff");
            tab.poll(&[]);
            assert!(Arc::ptr_eq(
                tab.diff.as_ref().expect("current diff"),
                &current
            ));
        }
    }

    const TWO: &str = "a\n<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\nb\n<<<<<<< ours\n1\n=======\n2\n>>>>>>> theirs\n";

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn squash_message_prefills_a_normal_commit() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let repo = git2::Repository::init(dir.path())?;
        std::fs::write(repo.path().join("SQUASH_MSG"), "Squashed commit body\n")?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.worker.send(Job::SquashMessage("topic".into()));
        let deadline = Instant::now() + Duration::from_secs(3);
        while tab.commit_summary.is_empty() && Instant::now() < deadline {
            tab.poll(&[]);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!tab.amend, "a squash is a new commit, not an amend");
        assert_eq!(tab.commit_summary, "Merge topic (squashed)");
        assert_eq!(tab.commit_body, "Squashed commit body");
        Ok(())
    }

    #[test]
    fn commit_submission_requires_staged_changes() {
        let (worker, _, _) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.commit_summary = "Draft".into();
        assert!(!tab.can_commit());
    }

    #[test]
    fn completing_a_squash_turns_off_amend() {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.amend = true;
        events
            .send(Event::OpFinished {
                label: "Squashing topic".into(),
                kind: OpKind::Squash("topic".into()),
                announce: false,
                result: Ok(String::new()),
            })
            .expect("squash completion");
        tab.poll(&[]);
        assert!(!tab.amend, "squashed changes must make a new commit");
        assert_eq!(tab.view, View::Changes);
        assert!(matches!(jobs.try_recv(), Ok(Job::SquashMessage(target)) if target == "topic"));
    }

    #[test]
    fn squash_prefill_preserves_drafts_and_rejects_ineligible_states() {
        for (state, amend, summary, body) in [
            (canopy_core::RepoState::Clean, false, "Typed summary", ""),
            (canopy_core::RepoState::Clean, false, "", "Typed body"),
            (canopy_core::RepoState::Clean, true, "", ""),
            (canopy_core::RepoState::Merge, false, "", ""),
            (canopy_core::RepoState::Rebase, false, "", ""),
        ] {
            let (worker, events, _jobs) = Worker::detached();
            let mut tab =
                RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
            tab.snap = Some(Arc::new(RepoSnapshot {
                state,
                ..Default::default()
            }));
            tab.amend = amend;
            tab.commit_summary = summary.into();
            tab.commit_body = body.into();
            events
                .send(Event::SquashMessage("Squash summary\n\nSquash body".into()))
                .expect("squash message");
            tab.poll(&[]);
            assert_eq!(tab.commit_summary, summary, "{state:?}, amend {amend}");
            assert_eq!(tab.commit_body, body, "{state:?}, amend {amend}");
        }
    }

    #[test]
    fn a_late_amend_message_cannot_replace_a_squash_message() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.amend = true;
        events
            .send(Event::OpFinished {
                label: "Squashing topic".into(),
                kind: OpKind::Squash("topic".into()),
                announce: false,
                result: Ok(String::new()),
            })
            .expect("squash completion");
        events
            .send(Event::HeadMessage("Old head".into()))
            .expect("late amend message");
        events
            .send(Event::SquashMessage(
                "Merge topic (squashed)\n\nBody".into(),
            ))
            .expect("squash message");
        tab.poll(&[]);
        assert!(!tab.amend);
        assert_eq!(tab.commit_summary, "Merge topic (squashed)");
        assert_eq!(tab.commit_body, "Body");
    }

    #[test]
    fn commit_submission_blocks_unresolved_conflicts() {
        let (worker, _, _) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.snap = Some(Arc::new(RepoSnapshot {
            state: canopy_core::RepoState::Merge,
            status: canopy_core::WorkingStatus {
                conflicted: vec![open_unstaged_file().entry],
                ..Default::default()
            },
            ..Default::default()
        }));
        tab.commit_summary = "Draft".into();
        assert!(!tab.can_commit());
    }

    #[test]
    fn commit_submission_blocks_running_operations() {
        let (worker, _, _) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.snap = Some(Arc::new(RepoSnapshot {
            state: canopy_core::RepoState::Merge,
            ..Default::default()
        }));
        tab.commit_summary = "Draft".into();
        tab.running = Some("Busy".into());
        assert!(!tab.can_commit());
    }

    /// Opening a repository marks the checked-out branch in the sidebar, as History shows its
    /// commit; a later snapshot leaves the user's own pick alone.
    #[test]
    fn opening_picks_the_checked_out_branch() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        let head = Oid::from_bytes(&[1; 20]).expect("oid");
        let snap = || {
            Arc::new(RepoSnapshot {
                head: canopy_core::HeadInfo {
                    branch: Some("main".into()),
                    oid: Some(head),
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        events.send(Event::Snapshot(snap())).expect("snapshot");
        tab.poll(&[]);
        assert_eq!(tab.sidebar_pick, Some(SidebarItem::Branch("main".into())));
        assert!(tab.sidebar_shows(|p| *p == SidebarItem::Branch("main".into()), Some(head)));

        tab.sidebar_pick = Some(SidebarItem::Tag("v1".into()));
        events.send(Event::Snapshot(snap())).expect("snapshot");
        tab.poll(&[]);
        assert_eq!(tab.sidebar_pick, Some(SidebarItem::Tag("v1".into())));
    }

    #[test]
    fn a_rebase_snapshot_clears_amend() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.amend = true;
        events
            .send(Event::Snapshot(Arc::new(RepoSnapshot {
                state: canopy_core::RepoState::Rebase,
                ..Default::default()
            })))
            .expect("snapshot");
        tab.poll(&[]);
        assert!(!tab.amend);
    }

    #[test]
    fn an_amend_prefill_after_a_merge_snapshot_preserves_the_draft() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.amend = true;
        tab.commit_summary = "Merge draft".into();
        events
            .send(Event::Snapshot(Arc::new(RepoSnapshot {
                state: canopy_core::RepoState::Merge,
                ..Default::default()
            })))
            .expect("snapshot");
        events
            .send(Event::HeadMessage("Old head message".into()))
            .expect("message");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "Merge draft");
    }

    #[test]
    fn leaving_a_merge_drops_its_prefilled_message_but_keeps_an_edited_one() {
        let snapshot = |state| {
            Event::Snapshot(Arc::new(RepoSnapshot {
                state,
                ..Default::default()
            }))
        };
        for edited in [false, true] {
            let (worker, events, _jobs) = Worker::detached();
            let mut tab =
                RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
            events
                .send(snapshot(canopy_core::RepoState::Merge))
                .expect("snapshot");
            events
                .send(Event::MergeMessage("Merge branch 'conf'\n\nbody".into()))
                .expect("message");
            tab.poll(&[]);
            assert_eq!(tab.commit_summary, "Merge branch 'conf'");
            if edited {
                tab.commit_summary.push_str(" (mine)");
            }
            events
                .send(snapshot(canopy_core::RepoState::Clean))
                .expect("snapshot");
            tab.poll(&[]);
            if edited {
                assert_eq!(tab.commit_summary, "Merge branch 'conf' (mine)");
            } else {
                assert!(tab.commit_summary.is_empty(), "{:?}", tab.commit_summary);
                assert!(tab.commit_body.is_empty(), "{:?}", tab.commit_body);
            }
        }
    }

    /// #92: the commit template fills an empty message field once; a draft, an amend and a merge
    /// keep theirs, and a template that cannot be read is reported.
    #[test]
    fn the_commit_template_fills_only_an_empty_message_field() {
        let clean = || {
            Event::Snapshot(Arc::new(RepoSnapshot {
                state: canopy_core::RepoState::Clean,
                ..Default::default()
            }))
        };
        let template = || Event::CommitTemplate(Ok(Some("\nfeat: \n\nWhy:\nHow:\n".into())));
        let fresh = || {
            let (worker, events, jobs) = Worker::detached();
            let mut tab =
                RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
            events.send(clean()).expect("snapshot");
            tab.poll(&[]);
            (tab, events, jobs)
        };

        let (mut tab, events, jobs) = fresh();
        let asked = std::iter::from_fn(|| jobs.try_recv().ok())
            .filter(|j| matches!(j, Job::CommitTemplate))
            .count();
        assert_eq!(asked, 1, "asked once, at the first snapshot");
        events.send(clean()).expect("snapshot");
        tab.poll(&[]);
        assert!(
            std::iter::from_fn(|| jobs.try_recv().ok()).all(|j| !matches!(j, Job::CommitTemplate)),
            "not asked again"
        );
        events.send(template()).expect("template");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "feat:");
        assert_eq!(tab.commit_body, "Why:\nHow:");

        let (mut tab, events, _jobs) = fresh();
        tab.commit_body = "my draft".into();
        events.send(template()).expect("template");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "");
        assert_eq!(tab.commit_body, "my draft");

        let (mut tab, events, _jobs) = fresh();
        tab.amend = true;
        events.send(template()).expect("template");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "", "never on an amend");

        let (mut tab, events, _jobs) = fresh();
        events.send(Event::CommitTemplate(Ok(None))).expect("none");
        events
            .send(Event::CommitTemplate(Err("fatal: bad config".into())))
            .expect("error");
        let notices = tab.poll(&[]);
        assert_eq!(tab.commit_summary, "");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "Could not read the commit template");
        assert!(notices[0].body.contains("bad config"));
    }

    /// A template sitting untouched in the boxes must not stop git's own messages from filling
    /// them: HEAD's for an amend, `MERGE_MSG` for a merge, `SQUASH_MSG` for a squash.
    #[test]
    fn an_untouched_template_yields_to_the_amend_merge_and_squash_messages() {
        let state = |state| {
            Event::Snapshot(Arc::new(RepoSnapshot {
                state,
                ..Default::default()
            }))
        };
        let template = || Event::CommitTemplate(Ok(Some("\nfeat: \n\nWhy:\n".into())));
        let fresh = |first| {
            let (worker, events, _jobs) = Worker::detached();
            let mut tab =
                RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
            events.send(state(first)).expect("snapshot");
            tab.poll(&[]);
            (tab, events)
        };

        let (mut tab, events) = fresh(canopy_core::RepoState::Clean);
        events.send(template()).expect("template");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "feat:");
        assert!(tab.message_is_unwritten());
        tab.amend = true;
        events
            .send(Event::HeadMessage("fix: the last one\n\nbecause".into()))
            .expect("head message");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "fix: the last one");
        assert_eq!(tab.commit_body, "because");

        let (mut tab, events) = fresh(canopy_core::RepoState::Clean);
        events.send(template()).expect("template");
        tab.poll(&[]);
        events
            .send(Event::SquashMessage(
                "Squashed commit of the following:".into(),
            ))
            .expect("squash message");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "Squashed commit of the following:");

        let (mut tab, events) = fresh(canopy_core::RepoState::Clean);
        events.send(template()).expect("template");
        tab.poll(&[]);
        events
            .send(state(canopy_core::RepoState::Merge))
            .expect("merge snapshot");
        tab.poll(&[]);
        events
            .send(Event::MergeMessage("Merge branch 'topic'".into()))
            .expect("merge message");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "Merge branch 'topic'");

        // Text the user changed is theirs: none of the three replaces it.
        let (mut tab, events) = fresh(canopy_core::RepoState::Clean);
        events.send(template()).expect("template");
        tab.poll(&[]);
        tab.commit_summary.push_str(" mine");
        assert!(!tab.message_is_unwritten());
        tab.amend = true;
        events
            .send(Event::HeadMessage("fix: the last one".into()))
            .expect("head message");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "feat: mine");
    }

    #[test]
    fn trusting_a_folder_retries_a_missing_template_without_reopening_the_tab() {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        events
            .send(Event::Snapshot(Arc::new(RepoSnapshot::default())))
            .expect("snapshot");
        events
            .send(Event::CommitTemplate(Ok(None)))
            .expect("untrusted read");
        tab.poll(&[]);
        let asked = std::iter::from_fn(|| jobs.try_recv().ok())
            .filter(|j| matches!(j, Job::CommitTemplate))
            .count();
        assert_eq!(asked, 1);

        tab.trust_changed(1);
        let retried = std::iter::from_fn(|| jobs.try_recv().ok())
            .filter(|j| matches!(j, Job::CommitTemplate))
            .count();
        assert_eq!(retried, 1, "retry after the trust list changes");
        events
            .send(Event::CommitTemplate(Ok(Some(
                "feat: trusted\n\nWhy".into(),
            ))))
            .expect("trusted template");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "feat: trusted");
        assert_eq!(tab.commit_body, "Why");

        tab.commit_summary.clear();
        tab.commit_body.clear();
        tab.trust_changed(2);
        assert!(
            std::iter::from_fn(|| jobs.try_recv().ok()).all(|j| !matches!(j, Job::CommitTemplate)),
            "a loaded template is never inserted again after the user clears it"
        );
    }

    #[test]
    fn a_pending_template_retry_cannot_refill_a_cleared_message() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        events
            .send(Event::Snapshot(Arc::new(RepoSnapshot::default())))
            .expect("snapshot");
        tab.poll(&[]);
        tab.trust_changed(1);
        events
            .send(Event::CommitTemplate(Ok(Some("feat: template".into()))))
            .expect("first reply");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "feat: template");
        tab.commit_summary.clear();
        events
            .send(Event::CommitTemplate(Ok(Some("feat: template".into()))))
            .expect("pending retry reply");
        tab.poll(&[]);
        assert!(
            tab.commit_summary.is_empty(),
            "keep the user's cleared message"
        );
        assert_eq!(tab.commit_body, "");
    }

    #[test]
    fn an_amend_prefill_preserves_a_draft_typed_while_loading() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.amend = true;
        tab.commit_summary = "Typed draft".into();
        events
            .send(Event::HeadMessage("Old head message".into()))
            .expect("message");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "Typed draft");
    }

    fn conflicted_entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            old_path: None,
            kind: canopy_core::ChangeKind::Modified,
            additions: 0,
            deletions: 0,
            lfs: false,
            old_mode: None,
            new_mode: None,
        }
    }

    fn conflicted(path: &str) -> FileSelection {
        FileSelection {
            source: DiffSource::Unstaged,
            entry: conflicted_entry(path),
            conflicted: true,
        }
    }

    /// #132: choices and an unfinished edit survive picking another conflicted file and coming
    /// back; a file with no authored work leaves nothing behind.
    #[test]
    fn selecting_another_conflicted_file_keeps_the_resolver_draft() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        let text = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n";
        tab.select_file(conflicted("a.rs"));
        events
            .send(Event::Conflict {
                path: "a.rs".into(),
                kind: None,
                file: Ok(Arc::new(ConflictFile::parse(text))),
            })
            .expect("conflict");
        tab.poll(&[]);
        let r = tab.resolver.as_mut().expect("resolver for a");
        r.choose(Choice::Theirs);
        r.editing = Some((0, "unfinished".into()));

        tab.select_file(conflicted("b.rs"));
        assert_eq!(tab.resolver.as_ref().map(|r| r.path.as_str()), Some("b.rs"));
        tab.select_file(conflicted("a.rs"));
        let r = tab.resolver.as_ref().expect("resolver for a again");
        assert_eq!(r.path, "a.rs");
        assert_eq!(r.choices, vec![Some(Choice::Theirs)]);
        assert_eq!(r.editing, Some((0, "unfinished".into())));
        assert!(tab.parked_resolvers.is_empty(), "b had no draft to park");
    }

    /// A parked draft goes when its file stops being conflicted (snapshot or status), but not
    /// when the status could not be read.
    #[test]
    fn parked_resolvers_follow_the_conflicted_list() {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        let draft = || {
            let mut r = Resolver::new("a.rs".into());
            r.choices = vec![Some(Choice::Ours)];
            r
        };
        let snap = |conflicted: &[&str], unavailable: bool| {
            let mut s = RepoSnapshot::default();
            s.status.conflicted = conflicted.iter().map(|p| conflicted_entry(p)).collect();
            if unavailable {
                s.status_unavailable = Some(canopy_core::StatusUnavailable {
                    message: "unreadable".into(),
                    can_download: false,
                });
            }
            Arc::new(s)
        };
        tab.parked_resolvers = vec![draft()];
        events
            .send(Event::Snapshot(snap(&[], true)))
            .expect("snapshot");
        tab.poll(&[]);
        assert_eq!(tab.parked_resolvers.len(), 1, "unreadable status keeps it");
        events
            .send(Event::Snapshot(snap(&["a.rs"], false)))
            .expect("snapshot");
        tab.poll(&[]);
        assert_eq!(tab.parked_resolvers.len(), 1, "still conflicted");
        events
            .send(Event::Status(canopy_core::WorkingStatus::default()))
            .expect("status");
        tab.poll(&[]);
        assert!(tab.parked_resolvers.is_empty(), "resolved elsewhere");
    }

    #[test]
    fn reloading_discards_an_edit_for_a_changed_conflict() {
        let mut resolver = Resolver::new("f".into());
        resolver.reload(
            None,
            Ok(Arc::new(ConflictFile::parse(
                "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n",
            ))),
        );
        resolver.editing = Some((0, "authored for a and b".into()));
        resolver.reload(
            None,
            Ok(Arc::new(ConflictFile::parse(
                "<<<<<<< ours\nchanged a\n=======\nb\n>>>>>>> theirs\n",
            ))),
        );
        assert_eq!(resolver.editing, None, "the edited region changed on disk");
    }

    #[test]
    fn reloading_keeps_choices_only_for_unchanged_conflicts() {
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(ConflictFile::parse(TWO))));
        assert_eq!(r.choices, vec![None, None]);
        r.choose(Choice::Ours);
        r.choose(Choice::Theirs);
        assert_eq!(r.resolved(), 2);
        // Same text: both choices stay.
        r.reload(None, Ok(Arc::new(ConflictFile::parse(TWO))));
        assert_eq!(r.resolved(), 2);
        // The second conflict edited in an editor: only the first choice stays.
        r.reload(
            None,
            Ok(Arc::new(ConflictFile::parse(
                &TWO.replace("\n2\n", "\n3\n"),
            ))),
        );
        assert_eq!(r.choices, vec![Some(Choice::Ours), None]);
        // One conflict resolved by hand: the remaining one is a different region now.
        let one = "a\nx\nb\n<<<<<<< ours\n1\n=======\n3\n>>>>>>> theirs\n";
        r.reload(None, Ok(Arc::new(ConflictFile::parse(one))));
        assert_eq!(r.choices, vec![None]);
        assert_eq!(r.focus, 0);
    }

    #[test]
    fn focus_moves_to_unresolved_conflicts_and_wraps() {
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(ConflictFile::parse(TWO))));
        r.choose(Choice::Ours);
        assert_eq!(r.focus, 1);
        r.move_focus(1);
        assert_eq!(r.focus, 1, "the only unresolved conflict");
        r.clear_focused();
        r.focus = 0;
        r.clear_focused();
        assert_eq!(r.resolved(), 0);
        r.move_focus(-1);
        assert_eq!(r.focus, 1, "wraps backwards");
    }

    #[test]
    fn snapshot_index_groups_remote_branches_and_measures_the_graph() {
        let remote = |name: &str| canopy_core::RemoteInfo {
            name: name.into(),
            url: None,
            default_branch: None,
        };
        let branch = |remote: &str, branch: &str| canopy_core::RemoteBranchInfo {
            remote: remote.into(),
            branch: branch.into(),
            target: None,
        };
        let s = RepoSnapshot {
            remotes: vec![remote("origin"), remote("fork"), remote("empty")],
            remote_branches: vec![
                branch("fork", "a"),
                branch("origin", "main"),
                branch("gone", "x"),
                branch("origin", "dev"),
            ],
            graph: vec![
                canopy_core::GraphRow {
                    width: 2,
                    ..Default::default()
                },
                canopy_core::GraphRow {
                    width: 5,
                    ..Default::default()
                },
            ],
            ..RepoSnapshot::default()
        };
        let i = SnapIndex::new(&s);
        assert_eq!(i.remote_branches, vec![vec![1, 3], vec![0], vec![]]);
        assert_eq!(i.graph_lanes, 5);
        assert_eq!(SnapIndex::new(&RepoSnapshot::default()).graph_lanes, 1);
    }

    #[test]
    fn a_highlight_for_an_older_diff_is_dropped() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let key = DiffKey {
            source: DiffSource::Unstaged,
            path: "a.rs".into(),
            old_path: None,
            conflicted: false,
            context: 3,
            whitespace: canopy_core::Whitespace::default(),
            highlight: true,
        };
        tab.pending_diff = Some(key.clone());
        let diff = |additions| {
            Arc::new(FileDiff {
                path: "a.rs".into(),
                additions,
                ..Default::default()
            })
        };
        let (old, new) = (diff(1), diff(2));
        let highlights = Arc::new(Highlights {
            lines: Vec::new(),
            language: "Rust".into(),
        });
        events.send(Event::Diff(key.clone(), Ok(Arc::clone(&old))))?;
        events.send(Event::Diff(key.clone(), Ok(Arc::clone(&new))))?;
        events.send(Event::Highlight(key.clone(), old, Arc::clone(&highlights)))?;
        tab.poll(&[]);
        assert!(tab.highlights.is_none(), "a highlight of the older diff");
        events.send(Event::Highlight(key.clone(), Arc::clone(&new), highlights))?;
        tab.poll(&[]);
        assert!(tab.highlights.is_some());
        // The worker resends the same `Arc` when nothing changed: the highlight stays.
        events.send(Event::Diff(key, Ok(new)))?;
        tab.poll(&[]);
        assert!(tab.highlights.is_some());
        Ok(())
    }

    #[test]
    fn a_refusal_that_trust_overtook_rereads_instead_of_showing_the_chip()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, jobs) = Worker::detached();
        tab.worker = worker;
        // Trust changes, to version 2, while no refusal is on record...
        tab.trust_changed(2);
        while jobs.try_recv().is_ok() {}
        // ...and a refusal the worker had queued under version 1 arrives afterwards.
        events.send(Event::DownloadsRefused {
            remote: "origin".into(),
            trust: 1,
        })?;
        tab.poll(&[]);
        assert!(tab.downloads_refused.is_none(), "a stale refusal");
        assert!(jobs.try_recv().is_ok(), "the tab asked the worker again");
        // The refusal of that second read comes from version 2: it is genuine.
        events.send(Event::DownloadsRefused {
            remote: "origin".into(),
            trust: 2,
        })?;
        tab.poll(&[]);
        assert_eq!(tab.downloads_refused.as_deref(), Some("origin"));
        Ok(())
    }

    #[test]
    fn the_first_refusal_after_a_trust_change_is_genuine_when_the_new_list_made_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, jobs) = Worker::detached();
        tab.worker = worker;
        tab.trust_changed(2);
        while jobs.try_recv().is_ok() {}
        // A read started after the change, so the new list refused it: the chip shows at once.
        events.send(Event::DownloadsRefused {
            remote: "origin".into(),
            trust: 2,
        })?;
        tab.poll(&[]);
        assert_eq!(tab.downloads_refused.as_deref(), Some("origin"));
        assert!(jobs.try_recv().is_err(), "nothing to read again");
        Ok(())
    }

    #[test]
    fn a_refusal_does_not_outlive_the_file_it_was_shown_on()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.file = Some(open_unstaged_file());
        tab.diff_error = Some(crate::worker::NOT_TRUSTED.into());
        tab.downloads_refused = Some("origin".into());
        // The refreshed status no longer lists the file: nothing is left to show the error on.
        events.send(Event::Status(canopy_core::WorkingStatus::default()))?;
        tab.poll(&[]);
        assert!(tab.file.is_none());
        assert!(tab.diff_error.is_none());
        assert!(tab.downloads_refused.is_none(), "nothing left to explain");
        Ok(())
    }

    #[test]
    fn closing_the_file_clears_the_refusal_only_its_diff_needed()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, _events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.file = Some(open_unstaged_file());
        tab.diff_error = Some(crate::worker::NOT_TRUSTED.into());
        tab.downloads_refused = Some("origin".into());
        // Another view, or another commit, takes the diff off the screen.
        tab.deselect_file();
        assert!(tab.file.is_none() && tab.diff_error.is_none());
        assert!(tab.downloads_refused.is_none(), "nothing left to explain");
        Ok(())
    }

    #[test]
    fn a_refusal_that_arrives_after_its_file_was_closed_does_not_keep_the_chip()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        let sel = open_unstaged_file();
        let key = tab.diff_key(&sel);
        tab.select_file(sel);
        // The user leaves the file before the worker answers...
        tab.deselect_file();
        // ...which then says the diff needed a download it did not run.
        events.send(Event::DownloadsRefused {
            remote: "origin".into(),
            trust: 0,
        })?;
        events.send(Event::Diff(key, Err(crate::worker::NOT_TRUSTED.into())))?;
        tab.poll(&[]);
        assert!(tab.diff_error.is_none(), "nobody waits for that diff");
        assert!(tab.downloads_refused.is_none(), "nothing left to explain");
        Ok(())
    }

    #[test]
    fn the_chip_explains_only_errors_that_are_refusals() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        let (a, b) = (Oid::from_bytes(&[1; 20])?, Oid::from_bytes(&[2; 20])?);
        tab.selected_commit = Some(a);
        tab.downloads_refused = Some("origin".into());
        // Another commit is selected: until its details are in, they may be refused too.
        tab.select_commit(b);
        assert!(
            tab.downloads_refused.is_some(),
            "its details are on their way"
        );
        // They fail for a reason that has nothing to do with trust.
        events.send(Event::DetailsFailed {
            oid: b,
            message: "object not found".into(),
            can_download: false,
        })?;
        tab.poll(&[]);
        assert!(tab.details_error.is_some());
        assert!(tab.downloads_refused.is_none(), "not the chip's to explain");

        // The same failure as a refusal keeps it.
        tab.downloads_refused = Some("origin".into());
        events.send(Event::DetailsFailed {
            oid: b,
            message: format!(
                "This content hasn't been downloaded yet. {}",
                crate::worker::NOT_TRUSTED
            ),
            can_download: false,
        })?;
        tab.poll(&[]);
        assert!(tab.downloads_refused.is_some(), "this one is a refusal");
        Ok(())
    }

    #[test]
    fn leaving_history_clears_a_refusal_for_hidden_commit_details()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, _events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        let oid = Oid::from_bytes(&[1; 20])?;
        tab.selected_commit = Some(oid);
        for view in [View::Changes, View::Requests] {
            for failed in [false, true] {
                tab.view = View::History;
                tab.downloads_refused = Some("origin".into());
                tab.details_error = failed.then(|| DetailsError {
                    oid,
                    message: crate::worker::NOT_TRUSTED.into(),
                    can_download: false,
                });
                tab.clear_obsolete_refusal();
                assert!(tab.downloads_refused.is_some(), "History still needs it");
                // The view switch keeps the commit selection for a later return to History.
                tab.view = view;
                tab.deselect_file();
                assert!(
                    tab.downloads_refused.is_none(),
                    "hidden details in {view:?}"
                );
                assert_eq!(tab.selected_commit, Some(oid));
            }
        }
        Ok(())
    }

    /// #36: a diff a partial clone could not download is not asked for again by the refreshes
    /// the file watcher makes; Retry (or choosing the file again) asks, its file row is marked
    /// until a read succeeds.
    #[test]
    fn a_diff_that_could_not_be_downloaded_waits_for_retry()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.view = View::Changes;
        let selection = open_unstaged_file();
        let key = tab.diff_key(&selection);
        let marker = (key.source, key.path.clone());
        tab.select_file(selection);
        let diff_jobs = || {
            jobs.try_iter()
                .filter(|j| matches!(j, Job::Diff(_)))
                .count()
        };
        assert_eq!(diff_jobs(), 1, "selecting asks once");

        events.send(Event::Diff(key.clone(), Err("not downloaded".into())))?;
        events.send(Event::DiffNotDownloaded(key.clone()))?;
        tab.poll(&[]);
        assert!(tab.diff_error.is_some() && tab.failed_diff.is_some());
        assert!(tab.not_downloaded.contains(&marker), "the row is marked");

        // What the watcher does after every change: no new download attempt.
        for _ in 0..3 {
            tab.reload_diff();
        }
        assert_eq!(diff_jobs(), 0, "a refresh does not ask again");
        assert!(tab.diff_error.is_some(), "the failure stays on screen");

        tab.retry_diff();
        assert_eq!(diff_jobs(), 1, "Retry asks");
        assert!(tab.failed_diff.is_none());
        events.send(Event::Diff(key, Ok(Arc::new(FileDiff::default()))))?;
        tab.poll(&[]);
        assert!(tab.diff_error.is_none());
        assert!(!tab.not_downloaded.contains(&marker), "the mark goes");
        Ok(())
    }

    #[test]
    fn a_refusal_split_from_its_diff_result_keeps_the_trust_action()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.view = View::Changes;
        let selection = open_unstaged_file();
        let key = tab.diff_key(&selection);
        tab.select_file(selection);
        for keep_old in [false, true] {
            tab.diff = keep_old.then(|| Arc::new(FileDiff::default()));
            tab.reload_diff();
            events.send(Event::DownloadsRefused {
                remote: "origin".into(),
                trust: 0,
            })?;
            tab.poll(&[]);
            assert!(
                tab.downloads_refused.is_some(),
                "refusal arrived before the result"
            );
            events.send(Event::Diff(
                key.clone(),
                Err(crate::worker::NOT_TRUSTED.into()),
            ))?;
            tab.poll(&[]);
            assert!(tab.diff_error.is_some());
            assert!(
                tab.downloads_refused.is_some(),
                "the visible error needs Trust"
            );
            // A later successful read removes the error and its chip.
            events.send(Event::Diff(key.clone(), Ok(Arc::new(FileDiff::default()))))?;
            tab.poll(&[]);
            assert!(tab.downloads_refused.is_none());
        }
        tab.reload_diff();
        events.send(Event::DownloadsRefused {
            remote: "origin".into(),
            trust: 0,
        })?;
        tab.poll(&[]);
        tab.deselect_file();
        assert!(
            tab.downloads_refused.is_none(),
            "closing the pending diff clears it"
        );
        Ok(())
    }

    #[test]
    fn a_late_line_count_refusal_does_not_reappear_outside_history()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        let oid = Oid::from_bytes(&[1; 20])?;
        tab.selected_commit = Some(oid);
        let author = canopy_core::Signature {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time: 0,
            offset_minutes: 0,
        };
        let details = Arc::new(CommitDetails {
            oid,
            message: "Missing line counts".into(),
            author: author.clone(),
            committer: author,
            parents: Vec::new(),
            files: Vec::new(),
            stats_known: false,
        });
        for view in [View::Changes, View::Requests] {
            for split_poll in [false, true] {
                tab.view = view;
                events.send(Event::Details(Arc::clone(&details)))?;
                if split_poll {
                    tab.poll(&[]);
                }
                // Counts fail after the offline file list, with no DetailsFailed event.
                events.send(Event::DownloadsRefused {
                    remote: "origin".into(),
                    trust: 0,
                })?;
                tab.poll(&[]);
                assert!(
                    tab.downloads_refused.is_none(),
                    "hidden counts, {view:?}, split={split_poll}"
                );
            }
        }
        // In History the same event sequence still explains the missing counts.
        tab.view = View::History;
        events.send(Event::Details(details))?;
        events.send(Event::DownloadsRefused {
            remote: "origin".into(),
            trust: 0,
        })?;
        tab.poll(&[]);
        assert!(tab.downloads_refused.is_some());
        Ok(())
    }

    #[test]
    fn a_signature_belongs_to_the_selected_commit_only() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, jobs) = Worker::detached();
        tab.worker = worker;
        let (a, b) = (
            Oid::from_str("1111111111111111111111111111111111111111")?,
            Oid::from_str("2222222222222222222222222222222222222222")?,
        );
        let status = canopy_core::SignatureStatus::Unavailable;
        tab.select_commit(a);
        assert!(
            jobs.try_iter()
                .any(|j| matches!(j, Job::Signature(o) if o == a))
        );
        events.send(Event::Signature {
            oid: b,
            status: Some(status.clone()),
        })?;
        tab.poll(&[]);
        assert_eq!(tab.signature, None, "another commit's reply is ignored");
        events.send(Event::Signature {
            oid: a,
            status: Some(status.clone()),
        })?;
        tab.poll(&[]);
        assert_eq!(tab.signature, Some((a, status)));
        tab.select_commit(b);
        assert_eq!(tab.signature, None, "a new selection forgets the old one");
        Ok(())
    }

    #[test]
    fn a_failed_status_refresh_does_not_leave_an_unexplained_refusal()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.view = View::Changes;
        events.send(Event::DownloadsRefused {
            remote: "origin".into(),
            trust: 0,
        })?;
        events.send(Event::Failed(crate::worker::NOT_TRUSTED.into()))?;
        let notices = tab.poll(&[]);
        assert!(
            tab.downloads_refused.is_none(),
            "no visible status or details refusal"
        );
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].body, crate::worker::NOT_TRUSTED);
        Ok(())
    }

    #[test]
    fn an_initial_load_failure_keeps_the_chip_only_when_it_explains_the_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.view = View::Changes;
        for (message, kept) in [
            (crate::worker::NOT_TRUSTED, true),
            ("object not found", false),
        ] {
            events.send(Event::DownloadsRefused {
                remote: "origin".into(),
                trust: 0,
            })?;
            events.send(Event::Failed(message.into()))?;
            tab.poll(&[]);
            assert_eq!(tab.load_error.as_deref(), Some(message));
            assert_eq!(tab.downloads_refused.is_some(), kept);
        }
        Ok(())
    }

    #[test]
    fn a_commit_too_big_for_line_counts_does_not_keep_the_chip()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        let commit = Oid::from_bytes(&[1; 20])?;
        tab.selected_commit = Some(commit);
        tab.downloads_refused = Some("origin".into());
        let author = canopy_core::Signature {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time: 0,
            offset_minutes: 0,
        };
        let files = |count: usize| {
            (0..count)
                .map(|i| FileEntry {
                    path: format!("f{i}.rs"),
                    old_path: None,
                    kind: canopy_core::ChangeKind::Modified,
                    additions: 0,
                    deletions: 0,
                    lfs: false,
                    old_mode: None,
                    new_mode: None,
                })
                .collect::<Vec<_>>()
        };
        let details = |count| {
            Arc::new(CommitDetails {
                oid: commit,
                message: "A commit".into(),
                author: author.clone(),
                committer: author.clone(),
                parents: Vec::new(),
                files: files(count),
                stats_known: false,
            })
        };
        // The worker asks for the counts of up to MAX_STAT_FILES files: this one still waits for them.
        events.send(Event::Details(details(canopy_core::repo::MAX_STAT_FILES)))?;
        tab.poll(&[]);
        // Details opens the first file too. Finish its independent diff request so only the
        // missing line counts can keep the chip; this test isolates the count limit.
        events.send(Event::Diff(
            tab.pending_diff
                .clone()
                .expect("the first file's diff request"),
            Ok(Arc::new(FileDiff::default())),
        ))?;
        tab.poll(&[]);
        assert!(tab.downloads_refused.is_some(), "its counts are refused");
        // One file more and the worker never asks: the missing counts are not a refusal.
        events.send(Event::Details(details(
            canopy_core::repo::MAX_STAT_FILES + 1,
        )))?;
        tab.poll(&[]);
        assert!(tab.downloads_refused.is_none(), "nothing left to explain");
        Ok(())
    }

    #[test]
    fn a_complete_commit_does_not_hide_a_refusal_still_in_effect()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot {
            status_unavailable: Some(canopy_core::StatusUnavailable {
                message: crate::worker::NOT_TRUSTED.into(),
                can_download: false,
            }),
            ..Default::default()
        }));
        let commit = Oid::from_bytes(&[1; 20])?;
        tab.selected_commit = Some(commit);
        tab.downloads_refused = Some("origin".into());
        let author = canopy_core::Signature {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time: 0,
            offset_minutes: 0,
        };
        let complete = Arc::new(CommitDetails {
            oid: commit,
            message: "A commit".into(),
            author: author.clone(),
            committer: author,
            parents: Vec::new(),
            files: Vec::new(),
            stats_known: true,
        });
        events.send(Event::Details(complete))?;
        tab.poll(&[]);
        assert!(
            tab.downloads_refused.is_some(),
            "the status still cannot be shown"
        );
        events.send(Event::Status(canopy_core::WorkingStatus::default()))?;
        tab.poll(&[]);
        assert!(tab.downloads_refused.is_none(), "nothing left to explain");
        Ok(())
    }

    #[test]
    fn a_good_diff_clears_the_refusal_its_failed_twin_left()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot::default()));
        tab.file = Some(open_unstaged_file());
        tab.downloads_refused = Some("origin".into());
        let key = DiffKey {
            source: DiffSource::Unstaged,
            path: "a.rs".into(),
            old_path: None,
            conflicted: false,
            context: 3,
            whitespace: canopy_core::Whitespace::default(),
            highlight: false,
        };
        tab.pending_diff = Some(key.clone());
        events.send(Event::Diff(
            key.clone(),
            Err(crate::worker::NOT_TRUSTED.into()),
        ))?;
        tab.poll(&[]);
        assert!(tab.diff_error.is_some());
        assert!(
            tab.downloads_refused.is_some(),
            "the diff error still needs the chip"
        );
        let diff = Arc::new(FileDiff {
            path: "a.rs".into(),
            ..Default::default()
        });
        events.send(Event::Diff(key, Ok(diff)))?;
        tab.poll(&[]);
        assert!(tab.diff_error.is_none());
        assert!(tab.downloads_refused.is_none(), "nothing left to explain");
        Ok(())
    }

    /// #228: the repository thread says whether the repository is trusted; an answer given
    /// before a change of the list this tab already knows of is out of date, and the change
    /// wakes the thread so that it answers again.
    #[test]
    fn a_trust_answer_older_than_a_known_change_is_ignored()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, jobs) = Worker::detached();
        tab.worker = worker;
        assert_eq!(
            tab.trust,
            RepoTrust::Unknown,
            "nothing is shown before it answers"
        );
        events.send(Event::Trust {
            trusted: false,
            version: 1,
        })?;
        tab.poll(&[]);
        assert_eq!(tab.trust, RepoTrust::Untrusted);

        tab.trust_changed(2);
        assert!(
            jobs.try_iter().any(|job| matches!(job, Job::RefreshStatus)),
            "the thread is asked to look again"
        );
        events.send(Event::Trust {
            trusted: false,
            version: 1,
        })?;
        events.send(Event::Trust {
            trusted: true,
            version: 2,
        })?;
        tab.poll(&[]);
        assert_eq!(tab.trust, RepoTrust::Trusted);
        events.send(Event::Trust {
            trusted: false,
            version: 1,
        })?;
        tab.poll(&[]);
        assert_eq!(tab.trust, RepoTrust::Trusted, "the older answer is dropped");
        Ok(())
    }

    /// An automatic fetch that fails the same way every interval (a server that is down) is shown
    /// once; a different failure, or a fetch that works, shows
    /// again.
    #[test]
    fn an_automatic_fetch_failure_is_shown_once() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let finished = |announce, result: Result<&str, &str>| Event::OpFinished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            announce,
            result: result.map(str::to_owned).map_err(str::to_owned),
        };
        let shown = |tab: &mut RepoTab, e: Event| -> Result<usize, Box<dyn std::error::Error>> {
            events.send(e)?;
            Ok(tab.poll(&[]).len())
        };
        let refusal = "could not read from the remote repository";
        assert_eq!(shown(&mut tab, finished(false, Err(refusal)))?, 1);
        assert_eq!(
            shown(&mut tab, finished(false, Err(refusal)))?,
            0,
            "again, quietly"
        );
        assert_eq!(
            shown(&mut tab, finished(false, Err("could not resolve host")))?,
            1
        );
        assert_eq!(shown(&mut tab, finished(false, Ok("")))?, 0);
        assert_eq!(
            shown(&mut tab, finished(false, Err(refusal)))?,
            1,
            "after a good fetch"
        );
        tab.trust_changed(1);
        assert_eq!(
            shown(&mut tab, finished(false, Err(refusal)))?,
            1,
            "trust changed"
        );
        // The fetch the user starts always says how it went.
        assert_eq!(shown(&mut tab, finished(true, Err(refusal)))?, 1);
        assert_eq!(shown(&mut tab, finished(true, Err(refusal)))?, 1);
        Ok(())
    }

    /// A fetch that finishes, the user's or the timer's, restarts the interval from then.
    #[test]
    fn a_finished_fetch_restarts_the_auto_fetch_interval() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let every = Duration::from_mins(10);
        let soon = Instant::now();
        tab.fetch_every = Some(every);
        tab.next_fetch = Some(soon);
        events.send(Event::OpFinished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            announce: true,
            result: Ok(String::new()),
        })?;
        tab.poll(&[]);
        let next = tab.next_fetch.expect("rescheduled");
        assert!(
            next >= soon + every,
            "a whole interval after the fetch, not the old due time"
        );
        // Another operation leaves the schedule alone.
        tab.next_fetch = Some(soon);
        events.send(Event::OpFinished {
            label: "Staging".into(),
            kind: OpKind::Other,
            announce: false,
            result: Ok(String::new()),
        })?;
        tab.poll(&[]);
        assert_eq!(tab.next_fetch, Some(soon));
        // Without auto-fetch there is no schedule to restart.
        tab.fetch_every = None;
        events.send(Event::OpFinished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            announce: true,
            result: Ok(String::new()),
        })?;
        tab.poll(&[]);
        assert_eq!(tab.next_fetch, None);
        Ok(())
    }

    fn tab_with_detached_worker() -> (RepoTab, crossbeam_channel::Sender<Event>) {
        let (worker, events, _jobs) = Worker::detached();
        let tab = RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        (tab, events)
    }

    fn quiet_fetch_finished(
        tab: &mut RepoTab,
        events: &crossbeam_channel::Sender<Event>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        events.send(Event::OpFinished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            announce: false,
            result: Ok(String::new()),
        })?;
        tab.poll(&[]);
        Ok(())
    }

    /// #62: a slot of the limit on background fetches is given back by the end of that fetch only:
    /// not by a read that failed meanwhile.
    #[test]
    fn an_unrelated_failure_does_not_free_the_automatic_fetch_slot()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut tab, events) = tab_with_detached_worker();
        tab.auto_fetch(false);
        assert!(tab.auto_fetch_pending);
        events.send(Event::Failed("the status read failed".into()))?;
        tab.poll(&[]);
        assert!(tab.auto_fetch_pending, "a failed read freed the slot");
        quiet_fetch_finished(&mut tab, &events)?;
        assert!(!tab.auto_fetch_pending, "its own end frees it");
        Ok(())
    }

    /// #62: a fetch a tab that has not loaded dropped is never answered, so it holds no slot and
    /// no place in the order of quiet fetches.
    #[test]
    fn an_automatic_fetch_that_was_not_sent_holds_no_slot() {
        let (worker, _events, _jobs) = Worker::lazy_detached(PathBuf::from("fixture"));
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.auto_fetch(false);
        assert!(!tab.auto_fetch_pending);
        assert!(tab.quiet_fetches.is_empty());
        assert_eq!(Worker::take_unstarted_sends(), ["operation"]);
    }

    /// #62: the quiet fetch sent after a request is merged is not the automatic one, whichever
    /// order the two were sent in.
    #[test]
    fn another_quiet_fetch_does_not_free_the_automatic_fetch_slot()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut tab, events) = tab_with_detached_worker();
        tab.op(Op::Fetch {
            prune: false,
            quiet: true,
        });
        tab.auto_fetch(false);
        quiet_fetch_finished(&mut tab, &events)?;
        assert!(
            tab.auto_fetch_pending,
            "the other fetch's end freed the slot"
        );
        quiet_fetch_finished(&mut tab, &events)?;
        assert!(!tab.auto_fetch_pending);

        tab.auto_fetch(false);
        tab.op(Op::Fetch {
            prune: false,
            quiet: true,
        });
        quiet_fetch_finished(&mut tab, &events)?;
        assert!(!tab.auto_fetch_pending, "the automatic one ended first");
        quiet_fetch_finished(&mut tab, &events)?;
        // A fetch the user started changes nothing about the count.
        tab.auto_fetch(false);
        tab.op(Op::Fetch {
            prune: false,
            quiet: false,
        });
        events.send(Event::OpFinished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            announce: true,
            result: Ok(String::new()),
        })?;
        tab.poll(&[]);
        assert!(tab.auto_fetch_pending);
        Ok(())
    }

    /// #234: a background failure never interrupts (the center and the bell hold it), while one
    /// the user started, and one that waits on them, does.
    #[test]
    fn notices_say_how_they_reach_the_user() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let fetch = |announce| Event::OpFinished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            announce,
            result: Err("could not resolve host".into()),
        };
        let delivery =
            |tab: &mut RepoTab, e: Event| -> Result<Delivery, Box<dyn std::error::Error>> {
                events.send(e)?;
                Ok(tab.poll(&[]).remove(0).delivery)
            };
        assert_eq!(delivery(&mut tab, fetch(false))?, Delivery::Quiet);
        assert_eq!(delivery(&mut tab, fetch(true))?, Delivery::Card);
        let stage = Event::OpFinished {
            label: "Staging".into(),
            kind: OpKind::Other,
            announce: false,
            result: Err("fatal: Unable to create index.lock".into()),
        };
        assert_eq!(delivery(&mut tab, stage)?, Delivery::Card, "the user's own");
        assert_eq!(
            delivery(
                &mut tab,
                Event::Conflicted {
                    label: "Pulling".into(),
                    autostashed: false,
                    stash: false
                }
            )?,
            Delivery::Attention
        );
        assert_eq!(
            delivery(&mut tab, Event::AutostashConflict)?,
            Delivery::Attention
        );
        Ok(())
    }

    /// The squash check is skipped while a repository is not trusted, so trusting it reads the
    /// markers again; an answer that changes nothing (still untrusted, already trusted) does not.
    #[test]
    fn trusting_a_repository_reads_the_merged_markers_again()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, jobs) = Worker::detached();
        tab.worker = worker;
        tab.snap = Some(Arc::new(RepoSnapshot {
            workflow: canopy_core::WorkflowInfo {
                base: Some(canopy_core::BranchBase {
                    references: vec!["main".into()],
                    protected: vec!["main".into()],
                }),
                ..canopy_core::WorkflowInfo::default()
            },
            ..RepoSnapshot::default()
        }));
        let asked = |jobs: &crossbeam_channel::Receiver<Job>| {
            jobs.try_iter()
                .filter(|j| matches!(j, Job::MergedBranches { .. }))
                .count()
        };
        tab.read_merged(false);
        assert_eq!(asked(&jobs), 1);

        events.send(Event::Trust {
            trusted: false,
            version: 1,
        })?;
        tab.poll(&[]);
        assert_eq!(asked(&jobs), 0, "still not trusted");

        events.send(Event::Trust {
            trusted: true,
            version: 2,
        })?;
        tab.poll(&[]);
        assert_eq!(asked(&jobs), 1, "trusted now: the squash check can run");

        events.send(Event::Trust {
            trusted: true,
            version: 3,
        })?;
        tab.poll(&[]);
        assert_eq!(asked(&jobs), 0, "already trusted");
        Ok(())
    }

    /// The merged markers cost a git process per branch that is not plainly merged: they are asked
    /// again only when a branch or remote-tracking tip, or the base, changed, never on a refresh
    /// that moved nothing; a cleanup form asks anyway; an older answer is dropped.
    #[test]
    fn the_merged_readings_run_again_only_when_a_branch_moves()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, jobs) = Worker::detached();
        tab.worker = worker;
        let oid = |n: u8| Oid::from_bytes(&[n; 20]).expect("oid");
        let snapshot = |tip: u8| RepoSnapshot {
            branches: vec![canopy_core::BranchInfo {
                name: "topic".into(),
                target: Some(oid(tip)),
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                worktree: None,
            }],
            workflow: canopy_core::WorkflowInfo {
                base: Some(canopy_core::BranchBase {
                    references: vec!["main".into(), "origin/main".into()],
                    protected: vec!["main".into()],
                }),
                ..canopy_core::WorkflowInfo::default()
            },
            ..RepoSnapshot::default()
        };
        let asked = |jobs: &crossbeam_channel::Receiver<Job>| {
            jobs.try_iter()
                .filter(|j| matches!(j, Job::MergedBranches { .. }))
                .count()
        };

        tab.snap = Some(Arc::new(snapshot(1)));
        let first = tab.read_merged(false);
        assert_eq!(asked(&jobs), 1);
        // A refresh that moved nothing.
        tab.snap = Some(Arc::new(snapshot(1)));
        assert_eq!(tab.read_merged(false), first);
        assert_eq!(asked(&jobs), 0);
        // A branch moved.
        tab.snap = Some(Arc::new(snapshot(2)));
        let second = tab.read_merged(false);
        assert_ne!(second, first);
        assert_eq!(asked(&jobs), 1);
        // A form asks for a reading made now.
        let third = tab.read_merged(true);
        assert_ne!(third, second);
        assert_eq!(asked(&jobs), 1);

        // Only the newest answer counts.
        let rows = |name: &str| -> Arc<[canopy_core::MergedBranch]> {
            Arc::from(vec![canopy_core::MergedBranch {
                name: name.into(),
                is_head: false,
                reason: Some(canopy_core::MergedReason::Reachable),
            }])
        };
        events.send(Event::MergedBranches {
            serial: second,
            bases: vec!["main".into()],
            result: Ok(rows("old")),
        })?;
        events.send(Event::MergedBranches {
            serial: third,
            bases: vec!["main".into(), "origin/main".into()],
            result: Ok(rows("new")),
        })?;
        tab.poll(&[]);
        let Some(Ok(marks)) = &tab.merged else {
            panic!("an answer");
        };
        assert_eq!(tab.merged_applied, third);
        assert_eq!(marks.shown(), "main");
        assert_eq!(
            marks.reason("new"),
            Some(canopy_core::MergedReason::Reachable)
        );
        assert_eq!(marks.reason("old"), None);

        // No base, no markers.
        let mut bare = snapshot(2);
        bare.workflow.base = None;
        tab.snap = Some(Arc::new(bare));
        tab.read_merged(false);
        assert!(tab.merged.is_none());
        assert_eq!(asked(&jobs), 0);
        Ok(())
    }

    /// #193: git's refusal to add a worktree goes to the dialog that asked (through the app), not
    /// straight to a toast; a success still says so.
    #[test]
    fn a_worktree_refusal_is_kept_for_its_dialog() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let finished = |result: Result<&str, &str>| Event::OpFinished {
            label: "Adding a worktree for topic".into(),
            kind: OpKind::Worktree,
            announce: true,
            result: result.map(str::to_owned).map_err(str::to_owned),
        };
        let refusal = "fatal: '/work/busy' already exists";
        events.send(finished(Err(refusal)))?;
        assert!(tab.poll(&[]).is_empty(), "no toast of its own");
        assert_eq!(tab.worktree_result.take(), Some(Err(refusal.to_owned())));
        events.send(finished(Ok("Added a worktree")))?;
        assert_eq!(tab.poll(&[]).len(), 1);
        assert_eq!(tab.worktree_result, Some(Ok(())));
        Ok(())
    }

    #[test]
    fn an_unmerged_branch_is_offered_for_a_forced_delete() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let finished = |result| Event::OpFinished {
            label: "Deleting topic".into(),
            kind: OpKind::SafeDelete("topic".into()),
            announce: true,
            result,
        };
        events.send(finished(Err(
            "error: the branch 'topic' is not fully merged".into(),
        )))?;
        let notices = tab.poll(&[]);
        assert!(
            notices.is_empty(),
            "git's refusal is a question, not an error"
        );
        assert_eq!(tab.force_delete_offer.as_deref(), Some("topic"));
        tab.force_delete_offer = None;
        events.send(finished(Err("fatal: branch 'topic' not found".into())))?;
        let notices = tab.poll(&[]);
        assert!(notices.iter().any(|n| !n.ok));
        assert!(tab.force_delete_offer.is_none());
        Ok(())
    }

    /// A diverged pull asks how to combine the branches (the worker says so, whatever git's
    /// wording); a pull that failed is an error, whatever its text.
    #[test]
    fn a_diverged_pull_asks_how_to_combine_the_branches() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let finished = |result| Event::OpFinished {
            label: "Pulling".into(),
            kind: OpKind::Pull,
            announce: true,
            result,
        };
        events.send(Event::PullDecision(Box::new(crate::worker::PullDecision {
            preview: canopy_core::PullPreview::default(),
            strategy: canopy_core::PullStrategy::FastForwardOnly,
            diverged: true,
            configured: false,
            moved: false,
        })))?;
        let notices = tab.poll(&[]);
        assert!(notices.is_empty(), "a choice to make, not a failure");
        assert!(tab.pull_decision.take().is_some_and(|d| d.diverged));
        events.send(finished(Err(
            "fatal: Not possible to fast-forward, aborting.".into(),
        )))?;
        let notices = tab.poll(&[]);
        assert!(notices.iter().any(|n| !n.ok), "git's words decide nothing");
        assert!(tab.pull_decision.is_none());
        events.send(finished(Err(
            "fatal: unable to access 'https://example.com/r.git/'".into(),
        )))?;
        let notices = tab.poll(&[]);
        assert!(notices.iter().any(|n| !n.ok));
        assert!(tab.pull_decision.is_none());
        Ok(())
    }

    /// A pull that waits for a decision stops showing as running; one that stopped on conflicts
    /// takes the user to Changes instead of reporting a failure.
    #[test]
    fn a_pull_decision_or_its_conflicts_end_the_running_pull()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.running = Some("Pulling".into());
        let decision = crate::worker::PullDecision {
            preview: canopy_core::PullPreview::default(),
            strategy: canopy_core::PullStrategy::Rebase,
            diverged: false,
            configured: false,
            moved: false,
        };
        events.send(Event::PullDecision(Box::new(decision.clone())))?;
        assert!(tab.poll(&[]).is_empty());
        assert_eq!(tab.running, None);
        assert_eq!(tab.pull_decision, Some(decision));
        tab.running = Some("Pulling".into());
        tab.view = View::History;
        events.send(Event::Conflicted {
            label: "Pulling".into(),
            autostashed: false,
            stash: false,
        })?;
        let notices = tab.poll(&[]);
        assert_eq!((tab.running.as_deref(), tab.view), (None, View::Changes));
        assert_eq!(notices.len(), 1);
        Ok(())
    }

    /// A stop on conflicts says where uncommitted changes are when git holds them aside, and says
    /// nothing of them when it does not.
    #[test]
    fn a_stop_on_conflicts_says_where_set_aside_changes_are() {
        let plain = stopped_body(false, false);
        assert!(plain.starts_with("Resolve each conflicted file in Changes"));
        assert!(!plain.contains("uncommitted"), "{plain}");
        let held = stopped_body(true, false);
        assert!(held.starts_with(&plain), "{held}");
        assert!(
            held.contains("uncommitted changes are held by git")
                && held.contains("come back when you finish or abort"),
            "{held}"
        );
        assert!(!held.contains("\n\n"), "one paragraph: {held}");
    }

    /// #265: a stash that stopped on conflicts has nothing to continue or abort, and says the
    /// stash is kept; changes git holds aside do not come into it.
    #[test]
    fn a_stash_that_stopped_on_conflicts_is_not_asked_to_continue_or_abort() {
        let body = stopped_body(false, true);
        assert!(
            body.starts_with("Resolve each conflicted file in Changes"),
            "{body}"
        );
        assert!(body.contains("stays in the stash list"), "{body}");
        assert!(
            !body.contains("abort") && !body.contains("continue"),
            "{body}"
        );
        assert_eq!(stopped_body(true, true), body);
    }

    /// The notice of a stop carries what [`stopped_body`] says for the event's `autostashed`.
    #[test]
    fn the_stopped_notice_follows_whether_changes_were_set_aside()
    -> Result<(), Box<dyn std::error::Error>> {
        for autostashed in [false, true] {
            let ctx = egui::Context::default();
            let dir = tempfile::tempdir()?;
            let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
            let (worker, events, _jobs) = Worker::detached();
            tab.worker = worker;
            events.send(Event::Conflicted {
                label: "Merging".into(),
                autostashed,
                stash: false,
            })?;
            let notices = tab.poll(&[]);
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].body, stopped_body(autostashed, false));
        }
        Ok(())
    }

    /// Changes put aside for a pull that conflict when they come back are said plainly, and the
    /// user lands in Changes, where the conflicted files and the resolver are.
    #[test]
    fn an_autostash_conflict_takes_the_user_to_changes() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        tab.view = View::History;
        events.send(Event::AutostashConflict)?;
        let notices = tab.poll(&[]);
        assert_eq!(tab.view, View::Changes);
        assert_eq!(notices.len(), 1);
        assert!(!notices[0].ok, "it needs the user");
        Ok(())
    }

    #[test]
    fn blame_rejects_superseded_replies_and_surfaces_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.read_blame("old.rs".into(), None);
        let Job::Blame { serial: old, .. } = jobs.recv()? else {
            panic!("blame job");
        };
        tab.read_blame("new.rs".into(), None);
        let Job::Blame {
            serial: current,
            path,
            rev: None,
        } = jobs.recv()?
        else {
            panic!("blame job");
        };
        assert_eq!(path, "new.rs");
        events.send(Event::Blame {
            serial: old,
            result: Err("old read".into()),
        })?;
        assert_eq!(tab.poll(&[]).len(), 0);
        assert!(tab.blame.as_ref().is_some_and(|b| b.result.is_none()));
        events.send(Event::Blame {
            serial: current,
            result: Err("no such path".into()),
        })?;
        let notices = tab.poll(&[]);
        assert_eq!(notices.len(), 1);
        assert!(!notices[0].ok);
        assert!(matches!(
            tab.blame,
            Some(BlameRead { result: Some(Err(ref error)), .. }) if error == "no such path"
        ));
        Ok(())
    }

    #[test]
    fn a_blame_follows_its_file_and_is_refreshed_with_the_working_tree()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        let file = |path: &str, source| FileSelection {
            source,
            entry: FileEntry {
                path: path.into(),
                old_path: None,
                kind: canopy_core::ChangeKind::Modified,
                additions: 0,
                deletions: 0,
                lfs: false,
                old_mode: None,
                new_mode: None,
            },
            conflicted: false,
        };
        tab.select_file(file("a.rs", DiffSource::Unstaged));
        tab.blame_file();
        assert!(tab.blame.is_some());
        let mut serials = Vec::new();
        while let Ok(job) = jobs.try_recv() {
            if let Job::Blame { serial, .. } = job {
                serials.push(serial);
            }
        }
        assert_eq!(serials.len(), 1);

        // The working tree changed: the same blame is read again, the old one stays meanwhile.
        tab.reload_diff();
        assert!(tab.blame.is_some());
        let again = std::iter::from_fn(|| jobs.try_recv().ok())
            .find_map(|job| match job {
                Job::Blame { serial, .. } => Some(serial),
                _ => None,
            })
            .expect("a new blame job");
        assert_ne!(again, serials[0]);

        // Another file, another source, or none: the blame goes, and its late reply is ignored.
        tab.select_file(file("a.rs", DiffSource::Staged));
        assert!(tab.blame.is_none());
        events.send(Event::Blame {
            serial: again,
            result: Err("late".into()),
        })?;
        assert!(tab.poll(&[]).is_empty(), "no toast for a view that is gone");
        tab.blame_file();
        assert!(tab.blame.is_some());
        tab.deselect_file();
        assert!(tab.blame.is_none());
        Ok(())
    }

    #[test]
    fn insights_rejects_superseded_replies_and_surfaces_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.read_insights();
        let Job::Insights(old) = jobs.recv()? else {
            panic!("insights job");
        };
        tab.read_insights();
        let Job::Insights(current) = jobs.recv()? else {
            panic!("insights job");
        };
        assert_ne!(old, current);
        events.send(Event::Insights {
            serial: old,
            result: Err("old read".into()),
        })?;
        assert_eq!(tab.poll(&[]).len(), 0);
        assert!(tab.insights.is_none());
        events.send(Event::Insights {
            serial: current,
            result: Err("missing objects".into()),
        })?;
        let notices = tab.poll(&[]);
        assert_eq!(notices.len(), 1);
        assert!(!notices[0].ok);
        assert!(matches!(tab.insights, Some(Err(ref error)) if error == "missing objects"));
        tab.read_insights();
        let Job::Insights(next) = jobs.recv()? else {
            panic!("insights job");
        };
        assert!(tab.insights.is_none());
        let data = Arc::new(crate::worker::Insights {
            repository: canopy_core::RepositoryMetrics {
                weeks: Vec::new(),
                authors: Vec::new(),
                last_commit: None,
                size_bytes: 0,
                objects: 0,
            },
            branches: Vec::new(),
        });
        events.send(Event::Insights {
            serial: next,
            result: Ok(Arc::clone(&data)),
        })?;
        assert_eq!(tab.poll(&[]).len(), 0);
        assert!(matches!(tab.insights, Some(Ok(ref result)) if Arc::ptr_eq(result, &data)));
        Ok(())
    }

    #[test]
    fn a_change_search_ignores_superseded_replies_and_keeps_its_error_for_the_dialog()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.search_changes("old".into(), false);
        let Job::SearchChanges { serial: old, .. } = jobs.recv()? else {
            panic!("search job");
        };
        tab.search_changes("(new".into(), true);
        let Job::SearchChanges {
            serial: current,
            query,
            regex,
        } = jobs.recv()?
        else {
            panic!("search job");
        };
        assert_eq!((query.as_str(), regex), ("(new", true));
        events.send(Event::ChangeSearch {
            serial: old,
            result: Err("old search".into()),
        })?;
        tab.poll(&[]);
        assert!(tab.change_search.is_none());
        events.send(Event::ChangeSearch {
            serial: current,
            result: Err("fatal: unmatched parenthesis".into()),
        })?;
        // The dialog shows the error beside the field: no toast as well.
        assert!(tab.poll(&[]).is_empty());
        assert!(matches!(tab.change_search, Some(Err(_))));
        tab.search_changes("again".into(), false);
        assert!(tab.change_search.is_none());
        Ok(())
    }

    #[test]
    fn file_history_rejects_superseded_replies_and_surfaces_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.read_file_history(None, "a.txt".into());
        let Job::FileHistory { serial: old, .. } = jobs.recv()? else {
            panic!("file history job");
        };
        tab.read_file_history(Some(Oid::ZERO_SHA1), "b.txt".into());
        let Job::FileHistory {
            serial: current,
            from,
            path,
        } = jobs.recv()?
        else {
            panic!("file history job");
        };
        assert_eq!((from, path.as_str()), (Some(Oid::ZERO_SHA1), "b.txt"));
        events.send(Event::FileHistory {
            serial: old,
            result: Err("old read".into()),
        })?;
        assert_eq!(tab.poll(&[]).len(), 0);
        assert!(tab.file_history.is_none());
        events.send(Event::FileHistory {
            serial: current,
            result: Err("fatal: bad revision".into()),
        })?;
        let notices = tab.poll(&[]);
        assert_eq!(notices.len(), 1);
        assert!(!notices[0].ok && notices[0].body == "fatal: bad revision");
        assert!(matches!(tab.file_history, Some(Err(_))));
        tab.read_file_history(None, "b.txt".into());
        assert!(tab.file_history.is_none());
        Ok(())
    }

    #[test]
    fn the_lfs_state_follows_the_workers_answers_and_lists_are_read_again_after_a_command()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        assert!(!tab.lfs.in_use());
        events.send(Event::Lfs(Arc::new(crate::worker::LfsInfo {
            in_use: true,
            files: vec![canopy_core::LfsFile {
                path: "big.bin".into(),
                pointer: canopy_core::LfsPointer {
                    oid: "a".repeat(64),
                    size: 9,
                    extensions: false,
                },
                downloaded: false,
            }],
            missing: None,
        })))?;
        tab.poll(&[]);
        assert!(tab.lfs.is_pointer_only("big.bin"));

        // Opening the dialog reads both lists; a stale answer is dropped.
        tab.read_lfs_lists();
        let Job::LfsPatterns(patterns) = jobs.recv()? else {
            panic!("patterns job");
        };
        let Job::LfsLocks(locks) = jobs.recv()? else {
            panic!("locks job");
        };
        events.send(Event::LfsLocks {
            serial: locks.wrapping_sub(1),
            result: Err("old".into()),
        })?;
        events.send(Event::LfsLocks {
            serial: locks,
            result: Ok(Vec::new()),
        })?;
        events.send(Event::LfsPatterns {
            serial: patterns,
            result: Err("git-lfs said no".into()),
        })?;
        tab.poll(&[]);
        assert!(matches!(tab.lfs.locks, Some(Ok(_))));
        assert!(matches!(&tab.lfs.patterns, Some(Err(e)) if e == "git-lfs said no"));

        // An LFS command that finished changes what the lists say.
        events.send(Event::OpFinished {
            label: "Locking big.bin".into(),
            kind: OpKind::Lfs,
            announce: true,
            result: Ok("Locked big.bin".into()),
        })?;
        tab.poll(&[]);
        assert!(matches!(jobs.recv()?, Job::LfsPatterns(_)));
        assert!(matches!(jobs.recv()?, Job::LfsLocks(_)));
        assert!(tab.lfs.locks.is_none(), "the lists are on their way again");
        Ok(())
    }

    #[test]
    fn a_file_version_opens_in_history_at_its_commit_and_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, _events, jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        tab.view = View::Changes;
        let oid = Oid::from_bytes(&[7; 20])?;
        tab.show_file_version(&canopy_core::FileHistoryEntry {
            oid,
            summary: "move".into(),
            author: "Ada".into(),
            time: 1,
            path: "new.txt".into(),
            old_path: Some("old.txt".into()),
            kind: Some(canopy_core::ChangeKind::Renamed),
        });
        assert_eq!((tab.view, tab.selected_commit), (View::History, Some(oid)));
        let file = tab.file.as_ref().expect("the file opens");
        assert_eq!(file.source, DiffSource::Commit(oid));
        assert_eq!(file.entry.old_path.as_deref(), Some("old.txt"));
        let mut diffs = Vec::new();
        while let Ok(job) = jobs.try_recv() {
            if let Job::Diff(key) = job {
                diffs.push((key.source, key.path, key.old_path));
            }
        }
        assert_eq!(
            diffs,
            [(
                DiffSource::Commit(oid),
                "new.txt".to_owned(),
                Some("old.txt".to_owned())
            )]
        );
        Ok(())
    }

    #[test]
    fn a_missing_tool_is_reported_once_per_run() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        let missing = |program: &str| canopy_core::MissingTool {
            program: program.into(),
            needed_for: "this repository stores files with Git LFS",
            install: vec!["Run brew install git-lfs.".into()],
        };
        events.send(Event::ToolsMissing(vec![missing("git-lfs-test-tool")]))?;
        let notices = tab.poll(&[]);
        assert_eq!(notices.len(), 1);
        assert!(!notices[0].ok);
        assert_eq!(notices[0].title, "git-lfs-test-tool is not installed");
        assert!(notices[0].body.contains("Git LFS"), "{}", notices[0].body);
        assert!(
            notices[0].body.contains("brew install"),
            "{}",
            notices[0].body
        );

        events.send(Event::ToolsMissing(vec![missing("git-lfs-test-tool")]))?;
        assert!(tab.poll(&[]).is_empty(), "already reported");
        Ok(())
    }

    #[test]
    fn a_folder_that_cannot_be_watched_is_a_quiet_notice() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir()?;
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 100, DiffPrefs::default());
        let (worker, events, _jobs) = Worker::detached();
        tab.worker = worker;
        events.send(Event::WatchFailed(
            "The system's limit on watched folders is reached.".into(),
        ))?;
        let notices = tab.poll(&[]);
        assert_eq!(notices.len(), 1);
        assert!(!notices[0].ok);
        assert_eq!(notices[0].delivery, Delivery::Quiet);
        assert_eq!(notices[0].title, "Changes on disk are not picked up");
        assert_eq!(
            notices[0].body,
            "The system's limit on watched folders is reached. Canopy shows changes on disk after a Refresh."
        );
        Ok(())
    }

    #[test]
    fn reloading_the_conflict_file_forgets_measured_heights() {
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(ConflictFile::parse(TWO))));
        r.layout = crate::views::resolve::Layout::with_height(1, 0, 99.0);
        r.reload(None, Ok(Arc::new(ConflictFile::parse(TWO))));
        assert!(r.layout.is_empty());
    }

    /// A tab on a forge whose request list is on screen, with request 7 open: the channels to
    /// feed it and read what it asks of the repository and hosting threads.
    fn tab_showing_requests() -> (
        RepoTab,
        crossbeam_channel::Sender<Event>,
        crossbeam_channel::Receiver<HostJob>,
    ) {
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("fixture"), 100, DiffPrefs::default(), worker);
        let forge = Forge {
            kind: ForgeKind::GitHub,
            host: "github.com".into(),
            port: None,
            path: "o/r".into(),
        };
        let (host, _host_events, host_jobs) = HostWorker::detached(forge);
        tab.host = Some(host);
        tab.requests.list = Some(Ok(Arc::from(Vec::new())));
        tab.requests.loaded_at = Some(Instant::now());
        tab.requests.selected = Some(7);
        tab.requests.details = Some(Err("a stand-in for the open request".into()));
        (tab, events, host_jobs)
    }

    fn finish(
        events: &crossbeam_channel::Sender<Event>,
        kind: OpKind,
        result: Result<String, String>,
    ) {
        let _ = events.send(Event::OpFinished {
            label: "Syncing".into(),
            kind,
            announce: false,
            result,
        });
    }

    fn jobs_of(host_jobs: &crossbeam_channel::Receiver<HostJob>) -> Vec<HostJob> {
        host_jobs.try_iter().collect()
    }

    #[test]
    fn a_fetch_pull_or_push_refreshes_the_request_list_quietly() {
        for kind in [OpKind::Fetch, OpKind::Pull, OpKind::Push] {
            let (mut tab, events, host_jobs) = tab_showing_requests();
            finish(&events, kind.clone(), Ok(String::new()));
            tab.poll(&[]);
            let jobs = jobs_of(&host_jobs);
            assert!(
                jobs.iter()
                    .any(|j| matches!(j, HostJob::List(k) if k.state == StateFilter::Open)),
                "{kind:?} asked for the list again: {jobs:?}"
            );
            assert!(
                jobs.iter().any(|j| matches!(j, HostJob::Details(7, _))),
                "{kind:?} asked for the open request again: {jobs:?}"
            );
            assert!(!tab.requests.loading, "{kind:?} showed a spinner");
            assert!(
                !tab.requests.details_loading,
                "{kind:?} blanked the details"
            );
            assert!(tab.requests.list.is_some(), "the list stays on screen");
            assert!(tab.requests.details.is_some(), "the open request stays");
            assert_eq!(tab.requests.selected, Some(7));
        }
    }

    #[test]
    fn a_failed_sync_or_other_operation_leaves_the_request_list_alone() {
        let (mut tab, events, host_jobs) = tab_showing_requests();
        finish(
            &events,
            OpKind::Fetch,
            Err("fatal: unable to access".into()),
        );
        finish(&events, OpKind::Other, Ok(String::new()));
        tab.poll(&[]);
        assert!(jobs_of(&host_jobs).is_empty());
    }

    #[test]
    fn a_sync_without_a_host_or_a_loaded_list_asks_the_forge_for_nothing() {
        let (mut tab, events, host_jobs) = tab_showing_requests();
        tab.requests.list = None;
        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        assert!(
            jobs_of(&host_jobs).is_empty(),
            "a list never loaded stays unloaded until its tab is opened"
        );

        let (mut tab, events, _host_jobs) = tab_showing_requests();
        tab.host = None;
        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        assert!(tab.host.is_none() && !tab.requests.loading);
    }

    #[test]
    fn a_late_comment_result_preserves_current_request_and_draft() {
        for result in [Ok(()), Err("offline".into())] {
            let (mut tab, _, _) = tab_showing_requests();
            let forge = tab.host.as_ref().expect("host").forge.clone();
            let (host, events, _) = HostWorker::detached(forge);
            tab.host = Some(host);
            tab.requests.comment = "submitted text".into();
            tab.host_action(HostJob::Comment(7, "submitted text".into()));
            tab.select_request(8);
            tab.requests.comment = "other request draft".into();
            let succeeded = result.is_ok();
            events
                .send(HostEvent::Done {
                    label: "Commenting".into(),
                    action: HostAction::Comment,
                    number: Some(7),
                    result,
                })
                .expect("result");
            tab.poll(&[]);
            assert_eq!(tab.requests.selected, Some(8));
            assert_eq!(tab.requests.comment, "other request draft");
            tab.select_request(7);
            assert_eq!(
                tab.requests.comment,
                if succeeded { "" } else { "submitted text" }
            );
        }
    }

    #[test]
    fn a_comment_result_does_not_clear_new_text_on_the_same_request() {
        let (mut tab, _, _) = tab_showing_requests();
        let forge = tab.host.as_ref().expect("host").forge.clone();
        let (host, events, _) = HostWorker::detached(forge);
        tab.host = Some(host);
        tab.requests.comment = "submitted text".into();
        tab.host_action(HostJob::Comment(7, "submitted text".into()));
        tab.requests.comment = "new unsent text".into();
        events
            .send(HostEvent::Done {
                label: "Commenting".into(),
                action: HostAction::Comment,
                number: Some(7),
                result: Ok(()),
            })
            .expect("result");
        tab.poll(&[]);
        assert_eq!(tab.requests.comment, "new unsent text");
    }

    #[test]
    fn a_manual_request_refresh_also_reloads_the_open_details() {
        let (mut tab, _events, host_jobs) = tab_showing_requests();
        tab.load_requests();
        let jobs = jobs_of(&host_jobs);
        assert!(
            matches!(
                jobs.as_slice(),
                [
                    HostJob::List(ListKey {
                        state: StateFilter::Open,
                        ..
                    }),
                    HostJob::Details(7, _)
                ]
            ),
            "both the list and selected request need fresh host data: {jobs:?}"
        );
        assert!(tab.requests.loading);
        assert!(tab.requests.details_loading);
        assert!(
            tab.requests.details.is_some(),
            "keep the old details until refreshed"
        );
    }

    #[test]
    fn late_request_events_cannot_replace_another_selection()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::hosting::HostEvent;
        let (mut tab, _events, _host_jobs) = tab_showing_requests();
        let forge = tab.host.as_ref().ok_or("no host")?.forge.clone();
        let (host, host_events, jobs) = HostWorker::detached(forge);
        tab.host = Some(host);

        // Request 7 is still in flight when the user selects request 9.
        tab.select_request(9);
        assert!(matches!(jobs.try_recv(), Ok(HostJob::Details(9, _))));
        assert!(tab.requests.details.is_none());
        host_events.send(HostEvent::Details(
            7,
            0,
            Box::new(Err("obsolete request body".into())),
        ))?;
        // An old filter's list result must not alter the current request selection.
        host_events.send(HostEvent::List(
            StateFilter::Closed.into(),
            Err("obsolete filter failure".into()),
        ))?;
        tab.poll(&[]);
        assert_eq!(tab.requests.selected, Some(9));
        assert!(tab.requests.details.is_none());
        assert!(tab.requests.details_loading);
        assert!(matches!(tab.requests.list, Some(Ok(_))));

        host_events.send(HostEvent::Details(
            9,
            tab.requests.details_generation,
            Box::new(Err("current request failure".into())),
        ))?;
        tab.poll(&[]);
        assert!(!tab.requests.details_loading);
        assert!(matches!(
            &tab.requests.details,
            Some(Err(message)) if message == "current request failure"
        ));
        Ok(())
    }

    #[test]
    fn revisiting_a_request_discards_its_older_in_flight_response()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::hosting::HostEvent;
        let (mut tab, _events, _jobs) = tab_showing_requests();
        let forge = tab.host.as_ref().ok_or("no host")?.forge.clone();
        let (host, host_events, host_jobs) = HostWorker::detached(forge);
        tab.host = Some(host);

        tab.select_request(9);
        let first = tab.requests.details_generation;
        tab.select_request(7);
        tab.select_request(9);
        let latest = tab.requests.details_generation;
        assert_ne!(first, latest);
        assert_eq!(jobs_of(&host_jobs).len(), 3);

        // Same number, but this is an obsolete result from the first visit.
        host_events.send(HostEvent::Details(
            9,
            first,
            Box::new(Err("obsolete failure".into())),
        ))?;
        tab.poll(&[]);
        assert_eq!(tab.requests.selected, Some(9));
        assert!(tab.requests.details.is_none());
        assert!(tab.requests.details_loading);

        host_events.send(HostEvent::Details(
            9,
            latest,
            Box::new(Err("latest failure".into())),
        ))?;
        tab.poll(&[]);
        assert!(!tab.requests.details_loading);
        assert!(matches!(
            &tab.requests.details,
            Some(Err(message)) if message == "latest failure"
        ));
        Ok(())
    }

    #[test]
    fn a_quiet_list_failure_does_not_turn_the_details_reload_into_an_explicit_one()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut tab, _events, _jobs) = tab_showing_requests();
        let forge = tab.host.as_ref().ok_or("no host")?.forge.clone();
        let (host, host_events, _host_jobs) = HostWorker::detached(forge);
        tab.host = Some(host);
        let details = Arc::new(RequestDetails {
            summary: RequestSummary {
                number: 7,
                title: String::new(),
                author: String::new(),
                state: canopy_hosting::RequestState::Open,
                draft: false,
                source_branch: "feature".into(),
                target_branch: "main".into(),
                from_fork: false,
                created_at: 0,
                updated_at: 0,
                web_url: String::new(),
                labels: Vec::new(),
                description: String::new(),
                rollup: None,
            },
            body: String::new(),
            head_sha: None,
            mergeable: None,
            commits: None,
            changed_files: None,
            additions: None,
            deletions: None,
            reviews: Vec::new(),
            checks: Vec::new(),
            comments: Vec::new(),
        });
        tab.requests.details = Some(Ok(Arc::clone(&details)));
        tab.refresh_requests_quietly();
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Err("offline".into()),
        ))?;
        host_events.send(HostEvent::Details(
            7,
            tab.requests.details_generation,
            Box::new(Err("offline".into())),
        ))?;
        tab.poll(&[]);
        assert!(matches!(&tab.requests.details, Some(Ok(kept)) if Arc::ptr_eq(kept, &details)));
        assert!(!tab.requests.details_loading);

        // The next manual reload must still surface its failure.
        tab.load_requests();
        host_events.send(HostEvent::Details(
            7,
            tab.requests.details_generation,
            Box::new(Err("offline".into())),
        ))?;
        tab.poll(&[]);
        assert!(matches!(&tab.requests.details, Some(Err(_))));
        Ok(())
    }

    #[test]
    fn a_quiet_refresh_that_fails_keeps_the_list_on_screen()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::hosting::HostEvent;
        let (mut tab, events, host_jobs) = tab_showing_requests();
        let Some(host) = &tab.host else {
            return Err("no host".into());
        };
        let (host, host_events, _) = HostWorker::detached(host.forge.clone());
        tab.host = Some(host);
        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        drop(host_jobs);
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Err("could not reach github.com".into()),
        ))?;
        tab.poll(&[]);
        assert!(
            matches!(tab.requests.list, Some(Ok(_))),
            "a network blip does not replace the list with an error page"
        );
        // A reload the user asked for shows its failure.
        tab.load_requests();
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Err("could not reach github.com".into()),
        ))?;
        tab.poll(&[]);
        assert!(matches!(tab.requests.list, Some(Err(_))));
        Ok(())
    }

    #[test]
    fn later_pages_join_the_list_and_a_reload_starts_over() -> Result<(), Box<dyn std::error::Error>>
    {
        use crate::hosting::HostEvent;
        let (mut tab, events, host_jobs) = tab_showing_requests();
        let Some(host) = &tab.host else {
            return Err("no host".into());
        };
        let (host, host_events, jobs) = HostWorker::detached(host.forge.clone());
        tab.host = Some(host);
        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        drop(host_jobs);
        let numbers = |tab: &RepoTab| -> Vec<u64> {
            match &tab.requests.list {
                Some(Ok(list)) => list.iter().map(|r| r.number).collect(),
                _ => Vec::new(),
            }
        };
        tab.load_requests();
        host_events.send(HostEvent::More {
            key: StateFilter::Open.into(),
            more: true,
            cursor: None,
        })?;
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(3, 30), listed(2, 20)]),
        ))?;
        tab.poll(&[]);
        assert!(tab.requests.more);
        // The next page asks the host once, whatever the number of clicks.
        tab.load_more_requests();
        tab.load_more_requests();
        let asked = jobs
            .try_iter()
            .filter(|j| matches!(j, HostJob::ListMore(k, 2, _) if k.state == StateFilter::Open))
            .count();
        assert_eq!(asked, 1);
        host_events.send(HostEvent::Page {
            key: StateFilter::Open.into(),
            page: 2,
            result: Ok((vec![listed(1, 10)], false)),
            cursor: None,
        })?;
        tab.poll(&[]);
        assert_eq!(numbers(&tab), [3, 2, 1]);
        assert!(!tab.requests.more && !tab.requests.loading_more);
        // The page again with its checks replaces its row instead of adding one.
        host_events.send(HostEvent::Page {
            key: StateFilter::Open.into(),
            page: 2,
            result: Ok((vec![listed(1, 11)], false)),
            cursor: None,
        })?;
        // A timed refresh of the first page keeps the later pages.
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(4, 40), listed(3, 30)]),
        ))?;
        tab.poll(&[]);
        assert_eq!(numbers(&tab), [4, 3, 1]);
        // A reload the user asked for starts from the first page again.
        tab.load_requests();
        assert!(!tab.requests.more && tab.requests.pages == 1);
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(4, 40)]),
        ))?;
        tab.poll(&[]);
        assert_eq!(numbers(&tab), [4]);
        Ok(())
    }

    #[test]
    fn a_search_goes_to_the_host_once_the_list_is_longer_than_a_page()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::hosting::HostEvent;
        let (mut tab, events, host_jobs) = tab_showing_requests();
        let Some(host) = &tab.host else {
            return Err("no host".into());
        };
        let (host, host_events, jobs) = HostWorker::detached(host.forge.clone());
        tab.host = Some(host);
        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        drop(host_jobs);
        let queries = |jobs: &crossbeam_channel::Receiver<HostJob>| -> Vec<RequestQuery> {
            jobs.try_iter()
                .filter_map(|j| match j {
                    HostJob::List(key) => Some(key.query),
                    _ => None,
                })
                .collect()
        };
        tab.requests.cli = Some(Ok(crate::hosting::demo_account()));
        tab.load_requests();
        host_events.send(HostEvent::More {
            key: StateFilter::Open.into(),
            more: true,
            cursor: None,
        })?;
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(3, 30), listed(2, 20)]),
        ))?;
        tab.poll(&[]);
        assert!(
            queries(&jobs).iter().all(RequestQuery::is_empty),
            "the plain list first"
        );

        // Typing plans a search; it waits for the typing to pause and keeps the list shown.
        tab.request_view.text = "needle".into();
        tab.request_view_changed();
        assert!(tab.run_search_if_due().is_some(), "not before the pause");
        assert_eq!(queries(&jobs).len(), 0);
        tab.requests.search_due = Some(Instant::now());
        assert_eq!(tab.run_search_if_due(), None);
        let asked = queries(&jobs);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].text, "needle");
        assert!(matches!(tab.requests.list, Some(Ok(_))), "the list stays");

        // An answer for the plain list is out of date now; the search's own is taken.
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(3, 30)]),
        ))?;
        tab.poll(&[]);
        assert!(tab.requests.loading, "the old answer was dropped");
        let key = ListKey {
            state: StateFilter::Open,
            query: asked[0].clone(),
        };
        host_events.send(HostEvent::List(key.clone(), Ok(vec![listed(9, 90)])))?;
        tab.poll(&[]);
        let shown = tab.requests.shown.clone();
        assert_eq!(
            shown,
            Some(vec![9]),
            "the host's rows are not filtered again here"
        );
        // The search's rows are not the plain list: more of them is not "truncated".
        host_events.send(HostEvent::More {
            key,
            more: false,
            cursor: None,
        })?;
        tab.poll(&[]);
        assert_eq!(tab.requests.truncated, Some(StateFilter::Open));

        // Clearing the text goes back to the plain list.
        tab.request_view.text.clear();
        tab.request_view_changed();
        tab.requests.search_due = Some(Instant::now());
        assert_eq!(tab.run_search_if_due(), None);
        assert_eq!(queries(&jobs), [RequestQuery::default()]);
        Ok(())
    }

    #[test]
    fn a_github_search_without_a_token_stays_on_the_loaded_rows()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::hosting::HostEvent;
        let (mut tab, events, host_jobs) = tab_showing_requests();
        let Some(host) = &tab.host else {
            return Err("no host".into());
        };
        let (host, host_events, jobs) = HostWorker::detached(host.forge.clone());
        tab.host = Some(host);
        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        drop(host_jobs);
        tab.requests.cli = Some(crate::hosting::signed_out(ForgeKind::GitHub, "github.com"));
        tab.load_requests();
        host_events.send(HostEvent::More {
            key: StateFilter::Open.into(),
            more: true,
            cursor: None,
        })?;
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(3, 30), listed(2, 20)]),
        ))?;
        tab.poll(&[]);
        let before = jobs.try_iter().count();
        tab.request_view.text = "Request 2".into();
        tab.request_view_changed();
        assert_eq!(tab.requests.search_due, None, "nothing to ask the host");
        assert_eq!(
            jobs.try_iter().count(),
            0,
            "{before} jobs before, none after"
        );
        tab.requests.search_due = None;
        assert_eq!(
            tab.local_request_view().text,
            "Request 2",
            "the loaded rows are filtered here"
        );
        Ok(())
    }

    /// #197: a failed page stops the end of the list from asking again until the first page
    /// loads again.
    #[test]
    fn a_failed_page_waits_for_the_next_load_of_the_list() -> Result<(), Box<dyn std::error::Error>>
    {
        use crate::hosting::HostEvent;
        let (mut tab, events, host_jobs) = tab_showing_requests();
        let Some(host) = &tab.host else {
            return Err("no host".into());
        };
        let (host, host_events, _jobs) = HostWorker::detached(host.forge.clone());
        tab.host = Some(host);
        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        drop(host_jobs);
        tab.load_requests();
        host_events.send(HostEvent::More {
            key: StateFilter::Open.into(),
            more: true,
            cursor: None,
        })?;
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(3, 30)]),
        ))?;
        tab.poll(&[]);

        tab.load_more_requests();
        host_events.send(HostEvent::Page {
            key: StateFilter::Open.into(),
            page: 2,
            result: Err("Bad Gateway".into()),
            cursor: None,
        })?;
        tab.poll(&[]);
        assert!(tab.requests.more_failed && !tab.requests.loading_more);
        assert!(tab.requests.more, "the host still holds more");

        // The timed refresh of the first page lets the end of the list ask again.
        tab.refresh_requests_quietly();
        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(3, 30)]),
        ))?;
        tab.poll(&[]);
        assert!(!tab.requests.more_failed);
        Ok(())
    }

    fn listed(number: u64, updated_at: i64) -> RequestSummary {
        RequestSummary {
            number,
            title: format!("Request {number}"),
            author: "sam".into(),
            state: canopy_hosting::RequestState::Open,
            draft: false,
            source_branch: format!("topic-{number}"),
            target_branch: "main".into(),
            from_fork: false,
            created_at: 0,
            updated_at,
            web_url: String::new(),
            labels: Vec::new(),
            description: String::new(),
            rollup: None,
        }
    }

    #[test]
    fn the_arrow_keys_select_nothing_in_a_list_that_has_become_empty()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::hosting::HostEvent;
        let (mut tab, _events, host_jobs) = tab_showing_requests();
        let forge = tab.host.as_ref().ok_or("no host")?.forge.clone();
        let (host, host_events, _) = HostWorker::detached(forge);
        tab.host = Some(host);
        // A drawn frame of the open list left its order behind.
        tab.requests.list = Some(Ok(Arc::from(vec![listed(2, 20), listed(1, 10)])));
        tab.requests.shown = Some(vec![2, 1]);
        tab.requests.selected = None;

        host_events.send(HostEvent::List(StateFilter::Open.into(), Ok(Vec::new())))?;
        tab.poll(&[]);
        tab.move_request(1);
        assert_eq!(
            tab.requests.selected, None,
            "no request is listed to select"
        );
        assert!(
            !jobs_of(&host_jobs)
                .iter()
                .any(|j| matches!(j, HostJob::Details(..))),
            "nothing was opened"
        );
        Ok(())
    }

    #[test]
    fn a_list_that_arrives_opens_on_the_row_the_list_shows_first()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::hosting::HostEvent;
        let (mut tab, _events, _jobs) = tab_showing_requests();
        let forge = tab.host.as_ref().ok_or("no host")?.forge.clone();
        let (host, host_events, _) = HostWorker::detached(forge);
        tab.host = Some(host);
        tab.requests.selected = None;
        tab.request_view.checks = Some(crate::request_view::ChecksChoice::Failed);

        // Request 1's checks were seen when it was opened; the list itself says nothing of them,
        // so the row shows the remembered failure and the filter keeps it.
        let seen = RequestDetails {
            summary: listed(1, 10),
            body: String::new(),
            head_sha: None,
            mergeable: None,
            commits: None,
            changed_files: None,
            additions: None,
            deletions: None,
            reviews: Vec::new(),
            checks: vec![canopy_hosting::Check {
                name: "build".into(),
                state: canopy_hosting::CheckState::Failure,
                url: None,
            }],
            comments: Vec::new(),
        };
        let ctx = tab.host.as_ref().ok_or("no host")?.context().clone();
        crate::views::requests::remember_signals(&ctx, &tab, &seen);

        host_events.send(HostEvent::List(
            StateFilter::Open.into(),
            Ok(vec![listed(2, 20), listed(1, 10)]),
        ))?;
        tab.poll(&[]);
        assert_eq!(tab.requests.selected, Some(1));
        Ok(())
    }

    #[test]
    fn opening_the_requests_view_refreshes_a_list_gone_stale() {
        let (mut tab, _events, host_jobs) = tab_showing_requests();
        tab.refresh_requests_if_stale();
        assert!(jobs_of(&host_jobs).is_empty(), "a fresh list is left alone");
        let long_ago = Instant::now()
            .checked_sub(REQUESTS_STALE_AFTER * 2)
            .unwrap_or_else(Instant::now);
        tab.requests.loaded_at = Some(long_ago);
        tab.refresh_requests_if_stale();
        assert!(
            jobs_of(&host_jobs)
                .iter()
                .any(|j| matches!(j, HostJob::List(_)))
        );
        assert!(!tab.requests.loading);
    }

    fn gate() -> RefreshGate {
        RefreshGate {
            view: View::Requests,
            focused: true,
            hosting: true,
            token: true,
            loaded: true,
            busy: false,
        }
    }

    #[test]
    fn the_timed_refresh_falls_due_once_the_interval_has_passed() {
        let last = Instant::now();
        let every = Duration::from_secs(60);
        let step = |after: u64| {
            request_refresh_step(last + Duration::from_secs(after), last, every, 0, gate())
        };
        assert_eq!(step(0), RefreshStep::Wait(every));
        assert_eq!(step(45), RefreshStep::Wait(Duration::from_secs(15)));
        assert_eq!(step(60), RefreshStep::Due);
        assert_eq!(step(500), RefreshStep::Due);
    }

    #[test]
    fn the_timed_refresh_waits_for_every_condition() {
        let last = Instant::now();
        let every = Duration::from_secs(60);
        let now = last + every * 2;
        let step = |g: RefreshGate| request_refresh_step(now, last, every, 0, g);
        assert_eq!(step(gate()), RefreshStep::Due);
        let cases = [
            (
                "another view",
                RefreshGate {
                    view: View::History,
                    ..gate()
                },
            ),
            (
                "the changes view",
                RefreshGate {
                    view: View::Changes,
                    ..gate()
                },
            ),
            (
                "an unfocused window",
                RefreshGate {
                    focused: false,
                    ..gate()
                },
            ),
            (
                "hosting off",
                RefreshGate {
                    hosting: false,
                    ..gate()
                },
            ),
            (
                "no sign-in (the anonymous rate limit is shared with Reload and actions)",
                RefreshGate {
                    token: false,
                    ..gate()
                },
            ),
            (
                "no list loaded",
                RefreshGate {
                    loaded: false,
                    ..gate()
                },
            ),
            (
                "a load under way",
                RefreshGate {
                    busy: true,
                    ..gate()
                },
            ),
        ];
        for (why, g) in cases {
            assert_eq!(step(g), RefreshStep::Paused, "{why}");
        }
    }

    #[test]
    fn failures_double_the_wait_up_to_the_cap_and_never_below_the_interval() {
        let last = Instant::now();
        let every = Duration::from_secs(60);
        let wait = |failures: u32, interval: Duration| match request_refresh_step(
            last,
            last,
            interval,
            failures,
            gate(),
        ) {
            RefreshStep::Wait(d) => d,
            other => panic!("expected a wait, got {other:?}"),
        };
        assert_eq!(wait(0, every), every);
        assert_eq!(wait(1, every), every * 2);
        assert_eq!(wait(3, every), every * 8);
        assert_eq!(wait(4, every), REQUESTS_REFRESH_BACKOFF_CAP);
        assert_eq!(wait(u32::MAX, every), REQUESTS_REFRESH_BACKOFF_CAP);
        let hourly = Duration::from_secs(3600);
        assert_eq!(wait(0, hourly), hourly);
        assert_eq!(
            wait(5, hourly),
            hourly,
            "the cap never shortens the interval"
        );
    }

    /// A tab on the Requests view with a loaded list that was loaded at `loaded`.
    fn timed_tab(
        loaded: Instant,
    ) -> (
        RepoTab,
        crossbeam_channel::Sender<HostEvent>,
        crossbeam_channel::Receiver<HostJob>,
    ) {
        let (mut tab, _events, _jobs) = tab_showing_requests();
        let Some(host) = &tab.host else {
            unreachable!("tab_showing_requests gives the tab a host")
        };
        let (host, host_events, host_jobs) = HostWorker::detached(host.forge.clone());
        tab.host = Some(host);
        tab.view = View::Requests;
        tab.requests.cli = Some(Ok(crate::hosting::demo_account()));
        tab.requests.loaded_at = Some(loaded);
        (tab, host_events, host_jobs)
    }

    fn lists_in(host_jobs: &crossbeam_channel::Receiver<HostJob>) -> usize {
        jobs_of(host_jobs)
            .iter()
            .filter(|j| matches!(j, HostJob::List(_)))
            .count()
    }

    #[test]
    fn the_open_requests_view_refreshes_quietly_when_the_timer_is_due()
    -> Result<(), Box<dyn std::error::Error>> {
        let start = Instant::now();
        let every = Duration::from_secs(60);
        let (mut tab, host_events, host_jobs) = timed_tab(start);

        assert_eq!(
            tab.refresh_requests_on_timer(start + Duration::from_secs(10), every, true),
            Some(Duration::from_secs(50))
        );
        assert_eq!(lists_in(&host_jobs), 0, "not due yet");

        let due = start + every;
        assert_eq!(tab.refresh_requests_on_timer(due, every, true), None);
        assert_eq!(lists_in(&host_jobs), 1);
        assert!(!tab.requests.loading, "a timed refresh shows no spinner");
        assert!(tab.requests.quiet_refresh);

        // Under way: it does not ask again however long the answer takes.
        assert_eq!(
            tab.refresh_requests_on_timer(due + every * 3, every, true),
            None
        );
        assert_eq!(lists_in(&host_jobs), 0, "one refresh in flight at a time");

        // The answer restarts the wait from when it arrived.
        host_events.send(HostEvent::List(StateFilter::Open.into(), Ok(Vec::new())))?;
        tab.poll(&[]);
        assert!(!tab.requests.refresh.in_flight);
        let Some(loaded) = tab.requests.loaded_at else {
            unreachable!("a list answer sets loaded_at")
        };
        assert_eq!(
            tab.refresh_requests_on_timer(loaded + Duration::from_secs(1), every, true),
            Some(Duration::from_secs(59))
        );
        Ok(())
    }

    #[test]
    fn the_timer_stays_quiet_while_a_condition_fails() {
        let start = Instant::now();
        let every = Duration::from_secs(60);
        let late = start + every * 2;

        let (mut tab, _events, host_jobs) = timed_tab(start);
        assert_eq!(tab.refresh_requests_on_timer(late, every, false), None);
        assert_eq!(lists_in(&host_jobs), 0, "an unfocused window");

        tab.view = View::History;
        assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
        assert_eq!(lists_in(&host_jobs), 0, "another view");

        tab.view = View::Requests;
        tab.requests.loading = true;
        assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
        assert_eq!(lists_in(&host_jobs), 0, "a load under way");

        tab.requests.loading = false;
        tab.requests.list = None;
        assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
        assert_eq!(lists_in(&host_jobs), 0, "no list loaded");

        tab.requests.list = Some(Err("Not Found".into()));
        assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
        assert_eq!(
            lists_in(&host_jobs),
            0,
            "a list that failed is not refreshed"
        );

        tab.requests.list = Some(Ok(Arc::from(Vec::new())));
        tab.host = None;
        assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
        assert_eq!(lists_in(&host_jobs), 0, "hosting off");
    }

    #[test]
    fn the_timer_never_runs_without_a_sign_in() {
        let start = Instant::now();
        let every = Duration::from_secs(60);
        let late = start + every * 2;
        let (mut tab, _events, host_jobs) = timed_tab(start);
        for (why, cli) in [
            ("the host has not reported yet", None),
            (
                "the CLI is signed out",
                Some(crate::hosting::signed_out(ForgeKind::GitHub, "github.com")),
            ),
        ] {
            tab.requests.cli = cli;
            assert_eq!(
                tab.refresh_requests_on_timer(late, every, true),
                None,
                "{why}"
            );
            assert_eq!(lists_in(&host_jobs), 0, "{why}");
        }
        tab.requests.cli = Some(Ok(crate::hosting::demo_account()));
        assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
        assert_eq!(lists_in(&host_jobs), 1, "a sign-in turns the timer on");
    }

    #[test]
    fn the_timer_waits_while_a_sync_runs() {
        let start = Instant::now();
        let every = Duration::from_secs(60);
        let late = start + every * 2;
        for kind in [OpKind::Fetch, OpKind::Pull, OpKind::Push] {
            let (mut tab, _events, host_jobs) = timed_tab(start);
            tab.running_kind = Some(kind.clone());
            assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
            assert_eq!(lists_in(&host_jobs), 0, "{kind:?} is running");
            tab.running_kind = None;
            assert_eq!(tab.refresh_requests_on_timer(late, every, true), None);
            assert_eq!(lists_in(&host_jobs), 1, "{kind:?} is over");
        }
    }

    #[test]
    fn a_sync_finishing_during_a_timed_refresh_does_not_refresh_twice() {
        let (mut tab, events, host_jobs) = tab_showing_requests();
        tab.view = View::Requests;
        tab.requests.cli = Some(Ok(crate::hosting::demo_account()));
        let every = Duration::from_secs(60);
        let due = Instant::now() + every;
        assert_eq!(tab.refresh_requests_on_timer(due, every, true), None);
        assert!(tab.requests.refresh.in_flight);

        finish(&events, OpKind::Fetch, Ok(String::new()));
        tab.poll(&[]);
        let jobs = jobs_of(&host_jobs);
        let lists = jobs
            .iter()
            .filter(|j| matches!(j, HostJob::List(_)))
            .count();
        let details = jobs
            .iter()
            .filter(|j| matches!(j, HostJob::Details(..)))
            .count();
        assert_eq!((lists, details), (1, 1), "{jobs:?}");
    }

    #[test]
    fn a_successful_sync_respects_the_request_refresh_backoff() {
        for held in [false, true] {
            let (mut tab, events, _host_jobs) = tab_showing_requests();
            let forge = tab.host.as_ref().expect("host").forge.clone();
            let (host, host_events, host_jobs) = HostWorker::detached(forge);
            tab.host = Some(host);
            tab.requests.refresh.interval = Duration::from_mins(5);
            tab.requests.refresh.failures = u32::from(!held);
            tab.requests.refresh.failed_at = (!held).then(|| {
                Instant::now()
                    .checked_sub(Duration::from_mins(5))
                    .expect("earlier instant")
            });
            if held {
                tab.requests.refresh.held_until = Some(Instant::now() + Duration::from_mins(30));
            }
            finish(&events, OpKind::Fetch, Ok(String::new()));
            tab.poll(&[]);
            assert!(
                jobs_of(&host_jobs).is_empty(),
                "sync must respect the API backoff (held={held})"
            );
            tab.load_requests();
            assert_eq!(
                lists_in(&host_jobs),
                1,
                "explicit Reload is still available"
            );
            tab.requests.refresh.failed_at = Some(Instant::now());
            host_events
                .send(HostEvent::List(StateFilter::Open.into(), Ok(Vec::new())))
                .expect("reload");
            tab.poll(&[]);
            finish(&events, OpKind::Fetch, Ok(String::new()));
            tab.poll(&[]);
            assert_eq!(
                lists_in(&host_jobs),
                1,
                "success clears the backoff immediately"
            );
        }
    }

    #[test]
    fn a_rate_limit_holds_the_timer_until_it_lifts_or_for_the_longest_backoff()
    -> Result<(), Box<dyn std::error::Error>> {
        let every = Duration::from_secs(60);
        let minutes = |m: u64| Duration::from_secs(m * 60);
        // (what the host said, wait before the next attempt in minutes)
        let now = chrono::Utc::now().timestamp();
        let cases = [
            ("a reset half an hour ahead", Some(now + 1_800), 30),
            ("no reset", None, 15),
            ("a reset already past", Some(now - 5), 15),
            ("a reset a day ahead", Some(now + 86_400), 60),
        ];
        for (why, reset, wait) in cases {
            let start = Instant::now();
            let (mut tab, host_events, host_jobs) = timed_tab(start);
            assert_eq!(
                tab.refresh_requests_on_timer(start + every, every, true),
                None
            );
            assert_eq!(lists_in(&host_jobs), 1, "{why}");
            host_events.send(HostEvent::RateLimited { reset })?;
            host_events.send(HostEvent::List(
                StateFilter::Open.into(),
                Err("rate limit exceeded".into()),
            ))?;
            tab.poll(&[]);
            let Some(failed_at) = tab.requests.refresh.failed_at else {
                return Err(format!("{why}: the failure was not recorded").into());
            };
            let early = failed_at + minutes(wait).saturating_sub(every);
            assert!(
                tab.refresh_requests_on_timer(early, every, true).is_some(),
                "{why}: held while the limit stands"
            );
            assert_eq!(lists_in(&host_jobs), 0, "{why}: no probe before it lifts");
            let after = failed_at + minutes(wait) + every;
            assert_eq!(
                tab.refresh_requests_on_timer(after, every, true),
                None,
                "{why}"
            );
            assert_eq!(lists_in(&host_jobs), 1, "{why}: asks again once it lifted");
        }
        Ok(())
    }

    #[test]
    fn switching_to_the_requests_view_respects_the_backoff() {
        let (mut tab, _events, host_jobs) = tab_showing_requests();
        let ago = |secs: u64| {
            Instant::now()
                .checked_sub(Duration::from_secs(secs))
                .unwrap_or_else(Instant::now)
        };
        tab.requests.loaded_at = Some(ago(600));
        // A refresh failed 10 s ago: the list is stale, but the host is not asked again yet.
        tab.requests.refresh.failures = 1;
        tab.requests.refresh.failed_at = Some(ago(10));
        tab.refresh_requests_if_stale();
        assert!(
            jobs_of(&host_jobs).is_empty(),
            "no resend before the backoff elapsed"
        );
        tab.requests.refresh.failures = 2;
        tab.requests.refresh.failed_at = Some(ago(150));
        tab.refresh_requests_if_stale();
        assert!(
            jobs_of(&host_jobs).is_empty(),
            "two failures wait four times the stale age"
        );
        tab.requests.refresh.failed_at = Some(ago(250));
        tab.refresh_requests_if_stale();
        assert_eq!(lists_in(&host_jobs), 1, "the backoff elapsed");
    }

    #[test]
    fn switching_to_the_requests_view_leaves_a_rate_limited_host_alone() {
        let (mut tab, _events, host_jobs) = tab_showing_requests();
        let long_ago = Instant::now()
            .checked_sub(REQUESTS_STALE_AFTER * 5)
            .unwrap_or_else(Instant::now);
        tab.requests.loaded_at = Some(long_ago);
        tab.requests.refresh.held_until = Some(Instant::now() + Duration::from_secs(600));
        tab.refresh_requests_if_stale();
        assert!(jobs_of(&host_jobs).is_empty());
    }

    #[test]
    fn a_failed_timed_refresh_backs_off_and_a_success_resets_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let start = Instant::now();
        let every = Duration::from_secs(60);
        let (mut tab, host_events, host_jobs) = timed_tab(start);
        let fail = |tab: &mut RepoTab| -> Result<(), Box<dyn std::error::Error>> {
            host_events.send(HostEvent::List(
                StateFilter::Open.into(),
                Err("rate limited".into()),
            ))?;
            tab.poll(&[]);
            Ok(())
        };

        assert_eq!(
            tab.refresh_requests_on_timer(start + every, every, true),
            None
        );
        assert_eq!(lists_in(&host_jobs), 1);
        fail(&mut tab)?;
        assert!(
            matches!(tab.requests.list, Some(Ok(_))),
            "the shown list stays"
        );
        assert_eq!(tab.requests.refresh.failures, 1);
        assert!(!tab.requests.refresh.in_flight);
        let Some(failed_at) = tab.requests.refresh.failed_at else {
            unreachable!("a failure records when it happened")
        };

        // One failure: the next attempt waits twice as long, counted from the failure.
        assert_eq!(
            tab.refresh_requests_on_timer(failed_at + every, every, true),
            Some(every)
        );
        assert_eq!(
            tab.refresh_requests_on_timer(failed_at + every * 2, every, true),
            None
        );
        assert_eq!(lists_in(&host_jobs), 1);
        fail(&mut tab)?;
        assert_eq!(tab.requests.refresh.failures, 2);

        // A success clears the backoff.
        tab.requests.refresh.failed_at = Some(start);
        assert_eq!(
            tab.refresh_requests_on_timer(start + every * 4, every, true),
            None
        );
        assert_eq!(lists_in(&host_jobs), 1);
        host_events.send(HostEvent::List(StateFilter::Open.into(), Ok(Vec::new())))?;
        tab.poll(&[]);
        assert_eq!(tab.requests.refresh.failures, 0);
        Ok(())
    }

    fn details_of(number: u64) -> RequestDetails {
        RequestDetails {
            summary: RequestSummary {
                number,
                title: format!("request {number}"),
                author: String::new(),
                state: canopy_hosting::RequestState::Open,
                draft: false,
                source_branch: "feature".into(),
                target_branch: "main".into(),
                from_fork: false,
                created_at: 0,
                updated_at: 0,
                web_url: String::new(),
                labels: Vec::new(),
                description: String::new(),
                rollup: None,
            },
            body: String::new(),
            head_sha: None,
            mergeable: None,
            commits: None,
            changed_files: None,
            additions: None,
            deletions: None,
            reviews: Vec::new(),
            checks: Vec::new(),
            comments: Vec::new(),
        }
    }

    /// A tab on a fresh hosting thread, nothing selected: the channels to feed it and read it.
    fn tab_on_a_fresh_host() -> (
        RepoTab,
        crossbeam_channel::Sender<HostEvent>,
        crossbeam_channel::Receiver<HostJob>,
    ) {
        let (mut tab, _events, _jobs) = tab_showing_requests();
        let forge = tab.host.as_ref().expect("host").forge.clone();
        let (host, host_events, host_jobs) = HostWorker::detached(forge);
        tab.host = Some(host);
        tab.requests.selected = None;
        tab.requests.details = None;
        (tab, host_events, host_jobs)
    }

    /// Selects `number` and answers its details load, as the hosting thread would.
    fn open_request(
        tab: &mut RepoTab,
        host_events: &crossbeam_channel::Sender<HostEvent>,
        host_jobs: &crossbeam_channel::Receiver<HostJob>,
        number: u64,
    ) {
        tab.select_request(number);
        let jobs = jobs_of(host_jobs);
        assert!(
            matches!(jobs.as_slice(), [HostJob::Details(n, _)] if *n == number),
            "opening request {number} loads it once: {jobs:?}"
        );
        host_events
            .send(HostEvent::Details(
                number,
                tab.requests.details_generation,
                Box::new(Ok(details_of(number))),
            ))
            .expect("details");
        tab.poll(&[]);
        assert!(matches!(&tab.requests.details, Some(Ok(d)) if d.summary.number == number));
    }

    /// #29: A, then B, then A again within the cache's lifetime shows A at once, without
    /// asking the host for it a second time.
    #[test]
    fn returning_to_a_request_shows_its_cached_details_without_a_new_load() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        open_request(&mut tab, &host_events, &host_jobs, 9);
        open_request(&mut tab, &host_events, &host_jobs, 4);
        let before = tab.requests.details_generation;

        tab.select_request(9);
        assert!(jobs_of(&host_jobs).is_empty(), "A is not loaded again");
        assert_eq!(tab.requests.selected, Some(9));
        assert!(!tab.requests.details_loading);
        assert!(matches!(&tab.requests.details, Some(Ok(d)) if d.summary.number == 9));
        assert_ne!(
            tab.requests.details_generation, before,
            "an in-flight result of the earlier selection must not land"
        );

        // B's late result (it was answered once; a duplicate or old one) changes nothing.
        host_events
            .send(HostEvent::Details(4, before, Box::new(Err("late".into()))))
            .expect("late");
        tab.poll(&[]);
        assert!(matches!(&tab.requests.details, Some(Ok(d)) if d.summary.number == 9));
    }

    #[test]
    fn a_failed_load_is_not_cached() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        tab.select_request(9);
        host_events
            .send(HostEvent::Details(
                9,
                tab.requests.details_generation,
                Box::new(Err("offline".into())),
            ))
            .expect("failure");
        tab.poll(&[]);
        drop(jobs_of(&host_jobs));
        tab.select_request(4);
        drop(jobs_of(&host_jobs));
        tab.select_request(9);
        assert!(matches!(
            jobs_of(&host_jobs).as_slice(),
            [HostJob::Details(9, _)]
        ));
    }

    #[test]
    fn an_expired_cache_entry_is_loaded_again() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        open_request(&mut tab, &host_events, &host_jobs, 9);
        open_request(&mut tab, &host_events, &host_jobs, 4);
        let long_ago = Instant::now()
            .checked_sub(REQUEST_DETAILS_CACHE_TTL * 2)
            .unwrap_or_else(Instant::now);
        for entry in &mut tab.requests.details_cache {
            entry.loaded_at = long_ago;
        }
        tab.select_request(9);
        assert!(matches!(
            jobs_of(&host_jobs).as_slice(),
            [HostJob::Details(9, _)]
        ));
        assert!(tab.requests.details_loading);
    }

    #[test]
    fn a_manual_reload_drops_the_cached_details() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        open_request(&mut tab, &host_events, &host_jobs, 9);
        open_request(&mut tab, &host_events, &host_jobs, 4);
        tab.load_requests();
        drop(jobs_of(&host_jobs));
        tab.select_request(9);
        assert!(matches!(
            jobs_of(&host_jobs).as_slice(),
            [HostJob::Details(9, _)]
        ));
    }

    #[test]
    fn an_automatic_list_refresh_drops_the_cached_details() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        open_request(&mut tab, &host_events, &host_jobs, 9);
        open_request(&mut tab, &host_events, &host_jobs, 4);
        tab.refresh_requests_quietly();
        drop(jobs_of(&host_jobs));
        tab.select_request(9);
        assert!(matches!(
            jobs_of(&host_jobs).as_slice(),
            [HostJob::Details(9, _)]
        ));
    }

    #[test]
    fn an_action_result_drops_the_cached_details() {
        for result in [Ok(()), Err("refused".to_owned())] {
            let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
            open_request(&mut tab, &host_events, &host_jobs, 9);
            open_request(&mut tab, &host_events, &host_jobs, 4);
            host_events
                .send(HostEvent::Done {
                    label: "Merging".into(),
                    action: HostAction::Merge,
                    number: Some(4),
                    result,
                })
                .expect("done");
            tab.poll(&[]);
            drop(jobs_of(&host_jobs));
            tab.select_request(9);
            assert!(matches!(
                jobs_of(&host_jobs).as_slice(),
                [HostJob::Details(9, _)]
            ));
        }
    }

    #[test]
    fn the_same_number_in_another_project_is_not_a_cache_hit() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        open_request(&mut tab, &host_events, &host_jobs, 9);
        let mut other = tab.host.as_ref().expect("host").forge.clone();
        other.path = "o/other".into();
        let (host, _events, other_jobs) = HostWorker::detached(other);
        tab.host = Some(host);
        tab.select_request(4);
        tab.select_request(9);
        let jobs = jobs_of(&other_jobs);
        assert!(
            matches!(
                jobs.as_slice(),
                [HostJob::Details(4, _), HostJob::Details(9, _)]
            ),
            "{jobs:?}"
        );
    }

    #[test]
    fn restarting_the_host_for_a_new_token_forgets_the_cached_details() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        open_request(&mut tab, &host_events, &host_jobs, 9);
        assert!(!tab.requests.details_cache.is_empty());
        tab.restart_host(&egui::Context::default(), false);
        assert!(tab.requests.details_cache.is_empty());
    }

    #[test]
    fn the_cache_is_bounded_and_drops_the_oldest_first() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        let first = 1000;
        let last = first + REQUEST_DETAILS_CACHE_MAX as u64;
        for number in first..=last {
            open_request(&mut tab, &host_events, &host_jobs, number);
        }
        assert_eq!(tab.requests.details_cache.len(), REQUEST_DETAILS_CACHE_MAX);
        tab.select_request(first);
        assert!(
            matches!(jobs_of(&host_jobs).as_slice(), [HostJob::Details(n, _)] if *n == first),
            "the oldest entry was dropped"
        );
        tab.select_request(last);
        assert!(jobs_of(&host_jobs).is_empty(), "the newest is still cached");
    }

    #[test]
    fn a_cached_return_restores_that_requests_own_comment_draft() {
        let (mut tab, host_events, host_jobs) = tab_on_a_fresh_host();
        open_request(&mut tab, &host_events, &host_jobs, 9);
        tab.requests.comment = "draft for nine".into();
        open_request(&mut tab, &host_events, &host_jobs, 4);
        assert_eq!(tab.requests.comment, "");
        tab.requests.comment = "draft for four".into();
        tab.select_request(9);
        assert_eq!(tab.requests.comment, "draft for nine");
        tab.select_request(4);
        assert_eq!(tab.requests.comment, "draft for four");
    }
}

/// Frame-cost check for huge repositories: renders the real views headlessly over synthetic
/// data (50k commits, 20k tags, a 50k-line diff, a resolver with 400 conflicts) while scrolling,
/// and prints the mean CPU time per frame. Not a pass/fail test; run it in release:
/// `cargo test --release -p canopy-app frame_cost -- --ignored --nocapture`.
#[cfg(test)]
mod perf {
    use super::*;
    use canopy_core::diff::{DiffLine, Hunk};
    use canopy_core::{
        BranchInfo, ChangeKind, ConflictFile, GraphRow, LineEnd, LineKind, RemoteBranchInfo,
        RemoteInfo, Signature, TagInfo, WorkingStatus,
    };
    use std::fmt::Write as _;

    fn oid(i: usize) -> Oid {
        let mut b = [0u8; 20];
        b[..8].copy_from_slice(&(i as u64 + 1).to_be_bytes());
        Oid::from_bytes(&b).expect("oid")
    }

    fn sig(i: usize) -> Signature {
        Signature {
            name: format!("Author {}", i % 37),
            email: format!("a{}@example.com", i % 37),
            time: 1_700_000_000 - i64::try_from(i).expect("a small index") * 600,
            offset_minutes: 0,
        }
    }

    fn entry(path: String) -> FileEntry {
        FileEntry {
            path,
            old_path: None,
            kind: ChangeKind::Modified,
            additions: 3,
            deletions: 1,
            lfs: false,
            old_mode: None,
            new_mode: None,
        }
    }

    fn snapshot() -> RepoSnapshot {
        let n = 50_000;
        let seg = |from, to| canopy_core::graph::Segment {
            from_lane: 0,
            from,
            to_lane: 0,
            to,
            color: 0,
        };
        RepoSnapshot {
            name: "huge".into(),
            head: canopy_core::HeadInfo {
                branch: Some("main".into()),
                oid: Some(oid(0)),
                detached: false,
            },
            branches: (0..2_000)
                .map(|i| BranchInfo {
                    name: if i == 0 {
                        "main".into()
                    } else {
                        format!("feature/topic-{i}")
                    },
                    target: Some(oid(i)),
                    is_head: i == 0,
                    upstream: Some(format!("origin/b{i}")),
                    ahead: i % 3,
                    behind: i % 2,
                    worktree: None,
                })
                .collect(),
            remote_branches: (0..5_000)
                .map(|i| RemoteBranchInfo {
                    remote: "origin".into(),
                    branch: format!("b{i}"),
                    target: Some(oid(i)),
                })
                .collect(),
            remotes: vec![RemoteInfo {
                name: "origin".into(),
                url: Some("https://github.com/example/huge.git".into()),
                default_branch: Some("main".into()),
            }],
            tags: (0..20_000)
                .map(|i| TagInfo {
                    name: format!("v{}.{}.{}", i / 100, i / 10 % 10, i % 10),
                    target: Some(oid(i)),
                    message: None,
                })
                .collect(),
            status: WorkingStatus {
                unstaged: std::iter::once(entry("src/big.rs".into()))
                    .chain((0..5_000).map(|i| entry(format!("gen/file_{i}.rs"))))
                    .collect(),
                ..WorkingStatus::default()
            },
            commits: (0..n)
                .map(|i| canopy_core::CommitSummary {
                    oid: oid(i),
                    summary: format!("Commit number {i}: adjust the frobnicator"),
                    author: sig(i),
                    parents: vec![oid(i + 1)],
                })
                .collect(),
            graph: (0..n)
                .map(|_| GraphRow {
                    lane: 0,
                    color: 0,
                    segments: vec![
                        seg(LineEnd::Top, LineEnd::Mid),
                        seg(LineEnd::Mid, LineEnd::Bottom),
                    ],
                    width: 1,
                    is_merge: false,
                })
                .collect(),
            ..RepoSnapshot::default()
        }
    }

    fn big_diff() -> FileDiff {
        let mut hunks = Vec::new();
        let mut no = 1u32;
        for h in 0..500 {
            let lines = (0..100)
                .map(|l| {
                    let kind = match l % 5 {
                        1 => LineKind::Removed,
                        2 | 3 => LineKind::Added,
                        _ => LineKind::Context,
                    };
                    no += 1;
                    DiffLine {
                        kind,
                        old_no: (kind != LineKind::Added).then_some(no),
                        new_no: (kind != LineKind::Removed).then_some(no),
                        text: format!("    let value_{h}_{l} = compute(\"{l}\", {h}); // step {l}"),
                    }
                })
                .collect();
            hunks.push(Hunk {
                header: format!("@@ -{no},100 +{no},100 @@ fn hunk_{h}()"),
                old_start: no,
                old_lines: 100,
                new_start: no,
                new_lines: 100,
                lines,
            });
        }
        FileDiff {
            path: "src/big.rs".into(),
            hunks,
            additions: 20_000,
            deletions: 10_000,
            ..FileDiff::default()
        }
    }

    fn conflict_text() -> String {
        let mut s = String::new();
        for c in 0..400 {
            for l in 0..20 {
                let _ = writeln!(s, "common line {c}.{l}");
            }
            s.push_str("<<<<<<< HEAD\n");
            for l in 0..8 {
                let _ = writeln!(s, "ours {c}.{l}");
            }
            s.push_str("=======\n");
            for l in 0..8 {
                let _ = writeln!(s, "theirs {c}.{l}");
            }
            s.push_str(">>>>>>> feature\n");
        }
        s
    }

    /// Mean milliseconds per frame over `frames` frames, scrolling at `pointer`.
    #[expect(clippy::cast_precision_loss, reason = "a frame count far below 2^52")]
    fn measure(
        ctx: &egui::Context,
        frames: usize,
        pointer: egui::Pos2,
        mut frame: impl FnMut(&mut egui::Ui),
    ) -> f64 {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
        let mut t = 0.0;
        let mut run = |events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(t),
                predicted_dt: 1.0 / 60.0,
                events,
                ..Default::default()
            };
            ctx.run_ui(input, &mut frame).drop_without_applying_deltas();
        };
        for _ in 0..5 {
            run(vec![egui::Event::PointerMoved(pointer)]);
        }
        let started = Instant::now();
        for _ in 0..frames {
            run(vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -120.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        started.elapsed().as_secs_f64() * 1000.0 / frames as f64
    }

    fn layout(ui: &mut egui::Ui, tab: &mut RepoTab, settings: &crate::settings::Settings) {
        let mut actions = Vec::new();
        egui::Panel::left("sidebar")
            .default_size(250.0)
            .show(ui, |ui| crate::views::sidebar::show(ui, tab, &mut actions));
        egui::CentralPanel::default().show(ui, |ui| match tab.view {
            View::History => {
                crate::views::history::show(ui, tab, settings, &mut actions);
            }
            _ => {
                crate::views::changes::show(ui, tab, &mut actions);
            }
        });
    }

    #[test]
    #[ignore = "timing harness; run in release with --ignored --nocapture"]
    fn frame_cost() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        crate::ui::theme::apply(
            &ctx,
            crate::ui::theme::ThemeId::Midnight,
            crate::ui::theme::Appearance::STILL,
        );
        let settings = crate::settings::Settings::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let mut tab = RepoTab::new(dir.path().to_path_buf(), &ctx, 2000, DiffPrefs::default());
        let snap = snapshot();
        let file = snap.status.unstaged[0].clone();
        tab.snap = Some(snap.into());
        let diff = big_diff();
        let frames = 120;

        // History: the graph table, sidebar and a commit with 3000 files and the big diff.
        tab.view = View::History;
        tab.selected_commit = Some(oid(0));
        tab.details = Some(
            CommitDetails {
                oid: oid(0),
                message: "Big commit".into(),
                author: sig(0),
                committer: sig(0),
                parents: vec![oid(1)],
                files: std::iter::once(file.clone())
                    .chain((0..3_000).map(|i| entry(format!("dir/f{i}.c"))))
                    .collect(),
                stats_known: true,
            }
            .into(),
        );
        tab.file = Some(FileSelection {
            source: DiffSource::Commit(oid(0)),
            entry: file.clone(),
            conflicted: false,
        });
        tab.diff = Some(diff.into());
        let history = measure(&ctx, frames, egui::pos2(900.0, 200.0), |ui| {
            layout(ui, &mut tab, &settings);
        });
        let history_diff = measure(&ctx, frames, egui::pos2(1100.0, 700.0), |ui| {
            layout(ui, &mut tab, &settings);
        });

        // Changes: 5000 unstaged files and the 50k-line diff, unified then side by side.
        tab.view = View::Changes;
        tab.file = Some(FileSelection {
            source: DiffSource::Unstaged,
            entry: file.clone(),
            conflicted: false,
        });
        let unified = measure(&ctx, frames, egui::pos2(1100.0, 500.0), |ui| {
            layout(ui, &mut tab, &settings);
        });
        tab.split_diff = true;
        let split = measure(&ctx, frames, egui::pos2(1100.0, 500.0), |ui| {
            layout(ui, &mut tab, &settings);
        });
        tab.split_diff = false;

        // The resolver on a file with 400 conflicts (11k lines), every common stretch expanded.
        let parsed = ConflictFile::parse(&conflict_text());
        let mut r = Resolver::new(file.path.clone());
        r.reload(None, Ok(parsed.into()));
        r.expanded = (0..2_000).collect();
        tab.resolver = Some(r);
        tab.file = Some(FileSelection {
            source: DiffSource::Unstaged,
            entry: file,
            conflicted: true,
        });
        let resolver = measure(&ctx, frames, egui::pos2(1100.0, 500.0), |ui| {
            layout(ui, &mut tab, &settings);
        });

        println!(
            "frame cost (ms/frame, mean of {frames}): history {history:.2} | history+diff \
             {history_diff:.2} | changes unified {unified:.2} | changes split {split:.2} | \
             resolver {resolver:.2}"
        );
    }
}
