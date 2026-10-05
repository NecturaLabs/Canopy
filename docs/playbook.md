# Playbook

How to operate on Canopy beyond the code: live runs, update testing, releasing. Kept for the next
session; edit it when a step changes. Rules and product limits stay in `AGENTS.md`.

What is open lives in `docs/backlog.md`, not here; a release's numbered build is `git rev-list
--count` on `main` (`release` skill).

## Checks

`tools/check.sh` is the gate. Pieces: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `python3
tools/release.py selftest`, `python3 tools/notices.py --check` (run without `--check` after a
dependency change), `python3 tools/check-docs.py`, `tools/vendor-eframe.sh --check` (network
access), `python3 tools/stall-check.py`, `packaging/test-install.sh`.

- The installer test passes with a `TMPDIR` that contains a space. It used to fail two
  "interrupted install exits 130" checks there, which looked like an ignored SIGINT and was the
  space.
- Keep `TMPDIR` on a filesystem that accepts non-UTF-8 file names. On NTFS the test
  `non_utf8_folders_are_listed_and_never_staged_as_gitlinks` fails with "Invalid argument", and
  `check.sh` stops there (`CONTRIBUTING.md`, `docs/backlog.md`).
- `cargo test` stops at the first test binary that fails, so later binaries never run; add
  `--no-fail-fast` to see every failure in one run.
- `shellcheck` and `cargo deny` run only when installed; CI runs them either way.

## Running the app live (private Xvfb)

Never open a Canopy window on the owner's Hyprland session. A launch that still has
`WAYLAND_DISPLAY` set opens there. A window on a hidden workspace gets no frame callbacks; Canopy
used to block in its buffer swap then and Hyprland raised "Application Not Responding" for it
(ADR 0007). `tools/stall-check.py` tests that safely, on a stand-in compositor. Run the app on its own
X display instead: `xorg-server-xvfb` and `xorg-xauth` (Arch) are installed, `xdotool` and
ImageMagick's `import` do the rest, and `tools/screenshot.sh` already prefers `xvfb-run`.

1. Isolate it. The session exports `XDG_*` and the real `~/.gitconfig` is read by git, so a wrapper
   script must export `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`,
   `XDG_STATE_HOME` and `TMPDIR` pointing into a scratch folder under
   `/run/media/nectura/Programming SSD/tmp`, `GIT_CONFIG_GLOBAL` pointing at an empty file there,
   `GIT_CONFIG_NOSYSTEM=1`, unset `GIT_AUTHOR_*`, `GIT_COMMITTER_*` and `EMAIL`, and then
   `exec target/release/canopy "$@"`. Run it only on a fixture (`tools/mkfixture.sh`).
2. Start the display and launch, with Wayland unset:
   `Xvfb :77 -screen 0 1440x900x24 -nolisten tcp &` then
   `env -u WAYLAND_DISPLAY DISPLAY=:77 LIBGL_ALWAYS_SOFTWARE=1 wrapper.sh FIXTURE &`. Add
   `CANOPY_LOG=canopy=debug,canopy_core=debug` to check what is logged. Check with
   `hyprctl clients -j` that no Canopy window exists on the real session.
3. Drive it. There is no window manager, so keyboard focus must be given explicitly:
   `xdotool windowfocus $(xdotool search --name Canopy | tail -1)` before typing. Then
   `xdotool mousemove X Y click 1`, `xdotool type --delay 40 -- TEXT` and `xdotool key ctrl+a`
   (clicks, typing and Ctrl chords all reach the window). Single keys are app shortcuts, so type
   text only after clicking into a field.
4. Look: `import -window root out.png`, then read the PNG (1440x900, scale 1). For the
   pull and merge requests view use `tools/screenshot.sh FIXTURE out.png --demo-requests SCENARIO`
   (it implies `--view requests`): invented requests, no network, only under `--screenshot`, in a
   repository without a GitHub or GitLab remote. Scenarios are in `tools/README.md`.
5. Settings are `$XDG_DATA_HOME/io.necturalabs.canopy/app.ron`, written on a graceful close
   (`xdotool key ctrl+q`) and by the periodic autosave. A fresh scratch folder starts at the real
   first-launch path: terms ("I agree"), crash reports ("Don't send"), then the Git identity
   question. To skip the dialogs edit key `canopy-settings-v1`: `accepted_terms:"<TERMS_VERSION>"`,
   `accepted_privacy:"<PRIVACY_VERSION>"`, `crash_reports:Some(false)`, `seen_intro:true`,
   `identity_asked:true`.
6. Clean up: quit with Ctrl+Q, kill `Xvfb` by its PID, delete the scratch folder, and remove any
   file the run wrote outside it.

Never `pkill -f` a pattern that appears in your own command line: it kills the shell. Use
`pgrep -x canopy` to see whether the app is still running.

## Testing an update end to end, locally

1. Build two binaries with different numbers and a local feed: `CANOPY_BUILD_NUMBER=1000
   CANOPY_COMMIT=1000aaa CANOPY_UPDATE_FEED=http://127.0.0.1:8765/canopy
   CANOPY_UPDATE_DOWNLOADS=http://127.0.0.1:8765/releases/download cargo build -p canopy-app
   --release`, then again with 1001 (the feed and downloads are compile-time settings; loopback HTTP
   is accepted nowhere else).
2. Lay out a fake feed folder like `packaging/test-install.sh` does: `canopy/latest.json` (`version`
   `1001`, the platform URL and its SHA-256), `releases/download/canopy-build-N/` holding
   `canopy-build-N-x86_64-unknown-linux-gnu.tar.gz` (a folder of that name with `canopy`, README,
   LICENSE, `legal/`, the desktop entry, metainfo, icon svg and png), `install.sh` and `SHA256SUMS`.
   Serve it with `python3 -m http.server 8765 --bind 127.0.0.1`.
3. Install build 1000 with the real installer: `CANOPY_INSTALL_BASE_URL=http://127.0.0.1:8765 HOME=...
   XDG_DATA_HOME=... sh packaging/install.sh --version 1000`. Run it on a private Xvfb (see
   "Running the app live"); it offers 1001.
4. To exercise the installer-side steps without the buttons, add a temporary `#[ignore]` test that
   fetches the manifest, builds `Plan::from_manifest` and calls `selfupdate::install_managed` on
   that folder, run it with `-- --ignored --nocapture`, check the folder (`VERSION`, receipt, the
   registered desktop entry and icons identical to the copies in the folder, `uninstall.sh` equal to
   the release's `install.sh`, no `.staging.*`, `.previous.*` or `.lock`), run its `uninstall.sh
   --uninstall`, and delete the test.

## Releasing, and the app-updates repository

- Read the `release` skill first. A release publishes to users: confirm the build and channel with
  the owner, then `gh workflow run create-release.yml --ref main -f channel=beta -f dry-run=true`,
  and for real without `dry-run`. Nothing is committed to `main`; the tag is `build-<n>`.
- `NecturaLabs/app-updates` (public) holds `canopy/latest.json`, `beta.json`, `install.sh`,
  `install.ps1` and the releases `canopy-build-<n>`. The build each channel serves is the `version`
  in its manifest: `gh api -H 'Accept: application/vnd.github.raw'
  repos/NecturaLabs/app-updates/contents/canopy/latest.json` (and `beta.json`). A stable release
  updates only `latest.json` and the installers: to keep the beta channel level with it, commit the
  same manifest as `canopy/beta.json` afterwards.
- Copies of Canopy older than the first numbered build cannot read the numbered manifest and report
  an update-check error until reinstalled with the one-line installer.
- The Release workflow's Linux builds run as root in an Ubuntu 22.04 container: the first run of
  build 119 failed there (git's "dubious ownership"), so its tag was deleted and the fix released
  as build 120. A release job's composite actions come from the tag's own commit, so a workflow fix
  needs a new build, not a rerun of the old tag.
- `APP_UPDATES_TOKEN` must exist in the Canopy repository's secrets or the feed step is skipped.
