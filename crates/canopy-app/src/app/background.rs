//! Work polled every frame: worker and hosting events, credential prompts, the update check
//! and auto-fetch.

use super::*;

impl CanopyApp {
    /// Runs the start-up update check if one is due.
    pub(super) fn start_due_update_check(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.update_check_due) {
            self.settings.last_update_check = chrono::Utc::now().timestamp();
            self.updater
                .start(ctx, self.settings.include_prereleases, false);
        }
    }

    /// Shows git's and ssh's credential questions, one at a time, above any other dialog.
    pub(super) fn poll_prompts(&mut self, ctx: &egui::Context) {
        // A prompt git or ssh gave up on (killed, timed out) closes by itself.
        self.dialogs.drop_withdrawn();
        let Some(server) = &self.askpass else { return };
        while let Ok(prompt) = server.prompts.try_recv() {
            if prompt.kind == crate::askpass::PromptKind::Notice {
                // ssh shows it while it waits (e.g. for a security key touch); nothing to answer.
                self.toasts
                    .push(ctx, ToastKind::Info, "SSH", prompt.text.clone());
                prompt.answer(Some(String::new()));
                continue;
            }
            // In front of any other dialog; behind a prompt already open.
            self.dialogs.open(Dialog::credential(prompt));
        }
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        for tab in &mut self.tabs {
            for n in tab.poll(&self.gitlab_hosts) {
                self.toasts.push(
                    ctx,
                    if n.ok {
                        ToastKind::Success
                    } else {
                        ToastKind::Error
                    },
                    n.title,
                    n.body,
                );
            }
        }
        // Start the pull/merge request thread for the active tab once its remotes are known, and
        // not before the start-up questions are answered: until then Canopy contacts no service
        // (the Privacy Policy promises this).
        let answered = crate::ui::legal::pending(&self.settings).is_none();
        if let Some(t) = self.tabs.get_mut(self.active).filter(|_| answered) {
            let saved = t
                .forge()
                .and_then(|(_, f)| self.tokens.get_for(f))
                .map(str::to_owned);
            t.ensure_host(ctx, saved, self.settings.hosting_api);
        }
        let mut finished = None;
        if let Some((_, rx, status)) = &mut self.clone {
            loop {
                match rx.try_recv() {
                    Ok(CloneMsg::Progress(l)) => *status = l,
                    Ok(CloneMsg::Done(r)) => finished = Some(r),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    // The thread ended without reporting (it panicked).
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        finished.get_or_insert(Err("The clone stopped unexpectedly.".into()));
                        break;
                    }
                }
            }
        }
        if let Some(r) = finished {
            self.clone = None;
            match r {
                Ok(p) => {
                    self.toasts.push(
                        ctx,
                        ToastKind::Success,
                        "Clone complete",
                        p.display().to_string(),
                    );
                    self.open_repo(ctx, p);
                }
                Err(e) => self.toasts.push(ctx, ToastKind::Error, "Clone failed", e),
            }
        }
        self.poll_prompts(ctx);
        if self.updater.poll() {
            let manual = self.updater.manual;
            match &self.updater.state {
                UpdateState::Available(a) => {
                    let v = &a.version;
                    let skipped = self.settings.skipped_version.as_deref() == Some(&v.to_string());
                    if manual {
                        self.dialogs.open(Dialog::Update(a.clone()));
                    } else if !skipped {
                        self.toasts.push(
                            ctx,
                            ToastKind::Info,
                            format!("Canopy {v} is available"),
                            "Click the update badge in the status bar for what's new.",
                        );
                    } else {
                        self.updater.state = UpdateState::UpToDate;
                    }
                }
                UpdateState::UpToDate if manual => self.toasts.push(
                    ctx,
                    ToastKind::Success,
                    "Canopy is up to date",
                    format!("Version {}", env!("CARGO_PKG_VERSION")),
                ),
                UpdateState::Failed(e) if manual => self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    "Could not check for updates",
                    e.clone(),
                ),
                _ => {}
            }
        }
    }

    /// Queues background fetches for tabs whose auto-fetch interval has passed.
    pub(super) fn auto_fetch(&mut self, ctx: &egui::Context) {
        let minutes = self.settings.auto_fetch_minutes;
        if minutes == 0 || self.screenshot.is_some() {
            return;
        }
        let every = Duration::from_secs(u64::from(minutes) * 60);
        let prune = self.settings.prune_on_fetch;
        for t in &mut self.tabs {
            let idle = t.running.is_none();
            let has_remote = t.snap.as_ref().is_some_and(|s| !s.remotes.is_empty());
            if idle && has_remote && t.last_fetch.elapsed() >= every {
                t.last_fetch = Instant::now();
                t.op(Op::Fetch { prune, quiet: true });
            }
        }
        ctx.request_repaint_after(Duration::from_secs(30));
    }
}
