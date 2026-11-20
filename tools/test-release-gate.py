#!/usr/bin/env python3
"""Check that tagging and publishing require tests of the exact release commit, that the feed and
site tokens live only in environments, and that releases are complete before they are published."""

import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile
from textwrap import dedent
import unittest

ROOT = Path(__file__).resolve().parent.parent


def job(workflow, name):
    text = (ROOT / ".github/workflows" / workflow).read_text()
    match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:\s*$|\Z)", text, re.M | re.S)
    if match is None:
        raise AssertionError(f"{workflow} has no {name} job")
    return match[1]


class ReleaseGateTests(unittest.TestCase):
    def test_setup_rust_uses_an_exact_toolchain(self):
        setup = (ROOT / ".github/actions/setup-rust/action.yml").read_text()
        self.assertNotRegex(setup, r"(?m)^\s*toolchain:\s*['\"]?stable\b")
        action = setup.split("dtolnay/rust-toolchain@", 1)[1].split("    - ", 1)[0]
        self.assertIn(
            "toolchain: ${{ steps.pin-unix.outputs.channel || steps.pin-windows.outputs.channel }}",
            action,
        )
        pin = (ROOT / "rust-toolchain.toml").read_text()
        channels = re.findall(r"(?m)^[ \t]*channel[ \t]*=[ \t]*(.+)$", pin)
        self.assertEqual(len(channels), 1)
        self.assertRegex(channels[0], r'^"1\.[0-9]+\.[0-9]+"\s*(?:#.*)?$')

    def test_release_ships_toolchain_report_before_checksums_and_attestation(self):
        build = job("release.yml", "build")
        for command in ("rustc -Vv", "cargo -V", "rustup show"):
            self.assertIn(command, build)
        release = job("release.yml", "release")
        self.assertLess(release.index("name: Assemble the toolchain report"), release.index("name: Checksums"))
        self.assertIn("dist/toolchain.txt", release.split("subject-path: |", 1)[1].split("      #", 1)[0])
        self.assertIn("dist/toolchain.txt", release.split("name: release-files", 1)[1].split("      - ", 1)[0])

    def test_a_published_release_downloads_its_report_when_checksummed(self):
        release = job("release.yml", "release")
        step = release.split("name: Take the published files\n", 1)[1].split("      - ", 1)[0]
        script = dedent(step.split("        run: |\n", 1)[1])
        gh = dedent("""\
            gh() {
              case "$7" in
                'canopy-build-*') mkdir -p "$9" && cp published/canopy-build-* "$9/" ;;
                SHA256SUMS) cp published/SHA256SUMS "$9" ;;
                toolchain.txt) cp published/toolchain.txt "$9/" ;;
                *) return 1 ;;
              esac
            }
            """)
        for has_report in (False, True):
            with self.subTest(has_report=has_report), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                published = root / "published"
                published.mkdir()
                assets = {"canopy-build-1-x86_64-unknown-linux-gnu.tar.gz": b"archive"}
                if has_report:
                    assets["toolchain.txt"] = b"rustc 1.100.0\n"
                for name, contents in assets.items():
                    (published / name).write_bytes(contents)
                sums = "".join(
                    f"{hashlib.sha256(contents).hexdigest()}  {name}\n"
                    for name, contents in assets.items()
                )
                (published / "SHA256SUMS").write_bytes(sums.encode())
                subprocess.run(
                    ["bash", "-e", "-o", "pipefail", "-c", gh + script], cwd=root, check=True,
                    env={
                        "PATH": os.environ["PATH"],
                        "RUNNER_TEMP": tmp,
                        "TAG": "build-1",
                        "GITHUB_REPOSITORY": "test/repo",
                    },
                )
                self.assertEqual({p.name: p.read_bytes() for p in (root / "dist").iterdir()}, assets)

    def test_tagging_waits_for_tests_of_dispatch_commit(self):
        tests = job("create-release.yml", "test")
        self.assertIn("uses: ./.github/workflows/test-release.yml", tests)
        self.assertIn("commit: ${{ github.sha }}", tests)
        prepare = job("create-release.yml", "prepare")
        self.assertIn("needs: [test]", prepare)
        self.assertIn("needs.test.result == 'success'", prepare)
        self.assertIn("ref: ${{ github.sha }}", prepare)

    def test_publishing_waits_for_tests_of_identified_commit(self):
        tests = job("release.yml", "test")
        self.assertIn("uses: ./.github/workflows/test-release.yml", tests)
        self.assertIn("needs: [identity]", tests)
        self.assertIn("commit: ${{ needs.identity.outputs.commit }}", tests)
        self.assertIn("test", re.search(r"needs: \[(.*?)\]", job("release.yml", "release"))[1].split(", "))

    def test_publishing_jobs_ship_the_identified_commit(self):
        # A tag moved after the identity job checked it must not change what ships.
        for name in ("release", "publish-update"):
            body = job("release.yml", name)
            self.assertIn("identity", re.search(r"needs: \[(.*?)\]", body)[1].split(", "), name)
            refs = re.findall(r"^\s+ref: (.*)$", body, re.M)
            self.assertTrue(refs, name)
            for ref in refs:
                self.assertTrue(ref.startswith("${{ needs.identity.outputs.commit }}"), f"{name}: {ref}")

    def test_feed_token_only_in_release_environment_jobs(self):
        text = (ROOT / ".github/workflows/release.yml").read_text()
        jobs = re.findall(r"^  ([a-z][a-z_-]*):\n", text.split("\njobs:\n", 1)[1], re.M)
        users = [name for name in jobs if "secrets.APP_UPDATES_TOKEN" in job("release.yml", name)]
        self.assertEqual(users, ["publish-update", "prune"])
        for name in users:
            self.assertRegex(job("release.yml", name), r"(?m)^    environment: release$", name)
        # Nothing hands the token over as a workflow secret, which any branch could read.
        self.assertNotIn("secrets.APP_UPDATES_TOKEN", text.split("\njobs:\n", 1)[0])
        self.assertNotIn("APP_UPDATES_TOKEN", (ROOT / ".github/workflows/create-release.yml").read_text().split("\njobs:\n", 1)[1])

    def test_releases_take_every_file_before_they_are_published(self):
        # Immutable releases refuse files once published: each is a draft until the end.
        for name, repo in (("release", '"$GITHUB_REPOSITORY"'), ("publish-update", "NecturaLabs/app-updates")):
            body = job("release.yml", name)
            upload = body.index("softprops/action-gh-release@")
            self.assertIn("draft: true", body[upload:], name)
            publish = body.index("--draft=false", upload)
            self.assertIn(f"--repo {repo}", body[publish:].splitlines()[0], name)
        self.assertNotIn("gh release upload", (ROOT / ".github/workflows/release.yml").read_text())

    def test_site_token_only_in_the_deploy_job(self):
        site = job("site.yml", "site")
        self.assertNotIn("secrets.", site)
        deploy = job("site.yml", "deploy")
        self.assertRegex(deploy, r"(?m)^    environment: site$")
        self.assertIn("needs: [site]", deploy)
        self.assertIn("secrets.NETLIFY_AUTH_TOKEN", deploy)
        # No package manager runs while the token is in the job.
        self.assertNotRegex(deploy, r"\b(npx|npm|pnpm|yarn|bunx|setup-node)\b")

    def test_tokens_appear_nowhere_else(self):
        allowed = {
            "APP_UPDATES_TOKEN": {("release.yml", "publish-update"), ("release.yml", "prune")},
            "NETLIFY_AUTH_TOKEN": {("site.yml", "deploy")},
            "MANIFEST_SIGNING_KEY": {("release.yml", "publish-update")},
        }
        reads = re.compile(r"secrets\s*(?:\.\s*|\[\s*['\"])(\w+)")
        paths = sorted((ROOT / ".github").rglob("*.y*ml"))
        self.assertTrue(paths)
        for path in paths:
            text = path.read_text()
            self.assertNotRegex(text, r"secrets\s*:\s*inherit", path.name)
            self.assertNotRegex(text, r"toJSON\(\s*secrets", path.name)
            head, _, body = text.partition("\njobs:\n")
            self.assertFalse(
                {m for m in reads.findall(head) if m in allowed}, f"{path.name} reads a token outside a job"
            )
            for name in re.findall(r"^  ([\w-]+):\s*$", body, re.M):
                for secret in set(reads.findall(job(path.name, name) if path.parent.name == "workflows" else "")):
                    if secret in allowed:
                        self.assertIn((path.name, name), allowed[secret], f"{path.name} {name} reads {secret}")
            if path.parent.name != "workflows":
                self.assertFalse({m for m in reads.findall(text) if m in allowed}, path.name)

    def test_signing_key_only_in_its_own_step(self):
        # The manifest key sits in one step that runs nothing but the signer: no clone, no push,
        # no other secret beside it.
        steps = re.split(r"(?m)^      - ", job("release.yml", "publish-update"))
        signing = [step for step in steps if "MANIFEST_SIGNING_KEY" in step]
        self.assertEqual(len(signing), 2, "an early check and the signing step")
        check, sign = signing
        self.assertIn("secrets.MANIFEST_SIGNING_KEY != ''", check)
        self.assertEqual(re.findall(r"secrets\.(\w+)", sign), ["MANIFEST_SIGNING_KEY"])
        run = sign.split("run: |", 1)[1]
        commands = [line.strip() for line in run.splitlines() if line.strip() and not line.strip().startswith("#")]
        self.assertTrue(commands)
        self.assertTrue(all(re.search(r'tools/sign-manifest\.py"? sign ', c) for c in commands), commands)
        self.assertNotRegex(sign, r"\b(git|gh|curl)\b")

    def test_release_builds_use_the_built_in_update_key(self):
        # A compile-time key override exists for local end-to-end tests only.
        for path in sorted((ROOT / ".github").rglob("*.y*ml")):
            self.assertNotIn("CANOPY_UPDATE_KEY", path.read_text(), path.name)

    def test_experimental_builds_ship_without_holding_the_release(self):
        release = job("release.yml", "release")
        needs = re.search(r"needs: \[(.*?)\]", release)[1].split(", ")
        self.assertIn("build-experimental", needs)
        self.assertIn(
            "if: ${{ !cancelled() && needs.identity.result == 'success' && needs.build.result == 'success' && needs.test.result == 'success' }}",
            release,
        )
        self.assertNotIn("needs.build-experimental.result", release)
        # The checksums and provenance cover the experimental archives too.
        self.assertLess(release.index("name: Add the experimental archives"), release.index("name: Checksums"))
        self.assertLess(release.index("name: Checksums"), release.index("attest-build-provenance@"))
        # The feed ships exactly the files the GitHub release did.
        feed = job("release.yml", "publish-update")
        self.assertIn("name: release-files", feed)
        self.assertNotIn("pattern: canopy-*", feed)
        self.assertIn("if: ${{ !cancelled() && needs.release.result == 'success' }}", feed)
        prune = job("release.yml", "prune")
        self.assertIn("needs: [identity, publish-update]", prune)
        self.assertIn("if: ${{ !cancelled() && needs.publish-update.result == 'success' }}", prune)

    def test_a_published_release_ships_its_own_files(self):
        release = job("release.yml", "release")
        state = release.index("name: Is the release already published")
        self.assertLess(state, release.index("pattern: canopy-*"))
        self.assertIn("gh release download", release)
        for step in re.split(r"(?m)^      - ", release):
            if "pattern: canopy-*" in step or "attest-build-provenance@" in step or "name: Add the experimental archives" in step:
                self.assertIn("steps.state.outputs.published != 'true'", step)
        for step in re.split(r"(?m)^      - ", release):
            if "gh release download" in step:
                self.assertIn("steps.state.outputs.published == 'true'", step)
            if "name: Add the installers" in step or "name: Checksums" in step:
                self.assertNotIn("if:", step)
        self.assertIn("name: Match the published checksums", release)
        feed = job("release.yml", "publish-update")
        self.assertIn("is published with other files than release", feed)
        self.assertIn("sha256sum --check", feed)

    def test_nothing_can_fail_after_the_release_is_published(self):
        release = job("release.yml", "release")
        published = release.index("--draft=false")
        self.assertLess(release.index("name: release-files"), published)
        self.assertNotRegex(release[published:], r"(?m)^      - ")
        # A rerun leaves an already published (immutable) release alone.
        for name in ("release", "publish-update"):
            body = job("release.yml", name)
            for step in re.split(r"(?m)^      - ", body):
                if "softprops/action-gh-release@" in step or "--draft=false" in step:
                    self.assertRegex(step, r"published != 'true'", name)

    def test_site_deploy_is_main_only_and_waits_until_live(self):
        deploy = job("site.yml", "deploy")
        self.assertIn(
            "if: github.ref == 'refs/heads/main' && (github.event_name == 'push' || github.event_name == 'workflow_dispatch')",
            deploy,
        )
        self.assertEqual(re.findall(r"uses: ([^@\s]+)@", deploy), ["actions/checkout"])
        self.assertNotRegex(deploy, r"\b(apt-get|apt|pip3?|cargo install|go install)\b|\| *(ba)?sh\b")
        # Only `ready` with a publish time counts as live.
        self.assertEqual(deploy.count("exit 0"), 1)
        cases = deploy[deploy.index("case $state in") : deploy.index("esac")]
        self.assertEqual(re.findall(r"^\s+([^\s(#][^()\n]*)\)", cases, re.M), ["ready", "error | rejected"])
        live = deploy.index("exit 0")
        self.assertIn("published_at", deploy[deploy.index("ready)"):live])

    def test_test_job_has_no_write_permission_or_optional_gate(self):
        tests = job("test-release.yml", "test")
        self.assertIn("contents: read", tests)
        self.assertNotIn("write", tests)
        self.assertIn("runs-on: ubuntu-latest", tests)
        self.assertNotRegex(tests, re.compile(r"^\s+(if|continue-on-error):", re.M))
        self.assertIn("ref: ${{ inputs.commit }}", tests)
        self.assertIn("persist-credentials: false", tests)
        self.assertIn("run: cargo test --workspace --locked --no-fail-fast", tests)


if __name__ == "__main__":
    unittest.main()
