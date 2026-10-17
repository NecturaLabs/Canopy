//! Look and feel: the design tokens, fonts and component kit come from `nectura-ui`; this module
//! adds the application's own widgets and screens.

pub mod cheatsheet;
pub mod folder_picker;
pub mod kit;
pub mod legal;
pub mod notes;
pub mod palette;
pub mod toast;
pub mod widgets;

pub use nectura_ui::{
    art, count_f32, custom_theme, fonts, int_f32, motion, points_i8, saturate_u8, saturate_usize,
    theme, to_f32,
};

/// What `nectura-ui` asks of the application: the system's accessibility settings and the key
/// that opens a context menu (the keymap's `ContextMenu` command).
#[derive(Debug)]
struct UiHost;

impl nectura_ui::host::Host for UiHost {
    fn prefers_reduced_motion(&self) -> Option<bool> {
        crate::platform::prefers_reduced_motion()
    }

    fn prefers_reduced_transparency(&self) -> Option<bool> {
        crate::platform::prefers_reduced_transparency()
    }

    fn consume_context_menu_key(&self, input: &mut egui::InputState) -> bool {
        crate::keymap::consume(input, crate::keymap::Command::ContextMenu)
    }
}

/// Installs the application's side of `nectura-ui`; call once at start-up.
pub fn install_host() {
    nectura_ui::host::install(&UiHost);
}
