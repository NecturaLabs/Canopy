//! What a killed or failed Canopy left behind, found and cleaned up (issue #9).
//!
//! Two rules decide who removes what. Files and folders in Canopy's own places (the data folder,
//! the avatar cache, the system's temporary folder, the install folder) are removed at the next
//! start. Anything in a folder that belongs to the user (a working tree) is only reported, with
//! its path, and the user decides. Git's own lock files and half-finished states are never
//! touched: git's message names them.
//!
//! Every removal needs proof that the file is abandoned, not in use by another running Canopy:
//! the process id in its name is gone (`canopy_core::leftovers`), or it has not changed for a
//! minute where the name carries no process id.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, bounded};

/// How long a staging file without a process id in its name must have been untouched before it is
/// abandoned. Writing one takes milliseconds.
const SETTLE: Duration = Duration::from_mins(1);

/// How many leftover files a notification lists before "and N more".
const SHOWN: usize = 5;

/// What a start's sweep removed, for the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub files: Vec<PathBuf>,
}

/// Whether `meta` is a plain file (not a link) nobody has written to for [`SETTLE`].
fn settled_file(meta: &std::fs::Metadata, now: SystemTime) -> bool {
    meta.is_file()
        && meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= SETTLE)
}

/// Removes the settled plain files in `dir` whose name `matches`, and returns them.
fn remove_settled(dir: &Path, now: SystemTime, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !matches(&name) {
            continue;
        }
        let path = entry.path();
        // `symlink_metadata`: a link is never a plain file here, so it is never followed.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if settled_file(&meta, now) {
            match std::fs::remove_file(&path) {
                Ok(()) => removed.push(path),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "could not remove a leftover staging file");
                }
            }
        }
    }
    removed
}

/// Canopy's data folder: the saved-token staging file (it holds a token, so it matters most) and
/// the avatar cache's.
fn sweep_data_dir(data: &Path, now: SystemTime) -> Vec<PathBuf> {
    let mut removed = remove_settled(data, now, |name| name == "tokens.json.tmp");
    removed.extend(remove_settled(&data.join("avatars"), now, |name| {
        Path::new(name).extension().is_some_and(|e| e == "new")
    }));
    removed
}

/// Cleans up Canopy's own places, once at start. Blocking: never on the UI thread.
pub fn sweep_at_start() -> Swept {
    let mut swept = Swept::default();
    if let Some(data) = eframe::storage_dir("canopy") {
        swept.files.extend(sweep_data_dir(&data, SystemTime::now()));
    }
    swept.files.extend(canopy_core::leftovers::sweep_temp_dirs(
        &std::env::temp_dir(),
        &[
            canopy_core::leftovers::PATCH_STAGING_PREFIX,
            crate::selfupdate::UPDATE_STAGING_PREFIX,
        ],
    ));
    if let Err(e) = crate::selfupdate::sweep_interrupted_update() {
        tracing::warn!(error = %e, "could not clear what an interrupted update left");
    }
    if !swept.files.is_empty() {
        tracing::info!(
            removed = swept.files.len(),
            "removed staging files left by an earlier run"
        );
    }
    swept
}

/// Runs [`sweep_at_start`] on its own thread.
pub fn sweep_at_start_async() {
    let spawned = std::thread::Builder::new()
        .name("housekeeping".into())
        .spawn(|| {
            sweep_at_start();
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the start-up cleanup");
    }
}

/// Looks, on a new thread, for the staging files an interrupted save left in the working tree
/// `root`, and wakes the UI when the answer is in. It only reads.
pub fn scan_async(ctx: &egui::Context, root: PathBuf) -> Receiver<Vec<PathBuf>> {
    let (tx, rx) = bounded(1);
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("leftovers".into())
        .spawn(move || {
            let found = canopy_core::Repo::open(&root)
                .and_then(|repo| repo.abandoned_staging_files())
                .unwrap_or_else(|e| {
                    tracing::debug!(error = %e, "could not look for leftover staging files");
                    Vec::new()
                });
            let _ = tx.send(found);
            ctx.request_repaint();
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the leftover files scan");
    }
    rx
}

/// The notification's title and body for the leftover `files`: paths first, then what they are
/// and that Canopy leaves them.
pub fn leftover_notice(files: &[PathBuf]) -> (&'static str, String) {
    let mut lines: Vec<String> = files
        .iter()
        .take(SHOWN)
        .map(|f| f.display().to_string())
        .collect();
    if files.len() > SHOWN {
        lines.push(format!("and {} more", files.len() - SHOWN));
    }
    lines.push("Left when Canopy stopped while saving. Canopy does not delete them.".to_owned());
    ("Files left by an interrupted save", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aged(path: &Path, age: Duration) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(SystemTime::now() - age))
            .expect("set the age");
    }

    #[test]
    fn the_data_folders_staging_files_go_once_settled_and_nothing_else() {
        let data = tempfile::tempdir().expect("data");
        let avatars = data.path().join("avatars");
        std::fs::create_dir(&avatars).expect("avatars");
        let write = |path: PathBuf, age: Duration| {
            std::fs::write(&path, "x").expect("write");
            aged(&path, age);
            path
        };
        let old = Duration::from_mins(10);
        let stale_tokens = write(data.path().join("tokens.json.tmp"), old);
        let stale_avatar = write(avatars.join("abc.new"), old);
        let writing_now = write(avatars.join("def.new"), Duration::from_secs(1));
        let tokens = write(data.path().join("tokens.json"), old);
        let record = write(avatars.join("abc.json"), old);
        let settings = write(data.path().join("app.ron"), old);

        let mut removed = sweep_data_dir(data.path(), SystemTime::now());
        removed.sort();
        let mut expected = vec![stale_tokens.clone(), stale_avatar.clone()];
        expected.sort();
        assert_eq!(removed, expected);
        assert!(!stale_tokens.exists() && !stale_avatar.exists());
        for kept in [&writing_now, &tokens, &record, &settings] {
            assert!(kept.is_file(), "{} stays", kept.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_with_a_staging_name_is_left_alone() {
        let data = tempfile::tempdir().expect("data");
        let target = tempfile::tempdir().expect("target");
        let secret = target.path().join("secret");
        std::fs::write(&secret, "keep").expect("write");
        let link = data.path().join("tokens.json.tmp");
        std::os::unix::fs::symlink(&secret, &link).expect("link");
        assert_eq!(
            sweep_data_dir(data.path(), SystemTime::now() + SETTLE * 2),
            Vec::<PathBuf>::new()
        );
        link.symlink_metadata().expect("the link stays");
        assert_eq!(std::fs::read_to_string(&secret).expect("read"), "keep");
    }

    #[test]
    fn a_missing_data_folder_is_nothing_to_sweep() {
        let data = tempfile::tempdir().expect("data");
        assert_eq!(
            sweep_data_dir(&data.path().join("none"), SystemTime::now()),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn the_notice_names_the_files_and_says_they_are_kept() {
        let files: Vec<PathBuf> = (1..=7)
            .map(|n| PathBuf::from(format!("/r/.f{n}.tmp")))
            .collect();
        let (title, body) = leftover_notice(&files);
        assert_eq!(title, "Files left by an interrupted save");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            &lines[..5],
            [
                "/r/.f1.tmp",
                "/r/.f2.tmp",
                "/r/.f3.tmp",
                "/r/.f4.tmp",
                "/r/.f5.tmp"
            ]
        );
        assert_eq!(lines[5], "and 2 more");
        assert!(lines[6].contains("does not delete"));
        let (_, one) = leftover_notice(&files[..1]);
        assert_eq!(one.lines().count(), 2);
    }
}
