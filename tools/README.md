# Tools

Scripts for development, verification, installation and releases. They run from any directory
and print usage with `-h` or when called without required arguments.

| Script | Purpose |
|---|---|
| `check.sh` | The local CI: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` (with git's configuration isolated to a fixed identity), the `release.py` self-test, `notices.py --check`, `check-docs.py`, `packaging/test-install.sh` (Linux and macOS), and ShellCheck and `cargo deny check` when they are installed. Must pass before a change is done. |
| `mkfixture.sh [--force] <dir> [kind]` | Creates a deterministic demo repository with plain `git` (fixed identities and dates, user config ignored). Kinds: `showcase` (default), `conflict`, `linear`, `big`. |
| `screenshot.sh <repo> <out.png> [--light] [--split] [--theme NAME] [--view history\|changes\|requests] [--overlay NAME] [--size WxH] [--zoom 0.8..1.6]` | Renders Canopy on Xvfb with software OpenGL, or in a native Hyprland window when Xvfb is unavailable, saves one frame once the repository has loaded and prints the PNG path. `<repo>` may be `-` for the welcome screen. Overlays: `settings`, `privacy`, `cheatsheet`, `palette`, `consent`, `crash-consent`, `legal`, `licences`, `credential`, `host-key`, `merge`. |
| `mock-hosting.py [port]` | A stand-in GitHub API on `127.0.0.1` with four pull requests, reviews, comments and checks. Start it and set `CANOPY_HOSTING_API_BASE=http://127.0.0.1:<port>` to screenshot the pull request view without a network or token. |
| `notices.py [--check]` | Regenerates `docs/legal/third-party-notices.txt`: the licence texts of the bundled fonts, of every crate compiled into the binary and of the assets some crates embed (egui-phosphor's icon fonts, two-face's syntaxes and themes; `EMBEDDED_ASSETS` in the script, which `--check` makes you review when such a crate changes major version), which each release ships and the About dialog shows. Licence texts a crate does not ship are kept in `licenses/`. CI fails while it is out of date, so run it after changing dependencies. |
| `check-docs.py` | Fails, one `file:line: problem` per finding, when a document under `docs/` is missing from `docs/README.md` (or an ADR from `docs/adr/README.md`), when a path in backticks or a relative link in a maintained document names nothing, when a source module is missing from its crate's section of `docs/architecture.md`, when `.agents/skills` differs from `.claude/skills`, or when a file under `tools/` is missing from this table. Reads the files git knows, untracked ones included. Run by `check.sh`. |
| `release.py plan [--force]\|number [REF]\|notes [REF]\|selftest` | Release automation. A release is a numbered build: the tag `build-<n>`, where `n` is `git rev-list --count` of the commit, so nothing is bumped or committed. `plan` prints the number to release from HEAD, or nothing when no commit since the last `build-*` tag changes what users get (`--force` releases anyway); `number` prints a commit's build number; `notes` prints the release notes from the Conventional Commits since the last tag; `selftest` runs its tests. Used by the Create release workflow. |
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
  for native capture. Set `CANOPY_WORKSPACE=1` to choose the native workspace (default 3). Native
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
| `stress-repos.txt` | The 100 repositories, one `owner/repo size-class` per line. |

```sh
tools/stress.sh clone --subset 10 --shallow --dir /tmp/stress-try   # rehearsal
tools/stress.sh open --dir /tmp/stress-try --subset 10 --duration 120
tools/stress.sh report
```

The stress tools need `git` (2.32 or newer), GNU coreutils and bash 4.3 or newer; `stress.sh
open` needs `/proc` and a display, and the report needs `python3`.
