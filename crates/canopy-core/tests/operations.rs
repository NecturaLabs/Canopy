//! End-to-end tests of `canopy-core` against throwaway repositories built with the `git` CLI.

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::ops::{HunkAction, MergeMode, ResetMode};
use canopy_core::{
    ChangeKind, Choice, ConflictKind, ConflictRegion, DiffSource, LineKind, RefKind, Repo,
    RepoState, Side,
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

        repo.commit("feat: squashed f", false).expect("commit");
        assert_eq!(parents(p, "HEAD"), [main], "diverge={diverge}");
    }
}
