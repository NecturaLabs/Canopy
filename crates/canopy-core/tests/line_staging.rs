//! Line-level staging (`Repo::apply_lines`) against throwaway repositories.
//!
//! Expected states come from running the same selection through git's own edit mode
//! (`git add -p`, `git reset -p`, `git checkout -p` with `e`, turning unselected lines into
//! context or deleting them as git's instructions say) on git 2.43.

#![allow(clippy::expect_used, clippy::unwrap_used)] // Test helpers may panic: that is a failed test.

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::ops::HunkAction;
use canopy_core::{DiffSource, Error, Hunk, HunkKey, LineKind, MissingObjects, Repo, Whitespace};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
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

/// Raw bytes of a git command's output (index blobs may lack a final newline).
fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
    out.stdout
}

/// A repository whose only commit holds `a.txt` with `content`; identity and signing are set in
/// the repository's own config.
fn fixture(content: &str) -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Ada Lovelace"]);
    git(p, &["config", "user.email", "ada@example.com"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    // Git for Windows sets core.autocrlf=true system-wide, which would rewrite the LF fixtures.
    git(p, &["config", "core.autocrlf", "false"]);
    fs::write(p.join("a.txt"), content).expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: first"]);
    let repo = Repo::open(p).expect("open");
    (dir, repo)
}

fn index_of(p: &Path, path: &str) -> Vec<u8> {
    git_bytes(p, &["show", &format!(":{path}")])
}

fn worktree_of(p: &Path, path: &str) -> Vec<u8> {
    fs::read(p.join(path)).expect("read")
}

/// The hunk lines of `git diff args…`, without the file header (whose blob ids vary).
fn diff_body(p: &Path, args: &[&str]) -> Vec<String> {
    let mut full = vec!["diff"];
    full.extend_from_slice(args);
    git(p, &full)
        .lines()
        .skip_while(|l| !l.starts_with("@@"))
        .map(str::to_owned)
        .collect()
}

fn only_hunk(repo: &Repo, source: &DiffSource, path: &str) -> Hunk {
    let d = repo
        .file_diff(
            source,
            path,
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    assert_eq!(d.hunks.len(), 1, "{d:#?}");
    d.hunks.into_iter().next().expect("one hunk")
}

/// Index in the hunk of the line with `kind` and `text`, as the UI would select it.
fn at(h: &Hunk, kind: LineKind, text: &str) -> usize {
    h.lines
        .iter()
        .position(|l| l.kind == kind && l.text == text)
        .unwrap_or_else(|| panic!("no {kind:?} {text:?} in {h:#?}"))
}

fn apply(repo: &Repo, action: HunkAction, path: &str, h: &Hunk, lines: &[usize]) {
    repo.apply_lines(action, path, 0, h.key(), lines, 3)
        .expect("apply lines");
}

#[test]
fn stages_two_of_five_added_lines() {
    let (dir, repo) = fixture("one\ntwo\nthree\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "one\ntwo\nthree\nA\nB\nC\nD\nE\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    let sel = [at(&h, LineKind::Added, "B"), at(&h, LineKind::Added, "D")];

    apply(&repo, HunkAction::Stage, "a.txt", &h, &sel);

    assert_eq!(index_of(p, "a.txt"), b"one\ntwo\nthree\nB\nD\n");
    assert_eq!(
        diff_body(p, &["--cached"]),
        ["@@ -1,3 +1,5 @@", " one", " two", " three", "+B", "+D"]
    );
    assert_eq!(
        diff_body(p, &[]),
        [
            "@@ -1,5 +1,8 @@",
            " one",
            " two",
            " three",
            "+A",
            " B",
            "+C",
            " D",
            "+E"
        ]
    );
    assert_eq!(worktree_of(p, "a.txt"), b"one\ntwo\nthree\nA\nB\nC\nD\nE\n");
}

#[test]
fn stages_a_subset_of_removals() {
    let (dir, repo) = fixture("one\ntwo\nthree\nfour\nfive\nsix\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "one\nsix\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    let sel = [
        at(&h, LineKind::Removed, "three"),
        at(&h, LineKind::Removed, "five"),
    ];

    apply(&repo, HunkAction::Stage, "a.txt", &h, &sel);

    assert_eq!(index_of(p, "a.txt"), b"one\ntwo\nfour\nsix\n");
    assert_eq!(
        diff_body(p, &[]),
        ["@@ -1,4 +1,2 @@", " one", "-two", "-four", " six"]
    );
}

#[test]
fn stages_only_the_addition_of_a_modified_line() {
    let (dir, repo) = fixture("one\ntwo\nthree\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "one\nTWO\nthree\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");

    apply(
        &repo,
        HunkAction::Stage,
        "a.txt",
        &h,
        &[at(&h, LineKind::Added, "TWO")],
    );

    assert_eq!(index_of(p, "a.txt"), b"one\ntwo\nTWO\nthree\n");
    assert_eq!(
        diff_body(p, &[]),
        ["@@ -1,4 +1,3 @@", " one", "-two", " TWO", " three"]
    );
}

#[test]
fn unstages_a_subset_of_a_staged_hunk() {
    let (dir, repo) = fixture("one\ntwo\nthree\nfour\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "one\nTWO\nTHREE\nfour\n").expect("write");
    git(p, &["add", "a.txt"]);
    let h = only_hunk(&repo, &DiffSource::Staged, "a.txt");
    let sel = [
        at(&h, LineKind::Removed, "three"),
        at(&h, LineKind::Added, "THREE"),
    ];

    apply(&repo, HunkAction::Unstage, "a.txt", &h, &sel);

    assert_eq!(index_of(p, "a.txt"), b"one\nthree\nTWO\nfour\n");
    assert_eq!(
        diff_body(p, &["--cached"]),
        ["@@ -1,4 +1,4 @@", " one", "-two", " three", "+TWO", " four"]
    );
    assert_eq!(
        diff_body(p, &[]),
        [
            "@@ -1,4 +1,4 @@",
            " one",
            "-three",
            " TWO",
            "+THREE",
            " four"
        ]
    );
    assert_eq!(worktree_of(p, "a.txt"), b"one\nTWO\nTHREE\nfour\n");
}

#[test]
fn discards_a_subset_in_the_working_tree() {
    let (dir, repo) = fixture("one\ntwo\nthree\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "one\nA\nB\nthree\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    let sel = [
        at(&h, LineKind::Removed, "two"),
        at(&h, LineKind::Added, "B"),
    ];

    apply(&repo, HunkAction::Discard, "a.txt", &h, &sel);

    assert_eq!(worktree_of(p, "a.txt"), b"one\ntwo\nA\nthree\n");
    assert_eq!(index_of(p, "a.txt"), b"one\ntwo\nthree\n");
}

#[test]
fn keeps_no_newline_markers_with_their_line() {
    // "x\ny\n" -> "x\nY" (no newline): stage only the new last line.
    let (dir, repo) = fixture("x\ny\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "x\nY").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    apply(
        &repo,
        HunkAction::Stage,
        "a.txt",
        &h,
        &[at(&h, LineKind::Added, "Y")],
    );
    assert_eq!(index_of(p, "a.txt"), b"x\ny\nY");

    // Same change, stage only the removal: the dropped "+Y" takes its marker along.
    let (dir, repo) = fixture("x\ny\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "x\nY").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    apply(
        &repo,
        HunkAction::Stage,
        "a.txt",
        &h,
        &[at(&h, LineKind::Removed, "y")],
    );
    assert_eq!(index_of(p, "a.txt"), b"x\n");
    assert_eq!(worktree_of(p, "a.txt"), b"x\nY");

    // Unstage the newline-less addition from a staged hunk.
    let (dir, repo) = fixture("x\ny\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "x\nY").expect("write");
    git(p, &["add", "a.txt"]);
    let h = only_hunk(&repo, &DiffSource::Staged, "a.txt");
    apply(
        &repo,
        HunkAction::Unstage,
        "a.txt",
        &h,
        &[at(&h, LineKind::Added, "Y")],
    );
    assert_eq!(index_of(p, "a.txt"), b"x\n");
}

#[test]
fn refuses_to_glue_a_line_without_newline_to_the_next() {
    // "a\nb" -> "a\nb\nc\n". Staging only "+c" through git's edit mode stages "a\nbc\n".
    let (dir, repo) = fixture("a\nb");
    let p = dir.path();
    fs::write(p.join("a.txt"), "a\nb\nc\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");

    let err = repo
        .apply_lines(
            HunkAction::Stage,
            "a.txt",
            0,
            h.key(),
            &[at(&h, LineKind::Added, "c")],
            3,
        )
        .expect_err("would merge two lines");
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert_eq!(index_of(p, "a.txt"), b"a\nb");

    // Taking the last line's old and new versions along works.
    let all: Vec<usize> = (0..h.lines.len()).collect();
    apply(&repo, HunkAction::Stage, "a.txt", &h, &all);
    assert_eq!(index_of(p, "a.txt"), b"a\nb\nc\n");
}

#[test]
fn refuses_empty_context_only_and_stale_selections() {
    let (dir, repo) = fixture("one\ntwo\nthree\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "one\nTWO\nthree\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    let context = at(&h, LineKind::Context, "one");
    let added = at(&h, LineKind::Added, "TWO");

    let cases: &[(&str, HunkKey, &[usize])] = &[
        ("empty", h.key(), &[]),
        ("context only", h.key(), &[context]),
        ("out of range", h.key(), &[added, 99]),
        ("stale key", (9, 9, 9, 9), &[added]),
    ];
    for (name, key, sel) in cases {
        let err = repo
            .apply_lines(HunkAction::Stage, "a.txt", 0, *key, sel, 3)
            .expect_err(name);
        assert!(matches!(err, Error::Invalid(_)), "{name}: {err:?}");
    }
    let err = repo
        .apply_lines(HunkAction::Stage, "a.txt", 1, h.key(), &[added], 3)
        .expect_err("no such hunk");
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert_eq!(index_of(p, "a.txt"), b"one\ntwo\nthree\n");
}

#[test]
fn untracked_file_lines_can_be_staged_and_discarded() {
    let (dir, repo) = fixture("one\n");
    let p = dir.path();
    fs::write(p.join("n.txt"), "a\nb\nc\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "n.txt");
    apply(
        &repo,
        HunkAction::Stage,
        "n.txt",
        &h,
        &[at(&h, LineKind::Added, "a"), at(&h, LineKind::Added, "c")],
    );
    assert_eq!(index_of(p, "n.txt"), b"a\nc\n");
    assert_eq!(worktree_of(p, "n.txt"), b"a\nb\nc\n");

    // Discarding part of a file that is not in the index edits it instead of deleting it.
    fs::write(p.join("u.txt"), "x\ny\nz\n").expect("write");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "u.txt");
    apply(
        &repo,
        HunkAction::Discard,
        "u.txt",
        &h,
        &[at(&h, LineKind::Added, "y")],
    );
    assert_eq!(worktree_of(p, "u.txt"), b"x\nz\n");
}

#[test]
fn partly_unstaging_a_new_file_keeps_it_in_the_index() {
    let (dir, repo) = fixture("one\n");
    let p = dir.path();
    fs::write(p.join("n.txt"), "a\nb\nc\n").expect("write");
    git(p, &["add", "n.txt"]);
    let h = only_hunk(&repo, &DiffSource::Staged, "n.txt");

    apply(
        &repo,
        HunkAction::Unstage,
        "n.txt",
        &h,
        &[at(&h, LineKind::Added, "b")],
    );
    assert_eq!(index_of(p, "n.txt"), b"a\nc\n");

    // Unstaging every remaining line removes the file from the index, like `git reset -p`.
    let h = only_hunk(&repo, &DiffSource::Staged, "n.txt");
    let all: Vec<usize> = (0..h.lines.len()).collect();
    apply(&repo, HunkAction::Unstage, "n.txt", &h, &all);
    assert_eq!(git(p, &["ls-files", "--", "n.txt"]), "");
    assert_eq!(worktree_of(p, "n.txt"), b"a\nb\nc\n");
}

#[test]
fn partly_staging_a_deletion_keeps_the_rest() {
    let (dir, repo) = fixture("one\ntwo\nthree\n");
    let p = dir.path();
    fs::remove_file(p.join("a.txt")).expect("delete");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");

    apply(
        &repo,
        HunkAction::Stage,
        "a.txt",
        &h,
        &[at(&h, LineKind::Removed, "two")],
    );
    assert_eq!(index_of(p, "a.txt"), b"one\nthree\n");
    assert!(!p.join("a.txt").exists());

    // Discarding one removed line brings the file back with just that line restored.
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    apply(
        &repo,
        HunkAction::Discard,
        "a.txt",
        &h,
        &[at(&h, LineKind::Removed, "three")],
    );
    assert_eq!(worktree_of(p, "a.txt"), b"three\n");
}

#[test]
fn handles_paths_with_spaces_and_non_ascii() {
    let (dir, repo) = fixture("one\n");
    let p = dir.path();
    let mut names = vec!["my notes.txt", "caf\u{e9} \u{2014} n\u{f6}tes.txt"];
    // Windows forbids quotes and tabs in names. Quotes force git to quote the name; a tab ends a
    // name on `---`/`+++` lines unless the name is quoted.
    if cfg!(unix) {
        names.push("caf\u{e9} \"q\".txt");
        names.push("tab\there.txt");
    }
    for name in names {
        fs::write(p.join(name), "a\n").expect("write");
        git(p, &["add", "--", name]);
        fs::write(p.join(name), "a\nb\nc\n").expect("write");
        let h = only_hunk(&repo, &DiffSource::Unstaged, name);
        apply(
            &repo,
            HunkAction::Stage,
            name,
            &h,
            &[at(&h, LineKind::Added, "c")],
        );
        assert_eq!(index_of(p, name), b"a\nc\n", "{name}");
    }
}

#[test]
fn stages_unstages_and_discards_lines_without_context() {
    // Context 0: the hunk sits in the middle of the file with nothing around it. Expected states
    // from `git diff -U0` edited by hand as `git add -p` would, applied with
    // `git apply --cached --unidiff-zero` (git 2.43).
    let original = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let (dir, repo) = fixture(original);
    let p = dir.path();
    fs::write(p.join("a.txt"), "1\n2\n3\n4\nA\nB\nC\n6\n7\n8\n9\n").expect("write");
    let hunk0 = |repo: &Repo, source: &DiffSource| {
        let d = repo
            .file_diff(
                source,
                "a.txt",
                None,
                0,
                Whitespace::Show,
                MissingObjects::Fail,
            )
            .expect("diff");
        assert_eq!(d.hunks.len(), 1, "{d:#?}");
        d.hunks.into_iter().next().expect("one hunk")
    };

    let h = hunk0(&repo, &DiffSource::Unstaged);
    assert_eq!(h.key(), (5, 1, 5, 3));
    repo.apply_lines(
        HunkAction::Stage,
        "a.txt",
        0,
        h.key(),
        &[at(&h, LineKind::Added, "B")],
        0,
    )
    .expect("stage B");
    assert_eq!(index_of(p, "a.txt"), b"1\n2\n3\n4\n5\nB\n6\n7\n8\n9\n");

    let h = hunk0(&repo, &DiffSource::Staged);
    repo.apply_lines(
        HunkAction::Unstage,
        "a.txt",
        0,
        h.key(),
        &[at(&h, LineKind::Added, "B")],
        0,
    )
    .expect("unstage B");
    assert_eq!(index_of(p, "a.txt"), original.as_bytes());

    let h = hunk0(&repo, &DiffSource::Unstaged);
    repo.apply_lines(
        HunkAction::Discard,
        "a.txt",
        0,
        h.key(),
        &[at(&h, LineKind::Removed, "5"), at(&h, LineKind::Added, "A")],
        0,
    )
    .expect("discard -5 +A");
    assert_eq!(
        worktree_of(p, "a.txt"),
        b"1\n2\n3\n4\n5\nB\nC\n6\n7\n8\n9\n"
    );
    assert_eq!(index_of(p, "a.txt"), original.as_bytes());
}

#[test]
fn crlf_with_autocrlf_matches_git() {
    let (dir, _) = fixture("one\ntwo\n");
    let p = dir.path();
    git(p, &["config", "core.autocrlf", "true"]);
    let repo = Repo::open(p).expect("reopen with the new config");
    fs::write(p.join("a.txt"), "one\r\ntwo\r\nA\r\nB\r\n").expect("write");

    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    apply(
        &repo,
        HunkAction::Stage,
        "a.txt",
        &h,
        &[at(&h, LineKind::Added, "B")],
    );
    assert_eq!(index_of(p, "a.txt"), b"one\ntwo\nB\n");

    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");
    apply(
        &repo,
        HunkAction::Discard,
        "a.txt",
        &h,
        &[at(&h, LineKind::Added, "A")],
    );
    assert_eq!(worktree_of(p, "a.txt"), b"one\r\ntwo\r\nB\r\n");
}

#[cfg(unix)]
#[test]
fn staging_lines_leaves_the_executable_bit_alone() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, repo) = fixture("one\n");
    let p = dir.path();
    fs::write(p.join("a.txt"), "one\ntwo\nthree\n").expect("write");
    fs::set_permissions(p.join("a.txt"), fs::Permissions::from_mode(0o755)).expect("chmod");
    let h = only_hunk(&repo, &DiffSource::Unstaged, "a.txt");

    apply(
        &repo,
        HunkAction::Stage,
        "a.txt",
        &h,
        &[at(&h, LineKind::Added, "three")],
    );
    assert_eq!(index_of(p, "a.txt"), b"one\nthree\n");
    assert!(git(p, &["ls-files", "-s", "a.txt"]).starts_with("100644 "));
}
