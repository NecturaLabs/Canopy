# Architecture

Canopy is a Cargo workspace with three crates. The split is strict: everything that knows about Git
lives in `canopy-core`, everything that draws lives in `canopy-app`, and the only bridge is a
background worker per repository. `canopy-hosting` is the one HTTP client: the GitHub and GitLab
REST APIs and Canopy's update manifest. `canopy-core` never makes an HTTP request, and
`canopy-app` reaches the network only through `git` and `canopy-hosting` (plus the opt-in crash
report transport).

```text
┌──────────────────────────── canopy-app ─────────────────────────────┐
│  main.rs ─▶ CanopyApp (app/)   ─▶ views/* ─▶ ui/* (theme, widgets)  │
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
| `error.rs` | `Error` and `Result` (thiserror). Wraps libgit2 errors, I/O errors and failed `git` invocations with their stderr; `NotDownloaded` for objects a partial clone has not fetched, `UnsafePath` for a path `paths.rs` refused. |
| `model.rs` | Plain data passed to the UI: `RepoSnapshot`, `BranchInfo`, `RemoteInfo`, `TagInfo`, `StashInfo`, `FileEntry`, `WorkingStatus`, `CommitSummary`, `CommitDetails`, `RefLabel`. |
| `repo.rs` | `Repo`: open, discover (walking upward), init. `snapshot()` gathers HEAD, repository state, branches with ahead/behind, remotes, tags, stashes, status and the first N commits in topological order. |
| `graph.rs` | Lane layout for the commit graph (below). Unit tested. |
| `diff.rs` | `FileDiff`, `Hunk`, `DiffLine`, `Whitespace`; context and whitespace options; builds single-hunk patches for hunk staging. |
| `lines.rs` | Line-level staging: stage, unstage or discard chosen lines of one hunk, as an edited patch piped to `git apply`. |
| `status.rs` | Working-tree status split into staged, unstaged and conflicted, with file mode and type changes and a limit above which untracked folders collapse into one entry (`untracked_children` expands one level). In a partial clone it never downloads; a missing `HEAD` tree is `Error::NotDownloaded`, and `snapshot()` then carries `status_unavailable` instead of failing. |
| `partial.rs` | Partial clones: recognising them (`promisor` remotes, `*.promisor` packs) and downloading the objects a read needs through `git fetch --filter=blob:none --stdin` via `cli.rs`, the command git itself uses for a lazy fetch. Reads (`commit_details`, `file_diff`, `status`) run libgit2 first and take a `MissingObjects` argument: `Fail` stays offline and returns `Error::NotDownloaded`, `Download` fetches exactly the missing trees and blobs and retries. Remote names that start with `-` are refused. |
| `ops.rs` | Every mutation, run through the `git` executable via `cli.rs`: stage, unstage, discard (paths passed literally); hunk and line stage/unstage/discard (`git apply`); commit and amend; branch create, checkout, rename, delete (`-d`, or `-D` as a separate call), publish; merge in four modes, rebase, continue/abort; tag create, delete, push; stash save, apply, pop, drop; reset soft, mixed, hard; detached checkout; cherry-pick; revert; remotes; clone, fetch, pull, push; fetching a pull request's head. |
| `conflict.rs` | Merge conflicts: parsing conflict markers, rendering a partial resolution, and resolving a conflicted path (the one plain file write, saving a hand-edited resolution, goes through `paths::safe_join`, and staging it goes through git). |
| `paths.rs` | `safe_join`: the only way `canopy-core` touches a file inside a working tree. Refuses empty, absolute, `..` and `.git` paths and any symbolic link component. |
| `ignore.rs` | Ignore rules: add a rule to `.gitignore`, `info/exclude` or the global file, preview what it would ignore, ask git why a path is ignored (`check-ignore -v`), stop tracking a path. |
| `flow.rs` | git-flow without the `git flow` binary: the AVH config keys, start and finish for feature, bugfix, release and hotfix, with a `merge-tree` pre-flight that refuses on conflicts. |
| `merged.rs` | Which local branches are merged into a base (reachable, upstream gone, squash-merged) and which check proved it. |
| `lfs.rs` | Git LFS awareness without `git-lfs`: pointer parsing, which paths use `filter=lfs`, and whether an object has been downloaded. |
| `submodule.rs` | Submodule status, update, sync, add and remove, and the commit pair a submodule's diff shows. |
| `forge.rs` | Recognises GitHub and GitLab remotes and builds their web, request and clone URLs. Pure string handling; credentials in a remote URL are dropped, never emitted. |
| `update.rs` | Build numbers for the update check: `parse_build` reads one (digits only; a semantic version has none) and `is_newer` orders builds as integers. |
| `cli.rs` | Runs the `git` executable non-interactively (`GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true`, no console window on Windows), returns git's stderr on failure and streams `--progress` output line by line for long operations. Registers Canopy's askpass hook and redacts URLs and arguments for the log (`redact_url`, `loggable`). |

The app still calls `delete_branch(name, force)` and `stage_all()`, thin wrappers over the newer
calls. It does not yet use `ignore.rs`, `flow.rs`, `merged.rs`, `lfs.rs`, `submodule.rs`,
`paths::safe_join`, `publish_branch`, `untracked_children`, `stage_all_except_repositories` or the
file-mode fields outside the core's own code and tests; the backlog entries for those features
are the UI work.

## canopy-hosting

Blocking (no async runtime) and synchronous like `canopy-core`; callers run it on a background
thread.

| Module | Responsibility |
|---|---|
| `client.rs` | `Client`: provider-neutral entry point for listing, reading, creating, merging, closing, commenting on and approving pull and merge requests. |
| `github.rs`, `gitlab.rs` | The two providers' REST calls (GitHub API v3, GitLab API v4 including self-hosted hosts). |
| `http.rs` | One `ureq` agent per client, the common headers, timeouts, and HTTP failures mapped to `Error` with the server's own message. The token travels only in an `Authorization` header. |
| `model.rs` | Provider-neutral request, review, check and comment types. |
| `token.rs` | Finding a token: provider environment variables (offered only to their own host) or git's credential helpers. |
| `updates.rs` | Canopy's own update manifest, fetched anonymously from the public `NecturaLabs/app-updates` repository. |
| `error.rs` | Errors, carrying the server's message. |

## canopy-app

| Module | Responsibility |
|---|---|
| `main.rs` | Argument parsing, logging, eframe launch; also the entry point of the askpass client (the same executable, started by git or ssh). |
| `action.rs` | `Action`: what the user asked for during a frame. Views emit actions and never import `app`. |
| `app/mod.rs` | `CanopyApp`: state, start-up, the frame (`eframe::App::ui`), opening repositories, theme. |
| `app/input.rs` | Keyboard: bare keys, chords and the commands they run. |
| `app/palette.rs` | The command palette's entries. |
| `app/dispatch.rs` | Applies the actions a frame collected. |
| `app/background.rs` | Polled every frame: worker and hosting events, credential prompts, the update check, auto-fetch. |
| `app/open.rs` | The opener thread: opens and creates repositories in request order, off the UI thread. |
| `app/clone.rs`, `app/request_flow.rs` | Cloning on a background thread; creating and checking out pull and merge requests. |
| `app/screenshot.rs` | Headless screenshots for `--screenshot`. |
| `tab.rs` | `RepoTab`: one open repository's snapshot, selections, in-flight work and the state a failed or superseded job leaves behind. |
| `worker.rs` | One background thread per open repository; `Job` in, `Event` out over crossbeam channels; queued refreshes are coalesced, and of the queued `Job::Details` and `Job::Diff` only the last runs; debounced file watcher. In a partial clone a `Details`, `Diff`, `RefreshStatus` or `Refresh` job may download objects on this thread (network), with an `Event::Progress` line around it. |
| `hosting.rs` | A background thread per repository tab for pull and merge requests through `canopy-hosting` (`HostJob`), and the token store (`tokens.json`, per host and port). |
| `askpass.rs` | Credential prompts for git and ssh: Canopy's own executable is `GIT_ASKPASS`/`SSH_ASKPASS`, and the running app answers over loopback (ADR 0005). |
| `updater.rs` | The update check against the manifest, and the background install (`Updater::install`, `InstallState`); nothing is downloaded until the user chooses Update. |
| `selfupdate.rs` | The in-app update ([ADR 0006](adr/0006-canopy-updates-itself-in-place.md)): detects an installer-managed install (`.canopy-install` receipt), downloads and verifies the archive, unpacks it, swaps the install folder in two phases with rollback and refreshes the receipt and registered desktop entry and icons; elsewhere runs the one-line installer; and starts the new program after the window closed. |
| `build_info.rs` | The build number and short commit hash embedded by `build.rs`, and the release notes the release workflow embeds. |
| `crash_reports.rs` | Opt-in, scrubbed crash reports to Sentry, and what a report may carry (`DATA_VERSION`). |
| `logging.rs` | Structured per-run JSON-lines logs, timed spans, local crash files and the diagnostics Settings copies. |
| `highlight.rs` | Diff syntax highlighting (`syntect`) on two shared highlighter threads, cached per file revision; stale requests are abandoned. |
| `keymap.rs` | Every keyboard shortcut in one table (`BINDINGS`) that dispatches keys and renders the cheatsheet and Settings → Keyboard; see `docs/keybindings.md`. |
| `settings.rs` | Settings persisted through eframe storage: appearance, Git, integrations, updates, privacy answers, recent and open repositories. |
| `platform.rs` | Handing things to the operating system (web pages, folders, files in an editor, a terminal) and reading its settings, such as reduced motion, off the UI thread. |
| `ui/theme.rs` | Design tokens: the seven palettes, lane colours, syntax colours, spacing, radii. The only place colours are defined. |
| `ui/motion.rs` | Motion tokens (100/150/200 ms), the reduce-motion preference and whether motion is currently reduced. |
| `ui/fonts.rs` | Registers the bundled Inter, JetBrains Mono and Phosphor fonts. |
| `ui/art.rs` | The SVG art in `crates/canopy-app/assets/art/`, rendered through egui's image loaders. |
| `ui/widgets.rs` | Ref pills, avatars, icon buttons, badges. |
| `ui/folder_picker.rs` | Native folder selection on a background thread; a bounded result channel and requesting-field lifetime keep the event loop responsive. |
| `ui/toast.rs` | Toast queue and animation. |
| `ui/palette.rs` | Command palette with fuzzy matching; entries built on opening, ranked on query change. |
| `ui/cheatsheet.rs` | The keyboard cheatsheet, drawn from `keymap::BINDINGS`. |
| `ui/legal.rs` | The Terms and Privacy Policy as shipped (`docs/legal/`) and the versions a user accepts. |
| `ui/notes.rs` | Release notes ("What's new") and the small Markdown renderer for notes and request text. |
| `views/welcome.rs` | Welcome screen and recent repositories. |
| `views/toolbar.rs` | Tab strip and top toolbar: fetch, pull, push, branch, stash, view switch. |
| `views/sidebar.rs` | Branches, remotes, tags and stashes. |
| `views/history.rs` | Virtualised commit-graph table. |
| `views/commit_detail.rs` | Selected commit: metadata, parents, changed files, diff. |
| `views/changes.rs` | Staging area and commit composer. |
| `views/diff.rs` | Unified and side-by-side diff viewer with line numbers, hunk and line actions; row layout cached per diff. |
| `views/resolve.rs` | Conflict resolver; off-screen cards skipped by measured height. |
| `views/requests.rs` | Pull and merge requests: list, details, checks, reviews and actions. |
| `views/settings_screen.rs` | The settings screen: appearance, Git, integrations, accounts, updates, privacy, keyboard. |
| `views/dialogs/mod.rs` | `Dialog` and `show`: the modal dialogs (clone, new branch, confirmations, consent, ...). |
| `views/dialogs/stack.rs` | `DialogStack`: credentials in front, the consent never replaced, a closed prompt always answered. |
| `views/dialogs/{url,start,credential,update,common}.rs` | The remote URL builder, where a new branch starts, credential prompts, the update dialog (what is new, installing, restarting), shared fields and buttons. |
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
- Clone destination validation runs on the clone thread. The submitted form waits for completion;
  failure preserves its input, success closes it, and Continue in background closes only the form.
- Native folder choosers run on a separate thread and return through a bounded channel polled
  with `try_recv`. Dialogs share a picker; Settings owns its own. Closing the requesting field
  drops its receiver, so a late result cannot update a replacement field.
- Each open repository owns one worker thread with its own `git2::Repository` handle
  (`Repository` is not `Sync`). Jobs for one repository therefore run in order, which keeps
  operations like "stage, then commit" consistent without locking.
- `Job` and `Event` travel over `crossbeam-channel` channels. After sending a result the worker
  calls `request_repaint` on the egui context so an idle UI wakes up.
- Long-running CLI jobs (clone, fetch, push, ...) stream progress lines back as intermediate
  results; the status bar shows the running job with a spinner.
- Closing a tab drops the job sender; the worker finishes its current job and exits.
- In a partial clone, reads that need objects the clone left on the server (`Details`, `Diff`,
  and a snapshot or status after a soft reset) fetch them on the worker through `git`, so the
  thread can wait for the network while later jobs queue behind it. The worker runs each read
  offline first and downloads only when that fails with `Error::NotDownloaded` (`can_download()`),
  skips a queued `Details` or `Diff` that a later one supersedes, and shows
  "Downloading from <remote>…" in the status bar meanwhile. A commit's file list is sent before
  its line counts, which are fetched in a follow-up unless the user already moved on. A failed
  `Details` is kept in the tab (`details_error`) and shown in the details pane, with a Retry
  button when downloading can fix it.

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

- **libgit2 for reads.** Status, revwalks, diffs and refs are fast, structured and need no output
  parsing. Every mutation runs the `git` executable (ADR 0001); the one plain file write is
  saving a hand-edited conflict resolution (`conflict::write_resolution`), and staging it still
  goes through git. `git2` is built with `default-features = false`, so no
  OpenSSL or libssh2 is linked on any platform.
- **The `git` CLI for every mutation and all network traffic.** Running the user's own `git` honours
  credential helpers, the SSH agent and `~/.ssh/config`, `includeIf`, hooks, proxies and
  `url.<base>.insteadOf` exactly as the terminal does. libgit2 reimplements these partially and
  differently. Fork and Sourcetree make the same choice. Merge and rebase also use the CLI so
  that behaviour, hooks and conflict state match what users see in the terminal.
- `GIT_TERMINAL_PROMPT=0` makes git fail instead of waiting for terminal input. Credential,
  passphrase and host-key questions come to Canopy's own prompt instead (`askpass.rs`, ADR 0005).
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

Settings (appearance, Git and integration preferences, the terms and crash-report answers, recent
and open repositories) are serialised with serde through eframe's storage under the key
`canopy-settings-v1`, which writes to the platform's application data directory
(`~/.local/share/canopy/` on Linux). Saved hosting tokens are in `tokens.json` beside it (mode 0600
on Unix, not the operating system's keychain), and `logs/` and `crashes/` hold the per-run logs
and local crash reports. Repository state is never cached on disk; it is always read from the
repository. The backlog's "Persistent, secured storage" entry moves all of this into
Canopy-specific folders.

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
- CI runs the suite on Linux; macOS and Windows are switched on by repository variables (see
  `.github/workflows/ci.yml`). `tools/check.sh` runs the local equivalent.

## Performance

- The snapshot loads the first 2000 commits by default; "Load more" extends the walk. Graph layout
  for those rows is computed once per snapshot, on the worker.
- Large results cross from the worker as `Arc`s (snapshot, commit details, diff, conflict file)
  and the tab keeps them that way: a frame clones pointers, never the data. Facts a frame would
  otherwise recompute from the snapshot (remote branches per remote, graph width) are derived
  once when it arrives (`tab::SnapIndex`).
- Every list that grows with the repository is virtualised: the history table, the sidebar's
  branches, remote branches, tags and stashes, changed files (Changes and commit details) and the
  request list lay out and paint only the rows in view (`commit_detail::virtual_rows`), so a
  frame costs the same for 2000 or 200,000 commits or tags.
- Diffs are computed on the worker when a file is selected and kept in the tab until the
  selection or the repository changes; replies for a superseded selection are dropped. The diff
  viewer builds its row layout (rows, positions, widest line, gutter width) once per diff and
  layout (`diff::DiffLayout`) and finds the visible rows by binary search, so scrolling a
  50,000-line diff costs the same as a short one. Galleys are left to egui's galley cache.
- The conflict resolver paints only the visible lines of each block and skips cards out of view,
  using their measured heights (estimates until first drawn).
- Syntax highlighting runs on two highlighter threads shared by all tabs (`highlight.rs`), never
  on the UI thread (only if neither thread could start does it run on the repository thread); the diff shows as plain text until the colours arrive, and a
  request superseded by a newer diff stops early. Results are cached per file revision.
- The command palette builds its entries when it opens (or when a snapshot or request list
  arrives while it is open) and ranks them only when the query changes.
- No per-frame allocation of large collections and no repository access per frame.
  `cargo test --release -p canopy-app frame_cost -- --ignored --nocapture` renders the real views
  over a synthetic huge repository and prints the CPU time per frame.
- `tools/mkfixture.sh <dir> big` creates a 5000-commit repository with regular merges for
  checking layout and scrolling.
