# Handoff: canopy-2/core-safety

**Status: merged, kept as the reference for wiring the app.** The branch was merged into `main` as
`d0782bf` (commits `51c58a7`, `329293c`, `76b160d`); the session, its local branch and its
worktree are gone, and so is its remote branch. The app has taken up only part of
it: branch delete asks `git branch -d` first (section 1, `7003ef0`), and `51c58a7` and
`paths::safe_join` are live. The rest still sits behind compatibility wrappers, and each piece of
UI work is a `docs/backlog.md` entry (`.gitignore`, file and directory links, research bugs, and
the app's calls to `publish_branch`, `untracked_children` and `stage_all_except_repositories`).
Delete this file when the last of those lands and the wrappers are gone. Sections below describe
the branch as it was handed over; "Left out, and known issues" says what became of each item.

Core only (`crates/canopy-core`). One test fixture in the app also changed, with the owner's
approval: `tab.rs` `entry()` gains `old_mode: None, new_mode: None`. Rebased on `main` after the
canopy-3 merge.
Spec: `docs/specs/lfs-flow-spec.md` B2, C and D2; brief: `docs/specs/parallel-sessions.md`.
(The file name flattens the branch's `/` so it does not create a folder.)

## Added, and what the worker/UI must do

The app still calls the old entry points below, kept as wrappers so `main` builds. Move the
worker to the new ones, then delete the wrappers.

### 1. Branch delete (`ops.rs`)
- `Repo::delete_merged_branch(name)`: `git branch -d -- name`. Unmerged fails with
  `Error::Cli { command: "branch", stderr }`, and the stderr contains "not fully merged".
- `Repo::force_delete_branch(name)`: `git branch -D -- name`.
- Wrapper: `delete_branch(name, force)` dispatches to the two.
- UI: done in `7003ef0`. The sidebar sends `Op::DeleteBranch { force: false }`, which the worker
  reports as `OpKind::SafeDelete`; on "not fully merged" the tab offers a forced delete behind a
  danger confirmation (`RepoTab::force_delete_offer`). The wrapper `delete_branch(name, force)`
  is still what the worker calls, and nothing calls `delete_merged_branch` or
  `force_delete_branch` directly yet.

### 2. New branches and publishing (`ops.rs`)
- `create_branch` now always passes `--no-track` (including from `origin/main`, and under
  `branch.autoSetupMerge=always`). It also refuses a name or start point that starts with `-`
  (`Error::Invalid`), which git used to read as an option: `create_branch("-D", Some("x"), …)`
  deleted `x`. The signature is unchanged.
- `Repo::publish_branch(name, remote, progress) -> Result<String>`:
  `git push --progress --set-upstream <remote> refs/heads/<name>:refs/heads/<name>`, for any local
  branch. It returns `Error::Invalid` for an unknown remote (or one starting with `-`) and for a
  missing local branch.
- UI: add `Op::PushBranch { name, remote }` (a streaming network job) on the branch menu, plus
  "Branch: Publish" in the palette. The new-branch dialog can offer a remote base and "publish
  after creating".

### 3. Modes and type changes (`model.rs`, `diff.rs`, `status.rs`, `repo.rs`)
- `FileEntry` and `FileDiff` gain `old_mode` / `new_mode: Option<u32>` (`0o100644`, `0o100755`,
  `0o120000` symlink, `0o160000` gitlink; `None` when that side is absent), plus
  `mode_change() -> Option<(old, new)>`. Status (staged and unstaged), `commit_details` and
  `file_diff` (all three sources) fill them.
- Diffs use `include_typechange`. A file to symlink swap is one `ChangeKind::Typechange` entry with
  both modes, in status, commit details and diffs. `core.fileMode=false` hides mode-only changes,
  as git does.
- `apply_hunk` and `apply_lines` on a typechange return `Error::Invalid` ("…changed type…"),
  telling the user to stage or discard the whole file.
- A typechange diff has no hunks and no line counts, so the link target is not in it. A
  symlink-to-symlink retarget (120000 on both sides, `Modified`) does show the target as a
  one-line hunk.
- UI: show "mode 100644 → 100755" and a badge in file lists. For a typechange, show "File →
  symbolic link" (or back) from the modes. `views/diff.rs:225` already shows "Only the file mode
  or metadata changed"; it can now print the modes. Disable hunk and line staging for
  `Typechange`.

### 4. Collapsed untracked folders (`status.rs`, `model.rs`)
- `status::collect` keeps its signature and uses `status::UNTRACKED_LIMIT` (2000).
  `status::collect_with(repo, limit)` takes an explicit limit.
- New `WorkingStatus::untracked_dirs: Vec<UntrackedDir { path /* ends in '/' */, files, repository,
  contains_repository }>`.
  Above the limit, each top-most untracked folder is one entry with its file count, and its files
  are not in `unstaged`. `is_clean()` and `len()` include it.
- `contains_repository` marks a collapsed folder with a nested repository somewhere below it.
  "Stage folder" on such a row (`git add -- dir/`) would add that repository as a gitlink, so ask
  first or expand it.
- Expand one level: `Repo::untracked_children(dir)` (also `status::untracked_children`), which
  returns `UntrackedListing { files, dirs }`. It refuses (`Error::Invalid`) `.` or a folder that is
  itself a repository.
- Names that are not UTF-8 are still listed. Detection and pathspecs use raw bytes, but
  `FileEntry::path` is lossy, as before.
- The first pass no longer recurses. A second, path-limited pass runs only when untracked folders
  exist.
- UI: render `untracked_dirs` as folder rows ("node_modules/ — 12,345 files") with Expand, Stage,
  Ignore folder and Discard. Expanding sends a read `Job` calling `untracked_children`. Until the
  UI renders them, files inside collapsed folders do not show in Changes.

### 5. Nested repositories in Stage all (`ops.rs`, `status.rs`)
- An untracked folder containing `.git` (not a submodule) is always an `UntrackedDir` with
  `repository: true`, at any depth.
- `Repo::stage_all_except_repositories() -> Result<Vec<String>>` runs
  `git add -A -- :/ ':(top,exclude,literal)<repo>'…` and returns the skipped folders.
- If a nested repository's name is not UTF-8, it cannot be excluded through a `&str` argument.
  `stage_all_except_repositories` then returns `Error::Invalid` and stages nothing.
- Wrapper: `stage_all()` does the same and drops the list. It never adds a gitlink any more.
- It finds the repositories with a full status pass (`collect_with(usize::MAX)`), which also runs
  canopy-3's `lfs::mark`, so Stage all may hash modified LFS files once.
  Until the UI is wired, the app's Stage all silently skips nested repositories.
- UI: when the list is non-empty, ask: "Add as submodule…", "Ignore", or "Stage anyway"
  (`Repo::stage(&[path])`). Mark these rows as nested repositories, with "Open in tab".

### 6. Safe paths (`paths.rs`, `error.rs`)
- `paths::safe_join(root, rel)` rejects empty, `.` (the root itself), NUL, absolute, `..`, any `.git` component
  (case-insensitive; on Windows also `.git.`, `.git ` and `GIT~1`) and any existing symlink
  component (on Windows, any reparse point). `paths::check_relative(rel)` is the text-only part.
- New error: `Error::UnsafePath { path, reason }`; nothing was read or written.
- Used by the conflict resolver (`conflict_file`, `write_resolution`, `mark_resolved`; the old
  `relative_path` and `write_atomic` moved to `paths.rs`), `conflict_view` (which previously read
  through symlinks) and ignore-file writes.
- Behaviour change: a conflicted path behind a symlinked folder is now refused. A conflicted file
  that is itself a symlink still gets "Choose our or their version", and `mark_resolved` stages it
  without reading the target.
- UI (not core): open-in-editor and any app-side file access should call `safe_join`, warning
  for links that point outside the repository. The watcher's `follow_symlinks(false)` (spec D2) is
  app-side and now done in `main` (`worker.rs`).

### 7. Ignore rules (`ignore.rs`, `cli.rs`)
- `IgnoreRule::{File(path), Extension(path), Folder(path), Custom { pattern, near }}` and
  `IgnoreTarget::{Root, Nearest, RepositoryExclude, Global}` (`ALL`, `label()`).
- `Repo::ignore_destination(target, path) -> IgnoreDestination { file, base }`, for showing the
  file in the destination dropdown. The exclude file comes from `git rev-parse --git-path
  info/exclude`. The global file comes from `git config --path --get core.excludesFile`, else
  `$XDG_CONFIG_HOME/git/ignore` or `~/.config/git/ignore`.
- `Repo::add_ignore_rule(&rule, target) -> IgnoreOutcome { file, pattern, added, check }`. It
  appends one LF-terminated line as bytes, after a missing final newline is fixed. It skips
  duplicates (case-insensitive under `core.ignorecase`) and escapes `\ * ? [ ]`, a leading `#`/`!`
  and trailing spaces. Patterns are relative to the nearest `.gitignore`'s folder. It refuses a
  symlinked file (`Error::UnsafePath`), and names with a line break or custom patterns that are
  comments or span lines (`Error::Invalid`). `check` is `why_ignored(path)` after writing: `None`,
  or a `negated()` match, means it is still not ignored, so show which rule wins.
- `Repo::preview_ignore_rule(&rule, target, &paths) -> IgnorePreview { ignored, unignored }`.
  It writes nothing. It asks git which rule decides each path (one `check-ignore -v -n` process),
  then tests the new pattern with libgit2 in-memory rules on a separate handle. The new rule
  counts only where its file outranks that rule, as in git: a closer `.gitignore` beats a parent
  one, every `.gitignore` beats `info/exclude`, `info/exclude` beats the global file, and in the
  same file the appended line wins. Pass the untracked entries (and folder rows) the view has.
- `Repo::why_ignored(path) -> Option<IgnoreMatch { source, line, pattern }>` uses
  `git check-ignore -v --no-index -z --stdin` (git requires `--stdin` with `-z`).
- `Repo::stop_tracking(path)` runs `git --literal-pathspecs rm --cached -r -q -- path`. Warn that
  collaborators lose the file on pull.
- `cli::run_answer` (crate-private) is for commands whose exit code is the answer.
- Rules are checked (`check_relative`) before anything is written; a bad `near` writes nothing.
- UI: `i` in Changes opens the ignore menu (file, `*.ext`, folder, custom with live preview),
  destination dropdown (global needs confirmation), "Stop tracking and ignore", and "Why is this
  ignored?". Every one of these runs on the worker as an `Op` or `Job`.

## Also fixed (separate commit)

- `stage`, `unstage` and `discard` now pass `--literal-pathspecs`. Before, discarding an untracked
  file named `[ab].txt` also deleted an untracked `b.txt` (`git clean -f -- [ab].txt` glob-matched
  it). The regression test fails on the old code.

## Tests

- `crates/canopy-core/tests/safety.rs`: 28 integration tests against temporary repositories, plus
  `stage_unstage_and_discard_treat_paths_literally` in `tests/operations.rs`.
  Three were checked to fail on the old code: no-track, typechange and literal paths. The
  reviewer reproduced the `create_branch` option-name and non-UTF-8 bugs on the old code, before
  their tests were written.
- Unit tests: `paths.rs` 4, `ignore.rs` 5.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`
  and `cargo test --workspace --locked` pass on the rebased branch: app 92 (1 ignored), core 60
  unit, 15 line staging, 37 operations, 28 safety, 26 workflows, hosting 27 + 17. Fixtures pin `core.autocrlf false`, point `core.excludesFile` away from the developer's,
  and use names Windows allows (the non-UTF-8 test is Linux-only).
- Linux only. The Unix-only tests are `#[cfg(unix)]`: chmod, symlinks. Windows reparse-point and
  `GIT~1` handling is untested.

## Left out, and known issues

- Not built (out of brief): gitignore templates, the show-ignored view, and "Update from base"
  (backlog: `.gitignore`, Feature-branch workflows). The watcher symlink and git-dir fixes were
  done in the app afterwards.
- Docs the brief kept out of this branch: `docs/architecture.md` now has the `paths` and `ignore`
  rows, and `CHANGELOG.md` records the user-visible changes (literal pathspecs, Stage all and
  nested repositories, `--no-track`, safe paths). `docs/features.md` gets rows when the UI lands.
- Above the limit, every top-level untracked folder collapses, even a small one. A
  `.gitignore` starting with a UTF-8 BOM defeats de-duplication on its first line.
  `conflict_view` on a conflicted file behind a symlinked folder now errors instead of reading
  through the link.
- A nested repository that is freshly `git init`ed and holds no other file is not listed by
  libgit2, so it is not flagged. `git add -A` (Stage all) then fails with git's own "does not
  have a commit checked out", which surfaces as an error toast. Nothing is staged.
- macOS HFS+ ignorable-Unicode `.git` spellings and Windows alternate data streams are not checked
  by `safe_join`. git's own path checks cover checkout.
- Now part of the backlog entry "Pull strategy": `tests/operations.rs` pull
  tests read the developer's global git config through libgit2 (`Repo::pull` checks
  `pull.rebase`). With `pull.rebase=true` in `~/.config/git/config`,
  `pull_merge_creates_a_merge_commit` and `pull_fast_forward_only_refuses_diverged_branches`
  fail locally. CI is unaffected. They pass with an isolated `HOME`/`XDG_CONFIG_HOME`.
- Path checks and the later file access are not atomic: a symlink swapped in between is not
  caught.
