//! Diffs as plain data, and single-hunk patches for staging part of a file.

use git2::{Diff, DiffOptions, Patch};

use crate::Result;
use crate::model::ChangeKind;

/// The ASCII characters git leaves unquoted in a path: space up to, not including, DEL.
const PRINTABLE_ASCII: std::ops::Range<u8> = 0x20..0x7f;
/// The control characters git names with a letter in a quoted path.
const BEL: u8 = 0x07;
const BS: u8 = 0x08;
const VT: u8 = 0x0b;
const FF: u8 = 0x0c;

/// The numbers of a hunk header (`old_start, old_lines, new_start, new_lines`), which identify a
/// hunk well enough to tell whether it is still the one the user saw.
pub type HunkKey = (u32, u32, u32, u32);

/// What a diff line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Unchanged line shown for context.
    Context,
    /// Line only in the new version.
    Added,
    /// Line only in the old version.
    Removed,
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Context, addition or removal.
    pub kind: LineKind,
    /// Line number in the old file.
    pub old_no: Option<u32>,
    /// Line number in the new file.
    pub new_no: Option<u32>,
    /// Text without the trailing newline; tabs kept.
    pub text: String,
}

/// A contiguous block of changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@ context` header line.
    pub header: String,
    /// First line in the old file.
    pub old_start: u32,
    /// Number of old lines.
    pub old_lines: u32,
    /// First line in the new file.
    pub new_start: u32,
    /// Number of new lines.
    pub new_lines: u32,
    /// The lines.
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// The identifying numbers of the header, used to check a hunk is still current.
    pub fn key(&self) -> HunkKey {
        (
            self.old_start,
            self.old_lines,
            self.new_start,
            self.new_lines,
        )
    }
}

/// The changes to one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// Path (new side).
    pub path: String,
    /// Old path when renamed.
    pub old_path: Option<String>,
    /// How the file changed, when known.
    pub kind: Option<ChangeKind>,
    /// Git considers the file binary; no hunks.
    pub binary: bool,
    /// Hunks, top to bottom.
    pub hunks: Vec<Hunk>,
    /// Lines added.
    pub additions: usize,
    /// Lines removed.
    pub deletions: usize,
    /// The path is a submodule on either side: the commits it moved between.
    pub submodule: Option<crate::submodule::SubmoduleChange>,
    /// Git file mode on the old side; `None` when it does not exist (or the diff is empty).
    pub old_mode: Option<u32>,
    /// Git file mode on the new side; `None` when it does not exist (or the diff is empty).
    pub new_mode: Option<u32>,
}

impl FileDiff {
    /// No hunks and not binary. A mode-only change is empty but has a [`Self::mode_change`].
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty() && !self.binary
    }

    /// The old and new mode when both sides exist and differ: an executable-bit change, or a
    /// file/symbolic link type change (one entry carrying both sides, never two).
    pub fn mode_change(&self) -> Option<(u32, u32)> {
        match (self.old_mode, self.new_mode) {
            (Some(old), Some(new)) if old != new => Some((old, new)),
            _ => None,
        }
    }
}

/// Hunk and line staging refuse a file that changed type: its diff has no hunks, and applying
/// part of it would make no sense.
pub(crate) fn typechange_error() -> crate::Error {
    crate::Error::invalid(
        "This file changed type (file and symbolic link). Stage or discard the whole file \
         instead of a part of it.",
    )
}

/// A libgit2 file mode as Git writes it (`0o100644`), `None` for a side that does not exist.
pub(crate) fn mode_of(file: &git2::DiffFile<'_>) -> Option<u32> {
    u32::try_from(i32::from(file.mode()))
        .ok()
        .filter(|m| *m != 0)
}

/// Which diff of a file to compute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffSource {
    /// Working tree against the index (what `git diff` shows).
    Unstaged,
    /// Index against `HEAD` (what `git diff --cached` shows).
    Staged,
    /// A commit against its first parent.
    Commit(git2::Oid),
}

/// How whitespace changes show in a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Whitespace {
    /// Every change, like `git diff`.
    #[default]
    Show,
    /// Ignore whitespace changes at line ends, like `git diff --ignore-space-at-eol`.
    IgnoreEol,
    /// Ignore all whitespace when comparing lines, like `git diff -w`.
    IgnoreAll,
}

impl Whitespace {
    /// Every mode, in menu order.
    pub const ALL: [Self; 3] = [Self::Show, Self::IgnoreEol, Self::IgnoreAll];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Show => "Show whitespace",
            Self::IgnoreEol => "Ignore at end of line",
            Self::IgnoreAll => "Ignore all",
        }
    }
}

pub(crate) fn options(
    path: &str,
    old_path: Option<&str>,
    context: u32,
    whitespace: Whitespace,
) -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.pathspec(path)
        .disable_pathspec_match(true)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        // A file replaced by a symbolic link (or back) is one typechange delta, not a deletion
        // and an addition of the same path.
        .include_typechange(true)
        .context_lines(context)
        .interhunk_lines(0);
    match whitespace {
        Whitespace::Show => {}
        Whitespace::IgnoreEol => {
            opts.ignore_whitespace_eol(true);
        }
        Whitespace::IgnoreAll => {
            opts.ignore_whitespace(true);
        }
    }
    if let Some(old) = old_path {
        opts.pathspec(old);
    }
    opts
}

pub(crate) fn delta_kind(status: git2::Delta) -> Option<ChangeKind> {
    use git2::Delta;
    Some(match status {
        Delta::Added => ChangeKind::Added,
        Delta::Deleted => ChangeKind::Deleted,
        Delta::Modified => ChangeKind::Modified,
        Delta::Renamed | Delta::Copied => ChangeKind::Renamed,
        Delta::Typechange => ChangeKind::Typechange,
        Delta::Untracked => ChangeKind::Untracked,
        Delta::Conflicted => ChangeKind::Conflicted,
        Delta::Unmodified | Delta::Ignored | Delta::Unreadable => return None,
    })
}

pub(crate) fn path_of(file: &git2::DiffFile<'_>) -> Option<String> {
    file.path().map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// Converts every delta of `diff` into one [`FileDiff`] (deltas are merged; callers pass a
/// diff restricted to one file).
pub(crate) fn to_file_diff(diff: &Diff<'_>, path: &str) -> Result<FileDiff> {
    let mut out = FileDiff {
        path: path.to_owned(),
        ..FileDiff::default()
    };
    for idx in 0..diff.deltas().len() {
        let Some(delta) = diff.get_delta(idx) else {
            continue;
        };
        out.kind = delta_kind(delta.status());
        out.old_mode = mode_of(&delta.old_file());
        out.new_mode = mode_of(&delta.new_file());
        if let Some(new) = path_of(&delta.new_file()) {
            out.path = new;
        }
        let old = path_of(&delta.old_file());
        if delta.status() == git2::Delta::Renamed && old.as_deref() != Some(out.path.as_str()) {
            out.old_path = old;
        }
        if delta.flags().is_binary() {
            out.binary = true;
            continue;
        }
        let Some(patch) = Patch::from_diff(diff, idx)? else {
            continue;
        };
        if delta.flags().is_binary() {
            out.binary = true;
            continue;
        }
        for h in 0..patch.num_hunks() {
            let (hunk, n) = patch.hunk(h)?;
            let mut lines = Vec::with_capacity(n);
            for l in 0..n {
                let line = patch.line_in_hunk(h, l)?;
                let kind = match line.origin() {
                    '+' => LineKind::Added,
                    '-' => LineKind::Removed,
                    ' ' => LineKind::Context,
                    _ => continue,
                };
                match kind {
                    LineKind::Added => out.additions += 1,
                    LineKind::Removed => out.deletions += 1,
                    LineKind::Context => {}
                }
                let mut text = String::from_utf8_lossy(line.content()).into_owned();
                while text.ends_with('\n') || text.ends_with('\r') {
                    text.pop();
                }
                lines.push(DiffLine {
                    kind,
                    old_no: line.old_lineno(),
                    new_no: line.new_lineno(),
                    text,
                });
            }
            out.hunks.push(Hunk {
                header: String::from_utf8_lossy(hunk.header()).trim_end().to_owned(),
                old_start: hunk.old_start(),
                old_lines: hunk.old_lines(),
                new_start: hunk.new_start(),
                new_lines: hunk.new_lines(),
                lines,
            });
        }
    }
    Ok(out)
}

/// Builds a patch containing only hunk number `index` of the single-file `diff`, byte-exact, for
/// `git apply`. Returns the hunk's key alongside so callers can check it is the one the user saw.
pub(crate) fn hunk_patch(diff: &Diff<'_>, index: usize) -> Result<Option<(Vec<u8>, HunkKey)>> {
    for idx in 0..diff.deltas().len() {
        let Some(delta) = diff.get_delta(idx) else {
            continue;
        };
        if delta.status() == git2::Delta::Typechange {
            return Err(typechange_error());
        }
        let Some(patch) = Patch::from_diff(diff, idx)? else {
            continue;
        };
        if index >= patch.num_hunks() {
            return Ok(None);
        }
        // Raw bytes, quoted as git quotes them, so names with spaces, tabs, quotes, backslashes
        // or non-ASCII (even non-UTF-8) bytes round-trip through `git apply`.
        let old = delta.old_file().path_bytes().unwrap_or_default();
        let new = delta.new_file().path_bytes().unwrap_or_default();
        let mut out = Vec::new();
        out.extend_from_slice(b"diff --git ");
        out.extend(quoted(b"a/", old));
        out.push(b' ');
        out.extend(quoted(b"b/", new));
        out.push(b'\n');
        let mode = |m: git2::FileMode| i32::from(m);
        let (old_absent, new_absent) = match delta.status() {
            git2::Delta::Added | git2::Delta::Untracked => {
                out.extend_from_slice(
                    format!("new file mode {:o}\n", mode(delta.new_file().mode())).as_bytes(),
                );
                (true, false)
            }
            git2::Delta::Deleted => {
                out.extend_from_slice(
                    format!("deleted file mode {:o}\n", mode(delta.old_file().mode())).as_bytes(),
                );
                (false, true)
            }
            _ => (false, false),
        };
        file_line(&mut out, b"--- ", b"a/", old, old_absent);
        file_line(&mut out, b"+++ ", b"b/", new, new_absent);
        let (hunk, n) = patch.hunk(index)?;
        out.extend_from_slice(hunk.header());
        if !hunk.header().ends_with(b"\n") {
            out.push(b'\n');
        }
        for l in 0..n {
            let line = patch.line_in_hunk(index, l)?;
            match line.origin() {
                '+' | '-' | ' ' => {
                    out.push(line.origin() as u8);
                    out.extend_from_slice(line.content());
                }
                // End-of-file markers: content is "\n\\ No newline at end of file\n".
                '=' | '>' | '<' => out.extend_from_slice(line.content()),
                _ => {}
            }
        }
        let key = (
            hunk.old_start(),
            hunk.old_lines(),
            hunk.new_start(),
            hunk.new_lines(),
        );
        return Ok(Some((out, key)));
    }
    Ok(None)
}

/// A `---` or `+++` line. Like `git diff`, a name containing a space is followed by a tab so
/// `git apply` knows where it ends.
pub(crate) fn file_line(out: &mut Vec<u8>, lead: &[u8], prefix: &[u8], path: &[u8], absent: bool) {
    out.extend_from_slice(lead);
    if absent {
        out.extend_from_slice(b"/dev/null");
    } else {
        out.extend(quoted(prefix, path));
        if path.contains(&b' ') {
            out.push(b'\t');
        }
    }
    out.push(b'\n');
}

/// `prefix` + `path`, C-quoted the way git quotes paths when it contains a control character,
/// a quote, a backslash or a non-ASCII byte (which also covers non-UTF-8 names).
pub(crate) fn quoted(prefix: &[u8], path: &[u8]) -> Vec<u8> {
    let special = |b: u8| !PRINTABLE_ASCII.contains(&b) || b == b'"' || b == b'\\';
    if !path.iter().copied().any(special) {
        return [prefix, path].concat();
    }
    let mut out = vec![b'"'];
    out.extend_from_slice(prefix);
    for &b in path {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            BEL => out.extend_from_slice(b"\\a"),
            BS => out.extend_from_slice(b"\\b"),
            VT => out.extend_from_slice(b"\\v"),
            FF => out.extend_from_slice(b"\\f"),
            b if special(b) => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
            b => out.push(b),
        }
    }
    out.push(b'"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths_like_git() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"plain.txt", b"a/plain.txt"),
            (b"with space.txt", b"a/with space.txt"),
            (b"tab\there", b"\"a/tab\\there\""),
            (b"q\"b\\", b"\"a/q\\\"b\\\\\""),
            ("caf\u{e9}".as_bytes(), b"\"a/caf\\303\\251\""),
            (b"raw\xff", b"\"a/raw\\377\""),
        ];
        for (path, want) in cases {
            assert_eq!(
                quoted(b"a/", path),
                *want,
                "{}",
                String::from_utf8_lossy(path)
            );
        }
    }
}
