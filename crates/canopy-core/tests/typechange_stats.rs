//! Commit line counts for type changes use both complete blobs, not the diff display limit.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixture helpers report failures by panicking"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::{ChangeKind, MissingObjects, Repo};

#[expect(
    clippy::disallowed_methods,
    reason = "tests use Git to construct indexed file types and provide the numstat oracle"
)]
fn git(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(path)
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

fn counts(old: &[u8], new: &[u8], old_mode: &str, new_mode: &str) -> (usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.name", "Canopy CI"]);
    git(path, &["config", "user.email", "ci@canopy.invalid"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    git(path, &["config", "core.hooksPath", ""]);
    git(path, &["config", "core.autocrlf", "false"]);
    git(path, &["commit", "-q", "--allow-empty", "-m", "base"]);
    fs::write(path.join("old-blob"), old).unwrap();
    fs::write(path.join("new-blob"), new).unwrap();
    let old_id = git(path, &["hash-object", "-w", "--", "old-blob"]);
    let new_id = git(path, &["hash-object", "-w", "--", "new-blob"]);
    for (mode, id, message) in [
        (old_mode, old_id.trim(), "before"),
        (new_mode, new_id.trim(), "after"),
    ] {
        let commit = git(path, &["rev-parse", "HEAD"]);
        let id = if mode == "160000" { commit.trim() } else { id };
        git(
            path,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{mode},{id},changed"),
            ],
        );
        git(path, &["commit", "-q", "-m", message]);
    }
    let repo = Repo::open(path).unwrap();
    let oid = repo.git().head().unwrap().target().unwrap();
    let details = repo.commit_details(oid, MissingObjects::Fail).unwrap();
    assert!(details.stats_known);
    assert_eq!(details.files.len(), 1);
    let entry = &details.files[0];
    assert_eq!(entry.kind, ChangeKind::Typechange);
    let actual = (entry.additions, entry.deletions);
    let expected = git(path, &["show", "--numstat", "--format=", "--", "changed"]);
    assert_eq!(
        expected.trim(),
        format!("{}\t{}\tchanged", actual.0, actual.1),
        "{old_mode} -> {new_mode}, {} -> {} bytes",
        old.len(),
        new.len()
    );
    actual
}

#[test]
fn large_type_changes_count_both_blob_sides() {
    let large = "a long line of text\n".repeat(20_000);
    assert_eq!(
        counts(large.as_bytes(), b"target", "100644", "120000"),
        (1, 20_000)
    );
    assert_eq!(
        counts(b"target", large.as_bytes(), "120000", "100644"),
        (20_000, 1)
    );
}

#[test]
fn large_file_and_gitlink_changes_count_text_and_pointer() {
    let large = "a long line of text\n".repeat(20_000);
    assert_eq!(
        counts(large.as_bytes(), b"", "100644", "160000"),
        (1, 20_000)
    );
    assert_eq!(
        counts(b"", large.as_bytes(), "160000", "100644"),
        (20_000, 1)
    );
}

#[test]
fn typechange_counts_handle_empty_and_unterminated_text() {
    for (text, lines) in [
        (&b""[..], 0),
        (&b"one\n"[..], 1),
        (&b"one"[..], 1),
        (&b"one\r\ntwo\r\n"[..], 2),
        (&b"one\r\ntwo"[..], 2),
        (&b"\xff\nlast\r"[..], 2),
    ] {
        assert_eq!(counts(text, b"target", "100644", "120000"), (1, lines));
        assert_eq!(counts(b"target", text, "120000", "100644"), (lines, 1));
    }
}
