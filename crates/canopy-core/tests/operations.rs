//! End-to-end tests of `canopy-core` against throwaway repositories built with the `git` CLI.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::identity::{self, Scope, Source};
use canopy_core::ops::{HunkAction, MergeMode, ResetMode};
use canopy_core::{
    ChangeKind, Choice, ConflictFile, ConflictKind, ConflictRegion, DiffSource, LineKind,
    MissingObjects, PullStrategy, RefKind, Repo, RepoState, Side, Whitespace,
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
    // Git for Windows sets core.autocrlf=true system-wide, which would rewrite
    // the fixtures' LF files as CRLF whenever git writes them.
    git(p, &["config", "core.autocrlf", "false"]);
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
    let staged = repo.snapshot(10).expect("snapshot").status.staged;
    assert!(staged.is_empty(), "{staged:?}");

    let untracked = repo.snapshot(10).expect("snapshot").status.unstaged;
    repo.discard(&untracked).expect("discard");
    assert!(repo.snapshot(10).expect("snapshot").status.is_clean());
    assert!(!p.join("new.txt").exists());

    fs::write(p.join("a.txt"), "again\n").expect("write");
    repo.stage_all().expect("stage all");
    let oid = repo
        .commit("fix: change a\n\nBody.", false, false)
        .expect("commit");
    let details = repo
        .commit_details(oid, MissingObjects::Fail)
        .expect("details");
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
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            1,
            Whitespace::Show,
            MissingObjects::Fail,
        )
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
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            1,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("staged");
    assert_eq!(staged.hunks.len(), 1);
    assert!(staged.hunks[0].lines.iter().any(|l| l.text == "TEN"));
    let unstaged = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            1,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("unstaged");
    assert_eq!(unstaged.hunks.len(), 1);
    assert!(unstaged.hunks[0].lines.iter().any(|l| l.text == "ONE"));

    repo.apply_hunk(HunkAction::Unstage, "a.txt", 0, staged.hunks[0].key(), 1)
        .expect("unstage hunk");
    let hunks = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            1,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("staged")
        .hunks;
    assert!(hunks.is_empty(), "{hunks:?}");

    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            1,
            Whitespace::Show,
            MissingObjects::Fail,
        )
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
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            0,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    assert_eq!(d.hunks.len(), 3, "{d:#?}");

    repo.apply_hunk(HunkAction::Stage, "a.txt", 1, d.hunks[1].key(), 0)
        .expect("stage the middle hunk");
    assert_eq!(
        index_blob(p, "a.txt"),
        "one\ntwo\nthree\nfour\nFIVE\nsix\nseven\neight\nnine\nten\n"
    );

    let staged = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            0,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("staged");
    assert_eq!(staged.hunks.len(), 1);
    repo.apply_hunk(HunkAction::Unstage, "a.txt", 0, staged.hunks[0].key(), 0)
        .expect("unstage the middle hunk");
    assert_eq!(index_blob(p, "a.txt"), original);

    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            0,
            Whitespace::Show,
            MissingObjects::Fail,
        )
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
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            0,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    assert_eq!(d.hunks.len(), 2, "{d:#?}");
    repo.apply_hunk(HunkAction::Stage, "a.txt", 0, d.hunks[0].key(), 0)
        .expect("stage the insertion");
    assert_eq!(
        index_blob(p, "a.txt"),
        "one\ntwo\nthree\nfour\nfive\nNEW\nsix\nseven\neight\nnine\nten\n"
    );
    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            0,
            Whitespace::Show,
            MissingObjects::Fail,
        )
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
            .file_diff(
                &DiffSource::Unstaged,
                name,
                None,
                1,
                Whitespace::Show,
                MissingObjects::Fail,
            )
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
            .file_diff(
                &DiffSource::Staged,
                name,
                None,
                1,
                Whitespace::Show,
                MissingObjects::Fail,
            )
            .expect("staged");
        repo.apply_hunk(HunkAction::Unstage, name, 0, staged.hunks[0].key(), 1)
            .unwrap_or_else(|e| panic!("unstage {name:?}: {e}"));
        assert_eq!(index_blob(p, name), "1\n2\n3\n4\n5\n6\n7\n8\n9\n", "{name}");
        let d = repo
            .file_diff(
                &DiffSource::Unstaged,
                name,
                None,
                1,
                Whitespace::Show,
                MissingObjects::Fail,
            )
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
        .file_diff(
            &DiffSource::Unstaged,
            name,
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
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
    assert!(snap.stashes.is_empty(), "{:?}", snap.stashes);
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

    repo.merge("other", MergeMode::Default).unwrap_err();
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

/// Runs the calling test again in a child process whose git sees no global or system
/// configuration and whose libgit2 sees no global one, so a developer's own settings
/// (`pull.rebase`, for one) cannot decide the result.
/// Returns true in the child, where the test body runs, and false in the parent once the child
/// has passed.
fn in_isolated_git_env(test: &str) -> bool {
    in_isolated_git_env_with_home_config(test, "")
}

/// [`in_isolated_git_env`], with `home_config` written to `$HOME/.gitconfig`: libgit2 reads that
/// file as global configuration while git, told to use an empty `GIT_CONFIG_GLOBAL`, does not.
fn in_isolated_git_env_with_home_config(test: &str, home_config: &str) -> bool {
    if std::env::var_os("CANOPY_TEST_ISOLATED_GIT").is_some_and(|v| v == "1") {
        return true;
    }
    let home = tempfile::tempdir().expect("tempdir");
    fs::write(home.path().join(".gitconfig"), home_config).expect("write");
    let out = Command::new(std::env::current_exe().expect("test binary"))
        .args([test, "--exact", "--test-threads=1"])
        .env("CANOPY_TEST_ISOLATED_GIT", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.path().join("gitconfig"))
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .env_remove("EMAIL")
        .output()
        .expect("test binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test} in an isolated git environment:\n{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

#[test]
fn identity_uses_git_scope_and_keeps_stored_history_authors() {
    if !in_isolated_git_env("identity_uses_git_scope_and_keeps_stored_history_authors") {
        return;
    }
    let global = std::env::var("GIT_CONFIG_GLOBAL").expect("global path");
    fs::write(
        &global,
        "[user]\nname = Global Person\nemail = global@example.com\n",
    )
    .expect("global config");
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);

    let before = identity::Identity::read(p).expect("identity");
    assert_eq!(before.author.name.value, "Global Person");
    assert_eq!(before.author.email.value, "global@example.com");
    assert!(
        matches!(before.author.name.source, Some(Source::Config { ref scope, .. }) if scope == "global")
    );
    assert_eq!(before.warning(), None);

    identity::set(Some(p), Scope::Local, "t", "t@t").expect("set local");
    let placeholder = identity::Identity::read(p).expect("local identity");
    assert_eq!(placeholder.author.name.value, "t");
    assert!(
        matches!(placeholder.author.name.source, Some(Source::Config { ref scope, .. }) if scope == "local")
    );
    assert!(placeholder.warning().is_some());

    fs::write(p.join("one.txt"), "one\n").expect("write");
    git(p, &["add", "--", "one.txt"]);
    git(p, &["commit", "-q", "-m", "first"]);
    let repo = Repo::open(p).expect("repo");
    let (commits, _) = repo.commits(10).expect("commits");
    assert_eq!(commits[0].author.name, "Ada Lovelace");

    identity::set(
        Some(p),
        Scope::Local,
        "Current Person",
        "current@example.com",
    )
    .expect("replace local");
    let (commits, _) = repo.commits(10).expect("history after config edit");
    assert_eq!(
        commits[0].author.name, "Ada Lovelace",
        "stored history must not change"
    );
    assert_eq!(
        identity::Identity::read(p)
            .expect("current")
            .author
            .name
            .value,
        "Current Person"
    );

    identity::clear(Some(p), Scope::Local).expect("clear local");
    assert_eq!(
        identity::Identity::read(p)
            .expect("fallback")
            .author
            .name
            .value,
        "Global Person"
    );
}

/// Runs the calling test again in a child process whose git sees a throw-away global
/// configuration and none of the identity variables except those in `env`. Returns true in the
/// child, where the test body runs, and false in the parent once the child has passed.
fn in_identity_child(test: &str, env: &[(&str, &str)]) -> bool {
    if std::env::var_os("CANOPY_IDENTITY_CHILD").is_some() {
        return true;
    }
    let home = tempfile::tempdir().expect("home");
    let mut child = Command::new(std::env::current_exe().expect("test binary"));
    child
        .args([test, "--exact", "--test-threads=1"])
        .env("CANOPY_IDENTITY_CHILD", "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.path().join("global"));
    for key in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "EMAIL",
    ] {
        child.env_remove(key);
    }
    for (key, value) in env {
        child.env(key, value);
    }
    let out = child.output().expect("test binary runs");
    assert!(
        out.status.success(),
        "{test}:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

#[test]
fn identity_follows_author_overrides_and_git_environment() {
    if !in_identity_child(
        "identity_follows_author_overrides_and_git_environment",
        &[("GIT_AUTHOR_NAME", "Environment Author")],
    ) {
        return;
    }
    let dir = tempfile::tempdir().expect("repo");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "User Name"]);
    git(p, &["config", "user.email", "user@example.com"]);
    git(p, &["config", "author.name", "Configured Author"]);
    git(p, &["config", "author.email", "author@example.com"]);
    let identity = identity::Identity::read(p).expect("identity");
    assert_eq!(identity.author.name.value, "Environment Author");
    assert_eq!(identity.author.email.value, "author@example.com");
    assert!(matches!(
        identity.author.name.source,
        Some(Source::Environment("GIT_AUTHOR_NAME"))
    ));
    assert!(
        matches!(identity.author.email.source, Some(Source::Config { ref scope, .. }) if scope == "local")
    );
    assert_eq!(identity.committer.name.value, "User Name");
    assert!(
        matches!(identity.committer.name.source, Some(Source::Config { ref scope, .. }) if scope == "local")
    );
}

#[test]
fn identity_flags_missing_and_unusable_identities() {
    if !in_isolated_git_env("identity_flags_missing_and_unusable_identities") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);

    // Nothing configured: git either refuses or guesses from the computer, depending on the host.
    let none = identity::Identity::read(p).expect("identity");
    assert!(!none.author.explicit());
    assert!(none.warning().is_some());
    if none.author.usable {
        assert_eq!(none.author.name.source, Some(Source::Guessed));
        assert_eq!(
            none.warning(),
            Some("Git guessed part of this identity from the computer.")
        );
    } else {
        assert_eq!(
            none.warning(),
            Some("Git cannot use this identity for a commit.")
        );
    }

    // With guessing switched off, a name alone is not an identity.
    git(p, &["config", "user.useConfigOnly", "true"]);
    git(p, &["config", "user.name", "Only Name"]);
    let half = identity::Identity::read(p).expect("name only");
    assert!(!half.author.usable);
    assert_eq!(half.author.name.value, "Only Name");
    assert!(
        half.author.email.value.is_empty(),
        "{:?}",
        half.author.email
    );
    assert_eq!(
        half.warning(),
        Some("Git cannot use this identity for a commit.")
    );

    // The first-launch view sees global configuration only: the repository's name does not count.
    let global = identity::Identity::read_global().expect("global");
    assert!(!global.author.usable);
    assert!(
        global.author.name.value.is_empty(),
        "{:?}",
        global.author.name
    );
    assert!(global.warning().is_some());
}

#[test]
fn identity_writes_awkward_values_and_reads_every_scope() {
    if !in_isolated_git_env("identity_writes_awkward_values_and_reads_every_scope") {
        return;
    }
    let global_file = std::path::PathBuf::from(std::env::var("GIT_CONFIG_GLOBAL").expect("global"));
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);

    // A value that starts with a dash is a value, and a second write replaces duplicates.
    git(p, &["config", "--add", "user.name", "Old One"]);
    git(p, &["config", "--add", "user.name", "Old Two"]);
    identity::set(Some(p), Scope::Local, "-dash", "dash@example.com").expect("dash");
    assert_eq!(
        git(p, &["config", "--local", "--get-all", "user.name"]),
        "-dash\n"
    );
    let read = identity::Identity::read(p).expect("identity");
    assert_eq!(read.author.name.value, "-dash");
    assert_eq!(read.author.email.value, "dash@example.com");
    assert_eq!(read.warning(), None);

    // Values git cannot store, or that make no sense as an address, are refused before git runs.
    for (name, email) in [
        ("", "a@example.com"),
        ("A<b>", "a@example.com"),
        ("Ada\nLovelace", "a@example.com"),
        ("Ada", "no-at-sign"),
        ("Ada", "ada@"),
        ("Ada", "a<b>@example.com"),
    ] {
        assert!(
            identity::set(Some(p), Scope::Local, name, email).is_err(),
            "{name:?} {email:?} must be refused"
        );
    }
    assert!(identity::set(None, Scope::Local, "Ada", "ada@example.com").is_err());
    assert!(identity::clear(None, Scope::Local).is_err());
    assert_eq!(
        git(p, &["config", "--local", "--get", "user.name"]),
        "-dash\n"
    );

    // A file included from the global configuration reports the global scope and its own path.
    let included = p.join("included.gitconfig");
    fs::write(
        &included,
        "[user]\n\tname = Included\n\temail = included@example.com\n",
    )
    .expect("included");
    // Forward slashes: git reads a backslash in a value as an escape.
    let included_path = included.to_string_lossy().replace('\\', "/");
    fs::write(
        &global_file,
        format!("[include]\n\tpath = {included_path}\n"),
    )
    .expect("global");
    identity::clear(Some(p), Scope::Local).expect("clear local");
    let included_read = identity::Identity::read(p).expect("included identity");
    assert_eq!(included_read.author.name.value, "Included");
    match included_read.author.name.source {
        Some(Source::Config { scope, origin }) => {
            assert_eq!(scope, "global");
            assert!(origin.ends_with("included.gitconfig"), "{origin}");
        }
        other => panic!("unexpected source {other:?}"),
    }

    // Writing and clearing the global scope without a repository.
    fs::write(&global_file, "").expect("empty global");
    assert!(
        !identity::Identity::read_global()
            .expect("empty")
            .author
            .usable
    );
    identity::set(None, Scope::Global, "Glo Bal", "glo@example.com").expect("set global");
    let global = identity::Identity::read_global().expect("global");
    assert!(global.author.usable);
    assert_eq!(global.author.name.value, "Glo Bal");
    assert_eq!(global.warning(), None);
    assert!(
        fs::read_to_string(&global_file)
            .expect("file")
            .contains("glo@example.com")
    );
    identity::clear(None, Scope::Global).expect("clear global");
    identity::clear(None, Scope::Global).expect("clearing nothing is fine");
    assert!(
        !identity::Identity::read_global()
            .expect("cleared")
            .author
            .usable
    );
}

#[test]
fn identity_takes_the_email_variable_when_nothing_else_names_an_address() {
    if !in_identity_child(
        "identity_takes_the_email_variable_when_nothing_else_names_an_address",
        &[("EMAIL", "from-env@example.com")],
    ) {
        return;
    }
    let dir = tempfile::tempdir().expect("repo");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Only Name"]);
    let read = identity::Identity::read(p).expect("identity");
    assert!(read.author.usable);
    assert_eq!(read.author.email.value, "from-env@example.com");
    assert_eq!(read.author.email.source, Some(Source::Environment("EMAIL")));
    assert_eq!(read.warning(), None);
}

#[test]
fn identity_ignores_an_empty_email_variable() {
    if !in_identity_child("identity_ignores_an_empty_email_variable", &[("EMAIL", "")]) {
        return;
    }
    let dir = tempfile::tempdir().expect("repo");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Only Name"]);
    // Git ignores an empty EMAIL: the address is then missing, or guessed on a host with a domain.
    let read = identity::Identity::read(p).expect("identity");
    assert_ne!(read.author.email.source, Some(Source::Environment("EMAIL")));
    assert!(read.warning().is_some(), "{read:?}");
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
        repo.file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            1,
            ws,
            MissingObjects::Fail,
        )
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
            MissingObjects::Fail,
        )
        .expect("diff");
    assert!(eol.hunks[0].lines.iter().any(|l| l.text == "    nine"));
    assert_eq!(hunks(Whitespace::IgnoreAll), 0);
    assert_eq!(hunks(Whitespace::IgnoreAll), cli_hunks("-w"));

    // Staged and commit diffs honour the mode too.
    repo.stage_all().expect("stage");
    let staged = |ws| {
        repo.file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            1,
            ws,
            MissingObjects::Fail,
        )
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
        repo.file_diff(
            &DiffSource::Commit(oid),
            "a.txt",
            None,
            1,
            ws,
            MissingObjects::Fail,
        )
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
    if !in_isolated_git_env("pull_fast_forward_only_refuses_diverged_branches") {
        return;
    }
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    diverge(up_dir.path(), &copy);
    let before = copy.head().expect("head").oid;
    let err = copy
        .pull(PullStrategy::FastForwardOnly, |_| {})
        .expect_err("diverged");
    match err {
        canopy_core::Error::Cli { command, stderr } => {
            assert_eq!(command, "pull");
            assert!(!stderr.is_empty(), "git explained nothing");
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
    if !in_isolated_git_env("pull_fast_forward_only_fast_forwards") {
        return;
    }
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
    if !in_isolated_git_env("pull_rebase_replays_local_commits") {
        return;
    }
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
    if !in_isolated_git_env("pull_merge_creates_a_merge_commit") {
        return;
    }
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    diverge(up_dir.path(), &copy);
    copy.pull(PullStrategy::Merge, |_| {}).expect("pull");
    assert_eq!(head_parents(&copy), 2);
    assert!(copy.workdir().join("up.txt").exists());
}

#[test]
fn pull_reads_pull_rebase_as_git_does() {
    // libgit2 sees pull.rebase=true in $HOME, git sees no setting: the strategy must apply.
    if !in_isolated_git_env_with_home_config(
        "pull_reads_pull_rebase_as_git_does",
        "[pull]\n\trebase = true\n",
    ) {
        return;
    }
    let (up_dir, _up, _c, copy) = upstream_and_clone();
    diverge(up_dir.path(), &copy);
    copy.pull(PullStrategy::Merge, |_| {}).expect("pull");
    assert_eq!(head_parents(&copy), 2);
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
    assert_eq!(head(&repo), (Some("pr/1".into()), Some(second)));
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
    repo.merge("other", MergeMode::Default).unwrap_err();

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
    assert!(
        repo.write_resolution("a.txt", &partial, true, file.fingerprint())
            .is_err()
    );
    assert_eq!(fs::read_to_string(p.join("a.txt")).expect("read"), before);
    assert!(repo.mark_resolved("a.txt").is_err());
    assert_eq!(stages(p, "a.txt").len(), 3);

    // Saving a partial resolution keeps the conflict open.
    repo.write_resolution("a.txt", &partial, false, file.fingerprint())
        .expect("save partial");
    assert_eq!(fs::read_to_string(p.join("a.txt")).expect("read"), partial);
    assert_eq!(stages(p, "a.txt").len(), 3);

    // The save changed the file, so the rest is resolved from a fresh parse, as the app does.
    let file = repo.conflict_file("a.txt").expect("reparse");
    assert_eq!(file.conflicts(), 1);
    let done = file.render(&[Some(Choice::Base)]);
    let expected = "one\nTHEIRS 2\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    assert_eq!(done, expected);
    repo.write_resolution("a.txt", &done, true, file.fingerprint())
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
    repo.merge("other", MergeMode::Default).unwrap_err();
    fs::write(p.join("a.txt"), "by hand\n").expect("write");
    repo.mark_resolved("a.txt").expect("mark resolved");
    let st = stages(p, "a.txt");
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].2, 0);
    assert_eq!(git(p, &["show", ":a.txt"]), "by hand\n");
}

#[test]
fn write_resolution_with_the_parsed_fingerprint_resolves() {
    let (dir, repo) = fixture();
    let p = dir.path();
    both_modified(p);
    repo.merge("other", MergeMode::Default).unwrap_err();
    let file = repo.conflict_file("a.txt").expect("parse");
    let done = file.render(&[Some(Choice::Ours), Some(Choice::Theirs)]);
    repo.write_resolution("a.txt", &done, true, file.fingerprint())
        .expect("resolve");
    assert_eq!(fs::read_to_string(p.join("a.txt")).expect("read"), done);
    let st = stages(p, "a.txt");
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].2, 0);
    assert_eq!(git(p, &["show", ":a.txt"]), done);
}

/// Regression: the user hand-fixes the file in an editor and saves, then clicks "Save and mark
/// resolved" before Canopy re-reads it. The stale rendering must not replace the hand edit.
#[test]
fn write_resolution_refuses_a_file_changed_since_it_was_parsed() {
    let (dir, repo) = fixture();
    let p = dir.path();
    both_modified(p);
    repo.merge("other", MergeMode::Default).unwrap_err();
    let original = fs::read_to_string(p.join("a.txt")).expect("read");
    let file = repo.conflict_file("a.txt").expect("parse");
    let stale = file.render(&[Some(Choice::Ours), Some(Choice::Ours)]);
    let conflicted = stages(p, "a.txt");
    assert_eq!(conflicted.len(), 3);

    let edits = [
        ("hand fix", "one\nby hand\nten\n".to_owned()),
        ("same length", original.replacen("OURS 2", "MINE 2", 1)),
        ("line endings", original.replace('\n', "\r\n")),
    ];
    for (name, edit) in &edits {
        fs::write(p.join("a.txt"), edit).expect("hand edit");
        for mark in [true, false] {
            let err = repo
                .write_resolution("a.txt", &stale, mark, file.fingerprint())
                .expect_err(name);
            assert!(
                matches!(&err, canopy_core::Error::ChangedOnDisk(path) if path == "a.txt"),
                "{name} mark={mark}: {err:?}"
            );
            assert_eq!(
                fs::read_to_string(p.join("a.txt")).expect("read"),
                *edit,
                "{name} mark={mark}"
            );
            assert_eq!(stages(p, "a.txt"), conflicted, "{name} mark={mark}");
        }
    }

    // A file removed since it was parsed is not brought back.
    fs::remove_file(p.join("a.txt")).expect("rm");
    let err = repo
        .write_resolution("a.txt", &stale, true, file.fingerprint())
        .expect_err("removed");
    assert!(
        matches!(&err, canopy_core::Error::ChangedOnDisk(_)),
        "{err:?}"
    );
    assert!(!p.join("a.txt").exists());
    assert_eq!(stages(p, "a.txt"), conflicted);

    // Reloading gives the fingerprint of what is on disk now, and saving works again.
    fs::write(p.join("a.txt"), &original).expect("restore");
    let reloaded = repo.conflict_file("a.txt").expect("reparse");
    repo.write_resolution("a.txt", &stale, true, reloaded.fingerprint())
        .expect("save after reload");
    assert_eq!(git(p, &["show", ":a.txt"]), stale);
}

#[test]
fn conflict_fingerprint_is_stable_across_parses() {
    let (dir, repo) = fixture();
    let p = dir.path();
    both_modified(p);
    repo.merge("other", MergeMode::Default).unwrap_err();
    let first = repo.conflict_file("a.txt").expect("parse");
    let second = repo.conflict_file("a.txt").expect("parse again");
    assert_eq!(first.fingerprint(), second.fingerprint());
    let text = fs::read_to_string(p.join("a.txt")).expect("read");
    assert_eq!(
        ConflictFile::parse(&text).fingerprint(),
        first.fingerprint()
    );
    // An editor saving identical bytes (a new file and mtime) is not a change.
    fs::remove_file(p.join("a.txt")).expect("rm");
    fs::write(p.join("a.txt"), &text).expect("rewrite");
    let resaved = repo.conflict_file("a.txt").expect("parse resaved");
    assert_eq!(resaved.fingerprint(), first.fingerprint());
    let done = first.render(&[Some(Choice::Theirs), Some(Choice::Theirs)]);
    repo.write_resolution("a.txt", &done, false, first.fingerprint())
        .expect("same bytes still match");
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
    repo.merge("other", MergeMode::Default).unwrap_err();
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
    let conflicted = repo.snapshot(10).expect("snapshot").status.conflicted;
    assert!(conflicted.is_empty(), "{conflicted:?}");
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
    repo.merge("other", MergeMode::Default).unwrap_err();

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
    repo.merge("other", MergeMode::Default).unwrap_err();
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
    let parsed = repo.conflict_file("a.txt").expect("parse");
    repo.write_resolution("a.txt", "new\n", false, parsed.fingerprint())
        .expect("write");
    assert_eq!(fs::read_to_string(&file).expect("read"), "new\n");
    let mode = fs::metadata(&file).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    let leftovers = fs::read_dir(p)
        .expect("ls")
        .filter_map(Result::ok)
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

#[test]
fn stage_unstage_and_discard_treat_paths_literally() {
    let (dir, repo) = fixture();
    let p = dir.path();
    // `[ab].txt` as a glob also matches b.txt.
    fs::write(p.join("[ab].txt"), "glob\n").unwrap();
    fs::write(p.join("b.txt"), "b\n").unwrap();

    repo.stage(&["[ab].txt".to_owned()]).unwrap();
    assert_eq!(git(p, &["diff", "--cached", "--name-only"]), "[ab].txt\n");
    git(p, &["add", "--", "b.txt"]);
    repo.unstage(&["[ab].txt".to_owned()]).unwrap();
    assert_eq!(git(p, &["diff", "--cached", "--name-only"]), "b.txt\n");
    git(p, &["reset", "-q"]);

    let st = canopy_core::status::collect(repo.git()).unwrap();
    let glob: Vec<_> = st
        .unstaged
        .iter()
        .filter(|e| e.path == "[ab].txt")
        .cloned()
        .collect();
    assert_eq!(glob.len(), 1);
    repo.discard(&glob).unwrap();
    assert!(!p.join("[ab].txt").exists());
    assert!(p.join("b.txt").exists(), "discard must not delete b.txt");
}
