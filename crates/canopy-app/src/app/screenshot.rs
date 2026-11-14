//! Headless screenshots for `--screenshot` (tools/screenshot.sh).

use std::sync::Arc;

use super::*;

#[derive(Debug)]
pub(super) struct Screenshot {
    pub(super) path: PathBuf,
    pub(super) ready_frames: u32,
    pub(super) ready_since: Option<f64>,
    pub(super) requested: bool,
    pub(super) started: f64,
    /// A popup kept open for the picture: popups close when a frame does not show them.
    pub(super) popup: Option<egui::Id>,
    /// The context menu `--overlay menu-*` opens for the picture.
    pub(super) menu: Option<MenuShot>,
    /// `--overlay long-message`: a commit draft with a too-long summary and description line
    /// is typed into the first tab, once.
    pub(super) long_message: bool,
    /// `--overlay trust-banner`: the prompt about the first tab's repository counts as answered
    /// with Don't trust, which leaves the banner.
    pub(super) declines_trust: bool,
    /// `--overlay` of a dialog that reads the repository (flow, merged, update from base): it
    /// opens, as the user would open it, once the first tab has its snapshot.
    pub(super) pending_dialog: Option<&'static str>,
    /// `--overlay worktrees` and the worktree overlays: the sidebar's Worktrees section, which
    /// starts closed, is opened once the first tab has its snapshot.
    pub(super) opens_worktrees: bool,
}

/// The `--overlay` names of the dialogs that open from the repository's own state.
const REPOSITORY_DIALOGS: [&str; 9] = [
    "remove-worktree",
    "remove-worktree-dirty",
    "remove-worktree-delete",
    "forget-worktrees",
    "clean-merged",
    "update-from-base",
    "flow-init",
    "flow-start",
    "flow-finish",
];

impl Screenshot {
    /// A picture saved to `path`, of `overlay` when that is one this module opens itself.
    pub(super) fn new(path: PathBuf, overlay: Option<&str>) -> Self {
        Self {
            path,
            ready_frames: 0,
            ready_since: None,
            requested: false,
            started: 0.0,
            popup: (overlay == Some("diff-options"))
                .then(|| egui::Id::new(crate::views::diff::OPTIONS_POPUP)),
            menu: overlay.and_then(MenuShot::of),
            long_message: overlay == Some("long-message"),
            declines_trust: overlay == Some("trust-banner"),
            pending_dialog: overlay
                .and_then(|name| REPOSITORY_DIALOGS.iter().copied().find(|d| *d == name)),
            opens_worktrees: matches!(overlay, Some("worktrees" | "menu-worktree")),
        }
    }
}

/// `--overlay menu-*`: a context menu opened for the picture by a right click on the top-most
/// (then left-most) widget whose AccessKit name starts with `name`, once the window has settled.
/// Found by what a screen reader hears (not by role, which the kit may refine), so the picture
/// does not depend on any view's ids.
#[derive(Debug)]
pub(super) struct MenuShot {
    name: &'static str,
    /// The view the widget is in.
    pub(super) view: View,
    /// The click that opens it: secondary for a context menu, primary for a button's popover.
    button: egui::PointerButton,
    /// Where the click goes once the widget is found, and how many of its two frames (press,
    /// then release) were sent.
    click: Option<(egui::Pos2, u8)>,
    /// Wait for a scrolled control to stop moving before aiming at it.
    candidate: Option<(egui::Rect, u8)>,
}

impl MenuShot {
    fn of(overlay: &str) -> Option<Self> {
        let secondary = egui::PointerButton::Secondary;
        let (name, view, button) = match overlay {
            "menu-branch" => ("feature/auth", View::History, secondary),
            // The first linked folder of the fixture the screenshot is taken of, named `wt-…`.
            "menu-worktree" => ("wt-", View::History, secondary),
            "menu-commit" => ("Commit ", View::History, secondary),
            "menu-file" => ("src/main.rs", View::Changes, secondary),
            "menu-lfs-file" => ("art/hero.psd", View::Changes, secondary),
            "blame" => ("Blame", View::Changes, egui::PointerButton::Primary),
            "notifications" => ("Notifications", View::History, egui::PointerButton::Primary),
            "amend-commit" => (
                "Amend last commit",
                View::Changes,
                egui::PointerButton::Primary,
            ),
            "commit-compact" => (
                "Commit details",
                View::History,
                egui::PointerButton::Primary,
            ),
            "settings-git-pull" | "settings-repository-pull" => {
                ("Pull strategy", View::History, egui::PointerButton::Primary)
            }
            "commit-expanded" => ("Expand", View::History, egui::PointerButton::Primary),
            "author-card" => (
                "Maya Chen (commit identity card)",
                View::History,
                egui::PointerButton::Primary,
            ),
            "branch-selected" => ("feature/auth", View::History, egui::PointerButton::Primary),
            "request-filters" => (
                "Filter by draft, checks",
                View::Requests,
                egui::PointerButton::Primary,
            ),
            "request-sort" => (
                "Sort requests",
                View::Requests,
                egui::PointerButton::Primary,
            ),
            _ => return None,
        };
        Some(Self {
            name,
            view,
            button,
            click: None,
            candidate: None,
        })
    }

    /// Whether the menu is open: finds its widget in this frame's AccessKit tree and aims the
    /// click at it, then waits for the click's two frames.
    fn opened(&mut self, ctx: &egui::Context) -> bool {
        if let Some((_, sent)) = self.click {
            return sent >= 2;
        }
        let found = ctx.viewport(|v| {
            let nodes = &v.this_pass.accesskit_state.as_ref()?.nodes;
            nodes
                .iter()
                .filter(|(_, n)| n.label().is_some_and(|l| l.starts_with(self.name)))
                .filter_map(|(id, _)| {
                    v.this_pass
                        .widgets
                        .get(*id)
                        .filter(|w| w.enabled)
                        .map(|w| w.interact_rect)
                })
                .filter(egui::Rect::is_positive)
                .min_by(|a, b| {
                    (a.top(), a.left())
                        .partial_cmp(&(b.top(), b.left()))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        if let Some(rect) = found {
            let frames = self
                .candidate
                .filter(|(previous, _)| *previous == rect)
                .map_or(1, |(_, frames)| frames + 1);
            self.candidate = Some((rect, frames));
            if frames >= 3 {
                self.click = Some((rect.center(), 0));
            }
        } else {
            self.candidate = None;
        }
        false
    }
}

impl CanopyApp {
    /// Makes the active tab show the `--demo-requests` scenario (see `demo_requests`), and the
    /// settings the requests view needs to draw it. Whether the picture may be taken yet.
    fn show_demo_requests(
        &mut self,
        ctx: &egui::Context,
        scenario: demo_requests::Scenario,
    ) -> bool {
        let now = chrono::Utc::now().timestamp();
        let shown = match self.tabs.get_mut(self.active) {
            Some(tab) => scenario.show(ctx, tab, now),
            // Nothing is open (the welcome screen): there is nothing to wait for.
            None => return true,
        };
        // The view draws only when pull and merge requests are on. That is switched on only once
        // the tab holds the demo's API thread, so the next frame has no reason to start a real
        // one; the pictures it would fetch from the host stay off.
        self.settings.hosting_api = shown;
        self.settings.download_avatars = false;
        shown
    }

    pub(super) fn handle_screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_none() {
            return;
        }
        // `--demo-requests`: the active tab shows invented requests instead of loading them.
        let demo = demo_requests::chosen();
        let demo_shown = demo.is_none_or(|scenario| self.show_demo_requests(ctx, scenario));
        if let Some(Dialog::Insights { repo }) = self.dialogs.current_mut()
            && let Some(tab) = self.tabs.get_mut(self.active)
            && repo.as_os_str().is_empty()
        {
            repo.clone_from(&tab.path);
            tab.read_insights();
        }
        if let Some(Dialog::FileHistory { repo, path, from }) = self.dialogs.current_mut()
            && let Some(tab) = self.tabs.get_mut(self.active)
            && path.is_empty()
            && tab.snap.is_some()
        {
            repo.clone_from(&tab.path);
            // A file with several commits in the fixture repository.
            "Cargo.toml".clone_into(path);
            // `file-history-commit` asks from the commit the History view selected, else HEAD's.
            if from.is_some() {
                *from = tab
                    .selected_commit
                    .or_else(|| tab.snap.as_ref().and_then(|s| s.head.oid));
            }
            tab.read_file_history(*from, path.clone());
        }
        if let Some(Dialog::Lfs(form)) = self.dialogs.current_mut()
            && let Some(tab) = self.tabs.get_mut(self.active)
            && form.repo.as_os_str().is_empty()
            && tab.snap.is_some()
        {
            form.repo.clone_from(&tab.path);
            tab.read_lfs_lists();
        }
        if matches!(self.dialogs.current(), Some(Dialog::CleanStale(form)) if form.repo.as_os_str().is_empty())
            && !self.tabs.is_empty()
        {
            self.dialogs.close();
            self.apply(ctx, vec![Action::CleanStale]);
        }
        if let Some(name) = self.screenshot.as_ref().and_then(|s| s.pending_dialog)
            && self.tabs.get(self.active).is_some_and(|t| t.snap.is_some())
        {
            if let Some(shot) = &mut self.screenshot {
                shot.pending_dialog = None;
            }
            // The linked folders the removal can be asked of: the first for `remove-worktree`, the
            // last for the two that need a folder holding changes.
            let removable: Vec<PathBuf> = self
                .tabs
                .get(self.active)
                .and_then(|t| t.snap.as_ref())
                .map(|s| {
                    s.worktrees
                        .iter()
                        .filter(|w| {
                            !w.main
                                && !w.current
                                && w.locked.is_none()
                                && w.state == canopy_core::WorktreeState::Ready
                        })
                        .map(|w| w.path.clone())
                        .collect()
                })
                .unwrap_or_default();
            let action = match name {
                "clean-merged" => Action::CleanMerged,
                "update-from-base" => Action::UpdateFromBase,
                "flow-init" => Action::FlowInit,
                "flow-start" => Action::FlowStart(None),
                "forget-worktrees" => Action::ForgetWorktrees,
                "remove-worktree" => removable.first().map_or(Action::ForgetWorktrees, |p| {
                    Action::RemoveWorktree(p.clone())
                }),
                "remove-worktree-dirty" | "remove-worktree-delete" => {
                    removable.last().map_or(Action::ForgetWorktrees, |p| {
                        Action::RemoveWorktree(p.clone())
                    })
                }
                _ => Action::FlowFinish(None),
            };
            self.apply(ctx, vec![action]);
            if name == "remove-worktree-delete"
                && let Some(Dialog::RemoveWorktree(form)) = self.dialogs.current_mut()
            {
                form.ask_destructive();
            }
            // A name for the picture, so the preview line shows what it will create.
            if let Some(Dialog::FlowStart(form)) = self.dialogs.current_mut() {
                "payments".clone_into(&mut form.name);
            }
        }
        if self.screenshot.as_ref().is_some_and(|s| s.opens_worktrees)
            && let Some(tab) = self.tabs.get_mut(self.active)
            && tab.snap.is_some()
        {
            tab.set_sidebar_open("worktrees", true);
            if let Some(shot) = &mut self.screenshot {
                shot.opens_worktrees = false;
            }
        }
        if self.screenshot.as_ref().is_some_and(|s| s.long_message)
            && let Some(tab) = self.tabs.get_mut(self.active)
        {
            tab.commit_summary =
                "Rework the commit composer so that it reports every long line before the commit"
                    .into();
            tab.commit_body = "Warn while typing, never block the commit.\n\nThis description line is far too long for most tools, which cut it off or wrap it in the middle of a sentence.".into();
            if let Some(shot) = &mut self.screenshot {
                shot.long_message = false;
            }
        }
        let Some(shot) = &mut self.screenshot else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if shot.started == 0.0 {
            shot.started = now;
        }
        if let Some(Dialog::SearchChanges {
            repo,
            query,
            searched,
            ..
        }) = self.dialogs.current_mut()
            && let Some(tab) = self.tabs.get_mut(self.active)
            && repo.as_os_str().is_empty()
            && tab.snap.is_some()
        {
            repo.clone_from(&tab.path);
            // Text the showcase fixture's history changes more than once.
            "tokio".clone_into(query);
            *searched = true;
            tab.search_changes(query.clone(), false);
        }
        let search_ready = !self
            .dialogs
            .any(|d| matches!(d, Dialog::SearchChanges { .. }))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.change_search.is_some());
        let insights_ready = !self.dialogs.any(|d| matches!(d, Dialog::Insights { .. }))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.insights.is_some());
        let file_history_ready = !self
            .dialogs
            .any(|d| matches!(d, Dialog::FileHistory { .. }))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.file_history.is_some());
        let lfs_ready = !self.dialogs.any(|d| matches!(d, Dialog::Lfs(_)))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.lfs.patterns.is_some() && t.lfs.locks.is_some());
        let stale_ready = !self.dialogs.any(|d| matches!(d, Dialog::CleanStale(_)))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.stale_result.is_some());
        let merged_ready = !self.dialogs.any(|d| matches!(d, Dialog::CleanMerged(_)))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.merged.is_some() && t.merged_applied == t.merged_serial);
        let base_ready = !self.dialogs.any(|d| matches!(d, Dialog::UpdateFromBase(_)))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.base_result.is_some());
        let worktree_ready = (!self.dialogs.any(|d| matches!(d, Dialog::RemoveWorktree(_)))
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.worktree_removal.is_some()))
            && (!self
                .dialogs
                .any(|d| matches!(d, Dialog::ForgetWorktrees(_)))
                || self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| t.worktree_forget.is_some()));
        let ready = worktree_ready
            && insights_ready
            && merged_ready
            && base_ready
            && shot.pending_dialog.is_none()
            && file_history_ready
            && lfs_ready
            && search_ready
            && stale_ready
            && demo_shown
            && self.pending_opens == 0
            && !crate::ui::art::growing(ctx)
            // A tab that is not shown has not loaded and never will in this run; the shown one
            // has to have.
            && self.tabs.get(self.active).is_none_or(RepoTab::is_loaded)
            && self.tabs.iter().filter(|t| t.is_loaded()).all(|t| {
                // A demo scene may be a loading or an error state on purpose: it is final.
                let requests_ready = demo.is_some()
                    || t.view != View::Requests
                    || t.host.is_none()
                    || (!t.requests.loading
                        && !t.requests.details_loading
                        && t.requests.list.is_some());
                t.snap.is_some()
                    && t.running.is_none()
                    && t.blame.as_ref().is_none_or(|b| b.result.is_some())
                    && (t.diff.is_some() || t.file.is_none())
                    && requests_ready
                    && !t.avatars.pending()
            });
        // A menu opens only once everything under it is final, and the picture waits for it.
        let ready = ready && shot.menu.as_mut().is_none_or(|menu| menu.opened(ctx));
        if ready {
            if let Some(id) = shot.popup {
                egui::Popup::open_id(ctx, id);
            }
            shot.ready_frames += 1;
            shot.ready_since.get_or_insert(now);
        } else {
            shot.ready_frames = 0;
            shot.ready_since = None;
        }
        let settled =
            shot.ready_frames > 30 && shot.ready_since.is_some_and(|since| now - since > 0.3);
        if !shot.requested && (settled || now - shot.started > 12.0) {
            shot.requested = true;
            if let Some(menu) = shot.menu.as_ref().filter(|m| m.click.is_none()) {
                tracing::warn!(
                    name = menu.name,
                    "no widget has this name: the picture shows no menu"
                );
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(image) = image {
            match save_png(&shot.path, &image) {
                Ok(()) => tracing::info!(path = %shot.path.display(), "screenshot saved"),
                Err(e) => tracing::error!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// `--overlay` names for the dialogs no other overlay opens (also listed in `OVERLAYS`);
/// [`dialog_overlay`] says what each shows.
#[cfg(test)]
const DIALOG_OVERLAYS: &[&str] = &[
    "open",
    "open-error",
    "clone",
    "clone-running",
    "clone-error",
    "init",
    "new-branch",
    "new-worktree",
    "new-worktree-error",
    "rename",
    "add-submodule",
    "git-missing",
    "remove-submodule",
    "discard-submodule",
    "new-tag",
    "stash",
    "export-patch",
    "apply-patch",
    "add-remote",
    "add-remote-refused",
    "publish",
    "new-request",
    "confirm",
    "confirm-danger",
    "confirm-host",
    "convert-partial-clone",
    "confirm-merge-risky",
    "diverged",
    "pull-conflicts",
    "pull-moved",
    "whats-new",
    "crash",
    "update-available",
    "update-failed",
    "terms",
    "consent-reading",
    "crash-reading",
    "username",
    "password",
    "approve",
    "notice",
];

impl CanopyApp {
    /// Opens the dialog `--overlay name` names (the names of `DIALOG_OVERLAYS`), with the state
    /// it shows.
    pub(super) fn open_dialog_overlay(&mut self, name: &str) {
        if name == "repository-insights" {
            self.dialogs.open(Dialog::Insights {
                repo: PathBuf::new(),
            });
            return;
        }
        if name == "file-history" || name == "file-history-commit" {
            self.dialogs.open(Dialog::FileHistory {
                repo: PathBuf::new(),
                path: String::new(),
                // A placeholder: the selected commit replaces it once the tab has one.
                from: (name == "file-history-commit").then_some(canopy_core::Oid::ZERO_SHA1),
            });
            return;
        }
        if let Some(page) = match name {
            "lfs" => Some(dialogs::LfsPage::Files),
            "lfs-patterns" => Some(dialogs::LfsPage::Patterns),
            "lfs-locks" => Some(dialogs::LfsPage::Locks),
            _ => None,
        } {
            // The repository of the first tab replaces the placeholder once it is open.
            self.dialogs
                .open(Dialog::Lfs(Box::new(dialogs::LfsForm::new(
                    PathBuf::new(),
                    page,
                ))));
            return;
        }
        if name == "search-changes" {
            self.dialogs.open(Dialog::SearchChanges {
                repo: PathBuf::new(),
                query: String::new(),
                regex: false,
                searched: false,
            });
            return;
        }
        if name == "clean-stale" {
            self.dialogs
                .open(Dialog::CleanStale(Box::new(dialogs::StaleForm::new(
                    PathBuf::new(),
                    self.settings.stale_inactivity_days,
                    0,
                ))));
            return;
        }
        if name == "update-failed" {
            self.updater.install = updater::InstallState::Failed {
                build: 1001,
                error: "The download's checksum does not match the release (expected 9f2c…e1, got 41aa…07).".into(),
            };
        }
        if let Some(d) = dialog_overlay(name, &self.settings) {
            self.dialogs.open(d);
        }
    }
}

/// The dialog `--overlay name` opens for a screenshot, filled with fixed illustrative content so
/// the picture never shows a fixture's temporary path; `None` for a name that is not a dialog's.
fn dialog_overlay(name: &str, settings: &Settings) -> Option<Dialog> {
    use crate::askpass::{Prompt, PromptKind};
    use crate::ui::legal::{Document, Pending};
    use canopy_core::forge::ForgeKind;

    let commit = canopy_core::Oid::from_str("38b14e7c2d9a4f61b0e5a7c3d2f1e0b9a8c7d6e5").ok()?;
    let available = |notes: &str| updater::Available {
        build: 1001,
        manifest: canopy_hosting::updates::UpdateManifest {
            version: "1001".into(),
            notes: notes.to_owned(),
            pub_date: Some("2026-10-04T09:30:00Z".into()),
            release_page: None,
            platforms: std::collections::BTreeMap::default(),
        },
    };
    let notes = "### Added\n\n- Settings search finds any option by name, description or synonym.\n- Repository settings: clone protocol, pull strategy and identity per repository.\n\n### Fixed\n\n- Author pictures load again for public GitHub repositories.";
    Some(match name {
        "open" | "open-error" => Dialog::Open {
            path: if name == "open" {
                "~/src/".into()
            } else {
                "~/Downloads".into()
            },
            recent: ["tidepool", "canopy", "lighthouse-api", "dotfiles"]
                .into_iter()
                .map(|n| PathBuf::from("/home/ada/src").join(n))
                .collect(),
            pending: None,
            error: (name == "open-error").then(|| dialogs::OpenFailure {
                message: "/home/ada/Downloads is not a Git repository".into(),
                not_a_repository: true,
                legacy_partial_clone: None,
            }),
        },
        "clone" | "clone-running" | "clone-error" => {
            let mut d = Dialog::clone_default(settings);
            if let Dialog::Clone {
                builder,
                parent,
                pending,
                error,
            } = &mut d
            {
                builder.project = "rust-lang/rustlings".into();
                *parent = "~/src".into();
                *pending = name == "clone-running";
                if name == "clone-error" {
                    *error = Some("fatal: destination path 'rustlings' already exists and is not an empty directory.".into());
                }
            }
            d
        }
        "init" => Dialog::new_repository("~/src/tidepool".into()),
        "new-branch" => Dialog::new_branch_at(Start::Head),
        "new-worktree" => Dialog::new_worktree(),
        "new-worktree-error" => Dialog::NewWorktree {
            create: false,
            name: String::new(),
            existing: "feature/auth".into(),
            folder: "~/src/fx-auth".into(),
            pending: false,
            error: Some("fatal: '/home/you/src/fx-auth' already exists".into()),
        },
        "rename" => Dialog::Rename {
            old: "feature/search".into(),
            new: "feature/search-v2".into(),
        },
        "git-missing" => Dialog::GitMissing,
        "add-submodule" => Dialog::AddSubmodule {
            url: "https://github.com/necturalabs/tidepool-ui.git".into(),
            path: String::new(),
            branch: String::new(),
        },
        "remove-submodule" => Dialog::Confirm {
            title: "Remove submodule vendor/tidepool-ui?".into(),
            body: "The folder and the submodule's own repository are deleted, including uncommitted changes and commits that were never pushed. The removal is staged, not committed.".into(),
            confirm: "Remove submodule".into(),
            danger: true,
            repo: PathBuf::new(),
            op: Op::SubmoduleRemove("vendor/tidepool-ui".into()),
        },
        "discard-submodule" => Dialog::Confirm {
            title: "Discard changes to vendor/tidepool-ui?".into(),
            body: changes::submodule_discard(false).into(),
            confirm: "Discard changes".into(),
            danger: true,
            repo: PathBuf::new(),
            op: Op::Discard(Vec::new()),
        },
        "new-tag" => Dialog::NewTag {
            name: "v1.4.0".into(),
            target: commit,
            message: String::new(),
        },
        "export-patch" => Dialog::ExportPatches {
            oid: commit,
            folder: "~/patches".into(),
            count: 3,
        },
        "apply-patch" => Dialog::ApplyPatches {
            path: "~/patches/0001-fix-shout-two.patch".into(),
        },
        "stash" => Dialog::Stash {
            message: String::new(),
            untracked: true,
        },
        "add-remote" | "add-remote-refused" | "publish" => {
            let mut d = Dialog::add_remote(settings);
            if name == "publish" {
                d = d.publishing();
            } else {
                // As the app names it beside the fixture's own origin.
                d.name_remote(&["origin".to_owned()]);
            }
            if name == "add-remote-refused"
                && let Dialog::AddRemote { builder, .. } = &mut d
            {
                // A URL with a password in it, which Canopy refuses to save.
                builder.source = dialogs::UrlSource::Url;
                builder.url = "https://ada:hunter2@git.example.com/acme/tidepool.git".into();
            }
            d
        }
        "new-request" => Dialog::NewRequest {
            kind: ForgeKind::GitHub,
            request: NewRequest {
                title: "Add settings search".into(),
                body: String::new(),
                source: "feature/settings-search".into(),
                target: "main".into(),
                draft: false,
            },
            branches: vec!["main".into(), "develop".into(), "release/2026.10".into()],
        },
        "confirm" => Dialog::Confirm {
            title: "Check out 38b14e7?".into(),
            body: "HEAD will point at this commit, not at a branch (a detached HEAD). Commits you make there belong to no branch until you create one.".into(),
            confirm: "Check out commit".into(),
            danger: false,
            repo: PathBuf::new(),
            op: Op::Checkout("38b14e7".into()),
        },
        "diverged" => Dialog::Diverged {
            repo: PathBuf::new(),
            mode: settings::PullMode::RECOMMENDED_WHEN_DIVERGED,
            remember: crate::action::Remember::Repository,
            everywhere: true,
            preview: canopy_core::PullPreview {
                upstream: "origin/main".into(),
                tip: None,
                ahead: 2,
                behind: 3,
                conflicts: Some(vec!["src/api/routes.rs".into()]),
                overlap: Vec::new(),
            },
        },
        "pull-conflicts" => Dialog::PullConflicts {
            repo: PathBuf::new(),
            strategy: canopy_core::PullStrategy::FastForwardOnly,
            moved: false,
            preview: canopy_core::PullPreview {
                upstream: "origin/main".into(),
                tip: None,
                ahead: 0,
                behind: 2,
                conflicts: None,
                overlap: vec!["README.md".into(), "src/api/routes.rs".into()],
            },
        },
        "pull-moved" => Dialog::PullConflicts {
            repo: PathBuf::new(),
            strategy: canopy_core::PullStrategy::Merge,
            moved: true,
            preview: canopy_core::PullPreview {
                upstream: "origin/main".into(),
                tip: None,
                ahead: 0,
                behind: 3,
                conflicts: None,
                overlap: Vec::new(),
            },
        },
        "confirm-danger" => Dialog::Confirm {
            title: "Delete branch feature/dark-mode?".into(),
            body: "feature/dark-mode has 1 commit that is on no other branch. You can restore it from the reflog for 90 days.".into(),
            confirm: "Delete branch".into(),
            danger: true,
            repo: PathBuf::new(),
            op: Op::DeleteBranch {
                name: "feature/dark-mode".into(),
                force: false,
            },
        },
        "convert-partial-clone" => Dialog::ConvertPartialClone {
            path: "/home/ada/src/tidepool".into(),
            remote: "origin".into(),
        },
        "confirm-host" => Dialog::ConfirmHost {
            title: "Approve pull request #428?".into(),
            body: "GitHub records your approval of Add settings search under your account.".into(),
            confirm: "Approve".into(),
            danger: false,
            job: HostJob::Approve(428),
        },
        "confirm-merge-risky" => Dialog::ConfirmHost {
            title: "Merge pull request #428 anyway?".into(),
            body: "Checks failed on Add settings search and a reviewer requested changes. Merging now puts it on main as it is.".into(),
            confirm: "Merge anyway".into(),
            danger: true,
            job: HostJob::Merge(428, canopy_hosting::MergeMethod::Merge, None),
        },
        "whats-new" => Dialog::WhatsNew {
            version: "build 1001".into(),
            notes: notes.into(),
        },
        "crash" => Dialog::Crash(crate::logging::CrashReport {
            path: PathBuf::from("/home/ada/.local/share/canopy/crashes/2026-10-05T18-22-41Z.txt"),
            message: "called `Option::unwrap()` on a `None` value at crates/canopy-app/src/views/history.rs:412:31".into(),
        }),
        "update-available" | "update-failed" => Dialog::Update(Box::new(available(notes))),
        "terms" => Dialog::Legal(Document::Terms),
        "consent-reading" => Dialog::Consent {
            step: Pending::Terms,
            updated: false,
            reading: Some(Document::Terms),
        },
        "crash-reading" => Dialog::Consent {
            step: Pending::CrashReports,
            updated: false,
            reading: Some(Document::Privacy),
        },
        "username" => Dialog::credential(Prompt::preview("Username for 'https://gitlab.example.com': ")),
        "password" => Dialog::credential(Prompt::preview("Password for 'https://ada@gitlab.example.com': ")),
        // ssh asks these through SSH_ASKPASS_PROMPT, which a preview's text cannot say.
        "approve" | "notice" => {
            let mut prompt = Prompt::preview(if name == "approve" {
                "Allow use of key /home/ada/.ssh/id_ed25519?\nKey fingerprint SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU."
            } else {
                "Confirm user presence for key ED25519-SK SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"
            });
            prompt.kind = if name == "approve" {
                PromptKind::Approve
            } else {
                PromptKind::Notice
            };
            Dialog::credential(prompt)
        }
        _ => return None,
    })
}

impl CanopyApp {
    /// A screenshot's input: no pointer (see [`without_pointer`]) but the right click that opens
    /// an `--overlay menu-*` menu.
    pub(super) fn screenshot_input(&mut self, input: &mut egui::RawInput) {
        let Some(shot) = &mut self.screenshot else {
            return;
        };
        without_pointer(input);
        if let Some(menu) = shot.menu.as_mut()
            && let Some((at, sent)) = menu.click.as_mut()
            && *sent < 2
        {
            input.events.retain(|e| *e != egui::Event::PointerGone);
            input.events.push(egui::Event::PointerMoved(*at));
            input.events.push(egui::Event::PointerButton {
                pos: *at,
                button: menu.button,
                pressed: *sent == 0,
                modifiers: egui::Modifiers::NONE,
            });
            *sent += 1;
        }
    }
}

/// Takes the pointer out of a screenshot's input, so the picture never depends on where the
/// virtual screen left it: Xvfb starts the pointer in the middle of the screen, where it would
/// hover (and give a tooltip to) whatever lies there.
fn without_pointer(input: &mut egui::RawInput) {
    input.events.retain(|event| {
        !matches!(
            event,
            egui::Event::PointerMoved(_)
                | egui::Event::MouseMoved(_)
                | egui::Event::PointerButton { .. }
                | egui::Event::MouseWheel { .. }
                | egui::Event::Touch { .. }
                | egui::Event::PointerGone
        )
    });
    input.events.push(egui::Event::PointerGone);
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        u32::try_from(image.size[0])?,
        u32::try_from(image.size[1])?,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_srgba_unmultiplied)
        .collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CR-5: a screenshot never sees the pointer. Xvfb starts it in the middle of the screen,
    /// where it hovered whatever lay there and put a tooltip into some pictures and not others.
    #[test]
    fn a_screenshot_ignores_the_pointer() {
        use eframe::App as _;
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.screenshot = Some(Screenshot::new(PathBuf::from("shot.png"), None));
        let at = egui::pos2(480.0, 300.0);
        let mut input = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Text("kept".into()),
            ],
            ..Default::default()
        };
        app.raw_input_hook(&ctx, &mut input);
        assert_eq!(
            input.events,
            [egui::Event::Text("kept".into()), egui::Event::PointerGone]
        );
        // A normal run keeps its pointer.
        app.screenshot = None;
        let mut input = egui::RawInput {
            events: vec![egui::Event::PointerMoved(at)],
            ..Default::default()
        };
        app.raw_input_hook(&ctx, &mut input);
        assert_eq!(input.events, [egui::Event::PointerMoved(at)]);
    }

    /// CR-6: `--overlay menu-*` opens its context menu by a right click on the widget it names,
    /// once the window has settled, and the picture waits for the menu.
    #[test]
    fn a_menu_overlay_opens_its_context_menu() {
        use eframe::App as _;
        // (The other menus need a repository's commits, branches or a loaded diff: their
        // pictures are checked with tools/screenshot.sh.)
        let (overlay, item) = ("menu-branch", "New branch from here…");
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.enable_accesskit();
        let mut app = test_app();
        app.settings.accepted_terms = crate::ui::legal::TERMS_VERSION.to_owned();
        app.settings.accepted_privacy = crate::ui::legal::PRIVACY_VERSION.to_owned();
        app.settings.crash_reports = Some(false);
        app.settings.crash_reports_version = crate::crash_reports::DATA_VERSION;
        app.settings.seen_intro = true;
        let mut tab = test_tab(canopy_core::RepoSnapshot {
            branches: vec![canopy_core::BranchInfo {
                name: "feature/auth".into(),
                target: None,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                worktree: None,
            }],
            ..Default::default()
        });
        app.screenshot = Some(Screenshot::new(PathBuf::from("shot.png"), Some(overlay)));
        if let Some(menu) = app.screenshot.as_ref().and_then(|s| s.menu.as_ref()) {
            tab.view = menu.view;
        }
        app.tabs.push(tab);
        let mut frame = eframe::Frame::_new_kittest();
        let mut shown = false;
        for i in 0..90 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(i) * 0.05),
                ..Default::default()
            };
            app.raw_input_hook(&ctx, &mut input);
            let mut out = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
            out.textures_delta.clear();
            shown = out.shapes.iter().any(|c| {
                fn has(shape: &egui::Shape, text: &str) -> bool {
                    match shape {
                        egui::Shape::Text(t) => t.galley.text() == text,
                        egui::Shape::Vec(v) => v.iter().any(|s| has(s, text)),
                        _ => false,
                    }
                }
                has(&c.shape, item)
            });
            if shown {
                break;
            }
        }
        assert!(shown, "--overlay {overlay} never showed {item:?}");
    }

    /// secfix2 F7: `--overlay add-remote-refused` shows Add remote with a URL Canopy refuses
    /// (credentials in it), so the refusal can be checked in a picture.
    #[test]
    fn the_refused_add_remote_overlay_shows_the_refusal() {
        let dialog = dialog_overlay("add-remote-refused", &Settings::default());
        let Some(Dialog::AddRemote { builder, .. }) = dialog else {
            panic!("--overlay add-remote-refused opens {dialog:?}");
        };
        assert!(builder.resolve().is_none());
        assert!(
            builder
                .problem()
                .is_some_and(|p| p.contains("password") || p.contains("credential")),
            "{:?}",
            builder.problem()
        );
        assert!(OVERLAYS.contains(&"add-remote-refused"));
    }

    #[test]
    fn every_dialog_overlay_opens_a_dialog_that_draws() {
        let settings = Settings::default();
        let updater = Updater::default();
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        for name in DIALOG_OVERLAYS {
            let mut dialog = dialog_overlay(name, &settings)
                .unwrap_or_else(|| panic!("--overlay {name} opens no dialog"));
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
                    dialogs::show(
                        &ctx,
                        &mut dialog,
                        None,
                        &mut picker,
                        false,
                        &updater,
                        "git version 2.55.0",
                    );
                });
                output.textures_delta.clear();
                assert!(!output.shapes.is_empty(), "--overlay {name} drew nothing");
            }
        }
        assert!(dialog_overlay("settings", &settings).is_none());
    }
}
