//! Crash reports sent to Sentry, only with the user's consent (asked on first start; changed in
//! Settings → Privacy). What a report contains is documented for users in
//! `docs/legal/privacy-policy.md`; keep the two in step.
//!
//! Only panics are reported. There is no logging integration, so Canopy's logs never leave the
//! computer, and [`scrub`] runs on every event before it is sent: it drops the host name, the
//! user and any request data, keeps only the operating system, Rust and processor-architecture
//! contexts, and in every string hides the user's home folder (and other users' folders),
//! credentials in URLs and access tokens (see [`scrub_text`]).
//! Release health counts sessions under a random id that is never stored.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use sentry::integrations;
use sentry::protocol::{Context, Event, Exception, Mechanism};

/// Where reports go: Canopy's Sentry project (EU region). Packagers can point a build elsewhere
/// through the `CANOPY_SENTRY_DSN` environment variable at compile time, or build without crash
/// reporting with the value `off`. An empty value (an unset CI variable) keeps the built-in one.
pub const DSN: &str = match option_env!("CANOPY_SENTRY_DSN") {
    Some(dsn) if !dsn.is_empty() => dsn,
    _ => {
        "https://421ae4df9c4d9c1ee673f85944548015@o4510922848403456.ingest.de.sentry.io/4512180345438288"
    }
};

/// Bumped whenever a crash report or session record starts carrying more than the Privacy Policy
/// described when the user agreed. A user who said yes to an older version is asked again.
pub const DATA_VERSION: u32 = 1;

/// The client while reporting is on. Dropping it flushes and ends the session.
static GUARD: Mutex<Option<sentry::ClientInitGuard>> = Mutex::new(None);

/// Whether this build can send crash reports at all.
pub fn available() -> bool {
    let dsn = DSN.trim();
    !dsn.is_empty() && dsn != "off"
}

/// Turns reporting on or off immediately.
#[expect(
    clippy::significant_drop_tightening,
    reason = "the lock is held until the client is started or stopped, so two calls cannot race"
)]
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
    o.release = Some(Cow::Owned(format!("canopy@{}", crate::build_info::NUMBER)));
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
    // The default integrations, except that panics carry the file and line where they happened:
    // release builds have no symbols, so the stack trace alone would only be addresses.
    o.default_integrations = false;
    o.integrations = vec![
        Arc::new(integrations::backtrace::AttachStacktraceIntegration),
        Arc::new(integrations::contexts::ContextIntegration::default()),
        Arc::new(integrations::panic::PanicIntegration::new().add_extractor(panic_event)),
        Arc::new(integrations::backtrace::ProcessStacktraceIntegration),
    ];
    sentry::init(o)
}

/// The event for a panic: its message followed by where it happened, `src/x.rs:12:5`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "sentry's panic extractor returns an Option"
)]
fn panic_event(info: &std::panic::PanicHookInfo<'_>) -> Option<Event<'static>> {
    let message = integrations::panic::message_from_panic_info(info);
    let value = match info.location() {
        Some(l) => format!("{message} (at {}:{}:{})", l.file(), l.line(), l.column()),
        None => message.to_owned(),
    };
    Some(Event {
        exception: vec![Exception {
            ty: "panic".into(),
            mechanism: Some(Mechanism {
                ty: "panic".into(),
                handled: Some(false),
                ..Default::default()
            }),
            value: Some(value),
            stacktrace: integrations::backtrace::current_stacktrace(),
            ..Default::default()
        }]
        .into(),
        level: sentry::Level::Fatal,
        ..Default::default()
    })
}

/// Hides what a report or the diagnostics text must not carry:
///
/// - credentials in URLs (`https://user:secret@host` becomes `https://<redacted>@host`);
/// - GitHub and GitLab access tokens (`ghp_…`, `github_pat_…`, `glpat-…` and the like),
///   `Bearer` and `PRIVATE-TOKEN` values, and Canopy's credential-prompt token, as `<token>`;
/// - the user's home folder `home` as `~`, only where it is a whole path component (with
///   `home` = `/home/ada`, `/home/adam` is another user's folder, not `~m`);
/// - the user-name part of other users' home folders as `<user>`, and root's `/root` as `~root`.
pub fn scrub_text(text: &str, home: &str) -> String {
    let out = redact_url_credentials(text);
    let out = redact_tokens(&out);
    let out = replace_home(&out, home);
    hide_user_folders(&out)
}

/// A character that can continue a file or user name, so a match ending before it is only the
/// start of a longer name.
fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | '+')
}

/// Whether `text[at..at + len]` is a whole path: not inside a longer path before it, and
/// followed by a separator or a character that cannot be part of a name.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find, match_indices and ASCII lengths, so they fall between characters"
)]
fn whole_path(text: &str, at: usize, len: usize) -> bool {
    let before = text[..at].chars().next_back();
    let after = text[at + len..].chars().next();
    let starts = before.is_none_or(|c| !(name_char(c) || matches!(c, '/' | '\\' | '~')));
    let ends = after.is_none_or(|c| matches!(c, '/' | '\\') || !name_char(c));
    starts && ends
}

/// Replaces each whole-path occurrence of `path` in `text` with `with`.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find, match_indices and ASCII lengths, so they fall between characters"
)]
fn replace_path(text: &str, path: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in text.match_indices(path) {
        if at < last || !whole_path(text, at, path.len()) {
            continue;
        }
        out.push_str(&text[last..at]);
        out.push_str(with);
        last = at + path.len();
    }
    out.push_str(&text[last..]);
    out
}

/// Replaces the user's home folder with `~`.
fn replace_home(text: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if home.len() <= 1 {
        return text.to_owned();
    }
    replace_path(text, home, "~")
}

/// Replaces `scheme://user:password@` with `scheme://<redacted>@` in every URL in `text`.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find, match_indices and ASCII lengths, so they fall between characters"
)]
fn redact_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let authority_end = tail
            .find(|c: char| {
                matches!(c, '/' | '?' | '#' | '\'' | '"' | '<' | '>' | '`') || c.is_whitespace()
            })
            .unwrap_or(tail.len());
        match tail[..authority_end].rfind('@') {
            Some(at) => {
                out.push_str("<redacted>");
                rest = &tail[at..];
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

/// Prefixes of GitHub and GitLab access tokens.
const TOKEN_PREFIXES: [&str; 11] = [
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "gldt-",
    "glptt-",
    "glrt-",
    "gloas-",
];

/// Labels whose value is a secret: the value after them is hidden, the label kept.
const SECRET_LABELS: [&str; 4] = [
    "Bearer ",
    "PRIVATE-TOKEN: ",
    "PRIVATE-TOKEN=",
    // The credential prompt's token (see `askpass`), should an environment dump reach a report.
    "CANOPY_ASKPASS_TOKEN=",
];

/// Characters a token is made of.
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

/// Replaces access tokens and secret values in `text` with `<token>`.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find, match_indices and ASCII lengths, so they fall between characters"
)]
fn redact_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < text.len() {
        let rest = &text[i..];
        let at_word_start = text[..i].chars().next_back().is_none_or(|c| !token_char(c));
        if at_word_start {
            for prefix in TOKEN_PREFIXES {
                if let Some(value) = rest.strip_prefix(prefix) {
                    let len = value.find(|c| !token_char(c)).unwrap_or(value.len());
                    // Real tokens are long; a bare prefix in prose is left alone.
                    if len >= 8 {
                        out.push_str("<token>");
                        i += prefix.len() + len;
                        continue 'scan;
                    }
                }
            }
        }
        for label in SECRET_LABELS {
            if let Some(value) = rest.strip_prefix(label) {
                let len = value
                    .find(|c: char| !(token_char(c) || matches!(c, '.' | '~' | '+' | '/' | '=')))
                    .unwrap_or(value.len());
                if len > 0 {
                    out.push_str(label);
                    out.push_str("<token>");
                    i += label.len() + len;
                    continue 'scan;
                }
            }
        }
        let Some(c) = rest.chars().next() else { break };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Hides the user-name part of home folders other than the user's own.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find, match_indices and ASCII lengths, so they fall between characters"
)]
fn hide_user_folders(text: &str) -> String {
    let mut out = replace_path(text, "/root", "~root");
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
    event.breadcrumbs = sentry::protocol::Values::default();
    event.extra.clear();
    event.tags.retain(|k, _| k == "os" || k == "release");
    event
        .contexts
        .retain(|k, _| matches!(k.as_str(), "os" | "rust" | "device"));
    if let Some(Context::Os(os)) = event.contexts.get_mut("os") {
        // The kernel string can name the distribution's build and date; the name and version
        // are enough.
        os.kernel_version = None;
        os.build = None;
        os.other.clear();
    }
    if let Some(Context::Device(device)) = event.contexts.get_mut("device") {
        let arch = device.arch.take();
        **device = sentry::protocol::DeviceContext::default();
        device.arch = arch;
    }
    let s = |t: &mut String| *t = scrub_text(t, home);
    if let Some(m) = event.message.as_mut() {
        s(m);
    }
    if let Some(l) = event.logentry.as_mut() {
        s(&mut l.message);
    }
    for exc in &mut event.exception.values {
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
    for thread in &mut event.threads.values {
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
    use sentry::protocol::{DeviceContext, User};

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
    fn home_is_replaced_only_as_a_whole_path() {
        let home = "/home/ada";
        assert_eq!(scrub_text("/home/adam/x", home), "/home/<user>/x");
        assert_eq!(scrub_text("/home/ada.old/x", home), "/home/<user>/x");
        assert_eq!(scrub_text("in /home/ada", home), "in ~");
        assert_eq!(scrub_text("'/home/ada/a' '/home/ada'", home), "'~/a' '~'");
        assert_eq!(
            scrub_text("cd /home/ada/ && ls", "/home/ada/"),
            "cd ~/ && ls"
        );
        // Inside another path it is not the home folder.
        assert_eq!(scrub_text("/mnt/home/ada/x", home), "/mnt/home/<user>/x");
        assert_eq!(
            scrub_text("C:\\Users\\Ada\\repo C:\\Users\\Adam\\x", "C:\\Users\\Ada"),
            "~\\repo C:\\Users\\<user>\\x"
        );
    }

    #[test]
    fn roots_home_is_covered() {
        assert_eq!(
            scrub_text("open /root/src/x failed", "/root"),
            "open ~/src/x failed"
        );
        assert_eq!(
            scrub_text("open /root/src/x failed", "/home/ada"),
            "open ~root/src/x failed"
        );
        assert_eq!(
            scrub_text("/rootfs/x and /srv/root/y", ""),
            "/rootfs/x and /srv/root/y"
        );
        // With no home folder known.
        assert_eq!(scrub_text("/root", ""), "~root");
    }

    #[test]
    fn url_credentials_are_redacted() {
        assert_eq!(
            scrub_text(
                "fatal: unable to access 'https://ada:s3cret@github.com/o/r.git/': 403",
                ""
            ),
            "fatal: unable to access 'https://<redacted>@github.com/o/r.git/': 403"
        );
        assert_eq!(
            scrub_text(
                "a=https://x:y@h/1,b=http://oauth2:tok@gitlab.corp:8443/g",
                ""
            ),
            "a=https://<redacted>@h/1,b=http://<redacted>@gitlab.corp:8443/g"
        );
        assert_eq!(
            scrub_text("https://host/a@b and ssh://git@host/x", ""),
            "https://host/a@b and ssh://<redacted>@host/x"
        );
        assert_eq!(scrub_text("no url: a@b", ""), "no url: a@b");
    }

    #[test]
    fn access_tokens_are_redacted() {
        let cases = [
            (
                "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                "token <token>",
            ),
            ("gho_ABCDEFGH12345678.", "<token>."),
            ("(ghu_ABCDEFGH12345678)", "(<token>)"),
            (
                "ghs_ABCDEFGH12345678 ghr_ABCDEFGH12345678",
                "<token> <token>",
            ),
            ("github_pat_11ABCDEFG0_abcdefghijklmnop", "<token>"),
            (
                "GITLAB_TOKEN=glpat-AbCdEf123456-xyz",
                "GITLAB_TOKEN=<token>",
            ),
            (
                "Authorization: Bearer abc.def-ghi",
                "Authorization: Bearer <token>",
            ),
            (
                "PRIVATE-TOKEN: glpat-xxxxxxxxxxxxxxxx",
                "PRIVATE-TOKEN: <token>",
            ),
            (
                "CANOPY_ASKPASS_TOKEN=0a1b2c3d4e5f",
                "CANOPY_ASKPASS_TOKEN=<token>",
            ),
        ];
        for (text, scrubbed) in cases {
            assert_eq!(scrub_text(text, ""), scrubbed, "{text}");
        }
        // Prefixes in ordinary words, or too short to be a token, stay.
        for text in ["a ghp_ prefix", "laughs_x", "myghp_abcdefghijk", "glpat-"] {
            assert_eq!(scrub_text(text, ""), text);
        }
    }

    #[test]
    fn everything_is_scrubbed_together() {
        assert_eq!(
            scrub_text(
                "push https://ada:ghp_abcdefgh12345678@github.com/o/r from /home/ada/r",
                "/home/ada"
            ),
            "push https://<redacted>@github.com/o/r from ~/r"
        );
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
        event.contexts.insert(
            "app".into(),
            Context::Other(std::collections::BTreeMap::default()),
        );
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
