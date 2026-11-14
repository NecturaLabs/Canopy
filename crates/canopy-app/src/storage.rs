//! Where Canopy keeps its files, per operating system (ADR 0016).
//!
//! Four folders, each holding the marker file [`MARKER`] once Canopy has claimed it, which is what
//! the installers' `--purge` looks for in a folder with a generic name:
//!
//! | | config | data | state | cache |
//! |---|---|---|---|---|
//! | Linux | `$XDG_CONFIG_HOME/canopy` | `$XDG_DATA_HOME/canopy` | `$XDG_STATE_HOME/canopy` | `$XDG_CACHE_HOME/canopy` |
//! | macOS | `~/Library/Application Support/io.necturalabs.Canopy` (config and data) | | `~/Library/Logs/Canopy` | `~/Library/Caches/io.necturalabs.Canopy` |
//! | Windows | `%APPDATA%\NecturaLabs\Canopy` (config and data) | | `%LOCALAPPDATA%\NecturaLabs\Canopy` (state and cache) | |
//!
//! Config holds the settings (`settings.json`) and eframe's window state (`app.ron`), data holds
//! `tokens.json`, state holds `logs/` and `crashes/` (on macOS the logs sit in the folder itself,
//! where Console looks), and cache holds `avatars/`.
//!
//! Builds before this layout kept everything in two folders that `eframe::storage_dir` names (the
//! `canopy` folder for tokens, logs, crashes and pictures, the window's app id for `app.ron`).
//! [`migrate`] moves their contents once, at start, a file at a time: a file is never overwritten,
//! never partly present at its new place, and removed from the old place only after it arrived,
//! so a run cut short or run twice finishes the job.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The file that tells an installer a folder with a generic name is Canopy's to delete.
pub const MARKER: &str = ".canopy-data";

/// The app id eframe names the settings folder after.
const APP_ID: &str = "io.necturalabs.Canopy";

/// Names Canopy has itself put in its folders; a folder holding only these is Canopy's even
/// without the marker (the previous layout's `canopy` folder is one).
const OWN_FILES: &[&str] = &[
    "tokens.json",
    "settings.json",
    "app.ron",
    "logs",
    "crashes",
    "avatars",
    MARKER,
];

/// The operating systems with a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Mac,
    Windows,
}

impl Os {
    /// The operating system this build runs on.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Linux
        }
    }
}

/// The environment the folders come from, so a test can give one of its own.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub xdg_config: Option<PathBuf>,
    pub xdg_data: Option<PathBuf>,
    pub xdg_state: Option<PathBuf>,
    pub xdg_cache: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
}

impl Env {
    /// This process's environment. A relative path is ignored, as the XDG specification says.
    pub fn from_process() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Self {
            home: std::env::home_dir().filter(|p| p.is_absolute()),
            xdg_config: var("XDG_CONFIG_HOME"),
            xdg_data: var("XDG_DATA_HOME"),
            xdg_state: var("XDG_STATE_HOME"),
            xdg_cache: var("XDG_CACHE_HOME"),
            appdata: var("APPDATA"),
            local_appdata: var("LOCALAPPDATA"),
        }
    }

    fn xdg(&self, set: Option<&PathBuf>, fallback: &[&str]) -> Option<PathBuf> {
        set.cloned().or_else(|| {
            self.home.as_ref().map(|home| {
                fallback
                    .iter()
                    .fold(home.clone(), |path, part| path.join(part))
            })
        })
    }
}

/// Canopy's folders and the places inside them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub crashes: PathBuf,
    pub avatars: PathBuf,
}

impl Layout {
    /// The folders for `os` under `env`; `None` when the environment names no home.
    pub fn resolve(os: Os, env: &Env) -> Option<Self> {
        let (config, data, state, cache) = match os {
            Os::Linux => (
                env.xdg(env.xdg_config.as_ref(), &[".config"])?
                    .join("canopy"),
                env.xdg(env.xdg_data.as_ref(), &[".local", "share"])?
                    .join("canopy"),
                env.xdg(env.xdg_state.as_ref(), &[".local", "state"])?
                    .join("canopy"),
                env.xdg(env.xdg_cache.as_ref(), &[".cache"])?.join("canopy"),
            ),
            Os::Mac => {
                let library = env.home.as_ref()?.join("Library");
                let support = library.join("Application Support").join(APP_ID);
                (
                    support.clone(),
                    support,
                    library.join("Logs").join("Canopy"),
                    library.join("Caches").join(APP_ID),
                )
            }
            Os::Windows => {
                let roaming = env.appdata.as_ref()?.join("NecturaLabs").join("Canopy");
                let local = env
                    .local_appdata
                    .as_ref()?
                    .join("NecturaLabs")
                    .join("Canopy");
                (roaming.clone(), roaming, local.clone(), local)
            }
        };
        let (logs, crashes) = match os {
            // Console shows `~/Library/Logs/<name>/` as it is.
            Os::Mac => (state.clone(), state.join("crashes")),
            Os::Linux | Os::Windows => (state.join("logs"), state.join("crashes")),
        };
        let avatars = cache.join("avatars");
        Some(Self {
            config,
            data,
            state,
            cache,
            logs,
            crashes,
            avatars,
        })
    }

    /// Canopy's settings.
    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// eframe's own state: the window's place and size, and what egui remembers.
    pub fn window_file(&self) -> PathBuf {
        self.config.join("app.ron")
    }

    /// Saved hosting tokens.
    pub fn tokens_file(&self) -> PathBuf {
        self.data.join("tokens.json")
    }

    /// The distinct folders, config first.
    pub fn folders(&self) -> Vec<&Path> {
        let mut folders: Vec<&Path> = Vec::new();
        for folder in [&self.config, &self.data, &self.state, &self.cache] {
            if !folders.contains(&folder.as_path()) {
                folders.push(folder);
            }
        }
        folders
    }
}

/// The two folders earlier builds used, which [`migrate`] empties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legacy {
    /// `eframe::storage_dir("canopy")`: tokens, logs, crashes and pictures.
    pub data: PathBuf,
    /// `eframe::storage_dir("io.necturalabs.Canopy")`: `app.ron`.
    pub settings: PathBuf,
}

impl Legacy {
    /// What `eframe::storage_dir` gave for `os` under `env`.
    pub fn resolve(os: Os, env: &Env) -> Option<Self> {
        let at = |base: PathBuf, name: &str| match os {
            Os::Linux => base.join(name.to_lowercase()),
            Os::Mac => base.join(name),
            Os::Windows => base.join(name).join("data"),
        };
        let base = match os {
            Os::Linux => env.xdg(env.xdg_data.as_ref(), &[".local", "share"])?,
            Os::Mac => env
                .home
                .as_ref()?
                .join("Library")
                .join("Application Support"),
            Os::Windows => env.appdata.clone()?,
        };
        Some(Self {
            data: at(base.clone(), "canopy"),
            settings: at(base, APP_ID),
        })
    }
}

static LAYOUT: OnceLock<Option<Layout>> = OnceLock::new();

/// This process's folders (nothing is created by asking), or `None` when the platform has no home.
pub fn layout() -> Option<&'static Layout> {
    LAYOUT
        .get_or_init(|| Layout::resolve(Os::current(), &Env::from_process()))
        .as_ref()
}

/// What [`migrate`] did, for the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Files that moved to their new place.
    pub moved: usize,
    /// Files left where they were because something already sits at their new place, or because
    /// they are not plain files and folders (a link).
    pub kept: usize,
    /// What could not be moved, with why. The old copy is still there, so the next start tries
    /// again.
    pub failed: Vec<(PathBuf, String)>,
}

/// What start-up did with the folders, kept for the log line [`log_setup`] writes once the
/// logger exists.
#[derive(Debug)]
pub struct Setup {
    pub migration: Migration,
    pub prepared: io::Result<()>,
}

static SETUP: OnceLock<Setup> = OnceLock::new();

/// Moves the previous layout's files into the new folders and creates and claims the folders.
/// Called once, first thing in `main`, before anything reads or writes a file.
pub fn prepare() {
    let Some(layout) = layout() else {
        return;
    };
    let env = Env::from_process();
    let migration = Legacy::resolve(Os::current(), &env)
        .map(|legacy| migrate(&legacy, layout))
        .unwrap_or_default();
    let prepared = prepare_folders(layout);
    let _ = SETUP.set(Setup {
        migration,
        prepared,
    });
}

/// Writes what [`prepare`] did to the log. Paths are Canopy's own folders, never a secret.
pub fn log_setup() {
    let Some(setup) = SETUP.get() else {
        return;
    };
    let migration = &setup.migration;
    if migration.moved > 0 {
        tracing::info!(
            moved = migration.moved,
            kept = migration.kept,
            "moved Canopy's files to the new folders"
        );
    }
    for (path, why) in &migration.failed {
        tracing::warn!("could not move {}: {why}", path.display());
    }
    if let Err(e) = &setup.prepared {
        tracing::warn!("could not prepare Canopy's folders: {e}");
    }
}

/// Creates every folder (owner only on Unix) and claims it with [`MARKER`].
pub fn prepare_folders(layout: &Layout) -> io::Result<()> {
    let mut result = Ok(());
    for folder in layout.folders() {
        if let Err(e) = create_private_dir(folder).and_then(|()| claim(folder)) {
            result = result.and(Err(e));
        }
    }
    result
}

/// Creates `dir` and its missing parents, readable by the owner only on Unix.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Puts [`MARKER`] in `dir` when the folder is empty or holds only Canopy's own files, and makes
/// it owner-only on Unix. A folder with someone else's files in it is left unmarked, so `--purge`
/// leaves it too.
fn claim(dir: &Path) -> io::Result<()> {
    if dir.join(MARKER).exists() {
        return Ok(());
    }
    if !holds_only_canopy_files(dir)? {
        tracing::warn!(
            "{} holds files that are not Canopy's; it is used but not marked",
            dir.display()
        );
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    crate::hosting::write_private(&dir.join(MARKER), b"")
}

fn holds_only_canopy_files(dir: &Path) -> io::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        let own = OWN_FILES.contains(&name.as_ref())
            || name.starts_with("settings.json.")
            || name.starts_with("app.ron.");
        if !own {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Moves what the previous layout kept into `layout`'s places. Safe to repeat and to interrupt.
pub fn migrate(legacy: &Legacy, layout: &Layout) -> Migration {
    let mut report = Migration::default();
    let items = [
        (legacy.settings.join("app.ron"), layout.window_file()),
        (legacy.data.join("tokens.json"), layout.tokens_file()),
        (legacy.data.join("logs"), layout.logs.clone()),
        (legacy.data.join("crashes"), layout.crashes.clone()),
        (legacy.data.join("avatars"), layout.avatars.clone()),
    ];
    for (from, to) in &items {
        move_item(from, to, &mut report);
    }
    // A folder the move emptied goes; one with anything else in it stays. On Windows the old
    // folders sit inside `<name>\data`.
    for old in [&legacy.settings, &legacy.data] {
        if layout.folders().contains(&old.as_path()) {
            continue;
        }
        if remove_if_empty(old)
            && old.file_name().is_some_and(|n| n == "data")
            && let Some(parent) = old.parent()
        {
            remove_if_empty(parent);
        }
    }
    report
}

fn remove_if_empty(dir: &Path) -> bool {
    fs::remove_dir(dir).is_ok()
}

fn move_item(from: &Path, to: &Path, report: &mut Migration) {
    if from == to {
        return;
    }
    let Ok(meta) = fs::symlink_metadata(from) else {
        return;
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        report.kept += 1;
    } else if kind.is_dir() {
        move_dir(from, to, report);
    } else if kind.is_file() {
        move_file(from, to, report);
    } else {
        report.kept += 1;
    }
}

fn move_dir(from: &Path, to: &Path, report: &mut Migration) {
    if let Err(e) = create_private_dir(to) {
        report.failed.push((from.to_path_buf(), e.to_string()));
        return;
    }
    let entries = match fs::read_dir(from) {
        Ok(entries) => entries,
        Err(e) => {
            report.failed.push((from.to_path_buf(), e.to_string()));
            return;
        }
    };
    for entry in entries.flatten() {
        move_item(&entry.path(), &to.join(entry.file_name()), report);
    }
    remove_if_empty(from);
}

fn move_file(from: &Path, to: &Path, report: &mut Migration) {
    if fs::symlink_metadata(to).is_ok() {
        report.kept += 1;
        return;
    }
    if let Some(parent) = to.parent()
        && let Err(e) = create_private_dir(parent)
    {
        report.failed.push((from.to_path_buf(), e.to_string()));
        return;
    }
    // A rename never leaves a half-written file. Across file systems it fails, and the copy goes
    // to a temporary name first for the same reason.
    let moved = fs::rename(from, to).or_else(|_| {
        let temp = to.with_file_name(format!(
            "{}.moving",
            to.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ));
        fs::copy(from, &temp)
            .and_then(|_| fs::rename(&temp, to))
            .and_then(|()| fs::remove_file(from))
            .inspect_err(|_| {
                let _ = fs::remove_file(&temp);
            })
    });
    match moved {
        Ok(()) => report.moved += 1,
        Err(e) => report.failed.push((from.to_path_buf(), e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(root: &Path) -> Env {
        Env {
            home: Some(root.join("home")),
            appdata: Some(root.join("appdata")),
            local_appdata: Some(root.join("local")),
            ..Env::default()
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, text).expect("write");
    }

    #[test]
    fn linux_follows_the_xdg_variables_and_ignores_relative_ones() {
        let root = Path::new("/h");
        let mut e = Env {
            home: Some(root.to_path_buf()),
            ..Env::default()
        };
        let l = Layout::resolve(Os::Linux, &e).expect("layout");
        assert_eq!(l.config, root.join(".config/canopy"));
        assert_eq!(l.data, root.join(".local/share/canopy"));
        assert_eq!(l.state, root.join(".local/state/canopy"));
        assert_eq!(l.cache, root.join(".cache/canopy"));
        assert_eq!(l.logs, root.join(".local/state/canopy/logs"));
        assert_eq!(l.crashes, root.join(".local/state/canopy/crashes"));
        assert_eq!(l.avatars, root.join(".cache/canopy/avatars"));
        e.xdg_config = Some("/x/config".into());
        e.xdg_state = Some("/x/state".into());
        let l = Layout::resolve(Os::Linux, &e).expect("layout");
        assert_eq!(l.config, Path::new("/x/config/canopy"));
        assert_eq!(
            l.settings_file(),
            Path::new("/x/config/canopy/settings.json")
        );
        assert_eq!(l.logs, Path::new("/x/state/canopy/logs"));
        assert_eq!(l.data, root.join(".local/share/canopy"));
        assert!(Layout::resolve(Os::Linux, &Env::default()).is_none());
    }

    #[test]
    fn macos_and_windows_use_their_own_folders() {
        let e = Env {
            home: Some("/Users/u".into()),
            appdata: Some("/roaming".into()),
            local_appdata: Some("/local".into()),
            ..Env::default()
        };
        let mac = Layout::resolve(Os::Mac, &e).expect("mac");
        let support = Path::new("/Users/u/Library/Application Support/io.necturalabs.Canopy");
        assert_eq!(mac.config, support);
        assert_eq!(mac.data, support);
        assert_eq!(mac.logs, Path::new("/Users/u/Library/Logs/Canopy"));
        assert_eq!(
            mac.crashes,
            Path::new("/Users/u/Library/Logs/Canopy/crashes")
        );
        assert_eq!(
            mac.avatars,
            Path::new("/Users/u/Library/Caches/io.necturalabs.Canopy/avatars")
        );
        assert_eq!(mac.folders().len(), 3);
        let win = Layout::resolve(Os::Windows, &e).expect("windows");
        assert_eq!(win.config, win.data);
        assert_eq!(win.state, win.cache);
        assert_eq!(win.folders().len(), 2);
        assert_eq!(win.config, Path::new("/roaming/NecturaLabs/Canopy"));
        assert_eq!(win.state, Path::new("/local/NecturaLabs/Canopy"));
        assert_eq!(win.logs, Path::new("/local/NecturaLabs/Canopy/logs"));
    }

    #[test]
    fn the_previous_folders_match_what_eframe_gave() {
        let e = Env {
            home: Some("/home/u".into()),
            appdata: Some("/appdata".into()),
            ..Env::default()
        };
        let l = Legacy::resolve(Os::Linux, &e).expect("linux");
        assert_eq!(l.data, Path::new("/home/u/.local/share/canopy"));
        assert_eq!(
            l.settings,
            Path::new("/home/u/.local/share/io.necturalabs.canopy")
        );
        let l = Legacy::resolve(Os::Mac, &e).expect("mac");
        assert_eq!(
            l.settings,
            Path::new("/home/u/Library/Application Support/io.necturalabs.Canopy")
        );
        let l = Legacy::resolve(Os::Windows, &e).expect("windows");
        assert_eq!(l.data, Path::new("/appdata/canopy/data"));
        assert_eq!(l.settings, Path::new("/appdata/io.necturalabs.Canopy/data"));
        // eframe's own answer, where the environment decides it alone.
        #[cfg(not(windows))]
        let here = Env::from_process();
        #[cfg(not(windows))]
        if let Some(l) = Legacy::resolve(Os::current(), &here) {
            assert_eq!(Some(l.data), eframe::storage_dir("canopy"));
            assert_eq!(Some(l.settings), eframe::storage_dir(APP_ID));
        }
    }

    #[test]
    fn folders_are_created_private_and_claimed() {
        let root = tempfile::tempdir().expect("tmp");
        let layout = Layout::resolve(Os::Linux, &env(root.path())).expect("layout");
        prepare_folders(&layout).expect("prepare");
        prepare_folders(&layout).expect("twice");
        for folder in [&layout.config, &layout.data, &layout.state, &layout.cache] {
            assert!(folder.join(MARKER).is_file(), "{}", folder.display());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = |p: &Path| fs::metadata(p).expect("meta").permissions().mode() & 0o777;
                assert_eq!(mode(folder), 0o700, "{}", folder.display());
                assert_eq!(mode(&folder.join(MARKER)), 0o600);
            }
        }
    }

    #[test]
    fn a_folder_with_someone_elses_files_is_not_claimed() {
        let root = tempfile::tempdir().expect("tmp");
        let layout = Layout::resolve(Os::Linux, &env(root.path())).expect("layout");
        write(&layout.config.join("their-config.toml"), "x");
        prepare_folders(&layout).expect("prepare");
        assert!(!layout.config.join(MARKER).exists());
        assert!(layout.data.join(MARKER).is_file());
    }

    fn old_folders(root: &Path) -> (Legacy, Layout) {
        let e = env(root);
        (
            Legacy::resolve(Os::Linux, &e).expect("legacy"),
            Layout::resolve(Os::Linux, &e).expect("layout"),
        )
    }

    fn populate(legacy: &Legacy) {
        write(
            &legacy.settings.join("app.ron"),
            "{\"canopy-settings-v1\": \"x\"}",
        );
        write(&legacy.data.join("tokens.json"), "{\"tokens\":{}}");
        write(&legacy.data.join("logs/canopy-1.jsonl"), "log 1");
        write(&legacy.data.join("logs/canopy-2.jsonl"), "log 2");
        write(&legacy.data.join("crashes/crash-1.txt"), "crash");
        write(&legacy.data.join("avatars/ab/cd.json"), "picture");
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("read")
    }

    #[test]
    fn a_populated_old_layout_moves_whole_and_the_old_folders_go() {
        let root = tempfile::tempdir().expect("tmp");
        let (legacy, layout) = old_folders(root.path());
        populate(&legacy);
        let report = migrate(&legacy, &layout);
        assert_eq!(report.failed, Vec::new());
        // settings, two logs, a crash report and a picture; tokens.json is already where it
        // belongs on Linux, where the data folder is the old folder.
        assert_eq!(report.moved, 5);
        assert_eq!(
            read(&layout.window_file()),
            "{\"canopy-settings-v1\": \"x\"}"
        );
        // tokens.json stays where it was on Linux: the data folder is the old folder.
        assert_eq!(read(&layout.tokens_file()), "{\"tokens\":{}}");
        assert_eq!(read(&layout.logs.join("canopy-2.jsonl")), "log 2");
        assert_eq!(read(&layout.crashes.join("crash-1.txt")), "crash");
        assert_eq!(read(&layout.avatars.join("ab/cd.json")), "picture");
        assert!(
            !legacy.settings.exists(),
            "the emptied settings folder goes"
        );
        assert!(!legacy.data.join("logs").exists());
        assert!(!legacy.data.join("avatars").exists());
        prepare_folders(&layout).expect("prepare");
        assert!(
            layout.data.join(MARKER).is_file(),
            "the old folder is claimed"
        );
    }

    #[test]
    fn migrating_twice_changes_nothing_the_second_time() {
        let root = tempfile::tempdir().expect("tmp");
        let (legacy, layout) = old_folders(root.path());
        populate(&legacy);
        migrate(&legacy, &layout);
        let again = migrate(&legacy, &layout);
        assert_eq!(again, Migration::default());
        assert_eq!(read(&layout.logs.join("canopy-1.jsonl")), "log 1");
    }

    #[test]
    fn a_run_cut_short_is_finished_by_the_next_and_nothing_is_overwritten() {
        let root = tempfile::tempdir().expect("tmp");
        let (legacy, layout) = old_folders(root.path());
        populate(&legacy);
        // The first run got through one log before it stopped; a log written since by the new
        // build has a name the old folder also has.
        write(&layout.logs.join("canopy-1.jsonl"), "log 1");
        fs::remove_file(legacy.data.join("logs/canopy-1.jsonl")).expect("moved already");
        write(&layout.logs.join("canopy-2.jsonl"), "newer log 2");
        // A copy the interrupted run left behind under its temporary name.
        write(&layout.avatars.join("ab/cd.json.moving"), "half");
        let report = migrate(&legacy, &layout);
        assert_eq!(report.failed, Vec::new());
        assert_eq!(read(&layout.logs.join("canopy-1.jsonl")), "log 1");
        assert_eq!(read(&layout.logs.join("canopy-2.jsonl")), "newer log 2");
        assert_eq!(
            read(&legacy.data.join("logs/canopy-2.jsonl")),
            "log 2",
            "the old copy of a file that has a newer namesake stays"
        );
        assert_eq!(report.kept, 1);
        assert_eq!(read(&layout.avatars.join("ab/cd.json")), "picture");
        assert_eq!(read(&layout.crashes.join("crash-1.txt")), "crash");
    }

    #[test]
    fn files_the_layout_does_not_know_stay_in_the_old_folder() {
        let root = tempfile::tempdir().expect("tmp");
        let (legacy, layout) = old_folders(root.path());
        populate(&legacy);
        write(&legacy.data.join("notes.txt"), "not ours to move");
        migrate(&legacy, &layout);
        assert_eq!(read(&legacy.data.join("notes.txt")), "not ours to move");
        prepare_folders(&layout).expect("prepare");
        assert!(
            !layout.data.join(MARKER).exists(),
            "a folder with a stranger's file is not claimed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_old_folder_is_left_alone() {
        let root = tempfile::tempdir().expect("tmp");
        let (legacy, layout) = old_folders(root.path());
        populate(&legacy);
        let elsewhere = root.path().join("elsewhere");
        write(&elsewhere.join("big.bin"), "data");
        fs::remove_dir_all(legacy.data.join("avatars")).expect("remove");
        std::os::unix::fs::symlink(&elsewhere, legacy.data.join("avatars")).expect("link");
        let report = migrate(&legacy, &layout);
        assert_eq!(report.kept, 1);
        assert!(!layout.avatars.exists());
        assert_eq!(read(&elsewhere.join("big.bin")), "data");
    }

    #[test]
    fn windows_old_folders_go_with_their_parent() {
        let root = tempfile::tempdir().expect("tmp");
        let e = env(root.path());
        let legacy = Legacy::resolve(Os::Windows, &e).expect("legacy");
        let layout = Layout::resolve(Os::Windows, &e).expect("layout");
        populate(&legacy);
        let report = migrate(&legacy, &layout);
        assert_eq!(report.failed, Vec::new());
        assert_eq!(read(&layout.tokens_file()), "{\"tokens\":{}}");
        assert_eq!(read(&layout.logs.join("canopy-1.jsonl")), "log 1");
        assert!(!root.path().join("appdata/canopy").exists());
        assert!(!root.path().join("appdata/io.necturalabs.Canopy").exists());
    }

    #[test]
    fn macos_keeps_settings_and_tokens_in_the_folder_they_already_have() {
        let root = tempfile::tempdir().expect("tmp");
        let e = env(root.path());
        let legacy = Legacy::resolve(Os::Mac, &e).expect("legacy");
        let layout = Layout::resolve(Os::Mac, &e).expect("layout");
        populate(&legacy);
        let report = migrate(&legacy, &layout);
        assert_eq!(report.failed, Vec::new());
        assert_eq!(
            read(&layout.window_file()),
            "{\"canopy-settings-v1\": \"x\"}"
        );
        assert_eq!(read(&layout.tokens_file()), "{\"tokens\":{}}");
        assert_eq!(read(&layout.logs.join("canopy-1.jsonl")), "log 1");
    }
}
