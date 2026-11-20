//! The status bar: branch, sync state, repository state and the running job.

use egui::{FontId, Layout, Rect, RichText, Ui, UiBuilder};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::kit::chip::Chip;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::{Size, Tone, menu};
use crate::ui::theme::{self, palette};
use crate::ui::toast::Toasts;
use crate::ui::widgets::{elide_middle, text_width};
use crate::updater::{InstallState, UpdateState};
use crate::views::commit_detail::without_icons;
use crate::views::dialogs::Dialog;
use crate::views::settings_screen::Section;
use crate::worker::Op;

/// Room between the status bar's items, and at its ends.
const ITEM_GAP: f32 = 14.0;
const END_PAD: f32 = 10.0;
/// The most of the bar the branch name takes, as a share of its width and in points; a longer
/// one is cut in its middle.
const BRANCH_SHARE: f32 = 0.32;
const BRANCH_MAX: f32 = 320.0;
/// The same for the upstream's name in the sync state.
const UPSTREAM_SHARE: f32 = 0.24;
const UPSTREAM_MAX: f32 = 240.0;

/// `before`, `name` and `after` on one line no wider than `max` in `font`, `name` cut in its
/// middle when the whole would be wider; and whether it was cut.
fn fit(ui: &Ui, font: &FontId, max: f32, before: &str, name: &str, after: &str) -> (String, bool) {
    let room = max - text_width(ui, before, font) - text_width(ui, after, font);
    let shown = elide_middle(ui, name, font, room.max(0.0));
    let cut = shown != name;
    (format!("{before}{shown}{after}"), cut)
}

/// A clickable chip that keeps the spoken name it had as a framed button, which says what a click
/// does more plainly than the chip's text.
fn chip(ui: &mut Ui, chip: Chip, spoken: &str) -> egui::Response {
    let response = chip.show(ui);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken));
    response
}

/// A label whose spoken name is `spoken` rather than its text, which may be cut or carry
/// icons.
fn spoken_label(ui: &mut Ui, text: RichText, spoken: &str) -> egui::Response {
    let response = ui.label(text);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, spoken));
    response
}

/// The running-job spinner.
fn spinner(ui: &mut Ui, what: &str) {
    Spinner::new().size(Size::Xs).label(what).show(ui);
}

/// Draws the status bar.
pub fn show(
    ui: &mut Ui,
    tab: Option<&RepoTab>,
    git_version: &str,
    update: &UpdateState,
    install: &InstallState,
    notifications: &mut Toasts,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_centered(|ui| {
        // The right-hand cluster (the bell, an update) always keeps its room: the left side gets
        // what it left last frame and is clipped there, so a long name can never push it off.
        let bar = ui.max_rect();
        let right_id = ui.id().with("status-right-width");
        let right_width: f32 = ui.ctx().data(|d| d.get_temp(right_id)).unwrap_or(0.0);
        let left_rect = Rect::from_min_max(
            bar.min,
            egui::pos2(
                (bar.max.x - right_width - ITEM_GAP).max(bar.min.x),
                bar.max.y,
            ),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(left_rect)
                .layout(Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.shrink_clip_rect(left_rect);
                left(ui, tab, bar.width(), actions);
            },
        );
        let right = ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            right_cluster(ui, git_version, update, install, notifications, actions);
        });
        let width = bar.max.x - right.response.rect.min.x;
        if (width - right_width).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(right_id, width));
            ui.ctx()
                .request_discard("the status bar's right-hand cluster changed width");
        }
    });
}

/// The status bar's left side: branch, sync state, repository state and the running job, with
/// names capped to their share of the bar's `bar_width`.
fn left(ui: &mut Ui, tab: Option<&RepoTab>, bar_width: f32, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let font = FontId::proportional(m.meta_size);
    let branch_max = (bar_width * BRANCH_SHARE).min(BRANCH_MAX);
    let upstream_max = (bar_width * UPSTREAM_SHARE).min(UPSTREAM_MAX);
    ui.add_space(END_PAD);
    ui.spacing_mut().item_spacing.x = ITEM_GAP;
    let small = |t: String, c| RichText::new(t).size(m.meta_size).color(c);
    if let Some(tab) = tab {
        if let Some(s) = &tab.snap {
            let name = s.head.label();
            let before = format!("{}  ", icon::GIT_BRANCH);
            let after = if s.head.branch.is_some() && s.head.oid.is_none() {
                " (no commits yet)"
            } else {
                ""
            };
            let (shown, cut) = fit(ui, &font, branch_max, &before, &name, after);
            let spoken = format!("{name}{after}");
            let label = spoken_label(ui, small(shown, p.text_dim), &spoken);
            if cut {
                label.kit_tooltip(name.as_str()).show();
            }
            if let Some(b) = s.current_branch() {
                whole(ui, "sync", |ui| match &b.upstream {
                    Some(u) if b.ahead == 0 && b.behind == 0 => {
                        let before = format!("{}  up to date with ", icon::CHECK);
                        let (shown, cut) = fit(ui, &font, upstream_max, &before, u, "");
                        let spoken = format!("up to date with {u}");
                        let label = spoken_label(ui, small(shown, p.text_faint), &spoken);
                        if cut {
                            label.kit_tooltip(spoken.as_str()).show();
                        }
                    }
                    Some(u) => {
                        let spoken = format!("{} ahead, {} behind {u}", b.ahead, b.behind);
                        let before = format!("↑{} ↓{}  ", b.ahead, b.behind);
                        let (shown, cut) = fit(ui, &font, upstream_max, &before, u, "");
                        let label = spoken_label(ui, small(shown, p.accent2), &spoken);
                        if cut {
                            label.kit_tooltip(spoken.as_str()).show();
                        }
                    }
                    None if s.remotes.is_empty() => {
                        without_icons(ui.label(small(
                            format!("{}  local only", icon::HARD_DRIVES),
                            p.text_faint,
                        )))
                        .kit_tooltip("This repository has no remote")
                        .extra(true)
                        .show();
                    }
                    None => {
                        let (text, tip, name) = unpublished(s, b);
                        let before = format!("{}  ", icon::CLOUD_SLASH);
                        let shown = match name
                            .as_deref()
                            .and_then(|n| text.split_once(n).map(|(head, tail)| (n, head, tail)))
                        {
                            Some((n, head, tail)) => {
                                let lead = format!("{before}{head}");
                                fit(ui, &font, upstream_max, &lead, n, tail).0
                            }
                            None => format!("{before}{text}"),
                        };
                        spoken_label(ui, small(shown, p.text_faint), &text)
                            .kit_tooltip(tip)
                            .extra(true)
                            .show();
                    }
                });
            }
            if s.state != canopy_core::RepoState::Clean {
                whole(ui, "state", |ui| {
                    without_icons(ui.label(small(
                        format!("{}  {}", icon::WARNING, s.state.label()),
                        p.warning,
                    )));
                });
            }
            let n = s.status.len();
            if n > 0 {
                whole(ui, "changed", |ui| {
                    without_icons(ui.label(small(
                        format!("{}  {n} changed", icon::PENCIL_SIMPLE_LINE),
                        p.text_faint,
                    )));
                });
            }
        }
        whole(ui, "job", |ui| {
            if let Some(job) = &tab.running {
                spinner(ui, job);
                let mut t = job.clone();
                if let Some(pr) = &tab.progress {
                    t.push_str("  ·  ");
                    t.push_str(pr);
                }
                ui.label(small(t, p.accent));
            } else if let Some(line) = &tab.progress {
                // A download for a read in a partial clone, outside any operation.
                spinner(ui, "files");
                ui.label(small(line.clone(), p.accent));
            }
        });
        let missing = tab.lfs.not_downloaded();
        if missing > 0 && tab.running.is_none() {
            whole(ui, "lfs", |ui| {
                let label = format!(
                    "{missing} LFS file{} not downloaded  ·  Download",
                    if missing == 1 { "" } else { "s" }
                );
                let reason = tab.lfs.unavailable();
                let resp = chip(
                    ui,
                    Chip::new(label.clone())
                        .icon(icon::CLOUD_SLASH)
                        .tone(Tone::Warning)
                        .id_salt("status-lfs")
                        .tooltip(reason.clone().unwrap_or_else(|| {
                            "These files hold only a pointer. Click to download their content \
                         (git lfs pull)."
                                .to_owned()
                        })),
                    &label,
                );
                if resp.clicked() {
                    // Without git-lfs the dialog says why and what to install.
                    actions.push(if reason.is_some() {
                        Action::Lfs(crate::views::dialogs::LfsPage::Files)
                    } else {
                        Action::Op(Op::LfsPull)
                    });
                }
            });
        }
        if let Some(remote) = &tab.downloads_refused {
            whole(ui, "trust", |ui| {
                let resp = chip(
                    ui,
                    Chip::new("Downloads off  ·  Trust…")
                        .icon(icon::SHIELD_WARNING)
                        .tone(Tone::Warning)
                        .id_salt("status-trust")
                        .tooltip(format!(
                            "This partial clone needs files from {remote}. Canopy downloads them \
                         only for a trusted repository. Click to choose."
                        )),
                    "Downloads off, trust this repository",
                );
                if resp.clicked() {
                    actions.push(Action::OpenDialog(Dialog::Trust {
                        repo: tab.trust_folders.first().unwrap_or(&tab.path).clone(),
                        folders: tab.trust_folders.clone(),
                    }));
                }
            });
        }
    } else {
        without_icons(ui.label(small(
            format!("{}  No repository open", icon::FOLDER_OPEN),
            p.text_dim,
        )));
    }
}

/// Draws one item of the status bar's left side only when it fits whole in what is left of the
/// bar, so a narrow window leaves an item out rather than cutting it mid-word. The item's width
/// is the one it had when last drawn; a changed width is drawn again in a discarded pass.
fn whole(ui: &mut Ui, key: &str, add: impl FnOnce(&mut Ui)) {
    let id = ui.id().with(("status-item", key));
    let known: Option<f32> = ui.ctx().data(|d| d.get_temp(id));
    let room = ui.available_width();
    if known.is_some_and(|width| width > room) {
        return;
    }
    let width = ui.scope(add).response.rect.width();
    if known != Some(width) {
        ui.ctx().data_mut(|d| d.insert_temp(id, width));
        if width > room {
            ui.ctx().request_discard("a status bar item did not fit");
        }
    }
}

/// The status bar's right-hand cluster, laid out right to left: the bell, then an update or a
/// missing git.
fn right_cluster(
    ui: &mut Ui,
    git_version: &str,
    update: &UpdateState,
    install: &InstallState,
    notifications: &mut Toasts,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let small = |t: String, c| RichText::new(t).size(m.meta_size).color(c);
    ui.spacing_mut().item_spacing.x = ITEM_GAP;
    ui.add_space(END_PAD);
    if notifications.bell(ui).value {
        actions.push(Action::SettingsGroup(
            Section::Notifications,
            "Notifications",
        ));
    }
    if let InstallState::Running { build, .. } = install {
        let name = crate::build_info::name(build);
        ui.label(small(format!("Updating to {name}…"), p.accent));
        spinner(ui, "the update");
    } else if let InstallState::Done {
        installed: crate::selfupdate::Installed { exe: Some(exe), .. },
        ..
    } = install
    {
        let resp = chip(
            ui,
            Chip::new("Restart to finish updating")
                .icon(icon::ARROW_CLOCKWISE)
                .tone(Tone::Accent)
                .id_salt("status-restart")
                .tooltip("Close Canopy and start the updated build"),
            "Restart to finish updating",
        );
        if resp.clicked() {
            actions.push(Action::RestartInto(exe.clone()));
        }
    } else if let UpdateState::Available(a) = update {
        let v = a.label();
        let label = format!("Canopy {v} is available");
        let resp = chip(
            ui,
            Chip::new(label.clone())
                .icon(icon::ARROW_CIRCLE_UP)
                .tone(Tone::Accent)
                .id_salt("status-update")
                .tooltip("See what's new and download it"),
            &label,
        );
        let page = a.page();
        menu::context(&resp, |m| {
            if m.item(icon::ARROW_SQUARE_OUT, "Release notes and downloads")
                .show(m.ui)
                .clicked()
            {
                actions.push(Action::OpenUrl(page.clone()));
            }
            if m.item(icon::X, "Skip this version").show(m.ui).clicked() {
                actions.push(Action::SkipVersion(a.skip_key()));
            }
        });
        if resp.clicked() {
            actions.push(Action::ShowUpdate);
        }
    }
    // The version itself is in About; here only its absence, which breaks every change.
    if !git_version.starts_with("git version") {
        without_icons(ui.label(small(format!("{}  {git_version}", icon::WARNING), p.danger)))
            .kit_tooltip("Install git and make sure it is on PATH")
            .show();
    }
}

/// What the status bar says about a branch with no upstream, and a tooltip for it: that it is
/// not published, or, when a remote branch of the same name exists, that it is there but not
/// tracked.
fn unpublished(
    snap: &canopy_core::RepoSnapshot,
    branch: &canopy_core::BranchInfo,
) -> (String, String, Option<String>) {
    // Where a push without an upstream goes (`Repo::push`): `origin`, else the only remote.
    let pushes_to = match snap.remotes.as_slice() {
        remotes if remotes.iter().any(|r| r.name == "origin") => Some("origin"),
        [only] => Some(only.name.as_str()),
        _ => None,
    };
    let there = snap
        .remote_branches
        .iter()
        .filter(|r| r.branch == branch.name)
        .min_by_key(|r| Some(r.remote.as_str()) != pushes_to);
    let Some(there) = there else {
        return (
            "not published".to_owned(),
            "No remote has this branch. Push publishes it.".to_owned(),
            None,
        );
    };
    let name = format!("{}/{}", there.remote, there.branch);
    let same = branch.target.is_some() && branch.target == there.target;
    let text = if same {
        format!("same as {name}, not tracking it")
    } else {
        format!("{name} exists, not tracking it")
    };
    let tip = if same && pushes_to == Some(there.remote.as_str()) {
        format!(
            "This branch has no upstream. Push sets {name}, at the same commit, as its upstream."
        )
    } else if let Some(remote) = pushes_to.filter(|r| *r != there.remote) {
        format!(
            "This branch has no upstream, and {name} is not where Push goes: it publishes the \
             branch to {remote}/{} and sets that as its upstream.",
            branch.name
        )
    } else if pushes_to.is_none() {
        format!(
            "This branch has no upstream, and {name} exists. With several remotes and no origin \
             Push refuses: set one with git push -u."
        )
    } else {
        format!("This branch has no upstream, and {name} is at another commit.")
    };
    (text, tip, Some(name))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A branch with no upstream is "not published" only when no remote has it.
    #[test]
    fn an_untracked_branch_says_whether_a_remote_has_it() {
        let oid = |hex: &str| canopy_core::Oid::from_str(hex).expect("an id");
        let (a, b) = (
            oid("1111111111111111111111111111111111111111"),
            oid("2222222222222222222222222222222222222222"),
        );
        let branch = canopy_core::BranchInfo {
            name: "feature/graph".into(),
            target: Some(a),
            is_head: true,
            upstream: None,
            ahead: 0,
            behind: 0,
            worktree: None,
        };
        let remote = |remote: &str, branch: &str, target| canopy_core::RemoteBranchInfo {
            remote: remote.into(),
            branch: branch.into(),
            target: Some(target),
        };
        let mut snap = canopy_core::RepoSnapshot::default();
        assert_eq!(unpublished(&snap, &branch).0, "not published");
        snap.remote_branches = vec![remote("origin", "main", a)];
        assert_eq!(unpublished(&snap, &branch).0, "not published");
        snap.remote_branches = vec![remote("origin", "feature/graph", a)];
        assert_eq!(
            unpublished(&snap, &branch).0,
            "same as origin/feature/graph, not tracking it"
        );
        snap.remote_branches = vec![remote("origin", "feature/graph", b)];
        assert_eq!(
            unpublished(&snap, &branch).0,
            "origin/feature/graph exists, not tracking it"
        );
    }

    /// The tooltip promises an upstream only where a push without one would really set it: to
    /// `origin`, else the only remote, and not at all with several remotes and no `origin`.
    #[test]
    fn the_untracked_tooltip_promises_only_what_the_push_does() {
        let oid =
            canopy_core::Oid::from_str("1111111111111111111111111111111111111111").expect("an id");
        let branch = canopy_core::BranchInfo {
            name: "feature/graph".into(),
            target: Some(oid),
            is_head: true,
            upstream: None,
            ahead: 0,
            behind: 0,
            worktree: None,
        };
        let tip = |remotes: &[&str], branch_on: &str| {
            let snap = canopy_core::RepoSnapshot {
                remotes: remotes
                    .iter()
                    .map(|name| canopy_core::RemoteInfo {
                        name: (*name).into(),
                        url: None,
                        default_branch: None,
                    })
                    .collect(),
                remote_branches: vec![canopy_core::RemoteBranchInfo {
                    remote: branch_on.into(),
                    branch: "feature/graph".into(),
                    target: Some(oid),
                }],
                ..Default::default()
            };
            unpublished(&snap, &branch).1
        };
        let promise = "Push sets fork/feature/graph";
        assert!(tip(&["fork"], "fork").contains(promise), "the only remote");
        let several = tip(&["origin", "fork"], "fork");
        assert!(!several.contains(promise), "{several}");
        assert!(several.contains("origin/feature/graph"), "{several}");
        let none = tip(&["upstream", "fork"], "fork");
        assert!(!none.contains(promise), "{none}");
        assert!(none.contains("git push -u"), "{none}");
        assert!(
            tip(&["origin", "fork"], "origin").contains("Push sets origin/feature/graph"),
            "origin is where it goes"
        );
    }

    /// A11Y-16 (adopts the a11y draft `a11y_5`): no icon glyph is part of a spoken name in the
    /// status bar, and the sync state reads as words.
    #[test]
    fn the_status_bar_speaks_no_icon_glyphs() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx.enable_accesskit();
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let mut snap = canopy_core::RepoSnapshot {
            state: canopy_core::RepoState::Merge,
            ..Default::default()
        };
        snap.head.branch = Some("main".into());
        snap.branches.push(canopy_core::BranchInfo {
            name: "main".into(),
            is_head: true,
            upstream: Some("origin/main".into()),
            ahead: 2,
            behind: 1,
            target: None,
            worktree: None,
        });
        tab.snap = Some(Arc::new(snap));
        let mut spoken = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(
                    ui,
                    Some(&tab),
                    "git: command not found",
                    &UpdateState::default(),
                    &InstallState::default(),
                    &mut Toasts::default(),
                    &mut Vec::new(),
                );
            });
            out.textures_delta.clear();
            spoken = out
                .platform_output
                .accesskit_update
                .map(|u| {
                    u.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
        }
        let glyphs: Vec<_> = spoken
            .iter()
            .filter(|l| l.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)))
            .collect();
        assert!(glyphs.is_empty(), "labels with icon glyphs: {glyphs:?}");
        for wanted in ["main", "2 ahead, 1 behind origin/main", "Merging"] {
            assert!(
                spoken.iter().any(|l| l.starts_with(wanted)),
                "{wanted:?} not in {spoken:?}"
            );
        }
    }
    /// A branch name too long for the bar is cut in its middle to its share of the bar, and the
    /// rest of the bar, the bell included, stays on screen (audit F1).
    #[test]
    fn a_long_branch_name_never_pushes_the_bell_off_the_bar() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx.enable_accesskit();
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let long = format!("feature/{}-end", "very-long-branch-name-".repeat(6));
        assert!(long.chars().count() > 140);
        let mut snap = canopy_core::RepoSnapshot::default();
        snap.head.branch = Some(long.clone());
        snap.remotes.push(canopy_core::RemoteInfo {
            name: "origin".into(),
            url: None,
            default_branch: None,
        });
        snap.branches.push(canopy_core::BranchInfo {
            name: long.clone(),
            is_head: true,
            upstream: None,
            ahead: 0,
            behind: 0,
            target: None,
            worktree: None,
        });
        snap.status.unstaged = (0..3)
            .map(|i| canopy_core::FileEntry {
                path: format!("file{i}.txt"),
                old_path: None,
                kind: canopy_core::ChangeKind::Modified,
                additions: 0,
                deletions: 0,
                lfs: false,
                old_mode: None,
                new_mode: None,
            })
            .collect();
        tab.snap = Some(Arc::new(snap));
        for width in [900.0, 640.0, 420.0] {
            let nodes = bar_nodes(&ctx, &tab, width);
            let find = |name: &str| {
                nodes
                    .iter()
                    .find(|(l, _)| l.starts_with(name))
                    .map_or_else(|| panic!("{name:?} not in {nodes:?}"), |(_, b)| *b)
            };
            let bell = find("Notifications");
            assert!(
                bell.x0 >= 0.0 && bell.x1 <= f64::from(width),
                "{width}: bell at {bell:?}"
            );
            let branch = find(&long);
            assert!(
                branch.x1 - branch.x0 <= f64::from(BRANCH_MAX),
                "branch {branch:?}"
            );
            // Every other item is drawn whole before the bell, or left out; none is cut.
            for (name, at) in nodes
                .iter()
                .filter(|(l, _)| !l.starts_with("Notifications"))
            {
                assert!(
                    at.x1 <= bell.x0,
                    "{width}: {name:?} at {at:?} runs into {bell:?}"
                );
            }
            if width >= 900.0 {
                find("not published");
                find("3 changed");
            }
        }
    }

    /// The status bar laid out `width` points wide, settled over a few frames: its spoken names
    /// with their bounds.
    fn bar_nodes(
        ctx: &egui::Context,
        tab: &RepoTab,
        width: f32,
    ) -> Vec<(String, egui::accesskit::Rect)> {
        let bar = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 28.0));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(bar),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(
                    ui,
                    Some(tab),
                    "git version 2.50.0",
                    &UpdateState::default(),
                    &InstallState::default(),
                    &mut Toasts::default(),
                    &mut Vec::new(),
                );
            });
            out.textures_delta.clear();
            nodes = out
                .platform_output
                .accesskit_update
                .map(|u| {
                    u.nodes
                        .iter()
                        .filter_map(|(_, n)| {
                            Some((n.label().or_else(|| n.value())?.to_owned(), n.bounds()?))
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
        nodes
    }
}
