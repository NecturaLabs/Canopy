# Playbook

How to operate on Canopy beyond the code: live runs, update testing, releasing. Kept for the next
session; edit it when a step changes. Rules and product limits stay in `AGENTS.md`.

What is open lives in [GitHub Issues](https://github.com/NecturaLabs/Canopy/issues), not here;
`docs/backlog.md` is the migration record. A release's numbered build is `git rev-list
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
  `non_utf8_folders_are_listed_and_never_staged_as_gitlinks` reports that the filesystem refused
  the name, with `TMPDIR` and NTFS in the explanation (`CONTRIBUTING.md`). The check script
  still runs the remaining test binaries through `--no-fail-fast` before reporting the failure.
- `cargo test` stops at the first test binary that fails, so later binaries never run;
  `check.sh` and CI's test job pass `--no-fail-fast` to see every failure in one run.
- `shellcheck` and `cargo deny` run only when installed; CI runs them either way.
- An unresolved `nectura-ui` item (`unresolved import super::kit::...`) can mean the
  `crates/nectura-ui` checkout lags the recorded pointer (`git status` shows the submodule
  modified). Another session may own that checkout: work in your own git worktree and run
  `git submodule update --init` there.
- Give every git worktree its own `CARGO_TARGET_DIR`. Cargo hashes the workspace's path crates
  by their path relative to the workspace root and judges freshness by file mtime, so a worktree
  whose files are older than another worktree's last build reuses that build: its tests then run
  against the other worktree's `nectura-ui` and fail (or pass) for the wrong reason. A target is
  about 10 GB; delete it once its branch merges, and keep 5 GB free on the drive.
- Run cargo only under your real `HOME` (or with `RUSTUP_HOME` and `CARGO_HOME` set to the real
  ones). Under a scratch `HOME` the rustup proxy installs a whole toolchain into it. Give a private
  `HOME` to the app or `git` process only.
- After a merge that `gh pr merge` refuses with "Base branch was modified", `main` moved during
  the check: merge `main` again and rerun the whole check. Merge nothing by hand while another
  branch's check runs.

## Running the app live (private Xvfb)

Never open a Canopy window on your own desktop session. A launch that still has
`WAYLAND_DISPLAY` set opens there. A window on a hidden workspace gets no frame callbacks; Canopy
used to block in its buffer swap then and Hyprland raised "Application Not Responding" for it
(ADR 0007). `tools/stall-check.py` tests that safely, on a stand-in compositor. Run the app on its own
X display instead: Xvfb (Arch `xorg-server-xvfb` and `xorg-xauth`, Debian/Ubuntu `xvfb`), `xdotool`
and ImageMagick's `import` do the rest, and `tools/screenshot.sh` already prefers `xvfb-run`.

1. Isolate it. The session exports `XDG_*` and the real `~/.gitconfig` is read by git, so a wrapper
   script must export `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`,
   `XDG_STATE_HOME` and `TMPDIR` pointing into a scratch folder (a
   `mktemp -d` directory), `GIT_CONFIG_GLOBAL` pointing at an empty file there,
   `GIT_CONFIG_NOSYSTEM=1`, unset `GIT_AUTHOR_*`, `GIT_COMMITTER_*` and `EMAIL`, and then
   `exec target/release/canopy "$@"`. Run it only on a fixture (`tools/mkfixture.sh`).
2. Start the display and launch, with Wayland unset:
   `Xvfb :77 -screen 0 1440x900x24 -nolisten tcp &` then
   `env -u WAYLAND_DISPLAY DISPLAY=:77 LIBGL_ALWAYS_SOFTWARE=1 wrapper.sh FIXTURE &`. Add
   `CANOPY_LOG=canopy=debug,canopy_core=debug` to check what is logged. Check on the
   real session (on Hyprland, `hyprctl clients -j`) that no Canopy window exists there.
3. Drive it. There is no window manager, so keyboard focus must be given explicitly:
   `xdotool windowfocus $(xdotool search --name Canopy | tail -1)` before typing. Then
   `xdotool mousemove X Y click 1`, `xdotool type --delay 40 -- TEXT` and `xdotool key ctrl+a`
   (clicks, typing and Ctrl chords all reach the window). Single keys are app shortcuts, so type
   text only after clicking into a field.
4. Look: `import -window root out.png`, then read the PNG (1440x900, scale 1). For the
   pull and merge requests view use `tools/screenshot.sh FIXTURE out.png --demo-requests SCENARIO`
   (it implies `--view requests`): invented requests, no network, only under `--screenshot`, in a
   repository without a GitHub or GitLab remote. Scenarios are in `tools/README.md`.
5. Settings are `$XDG_CONFIG_HOME/canopy/settings.json`, written atomically on a graceful close
   (`xdotool key ctrl+q`) and by the periodic autosave. A fresh scratch folder starts at the real
   first-launch path: terms ("I agree"), crash reports ("Don't send"), then the Git identity
   question. To skip the dialogs edit the `settings` object: `"accepted_terms": "<TERMS_VERSION>"`,
   `"accepted_privacy": "<PRIVACY_VERSION>"`, `"crash_reports": false`, `"seen_intro": true`,
   `"identity_asked": true`.
6. Clean up: quit with Ctrl+Q, kill `Xvfb` by its PID, delete the scratch folder, and remove any
   file the run wrote outside it.

Never `pkill -f` a pattern that appears in your own command line: it kills the shell. Use
`pgrep -x canopy` to see whether the app is still running.

## User simulation review (before every release)

A standing review: once every change planned for a release is in, and before the release is cut,
use Canopy as a user would and fix every finding before releasing. It complements the
screenshot checks and unit tests, which cannot see hover, timing, focus, menus that open and close,
or how flows feel end to end.

1. `tools/usersim.sh start [--kind showcase|conflict|linear|big] [--size WxH] [--env NAME=VALUE]...`
   builds the release binary and launches it on a private Xvfb display with its own home, XDG folders and Git
   configuration in a scratch folder, on a fixture with a local bare `origin`: never your own
   repositories, never your Canopy settings, never the desktop session. The app gets a clean
   environment; `--env` adds `TERMINAL`, `VISUAL`, `EDITOR`, `XDG_DATA_DIRS` or `XDG_CONFIG_DIRS` to
   it, to stage another machine's launcher lists (desktop entries and programs in a scratch folder).
2. Drive it with `tools/usersim.sh x ...` (xdotool: `x search --name Canopy windowfocus`, then
   `x mousemove X Y click 1`, `x key ctrl+k`, `x type --delay 40 -- TEXT`) and look with
   `tools/usersim.sh shot out.png` after every step that changes the screen.
3. Walk the first launch (terms, crash question, identity). The fixture is outside every trusted
   folder, so its repository asks "Trust this repository?": answer Trust before the flows below, or
   Don't trust to see the read-only banner. Then every change in the release, then
   the core flows around it: browse and select in History and the sidebar, context menus and
   their submenus, stage and commit, fetch, pull and push against the bare `origin`, a conflict
   (`--kind conflict`), Settings changes taking effect, notifications and the notification center,
   tooltips on hover, many tabs. Use the keyboard as well as the pointer. `restart` checks that
   settings survive a relaunch.
4. Write each finding down as it is found (what, where, a screenshot), fix it with a test where one
   can catch it, and walk the affected flow again. The release waits until every finding is fixed.
5. `tools/usersim.sh stop` quits, stops Xvfb and deletes the scratch folder.

## Testing an update end to end, locally

1. Build two binaries with different numbers and a local feed: `CANOPY_BUILD_NUMBER=1000
   CANOPY_COMMIT=1000aaa CANOPY_UPDATE_FEED=http://127.0.0.1:8765/canopy
   CANOPY_UPDATE_DOWNLOADS=http://127.0.0.1:8765/releases/download
   CANOPY_UPDATE_KEY=<public key> cargo build -p canopy-app --release`, then again with 1001 (the
   feed, downloads and key are compile-time settings; loopback HTTP is accepted nowhere else). The
   key is the last line of a minisign public key file of a throwaway key: make one as the
   `selftest` in `tools/sign-manifest.py` does (OpenSSL 3).
2. Lay out a fake feed folder like `packaging/test-install.sh` does: `canopy/latest.json` (`version`
   `1001`, the platform URL and its SHA-256, `installers` with the SHA-256 of `install.sh` and
   `install.ps1`; `tools/update-manifest.sh` writes it), signed with `CANOPY_MANIFEST_KEY=<pem>
   python3 tools/sign-manifest.py sign <public key file> canopy/latest.json`,
   `releases/download/canopy-build-N/` holding
   `canopy-build-N-x86_64-unknown-linux-gnu.tar.gz` (a folder of that name with `canopy`, README,
   LICENSE, `legal/`, the desktop entry, metainfo, icon png), `install.sh` and `SHA256SUMS`.
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

- Release enforcement uses mandatory Linux workspace tests on the exact commit, in a read-only
  GitHub-hosted job. Create release waits for them before tagging; Release runs them again before
  publishing, including a direct dispatch or rebuild of an existing tag. Failed or cancelled tests
  prevent the tag or publication. Dry runs do not run these tests and cannot tag.
- CI runs the workspace tests on Linux, macOS and Windows for every push that changes code and
  for a fork's pull request; the "CI for dev" and "CI for main" rulesets require its `Tests`
  check. `CI_LINUX`, `CI_MACOS` and `CI_WINDOWS` pick the release platforms only. The local
  pre-commit hook is an early check, while the mandatory release job is the enforced publication
  gate.
- `dev` takes a pull request only when its branch is up to date, so after each merge the other
  open pull requests need **Update branch** (`gh pr update-branch <n>`) and Dependabot's need a
  `@dependabot rebase` comment. `main` does not require it: its release merge commits never reach
  `dev`.
- Manifests are signed (ADR 0021): every `canopy/*.json` in app-updates needs a `.minisig` whose
  trusted comment is `Canopy update manifest: channel=<channel> build=<build>`, or new builds
  refuse it. The release workflow writes that form; a manifest changed by hand is re-signed with
  `CANOPY_MANIFEST_KEY=<pem> python3 tools/sign-manifest.py sign packaging/update-signing.pub
  canopy/latest.json canopy/beta.json` from a clone of app-updates (done for build 1683 on
  2026-11-18, after its content verified against the earlier signature).
- Read the `release` skill first. Work lands in `dev`; merge a pull request from `dev` into `main`
  first (ADR 0020). A release publishes to users: confirm the build and channel with
  the owner, then dispatch it once for real (`gh workflow run create-release.yml --ref main -f
  channel=beta -f dry-run=false`): no dry run first. Nothing is committed to `main`; the tag is `build-<n>`. It can
  be dispatched right after a merge: it waits for main's CI run (under a minute when the merged
  branch was up to date with `main`, since that run reuses the branch's).
- The experimental builds (aarch64 Linux, Intel macOS) may fail without failing the release: it
  waits for them (20 minute limit; build 1116 was once stuck for an hour on a hung aarch64 job,
  #144) and ships the archives that built. Releases are immutable in Canopy and app-updates: each
  is a draft until every file is attached, and nothing can be added after it is published.
- `NecturaLabs/app-updates` (public) holds `canopy/latest.json`, `beta.json`, `install.sh`,
  `install.ps1` and the releases `canopy-build-<n>`. The build each channel serves is the `version`
  in its manifest: `gh api -H 'Accept: application/vnd.github.raw'
  repos/NecturaLabs/app-updates/contents/canopy/latest.json` (and `beta.json`). A stable release
  updates `latest.json`, the installers and, unless it already offers a newer build, `beta.json`
  with the same manifest, so both channels name the same feed release. A beta release updates
  only `beta.json`.
- Copies of Canopy older than the first numbered build cannot read the numbered manifest and report
  an update-check error until reinstalled with the one-line installer.
- The Release workflow's Linux builds run as root in an Ubuntu 22.04 container: the first run of
  build 119 failed there (git's "dubious ownership"), so its tag was deleted and the fix released
  as build 120. A release job's composite actions come from the tag's own commit, so a workflow fix
  needs a new build, not a rerun of the old tag.
- `APP_UPDATES_TOKEN` must exist in the Canopy repository's `release` environment (main only) or the feed step is skipped.
  `MANIFEST_SIGNING_KEY` (the PEM Ed25519 key of `packaging/update-signing.pub`; ADR 0021) must be
  there too, or the release fails before it touches the feed; the owner keeps an offline copy.
  `NETLIFY_AUTH_TOKEN` is the `site`
  environment's (main only). A secret added as a repository or organisation secret would reach
  any branch's workflow: keep them in the environments.

For the expanded commit-details layout, render with `--overlay commit-expanded`; it activates
the header’s Expand button by its accessible name. Use `--size 960x640` for compact details.

Use `--overlay commit-compact --size 960x400` to select Commit details in the compact
History layout through its accessible control.

Use `--overlay amend-commit` with a clean fixture to open the amend composer through its
secondary action and capture the message read by the worker.

Use `--overlay settings-git-pull` or `--overlay settings-repository-pull` to capture the
Pull strategy option list through its accessible control. `--overlay settings-repository-diffs`
opens the repository's Diffs group.
