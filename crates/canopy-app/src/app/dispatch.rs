//! Applying the actions a frame collected.

use super::*;
use crate::action::Remember;
use crate::system_apps::Kind;

/// The question before amending a commit its upstream already has: the amend rewrites published
/// history, so the branch ends up diverged and its next push must be forced.
fn amend_pushed_dialog(tab: &RepoTab, op: Op) -> Option<Dialog> {
    let snap = tab.snap.as_ref()?;
    let branch = snap.branches.iter().find(|b| b.is_head)?;
    let upstream = branch.upstream.as_deref().filter(|_| branch.ahead == 0)?;
    Some(Dialog::Confirm {
        title: format!("Amend {}, already on {upstream}?", branch.name),
        body: format!(
            "{upstream} has this commit. Amending rewrites it, so {} and {upstream} diverge and \
             the next push must be a force push (--force-with-lease).",
            branch.name
        ),
        confirm: "Amend commit".into(),
        danger: true,
        repo: tab.path.clone(),
        op,
    })
}

impl CanopyApp {
    pub(super) fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::CleanStale => {
                    let days = self.settings.stale_inactivity_days;
                    if let Some(tab) = self.tab() {
                        let serial = tab.read_stale(days);
                        let form = dialogs::StaleForm::new(tab.path.clone(), days, serial);
                        self.dialogs.open(Dialog::CleanStale(Box::new(form)));
                    }
                }
                Action::CleanMerged => {
                    if let Some(tab) = self.tab() {
                        let serial = tab.read_merged(true);
                        let form = dialogs::MergedForm::new(tab.path.clone(), serial);
                        self.dialogs.open(Dialog::CleanMerged(Box::new(form)));
                    }
                }
                Action::UpdateFromBase => {
                    if let Some(tab) = self.tab() {
                        let base = tab.snap.as_ref().and_then(|s| s.workflow.head_base.clone());
                        if let Some(base) = base {
                            let serial = tab.read_base_compare(&base);
                            let form = dialogs::UpdateBaseForm::new(tab.path.clone(), base, serial);
                            self.dialogs.open(Dialog::UpdateFromBase(Box::new(form)));
                        }
                    }
                }
                Action::FlowInit => {
                    let snap = self.tab().and_then(|t| t.snap.clone());
                    let form = dialogs::FlowInitForm::new(snap.as_deref());
                    self.dialogs.open(Dialog::FlowInit(Box::new(form)));
                }
                Action::FlowStart(kind) => {
                    self.dialogs
                        .open(Dialog::FlowStart(Box::new(dialogs::FlowStartForm::new(
                            kind,
                        ))));
                }
                Action::FlowFinish(branch) => {
                    let snap = self.tab().and_then(|t| t.snap.clone());
                    let form = dialogs::FlowFinishForm::new(branch, snap.as_deref());
                    self.dialogs.open(Dialog::FlowFinish(Box::new(form)));
                }
                Action::ReadIgnore { repo, query } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.path == repo) {
                        t.read_ignore(query);
                    }
                }
                Action::Op(op) => {
                    if let Some(t) = self.tab() {
                        t.op(op);
                    }
                }
                Action::OpIn { repo, op } => {
                    if let Some(t) = self.tab_at(&repo) {
                        t.op(op);
                    }
                }
                Action::Confirm {
                    title,
                    body,
                    confirm,
                    danger,
                    op,
                } => {
                    if let Some(repo) = self.tab().map(|t| t.path.clone()) {
                        self.dialogs.open(Dialog::Confirm {
                            title,
                            body,
                            confirm,
                            danger,
                            repo,
                            op,
                        });
                    }
                }
                Action::ConvertPartialClone { path } => self.request_open(
                    ctx,
                    open::OpenRequest {
                        path,
                        create: None,
                        focus: true,
                        replace: None,
                        convert_legacy: true,
                    },
                ),
                Action::RepositoryInsights(repo) => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.path == repo) {
                        tab.read_insights();
                        let repo = tab.path.clone();
                        if !self
                            .dialogs
                            .any(|d| matches!(d, Dialog::Insights { repo: path } if *path == repo))
                        {
                            self.dialogs.open(Dialog::Insights { repo });
                        }
                    }
                }
                Action::FileHistory { from, path } => {
                    if let Some(tab) = self.tab() {
                        tab.read_file_history(from, path.clone());
                        let repo = tab.path.clone();
                        self.dialogs.open(Dialog::FileHistory { repo, path, from });
                    }
                }
                Action::Lfs(page) => {
                    if let Some(tab) = self.tab() {
                        tab.read_lfs_lists();
                        let repo = tab.path.clone();
                        self.dialogs
                            .open(Dialog::Lfs(Box::new(dialogs::LfsForm::new(repo, page))));
                    }
                }
                Action::LfsLists => {
                    if let Some(tab) = self.tab() {
                        tab.read_lfs_lists();
                    }
                }
                Action::SearchChanges { query, regex } => {
                    if let Some(tab) = self.tab() {
                        tab.search_changes(query, regex);
                    }
                }
                Action::ShowFileVersion(entry) => {
                    if let Some(tab) = self.tab() {
                        tab.show_file_version(&entry);
                    }
                }
                Action::BlameFile => {
                    if let Some(tab) = self.tab() {
                        tab.blame_file();
                    }
                }
                Action::OpenDialog(mut d) => {
                    self.folder_picker.cancel();
                    if let Dialog::Open { recent, .. } = &mut d {
                        recent.clone_from(&self.settings.recent);
                    }
                    d.name_remote(&self.remote_names());
                    self.dialogs.open(d);
                }
                Action::OpenRepo(p) => self.open_repo(ctx, p),
                Action::SwitchWorktree(p) => self.switch_worktree(ctx, p),
                Action::Clone { url, dest } => self.start_clone(ctx, url, dest),
                Action::Init(p, template) => self.init_repo(ctx, p, template),
                Action::ForgetRecent(p) => self.settings.recent.retain(|r| r != &p),
                Action::CloseTab(i) => {
                    if i < self.tabs.len() {
                        let closed = self.tabs.remove(i);
                        open::keep_draft(ctx, &closed);
                        // Whichever tab now sits at the active index (a neighbour, or the last one
                        // when the closed tab was) is the shown one, so it loads now.
                        self.activate(ctx, self.active);
                    }
                }
                Action::SwitchTab(i) => {
                    if i < self.tabs.len() {
                        // A tab shown for the first time loads in full ([`CanopyApp::activate`])
                        // instead of refreshing.
                        let loaded = self.tabs[i].is_loaded();
                        self.activate(ctx, i);
                        if loaded {
                            self.tabs[i].refresh();
                            self.request_identity(IdentityJob::ReadRepo(self.tabs[i].path.clone()));
                        }
                    }
                }
                Action::SetView(v) => {
                    if let Some(t) = self.tab() {
                        if v == View::History {
                            t.history_layout.show_list();
                        }
                        if t.view != v {
                            t.view = v;
                            t.deselect_file();
                            if v == View::History
                                && let Some(oid) = t.selected_commit.take()
                            {
                                t.select_commit(oid);
                                t.reveal_selected = true;
                            }
                            if v == View::Requests {
                                if t.requests.list.is_none() {
                                    t.load_requests();
                                } else {
                                    t.refresh_requests_if_stale();
                                }
                            }
                        }
                    }
                }
                Action::Reveal(oid) => {
                    if let Some(t) = self.tab() {
                        t.view = View::History;
                        t.history_layout.show_list();
                        t.filter.clear();
                        t.select_commit(oid);
                        t.scroll_to_selected = true;
                    }
                }
                Action::Refresh => {
                    if let Some(t) = self.tab() {
                        t.refresh_by_user();
                        if t.view == View::Requests {
                            t.load_requests();
                        }
                    }
                }
                Action::ForcePush => {
                    if let Some(t) = self.tab() {
                        t.worker.send(Job::PushTarget);
                    }
                }
                Action::LoadMore => {
                    let page = self.settings.commit_page;
                    if let Some(t) = self.tab() {
                        t.commit_limit += page;
                        t.refresh();
                    }
                }
                Action::Fetch => {
                    let settings = &self.settings;
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        let prune = settings.prune_for(&t.path);
                        t.op(Op::Fetch {
                            prune,
                            quiet: false,
                        });
                    }
                }
                Action::Pull => {
                    let settings = &self.settings;
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        let choice = settings.pull_for(&t.path);
                        t.op(Op::Pull {
                            strategy: choice.mode.strategy(),
                            follow_config: choice.follow_config,
                            prune: settings.prune_for(&t.path),
                            checked: None,
                        });
                    }
                }
                Action::PullAs {
                    repo,
                    mode,
                    remember,
                    seen,
                } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.path == repo) {
                        match remember {
                            Remember::No => {}
                            Remember::Repository => {
                                let own = self.settings.repo_mut(&t.path);
                                own.pull_mode = Some(mode);
                                // Chosen for this repository: its git configuration no longer
                                // decides.
                                own.pull_follow_config = Some(false);
                            }
                            Remember::Everywhere => {
                                // Offered only where Canopy's own setting, not the git
                                // configuration, asked for fast-forwards only.
                                self.settings.pull_mode = mode;
                                let own = self.settings.repo_mut(&t.path);
                                own.pull_mode = None;
                                own.pull_follow_config = None;
                            }
                        }
                        t.op(Op::Pull {
                            strategy: mode.strategy(),
                            follow_config: false,
                            prune: self.settings.prune_for(&t.path),
                            checked: seen,
                        });
                    }
                }
                Action::PullAnyway {
                    repo,
                    strategy,
                    seen,
                } => {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.path == repo) {
                        let prune = self.settings.prune_for(&t.path);
                        t.op(Op::Pull {
                            strategy,
                            follow_config: false,
                            prune,
                            checked: seen,
                        });
                    }
                }
                Action::Push => {
                    if self
                        .tabs
                        .get(self.active)
                        .is_some_and(RepoTab::is_local_only)
                    {
                        self.dialogs.open(self.add_remote_dialog(true));
                    } else if let Some(t) = self.tab() {
                        t.op(Op::Push {
                            force: false,
                            expect: None,
                        });
                    }
                }
                Action::WhatsNew => {
                    if let Some(d) = whats_new() {
                        self.dialogs.open(d);
                    }
                }
                Action::CopyDiagnostics => {
                    ctx.copy_text(self.diagnostics());
                    self.toasts.push(
                        ctx,
                        ToastKind::Info,
                        Topic::Confirmation,
                        "Diagnostics copied",
                        "Check the text before you share it: it can name files and branches.",
                    );
                }
                Action::OpenLogs => match crate::logging::log_dir() {
                    Some(dir) => {
                        let file_manager = self.file_manager(ctx);
                        let opened = std::fs::create_dir_all(&dir)
                            .map_err(|e| format!("{}: {e}", dir.display()))
                            .and_then(|()| {
                                crate::platform::open_folder(&dir, &file_manager.command)
                            });
                        self.finish_launch(
                            ctx,
                            &file_manager,
                            opened,
                            "Could not open the log folder",
                        );
                    }
                    None => self.toasts.push(
                        ctx,
                        ToastKind::Error,
                        Topic::Error,
                        "No log folder",
                        "This platform has no folder for Canopy's files.",
                    ),
                },
                Action::ShowUpdate => {
                    if let UpdateState::Available(a) = &self.updater.state {
                        self.dialogs.open(Dialog::Update(a.clone()));
                    }
                }
                Action::AddRemoteDialog => {
                    self.dialogs.open(self.add_remote_dialog(false));
                }
                Action::Publish => {
                    self.dialogs.open(self.add_remote_dialog(true));
                }
                Action::Commit => {
                    let signoff = self
                        .tabs
                        .get(self.active)
                        .map_or(self.settings.signoff, |t| {
                            self.settings.signoff_for(&t.path)
                        });
                    if let Some(t) = self.tab().filter(|tab| tab.can_commit()) {
                        let mut message = t.commit_summary.trim().to_owned();
                        if !t.commit_body.trim().is_empty() {
                            message.push_str("\n\n");
                            message.push_str(t.commit_body.trim());
                        }
                        if !message.is_empty() {
                            let op = Op::Commit {
                                message,
                                amend: t.amend,
                                signoff,
                            };
                            match t
                                .amend
                                .then(|| amend_pushed_dialog(t, op.clone()))
                                .flatten()
                            {
                                Some(dialog) => self.dialogs.open(dialog),
                                None => t.op(op),
                            }
                        }
                    }
                }
                Action::PrefillAmend => {
                    if let Some(t) = self.tab() {
                        t.worker.send(Job::HeadMessage);
                    }
                }
                Action::SetTheme(id) => self.set_theme(ctx, id),
                Action::SetAppearance(choice) => self.set_appearance(ctx, choice),
                Action::NextTheme => {
                    let next = self.settings.theme.next();
                    self.set_theme(ctx, next);
                    self.toasts.push(
                        ctx,
                        ToastKind::Info,
                        Topic::Confirmation,
                        format!("Theme: {}", next.name()),
                        "",
                    );
                }
                Action::Palette => self.palette.open(),
                Action::Cheatsheet => self.cheatsheet.toggle(),
                Action::Settings => self.open_settings(SettingsScreen::default()),
                Action::SettingsAt(section) => self.open_settings(SettingsScreen::at(section)),
                Action::SettingsGroup(section, group) => {
                    self.open_settings(SettingsScreen::at_group(section, group));
                }
                Action::RepositorySettings(path) => {
                    if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
                        self.activate(ctx, i);
                        self.open_settings(SettingsScreen::at(
                            settings_screen::Section::Repository,
                        ));
                    }
                }
                Action::RememberSidebar { repo, closed } => {
                    self.settings
                        .repo_sidebar
                        .insert(repo.clone(), closed.clone());
                    for tab in &mut self.tabs {
                        if tab.sidebar_key() == Some(repo.as_path()) {
                            tab.collapsed = closed.iter().cloned().collect();
                        }
                    }
                }
                Action::RememberRequestView { repo, view } => {
                    for tab in &mut self.tabs {
                        if tab.sidebar_key() == Some(repo.as_path()) {
                            tab.request_view = view.clone();
                            tab.request_view_changed();
                        }
                    }
                    // The default needs no entry, so repositories left alone do not pile up.
                    if view.is_default() {
                        self.settings.repo_requests.remove(&repo);
                    } else {
                        self.settings.repo_requests.insert(repo.clone(), view);
                    }
                }
                Action::SetRepoDiff { path, prefs } => {
                    match prefs {
                        Some(prefs) => self.settings.repo_diff.insert(path.clone(), prefs),
                        None => self.settings.repo_diff.remove(&path),
                    };
                    let prefs = prefs.unwrap_or(self.settings.diff);
                    for t in self.tabs.iter_mut().filter(|t| t.path == path) {
                        if t.diff_prefs != prefs {
                            t.diff_prefs = prefs;
                            t.reload_diff();
                        }
                    }
                }
                Action::ResetRepoSettings => {
                    self.settings.repos.clear();
                    self.settings.repo_diff.clear();
                    let prefs = self.settings.diff;
                    for t in &mut self.tabs {
                        if t.diff_prefs != prefs {
                            t.diff_prefs = prefs;
                            t.reload_diff();
                        }
                    }
                }
                Action::SettingsReplaced => {
                    crate::trust::publish(&self.settings);
                    crate::logging::set_level(self.settings.log_level);
                    for t in &mut self.tabs {
                        let prefs = self.settings.diff_for(&t.path);
                        if t.diff_prefs != prefs {
                            t.diff_prefs = prefs;
                            t.reload_diff();
                        }
                    }
                }
                Action::IdentitySettings => {
                    self.open_settings(SettingsScreen::at(settings_screen::Section::Git));
                }
                Action::SaveIdentity {
                    repo,
                    scope,
                    name,
                    email,
                } => {
                    if let Some(Dialog::Identity { pending, .. }) = self.dialogs.current_mut() {
                        *pending = true;
                    }
                    self.request_identity(IdentityJob::Set {
                        repo,
                        scope,
                        name,
                        email,
                    });
                }
                Action::ClearIdentity { repo, scope } => {
                    self.request_identity(IdentityJob::Clear { repo, scope });
                }
                Action::SaveGitConfig {
                    repo,
                    scope,
                    setting,
                    value,
                } => {
                    if self.may_edit_git_config(ctx) {
                        self.request_git_config(
                            ctx,
                            IdentityJob::Config {
                                repo,
                                scope,
                                setting,
                                value,
                            },
                        );
                    }
                }
                Action::UseRecommendedGitConfig => {
                    if self.may_edit_git_config(ctx) {
                        self.request_git_config(ctx, IdentityJob::Recommendations);
                    }
                }
                Action::TrustFolder(folder) => {
                    let shown = folder.display().to_string();
                    if crate::trust::add(&mut self.settings.trusted_folders, &folder) {
                        crate::trust::publish(&self.settings);
                        self.toasts.push(
                            ctx,
                            ToastKind::Success,
                            Topic::Confirmation,
                            "Folder trusted",
                            shown,
                        );
                    }
                }
                Action::SetTrustSafety(on) => {
                    self.settings.trust_safety = on;
                    crate::trust::publish(&self.settings);
                }
                Action::Copy(text) => {
                    ctx.copy_text(text.clone());
                    self.toasts
                        .push(ctx, ToastKind::Info, Topic::Confirmation, "Copied", text);
                }
                Action::OpenUrl(url) => {
                    if let Err(e) = crate::platform::open_url(&url) {
                        self.toasts.push(
                            ctx,
                            ToastKind::Error,
                            Topic::Error,
                            "Could not open the page",
                            e,
                        );
                    }
                }
                Action::OpenPath(dir) => {
                    let file_manager = self.file_manager(ctx);
                    let opened = crate::platform::open_folder(&dir, &file_manager.command);
                    self.finish_launch(ctx, &file_manager, opened, "Could not open the folder");
                }
                Action::OpenInEditor(file) => {
                    let editor = self.launcher(ctx, Kind::Editor);
                    let opened = crate::platform::open_in_editor(&file, &editor.command);
                    self.finish_launch(ctx, &editor, opened, "Could not open the file");
                }
                Action::OpenTerminal(p) => {
                    let terminal = self.launcher(ctx, Kind::Terminal);
                    let opened = crate::platform::open_terminal(&p, &terminal.command);
                    self.finish_launch(ctx, &terminal, opened, "Could not open a terminal");
                }
                Action::Command(c) => {
                    let mut more = Vec::new();
                    self.command(ctx, c, 0, &mut more);
                    self.apply(ctx, more);
                }
                Action::CheckUpdates => {
                    self.settings.last_update_check = chrono::Utc::now().timestamp();
                    self.updater
                        .start(ctx, self.settings.include_prereleases, true);
                }
                Action::InstallUpdate(available) => self.updater.install(ctx, &available),
                Action::RestartInto(exe) => {
                    crate::selfupdate::request_restart(exe);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Action::SkipVersion(v) => {
                    self.settings.skipped_version = Some(v);
                    self.updater.state = UpdateState::UpToDate;
                }
                Action::CreateRequest { source } => self.create_request(ctx, &source),
                Action::Host(job) => {
                    if let Some(t) = self.tab() {
                        t.host_action(job);
                    }
                }
                Action::LoadMoreRequests => {
                    if let Some(t) = self.tab() {
                        t.load_more_requests();
                    }
                }
                Action::CheckoutRequest(r) => self.checkout_request(r),
                Action::DiffPrefsChanged => {
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        let prefs = t.diff_prefs;
                        let path = t.path.clone();
                        t.reload_diff();
                        if prefs == self.settings.diff {
                            self.settings.repo_diff.remove(&path);
                        } else {
                            self.settings.repo_diff.insert(path, prefs);
                        }
                    }
                }
                Action::DiffPrefsForEveryRepository => {
                    if let Some(t) = self.tabs.get(self.active) {
                        let prefs = t.diff_prefs;
                        self.settings.repo_diff.remove(&t.path);
                        self.settings.diff = prefs;
                        for t in &mut self.tabs {
                            if !self.settings.repo_diff.contains_key(&t.path)
                                && t.diff_prefs != prefs
                            {
                                t.diff_prefs = prefs;
                                t.reload_diff();
                            }
                        }
                    }
                }
                Action::DismissIntro => self.settings.seen_intro = true,
                Action::AcceptTerms => {
                    crate::ui::legal::TERMS_VERSION.clone_into(&mut self.settings.accepted_terms);
                    crate::ui::legal::PRIVACY_VERSION
                        .clone_into(&mut self.settings.accepted_privacy);
                    // The consent dialog stays open until its actions are applied; this replaces
                    // it with the next question, or ends it.
                    self.dialogs.close_where(is_consent);
                    match crate::ui::legal::pending(&self.settings) {
                        Some(step) => self.dialogs.open(Dialog::Consent {
                            step,
                            updated: false,
                            reading: None,
                        }),
                        None => self.consent_done(ctx),
                    }
                }
                Action::SetCrashReports(on) => {
                    self.settings.crash_reports = Some(on);
                    self.settings.crash_reports_version = crate::crash_reports::DATA_VERSION;
                    crate::crash_reports::set_enabled(on);
                    if self.dialogs.any(is_consent) {
                        self.dialogs.close_where(is_consent);
                        self.consent_done(ctx);
                    }
                }
                Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Action::SetSplitDiff(split) => {
                    self.settings.split_diff = split;
                    for t in &mut self.tabs {
                        t.split_diff = split;
                    }
                }
                Action::SetRequestDensity(density) => {
                    self.settings.request_density = density;
                    for t in &mut self.tabs {
                        t.request_density = density;
                    }
                }
            }
        }
    }

    /// Opens Settings, reading the identity it shows (the active repository's, else the global
    /// one) off the UI thread.
    pub(super) fn open_settings(&mut self, screen: SettingsScreen) {
        self.settings_screen = Some(screen);
        if let Some(t) = self.tabs.get(self.active) {
            self.request_identity(IdentityJob::ReadRepo(t.path.clone()));
        } else {
            self.request_identity(IdentityJob::ReadGlobal);
        }
    }

    /// The command an open action runs: the one chosen in Settings, else the system's. A chosen
    /// well-known program that is no longer installed is not started: the system's runs instead.
    fn launcher(&self, _ctx: &egui::Context, kind: Kind) -> crate::system_apps::Launch {
        let apps = crate::system_apps::get();
        self.launcher_with(kind, apps.as_deref())
    }

    /// [`Self::launcher`] given what was detected.
    fn launcher_with(
        &self,
        kind: Kind,
        apps: Option<&crate::system_apps::Apps>,
    ) -> crate::system_apps::Launch {
        let configured = match kind {
            Kind::Editor => &self.settings.editor,
            Kind::Terminal => &self.settings.terminal,
            Kind::FileManager => &self.settings.file_manager,
        };
        crate::system_apps::launch(apps, configured, kind)
    }

    /// Reports a fallback only after the operating system accepted the replacement program;
    /// otherwise the launch error stands on its own, with no contradictory success message.
    fn finish_launch(
        &mut self,
        ctx: &egui::Context,
        launch: &crate::system_apps::Launch,
        opened: Result<(), String>,
        error_title: &'static str,
    ) {
        match opened {
            Ok(()) => {
                if let Some(fallback) = &launch.fallback {
                    self.toasts.push(
                        ctx,
                        ToastKind::Info,
                        Topic::Attention,
                        format!("{} is not installed", fallback.missing),
                        format!(
                            "Opened with {} instead. Choose another program in Settings → Integrations.",
                            fallback.used
                        ),
                    );
                }
            }
            Err(e) => {
                self.toasts
                    .push(ctx, ToastKind::Error, Topic::Error, error_title, e);
            }
        }
    }

    fn file_manager(&self, ctx: &egui::Context) -> crate::system_apps::Launch {
        self.launcher(ctx, Kind::FileManager)
    }

    /// Each tab's name, icon, pull choice and git options as the settings say (cheap: runs every
    /// frame; the worker hears of the options only when they change).
    pub(super) fn apply_repo_looks(&mut self) {
        for t in &mut self.tabs {
            let own = self.settings.repos.get(&t.path);
            let icon = crate::tab::tab_icon(own.and_then(|r| r.icon.as_deref()));
            if t.icon != icon {
                t.icon = icon;
            }
            t.pull = self.settings.pull_for(&t.path);
            let options = self.settings.git_options();
            // A tab that has not loaded has no thread to tell: it hears of them in the frame
            // that loads it, as a new tab always did.
            if t.is_loaded() && t.git_options != Some(options) {
                t.git_options = Some(options);
                t.worker.send(Job::Options(options));
            }
            match own.and_then(|r| r.display_name.as_deref()) {
                Some(name) if t.name != name => name.clone_into(&mut t.name),
                Some(_) => {}
                None => {
                    let folder = t
                        .path
                        .file_name()
                        .unwrap_or_else(|| t.path.as_os_str())
                        .to_string_lossy();
                    if folder.as_ref() != t.name {
                        folder.as_ref().clone_into(&mut t.name);
                    }
                }
            }
        }
    }

    /// The remotes of the active repository, by name.
    fn remote_names(&self) -> Vec<String> {
        self.tabs
            .get(self.active)
            .and_then(|t| t.snap.as_ref())
            .map(|s| s.remotes.iter().map(|r| r.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The add-remote dialog (or, `publish`, the publish dialog) for the active repository, with
    /// a remote name it does not use yet and its URL protocol the repository's choice.
    fn add_remote_dialog(&self, publish: bool) -> Dialog {
        let mut d = Dialog::add_remote(&self.settings);
        if let (Some(t), Dialog::AddRemote { builder, .. }) = (self.tabs.get(self.active), &mut d) {
            builder.ssh = self.settings.protocol_for(&t.path) == settings::CloneProtocol::Ssh;
        }
        if publish {
            d = d.publishing();
        }
        d.name_remote(&self.remote_names());
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::AppearanceChoice;
    use crate::settings::DiffPrefs;
    use crate::ui::motion::MotionPref;
    use crate::ui::theme::{Density, Style};

    fn app_with_tab() -> CanopyApp {
        let mut app = test_app();
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        app
    }

    /// A saved well-known program that is no longer installed is not started: the system's runs
    /// and a notification says which program and why; any other command runs as typed, silently.
    #[test]
    fn opening_with_a_program_that_is_no_longer_installed_uses_the_systems_and_says_so() {
        use crate::system_apps::{Apps, Missing, SystemChoice};
        let ctx = egui::Context::default();
        let apps = Apps {
            terminal: Some(SystemChoice {
                name: "kitty".to_owned(),
                command: Some("kitty".to_owned()),
                id: None,
            }),
            missing: vec![Missing::new(Kind::Terminal, "Alacritty", ["alacritty"])],
            ..Apps::default()
        };
        let mut app = test_app();
        app.settings.terminal = "alacritty --working-directory %d".to_owned();
        let launch = app.launcher_with(Kind::Terminal, Some(&apps));
        assert_eq!(
            launch.command, "kitty",
            "the system's terminal runs, never the missing one"
        );
        app.finish_launch(&ctx, &launch, Ok(()), "Could not open a terminal");
        let toasts = format!("{:?}", app.toasts);
        assert!(toasts.contains("Alacritty is not installed"), "{toasts}");
        assert!(toasts.contains("Opened with kitty instead"), "{toasts}");

        let mut app = test_app();
        app.settings.terminal = "foot".to_owned();
        assert_eq!(
            app.launcher_with(Kind::Terminal, Some(&apps)).command,
            "foot"
        );
        app.settings.terminal = String::new();
        assert_eq!(
            app.launcher_with(Kind::Terminal, Some(&apps)).command,
            "kitty"
        );
        assert!(!format!("{:?}", app.toasts).contains("not installed"));
    }

    /// A fallback is not a success until the operating system accepted the launch. In particular,
    /// a missing saved terminal with no detected replacement must produce only the launch error.
    #[test]
    fn a_fallback_is_not_announced_before_the_launch_succeeds() {
        use crate::system_apps::{Apps, Missing};
        let ctx = egui::Context::default();
        let apps = Apps {
            missing: vec![Missing::new(Kind::Terminal, "Alacritty", ["alacritty"])],
            ..Apps::default()
        };
        let mut app = test_app();
        app.settings.terminal = "alacritty".to_owned();
        let launch = app.launcher_with(Kind::Terminal, Some(&apps));
        app.finish_launch(
            &ctx,
            &launch,
            Err("No terminal emulator found".to_owned()),
            "Could not open a terminal",
        );
        let toasts = format!("{:?}", app.toasts);
        assert!(!toasts.contains("Opened with"), "{toasts}");
        assert!(toasts.contains("No terminal emulator found"), "{toasts}");
    }

    /// A divergence answered for this repository becomes its own pull strategy; answered for
    /// every repository it becomes the default, which this repository then follows too.
    #[test]
    fn a_remembered_pull_choice_reaches_as_far_as_chosen() {
        use crate::settings::PullMode;
        let ctx = egui::Context::default();
        let mut app = app_with_tab();
        let path = app.tabs[0].path.clone();
        let default = app.settings.pull_mode;
        let pull = |mode, remember| Action::PullAs {
            repo: path.clone(),
            mode,
            remember,
            seen: None,
        };
        app.apply(&ctx, vec![pull(PullMode::Merge, Remember::No)]);
        assert_eq!(app.settings.pull_mode_for(&path), default);
        app.apply(&ctx, vec![pull(PullMode::Merge, Remember::Repository)]);
        assert_eq!(app.settings.pull_mode_for(&path), PullMode::Merge);
        assert_eq!(app.settings.pull_mode, default);
        app.apply(&ctx, vec![pull(PullMode::Rebase, Remember::Everywhere)]);
        assert_eq!(app.settings.pull_mode, PullMode::Rebase);
        let here = app.settings.pull_for(&path);
        let everywhere = app.settings.pull_default();
        assert_eq!(
            (here.mode, here.follow_config),
            (everywhere.mode, everywhere.follow_config)
        );
    }

    #[test]
    fn stale_cleanup_uses_the_setting_and_a_fresh_repository_bound_preview() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let (worker, events, jobs) = crate::worker::Worker::detached();
        let mut tab = test_tab(canopy_core::RepoSnapshot::default());
        let path = tab.path.clone();
        tab.worker = worker;
        app.tabs.push(tab);
        assert_eq!(app.settings.stale_inactivity_days, 90);
        app.settings.stale_inactivity_days = 12;
        app.apply(&ctx, vec![Action::CleanStale]);
        let Job::StaleCandidates {
            serial: first,
            older_than_days,
        } = jobs.recv().expect("preview")
        else {
            panic!("a preview must run on the repository worker")
        };
        assert_eq!(older_than_days, 12);
        assert!(
            matches!(app.dialogs.current(), Some(Dialog::CleanStale(form)) if form.repo == path)
        );
        app.apply(&ctx, vec![Action::CleanStale]);
        let Job::StaleCandidates { serial: second, .. } = jobs.recv().expect("fresh preview")
        else {
            panic!("fresh preview")
        };
        assert_ne!(first, second);
        events
            .send(crate::worker::Event::StaleCandidates {
                serial: first,
                result: Err("old preview".into()),
            })
            .expect("old reply");
        app.tabs[0].poll(&[]);
        assert!(app.tabs[0].stale_result.is_none());
        events
            .send(crate::worker::Event::StaleCandidates {
                serial: second,
                result: Ok(std::sync::Arc::from(Vec::new())),
            })
            .expect("new reply");
        app.tabs[0].poll(&[]);
        assert!(matches!(&app.tabs[0].stale_result, Some(Ok(rows)) if rows.is_empty()));
    }

    /// The force push question names the branch and where it goes, which the repository thread
    /// works out, and confirming it carries that answer to the push.
    #[test]
    fn the_force_push_question_names_the_branch_and_its_destination() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let (worker, events, jobs) = crate::worker::Worker::detached();
        let mut tab = test_tab(canopy_core::RepoSnapshot::default());
        tab.worker = worker;
        app.tabs.push(tab);
        app.apply(&ctx, vec![Action::ForcePush]);
        assert!(
            app.dialogs.current().is_none(),
            "nothing is asked before the answer"
        );
        let sent: Vec<_> = jobs.try_iter().collect();
        assert!(
            sent.iter().any(|j| matches!(j, Job::PushTarget)),
            "{sent:?}"
        );
        events
            .send(crate::worker::Event::PushTarget(Ok(
                canopy_core::PushTarget {
                    branch: "feature/dark-mode".into(),
                    remote: "origin".into(),
                    dest: "feature/dark-mode".into(),
                },
            )))
            .expect("the tab listens");
        app.poll(&ctx);
        let Some(Dialog::Confirm {
            title, body, op, ..
        }) = app.dialogs.current()
        else {
            panic!("no confirmation: {:?}", app.dialogs.current());
        };
        assert_eq!(
            title,
            "Force push feature/dark-mode to origin/feature/dark-mode?"
        );
        assert!(
            body.contains("origin/feature/dark-mode is overwritten"),
            "{body}"
        );
        let Op::Push {
            force: true,
            expect: Some(expect),
        } = op
        else {
            panic!("a force push naming its target: {op:?}");
        };
        assert_eq!(
            (&*expect.branch, &*expect.remote, &*expect.dest),
            ("feature/dark-mode", "origin", "feature/dark-mode")
        );
    }

    /// #172: amending the commit the upstream already has asks first; an unpushed one does not.
    #[test]
    fn amending_a_pushed_commit_asks_first() {
        let ctx = egui::Context::default();
        let branch = |ahead| canopy_core::BranchInfo {
            name: "main".into(),
            target: None,
            is_head: true,
            upstream: Some("origin/main".into()),
            ahead,
            behind: 0,
            worktree: None,
        };
        for (ahead, asks) in [(0, true), (1, false)] {
            let mut app = test_app();
            let mut tab = test_tab(canopy_core::RepoSnapshot {
                branches: vec![branch(ahead)],
                ..Default::default()
            });
            tab.amend = true;
            tab.commit_summary = "Reword".into();
            app.tabs.push(tab);
            app.apply(&ctx, vec![Action::Commit]);
            match app.dialogs.current() {
                Some(Dialog::Confirm {
                    title,
                    body,
                    op: Op::Commit { amend: true, .. },
                    ..
                }) => {
                    assert!(asks, "an unpushed commit is amended without a question");
                    assert_eq!(title, "Amend main, already on origin/main?");
                    assert!(body.contains("force push"), "{body}");
                }
                other => assert!(!asks, "no confirmation: {other:?}"),
            }
        }
    }

    #[test]
    fn the_force_delete_question_names_the_branch() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        app.tabs[0].force_delete_offer = Some("topic".into());
        app.poll(&ctx);
        let Some(Dialog::Confirm { title, .. }) = app.dialogs.current() else {
            panic!("no confirmation: {:?}", app.dialogs.current());
        };
        assert_eq!(title, "Force delete branch topic?");
    }

    /// A pull decision, or the offer to force-delete a branch, that comes back after the user
    /// moved to another tab is answered for the repository that asked, never the active one.
    #[test]
    fn a_late_question_is_answered_for_the_repository_that_asked() {
        use crate::settings::PullMode;
        let ctx = egui::Context::default();
        let mut app = test_app();
        let mut receivers = Vec::new();
        for name in ["asked", "active"] {
            let (worker, _, jobs) = crate::worker::Worker::detached();
            let mut tab = test_tab(canopy_core::RepoSnapshot::default());
            tab.worker = worker;
            tab.path = PathBuf::from(name);
            app.tabs.push(tab);
            receivers.push(jobs);
        }
        app.active = 1;
        app.tabs[0].pull_decision = Some(crate::worker::PullDecision {
            preview: canopy_core::PullPreview::default(),
            strategy: canopy_core::PullStrategy::FastForwardOnly,
            diverged: true,
            configured: false,
            moved: false,
        });
        app.tabs[0].force_delete_offer = Some("topic".into());
        app.poll(&ctx);
        let asked = PathBuf::from("asked");
        let mut answers = Vec::new();
        while let Some(d) = app.dialogs.current().cloned() {
            match d {
                Dialog::Diverged { repo, .. } => {
                    assert_eq!(repo, asked);
                    answers.push(Action::PullAs {
                        repo,
                        mode: PullMode::Merge,
                        remember: Remember::Repository,
                        seen: None,
                    });
                }
                Dialog::Confirm { repo, op, .. } => {
                    assert_eq!(repo, asked);
                    answers.push(Action::OpIn { repo, op });
                }
                other => panic!("unexpected {other:?}"),
            }
            app.dialogs.close();
        }
        assert_eq!(answers.len(), 2, "both questions were asked");
        app.apply(&ctx, answers);
        let sent: Vec<_> = receivers[0].try_iter().collect();
        assert!(
            sent.iter().any(|j| matches!(j, Job::Op(Op::Pull { .. }))),
            "{sent:?}"
        );
        assert!(
            sent.iter()
                .any(|j| matches!(j, Job::Op(Op::DeleteBranch { force: true, .. }))),
            "{sent:?}"
        );
        assert_eq!(
            receivers[1].try_iter().count(),
            0,
            "the active tab is untouched"
        );
        assert_eq!(app.settings.pull_mode_for(&asked), PullMode::Merge);
        assert_ne!(
            app.settings.pull_mode_for(std::path::Path::new("active")),
            PullMode::Merge
        );
    }

    /// A repository's diff options made every repository's default become the default, and the
    /// repository no longer keeps its own.
    #[test]
    fn diff_options_become_every_repositorys() {
        let ctx = egui::Context::default();
        let mut app = app_with_tab();
        let path = app.tabs[0].path.clone();
        let own = DiffPrefs {
            context: 9,
            ..DiffPrefs::default()
        };
        app.tabs[0].diff_prefs = own;
        app.apply(&ctx, vec![Action::DiffPrefsChanged]);
        assert_eq!(app.settings.repo_diff.get(&path), Some(&own));
        app.apply(&ctx, vec![Action::DiffPrefsForEveryRepository]);
        assert_eq!(app.settings.diff, own);
        assert!(app.settings.repo_diff.is_empty());
        assert_eq!(app.tabs[0].diff_prefs, own);
    }

    /// A repository's own diff options reach its tab; clearing them returns it to the default.
    #[test]
    fn a_repository_diff_choice_reaches_its_tab_and_resets() {
        let ctx = egui::Context::default();
        let mut app = app_with_tab();
        let path = app.tabs[0].path.clone();
        let own = DiffPrefs {
            context: 10,
            ..DiffPrefs::default()
        };
        app.apply(
            &ctx,
            vec![Action::SetRepoDiff {
                path: path.clone(),
                prefs: Some(own),
            }],
        );
        assert_eq!(app.tabs[0].diff_prefs, own);
        assert_eq!(app.settings.repo_diff.get(&path), Some(&own));
        app.apply(&ctx, vec![Action::ResetRepoSettings]);
        assert!(app.settings.repo_diff.is_empty() && app.settings.repos.is_empty());
        assert_eq!(app.tabs[0].diff_prefs, app.settings.diff);
    }

    /// "Repository settings" on a tab switches to that tab and opens This repository.
    #[test]
    fn repository_settings_open_on_their_tab() {
        let ctx = egui::Context::default();
        let mut app = app_with_tab();
        let mut other = test_tab(canopy_core::RepoSnapshot::default());
        other.path = PathBuf::from("other");
        app.tabs.push(other);
        app.apply(
            &ctx,
            vec![Action::RepositorySettings(PathBuf::from("other"))],
        );
        assert_eq!(app.active, 1);
        assert!(format!("{:?}", app.settings_screen).contains("Repository"));
        // A repository with no tab opens nothing.
        app.settings_screen = None;
        app.apply(
            &ctx,
            vec![Action::RepositorySettings(PathBuf::from("gone"))],
        );
        assert!(app.settings_screen.is_none());
    }

    /// The palette's Appearance entries change the setting as Settings → Appearance does.
    #[test]
    fn appearance_choices_from_the_palette_change_the_settings() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.apply(
            &ctx,
            vec![
                Action::SetAppearance(AppearanceChoice::Style(Style::Deeper)),
                Action::SetAppearance(AppearanceChoice::Density(Density::Dense)),
                Action::SetAppearance(AppearanceChoice::Motion(MotionPref::Reduced)),
            ],
        );
        assert_eq!(
            (
                app.settings.style,
                app.settings.density,
                app.settings.motion
            ),
            (Style::Deeper, Density::Dense, MotionPref::Reduced)
        );
    }

    /// High contrast from the palette or the theme menu turns the switch on and the look follows
    /// on the same frame; turning it off restores the theme's own colours.
    #[test]
    fn high_contrast_from_the_palette_changes_the_look() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.apply(
            &ctx,
            vec![Action::SetAppearance(AppearanceChoice::HighContrast(true))],
        );
        assert!(app.settings.high_contrast);
        assert!(theme::high_contrast());
        assert!(palette(&ctx).strict);
        app.apply(
            &ctx,
            vec![Action::SetAppearance(AppearanceChoice::HighContrast(false))],
        );
        assert!(!theme::high_contrast());
        assert!(!palette(&ctx).strict);
    }
    #[test]
    fn insights_refresh_targets_its_repository_and_keeps_one_dialog() {
        let ctx = egui::Context::default();
        let mut app = app_with_tab();
        let mut other = test_tab(canopy_core::RepoSnapshot::default());
        other.path = PathBuf::from("other");
        let (worker, _, jobs) = crate::worker::Worker::detached();
        other.worker = worker;
        app.tabs.push(other);
        let action = Action::RepositoryInsights(PathBuf::from("other"));
        app.apply(&ctx, vec![action.clone()]);
        assert!(matches!(jobs.try_recv(), Ok(Job::Insights(_))));
        assert!(
            matches!(app.dialogs.current(), Some(Dialog::Insights { repo }) if repo == &PathBuf::from("other"))
        );
        app.apply(&ctx, vec![action]);
        assert!(matches!(jobs.try_recv(), Ok(Job::Insights(_))));
        app.dialogs.close();
        assert!(app.dialogs.current().is_none());
        assert_eq!(app.active, 0);
    }

    #[test]
    fn file_history_reads_off_the_ui_thread_and_opens_its_dialog() {
        let ctx = egui::Context::default();
        let mut app = app_with_tab();
        let (worker, _, jobs) = crate::worker::Worker::detached();
        app.tabs[0].worker = worker;
        app.apply(
            &ctx,
            vec![Action::FileHistory {
                from: None,
                path: "a.txt".into(),
            }],
        );
        assert!(matches!(
            jobs.try_recv(),
            Ok(Job::FileHistory { from: None, path, .. }) if path == "a.txt"
        ));
        assert!(
            matches!(app.dialogs.current(), Some(Dialog::FileHistory { repo, path, from: None }) if path == "a.txt" && repo == &app.tabs[0].path)
        );
    }
}
