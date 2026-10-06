//! Cloning a repository on a background thread.

use super::*;

#[derive(Debug)]
pub(super) enum CloneMsg {
    Progress(String),
    Done(Result<PathBuf, String>),
}

impl CanopyApp {
    pub(super) fn start_clone(&mut self, ctx: &egui::Context, url: String, dest: PathBuf) {
        let (tx, rx) = unbounded();
        let thread_ctx = ctx.clone();
        // The status bar shows the URL for the whole clone.
        let shown = canopy_core::cli::redact_url(&url);
        let spawned = std::thread::Builder::new()
            .name("clone".into())
            .spawn(move || {
                let send = |m: CloneMsg| {
                    let _ = tx.send(m);
                    thread_ctx.request_repaint();
                };
                let result = clone_repository(&url, &dest, |l| {
                    send(CloneMsg::Progress(canopy_core::cli::redact_url(l)));
                });
                send(CloneMsg::Done(result));
            });
        match spawned {
            Ok(_) => self.clone = Some((shown, rx, "Starting…".into())),
            // The status bar never shows a clone that is not running.
            Err(e) => {
                tracing::error!("could not start the clone thread: {e}");
                self.clone_finished(ctx, Err(format!("Could not start the clone: {e}")));
            }
        }
    }

    /// Reports a finished clone: a success closes its form and opens the repository; a failure
    /// is shown in the form that started it, or as a toast once that form is gone.
    pub(super) fn clone_finished(&mut self, ctx: &egui::Context, result: Result<PathBuf, String>) {
        let shown = self
            .dialogs
            .finish_clone_with(result.as_ref().map(|_| ()).map_err(String::as_str));
        match result {
            Ok(path) => {
                self.toasts.push(
                    ctx,
                    ToastKind::Success,
                    Topic::Finished,
                    "Clone complete",
                    path.display().to_string(),
                );
                self.open_repo(ctx, path);
            }
            Err(e) if !shown => {
                self.toasts
                    .push(ctx, ToastKind::Error, Topic::Error, "Clone failed", e);
            }
            Err(_) => {}
        }
    }
}

fn clone_repository(
    url: &str,
    dest: &std::path::Path,
    progress: impl FnMut(&str),
) -> Result<PathBuf, String> {
    if dest
        .try_exists()
        .map_err(|e| format!("Could not check {}: {e}", dest.display()))?
    {
        return Err(format!(
            "The destination folder already exists: {}",
            dest.display()
        ));
    }
    canopy_core::ops::clone(url, dest, progress)
        .map_err(|e| canopy_core::cli::redact_url(&e.to_string()))?;
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app with no window, threads or saved state, for code that only reports results.
    fn quiet_app() -> CanopyApp {
        CanopyApp {
            settings: Settings::default(),
            tabs: Vec::new(),
            active: 0,
            dialogs: DialogStack::default(),
            settings_screen: None,
            folder_picker: crate::ui::folder_picker::FolderPicker::default(),
            palette: CommandPalette::default(),
            cheatsheet: Cheatsheet::default(),
            toasts: Toasts::default(),
            screenshot: None,
            initial_view: None,
            clone: None,
            opener: None,
            pending_opens: 0,
            git_version: String::new(),
            tokens: TokenStore::default(),
            updater: Updater::default(),
            identity_worker: None,
            global_identity: None,
            global_identity_error: None,
            gitlab_hosts: Vec::new(),
            update_check_due: false,
            askpass: None,
            trust_seen: 0,
            applied: None,
            gallery: None,
            glass_soak: None,
        }
    }

    /// The clone form that started a clone, still waiting for it.
    fn waiting_form() -> Dialog {
        let mut form = Dialog::clone_default(&Settings::default());
        if let Dialog::Clone { pending, .. } = &mut form {
            *pending = true;
        }
        form
    }

    /// DL-F2, DL-F10: git's message for a failed clone goes to the form that started it, and
    /// to a toast only when that form is gone.
    #[test]
    fn a_failed_clone_says_why_in_its_form_else_in_a_toast() {
        let ctx = egui::Context::default();
        let message = "fatal: repository 'https://example.com/x.git/' not found";
        let mut app = quiet_app();
        app.dialogs.open(waiting_form());
        app.clone_finished(&ctx, Err(message.to_owned()));
        assert!(
            matches!(app.dialogs.current(), Some(Dialog::Clone { error: Some(e), pending: false, .. }) if e == message),
            "{:?}",
            app.dialogs.current()
        );
        assert!(!format!("{:?}", app.toasts).contains("Clone failed"));

        let mut alone = quiet_app();
        alone.clone_finished(&ctx, Err(message.to_owned()));
        let toasts = format!("{:?}", alone.toasts);
        assert!(
            toasts.contains("Clone failed") && toasts.contains("not found"),
            "{toasts}"
        );
    }

    /// DL-F2: the app's poll hands a clone thread's failure to the form, not only to a toast.
    #[test]
    fn the_poll_reports_a_failed_clone_in_its_form() {
        let ctx = egui::Context::default();
        let mut app = quiet_app();
        app.dialogs.open(waiting_form());
        let (tx, rx) = unbounded();
        app.clone = Some(("https://example.com/x.git".into(), rx, String::new()));
        tx.send(CloneMsg::Done(Err("fatal: could not read Username".into())))
            .expect("send");
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.poll(ui.ctx()));
        out.textures_delta.clear();
        assert!(app.clone.is_none());
        assert!(
            matches!(app.dialogs.current(), Some(Dialog::Clone { error: Some(e), .. }) if e.contains("Username")),
            "{:?}",
            app.dialogs.current()
        );
    }

    /// A finished clone closes its form and says so.
    #[test]
    fn a_finished_clone_closes_its_form() {
        let ctx = egui::Context::default();
        let mut app = quiet_app();
        app.dialogs.open(waiting_form());
        app.clone_finished(&ctx, Ok(PathBuf::from("/home/ada/src/tidepool")));
        assert!(
            app.dialogs.current().is_none(),
            "{:?}",
            app.dialogs.current()
        );
        assert!(format!("{:?}", app.toasts).contains("Clone complete"));
    }

    /// The status bar shows the clone's URL for as long as it runs: never its credentials.
    #[test]
    fn a_running_clone_shows_no_credentials() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("temporary folder");
        let mut app = quiet_app();
        app.start_clone(
            &ctx,
            "https://user:secret@127.0.0.1:9/o/r.git".to_owned(),
            dir.path().join("r"),
        );
        let (url, rx, _) = app.clone.as_ref().expect("a clone runs");
        assert!(!url.contains("secret"), "{url}");
        let done = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the clone ends");
        assert!(
            matches!(&done, CloneMsg::Done(Err(e)) if !e.contains("secret")),
            "{done:?}"
        );
    }

    #[test]
    fn an_existing_destination_is_refused_before_cloning() {
        let dir = tempfile::tempdir().expect("temporary clone fixture");
        let source = dir.path().join("source");
        Repo::init(&source).expect("empty local repository");
        let dest = dir.path().join("existing");
        std::fs::create_dir(&dest).expect("existing destination");
        let result = clone_repository(&source.to_string_lossy(), &dest, |_| {});
        assert!(
            result.is_err(),
            "existing folders must be refused before Git accepts an empty one"
        );
        assert!(
            !dest.join(".git").exists(),
            "existing folder must be untouched"
        );
    }
}
