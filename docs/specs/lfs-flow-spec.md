# Git LFS, feature branches, .gitignore, submodules and links: research spec

Research by the canopy-ux-research agent, 2026-10-01. Recommendations for Canopy; claims marked (snippet) or (unverified) were not read from the primary source.

## A. Git LFS

### A1. Status correctness: recommendation

Keep libgit2 as the fast path and add an in-process LFS check for entries it reports as modified. Do not switch to `git status` for LFS repositories.

**The symptom is verified.** libgit2 reports LFS files as modified while git reports clean (libgit2 #3373). All LFS files appear as changes in GitButler (#2595, planned for its V3, "gitoxide might help"). LFS files show as unstaged in Sublime Merge (#425, open since 2019). GitKraken moved off libgit2 to the git executable, and since 9.13 calls git-lfs directly (snippets).

**The cause is inferred.** libgit2 has a filter API but does not run external drivers such as `filter.lfs.*`. When a smudged file's stat differs from the index (touch, restore, racy timestamp), libgit2 re-hashes it with built-in filters only. The big file's hash then differs from the pointer blob. Pointer-only checkouts are fine, because the file on disk is the pointer. Staged changes are fine, because HEAD and the index are compared by object ID.

**What to build:**
1. Gate on `WT_MODIFIED` entries only, so repositories without LFS pay nothing.
2. For each such entry, call `repo.get_attr(path, "filter")`. git2 0.21 has `get_attr`. Continue only if the value is `lfs`.
3. Read the index blob and parse it as a pointer. The format is verified: `version https://git-lfs.github.com/spec/v1`, `oid sha256:<hex>`, `size N`, under 1024 bytes.
4. If the working file's size differs from the pointer's `size`, report it as modified without hashing.
5. Otherwise stream SHA-256 over the file (`sha2` crate, check cargo-deny). If it equals the pointer's `oid`, drop the entry as clean.
6. Cache the result in the worker, keyed by `(path, size, mtime, index blob oid)`. Otherwise every watcher refresh re-hashes multi-gigabyte files.
7. If the pointer has `ext-` lines or `lfs.extension.*` is set, fall back to CLI `git status --porcelain=v2 -z` for that repository.

**Why not the alternatives:**
- CLI status is accurate. But with `filter.lfs.required=true` and no git-lfs installed, git fails outright on stat-dirty files. It also spawns a process on every debounced refresh and adds a second parser.
- `git lfs clean` per file means one process per file.

The in-process check works without git-lfs, so it also works behind the "git-lfs missing" banner.

**Other rules:**
- Skip the libgit2 working-tree patch for LFS paths. It would read a multi-gigabyte file. Synthesise the diff from the pointer.
- Disable hunk and line staging for LFS paths. File staging stays on the CLI, which runs the clean filter.
- Test without git-lfs: write a pointer blob and a matching file, `touch` it, and assert that libgit2 says modified and `collect` says clean. I could not run this here.

### A2. Feature expectations and pitfalls

| Client | Notes |
|---|---|
| Fork | Bundles git-lfs (3.6.0 in Fork 2.50). Has lock and unlock in the file context menu, an LFS status dialog, and image preview of LFS files. |
| GitKraken | Needs git-lfs 3.0.0 or later. A feedback-board request says it has no locking (may be stale). |
| Tower | Bundled. "Enable Git LFS" button in Settings. LFS submenu on working-copy files to track a pattern. |
| Sourcetree | Repository > Git LFS > Initialize Repository. Locking is an open request (SRCTREE-4663). |
| GitHub Desktop | Bundled. Prompts "Initialize Git LFS" on open, even when already set up (#6240). |
| Sublime Merge | Added lock and unlock in builds 2124 and 2125 (snippet). Push failed when git-lfs was not on the app's PATH (#1138). |
| lazygit | No LFS keybindings. |

**Pitfalls for Canopy's CLI jobs:**
- **Credentials.** git-lfs honours `GIT_ASKPASS` and `core.askpass` (verified in its config doc), so Canopy's askpass dialog covers HTTPS LFS. Over SSH, `git-lfs-authenticate` goes through ssh, which `SSH_ASKPASS_REQUIRE=force` already covers. Without a helper, expect "LFS: Git credentials for … not found" (git-lfs #1485); show git's stderr in the toast.
- **Progress.** git-lfs suppresses progress when stdout is not a terminal. Set `GIT_LFS_FORCE_PROGRESS=1` on LFS jobs.
- **Incomplete push.** `lfs.allowincompletepush` defaults to false. Pushing after a skip-smudge clone fails with missing objects. Hint at `git lfs fetch`.
- **Locks.** `lfs.locksverify` makes a push fail on files locked by others. Surface stderr.
- **Locks need the network.** Run them only as user-triggered jobs, never in the refresh loop.
- **Clone.** Keep plain `git clone`, which smudges through the filter process. Add a "Download LFS files" checkbox (default on). When off, set `GIT_LFS_SKIP_SMUDGE=1` on the clone, then offer "Download" (`git lfs pull`).

### A3. Scope

**Must-have for v1:**
1. The status fix above.
2. An "LFS" badge on files, found through the `filter=lfs` attribute.
3. A "not downloaded" state: the pointer is valid and `<common-git-dir>/lfs/objects/aa/bb/<oid>` is missing. Honour `lfs.storage`. A status-bar chip offers "Download".
4. An LFS diff card showing size and OID, old to new. Show the real content only if the object exists locally, with a "Show pointer text" toggle.
5. Palette commands "LFS: Pull" and "LFS: Fetch all".
6. A banner when the repository uses LFS but `git lfs version` fails. Do not bundle git-lfs; recommend it as a package dependency.
7. A "Set up Git LFS" action running `git lfs install --local` when the filter or hook is missing.

**Next:** track and untrack from the file context menu (`git lfs track "<pattern>"`; the `.gitattributes` change shows as a normal edit), `git lfs prune`, and a locks list. Locking works on GitHub and GitLab (GitLab file locks are on all tiers). Lock, unlock, and force-unlock all need confirmation.

**Later:** image preview and `lfs migrate` (it rewrites history).

## B. Feature branches

### B1. git-flow

**Recommendation:** build it in, with no dependency on a `git flow` binary, and read and write the AVH config keys. git-flow-next (Tower, BSD-2) imports those keys, and AVH was archived in June 2023.

**Keys and defaults (AVH, verified):**
- Branch keys: `gitflow.branch.master` (`master`) and `gitflow.branch.develop` (`develop`).
- Prefix keys: `gitflow.prefix.feature` (`feature/`), `bugfix` (`bugfix/`), `release` (`release/`), `hotfix` (`hotfix/`), `support` (`support/`), and `versiontag` (empty).
- The init dialog should default the production branch to whichever of `main` or `master` exists. It should create `develop` from it if missing. It needs at least one commit.

**What each step runs:**

| Step | Commands |
|---|---|
| feature start | `git switch -c feature/x <base, default develop>`. AVH checks that the branch is new and the base exists, and optionally fetches first. |
| feature finish | Clean tree. `git checkout develop`, `git merge --no-ff feature/x` (`--squash` or rebase optional), `git branch -d` (`-D` if squashed), optional `git push origin :feature/x`. |
| release start | `git switch -c release/V develop`. AVH requires one release at a time and a clean tree. |
| release finish | `git merge --no-ff release/V` into master, `git tag -a [-m msg] <prefix>V`, then back-merge the tag into develop with `--no-ff`. Delete the branch. Optional push of master, develop and tags. |
| hotfix | Start from master. Finish is the same as release finish. The nvie model merges into an existing release branch instead of develop. AVH's behaviour there is unverified. |

**Client behaviour:**
- **Fork:** Git Flow submenu on a branch, plus an init dialog for production, develop and prefixes. Start and finish are in the branch and commit menus.
- **GitKraken:** Preferences > Gitflow, then right-click Finish. Options are "Delete branch" (checked) and "Rebase on develop" (unchecked). A tag prefix is set in Preferences.
- **Sourcetree:** keep-or-delete, and "Do not fast-forward". That option was reported ignored (SRCTREEWIN-1144).
- **Tower:** toolbar menu, sidebar Finish, back-merge and tag. It also maintains git-flow-next.
- **lazygit:** `i` on a branch opens the git-flow menu.
- **GitHub Desktop, Sublime Merge, GitButler:** no git-flow found (unverified).

**Canopy rules:**
- Run a pre-flight conflict check with `git merge-tree --write-tree` (introduced in Git 2.38, from memory). On conflict, refuse and offer to merge `develop` into the feature first. This avoids AVH's half-finished state.
- Local branch deletion defaults to on. Remote deletion defaults to off, with confirmation.
- Push only the named branches and the one tag, never `--tags`.
- Treat `bugfix` as a topic type that uses the same code as `feature`. Skip `support`.

### B2. Plain feature-branch workflow: gaps in Canopy

I read `sidebar.rs`, `dialogs.rs` and `ops.rs`.

**Exists:**
- Branch create (a `start` argument and a check-out checkbox, no base picker), checkout, rename, delete, merge dialog, rebase with confirmation.
- Ahead/behind badges.
- Push that sets the upstream, but only for the current branch.
- Force-with-lease, remote branch delete, and "Create pull request" in the branch menu.

**Missing:**
- Branch folders. The sidebar list is flat. Fork has had folders since 2016, and GitKraken builds folders from `/`.
- A "merged" indicator, and "delete merged" (Tower has a badge and explains which check matched).
- Delete always passes `-D` (`force: true`).
- Publishing a non-current branch, an "upstream gone" marker, and an "unpublished" marker.
- A base picker in the new-branch dialog.
- "Update from base".

**Recommendation:**
1. **Folders.** Group by prefix, collapsible, with state kept in `tab.collapsed` (for example `branches:feature`).
2. **Merged detection, local only.** First, tip reachable from the base (`graph_descendant_of`). Second, upstream configured but its remote ref gone after prune. Third, squash or rebase merges: `merge-tree --write-tree` result tree equals the base tree (Git 2.38 or later; skip on older git). `git cherry` also works for rebase merges. Cap the work on the worker. Name the matching check in the tooltip.
3. **"Clean up branches…" dialog.** A checklist of merged and gone branches, never the current branch or protected names. Use `-d` when reachability proves it merged. Use `-D` only after explicit confirmation of a heuristic match.
4. **New-branch dialog.** Add a base dropdown and a "publish after creating" option. Always pass `--no-track`, then `git push -u <remote> <name>`. Creating from `origin/main` auto-tracks `origin/main`. Under `push.default=simple` that makes a plain push fail, and under `upstream` it would push to main. Canopy's `push()` runs a plain `git push` whenever an upstream exists. The current UI does not offer a remote base, but the picker would. Git's `branch.autoSetupMerge` is verified; the push failure is known git behaviour (unverified from a fetched source).
5. **Publish any local branch.** Add `Op::PushBranch`, and a toast action to create the pull request afterwards.
6. **"Update from base".** A one-step rebase or merge of `origin/<base>`, using the existing confirmation flows.

### B3. Keyboard

Bare keys must not collide with existing ones. `F`, `i` and `I` are unused.

- `F` opens a context-aware flow menu. On `develop` it offers "Start feature". On `feature/*` it offers "Finish". If flow is not initialised it offers "Initialise".
- Palette commands: "Flow: Start feature, release or hotfix…", "Flow: Finish current…", "Branch: Publish", "Branch: Clean up merged…", "Branch: Update from base", "Branch: Set upstream…".
- Existing `n` and `m` stay. The new-branch dialog is Tab-navigable.

## C. .gitignore

**What clients offer:**
- **Fork:** add files, folders or custom patterns since 1.0.39. A pattern UI with a preview of affected files (Windows 1.18). A template picker in 2.65.
- **GitKraken:** right-click Ignore by name, extension or directory. It writes the root `.gitignore` only. "Ignore and Stop Tracking" for tracked files. A template dropdown in the init dialog.
- **Tower:** right-click Ignore, and it asks to untrack tracked files.
- **Sourcetree:** an Ignore dialog with "this repository only" versus a global list.
- **GitHub Desktop:** "Ignore File" in the context menu. A pattern editor in repository settings. A template dropdown at repository creation. Known bugs: ignores `core.excludesFile` (#16028), wrote CRLF line endings (#3514), no folder item at first (#13619).
- **Sublime Merge:** only Tools > Edit .gitignore. Requested in #2018.
- **lazygit:** `i` offers "Add to .gitignore" or ".git/info/exclude". A global target is requested in #5108. An ignored-files view is requested in #5880.

**Recommendation, must-have:**
1. **Ignore menu** on untracked entries, with these choices:
   - this file, as `/path`
   - all `*.ext`
   - this folder, as `/dir/`
   - a custom pattern with a live preview. The preview needs no write: add an internal rule with `add_ignore_rule`, test with `is_path_ignored`, then `clear_ignore_rules`, all on a worker-side `Repository`.
2. **Destination dropdown:**
   - root `.gitignore` (default)
   - the nearest `.gitignore` when one exists
   - the repository-local exclude file, found with `git rev-parse --git-path info/exclude` (never join `.git`)
   - the global file, from `core.excludesFile` or the XDG default, with confirmation
3. **Stop tracking and ignore:** `git rm --cached -r -- <path>` plus the rule. Warn that collaborators lose the file on pull.
4. **Safe writes:**
   - LF line endings only.
   - Skip duplicates (case-insensitive when `core.ignorecase` is set).
   - Escape a leading `#` or `!`, trailing spaces, and `*?[]\`.
   - Write bytes, not assuming UTF-8.
   - Refuse a `.gitignore` or exclude file that is a symlink.
5. **Verify after writing** with `git check-ignore -v`. If the path is still not ignored, say which rule wins.
6. **"Why is this ignored?"** with `git check-ignore -v --no-index -z`, which shows source, line and pattern. libgit2 only returns a boolean. The `--no-index` flag is needed for tracked files (verified).
7. **Template picker in the init dialog**, plus a palette command for existing repositories.

**Templates:** github/gitignore is CC0-1.0 (verified).
- Bundle the root and `Global/` templates: about 123 files in `Global/` (verified) and 150 or more at the root. Skip `community/`.
- Generate the bundle with a script pinned to a commit hash. Never fetch at runtime.
- Add a licence row in the assets README, per AGENTS.md.
- Size is unverified: I estimate under 400 KB raw. The repository is 3 MB with history (`docs/stress-test.md`).
- Allow selecting several templates, written as commented sections.

**Nice-to-have:**
- Show-ignored toggle `I`: `include_ignored(true)` without `recurse_ignored_dirs`. Directories then collapse to one entry, which is essential for `node_modules` and `target`. Run it as a separate job.
- An in-app `.gitignore` editor.
- "Remove this rule" from the "why" popup.

**Pitfalls (verified against gitignore(5) unless marked):**
- Negation cannot re-include a file under an excluded parent directory.
- Precedence is the `.gitignore` in the same directory or parents (closer wins), then the repository-local exclude file, then `core.excludesFile`.
- A pattern containing a slash is relative to its own `.gitignore`. A trailing slash matches directories only. Trailing spaces are ignored unless escaped. Tracked files are unaffected.
- A trailing-slash pattern does not match a symlink to a directory (unverified).
- **Current status issue:** `status.rs` sets `recurse_untracked_dirs(true)`. An un-ignored `node_modules` would list 100,000 or more rows. Above a threshold (say 2,000 entries), collapse to directory rows with an "Ignore folder" button.

**Keys:** `i` in Changes opens the ignore menu. Navigate with `j` and `k`, choose with Enter or `1` to `4`. `I` toggles the ignored view.

## D. Submodules and links

### D1. Submodules

**Current state:** `status.rs` uses `exclude_submodules(false)`, so libgit2 computes submodule state on every refresh. Other tools report this as the slow part. libgit2's `git_submodule_status` is the bottleneck, and `--ignore-submodules` is the usual workaround (snippets).

**Client behaviour:**
- **Tower:** sidebar list, double-click to open, add through the "+" menu, update on pointer move, and sync and repair guides.
- **GitKraken:** a Submodules pane, add and update, an init prompt after clone, and a "Keep Submodules Up to Date" setting.
- **Fork:** auto re-init on pull, a submodule icon, and a fix for diffs when the commit is missing.
- **GitHub Desktop 3.1:** a submodule diff showing the old and new SHA, an "Open submodule" button, and messages for dirty content.
- **Sourcetree:** recursion only at clone. Complaints say there is no manual or recursive update on pull or checkout.
- **lazygit:** Submodules tab. Enter opens, `u` updates, `i` initialises, `b` bulk, `n` new, `d` removes, `e` edits the URL.

**Recommendation:**
- **Status.** Pass `exclude_submodules(true)` on the hot path. Compute submodule rows in a separate `Job::Submodules` using `head_id`, `index_id` and `workdir_id` on libgit2's submodule type, which are cheap. Staged means `index_id` differs from `head_id`. Unstaged means `workdir_id` differs from `index_id`. Check dirtiness lazily, per submodule, on demand.
- **Sidebar section.** Show these states: not initialised, new commits, dirty, conflicted. Treat a detached HEAD as normal.
- **Actions:**
  - open in a tab
  - `git submodule update --init [--recursive] -- <path>`, plus update all
  - `sync --recursive`
  - add
  - remove, with confirmation: `deinit -f`, `git rm`, then delete `.git/modules/<name>`
- **Keep in sync.** Offer a setting, on by default, that runs `git submodule update --recursive` (no `--init`, no `--force`) after checkout, pull, merge and rebase. Show a banner for uninitialised submodules.
- **Clone.** `--recurse-submodules`, default on.
- **Diff.** A submodule entry (mode 160000) shows `old..new`, the commit log from the submodule if initialised, "commit not fetched" if missing, and "Open submodule". Staging the pointer is `git add -- <path>`.
- **Discard.** For a pointer change, discard must run `git submodule update -- <path>`. Plain checkout does not touch it. Block it if the submodule is dirty.
- **Credentials.** Each submodule clone triggers its own askpass prompt, so the dialog must show the URL. Relative URLs resolve against the superproject's remote.
- **Test fixtures.** They likely need `-c protocol.file.allow=always` for local-path submodules (unverified, from memory).
- **Scope.** Must-have: list, state, open in tab, update and init, sync-after-checkout, the diff card. Nice-to-have: add, remove, nested tree, update-URL.

### D2. Symlinks and other links

- **Representation.** A symlink is a blob holding the target path, with mode 120000 (verified via Git for Windows snippets). Show "Symbolic link: old → new", never a text hunk. A file/symlink type change should be one entry with both sides. Fork's #1938 shows only the first diff, because git emits two. `to_file_diff` merges deltas, so add a test.
- **Watcher (verified in source).** `worker.rs:581` calls `notify::recommended_watcher` with the default config. In `notify` 8.2.0, `Config::default()` sets `follow_symlinks: true`, and the inotify backend passes it to `WalkDir::follow_links`. A repository containing `link -> /` or `-> ~` makes Canopy watch outside the repository. Switch to `with_follow_symlinks(false)`.
- **Safe path helper.** Cloned repositories are untrusted. Add one helper used by every file read, write and open-in-editor. It rejects `..`, absolute paths, and any symlink component. `conflict.rs` already does this check (lines 564 and 646). Otherwise "Ignore" could append to a symlinked `.gitignore` that points at `~/.bashrc`. Opening an outside-pointing link in an editor should show a warning.
- **Windows.** With `core.symlinks=false`, git checks symlinks out as plain text files containing the target, and status stays clean (verified snippets). Show "symlink stored as text on this system", with a hint to enable Developer Mode and run `git config core.symlinks true`. GitHub Desktop #20269 and Sublime Merge #292 show the confusion. Junctions are not symlinks to Rust's `is_symlink`; treat reparse points as links for Canopy's own file access (unverified). Hard links need no special UI.
- **Executable bit.** Mode-only changes currently show nothing (`diff.rs:85`). Show "mode 100644 → 100755" and a badge in file lists. `core.fileMode=false` suppresses it, as in git. The hunk patch emits no mode line for modified files, so staging a hunk stages content only.
- **Embedded repositories.** A nested non-submodule repository shows as an untracked directory. Mark it as a nested repository with "Open in tab", "Add as submodule…" and "Ignore". `git add -A` (Canopy's `stage_all`) would add it as a gitlink with no `.gitmodules`. Exclude it from "Stage all" unless confirmed.
- **Worktrees.** Keep them at "Next", as in the backlog. Tower has the deepest support, GitKraken is limited, Sourcetree has no GUI, and lazygit has a full panel (`n`, Space, `o`, `d`). Fix a prerequisite first. `worker.rs:584` watches `root.join(".git")`. In a linked worktree or a submodule opened in a tab, `.git` is a file and the real git directory lies elsewhere, so ref changes there never trigger a refresh. Watch the path returned by `Repository::path()` and the common directory instead.

## Priority order
1. Watcher fixes: no symlink following, and the real git directory.
2. LFS status check.
3. `--no-track` and publish-branch.
4. Ignore menu.
5. Submodule status and sidebar.
6. git-flow.
7. Templates and branch cleanup.

## Sources
- https://github.com/libgit2/libgit2/issues/3373 — libgit2 marks LFS files modified when git says clean.
- https://github.com/gitbutlerapp/gitbutler/issues/2595 — GitButler shows all LFS files as changes.
- https://github.com/sublimehq/sublime_merge/issues/425 — Sublime Merge shows LFS files as unstaged.
- https://github.com/sublimehq/sublime_merge/issues/1138 — Sublime Merge push fails when git-lfs is not on PATH.
- https://github.com/gitui-org/gitui/issues/2812 — libgit2 exposes the filter hook, git2-rs does not.
- https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md — pointer format and size limit.
- https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-config.adoc — askpass, `GIT_LFS_SKIP_SMUDGE`, `GIT_LFS_FORCE_PROGRESS`, `locksverify`, `allowincompletepush`.
- https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-ls-files.adoc — `*` versus `-` markers.
- https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-install.adoc — `--local`, `--skip-smudge`, hooks.
- https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-pre-push.adoc — the pre-push hook.
- https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-track.adoc — track and `--lockable`.
- https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-locks.adoc — `--verify`, `--local`, `--json`.
- https://github.com/git-lfs/git-lfs/blob/main/docs/api/locking.md — lock object and verify response.
- https://docs.gitlab.com/user/project/file_lock — GitLab file locks on all tiers.
- https://github.com/desktop/desktop/issues/6240 — GitHub Desktop's "Initialize Git LFS" prompt.
- https://github.com/petervanderdoes/gitflow-avh/wiki/Reference:-git-flow-release — AVH release options.
- https://raw.githubusercontent.com/petervanderdoes/gitflow-avh/develop/git-flow-feature — AVH feature commands.
- https://raw.githubusercontent.com/petervanderdoes/gitflow-avh/develop/git-flow-release — AVH release commands.
- https://raw.githubusercontent.com/petervanderdoes/gitflow-avh/develop/git-flow-hotfix — AVH hotfix commands.
- https://raw.githubusercontent.com/petervanderdoes/gitflow-avh/develop/git-flow-init — AVH defaults and config keys.
- https://github.com/gittower/git-flow-next/blob/main/CONFIGURATION.md — git-flow-next config and AVH key import.
- https://github.com/git/git/blob/master/Documentation/git-branch.adoc — `-d` versus `-D`, upstream options.
- https://github.com/git/git/blob/master/Documentation/git-merge-tree.adoc — `--write-tree` mode.
- https://github.com/git/git/blob/master/Documentation/git-cherry.adoc — patch equivalence.
- https://github.com/jesseduffield/lazygit/blob/master/docs/keybindings/Keybindings_en.md — lazygit branches, submodules and worktrees keys.
- https://github.com/jesseduffield/lazygit/issues/5108 — missing global-excludes target.
- https://github.com/jesseduffield/lazygit/issues/5880 — ignored-files view request.
- https://github.com/desktop/desktop/issues/16028 — GitHub Desktop ignores `core.excludesFile`.
- https://github.com/desktop/desktop/issues/3514 — GitHub Desktop wrote CRLF into `.gitignore`.
- https://github.com/sublimehq/sublime_merge/issues/2018 — Sublime Merge lacks gitignore management.
- https://github.com/github/gitignore — CC0-1.0 template repository.
- https://github.com/git/git/blob/master/Documentation/gitignore.adoc — ignore precedence, negation, tracked files.
- https://github.com/git/git/blob/master/Documentation/git-check-ignore.adoc — `-v`, `--no-index`, output format.
- https://github.com/git/git/blob/master/Documentation/git-submodule.adoc — submodule commands and removal.
- https://github.com/git/git/blob/master/Documentation/gitmodules.adoc — `submodule.<name>.ignore` values.
- https://github.com/desktop/desktop/pull/15176 — GitHub Desktop submodule diff.
- https://github.com/git/git/blob/master/Documentation/git-status.adoc — porcelain v2 format.
- https://github.com/fork-dev/Tracker/issues/1938 — Fork typechange diff bug.
- https://github.com/sublimehq/sublime_merge/issues/292 — Sublime Merge symlink status bug on Windows.
- https://github.com/desktop/desktop/issues/20269 — `core.symlinks` confusion on Windows.
- Search-result summaries only, because the sites were blocked:
  - GitKraken: help.gitkraken.com (LFS, git-flow, submodules, staging and ignore) and gitkraken.com/blog (libgit2 migration).
  - Tower: git-tower.com (LFS, git-flow, submodules, branch management, Tower 17 merge detection).
  - Fork: git-fork.com/releasenotes (LFS, git-flow, ignore, templates, submodules).
  - Sourcetree: community.atlassian.com and jira.atlassian.com SRCTREE-4663.
- Local files read:
  - /home/user/Canopy/crates/canopy-core/src/status.rs
  - /home/user/Canopy/crates/canopy-core/src/ops.rs
  - /home/user/Canopy/crates/canopy-core/src/diff.rs
  - /home/user/Canopy/crates/canopy-core/src/cli.rs
  - /home/user/Canopy/crates/canopy-app/src/worker.rs
  - /home/user/Canopy/crates/canopy-app/src/views/sidebar.rs
  - /home/user/Canopy/crates/canopy-app/src/views/dialogs.rs
  - /home/user/Canopy/crates/canopy-app/src/keymap.rs
  - /home/user/Canopy/docs/backlog.md
  - git2 0.21.0 and notify 8.2.0 sources under the cargo registry