# Feature matrix

Canopy's status per feature, next to three established clients. **Done** means shipped in v0.1;
**Unreleased** means merged into `main` and in the code, but in no release yet (the backlog's
"Implemented, awaiting in-app verification" list says which of these the owner has not yet
confirmed in the running app); **Planned** means it has an entry in [backlog.md](backlog.md),
and "backend merged" means `canopy-core` has it but no screen uses it yet. Competitor columns reflect their
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
| One-line install into its own folder, with uninstall (Linux, macOS, Windows) | Unreleased | No | No | No |
| In-app update: download, SHA-256 check and install in place where the installer put Canopy (Linux), the installer elsewhere, then restart | Unreleased | ? | ? | ? |
| Releases named by build number (`build 412 (a1b2c3d)`), shown in About, `--version`, logs and the update check; no version bumps | Unreleased | ? | ? | ? |
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
| Submodules | Planned (backend merged) | Yes | Yes | Yes |
| Worktrees: a sidebar section lists the working folders with their branches (the open one marked) and opens one in a tab; creating and removing them is planned | Unreleased | Yes | Yes | ? |
| Git LFS indicator and commands | Planned (status backend merged) | Yes | Yes | Yes |
| Partial clones (`--filter=blob:none`, `--filter=tree:0`): history, commit details and diffs download the missing objects through `git` on demand | Unreleased | Yes | ? | ? |
| Very large repositories | Planned | ? | ? | ? |
| Local-only repositories (no remote), with Publish to add one and push | Unreleased | ? | ? | ? |

Partial clones differ from a full clone in what the views can show without the network. Commit
details list the changed files at once and fetch the line counts afterwards (a commit with more
than 200 files keeps its list without counts). While file contents are not downloaded, a rename
that is not an exact copy shows as a deletion plus an addition, in commit details and in the
staged list of the Changes view, where `git status` would download the blobs to pair them. A
treeless clone after `git reset --soft` downloads the tree of `HEAD` before the Changes view can
list anything, and says why when it cannot. Repositories made by Git before 2.24
(`extensions.partialClone`) are refused with the commands that convert them.

Downloads run on their own only for repositories inside a folder trusted in Settings (Privacy,
Trusted folders), because git then follows the repository's own configuration, which can run
programs and contact servers. Elsewhere Canopy shows what is on disk, says why a read needs a
download, and the status bar offers "Downloads off · Trust…" with the repository's folder or its
parent. Nothing is trusted by default (ADR 0009).

## History

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Commit graph with coloured lanes | Done | Yes | Yes | Yes |
| Ref labels (HEAD, local, remote, tags) | Done | Yes | Yes | Yes |
| Author avatars | Done (GitHub/GitLab pictures, initials fallback) | Yes | Yes | Yes |
| On-demand History search by message, author or SHA (Ctrl+F or `/` toggles it) | Done | Yes | Yes | Yes |
| Commit details: metadata, parents, files with +/- counts, per-file diff; persistent information disclosure, expand/return header and compact full view | Done | Yes | Yes | Yes |
| Incremental loading ("Load more") | Done | ? | ? | ? |
| Search file contents across history (pickaxe) | Planned | ? | ? | ? |
| File history | Planned | Yes | Yes | Yes |
| Blame | Planned | Yes | Yes | Yes |
| Reflog view | Planned | ? | ? | ? |
| Undo last operation | Planned | ? | Yes | No |
| Graph minimap | Planned | No | ? | No |
| Signature verification badges | Planned | ? | Yes | ? |

Operation state and guarded Continue/Abort actions stay visible in History, Changes and Requests.
The merge composer names the conclusion and prefills Git's merge message without replacing a draft.

Recovery currently uses explicit actions: History → Revert creates an inverse commit, Reset
moves a branch with the chosen working-tree effect, and the conflict resolver can undo a choice
while it is open. There is no universal Undo button: discarded unstaged or untracked content is
not recoverable from Git's reflog. Operation-specific undo and the reflog view remain planned.

## Working copy and committing

Git identity can be inspected and changed in Settings → Git: the name and email Git would use for
the next commit, where each comes from (environment variable, repository or global configuration, or
Git's own guess), and a save or clear (after a confirmation) for this repository or for every
repository. Canopy writes only `user.name` and `user.email`, never signs in as someone else and
never invents values. The Changes view warns, with a link to Settings, when Git has no usable
identity, guessed it, or the values look like placeholders; Git's own error still appears if you
commit anyway. After the consent dialogs, first launch asks once for a global identity when none
is configured. History's "Author" column shows what each commit recorded, which does not
change when current settings change; Canopy never rewrites existing commits to fix an author.

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Unstaged, staged and conflicted lists (empty ones hidden while another has content) | Done | Yes | Yes | Yes |
| Clean working tree summary: branch status, Push/Pull/Publish, last commit | Unreleased | ? | ? | ? |
| Stage, unstage, discard per file and all | Done | Yes | Yes | Yes |
| Hunk staging, unstaging and discard | Done | Yes | Yes | Yes |
| Line-level staging | Unreleased | Yes | Yes | Yes |
| Diff with old and new line numbers | Done | Yes | Yes | Yes |
| Side-by-side diff (a new or deleted file stays one column) | Unreleased | Yes | Yes | No |
| Syntax highlighting in diffs | Unreleased | Yes | Yes | ? |
| Image diff | Planned | Yes | Yes | ? |
| Ignore whitespace, context lines setting | Unreleased | Yes | Yes | Yes |
| Commit composer with summary counter; follows the file lists, says why Commit is disabled | Done | Yes | Yes | Partial |
| Unsent commit message kept when its tab closes, until the repository opens again (same session) | Unreleased | ? | ? | ? |
| Amend | Done | Yes | Yes | Yes |
| Commit signing (GPG/SSH) | Planned | Yes | Yes | Yes |
| Commit message templates and linting | Planned | ? | Yes | ? |

## Branches and merging

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Create (from any commit), checkout, rename, delete | Done | Yes | Yes | Yes |
| Delete with `git branch -d`, forcing only after Git refuses and you confirm | Unreleased | ? | ? | ? |
| Checkout remote branch as tracking branch | Done | Yes | Yes | Yes |
| Ahead/behind badges | Done | Yes | Yes | Yes |
| Sidebar marks the branch, remote branch, tag, stash or worktree you clicked while History shows its commit (the checked-out branch on opening), apart from the checked-out branch (accent check, semibold) | Unreleased | Yes | Yes | Yes |
| A branch checked out in another worktree opens it (double-click or Open worktree) instead of a checkout Git would refuse | Unreleased | Yes | Yes | ? |
| Merge into current | Done | Yes | Yes | Yes |
| Rebase current onto | Done | Yes | Yes | Yes |
| Merge, rebase, cherry-pick in progress: continue and abort | Done | Yes | Yes | Yes |
| Interactive rebase editor | Planned | Yes | Yes | Yes |
| Merge modes: fast-forward when possible, no fast-forward, fast-forward only, squash | Unreleased | ? | ? | ? |
| Built-in 3-way conflict resolver | Unreleased | Yes | Yes | No |
| Drag a branch onto another to merge or rebase | Planned | Yes | Yes | ? |
| git-flow | Planned (backend merged) | Yes | Yes | Yes |
| Bisect | Planned | ? | ? | ? |

## Remotes

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Fetch (all remotes, prune) | Done | Yes | Yes | Yes |
| Pull (fast-forward or rebase per git config) | Done | Yes | Yes | Yes |
| Pull checks first: fast-forward only by default, asks Rebase or Merge (remembered per repository if wanted) when branches diverged, and names the files before a pull that would conflict | Unreleased | ? | ? | ? |
| Recommended git defaults passed explicitly (autostash, push tags with their commits, `push.default=simple`, zdiff3 conflicts, `main` for new repositories, no tag pruning, `#` lines kept in commit messages), with Settings saying where your git configuration differs | Unreleased | ? | ? | ? |
| A pull that stops on conflicts, or whose set-aside changes conflict when they come back, opens Changes to resolve them | Unreleased | ? | ? | ? |
| Push, setting upstream when missing | Done | Yes | Yes | Yes |
| Force push with lease | Done | Yes | ? | ? |
| In-app credential and SSH passphrase prompts | Unreleased | Yes | Yes | Yes |
| Pull request integration (GitHub) | Unreleased | ? | Yes | ? |
| Merge request integration (GitLab, including self-hosted) | Unreleased | ? | ? | ? |
| Open a commit, branch or remote on GitHub or GitLab | Unreleased | ? | ? | ? |

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
| Pop and drop act on the stash you picked, by its commit, even if the list changed | Unreleased | ? | ? | ? |
| Patches: create and apply | Planned | Yes | ? | Yes |

Requests supports Ctrl+Shift+A to approve and Ctrl+Shift+M to open the merge confirmation,
with the same state and busy guards as the buttons.

Without a GitHub or GitLab remote, the requests view offers Publish… for a repository with no
remote and Add remote… for one whose remotes are elsewhere.

Request list icons show lifecycle and draft state. Branches have a separate line with room
reserved for the target. The request list shows each request's checks (passed, failed, running)
and review (approved, changes requested, and on GitHub review required) without opening it when
Canopy has a token for the host; without a token a row shows them once the request was opened in
the session.

The request list stays current without pressing Reload: a fetch (manual or the background one),
pull or push that succeeds refreshes it, and the open request, quietly (no spinner or toast; the
list, selection and open request stay on screen, and a failed refresh keeps them), and opening
the Requests view reloads a list older than a minute. These use the same list and details calls
as Reload, and only with hosting on and a list already loaded.

## Interface

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Command palette | Done | Yes | Yes | No |
| Keyboard shortcuts for common operations | Done | Yes | Yes | Yes |
| Keyboard cheatsheet and vim-style single-key actions | Unreleased | ? | ? | ? |
| Settings screen (appearance, Git, integrations, accounts, updates, privacy, keyboard; relative/calendar History dates with English or ISO calendar format) | Unreleased | ? | ? | ? |
| Compact layouts and 80–160% interface zoom | Unreleased | ? | ? | ? |
| Shared raised controls, larger input targets and visible keyboard focus | Unreleased | ? | ? | ? |
| Keyboard-only use: one Tab stop per list with the arrows inside, Shift+F10 context menus, modal palette, cheatsheet and dialogs that give focus back | Unreleased | ? | ? | ? |
| Native folder chooser without blocking the app | Unreleased | ? | ? | ? |
| Configurable keybindings | Planned | ? | ? | ? |
| Dark and light themes (Midnight and Daylight) | Done | Yes | Yes | ? |
| Observatory theme (the previous Midnight) | Unreleased | ? | ? | ? |
| High contrast switch for every theme (keeps the theme's backgrounds and colours, raises text, outlines and focus) | Unreleased | ? | ? | ? |
| History graph options: commit marks (dots, rings, squares, diamonds) and lines (curves, metro, circuit) | Unreleased | ? | ? | ? |
| Appearance options: Style (Standard, Deeper), Density (Comfortable, Dense), Motion (match system, reduced, full); Translucent surfaces (off by default; menus, select lists, popovers, dialogs, toasts, the command palette and the cheatsheet) | Unreleased | ? | ? | ? |
| Command palette entries for Style, Density, Translucent surfaces, High contrast and Motion; High contrast also in the toolbar's theme menu | Unreleased | ? | ? | ? |
| Custom theme: Midnight or Daylight recoloured from an accent (swatches, hue, intensity) and a tint (swatches, hue, strength); text and lines keep their contrast; works with High contrast | Unreleased | No | Yes | No |
| Confirmation for destructive operations | Done | Yes | Yes | Yes |
| Toast notifications with git's error output and a viewport-bounded overflow stack | Done | ? | ? | ? |
| Notification cards that stay as long as their words take to read (5–10 s, errors 8–15 s, decisions 15 s then the center), paused while pointed at, focused or the window is away; decisions as two named buttons, no close button | Unreleased | ? | ? | ? |
| Notification settings (Appearance → Notifications): cards on or off, and per type — errors, needs your attention, updates, finished operations, confirmations; by default only the first three show as cards | Unreleased | ? | ? | ? |
| Tooltip settings (Appearance → Tooltips): Helpful (default; only what the screen does not show), All, or Off (only why a control is unavailable), and a Quick, Normal or Slow delay | Unreleased | ? | ? | ? |
| Notification center: the status bar's bell lists the last 100 notifications, whatever shows as a card, with an unread count for those that need you; copy for failures, clear all, and the decisions a notification asks for (pull, accounts, review or restart into an update) | Unreleased | ? | Yes | ? |
| Localisation | Planned | ? | ? | Yes |
| Update check, which can be turned off (nothing is sent before you accept the Terms) | Unreleased | ? | ? | ? |
| Opt-in, scrubbed crash reports | Unreleased | ? | ? | ? |
| Screen reader basics (AccessKit): window named for the repository, diff lines and conflict code read, unavailable buttons say why, no icon glyphs read, Settings scrolls to the focused control | Unreleased | ? | ? | ? |
| Full screen reader support (AccessKit) | Planned | ? | ? | ? |

Author pictures are fetched after consent when enabled in Privacy, through the repository hosting worker. GitLab-provided Gravatar and Libravatar image URLs need no API keys. Pictures, and authors without one, are cached locally for 12 hours across restarts; failed refreshes retain the previous picture. A private repository without a token, an unpushed commit or a lost connection no longer hides pictures, and GitLab authors without a picture keep their initials instead of a generated pattern. Lookups take at most half of GitHub's anonymous request limit, and cache files unused for 30 days are deleted.

Request review and merge shortcuts use the selected request's refreshed details when available,
matching the state displayed by the action buttons. Merge controls wrap within narrow panes.
