//! Opening and creating repositories on a thread of their own, in request order, so a slow disk
//! or a large repository never stalls the UI.

use crossbeam_channel::Sender;

use super::*;

/// How a new repository starts.
#[derive(Debug)]
pub(super) struct NewRepo {
    pub(super) ignore_template: canopy_core::IgnoreTemplate,
    /// Its first branch, unless `follow_config` and `init.defaultBranch` names one.
    pub(super) branch: String,
    pub(super) follow_config: bool,
}

/// A repository to open (or create first).
#[derive(Debug)]
pub(super) struct OpenRequest {
    pub(super) path: PathBuf,
    /// Create the repository first.
    pub(super) create: Option<NewRepo>,
    /// Switch to the tab when it opens (a session restore focuses only its first tab).
    pub(super) focus: bool,
    /// Tab path to replace after the target worktree opens successfully.
    pub(super) replace: Option<WorktreeSwitch>,
    /// Convert an old-style partial clone first (the user confirmed it).
    pub(super) convert_legacy: bool,
}

#[derive(Debug)]
pub(super) struct WorktreeSwitch {
    from: PathBuf,
    serial: u64,
}

fn switch_id(from: &std::path::Path) -> egui::Id {
    egui::Id::new(("pending-worktree-switch", from))
}

/// What the opener thread found.
#[derive(Debug)]
pub(super) struct Opened {
    request: OpenRequest,
    /// The working tree's canonical root, or git's (or libgit2's) message.
    result: Result<PathBuf, dialogs::OpenFailure>,
}

/// Starts the opener thread. Requests are handled one at a time, so tabs open in the order they
/// were asked for. `None` when the thread cannot start.
pub(super) fn start(ctx: &egui::Context) -> Option<(Sender<OpenRequest>, Receiver<Opened>)> {
    let (req_tx, req_rx) = unbounded::<OpenRequest>();
    let (done_tx, done_rx) = unbounded::<Opened>();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("opener".into())
        .spawn(move || {
            for request in req_rx {
                let result = open(
                    &request.path,
                    request.create.as_ref(),
                    request.convert_legacy,
                )
                .map_err(|e| dialogs::OpenFailure {
                    not_a_repository: matches!(e, canopy_core::Error::NotARepository(_)),
                    legacy_partial_clone: match &e {
                        canopy_core::Error::LegacyPartialClone { remote } => Some(remote.clone()),
                        _ => None,
                    },
                    message: e.to_string(),
                });
                if done_tx.send(Opened { request, result }).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    match spawned {
        Ok(_) => Some((req_tx, done_rx)),
        Err(e) => {
            tracing::error!("could not start the opener thread: {e}");
            None
        }
    }
}

fn open(
    path: &std::path::Path,
    create: Option<&NewRepo>,
    convert_legacy: bool,
) -> canopy_core::Result<PathBuf> {
    let path = match path.to_str() {
        Some(typed) if typed.trim_start().starts_with('~') => settings::expand_home(typed),
        _ => path.to_path_buf(),
    };
    if convert_legacy {
        canopy_core::convert_legacy_partial_clone(&path)?;
    }
    if let Some(new) = create {
        Repo::init(&path, &new.branch, new.follow_config)?;
        Repo::open(&path)?.add_ignore_template(new.ignore_template)?;
    }
    let root = Repo::open(&path)?.workdir().to_path_buf();
    Ok(root.canonicalize().unwrap_or(root))
}

/// Unsaved text retained for a repository during this run.
#[derive(Clone)]
struct RepositoryDraft {
    summary: String,
    body: String,
    merge_prefill: Option<(String, String)>,
    template_prefill: Option<(String, String)>,
    resolution: Option<(Option<crate::tab::FileSelection>, crate::tab::Resolver)>,
    parked_resolvers: Vec<crate::tab::Resolver>,
    comments: Vec<crate::tab::RequestCommentDraft>,
}

fn draft_id(path: &std::path::Path) -> egui::Id {
    egui::Id::new(("repository-draft", path))
}

/// Keeps unsent commit messages, conflict choices and comments when a tab closes or changes worktree.
pub(super) fn keep_draft(ctx: &egui::Context, tab: &RepoTab) {
    let resolution = tab.resolver.as_ref().and_then(|resolver| {
        resolver.has_draft().then(|| {
            let file = tab
                .file
                .as_ref()
                .filter(|file| file.conflicted && file.entry.path == resolver.path)
                .cloned();
            (file, resolver.clone())
        })
    });
    let comments = tab.request_comment_drafts();
    if !tab.commit_summary.is_empty()
        || !tab.commit_body.is_empty()
        || resolution.is_some()
        || !tab.parked_resolvers.is_empty()
        || !comments.is_empty()
    {
        let draft = RepositoryDraft {
            summary: tab.commit_summary.clone(),
            body: tab.commit_body.clone(),
            merge_prefill: tab.merge_prefill.clone(),
            template_prefill: tab.template_prefill.clone(),
            resolution,
            parked_resolvers: tab.parked_resolvers.clone(),
            comments,
        };
        ctx.data_mut(|d| d.insert_temp(draft_id(&tab.path), draft));
    }
}

/// Restores the draft; the next worker snapshot checks the file and reloads its conflicts.
fn restore_draft(ctx: &egui::Context, tab: &mut RepoTab) {
    let id = draft_id(&tab.path);
    if let Some(draft) = ctx.data_mut(|d| {
        let draft = d.get_temp::<RepositoryDraft>(id);
        d.remove::<RepositoryDraft>(id);
        draft
    }) {
        tab.commit_summary = draft.summary;
        tab.commit_body = draft.body;
        tab.merge_prefill = draft.merge_prefill;
        tab.template_prefill = draft.template_prefill;
        tab.request_comments = draft.comments;
        tab.parked_resolvers = draft.parked_resolvers;
        if let Some((file, resolver)) = draft.resolution {
            tab.file = file;
            tab.resolver = Some(resolver);
        }
    }
}

impl CanopyApp {
    /// Opens `path` in a tab (or switches to it) once the opener thread has found it.
    pub(super) fn open_repo(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.request_open(
            ctx,
            OpenRequest {
                path,
                create: None,
                focus: true,
                replace: None,
                convert_legacy: false,
            },
        );
    }

    /// Switches to a linked worktree only after Git confirms it is openable.
    pub(super) fn switch_worktree(&mut self, ctx: &egui::Context, path: PathBuf) {
        let Some(from) = self.tab().map(|t| t.path.clone()) else {
            return;
        };
        let serial = ctx.data_mut(|data| {
            let counter = egui::Id::new("worktree-switch-serial");
            let next = data.get_temp::<u64>(counter).unwrap_or(0).wrapping_add(1);
            data.insert_temp(counter, next);
            data.insert_temp(switch_id(&from), next);
            next
        });
        self.request_open(
            ctx,
            OpenRequest {
                path,
                create: None,
                focus: true,
                replace: Some(WorktreeSwitch { from, serial }),
                convert_legacy: false,
            },
        );
    }

    /// Creates a repository at `path`, then opens it.
    pub(super) fn init_repo(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        ignore_template: canopy_core::IgnoreTemplate,
    ) {
        self.request_open(
            ctx,
            OpenRequest {
                path,
                create: Some(NewRepo {
                    ignore_template,
                    branch: self.settings.initial_branch().to_owned(),
                    follow_config: self.settings.git_options().follow_config,
                }),
                focus: true,
                replace: None,
                convert_legacy: false,
            },
        );
    }

    pub(super) fn request_open(&mut self, ctx: &egui::Context, request: OpenRequest) {
        let (path, create) = (request.path.clone(), request.create.is_some());
        let sent = self
            .opener
            .as_ref()
            .is_some_and(|(tx, _)| tx.send(request).is_ok());
        if sent {
            self.pending_opens += 1;
            return;
        }
        // The form that asked says why and takes input again; otherwise a toast says it.
        let failure = dialogs::OpenFailure {
            not_a_repository: false,
            message: "Canopy could not start the thread that opens repositories.".to_owned(),
            legacy_partial_clone: None,
        };
        if !self.dialogs.finish_open(&path, create, Err(&failure)) {
            self.toasts.push(
                ctx,
                ToastKind::Error,
                Topic::Error,
                if create {
                    "Could not create repository"
                } else {
                    "Could not open repository"
                },
                failure.message,
            );
        }
    }

    /// Adds the tabs the opener thread finished.
    pub(super) fn poll_opened(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.opener else { return };
        let done: Vec<Opened> = rx.try_iter().collect();
        for Opened { request, result } in done {
            self.pending_opens = self.pending_opens.saturating_sub(1);
            if let Some(replacement) = &request.replace {
                let current = ctx.data_mut(|data| {
                    let id = switch_id(&replacement.from);
                    let current = data.get_temp::<u64>(id) == Some(replacement.serial);
                    if current {
                        data.remove::<u64>(id);
                    }
                    current
                });
                if !current {
                    continue;
                }
            }
            if let Err(e) = &result {
                tracing::warn!(path = %request.path.display(), create = request.create.is_some(), "could not open the repository: {}", e.message);
            }
            // The dialog that asked closes on success and shows a failure itself.
            let asked = self.dialogs.finish_open(
                &request.path,
                request.create.is_some(),
                result.as_ref().map(|_| ()),
            );
            match result {
                Ok(root) => {
                    if request.create.is_some() {
                        self.toasts.push(
                            ctx,
                            ToastKind::Success,
                            Topic::Finished,
                            "Repository created",
                            root.display().to_string(),
                        );
                    }
                    if let Some(replacement) = request.replace {
                        self.replace_tab(ctx, &replacement.from, root);
                    } else {
                        self.add_tab(ctx, root, request.focus);
                    }
                }
                Err(dialogs::OpenFailure {
                    legacy_partial_clone: Some(remote),
                    ..
                }) if request.create.is_none() => {
                    // The Open form (if it asked) keeps the message; this asks to convert.
                    self.dialogs.open(Dialog::ConvertPartialClone {
                        remote,
                        path: request.path.clone(),
                    });
                }
                Err(_) if asked => {}
                Err(e) => self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    Topic::Error,
                    if request.create.is_some() {
                        "Could not create repository"
                    } else {
                        "Could not open repository"
                    },
                    e.message,
                ),
            }
        }
    }

    fn new_tab(&self, ctx: &egui::Context, root: PathBuf) -> RepoTab {
        let prefs = self.settings.diff_for(&root);
        let mut tab = RepoTab::new(root, ctx, self.settings.commit_page, prefs);
        restore_draft(ctx, &mut tab);
        tab.split_diff = self.settings.split_diff;
        tab.request_density = self.settings.request_density;
        tab.highlight_diff = self.settings.syntax_highlight;
        if let Some(v) = self.initial_view {
            tab.view = v;
        }
        tab
    }

    /// Starts reading the repository's git configuration, to report what goes against
    /// Canopy's recommendation. A screenshot run reads nothing.
    pub(super) fn read_config_for(&mut self, ctx: &egui::Context, root: &std::path::Path) {
        if self.screenshot.is_none() {
            let rx = crate::repo_config::read_async(ctx, Some(root.to_path_buf()));
            self.config_reads.push((root.to_path_buf(), rx));
        }
    }

    /// Puts what the finished configuration reads found into the notification center.
    pub(super) fn poll_config_reads(&mut self, ctx: &egui::Context) {
        let mut waiting = Vec::new();
        for (path, rx) in std::mem::take(&mut self.config_reads) {
            match rx.try_recv() {
                Ok(facts) => {
                    let lines =
                        settings_screen::against_recommendation(&facts, &self.settings, &path);
                    if !lines.is_empty() {
                        self.toasts.push_offer_for(
                            ctx,
                            Topic::Attention,
                            "Git settings differ from the recommendation",
                            lines.join("\n"),
                            Offer::GitSettings,
                            &path,
                        );
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => waiting.push((path, rx)),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {}
            }
        }
        self.config_reads = waiting;
    }

    fn add_tab(&mut self, ctx: &egui::Context, root: PathBuf, focus: bool) {
        if let Some(i) = self.tabs.iter().position(|t| t.path == root) {
            if focus {
                self.active = i;
            }
        } else {
            let tab = self.new_tab(ctx, root.clone());
            self.tabs.push(tab);
            self.read_config_for(ctx, &root);
            if focus || self.tabs.len() == 1 {
                self.active = self.tabs.len() - 1;
            }
        }
        let identity_path = root.clone();
        self.settings.touch_recent(root);
        self.request_identity(IdentityJob::ReadRepo(identity_path));
    }

    /// Replaces the worktree in its existing tab, never a tab the user subsequently switched to.
    /// A closed source tab is not resurrected when its asynchronous open finally completes.
    fn replace_tab(&mut self, ctx: &egui::Context, from: &std::path::Path, root: PathBuf) {
        let Some(source) = self.tabs.iter().position(|t| t.path == from) else {
            return;
        };
        if self.tabs[source].path == root {
            return;
        }
        let focused = self.tab().map(|t| t.path.clone());
        let former = self.tabs.remove(source);
        let view = former.view;
        keep_draft(ctx, &former);

        if !self.tabs.iter().any(|t| t.path == root) {
            let mut tab = self.new_tab(ctx, root.clone());
            tab.view = view;
            self.tabs.insert(source, tab);
            self.read_config_for(ctx, &root);
        }
        let next_focus = if focused.as_deref() == Some(from) {
            Some(&root)
        } else {
            focused.as_ref()
        };
        if let Some(index) = next_focus.and_then(|p| self.tabs.iter().position(|t| &t.path == p)) {
            self.active = index;
        } else {
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        }
        let identity_path = root.clone();
        self.settings.touch_recent(root);
        self.request_identity(IdentityJob::ReadRepo(identity_path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// review-views-6: when the opener thread is gone, the Open form that asked says so and
    /// takes input again instead of waiting on "Opening…" for ever.
    #[test]
    fn an_open_the_opener_cannot_take_frees_its_form() {
        let ctx = egui::Context::default();
        let path = PathBuf::from("/home/ada/src/tidepool");
        let mut app = test_app();
        let mut form = Dialog::open();
        if let Dialog::Open { pending, .. } = &mut form {
            *pending = Some(path.clone());
        }
        app.dialogs.open(form);
        app.open_repo(&ctx, path);
        assert!(
            matches!(
                app.dialogs.current(),
                Some(Dialog::Open { pending: None, error: Some(e), .. })
                    if e.message.contains("could not start")
            ),
            "{:?}",
            app.dialogs.current()
        );
        assert_eq!(app.pending_opens, 0);
        assert!(
            !format!("{:?}", app.toasts).contains("Could not open"),
            "the form says it; no toast as well"
        );

        // With no form waiting, a toast says it.
        let mut alone = test_app();
        alone.open_repo(&ctx, PathBuf::from("/home/ada/src/other"));
        assert!(format!("{:?}", alone.toasts).contains("Could not open repository"));
    }

    /// #36: a partial clone from Git before 2.24 does not open; Canopy asks before it converts
    /// the configuration, and only the confirmed request does, then opens the repository.
    #[test]
    fn a_legacy_partial_clone_is_converted_only_after_the_confirmation() {
        let git = |dir: &std::path::Path, args: &[&str]| {
            canopy_core::cli::run(dir, args).expect("git runs");
        };
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("a folder");
        let repo = dir.path().join("legacy");
        std::fs::create_dir(&repo).expect("folder");
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "core.repositoryformatversion", "1"]);
        git(&repo, &["config", "extensions.partialClone", "origin"]);
        let mut app = test_app();
        app.opener = start(&ctx);
        app.open_repo(&ctx, repo.clone());
        let wait = |app: &mut CanopyApp| {
            for _ in 0..500 {
                app.poll_opened(&ctx);
                if app.pending_opens == 0 {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("the opener never answered");
        };
        wait(&mut app);
        assert!(app.tabs.is_empty(), "nothing opened before the answer");
        assert!(
            matches!(
                app.dialogs.current(),
                Some(Dialog::ConvertPartialClone { remote, .. }) if remote == "origin"
            ),
            "{:?}",
            app.dialogs.current()
        );
        let config = |key: &str| canopy_core::cli::run(&repo, &["config", "--get", key]).ok();
        assert_eq!(
            config("extensions.partialClone").as_deref().map(str::trim),
            Some("origin"),
            "asking changed nothing"
        );
        // The confirmed answer converts and opens.
        app.dialogs.close();
        app.apply(
            &ctx,
            vec![Action::ConvertPartialClone { path: repo.clone() }],
        );
        wait(&mut app);
        assert_eq!(app.tabs.len(), 1, "{:?}", app.dialogs.current());
        assert_eq!(
            config("remote.origin.promisor").as_deref().map(str::trim),
            Some("true")
        );
        assert_eq!(config("extensions.partialClone"), None);
    }

    /// Switching a worktree replaces the source tab, preserves its current view and draft,
    /// and never redirects focus away from a tab chosen while the opener was busy.
    #[test]
    fn switching_worktrees_keeps_generated_message_provenance() {
        use crate::worker::{Event, Worker};
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("folder");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for path in [&a, &b] {
            std::fs::create_dir(path).expect("folder");
        }
        let mut app = test_app();
        app.add_tab(&ctx, a.clone(), true);
        let (worker, events, _jobs) = Worker::detached();
        app.tabs[0].worker = worker;
        let merge = canopy_core::RepoSnapshot {
            state: canopy_core::RepoState::Merge,
            ..Default::default()
        };
        events
            .send(Event::Snapshot(std::sync::Arc::new(merge)))
            .expect("snapshot");
        events
            .send(Event::MergeMessage("Merge branch topic".into()))
            .expect("message");
        let _ = app.tabs[0].poll(&[]);
        assert_ne!(app.tabs[0].commit_summary, "");
        app.replace_tab(&ctx, &a, b.clone());
        app.replace_tab(&ctx, &b, a.clone());
        let (worker, events, _jobs) = Worker::detached();
        app.tabs[0].worker = worker;
        events
            .send(Event::Snapshot(std::sync::Arc::default()))
            .expect("aborted merge snapshot");
        let _ = app.tabs[0].poll(&[]);
        assert!(
            app.tabs[0].commit_summary.is_empty(),
            "an untouched aborted merge message is removed"
        );
        events
            .send(Event::CommitTemplate(Ok(Some(
                "Template summary\nTemplate body".into(),
            ))))
            .expect("template");
        let _ = app.tabs[0].poll(&[]);
        assert!(app.tabs[0].message_is_unwritten());
        app.replace_tab(&ctx, &a, b.clone());
        app.replace_tab(&ctx, &b, a);
        assert!(
            app.tabs[0].message_is_unwritten(),
            "Amend can replace the untouched restored template"
        );
        app.tabs[0].commit_summary.push_str(" edited");
        assert!(
            !app.tabs[0].message_is_unwritten(),
            "user edits remain drafts"
        );
    }

    #[test]
    fn the_latest_pending_worktree_switch_wins() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("folders");
        let source = dir.path().join("source");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for path in [&source, &a, &b] {
            std::fs::create_dir(path).expect("folder");
        }
        let mut app = test_app();
        app.add_tab(&ctx, source.clone(), true);
        app.tabs[0].commit_summary = "Keep this draft".into();
        let (requests, queued) = unbounded();
        let (replies, opened) = unbounded();
        app.opener = Some((requests, opened));
        app.switch_worktree(&ctx, a.clone());
        app.switch_worktree(&ctx, b.clone());
        let first = queued.recv().expect("first request");
        let second = queued.recv().expect("second request");
        replies
            .send(Opened {
                request: first,
                result: Ok(a),
            })
            .expect("first reply");
        app.poll_opened(&ctx);
        assert_eq!(
            app.tabs[0].path, source,
            "obsolete reply must not replace the source"
        );
        replies
            .send(Opened {
                request: second,
                result: Ok(b.clone()),
            })
            .expect("latest reply");
        app.poll_opened(&ctx);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].path, b);
        assert_eq!(app.pending_opens, 0);
        app.replace_tab(&ctx, &b, source);
        assert_eq!(app.tabs[0].commit_summary, "Keep this draft");
    }

    #[test]
    fn sidebar_sections_survive_saving_and_reopening_a_repository() {
        #[derive(Default)]
        struct Stored(std::collections::HashMap<String, String>);
        impl eframe::Storage for Stored {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.to_owned(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let ctx = egui::Context::default();
        let snap = canopy_core::RepoSnapshot {
            workdir: PathBuf::from("/fixture/main"),
            common_dir: PathBuf::from("/fixture/main/.git"),
            ..Default::default()
        };
        let mut app = test_app();
        let mut tab = test_tab(snap.clone());
        tab.collapsed.clear();
        tab.collapsed.insert("branches".into());
        tab.collapsed.insert("remote:origin".into());
        let wanted = tab.collapsed.clone();
        tab.sidebar_restored = true;
        app.tabs.push(tab);
        let other_snap = canopy_core::RepoSnapshot {
            workdir: PathBuf::from("/fixture/other"),
            common_dir: PathBuf::from("/fixture/other/.git"),
            ..Default::default()
        };
        let mut other = test_tab(other_snap.clone());
        other.sidebar_restored = true;
        other.collapsed.clear();
        app.tabs.push(other);
        let mut untouched = test_tab(canopy_core::RepoSnapshot {
            workdir: PathBuf::from("/fixture/untouched"),
            common_dir: PathBuf::from("/fixture/untouched/.git"),
            ..Default::default()
        });
        untouched.sidebar_restored = true;
        app.tabs.push(untouched);
        let mut storage = Stored::default();
        eframe::App::save(&mut app, &mut storage);
        assert!(
            !app.settings
                .repo_sidebar
                .contains_key(&PathBuf::from("/fixture/untouched/.git")),
            "a sidebar left as it opened stores nothing"
        );
        let mut reopened = test_app();
        reopened.settings = eframe::get_value(&storage, settings::KEY).expect("saved settings");
        let mut linked = snap.clone();
        linked.workdir = PathBuf::from("/fixture/linked");
        reopened.tabs.push(test_tab(snap));
        reopened.tabs.push(test_tab(linked));
        reopened.tabs.push(test_tab(other_snap));
        eframe::App::save(&mut reopened, &mut Stored::default());
        assert_eq!(
            reopened.settings.repo_sidebar[&PathBuf::from("/fixture/main/.git")],
            wanted
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            "saving before the first snapshot keeps the stored state"
        );
        reopened.poll(&ctx);
        assert_eq!(reopened.tabs[0].collapsed, wanted);
        assert_eq!(
            reopened.tabs[1].collapsed, wanted,
            "worktrees share the repository's state"
        );
        assert!(
            reopened.tabs[2].collapsed.is_empty(),
            "all-open state survives independently"
        );
        let closed = std::collections::BTreeSet::from(["tags".to_owned()]);
        reopened.apply(
            &ctx,
            vec![Action::RememberSidebar {
                repo: PathBuf::from("/fixture/main/.git"),
                closed: closed.clone(),
            }],
        );
        assert_eq!(reopened.tabs[0].collapsed, closed.iter().cloned().collect());
        assert_eq!(reopened.tabs[1].collapsed, reopened.tabs[0].collapsed);
        assert!(reopened.tabs[2].collapsed.is_empty());
        assert_eq!(
            reopened.settings.repo_sidebar[&PathBuf::from("/fixture/main/.git")],
            closed
        );
        reopened.poll(&ctx);
        assert_eq!(
            reopened.tabs[0].collapsed, reopened.tabs[1].collapsed,
            "refresh does not restore old state"
        );
    }

    #[test]
    fn request_filters_and_sort_are_kept_per_repository() {
        use crate::request_view::{ChecksChoice, RequestSort, RequestView};
        let ctx = egui::Context::default();
        let snap = |name: &str| canopy_core::RepoSnapshot {
            workdir: PathBuf::from(format!("/fixture/{name}")),
            common_dir: PathBuf::from(format!("/fixture/{name}/.git")),
            ..Default::default()
        };
        let mut app = test_app();
        app.tabs.push(test_tab(snap("main")));
        let mut linked = snap("main");
        linked.workdir = PathBuf::from("/fixture/linked");
        app.tabs.push(test_tab(linked.clone()));
        app.tabs.push(test_tab(snap("other")));
        app.poll(&ctx);
        let wanted = RequestView {
            text: "graph".into(),
            checks: Some(ChecksChoice::Failed),
            author: Some("sam".into()),
            sort: RequestSort::Oldest,
            ..RequestView::default()
        };
        let main = PathBuf::from("/fixture/main/.git");
        app.apply(
            &ctx,
            vec![Action::RememberRequestView {
                repo: main.clone(),
                view: wanted.clone(),
            }],
        );
        assert_eq!(app.tabs[0].request_view, wanted);
        assert_eq!(
            app.tabs[1].request_view, wanted,
            "worktrees share the repository's view"
        );
        assert!(app.tabs[2].request_view.is_default());

        // Across a restart: the settings round-trip, and a tab opened later restores its view.
        let saved = serde_json::to_string(&app.settings).expect("settings serialize");
        let mut reopened = test_app();
        reopened.settings = serde_json::from_str(&saved).expect("settings deserialize");
        reopened.tabs.push(test_tab(linked));
        reopened.tabs.push(test_tab(snap("other")));
        reopened.poll(&ctx);
        assert_eq!(reopened.tabs[0].request_view, wanted);
        assert!(reopened.tabs[1].request_view.is_default());

        // Back at the defaults, the repository stores nothing.
        reopened.apply(
            &ctx,
            vec![Action::RememberRequestView {
                repo: main.clone(),
                view: RequestView::default(),
            }],
        );
        assert!(!reopened.settings.repo_requests.contains_key(&main));
        assert!(reopened.tabs[0].request_view.is_default());
    }

    #[test]
    fn switching_worktrees_reuses_the_source_tab() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("a folder");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        let c = dir.path().join("c");
        for path in [&a, &b, &c] {
            std::fs::create_dir(path).expect("worktree folder");
        }
        let mut app = test_app();
        app.add_tab(&ctx, a.clone(), true);
        app.tabs[0].view = View::Changes;
        app.tabs[0].commit_summary = "Unsent".into();
        app.replace_tab(&ctx, &a, b.clone());
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].path, b);
        assert_eq!(app.tabs[0].view, View::Changes);
        assert_eq!(app.active, 0);

        // Switching an inactive source must not steal focus from the active tab.
        app.add_tab(&ctx, c.clone(), true);
        app.replace_tab(&ctx, &b, a.clone());
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs[0].path, a);
        assert_eq!(app.tabs[app.active].path, c);
        assert_eq!(app.tabs[0].commit_summary, "Unsent");
    }

    #[test]
    fn switching_worktrees_preserves_unsaved_conflict_resolution_text() {
        use crate::tab::{FileSelection, Resolver};
        use canopy_core::{ChangeKind, Choice, ConflictFile, DiffSource, FileEntry};

        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("a folder");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for path in [&a, &b] {
            std::fs::create_dir(path).expect("worktree folder");
        }
        let mut app = test_app();
        app.add_tab(&ctx, a.clone(), true);
        let mut resolver = Resolver::new("conflict.txt".into());
        resolver.reload(
            None,
            Ok(std::sync::Arc::new(ConflictFile::parse(
                "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\n",
            ))),
        );
        resolver.choices[0] = Some(Choice::Custom("custom result".into()));
        resolver.editing = Some((0, "unfinished edit".into()));
        app.tabs[0].resolver = Some(resolver);
        app.tabs[0].file = Some(FileSelection {
            source: DiffSource::Unstaged,
            entry: FileEntry {
                path: "conflict.txt".into(),
                old_path: None,
                kind: ChangeKind::Modified,
                additions: 0,
                deletions: 0,
                lfs: false,
                old_mode: None,
                new_mode: None,
            },
            conflicted: true,
        });

        app.replace_tab(&ctx, &a, b.clone());
        assert!(
            app.tabs[0].resolver.is_none(),
            "drafts stay with their worktree"
        );
        app.replace_tab(&ctx, &b, a);
        let (worker, events, jobs) = crate::worker::Worker::detached();
        app.tabs[0].worker = worker;
        let mut snapshot = canopy_core::RepoSnapshot::default();
        snapshot.head.oid = Some(canopy_core::Oid::from_bytes(&[1; 20]).expect("oid"));
        snapshot.status.conflicted.push(
            app.tabs[0]
                .file
                .as_ref()
                .expect("selected conflict")
                .entry
                .clone(),
        );
        events
            .send(crate::worker::Event::Snapshot(std::sync::Arc::new(
                snapshot,
            )))
            .expect("snapshot");
        app.tabs[0].poll(&[]);
        assert!(
            app.tabs[0].file.is_some(),
            "initial snapshot keeps the restored selection"
        );
        assert!(jobs.try_iter().any(|job| matches!(job, Job::Conflict(_))));
        events
            .send(crate::worker::Event::Conflict {
                path: "conflict.txt".into(),
                kind: None,
                file: Ok(std::sync::Arc::new(ConflictFile::parse(
                    "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\n",
                ))),
            })
            .expect("conflict reload");
        app.tabs[0].poll(&[]);
        app.replace_tab(&ctx, &app.tabs[0].path.clone(), b.clone());
        app.replace_tab(&ctx, &b, dir.path().join("a"));
        let restored = app.tabs[0].resolver.as_ref().expect("saved resolver");
        assert_eq!(restored.editing, Some((0, "unfinished edit".into())));
        assert_eq!(
            restored.choices,
            vec![Some(Choice::Custom("custom result".into()))]
        );
        assert!(
            app.tabs[0]
                .file
                .as_ref()
                .is_some_and(|file| file.conflicted)
        );
        // Returning to History hides the file without discarding its authored resolution.
        let mut history_file = app.tabs[0]
            .file
            .as_ref()
            .expect("conflicted selection")
            .clone();
        history_file.source =
            DiffSource::Commit(canopy_core::Oid::from_bytes(&[1; 20]).expect("oid"));
        history_file.conflicted = false;
        app.tabs[0].select_file(history_file);
        app.replace_tab(&ctx, &dir.path().join("a"), b.clone());
        app.replace_tab(&ctx, &b, dir.path().join("a"));
        let restored = app.tabs[0].resolver.as_ref().expect("draft after History");
        assert_eq!(restored.editing, Some((0, "unfinished edit".into())));
        assert_eq!(
            restored.choices,
            vec![Some(Choice::Custom("custom result".into()))]
        );
        assert!(app.tabs[0].file.is_none());
    }

    #[test]
    fn switching_worktrees_preserves_resolvers_parked_for_other_conflicted_files() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("a folder");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for path in [&a, &b] {
            std::fs::create_dir(path).expect("worktree folder");
        }
        let mut app = test_app();
        app.add_tab(&ctx, a.clone(), true);
        let mut parked = crate::tab::Resolver::new("other.txt".into());
        parked.choices = vec![Some(canopy_core::Choice::Ours)];
        app.tabs[0].parked_resolvers = vec![parked];
        app.replace_tab(&ctx, &a, b.clone());
        assert!(app.tabs[0].parked_resolvers.is_empty());
        app.replace_tab(&ctx, &b, a);
        let restored = &app.tabs[0].parked_resolvers;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].path, "other.txt");
        assert_eq!(restored[0].choices, vec![Some(canopy_core::Choice::Ours)]);
    }

    #[test]
    fn switching_worktrees_preserves_comments_for_their_forge_and_request() {
        use crate::hosting::{HostEvent, HostWorker};
        use canopy_core::forge::{Forge, ForgeKind};
        use canopy_hosting::{RequestState, RequestSummary, StateFilter};

        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("folders");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for path in [&a, &b] {
            std::fs::create_dir(path).expect("worktree folder");
        }
        let forge = Forge {
            kind: ForgeKind::GitHub,
            host: "github.com".into(),
            port: None,
            path: "example/project".into(),
        };
        let summary = || RequestSummary {
            number: 7,
            title: "Request".into(),
            author: "user".into(),
            state: RequestState::Open,
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
        };
        let mut app = test_app();
        app.add_tab(&ctx, a.clone(), true);
        let (host, _, _) = HostWorker::detached(forge.clone());
        app.tabs[0].host = Some(host);
        app.tabs[0].requests.selected = Some(7);
        app.tabs[0].requests.comment = "Unsent review text".into();
        app.replace_tab(&ctx, &a, b.clone());
        app.replace_tab(&ctx, &b, a);

        // A changed remote with the same request number must never inherit the comment.
        let mut other = forge.clone();
        other.path = "example/other".into();
        let (host, events, _) = HostWorker::detached(other);
        app.tabs[0].host = Some(host);
        events
            .send(HostEvent::List(
                StateFilter::Open.into(),
                Ok(vec![summary()]),
            ))
            .expect("list");
        app.tabs[0].poll(&[]);
        assert_eq!(app.tabs[0].requests.comment, "");
        app.tabs[0].ensure_host(&ctx, None, false);

        let (host, events, _) = HostWorker::detached(forge);
        app.tabs[0].host = Some(host);
        events
            .send(HostEvent::List(
                StateFilter::Open.into(),
                Ok(vec![summary()]),
            ))
            .expect("list");
        app.tabs[0].poll(&[]);
        assert_eq!(app.tabs[0].requests.comment, "Unsent review text");
        app.tabs[0].select_request(8);
        assert_eq!(app.tabs[0].requests.comment, "");
        app.tabs[0].select_request(7);
        assert_eq!(app.tabs[0].requests.comment, "Unsent review text");
    }

    #[test]
    fn switching_to_an_already_open_worktree_never_duplicates_a_tab() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("a folder");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).expect("folder");
        std::fs::create_dir(&b).expect("folder");
        let mut app = test_app();
        app.add_tab(&ctx, a.clone(), true);
        app.add_tab(&ctx, b.clone(), false);
        app.replace_tab(&ctx, &a, b.clone());
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[app.active].path, b);
        app.replace_tab(&ctx, &a, a.clone()); // source was closed in the meantime
        assert_eq!(app.tabs.len(), 1);
    }

    /// review-views-10: closing a tab keeps its unsent commit message; it is back when the
    /// repository opens again.
    #[test]
    fn a_closed_tab_keeps_its_commit_message_for_when_it_opens_again() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("a folder");
        let root = dir.path().to_path_buf();
        let mut app = test_app();
        app.add_tab(&ctx, root.clone(), true);
        "Fix the parser".clone_into(&mut app.tabs[0].commit_summary);
        "It read past the end.".clone_into(&mut app.tabs[0].commit_body);
        app.apply(&ctx, vec![Action::CloseTab(0)]);
        assert!(app.tabs.is_empty());
        app.add_tab(&ctx, root, true);
        assert_eq!(app.tabs[0].commit_summary, "Fix the parser");
        assert_eq!(app.tabs[0].commit_body, "It read past the end.");
    }
}
