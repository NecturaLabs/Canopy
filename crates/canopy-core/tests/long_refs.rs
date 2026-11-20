//! A ref whose name is longer than libgit2 accepts (git takes it) leaves the rest of the
//! repository readable, against throwaway repositories (#270).

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

mod support;

use std::ffi::OsStr;
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
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let out = Command::new("git")
        .current_dir(dir)
        .args(&args)
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A name of `parts` path components of 190 characters each, so no component passes a file
/// system's 255-byte limit and git can keep it as a loose ref.
fn long_name(lead: &str, parts: usize) -> String {
    (0..parts)
        .map(|i| format!("{lead}{i}-{}", "x".repeat(190)))
        .collect::<Vec<_>>()
        .join("/")
}

fn commit_on(p: &Path, branch: &str, message: &str) -> String {
    git(p, &["switch", "-q", "-c", branch, "main"]);
    fs::write(p.join("f"), format!("{message}\n")).expect("write");
    git(p, &["add", "f"]);
    git(p, &["commit", "-q", "-m", message]);
    let id = git(p, &["rev-parse", "HEAD"]).trim().to_owned();
    git(p, &["switch", "-q", "main"]);
    id
}

/// A packed ref has no file of its own, so its name can be one long component, on every
/// platform: `from` (a ref in `.git/packed-refs`) is renamed `to` there, keeping the file's order.
fn rename_packed(p: &Path, from: &str, to: &str) {
    let refs = fs::read_to_string(p.join(".git/packed-refs")).expect("packed-refs");
    assert!(refs.contains(from), "{from} is packed");
    fs::write(p.join(".git/packed-refs"), refs.replace(from, to)).expect("write packed-refs");
}

/// With a branch and an annotated tag whose names are over libgit2's 1024-byte limit, each the
/// only ref to its commit, the history, snapshot, branches and Insights read and include those
/// commits. Git keeps such names packed on every platform.
#[test]
fn a_ref_name_over_libgit2s_limit_leaves_the_repository_readable() {
    isolate_git!(a_ref_name_over_libgit2s_limit_leaves_the_repository_readable);
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    fs::write(p.join("a"), "a\n").expect("write");
    git(p, &["add", "a"]);
    git(p, &["commit", "-q", "-m", "first"]);
    let packed_id = commit_on(p, "to-pack", "only the packed ref");
    let tag_id = commit_on(p, "to-tag", "only the tag");
    git(p, &["tag", "-a", "-m", "long", "tag-to-pack", "to-tag"]);
    git(p, &["branch", "-q", "-D", "to-tag"]);
    git(p, &["pack-refs", "--all"]);
    // Both stay in the file's order: "main" < "packed-…", and the only tag.
    rename_packed(
        p,
        "refs/heads/to-pack",
        &format!("refs/heads/packed-{}", "p".repeat(6000)),
    );
    rename_packed(
        p,
        "refs/tags/tag-to-pack",
        &format!("refs/tags/tag-{}", "t".repeat(3400)),
    );
    git(p, &["branch", "ordinary"]);
    let listed = git(p, &["for-each-ref", "--format=%(refname)"]);
    assert_eq!(
        listed.lines().filter(|l| l.len() > 3400).count(),
        2,
        "git keeps both: {listed}"
    );

    let repo = Repo::open(p).expect("open");
    let (commits, _) = repo.commits(100).expect("the history reads");
    for (id, what) in [(&packed_id, "branch"), (&tag_id, "tag")] {
        assert!(
            commits.iter().any(|c| c.oid.to_string() == *id),
            "the commit only the long {what} reaches is in the history"
        );
    }
    assert_eq!(commits.len(), 3);
    repo.snapshot(100).expect("the snapshot reads");
    let branches = repo.branches().expect("branches read");
    assert!(branches.iter().any(|b| b.name == "ordinary"));
    let metrics = repo.repository_metrics().expect("insights read");
    assert_eq!(metrics.weeks.iter().map(|w| w.commits).sum::<u64>(), 3);
}

/// Linux keeps paths of any length, so git can also store a long name as a loose ref (macOS and
/// Windows refuse the path); and a folder of refs deeper than libgit2 lists (100 levels) outside
/// the branches, remote branches and tags hides none of their commits.
#[cfg(target_os = "linux")]
#[test]
fn a_long_loose_ref_and_a_deep_ref_folder_leave_the_history_whole() {
    isolate_git!(a_long_loose_ref_and_a_deep_ref_folder_leave_the_history_whole);
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    fs::write(p.join("a"), "a\n").expect("write");
    git(p, &["add", "a"]);
    git(p, &["commit", "-q", "-m", "first"]);
    let loose = long_name("loose", 17);
    let loose_id = commit_on(p, &loose, "only the loose ref");
    let tag_id = commit_on(p, "to-tag", "only the tag");
    git(p, &["tag", "kept", "to-tag"]);
    git(p, &["branch", "-q", "-D", "to-tag"]);
    let deep = (0..101).map(|_| "d").collect::<Vec<_>>().join("/");
    git(p, &["update-ref", &format!("refs/notes/{deep}/x"), "main"]);

    let repo = Repo::open(p).expect("open");
    let (commits, _) = repo.commits(100).expect("the history reads");
    for (id, what) in [(&loose_id, "long loose branch"), (&tag_id, "tag")] {
        assert!(
            commits.iter().any(|c| c.oid.to_string() == *id),
            "the commit only the {what} reaches is in the history"
        );
    }
    let branches = repo.branches().expect("branches read");
    assert!(branches.iter().any(|b| b.name == loose));
    let metrics = repo.repository_metrics().expect("insights read");
    assert_eq!(metrics.weeks.iter().map(|w| w.commits).sum::<u64>(), 3);
}
