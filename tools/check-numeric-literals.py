#!/usr/bin/env python3
"""Flag numeric literals in rules code that have neither a name nor a comment.

The quality standard (C3) asks for a named constant, with its unit, for every meaningful number.
This checks the Git and hosting backends (`crates/canopy-core/src`, `crates/canopy-hosting/src`),
where the rules live; the UI takes its sizes and times from `ui/theme.rs` tokens, which
`check-ui-colors.py` and review guard. A literal passes when it is

- 0 or 1 (integer or float, any suffix), or part of a tuple index or a range bound written as a
  tuple/field access such as `.0`;
- in a `const` or `static` item, or in an attribute;
- on a line that carries a comment (the comment says what the number is);
- after the first `#[cfg(test)]`, where this project keeps its unit tests, or anywhere in a file
  that carries the inner attribute `#![cfg(test)]` (a file of test support or tests).

Strings, raw strings, characters, lifetimes and comments are skipped by a small scanner, not a
regular expression, so digits inside a message or a GraphQL query never count. Standard library
only; `--selftest` tests the scanner.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCOPES = ("crates/canopy-core/src/", "crates/canopy-hosting/src/")
ALLOWED = {"0", "1", "0.0", "1.0"}
SUFFIX = re.compile(r"_?(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64)$")
NUMBER = re.compile(r"\d[\w]*(?:\.\d[\w]*)?(?:[eE][+-]?\d+)?")


def scan(text: str) -> list[tuple[int, str, bool]]:
    """Every numeric literal of `text` as (line, value, line has a comment), strings skipped."""
    out: list[tuple[int, str, bool]] = []
    commented: set[int] = set()
    i, line, n = 0, 1, len(text)
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            commented.add(line)
            while i < n and text[i] != "\n":
                i += 1
        elif text.startswith("/*", i):
            depth = 0
            while i < n:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                    if depth == 0:
                        break
                else:
                    if text[i] == "\n":
                        line += 1
                    commented.add(line)
                    i += 1
        elif c == "r" and re.match(r'r#*"', text[i:i + 8]) and not (i and (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = len(re.match(r"r(#*)", text[i:]).group(1))
            end = text.find('"' + "#" * hashes, i + 2 + hashes)
            end = n if end < 0 else end + 1 + hashes
            line += text.count("\n", i, end)
            i = end
        elif c == '"':
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\":
                    i += 1
                if i < n and text[i] == "\n":
                    line += 1
                i += 1
            i += 1
        elif c == "'":
            # A character literal ('a', '\n', '\u{1f}') or a lifetime ('a, 'static).
            m = re.match(r"'(?:\\.[^']*|[^\\'])'", text[i:])
            i += m.end() if m else 1
        elif c.isdigit() and not (
            i and (text[i - 1].isalnum() or text[i - 1] == "_" or (text[i - 1] == "." and text[i - 2] != "."))
        ):
            m = NUMBER.match(text, i)
            out.append((line, m.group(), False))
            i = m.end()
        elif c.isalpha() or c == "_":
            m = re.compile(r"[A-Za-z_]\w*").match(text, i)
            i = m.end()
        else:
            i += 1
    return [(ln, v, ln in commented) for ln, v, _ in out]


def unnamed(text: str) -> list[tuple[int, str]]:
    """The literals of `text` outside tests that need a name or a comment."""
    head = [line.strip() for line in text.split("\n")]
    head = [line for line in head if line and not line.startswith("//")]
    if head and head[0] == "#![cfg(test)]":
        return []
    cut = text.find("#[cfg(test)]")
    if cut >= 0:
        text = text[:cut]
    lines = text.split("\n")
    exempt: set[int] = set()
    in_item = in_attribute = False
    for number, raw in enumerate(lines, 1):
        stripped = raw.strip()
        if re.match(r"(pub(\([a-z]+\))? )?(const|static) ", stripped):
            in_item = True
        if stripped.startswith("#[") or stripped.startswith("#!["):
            in_attribute = True
        if in_item or in_attribute:
            exempt.add(number)
        if in_item and stripped.endswith(";"):
            in_item = False
        if in_attribute and stripped.endswith("]"):
            in_attribute = False
    found = []
    for line, value, has_comment in scan(text):
        plain = SUFFIX.sub("", value.replace("_", ""))
        if has_comment or line in exempt or plain in ALLOWED:
            continue
        found.append((line, value))
    return found


def check() -> int:
    files = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "crates"],
        cwd=ROOT, check=True, stdout=subprocess.PIPE,
    ).stdout.decode("utf-8").split("\0")
    bad = 0
    for name in sorted(set(files)):
        if not name.endswith(".rs") or not name.startswith(SCOPES) or not (ROOT / name).is_file():
            continue
        text = (ROOT / name).read_text(encoding="utf-8")
        for line, value in unnamed(text):
            print(f"{name}:{line}: {value}: name this number as a const (with its unit), or comment it")
            bad += 1
    return int(bool(bad))


def selftest() -> int:
    def lits(src: str) -> list[str]:
        return [v for _, v in unnamed(src)]

    assert lits("let a = 0; let b = 1u8; let c = 1.0; let d = 0.0f32;") == []
    assert lits("let a = x.take(2000);") == ["2000"]
    assert lits("//! docs\n#![cfg(test)]\nlet a = x.take(2000);") == []
    assert lits("let a = 2 + 3;") == ["2", "3"]
    assert lits("let a = x.take(2000); // lines kept") == []
    assert lits("const LIMIT: usize = 2000;") == []
    assert lits("pub(crate) static T: [u8; 2] = [\n 3,\n 4,\n];\nlet x = 5;") == ["5"]
    assert lits('#[expect(clippy::x, reason = "see 42")]\nfn f() {}') == []
    assert lits('let m = "status 404 on 12 lines"; let r = r#"first: 50"#;') == []
    assert lits('let m = "multi \\\n line 77";\nlet x = 9;') == ["9"]
    assert lits("let q = r\"\n 500 \n\"; let y = 8;") == ["8"]
    assert lits("let c = 'a'; let d = '\\n'; let e: &'static str = \"\"; let z = 6;") == ["6"]
    assert lits("let t = pair.0 + pair.1; let r = &x[0..1];") == []
    assert lits("let r = &x[2..4];") == ["2", "4"]
    assert lits("let big = 0x7f_u8; let f = 1e3;") == ["0x7f_u8", "1e3"]
    assert lits("/* 100 */ let a = 7;\nlet b = 8; /* 9\n 10 */") == []
    assert lits("/* 100 */\nlet a = 7;") == ["7"]
    assert lits("let ok = 1;\n#[cfg(test)]\nmod tests { fn t() { assert_eq!(f(), 42); } }") == []
    assert lits("let v = x7 + y_8;") == []
    print("numeric literal check self-test passed")
    return 0


if __name__ == "__main__":
    sys.exit(selftest() if sys.argv[1:] == ["--selftest"] else check())
