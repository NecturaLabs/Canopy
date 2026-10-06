//! What screen readers and the keyboard need from every component beyond its own role and name:
//! [`group`] (a container the widgets in a `Ui` are announced as parts of), [`disclosure_name`]
//! (an open or closed state Linux screen readers would not otherwise hear), [`FocusReturn`] (an
//! overlay gives focus back to the control that had it) and [`trap_focus`] (Tab stays inside an
//! overlay while it is open).

use egui::accesskit::Role;
use egui::{Context, Id, LayerId, Ui};

/// Marks `ui`'s own accessibility node as a `role` named `label`, so the widgets inside it are
/// announced as its parts (the commits of a list, the controls of a dialog). Every egui `Ui` has a
/// node of its own, a generic container screen readers pass over; this gives it a meaning. Call
/// it on a `Ui` that holds only what belongs to the group (a `scope` around the rows).
pub fn group(ui: &Ui, role: Role, label: &str) {
    ui.ctx().accesskit_node_builder(ui.unique_id(), |n| {
        n.set_role(role);
        n.set_label(label);
    });
}

/// Marks `ui` (the card of an overlay) as a modal dialog named `label`: its controls are its
/// parts, and screen readers keep to it while it is open (an `AlertDialog` when it asks before
/// destroying something).
pub fn dialog(ui: &Ui, label: &str, alert: bool) {
    group(
        ui,
        if alert {
            Role::AlertDialog
        } else {
            Role::Dialog
        },
        label,
    );
    ui.ctx()
        .accesskit_node_builder(ui.unique_id(), egui::accesskit::Node::set_modal);
}

/// Whether the platform's accessibility adapter passes a node's expanded state on. AccessKit's
/// Linux adapter (`accesskit_atspi_common` 0.18) has no expanded state, so Orca cannot say
/// whether a disclosure is open; there the state goes into the name instead.
const ADAPTER_SPEAKS_EXPANDED: bool = !cfg!(all(unix, not(target_os = "macos")));

/// The name a disclosure (a tree row with children, a section header, a collapsing block) is
/// announced by: `label`, and on Linux its state after it ("Remotes, expanded"), because the
/// adapter there drops the expanded state the node also carries.
pub fn disclosure_name(label: &str, open: bool) -> String {
    if ADAPTER_SPEAKS_EXPANDED {
        label.to_owned()
    } else if open {
        format!("{label}, expanded")
    } else {
        format!("{label}, collapsed")
    }
}

/// Where keyboard focus goes back to when an overlay (the palette, the cheatsheet) closes: the
/// control that had it when the overlay opened. Keep one in the overlay's state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FocusReturn {
    to: Option<Id>,
    armed: bool,
}

impl FocusReturn {
    /// Remembers what has focus now; call it on the overlay's first frame, before it takes focus.
    pub fn remember(&mut self, ctx: &Context) {
        self.to = ctx.memory(egui::Memory::focused);
        self.armed = true;
    }

    /// Gives focus back to what had it when [`Self::remember`] ran, once, however the overlay on
    /// `layer` closed (Esc, a click outside, a chosen entry). Call it every frame the overlay is
    /// closed. Focus that has moved on elsewhere (into the dialog a chosen command opened) stays.
    pub fn restore(&mut self, ctx: &Context, layer: LayerId) {
        if !std::mem::take(&mut self.armed) {
            return;
        }
        let moved_on = ctx
            .memory(egui::Memory::focused)
            .and_then(|f| ctx.read_response(f))
            .is_some_and(|r| r.layer_id != layer);
        if let Some(id) = self.to.take()
            && !moved_on
        {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }
}

/// Keeps keyboard focus and the pointer inside `layer` and the layers above it for this frame:
/// Tab cycles through the overlay's own controls, and nothing behind it takes a key or a click.
/// Call it every frame the overlay is open, with the overlay's lowest layer (its scrim).
pub fn trap_focus(ctx: &Context, layer: LayerId) {
    ctx.memory_mut(|m| m.set_modal_layer(layer));
}

#[cfg(test)]
mod tests {
    use egui::{Key, Order};

    use super::super::{Button, testkit};
    use super::*;

    #[test]
    fn a_group_announces_the_widgets_inside_it_as_its_parts() {
        let ctx = testkit::ctx();
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.scope(|ui| {
                group(ui, Role::List, "Branches");
                Button::new("main").show(ui);
            });
        });
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.expect("accesskit");
        let label = |id| {
            update
                .nodes
                .iter()
                .find(|(n, _)| *n == id)
                .and_then(|(_, n)| n.label().map(str::to_owned))
        };
        let (_, list) = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::List)
            .expect("the list");
        assert_eq!(list.label(), Some("Branches"));
        let children: Vec<_> = list.children().iter().filter_map(|c| label(*c)).collect();
        assert_eq!(children, ["main"], "the button is the list's part");
    }

    #[test]
    fn a_disclosure_says_its_state_where_the_adapter_cannot() {
        let open = disclosure_name("Remotes", true);
        let shut = disclosure_name("Remotes", false);
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(open, "Remotes, expanded");
            assert_eq!(shut, "Remotes, collapsed");
        } else {
            assert_eq!((open.as_str(), shut.as_str()), ("Remotes", "Remotes"));
        }
    }

    #[test]
    fn focus_returns_once_to_what_had_it() {
        let ctx = testkit::ctx();
        let id = Id::new("trigger");
        ctx.memory_mut(|m| m.request_focus(id));
        let mut back = FocusReturn::default();
        back.remember(&ctx);
        ctx.memory_mut(|m| m.request_focus(Id::new("overlay field")));
        back.restore(&ctx, LayerId::background());
        assert!(ctx.memory(|m| m.has_focus(id)));
        ctx.memory_mut(|m| m.request_focus(Id::new("elsewhere")));
        back.restore(&ctx, LayerId::background());
        assert!(!ctx.memory(|m| m.has_focus(id)), "restored only once");
    }

    /// Tab moves only between the overlay's controls while it traps focus.
    #[test]
    fn a_trapped_overlay_keeps_tab_inside() {
        let ctx = testkit::ctx();
        let scrim = LayerId::new(Order::Foreground, Id::new("test-scrim"));
        let mut show = |ui: &mut Ui| {
            Button::new("Behind").show(ui);
            trap_focus(ui.ctx(), scrim);
            egui::Area::new(Id::new("test-overlay"))
                .order(Order::Tooltip)
                .show(ui.ctx(), |ui| {
                    Button::new("First").show(ui);
                    Button::new("Second").show(ui);
                });
        };
        // The trap holds from the frame after the overlay first asks for it.
        testkit::frame(&ctx, Vec::new(), &mut show);
        let mut seen = Vec::new();
        for _ in 0..6 {
            testkit::frame(&ctx, testkit::key(Key::Tab), &mut show);
            testkit::frame(&ctx, Vec::new(), &mut show);
            let focused = ctx.memory(egui::Memory::focused);
            let layer = focused
                .and_then(|f| ctx.read_response(f))
                .map(|r| r.layer_id);
            seen.push(layer);
        }
        assert!(
            seen.iter()
                .all(|l| l.is_some_and(|l| l.order == Order::Tooltip)),
            "Tab left the overlay: {seen:?}"
        );
    }
}
