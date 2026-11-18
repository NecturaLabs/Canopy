#!/usr/bin/env python3
"""Check that tagging and publishing require tests of the exact release commit, that the feed and
site tokens live only in environments, and that releases are complete before they are published."""

from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parent.parent


def job(workflow, name):
    text = (ROOT / ".github/workflows" / workflow).read_text()
    match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [a-z][a-z_-]*:|\Z)", text, re.M | re.S)
    if match is None:
        raise AssertionError(f"{workflow} has no {name} job")
    return match[1]


class ReleaseGateTests(unittest.TestCase):
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
        }
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            text = path.read_text()
            self.assertNotIn("secrets: inherit", text, path.name)
            if "\njobs:\n" not in text:
                continue
            for name in re.findall(r"^  ([a-z][a-z_-]*):\n", text.split("\njobs:\n", 1)[1], re.M):
                body = job(path.name, name)
                for secret, where in allowed.items():
                    if f"secrets.{secret}" in body:
                        self.assertIn((path.name, name), where, f"{path.name} {name} reads {secret}")

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
        self.assertEqual(re.findall(r"^\s+(\w+)\)", deploy, re.M), ["ready"])
        self.assertIn("published_at", deploy)

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
