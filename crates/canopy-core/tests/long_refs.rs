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

/// With a packed branch, a loose branch and a tag, each over libgit2's 1024-byte limit and the
/// only ref to its commit, the history, snapshot, branches and Insights read and include those
/// commits.
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
    git(p, &["pack-refs", "--all"]);
    // A packed ref has no file of its own, so its name can be one long component.
    let refs = fs::read_to_string(p.join(".git/packed-refs")).expect("packed-refs");
    let packed = format!("packed-{}", "p".repeat(6000));
    fs::write(
        p.join(".git/packed-refs"),
        refs.replace("refs/heads/to-pack", &format!("refs/heads/{packed}")),
    )
    .expect("write packed-refs");
    let loose = long_name("loose", 17);
    let loose_id = commit_on(p, &loose, "only the loose ref");
    let tag_id = commit_on(p, "to-tag", "only the tag");
    git(
        p,
        &["tag", "-a", "-m", "long", &long_name("tag", 17), "to-tag"],
    );
    git(p, &["branch", "-q", "-D", "to-tag"]);
    git(p, &["branch", "ordinary"]);
    let listed = git(p, &["for-each-ref", "--format=%(refname)"]);
    assert!(
        listed.lines().any(|l| l.len() > 6000),
        "git keeps the packed ref: {listed}"
    );

    let repo = Repo::open(p).expect("open");
    let (commits, _) = repo.commits(100).expect("the history reads");
    for (id, what) in [
        (&packed_id, "packed"),
        (&loose_id, "loose"),
        (&tag_id, "tag"),
    ] {
        assert!(
            commits.iter().any(|c| c.oid.to_string() == *id),
            "the commit only the long {what} ref reaches is in the history"
        );
    }
    assert_eq!(commits.len(), 4);
    repo.snapshot(100).expect("the snapshot reads");
    let branches = repo.branches().expect("branches read");
    assert!(branches.iter().any(|b| b.name == "ordinary"));
    assert!(branches.iter().any(|b| b.name == loose));
    let metrics = repo.repository_metrics().expect("insights read");
    assert_eq!(metrics.weeks.iter().map(|w| w.commits).sum::<u64>(), 4);
}
