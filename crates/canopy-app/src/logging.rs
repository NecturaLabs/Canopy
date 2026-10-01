//! Local diagnostics: a rotating log file and crash reports, both in Canopy's data directory
//! (`~/.local/share/canopy/` on Linux). Nothing here is uploaded anywhere; the user decides
//! whether to share a report.
//!
//! - `logs/canopy.log` receives everything the stderr logger does, without colours. It is rotated
//!   at start-up once it passes [`MAX_LOG_BYTES`], keeping [`KEEP_LOGS`] older files.
//! - A panic writes `crashes/crash-<UTC timestamp>.txt` with the message, location, version,
//!   platform and a backtrace, then lets the default hook run. The next launch offers to open the
//!   newest report it has not shown before.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Size at which `canopy.log` is rotated.
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
/// Rotated logs kept (`canopy.log.1` … `canopy.log.N`).
pub const KEEP_LOGS: usize = 3;
/// Crash reports kept; older ones are deleted at start-up.
pub const KEEP_CRASHES: usize = 10;

/// Canopy's data directory, if the platform has one.
pub fn data_dir() -> Option<PathBuf> {
    eframe::storage_dir("canopy")
}

/// Where logs go.
pub fn log_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("logs"))
}

/// Where crash reports go.
pub fn crash_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("crashes"))
}

/// Shifts `canopy.log` → `.1` → `.2` … when it is larger than `max`, dropping the oldest.
fn rotate(dir: &Path, max: u64, keep: usize) {
    let current = dir.join("canopy.log");
    let big = fs::metadata(&current).is_ok_and(|m| m.len() > max);
    if !big {
        return;
    }
    let _ = fs::remove_file(dir.join(format!("canopy.log.{keep}")));
    for n in (1..keep).rev() {
        let _ = fs::rename(
            dir.join(format!("canopy.log.{n}")),
            dir.join(format!("canopy.log.{}", n + 1)),
        );
    }
    let _ = fs::rename(&current, dir.join("canopy.log.1"));
}

fn open_log(dir: &Path) -> Option<File> {
    fs::create_dir_all(dir).ok()?;
    rotate(dir, MAX_LOG_BYTES, KEEP_LOGS);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("canopy.log"))
        .ok()
}

/// Installs the stderr logger, the log file (when the data directory is writable) and the
/// crash-report hook. `filter` is a `tracing` filter such as `info,canopy_core=debug`.
pub fn init(filter: &str) {
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(EnvFilter::new(filter));
    let file = log_dir().and_then(|d| open_log(&d)).map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(f))
            .with_filter(EnvFilter::new(filter))
    });
    let _ = tracing_subscriber::registry()
        .with(stderr)
        .with(file)
        .try_init();
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
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned();
            let backtrace = std::backtrace::Backtrace::force_capture();
            let report = render_report(&message, &location, &thread, &backtrace.to_string());
            match write_report(&dir, &report) {
                Ok(path) => tracing::error!("crash report written to {}", path.display()),
                Err(e) => tracing::error!("could not write a crash report: {e}"),
            }
        }
        default(info);
    }));
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
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        time = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

fn write_report(dir: &Path, report: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
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
    fn rotation_keeps_a_bounded_number_of_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        for round in 0..5 {
            fs::write(dir.path().join("canopy.log"), vec![b'x'; 20]).expect("write");
            rotate(dir.path(), 10, 3);
            assert!(!dir.path().join("canopy.log").exists(), "round {round}");
        }
        assert!(dir.path().join("canopy.log.1").exists());
        assert!(dir.path().join("canopy.log.3").exists());
        assert!(!dir.path().join("canopy.log.4").exists());
        // Small logs stay.
        fs::write(dir.path().join("canopy.log"), b"ok").expect("write");
        rotate(dir.path(), 10, 3);
        assert!(dir.path().join("canopy.log").exists());
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
