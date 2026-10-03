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

## References

- [git-config: core.sshCommand, credential.helper, url.<base>.insteadOf](https://git-scm.com/docs/git-config)
- [Partial clone](https://git-scm.com/docs/partial-clone)
- [ADR 0001](0001-git-cli-for-mutations-libgit2-for-reads.md): the `git` executable runs network traffic.
