//! Git LFS awareness without running git-lfs: which paths the `filter=lfs` attribute covers,
//! pointer files, and whether a pointer's object has been downloaded.

use std::path::{Path, PathBuf};

use git2::{AttrCheckFlags, FileMode, Repository};

use crate::model::WorkingStatus;
use crate::{Repo, Result};

/// Pointer files are smaller than this (git-lfs specification).
const MAX_POINTER_SIZE: usize = 1024;

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
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

/// Whether the `filter` attribute of `path` is `lfs`.
pub(crate) fn is_lfs(repo: &Repository, path: &Path) -> bool {
    matches!(
        repo.get_attr(path, "filter", AttrCheckFlags::FILE_THEN_INDEX),
        Ok(Some("lfs"))
    )
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

impl Repo {
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
        let (a, b) = (oid.get(0..2)?, oid.get(2..4)?);
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
            let stage = (entry.flags >> 12) & 0x3;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

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
}
