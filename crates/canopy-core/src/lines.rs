//! Line-level staging: stage, unstage or discard some of the `+` and `-` lines of one hunk.
//!
//! The selection is turned into an edited hunk exactly as a user would edit it in the `e` mode
//! of `git add -p` (forward) or `git reset -p` / `git checkout -p` (reverse), and the result is
//! piped to `git apply`. In a forward patch git matches the old side against the index, so an
//! unselected removal becomes context (the line stays) and an unselected addition is dropped
//! (the line is not added). A reverse patch is matched on its new side, so the roles swap: an
//! unselected addition becomes context and an unselected removal is dropped.
//!
//! Every line carries its own "no newline at end of file" state, so a dropped line takes its
//! `\ No newline at end of file` marker with it. A selection that would leave a line without a
//! newline in the middle of the result is refused: `git apply` would otherwise join it with the
//! next line (git's own edit mode does exactly that).

use git2::{Diff, Patch};

use crate::cli;
use crate::diff::{DiffSource, HunkKey, file_line, quoted};
use crate::ops::HunkAction;
use crate::{Error, Repo, Result};

/// The length of the `@@` that opens and closes a hunk header's ranges.
const MARKER_LEN: usize = 2;
const NOTHING_SELECTED: &str = "Select at least one added or removed line.";
const NOT_IN_HUNK: &str = "The selected lines are not part of that change. Refresh and try again.";
const MISSING_NEWLINE: &str = "The last line of the file has no newline at its end. Select its \
                               removed and added lines together.";

/// Which side of a hunk line it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Context,
    Added,
    Removed,
}

impl Origin {
    fn marker(self) -> u8 {
        match self {
            Self::Context => b' ',
            Self::Added => b'+',
            Self::Removed => b'-',
        }
    }
}

/// One line of a hunk, byte-exact.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    origin: Origin,
    /// Content without the final `\n` (a `\r` before it is kept).
    text: Vec<u8>,
    /// The line ends with `\n`; false only for the last line of a file without one.
    newline: bool,
}

/// A hunk as libgit2 produced it.
#[derive(Debug, Clone)]
struct RawHunk {
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    /// The text after the closing `@@` (usually the enclosing function), without line end.
    section: Vec<u8>,
    /// The lines, in the same order and with the same indices as [`crate::Hunk::lines`].
    lines: Vec<Line>,
}

impl RawHunk {
    fn key(&self) -> HunkKey {
        (
            self.old_start,
            self.old_lines,
            self.new_start,
            self.new_lines,
        )
    }
}

/// The file part of a patch header.
#[derive(Debug, Clone)]
struct FileHeader {
    old_path: Vec<u8>,
    new_path: Vec<u8>,
    old_mode: i32,
    new_mode: i32,
    /// The file does not exist before the change (added or untracked).
    old_absent: bool,
    /// The file does not exist after the change (deleted).
    new_absent: bool,
}

/// A hunk after applying a selection, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edited {
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    lines: Vec<Line>,
}

impl Repo {
    /// Stages, unstages or discards only some lines of hunk number `index` of `path`.
    ///
    /// `lines` are indices into that hunk's [`crate::Hunk::lines`] as the user saw them
    /// ([`DiffSource::Unstaged`] for [`HunkAction::Stage`] and [`HunkAction::Discard`],
    /// [`DiffSource::Staged`] for [`HunkAction::Unstage`], computed with the same `context`).
    /// Selected `+` and `-` lines are applied; context lines in the selection are ignored. The
    /// result matches editing the hunk in `git add -p`, `git reset -p` or `git checkout -p`.
    ///
    /// `expected` is the hunk's [`crate::Hunk::key`]; if the file changed since, nothing happens
    /// and an error asks for a refresh. Nothing changes either when the selection holds no `+`
    /// or `-` line, or when it would leave a line without its newline in the middle of the file.
    pub fn apply_lines(
        &self,
        action: HunkAction,
        path: &str,
        index: usize,
        expected: HunkKey,
        lines: &[usize],
        context: u32,
    ) -> Result<()> {
        if lines.is_empty() {
            return Err(Error::invalid(NOTHING_SELECTED));
        }
        let (source, reverse) = match action {
            HunkAction::Stage => (DiffSource::Unstaged, false),
            HunkAction::Unstage => (DiffSource::Staged, true),
            HunkAction::Discard => (DiffSource::Unstaged, true),
        };
        // Offline, like `apply_hunk`: a partial clone already has the objects of a diff it showed;
        // if not, the caller gets Error::NotDownloaded rather than a hidden fetch.
        let found = self.read_diff(&source, path, None, crate::MissingObjects::Fail, || {
            let diff = self.raw_diff(&source, path, None, context, crate::Whitespace::Show)?;
            read_hunk(&diff, index)
        })?;
        let Some((file, hunk)) = found else {
            return Err(Error::invalid(
                "That change no longer exists. Refresh and try again.",
            ));
        };
        if hunk.key() != expected {
            return Err(Error::invalid(
                "The file changed since it was shown. Refresh and try again.",
            ));
        }
        let edited = edit(&hunk, lines, reverse)?;
        let patch = render(&file, &hunk.section, &edited);
        cli::run_with_input(self.workdir(), &action.apply_args(context), &patch).map(drop)
    }
}

/// Reads hunk number `index` of the single-file `diff`, with the header of its file.
fn read_hunk(diff: &Diff<'_>, index: usize) -> Result<Option<(FileHeader, RawHunk)>> {
    for idx in 0..diff.deltas().len() {
        let Some(delta) = diff.get_delta(idx) else {
            continue;
        };
        if delta.status() == git2::Delta::Typechange {
            return Err(crate::diff::typechange_error());
        }
        let Some(patch) = Patch::from_diff(diff, idx)? else {
            continue;
        };
        if index >= patch.num_hunks() {
            return Ok(None);
        }
        let status = delta.status();
        let file = FileHeader {
            old_path: delta.old_file().path_bytes().unwrap_or_default().to_vec(),
            new_path: delta.new_file().path_bytes().unwrap_or_default().to_vec(),
            old_mode: i32::from(delta.old_file().mode()),
            new_mode: i32::from(delta.new_file().mode()),
            old_absent: matches!(status, git2::Delta::Added | git2::Delta::Untracked),
            new_absent: status == git2::Delta::Deleted,
        };
        let (header, n) = patch.hunk(index)?;
        let mut lines = Vec::with_capacity(n);
        for l in 0..n {
            let line = patch.line_in_hunk(index, l)?;
            // The same filter as `diff::to_file_diff`, so indices match what the UI shows.
            // End-of-file markers ('=', '>', '<') are implied by `newline` below.
            let origin = match line.origin() {
                '+' => Origin::Added,
                '-' => Origin::Removed,
                ' ' => Origin::Context,
                _ => continue,
            };
            let content = line.content();
            let (text, newline) = match content.strip_suffix(b"\n") {
                Some(text) => (text, true),
                None => (content, false),
            };
            lines.push(Line {
                origin,
                text: text.to_vec(),
                newline,
            });
        }
        let hunk = RawHunk {
            old_start: header.old_start(),
            old_lines: header.old_lines(),
            new_start: header.new_start(),
            new_lines: header.new_lines(),
            section: section(header.header()),
            lines,
        };
        return Ok(Some((file, hunk)));
    }
    Ok(None)
}

/// The text after the closing `@@` of a hunk header, without its line end.
fn section(header: &[u8]) -> Vec<u8> {
    let rest = header.get(MARKER_LEN..).unwrap_or_default();
    let Some(close) = rest.windows(MARKER_LEN).position(|w| w == b"@@") else {
        return Vec::new();
    };
    let mut s = rest[close + MARKER_LEN..].to_vec();
    while s.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        s.pop();
    }
    s
}

/// Applies the selection `selected` (indices into `hunk.lines`) to `hunk`. `reverse` is true
/// when the patch will be applied with `--reverse`.
fn edit(hunk: &RawHunk, selected: &[usize], reverse: bool) -> Result<Edited> {
    let mut chosen = vec![false; hunk.lines.len()];
    for &i in selected {
        let Some(slot) = chosen.get_mut(i) else {
            return Err(Error::invalid(NOT_IN_HUNK));
        };
        *slot = true;
    }
    let any_change = hunk
        .lines
        .iter()
        .zip(&chosen)
        .any(|(line, &c)| c && line.origin != Origin::Context);
    if !any_change {
        return Err(Error::invalid(NOTHING_SELECTED));
    }

    // The side git matches against (old when forward, new when reverse) must stay intact, so
    // unselected lines of that side become context and unselected lines of the other side go.
    let stays = if reverse {
        Origin::Added
    } else {
        Origin::Removed
    };
    let mut lines = Vec::with_capacity(hunk.lines.len());
    for (line, &c) in hunk.lines.iter().zip(&chosen) {
        if c || line.origin == Origin::Context {
            lines.push(line.clone());
        } else if line.origin == stays {
            lines.push(Line {
                origin: Origin::Context,
                ..line.clone()
            });
        }
    }

    // On the side that is produced, only the last line may lack a newline; otherwise git apply
    // would glue it to the line after it.
    let produced = |l: &&Line| {
        if reverse {
            l.origin != Origin::Added
        } else {
            l.origin != Origin::Removed
        }
    };
    if lines
        .iter()
        .filter(produced)
        .rev()
        .skip(1)
        .any(|l| !l.newline)
    {
        return Err(Error::invalid(MISSING_NEWLINE));
    }

    let old_lines = count(&lines, Origin::Added);
    let new_lines = count(&lines, Origin::Removed);
    // The matched side keeps its position; the produced side starts at the same place, in the
    // unified-diff convention that an empty side names the line before it.
    let (old_start, new_start) = if reverse {
        let at = lines_before(hunk.new_start, hunk.new_lines);
        (start_of(at, old_lines), hunk.new_start)
    } else {
        let at = lines_before(hunk.old_start, hunk.old_lines);
        (hunk.old_start, start_of(at, new_lines))
    };
    Ok(Edited {
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines,
    })
}

/// Lines of one side: every line that is not of the other side's `excluded` origin.
fn count(lines: &[Line], excluded: Origin) -> u32 {
    let n = lines.iter().filter(|l| l.origin != excluded).count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// How many lines of a side come before a hunk that starts at `start` with `len` lines.
fn lines_before(start: u32, len: u32) -> u32 {
    if len == 0 {
        start
    } else {
        start.saturating_sub(1)
    }
}

/// The header start number of a side with `len` lines that follows `before` lines.
fn start_of(before: u32, len: u32) -> u32 {
    if len == 0 { before } else { before + 1 }
}

/// Renders a single-hunk patch for `git apply`.
fn render(file: &FileHeader, section: &[u8], e: &Edited) -> Vec<u8> {
    // A side that was absent stays absent only while it is empty: a partly unstaged new file is
    // still in the index, a partly staged deletion still leaves the file.
    let old_absent = file.old_absent && e.old_lines == 0;
    let new_absent = file.new_absent && e.new_lines == 0;

    let mut out = Vec::new();
    out.extend_from_slice(b"diff --git ");
    out.extend(quoted(b"a/", &file.old_path));
    out.push(b' ');
    out.extend(quoted(b"b/", &file.new_path));
    out.push(b'\n');
    if old_absent {
        out.extend_from_slice(format!("new file mode {:o}\n", file.new_mode).as_bytes());
    } else if new_absent {
        out.extend_from_slice(format!("deleted file mode {:o}\n", file.old_mode).as_bytes());
    }
    file_line(&mut out, b"--- ", b"a/", &file.old_path, old_absent);
    file_line(&mut out, b"+++ ", b"b/", &file.new_path, new_absent);
    out.extend_from_slice(
        format!(
            "@@ -{},{} +{},{} @@",
            e.old_start, e.old_lines, e.new_start, e.new_lines
        )
        .as_bytes(),
    );
    out.extend_from_slice(section);
    out.push(b'\n');
    for line in &e.lines {
        out.push(line.origin.marker());
        out.extend_from_slice(&line.text);
        out.push(b'\n');
        if !line.newline {
            out.extend_from_slice(b"\\ No newline at end of file\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a hunk from diff-style text: one line per entry, first byte the origin, a trailing
    /// `$` meaning "no newline at end of file".
    fn hunk(header: HunkKey, body: &[&str]) -> RawHunk {
        let lines = body
            .iter()
            .map(|l| {
                let (origin, rest) = l.split_at(1);
                let origin = match origin {
                    "+" => Origin::Added,
                    "-" => Origin::Removed,
                    _ => Origin::Context,
                };
                let (text, newline) = match rest.strip_suffix('$') {
                    Some(t) => (t, false),
                    None => (rest, true),
                };
                Line {
                    origin,
                    text: text.as_bytes().to_vec(),
                    newline,
                }
            })
            .collect();
        RawHunk {
            old_start: header.0,
            old_lines: header.1,
            new_start: header.2,
            new_lines: header.3,
            section: Vec::new(),
            lines,
        }
    }

    fn body(e: &Edited) -> Vec<String> {
        e.lines
            .iter()
            .map(|l| {
                let mut s = String::from(l.origin.marker() as char);
                s.push_str(&String::from_utf8_lossy(&l.text));
                if !l.newline {
                    s.push('$');
                }
                s
            })
            .collect()
    }

    #[test]
    fn edits_like_git_add_p_and_reset_p() {
        // (case, hunk key, hunk body, selection, reverse, expected key, expected body)
        type Case<'a> = (
            &'a str,
            HunkKey,
            &'a [&'a str],
            &'a [usize],
            bool,
            HunkKey,
            &'a [&'a str],
        );
        let cases: &[Case<'_>] = &[
            (
                "forward: drop unselected additions",
                (1, 3, 1, 8),
                &[" one", " two", " three", "+A", "+B", "+C", "+D", "+E"],
                &[4, 6],
                false,
                (1, 3, 1, 5),
                &[" one", " two", " three", "+B", "+D"],
            ),
            (
                "forward: unselected removals become context",
                (1, 6, 1, 2),
                &[" one", "-two", "-three", "-four", "-five", " six"],
                &[2, 4],
                false,
                (1, 6, 1, 4),
                &[" one", " two", "-three", " four", "-five", " six"],
            ),
            (
                "reverse: unselected additions become context, removals go",
                (1, 4, 1, 4),
                &[" one", "-two", "-three", "+TWO", "+THREE", " four"],
                &[2, 4],
                true,
                (1, 4, 1, 4),
                &[" one", "-three", " TWO", "+THREE", " four"],
            ),
            (
                "forward: everything removed, new side empty starts before the hunk",
                (4, 2, 3, 0),
                &["-x", "-y"],
                &[0, 1],
                false,
                (4, 2, 3, 0),
                &["-x", "-y"],
            ),
            (
                "forward: insertion into an empty file",
                (0, 0, 1, 2),
                &["+a", "+b"],
                &[1],
                false,
                (0, 0, 1, 1),
                &["+b"],
            ),
            (
                "reverse: part of a new file keeps the rest as context",
                (0, 0, 1, 3),
                &["+a", "+b", "+c"],
                &[1],
                true,
                (1, 2, 1, 3),
                &[" a", "+b", " c"],
            ),
            (
                "reverse: all of a new file empties the old side",
                (0, 0, 1, 2),
                &["+a", "+b"],
                &[0, 1],
                true,
                (0, 0, 1, 2),
                &["+a", "+b"],
            ),
            (
                "reverse: position comes from the new side",
                (7, 3, 9, 5),
                &[" a", "+b", "+c", " d", " e"],
                &[2],
                true,
                (9, 4, 9, 5),
                &[" a", " b", "+c", " d", " e"],
            ),
            (
                "forward: dropped addition takes its no-newline marker along",
                (1, 2, 1, 2),
                &[" x", "-y", "+Y$"],
                &[1],
                false,
                (1, 2, 1, 1),
                &[" x", "-y"],
            ),
            (
                "forward: selected addition without newline stays last",
                (1, 2, 1, 2),
                &[" x", "-y", "+Y$"],
                &[2],
                false,
                (1, 2, 1, 3),
                &[" x", " y", "+Y$"],
            ),
            (
                "selected context lines are ignored",
                (1, 2, 1, 3),
                &[" a", "+b", " c"],
                &[0, 1, 2],
                false,
                (1, 2, 1, 3),
                &[" a", "+b", " c"],
            ),
        ];
        for (name, key, lines, sel, reverse, want_key, want_body) in cases {
            let e = edit(&hunk(*key, lines), sel, *reverse).unwrap_or_else(|err| {
                panic!("{name}: {err}");
            });
            assert_eq!(
                (e.old_start, e.old_lines, e.new_start, e.new_lines),
                *want_key,
                "{name}"
            );
            assert_eq!(body(&e), *want_body, "{name}");
        }
    }

    #[test]
    fn refuses_selections_that_change_nothing_or_break_lines() {
        let h = hunk((1, 2, 1, 3), &[" a", "-b$", "+b", "+c"]);
        let cases: &[(&str, &[usize], bool, &str)] = &[
            ("empty", &[], false, NOTHING_SELECTED),
            ("context only", &[0], false, NOTHING_SELECTED),
            ("out of range", &[1, 9], false, NOT_IN_HUNK),
            // Old "a\nb" (no newline) -> "a\nb\nc\n": staging only "+c" would give "a\nbc\n".
            ("forward glue", &[3], false, MISSING_NEWLINE),
        ];
        for (name, sel, reverse, want) in cases {
            match edit(&h, sel, *reverse) {
                Err(Error::Invalid(msg)) => assert_eq!(msg, *want, "{name}"),
                other => panic!("{name}: {other:?}"),
            }
        }
        // Reverse mirror, the same hunk as a staged diff (HEAD "a\nb", index "a\nb\nc\n"):
        // unstaging only "-b$" would put "b" without a newline before "b" and "c".
        assert!(matches!(edit(&h, &[1], true), Err(Error::Invalid(m)) if m == MISSING_NEWLINE));
        // Unstaging only "+c" is fine: the index becomes "a\nb\n".
        let e = edit(&h, &[3], true).expect("reverse +c");
        assert_eq!(body(&e), [" a", " b", "+c"]);
        // Selecting both versions of the last line is fine.
        edit(&h, &[1, 2, 3], false).unwrap();
    }

    #[test]
    fn renders_headers_for_absent_sides() {
        let file = FileHeader {
            old_path: b"n.txt".to_vec(),
            new_path: b"n.txt".to_vec(),
            old_mode: 0,
            new_mode: 0o100_644,
            old_absent: true,
            new_absent: false,
        };
        let h = hunk((0, 0, 1, 2), &["+a", "+b"]);
        let whole = render(&file, b"", &edit(&h, &[0, 1], true).expect("edit"));
        assert!(whole.starts_with(b"diff --git a/n.txt b/n.txt\nnew file mode 100644\n"));
        // Unstaging part of a new file must not delete it: the patch becomes a modification.
        let part = render(&file, b"", &edit(&h, &[1], true).expect("edit"));
        assert_eq!(
            String::from_utf8_lossy(&part),
            "diff --git a/n.txt b/n.txt\n--- a/n.txt\n+++ b/n.txt\n@@ -1,1 +1,2 @@\n a\n+b\n"
        );
    }

    #[test]
    fn extracts_the_hunk_section() {
        assert_eq!(section(b"@@ -1,3 +1,4 @@ fn main() {\n"), b" fn main() {");
        assert_eq!(section(b"@@ -1 +1 @@\n"), b"");
    }
}
