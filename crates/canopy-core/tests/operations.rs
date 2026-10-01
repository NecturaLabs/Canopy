//! End-to-end tests of `canopy-core` against throwaway repositories built with the `git` CLI.

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::ops::{HunkAction, ResetMode};
use canopy_core::{
    ChangeKind, DiffSource, LineKind, PullStrategy, RefKind, Repo, RepoState, Whitespace,
};

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
        .commit("fix: change a\n\nBody.", false, false)
        .expect("commit");
    let details = repo.commit_details(oid).expect("details");
    assert_eq!(details.message.trim(), "fix: change a\n\nBody.");
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].path, "a.txt");

    repo.commit("fix: reworded", true, false).expect("amend");
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
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 1, Whitespace::Show)
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
        .file_diff(&DiffSource::Staged, "a.txt", None, 1, Whitespace::Show)
        .expect("staged");
    assert_eq!(staged.hunks.len(), 1);
    assert!(staged.hunks[0].lines.iter().any(|l| l.text == "TEN"));
    let unstaged = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 1, Whitespace::Show)
        .expect("unstaged");
    assert_eq!(unstaged.hunks.len(), 1);
    assert!(unstaged.hunks[0].lines.iter().any(|l| l.text == "ONE"));

    repo.apply_hunk(HunkAction::Unstage, "a.txt", 0, staged.hunks[0].key(), 1)
        .expect("unstage hunk");
    assert!(
        repo.file_diff(&DiffSource::Staged, "a.txt", None, 1, Whitespace::Show)
            .expect("staged")
            .hunks
            .is_empty()
    );

    let d = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 1, Whitespace::Show)
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
    let second = repo.commit("feat: second", false, false).expect("commit");
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
    repo.commit("feat: ahead", false, false).expect("commit");
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.current_branch().expect("main").ahead, 1);

    repo.fetch(None, true, |_| {}).expect("fetch");
    repo.pull(PullStrategy::Merge, |_| {}).expect("pull");
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

/// Gives a repository the fixed test identity and turns off signing and automatic pruning, so
/// tests do not depend on the user's global configuration.
fn configure(p: &Path) {
    git(p, &["config", "user.name", "Ada Lovelace"]);
    git(p, &["config", "user.email", "ada@example.com"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    git(p, &["config", "tag.gpgsign", "false"]);
    git(p, &["config", "fetch.prune", "false"]);
}

/// The fixture as upstream plus a clone of it made with `ops::clone`.
fn upstream_and_clone() -> (tempfile::TempDir, Repo, tempfile::TempDir, Repo) {
    let (up_dir, up) = fixture();
    let clone_dir = tempfile::tempdir().expect("tempdir");
    let dest = clone_dir.path().join("clone");
    canopy_core::ops::clone(&up_dir.path().to_string_lossy(), &dest, |_| {}).expect("clone");
    configure(&dest);
    let copy = Repo::open(&dest).expect("open clone");
    (up_dir, up, clone_dir, copy)
}

/// Upstream and the clone each gain one commit, so their histories diverge.
fn diverge(up: &Path, copy: &Repo) {
    fs::write(up.join("up.txt"), "upstream\n").expect("write");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "feat: upstream"]);
    fs::write(copy.workdir().join("local.txt"), "local\n").expect("write");
    copy.stage_all().expect("stage");
    copy.commit("feat: local", false, false).expect("commit");
}

fn head_parents(repo: &Repo) -> usize {
    repo.git()
        .head()
        .expect("head")
        .peel_to_commit()
        .expect("commit")
        .parent_count()
}

#[test]
fn commit_signoff_adds_a_trailer() {
    let (dir, repo) = fixture();
    fs::write(dir.path().join("a.txt"), "signed\n").expect("write");
    repo.stage_all().expect("stage");
    repo.commit("feat: signed\n\nBody.", false, true)
        .expect("commit");
    let msg = repo.head_message().expect("message");
    assert_eq!(
        msg.trim_end(),
        "feat: signed\n\nBody.\n\nSigned-off-by: Ada Lovelace <ada@example.com>"
    );

    // Amending without signoff keeps the message as given; with signoff the trailer is added
    // once, as `git commit --amend --signoff` does.
    repo.commit("feat: plain", true, false).expect("amend");
    assert!(
        !repo
            .head_message()
            .expect("message")
            .contains("Signed-off-by")
    );
    repo.commit("feat: again", true, true)
        .expect("amend signoff");
    let msg = repo.head_message().expect("message");
    assert_eq!(
        msg.matches("Signed-off-by: Ada Lovelace").count(),
        1,
        "{msg}"
    );
}

#[test]
fn whitespace_modes_hide_whitespace_only_changes() {
    let (dir, repo) = fixture();
    let p = dir.path();
    // Trailing spaces on line 2, new indentation on line 9.
    fs::write(
        p.join("a.txt"),
        "one\ntwo   \nthree\nfour\nfive\nsix\nseven\neight\n    nine\nten\n",
    )
    .expect("write");

    let hunks = |ws: Whitespace| {
        repo.file_diff(&DiffSource::Unstaged, "a.txt", None, 1, ws)
            .expect("diff")
            .hunks
            .len()
    };
    let cli_hunks = |flag: &str| {
        let mut args = vec!["diff", "-U1"];
        if !flag.is_empty() {
            args.push(flag);
        }
        git(p, &args)
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count()
    };
    assert_eq!(hunks(Whitespace::Show), 2);
    assert_eq!(hunks(Whitespace::Show), cli_hunks(""));
    assert_eq!(hunks(Whitespace::IgnoreEol), 1);
    assert_eq!(
        hunks(Whitespace::IgnoreEol),
        cli_hunks("--ignore-space-at-eol")
    );
    let eol = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            1,
            Whitespace::IgnoreEol,
        )
        .expect("diff");
    assert!(eol.hunks[0].lines.iter().any(|l| l.text == "    nine"));
    assert_eq!(hunks(Whitespace::IgnoreAll), 0);
    assert_eq!(hunks(Whitespace::IgnoreAll), cli_hunks("-w"));

    // Staged and commit diffs honour the mode too.
    repo.stage_all().expect("stage");
    let staged = |ws| {
        repo.file_diff(&DiffSource::Staged, "a.txt", None, 1, ws)
            .expect("diff")
            .hunks
            .len()
    };
    assert_eq!(
        (staged(Whitespace::Show), staged(Whitespace::IgnoreAll)),
        (2, 0)
    );
    let oid = repo
        .commit("style: whitespace", false, false)
        .expect("commit");
    let commit = |ws| {
        repo.file_diff(&DiffSource::Commit(oid), "a.txt", None, 1, ws)
            .expect("diff")
            .hunks
            .len()
    };
    assert_eq!(
        (
            commit(Whitespace::Show),
            commit(Whitespace::IgnoreEol),
            commit(Whitespace::IgnoreAll)
        ),
        (2, 1, 0)
    );
}

#[test]
fn whitespace_labels() {
    assert_eq!(Whitespace::default(), Whitespace::Show);
    let labels: Vec<&str> = Whitespace::ALL.iter().map(|w| w.label()).collect();
    assert_eq!(
        labels,
        ["Show whitespace", "Ignore at end of line", "Ignore all"]
    );
}

#[test]
fn pull_strategy_labels() {
    assert_eq!(PullStrategy::default(), PullStrategy::Merge);
    let labels: Vec<&str> = PullStrategy::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(labels, ["Merge", "Rebase", "Fast-forward only"]);
}

#[test]
fn pull_fast_forward_only_refuses_diverged_branches() {
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    diverge(up_dir.path(), &copy);
    let before = copy.head().expect("head").oid;
    let err = copy
        .pull(PullStrategy::FastForwardOnly, |_| {})
        .expect_err("diverged");
    match err {
        canopy_core::Error::Cli { command, stderr } => {
            assert_eq!(command, "pull");
            assert!(!stderr.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
    let snap = copy.snapshot(10).expect("snapshot");
    assert_eq!(snap.head.oid, before);
    assert_eq!(snap.state, RepoState::Clean);
    assert!(snap.status.is_clean());
}

#[test]
fn pull_fast_forward_only_fast_forwards() {
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    let up = up_dir.path();
    fs::write(up.join("up.txt"), "upstream\n").expect("write");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "feat: upstream"]);
    copy.pull(PullStrategy::FastForwardOnly, |_| {})
        .expect("pull");
    let up_head = git(up, &["rev-parse", "HEAD"]);
    assert_eq!(
        copy.head().expect("head").oid.map(|o| o.to_string()),
        Some(up_head.trim().to_owned())
    );
    assert!(copy.workdir().join("up.txt").exists());
}

#[test]
fn pull_rebase_replays_local_commits() {
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    diverge(up_dir.path(), &copy);
    copy.pull(PullStrategy::Rebase, |_| {}).expect("pull");
    assert_eq!(head_parents(&copy), 1);
    let parent = git(copy.workdir(), &["rev-parse", "HEAD^"]);
    let up_head = git(up_dir.path(), &["rev-parse", "HEAD"]);
    assert_eq!(parent, up_head);
    assert_eq!(copy.head_message().expect("message").trim(), "feat: local");
    assert!(copy.workdir().join("up.txt").exists());
    assert!(copy.workdir().join("local.txt").exists());
}

#[test]
fn pull_merge_creates_a_merge_commit() {
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    diverge(up_dir.path(), &copy);
    copy.pull(PullStrategy::Merge, |_| {}).expect("pull");
    assert_eq!(head_parents(&copy), 2);
    assert!(copy.workdir().join("up.txt").exists());
}

#[test]
fn pull_follows_configured_pull_rebase() {
    // pull.rebase=true wins over a Merge strategy...
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    git(copy.workdir(), &["config", "pull.rebase", "true"]);
    diverge(up_dir.path(), &copy);
    copy.pull(PullStrategy::Merge, |_| {}).expect("pull");
    assert_eq!(head_parents(&copy), 1);

    // ...and pull.rebase=false over a Rebase strategy.
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    git(copy.workdir(), &["config", "pull.rebase", "false"]);
    diverge(up_dir.path(), &copy);
    copy.pull(PullStrategy::Rebase, |_| {}).expect("pull");
    assert_eq!(head_parents(&copy), 2);
}

#[test]
fn fetch_prunes_only_when_asked() {
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    let up = up_dir.path();
    git(up, &["branch", "doomed"]);
    git(up, &["tag", "v-new"]);
    copy.fetch(Some("origin"), false, |_| {}).expect("fetch");
    let has = |name: &str| copy.git().find_reference(name).is_ok();
    assert!(has("refs/remotes/origin/doomed"));
    assert!(has("refs/tags/v-new"), "tags are fetched");

    git(up, &["branch", "-D", "doomed"]);
    copy.fetch(Some("origin"), false, |_| {}).expect("fetch");
    assert!(has("refs/remotes/origin/doomed"), "kept without prune");
    copy.fetch(None, true, |_| {}).expect("fetch --prune");
    assert!(!has("refs/remotes/origin/doomed"), "pruned");
    assert!(has("refs/remotes/origin/main"));
}

#[test]
fn remotes_report_the_default_branch() {
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    let origin = |repo: &Repo| {
        repo.snapshot(10)
            .expect("snapshot")
            .remotes
            .into_iter()
            .find(|r| r.name == "origin")
            .expect("origin")
    };
    assert_eq!(origin(&copy).default_branch.as_deref(), Some("main"));

    // `git remote set-head` moves it; deleting it clears it.
    git(up_dir.path(), &["branch", "develop"]);
    copy.fetch(Some("origin"), false, |_| {}).expect("fetch");
    git(copy.workdir(), &["remote", "set-head", "origin", "develop"]);
    assert_eq!(origin(&copy).default_branch.as_deref(), Some("develop"));
    git(copy.workdir(), &["remote", "set-head", "origin", "-d"]);
    assert_eq!(origin(&copy).default_branch, None);

    // A remote added by hand and pushed to has no HEAD of its own.
    let (_dir, repo) = fixture();
    let bare = tempfile::tempdir().expect("tempdir");
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    repo.add_remote("origin", &bare.path().to_string_lossy())
        .expect("remote");
    repo.push(false, |_| {}).expect("push");
    let remote = origin(&repo);
    assert!(remote.url.is_some());
    assert_eq!(remote.default_branch, None);
}

#[test]
fn checkout_request_fetches_and_fast_forwards_the_request_branch() {
    // A working repository pushes `main` and a request head to a bare "server".
    let (work_dir, _work) = fixture();
    let work = work_dir.path();
    let server = tempfile::tempdir().expect("tempdir");
    git(server.path(), &["init", "-q", "--bare", "-b", "main"]);
    let server_url = server.path().to_string_lossy().into_owned();
    git(work, &["push", "-q", &server_url, "main"]);
    git(work, &["switch", "-q", "-c", "contrib"]);
    fs::write(work.join("pr.txt"), "one\n").expect("write");
    git(work, &["add", "."]);
    git(work, &["commit", "-q", "-m", "feat: request"]);
    git(
        work,
        &["push", "-q", &server_url, "contrib:refs/pull/1/head"],
    );
    let rev = |dir: &Path, r: &str| git(dir, &["rev-parse", r]).trim().to_owned();
    let first = rev(work, "HEAD");

    let clone_dir = tempfile::tempdir().expect("tempdir");
    let dest = clone_dir.path().join("clone");
    canopy_core::ops::clone(&server_url, &dest, |_| {}).expect("clone");
    configure(&dest);
    let repo = Repo::open(&dest).expect("open");
    let head = |repo: &Repo| {
        let h = repo.head().expect("head");
        (h.branch, h.oid.map(|o| o.to_string()))
    };

    // New branch: created at the request head, with no upstream.
    repo.checkout_request("origin", "refs/pull/1/head", "pr/1", |_| {})
        .expect("checkout request");
    assert_eq!(head(&repo), (Some("pr/1".into()), Some(first.clone())));
    assert_eq!(rev(&dest, "refs/remotes/origin/pr/1"), first);
    let pr = repo
        .branches()
        .expect("branches")
        .into_iter()
        .find(|b| b.name == "pr/1")
        .expect("pr/1");
    assert_eq!(pr.upstream, None);
    assert!(dest.join("pr.txt").exists());

    // The request advances; checking it out again from another branch fast-forwards it.
    fs::write(work.join("pr.txt"), "two\n").expect("write");
    git(work, &["commit", "-q", "-am", "feat: request update"]);
    git(
        work,
        &["push", "-q", &server_url, "contrib:refs/pull/1/head"],
    );
    let second = rev(work, "HEAD");
    repo.checkout_branch("main").expect("switch to main");
    repo.checkout_request("origin", "refs/pull/1/head", "pr/1", |_| {})
        .expect("update request");
    assert_eq!(head(&repo), (Some("pr/1".into()), Some(second.clone())));
    assert_eq!(
        fs::read_to_string(dest.join("pr.txt")).expect("read"),
        "two\n"
    );

    // Local commits on the request branch are never thrown away: a rewritten request head
    // cannot be fast-forwarded and git's error comes back.
    fs::write(dest.join("mine.txt"), "mine\n").expect("write");
    repo.stage_all().expect("stage");
    let mine = repo.commit("feat: mine", false, false).expect("commit");
    fs::write(work.join("pr.txt"), "rewritten\n").expect("write");
    git(
        work,
        &["commit", "-q", "-a", "--amend", "-m", "feat: rewritten"],
    );
    git(
        work,
        &["push", "-q", "-f", &server_url, "contrib:refs/pull/1/head"],
    );
    match repo.checkout_request("origin", "refs/pull/1/head", "pr/1", |_| {}) {
        Err(canopy_core::Error::Cli { command, stderr }) => {
            assert_eq!(command, "merge");
            assert!(!stderr.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(head(&repo), (Some("pr/1".into()), Some(mine.to_string())));
    assert!(repo.snapshot(10).expect("snapshot").status.is_clean());
    // The tracking ref still moved to the rewritten head (it is forced, like a fetch).
    assert_eq!(rev(&dest, "refs/remotes/origin/pr/1"), rev(work, "HEAD"));
}

#[test]
fn checkout_request_rejects_bad_names_without_fetching() {
    let (_dir, repo) = fixture();
    let before = repo.head().expect("head");
    for (remote, remote_ref, branch) in [
        ("origin", "refs/pull/1/head", "bad..name"),
        ("origin", "refs/pull/1/head", "-x"),
        ("origin", "refs/pull/1/head", "@{-1}"),
        ("origin", "refs/pull/1/head", ""),
        ("origin", "refs/pull/1/head", "a b"),
        ("origin", "pull/1/head", "pr/1"),
        ("origin", "refs/pull/1/head..x", "pr/1"),
        ("origin", "--upload-pack=evil", "pr/1"),
        ("-oops", "refs/pull/1/head", "pr/1"),
        ("", "refs/pull/1/head", "pr/1"),
    ] {
        match repo.checkout_request(remote, remote_ref, branch, |_| {}) {
            Err(canopy_core::Error::Invalid(_)) => {}
            other => panic!("{remote:?} {remote_ref:?} {branch:?}: {other:?}"),
        }
    }
    assert_eq!(repo.head().expect("head"), before);
    assert_eq!(repo.branches().expect("branches").len(), 1);
}
