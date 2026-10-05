//! Motion: durations, curves and springs, the Motion preference and the operating system's own
//! setting. See `docs/design.md`, "Motion".
//!
//! Motion in Canopy is feedback, never decoration. Anything driven by the keyboard or repeated
//! often (row hover, selection, focus, tab switches, palette filtering, scrolling to the selection)
//! changes instantly. Pointer feedback, popovers, dialogs and toasts ease, and with reduced motion
//! every duration here is zero and every spring lands on its target. The operating system's own
//! setting is read by [`crate::platform::prefers_reduced_motion`] on a background thread
//! ([`query_system`]).

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

/// Rows, selection, menu-item highlight, palette filtering, keyboard-opened menus.
#[expect(dead_code, reason = "used by the component phase")]
pub const INSTANT: f32 = 0.0;
/// Hover-in, press release, focus ring in, icon opacity, colour crossfades.
pub const FAST: f32 = 0.08;
/// Hover-out.
pub const FAST_OUT: f32 = 0.12;
/// Chevron rotation, label crossfades, tooltip in, field halo.
pub const BASE: f32 = 0.15;
/// Dialog and popover entrance transform.
pub const SLOW: f32 = 0.22;
/// Toast slide-in.
pub const TOAST_IN: f32 = 0.28;
/// Exits: a fade and a small reverse transform.
pub const EXIT_POPUP: f32 = 0.10;
pub const EXIT_DIALOG: f32 = 0.12;
pub const EXIT_TOAST: f32 = 0.16;
/// A tooltip's first delay, and the window after one closes in which the next shows at once.
pub const TOOLTIP_DELAY: f32 = 0.45;
pub const TOOLTIP_SKIP: f32 = 0.30;
/// Spinners and skeletons appear only after this long and then stay at least `LOADING_MIN`.
pub const LOADING_DELAY: f32 = 0.15;
pub const LOADING_MIN: f32 = 0.40;
/// The second of two toasts arriving together enters this much later.
pub const TOAST_STAGGER: f32 = 0.09;
/// Tooltip exit fade.
pub const TOOLTIP_OUT: f32 = 0.06;
/// A submenu opens after the pointer rests on its item this long.
pub const SUBMENU_DELAY: f32 = 0.10;
/// A determinate progress fill easing towards a larger value.
pub const PROGRESS_FILL: f32 = 0.18;
/// Loops (they run while visible and stop with reduced motion): one indeterminate sweep, one sweep
/// of the line under search inputs and its wait before the first, one spinner turn, one skeleton
/// pulse, and the reduced-motion spinner's pulse.
pub const INDETERMINATE_PERIOD: f32 = 1.35;
pub const LINE_PERIOD: f32 = 1.5;
pub const LINE_DELAY: f32 = 1.0;
pub const SPIN_PERIOD: f32 = 0.8;
pub const SKELETON_PERIOD: f32 = 1.2;
pub const PULSE_PERIOD: f32 = 1.0;

/// A CSS-style cubic Bézier timing curve from (0, 0) to (1, 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl Bezier {
    /// Progress at time `t` (0 to 1): Newton–Raphson on x, 8 iterations, then y.
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t <= 0.0 || t >= 1.0 {
            return t;
        }
        let curve = |a: f32, b: f32, s: f32| {
            let u = 1.0 - s;
            3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
        };
        let slope = |a: f32, b: f32, s: f32| {
            let u = 1.0 - s;
            3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
        };
        let mut s = t;
        for _ in 0..8 {
            let dx = slope(self.x1, self.x2, s);
            if dx.abs() < 1e-6 {
                break;
            }
            s = (s - (curve(self.x1, self.x2, s) - t) / dx).clamp(0.0, 1.0);
        }
        curve(self.y1, self.y2, s)
    }
}

/// Default for entrances and colour.
pub const EASE_OUT: Bezier = Bezier {
    x1: 0.2,
    y1: 0.8,
    x2: 0.2,
    y2: 1.0,
};
/// Indeterminate progress.
pub const EASE_IN_OUT: Bezier = Bezier {
    x1: 0.45,
    y1: 0.05,
    x2: 0.55,
    y2: 0.95,
};
/// About 3% overshoot: dialog and popover scale.
#[cfg_attr(not(test), expect(dead_code, reason = "used by the component phase"))]
pub const SPRING_SOFT: Bezier = Bezier {
    x1: 0.34,
    y1: 1.35,
    x2: 0.64,
    y2: 1.0,
};
/// About 10% overshoot (concept 1's `--spring`): the checkbox tick and radio dot, and the
/// entrance transform of menus, popovers, dialogs, tooltips and toasts.
pub const SPRING_CURVE: Bezier = Bezier {
    x1: 0.34,
    y1: 1.56,
    x2: 0.64,
    y2: 1.0,
};

/// A damped spring: natural frequency `omega` (rad/s) and damping ratio `zeta`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub omega: f32,
    pub zeta: f32,
}

/// The edge of a tab or segment indicator moving towards the target.
pub const INDICATOR_LEAD: Spring = Spring {
    omega: 22.0,
    zeta: 0.82,
};
/// The other edge (stretch, then settle).
pub const INDICATOR_TRAIL: Spring = Spring {
    omega: 16.0,
    zeta: 0.82,
};
/// The switch thumb: a slight overshoot.
pub const SWITCH: Spring = Spring {
    omega: 22.0,
    zeta: 0.62,
};
/// Press scale.
pub const PRESS: Spring = Spring {
    omega: 40.0,
    zeta: 1.0,
};

/// The user's choice in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MotionPref {
    /// Follow the operating system's reduce-motion setting.
    #[default]
    System,
    /// No movement: everything changes instantly.
    Reduced,
    /// Transitions, whatever the system says.
    Full,
}

impl MotionPref {
    pub const ALL: [Self; 3] = [Self::System, Self::Reduced, Self::Full];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Match system",
            Self::Reduced => "Reduced",
            Self::Full => "Full",
        }
    }

    /// The preference for a `--motion` argument.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "reduced" => Some(Self::Reduced),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// Whether motion is reduced, given what the system asked for (`None` when unknown).
    pub fn reduces(self, system: Option<bool>) -> bool {
        match self {
            Self::System => system.unwrap_or(false),
            Self::Reduced => true,
            Self::Full => false,
        }
    }
}

#[cfg(not(test))]
static REDUCED: AtomicBool = AtomicBool::new(false);

// Tests run in parallel threads, each with its own setting.
#[cfg(test)]
thread_local! {
    static REDUCED_LOCAL: AtomicBool = const { AtomicBool::new(false) };
}

#[cfg(not(test))]
fn reduced_flag<R>(f: impl FnOnce(&AtomicBool) -> R) -> R {
    f(&REDUCED)
}

#[cfg(test)]
fn reduced_flag<R>(f: impl FnOnce(&AtomicBool) -> R) -> R {
    REDUCED_LOCAL.with(f)
}

/// What the operating system said: 0 not known yet, 1 full motion, 2 reduced.
static SYSTEM: AtomicU8 = AtomicU8::new(0);
static QUERYING: AtomicBool = AtomicBool::new(false);

/// Whether motion is currently reduced.
pub fn reduced() -> bool {
    reduced_flag(|r| r.load(Ordering::Relaxed))
}

/// Records whether motion is reduced; the theme reads it when it installs the style.
pub fn set_reduced(on: bool) {
    reduced_flag(|r| r.store(on, Ordering::Relaxed));
}

/// The operating system's answer, once it has arrived (`None` while unknown or unanswered).
pub fn system() -> Option<bool> {
    match SYSTEM.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

/// Asks the operating system for its reduce-motion setting on a background thread (the query can
/// take seconds) and repaints when it answers, so the app re-applies the theme. One query at a
/// time; until it answers, motion is not reduced.
pub fn query_system(ctx: &egui::Context) {
    if QUERYING.swap(true, Ordering::Relaxed) {
        return;
    }
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("motion-query".into())
        .spawn(move || {
            let answer = crate::platform::prefers_reduced_motion();
            SYSTEM.store(
                match answer {
                    Some(true) => 2,
                    Some(false) => 1,
                    None => 0,
                },
                Ordering::Relaxed,
            );
            QUERYING.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    if let Err(e) = spawned {
        QUERYING.store(false, Ordering::Relaxed);
        tracing::warn!("could not ask the system about reduced motion: {e}");
    }
}

/// `seconds`, or zero when motion is reduced.
pub fn time(seconds: f32) -> f32 {
    if reduced() { 0.0 } else { seconds }
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

    #[test]
    fn bezier_matches_its_ends_and_shape() {
        for b in [EASE_OUT, EASE_IN_OUT, SPRING_SOFT, SPRING_CURVE] {
            assert_eq!(b.at(0.0), 0.0);
            assert_eq!(b.at(1.0), 1.0);
        }
        // Ease-out is past halfway early; ease-in-out is symmetric about the middle.
        assert!(EASE_OUT.at(0.3) > 0.6, "{}", EASE_OUT.at(0.3));
        assert!((EASE_IN_OUT.at(0.5) - 0.5).abs() < 0.01);
        // The springy curves overshoot 1 before settling.
        let peak = (1..100)
            .map(|i| SPRING_CURVE.at(i as f32 / 100.0))
            .fold(0.0_f32, f32::max);
        assert!(peak > 1.05 && peak < 1.15, "{peak}");
        let soft = (1..100)
            .map(|i| SPRING_SOFT.at(i as f32 / 100.0))
            .fold(0.0_f32, f32::max);
        assert!(soft > 1.01 && soft < peak, "{soft}");
    }

    #[test]
    fn motion_names_parse() {
        assert_eq!(MotionPref::from_name("Reduced"), Some(MotionPref::Reduced));
        assert_eq!(MotionPref::from_name("system"), Some(MotionPref::System));
        assert_eq!(MotionPref::from_name("nope"), None);
        assert_eq!(MotionPref::ALL.map(MotionPref::label)[0], "Match system");
    }
}
