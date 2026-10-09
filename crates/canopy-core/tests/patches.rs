//! Exporting commits as patch files and applying them, against throwaway repositories with the
//! git CLI as the oracle.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::{Repo, RepoState};

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Grace Hopper")
        .env("GIT_COMMITTER_EMAIL", "grace@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with one commit of `a.txt`, a local identity, no signing and LF line endings.
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
    fs::write(p.join("a.txt"), "one\ntwo\nthree\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: first"]);
}

fn commit_file(p: &Path, file: &str, text: &str, message: &str) {
    fs::write(p.join(file), text).expect("write");
    git(p, &["add", "--", file]);
    git(p, &["commit", "-q", "-m", message]);
}

fn oid(p: &Path, rev: &str) -> git2::Oid {
    git2::Oid::from_str(git(p, &["rev-parse", rev]).trim()).expect("an id")
}

/// Subject, author and tree of every commit, newest first.
fn history(p: &Path) -> String {
    git(p, &["log", "--format=%s|%an <%ae>|%T"])
}

#[test]
fn exported_commits_apply_in_order_and_keep_their_authors() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    commit_file(source.path(), "b.txt", "b\n", "feat: add b");
    commit_file(source.path(), "c.txt", "c\n", "feat: add c");
    let repo = Repo::open(source.path()).expect("open");
    let out = tempfile::tempdir().expect("tempdir");
    let dir = out.path().join("patches");

    // Selected newest first: the files still come out parents first.
    let files = repo
        .export_patches(
            &[oid(source.path(), "HEAD"), oid(source.path(), "HEAD~1")],
            &dir,
        )
        .expect("export");
    let names: Vec<_> = files
        .iter()
        .map(|f| f.file_name().expect("name").to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["0001-feat-add-b.patch", "0002-feat-add-c.patch"]);
    assert!(files.iter().all(|f| f.is_file()));

    let twin = tempfile::tempdir().expect("tempdir");
    init(twin.path());
    let target = Repo::open(twin.path()).expect("open");
    target.apply_patches(&files).expect("apply");
    assert_eq!(history(twin.path()), history(source.path()));

    let oracle = tempfile::tempdir().expect("tempdir");
    init(oracle.path());
    let mut args = vec!["am", "--3way", "--"];
    let paths: Vec<String> = files
        .iter()
        .map(|f| f.to_string_lossy().into_owned())
        .collect();
    args.extend(paths.iter().map(String::as_str));
    git(oracle.path(), &args);
    assert_eq!(history(twin.path()), history(oracle.path()));
}

#[test]
fn a_patch_that_does_not_apply_can_be_checked_and_the_apply_aborted() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    commit_file(
        source.path(),
        "a.txt",
        "one\nTWO\nthree\n",
        "fix: shout two",
    );
    let repo = Repo::open(source.path()).expect("open");
    let out = tempfile::tempdir().expect("tempdir");
    let files = repo
        .export_patches(&[oid(source.path(), "HEAD")], out.path())
        .expect("export");

    let twin = tempfile::tempdir().expect("tempdir");
    init(twin.path());
    commit_file(twin.path(), "a.txt", "one\n2\nthree\n", "fix: number two");
    let target = Repo::open(twin.path()).expect("open");
    let before = git(twin.path(), &["rev-parse", "HEAD"]);

    let check = target.check_patch(&files[0]).expect_err("does not apply");
    assert!(check.to_string().contains("a.txt"), "{check}");
    let applied = target.apply_patches(&files).expect_err("conflicts");
    assert!(applied.to_string().contains("a.txt"), "{applied}");
    let state = target.snapshot(10).expect("snapshot").state;
    assert_eq!(state, RepoState::ApplyMailbox);

    target
        .continue_or_abort(RepoState::ApplyMailbox, true)
        .expect("abort");
    assert_eq!(git(twin.path(), &["rev-parse", "HEAD"]), before);
    assert_eq!(git(twin.path(), &["status", "--porcelain"]), "");
    assert_eq!(
        target.snapshot(10).expect("snapshot").state,
        RepoState::Clean
    );
}

#[test]
fn a_patch_folder_must_be_a_full_path() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    let repo = Repo::open(source.path()).expect("open");
    let error = repo
        .export_patches(&[oid(source.path(), "HEAD")], Path::new("patches"))
        .expect_err("relative");
    assert!(error.to_string().contains("full folder path"));
}

#[test]
fn export_never_replaces_a_patch_file_already_in_the_folder() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    commit_file(source.path(), "b.txt", "b\n", "feat: add b");
    let repo = Repo::open(source.path()).expect("open");
    let head = oid(source.path(), "HEAD");
    let out = tempfile::tempdir().expect("tempdir");
    let first = repo
        .export_patches(&[head], out.path())
        .expect("first export");
    fs::write(&first[0], "mine").expect("edit");
    let error = repo
        .export_patches(&[head], out.path())
        .expect_err("a second export must refuse");
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(fs::read_to_string(&first[0]).expect("read"), "mine");
    assert_eq!(
        fs::read_dir(out.path()).expect("list").count(),
        1,
        "no staging folder is left behind"
    );
}
