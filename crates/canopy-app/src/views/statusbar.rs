//! The status bar: branch, sync state, repository state and the running job.

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::tab::RepoTab;
use crate::ui::theme::palette;

/// Draws the status bar.
pub fn show(ui: &mut Ui, tab: Option<&RepoTab>, git_version: &str) {
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
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            ui.label(small(git_version.to_owned(), p.text_faint));
            if let Some(tab) = tab {
                ui.label(small(tab.path.display().to_string(), p.text_faint));
            }
        });
    });
}
