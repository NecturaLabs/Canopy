#!/usr/bin/env python3
"""Release automation for Canopy, driven by Conventional Commits.

Usage:
  tools/release.py plan [--bump auto|patch|minor|major] [--pre beta]
                                                         next version, or nothing to release
  tools/release.py apply <version> [--date YYYY-MM-DD]    bump files and write the changelog
  tools/release.py notes <version>                        that version's CHANGELOG.md section
  tools/release.py selftest                               run this script's tests

`plan` looks at the commits since the last `v*` tag (before the first tag: since the last change
to the workspace version):

  feat                         minor
  fix, perf, revert, build(deps)  patch
  a `!` after the type, or a `BREAKING CHANGE:` footer
                               major (minor while the version is 0.x)
  anything else (docs, ci, chore, test, refactor, style, build)  no release

`apply` sets the workspace version in Cargo.toml and the workspace crates in Cargo.lock, turns
CHANGELOG.md's [Unreleased] section into the new version's section (generating it from the
commits when [Unreleased] is empty), updates the comparison links, and adds the release to the
AppStream metainfo. The "Create release" workflow (.github/workflows/create-release.yml) runs plan
and apply, tags the result and hands it to the Release workflow, which builds and publishes it.
"""

from __future__ import annotations

import datetime as _dt
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO_URL = "https://github.com/NecturaLabs/Canopy"
WORKSPACE_CRATES = ("canopy-app", "canopy-core", "canopy-hosting")
METAINFO = ROOT / "packaging/linux/io.necturalabs.Canopy.metainfo.xml"

SUBJECT = re.compile(r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]*)\))?(?P<bang>!)?: (?P<desc>.+)$")
VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?$")


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout


def tags() -> list[str]:
    """`v*` version tags reachable from HEAD, newest version first."""
    out = git("tag", "--list", "v[0-9]*", "--merged", "HEAD", "--sort=-v:refname")
    return [t for t in out.split() if VERSION.match(t[1:])]


def last_tag() -> str | None:
    """The newest stable (non-pre-release) tag reachable from HEAD."""
    return next((t for t in tags() if "-" not in t), None)


def baseline() -> str | None:
    """Where the current version starts: the last `v*` tag, else (before the first tag) the last
    commit that changed the workspace version in Cargo.toml."""
    tag = last_tag()
    if tag:
        return tag
    out = git("log", "-1", "--format=%H", "-G", r'^version = "', "--", "Cargo.toml").strip()
    return out or None


def commits_since(base: str | None) -> list[tuple[str, str]]:
    """(subject, body) of the commits after `base`, oldest first, merges skipped."""
    rng = f"{base}..HEAD" if base else "HEAD"
    out = git("log", "--no-merges", "--reverse", "--format=%s%x1f%b%x1e", rng)
    result = []
    for record in out.split("\x1e"):
        record = record.strip("\n")
        if not record:
            continue
        subject, _, body = record.partition("\x1f")
        result.append((subject.strip(), body.strip()))
    return result


def classify(subject: str, body: str) -> str | None:
    """'major', 'minor', 'patch' or None for one commit."""
    m = SUBJECT.match(subject)
    if not m:
        return None
    kind, scope = m["type"], m["scope"] or ""
    if m["bang"] or re.search(r"^BREAKING[ -]CHANGE: ", body, re.M):
        return "major"
    if kind == "feat":
        return "minor"
    if kind in ("fix", "perf", "revert") or (kind == "build" and scope == "deps"):
        return "patch"
    return None


def bump(version: str, level: str) -> str:
    m = VERSION.match(version)
    if not m:
        raise SystemExit(f"not a version: {version}")
    major, minor, patch = (int(x) for x in m.groups())
    if level == "major" and major == 0:
        level = "minor"  # 0.x: breaking changes bump the minor version
    if level == "major":
        return f"{major + 1}.0.0"
    if level == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def level_of(commits: list[tuple[str, str]]) -> str | None:
    order = {"patch": 1, "minor": 2, "major": 3}
    best = None
    for subject, body in commits:
        lvl = classify(subject, body)
        if lvl and (best is None or order[lvl] > order[best]):
            best = lvl
    return best


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text()
    section = text.split("[workspace.package]", 1)[1]
    m = re.search(r'^version = "([^"]+)"', section, re.M)
    if not m:
        raise SystemExit("no [workspace.package] version in Cargo.toml")
    return m.group(1)


def plan(forced: str, pre: str | None = None) -> str | None:
    """The next version: from the last stable release, bumped as the commits (or `forced`) say.
    With `pre` (e.g. "beta"), a pre-release of that version: X.Y.Z-beta.N, N one past the
    newest existing one."""
    tag = last_tag()
    commits = commits_since(baseline())
    level = level_of(commits) if forced == "auto" else forced
    if level is None:
        return None
    base = tag[1:] if tag else workspace_version()
    version = bump(base, level)
    if pre:
        return next_prerelease(version, pre, tags())
    return version


def next_prerelease(version: str, pre: str, existing: list[str]) -> str:
    prefix = f"v{version}-{pre}."
    numbers = [int(t[len(prefix):]) for t in existing if t.startswith(prefix) and t[len(prefix):].isdigit()]
    return f"{version}-{pre}.{max(numbers, default=0) + 1}"


GROUPS = (
    ("Added", ("feat",)),
    ("Fixed", ("fix", "revert")),
    ("Changed", ("perf", "build")),
)


def generated_notes(commits: list[tuple[str, str]]) -> str:
    """Keep a Changelog sections built from commit subjects."""
    sections = []
    for title, kinds in GROUPS:
        items = []
        for subject, body in commits:
            m = SUBJECT.match(subject)
            if not m or classify(subject, body) is None or m["type"] not in kinds:
                continue
            desc = m["desc"].strip().rstrip(".")
            items.append(f"- {desc[:1].upper()}{desc[1:]}.")
        if items:
            sections.append(f"### {title}\n\n" + "\n".join(items))
    return "\n\n".join(sections)


def update_changelog(text: str, version: str, date: str, fallback: str, previous: str | None) -> str:
    head, sep, rest = text.partition("## [Unreleased]")
    if not sep:
        raise SystemExit("CHANGELOG.md has no [Unreleased] section")
    m = re.search(r"^## \[|^\[[^\]]+\]: ", rest, re.M)
    body, tail = (rest[: m.start()], rest[m.start():]) if m else (rest, "")
    notes = body.strip() or fallback.strip() or "### Changed\n\n- Maintenance release."
    # A pre-release copies the notes; the stable release that follows takes them over.
    kept = f"{body.strip()}\n\n" if "-" in version and body.strip() else ""
    new = f"{head}## [Unreleased]\n\n{kept}## [{version}] - {date}\n\n{notes}\n\n{tail.lstrip()}"
    # Comparison links.
    link_unreleased = f"[Unreleased]: {REPO_URL}/compare/v{version}...HEAD"
    link_version = (
        f"[{version}]: {REPO_URL}/compare/v{previous}...v{version}"
        if previous
        else f"[{version}]: {REPO_URL}/releases/tag/v{version}"
    )
    if re.search(r"^\[Unreleased\]: .*$", new, re.M):
        new = re.sub(
            r"^\[Unreleased\]: .*$", f"{link_unreleased}\n{link_version}", new, count=1, flags=re.M
        )
    else:
        new = new.rstrip("\n") + f"\n\n{link_unreleased}\n{link_version}\n"
    return new


def section(text: str, version: str) -> str:
    lines = text.splitlines()
    out, inside = [], False
    for line in lines:
        if line.startswith(f"## [{version}]"):
            inside = True
            continue
        if inside and (line.startswith("## [") or re.match(r"^\[[^\]]+\]: ", line)):
            break
        if inside:
            out.append(line)
    return "\n".join(out).strip()


def plain_summary(notes: str) -> str:
    items = [l[2:].strip() for l in notes.splitlines() if l.startswith("- ")]
    text = " ".join(items[:4]) if items else "Maintenance release."
    text = re.sub(r"[`*]", "", text)
    if len(text) > 400:
        text = text[:397].rsplit(" ", 1)[0] + "…"
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def apply(version: str, date: str) -> None:
    if not VERSION.match(version):
        raise SystemExit(f"not a version: {version}")
    previous_tag = last_tag()
    previous = previous_tag[1:] if previous_tag else None
    # Cargo.toml
    cargo = ROOT / "Cargo.toml"
    text = cargo.read_text()
    head, sep, rest = text.partition("[workspace.package]")
    rest = re.sub(r'^version = "[^"]+"', f'version = "{version}"', rest, count=1, flags=re.M)
    cargo.write_text(head + sep + rest)
    # Cargo.lock: the workspace crates only.
    lock = ROOT / "Cargo.lock"
    if lock.exists():
        lt = lock.read_text()
        for name in WORKSPACE_CRATES:
            lt = re.sub(
                rf'(\[\[package\]\]\nname = "{re.escape(name)}"\nversion = )"[^"]+"',
                rf'\1"{version}"',
                lt,
            )
        lock.write_text(lt)
    # CHANGELOG.md
    changelog = ROOT / "CHANGELOG.md"
    fallback = generated_notes(commits_since(baseline()))
    new_log = update_changelog(changelog.read_text(), version, date, fallback, previous)
    changelog.write_text(new_log)
    # AppStream metainfo
    if METAINFO.exists():
        xml = METAINFO.read_text()
        if f'<release version="{version}"' not in xml:
            summary = plain_summary(section(new_log, version))
            kind = ' type="development"' if "-" in version else ""
            entry = (
                f'    <release version="{version}" date="{date}"{kind}>\n'
                f"      <description>\n        <p>{summary}</p>\n      </description>\n"
                f"    </release>\n"
            )
            xml = xml.replace("<releases>\n", "<releases>\n" + entry, 1)
            METAINFO.write_text(xml)


def selftest() -> None:
    assert classify("feat(app): x", "") == "minor"
    assert classify("fix: x", "") == "patch"
    assert classify("build(deps): bump egui", "") == "patch"
    assert classify("build: tweak", "") is None
    assert classify("docs: x", "") is None
    assert classify("feat!: drop y", "") == "major"
    assert classify("refactor: z", "BREAKING CHANGE: gone") == "major"
    assert classify("Merge branch 'x'", "") is None
    assert classify("chore(release): v1.2.3", "") is None
    assert bump("0.1.0", "major") == "0.2.0"
    assert bump("1.4.2", "major") == "2.0.0"
    assert bump("1.4.2", "minor") == "1.5.0"
    assert bump("1.4.2", "patch") == "1.4.3"
    assert bump("1.5.0-rc.1", "patch") == "1.5.1"
    assert level_of([("docs: a", ""), ("fix: b", ""), ("feat: c", "")]) == "minor"
    assert next_prerelease("0.3.0", "beta", []) == "0.3.0-beta.1"
    assert next_prerelease("0.3.0", "beta", ["v0.3.0-beta.2", "v0.3.0-beta.10", "v0.2.0-beta.7"]) == "0.3.0-beta.11"
    assert level_of([("ci: a", ""), ("chore: b", "")]) is None
    notes = generated_notes([("feat(app): side by side diffs", ""), ("fix: crash on empty repo.", ""), ("docs: x", "")])
    assert notes == "### Added\n\n- Side by side diffs.\n\n### Fixed\n\n- Crash on empty repo.", notes
    log = (
        "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- New thing.\n\n"
        "## [0.1.0] - 2026-09-01\n\n- First.\n\n"
        "[Unreleased]: https://x/compare/v0.1.0...HEAD\n[0.1.0]: https://x/releases/tag/v0.1.0\n"
    )
    out = update_changelog(log, "0.2.0", "2026-10-01", "### Fixed\n\n- Ignored.", "0.1.0")
    assert "## [Unreleased]\n\n## [0.2.0] - 2026-10-01\n\n### Added\n\n- New thing.\n\n## [0.1.0]" in out, out
    assert "Ignored" not in out
    assert f"[Unreleased]: {REPO_URL}/compare/v0.2.0...HEAD\n[0.2.0]: {REPO_URL}/compare/v0.1.0...v0.2.0" in out, out
    assert section(out, "0.2.0") == "### Added\n\n- New thing."
    empty = update_changelog(log.replace("### Added\n\n- New thing.\n\n", ""), "0.2.0", "2026-10-01", "### Fixed\n\n- Generated.", "0.1.0")
    assert section(empty, "0.2.0") == "### Fixed\n\n- Generated."
    beta = update_changelog(log, "0.2.0-beta.1", "2026-10-01", "", "0.1.0")
    assert section(beta, "Unreleased") == "### Added\n\n- New thing.", beta
    assert section(beta, "0.2.0-beta.1") == "### Added\n\n- New thing."
    assert plain_summary("### Added\n\n- A `b`.\n- C & D.") == "A b. C &amp; D."
    print("release.py: all tests passed")


def main(argv: list[str]) -> None:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return
    cmd, args = argv[0], argv[1:]
    if cmd == "plan":
        forced, pre = "auto", None
        it = iter(args)
        for a in it:
            if a == "--bump":
                forced = next(it, "")
            elif a == "--pre":
                pre = next(it, "") or None
            else:
                raise SystemExit(f"unknown argument {a}")
        if forced not in ("auto", "patch", "minor", "major"):
            raise SystemExit(f"--bump takes auto, patch, minor or major, not {forced}")
        if pre is not None and not re.fullmatch(r"[a-z]+", pre):
            raise SystemExit(f"--pre takes a lower-case label such as beta or rc, not {pre}")
        v = plan(forced, pre)
        if v:
            print(v)
    elif cmd == "apply" and args:
        date = _dt.date.today().isoformat()
        if args[1:2] == ["--date"] and len(args) > 2:
            date = args[2]
        apply(args[0], date)
    elif cmd == "notes" and args:
        text = section((ROOT / "CHANGELOG.md").read_text(), args[0])
        if not text:
            raise SystemExit(f"no CHANGELOG.md section for {args[0]}")
        print(text)
    elif cmd == "selftest":
        selftest()
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
