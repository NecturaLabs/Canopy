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
            let abort_button = Button::new(format!("Abort {command}…"))
                .ghost()
                .size(Size::Sm)
                .icon(icon::X_CIRCLE)
                .icon_tone(Tone::Danger)
                .enabled(!busy)
                .disabled_reason(WAIT)
                .id_salt("state-abort")
                .show(&mut child);
            if crate::views::commit_detail::say_unavailable(abort_button, WAIT).clicked() {
                actions.push(abort(state, command));
            }
            let can_continue = conflicts == 0 && !busy;
            let why = if busy {
                WAIT
            } else {
                "Resolve and stage every conflicted file first"
            };
            let continue_button = Button::new("Continue")
                .ghost()
                .size(Size::Sm)
                .icon(icon::ARROW_RIGHT)
                .enabled(can_continue)
                .tooltip("Continue this operation using the staged resolutions")
                .disabled_reason(why)
                .id_salt("state-continue")
                .show(&mut child);
            if crate::views::commit_detail::say_unavailable(continue_button, why).clicked() {
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

    /// Whether `tip` is painted after the pointer rests on the banner's Continue button.
    fn continue_tip_shown(level: crate::ui::kit::overlay::TooltipLevel, tip: &str) -> bool {
        let mut snap = canopy_core::RepoSnapshot {
            state: canopy_core::RepoState::Merge,
            ..Default::default()
        };
        snap.status.conflicted.push(canopy_core::FileEntry {
            path: "a.txt".into(),
            old_path: None,
            kind: canopy_core::ChangeKind::Modified,
            additions: 0,
            deletions: 0,
            lfs: false,
            old_mode: None,
            new_mode: None,
        });
        let tab = crate::app::test_tab(snap);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        crate::ui::kit::overlay::set_tooltip_level(level);
        let mut at = egui::Pos2::ZERO;
        let mut found = false;
        for i in 0..16 {
            let events = if i == 1 {
                vec![egui::Event::PointerMoved(at)]
            } else {
                Vec::new()
            };
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 400.0))),
                    time: Some(f64::from(i) * 0.2),
                    events,
                    ..Default::default()
                },
                |ui| show(ui, &tab, &mut Vec::new()),
            );
            out.textures_delta.clear();
            if let Some(update) = out.platform_output.accesskit_update {
                for (_, node) in update.nodes {
                    if node.label().is_some_and(|l| l.starts_with("Continue"))
                        && let Some(b) = node.bounds()
                    {
                        at = pos2(
                            crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
                            crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
                        );
                    }
                }
            }
            found = out
                .shapes
                .iter()
                .any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == tip));
        }
        crate::ui::kit::overlay::set_tooltip_level(crate::ui::kit::overlay::TooltipLevel::Helpful);
        found
    }

    /// F18: a disabled Continue says why on hover at every tooltip level.
    #[test]
    fn a_disabled_continue_says_why_at_every_tooltip_level() {
        use crate::ui::kit::overlay::TooltipLevel;
        let shown: Vec<bool> = [TooltipLevel::Off, TooltipLevel::Helpful, TooltipLevel::All]
            .into_iter()
            .map(|l| continue_tip_shown(l, "Resolve and stage every conflicted file first"))
            .collect();
        assert_eq!(shown, [true, true, true], "Off, Helpful, All");
    }

    #[test]
    fn the_body_counts_the_conflicts_left() {
        assert_eq!(conflicts_left(0), "No conflicts left");
        assert_eq!(conflicts_left(1), "1 conflict left");
        assert_eq!(conflicts_left(3), "3 conflicts left");
    }
}
