//! The commit template Canopy offers is what `git commit` would open the editor with, minus the
//! comment lines it strips.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::Repo;

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn run(dir: &Path, args: &[&str], editor: Option<&Path>) -> std::process::Output {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    if let Some(editor) = editor {
        command.env("GIT_EDITOR", editor);
    }
    command.output().expect("git runs")
}

fn git(dir: &Path, args: &[&str], editor: Option<&Path>) -> String {
    let out = run(dir, args, editor);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repository() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"], None);
    git(dir.path(), &["config", "user.name", "T"], None);
    git(dir.path(), &["config", "user.email", "t@example.com"], None);
    let repo = Repo::open(dir.path()).expect("open");
    (dir, repo)
}

#[test]
fn no_template_means_none() {
    let (_dir, repo) = repository();
    assert_eq!(repo.commit_template().expect("template"), None);
}

#[test]
fn a_relative_template_matches_what_the_editor_gets_without_its_comments() {
    let (dir, repo) = repository();
    let template = "feat(scope): summary\n\nWhy:\n\n# Please explain the change.\n# Lines starting with # are removed.\n";
    fs::write(dir.path().join("TEMPLATE"), template).expect("write");
    git(dir.path(), &["config", "commit.template", "TEMPLATE"], None);

    // The oracle: the text `git commit` hands its editor, from a subdirectory too.
    let capture = dir.path().join("capture.sh");
    let seen = dir.path().join("seen.txt");
    fs::write(
        &capture,
        format!("#!/bin/sh\ncp \"$1\" '{}'\nexit 1\n", seen.display()),
    )
    .expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    fs::write(dir.path().join("f"), "x").expect("write");
    git(dir.path(), &["add", "f"], None);
    // The editor exits non-zero on purpose: only the text it was given matters.
    run(dir.path(), &["commit"], Some(&capture));
    let editor_text = fs::read_to_string(&seen).expect("the editor ran");
    assert!(editor_text.starts_with(template), "{editor_text}");

    let got = repo.commit_template().expect("template").expect("some");
    assert_eq!(got, "feat(scope): summary\n\nWhy:");
}

#[test]
fn a_missing_template_file_says_so() {
    let (dir, repo) = repository();
    git(
        dir.path(),
        &["config", "commit.template", "nowhere.txt"],
        None,
    );
    let error = repo.commit_template().expect_err("missing");
    assert!(error.to_string().contains("nowhere.txt"), "{error}");
}
