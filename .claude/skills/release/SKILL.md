---
name: release
description: Cut a Canopy release (vX.Y.Z or a beta). Use when asked to release, publish, tag or bump the version. Covers what to check before, the manually triggered Create release workflow (semantic version from Conventional Commits, changelog, tag), the Release workflow that builds, publishes and updates the update feed, and verifying the result. Dispatching it publishes a release, so confirm the version with the user first.
---

# Release

Nobody bumps a version or tags by hand. Two workflows in `.github/workflows/` do it, driven by
`tools/release.py`; read them when something is unclear, they are commented.

| Workflow | Trigger | Does |
|---|---|---|
| `create-release.yml` ("Create release") | Manual, on `main` only | Picks the version, bumps files, commits `chore(release): vX.Y.Z` to `main`, tags `vX.Y.Z`, then calls Release on the tag |
| `release.yml` ("Release") | Called by Create release, or manual with an existing tag | Builds every platform, publishes the GitHub release, points the update feed at it |

A pushed tag alone starts nothing, so a release cannot run twice by accident.

## How the version is chosen

`python3 tools/release.py plan [--bump auto|patch|minor|major] [--pre beta]` looks at the
Conventional Commits since the last stable `v*` tag (before the first tag: since the last change
to the workspace version):

- `feat` bumps minor; `fix`, `perf`, `revert` and `build(deps)` bump patch; a `!` after the type
  or a `BREAKING CHANGE:` footer bumps major (minor while the version is 0.x).
- `docs`, `ci`, `chore`, `test`, `refactor`, `style` and plain `build` bring no release. With
  nothing releasable the workflow stops with a notice unless a bump is forced.
- `--pre beta` makes `X.Y.Z-beta.N`, N one past the newest existing beta; it goes to the beta
  update channel and is marked a pre-release. The `[Unreleased]` notes are copied into it and kept
  for the stable release.

`plan` only reads git and prints the version: it is safe to run locally. `apply` rewrites
`Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` and the AppStream metainfo; run it only in a throwaway
clone, never in the working tree, since the workflow does it. `python3 tools/release.py selftest`
is part of `tools/check.sh` and CI.

## Before dispatching

1. Confirm the version and channel with the user: run `python3 tools/release.py plan` on an
   up-to-date `main` (add `--bump` or `--pre beta` to match what you will choose) and show the
   result. A release is public and cannot be unpublished cleanly.
2. `main` is clean, up to date and has a green CI run on its tip: the workflow refuses otherwise
   (`gh run list --workflow ci.yml --branch main`). CI includes cargo-deny and the licence-notice
   check, so a dependency change must already have regenerated
   `docs/legal/third-party-notices.txt` (`python3 tools/notices.py`).
3. `CHANGELOG.md` `[Unreleased]` reads well for users. The workflow moves it into the new
   version's section, and only generates one from commit subjects when `[Unreleased]` is empty.
4. The README images in `docs/screenshots/` match the UI (`screenshot` skill) and the legal
   documents match what the build sends (`docs/legal/`).
5. Settings the workflows need: secret `APP_UPDATES_TOKEN` (Contents read/write on
   `NecturaLabs/app-updates` only; without it the release is published but the feed step is
   skipped with a warning and running copies are not offered the update); `RELEASE_TOKEN` only if
   `main` is protected against direct pushes; variable `CANOPY_SENTRY_DSN` overrides the built-in
   crash-report DSN (`off` builds without crash reporting; unset keeps the built-in one);
   variable `USE_SELF_HOSTED=0` moves the builds to GitHub-hosted runners.

## Procedure

1. Dry run, which reports the version and changes nothing:

   ```sh
   gh workflow run create-release.yml --ref main -f bump=auto -f channel=stable -f dry-run=true
   ```

   Read the "Next version" notice in the run (`gh run watch`).
2. After the user confirms that version, run it for real (`-f dry-run=false`; `-f channel=beta`
   for a pre-release; `-f bump=patch|minor|major` only to override the commits).
3. The workflow then runs `tools/release.py apply`, commits and tags, and calls Release, which:
   - builds `canopy-app` with `--locked` for `x86_64-unknown-linux-gnu` (in an Ubuntu 22.04
     container on the self-hosted runner, with a check that it needs no newer than glibc 2.35),
     `aarch64-unknown-linux-gnu` (cross-compiled in the same container with Ubuntu's aarch64 GCC,
     same glibc check), `aarch64-apple-darwin`, `x86_64-apple-darwin` (cross-compiled on the
     Apple silicon runner) and `x86_64-pc-windows-msvc`; the Linux and Windows builds use the
     self-hosted runners from an empty workspace and job-local toolchain, macOS runs hosted. The
     two cross-compiled targets are `experimental`: their failure shows as a failed job but does
     not stop the release, and the installer reports that platform as having no build;
   - packages `canopy-<version>-<target>.tar.gz` (Linux, macOS) or `.zip` (Windows):
     `README.md`, `LICENSE`, `legal/` (the Privacy Policy, Terms of Service and third-party
     notices) and the program: on Linux the binary, desktop entry, AppStream metainfo and icons;
     on macOS an ad-hoc signed `Canopy.app` (from `packaging/macos/Info.plist`, `.icns` made from
     `assets/icon-512.png`); on Windows `canopy.exe`;
   - publishes the GitHub release `vX.Y.Z` with those archives, `install.sh`, `install.ps1`, a
     `SHA256SUMS` file and the changelog section as notes (`prerelease` for a `-beta.N` tag);
   - copies them to a release `canopy-vX.Y.Z` in `NecturaLabs/app-updates` and commits
     `canopy/latest.json` (`canopy/beta.json` for a pre-release) through
     `tools/update-manifest.sh`; a stable release also commits `packaging/install.sh` and
     `packaging/install.ps1` as `canopy/install.sh` and `canopy/install.ps1`, the files the
     one-line install commands fetch. A channel is never moved backwards, and a pre-release never
     replaces the installers.

## Verify

- `gh run list --workflow create-release.yml` and `--workflow release.yml` succeeded, including
  `Publish to the update feed` (it must not have warned that the token is missing).
- `gh release view vX.Y.Z` lists five archives (three if an experimental build failed: check
  why), `install.sh`, `install.ps1` and `SHA256SUMS`; the notes match the changelog.
- Download the Linux archive: `sha256sum -c` against `SHA256SUMS`, `./canopy --version` prints
  X.Y.Z, and the archive holds the desktop entry, metainfo, icons and `legal/`.
- The manifest on the channel names X.Y.Z and the platform URLs resolve (`raw.githubusercontent.com`
  `/NecturaLabs/app-updates/main/canopy/latest.json`).
- For a stable release, `canopy/install.sh` and `canopy/install.ps1` on app-updates' `main` match
  `packaging/` at the tag, and the one-liner installs X.Y.Z in a throwaway account or VM:
  `curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh`
  then `~/.local/opt/canopy/bin/canopy --version`. Never point it at a real user's home on a
  shared machine. `packaging/README.md` describes what the installers do.

## If something fails

- Create release stops before tagging (not on `main`, CI not green, `vX.Y.Z` already exists):
  fix the cause and dispatch it again; nothing was changed.
- A build or publish job fails after the tag exists: fix the runner or workflow problem and run
  Release by hand with that tag (`gh workflow run release.yml -f tag=vX.Y.Z`). Do not run Create
  release again: it would compute a new version. Release refuses a tag that does not match the
  workspace version in `Cargo.toml`, or that has no `CHANGELOG.md` section.
- The code is wrong after a release was published: never move or delete a published tag. Merge the
  fix and cut a new patch version.
- A tag that never produced a published release may be deleted
  (`git push origin :refs/tags/vX.Y.Z`) once the cause is fixed on `main`, with the user's
  approval; the release commit stays in history.
