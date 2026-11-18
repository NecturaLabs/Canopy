//! Local diagnostics: structured logs and crash reports, both in Canopy's state folder
//! (`~/.local/state/canopy/` on Linux). Nothing here is uploaded anywhere; the user decides
//! whether to share a file.
//!
//! - Each run writes `logs/canopy-<UTC start>-<pid>.jsonl`: one JSON object per line with
//!   `timestamp`, `level`, `target`, `threadName`, the current `span` (repository job, git
//!   command, API call…) and the event's fields. The first line records the run: version,
//!   platform and a random run id. The newest [`KEEP_RUNS`] files are kept and a file stops
//!   growing at [`MAX_RUN_BYTES`]. Standard error gets the same events as readable text.
//! - The level comes from `CANOPY_LOG` (or `RUST_LOG`) if set, otherwise from Settings →
//!   Privacy → Log detail, and changes without a restart ([`set_level`]).
//! - The last warnings and errors are kept in memory for Settings → Copy diagnostics.
//! - A panic writes `crashes/crash-<UTC timestamp>.txt` with the message, location, version,
//!   platform and a backtrace, then lets the default hook run. The next launch offers to open the
//!   newest report it has not shown before.
//!
//! Secrets never reach a log: tokens and passwords are not logged at all, git arguments go
//! through `canopy_core::cli::loggable` (URL credentials, commit messages and config values
//! removed), and answers to credential prompts are not logged.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, reload};

/// Run logs kept; older ones are deleted at start-up.
pub const KEEP_RUNS: usize = 10;
/// A run's log stops growing at this size (one line says so).
pub const MAX_RUN_BYTES: u64 = 20 * 1024 * 1024;
/// Crash reports kept; older ones are deleted at start-up.
pub const KEEP_CRASHES: usize = 10;
/// Warnings and errors remembered for diagnostics.
pub const RECENT: usize = 50;

/// How much the log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LogLevel {
    /// Start-up, operations and their outcome, warnings and errors.
    #[default]
    Normal,
    /// Also every git command, API call and repository job, with timings.
    Detailed,
    /// Everything, including libraries' own diagnostics. For chasing a specific problem.
    Trace,
}

impl LogLevel {
    pub const ALL: [Self; 3] = [Self::Normal, Self::Detailed, Self::Trace];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Detailed => "Detailed",
            Self::Trace => "Everything",
        }
    }

    /// The `tracing` filter. Canopy's own crates are `canopy` (the app), `canopy_core` and
    /// `canopy_hosting`.
    pub fn filter(self) -> &'static str {
        match self {
            Self::Normal => "warn,canopy=info,canopy_core=info,canopy_hosting=info",
            Self::Detailed => "info,canopy=debug,canopy_core=debug,canopy_hosting=debug",
            Self::Trace => "debug,canopy=trace,canopy_core=trace,canopy_hosting=trace",
        }
    }
}

/// Where logs go, if the platform has a home for them.
pub fn log_dir() -> Option<PathBuf> {
    crate::storage::layout().map(|l| l.logs.clone())
}

/// Where crash reports go.
pub fn crash_dir() -> Option<PathBuf> {
    crate::storage::layout().map(|l| l.crashes.clone())
}

/// What identifies this run in its log and in diagnostics.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: String,
    pub file: Option<PathBuf>,
}

static RUN: OnceLock<Run> = OnceLock::new();
static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static RECENT_EVENTS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// This run's id and log file.
pub fn run() -> Option<&'static Run> {
    RUN.get()
}

/// The newest warnings and errors, oldest first, as `time level target: message` lines.
pub fn recent_problems() -> Vec<String> {
    RECENT_EVENTS
        .lock()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default()
}

/// The filter from the environment, if the user set one (it wins over the setting).
fn env_filter() -> Option<String> {
    std::env::var("CANOPY_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
        .filter(|f| !f.trim().is_empty())
}

/// Applies `level` now, unless `CANOPY_LOG`/`RUST_LOG` chose the filter.
pub fn set_level(level: LogLevel) {
    if env_filter().is_some() {
        return;
    }
    if let Some(handle) = FILTER.get() {
        let _ = handle.reload(EnvFilter::new(level.filter()));
        tracing::info!(level = level.label(), "log detail changed");
    }
}

/// Deletes all but the newest `keep - 1` run logs (making room for this run's) and the files of
/// the old single-file format.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "Canopy names these files itself, in lower case"
)]
fn prune(dir: &Path, keep: usize) {
    let mut runs: Vec<PathBuf> = fs::read_dir(dir)
        .map(|it| it.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    runs.retain(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name == "canopy.log" || name.starts_with("canopy.log.") {
            let _ = fs::remove_file(p);
            return false;
        }
        name.starts_with("canopy-") && name.ends_with(".jsonl")
    });
    runs.sort();
    let excess = runs.len().saturating_sub(keep.saturating_sub(1));
    for old in &runs[..excess] {
        let _ = fs::remove_file(old);
    }
}

/// A log file that stops at a size limit instead of filling the disk.
struct Capped {
    file: File,
    written: u64,
    limit: u64,
    full: bool,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.full {
            return Ok(buf.len());
        }
        if self.written + buf.len() as u64 > self.limit {
            self.full = true;
            let note = format!(
                "{{\"timestamp\":\"{}\",\"level\":\"WARN\",\"target\":\"canopy::logging\",\"message\":\"log size limit reached; later events are not written\"}}\n",
                jiff::Timestamp::now()
            );
            self.file.write_all(note.as_bytes())?;
            return Ok(buf.len());
        }
        self.written += buf.len() as u64;
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn open_run_log(dir: &Path) -> Option<(PathBuf, Capped)> {
    fs::create_dir_all(dir).ok()?;
    prune(dir, KEEP_RUNS);
    let stamp = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("canopy-{stamp}-{}.jsonl", std::process::id()));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    Some((
        path,
        Capped {
            file,
            written: 0,
            limit: MAX_RUN_BYTES,
            full: false,
        },
    ))
}

/// Remembers warnings and errors for diagnostics.
struct RecentProblems;

impl<S: tracing::Subscriber> Layer<S> for RecentProblems {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > tracing::Level::WARN {
            return;
        }
        let mut text = String::new();
        event.record(&mut MessageVisitor(&mut text));
        let line = format!(
            "{} {} {}: {}",
            jiff::Timestamp::now().strftime("%H:%M:%S"),
            meta.level(),
            meta.target(),
            text
        );
        if let Ok(mut recent) = RECENT_EVENTS.lock() {
            if recent.len() == RECENT {
                recent.pop_front();
            }
            recent.push_back(line);
        }
    }
}

/// Writes an event's fields as `message key=value …`.
struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
            if self.0.starts_with('"') {
                #[expect(
                    clippy::assigning_clones,
                    reason = "the source borrows the target, so clone_into cannot take it"
                )]
                {
                    *self.0 = self.0.trim_matches('"').to_owned();
                }
            }
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        use std::fmt::Write as _;
        if field.name() == "message" {
            self.0.insert_str(0, value);
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }
}

fn run_id() -> String {
    use std::fmt::Write as _;

    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Installs the loggers and the crash-report hook. `level` applies unless `CANOPY_LOG` or
/// `RUST_LOG` is set.
pub fn init(level: LogLevel) {
    let filter = env_filter().unwrap_or_else(|| level.filter().to_owned());
    let (filter_layer, handle) = reload::Layer::new(EnvFilter::new(&filter));
    let stderr = tracing_subscriber::fmt::layer()
        .compact()
        .with_writer(std::io::stderr);
    let opened = log_dir().and_then(|d| open_run_log(&d));
    let path = opened.as_ref().map(|(p, _)| p.clone());
    let file = opened.map(|(_, w)| {
        tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .with_thread_names(true)
            .with_writer(Mutex::new(w))
    });
    let installed = tracing_subscriber::registry()
        .with(filter_layer)
        .with(stderr)
        .with(file)
        .with(RecentProblems)
        .try_init()
        .is_ok();
    if installed {
        let _ = FILTER.set(handle);
    }
    let run = Run {
        id: run_id(),
        file: path,
    };
    tracing::info!(
        run = %run.id,
        version = %crate::build_info::label(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        pid = std::process::id(),
        filter = %filter,
        "canopy started"
    );
    let _ = RUN.set(run);
    install_panic_hook();
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = crash_dir() {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(no message)".to_owned());
            let location = info.location().map(|l| (l.file(), l.line()));
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned();
            match write_panic_report(&dir, &message, location, &thread) {
                Ok(Some(path)) => tracing::error!("crash report written to {}", path.display()),
                Ok(None) => {}
                Err(e) => tracing::error!("could not write a crash report: {e}"),
            }
        }
        default(info);
    }));
}

fn write_panic_report(
    dir: &Path,
    message: &str,
    location: Option<(&str, u32)>,
    thread: &str,
) -> std::io::Result<Option<PathBuf>> {
    if location
        .is_some_and(|(file, _)| crate::crash_reports::is_accessibility_bus_panic(message, file))
    {
        return Ok(None);
    }
    let location = location
        .map(|(file, line)| format!("{file}:{line}"))
        .unwrap_or_default();
    let backtrace = std::backtrace::Backtrace::force_capture();
    let report = render_report(message, &location, thread, &backtrace.to_string());
    write_report(dir, &report).map(Some)
}

/// The text of a crash report.
fn render_report(message: &str, location: &str, thread: &str, backtrace: &str) -> String {
    format!(
        "Canopy crash report\n\
         ===================\n\n\
         Version:  {version}\n\
         Platform: {os} {arch}\n\
         Time:     {time}\n\
         Thread:   {thread}\n\
         Location: {location}\n\n\
         Message:\n{message}\n\n\
         Backtrace:\n{backtrace}\n\n\
         This report stays on your computer. If you share it, check it first: it can contain\n\
         file paths and branch names.\n",
        version = crate::build_info::label(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        time = jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ"),
    )
}

fn write_report(dir: &Path, report: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
    let mut path = dir.join(format!("crash-{stamp}.txt"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("crash-{stamp}-{n}.txt"));
        n += 1;
    }
    let mut f = File::create(&path)?;
    f.write_all(report.as_bytes())?;
    Ok(path)
}

/// Crash reports in `dir`, oldest first (their names sort by time).
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "Canopy names these files itself, in lower case"
)]
fn reports(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// A crash report the user has not been shown yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    pub path: PathBuf,
    /// The panic message, for the dialog.
    pub message: String,
}

/// Deletes all but the newest [`KEEP_CRASHES`] reports and returns the newest one whose file
/// name sorts after `last_seen` (the name of the last report shown).
pub fn unseen_crash(last_seen: &str) -> Option<CrashReport> {
    unseen_in(&crash_dir()?, last_seen)
}

fn unseen_in(dir: &Path, last_seen: &str) -> Option<CrashReport> {
    let all = reports(dir);
    let excess = all.len().saturating_sub(KEEP_CRASHES);
    for old in &all[..excess] {
        let _ = fs::remove_file(old);
    }
    let newest = all.last()?;
    let name = newest.file_name()?.to_str()?;
    if name <= last_seen {
        return None;
    }
    let text = fs::read_to_string(newest).unwrap_or_default();
    let message = text
        .split_once("Message:\n")
        .and_then(|(_, rest)| rest.split("\n\n").next())
        .unwrap_or("")
        .trim()
        .chars()
        .take(300)
        .collect();
    Some(CrashReport {
        path: newest.clone(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_accessibility_registry_does_not_create_a_local_crash_report() {
        let dir = tempfile::tempdir().expect("tempdir");
        let crashes = dir.path().join("crashes");
        let message = "called `Result::unwrap()` on an `Err` value: MethodError(OwnedErrorName(\"org.freedesktop.DBus.Error.ServiceUnknown\"), Some(\"The name org.a11y.atspi.Registry was not provided by any .service files\"), Msg { type: Error })";
        let source = "/build/registry/accesskit_unix-0.21.1/src/context.rs";
        let report = write_panic_report(&crashes, message, Some((source, 61)), "unnamed")
            .expect("write panic report");
        assert_eq!(report, None, "nonfatal accessibility panic was reported");
        assert!(
            !crashes.exists(),
            "nonfatal panic created a crash directory"
        );
        assert_eq!(
            unseen_in(&crashes, ""),
            None,
            "false crash dialog next launch"
        );
    }

    #[test]
    fn other_panics_still_create_local_reports() {
        let missing_registry = "called `Result::unwrap()` on an `Err` value: MethodError(OwnedErrorName(\"org.freedesktop.DBus.Error.ServiceUnknown\"), Some(\"The name org.a11y.atspi.Registry was not provided by any .service files\"), Msg { type: Error })";
        let accesskit = "/build/registry/accesskit_unix-0.21.1/src/context.rs";
        for (message, location) in [
            (missing_registry, Some(("src/worker.rs", 61))),
            (
                missing_registry,
                Some(("/build/accesskit_unix_other/src/context.rs", 61)),
            ),
            (
                missing_registry,
                Some(("/build/accesskit_unix-0.21.1/src/adapter.rs", 61)),
            ),
            (missing_registry, None),
            ("index out of bounds", Some(("src/worker.rs", 12))),
            (
                "called `Result::unwrap()` on an `Err` value: PoisonError { .. }",
                Some((accesskit, 61)),
            ),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let crashes = dir.path().join("crashes");
            let path = write_panic_report(&crashes, message, location, "unnamed")
                .expect("write report")
                .expect("real panic report");
            let report = unseen_in(&crashes, "").expect("next launch must offer real crash");
            assert_eq!(report.path, path);
            assert_eq!(
                report.message,
                message.chars().take(300).collect::<String>()
            );
        }
    }

    #[test]
    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "Canopy names these files itself, in lower case"
    )]
    fn old_run_logs_are_pruned() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("canopy.log"), b"old format").expect("write");
        fs::write(dir.path().join("canopy.log.2"), b"old format").expect("write");
        for i in 0..KEEP_RUNS + 4 {
            fs::write(
                dir.path()
                    .join(format!("canopy-2026010{i:02}T000000Z-1.jsonl")),
                b"{}",
            )
            .expect("write");
        }
        fs::write(dir.path().join("notes.txt"), b"not ours").expect("write");
        prune(dir.path(), KEEP_RUNS);
        let left: Vec<String> = fs::read_dir(dir.path())
            .expect("dir")
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .collect();
        assert_eq!(
            left.iter().filter(|n| n.ends_with(".jsonl")).count(),
            KEEP_RUNS - 1
        );
        assert!(left.contains(&"notes.txt".to_owned()));
        assert!(!left.iter().any(|n| n.starts_with("canopy.log")));
        // The newest survive.
        assert!(
            left.iter()
                .any(|n| n.contains(&format!("{:02}T", KEEP_RUNS + 3)))
        );
    }

    #[test]
    fn a_capped_log_stops_at_its_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("run.jsonl");
        let mut w = Capped {
            file: File::create(&path).expect("create"),
            written: 0,
            limit: 100,
            full: false,
        };
        for _ in 0..10 {
            w.write_all(&[b'x'; 30]).expect("write");
        }
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.len() < 300, "{}", text.len());
        assert_eq!(text.matches("size limit").count(), 1);
    }

    #[test]
    fn reports_are_found_once_and_pruned() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(unseen_in(dir.path(), ""), None);
        let report = render_report("index out of bounds", "src/x.rs:1", "main", "bt");
        let path = write_report(dir.path(), &report).expect("write");
        let found = unseen_in(dir.path(), "").expect("unseen");
        assert_eq!(found.path, path);
        assert_eq!(found.message, "index out of bounds");
        let name = path.file_name().and_then(|n| n.to_str()).expect("name");
        assert_eq!(unseen_in(dir.path(), name), None, "already shown");

        for i in 0..KEEP_CRASHES + 3 {
            fs::write(dir.path().join(format!("crash-0000{i:02}.txt")), "x").expect("write");
        }
        let _ = unseen_in(dir.path(), "");
        assert_eq!(reports(dir.path()).len(), KEEP_CRASHES);
    }
}
