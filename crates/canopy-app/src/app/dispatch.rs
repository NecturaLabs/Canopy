//! Applying the actions a frame collected.

use super::*;

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
                Action::OpenDialog(d) => {
                    self.folder_picker.cancel();
                    self.dialogs.open(d);
                }
                Action::OpenRepo(p) => self.open_repo(ctx, p),
                Action::Clone { url, dest } => self.start_clone(ctx, url, dest),
                Action::Init(p) => self.init_repo(ctx, p),
                Action::ForgetRecent(p) => self.settings.recent.retain(|r| r != &p),
                Action::CloseTab(i) => {
                    if i < self.tabs.len() {
                        self.tabs.remove(i);
                        if self.active >= self.tabs.len() {
                            self.active = self.tabs.len().saturating_sub(1);
                        }
                    }
                }
                Action::SwitchTab(i) => {
                    if i < self.tabs.len() {
                        self.active = i;
                        self.tabs[i].refresh();
                    }
                }
                Action::SetView(v) => {
                    if let Some(t) = self.tab() {
                        if v == View::History {
                            t.history_layout.show_list();
                        }
                        if t.view != v {
                            t.view = v;
                            t.file = None;
                            t.diff = None;
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
                    let prune = self.settings.prune_on_fetch;
                    if let Some(t) = self.tab() {
                        t.op(Op::Fetch {
                            prune,
                            quiet: false,
                        });
                    }
                }
                Action::Pull => {
                    let strategy = self.settings.pull_mode.strategy();
                    if let Some(t) = self.tab() {
                        t.op(Op::Pull { strategy });
                    }
                }
                Action::Push => {
                    if self
                        .tabs
                        .get(self.active)
                        .is_some_and(RepoTab::is_local_only)
                    {
                        self.dialogs
                            .open(Dialog::add_remote(&self.settings).publishing());
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
                        let _ = std::fs::create_dir_all(&dir);
                        if let Err(e) = crate::platform::open_folder(&dir) {
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
                    self.dialogs.open(Dialog::add_remote(&self.settings));
                }
                Action::Publish => {
                    self.dialogs
                        .open(Dialog::add_remote(&self.settings).publishing());
                }
                Action::Commit => {
                    let signoff = self.settings.signoff;
                    if let Some(t) = self.tab() {
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
                Action::NextTheme => {
                    let next = self.settings.theme.next();
                    self.set_theme(ctx, next);
                    self.toasts
                        .push(ctx, ToastKind::Info, format!("Theme: {}", next.name()), "");
                }
                Action::Palette => self.palette.open(),
                Action::Cheatsheet => self.cheatsheet.toggle(),
                Action::Settings => self.settings_screen = Some(SettingsScreen::default()),
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
                    if let Err(e) = crate::platform::open_folder(&dir) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the folder", e);
                    }
                }
                Action::OpenInEditor(file) => {
                    if let Err(e) = crate::platform::open_in_editor(&file, &self.settings.editor) {
                        self.toasts
                            .push(ctx, ToastKind::Error, "Could not open the file", e);
                    }
                }
                Action::OpenTerminal(p) => {
                    if let Err(e) = crate::platform::open_terminal(&p, &self.settings.terminal) {
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
                Action::SkipVersion(v) => {
                    self.settings.skipped_version = Some(v);
                    self.updater.state = UpdateState::UpToDate;
                }
                Action::CreateRequest { source } => self.create_request(ctx, source),
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
                    self.settings.accepted_terms = crate::ui::legal::TERMS_VERSION.to_owned();
                    self.settings.accepted_privacy = crate::ui::legal::PRIVACY_VERSION.to_owned();
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
}
