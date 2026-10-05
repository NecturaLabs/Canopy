//! Screens and panels. Views read tab state and push [`crate::action::Action`]s; they never call git.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "geometry, alpha and animation time: egui works in f32 points, u8 channels and i8 margins, and the values are bounded by the layout"
)]

pub mod changes;
pub mod commit_detail;
pub mod dialogs;
pub mod diff;
pub mod history;
pub mod requests;
pub mod resolve;
pub mod settings_screen;
pub mod sidebar;
pub mod statusbar;
pub mod toolbar;
pub mod welcome;
