//! The status bar: branch, sync state, repository state and the running job.

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::kit::chip::Chip;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::{Size, Tone, menu};
use crate::ui::theme::{self, palette};
use crate::ui::toast::Toasts;
use crate::updater::{InstallState, UpdateState};
use crate::views::commit_detail::without_icons;
use crate::views::dialogs::Dialog;
use crate::views::settings_screen::Section;

/// Room between the status bar's items, and at its ends.
const ITEM_GAP: f32 = 14.0;
const END_PAD: f32 = 10.0;

/// A clickable chip that keeps the spoken name it had as a framed button, which says what a click
/// does more plainly than the chip's text.
fn chip(ui: &mut Ui, chip: Chip, spoken: &str) -> egui::Response {
    let response = chip.show(ui);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken));
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
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal_centered(|ui| {
        ui.add_space(END_PAD);
        ui.spacing_mut().item_spacing.x = ITEM_GAP;
        let small = |t: String, c| RichText::new(t).size(m.meta_size).color(c);
        if let Some(tab) = tab {
            if let Some(s) = &tab.snap {
                without_icons(ui.label(small(
                    format!(
                        "{}  {}{}",
                        icon::GIT_BRANCH,
                        s.head.label(),
                        if s.head.branch.is_some() && s.head.oid.is_none() {
                            " (no commits yet)"
                        } else {
                            ""
                        }
                    ),
                    p.text_dim,
                )));
                if let Some(b) = s.current_branch() {
                    match &b.upstream {
                        Some(u) if b.ahead == 0 && b.behind == 0 => {
                            without_icons(ui.label(small(
                                format!("{}  up to date with {u}", icon::CHECK),
                                p.text_faint,
                            )));
                        }
                        Some(u) => {
                            let spoken = format!("{} ahead, {} behind {u}", b.ahead, b.behind);
                            ui.label(small(format!("↑{} ↓{}  {u}", b.ahead, b.behind), p.accent2))
                                .widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Label,
                                        true,
                                        &spoken,
                                    )
                                });
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
                            let (text, tip) = unpublished(s, b);
                            without_icons(ui.label(small(
                                format!("{}  {text}", icon::CLOUD_SLASH),
                                p.text_faint,
                            )))
                            .kit_tooltip(tip)
                            .extra(true)
                            .show();
                        }
                    }
                }
                if s.state != canopy_core::RepoState::Clean {
                    without_icons(ui.label(small(
                        format!("{}  {}", icon::WARNING, s.state.label()),
                        p.warning,
                    )));
                }
                let n = s.status.len();
                if n > 0 {
                    without_icons(ui.label(small(
                        format!("{}  {n} changed", icon::PENCIL_SIMPLE_LINE),
                        p.text_faint,
                    )));
                }
            }
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
            if let Some(remote) = &tab.downloads_refused {
                let resp = chip(
                    ui,
                    Chip::new("Downloads off  ·  Trust…")
                        .icon(icon::SHIELD_WARNING)
                        .tone(Tone::Warning)
                        .id_salt("status-trust")
                        .tooltip(format!(
                            "This partial clone needs files from {remote}. Canopy downloads them \
                             only for repositories in a trusted folder. Click to choose."
                        )),
                    "Downloads off, trust this folder",
                );
                if resp.clicked() {
                    actions.push(Action::OpenDialog(Dialog::TrustFolder {
                        repo: tab.path.clone(),
                        remote: remote.clone(),
                    }));
                }
            }
        } else {
            without_icons(ui.label(small(
                format!("{}  No repository open", icon::FOLDER_OPEN),
                p.text_dim,
            )));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                without_icons(
                    ui.label(small(format!("{}  {git_version}", icon::WARNING), p.danger)),
                )
                .kit_tooltip("Install git and make sure it is on PATH")
                .show();
            }
        });
    });
}

/// What the status bar says about a branch with no upstream, and a tooltip for it: that it is
/// not published, or, when a remote branch of the same name exists, that it is there but not
/// tracked.
fn unpublished(
    snap: &canopy_core::RepoSnapshot,
    branch: &canopy_core::BranchInfo,
) -> (String, String) {
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
    (text, tip)
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
}
