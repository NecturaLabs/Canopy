//! The update dialog: what is new, installing it, and restarting into it.

use super::*;
use crate::selfupdate::{Install, Phase};
use crate::updater::{Available, InstallState, Updater};

/// "12.3 MB", or "12.3 of 45.6 MB" while the size is known.
fn megabytes(done: u64, total: Option<u64>) -> String {
    let mb = |n: u64| n as f64 / (1024.0 * 1024.0);
    match total {
        Some(total) => format!("{:.1} of {:.1} MB", mb(done), mb(total)),
        None => format!("{:.1} MB", mb(done)),
    }
}

/// The update dialog for `a`. `updater` says how this copy was installed and where an install
/// started from here stands.
pub(super) fn update(ui: &mut Ui, a: &Available, updater: &Updater) -> Outcome {
    let p = palette(ui.ctx());
    let mut outcome = Outcome::Keep;
    let title = |ui: &mut Ui, glyph: &str, text: &str| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(glyph).size(theme::ICON_TITLE).color(p.accent));
            ui.label(RichText::new(text).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
        });
        ui.add_space(12.0);
    };
    // Only an install of this update counts for this dialog.
    let state = match &updater.install {
        InstallState::Running { build, .. }
        | InstallState::Done { build, .. }
        | InstallState::Failed { build, .. }
            if *build != a.build =>
        {
            &InstallState::Idle
        }
        other => other,
    };
    let released = a
        .manifest
        .pub_date
        .as_deref()
        .and_then(|d| d.get(..10))
        .map(|d| format!(" Released {d}."))
        .unwrap_or_default();
    match state {
        InstallState::Running { phase, .. } => {
            title(
                ui,
                icon::ARROW_CIRCLE_UP,
                &format!("Updating to Canopy {}", a.label()),
            );
            let fraction = match phase {
                Phase::Downloading {
                    done,
                    total: Some(total),
                } if *total > 0 => (*done as f32 / *total as f32) * 0.8,
                Phase::Downloading { .. } => 0.0,
                Phase::Verifying => 0.8,
                Phase::Unpacking => 0.85,
                Phase::Installing => 0.92,
                Phase::Finishing => 0.98,
            };
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_height(8.0)
                    .fill(p.accent),
            );
            ui.add_space(6.0);
            let detail = match phase {
                Phase::Downloading { done, total } if *done > 0 => {
                    format!("{}  ·  {}", phase.text(), megabytes(*done, *total))
                }
                _ => phase.text(),
            };
            ui.label(
                RichText::new(detail)
                    .size(theme::META_SIZE)
                    .color(p.text_dim),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("Canopy keeps working while it updates, and stays as it is if anything goes wrong.")
                    .size(theme::META_SIZE)
                    .color(p.text_faint),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_sized([90.0, 34.0], egui::Button::new("Hide"))
                        .clicked()
                    {
                        outcome = Outcome::Close;
                    }
                });
            });
        }
        InstallState::Done { installed, .. } => {
            title(
                ui,
                icon::CHECK_CIRCLE,
                &format!("Canopy {} is installed", a.label()),
            );
            let text = if installed.exe.is_some() {
                "Restart Canopy to start using it. Your repositories and settings are untouched."
            } else {
                "Start it from your launcher or the command line. This window still runs the old build."
            };
            ui.label(RichText::new(text).color(p.text_dim));
            for note in &installed.notes {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{}  {note}", icon::WARNING))
                        .size(theme::META_SIZE)
                        .color(p.warning),
                );
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(exe) = &installed.exe
                        && widgets::primary_button(ui, "Restart now", true, 150.0).clicked()
                    {
                        outcome = Outcome::Run(vec![Action::RestartInto(exe.clone())]);
                    }
                    if ui
                        .add_sized([90.0, 34.0], egui::Button::new("Later"))
                        .clicked()
                    {
                        outcome = Outcome::Close;
                    }
                });
            });
        }
        InstallState::Idle | InstallState::Failed { .. } => {
            title(
                ui,
                icon::ARROW_CIRCLE_UP,
                &format!("Canopy {} is available", a.label()),
            );
            ui.label(
                RichText::new(format!(
                    "You have {}.{released}",
                    crate::build_info::label()
                ))
                .size(theme::META_SIZE)
                .color(p.text_dim),
            );
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(p.panel)
                .stroke(egui::Stroke::new(1.0, p.border))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            let notes = a.manifest.notes.trim();
                            if notes.is_empty() {
                                ui.label(RichText::new("No release notes.").color(p.text_faint));
                            } else {
                                crate::ui::notes::markdown(ui, notes);
                            }
                        });
                });
            ui.add_space(8.0);
            let how = match &updater.kind {
                Install::Managed { root } => format!(
                    "Canopy updates itself in {}. Settings and repositories are untouched, and the folder stays as it is if anything goes wrong.",
                    root.display()
                ),
                Install::Other => "This copy was not put here by Canopy's installer. Updating runs the installer, which installs a copy of its own and leaves this one as it is.".to_owned(),
            };
            ui.label(
                RichText::new(how)
                    .size(theme::META_SIZE)
                    .color(p.text_faint),
            );
            if let InstallState::Failed { error, .. } = state {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("{}  {error}", icon::WARNING))
                        .size(theme::META_SIZE)
                        .color(p.danger),
                );
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let retry = matches!(state, InstallState::Failed { .. });
                    if widgets::primary_button(
                        ui,
                        if retry { "Try again" } else { "Update" },
                        true,
                        150.0,
                    )
                    .clicked()
                    {
                        outcome =
                            Outcome::RunKeep(vec![Action::InstallUpdate(Box::new(a.clone()))]);
                    }
                    if ui
                        .add_sized([90.0, 34.0], egui::Button::new("Later"))
                        .clicked()
                    {
                        outcome = Outcome::Close;
                    }
                    if ui
                        .add_sized([130.0, 34.0], egui::Button::new("Skip this version"))
                        .clicked()
                    {
                        outcome = Outcome::Run(vec![Action::SkipVersion(a.build.to_string())]);
                    }
                });
            });
            ui.add_space(4.0);
            if ui
                .link("Download it yourself")
                .on_hover_text(a.download())
                .clicked()
            {
                outcome = Outcome::Run(vec![Action::OpenUrl(a.download())]);
            }
        }
    }
    outcome
}
