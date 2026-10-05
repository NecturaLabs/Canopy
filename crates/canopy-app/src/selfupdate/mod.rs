//! Updating Canopy from inside the app.
//!
//! Where the one-line installer put Canopy (Linux: a folder with a `.canopy-install` receipt whose
//! `root=` holds the running executable), the update is done here, the way the installer does it:
//! download the build's archive over HTTPS from Canopy's own release, check its SHA-256 against the
//! manifest, unpack it into `<root>/.staging.*`, swap the entries of the install folder in two
//! phases (the old ones aside into `.previous.*`, the new ones in; a failure puts the old ones
//! back), bring the receipt, `VERSION`, `uninstall.sh` and the registered desktop entry and icon
//! copies up to date (the uninstaller needs those identical), and offer a restart. The folder's
//! `.lock` keeps the installer and the updater from running together.
//!
//! Any other installation (a source build, macOS, Windows) runs the one-line installer pinned to
//! the new build instead, which installs Canopy's own folder next to it.
//!
//! Nothing here touches settings, tokens, logs or caches, and no file is written outside the
//! install folder except the desktop entry and icon copies the receipt already lists.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use canopy_hosting::updates::{self, UpdateManifest};

/// Largest archive downloaded.
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
/// Largest total of the files unpacked from it.
const MAX_UNPACKED: u64 = 768 * 1024 * 1024;
/// Most entries an archive may hold.
const MAX_ENTRIES: usize = 4096;
/// Largest installer script or checksum list.
const MAX_SCRIPT: u64 = 1024 * 1024;

const APP_ID: &str = "io.necturalabs.Canopy";
const RECEIPT: &str = ".canopy-install";

/// What the installer puts in a Linux install folder: the only entries an update replaces. Anything
/// else in the folder is not Canopy's and stays where it is.
const ENTRIES: [&str; 7] = [
    "bin",
    "share",
    "legal",
    "LICENSE",
    "README.md",
    "VERSION",
    "uninstall.sh",
];

/// Where an update stands, for the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Downloading { done: u64, total: Option<u64> },
    Verifying,
    Unpacking,
    Installing,
    Finishing,
}

impl Phase {
    /// What the dialog says is going on.
    pub fn text(&self) -> String {
        match self {
            Self::Downloading { .. } => "Downloading".to_owned(),
            Self::Verifying => "Checking the download".to_owned(),
            Self::Unpacking => "Unpacking".to_owned(),
            Self::Installing => "Installing".to_owned(),
            Self::Finishing => "Finishing".to_owned(),
        }
    }
}

/// A finished update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The new executable to restart into, when there is one.
    pub exe: Option<PathBuf>,
    /// Things that did not stop the update but the user should know.
    pub notes: Vec<String>,
}

/// How the running Canopy was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// By the one-line installer, in `root`: updated in place.
    Managed { root: PathBuf },
    /// Any other way: updated by running the one-line installer.
    Other,
}

/// How this copy of Canopy was installed, found from the running executable.
pub fn detect() -> Install {
    match std::env::current_exe() {
        Ok(exe) => detect_from(&exe),
        Err(_) => Install::Other,
    }
}

/// The installation `exe` belongs to: a Linux install folder (`<root>/bin/canopy`) whose
/// receipt says it is `root`.
fn detect_from(exe: &Path) -> Install {
    if !cfg!(target_os = "linux") {
        return Install::Other;
    }
    let Ok(exe) = fs::canonicalize(exe) else {
        return Install::Other;
    };
    let Some(root) = exe
        .parent()
        .filter(|bin| bin.file_name().is_some_and(|n| n == "bin"))
        .and_then(Path::parent)
    else {
        return Install::Other;
    };
    let receipt = root.join(RECEIPT);
    let Ok(text) = fs::read_to_string(&receipt) else {
        return Install::Other;
    };
    let recorded = text.lines().find_map(|l| l.strip_prefix("root="));
    match recorded.map(fs::canonicalize) {
        Some(Ok(r)) if r == root => Install::Managed {
            root: root.to_path_buf(),
        },
        _ => Install::Other,
    }
}

// ---------------------------------------------------------------------------------------------
// The release to install

/// What to download for a build, checked against the rules the installers apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub build: u64,
    pub archive_name: String,
    pub archive_url: String,
    pub sha256: String,
    /// The folder of the release's files (`…/canopy-build-<n>`).
    pub release_url: String,
}

/// The target triple and archive extension of this platform's builds.
fn target_of(platform_key: &str) -> Option<(&'static str, &'static str)> {
    Some(match platform_key {
        "linux-x86_64" => ("x86_64-unknown-linux-gnu", "tar.gz"),
        "linux-aarch64" => ("aarch64-unknown-linux-gnu", "tar.gz"),
        "darwin-aarch64" => ("aarch64-apple-darwin", "tar.gz"),
        "darwin-x86_64" => ("x86_64-apple-darwin", "tar.gz"),
        "windows-x86_64" => ("x86_64-pc-windows-msvc", "zip"),
        _ => return None,
    })
}

impl Plan {
    /// The download for this platform from `manifest`. It must be Canopy's own archive from
    /// Canopy's own release under `downloads` (`updates::RELEASE_DOWNLOADS`), with a 64-digit
    /// SHA-256: anything else in a manifest is refused.
    pub fn from_manifest(manifest: &UpdateManifest, downloads: &str) -> Result<Self, String> {
        let build = manifest
            .build()
            .ok_or_else(|| "the update manifest names no build number".to_owned())?;
        let key = updates::platform_key();
        let artifact = manifest
            .platforms
            .get(&key)
            .ok_or_else(|| format!("build {build} has no download for this system ({key})"))?;
        let (target, ext) =
            target_of(&key).ok_or_else(|| format!("no known download format for {key}"))?;
        let archive_name = format!("canopy-build-{build}-{target}.{ext}");
        let release_url = format!("{}/canopy-build-{build}", downloads.trim_end_matches('/'));
        let archive_url = format!("{release_url}/{archive_name}");
        if artifact.url != archive_url {
            return Err(format!(
                "the manifest points outside Canopy's releases: {}",
                canopy_core::cli::redact_url(&artifact.url)
            ));
        }
        let sha256 = artifact.sha256.clone().unwrap_or_default();
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(format!(
                "the manifest has no valid sha256 for {archive_name}"
            ));
        }
        Ok(Self {
            build,
            archive_name,
            archive_url,
            sha256,
            release_url,
        })
    }

    /// The top folder inside the archive.
    fn top(&self) -> String {
        self.archive_name
            .strip_suffix(".tar.gz")
            .or_else(|| self.archive_name.strip_suffix(".zip"))
            .unwrap_or(&self.archive_name)
            .to_owned()
    }
}

// ---------------------------------------------------------------------------------------------
// Unpacking

fn io_err(what: &str, path: &Path, e: &io::Error) -> String {
    format!("{what} {}: {e}", path.display())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// Unpacks the tar.gz at `archive` into the new folder `dest`. Only regular files and folders under
/// the one top folder `top` are accepted; a link, a special file, an absolute or `..` path, a
/// second top folder or a size or count over the limits ends the unpacking. Files are written
/// here, never by the tar library, and never over another file.
fn unpack(archive: &Path, dest: &Path, top: &str) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| io_err("could not open", archive, &e))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));
    let entries = tar
        .entries()
        .map_err(|e| format!("could not read the archive: {e}"))?;
    let mut count = 0usize;
    let mut total = 0u64;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("could not read the archive: {e}"))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err("the archive holds too many files".to_owned());
        }
        let kind = entry.header().entry_type();
        if matches!(
            kind,
            tar::EntryType::XGlobalHeader | tar::EntryType::XHeader
        ) {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("the archive has a bad file name: {e}"))?
            .into_owned();
        let mut parts = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(name) => parts.push(name.to_owned()),
                Component::CurDir => {}
                _ => {
                    return Err(format!(
                        "the archive has an unsafe path: {}",
                        path.display()
                    ));
                }
            }
        }
        if parts
            .first()
            .is_none_or(|first| first.as_os_str() != std::ffi::OsStr::new(top))
        {
            return Err(format!(
                "the archive holds {} outside {top}/",
                path.display()
            ));
        }
        let target = parts
            .iter()
            .fold(dest.to_path_buf(), |p, part| p.join(part));
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(|e| io_err("could not create", &target, &e))?;
        } else if kind == tar::EntryType::Regular {
            if parts.len() < 2 {
                return Err(format!(
                    "{} is a file where {top}/ should be",
                    path.display()
                ));
            }
            let size = entry.header().size().unwrap_or(u64::MAX);
            total = total.saturating_add(size);
            if total > MAX_UNPACKED {
                return Err("the archive unpacks to more than is allowed".to_owned());
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| io_err("could not create", parent, &e))?;
            }
            let mode = entry.header().mode().unwrap_or(0o644);
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| io_err("could not create", &target, &e))?;
            // Never more than the header announced, so a lying archive cannot fill the disk.
            let copied = io::copy(&mut (&mut entry).take(size), &mut out)
                .map_err(|e| io_err("could not write", &target, &e))?;
            if copied != size {
                return Err(format!(
                    "{} is shorter than the archive says",
                    path.display()
                ));
            }
            out.flush()
                .map_err(|e| io_err("could not write", &target, &e))?;
            drop(out);
            set_mode(&target, if mode & 0o111 != 0 { 0o755 } else { 0o644 })
                .map_err(|e| io_err("could not set the mode of", &target, &e))?;
        } else {
            return Err(format!(
                "the archive holds a link or special file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Staging the new install folder

fn copy_file(from: &Path, to: &Path, mode: u32) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err("could not create", parent, &e))?;
    }
    fs::copy(from, to).map_err(|e| io_err("could not copy to", to, &e))?;
    set_mode(to, mode).map_err(|e| io_err("could not set the mode of", to, &e))
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| io_err("could not create", to, &e))?;
    for entry in fs::read_dir(from).map_err(|e| io_err("could not read", from, &e))? {
        let entry = entry.map_err(|e| io_err("could not read", from, &e))?;
        let kind = entry
            .file_type()
            .map_err(|e| io_err("could not read", &entry.path(), &e))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_file(&entry.path(), &target, 0o644)?;
        }
    }
    Ok(())
}

/// The desktop entry for the install folder `root`: `Exec=` of the `[Desktop Entry]` group points
/// at the installed binary, and the group carries the `X-Canopy-Install-Dir` marker the uninstaller
/// looks for. Other groups (desktop actions) keep their own `Exec=` lines. `None` when the file
/// has no `[Desktop Entry]` group.
fn desktop_entry(source: &str, root: &Path) -> Option<String> {
    let exec = format!("Exec=\"{}/bin/canopy\" %F", root.display());
    let marker = format!("X-Canopy-Install-Dir={}", root.display());
    let key = |line: &str, name: &str| {
        line.strip_prefix(name)
            .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with('='))
    };
    let mut out = Vec::new();
    let (mut main, mut done) = (false, false);
    for line in source.lines() {
        if line.trim_start_matches([' ', '\t']).starts_with('[') {
            if main && !done {
                out.push(marker.clone());
                done = true;
            }
            main = line == "[Desktop Entry]";
        }
        if main && key(line, "Exec") {
            out.push(exec.clone());
        } else if main && key(line, "X-Canopy-Install-Dir") {
        } else {
            out.push(line.to_owned());
        }
    }
    if main && !done {
        out.push(marker);
        done = true;
    }
    done.then(|| {
        let mut text = out.join("\n");
        text.push('\n');
        text
    })
}

/// Builds the new content of install folder `root` in `new` from the unpacked archive `src`.
/// `uninstaller` is the script to install as `uninstall.sh`.
fn stage(
    src: &Path,
    new: &Path,
    root: &Path,
    build: u64,
    uninstaller: &Path,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    if !src.join("canopy").is_file() {
        return Err("the archive has no canopy binary".to_owned());
    }
    for dir in [
        "bin",
        "share/applications",
        "share/metainfo",
        "share/icons/hicolor/256x256/apps",
    ] {
        fs::create_dir_all(new.join(dir))
            .map_err(|e| io_err("could not create", &new.join(dir), &e))?;
    }
    copy_file(&src.join("canopy"), &new.join("bin/canopy"), 0o755)?;
    for name in ["README.md", "LICENSE"] {
        if src.join(name).is_file() {
            copy_file(&src.join(name), &new.join(name), 0o644)?;
        }
    }
    if src.join("legal").is_dir() {
        copy_tree(&src.join("legal"), &new.join("legal"))?;
    }
    let desktop = src.join(format!("{APP_ID}.desktop"));
    if desktop.is_file() {
        let text =
            fs::read_to_string(&desktop).map_err(|e| io_err("could not read", &desktop, &e))?;
        match desktop_entry(&text, root) {
            Some(entry) => {
                let to = new.join(format!("share/applications/{APP_ID}.desktop"));
                fs::write(&to, entry).map_err(|e| io_err("could not write", &to, &e))?;
                set_mode(&to, 0o644).map_err(|e| io_err("could not set the mode of", &to, &e))?;
            }
            None => notes.push(
                "the desktop entry in the download has no [Desktop Entry] group, so it was not installed"
                    .to_owned(),
            ),
        }
    }
    for (name, dir) in [
        (format!("{APP_ID}.metainfo.xml"), "share/metainfo"),
        (format!("{APP_ID}.png"), "share/icons/hicolor/256x256/apps"),
    ] {
        if src.join(&name).is_file() {
            copy_file(&src.join(&name), &new.join(dir).join(&name), 0o644)?;
        }
    }
    let version = new.join("VERSION");
    fs::write(&version, format!("{build}\n"))
        .map_err(|e| io_err("could not write", &version, &e))?;
    set_mode(&version, 0o644).map_err(|e| io_err("could not set the mode of", &version, &e))?;
    copy_file(uninstaller, &new.join("uninstall.sh"), 0o755)
}

// ---------------------------------------------------------------------------------------------
// The two-phase swap

/// The entries of `dir` a swap moves: everything not hidden, like the installer's `"$ROOT"/*`.
/// The receipt, the lock and the staging and previous folders are hidden and stay where they are.
fn swappable(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with('.') {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

fn remove_any(path: &Path) -> io::Result<()> {
    if path.symlink_metadata()?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Moves the staged content `new` into `root`, replacing what is there, in two phases: the old
/// entries named in `owned` go aside into `previous` (a new folder inside `root`; other entries
/// stay in `root`), then the staged ones move in.
/// When a move fails, the phase reached is undone: staged entries already moved in are removed
/// and the old ones put back, never over an existing name. `mv` moves one entry (a rename).
fn swap_in(
    root: &Path,
    new: &Path,
    previous: &Path,
    owned: &[&str],
    mv: &mut dyn FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    let staged = swappable(new).map_err(|e| io_err("could not read", new, &e))?;
    fs::create_dir(previous).map_err(|e| io_err("could not create", previous, &e))?;
    let old: Vec<_> = swappable(root)
        .map_err(|e| io_err("could not read", root, &e))?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| owned.iter().any(|o| std::ffi::OsStr::new(o) == name))
        })
        .collect();
    // Phase one: the old entries aside.
    let mut moved_aside = Vec::new();
    for entry in &old {
        let Some(name) = entry.file_name() else {
            continue;
        };
        if let Err(e) = mv(entry, &previous.join(name)) {
            return Err(rollback(
                root,
                previous,
                &[],
                &moved_aside,
                &format!("could not move {} aside: {e}", entry.display()),
            ));
        }
        moved_aside.push(name.to_owned());
    }
    // Phase two: the new entries in.
    let mut moved_in = Vec::new();
    for entry in &staged {
        let Some(name) = entry.file_name() else {
            continue;
        };
        if let Err(e) = mv(entry, &root.join(name)) {
            return Err(rollback(
                root,
                previous,
                &moved_in,
                &moved_aside,
                &format!("could not move {} into place: {e}", entry.display()),
            ));
        }
        moved_in.push(name.to_owned());
    }
    Ok(())
}

/// Undoes a failed swap: removes the staged entries that were moved in, then puts every old entry
/// back. Returns the message to show, naming where the old files are if something stayed behind.
fn rollback(
    root: &Path,
    previous: &Path,
    moved_in: &[std::ffi::OsString],
    moved_aside: &[std::ffi::OsString],
    why: &str,
) -> String {
    let mut ok = true;
    for name in moved_in {
        let path = root.join(name);
        if exists(&path) && remove_any(&path).is_err() {
            ok = false;
        }
    }
    for name in moved_aside {
        let from = previous.join(name);
        let to = root.join(name);
        if exists(&to) || fs::rename(&from, &to).is_err() {
            ok = false;
        }
    }
    if ok && fs::remove_dir(previous).is_ok() {
        format!("{why}; the previous version was kept")
    } else {
        format!(
            "{why}, and not all of the previous version could be put back: its remaining files are in {}; move them back into {}, or run the installer again",
            previous.display(),
            root.display()
        )
    }
}

// ---------------------------------------------------------------------------------------------
// The receipt, and what it registered outside the install folder

/// What an install's receipt lists besides its folder.
#[derive(Debug, Default, PartialEq, Eq)]
struct Receipt {
    links: Vec<PathBuf>,
    desktops: Vec<PathBuf>,
    /// (registered copy, original inside the install folder)
    icons: Vec<(PathBuf, PathBuf)>,
}

fn parse_receipt(text: &str) -> Receipt {
    let mut r = Receipt::default();
    for line in text.lines() {
        if let Some(f) = line.strip_prefix("link=") {
            r.links.push(PathBuf::from(f));
        } else if let Some(f) = line.strip_prefix("desktop=") {
            r.desktops.push(PathBuf::from(f));
        } else if let Some(pair) = line.strip_prefix("icon=")
            && let Some((dst, src)) = pair.split_once('|')
        {
            r.icons.push((PathBuf::from(dst), PathBuf::from(src)));
        }
    }
    r
}

/// Writes `from` to `to` through a temporary file and a rename, so nothing is written through a
/// symlink or other file already at `to`.
fn put_file(from: &Path, to: &Path) -> io::Result<()> {
    let mut tmp = to.as_os_str().to_owned();
    tmp.push(format!(".canopy-new.{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let _ = fs::remove_file(&tmp);
    let result = fs::copy(from, &tmp).and_then(|_| fs::rename(&tmp, to));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// Brings what the old receipt registered outside `root` in line with the new files, by the rules
/// the installer applies, and writes the new receipt. Problems are returned as notes: the new
/// version is installed whatever happens here.
fn integrate(root: &Path, build: u64, old: &Receipt, notes: &mut Vec<String>) {
    let mut lines = vec![
        format!(
            "# Written by the Canopy installer: what it created outside {}.",
            root.display()
        ),
        format!("root={}", root.display()),
        format!("version={build}"),
    ];
    let desktop_src = root.join(format!("share/applications/{APP_ID}.desktop"));
    let marker = format!("X-Canopy-Install-Dir={}", root.display());
    for entry in &old.desktops {
        if !desktop_src.is_file() {
            continue;
        }
        let ours = !is_symlink(entry)
            && (!exists(entry)
                || fs::read_to_string(entry).is_ok_and(|t| t.lines().any(|l| l == marker)));
        if !ours {
            notes.push(format!(
                "{} is not Canopy's desktop entry any more; left it alone",
                entry.display()
            ));
        } else if put_file(&desktop_src, entry).is_ok() {
            lines.push(format!("desktop={}", entry.display()));
        } else {
            notes.push(format!("could not write {}", entry.display()));
        }
    }
    for (dst, src) in &old.icons {
        if !src.starts_with(root) {
            continue;
        }
        if !src.is_file() {
            // Earlier builds also registered a scalable SVG; this one ships none. One that cannot
            // be removed stays in the receipt, to be tried again.
            if dst.ends_with(format!("hicolor/scalable/apps/{APP_ID}.svg"))
                && dst.symlink_metadata().is_ok_and(|m| m.is_file())
                && fs::remove_file(dst).is_err()
            {
                notes.push(format!("could not remove {}", dst.display()));
                lines.push(format!("icon={}|{}", dst.display(), src.display()));
            }
            continue;
        }
        if is_symlink(dst) {
            notes.push(format!("{} is a symlink now; left it alone", dst.display()));
        } else if dst
            .parent()
            .is_some_and(|p| fs::create_dir_all(p).is_ok() && put_file(src, dst).is_ok())
        {
            lines.push(format!("icon={}|{}", dst.display(), src.display()));
        } else {
            notes.push(format!("could not write {}", dst.display()));
        }
    }
    for link in &old.links {
        if fs::read_link(link).is_ok_and(|t| t == root.join("bin/canopy")) {
            lines.push(format!("link={}", link.display()));
        }
    }
    lines.push(String::new());
    let tmp = root.join(".canopy-install.new");
    let receipt = root.join(RECEIPT);
    if let Err(e) = fs::write(&tmp, lines.join("\n")).and_then(|()| fs::rename(&tmp, &receipt)) {
        let _ = fs::remove_file(&tmp);
        notes.push(format!("could not update {}: {e}", receipt.display()));
    }
    refresh_desktop_caches(old);
}

/// Refreshes the desktop and icon caches that already exist, the way the installer does; the
/// entry declares no MIME types, and running either tool where there is no cache would create one.
#[expect(
    clippy::disallowed_methods,
    reason = "runs the desktop's cache tools, as the installer does"
)]
fn refresh_desktop_caches(receipt: &Receipt) {
    let run = |program: &str, args: &[&std::ffi::OsStr]| {
        let _ = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    };
    for entry in &receipt.desktops {
        if let Some(apps) = entry.parent()
            && apps.join("mimeinfo.cache").is_file()
        {
            run(
                "update-desktop-database",
                &["-q".as_ref(), apps.as_os_str()],
            );
        }
    }
    let hicolor = receipt.icons.iter().find_map(|(dst, _)| {
        dst.ancestors()
            .find(|a| a.file_name().is_some_and(|n| n == "hicolor"))
    });
    if let Some(hicolor) = hicolor {
        if hicolor.join("icon-theme.cache").is_file() {
            run(
                "gtk-update-icon-cache",
                &[
                    "-q".as_ref(),
                    "-t".as_ref(),
                    "-f".as_ref(),
                    hicolor.as_os_str(),
                ],
            );
        } else {
            let _ =
                fs::File::open(hicolor).and_then(|f| f.set_modified(std::time::SystemTime::now()));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The update

/// Removes a folder when dropped.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The install folder's lock, taken as the installer takes it: a folder made with `mkdir`.
struct Lock(PathBuf);

impl Lock {
    fn take(root: &Path) -> Result<Self, String> {
        let path = root.join(".lock");
        fs::create_dir(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                format!(
                    "another Canopy install or update is running (remove {} if it is not)",
                    path.display()
                )
            } else {
                io_err("could not create", &path, &e)
            }
        })?;
        Ok(Self(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

/// Clears what an interrupted update left in `root`. A `.previous.*` folder is the old version's
/// files: it is deleted only when the install in `root` is whole, else the update is refused.
fn sweep(root: &Path) -> Result<(), String> {
    let mut staging = Vec::new();
    let mut previous = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| io_err("could not read", root, &e))? {
        let entry = entry.map_err(|e| io_err("could not read", root, &e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".staging.") {
            staging.push(entry.path());
        } else if name.starts_with(".previous.") {
            previous.push(entry.path());
        }
    }
    if !previous.is_empty() && !root.join("bin/canopy").is_file() {
        return Err(format!(
            "{} holds the files of an update that was interrupted and has no Canopy of its own: run the installer again to repair it",
            root.display()
        ));
    }
    for path in staging.into_iter().chain(previous) {
        remove_any(&path).map_err(|e| io_err("could not remove", &path, &e))?;
    }
    Ok(())
}

fn unique_name(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{prefix}{}-{nanos}", std::process::id())
}

/// The installer script of `plan`'s release, when it can be trusted: listed in the release's
/// SHA256SUMS with the digest it has, headed `# canopy-installer` and passing `sh -n`.
#[expect(
    clippy::disallowed_methods,
    reason = "checks the installer script with `sh -n`"
)]
fn fetch_uninstaller(plan: &Plan, work: &Path) -> Result<PathBuf, String> {
    let sums = updates::download_bytes(&format!("{}/SHA256SUMS", plan.release_url), MAX_SCRIPT)
        .map_err(|e| format!("could not read the release's checksums: {e}"))?;
    let script = updates::download_bytes(&format!("{}/install.sh", plan.release_url), MAX_SCRIPT)
        .map_err(|e| format!("could not download install.sh: {e}"))?;
    let expected = String::from_utf8_lossy(&sums)
        .lines()
        .filter_map(|l| l.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == "install.sh")
        .map(|(digest, _)| digest.to_owned())
        .ok_or_else(|| "the release's checksums do not list install.sh".to_owned())?;
    if updates::sha256_hex(&script) != expected {
        return Err("install.sh does not match the release's checksums".to_owned());
    }
    if script
        .split(|b| *b == b'\n')
        .nth(1)
        .is_none_or(|l| !l.starts_with(b"# canopy-installer"))
    {
        return Err("install.sh is not Canopy's installer".to_owned());
    }
    let path = work.join("install.sh");
    fs::write(&path, &script).map_err(|e| io_err("could not write", &path, &e))?;
    let syntax = Command::new("sh")
        .arg("-n")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !syntax.is_ok_and(|s| s.success()) {
        return Err("install.sh does not pass a syntax check".to_owned());
    }
    Ok(path)
}

/// Updates the installation in `root` to `plan`'s build, reporting progress. On any failure the
/// installation is as it was.
pub fn install_managed(
    root: &Path,
    plan: &Plan,
    progress: &mut dyn FnMut(Phase),
) -> Result<Installed, String> {
    let root = fs::canonicalize(root).map_err(|e| io_err("could not find", root, &e))?;
    let receipt_path = root.join(RECEIPT);
    let old_receipt = fs::read_to_string(&receipt_path)
        .map_err(|e| io_err("could not read", &receipt_path, &e))?;
    let _lock = Lock::take(&root)?;
    sweep(&root)?;
    let work = root.join(unique_name(".staging."));
    fs::create_dir(&work).map_err(|e| io_err("could not create", &work, &e))?;
    let _work = Cleanup(work.clone());

    let archive = work.join(&plan.archive_name);
    let downloaded = updates::download(
        &plan.archive_url,
        &archive,
        MAX_ARCHIVE,
        &mut |done, total| {
            progress(Phase::Downloading { done, total });
        },
    )
    .map_err(|e| format!("could not download {}: {e}", plan.archive_name))?;
    progress(Phase::Verifying);
    if downloaded.sha256 != plan.sha256 {
        return Err(format!(
            "checksum mismatch for {}: the manifest says {}, the download is {}; nothing was installed",
            plan.archive_name, plan.sha256, downloaded.sha256
        ));
    }
    progress(Phase::Unpacking);
    let unpacked = work.join("x");
    fs::create_dir(&unpacked).map_err(|e| io_err("could not create", &unpacked, &e))?;
    unpack(&archive, &unpacked, &plan.top())?;
    let src = unpacked.join(plan.top());

    let mut notes = Vec::new();
    let uninstaller = match fetch_uninstaller(plan, &work) {
        Ok(path) => path,
        Err(why) => {
            notes.push(format!(
                "uninstall.sh was not refreshed ({why}); the previous one was kept"
            ));
            root.join("uninstall.sh")
        }
    };
    progress(Phase::Installing);
    let new = work.join("new");
    fs::create_dir(&new).map_err(|e| io_err("could not create", &new, &e))?;
    stage(&src, &new, &root, plan.build, &uninstaller, &mut notes)?;
    let previous = root.join(unique_name(".previous."));
    swap_in(&root, &new, &previous, &ENTRIES, &mut |from, to| {
        fs::rename(from, to)
    })?;
    progress(Phase::Finishing);
    if let Err(e) = fs::remove_dir_all(&previous) {
        notes.push(format!("could not remove {}: {e}", previous.display()));
    }
    integrate(&root, plan.build, &parse_receipt(&old_receipt), &mut notes);
    Ok(Installed {
        exe: Some(root.join("bin/canopy")),
        notes,
    })
}

// ---------------------------------------------------------------------------------------------
// Any other installation: the one-line installer

/// Where the installer puts Canopy by default, and so where the update ran from.
fn installer_target() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let over = std::env::var_os("CANOPY_INSTALL_DIR").map(PathBuf::from);
    if cfg!(windows) {
        let root = over.or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(|d| PathBuf::from(d).join("Programs").join("Canopy"))
        })?;
        Some(root.join("canopy.exe"))
    } else if cfg!(target_os = "macos") {
        let app = over.or_else(|| home().map(|h| h.join("Applications/Canopy.app")))?;
        Some(app.join("Contents/MacOS/canopy"))
    } else {
        let root = over.or_else(|| home().map(|h| h.join(".local/opt/canopy")))?;
        Some(root.join("bin/canopy"))
    }
}

/// The last lines of what the installer said, for an error message.
fn tail(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(6)..].join("\n")
}

/// Runs the one-line installer from `feed` pinned to `build`.
#[expect(clippy::disallowed_methods, reason = "runs the release's installer")]
pub fn install_with_installer(
    feed: &str,
    build: u64,
    progress: &mut dyn FnMut(Phase),
) -> Result<Installed, String> {
    let feed = feed.trim_end_matches('/');
    let windows = cfg!(windows);
    let name = if windows { "install.ps1" } else { "install.sh" };
    progress(Phase::Downloading {
        done: 0,
        total: None,
    });
    let script = updates::download_bytes(&format!("{feed}/{name}"), MAX_SCRIPT)
        .map_err(|e| format!("could not download {name}: {e}"))?;
    if script
        .split(|b| *b == b'\n')
        .nth(1)
        .is_none_or(|l| !l.starts_with(b"# canopy-installer"))
    {
        return Err(format!(
            "{name} from the update feed is not Canopy's installer"
        ));
    }
    let dir = std::env::temp_dir().join(unique_name("canopy-update."));
    {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&dir)
            .map_err(|e| io_err("could not create", &dir, &e))?;
    }
    let _dir = Cleanup(dir.clone());
    let path = dir.join(name);
    fs::write(&path, &script).map_err(|e| io_err("could not write", &path, &e))?;
    progress(Phase::Installing);
    let mut command = if windows {
        let mut c = Command::new("powershell.exe");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&path)
        .arg("-Version")
        .arg(build.to_string());
        c
    } else {
        let mut c = Command::new("sh");
        c.arg(&path).arg("--version").arg(build.to_string());
        c
    };
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run the installer: {e}"))?;
    if !output.status.success() {
        let mut said = tail(&output.stderr);
        if said.is_empty() {
            said = tail(&output.stdout);
        }
        return Err(format!("the installer failed: {said}"));
    }
    progress(Phase::Finishing);
    let notes = tail(&output.stderr)
        .lines()
        .filter(|l| l.starts_with("warning:"))
        .map(|l| l.trim_start_matches("warning:").trim().to_owned())
        .collect();
    Ok(Installed {
        exe: installer_target().filter(|p| p.is_file()),
        notes,
    })
}

// ---------------------------------------------------------------------------------------------
// Restart

/// The program to start once the window has closed and settings are saved.
static RESTART: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Asks for `exe` to be started when Canopy exits.
pub fn request_restart(exe: PathBuf) {
    *RESTART
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(exe);
}

/// Starts the program a restart was requested for, with the arguments this run had. Called after
/// the window closed, so the new Canopy reads the settings this one saved.
#[expect(clippy::disallowed_methods, reason = "starts the new Canopy")]
pub fn run_requested_restart() {
    let exe = RESTART
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(exe) = exe {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if let Err(e) = Command::new(&exe).args(args).spawn() {
            tracing::warn!("could not restart into {}: {e}", exe.display());
        }
    }
}

/// Names of `dir`'s entries, for tests and diagnostics.
#[cfg(test)]
fn names(dir: &Path) -> std::collections::BTreeSet<String> {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
