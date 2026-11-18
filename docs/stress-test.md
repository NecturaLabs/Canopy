# Stress test: 30 repositories at once

## Purpose

Canopy must stay fast and stable with a lot open. The owner's requirement: **30 different
projects open at the same time, one per tab, 2 large, 20 medium and the rest small, with concurrent
operations running without issues or performance hiccups** (lowered from 100 repositories,
including monstrous ones, on 2026-11-18). This document is the procedure for
checking that on a real machine, and the pass criteria. The tooling is:

| File | Role |
|---|---|
| `tools/stress-repos.txt` | The 30 repositories, one `owner/repo size-class` per line. The single source; the table below mirrors it. |
| `tools/stress.sh` | `clone` the repositories, `open` Canopy on all of them while sampling resources, `churn` the clones from outside, `report` a run. |
| `tools/stress_report.py` | Reads Canopy's JSON log and the sampler CSV and prints a summary and a verdict. Standard library only. |

The test is run by hand on a desktop session (it needs a display and the real GPU path; software
rendering says nothing about frame times). It is not part of CI: the clones take several gigabytes and the run needs a desktop.

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
7. **Log size.** A run's log stops at 20 MiB. At the Detailed level, 30 tabs and heavy churn can
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

Thirty public repositories on github.com, all verified to exist (and not to be forks) when
this list was written. Sizes are what GitHub reports for the repository (an upper-bound style
figure: a real clone is often somewhat smaller, and Git LFS objects are not included). The size
class is a rough bucket by size: **large** (about 300 MB to 1 GB), **medium** (about 40 to
300 MB), **small** (under 40 MB). 2 large, 20 medium, 8 small. `tools/stress.sh` also accepts a
**monstrous** class (about 1 GB or more, or a very long history) for a list of your own
(`--list`); the standard list has none.

Where "why it is here" says "probably" or "likely", the trait is expected but was not checked
file by file; the one-liners at the end of this document verify it after cloning.

| # | Repository | Class | GitHub size | Why it is here |
|---:|---|---|---:|---|
| 1 | [`python/cpython`](https://github.com/python/cpython) | large | 862 MB | long history, many branches and tags, C and Python mix |
| 2 | [`DefinitelyTyped/DefinitelyTyped`](https://github.com/DefinitelyTyped/DefinitelyTyped) | large | 809 MB | tens of thousands of tiny files across thousands of packages, constant churn |
| 3 | [`numpy/numpy`](https://github.com/numpy/numpy) | medium | 180 MB | C and Python, many tags, uses submodules for vendored code |
| 4 | [`denoland/deno`](https://github.com/denoland/deno) | medium | 247 MB | Rust monorepo that uses submodules for test suites |
| 5 | [`redis/redis`](https://github.com/redis/redis) | medium | 212 MB | dense C history, many tags |
| 6 | [`chinese-poetry/chinese-poetry`](https://github.com/chinese-poetry/chinese-poetry) | medium | 222 MB | CJK directory and file names, very large JSON files |
| 7 | [`spring-projects/spring-framework`](https://github.com/spring-projects/spring-framework) | medium | 240 MB | deep Java package paths, many tags |
| 8 | [`curl/curl`](https://github.com/curl/curl) | medium | 145 MB | very long history, signed tags |
| 9 | [`jesseduffield/lazygit`](https://github.com/jesseduffield/lazygit) | medium | 155 MB | another Git client (a TUI); committed vendor folder with thousands of files |
| 10 | [`FortAwesome/Font-Awesome`](https://github.com/FortAwesome/Font-Awesome) | medium | 159 MB | font and SVG binaries |
| 11 | [`microsoft/Windows-driver-samples`](https://github.com/microsoft/Windows-driver-samples) | medium | 175 MB | many sample projects, Windows-style paths and (likely) CRLF |
| 12 | [`PowerShell/PowerShell`](https://github.com/PowerShell/PowerShell) | medium | 107 MB | Windows origin, mixed CRLF and LF |
| 13 | [`obsproject/obs-studio`](https://github.com/obsproject/obs-studio) | medium | 81 MB | C/C++ with many translation files |
| 14 | [`libgit2/libgit2`](https://github.com/libgit2/libgit2) | medium | 74 MB | the library Canopy reads repositories with; also a cross-check for odd repository states |
| 15 | [`rust-lang/cargo`](https://github.com/rust-lang/cargo) | medium | 73 MB | Rust, merge-heavy, many tags |
| 16 | [`simple-icons/simple-icons`](https://github.com/simple-icons/simple-icons) | medium | 68 MB | thousands of SVG files in one directory |
| 17 | [`mattn/go-sqlite3`](https://github.com/mattn/go-sqlite3) | medium | 48 MB | contains the multi-megabyte SQLite amalgamation: giant single-file diffs, very long lines |
| 18 | [`vuejs/core`](https://github.com/vuejs/core) | medium | 45 MB | TypeScript monorepo, many tags |
| 19 | [`sharkdp/bat`](https://github.com/sharkdp/bat) | medium | 34 MB | submodules for syntax and theme assets |
| 20 | [`django/django`](https://github.com/django/django) | medium | 276 MB | long history, many branches and tags |
| 21 | [`nlohmann/json`](https://github.com/nlohmann/json) | medium | 297 MB | a single ~1 MB header (single_include/json.hpp): very long diffs; large test data |
| 22 | [`o3de/o3de`](https://github.com/o3de/o3de) | medium | 229 MB | uses Git LFS for assets (install git-lfs to get the real files) |
| 23 | [`octocat/Hello-World`](https://github.com/octocat/Hello-World) | small | < 1 MB | tiny; a few commits and branches |
| 24 | [`kelseyhightower/nocode`](https://github.com/kelseyhightower/nocode) | small | < 1 MB | almost empty tree |
| 25 | [`antirez/kilo`](https://github.com/antirez/kilo) | small | < 1 MB | one C source file |
| 26 | [`github/gitignore`](https://github.com/github/gitignore) | small | 3 MB | hundreds of small template files in one folder |
| 27 | [`leachim6/hello-world`](https://github.com/leachim6/hello-world) | small | 5 MB | hundreds of tiny files in many languages, unusual file names |
| 28 | [`nothings/stb`](https://github.com/nothings/stb) | small | 6 MB | single-header C libraries: very long files |
| 29 | [`ohmyzsh/ohmyzsh`](https://github.com/ohmyzsh/ohmyzsh) | small | 14 MB | shell plugins, very many contributors |
| 30 | [`tmux/tmux`](https://github.com/tmux/tmux) | small | 23 MB | C, long history |

### Disk needed

By GitHub-reported size, full history: large about 1.7 GB, medium about 3.1 GB, small about
50 MB: **about 5 GB in all**, without `o3de/o3de`'s Git LFS assets. Working trees come on top of the history for a full clone (the figures
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
- `--shallow-for owner/repo`: shallow for named repositories only; add other names with a comma.
  The recommended first run is the full set with full history.
- `--subset N`: N evenly spaced entries of the list, so every size class is represented
  (`--subset 10` gives one large, seven medium and two small). Use it to rehearse the procedure
  before committing to all 30.
- `--class large,medium`, `--exclude owner/repo`: narrower selections.

Git LFS: `o3de/o3de` and possibly others store assets in LFS. With `git-lfs` installed the clone
downloads them; to skip, run the clone as `GIT_LFS_SKIP_SMUDGE=1 tools/stress.sh clone ...`.

## Setup

1. **Build**: `cargo build --release -p canopy-app` (the stress binary is the release one). The
   script reads `target/release/canopy`, or `--bin`, or `$CANOPY_BIN`.
2. **Raise the limits.** One inotify instance serves every tab, but each tab's whole tree,
   including `.git`, is watched folder by folder, so large trees need many watches.
   For this session:

   ```sh
   sudo sysctl fs.inotify.max_user_watches=2097152
   ulimit -n 65535        # in the shell that starts Canopy
   ```

   `open` warns when these are low. Record the values; the pass criteria are about Canopy staying
   inside them, so also run once with the stock limits and note what fails and how it shows up
   (see instrumentation item 4).
3. **Check git**: `git --version` (2.32 or newer), `git-lfs` optional.
4. **Clone** into the cache folder (default `~/.cache/canopy-stress`, or `--dir`, or
   `$CANOPY_STRESS_DIR`). Put it on a fast disk with the space above.

   ```sh
   tools/stress.sh clone --jobs 4                                     # about 6 GB
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

### (a) Cold open of all 30 tabs from the command line

```sh
sync; echo 3 | sudo tee /proc/sys/vm/drop_caches     # cold cache; skip for a warm run
tools/stress.sh open --label a-cold --duration 600
# touch nothing for the first minute (one tab loaded), then show every tab once (see below),
# leave it idle for five minutes, then close Canopy or let --duration do it
tools/stress.sh report
```

**Tabs load when first shown (#62).** Canopy loads the active tab at once and lists the other 29
by name only: no repository thread, file watcher or snapshot until a tab is first shown. So a run
that touches nothing measures one loaded tab (threads are about 1 per loaded tab plus the shared
threads; one inotify instance serves them all). To load them all, show each tab once, for example holding
`Ctrl+Tab` (or `xdotool key --repeat 30 --delay 300 ctrl+Tab`). The report's "all tabs loaded"
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
Hold `Ctrl+Tab` for 30 seconds so the tab strip cycles through all 30 several times; then switch
with `Ctrl+Tab` while the History and Changes views (`1`, `2`) alternate. With the optional
automation:

```sh
xdotool key --repeat 600 --delay 30 ctrl+Tab       # about 33 switches a second
```

Watch for: any visible stall, tabs that show stale or empty content after switching, the
strip itself being slow with 30 tabs. The frame criteria read `frame stats`.

### (c) Heavy views on the biggest repositories

For each of the two large repositories, `python/cpython` and `DefinitelyTyped/DefinitelyTyped`,
and the longest medium histories, `django/django`, `curl/curl` and `spring-projects/spring-framework`:

1. **Scroll the history** from the top to as far as it loads, with the wheel and with `j`/`k` and
   Page Down, for about a minute: `xdotool click --repeat 400 --delay 15 5` scrolls down.
   Note whether more history loads smoothly.
2. **Open huge commits and diffs.** Find big commits with
   `git -C <repo> log -n 20000 --format=%h --shortstat | awk '/^[0-9a-f]{7,}$/ {c=$1} /files? changed/ {print $1, c}' | sort -rn | head`
   and open the top few from the filter box (`Ctrl+F`, paste the hash). Open the largest
   files' diffs, toggle split view, whitespace modes and syntax highlighting.
3. **Changes view with thousands of modified files.** Make them from outside, in the large
   clones and a few medium ones, then look at them in Canopy and stage, unstage and discard them:

   ```sh
   tools/stress.sh churn --class large,medium --repos 4 --modify-many 5000
   ```

   Press `2` for Changes in those tabs; scroll the list; stage all, unstage all. Afterwards
   `tools/stress.sh churn --restore`.

Record the slowest actions. The log gives `details`, `diff` and `Staging` durations per repository;
`report` ranks the slowest repositories.

### (d) Concurrent operations: fetch everywhere, and external changes everywhere

Two things at once, for 10 to 15 minutes, with all 30 tabs open:

1. **Fetch in many tabs.** Easiest and most demanding: set Settings -> Git -> **Fetch in the
   background** to the shortest option (every 5 minutes). A tab's schedule starts when it first
   sees auto-fetch on (opening the tab, or turning the setting on, or changing the interval, with
   tabs already open), and its first fetch falls due between half the interval and a whole one
   later, by a hash of its path, so the 30 tabs' first fetches spread over the second half of
   the interval instead of one herd. Each tab then waits a whole interval after every fetch, its
   own or one you ran by hand. At most 4 of these automatic fetches run at once across all tabs
   (`MAX_BACKGROUND_FETCHES`): a tab that falls due while 4 run waits, keeping its due time, and
   starts when one finishes. So with 30 tabs the number of `git fetch` children in
   `samples.csv` stays at 4 or fewer (plus fetches you start by hand, which are never limited),
   and every tab still fetches once per interval; if some do not, the `operation started` events
   for `Fetching` show it. For a burst, press `f` (fetch) in tabs by hand or with
   `for i in $(seq 30); do xdotool key --delay 150 ctrl+Tab f; done`. This uses the network:
   failures from GitHub throttling are not Canopy's; use a local mirror (`--base-url`) when you
   need to separate the two.
2. **External process mutating the working trees**, so the file watchers fire everywhere:

   ```sh
   tools/stress.sh churn --repos 15 --duration 900 --interval 2
   ```

   Each of the 15 clones (evenly spread over all size classes) gets its own concurrent loop that
   creates, edits, renames and deletes files under `stress-churn/`, commits them, switches among
   `stress/a`, `stress/b` and the original branch, creates and deletes tags and branches, and
   appends lines to tracked files. `--deep` also checks out commits far back (full clones only).
   The action log goes into the run folder; `report` uses it to measure how long after each change
   the app showed it (refresh lag).

   The refreshes this causes in tabs that are not shown share a limit of 2 to 4 at a time (the
   number of cores, `read_limit.rs`), so with 15 tabs touched together the last one waits for the
   queue; the shown tab never waits. A refresh-lag failure that comes from this shows as
   `background refresh waited for its turn` events (DEBUG, with `ms`) in the log: read them
   before blaming git, and raise `MAX_PERMITS` only with a run that shows it helps.

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

All numbers are for the reference tier above, with 30 tabs open. `tools/stress_report.py`
prints each as PASS, FAIL, REVIEW or n/a; the thresholds are its `LIMITS` (override one with
`--limit NAME=VALUE` to try another figure). The report exits with status 1 when any criterion
fails.

| Criterion | Limit | Scenario |
|---|---|---|
| Panics, crash reports, ERROR log events | 0 | all |
| Canopy exit | clean (status 0, or 143 when stopped by `--duration`) | all |
| Warnings | each read and explained; unexpected ones fail the run (REVIEW until judged) | all |
| Every repository opens (no `cannot open the repository`) | 30 of 30 | a |
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
| Idle CPU, 30 tabs, nothing happening | median at most 2 %, p95 at most 5 % of one core | a, e |
| RSS in the minute after all tabs loaded | at most 2 GiB | a |
| Peak RSS over the whole run | at most 4 GiB | all |
| Heap growth during the soak | last five idle minutes at most 10 % and 50 MiB above the first five | e |
| Threads | peak at most 450 (about 1 per loaded tab, plus the UI and the shared watcher); back within 20 of baseline after churn | e |
| Child processes | peak at most 8 direct children of the process, sampled each second, when nothing is started by hand: at most 4 automatic fetches at once, plus the shown tab's own git work and a few short helpers (`--limit children_peak=N` for a run that includes the burst of fetches by hand in (d), which are never limited). All zero means the kernel has no `/proc/<pid>/task/<tid>/children` (`CONFIG_PROC_CHILDREN`) or nothing ran, and the report says n/a. This catches a herd, not a regression of the fetch limit alone: the staggered schedule keeps few fetches overlapping even without it | a, b, c, d (timer only), e |
| Open descriptors | peak at most 70 % of `ulimit -n`; back within 50 of baseline | e |
| Log not truncated | the 20 MiB cap not reached | all |

The memory and thread figures are first estimates from reading the code (one worker thread per
tab that has been shown, and one watcher thread and one inotify backend thread in all; a tab that
has not been shown has none, see "Tabs load when first shown" under (a)). The thread figure counts
threads that exist, which the limits on background work do not change: they bound how many of them
run git at once, so the child-process figure, not the thread one, shows them. The first real run
should set all of these properly. Tighten them to what the first clean run shows, with headroom, and write the values here.

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
