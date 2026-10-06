#!/usr/bin/env python3
"""Usage: tools/mock-hosting.py [port]

A stand-in for the GitHub REST API with a few canned pull requests, so the pull request view can
be developed and screenshotted without a network or a token. Point Canopy at it with
CANOPY_HOSTING_API_BASE=http://127.0.0.1:<port> (development builds and --screenshot honour it).
Every repository gets the same pull requests. POST /graphql answers the request list's checks and
reviews (the rollup query) in GitHub's GraphQL shape; other POST/PUT/PATCH requests succeed and
are logged.
"""
import json
import re
import sys
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

NOW = datetime.now(timezone.utc)


def ago(**kw):
    return (NOW - timedelta(**kw)).strftime("%Y-%m-%dT%H:%M:%SZ")


def pull(number, title, user, head, base="main", draft=False, state="open", merged=None, labels=(), hours=1):
    repo = {"full_name": "NecturaLabs/demo"}
    return {
        "number": number, "title": title, "user": {"login": user}, "state": state,
        "draft": draft, "merged_at": merged,
        "head": {"ref": head, "sha": f"{number:040x}", "repo": repo},
        "base": {"ref": base, "sha": "0" * 40, "repo": repo},
        "created_at": ago(hours=hours + 30), "updated_at": ago(hours=hours),
        "html_url": f"https://github.com/NecturaLabs/demo/pull/{number}",
        "labels": [{"name": l} for l in labels],
        "body": None, "mergeable": True, "commits": 3, "changed_files": 7,
        "additions": 214, "deletions": 38,
    }


PULLS = [
    pull(42, "Add dark mode toggle to the header", "lucia-ortega", "feature/dark-mode",
         labels=("ui", "enhancement"), hours=2),
    pull(41, "Rate-limit login attempts", "jonas-weber", "feature/auth", labels=("security",), hours=5),
    pull(39, "Draft: graph layout for octopus merges", "priya-raman", "feature/graph", draft=True, hours=26),
    pull(37, "Fix typo in README", "sam-okafor", "fix/typo", hours=50),
]
# The list's checks and reviews per pull request, as GitHub's GraphQL API answers them: the check
# rollup's state, the branch rules' review decision (null without required reviews) and the
# reviewers' latest verdicts.
ROLLUP = {
    42: {"statusCheckRollup": {"state": "SUCCESS"}, "reviewDecision": "APPROVED",
         "latestOpinionatedReviews": {"nodes": [{"state": "APPROVED"}]}},
    41: {"statusCheckRollup": {"state": "PENDING"}, "reviewDecision": "REVIEW_REQUIRED",
         "latestOpinionatedReviews": {"nodes": []}},
    39: {"statusCheckRollup": None, "reviewDecision": None,
         "latestOpinionatedReviews": {"nodes": []}},
    37: {"statusCheckRollup": {"state": "FAILURE"}, "reviewDecision": None,
         "latestOpinionatedReviews": {"nodes": [{"state": "APPROVED"}, {"state": "CHANGES_REQUESTED"}]}},
}
BODY = """Adds a theme toggle to the header and remembers the choice.

### Changes
- New `ThemeToggle` component in the header
- Colour tokens moved to `tokens.css`
- Respects `prefers-color-scheme` on first visit

### Testing
- Manual check in Firefox and Chromium
- Unit tests for the token loader"""


class Handler(BaseHTTPRequestHandler):
    def reply(self, data, status=200):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        parts = path.strip("/").split("/")
        if len(parts) == 4 and parts[3] == "commits":
            sha = parse_qs(urlsplit(self.path).query).get("sha", [""])[0]
            return self.reply([{"sha": sha, "author": {"avatar_url": f"http://127.0.0.1:{self.server.server_port}/avatar.png"}}])
        if path == "/avatar.png":
            # A bundled PNG fixture; never fetch a real person's picture for the demo.
            body = (Path(__file__).resolve().parent.parent / "assets" / "icon-512.png").read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if path == "/user":
            return self.reply({"login": "maya-chen"})
        if len(parts) == 4 and parts[3] == "pulls":
            return self.reply(PULLS)
        if len(parts) == 5 and parts[3] == "pulls":
            p = next((p for p in PULLS if p["number"] == int(parts[4])), None)
            if not p:
                return self.reply({"message": "Not Found"}, 404)
            return self.reply(dict(p, body=BODY, mergeable=not p["draft"]))
        if parts[-1] == "reviews":
            return self.reply([{"user": {"login": "maya-chen"}, "state": "APPROVED"},
                               {"user": {"login": "sam-okafor"}, "state": "COMMENTED"}])
        if parts[-1] == "comments":
            return self.reply([
                {"user": {"login": "sam-okafor"}, "body": "Looks great. Could the toggle remember the choice per device?", "created_at": ago(hours=3)},
                {"user": {"login": "lucia-ortega"}, "body": "Done in the latest commit: it uses local storage.", "created_at": ago(hours=2)},
            ])
        if parts[-1] == "check-runs":
            return self.reply({"check_runs": [
                {"name": "build (ubuntu)", "status": "completed", "conclusion": "success", "html_url": "https://example.invalid/1"},
                {"name": "test (ubuntu)", "status": "completed", "conclusion": "success", "html_url": "https://example.invalid/2"},
                {"name": "lint", "status": "in_progress", "conclusion": None, "html_url": "https://example.invalid/3"},
            ]})
        if parts[-1] == "status":
            return self.reply({"statuses": []})
        return self.reply({"message": "Not Found"}, 404)

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0)).decode()
        self.log_message("%s", body)
        if self.path.split("?")[0] == "/graphql":
            return self.graphql(json.loads(body or "{}"))
        self.reply(dict(PULLS[0], number=43, title="Created from Canopy"), 201)

    def graphql(self, request):
        """The request list's rollup: `pN: pullRequest(number: N)` per listed pull request."""
        found, errors = {}, []
        for alias, number in re.findall(r"(p\d+): pullRequest\(number: (\d+)\)", request.get("query", "")):
            number = int(number)
            if number in ROLLUP:
                found[alias] = dict(ROLLUP[number], number=number)
            else:
                found[alias] = None
                errors.append({"type": "NOT_FOUND", "path": ["repository", alias],
                               "message": f"Could not resolve to a PullRequest with the number of {number}."})
        answer = {"data": {"repository": found}}
        if errors:
            answer["errors"] = errors
        self.reply(answer)

    do_PUT = do_PATCH = do_POST


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8787
    print(f"mock GitHub API on http://127.0.0.1:{port}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
