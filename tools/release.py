#!/usr/bin/env python3
"""Release automation for Canopy: build numbers, and release notes from Conventional Commits.

A release is a build, named by its number: `git rev-list --count` of its commit on `main`. The
release is the tag `build-<number>` on that commit; nothing is committed and no version is bumped.

Usage:
  tools/release.py plan [--force]       the build number to release from HEAD, or nothing to release
  tools/release.py number [REF]         the build number of REF (default HEAD)
  tools/release.py notes [REF]          the release notes of the build at REF (default HEAD)
  tools/release.py selftest             run this script's tests

`plan` looks at the commits since the last `build-*` tag (before the first tag: all of them) and
prints HEAD's build number when one of them changes what users get; with `--force` whenever HEAD
is not already a tagged build:

  feat, fix, perf, revert, build(deps)   release
  anything else (docs, ci, chore, test, refactor, style, build)  no release

`notes` writes the release notes: the commits since the last `build-*` tag, grouped as Keep a
Changelog sections: Security (a `security` scope or a `Security:` footer) and Breaking changes
(`!` or a `BREAKING CHANGE:` footer) first, then feat Added, fix and revert Fixed, perf and build
Changed. A breaking security fix is listed in both of the first two. Before the first
`build-*` tag it prints the Unreleased section of CHANGELOG.md instead, which holds the
hand-written notes of everything since the last release made with semantic versions. The "Create
release" workflow (.github/workflows/create-release.yml) runs `plan`, tags the result and hands it
to the Release workflow, which builds, publishes and updates the feed.
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TAG_PREFIX = "build-"

SUBJECT = re.compile(r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]*)\))?(?P<bang>!)?: (?P<desc>.+)$")
TAG = re.compile(r"^build-([1-9][0-9]{0,8})$")


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout


def number_of(ref: str = "HEAD") -> int:
    """The build number of `ref`: the commits reachable from it."""
    if git("rev-parse", "--is-shallow-repository").strip() == "true":
        raise SystemExit("this clone is shallow: its commit count is not the build number (fetch the full history)")
    return int(git("rev-list", "--count", ref).strip())


def build_tags(ref: str = "HEAD") -> list[tuple[int, str]]:
    """(number, tag) of the `build-*` tags reachable from `ref`, newest build first."""
    out = git("tag", "--list", f"{TAG_PREFIX}*", "--merged", ref)
    found = [(int(m[1]), t) for t in out.split() if (m := TAG.match(t))]
    return sorted(found, reverse=True)


def previous_tag(ref: str = "HEAD") -> str | None:
    """The newest build tag before `ref`'s own commit."""
    head = git("rev-parse", f"{ref}^{{commit}}").strip()
    for _, tag in build_tags(ref):
        if git("rev-parse", f"{tag}^{{commit}}").strip() != head:
            return tag
    return None


def tag_on(ref: str = "HEAD") -> str | None:
    """A build tag on `ref`'s own commit."""
    head = git("rev-parse", f"{ref}^{{commit}}").strip()
    return next((t for _, t in build_tags(ref) if git("rev-parse", f"{t}^{{commit}}").strip() == head), None)


def commits_since(base: str | None, ref: str = "HEAD") -> list[tuple[str, str, str, str]]:
    """(subject, body, author as `Name <email>`, hash) of the commits after `base` up to `ref`,
    oldest first, merges skipped."""
    rng = f"{base}..{ref}" if base else ref
    out = git("log", "--no-merges", "--reverse", "--format=%H%x1f%s%x1f%an <%ae>%x1f%b%x1e", rng)
    result = []
    for record in out.split("\x1e"):
        record = record.strip("\n")
        if not record:
            continue
        sha, subject, author, body = (record.split("\x1f", 3) + ["", "", ""])[:4]
        result.append((subject.strip(), body.strip(), author.strip(), sha.strip()))
    return result


# GitHub logins whose commits are not credited: the maintainers. Bots are never credited.
MAINTAINERS = ("nectura",)  # compared case-insensitively, as GitHub compares logins
NOREPLY = re.compile(r"^(?:\d+\+)?(?P<login>[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?(?:\[bot\])?)@users\.noreply\.github\.com$")
PERSON = re.compile(r"^(?P<name>.*?)\s*<(?P<email>[^>]*)>$")


def credits(author: str, body: str) -> list[str]:
    """Who to credit for a commit: its author and `Co-authored-by:` people, as `@login` when the
    address is GitHub's no-reply one, else by name (never the address); maintainers and bots left out."""
    people = [author] + re.findall(r"^Co-authored-by:\s*(.+)$", body, re.M | re.I)
    out = []
    for person in people:
        m = PERSON.match(person.strip())
        if not m:
            continue
        name, email = m["name"].strip(), m["email"].strip()
        login = n["login"] if (n := NOREPLY.match(email)) else None
        if login and (login.endswith("[bot]") or login.casefold() in MAINTAINERS):
            continue
        if not login and (not name or name.casefold() in MAINTAINERS or name.endswith("[bot]")):
            continue
        tag = f"@{login}" if login else name
        if tag not in out:
            out.append(tag)
    return out


def releasable(subject: str, body: str) -> bool:
    """Whether one commit changes what users get."""
    m = SUBJECT.match(subject)
    if not m:
        return False
    kind, scope = m["type"], m["scope"] or ""
    if m["bang"] or re.search(r"^BREAKING[ -]CHANGE: ", body, re.M):
        return True
    return kind in ("feat", "fix", "perf", "revert") or (kind == "build" and scope == "deps")


def plan(force: bool, ref: str = "HEAD") -> int | None:
    """The build number to release from `ref`, or None when there is nothing to release."""
    if tag_on(ref):
        return None
    commits = commits_since(previous_tag(ref), ref)
    if not force and not any(releasable(s, b) for s, b, *_ in commits):
        return None
    return number_of(ref)


GROUPS = (
    ("Added", ("feat",)),
    ("Fixed", ("fix", "revert")),
    ("Changed", ("perf", "build")),
)


def special_groups(m: re.Match, body: str) -> list[str]:
    """Security for a `security` scope or a `Security:` footer; breaking for `!` or the footer."""
    groups = []
    if m["scope"] == "security" or re.search(r"^Security: ", body, re.M):
        groups.append("Security")
    if m["bang"] or re.search(r"^BREAKING[ -]CHANGE: ", body, re.M):
        groups.append("Breaking changes")
    return groups


def generated_notes(commits: list[tuple[str, ...]]) -> str:
    """Keep a Changelog sections built from commit subjects, each line crediting its outside
    contributors; security fixes and breaking changes come first, in sections of their own."""
    groups: dict[str, list[str]] = {}
    for subject, body, *rest in commits:
        m = SUBJECT.match(subject)
        if not m or not releasable(subject, body):
            continue
        titles = special_groups(m, body) or [t for t, kinds in GROUPS if m["type"] in kinds]
        desc = m["desc"].strip().rstrip(".")
        people = credits(rest[0] if rest else "", body)
        by = f" by {' and '.join([', '.join(people[:-1]), people[-1]] if len(people) > 1 else people)}" if people else ""
        for title in titles:
            groups.setdefault(title, []).append(f"- {desc[:1].upper()}{desc[1:]}{by}.")
    order = ["Security", "Breaking changes"] + [t for t, _ in GROUPS]
    return "\n\n".join(f"### {t}\n\n" + "\n".join(groups[t]) for t in order if t in groups)


def pull_request(sha: str, ref: str) -> str | None:
    """The pull request that brought a commit in: a squash merge's `(#N)` or the first
    `Merge pull request #N` merge after it."""
    if m := re.search(r"\(#(\d+)\)$", git("log", "-1", "--format=%s", sha).strip()):
        return m[1]
    merges = git("log", "--merges", "--ancestry-path", "--reverse", "--format=%H%x1f%s", f"{sha}..{ref}")
    for line in merges.splitlines():
        merge, _, subject = line.partition("\x1f")
        # The merge that brought the commit in: on its second parent's side, not its first's.
        if is_ancestor(sha, f"{merge}^1") or not is_ancestor(sha, f"{merge}^2"):
            continue
        m = re.match(r"Merge pull request #(\d+) ", subject)
        return m[1] if m else None
    return None


def is_ancestor(a: str, b: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b], cwd=ROOT, capture_output=True).returncode == 0


def new_contributors(base: str | None, ref: str, commits: list[tuple[str, ...]]) -> str:
    """`@x made their first contribution in #N` for people credited for the first time."""
    seen: set[str] = set()
    if base:
        for subject, body, author, *_ in commits_since(None, base):
            seen.update(credits(author, body))
    lines = []
    for subject, body, author, sha in commits:
        if not releasable(subject, body):
            continue
        for person in credits(author, body):
            if person in seen:
                continue
            seen.add(person)
            pr = pull_request(sha, ref)
            lines.append(f"- {person} made their first contribution" + (f" in #{pr}." if pr else "."))
    return "### New contributors\n\n" + "\n".join(lines) if lines else ""


def section(text: str, name: str) -> str:
    """The body of `## [name]` in a Keep a Changelog file."""
    out, inside = [], False
    for line in text.splitlines():
        if line.startswith(f"## [{name}]"):
            inside = True
            continue
        if inside and (line.startswith("## [") or re.match(r"^\[[^\]]+\]: ", line)):
            break
        if inside:
            out.append(line)
    return "\n".join(out).strip()


def notes(ref: str = "HEAD") -> str:
    """The release notes of the build at `ref`."""
    previous = previous_tag(ref)
    if previous:
        commits = commits_since(previous, ref)
        text = "\n\n".join(p for p in (generated_notes(commits), new_contributors(previous, ref, commits)) if p)
    else:
        changelog = git("show", f"{ref}:CHANGELOG.md")
        text = section(changelog, "Unreleased")
    return text or "### Changed\n\n- Maintenance build."


class SelftestFailure(Exception):
    """A selftest check failed. Raised explicitly: `assert` is stripped under `python -O`."""


def expect(condition: bool, what: str, detail: object = "") -> None:
    if not condition:
        raise SelftestFailure(f"expected {what}" + (f"\n{detail}" if detail != "" else ""))


def expect_eq(actual: object, expected: object) -> None:
    if actual != expected:
        raise SelftestFailure(f"expected {expected!r}, got {actual!r}")


def selftest() -> None:
    for subject, body, expected in [
        ("feat(app): x", "", True),
        ("fix: x", "", True),
        ("perf(graph): x", "", True),
        ("revert: x", "", True),
        ("build(deps): bump egui", "", True),
        ("build: tweak", "", False),
        ("docs: x", "", False),
        ("ci: x", "", False),
        ("chore: x", "", False),
        ("feat!: drop y", "", True),
        ("refactor: z", "BREAKING CHANGE: gone", True),
        ("Merge branch 'x'", "", False),
    ]:
        expect_eq(releasable(subject, body), expected)
    notes_text = generated_notes([("feat(app): side by side diffs", ""), ("fix: crash on empty repo.", ""), ("docs: x", ""), ("build: y", "")])
    expect_eq(notes_text, "### Added\n\n- Side by side diffs.\n\n### Fixed\n\n- Crash on empty repo.")
    expect_eq(generated_notes([("docs: x", "")]), "")
    # Security fixes and breaking changes are named in sections of their own, ahead of the rest.
    expect_eq(
        generated_notes([
            ("feat: new thing", ""),
            ("fix(security): token in a log line", ""),
            ("fix: escape a path", "Security: GHSA-xxxx-xxxx-xxxx"),
            ("feat!: drop the old flag", ""),
            ("refactor: read settings in the new format", "BREAKING CHANGE: old files are not read"),
            ("fix(security)!: refuse unsigned updates", ""),
        ]),
        "### Security\n\n- Token in a log line.\n- Escape a path.\n- Refuse unsigned updates.\n\n"
        "### Breaking changes\n\n- Drop the old flag.\n- Read settings in the new format.\n"
        "- Refuse unsigned updates.\n\n"
        "### Added\n\n- New thing.",
    )
    # Outside contributors are credited on their lines; maintainers and bots are not.
    expect_eq(
        generated_notes([
            ("feat(appearance): match the system's mode", "", "Billy M <142944908+billy1204@users.noreply.github.com>"),
            ("fix: a crash", "", "Nectura <45967738+Nectura@users.noreply.github.com>"),
            ("build(deps): bump egui", "", "dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>"),
            ("fix: a race", "Co-authored-by: Ada <ada@example.invalid>\nCo-authored-by: Nectura <45967738+Nectura@users.noreply.github.com>", "Nectura <45967738+Nectura@users.noreply.github.com>"),
            ("perf: faster graph", "", "Lin <lin@users.noreply.github.com>"),
            ("fix: lower-case maintainer", "", "N <nectura@users.noreply.github.com>"),
        ]),
        "### Added\n\n- Match the system's mode by @billy1204.\n\n### Fixed\n\n- A crash.\n- A race by Ada.\n- Lower-case maintainer.\n\n"
        "### Changed\n\n- Bump egui.\n- Faster graph by @lin.",
    )
    log = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- New thing.\n\n## [0.1.0] - 2026-09-01\n\n- First.\n\n[0.1.0]: https://x\n"
    expect_eq(section(log, "Unreleased"), "### Added\n\n- New thing.")
    expect_eq(section(log, "0.1.0"), "- First.")
    expect_eq(section(log, "9.9.9"), "")
    expect_eq(TAG.match("build-412") is not None, True)
    for bad in ("build-0", "build-0412", "build-4x", "v1.2.3", "build-", "build-1234567890", "build-412-beta"):
        expect(TAG.match(bad) is None, f"{bad} is not a build tag")
    repo_selftest(log)
    print("release.py: all tests passed")


def repo_selftest(changelog: str) -> None:
    """plan, notes and the tags, against a temporary repository."""
    global ROOT
    saved = ROOT
    with tempfile.TemporaryDirectory() as tmp:
        ROOT = Path(tmp)
        try:
            def commit(subject: str, text: str | None = None) -> None:
                (ROOT / "file.txt").write_text(f"{subject}\n")
                if text is not None:
                    (ROOT / "CHANGELOG.md").write_text(text)
                git("add", "-A")
                git("-c", "user.name=Nectura", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", "commit", "-q", "-m", subject)

            git("init", "-q", "-b", "main")
            commit("docs: start", changelog)
            expect_eq(plan(False), None)
            expect_eq(plan(True), 1)
            commit("feat: a thing")
            expect_eq(number_of(), 2)
            expect_eq(plan(False), 2)
            expect_eq(notes(), "### Added\n\n- New thing.")
            git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "tag", "-a", "build-2", "-m", "build 2")
            expect_eq(tag_on(), "build-2")
            expect_eq(plan(True), None)
            commit("docs: more docs")
            expect_eq(tag_on(), None)
            expect_eq(plan(False), None)
            expect_eq(plan(True), 3)
            expect_eq(notes(), "### Changed\n\n- Maintenance build.")
            commit("fix(core): a crash.")
            commit("chore: tidy")
            expect_eq(plan(False), 5)
            expect_eq(notes(), "### Fixed\n\n- A crash.")
            # Tags are ordered as numbers: build-10 is newer than build-9 (the names are arbitrary here).
            git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "tag", "-a", "build-9", "-m", "x", "HEAD~1")
            git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "tag", "-a", "build-10", "-m", "x", "HEAD")
            expect_eq([n for n, _ in build_tags()], [10, 9, 2])
            expect_eq(previous_tag(), "build-9")
            expect_eq(previous_tag("HEAD~1"), "build-2")
            # An outside contributor's pull request: credited on the line, and once as new, with
            # the pull request's number; a later contribution is not new.
            ident = ["-c", "user.name=Billy M", "-c", "user.email=142944908+billy1204@users.noreply.github.com", "-c", "commit.gpgsign=false"]
            git("checkout", "-q", "-b", "topic")
            (ROOT / "topic.txt").write_text("1\n")
            git("add", "-A")
            git(*ident, "commit", "-q", "-m", "feat(app): follow the system")
            git("checkout", "-q", "main")
            git("-c", "user.name=Nectura", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false",
                "merge", "-q", "--no-ff", "-m", "Merge pull request #7 from billy1204/topic", "topic")
            expect_eq(notes(), "### Added\n\n- Follow the system by @billy1204.\n\n"
                      "### New contributors\n\n- @billy1204 made their first contribution in #7.")
            git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "tag", "-a", "build-12", "-m", "x", "HEAD")
            (ROOT / "topic.txt").write_text("2\n")
            git("add", "-A")
            git(*ident, "commit", "-q", "-m", "fix: a glitch (#8)")
            expect_eq(notes(), "### Fixed\n\n- A glitch (#8) by @billy1204.")
            # A first-timer's direct commit is not credited to a later, unrelated pull request.
            git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "tag", "-a", "build-13", "-m", "x", "HEAD")
            git("-c", "user.name=Ada", "-c", "user.email=ada@example.invalid", "-c", "commit.gpgsign=false",
                "commit", "-q", "--allow-empty", "-m", "fix: a typo")
            git("checkout", "-q", "-b", "other")
            git("-c", "user.name=Nectura", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false",
                "commit", "-q", "--allow-empty", "-m", "docs: other")
            git("checkout", "-q", "main")
            git("-c", "user.name=Nectura", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false",
                "merge", "-q", "--no-ff", "-m", "Merge pull request #9 from x/other", "other")
            expect_eq(notes(), "### Fixed\n\n- A typo by Ada.\n\n"
                      "### New contributors\n\n- Ada made their first contribution.")
        finally:
            ROOT = saved


def main(argv: list[str]) -> None:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return
    cmd, args = argv[0], argv[1:]
    if cmd == "plan" and args in ([], ["--force"]):
        n = plan(bool(args))
        if n is not None:
            print(n)
    elif cmd == "number" and len(args) <= 1:
        print(number_of(*args))
    elif cmd == "notes" and len(args) <= 1:
        print(notes(*args))
    elif cmd == "selftest":
        selftest()
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
