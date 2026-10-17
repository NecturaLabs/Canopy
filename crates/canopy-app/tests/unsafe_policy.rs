//! Canopy's policy on `unsafe` (AGENTS.md; design spec 7.3): the backdrop blur's OpenGL calls in
//! `crates/nectura-ui/src/kit/glass/gl.rs` are the only unsafe code. The workspace denies the
//! `unsafe_code` lint and that file alone explicitly expects it; these tests fail when unsafe code, or an
//! attribute naming the lint, appears in any other Rust file under `crates/`, and when the
//! workspace or a crate stops denying it. Comments and string literals are ignored, so messages
//! such as "the archive has an unsafe path" are not code.
//!
//! What is scanned: every `.rs` file under each crate except that crate's own top-level `target`
//! directory (a directory named `target` anywhere else is scanned). A directory that cannot be
//! read, or a symbolic link, fails the scan instead of being skipped.
//!
//! Code that reaches the compiler from outside those files is rejected, so it cannot hide from
//! the scan: `include!` is refused anywhere (`gl.rs` included), as is naming it in a path or a
//! `use` so that it could be called under another name, and `#[path = "..."]` must name a
//! `.rs` file inside the crate as a plain string literal. That also covers a build script's
//! output: generated Rust in `OUT_DIR` can only be compiled through `include!` or `#[path]`, and
//! both are refused. `include_str!` and `include_bytes!` yield data, never code; one that names a
//! `.rs` file fails anyway. Behind all of this the compiler's own `unsafe_code = "deny"` rejects
//! unsafe code in any file, scanned or not, unless a suppression reaches it, and a suppression in
//! a scanned file is exactly what these tests report.
//!
//! What this does not cover: unsafe code inside dependencies, including what a dependency's macro
//! expands to in our files (crossbeam's `select!` in `worker.rs` expands to `unsafe` blocks; the
//! compiler treats macro output from another crate as that crate's code). That is third-party
//! code, chosen and reviewed as a dependency (`deny.toml`, `vendor/`), not unsafe code we write.

use std::path::{Path, PathBuf};

/// The one file allowed to contain unsafe code, relative to the repository root.
const ALLOWED: &str = "crates/nectura-ui/src/kit/glass/gl.rs";
/// Its first attribute, as the design spec words it (rustfmt may wrap it).
const EXPECT_LINE: &str = r#"#![expect(unsafe_code, reason = "OpenGL calls for the backdrop blur; owner-approved 2026-10-05")]"#;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.rs` file under `dir`, appended to `out`; whatever cannot be scanned (an unreadable
/// directory, a symbolic link) is appended to `problems`. `crate_root` is true for a crate's own
/// directory, whose top-level `target` directory (build output) is the only one skipped.
fn collect_rust_files(
    dir: &Path,
    crate_root: bool,
    out: &mut Vec<PathBuf>,
    problems: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            problems.push(format!("{}: cannot read directory: {err}", relative(dir)));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                problems.push(format!("{}: cannot read an entry: {err}", relative(dir)));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                problems.push(format!("{}: cannot stat: {err}", relative(&path)));
                continue;
            }
        };
        if file_type.is_symlink() {
            problems.push(format!(
                "{}: symbolic link (the scan does not follow links)",
                relative(&path)
            ));
        } else if file_type.is_dir() {
            if crate_root && path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rust_files(&path, false, out, problems);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("rs"))
        {
            out.push(path);
        }
    }
}

/// Every `.rs` file of every crate under `crates`, with the problems met on the way.
fn rust_files(crates: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut problems = Vec::new();
    match std::fs::read_dir(crates) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(kind) if kind.is_symlink() => problems.push(format!(
                        "{}: symbolic link (the scan does not follow links)",
                        relative(&path)
                    )),
                    Ok(kind) if kind.is_dir() => {
                        collect_rust_files(&path, true, &mut files, &mut problems);
                    }
                    Ok(_)
                        if path
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("rs")) =>
                    {
                        files.push(path);
                    }
                    Ok(_) => {}
                    Err(err) => problems.push(format!("{}: cannot stat: {err}", relative(&path))),
                }
            }
        }
        Err(err) => problems.push(format!(
            "{}: cannot read directory: {err}",
            relative(crates)
        )),
    }
    (files, problems)
}

/// `src` with every comment, string and character literal blanked out (line breaks kept, so line
/// numbers still match).
fn code_only(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |c: char, out: &mut String| out.push(if c == '\n' { '\n' } else { ' ' });
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let prev_ident = i > 0 && ident(chars[i - 1]);
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                blank(chars[i], &mut out);
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(chars[i], &mut out);
                    i += 1;
                }
            }
            continue;
        }
        // Raw strings: r"..", r#".."#, br#".."#, cr#".."# (not part of a longer identifier).
        if !prev_ident && matches!(c, 'r' | 'b' | 'c') {
            let mut j = i;
            if matches!(chars[j], 'b' | 'c') && chars.get(j + 1) == Some(&'r') {
                j += 1;
            }
            if chars[j] == 'r' {
                let mut k = j + 1;
                let mut hashes = 0;
                while chars.get(k) == Some(&'#') {
                    hashes += 1;
                    k += 1;
                }
                if chars.get(k) == Some(&'"') {
                    let close: Vec<char> = std::iter::once('"')
                        .chain(std::iter::repeat_n('#', hashes))
                        .collect();
                    let mut end = k + 1;
                    while end < chars.len() && !chars[end..].starts_with(&close) {
                        end += 1;
                    }
                    let end = (end + close.len()).min(chars.len());
                    for &ch in &chars[i..end] {
                        blank(ch, &mut out);
                    }
                    i = end;
                    continue;
                }
            }
        }
        // Strings, with b"" and c"" prefixes.
        if c == '"' {
            blank(c, &mut out);
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                blank(ch, &mut out);
                i += 1;
                if ch == '\\' {
                    if let Some(&escaped) = chars.get(i) {
                        blank(escaped, &mut out);
                        i += 1;
                    }
                } else if ch == '"' {
                    break;
                }
            }
            continue;
        }
        // Character literals ('a', '\n', '\u{1F600}', b'x'); a lone quote is a lifetime or label.
        if c == '\'' {
            let literal_end = if next == Some('\\') {
                // The escaped character (`'`, `\`, `n`, `u`, `x`) is never the closing quote, so
                // look for that after it: `'\''`, `'\\'`, `'\u{27}'`, `'\x27'`.
                let mut k = i + 3;
                while k < chars.len() && chars[k] != '\'' && chars[k] != '\n' {
                    k += 1;
                }
                (chars.get(k) == Some(&'\'')).then_some(k + 1)
            } else if chars.get(i + 2) == Some(&'\'') {
                Some(i + 3)
            } else {
                None
            };
            if let Some(end) = literal_end {
                for &ch in &chars[i..end] {
                    blank(ch, &mut out);
                }
                i = end;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Every identifier or keyword in `code` with its 1-based line.
fn words(code: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in code.lines().enumerate() {
        let mut word = String::new();
        for c in line.chars().chain(std::iter::once(' ')) {
            if c.is_alphanumeric() || c == '_' {
                word.push(c);
            } else if !word.is_empty() {
                out.push((n + 1, std::mem::take(&mut word)));
            }
        }
    }
    out
}

/// The first non-whitespace position at or after `from`.
fn skip_space(chars: &[char], mut from: usize) -> usize {
    while chars.get(from).is_some_and(|c| c.is_whitespace()) {
        from += 1;
    }
    from
}

/// The string literal (`"..."`) starting at `at`, without its quotes, or `None` when what starts
/// there is anything else (a raw string, a macro call, an identifier).
fn plain_string(src: &[char], at: usize) -> Option<String> {
    if src.get(at) != Some(&'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = at + 1;
    while let Some(&c) = src.get(i) {
        match c {
            '"' => return Some(out),
            '\\' => {
                out.push(c);
                i += 1;
                if let Some(&escaped) = src.get(i) {
                    out.push(escaped);
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    None
}

/// Whether a `#[path = "..."]` value names a `.rs` file inside the crate (no absolute path, no
/// `..`, nothing under a `target` directory, which the scan skips).
fn path_attribute_is_scanned(path: &str) -> bool {
    let p = Path::new(path);
    p.extension().is_some_and(|e| e == "rs")
        && !path.contains('\\')
        && p.components().all(|c| match c {
            std::path::Component::Normal(part) => part != "target",
            std::path::Component::CurDir => true,
            _ => false,
        })
}

/// The character ranges inside `#[...]` and `#![...]` in `code` (comments and strings blanked).
fn attribute_bodies(code: &[char]) -> Vec<std::ops::Range<usize>> {
    let mut bodies = Vec::new();
    for (at, &c) in code.iter().enumerate() {
        if c != '#' {
            continue;
        }
        let mut open = skip_space(code, at + 1);
        if code.get(open) == Some(&'!') {
            open = skip_space(code, open + 1);
        }
        if code.get(open) != Some(&'[') {
            continue;
        }
        let mut depth = 0_usize;
        let mut close = open;
        while let Some(&ch) = code.get(close) {
            if ch == '[' {
                depth += 1;
            } else if ch == ']' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            close += 1;
        }
        bodies.push(open..close);
    }
    bodies
}

/// Whether the word `include` at `start` names the macro without calling it: as a path segment
/// (`std::include`) or in a `use` declaration (`use core::{include as i}`), so that it can be
/// called under another name.
fn names_the_macro(code: &[char], start: usize) -> bool {
    let before: String = code[..start].iter().collect();
    let before = before.trim_end();
    if before.ends_with("::") {
        return true;
    }
    if !(before.ends_with('{') || before.ends_with(',')) {
        return false;
    }
    let statement = before.rsplit(';').next().unwrap_or(before);
    statement
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "use")
}

/// How `src` pulls in code from places the scan does not read: `include!`, a `#[path]` that is not
/// a plain string literal naming a `.rs` file inside the crate, and `include_str!` or
/// `include_bytes!` of a `.rs` file. Each entry is a 1-based line and a reason.
fn code_from_outside(src: &str) -> Vec<(usize, String)> {
    let orig: Vec<char> = src.chars().collect();
    let code: Vec<char> = code_only(src).chars().collect();
    if orig.len() != code.len() {
        return vec![(1, "the scanner lost track of the source".to_owned())];
    }
    let line_of = |at: usize| orig[..at].iter().filter(|&&c| c == '\n').count() + 1;
    let attributes = attribute_bodies(&code);
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut found = Vec::new();
    let mut at = 0;
    while at < code.len() {
        if !ident(code[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while code.get(at).is_some_and(|&c| ident(c)) {
            at += 1;
        }
        let word: String = code[start..at].iter().collect();
        let after = skip_space(&code, at);
        let is_macro = code.get(after) == Some(&'!');
        match word.as_str() {
            "include" if is_macro || names_the_macro(&code, start) => found.push((
                line_of(start),
                "`include!` compiles code the scan cannot read".to_owned(),
            )),
            "include_str" | "include_bytes" if is_macro => {
                // Data, not code; but a Rust source file named in the call is not data.
                let mut end = after;
                let mut depth = 0_usize;
                while let Some(&ch) = code.get(end) {
                    if matches!(ch, '(' | '[' | '{') {
                        depth += 1;
                    } else if matches!(ch, ')' | ']' | '}') {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            break;
                        }
                    }
                    end += 1;
                }
                let call: String = orig[after..end.min(orig.len())].iter().collect();
                if call.to_ascii_lowercase().contains(".rs\"") {
                    found.push((
                        line_of(start),
                        format!("`{word}!` names a Rust source file: {call}"),
                    ));
                }
            }
            "path"
                if attributes.iter().any(|a| a.contains(&start))
                    && code.get(after) == Some(&'=')
                    && code.get(after + 1) != Some(&'=') =>
            {
                let value = skip_space(&orig, after + 1);
                match plain_string(&orig, value) {
                    Some(path) if path_attribute_is_scanned(&path) => {}
                    Some(path) => found.push((
                        line_of(start),
                        format!("`path = \"{path}\"` is not a `.rs` file inside the crate"),
                    )),
                    None => found.push((
                        line_of(start),
                        "`path = ...` is not a plain string literal".to_owned(),
                    )),
                }
            }
            _ => {}
        }
    }
    found
}

fn relative(path: &Path) -> String {
    let root = root().canonicalize().unwrap_or_else(|_| root());
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    path.strip_prefix(&root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn only_the_glass_renderer_is_unsafe() {
    let (files, mut found) = rust_files(&root().join("crates"));
    assert!(files.len() > 50, "found only {} Rust files", files.len());
    let mut allowed_seen = false;
    for path in &files {
        let rel = relative(path);
        let Ok(src) = std::fs::read_to_string(path) else {
            found.push(format!("{rel}: unreadable"));
            continue;
        };
        for (line, reason) in code_from_outside(&src) {
            found.push(format!("{rel}:{line}: {reason}"));
        }
        if rel == ALLOWED {
            allowed_seen = true;
            let squash = |s: &str| s.split_whitespace().collect::<String>();
            let first = src.split_inclusive(")]").next().unwrap_or_default();
            assert_eq!(
                squash(first),
                squash(EXPECT_LINE),
                "{ALLOWED} must begin with the owner-approved expectation"
            );
            continue;
        }
        for (line, word) in words(&code_only(&src)) {
            if word == "unsafe" || word == "unsafe_code" {
                found.push(format!("{rel}:{line}: `{word}`"));
            }
        }
    }
    assert!(allowed_seen, "{ALLOWED} is missing");
    assert!(
        found.is_empty(),
        "unsafe code, or code the scan cannot read, outside {ALLOWED}:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_workspace_denies_unsafe_code() {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).unwrap_or_default();
    let section = |name: &str| {
        manifest
            .split(&format!("[{name}]"))
            .nth(1)
            .and_then(|rest| rest.split("\n[").next())
            .unwrap_or_default()
            .to_owned()
    };
    let rust = section("workspace.lints.rust");
    assert!(rust.contains("unsafe_code = \"deny\""), "{rust}");
    let clippy = section("workspace.lints.clippy");
    for lint in [
        "undocumented_unsafe_blocks = \"deny\"",
        "multiple_unsafe_ops_per_block = \"deny\"",
        "unnecessary_safety_comment = \"warn\"",
    ] {
        assert!(clippy.contains(lint), "missing {lint}");
    }
    let Ok(crates) = std::fs::read_dir(root().join("crates")) else {
        panic!("no crates directory");
    };
    let mut checked = 0;
    for krate in crates.flatten() {
        let Ok(manifest) = std::fs::read_to_string(krate.path().join("Cargo.toml")) else {
            continue;
        };
        let lints = manifest
            .split("[lints]")
            .nth(1)
            .and_then(|rest| rest.split("\n[").next())
            .unwrap_or_default();
        // A crate inherits the workspace lints, or (the shared component library, which other
        // workspaces use too) carries the same unsafe-related ones in its own manifest.
        let lint_lines = |name: &str| {
            manifest
                .split(&format!("[lints.{name}]"))
                .nth(1)
                .and_then(|rest| rest.split("\n[").next())
                .unwrap_or_default()
                .lines()
                .filter(|line| !line.trim_start().starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let (rust, clippy) = (lint_lines("rust"), lint_lines("clippy"));
        let own = rust.contains("unsafe_code = \"deny\"")
            && [
                "undocumented_unsafe_blocks = \"deny\"",
                "multiple_unsafe_ops_per_block = \"deny\"",
                "unnecessary_safety_comment = \"warn\"",
            ]
            .iter()
            .all(|lint| clippy.contains(lint));
        assert!(
            lints.contains("workspace = true") || own,
            "{} neither inherits the workspace lints nor denies unsafe code itself",
            krate.path().display()
        );
        checked += 1;
    }
    assert!(checked >= 3, "checked {checked} crates");
}

/// The manifest's `deny` holds only if no build configuration lifts it: cargo config, CI and the
/// tools that build releases must not pass `-A unsafe_code` or cap lints.
#[test]
fn no_build_configuration_lifts_the_unsafe_lint() {
    let mut files = vec![
        root().join(".cargo/config"),
        root().join(".cargo/config.toml"),
    ];
    for dir in [".github/workflows", ".github/actions/setup-rust", "tools"] {
        if let Ok(entries) = std::fs::read_dir(root().join(dir)) {
            files.extend(entries.flatten().map(|e| e.path()));
        }
    }
    let mut read = 0;
    for file in files.iter().filter(|f| f.is_file()) {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        read += 1;
        assert_eq!(
            lint_lifts(&text),
            Vec::<String>::new(),
            "{} lifts the unsafe_code lint",
            relative(file)
        );
    }
    assert!(read >= 5, "read {read} build files");
}

/// Every place in `text` (a config file, workflow or script) that would lift the `unsafe_code`
/// deny: an `-A`/`-W`/`--allow`/`--warn`/`--force-warn` naming the lint, or lint capping. The
/// command line spells lint names with `-` or `_` alike.
fn lint_lifts(text: &str) -> Vec<String> {
    let text = text.replace('-', "_");
    let mut found = Vec::new();
    if text.contains("cap_lints") {
        found.push("cap-lints".to_owned());
    }
    let pieces: Vec<&str> = text.split("unsafe_code").collect();
    for piece in &pieces[..pieces.len() - 1] {
        let before = piece.trim_end_matches([' ', '=', '"', '\'', ',', '\t']);
        if let Some(flag) = ["_A", "_W", "__allow", "__warn", "__force_warn"]
            .iter()
            .find(|flag| before.ends_with(*flag))
        {
            found.push(format!("{} unsafe-code", flag.replace('_', "-")));
        }
    }
    found
}

#[test]
fn lint_lifting_flags_are_found_in_any_spelling() {
    for lifted in [
        "RUSTFLAGS=\"-A unsafe_code\"",
        "RUSTFLAGS='-A unsafe-code' cargo build",
        "rustflags = [\"-A\", \"unsafe_code\"]",
        "cargo rustc -- -Aunsafe-code",
        "rustflags = [\"--allow=unsafe-code\"]",
        "RUSTFLAGS=\"-W unsafe_code\"",
        "RUSTFLAGS=\"--force-warn unsafe-code\"",
        "RUSTFLAGS=\"--cap-lints warn\"",
    ] {
        assert_ne!(lint_lifts(lifted), Vec::<String>::new(), "{lifted}");
    }
    for fine in [
        "# the unsafe-code policy (only gl.rs may contain unsafe code)",
        "cargo test -p canopy-app --test unsafe_policy",
        "RUSTFLAGS=\"-D warnings\"",
    ] {
        assert_eq!(lint_lifts(fine), Vec::<String>::new(), "{fine}");
    }
}

#[test]
fn comments_and_strings_are_not_code() {
    let src = r###"
// unsafe in a comment
/* unsafe /* nested unsafe */ still comment */
/// unsafe doc
let a = "unsafe \" still string";
let b = r#"unsafe "quoted" "#;
let c = br##"unsafe"##;
let d = b"unsafe";
let e = '"'; let f = '\''; let g = b'\\';
fn h<'a>(x: &'a str) -> &'a str { 'outer: loop { break 'outer x; } }
let not_unsafe = 1; let unsafe_ish = 2;
unsafe { real() }
"###;
    let hits: Vec<usize> = words(&code_only(src))
        .into_iter()
        .filter(|(_, w)| w == "unsafe")
        .map(|(line, _)| line)
        .collect();
    assert_eq!(hits, [12]);
    assert_eq!(code_only(src).lines().count(), src.lines().count());
    // An attribute naming the lint counts too.
    let attr = "#![allow(unsafe_code)]\nfn main() {}\n";
    assert!(
        words(&code_only(attr))
            .iter()
            .any(|(_, w)| w == "unsafe_code")
    );
}

/// The 1-based lines of `src` where the scanner sees `unsafe` or `unsafe_code` as code.
fn unsafe_lines(src: &str) -> Vec<usize> {
    words(&code_only(src))
        .into_iter()
        .filter(|(_, w)| w == "unsafe" || w == "unsafe_code")
        .map(|(line, _)| line)
        .collect()
}

#[test]
fn literals_do_not_hide_the_code_after_them() {
    // Each line is valid Rust that holds quotes in a literal; `unsafe` inside its strings is not
    // code, and the `unsafe` block on the next line must still be reported (security pass F1:
    // `'\''` once flipped the scanner's string state and hid everything up to the next quote).
    let lines = [
        r#"let _ = stringify!('\''"'");"#,
        r#"let _ = ('\'', "unsafe");"#,
        r#"let _ = (b'\'', "unsafe");"#,
        r#"let _ = ('\u{27}', "unsafe");"#,
        r#"let _ = ('\x27', "unsafe");"#,
        r#"let _ = ('\u{22}', "unsafe");"#,
        r#"let _ = ('\\', "unsafe");"#,
        r#"let _ = (b'\\', "unsafe");"#,
        r#"let _ = ('"', "unsafe");"#,
        r#"let _ = (b'"', "unsafe");"#,
        r#"let _ = ('\"', "unsafe");"#,
        r#"let _ = ('\n', '\t', '\0', "unsafe");"#,
        r#"let _ = ('\u{1F600}', '😀', 'é', "unsafe");"#,
        r#"let _ = ('{', '}', '[', '/', '*', "unsafe");"#,
        r##"let _ = (r#"a " quote and unsafe"#, "unsafe");"##,
        r###"let _ = (r##"a "# still unsafe"##, "unsafe");"###,
        r##"let _ = (br#"b " unsafe"#, cr#"c " unsafe"#, "unsafe");"##,
        r#"let _ = ("a \" unsafe \\", "unsafe");"#,
        r#"let _ = "/* unsafe"; let _ = "// unsafe";"#,
        r#"let _ = ("'", '"', "'", "unsafe");"#,
    ];
    let mut wrong = Vec::new();
    for line in lines {
        let src = format!("{line}\nunsafe {{ real() }}\n");
        if unsafe_lines(&src) != [2] || code_only(&src).lines().count() != src.lines().count() {
            wrong.push(line);
        }
    }
    assert!(wrong.is_empty(), "mis-scanned:\n{}", wrong.join("\n"));
}

#[test]
fn lifetimes_and_labels_are_not_character_literals() {
    let src = r#"
fn f<'a, 'b: 'a>(x: &'a str, y: &'b str) -> &'a str { let _ = ('a', "unsafe"); x }
struct S<'a>(&'a str); impl<'a> S<'a> { fn g(&self) -> &'a str { self.0 } }
fn h() { 'outer: loop { 'inner: loop { break 'outer; } } let _ = "unsafe"; }
fn i(_: &'static str) { let _ = 'x'; let _ = "unsafe"; }
unsafe { real() }
"#;
    assert_eq!(unsafe_lines(src), [6]);
}

#[test]
fn comments_do_not_hide_the_code_around_them() {
    let src = r#"
/* a /* b */ unsafe c */ let _ = 1;
/* it's a "quote */ let _ = 2;
// it's a "quote
/// doc with a "quote and unsafe
//! inner doc '
/*/ unsafe */
/**/ let _ = "unsafe";
/* "unsafe" /* 'x */ unsafe */ let _ = 3;
unsafe { real() }
"#;
    assert_eq!(unsafe_lines(src), [10]);
}

#[test]
fn a_string_continued_over_lines_stays_a_string() {
    let src = "let _ = \"unsafe \\\n   unsafe\";\nunsafe { real() }\n";
    assert_eq!(unsafe_lines(src), [3]);
}

#[test]
fn code_from_outside_the_scanned_files_is_rejected() {
    let rejected = [
        r#"include!("generated.in");"#,
        r#"include !("generated.in");"#,
        r#"core::include!("generated.in");"#,
        r#"include!(concat!(env!("OUT_DIR"), "/generated.rs"));"#,
        "include!(\n    \"generated.rs\"\n);",
        r#"#[path = "generated.txt"] mod generated;"#,
        r#"#[path = "../vendor/x.rs"] mod x;"#,
        r#"#[path = "/etc/x.rs"] mod x;"#,
        r#"#[path = "target/x.rs"] mod x;"#,
        r#"#[path = "src\\x.rs"] mod x;"#,
        r#"#[path = r"x.rs"] mod x;"#,
        r#"#![path = "x.in"]"#,
        r#"#[cfg_attr(unix, path = "x.txt")] mod x;"#,
        "#[\n    path\n        = \"x.txt\"\n] mod x;",
        "macro_rules! m { ($p:literal) => { #[path = $p] mod x; }; }",
        r#"const S: &str = include_str!("lib.rs");"#,
        r#"const S: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/x.rs"));"#,
        // The macro under another name is still the macro.
        r#"use std::include as inc; inc!("x.rs");"#,
        "use core::{include as i};",
        "use std::{self, include};",
        r#"fn f() { use std::{include as i}; i!("x.rs"); }"#,
    ];
    for src in rejected {
        let found = code_from_outside(src);
        assert_eq!(found.len(), 1, "{src}: {found:?}");
    }
    // The reported line is the line of the offending call.
    assert_eq!(
        code_from_outside("fn a() {}\n\ninclude!(\"x.in\");\n")[0].0,
        3
    );
    assert_eq!(
        code_from_outside("fn a() {}\n#[path = \"x.in\"]\nmod x;\n")[0].0,
        2
    );
}

#[test]
fn ordinary_code_is_not_rejected() {
    let fine = [
        r#"#[path = "generated.rs"] mod generated;"#,
        r#"#[path = "./nested/generated.rs"] mod generated;"#,
        r#"#[cfg_attr(unix, path = "unix.rs")] mod platform;"#,
        r#"const S: &str = include_str!("../../docs/legal/terms.md");"#,
        r#"const S: &str = include_str!(concat!(env!("OUT_DIR"), "/release-notes.md"));"#,
        r#"const F: &[u8] = include_bytes!("../assets/Inter-Regular.ttf");"#,
        r#"let path = "x.txt"; let path = path; assert!(path == "y.txt");"#,
        r##"let _ = "include!(x)"; let _ = "#[path = \"x.txt\"]";"##,
        "// include!(\"x.in\");\n/* #[path = \"x.txt\"] */\n/// include!(x)\n",
        "#[derive(Debug)] struct Include; fn include() {} fn r#path() {}",
        r#"#[cfg(feature = "path")] fn f() { let _ = include; }"#,
        "#[serde(path)] struct S;",
    ];
    for src in fine {
        assert_eq!(code_from_outside(src), [], "{src}");
    }
}

/// A scratch directory under cargo's per-target test directory, emptied first.
fn scratch(name: &str) -> std::io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("unsafe_policy_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

#[test]
fn only_a_crates_own_target_directory_is_skipped() -> std::io::Result<()> {
    let crates = scratch("target")?;
    write(&crates.join("a/src/lib.rs"), "")?;
    write(&crates.join("a/target/debug/build/gen.rs"), "unsafe {}")?;
    write(&crates.join("a/src/target/evil.rs"), "unsafe {}")?;
    write(&crates.join("a/tests/target/evil.rs"), "unsafe {}")?;
    write(&crates.join("a/src/deep/er/target/evil.rs"), "unsafe {}")?;
    write(&crates.join("b/build.rs"), "")?;
    write(&crates.join("b/src/SHOUT.RS"), "unsafe {}")?;
    let (files, problems) = rust_files(&crates);
    assert_eq!(problems, Vec::<String>::new());
    let mut names: Vec<String> = files
        .iter()
        .map(|f| {
            f.strip_prefix(&crates)
                .expect("under the scratch directory")
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "a/src/deep/er/target/evil.rs",
            "a/src/lib.rs",
            "a/src/target/evil.rs",
            "a/tests/target/evil.rs",
            "b/build.rs",
            "b/src/SHOUT.RS",
        ]
    );
    let _ = std::fs::remove_dir_all(&crates);
    Ok(())
}

#[test]
fn a_directory_that_cannot_be_read_fails_the_scan() -> std::io::Result<()> {
    let crates = scratch("unreadable")?;
    let (files, problems) = rust_files(&crates.join("missing"));
    assert_eq!(files, Vec::<PathBuf>::new());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("cannot read directory"),
        "{problems:?}"
    );
    let mut problems = Vec::new();
    collect_rust_files(&crates.join("nope"), false, &mut Vec::new(), &mut problems);
    assert_eq!(problems.len(), 1, "{problems:?}");
    let _ = std::fs::remove_dir_all(&crates);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symbolic_links_fail_the_scan() -> std::io::Result<()> {
    let crates = scratch("links")?;
    write(&crates.join("a/src/lib.rs"), "")?;
    write(&crates.join("elsewhere/hidden.rs"), "unsafe {}")?;
    std::os::unix::fs::symlink(crates.join("elsewhere"), crates.join("a/src/linked"))?;
    std::os::unix::fs::symlink(
        crates.join("elsewhere/hidden.rs"),
        crates.join("a/src/h.rs"),
    )?;
    std::os::unix::fs::symlink(crates.join("elsewhere"), crates.join("c"))?;
    let (_, problems) = rust_files(&crates);
    let linked = problems
        .iter()
        .filter(|p| p.contains("symbolic link"))
        .count();
    assert_eq!(linked, 3, "{problems:?}");
    let _ = std::fs::remove_dir_all(&crates);
    Ok(())
}
