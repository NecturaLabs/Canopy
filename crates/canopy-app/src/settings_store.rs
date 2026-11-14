//! The settings file: `settings.json` in Canopy's config folder (ADR 0016).
//!
//! The file is `{"schema": 1, "settings": {…}}`. `schema` is the version of this envelope and the
//! rules for reading what is in it; `settings` is [`Settings`], whose fields all have defaults, so
//! a file from an older build loads with the new fields at their defaults.
//!
//! **Writing** never touches the live file in place. [`Writer`] owns a thread; the frame that
//! saves only serialises the settings (a few kilobytes) and hands the text over. The thread writes
//! it to a temporary file beside the real one, flushes it to disk, renames it over the real one
//! and flushes the folder. A rename replaces a file whole, so a crash, a power cut or a full disk
//! at any point leaves the previous file, complete.
//!
//! **Reading** an unreadable file (not JSON, cut short, a value of the wrong kind, or written by a
//! newer Canopy) never ends in an error dialog or in silently starting from defaults and then
//! overwriting it: [`load`] moves the file to `settings.json.bad` and says so, and the app tells
//! the user once in the notification center.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::Serialize;
use serde_json::Value;

use crate::settings::Settings;

/// The newest `schema` this build reads and the one it writes.
pub const SCHEMA: u64 = 1;
/// The largest settings file Canopy reads.
const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// How long a temporary file from a write that never finished lingers before it is swept.
const STALE_TEMP: Duration = Duration::from_hours(1);

/// What reading the settings file found.
#[derive(Debug)]
pub enum Loaded {
    /// No file: a first start, or the settings are still in eframe's storage (before this layout).
    Missing,
    Settings(Box<Settings>),
    /// The file could not be used. `kept` is where it was moved to, unless that failed too.
    Unreadable {
        reason: String,
        kept: Option<PathBuf>,
    },
}

#[derive(Serialize)]
struct Envelope<'a> {
    schema: u64,
    settings: &'a Settings,
}

/// The file's text for `settings`.
fn encode(settings: &Settings) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec_pretty(&Envelope {
        schema: SCHEMA,
        settings,
    })
}

fn decode(bytes: &[u8]) -> Result<Settings, String> {
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("it is not valid JSON ({e})"))?;
    let schema = value
        .get("schema")
        .and_then(Value::as_u64)
        .ok_or("it has no schema number")?;
    if schema > SCHEMA {
        return Err(format!(
            "it was written by a newer Canopy (schema {schema}, this build reads up to {SCHEMA})"
        ));
    }
    let settings = value
        .get_mut("settings")
        .map(Value::take)
        .ok_or("it has no settings")?;
    serde_json::from_value(settings).map_err(|e| format!("its settings are not readable ({e})"))
}

/// Reads the settings at `path`. An unreadable file is moved to `<path>.bad` (replacing an older
/// one) so the next write cannot destroy it.
pub fn load(path: &Path) -> Loaded {
    sweep_stale_temps(path);
    let bytes = match read_capped(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Loaded::Missing,
        Err(e) => return unreadable(path, format!("it cannot be read ({e})")),
    };
    match decode(&bytes) {
        Ok(settings) => Loaded::Settings(Box::new(settings)),
        Err(reason) => unreadable(path, reason),
    }
}

fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("the file is larger than 16 MiB"));
    }
    Ok(bytes)
}

fn unreadable(path: &Path, reason: String) -> Loaded {
    let bad = aside_path(path);
    let kept = match fs::rename(path, &bad) {
        Ok(()) => Some(bad),
        Err(e) => {
            tracing::warn!("could not move the unreadable settings aside: {e}");
            None
        }
    };
    tracing::warn!("settings file unreadable, starting from defaults: {reason}");
    Loaded::Unreadable { reason, kept }
}

fn aside_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bad");
    path.with_file_name(name)
}

/// Deletes temporary files an interrupted write of another run left beside `path`.
fn sweep_stale_temps(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > STALE_TEMP);
        if file.starts_with(&prefix) && file.ends_with(".tmp") && old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// How far a write got; a test stops it part-way to stand for a crash there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    /// The temporary file exists and is empty.
    Created,
    /// All the bytes are in it, not yet flushed to disk.
    Written,
    /// Flushed to disk.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "a test stops a write here to stand for a crash")
    )]
    Synced,
    /// Renamed over the real file, the folder not yet flushed.
    Renamed,
    /// Done.
    Finished,
}

/// Replaces the file at `path` with `bytes`: all of it or none of it.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_until(path, bytes, Stage::Finished)
}

fn write_until(path: &Path, bytes: &[u8], until: Stage) -> io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let written = write_temp(&temp, bytes, until).and_then(|()| {
        if until < Stage::Renamed {
            return Ok(());
        }
        fs::rename(&temp, path)?;
        if until >= Stage::Finished {
            sync_parent(path)?;
        }
        Ok(())
    });
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// Creates the temporary file, fills it and flushes it, as far as `until` says.
fn write_temp(temp: &Path, bytes: &[u8], until: Stage) -> io::Result<()> {
    let _ = fs::remove_file(temp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(temp)?;
    if until <= Stage::Created {
        return Ok(());
    }
    file.write_all(bytes)?;
    if until <= Stage::Written {
        return Ok(());
    }
    file.sync_all()
}

/// Makes the rename itself durable (Unix; Windows has no way to flush a folder).
fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Writes the settings on a thread of its own.
pub struct Writer {
    send: Option<Sender<Vec<u8>>>,
    errors: Receiver<String>,
    report: Sender<String>,
    written: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    /// Starts the thread; it writes to `path` (its folder must exist).
    pub fn spawn(path: PathBuf) -> Self {
        let (send, jobs) = unbounded::<Vec<u8>>();
        let (report, errors) = unbounded();
        let written = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("settings-writer".to_owned())
            .spawn({
                let (report, written) = (report.clone(), Arc::clone(&written));
                move || write_loop(&path, &jobs, &report, &written)
            })
            .map_err(|e| tracing::warn!("could not start the settings writer: {e}"))
            .ok();
        Self {
            send: Some(send),
            errors,
            report,
            written,
            thread,
        }
    }

    /// Queues `settings`. Only the serialisation happens here; the disk work is the thread's.
    pub fn save(&self, settings: &Settings) {
        match encode(settings) {
            Ok(bytes) => {
                if let Some(send) = &self.send {
                    let _ = send.send(bytes);
                }
            }
            Err(e) => {
                let _ = self.report.send(format!("they could not be encoded ({e})"));
            }
        }
    }

    /// Whether a write has finished since the start.
    pub fn has_written(&self) -> bool {
        self.written.load(Ordering::Acquire)
    }

    /// The reasons writes failed since the last call, each once until a write succeeds in between.
    pub fn take_errors(&self) -> Vec<String> {
        self.errors.try_iter().collect()
    }

    /// Writes what is queued and stops the thread.
    pub fn finish(&mut self) {
        self.send = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.finish();
    }
}

fn write_loop(
    path: &Path,
    jobs: &Receiver<Vec<u8>>,
    report: &Sender<String>,
    written: &AtomicBool,
) {
    let mut last_written: Option<Vec<u8>> = None;
    let mut last_error: Option<String> = None;
    while let Ok(mut bytes) = jobs.recv() {
        // Only the newest text matters.
        while let Ok(newer) = jobs.try_recv() {
            bytes = newer;
        }
        if last_written.as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        match write_atomic(path, &bytes) {
            Ok(()) => {
                last_written = Some(bytes);
                last_error = None;
                written.store(true, Ordering::Release);
            }
            Err(e) => {
                let why = format!("{} could not be written ({e})", path.display());
                tracing::warn!("{why}");
                if last_error.as_deref() != Some(why.as_str()) {
                    let _ = report.send(why.clone());
                }
                last_error = Some(why);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    fn sample() -> Settings {
        Settings {
            zoom: 1.2,
            ..Settings::default()
        }
    }

    fn loaded(path: &Path) -> Settings {
        match load(path) {
            Loaded::Settings(s) => *s,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn what_is_written_is_read_back() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = store(dir.path());
        assert!(matches!(load(&path), Loaded::Missing));
        write_atomic(&path, &encode(&sample()).expect("encode")).expect("write");
        assert!((loaded(&path).zoom - 1.2).abs() < f32::EPSILON);
        let text = fs::read_to_string(&path).expect("text");
        assert!(text.contains("\"schema\": 1"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn a_write_cut_short_at_any_point_leaves_the_previous_file_whole() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = store(dir.path());
        write_atomic(&path, &encode(&sample()).expect("encode")).expect("first");
        let newer = Settings {
            zoom: 1.4,
            ..Settings::default()
        };
        let newer = encode(&newer).expect("encode");
        for stage in [Stage::Created, Stage::Written, Stage::Synced] {
            write_until(&path, &newer, stage).expect("cut short");
            assert!((loaded(&path).zoom - 1.2).abs() < f32::EPSILON, "{stage:?}");
        }
        // The cut-short writes left a temporary file; the next complete write replaces it, and
        // nothing but the settings and the unfinished temporary is in the folder.
        write_atomic(&path, &newer).expect("complete");
        assert!((loaded(&path).zoom - 1.4).abs() < f32::EPSILON);
        let names: Vec<_> = fs::read_dir(dir.path())
            .expect("dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["settings.json"]);
    }

    #[test]
    fn a_write_that_fails_leaves_the_previous_file_and_no_temporary() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("gone/settings.json");
        assert!(write_atomic(&path, b"x").is_err());
        let path = store(dir.path());
        write_atomic(&path, &encode(&sample()).expect("encode")).expect("first");
        // A directory where the file belongs makes the final rename fail.
        let blocked = dir.path().join("blocked.json");
        fs::create_dir(&blocked).expect("dir");
        fs::write(blocked.join("keep"), "x").expect("file");
        assert!(write_atomic(&blocked, b"{}").is_err());
        assert!(blocked.join("keep").exists());
        assert!((loaded(&path).zoom - 1.2).abs() < f32::EPSILON);
        let temps = fs::read_dir(dir.path())
            .expect("dir")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(temps, 0);
    }

    #[test]
    fn an_unreadable_file_is_kept_aside_and_never_overwritten() {
        for (case, content) in [
            ("empty", Vec::new()),
            ("cut short", b"{\"schema\": 1, \"settings\": {\"zo".to_vec()),
            ("not json", b"\x00\xff nonsense".to_vec()),
            ("no schema", b"{\"settings\": {}}".to_vec()),
            (
                "wrong kind",
                b"{\"schema\": 1, \"settings\": {\"zoom\": \"big\"}}".to_vec(),
            ),
            ("newer", b"{\"schema\": 99, \"settings\": {}}".to_vec()),
        ] {
            let dir = tempfile::tempdir().expect("tmp");
            let path = store(dir.path());
            fs::write(&path, &content).expect("write");
            let Loaded::Unreadable { reason, kept } = load(&path) else {
                panic!("{case} should be unreadable");
            };
            assert!(!reason.is_empty(), "{case}");
            let kept = kept.expect("kept aside");
            assert_eq!(kept, dir.path().join("settings.json.bad"), "{case}");
            assert_eq!(fs::read(&kept).expect("kept"), content, "{case}");
            assert!(!path.exists(), "{case}");
            // The next start finds no file and writes a fresh one.
            assert!(matches!(load(&path), Loaded::Missing), "{case}");
        }
    }

    #[test]
    fn a_newer_unreadable_file_replaces_an_older_one_kept_aside() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = store(dir.path());
        fs::write(dir.path().join("settings.json.bad"), "first").expect("old");
        fs::write(&path, "second").expect("write");
        let Loaded::Unreadable { kept, .. } = load(&path) else {
            panic!("unreadable");
        };
        assert_eq!(
            fs::read_to_string(kept.expect("kept")).expect("read"),
            "second"
        );
    }

    #[test]
    fn a_file_from_an_older_build_loads_with_the_new_fields_at_their_defaults() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = store(dir.path());
        fs::write(&path, "{\"schema\": 1, \"settings\": {\"zoom\": 1.1}}").expect("write");
        let settings = loaded(&path);
        assert!((settings.zoom - 1.1).abs() < f32::EPSILON);
        assert_eq!(settings.theme, Settings::default().theme);
    }

    #[test]
    fn stale_temporaries_are_swept_but_fresh_ones_stay() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = store(dir.path());
        let stale = dir.path().join("settings.json.111.tmp");
        let fresh = dir.path().join("settings.json.222.tmp");
        fs::write(&stale, "x").expect("stale");
        fs::write(&fresh, "x").expect("fresh");
        let old = SystemTime::now() - Duration::from_hours(2);
        fs::File::options()
            .write(true)
            .open(&stale)
            .expect("open")
            .set_modified(old)
            .expect("mtime");
        assert!(matches!(load(&path), Loaded::Missing));
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn the_writer_writes_off_the_calling_thread_and_finishes_what_is_queued() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = store(dir.path());
        let mut writer = Writer::spawn(path.clone());
        for zoom in [1.0_f32, 1.1, 1.2, 1.3] {
            writer.save(&Settings {
                zoom,
                ..Settings::default()
            });
        }
        writer.finish();
        assert!(writer.has_written());
        assert!((loaded(&path).zoom - 1.3).abs() < f32::EPSILON);
        assert_eq!(writer.take_errors(), Vec::<String>::new());
    }

    #[test]
    fn a_failing_writer_says_so_once() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut writer = Writer::spawn(dir.path().join("missing/settings.json"));
        writer.save(&sample());
        writer.finish();
        assert!(!writer.has_written());
        let errors = writer.take_errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("could not be written"), "{errors:?}");
    }
}
