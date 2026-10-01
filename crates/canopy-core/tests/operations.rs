//! End-to-end tests of `canopy-core` against throwaway repositories built with the `git` CLI.

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::ops::{HunkAction, ResetMode};
use canopy_core::{ChangeKind, DiffSource, LineKind, RefKind, Repo, RepoState};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with identity configured locally, so `Repo::commit` works without global config.
fn fixture() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Ada Lovelace"]);
    git(p, &["config", "user.email", "ada@example.com"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    git(p, &["config", "tag.gpgsign", "false"]);
    fs::write(
        p.join("a.txt"),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    )
    .expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: first"]);
    let repo = Repo::open(p).expect("open");
    (dir, repo)
}

#[test]
fn snapshot_reads_head_branches_and_graph() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "feature"]);
    fs::write(p.join("b.txt"), "b\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: b"]);
    git(p, &["switch", "-q", "main"]);
    fs::write(p.join("c.txt"), "c\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: c"]);
    git(p, &["merge", "-q", "--no-ff", "--no-edit", "feature"]);
    git(p, &["tag", "v1"]);

    let snap = repo.snapshot(100).expect("snapshot");
    assert_eq!(snap.head.branch.as_deref(), Some("main"));
    assert_eq!(snap.branches.len(), 2);
    assert!(snap.branches[0].is_head);
    assert_eq!(snap.commits.len(), 4);
    assert_eq!(snap.graph.len(), 4);
    assert!(snap.graph[0].is_merge);
    assert_eq!(snap.tags.len(), 1);
    let labels = &snap.refs[&snap.commits[0].oid];
    assert!(
        labels
            .iter()
            .any(|l| l.kind == RefKind::CurrentBranch && l.name == "main")
    );
    assert!(
        labels
            .iter()
            .any(|l| l.kind == RefKind::Tag && l.name == "v1")
    );
    assert!(!snap.truncated);
    assert!(repo.snapshot(2).expect("snapshot").truncated);
}

#[test]
fn stage_unstage_discard_and_commit() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::write(p.join("a.txt"), "changed\n").expect("write");
    fs::write(p.join("new.txt"), "new\n").expect("write");

    let st = repo.snapshot(10).expect("snapshot").status;
    assert_eq!(st.unstaged.len(), 2);
    assert!(
        st.unstaged
            .iter()
            .any(|f| f.path == "new.txt" && f.kind == ChangeKind::Untracked)
    );

    repo.stage(&["a.txt".into()]).expect("stage");
    let st = repo.snapshot(10).expect("snapshot").status;
    assert_eq!(st.staged.len(), 1);
    assert_eq!(st.staged[0].kind, ChangeKind::Modified);

    repo.unstage(&["a.txt".into()]).expect("unstage");
    assert!(
        repo.snapshot(10)
            .expect("snapshot")
            .status
            .staged
            .is_empty()
    );

    let untracked = repo.snapshot(10).expect("snapshot").status.unstaged;
    repo.discard(&untracked).expect("discard");
    assert!(repo.snapshot(10).expect("snapshot").status.is_clean());
    assert!(!p.join("new.txt").exists());

    fs::write(p.join("a.txt"), "again\n").expect("write");
    repo.stage_all().expect("stage all");
    let oid = repo
        .commit("fix: change a\n\nBody.", false)
        .expect("commit");
    let details = repo.commit_details(oid).expect("details");
    assert_eq!(details.message.trim(), "fix: change a\n\nBody.");
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].path, "a.txt");

    repo.commit("fix: reworded", true).expect("amend");
    assert_eq!(
        repo.head_message().expect("message").trim(),
        "fix: reworded"
    );
}

#[test]
fn hunk_staging_stages_only_the_chosen_hunk() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::write(
        p.join("a.txt"),
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    )
    .expect("write");

    let d = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 1)
        .expect("diff");
    assert_eq!(d.hunks.len(), 2, "{d:#?}");
    assert_eq!(d.additions, 2);
    assert!(
        d.hunks[0]
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Added && l.text == "ONE")
    );

    repo.apply_hunk(HunkAction::Stage, "a.txt", 1, d.hunks[1].key(), 1)
        .expect("stage hunk");
    let staged = repo
        .file_diff(&DiffSource::Staged, "a.txt", None, 1)
        .expect("staged");
    assert_eq!(staged.hunks.len(), 1);
    assert!(staged.hunks[0].lines.iter().any(|l| l.text == "TEN"));
    let unstaged = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 1)
        .expect("unstaged");
    assert_eq!(unstaged.hunks.len(), 1);
    assert!(unstaged.hunks[0].lines.iter().any(|l| l.text == "ONE"));

    repo.apply_hunk(HunkAction::Unstage, "a.txt", 0, staged.hunks[0].key(), 1)
        .expect("unstage hunk");
    assert!(
        repo.file_diff(&DiffSource::Staged, "a.txt", None, 1)
            .expect("staged")
            .hunks
            .is_empty()
    );

    let d = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 1)
        .expect("diff");
    repo.apply_hunk(HunkAction::Discard, "a.txt", 0, d.hunks[0].key(), 1)
        .expect("discard hunk");
    let left = fs::read_to_string(p.join("a.txt")).expect("read");
    assert!(
        left.starts_with("one\n") && left.ends_with("TEN\n"),
        "{left}"
    );

    // A stale key is refused rather than applying the wrong hunk.
    let err = repo.apply_hunk(HunkAction::Stage, "a.txt", 0, (9, 9, 9, 9), 1);
    assert!(err.is_err());
}

#[test]
fn branches_tags_stash_reset() {
    let (dir, repo) = fixture();
    let p = dir.path();
    repo.create_branch("topic", None, true).expect("branch");
    assert_eq!(repo.head().expect("head").branch.as_deref(), Some("topic"));
    repo.rename_branch("topic", "topic2").expect("rename");
    repo.checkout_branch("main").expect("checkout");
    repo.delete_branch("topic2", false).expect("delete");
    assert_eq!(repo.branches().expect("branches").len(), 1);

    let head = repo.head().expect("head").oid.expect("oid");
    repo.create_tag("v0.1.0", head, Some("First release"))
        .expect("tag");
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.tags[0].message.as_deref(), Some("First release"));
    repo.delete_tag("v0.1.0").expect("delete tag");

    fs::write(p.join("a.txt"), "stashed\n").expect("write");
    repo.stash_save(Some("wip"), false).expect("stash");
    let snap = repo.snapshot(10).expect("snapshot");
    assert!(snap.status.is_clean());
    assert_eq!(snap.stashes.len(), 1);
    assert!(snap.stashes[0].message.contains("wip"));
    repo.stash_apply(0, true).expect("pop");
    let snap = repo.snapshot(10).expect("snapshot");
    assert!(snap.stashes.is_empty());
    assert_eq!(snap.status.unstaged.len(), 1);

    repo.stage_all().expect("stage");
    let second = repo.commit("feat: second", false).expect("commit");
    repo.reset(head, ResetMode::Soft).expect("reset");
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.head.oid, Some(head));
    assert_eq!(snap.status.staged.len(), 1);
    repo.reset(second, ResetMode::Hard).expect("reset hard");
    assert!(repo.snapshot(10).expect("snapshot").status.is_clean());
}

#[test]
fn merge_conflict_is_reported_and_abortable() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "other"]);
    fs::write(p.join("a.txt"), "theirs\n").expect("write");
    git(p, &["commit", "-q", "-am", "theirs"]);
    git(p, &["switch", "-q", "main"]);
    fs::write(p.join("a.txt"), "ours\n").expect("write");
    git(p, &["commit", "-q", "-am", "ours"]);

    assert!(repo.merge("other", false).is_err());
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.state, RepoState::Merge);
    assert_eq!(snap.status.conflicted.len(), 1);
    let view = repo.conflict_view("a.txt").expect("view");
    assert!(
        view.hunks[0]
            .lines
            .iter()
            .any(|l| l.text.starts_with("<<<<<<<"))
    );

    repo.continue_or_abort(RepoState::Merge, true)
        .expect("abort");
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.state, RepoState::Clean);
    assert!(snap.status.is_clean());
}

#[test]
fn cherry_pick_and_revert() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "side"]);
    fs::write(p.join("side.txt"), "side\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: side"]);
    let side = repo.head().expect("head").oid.expect("oid");
    git(p, &["switch", "-q", "main"]);

    repo.cherry_pick(side).expect("cherry-pick");
    assert!(p.join("side.txt").exists());
    let picked = repo.head().expect("head").oid.expect("oid");
    repo.revert(picked).expect("revert");
    assert!(!p.join("side.txt").exists());
}

#[test]
fn push_and_fetch_against_a_local_remote() {
    let (dir, repo) = fixture();
    let remote = tempfile::tempdir().expect("tempdir");
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    let url = remote.path().to_string_lossy().into_owned();
    repo.add_remote("origin", &url).expect("remote");

    let mut lines = Vec::new();
    repo.push(false, |l| lines.push(l.to_owned()))
        .expect("push");
    let snap = repo.snapshot(10).expect("snapshot");
    let main = snap.current_branch().expect("main");
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));
    assert_eq!((main.ahead, main.behind), (0, 0));

    fs::write(dir.path().join("a.txt"), "ahead\n").expect("write");
    repo.stage_all().expect("stage");
    repo.commit("feat: ahead", false).expect("commit");
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.current_branch().expect("main").ahead, 1);

    repo.fetch(None, |_| {}).expect("fetch");
    repo.pull(false, |_| {}).expect("pull");
    repo.push(false, |_| {}).expect("push again");
    assert_eq!(
        repo.snapshot(10)
            .expect("snapshot")
            .current_branch()
            .expect("main")
            .ahead,
        0
    );

    let clone_dir = tempfile::tempdir().expect("tempdir");
    let dest = clone_dir.path().join("copy");
    canopy_core::ops::clone(&url, &dest, |_| {}).expect("clone");
    let copy = Repo::open(&dest).expect("open clone");
    assert_eq!(copy.snapshot(10).expect("snapshot").commits.len(), 2);
}
