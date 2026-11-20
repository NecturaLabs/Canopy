# Tools

Scripts for development, verification, installation and releases. They run from any directory
and print usage with `-h` or when called without required arguments.

| Script | Purpose |
|---|---|
| `check.sh` | The local CI: formatting, strict Clippy, unsafe-code policy, source-header and UI-colour guards, workspace tests with isolated Git configuration, release self-tests, licence notices, documentation and site checks, vendored-eframe verification, the hidden-window check and installer tests (Linux and macOS). Also runs ShellCheck and `cargo deny check` when installed. Must pass before a change is done. |
| `install-hooks.sh [--uninstall]` | Sets `core.hooksPath` to `tools/hooks`, so `hooks/pre-commit` runs before every commit in every worktree: it refuses the commit while `cargo clippy --workspace --all-targets --locked -- -D warnings` fails, and is skipped when the commit stages no Rust source, `Cargo.toml`, `Cargo.lock` or `clippy.toml`. `--uninstall` removes the setting. |
| `test-release-gate.py` | Checks that tagging and publishing require Linux tests of the exact commit. Runs in `check.sh` and CI. |
| `test-hooks.py` | Verifies the pre-commit hook's staged-file detection and failure handling, including large changes and filenames containing newlines. Runs in `check.sh`. |
| `check-pr-commits.py BASE HEAD [--signoff required]\|--selftest` | Checks every non-merge commit in `BASE..HEAD` against the release subject pattern. `--signoff required` also checks for a DCO trailer matching each commit's author. Run from a Git checkout with full history. The self-test runs in `check.sh`. |
| `check-numeric-literals.py [--selftest]` | Flags numeric literals other than 0 and 1 in the Git and hosting backends (`canopy-core`, `canopy-hosting` sources, outside tests) that have neither a name (a `const`) nor a comment on their line; `--selftest` tests the scanner. |
| `check-source-headers.py [--selftest]` | Requires a header comment on every maintained crate Rust source and test file; `--selftest` tests the guard. |
| `check-ui-colors.py [--selftest]` | Rejects runtime UI colour constructors and named colour constants outside the library's `theme.rs`, except `TRANSPARENT`, `PLACEHOLDER` and the reasoned, staleness-checked texture, contrast and measurement allowlist; `--selftest` tests the guard and every exception. |
| `usersim.sh start [--kind KIND] [--size WxH]\|x XDOTOOL-ARGS...\|shot OUT.png\|restart\|stop` | The user simulation review (`docs/playbook.md`): Canopy on a private Xvfb display with its own home, XDG folders and Git configuration in a scratch folder, on a fixture with a local bare `origin`; drive it with xdotool, capture the screen, relaunch, then remove everything. `DISPLAY_BASE=N` keeps its Xvfb to displays `:N` to `:N+9` (default `:70` to `:99`), so parallel runs do not collide. |
| `mkfixture.sh [--force] <dir> [kind]` | Creates a deterministic demo repository with plain `git` (fixed identities and dates, user config ignored). Kinds: `showcase` (default), `conflict`, `linear`, `big`. |
| `screenshot.sh <repo> <out.png> [--light] [--split] [--theme NAME] [--view history\|changes\|requests] [--overlay NAME] [--style S] [--density D] [--request-density D] [--translucent on\|off] [--contrast on\|off] [--custom-theme SPEC] [--motion M] [--gallery-page N] [--demo-requests SCENARIO] [--size WxH] [--zoom 0.8..1.6]` (`-h` lists every value) | Renders Canopy on Xvfb with software OpenGL, or, only when Xvfb is unavailable, in a native Hyprland window (install Xvfb instead of using it on someone else's session), saves one frame once the repository has loaded and prints the PNG path. `DISPLAY_BASE=N` keeps the Xvfb to displays `:N` to `:N+9`. `<repo>` may be `-` for the welcome screen. Overlays (`-h` lists every one): `gallery` (every component in every state; `--gallery-page N` scrolls down N window heights), `settings`, `privacy`, `cheatsheet`, `palette`, `consent`, `crash-consent`, `legal`, `licences`, `credential`, `host-key`, `merge`, `identity`, `settings-git`, `clear-identity`. `--demo-requests` fills the pull/merge requests view with a fixed set of invented requests (`app/demo_requests.rs`), with no network, no mock server and no sign-in: `list`, `list-all`, `list-closed`, `list-overflow` (36 open requests, longer than any window), a request number (`428` open and approved with every check passing, `427` draft with checks running, `425` failing checks and changes requested, `422` merged, `420` closed with conflicts, `419` conflicts, `424` from a fork, `416` draft), `busy` (a merge in flight), `loading`, `details-loading`, `details-error`, `error`, `signed-out` or `empty`; prefix `gitlab:` for merge requests (`gitlab:425`). It implies `--view requests`, works only with `--screenshot`, and needs a repository without a GitHub or GitLab remote (the `mkfixture.sh` repositories have none). |
| `mock-hosting.py [port]` | A stand-in GitHub API on `127.0.0.1` with four pull requests, reviews, comments, checks and author picture fixtures. Start it and set `CANOPY_HOSTING_API_BASE=http://127.0.0.1:<port>` to screenshot the pull request view without a network or token. |
| `notices.py [--check]` | Regenerates `docs/legal/third-party-notices.txt` and the site's copy, `site/third-party-notices.txt`: the licence texts of the bundled fonts, of every crate compiled into the binary and of the assets some crates embed (egui-phosphor's icon fonts, two-face's syntaxes and themes; `EMBEDDED_ASSETS` in the script, which `--check` makes you review when such a crate changes major version), which each release ships and the About dialog shows. Licence texts a crate does not ship are kept in `licenses/`. CI fails while it is out of date, so run it after changing dependencies. |
| `check-docs.py` | Fails, one `file:line: problem` per finding, when a document under `docs/` is missing from `docs/README.md` (or an ADR from `docs/adr/README.md`), when a path in backticks or a relative link in a maintained document names nothing, when a source module is missing from its crate's section of `docs/architecture.md`, when `.agents/skills` differs from `.claude/skills`, when `docs/backlog.md` grows a task list (GitHub Issues is the sole backlog), or when a file under `tools/` is missing from this table. Reads the files git knows, untracked ones included. Run by `check.sh`. |
| `check-site.py [--write-legal] [DIR]` | Checks the landing site in `site/` (or `DIR`) before it is deployed, one `file:line: problem` per finding: every file a page, stylesheet or script names exists, nothing loads over plain http or from a host the content security policy in `../site/netlify.toml` refuses for that kind of load, every script parses (`node --check`, skipped with a note when Node is missing), `privacy.html` and `terms.html` are the current render of `../docs/legal/`, and `third-party-notices.txt` is an exact copy of the one there. `--write-legal` renders those two pages again and copies the notices; run it after changing a legal document or the dependencies. Run by `check.sh`, CI and the Site workflow. |
| `stall-check.py [--require] [PATH-TO-CANOPY]` | Fails when Canopy stops answering its compositor while its window is hidden, the cause of Hyprland's "Application Not Responding" (ADR 0007). Builds `stall-check/wlstub.c`, a stand-in Wayland compositor that withholds frame callbacks from a hidden window and pings its client, runs Canopy on it with software OpenGL on a private socket (nothing appears on your session), hides the window, forces repaints and fails on three unanswered pings in a row or no repaint when shown. Needs Linux, a C compiler, `wayland-scanner`, `pkg-config`, `libwayland-server` and `wayland-protocols`; without them it says so and passes (`--require` makes that an error). Builds the debug app unless given a binary. Run by `check.sh` and by CI's Linux test job, where it must pass. |
| `glass-check.py compare OFF.png ON.png --region X0,Y0,X1,Y1 [--base BASE.png] [--text RRGGBB:RATIO] [--min-spread S]\|soak [--frames N] [PATH-TO-CANOPY]\|selftest` | Checks Translucent surfaces, the backdrop blur. `compare` takes two `screenshot.sh` renders of one screen, `--translucent off` and `on`, and fails when a pixel outside the given glass regions differs, when what shows through a region is not blurred (sharper than `--max-sharpness`, or, with `--base`, than the same screen without the layer), when it varies by less than `--min-spread` grey levels, or when a `--text` colour falls under its contrast ratio on the glass. `soak` runs the debug app with `--glass-soak N` (default 2000 frames after a 1440-frame warm-up) on a private Xvfb display, with glass and again with opaque layers, and fails when the blur did not build, left OpenGL objects undeleted or turned itself off, or resident memory grew by more than 2 MB. Standard library only; not run by `check.sh`. |
| `vendor-eframe.sh [--check]` | Rebuilds `vendor/eframe` from the published eframe crate (SHA-256 pinned) plus `vendor/eframe.patch`, or with `--check` fails when the committed copy differs or the workspace does not build it (needs network access; run by `check.sh` and CI). See `vendor/README.md`. |
| `release.py plan [--force]\|number [REF]\|notes [REF]\|selftest` | Release automation. A release is a numbered build: the tag `build-<n>`, where `n` is `git rev-list --count` of the commit, so nothing is bumped or committed. `plan` prints the number to release from HEAD, or nothing when no commit since the last `build-*` tag changes what users get (`--force` releases anyway); `number` prints a commit's build number; `notes` prints the release notes from the Conventional Commits since the last tag; `selftest` runs its tests. Used by the Create release workflow. |
| `prune-releases.py run --feed-dir DIR --keep BUILD [--dry-run]\|selftest` | After a release: writes every Canopy feed release's notes into canopy/CHANGELOG.md of an app-updates clone (newest first, kept across runs) and pushes it, then deletes every other Canopy release in app-updates (with its tag) and every other `build-*` release here (keeping the tag), except BUILD and the builds `latest.json` and `beta.json` offer. Used by the Release workflow's prune job. |
| `sign-manifest.py sign PUBLIC_KEY MANIFEST...\|selftest` | Signs update manifests in minisign format with the Ed25519 key in `CANOPY_MANIFEST_KEY` (PEM), using openssl, after checking the key matches PUBLIC_KEY (`packaging/update-signing.pub`); writes `MANIFEST.minisig`. Used by the Release workflow's feed job (ADR 0021). |
| `update-manifest.sh <dist-dir> <build> <release-page> <download-base> <notes-file>` | Writes the update-feed manifest (`latest.json`, on stdout) for a release's archives and `SHA256SUMS`. Used by the Release workflow. |
| `install-desktop.sh [--uninstall]` | Builds a release binary and installs it with its desktop entry, AppStream metainfo and icons under `~/.local` (Linux). |

## Fixture kinds


- `showcase`: about 40 commits of a small Rust/web project. Merged (`feature/auth`,
  `feature/graph`), fast-forwarded (`fix/typo`) and open (`feature/dark-mode`) branches, a
  `release/1.0` branch, annotated tags `v0.1.0`, `v0.2.0` and `v1.0.0`, five authors, and a bare
  remote at `<dir>.remote.git` added as `origin`. `main` is two commits ahead of `origin/main` and
  `feature/dark-mode` one commit behind its upstream. It also has one stash entry and a dirty
  working tree: a modified file with two hunks, a staged change, an untracked file and a deleted
  file. This is the fixture used for screenshots.
- `conflict`: stopped mid-merge with `src/retry.rs` conflicted and `README.md` merged cleanly.
- `linear`: 100 linear commits.
- `big`: `BIG_COMMITS` (default 5000) mainline commits written with `git fast-import`, a
  three-commit `topic/<n>` branch merged back every 50 commits and a tag every 500. Use it to check
  graph layout and scrolling performance.

The same kind always produces the same commit IDs, so screenshots and bug reports are
reproducible. `--force` replaces an existing directory only when it carries the marker file
this script writes, so it cannot delete a real repository.

## Typical session

```sh
tools/mkfixture.sh /tmp/canopy-demo
tools/screenshot.sh /tmp/canopy-demo screenshots-out/history.png
tools/screenshot.sh /tmp/canopy-demo screenshots-out/changes-light.png --light --view changes
```

`screenshots-out/` is ignored by git. The images in `docs/screenshots/` are regenerated the same
way at 1440x900.

## Requirements

- `git` 2.32 or newer for `mkfixture.sh` (it relies on `GIT_CONFIG_GLOBAL`).
- `xvfb-run` and Mesa (llvmpipe) for headless `screenshot.sh`, or a running Hyprland session
  for native capture (the fallback when `xvfb-run` is missing). Set `CANOPY_WORKSPACE=1` to
  choose the Hyprland workspace the window opens on (default 3). Native
  capture preserves Wayland when available; temporary runner files stay beside the output PNG.
  `CANOPY_BIN=target/debug/canopy` reuses a build; `--zoom 1.6` checks the largest interface scale.
  Software rendering says nothing about real frame times; use it for layout and visuals only.

## Stress test tools

The performance stress test (100 public repositories open at once, one per tab, with concurrent
fetches, external changes, a soak and conflicts) is described in `docs/stress-test.md`. It is
run by hand on a desktop session, not in CI.

| Script | Purpose |
|---|---|
| `stress.sh clone [--shallow\|--blobless] [--subset N] [--class A,B] [--shallow-for R,R] [--jobs N] [--dir DIR]` | Clones the repositories listed in `stress-repos.txt` into a cache folder (`$CANOPY_STRESS_DIR`, default `${XDG_CACHE_HOME:-~/.cache}/canopy-stress`). Parallel with a bound, resumable, skips finished clones. |
| `stress.sh open [--bin FILE] [--duration SEC] [--label TEXT]` | Launches Canopy with every cached clone as a tab and the Detailed log level, in its own data folder, and samples RSS, threads, descriptors, inotify watches, child processes and CPU every second into `samples.csv` in a run folder under `${XDG_STATE_HOME:-~/.local/state}/canopy-stress`. Linux only. |
| `stress.sh churn [--repos N] [--duration SEC] [--modify-many N] [--conflict-setup] [--restore]` | Mutates N cached clones concurrently (files, commits, branch switches, tags) so the file watchers fire; prepares merge conflicts; restores the clones. Refuses any path that is not a clone made by `clone` inside the cache folder. |
| `stress.sh report [--run DIR]` | Summarises a run with `stress_report.py`. |
| `stress_report.py --run DIR` | Reads Canopy's JSON-lines log, the sampler CSV and the churn log; prints error and warning counts, job, operation and git durations (p50, p95, p99, max), slowest repositories, load times, peak memory, threads and descriptors, idle CPU, refresh lag, and a verdict against the pass criteria. Standard library only. |
| `stress-repos.txt` | The 30 repositories, one `owner/repo size-class` per line. |

```sh
tools/stress.sh clone --subset 10 --shallow --dir /tmp/stress-try   # rehearsal
tools/stress.sh open --dir /tmp/stress-try --subset 10 --duration 120
tools/stress.sh report
```

The stress tools need `git` (2.32 or newer), GNU coreutils and bash 4.3 or newer; `stress.sh
open` needs `/proc` and a display, and the report needs `python3`.
