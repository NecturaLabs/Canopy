# Playbook

How to operate on Canopy beyond the code: live runs, update testing, releasing. Kept for the next
session; edit it when a step changes. Rules and product limits stay in `AGENTS.md`.

## Where things stand (2 October 2026)

- Done: releases are numbered builds (`build 412 (a1b2c3d)`, tag `build-412`, no version bump) and
  Canopy updates itself (`selfupdate.rs`, ADR 0006). Both are unreleased: no `build-*` tag exists yet.
- Next in `docs/backlog.md`: the git identity items (Settings, warning, first-launch step), then the
  rest of "Now".
- Not verified: the Update and Restart buttons clicked in a real window; a native macOS or Windows
  update (those run the installer); `cargo deny` and `shellcheck` (not installed here, CI runs them).
- `packaging/test-install.sh` fails two checks here ("interrupted install exits 130", Linux and
  macOS): they fail the same on untouched `main` in the agent's shell, which ignores SIGINT.

## Checks

`tools/check.sh` is the gate. Pieces: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `python3
tools/release.py selftest`, `python3 tools/notices.py --check` (run without `--check` after a
dependency change), `packaging/test-install.sh`.

## Running the app live (Hyprland, workspace 2)

Workspace 2 is on the second monitor (HDMI-A-1, x=2560, scale 1.5), so nothing steals the main one.

1. Isolate it. The session exports `XDG_*`; a different `HOME` alone is not enough and the app would
   write the real `~/.local/share/canopy`. Always pass `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`,
   `XDG_STATE_HOME` and `XDG_CACHE_HOME` pointing into a scratch folder under `/run/media/nectura/Programming SSD/tmp`.
2. Launch without moving the user's focus:
   `hyprctl dispatch "hl.dsp.exec_cmd(\"[workspace 2 silent] env -u WAYLAND_DISPLAY DISPLAY=:0 HOME=... canopy REPO\")"`
   (Hyprland 0.56 takes Lua; the old `exec [workspace 2 silent]` syntax fails.) `-u WAYLAND_DISPLAY`
   makes it an XWayland window, which `xdotool` can see.
3. Settings are `$XDG_DATA_HOME/io.necturalabs.canopy/app.ron`, written only on a graceful close
   (`hyprctl dispatch "hl.dsp.window.close({ window = 'address:0x...' })"`). To skip the consent
   dialogs run once, close, then edit key `canopy-settings-v1`: `accepted_terms:"<TERMS_VERSION>"`,
   `accepted_privacy:"<PRIVACY_VERSION>"`, `crash_reports:Some(false)`, `seen_intro:true`.
4. Look: `grim -g "X,Y WxH" out.png` with the window's `at` and `size` from `hyprctl clients -j`,
   then read the PNG (it is 1.5x the logical size).
5. Drive: plain keys with `wtype` work after the window is focused (move the cursor onto it with
   `hl.dsp.cursor.move({ x = , y = })`, then `hl.dsp.focus({ window = 'address:...' })`; check
   `hyprctl activewindow -j` before typing, or the keys go to the user's browser). Ctrl chords (the
   palette) and mouse clicks through `wtype` and `xdotool` did not reach the window; a way that does
   is still to be found. Single keys are app shortcuts, so typing text outside a field runs them.
6. Clean up: close the window, kill helpers, delete the scratch folder, and remove any file the run
   wrote outside it.

Never `pkill -f` a pattern that appears in your own command line: it kills the shell.

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
   XDG_DATA_HOME=... sh packaging/install.sh --version 1000`. Run it on workspace 2; it offers 1001.
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
  `install.ps1` and the releases `canopy-build-<n>`. Today it still serves the semantic-version
  files: `latest.json` 0.1.0 (no builds), `beta.json` 0.2.1-beta.1, and the installers of that beta.
  Its schema and README already accept build numbers (2 October 2026).
- The first numbered release is a handoff of formats. The new installers read only build numbers and
  treat a semantic-version manifest as absent, and the Release workflow replaces the installers in
  the feed only for a stable release. So publish the first numbered build as stable, or copy
  `packaging/install.sh` and `install.ps1` to `canopy/` in the same session as a beta, and never
  before the manifest exists: replacing them earlier breaks `sh -s -- --beta` for the current beta.
  Copies of Canopy older than the first numbered build cannot read the new manifest and report an
  update-check error until reinstalled with the one-line installer.
- `APP_UPDATES_TOKEN` must exist in the Canopy repository's secrets or the feed step is skipped.
