# Backlog

The task board, kept current during work sessions: update it in the same change that starts,
merges or finishes an entry. Take entries from the top of **Now** down. Each entry ends with its
acceptance criterion. Tick an entry (`- [x]`) only when the change is verified in the running app,
with a screenshot from the `screenshot` skill for anything visible, and move it to the Done section
of the release it ships in. Add new entries where their priority puts them, not at the bottom.

Workflow: every entry is worked on its own short branch from `main` and merged straight into
`main` (no pull requests) once `tools/check.sh` passes, the `canopy-review` agent has reviewed it
and CI is green on the branch. `docs/specs/parallel-sessions.md` holds the setup and briefs of the
first two parallel sessions, as a template for the next.

## In flight

Who works on what, on which branch. Remove a row when its branch is merged. Nothing is in flight
at the moment. Merged into `main` so far: the original working branch, `perf/views` (frame cost
119–209 ms → 0.3–0.9 ms on huge data), `fix/installer-review` (every security-review finding),
`canopy-3/core-workflows` (git-flow, merged detection, LFS status, submodules in core),
`canopy-2/core-safety` (safe branch delete, publish, mode and type changes, collapsed untracked
folders, nested repositories, safe paths and ignore rules in core; the app still calls
compatibility wrappers, see `docs/specs/HANDOFF-canopy-2-core-safety.md`) and
`feat/partial-clones` (partial-clone reads, downloads and Retry), plus the branch-delete fix.

| Entry | Branch | Who | State |
| --- | --- | --- | --- |

## Now

In order. The UI entries come before accessibility, and accessibility before the docs sweep.

- [ ] **Reviews and passes still owed on merged work** (security-sensitive areas first): a security
  pass on the partial-clone code (`cli.rs`, `partial.rs` and the merged `status.rs` and `repo.rs`
  hunks); an independent security review of the installer fixes (`85d2961`) and a Windows
  PowerShell 5.1 run of `install.ps1`; a review of canopy-3's core (`flow.rs`, `merged.rs`,
  `lfs.rs`, `submodule.rs`) when the app starts calling it (canopy-2's live changes, the literal
  pathspecs and `paths.rs`, were reviewed with no findings). Accept: each review's findings are
  fixed or answered, and the entry is deleted.
- [ ] **Pull reads `pull.rebase` through libgit2**: `Repo::pull` (`ops.rs`) decides whether to pass
  `--rebase`/`--no-rebase`/`--ff-only` from `config_string("pull.rebase")`, which is libgit2's view
  of the configuration. libgit2 ignores `GIT_CONFIG_GLOBAL` and resolves `includeIf` differently
  from git, so when they disagree git gets no flag and fails with "Need to specify how to
  reconcile divergent branches". Read the value with `git config --get pull.rebase` through
  `cli`, and give the pull tests in `crates/canopy-core/tests/operations.rs` an isolated
  `HOME`/`XDG_CONFIG_HOME` (they read the developer's global config today). Accept: with
  `pull.rebase=true` in `~/.gitconfig`, `cargo test -p canopy-core` passes, and a regression test
  covers a value only git sees.
- [ ] **`packaging/test-install.sh` aborts on a dangling link in `/usr/bin`**: `path_without`
  (line 173) links every entry of `/usr/local/bin`, `/usr/bin` and `/bin` into a folder and skips
  names already there with `[ -e ]`, which is false for a link whose target is missing. Where
  `/bin` is `/usr/bin` every entry is visited twice, so the second `ln -s` fails with "File
  exists" and `set -e` ends the test at "no curl and no wget" (seen on Arch with a dangling
  `/usr/bin/QtWebEngineProcess6`), and `tools/check.sh` fails with it. Test with `[ -e ] || [ -L ]`.
  Accept: `sh packaging/test-install.sh` passes on a host with a dangling link in `/usr/bin`.
- [ ] **Architecture split, rest** (done in `main`: `app/` and `views/dialogs/` modules,
  `action.rs`, typed `OpKind`/`HostAction`, repositories opened and watched off the UI thread):
  shared widgets (inset frame, file header, empty state), native file dialogs off the UI thread,
  tests for tab state and the palette model, a clippy lint against colours outside `theme.rs`,
  the merge dialog's per-frame branch list. Accept: check.sh passes and screenshots are
  unchanged.
- [ ] **Persistent, secured storage**: Canopy-specific per-OS folders for config, data, state and
  cache (Linux `~/.config/canopy` etc., macOS `io.necturalabs.Canopy`, Windows
  `NecturaLabs\Canopy`), each with a `.canopy-data` marker the installer's `--purge` relies on;
  migration from today's folder; settings written atomically on change with a schema version and
  corruption recovery; 0700/0600 permissions; tokens in the OS keychain with a file fallback;
  redirects never forward the Authorization header; session restore. Accept: a corrupted
  settings.json is set aside and defaults load with a toast; tokens survive a restart from the
  keychain; the old folder migrates once.
- [ ] **Partial clones in the views**: a "not downloaded yet" state for blobs missing from a
  partial clone, fetched on demand off the UI thread. Accept: a `--filter=blob:none` clone browses
  history without blocking and shows file contents after fetching. The core reads are done
  (`feat/partial-clones`: `MissingObjects`, `partial.rs`, offline file lists, `status_unavailable`),
  and so are the worker (offline first, download with a progress line, superseded reads dropped)
  and the details pane's error with Retry. Left: verify against a real `--filter=blob:none` and
  `--filter=tree:0` clone in the running app, screenshots of the error pane and the "Changes not
  available" state, a Retry for a failed diff (only commit details have one), and a per-file
  "not downloaded" marker before the diff is opened.
- [ ] **Git LFS** (core status, pointer and not-downloaded detection merged from canopy-3; see
  `docs/specs/HANDOFF-canopy-3-core-workflows.md`): correct status for LFS files, an LFS indicator
  and pointer-only state, LFS diffs, `git lfs pull`/fetch, track and untrack, locks, and a prompt
  when `git-lfs` is missing. The status fix is not finished: the SHA-256 step that drops
  stat-dirty but unchanged LFS files needs a hashing crate (`sha2`, MIT/Apache-2.0) in
  `Cargo.toml`, which is the owner's call to add. Accept: an LFS fixture shows no phantom changes,
  marks pointer-only files and pulls them.
- [ ] **Feature-branch workflows** (git-flow and merged detection in core, merged from canopy-3):
  git-flow start and finish (feature, bugfix, release, hotfix, AVH config) without the `git flow`
  binary, branch folders by prefix in the sidebar, publish with upstream, merged and
  squash-merged indicators, delete merged branches (`-d`, force only after confirmation), and
  "Update from base" (`docs/specs/lfs-flow-spec.md`, part B). Known limits of the core: finishing
  always merges with `--no-ff` and needs Git 2.38 or newer; "rebase before finish" and
  hotfix-into-an-open-release are not offered. Accept: refs, tags and merges match the spec's
  command table.
- [ ] **.gitignore** (core rules, why-ignored, stop tracking and preview merged from canopy-2): ignore a file, extension, folder or pattern from the changes list, local-only
  exclude, stop tracking and ignore, show ignored files, "why ignored", templates on init.
  Accept: each action's result matches `git check-ignore -v`.
- [ ] **Submodules** (core status and operations merged from canopy-3): sidebar list with state, open in a tab, init, update, sync, add, remove,
  pointer diffs as commit ranges, recurse on pull, checkout and clone. Accept: an out-of-date
  submodule fixture shows its state and updates.
- [ ] **File and directory links**: show and diff symlinks, file/symlink type changes and mode
  changes; never follow links when reading, watching or opening (targets outside the repository,
  loops); Windows `core.symlinks=false`. Accept: a fixture with each case shows the right rows and
  nothing outside the repository is read.
- [ ] **Research bugs** (fixed in `main`: the watcher no longer follows symlinks and watches a
  worktree's or submodule's git folder): nested repositories in Stage all and 100,000 untracked
  rows (core from canopy-2, then the UI). Accept: each has a test.
- [ ] **Missing tools**: detect git (minimum version), git-lfs, ssh and signing tools when a
  repository needs them, with the macOS Finder PATH and Windows Git locations; prompt with
  OS-specific install steps. Accept: removing git-lfs from PATH on an LFS repository shows the
  prompt with working steps.
- [ ] **Colour adoption in views**: `Palette::selected()`, `warning_tint()`, `ink()` and
  `control_border` everywhere; remove ad-hoc tints that fail contrast. Accept: the contrast test
  covers every tint the views draw, and screenshots of all seven themes read cleanly.
- [ ] **Typography**: type-scale tokens in `theme.rs` (`docs/specs/ui-spec.md` section 2) and every
  ad-hoc size migrated. Accept: no `.size(` or `FontId::new` literal outside `theme.rs`.
- [ ] **UI controls**: switches, checkboxes, segmented controls, sliders, inputs and buttons to
  modern conventions (`docs/specs/ui-spec.md` section 1). Accept: every control has hover, focus,
  pressed and disabled states in all themes.
- [ ] **Motion**: a reduce-motion setting (system, reduced, full) with the system query on a
  background thread; token durations; no row-hover or selection animation; transitions for
  dialogs, toasts and popovers. Accept: with reduced motion, nothing animates.
- [ ] **Settings fully in the UI**: every persisted preference has a control; consent review and
  withdrawal; recent-list and session controls; search, reset, import and export; a Git config
  section (identity, signing); remappable keybindings with conflict detection. Accept: no setting
  needs a file edit.
- [ ] **Responsive layout**: no overlap or clipping from 960x600 to 4K at scale 1 to 2 (legal
  dialog width, diff header, resolver header and footer, toolbar, history pill clip, status bar,
  ellipsis truncation). Accept: the size matrix screenshots are clean.
- [ ] **Visual polish**: resolver cards, choice buttons, chips, gutter, sidebar, PR view, keycaps,
  palette, merged pills, graph seams, welcome banding, horizontal scroll in split diffs; a final
  whole-app design review. Accept: the design review has no open findings.
- [ ] **Stress readiness in code** (the test itself runs only when the owner says so): lazy
  background tabs, bounded threads, staggered auto-fetch, one shared watcher, the watcher-failure
  warning, frame-time and first-frame log events. Accept: 100 tabs open without a thread per tab.
- [ ] **Accessibility**: AccessKit names and roles for every control, focus order and visible
  focus, keyboard reachability, contrast, reduced motion. Accept: Orca navigates the history list,
  the changes list and the commit composer.
- [ ] **Docs sweep**: README, architecture, features, keybindings, CHANGELOG and tools/README were
  brought up to date once `canopy-2` and `canopy-3` merged; what is left is `docs/design.md`
  (a table for each of the five other themes and the type-scale tokens once they exist),
  CONTRIBUTING (it still describes pull requests and an older `check.sh`), the screenshots in
  `docs/screenshots/` (rendered at v0.1.0, before the themes and the settings screen), and
  retiring HANDOFF.md when its notes are all folded into the docs. Accept: every doc matches the
  code.
- [ ] **Verify the one-line installers on real machines** after the first stable release that
  publishes them (CI tests `install.sh` end to end against a fake feed on Linux and macOS and
  only smoke-tests `install.ps1`). Accept: on Ubuntu 22.04, Fedora, macOS (Apple silicon) and Windows 11 (Windows
  PowerShell 5.1 and PowerShell 7) the one-liner installs, Canopy starts from the launcher, Start
  Menu or Spotlight with its icon, a re-run upgrades, and uninstall leaves only the user data.

## Next

- [ ] **Interactive rebase editor**: list commits from a chosen base; drag to reorder, and mark
  pick, reword, squash, fixup or drop. Accept: a reorder plus squash plus reword produces the same
  history as the equivalent `git rebase -i` todo list.
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
- [ ] **Worktrees**: list worktrees, open in a tab, add and remove (with confirmation). Accept:
  actions match `git worktree list` before and after.
- [ ] **Commit message templates and linting**: honour `commit.template`; optional Conventional
  Commits check with inline hints. Accept: the template pre-fills the composer and an invalid type
  is flagged before commit.
- [ ] **Performance budget tests**: benchmarks for snapshot and graph layout on the `big` fixture
  and a generated one-million-commit history. Accept: CI fails if layout of 100,000 commits
  exceeds its recorded budget by more than 20%.
- [ ] **Incremental graph for large repositories**: stream commits in pages and extend the layout
  without recomputing it. Accept: first paint of a one-million-commit repository in under one
  second on the reference machine.

## Later

- [ ] **Linux arm64 and Intel macOS builds out of experimental**: the release builds them
  (cross-compiled) but lets them fail without holding back a release. Accept: one release ships
  both, each starts on real hardware, and `experimental` is removed from their matrix entries.
- [ ] **Signed and notarised macOS app, signed Windows binary**: Developer ID signing and
  notarisation for `Canopy.app`, Authenticode for `canopy.exe`. Accept: a browser-downloaded
  archive opens without a Gatekeeper or SmartScreen warning.
- [ ] **Flatpak**: Flathub-ready manifest with host `git` access. Accept: the Flatpak opens,
  fetches and pushes a repository in the home directory.
- [ ] **AUR packages**: `canopy-bin` and a source package. Accept: both build in a clean chroot
  with `makepkg`.
- [ ] **.deb and .rpm**: `cargo-deb` and `cargo-generate-rpm` in the release workflow. Accept:
  packages install on Ubuntu LTS and Fedora and show the launcher icon.
- [ ] **AppImage**: built in the release workflow. Accept: runs on a fresh Ubuntu LTS and Fedora
  live session.
- [ ] **Custom themes**: import theme files mapping to the tokens in `ui/theme.rs`, and optionally
  follow Omarchy's active theme. Accept: switching the Omarchy theme recolours Canopy within a
  second.
- [ ] **Patches**: save commits as patches (`format-patch`) and apply patch files. Accept: a
  round trip reproduces identical commits.
- [ ] **Bisect**: start, mark good or bad from the graph, reset. Accept: bisecting the `linear`
  fixture finds the planted commit.
- [ ] **Graph minimap**: a scrollbar-sized overview of lanes and refs that scrolls the history.
  Accept: clicking the minimap jumps to the matching rows.
- [ ] **gitoxide for history walks**: evaluate `gix` for revwalks and status on very large
  repositories. Accept: a written comparison with numbers, and adoption only if it is faster.
- [ ] **Localisation**: extract strings and load translations. Accept: a German translation covers
  every visible string.

## Implemented, awaiting in-app verification

In the code and tested, not yet ticked by a check in the running app.

- [ ] Credential prompts for git and SSH passphrases through Canopy's askpass.
- [ ] Ahead and behind counts in each repository tab header, matching the status bar.
- [ ] Branch delete runs `git branch -d` and offers a forced delete only when Git refuses.
- [ ] Local-only repositories: the badge and Publish; GitHub and GitLab "Open on" links and the
  clone and add-remote address builders.
- [ ] The settings screen, the keymap table with its cheatsheet, and the update check with "What's
  new".
- [ ] Merge modes (default, no fast-forward, fast-forward only, squash).
- [ ] Line-level stage, unstage and discard.
- [ ] Structured per-run JSON logs; opt-in, scrubbed crash reports with consent.
- [ ] Diff settings (context lines, ignore whitespace), syntax highlighting, side-by-side diffs.
- [ ] Three-way conflict resolver.
- [ ] GitHub and GitLab pull and merge requests through the REST API with a user token.
- [ ] Seven themes meeting WCAG contrast on every surface; Midnight the default.
- [ ] One-line installers (Linux tested end to end in CI).

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
