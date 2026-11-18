#!/usr/bin/env python3
"""Check the landing site in site/ (or the folder given) before it is deployed.

Every file a page, stylesheet or script names must exist in site/, every address outside it must
be https and on a host the content security policy in site/netlify.toml allows or a link the
visitor follows, the scripts must parse, and the site's Privacy Policy and Terms of Service pages
must be the current render of docs/legal/. Exits non-zero with one line per problem.
--write-legal renders those two pages again from docs/legal/.
"""

from __future__ import annotations

import html
import re
import shutil
import subprocess
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit

ARGS = [a for a in sys.argv[1:] if a != "--write-legal"]
SITE = (
    Path(ARGS[0]) if ARGS else Path(__file__).resolve().parent.parent / "site"
).resolve()
LEGAL = SITE.parent / "docs" / "legal"
# Each page the site publishes from docs/legal/: (page, source, title).
LEGAL_PAGES = (
    ("privacy.html", "privacy-policy.md", "Privacy Policy"),
    ("terms.html", "terms-of-service.md", "Terms of Service"),
)
LEGAL_PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>{title}: Canopy</title>
<meta name="description" content="Canopy's {title}.">
<meta name="color-scheme" content="dark light">
<meta name="theme-color" content="#09090B" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#FBFBFC" media="(prefers-color-scheme: light)">
<link rel="icon" type="image/png" sizes="64x64" href="assets/icon-64.png">
<link rel="canonical" href="https://canopy.necturalabs.com/{page}">
<link rel="preload" href="fonts/InterDisplay-SemiBold.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="fonts/Inter-Regular.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="styles.css">
<script src="theme.js"></script>
</head>
<body>
<a class="skip" href="#main">Skip to content</a>

<header class="top">
  <div class="wrap top-inner">
    <a class="brand" href="./"><img src="assets/icon-64.png" width="26" height="26" alt=""><span translate="no">Canopy</span></a>
    <div class="top-end">
      <a class="btn btn-cta btn-sm" href="./#download">Download</a>
    </div>
  </div>
</header>

<main id="main" class="doc wrap">
<!-- Rendered from docs/legal/{source} by tools/check-site.py --write-legal. Edit that file, not this one. -->
<article class="doc-body">
{body}
</article>
</main>

<footer class="foot">
  <div class="wrap foot-inner">
    <div class="foot-brand">
      <a class="brand" href="./"><img src="assets/icon-64.png" width="22" height="22" alt=""><span translate="no">Canopy</span></a>
      <p>Canopy is open source under the MIT or Apache-2.0 licence and made by NecturaLabs.</p>
    </div>
    <nav class="foot-nav" aria-label="Footer">
      <div class="foot-col">
        <h2 class="foot-h">Canopy</h2>
        <a href="./">Home</a>
        <a href="./#download">Download</a>
        <a href="https://github.com/NecturaLabs/Canopy">Source code</a>
      </div>
      <div class="foot-col">
        <h2 class="foot-h">Legal</h2>
        <a href="privacy.html">Privacy Policy</a>
        <a href="terms.html">Terms of Service</a>
      </div>
    </nav>
  </div>
</footer>
</body>
</html>
"""


def inline(text: str) -> str:
    """Escape a line of the legal text and link its web addresses and email addresses."""
    out = []
    for part in re.split(r"(https://[^\s,;)]*[^\s,;.)]|[\w.+-]+@[\w-]+\.[\w.-]*\w)", text):
        if part.startswith("https://"):
            out.append(f'<a href="{html.escape(part)}">{html.escape(part)}</a>')
        elif "@" in part and " " not in part:
            out.append(f'<a href="mailto:{html.escape(part)}">{html.escape(part)}</a>')
        else:
            out.append(html.escape(part, quote=False))
    return "".join(out)


def render_legal(markdown: str) -> str:
    """The legal documents use headings, paragraphs and flat lists only, one block per blank line."""
    blocks = []
    for block in re.split(r"\n\s*\n", markdown.strip()):
        lines = block.splitlines()
        if block.startswith("# "):
            blocks.append(f"<h1>{inline(block[2:].strip())}</h1>")
        elif block.startswith("## "):
            title = block[3:].strip()
            anchor = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
            blocks.append(f'<h2 id="{anchor}">{inline(title)}</h2>')
        elif all(line.startswith("- ") for line in lines):
            items = "\n".join(f"  <li>{inline(line[2:].strip())}</li>" for line in lines)
            blocks.append(f"<ul>\n{items}\n</ul>")
        elif block.startswith("Last updated:"):
            blocks.append(f'<p class="doc-updated">{inline(block)}</p>')
        else:
            blocks.append(f"<p>{inline(' '.join(lines))}</p>")
    return "\n".join(blocks)


def legal_pages() -> dict[Path, str]:
    """Each legal page the site should hold, rendered from docs/legal/; none without that folder."""
    if not LEGAL.is_dir():
        return {}
    return {
        SITE / page: LEGAL_PAGE.format(
            title=title,
            page=page,
            source=source,
            body=render_legal((LEGAL / source).read_text(encoding="utf-8")),
        )
        for page, source, title in LEGAL_PAGES
    }

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
    if "--write-legal" in sys.argv[1:]:
        for path, text in legal_pages().items():
            path.write_text(text, encoding="utf-8")
            print(f"check-site: wrote {path.relative_to(SITE.parent)}")
    problems: list[str] = []
    for path, text in legal_pages().items():
        rel = path.relative_to(SITE.parent)
        if not path.exists() or path.read_text(encoding="utf-8") != text:
            problems.append(
                f"{rel}: not the current render of docs/legal/; run python3 tools/check-site.py --write-legal"
            )
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
