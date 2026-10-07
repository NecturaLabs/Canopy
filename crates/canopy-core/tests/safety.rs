//! Tests of the safety work in `canopy-core` (branch deletion and publishing, file modes and
//! type changes, large untracked folders, embedded repositories, safe paths and ignore rules),
//! against throwaway repositories built with the `git` CLI.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::{Error, MissingObjects, Repo};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = run_git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn run_git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs")
}

fn configure(p: &Path) {
    git(p, &["config", "user.name", "Ada Lovelace"]);
    git(p, &["config", "user.email", "ada@example.com"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    git(p, &["config", "core.autocrlf", "false"]);
    // Keep the developer's global excludes file out of ignore tests.
    let none = p.join(".git").join("no-global-excludes");
    git(p, &["config", "core.excludesFile", &none.to_string_lossy()]);
}

/// A repository with one commit holding `a.txt`.
fn fixture() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    configure(p);
    fs::write(p.join("a.txt"), "one\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: first"]);
    let repo = Repo::open(p).expect("open");
    (dir, repo)
}

fn commit_file(p: &Path, name: &str, content: &str) {
    fs::write(p.join(name), content).expect("write");
    git(p, &["add", "--", name]);
    git(p, &["commit", "-q", "-m", name]);
}

fn ref_exists(p: &Path, name: &str) -> bool {
    run_git(p, &["rev-parse", "--verify", "-q", name])
        .status
        .success()
}

// --- Branch deletion ---------------------------------------------------------------------------

#[test]
fn merged_branch_deletes_with_dash_d() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["branch", "done"]);
    repo.delete_merged_branch("done")
        .expect("merged branch deletes");
    assert!(!ref_exists(p, "refs/heads/done"));
}

#[test]
fn unmerged_branch_refuses_with_gits_message() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "wip"]);
    commit_file(p, "w.txt", "w\n");
    git(p, &["switch", "-q", "main"]);

    let err = repo.delete_merged_branch("wip").expect_err("unmerged");
    match &err {
        Error::Cli { command, stderr } => {
            assert_eq!(command, "branch");
            assert!(stderr.contains("not fully merged"), "{stderr}");
        }
        other => panic!("expected git's error, got {other:?}"),
    }
    assert!(ref_exists(p, "refs/heads/wip"), "nothing deleted");
    // The compatibility entry point without force behaves the same.
    assert!(repo.delete_branch("wip", false).is_err());

    repo.force_delete_branch("wip").expect("force deletes");
    assert!(!ref_exists(p, "refs/heads/wip"));
}

#[test]
fn branch_delete_does_not_read_a_name_as_an_option() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["branch", "keep"]);
    assert!(repo.delete_merged_branch("-D").is_err());
    assert!(repo.force_delete_branch("--all").is_err());
    assert!(ref_exists(p, "refs/heads/keep"));
}

#[test]
fn create_branch_does_not_read_a_name_or_start_as_an_option() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["branch", "other"]);
    for checkout in [false, true] {
        assert!(matches!(
            repo.create_branch("-D", Some("other"), checkout),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            repo.create_branch("new", Some("--orphan"), checkout),
            Err(Error::Invalid(_))
        ));
    }
    assert!(ref_exists(p, "refs/heads/other"), "nothing deleted");
    assert!(!ref_exists(p, "refs/heads/new"));
    assert_eq!(repo.head().unwrap().branch.as_deref(), Some("main"));
}

// --- New branches and publishing ---------------------------------------------------------------

/// A bare "origin" with `main`, and a clone of it.
fn upstream_and_clone() -> (tempfile::TempDir, tempfile::TempDir, Repo) {
    let origin = tempfile::tempdir().expect("tempdir");
    git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
    let work = tempfile::tempdir().expect("tempdir");
    let wp = work.path();
    git(wp, &["clone", "-q", &origin.path().to_string_lossy(), "."]);
    configure(wp);
    commit_file(wp, "a.txt", "one\n");
    git(wp, &["push", "-q", "origin", "main"]);
    git(wp, &["fetch", "-q", "origin"]);
    let repo = Repo::open(wp).expect("open");
    (origin, work, repo)
}

fn upstream_of(p: &Path, branch: &str) -> Option<String> {
    let out = run_git(
        p,
        &[
            "rev-parse",
            "--abbrev-ref",
            &format!("{branch}@{{upstream}}"),
        ],
    );
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[test]
fn branch_from_a_remote_tracking_ref_has_no_upstream() {
    let (_origin, work, repo) = upstream_and_clone();
    let p = work.path();
    // Even with the setting that tracks every start point.
    git(p, &["config", "branch.autoSetupMerge", "always"]);

    repo.create_branch("topic", Some("origin/main"), false)
        .expect("create");
    assert_eq!(upstream_of(p, "topic"), None);
    repo.create_branch("topic2", Some("origin/main"), true)
        .expect("create and switch");
    assert_eq!(upstream_of(p, "topic2"), None);
    assert_eq!(repo.head().unwrap().branch.as_deref(), Some("topic2"));
    assert_eq!(rev(p, "topic2"), rev(p, "origin/main"));
}

fn rev(p: &Path, r: &str) -> String {
    git(p, &["rev-parse", r]).trim().to_owned()
}

#[test]
fn publish_branch_pushes_a_branch_that_is_not_checked_out() {
    let (origin, work, repo) = upstream_and_clone();
    let p = work.path();
    repo.create_branch("feature/x", Some("origin/main"), true)
        .expect("create");
    commit_file(p, "x.txt", "x\n");
    git(p, &["switch", "-q", "main"]);

    repo.publish_branch("feature/x", "origin", |_| {})
        .expect("publish");
    assert_eq!(
        upstream_of(p, "feature/x").as_deref(),
        Some("origin/feature/x")
    );
    assert_eq!(rev(origin.path(), "feature/x"), rev(p, "feature/x"));
    assert_eq!(repo.head().unwrap().branch.as_deref(), Some("main"));
    // Pushing after publishing goes to the branch's own upstream.
    assert!(
        repo.branches()
            .unwrap()
            .iter()
            .any(|b| b.name == "feature/x" && b.upstream.as_deref() == Some("origin/feature/x"))
    );
}

#[test]
fn publish_branch_rejects_unknown_remotes_and_branches() {
    let (_origin, _work, repo) = upstream_and_clone();
    assert!(matches!(
        repo.publish_branch("main", "nowhere", |_| {}),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        repo.publish_branch("main", "--mirror", |_| {}),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        repo.publish_branch("missing", "origin", |_| {}),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn push_arguments_do_not_treat_remote_names_or_refs_as_options() {
    let (origin, work, repo) = upstream_and_clone();
    let p = work.path();
    git(p, &["tag", "v1"]);
    repo.push_tag("origin", "v1", |_| {}).expect("push tag");
    assert!(ref_exists(origin.path(), "refs/tags/v1"));

    for result in [
        repo.push_tag("--mirror", "v1", |_| {}),
        repo.delete_remote_branch("origin", "--mirror", |_| {}),
    ] {
        assert!(
            result.is_err(),
            "option-shaped input must not run an option"
        );
    }
    assert!(ref_exists(origin.path(), "refs/heads/main"));
    assert!(ref_exists(origin.path(), "refs/tags/v1"));

    // The ordinary deletion works with the separator: do not delete main, which the
    // non-bare upstream has checked out (receive.denyDeleteCurrent).
    git(p, &["branch", "feature-delete"]);
    git(p, &["push", "origin", "feature-delete"]);
    assert!(ref_exists(origin.path(), "refs/heads/feature-delete"));
    repo.delete_remote_branch("origin", "feature-delete", |_| {})
        .expect("delete remote branch");
    assert!(!ref_exists(origin.path(), "refs/heads/feature-delete"));
}

#[test]
fn rename_and_checkout_refuse_option_shaped_names() {
    let (_origin, work, repo) = upstream_and_clone();
    assert!(repo.rename_branch("main", "--delete").is_err());
    assert!(repo.checkout_remote_branch("origin", "--orphan").is_err());
    assert_eq!(repo.head().unwrap().branch.as_deref(), Some("main"));
    assert!(ref_exists(work.path(), "refs/heads/main"));
}

// --- File modes and type changes ---------------------------------------------------------------

#[cfg(unix)]
fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).expect("chmod");
}

#[cfg(unix)]
#[test]
fn mode_only_change_shows_in_status_and_diff() {
    use canopy_core::{ChangeKind, DiffSource, Whitespace};
    let (dir, repo) = fixture();
    let p = dir.path();
    chmod(&p.join("a.txt"), 0o755);

    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert_eq!(st.unstaged.len(), 1, "{st:?}");
    let e = &st.unstaged[0];
    assert_eq!(e.kind, ChangeKind::Modified);
    assert_eq!(e.mode_change(), Some((0o100_644, 0o100_755)));

    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .unwrap();
    assert!(d.hunks.is_empty(), "{:?}", d.hunks);
    assert_eq!(d.mode_change(), Some((0o100_644, 0o100_755)));

    // Staged, and then in the commit.
    repo.stage(&["a.txt".to_owned()]).unwrap();
    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert!(st.unstaged.is_empty(), "{:?}", st.unstaged);
    assert_eq!(st.staged[0].mode_change(), Some((0o100_644, 0o100_755)));
    let d = repo
        .file_diff(
            &DiffSource::Staged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .unwrap();
    assert_eq!(d.mode_change(), Some((0o100_644, 0o100_755)));
    let oid = repo
        .commit("chore: make a.txt executable", false, false)
        .unwrap();
    let details = repo.commit_details(oid, MissingObjects::Fail).unwrap();
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].mode_change(), Some((0o100_644, 0o100_755)));
    let d = repo
        .file_diff(
            &DiffSource::Commit(oid),
            "a.txt",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .unwrap();
    assert_eq!(d.mode_change(), Some((0o100_644, 0o100_755)));
}

#[cfg(unix)]
#[test]
fn core_filemode_false_hides_mode_only_changes() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["config", "core.fileMode", "false"]);
    chmod(&p.join("a.txt"), 0o755);
    // Same answer as git.
    assert_eq!(git(p, &["status", "--porcelain"]), "");
    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert!(st.is_clean(), "{st:?}");
}

#[cfg(unix)]
#[test]
fn file_to_symlink_is_one_typechange_entry_with_both_modes() {
    use canopy_core::{ChangeKind, DiffSource, Whitespace};
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::remove_file(p.join("a.txt")).unwrap();
    std::os::unix::fs::symlink("target.txt", p.join("a.txt")).unwrap();

    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert_eq!(st.unstaged.len(), 1, "{st:?}");
    assert_eq!(st.unstaged[0].kind, ChangeKind::Typechange);
    assert_eq!(st.unstaged[0].mode_change(), Some((0o100_644, 0o120_000)));

    let d = repo
        .file_diff(
            &DiffSource::Unstaged,
            "a.txt",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .unwrap();
    assert_eq!(d.kind, Some(ChangeKind::Typechange));
    assert_eq!(d.mode_change(), Some((0o100_644, 0o120_000)));

    // Hunk staging refuses a type change instead of applying half of it.
    let key = d.hunks.first().map_or((0, 0, 0, 0), canopy_core::Hunk::key);
    for result in [
        repo.apply_hunk(canopy_core::HunkAction::Stage, "a.txt", 0, key, 3),
        repo.apply_lines(canopy_core::HunkAction::Stage, "a.txt", 0, key, &[0], 3),
    ] {
        match result {
            Err(Error::Invalid(msg)) => assert!(msg.contains("changed type"), "{msg}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    repo.stage(&["a.txt".to_owned()]).unwrap();
    let oid = repo.commit("refactor: link a.txt", false, false).unwrap();
    let details = repo.commit_details(oid, MissingObjects::Fail).unwrap();
    assert_eq!(details.files.len(), 1, "{:?}", details.files);
    assert_eq!(details.files[0].kind, ChangeKind::Typechange);
    assert_eq!(details.files[0].mode_change(), Some((0o100_644, 0o120_000)));
    let d = repo
        .file_diff(
            &DiffSource::Commit(oid),
            "a.txt",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .unwrap();
    assert_eq!(d.kind, Some(ChangeKind::Typechange));
    assert_eq!(d.mode_change(), Some((0o100_644, 0o120_000)));
}

// --- Untracked folders ---------------------------------------------------------------------------

fn write_files(root: &Path, dir: &str, n: usize) {
    let d = root.join(dir);
    fs::create_dir_all(&d).unwrap();
    for i in 0..n {
        fs::write(d.join(format!("f{i}.js")), "x\n").unwrap();
    }
}

#[test]
fn small_untracked_folders_list_every_file() {
    let (dir, repo) = fixture();
    let p = dir.path();
    write_files(p, "new/deep", 3);
    fs::write(p.join("loose.txt"), "l\n").unwrap();
    let st = canopy_core::status::collect_with(repo.git(), 10).unwrap();
    let paths: Vec<&str> = st.unstaged.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "loose.txt",
            "new/deep/f0.js",
            "new/deep/f1.js",
            "new/deep/f2.js"
        ]
    );
    assert!(st.untracked_dirs.is_empty(), "{:?}", st.untracked_dirs);
    assert_eq!(st.unstaged[0].new_mode, Some(0o100_644));
}

#[test]
fn many_untracked_files_collapse_to_folders_and_expand_one_level() {
    let (dir, repo) = fixture();
    let p = dir.path();
    write_files(p, "node_modules/a", 6);
    write_files(p, "node_modules", 2);
    write_files(p, "build", 3);
    fs::write(p.join("loose.txt"), "l\n").unwrap();
    // Ignored files are neither listed nor counted.
    fs::write(p.join(".gitignore"), "*.log\n").unwrap();
    fs::write(p.join("node_modules").join("debug.log"), "d\n").unwrap();

    let st = canopy_core::status::collect_with(repo.git(), 5).unwrap();
    let paths: Vec<&str> = st.unstaged.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, [".gitignore", "loose.txt"]);
    let dirs: Vec<(&str, usize, bool)> = st
        .untracked_dirs
        .iter()
        .map(|d| (d.path.as_str(), d.files, d.repository))
        .collect();
    assert_eq!(dirs, [("build/", 3, false), ("node_modules/", 8, false)]);
    assert!(!st.is_clean());

    let listing = repo.untracked_children("node_modules").unwrap();
    let files: Vec<&str> = listing.files.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(files, ["node_modules/f0.js", "node_modules/f1.js"]);
    let sub: Vec<(&str, usize)> = listing
        .dirs
        .iter()
        .map(|d| (d.path.as_str(), d.files))
        .collect();
    assert_eq!(sub, [("node_modules/a/", 6)]);

    // The default limit lists them all.
    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert_eq!(st.unstaged.len(), 13);
    assert!(st.untracked_dirs.is_empty(), "{:?}", st.untracked_dirs);
    assert!(matches!(
        repo.untracked_children("../outside"),
        Err(Error::UnsafePath { .. })
    ));
}

// --- Embedded repositories ---------------------------------------------------------------------

fn nested_repository(root: &Path, dir: &str) {
    let d = root.join(dir);
    fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "main"]);
    configure(&d);
    commit_file(&d, "inner.txt", "inner\n");
}

#[test]
fn embedded_repositories_are_listed_and_not_staged_by_stage_all() {
    let (dir, repo) = fixture();
    let p = dir.path();
    nested_repository(p, "vendored");
    nested_repository(p, "tools/sub repo");
    fs::write(p.join("tools").join("script.sh"), "echo\n").unwrap();
    fs::write(p.join("new.txt"), "n\n").unwrap();

    let st = canopy_core::status::collect(repo.git()).unwrap();
    let repos: Vec<(&str, bool)> = st
        .untracked_dirs
        .iter()
        .map(|d| (d.path.as_str(), d.repository))
        .collect();
    assert_eq!(repos, [("tools/sub repo/", true), ("vendored/", true)]);
    let paths: Vec<&str> = st.unstaged.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["new.txt", "tools/script.sh"]);

    let skipped = repo.stage_all_except_repositories().unwrap();
    assert_eq!(skipped, ["tools/sub repo/", "vendored/"]);
    let staged = git(p, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "new.txt\ntools/script.sh\n");
    // No gitlink anywhere in the index.
    assert!(!git(p, &["ls-files", "-s"]).contains("160000"));

    // The compatibility entry point does the same.
    git(p, &["reset", "-q"]);
    repo.stage_all().unwrap();
    assert!(!git(p, &["ls-files", "-s"]).contains("160000"));
    assert_eq!(
        git(p, &["diff", "--cached", "--name-only"]),
        "new.txt\ntools/script.sh\n"
    );
}

// --- Ignore rules --------------------------------------------------------------------------------

use canopy_core::{IgnoreRule, IgnoreTarget};

/// Whether two spellings name the same file, which need not exist yet. On Windows the work folder
/// comes back from libgit2 with `/` and long names, the temporary folder in its 8.3 short form.
#[track_caller]
fn assert_same_file(actual: &Path, expected: &Path) {
    let folder = |p: &Path| fs::canonicalize(p.parent().expect("a folder")).expect("canonical");
    assert!(
        actual.file_name() == expected.file_name() && folder(actual) == folder(expected),
        "{} is not {}",
        actual.display(),
        expected.display()
    );
}

#[test]
fn ignore_a_file_at_the_root_and_ask_why() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::create_dir_all(p.join("dir")).unwrap();
    fs::write(p.join("dir").join("#odd [1].txt"), "x\n").unwrap();
    fs::write(p.join(".gitignore"), "# existing, no final newline").unwrap();

    let rule = IgnoreRule::File("dir/#odd [1].txt".into());
    let out = repo.add_ignore_rule(&rule, IgnoreTarget::Root).unwrap();
    assert!(out.added);
    assert_eq!(out.pattern, "/dir/#odd \\[1\\].txt");
    assert_same_file(&out.file, &p.join(".gitignore"));
    assert_eq!(
        fs::read(p.join(".gitignore")).unwrap(),
        b"# existing, no final newline\n/dir/#odd \\[1\\].txt\n"
    );
    let check = out.check.expect("now ignored");
    assert_eq!(check.source, ".gitignore");
    assert_eq!(check.line, 2);
    assert!(!check.negated());
    // git agrees, and status no longer lists it.
    git(p, &["check-ignore", "-q", "--", "dir/#odd [1].txt"]);
    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert!(st.unstaged.iter().all(|e| e.path != "dir/#odd [1].txt"));

    // Adding it again writes nothing.
    let again = repo.add_ignore_rule(&rule, IgnoreTarget::Root).unwrap();
    assert!(!again.added);
    assert_eq!(
        fs::read_to_string(p.join(".gitignore"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn nearest_gitignore_gets_a_pattern_relative_to_its_folder() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::create_dir_all(p.join("sub").join("deep")).unwrap();
    fs::write(p.join("sub").join(".gitignore"), "").unwrap();
    fs::write(p.join("sub").join("deep").join("x.log"), "x\n").unwrap();

    let file = IgnoreRule::File("sub/deep/x.log".into());
    let dest = repo
        .ignore_destination(IgnoreTarget::Nearest, file.path())
        .unwrap();
    assert_same_file(&dest.file, &p.join("sub").join(".gitignore"));
    assert_eq!(dest.base, "sub");
    let out = repo.add_ignore_rule(&file, IgnoreTarget::Nearest).unwrap();
    assert_eq!(out.pattern, "/deep/x.log");
    assert_eq!(out.check.unwrap().source, "sub/.gitignore");

    // Without a closer file, the root one.
    let top = IgnoreRule::Folder("other/".into());
    assert_same_file(
        &repo
            .ignore_destination(IgnoreTarget::Nearest, top.path())
            .unwrap()
            .file,
        &p.join(".gitignore"),
    );
}

#[test]
fn repository_exclude_and_global_files() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::write(p.join("local.tmp"), "x\n").unwrap();
    let out = repo
        .add_ignore_rule(
            &IgnoreRule::Extension("local.tmp".into()),
            IgnoreTarget::RepositoryExclude,
        )
        .unwrap();
    assert_eq!(out.pattern, "*.tmp");
    let exclude = p.join(".git").join("info").join("exclude");
    assert_same_file(&out.file, &exclude);
    assert!(fs::read_to_string(&exclude).unwrap().ends_with("*.tmp\n"));
    assert!(out.check.unwrap().source.ends_with("info/exclude"));

    let global_dir = tempfile::tempdir().unwrap();
    let global = global_dir.path().join("ignore");
    git(
        p,
        &["config", "core.excludesFile", &global.to_string_lossy()],
    );
    fs::write(p.join("notes.swp"), "x\n").unwrap();
    let out = repo
        .add_ignore_rule(
            &IgnoreRule::Custom {
                pattern: "*.swp".into(),
                near: "notes.swp".into(),
            },
            IgnoreTarget::Global,
        )
        .unwrap();
    assert_same_file(&out.file, &global);
    assert_eq!(fs::read(&global).unwrap(), b"*.swp\n");
    assert!(out.check.is_some());
}

#[cfg(unix)]
#[test]
fn ignore_files_that_are_symlinks_are_refused() {
    let (dir, repo) = fixture();
    let p = dir.path();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("bashrc");
    fs::write(&victim, "keep\n").unwrap();
    std::os::unix::fs::symlink(&victim, p.join(".gitignore")).unwrap();
    fs::write(p.join("x.txt"), "x\n").unwrap();

    for target in [IgnoreTarget::Root, IgnoreTarget::Nearest] {
        let err = repo
            .add_ignore_rule(&IgnoreRule::File("x.txt".into()), target)
            .expect_err("symlinked .gitignore");
        assert!(matches!(err, Error::UnsafePath { .. }), "{err:?}");
    }
    let exclude = p.join(".git").join("info").join("exclude");
    let _ = fs::remove_file(&exclude);
    std::os::unix::fs::symlink(&victim, &exclude).unwrap();
    assert!(matches!(
        repo.add_ignore_rule(
            &IgnoreRule::File("x.txt".into()),
            IgnoreTarget::RepositoryExclude
        ),
        Err(Error::UnsafePath { .. })
    ));
    // A folder reached through a link is refused too.
    std::os::unix::fs::symlink(outside.path(), p.join("linked")).unwrap();
    assert!(matches!(
        repo.add_ignore_rule(
            &IgnoreRule::File("linked/y.txt".into()),
            IgnoreTarget::Nearest
        ),
        Err(Error::UnsafePath { .. })
    ));
    assert_eq!(fs::read(&victim).unwrap(), b"keep\n");
}

#[test]
fn why_ignored_covers_tracked_files_and_negation() {
    let (dir, repo) = fixture();
    let p = dir.path();
    assert_eq!(repo.why_ignored("a.txt").unwrap(), None);
    fs::write(p.join(".gitignore"), "*.txt\n!keep.txt\n").unwrap();
    // Tracked, yet a rule matches it.
    let m = repo.why_ignored("a.txt").unwrap().expect("matches");
    assert_eq!(
        (m.source.as_str(), m.line, m.pattern.as_str()),
        (".gitignore", 1, "*.txt")
    );
    let keep = repo.why_ignored("keep.txt").unwrap().expect("negation");
    assert!(keep.negated());
    assert_eq!(keep.line, 2);
    repo.why_ignored("../x").unwrap_err();
}

#[test]
fn stop_tracking_keeps_the_file() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::create_dir_all(p.join("gen")).unwrap();
    fs::write(p.join("gen").join("out.bin"), "o\n").unwrap();
    // Both names are valid on Windows; as a glob, `[ab].txt` would also match a.txt.
    fs::write(p.join("[ab].txt"), "glob\n").unwrap();
    git(p, &["add", "--", "gen", ":(literal)[ab].txt"]);
    git(p, &["commit", "-q", "-m", "add generated"]);

    repo.stop_tracking("gen/").unwrap();
    // A literal name, not a glob: a.txt stays tracked.
    repo.stop_tracking("[ab].txt").unwrap();
    let tracked = git(p, &["ls-files"]);
    assert_eq!(tracked, "a.txt\n");
    assert!(p.join("gen").join("out.bin").exists());
    assert!(p.join("[ab].txt").exists());
}

#[test]
fn preview_reports_what_a_rule_would_change_without_writing() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::create_dir_all(p.join("sub").join("deep")).unwrap();
    fs::write(p.join("sub").join(".gitignore"), "keep.log\n").unwrap();
    let paths: Vec<String> = [
        "top.log",
        "sub/a.log",
        "sub/deep/b.log",
        "sub/c.txt",
        "a.txt",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    let rule = IgnoreRule::Extension("sub/a.log".into());
    let nearest = repo
        .preview_ignore_rule(&rule, IgnoreTarget::Nearest, &paths)
        .unwrap();
    assert_eq!(nearest.ignored, ["sub/a.log", "sub/deep/b.log"]);
    let root = repo
        .preview_ignore_rule(&rule, IgnoreTarget::Root, &paths)
        .unwrap();
    assert_eq!(root.ignored, ["top.log", "sub/a.log", "sub/deep/b.log"]);

    let neg = IgnoreRule::Custom {
        pattern: "!keep.log".into(),
        near: "sub/keep.log".into(),
    };
    let un = repo
        .preview_ignore_rule(&neg, IgnoreTarget::Nearest, &["sub/keep.log".to_owned()])
        .unwrap();
    assert_eq!(un.unignored, ["sub/keep.log"]);

    // Nothing was written, and the repository's own handle has no extra rules.
    assert!(!p.join(".gitignore").exists());
    assert_eq!(
        fs::read_to_string(p.join("sub").join(".gitignore")).unwrap(),
        "keep.log\n"
    );
    assert!(!repo.git().is_path_ignored("top.log").unwrap());
}

#[test]
fn ignore_case_setting_decides_duplicates() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::write(p.join(".gitignore"), "/README.MD\n").unwrap();
    git(p, &["config", "core.ignorecase", "true"]);
    let out = repo
        .add_ignore_rule(&IgnoreRule::File("readme.md".into()), IgnoreTarget::Root)
        .unwrap();
    assert!(!out.added);
    git(p, &["config", "core.ignorecase", "false"]);
    let out = repo
        .add_ignore_rule(&IgnoreRule::File("readme.md".into()), IgnoreTarget::Root)
        .unwrap();
    assert!(out.added);
}

// --- Safe paths in existing readers ------------------------------------------------------------

#[cfg(unix)]
#[test]
fn conflict_view_refuses_a_path_through_a_symlink() {
    let (dir, repo) = fixture();
    let p = dir.path();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "s\n").unwrap();
    std::os::unix::fs::symlink(outside.path(), p.join("out")).unwrap();
    assert!(matches!(
        repo.conflict_view("out/secret"),
        Err(Error::UnsafePath { .. })
    ));
    assert!(matches!(
        repo.conflict_view("../x"),
        Err(Error::UnsafePath { .. })
    ));
}

#[test]
fn preview_follows_gits_precedence() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::create_dir_all(p.join("build")).unwrap();
    fs::write(p.join("build").join(".gitignore"), "!a.o\n").unwrap();
    fs::write(p.join(".gitignore"), "!keep.tmp\n").unwrap();
    let paths: Vec<String> = ["build/a.o", "b.o", "keep.tmp", "x.tmp"]
        .into_iter()
        .map(String::from)
        .collect();

    // A closer .gitignore's negation beats a root rule.
    let root = repo
        .preview_ignore_rule(
            &IgnoreRule::Extension("b.o".into()),
            IgnoreTarget::Root,
            &paths,
        )
        .unwrap();
    assert_eq!(root.ignored, ["b.o"]);
    // Every .gitignore beats the exclude file.
    let exclude = repo
        .preview_ignore_rule(
            &IgnoreRule::Extension("x.tmp".into()),
            IgnoreTarget::RepositoryExclude,
            &paths,
        )
        .unwrap();
    assert_eq!(exclude.ignored, ["x.tmp"]);

    // And the preview agrees with git after writing.
    repo.add_ignore_rule(&IgnoreRule::Extension("b.o".into()), IgnoreTarget::Root)
        .unwrap();
    assert!(repo.why_ignored("build/a.o").unwrap().unwrap().negated());
    assert!(!repo.why_ignored("b.o").unwrap().unwrap().negated());
}

#[test]
fn preview_escapes_the_nearest_folder() {
    let (dir, repo) = fixture();
    let p = dir.path();
    fs::create_dir_all(p.join("app[1]").join("x")).unwrap();
    fs::write(p.join("app[1]").join(".gitignore"), "").unwrap();
    let rule = IgnoreRule::Extension("app[1]/x/y.log".into());
    let preview = repo
        .preview_ignore_rule(&rule, IgnoreTarget::Nearest, &["app[1]/x/y.log".to_owned()])
        .unwrap();
    assert_eq!(preview.ignored, ["app[1]/x/y.log"]);
}

#[test]
fn ignore_rule_with_a_bad_path_writes_nothing() {
    let (dir, repo) = fixture();
    let p = dir.path();
    for near in ["", "../x", "."] {
        let rule = IgnoreRule::Custom {
            pattern: "*.zzz".into(),
            near: near.into(),
        };
        for target in [IgnoreTarget::Root, IgnoreTarget::RepositoryExclude] {
            assert!(repo.add_ignore_rule(&rule, target).is_err(), "{near:?}");
        }
    }
    assert!(!p.join(".gitignore").exists());
    let exclude =
        fs::read_to_string(p.join(".git").join("info").join("exclude")).unwrap_or_default();
    assert!(!exclude.contains("zzz"));
}

#[test]
fn stop_tracking_refuses_the_whole_tree() {
    let (dir, repo) = fixture();
    let p = dir.path();
    for root in [".", "./", ""] {
        assert!(repo.stop_tracking(root).is_err(), "{root:?}");
    }
    assert_eq!(git(p, &["ls-files"]), "a.txt\n");
}

#[test]
fn collapsed_folders_flag_repositories_inside() {
    let (dir, repo) = fixture();
    let p = dir.path();
    write_files(p, "top/files", 4);
    nested_repository(p, "top/inner");
    let st = canopy_core::status::collect_with(repo.git(), 2).unwrap();
    let top: Vec<_> = st
        .untracked_dirs
        .iter()
        .map(|d| {
            (
                d.path.as_str(),
                d.files,
                d.repository,
                d.contains_repository,
            )
        })
        .collect();
    assert_eq!(top, [("top/", 4, false, true)]);

    let listing = repo.untracked_children("top").unwrap();
    let dirs: Vec<_> = listing
        .dirs
        .iter()
        .map(|d| (d.path.as_str(), d.files, d.repository))
        .collect();
    assert_eq!(dirs, [("top/files/", 4, false), ("top/inner/", 0, true)]);
    assert!(matches!(
        repo.untracked_children("top/inner"),
        Err(Error::Invalid(_))
    ));
}

/// Linux file systems accept names that are not UTF-8 (macOS and Windows do not).
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_folders_are_listed_and_never_staged_as_gitlinks() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let create_non_utf8_dir = |path: &Path| {
        fs::create_dir(path).unwrap_or_else(|error| {
            panic!(
                "cannot create non-UTF-8 test directory at {} (TMPDIR may be on NTFS or another filesystem that rejects this name): {error}",
                path.display()
            )
        });
    };
    let (dir, repo) = fixture();
    let p = dir.path();
    let odd = p.join(OsStr::from_bytes(b"odd\xffdir"));
    create_non_utf8_dir(&odd);
    fs::write(odd.join("f.txt"), "f\n").unwrap();

    // The folder's file is listed, not lost.
    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert_eq!(st.unstaged.len(), 1, "{st:?}");
    assert!(st.unstaged[0].path.ends_with("/f.txt"));
    let st = canopy_core::status::collect_with(repo.git(), 0).unwrap();
    assert_eq!(st.untracked_dirs.len(), 1, "{st:?}");
    assert_eq!(st.untracked_dirs[0].files, 1);

    // A repository with such a name is found, and Stage all refuses rather than adding it.
    fs::remove_dir_all(&odd).unwrap();
    let nested = p.join(OsStr::from_bytes(b"repo\xff"));
    create_non_utf8_dir(&nested);
    git(&nested, &["init", "-q", "-b", "main"]);
    configure(&nested);
    commit_file(&nested, "inner.txt", "inner\n");
    let st = canopy_core::status::collect(repo.git()).unwrap();
    assert!(st.untracked_dirs.iter().any(|d| d.repository), "{st:?}");
    assert!(matches!(
        repo.stage_all_except_repositories(),
        Err(Error::Invalid(_))
    ));
    assert!(!git(p, &["ls-files", "-s"]).contains("160000"));
}
