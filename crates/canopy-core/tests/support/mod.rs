//! Run each integration test with its own Git configuration.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

const ISOLATED: &str = "CANOPY_TEST_ISOLATED_GIT";

/// Start the body only in the isolated child. Use the libtest name of the calling test.
///
/// The test runs again in a child process without any inherited `GIT_*` variable, with a private
/// home and an empty global Git configuration. `before_isolation` names a callback that runs in
/// every process other than that child, so a test can first start an intermediate process with
/// its own environment (see `commits_ignore_inherited_global_hooks`). The callback returns true
/// when the isolated child should then be started from that process.
///
/// libgit2 still reads the system-level configuration (`/etc/gitconfig`, and on Windows Git for
/// Windows' `etc\gitconfig` and `%PROGRAMDATA%\Git\config`): `GIT_CONFIG_NOSYSTEM` stops only
/// Git itself, and hiding the file from libgit2 needs `git2::opts::set_search_path`, which is
/// `unsafe`.
macro_rules! isolate_git {
    ($test:ident $(, before_isolation = $setup:path)?) => {
        $(
            if !$crate::support::is_git_isolated(stringify!($test)) && !$setup(stringify!($test)) {
                return;
            }
        )?
        $crate::support::isolate_git!($test, "", &[]);
    };
    ($test:ident, $home_config:expr, $env:expr) => {
        if !$crate::support::in_isolated_git_env(stringify!($test), $home_config, $env) {
            return;
        }
    };
}

pub(crate) use isolate_git;

#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the test runs itself again in a child process"
)]
pub(crate) fn test_command(test: &str, home: &Path) -> Command {
    let mut child = Command::new(std::env::current_exe().expect("test binary"));
    child.args([
        test,
        "--exact",
        "--include-ignored",
        "--test-threads=1",
        "--nocapture",
    ]);
    remove_inherited_git_variables(&mut child, std::env::vars_os().map(|(name, _)| name));
    child
        // Git ignores the system-level file, and never prompts on a terminal.
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home);
    child
}

/// Remove every inherited `GIT_*` variable (names are case-insensitive on Windows) and `EMAIL`,
/// which Git uses as an identity. Any of them can change what Git does: `GIT_TEMPLATE_DIR`
/// installs hooks, `GIT_NAMESPACE` moves refs, `GIT_DIFF_OPTS` reshapes patches, `GIT_ATTR_SOURCE`
/// reads attributes from a commit. A list of known names would miss the next one.
pub(crate) fn remove_inherited_git_variables(
    child: &mut Command,
    names: impl IntoIterator<Item = OsString>,
) {
    for name in names {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("GIT_") || upper == "EMAIL" {
            child.env_remove(name);
        }
    }
}

pub(crate) fn is_git_isolated(test: &str) -> bool {
    std::env::var_os(ISOLATED).is_some_and(|value| value == test)
}

#[expect(
    clippy::expect_used,
    clippy::print_stderr,
    reason = "show child diagnostics in the parent test"
)]
pub(crate) fn assert_test_passed(test: &str, child: &mut Command) {
    let out = child.output().expect("child test runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success()
            && stdout
                .lines()
                .any(|line| line.starts_with("test result: ok. 1 passed;")),
        "{test}:\n{stdout}{stderr}"
    );
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
}

/// Set the environment before the test body runs so fixture commands and commands started by
/// Canopy both inherit it. Git's global file (`GIT_CONFIG_GLOBAL`, `gitconfig`) does not exist
/// in the private home. libgit2's two files, `.gitconfig` and the XDG `git/config`, are written
/// on purpose: it reads the first one it finds, looking in `%HOME%` and `%XDG_CONFIG_HOME%`
/// first, so even when empty they win over the real profile's files on Windows. Tests of Git
/// configuration can populate the files without affecting other tests.
///
/// `home_config` supplies the content of `.gitconfig` when a test needs one.
#[expect(
    clippy::expect_used,
    reason = "a failure to set up the child fails the test"
)]
pub(crate) fn in_isolated_git_env(test: &str, home_config: &str, env: &[(&str, &str)]) -> bool {
    if is_git_isolated(test) {
        return true;
    }

    let home = tempfile::tempdir().expect("private home");
    fs::write(home.path().join(".gitconfig"), home_config).expect("write libgit2 global config");
    // Without this file libgit2 falls back to the real profile's AppData on Windows.
    let xdg_git = home.path().join("git");
    fs::create_dir(&xdg_git).expect("create XDG git directory");
    fs::write(xdg_git.join("config"), "").expect("write empty XDG config");
    assert_test_passed(
        test,
        test_command(test, home.path())
            .env(ISOLATED, test)
            .envs(env.iter().copied()),
    );
    false
}
