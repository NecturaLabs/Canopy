//! The status bar: branch, sync state, repository state and the running job.

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::theme::{Palette, palette};
use crate::updater::UpdateState;

/// Draws the status bar.
pub fn show(
    ui: &mut Ui,
    tab: Option<&RepoTab>,
    git_version: &str,
    update: &UpdateState,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    ui.horizontal_centered(|ui| {
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.x = 14.0;
        let small = |t: String, c| RichText::new(t).size(12.0).color(c);
        if let Some(tab) = tab {
            if let Some(s) = &tab.snap {
                ui.label(small(
                    format!("{}  {}", icon::GIT_BRANCH, s.head.label()),
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
                ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                let mut t = job.clone();
                if let Some(pr) = &tab.progress {
                    t.push_str("  ·  ");
                    t.push_str(pr);
                }
                ui.label(small(t, p.accent));
            } else if let Some(line) = &tab.progress {
                // A download for a read in a partial clone, outside any operation.
                ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                ui.label(small(line.clone(), p.accent));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            if let UpdateState::Available(a) = update {
                let v = &a.version;
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
            ui.label(small(git_version.to_owned(), p.text_faint));
        });
    });
}
