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
fn a_series_stopped_by_a_conflict_continues_with_the_rest_after_it_is_resolved() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    commit_file(source.path(), "b.txt", "b\n", "feat: add b");
    commit_file(
        source.path(),
        "a.txt",
        "one\nTWO\nthree\n",
        "fix: shout two",
    );
    commit_file(source.path(), "c.txt", "c\n", "feat: add c");
    let repo = Repo::open(source.path()).expect("open");
    let out = tempfile::tempdir().expect("tempdir");
    let files = repo
        .export_patches(
            &[
                oid(source.path(), "HEAD~2"),
                oid(source.path(), "HEAD~1"),
                oid(source.path(), "HEAD"),
            ],
            out.path(),
        )
        .expect("export");
    assert_eq!(files.len(), 3);

    let twin = tempfile::tempdir().expect("tempdir");
    init(twin.path());
    commit_file(twin.path(), "a.txt", "one\n2\nthree\n", "fix: number two");
    let target = Repo::open(twin.path()).expect("open");

    let stopped = target.apply_patches(&files).expect_err("second conflicts");
    assert!(stopped.to_string().contains("a.txt"), "{stopped}");
    assert_eq!(
        target.snapshot(10).expect("snapshot").state,
        RepoState::ApplyMailbox
    );
    assert_eq!(
        git(twin.path(), &["log", "-1", "--format=%s"]).trim(),
        "feat: add b"
    );

    // Resolve the conflict, stage it and continue: the third patch must follow.
    fs::write(twin.path().join("a.txt"), "one\nTWO\nthree\n").expect("write");
    git(twin.path(), &["add", "--", "a.txt"]);
    target
        .continue_or_abort(RepoState::ApplyMailbox, false)
        .expect("continue");

    assert_eq!(
        target.snapshot(10).expect("snapshot").state,
        RepoState::Clean
    );
    assert_eq!(git(twin.path(), &["status", "--porcelain"]), "");
    assert_eq!(
        git(twin.path(), &["log", "--format=%s|%an", "-3"]),
        "feat: add c|Ada Lovelace\nfix: shout two|Ada Lovelace\nfeat: add b|Ada Lovelace\n"
    );
    assert!(twin.path().join("c.txt").is_file());
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

/// `main` with `feat: add c` after a merge of `side` (`feat: add s`) into the line of
/// `feat: add b`, returning the merge commit.
fn history_with_a_merge(p: &Path) -> git2::Oid {
    init(p);
    git(p, &["checkout", "-q", "-b", "side"]);
    commit_file(p, "s.txt", "s\n", "feat: add s");
    git(p, &["checkout", "-q", "main"]);
    commit_file(p, "b.txt", "b\n", "feat: add b");
    git(p, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let merge = oid(p, "HEAD");
    commit_file(p, "c.txt", "c\n", "feat: add c");
    merge
}

#[test]
fn the_first_parent_chain_is_newest_first_and_stops_at_the_root() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    commit_file(source.path(), "b.txt", "b\n", "feat: add b");
    commit_file(source.path(), "c.txt", "c\n", "feat: add c");
    let repo = Repo::open(source.path()).expect("open");
    let head = oid(source.path(), "HEAD");
    let expected: Vec<_> = ["HEAD", "HEAD~1", "HEAD~2"]
        .iter()
        .map(|rev| oid(source.path(), rev))
        .collect();

    assert_eq!(
        repo.first_parent_chain(head, 1).expect("one"),
        &expected[..1]
    );
    assert_eq!(
        repo.first_parent_chain(head, 2).expect("two"),
        &expected[..2]
    );
    // Past the root, and the cap, the chain is just the whole history; zero still selects the commit.
    assert_eq!(repo.first_parent_chain(head, 3).expect("all"), expected);
    assert_eq!(
        repo.first_parent_chain(head, 50).expect("past the root"),
        expected
    );
    assert_eq!(
        repo.first_parent_chain(head, 0).expect("zero"),
        &expected[..1]
    );
    // Starting in the middle walks only what is before it.
    assert_eq!(
        repo.first_parent_chain(expected[1], 9).expect("middle"),
        &expected[1..]
    );
}

#[test]
fn the_chain_is_capped_and_follows_the_first_parent_through_a_merge() {
    let source = tempfile::tempdir().expect("tempdir");
    let merge = history_with_a_merge(source.path());
    let repo = Repo::open(source.path()).expect("open");
    let head = oid(source.path(), "HEAD");

    // Oracle: `git rev-list --first-parent` skips `side`'s commit and keeps the merge.
    let listed = git(source.path(), &["rev-list", "--first-parent", "HEAD"]);
    let expected: Vec<git2::Oid> = listed
        .lines()
        .map(|id| git2::Oid::from_str(id).expect("an id"))
        .collect();
    assert_eq!(expected.len(), 4);
    assert!(expected.contains(&merge));
    assert_eq!(repo.first_parent_chain(head, 50).expect("chain"), expected);
    assert_eq!(
        repo.first_parent_chain(head, 3).expect("three"),
        &expected[..3]
    );

    // The cap: a long history is cut at MAX_PATCH_SERIES whatever is asked.
    let long = tempfile::tempdir().expect("tempdir");
    init(long.path());
    for n in 0..canopy_core::MAX_PATCH_SERIES + 5 {
        commit_file(
            long.path(),
            "n.txt",
            &format!("{n}\n"),
            &format!("step {n}"),
        );
    }
    let long_repo = Repo::open(long.path()).expect("open");
    let chain = long_repo
        .first_parent_chain(oid(long.path(), "HEAD"), usize::MAX)
        .expect("capped");
    assert_eq!(chain.len(), canopy_core::MAX_PATCH_SERIES);
}

#[test]
fn a_merge_in_the_series_is_set_apart_and_exporting_it_is_refused() {
    let source = tempfile::tempdir().expect("tempdir");
    let merge = history_with_a_merge(source.path());
    let repo = Repo::open(source.path()).expect("open");
    let head = oid(source.path(), "HEAD");
    let out = tempfile::tempdir().expect("tempdir");

    let series = repo.patch_series(head, 4).expect("series");
    assert_eq!(series.merges, [merge]);
    assert_eq!(
        series.commits,
        [
            oid(source.path(), "HEAD"),
            oid(source.path(), "HEAD~2"),
            oid(source.path(), "HEAD~3"),
        ]
    );
    let files = repo
        .export_patches(&series.commits, out.path())
        .expect("export");
    let names: Vec<_> = files
        .iter()
        .map(|f| f.file_name().expect("name").to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "0001-feat-first.patch",
            "0002-feat-add-b.patch",
            "0003-feat-add-c.patch"
        ]
    );

    // The merge alone has no patch: git would write its parent's instead.
    let alone = repo.patch_series(merge, 1).expect("alone");
    assert_eq!(alone.commits, Vec::<git2::Oid>::new());
    assert_eq!(alone.merges, [merge]);
    let git_wrote = git(
        source.path(),
        &[
            "format-patch",
            "-1",
            "-o",
            &out.path().join("oracle").to_string_lossy(),
            &merge.to_string(),
        ],
    );
    assert!(git_wrote.contains("feat-add-b"), "{git_wrote}");
    let refused = repo
        .export_patches(&[merge], &out.path().join("none"))
        .expect_err("a merge");
    assert!(
        refused
            .to_string()
            .contains("A merge commit cannot be exported as a patch"),
        "{refused}"
    );
    assert!(!out.path().join("none").exists(), "nothing is written");
}

/// The case that went wrong: `git format-patch -1` of a merge writes the patch of its nearest
/// non-merge ancestor, so a merge selected on its own must be refused, not exported as another
/// commit.
#[test]
fn a_merge_is_never_exported_as_the_commit_before_it() {
    let p = tempfile::tempdir().expect("tempdir");
    init(p.path());
    git(p.path(), &["checkout", "-q", "-b", "f"]);
    commit_file(p.path(), "b.txt", "b\n", "feat: b");
    git(p.path(), &["checkout", "-q", "main"]);
    commit_file(p.path(), "c.txt", "c\n", "feat: c");
    git(p.path(), &["merge", "-q", "--no-ff", "-m", "merge f", "f"]);
    let merge = oid(p.path(), "HEAD");
    let out = p.path().join("out");

    // git itself writes the patch of "c" for the merge.
    let wrote = git(
        p.path(),
        &["format-patch", "-1", "-o", &out.to_string_lossy(), "HEAD"],
    );
    assert!(wrote.contains("0001-feat-c.patch"), "{wrote}");
    fs::remove_dir_all(&out).expect("clean");

    let repo = Repo::open(p.path()).expect("open");
    let refused = repo
        .export_patches(&[merge], &out)
        .expect_err("a merge cannot be a patch");
    assert_eq!(
        refused.to_string(),
        format!(
            "A merge commit cannot be exported as a patch: {}",
            canopy_core::short_id(merge)
        )
    );
    // Among others it refuses too, and nothing at all is written.
    let refused_among = repo
        .export_patches(&[oid(p.path(), "HEAD~1"), merge], &out)
        .expect_err("a merge among others");
    assert!(refused_among.to_string().contains("A merge commit"));
    assert!(!out.exists(), "nothing is written");
}

#[test]
fn a_configured_cover_letter_is_not_exported_with_the_patches() {
    let source = tempfile::tempdir().expect("tempdir");
    init(source.path());
    commit_file(source.path(), "b.txt", "b\n", "feat: add b");
    git(source.path(), &["config", "format.coverLetter", "true"]);
    let repo = Repo::open(source.path()).expect("open");
    let out = tempfile::tempdir().expect("tempdir");
    let files = repo
        .export_patches(&[oid(source.path(), "HEAD")], out.path())
        .expect("export");
    assert_eq!(files.len(), 1, "{files:?}");

    let twin = tempfile::tempdir().expect("tempdir");
    init(twin.path());
    let target = Repo::open(twin.path()).expect("open");
    target.apply_patches(&files).expect("applies");
    assert_eq!(history(twin.path()), history(source.path()));
}
