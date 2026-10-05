//! The identity Git will put into the next commit, and the configuration behind it.
//!
//! A commit's author is stored in the commit object. Changing these settings affects future
//! commits only; History always displays each object's stored author.

use std::path::Path;
use std::time::Duration;

use crate::{Error, Result, cli};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Environment(&'static str),
    Config { scope: String, origin: String },
    Guessed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub value: String,
    pub source: Option<Source>,
}

impl Field {
    fn missing() -> Self {
        Self {
            value: String::new(),
            source: None,
        }
    }

    pub fn explicit(&self) -> bool {
        matches!(
            self.source,
            Some(Source::Environment(_) | Source::Config { .. })
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: Field,
    pub email: Field,
    pub usable: bool,
}

impl Person {
    pub fn explicit(&self) -> bool {
        self.name.explicit() && self.email.explicit()
    }

    pub fn looks_like_placeholder(&self) -> bool {
        let name = self.name.value.trim();
        let domain = self.email.value.rsplit_once('@').map_or("", |(_, d)| d);
        name.chars().count() < 2
            || domain.is_empty()
            || !domain.contains('.')
            || domain.ends_with("(none)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub author: Person,
    pub committer: Person,
}

impl Identity {
    /// The author and committer that `git commit` would use in `workdir` now. A missing identity
    /// is data, not an error: `usable` records whether `git var` accepted it.
    pub fn read(workdir: &Path) -> Result<Self> {
        check_repository(command(Some(workdir)))?;
        let configured = read_config(Some(workdir))?;
        Ok(Self {
            author: read_person(Some(workdir), &configured, Kind::Author)?,
            committer: read_person(Some(workdir), &configured, Kind::Committer)?,
        })
    }

    /// Global settings for the first-launch question, before a repository exists. This view
    /// excludes repository config and Git's machine-derived guess.
    pub fn read_global() -> Result<Self> {
        let configured = read_config(None)?;
        Ok(Self {
            author: read_person(None, &configured, Kind::Author)?,
            committer: read_person(None, &configured, Kind::Committer)?,
        })
    }

    pub fn warning(&self) -> Option<&'static str> {
        if !self.author.usable || !self.committer.usable {
            Some("Git cannot use this identity for a commit.")
        } else if !self.author.explicit() || !self.committer.explicit() {
            Some("Git guessed part of this identity from the computer.")
        } else if self.author.looks_like_placeholder() || self.committer.looks_like_placeholder() {
            Some("This identity looks like a placeholder.")
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    Local,
    #[default]
    Global,
}

impl Scope {
    fn flag(self) -> &'static str {
        match self {
            Self::Local => "--local",
            Self::Global => "--global",
        }
    }
}

/// How long one git call may take. The identity helper thread is shared by every tab, so a
/// repository whose configuration makes git block must not hold it for ever.
const GIT_LIMIT: Duration = Duration::from_secs(15);

fn git(workdir: Option<&Path>, args: &[&str]) -> Result<cli::Output> {
    cli::output_within(command(workdir), args, GIT_LIMIT)
}

/// Writes get longer: killing a `git config` that holds `config.lock` leaves the lock behind, which
/// would fail every later save until it is removed by hand.
const WRITE_LIMIT: Duration = Duration::from_secs(60);

fn git_write(workdir: Option<&Path>, args: &[&str]) -> Result<cli::Output> {
    cli::output_within(command(workdir), args, WRITE_LIMIT)
}

/// Change Git's `user.*` settings.
///
/// Git chooses the target global file itself. Environment and `author.*`/`committer.*` overrides
/// may still take precedence, which the next read reveals. If the email cannot be written after the
/// name was, the name is put back.
pub fn set(workdir: Option<&Path>, scope: Scope, name: &str, email: &str) -> Result<()> {
    if scope == Scope::Local && workdir.is_none() {
        return Err(Error::invalid(
            "A repository is needed for local identity settings.",
        ));
    }
    let name = validate(name, "name")?;
    let email = validate(email, "email")?;
    if !email.contains('@') || email.rsplit_once('@').is_none_or(|(_, d)| d.is_empty()) {
        return Err(Error::invalid("Enter an email address with a domain."));
    }
    store(workdir, scope, name, email, &write)
}

/// How `store` runs one git call; a test substitutes one that fails.
type Writer = dyn Fn(Option<&Path>, &[&str]) -> Result<()>;

fn store(
    workdir: Option<&Path>,
    scope: Scope,
    name: &str,
    email: &str,
    put: &Writer,
) -> Result<()> {
    let flag = scope.flag();
    let previous = scope_values(workdir, scope, "user.name")?;
    put(
        workdir,
        &["config", flag, "--replace-all", "--", "user.name", name],
    )?;
    if let Err(err) = put(
        workdir,
        &["config", flag, "--replace-all", "--", "user.email", email],
    ) {
        return Err(put_back(err, workdir, scope, &previous));
    }
    Ok(())
}

/// Every value `key` has at one scope, in file order.
fn scope_values(workdir: Option<&Path>, scope: Scope, key: &str) -> Result<Vec<String>> {
    let args = ["config", scope.flag(), "-z", "--get-all", "--", key];
    let out = git(workdir, &args)?;
    match out.code {
        Some(0) => Ok(out
            .stdout
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .collect()),
        Some(1) => Ok(Vec::new()),
        _ => Err(out.error(&args)),
    }
}

/// Put `key` back to exactly the values `scope_values` read.
fn restore(workdir: Option<&Path>, scope: Scope, key: &str, previous: &[String]) -> Result<()> {
    let Some((first, rest)) = previous.split_first() else {
        return unset(workdir, scope, key);
    };
    write(
        workdir,
        &["config", scope.flag(), "--replace-all", "--", key, first],
    )?;
    for value in rest {
        write(
            workdir,
            &["config", scope.flag(), "--add", "--", key, value],
        )?;
    }
    Ok(())
}

/// Remove the values at one scope so Git falls back to the next source. Missing keys are fine.
pub fn clear(workdir: Option<&Path>, scope: Scope) -> Result<()> {
    if scope == Scope::Local && workdir.is_none() {
        return Err(Error::invalid(
            "A repository is needed for local identity settings.",
        ));
    }
    let previous = scope_values(workdir, scope, "user.name")?;
    unset(workdir, scope, "user.name")?;
    if let Err(err) = unset(workdir, scope, "user.email") {
        return Err(put_back(err, workdir, scope, &previous));
    }
    Ok(())
}

/// `err` after an attempt to put `user.name` back to `previous`; says so if that failed too.
fn put_back(err: Error, workdir: Option<&Path>, scope: Scope, previous: &[String]) -> Error {
    match restore(workdir, scope, "user.name", previous) {
        Ok(()) => err,
        Err(undo) => Error::invalid(format!(
            "{err}. The name was changed but could not be put back: {undo}"
        )),
    }
}

fn unset(workdir: Option<&Path>, scope: Scope, key: &str) -> Result<()> {
    let args = ["config", scope.flag(), "--unset-all", "--", key];
    let out = git_write(workdir, &args)?;
    if matches!(out.code, Some(0 | 5)) {
        Ok(())
    } else {
        Err(out.error(&args))
    }
}

/// A git command for `workdir`. Without one, the command must see global configuration only and
/// must not depend on the repository Canopy happened to start in (`includeIf "gitdir:..."`
/// conditions in the global file look at it), so it runs from the temporary directory.
fn command(workdir: Option<&Path>) -> std::process::Command {
    let mut cmd = cli::command(workdir);
    if workdir.is_none() {
        cmd.current_dir(std::env::temp_dir());
    }
    cmd
}

fn validate<'a>(value: &'a str, what: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>'))
    {
        return Err(Error::invalid(format!(
            "Enter a {what} without control characters or angle brackets."
        )));
    }
    Ok(value)
}

fn write(workdir: Option<&Path>, args: &[&str]) -> Result<()> {
    let out = git_write(workdir, args)?;
    if out.code == Some(0) {
        Ok(())
    } else {
        Err(out.error(args))
    }
}

/// `git var` treats a repository git refuses to read ("dubious ownership") as no repository and
/// answers from global configuration, so ask git to open the repository first and report its refusal.
fn check_repository(cmd: std::process::Command) -> Result<()> {
    let args = ["rev-parse", "--git-dir"];
    let out = cli::output_within(cmd, &args, GIT_LIMIT)?;
    if out.code == Some(0) {
        Ok(())
    } else {
        Err(out.error(&args))
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Author,
    Committer,
}

impl Kind {
    fn section(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Committer => "committer",
        }
    }

    fn ident_variable(self) -> &'static str {
        match self {
            Self::Author => "GIT_AUTHOR_IDENT",
            Self::Committer => "GIT_COMMITTER_IDENT",
        }
    }

    fn name_variable(self) -> &'static str {
        match self {
            Self::Author => "GIT_AUTHOR_NAME",
            Self::Committer => "GIT_COMMITTER_NAME",
        }
    }

    fn email_variable(self) -> &'static str {
        match self {
            Self::Author => "GIT_AUTHOR_EMAIL",
            Self::Committer => "GIT_COMMITTER_EMAIL",
        }
    }
}

/// The identity settings Git has, in the order it read them; the last entry of a key wins.
type Configured = Vec<(String, Field)>;

/// Where each value comes from, in Git's order: the `GIT_*` variables, then `author.*` or
/// `committer.*`, then `user.*`, then (for the email) `EMAIL`. `git var` then supplies the value a
/// commit would really get, including what Git guesses from the computer when nothing is set.
fn read_person(workdir: Option<&Path>, configured: &Configured, kind: Kind) -> Result<Person> {
    let section = kind.section();
    let configured = |key: String| {
        configured
            .iter()
            .rev()
            .find(|(k, _)| *k == key)
            .map(|(_, field)| field.clone())
    };
    let mut name = env_field(kind.name_variable())
        .or_else(|| configured(format!("{section}.name")))
        .or_else(|| configured("user.name".into()))
        .unwrap_or_else(Field::missing);
    let mut email = env_field(kind.email_variable())
        .or_else(|| configured(format!("{section}.email")))
        .or_else(|| configured("user.email".into()))
        .or_else(|| env_field("EMAIL").filter(|field| !field.value.is_empty()))
        .unwrap_or_else(Field::missing);
    let usable = if let Some(dir) = workdir {
        let args = ["var", kind.ident_variable()];
        let out = git(Some(dir), &args)?;
        if out.code == Some(0) {
            let value = String::from_utf8_lossy(&out.stdout);
            let (effective_name, effective_email) = parse_ident(&value)
                .ok_or_else(|| Error::invalid("Git returned an unreadable identity."))?;
            effective_name.clone_into(&mut name.value);
            effective_email.clone_into(&mut email.value);
            name.source.get_or_insert(Source::Guessed);
            email.source.get_or_insert(Source::Guessed);
            true
        } else if out.code == Some(128) {
            false
        } else {
            return Err(out.error(&args));
        }
    } else {
        !name.value.is_empty() && !email.value.is_empty()
    };
    Ok(Person {
        name,
        email,
        usable,
    })
}

fn env_field(key: &'static str) -> Option<Field> {
    std::env::var_os(key).map(|value| Field {
        value: value.to_string_lossy().into_owned(),
        source: Some(Source::Environment(key)),
    })
}

/// Every `user.*`, `author.*` and `committer.*` name and email Git knows, with the scope and file
/// each came from. `workdir` of `None` reads the global configuration only.
fn read_config(workdir: Option<&Path>) -> Result<Configured> {
    let mut args = vec!["config"];
    if workdir.is_none() {
        args.push("--global");
    }
    args.extend([
        "-z",
        "--show-scope",
        "--show-origin",
        "--get-regexp",
        "^(user|author|committer)\\.(name|email)$",
    ]);
    let out = git(workdir, &args)?;
    if out.code == Some(1) {
        return Ok(Vec::new());
    }
    if out.code != Some(0) {
        return Err(out.error(&args));
    }
    // Each entry is `scope NUL origin NUL key NL value NUL`.
    let mut parts = out.stdout.split(|b| *b == 0);
    let mut entries = Vec::new();
    while let (Some(scope), Some(origin), Some(pair)) = (parts.next(), parts.next(), parts.next()) {
        let mut pair = pair.splitn(2, |b| *b == b'\n');
        let (Some(key), Some(value)) = (pair.next(), pair.next()) else {
            continue;
        };
        entries.push((
            String::from_utf8_lossy(key).into_owned(),
            Field {
                value: String::from_utf8_lossy(value).into_owned(),
                source: Some(Source::Config {
                    scope: String::from_utf8_lossy(scope).into_owned(),
                    origin: String::from_utf8_lossy(origin).into_owned(),
                }),
            },
        ));
    }
    Ok(entries)
}

fn parse_ident(ident: &str) -> Option<(&str, &str)> {
    let (name, tail) = ident.rsplit_once(" <")?;
    let (email, _) = tail.split_once("> ")?;
    Some((name, email))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
    fn run_git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn a_failed_email_write_puts_the_name_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "--local", "user.name", "Old One"]);
        run_git(
            dir.path(),
            &["config", "--local", "--add", "user.name", "Old Two"],
        );
        let failing = |workdir: Option<&Path>, args: &[&str]| {
            if args.contains(&"user.email") {
                Err(Error::invalid("config.lock appeared"))
            } else {
                write(workdir, args)
            }
        };
        let err = store(
            Some(dir.path()),
            Scope::Local,
            "New",
            "new@example.com",
            &failing,
        )
        .expect_err("the email write fails");
        assert!(err.to_string().contains("config.lock appeared"), "{err}");
        assert_eq!(
            scope_values(Some(dir.path()), Scope::Local, "user.name").expect("read"),
            ["Old One", "Old Two"],
            "the name is exactly as it was"
        );

        let fresh = tempfile::tempdir().expect("tempdir");
        run_git(fresh.path(), &["init", "-q"]);
        store(
            Some(fresh.path()),
            Scope::Local,
            "New",
            "new@example.com",
            &failing,
        )
        .expect_err("the email write fails");
        assert!(
            scope_values(Some(fresh.path()), Scope::Local, "user.name")
                .expect("read")
                .is_empty(),
            "a name that did not exist before is removed again"
        );
    }

    #[test]
    fn a_failed_clear_puts_the_name_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "--local", "user.name", "Old"]);
        // The name was removed and the email step then fails: the name must come back.
        let failing = |workdir: Option<&Path>, args: &[&str]| {
            if args.contains(&"user.email") {
                Err(Error::invalid("config.lock appeared"))
            } else {
                write(workdir, args)
            }
        };
        let previous = scope_values(Some(dir.path()), Scope::Local, "user.name").expect("read");
        unset(Some(dir.path()), Scope::Local, "user.name").expect("unset");
        let err = put_back(
            failing(Some(dir.path()), &["config", "x", "user.email"]).unwrap_err(),
            Some(dir.path()),
            Scope::Local,
            &previous,
        );
        assert!(err.to_string().contains("config.lock appeared"), "{err}");
        assert_eq!(
            scope_values(Some(dir.path()), Scope::Local, "user.name").expect("read"),
            ["Old"]
        );
    }

    #[test]
    fn a_repository_git_refuses_is_a_read_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        run_git(dir.path(), &["init", "-q"]);
        check_repository(command(Some(dir.path()))).expect("an ordinary repository opens");
        let mut refused = command(Some(dir.path()));
        refused.env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");
        let err = check_repository(refused).expect_err("git refuses it");
        assert!(err.to_string().contains("dubious ownership"), "{err}");
    }

    #[test]
    fn placeholder_rule() {
        let person = |name: &str, email: &str| Person {
            name: Field {
                value: name.into(),
                source: None,
            },
            email: Field {
                value: email.into(),
                source: None,
            },
            usable: true,
        };
        assert!(person("t", "t@t").looks_like_placeholder());
        assert!(person("Ada", "ada@localhost").looks_like_placeholder());
        assert!(!person("Ada", "ada@example.com").looks_like_placeholder());
    }
}
