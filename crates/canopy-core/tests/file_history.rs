//! A file's history across renames, against throwaway repositories with `git log --follow` as the
//! oracle.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::{DiffSource, MissingObjects, Repo, Whitespace};

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

fn write(p: &Path, file: &str, text: &str) {
    let full = p.join(file);
    fs::create_dir_all(full.parent().expect("parent")).expect("dirs");
    fs::write(full, text).expect("write");
}

/// `a.txt` created, edited, renamed to `b.txt`, edited, moved to `docs/c d.txt` and edited; an
/// unrelated file changes in between.
fn renamed_twice(p: &Path) {
    git(p, &["init", "-q", "-b", "main"]);
    for (key, value) in [("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        git(p, &["config", key, value]);
    }
    let lines = |n: usize| {
        let rows: Vec<String> = (1..=n).map(|i| format!("line {i}")).collect();
        format!("{}\n", rows.join("\n"))
    };
    write(p, "a.txt", &lines(20));
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "add a"]);
    write(p, "a.txt", &lines(21));
    git(p, &["commit", "-q", "-am", "edit a"]);
    git(p, &["mv", "a.txt", "b.txt"]);
    git(p, &["commit", "-q", "-m", "rename to b"]);
    write(p, "other.txt", "x\n");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "unrelated"]);
    write(p, "b.txt", &lines(22));
    git(p, &["commit", "-q", "-am", "edit b"]);
    fs::create_dir_all(p.join("docs")).expect("dir");
    git(p, &["mv", "b.txt", "docs/c d.txt"]);
    git(p, &["commit", "-q", "-m", "move to docs"]);
    write(p, "docs/c d.txt", &lines(23));
    git(p, &["commit", "-q", "-am", "edit c"]);
}

/// What `git log --follow` lists for `path`, newest first.
fn oracle(p: &Path, path: &str) -> Vec<String> {
    git(p, &["log", "--follow", "--format=%H", "--", path])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn history_follows_a_file_through_renames_like_git() {
    let dir = tempfile::tempdir().expect("tmp");
    renamed_twice(dir.path());
    let repo = Repo::open(dir.path()).expect("repo");

    let history = repo.file_history("docs/c d.txt", 100).expect("history");
    let ids: Vec<String> = history.entries.iter().map(|e| e.oid.to_string()).collect();
    assert_eq!(ids, oracle(dir.path(), "docs/c d.txt"));
    assert_eq!(ids.len(), 6, "the unrelated commit is not in it: {ids:?}");
    assert!(!history.truncated);

    let rows: Vec<(&str, &str, Option<&str>)> = history
        .entries
        .iter()
        .map(|e| (e.summary.as_str(), e.path.as_str(), e.old_path.as_deref()))
        .collect();
    assert_eq!(
        rows,
        [
            ("edit c", "docs/c d.txt", None),
            ("move to docs", "docs/c d.txt", Some("b.txt")),
            ("edit b", "b.txt", None),
            ("rename to b", "b.txt", Some("a.txt")),
            ("edit a", "a.txt", None),
            ("add a", "a.txt", None),
        ]
    );
    assert!(history.entries.iter().all(|e| e.author == "Ada Lovelace"));
}

#[test]
fn a_historical_diff_opens_without_touching_the_working_tree() {
    let dir = tempfile::tempdir().expect("tmp");
    renamed_twice(dir.path());
    let repo = Repo::open(dir.path()).expect("repo");
    let history = repo.file_history("docs/c d.txt", 100).expect("history");
    let before = git(dir.path(), &["status", "--porcelain=v2", "--branch"]);

    let edit_b = &history.entries[2];
    let diff = repo
        .file_diff(
            &DiffSource::Commit(edit_b.oid),
            &edit_b.path,
            edit_b.old_path.as_deref(),
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let added: Vec<&str> = diff
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| l.kind == canopy_core::LineKind::Added)
        .map(|l| l.text.trim_end())
        .collect();
    assert_eq!(added, ["line 22"]);
    assert_eq!(
        git(dir.path(), &["status", "--porcelain=v2", "--branch"]),
        before
    );
    assert!(dir.path().join("docs/c d.txt").is_file());
    assert!(!dir.path().join("b.txt").exists());
}

#[test]
fn a_limit_keeps_the_newest_commits_and_says_there_are_more() {
    let dir = tempfile::tempdir().expect("tmp");
    renamed_twice(dir.path());
    let repo = Repo::open(dir.path()).expect("repo");
    let all = repo.file_history("docs/c d.txt", 100).expect("history");

    let part = repo.file_history("docs/c d.txt", 3).expect("history");
    assert_eq!(part.entries, all.entries[..3]);
    assert!(part.truncated);
    let exact = repo.file_history("docs/c d.txt", 6).expect("history");
    assert!(!exact.truncated);
}

#[test]
fn a_path_is_literal_and_an_unknown_file_has_no_history() {
    let dir = tempfile::tempdir().expect("tmp");
    renamed_twice(dir.path());
    let repo = Repo::open(dir.path()).expect("repo");
    for path in ["docs/*", "missing.txt"] {
        let history = repo.file_history(path, 10).expect("history");
        assert!(history.entries.is_empty(), "{path}");
    }
}

#[test]
fn an_unborn_branch_reports_gits_message() {
    let dir = tempfile::tempdir().expect("tmp");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    let repo = Repo::open(dir.path()).expect("repo");
    let error = repo.file_history("a.txt", 10).expect_err("no commits");
    assert!(
        error.to_string().contains("does not have any commits"),
        "{error}"
    );
}
