//! State of one open repository: the latest snapshot, selections and in-flight work.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use canopy_core::forge::Forge;
use canopy_core::{CommitDetails, DiffSource, FileDiff, FileEntry, Oid, PullConfig, RepoSnapshot};
use canopy_hosting::{MergeMethod, RequestDetails, RequestSummary, StateFilter};

use crate::highlight::Highlights;
use crate::hosting::{HostAction, HostEvent, HostJob, HostWorker, TokenSource};
use crate::settings::{DiffPrefs, PullChoice};
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

/// A file whose diff is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSelection {
    pub source: DiffSource,
    pub entry: FileEntry,
    pub conflicted: bool,
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
        }
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
        if choices.len() != self.choices.len()
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

/// Outcome of a finished operation, surfaced as a toast by the app.
#[derive(Debug)]
pub struct Notice {
    pub ok: bool,
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
    ("Unstaging", "Unstaged", "unstage"),
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
        | OpKind::Squash(_)
        | OpKind::SafeDelete(_)
        | OpKind::Merge
        | OpKind::Stash
        | OpKind::Pop
        | OpKind::Branch
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
const REQUESTS_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// Pull or merge requests of the tab's forge.
#[derive(Debug)]
pub struct Requests {
    pub filter: StateFilter,
    /// Shared so a frame can hold the list while it mutates the tab, without copying it.
    pub list: Option<Result<Arc<[RequestSummary]>, String>>,
    pub loading: bool,
    pub selected: Option<u64>,
    pub details: Option<Result<Arc<RequestDetails>, String>>,
    pub details_loading: bool,
    /// An action in flight ("Merging PR #12").
    pub busy: Option<String>,
    pub comment: String,
    pub merge_method: MergeMethod,
    /// Where the token came from; `None` while unknown, `Some(None)` without a token.
    #[expect(
        clippy::option_option,
        reason = "not yet known, known to be absent, or known, as the doc comment says"
    )]
    pub token: Option<Option<TokenSource>>,
    /// When the list was last loaded.
    pub loaded_at: Option<Instant>,
    /// The newest load was a quiet refresh (no spinner): if it fails, the list on screen stays.
    /// Any load the user asked for clears it.
    pub quiet_refresh: bool,
}

impl Default for Requests {
    fn default() -> Self {
        Self {
            filter: StateFilter::Open,
            list: None,
            loading: false,
            selected: None,
            details: None,
            details_loading: false,
            busy: None,
            comment: String::new(),
            merge_method: MergeMethod::Merge,
            token: None,
            loaded_at: None,
            quiet_refresh: false,
        }
    }
}

/// Facts derived from a snapshot once, when it arrives, so frames do not recompute them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SnapIndex {
    /// Indices into `remote_branches` for each remote, in `remotes` order.
    pub remote_branches: Vec<Vec<usize>>,
    /// The widest graph row, in lanes.
    pub graph_lanes: u16,
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
        Self {
            remote_branches,
            graph_lanes: s.graph.iter().map(|g| g.width).max().unwrap_or(1),
        }
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
    /// Set when loading the details of `selected_commit` failed.
    pub details_error: Option<DetailsError>,
    pub file: Option<FileSelection>,
    pub diff: Option<Arc<FileDiff>>,
    /// Row layout of `diff` for the diff view, rebuilt only when the diff or layout changes.
    pub diff_layout: Option<crate::views::diff::DiffLayout>,
    pub diff_error: Option<String>,
    pub pending_diff: Option<DiffKey>,
    /// Diff context and whitespace for this repository.
    pub diff_prefs: DiffPrefs,
    /// Every repository's diff context and whitespace (Settings), kept current by the app.
    pub diff_default: DiffPrefs,
    /// Syntax highlighting of `diff`.
    pub highlights: Option<Arc<Highlights>>,
    /// Lines picked in the diff for line-level staging.
    pub line_sel: Option<LineSelection>,
    /// The conflict resolver, while a conflicted file is selected.
    pub resolver: Option<Resolver>,
    /// Side-by-side diff and syntax highlighting (from the settings).
    pub split_diff: bool,
    pub highlight_diff: bool,
    pub filter: String,
    pub filtered: Option<Vec<usize>>,
    pub filter_for: String,
    pub commit_summary: String,
    pub commit_body: String,
    pub amend: bool,
    /// Move keyboard focus to the commit summary next frame.
    pub focus_summary: bool,
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
    pull_config_branch: Option<String>,
    /// A branch `git branch -d` refused as not fully merged; the app asks before forcing it.
    pub force_delete_offer: Option<String>,
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
    pub collapsed: HashSet<String>,
    /// The sidebar row last clicked ([`RepoTab::sidebar_shows`]).
    pub sidebar_pick: Option<SidebarItem>,
    /// GitHub and GitLab remotes: (remote name, forge).
    pub forges: Vec<(String, Forge)>,
    /// The API thread for the main forge remote.
    pub host: Option<HostWorker>,
    /// Self-hosted GitLab hosts the user listed in settings: environment tokens may go there.
    pub trusted_gitlab_hosts: Vec<String>,
    pub requests: Requests,
    /// The promisor remote a read needed but Canopy did not download from, because the
    /// repository is outside every trusted folder; the status bar offers to trust it.
    pub downloads_refused: Option<String>,
    /// The newest version of the trusted list this tab has been told about: a refusal from an
    /// older one is out of date.
    trust_version: u64,
    pub avatars: crate::avatars::Avatars,
    /// The last fetch, for auto-fetch.
    pub last_fetch: Instant,
}

impl RepoTab {
    /// Opens a tab and asks for the first snapshot.
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

    pub(crate) fn with_worker(
        path: PathBuf,
        commit_limit: usize,
        diff_prefs: DiffPrefs,
        worker: Worker,
    ) -> Self {
        let name = folder_name(&path);
        Self {
            path,
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
            details_error: None,
            file: None,
            diff: None,
            diff_layout: None,
            diff_error: None,
            pending_diff: None,
            diff_prefs,
            diff_default: diff_prefs,
            highlights: None,
            line_sel: None,
            resolver: None,
            split_diff: false,
            highlight_diff: true,
            filter: String::new(),
            filtered: None,
            filter_for: String::new(),
            commit_summary: String::new(),
            commit_body: String::new(),
            amend: false,
            focus_summary: false,
            running: None,
            running_kind: None,
            finished: None,
            pull: PullChoice::default(),
            git_options: None,
            pull_config: None,
            pull_config_asked: false,
            pull_config_branch: None,
            force_delete_offer: None,
            force_push_offer: None,
            pull_decision: None,
            progress: None,
            commit_limit,
            scroll_to_selected: false,
            reveal_selected: false,
            collapsed: HashSet::new(),
            sidebar_pick: None,
            forges: Vec::new(),
            host: None,
            trusted_gitlab_hosts: Vec::new(),
            requests: Requests::default(),
            downloads_refused: None,
            trust_version: 0,
            avatars: crate::avatars::Avatars::default(),
            last_fetch: Instant::now(),
        }
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
    pub fn op(&self, op: Op) {
        self.worker.send(Job::Op(op));
    }

    /// Requests a full refresh.
    pub fn refresh(&self) {
        self.worker.send(Job::Refresh(self.commit_limit));
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
        self.trusted_gitlab_hosts = gitlab_hosts.to_vec();
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
    pub fn ensure_host(&mut self, ctx: &egui::Context, saved_token: Option<String>, enabled: bool) {
        let want = if enabled {
            self.forge().map(|(_, f)| f.clone())
        } else {
            None
        };
        if self.host.as_ref().map(|h| &h.forge) == want.as_ref() {
            return;
        }
        self.requests = Requests::default();
        self.avatars = crate::avatars::Avatars::default();
        let trusted = self.trusted_gitlab_hosts.clone();
        self.host = want.map(|f| HostWorker::spawn(f, saved_token, trusted, ctx.clone()));
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

    /// Restarts the API thread, e.g. after the token changed.
    pub fn restart_host(
        &mut self,
        ctx: &egui::Context,
        saved_token: Option<String>,
        enabled: bool,
    ) {
        self.host = None;
        self.ensure_host(ctx, saved_token, enabled);
    }

    /// Reloads the request list.
    pub fn load_requests(&mut self) {
        if let Some(h) = &self.host {
            self.requests.quiet_refresh = false;
            self.requests.loading = true;
            h.send(HostJob::List(self.requests.filter));
            // Refreshing the rows must refresh the open request's body and action state too.
            // Otherwise a manual refresh leaves its details stale until another selection.
            if let Some(n) = self.requests.selected
                && self.requests.details.is_some()
            {
                self.requests.details_loading = true;
                h.send(HostJob::Details(n));
            }
        }
    }

    /// Loads the request list again without a spinner, keeping the list, the selection and the
    /// open request on screen until the new ones arrive. Does nothing without a hosting thread
    /// (hosting off, no consent, no forge) or while the list was never loaded: a tab nobody
    /// opened asks its forge for nothing.
    pub fn refresh_requests_quietly(&mut self) {
        let Some(h) = &self.host else { return };
        if self.requests.list.is_none() {
            return;
        }
        self.requests.quiet_refresh = true;
        h.send(HostJob::List(self.requests.filter));
        if let Some(n) = self.requests.selected
            && self.requests.details.is_some()
        {
            h.send(HostJob::Details(n));
        }
    }

    /// Refreshes the list quietly when it was loaded more than [`REQUESTS_STALE_AFTER`] ago:
    /// the Requests view opens on a list as current as the last sync, not as old as the tab.
    pub fn refresh_requests_if_stale(&mut self) {
        if !self.requests.loading
            && self
                .requests
                .loaded_at
                .is_some_and(|at| at.elapsed() >= REQUESTS_STALE_AFTER)
        {
            self.refresh_requests_quietly();
        }
    }

    /// Shows one request.
    pub fn select_request(&mut self, number: u64) {
        if self.requests.selected == Some(number) && self.requests.details.is_some() {
            return;
        }
        self.requests.selected = Some(number);
        self.requests.details = None;
        self.requests.comment.clear();
        if let Some(h) = &self.host {
            self.requests.details_loading = true;
            h.send(HostJob::Details(number));
        }
    }

    /// Runs an API action.
    pub fn host_job(&self, job: HostJob) {
        if let Some(h) = &self.host {
            h.send(job);
        }
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
        self.details_error = None;
        self.deselect_file();
        self.worker.send(Job::Details(oid));
    }

    /// Closes the open file and its diff, with any error that was shown on it. A diff still on its
    /// way for that file is no longer wanted.
    pub fn deselect_file(&mut self) {
        self.file = None;
        self.diff = None;
        self.diff_error = None;
        self.pending_diff = None;
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
    pub fn trust_changed(&mut self, version: u64) {
        self.trust_version = version;
        if self.downloads_refused.take().is_some() {
            self.reread();
        }
    }

    fn reread(&mut self) {
        self.refresh();
        self.details_error = None;
        if let Some(oid) = self.selected_commit {
            self.worker.send(Job::Details(oid));
        }
        self.reload_diff();
    }

    /// Hides the "Downloads off" chip once nothing it explains is left on screen. It explains a
    /// refusal's own message in the status, the details or the diff, line counts left undone, and
    /// a commit whose details are still on their way (they may be refused too); an error of
    /// another kind is not its to explain.
    fn clear_obsolete_refusal(&mut self) {
        let refusal = |message: &str| message.contains(crate::worker::NOT_TRUSTED);
        // No snapshot yet: the status is still to come.
        let status = self.snap.as_ref().is_none_or(|s| {
            s.status_unavailable
                .as_ref()
                .is_some_and(|u| refusal(&u.message))
        });
        let details_refused = self
            .details_error
            .as_ref()
            .is_some_and(|e| refusal(&e.message));
        // With no file open there is no diff to show the error on.
        let diff_refused = self.file.is_some() && self.diff_error.as_deref().is_some_and(refusal);
        // The worker asks for line counts only of a commit with up to `MAX_STAT_FILES` files.
        let details_open = self.selected_commit.is_some_and(|oid| {
            self.details_error.is_none()
                && self.details.as_ref().is_none_or(|d| {
                    d.oid != oid
                        || (!d.stats_known && d.files.len() <= canopy_core::repo::MAX_STAT_FILES)
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
        }
        if sel.conflicted {
            if self
                .resolver
                .as_ref()
                .is_none_or(|r| r.path != sel.entry.path)
            {
                self.resolver = Some(Resolver::new(sel.entry.path.clone()));
            }
            self.worker.send(Job::Conflict(sel.entry.path.clone()));
        } else {
            self.resolver = None;
        }
        self.file = Some(sel);
        self.diff_error = None;
        self.pending_diff = Some(key.clone());
        self.worker.send(Job::Diff(key));
    }

    /// Reloads the open diff, keeping the old one on screen until the new one arrives.
    pub fn reload_diff(&mut self) {
        if let Some(sel) = self.file.clone() {
            if sel.conflicted {
                self.worker.send(Job::Conflict(sel.entry.path.clone()));
            }
            let key = self.diff_key(&sel);
            self.diff_error = None;
            self.pending_diff = Some(key.clone());
            self.worker.send(Job::Diff(key));
        }
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
        let numbers: Vec<u64> = match &self.requests.list {
            Some(Ok(l)) => l.iter().map(|r| r.number).collect(),
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

    /// Applies events from the workers. Returns notices for the app to show.
    pub fn poll(&mut self, gitlab_hosts: &[String]) -> Vec<Notice> {
        let mut notices = Vec::new();
        while let Ok(ev) = self.worker.events.try_recv() {
            match ev {
                Event::Snapshot(s) => {
                    let entering_merge = s.state == canopy_core::RepoState::Merge
                        && self
                            .snap
                            .as_ref()
                            .is_none_or(|old| old.state != canopy_core::RepoState::Merge);
                    if s.state != canopy_core::RepoState::Clean {
                        self.amend = false;
                    }
                    if entering_merge
                        && self.commit_summary.trim().is_empty()
                        && self.commit_body.trim().is_empty()
                    {
                        self.worker.send(Job::MergeMessage);
                    }
                    // The pull settings depend on the current branch: read them again when it
                    // changes (and for the first snapshot).
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
                    self.generation += 1;
                    self.update_forges(gitlab_hosts);
                    self.load_error = None;
                    self.filter_for = String::from("\u{0}");
                    if !had || self.selected_commit.is_none() {
                        if let Some(h) = head.or_else(|| {
                            self.snap
                                .as_ref()
                                .and_then(|s| s.commits.first().map(|c| c.oid))
                        }) {
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
                    if let Some(s) = &mut self.snap {
                        // Copies the snapshot only if a frame still shares it (it does not
                        // between frames).
                        let s = Arc::make_mut(s);
                        s.status = st;
                        s.status_unavailable = None;
                    }
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
                        match result {
                            Ok(d) => {
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
                        && self.commit_summary.trim().is_empty()
                        && self.commit_body.trim().is_empty()
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
                Event::PullConfig(result) => match result {
                    Ok(config) => self.pull_config = Some(config),
                    Err(e) => {
                        tracing::debug!(error = %e, "pull settings not read");
                        self.pull_config = None;
                    }
                },
                Event::PushTarget(result) => match result {
                    Ok(target) => self.force_push_offer = Some(target),
                    Err(e) => notices.push(Notice {
                        ok: false,
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
                        || !self.commit_summary.trim().is_empty()
                        || !self.commit_body.trim().is_empty()
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
                        && self.commit_summary.trim().is_empty()
                        && self.commit_body.trim().is_empty()
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
                            if kind == OpKind::Fetch {
                                self.last_fetch = Instant::now();
                            }
                            // The remote moved, or Canopy moved it: its requests may have too.
                            if kind.syncs_remote() {
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
                            _ => {
                                self.finished = Some(Finished {
                                    label: label.clone(),
                                    kind: kind.clone(),
                                    ok: false,
                                    at: Instant::now(),
                                });
                                notices.push(Notice {
                                    ok: false,
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
                Event::Conflicted(label) => {
                    self.running = None;
                    self.running_kind = None;
                    self.progress = None;
                    // The resolver lives in Changes: take the user to the conflicted files.
                    self.view = View::Changes;
                    notices.push(Notice {
                        ok: false,
                        title: format!("{label} stopped on conflicts"),
                        body: "Resolve each conflicted file in Changes, then continue, or abort to go back to where you were.".into(),
                    });
                }
                Event::AutostashConflict => {
                    // The resolver lives in Changes: take the user to the conflicted files.
                    self.view = View::Changes;
                    notices.push(Notice {
                        ok: false,
                        title: "Your changes conflict with what came in".into(),
                        body: "Canopy set your uncommitted changes aside for this, and putting them back conflicts with the new commits. Resolve the conflicted files in Changes. Your changes also stay in the stash list until you drop that stash.".into(),
                    });
                }
                Event::Failed(e) => {
                    if self.snap.is_none() {
                        self.load_error = Some(e.clone());
                    }
                    notices.push(Notice {
                        ok: false,
                        title: "Git error".into(),
                        body: e,
                    });
                }
            }
        }
        self.poll_host(&mut notices);
        notices
    }

    fn poll_host(&mut self, notices: &mut Vec<Notice>) {
        let Some(host) = &self.host else { return };
        let kind = host.forge.kind;
        let ctx = host.context().clone();
        let events: Vec<HostEvent> = host.events.try_iter().collect();
        for ev in events {
            match ev {
                HostEvent::Avatar { oid, email, result } => {
                    self.avatars.finish(&ctx, oid, email, result);
                }
                HostEvent::Token(t) => self.requests.token = Some(t),
                HostEvent::List(filter, result) => {
                    let kept_through_failure = self.requests.quiet_refresh
                        && result.is_err()
                        && matches!(self.requests.list, Some(Ok(_)));
                    if kept_through_failure && filter == self.requests.filter {
                        // A quiet refresh failed (a network blip, a rate limit): the list on
                        // screen is still the best there is. The next sync tries again.
                        tracing::debug!("a quiet request refresh failed; keeping the list");
                        self.requests.quiet_refresh = false;
                    } else if filter == self.requests.filter {
                        self.requests.loading = false;
                        self.requests.loaded_at = Some(Instant::now());
                        let first = result
                            .as_ref()
                            .ok()
                            .and_then(|l| l.first())
                            .map(|r| r.number);
                        self.generation += 1;
                        let keep = self.requests.selected.is_some_and(|n| {
                            result
                                .as_ref()
                                .is_ok_and(|l| l.iter().any(|r| r.number == n))
                        });
                        self.requests.list = Some(result.map(Arc::from));
                        if !keep {
                            self.requests.selected = None;
                            self.requests.details = None;
                            if let Some(n) = first {
                                self.select_request(n);
                            }
                        }
                    }
                }
                HostEvent::Details(n, result) => {
                    if self.requests.selected == Some(n) {
                        self.requests.details_loading = false;
                        // A quiet refresh of the open request that fails leaves it shown.
                        let kept = self.requests.quiet_refresh
                            && result.is_err()
                            && matches!(self.requests.details, Some(Ok(_)));
                        if !kept {
                            self.requests.details = Some(result.map(Arc::new));
                        }
                    }
                }
                HostEvent::Created(result) => {
                    self.requests.busy = None;
                    match result {
                        Ok(r) => {
                            notices.push(Notice {
                                ok: true,
                                title: format!("{} #{} created", kind.request_abbrev(), r.number),
                                body: r.title.clone(),
                            });
                            self.requests.filter = StateFilter::Open;
                            self.load_requests();
                            self.select_request(r.number);
                        }
                        Err(e) => notices.push(Notice {
                            ok: false,
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
                    self.requests.busy = None;
                    match result {
                        Ok(()) => {
                            if action == HostAction::Comment {
                                self.requests.comment.clear();
                            }
                            notices.push(Notice {
                                ok: true,
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
                            title: outcome_title(&label, false),
                            body: e,
                        }),
                    }
                    self.load_requests();
                    if let Some(n) = number {
                        self.requests.details = None;
                        self.requests.selected = None;
                        self.select_request(n);
                    }
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
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while tab.commit_summary.is_empty() && Instant::now() < deadline {
            tab.poll(&[]);
            std::thread::sleep(std::time::Duration::from_millis(5));
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
        events.send(Event::Conflicted("Pulling".into()))?;
        let notices = tab.poll(&[]);
        assert_eq!((tab.running.as_deref(), tab.view), (None, View::Changes));
        assert_eq!(notices.len(), 1);
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
        use canopy_core::forge::ForgeKind;
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
                    .any(|j| matches!(j, HostJob::List(StateFilter::Open))),
                "{kind:?} asked for the list again: {jobs:?}"
            );
            assert!(
                jobs.iter().any(|j| matches!(j, HostJob::Details(7))),
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
    fn a_manual_request_refresh_also_reloads_the_open_details() {
        let (mut tab, _events, host_jobs) = tab_showing_requests();
        tab.load_requests();
        let jobs = jobs_of(&host_jobs);
        assert!(
            matches!(
                jobs.as_slice(),
                [HostJob::List(StateFilter::Open), HostJob::Details(7)]
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
            StateFilter::Open,
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
            StateFilter::Open,
            Err("could not reach github.com".into()),
        ))?;
        tab.poll(&[]);
        assert!(matches!(tab.requests.list, Some(Err(_))));
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
