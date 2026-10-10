//! A file's history through renames, rendered from the tab's worker-loaded `file_history`.

use egui::Ui;

use super::{Dialog, DialogWidth, InitialFocus, KitDialog, Outcome, answer, note, working};
use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::kit::overlay::{Layer, ground};
use crate::ui::kit::row::{ListRow, RowKind};
use crate::ui::theme::palette;
use crate::ui::widgets::absolute_date;
use crate::views::commit_detail::virtual_rows;
use canopy_core::{ChangeKind, FileHistory};
use egui_phosphor::regular as icon;

/// Space for visible commit rows before the list scrolls.
const LIST_HEIGHT: f32 = 320.0;

pub(super) fn show(ctx: &egui::Context, dialog: &Dialog, tab: Option<&RepoTab>) -> Outcome {
    let Dialog::FileHistory { path, .. } = dialog else {
        return Outcome::Close;
    };
    let p = palette(ctx);
    let mut chosen = None;
    let result = KitDialog::new(super::salt(dialog))
        .title("File history")
        .description(path)
        .width(DialogWidth::Reading)
        .initial_focus(InitialFocus::Cancel)
        .body(|ui| match tab.and_then(|t| t.file_history.as_ref()) {
            None => working(ui, "Reading the file's history…"),
            Some(Err(error)) => note(ui, error, p.danger),
            Some(Ok(history)) if history.entries.is_empty() => {
                note(ui, "No commit has changed this file.", p.text_dim);
            }
            Some(Ok(history)) => chosen = list(ui, history),
        })
        .footer(|f| {
            f.cancel().cancel_label("Close");
        })
        .show(ctx);
    if let Some(entry) = chosen {
        return Outcome::Run(vec![Action::ShowFileVersion(entry)]);
    }
    answer(result, || Outcome::Close)
}

/// The commits, newest first; the one clicked.
fn list(ui: &mut Ui, history: &FileHistory) -> Option<canopy_core::FileHistoryEntry> {
    let p = palette(ui.ctx());
    note(
        ui,
        "Follows the file through renames. Choose a commit to see the file's diff there; \
         your working tree is not changed.",
        p.text_dim,
    );
    if history.truncated {
        note(
            ui,
            format!("Showing the newest {} commits.", history.entries.len()),
            p.warning,
        );
    }
    let ground = ground(ui.ctx(), Layer::Dialog);
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt("file-history")
        .max_height(LIST_HEIGHT)
        .show(ui, |ui| {
            virtual_rows(
                ui,
                history.entries.len(),
                RowKind::File.height(),
                |ui, i| {
                    let entry = &history.entries[i];
                    let detail = format!("{} · {}", entry.author, absolute_date(entry.time));
                    let tip = match &entry.old_path {
                        Some(old) => format!("{} (renamed from {old})", entry.path),
                        None => entry.path.clone(),
                    };
                    let row = ListRow::new(("file-history", entry.oid))
                        .kind(RowKind::File)
                        .icon(glyph(entry.kind))
                        .label(&entry.summary)
                        .detail(detail)
                        .trailing(canopy_core::short_id(entry.oid))
                        .tooltip(tip)
                        .ground(ground)
                        .show(ui);
                    if row.clicked() {
                        chosen = Some(entry.clone());
                    }
                },
            );
        });
    chosen
}

/// The row's glyph: what the commit did to the file.
fn glyph(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => icon::PLUS,
        ChangeKind::Deleted => icon::TRASH,
        ChangeKind::Renamed => icon::ARROWS_LEFT_RIGHT,
        _ => icon::PENCIL_SIMPLE,
    }
}
