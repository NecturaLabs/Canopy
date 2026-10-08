#!/usr/bin/env python3
"""Exercise the pre-commit hook's staged-file gate and failure propagation."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


HOOK = Path(__file__).resolve().parent / "hooks" / "pre-commit"


class HookTests(unittest.TestCase):
    def run_hook(self, paths, diff_status=0, cargo_status=0):
        with tempfile.TemporaryDirectory(prefix="canopy-hook-test-") as scratch:
            root = Path(scratch)
            (root / "paths.json").write_text(json.dumps(paths), encoding="utf-8")
            git = root / "git"
            git.write_text(
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                "from pathlib import Path\n"
                "root = Path(os.environ['HOOK_TEST_ROOT'])\n"
                "if sys.argv[1:] == ['rev-parse', '--show-toplevel']:\n"
                "    print(root)\n"
                "elif sys.argv[1] == 'diff':\n"
                "    paths = json.loads((root / 'paths.json').read_text())\n"
                "    if '-z' in sys.argv:\n"
                "        sys.stdout.buffer.write(b''.join(os.fsencode(p) + b'\\0' for p in paths))\n"
                "    else:\n"
                "        for path in paths:\n"
                "            print(json.dumps(path) if '\\n' in path else path)\n"
                "    sys.exit(int(os.environ['HOOK_TEST_DIFF_STATUS']))\n"
                "else:\n"
                "    sys.exit(99)\n",
                encoding="utf-8",
            )
            cargo = root / "cargo"
            cargo.write_text(
                f"#!{sys.executable}\n"
                "import os, sys\n"
                "from pathlib import Path\n"
                "Path(os.environ['HOOK_TEST_ROOT'], 'cargo.json').write_text(str(sys.argv[1:]))\n"
                "sys.exit(int(os.environ['HOOK_TEST_CARGO_STATUS']))\n",
                encoding="utf-8",
            )
            git.chmod(0o700)
            cargo.chmod(0o700)
            env = os.environ | {
                "PATH": str(root) + os.pathsep + os.environ["PATH"],
                "HOOK_TEST_ROOT": str(root),
                "HOOK_TEST_DIFF_STATUS": str(diff_status),
                "HOOK_TEST_CARGO_STATUS": str(cargo_status),
            }
            result = subprocess.run(["bash", str(HOOK)], env=env, capture_output=True)
            invocation = root / "cargo.json"
            return result.returncode, invocation.read_text() if invocation.exists() else None

    def test_large_matching_diff_runs_clippy(self):
        paths = ["first.rs"] + [f"docs/long-document-name-{i}.md" for i in range(100_000)]
        status, invocation = self.run_hook(paths)
        self.assertEqual(status, 0)
        self.assertIsNotNone(invocation)
        self.assertIn("'--locked'", invocation)
        self.assertIn("'-D', 'warnings'", invocation)

    def test_unusual_rust_filename_runs_clippy(self):
        status, invocation = self.run_hook(["src/a\nquoted name.rs"])
        self.assertEqual(status, 0)
        self.assertIsNotNone(invocation)

    def test_failed_diff_refuses_the_commit(self):
        status, invocation = self.run_hook(["first.rs"], diff_status=97)
        self.assertEqual(status, 97)
        self.assertIsNone(invocation)

    def test_failed_clippy_refuses_the_commit(self):
        status, invocation = self.run_hook(["nested/Cargo.toml"], cargo_status=42)
        self.assertEqual(status, 1)
        self.assertIsNotNone(invocation)

    def test_unrelated_files_skip_clippy(self):
        status, invocation = self.run_hook(["docs/example.md", "art/sample.png"])
        self.assertEqual(status, 0)
        self.assertIsNone(invocation)


if __name__ == "__main__":
    unittest.main()
