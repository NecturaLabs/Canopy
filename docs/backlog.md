# Backlog

The task board. Take entries from the top of **Now** down. Each entry ends with its acceptance
criterion. Tick an entry (`- [x]`) only when the change is verified in the running app, with a
screenshot from the `screenshot` skill for anything visible, and move it to the Done section of
the release it ships in. Add new entries where their priority puts them, not at the bottom.

## Now

- [ ] **Credential prompt (`canopy askpass`)**: add an `askpass` subcommand and set `GIT_ASKPASS`
  and `SSH_ASKPASS` for CLI jobs so git asks Canopy for usernames, passwords and tokens through a
  dialog. Accept: fetch from an HTTPS remote with no credential helper prompts in-app and succeeds.
- [ ] **SSH passphrase prompt**: route `SSH_ASKPASS` (with `SSH_ASKPASS_REQUIRE=force`) through the
  same dialog. Accept: push over SSH with a passphrase-protected key and no agent prompts in-app.
- [ ] **Line-level staging**: select one or more lines in a hunk and stage, unstage or discard just
  those. Accept: staging two of five added lines leaves the other three unstaged, matching `git add -p` edit mode.
- [ ] **Log file and crash report**: write logs to the platform data directory with rotation and
  record panics with a backtrace. Accept: a forced panic leaves a report and the next launch offers
  to open it.
- [ ] **Diff settings**: context lines (0 to 25) and ignore whitespace (none, at end of line, all)
  per repository. Accept: changing either re-renders the open diff without reselecting the file.
- [ ] **Syntax highlighting in diffs**: highlight with `syntect` on the worker, cached per file
  revision. Accept: Rust, TypeScript, Python and Markdown diffs are highlighted in both themes,
  and a 10,000-line diff scrolls without frame drops.
- [ ] **Side-by-side diff**: toggle between unified and split views, remembered per user. Accept:
  hunks align line by line, with filler rows for unbalanced changes.
- [ ] **Push and pull counts in tab headers**: show ahead/behind next to each tab name. Accept:
  counts update after fetch and match the status bar.

## Next

- [ ] **Interactive rebase editor**: list commits from a chosen base; drag to reorder, and mark
  pick, reword, squash, fixup or drop. Accept: a reorder plus squash plus reword produces the same
  history as the equivalent `git rebase -i` todo list.
- [ ] **3-way conflict resolver**: base, ours and theirs with a result pane; take a side per hunk or
  edit manually, then mark resolved. Accept: resolving the `conflict` fixture in-app completes the
  merge with the expected file content.
- [ ] **File history**: commits that touched a path (following renames) with a diff per commit.
  Accept: history for a renamed file in the showcase fixture includes commits before the rename.
- [ ] **Blame view**: per-line author, date and commit, with click-through to the commit. Accept:
  output matches `git blame --porcelain` for every line of a test file.
- [ ] **Reflog and undo**: show the HEAD reflog and offer "Undo" for the last commit, checkout,
  reset, merge or rebase. Accept: undoing a hard reset restores the previous HEAD and the toast
  names what was undone.
- [ ] **Search across history (pickaxe)**: search added or removed content (`-S`/`-G`) on the
  worker, cancellable. Accept: results stream into the history view and match `git log -S`.
- [ ] **Drag and drop branches**: drop a branch onto another in the sidebar or graph to merge or
  rebase, with a confirmation menu. Accept: both actions run the same jobs as the menu commands.
- [ ] **Signing visibility**: show signed, verified and unverified badges for GPG and SSH signatures
  and honour `commit.gpgsign` when committing. Accept: badges match `git log --show-signature` on
  a fixture with each case.
- [ ] **Image diff**: side-by-side and swipe views for PNG, JPEG, GIF, WebP and SVG. Accept:
  changed images in a commit render both versions with their dimensions.
- [ ] **Submodules**: list submodules in the sidebar with status; open one in a new tab; update
  and init. Accept: a fixture with an out-of-date submodule shows it and updates it.
- [ ] **Worktrees**: list worktrees, open in a tab, add and remove (with confirmation). Accept:
  actions match `git worktree list` before and after.
- [ ] **Commit message templates and linting**: honour `commit.template`; optional Conventional
  Commits check with inline hints. Accept: the template pre-fills the composer and an invalid type
  is flagged before commit.
- [ ] **Configurable keybindings**: edit shortcuts in settings with conflict detection. Accept:
  a rebound command works from the keyboard and the palette shows the new shortcut.
- [ ] **Performance budget tests**: benchmarks for snapshot and graph layout on the `big` fixture
  and a generated one-million-commit history. Accept: CI fails if layout of 100,000 commits
  exceeds its recorded budget by more than 20%.
- [ ] **Incremental graph for large repositories**: stream commits in pages and extend the layout
  without recomputing it. Accept: first paint of a one-million-commit repository in under one
  second on the reference machine.

## Later

- [ ] **Flatpak**: Flathub-ready manifest with host `git` access. Accept: the Flatpak opens,
  fetches and pushes a repository in the home directory.
- [ ] **AUR packages**: `canopy-bin` and a source package. Accept: both build in a clean chroot
  with `makepkg`.
- [ ] **.deb and .rpm**: `cargo-deb` and `cargo-generate-rpm` in the release workflow. Accept:
  packages install on Ubuntu LTS and Fedora and show the launcher icon.
- [ ] **AppImage**: built in the release workflow. Accept: runs on a fresh Ubuntu LTS and Fedora
  live session.
- [ ] **Accessibility audit**: AccessKit names and roles for every control, focus order and
  contrast checks. Accept: Orca can navigate the history list and commit composer.
- [ ] **Custom themes**: import theme files mapping to the tokens in `ui/theme.rs`, and optionally
  follow Omarchy's active theme. Accept: switching the Omarchy theme recolours Canopy within a
  second.
- [ ] **GitHub pull requests**: list PRs for the current repository, check out a PR branch,
  create a PR from the current branch. Accept: works with a fine-grained token stored in the
  system keyring.
- [ ] **Avatar images**: optional Gravatar (off by default, with a privacy note), cached on disk.
  Accept: no network request is made while the setting is off.
- [ ] **Patches**: save commits as patches (`format-patch`) and apply patch files. Accept: a
  round trip reproduces identical commits.
- [ ] **Bisect**: start, mark good or bad from the graph, reset. Accept: bisecting the `linear`
  fixture finds the planted commit.
- [ ] **git-flow**: start and finish feature, release and hotfix branches. Accept: branch names and
  merges match `git flow` with default prefixes.
- [ ] **LFS indicator**: mark LFS-tracked files and pointer-only checkouts; run `git lfs pull`.
  Accept: a fixture with LFS pointers shows the indicator.
- [ ] **Graph minimap**: a scrollbar-sized overview of lanes and refs that scrolls the history.
  Accept: clicking the minimap jumps to the matching rows.
- [ ] **Partial clone support**: clone with `--filter=blob:none` from the clone dialog and fetch
  missing blobs lazily. Accept: browsing history of a partial clone does not block the UI.
- [ ] **gitoxide for history walks**: evaluate `gix` for revwalks and status on very large
  repositories. Accept: a written comparison with numbers, and adoption only if it is faster.
- [ ] **Localisation**: extract strings and load translations. Accept: a German translation covers
  every visible string.
- [ ] **Update check**: optional check against GitHub releases, off by default. Accept: shows a
  toast with release notes when a newer version exists and makes no request when disabled.

## Done (v0.1)

- [x] Welcome screen with recent repositories; open, clone with live progress, init.
- [x] Repository tabs with Ctrl+Tab navigation and Ctrl+W to close.
- [x] Commit graph with coloured lanes, curved merge edges, ref pills, initials avatars, relative
  dates, and filtering by message, author or SHA.
- [x] Commit details: metadata, parents, changed files with +/- counts and per-file diff.
- [x] Working copy: unstaged, staged and conflicted lists; file and hunk stage, unstage, discard;
  diff viewer with line numbers.
- [x] Commit composer with summary counter, description and amend.
- [x] Branches: create, checkout, rename, delete, merge, rebase, ahead/behind, tracking checkout.
- [x] Remotes: fetch (all, prune), pull, push (upstream, force-with-lease).
- [x] Tags (lightweight, annotated, push, delete) and stashes (save, apply, pop, drop).
- [x] Commit menu: checkout, new branch, new tag, cherry-pick, revert, reset, copy SHA and message.
- [x] Merge, rebase and cherry-pick in-progress banner with continue and abort.
- [x] Auto refresh on file changes and window focus.
- [x] Command palette, keyboard shortcuts, toasts, status bar.
- [x] Midnight (dark) and Daylight (light) themes.
