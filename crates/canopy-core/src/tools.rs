//! The programs besides `git` that a repository needs, and how to install the ones that are
//! missing.
//!
//! Canopy runs `git`, and `git` in turn runs `git-lfs` for a repository that stores files with
//! LFS, `ssh` for an SSH remote and `gpg`, `gpgsm` or `ssh-keygen` for signing. When one is not
//! on the `PATH` (a macOS app opened from the Finder gets a minimal one), git's own error comes
//! late and says little; [`Repo::missing_tools`] finds them as the repository is opened.

use std::path::{Path, PathBuf};

use crate::{Repo, cli};

/// A program a repository needs and the system lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingTool {
    /// The program git looks for, as it names it (`git-lfs`, `ssh`, `gpg`).
    pub program: String,
    /// What this repository needs it for.
    pub needed_for: &'static str,
    /// How to install it on this system, one step each.
    pub install: Vec<String>,
}

/// What `program` is part of, for the installation steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Package {
    GitLfs,
    OpenSsh,
    GnuPg,
}

impl Package {
    /// The package that carries a program named by git's configuration or defaults; custom
    /// programs read as `GnuPG`'s, the usual reason to configure one.
    fn of(program: &str) -> Self {
        let name = Path::new(program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(program);
        match name {
            "git-lfs" => Self::GitLfs,
            "ssh" | "ssh-keygen" => Self::OpenSsh,
            _ => Self::GnuPg,
        }
    }

    fn steps(self) -> Vec<String> {
        let (linux, mac, windows): (&str, &str, &str) = match self {
            Self::GitLfs => (
                "Install git-lfs with your package manager (sudo pacman -S git-lfs, sudo apt install git-lfs or sudo dnf install git-lfs), then run git lfs install once.",
                "Run brew install git-lfs, then git lfs install.",
                "Run winget install --id GitHub.GitLFS, or reinstall Git for Windows, which includes it.",
            ),
            Self::OpenSsh => (
                "Install OpenSSH with your package manager: sudo pacman -S openssh, sudo apt install openssh-client or sudo dnf install openssh-clients.",
                "macOS ships ssh in /usr/bin; run xcode-select --install if it is gone.",
                "Turn on OpenSSH Client under Settings, Optional features, or reinstall Git for Windows.",
            ),
            Self::GnuPg => (
                "Install GnuPG with your package manager: sudo pacman -S gnupg, sudo apt install gnupg or sudo dnf install gnupg2.",
                "Run brew install gnupg.",
                "Run winget install --id GnuPG.Gpg4win.",
            ),
        };
        let mut steps = vec![
            if cfg!(target_os = "macos") {
                mac
            } else if cfg!(target_os = "windows") {
                windows
            } else {
                linux
            }
            .to_owned(),
        ];
        if cfg!(target_os = "macos") {
            steps.push(
                "Apps opened from the Finder do not get your shell's PATH: if it is installed but not found, start Canopy from a terminal."
                    .to_owned(),
            );
        } else {
            steps.push("Then restart Canopy so it sees the new PATH.".to_owned());
        }
        steps
    }
}

/// Whether `program` is an executable file: given by path, or found in one of `dirs`.
fn exists_in(program: &str, dirs: &[PathBuf]) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return is_executable(path);
    }
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat", ".com"]
    } else {
        &[""]
    };
    dirs.iter().any(|dir| {
        extensions
            .iter()
            .any(|ext| is_executable(&dir.join(format!("{program}{ext}"))))
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Whether `program` is on the `PATH` Canopy (and so git) runs with.
fn on_path(program: &str) -> bool {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    exists_in(program, &dirs)
}

/// Whether `url` is an SSH remote: `ssh://`, `git+ssh://` or the `[user@]host:path` form (not a
/// local path, and not a Windows drive).
fn is_ssh_url(url: &str) -> bool {
    let url = url.trim();
    if let Some((scheme, _)) = url.split_once("://") {
        return matches!(scheme, "ssh" | "git+ssh" | "ssh+git");
    }
    let Some((host, _)) = url.split_once(':') else {
        return false;
    };
    !host.is_empty()
        && host.len() > 1
        && !host.contains(['/', '\\'])
        && !url.starts_with(['/', '.', '~'])
}

impl Repo {
    /// The programs this repository needs that `has` does not find: `git-lfs` when its
    /// attributes use `filter=lfs`, `ssh` for an SSH remote (unless `core.sshCommand`, `GIT_SSH`
    /// or `GIT_SSH_COMMAND` names another), and the program that signs when `commit.gpgsign` or
    /// `tag.gpgsign` is on. Reads configuration and two small files; run it off the UI thread.
    fn missing_tools_with(&self, has: &dyn Fn(&str) -> bool) -> Vec<MissingTool> {
        let mut missing = Vec::new();
        let mut need = |program: &str, needed_for: &'static str| {
            if !has(program) && !missing.iter().any(|m: &MissingTool| m.program == program) {
                missing.push(MissingTool {
                    program: program.to_owned(),
                    needed_for,
                    install: Package::of(program).steps(),
                });
            }
        };

        if self.uses_lfs() {
            need("git-lfs", "this repository stores files with Git LFS");
        }
        if !cfg!(windows)
            && self.has_ssh_remote()
            && self.config_string("core.sshCommand").is_none()
            && std::env::var_os("GIT_SSH").is_none()
            && std::env::var_os("GIT_SSH_COMMAND").is_none()
        {
            need("ssh", "a remote of this repository uses SSH");
        }
        let signs = |key: &str| {
            self.git()
                .config()
                .and_then(|c| c.get_bool(key))
                .unwrap_or(false)
        };
        if signs("commit.gpgsign") || signs("tag.gpgsign") {
            let format = self
                .config_string("gpg.format")
                .unwrap_or_else(|| "openpgp".into());
            let (key, default) = match format.as_str() {
                "ssh" => ("gpg.ssh.program", "ssh-keygen"),
                "x509" => ("gpg.x509.program", "gpgsm"),
                _ => ("gpg.openpgp.program", "gpg"),
            };
            let program = self
                .config_string(key)
                .or_else(|| {
                    if key == "gpg.openpgp.program" {
                        self.config_string("gpg.program")
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| default.into());
            need(&program, "commits or tags of this repository are signed");
        }
        missing
    }

    /// [`Repo::missing_tools_with`] against the `PATH` Canopy runs with.
    pub fn missing_tools(&self) -> Vec<MissingTool> {
        self.missing_tools_with(&|program| {
            if program == "git-lfs" {
                git_lfs_available(self.workdir())
            } else {
                on_path(program)
            }
        })
    }

    /// Whether `.gitattributes` in the working folder or `info/attributes` names `filter=lfs`.
    fn uses_lfs(&self) -> bool {
        let attributes = [
            self.workdir().join(".gitattributes"),
            self.git().commondir().join("info").join("attributes"),
        ];
        attributes.iter().any(|file| {
            std::fs::read_to_string(file)
                .is_ok_and(|text| text.lines().any(|l| l.contains("filter=lfs")))
        })
    }

    fn has_ssh_remote(&self) -> bool {
        let Ok(names) = self.git().remotes() else {
            return false;
        };
        names
            .iter()
            .flatten()
            .flatten()
            .filter_map(|name| self.git().find_remote(name).ok())
            .any(|remote| remote.url().is_ok_and(is_ssh_url))
    }
}

/// Whether `git lfs` works: the `git-lfs` program is a git subcommand, found on the `PATH` or in
/// git's exec path.
pub fn git_lfs_available(workdir: &Path) -> bool {
    cli::run_unchecked(workdir, &["lfs", "version"]).is_ok_and(|out| out.code == Some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_remotes_are_told_from_local_paths() {
        for url in [
            "ssh://git@host/r.git",
            "git+ssh://host/r",
            "git@github.com:o/r.git",
            "host:path/r.git",
        ] {
            assert!(is_ssh_url(url), "{url}");
        }
        for url in [
            "https://github.com/o/r.git",
            "/srv/git/r.git",
            "./r",
            "../r",
            "C:\\repos\\r",
            "C:/repos/r",
            "file:///srv/r",
            "",
        ] {
            assert!(!is_ssh_url(url), "{url}");
        }
    }

    #[test]
    fn a_program_is_found_by_path_or_in_the_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool = dir.path().join("fake-tool");
        std::fs::write(&tool, "#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dirs = [dir.path().to_path_buf()];
            assert!(!exists_in("fake-tool", &dirs), "not executable yet");
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("mode");
            assert!(exists_in("fake-tool", &dirs));
            assert!(exists_in(&tool.to_string_lossy(), &[]), "given by path");
            assert!(!exists_in("other-tool", &dirs));
            assert!(!exists_in(
                &dir.path().join("nope").to_string_lossy(),
                &dirs
            ));
        }
    }

    fn repo_in(dir: &Path) -> Repo {
        Repo::init(dir, "main", false).expect("init")
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test sets configuration with libgit2"
    )]
    fn a_repository_names_the_programs_it_needs_and_the_system_lacks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = repo_in(dir.path());
        let nothing = |_: &str| false;
        assert!(
            repo.missing_tools_with(&nothing).is_empty(),
            "a plain repository needs nothing"
        );

        std::fs::write(
            dir.path().join(".gitattributes"),
            "*.bin filter=lfs -text\n",
        )
        .expect("attributes");
        let mut config = repo.git().config().expect("config");
        config
            .set_str("remote.origin.url", "git@example.com:o/r.git")
            .expect("url");
        config.set_bool("commit.gpgsign", true).expect("sign");
        config.set_str("gpg.format", "ssh").expect("format");
        let wanted = |m: &[MissingTool]| m.iter().map(|t| t.program.clone()).collect::<Vec<_>>();

        let all = repo.missing_tools_with(&nothing);
        let expected: &[&str] = if cfg!(windows) {
            &["git-lfs", "ssh-keygen"]
        } else {
            &["git-lfs", "ssh", "ssh-keygen"]
        };
        assert_eq!(wanted(&all), expected);
        assert!(
            all.iter()
                .all(|t| !t.install.is_empty() && !t.needed_for.is_empty())
        );
        assert!(
            repo.missing_tools_with(&|_: &str| true).is_empty(),
            "everything is installed"
        );

        config
            .set_str("gpg.ssh.program", "my-signer")
            .expect("program");
        let custom = repo.missing_tools_with(&|p: &str| p != "my-signer");
        assert!(wanted(&custom).contains(&"my-signer".to_owned()));
        config.set_str("gpg.format", "openpgp").expect("format");
        config.set_str("gpg.program", "gpg2").expect("program");
        assert!(wanted(&repo.missing_tools_with(&nothing)).contains(&"gpg2".to_owned()));
        config
            .set_str("gpg.openpgp.program", "gpg-new")
            .expect("program");
        let sign = wanted(&repo.missing_tools_with(&nothing));
        assert!(sign.contains(&"gpg-new".to_owned()) && !sign.contains(&"gpg2".to_owned()));
    }

    #[test]
    fn install_steps_name_the_package_of_the_program() {
        assert_eq!(Package::of("git-lfs"), Package::GitLfs);
        assert_eq!(Package::of("/usr/bin/ssh-keygen"), Package::OpenSsh);
        assert_eq!(Package::of("gpg"), Package::GnuPg);
        assert_eq!(Package::of("gpgsm"), Package::GnuPg);
        for package in [Package::GitLfs, Package::OpenSsh, Package::GnuPg] {
            assert!(package.steps().len() >= 2);
        }
    }
}
