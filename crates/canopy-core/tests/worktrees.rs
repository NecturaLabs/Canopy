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
    let listed = f.repo.forget_preview().expect("preview");
    assert_eq!(listed.blocked, None);
    assert_eq!(listed.entries.len(), 2, "{listed:?}");
    assert_eq!(dry.matches("Removing worktrees/").count(), 2, "{dry}");
    assert!(
        listed
            .entries
            .iter()
            .all(|e| e.gone && e.reason.contains("gitdir")),
        "git's own reason is kept: {listed:?}"
    );
    let ids: Vec<&str> = listed.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["also", "gone"], "by folder");
    assert!(
        dry.contains("worktrees/gone") && dry.contains("worktrees/also"),
        "{dry}"
    );

    // A folder that went missing after the confirmation is refused, nothing is forgotten.
    fs::remove_dir_all(&kept).expect("delete by hand");
    f.repo
        .forget_worktrees(&listed)
        .expect_err("the set changed");
    assert!(f.registered().contains(&*gone.to_string_lossy()));
    fs::create_dir_all(&kept).expect("put the folder back");
    // (An empty folder without `.git` is as gone as a deleted one to git.)
    let expected = f.repo.forget_preview().expect("preview");
    assert_eq!(expected.entries.len(), 3);
    f.repo.forget_worktrees(&expected).expect("forget");
    let after = f.registered();
    assert!(!after.contains(&*gone.to_string_lossy()), "{after}");
    assert!(!after.contains(&*also.to_string_lossy()), "{after}");
    assert!(!after.contains(&*kept.to_string_lossy()), "{after}");
    // The locked one stays registered, with its reason, and so do the branches.
    assert!(after.contains(&*held.to_string_lossy()), "{after}");
    assert!(after.contains("locked drive"), "{after}");
    let branches = git(&f.main, &["branch", "--list"]);
    assert!(branches.contains("gone") && branches.contains("held"));
    let nothing = f.repo.forget_preview().expect("preview");
    assert_eq!(nothing, canopy_core::WorktreeForget::default());
    // Nothing to forget is not an error.
    f.repo.forget_worktrees(&nothing).expect("nothing to do");
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
}

/// Git prunes an entry when `lstat` of its `.git` fails for any reason, so a folder that merely
/// cannot be read goes with the missing ones. Forgetting must not drop what the user was not
/// told about: the preview is git's own set, says which folder Canopy does not see as gone, and
/// the forgetting is refused.
#[test]
fn forgetting_does_not_prune_an_unreadable_folder_next_to_a_missing_one() {
    let f = fixture();
    let gone = f.linked("gone");
    let lo = f.linked("lo");
    fs::remove_dir_all(&gone).expect("delete by hand");
    fs::set_permissions(&lo, fs::Permissions::from_mode(0o000)).expect("chmod");
    let readable = fs::read_dir(&lo).is_ok();
    let preview = f.repo.forget_preview().expect("preview");
    let result = f.repo.forget_worktrees(&preview);
    let after = f.registered();
    fs::set_permissions(&lo, fs::Permissions::from_mode(0o755)).expect("chmod back");
    if readable {
        return; // running as a user the permission does not stop
    }
    assert_eq!(preview.entries.len(), 2, "git's own set: {preview:?}");
    let lo_entry = preview.entries.iter().find(|e| e.id == "lo").expect("lo");
    assert!(!lo_entry.gone, "{lo_entry:?}");
    let reason = preview.blocked.clone().expect("blocked");
    assert!(reason.contains("lo"), "{reason}");
    result.expect_err("refused");
    assert!(after.contains(&*lo.to_string_lossy()), "still registered");
    assert!(
        after.contains(&*gone.to_string_lossy()),
        "nothing was pruned"
    );
}

/// What forgetting deletes is in the preview: a detached `HEAD` whose commits nothing else
/// reaches, a tag keeping one, a submodule repository, an operation in progress.
#[test]
fn the_forget_preview_names_what_forgetting_loses() {
    let f = fixture();
    let lonely = f.beside.join("lonely");
    git(
        &f.main,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &lonely.to_string_lossy(),
        ],
    );
    fs::write(lonely.join("a.txt"), "two\n").expect("edit");
    git(&lonely, &["commit", "-q", "-am", "only here"]);
    let lonely_tip = git(&lonely, &["rev-parse", "HEAD"]).trim().to_owned();
    let tagged = f.beside.join("tagged");
    git(
        &f.main,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &tagged.to_string_lossy(),
        ],
    );
    fs::write(tagged.join("a.txt"), "three\n").expect("edit");
    git(&tagged, &["commit", "-q", "-am", "kept by a tag"]);
    let tagged_tip = git(&tagged, &["rev-parse", "HEAD"]);
    git(&f.main, &["tag", "keep", tagged_tip.trim()]);
    let busy = f.linked("busy");
    // A rebase stopped in that folder leaves its state in the administrative folder.
    let admin = f.main.join(".git/worktrees/busy");
    fs::create_dir_all(admin.join("rebase-merge")).expect("state");
    fs::create_dir_all(admin.join("modules/sub")).expect("modules");
    for folder in [&lonely, &tagged, &busy] {
        fs::remove_dir_all(folder).expect("delete by hand");
    }

    let preview = f.repo.forget_preview().expect("preview");
    assert_eq!(preview.blocked, None);
    let by_id = |id: &str| preview.entries.iter().find(|e| e.id == id).expect("entry");
    assert_eq!(by_id("lonely").unreachable_commits, 1, "{preview:?}");
    assert_eq!(
        by_id("lonely").head.map(|h| h.to_string()),
        Some(lonely_tip)
    );
    assert_eq!(by_id("tagged").unreachable_commits, 0, "a tag keeps it");
    assert_eq!(by_id("busy").branch.as_deref(), Some("busy"));
    assert_eq!(by_id("busy").unreachable_commits, 0);
    assert_eq!(by_id("busy").operation.as_deref(), Some("rebase"));
    assert!(by_id("busy").submodule_repositories);
    assert!(!by_id("lonely").submodule_repositories);
    assert_eq!(by_id("lonely").operation, None);

    // The deletion is real: the administrative folder goes, the commit loses its reference.
    f.repo.forget_worktrees(&preview).expect("forget");
    assert!(!admin.exists());
    let contains = git(
        &f.main,
        &[
            "branch",
            "--contains",
            &by_id("lonely").head.expect("head").to_string(),
        ],
    );
    assert!(contains.trim().is_empty(), "{contains}");
}

impl Fixture {
    /// A repository `deps/sub` as a submodule of the main one, committed, and a linked folder
    /// `name` where it is initialized (`init`) or not.
    fn with_submodule(&self, name: &str, init: bool) -> PathBuf {
        let sub = self.beside.join("sub-origin");
        if !sub.exists() {
            fs::create_dir_all(&sub).expect("folder");
            git(&sub, &["init", "-q", "-b", "main"]);
            fs::write(sub.join("lib.txt"), "lib\n").expect("file");
            git(&sub, &["add", "."]);
            git(&sub, &["commit", "-q", "-m", "lib"]);
            git(
                &self.main,
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "-q",
                    &sub.to_string_lossy(),
                    "deps/sub",
                ],
            );
            git(&self.main, &["commit", "-q", "-m", "add the submodule"]);
        }
        let wt = self.linked(name);
        if init {
            git(
                &wt,
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "update",
                    "-q",
                    "--init",
                ],
            );
        }
        wt
    }
}

/// A plain `git worktree remove` refuses a worktree with an initialized submodule, and `--force`
/// deletes the submodule's repository (under `.git/worktrees/<n>/modules`) with any commit that
/// was never pushed. Canopy says so up front and removes nothing, whatever the submodule holds.
#[test]
fn a_worktree_with_an_initialized_submodule_is_never_removed() {
    let f = fixture();
    let admin_modules = |name: &str| f.main.join(".git/worktrees").join(name).join("modules");

    // An unpushed commit in the submodule.
    let unpushed = f.with_submodule("unpushed", true);
    fs::write(unpushed.join("deps/sub/new.txt"), "work\n").expect("file");
    git(&unpushed.join("deps/sub"), &["add", "new.txt"]);
    git(
        &unpushed.join("deps/sub"),
        &["commit", "-q", "-m", "never pushed"],
    );
    // Only an untracked file in the submodule.
    let untracked = f.with_submodule("untracked", true);
    fs::write(untracked.join("deps/sub/scratch.txt"), "x\n").expect("file");
    // Nothing at all: git refuses it too.
    let clean = f.with_submodule("clean", true);
    let plain = git_output(
        &f.main,
        &["worktree", "remove", "--", &clean.to_string_lossy()],
    );
    assert!(!plain.status.success(), "git itself refuses a clean one");

    for (name, folder) in [
        ("unpushed", &unpushed),
        ("untracked", &untracked),
        ("clean", &clean),
    ] {
        let removal = f.repo.worktree_removal(folder).expect("inspect");
        let reason = removal.blocked.clone().unwrap_or_default();
        assert!(
            reason.contains("submodule") && reason.contains("deps/sub"),
            "{name}: {reason}"
        );
        let error = f.repo.remove_worktree(&removal).expect_err("refused");
        assert!(error.to_string().contains("submodule"), "{name}: {error}");
        assert!(exists(folder), "{name}: folder kept");
        assert!(
            admin_modules(name).is_dir(),
            "{name}: the submodule's repository is kept"
        );
    }
    // The commit that was never pushed is still there.
    let log = git(&unpushed.join("deps/sub"), &["log", "--oneline", "-1"]);
    assert!(log.contains("never pushed"), "{log}");
}

/// A submodule that was never initialized is an empty folder: git removes the worktree, and so
/// does Canopy.
#[test]
fn a_worktree_with_an_uninitialized_submodule_is_removed() {
    let f = fixture();
    let wt = f.with_submodule("fresh", false);
    let removal = f.repo.worktree_removal(&wt).expect("inspect");
    assert_eq!(removal.blocked, None);
    f.repo.remove_worktree(&removal).expect("remove");
    assert!(!exists(&wt));
}

/// A repository inside the folder, or another worktree registered under it, looks like an
/// untracked file; `--force` would delete it (and leave the inner worktree registered).
#[test]
fn a_repository_or_worktree_inside_the_folder_is_never_deleted_with_it() {
    let f = fixture();

    // An unrelated repository with a commit, in an untracked folder.
    let outer = f.linked("outer");
    let inner = outer.join("vendor/inner");
    fs::create_dir_all(&inner).expect("folder");
    git(&inner, &["init", "-q", "-b", "main"]);
    fs::write(inner.join("work.txt"), "unpublished\n").expect("file");
    git(&inner, &["add", "."]);
    git(
        &inner,
        &["commit", "-q", "-m", "only in the nested repository"],
    );
    let removal = f.repo.worktree_removal(&outer).expect("inspect");
    let reason = removal.blocked.clone().unwrap_or_default();
    assert!(
        reason.contains("another repository") && reason.contains("vendor/inner"),
        "{reason}"
    );
    f.repo.remove_worktree(&removal).expect_err("refused");
    assert!(
        exists(&inner.join("work.txt")),
        "the nested repository is kept"
    );

    // Another worktree of this repository, created inside it.
    let host = f.linked("host");
    let guest = host.join("guest");
    git(
        &f.main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "guest",
            &guest.to_string_lossy(),
        ],
    );
    let removal = f.repo.worktree_removal(&host).expect("inspect");
    let reason = removal.blocked.clone().unwrap_or_default();
    assert!(
        reason.contains("another worktree") && reason.contains("guest"),
        "{reason}"
    );
    f.repo.remove_worktree(&removal).expect_err("refused");
    assert!(exists(&guest), "the inner worktree is kept");
    assert!(f.registered().contains(&*guest.to_string_lossy()));

    // The inner one is removable by itself, and then the outer one is too.
    let inner_removal = f.repo.worktree_removal(&guest).expect("inspect");
    assert_eq!(inner_removal.blocked, None);
    f.repo
        .remove_worktree(&inner_removal)
        .expect("remove the inner one");
    let removal = f.repo.worktree_removal(&host).expect("inspect");
    assert_eq!(removal.blocked, None);
    f.repo
        .remove_worktree(&removal)
        .expect("remove the outer one");
}
