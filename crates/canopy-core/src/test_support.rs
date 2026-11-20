//! Runs a unit test that uses Git with its own Git configuration, as `tests/support` does for the
//! integration tests (#256): the test runs again in a child process with a private home, an empty
//! global configuration and no inherited `GIT_*` variable, so the developer's own configuration
//! (hooks, signing, aliases, line endings) cannot change its result.

use std::fs;
use std::process::Command;

const ISOLATED: &str = "CANOPY_TEST_ISOLATED_GIT";

/// Starts the body only in the isolated child. Use the name of the calling test function.
macro_rules! isolate_git {
    ($test:ident) => {
        if !$crate::test_support::in_isolated_git_env(&$crate::test_support::test_name(
            module_path!(),
            stringify!($test),
        )) {
            return;
        }
    };
}

pub(crate) use isolate_git;

/// The name libtest knows the test `name` by, from the `module_path!()` it is declared in: the
/// path without the crate's own name (`cli::tests::name`).
pub(crate) fn test_name(module_path: &str, name: &str) -> String {
    match module_path.split_once("::") {
        Some((_, module)) => format!("{module}::{name}"),
        None => name.to_owned(),
    }
}

/// Whether this is the isolated child of `test`; otherwise runs it and fails unless it passed.
///
/// Git's global file (`GIT_CONFIG_GLOBAL`) does not exist in the private home. libgit2's two
/// files, `.gitconfig` and the XDG `git/config`, are written empty on purpose: libgit2 reads the
/// first one it finds, so they win over the real profile's files on Windows too.
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs itself again in a child process"
)]
pub(crate) fn in_isolated_git_env(test: &str) -> bool {
    if std::env::var_os(ISOLATED).is_some_and(|value| value == test) {
        return true;
    }
    let home = tempfile::tempdir().expect("private home");
    fs::write(home.path().join(".gitconfig"), "").expect("write libgit2 global config");
    let xdg_git = home.path().join("git");
    fs::create_dir(&xdg_git).expect("create XDG git directory");
    fs::write(xdg_git.join("config"), "").expect("write empty XDG config");
    let mut child = Command::new(std::env::current_exe().expect("test binary"));
    child.args([
        test,
        "--exact",
        "--include-ignored",
        "--test-threads=1",
        "--nocapture",
    ]);
    // Any inherited `GIT_*` variable, or `EMAIL`, can change what git does.
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("GIT_") || upper == "EMAIL" {
            child.env_remove(name);
        }
    }
    let out = child
        .env(ISOLATED, test)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.path().join("gitconfig"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .output()
        .expect("child test runs");
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
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_test_is_named_by_its_module_path_without_the_crate() {
        assert_eq!(
            test_name("canopy_core::cli::tests", "runs"),
            "cli::tests::runs"
        );
        assert_eq!(test_name("canopy_core", "runs"), "runs");
    }
}
