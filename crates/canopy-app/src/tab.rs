//! State of one open repository: the latest snapshot, selections and in-flight work.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use canopy_core::forge::Forge;
use canopy_core::{CommitDetails, DiffSource, FileDiff, FileEntry, Oid, RepoSnapshot};
use canopy_hosting::{MergeMethod, RequestDetails, RequestSummary, StateFilter};

use crate::highlight::Highlights;
use crate::hosting::{HostEvent, HostJob, HostWorker, TokenSource};
use crate::settings::DiffPrefs;
use crate::worker::{DiffKey, Event, Job, Op, Worker};

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
    pub file: Option<Result<canopy_core::ConflictFile, String>>,
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
}

impl Resolver {
    fn new(path: String) -> Self {
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
        let step = |i: usize| (i as isize + delta).rem_euclid(n as isize) as usize;
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
        file: Result<canopy_core::ConflictFile, String>,
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

/// Pull or merge requests of the tab's forge.
#[derive(Debug)]
pub struct Requests {
    pub filter: StateFilter,
    /// Shared so a frame can hold the list while it mutates the tab, without copying it.
    pub list: Option<Result<Arc<Vec<RequestSummary>>, String>>,
    pub loading: bool,
    pub selected: Option<u64>,
    pub details: Option<Result<Arc<RequestDetails>, String>>,
    pub details_loading: bool,
    /// An action in flight ("Merging PR #12").
    pub busy: Option<String>,
    pub comment: String,
    pub merge_method: MergeMethod,
    /// Where the token came from; `None` while unknown, `Some(None)` without a token.
    pub token: Option<Option<TokenSource>>,
    /// When the list was last loaded.
    pub loaded_at: Option<Instant>,
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
        }
    }
}

/// One repository tab.
#[derive(Debug)]
pub struct RepoTab {
    pub path: PathBuf,
    pub name: String,
    pub worker: Worker,
    pub snap: Option<RepoSnapshot>,
    pub load_error: Option<String>,
    pub view: View,
    pub selected_commit: Option<Oid>,
    pub details: Option<CommitDetails>,
    pub file: Option<FileSelection>,
    pub diff: Option<FileDiff>,
    pub diff_error: Option<String>,
    pub pending_diff: Option<DiffKey>,
    /// Diff context and whitespace for this repository.
    pub diff_prefs: DiffPrefs,
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
    pub progress: Option<String>,
    pub commit_limit: usize,
    /// Centre the selected commit next frame.
    pub scroll_to_selected: bool,
    /// Scroll just enough to show the selected commit next frame.
    pub reveal_selected: bool,
    pub collapsed: HashSet<String>,
    /// GitHub and GitLab remotes: (remote name, forge).
    pub forges: Vec<(String, Forge)>,
    /// The API thread for the main forge remote.
    pub host: Option<HostWorker>,
    /// Self-hosted GitLab hosts the user listed in settings: environment tokens may go there.
    pub trusted_gitlab_hosts: Vec<String>,
    pub requests: Requests,
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
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let worker = Worker::spawn(path.clone(), ctx.clone());
        worker.send(Job::Refresh(commit_limit));
        Self {
            path,
            name,
            worker,
            snap: None,
            load_error: None,
            view: View::History,
            selected_commit: None,
            details: None,
            file: None,
            diff: None,
            diff_error: None,
            pending_diff: None,
            diff_prefs,
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
            progress: None,
            commit_limit,
            scroll_to_selected: false,
            reveal_selected: false,
            collapsed: HashSet::new(),
            forges: Vec::new(),
            host: None,
            trusted_gitlab_hosts: Vec::new(),
            requests: Requests::default(),
            last_fetch: Instant::now(),
        }
    }

    /// Queues a mutation.
    pub fn op(&self, op: Op) {
        self.worker.send(Job::Op(op));
    }

    /// Requests a full refresh.
    pub fn refresh(&self) {
        self.worker.send(Job::Refresh(self.commit_limit));
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
        let trusted = self.trusted_gitlab_hosts.clone();
        self.host = want.map(|f| HostWorker::spawn(f, saved_token, trusted, ctx.clone()));
        self.load_requests();
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
            self.requests.loading = true;
            h.send(HostJob::List(self.requests.filter));
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
    pub fn host_job(&mut self, job: HostJob) {
        if let Some(h) = &self.host {
            h.send(job);
        }
    }

    /// Selects a commit and loads its details.
    pub fn select_commit(&mut self, oid: Oid) {
        if self.selected_commit == Some(oid) {
            return;
        }
        self.selected_commit = Some(oid);
        self.details = None;
        self.file = None;
        self.diff = None;
        self.worker.send(Job::Details(oid));
    }

    fn diff_key(&self, sel: &FileSelection) -> DiffKey {
        DiffKey {
            source: sel.source.clone(),
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

    /// Moves the commit selection by `delta` rows (clamped); `None` jumps to the first row and
    /// `Some(isize::MAX)` to the last.
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
            (_, isize::MIN) => 0,
            (_, isize::MAX) => last,
            (None, _) => 0,
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
                self.file = None;
                self.diff = None;
            }
        }
    }

    /// Applies events from the workers. Returns notices for the app to show.
    pub fn poll(&mut self, gitlab_hosts: &[String]) -> Vec<Notice> {
        let mut notices = Vec::new();
        while let Ok(ev) = self.worker.events.try_recv() {
            match ev {
                Event::Snapshot(s) => {
                    self.name = s.name.clone();
                    let head = s.head.oid;
                    let had = self.snap.is_some();
                    self.snap = Some(*s);
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
                    } else if let Some(sel) = self.selected_commit
                        && self
                            .snap
                            .as_ref()
                            .is_some_and(|s| !s.commits.iter().any(|c| c.oid == sel))
                    {
                        self.selected_commit = None;
                        self.details = None;
                    }
                    self.reconcile_file_selection();
                }
                Event::Status(st) => {
                    if let Some(s) = &mut self.snap {
                        s.status = st;
                    }
                    self.reconcile_file_selection();
                }
                Event::Details(d) => {
                    if Some(d.oid) == self.selected_commit {
                        let first = d.files.first().cloned();
                        let oid = d.oid;
                        self.details = Some(*d);
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
                }
                Event::Diff(key, result) => {
                    if self.pending_diff.as_ref() == Some(&key) {
                        match result {
                            Ok(d) => {
                                if self.diff.as_ref() != Some(&d) {
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
                }
                Event::Highlight(key, h) => {
                    if self.pending_diff.as_ref() == Some(&key) {
                        self.highlights = Some(h);
                    }
                }
                Event::Conflict { path, kind, file } => {
                    if let Some(r) = self.resolver.as_mut().filter(|r| r.path == path) {
                        r.reload(kind, file);
                    }
                }
                Event::HeadMessage(m) => {
                    let mut parts = m.splitn(2, '\n');
                    self.commit_summary = parts.next().unwrap_or("").trim().to_owned();
                    self.commit_body = parts.next().unwrap_or("").trim().to_owned();
                }
                Event::OpStarted(label) => {
                    self.running = Some(label);
                    self.progress = None;
                }
                Event::Progress(line) => self.progress = Some(line),
                Event::OpFinished {
                    label,
                    announce,
                    result,
                } => {
                    self.running = None;
                    self.progress = None;
                    // Any operation can change the file being resolved (a save, a stash, a
                    // checkout): parse it again so choices and the save guard match the disk.
                    if let Some(r) = &self.resolver {
                        self.worker.send(Job::Conflict(r.path.clone()));
                    }
                    match result {
                        Ok(out) => {
                            if label.starts_with("Commit") || label.starts_with("Amend") {
                                self.commit_summary.clear();
                                self.commit_body.clear();
                                self.amend = false;
                                self.selected_commit = None;
                            }
                            if let Some(target) = label.strip_prefix("Squashing ") {
                                // Nothing is committed yet: prefill the message and show the
                                // staged result for review.
                                self.worker.send(Job::SquashMessage(target.to_owned()));
                                self.view = View::Changes;
                            }
                            if label.starts_with("Fetch") {
                                self.last_fetch = Instant::now();
                            }
                            if announce {
                                notices.push(Notice {
                                    ok: true,
                                    title: format!("{label} — done"),
                                    body: out.trim().to_owned(),
                                });
                            }
                        }
                        Err(e) => notices.push(Notice {
                            ok: false,
                            title: format!("{label} failed"),
                            body: e,
                        }),
                    }
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
        let events: Vec<HostEvent> = host.events.try_iter().collect();
        for ev in events {
            match ev {
                HostEvent::Token(t) => self.requests.token = Some(t),
                HostEvent::List(filter, result) => {
                    if filter == self.requests.filter {
                        self.requests.loading = false;
                        self.requests.loaded_at = Some(Instant::now());
                        let first = result
                            .as_ref()
                            .ok()
                            .and_then(|l| l.first())
                            .map(|r| r.number);
                        let keep = self.requests.selected.is_some_and(|n| {
                            result
                                .as_ref()
                                .is_ok_and(|l| l.iter().any(|r| r.number == n))
                        });
                        self.requests.list = Some(result.map(Arc::new));
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
                        self.requests.details = Some(result.map(Arc::new));
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
                    number,
                    result,
                } => {
                    self.requests.busy = None;
                    match result {
                        Ok(()) => {
                            if label.starts_with("Commenting") {
                                self.requests.comment.clear();
                            }
                            notices.push(Notice {
                                ok: true,
                                title: format!("{label} — done"),
                                body: String::new(),
                            });
                            if label.starts_with("Merging") {
                                // The target branch moved on the remote.
                                self.op(Op::Fetch {
                                    prune: false,
                                    quiet: true,
                                });
                            }
                        }
                        Err(e) => notices.push(Notice {
                            ok: false,
                            title: format!("{label} failed"),
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

    const TWO: &str = "a\n<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\nb\n<<<<<<< ours\n1\n=======\n2\n>>>>>>> theirs\n";

    #[test]
    fn reloading_keeps_choices_only_for_unchanged_conflicts() {
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(ConflictFile::parse(TWO)));
        assert_eq!(r.choices, vec![None, None]);
        r.choose(Choice::Ours);
        r.choose(Choice::Theirs);
        assert_eq!(r.resolved(), 2);
        // Same text: both choices stay.
        r.reload(None, Ok(ConflictFile::parse(TWO)));
        assert_eq!(r.resolved(), 2);
        // The second conflict edited in an editor: only the first choice stays.
        r.reload(
            None,
            Ok(ConflictFile::parse(&TWO.replace("\n2\n", "\n3\n"))),
        );
        assert_eq!(r.choices, vec![Some(Choice::Ours), None]);
        // One conflict resolved by hand: the remaining one is a different region now.
        let one = "a\nx\nb\n<<<<<<< ours\n1\n=======\n3\n>>>>>>> theirs\n";
        r.reload(None, Ok(ConflictFile::parse(one)));
        assert_eq!(r.choices, vec![None]);
        assert_eq!(r.focus, 0);
    }

    #[test]
    fn focus_moves_to_unresolved_conflicts_and_wraps() {
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(ConflictFile::parse(TWO)));
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
}
