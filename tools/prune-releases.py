#!/usr/bin/env python3
"""Keeps only the current Canopy releases, with every build's notes saved first.

Run by the Release workflow once a build is published, or by hand:

    prune-releases.py run --feed-dir DIR --keep BUILD [--dry-run]
    prune-releases.py selftest

`run` works in DIR, a clone of NecturaLabs/app-updates with push access. It first writes every
Canopy feed release's notes into `canopy/CHANGELOG.md` there (newest first, one section per
release, kept across runs) and pushes it. Then it deletes every Canopy release in app-updates
(with its tag) and every `build-*` release in NecturaLabs/Canopy (keeping the tag, which
`tools/release.py` reads), except BUILD and the builds `canopy/latest.json` and `canopy/beta.json`
offer, so a running copy can always download the build it is offered. A feed release whose notes
are not in the changelog is never deleted.

`gh` does the GitHub calls. CANOPY_TOKEN and FEED_TOKEN, when set, are the tokens for the Canopy
and app-updates repositories; otherwise `gh`'s own sign-in is used.
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CANOPY = "NecturaLabs/Canopy"
FEED = "NecturaLabs/app-updates"
FEED_PREFIX = "canopy-"
CANOPY_PREFIX = "build-"
CHANGELOG = Path("canopy") / "CHANGELOG.md"
HEADER = """# Canopy release notes

The notes of every Canopy build, newest first. Only the current builds keep a release; this file
keeps the notes of the ones that were removed.
"""
MARKER = re.compile(r"^<!-- release: (\S+) (\S+) -->$", re.M)


def gh(args, token_env, capture=True):
    env = dict(os.environ)
    token = os.environ.get(token_env)
    if token:
        env["GH_TOKEN"] = token
    out = subprocess.run(["gh", *args], env=env, check=True, text=True, capture_output=capture)
    return out.stdout if capture else ""


def releases(repo, token_env):
    """Every published release of `repo`, as dicts with tag, name, published and body."""
    out = gh(
        [
            "api",
            "--paginate",
            f"repos/{repo}/releases?per_page=100",
            "--jq",
            # A draft never shipped: a failed run's leftover is neither kept as notes nor pruned.
            ".[] | select(.draft | not) | {tag: .tag_name, name: .name, published: (.published_at // .created_at), body: (.body // \"\")}",
        ],
        token_env,
    )
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def parse(text):
    """The sections of a changelog: tag -> (published, section text)."""
    sections = {}
    marks = list(MARKER.finditer(text))
    for i, m in enumerate(marks):
        end = marks[i + 1].start() if i + 1 < len(marks) else len(text)
        sections[m[1]] = (m[2], text[m.start() : end].rstrip() + "\n")
    return sections


def section(release):
    published = release["published"]
    day = datetime.datetime.fromisoformat(published.replace("Z", "+00:00")).strftime("%-d %B %Y")
    notes = release["body"].strip() or "_No notes._"
    name = release["name"] or release["tag"]
    return f"<!-- release: {release['tag']} {published} -->\n## {name} ({day})\n\n{notes}\n"


def merge(text, feed_releases):
    """The changelog `text` with a section for every release in `feed_releases` it lacks."""
    sections = parse(text)
    for r in feed_releases:
        if r["tag"] not in sections:
            sections[r["tag"]] = (r["published"], section(r))
    ordered = sorted(sections.values(), key=lambda s: s[0], reverse=True)
    return HEADER + "".join("\n" + body for _, body in ordered)


def offered(feed_dir):
    """The builds the channel manifests offer."""
    builds = set()
    for channel in ("latest", "beta"):
        path = Path(feed_dir) / "canopy" / f"{channel}.json"
        if path.is_file():
            version = str(json.loads(path.read_text()).get("version", ""))
            if version.isdigit():
                builds.add(int(version))
    return builds


def run(feed_dir, keep, dry_run):
    feed_dir = Path(feed_dir)
    kept = offered(feed_dir) | {keep}
    print(f"keeping builds {sorted(kept)}")
    feed = [r for r in releases(FEED, "FEED_TOKEN") if r["tag"].startswith(FEED_PREFIX)]
    path = feed_dir / CHANGELOG
    old = path.read_text() if path.is_file() else ""
    new = merge(old, feed)
    if new != old:
        if dry_run:
            print(f"would write {CHANGELOG} with {len(parse(new))} releases")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(new)
            git = ["git", "-C", str(feed_dir)]
            subprocess.run([*git, "add", "--", str(CHANGELOG)], check=True)
            subprocess.run([*git, "commit", "-q", "-m", "canopy: keep the notes of every build"], check=True)
            subprocess.run([*git, "push", "-q", "origin", "HEAD:main"], check=True)
            print(f"wrote {CHANGELOG} with {len(parse(new))} releases")
    saved = set(parse(new))
    for r in feed:
        if r["tag"] in {f"{FEED_PREFIX}build-{b}" for b in kept}:
            continue
        if r["tag"] not in saved:
            print(f"::warning::{r['tag']} is not in the changelog; not deleting it")
            continue
        print(f"{'would delete' if dry_run else 'deleting'} {FEED} release {r['tag']}")
        if not dry_run:
            gh(["release", "delete", r["tag"], "--repo", FEED, "--yes", "--cleanup-tag"], "FEED_TOKEN", capture=False)
    for r in releases(CANOPY, "CANOPY_TOKEN"):
        tag = r["tag"]
        if not tag.startswith(CANOPY_PREFIX) or tag in {f"{CANOPY_PREFIX}{b}" for b in kept}:
            continue
        print(f"{'would delete' if dry_run else 'deleting'} {CANOPY} release {tag} (the tag stays)")
        if not dry_run:
            gh(["release", "delete", tag, "--repo", CANOPY, "--yes"], "CANOPY_TOKEN", capture=False)


class Tests(unittest.TestCase):
    def rel(self, tag, published, body="- A fix."):
        return {"tag": tag, "name": f"Canopy {tag}", "published": published, "body": body}

    def test_merge_adds_missing_releases_newest_first(self):
        text = merge("", [self.rel("canopy-build-1", "2026-10-01T00:00:00Z"), self.rel("canopy-build-2", "2026-11-01T00:00:00Z")])
        self.assertTrue(text.startswith(HEADER))
        self.assertLess(text.index("canopy-build-2"), text.index("canopy-build-1"))
        self.assertIn("## Canopy canopy-build-2 (1 November 2026)", text)

    def test_merge_keeps_sections_of_deleted_releases(self):
        first = merge("", [self.rel("canopy-build-1", "2026-10-01T00:00:00Z", "- Old note.")])
        second = merge(first, [self.rel("canopy-build-3", "2026-12-01T00:00:00Z")])
        self.assertIn("- Old note.", second)
        self.assertEqual(list(parse(second)), ["canopy-build-3", "canopy-build-1"])
        self.assertEqual(merge(second, [self.rel("canopy-build-3", "2026-12-01T00:00:00Z")]), second)

    def test_empty_notes_say_so(self):
        self.assertIn("_No notes._", merge("", [self.rel("canopy-build-1", "2026-10-01T00:00:00Z", "  ")]))

    def test_offered_reads_numeric_versions_only(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "canopy").mkdir()
            (Path(d) / "canopy" / "latest.json").write_text('{"version": "1661"}')
            (Path(d) / "canopy" / "beta.json").write_text('{"version": "0.2.1-beta.1"}')
            self.assertEqual(offered(d), {1661})


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--feed-dir", required=True)
    r.add_argument("--keep", type=int, required=True)
    r.add_argument("--dry-run", action="store_true")
    sub.add_parser("selftest")
    a = p.parse_args()
    if a.cmd == "selftest":
        result = unittest.TextTestRunner(verbosity=1).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
        sys.exit(0 if result.wasSuccessful() else 1)
    run(a.feed_dir, a.keep, a.dry_run)


if __name__ == "__main__":
    main()
