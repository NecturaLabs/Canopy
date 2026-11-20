#!/usr/bin/env python3
"""Guard the theme boundary: runtime UI colours are defined in nectura-ui's theme.rs."""
from __future__ import annotations
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PREFIX = "crates/canopy-app/src/"
# The component library is a submodule, so `git ls-files` of this repository does not list its files.
LIBRARY = "crates/nectura-ui/src/"
THEME = LIBRARY + "theme.rs"
CONSTRUCTOR = re.compile(
    r"\bColor32\s*::\s*from_(?:rgb\w*|gray|black_alpha|white_alpha|additive_luminance|hex)\s*\("
)
NAMED = re.compile(r"\bColor32\s*::\s*([A-Z][A-Z0-9_]*)\b")
# Exact non-design expressions: texture/alpha channels, black-or-white contrast selection, gallery
# opacity/blur samples. One complete, unique expression keeps every other use a violation.
ALLOWED_CONSTANTS = {
    (LIBRARY + "contrast.rs", "BLACK"): [
        (
            "if worst(Color32::WHITE) >= worst(Color32::BLACK) {\n        Color32::WHITE\n    } else {\n        Color32::BLACK\n    }",
            "black is one of the two contrast candidates",
        )
    ],
    (LIBRARY + "contrast.rs", "WHITE"): [
        (
            "if worst(Color32::WHITE) >= worst(Color32::BLACK) {\n        Color32::WHITE\n    } else {\n        Color32::BLACK\n    }",
            "white is one of the two contrast candidates",
        )
    ],
    (LIBRARY + "kit/avatar.rs", "WHITE"): [
        (
            "RectShape::filled(rect, radius, Color32::WHITE.gamma_multiply(shown))\n                .with_texture(tex, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)))",
            "white carries a picture's alpha channel, not a theme colour",
        )
    ],
    (LIBRARY + "kit/paint.rs", "WHITE"): [
        (
            "RectShape::filled(rect, radius, Color32::WHITE)\n        .with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)))",
            "white carries texture alpha, not a theme colour",
        ),
        (
            "Palette::tint(Color32::WHITE, crate::saturate_u8(glow_falloff(t) * 255.0))",
            "white carries glow alpha, not a theme colour",
        ),
    ],
    (LIBRARY + "kit/glass/mod.rs", "BLACK"): [
        (
            "let c = if k % 2 == 0 {\n            Color32::WHITE\n        } else {\n            Color32::BLACK\n        };",
            "the glass gallery uses black as opacity/blur test content",
        ),
        (
            "if row % 2 == 0 { p.text } else { Color32::BLACK },",
            "the glass gallery uses black as opacity/blur test content",
        ),
    ],
    (LIBRARY + "kit/glass/mod.rs", "WHITE"): [
        (
            "let c = if k % 2 == 0 {\n            Color32::WHITE\n        } else {\n            Color32::BLACK\n        };",
            "the glass gallery uses white as opacity/blur test content",
        )
    ],
}
TEST_ITEM = re.compile(
    r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]"
    r"(?:\s*#\[[^]]*\])*\s*(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:async\s+)?(?:mod|fn)\s+\w+\b"
)
RAW_STRING = re.compile(r'r(#{0,255})"')
CHAR_LITERAL = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'")

def code_only(source: str) -> str:
    """Blank Rust comments and literals, preserving offsets and line numbers."""
    spans = []
    i = 0
    while i < len(source):
        start = i
        if source.startswith("//", i):
            end = source.find("\n", i)
            i = len(source) if end < 0 else end
        elif source.startswith("/*", i):
            depth = 1
            i += 2
            while i < len(source) and depth:
                if source.startswith("/*", i):
                    depth += 1
                    i += 2
                elif source.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
        elif raw := RAW_STRING.match(source, i):
            delimiter = '"' + raw[1]
            end = source.find(delimiter, raw.end())
            i = len(source) if end < 0 else end + len(delimiter)
        elif source[i] == '"':
            i += 1
            while i < len(source):
                if source[i] == "\\":
                    i += 2
                elif source[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
        elif char := CHAR_LITERAL.match(source, i):
            i = char.end()
        else:
            i += 1
            continue
        spans.append((start, i))
    chars = list(source)
    for start, end in spans:
        chars[start:end] = ["\n" if c == "\n" else " " for c in source[start:end]]
    return "".join(chars)

def runtime_code(source: str) -> str:
    """Exclude each test-only function/module body, never the remainder of its file."""
    code = code_only(source)
    chars = list(code)
    for item in TEST_ITEM.finditer(code):
        start = item.end()
        nesting = 0
        while start < len(code):
            c = code[start]
            if c in "([":
                nesting += 1
            elif c in ")]":
                nesting -= 1
            elif c in "{;" and nesting == 0:
                break
            start += 1
        if start == len(code) or code[start] == ";":
            continue
        depth = 1
        end = start + 1
        while end < len(code) and depth:
            depth += (code[end] == "{") - (code[end] == "}")
            end += 1
        chars[item.start():end] = ["\n" if c == "\n" else " " for c in code[item.start():end]]
    return "".join(chars)

def violations(name: str, source: str) -> list[tuple[int, str]]:
    if name == THEME or not name.startswith((PREFIX, LIBRARY)) or not name.endswith(".rs"):
        return []
    if "/tests/" in name or name.endswith("/tests.rs"):
        return []
    runtime = runtime_code(source)
    problems = []
    for m in CONSTRUCTOR.finditer(runtime):
        # Colour-space conversion is not a hard-coded design colour.
        if name == LIBRARY + "contrast.rs" and runtime[m.start():].startswith(
            "Color32::from_rgb(from_linear("
        ):
            continue
        problems.append((source.count("\n", 0, m.start()) + 1, m.group()[:-1].strip()))
    for m in NAMED.finditer(runtime):
        constant = m[1]
        if constant in {"TRANSPARENT", "PLACEHOLDER"}:
            continue
        allowed = ALLOWED_CONSTANTS.get((name, constant), [])
        if any(
            runtime.count(expression) == 1
            and runtime.find(expression) <= m.start()
            and m.end() <= runtime.find(expression) + len(expression)
            for expression, _ in allowed
        ):
            continue
        problems.append((source.count("\n", 0, m.start()) + 1, m.group()))
    return problems

def check() -> int:
    out = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", PREFIX],
        cwd=ROOT, check=True, stdout=subprocess.PIPE,
    ).stdout
    names = set(out.decode("utf-8").split("\0"))
    # crates/nectura-ui is a submodule: git ls-files lists only its link, not its files.
    library = {
        path.relative_to(ROOT).as_posix()
        for path in (ROOT / LIBRARY).rglob("*.rs")
        if "target" not in path.parts
    }
    if not library:
        print(f"check-ui-colors: no Rust files under {LIBRARY}: run `git submodule update --init`")
        return 1
    names |= library
    failed = False
    for (name, constant), cases in ALLOWED_CONSTANTS.items():
        runtime = runtime_code((ROOT / name).read_text(encoding="utf-8"))
        for expression, reason in cases:
            if runtime.count(expression) != 1:
                print(f"{name}: stale colour allowlist entry for Color32::{constant}: {reason}")
                failed = True
    for name in sorted(names):
        path = ROOT / name
        if not name.endswith(".rs") or not path.is_file():
            continue
        for lineno, constructor in violations(name, path.read_text(encoding="utf-8")):
            print(f"{name}:{lineno}: {constructor} belongs in nectura-ui's theme.rs or a test")
            failed = True
    return int(failed)

def selftest() -> int:
    p = PREFIX + "views/synthetic.rs"
    assert violations(p, "Color32::from_rgb(1, 2, 3)") == [(1, "Color32::from_rgb")]
    assert violations(p, "\nColor32::from_hex(\"#fff\")") == [(2, "Color32::from_hex")]
    assert violations(p, "Color32::from_black_alpha(4)") == [(1, "Color32::from_black_alpha")]
    assert violations(p, "Color32::from_white_alpha(4)") == [(1, "Color32::from_white_alpha")]
    assert violations(p, "Color32::from_additive_luminance(4)") == [
        (1, "Color32::from_additive_luminance")
    ]
    assert violations(p, "Color32::WHITE") == [(1, "Color32::WHITE")]
    assert violations(p, "Color32::TRANSPARENT") == []
    assert violations(p, "Color32::PLACEHOLDER") == []
    assert violations(p, "#[cfg(test)]\nmod tests { fn check() { Color32::from_gray(4); } }") == []
    assert violations(LIBRARY + "kit/glass/tests.rs", "Color32::from_rgb(1,2,3)") == []
    assert violations(THEME, "Color32::from_rgb(1,2,3)") == []
    assert violations(LIBRARY + "contrast.rs",
        "Color32::from_rgb(from_linear(r), from_linear(g), from_linear(b))") == []
    assert violations(LIBRARY + "contrast.rs", "Color32::from_rgb(1,2,3)") == [
        (1, "Color32::from_rgb")
    ]
    assert violations(p, "#[cfg(test)]\nfn helper() {}\nColor32::from_rgb(1,2,3)") == [
        (3, "Color32::from_rgb")
    ]
    assert violations(p, "#[cfg(test)]\nmod tests { fn helper() { Color32::from_gray(4); } }\nColor32::from_rgb(1,2,3)") == [
        (3, "Color32::from_rgb")
    ]
    assert violations(p, '/* outer /* nested */ Color32::from_gray(4) */\n"Color32::from_gray(4)"') == []
    assert violations(p, 'r##"#[cfg(test)] mod tests { }"##;\nColor32::from_gray(4)') == [
        (2, "Color32::from_gray")
    ]
    assert violations(p, "#[cfg(test)] fn check() { let brace = '}'; }\nColor32::from_gray(4)") == [
        (2, "Color32::from_gray")
    ]
    assert violations(
        LIBRARY + "kit/paint.rs",
        "fn draw(painter: &Painter, rect: Rect) {\npainter.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);\n}",
    ) == [(2, "Color32::WHITE")]
    assert violations(
        LIBRARY + "kit/paint.rs",
        "fn white_rectangle(rect: Rect, radius: CornerRadius) -> RectShape {\n    RectShape::filled(rect, radius, Color32::WHITE)\n}",
    ) == [(2, "Color32::WHITE")]
    for (name, constant), cases in ALLOWED_CONSTANTS.items():
        for expression, reason in cases:
            assert violations(name, expression) == [], reason
    print("UI colour guard self-test passed")
    return 0

if __name__ == "__main__":
    sys.exit(selftest() if sys.argv[1:] == ["--selftest"] else check())
