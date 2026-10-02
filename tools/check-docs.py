#!/usr/bin/env python3
"""Checks that the maintained documentation still matches the repository.

Usage: tools/check-docs.py

Prints one `file:line: problem` per finding and exits 1 when there is any:

  - a file under docs/ that docs/README.md does not list, or an ADR that docs/adr/README.md does not
  - a path in backticks, or a relative link, in a document that is kept true with the code, that
    names nothing in the repository
  - a source module under crates/*/src that docs/architecture.md does not mention
  - a skill that differs between .claude/skills and its mirror .agents/skills
  - a file under tools/ that tools/README.md does not mention

It reads the files git knows about (tracked, or untracked and not ignored), so a new document
counts before it is committed. Standard library only. Part of tools/check.sh.
"""

from __future__ import annotations

import posixpath
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Documents kept true with the code. Records (docs/adr), research and handoffs (docs/specs), legal
# texts, the changelog and the process skills written for any repository are left out: they are
# point-in-time, dated or generic.
MAINTAINED_FILES = {
    "AGENTS.md",
    "CONTRIBUTING.md",
    "README.md",
    "tools/README.md",
    "packaging/README.md",
}
MAINTAINED_PATTERNS = (
    re.compile(r"docs/[^/]+\.md$"),
    re.compile(r"\.claude/agents/[^/]+\.md$"),
    re.compile(
        r"\.claude/skills/(add-git-operation|add-keybinding|add-theme|fixture-repo|hosting-api|release|screenshot)/.+\.md$"
    ),
)

# A backticked token that starts with one of these is a path from the repository root.
ROOT_PREFIXES = (
    "crates/",
    "docs/",
    "tools/",
    "packaging/",
    "assets/",
    ".github/",
    ".claude/",
    ".agents/",
    ".codex/",
)
# Any other token with a slash is checked only when it names a file of these kinds: directory names
# such as `logs/` or `target/` are too often something that exists only at run time.
SOURCE_EXTENSIONS = (".rs", ".md", ".toml", ".yml", ".sh", ".py", ".ps1", ".svg")
BARE_NAME = re.compile(r"^[A-Za-z0-9_.-]+\.(rs|md|sh|py|yml|toml|ps1)$")

# Named in the documents but not in this repository: written at install time, made by a fixture
# script, or kept in the NecturaLabs/app-updates repository.
EXTERNAL_NAMES = {
    "uninstall.sh",
    "uninstall.ps1",
    "src/retry.rs",
    "canopy/install.sh",
    "canopy/install.ps1",
}

# A directory whose files are listed in a document of their own.
DELEGATED_DOCS = {"docs/adr": "docs/adr/README.md"}

NOT_A_MODULE = {"mod.rs", "lib.rs", "main.rs", "tests.rs"}

TOKEN = re.compile(r"`([^`\s]+)`")
LINK = re.compile(r"\]\(([^)\s]+)\)")
BRACES = re.compile(r"^(.*)\{([^{}]*)\}(.*)$")


def project_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return sorted(f for f in out.splitlines() if f and (ROOT / f).exists())


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def prose_lines(path: str):
    """(line number, text) of the lines outside fenced code blocks."""
    fenced = False
    for number, line in enumerate(read(path).splitlines(), 1):
        if line.lstrip().startswith("```"):
            fenced = not fenced
        elif not fenced:
            yield number, line


def expand(token: str) -> list[str]:
    """`a/{b,c}.rs` as `a/b.rs` and `a/c.rs`; any other token as itself."""
    m = BRACES.match(token)
    if not m:
        return [token]
    return [m[1] + part + m[3] for part in m[2].split(",")]


def main() -> int:
    files = project_files()
    file_set = set(files)
    dirs = {posixpath.dirname(f) for f in files}
    dirs |= {posixpath.dirname(d) for d in dirs}
    dirs.discard("")
    basenames = {posixpath.basename(f) for f in files}
    problems: list[str] = []

    def problem(path: str, line: int, message: str) -> None:
        problems.append(f"{path}:{line}: {message}")

    def exists(path: str) -> bool:
        return path in file_set or path in dirs

    # How the documents shorten a path inside a crate: `ui/theme.rs`, `tests/safety.rs`,
    # `canopy-core/src/cli.rs`. Every tail of a path below crates/.
    crate_paths: set[str] = set()
    for f in files:
        if f.startswith("crates/"):
            parts = f.split("/")[1:]
            for i in range(len(parts)):
                crate_paths.add("/".join(parts[i:]))

    # 1. docs/README.md lists every document under docs/; docs/adr/README.md lists the records.
    docs_map = read("docs/README.md")
    listed = {t for t in LINK.findall(docs_map) + TOKEN.findall(docs_map) if not t.startswith(("http", "#"))}
    for f in files:
        if not f.startswith("docs/") or f == "docs/README.md":
            continue
        rel = f[len("docs/"):]
        delegate = next((d for d in DELEGATED_DOCS if f.startswith(d + "/")), None)
        if delegate:
            index = DELEGATED_DOCS[delegate]
            if f != index and posixpath.basename(f) not in read(index):
                problem(index, 1, f"{f} is not listed")
        elif not (rel in listed or any(t.endswith("/") and rel.startswith(t) for t in listed)):
            problem("docs/README.md", 1, f"{f} is not listed")

    # 2. Paths and links in the documents that are kept true with the code.
    maintained = [f for f in files if f in MAINTAINED_FILES or any(p.match(f) for p in MAINTAINED_PATTERNS)]
    for doc in maintained:
        here = posixpath.dirname(doc)
        for number, line in prose_lines(doc):
            for target in LINK.findall(line):
                if target.startswith(("http", "mailto:", "#")):
                    continue
                if not exists(posixpath.normpath(posixpath.join(here, target.split("#")[0]))):
                    problem(doc, number, f"link to {target} names nothing")
            for written in TOKEN.findall(line):
                for token in expand(written):
                    if token in EXTERNAL_NAMES or token.startswith(("-", "~", "/", "http")):
                        continue
                    if any(c in token for c in "*{}<>$|()=:,;"):
                        continue
                    path = posixpath.normpath(token)
                    if token.startswith(("./", "../")):
                        if not exists(posixpath.normpath(posixpath.join(here, token))):
                            problem(doc, number, f"`{written}` names nothing")
                    elif token.startswith(ROOT_PREFIXES):
                        if not exists(path):
                            problem(doc, number, f"`{written}` names nothing")
                    elif "/" not in token:
                        if BARE_NAME.match(token) and token not in basenames:
                            problem(doc, number, f"`{written}` names no file")
                    elif token.endswith(SOURCE_EXTENSIONS):
                        if not (path in crate_paths or exists(posixpath.normpath(posixpath.join(here, token)))):
                            problem(doc, number, f"`{written}` names nothing")

    # 3. Every source module is in the architecture document.
    architecture = read("docs/architecture.md")
    written_paths = {t for w in TOKEN.findall(architecture) for t in expand(w)}
    modules = [f for f in files if re.match(r"crates/[^/]+/src/.+\.rs$", f)]
    base_count: dict[str, int] = {}
    for f in modules:
        base_count[posixpath.basename(f)] = base_count.get(posixpath.basename(f), 0) + 1
    for f in modules:
        rel = f.split("/src/", 1)[1]
        base = posixpath.basename(f)
        if base in NOT_A_MODULE:
            continue
        named = rel in written_paths or (base_count[base] == 1 and base in written_paths)
        if not named:
            problem("docs/architecture.md", 1, f"{f} is not mentioned")

    # 4. The two copies of the skills are identical.
    claude = {f[len(".claude/skills/"):] for f in files if f.startswith(".claude/skills/")}
    agents = {f[len(".agents/skills/"):] for f in files if f.startswith(".agents/skills/")}
    for rel in sorted(claude - agents):
        problem(f".agents/skills/{rel}", 1, "missing from the mirror of .claude/skills")
    for rel in sorted(agents - claude):
        problem(f".claude/skills/{rel}", 1, "missing, but .agents/skills has it")
    for rel in sorted(claude & agents):
        if (ROOT / ".claude/skills" / rel).read_bytes() != (ROOT / ".agents/skills" / rel).read_bytes():
            problem(f".agents/skills/{rel}", 1, f"differs from .claude/skills/{rel}")

    # 5. Every file under tools/ is in tools/README.md.
    tools_readme = read("tools/README.md")
    for f in files:
        if not f.startswith("tools/") or f == "tools/README.md":
            continue
        rel = f[len("tools/"):]
        top = rel.split("/")[0]
        if not (rel in tools_readme or posixpath.basename(f) in tools_readme or ("/" in rel and f"{top}/" in tools_readme)):
            problem("tools/README.md", 1, f"{f} is not mentioned")

    for line in problems:
        print(line)
    if problems:
        print(f"\ncheck-docs: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print("check-docs: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
