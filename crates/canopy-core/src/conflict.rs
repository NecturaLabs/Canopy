//! Merge conflicts: parsing conflict markers in a working file, rendering a (partial) resolution,
//! and resolving conflicted paths.
//!
//! Parsing and rendering are pure text operations on [`ConflictFile`]. The [`Repo`] methods read
//! the conflict stages from the index with libgit2 and change the index only through the `git`
//! executable (`git checkout --ours/--theirs`, `git add`, `git rm`), so filters, `core.autocrlf`
//! and the executable bit behave as they do in a terminal.
//!
//! "Ours" and "theirs" follow git's own convention: during a merge "ours" is the checked-out
//! branch, during a rebase "ours" is the branch being rebased onto and "theirs" is the commit
//! being replayed. The labels git writes after the markers ([`ConflictRegion::ours_label`],
//! [`ConflictRegion::theirs_label`]) say which is which.

use std::path::{Path, PathBuf};

use crate::paths::{check_relative, safe_join, write_atomic};
use crate::{Error, Repo, Result};

/// The marker length git uses unless the `conflict-marker-size` attribute says otherwise.
pub const DEFAULT_MARKER_SIZE: usize = 7;

/// One conflicted region between `<<<<<<<` and `>>>>>>>`. Lines carry no line terminators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRegion {
    /// Lines of our side (between `<<<<<<<` and `|||||||` or `=======`).
    pub ours: Vec<String>,
    /// Lines of the merge base, present with `merge.conflictStyle` `diff3` or `zdiff3`.
    pub base: Option<Vec<String>>,
    /// Lines of their side (between `=======` and `>>>>>>>`).
    pub theirs: Vec<String>,
    /// Text after `<<<<<<<`, e.g. `HEAD`.
    pub ours_label: String,
    /// Text after `>>>>>>>`, e.g. the merged branch name.
    pub theirs_label: String,
    /// Text after `|||||||`, e.g. an abbreviated commit id; empty without a base section.
    pub base_label: String,
    /// Length of each marker: 7 unless the file uses `conflict-marker-size`.
    pub marker_size: usize,
}

impl Default for ConflictRegion {
    fn default() -> Self {
        Self {
            ours: Vec::new(),
            base: None,
            theirs: Vec::new(),
            ours_label: String::new(),
            theirs_label: String::new(),
            base_label: String::new(),
            marker_size: DEFAULT_MARKER_SIZE,
        }
    }
}

/// How to resolve one conflicted region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Keep our lines.
    Ours,
    /// Keep their lines.
    Theirs,
    /// Keep our lines followed by theirs.
    OursThenTheirs,
    /// Keep their lines followed by ours.
    TheirsThenOurs,
    /// Keep the merge base's lines (nothing when the file has no base section).
    Base,
    /// Replace the region with this text. A single trailing line break is ignored, so `"a\nb"`
    /// and `"a\nb\n"` both give two lines; `""` removes the region.
    Custom(String),
}

/// A whole-file side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The version in index stage 2 (`git checkout --ours`).
    Ours,
    /// The version in index stage 3 (`git checkout --theirs`).
    Theirs,
}

/// Which sides of a conflicted path exist, as `git status` reports it (`UU`, `AA`, `DU`…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictKind {
    /// `UU`: both sides changed the file.
    BothModified,
    /// `AA`: both sides added a file at this path.
    BothAdded,
    /// `DU`: we deleted the file, they modified it.
    DeletedByUs,
    /// `UD`: they deleted the file, we modified it.
    DeletedByThem,
    /// `AU`: only we have the file (for example after a rename on our side).
    AddedByUs,
    /// `UA`: only they have the file.
    AddedByThem,
    /// `DD`: both sides removed the file (for example renamed it differently).
    BothDeleted,
}

impl ConflictKind {
    /// User-facing name, e.g. "Deleted by us".
    pub fn label(self) -> &'static str {
        match self {
            Self::BothModified => "Both modified",
            Self::BothAdded => "Both added",
            Self::DeletedByUs => "Deleted by us",
            Self::DeletedByThem => "Deleted by them",
            Self::AddedByUs => "Added by us",
            Self::AddedByThem => "Added by them",
            Self::BothDeleted => "Both deleted",
        }
    }

    /// The two-letter code `git status --short` shows.
    pub fn code(self) -> &'static str {
        match self {
            Self::BothModified => "UU",
            Self::BothAdded => "AA",
            Self::DeletedByUs => "DU",
            Self::DeletedByThem => "UD",
            Self::AddedByUs => "AU",
            Self::AddedByThem => "UA",
            Self::BothDeleted => "DD",
        }
    }

    /// Classifies a path from which of index stages 1 (base), 2 (ours) and 3 (theirs) exist.
    /// `None` when there is neither side nor base, i.e. the path is not conflicted.
    pub fn from_stages(base: bool, ours: bool, theirs: bool) -> Option<Self> {
        Some(match (base, ours, theirs) {
            (true, true, true) => Self::BothModified,
            (false, true, true) => Self::BothAdded,
            (true, false, true) => Self::DeletedByUs,
            (true, true, false) => Self::DeletedByThem,
            (false, true, false) => Self::AddedByUs,
            (false, false, true) => Self::AddedByThem,
            (true, false, false) => Self::BothDeleted,
            (false, false, false) => return None,
        })
    }

    /// Whether `side` keeps a file at this path (its index stage exists). Choosing a side that
    /// does not removes the file.
    pub fn has_side(self, side: Side) -> bool {
        match side {
            Side::Ours => matches!(
                self,
                Self::BothModified | Self::BothAdded | Self::DeletedByThem | Self::AddedByUs
            ),
            Side::Theirs => matches!(
                self,
                Self::BothModified | Self::BothAdded | Self::DeletedByUs | Self::AddedByThem
            ),
        }
    }
}

/// A stretch of a conflicted file: unchanged text or a conflicted region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Lines outside any conflict.
    Common(Vec<String>),
    /// A conflicted region.
    Conflict(ConflictRegion),
}

/// A working file split into common text and conflicted regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFile {
    /// The file in order.
    pub segments: Vec<Segment>,
    /// The line terminator to write: `"\r\n"` when most lines end that way, otherwise `"\n"`.
    pub eol: String,
    /// Whether the file ends with a line terminator.
    pub trailing_newline: bool,
    /// [`fingerprint`] of the exact text this was parsed from.
    fingerprint: u64,
}

impl Default for ConflictFile {
    /// The same as parsing an empty file.
    fn default() -> Self {
        Self {
            segments: Vec::new(),
            eol: "\n".to_owned(),
            trailing_newline: false,
            fingerprint: fingerprint(b""),
        }
    }
}

/// A 64-bit FNV-1a hash of `bytes` followed by their length, used to notice that a file changed
/// between reading and writing it. It is deterministic across runs, builds and platforms. It is
/// not cryptographic: it catches an editor's save, not a collision crafted on purpose. Content,
/// not the modification time, because mtimes are coarse or unreliable on network mounts and WSL
/// and editors that save by renaming change them even when the text is identical.
fn fingerprint(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    bytes
        .iter()
        .chain(&len.to_le_bytes())
        .fold(OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(PRIME))
}

/// Whether `text` contains at least one complete conflict region (`<<<<<<<` … `=======` …
/// `>>>>>>>`). Stray marker-like lines such as a Markdown `=======` underline do not count.
pub fn has_conflict_markers(text: &str) -> bool {
    ConflictFile::parse(text).conflicts() > 0
}

/// If `line` is a marker made of `ch` (at least 7 of them, then nothing or a space and a label),
/// returns its length and label.
fn marker(line: &str, ch: u8) -> Option<(usize, &str)> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let size = line.bytes().take_while(|&b| b == ch).count();
    if size < DEFAULT_MARKER_SIZE {
        return None;
    }
    // `ch` is ASCII, so `size` is a char boundary.
    let rest = &line[size..];
    if rest.is_empty() {
        Some((size, ""))
    } else {
        rest.strip_prefix(' ').map(|label| (size, label))
    }
}

/// Like [`marker`], only for markers of exactly `size` characters.
fn sized_marker(line: &str, ch: u8, size: usize) -> Option<&str> {
    marker(line, ch).and_then(|(n, label)| (n == size).then_some(label))
}

fn marker_line(ch: char, size: usize, label: &str) -> String {
    let mut s: String = std::iter::repeat_n(ch, size).collect();
    if !label.is_empty() {
        s.push(' ');
        s.push_str(label);
    }
    s
}

fn owned(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| (*l).to_owned()).collect()
}

/// Parses the region opened at `lines[start]`. Returns it and the index after its closing
/// marker, or `None` when the region is not closed (end of file, or another opening marker of the
/// same size first).
fn parse_region(
    lines: &[&str],
    start: usize,
    size: usize,
    ours_label: &str,
) -> Option<(ConflictRegion, usize)> {
    enum Part {
        Ours,
        Base,
        Theirs,
    }
    let mut part = Part::Ours;
    let mut region = ConflictRegion {
        ours_label: ours_label.to_owned(),
        marker_size: size,
        ..ConflictRegion::default()
    };
    let mut base = Vec::new();
    for (i, &line) in lines.iter().enumerate().skip(start + 1) {
        if sized_marker(line, b'<', size).is_some() {
            return None;
        }
        match part {
            Part::Ours => {
                if let Some(label) = sized_marker(line, b'|', size) {
                    region.base_label = label.to_owned();
                    part = Part::Base;
                } else if sized_marker(line, b'=', size).is_some() {
                    part = Part::Theirs;
                } else if sized_marker(line, b'>', size).is_some() {
                    return None;
                } else {
                    region.ours.push(line.to_owned());
                }
            }
            Part::Base => {
                if sized_marker(line, b'=', size).is_some() {
                    region.base = Some(std::mem::take(&mut base));
                    part = Part::Theirs;
                } else if sized_marker(line, b'>', size).is_some() {
                    return None;
                } else {
                    base.push(line.to_owned());
                }
            }
            Part::Theirs => {
                if let Some(label) = sized_marker(line, b'>', size) {
                    region.theirs_label = label.to_owned();
                    return Some((region, i + 1));
                }
                region.theirs.push(line.to_owned());
            }
        }
    }
    None
}

/// Splits replacement text into lines, ignoring one trailing line break.
fn text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
        .collect()
}

impl ConflictRegion {
    /// The lines `choice` keeps.
    pub fn resolve(&self, choice: &Choice) -> Vec<String> {
        match choice {
            Choice::Ours => self.ours.clone(),
            Choice::Theirs => self.theirs.clone(),
            Choice::OursThenTheirs => self.ours.iter().chain(&self.theirs).cloned().collect(),
            Choice::TheirsThenOurs => self.theirs.iter().chain(&self.ours).cloned().collect(),
            Choice::Base => self.base.clone().unwrap_or_default(),
            Choice::Custom(text) => text_lines(text),
        }
    }

    /// The region as it appears in the file, markers included.
    pub fn marked_lines(&self) -> Vec<String> {
        let size = self.marker_size.max(DEFAULT_MARKER_SIZE);
        let mut out = Vec::with_capacity(
            self.ours.len() + self.theirs.len() + self.base.as_ref().map_or(0, Vec::len) + 4,
        );
        out.push(marker_line('<', size, &self.ours_label));
        out.extend(self.ours.iter().cloned());
        if let Some(base) = &self.base {
            out.push(marker_line('|', size, &self.base_label));
            out.extend(base.iter().cloned());
        }
        out.push(marker_line('=', size, ""));
        out.extend(self.theirs.iter().cloned());
        out.push(marker_line('>', size, &self.theirs_label));
        out
    }
}

impl ConflictFile {
    /// Parses a working file containing conflict markers. Markers are 7 characters long by
    /// default; a longer `<<<<<<<` (from `conflict-marker-size`) sets the length the rest of that
    /// region must use. Unbalanced or malformed markers are kept as ordinary text.
    pub fn parse(text: &str) -> ConflictFile {
        let mut raw: Vec<&str> = text.split('\n').collect();
        let trailing_newline = text.ends_with('\n');
        if trailing_newline || text.is_empty() {
            raw.pop();
        }
        // Every element except an unterminated last line was followed by '\n'.
        let terminated = if trailing_newline {
            raw.len()
        } else {
            raw.len().saturating_sub(1)
        };
        let crlf = raw[..terminated]
            .iter()
            .filter(|l| l.ends_with('\r'))
            .count();
        let use_crlf = crlf > 0 && crlf * 2 >= terminated;
        let lines: Vec<&str> = if use_crlf {
            raw.iter()
                .enumerate()
                .map(|(i, l)| {
                    if i < terminated {
                        l.strip_suffix('\r').unwrap_or(l)
                    } else {
                        l
                    }
                })
                .collect()
        } else {
            raw
        };

        let mut segments = Vec::new();
        let mut common: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            if let Some((size, label)) = marker(lines[i], b'<')
                && let Some((region, next)) = parse_region(&lines, i, size, label)
            {
                if !common.is_empty() {
                    segments.push(Segment::Common(owned(&common)));
                    common.clear();
                }
                segments.push(Segment::Conflict(region));
                i = next;
                continue;
            }
            common.push(lines[i]);
            i += 1;
        }
        if !common.is_empty() {
            segments.push(Segment::Common(owned(&common)));
        }

        ConflictFile {
            segments,
            eol: if use_crlf { "\r\n" } else { "\n" }.to_owned(),
            trailing_newline,
            fingerprint: fingerprint(text.as_bytes()),
        }
    }

    /// A hash of the exact text this was parsed from; equal texts give equal fingerprints. Pass
    /// it to [`Repo::write_resolution`] so a file edited on disk since is not overwritten. It
    /// describes the original text, so changing [`Self::segments`] does not change it.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Number of conflicted regions.
    pub fn conflicts(&self) -> usize {
        self.regions().count()
    }

    /// The conflicted regions in file order; index `i` matches `choices[i]` in [`Self::render`].
    pub fn regions(&self) -> impl Iterator<Item = &ConflictRegion> {
        self.segments.iter().filter_map(|s| match s {
            Segment::Conflict(r) => Some(r),
            Segment::Common(_) => None,
        })
    }

    /// Whether `choices` resolves every region.
    pub fn is_resolved(&self, choices: &[Option<Choice>]) -> bool {
        (0..self.conflicts()).all(|i| matches!(choices.get(i), Some(Some(_))))
    }

    /// The file text with region `i` replaced according to `choices[i]`. A `None` (or missing)
    /// choice writes that region back with its original markers, so a partly resolved file still
    /// shows its open conflicts. Uses [`Self::eol`] and keeps [`Self::trailing_newline`].
    pub fn render(&self, choices: &[Option<Choice>]) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut index = 0;
        for segment in &self.segments {
            match segment {
                Segment::Common(common) => lines.extend(common.iter().cloned()),
                Segment::Conflict(region) => {
                    match choices.get(index).and_then(Option::as_ref) {
                        Some(choice) => lines.extend(region.resolve(choice)),
                        None => lines.extend(region.marked_lines()),
                    }
                    index += 1;
                }
            }
        }
        if lines.is_empty() {
            return String::new();
        }
        let mut out = lines.join(&self.eol);
        if self.trailing_newline {
            out.push_str(&self.eol);
        }
        out
    }
}

/// Whether the working copy of `path` is itself a symbolic link (a link further up is caught by
/// [`safe_join`]).
fn is_link(root: &Path, path: &str) -> Result<bool> {
    let probe = root.join(check_relative(path)?);
    Ok(std::fs::symlink_metadata(probe).is_ok_and(|m| m.file_type().is_symlink()))
}

/// [`safe_join`] for a conflicted file the resolver reads or writes, refusing a symbolic link
/// with the advice to pick a side.
fn text_file(root: &Path, path: &str) -> Result<PathBuf> {
    if is_link(root, path)? {
        return Err(Error::invalid(format!(
            "{path} is a symbolic link. Choose our or their version."
        )));
    }
    safe_join(root, path)
}

impl Repo {
    /// What kind of conflict `path` has, read from index stages 1–3; `None` when it is not
    /// conflicted.
    pub fn conflict_kind(&self, path: &str) -> Result<Option<ConflictKind>> {
        let mut index = self.git().index()?;
        // The CLI changes the index behind libgit2's back.
        index.read(true)?;
        let wanted = path.as_bytes();
        for conflict in index.conflicts()? {
            let conflict = conflict?;
            let matches =
                |e: &Option<git2::IndexEntry>| e.as_ref().is_some_and(|e| e.path == wanted);
            if matches(&conflict.ancestor) || matches(&conflict.our) || matches(&conflict.their) {
                return Ok(ConflictKind::from_stages(
                    conflict.ancestor.is_some(),
                    conflict.our.is_some(),
                    conflict.their.is_some(),
                ));
            }
        }
        Ok(None)
    }

    /// Reads the working copy of `path` and splits it at its conflict markers. Fails with
    /// [`Error::Invalid`] for a file that is missing, a symbolic link, binary (contains a NUL
    /// byte) or not UTF-8, which can only be resolved by choosing a whole side.
    pub fn conflict_file(&self, path: &str) -> Result<ConflictFile> {
        let full = text_file(self.workdir(), path)?;
        match std::fs::symlink_metadata(&full) {
            Ok(m) if !m.is_file() => {
                return Err(Error::invalid(format!(
                    "{path} is not a regular file. Choose our or their version."
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::invalid(format!(
                    "{path} is not in the working tree. Choose our or their version."
                )));
            }
            Err(e) => return Err(e.into()),
        }
        let bytes = std::fs::read(&full)?;
        if bytes.contains(&0) {
            return Err(Error::invalid(format!(
                "{path} is a binary file. Choose our or their version."
            )));
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            Error::invalid(format!(
                "{path} is not UTF-8 text. Choose our or their version."
            ))
        })?;
        Ok(ConflictFile::parse(&text))
    }

    /// Resolves `path` by taking one side's whole version, like
    /// `git checkout --ours -- path && git add -- path`. When the chosen side deleted the file
    /// (for example [`Side::Theirs`] on [`ConflictKind::DeletedByThem`]) the path is removed with
    /// `git rm`. Any edits in the working copy are replaced.
    ///
    /// Unlike [`Self::write_resolution`] this takes no [`ConflictFile::fingerprint`], on purpose.
    /// What it writes comes from the index, not from an earlier parse of the file, so a stale
    /// parse cannot change the result: "use ours" means the same whatever the working copy holds
    /// now, and the explicit choice wins over a hand edit, as it does with
    /// `git checkout --ours`. It also has to work for binary files, symbolic links and deleted
    /// files, which have no parse to fingerprint. Callers should confirm first, since edits in
    /// the working copy cannot be recovered.
    pub fn resolve_with_side(&self, path: &str, side: Side) -> Result<()> {
        check_relative(path)?;
        let Some(kind) = self.conflict_kind(path)? else {
            return Err(Error::invalid(format!("{path} is not conflicted.")));
        };
        if kind.has_side(side) {
            let flag = match side {
                Side::Ours => "--ours",
                Side::Theirs => "--theirs",
            };
            self.git_cmd(&["--literal-pathspecs", "checkout", flag, "--", path])?;
            self.git_cmd(&["--literal-pathspecs", "add", "--", path])?;
        } else {
            self.git_cmd(&["--literal-pathspecs", "rm", "-q", "--", path])?;
        }
        Ok(())
    }

    /// Replaces the working copy of `path` with `content` (written atomically, keeping the file's
    /// mode) and, when `mark_resolved` is set, stages it with `git add`.
    ///
    /// `expected` is the [`ConflictFile::fingerprint`] of the parse `content` was rendered from.
    /// When the file on disk no longer matches it (edited in another program, or removed) this
    /// fails with [`Error::ChangedOnDisk`]. Refuses to mark a file resolved while `content` still
    /// contains a conflict region. When it refuses, nothing is written or staged.
    pub fn write_resolution(
        &self,
        path: &str,
        content: &str,
        mark_resolved: bool,
        expected: u64,
    ) -> Result<()> {
        if mark_resolved && has_conflict_markers(content) {
            return Err(unresolved_markers(path));
        }
        let full = text_file(self.workdir(), path)?;
        // The check and the write are not atomic: a save landing between them is still lost.
        // That window is the microseconds between this read and the rename in `write_atomic`,
        // instead of the file watcher's debounce, and editors honour no lock that could close it.
        let on_disk = match std::fs::read(&full) {
            Ok(bytes) => Some(fingerprint(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if on_disk != Some(expected) {
            return Err(Error::ChangedOnDisk(path.to_owned()));
        }
        write_atomic(&full, content.as_bytes())?;
        if mark_resolved {
            self.git_cmd(&["--literal-pathspecs", "add", "--", path])?;
        }
        Ok(())
    }

    /// Marks `path` resolved with `git add` (a deleted working copy stages the deletion). Refuses
    /// while the working copy still contains a conflict region.
    pub fn mark_resolved(&self, path: &str) -> Result<()> {
        // A symbolic link is staged as a link; its target is never read.
        if !is_link(self.workdir(), path)? {
            let full = safe_join(self.workdir(), path)?;
            if std::fs::symlink_metadata(&full).is_ok_and(|m| m.is_file()) {
                let bytes = std::fs::read(&full)?;
                if !bytes.contains(&0) && has_conflict_markers(&String::from_utf8_lossy(&bytes)) {
                    return Err(unresolved_markers(path));
                }
            }
        }
        self.git_cmd(&["--literal-pathspecs", "add", "--", path])
            .map(drop)
    }
}

fn unresolved_markers(path: &str) -> Error {
    Error::invalid(format!(
        "{path} still contains conflict markers. Resolve every conflict first."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| (*s).to_owned()).collect()
    }

    const TWO_WAY: &str =
        "top\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> feature\nbottom\n";

    #[test]
    fn parses_two_way_markers() {
        let f = ConflictFile::parse(TWO_WAY);
        assert_eq!(f.eol, "\n");
        assert!(f.trailing_newline);
        assert_eq!(f.conflicts(), 1);
        assert_eq!(
            f.segments,
            vec![
                Segment::Common(v(&["top"])),
                Segment::Conflict(ConflictRegion {
                    ours: v(&["ours 1", "ours 2"]),
                    base: None,
                    theirs: v(&["theirs"]),
                    ours_label: "HEAD".into(),
                    theirs_label: "feature".into(),
                    ..ConflictRegion::default()
                }),
                Segment::Common(v(&["bottom"])),
            ]
        );
    }

    #[test]
    fn render_applies_each_choice() {
        let f = ConflictFile::parse(TWO_WAY);
        let cases: [(Choice, &str); 6] = [
            (Choice::Ours, "top\nours 1\nours 2\nbottom\n"),
            (Choice::Theirs, "top\ntheirs\nbottom\n"),
            (
                Choice::OursThenTheirs,
                "top\nours 1\nours 2\ntheirs\nbottom\n",
            ),
            (
                Choice::TheirsThenOurs,
                "top\ntheirs\nours 1\nours 2\nbottom\n",
            ),
            (Choice::Base, "top\nbottom\n"),
            (Choice::Custom("mine\n".into()), "top\nmine\nbottom\n"),
        ];
        for (choice, expected) in cases {
            assert_eq!(f.render(&[Some(choice.clone())]), expected, "{choice:?}");
        }
        assert_eq!(
            f.render(&[Some(Choice::Custom(String::new()))]),
            "top\nbottom\n"
        );
    }

    #[test]
    fn parses_diff3_base_section() {
        let text = "<<<<<<< HEAD\nO\n||||||| 2ec4c1a\nB1\nB2\n=======\nT\n>>>>>>> other\n";
        let f = ConflictFile::parse(text);
        let r = f.regions().next().expect("region");
        assert_eq!(r.base.as_deref(), Some(&v(&["B1", "B2"])[..]));
        assert_eq!(r.base_label, "2ec4c1a");
        assert_eq!(f.render(&[Some(Choice::Base)]), "B1\nB2\n");
        assert_eq!(f.render(&[None]), text);
        assert_eq!(f.render(&[]), text);
    }

    #[test]
    fn empty_base_section_is_some_empty() {
        let f = ConflictFile::parse("<<<<<<< a\nx\n|||||||\n=======\ny\n>>>>>>> b\n");
        let r = f.regions().next().expect("region");
        assert_eq!(r.base, Some(Vec::new()));
        assert_eq!(r.base_label, "");
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let text = "a\r\n<<<<<<< HEAD\r\nO\r\n=======\r\nT\r\n>>>>>>> b\r\nz\r\n";
        let f = ConflictFile::parse(text);
        assert_eq!(f.eol, "\r\n");
        let r = f.regions().next().expect("region");
        assert_eq!(r.ours, v(&["O"]));
        assert_eq!(r.theirs_label, "b");
        assert_eq!(f.render(&[Some(Choice::Theirs)]), "a\r\nT\r\nz\r\n");
        assert_eq!(f.render(&[None]), text);
    }

    #[test]
    fn keeps_missing_trailing_newline() {
        let text = "a\n<<<<<<< HEAD\nO\n=======\nT\n>>>>>>> b\nlast";
        let f = ConflictFile::parse(text);
        assert!(!f.trailing_newline);
        assert_eq!(f.render(&[Some(Choice::Ours)]), "a\nO\nlast");
        assert_eq!(f.render(&[None]), text);

        let ends_in_marker = "<<<<<<< HEAD\nO\n=======\nT\n>>>>>>> b";
        let f = ConflictFile::parse(ends_in_marker);
        assert_eq!(f.conflicts(), 1);
        assert_eq!(f.render(&[Some(Choice::Theirs)]), "T");
        assert_eq!(f.render(&[None]), ends_in_marker);
    }

    #[test]
    fn mixed_choices_and_unresolved_regions() {
        let text = "\
head
<<<<<<< HEAD
o1
=======
t1
>>>>>>> x
mid
<<<<<<< HEAD
o2
=======
t2
>>>>>>> x
mid2
<<<<<<< HEAD
o3
=======
t3
>>>>>>> x
tail
";
        let f = ConflictFile::parse(text);
        assert_eq!(f.conflicts(), 3);
        let choices = [Some(Choice::Theirs), None, Some(Choice::OursThenTheirs)];
        assert!(!f.is_resolved(&choices));
        let out = f.render(&choices);
        assert_eq!(
            out,
            "head\nt1\nmid\n<<<<<<< HEAD\no2\n=======\nt2\n>>>>>>> x\nmid2\no3\nt3\ntail\n"
        );
        assert!(has_conflict_markers(&out));
        let all = [
            Some(Choice::Ours),
            Some(Choice::Custom("c".into())),
            Some(Choice::Base),
        ];
        assert!(f.is_resolved(&all));
        let out = f.render(&all);
        assert_eq!(out, "head\no1\nmid\nc\nmid2\ntail\n");
        assert!(!has_conflict_markers(&out));
    }

    #[test]
    fn malformed_markers_stay_common_text() {
        let cases: &[(&str, &str)] = &[
            ("unclosed", "a\n<<<<<<< HEAD\nb\n=======\nc\n"),
            ("no separator", "<<<<<<< HEAD\nb\n>>>>>>> x\n"),
            ("stray separator", "title\n=======\ntext\n"),
            ("stray closer", ">>>>>>> x\n"),
            ("short marker", "<<<<<< a\nb\n======\nc\n>>>>>> d\n"),
            (
                "marker glued to text",
                "<<<<<<<a\nb\n=======\nc\n>>>>>>>d\n",
            ),
            ("closer inside base", "<<<<<<< a\nb\n||||||| c\n>>>>>>> d\n"),
            ("only opener", "<<<<<<<"),
            ("empty", ""),
        ];
        for (name, text) in cases {
            let f = ConflictFile::parse(text);
            assert_eq!(f.conflicts(), 0, "{name}");
            assert_eq!(f.render(&[]), *text, "{name}");
            assert!(!has_conflict_markers(text), "{name}");
        }
    }

    #[test]
    fn unmatched_opener_before_a_real_conflict() {
        let text = "<<<<<<< stray\nx\n<<<<<<< HEAD\nO\n=======\nT\n>>>>>>> b\n";
        let f = ConflictFile::parse(text);
        assert_eq!(f.conflicts(), 1);
        assert_eq!(f.segments[0], Segment::Common(v(&["<<<<<<< stray", "x"])));
        assert_eq!(f.render(&[Some(Choice::Ours)]), "<<<<<<< stray\nx\nO\n");
    }

    #[test]
    fn longer_markers_set_the_region_size() {
        // conflict-marker-size=9; 7-character lines inside are content.
        let text = "<<<<<<<<< HEAD\n=======\n=========\n>>>>>>> not a marker\n>>>>>>>>> b\n";
        let f = ConflictFile::parse(text);
        assert_eq!(f.conflicts(), 1);
        let r = f.regions().next().expect("region");
        assert_eq!(r.marker_size, 9);
        assert_eq!(r.ours, v(&["======="]));
        assert_eq!(r.theirs, v(&[">>>>>>> not a marker"]));
        assert_eq!(f.render(&[None]), text);
    }

    #[test]
    fn custom_text_line_breaks() {
        let r = ConflictRegion::default();
        assert_eq!(
            r.resolve(&Choice::Custom("a\r\nb\r\n".into())),
            v(&["a", "b"])
        );
        assert_eq!(r.resolve(&Choice::Custom("\n".into())), v(&[""]));
        assert_eq!(r.resolve(&Choice::Custom("a\n\n".into())), v(&["a", ""]));
    }

    #[test]
    fn fingerprint_follows_the_bytes_not_the_parse() {
        assert_eq!(ConflictFile::default(), ConflictFile::parse(""));
        // Different bytes give different fingerprints, even when (`same_parse`) both texts parse
        // to the same segments, eol and trailing newline.
        let pairs: &[(&str, &str, &str, bool)] = &[
            ("mixed vs all CRLF", "a\r\nb\n", "a\r\nb\r\n", true),
            ("same length", "x\nOURS\n", "x\nMINE\n", false),
            ("newline vs empty", "\n", "", false),
        ];
        for &(name, a, b, same_parse) in pairs {
            let (fa, fb) = (ConflictFile::parse(a), ConflictFile::parse(b));
            if same_parse {
                assert_eq!(
                    (&fa.segments, &fa.eol, fa.trailing_newline),
                    (&fb.segments, &fb.eol, fb.trailing_newline),
                    "{name}"
                );
            }
            assert_ne!(fa.fingerprint(), fb.fingerprint(), "{name}");
        }
        // Editing the parsed segments does not change what the file on disk looked like.
        let mut f = ConflictFile::parse(TWO_WAY);
        let before = f.fingerprint();
        f.segments.clear();
        assert_eq!(f.fingerprint(), before);
    }

    #[test]
    fn kinds_from_stages() {
        let cases = [
            ((true, true, true), Some(ConflictKind::BothModified)),
            ((false, true, true), Some(ConflictKind::BothAdded)),
            ((true, false, true), Some(ConflictKind::DeletedByUs)),
            ((true, true, false), Some(ConflictKind::DeletedByThem)),
            ((false, true, false), Some(ConflictKind::AddedByUs)),
            ((false, false, true), Some(ConflictKind::AddedByThem)),
            ((true, false, false), Some(ConflictKind::BothDeleted)),
            ((false, false, false), None),
        ];
        for ((b, o, t), expected) in cases {
            let kind = ConflictKind::from_stages(b, o, t);
            assert_eq!(kind, expected, "{b} {o} {t}");
            if let Some(k) = kind {
                assert_eq!(k.has_side(Side::Ours), o, "{k:?} ours");
                assert_eq!(k.has_side(Side::Theirs), t, "{k:?} theirs");
            }
        }
    }

    #[test]
    fn rejects_paths_outside_the_working_tree() {
        for bad in ["", "../x", "/etc/passwd", "a/../../x", ".git/config"] {
            assert!(check_relative(bad).is_err(), "{bad}");
        }
        for good in ["a.txt", "dir/with space/f", "./a"] {
            assert!(check_relative(good).is_ok(), "{good}");
        }
    }
}
