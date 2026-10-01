#!/usr/bin/env python3
"""Writes docs/legal/third-party-notices.txt: the licence texts of everything Canopy ships.

The file lists every crate compiled into the `canopy` binary on Linux, macOS or Windows (normal
dependencies only; build scripts, proc macros and test-only crates ship nothing) with its
version, licence expression and the licence and notice files from its source package, plus the
bundled fonts. Crates that share a licence text are grouped under it, so the file stays readable.

    tools/notices.py           regenerate the file
    tools/notices.py --check   exit 1 if the committed file is out of date (CI)

Needs the crates' sources (`cargo fetch --locked` downloads them) and Python 3.9 or newer.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "legal" / "third-party-notices.txt"
TARGETS = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
BINARY_PACKAGE = "canopy-app"
LICENCE_FILE = re.compile(r"^(licen[cs]e|copying|notice|copyright|unlicense|authors)", re.I)
# Standard texts (SPDX license-list-data) for crates that publish no licence file of their own.
STANDARD = ROOT / "tools" / "licenses"
PREFERRED = ["MIT", "Apache-2.0", "BSL-1.0"]
# Subfolders whose text files are licences of bundled assets (epaint_default_fonts' fonts).
ASSET_FOLDERS = {"fonts", "licenses", "LICENSES"}
FONTS = [
    ("Inter 4.1 by Rasmus Andersson", "crates/canopy-app/assets/fonts/OFL-Inter.txt"),
    ("JetBrains Mono by JetBrains", "crates/canopy-app/assets/fonts/OFL-JetBrainsMono.txt"),
]


def metadata(target: str) -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def shipped(meta: dict) -> set[str]:
    """Package ids linked into the binary: normal dependencies reachable from the app."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    packages = {p["id"]: p for p in meta["packages"]}
    start = next(p["id"] for p in meta["packages"] if p["name"] == BINARY_PACKAGE)
    seen: set[str] = set()
    todo = [start]
    while todo:
        pid = todo.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            kinds = {k["kind"] for k in dep["dep_kinds"]}
            if None in kinds and not is_proc_macro(packages[dep["pkg"]]):
                todo.append(dep["pkg"])
    members = set(meta["workspace_members"])
    return {pid for pid in seen if pid not in members}


def is_proc_macro(package: dict) -> bool:
    return any("proc-macro" in t["kind"] for t in package["targets"])


def licence_files(package: dict) -> list[Path]:
    folder = Path(package["manifest_path"]).parent
    files = sorted(
        (f for f in folder.iterdir() if f.is_file() and LICENCE_FILE.match(f.name)),
        key=lambda f: f.name.lower(),
    )
    for sub in folder.iterdir():
        if sub.is_dir() and sub.name in ASSET_FOLDERS:
            files += sorted(f for f in sub.iterdir() if f.is_file() and f.suffix == ".txt")
    named = package.get("license_file")
    if named:
        extra = (folder / named).resolve()
        if extra.is_file() and extra not in files:
            files.append(extra)
    return files


def chosen(package: dict) -> str | None:
    """The licence Canopy takes a choice-of-licence crate under (`MIT OR Apache-2.0` → MIT)."""
    expression = (package.get("license") or "").replace("/", " OR ")
    if " AND " in expression or "(" in expression or " WITH " in expression:
        return None
    options = [o.strip() for o in expression.split(" OR ")]
    return next((spdx for spdx in PREFERRED if spdx in options), None)


def authors(package: dict) -> str:
    names = [re.sub(r"\s*<[^>]*>", "", a).strip() for a in package.get("authors") or []]
    return ", ".join(n for n in names if n) or f"the {package['name']} developers"


def words(text: str) -> str:
    return " ".join(text.split()).lower()


def without_copyright(text: str) -> str:
    return "\n".join(
        line for line in text.splitlines()
        if not re.match(r"\s*(copyright\b|\(c\)|©|mit license\s*$|the mit license)", line, re.I)
    )


MIT_BODY = None


def is_standard_mit(text: str) -> bool:
    global MIT_BODY
    if MIT_BODY is None:
        MIT_BODY = words(without_copyright(read(STANDARD / "MIT.txt")))
    return words(without_copyright(text)) == MIT_BODY


APACHE_BODY = None


def is_standard_apache(text: str) -> bool:
    """The Apache-2.0 text and nothing more (an appendix with a filled-in notice is allowed)."""
    global APACHE_BODY
    if APACHE_BODY is None:
        APACHE_BODY = words(read(STANDARD / "Apache-2.0.txt"))
    w = words(text)
    terms = w.split("end of terms and conditions")[0]
    standard_terms = APACHE_BODY.split("end of terms and conditions")[0]
    start = terms.find("terms and conditions for use")
    return (
        start != -1
        and terms[start:] == standard_terms[standard_terms.find("terms and conditions for use"):]
        and len(w) < len(APACHE_BODY) + 400
    )


def copyright_lines(text: str) -> list[str]:
    return [
        line.strip() for line in text.splitlines()
        if re.match(r"\s*(copyright\b|\(c\)|©)", line, re.I) and "<" not in line
    ]


def read(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="replace").replace("\r\n", "\n")
    return text.strip() + "\n"


def render() -> str:
    packages: dict[str, dict] = {}
    for target in TARGETS:
        meta = metadata(target)
        by_id = {p["id"]: p for p in meta["packages"]}
        for pid in shipped(meta):
            packages[pid] = by_id[pid]

    mit: list[str] = []  # crates under the standard MIT text, with their copyright notices
    apache: list[str] = []  # crates under the standard Apache-2.0 text
    groups: dict[str, list[str]] = {}  # any other text, with the crates it covers
    missing = []
    for package in sorted(packages.values(), key=lambda p: (p["name"], p["version"])):
        label = f"{package['name']} {package['version']} ({package.get('license') or 'see text'})"
        choice = chosen(package)
        files = licence_files(package)
        if choice and " OR " in (package.get("license") or "").replace("/", " OR "):
            label += f", used under {choice}"
        if choice:
            # Of a choice of licences only the chosen one's file is needed.
            picked = [f for f in files if choice.split("-")[0].lower() in f.name.lower()]
            files = picked or [f for f in files if "apache" not in f.name.lower()
                                and "mit" not in f.name.lower()]
        own = []
        for f in files:
            text = read(f)
            if is_standard_mit(text):
                notices = copyright_lines(text) or [f"Copyright (c) {authors(package)}"]
                mit.append(f"{label}: {'; '.join(notices)}")
            elif is_standard_apache(text):
                apache.append(label)
            else:
                own.append(f"--- {f.name} ---\n\n{text}")
        if own:
            groups.setdefault("\n".join(own), []).append(label)
        elif not files:
            if choice == "MIT":
                mit.append(f"{label}: Copyright (c) {authors(package)}")
            elif choice == "Apache-2.0":
                apache.append(label)
            elif choice:
                text = read(STANDARD / f"{choice}.txt")
                groups.setdefault(text, []).append(f"{label}; copyright {authors(package)}")
            else:
                missing.append(label)

    if missing:
        sys.exit(
            "No licence file found for:\n  " + "\n  ".join(missing)
            + "\nAdd its licence's standard text to tools/licenses/ and PREFERRED."
        )

    rule = "=" * 78
    out = [
        "Third-party software in Canopy",
        "==============================",
        "",
        "Canopy includes the open-source components and fonts listed below. Each is",
        "used under its own licence, reproduced here as published by its authors.",
        "These licences apply to those components only; Canopy itself is licensed",
        "under the Terms of Service. Where a component offers a choice of licences,",
        "the one Canopy uses is named.",
        "",
        "This file is generated by tools/notices.py.",
        "",
        "",
        "Fonts",
        "-----",
        "",
    ]
    for name, path in FONTS:
        out += [rule, name, rule, "", read(ROOT / path), ""]
    out += ["", "Rust crates", "-----------", ""]
    standard_mit = read(STANDARD / "MIT.txt").replace(
        "Copyright (c) <year> <copyright holders>", "Copyright (c) <the copyright holders above>"
    )
    out += [rule, "MIT License: these crates, with their copyright notices", rule, "", *mit, "",
            standard_mit, ""]
    out += [rule, "Apache License 2.0: these crates", rule, "", *apache, "",
            read(STANDARD / "Apache-2.0.txt"), ""]
    for text, labels in sorted(groups.items(), key=lambda g: g[1][0]):
        out += [rule, *labels, rule, "", text, ""]
    return "\n".join(out).rstrip() + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="fail if the file is out of date")
    args = parser.parse_args()
    text = render()
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current != text:
            sys.exit(f"{OUT.relative_to(ROOT)} is out of date: run tools/notices.py")
        print(f"{OUT.relative_to(ROOT)} is up to date")
        return
    OUT.write_text(text, encoding="utf-8")
    print(OUT.relative_to(ROOT))


if __name__ == "__main__":
    main()
