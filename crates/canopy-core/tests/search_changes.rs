//! Pickaxe search against throwaway repositories, with `git log -S`/`-G` as the oracle.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

mod support;

use std::fs;
use std::path::Path;
use std::process::Command;

use support::isolate_git;

use canopy_core::Repo;

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
    fs::write(p.join(file), text).expect("write");
}

/// `needle` appears in `a.txt` (commit 2) and again in `b.txt` (commit 3), and goes away from
/// `a.txt` (commit 4); commit 1 has neither.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    for (key, value) in [("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        git(p, &["config", key, value]);
    }
    write(p, "a.txt", "one\n");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "start"]);
    write(p, "a.txt", "one\nneedle\n");
    git(p, &["commit", "-q", "-am", "add needle to a"]);
    write(p, "b.txt", "other needle\n");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "add needle to b"]);
    write(p, "a.txt", "one\n");
    git(p, &["commit", "-q", "-am", "drop needle from a"]);
    dir
}

#[test]
fn a_search_lists_each_matching_commit_and_file_newest_first() {
    isolate_git!(a_search_lists_each_matching_commit_and_file_newest_first);
    let dir = fixture();
    let repo = Repo::open(dir.path()).expect("repo");
    let found = repo
        .search_changes(None, "needle", false, 100)
        .expect("search");
    let rows: Vec<(&str, &str)> = found
        .hits
        .iter()
        .map(|h| (h.summary.as_str(), h.path.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("drop needle from a", "a.txt"),
            ("add needle to b", "b.txt"),
            ("add needle to a", "a.txt"),
        ]
    );
    assert!(!found.truncated);
    // The oracle: the same commits in the same order.
    let oracle = git(dir.path(), &["log", "-Sneedle", "--format=%s"]);
    let want: Vec<&str> = oracle.lines().collect();
    let got: Vec<&str> = found.hits.iter().map(|h| h.summary.as_str()).collect();
    assert_eq!(got, want);
    assert_eq!(found.hits[0].author, "Ada Lovelace");
}

#[test]
fn a_regular_expression_matches_changed_lines_and_a_limit_truncates() {
    isolate_git!(a_regular_expression_matches_changed_lines_and_a_limit_truncates);
    let dir = fixture();
    let repo = Repo::open(dir.path()).expect("repo");
    let found = repo
        .search_changes(None, "ne+dle$", true, 100)
        .expect("search");
    assert_eq!(found.hits.len(), 3);
    let part = repo
        .search_changes(None, "needle", false, 2)
        .expect("search");
    assert_eq!(part.hits.len(), 2);
    assert!(part.truncated);
}

#[test]
fn text_that_looks_like_an_option_is_searched_for_not_run() {
    isolate_git!(text_that_looks_like_an_option_is_searched_for_not_run);
    let dir = fixture();
    let repo = Repo::open(dir.path()).expect("repo");
    let found = repo
        .search_changes(None, "--output=/nonexistent/x", false, 10)
        .expect("search");
    assert_eq!(found.hits, []);
    repo.search_changes(None, "", false, 10)
        .expect_err("an empty query");
}

/// A repository that names a textconv program for `*.txt` must not make the search run it.
#[cfg(unix)]
#[test]
fn a_repositorys_textconv_program_does_not_run() {
    isolate_git!(a_repositorys_textconv_program_does_not_run);
    let dir = fixture();
    let p = dir.path();
    let marker = p.join("textconv-ran");
    let script = p.join("conv.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\ncat \"$1\"\n", marker.display()),
    )
    .expect("script");
    git(
        p,
        &[
            "config",
            "diff.evil.textconv",
            &format!("sh {}", script.display()),
        ],
    );
    write(p, ".gitattributes", "*.txt diff=evil\n");
    let repo = Repo::open(p).expect("repo");
    let found = repo
        .search_changes(None, "needle", false, 10)
        .expect("search");
    assert_eq!(found.hits.len(), 3);
    assert!(!marker.exists(), "the textconv program ran");
}
