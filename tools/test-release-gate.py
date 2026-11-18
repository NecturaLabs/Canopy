#!/usr/bin/env python3
"""Check that tagging and publishing require tests of the exact release commit."""

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
        for name in ("release", "publish-update", "attach-experimental"):
            body = job("release.yml", name)
            self.assertIn("identity", re.search(r"needs: \[(.*?)\]", body)[1].split(", "), name)
            refs = re.findall(r"^\s+ref: (.*)$", body, re.M)
            self.assertTrue(refs, name)
            for ref in refs:
                self.assertTrue(ref.startswith("${{ needs.identity.outputs.commit }}"), f"{name}: {ref}")

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
