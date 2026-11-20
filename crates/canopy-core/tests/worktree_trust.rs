//! Trust for linked worktrees: whether Canopy may run a repository's own git configuration is
//! decided from where the repository really lives (its working folder, the common git directory
//! and the worktree's private git directory), and a forged pointer cannot borrow a trusted
//! repository's place. Real repositories, built with `git worktree add`.

#![cfg(unix)]
#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

mod support;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use support::isolate_git;

use canopy_core::Repo;

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with one commit at `dir`.
fn repository(dir: &Path) {
    fs::create_dir_all(dir).expect("folder");
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Ada Lovelace"]);
    git(dir, &["config", "user.email", "ada@example.com"]);
    fs::write(dir.join("a.txt"), "one\n").expect("file");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "base"]);
}

fn worktree(main: &Path, at: &Path, branch: &str) {
    fs::create_dir_all(at.parent().expect("parent")).expect("folder");
    git(
        main,
        &["worktree", "add", "-q", "-b", branch, &at.to_string_lossy()],
    );
}

fn trusted(repo: &Path, folders: &[&Path]) -> bool {
    let folders: Vec<PathBuf> = folders.iter().map(|p| p.to_path_buf()).collect();
    Repo::open(repo)
        .expect("open")
        .is_in_trusted_folder(&folders)
}

struct Fixture {
    _root: tempfile::TempDir,
    trusted: PathBuf,
    elsewhere: PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().expect("tempdir");
    let base = fs::canonicalize(root.path()).expect("canonical");
    let trusted = base.join("trusted");
    let elsewhere = base.join("elsewhere");
    fs::create_dir_all(&trusted).expect("trusted");
    fs::create_dir_all(&elsewhere).expect("elsewhere");
    Fixture {
        _root: root,
        trusted,
        elsewhere,
    }
}

#[test]
fn a_worktree_is_trusted_only_when_its_folder_and_the_shared_git_folder_both_are() {
    isolate_git!(a_worktree_is_trusted_only_when_its_folder_and_the_shared_git_folder_both_are);
    let f = fixture();
    let main = f.trusted.join("main");
    repository(&main);
    // Both inside: trusted.
    let inside = f.trusted.join("wt-inside");
    worktree(&main, &inside, "inside");
    assert!(trusted(&main, &[&f.trusted]));
    assert!(trusted(&inside, &[&f.trusted]));
    // A worktree of a trusted repository, placed outside: its folder is not trusted.
    let outside = f.elsewhere.join("wt-outside");
    worktree(&main, &outside, "outside");
    assert!(!trusted(&outside, &[&f.trusted]));
    // A worktree inside the trusted folder of a repository outside it: the shared git folder
    // (and its configuration) lies outside, so it is not trusted.
    let foreign = f.elsewhere.join("foreign");
    repository(&foreign);
    let planted = f.trusted.join("wt-of-foreign");
    worktree(&foreign, &planted, "planted");
    assert!(!trusted(&planted, &[&f.trusted]));
    assert!(!trusted(&foreign, &[&f.trusted]));
}

#[test]
fn a_forged_git_file_cannot_borrow_a_trusted_repositorys_trust() {
    isolate_git!(a_forged_git_file_cannot_borrow_a_trusted_repositorys_trust);
    let f = fixture();
    let main = f.trusted.join("main");
    repository(&main);
    let real = f.trusted.join("real-wt");
    worktree(&main, &real, "real");
    let private = main.join(".git").join("worktrees").join("real-wt");
    assert!(private.is_dir(), "{private:?}");

    // A folder outside whose `.git` file points at the trusted worktree's private git folder.
    let forged = f.elsewhere.join("forged");
    fs::create_dir_all(&forged).expect("folder");
    fs::write(
        forged.join(".git"),
        format!("gitdir: {}\n", private.display()),
    )
    .expect("forged pointer");
    // libgit2 checks the pointer against the private folder's own record of its worktree, so the
    // repository it opens is the trusted worktree itself: git runs in that folder, never in the
    // forged one, and only ever reads the trusted repository's own configuration.
    if let Ok(repo) = Repo::open(&forged) {
        let said = fs::canonicalize(repo.workdir()).expect("workdir");
        assert_ne!(said, fs::canonicalize(&forged).expect("forged"));
        if repo.is_in_trusted_folder(std::slice::from_ref(&f.trusted)) {
            assert_eq!(said, fs::canonicalize(&real).expect("real"));
        }
    }

    // The same with a trusted repository's own git folder.
    let forged_main = f.elsewhere.join("forged-main");
    fs::create_dir_all(&forged_main).expect("folder");
    fs::write(
        forged_main.join(".git"),
        format!("gitdir: {}\n", main.join(".git").display()),
    )
    .expect("forged pointer");
    if let Ok(repo) = Repo::open(&forged_main) {
        let said = fs::canonicalize(repo.workdir()).expect("workdir");
        assert_ne!(said, fs::canonicalize(&forged_main).expect("forged"));
        if repo.is_in_trusted_folder(std::slice::from_ref(&f.trusted)) {
            assert_eq!(said, fs::canonicalize(&main).expect("main"));
        }
    }

    // Trusting the forged folder does not trust a git folder that lies elsewhere.
    if let Ok(repo) = Repo::open(&forged) {
        assert!(!repo.is_in_trusted_folder(std::slice::from_ref(&f.elsewhere)));
    }
}

#[test]
fn a_rewritten_commondir_or_gitdir_in_a_worktree_cannot_move_it_into_trust() {
    isolate_git!(a_rewritten_commondir_or_gitdir_in_a_worktree_cannot_move_it_into_trust);
    let f = fixture();
    let foreign = f.elsewhere.join("foreign");
    repository(&foreign);
    let wt = f.trusted.join("wt");
    worktree(&foreign, &wt, "wt");
    assert!(!trusted(&wt, &[&f.trusted]), "baseline");

    // The private git folder's `commondir` made to name a trusted repository's git folder.
    let main = f.trusted.join("main");
    repository(&main);
    let private = foreign.join(".git").join("worktrees").join("wt");
    let before = fs::read_to_string(private.join("commondir")).expect("commondir");
    fs::write(
        private.join("commondir"),
        format!("{}\n", main.join(".git").display()),
    )
    .expect("rewrite");
    if let Ok(repo) = Repo::open(&wt) {
        // Whatever libgit2 now reports, the private git folder itself is outside the trusted
        // folder, so the repository is not trusted.
        assert!(!repo.is_in_trusted_folder(std::slice::from_ref(&f.trusted)));
    }
    fs::write(private.join("commondir"), before).expect("restore");
}

#[test]
fn links_cannot_move_a_worktree_into_or_out_of_a_trusted_folder() {
    isolate_git!(links_cannot_move_a_worktree_into_or_out_of_a_trusted_folder);
    let f = fixture();
    let main = f.trusted.join("main");
    repository(&main);
    let wt = f.elsewhere.join("wt");
    worktree(&main, &wt, "wt");
    // A link inside the trusted folder to the worktree outside it.
    symlink(&wt, f.trusted.join("wt-link")).expect("link");
    assert!(!trusted(&f.trusted.join("wt-link"), &[&f.trusted]));
    // A link elsewhere to a trusted worktree: it really is inside.
    let inside = f.trusted.join("wt2");
    worktree(&main, &inside, "wt2");
    symlink(&inside, f.elsewhere.join("shortcut")).expect("link");
    assert!(trusted(&f.elsewhere.join("shortcut"), &[&f.trusted]));
    // A trusted folder that is a link to the folder holding them all.
    symlink(&f.trusted, f.elsewhere.join("trusted-alias")).expect("link");
    assert!(trusted(&inside, &[&f.elsewhere.join("trusted-alias")]));
    // The parent-folder choice: trusting the folder above both covers both.
    let parent = f.trusted.parent().expect("parent");
    assert!(trusted(&wt, &[parent]));
    assert!(trusted(&main, &[parent]));
}
