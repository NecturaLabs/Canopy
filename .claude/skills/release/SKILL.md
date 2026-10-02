---
name: release
description: Cut a Canopy release (a numbered build, stable or beta). Use when asked to release, publish or tag. Covers what to check before, the manually triggered Create release workflow (build number from the commit count, tag), the Release workflow that builds, publishes and updates the update feed, and verifying the result. Dispatching it publishes a release, so confirm the build with the user first.
---

# Release

A release is a build. Its number is `git rev-list --count` of its commit on `main`, and the release
is the tag `build-<number>` on that commit. Nobody bumps a version, commits or tags by hand: no
version is bumped and nothing is committed to `main`. Two workflows in `.github/workflows/` do it,
driven by `tools/release.py`; read them when something is unclear, they are commented.

| Workflow | Trigger | Does |
|---|---|---|
| `create-release.yml` ("Create release") | Manual, on `main` only | Decides the build number, checks CI passed on the commit, tags it `build-<number>`, then calls Release on the tag |
| `release.yml` ("Release") | Called by Create release, or manual with an existing tag and its channel | Checks the tag, builds the platforms the CI variables turn on, publishes the GitHub release, points the update feed at it |

A pushed tag alone starts nothing, so a release cannot run twice by accident.

## How the build is chosen

`python3 tools/release.py plan [--force]` looks at the Conventional Commits since the last
`build-*` tag (before the first tag: all of them) and prints HEAD's build number when one of them
changes what users get: `feat`, `fix`, `perf`, `revert`, `build(deps)`, or a `!` or `BREAKING
CHANGE:` footer. `docs`, `ci`, `chore`, `test`, `refactor`, `style` and plain `build` bring no
release: with nothing releasable, or when HEAD already is a tagged build, the workflow stops with a
notice unless "force" is chosen.

The release notes are those commits grouped as Added (`feat`), Fixed (`fix`, `revert`) and Changed
(`perf`, `build`), so a releasable commit's subject is user-facing text. Before the first
`build-*` tag the notes are the `[Unreleased]` section of `CHANGELOG.md`. The Release workflow
embeds them in the build for the "What's new" dialog and publishes them with the release and in
the feed manifest.

The channel is chosen per release: `stable` goes to `latest.json`, `beta` to `beta.json` and the
GitHub release is marked a pre-release. A build is one or the other; a beta is not promoted, the
next build is released as stable.

`plan` and `number` only read git and print: they are safe to run locally.
`python3 tools/release.py selftest` is part of `tools/check.sh` and CI.

## Before dispatching

1. Confirm the build and channel with the user: run `python3 tools/release.py plan` and
   `python3 tools/release.py notes` on an up-to-date `main` (full history, not a shallow clone) and
   show the result. A release is public and cannot be unpublished cleanly.
2. `main` is clean, up to date and has a green CI run on its tip: the workflow refuses otherwise
   (`gh run list --workflow ci.yml --branch main`). CI includes cargo-deny and the licence-notice
   check, so a dependency change must already have regenerated
   `docs/legal/third-party-notices.txt` (`python3 tools/notices.py`).
3. The notes read well for users: they are the commit subjects (see above), so a poorly worded
   `feat` or `fix` subject is what users read. Reword by releasing from a later commit, never by
   rewriting `main`.
4. The README images in `docs/screenshots/` match the UI (`screenshot` skill) and the legal
   documents match what the build sends (`docs/legal/`).
5. Settings the workflows need: secret `APP_UPDATES_TOKEN` (Contents read/write on
   `NecturaLabs/app-updates` only; without it the release is published but the feed step is
   skipped with a warning and running copies are not offered the update); `RELEASE_TOKEN` only if
   tags are protected against creation by the workflow's own token; variable `CANOPY_SENTRY_DSN` overrides the built-in
   crash-report DSN (`off` builds without crash reporting; unset keeps the built-in one);
   variable `USE_SELF_HOSTED=0` moves the builds to GitHub-hosted runners; and the CI platform
   variables `CI_LINUX`, `CI_MACOS` and `CI_WINDOWS` decide which platforms Release builds
   (`CI_MACOS` and `CI_WINDOWS` are off unless `1`). Read them with `gh variable list` and tell
   the user which platforms the release will ship before dispatching.

## Procedure

1. Dry run, which reports the version and changes nothing:

   ```sh
   gh workflow run create-release.yml --ref main -f channel=stable -f dry-run=true
   ```

   Read the "Next build" notice in the run (`gh run watch`).
2. After the user confirms that build, run it for real (`-f dry-run=false`; `-f channel=beta` for
   the beta channel; `-f force=true` only to release without a releasable commit).
3. The workflow then tags `build-<number>` and calls Release, which:
   - checks that the tag is `build-<n>` on a commit of `main` with `n` commits behind it, and
     writes the release notes;
   - builds `canopy-app` with `--locked` for the platforms its `platforms` job turns on from the
     CI variables, like CI's test job: Linux unless `CI_LINUX` is `0`, macOS only when `CI_MACOS`
     is `1`, Windows only when `CI_WINDOWS` is `1` (it fails if all three are off). Linux is
     `x86_64-unknown-linux-gnu` (in an Ubuntu 22.04 container on the self-hosted runner, with a
     check that it needs no newer than glibc 2.35) and `aarch64-unknown-linux-gnu` (cross-compiled
     in the same container with Ubuntu's aarch64 GCC, same glibc check); macOS is
     `aarch64-apple-darwin` and `x86_64-apple-darwin` (cross-compiled on the Apple silicon
     runner); Windows is `x86_64-pc-windows-msvc`. The Linux and Windows builds use the
     self-hosted runners from an empty workspace and job-local toolchain, macOS runs hosted. The
     two cross-compiled targets are `experimental`: their failure shows as a failed job but does
     not stop the release, and the installer reports that platform as having no build;
   - builds with the number, the commit's short hash and the notes embedded
     (`crates/canopy-app/build.rs`), so `canopy --version` prints `canopy build <n> (<hash>)`;
   - packages `canopy-build-<number>-<target>.tar.gz` (Linux, macOS) or `.zip` (Windows):
     `README.md`, `LICENSE`, `legal/` (the Privacy Policy, Terms of Service and third-party
     notices) and the program: on Linux the binary, desktop entry, AppStream metainfo and icons;
     on macOS an ad-hoc signed `Canopy.app` (from `packaging/macos/Info.plist`, `.icns` made from
     `assets/icon-512.png`); on Windows `canopy.exe`;
   - publishes the GitHub release `build-<number>` with those archives, `install.sh`,
     `install.ps1`, a `SHA256SUMS` file and the notes (`prerelease` for the beta channel);
   - copies them to a release `canopy-build-<number>` in `NecturaLabs/app-updates` and commits
     `canopy/latest.json` (`canopy/beta.json` for the beta channel) through
     `tools/update-manifest.sh`; a stable release also commits `packaging/install.sh` and
     `packaging/install.ps1` as `canopy/install.sh` and `canopy/install.ps1`, the files the
     one-line install commands fetch. A channel is never moved backwards (a manifest without a
     build number counts as 0), and a beta build never replaces the installers.

## Verify

- `gh run list --workflow create-release.yml` and `--workflow release.yml` succeeded, including
  `Publish to the update feed` (it must not have warned that the token is missing).
- `gh release view build-<number>` lists one archive per target built (two for Linux, two for macOS,
  one for Windows, for the platforms that were on; one fewer for each experimental build that
  failed: check why), `install.sh`, `install.ps1` and `SHA256SUMS`; the notes match
  `tools/release.py notes build-<number>`.
- If Linux was built, download its archive: `sha256sum -c` against `SHA256SUMS`,
  `./canopy --version` prints `canopy build <number> (<hash>)`, and the archive holds the desktop entry, metainfo, icons and `legal/`.
- The manifest on the channel names `<number>` as its `version` and the platform URLs resolve (`raw.githubusercontent.com`
  `/NecturaLabs/app-updates/main/canopy/latest.json`).
- For a stable release, `canopy/install.sh` and `canopy/install.ps1` on app-updates' `main` match
  `packaging/` at the tag, and the one-liner installs build `<number>` in a throwaway account or VM:
  `curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh`
  then `~/.local/opt/canopy/bin/canopy --version`. Never point it at a real user's home on a
  shared machine. `packaging/README.md` describes what the installers do.

## If something fails

- Create release stops before tagging (not on `main`, CI not green, nothing releasable,
  `build-<number>` already exists): fix the cause and dispatch it again; nothing was changed.
- A build or publish job fails after the tag exists: fix the runner or workflow problem and run
  Release by hand with that tag and its channel (`gh workflow run release.yml -f tag=build-<n> -f
  channel=stable|beta`). Do not run Create release again: it would stop, since HEAD is a tagged
  build. Release refuses a tag whose number is not the commit count of its commit, or whose
  commit is not on `main`.
- The code is wrong after a release was published: never move or delete a published tag. Merge the
  fix and cut a new build.
- A tag that never produced a published release may be deleted
  (`git push origin :refs/tags/build-<n>`) once the cause is fixed on `main`, with the user's
  approval.
