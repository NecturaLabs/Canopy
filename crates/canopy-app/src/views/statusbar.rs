//! The status bar: branch, sync state, repository state and the running job.

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::theme::{self, Palette, palette};
use crate::updater::{InstallState, UpdateState};
use crate::views::dialogs::Dialog;

/// Draws the status bar.
pub fn show(
    ui: &mut Ui,
    tab: Option<&RepoTab>,
    git_version: &str,
    update: &UpdateState,
    install: &InstallState,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    ui.horizontal_centered(|ui| {
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.x = 14.0;
        let small = |t: String, c| RichText::new(t).size(theme::META_SIZE).color(c);
        if let Some(tab) = tab {
            if let Some(s) = &tab.snap {
                ui.label(small(
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
                ));
                if let Some(b) = s.current_branch() {
                    match &b.upstream {
                        Some(u) if b.ahead == 0 && b.behind == 0 => {
                            ui.label(small(
                                format!("{}  up to date with {u}", icon::CHECK),
                                p.text_faint,
                            ));
                        }
                        Some(u) => {
                            ui.label(small(format!("↑{} ↓{}  {u}", b.ahead, b.behind), p.accent2));
                        }
                        None if s.remotes.is_empty() => {
                            ui.label(small(
                                format!("{}  local only", icon::HARD_DRIVES),
                                p.text_faint,
                            ))
                            .on_hover_text("This repository has no remote");
                        }
                        None => {
                            ui.label(small(
                                format!("{}  not published", icon::CLOUD_SLASH),
                                p.text_faint,
                            ));
                        }
                    }
                }
                if s.state != canopy_core::RepoState::Clean {
                    ui.label(small(
                        format!("{}  {}", icon::WARNING, s.state.label()),
                        p.warning,
                    ));
                }
                let n = s.status.len();
                if n > 0 {
                    ui.label(small(
                        format!("{}  {n} changed", icon::PENCIL_SIMPLE_LINE),
                        p.text_faint,
                    ));
                }
            }
            if let Some(job) = &tab.running {
                ui.add(egui::Spinner::new().size(theme::META_SIZE).color(p.accent));
                let mut t = job.clone();
                if let Some(pr) = &tab.progress {
                    t.push_str("  ·  ");
                    t.push_str(pr);
                }
                ui.label(small(t, p.accent));
            } else if let Some(line) = &tab.progress {
                // A download for a read in a partial clone, outside any operation.
                ui.add(egui::Spinner::new().size(theme::META_SIZE).color(p.accent));
                ui.label(small(line.clone(), p.accent));
            }
            if let Some(remote) = &tab.downloads_refused {
                let resp = egui::Frame::new()
                    .fill(Palette::tint(p.warning, 40))
                    .corner_radius(egui::CornerRadius::same(9))
                    .inner_margin(egui::Margin::symmetric(8, 1))
                    .show(ui, |ui| {
                        ui.label(small(
                            format!("{}  Downloads off  ·  Trust…", icon::SHIELD_WARNING),
                            p.warning,
                        ))
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text(format!(
                        "This partial clone needs files from {remote}. Canopy downloads them \
                         only for repositories in a trusted folder. Click to choose."
                    ));
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        "Downloads off, trust this folder",
                    )
                });
                if resp.clicked() {
                    actions.push(Action::OpenDialog(Dialog::TrustFolder {
                        repo: tab.path.clone(),
                        remote: remote.clone(),
                    }));
                }
            }
        } else {
            ui.label(small(
                format!("{}  No repository open", icon::FOLDER_OPEN),
                p.text_dim,
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            if let InstallState::Running { build, .. } = install {
                ui.add(egui::Spinner::new().size(theme::META_SIZE).color(p.accent));
                ui.label(small(
                    format!("Updating to {}…", crate::build_info::name(build)),
                    p.accent,
                ));
            } else if let InstallState::Done {
                installed: crate::selfupdate::Installed { exe: Some(exe), .. },
                ..
            } = install
            {
                let resp = egui::Frame::new()
                    .fill(Palette::tint(p.accent, 50))
                    .corner_radius(egui::CornerRadius::same(9))
                    .inner_margin(egui::Margin::symmetric(8, 1))
                    .show(ui, |ui| {
                        ui.label(small(
                            format!("{}  Restart to finish updating", icon::ARROW_CLOCKWISE),
                            p.text,
                        ))
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text("Close Canopy and start the updated build");
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        "Restart to finish updating",
                    )
                });
                if resp.clicked() {
                    actions.push(Action::RestartInto(exe.clone()));
                }
            } else if let UpdateState::Available(a) = update {
                let v = a.label();
                let resp = egui::Frame::new()
                    .fill(Palette::tint(p.accent, 50))
                    .corner_radius(egui::CornerRadius::same(9))
                    .inner_margin(egui::Margin::symmetric(8, 1))
                    .show(ui, |ui| {
                        ui.label(small(
                            format!("{}  Canopy {v} is available", icon::ARROW_CIRCLE_UP),
                            p.text,
                        ))
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text("See what's new and download it");
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Canopy {v} is available"),
                    )
                });
                let version = v.to_string();
                let page = a.page();
                resp.context_menu(|ui| {
                    if ui
                        .button(format!(
                            "{}  Release notes and downloads",
                            icon::ARROW_SQUARE_OUT
                        ))
                        .clicked()
                    {
                        actions.push(Action::OpenUrl(page.clone()));
                    }
                    if ui
                        .button(format!("{}  Skip this version", icon::X))
                        .clicked()
                    {
                        actions.push(Action::SkipVersion(version.clone()));
                    }
                });
                if resp.clicked() {
                    actions.push(Action::ShowUpdate);
                }
            }
            // The version itself is in About; here only its absence, which breaks every change.
            if !git_version.starts_with("git version") {
                ui.label(small(format!("{}  {git_version}", icon::WARNING), p.danger))
                    .on_hover_text("Install git and make sure it is on PATH");
            }
        });
    });
}
