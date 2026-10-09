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
fn run(dir: &Path, args: &[&str], editor: Option<&str>) -> std::process::Output {
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

fn git(dir: &Path, args: &[&str], editor: Option<&str>) -> String {
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

    // The oracle: the text `git commit` hands its editor.
    let capture = dir.path().join("capture.sh");
    let seen = dir.path().join("seen.txt");
    fs::write(
        &capture,
        "#!/bin/sh\ncp \"$1\" \"$(dirname \"$0\")/seen.txt\"\nexit 1\n",
    )
    .expect("write");
    // Git for Windows also runs editors through sh; native backslashes are shell escapes.
    let script = capture.to_str().expect("UTF8").replace('\\', "/");
    let editor = format!("sh '{}'", script.replace('\'', "'\\''"));
    fs::write(dir.path().join("f"), "x").expect("write");
    git(dir.path(), &["add", "f"], None);
    // The editor exits non-zero on purpose: only the text it was given matters.
    let output = run(dir.path(), &["commit"], Some(&editor));
    assert!(
        !output.status.success(),
        "the capture editor aborts the commit"
    );
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

#[cfg(unix)]
#[test]
fn a_template_that_is_not_a_regular_file_is_refused_not_read() {
    let (_dir, repo) = repository();
    git(
        repo.workdir(),
        &["config", "commit.template", "/dev/zero"],
        None,
    );
    let error = repo
        .commit_template()
        .expect_err("a device is not a template");
    assert!(error.to_string().contains("not a regular file"), "{error}");
}

#[test]
fn a_huge_template_is_refused() {
    let (dir, repo) = repository();
    fs::write(dir.path().join("big"), vec![b'x'; 300 * 1024]).expect("write");
    git(repo.workdir(), &["config", "commit.template", "big"], None);
    let error = repo.commit_template().expect_err("too large");
    assert!(error.to_string().contains("too large"), "{error}");
}
