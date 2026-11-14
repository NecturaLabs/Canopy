# Stress test: 100 repositories at once

## Purpose

Canopy must stay fast and stable with a lot open. The owner's requirement: **100 different
projects open at the same time, one per tab, some monstrous and others less big, with concurrent
operations running without issues or performance hiccups.** This document is the procedure for
checking that on a real machine, and the pass criteria. The tooling is:

| File | Role |
|---|---|
| `tools/stress-repos.txt` | The 100 repositories, one `owner/repo size-class` per line. The single source; the table below mirrors it. |
| `tools/stress.sh` | `clone` the repositories, `open` Canopy on all of them while sampling resources, `churn` the clones from outside, `report` a run. |
| `tools/stress_report.py` | Reads Canopy's JSON log and the sampler CSV and prints a summary and a verdict. Standard library only. |

The test is run by hand on a desktop session (it needs a display and the real GPU path; software
rendering says nothing about frame times). It is not part of CI: the clones are tens to hundreds
of gigabytes.

Nothing here touches any repository outside the stress cache folder. `churn` is the only command
that changes repositories; it refuses any path that is not a clone made by `clone` inside the
cache folder (see "Safety" below), in line with the project rule never to run mutating git
commands against real repositories.

## What the app logs, and what it does not

The report is built from what Canopy writes today (`crates/canopy-app/src/logging.rs`,
`worker.rs`, `canopy-core/src/cli.rs`). One JSON-lines file per run in the state folder's `logs/`
(`~/.local/state/canopy/logs/canopy-<UTC start>-<pid>.jsonl`); every event carries `timestamp`,
`level`, `target`, `threadName`, `message`, its own fields, and `span` (`{"name":"repo","path":...}`)
when it comes from a repository's worker thread. `CANOPY_LOG` (or Settings -> Privacy -> Log
detail) sets the detail; `tools/stress.sh open` sets `CANOPY_LOG` to the Detailed filter.

| Event (`message`) | Level | Fields | Needs | Used for |
|---|---|---|---|---|
| `canopy started` | INFO | `run`, `version`, `pid`, `filter` | Normal | time zero |
| `snapshot` | DEBUG | `commits`, `ms` | Detailed | the first one per repository marks "tab loaded"; later ones are refreshes |
| `loading the tab shown for the first time` | DEBUG | `path` | Detailed | when a tab was first shown (#62); a tab's load time runs from here to its first `snapshot` |
| `job done` | DEBUG | `job` (`details`, `diff`, `conflict`, `head-message`, `squash-message`, `operation`), `ms` | Detailed | job durations |
| `operation started` / `operation succeeded` / `operation failed` | INFO / INFO / WARN | `operation` (a label such as `Fetching` or `Checking out feature/x`), `ms`, `error` | Normal | operation durations, hung operations |
| `git finished` / `git failed` | DEBUG / WARN | `command`, `args`, `ms`, `exit`, `stderr` | Detailed | git command durations |
| `frame stats` | DEBUG | `frames`, `p50_ms`, `p95_ms`, `p99_ms`, `max_ms`, `hitches_33` (frames over 33 ms), `hitches_100` | Detailed | frame times, about every 5 s while frames are produced (below) |
| `first frame` | INFO | `ms` since process start | Normal | start-up criterion |
| `cannot open the repository`, `snapshot failed` | WARN | `error` | Normal | tabs that did not load |
| `log size limit reached` | WARN | none | Normal | the log stopped growing at 20 MiB |

### Frame events

`frame stats` and `first frame` are logged by `crates/canopy-app/src/frame_stats.rs`. A frame's
time is the CPU time eframe reports for it: the UI pass, tessellation and painting, without the
wait for the display. The percentiles come from a fixed histogram with 0.25 ms buckets up to
100 ms (a percentile is the upper edge of its bucket, never above `max_ms`; `max_ms` is exact), so
recording a frame is one array increment and allocates nothing. A `frame stats` event covers the
frames of the 5 s since a window's first frame and is written by the first frame after that; an
idle window produces no frames and no event, and the frames left when the window goes idle are
written when the next one arrives. The event has no `context` field: a window mixes views, so
judge scrolling or tab switching by the minutes (`report --from/--until`). `first frame` is
logged once, when the first frame's UI pass has finished, with the time since the start of `main`.
The report ranks the slowest windows under "Frame times". It leaves the windows that fall in the
first 30 s after start (loading, not interaction) out of that table and out of the verdicts, and
prints them on one line of their own: frames, frames over 33 ms and the worst frame.

### Instrumentation the app does not have yet

The requirements below depend on these. None exists today; the report prints `n/a` for the
criteria that need them, and does not invent numbers. The numbering is kept from when frame
times and the first frame (items 1 and 2) were on this list too.

3. **Tabs loaded.** Optional: an INFO event `tabs loaded` with `tabs` and `ms`. The report derives
   the same from the first `snapshot` of each repository, so this is a convenience.
4. **Watcher failures.** Logged: a WARN starting `no file watcher` (`watcher.rs`) when the watcher
   cannot start or a folder cannot be watched (the inotify watch limit), and the tab says so. The
   report fails the run on any of them ("every repository watched").
5. **Status refreshes.** A working-tree change triggers a `RefreshStatus` that is not logged (only
   full snapshots are). Needed: a DEBUG `status` event with `ms`, so refresh lag after file churn
   can be measured, not only after changes under `.git`.
6. **Queue wait.** Jobs for one repository run one after another on its worker thread, so a diff
   can wait behind a snapshot. `job done` reports run time only; a `wait_ms` field (time from
   queueing to start) would show that.
7. **Log size.** A run's log stops at 20 MiB. At the Detailed level, 100 tabs and heavy churn can
   reach that within a long soak. The report flags it; if it happens, either narrow the filter with
   `--log-filter` (for example `warn,canopy=debug,canopy_core=info`, which drops `git finished`) or
   raise the cap for stress runs.

## Machine to record

`tools/stress.sh open` writes `meta.txt` into the run folder with the CPU model, thread count,
RAM, swap, kernel, OS, the file system of the cache folder, `ulimit -n`, the two inotify limits,
git and Canopy versions. Add by hand: the disk type (NVMe, SATA SSD, HDD) and whether the cache
is on the same disk as the home folder; the GPU and driver; the session (Wayland or X11) and
compositor; the monitor resolution and scale; the power profile; anything else running; and
whether the page cache was cold (`sync; echo 3 | sudo tee /proc/sys/vm/drop_caches`) at launch.
Results from different machines are not comparable; always report with the machine.

Reference tier for the numbers below: a recent 8-core desktop, 16 GB of RAM or more, NVMe, real
GPU. On weaker hardware, say so and scale the time criteria, not the stability criteria (no
panics, no hung operations, no leaks stay absolute).

## The repositories

One hundred public repositories on github.com, all verified to exist (and not to be forks) when
this list was written. Sizes are what GitHub reports for the repository (an upper-bound style
figure: a real clone is often somewhat smaller, and Git LFS objects are not included). The size
class is a rough bucket by size and history length: **monstrous** (about 1 GB or more, or a very
long history; `rust-lang/rust` and `git/git` are in it for their history), **large** (about
300 MB to 1 GB), **medium** (about 40 to 300 MB), **small** (under 40 MB). 20 monstrous, 22 large,
28 medium, 30 small.

Where "why it is here" says "probably" or "likely", the trait is expected but was not checked
file by file; the one-liners at the end of this document verify it after cloning.

| # | Repository | Class | GitHub size | Why it is here |
|---:|---|---|---:|---|
| 1 | [`torvalds/linux`](https://github.com/torvalds/linux) | monstrous | 6.1 GB | over a million commits, about 90,000 files, hundreds of tags, merge-heavy graph; some file names differ only by case (checkout caveat on macOS and Windows) |
| 2 | [`llvm/llvm-project`](https://github.com/llvm/llvm-project) | monstrous | 4.3 GB | monorepo, several hundred thousand commits, many release tags |
| 3 | [`NixOS/nixpkgs`](https://github.com/NixOS/nixpkgs) | monstrous | 3.3 GB | extremely busy history, very wide directories (thousands of entries in one folder), many branches |
| 4 | [`chromium/chromium`](https://github.com/chromium/chromium) | monstrous | 63.9 GB | largest here (GitHub reports ~65 GB); mirror of the Chromium source, over a million commits. Use `--shallow-for` |
| 5 | [`gcc-mirror/gcc`](https://github.com/gcc-mirror/gcc) | monstrous | 4.0 GB | history converted from SVN, thousands of branches and tags |
| 6 | [`WebKit/WebKit`](https://github.com/WebKit/WebKit) | monstrous | 12.5 GB | history converted from SVN, hundreds of thousands of commits, huge tree |
| 7 | [`freebsd/freebsd-src`](https://github.com/freebsd/freebsd-src) | monstrous | 3.3 GB | whole OS in one tree, many release and vendor branches, very long history |
| 8 | [`mozilla/gecko-dev`](https://github.com/mozilla/gecko-dev) | monstrous | 6.0 GB | archived (still clonable); very long converted history, many branches and tags |
| 9 | [`microsoft/vscode`](https://github.com/microsoft/vscode) | monstrous | 1.5 GB | TypeScript monorepo, very busy, many small commits and many contributors |
| 10 | [`kubernetes/kubernetes`](https://github.com/kubernetes/kubernetes) | monstrous | 1.5 GB | large vendored/staging trees, hundreds of release tags, many branches |
| 11 | [`tensorflow/tensorflow`](https://github.com/tensorflow/tensorflow) | monstrous | 1.3 GB | C++ and Python mix, many tags, large generated files |
| 12 | [`pytorch/pytorch`](https://github.com/pytorch/pytorch) | monstrous | 1.5 GB | many submodules (third_party), huge mixed tree |
| 13 | [`openjdk/jdk`](https://github.com/openjdk/jdk) | monstrous | 1.4 GB | very long converted history, many tags, deep source paths |
| 14 | [`dotnet/runtime`](https://github.com/dotnet/runtime) | monstrous | 1.2 GB | huge tree of many projects, deep paths, very busy |
| 15 | [`rust-lang/rust`](https://github.com/rust-lang/rust) | monstrous | 989 MB | several hundred thousand commits, merge-commit workflow, many tags, submodules |
| 16 | [`git/git`](https://github.com/git/git) | monstrous | 315 MB | 80k+ commits since 2005 (314 MB only), signed tags, octopus merges; the repository Canopy drives |
| 17 | [`ClickHouse/ClickHouse`](https://github.com/ClickHouse/ClickHouse) | monstrous | 12.1 GB | many submodules (contrib), very large C++ tree |
| 18 | [`LibreOffice/core`](https://github.com/LibreOffice/core) | monstrous | 6.8 GB | huge tree and history, many binary test documents |
| 19 | [`JetBrains/intellij-community`](https://github.com/JetBrains/intellij-community) | monstrous | 5.7 GB | giant tree (hundreds of thousands of files), very many tags |
| 20 | [`KhronosGroup/glTF-Sample-Assets`](https://github.com/KhronosGroup/glTF-Sample-Assets) | monstrous | 1.7 GB | binary assets (.glb, images): few commits but fat blobs, slow diffs and checkouts |
| 21 | [`home-assistant/core`](https://github.com/home-assistant/core) | large | 885 MB | thousands of small files under one components folder, extremely busy history |
| 22 | [`python/cpython`](https://github.com/python/cpython) | large | 862 MB | long history, many branches and tags, C and Python mix |
| 23 | [`DefinitelyTyped/DefinitelyTyped`](https://github.com/DefinitelyTyped/DefinitelyTyped) | large | 809 MB | tens of thousands of tiny files across thousands of packages, constant churn |
| 24 | [`postgres/postgres`](https://github.com/postgres/postgres) | large | 774 MB | history since 1996 with CVS conversion, many release branches |
| 25 | [`qemu/qemu`](https://github.com/qemu/qemu) | large | 690 MB | mailing-list workflow with many merge commits, large C tree |
| 26 | [`angular/angular`](https://github.com/angular/angular) | large | 644 MB | TypeScript monorepo, many tags |
| 27 | [`grpc/grpc`](https://github.com/grpc/grpc) | large | 612 MB | submodules (third_party), multi-language monorepo |
| 28 | [`apache/spark`](https://github.com/apache/spark) | large | 603 MB | Scala/Java monorepo, many tags and branches |
| 29 | [`dolphin-emu/dolphin`](https://github.com/dolphin-emu/dolphin) | large | 548 MB | submodules (Externals), binary data files |
| 30 | [`golang/go`](https://github.com/golang/go) | large | 449 MB | history since 2007 with conversions, many release branches and tags |
| 31 | [`ceph/ceph`](https://github.com/ceph/ceph) | large | 984 MB | submodules, large C++ tree, many release branches |
| 32 | [`systemd/systemd`](https://github.com/systemd/systemd) | large | 724 MB | many small commits, many test fixtures |
| 33 | [`opencv/opencv`](https://github.com/opencv/opencv) | large | 555 MB | large C++ tree with test data, many tags |
| 34 | [`sqlite/sqlite`](https://github.com/sqlite/sqlite) | large | 554 MB | mirror of a Fossil repository: unusual history shape |
| 35 | [`godotengine/godot-demo-projects`](https://github.com/godotengine/godot-demo-projects) | large | 415 MB | hundreds of small projects with image, audio and 3D binary assets |
| 36 | [`pandas-dev/pandas`](https://github.com/pandas-dev/pandas) | large | 404 MB | large Python tree, many tags |
| 37 | [`jackfrued/Python-100-Days`](https://github.com/jackfrued/Python-100-Days) | large | 396 MB | probably CJK file names and many images (check with the one-liner in the doc) |
| 38 | [`notepad-plus-plus/notepad-plus-plus`](https://github.com/notepad-plus-plus/notepad-plus-plus) | large | 322 MB | Windows project, CRLF line endings |
| 39 | [`neovim/neovim`](https://github.com/neovim/neovim) | large | 393 MB | frequently moved tags (nightly, stable), vendored test data |
| 40 | [`JuliaLang/julia`](https://github.com/JuliaLang/julia) | large | 379 MB | mixed languages, many tags and branches |
| 41 | [`twbs/bootstrap`](https://github.com/twbs/bootstrap) | large | 316 MB | documentation site plus library, many tags and branches |
| 42 | [`openssl/openssl`](https://github.com/openssl/openssl) | large | 372 MB | very long history, many tags and branches |
| 43 | [`numpy/numpy`](https://github.com/numpy/numpy) | medium | 180 MB | C and Python, many tags, uses submodules for vendored code |
| 44 | [`denoland/deno`](https://github.com/denoland/deno) | medium | 247 MB | Rust monorepo that uses submodules for test suites |
| 45 | [`redis/redis`](https://github.com/redis/redis) | medium | 212 MB | dense C history, many tags |
| 46 | [`chinese-poetry/chinese-poetry`](https://github.com/chinese-poetry/chinese-poetry) | medium | 222 MB | CJK directory and file names, very large JSON files |
| 47 | [`apache/arrow`](https://github.com/apache/arrow) | medium | 230 MB | multi-language monorepo, submodules for test data |
| 48 | [`spring-projects/spring-framework`](https://github.com/spring-projects/spring-framework) | medium | 240 MB | deep Java package paths, many tags |
| 49 | [`curl/curl`](https://github.com/curl/curl) | medium | 145 MB | very long history, signed tags |
| 50 | [`jesseduffield/lazygit`](https://github.com/jesseduffield/lazygit) | medium | 155 MB | another Git client (a TUI); committed vendor folder with thousands of files |
| 51 | [`FortAwesome/Font-Awesome`](https://github.com/FortAwesome/Font-Awesome) | medium | 159 MB | font and SVG binaries |
| 52 | [`microsoft/Windows-driver-samples`](https://github.com/microsoft/Windows-driver-samples) | medium | 175 MB | many sample projects, Windows-style paths and (likely) CRLF |
| 53 | [`rust-lang/rust-analyzer`](https://github.com/rust-lang/rust-analyzer) | medium | 123 MB | dense history, weekly tags |
| 54 | [`CyC2018/CS-Notes`](https://github.com/CyC2018/CS-Notes) | medium | 113 MB | probably CJK file names with spaces, many images (check with the one-liner) |
| 55 | [`PowerShell/PowerShell`](https://github.com/PowerShell/PowerShell) | medium | 107 MB | Windows origin, mixed CRLF and LF |
| 56 | [`ShareX/ShareX`](https://github.com/ShareX/ShareX) | medium | 96 MB | Windows C# project, CRLF |
| 57 | [`obsproject/obs-studio`](https://github.com/obsproject/obs-studio) | medium | 81 MB | C/C++ with many translation files |
| 58 | [`nginx/nginx`](https://github.com/nginx/nginx) | medium | 75 MB | history converted from another VCS, many tags |
| 59 | [`libgit2/libgit2`](https://github.com/libgit2/libgit2) | medium | 74 MB | the library Canopy reads repositories with; also a cross-check for odd repository states |
| 60 | [`rust-lang/cargo`](https://github.com/rust-lang/cargo) | medium | 73 MB | Rust, merge-heavy, many tags |
| 61 | [`rust-lang/book`](https://github.com/rust-lang/book) | medium | 70 MB | documentation with many listing files and images |
| 62 | [`simple-icons/simple-icons`](https://github.com/simple-icons/simple-icons) | medium | 68 MB | thousands of SVG files in one directory |
| 63 | [`lodash/lodash`](https://github.com/lodash/lodash) | medium | 49 MB | large monolithic source files |
| 64 | [`mattn/go-sqlite3`](https://github.com/mattn/go-sqlite3) | medium | 48 MB | contains the multi-megabyte SQLite amalgamation: giant single-file diffs, very long lines |
| 65 | [`vuejs/core`](https://github.com/vuejs/core) | medium | 45 MB | TypeScript monorepo, many tags |
| 66 | [`sharkdp/bat`](https://github.com/sharkdp/bat) | medium | 34 MB | submodules for syntax and theme assets |
| 67 | [`django/django`](https://github.com/django/django) | medium | 276 MB | long history, many branches and tags |
| 68 | [`nlohmann/json`](https://github.com/nlohmann/json) | medium | 297 MB | a single ~1 MB header (single_include/json.hpp): very long diffs; large test data |
| 69 | [`rails/rails`](https://github.com/rails/rails) | medium | 284 MB | long history, many branches and tags |
| 70 | [`o3de/o3de`](https://github.com/o3de/o3de) | medium | 229 MB | uses Git LFS for assets (install git-lfs to get the real files) |
| 71 | [`octocat/Hello-World`](https://github.com/octocat/Hello-World) | small | < 1 MB | tiny; a few commits and branches |
| 72 | [`octocat/Spoon-Knife`](https://github.com/octocat/Spoon-Knife) | small | < 1 MB | tiny HTML and CSS |
| 73 | [`kelseyhightower/nocode`](https://github.com/kelseyhightower/nocode) | small | < 1 MB | almost empty tree |
| 74 | [`torvalds/uemacs`](https://github.com/torvalds/uemacs) | small | < 1 MB | tiny, old C history |
| 75 | [`torvalds/pesconvert`](https://github.com/torvalds/pesconvert) | small | < 1 MB | tiny, C |
| 76 | [`antirez/kilo`](https://github.com/antirez/kilo) | small | < 1 MB | one C source file |
| 77 | [`karpathy/nanoGPT`](https://github.com/karpathy/nanoGPT) | small | < 1 MB | few files, Python |
| 78 | [`karpathy/micrograd`](https://github.com/karpathy/micrograd) | small | < 1 MB | few files, a notebook |
| 79 | [`mathiasbynens/dotfiles`](https://github.com/mathiasbynens/dotfiles) | small | < 1 MB | dotfiles at the root |
| 80 | [`sindresorhus/awesome`](https://github.com/sindresorhus/awesome) | small | 2 MB | one large README, many contributors |
| 81 | [`sharkdp/hyperfine`](https://github.com/sharkdp/hyperfine) | small | 2 MB | Rust, tags |
| 82 | [`sharkdp/fd`](https://github.com/sharkdp/fd) | small | 2 MB | Rust, tags |
| 83 | [`spf13/cobra`](https://github.com/spf13/cobra) | small | 2 MB | Go library, tags |
| 84 | [`tj/commander.js`](https://github.com/tj/commander.js) | small | 4 MB | JavaScript library, tags |
| 85 | [`jlevy/the-art-of-command-line`](https://github.com/jlevy/the-art-of-command-line) | small | 3 MB | many translated READMEs |
| 86 | [`github/gitignore`](https://github.com/github/gitignore) | small | 3 MB | hundreds of small template files in one folder |
| 87 | [`nvm-sh/nvm`](https://github.com/nvm-sh/nvm) | small | 4 MB | shell scripts, many tags |
| 88 | [`rust-lang/rustlings`](https://github.com/rust-lang/rustlings) | small | 4 MB | many small exercise files |
| 89 | [`leachim6/hello-world`](https://github.com/leachim6/hello-world) | small | 5 MB | hundreds of tiny files in many languages, unusual file names |
| 90 | [`ohmyzsh/ohmyzsh`](https://github.com/ohmyzsh/ohmyzsh) | small | 14 MB | shell plugins, very many contributors |
| 91 | [`psf/black`](https://github.com/psf/black) | small | 8 MB | Python tool, many tags |
| 92 | [`gin-gonic/gin`](https://github.com/gin-gonic/gin) | small | 10 MB | Go library |
| 93 | [`nothings/stb`](https://github.com/nothings/stb) | small | 6 MB | single-header C libraries: very long files |
| 94 | [`BurntSushi/ripgrep`](https://github.com/BurntSushi/ripgrep) | small | 6 MB | Rust, tags |
| 95 | [`junegunn/fzf`](https://github.com/junegunn/fzf) | small | 8 MB | Go, long single source files |
| 96 | [`pallets/flask`](https://github.com/pallets/flask) | small | 12 MB | Python, tags |
| 97 | [`psf/requests`](https://github.com/psf/requests) | small | 13 MB | Python, tags |
| 98 | [`libuv/libuv`](https://github.com/libuv/libuv) | small | 18 MB | C, tags |
| 99 | [`tmux/tmux`](https://github.com/tmux/tmux) | small | 23 MB | C, long history |
| 100 | [`git-lfs/git-lfs`](https://github.com/git-lfs/git-lfs) | small | 21 MB | the Git LFS client source (Go) |

### Disk needed

By GitHub-reported size, full history: monstrous about 139 GB (of which `chromium/chromium` is
about 64 GB), large about 12 GB, medium about 4 GB, small under 1 GB: **about 156 GB in all, about
92 GB without Chromium.** Working trees come on top of the history for a full clone (the figures
above are history only), so allow roughly 30 percent more, and check with `du` after the first
clone: record the real number in the results. Modes that need less:

- `--shallow` (`--depth 1`): working tree plus one commit. Far smaller, but there is no history to
  scroll, so use it only to test tree size, checkouts and the Changes view.
- `--blobless` (`--filter=blob:none`): full commit history, file contents fetched on demand. Good
  for history scrolling. Commit details and diffs of old commits download the objects they need
  through `git` (the status bar says "Downloading from origin…"), so the first diff of each old
  commit waits for the network; that wait is expected, and a failed download shows git's own
  message in the details pane with a Retry button. "Object not found" on a blobless or treeless
  clone is a bug now: record it.
- `--shallow-for chromium/chromium`: shallow for named repositories only. **The recommended
  first run is the full set with Chromium shallow.** Add other names with a comma.
- `--subset N`: N evenly spaced entries of the list, so every size class is represented
  (`--subset 10` gives about two monstrous, two large, three medium and three small). Use it to
  rehearse the procedure before committing to 100.
- `--class monstrous,large`, `--exclude owner/repo`: narrower selections.

Git LFS: `o3de/o3de` and possibly others store assets in LFS. With `git-lfs` installed the clone
downloads them; to skip, run the clone as `GIT_LFS_SKIP_SMUDGE=1 tools/stress.sh clone ...`.

## Setup

1. **Build**: `cargo build --release -p canopy-app` (the stress binary is the release one). The
   script reads `target/release/canopy`, or `--bin`, or `$CANOPY_BIN`.
2. **Raise the limits.** One inotify instance serves every tab, but each tab's whole tree,
   including `.git`, is watched folder by folder, so large trees need many watches.
   For this session:

   ```sh
   sudo sysctl fs.inotify.max_user_instances=1024 fs.inotify.max_user_watches=2097152
   ulimit -n 65535        # in the shell that starts Canopy
   ```

   `open` warns when these are low. Record the values; the pass criteria are about Canopy staying
   inside them, so also run once with the stock limits and note what fails and how it shows up
   (see instrumentation item 4).
3. **Check git**: `git --version` (2.32 or newer), `git-lfs` optional.
4. **Clone** into the cache folder (default `~/.cache/canopy-stress`, or `--dir`, or
   `$CANOPY_STRESS_DIR`). Put it on a fast disk with the space above.

   ```sh
   tools/stress.sh clone --shallow-for chromium/chromium --jobs 4     # about 90+ GB, hours
   tools/stress.sh clone --dry-run                                    # what it would do
   tools/stress.sh clone --subset 10 --shallow --dir /tmp/stress-try  # rehearsal
   ```

   It is resumable: finished clones are skipped, an interrupted one is removed and redone. It
   retries failures twice and exits non-zero if any clone failed; git's messages are in
   `<cache>/.logs/`. GitHub may throttle anonymous clones; lower `--jobs` if you see failures.
   `--base-url file:///path/to/mirrors` (or `$CANOPY_STRESS_BASE_URL`) clones from a local mirror.
5. **First-run dialogs.** `open` keeps Canopy's settings and logs in its own data folder
   (`<results>/data`) so your real ones are untouched. The first launch there shows the terms and
   consent dialogs: run `tools/stress.sh open --subset 3 --duration 60` once, accept them, and
   quit. In Settings -> Git set **Fetch in the background** as needed by scenario (d).
6. **Results folder**: default `~/.local/state/canopy-stress` (or `--results`, or
   `$CANOPY_STRESS_RESULTS`). Each `open` makes `run-<UTC time>/` there and points `latest` at it.

## Scenarios

Run them in this order; each one is a separate `open` run unless stated. The first thing to do
every time: close everything else that is heavy, and note the time of each step you do by hand
(the log and the CSV are timestamped, so `report --from/--until` can cut a window out).

Optional automation on X11 or XWayland (not Wayland-native windows): `xdotool` can send the keys
used below. By hand works equally.

### (a) Cold open of all 100 tabs from the command line

```sh
sync; echo 3 | sudo tee /proc/sys/vm/drop_caches     # cold cache; skip for a warm run
tools/stress.sh open --label a-cold --duration 600
# touch nothing for the first minute (one tab loaded), then show every tab once (see below),
# leave it idle for five minutes, then close Canopy or let --duration do it
tools/stress.sh report
```

**Tabs load when first shown (#62).** Canopy loads the active tab at once and lists the other 99
by name only: no repository thread, file watcher or snapshot until a tab is first shown. So a run
that touches nothing measures one loaded tab (threads are about 1 per loaded tab plus the shared
threads; one inotify instance serves them all). To load them all, show each tab once, for example holding
`Ctrl+Tab` (or `xdotool key --repeat 100 --delay 300 ctrl+Tab`). The report's "all tabs loaded"
criterion fails until every tab has been shown; each tab's load time runs from the moment it was
first shown (the `loading the tab shown for the first time` event) to its first snapshot, so the
pace of your switching does not count against it. Every other scenario below starts from
"everything loaded" and therefore from this step.

Watch: how long until the tabs fill in (the report's "Loading" section: first snapshot per
repository), whether the window stays responsive while loading (try moving and resizing it),
memory and thread peaks, `index.lock` or watcher problems in the log. Then repeat once warm
(without dropping caches) and record both. Leave the app untouched for five minutes after loading
so the idle CPU criterion has a clean stretch (`--idle-from`/`--idle-to` pick it explicitly).

### (b) Switching rapidly between tabs

With everything loaded: `Ctrl+Tab` and `Ctrl+Shift+Tab` (next, previous), `Ctrl+1` to `Ctrl+9`.
Hold `Ctrl+Tab` for 30 seconds so the tab strip cycles through all 100 several times; then switch
with `Ctrl+Tab` while the History and Changes views (`1`, `2`) alternate. With the optional
automation:

```sh
xdotool key --repeat 600 --delay 30 ctrl+Tab       # about 33 switches a second
```

Watch for: any visible stall, tabs that show stale or empty content after switching, the
strip itself being slow with 100 tabs. The frame criteria read `frame stats`.

### (c) Heavy views on the monstrous repositories

For each of `torvalds/linux`, `llvm/llvm-project`, `NixOS/nixpkgs`, `gcc-mirror/gcc`,
`mozilla/gecko-dev`, `WebKit/WebKit`, `freebsd/freebsd-src`, `microsoft/vscode`:

1. **Scroll the history** from the top to as far as it loads, with the wheel and with `j`/`k` and
   Page Down, for about a minute: `xdotool click --repeat 400 --delay 15 5` scrolls down.
   Note whether more history loads smoothly.
2. **Open huge commits and diffs.** Find big commits with
   `git -C <repo> log -n 20000 --format=%h --shortstat | awk '/^[0-9a-f]{7,}$/ {c=$1} /files? changed/ {print $1, c}' | sort -rn | head`
   and open the top few from the filter box (`Ctrl+F`, paste the hash). In `torvalds/linux`,
   `1da177e4c3f4` (the initial import, roughly 17,000 files) is the classic. Open the largest
   files' diffs, toggle split view, whitespace modes and syntax highlighting.
3. **Changes view with thousands of modified files.** Make them from outside, in a few monstrous
   clones, then look at them in Canopy and stage, unstage and discard them:

   ```sh
   tools/stress.sh churn --class monstrous --repos 4 --modify-many 5000
   ```

   Press `2` for Changes in those tabs; scroll the list; stage all, unstage all. Afterwards
   `tools/stress.sh churn --restore`.

Record the slowest actions. The log gives `details`, `diff` and `Staging` durations per repository;
`report` ranks the slowest repositories.

### (d) Concurrent operations: fetch everywhere, and external changes everywhere

Two things at once, for 10 to 15 minutes, with all 100 tabs open:

1. **Fetch in many tabs.** Easiest and most demanding: set Settings -> Git -> **Fetch in the
   background** to the shortest option (every 5 minutes). A tab's schedule starts when it first
   sees auto-fetch on (opening the tab, or turning the setting on, or changing the interval, with
   tabs already open), and its first fetch falls due between half the interval and a whole one
   later, by a hash of its path, so the 100 tabs' first fetches spread over the second half of
   the interval instead of one herd. Each tab then waits a whole interval after every fetch, its
   own or one you ran by hand. For a burst, press `f` (fetch) in tabs by hand or with
   `for i in $(seq 100); do xdotool key --delay 150 ctrl+Tab f; done`. This uses the network:
   failures from GitHub throttling are not Canopy's; use a local mirror (`--base-url`) when you
   need to separate the two.
2. **External process mutating the working trees**, so the file watchers fire everywhere:

   ```sh
   tools/stress.sh churn --repos 40 --duration 900 --interval 2
   ```

   Each of the 40 clones (evenly spread over all size classes) gets its own concurrent loop that
   creates, edits, renames and deletes files under `stress-churn/`, commits them, switches among
   `stress/a`, `stress/b` and the original branch, creates and deletes tags and branches, and
   appends lines to tracked files. `--deep` also checks out commits far back (full clones only).
   The action log goes into the run folder; `report` uses it to measure how long after each change
   the app showed it (refresh lag).

During this, work in the app in a few tabs: stage, commit, switch branches, open diffs. Watch for
toast errors, `index.lock` messages (the churn log counts the ones its own git hit: they mean the
app or another git held the lock), tabs that stop updating, and the UI stalling.

### (e) Soak: 30 minutes and more, memory and threads

One run of 45 minutes with phases, so that growth and leaks stand out from load:

| Minutes | Phase |
|---|---|
| 0 to 5 | load; do not touch |
| 5 to 15 | idle baseline |
| 15 to 30 | churn: `tools/stress.sh churn --repos 20 --duration 900 --interval 5` started at minute 15 |
| 30 to 45 | idle again |

```sh
tools/stress.sh open --label e-soak --duration 2700
# in another terminal, at minute 15:
tools/stress.sh churn --repos 20 --duration 900 --interval 5
# afterwards:
tools/stress.sh report --idle-from 300 --idle-to 840
```

The report compares memory (heap, `RssAnon`), threads and descriptors in the first idle phase
with the last: after churn stops they must return to where they were. Also look at
`threads.txt` in the run folder (a thread-name histogram every five minutes) and at CPU while idle at
the end. Optionally extend to two hours.

### (f) Conflicts while others work

```sh
tools/stress.sh churn --class small,medium --repos 5 --conflict-setup
```

This prepares `stress/conflict-a` and `stress/conflict-b` in five clones so that merging b into
a conflicts (an add/add conflict on `stress-conflict.txt`, plus the first line of `README.md` when
there is one). It leaves them on `a`. Start `open` and, in a second terminal, churn the rest:

```sh
tools/stress.sh churn --repos 30 --duration 600      # skips the conflict repositories
```

In Canopy, in those five tabs: merge `stress/conflict-b` (palette or branch menu), see the
conflict banner and the conflicted file list, open the conflict view, resolve it (take a side or
edit), mark resolved, continue or abort the merge; do it in several tabs in a row while churn
runs. `--leave-merging` instead leaves the merge already in progress (like the `conflict`
fixture) for testing the in-progress banner and abort at scale. The merge itself fails with
conflicts, so `operation failed: Merging` warnings are expected: pass
`--allow 'operation failed: Merging'` to `report`. Churn skips any repository with a merge in
progress or prepared conflict, so nothing fights the person resolving. Afterwards
`tools/stress.sh churn --restore` puts every touched clone back on its original branch.

## What to measure

| What | How | Where it ends up |
|---|---|---|
| Start-up to first frame | `first frame` event (`ms` since process start) | `report`, "Loading" |
| Load time per tab | first `snapshot` event per repository, relative to its `loading the tab shown for the first time` event (the process start when that event is absent) | `report`, "Loading" |
| Frame times p50/p95/p99, hitches over 33 ms | `frame stats` events | `report`, "Frame times" |
| RSS over time (total, heap `RssAnon`, mapped files `RssFile`) | `/proc/<pid>/status`, every second | `samples.csv` |
| Thread count | `/proc/<pid>/status` `Threads`; names every 5 min | `samples.csv`, `threads.txt` |
| Open file descriptors | entries in `/proc/<pid>/fd` | `samples.csv` |
| inotify instances and watches | `inotify` entries in `/proc/<pid>/fdinfo`, every 10 s | `samples.csv` |
| Child processes (git) | direct children of the process, every second | `samples.csv` |
| CPU while idle | `utime+stime` from `/proc/<pid>/stat`, 100 = one core | `samples.csv`, `report` finds quiet stretches |
| Job and operation durations | `job done`, `operation succeeded/failed` events | `report` |
| git command durations | `git finished`, `git failed` events | `report` |
| Refresh lag after external changes | churn action time to next `snapshot` of that repository | `report`, "External churn" |
| Errors, warnings, panics, crash reports | log levels, `crashes.txt`, exit code | `report` |
| Hung operations | `operation started` with no matching end | `report` |

## Pass criteria

All numbers are for the reference tier above, with 100 tabs open. `tools/stress_report.py`
prints each as PASS, FAIL, REVIEW or n/a; the thresholds are its `LIMITS` (override one with
`--limit NAME=VALUE` to try another figure). The report exits with status 1 when any criterion
fails.

| Criterion | Limit | Scenario |
|---|---|---|
| Panics, crash reports, ERROR log events | 0 | all |
| Canopy exit | clean (status 0, or 143 when stopped by `--duration`) | all |
| Warnings | each read and explained; unexpected ones fail the run (REVIEW until judged) | all |
| Every repository opens (no `cannot open the repository`) | 100 of 100 | a |
| First frame | at most 2 s | a |
| Load time of a tab (from first shown to its first snapshot) | slowest at most 120 s; 95 % of tabs within 60 s | a |
| UI usable while tabs load (no frozen window) | by eye, and the report's loading line (first 30 s): a worst frame of about 1 s or more is a frozen window; not a verdict | a |
| Frames while tab switching | none over 100 ms; at most 1 % over 33 ms | b |
| Frames while scrolling history | p95 at most 16.7 ms, p99 at most 33 ms, none over 100 ms (use `--from/--until`) | c |
| Share of frames over 33 ms over the whole run | at most 1 % | all |
| `details` and `diff` jobs | p95 at most 500 ms, max at most 10 s | c, d |
| `snapshot` (history refresh) | p95 at most 2 s, max at most 30 s | all |
| Local operations (stage, commit, checkout, merge, ...) | p95 at most 5 s, max at most 60 s | d, f |
| Fetch, pull, push | none longer than 10 min; none unfinished at exit | d |
| Hung operations | 0 | all |
| Refresh lag after a change under `.git` | p95 at most 5 s; no change without a refresh within 30 s | d, e |
| One shared file watcher | exactly 1 inotify instance, whatever the number of tabs; no `no file watcher` warning | a |
| inotify watches | at most 90 % of `max_user_watches` | a |
| Idle CPU, 100 tabs, nothing happening | median at most 2 %, p95 at most 5 % of one core | a, e |
| RSS in the minute after all tabs loaded | at most 2 GiB | a |
| Peak RSS over the whole run | at most 4 GiB | all |
| Heap growth during the soak | last five idle minutes at most 10 % and 50 MiB above the first five | e |
| Threads | peak at most 450 (about 1 per loaded tab, plus the UI and the shared watcher); back within 20 of baseline after churn | e |
| Open descriptors | peak at most 70 % of `ulimit -n`; back within 50 of baseline | e |
| Log not truncated | the 20 MiB cap not reached | all |

The memory and thread figures are first estimates from reading the code (one worker thread per
tab that has been shown, and one watcher thread and one inotify backend thread in all; a tab that
has not been shown has none, see "Tabs load when first shown" under (a)): the first real run should set them
properly. Tighten them to what the first clean run shows, with headroom, and write the values here.

## Reporting results

```sh
tools/stress.sh report --run <run folder> --allow '<expected warning regex>' --json report.json > report.txt
```

Add the result to the pull request or issue as:

1. The machine: contents of `meta.txt` plus the by-hand items from "Machine to record".
2. The command lines used (clone mode, subset, scenarios, the churn parameters) and the Canopy
   commit (`git rev-parse HEAD`) and the cache's `mode=` (full, shallow, blobless).
3. For each scenario: pass or fail by criterion, taken from the report's verdict, and the report
   output for failures.
4. For each failure: the repository, the time (seconds after start), the log lines around it
   (`jq 'select(.span.path|test("<repo>"))' canopy.jsonl`), and what was on screen.
5. The run folder (`meta.txt`, `samples.csv`, `canopy.jsonl`, `report.txt`; leave `stderr.txt`
   out). The log contains local repository paths, so look before sharing.

A short table for the summary:

| Scenario | Result | Notes |
|---|---|---|
| a cold open | | |
| a warm open | | |
| b tab switching | | |
| c heavy views | | |
| d concurrent operations | | |
| e soak 45 min | | |
| f conflicts | | |

## Safety

- `clone` writes only inside the cache folder, and refuses a non-empty folder that is not a stress
  cache. It writes a marker file (`.canopy-stress-cache`) in the folder and `.git/canopy-stress`
  in each clone.
- `churn` only works on directories that are inside the cache folder (after resolving
  symlinks), exactly `owner/repo` deep, with a real `.git` directory carrying the clone marker. Every
  git call goes through one wrapper that checks this again, runs with no user or system git
  configuration, no hooks, no gc, no network and no terminal prompts. It never pushes, fetches or
  runs a command that leaves the cache. It does use `checkout -f`, `clean` and `update-ref -d`,
  and only inside those clones, to restore them.
- `open` uses its own Canopy data folder under the results folder unless `--real-data` is passed.

## Cleanup

```sh
tools/stress.sh churn --restore       # clones back on their original branch, churn leftovers removed
rm -rf ~/.cache/canopy-stress         # the cache (or your --dir / $CANOPY_STRESS_DIR)
rm -rf ~/.local/state/canopy-stress   # results, the isolated Canopy data folder and its logs
```

Move the run folders you want to keep out of the results folder first. System changes made for
the test (`sysctl`, `ulimit`) last until reboot or the end of the shell, unless you put them into
`/etc/sysctl.d/`; remove them from there if you did.

## Verifying a repository's traits

After cloning, to confirm what the table says (run in the clone):

```sh
git ls-files -z | grep -zcP '[^\x00-\x7F]'                       # files with non-ASCII names
git ls-files --eol | grep -c 'w/crlf'                            # files with CRLF in the working tree
git ls-files | awk -F/ '{print NF}' | sort -n | tail -1          # deepest path
git ls-files | awk '{ if (length($0) > m) m = length($0) } END { print m }'   # longest path
git tag | wc -l; git branch -r | wc -l                           # tags and remote branches
git rev-list --count HEAD                                        # commits
git ls-files | wc -l                                             # files in the tree
test -f .gitmodules && git config -f .gitmodules --get-regexp path | wc -l   # submodules
git lfs ls-files 2>/dev/null | wc -l                             # LFS files (needs git-lfs)
```

If a trait in the table is wrong, change `tools/stress-repos.txt` and this table together.
