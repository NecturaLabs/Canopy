//! Screens and panels. Views read tab state and push [`crate::action::Action`]s; they never call git.

pub mod changes;
pub mod commit_detail;
pub mod dialogs;
pub mod diff;
pub mod history;
pub mod requests;
pub mod resolve;
pub mod settings_screen;
pub mod sidebar;
pub mod state_banner;
pub mod statusbar;
pub mod toolbar;
pub mod welcome;
