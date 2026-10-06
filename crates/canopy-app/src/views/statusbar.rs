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
                            without_icons(ui.label(small(
                                format!("{}  not published", icon::CLOUD_SLASH),
                                p.text_faint,
                            )));
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
                actions.push(Action::SettingsGroup(Section::Appearance, "Notifications"));
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

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
