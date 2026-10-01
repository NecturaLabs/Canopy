//! End-to-end tests of `canopy-core` against throwaway repositories built with the `git` CLI.

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::ops::{HunkAction, MergeMode, ResetMode};
use canopy_core::{
    ChangeKind, Choice, ConflictKind, ConflictRegion, DiffSource, LineKind, PullStrategy, RefKind,
    Repo, RepoState, Side, Whitespace,
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

/// The index blob of `path` (`git show :path`).
fn index_blob(dir: &Path, path: &str) -> String {
    git(dir, &["show", &format!(":{path}")])
}

#[test]
fn hunk_staging_works_without_context_lines() {
    // With context 0 every hunk sits in the middle of the file with nothing around it to anchor
    // it; git apply needs --unidiff-zero for that (recorded with git 2.43: `git diff -U0 >p;
    // git apply --cached p` fails with "patch failed", `--unidiff-zero` applies it).
    let (dir, repo) = fixture();
    let p = dir.path();
    let original = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    fs::write(
        p.join("a.txt"),
        "ONE\ntwo\nthree\nfour\nFIVE\nsix\nseven\neight\nnine\nTEN\n",
    )
    .expect("write");
    let d = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 0, Whitespace::Show)
        .expect("diff");
    assert_eq!(d.hunks.len(), 3, "{d:#?}");

    repo.apply_hunk(HunkAction::Stage, "a.txt", 1, d.hunks[1].key(), 0)
        .expect("stage the middle hunk");
    assert_eq!(
        index_blob(p, "a.txt"),
        "one\ntwo\nthree\nfour\nFIVE\nsix\nseven\neight\nnine\nten\n"
    );

    let staged = repo
        .file_diff(&DiffSource::Staged, "a.txt", None, 0, Whitespace::Show)
        .expect("staged");
    assert_eq!(staged.hunks.len(), 1);
    repo.apply_hunk(HunkAction::Unstage, "a.txt", 0, staged.hunks[0].key(), 0)
        .expect("unstage the middle hunk");
    assert_eq!(index_blob(p, "a.txt"), original);

    let d = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 0, Whitespace::Show)
        .expect("diff");
    repo.apply_hunk(HunkAction::Discard, "a.txt", 1, d.hunks[1].key(), 0)
        .expect("discard the middle hunk");
    assert_eq!(
        fs::read_to_string(p.join("a.txt")).expect("read"),
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n"
    );
    assert_eq!(index_blob(p, "a.txt"), original);

    // A pure insertion in the middle (`@@ -5,0 +6 @@`) and a pure deletion work too.
    fs::write(
        p.join("a.txt"),
        "one\ntwo\nthree\nfour\nfive\nNEW\nsix\nseven\nnine\nten\n",
    )
    .expect("write");
    let d = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 0, Whitespace::Show)
        .expect("diff");
    assert_eq!(d.hunks.len(), 2, "{d:#?}");
    repo.apply_hunk(HunkAction::Stage, "a.txt", 0, d.hunks[0].key(), 0)
        .expect("stage the insertion");
    assert_eq!(
        index_blob(p, "a.txt"),
        "one\ntwo\nthree\nfour\nfive\nNEW\nsix\nseven\neight\nnine\nten\n"
    );
    let d = repo
        .file_diff(&DiffSource::Unstaged, "a.txt", None, 0, Whitespace::Show)
        .expect("diff");
    repo.apply_hunk(HunkAction::Stage, "a.txt", 0, d.hunks[0].key(), 0)
        .expect("stage the deletion");
    assert_eq!(
        index_blob(p, "a.txt"),
        "one\ntwo\nthree\nfour\nfive\nNEW\nsix\nseven\nnine\nten\n"
    );
}

#[test]
fn hunk_staging_handles_unusual_paths() {
    // `git diff` C-quotes names with quotes, control characters or non-ASCII bytes and ends a
    // name containing a space with a tab on the ---/+++ lines; the hunk patch must do the same.
    let (dir, repo) = fixture();
    let p = dir.path();
    let mut names = vec!["my notes.txt", "caf\u{e9} \"q\".txt", "back\\slash.txt"];
    if cfg!(unix) {
        // Windows forbids tabs and backslashes in file names.
        names.push("tab\there.txt");
    } else {
        names.retain(|n| !n.contains('\\'));
    }
    for name in names {
        fs::write(p.join(name), "1\n2\n3\n4\n5\n6\n7\n8\n9\n").expect("write");
        git(p, &["add", "--", name]);
        git(p, &["commit", "-q", "-m", "test: add file"]);
        fs::write(p.join(name), "1\n2\nTHREE\n4\n5\n6\n7\nEIGHT\n9\n").expect("write");
        let d = repo
            .file_diff(&DiffSource::Unstaged, name, None, 1, Whitespace::Show)
            .expect("diff");
        assert_eq!(d.hunks.len(), 2, "{name}: {d:#?}");
        repo.apply_hunk(HunkAction::Stage, name, 1, d.hunks[1].key(), 1)
            .unwrap_or_else(|e| panic!("stage {name:?}: {e}"));
        assert_eq!(
            index_blob(p, name),
            "1\n2\n3\n4\n5\n6\n7\nEIGHT\n9\n",
            "{name}"
        );
        let staged = repo
            .file_diff(&DiffSource::Staged, name, None, 1, Whitespace::Show)
            .expect("staged");
        repo.apply_hunk(HunkAction::Unstage, name, 0, staged.hunks[0].key(), 1)
            .unwrap_or_else(|e| panic!("unstage {name:?}: {e}"));
        assert_eq!(index_blob(p, name), "1\n2\n3\n4\n5\n6\n7\n8\n9\n", "{name}");
        let d = repo
            .file_diff(&DiffSource::Unstaged, name, None, 1, Whitespace::Show)
            .expect("diff");
        repo.apply_hunk(HunkAction::Discard, name, 0, d.hunks[0].key(), 1)
            .unwrap_or_else(|e| panic!("discard {name:?}: {e}"));
        assert_eq!(
            fs::read_to_string(p.join(name)).expect("read"),
            "1\n2\n3\n4\n5\n6\n7\nEIGHT\n9\n",
            "{name}"
        );
    }

    // A new (untracked) file with an unusual name: its single hunk stages as a new file.
    let name = "new caf\u{e9}.txt";
    fs::write(p.join(name), "x\n").expect("write");
    let d = repo
        .file_diff(&DiffSource::Unstaged, name, None, 3, Whitespace::Show)
        .expect("diff");
    repo.apply_hunk(HunkAction::Stage, name, 0, d.hunks[0].key(), 3)
        .expect("stage new file");
    assert_eq!(index_blob(p, name), "x\n");
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

    assert!(repo.merge("other", MergeMode::Default).is_err());
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
    assert_eq!(rev(&dest, "refs/canopy/requests/origin/pull/1/head"), first);
    let has = |name: &str| repo.git().find_reference(name).is_ok();
    assert!(!has("refs/remotes/origin/pr/1"), "no remote-tracking ref");
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

    // Local commits on top of the request head: checking it out again just switches to the
    // branch, like `git merge --ff-only` finding it already up to date.
    fs::write(dest.join("mine.txt"), "mine\n").expect("write");
    repo.stage_all().expect("stage");
    let mine = repo.commit("feat: mine", false, false).expect("commit");
    repo.checkout_branch("main").expect("switch to main");
    repo.checkout_request("origin", "refs/pull/1/head", "pr/1", |_| {})
        .expect("branch ahead of the request");
    assert_eq!(head(&repo), (Some("pr/1".into()), Some(mine.to_string())));

    // Local commits are never thrown away: once the request head is rewritten the two have
    // diverged, and nothing is switched or moved.
    fs::write(work.join("pr.txt"), "rewritten\n").expect("write");
    git(
        work,
        &["commit", "-q", "-a", "--amend", "-m", "feat: rewritten"],
    );
    git(
        work,
        &["push", "-q", "-f", &server_url, "contrib:refs/pull/1/head"],
    );
    repo.checkout_branch("main").expect("switch to main");
    let main = rev(&dest, "main");
    match repo.checkout_request("origin", "refs/pull/1/head", "pr/1", |_| {}) {
        Err(canopy_core::Error::Invalid(msg)) => assert!(msg.contains("diverged"), "{msg}"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(head(&repo), (Some("main".into()), Some(main)));
    assert_eq!(rev(&dest, "pr/1"), mine.to_string());
    assert!(repo.snapshot(10).expect("snapshot").status.is_clean());
    // The private ref still moved to the rewritten head (it is forced, like a fetch).
    assert_eq!(
        rev(&dest, "refs/canopy/requests/origin/pull/1/head"),
        rev(work, "HEAD")
    );
}

#[test]
fn checkout_request_leaves_a_real_remote_branch_of_the_same_name_alone() {
    // The server has a real branch `pr/1` and, separately, the request head refs/pull/1/head.
    let (work_dir, _work) = fixture();
    let work = work_dir.path();
    let server = tempfile::tempdir().expect("tempdir");
    git(server.path(), &["init", "-q", "--bare", "-b", "main"]);
    let server_url = server.path().to_string_lossy().into_owned();
    git(work, &["push", "-q", &server_url, "main"]);
    git(work, &["switch", "-q", "-c", "real"]);
    fs::write(work.join("real.txt"), "real\n").expect("write");
    git(work, &["add", "."]);
    git(work, &["commit", "-q", "-m", "feat: real branch"]);
    git(work, &["push", "-q", &server_url, "real:refs/heads/pr/1"]);
    let real = git(work, &["rev-parse", "HEAD"]).trim().to_owned();
    git(work, &["switch", "-q", "main"]);
    git(work, &["switch", "-q", "-c", "contrib"]);
    fs::write(work.join("pr.txt"), "request\n").expect("write");
    git(work, &["add", "."]);
    git(work, &["commit", "-q", "-m", "feat: request"]);
    git(
        work,
        &["push", "-q", &server_url, "contrib:refs/pull/1/head"],
    );
    let request = git(work, &["rev-parse", "HEAD"]).trim().to_owned();

    let clone_dir = tempfile::tempdir().expect("tempdir");
    let dest = clone_dir.path().join("clone");
    canopy_core::ops::clone(&server_url, &dest, |_| {}).expect("clone");
    configure(&dest);
    let repo = Repo::open(&dest).expect("open");
    let rev = |r: &str| git(&dest, &["rev-parse", r]).trim().to_owned();
    assert_eq!(rev("refs/remotes/origin/pr/1"), real);

    repo.checkout_request("origin", "refs/pull/1/head", "pr/1", |_| {})
        .expect("checkout request");
    assert_eq!(
        rev("refs/remotes/origin/pr/1"),
        real,
        "tracking ref untouched"
    );
    assert_eq!(rev("refs/canopy/requests/origin/pull/1/head"), request);
    assert_eq!(rev("HEAD"), request);
    let pr = repo
        .branches()
        .expect("branches")
        .into_iter()
        .find(|b| b.name == "pr/1")
        .expect("pr/1");
    assert_eq!(pr.upstream, None, "--no-track");

    // A prune keeps both the real branch's tracking ref and the private request ref.
    repo.fetch(Some("origin"), true, |_| {})
        .expect("fetch --prune");
    assert_eq!(rev("refs/remotes/origin/pr/1"), real);
    assert_eq!(rev("refs/canopy/requests/origin/pull/1/head"), request);

    // A local `pr/1` that is the real branch (tracking origin/pr/1) is unrelated to the request:
    // refused, and neither the branch nor HEAD moves.
    let (_d2, other) = {
        let d = tempfile::tempdir().expect("tempdir");
        let dest2 = d.path().join("clone");
        canopy_core::ops::clone(&server_url, &dest2, |_| {}).expect("clone");
        configure(&dest2);
        let r = Repo::open(&dest2).expect("open");
        (d, r)
    };
    let p2 = other.workdir().to_path_buf();
    git(&p2, &["branch", "--track", "pr/1", "origin/pr/1"]);
    match other.checkout_request("origin", "refs/pull/1/head", "pr/1", |_| {}) {
        Err(canopy_core::Error::Invalid(msg)) => assert!(msg.contains("pr/1"), "{msg}"),
        res => panic!("unexpected {res:?}"),
    }
    assert_eq!(git(&p2, &["rev-parse", "pr/1"]).trim(), real);
    assert_eq!(
        git(&p2, &["symbolic-ref", "--short", "HEAD"]).trim(),
        "main"
    );
    assert_eq!(
        git(&p2, &["rev-parse", "refs/remotes/origin/pr/1"]).trim(),
        real
    );
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

// ---------------------------------------------------------------------------------------------
// Conflict resolution and merge modes. Expected states were recorded with git 2.43 by running the
// same commands by hand (`git checkout --ours/--theirs`, `git add`, `git rm`, `git merge --squash`
// / `--ff-only` / `--no-ff`) in a scratch repository.

/// Index entries of `path` as `(mode, blob, stage)`, from `git ls-files -s`.
fn stages(dir: &Path, path: &str) -> Vec<(String, String, u8)> {
    git(dir, &["--literal-pathspecs", "ls-files", "-s", "--", path])
        .lines()
        .map(|l| {
            let (meta, _) = l.split_once('\t').expect("tab");
            let f: Vec<&str> = meta.split(' ').collect();
            (
                f[0].to_owned(),
                f[1].to_owned(),
                f[2].parse().expect("stage"),
            )
        })
        .collect()
}

fn rev(dir: &Path, rev: &str) -> String {
    git(dir, &["rev-parse", rev]).trim().to_owned()
}

fn parents(dir: &Path, rev: &str) -> Vec<String> {
    let out = git(dir, &["rev-list", "--parents", "-n", "1", rev]);
    out.split_whitespace().skip(1).map(str::to_owned).collect()
}

/// `main` and `other` both change lines 2 and 7 of `a.txt`.
fn both_modified(dir: &Path) {
    git(dir, &["switch", "-q", "-c", "other"]);
    fs::write(
        dir.join("a.txt"),
        "one\nTHEIRS 2\nthree\nfour\nfive\nsix\nTHEIRS 7\neight\nnine\nten\n",
    )
    .expect("write");
    git(dir, &["commit", "-q", "-am", "theirs"]);
    git(dir, &["switch", "-q", "main"]);
    fs::write(
        dir.join("a.txt"),
        "one\nOURS 2\nthree\nfour\nfive\nsix\nOURS 7\neight\nnine\nten\n",
    )
    .expect("write");
    git(dir, &["commit", "-q", "-am", "ours"]);
}

#[test]
fn diff3_conflict_resolved_in_the_editor_completes_the_merge() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["config", "merge.conflictStyle", "diff3"]);
    both_modified(p);
    assert!(repo.merge("other", MergeMode::Default).is_err());

    assert_eq!(
        repo.conflict_kind("a.txt").expect("kind"),
        Some(ConflictKind::BothModified)
    );
    let file = repo.conflict_file("a.txt").expect("conflict file");
    let regions: Vec<&ConflictRegion> = file.regions().collect();
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].ours, ["OURS 2"]);
    assert_eq!(regions[0].base.as_deref(), Some(&["two".to_owned()][..]));
    assert_eq!(regions[0].theirs, ["THEIRS 2"]);
    assert_eq!(regions[0].ours_label, "HEAD");
    assert_eq!(regions[0].theirs_label, "other");
    assert_eq!(regions[1].base.as_deref(), Some(&["seven".to_owned()][..]));

    // Refused while markers remain: nothing is written or staged.
    let before = fs::read_to_string(p.join("a.txt")).expect("read");
    let partial = file.render(&[Some(Choice::Theirs), None]);
    assert!(repo.write_resolution("a.txt", &partial, true).is_err());
    assert_eq!(fs::read_to_string(p.join("a.txt")).expect("read"), before);
    assert!(repo.mark_resolved("a.txt").is_err());
    assert_eq!(stages(p, "a.txt").len(), 3);

    // Saving a partial resolution keeps the conflict open.
    repo.write_resolution("a.txt", &partial, false)
        .expect("save partial");
    assert_eq!(fs::read_to_string(p.join("a.txt")).expect("read"), partial);
    assert_eq!(stages(p, "a.txt").len(), 3);

    let done = file.render(&[Some(Choice::Theirs), Some(Choice::Base)]);
    let expected = "one\nTHEIRS 2\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    assert_eq!(done, expected);
    repo.write_resolution("a.txt", &done, true)
        .expect("resolve");
    let st = stages(p, "a.txt");
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].2, 0);
    assert_eq!(repo.conflict_kind("a.txt").expect("kind"), None);

    repo.continue_or_abort(RepoState::Merge, false)
        .expect("merge --continue");
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.state, RepoState::Clean);
    assert!(snap.status.is_clean());
    assert_eq!(snap.commits[0].parents.len(), 2);
    assert_eq!(git(p, &["show", "HEAD:a.txt"]), expected);
}

#[test]
fn mark_resolved_stages_a_hand_edited_file() {
    let (dir, repo) = fixture();
    let p = dir.path();
    both_modified(p);
    assert!(repo.merge("other", MergeMode::Default).is_err());
    fs::write(p.join("a.txt"), "by hand\n").expect("write");
    repo.mark_resolved("a.txt").expect("mark resolved");
    let st = stages(p, "a.txt");
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].2, 0);
    assert_eq!(git(p, &["show", ":a.txt"]), "by hand\n");
}

#[test]
fn resolve_with_side_matches_git_checkout_ours_and_theirs() {
    let (dir, repo) = fixture();
    let p = dir.path();
    // A path with spaces and glob characters, next to a file the glob would match.
    let odd = "dir x/notes [v1].txt";
    fs::create_dir_all(p.join("dir x")).expect("mkdir");
    fs::write(p.join(odd), "base\n").expect("write");
    fs::write(p.join("dir x/notes v.txt"), "bystander\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "odd"]);
    both_modified(p);
    git(p, &["switch", "-q", "other"]);
    fs::write(p.join(odd), "theirs\n").expect("write");
    // Their version is also executable; taking their side must take the mode too.
    // (`commit -a` would re-stage the non-executable working copy, so stage, chmod, commit, then
    // check out to give the working copy the bit as well.)
    git(p, &["--literal-pathspecs", "add", "--", odd]);
    git(p, &["update-index", "--chmod=+x", "--", odd]);
    git(p, &["commit", "-q", "-m", "theirs odd"]);
    git(p, &["--literal-pathspecs", "checkout", "--", odd]);
    let tree = git(p, &["--literal-pathspecs", "ls-tree", "HEAD", "--", odd]);
    assert!(tree.starts_with("100755"), "{tree}");
    git(p, &["switch", "-q", "main"]);
    fs::write(p.join(odd), "ours\n").expect("write");
    git(p, &["commit", "-q", "-am", "ours odd"]);
    assert!(repo.merge("other", MergeMode::Default).is_err());
    fs::write(p.join("dir x/notes v.txt"), "edited\n").expect("write");

    for (path, side, stage) in [("a.txt", Side::Ours, 2), (odd, Side::Theirs, 3)] {
        let (mode, blob, _) = stages(p, path)
            .into_iter()
            .find(|s| s.2 == stage)
            .expect("stage present");
        let content = git(p, &["cat-file", "-p", &blob]);
        repo.resolve_with_side(path, side).expect("resolve");
        assert_eq!(stages(p, path), [(mode, blob, 0)], "{path}");
        assert_eq!(
            fs::read_to_string(p.join(path)).expect("read"),
            content,
            "{path}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(p.join(odd))
            .expect("meta")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "executable bit from their side");
    }
    // The bystander matching the glob was neither checked out nor staged.
    assert_eq!(
        fs::read_to_string(p.join("dir x/notes v.txt")).expect("read"),
        "edited\n"
    );
    assert_eq!(
        git(
            p,
            &["diff", "--cached", "--name-only", "--", "dir x/notes v.txt"]
        ),
        ""
    );
    assert!(
        repo.snapshot(10)
            .expect("snapshot")
            .status
            .conflicted
            .is_empty()
    );
    assert!(repo.resolve_with_side("a.txt", Side::Ours).is_err());
    assert_eq!(repo.conflict_kind("a.txt").expect("kind"), None);
}

#[test]
fn modify_delete_conflicts_resolve_by_keeping_or_removing() {
    let (dir, repo) = fixture();
    let p = dir.path();
    for f in ["d1.txt", "d2.txt", "e1.txt", "e2.txt"] {
        fs::write(p.join(f), format!("{f}\n")).expect("write");
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "files"]);
    git(p, &["switch", "-q", "-c", "other"]);
    git(p, &["rm", "-q", "d1.txt", "d2.txt"]);
    fs::write(p.join("e1.txt"), "theirs e1\n").expect("write");
    fs::write(p.join("e2.txt"), "theirs e2\n").expect("write");
    git(p, &["commit", "-q", "-am", "theirs"]);
    git(p, &["switch", "-q", "main"]);
    git(p, &["rm", "-q", "e1.txt", "e2.txt"]);
    fs::write(p.join("d1.txt"), "ours d1\n").expect("write");
    fs::write(p.join("d2.txt"), "ours d2\n").expect("write");
    git(p, &["commit", "-q", "-am", "ours"]);
    assert!(repo.merge("other", MergeMode::Default).is_err());

    for (path, kind) in [
        ("d1.txt", ConflictKind::DeletedByThem),
        ("d2.txt", ConflictKind::DeletedByThem),
        ("e1.txt", ConflictKind::DeletedByUs),
        ("e2.txt", ConflictKind::DeletedByUs),
    ] {
        assert_eq!(
            repo.conflict_kind(path).expect("kind"),
            Some(kind),
            "{path}"
        );
    }

    // Taking the deleting side removes the file; taking the other side keeps its version.
    repo.resolve_with_side("d1.txt", Side::Theirs)
        .expect("d1 theirs");
    repo.resolve_with_side("d2.txt", Side::Ours)
        .expect("d2 ours");
    repo.resolve_with_side("e1.txt", Side::Ours)
        .expect("e1 ours");
    repo.resolve_with_side("e2.txt", Side::Theirs)
        .expect("e2 theirs");

    assert!(stages(p, "d1.txt").is_empty() && !p.join("d1.txt").exists());
    assert!(stages(p, "e1.txt").is_empty() && !p.join("e1.txt").exists());
    assert_eq!(
        fs::read_to_string(p.join("d2.txt")).expect("read"),
        "ours d2\n"
    );
    assert_eq!(
        fs::read_to_string(p.join("e2.txt")).expect("read"),
        "theirs e2\n"
    );
    assert_eq!(stages(p, "d2.txt")[0].2, 0);
    assert_eq!(stages(p, "e2.txt")[0].2, 0);

    repo.continue_or_abort(RepoState::Merge, false)
        .expect("merge --continue");
    let files = git(p, &["ls-tree", "--name-only", "HEAD"]);
    let files: Vec<&str> = files.lines().collect();
    assert_eq!(files, ["a.txt", "d2.txt", "e2.txt"]);
}

#[test]
fn conflict_file_refuses_binary_content() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "other"]);
    fs::write(p.join("a.txt"), b"theirs\0binary\n").expect("write");
    git(p, &["commit", "-q", "-am", "theirs"]);
    git(p, &["switch", "-q", "main"]);
    fs::write(p.join("a.txt"), b"ours\0binary\n").expect("write");
    git(p, &["commit", "-q", "-am", "ours"]);
    assert!(repo.merge("other", MergeMode::Default).is_err());
    assert_eq!(
        repo.conflict_kind("a.txt").expect("kind"),
        Some(ConflictKind::BothModified)
    );
    assert!(matches!(
        repo.conflict_file("a.txt"),
        Err(canopy_core::Error::Invalid(_))
    ));
    repo.resolve_with_side("a.txt", Side::Theirs)
        .expect("theirs");
    assert_eq!(
        fs::read(p.join("a.txt")).expect("read"),
        b"theirs\0binary\n"
    );
}

#[cfg(unix)]
#[test]
fn write_resolution_keeps_the_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, repo) = fixture();
    let p = dir.path();
    let file = p.join("a.txt");
    let mut perms = fs::metadata(&file).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&file, perms).expect("chmod");
    repo.write_resolution("a.txt", "new\n", false)
        .expect("write");
    assert_eq!(fs::read_to_string(&file).expect("read"), "new\n");
    let mode = fs::metadata(&file).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    let leftovers = fs::read_dir(p)
        .expect("ls")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0);
}

/// Adds `feature`, one commit ahead of `main`; `diverge` also adds a commit on `main`. Returns
/// `feature`'s commit id.
fn feature_branch(dir: &Path, diverge: bool) -> String {
    git(dir, &["switch", "-q", "-c", "feature"]);
    fs::write(dir.join("f.txt"), "feature\n").expect("write");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "feat: f"]);
    git(dir, &["switch", "-q", "main"]);
    if diverge {
        fs::write(dir.join("m.txt"), "main\n").expect("write");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "feat: m"]);
    }
    rev(dir, "feature")
}

#[test]
fn merge_mode_default_fast_forwards_or_merges() {
    let (dir, repo) = fixture();
    let p = dir.path();
    // `merge.ff=false` would make a plain `git merge` create a commit; the mode wins.
    git(p, &["config", "merge.ff", "false"]);
    let feature = feature_branch(p, false);
    repo.merge("feature", MergeMode::Default).expect("merge");
    assert_eq!(rev(p, "HEAD"), feature);

    let (dir, repo) = fixture();
    let p = dir.path();
    let feature = feature_branch(p, true);
    let main = rev(p, "HEAD");
    repo.merge("feature", MergeMode::Default).expect("merge");
    assert_eq!(parents(p, "HEAD"), [main, feature]);
}

#[test]
fn merge_mode_no_ff_creates_a_merge_commit_even_when_ff_is_possible() {
    let (dir, repo) = fixture();
    let p = dir.path();
    let main = rev(p, "HEAD");
    let feature = feature_branch(p, false);
    repo.merge("feature", MergeMode::NoFastForward)
        .expect("merge");
    assert_eq!(parents(p, "HEAD"), [main, feature]);
    assert!(repo.snapshot(10).expect("snapshot").status.is_clean());
}

#[test]
fn merge_mode_ff_only_fails_on_diverged_history_without_changes() {
    let (dir, repo) = fixture();
    let p = dir.path();
    feature_branch(p, true);
    let main = rev(p, "HEAD");
    let err = repo
        .merge("feature", MergeMode::FastForwardOnly)
        .expect_err("diverged");
    assert!(matches!(err, canopy_core::Error::Cli { .. }), "{err:?}");
    assert_eq!(rev(p, "HEAD"), main);
    let snap = repo.snapshot(10).expect("snapshot");
    assert_eq!(snap.state, RepoState::Clean);
    assert!(snap.status.is_clean());

    let (dir, repo) = fixture();
    let p = dir.path();
    let feature = feature_branch(p, false);
    repo.merge("feature", MergeMode::FastForwardOnly)
        .expect("ff");
    assert_eq!(rev(p, "HEAD"), feature);
}

#[test]
fn merge_mode_squash_stages_changes_without_committing_or_merge_head() {
    for diverge in [false, true] {
        let (dir, repo) = fixture();
        let p = dir.path();
        feature_branch(p, diverge);
        let main = rev(p, "HEAD");
        repo.merge("feature", MergeMode::Squash).expect("squash");
        assert_eq!(rev(p, "HEAD"), main, "diverge={diverge}");
        let git_dir = repo.git().path().to_path_buf();
        assert!(!git_dir.join("MERGE_HEAD").exists(), "diverge={diverge}");
        assert!(git_dir.join("SQUASH_MSG").exists(), "diverge={diverge}");
        let snap = repo.snapshot(10).expect("snapshot");
        assert_eq!(snap.state, RepoState::Clean);
        assert_eq!(snap.status.staged.len(), 1);
        assert_eq!(snap.status.staged[0].path, "f.txt");
        assert_eq!(snap.status.staged[0].kind, ChangeKind::Added);

        repo.commit("feat: squashed f", false, false)
            .expect("commit");
        assert_eq!(parents(p, "HEAD"), [main], "diverge={diverge}");
    }
}
