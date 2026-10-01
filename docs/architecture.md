# Architecture

Canopy is a Cargo workspace with two crates. The split is strict: everything that knows about Git
lives in `canopy-core`, everything that draws lives in `canopy-app`, and the only bridge is a
background worker per repository.

```text
┌──────────────────────────── canopy-app ─────────────────────────────┐
│  main.rs ─▶ CanopyApp (app.rs) ─▶ views/* ─▶ ui/* (theme, widgets)  │
│                     │  ▲                                            │
│               Job   │  │  Event        (crossbeam channels)     │
│                     ▼  │                                            │
│              worker.rs: one thread per repository + file watcher   │
└─────────────────────────────┬───────────────────────────────────────┘
                              │ plain function calls
┌──────────────────────────── canopy-core ────────────────────────────┐
│  repo.rs  status.rs  diff.rs  graph.rs  ops.rs  │  cli.rs           │
│            libgit2 (git2, no network)           │  `git` executable │
└─────────────────────────────────────────────────────────────────────┘
```

## canopy-core

UI-agnostic and synchronous. It returns plain data and never spawns threads of its own (except the
`git` child processes in `cli.rs`).

| Module | Responsibility |
|---|---|
| `lib.rs` | Public re-exports. |
| `error.rs` | `Error` and `Result` (thiserror). Wraps libgit2 errors, I/O errors and failed `git` invocations with their stderr. |
| `model.rs` | Plain data passed to the UI: `RepoSnapshot`, `BranchInfo`, `RemoteInfo`, `TagInfo`, `StashInfo`, `FileEntry`, `CommitSummary`, `CommitDetails`, `RefLabel`. |
| `repo.rs` | `Repo`: open, discover (walking upward), init. `snapshot()` gathers HEAD, repository state, branches with ahead/behind, remotes, tags, stashes, status and the first N commits in topological order. |
| `graph.rs` | Lane layout for the commit graph (below). Unit tested. |
| `diff.rs` | `FileDiff`, `Hunk`, `DiffLine`; builds single-hunk patches for hunk staging. |
| `status.rs` | Working-tree status split into staged, unstaged and conflicted. |
| `ops.rs` | Every mutation, run through the `git` executable via `cli.rs`: stage, unstage, discard; hunk stage/unstage/discard (`git apply`); commit and amend; branch create, checkout, rename, delete; merge, rebase, continue/abort; tag create, delete, push; stash save, apply, pop, drop; reset soft, mixed, hard; detached checkout; cherry-pick; revert; remotes; clone, fetch, pull, push. |
| `cli.rs` | Runs the `git` executable non-interactively (`GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true`, no console window on Windows), returns git's stderr on failure and streams `--progress` output line by line for long operations. |

## canopy-app

| Module | Responsibility |
|---|---|
| `main.rs` | Argument parsing, logging (`tracing-subscriber`, filter from `CANOPY_LOG` or `RUST_LOG`), eframe launch. |
| `app.rs` | `CanopyApp`: repository tabs, global shortcuts, dialogs, toasts, command palette. |
| `worker.rs` | One background thread per open repository; `Job` in, `Event` out over crossbeam channels; queued refreshes are coalesced; debounced file watcher. |
| `settings.rs` | Settings persisted through eframe storage: theme, recent repositories, UI scale. |
| `ui/theme.rs` | Design tokens: palette, lane colours, spacing, radii. The only place colours are defined. |
| `ui/fonts.rs` | Registers the bundled Inter and JetBrains Mono fonts. |
| `ui/widgets.rs` | Ref pills, avatars, icon buttons, badges. |
| `ui/toast.rs` | Toast queue and animation. |
| `ui/palette.rs` | Command palette with fuzzy matching. |
| `views/welcome.rs` | Welcome screen and recent repositories. |
| `views/toolbar.rs` | Top toolbar: fetch, pull, push, branch, stash, view switch. |
| `views/sidebar.rs` | Branches, remotes, tags and stashes. |
| `views/history.rs` | Virtualised commit-graph table. |
| `views/commit_detail.rs` | Selected commit: metadata, parents, changed files, diff. |
| `views/changes.rs` | Staging area and commit composer. |
| `views/diff.rs` | Diff viewer with line numbers and hunk actions. |
| `views/dialogs.rs` | Modal dialogs (clone, new branch, confirmations, ...). |
| `views/statusbar.rs` | Branch, ahead/behind, repository state, running job. |

## Threading model

```text
 UI thread (eframe)                       worker thread (per repository)
 ──────────────────                       ──────────────────────────────
 input ─▶ CanopyApp::ui                   loop {
          │                                 job = jobs.recv()
          │ worker.send(Job::Op(..)) ─────▶  event = run(job)    // canopy-core
          │                                 events.send(event)
          │                                 ctx.request_repaint()
          ◀───────────── Event ──────────── }
          │ try_recv() each frame
          ▼                               notify watcher ─(debounce)─▶ Job::Refresh
 tab state ─▶ views render
```

- The UI thread never calls git, libgit2 or reads repository files. A frame only reads tab state,
  so rendering cost does not depend on repository size or disk speed.
- Each open repository owns one worker thread with its own `git2::Repository` handle
  (`Repository` is not `Sync`). Jobs for one repository therefore run in order, which keeps
  operations like "stage, then commit" consistent without locking.
- `Job` and `Event` travel over `crossbeam-channel` channels. After sending a result the worker
  calls `request_repaint` on the egui context so an idle UI wakes up.
- Long-running CLI jobs (clone, fetch, push, ...) stream progress lines back as intermediate
  results; the status bar shows the running job with a spinner.
- Closing a tab drops the job sender; the worker finishes its current job and exits.

## Data flow

1. A user action (click, shortcut, palette command) builds a `Job`, for example
   `Job::Checkout { branch }`, and sends it to the tab's worker. Destructive jobs are only sent
   after the confirmation dialog.
2. The worker calls the matching `canopy-core` function.
3. Mutating jobs are followed by a fresh `snapshot()`, so the UI always receives a consistent
   `RepoSnapshot` rather than patching its own copy.
4. The `Event` (new snapshot, details, diff, progress line or error) is applied to the tab
   state on the next frame; errors become toasts.

## Commit graph layout

`graph.rs` turns commits in topological order (newest first) into rows the history view can draw
without any further computation.

- **Lanes** are columns. The layout keeps a vector of active lanes; each holds the commit ID it is
  waiting for (the next expected parent) or is free.
- **Placing a commit**: if one or more lanes wait for this commit, the leftmost becomes the node
  lane and the others end here (they converge into the node). Otherwise the commit is a branch
  tip and takes the first free lane, or a new lane on the right.
- **First parent keeps the lane**: after placing a commit, its node lane now waits for its first
  parent, so a branch's mainline stays in one straight column.
- **Merge parents**: each further parent either joins a lane that already waits for it (an
  existing lane) or is assigned the first free lane (a new lane). This draws the merge edge
  toward the parent's column.
- **Colours** are allocated per lane assignment, not per column: when a lane starts a new line of
  history it takes the next colour index from the ten lane colours, and the colour stays with that
  line while it continues. A column reused later can get a different colour.
- **Rows store segments**: each row records its node lane and colour index plus line segments in
  two halves, top to middle (edges arriving from the row above) and middle to bottom (edges leaving
  toward the row below), each with a start lane, end lane and colour. The view draws straight
  segments as lines and lane changes as cubic curves; it does no graph reasoning of its own.
- Trailing free lanes are trimmed so the graph column is only as wide as needed.

The layout is linear in the number of commits times the number of active lanes and runs on the
worker as part of the snapshot. It is covered by unit tests for linear history, branch and merge,
octopus merges, multiple roots and lane reuse.

## Why libgit2 and the git CLI

- **libgit2 for reads and local writes.** Status, revwalks, diffs, refs and index updates are fast,
  structured and need no output parsing. `git2` is built with `default-features = false`, so no
  OpenSSL or libssh2 is linked on any platform.
- **The `git` CLI for network and history rewriting.** Running the user's own `git` honours
  credential helpers, the SSH agent and `~/.ssh/config`, `includeIf`, hooks, proxies and
  `url.<base>.insteadOf` exactly as the terminal does. libgit2 reimplements these partially and
  differently. Fork and Sourcetree make the same choice. Merge and rebase also use the CLI so
  that behaviour, hooks and conflict state match what users see in the terminal.
- `GIT_TERMINAL_PROMPT=0` makes git fail instead of waiting for terminal input. A built-in
  credential prompt (`GIT_ASKPASS`) is on the backlog.
- Hunk staging builds a single-hunk patch in `diff.rs` and applies it with
  `git apply --cached`, which handles context and whitespace the same way `git add -p` does.

Runtime requirement: `git` 2.30 or newer on `PATH`.

## File watching and refresh

`worker.rs` runs a `notify` watcher on the working tree (which includes `.git`). Events are
debounced (350 ms of quiet, so a checkout or a build yields one refresh) and classified: a change
inside `.git` other than `objects/`, `logs/` and `*.lock` files asks for a full snapshot
(`Job::Refresh`), a change to a file that is not ignored by `.gitignore` asks only for a status
refresh (`Job::RefreshStatus`), and changes to ignored paths such as `target/` are dropped. The
worker coalesces queued refreshes, so a burst of requests costs one read. Canopy also refreshes when its window regains focus,
which covers file systems where watching is unreliable (network mounts, some WSL setups).

## Persistence

Settings (theme, recent repositories, UI scale) are serialised with serde through eframe's
storage, which writes to the platform's application data directory. Repository state is never
cached on disk; it is always read from the repository.

## Error handling

- `canopy-core` returns `canopy_core::Result<T>`. CLI failures carry the exit status and git's
  stderr unchanged, because git's own message is usually the most useful one.
- The worker converts errors into `Event` errors; the UI shows them as toasts and logs them
  with `tracing`. Nothing in the app panics on a Git error.
- `unwrap()` and `expect()` are not used outside tests (clippy `unwrap_used` is enabled) and
  `unsafe` is forbidden.

## Testing strategy

- **canopy-core**: every operation is tested against a temporary repository (`tempfile`) created
  in the test, with a fixed identity. Tests assert on repository state (refs, index, working tree)
  and, where semantics are subtle, compare with what the `git` CLI produces.
- **Graph layout**: pure unit tests over synthetic commit lists.
- **canopy-app**: logic that can be separated from drawing (fuzzy matching, argument parsing,
  settings) is unit tested. Rendering is verified by screenshots of deterministic fixtures
  (`tools/mkfixture.sh` and `tools/screenshot.sh`), which agents and reviewers inspect.
- CI runs the suite on Linux, macOS and Windows.

## Performance

- The snapshot loads the first 2000 commits by default; "Load more" extends the walk. Graph layout
  for those rows is computed once per snapshot, on the worker.
- The history table is virtualised: only visible rows are laid out and painted, so a frame costs
  the same for 2000 or 200,000 loaded commits.
- Diffs are computed on the worker when a file is selected and kept in the tab until the
  selection or the repository changes; replies for a superseded selection are dropped. The diff
  viewer paints only the visible lines, so scrolling a large diff does no Git work and stays cheap.
- No per-frame allocation of large collections and no repository access per frame.
- `tools/mkfixture.sh <dir> big` creates a 5000-commit repository with regular merges for
  checking layout and scrolling.
