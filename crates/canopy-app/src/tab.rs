//! State of one open repository: the latest snapshot, selections and in-flight work.

use std::collections::HashSet;
use std::path::PathBuf;

use canopy_core::{CommitDetails, DiffSource, FileDiff, FileEntry, Oid, RepoSnapshot};

use crate::worker::{DiffKey, Event, Job, Op, Worker};

/// The two main views of a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Commit graph and details.
    History,
    /// Staging area and commit composer.
    Changes,
}

/// A file whose diff is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSelection {
    pub source: DiffSource,
    pub entry: FileEntry,
    pub conflicted: bool,
}

/// Outcome of a finished operation, surfaced as a toast by the app.
#[derive(Debug)]
pub struct Notice {
    pub ok: bool,
    pub title: String,
    pub body: String,
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
    pub filter: String,
    pub filtered: Option<Vec<usize>>,
    pub filter_for: String,
    pub commit_summary: String,
    pub commit_body: String,
    pub amend: bool,
    pub running: Option<String>,
    pub progress: Option<String>,
    pub commit_limit: usize,
    pub scroll_to_selected: bool,
    pub collapsed: HashSet<String>,
}

impl RepoTab {
    /// Opens a tab and asks for the first snapshot.
    pub fn new(path: PathBuf, ctx: &egui::Context, commit_limit: usize) -> Self {
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
            filter: String::new(),
            filtered: None,
            filter_for: String::new(),
            commit_summary: String::new(),
            commit_body: String::new(),
            amend: false,
            running: None,
            progress: None,
            commit_limit,
            scroll_to_selected: false,
            collapsed: HashSet::new(),
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

    /// Shows the diff of a file.
    pub fn select_file(&mut self, sel: FileSelection) {
        let key = DiffKey {
            source: sel.source.clone(),
            path: sel.entry.path.clone(),
            old_path: sel.entry.old_path.clone(),
            conflicted: sel.conflicted,
            context: 3,
        };
        if self.file.as_ref() == Some(&sel) && self.diff.is_some() {
            return;
        }
        self.file = Some(sel);
        self.diff_error = None;
        self.pending_diff = Some(key.clone());
        self.worker.send(Job::Diff(key));
    }

    fn reload_diff(&mut self) {
        if let Some(sel) = self.file.clone() {
            self.diff = None;
            self.select_file(sel);
        }
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

    /// Applies events from the worker. Returns notices for the app to show.
    pub fn poll(&mut self) -> Vec<Notice> {
        let mut notices = Vec::new();
        while let Ok(ev) = self.worker.events.try_recv() {
            match ev {
                Event::Snapshot(s) => {
                    self.name = s.name.clone();
                    let head = s.head.oid;
                    let had = self.snap.is_some();
                    self.snap = Some(*s);
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
                                self.diff = Some(d);
                                self.diff_error = None;
                            }
                            Err(e) => {
                                self.diff = None;
                                self.diff_error = Some(e);
                            }
                        }
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
                    match result {
                        Ok(out) => {
                            if label.starts_with("Commit") || label.starts_with("Amend") {
                                self.commit_summary.clear();
                                self.commit_body.clear();
                                self.amend = false;
                                self.selected_commit = None;
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
        notices
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
