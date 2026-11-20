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
    def run_hook(self, paths, diff_status=0, cargo_status=0, test_status=0):
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
                "elif sys.argv[1:] == ['rev-parse', '--local-env-vars']:\n"
                "    print('GIT_DIR\\nGIT_WORK_TREE\\nGIT_INDEX_FILE\\nGIT_PREFIX')\n"
                "elif sys.argv[1] == 'config':\n"
                "    sys.exit(0)\n"
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
                "with Path(os.environ['HOOK_TEST_ROOT'], 'cargo.json').open('a') as log:\n"
                "    log.write(str(sys.argv[1:]) + '\\n')\n"
                "key = 'HOOK_TEST_TEST_STATUS' if sys.argv[1] == 'test' else 'HOOK_TEST_CARGO_STATUS'\n"
                "sys.exit(int(os.environ[key]))\n",
                encoding="utf-8",
            )
            git.chmod(0o700)
            cargo.chmod(0o700)
            env = os.environ | {
                "PATH": str(root) + os.pathsep + os.environ["PATH"],
                "HOOK_TEST_ROOT": str(root),
                "HOOK_TEST_DIFF_STATUS": str(diff_status),
                "HOOK_TEST_CARGO_STATUS": str(cargo_status),
                "HOOK_TEST_TEST_STATUS": str(test_status),
            }
            result = subprocess.run(["bash", str(HOOK)], env=env, capture_output=True)
            invocation = root / "cargo.json"
            return result.returncode, invocation.read_text() if invocation.exists() else None

    def test_a_real_commit_does_not_pass_git_context_to_cargo(self):
        self.run_real_commit(check_config=False)

    def test_workspace_tests_use_isolated_git_configuration(self):
        self.run_real_commit(check_config=True)

    def run_real_commit(self, check_config):
        with tempfile.TemporaryDirectory(prefix="canopy-hook-context-") as scratch:
            root = Path(scratch)
            repo = root / "repo"
            repo.mkdir()
            fake_bin = root / "bin"
            fake_bin.mkdir()
            cargo = fake_bin / "cargo"
            cargo.write_text(
                f"#!{sys.executable}\n"
                "import os, subprocess, sys\n"
                "from pathlib import Path\n"
                "if any(k in os.environ for k in ['GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE', 'GIT_PREFIX']):\n"
                "    sys.exit(99)\n"
                "if sys.argv[1] == 'test' and os.environ['HOOK_CHECK_CONFIG'] == '1':\n"
                "    fixture = Path(os.environ['HOOK_TEST_ROOT']) / 'fixture'\n"
                "    subprocess.run(['git', 'init', '-q', '-b', 'main', str(fixture)], check=True)\n"
                "    def value(key):\n"
                "        return subprocess.run(['git', '-C', str(fixture), 'config', '--get', key], capture_output=True, text=True).stdout.strip()\n"
                "    if value('user.name') != 'Canopy CI' or value('user.email') != 'ci@canopy.invalid':\n"
                "        sys.exit(98)\n"
                "    if any(value(k) for k in ['commit.gpgsign', 'core.hooksPath', 'core.autocrlf', 'canopy.systemSentinel']):\n"
                "        sys.exit(97)\n"
                "sys.exit(0)\n",
                encoding="utf-8",
            )
            cargo.chmod(0o700)
            env = os.environ | {
                "PATH": str(fake_bin) + os.pathsep + os.environ["PATH"],
                "GIT_CONFIG_GLOBAL": str(root / "global-config"),
                "GIT_CONFIG_SYSTEM": str(root / "system-config"),
                "HOOK_TEST_ROOT": str(root),
                "HOOK_CHECK_CONFIG": "1" if check_config else "0",
            }
            env.pop("GIT_CONFIG_NOSYSTEM", None)
            (root / "global-config").write_text(
                "[user]\n name = Outside Test\n email = outside@example.invalid\n"
                "[commit]\n gpgsign = true\n[core]\n autocrlf = true\n hooksPath = /nonexistent\n"
            )
            (root / "system-config").write_text("[canopy]\n systemSentinel = outside\n")
            def git(*args):
                return subprocess.run(["git", "-C", str(repo), *args], env=env, capture_output=True)
            self.assertEqual(git("init", "-q", "-b", "main").returncode, 0)
            for key, value in [("user.name", "Hook Test"), ("user.email", "hook@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", str(HOOK.parent))]:
                self.assertEqual(git("config", key, value).returncode, 0)
            (repo / "test.rs").write_text("// Fixture\n")
            self.assertEqual(git("add", "test.rs").returncode, 0)
            result = git("commit", "-qm", "fixture")
            self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_large_matching_diff_runs_clippy(self):
        paths = ["first.rs"] + [f"docs/long-document-name-{i}.md" for i in range(100_000)]
        status, invocation = self.run_hook(paths)
        self.assertEqual(status, 0)
        self.assertIsNotNone(invocation)
        self.assertIn("'--locked'", invocation)
        self.assertIn("'-D', 'warnings'", invocation)
        self.assertIn("['test', '--workspace', '--locked']", invocation)

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

    def test_failed_test_refuses_the_commit(self):
        status, invocation = self.run_hook(["first.rs"], test_status=42)
        self.assertEqual(status, 1)
        self.assertIn("['test', '--workspace', '--locked']", invocation)

    def test_the_toolchain_pin_runs_clippy(self):
        status, invocation = self.run_hook(["rust-toolchain.toml"])
        self.assertEqual(status, 0)
        self.assertIsNotNone(invocation)

    def test_unrelated_files_skip_clippy(self):
        status, invocation = self.run_hook(["docs/example.md", "art/sample.png"])
        self.assertEqual(status, 0)
        self.assertIsNone(invocation)


if __name__ == "__main__":
    unittest.main()
