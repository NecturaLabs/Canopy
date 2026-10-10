//! The commits that changed one file, followed back through renames (`git log --follow`).

use git2::Oid;

use crate::cli;
use crate::{ChangeKind, Repo, Result};

/// The longest a history read may take.
const TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// One commit in a file's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistoryEntry {
    /// The commit.
    pub oid: Oid,
    /// First line of its message.
    pub summary: String,
    /// Author name.
    pub author: String,
    /// Author time, seconds since the Unix epoch.
    pub time: i64,
    /// The file's path in this commit.
    pub path: String,
    /// The path before this commit, when it renamed or copied the file.
    pub old_path: Option<String>,
    /// What the commit did to the file ([`ChangeKind::Modified`] where git lists no change, as for
    /// a merge).
    pub kind: ChangeKind,
}

/// A file's history, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistory {
    pub entries: Vec<FileHistoryEntry>,
    /// There are older commits than `entries` holds.
    pub truncated: bool,
}

impl Repo {
    /// The commits reachable from `from` (`HEAD` when `None`) that changed `path` (relative to
    /// the repository root), at most `limit`, following the file back through renames. Each
    /// entry carries the path the file had in that commit, so its diff can be opened with
    /// [`crate::DiffSource::Commit`]. A partial clone downloads nothing for it.
    ///
    /// # Errors
    /// Git's own message when the history cannot be read (an unborn branch, objects a partial
    /// clone has not downloaded), or that git took over a minute.
    pub fn file_history(&self, from: Option<Oid>, path: &str, limit: usize) -> Result<FileHistory> {
        let count = format!("-n{}", limit.saturating_add(1));
        let start = from.map_or_else(|| "HEAD".to_owned(), |oid| oid.to_string());
        let args = [
            "--literal-pathspecs",
            "log",
            "--follow",
            "-M",
            "--name-status",
            "-z",
            "--no-show-signature",
            &count,
            "--format=%x1e%H%x1f%an%x1f%at%x1f%s",
            &start,
            "--",
            path,
        ];
        // Rename detection reads blobs, which a partial clone would download: not from here, and
        // not without an end, since `--follow` may walk the whole history.
        let mut cmd = cli::command(Some(self.workdir()));
        cmd.env("GIT_NO_LAZY_FETCH", "1");
        let out = cli::output_within(cmd, &args, TIME_LIMIT)?;
        if out.code != Some(0) {
            return Err(out.error(&args));
        }
        let mut entries = parse(&String::from_utf8_lossy(&out.stdout), path);
        let truncated = entries.len() > limit;
        entries.truncate(limit);
        Ok(FileHistory { entries, truncated })
    }
}

/// Reads `log -z --name-status` records (a `\x1e`, the format's fields, a NUL, a newline, then
/// the status and its paths each ended by a NUL). The log walks newest first, so the path starts
/// as the one asked for and moves back to the old name at each rename.
fn parse(out: &str, path: &str) -> Vec<FileHistoryEntry> {
    let mut current = path.to_owned();
    let mut entries = Vec::new();
    for record in out.split('\x1e').skip(1) {
        let mut tokens = record.split('\0');
        let mut fields = tokens.next().unwrap_or_default().split('\x1f');
        let (Some(id), Some(author), Some(time), Some(summary)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Ok(oid) = Oid::from_str(id) else { continue };
        let status = tokens.next().unwrap_or_default().trim_start();
        let old_path = if status.starts_with(['R', 'C'])
            && let (Some(old), Some(new)) = (tokens.next(), tokens.next())
            && new == current
        {
            Some(old.to_owned())
        } else {
            None
        };
        let kind = match status.chars().next() {
            Some('A') => ChangeKind::Added,
            Some('D') => ChangeKind::Deleted,
            Some('R' | 'C') => ChangeKind::Renamed,
            Some('T') => ChangeKind::Typechange,
            _ => ChangeKind::Modified,
        };
        entries.push(FileHistoryEntry {
            oid,
            summary: summary.to_owned(),
            author: author.to_owned(),
            time: time.parse().unwrap_or_default(),
            path: current.clone(),
            old_path: old_path.clone(),
            kind,
        });
        if let Some(old) = old_path {
            current = old;
        }
    }
    entries
}
