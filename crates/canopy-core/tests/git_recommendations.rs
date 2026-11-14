//! The audit of Canopy's recommended Git settings (docs/adr/0012-git-defaults.md, "Audit of the
//! recommendations"): what Use recommended writes to a private global configuration, and, with the
//! git executable alone, what git then does for each key. Every test runs in a child process
//! whose `GIT_CONFIG_GLOBAL` is its own file, so the developer's configuration decides nothing.
//! Where Canopy relays a choice to git, its result is compared with the git CLI's.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use canopy_core::ops::MergeMode;
use canopy_core::{GitOptions, Repo, cli, config, config::Setting};

/// Runs the calling test again in a child process with a private global configuration. Returns
/// true in the child, where the test body runs, and false in the parent once the child passed.
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs itself again in a child process"
)]
fn isolated(test: &str) -> bool {
    if std::env::var_os("CANOPY_AUDIT_TEST_CHILD").is_some() {
        return true;
    }
    let home = tempfile::tempdir().expect("private home");
    let out = Command::new(std::env::current_exe().expect("test binary"))
        .args([test, "--exact", "--test-threads=1"])
        .env("CANOPY_AUDIT_TEST_CHILD", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.path().join("global"))
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .env_remove("EMAIL")
        .output()
        .expect("child test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}:\n{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

fn global() -> PathBuf {
    PathBuf::from(std::env::var_os("GIT_CONFIG_GLOBAL").expect("private global config"))
}

/// `git args` in `dir`, whatever it exits with. `env` adds to the child's environment.
#[expect(
    clippy::disallowed_methods,
    reason = "tests drive git directly: the git CLI is the oracle"
)]
fn run(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .envs(env.iter().copied())
        .output()
        .expect("git runs")
}

/// `git args` in `dir`, which must succeed; its standard output.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args, &[]);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `git args` in `dir`, which must fail; its standard error.
fn refused(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args, &[]);
    assert!(
        !out.status.success(),
        "git {args:?} succeeded: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn at_least(version: (u32, u32)) -> bool {
    cli::version_number().is_some_and(|v| v >= version)
}

/// What the private global file holds, `git config --global --list` as key and value.
fn global_entries(dir: &Path) -> BTreeMap<String, String> {
    git(dir, &["config", "--global", "--list"])
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// Identity for the commits the tests make; applying the recommendations leaves it alone.
fn identity() {
    let scratch = tempfile::tempdir().expect("scratch");
    for (key, value) in [
        ("user.name", "Ada Lovelace"),
        ("user.email", "ada@example.com"),
        // Submodules are cloned from local paths in the submodule tests.
        ("protocol.file.allow", "always"),
    ] {
        git(scratch.path(), &["config", "--global", key, value]);
    }
}

/// Applies the recommendations to the private global configuration.
fn apply(dir: &Path) {
    config::apply_recommended(dir).expect("apply recommendations");
}

const LINES: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\n";

/// A bare upstream with `a.txt` on `main`, and room for clones.
struct World {
    root: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        identity();
        let root = tempfile::tempdir().expect("world");
        git(
            root.path(),
            &["init", "-q", "--bare", "-b", "main", "up.git"],
        );
        let world = Self { root };
        let seed = world.clone_to("seed");
        commit_file(&seed, "a.txt", LINES, "feat: first");
        git(&seed, &["push", "-q", "origin", "main"]);
        world
    }

    fn up(&self) -> PathBuf {
        self.root.path().join("up.git")
    }

    fn clone_to(&self, name: &str) -> PathBuf {
        let dest = self.root.path().join(name);
        git(
            self.root.path(),
            &["clone", "-q", "up.git", &dest.to_string_lossy()],
        );
        dest
    }
}

fn commit_file(dir: &Path, file: &str, text: &str, message: &str) {
    fs::write(dir.join(file), text).expect("write");
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Upstream changed line one of `a.txt`; the clone, not yet told, will have line nine edited
/// and uncommitted.
fn upstream_edits_line_one(world: &World, clone: &Path) {
    let other = world.clone_to("upstream-editor");
    commit_file(
        &other,
        "a.txt",
        &LINES.replacen("one", "ONE", 1),
        "feat: upstream",
    );
    git(&other, &["push", "-q", "origin", "main"]);
    git(clone, &["fetch", "-q"]);
}

fn dirty_line_nine(clone: &Path) {
    let text = fs::read_to_string(clone.join("a.txt")).expect("read");
    fs::write(clone.join("a.txt"), text.replace("nine", "NINE")).expect("write");
}

/// Both edits are in the file, and nothing is left in the stash.
fn assert_both_edits_and_no_stash(clone: &Path) {
    let text = fs::read_to_string(clone.join("a.txt")).expect("read");
    assert!(
        text.contains("ONE") && text.contains("NINE"),
        "edits lost:\n{text}"
    );
    assert_eq!(git(clone, &["stash", "list"]), "");
}

#[test]
fn use_recommended_writes_exactly_these_keys_to_the_global_file_and_git_reads_them_back() {
    if !isolated(
        "use_recommended_writes_exactly_these_keys_to_the_global_file_and_git_reads_them_back",
    ) {
        return;
    }
    let world = World::new();
    let clone = world.clone_to("clone");
    // A choice of the user's own that Use recommended must leave alone, and signing and line
    // endings it must not write whether set or not.
    for (key, value) in [
        ("core.autocrlf", "input"),
        ("commit.gpgsign", "true"),
        ("gpg.format", "x509"),
        ("core.hooksPath", "user-hooks"),
    ] {
        git(&clone, &["config", "--global", key, value]);
    }
    let before = global_entries(&clone);
    apply(&clone);
    let after = global_entries(&clone);

    let conflict_style = if at_least((2, 35)) { "zdiff3" } else { "diff3" };
    let expected: BTreeMap<String, String> = [
        ("pull.rebase", "false"),
        ("pull.ff", "only"),
        ("pull.autostash", "true"),
        ("rebase.autostash", "true"),
        ("merge.autostash", "true"),
        ("fetch.prune", "true"),
        ("fetch.prunetags", "false"),
        ("push.followtags", "true"),
        ("submodule.recurse", "true"),
        ("push.default", "simple"),
        ("merge.conflictstyle", conflict_style),
        ("init.defaultbranch", "main"),
        ("commit.cleanup", "default"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let written: BTreeMap<String, String> = after
        .iter()
        .filter(|(k, v)| before.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    assert_eq!(written, expected, "what Use recommended wrote");
    for (key, value) in &before {
        assert!(
            expected.contains_key(key) || after.get(key) == Some(value),
            "{key} was changed"
        );
    }
    assert_eq!(
        after.get("core.autocrlf").map(String::as_str),
        Some("input")
    );
    assert_eq!(
        after.get("commit.gpgsign").map(String::as_str),
        Some("true")
    );
    assert_eq!(after.get("gpg.format").map(String::as_str), Some("x509"));

    // The scope is global: nothing landed in the repository, and git's own effective answer for
    // every key comes from the global file with the recommended value.
    assert!(
        git(&clone, &["config", "--local", "--list"])
            .lines()
            .all(|line| !expected.keys().any(|k| line.starts_with(&format!("{k}=")))),
        "a recommendation was written into the repository"
    );
    for (key, value) in &expected {
        let entries = config::read(&clone, false, config::Query::Get(key)).expect("read");
        assert_eq!(entries.len(), 1, "{key}");
        assert_eq!(entries[0].value.as_deref(), Some(value.as_str()), "{key}");
        assert_eq!(entries[0].scope, "global", "{key}");
        assert_eq!(
            entries[0].file.as_deref(),
            Some(global().as_path()),
            "{key}"
        );
    }

    // Unset line endings and signing stay unset.
    fs::write(global(), "").expect("empty global");
    identity();
    apply(&clone);
    let after = global_entries(&clone);
    for key in ["core.autocrlf", "commit.gpgsign", "gpg.format"] {
        assert!(!after.contains_key(key), "{key} was written");
    }
}

#[test]
fn recommendations_by_key_for_every_supported_git() {
    let table = |git| -> Vec<(&'static str, &'static str)> {
        Setting::ALL
            .iter()
            .map(|s| (s.key(), s.recommendation_for(git)))
            .collect()
    };
    let common = [
        ("pull.rebase", "false"),
        ("pull.ff", "only"),
        ("pull.autoStash", "true"),
        ("fetch.prune", "true"),
        ("fetch.pruneTags", "false"),
        ("rebase.autoStash", "true"),
        ("merge.autoStash", "true"),
        ("push.followTags", "true"),
        ("submodule.recurse", "true"),
        ("push.default", "simple"),
        ("init.defaultBranch", "main"),
        ("commit.cleanup", "default"),
        ("commit.gpgsign", "false"),
    ];
    let line_endings = if cfg!(windows) { "true" } else { "input" };
    for (git, conflict_style, signing_format) in [
        (Some((2, 55)), "zdiff3", "ssh"),
        (Some((2, 35)), "zdiff3", "ssh"),
        (Some((2, 34)), "diff3", "ssh"),
        (Some((2, 33)), "diff3", "openpgp"),
        (None, "diff3", "openpgp"),
    ] {
        let mut expected: Vec<(&str, &str)> = common.to_vec();
        expected.extend([
            ("merge.conflictStyle", conflict_style),
            ("core.autocrlf", line_endings),
            ("gpg.format", signing_format),
        ]);
        let mut actual = table(git);
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "git {git:?}");
    }
}

/// The ADR whose audit table says what is recommended for each key and why.
const ADR: &str = include_str!("../../../docs/adr/0012-git-defaults.md");

#[test]
fn the_adr_table_names_every_recommendation_and_its_source() {
    let (_, audit) = ADR
        .split_once("## Audit of the recommendations")
        .expect("the audit section");
    for setting in Setting::ALL {
        let row = audit
            .lines()
            .find(|line| line.starts_with(&format!("| `{}` |", setting.key())))
            .unwrap_or_else(|| panic!("no row for {}", setting.key()));
        let cells: Vec<&str> = row.split(" | ").collect();
        // Key, recommended, Git's default, why, sources: the values are the backticked words.
        assert_eq!(cells.len(), 5, "{}: {row}", setting.key());
        let recommended: Vec<&str> = cells[1].split('`').skip(1).step_by(2).collect();
        for git in [Some((2, 55)), None] {
            assert!(
                recommended.contains(&setting.recommendation_for(git)),
                "{}: the ADR says {recommended:?}, the code {} on git {git:?}",
                setting.key(),
                setting.recommendation_for(git)
            );
        }
        assert!(
            cells[4].contains("https://git-scm.com/docs/") || cells[4].contains("docs.github.com"),
            "{} has no cited source",
            setting.key()
        );
    }
}

#[test]
fn pulls_fast_forward_or_stop_and_a_rebase_configuration_cannot_override_that() {
    if !isolated("pulls_fast_forward_or_stop_and_a_rebase_configuration_cannot_override_that") {
        return;
    }
    let world = World::new();
    let clone = world.clone_to("clone");
    // What a user who rebases on pull had, and what Canopy's first recommendation replaces.
    git(&clone, &["config", "--global", "pull.rebase", "true"]);
    git(&clone, &["config", "--global", "pull.ff", "false"]);
    apply(&clone);

    // Behind upstream: a plain fast-forward, no merge commit.
    upstream_edits_line_one(&world, &clone);
    git(&clone, &["pull", "-q"]);
    assert_eq!(
        git(&clone, &["rev-parse", "HEAD"]),
        git(&clone, &["rev-parse", "origin/main"])
    );

    // Diverged: git stops, and the branch, the files and the repository state stay as they were.
    let other = world.clone_to("other");
    commit_file(&other, "up.txt", "up\n", "feat: more upstream");
    git(&other, &["push", "-q", "origin", "main"]);
    commit_file(&clone, "local.txt", "local\n", "feat: local");
    git(&clone, &["fetch", "-q"]);
    let head = git(&clone, &["rev-parse", "HEAD"]);
    let message = refused(&clone, &["pull"]);
    assert!(
        message.contains("Not possible to fast-forward"),
        "git said: {message}"
    );
    assert_eq!(git(&clone, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&clone, &["status", "--porcelain"]), "");
    assert!(!clone.join(".git/MERGE_HEAD").exists());
    assert!(!clone.join(".git/rebase-merge").exists());

    // Since git 2.34 `pull.ff=only` also beats `pull.rebase=true` and `branch.<name>.rebase`
    // (only a `--rebase` on the command line overrides it); older git rebased there, which is
    // why `pull.rebase=false` is written next to it.
    if at_least((2, 34)) {
        let message = refused(&clone, &["-c", "pull.rebase=true", "pull"]);
        assert!(
            message.contains("Not possible to fast-forward"),
            "{message}"
        );
        git(&clone, &["config", "branch.main.rebase", "true"]);
        let message = refused(&clone, &["pull"]);
        assert!(
            message.contains("Not possible to fast-forward"),
            "{message}"
        );
        git(&clone, &["config", "--unset", "branch.main.rebase"]);
    }

    // Nothing is locked: asking for a rebase on the command line still works.
    git(&clone, &["pull", "-q", "--rebase"]);
    assert_eq!(
        git(&clone, &["rev-list", "--count", "origin/main..HEAD"]).trim(),
        "1"
    );
}

#[test]
fn the_three_autostash_keys_let_git_run_a_rebase_a_merge_and_a_pull_over_local_changes() {
    if !isolated(
        "the_three_autostash_keys_let_git_run_a_rebase_a_merge_and_a_pull_over_local_changes",
    ) {
        return;
    }
    let no_autostash = [
        "-c",
        "rebase.autoStash=false",
        "-c",
        "merge.autoStash=false",
    ];

    // rebase.autoStash
    let world = World::new();
    let clone = world.clone_to("rebase");
    commit_file(&clone, "b.txt", "b\n", "feat: local");
    upstream_edits_line_one(&world, &clone);
    dirty_line_nine(&clone);
    let message = refused(
        &clone,
        &[no_autostash.as_slice(), &["rebase", "origin/main"]].concat(),
    );
    assert!(message.contains("cannot rebase"), "git said: {message}");
    apply(&clone);
    git(&clone, &["rebase", "origin/main"]);
    assert_both_edits_and_no_stash(&clone);

    // merge.autoStash
    let world = World::new();
    let clone = world.clone_to("merge");
    commit_file(&clone, "b.txt", "b\n", "feat: local");
    upstream_edits_line_one(&world, &clone);
    dirty_line_nine(&clone);
    let message = refused(
        &clone,
        &[no_autostash.as_slice(), &["merge", "origin/main"]].concat(),
    );
    assert!(
        message.contains("would be overwritten by merge"),
        "git said: {message}"
    );
    git(&clone, &["merge", "--no-edit", "origin/main"]);
    assert_both_edits_and_no_stash(&clone);
    assert_eq!(
        git(&clone, &["rev-list", "--parents", "-1", "HEAD"])
            .split(' ')
            .count(),
        3
    );

    // pull.autoStash, and its place in git's order: set to either value it beats merge.autoStash
    // and rebase.autoStash, but only git 2.51 and later read it.
    let world = World::new();
    let clone = world.clone_to("pull");
    upstream_edits_line_one(&world, &clone);
    dirty_line_nine(&clone);
    let message = refused(
        &clone,
        &[
            "-c",
            "pull.autoStash=false",
            "-c",
            "merge.autoStash=false",
            "pull",
        ],
    );
    assert!(
        message.contains("would be overwritten by merge"),
        "{message}"
    );
    let beaten = run(&clone, &["-c", "pull.autoStash=false", "pull"], &[]);
    assert_eq!(
        beaten.status.success(),
        !at_least((2, 51)),
        "pull.autoStash=false against merge.autoStash=true on git {:?}: {}",
        cli::version_number(),
        String::from_utf8_lossy(&beaten.stderr)
    );
    git(&clone, &["reset", "-q", "--hard"]);
    dirty_line_nine(&clone);
    git(
        &clone,
        &[
            "-c",
            "merge.autoStash=false",
            "-c",
            "rebase.autoStash=false",
            "pull",
            "-q",
        ],
    );
    assert_both_edits_and_no_stash(&clone);
}

#[test]
fn canopys_merge_stashes_what_its_choice_and_the_configuration_say_as_git_does() {
    if !isolated("canopys_merge_stashes_what_its_choice_and_the_configuration_say_as_git_does") {
        return;
    }
    // (merge.autoStash in the repository, Canopy's autostash choice) -> does the merge run?
    for (configured, canopy, runs) in [
        (None, true, true),
        (None, false, false),
        (Some("false"), true, false),
        (Some("true"), false, true),
    ] {
        for (mode, diverged) in [
            (MergeMode::Default, false),
            (MergeMode::Default, true),
            (MergeMode::NoFastForward, false),
            (MergeMode::FastForwardOnly, false),
        ] {
            let world = World::new();
            let clone = world.clone_to("clone");
            if diverged {
                commit_file(&clone, "b.txt", "b\n", "feat: local");
            }
            upstream_edits_line_one(&world, &clone);
            dirty_line_nine(&clone);
            if let Some(value) = configured {
                git(&clone, &["config", "merge.autoStash", value]);
            }
            let repo = Repo::open(&clone).expect("open");
            repo.set_options(GitOptions {
                autostash: canopy,
                ..GitOptions::default()
            });
            let result = repo.merge("origin/main", mode);
            assert_eq!(
                result.is_ok(),
                runs,
                "{mode:?} (diverged: {diverged}) with merge.autoStash {configured:?} and Canopy {canopy}: {result:?}"
            );
            if runs {
                assert_eq!(git(&clone, &["stash", "list"]), "", "{mode:?}");
                let text = fs::read_to_string(clone.join("a.txt")).expect("read");
                assert!(text.contains("ONE") && text.contains("NINE"), "{mode:?}");
            } else {
                let text = fs::read_to_string(clone.join("a.txt")).expect("read");
                assert!(text.contains("NINE") && !text.contains("ONE"), "{mode:?}");
            }
        }
    }
}

/// A clone of `world` whose local branch and upstream both moved on, `d.txt` committed and then
/// edited without committing. With `conflict` both sides change line one of `a.txt`, else the
/// upstream adds `s.txt` and the clone `b.txt`.
fn squash_ready(world: &World, conflict: bool) -> PathBuf {
    let clone = world.clone_to("clone");
    commit_file(&clone, "d.txt", "d\n", "feat: d");
    let other = world.clone_to("upstream-editor");
    if conflict {
        let text = LINES.replacen("one", "ONE", 1);
        commit_file(&other, "a.txt", &text, "feat: upstream");
        commit_file(
            &clone,
            "a.txt",
            &LINES.replacen("one", "UNO", 1),
            "feat: local",
        );
    } else {
        commit_file(&other, "s.txt", "s\n", "feat: upstream");
        commit_file(&clone, "b.txt", "b\n", "feat: local");
    }
    git(&other, &["push", "-q", "origin", "main"]);
    git(&clone, &["fetch", "-q"]);
    fs::write(clone.join("d.txt"), "d\nmy edit\n").expect("write");
    clone
}

/// The edit is in the working tree, and nothing is held back: no stash, and no autostash that
/// only `git commit` or `git merge --continue` would bring back.
fn assert_edit_in_the_tree_and_nothing_hidden(clone: &Path) {
    assert_eq!(
        fs::read_to_string(clone.join("d.txt")).expect("read"),
        "d\nmy edit\n"
    );
    assert_eq!(git(clone, &["stash", "list"]), "");
    assert!(
        !clone.join(".git").join("MERGE_AUTOSTASH").exists(),
        "an autostash is held back"
    );
}

#[test]
fn a_squash_never_hides_your_changes() {
    if !isolated("a_squash_never_hides_your_changes") {
        return;
    }
    // The premise: git's own `--autostash` on a squash that does not fast-forward stashes the
    // edit and never puts it back, out of `git stash list`. (Without the premise the rest
    // would prove nothing.)
    if at_least((2, 27)) {
        let world = World::new();
        let clone = squash_ready(&world, false);
        git(
            &clone,
            &[
                "merge",
                "--no-edit",
                "--squash",
                "--autostash",
                "--",
                "origin/main",
            ],
        );
        assert!(clone.join(".git").join("MERGE_AUTOSTASH").exists());
        assert_eq!(
            fs::read_to_string(clone.join("d.txt")).expect("read"),
            "d\n",
            "git hid the edit"
        );
    }

    // Canopy's squash leaves the edit where it is, whatever Canopy's choice or the
    // configuration says, for a squash that merges and for one that conflicts.
    for (configured, canopy) in [(None, true), (None, false), (Some("true"), true)] {
        for conflict in [false, true] {
            let world = World::new();
            let clone = squash_ready(&world, conflict);
            if let Some(value) = configured {
                git(&clone, &["config", "merge.autoStash", value]);
            }
            let repo = Repo::open(&clone).expect("open");
            repo.set_options(GitOptions {
                autostash: canopy,
                ..GitOptions::default()
            });
            let result = repo.merge("origin/main", MergeMode::Squash);
            let what =
                format!("conflict {conflict}, merge.autoStash {configured:?}, Canopy {canopy}");
            assert_eq!(result.is_err(), conflict, "{what}: {result:?}");
            assert_edit_in_the_tree_and_nothing_hidden(&clone);
            let status = git(&clone, &["status", "--porcelain"]);
            if conflict {
                assert!(status.contains("UU a.txt"), "{what}: {status}");
            } else {
                assert!(
                    status.contains("A  s.txt") && status.contains(" M d.txt"),
                    "{what}: {status}"
                );
            }
        }
    }

    // Over a file the squash changes, it is refused as git refuses it without autostash.
    let world = World::new();
    let clone = world.clone_to("clone");
    upstream_edits_line_one(&world, &clone);
    dirty_line_nine(&clone);
    let repo = Repo::open(&clone).expect("open");
    let message = repo
        .merge("origin/main", MergeMode::Squash)
        .expect_err("refused");
    assert!(
        message.to_string().contains("would be overwritten"),
        "{message}"
    );
    assert!(!clone.join(".git").join("MERGE_AUTOSTASH").exists());
    let text = fs::read_to_string(clone.join("a.txt")).expect("read");
    assert!(text.contains("NINE") && !text.contains("ONE"));
}

#[test]
fn changes_git_holds_aside_for_a_stopped_merge_or_rebase_are_recognised() {
    if !isolated("changes_git_holds_aside_for_a_stopped_merge_or_rebase_are_recognised") {
        return;
    }
    for rebase in [false, true] {
        for dirty in [false, true] {
            let world = World::new();
            let clone = squash_ready(&world, true);
            if !dirty {
                git(&clone, &["checkout", "-q", "--", "d.txt"]);
            }
            let repo = Repo::open(&clone).expect("open");
            let what = format!("rebase {rebase}, uncommitted changes {dirty}");
            assert!(!repo.autostash_held(), "{what}: before");
            let result = if rebase {
                repo.rebase("origin/main")
            } else {
                repo.merge("origin/main", MergeMode::Default)
            };
            assert!(result.is_err(), "{what}: stops on conflicts");
            assert!(repo.stopped_on_conflicts().expect("state"), "{what}");
            assert_eq!(repo.autostash_held(), dirty, "{what}");
            let state = if rebase {
                canopy_core::model::RepoState::Rebase
            } else {
                canopy_core::model::RepoState::Merge
            };
            repo.continue_or_abort(state, true).expect("abort");
            assert!(!repo.autostash_held(), "{what}: after the abort");
            if dirty {
                assert_edit_in_the_tree_and_nothing_hidden(&clone);
            }
        }
    }
}

#[test]
fn changes_held_aside_in_a_linked_worktree_are_recognised_there_only() {
    if !isolated("changes_held_aside_in_a_linked_worktree_are_recognised_there_only") {
        return;
    }
    let world = World::new();
    let clone = squash_ready(&world, true);
    let linked = world.root.path().join("linked");
    git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "topic",
            &linked.to_string_lossy(),
        ],
    );
    fs::write(linked.join("d.txt"), "d\nmy edit\n").expect("write");
    let repo = Repo::open(&linked).expect("open");
    repo.merge("origin/main", MergeMode::Default)
        .expect_err("stops on conflicts");
    assert!(repo.autostash_held(), "the linked worktree holds it");
    assert!(
        !Repo::open(&clone).expect("open").autostash_held(),
        "the main worktree holds nothing"
    );
    repo.continue_or_abort(canopy_core::model::RepoState::Merge, true)
        .expect("abort");
    assert!(!repo.autostash_held());
    assert_eq!(
        fs::read_to_string(linked.join("d.txt")).expect("read"),
        "d\nmy edit\n"
    );
}

#[test]
fn a_merge_whose_stash_conflicts_on_the_way_back_is_recognised() {
    if !isolated("a_merge_whose_stash_conflicts_on_the_way_back_is_recognised") {
        return;
    }
    let world = World::new();
    let clone = world.clone_to("clone");
    upstream_edits_line_one(&world, &clone);
    let text = fs::read_to_string(clone.join("a.txt")).expect("read");
    fs::write(clone.join("a.txt"), text.replacen("one", "UNO", 1)).expect("write");
    let repo = Repo::open(&clone).expect("open");
    repo.merge("origin/main", MergeMode::Default)
        .expect("the merge itself succeeds");
    assert!(repo.autostash_conflicted().expect("check"));
    assert!(git(&clone, &["stash", "list"]).contains("autostash"));
}

#[test]
fn fetch_prunes_stale_branches_and_never_the_tags_only_you_have() {
    if !isolated("fetch_prunes_stale_branches_and_never_the_tags_only_you_have") {
        return;
    }
    let world = World::new();
    let publisher = world.clone_to("publisher");
    git(
        &publisher,
        &["push", "-q", "origin", "main:refs/heads/gone"],
    );
    let clone = world.clone_to("clone");
    apply(&clone);
    git(&clone, &["tag", "local-only"]);
    git(&publisher, &["push", "-q", "origin", "--delete", "gone"]);

    // Git's default keeps the stale remote-tracking branch; the recommendation removes it.
    git(&clone, &["-c", "fetch.prune=false", "fetch", "-q"]);
    assert!(git(&clone, &["branch", "-r"]).contains("origin/gone"));
    git(&clone, &["fetch", "-q"]);
    assert!(!git(&clone, &["branch", "-r"]).contains("origin/gone"));
    assert_eq!(
        git(&clone, &["tag", "--list", "local-only"]).trim(),
        "local-only"
    );

    // Why `fetch.pruneTags` is off: on, the same fetch deletes the tag that was never pushed.
    git(&clone, &["-c", "fetch.pruneTags=true", "fetch", "-q"]);
    assert_eq!(git(&clone, &["tag", "--list", "local-only"]), "");
}

#[test]
fn a_plain_push_sends_annotated_tags_on_the_pushed_commits_and_never_lightweight_ones() {
    if !isolated(
        "a_plain_push_sends_annotated_tags_on_the_pushed_commits_and_never_lightweight_ones",
    ) {
        return;
    }
    let world = World::new();
    let clone = world.clone_to("clone");
    apply(&clone);
    commit_file(&clone, "b.txt", "b\n", "feat: b");
    git(&clone, &["tag", "-a", "-m", "release one", "v1"]);
    git(&clone, &["tag", "light"]);
    git(&clone, &["-c", "push.followTags=false", "push", "-q"]);
    assert_eq!(git(&world.up(), &["tag", "--list"]), "");
    commit_file(&clone, "c.txt", "c\n", "feat: c");
    git(&clone, &["tag", "-a", "-m", "release two", "v2"]);
    git(&clone, &["push", "-q"]);
    assert_eq!(git(&world.up(), &["tag", "--list"]), "v1\nv2\n");
}

#[test]
fn simple_pushes_only_to_an_upstream_of_the_same_name() {
    if !isolated("simple_pushes_only_to_an_upstream_of_the_same_name") {
        return;
    }
    let world = World::new();
    let clone = world.clone_to("clone");
    apply(&clone);
    git(&clone, &["switch", "-q", "-c", "topic"]);
    commit_file(&clone, "b.txt", "b\n", "feat: b");
    git(&clone, &["push", "-q", "-u", "origin", "topic:other"]);
    commit_file(&clone, "c.txt", "c\n", "feat: c");
    let message = refused(&clone, &["push"]);
    assert!(
        message.contains("upstream branch of your current branch does not match"),
        "git said: {message}"
    );
    // The documented alternative sends it; the refusal is `simple`'s doing.
    git(&clone, &["-c", "push.default=upstream", "push", "-q"]);
    // Same name: a bare push works.
    git(&clone, &["push", "-q", "-u", "origin", "topic"]);
    commit_file(&clone, "d.txt", "d\n", "feat: d");
    git(&clone, &["push", "-q"]);
}

/// `base` merged with two branches that edit the middle line differently and the outer lines
/// identically; the conflicted `file.txt` after `git merge`, run with `extra` options.
fn conflicted_file(extra: &[&str]) -> String {
    let dir = tempfile::tempdir().expect("repository");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    commit_file(p, "file.txt", "start\nmid\nend\n", "base");
    git(p, &["switch", "-q", "-c", "theirs"]);
    commit_file(p, "file.txt", "START\ntheirs\nEND\n", "theirs");
    git(p, &["switch", "-q", "main"]);
    commit_file(p, "file.txt", "START\nours\nEND\n", "ours");
    let mut args = extra.to_vec();
    args.extend(["merge", "-q", "theirs"]);
    refused(p, &args);
    fs::read_to_string(p.join("file.txt")).expect("read")
}

#[test]
fn conflicts_show_the_original_and_leave_out_the_lines_both_sides_share() {
    if !isolated("conflicts_show_the_original_and_leave_out_the_lines_both_sides_share") {
        return;
    }
    identity();
    let scratch = tempfile::tempdir().expect("scratch");
    apply(scratch.path());
    let recommended = conflicted_file(&[]);
    if at_least((2, 35)) {
        // zdiff3: the original is shown, and the lines both sides share stay outside the markers.
        assert!(
            recommended.starts_with("START\n<<<<<<< HEAD\nours\n||||||| "),
            "{recommended}"
        );
        assert!(
            recommended.ends_with("\n=======\ntheirs\n>>>>>>> theirs\nEND\n"),
            "{recommended}"
        );
        // diff3 keeps them inside the conflict.
        assert!(
            conflicted_file(&["-c", "merge.conflictStyle=diff3"])
                .starts_with("<<<<<<< HEAD\nSTART\n")
        );
    } else {
        assert!(
            recommended.starts_with("<<<<<<< HEAD\nSTART\n"),
            "{recommended}"
        );
    }
    assert!(!conflicted_file(&["-c", "merge.conflictStyle=merge"]).contains("|||||||"));
}

#[test]
fn new_repositories_start_on_main() {
    if !isolated("new_repositories_start_on_main") {
        return;
    }
    let scratch = tempfile::tempdir().expect("scratch");
    apply(scratch.path());
    git(scratch.path(), &["init", "-q", "fresh"]);
    assert_eq!(
        git(&scratch.path().join("fresh"), &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/main"
    );
}

/// The messages left by a commit, an amend and a squash in an interactive rebase when the editor
/// keeps git's template and adds a `#123` line and a subject above it, with `config` as `-c`
/// options and `env` added to the environment. Git's own global file is replaced by `global`.
/// Unix only: the editors are `sh` and `sed -i.bak` scripts.
#[cfg(unix)]
fn edited_messages(config: &[&str], global: &Path) -> Vec<String> {
    let dir = tempfile::tempdir().expect("repository");
    let p = dir.path();
    let script = p.join("editor.sh");
    fs::write(
        &script,
        "{ printf '#123 keep me\\nsubject line\\n\\n'; cat \"$1\"; } > \"$1.new\" && mv \"$1.new\" \"$1\"\n",
    )
    .expect("editor");
    let editor = format!("sh '{}'", script.display());
    let global = global.to_string_lossy();
    let env = [
        ("GIT_EDITOR", editor.as_str()),
        ("GIT_SEQUENCE_EDITOR", "sed -i.bak 2s/^pick/squash/"),
        ("GIT_CONFIG_GLOBAL", global.as_ref()),
        ("GIT_AUTHOR_NAME", "Ada Lovelace"),
        ("GIT_AUTHOR_EMAIL", "ada@example.com"),
        ("GIT_COMMITTER_NAME", "Ada Lovelace"),
        ("GIT_COMMITTER_EMAIL", "ada@example.com"),
    ];
    let with = |args: &[&str]| {
        let all: Vec<&str> = config.iter().chain(args).copied().collect();
        let out = run(p, &all, &env);
        assert!(
            out.status.success(),
            "git {all:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let message = || git(p, &["log", "-1", "--format=%B"]);
    git(p, &["init", "-q", "-b", "main"]);
    let mut messages = Vec::new();
    fs::write(p.join("a.txt"), "a\n").expect("write");
    git(p, &["add", "a.txt"]);
    with(&["commit", "-q"]);
    messages.push(message());
    with(&["commit", "-q", "--amend"]);
    messages.push(message());
    for (file, subject) in [("b.txt", "four"), ("c.txt", "five")] {
        fs::write(p.join(file), "x\n").expect("write");
        git(p, &["add", file]);
        with(&["commit", "-q", "-m", subject]);
    }
    with(&["rebase", "-q", "-i", "HEAD~2"]);
    messages.push(message());
    messages
}

#[cfg(unix)]
#[test]
fn the_recommended_commit_cleanup_is_gits_own_and_whitespace_leaks_the_editor_template() {
    if !isolated(
        "the_recommended_commit_cleanup_is_gits_own_and_whitespace_leaks_the_editor_template",
    ) {
        return;
    }
    identity();
    let scratch = tempfile::tempdir().expect("scratch");
    apply(scratch.path());
    let empty = scratch.path().join("empty-global");
    fs::write(&empty, "").expect("empty file");

    // Written globally, the recommendation changes nothing for any message written in an editor,
    // by `git commit`, `--amend` or a squash in `rebase -i`: it is what git does without it.
    let recommended = edited_messages(&[], &global());
    assert_eq!(recommended, edited_messages(&[], &empty));
    for message in &recommended {
        assert!(
            !message.contains("# Please enter") && !message.contains("# This is a combination"),
            "template left in {message:?}"
        );
    }

    // `whitespace`, the earlier recommendation, keeps `#` lines in every message, git's own
    // template included (`commit.cleanup` in git-config: "you will have to remove the help lines
    // ... yourself").
    let whitespace = edited_messages(&["-c", "commit.cleanup=whitespace"], &empty);
    assert!(
        whitespace[0].contains("# Please enter the commit message"),
        "{}",
        whitespace[0]
    );
    assert!(
        whitespace[1].contains("# Please enter the commit message"),
        "{}",
        whitespace[1]
    );
    assert!(
        whitespace[2].contains("# This is a combination of 2 commits"),
        "{}",
        whitespace[2]
    );
    // `scissors` drops the template of a commit but not the squash's.
    let scissors = edited_messages(&["-c", "commit.cleanup=scissors"], &empty);
    assert!(!scissors[0].contains("# Please enter"), "{}", scissors[0]);
    assert!(
        scissors[2].contains("# This is a combination of 2 commits"),
        "{}",
        scissors[2]
    );

    // Canopy's own commit is never edited: with the recommendation written, it keeps the `#`
    // line the user typed and gives what `git commit -F` gives.
    let repo_dir = tempfile::tempdir().expect("repository");
    let twin_dir = tempfile::tempdir().expect("twin");
    let message = "fix: tab\n\n# Notes\n";
    let msg_file = scratch.path().join("message");
    fs::write(&msg_file, message).expect("message");
    for dir in [repo_dir.path(), twin_dir.path()] {
        git(dir, &["init", "-q", "-b", "main"]);
        fs::write(dir.join("a.txt"), "a\n").expect("write");
        git(dir, &["add", "a.txt"]);
    }
    git(
        twin_dir.path(),
        &["commit", "-q", "-F", &msg_file.to_string_lossy()],
    );
    let repo = Repo::open(repo_dir.path()).expect("open");
    repo.set_options(GitOptions::default());
    repo.commit(message, false, false).expect("commit");
    let canopy = git(repo_dir.path(), &["log", "-1", "--format=%B"]);
    assert_eq!(canopy, git(twin_dir.path(), &["log", "-1", "--format=%B"]));
    assert!(canopy.contains("# Notes"));
}

#[test]
fn submodules_follow_a_checkout_and_a_push_becomes_on_demand() {
    if !isolated("submodules_follow_a_checkout_and_a_push_becomes_on_demand") {
        return;
    }
    let world = World::new();
    let root = world.root.path();
    let sub_url = root.join("sub.git");
    git(root, &["init", "-q", "--bare", "-b", "main", "sub.git"]);
    let sub_work = world.clone_to("sub-seed");
    git(
        &sub_work,
        &["remote", "set-url", "origin", &sub_url.to_string_lossy()],
    );
    git(&sub_work, &["push", "-q", "origin", "main"]);
    let work = world.clone_to("super");
    git(
        &work,
        &["submodule", "-q", "add", &sub_url.to_string_lossy(), "m"],
    );
    git(&work, &["commit", "-q", "-m", "add m"]);
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["switch", "-q", "-c", "other"]);
    let m = work.join("m");
    git(&m, &["switch", "-q", "-c", "work"]);
    commit_file(&m, "a.txt", "changed\n", "s2");
    git(&m, &["push", "-q", "origin", "work"]);
    git(&work, &["add", "m"]);
    git(&work, &["commit", "-q", "-m", "bump m"]);
    git(&work, &["push", "-q", "-u", "origin", "other"]);
    git(&work, &["switch", "-q", "main"]);
    git(&work, &["submodule", "-q", "update"]);
    apply(&work);

    // Without it a checkout leaves the submodule behind, and status says so; with it the
    // submodule follows.
    git(
        &work,
        &["-c", "submodule.recurse=false", "switch", "-q", "other"],
    );
    assert_eq!(git(&work, &["status", "--porcelain"]), " M m\n");
    git(&work, &["switch", "-q", "main"]);
    git(&work, &["switch", "-q", "other"]);
    assert_eq!(git(&m, &["log", "-1", "--format=%s"]), "s2\n");
    assert_eq!(git(&work, &["status", "--porcelain"]), "");

    // The documented side effect (git-config, `push.recurseSubmodules`: "unless submodule.recurse
    // is set, in which case a true value means on-demand"): a push also pushes the submodule
    // commits it needs, with the superproject's branch name, and is aborted where it cannot.
    commit_file(&m, "a.txt", "unpushed\n", "s3");
    git(&work, &["add", "m"]);
    git(&work, &["commit", "-q", "-m", "bump m again"]);
    let remote_before = git(&work, &["ls-remote", "origin", "other"]);
    refused(&work, &["push"]);
    assert_eq!(git(&work, &["ls-remote", "origin", "other"]), remote_before);
    git(&work, &["push", "-q", "--no-recurse-submodules"]);
    assert_ne!(git(&work, &["ls-remote", "origin", "other"]), remote_before);
    // With a branch of the same name in the submodule, the on-demand push goes through.
    commit_file(&m, "a.txt", "unpushed again\n", "s4");
    git(&m, &["switch", "-q", "-c", "other"]);
    git(&work, &["add", "m"]);
    git(&work, &["commit", "-q", "-m", "bump m a third time"]);
    git(&work, &["push", "-q"]);
    assert!(git(&sub_url, &["branch", "--list", "other"]).contains("other"));
}

/// A superproject whose branch `other` records a commit of submodule `m` that is on no remote.
/// The submodule is on branch `work` (`how` = "branch"), detached ("detached") or on a branch
/// named like the superproject's ("same name"). Returns the superproject and the submodule's
/// remote.
fn superproject_with_unpushed_submodule(world: &World, how: &str) -> (PathBuf, PathBuf) {
    let root = world.root.path();
    let sub_url = root.join("sub.git");
    git(root, &["init", "-q", "--bare", "-b", "main", "sub.git"]);
    let sub_work = world.clone_to("sub-seed");
    git(
        &sub_work,
        &["remote", "set-url", "origin", &sub_url.to_string_lossy()],
    );
    git(&sub_work, &["push", "-q", "origin", "main"]);
    let work = world.clone_to("super");
    git(
        &work,
        &["submodule", "-q", "add", &sub_url.to_string_lossy(), "m"],
    );
    git(&work, &["commit", "-q", "-m", "add m"]);
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["switch", "-q", "-c", "other"]);
    git(&work, &["push", "-q", "-u", "origin", "other"]);
    let m = work.join("m");
    match how {
        "detached" => git(&m, &["switch", "-q", "--detach"]),
        "same name" => git(&m, &["switch", "-q", "-c", "other"]),
        _ => git(&m, &["switch", "-q", "-c", "work"]),
    };
    commit_file(&m, "a.txt", "unpushed\n", "s2");
    git(&work, &["add", "m"]);
    git(&work, &["commit", "-q", "-m", "bump m"]);
    (work, sub_url)
}

/// A push of a commit that records a submodule commit no remote has stops with git's own message
/// naming the submodule (`--recurse-submodules=check`), whether `submodule.recurse` is on (where a
/// bare git push would try an on-demand push and fail on the branch name) or off (where it would
/// publish a pointer nobody can fetch), and whichever way the submodule is checked out. Nothing
/// reaches the remote; once the submodule's commit is pushed, the push goes through.
#[test]
fn a_push_stops_for_a_submodule_commit_no_remote_has() {
    if !isolated("a_push_stops_for_a_submodule_commit_no_remote_has") {
        return;
    }
    for recurse in ["true", "false"] {
        for how in ["branch", "detached", "same name"] {
            let world = World::new();
            let (work, sub_url) = superproject_with_unpushed_submodule(&world, how);
            git(&work, &["config", "submodule.recurse", recurse]);
            let remote_before = git(&work, &["ls-remote", "origin", "other"]);
            let oracle = refused(&work, &["push", "--recurse-submodules=check"]);
            assert_eq!(git(&work, &["ls-remote", "origin", "other"]), remote_before);

            let repo = Repo::open(&work).expect("open");
            let err = repo
                .push(false, |_| {})
                .expect_err("the submodule commit is on no remote")
                .to_string();
            assert!(
                err.contains("not be found on any remote") && err.contains("\n  m\n"),
                "recurse {recurse}, {how}: {err}"
            );
            assert!(
                oracle.contains("not be found on any remote") && oracle.contains("\n  m\n"),
                "{oracle}"
            );
            assert_eq!(
                git(&work, &["ls-remote", "origin", "other"]),
                remote_before,
                "recurse {recurse}, {how}: nothing is pushed"
            );
            // A new branch and a tag stop the same way.
            git(&work, &["branch", "fresh"]);
            repo.publish_branch("fresh", "origin", |_| {})
                .expect_err("a new branch records it too");
            git(&work, &["tag", "-a", "-m", "v", "v1"]);
            repo.push_tag("origin", "v1", |_| {})
                .expect_err("a tag's commits record it too");
            assert_eq!(git(&work, &["ls-remote", "--tags", "origin"]), "");

            // The submodule's commit on a remote: the push goes through, as git's does.
            git(
                &work.join("m"),
                &["push", "-q", "origin", "HEAD:refs/heads/kept"],
            );
            repo.push(false, |_| {})
                .expect("push after the submodule's");
            assert_eq!(
                git(&work, &["rev-parse", "other"]),
                git(&work, &["rev-parse", "origin/other"]),
                "recurse {recurse}, {how}"
            );
            assert!(git(&sub_url, &["branch", "--list", "kept"]).contains("kept"));
        }
    }
}

/// The configuration decides where it sets `push.recurseSubmodules` and Canopy follows it, as it
/// does for every choice; not following it, Canopy's check applies whatever it says.
#[test]
fn push_recurse_submodules_in_the_configuration_decides_while_followed() {
    if !isolated("push_recurse_submodules_in_the_configuration_decides_while_followed") {
        return;
    }
    for (value, follow, pushes) in [
        ("no", true, true),
        ("no", false, false),
        ("check", true, false),
        ("on-demand", true, false),
    ] {
        let world = World::new();
        let (work, _) = superproject_with_unpushed_submodule(&world, "branch");
        git(&work, &["config", "submodule.recurse", "true"]);
        git(&work, &["config", "push.recurseSubmodules", value]);
        let repo = Repo::open(&work).expect("open");
        repo.set_options(GitOptions {
            follow_config: follow,
            ..GitOptions::default()
        });
        let git_alone = run(&work, &["push", "--dry-run"], &[]).status.success();
        let result = repo.push(false, |_| {});
        assert_eq!(
            result.is_ok(),
            pushes,
            "push.recurseSubmodules={value}, follow {follow}: {result:?}"
        );
        if follow {
            assert_eq!(result.is_ok(), git_alone, "git's own push: {value}");
        }
    }
}

#[test]
fn changing_line_endings_is_not_neutral_for_files_already_committed_with_crlf() {
    if !isolated("changing_line_endings_is_not_neutral_for_files_already_committed_with_crlf") {
        return;
    }
    identity();
    let dir = tempfile::tempdir().expect("repository");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    commit_file(p, "crlf.txt", "one\r\ntwo\r\n", "feat: windows file");
    assert_eq!(git(p, &["status", "--porcelain"]), "");
    // The recommended `input` stores LF: the next time git looks at the file it differs, and a
    // commit of it rewrites every line.
    git(p, &["config", "core.autocrlf", "input"]);
    git(p, &["add", "--renormalize", "crlf.txt"]);
    assert_eq!(git(p, &["status", "--porcelain"]), "M  crlf.txt\n");
}
