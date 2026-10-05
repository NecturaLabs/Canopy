//! Canopy's policy on `unsafe` (AGENTS.md; design spec 7.3): the backdrop blur's OpenGL calls in
//! `crates/canopy-app/src/ui/kit/glass/gl.rs` are the only unsafe code. The workspace denies the
//! `unsafe_code` lint and that file alone allows it; these tests fail when unsafe code, or an
//! attribute naming the lint, appears in any other Rust file under `crates/`, and when the
//! workspace or a crate stops denying it. Comments and string literals are ignored, so messages
//! such as "the archive has an unsafe path" are not code.

use std::path::{Path, PathBuf};

/// The one file allowed to contain unsafe code, relative to the repository root.
const ALLOWED: &str = "crates/canopy-app/src/ui/kit/glass/gl.rs";
/// Its first attribute, as the design spec words it (rustfmt may wrap it).
const ALLOW_LINE: &str = r#"#![allow(unsafe_code, reason = "OpenGL calls for the backdrop blur; owner-approved 2026-10-05")]"#;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
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
    let mut files = Vec::new();
    rust_files(&root().join("crates"), &mut files);
    assert!(files.len() > 50, "found only {} Rust files", files.len());
    let mut found = Vec::new();
    let mut allowed_seen = false;
    for path in &files {
        let rel = relative(path);
        let Ok(src) = std::fs::read_to_string(path) else {
            found.push(format!("{rel}: unreadable"));
            continue;
        };
        if rel == ALLOWED {
            allowed_seen = true;
            let squash = |s: &str| s.split_whitespace().collect::<String>();
            let first = src.split_inclusive(")]").next().unwrap_or_default();
            assert_eq!(
                squash(first),
                squash(ALLOW_LINE),
                "{ALLOWED} must begin with the owner-approved allow"
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
        "unsafe code outside {ALLOWED}:\n{}",
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
        assert!(
            lints.contains("workspace = true"),
            "{} does not inherit the workspace lints",
            krate.path().display()
        );
        checked += 1;
    }
    assert!(checked >= 3, "checked {checked} crates");
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
