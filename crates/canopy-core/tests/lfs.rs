//! Git LFS in the backend: status that agrees with `git status`, the diff of a stored path, and
//! the `git lfs` commands. Tests that need the git-lfs program say so and skip without it; the
//! rest write pointers by hand, as `workflows.rs` does, and need nothing installed.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

mod support;

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use support::isolate_git;

use canopy_core::{DiffSource, LfsOmitted, MissingObjects, Repo, Whitespace};

/// SHA-256 of `hello\n`.
const HELLO_OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .output()
        .expect("git runs")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_output(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Whether the git-lfs program works here.
fn git_lfs_installed() -> bool {
    git_output(Path::new("."), &["lfs", "version"])
        .status
        .success()
}

/// Skips a test that needs git-lfs, saying why.
macro_rules! need_git_lfs {
    () => {
        if !git_lfs_installed() {
            eprintln!("skipped: git-lfs is not installed");
            return;
        }
    };
}

fn init(p: &Path, branch: &str) {
    git(p, &["init", "-q", "-b", branch]);
    for (key, value) in [
        ("user.name", "Ada Lovelace"),
        ("user.email", "ada@example.com"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(p, &[("config"), key, value]);
    }
}

fn pointer(oid: &str, size: u64) -> String {
    format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {size}\n")
}

/// Records `pointer_text` as the blob of `path` in the index, with no stat information, as
/// `git update-index` does for a path it did not read from disk.
fn index_pointer(p: &Path, scratch: &Path, path: &str, pointer_text: &str) {
    let file = scratch.join("pointer.txt");
    fs::write(&file, pointer_text).expect("write");
    let oid = git(p, &["hash-object", "-w", &file.to_string_lossy()]);
    let cacheinfo = format!("100644,{},{path}", oid.trim());
    git(p, &["update-index", "--add", "--cacheinfo", &cacheinfo]);
}

/// A repository where `*.bin` is stored with LFS by attribute only (no filter configured, so
/// git-lfs is not needed), with `big.bin` committed as the pointer of `hello\n` and holding
/// exactly that text.
fn hand_made() -> (tempfile::TempDir, tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    let scratch = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(p, "main");
    fs::write(p.join(".gitattributes"), "*.bin filter=lfs -text\n").expect("write");
    fs::write(p.join("big.bin"), "hello\n").expect("write");
    git(p, &["add", ".gitattributes"]);
    index_pointer(p, scratch.path(), "big.bin", &pointer(HELLO_OID, 6));
    git(p, &["commit", "-q", "-m", "feat: big"]);
    let repo = Repo::open(p).expect("open");
    (dir, scratch, repo)
}

fn later(path: &Path, seconds: u64) {
    let file = fs::File::options().write(true).open(path).expect("open");
    file.set_modified(SystemTime::now() + Duration::from_secs(seconds))
        .expect("touch");
}

fn unstaged(repo: &Repo) -> Vec<String> {
    repo.status(MissingObjects::Fail)
        .expect("status")
        .unstaged
        .into_iter()
        .map(|f| f.path)
        .collect()
}

#[test]
fn an_lfs_file_whose_content_is_its_pointer_is_not_listed_as_modified() {
    isolate_git!(an_lfs_file_whose_content_is_its_pointer_is_not_listed_as_modified);
    let (dir, _scratch, repo) = hand_made();
    let p = dir.path();
    // libgit2 does not run the LFS filter: the file's stat is unknown to the index, so it hashes
    // the file as it is and finds it unlike the pointer blob. Git, with the filter, calls it clean.
    let raw = repo
        .git()
        .statuses(None)
        .expect("statuses")
        .iter()
        .any(|e| {
            e.path().ok() == Some("big.bin") && e.status().contains(git2::Status::WT_MODIFIED)
        });
    assert!(raw, "libgit2 alone reports the file as modified");
    assert_eq!(unstaged(&repo), Vec::<String>::new());

    // Same size, other content: a real change.
    fs::write(p.join("big.bin"), "hellp\n").expect("write");
    later(&p.join("big.bin"), 5);
    assert_eq!(unstaged(&repo), ["big.bin"]);
    // Another size: a real change, without hashing.
    fs::write(p.join("big.bin"), "hello, world\n").expect("write");
    assert_eq!(unstaged(&repo), ["big.bin"]);
    // Back to the pointer's content: clean again, though its time changed.
    fs::write(p.join("big.bin"), "hello\n").expect("write");
    later(&p.join("big.bin"), 10);
    assert_eq!(unstaged(&repo), Vec::<String>::new());
}

#[test]
fn a_mode_change_of_an_lfs_file_stays_listed() {
    isolate_git!(a_mode_change_of_an_lfs_file_stays_listed);
    let (dir, _scratch, repo) = hand_made();
    let p = dir.path();
    git(p, &["update-index", "--chmod=+x", "big.bin"]);
    git(p, &["commit", "-q", "-m", "chore: exec"]);
    // The index says executable; the file on disk is not.
    #[cfg(unix)]
    {
        assert_eq!(unstaged(&repo), ["big.bin"]);
    }
    #[cfg(not(unix))]
    let _ = repo;
}

#[test]
fn an_lfs_file_agrees_with_git_status_with_the_real_filter() {
    isolate_git!(an_lfs_file_agrees_with_git_status_with_the_real_filter);
    need_git_lfs!();
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(p, "main");
    git(p, &["lfs", "install", "--local"]);
    git(p, &["lfs", "track", "*.bin"]);
    fs::write(p.join("data.bin"), vec![7u8; 5000]).expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: data"]);
    let repo = Repo::open(p).expect("open");
    assert_eq!(git(p, &["status", "--porcelain"]), "");
    // A file touched after the commit has a stat the index does not know.
    later(&p.join("data.bin"), 5);
    assert_eq!(git(p, &["status", "--porcelain"]), "");
    assert_eq!(unstaged(&repo), Vec::<String>::new());
    fs::write(p.join("data.bin"), vec![8u8; 5000]).expect("write");
    assert_eq!(git(p, &["status", "--porcelain"]).trim(), "M data.bin");
    assert_eq!(unstaged(&repo), ["data.bin"]);
}

fn store(repo: &Repo, oid: &str, content: &[u8]) {
    let object = repo.lfs_object_path(oid).expect("object path");
    fs::create_dir_all(object.parent().expect("parent")).expect("mkdir");
    fs::write(object, content).expect("write");
}

const NEW_OID: &str = "2222222222222222222222222222222222222222222222222222222222222222";

#[test]
fn the_diff_of_an_lfs_path_is_its_pointers_not_its_file() {
    isolate_git!(the_diff_of_an_lfs_path_is_its_pointers_not_its_file);
    let (dir, scratch, repo) = hand_made();
    let p = dir.path();
    index_pointer(p, scratch.path(), "big.bin", &pointer(NEW_OID, 9_000_000));
    git(p, &["commit", "-q", "-m", "feat: bigger"]);
    let head = repo.head().expect("head").oid.expect("commit");
    let d = repo
        .file_diff(
            &DiffSource::Commit(head),
            "big.bin",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let lfs = d.lfs.expect("an LFS diff");
    let old = lfs.old.expect("old side");
    let new = lfs.new.expect("new side");
    assert_eq!((old.oid.as_deref(), old.size), (Some(HELLO_OID), 6));
    assert_eq!((new.oid.as_deref(), new.size), (Some(NEW_OID), 9_000_000));
    assert!(new.pointer.as_deref().is_some_and(|t| t.contains(NEW_OID)));
    assert!(!old.downloaded && !new.downloaded);
    assert_eq!(lfs.omitted, Some(LfsOmitted::NotDownloaded));
    assert!(d.hunks.is_empty() && !d.binary);
}

#[test]
fn the_content_of_an_lfs_path_is_diffed_only_when_it_is_local() {
    isolate_git!(the_content_of_an_lfs_path_is_diffed_only_when_it_is_local);
    let (dir, _scratch, repo) = hand_made();
    let p = dir.path();
    // The working file is plain text over a pointer whose object is local.
    store(&repo, HELLO_OID, b"one\ntwo\nthree\n");
    fs::write(p.join("big.bin"), "one\n2\nthree\n").expect("write");
    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "big.bin",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let lfs = d.lfs.as_ref().expect("an LFS diff");
    assert_eq!(lfs.omitted, None);
    assert!(lfs.old.as_ref().is_some_and(|s| s.downloaded));
    assert_eq!((d.additions, d.deletions), (1, 1));
    assert_eq!(d.hunks.len(), 1);

    // Binary content is not shown.
    store(&repo, HELLO_OID, b"a\0b");
    fs::write(p.join("big.bin"), "x").expect("write");
    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "big.bin",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    assert_eq!(d.lfs.expect("lfs").omitted, Some(LfsOmitted::Binary));
    assert_eq!(d.hunks, Vec::new());
}

#[test]
fn a_pointer_only_working_file_is_compared_as_a_pointer() {
    isolate_git!(a_pointer_only_working_file_is_compared_as_a_pointer);
    let (dir, _scratch, repo) = hand_made();
    let p = dir.path();
    fs::write(p.join("big.bin"), pointer(NEW_OID, 77)).expect("write");
    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "big.bin",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let lfs = d.lfs.expect("lfs");
    assert_eq!(lfs.new.as_ref().expect("new").oid.as_deref(), Some(NEW_OID));
    assert!(lfs.pointer_only());
}

#[test]
fn a_new_lfs_file_has_only_a_new_side_and_a_deleted_one_only_an_old_side() {
    isolate_git!(a_new_lfs_file_has_only_a_new_side_and_a_deleted_one_only_an_old_side);
    let (dir, _scratch, repo) = hand_made();
    let p = dir.path();
    fs::write(p.join("fresh.bin"), "text\n").expect("write");
    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "fresh.bin",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let lfs = d.lfs.expect("lfs");
    assert!(lfs.old.is_none());
    assert_eq!(lfs.new.expect("new").size, 5);
    assert_eq!(d.additions, 1);

    fs::remove_file(p.join("big.bin")).expect("remove");
    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "big.bin",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let lfs = d.lfs.expect("lfs");
    assert!(lfs.new.is_none() && lfs.old.is_some());
}

#[test]
fn hunks_and_lines_of_an_lfs_path_cannot_be_staged() {
    isolate_git!(hunks_and_lines_of_an_lfs_path_cannot_be_staged);
    let (dir, _scratch, repo) = hand_made();
    let p = dir.path();
    store(&repo, HELLO_OID, b"one\ntwo\n");
    fs::write(p.join("big.bin"), "one\n2\n").expect("write");
    let error = repo
        .apply_hunk(
            canopy_core::HunkAction::Stage,
            "big.bin",
            0,
            (0, 0, 0, 0),
            3,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("Git LFS"), "{error}");
    let error = repo
        .apply_lines(
            canopy_core::HunkAction::Stage,
            "big.bin",
            0,
            (0, 0, 0, 0),
            &[0],
            3,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("Git LFS"), "{error}");
    // A path that is not stored with LFS is not refused for that reason.
    assert!(
        repo.apply_hunk(
            canopy_core::HunkAction::Stage,
            "plain.txt",
            0,
            (0, 0, 0, 0),
            3
        )
        .map_err(|e| e.to_string())
        .err()
        .is_none_or(|e| !e.contains("Git LFS"))
    );
}

/// A bare remote and a clone of it that downloaded no LFS object: its `big.bin` is a pointer.
/// Returns the remote, the clone's folder and the content that was pushed.
fn pointer_only_clone() -> (tempfile::TempDir, tempfile::TempDir, Repo, Vec<u8>) {
    let root = tempfile::tempdir().expect("tempdir");
    let origin = root.path().join("origin.git");
    git(
        root.path(),
        &["init", "-q", "--bare", "-b", "main", "origin.git"],
    );
    let source = root.path().join("source");
    fs::create_dir(&source).expect("mkdir");
    init(&source, "main");
    git(&source, &["lfs", "install", "--local"]);
    git(&source, &["lfs", "track", "*.bin"]);
    let content: Vec<u8> = (0..4000u32)
        .map(|n| u8::try_from(n % 251).expect("byte"))
        .collect();
    fs::write(source.join("big.bin"), &content).expect("write");
    git(&source, &["add", "."]);
    git(&source, &["commit", "-q", "-m", "feat: big"]);
    git(
        &source,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    git(&source, &["push", "-q", "origin", "main"]);

    let clone = tempfile::tempdir().expect("tempdir");
    clone_without_objects(&origin, clone.path());
    let repo = Repo::open(clone.path()).expect("open");
    (root, clone, repo, content)
}

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn clone_without_objects(origin: &Path, dest: &Path) {
    let out = Command::new("git")
        .args([
            "clone",
            "-q",
            &origin.to_string_lossy(),
            &dest.to_string_lossy(),
        ])
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git(dest, &["lfs", "install", "--local"]);
}

#[test]
fn lfs_fetch_downloads_objects_and_lfs_pull_writes_them_into_the_files() {
    isolate_git!(lfs_fetch_downloads_objects_and_lfs_pull_writes_them_into_the_files);
    need_git_lfs!();
    let (_root, clone, repo, content) = pointer_only_clone();
    let p = clone.path();
    let files = repo.lfs_files().expect("lfs files");
    assert_eq!((files.len(), files[0].downloaded), (1, false));
    assert!(
        fs::read_to_string(p.join("big.bin"))
            .expect("read")
            .starts_with("version ")
    );

    let mut lines = Vec::new();
    repo.lfs_fetch(|l| lines.push(l.to_owned())).expect("fetch");
    assert!(repo.lfs_files().expect("lfs files")[0].downloaded);
    assert!(
        fs::read_to_string(p.join("big.bin"))
            .expect("read")
            .starts_with("version "),
        "a fetch leaves the working tree alone"
    );
    assert!(!lines.is_empty(), "progress lines arrive");

    repo.lfs_pull(|_| {}).expect("pull");
    assert_eq!(fs::read(p.join("big.bin")).expect("read"), content);
    assert_eq!(git(p, &["status", "--porcelain"]), "");
    assert_eq!(unstaged(&repo), Vec::<String>::new());
}

#[test]
fn a_failed_lfs_pull_says_what_git_lfs_said() {
    isolate_git!(a_failed_lfs_pull_says_what_git_lfs_said);
    need_git_lfs!();
    let (_root, clone, repo, _content) = pointer_only_clone();
    git(
        clone.path(),
        &["remote", "set-url", "origin", "/nonexistent/place.git"],
    );
    let error = repo.lfs_pull(|_| {}).unwrap_err().to_string();
    assert!(
        !error.trim().is_empty() && !error.contains("operation failed"),
        "{error}"
    );
}

#[test]
fn track_and_untrack_edit_gitattributes_like_the_git_lfs_cli() {
    isolate_git!(track_and_untrack_edit_gitattributes_like_the_git_lfs_cli);
    need_git_lfs!();
    let ours = tempfile::tempdir().expect("tempdir");
    let theirs = tempfile::tempdir().expect("tempdir");
    for p in [ours.path(), theirs.path()] {
        init(p, "main");
        git(p, &["lfs", "install", "--local"]);
    }
    let repo = Repo::open(ours.path()).expect("open");
    repo.lfs_track("*.psd", false).expect("track");
    repo.lfs_track("a dir/odd*.bin", true)
        .expect("track a file");
    git(theirs.path(), &["lfs", "track", "*.psd"]);
    git(
        theirs.path(),
        &["lfs", "track", "--filename", "--", "a dir/odd*.bin"],
    );
    let read = |p: &Path| fs::read_to_string(p.join(".gitattributes")).expect("read");
    assert_eq!(read(ours.path()), read(theirs.path()));
    assert!(read(ours.path()).contains("*.psd filter=lfs"));

    let patterns = repo.lfs_patterns().expect("patterns");
    assert_eq!(patterns[0].pattern, "*.psd");
    assert_eq!(patterns[0].source, ".gitattributes");
    assert_eq!(patterns.len(), 2);

    repo.lfs_untrack("*.psd").expect("untrack");
    git(theirs.path(), &["lfs", "untrack", "*.psd"]);
    assert_eq!(read(ours.path()), read(theirs.path()));
    assert!(!read(ours.path()).contains("*.psd"));
    // The change is an ordinary working-tree edit.
    assert_eq!(unstaged(&repo), [".gitattributes"]);
}

#[test]
fn track_refuses_what_is_not_a_pattern() {
    isolate_git!(track_refuses_what_is_not_a_pattern);
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path(), "main");
    let repo = Repo::open(dir.path()).expect("open");
    for bad in ["", "  ", "--all", "a\nb"] {
        assert!(repo.lfs_track(bad, false).is_err(), "{bad:?}");
        assert!(repo.lfs_untrack(bad).is_err(), "{bad:?}");
    }
    repo.lfs_lock("../outside").unwrap_err();
    repo.lfs_unlock(".git/config", true).unwrap_err();
}

/// A just-enough LFS locking server on localhost: lists, creates and releases locks, keeping
/// them in memory. Returns its URL; the thread ends with the test process.
fn lock_server() -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let locks: Arc<Mutex<Vec<(u32, String, String)>>> =
        Arc::new(Mutex::new(vec![(1, "other.bin".into(), "grace".into())]));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut request = String::new();
            reader.read_line(&mut request).expect("request line");
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("header");
                if header.trim().is_empty() {
                    break;
                }
                if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().expect("length");
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).expect("body");
            let body = String::from_utf8_lossy(&body).into_owned();
            let mut parts = request.split_whitespace();
            let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let json = |l: &(u32, String, String)| {
                format!(
                    r#"{{"id":"{}","path":"{}","locked_at":"2026-01-01T00:00:00Z","owner":{{"name":"{}"}}}}"#,
                    l.0, l.1, l.2
                )
            };
            let mut held = locks.lock().expect("locks");
            let (status, reply) = match (method, path) {
                ("GET", p) if p.starts_with("/locks") => {
                    // `?path=` and `?id=` narrow the list, as git-lfs asks before it unlocks.
                    let wanted = |key: &str| {
                        p.split(&format!("{key}="))
                            .nth(1)
                            .map(|v| v.split('&').next().unwrap_or("").replace("%2F", "/"))
                    };
                    let all: Vec<String> = held
                        .iter()
                        .filter(|l| wanted("path").is_none_or(|w| w == l.1))
                        .filter(|l| wanted("id").is_none_or(|w| w == l.0.to_string()))
                        .map(json)
                        .collect();
                    ("200 OK", format!(r#"{{"locks":[{}]}}"#, all.join(",")))
                }
                ("POST", "/locks") => {
                    let path = body
                        .split(r#""path":""#)
                        .nth(1)
                        .and_then(|rest| rest.split('"').next())
                        .unwrap_or("")
                        .to_owned();
                    let lock = (
                        u32::try_from(held.len()).expect("small") + 2,
                        path,
                        "ada".into(),
                    );
                    let reply = format!(r#"{{"lock":{}}}"#, json(&lock));
                    held.push(lock);
                    ("201 Created", reply)
                }
                ("POST", p) if p.ends_with("/unlock") => {
                    let id: u32 = p
                        .split('/')
                        .nth(2)
                        .and_then(|i| i.parse().ok())
                        .unwrap_or(0);
                    match held.iter().position(|l| l.0 == id) {
                        Some(at) => {
                            let lock = held.remove(at);
                            ("200 OK", format!(r#"{{"lock":{}}}"#, json(&lock)))
                        }
                        None => ("404 Not Found", r#"{"message":"no such lock"}"#.into()),
                    }
                }
                _ => ("404 Not Found", r#"{"message":"unknown"}"#.into()),
            };
            drop(held);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/vnd.git-lfs+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    url
}

#[test]
fn locks_are_listed_taken_and_released_through_git_lfs() {
    isolate_git!(locks_are_listed_taken_and_released_through_git_lfs);
    need_git_lfs!();
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(p, "main");
    git(p, &["lfs", "install", "--local"]);
    git(p, &["lfs", "track", "*.bin"]);
    fs::write(p.join("mine.bin"), "x").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: mine"]);
    git(p, &["config", "lfs.url", &lock_server()]);
    let repo = Repo::open(p).expect("open");

    let locks = repo.lfs_locks().expect("locks");
    assert_eq!(
        locks
            .iter()
            .map(|l| (l.path.as_str(), l.owner.as_str()))
            .collect::<Vec<_>>(),
        [("other.bin", "grace")]
    );
    repo.lfs_lock("mine.bin").expect("lock");
    let locks = repo.lfs_locks().expect("locks");
    assert!(
        locks
            .iter()
            .any(|l| l.path == "mine.bin" && l.owner == "ada"),
        "{locks:?}"
    );
    repo.lfs_unlock("mine.bin", false).expect("unlock");
    assert!(
        !repo
            .lfs_locks()
            .expect("locks")
            .iter()
            .any(|l| l.path == "mine.bin")
    );
    // Someone else's lock is released only by force.
    repo.lfs_unlock("other.bin", true).expect("force unlock");
    assert_eq!(repo.lfs_locks().expect("locks"), Vec::new());
}

#[test]
fn a_lock_the_server_refuses_says_what_it_said() {
    isolate_git!(a_lock_the_server_refuses_says_what_it_said);
    need_git_lfs!();
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path(), "main");
    git(dir.path(), &["config", "lfs.url", "http://127.0.0.1:1"]);
    let repo = Repo::open(dir.path()).expect("open");
    let error = repo.lfs_locks().unwrap_err().to_string();
    assert!(!error.trim().is_empty(), "{error}");
}
