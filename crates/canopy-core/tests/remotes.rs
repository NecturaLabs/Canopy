//! Changing a remote's URL, against temporary repositories and compared with the git CLI.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::path::Path;
use std::process::Command;

use canopy_core::Repo;
use canopy_core::ops::url_has_credentials;

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repository() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(
        dir.path(),
        &["remote", "add", "origin", "https://github.com/o/r.git"],
    );
    let repo = Repo::open(dir.path()).expect("open");
    (dir, repo)
}

#[test]
fn a_remote_url_changes_as_git_remote_set_url_changes_it() {
    let (dir, repo) = repository();
    repo.set_remote_url("origin", "  git@github.com:o/r.git ")
        .expect("set-url");
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "origin"]),
        "git@github.com:o/r.git"
    );
    // The push URL follows when none of its own is set, as with the CLI.
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "--push", "origin"]),
        "git@github.com:o/r.git"
    );
    // A URL that looks like an option is refused, and nothing changes.
    repo.set_remote_url("origin", "-oops").expect_err("refused");
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "origin"]),
        "git@github.com:o/r.git"
    );
}

#[test]
fn a_missing_remote_reports_gits_own_message() {
    let (_dir, repo) = repository();
    let err = repo
        .set_remote_url("upstream", "https://github.com/o/r.git")
        .expect_err("no such remote");
    assert!(err.to_string().contains("upstream"), "{err}");
}

#[test]
fn credentials_never_reach_the_configuration() {
    let (dir, repo) = repository();
    for url in [
        "https://ghp_secret@github.com/o/r.git",
        "https://user:pass@gitlab.example.com/g/r.git",
        "ssh://git:secret@host/r.git",
    ] {
        let err = repo
            .set_remote_url("origin", url)
            .expect_err("credentials refused");
        assert!(err.to_string().contains("user name and password"), "{err}");
        assert!(!err.to_string().contains("secret"), "{err}");
    }
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "origin"]),
        "https://github.com/o/r.git",
        "nothing was written"
    );
    assert!(
        repo.set_remote_url("-x", "https://github.com/o/r.git")
            .is_err()
    );
    assert!(repo.set_remote_url("origin", "   ").is_err());
}

#[test]
fn a_new_remote_takes_neither_credentials_nor_options() {
    let (dir, repo) = repository();
    let err = repo
        .add_remote("mine", "https://user:secret@github.com/o/r.git")
        .expect_err("credentials refused");
    assert!(err.to_string().contains("user name and password"), "{err}");
    assert!(!err.to_string().contains("secret"), "{err}");
    for (name, url) in [
        ("-t", "https://github.com/o/r.git"),
        ("mine", "--mirror=push"),
        ("", "https://github.com/o/r.git"),
        ("mine", "  "),
    ] {
        repo.add_remote(name, url)
            .expect_err(&format!("{name:?} {url:?} refused"));
    }
    assert_eq!(git(dir.path(), &["remote"]), "origin", "nothing was added");
    repo.add_remote("mine", " https://github.com/me/r.git ")
        .expect("add");
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "mine"]),
        "https://github.com/me/r.git"
    );
    repo.remove_remote("mine").expect("remove");
    assert_eq!(git(dir.path(), &["remote"]), "origin");
}

#[test]
fn a_clone_url_with_credentials_is_refused_before_git_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("r");
    let err = canopy_core::ops::clone(
        "https://user:secret@127.0.0.1:9/o/r.git",
        &dest,
        false,
        |_| {},
    )
    .expect_err("credentials refused");
    assert!(err.to_string().contains("user name and password"), "{err}");
    assert!(!err.to_string().contains("secret"), "{err}");
    assert!(!dest.exists());
}

#[test]
fn credentials_are_recognised_in_urls() {
    assert!(url_has_credentials("https://token@github.com/o/r"));
    assert!(url_has_credentials("HTTP://u:p@host/r"));
    assert!(url_has_credentials("ssh://git:pw@host/r"));
    assert!(!url_has_credentials("ssh://git@host/r"));
    assert!(!url_has_credentials("git@github.com:o/r.git"));
    assert!(!url_has_credentials("https://github.com/o/r@v2"));
    assert!(!url_has_credentials("/srv/git/r.git"));
}

/// Git and libcurl skip extra slashes after `://`, and git takes everything before the first `@`
/// ahead of the first slash as the user name and password.
#[test]
fn credentials_are_recognised_in_every_spelling_git_accepts() {
    for url in [
        "https:///user:secret@github.com/o/r.git",
        "https:////token@github.com/o/r",
        "HTTPS://u%3Asecret@host/r",
        "https://user:pa?ss@host/r",
        "https://user:pa#ss@host/r",
        " https://token@host/r",
        "ssh:///git:pw@host/r",
        "ftps://u:p@host/r",
        "http::https://token@host/r",
        "http::https:/user:pw@host/r",
        "https:/user:pw@host/r",
    ] {
        assert!(url_has_credentials(url), "{url}");
    }
    // The URL forms of the git-clone documentation, without credentials.
    for url in [
        "ssh://user@host.xz:22/path/to/repo.git/",
        "ssh://user@host.xz/~user/path/to/repo.git/",
        "git://host.xz:9418/path/to/repo.git/",
        "git://host.xz/~user/path/to/repo.git/",
        "https://host.xz:443/path/to/repo.git/",
        "http://host.xz/path/to/repo.git/",
        "ftp://host.xz/path/to/repo.git/",
        "user@host.xz:path/to/repo.git/",
        "host.xz:/path/to/repo.git/",
        "file:///path/to/repo.git/",
        "/path/to/repo.git/",
        "https://host.xz/path/a@b",
    ] {
        assert!(!url_has_credentials(url), "{url}");
    }
}
