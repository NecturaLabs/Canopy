//! Pickaxe search: the commits that added or removed a piece of text (`git log -S` and `-G`).

use git2::Oid;

use crate::cli;
use crate::{Error, Repo, Result};

/// The longest a search may take: it reads every commit's blobs.
const TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// One file of one commit whose change matches the search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeHit {
    /// The commit.
    pub oid: Oid,
    /// First line of its message.
    pub summary: String,
    /// Author name.
    pub author: String,
    /// Author time, seconds since the Unix epoch.
    pub time: i64,
    /// The file whose change matched, as the commit has it.
    pub path: String,
}

/// The matches of a search, newest commit first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSearch {
    pub hits: Vec<ChangeHit>,
    /// There are more matching commits than `hits` holds.
    pub truncated: bool,
}

impl Repo {
    /// The commits reachable from `from` (`HEAD` when `None`) that added or removed `query`
    /// (a count change of the text, `git log -S`), or with `regex` whose changed lines match it
    /// (`git log -G`), at most `limit` commits, one hit per matching file. Merge commits show no
    /// diff to search. A partial clone downloads nothing for it: missing objects are skipped.
    ///
    /// # Errors
    /// An empty query or one with a NUL, git's own message when the search cannot run (an unborn
    /// branch, a regular expression it rejects), or that git took over two minutes.
    pub fn search_changes(
        &self,
        from: Option<Oid>,
        query: &str,
        regex: bool,
        limit: usize,
    ) -> Result<ChangeSearch> {
        if query.is_empty() || query.contains('\0') {
            return Err(Error::invalid("Enter the text to search for."));
        }
        // The text is part of one argument, so it can never read as an option.
        let pickaxe = format!("{}{query}", if regex { "-G" } else { "-S" });
        let count = format!("-n{}", limit.saturating_add(1));
        let start = from.map_or_else(|| "HEAD".to_owned(), |oid| oid.to_string());
        let args = [
            "log",
            "--name-only",
            "-z",
            "--no-show-signature",
            "--no-color",
            &pickaxe,
            &count,
            "--format=%x1e%H%x1f%an%x1f%at%x1f%s",
            &start,
            "--",
        ];
        let mut cmd = cli::command(Some(self.workdir()));
        cmd.env("GIT_NO_LAZY_FETCH", "1");
        let out = cli::output_within(cmd, &args, TIME_LIMIT)?;
        if out.code != Some(0) {
            return Err(out.error(&args));
        }
        let mut commits = parse(&String::from_utf8_lossy(&out.stdout));
        let truncated = commits.len() > limit;
        commits.truncate(limit);
        Ok(ChangeSearch {
            hits: commits.into_iter().flatten().collect(),
            truncated,
        })
    }
}

/// Reads `log -z --name-only` records: a `\x1e`, the format's fields, a NUL, a newline, then
/// each matching path ended by a NUL. A commit with no matching file (a merge) has no hits.
fn parse(out: &str) -> Vec<Vec<ChangeHit>> {
    out.split('\x1e')
        .skip(1)
        .filter_map(|record| {
            let mut tokens = record.split('\0');
            let mut fields = tokens.next().unwrap_or_default().split('\x1f');
            let (Some(id), Some(author), Some(time), Some(summary)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return None;
            };
            let oid = Oid::from_str(id).ok()?;
            let hits = tokens
                .map(|path| path.trim_start_matches('\n'))
                .filter(|path| !path.is_empty())
                .map(|path| ChangeHit {
                    oid,
                    summary: summary.to_owned(),
                    author: author.to_owned(),
                    time: time.parse().unwrap_or_default(),
                    path: path.to_owned(),
                })
                .collect();
            Some(hits)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_lists_each_matching_file_and_a_merge_has_none() {
        let id = "1".repeat(40);
        let out = format!(
            "\x1e{id}\x1fAda\x1f2\x1fedit\0\nsrc/a.rs\0docs/b d.md\0\x1e{id}\x1fAda\x1f1\x1fmerge\0\n"
        );
        let found = parse(&out);
        let paths: Vec<_> = found
            .iter()
            .map(|hits| hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>())
            .collect();
        assert_eq!(paths, [vec!["src/a.rs", "docs/b d.md"], vec![]]);
        assert_eq!(found[0][0].summary, "edit");
    }
}
