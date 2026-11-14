//! Work polled every frame: worker and hosting events, credential prompts, the update check
//! and auto-fetch.

use super::*;
use std::path::Path;

use canopy_core::identity::{Scope, Source};

use crate::identity_worker::Event as IdentityEvent;
use crate::tab::{Delivery, Notice, REQUESTS_REFRESH_EVERY, RepoTrust};
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

/// The kind and the filter topic a repository [`Notice`](crate::tab::Notice) is announced under.
fn notice_route(ok: bool, delivery: Delivery) -> (ToastKind, Topic) {
    match (ok, delivery) {
        (true, _) => (ToastKind::Success, Topic::Finished),
        (false, Delivery::Attention) => (ToastKind::Error, Topic::Attention),
        (false, Delivery::Card | Delivery::Quiet) => (ToastKind::Error, Topic::Error),
    }
}

/// Announces a repository notice under the topic and delivery it carries.
fn announce_notice(toasts: &mut Toasts, ctx: &egui::Context, repo: &Path, n: Notice) {
    let (kind, topic) = notice_route(n.ok, n.delivery);
    if n.delivery == Delivery::Quiet {
        toasts.push_quiet_for(ctx, kind, topic, n.title, n.body, repo);
    } else {
        toasts.push_for(ctx, kind, topic, n.title, n.body, Some(repo));
    }
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
fn force_push_dialog(repo: &Path, target: canopy_core::PushTarget) -> Dialog {
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
    /// Whether Settings may write the git configuration (Edit my git config); when not, says so
    /// and settles the screen, which never offers a write then.
    pub(super) fn may_edit_git_config(&mut self, ctx: &egui::Context) -> bool {
        if self.settings.edit_git_config {
            return true;
        }
        if let Some(screen) = &mut self.settings_screen {
            screen.git_config_finished();
        }
        self.toasts.push(
            ctx,
            ToastKind::Error,
            Topic::Error,
            "Git settings were not saved",
            "Edit my git config is off in Settings → Git.",
        );
        false
    }

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
                    // A tab that has not loaded reads all of this when it first does.
                    let paths: Vec<_> = self
                        .tabs
                        .iter()
                        .filter(|tab| tab.is_loaded())
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
                    let loaded: Vec<_> = self
                        .tabs
                        .iter()
                        .filter(|t| t.is_loaded())
                        .map(|t| t.path.clone())
                        .collect();
                    for path in loaded {
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
        self.load_active_tab(ctx);
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
        crate::trust::publish(&self.settings);
        let changes = crate::trust::changes();
        if changes != self.trust_seen {
            self.trust_seen = changes;
            for tab in &mut self.tabs {
                tab.trust_changed(changes);
            }
        }
        self.ask_about_trust();
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
        self.poll_leftover_reads(ctx);
        self.poll_swept(ctx);
        self.poll_identity(ctx);
        for tab in &mut self.tabs {
            tab.requests.refresh.interval =
                requests_refresh_interval(self.settings.auto_fetch_for(&tab.path));
            for n in tab.poll(&self.gitlab_hosts) {
                announce_notice(&mut self.toasts, ctx, &tab.path, n);
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
                    tab.request_view_changed();
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
            if let Some(result) = tab.worktree_result.take() {
                let shown = self
                    .dialogs
                    .finish_worktree(result.as_ref().map_err(String::as_str).copied());
                if let Err(e) = result
                    && !shown
                {
                    self.toasts.push_for(
                        ctx,
                        ToastKind::Error,
                        Topic::Error,
                        "Could not add the worktree".to_owned(),
                        e,
                        Some(&tab.path),
                    );
                }
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
            t.ensure_host(ctx, self.settings.hosting_api);
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

    /// Asks whether to trust the active tab's repository, once, as soon as its thread says it is
    /// not trusted. Other tabs ask when they are first active; each is read-only meanwhile, with
    /// the banner. A prompt for a repository that has become trusted since (a parent folder was
    /// added in Settings, say) goes away.
    pub(super) fn ask_about_trust(&mut self) {
        let tabs = &self.tabs;
        self.dialogs.close_where(|dialog| {
            matches!(dialog, Dialog::Trust { folders, .. }
                if tabs.iter().any(|t| t.trust == RepoTrust::Trusted && t.trust_folders == *folders))
        });
        if self
            .dialogs
            .any(|dialog| matches!(dialog, Dialog::Trust { .. }))
        {
            return;
        }
        let declines = self.screenshot.as_ref().is_some_and(|s| s.declines_trust);
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.trust != RepoTrust::Untrusted || tab.trust_asked {
            return;
        }
        tab.trust_asked = true;
        if !declines {
            self.dialogs.open(Dialog::Trust {
                repo: tab.trust_folders.first().unwrap_or(&tab.path).clone(),
                folders: tab.trust_folders.clone(),
            });
        }
    }

    /// Queues background fetches for tabs whose auto-fetch fell due. A repository that is not
    /// trusted is read-only, so it is not fetched.
    ///
    /// Each tab has its own due time (`RepoTab::next_fetch`). A tab is scheduled the first time
    /// this sees auto-fetch on for it, and again when the interval changes, with a delay from
    /// [`first_fetch_delay`] that depends only on its path: tabs scheduled together (a restored
    /// session, or the setting turned on with many tabs open) spread their first fetches over the
    /// second half of the interval instead of falling due in one frame. After a fetch the tab
    /// waits one whole interval.
    ///
    /// At most [`MAX_BACKGROUND_FETCHES`] of these fetches run at once across all tabs, so the
    /// processes, connections and disk reads they cost do not grow with the number of tabs. A tab
    /// that falls due while they are all taken waits, and starts when one finishes: the longest
    /// waiting first. A fetch the user starts is not counted and never waits.
    pub(super) fn auto_fetch(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_some() {
            return;
        }
        if let Some(wake) = self.auto_fetch_at(Instant::now()) {
            ctx.request_repaint_after(wake);
        }
    }

    /// [`Self::auto_fetch`] at `now`; the time until the frame loop should look again, or `None`
    /// when no tab has auto-fetch on.
    fn auto_fetch_at(&mut self, now: Instant) -> Option<Duration> {
        let mut any = false;
        let mut wake = AUTO_FETCH_POLL;
        // Tabs whose fetch is due, with when it fell due.
        let mut due_now: Vec<(Instant, usize)> = Vec::new();
        for (index, t) in self.tabs.iter_mut().enumerate() {
            let minutes = self.settings.auto_fetch_for(&t.path);
            if minutes == 0 {
                t.next_fetch = None;
                t.fetch_every = None;
                continue;
            }
            any = true;
            let every = Duration::from_secs(u64::from(minutes) * 60);
            if t.fetch_every != Some(every) {
                t.fetch_every = Some(every);
                t.next_fetch = None;
            }
            let due = *t.next_fetch.get_or_insert_with(|| {
                now.checked_add(first_fetch_delay(&t.path, every))
                    .unwrap_or(now)
            });
            let idle = t.running.is_none() && t.trust != RepoTrust::Untrusted;
            let has_remote = t.snap.as_ref().is_some_and(|s| !s.remotes.is_empty());
            if !idle || !has_remote {
                // Busy, untrusted and not yet loaded tabs are looked at again by the poll, not
                // by their own due time.
                continue;
            }
            if now >= due {
                due_now.push((due, index));
            } else {
                wake = wake.min(next_due(now, due));
            }
        }
        // The tabs that have waited longest go first, and a tab the limit holds back keeps its
        // due time: the next pass starts it once a slot is free. No wake time is set for it, since
        // it is due already; a running fetch ends with an event that repaints, and the poll finds
        // it at the latest.
        let free = MAX_BACKGROUND_FETCHES
            .saturating_sub(self.tabs.iter().filter(|t| t.auto_fetch_pending).count());
        due_now.sort_by_key(|(due, _)| *due);
        if due_now.len() > free {
            tracing::debug!(
                due = due_now.len(),
                started = free,
                "background fetches limited; the rest wait for a slot"
            );
        }
        for (_, index) in due_now.into_iter().take(free) {
            let prune = self.settings.prune_for(&self.tabs[index].path);
            let t = &mut self.tabs[index];
            t.next_fetch = t.fetch_every.and_then(|every| now.checked_add(every));
            t.auto_fetch_pending = true;
            t.op(Op::Fetch { prune, quiet: true });
        }
        any.then_some(wake)
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

/// How many automatic fetches run at once across all tabs. A fetch waits on the network and the
/// server, not on this machine's cores, so the number does not follow them. Four is enough to
/// overlap the waiting, so a long session's fetches finish in a few rounds, and keeps the `git`
/// and `ssh` processes, connections and pack indexing that background fetching causes to a
/// handful however many tabs are open. A first estimate: the stress run (docs/stress-test.md,
/// scenario d) is where to tune it.
const MAX_BACKGROUND_FETCHES: usize = 4;

/// The longest the frame loop sleeps between looks at auto-fetch. A tab that is busy, untrusted
/// or not loaded yet has no wake time of its own and is found by this poll.
const AUTO_FETCH_POLL: Duration = Duration::from_secs(30);

/// How long after a tab is scheduled its first automatic fetch falls due: from half the interval
/// to just under a whole one, by a hash of the tab's path. The same path always gets the same
/// delay, so closing, reordering or opening other tabs moves nothing, and a lone tab never waits
/// longer than the interval.
fn first_fetch_delay(path: &Path, every: Duration) -> Duration {
    // FNV-1a: fixed by definition, unlike the standard library's default hasher.
    let hash = path
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(FNV_OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        });
    let step = u32::try_from(hash % u64::from(STAGGER_STEPS)).unwrap_or(0);
    let half = every / 2;
    half + (half / STAGGER_STEPS) * step
}

/// FNV-1a 64-bit parameters.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;
/// How many distinct delays the second half of the interval is divided into.
const STAGGER_STEPS: u32 = 1_024;

/// How long the frame loop may sleep for a tab that falls due at `due`: until then, never past
/// the poll.
fn next_due(now: Instant, due: Instant) -> Duration {
    due.saturating_duration_since(now).min(AUTO_FETCH_POLL)
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
    use crate::settings::{DiffPrefs, NotificationPrefs};
    use crate::worker::OpKind;
    use crossbeam_channel::Sender;

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

    /// #234: what waits on the user is an attention notice, a failure is an error, a success is
    /// finished work, whatever its delivery.
    #[test]
    fn notices_are_filed_under_the_topic_that_fits() {
        assert_eq!(
            notice_route(true, Delivery::Card),
            (ToastKind::Success, Topic::Finished)
        );
        assert_eq!(
            notice_route(false, Delivery::Attention),
            (ToastKind::Error, Topic::Attention)
        );
        for delivery in [Delivery::Card, Delivery::Quiet] {
            assert_eq!(
                notice_route(false, delivery),
                (ToastKind::Error, Topic::Error)
            );
        }
    }

    /// #35: a real fetch result goes through `poll` and the card filter. A foreground success is
    /// a Finished notice that follows its switch and always reaches the center; a timer-driven
    /// success announces nothing at all, whatever the switch says.
    #[test]
    fn a_fetch_result_follows_the_finished_filter() {
        use crate::worker::{Event, OpKind, Worker};
        let ctx = egui::Context::default();
        let repo = PathBuf::from("/work/mote");
        let fetched = |announce| Event::OpFinished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            announce,
            result: Ok("Nothing new on any remote.".into()),
        };
        for finished in [false, true] {
            let (worker, events, _jobs) = Worker::detached();
            let mut tab = RepoTab::with_worker(repo.clone(), 100, DiffPrefs::default(), worker);
            let mut toasts = Toasts::default();
            toasts.set_prefs(NotificationPrefs {
                finished,
                ..NotificationPrefs::default()
            });

            events.send(fetched(false)).expect("background fetch");
            for n in tab.poll(&[]) {
                announce_notice(&mut toasts, &ctx, &repo, n);
            }
            assert_eq!(
                toasts.counts(),
                (0, 0),
                "a quiet success is announced nowhere"
            );

            events.send(fetched(true)).expect("foreground fetch");
            for n in tab.poll(&[]) {
                announce_notice(&mut toasts, &ctx, &repo, n);
            }
            assert_eq!(
                toasts.counts(),
                (usize::from(finished), 1),
                "Finished operations {finished}: a card only when on, the center either way"
            );
        }
    }

    /// A tab on a repository that is not trusted, on a worker that answers nothing.
    fn untrusted_tab(name: &str) -> RepoTab {
        let mut tab = test_tab(canopy_core::RepoSnapshot::default());
        tab.path = PathBuf::from(name);
        tab.trust = RepoTrust::Untrusted;
        tab.trust_folders = vec![PathBuf::from(name)];
        tab
    }

    fn trust_prompt(app: &CanopyApp) -> Option<Vec<PathBuf>> {
        match app.dialogs.current() {
            Some(Dialog::Trust { folders, .. }) => Some(folders.clone()),
            _ => None,
        }
    }

    /// A write to the git configuration that reaches the app while Edit my git config is off
    /// (off by default) is refused with a notice; once it is on, it goes ahead.
    #[test]
    fn git_configuration_writes_wait_for_edit_my_git_config() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        assert!(!app.settings.edit_git_config);
        let (_, before) = app.toasts.counts();
        assert!(!app.may_edit_git_config(&ctx));
        assert_eq!(app.toasts.counts().1, before + 1, "the refusal is reported");
        app.settings.edit_git_config = true;
        assert!(app.may_edit_git_config(&ctx));
        assert_eq!(app.toasts.counts().1, before + 1);
    }

    /// #228: the active tab is asked once; the others are read-only until they are active, and a
    /// tab that was asked and declined is not asked again.
    #[test]
    fn only_the_active_untrusted_tab_is_asked_about_trust_and_only_once() {
        let mut app = test_app();
        app.tabs = vec![untrusted_tab("/src/one"), untrusted_tab("/src/two")];
        app.ask_about_trust();
        assert_eq!(trust_prompt(&app), Some(vec![PathBuf::from("/src/one")]));
        assert!(app.tabs[0].trust_asked && !app.tabs[1].trust_asked);

        app.ask_about_trust();
        assert!(app.dialogs.any(|d| matches!(d, Dialog::Trust { .. })));
        app.dialogs.close();
        app.ask_about_trust();
        assert!(
            trust_prompt(&app).is_none(),
            "declined: the banner stays, no prompt"
        );

        app.active = 1;
        app.ask_about_trust();
        assert_eq!(trust_prompt(&app), Some(vec![PathBuf::from("/src/two")]));
    }

    #[test]
    fn nothing_is_asked_while_the_thread_has_not_answered_or_the_repository_is_trusted() {
        let mut app = test_app();
        let mut waiting = untrusted_tab("/src/one");
        waiting.trust = RepoTrust::Unknown;
        let mut trusted = untrusted_tab("/src/two");
        trusted.trust = RepoTrust::Trusted;
        app.tabs = vec![waiting, trusted];
        app.ask_about_trust();
        assert!(trust_prompt(&app).is_none());
        app.active = 1;
        app.ask_about_trust();
        assert!(trust_prompt(&app).is_none());
        assert!(!app.tabs[0].trust_asked && !app.tabs[1].trust_asked);
    }

    /// A prompt that waits behind another dialog goes away once its repository is trusted by
    /// other means (a parent folder added in Settings).
    #[test]
    fn a_prompt_goes_away_when_its_repository_becomes_trusted() {
        let mut app = test_app();
        app.tabs = vec![untrusted_tab("/src/one")];
        app.ask_about_trust();
        assert!(trust_prompt(&app).is_some());
        app.tabs[0].trust = RepoTrust::Trusted;
        app.ask_about_trust();
        assert!(app.dialogs.current().is_none());
    }

    /// Background fetches are not sent to a repository that is read-only.
    #[test]
    fn a_background_fetch_skips_a_repository_that_is_not_trusted() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.settings.auto_fetch_minutes = 1;
        let mut tabs = Vec::new();
        let mut queues = Vec::new();
        for trust in [RepoTrust::Untrusted, RepoTrust::Trusted] {
            let (worker, _events, jobs) = crate::worker::Worker::detached();
            let snap = canopy_core::RepoSnapshot {
                remotes: vec![canopy_core::RemoteInfo {
                    name: "origin".into(),
                    url: None,
                    default_branch: None,
                }],
                ..canopy_core::RepoSnapshot::default()
            };
            let mut tab = test_tab(snap);
            tab.worker = worker;
            tab.trust = trust;
            // Scheduled for the interval Settings has, and long overdue.
            tab.fetch_every = Some(Duration::from_mins(1));
            tab.next_fetch = Some(
                Instant::now()
                    .checked_sub(Duration::from_hours(1))
                    .expect("an hour ago"),
            );
            tabs.push(tab);
            queues.push(jobs);
        }
        app.tabs = tabs;
        app.auto_fetch(&ctx);
        assert!(queues[0].is_empty(), "read-only: no fetch");
        assert!(
            queues[1]
                .try_iter()
                .any(|job| matches!(job, Job::Op(Op::Fetch { quiet: true, .. }))),
            "trusted: fetched"
        );
    }

    fn tab_with_remote(path: &str) -> (RepoTab, Receiver<Job>) {
        let (tab, jobs, _events) = tab_with_remote_events(path);
        (tab, jobs)
    }

    /// [`tab_with_remote`], and the sender for the events its worker would send.
    fn tab_with_remote_events(
        path: &str,
    ) -> (RepoTab, Receiver<Job>, Sender<crate::worker::Event>) {
        let (worker, events, jobs) = crate::worker::Worker::detached();
        let snap = canopy_core::RepoSnapshot {
            remotes: vec![canopy_core::RemoteInfo {
                name: "origin".into(),
                url: None,
                default_branch: None,
            }],
            ..canopy_core::RepoSnapshot::default()
        };
        let mut tab = test_tab(snap);
        tab.path = path.into();
        tab.worker = worker;
        (tab, jobs, events)
    }

    fn fetched(queue: &Receiver<Job>) -> bool {
        queue
            .try_iter()
            .any(|job| matches!(job, Job::Op(Op::Fetch { quiet: true, .. })))
    }

    /// Tabs with auto-fetch on, a hundred repositories under `/repos`.
    fn app_with_tabs(count: usize, minutes: u32) -> (CanopyApp, Vec<Receiver<Job>>) {
        let mut app = test_app();
        app.settings.auto_fetch_minutes = minutes;
        let mut queues = Vec::new();
        for n in 0..count {
            let (tab, jobs) = tab_with_remote(&format!("/repos/project-{n:03}"));
            app.tabs.push(tab);
            queues.push(jobs);
        }
        (app, queues)
    }

    /// `count` tabs with a remote, all overdue for their automatic fetch, the first one the most.
    fn overdue_tabs(
        count: usize,
        now: Instant,
    ) -> (
        CanopyApp,
        Vec<Receiver<Job>>,
        Vec<Sender<crate::worker::Event>>,
    ) {
        let mut app = test_app();
        app.settings.auto_fetch_minutes = 5;
        let (mut queues, mut senders) = (Vec::new(), Vec::new());
        for n in 0..count {
            let (mut tab, jobs, events) = tab_with_remote_events(&format!("/repos/project-{n:03}"));
            tab.fetch_every = Some(Duration::from_mins(5));
            let overdue = Duration::from_secs(u64::try_from(count - n).expect("small") * 10);
            tab.next_fetch = Some(now.checked_sub(overdue).expect("in the past"));
            app.tabs.push(tab);
            queues.push(jobs);
            senders.push(events);
        }
        (app, queues, senders)
    }

    fn finish_fetch(
        app: &mut CanopyApp,
        senders: &[Sender<crate::worker::Event>],
        index: usize,
        announce: bool,
    ) {
        senders[index]
            .send(crate::worker::Event::OpFinished {
                label: "Fetching".into(),
                kind: OpKind::Fetch,
                announce,
                result: Ok(String::new()),
            })
            .expect("the tab listens");
        app.tabs[index].poll(&[]);
    }

    /// #62: with many tabs due, the background fetch starts only up to its limit, the tabs it
    /// skips keep their due time, and the frame loop is not asked to wake at once for them.
    #[test]
    fn auto_fetch_starts_at_most_the_limit_and_leaves_the_rest_due() {
        let now = Instant::now();
        let (mut app, queues, _senders) = overdue_tabs(10, now);
        let due: Vec<_> = app.tabs.iter().map(|t| t.next_fetch).collect();
        let wake = app.auto_fetch_at(now).expect("auto-fetch is on");
        let started: Vec<_> = queues.iter().map(fetched).collect();
        assert_eq!(
            started.iter().filter(|s| **s).count(),
            MAX_BACKGROUND_FETCHES,
            "{started:?}"
        );
        assert!(
            started[..MAX_BACKGROUND_FETCHES].iter().all(|s| *s),
            "the tabs that waited longest start first: {started:?}"
        );
        for (n, tab) in app.tabs.iter().enumerate().skip(MAX_BACKGROUND_FETCHES) {
            assert_eq!(tab.next_fetch, due[n], "tab {n} keeps its due time");
            assert!(!tab.auto_fetch_pending);
        }
        assert_eq!(
            wake, AUTO_FETCH_POLL,
            "tabs that are overdue but waiting for a slot do not wake the loop at once"
        );
        // Asking again changes nothing while those fetches run.
        app.auto_fetch_at(now + Duration::from_secs(1));
        assert!(
            queues.iter().all(Receiver::is_empty),
            "no fifth fetch started"
        );
    }

    /// #62: a finished fetch frees a slot for the tab that has waited longest, and every tab
    /// gets its fetch, never more than the limit at once.
    #[test]
    fn a_finished_background_fetch_frees_a_slot_for_the_next_tab() {
        let now = Instant::now();
        let (mut app, queues, senders) = overdue_tabs(10, now);
        app.auto_fetch_at(now);
        let mut started: Vec<usize> = (0..10).filter(|n| fetched(&queues[*n])).collect();
        assert_eq!(started, [0, 1, 2, 3]);

        // A fetch the user ran by hand and finished does not free a slot of the timer's.
        finish_fetch(&mut app, &senders, 0, true);
        app.auto_fetch_at(now);
        assert!(queues.iter().all(Receiver::is_empty), "still four running");
        finish_fetch(&mut app, &senders, 0, false);
        finish_fetch(&mut app, &senders, 2, false);
        app.auto_fetch_at(now);
        let next: Vec<usize> = (0..10).filter(|n| fetched(&queues[*n])).collect();
        assert_eq!(next, [4, 5], "two slots, the two longest waiting");
        started.extend(next);

        // The rest follow one by one until every tab has fetched once.
        let mut running = vec![1, 3, 4, 5];
        while started.len() < 10 {
            let done = running.remove(0);
            finish_fetch(&mut app, &senders, done, false);
            app.auto_fetch_at(now);
            let begun: Vec<usize> = (0..10).filter(|n| fetched(&queues[*n])).collect();
            assert_eq!(begun.len(), 1, "one slot, one fetch");
            running.extend(&begun);
            started.extend(begun);
            assert!(running.len() <= MAX_BACKGROUND_FETCHES);
        }
        started.sort_unstable();
        assert_eq!(
            started,
            (0..10).collect::<Vec<_>>(),
            "each tab fetched once"
        );
    }

    /// #62: a fetch the worker never ran (the repository would not open) does not hold its slot.
    #[test]
    fn a_failure_the_worker_reports_frees_the_fetch_slot() {
        let now = Instant::now();
        let (mut app, queues, senders) = overdue_tabs(6, now);
        app.auto_fetch_at(now);
        assert_eq!(queues.iter().filter(|q| fetched(q)).count(), 4);
        senders[1]
            .send(crate::worker::Event::Failed("cannot open".into()))
            .expect("the tab listens");
        app.tabs[1].poll(&[]);
        assert!(!app.tabs[1].auto_fetch_pending);
        app.auto_fetch_at(now);
        assert_eq!(queues.iter().filter(|q| fetched(q)).count(), 1);
    }

    #[test]
    fn the_first_fetch_delay_is_the_second_half_of_the_interval_by_path() {
        let every = Duration::from_mins(10);
        let delay = |path: &str| first_fetch_delay(Path::new(path), every);
        assert_eq!(
            delay("/repos/a"),
            delay("/repos/a"),
            "the same path, the same delay"
        );
        let delays: Vec<Duration> = (0..100)
            .map(|n| delay(&format!("/repos/project-{n:03}")))
            .collect();
        assert!(
            delays.iter().all(|d| *d >= every / 2 && *d < every),
            "never sooner than half the interval, never a whole one: {delays:?}"
        );
        let distinct: std::collections::BTreeSet<_> = delays.iter().collect();
        assert!(
            distinct.len() >= 90,
            "{} distinct delays of 100",
            distinct.len()
        );
        assert!(
            delays.iter().filter(|d| **d < every * 3 / 4).count() >= 30
                && delays.iter().filter(|d| **d >= every * 3 / 4).count() >= 30,
            "spread over the half, not bunched"
        );
        assert_eq!(
            first_fetch_delay(Path::new("/x"), Duration::ZERO),
            Duration::ZERO
        );
    }

    #[test]
    fn the_wake_time_is_the_due_time_but_never_past_the_poll() {
        let now = Instant::now();
        assert_eq!(
            next_due(now, now + Duration::from_secs(7)),
            Duration::from_secs(7)
        );
        assert_eq!(next_due(now, now + Duration::from_mins(5)), AUTO_FETCH_POLL);
        assert_eq!(next_due(now, now), Duration::ZERO, "due now");
        assert_eq!(
            next_due(now + Duration::from_secs(5), now),
            Duration::ZERO,
            "already past"
        );
    }

    /// Turning auto-fetch on, or shortening its interval, with many tabs open for an hour does not
    /// fetch them all in one frame, nor at once after the interval: they fall due one after another.
    #[test]
    fn enabling_auto_fetch_spreads_tabs_that_have_been_open_for_a_while() {
        let (mut app, queues) = app_with_tabs(40, 0);
        let opened = Instant::now();
        assert_eq!(app.auto_fetch_at(opened), None, "off: nothing scheduled");
        assert!(app.tabs.iter().all(|t| t.next_fetch.is_none()));

        // An hour later the user turns it on, every five minutes.
        let on = opened + Duration::from_hours(1);
        app.settings.auto_fetch_minutes = 5;
        assert!(app.auto_fetch_at(on).is_some());
        assert!(
            queues.iter().all(|q| !fetched(q)),
            "the pass that turns it on fetches nothing"
        );
        // Each tab's step is a share of the second half of the interval.
        let every = Duration::from_mins(5);
        let mut fired = Vec::new();
        for second in (0..=300).step_by(5) {
            app.auto_fetch_at(on + Duration::from_secs(second));
            let now_fired = queues.iter().filter(|q| fetched(q)).count();
            fired.push(now_fired);
            // Each fetch is over before the next step (the limit on running ones is tested apart).
            for tab in &mut app.tabs {
                tab.auto_fetch_pending = false;
            }
        }
        assert_eq!(fired.iter().sum::<usize>(), 40, "every tab fetched once");
        assert_eq!(
            fired[..30].iter().sum::<usize>(),
            0,
            "nothing before half the interval"
        );
        assert!(
            fired.iter().all(|n| *n <= 8),
            "no more than a few in one five-second step: {fired:?}"
        );
        assert!(
            app.tabs
                .iter()
                .all(|t| t.next_fetch.is_some_and(|n| n > on + every)),
            "after a fetch a tab waits a whole interval"
        );
    }

    #[test]
    fn changing_the_interval_or_turning_it_off_reschedules() {
        let (mut app, queues) = app_with_tabs(3, 10);
        let t0 = Instant::now();
        app.auto_fetch_at(t0);
        let scheduled: Vec<_> = app.tabs.iter().map(|t| t.next_fetch).collect();
        assert!(scheduled.iter().all(Option::is_some));
        app.auto_fetch_at(t0 + Duration::from_secs(1));
        assert_eq!(
            scheduled,
            app.tabs.iter().map(|t| t.next_fetch).collect::<Vec<_>>(),
            "the schedule stays while nothing changes"
        );

        // Shortened to one minute: scheduled again from now, within the new interval.
        let later = t0 + Duration::from_mins(2);
        app.settings.auto_fetch_minutes = 1;
        app.auto_fetch_at(later);
        for t in &app.tabs {
            let due = t.next_fetch.expect("scheduled");
            assert!(due >= later + Duration::from_secs(30) && due < later + Duration::from_mins(1));
            assert_eq!(t.fetch_every, Some(Duration::from_mins(1)));
        }
        assert!(
            queues.iter().all(|q| !fetched(q)),
            "a new schedule fetches nothing at once"
        );

        app.settings.auto_fetch_minutes = 0;
        assert_eq!(app.auto_fetch_at(later), None);
        assert!(
            app.tabs
                .iter()
                .all(|t| t.next_fetch.is_none() && t.fetch_every.is_none())
        );
    }

    #[test]
    fn closing_reordering_or_opening_tabs_does_not_move_the_others() {
        let (mut app, _queues) = app_with_tabs(6, 10);
        let t0 = Instant::now();
        app.auto_fetch_at(t0);
        let due = |app: &CanopyApp| -> Vec<(PathBuf, Option<Instant>)> {
            let mut v: Vec<_> = app
                .tabs
                .iter()
                .map(|t| (t.path.clone(), t.next_fetch))
                .collect();
            v.sort();
            v
        };
        let before = due(&app);

        app.tabs.remove(2);
        app.tabs.reverse();
        let (lone, _jobs) = tab_with_remote("/repos/opened-later");
        app.tabs.push(lone);
        app.auto_fetch_at(t0 + Duration::from_secs(1));
        let after = due(&app);
        for (path, due) in &before {
            if let Some((_, now)) = after.iter().find(|(p, _)| p == path) {
                assert_eq!(due, now, "{path:?} kept its time");
            }
        }
        // A tab opened ten minutes in waits like a lone tab: under one interval, not two.
        let late = t0 + Duration::from_mins(10);
        let (tab, _jobs) = tab_with_remote("/repos/opened-much-later");
        app.tabs.push(tab);
        app.auto_fetch_at(late);
        let due = app
            .tabs
            .last()
            .and_then(|t| t.next_fetch)
            .expect("scheduled");
        assert!(due >= late + Duration::from_mins(5) && due < late + Duration::from_mins(10));
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
