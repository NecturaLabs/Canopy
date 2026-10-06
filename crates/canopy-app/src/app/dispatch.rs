//! Applying the actions a frame collected.

use super::*;
use crate::system_apps::Kind;

impl CanopyApp {
    pub(super) fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Op(op) => {
                    if let Some(t) = self.tab() {
                        t.op(op);
                    }
                }
                Action::Confirm {
                    title,
                    body,
                    confirm,
                    danger,
                    op,
                } => self.dialogs.open(Dialog::Confirm {
                    title,
                    body,
                    confirm,
                    danger,
                    op,
                }),
                Action::OpenDialog(mut d) => {
                    self.folder_picker.cancel();
                    if let Dialog::Open { recent, .. } = &mut d {
                        recent.clone_from(&self.settings.recent);
                    }
                    d.name_remote(&self.remote_names());
                    self.dialogs.open(d);
                }
                Action::OpenRepo(p) => self.open_repo(ctx, p),
                Action::Clone { url, dest } => self.start_clone(ctx, url, dest),
                Action::Init(p) => self.init_repo(ctx, p),
                Action::ForgetRecent(p) => self.settings.recent.retain(|r| r != &p),
                Action::CloseTab(i) => {
                    if i < self.tabs.len() {
                        let closed = self.tabs.remove(i);
                        open::keep_draft(ctx, &closed);
                        if self.active >= self.tabs.len() {
                            self.active = self.tabs.len().saturating_sub(1);
                        }
                    }
                }
                Action::SwitchTab(i) => {
                    if i < self.tabs.len() {
                        self.active = i;
                        self.tabs[i].refresh();
                        self.request_identity(IdentityJob::ReadRepo(self.tabs[i].path.clone()));
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
                            if v == View::Requests && t.requests.list.is_none() {
                                t.load_requests();
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
                        t.refresh();
                        if t.view == View::Requests {
                            t.load_requests();
                        }
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
                        });
                    }
                }
                Action::ReadPullConfig => {
                    if let Some(t) = self.tabs.get(self.active) {
                        t.worker.send(Job::PullConfig);
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
                        t.op(Op::Push { force: false });
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
                        "Diagnostics copied",
                        "Check the text before you share it: it can name files and branches.",
                    );
                }
                Action::OpenLogs => match crate::logging::log_dir() {
                    Some(dir) => {
                        let opened = std::fs::create_dir_all(&dir)
                            .map_err(|e| format!("{}: {e}", dir.display()))
                            .and_then(|()| {
                                crate::platform::open_folder(&dir, &self.file_manager())
                            });
                        if let Err(e) = opened {
                            self.toasts.push(ctx, ToastKind::Error, "Could not open", e);
                        }
                    }
                    None => self.toasts.push(
                        ctx,
                        ToastKind::Error,
                        "No log folder",
                        "This platform has no data folder for Canopy.",
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
                            t.op(Op::Commit {
                                message,
                                amend: t.amend,
                                signoff,
                            });
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
                    self.toasts
                        .push(ctx, ToastKind::Info, format!("Theme: {}", next.name()), "");
                }
                Action::Palette => self.palette.open(),
                Action::Cheatsheet => self.cheatsheet.toggle(),
                Action::Settings => self.open_settings(SettingsScreen::default()),
                Action::SettingsAt(section) => self.open_settings(SettingsScreen::at(section)),
                Action::RepositorySettings(path) => {
                    if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
                        self.active = i;
                        self.open_settings(SettingsScreen::at(
                            settings_screen::Section::Repository,
                        ));
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
                    crate::trust::set(&self.settings.trusted_folders);
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
                Action::TrustFolder(folder) => {
                    let shown = folder.display().to_string();
                    if crate::trust::add(&mut self.settings.trusted_folders, &folder) {
                        crate::trust::set(&self.settings.trusted_folders);
                        self.toasts
                            .push(ctx, ToastKind::Success, "Folder trusted", shown);
                    }
                }
                Action::Copy(text) => {
                    ctx.copy_text(text.clone());
                    self.toasts.push(ctx, ToastKind::Info, "Copied", text);
                }
                Action::OpenUrl(url) => {
                    if let Err(e) = crate::platform::open_url(&url) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the page", e);
                    }
                }
                Action::OpenPath(dir) => {
                    if let Err(e) = crate::platform::open_folder(&dir, &self.file_manager()) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the folder", e);
                    }
                }
                Action::OpenInEditor(file) => {
                    if let Err(e) =
                        crate::platform::open_in_editor(&file, &self.launcher(Kind::Editor))
                    {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the file", e);
                    }
                }
                Action::OpenTerminal(p) => {
                    if let Err(e) =
                        crate::platform::open_terminal(&p, &self.launcher(Kind::Terminal))
                    {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open a terminal", e);
                    }
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
                        let kind = t.host.as_ref().map(|h| h.forge.kind);
                        if let Some(kind) = kind {
                            t.requests.busy = Some(match &job {
                                HostJob::Create(_) => format!("Creating {}", kind.request_noun()),
                                _ => "Working…".into(),
                            });
                        }
                        t.host_job(job);
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

    /// The command an open action runs: the one chosen in Settings, else the system's.
    fn launcher(&self, kind: Kind) -> String {
        let configured = match kind {
            Kind::Editor => &self.settings.editor,
            Kind::Terminal => &self.settings.terminal,
            Kind::FileManager => &self.settings.file_manager,
        };
        crate::system_apps::command_for(configured, kind)
    }

    fn file_manager(&self) -> String {
        self.launcher(Kind::FileManager)
    }

    /// Each tab's name, icon and pull choice as its repository's settings say (cheap: runs every
    /// frame).
    pub(super) fn apply_repo_looks(&mut self) {
        for t in &mut self.tabs {
            let own = self.settings.repos.get(&t.path);
            let icon = crate::tab::tab_icon(own.and_then(|r| r.icon.as_deref()));
            if t.icon != icon {
                t.icon = icon;
            }
            t.pull = self.settings.pull_for(&t.path);
            match own.and_then(|r| r.display_name.as_deref()) {
                Some(name) if t.name != name => name.clone_into(&mut t.name),
                Some(_) => {}
                None => {
                    let folder = t.path.file_name().map(|n| n.to_string_lossy());
                    if folder.as_deref() != Some(t.name.as_str()) {
                        t.name = crate::tab::folder_name(&t.path);
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
}
