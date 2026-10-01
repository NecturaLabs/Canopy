//! Motion: the three animation durations, the reduce-motion preference and the operating
//! system's own setting. See `docs/design.md`, "Motion".
//!
//! Motion in Canopy is feedback, never decoration. Anything driven by the keyboard or repeated
//! often (row hover, selection, focus, tab switches, palette filtering, scrolling to the selection)
//! changes instantly. Pointer feedback, popovers, dialogs and toasts ease out over 100 to 200 ms,
//! and with reduce motion every duration here is zero. The operating system's own setting is
//! read by [`crate::platform::prefers_reduced_motion`].

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

/// Hover and press feedback, switch thumbs, checkboxes.
#[expect(dead_code, reason = "adopted by the motion pass")]
pub const FAST: f32 = 0.10;
/// Popovers, menus and tooltips appearing; expand and collapse; toasts leaving.
pub const BASE: f32 = 0.15;
/// Dialogs and toasts arriving.
#[expect(dead_code, reason = "adopted by the motion pass")]
pub const SLOW: f32 = 0.20;

/// The user's choice in Settings.
#[cfg_attr(not(test), expect(dead_code, reason = "adopted by the motion pass"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MotionPref {
    /// Follow the operating system's reduce-motion setting.
    #[default]
    System,
    /// No movement: everything changes instantly.
    Reduced,
    /// Subtle transitions, whatever the system says.
    Full,
}

impl MotionPref {
    #[expect(dead_code, reason = "adopted by the motion pass")]
    pub const ALL: [MotionPref; 3] = [MotionPref::System, MotionPref::Reduced, MotionPref::Full];

    #[expect(dead_code, reason = "adopted by the motion pass")]
    pub fn label(self) -> &'static str {
        match self {
            MotionPref::System => "Match system",
            MotionPref::Reduced => "Reduced",
            MotionPref::Full => "Full",
        }
    }

    /// Whether motion is reduced, given what the system asked for (`None` when unknown).
    #[cfg_attr(not(test), expect(dead_code, reason = "adopted by the motion pass"))]
    pub fn reduces(self, system: Option<bool>) -> bool {
        match self {
            MotionPref::System => system.unwrap_or(false),
            MotionPref::Reduced => true,
            MotionPref::Full => false,
        }
    }
}

static REDUCED: AtomicBool = AtomicBool::new(false);

/// Whether motion is currently reduced.
pub fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Records whether motion is reduced; the theme reads it when it installs the style.
pub fn set_reduced(on: bool) {
    REDUCED.store(on, Ordering::Relaxed);
}

/// `seconds`, or zero when motion is reduced.
pub fn time(seconds: f32) -> f32 {
    if reduced() { 0.0 } else { seconds }
}

/// Eases towards `value` over `seconds` (zero with reduce motion) and returns how far along the
/// transition is, from 0 to 1. Repaints only while the transition runs.
#[expect(dead_code, reason = "adopted by the motion pass")]
pub fn ease(ctx: &egui::Context, id: egui::Id, value: bool, seconds: f32) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, value, time(seconds), egui::emath::easing::cubic_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_overrides_the_system() {
        assert!(MotionPref::System.reduces(Some(true)));
        assert!(!MotionPref::System.reduces(None));
        assert!(MotionPref::Reduced.reduces(Some(false)));
        assert!(!MotionPref::Full.reduces(Some(true)));
    }
}
