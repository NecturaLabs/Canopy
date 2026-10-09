//! Signature status of commits, with SSH signing so no keyring is needed, against what
//! `git log --format=%G?` says.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::{Repo, SignatureStatus};

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git and ssh-keygen directly to build and check repositories"
)]
fn run(program: &str, dir: &Path, args: &[&str]) -> String {
    let out = Command::new(program)
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("program runs");
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn git(dir: &Path, args: &[&str]) -> String {
    run("git", dir, args)
}

fn oid(p: &Path, rev: &str) -> git2::Oid {
    git2::Oid::from_str(git(p, &["rev-parse", rev]).trim()).expect("an id")
}

fn letter(p: &Path, rev: &str) -> String {
    git(p, &["log", "-1", "--format=%G?", rev])
        .trim()
        .to_owned()
}

/// A repository whose signing and verifying use a fresh SSH key; its allowed-signers file lists
/// that key unless `known` is false.
fn repository(known: bool) -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    let key = p.join("key");
    run(
        "ssh-keygen",
        p,
        &["-q", "-t", "ed25519", "-N", "", "-C", "ada", "-f", "key"],
    );
    let public = fs::read_to_string(p.join("key.pub")).expect("public key");
    let signers = p.join("signers");
    let listed = if known {
        format!("ada@example.com {public}")
    } else {
        String::new()
    };
    fs::write(&signers, listed).expect("write");
    for (name, value) in [
        ("user.name", "Ada"),
        ("user.email", "ada@example.com"),
        ("gpg.format", "ssh"),
        (
            "user.signingkey",
            &key.with_extension("pub").to_string_lossy(),
        ),
        ("gpg.ssh.allowedSignersFile", &signers.to_string_lossy()),
        ("core.autocrlf", "false"),
    ] {
        git(p, &["config", name, value]);
    }
    let repo = Repo::open(p).expect("open");
    (dir, repo)
}

fn commit(p: &Path, file: &str, sign: bool) {
    fs::write(p.join(file), file).expect("write");
    git(p, &["add", "--", file]);
    let flag = if sign { "-S" } else { "--no-gpg-sign" };
    git(p, &["commit", "-q", flag, "-m", file]);
}

#[test]
fn an_unsigned_commit_is_unsigned() {
    let (dir, repo) = repository(true);
    commit(dir.path(), "a", false);
    assert_eq!(letter(dir.path(), "HEAD"), "N");
    assert_eq!(
        repo.commit_signature(oid(dir.path(), "HEAD"))
            .expect("status"),
        SignatureStatus::Unsigned
    );
}

#[test]
fn a_signature_from_a_listed_key_is_verified() {
    let (dir, repo) = repository(true);
    commit(dir.path(), "a", true);
    assert_eq!(letter(dir.path(), "HEAD"), "G");
    let status = repo
        .commit_signature(oid(dir.path(), "HEAD"))
        .expect("status");
    assert!(status.is_trusted(), "{status:?}");
}

#[test]
fn a_signature_from_an_unlisted_key_is_never_shown_as_trusted() {
    let (dir, repo) = repository(false);
    commit(dir.path(), "a", true);
    let expected = letter(dir.path(), "HEAD");
    assert_ne!(expected, "G");
    let status = repo
        .commit_signature(oid(dir.path(), "HEAD"))
        .expect("status");
    assert!(!status.is_trusted(), "{status:?} for git's {expected}");
    assert_ne!(status, SignatureStatus::Unsigned, "git's {expected}");
}

#[test]
fn a_signed_commit_git_cannot_check_is_not_shown_as_unsigned() {
    let (dir, repo) = repository(true);
    commit(dir.path(), "a", true);
    git(
        dir.path(),
        &["config", "--unset", "gpg.ssh.allowedSignersFile"],
    );
    assert_eq!(letter(dir.path(), "HEAD"), "N", "git cannot check it");
    assert_eq!(
        repo.commit_signature(oid(dir.path(), "HEAD"))
            .expect("status"),
        SignatureStatus::Unavailable
    );
}
