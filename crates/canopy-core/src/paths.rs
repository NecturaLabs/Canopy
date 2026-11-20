//! Safe access to files inside a working tree.
//!
//! Cloned repositories are untrusted: a path stored in Git can be `../x`, and a committed symbolic
//! link can point anywhere (`link -> /`, `.gitignore -> ~/.bashrc`). Every plain file read or
//! write `canopy-core` makes inside a working tree goes through [`safe_join`], which accepts only
//! a relative path that stays inside the tree and crosses no symbolic link.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

/// How many temporary names are tried before giving up.
const MAX_TEMP_ATTEMPTS: u32 = 100;

fn unsafe_path(path: &str, reason: &'static str) -> Error {
    Error::UnsafePath {
        path: path.to_owned(),
        reason,
    }
}

/// Whether a path component names the `.git` folder: compared case-insensitively, and on
/// Windows also as the file system resolves it (trailing dots and spaces dropped, `GIT~1`).
fn is_git_dir(name: &std::ffi::OsStr) -> bool {
    if name.eq_ignore_ascii_case(".git") {
        return true;
    }
    if cfg!(windows) {
        let name = name.to_string_lossy().to_ascii_lowercase();
        return name.trim_end_matches(['.', ' ']) == ".git" || name.starts_with("git~");
    }
    false
}

/// Checks that `path` is a plain relative path below the working tree root.
///
/// Not empty, not the root itself (`.`), no NUL byte, not absolute, no `..` and no `.git`
/// component. Looks at the text only, not the file system.
pub fn check_relative(path: &str) -> Result<&Path> {
    let p = Path::new(path);
    if path.is_empty() {
        return Err(unsafe_path(path, "is empty"));
    }
    if path.contains('\0') {
        return Err(unsafe_path(path, "contains a NUL byte"));
    }
    for c in p.components() {
        match c {
            Component::Normal(name) if is_git_dir(name) => {
                return Err(unsafe_path(path, "is inside the .git folder"));
            }
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(unsafe_path(path, "leaves the working tree (contains ..)"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_path(path, "is not relative to the working tree"));
            }
        }
    }
    if !p.components().any(|c| matches!(c, Component::Normal(_))) {
        return Err(unsafe_path(path, "is the working tree itself"));
    }
    Ok(p)
}

/// Whether `meta` describes a symbolic link, or on Windows any reparse point (junctions and
/// mount points are not symbolic links to [`std::fs::FileType::is_symlink`] but redirect the
/// same way).
fn is_link(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink() || is_reparse_point(meta)
}

/// Whether `meta` carries the Windows reparse-point attribute (junctions, mount points).
#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Reparse points exist only on Windows.
#[cfg(not(windows))]
const fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Joins `path` (as Git stores it, relative to the working tree) onto `root`, safely.
///
/// `path` must pass [`check_relative`], and no existing component below `root` may be a symbolic
/// link (or, on Windows, a junction).
///
/// Components that do not exist yet are allowed, so the result can name a file about to be created.
/// `root` itself may lie under symbolic links.
///
/// The check and the caller's later access are not atomic: a link created in between is not
/// caught.
pub fn safe_join(root: &Path, path: &str) -> Result<PathBuf> {
    let rel = check_relative(path)?;
    let mut full = root.to_path_buf();
    let mut exists = true;
    for c in rel.components() {
        let Component::Normal(name) = c else {
            continue;
        };
        full.push(name);
        if !exists {
            continue;
        }
        match std::fs::symlink_metadata(&full) {
            Ok(meta) if is_link(&meta) => {
                return Err(unsafe_path(path, "goes through a symbolic link"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => exists = false,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(full)
}

/// Fails with [`Error::UnsafePath`] when `file` exists and is a symbolic link (or, on Windows, a
/// reparse point). For files outside the working tree, such as the repository's exclude file or
/// the global excludes file, where [`safe_join`] does not apply.
pub(crate) fn refuse_link(file: &Path) -> Result<()> {
    match std::fs::symlink_metadata(file) {
        Ok(meta) if is_link(&meta) => Err(unsafe_path(
            &file.to_string_lossy(),
            "is a symbolic link; edit it by hand",
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// A path from raw bytes as git or libgit2 report it, without assuming UTF-8 on Unix (other
/// systems store paths as Unicode, which git reports as UTF-8).
pub fn os_path(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Writes `content` to `dest` through a temporary file in the same directory, keeping the
/// permissions (and so the executable bit) of the file it replaces.
pub(crate) fn write_atomic(dest: &Path, content: &[u8]) -> Result<()> {
    let dir = dest
        .parent()
        .ok_or_else(|| Error::invalid(format!("{} has no parent folder.", dest.display())))?;
    std::fs::create_dir_all(dir)?;
    let permissions = std::fs::metadata(dest).ok().map(|m| m.permissions());
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut attempt = 0u32;
    let (tmp, mut file) = loop {
        let tmp: PathBuf = dir.join(crate::leftovers::staging_file_name(
            &name,
            std::process::id(),
            attempt,
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => break (tmp, f),
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < MAX_TEMP_ATTEMPTS =>
            {
                attempt += 1;
            }
            Err(e) => return Err(e.into()),
        }
    };
    let result = (|| -> Result<()> {
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        if let Some(p) = permissions {
            std::fs::set_permissions(&tmp, p)?;
        }
        std::fs::rename(&tmp, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use crate::test_support::isolate_git;

    use super::*;

    #[test]
    fn rejects_paths_that_leave_the_tree() {
        isolate_git!(rejects_paths_that_leave_the_tree);
        let root = Path::new("/repo");
        for bad in [
            "",
            "../x",
            "a/../../x",
            "a/..",
            ".git/config",
            "A/.GIT/hooks/x",
            "a\0b",
            ".",
            "./",
            "./.",
        ] {
            assert!(
                matches!(safe_join(root, bad), Err(Error::UnsafePath { .. })),
                "{bad:?} must be refused"
            );
        }
        #[cfg(unix)]
        safe_join(root, "/etc/passwd").unwrap_err();
        #[cfg(windows)]
        for bad in [
            "C:\\x",
            "\\\\server\\share\\x",
            "C:x",
            "\\x",
            ".git./config",
            ".git /hooks",
            "GIT~1/config",
        ] {
            assert!(safe_join(root, bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn accepts_plain_and_missing_paths() {
        isolate_git!(accepts_plain_and_missing_paths);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        assert_eq!(
            safe_join(dir.path(), "a/./b.txt").unwrap(),
            dir.path().join("a").join("b.txt")
        );
        assert_eq!(
            safe_join(dir.path(), "new/deeper/file").unwrap(),
            dir.path().join("new").join("deeper").join("file")
        );
        assert_eq!(
            safe_join(dir.path(), ".gitignore").unwrap(),
            dir.path().join(".gitignore")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_any_symlink_component() {
        isolate_git!(rejects_any_symlink_component);
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", dir.path().join("file-link")).unwrap();
        for bad in ["out", "out/x", "out/new/y", "file-link"] {
            assert!(
                matches!(safe_join(dir.path(), bad), Err(Error::UnsafePath { .. })),
                "{bad:?} must be refused"
            );
        }
        assert!(refuse_link(&dir.path().join("file-link")).is_err());
        refuse_link(&dir.path().join("missing")).unwrap();
    }

    #[test]
    fn write_atomic_replaces_content() {
        isolate_git!(write_atomic_replaces_content);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sub").join("f.txt");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        let leftovers = std::fs::read_dir(dir.path().join("sub")).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary files left behind");
    }

    #[test]
    fn a_failed_write_removes_its_staging_file() {
        isolate_git!(a_failed_write_removes_its_staging_file);
        let dir = tempfile::tempdir().unwrap();
        // The final rename cannot replace a folder, after the staging file was fully written.
        let in_the_way = dir.path().join("f.txt");
        std::fs::create_dir(&in_the_way).unwrap();
        write_atomic(&in_the_way, b"content").unwrap_err();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["f.txt"], "no staging file is left");
    }

    #[test]
    fn a_staging_file_is_named_so_a_later_run_can_find_its_owner() {
        isolate_git!(a_staging_file_is_named_so_a_later_run_can_find_its_owner);
        let dir = tempfile::tempdir().unwrap();
        let tmp_name = crate::leftovers::staging_file_name("f.txt", std::process::id(), 0);
        let f = dir.path().join("f.txt");
        // Holding the first name makes `write_atomic` take the next attempt.
        std::fs::write(dir.path().join(&tmp_name), "other").unwrap();
        write_atomic(&f, b"x").unwrap();
        assert!(
            dir.path().join(&tmp_name).is_file(),
            "the other file is untouched"
        );
        assert_eq!(
            crate::leftovers::staging_file_owner(&tmp_name),
            Some(std::process::id())
        );
    }
}
