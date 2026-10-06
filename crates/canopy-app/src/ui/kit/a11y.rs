//! What screen readers need from every component beyond its own role and name: [`group`] (a
//! container the widgets in a `Ui` are announced as parts of) and [`disclosure_name`] (an open or
//! closed state Linux screen readers would not otherwise hear).

use egui::Ui;
use egui::accesskit::Role;

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

#[cfg(test)]
mod tests {
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
}
