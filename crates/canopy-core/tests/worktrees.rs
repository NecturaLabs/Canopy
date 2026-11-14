//! Removing linked working folders and forgetting the ones that are gone (#90): what a removal
//! would delete, the checks repeated right before git runs, and the missing and locked states,
//! each compared with the `git` command line on a temporary repository.

#![cfg(unix)]
#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use canopy_core::{Repo, WorktreeInfo, WorktreeState};

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_output(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Fixture {
    _root: tempfile::TempDir,
    main: PathBuf,
    /// Where linked folders go.
    beside: PathBuf,
    repo: Repo,
}

/// A repository with one commit and a `.gitignore` for `*.log` and `build/`.
fn fixture() -> Fixture {
    let root = tempfile::tempdir().expect("tempdir");
    let main = root.path().join("main");
    let beside = root.path().join("linked");
    fs::create_dir_all(&main).expect("folder");
    fs::create_dir_all(&beside).expect("folder");
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["config", "commit.gpgsign", "false"]);
    git(&main, &["config", "core.autocrlf", "false"]);
    fs::write(main.join("a.txt"), "one\n").expect("file");
    fs::write(main.join(".gitignore"), "*.log\nbuild/\n").expect("file");
    git(&main, &["add", "."]);
    git(&main, &["commit", "-q", "-m", "base"]);
    let repo = Repo::open(&main).expect("open");
    Fixture {
        _root: root,
        main,
        beside,
        repo,
    }
}

impl Fixture {
    /// A linked folder on a new branch `name`, in the shared parent folder.
    fn linked(&self, name: &str) -> PathBuf {
        let at = self.beside.join(name);
        git(
            &self.main,
            &["worktree", "add", "-q", "-b", name, &at.to_string_lossy()],
        );
        at
    }

    fn info(&self, folder: &Path) -> WorktreeInfo {
        let want = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        self.repo
            .worktrees()
            .into_iter()
            .find(|w| fs::canonicalize(&w.path).unwrap_or_else(|_| w.path.clone()) == want)
            .expect("the folder is listed")
    }

    fn registered(&self) -> String {
        git(&self.main, &["worktree", "list", "--porcelain"])
    }
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

#[test]
fn a_clean_worktree_is_removed_and_its_branch_stays() {
    let f = fixture();
    let wt = f.linked("topic");
    let removal = f.repo.worktree_removal(&wt).expect("inspect");
    assert_eq!(removal.branch.as_deref(), Some("topic"));
    assert!(removal.blocked.is_none(), "{:?}", removal.blocked);
    assert_eq!(removal.changed_files(), 0);
    assert_eq!(removal.ignored.count, 0);
    assert_eq!(removal.unreachable_commits, 0);
    assert!(!removal.needs_force());

    f.repo.remove_worktree(&removal).expect("remove");
    assert!(!exists(&wt));
    assert!(!f.registered().contains("topic\n"), "{}", f.registered());
    assert!(!f.registered().contains(&*wt.to_string_lossy()));
    let branches = git(&f.main, &["branch", "--list", "topic"]);
    assert!(branches.contains("topic"), "branch stays: {branches}");
    assert_eq!(f.repo.worktrees().len(), 1);
}

#[test]
fn each_kind_of_dirt_is_counted_and_git_refuses_it_without_force() {
    let f = fixture();
    let wt = f.linked("dirty");
    fs::write(wt.join("a.txt"), "changed\n").expect("modify");
    fs::write(wt.join("staged.txt"), "new\n").expect("stage");
    git(&wt, &["add", "staged.txt"]);
    fs::create_dir_all(wt.join("notes")).expect("folder");
    fs::write(wt.join("notes/one.md"), "x").expect("untracked");
    fs::write(wt.join("notes/two.md"), "x").expect("untracked");
    fs::write(wt.join("debug.log"), "x").expect("ignored");
    fs::create_dir_all(wt.join("build/deep")).expect("folder");
    fs::write(wt.join("build/deep/out.o"), "x").expect("ignored");

    let removal = f.repo.worktree_removal(&wt).expect("inspect");
    assert_eq!(removal.modified.count, 2, "{removal:?}");
    assert!(removal.modified.first.contains(&"a.txt".to_owned()));
    assert!(removal.modified.first.contains(&"staged.txt".to_owned()));
    assert_eq!(removal.untracked.count, 2, "{removal:?}");
    assert!(removal.untracked.first.contains(&"notes/one.md".to_owned()));
    // An ignored folder is one item, not every file in it.
    assert_eq!(removal.ignored.count, 2, "{removal:?}");
    assert!(removal.ignored.first.contains(&"build/".to_owned()));
    assert!(removal.ignored.first.contains(&"debug.log".to_owned()));
    assert!(removal.needs_force());
    assert_eq!(removal.changed_files(), 4);

    // Git agrees that it needs --force.
    let plain = git_output(
        &f.main,
        &["worktree", "remove", "--", &wt.to_string_lossy()],
    );
    assert!(!plain.status.success());
    assert!(exists(&wt));

    f.repo.remove_worktree(&removal).expect("forced removal");
    assert!(!exists(&wt));
    assert!(!f.registered().contains(&*wt.to_string_lossy()));
}

#[test]
fn ignored_files_alone_need_no_force_and_are_deleted() {
    let f = fixture();
    let wt = f.linked("cache");
    fs::write(wt.join("debug.log"), "x").expect("ignored");
    let removal = f.repo.worktree_removal(&wt).expect("inspect");
    assert_eq!(removal.ignored.count, 1);
    assert!(!removal.needs_force());
    // Git's own behaviour: a plain remove deletes ignored files.
    f.repo.remove_worktree(&removal).expect("remove");
    assert!(!exists(&wt));
}

#[test]
fn commits_only_a_detached_head_reaches_are_listed() {
    let f = fixture();
    let wt = f.beside.join("loose");
    git(
        &f.main,
        &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()],
    );
    let at_tip = f.repo.worktree_removal(&wt).expect("inspect");
    assert_eq!(at_tip.branch, None);
    assert_eq!(at_tip.unreachable_commits, 0, "main reaches HEAD");

    fs::write(wt.join("a.txt"), "two\n").expect("edit");
    git(&wt, &["commit", "-q", "-am", "only here"]);
    fs::write(wt.join("b.txt"), "three\n").expect("edit");
    git(&wt, &["add", "b.txt"]);
    git(&wt, &["commit", "-q", "-m", "and here"]);
    let lonely = f.repo.worktree_removal(&wt).expect("inspect");
    assert_eq!(lonely.unreachable_commits, 2);
    assert!(!lonely.needs_force());
    let tip = lonely.head.expect("head");

    // Git removes it without a word; the commits lose their last reference.
    f.repo.remove_worktree(&lonely).expect("remove");
    assert!(!exists(&wt));
    let reachable = git(&f.main, &["branch", "--contains", &tip.to_string()]);
    assert!(reachable.trim().is_empty(), "no branch has it: {reachable}");

    // A branch checked out there keeps its commits: nothing is lost.
    let on_branch = f.linked("kept");
    fs::write(on_branch.join("c.txt"), "x").expect("edit");
    git(&on_branch, &["add", "c.txt"]);
    git(&on_branch, &["commit", "-q", "-m", "on a branch"]);
    assert_eq!(
        f.repo
            .worktree_removal(&on_branch)
            .expect("inspect")
            .unreachable_commits,
        0
    );
}

#[test]
fn a_locked_worktree_is_listed_with_git_reason_and_never_removed() {
    let f = fixture();
    let wt = f.linked("usb");
    git(
        &f.main,
        &[
            "worktree",
            "lock",
            "--reason",
            "on a USB drive",
            &wt.to_string_lossy(),
        ],
    );
    let listed = f.info(&wt);
    assert_eq!(listed.locked.as_deref(), Some("on a USB drive"));
    assert_eq!(listed.state, WorktreeState::Ready);

    let removal = f.repo.worktree_removal(&wt).expect("inspect");
    let reason = removal.blocked.clone().expect("blocked");
    assert!(reason.contains("on a USB drive"), "{reason}");
    let error = f.repo.remove_worktree(&removal).expect_err("refused");
    assert!(error.to_string().contains("locked"), "{error}");
    assert!(exists(&wt));
    assert!(f.registered().contains("locked on a USB drive"));

    // A lock without a reason is locked all the same.
    let bare = f.linked("bare");
    git(&f.main, &["worktree", "lock", &bare.to_string_lossy()]);
    assert_eq!(f.info(&bare).locked.as_deref(), Some(""));
    assert!(
        f.repo
            .worktree_removal(&bare)
            .expect("inspect")
            .blocked
            .is_some()
    );
}

#[test]
fn the_main_and_the_open_folder_and_strangers_are_not_removable() {
    let f = fixture();
    let wt = f.linked("other");
    let error = f.repo.worktree_removal(&f.main).expect_err("main");
    assert!(error.to_string().contains("main working folder"), "{error}");
    let stranger = f.beside.join("nothing");
    fs::create_dir_all(&stranger).expect("folder");
    f.repo
        .worktree_removal(&stranger)
        .expect_err("not a worktree");

    // From inside the linked folder, that folder is the one open here; the main one is a
    // sibling that may go away only through its own repository.
    let inside = Repo::open(&wt).expect("open linked");
    let open_here = inside.worktree_removal(&wt).expect("inspect");
    let reason = open_here.blocked.clone().expect("blocked");
    assert!(reason.contains("open in this tab"), "{reason}");
    inside.remove_worktree(&open_here).expect_err("refused");
    assert!(exists(&wt));
    // It can remove a sibling.
    let sibling = f.linked("sibling");
    let removal = inside.worktree_removal(&sibling).expect("inspect");
    inside.remove_worktree(&removal).expect("remove sibling");
    assert!(!exists(&sibling));
}

#[test]
fn a_worktree_that_changed_after_the_confirmation_is_refused() {
    let f = fixture();

    // A file edited after the preview.
    let wt = f.linked("edited");
    let seen = f.repo.worktree_removal(&wt).expect("inspect");
    assert!(!seen.needs_force());
    fs::write(wt.join("a.txt"), "precious\n").expect("edit");
    let error = f.repo.remove_worktree(&seen).expect_err("refused");
    assert!(error.to_string().contains("changed since"), "{error}");
    assert_eq!(
        fs::read_to_string(wt.join("a.txt")).expect("kept"),
        "precious\n"
    );

    // One more untracked file after a forced removal was confirmed.
    fs::write(wt.join("one.txt"), "x").expect("untracked");
    let seen = f.repo.worktree_removal(&wt).expect("inspect");
    assert!(seen.needs_force());
    fs::write(wt.join("two.txt"), "y").expect("untracked");
    f.repo.remove_worktree(&seen).expect_err("refused");
    assert!(exists(&wt.join("two.txt")));

    // A new commit moves the branch.
    let moved = f.linked("moved");
    let seen = f.repo.worktree_removal(&moved).expect("inspect");
    fs::write(moved.join("n.txt"), "x").expect("file");
    git(&moved, &["add", "n.txt"]);
    git(&moved, &["commit", "-q", "-m", "more"]);
    let error = f.repo.remove_worktree(&seen).expect_err("refused");
    assert!(error.to_string().contains("branch or commit"), "{error}");
    assert!(exists(&moved));

    // A folder locked after the preview.
    let late = f.linked("late");
    let seen = f.repo.worktree_removal(&late).expect("inspect");
    git(
        &f.main,
        &[
            "worktree",
            "lock",
            "--reason",
            "now",
            &late.to_string_lossy(),
        ],
    );
    let error = f.repo.remove_worktree(&seen).expect_err("refused");
    assert!(error.to_string().contains("locked"), "{error}");
    assert!(exists(&late));
}

#[test]
fn assume_unchanged_flags_block_removal_as_they_do_stale_cleanup() {
    let f = fixture();
    let wt = f.linked("hidden");
    git(&wt, &["update-index", "--assume-unchanged", "a.txt"]);
    fs::write(wt.join("a.txt"), "hidden edit\n").expect("edit");
    let removal = f.repo.worktree_removal(&wt).expect("inspect");
    let reason = removal.blocked.clone().expect("blocked");
    assert!(reason.contains("Assume-unchanged"), "{reason}");
    f.repo.remove_worktree(&removal).expect_err("refused");
    assert_eq!(
        fs::read_to_string(wt.join("a.txt")).expect("kept"),
        "hidden edit\n"
    );
}

#[test]
fn missing_worktrees_are_listed_and_forgetting_matches_git_prune() {
    let f = fixture();
    let gone = f.linked("gone");
    let also = f.linked("also");
    let held = f.linked("held");
    let kept = f.linked("kept");
    git(
        &f.main,
        &[
            "worktree",
            "lock",
            "--reason",
            "drive",
            &held.to_string_lossy(),
        ],
    );
    for folder in [&gone, &also, &held] {
        fs::remove_dir_all(folder).expect("delete by hand");
    }

    let gone_info = f
        .repo
        .worktrees()
        .into_iter()
        .find(|w| w.path.ends_with("gone"))
        .expect("listed");
    assert_eq!(gone_info.state, WorktreeState::Missing);
    assert_eq!(gone_info.branch.as_deref(), Some("gone"));
    assert!(gone_info.head.is_some());
    assert_eq!(gone_info.locked, None);
    let held_info = f
        .repo
        .worktrees()
        .into_iter()
        .find(|w| w.path.ends_with("held"))
        .expect("listed");
    assert_eq!(held_info.state, WorktreeState::Missing);
    assert_eq!(held_info.locked.as_deref(), Some("drive"));
    assert_eq!(f.info(&kept).state, WorktreeState::Ready);
    // Not switchable: the branch of a folder that is gone does not claim a worktree.
    let branches = f.repo.branches().expect("branches");
    assert!(
        branches
            .iter()
            .find(|b| b.name == "gone")
            .expect("gone")
            .worktree
            .is_none()
    );
    assert!(
        branches
            .iter()
            .find(|b| b.name == "kept")
            .expect("kept")
            .worktree
            .is_some()
    );

    // What git would forget is exactly what Canopy lists to confirm.
    let dry = git_output(&f.main, &["worktree", "prune", "--dry-run", "-v"]);
    let dry = format!(
        "{}{}",
        String::from_utf8_lossy(&dry.stderr),
        String::from_utf8_lossy(&dry.stdout)
    );
    let mut listed = f.repo.missing_worktrees();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert_eq!(dry.matches("Removing worktrees/").count(), 2, "{dry}");
    assert!(
        dry.contains("worktrees/gone") && dry.contains("worktrees/also"),
        "{dry}"
    );
    listed.sort();

    // A folder that went missing after the confirmation is refused, nothing is forgotten.
    fs::remove_dir_all(&kept).expect("delete by hand");
    f.repo
        .forget_missing_worktrees(&listed)
        .expect_err("the set changed");
    assert!(f.registered().contains(&*gone.to_string_lossy()));
    fs::create_dir_all(&kept).expect("put the folder back");
    // (An empty folder without `.git` is as gone as a deleted one to git.)
    let expected = f.repo.missing_worktrees();
    assert_eq!(expected.len(), 3);
    f.repo.forget_missing_worktrees(&expected).expect("forget");
    let after = f.registered();
    assert!(!after.contains(&*gone.to_string_lossy()), "{after}");
    assert!(!after.contains(&*also.to_string_lossy()), "{after}");
    assert!(!after.contains(&*kept.to_string_lossy()), "{after}");
    // The locked one stays registered, with its reason, and so do the branches.
    assert!(after.contains(&*held.to_string_lossy()), "{after}");
    assert!(after.contains("locked drive"), "{after}");
    let branches = git(&f.main, &["branch", "--list"]);
    assert!(branches.contains("gone") && branches.contains("held"));
    assert_eq!(f.repo.missing_worktrees(), Vec::<PathBuf>::new());
    // Nothing to forget is not an error.
    f.repo.forget_missing_worktrees(&[]).expect("nothing to do");
}

#[test]
fn removing_a_missing_worktree_is_refused_with_the_way_out() {
    let f = fixture();
    let wt = f.linked("vanished");
    fs::remove_dir_all(&wt).expect("delete by hand");
    let removal = f.repo.worktree_removal(&wt).expect("inspect");
    let reason = removal.blocked.clone().expect("blocked");
    assert!(reason.contains("Forget"), "{reason}");
    f.repo.remove_worktree(&removal).expect_err("refused");
    assert!(f.registered().contains("vanished"));
}

#[test]
fn a_folder_that_cannot_be_read_is_listed_as_unreadable_not_missing() {
    let f = fixture();
    let wt = f.linked("locked-out");
    fs::set_permissions(&wt, fs::Permissions::from_mode(0o000)).expect("chmod");
    let readable = fs::read_dir(&wt).is_ok();
    let info = f
        .repo
        .worktrees()
        .into_iter()
        .find(|w| w.path.ends_with("locked-out"));
    fs::set_permissions(&wt, fs::Permissions::from_mode(0o755)).expect("chmod back");
    if readable {
        return; // running as a user the permission does not stop
    }
    let info = info.expect("listed");
    assert_eq!(info.state, WorktreeState::Unreadable);
    assert_eq!(info.branch.as_deref(), Some("locked-out"));
    assert!(
        !f.repo
            .missing_worktrees()
            .iter()
            .any(|p| p.ends_with("locked-out"))
    );
}
