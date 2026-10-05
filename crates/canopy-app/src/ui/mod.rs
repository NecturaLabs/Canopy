//! Look and feel: design tokens, fonts and shared widgets.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "geometry, alpha and animation time: egui works in f32 points, u8 channels and i8 margins, and the values are bounded by the layout"
)]

pub mod art;
pub mod cheatsheet;
pub mod folder_picker;
pub mod fonts;
pub mod kit;
pub mod legal;
pub mod motion;
pub mod notes;
pub mod palette;
pub mod theme;
pub mod toast;
pub mod widgets;
