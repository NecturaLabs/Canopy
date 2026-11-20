#!/usr/bin/env python3
"""Check that tagging and publishing require tests of the exact release commit, that the feed and
site tokens live only in environments, and that releases are complete before they are published."""

import hashlib
import json
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


def run_block(step):
    """The dedented body of a step's `run: |` block, without comments that follow the step."""
    if "run: |\n" not in step:
        return re.search(r"(?m)^\s*run: (.+)$", step)[1] + "\n"
    script = step.split("run: |\n", 1)[1].splitlines()
    indent = len(script[0]) - len(script[0].lstrip())
    kept = [line for line in script if not line.strip() or line.startswith(" " * indent)]
    return dedent("\n".join(kept)) + "\n"


def step_script(body, name):
    """The `run:` script of the step called `name` in a job's text."""
    steps = [s for s in re.split(r"(?m)^      - ", body) if s.startswith(f"name: {name}\n")]
    if len(steps) != 1:
        raise AssertionError(f"expected one step called {name}")
    return run_block(steps[0])


def action_pin_step(identifier):
    action = (ROOT / ".github/actions/setup-rust/action.yml").read_text()
    steps = [s for s in re.split(r"(?m)^    - ", action) if f"id: {identifier}\n" in s]
    if len(steps) != 1:
        raise AssertionError(f"setup-rust has no step {identifier}")
    return steps[0]


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
        # A stand-in for `gh release download` that reads its options by name.
        gh = dedent("""\
            gh() {
              local pattern='' dir='' output=''
              shift 2
              while (( $# )); do
                case $1 in
                  --pattern) pattern=$2; shift ;;
                  --dir) dir=$2; shift ;;
                  --output) output=$2; shift ;;
                esac
                shift
              done
              case $pattern in
                'canopy-build-*') mkdir -p "$dir" && cp published/canopy-build-* "$dir/" ;;
                SHA256SUMS) cp published/SHA256SUMS "$output" ;;
                toolchain.txt) mkdir -p "$dir" && cp published/toolchain.txt "$dir/" ;;
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

    def test_the_checkout_is_checked_before_anything_from_it_runs(self):
        # setup-rust is a local action and reads rust-toolchain.toml from the checkout, so the
        # check that the checkout is the tagged commit comes first.
        build = job("release.yml", "build")
        self.assertLess(
            build.index("name: Check the checkout is the tagged commit"),
            build.index("uses: ./.github/actions/setup-rust"),
        )

    def test_new_releases_require_a_rust_pin_from_the_identity_job(self):
        identity = job("release.yml", "identity")
        script = step_script(identity, "Require a Rust pin for new releases")
        self.assertIn("rust-toolchain.toml", script)
        self.assertIn("exit 1", script)
        self.assertIn("gh release view", script)

    def test_the_tests_check_depends_on_the_msrv_job(self):
        tests = job("ci.yml", "tests")
        self.assertIn("msrv", re.search(r"needs: \[(.*?)\]", tests)[1].split(", "))
        self.assertIn("MSRV: ${{ needs.msrv.result }}", tests)
        self.assertIn('for r in "$CHANGES" "$RESULT" "$MSRV"; do', tests)
        self.assertIn("if: always()", tests)

    def test_every_job_that_runs_cargo_installs_the_pin_first(self):
        # cargo reads rust-toolchain.toml through rustup, which would otherwise install the
        # toolchain unverified and unlogged.
        for workflow, name in (("advisories.yml", "advisories"), ("ci.yml", "checks")):
            body = job(workflow, name)
            self.assertIn("uses: ./.github/actions/setup-rust", body, name)
            first_cargo = re.search(r"(?m)^\s+(?:- )?run: cargo\b", body)
            self.assertLess(body.index("setup-rust"), first_cargo.start(), name)

    def run_pin_step(self, contents):
        """Run setup-rust's Unix pin step on a rust-toolchain.toml; return the channel or None."""
        script = run_block(action_pin_step("pin-unix"))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "rust-toolchain.toml").write_bytes(contents)
            out, env_file = root / "output", root / "env"
            out.touch()
            env_file.touch()
            result = subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", script], cwd=root, capture_output=True,
                env={"PATH": os.environ["PATH"], "GITHUB_OUTPUT": str(out), "GITHUB_ENV": str(env_file)},
            )
            if result.returncode != 0:
                self.assertEqual(out.read_text(), "")
                return None
            self.assertEqual(env_file.read_text(), "RUSTUP_AUTO_INSTALL=0\n")
            return out.read_text()

    def pin_pwsh_channel(self, contents):
        """The Windows pin step's pattern applied as Select-String does, or None when rejected.

        PowerShell is not available here, so this translates the .NET pattern to Python's `re`.
        That is equivalent for this pattern: it uses only literals, `\\s`, `[0-9]`, `.`, groups and
        anchors, which .NET and Python read the same on ASCII input (they differ on exotic
        Unicode whitespace, which the cases below do not use), and Select-String matches line by
        line (StreamReader splits on CRLF, LF and CR, so no line keeps its `\\r`) and is made
        case-sensitive by -CaseSensitive.
        """
        step = action_pin_step("pin-windows")
        pattern = re.search(r"-CaseSensitive -Pattern '([^']+)'", step)[1]
        self.assertIn("$pin.Count -ne 1", step)
        matches = [m for line in re.split(r"\r\n|\r|\n", contents.decode()) if (m := re.search(pattern, line))]
        return matches[0][1] if len(matches) == 1 else None

    def test_the_rust_pin_is_read_strictly_on_every_platform(self):
        accepted = {
            "plain": b'[toolchain]\nchannel = "1.100.0"\n',
            "crlf": b'[toolchain]\r\nchannel = "1.100.0"\r\nprofile = "minimal"\r\n',
            "trailing comment": b'[toolchain]\nchannel = "1.100.0" # the pin\n',
            "commented-out second channel": b'[toolchain]\nchannel = "1.100.0"\n# channel = "1.99.0"\n',
            "indented, tight": b'[toolchain]\n\tchannel="1.100.0"\n',
        }
        rejected = {
            "duplicate channel lines": b'[toolchain]\nchannel = "1.100.0"\nchannel = "1.100.0"\n',
            "prerelease": b'[toolchain]\nchannel = "1.100.0-beta"\n',
            "two components": b'[toolchain]\nchannel = "1.100"\n',
            "stable": b'[toolchain]\nchannel = "stable"\n',
            "no channel": b'[toolchain]\nprofile = "minimal"\n',
            "only a comment": b'[toolchain]\n# channel = "1.100.0"\n',
        }
        for label, contents in accepted.items():
            with self.subTest(label=label):
                self.assertEqual(self.run_pin_step(contents), "channel=1.100.0\n")
                self.assertEqual(self.pin_pwsh_channel(contents), "1.100.0")
        for label, contents in rejected.items():
            with self.subTest(label=label):
                self.assertIsNone(self.run_pin_step(contents))
                self.assertIsNone(self.pin_pwsh_channel(contents))

    def run_steps(self, root, body, names, env):
        for name in names:
            script = step_script(body, name)
            subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", script], cwd=root, check=True,
                env={"PATH": os.environ["PATH"]} | env, capture_output=True,
            )

    def test_the_experimental_copy_takes_either_archive_and_names_what_is_missing(self):
        release = job("release.yml", "release")
        cases = {
            "linux-gnu": ("canopy-build-7-aarch64-unknown-linux-gnu", ".tar.gz", True),
            "windows": ("canopy-build-7-aarch64-pc-windows-msvc", ".zip", True),
            "no archive": ("canopy-build-7-x86_64-apple-darwin", None, True),
            "no report": ("canopy-build-7-riscv64gc-unknown-linux-gnu", ".tar.gz", False),
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "late").mkdir()
            (root / "dist").mkdir()
            targets = []
            for name, extension, has_report in cases.values():
                targets.append({"target": name.removeprefix("canopy-build-7-")})
                if extension:
                    (root / "late" / f"{name}{extension}").write_bytes(b"archive")
                if has_report:
                    (root / "late" / f"{name}.toolchain.txt").write_bytes(b"report")
            script = step_script(release, "Add the experimental archives")
            result = subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", script], cwd=root, capture_output=True, text=True,
                env={"PATH": os.environ["PATH"], "BUILD": "7", "EXPECTED": json.dumps(targets)},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                sorted(p.name for p in (root / "dist").iterdir()),
                sorted([
                    "canopy-build-7-aarch64-unknown-linux-gnu.tar.gz",
                    "canopy-build-7-aarch64-unknown-linux-gnu.toolchain.txt",
                    "canopy-build-7-aarch64-pc-windows-msvc.zip",
                    "canopy-build-7-aarch64-pc-windows-msvc.toolchain.txt",
                ]),
            )
            warnings = [l for l in result.stdout.splitlines() if l.startswith("::warning::")]
            self.assertEqual(len(warnings), 2, warnings)
            self.assertIn("No experimental x86_64-apple-darwin archive was found", warnings[0])
            self.assertIn("riscv64gc-unknown-linux-gnu archive has no toolchain report", warnings[1])

    def test_per_target_reports_are_merged_and_removed_before_checksums_and_the_manifest(self):
        # tools/update-manifest.sh finds a platform's archive by the target triple in a file name,
        # so a leftover canopy-build-N-<triple>.toolchain.txt must never reach SHA256SUMS.
        release = job("release.yml", "release")
        self.assertLess(release.index("name: Assemble the toolchain report"), release.index("name: Checksums"))
        archives = {
            "canopy-build-7-x86_64-unknown-linux-gnu.tar.gz": "linux-x86_64",
            "canopy-build-7-aarch64-apple-darwin.tar.gz": "darwin-aarch64",
            "canopy-build-7-x86_64-pc-windows-msvc.zip": "windows-x86_64",
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "dist").mkdir()
            (root / "packaging").mkdir()
            (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.100.0"\n')
            (root / "packaging/install.sh").write_text("#!/bin/sh\n")
            (root / "packaging/install.ps1").write_text("# installer\n")
            for name in archives:
                (root / "dist" / name).write_bytes(b"archive")
                stem = name.removesuffix(".tar.gz").removesuffix(".zip")
                (root / "dist" / f"{stem}.toolchain.txt").write_text(f"Target: {stem}\nrustc 1.100.0\n")
            self.run_steps(
                root, release,
                ["Assemble the toolchain report", "Add the installers", "Checksums"], {},
            )
            names = sorted(p.name for p in (root / "dist").iterdir())
            self.assertEqual(names, sorted([*archives, "install.sh", "install.ps1", "toolchain.txt", "SHA256SUMS"]))
            self.assertEqual((root / "dist/toolchain.txt").read_text().count("rustc 1.100.0"), 3)
            self.assertFalse([l for l in (root / "dist/SHA256SUMS").read_text().splitlines() if "canopy-build-7-" in l and "toolchain" in l])
            (root / "notes.md").write_text("notes")
            manifest = subprocess.run(
                ["bash", str(ROOT / "tools/update-manifest.sh"), str(root / "dist"), "7", "https://example.invalid/page",
                 "https://example.invalid/dl", str(root / "notes.md")],
                check=True, capture_output=True, text=True, env={"PATH": os.environ["PATH"]},
            )
            platforms = json.loads(manifest.stdout)["platforms"]
            self.assertEqual({k: v["url"].rsplit("/", 1)[1] for k, v in platforms.items()},
                             {key: name for name, key in archives.items()})

    def test_the_manifest_ignores_files_that_only_mention_a_target(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = "canopy-build-7-x86_64-unknown-linux-gnu.tar.gz"
            files = {
                archive: b"archive",
                # Sorts after the archive, so it would overwrite the entry if it were matched.
                "canopy-build-7-x86_64-unknown-linux-gnu.toolchain.txt": b"report",
                "install.sh": b"#!/bin/sh\n",
                "install.ps1": b"# installer\n",
            }
            for name, contents in files.items():
                (root / name).write_bytes(contents)
            (root / "SHA256SUMS").write_text(
                "".join(f"{hashlib.sha256(c).hexdigest()}  {n}\n" for n, c in files.items())
            )
            (root / "notes.md").write_text("notes")
            result = subprocess.run(
                ["bash", str(ROOT / "tools/update-manifest.sh"), str(root), "7", "https://example.invalid/page",
                 "https://example.invalid/dl", str(root / "notes.md")],
                check=True, capture_output=True, text=True, env={"PATH": os.environ["PATH"]},
            )
            platforms = json.loads(result.stdout)["platforms"]
            self.assertEqual(platforms, {
                "linux-x86_64": {
                    "url": f"https://example.invalid/dl/{archive}",
                    "sha256": hashlib.sha256(b"archive").hexdigest(),
                },
            })

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
