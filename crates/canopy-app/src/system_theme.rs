//! The system's light or dark mode, for Settings → Appearance → Match system.
//!
//! macOS and Windows report it through the window toolkit (`egui::Context::system_theme`), and
//! say so again whenever it changes. Linux reports nothing there, so the desktop is asked
//! instead ([`crate::platform::prefers_dark`]) on a background thread: at start-up, when Match
//! system is switched on, and each time the window gets the focus back, which is when the user
//! returns from changing it.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// What the desktop said: 0 not known yet, 1 light, 2 dark.
static ANSWER: AtomicU8 = AtomicU8::new(0);
static QUERYING: AtomicBool = AtomicBool::new(false);

/// Whether the system is dark: the window toolkit's answer, else the desktop's once it has
/// arrived. `None` while unknown.
pub fn dark(ctx: &egui::Context) -> Option<bool> {
    ctx.system_theme()
        .map(|theme| theme == egui::Theme::Dark)
        .or_else(|| match ANSWER.load(Ordering::Relaxed) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        })
}

/// Asks the desktop on a background thread (the query runs a helper program) and repaints when
/// the answer changed, so the app re-applies the theme. One query at a time; an answer that
/// cannot be read keeps the last one.
pub fn query(ctx: &egui::Context) {
    if QUERYING.swap(true, Ordering::Relaxed) {
        return;
    }
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("theme-query".into())
        .spawn(move || {
            if let Some(dark) = crate::platform::prefers_dark() {
                let answer = if dark { 2 } else { 1 };
                if ANSWER.swap(answer, Ordering::Relaxed) != answer {
                    ctx.request_repaint();
                }
            }
            QUERYING.store(false, Ordering::Relaxed);
        });
    if let Err(e) = spawned {
        QUERYING.store(false, Ordering::Relaxed);
        tracing::warn!("could not ask the system about dark mode: {e}");
    }
}
