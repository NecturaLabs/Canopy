#!/usr/bin/env python3
"""Guard the theme boundary: runtime UI colours are defined in ui/theme.rs."""
from __future__ import annotations
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PREFIX = "crates/canopy-app/src/"
THEME = PREFIX + "ui/theme.rs"
CONSTRUCTOR = re.compile(r"\bColor32\s*::\s*from_(?:rgb\w*|gray|hex)\s*\(")
TEST_MODULE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")

def violations(name: str, source: str) -> list[tuple[int, str]]:
    if name == THEME or not name.startswith(PREFIX) or not name.endswith(".rs"):
        return []
    if "/tests/" in name or name.endswith("/tests.rs"):
        return []
    marker = TEST_MODULE.search(source)
    runtime = source[:marker.start()] if marker else source
    problems = []
    for m in CONSTRUCTOR.finditer(runtime):
        # Colour-space conversion is not a hard-coded design colour.
        if name == PREFIX + "ui/contrast.rs" and runtime[m.start():].startswith(
            "Color32::from_rgb(from_linear("
        ):
            continue
        problems.append((source.count("\n", 0, m.start()) + 1, m.group()[:-1].strip()))
    return problems

def check() -> int:
    out = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", PREFIX],
        cwd=ROOT, check=True, stdout=subprocess.PIPE,
    ).stdout
    failed = False
    for name in sorted(set(out.decode("utf-8").split("\0"))):
        path = ROOT / name
        if not name.endswith(".rs") or not path.is_file():
            continue
        for lineno, constructor in violations(name, path.read_text(encoding="utf-8")):
            print(f"{name}:{lineno}: {constructor} belongs in ui/theme.rs or a test")
            failed = True
    return int(failed)

def selftest() -> int:
    p = PREFIX + "views/synthetic.rs"
    assert violations(p, "Color32::from_rgb(1, 2, 3)") == [(1, "Color32::from_rgb")]
    assert violations(p, "\nColor32::from_hex(\"#fff\")") == [(2, "Color32::from_hex")]
    assert violations(p, "#[cfg(test)]\nColor32::from_gray(4)") == []
    assert violations(PREFIX + "ui/kit/glass/tests.rs", "Color32::from_rgb(1,2,3)") == []
    assert violations(THEME, "Color32::from_rgb(1,2,3)") == []
    assert violations(PREFIX + "ui/contrast.rs",
        "Color32::from_rgb(from_linear(r), from_linear(g), from_linear(b))") == []
    assert violations(PREFIX + "ui/contrast.rs", "Color32::from_rgb(1,2,3)") == [
        (1, "Color32::from_rgb")
    ]
    print("UI colour guard self-test passed")
    return 0

if __name__ == "__main__":
    sys.exit(selftest() if sys.argv[1:] == ["--selftest"] else check())
