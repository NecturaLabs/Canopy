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

A repository downloads missing objects on its own only when its working folder and its git
directory both lie inside a folder the user trusted in Settings (Privacy, Trusted folders;
`Settings::trusted_folders`). Nothing is trusted by default. Every path is resolved first (symbolic
links, `..`), so a link cannot carry trust into or out of a folder; a folder that cannot be resolved
trusts nothing; a filesystem root is never accepted (`partial::is_in_trusted_folder`).

Elsewhere the offline read still runs and shows what is on disk, a read that needs a download fails
with a message that says why, and the status bar shows "Downloads off · Trust…". Trust… offers the
repository's folder or its parent, never a root. The check runs on the repository thread, so the UI
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
  through the rule the application installs (`partial::set_download_gate`). A download already
  running when trust is withdrawn finishes: it was allowed when it began, and a running git
  cannot be recalled.
- Trust follows where the repository's directories really are, after resolving links. A link
  inside a trusted folder that leads out gives no trust; a link elsewhere that leads in does not
  need any, because the repository and its configuration are inside the folder.
- No gate closes the gap between its check and the new process: a trusted folder withdrawn in
  that instant is the same as one withdrawn a moment after git started. Someone who can rewrite
  the repository's own folders at that moment already has the user's files.
- Only the filesystem root is refused as a trusted folder: by its spelling when it is added (a
  reading of the path, since that runs on the UI thread), and again by core after resolving
  links, so a link to the root can be saved but trusts nothing. The root of a mounted volume
  may be trusted: an external drive of projects is a normal thing to trust whole.
- A linked worktree needs both its own folder and the folder of the repository that holds its
  common git directory trusted; the dialog offers the opened folder and its parent only.

## References

- [git-config: core.sshCommand, credential.helper, url.<base>.insteadOf](https://git-scm.com/docs/git-config)
- [Partial clone](https://git-scm.com/docs/partial-clone)
- [ADR 0001](0001-git-cli-for-mutations-libgit2-for-reads.md): the `git` executable runs network traffic.
