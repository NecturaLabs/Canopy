---
name: release
description: Cut a Canopy release (vX.Y.Z). Use when asked to release, publish, tag or bump the version. Covers the version bump, CHANGELOG and AppStream metainfo, checks, tagging, the GitHub release workflow and verifying its artifacts. Pushing the tag publishes a release, so confirm the version with the user before pushing.
---

# Release

Canopy follows Semantic Versioning. Pushing a `v*` tag runs `.github/workflows/release.yml`,
which builds Linux (x86_64, tar.gz with the desktop entry, metainfo and icon), macOS (Apple
silicon, tar.gz) and Windows (x86_64, zip) binaries and publishes a GitHub release whose notes
come from the matching `CHANGELOG.md` section.

## Procedure

1. Confirm the version number with the user. Start from an up-to-date `main` with a clean working
   tree and green CI.
2. Bump `version` under `[workspace.package]` in the root `Cargo.toml`. Run
   `cargo update --workspace` so `Cargo.lock` picks up the new version (the release build uses
   `--locked`).
3. `CHANGELOG.md`: rename `[Unreleased]` to `[X.Y.Z] - YYYY-MM-DD`, add a fresh empty
   `[Unreleased]` above it, and update the comparison links at the bottom.
4. `packaging/linux/io.necturalabs.Canopy.metainfo.xml`: add a `<release version="X.Y.Z"
   date="YYYY-MM-DD">` entry at the top of `<releases>` with a one-paragraph summary. Validate
   with `appstreamcli validate --no-net` if available.
5. Run `tools/check.sh`. Take fresh screenshots if the UI changed (`screenshot` skill) and update
   `docs/screenshots/` if the README images are out of date.
6. Commit: `chore(release): vX.Y.Z`. Open a pull request or push to `main` as the team prefers,
   and wait for CI.
7. Tag the release commit with an annotated tag and push it, after the user confirms:

   ```sh
   git tag -a vX.Y.Z -m "Canopy X.Y.Z"
   git push origin vX.Y.Z
   ```

8. Verify: the Release workflow succeeds (`gh run list --workflow release.yml`, `gh run watch`),
   and the release has three archives plus `SHA256SUMS` (`gh release view vX.Y.Z`). Download the
   Linux archive, check that `canopy --version` prints X.Y.Z and that the archive contains the
   desktop entry, metainfo and icons.

## If something fails

The workflow refuses a tag that does not match the workspace version or has no CHANGELOG section.
Fix the problem on `main`, delete the tag locally and remotely (`git push origin :refs/tags/vX.Y.Z`,
only for a release that was never published), and tag again. Never move a tag that has a
published release; release a patch version instead.
