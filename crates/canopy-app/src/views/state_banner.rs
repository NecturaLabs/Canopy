//! The merge, rebase, cherry-pick or revert in progress, shown the same way above History,
//! Changes and the request list: a kit warning [`Banner`] with Continue and Abort at its right.

use egui::{Align, Layout, Rect, Ui, UiBuilder, pos2, vec2};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::kit::surface::Banner;
use crate::ui::kit::{Button, Size, Tone};
use crate::ui::theme;
use crate::worker::Op;

/// Why Continue or Abort waits.
const WAIT: &str = "Wait for the current operation";

/// Draws the banner while the repository is in the middle of an operation; nothing when it is
/// clean.
pub fn show(ui: &mut Ui, tab: &RepoTab, actions: &mut Vec<Action>) {
    let Some(snap) = &tab.snap else {
        return;
    };
    if snap.state == canopy_core::RepoState::Clean {
        return;
    }
    let state = snap.state;
    let conflicts = snap.status.conflicted.len();
    let busy = tab.running.is_some();
    egui::Panel::top("repository-operation-state")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let banner = Banner::new(
                Tone::Warning,
                format!("{} in progress on {}", state.label(), snap.head.label()),
            )
            .body(conflicts_left(conflicts))
            .id_salt("repository-operation-state")
            .show(ui);
            let Some(command) = state.command() else {
                return;
            };
            // The banner's own actions cannot be disabled or explain themselves, so Continue and
            // Abort sit over its right end, where its actions would be.
            let rect = banner.response.rect;
            if rect.height() <= 0.0 {
                return;
            }
            // The banner grows in from its top edge: its content sits on the bottom.
            let full = theme::metrics().pane_header_height;
            let row = Rect::from_min_size(
                pos2(rect.left(), rect.bottom() - full),
                vec2(rect.width(), full),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(row.shrink2(vec2(theme::SM, 0.0)))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            child.spacing_mut().item_spacing.x = theme::SM;
            if Button::new(format!("Abort {command}…"))
                .ghost()
                .size(Size::Sm)
                .icon(icon::X_CIRCLE)
                .icon_tone(Tone::Danger)
                .enabled(!busy)
                .disabled_reason(WAIT)
                .id_salt("state-abort")
                .show(&mut child)
                .clicked()
            {
                actions.push(abort(state, command));
            }
            let can_continue = conflicts == 0 && !busy;
            let why = if busy {
                WAIT
            } else {
                "Resolve and stage every conflicted file first"
            };
            if Button::new("Continue")
                .ghost()
                .size(Size::Sm)
                .icon(icon::ARROW_RIGHT)
                .enabled(can_continue)
                .tooltip("Continue this operation using the staged resolutions")
                .disabled_reason(why)
                .id_salt("state-continue")
                .show(&mut child)
                .clicked()
            {
                actions.push(Action::Op(Op::ContinueOrAbort {
                    state,
                    abort: false,
                }));
            }
        });
}

/// The banner's body: how many conflicted files are left.
fn conflicts_left(conflicts: usize) -> String {
    match conflicts {
        0 => "No conflicts left".to_owned(),
        1 => "1 conflict left".to_owned(),
        n => format!("{n} conflicts left"),
    }
}

/// The confirmation before the operation in progress, git's `command`, is aborted.
fn abort(state: canopy_core::RepoState, command: &str) -> Action {
    Action::Confirm {
        title: format!("Abort the {command}?"),
        body: "The repository returns to where it was before the operation started. Resolved \
               conflicts are discarded."
            .into(),
        confirm: format!("Abort {command}"),
        danger: true,
        op: Op::ContinueOrAbort { state, abort: true },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mop-up D24: the title asks a question naming what goes, the answer is verb + object.
    #[test]
    fn aborting_asks_about_the_operation_by_name() {
        let Action::Confirm { title, confirm, .. } =
            abort(canopy_core::RepoState::Rebase, "rebase")
        else {
            panic!("not a confirmation");
        };
        assert_eq!(title, "Abort the rebase?");
        assert_eq!(confirm, "Abort rebase");
    }

    #[test]
    fn the_body_counts_the_conflicts_left() {
        assert_eq!(conflicts_left(0), "No conflicts left");
        assert_eq!(conflicts_left(1), "1 conflict left");
        assert_eq!(conflicts_left(3), "3 conflicts left");
    }
}
