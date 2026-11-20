//! What a repository's own files make `git` run on this computer (issue #28).
//!
//! A repository handed over as a folder (an archive, a copied tree) brings its `.git/config`,
//! `.git/hooks` and attributes along, and `git` obeys them: hooks run on a commit or a checkout,
//! a `filter.<name>.clean` command on a stage, a merge driver on a merge, `core.sshCommand` and
//! credential helpers on a network command, `gpg.program` on a signature. The application refuses
//! to act on such a repository until the user trusts its folder; [`Repo::programs`] names what
//! the repository would run, so the refusal can say which program source it is about.
//!
//! Only the repository's own configuration counts (repository and worktree level), never the
//! user's own: what the user configured globally is theirs to run.

use std::path::{Path, PathBuf};

use git2::{ConfigLevel, ErrorCode, Repository};

use crate::cli;

/// What [`programs`] reports when it could not read the configuration: nothing can be said to
/// run no program then.
pub const UNREADABLE: &str = "a configuration that could not be read";

/// The permission bits (owner, group, other) that let a file run.
#[cfg(unix)]
const EXECUTE_BITS: u32 = 0o111;

/// The hooks git runs by name (`githooks`): a file with another name there is never run.
const HOOK_NAMES: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
];

/// The names of the configuration keys (repository and worktree level) and of the hooks of
/// `repo` that make git run a program, sorted and without repeats. Empty for a repository whose
/// own files name none.
///
/// A configuration key is named as written (a credential section by its URL with any password
/// removed); a hook as `hook <file name>`.
pub fn programs(repo: &Repository) -> Vec<String> {
    let mut found = Vec::new();
    let mut own_hooks_path = None;
    let mut user_hooks_path = false;
    match repo.config() {
        Ok(config) => {
            for level in [ConfigLevel::Local, ConfigLevel::Worktree] {
                let config = match config.open_level(level) {
                    Ok(config) => config,
                    // A repository with no worktree configuration is normal.
                    Err(e) if e.code() == ErrorCode::NotFound => continue,
                    Err(_) => {
                        found.push(UNREADABLE.to_owned());
                        continue;
                    }
                };
                let Ok(entries) = config.entries(None) else {
                    found.push(UNREADABLE.to_owned());
                    continue;
                };
                // A walk that stops on an entry it cannot read has not seen the rest.
                let walked = entries.for_each(|entry| {
                    if let Ok(name) = entry.name() {
                        let value = String::from_utf8_lossy(entry.value_bytes());
                        if name.eq_ignore_ascii_case("core.hookspath") {
                            own_hooks_path = Some(value.to_string());
                        }
                        if runs_a_program(name, &value) {
                            found.push(cli::redact_url(name));
                        }
                    }
                });
                if walked.is_err() {
                    found.push(UNREADABLE.to_owned());
                }
            }
            // A hooks directory the user's own configuration names holds the user's hooks.
            user_hooks_path =
                own_hooks_path.is_none() && config.get_entry("core.hookspath").is_ok();
            if let Some(raw) = &own_hooks_path {
                let expanded = config
                    .get_path("core.hookspath")
                    .unwrap_or_else(|_| PathBuf::from(raw));
                let base = repo.workdir().unwrap_or_else(|| repo.path());
                found.extend(hooks(&base.join(expanded)));
            }
        }
        Err(_) => found.push(UNREADABLE.to_owned()),
    }
    if own_hooks_path.is_none() && !user_hooks_path {
        found.extend(hooks(&repo.commondir().join("hooks")));
    }
    found.sort();
    found.dedup();
    found
}

/// The hooks in `dir` git would run: a file named after a hook that git can execute (git skips
/// one that is not executable, and the samples `git init` writes carry another name).
fn hooks(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|n| HOOK_NAMES.contains(&n))
        })
        .filter(|entry| is_runnable(&entry.path()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .map(|name| format!("hook {name}"))
        .collect()
}

/// Whether `path` is a file git would execute as a hook.
fn is_runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & EXECUTE_BITS != 0
    }
    // Git for Windows has no execute bit: it runs any file it finds.
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Whether git treats `value` as a switch rather than a command (`core.fsmonitor=true` is the
/// built-in daemon, `core.pager=false` no pager).
fn is_switch(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "1" | "0" | ""
    )
}

/// Whether the configuration key `name` (section and variable lower-cased by libgit2, the
/// subsection as written) set to `value` makes git run a program.
fn runs_a_program(name: &str, value: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let (section, rest) = name.split_once('.').unwrap_or((&name, ""));
    let variable = rest.rsplit('.').next().unwrap_or_default();
    let has_subsection = rest.contains('.');
    match (section, has_subsection, variable) {
        (
            "core",
            false,
            "sshcommand" | "gitproxy" | "hookspath" | "askpass" | "editor" | "alternaterefscommand",
        )
        | ("sequence", false, "editor")
        | ("diff", false, "external")
        | ("diff", true, "textconv" | "command")
        | ("filter", true, "clean" | "smudge" | "process")
        | ("merge", true, "driver")
        | ("credential", _, "helper")
        | ("gpg", false, "program")
        | ("gpg", true, "program" | "defaultkeycommand")
        | ("remote", true, "uploadpack" | "receivepack" | "vcs")
        | ("trailer", true, "command" | "cmd")
        // A hook configured instead of a file (git 2.54 and later).
        | ("hook", true, "command") => true,
        ("core", false, "fsmonitor" | "pager") | ("pager", _, _) => !is_switch(value),
        // Only the command form of a submodule's update runs a program.
        ("submodule", true, "update") => value.starts_with('!'),
        // The `ext::` transport runs the command in the URL.
        ("remote", true, "url" | "pushurl") => value.starts_with("ext::"),
        // libgit2 does not evaluate `hasconfig:` conditions but git does: what such an include
        // brings in (`core.sshCommand`, a credential helper) would never be seen here.
        ("includeif", ..) => rest.starts_with("hasconfig:"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::isolate_git;

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_repository_names_the_programs_its_own_files_run() {
        isolate_git!(a_repository_names_the_programs_its_own_files_run);
        let root = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(root.path()).expect("init");
        assert!(programs(&repo).is_empty(), "a plain repository");
        let mut config = repo.config().expect("config");
        config
            .set_str("remote.origin.url", "git@example.com:a/b.git")
            .expect("url");
        config.set_str("user.name", "Ada").expect("name");
        config.set_str("core.fsmonitor", "true").expect("daemon");
        config.set_str("core.pager", "false").expect("pager");
        config
            .set_str("submodule.s.update", "checkout")
            .expect("update");
        config.set_str("filter.lfs.required", "true").expect("lfs");
        assert!(programs(&repo).is_empty(), "settings that run nothing");

        let keys = [
            ("core.sshCommand", "touch ran"),
            ("core.fsmonitor", "./monitor"),
            ("core.pager", "less"),
            ("core.editor", "vim"),
            ("sequence.editor", "vim"),
            ("diff.external", "difft"),
            ("diff.md.textconv", "cat"),
            ("filter.lfs.clean", "cat"),
            ("filter.lfs.smudge", "cat"),
            ("merge.x.driver", "touch ran"),
            ("submodule.s.update", "!touch ran"),
            ("remote.origin.uploadpack", "touch ran"),
            ("remote.up.url", "ext::touch ran"),
            ("credential.https://example.com.helper", "!touch ran"),
            ("gpg.program", "touch ran"),
            ("gpg.ssh.program", "touch ran"),
            ("core.hooksPath", "hooks"),
            ("hook.lint.command", "touch ran"),
        ];
        for (key, value) in keys {
            config.set_str(key, value).expect(key);
        }
        let listed = programs(&repo);
        for (key, _) in keys {
            let key = key.to_ascii_lowercase();
            assert!(
                listed.iter().any(|k| k.to_ascii_lowercase() == key),
                "{key} in {listed:?}"
            );
        }
        config
            .set_str("submodule.s.update", "checkout")
            .expect("reset");
        assert!(
            !programs(&repo).iter().any(|k| k == "submodule.s.update"),
            "only the command form of a submodule update is a program"
        );
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn an_include_only_git_evaluates_and_the_hooks_are_named() {
        isolate_git!(an_include_only_git_evaluates_and_the_hooks_are_named);
        let root = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(root.path()).expect("init");
        let config_file = repo.path().join("config");
        let mut text = std::fs::read_to_string(&config_file).expect("config");
        text.push_str("[includeIf \"hasconfig:remote.*.url:ssh://**\"]\n\tpath = evil.inc\n");
        std::fs::write(&config_file, text).expect("write config");
        let repo = Repository::open(root.path()).expect("open");
        assert!(
            programs(&repo)
                .iter()
                .any(|k| k.starts_with("includeif.hasconfig:")),
            "{:?}",
            programs(&repo)
        );

        // A machine whose own configuration redirects every repository's hooks runs none of
        // these.
        if repo
            .config()
            .expect("config")
            .get_entry("core.hookspath")
            .is_ok()
        {
            return;
        }
        let hooks = repo.path().join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks");
        std::fs::write(hooks.join("pre-commit.sample"), "").expect("sample");
        assert!(!programs(&repo).iter().any(|k| k.contains("pre-commit")));
        for hook in ["pre-commit", "post-index-change", "reference-transaction"] {
            executable(&hooks.join(hook));
        }
        let listed = programs(&repo);
        for hook in ["pre-commit", "post-index-change", "reference-transaction"] {
            assert!(
                listed.iter().any(|k| *k == format!("hook {hook}")),
                "{hook} in {listed:?}"
            );
        }
    }

    /// Only a file git would run is a hook: by git's names, and executable.
    #[test]
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn only_files_git_runs_as_hooks_are_named() {
        isolate_git!(only_files_git_runs_as_hooks_are_named);
        let root = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(root.path()).expect("init");
        if repo
            .config()
            .expect("config")
            .get_entry("core.hookspath")
            .is_ok()
        {
            return;
        }
        let hooks = repo.path().join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks");
        std::fs::write(hooks.join("README"), "notes").expect("readme");
        executable(&hooks.join("helper.sh"));
        std::fs::write(hooks.join("pre-push"), "").expect("not executable");
        std::fs::create_dir(hooks.join("pre-rebase")).expect("directory");
        assert!(programs(&repo).is_empty(), "{:?}", programs(&repo));
        executable(&hooks.join("commit-msg"));
        assert_eq!(programs(&repo), ["hook commit-msg"]);
    }

    /// `core.hooksPath` moves the hooks: git runs those of the named directory (relative to the
    /// working tree) and none in `.git/hooks`.
    #[test]
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_hooks_path_replaces_the_hooks_directory() {
        isolate_git!(a_hooks_path_replaces_the_hooks_directory);
        let root = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(root.path()).expect("init");
        let default = repo.path().join("hooks");
        std::fs::create_dir_all(&default).expect("hooks");
        executable(&default.join("pre-commit"));
        let elsewhere = root.path().join("tools").join("hooks");
        std::fs::create_dir_all(&elsewhere).expect("dir");
        executable(&elsewhere.join("post-merge"));
        repo.config()
            .expect("config")
            .set_str("core.hooksPath", "tools/hooks")
            .expect("path");
        let listed = programs(&repo);
        assert!(listed.iter().any(|k| k == "hook post-merge"), "{listed:?}");
        assert!(
            listed
                .iter()
                .any(|k| k.eq_ignore_ascii_case("core.hookspath"))
        );
        assert!(!listed.iter().any(|k| k == "hook pre-commit"), "{listed:?}");
    }

    /// Marks `path` as a hook git can run.
    fn executable(path: &Path) {
        std::fs::write(path, "#!/bin/sh\n").expect("hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        }
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_credential_url_is_named_without_its_password() {
        isolate_git!(a_credential_url_is_named_without_its_password);
        let root = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(root.path()).expect("init");
        repo.config()
            .expect("config")
            .set_str("credential.https://ada:secret@example.com.helper", "!x")
            .expect("helper");
        let listed = programs(&repo);
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert!(!listed[0].contains("secret"), "{listed:?}");
    }
}

/// The reads that run `git` without being a user's action (blame, a file's history, the change
/// search, the insights): each runs in a repository that is not trusted, so none may start a
/// program the repository's own files name (#28). Every source has a control: ordinary git,
/// run as a terminal would, starts the program, so the missing marker means something.
#[cfg(test)]
#[cfg(unix)]
mod reads {
    use crate::test_support::isolate_git;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use crate::{MissingObjects, Repo};

    #[expect(
        clippy::disallowed_methods,
        reason = "the control runs ordinary git to show that the fixture's program does run"
    )]
    fn terminal_git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .output()
            .expect("git runs")
            .status
            .success()
    }

    /// A repository with a file `a.txt` changed in two commits and once more in the working
    /// tree, whose attributes name the drivers `probe` for filter and diff, and the directory
    /// that holds the markers.
    struct Fixture {
        dir: tempfile::TempDir,
        repo: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let repo = dir.path().join("repo");
            std::fs::create_dir(&repo).expect("repo");
            let fixture = Self { dir, repo };
            fixture.git(&["init", "-q", "-b", "main"]);
            fixture.git(&["config", "commit.gpgsign", "false"]);
            std::fs::write(
                fixture.repo.join(".gitattributes"),
                "a.txt filter=probe diff=probe\n",
            )
            .expect("attributes");
            for (n, text) in [(1, "one\n"), (2, "one\ntwo needle\n")] {
                std::fs::write(fixture.repo.join("a.txt"), text).expect("write");
                fixture.git(&["add", "."]);
                fixture.git(&["commit", "-q", "-m", &format!("commit {n}")]);
            }
            std::fs::write(fixture.repo.join("a.txt"), "one\ntwo needle\nthree\n").expect("write");
            fixture
        }

        fn git(&self, args: &[&str]) {
            assert!(terminal_git(&self.repo, args), "git {args:?}");
        }

        fn marker(&self, label: &str) -> PathBuf {
            self.dir.path().join(format!("ran-{label}"))
        }

        /// A program that leaves `label`'s marker and then copies its input (or file) through.
        fn program(&self, label: &str) -> String {
            // A name without spaces, which a shell would split on.
            let slug = label.replace(' ', "-");
            let path = self.dir.path().join(format!("program-{slug}"));
            let text = format!(
                "#!/bin/sh\ntouch '{}'\nif [ -n \"$1\" ]; then cat \"$1\"; else cat; fi\n",
                self.marker(label).display()
            );
            std::fs::write(&path, text).expect("script");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
            path.to_string_lossy().into_owned()
        }

        fn ran(&self, label: &str) -> bool {
            self.marker(label).exists()
        }

        fn reset(&self, label: &str) {
            let _ = std::fs::remove_file(self.marker(label));
        }

        /// Arms `key` with a program for `label`, shows that `control` (ordinary git) runs it,
        /// then returns the repository opened, with the marker removed again.
        fn armed(&self, label: &str, key: &str, control: &[&str]) -> Repo {
            let program = self.program(label);
            self.git(&["config", key, &program]);
            assert!(!self.ran(label), "{label}: arming ran it");
            terminal_git(&self.repo, control);
            assert!(
                self.ran(label),
                "{label}: ordinary git never ran it, so the case proves nothing"
            );
            self.reset(label);
            Repo::open(&self.repo).expect("open")
        }
    }

    #[test]
    fn blame_runs_no_textconv() {
        isolate_git!(blame_runs_no_textconv);
        let f = Fixture::new();
        let repo = f.armed(
            "textconv blame",
            "diff.probe.textconv",
            &["blame", "--porcelain", "HEAD", "--", "a.txt"],
        );
        let head = repo.git().head().expect("head").target();
        assert!(head.is_some());
        repo.blame("a.txt", head, MissingObjects::Fail, false)
            .expect("blame a commit");
        assert!(!f.ran("textconv blame"), "blame ran the textconv program");
    }

    #[test]
    fn a_working_tree_blame_runs_no_clean_filter_while_untrusted() {
        isolate_git!(a_working_tree_blame_runs_no_clean_filter_while_untrusted);
        let f = Fixture::new();
        let repo = f.armed(
            "clean blame",
            "filter.probe.clean",
            &["blame", "--porcelain", "--", "a.txt"],
        );
        let refused = repo
            .blame("a.txt", None, MissingObjects::Fail, false)
            .expect_err("refused");
        assert!(refused.to_string().contains("filter"), "{refused}");
        assert!(!f.ran("clean blame"), "blame ran the clean filter");
    }

    #[test]
    fn the_change_search_runs_no_textconv() {
        isolate_git!(the_change_search_runs_no_textconv);
        let f = Fixture::new();
        let repo = f.armed(
            "textconv search",
            "diff.probe.textconv",
            &["log", "-Gneedle", "--name-only"],
        );
        let found = repo
            .search_changes(None, "needle", false, 10)
            .expect("search");
        assert_eq!(found.hits.len(), 1, "the search finds the commit");
        let found = repo
            .search_changes(None, "needle", true, 10)
            .expect("search");
        assert_eq!(found.hits.len(), 1, "the search finds the commit");
        assert!(
            !f.ran("textconv search"),
            "the search ran the textconv program"
        );
    }

    /// A lock-in, not a regression test: `git log --follow --name-status` runs neither program
    /// even without the two flags (only patch output does, which the controls ask for), so this
    /// passes with or without them. It fails when a change to the history's command makes git
    /// print patches or otherwise run a diff program.
    #[test]
    fn a_files_history_keeps_running_neither_external_diff_nor_textconv() {
        isolate_git!(a_files_history_keeps_running_neither_external_diff_nor_textconv);
        for (label, key, control) in [
            (
                "textconv history",
                "diff.probe.textconv",
                &["log", "--follow", "-p", "--", "a.txt"][..],
            ),
            (
                "external diff history",
                "diff.external",
                &["log", "--follow", "-p", "--ext-diff", "--", "a.txt"][..],
            ),
        ] {
            let f = Fixture::new();
            let repo = f.armed(label, key, control);
            let history = repo.file_history(None, "a.txt", 10).expect("history");
            assert_eq!(history.entries.len(), 2, "{label}");
            assert!(!f.ran(label), "{label}: the history ran it");
        }
    }

    #[test]
    fn the_insights_run_no_program() {
        isolate_git!(the_insights_run_no_program);
        let f = Fixture::new();
        let repo = f.armed(
            "fsmonitor insights",
            "core.fsmonitor",
            &["status", "--porcelain"],
        );
        repo.repository_metrics().expect("metrics");
        repo.branch_metrics().expect("branches");
        assert!(
            !f.ran("fsmonitor insights"),
            "the insights ran core.fsmonitor"
        );
    }
}
