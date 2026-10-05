//! The update dialog: what is new, installing it, and restarting into it.

use super::*;
use crate::selfupdate::{Install, Phase};
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Progress;
use crate::updater::{Available, InstallState, Updater};

/// "12.3 MB", or "12.3 of 45.6 MB" while the size is known.
fn megabytes(done: u64, total: Option<u64>) -> String {
    #[expect(
        clippy::cast_precision_loss,
        reason = "shown to a tenth of a megabyte; f64 is exact far beyond any download"
    )]
    let mb = |n: u64| n as f64 / (1024.0 * 1024.0);
    match total {
        Some(total) => format!("{:.1} of {:.1} MB", mb(done), mb(total)),
        None => format!("{:.1} MB", mb(done)),
    }
}

/// The update dialog for `a`. `updater` says how this copy was installed and where an install
/// started from here stands.
pub(super) fn update(
    ctx: &egui::Context,
    salt: impl std::hash::Hash + std::fmt::Debug,
    a: &Available,
    updater: &Updater,
) -> Outcome {
    let p = palette(ctx);
    let m = theme::metrics();
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
    let dialog = KitDialog::new(salt).width(DialogWidth::Form);
    let mut extra: Option<Outcome> = None;
    match state {
        InstallState::Running { phase, .. } => {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a share of the progress bar; f32 rounding is invisible there"
            )]
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
            let detail = match phase {
                Phase::Downloading { done, total } if *done > 0 => megabytes(*done, *total),
                _ => String::new(),
            };
            let result = dialog
                .title(format!("Updating to Canopy {}", a.label()))
                .icon(icon::ARROW_CIRCLE_UP)
                .body(|ui| {
                    Progress::new(fraction)
                        .label(phase.text())
                        .value_text(detail)
                        .id_salt("update-progress")
                        .show(ui);
                    note(
                        ui,
                        "Canopy keeps working while it updates, and stays as it is if anything goes wrong.",
                        p.text_faint,
                    );
                })
                .footer(|f| {
                    f.cancel_label("Hide");
                })
                .show(ctx);
            answer(result, || Outcome::Keep)
        }
        InstallState::Done { installed, .. } => {
            let text = if installed.exe.is_some() {
                "Restart Canopy to start using it. Your repositories and settings are untouched."
            } else {
                "Start it from your launcher or the command line. This window still runs the old build."
            };
            let result = dialog
                .title(format!("Canopy {} is installed", a.label()))
                .icon(icon::CHECK_CIRCLE)
                .description(text)
                .body(|ui| {
                    for note_text in &installed.notes {
                        note(ui, format!("{}  {note_text}", icon::WARNING), p.warning);
                    }
                })
                .footer(|f| {
                    f.cancel_label("Later");
                    if installed.exe.is_some() {
                        f.confirm("Restart now");
                    }
                })
                .show(ctx);
            answer(result, || match &installed.exe {
                Some(exe) => Outcome::Run(vec![Action::RestartInto(exe.clone())]),
                None => Outcome::Keep,
            })
        }
        InstallState::Idle | InstallState::Failed { .. } => {
            let retry = matches!(state, InstallState::Failed { .. });
            let how = match &updater.kind {
                Install::Managed { root } => format!(
                    "Canopy updates itself in {}. Settings and repositories are untouched, and the folder stays as it is if anything goes wrong.",
                    root.display()
                ),
                Install::Other => "This copy was not put here by Canopy's installer. Updating runs the installer, which installs a copy of its own and leaves this one as it is.".to_owned(),
            };
            let result = dialog
                .title(format!("Canopy {} is available", a.label()))
                .icon(icon::ARROW_CIRCLE_UP)
                .description(format!(
                    "You have {}.{released}",
                    crate::build_info::label()
                ))
                .body(|ui| {
                    InsetBox::new().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("update-notes")
                            .max_height(220.0)
                            .show(ui, |ui| {
                                let notes = a.manifest.notes.trim();
                                if notes.is_empty() {
                                    ui.label(
                                        RichText::new("No release notes.").color(p.text_faint),
                                    );
                                } else {
                                    crate::ui::notes::markdown(ui, notes);
                                }
                            });
                    });
                    note(ui, how, p.text_faint);
                    if let InstallState::Failed { error, .. } = state {
                        note(ui, format!("{}  {error}", icon::WARNING), p.danger);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = theme::MD;
                        if Button::new("Skip this version")
                            .ghost()
                            .size(Size::Sm)
                            .show(ui)
                            .clicked()
                        {
                            extra =
                                Some(Outcome::Run(vec![Action::SkipVersion(a.build.to_string())]));
                        }
                        if LinkButton::new("Download it yourself")
                            .text_size(m.meta_size)
                            .show(ui)
                            .kit_tooltip(a.download())
                            .show()
                            .clicked()
                        {
                            extra = Some(Outcome::Run(vec![Action::OpenUrl(a.download())]));
                        }
                    });
                })
                .footer(|f| {
                    f.cancel_label("Later")
                        .confirm(if retry { "Try again" } else { "Update" });
                })
                .show(ctx);
            let outcome = answer(result, || {
                Outcome::RunKeep(vec![Action::InstallUpdate(Box::new(a.clone()))])
            });
            match (extra, outcome) {
                (Some(e), Outcome::Keep) => e,
                (_, o) => o,
            }
        }
    }
}
