//! Search of the history for commits that added or removed some text (the pickaxe), rendered from
//! the tab's worker-loaded `change_search`.

use egui::Ui;

use super::{
    Dialog, DialogResult, DialogWidth, InitialFocus, KitDialog, Outcome, confirm, field_with_error,
    note, working,
};
use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::kit::overlay::{Layer, ground};
use crate::ui::kit::row::{ListRow, RowKind};
use crate::ui::kit::toggle::Checkbox;
use crate::ui::theme::palette;
use crate::ui::widgets::absolute_date;
use crate::views::commit_detail::virtual_rows;
use canopy_core::{ChangeHit, ChangeSearch};
use egui_phosphor::regular as icon;

/// Space for visible result rows before the list scrolls.
const LIST_HEIGHT: f32 = 280.0;

pub(super) fn show(ctx: &egui::Context, dialog: &mut Dialog, tab: Option<&RepoTab>) -> Outcome {
    let salt = super::salt(dialog);
    let Dialog::SearchChanges {
        query,
        regex,
        searched,
        ..
    } = dialog
    else {
        return Outcome::Close;
    };
    let p = palette(ctx);
    let mut chosen = None;
    let mut submitted = false;
    let ok = !query.is_empty();
    let result = KitDialog::new(salt)
        .title("Search history for changed text")
        .icon(icon::MAGNIFYING_GLASS)
        .width(DialogWidth::Reading)
        .initial_focus(InitialFocus::Body)
        .body(|ui| {
            note(
                ui,
                "Finds the commits that added or removed this text, with the files it changed in. \
                 Your working tree is not changed.",
                p.text_dim,
            );
            let r = field_with_error(ui, "Text", query, "needle", true, None);
            submitted = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            Checkbox::new(
                regex,
                "Treat as a regular expression (changed lines that match)",
            )
            .show(ui);
            if *searched {
                match tab.and_then(|t| t.change_search.as_ref()) {
                    None => working(ui, "Searching the history…"),
                    Some(Err(error)) => note(ui, error, p.danger),
                    Some(Ok(found)) if found.hits.is_empty() => {
                        note(ui, "No commit changed this text.", p.text_dim);
                    }
                    Some(Ok(found)) => chosen = list(ui, found),
                }
            }
        })
        .footer(|f| {
            confirm(f, "Search", ok, "Enter the text to search for");
        })
        .show(ctx);
    if let Some(hit) = chosen {
        return Outcome::Run(vec![Action::ShowFileVersion(
            canopy_core::FileHistoryEntry {
                oid: hit.oid,
                summary: hit.summary,
                author: hit.author,
                time: hit.time,
                path: hit.path,
                old_path: None,
                kind: None,
            },
        )]);
    }
    let result = if submitted && ok {
        DialogResult::Confirm
    } else {
        result
    };
    match result {
        DialogResult::Cancel => Outcome::Close,
        DialogResult::Confirm if ok => {
            *searched = true;
            Outcome::RunKeep(vec![Action::SearchChanges {
                query: query.clone(),
                regex: *regex,
            }])
        }
        DialogResult::Open | DialogResult::Confirm => Outcome::Keep,
    }
}

/// The matches, newest commit first; the one clicked.
fn list(ui: &mut Ui, found: &ChangeSearch) -> Option<ChangeHit> {
    let p = palette(ui.ctx());
    note(
        ui,
        "Choose a result to see the file's diff in that commit.",
        p.text_dim,
    );
    if found.truncated {
        note(ui, "Showing the newest matching commits only.", p.warning);
    }
    let ground = ground(ui.ctx(), Layer::Dialog);
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt("search-changes")
        .max_height(LIST_HEIGHT)
        .show(ui, |ui| {
            virtual_rows(ui, found.hits.len(), RowKind::File.height(), |ui, i| {
                let hit = &found.hits[i];
                let detail = format!("{} · {}", hit.author, absolute_date(hit.time));
                let row = ListRow::new(("search-changes", hit.oid, &hit.path))
                    .kind(RowKind::File)
                    .icon(icon::GIT_COMMIT)
                    .label(&hit.summary)
                    .detail(format!("{} · {detail}", hit.path))
                    .whole_detail()
                    .trailing(canopy_core::short_id(hit.oid))
                    .tooltip(hit.path.clone())
                    .ground(ground)
                    .show(ui);
                if row.clicked() {
                    chosen = Some(hit.clone());
                }
            });
        });
    chosen
}
