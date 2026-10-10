//! Git configuration values with their scope and origin, read without quoting file names.
//!
//! File paths retain their original bytes; the display origin is separate and always absolute.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Result, cli, paths::os_path};

/// Which values to ask Git for, in its configuration reading order.
#[derive(Debug, Clone, Copy)]
pub enum Query<'a> {
    /// The effective value of one key.
    Get(&'a str),
    /// Every key matching a Git regular expression, including repeated values.
    Matching(&'a str),
}

/// One configuration entry and where Git read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    /// `None` for an implicit value in a matching query; `Some("")` for an explicit empty value.
    /// A single-key query cannot distinguish them: both are returned as `Some("")`.
    pub value: Option<String>,
    /// Empty on Git versions without `--show-scope`.
    pub scope: String,
    /// Display text; file origins are absolute and unquoted.
    pub origin: String,
    /// The actual file path, preserving non-UTF-8 names.
    pub file: Option<PathBuf>,
}

/// One read must not hold up the background configuration reader indefinitely.
const READ_LIMIT: Duration = Duration::from_secs(15);
/// Scope, origin and entry in current Git's NUL-delimited output.
const SCOPED_FIELDS: usize = 3;
/// Origin and entry before Git supported `--show-scope`.
const ORIGIN_FIELDS: usize = 2;

/// Read repository configuration, or only the global configuration when `global` is true.
/// Blocking: callers with a UI must run this off its thread.
pub fn read(dir: &Path, global: bool, query: Query<'_>) -> Result<Vec<Entry>> {
    let mut args = vec!["config", "-z"];
    if global {
        args.push("--global");
    }
    args.extend(["--show-origin", "--show-scope"]);
    match query {
        Query::Get(key) => args.extend(["--get", "--", key]),
        Query::Matching(pattern) => args.extend(["--get-regexp", "--", pattern]),
    }
    let mut out = cli::output_within(cli::command(Some(dir)), &args, READ_LIMIT)?;
    let with_scope = if !matches!(out.code, Some(0 | 1))
        && String::from_utf8_lossy(&out.stderr).contains("show-scope")
    {
        args.retain(|arg| *arg != "--show-scope");
        out = cli::output_within(cli::command(Some(dir)), &args, READ_LIMIT)?;
        false
    } else {
        true
    };
    match out.code {
        Some(1) => Ok(Vec::new()),
        Some(0) => {
            // Git prints repository-local file origins relative to the worktree root even when
            // invoked from a subdirectory. Global-only reads use the supplied working directory.
            let root = if global {
                None
            } else {
                git2::Repository::discover(dir)
                    .ok()
                    .and_then(|repo| repo.workdir().map(Path::to_path_buf))
            };
            let root = root.unwrap_or(std::path::absolute(dir)?);
            Ok(parse(&out.stdout, query, with_scope, &root))
        }
        _ => Err(out.error(&args)),
    }
}

/// Parse `git config -z --show-origin [--show-scope]` output for `query`.
/// `root` is the absolute worktree root, or the directory used for a global-only read.
pub fn parse(out: &[u8], query: Query<'_>, with_scope: bool, root: &Path) -> Vec<Entry> {
    let Some(out) = out.strip_suffix(&[0]) else {
        return Vec::new();
    };
    let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    let width = if with_scope {
        SCOPED_FIELDS
    } else {
        ORIGIN_FIELDS
    };
    fields
        .chunks_exact(width)
        .map(|record| {
            let (scope, rest) = if with_scope {
                (record[0], &record[1..])
            } else {
                (b"".as_slice(), record)
            };
            let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
            let (key, value) = match query {
                Query::Get(key) => (key.to_owned(), Some(text(rest[1]))),
                Query::Matching(_) => match rest[1].iter().position(|&b| b == b'\n') {
                    Some(at) => (text(&rest[1][..at]), Some(text(&rest[1][at + 1..]))),
                    None => (text(rest[1]), None),
                },
            };
            let file = rest[0].strip_prefix(b"file:").map(|bytes| {
                let path = os_path(bytes);
                if path.is_relative() {
                    root.join(path)
                } else {
                    path
                }
            });
            let origin = file
                .as_ref()
                .map_or_else(|| text(rest[0]), |path| format!("file:{}", path.display()));
            Entry {
                key,
                value,
                scope: text(scope),
                origin,
                file,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_values_keep_order_empty_implicit_and_multiline_values() {
        let entries = parse(
            b"local\0file:.git/config\0a.b\0local\0file:.git/config\0a.b\n\0command\0command line:\0a.b\nfirst\nsecond\0",
            Query::Matching("a.b"), true, Path::new("/repo"),
        );
        assert_eq!(
            entries
                .iter()
                .map(|e| e.value.as_deref())
                .collect::<Vec<_>>(),
            [None, Some(""), Some("first\nsecond")]
        );
        assert_eq!(
            entries[0].origin,
            format!("file:{}", Path::new("/repo").join(".git/config").display())
        );
        assert_eq!(entries[2].file, None);
        assert_eq!(entries[2].origin, "command line:");
        assert_eq!(
            parse(b"", Query::Get("a.b"), true, Path::new("/repo")),
            Vec::<Entry>::new()
        );
    }

    #[test]
    fn a_single_value_without_scope_keeps_an_empty_value() {
        let entries = parse(
            b"file:.git/config\0\0",
            Query::Get("a.b"),
            false,
            Path::new("/repo"),
        );
        assert_eq!(entries[0].scope, "");
        assert_eq!(entries[0].value.as_deref(), Some(""));
    }

    #[test]
    fn reading_from_a_subdirectory_uses_the_repository_root() {
        let root = tempfile::tempdir().expect("tempdir");
        cli::run(root.path(), &["init", "-q", "-b", "main"]).expect("init");
        cli::run(root.path(), &["config", "a.b", "value"]).expect("config");
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).expect("directory");
        let entries = read(&sub, false, Query::Get("a.b")).expect("read");
        assert_eq!(
            entries[0].file.as_deref(),
            Some(root.path().join(".git/config").as_path())
        );
    }
}
