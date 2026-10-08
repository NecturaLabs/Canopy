# ADR 0009: Partial clones download on their own only inside trusted folders

**Status:** accepted
**Date:** 2026-10-03

## Context

Opening a commit or a diff in a partial clone (`--filter=blob:none`, `--filter=tree:0`) can need
objects the clone left on the server. Canopy fetches them through `git fetch` in the repository,
without the user pressing anything (`canopy-core::partial`). Git then obeys the repository's own
configuration: `core.sshCommand`, a credential helper starting with `!`, `url.<base>.insteadOf`
and the remote's own URL can run programs or contact a server the user never chose. A repository
the user downloaded from someone else and merely opened could therefore run code, or send the
user's IP address and credentials prompts to another server, just by being viewed. Fetch, pull and
push stay the user's own action; only the automatic download needed a guard.

## Decision

A repository downloads missing objects on its own only when its working folder, common Git
directory, and worktree-specific Git directory (which may hold `config.worktree`) all lie
inside a folder the user trusted in Settings (Privacy, Trusted folders;
`Settings::trusted_folders`). Nothing is trusted by default. Every path is resolved first (symbolic
links, `..`), so a link cannot carry trust into or out of a folder; a folder that cannot be resolved
trusts nothing; a filesystem root is never accepted (`partial::is_in_trusted_folder`).

Elsewhere the offline read still runs and shows what is on disk, a read that needs a download fails
with a message that says why, and the status bar shows "Downloads off · Trust…". Trust… offers the
repository's folder or its parent, never a root. It also lists any external Git folders the gate
requires; confirming trusts those folders too. The check runs on the repository thread, so the UI
thread does no file I/O for it.

## Alternatives and costs

Asking before every download would interrupt every commit selection. Trusting a repository by its
remote URL does not help, since the configuration that runs code is the repository's, not the
remote's. Running the fetch with git's configuration neutralised (`-c core.sshCommand=`,
`GIT_CONFIG_NOSYSTEM`, a fixed credential helper) would break the SSH and credential setups that
make the download work for the user. The cost of the chosen rule is one decision per folder: a
partial clone opened for the first time shows only what is on disk until the user trusts its folder.

## Limits, decided on purpose

- The list is published to every repository thread the moment it changes (`trust::set`). The
  worker asks before a read goes to the network (to say why nothing downloaded), and core asks
  again at the very point where `git fetch` starts, for every remote and round of one read,
  through the rule the application installs (`partial::set_download_gate`). Resolving paths takes
  time and holds no lock, so the rule asks again when the list changed while it was working: its
  yes always belongs to the list in force when it answers. When a download fails, the worker asks
  once more, and a read that core stopped because trust was withdrawn in between is reported as a
  refusal (with the Trust… chip), not as a failed download. A download already running when trust
  is withdrawn finishes: it was allowed when it began, and a running git cannot be recalled.
- The gate answers for the repository Canopy opened, so git must work in that one: commands
  run in its working directory with the environment's repository variables removed
  (`GIT_DIR`, `GIT_WORK_TREE`, ... in `cli.rs`). Without that, a Canopy started by a git alias
  or hook inside another repository would fetch in that repository while the gate approved the
  tab's. libgit2 reads none of them, so Canopy never meant them to apply.
- A refusal reaches the tab stamped with the version of the list that made it. A tab that has
  since been told of a newer list reads again instead of showing the refusal, so a refusal the
  change overtook never leaves the status bar chip behind, and one the new list made shows at once.
- Paths are compared by their resolved spelling. On a volume that ignores capitalization and
  keeps the one it was given (macOS, possibly), the same folder written another way is not
  recognised: nothing downloads, which is the safe side, and Trust… offers the spelling Canopy
  resolved itself. Comparing without regard to capitalization would trust a different folder on a
  case-sensitive volume. Not tested on macOS or Windows.
- Trust follows where the repository's directories really are, after resolving links. A link
  inside a trusted folder that leads out gives no trust; a link elsewhere that leads in does not
  need any, because the repository and its configuration are inside the folder.
- No gate closes the gap between its check and the new process: a trusted folder withdrawn in
  that instant is the same as one withdrawn a moment after git started. Someone who can rewrite
  the repository's own folders at that moment already has the user's files.
- A folder is saved as an absolute path, whatever way it was offered, and settings never read a
  relative entry back: it would mean another folder from another working directory.
- Only the filesystem root is refused as a trusted folder: by its spelling when it is added (a
  reading of the path, since that runs on the UI thread), and again by core after resolving
  links, so a link to the root can be saved but trusts nothing. The root of a mounted volume
  may be trusted: an external drive of projects is a normal thing to trust whole.
- A linked worktree needs its own folder, common Git directory and private Git directory trusted.
  The worker resolves them before offering the dialog's folders, omitting a directory already
  covered by another choice. A main repository outside the worktree's parent is covered by its
  exact Git directory; the dialog does not silently trust that repository's whole parent.

## References

- [git-config: core.sshCommand, credential.helper, url.<base>.insteadOf](https://git-scm.com/docs/git-config)
- [Partial clone](https://git-scm.com/docs/partial-clone)
- [ADR 0001](0001-git-cli-for-mutations-libgit2-for-reads.md): the `git` executable runs network traffic.
