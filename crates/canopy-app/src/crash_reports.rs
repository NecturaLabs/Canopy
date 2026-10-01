//! Crash reports sent to Sentry, only with the user's consent (asked on first start; changed in
//! Settings → Privacy). What a report contains is documented for users in
//! `docs/legal/privacy-policy.md`; keep the two in step.
//!
//! Only panics are reported. There is no logging integration, so Canopy's logs never leave the
//! computer, and [`scrub`] runs on every event before it is sent: it drops the host name, the
//! user and any request data, keeps only the operating system, Rust and processor-architecture
//! contexts, and hides the user's home folder (and other users' folders) in every string.
//! Release health counts sessions under a random id that is never stored.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use sentry::protocol::{Context, Event};

/// Where reports go: Canopy's Sentry project (EU region). Packagers can point a build elsewhere,
/// or build without crash reporting with an empty value, through the `CANOPY_SENTRY_DSN`
/// environment variable at compile time.
pub const DSN: &str = match option_env!("CANOPY_SENTRY_DSN") {
    Some(dsn) => dsn,
    None => {
        "https://421ae4df9c4d9c1ee673f85944548015@o4510922848403456.ingest.de.sentry.io/4512180345438288"
    }
};

/// The client while reporting is on. Dropping it flushes and ends the session.
static GUARD: Mutex<Option<sentry::ClientInitGuard>> = Mutex::new(None);

/// Whether this build can send crash reports at all.
pub fn available() -> bool {
    !DSN.trim().is_empty()
}

/// Turns reporting on or off immediately.
pub fn set_enabled(on: bool) {
    let mut guard = GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match (on && available(), guard.is_some()) {
        (true, false) => *guard = Some(init()),
        (false, true) => {
            // Ends the session and stops the client; the panic hook stays but has nothing to
            // send to.
            if let Some(g) = guard.take() {
                drop(g);
            }
            sentry::Hub::main().bind_client(None);
        }
        _ => {}
    }
}

/// Flushes and ends the session at exit.
pub fn shutdown() {
    let mut guard = GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.take();
}

fn init() -> sentry::ClientInitGuard {
    let home = crate::settings::home_dir().display().to_string();
    let mut o = sentry::ClientOptions::default();
    o.dsn = DSN.parse().ok();
    o.release = Some(Cow::Owned(format!("canopy@{}", env!("CARGO_PKG_VERSION"))));
    o.environment = Some(Cow::Borrowed(if cfg!(debug_assertions) {
        "development"
    } else {
        "production"
    }));
    o.send_default_pii = false;
    o.server_name = None;
    o.max_breadcrumbs = 0;
    o.attach_stacktrace = false;
    o.auto_session_tracking = true;
    o.session_mode = sentry::SessionMode::Application;
    o.before_send = Some(Arc::new(move |event| Some(scrub(event, &home))));
    o.before_breadcrumb = Some(Arc::new(|_| None));
    sentry::init(o)
}

/// Replaces `home` with `~`, and the user-name part of other home-folder paths with `<user>`.
fn scrub_text(text: &str, home: &str) -> String {
    let mut out = if home.len() > 1 {
        text.replace(home, "~")
    } else {
        text.to_owned()
    };
    for prefix in ["/home/", "/Users/", "\\Users\\", "/var/home/"] {
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(i) = rest.find(prefix) {
            let (before, after) = rest.split_at(i + prefix.len());
            result.push_str(before);
            // User names can contain spaces: hide everything up to the next separator.
            let end = after
                .find(['/', '\\', '\'', '"', '\n'])
                .unwrap_or(after.len());
            if end > 0 {
                result.push_str("<user>");
            }
            rest = &after[end..];
        }
        result.push_str(rest);
        out = result;
    }
    out
}

/// Removes everything a report must not carry (see the module documentation).
fn scrub(mut event: Event<'static>, home: &str) -> Event<'static> {
    event.server_name = None;
    event.user = None;
    event.request = None;
    event.breadcrumbs = Default::default();
    event.extra.clear();
    event.tags.retain(|k, _| k == "os" || k == "release");
    event
        .contexts
        .retain(|k, _| matches!(k.as_str(), "os" | "rust" | "device"));
    if let Some(Context::Device(device)) = event.contexts.get_mut("device") {
        let arch = device.arch.take();
        **device = Default::default();
        device.arch = arch;
    }
    let s = |t: &mut String| *t = scrub_text(t, home);
    if let Some(m) = event.message.as_mut() {
        s(m);
    }
    if let Some(l) = event.logentry.as_mut() {
        s(&mut l.message);
    }
    for exc in event.exception.values.iter_mut() {
        if let Some(v) = exc.value.as_mut() {
            s(v);
        }
        if let Some(st) = exc.stacktrace.as_mut() {
            for f in &mut st.frames {
                for field in [&mut f.abs_path, &mut f.filename] {
                    if let Some(v) = field.as_mut() {
                        s(v);
                    }
                }
            }
        }
    }
    for thread in event.threads.values.iter_mut() {
        if let Some(st) = thread.stacktrace.as_mut() {
            for f in &mut st.frames {
                for field in [&mut f.abs_path, &mut f.filename] {
                    if let Some(v) = field.as_mut() {
                        s(v);
                    }
                }
            }
        }
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentry::protocol::{DeviceContext, Exception, User};

    #[test]
    fn hides_user_folders() {
        assert_eq!(
            scrub_text("open /home/ada/src/x failed", "/home/ada"),
            "open ~/src/x failed"
        );
        assert_eq!(
            scrub_text("C:\\Users\\Ada Lovelace\\repo and /Users/bob/x", ""),
            "C:\\Users\\<user>\\repo and /Users/<user>/x"
        );
        assert_eq!(scrub_text("/home/", ""), "/home/");
        assert_eq!(scrub_text("no paths", "/home/ada"), "no paths");
    }

    #[test]
    fn scrub_removes_identifying_fields() {
        let mut event = Event {
            server_name: Some("ada-laptop".into()),
            user: Some(User {
                username: Some("ada".into()),
                ..Default::default()
            }),
            message: Some("panicked at /home/ada/x.rs".into()),
            ..Default::default()
        };
        event.exception.values.push(Exception {
            value: Some("index out of range in /home/ada/repo".into()),
            ..Default::default()
        });
        event.contexts.insert(
            "device".into(),
            Context::Device(Box::new(DeviceContext {
                model: Some("ThinkPad X1".into()),
                arch: Some("x86_64".into()),
                ..Default::default()
            })),
        );
        event
            .contexts
            .insert("app".into(), Context::Other(Default::default()));
        event.extra.insert("x".into(), "y".into());
        let e = scrub(event, "/home/ada");
        assert_eq!(e.server_name, None);
        assert!(e.user.is_none() && e.extra.is_empty());
        assert_eq!(e.message.as_deref(), Some("panicked at ~/x.rs"));
        assert_eq!(
            e.exception.values[0].value.as_deref(),
            Some("index out of range in ~/repo")
        );
        assert!(!e.contexts.contains_key("app"));
        match e.contexts.get("device") {
            Some(Context::Device(d)) => {
                assert_eq!(d.model, None);
                assert_eq!(d.arch.as_deref(), Some("x86_64"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn this_build_reports_to_the_eu_project() {
        if option_env!("CANOPY_SENTRY_DSN").is_some() {
            return;
        }
        assert!(available());
        assert!(DSN.contains(".de.sentry.io"), "EU data region");
    }
}
