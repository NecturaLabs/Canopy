//! Run each integration test with its own Git configuration.

use std::fs;
use std::process::Command;

/// Start the body only in the isolated child. Use the libtest name of the calling test.
macro_rules! isolate_git {
    ($test:ident) => {
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
pub(crate) fn test_command(test: &str) -> Command {
    let mut child = Command::new(std::env::current_exe().expect("test binary"));
    child.args([test, "--exact", "--test-threads=1", "--nocapture"]);
    child
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
        out.status.success() && stdout.contains("1 passed"),
        "{test}:\n{stdout}{stderr}"
    );
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
}

/// Set the environment before the test body runs so fixture commands and commands started by
/// Canopy both inherit it. The global file starts absent in a private directory. Tests of Git
/// configuration can populate it without affecting other tests.
///
/// `home_config` supplies libgit2's separate global configuration when a test needs one.
#[expect(
    clippy::expect_used,
    reason = "a failure to set up the child fails the test"
)]
pub(crate) fn in_isolated_git_env(test: &str, home_config: &str, env: &[(&str, &str)]) -> bool {
    const ISOLATED: &str = "CANOPY_TEST_ISOLATED_GIT";
    if std::env::var_os(ISOLATED).is_some_and(|value| value == test) {
        return true;
    }

    let home = tempfile::tempdir().expect("private home");
    fs::write(home.path().join(".gitconfig"), home_config).expect("write libgit2 global config");
    // On Windows libgit2 looks in AppData if it cannot find an XDG config.
    let xdg_git = home.path().join("git");
    fs::create_dir(&xdg_git).expect("create XDG git directory");
    fs::write(xdg_git.join("config"), "").expect("write empty XDG config");
    let mut child = test_command(test);
    child
        .env(ISOLATED, test)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.path().join("gitconfig"))
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path());
    for key in [
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "EMAIL",
    ] {
        child.env_remove(key);
    }
    assert_test_passed(test, child.envs(env.iter().copied()));
    false
}
