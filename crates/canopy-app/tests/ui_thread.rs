//! AGENTS.md, "Conventions": the UI thread never calls git, libgit2, the hosting API, the network
//! or the filesystem for repository data. Views and the toolkit under `src/views`, `src/ui` and
//! the `nectura-ui` crate render from tab state; anything that reads a repository or the disk goes through a worker job.
//!
//! These tests fail when the production code of those folders (everything before a file's
//! `#[cfg(test)] mod tests`) names a filesystem, process, network, libgit2 or git-CLI API. The
//! scan is textual: it cannot see a call made through a helper in another module, which is why it
//! covers the folders that hold the drawing code, not the whole crate. Comments are skipped.

#![expect(
    clippy::expect_used,
    reason = "helpers outside #[test] functions may panic too: that is a failed test"
)]

use std::path::{Path, PathBuf};

/// What the drawing code may not name.
const FORBIDDEN: &[&str] = &[
    "std::fs",
    "fs::read",
    "fs::write",
    "std::process",
    "std::net",
    "Repo::open",
    "canopy_core::cli",
    "cli::run",
    "git2::",
    "ureq::",
    ".exists()",
    ".try_exists()",
    ".is_dir()",
    ".is_file()",
    "read_dir(",
    "canonicalize(",
];

/// Reviewed exceptions: file (relative to `src`; `nectura-ui/` names the library), a text of the line, and why it is fine.
const ALLOWED: &[(&str, &str, &str)] = &[(
    "nectura-ui/kit/glass/mod.rs",
    "std::fs::read_to_string(\"/proc/self/status\")",
    "the glass soak diagnostic reads its own memory use; it runs only under --glass-soak",
)];

/// The library's `src`, which holds the toolkit.
fn library_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../nectura-ui/src")
}

/// The file an `ALLOWED` or scanned name stands for.
fn locate(file: &str) -> PathBuf {
    file.strip_prefix("nectura-ui/").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file),
        |rest| library_src().join(rest),
    )
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("source folder").flatten() {
        let path = entry.path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The forbidden names in `source`'s production code, as `(line, name)`.
fn violations(file: &str, source: &str) -> Vec<(usize, &'static str)> {
    let source = source.replace("\r\n", "\n");
    let production = source
        .split("\n#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or_default();
    let mut found = Vec::new();
    for (i, line) in production.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for name in FORBIDDEN {
            let excused = ALLOWED
                .iter()
                .any(|(f, text, _)| *f == file && line.contains(text));
            if line.contains(name) && !excused {
                found.push((i + 1, *name));
                break;
            }
        }
    }
    found
}

#[test]
fn views_and_the_toolkit_never_touch_the_disk_the_network_or_git() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for folder in ["views", "ui"] {
        source_files(&src.join(folder), &mut files);
    }
    let library = library_src();
    source_files(&library, &mut files);
    assert!(
        files.len() > 20,
        "the scan found only {} files",
        files.len()
    );
    let mut found = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&src)
            .map_or_else(
                |_| {
                    let inside = path.strip_prefix(&library).expect("inside a source folder");
                    format!("nectura-ui/{}", inside.to_string_lossy())
                },
                |inside| inside.to_string_lossy().into_owned(),
            )
            .replace('\\', "/");
        let text = std::fs::read_to_string(&path).expect("source");
        for (line, name) in violations(&relative, &text) {
            found.push(format!("{relative}:{line}: {name}"));
        }
    }
    assert!(
        found.is_empty(),
        "the UI thread must not do this; add a worker job (AGENTS.md, Conventions):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_scan_finds_calls_in_production_code_only() {
    let injected = "fn draw() {\n    let _ = std::fs::read_to_string(\"x\");\n}\n";
    assert_eq!(violations("views/x.rs", injected), vec![(2, "std::fs")]);
    let in_tests =
        "fn draw() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() { std::fs::read(\"x\"); }\n}\n";
    assert_eq!(violations("views/x.rs", in_tests), []);
    let commented = "// std::fs::read(\"x\") is what a worker does\nfn draw() {}\n";
    assert_eq!(violations("views/x.rs", commented), []);
    let after_a_helper_test_module =
        "fn a() { repo.path().exists(); }\n#[cfg(test)]\nmod tests {}\n";
    assert_eq!(
        violations("views/x.rs", after_a_helper_test_module),
        vec![(1, ".exists()")]
    );
}

#[test]
fn every_exception_is_still_needed() {
    for (file, text, why) in ALLOWED {
        let source = std::fs::read_to_string(locate(file)).expect("excepted file");
        assert!(
            source.contains(text),
            "{file} no longer has `{text}` ({why})"
        );
    }
}
