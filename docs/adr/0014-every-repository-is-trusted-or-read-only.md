# ADR 0014: Every repository Canopy opens is trusted or read-only

**Status:** accepted; amends [ADR 0009](0009-partial-clone-downloads-only-in-trusted-folders.md)
**Date:** 2026-10-14

## Context

ADR 0009 asked for trust only before a partial clone downloaded. A repository's own hooks,
configuration and filters can run programs on this computer whenever Canopy runs `git` in it
(#28), and nothing asked about that when a repository was opened. The owner decided on 2026-10-13
(#228): an untrusted repository is read-only and starts no programs; Don't trust opens it
read-only with a banner and a Trust button; and trust safety can be turned off in Settings.

## Decision

- **Every repository Canopy opens is trusted or read-only.** A repository is trusted when its
  working folder, common Git directory and the worktree's private Git directory lie inside a
  folder in `Settings::trusted_folders`, at any depth. Nothing is trusted by default. A repository
  outside them opens with a prompt (`Dialog::Trust`): Trust adds its folders (plus the Git
  folders of a linked worktree that lie elsewhere) to the list; Don't trust leaves it open and
  read-only, with a banner above History, Changes and the request list whose Trust… button asks
  again. The partial-clone prompt of ADR 0009 is the same prompt; its separate parent-folder
  choice is gone (a parent is added in Settings → Privacy).
- **Read-only means the repository thread refuses.** Every `Job::Op` (stage, commit, checkout,
  fetch, pull, push, stash, worktree, submodule, ...) asks the trusted list when it starts and, in
  a repository that is not trusted, ends in `READ_ONLY` without starting `git`: the toast says why.
  Background fetches are not sent to it. Reading history and diffs (libgit2) stays allowed.
  Downloads, signature checks and the commit template keep ADR 0009's gate. The refusal is the
  thread's, whatever the interface shows, so the prompt is a convenience and no race between the
  prompt and the first job can start a program.
- **The list keeps only outermost folders.** Adding a folder drops the entries below it and adding
  a covered folder changes nothing. This reads the spellings, without touching the disk, because
  it runs on the UI thread; a spelling it cannot judge (one with `..`) is kept as given, and a
  redundant entry is harmless because core compares what the folders are.
- **Folders are compared by identity, not spelling (amends ADR 0009's resolved-spelling rule).**
  A path is inside a trusted folder when one of its resolved ancestors is that same folder on disk
  (`same_file::Handle`: device and file number, or the volume's file index). The owner's text
  said to compare case-insensitively on macOS and Windows volumes, and #228 recommended probing the
  volume and folding case. Identity gives the same answer where the volume ignores capitalization
  and the opposite where it does not, with no probe to fail and no folding rule to get wrong
  (Unicode, normalisation). A folder that cannot be opened trusts nothing, as before; a filesystem
  root is still never accepted.
- **Trust safety can be turned off** (`Settings::trust_safety`, on by default). Turning it off asks
  first, explaining the danger; while off no repository is prompted about, refused or gated (every
  place that reads the list reads the switch with it, so they cannot disagree), Settings shows a
  notice on every page, and the trusted list is kept. A settings file never sets the list or the
  switch (`settings_file::NOT_PREFERENCES`).
- **Opened without a user action** (tabs restored at start, a path on the command line, a clone
  Canopy just made): the prompt is for the active tab only; the other tabs open read-only with the
  banner and ask once when first active. A clone into a trusted folder needs no prompt; one
  elsewhere asks on its first open. A folder that is not a repository is never asked about; the
  prompt belongs to the repository that is opened inside it.

## Alternatives and costs

Asking only when the user first acts (commit, stage) would let a hook run before the question is
understood, and a tab would still need a state for "asked". Refusing in the interface alone (greying
buttons) would leave every other route open. Blocking reads too would make an untrusted repository
unusable for the very thing it is opened for. The cost of the chosen rule is one decision per
folder: a repository the user did not place in a trusted folder is read-only until trusted, and
anyone who trusts a large folder trusts everything that is later put in it.

## Limits, decided on purpose

- The trusted check happens once per job on the repository thread, so a job already running when
  trust is withdrawn finishes, as in ADR 0009; the next one is refused.
- Reads that run `git` (blame, file history, the change search, repository insights, the ignore
  checks) are not refused here. They pass `--no-textconv`/`--no-ext-diff` where git could run a
  program for a diff, and `core.fsmonitor` is off for every call; #28 audits each program source
  with a control fixture. Writing the repository's own `git config` (the identity of a repository,
  say) is not an operation on the repository's content and is not refused.
- Case-insensitive volumes are compared by identity, which relies on the operating system giving
  both spellings the same file number. Tested on Linux with links and aliases; not tested on a
  case-insensitive volume of macOS or Windows (#37 owns that verification).
- Screenshot mode trusts the repositories it opens unless the overlay is about trust
  (`tools/screenshot.sh -h`).

## References

- [ADR 0009](0009-partial-clone-downloads-only-in-trusted-folders.md): the download gate this
  extends.
- #228 (this decision), #28 (program sources), #37 (linked worktrees, platform volumes).
