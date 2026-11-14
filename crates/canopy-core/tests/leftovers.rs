//! What failed, cancelled and killed operations leave in a repository and in the temporary
//! folder (issue #9), against throwaway repositories with the git CLI as the oracle.
//!
//! One test here exports patches, and the others never do, so what it asserts about the shared
//! temporary folder cannot be disturbed by a parallel test of this file.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use canopy_core::leftovers::sweep_temp_dirs;
use canopy_core::{Oid, Repo};

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with one commit of `a.txt`, a local identity and no signing.
fn init(p: &Path) {
    git(p, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Grace Hopper"),
        ("user.email", "grace@example.com"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(p, &["config", key, value]);
    }
    fs::write(p.join("a.txt"), "one\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: first"]);
}

/// A process id no process has: the staging name of a Canopy that was killed.
const GONE: u32 = u32::MAX;

fn staging(dir: &Path, name: &str, pid: u32) -> PathBuf {
    let path = dir.join(format!(".{name}.canopy-{pid}-0.tmp"));
    fs::write(&path, "half a write").expect("write");
    path
}

#[test]
fn a_killed_write_is_reported_and_never_deleted() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    fs::create_dir(dir.path().join("src")).expect("mkdir");
    let mine = staging(dir.path(), "a.txt", GONE);
    let nested = staging(&dir.path().join("src"), "lib.rs", GONE);
    // `*.tmp` is a common ignore rule: the leftover is still reported.
    fs::write(dir.path().join(".gitignore"), "*.tmp\n").expect("ignore");
    let repo = Repo::open(dir.path()).expect("open");
    let found = repo.abandoned_staging_files().expect("scan");
    assert_eq!(found, [mine.clone(), nested.clone()]);
    assert!(mine.is_file() && nested.is_file(), "nothing is deleted");
    assert_eq!(fs::read_to_string(&mine).expect("read"), "half a write");
}

#[test]
fn a_file_being_written_now_and_other_files_are_not_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    // This process is writing it; so is process 1 (always running on Linux).
    staging(dir.path(), "a.txt", std::process::id());
    if cfg!(target_os = "linux") {
        staging(dir.path(), "b.txt", 1);
    }
    // Names that only look similar, and a committed file whose name matches the pattern.
    fs::write(dir.path().join("notes.canopy-5-0.tmp"), "x").expect("write");
    fs::write(dir.path().join(".a.txt.canopy-x-0.tmp"), "x").expect("write");
    fs::write(dir.path().join(".b.canopy-7-0.bak"), "x").expect("write");
    let repo = Repo::open(dir.path()).expect("open");
    assert_eq!(
        repo.abandoned_staging_files().expect("scan"),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn a_clean_repository_reports_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let repo = Repo::open(dir.path()).expect("open");
    assert_eq!(
        repo.abandoned_staging_files().expect("scan"),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn swept_temp_folders_are_only_canopys_own_and_abandoned() {
    let temp = tempfile::tempdir().expect("tempdir");
    let make = |name: &str| {
        let dir = temp.path().join(name);
        fs::create_dir_all(dir.join("x")).expect("mkdir");
        fs::write(dir.join("x/file"), "data").expect("write");
        dir
    };
    let gone_patches = make(&format!("canopy-patches-{GONE}-0"));
    let gone_update = make(&format!("canopy-update.{GONE}-123-0"));
    let own = make(&format!("canopy-patches-{}-4", std::process::id()));
    let similar = make("canopy-patches-notes");
    let other = make(&format!("someone-elses-{GONE}-0"));
    let file = temp.path().join(format!("canopy-patches-{GONE}-9"));
    fs::write(&file, "a plain file").expect("write");
    let outside = tempfile::tempdir().expect("tempdir");
    #[cfg(unix)]
    let link = {
        let link = temp.path().join(format!("canopy-update.{GONE}-1-1"));
        std::os::unix::fs::symlink(outside.path(), &link).expect("link");
        link
    };
    #[cfg(target_os = "linux")]
    let running = make("canopy-patches-1-0");

    let mut removed = sweep_temp_dirs(temp.path(), &["canopy-patches-", "canopy-update."]);
    removed.sort();
    let mut expected = vec![gone_patches.clone(), gone_update.clone()];
    expected.sort();
    assert_eq!(removed, expected);
    assert!(!gone_patches.exists() && !gone_update.exists());
    assert!(own.is_dir() && similar.is_dir() && other.is_dir() && file.is_file());
    #[cfg(target_os = "linux")]
    assert!(running.is_dir(), "a running process's folder stays");
    #[cfg(unix)]
    assert!(link.symlink_metadata().is_ok(), "a link is never followed");
    assert!(outside.path().is_dir());
}

fn temp_patch_folders(pid: u32) -> Vec<PathBuf> {
    let prefix = format!("{}{pid}-", canopy_core::leftovers::PATCH_STAGING_PREFIX);
    fs::read_dir(std::env::temp_dir())
        .expect("temp")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect()
}

fn head(p: &Path) -> Oid {
    Oid::from_str(git(p, &["rev-parse", "HEAD"]).trim()).expect("an id")
}

#[test]
fn a_patch_export_stages_outside_the_users_folder_and_cleans_up_on_failure() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    let repo = Repo::open(source.path()).expect("open");
    let out = tempfile::tempdir().expect("tempdir");
    let id = head(source.path());
    let pid = std::process::id();

    let written = repo.export_patches(&[id], out.path()).expect("export");
    assert_eq!(written.len(), 1);
    // The folder the user chose holds the patch and nothing else, and nothing stays in temp.
    assert_eq!(fs::read_dir(out.path()).expect("list").count(), 1);
    assert!(
        temp_patch_folders(pid).is_empty(),
        "{:?}",
        temp_patch_folders(pid)
    );

    // A second export finds the patch and refuses, after staging: the folder is removed anyway.
    let error = repo.export_patches(&[id], out.path()).expect_err("refused");
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(fs::read_dir(out.path()).expect("list").count(), 1);
    assert_eq!(temp_patch_folders(pid), Vec::<PathBuf>::new());
}

/// Starts `git add` with a clean filter that never ends (git holds `index.lock` while it filters),
/// waits for the filter to run, kills git and the filter with SIGKILL, and returns once git is
/// dead.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly and kill it mid-operation"
)]
fn kill_git_mid_add(repo: &Path, file: &str) {
    use std::os::unix::process::CommandExt;
    let started = repo.join("filter-started");
    fs::write(repo.join(".gitattributes"), format!("{file} filter=hang\n")).expect("attributes");
    git(
        repo,
        &[
            "config",
            "filter.hang.clean",
            &format!("touch '{}'; exec sleep 60", started.display()),
        ],
    );
    let mut child = Command::new("git")
        .current_dir(repo)
        .args(["add", "--", file])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("git starts");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the filter never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The group id is the child's pid: one signal ends git and the filter.
    let group = format!("-{}", child.id());
    let killed = Command::new("kill")
        .args(["-KILL", "--", &group])
        .status()
        .expect("kill runs");
    assert!(killed.success());
    child.wait().expect("git is gone");
    // The next operations must not run the filter again.
    git(repo, &["config", "--unset", "filter.hang.clean"]);
    fs::remove_file(repo.join(".gitattributes")).expect("attributes");
}

#[cfg(unix)]
#[test]
fn a_git_killed_mid_add_leaves_its_lock_and_the_next_error_names_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    fs::write(dir.path().join("b.txt"), "two\n").expect("write");
    kill_git_mid_add(dir.path(), "b.txt");

    let lock = dir.path().join(".git/index.lock");
    assert!(lock.is_file(), "git was killed holding the index lock");

    let repo = Repo::open(dir.path()).expect("open");
    let error = repo
        .stage(&["a.txt".to_owned()])
        .expect_err("the lock blocks the next operation");
    let message = error.to_string();
    assert!(message.contains("index.lock"), "{message}");
    assert!(lock.is_file(), "Canopy never deletes the lock");
    // The scan for staging files is not confused by it either.
    assert_eq!(
        repo.abandoned_staging_files().expect("scan"),
        Vec::<PathBuf>::new()
    );
}
