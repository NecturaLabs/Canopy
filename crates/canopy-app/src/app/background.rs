//! Work polled every frame: worker and hosting events, credential prompts, the update check
//! and auto-fetch.

use super::*;
use canopy_core::identity::{Scope, Source};

use crate::identity_worker::Event as IdentityEvent;
use crate::tab::REQUESTS_REFRESH_EVERY;
use crate::views::settings_screen::source_label;

/// What the first-launch identity question should do now.
#[derive(Debug, PartialEq, Eq)]
enum IdentityStep {
    /// Not yet: already asked, a consent question or the question itself is on screen, or the
    /// global identity has not been read.
    Wait,
    /// A sensible global identity exists, so there is nothing to ask.
    Settled,
    /// Show the question.
    Ask,
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "the step is decided from independent facts about the app's state"
)]
fn identity_step(
    asked: bool,
    consent_pending: bool,
    dialog_open: bool,
    global: Option<&Identity>,
    global_failed: bool,
) -> IdentityStep {
    if asked || consent_pending || dialog_open {
        return IdentityStep::Wait;
    }
    match global {
        Some(identity) if identity.warning().is_none() => IdentityStep::Settled,
        Some(_) => IdentityStep::Ask,
        None if global_failed => IdentityStep::Ask,
        None => IdentityStep::Wait,
    }
}

/// The toast for a finished save or clear. A clear reports what Git uses now, since a value that
/// also comes from another file keeps winning.
fn changed_toast(
    cleared: bool,
    scope: Scope,
    result: Result<(), String>,
    now: Option<&Identity>,
) -> (ToastKind, &'static str, String) {
    match result {
        Err(error) => (ToastKind::Error, "Could not change Git identity", error),
        Ok(()) if cleared => (
            ToastKind::Success,
            "Git identity cleared",
            cleared_body(scope, now),
        ),
        Ok(()) => (
            ToastKind::Success,
            "Git identity saved",
            "Future commits use it, unless an environment variable or an author setting overrides it."
                .to_owned(),
        ),
    }
}

fn cleared_body(scope: Scope, now: Option<&Identity>) -> String {
    let Some(now) = now else {
        return "Removed from that configuration file. Settings → Git shows what Git uses now."
            .to_owned();
    };
    let cleared_scope = match scope {
        Scope::Local => "local",
        Scope::Global => "global",
    };
    let author = &now.author;
    if let Some(field) = [&author.name, &author.email]
        .into_iter()
        .find(|f| matches!(&f.source, Some(Source::Config { scope, .. }) if scope == cleared_scope))
    {
        return format!(
            "Some of it is still set in {}, which Canopy did not change.",
            source_label(field.source.as_ref())
        );
    }
    if !author.usable {
        return "Git has no usable identity now.".to_owned();
    }
    format!(
        "Git now uses {} <{}>: name from {}, email from {}.",
        author.name.value,
        author.email.value,
        source_label(author.name.source.as_ref()),
        source_label(author.email.source.as_ref()),
    )
}

/// A save started from the first-launch dialog closes it when it worked and re-enables it when
/// it did not, so the user can correct the values.
fn settle_identity_dialog(dialogs: &mut DialogStack, saved: bool) {
    if saved {
        dialogs.close_where(|d| matches!(d, Dialog::Identity { pending: true, .. }));
    } else if let Some(Dialog::Identity { pending, .. }) = dialogs.current_mut() {
        *pending = false;
    }
}

/// The question before a force push: the branch and the remote branch it overwrites.
fn force_push_dialog(repo: &std::path::Path, target: canopy_core::PushTarget) -> Dialog {
    let canopy_core::PushTarget {
        branch,
        remote,
        dest,
    } = target;
    Dialog::Confirm {
        title: format!("Force push {branch} to {remote}/{dest}?"),
        body: format!(
            "{remote}/{dest} is overwritten with your local {branch}. --force-with-lease refuses if someone else pushed in the meantime."
        ),
        confirm: "Force push branch".into(),
        danger: true,
        repo: repo.to_path_buf(),
        op: Op::Push {
            force: true,
            expect: Some(canopy_core::PushTarget {
                branch,
                remote,
                dest,
            }),
        },
    }
}

impl CanopyApp {
    pub(super) fn request_git_config(&mut self, ctx: &egui::Context, job: IdentityJob) {
        if self
            .identity_worker
            .as_ref()
            .is_some_and(|worker| worker.send(job))
        {
            return;
        }
        if let Some(screen) = &mut self.settings_screen {
            screen.git_config_finished();
        }
        self.toasts.push(
            ctx,
            ToastKind::Error,
            Topic::Error,
            "Git settings were not saved",
            "The Git settings helper is unavailable. Restart Canopy and try again.",
        );
    }

    pub(super) fn request_identity(&mut self, job: IdentityJob) {
        if self
            .identity_worker
            .as_ref()
            .is_some_and(|worker| worker.send(job))
        {
            return;
        }
        tracing::error!("could not queue Git identity work");
        // Without this Settings would say "Reading Git identity…" for ever.
        let message =
            "Git identity is unavailable: its helper thread is not running. Restart Canopy.";
        self.global_identity_error = Some(message.to_owned());
        for tab in &mut self.tabs {
            tab.identity_error = Some(message.to_owned());
        }
    }

    pub(super) fn offer_identity_step(&mut self) {
        let step = identity_step(
            self.settings.identity_asked,
            crate::ui::legal::pending(&self.settings).is_some(),
            self.dialogs.any(|d| matches!(d, Dialog::Identity { .. })),
            self.global_identity.as_ref(),
            self.global_identity_error.is_some(),
        );
        match step {
            IdentityStep::Wait => return,
            IdentityStep::Settled => {
                self.settings.identity_asked = true;
                return;
            }
            IdentityStep::Ask => {}
        }
        // The question counts as asked once it is on screen, so dismissing it any way (Escape, a
        // click outside) does not bring it back at every launch.
        self.settings.identity_asked = true;
        self.dialogs.open(Dialog::Identity {
            name: self
                .global_identity
                .as_ref()
                .map_or(String::new(), |i| i.author.name.value.clone()),
            email: self
                .global_identity
                .as_ref()
                .map_or(String::new(), |i| i.author.email.value.clone()),
            pending: false,
        });
    }

    fn poll_identity(&mut self, ctx: &egui::Context) {
        let Some(worker) = &self.identity_worker else {
            return;
        };
        let events: Vec<_> = worker.events.try_iter().collect();
        for event in events {
            match event {
                IdentityEvent::ConfigChanged {
                    repo,
                    recommended,
                    result,
                } => {
                    if recommended && result.is_ok() {
                        self.settings.use_recommended_git_choices();
                    }
                    if let Some(screen) = &mut self.settings_screen {
                        screen.git_config_finished();
                    }
                    match result {
                        Ok(()) => self.toasts.push(ctx, ToastKind::Success, Topic::Confirmation,
                            "Git settings saved", if recommended {
                                "Recommended values were saved globally. Repository overrides remain in force."
                            } else { "The selected Git configuration was updated." }),
                        Err(error) => self.toasts.push(ctx, ToastKind::Error, Topic::Error,
                            "Git settings were not fully saved", error),
                    }
                    let paths: Vec<_> = self
                        .tabs
                        .iter()
                        .filter(|tab| repo.as_ref().is_none_or(|path| tab.path == *path))
                        .map(|tab| {
                            tab.refresh();
                            tab.path.clone()
                        })
                        .collect();
                    for path in paths {
                        self.request_identity(IdentityJob::ReadRepo(path.clone()));
                        self.read_config_for(ctx, &path);
                    }
                    self.request_identity(IdentityJob::ReadGlobal);
                }
                IdentityEvent::Repo { path, result } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.path == path) {
                        match result {
                            Ok(identity) => {
                                tab.identity = Some(identity);
                                tab.identity_error = None;
                            }
                            Err(error) => {
                                tab.identity_error = Some(error);
                                tab.identity = None;
                            }
                        }
                    }
                }
                IdentityEvent::Global(result) => {
                    match result {
                        Ok(identity) => {
                            self.global_identity = Some(identity);
                            self.global_identity_error = None;
                        }
                        Err(error) => {
                            self.global_identity_error = Some(error);
                            self.global_identity = None;
                        }
                    }
                    self.offer_identity_step();
                }
                IdentityEvent::Changed {
                    scope,
                    cleared,
                    result,
                    now,
                } => {
                    settle_identity_dialog(&mut self.dialogs, result.is_ok());
                    let (kind, title, body) = changed_toast(cleared, scope, result, now.as_ref());
                    let topic = if kind == ToastKind::Error {
                        Topic::Error
                    } else {
                        Topic::Confirmation
                    };
                    self.toasts.push(ctx, kind, topic, title, body);
                    for path in self.tabs.iter().map(|t| t.path.clone()).collect::<Vec<_>>() {
                        self.request_identity(IdentityJob::ReadRepo(path));
                    }
                    self.request_identity(IdentityJob::ReadGlobal);
                }
            }
        }
    }

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
                self.toasts.push(
                    ctx,
                    ToastKind::Info,
                    Topic::Attention,
                    "SSH",
                    prompt.text.clone(),
                );
                prompt.answer(Some(String::new()));
                continue;
            }
            // In front of any other dialog; behind a prompt already open.
            self.dialogs.open(Dialog::credential(prompt));
        }
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.toasts.set_prefs(self.settings.notifications);
        self.toasts
            .set_settings_open(self.settings_screen.is_some());
        crate::ui::kit::overlay::set_tooltip_level(self.settings.tooltips);
        keymap::apply_saved(&self.settings.keybindings);
        let delay = self.settings.tooltip_delay.seconds();
        if (ctx.global_style().interaction.tooltip_delay - delay).abs() > f32::EPSILON {
            ctx.all_styles_mut(|s| s.interaction.tooltip_delay = delay);
        }
        // Also a backstop for any path that changed the list without publishing it.
        crate::trust::set(&self.settings.trusted_folders);
        let changes = crate::trust::changes();
        if changes != self.trust_seen {
            self.trust_seen = changes;
            for tab in &mut self.tabs {
                tab.trust_changed(changes);
            }
        }
        let pictures = crate::ui::legal::pending(&self.settings).is_none()
            && self.settings.hosting_api
            && self.settings.download_avatars
            && self.settings.show_avatars;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            tab.avatars.set_enabled(pictures);
            if let Some(host) = &tab.host {
                host.set_avatars_enabled(pictures && index == self.active);
            }
        }
        self.poll_opened(ctx);
        self.poll_config_reads(ctx);
        self.poll_identity(ctx);
        for tab in &mut self.tabs {
            tab.requests.refresh.interval =
                requests_refresh_interval(self.settings.auto_fetch_for(&tab.path));
            for n in tab.poll(&self.gitlab_hosts) {
                let (kind, topic) = if n.ok {
                    (ToastKind::Success, Topic::Finished)
                } else {
                    (ToastKind::Error, Topic::Error)
                };
                self.toasts
                    .push_for(ctx, kind, topic, n.title, n.body, Some(&tab.path));
            }
            if !tab.sidebar_restored
                && let Some(key) = tab.sidebar_key()
            {
                if let Some(closed) = self.settings.repo_sidebar.get(key) {
                    tab.collapsed = closed.iter().cloned().collect();
                }
                tab.sidebar_restored = true;
            }
            if !tab.request_view_restored
                && let Some(key) = tab.sidebar_key()
            {
                if let Some(view) = self.settings.repo_requests.get(key) {
                    tab.request_view = view.clone();
                }
                tab.request_view_restored = true;
            }
            // Taken after the poll that set it, so the dialog shows in this frame.
            if let Some(d) = tab.pull_decision.take() {
                self.dialogs.open(if d.diverged {
                    Dialog::Diverged {
                        repo: tab.path.clone(),
                        mode: settings::PullMode::RECOMMENDED_WHEN_DIVERGED,
                        remember: crate::action::Remember::Repository,
                        everywhere: !d.configured,
                        preview: d.preview,
                    }
                } else {
                    Dialog::PullConflicts {
                        repo: tab.path.clone(),
                        strategy: d.strategy,
                        preview: d.preview,
                        moved: d.moved,
                    }
                });
            }
            if let Some(target) = tab.force_push_offer.take() {
                self.dialogs.open(force_push_dialog(&tab.path, target));
            }
            if let Some(name) = tab.force_delete_offer.take() {
                self.dialogs.open(Dialog::Confirm {
                    title: format!("Force delete branch {name}?"),
                    body: format!(
                        "{name} has commits that are not merged into its upstream or the current branch. Delete it anyway? Those commits stay reachable only through the reflog."
                    ),
                    confirm: "Force delete".into(),
                    danger: true,
                    repo: tab.path.clone(),
                    op: Op::DeleteBranch { name, force: true },
                });
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
            t.avatars.set_enabled(pictures);
            if let Some(host) = &t.host {
                host.set_avatars_enabled(pictures);
            }
        }
        let mut finished = None;
        let mut warning = None;
        if let Some((_, rx, status)) = &mut self.clone {
            loop {
                match rx.try_recv() {
                    Ok(CloneMsg::Progress(l)) => *status = l,
                    Ok(CloneMsg::Done(r)) => finished = Some(r),
                    Ok(CloneMsg::Incomplete(path, message)) => {
                        warning = Some(message);
                        finished = Some(Ok(path));
                    }
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
            self.clone_finished(ctx, r);
            if let Some(message) = warning {
                self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    Topic::Error,
                    "Cloned, but git reported a problem",
                    message,
                );
            }
        }
        self.poll_prompts(ctx);
        if self.updater.poll_install() {
            match &self.updater.install {
                updater::InstallState::Done { build, installed } => {
                    let title = format!("Canopy {} is installed", crate::build_info::name(build));
                    match &installed.exe {
                        Some(exe) => self.toasts.push_offer(
                            ctx,
                            ToastKind::Success,
                            Topic::Attention,
                            title,
                            "Restart Canopy to use it.",
                            Offer::Restart(exe.clone()),
                        ),
                        None => self.toasts.push(
                            ctx,
                            ToastKind::Success,
                            Topic::Attention,
                            title,
                            "Start it from your launcher; this window still runs the old build.",
                        ),
                    }
                }
                updater::InstallState::Failed { error, .. } => self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    Topic::Error,
                    "Could not update Canopy",
                    error.clone(),
                ),
                _ => {}
            }
        }
        if self.updater.poll() {
            let manual = self.updater.manual;
            match &self.updater.state {
                UpdateState::Available(a) => {
                    let v = a.label();
                    let skipped = a.is_skipped(self.settings.skipped_version.as_deref());
                    if manual {
                        self.dialogs.open(Dialog::Update(a.clone()));
                    } else if !skipped {
                        self.toasts.push_offer(
                            ctx,
                            ToastKind::Info,
                            Topic::Update,
                            format!("Canopy {v} is available"),
                            "",
                            Offer::ReviewUpdate,
                        );
                    } else {
                        self.updater.state = UpdateState::UpToDate;
                    }
                }
                UpdateState::UpToDate if manual => self.toasts.push(
                    ctx,
                    ToastKind::Success,
                    Topic::Update,
                    "Canopy is up to date",
                    format!("You have {}", crate::build_info::label()),
                ),
                UpdateState::Failed(e) if manual => self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    Topic::Error,
                    "Could not check for updates",
                    e.clone(),
                ),
                _ => {}
            }
        }
    }

    /// Queues background fetches for tabs whose auto-fetch interval has passed.
    pub(super) fn auto_fetch(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_some() {
            return;
        }
        let mut any = false;
        for t in &mut self.tabs {
            let minutes = self.settings.auto_fetch_for(&t.path);
            if minutes == 0 {
                continue;
            }
            any = true;
            let every = Duration::from_secs(u64::from(minutes) * 60);
            let idle = t.running.is_none();
            let has_remote = t.snap.as_ref().is_some_and(|s| !s.remotes.is_empty());
            if idle && has_remote && t.last_fetch.elapsed() >= every {
                t.last_fetch = Instant::now();
                let prune = self.settings.prune_for(&t.path);
                t.op(Op::Fetch { prune, quiet: true });
            }
        }
        if any {
            ctx.request_repaint_after(Duration::from_secs(30));
        }
    }

    /// Refreshes the active tab's open request list on a timer: as often as the background
    /// fetch runs, or every minute when that is off. Wakes the frame loop only when it falls due.
    pub(super) fn auto_refresh_requests(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_some() {
            return;
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        let interval = requests_refresh_interval(self.settings.auto_fetch_for(&t.path));
        if let Some(next) = t.refresh_requests_on_timer(Instant::now(), interval, focused) {
            ctx.request_repaint_after(next);
        }
    }
}

/// How often the open request list refreshes when the background fetch runs every `minutes`
/// (0: off, so every [`REQUESTS_REFRESH_EVERY`]).
fn requests_refresh_interval(minutes: u32) -> Duration {
    match minutes {
        0 => REQUESTS_REFRESH_EVERY,
        minutes => Duration::from_secs(u64::from(minutes) * 60),
    }
}

#[cfg(test)]
mod tests {
    use canopy_core::identity::{Field, Person};

    use super::*;

    fn identity(name: &str, email: &str, explicit: bool) -> Identity {
        let field = |value: &str| Field {
            value: value.into(),
            source: Some(if explicit {
                Source::Config {
                    scope: "global".into(),
                    origin: "file:/home/ada/.gitconfig".into(),
                }
            } else {
                Source::Guessed
            }),
        };
        let person = Person {
            name: field(name),
            email: field(email),
            usable: true,
        };
        Identity {
            author: person.clone(),
            committer: person,
        }
    }

    #[test]
    fn the_request_list_refreshes_with_the_background_fetch_or_every_minute() {
        for (minutes, secs) in [(0, 60), (1, 60), (5, 300), (30, 1_800)] {
            assert_eq!(
                requests_refresh_interval(minutes),
                Duration::from_secs(secs),
                "{minutes} minutes"
            );
        }
        assert_eq!(requests_refresh_interval(0), REQUESTS_REFRESH_EVERY);
    }

    #[test]
    fn a_clear_says_what_git_uses_afterwards() {
        let global = identity("Ada Lovelace", "ada@example.com", true);
        let (kind, title, body) = changed_toast(true, Scope::Local, Ok(()), Some(&global));
        assert_eq!(kind, ToastKind::Success);
        assert_eq!(title, "Git identity cleared");
        assert!(
            body.starts_with("Git now uses Ada Lovelace <ada@example.com>"),
            "{body}"
        );

        // The cleared scope still supplies a value (an included file): say so.
        let mut included = global.clone();
        included.author.name.source = Some(Source::Config {
            scope: "local".into(),
            origin: "file:/work/extra.inc".into(),
        });
        let (_, _, body) = changed_toast(true, Scope::Local, Ok(()), Some(&included));
        assert!(body.starts_with("Some of it is still set in"), "{body}");
        assert!(body.contains("/work/extra.inc"), "{body}");

        let mut unusable = global;
        unusable.author.usable = false;
        unusable.author.name.source = None;
        unusable.author.email.source = None;
        let (_, _, body) = changed_toast(true, Scope::Global, Ok(()), Some(&unusable));
        assert_eq!(body, "Git has no usable identity now.");

        let (_, _, body) = changed_toast(true, Scope::Global, Ok(()), None);
        assert!(body.contains("Settings"), "{body}");
    }

    #[test]
    fn a_finished_save_is_reported_and_settles_the_dialog() {
        let (kind, title, _) = changed_toast(false, Scope::Global, Ok(()), None);
        assert_eq!((kind, title), (ToastKind::Success, "Git identity saved"));
        let (kind, title, body) =
            changed_toast(false, Scope::Global, Err("config.lock exists".into()), None);
        assert_eq!(kind, ToastKind::Error);
        assert_eq!(title, "Could not change Git identity");
        assert_eq!(body, "config.lock exists", "git's own message is shown");

        let open = |pending| Dialog::Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            pending,
        };
        let mut dialogs = DialogStack::default();
        dialogs.open(open(true));
        settle_identity_dialog(&mut dialogs, false);
        assert!(
            matches!(
                dialogs.current_mut(),
                Some(Dialog::Identity { pending: false, .. })
            ),
            "a refused save keeps the dialog open and usable"
        );
        dialogs.close_where(|_| true);
        dialogs.open(open(true));
        settle_identity_dialog(&mut dialogs, true);
        assert!(
            !dialogs.any(|d| matches!(d, Dialog::Identity { .. })),
            "a saved identity closes it"
        );
    }

    #[test]
    fn the_first_launch_question_waits_for_everything_it_depends_on() {
        let good = identity("Ada Lovelace", "ada@example.com", true);
        let placeholder = identity("t", "t@t", true);
        let guessed = identity("ada", "ada@host.example", false);
        let step = identity_step;

        // Asked, or another question on screen: never ask again, change nothing.
        assert_eq!(
            step(true, false, false, Some(&placeholder), false),
            IdentityStep::Wait
        );
        assert_eq!(
            step(false, true, false, Some(&placeholder), false),
            IdentityStep::Wait
        );
        assert_eq!(
            step(false, false, true, Some(&placeholder), false),
            IdentityStep::Wait
        );
        // The global identity has not been read yet.
        assert_eq!(step(false, false, false, None, false), IdentityStep::Wait);
        // A sensible identity needs no question; a placeholder, a guess or an unreadable
        // configuration does.
        assert_eq!(
            step(false, false, false, Some(&good), false),
            IdentityStep::Settled
        );
        assert_eq!(
            step(false, false, false, Some(&placeholder), false),
            IdentityStep::Ask
        );
        assert_eq!(
            step(false, false, false, Some(&guessed), false),
            IdentityStep::Ask
        );
        assert_eq!(step(false, false, false, None, true), IdentityStep::Ask);
    }
}
