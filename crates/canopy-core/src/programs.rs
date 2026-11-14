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

use std::path::Path;

use git2::{ConfigLevel, ErrorCode, Repository};

use crate::cli;

/// What [`programs`] reports when it could not read the configuration: nothing can be said to
/// run no program then.
pub const UNREADABLE: &str = "a configuration that could not be read";

/// The hook files git does not run: the samples `git init` writes.
const SAMPLE_SUFFIX: &str = ".sample";

/// The names of the configuration keys (repository and worktree level) and of the hooks of
/// `repo` that make git run a program, sorted and without repeats. Empty for a repository whose
/// own files name none.
///
/// A configuration key is named as written (a credential section by its URL with any password
/// removed); a hook as `hook <file name>`.
pub fn programs(repo: &Repository) -> Vec<String> {
    let mut found = Vec::new();
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
                    if let Ok(name) = entry.name()
                        && runs_a_program(name, &String::from_utf8_lossy(entry.value_bytes()))
                    {
                        found.push(cli::redact_url(name));
                    }
                });
                if walked.is_err() {
                    found.push(UNREADABLE.to_owned());
                }
            }
        }
        Err(_) => found.push(UNREADABLE.to_owned()),
    }
    found.extend(hooks(&repo.commondir().join("hooks")));
    found.sort();
    found.dedup();
    found
}

/// The hooks in `dir`: every file git could run, which is any file there but the samples.
fn hooks(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.ends_with(SAMPLE_SUFFIX))
        .map(|name| format!("hook {name}"))
        .collect()
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
        | ("trailer", true, "command" | "cmd") => true,
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

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_repository_names_the_programs_its_own_files_run() {
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

        let hooks = repo.path().join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks");
        std::fs::write(hooks.join("pre-commit.sample"), "").expect("sample");
        assert!(!programs(&repo).iter().any(|k| k.contains("pre-commit")));
        for hook in ["pre-commit", "post-index-change", "reference-transaction"] {
            std::fs::write(hooks.join(hook), "").expect("hook");
        }
        let listed = programs(&repo);
        for hook in ["pre-commit", "post-index-change", "reference-transaction"] {
            assert!(
                listed.iter().any(|k| *k == format!("hook {hook}")),
                "{hook} in {listed:?}"
            );
        }
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its repository with libgit2"
    )]
    fn a_credential_url_is_named_without_its_password() {
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

    #[test]
    fn a_files_history_runs_no_external_diff_and_no_textconv() {
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
