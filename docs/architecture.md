# Architecture

Canopy is a Cargo workspace with three crates. The split is strict: everything that knows about Git
lives in `canopy-core`, everything that draws lives in `canopy-app`, and the only bridge is a
background worker per repository. `canopy-hosting` is the one HTTP client: the GitHub and GitLab
REST APIs (plus one GraphQL query for a request list's checks and reviews) and Canopy's update
manifest. `canopy-core` never makes an HTTP request, and
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
| `message.rs` | Hints on a commit message before it is committed: a summary over 72 characters and description lines over 72 that cannot be wrapped (not indented, not quoted, more than one word). Pure; counts characters, not bytes. The composer shows them when Settings → Git → Check commit messages is on, and they never block the commit. |
| `lines.rs` | Line-level staging: stage, unstage or discard chosen lines of one hunk, as an edited patch piped to `git apply`. |
| `status.rs` | Working-tree status split into staged, unstaged and conflicted, with file mode and type changes and a limit above which untracked folders collapse into one entry (`untracked_children` expands one level). In a partial clone it never downloads; a missing `HEAD` tree is `Error::NotDownloaded`, and `snapshot()` then carries `status_unavailable` instead of failing. |
| `partial.rs` | Partial clones: recognising them (`promisor` remotes, `*.promisor` packs) and downloading the objects a read needs through `git fetch --filter=blob:none --stdin` via `cli.rs`, the command git itself uses for a lazy fetch. Reads (`commit_details`, `file_diff`, `status`) run libgit2 first and take a `MissingObjects` argument: `Fail` stays offline and returns `Error::NotDownloaded`, `Download` fetches exactly the missing trees and blobs and retries. Remote names that start with `-` are refused. `convert_legacy_partial_clone` rewrites an `extensions.partialClone` repository into the `promisor` form through `git config` (the app asks first and opens it afterwards). `is_in_trusted_folder` says whether a repository's working folder and git directory lie inside a trusted folder (paths resolved, a root never accepted); the app downloads on its own only then (ADR 0009). `fetch_programs` lists the configuration keys and hooks of a repository that make a fetch run a program; auto-fetch (`worker.rs` `perform`) refuses outside a trusted folder when it is not empty. |
| `ops.rs` | Every mutation, run through the `git` executable via `cli.rs`: stage, unstage, discard (paths passed literally); hunk and line stage/unstage/discard (`git apply`); commit and amend; branch create, checkout, rename, delete (`-d`, or `-D` as a separate call), publish; merge in four modes, rebase, continue/abort; tag create, delete, push; stash save, apply, pop, drop; reset soft, mixed, hard; detached checkout; cherry-pick; revert; remotes; clone, fetch, pull, push; fetching a pull request's head. `Repo::force_push_target` (`PushTarget`) names the remote branch a force push would overwrite, by the same rules as `push`. |
| `conflict.rs` | Merge conflicts: parsing conflict markers, rendering a partial resolution, and resolving a conflicted path (the one plain file write, saving a hand-edited resolution, goes through `paths::safe_join`, and staging it goes through git). |
| `pull.rs` | The user's effective `git pull` configuration (`pull.ff`, `branch.<name>.rebase`, `pull.rebase`, `merge.ff`), read through git with the setting, value, scope and file it comes from (`Repo::pull_config`, `PullConfig`, `ConfigSource`, `ConfigScope`): the default a pull strategy starts from. `ops.rs` always passes the chosen `PullStrategy` (five strategies) to git, and `pull_strategy` follows the configuration, else a fallback; `pull_fetched` brings in the upstream as `pull_preview` fetched it. |
| `pull_preview.rs` | What a pull would do, worked out before it runs (`Repo::pull_preview`, `PullPreview`, whose `tip` is the upstream commit it compared): fetches the upstream, counts commits each side has, finds conflicts with `git merge-tree --write-tree` when both moved on, and the incoming files that are also uncommitted; `Repo::compare_upstream` does the same without fetching. The worker asks the user before a pull that diverges under fast-forward only or would conflict, then brings in the fetched upstream with `Repo::pull_fetched` (ADR 0012); if the upstream moved since the preview it asks again instead. |
| `options.rs` | Canopy's git choices (`GitOptions`: follow the git configuration, autostash, follow-tags, submodule updates, `ConflictStyle`, `PushDefault`, `Cleanup`, tag pruning), kept on the `Repo` and set by the worker from Settings, and the flags and `-c` overrides each run gets: none for a choice the user's configuration sets while Canopy follows it, Canopy's choice otherwise (ADR 0012). |
| `paths.rs` | `safe_join`: the only way `canopy-core` touches a file inside a working tree. Refuses empty, absolute, `..` and `.git` paths and any symbolic link component. |
| `config.rs` | Shared bounded Git configuration reader with NUL-delimited scope/origin records, include expansion, canonical boolean reads and non-UTF-8 file paths; typed supported-key editing and direct-scope reset through the Git CLI; non-sensitive global recommendations with partial-failure reporting. |
| `identity.rs` | The identity Git will put into the next commit: author and committer from `git var`, each value's source (environment variable, configuration scope and file, or Git's guess from the computer) from one `git config --get-regexp`, and `set` / `clear` for `user.name` and `user.email` at the local or global scope. Canopy keeps no identity of its own and never writes `author.*` or `committer.*`. Inputs are validated before git runs; `looks_like_placeholder` only warns. |
| `ignore.rs` | Ignore rules: add a rule to `.gitignore`, `info/exclude` or the global file, preview what it would ignore, ask git why a path is ignored (`check-ignore -v`), stop tracking a path. Bounded ignored-path reads with collapsed ignored folders and atomic, optional starter templates. |
| `stale.rs` | Stale linked worktree and local branch candidates, commit-age filtering and protection reasons; fresh identity, tip, working-tree and reachability checks before batch removal through safe branch deletion and non-force worktree removal. |
| `flow.rs` | git-flow without the `git flow` binary: the AVH config keys, start and finish for feature, bugfix, release and hotfix, with a `merge-tree` pre-flight that refuses on conflicts; a single-commit feature or bugfix fast-forwards when possible, matching AVH. |
| `metrics.rs` | Local repository activity across refs and HEAD: 52 UTC weeks, mailmap-aware shortlog authors, last commit and object storage. Local branch comparisons against the cached remote default (then main/master) and upstream, unique commits, age and stale/merged hints. It uses conservative tree-entry containment for squash hints without merge-tree, object writes or repository-defined programs, counts only branches, tags and remotes, and stops its walk a week before the 52-week window; effective configuration is read through Git and CLI reads disable lazy fetching. |
| `merged.rs` | Which local branches are merged into a base (reachable, upstream gone, squash-merged) and which check proved it. |
| `blame.rs` | `Repo::blame`: `git blame --porcelain` of one file as of a commit or the working tree, parsed into lines and the commits they come from (renames followed; uncommitted lines carry the all-zero id). Never runs a textconv; reading the working tree runs the file's clean filter, so that is refused for a file with one outside a trusted folder. In a partial clone it downloads only when the caller allows it (`NotDownloaded` otherwise). |
| `signature.rs` | `Repo::commit_signature`: what `git log --format=%G?` says of a commit's signature (unsigned, verified, untrusted, expired, invalid, cannot be checked). Verifying can start the program `gpg.program`/`gpg.ssh.program` names, which a repository's configuration can set, so the worker runs it (`Job::Signature`, `Event::Signature`) only in a trusted folder. |
| `file_history.rs` | `Repo::file_history`: the commits that changed one path, newest first, from `HEAD` or a given commit, followed through renames with `git --literal-pathspecs log --follow -M --name-status -z` via `cli.rs`. Each entry carries the path the file had in that commit and the old path when it renamed it, so its diff opens with `DiffSource::Commit`. Read by the worker (`Job::FileHistory`, `Event::FileHistory`; only the last queued one runs), with a one-minute limit and `GIT_NO_LAZY_FETCH=1`, so a partial clone is never downloaded into for it. |
| `search_changes.rs` | `Repo::search_changes`: the pickaxe, `git log -S<text>` or `-G<regex> --name-only -z`, newest commit first, one hit per matching file; the text is one argument with its `-S`/`-G` prefix, so it never reads as an option; lazy fetch is off, so a partial clone downloads nothing |
| `tools.rs` | The programs a repository needs besides `git` (`git-lfs` for LFS attributes, `ssh` for an SSH remote, the signing program for `commit.gpgsign`/`tag.gpgsign`) and the installation steps per system; `Repo::missing_tools` reads configuration and two small files, so the worker runs it once per tab (`Job::Tools`, `Event::ToolsMissing`). |
| `lfs.rs` | Git LFS awareness without `git-lfs`: pointer parsing, which paths use `filter=lfs`, and whether an object has been downloaded. |
| `submodule.rs` | Submodule status (also in every snapshot), update, sync, add (with URL and folder checks) and remove, and the commit pair, with the commits between them, that a submodule's diff shows. |
| `forge.rs` | Recognises GitHub and GitLab remotes and builds their web, request and clone URLs. Pure string handling; credentials in a remote URL are dropped, never emitted. |
| `update.rs` | Build numbers for the update check: `parse_build` reads one (digits only; a semantic version has none) and `is_newer` orders builds as integers. |
| `cli.rs` | Runs the `git` executable non-interactively (`GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true`, no console window on Windows), returns git's stderr on failure and streams `--progress` output line by line for long operations. Keeps the environment's repository variables (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` and the rest of `git rev-parse --local-env-vars` but the configuration ones) away from git, so a command works in the repository Canopy gave it, the one it checked and shows. Registers Canopy's askpass hook and redacts URLs and arguments for the log (`redact_url`, `loggable`). |

The app calls `delete_merged_branch` and `force_delete_branch` (the old `delete_branch(name,
force)` wrapper is gone), `stage_all_except_repositories` (its `stage_all` wrapper is gone; it
returns the folders it left out) and `publish_branch` (the branch menu's Publish). It does not
yet use `flow.rs`, `merged.rs`, `lfs.rs`, `paths::safe_join` or `untracked_children` (the Changes list shows
the collapsed folders, not their contents) outside the core's own code and tests; their GitHub
Issues are the UI work.


## canopy-hosting

Blocking (no async runtime) and synchronous like `canopy-core`; callers run it on a background
thread.

| Module | Responsibility |
|---|---|
| `avatars/mod.rs` | Author and committer profile lookup and validated, credential-free HTTPS image downloads; exact image hosts, no redirects and a 2 MiB compressed limit (ADR 0008); a separate repository request (`Client::author_repository`) tells GitHub's two 404s apart, and Gravatar/Libravatar links ask for a 404 (`d=404`) instead of a generated stand-in. |
| `client.rs` | `Client`: provider-neutral entry point for listing, reading, creating, merging, closing, commenting on and approving pull and merge requests, and reading a page's checks and reviews (`rollup`, only with a token). `with_switch` stops every request the client sends, profile lookups included, once the hosting thread is let go. |
| `github.rs`, `gitlab.rs` | The two providers' REST calls (GitHub API v3, GitLab API v4 including self-hosted hosts) and the one GraphQL query each for a list's checks and reviews (`rollup`). |
| `http.rs` | One `ureq` agent per client, the common headers, timeouts, and HTTP failures mapped to `Error` with the server's own message, made plain text (`plain_text`) before client-token redaction. JSON shape errors retain their location without quoting response values. The token travels only in an `Authorization` header. The GraphQL endpoint on the same host (`graphql`: same headers, no redirects). One 10 MiB body limit for every response. |
| `model.rs` | Provider-neutral request, review, check and comment types; `Rollup`/`ReviewDecision` (a list row's checks and review). |
| `preview.rs` | `description_preview`: a request's description flattened to one line of plain text (at most `PREVIEW_CHARS`), from the list response's own body, for a row and its text filter. |
| `token.rs` | Finding a token: provider environment variables (offered only to their own host) or git's credential helpers. |
| `updates.rs` | Canopy's own update manifest, fetched anonymously from the public `NecturaLabs/app-updates` repository. |
| `error.rs` | Errors, carrying the server's message. |
| `test_server.rs`, `avatars/network_failures.rs` | Test-only: a scripted 127.0.0.1 listener (answer, hang up, stall, redirect) and the tests that drive profile lookups and picture downloads against it, asserting each failure names its cause and repeats no token, email, private repository or picture address. |

## canopy-app

| Module | Responsibility |
|---|---|
| `main.rs` | Argument parsing, logging, eframe launch; also the entry point of the askpass client (the same executable, started by git or ssh). |
| `action.rs` | `Action`: what the user asked for during a frame. Views emit actions and never import `app`. |
| `app/mod.rs` | `CanopyApp`: state, start-up, the frame (`eframe::App::ui`), opening repositories, theme. The window's AccessKit name ("<repository> — Canopy"); `Watched`, the copy of the settings whose change reloads diffs or forges, compared each frame instead of cloning `Settings`; and, for the app's tests, `test_app()` and `test_tab()`, a `CanopyApp` and a `RepoTab` with no threads. |
| `app/input.rs` | Keyboard: bare keys, chords and the commands they run. |
| `app/palette.rs` | The command palette's entries, including the Appearance ones (Style, Density, Translucent surfaces, High contrast, Motion). |
| `app/dispatch.rs` | Applies the actions a frame collected. |
| `app/background.rs` | Polled every frame: worker and hosting events, credential prompts, the update check, auto-fetch, the timed refresh of the open request list. |
| `app/open.rs` | The opener thread: opens and creates repositories in request order, off the UI thread. Keeps commit, conflict and request-comment drafts per worktree during the run. |
| `app/clone.rs`, `app/request_flow.rs` | Cloning on a background thread; creating and checking out pull and merge requests. |
| `app/screenshot.rs` | Headless screenshots for `--screenshot`: waits until the window is final, keeps the pointer out of the picture (`raw_input_hook` drops pointer input), opens the dialog overlays and, for `--overlay menu-*`, a context menu by a synthetic right click on the widget it finds by its AccessKit name. |
| `app/demo_requests.rs` | Invented pull and merge requests the requests view shows under `--screenshot --demo-requests`, each row carrying a rollup as with a token; never reachable in a normal run, no network. |
| `tab.rs` | `RepoTab`: one open repository's snapshot, selections, in-flight work and the state a failed or superseded job leaves behind. |
| `worker.rs` | Reads Git's pending merge message off the UI thread, applied only to an empty composer still in a merge. One background thread per open repository; `Job` in, `Event` out over crossbeam channels; queued refreshes are coalesced, and of the queued `Job::Details` and `Job::Diff` only the last runs; debounced file watcher. `Job::PushTarget` answers with `Event::PushTarget` (where a force push would go, for its confirmation). Dropping a `Worker` (closing its tab) stops both threads without waiting; the app's exit waits for them all at once (`wait_for_threads`), so no thread is still inside libgit2 or OpenSSL when the process tears them down. In a partial clone a `Details`, `Diff`, `RefreshStatus` or `Refresh` job may download objects on this thread (network), with an `Event::Progress` line around it, but only in a trusted folder; elsewhere it sends `Event::DownloadsRefused` and fails the read with a message that says why. |
| `identity_worker.rs` | Serial background jobs for identity and supported Git configuration writes, including global recommendations; events refresh identity, Settings facts and affected tabs, with Git errors surfaced as toasts. |
| `avatars.rs` | Twelve-hour persistent thumbnail and no-picture cache that classifies misses (about the author: cached; repository refused: the tab stops asking; unknown commit: another of the author's is tried, at most 1024 remembered; unreachable: a 10-minute pause), the off switch and budget checked before every provider request, `explain` (why pictures are missing, for the views), serialized across the per-host picture threads (which pull request jobs never wait for), an hourly lookup budget that leaves GitHub's anonymous quota to pull requests, files unused for 30 days deleted at start-up, with bounded PNG/JPEG decoding off the UI thread and a per-tab texture cache that drops the least recently shown picture past 256. |
| `hosting.rs` | A background thread per repository tab for pull and merge requests through `canopy-hosting` (`HostJob`), and the token store (`tokens.json`, per host and port). With a token, a list refresh also fetches the rows' checks and reviews (`add_rollup`, one query after the jobs queued behind the list, on a lane of its own so later jobs never wait for it, paused until a rate limit resets, an hour at most) and sends the list twice, first without them, dropping the checks of a list a newer one replaced. Dropping the `HostWorker` turns off its switch, which the thread's client and its author-picture thread's client both carry, so a job under way sends nothing more. |
| `askpass.rs` | Credential prompts for git and ssh: Canopy's own executable is `GIT_ASKPASS`/`SSH_ASKPASS`, and the running app answers over loopback (ADR 0005). |
| `updater.rs` | The update check against the manifest, and the background install (`Updater::install`, `InstallState`); nothing is downloaded until the user chooses Update. |
| `selfupdate/mod.rs` | The in-app update ([ADR 0006](adr/0006-canopy-updates-itself-in-place.md)): detects an installer-managed install (`.canopy-install` receipt), downloads and verifies the archive, unpacks it, swaps the install folder in two phases with rollback and refreshes the receipt and registered desktop entry and icons; elsewhere runs the one-line installer; and starts the new program after the window closed. |
| `build_info.rs` | The build number and short commit hash embedded by `build.rs`, and the release notes the release workflow embeds. |
| `crash_reports.rs` | Opt-in, scrubbed crash reports to Sentry, and what a report may carry (`DATA_VERSION`). |
| `logging.rs` | Structured per-run JSON-lines logs, timed spans, local crash files and the diagnostics Settings copies. |
| `highlight.rs` | Diff syntax highlighting (`syntect`) on two shared highlighter threads, cached per file revision; stale requests are abandoned. |
| `keymap.rs` | Every keyboard shortcut in one table (`BINDINGS`) that dispatches keys and renders the cheatsheet and Settings → Keyboard; the keys the user changed (`Settings::keybindings`) are applied on top of it each frame (`apply_saved`) and Settings → Keyboard listens for a new key with `capture_key`; see `docs/keybindings.md`. |
| `settings.rs` | Settings persisted through eframe storage: appearance, Git, integrations, updates, privacy answers, trusted folders, per-repository choices (`repos`, keyed by working directory), recent and open repositories. A saved theme or option this build no longer has reads as its default (`known`, `known_option`) and a setting it no longer has is ignored; the next save writes the file without either. |
| `settings_file.rs` | Export, import and reset of preferences: the versioned JSON format, the keys that are never exported or replaced (consent, trust, session memory), validation, and the file choosers on a background thread. |
| `request_view.rs` | The request list's text, draft, checks, review and author filters and its sort, applied to the loaded rows (`RequestView::visible`); one value per repository in `Settings::repo_requests`, kept like the sidebar's sections (`Action::RememberRequestView`). |
| `repo_config.rs` | Background Git facts for Settings and notifications: effective and directly editable supported values, remote-specific pruning, pull/autostash precedence, signing diagnostics and hooks-path information; failed reads retain contextual errors rather than masquerading as absent configuration. |
| `system_apps.rs` | The editor, terminal and file manager the system provides (`$VISUAL`/`$EDITOR`/`$TERMINAL`, desktop defaults, `PATH`) and the installed ones (Linux desktop entries, macOS `/Applications`, Windows App Paths), resolved on one background thread and cached; `command_for` is what the open actions run. |
| `trust.rs` | The trusted folders (`Settings::trusted_folders`) as a process-wide list the repository threads read; adding a folder (never a root), resolving the working/common/private Git folders on the repository thread for `Event::TrustFolders`, and the parent offered next to a repository's own folder (ADR 0009). |
| `platform.rs` | Handing things to the operating system (web pages, folders, files in an editor, a terminal) and reading its settings, such as reduced motion, off the UI thread; running commands with quotes and `%f`/`%F`/`%d` placeholders. |
| `ui/mod.rs` | The look-and-feel modules, and the conversions between egui's integer and float types (`points_i8`, `count_f32`, `int_f32`, `to_f32`, `saturate_u8`, `saturate_usize`), each a plain cast named for what it does out of range, so views never cast by hand. |
| `ui/theme.rs` | Design tokens: the three built-in palettes (shared colours plus Standard and Deeper surfaces), the custom theme's slot (`ThemeId::Custom`, `set_custom`), the high-contrast switch, the graph's node shape and curve, elevation materials, metrics per density and style, the active look (`apply`, `palette`, `surfaces`, `metrics`, `appearance`) and the contrast tests. The only place colours and sizes are defined. |
| `ui/motion.rs` | Motion tokens (durations, Bézier curves, springs), the Motion preference, the system's answer (asked on a background thread) and whether motion is currently reduced. |
| `ui/kit/mod.rs` | The component library: `Size`, `Tone`, `ForceState`, `KitResponse`, `focus_visible` (kept current every frame by a begin-pass hook), `show_focus` and the shared interaction state. |
| `ui/kit/anim.rs` | Eased booleans and values, springs, sliding rectangles, the press scale and `Flip` (list rows gliding to their new places); all instant with reduced motion. |
| `ui/kit/a11y.rs` | What screen readers and the keyboard need beyond each widget's own role: named groups and modal dialogs over a `Ui`'s node, open or closed states in a disclosure's name, focus given back when an overlay closes (`FocusReturn`) and Tab kept inside an open overlay (`trap_focus`). |
| `ui/kit/paint.rs` | Shadows, gradient fills, highlights, recesses, focus rings, pixel-snapped seams, Deeper panes with the focused-pane edge (`pane_focus`) and deck, spinners and scroll fades. |
| `ui/kit/button.rs` | `Button` (primary, secondary, ghost, danger), `IconButton`, `ToolButton`, `LinkButton`, `SplitButton`: the reference component. |
| `ui/kit/glass/mod.rs` | Translucent surfaces: decides which floating layers get the backdrop blur (the setting, the theme, the driver, at most four a frame), plans each in validated window pixels, queries the system's reduce-transparency answer off the UI thread, and falls back to the opaque overlay. |
| `ui/kit/glass/gl.rs` | The blur's OpenGL renderer, two `egui_glow` paint callbacks per layer: the capture copies and dual-Kawase blurs the region behind it, before its shadow; the composite keeps the backdrop's luminance in the theme's band (scaled down in dark themes, lifted towards white in light ones) and draws it inside the layer. GL objects are deleted only on the thread that owns the context. The only `unsafe` code in Canopy (ADR 0010). |
| `ui/kit/glass/shaders.rs` | The blur's GLSL and the dialect chosen for the context (OpenGL 3.0+, OpenGL ES 3.0+). |
| `ui/kit/gallery.rs` | The component gallery (`--overlay gallery`) with a section per kit module, plus token, elevation, style and motion strips. |
| `ui/kit/field.rs` | `TextField`, `SearchField`, `TextArea` and `CodeArea`: frameless `TextEdit`s inside the kit's fill, border, focus and error halo. |
| `ui/kit/select.rs` | `Select`, which replaces `ComboBox`: a Secondary trigger and a floating list driven from the keyboard. |
| `ui/kit/toggle.rs` | `Checkbox` (with indeterminate), `Radio`, `RadioGroup` (arrow keys, one Tab stop) and `Switch`. |
| `ui/kit/choice.rs` | `Segmented` with its sliding thumb, `ViewTabs` and `UnderlineTabs` with a sliding indicator, and `Swatches`, a radio group of colour circles. |
| `ui/kit/tabs.rs` | `DocTabs`, the title bar's repository tabs: busy spinner, ahead/behind counts, close on hover and middle click, sideways scrolling with edge fades, and closed tabs collapsing out. |
| `ui/kit/range.rs` | `Slider`, which replaces `egui::Slider`: keyboard steps and a value bubble while dragged. |
| `ui/kit/progress.rs` | `Progress` (determinate, indeterminate, line), `Spinner` and `Skeleton`; all static under reduced motion. |
| `ui/kit/menu.rs` | Menus and context menus: items with icon, shortcut, check, danger, disabled-with-reason and second line, headings, separators and delayed submenus. |
| `ui/kit/overlay.rs` | The floating surface every layer shares (shadow, glass backdrop, fill, ring) and the ground its content sits on, scrolling bodies with edge fades, the room a modal card may take, `Tooltip`, `Popover`, `Dialog` and its scrim, entrances and exit fades. |
| `ui/kit/toast.rs` | `ToastCard`, the toast's painting; the queue, its pausing timers and the cards' slide into place stay in `ui/toast.rs`. |
| `ui/kit/row.rs` | `ListRow`, `TreeRow`, `Disclosure`, `SectionHeader`, the Shift/Ctrl `Selection` and `Roving` (a list as one Tab stop, the arrows moving inside it; a click focuses the row). |
| `ui/kit/chip.rs` | `Badge`, `CountBadge`, `ChangeBadge`, `RefPill`, `RefOverflow`, `Keycap`, `KeyChord`, `StatusDot`, `DiffStat` and the status `Chip`. |
| `ui/kit/avatar.rs` | `Avatar` and `AvatarStack` with the picture crossfade. |
| `ui/kit/surface.rs` | `PaneHeader`, `Deck` (the top bars), `Card`, `Banner` and `Callout` with their `NoticeAction`s, `EmptyState` and `InsetBox`. |
| `ui/kit/graph.rs` | The history row painter: circle nodes, hollow merges, the HEAD halo, a dashed WIP node and lane spacing by density. |
| `ui/kit/split.rs` | `SplitHandle`, the handle between two panes of a custom split: pointer drag, double-click reset, arrow keys with keyboard focus, and a named splitter for screen readers. |
| `ui/kit/testkit.rs` | Test-only helpers for the kit's tests: a themed context, frames with input events and the AccessKit nodes a frame produced. |
| `ui/contrast.rs` | Colour maths on `OKLab` lightness: `derive` makes a theme's high-contrast palette (grounds and hues kept, each foreground moved to the ratio Midnight's contrast reference keeps), `lift` moves every foreground drawn on a ground to at least a reference palette's ratios times a margin, `fit` does that for one colour, and `clear` moves one colour until it reads at a given ratio on a given ground. |
| `ui/custom_theme.rs` | The custom theme: `CustomTheme`, the saved choices (base Midnight or Daylight, accent hue and intensity, tint hue and strength) and the `Palette` they make. Each colour is recoloured at its own luminance; then the grounds keep the base's steps (`keep_steps`), `contrast::lift` moves foregrounds to the base theme's ratios times 1.04, and text colours clear every wash they are drawn over (`clear_text`). Status colours, graph lanes, syntax, danger and diff colours keep the base's hues. |
| `ui/fonts.rs` | Registers the bundled fonts: Inter, JetBrains Mono NL, Phosphor, the heading face and the Canopy CJK fallback. |
| `ui/art.rs` | The illustrations (logo mark, welcome canopy, empty-state spots) drawn with the painter in the active theme's colours; each grows in once when it appears. |
| `ui/widgets.rs` | Ref pills, avatars, badges, segmented controls, switches, checkboxes, and the older button helpers, which now draw `ui::kit` buttons. |
| `ui/folder_picker.rs` | Native folder selection on a background thread; a bounded result channel and requesting-field lifetime keep the event loop responsive. |
| `ui/toast.rs` | Toast queue and animation, and the notification center's history, kept in eframe storage (`canopy-notifications-v1`; `Toasts::to_json`/`restore_json`). |
| `ui/palette.rs` | Command palette with fuzzy matching; entries built on opening, ranked on query change. |
| `ui/cheatsheet.rs` | The keyboard cheatsheet, drawn from `keymap::BINDINGS`. |
| `ui/legal.rs` | The Terms and Privacy Policy as shipped (`docs/legal/`) and the versions a user accepts. |
| `ui/notes.rs` | Release notes ("What's new") and the small Markdown renderer for notes and request text. |
| `views/welcome.rs` | Welcome screen and recent repositories. |
| `views/toolbar.rs` | Tab strip and top toolbar: fetch, pull, push, branch, stash, view switch. |
| `views/sidebar.rs` | Branches, worktrees, remotes, tags, stashes and submodules; expansion changes are persisted per shared Git directory, supplied by the repository worker snapshot. |
| `views/history.rs` | Virtualised commit-graph table. |
| `views/commit_detail.rs` | Selected commit: metadata, parents, changed files, diff. Also the views' shared file rows and screen-reader helpers: `without_icons` (keeps icon glyphs out of a label's spoken name) and `say_unavailable` (a disabled button says ", unavailable" and why, which the Linux AT-SPI adapter does not). |
| `views/changes.rs` | Staging area and commit composer. |
| `views/blame.rs` | The blame of the selected file, drawn in place of its diff (`RepoTab::blame`, read by `Job::Blame`): virtualised rows with commit id, author, date, line number and text; a click on a line's commit sends `Action::Reveal`. |
| `views/diff.rs` | Unified and side-by-side diff viewer with line numbers, hunk and line actions; row layout cached per diff. |
| `views/resolve.rs` | Conflict resolver; off-screen cards skipped by measured height. |
| `views/requests.rs` | Pull and merge requests: list, details, checks, reviews and actions. |
| `views/settings_screen.rs` | The settings screen: appearance, Git, integrations, accounts, updates, privacy, keyboard, backup and reset, and this repository; settings search. |
| `views/dialogs/mod.rs` | `Dialog` and `show`: the modal dialogs (clone, new branch, confirmations, consent, ...). |
| `views/dialogs/stack.rs` | `DialogStack`: credentials in front, the consent never replaced, a closed prompt always answered. |
| `views/dialogs/stale.rs` | Repository-bound stale cleanup preview, disabled protected rows and destructive confirmation. The repository worker reads candidates and revalidates the selected identities, commits and safety before removal. |
| `views/dialogs/file_history.rs` | File history: the commits of one file from the tab's worker-loaded `file_history`, a row opens that commit in History with the file's diff (`Action::ShowFileVersion`). |
| `views/dialogs/search_changes.rs` | Search the history for changed text: the text and regular-expression fields, a Search button that keeps the dialog open (`Outcome::RunKeep`) and the tab's worker-loaded `change_search`, a row opens that commit with the file's diff (`Action::ShowFileVersion`) |
| `views/dialogs/insights.rs` | Repository insights: the weekly activity painter, top authors and branch table, rendering only worker-loaded tab data, with loading/error states and an explicit refresh. |
| `views/dialogs/ignore.rs` | Repository-bound ignore form: rule destinations and previews, ignored paths and deciding-rule explanations. Reads and mutations go through the repository worker; serials discard superseded reads and status changes invalidate cached results. |
| `views/dialogs/reader.rs` | The full-size reading modal for the Terms of Service and Privacy Policy, used by the consent too: a measured column with headings, lists and kit links, scrolling with fades; each document starts at the top when the reader opens. |
| `views/dialogs/{url,start,credential,update,common}.rs` | The remote URL builder, where a new branch starts, credential prompts, the update dialog (what is new, installing, restarting), shared fields and buttons. |
| `views/statusbar.rs` | Branch, ahead/behind, repository state, running job. |
| `views/state_banner.rs` | The merge, rebase, cherry-pick or revert in progress above History, Changes and requests, with Continue and Abort. |

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
- One more thread, `git-identity` (`identity_worker.rs`), is shared by every tab and by the
  first-launch question. It runs the `git` executable, not libgit2, because libgit2 ignores
  `GIT_CONFIG_GLOBAL`, `includeIf`, `GIT_AUTHOR_*` and `EMAIL`, which decide the identity a commit
  gets. It reads configuration only, so this does not move ADR 0001's line for repository data.
- In a partial clone, reads that need objects the clone left on the server (`Details`, `Diff`,
  and a snapshot or status after a soft reset) fetch them on the worker through `git`, so the
  thread can wait for the network while later jobs queue behind it. The worker runs each read
  offline first and downloads only when that fails with `Error::NotDownloaded` (`can_download()`)
  and the repository is inside a trusted folder (`trust.rs`, ADR 0009; elsewhere it sends
  `Event::DownloadsRefused` and the tab's status bar offers Trust…),
  skips a queued `Details` or `Diff` that a later one supersedes, and shows
  "Downloading from <remote>…" in the status bar meanwhile. A commit's file list is sent before
  its line counts, which are fetched in a follow-up unless the user already moved on. A diff that failed with a retryable `NotDownloaded` is followed by `Event::DiffNotDownloaded`; the tab then keeps `failed_diff` (no request until Retry or choosing the file again) and marks the file in `not_downloaded`. A failed
  `Details` is kept in the tab (`details_error`) and shown in the details pane, with a Retry
  button when downloading can fix it.

## Window and rendering

eframe draws with OpenGL (glow). The build uses a patched copy of eframe (`vendor/eframe`,
`vendor/README.md`, ADR 0007): on Wayland a window the compositor is not drawing, such as one on
another workspace, must not be repainted, because the buffer swap would wait for a frame callback
that never comes and the UI thread would stop answering the compositor's pings (Hyprland's
"Application Not Responding"). The patch makes winit hold such repaints back until the window is
drawn again; `tools/stall-check.py` fails if that stops being true. No view code is involved.

## Data flow

1. A user action (click, shortcut, palette command) builds a `Job`, for example
   `Job::Checkout { branch }`, and sends it to the tab's worker. Destructive jobs are only sent
   after the confirmation dialog.
2. The worker calls the matching `canopy-core` function.
3. Mutating jobs are followed by a fresh `snapshot()`, so the UI always receives a consistent
   `RepoSnapshot` rather than patching its own copy.
4. The `Event` (new snapshot, details, diff, progress line or error) is applied to the tab
   state on the next frame; errors become toasts.

## Where common changes live

Paths are under `crates/canopy-app/src/` unless they start with `crates/`.

| To change | Start in | Then |
|---|---|---|
| A repository operation (stage, branch, rebase, ...) | The `add-git-operation` skill: the function and its test in `crates/canopy-core`, then an `Op` (with its `OpKind`) or `Job` in `worker.rs` and the `Event` it returns | An `Action` in `action.rs`, applied in `app/dispatch.rs`; in-flight state in `tab.rs`; a keybinding where one is wanted; `docs/features.md` |
| A setting | A field on `Settings` in `settings.rs` (the struct is `#[serde(default)]`, so old files load) | The control in `views/settings_screen.rs` (sections Appearance, Git, Integrations, Accounts, Updates, Privacy, Keyboard, Backup and reset, This repository; give the row its search words); read it where the behaviour happens, since a setting that changes nothing is a defect. `CanopyApp::save` in `app/mod.rs` persists it under `settings::KEY` |
| A dialog | A variant of `Dialog` and its drawing in `views/dialogs/mod.rs`, with a file beside it when it is large | Open it with `Action::OpenDialog`; stacking rules are in `views/dialogs/stack.rs` |
| A message to the user | `self.toasts.push(ctx, ToastKind::..., title, body)` (`ui/toast.rs`). A worker failure arrives as an `Event` that `Tab::poll` (`tab.rs`) turns into a notice, which `app/background.rs` shows as a toast; its body is git's own stderr | Log the detail with `tracing` |
| A keyboard shortcut | The `add-keybinding` skill: `BINDINGS` in `keymap.rs` | `docs/keybindings.md` |
| A colour, theme or token | The `add-theme` skill: `ui/theme.rs` (`ThemeId::ALL`) | `docs/design.md`; screenshots dark and light |
| A GitHub or GitLab call | The `hosting-api` skill: `crates/canopy-hosting`, then a `HostJob` in `hosting.rs` | `docs/legal/privacy-policy.md` when what leaves the machine changes |
| What first launch asks, or what a report carries | `TERMS_VERSION` and `PRIVACY_VERSION` in `ui/legal.rs`, `DATA_VERSION` in `crash_reports.rs`, the `Settings` fields `accepted_terms`, `accepted_privacy`, `crash_reports` and `seen_intro` | The texts in `docs/legal/`, with their `Last updated` date |
| A test that must not see the user's git configuration | `in_isolated_git_env` in `crates/canopy-core/tests/operations.rs` | `tools/check.sh` runs the suite with an isolated configuration too |
| CI, a release or the update feed | `docs/playbook.md`, the `release` skill and `.github/workflows/` | A change under `.github/` gets a security pass (`AGENTS.md`) |

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

### Repository configuration that runs programs (audit, issue #28)

Every `git` call comes from `cli::command` (clippy refuses `Command::new("git")` elsewhere), which
puts `-c core.fsmonitor=false` first and sets `GIT_TERMINAL_PROMPT`, `GIT_EDITOR` and
`GIT_SEQUENCE_EDITOR`. What a hostile `.git/config` could still make git run, per key:

| Key | Status |
|---|---|
| `core.fsmonitor` | Forced off on every call, submodule children included (test `repository_fsmonitor_command_is_not_executed`). |
| `core.editor`, `sequence.editor` | Overridden by `GIT_EDITOR` / `GIT_SEQUENCE_EDITOR` (test `repository_editors_are_not_executed`; `sequence.editor` outranked `GIT_EDITOR` before). |
| `core.pager`, `pager.*` | Output is always piped, so git never starts a pager (test `repository_pager_command_is_not_executed`). |
| `diff.external`, `diff.*.textconv` | Canopy runs no `git diff` that prints a patch (diffs are libgit2); the one `git diff --quiet` passes `--no-ext-diff`. |
| `alias.lfs` | The LFS probe on opening a repository empties it (`tools::LFS_PROBE`). |
| `core.sshCommand`, `credential.helper`, `remote.*.uploadpack`, `core.gitProxy`, `core.hooksPath` and its fetch hooks, `protocol.ext.allow` | Run only when the user starts a network command, as in a terminal. The one fetch nobody asked for, auto-fetch, refuses outside a trusted folder when they name a program (`partial::fetch_programs`) and neutralises the rest on its command line (`Repo::fetch_unattended`). |
| `gpg.program`, `gpg.ssh.program` | `Repo::commit_signature` is the only verifier; the worker calls it only in a trusted folder (`Job::Signature`, gated by `allowed()`). Signing a commit the user makes runs them as in a terminal. |
| `filter.*.clean/smudge`, merge drivers, hooks (`pre-commit`, `post-checkout`, ...), `submodule.*.update=!cmd` | Run when the user stages, commits, checks out, merges or updates submodules, as in a terminal. Not neutralised: doing so would break Git LFS and the user's own hooks. A repository from an untrusted source can still make these run once the user acts on it. |

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
repository. GitHub issue #40 (secure persistent storage) moves all of this into
Canopy-specific folders.

## Error handling

- `canopy-core` returns `canopy_core::Result<T>`. CLI failures carry the exit status and git's
  stderr unchanged, because git's own message is usually the most useful one.
- The worker converts errors into `Event` errors; the UI shows them as toasts and logs them
  with `tracing`. Nothing in the app panics on a Git error.
- `unwrap()`, `expect()`, `panic!` and printing are not used outside tests (clippy `unwrap_used`,
  `expect_used`, `panic`, `print_stdout` and `print_stderr`; `clippy.toml` exempts tests), and
  `unsafe` is denied everywhere except the backdrop blur's OpenGL calls in `ui/kit/glass/gl.rs`
  (ADR 0010; `crates/canopy-app/tests/unsafe_policy.rs` enforces it).
- The drawing code (`src/views`, `src/ui`) names no filesystem, process, network, libgit2 or git-CLI
  API in its production code: `crates/canopy-app/tests/ui_thread.rs` fails when it does, so repository
  and disk reads stay in worker jobs. `tools/check-ui-colors.py` does the same for colour constructors
  outside `ui/theme.rs`.

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
  otherwise recompute from the snapshot (remote branches per remote, graph width, visible submodule rows and their states) are derived
  once when it arrives (`tab::SnapIndex`). Submodule states also update on status-only events.
- Every list that grows with the repository is virtualised: the history table, the sidebar's
  branches, remote branches, tags, stashes and submodules, changed files (Changes and commit details) and the
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

Commit submission uses `RepoTab::can_commit` for both the composer and keyboard dispatch,
including staged changes, summary, busy state, conflicts and amend eligibility. Non-clean
repository snapshots clear Amend; late HEAD-message responses fill only an empty, still-active
amend draft in a clean repository.
Squash messages use a separate event, fill only an empty normal-commit draft, and never enable
Amend; successful squash operations clear Amend before opening Changes.
