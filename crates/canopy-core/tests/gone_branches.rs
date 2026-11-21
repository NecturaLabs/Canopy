//! Deleting the local branches whose upstream was deleted on the remote, after a fetch that
//! prunes, against throwaway repositories with a bare remote. The git CLI, run by hand in a twin
//! repository, is the oracle for which branches go.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

mod support;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use support::isolate_git;

use canopy_core::{GitOptions, GoneBranch, Repo};

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
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

fn commit(p: &Path, file: &str, message: &str) {
    fs::write(p.join(file), format!("{message}\n")).expect("write");
    git(p, &["add", "--", file]);
    git(p, &["commit", "-q", "-m", message]);
}

/// A bare remote `remote.git` and a clone `work` beside it, in `root`. On the remote, these
/// branches were pushed with an upstream and then deleted, and the clone has not fetched since:
///
/// - `merged`: merged into `main`, so `git branch -d` deletes it;
/// - `unmerged`: has a commit of its own, so `-d` refuses it;
/// - `elsewhere`: merged, but checked out in the worktree `elsewhere-wt`;
/// - `master` and `develop`: merged, but long-lived.
///
/// `alive` is merged and still on the remote, and `local-only` is merged and has no upstream.
/// `main` is checked out.
fn scenario(root: &Path) -> PathBuf {
    let remote = root.join("remote.git");
    fs::create_dir(&remote).expect("mkdir");
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let work = root.join("work");
    fs::create_dir(&work).expect("mkdir");
    git(&work, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Ada Lovelace"),
        ("user.email", "ada@example.com"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(&work, &["config", key, value]);
    }
    git(
        &work,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    commit(&work, "a.txt", "feat: first");
    git(&work, &["push", "-q", "-u", "origin", "main"]);

    git(&work, &["switch", "-q", "-c", "merged"]);
    commit(&work, "m.txt", "feat: merged");
    git(&work, &["push", "-q", "-u", "origin", "merged"]);
    git(&work, &["switch", "-q", "main"]);
    git(&work, &["merge", "-q", "--ff-only", "merged"]);
    git(&work, &["push", "-q", "origin", "main"]);

    git(&work, &["switch", "-q", "-c", "unmerged"]);
    commit(&work, "u.txt", "feat: unmerged");
    git(&work, &["push", "-q", "-u", "origin", "unmerged"]);
    git(&work, &["switch", "-q", "main"]);

    for name in ["alive", "elsewhere", "master", "develop"] {
        git(&work, &["branch", name, "main"]);
        git(&work, &["push", "-q", "-u", "origin", name]);
    }
    git(&work, &["branch", "--no-track", "local-only", "main"]);
    git(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            &root.join("elsewhere-wt").to_string_lossy(),
            "elsewhere",
        ],
    );
    git(
        &remote,
        &[
            "branch",
            "-D",
            "merged",
            "unmerged",
            "elsewhere",
            "master",
            "develop",
        ],
    );
    work
}

/// Opens `work` making Canopy's own choices, whatever the developer's git configuration sets.
fn open(work: &Path) -> Repo {
    let repo = Repo::open(work).expect("open");
    repo.set_options(GitOptions {
        follow_config: false,
        ..GitOptions::default()
    });
    repo
}

fn branches(p: &Path) -> Vec<String> {
    git(p, &["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn gone(name: &str, remote: &str) -> GoneBranch {
    GoneBranch {
        name: name.to_owned(),
        remote: remote.to_owned(),
    }
}

fn fetch(repo: &Repo, prune: bool) {
    repo.fetch(None, prune, |_| {}).expect("fetch");
}

#[test]
fn only_merged_branches_whose_upstream_is_gone_are_deleted() {
    isolate_git!(only_merged_branches_whose_upstream_is_gone_are_deleted);
    let root = tempfile::tempdir().expect("tempdir");
    let work = scenario(root.path());
    let repo = open(&work);
    fetch(&repo, true);
    let done = repo.remove_gone_branches(None, true).expect("remove");
    assert_eq!(done.removed, [gone("merged", "origin")]);
    assert_eq!(
        branches(&work),
        [
            "alive",
            "develop",
            "elsewhere",
            "local-only",
            "main",
            "master",
            "unmerged"
        ]
    );
    // The branch git refused keeps its marker.
    let track = git(
        &work,
        &[
            "for-each-ref",
            "--format=%(upstream:track)",
            "refs/heads/unmerged",
        ],
    );
    assert_eq!(track.trim(), "[gone]");
    // Nothing is left to do the second time.
    let again = repo.remove_gone_branches(None, true).expect("remove");
    assert!(again.removed.is_empty(), "{again:?}");
}

/// The oracle: `git fetch --prune`, then `git branch -d` for each branch git marks `[gone]`,
/// leaving out the checked-out one, those in other worktrees and the long-lived ones.
#[test]
fn the_same_branches_go_as_with_the_git_cli() {
    isolate_git!(the_same_branches_go_as_with_the_git_cli);
    let ours = tempfile::tempdir().expect("tempdir");
    let theirs = tempfile::tempdir().expect("tempdir");
    let work = scenario(ours.path());
    let twin = scenario(theirs.path());

    let repo = open(&work);
    fetch(&repo, true);
    repo.remove_gone_branches(None, true).expect("remove");

    git(&twin, &["fetch", "-q", "--prune"]);
    let in_use: Vec<String> = git(&twin, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|l| l.strip_prefix("branch refs/heads/"))
        .map(str::to_owned)
        .collect();
    let listing = git(
        &twin,
        &[
            "for-each-ref",
            "--format=%(refname:short) %(upstream:track)",
            "refs/heads",
        ],
    );
    for line in listing.lines() {
        let Some(name) = line.strip_suffix(" [gone]") else {
            continue;
        };
        if in_use.iter().any(|b| b == name) || ["main", "master", "develop"].contains(&name) {
            continue;
        }
        // A refusal keeps the branch.
        git_output(&twin, &["branch", "-d", "--", name]);
    }
    assert_eq!(branches(&work), branches(&twin));
}

#[test]
fn a_checked_out_branch_whose_upstream_is_gone_stays() {
    isolate_git!(a_checked_out_branch_whose_upstream_is_gone_stays);
    let root = tempfile::tempdir().expect("tempdir");
    let work = scenario(root.path());
    git(&work, &["switch", "-q", "merged"]);
    let repo = open(&work);
    fetch(&repo, true);
    let done = repo.remove_gone_branches(None, true).expect("remove");
    assert!(done.removed.is_empty(), "{done:?}");
    assert!(branches(&work).contains(&"merged".to_owned()));
}

#[test]
fn nothing_goes_when_the_fetch_did_not_prune() {
    isolate_git!(nothing_goes_when_the_fetch_did_not_prune);
    let root = tempfile::tempdir().expect("tempdir");
    let work = scenario(root.path());
    let repo = open(&work);
    fetch(&repo, false);
    // Pruned before, by hand: a fetch that does not prune still deletes nothing.
    git(&work, &["remote", "prune", "origin"]);
    let done = repo.remove_gone_branches(None, false).expect("remove");
    assert!(done.removed.is_empty(), "{done:?}");
    assert!(branches(&work).contains(&"merged".to_owned()));
}

#[test]
fn nothing_goes_when_the_setting_is_off() {
    isolate_git!(nothing_goes_when_the_setting_is_off);
    let root = tempfile::tempdir().expect("tempdir");
    let work = scenario(root.path());
    let repo = open(&work);
    repo.set_options(GitOptions {
        follow_config: false,
        remove_gone_branches: false,
        ..GitOptions::default()
    });
    fetch(&repo, true);
    let done = repo.remove_gone_branches(None, true).expect("remove");
    assert!(done.removed.is_empty(), "{done:?}");
    assert!(branches(&work).contains(&"merged".to_owned()));
}

/// While Canopy follows the git configuration, `fetch.prune` and `remote.<name>.prune` decide
/// whether a fetch pruned, as git reads them; otherwise Canopy's flags decided.
#[test]
fn the_git_configuration_decides_whether_the_fetch_pruned() {
    isolate_git!(the_git_configuration_decides_whether_the_fetch_pruned);
    for (fetch_prune, remote_prune, follow, prune, expect_removed) in [
        (Some("false"), None, true, true, false),
        (Some("false"), Some("true"), true, false, true),
        (Some("true"), Some("false"), true, true, false),
        (Some("true"), None, true, false, true),
        (None, None, true, true, true),
        (Some("false"), None, false, true, true),
        (Some("true"), Some("true"), false, false, false),
    ] {
        let root = tempfile::tempdir().expect("tempdir");
        let work = scenario(root.path());
        if let Some(value) = fetch_prune {
            git(&work, &["config", "fetch.prune", value]);
        }
        if let Some(value) = remote_prune {
            git(&work, &["config", "remote.origin.prune", value]);
        }
        let repo = open(&work);
        repo.set_options(GitOptions {
            follow_config: follow,
            ..GitOptions::default()
        });
        fetch(&repo, prune);
        let pruned = git(&work, &["branch", "-r", "--list", "origin/merged"]).is_empty();
        assert_eq!(
            pruned, expect_removed,
            "the fixture's fetch: {fetch_prune:?} {remote_prune:?} {follow} {prune}"
        );
        let done = repo.remove_gone_branches(None, prune).expect("remove");
        let removed = done.removed == [gone("merged", "origin")];
        assert_eq!(
            removed, expect_removed,
            "{fetch_prune:?} {remote_prune:?} follow={follow} prune={prune}: {done:?}"
        );
        assert!(removed || done.removed.is_empty(), "{done:?}");
    }
}

#[test]
fn only_the_fetched_remote_counts() {
    isolate_git!(only_the_fetched_remote_counts);
    let root = tempfile::tempdir().expect("tempdir");
    let work = scenario(root.path());
    let mirror = root.path().join("mirror.git");
    fs::create_dir(&mirror).expect("mkdir");
    git(&mirror, &["init", "-q", "--bare", "-b", "main"]);
    git(
        &work,
        &["remote", "add", "mirror", &mirror.to_string_lossy()],
    );
    git(&work, &["branch", "mirrored", "main"]);
    git(&work, &["push", "-q", "-u", "mirror", "mirrored"]);
    git(&mirror, &["branch", "-D", "mirrored"]);
    let repo = open(&work);
    fetch(&repo, true);

    let done = repo
        .remove_gone_branches(Some("origin"), true)
        .expect("remove");
    assert_eq!(done.removed, [gone("merged", "origin")]);
    let done = repo
        .remove_gone_branches(Some("mirror"), true)
        .expect("remove");
    assert_eq!(done.removed, [gone("mirrored", "mirror")]);
}

#[test]
fn a_branch_tracking_a_local_branch_is_left_alone() {
    isolate_git!(a_branch_tracking_a_local_branch_is_left_alone);
    let root = tempfile::tempdir().expect("tempdir");
    let work = scenario(root.path());
    // Upstream `.`: the branch tracks the local `local-only`, which then goes.
    git(&work, &["branch", "--track", "follower", "local-only"]);
    git(&work, &["branch", "-D", "local-only"]);
    let repo = open(&work);
    fetch(&repo, true);
    let done = repo.remove_gone_branches(None, true).expect("remove");
    assert_eq!(done.removed, [gone("merged", "origin")]);
    assert!(branches(&work).contains(&"follower".to_owned()));
}
