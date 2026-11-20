//! Signature status of commits, with SSH signing so no keyring is needed, against what
//! `git log --format=%G?` says.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

mod support;

use std::fs;
use std::path::Path;
use std::process::Command;

use support::isolate_git;

use canopy_core::{Repo, SignatureStatus};

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git and ssh-keygen directly to build and check repositories"
)]
fn run(program: &str, dir: &Path, args: &[&str]) -> String {
    let out = Command::new(program)
        .current_dir(dir)
        .args(args)
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
    isolate_git!(an_unsigned_commit_is_unsigned);
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
    isolate_git!(a_signature_from_a_listed_key_is_verified);
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
    isolate_git!(a_signature_from_an_unlisted_key_is_never_shown_as_trusted);
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
    isolate_git!(a_signed_commit_git_cannot_check_is_not_shown_as_unsigned);
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

/// The folder of a throwaway keyring; dropping it stops the `gpg-agent` the keyring started.
#[cfg(unix)]
struct Keyring(tempfile::TempDir);

#[cfg(unix)]
impl std::ops::Deref for Keyring {
    type Target = tempfile::TempDir;
    fn deref(&self) -> &tempfile::TempDir {
        &self.0
    }
}

#[cfg(unix)]
impl Drop for Keyring {
    #[expect(
        clippy::disallowed_methods,
        reason = "stops the agent this test's keyring started"
    )]
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .args(["--homedir"])
            .arg(self.0.path().join("gnupg"))
            .args(["--kill", "gpg-agent"])
            .output();
    }
}

/// A repository that signs with `OpenPGP` through a throwaway keyring: `gpg.program` is a script
/// that points gpg at it. `None` when `gpg` is not installed.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test asks whether gpg is installed before it builds a keyring"
)]
fn gpg_repository() -> Option<(Keyring, Repo)> {
    use std::os::unix::fs::PermissionsExt;

    if Command::new("gpg").arg("--version").output().is_err() {
        return None;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let home = p.join("gnupg");
    fs::create_dir(&home).expect("keyring");
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("mode");
    let program = p.join("gpg-here");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nexec gpg --homedir '{}' \"$@\"\n",
            home.display()
        ),
    )
    .expect("script");
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("mode");
    run(
        "gpg",
        p,
        &[
            "--homedir",
            &home.to_string_lossy(),
            "--batch",
            "--passphrase",
            "",
            "--quick-gen-key",
            "Ada <ada@example.com>",
            "ed25519",
            "sign",
            "never",
        ],
    );
    git(p, &["init", "-q", "-b", "main"]);
    for (name, value) in [
        ("user.name", "Ada"),
        ("user.email", "ada@example.com"),
        ("user.signingkey", "ada@example.com"),
        ("gpg.program", &program.to_string_lossy()),
        ("core.autocrlf", "false"),
    ] {
        git(p, &["config", name, value]);
    }
    let repo = Repo::open(p).expect("open");
    Some((Keyring(dir), repo))
}

#[cfg(unix)]
#[test]
fn a_gpg_signature_from_a_trusted_key_is_verified() {
    isolate_git!(a_gpg_signature_from_a_trusted_key_is_verified);
    let Some((dir, repo)) = gpg_repository() else {
        return;
    };
    commit(dir.path(), "a", true);
    // gpg trusts the keys it generates ultimately.
    assert_eq!(letter(dir.path(), "HEAD"), "G");
    let status = repo
        .commit_signature(oid(dir.path(), "HEAD"))
        .expect("status");
    assert!(
        matches!(&status, SignatureStatus::Verified(s) if s.name.contains("ada@example.com")),
        "{status:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_gpg_signature_from_a_key_you_do_not_trust_is_never_shown_as_verified() {
    isolate_git!(a_gpg_signature_from_a_key_you_do_not_trust_is_never_shown_as_verified);
    let Some((dir, repo)) = gpg_repository() else {
        return;
    };
    let fingerprint = run(
        "gpg",
        dir.path(),
        &[
            "--homedir",
            &dir.path().join("gnupg").to_string_lossy(),
            "--with-colons",
            "--list-keys",
            "ada@example.com",
        ],
    )
    .lines()
    .find_map(|l| l.strip_prefix("fpr:::::::::"))
    .map(|l| l.trim_end_matches(':').to_owned())
    .expect("a fingerprint");
    let trust = dir.path().join("trust");
    fs::write(&trust, format!("{fingerprint}:3:\n")).expect("write");
    run(
        "gpg",
        dir.path(),
        &[
            "--homedir",
            &dir.path().join("gnupg").to_string_lossy(),
            "--batch",
            "--import-ownertrust",
            &trust.to_string_lossy(),
        ],
    );
    commit(dir.path(), "a", true);
    assert_eq!(letter(dir.path(), "HEAD"), "U");
    let status = repo
        .commit_signature(oid(dir.path(), "HEAD"))
        .expect("status");
    assert!(
        matches!(&status, SignatureStatus::Untrusted(_)) && !status.is_trusted(),
        "{status:?}"
    );
}
