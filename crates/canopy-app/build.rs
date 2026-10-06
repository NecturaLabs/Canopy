//! Embeds the build identity: the build number (`git rev-list --count` of the commit on `main`)
//! and the commit's seven-character short hash, plus the release notes of a release build.
//!
//! The release workflow sets `CANOPY_BUILD_NUMBER` and `CANOPY_COMMIT`. Without them the identity
//! is computed from the git checkout the build runs in, and without a checkout (a source archive)
//! the build number is `dev` and the commit is empty. `CANOPY_RELEASE_NOTES_FILE` names a Markdown
//! file whose text becomes the build's "What's new"; without it the notes are empty.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Length of the short commit hash.
const SHORT: usize = 7;

/// Longest release notes kept, in bytes.
const MAX_NOTES: usize = 64 * 1024;

#[expect(
    clippy::disallowed_methods,
    reason = "the build script asks git about the commit it builds"
)]
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn env_value(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// A build number is digits only; anything else would end up in `--version`, the logs and the
/// update comparison.
fn is_number(s: &str) -> bool {
    !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_hash(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reruns the script when the file git keeps at `git_path` (relative to `.git`) changes.
fn watch(git_path: &str) {
    let Some(found) = git(&["rev-parse", "--git-path", git_path]) else {
        return;
    };
    let found = Path::new(&found);
    let file = if found.is_absolute() {
        found.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(found)
    };
    println!("cargo:rerun-if-changed={}", file.display());
}

/// Reruns the script when the checked-out commit changes.
fn watch_git() {
    watch("HEAD");
    watch("packed-refs");
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watch(&branch);
    }
}

/// Writes the release notes the workflow named, or an empty file, to `release-notes.md` in the
/// build's output folder, where `build_info.rs` includes it.
fn embed_notes() -> io::Result<()> {
    println!("cargo:rerun-if-env-changed=CANOPY_RELEASE_NOTES_FILE");
    let out = std::env::var_os("OUT_DIR")
        .map(|dir| PathBuf::from(dir).join("release-notes.md"))
        .ok_or_else(|| io::Error::other("OUT_DIR is not set"))?;
    let notes = match env_value("CANOPY_RELEASE_NOTES_FILE") {
        Some(path) => {
            println!("cargo:rerun-if-changed={path}");
            let mut text = std::fs::read_to_string(&path)?;
            if text.len() > MAX_NOTES {
                let mut end = MAX_NOTES;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            text
        }
        None => String::new(),
    };
    std::fs::write(out, notes)
}

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-env-changed=CANOPY_BUILD_NUMBER");
    println!("cargo:rerun-if-env-changed=CANOPY_COMMIT");
    println!("cargo:rerun-if-changed=build.rs");

    let given_number = env_value("CANOPY_BUILD_NUMBER").filter(|n| is_number(n));
    let given_commit = env_value("CANOPY_COMMIT").filter(|c| is_hash(c));
    if given_number.is_none() || given_commit.is_none() {
        watch_git();
    }
    // A shallow clone (CI's default checkout) holds a fraction of the history: counting it would
    // name the build with a wrong number, so it gets none.
    let shallow = git(&["rev-parse", "--is-shallow-repository"]).is_none_or(|s| s != "false");
    let number = given_number
        .or_else(|| {
            (!shallow)
                .then(|| git(&["rev-list", "--count", "HEAD"]))
                .flatten()
                .filter(|n| is_number(n))
        })
        .unwrap_or_else(|| "dev".to_owned());
    let commit = given_commit
        .or_else(|| git(&["rev-parse", "HEAD"]).filter(|c| is_hash(c)))
        .map(|c| c.chars().take(SHORT).collect::<String>())
        .unwrap_or_default();
    println!("cargo:rustc-env=CANOPY_BUILD_NUMBER={number}");
    println!("cargo:rustc-env=CANOPY_COMMIT={commit}");
    embed_notes()
}
