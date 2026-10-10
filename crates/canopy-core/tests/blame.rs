//! Blame of a file against what `git blame` says: the commit of every line, the author, renames,
//! a chosen revision and uncommitted changes.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::{MissingObjects, Repo};

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git_as(dir: &Path, who: &str, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", who)
        .env(
            "GIT_AUTHOR_EMAIL",
            format!("{}@example.com", who.to_lowercase()),
        )
        .env("GIT_COMMITTER_NAME", who)
        .env(
            "GIT_COMMITTER_EMAIL",
            format!("{}@example.com", who.to_lowercase()),
        )
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn commit(dir: &Path, who: &str, file: &str, content: &str, message: &str) {
    fs::write(dir.join(file), content).expect("write");
    git_as(dir, who, &["add", "--", file]);
    git_as(dir, who, &["commit", "-q", "-m", message]);
}

fn repository() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    git_as(dir.path(), "Ada", &["init", "-q", "-b", "main"]);
    let repo = Repo::open(dir.path()).expect("open");
    (dir, repo)
}

/// The commit id git blames each line on (`git blame -l`, with `--root` so the first commit is not a `^` boundary).
fn oracle(dir: &Path, args: &[&str]) -> Vec<String> {
    git_as(dir, "Ada", args)
        .lines()
        .map(|l| {
            let id = l.split(' ').next().expect("id");
            id.trim_start_matches('^').chars().take(40).collect()
        })
        .collect()
}

fn ids(blame: &canopy_core::Blame) -> Vec<String> {
    blame
        .lines
        .iter()
        .map(|l| blame.commits[l.commit].oid.to_string())
        .collect()
}

#[test]
fn every_line_names_the_commit_and_author_git_blames() {
    let (dir, repo) = repository();
    let p = dir.path();
    commit(p, "Ada", "f.txt", "one\ntwo\nthree\n", "add f");
    commit(
        p,
        "Bo",
        "f.txt",
        "one\nTWO\nthree\nfour\n",
        "change two, add four",
    );

    let blame = repo
        .blame("f.txt", None, MissingObjects::Fail, true)
        .expect("blame");
    assert_eq!(
        ids(&blame),
        oracle(p, &["blame", "--root", "-l", "--", "f.txt"])
    );
    let texts: Vec<_> = blame.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["one", "TWO", "three", "four"]);
    for c in &blame.commits {
        let show = |format: &str| {
            git_as(p, "Ada", &["log", "-1", format, &c.oid.to_string()])
                .trim()
                .to_owned()
        };
        assert_eq!(c.author, show("--format=%an"));
        assert_eq!(c.email, show("--format=%ae"));
        assert_eq!(c.summary, show("--format=%s"));
        assert_eq!(c.time.to_string(), show("--format=%at"));
    }
    assert_eq!(blame.commits.len(), 2);
}

#[test]
fn a_revision_blames_the_file_as_it_was_then() {
    let (dir, repo) = repository();
    let p = dir.path();
    commit(p, "Ada", "f.txt", "one\ntwo\n", "add f");
    let first = git2::Oid::from_str(git_as(p, "Ada", &["rev-parse", "HEAD"]).trim()).expect("id");
    commit(p, "Bo", "f.txt", "one\ntwo\nthree\n", "add three");

    let then = repo
        .blame("f.txt", Some(first), MissingObjects::Fail, true)
        .expect("blame");
    assert_eq!(then.lines.len(), 2);
    assert_eq!(
        ids(&then),
        oracle(
            p,
            &["blame", "--root", "-l", &first.to_string(), "--", "f.txt"]
        )
    );
    assert_eq!(
        repo.blame("f.txt", None, MissingObjects::Fail, true)
            .expect("blame")
            .lines
            .len(),
        3
    );
}

#[test]
fn renames_are_followed_and_odd_names_are_safe() {
    let (dir, repo) = repository();
    let p = dir.path();
    commit(p, "Ada", "-old name.txt", "one\ntwo\n", "add");
    git_as(p, "Bo", &["mv", "--", "-old name.txt", "-new name.txt"]);
    git_as(p, "Bo", &["commit", "-q", "-m", "rename"]);

    let blame = repo
        .blame("-new name.txt", None, MissingObjects::Fail, true)
        .expect("blame");
    assert_eq!(
        ids(&blame),
        oracle(p, &["blame", "--root", "-l", "--", "-new name.txt"])
    );
    assert_eq!(blame.commits[blame.lines[0].commit].author, "Ada");
}

#[test]
fn uncommitted_lines_are_marked_and_a_missing_file_is_git_s_error() {
    let (dir, repo) = repository();
    let p = dir.path();
    commit(p, "Ada", "f.txt", "one\n", "add f");
    fs::write(p.join("f.txt"), "one\nlocal\n").expect("write");

    let blame = repo
        .blame("f.txt", None, MissingObjects::Fail, true)
        .expect("blame");
    assert!(!blame.commits[blame.lines[0].commit].is_uncommitted());
    assert!(blame.commits[blame.lines[1].commit].is_uncommitted());
    assert_eq!(
        ids(&blame),
        oracle(p, &["blame", "--root", "-l", "--", "f.txt"])
    );

    let error = repo
        .blame("nope.txt", None, MissingObjects::Fail, true)
        .expect_err("no such file");
    assert!(error.to_string().contains("no such path"), "{error}");
}

#[test]
fn an_untrusted_folder_never_runs_the_repository_s_programs() {
    let (dir, repo) = repository();
    let p = dir.path();
    fs::write(p.join(".gitattributes"), "f.txt filter=x diff=y\n").expect("write");
    git_as(p, "Ada", &["add", "--", ".gitattributes"]);
    commit(p, "Ada", "f.txt", "one\n", "add f");
    // Programs that leave a marker when git runs them.
    let marker = |name: &str| format!("sh -c 'touch {}/{name}; cat' --", p.display());
    git_as(p, "Ada", &["config", "filter.x.clean", &marker("CLEAN")]);
    git_as(
        p,
        "Ada",
        &["config", "diff.y.textconv", &marker("TEXTCONV")],
    );
    fs::write(p.join("f.txt"), "one\nlocal\n").expect("write");

    // A commit is read without the textconv, trusted or not.
    repo.blame("f.txt", Some(git_oid(p)), MissingObjects::Fail, false)
        .expect("a commit blames");
    repo.blame("f.txt", Some(git_oid(p)), MissingObjects::Fail, true)
        .expect("a commit blames");
    assert!(!p.join("TEXTCONV").exists());

    // The working tree needs the clean filter: refused until the folder is trusted.
    let error = repo
        .blame("f.txt", None, MissingObjects::Fail, false)
        .expect_err("refused");
    assert!(error.to_string().contains("Trust the folder"), "{error}");
    assert!(!p.join("CLEAN").exists());
    let blame = repo
        .blame("f.txt", None, MissingObjects::Fail, true)
        .expect("trusted");
    assert_eq!(blame.lines.len(), 2);
    assert!(p.join("CLEAN").exists());
}

fn git_oid(dir: &Path) -> git2::Oid {
    git2::Oid::from_str(git_as(dir, "Ada", &["rev-parse", "HEAD"]).trim()).expect("id")
}
