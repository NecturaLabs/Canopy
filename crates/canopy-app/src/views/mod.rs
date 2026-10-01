//! Screens and panels. Views read tab state and push [`crate::app::Action`]s; they never call git.

pub mod changes;
pub mod commit_detail;
pub mod dialogs;
pub mod diff;
pub mod history;
pub mod sidebar;
pub mod statusbar;
pub mod toolbar;
pub mod welcome;
