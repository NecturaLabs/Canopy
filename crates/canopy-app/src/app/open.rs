//! Opening and creating repositories on a thread of their own, in request order, so a slow disk
//! or a large repository never stalls the UI.

use crossbeam_channel::Sender;

use super::*;

/// A repository to open (or create first).
#[derive(Debug)]
pub(super) struct OpenRequest {
    pub(super) path: PathBuf,
    pub(super) create: bool,
    /// Switch to the tab when it opens (a session restore focuses only its first tab).
    pub(super) focus: bool,
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
                let result =
                    open(&request.path, request.create).map_err(|e| dialogs::OpenFailure {
                        not_a_repository: matches!(e, canopy_core::Error::NotARepository(_)),
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

fn open(path: &std::path::Path, create: bool) -> canopy_core::Result<PathBuf> {
    let path = match path.to_str() {
        Some(typed) if typed.trim_start().starts_with('~') => settings::expand_home(typed),
        _ => path.to_path_buf(),
    };
    if create {
        Repo::init(&path)?;
    }
    let root = Repo::open(&path)?.workdir().to_path_buf();
    Ok(root.canonicalize().unwrap_or(root))
}

/// Where a closed tab's unsent commit message waits, for this run, until its repository opens
/// again.
fn draft_id(path: &std::path::Path) -> egui::Id {
    egui::Id::new(("commit-draft", path))
}

/// Keeps `tab`'s unsent commit message (summary and description) when the tab closes, so closing
/// a tab never throws a message away.
pub(super) fn keep_draft(ctx: &egui::Context, tab: &RepoTab) {
    if !tab.commit_summary.is_empty() || !tab.commit_body.is_empty() {
        let draft = (tab.commit_summary.clone(), tab.commit_body.clone());
        ctx.data_mut(|d| d.insert_temp(draft_id(&tab.path), draft));
    }
}

/// Puts back the message [`keep_draft`] kept for `tab`'s repository.
fn restore_draft(ctx: &egui::Context, tab: &mut RepoTab) {
    let id = draft_id(&tab.path);
    if let Some((summary, body)) = ctx.data_mut(|d| {
        let draft = d.get_temp::<(String, String)>(id);
        d.remove::<(String, String)>(id);
        draft
    }) {
        tab.commit_summary = summary;
        tab.commit_body = body;
    }
}

impl CanopyApp {
    /// Opens `path` in a tab (or switches to it) once the opener thread has found it.
    pub(super) fn open_repo(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.request_open(
            ctx,
            OpenRequest {
                path,
                create: false,
                focus: true,
            },
        );
    }

    /// Creates a repository at `path`, then opens it.
    pub(super) fn init_repo(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.request_open(
            ctx,
            OpenRequest {
                path,
                create: true,
                focus: true,
            },
        );
    }

    pub(super) fn request_open(&mut self, ctx: &egui::Context, request: OpenRequest) {
        let (path, create) = (request.path.clone(), request.create);
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
            if let Err(e) = &result {
                tracing::warn!(path = %request.path.display(), create = request.create, "could not open the repository: {}", e.message);
            }
            // The dialog that asked closes on success and shows a failure itself.
            let asked = self.dialogs.finish_open(
                &request.path,
                request.create,
                result.as_ref().map(|_| ()),
            );
            match result {
                Ok(root) => {
                    if request.create {
                        self.toasts.push(
                            ctx,
                            ToastKind::Success,
                            Topic::Finished,
                            "Repository created",
                            root.display().to_string(),
                        );
                    }
                    self.add_tab(ctx, root, request.focus);
                }
                Err(_) if asked => {}
                Err(e) => self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    Topic::Error,
                    if request.create {
                        "Could not create repository"
                    } else {
                        "Could not open repository"
                    },
                    e.message,
                ),
            }
        }
    }

    fn add_tab(&mut self, ctx: &egui::Context, root: PathBuf, focus: bool) {
        if let Some(i) = self.tabs.iter().position(|t| t.path == root) {
            if focus {
                self.active = i;
            }
        } else {
            let prefs = self.settings.diff_for(&root);
            let mut tab = RepoTab::new(root.clone(), ctx, self.settings.commit_page, prefs);
            restore_draft(ctx, &mut tab);
            tab.split_diff = self.settings.split_diff;
            tab.highlight_diff = self.settings.syntax_highlight;
            if let Some(v) = self.initial_view {
                tab.view = v;
            }
            self.tabs.push(tab);
            if focus || self.tabs.len() == 1 {
                self.active = self.tabs.len() - 1;
            }
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
