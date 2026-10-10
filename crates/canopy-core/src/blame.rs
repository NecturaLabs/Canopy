//! Line-by-line authorship of one file (`git blame --porcelain`): which commit last changed each
//! line, and who made it.

use std::collections::HashMap;
use std::time::Duration;

use git2::Oid;

use crate::cli;
use crate::partial::MissingObjects;
use crate::{Repo, Result};

/// How long git may take: blame walks the history of the file.
const LIMIT: Duration = Duration::from_mins(10);

/// A commit that last changed at least one line of the blamed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameCommit {
    /// The commit; the all-zero id stands for changes that are not committed yet.
    pub oid: Oid,
    /// The author's name.
    pub author: String,
    /// The author's address, without the angle brackets.
    pub email: String,
    /// When it was authored, in seconds since the Unix epoch (UTC).
    pub time: i64,
    /// The first line of the message.
    pub summary: String,
}

impl BlameCommit {
    /// The lines of this entry are in the working tree only.
    #[must_use]
    pub fn is_uncommitted(&self) -> bool {
        self.oid.is_zero()
    }
}

/// One line of the blamed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    /// Index into [`Blame::commits`].
    pub commit: usize,
    /// The line's text, without its line ending.
    pub text: String,
}

/// The blame of one file: its lines in order, and the commits they come from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    pub lines: Vec<BlameLine>,
}

impl Repo {
    /// Blames `path` (relative to the working directory) as of `rev`, or as it is in the working
    /// tree when `rev` is `None`. Git follows the file through renames. Slow on long histories:
    /// call it from the worker. In a partial clone git would download the file's old versions
    /// from the remote on its own; with [`MissingObjects::Fail`] it may not.
    pub fn blame(&self, path: &str, rev: Option<Oid>, missing: MissingObjects) -> Result<Blame> {
        let id = rev.map(|oid| oid.to_string());
        let mut args = vec!["blame", "--porcelain"];
        args.extend(id.as_deref());
        args.extend(["--", path]);
        let mut cmd = cli::command(Some(self.workdir()));
        if missing == MissingObjects::Fail {
            cmd.env("GIT_NO_LAZY_FETCH", "1");
        }
        let out = cli::output_within(cmd, &args, LIMIT)?;
        if out.code != Some(0) {
            return Err(out.error(&args));
        }
        Ok(parse(&String::from_utf8_lossy(&out.stdout)))
    }
}

/// Parses `git blame --porcelain`: a header (`<id> <original line> <final line> [<group size>]`)
/// per line, then, the first time a commit appears, its details, then the line as `\t<text>`.
fn parse(porcelain: &str) -> Blame {
    let mut blame = Blame::default();
    let mut index: HashMap<Oid, usize> = HashMap::new();
    let mut current = None;
    for line in porcelain.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            if let Some(commit) = current {
                blame.lines.push(BlameLine {
                    commit,
                    text: text.trim_end_matches('\r').to_owned(),
                });
            }
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if let Ok(oid) = Oid::from_str(key) {
            let next = blame.commits.len();
            let at = *index.entry(oid).or_insert(next);
            if at == next {
                blame.commits.push(BlameCommit {
                    oid,
                    author: String::new(),
                    email: String::new(),
                    time: 0,
                    summary: String::new(),
                });
            }
            current = Some(at);
            continue;
        }
        let Some(commit) = current.and_then(|at| blame.commits.get_mut(at)) else {
            continue;
        };
        match key {
            "author" => value.clone_into(&mut commit.author),
            "author-mail" => {
                value
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .clone_into(&mut commit.email);
            }
            "author-time" => commit.time = value.parse().unwrap_or_default(),
            "summary" => value.clone_into(&mut commit.summary),
            _ => {}
        }
    }
    blame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_gives_each_line_its_commit_and_details_once() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let text = format!(
            "{a} 1 1 1\nauthor Ada\nauthor-mail <ada@example.com>\nauthor-time 100\nauthor-tz +0000\nsummary first\nfilename f\n\tone\n\
             {b} 1 2 1\nauthor Bo\nauthor-mail <bo@example.com>\nauthor-time 200\nsummary second\nfilename f\n\ttwo\r\n\
             {a} 3 3 1\nfilename f\n\t\tthree\n"
        );
        let blame = parse(&text);
        assert_eq!(blame.commits.len(), 2);
        assert_eq!(blame.commits[0].author, "Ada");
        assert_eq!(blame.commits[0].email, "ada@example.com");
        assert_eq!(blame.commits[1].time, 200);
        assert_eq!(blame.commits[1].summary, "second");
        let lines: Vec<_> = blame.lines.iter().map(|l| (l.commit, &*l.text)).collect();
        assert_eq!(lines, [(0, "one"), (1, "two"), (0, "\tthree")]);
    }

    #[test]
    fn an_empty_file_has_no_lines() {
        assert_eq!(parse(""), Blame::default());
    }
}
