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

The release notes are those commits grouped as Security (a `security` scope or a `Security:`
footer), Breaking changes (`!` or a `BREAKING CHANGE:` footer), Added (`feat`), Fixed (`fix`,
`revert`) and Changed (`perf`, `build`), so a releasable commit's subject is user-facing text. Before the first
`build-*` tag the notes are the `[Unreleased]` section of `CHANGELOG.md`. The Release workflow
embeds them in the build for the "What's new" dialog and publishes them with the release and in
the feed manifest.

The channel is chosen per release: `stable` goes to `latest.json`, `beta` to `beta.json` and the
GitHub release is marked a pre-release. A build is one or the other; a beta is not promoted, the
next build is released as stable. A stable build also becomes the beta channel's manifest (the
same feed release) unless beta already offers a newer build.

`plan` and `number` only read git and print: they are safe to run locally.
`python3 tools/release.py selftest` is part of `tools/check.sh` and CI.

## Before dispatching

0. Bring the work to `main`: open a pull request from `dev` into `main` (the only source the
   required "Source branch" check accepts), let its checks pass and merge it.
1. Confirm the build and channel with the user: run `python3 tools/release.py plan` and
   `python3 tools/release.py notes` on an up-to-date `main` (full history, not a shallow clone) and
   show the result. A release is public and cannot be unpublished cleanly.
2. `main` is clean, up to date and has a green CI run on its tip (`gh run list --workflow ci.yml
   --branch main`). The workflow waits up to 25 minutes for a run still in progress and refuses
   unless it passed. Merging a branch that was up to date with `main` gives a
   commit with the branch's tree, whose CI run takes under a minute: it takes the branch's passing
   run as proof and runs only the documentation and site checks. CI includes cargo-deny and the
   licence-notice check, so a dependency change must already have regenerated
   `docs/legal/third-party-notices.txt` (`python3 tools/notices.py`).
3. The notes read well for users: they are the commit subjects (see above), so a poorly worded
   `feat` or `fix` subject is what users read. Reword by releasing from a later commit, never by
   rewriting `main`.
4. The README images in `docs/screenshots/` match the UI (`screenshot` skill) and the legal
   documents match what the build sends (`docs/legal/`).
5. The user simulation review ran after the release's last planned change and every finding is
   fixed (`tools/usersim.sh`, `docs/playbook.md`).
6. Settings the workflows need: secret `APP_UPDATES_TOKEN` in the `release` environment (Contents
   read/write on `NecturaLabs/app-updates` only; the environment admits only `main`, and only the
   feed jobs declare it; without it the release is published but the feed step is
   skipped with a warning and running copies are not offered the update); `RELEASE_TOKEN` only if
   tags are protected against creation by the workflow's own token; variable `CANOPY_SENTRY_DSN` overrides the built-in
   crash-report DSN (`off` builds without crash reporting; unset keeps the built-in one);
   and the CI platform
   variables `CI_LINUX`, `CI_MACOS` and `CI_WINDOWS` decide which platforms Release builds
   (`CI_MACOS` and `CI_WINDOWS` are off unless `1`). Read them with `gh variable list` and tell
   the user which platforms the release will ship before dispatching.

## Procedure

1. Confirm the build and channel with the user (`python3 tools/release.py plan`), then run it for
   real: no dry run first (the owner's rule). `-f channel=beta` for the beta channel; `-f
   force=true` only to release without a releasable commit. A request that already names the
   channel ("make a beta pre-release") is that confirmation once the number has been shown.

   ```sh
   gh workflow run create-release.yml --ref main -f channel=stable -f dry-run=false
   ```

   Find the run with `gh run list --workflow create-release.yml --limit 1` and watch it with
   `gh run watch <id> --exit-status` (several minutes). A run that ends green with "Nothing to
   release" made no build. A stable build also becomes the beta channel's manifest, so one stable
   release serves both channels.
2. The workflow then tags `build-<number>` and calls Release, which:
   - checks that the tag is `build-<n>` on a commit of `main` with `n` commits behind it, and
     writes the release notes;
   - builds `canopy-app` with `--locked` for the platforms its `platforms` job turns on from the
     CI variables, like CI's test job: Linux unless `CI_LINUX` is `0`, macOS only when `CI_MACOS`
     is `1`, Windows only when `CI_WINDOWS` is `1` (it fails if all three are off). Linux is
     `x86_64-unknown-linux-gnu` (on an Ubuntu 22.04 runner, with a check that it needs no newer
     than glibc 2.35) and `aarch64-unknown-linux-gnu` (cross-compiled on the same runner with
     Ubuntu's aarch64 GCC, same glibc check); macOS is
     `aarch64-apple-darwin` and `x86_64-apple-darwin` (cross-compiled on the Apple silicon
     runner); Windows is `x86_64-pc-windows-msvc`. Every build runs on a GitHub-hosted runner
     with a job-local toolchain. The
     two cross-compiled targets are `experimental`: they build in a job of their own (20 minute
     limit once a runner starts it) that may fail, so a failure or a hang never holds back the
     release beyond that limit; a job still queued for a runner does, until you cancel the run and
     dispatch Release by hand. Publishing waits for it and ships the archives that built, warning about each target
     that did not; the installer reports that platform as having no build;
   - publishes both releases (here and the feed's in app-updates) as drafts, attaches every file,
     then publishes them: both repositories have immutable releases on, so a published release's
     files and tag can never change;
   - builds with the number, the commit's short hash and the notes embedded
     (`crates/canopy-app/build.rs`), so `canopy --version` prints `canopy build <n> (<hash>)`;
   - packages `canopy-build-<number>-<target>.tar.gz` (Linux, macOS) or `.zip` (Windows):
     `README.md`, `LICENSE`, `legal/` (the Privacy Policy, Terms of Service and third-party
     notices) and the program: on Linux the binary, desktop entry, AppStream metainfo and the PNG icon
     (`io.necturalabs.Canopy.png`, from `assets/icon-256.png`);
     on macOS an ad-hoc signed `Canopy.app` (from `packaging/macos/Info.plist`, `.icns` made from
     `assets/icon-512.png`); on Windows `canopy.exe`;
   - publishes the GitHub release `build-<number>` with those archives, `install.sh`,
     `install.ps1`, a `SHA256SUMS` file and the notes (`prerelease` for the beta channel);
   - copies them to a release `canopy-build-<number>` in `NecturaLabs/app-updates` and commits
     `canopy/latest.json` (`canopy/beta.json` for the beta channel; a stable release writes both) through
     `tools/update-manifest.sh`; a stable release also commits `packaging/install.sh` and
     `packaging/install.ps1` as `canopy/install.sh` and `canopy/install.ps1`, the files the
     one-line install commands fetch. A channel is never moved backwards (a manifest without a
     build number counts as 0), and a beta build never replaces the installers;
   - removes the previous releases (job "Remove the previous releases", `tools/prune-releases.py`):
     first it writes every Canopy feed release's notes into app-updates' canopy/CHANGELOG.md,
     which the release page links to, then deletes every Canopy release in app-updates (with its
     tag) and every `build-*` release here (keeping the tag, which `release.py` reads), except
     this build and the ones `latest.json` and `beta.json` offer. A release whose notes are not
     in the changelog is never deleted. By hand: `python3 tools/prune-releases.py run --feed-dir
     <app-updates clone> --keep <n> --dry-run`, then without `--dry-run`.

## Verify

- The Create release run succeeded (`gh run view <id>`). Release runs inside it as a reusable
  workflow, so its jobs appear there as "Build and publish / …", not under
  `gh run list --workflow release.yml` (that lists only manual reruns). Every job is green except
  possibly an experimental build (allowed to fail),
  and `Publish to the update feed` did not warn that the token is missing.
- `git fetch --tags` locally; the tag `build-<number>` is on the commit you meant to release.
- `gh release view build-<number>` lists one archive per target built (two for Linux, two for macOS,
  one for Windows, for the platforms that were on; one fewer for each experimental build that
  failed: check why), `install.sh`,
  `install.ps1` and `SHA256SUMS`; the notes match
  `tools/release.py notes build-<number>`; `gh release view build-<number> --json isPrerelease`
  is `true` for a beta and `false` for a stable release.
- If Linux was built, download its archive: `gh attestation verify <archive> --repo NecturaLabs/Canopy`
  passes, `sha256sum -c` against `SHA256SUMS`,
  `./canopy --version` prints `canopy build <number> (<hash>)`, and the archive holds the desktop
  entry, metainfo, `io.necturalabs.Canopy.png` and `legal/`.
- The manifest on the channel names `<number>` as its `version` and the platform URLs resolve
  (`https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/latest.json`, or
  `beta.json` for a beta).
- For a stable release, `canopy/install.sh` and `canopy/install.ps1` on app-updates' `main` match
  `packaging/` at the tag, and the one-liner installs build `<number>` in a throwaway account or VM:
  `curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh`
  then `~/.local/opt/canopy/bin/canopy --version`. Never point it at a real user's home on a
  shared machine. `packaging/README.md` describes what the installers do.

## If something fails

- Create release stops before tagging (not on `main`, CI failed or still running after 25
  minutes, nothing releasable, `build-<number>` already exists): fix the cause and dispatch it
  again; nothing was changed.
- A build or publish job fails after the tag exists: fix the runner or workflow problem and run
  Release by hand with that tag and its channel (`gh workflow run release.yml -f tag=build-<n> -f
  channel=stable|beta`). Do not run Create release again: it would stop, since HEAD is a tagged
  build. Release refuses a tag whose number is not the commit count of its commit, or whose
  commit is not on `main`.
- An experimental build failed or ran out of time: the release is out without it, and an
  immutable release cannot take it later. It ships with the next build.
- A publish job failed: rerun the failed jobs (`gh run rerun <id> --failed`). A release still a
  draft is completed; one already published is left exactly as it is (immutable) and the job goes
  on to the steps after it, such as the feed manifest. Files missing from a published release can
  only ship with the next build.
- The code is wrong after a release was published: never move or delete a published tag. Merge the
  fix and cut a new build.
- A tag that never produced a published release may be deleted
  (`git push origin :refs/tags/build-<n>`) once the cause is fixed on `main`, with the user's
  approval.
