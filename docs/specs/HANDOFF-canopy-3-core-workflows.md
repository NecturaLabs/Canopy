# Handoff: canopy-3/core-workflows

**Status: merged, kept as the reference for wiring the app.** The branch was merged into `main` as
`4556dfb`; the session, its local branch, its worktree and its remote branch are gone. As of
8 October 2026 on PR #8, the app wires submodule management, pointer diffs and configurable
updates; remaining acceptance is tracked in GitHub issue #60. The git-flow and merged-branch
UI is wired (#58; its live check with `tools/usersim.sh` is still open there); LFS UI remains in
#57: review the corresponding core when it is wired.
GitHub Issues is the active tracker. The historical "Not done" list below is reference, not
the current task queue; the pull-configuration fixes now use the Git CLI. The installer test's
failures on a desktop host are fixed: the unbuffered start
in `7d79afc`, and `install.sh` runs `update-desktop-database` only when a `mimeinfo.cache`
already exists. Delete this file when the UI work has landed.

Branch `canopy-3/core-workflows`: the canopy-core half of `lfs-flow-spec.md` parts A, B1 and D1.
Only `crates/canopy-core/` and this file changed. The
file is named with `-` for the branch's `/`.

## Added

**`cli.rs`:** `Output` and `run_unchecked(workdir, args)` for commands whose exit code is the
answer (`merge-tree`, `diff --quiet`); `version_number() -> Option<(u32, u32)>` (cached).

**`flow.rs` (git-flow, no `git flow` binary):**
- Types: `FlowKind` (Feature, Bugfix, Release, Hotfix), `FlowConfig` (AVH keys; `Default` = AVH
  defaults; `prefix`, `branch`, `tag`, `default_base`, `classify(branch)`), `FlowFinish`
  (`delete_branch` true, `squash` false, `tag` true, `tag_message`).
- `Repo::flow_config()` returns `None` until both branch keys are set.
- `Repo::flow_suggested_config()` gives the stored config, or the defaults with production set
  to `main` or `master`, whichever exists.
- `Repo::flow_init(&config)` validates, writes every key with `git config --local`, and creates
  develop from production (`--no-track`) if it is missing.
- `Repo::flow_start(kind, name, base)` runs `git switch --no-track -c <prefix><name> <base>`.
  Release and hotfix also require a clean tree, one branch of the kind at a time, and an unused
  tag.
- `Repo::flow_finish(kind, name, &options)` follows the spec's table:
  - Feature and bugfix: `--no-ff` (or `--squash` plus `commit --no-edit`) into develop, then
    `branch -d` (`-D` after a squash).
  - Release and hotfix: `--no-ff` into production, `tag -a`, a back-merge of the tag (or of
    production with no tag) into develop, then `branch -d`.
  - It ends on develop.
  - Before anything changes, it requires a clean tree and no operation in progress, and runs
    `merge-tree --write-tree` for every merge. The back-merge is checked against a stand-in
    commit from `commit-tree --no-gpg-sign`, an unreferenced object.
  - A conflict refuses with the paths, and nothing is changed.
  - It needs Git 2.38+ and refuses on older git.
- `Repo::flow_push(kind, name, remote, delete_remote, progress)` pushes develop, plus
  production and the version tag for a release or hotfix, as named refspecs, never `--tags`.
  It optionally adds `:refs/heads/<branch>`.
- `Repo::flow_branches(&config, kind)` lists the local branches of a kind.

**`merged.rs`:** `Repo::merged_branches(base, squash) -> Vec<MergedBranch { name, is_head,
reason: Option<MergedReason> }>`. Checks run in this order:
1. `Reachable`: proof, so `branch -d` is enough.
2. `UpstreamGone`: `branch.<n>.merge` is set but its tracking ref is missing.
3. `SquashMerged`: `merge-tree` of base and tip gives the base tree. It runs only when `squash`
   is set and git is 2.38+, with one process per branch.

The base itself is excluded. `MergedReason::label()` gives the tooltip text and `is_proof()` says
whether `-d` suffices.

**`lfs.rs`:**
- `FileEntry.lfs: bool` (new field) is set for every status entry and commit-details file
  with `filter=lfs`.
- `LfsPointer::parse` accepts a pointer under 1 KiB with an `oid sha256:<64 lowercase hex>` line
  and a `size` line, and flags any `ext-` lines.
- `Repo::lfs_files()` lists stage-0 index entries with `filter=lfs` whose blob is a pointer, with
  `downloaded`.
- `Repo::lfs_storage_dir()` honours `lfs.storage`, relative to the common git dir.
- `Repo::lfs_object_path(oid)` returns `None` unless `oid` is a valid SHA-256.

**`submodule.rs`:**
- `Repo::submodules(check_dirty) -> Vec<SubmoduleInfo>` gives name, path, url, `head_id`,
  `index_id`, `workdir_id`, `conflicted` and `dirty: Option<bool>`. Helpers: `is_initialized`,
  `has_new_commits`, `is_staged`, and `state()` (Conflicted > NotInitialized > NewCommits >
  Dirty > Clean).
- `Repo::submodule_dirty(path)` checks one submodule, lazily.
- `submodule_update(path, init, recursive, progress)` and `submodule_add(url, path, branch,
  progress)` stream through `run_streaming`. `submodule_sync(path, recursive)` runs plainly.
- `submodule_remove(path)` runs `deinit -f`, then `git rm -f`, then deletes
  `git rev-parse --git-path modules/<name>`. Before deleting, it refuses a name that is not a
  plain relative path, a directory reached through a symlink or outside `modules/`, and a name
  that holds another submodule's repository.
- `FileDiff.submodule: Option<SubmoduleChange { old, new, old_present, new_present }>` (new
  field) is filled by `Repo::file_diff` for mode-160000 paths.

**Error variants:** no new ones. Refusals are `Error::Invalid` with a user-facing sentence; git
failures stay `Error::Cli` with git's stderr.

## What the worker and UI must do

- Each mutation (`flow_init`, `flow_start`, `flow_finish`, `flow_push`, `submodule_update`,
  `submodule_sync`, `submodule_add`, `submodule_remove`) becomes an `Op`, followed by a fresh
  snapshot.
- `flow_push`, `submodule_update` and `submodule_add` are network jobs with progress lines.
- `merged_branches(base, true)`, `submodules(false)`, `submodule_dirty` and `lfs_files()` become
  read `Job`s. Keep them off the refresh hot path; `merged_branches` with squash runs one git
  process per branch.
- Confirmations:
  - `submodule_remove` loses unpushed commits in the submodule.
  - `flow_push` with `delete_remote`.
  - Deleting a branch whose `MergedReason` is not `is_proof()` needs `-D`
    (`force_delete_branch(name)`) after explicit confirmation.
- The flow finish dialog maps to `FlowFinish`; the toast shows the returned summary or the
  refusal text.
- `FlowConfig::classify` tells the `F` menu whether the current branch is a flow branch.
- Show an LFS badge from `FileEntry.lfs`, a "not downloaded" chip from `lfs_files()`, and a
  submodule diff card from `FileDiff.submodule` (old..new, "commit not fetched" when a side is not
  present).
- `status.rs` still uses `exclude_submodules(false)`. The spec suggests `true` on the hot path,
  but that hides staged submodule pointer changes from the status lists, so it is a UI decision.
  It is left as is.

## Tests

- `crates/canopy-core/tests/workflows.rs` has 26 new integration tests (25 on Windows: the
  symlink test is Unix-only). The git-flow tests compare graph, trees, refs, tag subjects and HEAD
  with the same commands run by the git CLI in a twin repository. The fixture helper points
  `GIT_CONFIG_GLOBAL` at a missing file. Canopy's own git processes still see the global
  configuration, as they do for `tests/operations.rs`; `tools/check.sh` isolates it.
- `cli.rs` and `lfs.rs` gained 4 unit tests.
- A mutation check turned the relevant tests red for: the flow `--no-ff`/preflight, the
  submodule name lookup, and every review fix.
- An independent review (Sonnet) found:
  - an orphan branch aborting `merged_branches`;
  - a stale index in `lfs_files`;
  - an invalid tag name stopping a finish half-way;
  - a squash refused under `merge.ff=only`;
  - an unstaged `.gitmodules` leaving a half-removed submodule.

  All five are fixed, each with a test.
- `cargo test -p canopy-core --test workflows` passes, as does clippy with `-D warnings`.

## Not done, or for canopy-1

1. **LFS hash step not implemented (needs a crate).** No SHA-256 crate is in `Cargo.lock`, and
   `Cargo.toml` is out of scope. Stat-dirty but unchanged LFS files therefore still show as
   modified. To finish it, add `sha2` (RustCrypto, MIT/Apache-2.0, check cargo-deny). Then, in
   `status::collect`, for unstaged `Modified` entries with `lfs`: read the index blob,
   `LfsPointer::parse` it, skip if `extensions`, compare the working file size with
   `pointer.size`, stream SHA-256 on a match, and drop the entry when it equals `pointer.oid`.
   Cache the result by `(path, size, mtime, blob oid)` in the worker.
2. `flow_finish` always merges with `--no-ff`, as the spec's table says. AVH fast-forwards a
   single-commit feature, and Canopy does not. Feature "rebase before finish" is not offered.
   Hotfix-into-an-open-release (nvie) is not done; AVH behaviour there is unverified.
3. `flow_finish` refuses on git older than 2.38.
4. **Pre-existing bug (not fixed, `ops.rs` is shared with canopy-2).** `Repo::pull` decides
   whether to pass `--rebase`/`--no-rebase` from `config_string("pull.rebase")`, which is
   libgit2's view of the config. libgit2 ignores `GIT_CONFIG_GLOBAL` (and `includeIf`, as git
   resolves it), so when they disagree git gets no flag and fails with "Need to specify how to
   reconcile divergent branches".
   - Repro: put `pull.rebase=true` in `~/.gitconfig`, then run `tools/check.sh`.
     `pull_merge_creates_a_merge_commit` and `pull_rebase_replays_local_commits` fail. Without
     `check.sh`'s isolation, `pull_merge_creates_a_merge_commit` and
     `pull_fast_forward_only_refuses_diverged_branches` fail on the same machine.
   - Fix: read the value with `git config --get pull.rebase` through `cli`.
5. **`packaging/test-install.sh` failed on a desktop Linux host (fixed, see the status line).**
   - `update-desktop-database` writes `~/.local/share/applications/mimeinfo.cache`, which trips
     the "nothing else written" and "home is empty" checks.
   - With Python 3.14, `http.server`'s start-up line is buffered, so "the test server did not
     start" unless `PYTHONUNBUFFERED=1` is set.
   - It passes in CI. Not in scope for canopy-3.
6. `tests/workflows.rs` carries its own copy of the `git`/`fixture` helpers from
   `tests/operations.rs`, to avoid conflicts with canopy-2's edits there. They can move to
   `tests/common/` when merging.
7. Review notes not changed:
   - A branch with no commits of its own (tip equals the base) reports `Reachable`, as
     `git branch -d` agrees. A "delete merged" list may want to show it differently.
   - `lfs::mark` calls `get_attr` for every status entry on each refresh (cost not measured).
   - `commit_details` flags `lfs` from the current attributes, not the commit's.
   - `submodule_at` matches paths after turning `\` into `/`.
   - The symlink and `canonicalize` guards in `submodule_remove` were tested on Linux only.
8. Platform check: this machine is Linux, so the macOS `packaging/test-install.sh` run was not
   done.
