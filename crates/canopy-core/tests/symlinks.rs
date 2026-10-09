//! Symbolic links and file type changes, against throwaway repositories built with the `git` CLI.
//! A link is never followed: a secret file outside the repository, which the links point to, must
//! never appear in anything Canopy reads.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use canopy_core::{ChangeKind, DiffSource, LineKind, MissingObjects, Repo, Whitespace};

const SECRET: &str = "TOP-SECRET-OUTSIDE-THE-REPOSITORY";

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

/// A repository with `file.txt`, `link` (to `file.txt`) and `dir/x.txt`, committed, beside a
/// folder outside it holding the secret.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Repo) {
    let root = tempfile::tempdir().expect("tempdir");
    let outside = root.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), SECRET).unwrap();
    let p = root.path().join("repo");
    fs::create_dir_all(p.join("dir")).unwrap();
    git(&p, &["init", "-q", "-b", "main"]);
    git(&p, &["config", "user.name", "Ada Lovelace"]);
    git(&p, &["config", "user.email", "ada@example.com"]);
    git(&p, &["config", "core.symlinks", "true"]);
    fs::write(p.join("file.txt"), "plain\n").unwrap();
    fs::write(p.join("dir/x.txt"), "x\n").unwrap();
    symlink("file.txt", p.join("link")).unwrap();
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "base"]);
    let repo = Repo::open(&p).unwrap();
    (root, p, repo)
}

fn diff(repo: &Repo, source: &DiffSource, path: &str) -> canopy_core::FileDiff {
    repo.file_diff(
        source,
        path,
        None,
        3,
        Whitespace::Show,
        MissingObjects::Fail,
    )
    .unwrap()
}

/// The diff's lines as `-text` and `+text`, hunk by hunk.
fn signed(d: &canopy_core::FileDiff) -> Vec<String> {
    d.hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| {
            let sign = match l.kind {
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::Context => ' ',
            };
            format!("{sign}{}", l.text)
        })
        .collect()
}

fn changed_lines(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter(|l| {
            (l.starts_with('+') || l.starts_with('-'))
                && !l.starts_with("+++")
                && !l.starts_with("---")
        })
        .map(str::to_owned)
        .collect()
}

/// What `git diff` (or `--cached`) prints as changed lines for `path`, as `-text` and `+text`.
fn git_lines(p: &Path, cached: bool, path: &str) -> Vec<String> {
    let mut args = vec!["diff", "--no-color"];
    if cached {
        args.push("--cached");
    }
    args.extend(["--", path]);
    changed_lines(&git(p, &args))
}

fn never_the_secret(what: &dyn std::fmt::Debug) {
    assert!(
        !format!("{what:?}").contains(SECRET),
        "a link was followed: {what:?}"
    );
}

#[test]
fn a_file_replaced_by_a_link_outside_the_repository_shows_its_target_not_the_secret() {
    let (root, p, repo) = fixture();
    let target = root.path().join("outside").join("secret.txt");
    fs::remove_file(p.join("file.txt")).unwrap();
    symlink(&target, p.join("file.txt")).unwrap();
    assert!(git(&p, &["status", "--porcelain"]).contains(" T file.txt"));

    let d = diff(&repo, &DiffSource::Unstaged, "file.txt");
    assert_eq!(d.kind, Some(ChangeKind::Typechange));
    assert_eq!(d.mode_change(), Some((0o100_644, 0o120_000)));
    assert_eq!(
        signed(&d),
        ["-plain".to_owned(), format!("+{}", target.display())]
    );
    assert_eq!(signed(&d), git_lines(&p, false, "file.txt"));
    never_the_secret(&d);
    never_the_secret(&repo.status(MissingObjects::Fail).unwrap());

    // The same change, staged.
    git(&p, &["add", "file.txt"]);
    let d = diff(&repo, &DiffSource::Staged, "file.txt");
    assert_eq!(signed(&d), git_lines(&p, true, "file.txt"));
    never_the_secret(&d);
    assert_eq!((d.additions, d.deletions), (1, 1));

    // And as a commit.
    let oid = repo.commit("refactor: link it", false, false).unwrap();
    let d = diff(&repo, &DiffSource::Commit(oid), "file.txt");
    assert_eq!(d.kind, Some(ChangeKind::Typechange));
    assert_eq!(
        signed(&d),
        changed_lines(&git(&p, &["show", "--format=", "--", "file.txt"]))
    );
    never_the_secret(&d);
}

#[test]
fn a_link_replaced_by_a_file_shows_the_old_target_and_the_new_text() {
    let (_root, p, repo) = fixture();
    fs::remove_file(p.join("link")).unwrap();
    fs::write(p.join("link"), "now a file\nof two lines\n").unwrap();
    let d = diff(&repo, &DiffSource::Unstaged, "link");
    assert_eq!(d.kind, Some(ChangeKind::Typechange));
    assert_eq!(d.mode_change(), Some((0o120_000, 0o100_644)));
    assert_eq!(
        signed(&d),
        ["-file.txt", "+now a file", "+of two lines"].map(str::to_owned)
    );
    assert_eq!(signed(&d), git_lines(&p, false, "link"));
    git(&p, &["add", "link"]);
    let d = diff(&repo, &DiffSource::Staged, "link");
    assert_eq!(signed(&d), git_lines(&p, true, "link"));
}

#[test]
fn a_retargeted_link_shows_both_targets_and_never_follows_either() {
    let (root, p, repo) = fixture();
    let target = root.path().join("outside").join("secret.txt");
    fs::remove_file(p.join("link")).unwrap();
    symlink(&target, p.join("link")).unwrap();
    let d = diff(&repo, &DiffSource::Unstaged, "link");
    assert_eq!(d.kind, Some(ChangeKind::Modified));
    assert_eq!(
        signed(&d),
        ["-file.txt".to_owned(), format!("+{}", target.display())]
    );
    assert_eq!(signed(&d), git_lines(&p, false, "link"));
    never_the_secret(&d);
}

#[test]
fn a_link_to_a_folder_is_one_entry_whose_content_is_never_listed() {
    let (_root, p, repo) = fixture();
    symlink("dir", p.join("dirlink")).unwrap();
    let st = repo.status(MissingObjects::Fail).unwrap();
    let paths: Vec<_> = st.unstaged.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["dirlink"], "{st:?}");
    assert_eq!(git(&p, &["status", "--porcelain"]), "?? dirlink\n");
    let d = diff(&repo, &DiffSource::Unstaged, "dirlink");
    assert_eq!(signed(&d), ["+dir"]);
    assert_eq!(st.unstaged[0].new_mode, Some(0o120_000));
}

#[test]
fn a_binary_file_replaced_by_a_link_is_not_shown_as_text() {
    let (root, p, repo) = fixture();
    fs::write(p.join("blob.bin"), [0_u8, 1, 2, 3, 0]).unwrap();
    git(&p, &["add", "blob.bin"]);
    git(&p, &["commit", "-q", "-m", "binary"]);
    fs::remove_file(p.join("blob.bin")).unwrap();
    symlink(
        root.path().join("outside").join("secret.txt"),
        p.join("blob.bin"),
    )
    .unwrap();
    let d = diff(&repo, &DiffSource::Unstaged, "blob.bin");
    assert_eq!(d.kind, Some(ChangeKind::Typechange));
    assert!(d.binary, "{d:?}");
    never_the_secret(&d);
}
