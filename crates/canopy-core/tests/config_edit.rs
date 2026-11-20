//! Git configuration reads and writes against isolated files and the real Git executable.

#![expect(
    clippy::expect_used,
    reason = "integration fixture helpers may panic when setup fails"
)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use support::isolate_git;

use canopy_core::{
    Repo, cli, config, config::Setting, global_pull_config, identity::Scope, ops::PullStrategy,
};

fn global() -> PathBuf {
    PathBuf::from(std::env::var_os("GIT_CONFIG_GLOBAL").expect("private global config"))
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("repository");
    cli::run(dir.path(), &["init", "-q", "-b", "main"]).expect("init");
    for (key, value) in [
        ("user.name", "Ada"),
        ("user.email", "ada@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        cli::run(dir.path(), &["config", "--local", key, value]).expect("identity");
    }
    dir
}

fn included(dir: &Path, content: &str) -> PathBuf {
    let included = global().with_file_name("included");
    fs::write(&included, content).expect("include");
    cli::run(dir, &["config", "--global", "include.path", "included"]).expect("include path");
    included
}

#[test]
fn global_reads_expand_includes_and_preserve_their_origin() {
    isolate_git!(global_reads_expand_includes_and_preserve_their_origin);
    let dir = fixture();
    let file = included(dir.path(), "[fetch]\nprune = true\n");
    let entries =
        config::read(dir.path(), true, config::Query::Get("fetch.prune")).expect("global config");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value.as_deref(), Some("true"));
    assert_eq!(entries[0].scope, "global");
    assert_eq!(entries[0].file.as_ref(), Some(&file));
}

#[test]
fn global_pull_reads_numeric_booleans_from_includes() {
    isolate_git!(global_pull_reads_numeric_booleans_from_includes);
    let dir = fixture();
    included(dir.path(), "[pull]\nrebase = -2\n");
    assert_eq!(
        global_pull_config(dir.path())
            .expect("pull config")
            .strategy,
        Some(PullStrategy::Rebase)
    );
}

#[test]
fn init_rejects_invalid_configured_branch_before_creating_the_target() {
    isolate_git!(init_rejects_invalid_configured_branch_before_creating_the_target);
    let dir = tempfile::tempdir().expect("scratch");
    cli::run(
        dir.path(),
        &["config", "--global", "init.defaultBranch", "bad name"],
    )
    .expect("invalid configured branch");
    let target = dir.path().join("new");
    Repo::init(&target, "main", true).expect_err("invalid configured branch");
    assert!(
        !target.exists(),
        "invalid configuration created a directory"
    );
}

#[test]
fn settings_edit_only_the_selected_scope_and_reset_reveals_included_values() {
    isolate_git!(settings_edit_only_the_selected_scope_and_reset_reveals_included_values);

    let dir = fixture();
    let file = included(dir.path(), "[fetch]\nprune = false\n");
    let original = fs::read(&file).expect("include contents");
    config::set(dir.path(), Scope::Global, Setting::FetchPrune, Some("true")).expect("global edit");
    config::set(dir.path(), Scope::Local, Setting::FetchPrune, Some("false"))
        .expect("repository edit");
    let effective = config::read(dir.path(), false, config::Query::Get("fetch.prune"))
        .expect("effective value");
    assert_eq!(effective[0].scope, "local");
    assert_eq!(effective[0].value.as_deref(), Some("false"));
    config::set(dir.path(), Scope::Local, Setting::FetchPrune, None).expect("repository reset");
    let effective = config::read(dir.path(), false, config::Query::Get("fetch.prune"))
        .expect("effective value");
    assert_eq!(effective[0].scope, "global");
    assert_eq!(effective[0].value.as_deref(), Some("true"));
    config::set(dir.path(), Scope::Global, Setting::FetchPrune, None).expect("global reset");
    config::set(dir.path(), Scope::Global, Setting::FetchPrune, None).expect("repeated reset");
    let effective =
        config::read(dir.path(), false, config::Query::Get("fetch.prune")).expect("included value");
    assert_eq!(effective[0].value.as_deref(), Some("false"));
    assert_eq!(effective[0].file.as_ref(), Some(&file));
    assert_eq!(fs::read(file).expect("include contents"), original);
}

#[test]
fn recommendations_replace_old_global_choices_leave_sensitive_values_and_allow_later_edits() {
    isolate_git!(
        recommendations_replace_old_global_choices_leave_sensitive_values_and_allow_later_edits
    );

    let dir = fixture();
    for (key, value) in [
        ("pull.rebase", "true"),
        ("pull.ff", "false"),
        ("core.autocrlf", "input"),
        ("commit.gpgsign", "true"),
        ("gpg.format", "ssh"),
        ("user.name", "User Person"),
        ("core.hooksPath", "user-hooks"),
    ] {
        cli::run(dir.path(), &["config", "--global", key, value]).expect("old global choice");
    }
    config::set(
        dir.path(),
        Scope::Local,
        Setting::PushDefault,
        Some("current"),
    )
    .expect("local choice");
    config::apply_recommended(dir.path()).expect("apply recommendations");
    for setting in Setting::ALL {
        if let Some(expected) = setting.recommended_write() {
            let out = cli::run(dir.path(), &["config", "--global", "--get", setting.key()])
                .expect("recommended value");
            assert_eq!(out.trim(), expected, "{}", setting.key());
        }
    }
    for (key, expected) in [
        ("core.autocrlf", "input"),
        ("commit.gpgsign", "true"),
        ("gpg.format", "ssh"),
        ("user.name", "User Person"),
        ("core.hooksPath", "user-hooks"),
    ] {
        let out =
            cli::run(dir.path(), &["config", "--global", "--get", key]).expect("preserved value");
        assert_eq!(out.trim(), expected, "{key}");
    }
    assert_eq!(
        global_pull_config(dir.path())
            .expect("pull configuration")
            .strategy,
        Some(PullStrategy::FastForwardOnly)
    );
    assert_eq!(
        cli::run(dir.path(), &["config", "--get", "push.default"])
            .expect("local choice")
            .trim(),
        "current"
    );
    config::set(
        dir.path(),
        Scope::Global,
        Setting::CommitCleanup,
        Some("scissors"),
    )
    .expect("subsequent edit");
    assert_eq!(
        cli::run(
            dir.path(),
            &["config", "--global", "--get", "commit.cleanup"]
        )
        .expect("saved choice")
        .trim(),
        "scissors"
    );
}

#[test]
fn invalid_settings_and_branch_names_never_change_files() {
    isolate_git!(invalid_settings_and_branch_names_never_change_files);

    let dir = fixture();
    for (setting, value) in [
        (Setting::FetchPrune, "yes\n[core]\nhooksPath=attack"),
        (Setting::PushDefault, "--global"),
        // Git accepts these and the editor shows them where the user set them, but Canopy never
        // writes them (#261).
        (Setting::PushDefault, "matching"),
        (Setting::PushDefault, "nothing"),
        (Setting::SigningFormat, "program"),
        (Setting::CommitCleanup, "truncate"),
        (Setting::InitialBranch, "bad name"),
        (Setting::InitialBranch, "@{-1}"),
        (Setting::InitialBranch, "HEAD"),
        (Setting::InitialBranch, "-topic"),
        (Setting::InitialBranch, ""),
    ] {
        let before = fs::read(dir.path().join(".git/config")).expect("before");
        assert!(
            config::set(dir.path(), Scope::Local, setting, Some(value)).is_err(),
            "{setting:?} accepted {value:?}"
        );
        assert_eq!(
            fs::read(dir.path().join(".git/config")).expect("after"),
            before
        );
        assert!(!global().exists());
    }
    config::set(
        dir.path(),
        Scope::Local,
        Setting::InitialBranch,
        Some("release/1.x"),
    )
    .expect("valid branch");
    assert_eq!(
        cli::run(
            dir.path(),
            &["config", "--local", "--get", "init.defaultBranch"]
        )
        .expect("saved branch")
        .trim(),
        "release/1.x"
    );
}

#[test]
fn direct_reads_exclude_includes_and_worktree_overrides() {
    isolate_git!(direct_reads_exclude_includes_and_worktree_overrides);
    let dir = fixture();
    included(dir.path(), "[fetch]\nprune = false\n");
    assert_eq!(
        config::read_direct(dir.path(), Scope::Global, config::Query::Get("fetch.prune"))
            .expect("direct globals"),
        Vec::<config::Entry>::new()
    );
    config::set(dir.path(), Scope::Global, Setting::FetchPrune, Some("true")).expect("global edit");
    let entries = config::read_direct(dir.path(), Scope::Global, config::Query::Get("fetch.prune"))
        .expect("direct globals");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].file.as_ref(), Some(&global()));
    assert_eq!(entries[0].value.as_deref(), Some("true"));
    let local_include = dir.path().join("included-local");
    fs::write(&local_include, "[fetch]\nprune = false\n").expect("include");
    cli::run(
        dir.path(),
        &["config", "--local", "include.path", "../included-local"],
    )
    .expect("local include");
    assert_eq!(
        config::read_direct(dir.path(), Scope::Local, config::Query::Get("fetch.prune"))
            .expect("direct local"),
        Vec::<config::Entry>::new()
    );
    config::set(dir.path(), Scope::Local, Setting::FetchPrune, Some("true")).expect("local edit");
    cli::run(dir.path(), &["config", "extensions.worktreeConfig", "true"])
        .expect("enable worktree config");
    cli::run(
        dir.path(),
        &["config", "--worktree", "fetch.prune", "false"],
    )
    .expect("worktree override");
    assert_eq!(
        config::read(dir.path(), false, config::Query::Get("fetch.prune"))
            .expect("effective config")[0]
            .scope,
        "worktree"
    );
    assert_eq!(
        config::read_direct(dir.path(), Scope::Local, config::Query::Get("fetch.prune"))
            .expect("direct local")[0]
            .value
            .as_deref(),
        Some("true")
    );
    config::set(dir.path(), Scope::Local, Setting::FetchPrune, None).expect("reset local");
    assert_eq!(
        config::read(dir.path(), false, config::Query::Get("fetch.prune"))
            .expect("effective config")[0]
            .value
            .as_deref(),
        Some("false")
    );
}

#[test]
fn init_reads_config_for_the_new_repository_instead_of_its_parent() {
    isolate_git!(init_reads_config_for_the_new_repository_instead_of_its_parent);
    let dir = fixture();
    cli::run(
        dir.path(),
        &["config", "--global", "init.defaultBranch", "global-branch"],
    )
    .expect("global initial branch");
    cli::run(
        dir.path(),
        &["config", "--local", "init.defaultBranch", "bad outer name"],
    )
    .expect("parent configuration");
    let nested = Repo::init(&dir.path().join("nested"), "fallback", true)
        .expect("parent repository configuration must not affect init");
    assert_eq!(
        nested.head().expect("HEAD").branch.as_deref(),
        Some("global-branch")
    );
}

#[test]
fn init_follows_conditional_global_config_for_its_destination() {
    isolate_git!(init_follows_conditional_global_config_for_its_destination);
    let dir = tempfile::tempdir().expect("scratch");
    let target = dir.path().join("conditional");
    let file = global().with_file_name("conditional");
    fs::write(&file, "[init]\ndefaultBranch = destination\n").expect("conditional config");
    // Git matches `gitdir:` against the real path of the repository it creates (`/var` is
    // `/private/var` on macOS; Windows' short 8.3 names are expanded), as a pattern in which `\\`
    // escapes: so the folder is resolved and written with `/`, as git writes it on Windows too.
    let real = fs::canonicalize(dir.path())
        .expect("real path")
        .display()
        .to_string();
    let real = real
        .strip_prefix(r"\\?\")
        .unwrap_or(&real)
        .replace('\\', "/");
    let gitdir = format!("{real}/conditional/.git");
    let condition = format!("includeIf.gitdir:{gitdir}.path");
    cli::run(
        dir.path(),
        &["config", "--global", &condition, "conditional"],
    )
    .expect("conditional include");
    let repo = Repo::init(&target, "fallback", true).expect("initialize destination");
    assert_eq!(
        repo.head().expect("HEAD").branch.as_deref(),
        Some("destination")
    );
}

#[test]
fn all_canonical_values_roundtrip_and_write_errors_retain_git_stderr() {
    isolate_git!(all_canonical_values_roundtrip_and_write_errors_retain_git_stderr);
    let dir = fixture();
    for setting in Setting::ALL {
        for &value in setting.values() {
            if setting == Setting::ConflictStyle
                && value == "zdiff3"
                && cli::version_number().is_none_or(|version| version < (2, 35))
            {
                continue;
            }
            config::set(dir.path(), Scope::Local, setting, Some(value)).expect("canonical value");
            let out = cli::run(dir.path(), &["config", "--local", "--get", setting.key()])
                .expect("saved canonical value");
            assert_eq!(out.trim(), value, "{}", setting.key());
        }
    }
    let lock = global().with_extension("lock");
    fs::write(&lock, "another writer").expect("lock global config");
    let error = config::apply_recommended(dir.path()).expect_err("locked configuration");
    let text = error.to_string();
    assert!(text.contains("Saved 0"), "{text}");
    assert!(text.contains("pull.rebase"), "{text}");
    assert!(text.contains("File exists"), "{text}");
    assert!(!global().exists());
    assert_eq!(
        fs::read_to_string(lock).expect("lock preserved"),
        "another writer"
    );
}

#[test]
fn init_reinitialization_preserves_existing_unborn_and_committed_heads() {
    isolate_git!(init_reinitialization_preserves_existing_unborn_and_committed_heads);
    let dir = fixture();
    for committed in [false, true] {
        if committed {
            cli::run(
                dir.path(),
                &["commit", "-q", "--allow-empty", "-m", "Initial"],
            )
            .expect("commit");
        }
        let head = cli::run(dir.path(), &["symbolic-ref", "HEAD"]).expect("HEAD");
        Repo::init(dir.path(), "other", true).expect("reinitialize");
        assert_eq!(
            cli::run(dir.path(), &["symbolic-ref", "HEAD"]).expect("HEAD"),
            head
        );
    }
}

#[test]
fn boolean_reads_use_git_semantics_and_keep_included_origins() {
    isolate_git!(boolean_reads_use_git_semantics_and_keep_included_origins);
    let dir = fixture();
    let file = included(dir.path(), "[fetch]\nprune = -2\n");
    for (line, expected) in [
        ("prune = -2", "true"),
        ("prune", "true"),
        ("prune =", "false"),
        ("prune = no", "false"),
    ] {
        fs::write(&file, format!("[fetch]\n{line}\n")).expect("boolean config");
        let entry = config::read_bool(dir.path(), true, "fetch.prune")
            .expect("boolean value")
            .expect("configured");
        assert_eq!(entry.value.as_deref(), Some(expected), "{line}");
        assert_eq!(entry.file.as_ref(), Some(&file));
        assert_eq!(entry.scope, "global");
    }
    fs::write(&file, "[fetch]\nprune = invalid\n").expect("invalid boolean");
    let error = config::read_bool(dir.path(), true, "fetch.prune").expect_err("invalid boolean");
    assert!(error.to_string().contains("fetch.prune"), "{error}");
    assert!(
        config::read_bool(dir.path(), true, "missing.key")
            .expect("missing key")
            .is_none()
    );
}

#[test]
fn invalid_submodule_boolean_is_reported_instead_of_silently_disabling_updates() {
    isolate_git!(invalid_submodule_boolean_is_reported_instead_of_silently_disabling_updates);
    let dir = fixture();
    cli::run(dir.path(), &["config", "submodule.recurse", "sometimes"]).expect("invalid boolean");
    let repo = Repo::open(dir.path()).expect("repository");
    let error = repo
        .follow_submodules(|_| {})
        .expect_err("invalid config must be reported");
    assert!(error.to_string().contains("submodule.recurse"), "{error}");
}

#[test]
fn recommendations_report_later_include_overrides_without_rewriting_the_include() {
    isolate_git!(recommendations_report_later_include_overrides_without_rewriting_the_include);
    let dir = fixture();
    cli::run(dir.path(), &["config", "--global", "fetch.prune", "true"])
        .expect("direct setting before include");
    let file = included(dir.path(), "[fetch]\nprune = false\n");
    let original = fs::read(&file).expect("include before");
    let error =
        config::apply_recommended(dir.path()).expect_err("include still overrides recommendation");
    let text = error.to_string();
    assert!(text.contains("fetch.prune"), "{text}");
    assert!(text.contains("included"), "{text}");
    assert_eq!(fs::read(&file).expect("include after"), original);
    assert_eq!(
        config::read_bool(dir.path(), true, "fetch.prune")
            .expect("effective global")
            .expect("configured")
            .value
            .as_deref(),
        Some("false")
    );
    assert_eq!(
        config::read_direct(dir.path(), Scope::Global, config::Query::Get("fetch.prune"))
            .expect("direct global")[0]
            .value
            .as_deref(),
        Some("true")
    );
}

#[test]
fn values_shown_but_not_offered_are_no_choices() {
    isolate_git!(values_shown_but_not_offered_are_no_choices);
    let dir = fixture();
    for value in Setting::PushDefault.shown_only() {
        let error = config::set(dir.path(), Scope::Local, Setting::PushDefault, Some(value))
            .expect_err("never written")
            .to_string();
        assert_eq!(
            error,
            format!("Canopy does not write push.default = {value}, though git accepts it.")
        );
    }
    for setting in Setting::ALL {
        for shown in setting.shown_only() {
            assert!(!setting.values().contains(shown), "{}", setting.key());
        }
    }
    assert_eq!(
        Setting::PushDefault.values(),
        ["simple", "upstream", "current"]
    );
    assert_eq!(Setting::PushDefault.shown_only(), ["matching", "nothing"]);
}
