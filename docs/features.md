# Feature matrix

Canopy's status per feature, next to three established clients. **Done** means shipped in v0.1;
**Planned** means it has an entry in [backlog.md](backlog.md). Competitor columns reflect their
current desktop releases to the best of our knowledge: Yes, No, Partial, or ? where we have not
verified it. Correct any cell you can verify, citing the source in the pull request.

Update this table in the same change that adds or alters a feature.

## Platforms

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Linux | Done | No | Yes | No |
| macOS | Done | Yes | Yes | Yes |
| Windows | Done | Yes | Yes | Yes |
| Native (non-Electron) UI | Done | Yes | No | Yes |
| One-line install into its own folder, with uninstall (Linux, macOS, Windows) | Done | No | No | No |
| Builds for Linux arm64 and Intel macOS | Experimental | No | Yes | Yes |
| Network operations through the system `git` (credential helpers, SSH config, hooks) | Done | Yes | ? | Partial |

## Repositories

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Open, init | Done | Yes | Yes | Yes |
| Clone with live progress | Done | Yes | Yes | Yes |
| Several repositories in tabs | Done | Yes | Yes | Yes |
| Welcome screen with recent repositories | Done | Yes | Yes | Yes |
| Auto refresh (file watcher, window focus) | Done | Yes | Yes | Yes |
| Submodules | Planned | Yes | Yes | Yes |
| Worktrees | Planned | ? | Yes | ? |
| Git LFS indicator and commands | Planned | Yes | Yes | Yes |
| Partial clones (`--filter=blob:none`, `--filter=tree:0`): history, commit details and diffs download the missing objects through `git` on demand | Done | Yes | ? | ? |
| Very large repositories | Planned | ? | ? | ? |

Partial clones differ from a full clone in what the views can show without the network. Commit
details list the changed files at once and fetch the line counts afterwards (a commit with more
than 200 files keeps its list without counts). While file contents are not downloaded, a rename
that is not an exact copy shows as a deletion plus an addition, in commit details and in the
staged list of the Changes view, where `git status` would download the blobs to pair them. A
treeless clone after `git reset --soft` downloads the tree of `HEAD` before the Changes view can
list anything, and says why when it cannot. Repositories made by Git before 2.24
(`extensions.partialClone`) are refused with the commands that convert them.

## History

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Commit graph with coloured lanes | Done | Yes | Yes | Yes |
| Ref labels (HEAD, local, remote, tags) | Done | Yes | Yes | Yes |
| Author avatars | Done (initials) | Yes | Yes | Yes |
| Filter by message, author or SHA | Done | Yes | Yes | Yes |
| Commit details: metadata, parents, files with +/- counts, per-file diff | Done | Yes | Yes | Yes |
| Incremental loading ("Load more") | Done | ? | ? | ? |
| Search file contents across history (pickaxe) | Planned | ? | ? | ? |
| File history | Planned | Yes | Yes | Yes |
| Blame | Planned | Yes | Yes | Yes |
| Reflog view | Planned | ? | ? | ? |
| Undo last operation | Planned | ? | Yes | No |
| Graph minimap | Planned | No | ? | No |
| Signature verification badges | Planned | ? | Yes | ? |

## Working copy and committing

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Unstaged, staged and conflicted lists | Done | Yes | Yes | Yes |
| Stage, unstage, discard per file and all | Done | Yes | Yes | Yes |
| Hunk staging, unstaging and discard | Done | Yes | Yes | Yes |
| Line-level staging | Planned | Yes | Yes | Yes |
| Diff with old and new line numbers | Done | Yes | Yes | Yes |
| Side-by-side diff | Planned | Yes | Yes | No |
| Syntax highlighting in diffs | Planned | Yes | Yes | ? |
| Image diff | Planned | Yes | Yes | ? |
| Ignore whitespace, context lines setting | Planned | Yes | Yes | Yes |
| Commit composer with summary counter | Done | Yes | Yes | Partial |
| Amend | Done | Yes | Yes | Yes |
| Commit signing (GPG/SSH) | Planned | Yes | Yes | Yes |
| Commit message templates and linting | Planned | ? | Yes | ? |

## Branches and merging

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Create (from any commit), checkout, rename, delete | Done | Yes | Yes | Yes |
| Checkout remote branch as tracking branch | Done | Yes | Yes | Yes |
| Ahead/behind badges | Done | Yes | Yes | Yes |
| Merge into current | Done | Yes | Yes | Yes |
| Rebase current onto | Done | Yes | Yes | Yes |
| Merge, rebase, cherry-pick in progress: continue and abort | Done | Yes | Yes | Yes |
| Interactive rebase editor | Planned | Yes | Yes | Yes |
| Built-in 3-way conflict resolver | Planned | Yes | Yes | No |
| Drag a branch onto another to merge or rebase | Planned | Yes | Yes | ? |
| git-flow | Planned | Yes | Yes | Yes |
| Bisect | Planned | ? | ? | ? |

## Remotes

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Fetch (all remotes, prune) | Done | Yes | Yes | Yes |
| Pull (fast-forward or rebase per git config) | Done | Yes | Yes | Yes |
| Push, setting upstream when missing | Done | Yes | Yes | Yes |
| Force push with lease | Done | Yes | ? | ? |
| In-app credential and SSH passphrase prompts | Planned | Yes | Yes | Yes |
| Pull request integration (GitHub) | Planned | ? | Yes | ? |

## Commits, tags and stashes

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Checkout commit (detached) | Done | Yes | Yes | Yes |
| Cherry-pick | Done | Yes | Yes | Yes |
| Revert | Done | Yes | Yes | Yes |
| Reset current branch (soft, mixed, hard) | Done | Yes | Yes | Yes |
| Copy SHA or message | Done | Yes | Yes | Yes |
| Tags: lightweight and annotated, delete, push | Done | Yes | Yes | Yes |
| Stash: save (message, untracked), apply, pop, drop | Done | Yes | Yes | Yes |
| Patches: create and apply | Planned | Yes | ? | Yes |

## Interface

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Command palette | Done | Yes | Yes | No |
| Keyboard shortcuts for common operations | Done | Yes | Yes | Yes |
| Configurable keybindings | Planned | ? | ? | ? |
| Dark and light themes | Done | Yes | Yes | ? |
| Custom themes | Planned | No | Yes | No |
| Confirmation for destructive operations | Done | Yes | Yes | Yes |
| Toast notifications with git's error output | Done | ? | ? | ? |
| Localisation | Planned | ? | ? | Yes |
| Screen reader support (AccessKit) | Planned | ? | ? | ? |
