#!/usr/bin/env python3
"""Check the landing site in site/ (or the folder given) before it is deployed.

Every file a page, stylesheet or script names must exist in site/, every address outside it must
be https and on a host the content security policy in site/netlify.toml allows or a link the
visitor follows, and the scripts must parse. Exits non-zero with one line per problem.
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit

SITE = (
    Path(sys.argv[1])
    if len(sys.argv) > 1
    else Path(__file__).resolve().parent.parent / "site"
).resolve()

# Attributes that load something into the page, and the policy directive that governs each;
# any other address is a link the visitor follows.
LOADS = {
    ("script", "src"): "script-src",
    ("link", "href"): "style-src",
    ("img", "src"): "img-src",
    ("img", "srcset"): "img-src",
    ("source", "src"): "media-src",
    ("source", "srcset"): "img-src",
    ("video", "src"): "media-src",
    ("video", "poster"): "img-src",
}


class Refs(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.found: list[
            tuple[str, str | None, int]
        ] = []  # (url, directive or None, line)

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        line = self.getpos()[0]
        values = {name: value for name, value in attrs if value is not None}
        if tag == "meta":
            # Link previews fetch og:image and twitter:image.
            if (
                values.get("property", values.get("name", "")).endswith(":image")
                and "content" in values
            ):
                self.found.append((values["content"], None, line))
            return
        rel = values.get("rel", "")
        for name, value in values.items():
            if name == "srcset":
                urls = [part.strip().split(" ")[0] for part in value.split(",")]
            elif name in ("src", "href", "poster"):
                urls = [value]
            else:
                continue
            directive = LOADS.get((tag, name))
            if tag == "link":
                directive = {
                    "stylesheet": "style-src",
                    "icon": "img-src",
                    "preload": "font-src",
                }.get(rel)
            for url in urls:
                self.found.append((url, directive, line))


def local_target(page: Path, url: str) -> Path | None:
    parts = urlsplit(url)
    if (
        parts.scheme
        or parts.netloc
        or not parts.path
        or url.startswith(("#", "mailto:", "data:"))
    ):
        return None
    base = SITE if parts.path.startswith("/") else page.parent
    return (base / parts.path.lstrip("/")).resolve()


def allowed_hosts() -> dict[str, set[str]]:
    """The hosts each fetch directive of the policy in netlify.toml allows."""
    toml = (SITE / "netlify.toml").read_text(encoding="utf-8")
    policy = re.search(r'Content-Security-Policy\s*=\s*"([^"]*)"', toml)
    hosts: dict[str, set[str]] = {}
    for directive in (policy.group(1) if policy else "").split(";"):
        words = directive.split()
        if words:
            hosts[words[0]] = {
                urlsplit(w).netloc for w in words[1:] if w.startswith("https://")
            }
    return hosts


def main() -> int:
    problems: list[str] = []
    pages = sorted(SITE.rglob("*.html"))
    if not pages:
        problems.append("site/: no HTML page")
    csp = allowed_hosts()
    if not csp:
        problems.append("site/netlify.toml: no Content-Security-Policy header")

    for page in pages:
        parser = Refs()
        parser.feed(page.read_text(encoding="utf-8"))
        rel = page.relative_to(SITE.parent)
        for url, directive, line in parser.found:
            parts = urlsplit(url)
            if parts.scheme == "http":
                problems.append(f"{rel}:{line}: plain http address {url}")
            elif (
                parts.scheme == "https"
                and directive is not None
                and parts.netloc
                not in csp.get(directive, csp.get("default-src", set()))
            ):
                problems.append(
                    f"{rel}:{line}: loads {url}, which the content security policy refuses"
                )
            target = local_target(page, url)
            if target is not None and not target.exists():
                problems.append(f"{rel}:{line}: {url} does not exist")

    for sheet in sorted(SITE.rglob("*.css")):
        text = sheet.read_text(encoding="utf-8")
        for match in re.finditer(r"url\(\s*['\"]?([^'\")]+)['\"]?\s*\)", text):
            url = match.group(1)
            line = text.count("\n", 0, match.start()) + 1
            rel = sheet.relative_to(SITE.parent)
            if urlsplit(url).scheme in ("http", "https"):
                problems.append(
                    f"{rel}:{line}: stylesheet loads {url} from outside the site"
                )
            target = local_target(sheet, url)
            if target is not None and not target.exists():
                problems.append(f"{rel}:{line}: {url} does not exist")

    node = shutil.which("node")
    for script in sorted(SITE.rglob("*.js")):
        rel = script.relative_to(SITE.parent)
        if node is None:
            print(f"check-site: node not found, {rel} not parsed", file=sys.stderr)
            continue
        result = subprocess.run(
            [node, "--check", str(script)], capture_output=True, text=True, check=False
        )
        if result.returncode != 0:
            error = next(
                (line for line in result.stderr.splitlines() if "Error" in line),
                result.stderr.strip(),
            )
            problems.append(f"{rel}: does not parse: {error}")

    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print(f"check-site: ok ({len(pages)} page{'s' if len(pages) != 1 else ''})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
