---
name: add-git-operation
description: End-to-end recipe for adding a Git operation to Canopy (for example rename a remote, delete a remote branch, stash a single file, squash commits), from the canopy-core function and its test through the worker Job, UI entry points, confirmation, toasts, docs and screenshot verification. Use whenever a backlog entry or request adds or changes something Canopy does to a repository.
---

# Add a Git operation

Work through the steps in order; each one has a done condition. For subtle semantics, delegate the
core part to the `canopy-git` agent.

## 1. Core function (`crates/canopy-core`)

- Every mutation runs the `git` executable (ADR 0001; the conflict resolver's save of a
  hand-edited file is the one plain write): add a function to `ops.rs` that calls
  `cli::run`, `cli::run_with_input` (patches, messages on stdin) or `cli::run_streaming` (long
  network jobs; streams `--progress` lines). Pass arguments as separate argv entries with `--`
  before user-supplied paths and refs. Use `git2` only to read state the function needs, such as
  whether a local branch exists. Never write through libgit2.
- Check the behaviour with the real CLI in a scratch repository first and note the resulting refs,
  index and working tree.
- Write a function on `Repo` (or in the module) that takes explicit parameters (no hidden force)
  and returns `canopy_core::Result<T>`. CLI failures keep git's stderr.
- Add plain data to `model.rs` if the UI needs new information, and include it in `snapshot()`
  only if it is cheap.

Done when: `cargo test -p canopy-core` passes with a new test that creates a temporary repository
(`tempfile`, fixed identity), runs the operation and asserts on the resulting state, plus tests
for the edge cases that apply (unborn branch, detached HEAD, conflicts, nothing to do).

## 2. Worker (`crates/canopy-app/src/worker.rs`)

- A mutation is an `Op` variant carrying owned parameters (`Job::Op(Op)` runs it on the worker).
  Add the variant, its `label()` and `announce()` arms, and its arm in `perform`, which calls the
  core function; every `Op` is followed by a fresh snapshot. A read that is not a mutation is a
  new `Job` variant instead.
- The result comes back as an `Event` (`OpStarted`, `Progress`, `OpFinished`) that carries success
  text for the toast or the error. Long CLI jobs send progress lines as they arrive.

Done when: the UI thread only sends the job; nothing in `views/` or `app/` calls git.

## 3. UI entry points

- Where users expect it: commit or branch context menu, sidebar item menu, toolbar button for
  frequent operations, and always a command palette entry (`palette_entries` in `app/palette.rs`; the
  palette widget is `ui/palette.rs`).
- A shortcut only if the operation is frequent: add it to the table in
  `crates/canopy-app/src/keymap.rs` (`add-keybinding` skill); the cheatsheet and palette hints
  follow from it.
- Use `egui-phosphor` icons and `ui/theme.rs` tokens; reuse `ui/widgets.rs` components.
- Disable or hide the action when it cannot apply (for example, no upstream, detached HEAD),
  with a tooltip saying why.

## 4. Confirmation

Destructive operations (anything that can lose commits, uncommitted work, refs or stashes, or
rewrites a remote) open a confirmation dialog in `views/dialogs/mod.rs` before the job is sent. The
confirm button uses the danger style and names the action ("Delete branch feature/x"). Force push
always uses `--force-with-lease`.

## 5. Feedback

Success: a short toast naming the result ("Created tag v1.2.0"). Failure: an error toast with
git's stderr or the libgit2 message. While running, the status bar shows the job with a spinner.

## 6. Docs

- `docs/features.md`: add or update the row.
- `CHANGELOG.md`: an entry under `[Unreleased]`.
- `docs/architecture.md` if a module's responsibility changed; `docs/design.md` if a new
  component or token was added.

## 7. Verify and tick

1. `tools/check.sh` passes.
2. Run the operation on a fixture (`fixture-repo` skill) in the running app and compare the
   resulting state with the CLI (`git log --graph --all --oneline`, `git status`).
3. Take screenshots of the entry point and result (`screenshot` skill), dark and light.
4. Tick the backlog entry in `docs/backlog.md` only after steps 1 to 3.
5. Get an independent review: the `independent-review` skill, with the `canopy-review` agent as
   the reviewer.
