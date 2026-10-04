use std::io::{Read, Write};

use super::*;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent");
    }
    fs::write(path, text).expect("write");
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// Archives

/// One entry for [`archive`]: a name written byte for byte (so unsafe ones can be made), a kind and
/// the content.
struct Item {
    name: &'static str,
    kind: tar::EntryType,
    body: &'static str,
    mode: u32,
    link: &'static str,
}

fn file(name: &'static str, body: &'static str) -> Item {
    Item {
        name,
        kind: tar::EntryType::Regular,
        body,
        mode: 0o644,
        link: "",
    }
}

fn archive(items: &[Item]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for item in items {
        let mut header = tar::Header::new_gnu();
        let old = header.as_old_mut();
        old.name[..item.name.len()].copy_from_slice(item.name.as_bytes());
        old.linkname[..item.link.len()].copy_from_slice(item.link.as_bytes());
        header.set_entry_type(item.kind);
        header.set_size(item.body.len() as u64);
        header.set_mode(item.mode);
        header.set_cksum();
        builder
            .append(&header, item.body.as_bytes())
            .expect("append");
    }
    let tar = builder.into_inner().expect("tar");
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).expect("gzip");
    gz.finish().expect("gzip")
}

fn unpack_bytes(bytes: &[u8], top: &str) -> (tempfile::TempDir, Result<(), String>) {
    let dir = tempfile::tempdir().expect("dir");
    let archive = dir.path().join("a.tar.gz");
    fs::write(&archive, bytes).expect("archive");
    let dest = dir.path().join("x");
    fs::create_dir(&dest).expect("dest");
    let result = unpack(&archive, &dest, top);
    (dir, result)
}

#[test]
fn unpacks_files_and_folders_under_the_top_folder() {
    let bytes = archive(&[
        Item {
            name: "top/",
            kind: tar::EntryType::Directory,
            body: "",
            mode: 0o755,
            link: "",
        },
        Item {
            mode: 0o755,
            ..file("top/canopy", "#!/bin/sh\n")
        },
        file("top/legal/privacy.md", "p"),
    ]);
    let (dir, result) = unpack_bytes(&bytes, "top");
    result.expect("unpack");
    let x = dir.path().join("x/top");
    assert_eq!(read(&x.join("canopy")), "#!/bin/sh\n");
    assert_eq!(read(&x.join("legal/privacy.md")), "p");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &str| fs::metadata(x.join(p)).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode("canopy"), 0o755);
        assert_eq!(mode("legal/privacy.md"), 0o644);
    }
}

#[test]
fn unsafe_archives_are_refused() {
    let cases: Vec<(&str, Vec<Item>)> = vec![
        ("parent folder", vec![file("top/../escape", "x")]),
        ("absolute path", vec![file("/etc/passwd", "x")]),
        ("outside the top folder", vec![file("other/canopy", "x")]),
        ("a file where the folder belongs", vec![file("top", "x")]),
        (
            "a symlink",
            vec![Item {
                name: "top/link",
                kind: tar::EntryType::Symlink,
                body: "",
                mode: 0o777,
                link: "/etc",
            }],
        ),
        (
            "a hard link",
            vec![Item {
                name: "top/link",
                kind: tar::EntryType::Link,
                body: "",
                mode: 0o644,
                link: "top/canopy",
            }],
        ),
        (
            "a device",
            vec![Item {
                name: "top/dev",
                kind: tar::EntryType::Char,
                body: "",
                mode: 0o644,
                link: "",
            }],
        ),
        ("a file twice", vec![file("top/a", "1"), file("top/a", "2")]),
        (
            "a folder under a file",
            vec![file("top/a", "1"), file("top/a/b", "2")],
        ),
    ];
    for (what, items) in cases {
        let (dir, result) = unpack_bytes(&archive(&items), "top");
        assert!(result.is_err(), "{what} was accepted");
        assert!(!dir.path().join("escape").exists(), "{what} escaped");
    }
    let (_dir, result) = unpack_bytes(b"not an archive", "top");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------------------------
// The release to install

#[test]
fn a_plan_accepts_canopys_own_release_and_nothing_else() {
    let key = updates::platform_key();
    let Some((target, ext)) = target_of(&key) else {
        return;
    };
    let sha = "a".repeat(64);
    let manifest = |url: &str, sha: &str| {
        UpdateManifest::parse(
            format!(
                r#"{{"version":"412","platforms":{{"{key}":{{"url":"{url}","sha256":"{sha}"}}}}}}"#
            )
            .as_bytes(),
        )
        .expect("manifest")
    };
    let base = "https://example.test/dl";
    let good = format!("{base}/canopy-build-412/canopy-build-412-{target}.{ext}");
    let plan = Plan::from_manifest(&manifest(&good, &sha), base).expect("plan");
    assert_eq!(plan.build, 412);
    assert_eq!(plan.archive_url, good);
    assert_eq!(plan.release_url, format!("{base}/canopy-build-412"));
    assert_eq!(plan.top(), format!("canopy-build-412-{target}"));

    for bad in [
        format!("https://evil.test/canopy-build-412/canopy-build-412-{target}.{ext}"),
        format!("{base}/canopy-build-413/canopy-build-412-{target}.{ext}"),
        format!("{base}/canopy-build-412/canopy-build-412-other.{ext}"),
        format!("{good}?x=1"),
        format!("http://example.test/dl/canopy-build-412/canopy-build-412-{target}.{ext}"),
    ] {
        assert!(
            Plan::from_manifest(&manifest(&bad, &sha), base).is_err(),
            "{bad}"
        );
    }
    for bad_sha in ["", "abc", &"A".repeat(64), &"g".repeat(64), &"a".repeat(65)] {
        assert!(
            Plan::from_manifest(&manifest(&good, bad_sha), base).is_err(),
            "{bad_sha:?}"
        );
    }
    let no_platform = UpdateManifest::parse(br#"{"version":"412"}"#).expect("manifest");
    assert!(Plan::from_manifest(&no_platform, base).is_err());
    let semver = UpdateManifest::parse(br#"{"version":"0.2.1-beta.1"}"#).expect("manifest");
    assert!(Plan::from_manifest(&semver, base).is_err());
}

// ---------------------------------------------------------------------------------------------
// Desktop entry

const DESKTOP: &str = "[Desktop Entry]\nName=Canopy\nExec=canopy %F\nIcon=io.necturalabs.Canopy\n\n[Desktop Action new-window]\nName=New Window\nExec=canopy --new-window\n";

#[test]
fn the_desktop_entry_points_at_the_install_and_carries_the_marker() {
    let root = Path::new("/home/u/.local/opt/canopy");
    let entry = desktop_entry(DESKTOP, root).expect("entry");
    assert_eq!(
        entry,
        // As the installer's awk writes it: the marker goes just before the next group's header.
        "[Desktop Entry]\nName=Canopy\nExec=\"/home/u/.local/opt/canopy/bin/canopy\" %F\nIcon=io.necturalabs.Canopy\n\nX-Canopy-Install-Dir=/home/u/.local/opt/canopy\n[Desktop Action new-window]\nName=New Window\nExec=canopy --new-window\n"
    );
    // Doing it again, to an entry that already has a marker, changes nothing.
    assert_eq!(desktop_entry(&entry, root).expect("again"), entry);
    // A marker at the end of the file is added after the last line of the main group.
    let only_main = desktop_entry("[Desktop Entry]\nName=Canopy\n", root).expect("only main");
    assert!(only_main.ends_with("X-Canopy-Install-Dir=/home/u/.local/opt/canopy\n"));
    assert_eq!(desktop_entry("Name=Canopy\n[Other]\nExec=x\n", root), None);
}

// ---------------------------------------------------------------------------------------------
// The swap

/// An install folder with a receipt, the lock-free hidden files and the given entries.
fn folder(root: &Path, entries: &[(&str, &str)]) {
    fs::create_dir_all(root).expect("root");
    for (name, body) in entries {
        write(&root.join(name), body);
    }
}

fn tree(dir: &Path) -> Vec<(String, String)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .expect("read")
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for e in entries {
            let path = e.path();
            let rel = path.strip_prefix(base).expect("rel").display().to_string();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.push((rel, read(&path)));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn a_swap_replaces_every_visible_entry_and_keeps_the_hidden_ones() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    folder(
        &root,
        &[
            ("bin/canopy", "old"),
            ("VERSION", "1\n"),
            ("old-only", "x"),
            (".canopy-install", "receipt"),
        ],
    );
    let new = tmp.path().join("new");
    folder(&new, &[("bin/canopy", "new"), ("VERSION", "2\n")]);
    let previous = root.join(".previous.t");
    swap_in(
        &root,
        &new,
        &previous,
        &["bin", "VERSION", "old-only"],
        &mut |a, b| fs::rename(a, b),
    )
    .expect("swap");
    let now: Vec<_> = tree(&root)
        .into_iter()
        .filter(|(path, _)| !path.starts_with(".previous"))
        .collect();
    assert_eq!(
        now,
        vec![
            (".canopy-install".to_owned(), "receipt".to_owned()),
            ("VERSION".to_owned(), "2\n".to_owned()),
            ("bin/canopy".to_owned(), "new".to_owned()),
        ]
    );
    assert!(
        previous.exists(),
        "the old files wait there until the caller removes them"
    );
    assert_eq!(read(&previous.join("bin/canopy")), "old");
    assert!(!previous.join(".canopy-install").exists());
}

#[test]
fn a_swap_leaves_entries_that_are_not_canopys_alone() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    folder(
        &root,
        &[
            ("bin/canopy", "old"),
            ("my-notes.txt", "mine"),
            ("my-dir/x", "mine"),
        ],
    );
    let new = tmp.path().join("new");
    folder(&new, &[("bin/canopy", "new")]);
    let previous = root.join(".previous.t");
    swap_in(&root, &new, &previous, &ENTRIES, &mut |a, b| {
        fs::rename(a, b)
    })
    .expect("swap");
    assert_eq!(read(&root.join("bin/canopy")), "new");
    assert_eq!(read(&root.join("my-notes.txt")), "mine");
    assert_eq!(read(&root.join("my-dir/x")), "mine");
    assert!(!previous.join("my-notes.txt").exists());
}

#[test]
fn a_swap_that_fails_while_moving_the_old_version_aside_puts_it_back() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    folder(
        &root,
        &[("a", "1"), ("b", "2"), ("c", "3"), (".canopy-install", "r")],
    );
    let before = tree(&root);
    let new = tmp.path().join("new");
    folder(&new, &[("a", "new"), ("z", "new")]);
    let mut moves = 0;
    let err = swap_in(
        &root,
        &new,
        &root.join(".previous.t"),
        &["a", "b", "c"],
        &mut |a, b| {
            moves += 1;
            if moves == 2 {
                return Err(io::Error::other("disk on fire"));
            }
            fs::rename(a, b)
        },
    )
    .expect_err("failed");
    assert!(err.contains("the previous version was kept"), "{err}");
    assert_eq!(tree(&root), before);
    assert!(!root.join(".previous.t").exists());
    assert_eq!(
        tree(&new),
        vec![
            ("a".to_owned(), "new".to_owned()),
            ("z".to_owned(), "new".to_owned())
        ]
    );
}

#[test]
fn a_swap_that_fails_while_moving_the_new_version_in_puts_the_old_back() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    folder(&root, &[("a", "1"), ("b", "2"), (".canopy-install", "r")]);
    let before = tree(&root);
    let new = tmp.path().join("new");
    folder(&new, &[("a", "new"), ("b", "new"), ("c", "new")]);
    let mut moves = 0;
    let err = swap_in(
        &root,
        &new,
        &root.join(".previous.t"),
        &["a", "b", "c"],
        &mut |a, b| {
            moves += 1;
            // Two moves aside, then the first new entry in, then a failure.
            if moves == 4 {
                return Err(io::Error::other("no space"));
            }
            fs::rename(a, b)
        },
    )
    .expect_err("failed");
    assert!(err.contains("the previous version was kept"), "{err}");
    assert_eq!(
        tree(&root),
        before,
        "the staged entry that got in was removed"
    );
}

#[test]
fn a_rollback_that_cannot_finish_says_where_the_old_files_are() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    folder(&root, &[("a", "1"), ("b", "2")]);
    let new = tmp.path().join("new");
    folder(&new, &[("a", "new")]);
    let previous = root.join(".previous.t");
    let mut moves = 0;
    let err = swap_in(&root, &new, &previous, &["a", "b"], &mut |a, b| {
        moves += 1;
        // Both go aside, the new one fails, and then another process creates `b` in the way.
        if moves == 3 {
            write(&root.join("b"), "squatter");
            return Err(io::Error::other("failed"));
        }
        fs::rename(a, b)
    })
    .expect_err("failed");
    assert!(err.contains(&previous.display().to_string()), "{err}");
    assert_eq!(read(&root.join("a")), "1", "what could be put back was");
    assert_eq!(read(&root.join("b")), "squatter", "nothing was overwritten");
    assert_eq!(
        read(&previous.join("b")),
        "2",
        "the rest is where the message says"
    );
}

// ---------------------------------------------------------------------------------------------
// Sweeping, locking, the receipt

#[test]
fn a_sweep_removes_leftovers_of_an_interrupted_update() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    folder(
        &root,
        &[
            ("bin/canopy", "x"),
            (".staging.1/a", "x"),
            (".previous.2/b", "x"),
            (".keep", "k"),
        ],
    );
    sweep(&root).expect("sweep");
    assert_eq!(names(&root), [".keep", "bin"].map(String::from).into());
}

#[test]
fn a_sweep_refuses_to_delete_the_only_copy_of_the_old_version() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    folder(&root, &[(".previous.2/bin/canopy", "the only one")]);
    let err = sweep(&root).expect_err("refused");
    assert!(err.contains("run the installer again"), "{err}");
    assert_eq!(read(&root.join(".previous.2/bin/canopy")), "the only one");
}

#[test]
fn the_lock_is_exclusive_and_released() {
    let tmp = tempfile::tempdir().expect("tmp");
    let lock = Lock::take(tmp.path()).expect("first");
    let err = Lock::take(tmp.path()).map(|_| ()).expect_err("second");
    assert!(
        err.contains("another Canopy install or update is running"),
        "{err}"
    );
    drop(lock);
    assert!(!tmp.path().join(".lock").exists());
    drop(Lock::take(tmp.path()).expect("after release"));
}

#[test]
fn the_receipt_is_read_and_registered_copies_follow_the_new_files() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    let data = tmp.path().join("data");
    let entry = data.join("applications/io.necturalabs.Canopy.desktop");
    let png = data.join("icons/hicolor/256x256/apps/io.necturalabs.Canopy.png");
    let svg = data.join("icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg");
    let link = tmp.path().join("bin/canopy");
    let foreign_entry = data.join("applications/foreign.desktop");
    let marker = format!("X-Canopy-Install-Dir={}", root.display());
    // The new version, already swapped in.
    folder(
        &root,
        &[
            ("bin/canopy", "new"),
            (
                "share/applications/io.necturalabs.Canopy.desktop",
                "new desktop\n",
            ),
            (
                "share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png",
                "new png",
            ),
        ],
    );
    // What the old receipt registered: an entry that is ours, an icon, the scalable icon earlier
    // builds shipped, now a symlink, a link that still points at the install and an entry someone else took over.
    write(&entry, &format!("old desktop\n{marker}\n"));
    write(&png, "old png");
    write(&foreign_entry, "someone else's\n");
    fs::create_dir_all(svg.parent().expect("parent")).expect("dirs");
    write(&tmp.path().join("elsewhere.svg"), "elsewhere");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tmp.path().join("elsewhere.svg"), &svg).expect("symlink");
        fs::create_dir_all(link.parent().expect("parent")).expect("dirs");
        std::os::unix::fs::symlink(root.join("bin/canopy"), &link).expect("link");
    }
    let text = format!(
        "# header\nroot={}\nversion=1\nlink={}\ndesktop={}\ndesktop={}\nicon={}|{}\nicon={}|{}\n",
        root.display(),
        link.display(),
        entry.display(),
        foreign_entry.display(),
        png.display(),
        root.join("share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png")
            .display(),
        svg.display(),
        root.join("share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg")
            .display(),
    );
    let old = parse_receipt(&text);
    assert_eq!(old.desktops.len(), 2);
    assert_eq!(old.icons.len(), 2);
    assert_eq!(old.links, vec![link.clone()]);

    let mut notes = Vec::new();
    integrate(&root, 413, &old, &mut notes);

    assert_eq!(read(&entry), "new desktop\n");
    assert_eq!(read(&png), "new png");
    assert_eq!(
        read(&foreign_entry),
        "someone else's\n",
        "not ours: untouched"
    );
    #[cfg(unix)]
    {
        assert_eq!(
            read(&tmp.path().join("elsewhere.svg")),
            "elsewhere",
            "never written through a symlink"
        );
        assert!(is_symlink(&svg), "a symlink is never removed");
    }
    let receipt = parse_receipt(&read(&root.join(RECEIPT)));
    assert_eq!(receipt.desktops, vec![entry.clone()]);
    assert_eq!(receipt.icons.len(), 1);
    assert_eq!(receipt.icons[0].0, png);
    #[cfg(unix)]
    assert_eq!(receipt.links, vec![link]);
    assert!(read(&root.join(RECEIPT)).contains(&format!("root={}\nversion=413\n", root.display())));
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(!root.join(".canopy-install.new").exists());
}

#[test]
fn the_scalable_icon_earlier_builds_registered_is_removed() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    let data = tmp.path().join("data");
    let png = data.join("icons/hicolor/256x256/apps/io.necturalabs.Canopy.png");
    let svg = data.join("icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg");
    let other = data.join("icons/hicolor/scalable/apps/other.svg");
    folder(
        &root,
        &[(
            "share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png",
            "new png",
        )],
    );
    write(&png, "old png");
    write(&svg, "old svg");
    write(&other, "someone else's");
    let old = parse_receipt(&format!(
        "icon={}|{}\nicon={}|{}\nicon={}|{}\n",
        png.display(),
        root.join("share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png")
            .display(),
        svg.display(),
        root.join("share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg")
            .display(),
        other.display(),
        root.join("share/icons/hicolor/scalable/apps/other.svg")
            .display(),
    ));

    let mut notes = Vec::new();
    integrate(&root, 413, &old, &mut notes);

    assert_eq!(read(&png), "new png");
    assert!(!exists(&svg), "the old scalable icon is gone");
    assert_eq!(read(&other), "someone else's", "only Canopy's own icon");
    let receipt = parse_receipt(&read(&root.join(RECEIPT)));
    assert_eq!(receipt.icons.len(), 1);
    assert!(notes.is_empty(), "{notes:?}");
}

#[cfg(unix)]
#[test]
fn a_scalable_icon_that_cannot_be_removed_stays_registered() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("root");
    let svg = tmp
        .path()
        .join("data/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg");
    folder(&root, &[("bin/canopy", "new")]);
    write(&svg, "old svg");
    let apps = svg.parent().expect("parent").to_owned();
    fs::set_permissions(&apps, fs::Permissions::from_mode(0o555)).expect("chmod");
    let old = parse_receipt(&format!(
        "icon={}|{}\n",
        svg.display(),
        root.join("share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg")
            .display(),
    ));

    let mut notes = Vec::new();
    integrate(&root, 413, &old, &mut notes);
    fs::set_permissions(&apps, fs::Permissions::from_mode(0o755)).expect("chmod back");

    if exists(&svg) {
        let receipt = parse_receipt(&read(&root.join(RECEIPT)));
        assert_eq!(receipt.icons, old.icons, "kept, to be tried again");
        assert_eq!(notes.len(), 1, "{notes:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// The whole update, against a local server

/// Serves each path from `files` on a loopback port for `connections` requests.
fn serve(files: Vec<(String, Vec<u8>)>, connections: usize) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for _ in 0..connections {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (head, body) = match files.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => ("200 OK", body.clone()),
                None => ("404 Not Found", Vec::new()),
            };
            let reply = format!(
                "HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://127.0.0.1:{port}")
}

const TOP: &str = "canopy-build-413-x86_64-unknown-linux-gnu";
const INSTALL_SH: &str = "#!/bin/sh\n# canopy-installer (test)\nexit 0\n";

fn release_archive() -> Vec<u8> {
    archive(&[
        Item {
            mode: 0o755,
            ..file(
                "canopy-build-413-x86_64-unknown-linux-gnu/canopy",
                "#!/bin/sh\necho new\n",
            )
        },
        file(
            "canopy-build-413-x86_64-unknown-linux-gnu/README.md",
            "readme",
        ),
        file(
            "canopy-build-413-x86_64-unknown-linux-gnu/LICENSE",
            "licence",
        ),
        file(
            "canopy-build-413-x86_64-unknown-linux-gnu/legal/privacy-policy.md",
            "privacy 2",
        ),
        file(
            "canopy-build-413-x86_64-unknown-linux-gnu/io.necturalabs.Canopy.desktop",
            DESKTOP,
        ),
        file(
            "canopy-build-413-x86_64-unknown-linux-gnu/io.necturalabs.Canopy.metainfo.xml",
            "<component/>",
        ),
        file(
            "canopy-build-413-x86_64-unknown-linux-gnu/io.necturalabs.Canopy.png",
            "png 2",
        ),
        file(
            "canopy-build-413-x86_64-unknown-linux-gnu/io.necturalabs.Canopy.svg",
            "svg 2",
        ),
    ])
}

/// An installed Canopy (build 412) the way the installer leaves it, with its registered copies.
struct Installed412 {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    entry: PathBuf,
    png: PathBuf,
    settings: PathBuf,
}

fn installed() -> Installed412 {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("opt/canopy");
    let data = tmp.path().join("data");
    let entry = data.join("applications/io.necturalabs.Canopy.desktop");
    let png = data.join("icons/hicolor/256x256/apps/io.necturalabs.Canopy.png");
    let settings = tmp.path().join("config/settings.json");
    let marker = format!("X-Canopy-Install-Dir={}", root.display());
    folder(
        &root,
        &[
            ("bin/canopy", "#!/bin/sh\necho old\n"),
            ("VERSION", "412\n"),
            ("uninstall.sh", "#!/bin/sh\n# canopy-installer (old)\n"),
            ("README.md", "old readme"),
            ("legal/old-only.md", "old"),
            (
                "share/applications/io.necturalabs.Canopy.desktop",
                &format!("old desktop\n{marker}\n"),
            ),
            (
                "share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png",
                "png 1",
            ),
        ],
    );
    write(&entry, &format!("old desktop\n{marker}\n"));
    write(&png, "png 1");
    write(&settings, "{\"keep\":true}");
    write(
        &root.join(RECEIPT),
        &format!(
            "# Written by the Canopy installer.\nroot={}\nversion=412\ndesktop={}\nicon={}|{}\n",
            root.display(),
            entry.display(),
            png.display(),
            root.join("share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png")
                .display()
        ),
    );
    Installed412 {
        _tmp: tmp,
        root,
        entry,
        png,
        settings,
    }
}

fn plan_for(base: &str, archive: &[u8], sha: Option<String>) -> Plan {
    let name = format!("{TOP}.tar.gz");
    Plan {
        build: 413,
        archive_url: format!("{base}/{name}"),
        archive_name: name,
        sha256: sha.unwrap_or_else(|| updates::sha256_hex(archive)),
        release_url: base.to_owned(),
    }
}

fn sums(script: &str) -> Vec<u8> {
    format!(
        "{}  install.sh\n{}  other\n",
        updates::sha256_hex(script.as_bytes()),
        "0".repeat(64)
    )
    .into_bytes()
}

#[test]
fn an_update_replaces_the_install_and_everything_registered_with_it() {
    let old = installed();
    let bytes = release_archive();
    let base = serve(
        vec![
            (format!("/{TOP}.tar.gz"), bytes.clone()),
            ("/SHA256SUMS".to_owned(), sums(INSTALL_SH)),
            ("/install.sh".to_owned(), INSTALL_SH.as_bytes().to_vec()),
        ],
        3,
    );
    let mut phases = Vec::new();
    let done = install_managed(&old.root, &plan_for(&base, &bytes, None), &mut |p| {
        phases.push(p)
    })
    .expect("update");
    let root = fs::canonicalize(&old.root).expect("root");
    assert_eq!(done.exe, Some(root.join("bin/canopy")));
    assert!(done.notes.is_empty(), "{:?}", done.notes);

    assert_eq!(read(&root.join("bin/canopy")), "#!/bin/sh\necho new\n");
    assert_eq!(read(&root.join("VERSION")), "413\n");
    assert_eq!(read(&root.join("uninstall.sh")), INSTALL_SH);
    assert_eq!(read(&root.join("README.md")), "readme");
    assert_eq!(read(&root.join("legal/privacy-policy.md")), "privacy 2");
    assert!(
        !root.join("legal/old-only.md").exists(),
        "every old file is gone"
    );
    let installed_entry = read(&root.join("share/applications/io.necturalabs.Canopy.desktop"));
    assert!(installed_entry.contains(&format!("Exec=\"{}/bin/canopy\" %F", root.display())));
    // The registered copies are the new files, as the uninstaller needs them.
    assert_eq!(read(&old.entry), installed_entry);
    assert_eq!(read(&old.png), "png 2");
    let receipt = read(&root.join(RECEIPT));
    assert!(receipt.contains("version=413"), "{receipt}");
    assert!(
        receipt.contains(&format!("desktop={}", old.entry.display())),
        "{receipt}"
    );
    // Settings are not Canopy's install folder and are never touched; nothing is left behind.
    assert_eq!(read(&old.settings), "{\"keep\":true}");
    assert_eq!(
        names(&root),
        [
            ".canopy-install",
            "LICENSE",
            "README.md",
            "VERSION",
            "bin",
            "legal",
            "share",
            "uninstall.sh"
        ]
        .map(String::from)
        .into()
    );
    assert!(matches!(phases.first(), Some(Phase::Downloading { .. })));
    for expected in [
        Phase::Verifying,
        Phase::Unpacking,
        Phase::Installing,
        Phase::Finishing,
    ] {
        assert!(phases.contains(&expected), "{expected:?} in {phases:?}");
    }
}

#[test]
fn a_checksum_mismatch_leaves_the_install_exactly_as_it_was() {
    let old = installed();
    let before = tree(&old.root);
    let bytes = release_archive();
    let base = serve(vec![(format!("/{TOP}.tar.gz"), bytes.clone())], 1);
    let err = install_managed(
        &old.root,
        &plan_for(&base, &bytes, Some("0".repeat(64))),
        &mut |_| {},
    )
    .expect_err("mismatch");
    assert!(err.contains("checksum mismatch"), "{err}");
    assert!(err.contains("nothing was installed"), "{err}");
    assert_eq!(tree(&old.root), before);
    assert_eq!(read(&old.png), "png 1");
    assert!(!old.root.join(".lock").exists(), "the lock is released");
}

#[test]
fn a_download_that_fails_leaves_the_install_exactly_as_it_was() {
    let old = installed();
    let before = tree(&old.root);
    let base = serve(vec![], 1);
    let err =
        install_managed(&old.root, &plan_for(&base, b"x", None), &mut |_| {}).expect_err("404");
    assert!(err.contains("could not download"), "{err}");
    assert_eq!(tree(&old.root), before);
    assert!(!old.root.join(".lock").exists());
    assert!(
        names(&old.root)
            .iter()
            .all(|n| !n.starts_with(".staging.") && !n.starts_with(".previous."))
    );
}

#[test]
fn an_unsafe_archive_leaves_the_install_exactly_as_it_was() {
    let old = installed();
    let before = tree(&old.root);
    let bytes = archive(&[file(
        "canopy-build-413-x86_64-unknown-linux-gnu/../../evil",
        "x",
    )]);
    let base = serve(vec![(format!("/{TOP}.tar.gz"), bytes.clone())], 1);
    let err = install_managed(&old.root, &plan_for(&base, &bytes, None), &mut |_| {})
        .expect_err("unsafe");
    assert!(err.contains("unsafe path"), "{err}");
    assert_eq!(tree(&old.root), before);
    assert!(!old.root.parent().expect("opt").join("evil").exists());
}

#[test]
fn an_archive_without_a_binary_is_refused_before_anything_changes() {
    let old = installed();
    let before = tree(&old.root);
    let bytes = archive(&[file(
        "canopy-build-413-x86_64-unknown-linux-gnu/README.md",
        "readme",
    )]);
    let base = serve(
        vec![
            (format!("/{TOP}.tar.gz"), bytes.clone()),
            ("/SHA256SUMS".to_owned(), sums(INSTALL_SH)),
            ("/install.sh".to_owned(), INSTALL_SH.as_bytes().to_vec()),
        ],
        3,
    );
    let err = install_managed(&old.root, &plan_for(&base, &bytes, None), &mut |_| {})
        .expect_err("no binary");
    assert!(err.contains("no canopy binary"), "{err}");
    assert_eq!(tree(&old.root), before);
}

#[test]
fn an_installer_script_that_cannot_be_trusted_keeps_the_old_uninstaller() {
    for (what, sums_body, script) in [
        ("no checksums", None, INSTALL_SH.to_owned()),
        (
            "a script that does not match its checksum",
            Some(sums("other")),
            INSTALL_SH.to_owned(),
        ),
        (
            "a script that is not Canopy's",
            Some(sums("#!/bin/sh\nexit 0\n")),
            "#!/bin/sh\nexit 0\n".to_owned(),
        ),
        (
            "a script that does not parse",
            Some(sums("#!/bin/sh\n# canopy-installer\nif then\n")),
            "#!/bin/sh\n# canopy-installer\nif then\n".to_owned(),
        ),
    ] {
        let old = installed();
        let bytes = release_archive();
        let mut files = vec![
            (format!("/{TOP}.tar.gz"), bytes.clone()),
            ("/install.sh".to_owned(), script.into_bytes()),
        ];
        if let Some(body) = sums_body {
            files.push(("/SHA256SUMS".to_owned(), body));
        }
        let base = serve(files, 3);
        let done = install_managed(&old.root, &plan_for(&base, &bytes, None), &mut |_| {})
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(done.notes.len(), 1, "{what}: {:?}", done.notes);
        assert!(
            done.notes[0].contains("uninstall.sh was not refreshed"),
            "{what}: {:?}",
            done.notes
        );
        assert_eq!(
            read(&old.root.join("uninstall.sh")),
            "#!/bin/sh\n# canopy-installer (old)\n",
            "{what}"
        );
        assert_eq!(read(&old.root.join("VERSION")), "413\n", "{what}");
    }
}

#[test]
fn an_update_does_not_run_while_the_installer_holds_the_lock() {
    let old = installed();
    let before = tree(&old.root);
    fs::create_dir(old.root.join(".lock")).expect("lock");
    let err = install_managed(
        &old.root,
        &plan_for("http://127.0.0.1:1", b"x", None),
        &mut |_| {},
    )
    .expect_err("locked");
    assert!(
        err.contains("another Canopy install or update is running"),
        "{err}"
    );
    assert!(
        old.root.join(".lock").exists(),
        "someone else's lock is not ours to remove"
    );
    fs::remove_dir(old.root.join(".lock")).expect("unlock");
    assert_eq!(tree(&old.root), before);
}

// ---------------------------------------------------------------------------------------------
// Which installation this is

#[cfg(target_os = "linux")]
#[test]
fn a_binary_in_an_install_folder_with_its_receipt_is_managed() {
    let old = installed();
    let exe = old.root.join("bin/canopy");
    assert_eq!(
        detect_from(&exe),
        Install::Managed {
            root: fs::canonicalize(&old.root).expect("root")
        }
    );
    // No receipt, a receipt for another folder, a binary outside bin/: all "other".
    let elsewhere = old._tmp.path().join("other/bin/canopy");
    write(&elsewhere, "x");
    assert_eq!(detect_from(&elsewhere), Install::Other);
    write(
        &old._tmp.path().join("other").join(RECEIPT),
        &format!("root={}\n", old.root.display()),
    );
    assert_eq!(
        detect_from(&elsewhere),
        Install::Other,
        "the receipt names another folder"
    );
    let loose = old._tmp.path().join("loose/canopy");
    write(&loose, "x");
    assert_eq!(detect_from(&loose), Install::Other);
    assert_eq!(
        detect_from(Path::new("/nonexistent/bin/canopy")),
        Install::Other
    );
}

// ---------------------------------------------------------------------------------------------
// Any other installation

#[cfg(unix)]
#[test]
fn the_installer_fallback_runs_the_feeds_installer_pinned_to_the_build() {
    let tmp = tempfile::tempdir().expect("tmp");
    let log = tmp.path().join("args");
    let script = format!(
        "#!/bin/sh\n# canopy-installer (test)\nprintf '%s ' \"$@\" > '{}'\necho 'warning: a launcher entry was left alone' >&2\nexit 0\n",
        log.display()
    );
    let base = serve(vec![("/install.sh".to_owned(), script.into_bytes())], 1);
    let mut phases = Vec::new();
    let done = install_with_installer(&base, 413, &mut |p| phases.push(p)).expect("installer");
    assert_eq!(read(&log).trim(), "--version 413");
    assert_eq!(
        done.notes,
        vec!["a launcher entry was left alone".to_owned()]
    );
    assert!(phases.contains(&Phase::Installing));
}

#[cfg(unix)]
#[test]
fn the_installer_fallback_reports_a_failure_and_refuses_a_foreign_script() {
    let failing = "#!/bin/sh\n# canopy-installer (test)\necho 'Canopy build 413 is not published' >&2\nexit 1\n";
    let base = serve(
        vec![("/install.sh".to_owned(), failing.as_bytes().to_vec())],
        1,
    );
    let err = install_with_installer(&base, 413, &mut |_| {}).expect_err("failed");
    assert!(err.contains("the installer failed"), "{err}");
    assert!(err.contains("is not published"), "{err}");

    let foreign = serve(
        vec![("/install.sh".to_owned(), b"#!/bin/sh\nrm -rf x\n".to_vec())],
        1,
    );
    let err = install_with_installer(&foreign, 413, &mut |_| {}).expect_err("foreign");
    assert!(err.contains("is not Canopy's installer"), "{err}");

    let missing = serve(vec![], 1);
    assert!(install_with_installer(&missing, 413, &mut |_| {}).is_err());
}

#[test]
fn a_restart_runs_what_was_asked_after_the_window_closed() {
    // Nothing requested: nothing happens (and nothing panics).
    run_requested_restart();
    let tmp = tempfile::tempdir().expect("tmp");
    let marker = tmp.path().join("started");
    #[cfg(unix)]
    {
        let exe = tmp.path().join("canopy");
        write(&exe, &format!("#!/bin/sh\ntouch '{}'\n", marker.display()));
        set_mode(&exe, 0o755).expect("mode");
        request_restart(exe);
        run_requested_restart();
        for _ in 0..100 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists(), "the program was started");
        run_requested_restart();
    }
}
