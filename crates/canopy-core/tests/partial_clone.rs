//! Partial clones (`git clone --filter=blob:none`, `--filter=tree:0`, `--no-checkout`): reads
//! that need objects the clone left on the server either download them through the `git`
//! executable or fail with `Error::NotDownloaded`, and match what `git` shows for the same
//! commits in the full source repository.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use canopy_core::ops::HunkAction;
use canopy_core::{
    ChangeKind, DiffSource, Error, FileDiff, FileEntry, LineKind, MissingObjects, Oid, Repo,
    Whitespace,
};

use MissingObjects::{Download, Fail};

/// The directory inside a test's temporary root that serves as `HOME`, found from any path under
/// that root. [`Fixture::new`] creates it, with an empty `gitconfig` in it.
fn isolated_home(dir: &Path) -> PathBuf {
    dir.ancestors()
        .map(|a| a.join("home"))
        .find(|h| h.join("gitconfig").is_file())
        .expect("the path is inside a Fixture's temporary directory")
}

/// A `git` command for `dir` that reads none of the developer's own configuration: a fixed
/// identity, no system configuration, and an empty global configuration file and home directory
/// inside the test's temporary root (so `~/.gitconfig` aliases, `url.insteadOf`, signing or
/// filters never leak in).
///
/// This covers the `git` commands the tests run themselves. The ones Canopy runs through
/// `canopy_core::cli` (the downloads) inherit this process's environment, because `cli` has no
/// per-call environment override and setting variables on the process would race with the tests
/// running in parallel. They only need the repository's own configuration here (the clones use
/// `file://` URLs and carry `core.autocrlf=false`); a global setting that changes how
/// `git fetch` behaves can still reach them.
#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git_command(dir: &Path) -> Command {
    let home = isolated_home(dir);
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &home);
    cmd
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_command(dir).args(args).output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A full source repository served over `file://`, with filters allowed, and four commits:
/// `one` adds files, `two` edits `a.txt`, `three` renames and edits `d/x.txt` and adds `b.txt`
/// (tagged `v1`), `four` edits `a.txt`, `sp ace.txt` and adds a binary file.
struct Fixture {
    root: tempfile::TempDir,
    src: PathBuf,
    url: String,
    /// Commit IDs, oldest first.
    commits: Vec<Oid>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join("home")).expect("mkdir home");
        fs::write(root.path().join("home").join("gitconfig"), "").expect("empty global config");
        let src = root.path().join("src");
        fs::create_dir_all(src.join("d")).expect("mkdir");
        let p = src.as_path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.name", "Ada Lovelace"]);
        git(p, &["config", "user.email", "ada@example.com"]);
        git(p, &["config", "commit.gpgsign", "false"]);
        git(p, &["config", "tag.gpgsign", "false"]);
        git(p, &["config", "core.autocrlf", "false"]);
        git(p, &["config", "uploadpack.allowFilter", "true"]);
        git(p, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
        let write = |name: &str, data: &[u8]| fs::write(p.join(name), data).expect("write");
        write("a.txt", b"1\n2\n3\n");
        write("d/x.txt", b"alpha\nbeta\ngamma\ndelta\n");
        write("sp ace.txt", b"spaced\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "one"]);
        write("a.txt", b"1\n2b\n3\n");
        git(p, &["commit", "-q", "-am", "two"]);
        git(p, &["mv", "d/x.txt", "d/y.txt"]);
        write("d/y.txt", b"alpha\nbeta\ngamma\ndelta2\n");
        write("b.txt", b"new\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "three"]);
        git(p, &["tag", "-a", "v1", "-m", "v1"]);
        write("a.txt", b"1\n2c\n3\n");
        write("sp ace.txt", b"spaced\nmore\n");
        write("bin.dat", &[0, 1, 2, 0, 255, 254]);
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "four"]);
        let commits = git(p, &["rev-list", "--reverse", "HEAD"])
            .lines()
            .map(|l| Oid::from_str(l).expect("oid"))
            .collect();
        let s = src.to_string_lossy().replace('\\', "/");
        let url = if s.starts_with('/') {
            format!("file://{s}")
        } else {
            format!("file:///{s}")
        };
        Self {
            root,
            src,
            url,
            commits,
        }
    }

    /// Clones the source into `name` with extra `git clone` arguments.
    fn clone(&self, name: &str, extra: &[&str]) -> PathBuf {
        let mut args = vec!["clone", "-q", "-c", "core.autocrlf=false"];
        args.extend_from_slice(extra);
        args.push(&self.url);
        args.push(name);
        git(self.root.path(), &args);
        self.root.path().join(name)
    }
}

/// Objects reachable from any ref that the clone does not have. `--missing=print` never fetches.
fn missing(dir: &Path) -> Vec<String> {
    git(dir, &["rev-list", "--objects", "--all", "--missing=print"])
        .lines()
        .filter_map(|l| l.strip_prefix('?').map(str::to_owned))
        .collect()
}

/// `(path, additions, deletions)` per file, from `git diff-tree --numstat -M` in the full source.
fn cli_numstat(src: &Path, oid: Oid) -> Vec<(String, usize, usize)> {
    let out = git(
        src,
        &[
            "diff-tree",
            "-r",
            "-M",
            "--root",
            "--no-commit-id",
            "--numstat",
            "-z",
            &oid.to_string(),
        ],
    );
    let mut tokens = out.split('\0').filter(|t| !t.is_empty());
    let mut rows = Vec::new();
    while let Some(t) = tokens.next() {
        let mut parts = t.splitn(3, '\t');
        let num = |s: Option<&str>| s.and_then(|s| s.parse().ok()).unwrap_or(0);
        let (add, del) = (num(parts.next()), num(parts.next()));
        let mut path = parts.next().unwrap_or_default().to_owned();
        if path.is_empty() {
            // Rename: old and new path follow as separate fields.
            tokens.next();
            tokens.next().expect("new path").clone_into(&mut path);
        }
        rows.push((path, add, del));
    }
    rows.sort();
    rows
}

/// The `+`, `-` and context lines of a `git diff`/`git show` patch, prefix included.
fn cli_lines(patch: &str) -> Vec<String> {
    patch
        .lines()
        .skip_while(|l| !l.starts_with("@@"))
        .filter(|l| l.starts_with([' ', '+', '-']))
        .map(str::to_owned)
        .collect()
}

fn our_lines(d: &FileDiff) -> Vec<String> {
    d.hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| {
            let prefix = match l.kind {
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::Context => ' ',
            };
            format!("{prefix}{}", l.text)
        })
        .collect()
}

/// Checks every commit's details and every file diff against the CLI in the full source.
fn assert_history_matches_cli(fx: &Fixture, repo: &Repo, missing: MissingObjects) {
    for &oid in &fx.commits {
        let details = repo.commit_details(oid, missing).expect("details");
        let mut ours: Vec<(String, usize, usize)> = details
            .files
            .iter()
            .map(|f| (f.path.clone(), f.additions, f.deletions))
            .collect();
        ours.sort();
        assert_eq!(ours, cli_numstat(&fx.src, oid), "numstat of {oid}");

        for f in &details.files {
            let d = repo
                .file_diff(
                    &DiffSource::Commit(oid),
                    &f.path,
                    f.old_path.as_deref(),
                    3,
                    Whitespace::Show,
                    missing,
                )
                .expect("file diff");
            let mut args = vec![
                "show".to_owned(),
                "--format=".to_owned(),
                "-M".to_owned(),
                "-U3".to_owned(),
                oid.to_string(),
                "--".to_owned(),
                f.path.clone(),
            ];
            args.extend(f.old_path.clone());
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let patch = git(&fx.src, &args);
            assert_eq!(our_lines(&d), cli_lines(&patch), "{} in {oid}", f.path);
            assert_eq!(d.binary, patch.contains("Binary files"), "{}", f.path);
        }
    }
}

fn assert_not_downloaded(err: Error, remote: Option<&str>, attempted: bool) {
    match &err {
        Error::NotDownloaded { remote: r, reason } => {
            assert_eq!(r.as_deref(), remote, "{err}");
            assert_eq!(reason.is_some(), attempted, "{err}");
            assert!(err.to_string().contains("partial clone"), "{err}");
            assert_eq!(err.can_download(), remote.is_some());
        }
        other => panic!("expected NotDownloaded, got {other:?}"),
    }
}

#[test]
fn full_clone_is_not_partial_and_reads_offline() {
    let fx = Fixture::new();
    let dir = fx.clone("full", &[]);
    let repo = Repo::open(&dir).expect("open");
    assert!(!repo.is_partial_clone());
    let absent = missing(&dir);
    assert!(absent.is_empty(), "{absent:?}");
    assert_history_matches_cli(&fx, &repo, Fail);
}

#[test]
#[expect(clippy::string_slice, reason = "object IDs are ASCII hex")]
fn missing_object_in_full_repository_stays_a_libgit2_error() {
    // Corruption in a repository that is not a partial clone must not be reported as
    // "not downloaded", and must not trigger a download.
    let fx = Fixture::new();
    let dir = fx.clone("corrupt", &["--no-local"]);
    // Unpack so the old blob of a.txt is a loose object that can be deleted.
    let packs = dir.join(".git/objects/pack");
    for entry in fs::read_dir(&packs).expect("packs") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "pack") {
            let data = fs::read(&path).expect("read pack");
            fs::rename(&path, dir.join("old.pack")).expect("move pack");
            fs::remove_file(path.with_extension("idx")).expect("remove idx");
            let _ = fs::remove_file(path.with_extension("rev"));
            let mut child = git_command(&dir)
                .args(["unpack-objects", "-q"])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .expect("unpack");
            std::io::Write::write_all(child.stdin.as_mut().expect("stdin"), &data)
                .expect("feed pack");
            assert!(child.wait().expect("wait").success());
        }
    }
    let blob = git(&dir, &["rev-parse", &format!("{}:a.txt", fx.commits[0])]);
    let blob = blob.trim();
    fs::remove_file(dir.join(".git/objects").join(&blob[..2]).join(&blob[2..]))
        .expect("remove blob");

    let repo = Repo::open(&dir).expect("open");
    assert!(!repo.is_partial_clone());
    let err = repo
        .commit_details(fx.commits[1], Download)
        .expect_err("blob is gone");
    assert!(matches!(err, Error::Git(_)), "{err:?}");
}

#[test]
fn blobless_clone_fail_mode_stays_offline() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let before = missing(&dir);
    assert!(!before.is_empty(), "the clone should lack old blobs");

    let repo = Repo::open(&dir).expect("open");
    assert!(repo.is_partial_clone());
    let snap = repo.snapshot(100).expect("snapshot works offline");
    assert_eq!(snap.commits.len(), 4);
    assert_eq!(snap.tags.len(), 1);

    // The file list needs only trees, which a blobless clone has: it is returned offline, with
    // the line counts marked unknown.
    let details = repo
        .commit_details(fx.commits[1], Fail)
        .expect("the file list needs no blobs");
    assert!(!details.stats_known);
    assert_eq!(
        details.files,
        [FileEntry {
            path: "a.txt".into(),
            old_path: None,
            kind: ChangeKind::Modified,
            additions: 0,
            deletions: 0,
            lfs: false,
            old_mode: Some(0o100_644),
            new_mode: Some(0o100_644),
        }]
    );
    let err = repo
        .file_diff(
            &DiffSource::Commit(fx.commits[1]),
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Fail,
        )
        .expect_err("old blobs are missing");
    assert_not_downloaded(err, Some("origin"), false);
    assert_eq!(missing(&dir), before, "Fail must not download anything");
}

#[test]
fn details_list_files_offline_then_complete_with_a_download() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let before = missing(&dir);
    let repo = Repo::open(&dir).expect("open");

    // Commit three renames d/x.txt (edited) and adds b.txt: offline, names only, and the
    // inexact rename shows as a deletion and an addition.
    let offline = repo.commit_details(fx.commits[2], Fail).expect("offline");
    assert!(!offline.stats_known);
    let mut listed: Vec<(&str, ChangeKind)> = offline
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.kind))
        .collect();
    listed.sort_by_key(|(path, _)| *path);
    assert_eq!(
        listed,
        [
            ("b.txt", ChangeKind::Added),
            ("d/x.txt", ChangeKind::Deleted),
            ("d/y.txt", ChangeKind::Added),
        ]
    );
    assert!(offline.files.iter().all(|f| f.additions + f.deletions == 0));
    assert_eq!(missing(&dir), before, "the offline list downloads nothing");

    // Downloading completes the same commit with rename detection and counts, as in the CLI.
    let full = repo
        .commit_details(fx.commits[2], Download)
        .expect("download");
    assert!(full.stats_known);
    let mut ours: Vec<(String, usize, usize)> = full
        .files
        .iter()
        .map(|f| (f.path.clone(), f.additions, f.deletions))
        .collect();
    ours.sort();
    assert_eq!(ours, cli_numstat(&fx.src, fx.commits[2]));
    assert!(full.files.iter().any(|f| f.kind == ChangeKind::Renamed));
    // Once the blobs are local the offline read is complete too.
    assert!(
        repo.commit_details(fx.commits[2], Fail)
            .expect("local")
            .stats_known
    );
}

#[test]
fn exact_renames_show_in_the_offline_file_list() {
    let mut fx = Fixture::new();
    git(&fx.src, &["mv", "b.txt", "moved.txt"]);
    git(&fx.src, &["commit", "-q", "-m", "move b"]);
    let moved = fx.commits.len();
    git(&fx.src, &["rm", "-q", "moved.txt"]);
    git(&fx.src, &["commit", "-q", "-m", "remove it"]);
    fx.commits = git(&fx.src, &["rev-list", "--reverse", "HEAD"])
        .lines()
        .map(|l| Oid::from_str(l).expect("oid"))
        .collect();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let repo = Repo::open(&dir).expect("open");

    let offline = repo
        .commit_details(fx.commits[moved], Fail)
        .expect("offline");
    assert!(!offline.stats_known);
    assert_eq!(offline.files.len(), 1, "{:?}", offline.files);
    assert_eq!(offline.files[0].kind, ChangeKind::Renamed);
    assert_eq!(offline.files[0].path, "moved.txt");
    assert_eq!(offline.files[0].old_path.as_deref(), Some("b.txt"));
}

#[test]
fn huge_commits_skip_the_blob_download_and_leave_counts_unknown() {
    let mut fx = Fixture::new();
    let bulk = fx.src.join("bulk");
    fs::create_dir_all(&bulk).expect("mkdir");
    let files = canopy_core::repo::MAX_STAT_FILES + 1;
    for i in 0..files {
        fs::write(bulk.join(format!("f{i:03}.txt")), format!("file {i}\n")).expect("write");
    }
    git(&fx.src, &["add", "-A"]);
    git(&fx.src, &["commit", "-q", "-m", "bulk"]);
    let bulk_commit = Oid::from_str(git(&fx.src, &["rev-parse", "HEAD"]).trim()).expect("oid");
    // Remove them again so the clone's checkout does not download their blobs.
    git(&fx.src, &["rm", "-r", "-q", "bulk"]);
    git(&fx.src, &["commit", "-q", "-m", "unbulk"]);
    fx.commits.push(bulk_commit);
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let before = missing(&dir);
    assert!(
        before.len() >= files,
        "precondition: the bulk blobs are missing"
    );
    let repo = Repo::open(&dir).expect("open");

    let details = repo.commit_details(bulk_commit, Download).expect("details");
    assert!(!details.stats_known);
    assert_eq!(details.files.len(), files);
    assert!(details.files.iter().all(|f| f.kind == ChangeKind::Added));
    assert_eq!(
        missing(&dir),
        before,
        "no blob was downloaded for the counts"
    );
    // A single file's diff still downloads just its own blob.
    let d = repo
        .file_diff(
            &DiffSource::Commit(bulk_commit),
            "bulk/f007.txt",
            None,
            3,
            Whitespace::Show,
            Download,
        )
        .expect("diff");
    assert_eq!(our_lines(&d), ["+file 7"]);
}

/// The rule an application installs: no download for a repository whose path says so.
fn deny_marked_repositories(common_dir: &Path, _workdir: Option<&Path>) -> bool {
    !common_dir.to_string_lossy().contains("denied-by-gate")
}

#[test]
fn the_download_gate_stops_the_fetch_itself() {
    canopy_core::partial::set_download_gate(deny_marked_repositories);
    let fx = Fixture::new();
    let denied = fx.clone("denied-by-gate", &["--filter=blob:none"]);
    let repo = Repo::open(&denied).expect("open");
    let before = missing(&denied);
    assert!(!before.is_empty(), "the partial clone has every blob");
    let err = repo
        .commit_details(fx.commits[0], Download)
        .expect_err("the gate refuses the fetch");
    assert!(
        matches!(&err, Error::NotDownloaded { remote: None, reason: Some(r) } if r.contains("trusted")),
        "{err:?}"
    );
    assert_eq!(missing(&denied), before, "nothing was fetched");

    // Another repository in the same process still downloads.
    let allowed = fx.clone("allowed", &["--filter=blob:none"]);
    Repo::open(&allowed)
        .expect("open")
        .commit_details(fx.commits[0], Download)
        .expect("downloads");
}

#[test]
fn blobless_clone_downloads_blobs_and_matches_cli() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let refs = || git(&dir, &["for-each-ref", "--format=%(refname) %(objectname)"]);
    let refs_before = refs();
    let repo = Repo::open(&dir).expect("open");
    assert_history_matches_cli(&fx, &repo, Download);
    // Everything needed is now local: the same reads succeed offline.
    assert_history_matches_cli(&fx, &repo, Fail);
    assert!(missing(&dir).is_empty(), "{:?}", missing(&dir));
    // Only objects were fetched; no ref or FETCH_HEAD changed.
    assert!(!dir.join(".git/FETCH_HEAD").exists());
    assert_eq!(refs(), refs_before);
}

#[test]
fn treeless_clone_downloads_trees_then_blobs_and_matches_cli() {
    let fx = Fixture::new();
    let dir = fx.clone("treeless", &["--filter=tree:0"]);
    let repo = Repo::open(&dir).expect("open");
    assert!(repo.is_partial_clone());
    let snap = repo.snapshot(100).expect("history needs only commits");
    assert_eq!(snap.commits.len(), 4);
    assert_eq!(snap.graph.len(), 4);

    let err = repo
        .commit_details(fx.commits[0], Fail)
        .expect_err("old trees are missing");
    assert_not_downloaded(err, Some("origin"), false);

    assert_history_matches_cli(&fx, &repo, Download);
    assert_history_matches_cli(&fx, &repo, Fail);
}

#[test]
fn no_checkout_clone_staged_diff_downloads_head_blob() {
    let fx = Fixture::new();
    let dir = fx.clone("nocheckout", &["--filter=blob:none", "--no-checkout"]);
    let repo = Repo::open(&dir).expect("open");
    // Without an index every file of HEAD shows as a staged deletion, as in `git status`.
    let snap = repo.snapshot(100).expect("snapshot");
    let staged: Vec<&str> = snap.status.staged.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        staged,
        ["a.txt", "b.txt", "bin.dat", "d/y.txt", "sp ace.txt"]
    );

    let err = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Fail,
        )
        .expect_err("HEAD blob is missing");
    assert_not_downloaded(err, Some("origin"), false);
    let d = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Download,
        )
        .expect("download");
    assert_eq!(d.kind, Some(ChangeKind::Deleted));
    assert_eq!(our_lines(&d), ["-1", "-2c", "-3"]);
}

#[test]
fn status_needing_missing_blobs_for_renames_stays_offline() {
    let fx = Fixture::new();
    let dir = fx.clone("nocheckout", &["--filter=blob:none", "--no-checkout"]);
    // A staged addition next to staged deletions makes libgit2 look for renames, which needs
    // the deleted files' blobs.
    fs::write(dir.join("c.txt"), "1\n2c\n3\n").expect("write");
    git(&dir, &["add", "c.txt"]);
    let before = missing(&dir);
    let repo = Repo::open(&dir).expect("open");
    let snap = repo.snapshot(100).expect("snapshot works offline");
    let staged: Vec<(&str, ChangeKind)> = snap
        .status
        .staged
        .iter()
        .map(|f| (f.path.as_str(), f.kind))
        .collect();
    assert_eq!(
        staged,
        [
            ("a.txt", ChangeKind::Deleted),
            ("b.txt", ChangeKind::Deleted),
            ("bin.dat", ChangeKind::Deleted),
            ("c.txt", ChangeKind::Added),
            ("d/y.txt", ChangeKind::Deleted),
            ("sp ace.txt", ChangeKind::Deleted),
        ]
    );
    assert_eq!(missing(&dir), before, "status must not download");
}

#[test]
fn soft_reset_to_old_commit_keeps_snapshot_and_downloads_staged_diff() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let one = fx.commits[0].to_string();
    let four = fx.commits[3].to_string();
    git(&dir, &["reset", "-q", "--soft", &one]);
    let before = missing(&dir);
    let repo = Repo::open(&dir).expect("open");

    let snap = repo.snapshot(100).expect("snapshot works offline");
    let staged = name_status(&snap.status.staged);
    // Offline, renames cannot be detected: compare with git's --no-renames view.
    let mut expected: Vec<String> = git(
        &fx.src,
        &["diff", "--name-status", "--no-renames", &one, &four],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    expected.sort();
    assert_eq!(staged, expected);
    assert_eq!(missing(&dir), before, "snapshot must not download");

    let d = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Download,
        )
        .expect("download");
    let patch = git(&fx.src, &["diff", "-U3", &one, &four, "--", "a.txt"]);
    assert_eq!(our_lines(&d), cli_lines(&patch));
}

#[test]
fn unstaged_diff_against_missing_index_blob_downloads_it() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let one = fx.commits[0].to_string();
    let four = fx.commits[3].to_string();
    // Point the index at the first commit; the working tree keeps the newest content.
    git(&dir, &["read-tree", &one]);
    let index_blob = git(&dir, &["ls-files", "-s", "--", "a.txt"]);
    let index_blob = index_blob.split_whitespace().nth(1).expect("blob id");
    assert!(
        missing(&dir).iter().any(|m| m == index_blob),
        "precondition: the index blob is not downloaded"
    );
    let repo = Repo::open(&dir).expect("open");
    let snap = repo.snapshot(100).expect("snapshot");
    assert!(snap.status.unstaged.iter().any(|f| f.path == "a.txt"));

    let err = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Fail,
        )
        .expect_err("index blob is missing");
    assert_not_downloaded(err, Some("origin"), false);
    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Download,
        )
        .expect("download");
    let patch = git(&fx.src, &["diff", "-U3", &one, &four, "--", "a.txt"]);
    assert_eq!(our_lines(&d), cli_lines(&patch));
}

#[test]
fn hunk_staging_stays_offline_and_works_once_the_diff_was_downloaded() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let one = fx.commits[0].to_string();
    git(&dir, &["read-tree", &one]);
    let repo = Repo::open(&dir).expect("open");

    let err = repo
        .apply_hunk(HunkAction::Stage, "a.txt", 0, (1, 3, 1, 3), 3)
        .expect_err("index blob is missing");
    assert_not_downloaded(err, Some("origin"), false);

    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Download,
        )
        .expect("download");
    assert_eq!(d.hunks.len(), 1);
    repo.apply_hunk(HunkAction::Stage, "a.txt", 0, d.hunks[0].key(), 3)
        .expect("stage hunk");
    let staged = git(&dir, &["ls-files", "-s", "--", "a.txt"]);
    let newest = git(&fx.src, &["rev-parse", &format!("{}:a.txt", fx.commits[3])]);
    assert_eq!(staged.split_whitespace().nth(1), Some(newest.trim()));
}

#[test]
fn merge_conflicts_read_offline_in_a_blobless_clone() {
    // Conflict reading uses the index stages and the working copy, which `git merge` (lazily
    // fetching what it needs) has written; nothing further is downloaded.
    let fx = Fixture::new();
    git(&fx.src, &["switch", "-q", "-c", "side"]);
    fs::write(fx.src.join("a.txt"), "1\nside\n3\n").expect("write");
    git(&fx.src, &["commit", "-q", "-am", "side"]);
    git(&fx.src, &["switch", "-q", "main"]);
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    for (k, v) in [
        ("user.name", "Ada Lovelace"),
        ("user.email", "ada@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        git(&dir, &["config", k, v]);
    }
    fs::write(dir.join("a.txt"), "1\nmine\n3\n").expect("write");
    git(&dir, &["commit", "-q", "-am", "mine"]);
    let merge = git_command(&dir)
        .args(["merge", "-q", "--no-edit", "origin/side"])
        .output()
        .expect("git merge runs");
    assert!(!merge.status.success(), "the merge should conflict");
    let before = missing(&dir);

    let repo = Repo::open(&dir).expect("open");
    let snap = repo.snapshot(100).expect("snapshot");
    let conflicted: Vec<&str> = snap
        .status
        .conflicted
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(conflicted, ["a.txt"]);
    assert_eq!(
        repo.conflict_kind("a.txt").expect("kind"),
        Some(canopy_core::ConflictKind::BothModified)
    );
    let file = repo.conflict_file("a.txt").expect("parse");
    assert_eq!(file.conflicts(), 1);
    assert_eq!(missing(&dir), before, "reading conflicts must not download");

    repo.resolve_with_side("a.txt", canopy_core::Side::Ours)
        .expect("take ours");
    assert_eq!(
        fs::read_to_string(dir.join("a.txt")).expect("read"),
        "1\nmine\n3\n"
    );
}

#[test]
fn unreachable_remote_reports_git_error_and_recovers() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let moved = fx.root.path().join("src-moved");
    fs::rename(&fx.src, &moved).expect("hide the remote");

    let repo = Repo::open(&dir).expect("open");
    let err = repo
        .commit_details(fx.commits[1], Download)
        .expect_err("remote is gone");
    let Error::NotDownloaded {
        reason: Some(reason),
        ..
    } = &err
    else {
        panic!("expected NotDownloaded with a reason, got {err:?}");
    };
    assert!(!reason.is_empty(), "the error gives no reason");
    assert_not_downloaded(err, Some("origin"), true);
    assert!(fs::read_dir(dir.join(".git")).is_ok(), "repository intact");

    fs::rename(&moved, &fx.src).expect("restore the remote");
    let details = repo
        .commit_details(fx.commits[1], Download)
        .expect("works once the remote is back");
    assert_eq!(details.files.len(), 1);
}

#[test]
fn removed_promisor_remote_cannot_download() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    git(&dir, &["remote", "remove", "origin"]);
    let repo = Repo::open(&dir).expect("open");
    // The promisor packs still mark it as a partial clone.
    assert!(repo.is_partial_clone());
    repo.snapshot(100).expect("snapshot");

    assert!(
        !repo
            .commit_details(fx.commits[1], Fail)
            .expect("the file list needs no blobs")
            .stats_known
    );
    let err = repo
        .file_diff(
            &DiffSource::Commit(fx.commits[1]),
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Fail,
        )
        .expect_err("no remote");
    assert_not_downloaded(err, None, false);
    let err = repo
        .commit_details(fx.commits[1], Download)
        .expect_err("no remote");
    assert_not_downloaded(err, None, true);
}

#[test]
fn legacy_partial_clone_extension_is_reported_and_conversion_works() {
    let fx = Fixture::new();
    let dir = fx.clone("legacy", &["--filter=blob:none"]);
    // What Git before 2.24 wrote instead of remote.origin.promisor.
    git(&dir, &["config", "--unset", "remote.origin.promisor"]);
    git(&dir, &["config", "extensions.partialClone", "origin"]);
    match Repo::open(&dir) {
        Err(Error::LegacyPartialClone { remote }) => assert_eq!(remote, "origin"),
        other => panic!("expected LegacyPartialClone, got {other:?}"),
    }

    // The conversion the error message suggests.
    git(&dir, &["config", "remote.origin.promisor", "true"]);
    git(&dir, &["config", "--unset", "extensions.partialClone"]);
    let repo = Repo::open(&dir).expect("opens after conversion");
    assert!(repo.is_partial_clone());
    assert_history_matches_cli(&fx, &repo, Download);
}

#[test]
fn snapshot_after_a_soft_reset_in_a_treeless_clone_recovers_its_status() {
    let fx = Fixture::new();
    let dir = fx.clone("treeless", &["--filter=tree:0"]);
    let one = fx.commits[0].to_string();
    let four = fx.commits[3].to_string();
    git(&dir, &["reset", "-q", "--soft", &one]);
    let before = missing(&dir);
    let repo = Repo::open(&dir).expect("open");
    let mut expected: Vec<String> = git(
        &fx.src,
        &["diff", "--name-status", "--no-renames", &one, &four],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    expected.sort();

    let mut snap = repo.snapshot(100).expect("history needs no trees");
    assert_eq!(snap.commits.len(), 4);
    assert!(snap.status_unavailable.is_some());

    // Staying offline changes nothing.
    repo.complete_status(&mut snap, Fail);
    assert!(snap.status_unavailable.is_some());
    assert_eq!(missing(&dir), before);

    // The remote is unreachable: the snapshot keeps its history and says why the changes are
    // missing, in git's words.
    let moved = fx.root.path().join("src-moved");
    fs::rename(&fx.src, &moved).expect("hide the remote");
    repo.complete_status(&mut snap, Download);
    let unavailable = snap.status_unavailable.as_ref().expect("still unavailable");
    assert!(unavailable.can_download, "{unavailable:?}");
    assert!(unavailable.message.contains("origin"), "{unavailable:?}");
    assert_eq!(snap.commits.len(), 4);
    assert!(snap.status.is_clean());

    // Back online, the tree is downloaded and the status filled in.
    fs::rename(&moved, &fx.src).expect("restore the remote");
    repo.complete_status(&mut snap, Download);
    assert!(
        snap.status_unavailable.is_none(),
        "{:?}",
        snap.status_unavailable
    );
    assert_eq!(name_status(&snap.status.staged), expected);
    assert_eq!(snap.commits.len(), 4);
}

#[test]
fn legacy_partial_clone_extension_is_found_from_a_linked_worktree() {
    let fx = Fixture::new();
    let dir = fx.clone("legacy", &["--filter=blob:none"]);
    git(&dir, &["config", "--unset", "remote.origin.promisor"]);
    git(&dir, &["config", "extensions.partialClone", "origin"]);
    // Linked worktrees keep their git directory under `.git/worktrees`, and the configuration in
    // the common directory.
    let wt = fx.root.path().join("wt");
    git(
        &dir,
        &["worktree", "add", "-q", &wt.to_string_lossy(), "HEAD"],
    );
    match Repo::open(&wt) {
        Err(Error::LegacyPartialClone { remote }) => assert_eq!(remote, "origin"),
        other => panic!("expected LegacyPartialClone, got {other:?}"),
    }
}

#[test]
fn remote_named_like_an_option_is_refused_and_never_run() {
    let fx = Fixture::new();
    let dir = fx.clone("blobless", &["--filter=blob:none"]);
    let before = missing(&dir);
    // The only promisor remote is one whose name git would read as `--upload-pack=<command>`.
    let name = "--upload-pack=touch pwned";
    git(&dir, &["config", "--unset", "remote.origin.promisor"]);
    git(&dir, &["config", &format!("remote.{name}.url"), &fx.url]);
    git(
        &dir,
        &["config", &format!("remote.{name}.promisor"), "true"],
    );
    let repo = Repo::open(&dir).expect("open");
    assert!(repo.is_partial_clone());

    for missing_objects in [Fail, Download] {
        let err = repo
            .file_diff(
                &DiffSource::Commit(fx.commits[1]),
                "a.txt",
                None,
                3,
                Whitespace::Show,
                missing_objects,
            )
            .expect_err("nothing may be fetched");
        let Error::NotDownloaded {
            remote: None,
            reason: Some(reason),
        } = &err
        else {
            panic!("expected NotDownloaded without a remote, got {err:?}");
        };
        assert!(reason.contains("refused"), "{reason}");
        assert!(reason.contains("--upload-pack=touch pwned"), "{reason}");
        assert!(!err.can_download(), "{err}");
    }
    let err = repo
        .commit_details(fx.commits[1], Download)
        .expect_err("the counts need blobs");
    assert!(err.to_string().contains("refused"), "{err}");
    assert!(
        !dir.join("pwned").exists(),
        "the command in the remote name ran"
    );
    assert_eq!(missing(&dir), before, "nothing was fetched");

    // A real promisor remote next to it is still used, and the refused one is skipped.
    git(&dir, &["config", "remote.origin.promisor", "true"]);
    let d = repo
        .file_diff(
            &DiffSource::Commit(fx.commits[1]),
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Download,
        )
        .expect("origin still works");
    let patch = git(
        &fx.src,
        &[
            "show",
            "--format=",
            "-U3",
            &fx.commits[1].to_string(),
            "--",
            "a.txt",
        ],
    );
    assert_eq!(our_lines(&d), cli_lines(&patch));
    assert!(!dir.join("pwned").exists());
}

/// `A\tpath`-style lines for a status list, sorted, to compare with `git diff --name-status`.
fn name_status(files: &[FileEntry]) -> Vec<String> {
    let mut out: Vec<String> = files
        .iter()
        .map(|f| {
            let code = match f.kind {
                ChangeKind::Added => 'A',
                ChangeKind::Deleted => 'D',
                ChangeKind::Modified => 'M',
                ChangeKind::Renamed => 'R',
                _ => '?',
            };
            format!("{code}\t{}", f.path)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn treeless_soft_reset_never_reports_head_as_unborn() {
    // libgit2's status treats an unreadable HEAD tree as an unborn branch and would list every
    // file as added; the staged diff would compare against an empty tree.
    let fx = Fixture::new();
    let dir = fx.clone("treeless", &["--filter=tree:0"]);
    let one = fx.commits[0].to_string();
    let four = fx.commits[3].to_string();
    git(&dir, &["reset", "-q", "--soft", &one]);
    let before = missing(&dir);
    let repo = Repo::open(&dir).expect("open");

    // History does not depend on status: the commits are there, the changes say why they are not.
    let snap = repo.snapshot(100).expect("history needs no trees");
    assert_eq!(snap.commits.len(), 4);
    assert!(snap.status.is_clean(), "no file may be reported as added");
    let unavailable = snap
        .status_unavailable
        .as_ref()
        .expect("status unavailable");
    assert!(unavailable.can_download, "{unavailable:?}");
    assert!(
        unavailable.message.contains("partial clone"),
        "{unavailable:?}"
    );
    let err = repo
        .status(Fail)
        .expect_err("status alone reports the missing tree");
    assert_not_downloaded(err, Some("origin"), false);
    let err = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Fail,
        )
        .expect_err("HEAD tree is missing");
    assert_not_downloaded(err, Some("origin"), false);
    assert_eq!(missing(&dir), before, "Fail must not download anything");

    let status = repo.status(Download).expect("downloads the HEAD tree");
    let mut expected: Vec<String> = git(
        &fx.src,
        &["diff", "--name-status", "--no-renames", &one, &four],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    expected.sort();
    assert_eq!(name_status(&status.staged), expected);
    let snap = repo.snapshot(100).expect("snapshot");
    assert!(snap.status_unavailable.is_none(), "the tree is local now");
    assert_eq!(name_status(&snap.status.staged), expected);

    let d = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            Download,
        )
        .expect("download");
    let patch = git(&fx.src, &["diff", "-U3", &one, &four, "--", "a.txt"]);
    assert_eq!(our_lines(&d), cli_lines(&patch));
}
