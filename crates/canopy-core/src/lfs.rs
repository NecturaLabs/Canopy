//! Git LFS awareness without running git-lfs: which paths the `filter=lfs` attribute covers,
//! pointer files, and whether a pointer's object has been downloaded.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::SystemTime;

use git2::{AttrCheckFlags, FileMode, Oid, Patch, Repository};
use sha2::{Digest, Sha256};

use crate::diff::{self, DiffSource, FileDiff, Whitespace};
use crate::model::{ChangeKind, WorkingStatus};
use crate::{Error, Repo, Result, cli};

/// Pointer files are smaller than this (git-lfs specification).
const MAX_POINTER_SIZE: usize = 1024;
/// The length of a SHA-256 in hex, which names an LFS object.
const SHA256_HEX_LEN: usize = 64;
/// LFS stores an object under two directories named by its first two pairs of hex digits.
const FAN_OUT: usize = 2;
/// A git index entry keeps its merge stage in bits 12 and 13 of its flags.
const INDEX_STAGE_SHIFT: u32 = 12;
const INDEX_STAGE_MASK: u16 = 0x3;
/// The most bytes of one side the content diff of an LFS path reads.
const CONTENT_DIFF_LIMIT: u64 = 1024 * 1024;
/// The chunk a file is read in to hash it.
const HASH_CHUNK: usize = 64 * 1024;
/// The most results of "is this file's content its pointer" kept; more than this and the cache
/// starts over.
const CLEAN_CACHE_LIMIT: usize = 4096;
/// A line of `git lfs locks` has a path, an owner and an id.
const LOCK_FIELDS: usize = 3;
/// git-lfs reads these as `git lfs <subcommand>`: the repository's own `alias.lfs` must never
/// answer in place of the program.
const NO_ALIAS: [&str; 2] = ["-c", "alias.lfs="];
/// git-lfs shows its progress only when it writes to a terminal, unless told to.
const FORCE_PROGRESS: (&str, &str) = ("GIT_LFS_FORCE_PROGRESS", "1");

fn is_sha256_hex(text: &str) -> bool {
    text.len() == SHA256_HEX_LEN
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The contents of a Git LFS pointer file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsPointer {
    /// The object's SHA-256, 64 lowercase hex digits.
    pub oid: String,
    /// The object's size in bytes.
    pub size: u64,
    /// The pointer lists `ext-` extensions, so the stored object is not the file's content.
    pub extensions: bool,
}

impl LfsPointer {
    /// Parses a pointer file: a `version` line, then `key value` lines with `oid sha256:<hex>`
    /// and `size <bytes>`, under 1024 bytes in all. `None` for anything else.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() >= MAX_POINTER_SIZE {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.lines();
        let version = lines.next()?.strip_prefix("version ")?;
        if !matches!(
            version,
            "https://git-lfs.github.com/spec/v1" | "https://hawser.github.com/spec/v1"
        ) {
            return None;
        }
        let (mut oid, mut size, mut extensions) = (None, None, false);
        for line in lines.filter(|l| !l.is_empty()) {
            let (key, value) = line.split_once(' ')?;
            match key {
                "oid" => {
                    let hex = value.strip_prefix("sha256:")?;
                    oid = is_sha256_hex(hex).then(|| hex.to_owned());
                }
                "size" => size = value.parse().ok(),
                k if k.starts_with("ext-") => extensions = true,
                _ => {}
            }
        }
        Some(Self {
            oid: oid?,
            size: size?,
            extensions,
        })
    }
}

/// A file stored in Git LFS, as the index records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsFile {
    /// Path relative to the repository root, `/`-separated.
    pub path: String,
    /// The pointer in the index.
    pub pointer: LfsPointer,
    /// The object is in the local LFS store. When it is not, the working tree holds the pointer
    /// text instead of the content (a clone with `GIT_LFS_SKIP_SMUDGE=1`, or a failed download)
    /// and `git lfs pull` fetches it.
    pub downloaded: bool,
}

/// What one side of an LFS path's diff is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsSide {
    /// The stored object's SHA-256; `None` for a working-tree file, which is not hashed to show
    /// a diff.
    pub oid: Option<String>,
    /// The object's size: the pointer's `size`, or the file's.
    pub size: u64,
    /// The pointer file's text, when this side is a pointer (and not the content itself).
    pub pointer: Option<String>,
    /// The content is here: a pointer's object is in the local store, or the side is content.
    pub downloaded: bool,
}

/// Why the content of an LFS path is not diffed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfsOmitted {
    /// An object is not in the local store: `git lfs pull` downloads it.
    NotDownloaded,
    /// A side is binary.
    Binary,
    /// A side is larger than the preview reads, or has extensions.
    TooLarge,
}

/// The diff of a path stored with Git LFS: its pointers, old to new. Hunks are in
/// [`FileDiff::hunks`] only when the content of every side is local, small and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsDiff {
    pub old: Option<LfsSide>,
    pub new: Option<LfsSide>,
    /// Why there are no hunks, when a side exists.
    pub omitted: Option<LfsOmitted>,
}

impl LfsDiff {
    /// A side is a pointer whose object is not downloaded.
    pub fn pointer_only(&self) -> bool {
        [&self.old, &self.new]
            .into_iter()
            .flatten()
            .any(|s| s.pointer.is_some() && !s.downloaded)
    }
}

/// A pattern `.gitattributes` stores with LFS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsPattern {
    pub pattern: String,
    /// The attributes file that sets it.
    pub source: String,
    /// `git lfs lock` is allowed on the files it matches only when they are locked first.
    pub lockable: bool,
}

/// A file lock held on the LFS server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsLock {
    pub id: String,
    pub path: String,
    pub owner: String,
}

/// A side with its content, when it can be read.
struct Loaded {
    side: LfsSide,
    /// The content to compare: `None` when it is not local, too large, or not the content.
    bytes: Option<Vec<u8>>,
}

/// Whether the `filter` attribute of `path` is `lfs`.
pub(crate) fn is_lfs(repo: &Repository, path: &Path) -> bool {
    matches!(
        repo.get_attr(path, "filter", AttrCheckFlags::FILE_THEN_INDEX),
        Ok(Some("lfs"))
    )
}

/// Hunk and line staging refuse a path stored with LFS: a hunk of its working file is not a
/// change to the pointer Git stores, so only the whole file can be staged (git runs the clean
/// filter for that).
pub(crate) fn refuse_partial(repo: &Repository, path: &str) -> Result<()> {
    if is_lfs(repo, Path::new(path)) {
        return Err(Error::invalid(
            "This file is stored with Git LFS. Stage or discard the whole file instead of a part of it.",
        ));
    }
    Ok(())
}

/// Sets [`crate::FileEntry::lfs`] on every entry whose path has `filter=lfs`.
pub(crate) fn mark(repo: &Repository, status: &mut WorkingStatus) {
    for list in [
        &mut status.staged,
        &mut status.unstaged,
        &mut status.conflicted,
    ] {
        for entry in list.iter_mut() {
            entry.lfs = is_lfs(repo, Path::new(&entry.path));
        }
    }
}

/// What a cached "does this file's content equal its pointer" answer holds for: the same file
/// at the same size and modification time, against the same pointer blob.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CleanKey {
    workdir: PathBuf,
    path: String,
    size: u64,
    modified: Option<SystemTime>,
    blob: Oid,
}

static CLEAN: LazyLock<Mutex<HashMap<CleanKey, bool>>> = LazyLock::new(Mutex::default);

/// The SHA-256 of a file, hex, read in chunks.
fn sha256_hex(file: &mut std::fs::File) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; HASH_CHUNK];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    let mut hex = String::with_capacity(SHA256_HEX_LEN);
    for byte in hasher.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// Whether the working file of an LFS path holds exactly what its pointer in the index names:
/// libgit2 does not run the `filter.lfs` driver, so a file whose stat changed is hashed as is and
/// differs from the pointer blob although `git status` calls it clean. `false` whenever that
/// cannot be told (no pointer, extensions, another size, unreadable).
fn matches_pointer(repo: &Repository, workdir: &Path, path: &str) -> bool {
    let Ok(mut index) = repo.index() else {
        return false;
    };
    // libgit2 keeps the index loaded; the git CLI may have rewritten it since.
    if index.read(false).is_err() {
        return false;
    }
    let Some(entry) = index.get_path(Path::new(path), 0) else {
        return false;
    };
    let Ok(blob) = repo.find_blob(entry.id) else {
        return false;
    };
    let Some(pointer) = LfsPointer::parse(blob.content()) else {
        return false;
    };
    if pointer.extensions {
        return false;
    }
    let Ok(full) = diff::workdir_path(workdir, path) else {
        return false;
    };
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return false;
    };
    if !meta.is_file() || meta.len() != pointer.size {
        return false;
    }
    let key = CleanKey {
        workdir: workdir.to_path_buf(),
        path: path.to_owned(),
        size: meta.len(),
        modified: meta.modified().ok(),
        blob: entry.id,
    };
    if let Some(known) = CLEAN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return *known;
    }
    let same = std::fs::File::open(&full)
        .and_then(|mut file| sha256_hex(&mut file))
        .is_ok_and(|hex| hex == pointer.oid);
    let mut cache = CLEAN.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.len() >= CLEAN_CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(key, same);
    same
}

/// Removes from the unstaged list the LFS files libgit2 calls modified only because their stat
/// changed (see [`matches_pointer`]). A change of mode stays listed.
pub(crate) fn drop_unchanged(repo: &Repository, status: &mut WorkingStatus) {
    let Some(workdir) = repo.workdir() else {
        return;
    };
    status.unstaged.retain(|e| {
        !(e.lfs
            && e.kind == ChangeKind::Modified
            && e.old_mode == e.new_mode
            && matches_pointer(repo, workdir, &e.path))
    });
}

impl Repo {
    /// The diff of a path stored with LFS: the pointers old to new instead of a diff of the file
    /// (which could be gigabytes), and the content's own diff only when every side is local,
    /// small and text. Reads the working file only when it is small or is a pointer.
    pub(crate) fn lfs_file_diff(
        &self,
        source: &DiffSource,
        path: &str,
        old_path: Option<&str>,
        context: u32,
        whitespace: Whitespace,
    ) -> Result<FileDiff> {
        let d = self.raw_diff(source, path, old_path, context, whitespace)?;
        let mut out = FileDiff {
            path: path.to_owned(),
            ..FileDiff::default()
        };
        let Some(delta) = d.get_delta(0) else {
            return Ok(out);
        };
        out.kind = diff::delta_kind(delta.status());
        out.old_mode = diff::mode_of(&delta.old_file());
        out.new_mode = diff::mode_of(&delta.new_file());
        if let Some(new) = diff::path_of(&delta.new_file()) {
            out.path = new;
        }
        let old_name = diff::path_of(&delta.old_file());
        if delta.status() == git2::Delta::Renamed && old_name.as_deref() != Some(out.path.as_str())
        {
            out.old_path.clone_from(&old_name);
        }
        let old = self.blob_side(delta.old_file().id())?;
        let new = match source {
            DiffSource::Unstaged => self.working_side(&out.path)?,
            _ => self.blob_side(delta.new_file().id())?,
        };
        let omitted = omitted(old.as_ref(), new.as_ref());
        if omitted.is_none() && (old.is_some() || new.is_some()) {
            let mut opts = diff::options(&out.path, None, context, whitespace);
            let old_name = old_name.unwrap_or_else(|| out.path.clone());
            let patch = Patch::from_buffers(
                old.as_ref().and_then(|l| l.bytes.as_deref()).unwrap_or(&[]),
                old.as_ref().map(|_| Path::new(&old_name)),
                new.as_ref().and_then(|l| l.bytes.as_deref()).unwrap_or(&[]),
                new.as_ref().map(|_| Path::new(&out.path)),
                Some(&mut opts),
            )?;
            diff::fill_hunks(&mut out, &patch)?;
        }
        out.lfs = Some(LfsDiff {
            old: old.map(|l| l.side),
            new: new.map(|l| l.side),
            omitted,
        });
        Ok(out)
    }

    /// One side from a blob of the object database; `None` for no blob.
    fn blob_side(&self, id: Oid) -> Result<Option<Loaded>> {
        if id.is_zero() {
            return Ok(None);
        }
        let blob = self.git().find_blob(id)?;
        Ok(Some(self.side_of(
            blob.content(),
            blob.size() as u64,
            || (blob.size() as u64 <= CONTENT_DIFF_LIMIT).then(|| blob.content().to_vec()),
        )))
    }

    /// The working file as a side; `None` when there is none (or it is not a regular file).
    fn working_side(&self, path: &str) -> Result<Option<Loaded>> {
        let full = diff::workdir_path(self.workdir(), path)?;
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if !meta.is_file() {
            return Ok(None);
        }
        let bytes = diff::read_workdir_file(&full, &meta, CONTENT_DIFF_LIMIT)?;
        let head = bytes.clone().unwrap_or_default();
        Ok(Some(self.side_of(&head, meta.len(), || bytes)))
    }

    /// A pointer's side (with its object's content, when local), or the content itself.
    /// `head` is the start of the file, enough to tell a pointer; `content` its whole content
    /// when that is small enough to compare.
    fn side_of(&self, head: &[u8], size: u64, content: impl FnOnce() -> Option<Vec<u8>>) -> Loaded {
        let Some(pointer) = LfsPointer::parse(head) else {
            return Loaded {
                side: LfsSide {
                    oid: None,
                    size,
                    pointer: None,
                    downloaded: true,
                },
                bytes: content(),
            };
        };
        let object = self.lfs_object_path(&pointer.oid);
        let meta = object
            .as_deref()
            .and_then(|p| std::fs::symlink_metadata(p).ok())
            .filter(std::fs::Metadata::is_file);
        let bytes = match (&object, &meta) {
            (Some(path), Some(meta)) if !pointer.extensions => {
                diff::read_workdir_file(path, meta, CONTENT_DIFF_LIMIT)
                    .ok()
                    .flatten()
            }
            _ => None,
        };
        Loaded {
            side: LfsSide {
                oid: Some(pointer.oid),
                size: pointer.size,
                pointer: Some(String::from_utf8_lossy(head).into_owned()),
                downloaded: meta.is_some(),
            },
            bytes,
        }
    }

    /// `git lfs <args>` in the working folder, with its output.
    fn lfs_run(&self, args: &[&str]) -> Result<String> {
        cli::run(self.workdir(), &lfs_args(args))
    }

    /// `git lfs <args>`, streaming git-lfs's progress lines.
    fn lfs_stream(&self, args: &[&str], progress: impl FnMut(&str)) -> Result<String> {
        cli::run_streaming_env(
            Some(self.workdir()),
            &lfs_args(args),
            &[FORCE_PROGRESS],
            progress,
        )
    }

    /// `git lfs pull`: downloads the LFS objects of the current checkout and writes them into
    /// the working tree where only pointers were. A network operation.
    pub fn lfs_pull(&self, progress: impl FnMut(&str)) -> Result<String> {
        self.lfs_stream(&["pull"], progress)
    }

    /// `git lfs fetch`: downloads the LFS objects of the current branch into the local store,
    /// leaving the working tree alone. A network operation.
    pub fn lfs_fetch(&self, progress: impl FnMut(&str)) -> Result<String> {
        self.lfs_stream(&["fetch"], progress)
    }

    /// Starts storing the files `pattern` matches with LFS (`git lfs track`), which changes
    /// `.gitattributes` in the working folder. With `literal` the pattern is a file name, its
    /// special characters escaped, not a glob.
    pub fn lfs_track(&self, pattern: &str, literal: bool) -> Result<String> {
        check_pattern(pattern)?;
        let mut args = vec!["track"];
        if literal {
            args.push("--filename");
        }
        args.extend(["--", pattern]);
        self.lfs_run(&args)?;
        let mut done = format!("Now tracking {pattern} with Git LFS; .gitattributes changed.");
        if self.config_string("filter.lfs.clean").is_none() {
            done.push_str(
                " Git LFS is not set up for this repository: run git lfs install, or the \
                 files are stored as they are.",
            );
        }
        Ok(done)
    }

    /// Stops storing the files `pattern` (as `.gitattributes` spells it) with LFS
    /// (`git lfs untrack`). Files already committed stay pointers until they are migrated.
    pub fn lfs_untrack(&self, pattern: &str) -> Result<String> {
        check_pattern(pattern)?;
        self.lfs_run(&["untrack", "--", pattern])?;
        Ok(format!(
            "No longer tracking {pattern} with Git LFS; .gitattributes changed."
        ))
    }

    /// The patterns stored with LFS (`git lfs track`). Needs git-lfs.
    pub fn lfs_patterns(&self) -> Result<Vec<LfsPattern>> {
        Ok(parse_patterns(&self.lfs_run(&["track"])?))
    }

    /// The locks on the LFS server (`git lfs locks`). A network operation.
    pub fn lfs_locks(&self) -> Result<Vec<LfsLock>> {
        Ok(parse_locks(&self.lfs_run(&["locks"])?))
    }

    /// Locks `path` for the user on the LFS server (`git lfs lock`). A network operation.
    pub fn lfs_lock(&self, path: &str) -> Result<String> {
        crate::paths::check_relative(path)?;
        self.lfs_run(&["lock", "--", path])
            .map(|out| out.trim().to_owned())
    }

    /// Releases the lock on `path` (`git lfs unlock`); `force` also releases another user's.
    /// A network operation.
    pub fn lfs_unlock(&self, path: &str, force: bool) -> Result<String> {
        crate::paths::check_relative(path)?;
        let mut args = vec!["unlock"];
        if force {
            args.push("--force");
        }
        args.extend(["--", path]);
        self.lfs_run(&args).map(|out| out.trim().to_owned())
    }

    /// The local LFS store: `lfs.storage` when set (relative to the common git directory when not
    /// absolute), otherwise `lfs` in the common git directory, as git-lfs resolves it.
    pub fn lfs_storage_dir(&self) -> PathBuf {
        let common = self.git().commondir().to_path_buf();
        match self.config_string("lfs.storage").filter(|s| !s.is_empty()) {
            Some(dir) => common.join(dir),
            None => common.join("lfs"),
        }
    }

    /// Where the object `oid` lives in the local store: `objects/<oid[0..2]>/<oid[2..4]>/<oid>`.
    /// `None` unless `oid` is a SHA-256 in lowercase hex.
    pub fn lfs_object_path(&self, oid: &str) -> Option<PathBuf> {
        if !is_sha256_hex(oid) {
            return None;
        }
        let (a, b) = (oid.get(..FAN_OUT)?, oid.get(FAN_OUT..FAN_OUT + FAN_OUT)?);
        Some(
            self.lfs_storage_dir()
                .join("objects")
                .join(a)
                .join(b)
                .join(oid),
        )
    }

    /// Every file in the index (stage 0) whose path has `filter=lfs` and whose blob is a valid
    /// pointer, with whether its object is downloaded. Reads every index entry's attributes and
    /// each small blob, so run it as its own job rather than on every refresh.
    pub fn lfs_files(&self) -> Result<Vec<LfsFile>> {
        let git = self.git();
        let mut index = git.index()?;
        // libgit2 keeps the index loaded; the git CLI may have rewritten it since.
        index.read(false)?;
        let mut out = Vec::new();
        for entry in index.iter() {
            let stage = (entry.flags >> INDEX_STAGE_SHIFT) & INDEX_STAGE_MASK;
            let mode = entry.mode;
            if stage != 0
                || mode == u32::from(FileMode::Commit)
                || mode == u32::from(FileMode::Link)
            {
                continue;
            }
            let path = String::from_utf8_lossy(&entry.path).into_owned();
            if !is_lfs(git, Path::new(&path)) {
                continue;
            }
            let Ok(blob) = git.find_blob(entry.id) else {
                continue;
            };
            let Some(pointer) = LfsPointer::parse(blob.content()) else {
                continue;
            };
            let downloaded = self
                .lfs_object_path(&pointer.oid)
                .is_some_and(|p| p.is_file());
            out.push(LfsFile {
                path,
                pointer,
                downloaded,
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}

/// The arguments of `git lfs <args>`, with the repository's own `alias.lfs` emptied.
fn lfs_args<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut all: Vec<&str> = NO_ALIAS.to_vec();
    all.push("lfs");
    all.extend(args);
    all
}

/// Why the content of two sides is not diffed: `None` when it is, or when there is nothing to
/// compare.
fn omitted(old: Option<&Loaded>, new: Option<&Loaded>) -> Option<LfsOmitted> {
    let sides: Vec<&Loaded> = [old, new].into_iter().flatten().collect();
    if sides
        .iter()
        .any(|l| l.side.pointer.is_some() && !l.side.downloaded)
    {
        return Some(LfsOmitted::NotDownloaded);
    }
    if sides.iter().any(|l| l.bytes.is_none()) {
        return Some(LfsOmitted::TooLarge);
    }
    sides
        .iter()
        .any(|l| l.bytes.as_deref().is_some_and(|b| b.contains(&0)))
        .then_some(LfsOmitted::Binary)
}

/// A pattern `git lfs track` or `untrack` can take: not empty, not an option, one line.
fn check_pattern(pattern: &str) -> Result<()> {
    if pattern.trim().is_empty() {
        return Err(Error::invalid("Give a file pattern, such as *.psd."));
    }
    if pattern.starts_with('-') || pattern.chars().any(char::is_control) {
        return Err(Error::invalid(format!(
            "{pattern:?} is not a file pattern: it starts with - or has a control character."
        )));
    }
    Ok(())
}

/// The patterns in `git lfs track`'s listing: indented `pattern (source)` lines under
/// "Listing tracked patterns", with `[lockable]` after a lockable one.
fn parse_patterns(listing: &str) -> Vec<LfsPattern> {
    let mut out = Vec::new();
    let mut tracked = false;
    for line in listing.lines() {
        if line.starts_with("Listing tracked") {
            tracked = true;
        } else if line.starts_with("Listing") {
            tracked = false;
        } else if tracked && line.starts_with(char::is_whitespace) {
            let Some((rest, source)) = line.trim().rsplit_once(" (") else {
                continue;
            };
            let (pattern, lockable) = rest
                .strip_suffix(" [lockable]")
                .map_or((rest, false), |p| (p, true));
            out.push(LfsPattern {
                pattern: pattern.to_owned(),
                source: source.trim_end_matches(')').to_owned(),
                lockable,
            });
        }
    }
    out
}

/// The locks in `git lfs locks`' listing: `path`, owner and `ID:<id>`, tab separated; git-lfs
/// pads the first two with spaces to line the columns up.
fn parse_locks(listing: &str) -> Vec<LfsLock> {
    listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(LOCK_FIELDS, '\t');
            let (path, owner, id) = (parts.next()?, parts.next()?, parts.next()?);
            Some(LfsLock {
                id: id.trim().strip_prefix("ID:")?.to_owned(),
                path: path.trim_end().to_owned(),
                owner: owner.trim().to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

    #[test]
    fn parses_pointers() {
        let text =
            format!("version https://git-lfs.github.com/spec/v1\noid sha256:{OID}\nsize 12345\n");
        assert_eq!(
            LfsPointer::parse(text.as_bytes()),
            Some(LfsPointer {
                oid: OID.into(),
                size: 12345,
                extensions: false
            })
        );
        let ext = format!(
            "version https://git-lfs.github.com/spec/v1\next-0-foo sha256:{OID}\noid sha256:{OID}\nsize 1\n"
        );
        assert!(LfsPointer::parse(ext.as_bytes()).is_some_and(|p| p.extensions));
    }

    #[test]
    fn rejects_non_pointers() {
        assert_eq!(LfsPointer::parse(b"hello\n"), None);
        let no_size = format!("version https://git-lfs.github.com/spec/v1\noid sha256:{OID}\n");
        assert_eq!(LfsPointer::parse(no_size.as_bytes()), None);
        let short_oid = "version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 1\n";
        assert_eq!(LfsPointer::parse(short_oid.as_bytes()), None);
        let upper = format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 1\n",
            OID.to_uppercase()
        );
        assert_eq!(LfsPointer::parse(upper.as_bytes()), None);
        let mut big =
            format!("version https://git-lfs.github.com/spec/v1\noid sha256:{OID}\nsize 1\n");
        let _ = writeln!(big, "x-padding {}", "y".repeat(MAX_POINTER_SIZE));
        assert_eq!(LfsPointer::parse(big.as_bytes()), None);
    }

    #[test]
    fn git_lfs_is_never_run_through_a_repositorys_alias() {
        assert_eq!(
            lfs_args(&["pull"]),
            ["-c", "alias.lfs=", "lfs", "pull"],
            "an empty alias.lfs leaves the program as the only answer"
        );
    }

    #[test]
    fn patterns_are_read_from_the_listing() {
        let listing = "Listing tracked patterns\n    *.psd (.gitattributes)\n    my dir/*.bin [lockable] (sub/.gitattributes)\nListing excluded patterns\n    *.keep (.gitattributes)\n";
        assert_eq!(
            parse_patterns(listing),
            [
                LfsPattern {
                    pattern: "*.psd".into(),
                    source: ".gitattributes".into(),
                    lockable: false
                },
                LfsPattern {
                    pattern: "my dir/*.bin".into(),
                    source: "sub/.gitattributes".into(),
                    lockable: true
                },
            ]
        );
        assert_eq!(parse_patterns("Listing tracked patterns\n"), Vec::new());
    }

    #[test]
    fn locks_are_read_from_the_listing() {
        let listing = "a.bin        \talice\tID:1\nsub dir/b.bin\tme   \tID:22\nnoise\n";
        assert_eq!(
            parse_locks(listing),
            [
                LfsLock {
                    id: "1".into(),
                    path: "a.bin".into(),
                    owner: "alice".into()
                },
                LfsLock {
                    id: "22".into(),
                    path: "sub dir/b.bin".into(),
                    owner: "me".into()
                },
            ]
        );
        assert_eq!(parse_locks(""), Vec::new());
    }

    #[test]
    fn a_pattern_must_be_one_line_and_not_an_option() {
        for ok in ["*.psd", "a dir/f.bin", "dir/**"] {
            assert!(check_pattern(ok).is_ok(), "{ok}");
        }
        for bad in ["", " ", "-x", "--filename", "a\nb", "a\0b"] {
            assert!(check_pattern(bad).is_err(), "{bad:?}");
        }
    }
}
