---
name: fixture-repo
description: Create and use deterministic demo Git repositories for Canopy. Use whenever you need a repository to run Canopy against, to reproduce a bug, to test a destructive operation (discard, reset, delete, force push, rebase), to take screenshots, or to check performance on a large history. Never use the user's real repositories for these.
---

# Fixture repositories

`tools/mkfixture.sh` builds repositories with plain `git`, fixed author and committer identities
and dates, and the user's global and system git config ignored. The same kind always produces the
same commit IDs, so screenshots and bug reports are reproducible.

## Rules

- Never run Canopy's destructive operations, or destructive git commands, against the user's real
  repositories or this repository. Use a fixture in a temporary or scratch directory.
- In Rust tests, do not call the script: create the repository in the test with `tempfile` and a
  fixed identity, so tests are hermetic and run on Windows too. Use fixtures for manual checks,
  screenshots and performance work.
- Recreate a fixture rather than repairing one; `--force` replaces a directory only if this script
  created it.

## Kinds

| Kind | Contents | Use for |
|---|---|---|
| `showcase` (default) | ~40 commits, five authors, merged/fast-forwarded/open branches, `release/1.0`, tags `v0.1.0` `v0.2.0` `v1.0.0`, bare remote `<dir>.remote.git` as `origin` (main ahead 2, `feature/dark-mode` behind 1), a stash, a dirty tree (two-hunk modification, staged change, untracked file, deleted file) | Screenshots, most manual testing |
| `conflict` | Stopped mid-merge, `src/retry.rs` conflicted, `README.md` merged | Conflict lists, in-progress banner, continue/abort |
| `linear` | 100 linear commits | Scrolling, selection, simple graph |
| `big` | `BIG_COMMITS` (default 5000) mainline commits, a topic branch merged every 50, tags every 500 | Graph layout and scrolling performance |

## Usage

```sh
tools/mkfixture.sh /tmp/canopy-showcase
tools/mkfixture.sh --force /tmp/canopy-conflict conflict
BIG_COMMITS=20000 tools/mkfixture.sh /tmp/canopy-big big
cargo run -p canopy-app -- /tmp/canopy-showcase
```

Pushes from the showcase fixture go to `<dir>.remote.git`, a local bare repository, so fetch,
pull and push can be exercised without the network. To test against a remote that has moved on,
clone `<dir>.remote.git` elsewhere, commit, and push from that clone.

Requires `git` 2.32 or newer. Delete fixtures when done.

## Extending

Add a new kind as a `make_<kind>` function in `tools/mkfixture.sh`, using the `by`, `commit`,
`merge`, `tag`, `put`, `append` and `rewrite` helpers so it stays deterministic and portable (no
`sed -i`, no reliance on the user's config). Document it in `tools/README.md` and in the table
above.
