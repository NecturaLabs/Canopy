#!/usr/bin/env python3
"""Check the subjects and, when required, the DCO sign-offs in BASE..HEAD."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NamedTuple

from release import SUBJECT, expect_eq

SIGNER = re.compile(r"(?P<name>.*) <(?P<email>[^<>]*)>")

SUBJECT_HINT = (
    "Write each subject as 'type(scope): summary', with a lowercase type. Change the last commit with",
    "git commit --amend and an earlier one with git rebase -i.",
)
SIGNOFF_HINT = (
    "A Signed-off-by trailer must match the commit's author: the name exactly, the email ignoring",
    "letter case. git adds the trailer with the name and email you commit with now, which is the",
    "author's only when they are the same person. BASE below is the branch the pull request targets.",
    "- Commits you wrote under the name and email git is set to now: git commit --amend --no-edit",
    "  --signoff for the last commit, or git rebase --signoff BASE for several.",
    "- Commits you wrote under another name or email, which --signoff alone does not fix: set",
    "  user.name and user.email, then git commit --amend --no-edit --signoff --reset-author, or for",
    "  several git rebase --exec 'git commit --amend --no-edit --signoff --reset-author' BASE.",
    "  --reset-author makes you the author, so use it only on your own commits.",
    "- Commits another person wrote: they add their own sign-off.",
)


class Violation(NamedTuple):
    sha: str
    subject: str
    kind: str
    author: str


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


def signed_off(name: str, email: str, trailers: str) -> bool:
    """Whether a trailer names the author: the name exactly, the email ignoring letter case."""
    for line in trailers.split("\n"):
        signer = SIGNER.fullmatch(line)
        if signer and signer["name"] == name and signer["email"].lower() == email.lower():
            return True
    return False


def plain(text: str) -> str:
    """Text from a commit, safe to print: a control character becomes U+FFFD."""
    return "".join(char if char.isprintable() else "\ufffd" for char in text)


def report(errors: list[Violation]) -> list[str]:
    """One line per violation, starting with the commit's id; the hints follow, indented or plain.

    The subject and the author come from the pull request's author. They are printed after the id,
    never at the start of a line and never as an annotation, so none can start a workflow command.
    """
    lines = []
    for sha, subject, kind, author in errors:
        if kind == "subject":
            lines.append(f"{sha} {plain(subject)} - the subject is not 'type(scope): summary'.")
        else:
            lines.append(f"{sha} {plain(subject)} - no 'Signed-off-by: {plain(author)}' trailer.")
    for kind, hint in (("subject", SUBJECT_HINT), ("signoff", SIGNOFF_HINT)):
        if any(error.kind == kind for error in errors):
            lines.extend(("", *hint))
    return lines


def violations(repo: Path, base: str, head: str, require_signoff: bool) -> list[Violation]:
    commits = git(
        repo, "rev-list", "--no-merges", "--reverse", "--end-of-options", f"{base}..{head}", "--",
    )
    errors = []
    for sha in commits.splitlines():
        metadata = git(
            repo, "show", "--no-patch", "--no-show-signature", "--encoding=UTF-8",
            "--format=%s%x00%an%x00%ae%x00%(trailers:key=Signed-off-by,valueonly,unfold)", sha, "--",
        )
        subject, name, email, signoffs = metadata.split("\0", 3)
        if SUBJECT.fullmatch(subject.strip()) is None:
            errors.append(Violation(sha, subject, "subject", f"{name} <{email}>"))
        if require_signoff and not signed_off(name, email, signoffs):
            errors.append(Violation(sha, subject, "signoff", f"{name} <{email}>"))
    return errors


def selftest() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        repo = root / "repo"
        repo.mkdir()
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
        lowercase_key = commit(
            "chore: sign with a lowercase key", "-m", "signed-off-by: Test Author <author@example.invalid>",
        )
        noreply = commit(
            "fix: sign as a no-reply address", "--author", "Plus Person <12345+plus@users.noreply.github.com>",
            "-m", "Signed-off-by: Plus Person <12345+plus@users.noreply.github.com>",
        )
        non_ascii = commit(
            "feat: sign as a non-ASCII author", "--author", "Zo\u00eb M\u00fcller <zoe@example.invalid>",
            "-m", "Signed-off-by: Zo\u00eb M\u00fcller <zoe@example.invalid>",
        )
        email_case = commit(
            "fix: sign with another letter case in the email", "-m",
            "Signed-off-by: Test Author <AUTHOR@Example.Invalid>",
        )
        name_case = commit(
            "fix: sign with another letter case in the name", "-m",
            "Signed-off-by: test author <author@example.invalid>",
        )
        workflow_command = commit("::error::file=x::not a command", "-s")
        control_characters = commit("Bad subject\r::error::boom\x1b[31m", "-s")
        git(repo, "switch", "-q", "main")
        base = commit("Base branch change")
        git(repo, "switch", "-q", "topic")
        git(repo, "merge", "-q", "--no-ff", "-m", "Merge main into topic", "main")

        def found(base: str, head: str, require_signoff: bool) -> list[tuple[str, str]]:
            return [(v.sha, v.kind) for v in violations(repo, base, head, require_signoff)]

        expect_eq(found(base, signed, True), [])
        expect_eq(found(base, "HEAD", True), [
            (wrong_subject, "subject"), (unsigned, "signoff"),
            (wrong_signer, "signoff"), (body_only, "signoff"), (name_case, "signoff"),
            (workflow_command, "subject"), (control_characters, "subject"),
        ])
        expect_eq(found(base, "HEAD", False), [
            (wrong_subject, "subject"), (workflow_command, "subject"), (control_characters, "subject"),
        ])
        passing = [signed, lowercase_key, noreply, non_ascii, email_case]
        flagged = {sha for sha, _ in found(base, "HEAD", True)}
        expect_eq(sorted(set(passing) & flagged), [])

        script = str(Path(__file__).resolve())
        env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}

        def cli(*args: str, cwd: Path = repo) -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                [sys.executable, "-B", script, *args], cwd=cwd, env=env, check=False,
                capture_output=True, encoding="utf-8", errors="surrogateescape",
            )

        result = cli(base, signed, "--signoff", "required")
        expect_eq((result.returncode, result.stdout, result.stderr), (0, "PR commits passed.\n", ""))
        result = cli(base, "HEAD")
        expect_eq((result.returncode, result.stdout), (1, ""))
        expect_eq([line.split(" ", 1)[0] for line in result.stderr.splitlines() if re.match(r"[0-9a-f]{40} ", line)], [
            wrong_subject, workflow_command, control_characters,
        ])
        expect_eq("Signed-off-by" in result.stderr, False)
        result = cli(base, "HEAD", "--signoff", "required")
        expect_eq(result.returncode, 1)
        lines = result.stderr.splitlines()
        reported = {line.split(" ", 1)[0]: line for line in lines if re.match(r"[0-9a-f]{40} ", line)}
        expect_eq(sorted(reported), sorted([
            wrong_subject, unsigned, wrong_signer, body_only, name_case, workflow_command, control_characters,
        ]))
        expect_eq(reported[wrong_subject].startswith(f"{wrong_subject} Change without a Conventional"), True)
        expect_eq(reported[unsigned].startswith(f"{unsigned} docs: add an example"), True)
        expect_eq("Signed-off-by: Test Author <author@example.invalid>" in reported[unsigned], True)
        # A subject is text from the pull request's author: it never starts a line, and a control
        # character in it cannot start a new line or an escape sequence.
        expect_eq([line for line in lines if line.startswith("::")], [])
        expect_eq(reported[workflow_command].startswith(f"{workflow_command} ::error::file=x::not a command"), True)
        expect_eq(reported[control_characters].startswith(f"{control_characters} Bad subject"), True)
        expect_eq([c for c in result.stderr if c != "\n" and not c.isprintable()], [])
        expect_eq("--signoff --reset-author" in result.stderr, True)
        expect_eq([sha in result.stderr for sha in passing], [False] * len(passing))
        result = cli(base, "HEAD", "--signoff", "required", "--signoff", "exempt")
        expect_eq((result.returncode, unsigned in result.stderr), (1, False))
        result = cli(base, "HEAD", "--signoff", "sometimes")
        expect_eq((result.returncode, "invalid choice" in result.stderr), (2, True))
        result = cli("no-such-ref", "HEAD")
        expect_eq((result.returncode, "Could not read the commits" in result.stderr), (2, True))

        shallow = root / "shallow"
        git(root, "clone", "-q", "--depth", "1", repo.as_uri(), str(shallow))
        result = cli("HEAD", "HEAD", cwd=shallow)
        expect_eq((result.returncode, "shallow" in result.stderr), (2, True))
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

    for line in report(errors):
        print(line, file=sys.stderr)
    if not errors:
        print("PR commits passed.")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
