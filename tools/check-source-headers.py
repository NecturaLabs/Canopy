#!/usr/bin/env python3
"""Require an explanatory header comment in each maintained Rust source file.

Covers workspace crate sources and integration tests; excludes generated build
output and vendored dependencies. Standard library only.
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def is_source(name: str) -> bool:
    parts = Path(name).parts
    return (
        len(parts) >= 4
        and parts[0] == "crates"
        and parts[2] in {"src", "tests"}
        and name.endswith(".rs")
    )


def has_header(content: str) -> bool:
    # Crate-level inner attributes can precede the module documentation.
    in_attribute = False
    for line in content.lstrip("\ufeff").splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        if stripped.startswith("//") or stripped.startswith("/*"):
            return True
        if stripped.startswith("#!["):
            in_attribute = not stripped.endswith("]")
            continue
        if in_attribute:
            if stripped.endswith("]"):
                in_attribute = False
            continue
        return False
    return False


def check() -> int:
    files = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "crates"],
        cwd=ROOT, check=True, stdout=subprocess.PIPE,
    ).stdout.decode("utf-8").split("\0")
    missing = [
        name for name in sorted(set(files))
        if is_source(name)
        and (ROOT / name).is_file()
        and not has_header((ROOT / name).read_text(encoding="utf-8"))
    ]
    for name in missing:
        print(f"{name}:1: add a source header comment explaining this module")
    return int(bool(missing))


def selftest() -> int:
    assert is_source("crates/canopy-app/src/ui/mod.rs")
    assert is_source("crates/canopy-core/tests/operations.rs")
    assert not is_source("vendor/example/src/mod.rs")
    assert not is_source("crates/canopy-core/target/build/output.rs")
    assert has_header("\n//! Module responsibility.\nfn main() {}")
    assert has_header("/* Module responsibility. */\nuse std::fs;")
    assert has_header("\ufeff// Module responsibility.\nmod foo;")
    assert has_header('#![expect(\n  unsafe_code,\n  reason = "reviewed"\n)]\n//! Audited renderer.\n')
    assert not has_header('#![expect(unsafe_code)]\nuse std::fs;\n// Too late.')
    assert not has_header("\nuse std::fs;\n// Too late.")
    assert not has_header("")
    print("Rust header guard self-test passed")
    return 0


if __name__ == "__main__":
    sys.exit(selftest() if sys.argv[1:] == ["--selftest"] else check())
