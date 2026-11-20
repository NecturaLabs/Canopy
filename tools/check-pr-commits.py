#!/usr/bin/env python3
"""Check the subjects and, when required, the DCO sign-offs in BASE..HEAD."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from release import SUBJECT, expect_eq


def git(repo: Path, *args: str) -> str:
    # Inherited Git variables can redirect fixture commands away from their temporary repository.
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    return subprocess.run(
        ["git", *args],
        cwd=repo,
        env=env,
        check=True,
        capture_output=True,
        encoding="utf-8",
        errors="surrogateescape",
    ).stdout


def violations(repo: Path, base: str, head: str, require_signoff: bool) -> list[tuple[str, str]]:
    commits = git(
        repo, "rev-list", "--no-merges", "--reverse", "--end-of-options", f"{base}..{head}", "--",
    )
    errors = []
    for sha in commits.splitlines():
        metadata = git(
            repo, "show", "--no-patch", "--no-show-signature", "--encoding=UTF-8",
            "--format=%s%x00%an <%ae>%x00%(trailers:key=Signed-off-by,valueonly,unfold)", sha, "--",
        )
        subject, author, signoffs = metadata.split("\0", 2)
        if SUBJECT.fullmatch(subject.strip()) is None:
            errors.append((sha, "subject"))
        if require_signoff and author not in signoffs.split("\n"):
            errors.append((sha, "signoff"))
    return errors


def selftest() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp)
        git(repo, "init", "-q", "--template=", "-b", "main")
        git(repo, "config", "user.name", "Test Author")
        git(repo, "config", "user.email", "author@example.invalid")
        git(repo, "config", "commit.gpgsign", "false")

        def commit(subject: str, *args: str) -> str:
            git(repo, "commit", "-q", "--allow-empty", "-m", subject, *args)
            return git(repo, "rev-parse", "HEAD").strip()

        commit("Old subject")
        git(repo, "switch", "-qc", "topic")
        signed = commit(
            "feat(core)!: add a change", "-m", "Signed-off-by: Another Author <other@example.invalid>", "-s",
        )
        wrong_subject = commit("Change without a Conventional Commit subject", "-s")
        unsigned = commit("docs: add an example")
        wrong_signer = commit(
            "fix: correct the example", "-m",
            "Signed-off-by: Test Author <other@example.invalid>\n"
            "Signed-off-by: Another Author <author@example.invalid>",
        )
        body_only = commit(
            "test: cover the example", "-m",
            "Signed-off-by: Test Author <author@example.invalid>\n\nThis is the last paragraph.",
        )
        git(repo, "switch", "-q", "main")
        base = commit("Base branch change")
        git(repo, "switch", "-q", "topic")
        git(repo, "merge", "-q", "--no-ff", "-m", "Merge main into topic", "main")

        expect_eq(violations(repo, base, signed, True), [])
        expect_eq(violations(repo, base, "HEAD", True), [
            (wrong_subject, "subject"), (unsigned, "signoff"),
            (wrong_signer, "signoff"), (body_only, "signoff"),
        ])
        expect_eq(violations(repo, base, "HEAD", False), [(wrong_subject, "subject")])
    print("PR commit check selftest passed.")


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        selftest()
        return 0

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", help="base commit or ref")
    parser.add_argument("head", help="head commit or ref")
    parser.add_argument(
        "--signoff", choices=("exempt", "required"), default="exempt",
        help="require a DCO trailer matching the author (default: exempt)",
    )
    args = parser.parse_args()
    repo = Path.cwd()
    try:
        if git(repo, "rev-parse", "--is-shallow-repository").strip() == "true":
            parser.exit(2, "This checkout is shallow. Fetch the full history with git fetch --unshallow.\n")
        errors = violations(repo, args.base, args.head, args.signoff == "required")
    except subprocess.CalledProcessError:
        parser.exit(2, "Could not read the commits. Check that both refs and their history are available.\n")

    for sha, kind in errors:
        if kind == "subject":
            print(
                f"{sha}: Reword the subject as 'type(scope): summary' with git commit --amend "
                "or git rebase -i for an earlier commit.",
                file=sys.stderr,
            )
        else:
            print(
                f"{sha}: Add a Signed-off-by trailer matching the commit author's name and email. "
                "For your own commits, use git commit --amend -s or git rebase --signoff.",
                file=sys.stderr,
            )
    if not errors:
        print("PR commits passed.")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
