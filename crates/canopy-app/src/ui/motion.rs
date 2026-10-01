//! Motion: the three animation durations, the reduce-motion preference and the operating
//! system's own setting. See `docs/design.md`, "Motion".
//!
//! Motion in Canopy is feedback, never decoration. Anything driven by the keyboard or repeated
//! often (row hover, selection, focus, tab switches, palette filtering, scrolling to the selection)
//! changes instantly. Pointer feedback, popovers, dialogs and toasts ease out over 100 to 200 ms,
//! and with reduce motion every duration here is zero.

// Removed once the reduce-motion setting and the views adopt the module (plan task: Motion).
#![allow(dead_code)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Hover and press feedback, switch thumbs, checkboxes.
pub const FAST: f32 = 0.10;
/// Popovers, menus and tooltips appearing; expand and collapse; toasts leaving.
pub const BASE: f32 = 0.15;
/// Dialogs and toasts arriving.
pub const SLOW: f32 = 0.20;

/// The user's choice in Settings.
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
    pub const ALL: [MotionPref; 3] = [MotionPref::System, MotionPref::Reduced, MotionPref::Full];

    pub fn label(self) -> &'static str {
        match self {
            MotionPref::System => "Match system",
            MotionPref::Reduced => "Reduced",
            MotionPref::Full => "Full",
        }
    }

    /// Whether motion is reduced, given what the system asked for (`None` when unknown).
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
pub fn ease(ctx: &egui::Context, id: egui::Id, value: bool, seconds: f32) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, value, time(seconds), egui::emath::easing::cubic_out)
}

/// How long a system query may take before Canopy gives up on it.
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Asks the operating system whether the user wants reduced motion. Blocks for up to a few
/// seconds while a helper program answers, so call it off the UI thread. `None` when the system
/// does not say.
pub fn system_prefers_reduced() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        // The desktop portal works across GNOME, KDE and others; older GNOME only has gsettings.
        let portal = run(
            "gdbus",
            &[
                "call",
                "--session",
                "--timeout",
                "2",
                "--dest",
                "org.freedesktop.portal.Desktop",
                "--object-path",
                "/org/freedesktop/portal/desktop",
                "--method",
                "org.freedesktop.portal.Settings.ReadOne",
                "org.freedesktop.appearance",
                "reduced-motion",
            ],
        )
        .and_then(|out| parse_portal(&out));
        portal.or_else(|| {
            run(
                "gsettings",
                &["get", "org.gnome.desktop.interface", "enable-animations"],
            )
            .and_then(|out| parse_gsettings(&out))
        })
    }
    #[cfg(target_os = "macos")]
    {
        run(
            "defaults",
            &["read", "com.apple.universalaccess", "reduceMotion"],
        )
        .and_then(|out| parse_defaults(&out))
    }
    #[cfg(target_os = "windows")]
    {
        // "Show animations in Windows" (SPI_GETCLIENTAREAANIMATION), read through .NET so Canopy
        // needs no unsafe system call.
        run(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Add-Type -AssemblyName PresentationFramework; \
                 [System.Windows.SystemParameters]::ClientAreaAnimation",
            ],
        )
        .and_then(|out| parse_windows(&out))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Runs `program` and returns its standard output when it succeeds within [`QUERY_TIMEOUT`].
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() > QUERY_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::debug!(program, "reduce-motion query timed out");
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

/// `(<uint32 1>,)` from the portal: 1 asks for reduced motion.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_portal(out: &str) -> Option<bool> {
    let digits: String = out
        .trim()
        .trim_start_matches("(<")
        .trim_end_matches(">,)")
        .trim_start_matches("uint32")
        .trim()
        .to_owned();
    match digits.as_str() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// `true` or `false` for `enable-animations`; disabled animations mean reduced motion.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_gsettings(out: &str) -> Option<bool> {
    match out.trim() {
        "true" => Some(false),
        "false" => Some(true),
        _ => None,
    }
}

/// `1` or `0` from `defaults read`.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn parse_defaults(out: &str) -> Option<bool> {
    match out.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// `True` or `False` for client-area animation; animations off mean reduced motion.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn parse_windows(out: &str) -> Option<bool> {
    match out.trim().to_ascii_lowercase().as_str() {
        "true" => Some(false),
        "false" => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_answers_are_read() {
        assert_eq!(parse_portal("(<uint32 1>,)\n"), Some(true));
        assert_eq!(parse_portal("(<uint32 0>,)"), Some(false));
        assert_eq!(parse_portal("Error: no such key"), None);
        assert_eq!(parse_gsettings("false\n"), Some(true));
        assert_eq!(parse_gsettings("true"), Some(false));
        assert_eq!(parse_defaults("1\n"), Some(true));
        assert_eq!(
            parse_defaults("The domain/default pair does not exist"),
            None
        );
        assert_eq!(parse_windows("False\r\n"), Some(true));
        assert_eq!(parse_windows("True"), Some(false));
    }

    #[test]
    fn preference_overrides_the_system() {
        assert!(MotionPref::System.reduces(Some(true)));
        assert!(!MotionPref::System.reduces(None));
        assert!(MotionPref::Reduced.reduces(Some(false)));
        assert!(!MotionPref::Full.reduces(Some(true)));
    }
}
