//! git-flow, merged-branch detection, Git LFS awareness and submodules, against throwaway
//! repositories. Where Canopy runs a sequence of git commands, the same sequence run with the git
//! CLI in a twin repository is the oracle.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;

use canopy_core::{
    ChangeKind, DiffSource, FlowConfig, FlowFinish, FlowKind, GitOptions, MergedReason,
    MissingObjects, PullStrategy, Repo, SubmoduleState, Whitespace, flow_finish_requirement,
    submodule_add_problem,
};

fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    git_output_os(dir, &args)
}

#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly to build and check repositories"
)]
fn git_output_os(dir: &Path, args: &[&OsStr]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // The fixtures must not depend on the user's global configuration; a missing file reads
        // as empty.
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("canopy-tests-no-global-gitconfig"),
        )
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

/// Initialises `p` with a local identity, no signing and LF line endings, and one commit of
/// `a.txt` on `main`.
fn init(p: &Path) {
    git(p, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Ada Lovelace"),
        ("user.email", "ada@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(p, &["config", key, value]);
    }
    fs::write(p.join("a.txt"), "one\ntwo\nthree\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: first"]);
}

fn fixture() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let repo = Repo::open(dir.path()).expect("open");
    (dir, repo)
}

fn commit_file(p: &Path, file: &str, text: &str, message: &str) {
    if let Some(parent) = Path::new(file).parent() {
        fs::create_dir_all(p.join(parent)).expect("mkdir");
    }
    fs::write(p.join(file), text).expect("write");
    git(p, &["add", "--", file]);
    git(p, &["commit", "-q", "-m", message]);
}

/// The history as trees, subjects and graph shape (commit ids differ between twins because of
/// commit times), the refs, the annotated tags' subjects and the checked-out branch.
fn shape(p: &Path) -> String {
    let log = git(p, &["log", "--graph", "--all", "--format=%T %s"]);
    let refs = git(
        p,
        &[
            "for-each-ref",
            "--format=%(refname) %(objecttype) %(contents:subject)",
        ],
    );
    let head = git(p, &["symbolic-ref", "HEAD"]);
    format!("{log}\n{refs}\n{head}")
}

/// Two identical git-flow repositories: `main` with one commit, `develop` with one more.
fn flow_twins() -> (tempfile::TempDir, Repo, tempfile::TempDir) {
    let (dir, repo) = fixture();
    let cli = tempfile::tempdir().expect("tempdir");
    init(cli.path());
    repo.flow_init(&FlowConfig {
        master: "main".into(),
        version_tag: "v".into(),
        ..FlowConfig::default()
    })
    .expect("flow init");
    for p in [dir.path(), cli.path()] {
        if p == cli.path() {
            git(p, &["branch", "--no-track", "develop", "main"]);
        }
        git(p, &["switch", "-q", "develop"]);
        commit_file(p, "dev.txt", "dev\n", "feat: develop work");
    }
    (dir, repo, cli)
}

#[test]
fn flow_init_writes_the_avh_keys_and_creates_develop() {
    let (dir, repo) = fixture();
    let p = dir.path();
    assert_eq!(repo.flow_config().expect("config"), None);
    let suggested = repo.flow_suggested_config().expect("suggested");
    assert_eq!(suggested.master, "main");
    assert_eq!(suggested.develop, "develop");

    repo.flow_init(&suggested).expect("init");
    assert_eq!(git(p, &["config", "gitflow.branch.master"]).trim(), "main");
    assert_eq!(
        git(p, &["config", "gitflow.prefix.feature"]).trim(),
        "feature/"
    );
    assert_eq!(
        git(p, &["rev-parse", "develop"]),
        git(p, &["rev-parse", "main"])
    );
    assert_eq!(repo.flow_config().expect("config"), Some(suggested.clone()));
    // HEAD stays where it was.
    assert_eq!(git(p, &["symbolic-ref", "--short", "HEAD"]).trim(), "main");

    let same = FlowConfig {
        develop: "main".into(),
        ..suggested.clone()
    };
    assert!(repo.flow_init(&same).is_err());
    let missing = FlowConfig {
        master: "trunk".into(),
        ..suggested
    };
    assert!(repo.flow_init(&missing).is_err());
}

#[test]
fn flow_config_reads_custom_prefixes() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["config", "gitflow.branch.master", "main"]);
    git(p, &["config", "gitflow.branch.develop", "dev"]);
    git(p, &["config", "gitflow.prefix.feature", "feat-"]);
    let config = repo.flow_config().expect("config").expect("initialised");
    assert_eq!(config.develop, "dev");
    assert_eq!(config.branch(FlowKind::Feature, "x"), "feat-x");
    assert_eq!(config.release, "release/");
    assert_eq!(
        config.classify("feat-login"),
        Some((FlowKind::Feature, "login"))
    );
    assert_eq!(config.classify("main"), None);
}

#[test]
fn feature_start_and_finish_match_the_git_commands() {
    let (dir, repo, cli) = flow_twins();
    let (p, c) = (dir.path(), cli.path());

    assert_eq!(
        repo.flow_start(FlowKind::Feature, "login", None)
            .expect("start"),
        "feature/login"
    );
    git(c, &["switch", "-q", "-c", "feature/login", "develop"]);
    for d in [p, c] {
        commit_file(d, "login.txt", "login\n", "feat: login");
        commit_file(d, "login2.txt", "more\n", "feat: more login");
    }

    repo.flow_finish(FlowKind::Feature, "login", &FlowFinish::default())
        .expect("finish");
    git(c, &["switch", "-q", "develop"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", "feature/login"]);
    git(c, &["branch", "-d", "feature/login"]);

    assert_eq!(shape(p), shape(c));
    assert!(shape(p).contains("Merge branch 'feature/login' into develop"));
}

#[test]
fn a_single_commit_feature_or_bugfix_finishes_with_a_fast_forward() {
    for kind in [FlowKind::Feature, FlowKind::Bugfix] {
        for setting in ["false", "only"] {
            let (dir, repo, cli) = flow_twins();
            let (p, c) = (dir.path(), cli.path());
            let branch = repo.flow_start(kind, "small", None).expect("start");
            git(c, &["switch", "-q", "-c", &branch, "develop"]);
            for d in [p, c] {
                git(d, &["config", "merge.ff", setting]);
                commit_file(d, "small.txt", "one change\n", "feat: one small change");
            }
            let tip = git(p, &["rev-parse", "HEAD"]);

            repo.flow_finish(kind, "small", &FlowFinish::default())
                .expect("finish");
            git(c, &["switch", "-q", "develop"]);
            git(c, &["merge", "--ff", "--no-edit", "-q", &branch]);
            git(c, &["branch", "-d", &branch]);

            assert_eq!(git(p, &["rev-parse", "HEAD"]), tip);
            assert_eq!(shape(p), shape(c));
        }
    }
}

#[test]
fn a_diverged_single_commit_feature_finishes_with_a_merge() {
    let (dir, repo, cli) = flow_twins();
    let (p, c) = (dir.path(), cli.path());
    let branch = repo
        .flow_start(FlowKind::Feature, "small", None)
        .expect("start");
    git(c, &["switch", "-q", "-c", &branch, "develop"]);
    for d in [p, c] {
        commit_file(d, "small.txt", "one change\n", "feat: one small change");
        git(d, &["switch", "-q", "develop"]);
        commit_file(d, "other.txt", "another change\n", "feat: develop moved");
        git(d, &["config", "merge.ff", "only"]);
    }

    repo.flow_finish(FlowKind::Feature, "small", &FlowFinish::default())
        .expect("finish");
    git(c, &["merge", "--ff", "--no-edit", "-q", &branch]);
    git(c, &["branch", "-d", &branch]);

    assert_eq!(shape(p), shape(c));
    assert!(shape(p).contains("Merge branch 'feature/small' into develop"));
}

#[test]
fn bugfix_squash_finish_matches_the_git_commands() {
    let (dir, repo, cli) = flow_twins();
    let (p, c) = (dir.path(), cli.path());
    repo.flow_start(FlowKind::Bugfix, "crash", Some("main"))
        .expect("start");
    git(c, &["switch", "-q", "-c", "bugfix/crash", "main"]);
    for d in [p, c] {
        commit_file(d, "fix.txt", "fix\n", "fix: crash");
    }

    // develop and the bugfix have diverged; `merge.ff=only` must not stop the squash.
    git(p, &["config", "merge.ff", "only"]);
    let squash = FlowFinish {
        squash: true,
        ..FlowFinish::default()
    };
    repo.flow_finish(FlowKind::Bugfix, "crash", &squash)
        .expect("finish");
    git(c, &["switch", "-q", "develop"]);
    git(c, &["merge", "--squash", "-q", "bugfix/crash"]);
    git(c, &["commit", "-q", "--no-edit"]);
    git(c, &["branch", "-D", "bugfix/crash"]);

    assert_eq!(shape(p), shape(c));
    assert_eq!(
        git(p, &["rev-list", "--merges", "--count", "develop"]).trim(),
        "0"
    );
}

#[test]
fn release_finish_merges_tags_and_merges_back_like_the_git_commands() {
    let (dir, repo, cli) = flow_twins();
    let (p, c) = (dir.path(), cli.path());
    repo.flow_start(FlowKind::Release, "1.2.0", None)
        .expect("start");
    git(c, &["switch", "-q", "-c", "release/1.2.0", "develop"]);
    for d in [p, c] {
        commit_file(d, "VERSION", "1.2.0\n", "chore: bump version");
    }

    let options = FlowFinish {
        tag_message: Some("Release 1.2.0".into()),
        ..FlowFinish::default()
    };
    repo.flow_finish(FlowKind::Release, "1.2.0", &options)
        .expect("finish");
    git(c, &["switch", "-q", "main"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", "release/1.2.0"]);
    git(c, &["tag", "-a", "v1.2.0", "-m", "Release 1.2.0", "main"]);
    git(c, &["switch", "-q", "develop"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", "v1.2.0"]);
    git(c, &["branch", "-d", "release/1.2.0"]);

    assert_eq!(shape(p), shape(c));
    assert_eq!(git(p, &["cat-file", "-t", "v1.2.0"]).trim(), "tag");
    assert_eq!(
        git(p, &["rev-parse", "v1.2.0^{commit}"]),
        git(p, &["rev-parse", "main"])
    );
}

#[test]
fn hotfix_starts_from_production_and_finishes_without_a_tag() {
    let (dir, repo, cli) = flow_twins();
    let (p, c) = (dir.path(), cli.path());
    repo.flow_start(FlowKind::Hotfix, "1.0.1", None)
        .expect("start");
    assert_eq!(
        git(p, &["rev-parse", "HEAD"]),
        git(p, &["rev-parse", "main"])
    );
    git(c, &["switch", "-q", "-c", "hotfix/1.0.1", "main"]);
    for d in [p, c] {
        commit_file(d, "a.txt", "one\ntwo\nthree\nfixed\n", "fix: urgent");
    }

    let options = FlowFinish {
        tag: false,
        ..FlowFinish::default()
    };
    repo.flow_finish(FlowKind::Hotfix, "1.0.1", &options)
        .expect("finish");
    git(c, &["switch", "-q", "main"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", "hotfix/1.0.1"]);
    git(c, &["switch", "-q", "develop"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", "main"]);
    git(c, &["branch", "-d", "hotfix/1.0.1"]);

    assert_eq!(shape(p), shape(c));
    assert_eq!(git(p, &["tag"]).trim(), "");
}

#[test]
fn flow_start_refuses_what_git_flow_refuses() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    repo.flow_start(FlowKind::Feature, "x", None)
        .expect("start");
    git(p, &["switch", "-q", "develop"]);
    repo.flow_start(FlowKind::Feature, "x", None).unwrap_err();
    repo.flow_start(FlowKind::Feature, "y", Some("nope"))
        .unwrap_err();
    repo.flow_start(FlowKind::Feature, "bad..name", None)
        .unwrap_err();

    repo.flow_start(FlowKind::Release, "1.0", None)
        .expect("release");
    git(p, &["switch", "-q", "develop"]);
    repo.flow_start(FlowKind::Release, "2.0", None).unwrap_err();

    git(p, &["tag", "v3.0"]);
    git(p, &["branch", "-D", "release/1.0"]);
    git(p, &["config", "gitflow.prefix.versiontag", "v"]);
    repo.flow_start(FlowKind::Hotfix, "3.0", None).unwrap_err();

    fs::write(p.join("a.txt"), "dirty\n").expect("write");
    repo.flow_start(FlowKind::Hotfix, "3.1", None).unwrap_err();
    // A feature may start with local changes, as `git switch -c` carries them.
    repo.flow_start(FlowKind::Feature, "z", None)
        .expect("feature with changes");
}

#[test]
fn flow_finish_refuses_a_conflict_and_changes_nothing() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    repo.flow_start(FlowKind::Feature, "clash", None)
        .expect("start");
    commit_file(p, "a.txt", "feature\n", "feat: clash");
    git(p, &["switch", "-q", "develop"]);
    commit_file(p, "a.txt", "develop\n", "feat: other");
    git(p, &["switch", "-q", "feature/clash"]);
    let before = shape(p);

    let err = repo
        .flow_finish(FlowKind::Feature, "clash", &FlowFinish::default())
        .expect_err("conflict");
    assert!(err.to_string().contains("a.txt"), "{err}");
    assert_eq!(shape(p), before);
    assert!(repo.snapshot(10).expect("snapshot").status.is_clean());
}

#[test]
fn release_finish_refuses_a_back_merge_conflict_before_merging() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    repo.flow_start(FlowKind::Hotfix, "1.0.1", None)
        .expect("start");
    commit_file(p, "dev.txt", "hotfix\n", "fix: touches dev.txt");
    let before = shape(p);

    // Merging into main is clean (main has no dev.txt); merging back into develop is not.
    let err = repo
        .flow_finish(FlowKind::Hotfix, "1.0.1", &FlowFinish::default())
        .expect_err("conflict");
    assert!(err.to_string().contains("dev.txt"), "{err}");
    assert_eq!(shape(p), before);
}

#[test]
fn flow_finish_refuses_a_dirty_tree() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    repo.flow_start(FlowKind::Feature, "x", None)
        .expect("start");
    commit_file(p, "x.txt", "x\n", "feat: x");
    fs::write(p.join("x.txt"), "changed\n").expect("write");
    let before = shape(p);
    repo.flow_finish(FlowKind::Feature, "x", &FlowFinish::default())
        .unwrap_err();
    git(p, &["add", "x.txt"]);
    repo.flow_finish(FlowKind::Feature, "x", &FlowFinish::default())
        .unwrap_err();
    assert_eq!(shape(p), before);
    // Untracked files do not block a finish, as in git-flow.
    git(p, &["restore", "--staged", "--worktree", "x.txt"]);
    fs::write(p.join("scratch.txt"), "notes\n").expect("write");
    repo.flow_finish(FlowKind::Feature, "x", &FlowFinish::default())
        .expect("finish with untracked file");
}

#[test]
fn flow_push_pushes_only_the_finished_refs() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    let remote = tempfile::tempdir().expect("tempdir");
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    let url = remote.path().to_string_lossy().into_owned();
    git(p, &["remote", "add", "origin", &url]);
    git(p, &["tag", "unrelated"]);
    repo.flow_start(FlowKind::Release, "2.0", None)
        .expect("start");
    commit_file(p, "VERSION", "2.0\n", "chore: 2.0");
    git(p, &["push", "-q", "origin", "release/2.0"]);
    repo.flow_finish(FlowKind::Release, "2.0", &FlowFinish::default())
        .expect("finish");

    repo.flow_push(FlowKind::Release, "2.0", "origin", true, |_| {})
        .expect("push");
    let refs = git(remote.path(), &["for-each-ref", "--format=%(refname)"]);
    let refs: Vec<&str> = refs.lines().collect();
    assert_eq!(
        refs,
        ["refs/heads/develop", "refs/heads/main", "refs/tags/v2.0"]
    );
    assert_eq!(
        git(remote.path(), &["rev-parse", "main"]),
        git(p, &["rev-parse", "main"])
    );
    repo.flow_push(FlowKind::Release, "2.0", "nope", false, |_| {})
        .unwrap_err();
}

#[test]
fn merged_branches_name_the_check_that_matched() {
    let (dir, repo) = fixture();
    let p = dir.path();
    let remote = tempfile::tempdir().expect("tempdir");
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    let url = remote.path().to_string_lossy().into_owned();
    git(p, &["remote", "add", "origin", &url]);

    // Merged with a merge commit: reachable.
    git(p, &["switch", "-q", "-c", "merged"]);
    commit_file(p, "m.txt", "m\n", "feat: m");
    git(p, &["switch", "-q", "main"]);
    git(p, &["merge", "-q", "--no-ff", "--no-edit", "merged"]);
    // Squash-merged: its change is in main under another commit.
    git(p, &["switch", "-q", "-c", "squashed"]);
    commit_file(p, "s.txt", "s\n", "feat: s1");
    commit_file(p, "s.txt", "s\ns\n", "feat: s2");
    git(p, &["switch", "-q", "main"]);
    git(p, &["merge", "-q", "--squash", "squashed"]);
    git(p, &["commit", "-q", "-m", "feat: s"]);
    // Upstream deleted on the remote and pruned.
    git(p, &["switch", "-q", "-c", "gone"]);
    commit_file(p, "g.txt", "g\n", "feat: g");
    git(p, &["push", "-q", "-u", "origin", "gone"]);
    git(remote.path(), &["branch", "-D", "gone"]);
    git(p, &["fetch", "-q", "--prune", "origin"]);
    // Not merged at all.
    git(p, &["switch", "-q", "-c", "open", "main"]);
    commit_file(p, "o.txt", "o\n", "feat: o");
    // Unrelated history, which merge-tree refuses: not merged, and no error for the rest.
    git(p, &["switch", "-q", "--orphan", "pages"]);
    commit_file(p, "index.html", "<p>\n", "docs: pages");
    git(p, &["switch", "-q", "main"]);

    let merged = repo.merged_branches("main", true).expect("merged");
    let reasons: Vec<(&str, Option<MergedReason>)> =
        merged.iter().map(|b| (b.name.as_str(), b.reason)).collect();
    assert_eq!(
        reasons,
        [
            ("gone", Some(MergedReason::UpstreamGone)),
            ("merged", Some(MergedReason::Reachable)),
            ("open", None),
            ("pages", None),
            ("squashed", Some(MergedReason::SquashMerged)),
        ]
    );

    let quick = repo.merged_branches("main", false).expect("merged");
    assert_eq!(
        quick
            .iter()
            .find(|b| b.name == "squashed")
            .map(|b| b.reason),
        Some(None)
    );
    let error = repo.merged_branches("nope", true).unwrap_err().to_string();
    assert!(
        error.contains("Could not resolve the base 'nope':"),
        "{error}"
    );
    assert!(
        error.contains("not found") || error.contains("revspec") || error.contains("reference"),
        "the underlying libgit2 error is preserved: {error}"
    );
}

#[test]
fn merged_branches_keep_the_cause_of_a_base_that_is_no_commit() {
    let (_dir, repo) = fixture();
    // A tree is found but is no commit: libgit2's own reason follows the base's name.
    let error = repo
        .merged_branches("HEAD^{tree}", false)
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("Could not resolve the base 'HEAD^{tree}': "),
        "{error}"
    );
    assert!(
        error.contains("can not be successfully peeled into a commit"),
        "libgit2's reason is preserved: {error}"
    );
}

#[test]
fn a_finish_with_rebase_matches_the_git_commands() {
    let (dir, repo, cli) = flow_twins();
    let (p, c) = (dir.path(), cli.path());
    let branch = repo
        .flow_start(FlowKind::Feature, "wide", None)
        .expect("start");
    git(c, &["switch", "-q", "-c", &branch, "develop"]);
    for d in [p, c] {
        commit_file(d, "one.txt", "1\n", "feat: one");
        commit_file(d, "two.txt", "2\n", "feat: two");
        git(d, &["switch", "-q", "develop"]);
        commit_file(d, "other.txt", "x\n", "feat: develop moved");
    }

    let options = FlowFinish {
        rebase: true,
        ..FlowFinish::default()
    };
    repo.flow_finish(FlowKind::Feature, "wide", &options)
        .expect("finish");
    git(c, &["switch", "-q", &branch]);
    git(c, &["rebase", "-q", "develop"]);
    git(c, &["switch", "-q", "develop"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", &branch]);
    git(c, &["branch", "-d", &branch]);

    assert_eq!(shape(p), shape(c));
    // The rebase made the history linear under the merge commit.
    assert_eq!(
        git(p, &["rev-list", "--merges", "--count", "develop"]).trim(),
        "1"
    );
}

#[test]
fn a_rebase_that_conflicts_refuses_and_changes_nothing() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    let branch = repo
        .flow_start(FlowKind::Feature, "clash", None)
        .expect("start");
    commit_file(p, "dev.txt", "feature\n", "feat: feature edit");
    commit_file(p, "extra.txt", "e\n", "feat: more");
    git(p, &["switch", "-q", "develop"]);
    commit_file(p, "dev.txt", "develop\n", "feat: develop edit");
    let before = shape(p);

    let options = FlowFinish {
        rebase: true,
        ..FlowFinish::default()
    };
    let error = repo
        .flow_finish(FlowKind::Feature, "clash", &options)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("would conflict in dev.txt") && error.contains("Nothing was changed"),
        "{error}"
    );
    assert_eq!(shape(p), before);
    assert_eq!(git(p, &["status", "--porcelain"]).trim(), "");
    assert_eq!(
        git(p, &["rev-parse", "--verify", "-q", &branch])
            .trim()
            .len(),
        40,
        "the branch is still there"
    );
}

#[test]
fn a_rebase_is_refused_for_a_release() {
    let (_dir, repo, _cli) = flow_twins();
    repo.flow_start(FlowKind::Release, "1.0", None)
        .expect("start");
    let options = FlowFinish {
        rebase: true,
        ..FlowFinish::default()
    };
    let error = repo
        .flow_finish(FlowKind::Release, "1.0", &options)
        .unwrap_err()
        .to_string();
    assert!(error.contains("feature or bugfix"), "{error}");
}

#[test]
fn a_hotfix_finishes_into_the_open_release_instead_of_develop() {
    let (dir, repo, cli) = flow_twins();
    let (p, c) = (dir.path(), cli.path());
    repo.flow_start(FlowKind::Release, "1.1", None)
        .expect("start");
    git(c, &["switch", "-q", "-c", "release/1.1", "develop"]);
    for d in [p, c] {
        commit_file(d, "VERSION", "1.1\n", "chore: bump version");
        git(d, &["switch", "-q", "main"]);
    }
    repo.flow_start(FlowKind::Hotfix, "1.0.1", None)
        .expect("start");
    git(c, &["switch", "-q", "-c", "hotfix/1.0.1", "main"]);
    for d in [p, c] {
        commit_file(d, "a.txt", "one\ntwo\nthree\nfixed\n", "fix: urgent");
    }
    let config = repo.flow_config().expect("config").expect("initialised");
    assert_eq!(
        repo.flow_back_merge_target(&config, FlowKind::Hotfix)
            .expect("target"),
        "release/1.1"
    );
    assert_eq!(
        repo.flow_back_merge_target(&config, FlowKind::Release)
            .expect("target"),
        "develop"
    );

    let options = FlowFinish {
        tag: false,
        ..FlowFinish::default()
    };
    let message = repo
        .flow_finish(FlowKind::Hotfix, "1.0.1", &options)
        .expect("finish");
    git(c, &["switch", "-q", "main"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", "hotfix/1.0.1"]);
    git(c, &["switch", "-q", "release/1.1"]);
    git(c, &["merge", "--no-ff", "--no-edit", "-q", "main"]);
    git(c, &["branch", "-d", "hotfix/1.0.1"]);
    git(c, &["switch", "-q", "develop"]);

    assert!(message.contains("release/1.1"), "{message}");
    assert_eq!(shape(p), shape(c));
    // develop did not get the fix; the release did.
    assert_eq!(
        git(p, &["rev-list", "--count", "develop..release/1.1"]).trim(),
        "4"
    );
    assert_eq!(
        git(p, &["rev-list", "--count", "release/1.1..develop"]).trim(),
        "0"
    );
}

#[test]
fn a_back_merge_conflict_with_the_open_release_refuses_before_merging() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    repo.flow_start(FlowKind::Release, "1.1", None)
        .expect("start");
    commit_file(p, "a.txt", "release\n", "chore: release edit");
    git(p, &["switch", "-q", "main"]);
    repo.flow_start(FlowKind::Hotfix, "1.0.1", None)
        .expect("start");
    commit_file(p, "a.txt", "hotfix\n", "fix: urgent");
    let before = shape(p);
    let error = repo
        .flow_finish(FlowKind::Hotfix, "1.0.1", &FlowFinish::default())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("back into release/1.1 would conflict in a.txt"),
        "{error}"
    );
    assert_eq!(shape(p), before);
}

#[test]
fn finishing_needs_git_2_38() {
    assert!(flow_finish_requirement(Some((2, 37))).is_some_and(|r| r.contains("Git 2.38")));
    assert!(flow_finish_requirement(None).is_some());
    assert_eq!(flow_finish_requirement(Some((2, 38))), None);
    assert_eq!(flow_finish_requirement(Some((3, 0))), None);
}

/// The bases the cleanup fixture's branches are compared with.
fn bases() -> Vec<String> {
    vec!["main".to_owned(), "origin/main".to_owned()]
}

/// `main` with merged, squash-merged, unmerged and checked-out branches, and an `origin` whose
/// default branch is `main`.
fn cleanup_fixture() -> (tempfile::TempDir, Repo, tempfile::TempDir) {
    let (dir, repo) = fixture();
    let p = dir.path();
    let remote = tempfile::tempdir().expect("tempdir");
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(
        p,
        &["remote", "add", "origin", &remote.path().to_string_lossy()],
    );
    git(p, &["push", "-q", "-u", "origin", "main"]);
    git(p, &["remote", "set-head", "origin", "main"]);
    git(p, &["switch", "-q", "-c", "feature/merged"]);
    commit_file(p, "m.txt", "m\n", "feat: m");
    git(p, &["switch", "-q", "main"]);
    git(
        p,
        &["merge", "-q", "--no-ff", "--no-edit", "feature/merged"],
    );
    git(p, &["switch", "-q", "-c", "feature/squashed"]);
    commit_file(p, "s.txt", "s\n", "feat: s1");
    commit_file(p, "s.txt", "s\ns\n", "feat: s2");
    git(p, &["switch", "-q", "main"]);
    git(p, &["merge", "-q", "--squash", "feature/squashed"]);
    git(p, &["commit", "-q", "-m", "feat: s"]);
    git(p, &["switch", "-q", "-c", "feature/open", "main"]);
    commit_file(p, "o.txt", "o\n", "feat: o");
    git(p, &["push", "-q", "origin", "main"]);
    git(p, &["switch", "-q", "main"]);
    (dir, repo, remote)
}

#[test]
fn the_base_is_the_remote_default_branch_and_long_lived_branches_are_protected() {
    let (_dir, repo, _remote) = cleanup_fixture();
    let base = repo.branch_base().expect("base").expect("some");
    assert_eq!(base.references, ["main", "origin/main"]);
    assert!(base.protected.contains(&"main".to_owned()));
    assert!(!base.protected.contains(&"feature/merged".to_owned()));
}

#[test]
fn the_base_is_develop_in_a_git_flow_repository() {
    let (_dir, repo, _cli) = flow_twins();
    let base = repo.branch_base().expect("base").expect("some");
    assert_eq!(base.references, ["develop"]);
    assert!(base.protected.contains(&"main".to_owned()));
    assert!(base.protected.contains(&"develop".to_owned()));
}

#[test]
fn a_repository_without_a_default_branch_has_no_base() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "trunk"]);
    let repo = Repo::open(dir.path()).expect("open");
    assert_eq!(repo.branch_base().expect("base"), None);
}

#[test]
fn removal_deletes_proven_branches_with_d_and_heuristic_ones_only_when_allowed() {
    let (dir, repo, _remote) = cleanup_fixture();
    let p = dir.path();
    let names = |list: &[&str]| list.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();

    // A heuristic match without the user's confirmation is refused before anything is deleted.
    let error = repo
        .remove_merged_branches(
            &bases(),
            &names(&["feature/merged", "feature/squashed"]),
            false,
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("feature/squashed") && error.contains("confirmation"),
        "{error}"
    );
    assert!(git(p, &["branch", "--list", "feature/merged"]).contains("feature/merged"));

    // Not merged, the base itself, a name that is no branch and the checked-out branch are
    // refused.
    git(p, &["branch", "develop", "main"]);
    for (name, why) in [
        ("feature/open", "no longer merged"),
        ("main", "not a local branch"),
        ("nope", "not a local branch"),
        ("develop", "long-lived"),
    ] {
        let error = repo
            .remove_merged_branches(&bases(), &names(&[name]), true)
            .unwrap_err()
            .to_string();
        assert!(error.contains(why), "{name}: {error}");
    }
    git(p, &["switch", "-q", "feature/merged"]);
    let error = repo
        .remove_merged_branches(&bases(), &names(&["feature/merged"]), true)
        .unwrap_err()
        .to_string();
    assert!(error.contains("checked out"), "{error}");
    git(p, &["switch", "-q", "feature/open"]);

    let done = repo
        .remove_merged_branches(
            &bases(),
            &names(&["feature/merged", "feature/squashed"]),
            true,
        )
        .expect("remove");
    assert_eq!(done.removed, ["feature/merged", "feature/squashed"]);
    assert_eq!(done.failed, Vec::<(String, String)>::new());
    let left = git(p, &["branch", "--format=%(refname:short)"]);
    assert_eq!(
        left.lines().collect::<Vec<_>>(),
        ["develop", "feature/open", "main"]
    );
}

#[test]
fn a_merge_made_on_the_server_counts_though_the_local_base_is_behind() {
    let (dir, repo, _remote) = cleanup_fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "on-server", "main"]);
    commit_file(p, "srv.txt", "s\n", "feat: merged on the server");
    git(p, &["push", "-q", "origin", "on-server:main"]);
    git(p, &["switch", "-q", "main"]);
    let only_local = repo.merged_branches("main", false).expect("merged");
    let reason = |rows: &[canopy_core::MergedBranch]| {
        rows.iter()
            .find(|b| b.name == "on-server")
            .map(|b| b.reason)
    };
    assert_eq!(reason(&only_local), Some(None));
    let both = repo.merged_branches_among(&bases(), false).expect("merged");
    assert_eq!(reason(&both), Some(Some(MergedReason::Reachable)));
    repo.merged_branches_among(&[], false).unwrap_err();
}

#[test]
fn git_refusing_one_branch_is_reported_and_the_others_still_go() {
    let (dir, repo, _remote) = cleanup_fixture();
    let p = dir.path();
    // `branch -d` compares with HEAD when a branch has no upstream: from a branch that lacks the
    // merged work it refuses, though the work is in the base. `feature/squashed` has no upstream
    // either and only looks merged, so `-D` removes it regardless.
    let root = git(p, &["rev-list", "--max-parents=0", "main"]);
    git(p, &["switch", "-q", "-c", "side", root.trim()]);
    let done = repo
        .remove_merged_branches(
            &bases(),
            &["feature/merged".to_owned(), "feature/squashed".to_owned()],
            true,
        )
        .expect("remove");
    assert_eq!(done.removed, ["feature/squashed"]);
    assert_eq!(done.failed.len(), 1);
    assert_eq!(done.failed[0].0, "feature/merged");
    assert!(
        done.failed[0].1.contains("not fully merged"),
        "{:?}",
        done.failed
    );
    let summary = done.summary();
    assert!(
        summary.starts_with("Deleted 1 branch\nCould not delete feature/merged: "),
        "{summary}"
    );
}

#[test]
fn a_branch_updates_from_the_base_its_kind_or_remote_names() {
    let (_dir, repo, _remote) = cleanup_fixture();
    assert_eq!(
        repo.update_base("feature/open").expect("base"),
        Some("origin/main".to_owned())
    );
    // The base itself is a pull's business.
    assert_eq!(repo.update_base("main").expect("base"), None);

    let (_dir, flow, _cli) = flow_twins();
    flow.flow_start(FlowKind::Feature, "x", None)
        .expect("start");
    assert_eq!(
        flow.update_base("feature/x").expect("base"),
        Some("develop".to_owned())
    );
    assert_eq!(
        flow.update_base("hotfix/y").expect("base"),
        Some("main".to_owned())
    );
}

#[test]
fn updating_from_the_base_merges_or_rebases_what_the_base_gained() {
    for strategy in [PullStrategy::Merge, PullStrategy::Rebase] {
        let (dir, repo, _remote) = cleanup_fixture();
        let p = dir.path();
        git(p, &["switch", "-q", "feature/open"]);
        commit_file(p, "mine.txt", "mine\n", "feat: mine");
        git(p, &["switch", "-q", "main"]);
        commit_file(p, "base.txt", "base\n", "feat: base moved");
        git(p, &["push", "-q", "origin", "main"]);
        git(p, &["switch", "-q", "feature/open"]);

        let preview = repo.compare_base("origin/main").expect("preview");
        assert_eq!((preview.ahead, preview.behind), (2, 1), "{preview:?}");
        assert_eq!(preview.conflicts, Some(Vec::new()));
        repo.update_from_base(strategy, preview.tip.expect("tip"), |_| {})
            .expect("update");

        assert_eq!(
            git(p, &["rev-list", "--count", "HEAD..origin/main"]).trim(),
            "0"
        );
        assert_eq!(
            git(p, &["rev-list", "--count", "origin/main..HEAD"]).trim(),
            if strategy == PullStrategy::Merge {
                "3"
            } else {
                "2"
            }
        );
        let merges = git(p, &["rev-list", "--merges", "--count", "origin/main..HEAD"]);
        assert_eq!(
            merges.trim(),
            if strategy == PullStrategy::Merge {
                "1"
            } else {
                "0"
            }
        );
        assert!(p.join("base.txt").exists() && p.join("mine.txt").exists());
    }
}

#[test]
fn updating_from_the_base_fast_forwards_a_branch_that_only_fell_behind() {
    let (dir, repo, _remote) = cleanup_fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "behind", "main~1"]);
    git(p, &["reset", "-q", "--hard", "main"]);
    git(p, &["switch", "-q", "main"]);
    commit_file(p, "base.txt", "base\n", "feat: base moved");
    git(p, &["push", "-q", "origin", "main"]);
    git(p, &["switch", "-q", "behind"]);
    let preview = repo.compare_base("origin/main").expect("preview");
    assert_eq!((preview.ahead, preview.behind), (0, 1));
    repo.update_from_base(
        PullStrategy::FastForwardOnly,
        preview.tip.expect("tip"),
        |_| {},
    )
    .expect("update");
    assert_eq!(
        git(p, &["rev-parse", "HEAD"]),
        git(p, &["rev-parse", "origin/main"])
    );
}

#[test]
fn a_fast_forward_only_update_refuses_diverged_branches() {
    let (dir, repo, _remote) = cleanup_fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "main"]);
    commit_file(p, "base.txt", "base\n", "feat: base moved");
    git(p, &["push", "-q", "origin", "main"]);
    git(p, &["switch", "-q", "feature/open"]);
    let preview = repo.compare_base("origin/main").expect("preview");
    assert!(preview.diverged());
    let before = git(p, &["rev-parse", "HEAD"]);
    repo.update_from_base(
        PullStrategy::FastForwardOnly,
        preview.tip.expect("tip"),
        |_| {},
    )
    .unwrap_err();
    assert_eq!(git(p, &["rev-parse", "HEAD"]), before);
}

#[test]
fn comparing_with_an_unknown_base_names_it() {
    let (_dir, repo, _remote) = cleanup_fixture();
    let error = repo.compare_base("origin/nope").unwrap_err().to_string();
    assert!(
        error.contains("Could not resolve the base 'origin/nope'"),
        "{error}"
    );
    repo.compare_base("--force").unwrap_err();
}

#[test]
fn a_snapshot_carries_the_feature_branch_facts() {
    let (_dir, flow, _cli) = flow_twins();
    flow.flow_start(FlowKind::Feature, "x", None)
        .expect("start");
    let snap = flow.snapshot(10).expect("snapshot");
    assert_eq!(
        snap.workflow.flow.as_ref().map(|f| f.develop.as_str()),
        Some("develop")
    );
    assert_eq!(snap.workflow.head_base.as_deref(), Some("develop"));
    assert_eq!(
        snap.workflow
            .base
            .as_ref()
            .map(canopy_core::BranchBase::shown),
        Some("develop")
    );
}

const POINTER_OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

fn pointer_text(size: u64) -> String {
    format!("version https://git-lfs.github.com/spec/v1\noid sha256:{POINTER_OID}\nsize {size}\n")
}

/// A repository whose `*.bin` files use LFS, with `big.bin` committed as a bare pointer (as after
/// a clone with `GIT_LFS_SKIP_SMUDGE=1`). The filter driver is turned off so the test does not
/// depend on git-lfs being installed or configured globally.
fn lfs_fixture() -> (tempfile::TempDir, Repo) {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["config", "filter.lfs.clean", "cat"]);
    git(p, &["config", "filter.lfs.smudge", "cat"]);
    git(p, &["config", "filter.lfs.process", ""]);
    git(p, &["config", "filter.lfs.required", "false"]);
    fs::write(
        p.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    )
    .expect("write");
    fs::write(p.join("big.bin"), pointer_text(12345)).expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: lfs"]);
    (dir, repo)
}

#[test]
fn status_marks_lfs_entries() {
    let (dir, repo) = lfs_fixture();
    let p = dir.path();
    fs::write(p.join("big.bin"), pointer_text(99)).expect("write");
    fs::write(p.join("new.bin"), "x").expect("write");
    fs::write(p.join("a.txt"), "changed\n").expect("write");

    let status = repo.snapshot(10).expect("snapshot").status;
    let lfs: Vec<(&str, bool)> = status
        .unstaged
        .iter()
        .map(|f| (f.path.as_str(), f.lfs))
        .collect();
    assert_eq!(
        lfs,
        [("a.txt", false), ("big.bin", true), ("new.bin", true)]
    );
    let head = repo.head().expect("head").oid.expect("commit");
    let details = repo
        .commit_details(head, MissingObjects::Fail)
        .expect("details");
    assert!(details.files.iter().any(|f| f.path == "big.bin" && f.lfs));
    assert!(
        details
            .files
            .iter()
            .any(|f| f.path == ".gitattributes" && !f.lfs)
    );
}

/// That `actual` is `folder` joined with `rest`, where `folder` exists. On Windows the work folder
/// comes back from libgit2 with `/` and long names, the temporary folder in its 8.3 short form.
#[track_caller]
fn assert_inside(actual: &Path, folder: &Path, rest: &Path) {
    let base = actual
        .ancestors()
        .nth(rest.components().count())
        .expect("deep enough");
    assert!(
        actual.ends_with(rest)
            && fs::canonicalize(base).expect("canonical")
                == fs::canonicalize(folder).expect("canonical"),
        "{} is not {}",
        actual.display(),
        folder.join(rest).display()
    );
}

#[test]
fn lfs_files_report_whether_objects_are_downloaded() {
    let (dir, repo) = lfs_fixture();
    let p = dir.path();
    let files = repo.lfs_files().expect("lfs files");
    assert_eq!(files.len(), 1);
    // A pointer staged after the first read is seen by the same `Repo`.
    fs::write(p.join("more.bin"), pointer_text(7)).expect("write");
    git(p, &["add", "more.bin"]);
    let paths: Vec<String> = repo
        .lfs_files()
        .expect("lfs files")
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(paths, ["big.bin", "more.bin"]);
    git(p, &["rm", "-q", "--cached", "more.bin"]);
    assert_eq!(files[0].path, "big.bin");
    assert_eq!(files[0].pointer.oid, POINTER_OID);
    assert_eq!(files[0].pointer.size, 12345);
    assert!(!files[0].downloaded);

    let object = repo.lfs_object_path(POINTER_OID).expect("object path");
    let in_store = Path::new("objects").join("4d").join("7a").join(POINTER_OID);
    assert_inside(&object, &p.join(".git"), &Path::new("lfs").join(&in_store));
    fs::create_dir_all(object.parent().expect("parent")).expect("mkdir");
    fs::write(&object, "content").expect("write");
    assert!(repo.lfs_files().expect("lfs files")[0].downloaded);

    // A relative `lfs.storage` moves the store, relative to the git directory.
    git(p, &["config", "lfs.storage", "elsewhere"]);
    assert!(!repo.lfs_files().expect("lfs files")[0].downloaded);
    assert_inside(
        &repo.lfs_object_path(POINTER_OID).expect("path"),
        &p.join(".git"),
        &Path::new("elsewhere").join(&in_store),
    );
    assert_eq!(repo.lfs_object_path("../../../../etc/passwd"), None);
    assert_eq!(repo.lfs_object_path(&POINTER_OID.to_uppercase()), None);
}

/// A superproject with `deps/lib` as a submodule of a local repository with two commits; the
/// superproject records the first.
fn submodule_fixture() -> (tempfile::TempDir, Repo, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let lib = dir.path().join("lib");
    let sup = dir.path().join("super");
    fs::create_dir_all(&lib).expect("mkdir");
    fs::create_dir_all(&sup).expect("mkdir");
    init(&lib);
    commit_file(&lib, "b.txt", "b\n", "feat: second");
    init(&sup);
    let url = lib.to_string_lossy().into_owned();
    git(
        &sup,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            "--",
            &url,
            "deps/lib",
        ],
    );
    let sub = sup.join("deps").join("lib");
    git(&sub, &["checkout", "-q", "HEAD~1"]);
    git(&sup, &["add", "deps/lib"]);
    git(&sup, &["commit", "-q", "-m", "feat: add lib"]);
    let repo = Repo::open(&sup).expect("open");
    (dir, repo, sub)
}

#[test]
fn submodule_status_reads_ids_and_states() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let list = repo.submodules(true).expect("submodules");
    assert_eq!(list.len(), 1);
    let info = &list[0];
    assert_eq!(info.path, "deps/lib");
    assert_eq!(info.name, "deps/lib");
    assert_eq!(info.head_id, info.index_id);
    assert_eq!(info.workdir_id, info.index_id);
    assert_eq!(info.dirty, Some(false));
    assert_eq!(info.state(), SubmoduleState::Clean);
    assert!(
        repo.submodules(false).expect("submodules")[0]
            .dirty
            .is_none()
    );

    fs::write(sub.join("a.txt"), "edited\n").expect("write");
    assert!(repo.submodule_dirty("deps/lib").expect("dirty"));
    assert_eq!(
        repo.submodules(true).expect("submodules")[0].state(),
        SubmoduleState::Dirty
    );
    git(&sub, &["checkout", "-q", "--", "a.txt"]);

    git(&sub, &["checkout", "-q", "main"]);
    let info = &repo.submodules(false).expect("submodules")[0];
    assert!(info.has_new_commits());
    assert!(!info.is_staged());
    assert_eq!(info.state(), SubmoduleState::NewCommits);
    git(&sup, &["add", "deps/lib"]);
    assert!(repo.submodules(false).expect("submodules")[0].is_staged());
    git(&sup, &["reset", "-q", "deps/lib"]);

    git(&sup, &["submodule", "deinit", "-q", "-f", "--", "deps/lib"]);
    assert_eq!(
        repo.submodules(false).expect("submodules")[0].state(),
        SubmoduleState::NotInitialized
    );
}

#[test]
fn submodule_update_reactivates_and_checks_out_the_recorded_commit() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let recorded = git(&sup, &["rev-parse", "HEAD:deps/lib"]);
    git(&sub, &["checkout", "-q", "main"]);
    repo.submodule_update(Some("deps/lib"), false, false, |_| {})
        .expect("update");
    assert_eq!(git(&sub, &["rev-parse", "HEAD"]), recorded);

    git(&sup, &["submodule", "deinit", "-q", "-f", "--", "deps/lib"]);
    // The repository under .git/modules is reused, so no clone runs.
    repo.submodule_update(None, true, true, |_| {})
        .expect("update --init");
    assert_eq!(git(&sub, &["rev-parse", "HEAD"]), recorded);
    assert_eq!(
        repo.submodules(true).expect("submodules")[0].state(),
        SubmoduleState::Clean
    );
}

#[test]
fn submodule_sync_copies_the_url_into_the_configuration() {
    let (dir, repo, _sub) = submodule_fixture();
    let sup = dir.path().join("super");
    git(
        &sup,
        &[
            "config",
            "-f",
            ".gitmodules",
            "submodule.deps/lib.url",
            "https://example.com/lib.git",
        ],
    );
    repo.submodule_sync(None, true).expect("sync");
    assert_eq!(
        git(&sup, &["config", "submodule.deps/lib.url"]).trim(),
        "https://example.com/lib.git"
    );
}

#[test]
fn submodule_add_adopts_a_repository_already_in_place() {
    let (dir, repo, _sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let lib = dir.path().join("lib");
    let url = lib.to_string_lossy().into_owned();
    git(
        dir.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "clone",
            "-q",
            &url,
            "super/other",
        ],
    );
    repo.submodule_add(&url, "other", None, |_| {})
        .expect("add");
    assert_eq!(
        git(
            &sup,
            &["config", "-f", ".gitmodules", "submodule.other.path"]
        )
        .trim(),
        "other"
    );
    let staged = git(&sup, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.lines().collect::<Vec<_>>(), [".gitmodules", "other"]);
    repo.submodule_add("", "x", None, |_| {}).unwrap_err();
}

#[test]
fn following_submodules_checks_out_the_recorded_commit_unless_told_not_to() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let recorded = git(&sup, &["rev-parse", "HEAD:deps/lib"]);
    git(&sub, &["checkout", "-q", "main"]);

    repo.set_options(GitOptions {
        recurse_submodules: false,
        ..GitOptions::default()
    });
    repo.follow_submodules(|_| {}).expect("off");
    assert_ne!(git(&sub, &["rev-parse", "HEAD"]), recorded);

    repo.set_options(GitOptions::default());
    // Where the git configuration sets submodule.recurse, git's own commands decide.
    git(&sup, &["config", "submodule.recurse", "false"]);
    repo.follow_submodules(|_| {}).expect("decided by git");
    assert_ne!(git(&sub, &["rev-parse", "HEAD"]), recorded);
    // ...and git's own value applies even where Canopy's choice is off.
    git(&sup, &["config", "submodule.recurse", "true"]);
    repo.set_options(GitOptions {
        recurse_submodules: false,
        ..GitOptions::default()
    });
    repo.follow_submodules(|_| {}).expect("configured on");
    assert_eq!(git(&sub, &["rev-parse", "HEAD"]), recorded);
    git(&sub, &["checkout", "-q", "main"]);
    git(&sup, &["config", "--unset", "submodule.recurse"]);
    repo.set_options(GitOptions::default());

    repo.follow_submodules(|_| {}).expect("on");
    assert_eq!(git(&sub, &["rev-parse", "HEAD"]), recorded);
}

#[test]
fn following_submodules_leaves_a_deinitialised_one_alone() {
    let (dir, repo, _sub) = submodule_fixture();
    let sup = dir.path().join("super");
    git(&sup, &["submodule", "deinit", "-q", "-f", "deps/lib"]);
    // A leading "-" in `git submodule status` marks a submodule that is not initialised.
    assert!(git(&sup, &["submodule", "status"]).starts_with('-'));

    repo.follow_submodules(|_| {}).expect("follow");

    assert!(
        git(&sup, &["submodule", "status"]).starts_with('-'),
        "following a pull or checkout must not initialise it again"
    );
    assert_eq!(
        fs::read_dir(sup.join("deps/lib")).expect("folder").count(),
        0,
        "and must not clone it"
    );
}

#[test]
fn a_checkout_moves_submodules_only_as_canopy_or_a_followed_configuration_says() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let first = git(&sup, &["rev-parse", "HEAD:deps/lib"]);
    git(&sup, &["switch", "-q", "-c", "advanced"]);
    git(&sub, &["checkout", "-q", "main"]);
    git(&sup, &["add", "deps/lib"]);
    git(&sup, &["commit", "-q", "-m", "chore: advance lib"]);
    git(&sup, &["switch", "-q", "-"]);
    git(&sub, &["checkout", "-q", first.trim()]);
    git(&sup, &["config", "submodule.recurse", "true"]);

    // Canopy does not follow the configuration and its own choice is off: nothing moves.
    repo.set_options(GitOptions {
        follow_config: false,
        recurse_submodules: false,
        ..GitOptions::default()
    });
    repo.checkout_branch("advanced").expect("checkout");
    assert_eq!(git(&sub, &["rev-parse", "HEAD"]), first);

    // Following the configuration, git's value decides: it recurses (the worker follows every
    // checkout with `follow_submodules`).
    repo.set_options(GitOptions::default());
    repo.checkout_branch("main").expect("checkout back");
    repo.follow_submodules(|_| {}).expect("follow");
    repo.checkout_branch("advanced").expect("checkout");
    repo.follow_submodules(|_| {}).expect("follow");
    assert_ne!(git(&sub, &["rev-parse", "HEAD"]), first);
}

/// `git switch --recurse-submodules`, which `submodule.recurse = true` turns on, drops an edit
/// inside a submodule to a file both submodule commits share, without a word (git 2.56).
/// `git submodule update` keeps it, so Canopy moves submodules only that way.
#[test]
fn a_checkout_keeps_uncommitted_edits_inside_a_submodule_when_git_recurses() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let first = git(&sup, &["rev-parse", "HEAD:deps/lib"]);
    git(&sup, &["switch", "-q", "-c", "advanced"]);
    git(&sub, &["checkout", "-q", "main"]);
    git(&sup, &["add", "deps/lib"]);
    git(&sup, &["commit", "-q", "-m", "chore: advance lib"]);
    git(&sup, &["config", "submodule.recurse", "true"]);
    // a.txt is the same in both submodule commits.
    fs::write(sub.join("a.txt"), "edited\n").expect("edit");

    repo.checkout_branch("main").expect("checkout");
    repo.follow_submodules(|_| {}).expect("follow");

    assert_eq!(git(&sub, &["rev-parse", "HEAD"]), first);
    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).expect("read"),
        "edited\n",
        "the edit inside the submodule must survive the checkout"
    );
}

#[test]
fn a_snapshot_lists_the_submodules() {
    let (_dir, repo, _sub) = submodule_fixture();
    let snap = repo.snapshot(10).expect("snapshot");
    let paths: Vec<_> = snap.submodules.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, ["deps/lib"]);
    assert!(snap.submodules[0].dirty.is_none());
}

#[test]
fn a_submodule_needs_a_plain_relative_folder_and_a_url_without_credentials() {
    let ok = "https://example.com/lib.git";
    assert_eq!(submodule_add_problem(ok, "deps/lib"), None);
    assert_eq!(
        submodule_add_problem("git@example.com:o/lib.git", "lib"),
        None
    );
    for url in [
        "",
        "  ",
        "-oProxyCommand=x",
        "ext::sh -c id",
        "https://token@example.com/lib.git",
    ] {
        assert!(submodule_add_problem(url, "lib").is_some(), "{url}");
    }
    for path in [
        "",
        " ",
        "-x",
        "/abs",
        "../up",
        "a/../../up",
        "./",
        ".git/hooks",
        "a/.GIT/x",
        "C:\\x",
        "a\\b",
        "c:d",
    ] {
        assert!(submodule_add_problem(ok, path).is_some(), "{path:?}");
    }
}

#[test]
fn submodule_add_refuses_an_unsafe_folder_before_running_git() {
    let (dir, repo, _sub) = submodule_fixture();
    let url = dir.path().join("lib").to_string_lossy().into_owned();
    let sup = dir.path().join("super");
    repo.submodule_add(&url, "../escape", None, |_| {})
        .unwrap_err();
    assert!(!dir.path().join("escape").exists());
    assert_eq!(git(&sup, &["status", "--porcelain"]), "");
}

#[test]
fn submodule_remove_deinits_removes_and_cleans_the_module_directory() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let modules = sup.join(".git").join("modules").join("deps").join("lib");
    assert!(modules.is_dir());
    repo.submodule_remove("deps/lib").expect("remove");
    assert!(!sub.exists());
    assert!(!modules.exists());
    assert_eq!(git(&sup, &["ls-files", "--stage", "deps/lib"]), "");
    let gitmodules = fs::read_to_string(sup.join(".gitmodules")).unwrap_or_default();
    assert!(!gitmodules.contains("deps/lib"), "{gitmodules}");
    // The removal is staged: HEAD still records the submodule, the index no longer does.
    let left = repo.submodules(false).expect("submodules");
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(left[0].head_id.is_some() && left[0].index_id.is_none());
    assert!(left[0].is_staged());
    git(&sup, &["commit", "-q", "-m", "chore: drop lib"]);
    let submodules = repo.submodules(false).expect("submodules");
    assert!(submodules.is_empty(), "{submodules:?}");
    assert!(repo.submodule_remove("deps/lib").is_err());
}

#[test]
fn submodule_diff_gives_both_commits_and_whether_they_exist_locally() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let old = git(&sup, &["rev-parse", "HEAD:deps/lib"]);
    git(&sub, &["checkout", "-q", "main"]);
    let new = git(&sub, &["rev-parse", "HEAD"]);

    let diff = repo
        .file_diff(
            &DiffSource::Unstaged,
            "deps/lib",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let change = diff.submodule.expect("submodule change");
    assert_eq!(change.old.map(|o| o.to_string()), Some(old.trim().into()));
    assert_eq!(change.new.map(|o| o.to_string()), Some(new.trim().into()));
    assert!(change.old_present && change.new_present);
    let added: Vec<_> = change
        .log
        .added
        .iter()
        .map(|c| c.summary.as_str())
        .collect();
    assert_eq!(added, ["feat: second"]);
    assert_eq!(change.log.removed.len(), 0);
    assert!(!change.log.truncated);
    // Moving back lists the commits the new pointer leaves out.
    git(&sup, &["add", "deps/lib"]);
    git(&sub, &["checkout", "-q", "HEAD~1"]);
    let back = repo
        .file_diff(
            &DiffSource::Unstaged,
            "deps/lib",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff")
        .submodule
        .expect("submodule change");
    assert_eq!(back.log.added.len(), 0);
    let removed: Vec<_> = back
        .log
        .removed
        .iter()
        .map(|c| c.summary.as_str())
        .collect();
    assert_eq!(removed, ["feat: second"]);
    git(&sup, &["reset", "-q", "deps/lib"]);
    git(&sub, &["checkout", "-q", "main"]);

    // A recorded commit the submodule never fetched.
    let missing = "1111111111111111111111111111111111111111";
    git(
        &sup,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{missing},deps/lib"),
        ],
    );
    let staged = repo
        .file_diff(
            &DiffSource::Staged,
            "deps/lib",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    let change = staged.submodule.expect("submodule change");
    assert_eq!(change.new.map(|o| o.to_string()).as_deref(), Some(missing));
    assert!(change.old_present);
    assert!(!change.new_present);

    let plain = repo
        .file_diff(
            &DiffSource::Unstaged,
            ".gitmodules",
            None,
            3,
            Whitespace::Show,
            MissingObjects::Fail,
        )
        .expect("diff");
    assert!(plain.submodule.is_none());
}

#[test]
fn submodule_conflict_is_reported() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    // Two commits that do not contain each other, so git cannot fast-forward the pointer.
    git(&sub, &["checkout", "-q", "main"]);
    commit_file(&sub, "c.txt", "c\n", "feat: third");
    git(&sub, &["switch", "-q", "-c", "side", "main~2"]);
    commit_file(&sub, "side.txt", "side\n", "feat: side");
    git(&sup, &["switch", "-q", "-c", "other"]);
    git(&sup, &["commit", "-q", "-am", "feat: lib at side"]);
    git(&sup, &["switch", "-q", "main"]);
    git(&sub, &["checkout", "-q", "main"]);
    git(&sup, &["commit", "-q", "-am", "feat: lib at third"]);
    let merge = git_output(&sup, &["merge", "--no-edit", "other"]);
    assert!(!merge.status.success(), "the merge should conflict");

    let info = &repo.submodules(false).expect("submodules")[0];
    assert!(info.conflicted);
    assert_eq!(info.state(), SubmoduleState::Conflicted);
    let status = repo.snapshot(10).expect("snapshot").status;
    assert!(
        status
            .conflicted
            .iter()
            .any(|f| f.path == "deps/lib" && f.kind == ChangeKind::Conflicted)
    );
}

#[test]
fn submodule_remove_refuses_a_name_that_holds_another_submodules_repository() {
    let (dir, repo) = fixture();
    let p = dir.path();
    // Git refuses to create nested module directories itself; a hand-written `.gitmodules` can
    // still name them.
    let head = git(p, &["rev-parse", "HEAD"]);
    fs::write(
        p.join(".gitmodules"),
        "[submodule \"deps\"]\n\tpath = a\n\turl = ./x\n\
         [submodule \"deps/lib\"]\n\tpath = b\n\turl = ./y\n",
    )
    .expect("write");
    for path in ["a", "b"] {
        git(
            p,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},{path}", head.trim()),
            ],
        );
    }
    git(p, &["add", ".gitmodules"]);
    git(p, &["commit", "-q", "-m", "feat: crafted submodules"]);
    let before = git(p, &["ls-files", "--stage"]);

    let err = repo.submodule_remove("a").expect_err("refused");
    assert!(err.to_string().contains("deps/lib"), "{err}");
    assert_eq!(git(p, &["ls-files", "--stage"]), before);
}

#[test]
fn submodule_remove_cleans_the_directory_of_its_name_not_its_path() {
    let (dir, repo, _sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let url = dir.path().join("lib").to_string_lossy().into_owned();
    git(
        &sup,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            "--name",
            "renamed",
            "--",
            &url,
            "vendor/lib",
        ],
    );
    git(&sup, &["commit", "-q", "-m", "feat: vendor lib"]);
    let modules = sup.join(".git").join("modules");
    assert!(modules.join("renamed").is_dir());

    fs::write(sup.join("vendor").join("lib").join("a.txt"), "edited\n").expect("write");
    assert!(repo.submodule_dirty("vendor/lib").expect("dirty"));
    git(
        &sup.join("vendor").join("lib"),
        &["checkout", "-q", "--", "a.txt"],
    );

    repo.submodule_remove("vendor/lib").expect("remove");
    assert!(!modules.join("renamed").exists());
    // The other submodule is untouched.
    assert!(modules.join("deps").join("lib").is_dir());
    assert!(repo.submodule_remove("not/a/submodule").is_err());
}

#[test]
fn flow_refuses_a_name_that_makes_an_invalid_tag_before_changing_anything() {
    let (dir, repo, _cli) = flow_twins();
    let p = dir.path();
    git(p, &["config", "gitflow.prefix.versiontag", ""]);
    repo.flow_start(FlowKind::Release, "-x", None).unwrap_err();
    assert_eq!(git(p, &["branch", "--list", "release/*"]), "");

    // A branch made outside Canopy: finishing it must refuse before the first merge.
    git(p, &["branch", "release/-x", "develop"]);
    git(p, &["switch", "-q", "release/-x"]);
    commit_file(p, "VERSION", "x\n", "chore: x");
    let before = shape(p);
    repo.flow_finish(FlowKind::Release, "-x", &FlowFinish::default())
        .unwrap_err();
    assert_eq!(shape(p), before);
}

#[test]
fn submodule_remove_refuses_unstaged_gitmodules_changes() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let mut text = fs::read_to_string(sup.join(".gitmodules")).expect("read");
    text.push_str("# local note\n");
    fs::write(sup.join(".gitmodules"), text).expect("write");
    assert!(repo.submodule_remove("deps/lib").is_err());
    assert!(
        sub.join(".git").exists(),
        "the submodule must stay checked out"
    );
    assert_eq!(
        repo.submodules(false).expect("submodules")[0].state(),
        SubmoduleState::Clean
    );
}

#[cfg(unix)]
#[test]
fn submodule_remove_refuses_a_module_directory_behind_a_symlink() {
    let (dir, repo, _sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let modules = sup.join(".git").join("modules");
    let outside = dir.path().join("outside");
    fs::rename(modules.join("deps"), &outside).expect("move");
    std::os::unix::fs::symlink(&outside, modules.join("deps")).expect("symlink");
    let before = git(&sup, &["ls-files", "--stage"]);

    assert!(repo.submodule_remove("deps/lib").is_err());
    assert!(outside.join("lib").is_dir());
    assert_eq!(git(&sup, &["ls-files", "--stage"]), before);
}

/// The Changes row of the submodule at `deps/lib`.
fn submodule_row(repo: &Repo) -> Vec<canopy_core::FileEntry> {
    repo.snapshot(10)
        .expect("snapshot")
        .status
        .unstaged
        .into_iter()
        .filter(|f| f.path == "deps/lib")
        .collect()
}

/// #285: Discard on a submodule row brings the submodule back to what the superproject records:
/// tracked edits inside it go, the recorded commit is checked out, and untracked files stay.
/// `git checkout -- <submodule>` alone did nothing, whatever the row showed.
#[test]
fn discarding_a_submodule_restores_what_the_superproject_records_and_keeps_untracked_files() {
    for recurse in [false, true] {
        let (dir, repo, sub) = submodule_fixture();
        let sup = dir.path().join("super");
        git(
            &sup,
            &[
                "config",
                "submodule.recurse",
                if recurse { "true" } else { "false" },
            ],
        );
        let recorded = git(&sup, &["rev-parse", "HEAD:deps/lib"]);
        // A new commit on a branch, an edit to a tracked file and an untracked file inside.
        git(&sub, &["checkout", "-q", "main"]);
        fs::write(sub.join("a.txt"), "edited\n").expect("edit");
        fs::write(sub.join("mine.txt"), "keep me\n").expect("untracked");
        let row = submodule_row(&repo);
        assert_eq!(row.len(), 1, "the submodule shows as changed");
        assert!(row[0].is_submodule(), "{row:?}");

        repo.discard(&row).expect("discard");

        assert_eq!(git(&sub, &["rev-parse", "HEAD"]), recorded);
        assert_eq!(
            fs::read_to_string(sub.join("a.txt")).expect("read"),
            "one\ntwo\nthree\n"
        );
        assert_eq!(
            fs::read_to_string(sub.join("mine.txt")).expect("read"),
            "keep me\n",
            "recurse={recurse}: an untracked file inside the submodule stays"
        );
        assert!(git(&sub, &["branch", "--list", "main"]).contains("main"));
    }
}

/// A commit made in a submodule on no branch would be left behind by going back to the recorded
/// commit: Discard refuses and changes nothing.
#[test]
fn discarding_a_submodule_refuses_to_strand_a_commit_on_no_branch() {
    for recurse in [false, true] {
        let (dir, repo, sub) = submodule_fixture();
        git(
            &dir.path().join("super"),
            &[
                "config",
                "submodule.recurse",
                if recurse { "true" } else { "false" },
            ],
        );
        fs::write(sub.join("a.txt"), "detached work\n").expect("edit");
        git(&sub, &["commit", "-q", "-am", "feat: detached"]);
        let detached = git(&sub, &["rev-parse", "HEAD"]);
        fs::write(sub.join("a.txt"), "more\n").expect("edit");

        let err = repo.discard(&submodule_row(&repo)).expect_err("refused");

        assert!(err.to_string().contains("no branch"), "{err}");
        assert_eq!(git(&sub, &["rev-parse", "HEAD"]), detached);
        assert_eq!(
            fs::read_to_string(sub.join("a.txt")).expect("read"),
            "more\n",
            "recurse={recurse}: nothing was discarded"
        );
    }
}

/// A submodule whose folder was deleted shows as a change; Discard checks it out again at the
/// recorded commit instead of failing on the missing folder.
#[test]
fn discarding_a_submodule_whose_folder_was_deleted_checks_it_out_again() {
    for recurse in [false, true] {
        let (dir, repo, sub) = submodule_fixture();
        let sup = dir.path().join("super");
        git(
            &sup,
            &[
                "config",
                "submodule.recurse",
                if recurse { "true" } else { "false" },
            ],
        );
        let recorded = git(&sup, &["rev-parse", "HEAD:deps/lib"]);
        fs::remove_dir_all(&sub).expect("delete the submodule folder");
        let row = submodule_row(&repo);

        repo.discard(&row).expect("discard");

        assert_eq!(git(&sub, &["rev-parse", "HEAD"]), recorded);
        assert!(sub.join("a.txt").exists());
    }
}

/// With Canopy's choice to delete them, Discard on a submodule also deletes its untracked files;
/// ignored files stay.
#[test]
fn discarding_a_submodule_deletes_its_untracked_files_when_chosen() {
    for recurse in [false, true] {
        let (dir, repo, sub) = submodule_fixture();
        git(
            &dir.path().join("super"),
            &[
                "config",
                "submodule.recurse",
                if recurse { "true" } else { "false" },
            ],
        );
        fs::write(sub.join(".gitignore"), "*.log\n").expect("ignore");
        fs::write(sub.join("debug.log"), "ignored\n").expect("ignored");
        fs::write(sub.join("mine.txt"), "untracked\n").expect("untracked");
        fs::write(sub.join("a.txt"), "edited\n").expect("edit");
        repo.set_options(GitOptions {
            discard_untracked_in_submodules: true,
            ..GitOptions::default()
        });

        repo.discard(&submodule_row(&repo)).expect("discard");

        assert!(
            !sub.join("mine.txt").exists(),
            "recurse={recurse}: the untracked file is deleted"
        );
        assert!(sub.join("debug.log").exists(), "an ignored file stays");
        assert_eq!(
            fs::read_to_string(sub.join("a.txt")).expect("read"),
            "one\ntwo\nthree\n"
        );
    }
}

/// An untracked file or directory obstructing a tracked path makes `git reset --hard` delete it.
/// Keep-untracked Discard must refuse before changing anything, whatever recursive setting Git has.
#[test]
fn discarding_a_submodule_refuses_to_delete_an_obstructing_untracked_path() {
    for recurse in [false, true] {
        let (dir, repo, sub) = submodule_fixture();
        git(
            &dir.path().join("super"),
            &[
                "config",
                "submodule.recurse",
                if recurse { "true" } else { "false" },
            ],
        );
        fs::remove_file(sub.join("a.txt")).expect("remove tracked file");
        fs::create_dir(sub.join("a.txt")).expect("make untracked directory");
        fs::write(sub.join("a.txt").join("notes"), "precious\n").expect("write note");

        let err = repo.discard(&submodule_row(&repo)).expect_err("refused");

        assert!(err.to_string().contains("obstruct"), "{err}");
        assert_eq!(
            fs::read_to_string(sub.join("a.txt").join("notes")).expect("read note"),
            "precious\n",
            "recurse={recurse}: the untracked note was not deleted"
        );
    }
}

/// Ignored files always stay, even when deleting untracked files was chosen, so an ignored path
/// obstructing a tracked path is refused too.
#[test]
fn discarding_a_submodule_refuses_to_delete_an_obstructing_ignored_path() {
    let (dir, repo, sub) = submodule_fixture();
    let _ = dir;
    fs::write(sub.join(".gitignore"), "a.txt\n").expect("ignore");
    fs::remove_file(sub.join("a.txt")).expect("remove tracked file");
    fs::create_dir(sub.join("a.txt")).expect("make ignored directory");
    fs::write(sub.join("a.txt").join("notes"), "precious\n").expect("write note");
    repo.set_options(GitOptions {
        discard_untracked_in_submodules: true,
        ..GitOptions::default()
    });

    let err = repo.discard(&submodule_row(&repo)).expect_err("refused");

    assert!(err.to_string().contains("ignored"), "{err}");
    assert_eq!(
        fs::read_to_string(sub.join("a.txt").join("notes")).expect("read note"),
        "precious\n"
    );
}

/// Different spellings are unrelated on a case-sensitive filesystem. They are allowed and kept;
/// on a case-insensitive or normalizing one the canonical-path preflight sees the collision.
#[test]
fn discarding_a_submodule_allows_an_unrelated_case_variant_path() {
    let (dir, repo, sub) = submodule_fixture();
    let _ = dir;
    fs::remove_file(sub.join("a.txt")).expect("remove tracked file");
    fs::create_dir(sub.join("A.TXT")).expect("make case-variant directory");
    fs::write(sub.join("A.TXT").join("notes"), "precious\n").expect("write note");

    repo.discard(&submodule_row(&repo)).expect("discard");

    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).expect("restored file"),
        "one\ntwo\nthree\n"
    );
    assert_eq!(
        fs::read_to_string(sub.join("A.TXT").join("notes")).expect("read note"),
        "precious\n"
    );
}

/// A staged deletion is restored by reset --hard too, and must not overwrite an untracked
/// directory at that path.
#[test]
fn discarding_a_submodule_refuses_an_obstruction_after_a_staged_deletion() {
    let (dir, repo, sub) = submodule_fixture();
    let _ = dir;
    git(&sub, &["rm", "--cached", "-q", "a.txt"]);
    fs::remove_file(sub.join("a.txt")).expect("remove untracked file");
    fs::create_dir(sub.join("a.txt")).expect("make untracked directory");
    fs::write(sub.join("a.txt").join("notes"), "precious\n").expect("write note");

    let err = repo.discard(&submodule_row(&repo)).expect_err("refused");

    assert!(err.to_string().contains("obstruct"), "{err}");
    assert_eq!(
        fs::read_to_string(sub.join("a.txt").join("notes")).expect("read note"),
        "precious\n"
    );
}

/// A dangling symlink has no target whose contents need preserving: reset can replace it with the
/// tracked file instead of failing canonicalization.
#[cfg(unix)]
#[test]
fn discarding_a_submodule_replaces_a_dangling_symlink() {
    use std::os::unix::fs::symlink;
    let (dir, repo, sub) = submodule_fixture();
    let _ = dir;
    fs::remove_file(sub.join("a.txt")).expect("remove tracked file");
    symlink("missing-target", sub.join("a.txt")).expect("dangling symlink");

    repo.discard(&submodule_row(&repo)).expect("discard");

    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).expect("restored file"),
        "one\ntwo\nthree\n"
    );
    assert!(!sub.join("missing-target").exists());
}

/// An unrelated untracked file is not an obstruction just because another tracked file was
/// deleted: reset restores the latter and leaves the former alone.
#[test]
fn discarding_a_submodule_allows_an_unrelated_untracked_file_beside_a_deletion() {
    let (dir, repo, sub) = submodule_fixture();
    let _ = dir;
    fs::remove_file(sub.join("a.txt")).expect("remove tracked file");
    fs::write(sub.join("notes.txt"), "unrelated\n").expect("write unrelated");

    repo.discard(&submodule_row(&repo)).expect("discard");

    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).expect("restored file"),
        "one\ntwo\nthree\n"
    );
    assert_eq!(
        fs::read_to_string(sub.join("notes.txt")).expect("read unrelated"),
        "unrelated\n"
    );
}

/// All selected submodules are checked before the first one is reset: a later obstruction cannot
/// turn a reported refusal into a partial Discard.
#[test]
fn discarding_multiple_submodules_checks_all_before_resetting_any() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    let url = dir.path().join("lib").to_string_lossy().into_owned();
    git(
        &sup,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            "--",
            &url,
            "deps/other",
        ],
    );
    let other = sup.join("deps/other");
    git(&other, &["checkout", "-q", "HEAD~1"]);
    git(&sup, &["add", "deps/other"]);
    git(&sup, &["commit", "-q", "-m", "feat: add another lib"]);
    fs::write(sub.join("a.txt"), "first edit\n").expect("first edit");
    fs::remove_file(other.join("a.txt")).expect("remove second tracked file");
    fs::create_dir(other.join("a.txt")).expect("make second obstruction");
    fs::write(other.join("a.txt").join("notes"), "precious\n").expect("write note");
    let mut rows = repo
        .snapshot(10)
        .expect("snapshot")
        .status
        .unstaged
        .into_iter()
        .filter(|f| ["deps/lib", "deps/other"].contains(&f.path.as_str()))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(rows.len(), 2);

    let err = repo.discard(&rows).expect_err("refused");

    assert!(err.to_string().contains("obstruct"), "{err}");
    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).expect("first file"),
        "first edit\n",
        "the earlier submodule was not reset"
    );
    assert_eq!(
        fs::read_to_string(other.join("a.txt").join("notes")).expect("read note"),
        "precious\n"
    );
}

/// A submodule path is a literal path, never a pathspec that can select another submodule.
#[test]
fn discarding_a_submodule_with_glob_characters_touches_only_its_literal_path() {
    for recurse in [false, true] {
        let (dir, repo, _sub) = submodule_fixture();
        let sup = dir.path().join("super");
        let url = dir.path().join("lib").to_string_lossy().into_owned();
        for path in ["deps/lib1", "deps/lib[1]"] {
            git(
                &sup,
                &[
                    "--literal-pathspecs",
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "-q",
                    "--",
                    &url,
                    path,
                ],
            );
            let sub = sup.join(path);
            git(&sub, &["checkout", "-q", "HEAD~1"]);
        }
        git(&sup, &["add", "."]);
        git(
            &sup,
            &["commit", "-q", "-m", "feat: add literal path fixtures"],
        );
        git(
            &sup,
            &[
                "config",
                "submodule.recurse",
                if recurse { "true" } else { "false" },
            ],
        );
        let glob = sup.join("deps/lib[1]");
        let plain = sup.join("deps/lib1");
        git(&glob, &["checkout", "-q", "main"]);
        git(&plain, &["checkout", "-q", "main"]);
        let plain_head = git(&plain, &["rev-parse", "HEAD"]);
        let row = repo
            .snapshot(10)
            .expect("snapshot")
            .status
            .unstaged
            .into_iter()
            .filter(|f| f.path == "deps/lib[1]")
            .collect::<Vec<_>>();
        assert_eq!(row.len(), 1);

        repo.discard(&row).expect("discard literal path");

        let recorded = git(&sup, &["rev-parse", "HEAD:deps/lib[1]"]);
        assert_eq!(git(&glob, &["rev-parse", "HEAD"]), recorded);
        assert_eq!(
            git(&plain, &["rev-parse", "HEAD"]),
            plain_head,
            "recurse={recurse}: deps/lib[1] must not match deps/lib1"
        );
    }
}

/// Submodule operations keep a native non-UTF-8 path rather than looking for U+FFFD on disk.
#[cfg(unix)]
#[test]
fn discarding_a_submodule_uses_its_non_utf8_path() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let lib = dir.path().join("lib");
    let sup = dir.path().join("super");
    fs::create_dir_all(&lib).expect("mkdir");
    fs::create_dir_all(&sup).expect("mkdir");
    init(&lib);
    commit_file(&lib, "b.txt", "b\n", "feat: second");
    init(&sup);
    let native = OsStr::from_bytes(b"deps/lib\xff");
    let added = git_output_os(
        &sup,
        &[
            OsStr::new("-c"),
            OsStr::new("protocol.file.allow=always"),
            OsStr::new("submodule"),
            OsStr::new("add"),
            OsStr::new("-q"),
            OsStr::new("--"),
            lib.as_os_str(),
            native,
        ],
    );
    assert!(
        added.status.success(),
        "add non-UTF-8 submodule: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    let sub = sup.join(native);
    git(&sub, &["checkout", "-q", "HEAD~1"]);
    git(&sup, &["add", "."]);
    git(&sup, &["commit", "-q", "-m", "feat: add non-UTF-8 lib"]);
    let repo = Repo::open(&sup).expect("open");
    fs::write(sub.join("a.txt"), "edited\n").expect("edit");
    let row = repo
        .snapshot(10)
        .expect("snapshot")
        .status
        .unstaged
        .into_iter()
        .filter(canopy_core::FileEntry::is_submodule)
        .collect::<Vec<_>>();
    assert_eq!(row.len(), 1, "the native path must reach the changed row");

    repo.discard(&row).expect("discard non-UTF-8 path");

    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).expect("read"),
        "one\ntwo\nthree\n"
    );
}

/// A direct rebase must not let Git's own recursive update drop an edit inside a submodule;
/// Canopy's explicit update afterwards preserves it.
#[test]
fn rebasing_keeps_edits_inside_submodules() {
    let (dir, repo, sub) = submodule_fixture();
    let sup = dir.path().join("super");
    git(&sup, &["switch", "-q", "-c", "topic"]);
    git(&sup, &["switch", "-q", "main"]);
    git(&sub, &["checkout", "-q", "main"]);
    let recorded = git(&sub, &["rev-parse", "HEAD"]);
    git(&sup, &["add", "deps/lib"]);
    git(&sup, &["commit", "-q", "-m", "feat: update lib"]);
    git(&sup, &["switch", "-q", "topic"]);
    commit_file(&sup, "topic.txt", "topic\n", "feat: topic");
    git(&sup, &["config", "submodule.recurse", "true"]);
    git(&sub, &["checkout", "-q", "HEAD~1"]);
    fs::write(sub.join("a.txt"), "edited\n").expect("edit");

    repo.rebase("main").expect("rebase");
    repo.follow_submodules(&mut |_: &str| {}).expect("follow");

    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).expect("read"),
        "edited\n",
        "the recursive sequencer must not drop the edit"
    );
    assert_eq!(git(&sub, &["rev-parse", "HEAD"]), recorded);
    assert_eq!(git(&sup, &["rev-parse", "HEAD:deps/lib"]), recorded);
}
