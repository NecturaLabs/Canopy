# Feature matrix

Canopy's status per feature, next to three established clients. **Done** means shipped in a
release (build 763 or earlier); **Unreleased** means merged into `main` and in the code, but in no release yet (unverified work is tracked in [GitHub Issues](https://github.com/NecturaLabs/Canopy/issues) with the `needs-verification` label); **Planned** means it is tracked in GitHub Issues, and "backend merged" means `canopy-core` has it but no screen uses it yet. Competitor columns reflect their
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
| In-app update: download, SHA-256 check and install in place where the installer put Canopy (Linux), the installer elsewhere, then restart | Done | ? | ? | ? |
| Releases named by build number (`build 412 (a1b2c3d)`), shown in About, `--version`, logs and the update check; no version bumps | Done | ? | ? | ? |
| Builds for Linux arm64 and Intel macOS | Experimental | No | Yes | Yes |
| Network operations through the system `git` (credential helpers, SSH config, hooks) | Done | Yes | ? | Partial |
| Missing programs named when a repository is opened: `git` itself (dialog) and, for the repository that needs them, `git-lfs` (its attributes use LFS), `ssh` (an SSH remote) and the signing program (`gpg`, `gpgsm` or `ssh-keygen`, or the one `gpg.*program` names); one toast each per run, with install steps for the system | Done | ? | ? | ? |

## Repositories

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Open, init | Done | Yes | Yes | Yes |
| Ignore management: file/extension/folder/pattern rules, preview, local/global exclude, ignored paths, why ignored, stop tracking; optional starter templates on init | In progress (#59) | Yes | Yes | Yes |
| Clone with live progress; a clone whose submodules cannot be downloaded is kept and opened with git's message, never deleted | Done | Yes | Yes | Yes |
| Several repositories in tabs; closing or changing a worktree preserves unsent commit messages, conflict edits and request comments for this session | Done | Yes | Yes | Yes |
| Welcome screen with recent repositories | Done | Yes | Yes | Yes |
| Auto refresh (file watcher, window focus) | Done | Yes | Yes | Yes |
| Submodules: a sidebar section with state, open in a tab, initialize/update, sync, add and confirmed remove; a submodule's diff lists the commits between its two pointers; pull, checkout and clone bring submodules along (Settings → Git → Update submodules) | In progress (#60) | Yes | Yes | Yes |
| Worktrees: a sidebar section lists the working folders with their branches (the open one marked) and switches to one in the current tab; the toolbar's Worktree button (and the palette's New worktree…) opens a dialog that adds one on a new or an existing branch in a chosen folder (`git worktree add`; the toolbar's Branch and Worktree tooltips say when to use each), and git's refusal (branch already checked out, folder not empty) shows as the error; stale linked worktrees can be removed through cleanup | Unreleased (#138) | Yes | Yes | ? |
| Stale cleanup: configurable commit-age period (Settings → Git, default 90 days), preview and confirmed removal of linked worktrees and local branches, protecting dirty worktrees and unmerged commits | Implemented (#91); live verification pending | ? | ? | ? |
| Git LFS indicator and commands | Planned (status backend merged) | Yes | Yes | Yes |
| Partial clones (`--filter=blob:none`, `--filter=tree:0`): history, commit details and diffs download the missing objects through `git` on demand | Done | Yes | ? | ? |
| Very large repositories | Planned | ? | ? | ? |
| Repository insights: 52 weekly commit counts, mailmap authors, last commit, object storage and local branch comparisons, unique commits, age and merged/stale hints, including conservative tree-based squash hints; local calculations only | Done | ? | ? | ? |
| Local-only repositories (no remote), with quiet status labels and compact Publish/Add remote actions | Done | ? | ? | ? |

Partial clones differ from a full clone in what the views can show without the network. Commit
details list the changed files at once and fetch the line counts afterwards (a commit with more
than 200 files keeps its list without counts). While file contents are not downloaded, a rename
that is not an exact copy shows as a deletion plus an addition, in commit details and in the
staged list of the Changes view, where `git status` would download the blobs to pair them. A
treeless clone after `git reset --soft` downloads the tree of `HEAD` before the Changes view can
list anything, and says why when it cannot. Repositories made by Git before 2.24
(`extensions.partialClone`) do not open; Canopy offers to convert them (it sets
`remote.<name>.promisor` and removes the old setting) after a confirmation, and the commands stay
in the error text. A diff that could not be downloaded keeps its message with a Retry button and
is not asked for again by the status refreshes; its file row carries a "not downloaded" mark
until a read succeeds.

Downloads run on their own only for repositories inside a folder trusted in Settings (Privacy,
Trusted folders), because git then follows the repository's own configuration, which can run
programs and contact servers. Elsewhere Canopy shows what is on disk, says why a read needs a
download, and the status bar offers "Downloads off · Trust…" with the repository's folder or its
parent. Linked worktrees also list the external Git folders they use; confirmation trusts those
folders too. The chip disappears when its refused details are no longer visible. Nothing is
trusted by default (ADR 0009).

A download of the working tree status that failed (server unreachable) is remembered: the
refreshes the file watcher makes show the same message without trying the network again, until
the user presses Retry or Refresh. A download is killed after ten minutes.

The same list guards auto-fetch. A repository whose own files name a program a fetch would run
(`core.sshCommand`, a credential helper, `remote.<name>.uploadpack`, `core.hooksPath`, a
`reference-transaction` or `pre-auto-gc` hook) is not fetched by the timer outside a trusted
folder, and the failure names what it found; a fetch the user starts runs as in a terminal. Every
git command Canopy runs disables `core.fsmonitor`, which a repository can set to a command that
git would run on every `git status`.

## History

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Commit graph with coloured lanes | Done | Yes | Yes | Yes |
| Ref labels (HEAD, local, remote, tags) | Done | Yes | Yes | Yes |
| Author avatars | Done (GitHub/GitLab pictures, initials fallback) | Yes | Yes | Yes |
| On-demand History search by message, author or SHA (Ctrl+F or `/` toggles it) | Done | Yes | Yes | Yes |
| Commit details: summary, copyable commit id, parent links and committer picture/name in the header; scrolling description and changed-file card, per-file diff, expand/return and compact full view | Done | Yes | Yes | Yes |
| Incremental loading ("Load more") | Done | ? | ? | ? |
| Search file contents across history (pickaxe) | Planned | ? | ? | ? |
| File history: the commits that changed a file, followed back through renames (`git log --follow`), from the Changes file menu, a commit's file menu or the command palette; choosing one opens that commit's diff of the file, leaving the working tree alone; each row keeps its author and date whole (the summary is cut first), a merge commit that lists no change to the file shows a merge glyph instead of a status, and the dialog says when it lists only the commits up to the one it was opened from | Done | Yes | Yes | Yes |
| Blame: a file's lines with the short id, author and date of the commit that last changed each (follows renames); the Blame button in a file's diff header, or "Blame selected file" in the command palette, shows it in place of the diff, as of the commit being viewed or the working tree; clicking a line's commit opens it in History; changes not committed yet are marked | Done | Yes | Yes | Yes |
| Reflog view | Planned | ? | ? | ? |
| Undo last operation | Planned | ? | Yes | No |
| Graph minimap | Planned | No | ? | No |
| Signature verification badges | Done: distinct verified, invalid, unavailable, untrusted and expired/revoked marks in commit details and the author card; only verified signatures use the trusted mark; checked by git in trusted folders only | ? | Yes | ? |
| Commit identity card and parent links in commit details | Done: the committer name opens a card (email, original author, signature, profile or forge search link, picture hint); parents select their commit | ? | Yes |

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
| Clean working tree summary: branch status, Push/Pull/Publish, last commit | Done | ? | ? | ? |
| Stage, unstage, discard per file and all (Stage selected beside Stage all; Ctrl/Shift-click select several files) | Done | Yes | Yes | Yes |
| Untracked folders the status lists as one entry (a folder holding a repository of its own, or any top folder when there are very many untracked files) are listed under "Untracked folders" with a Stage action; staging one that holds a repository asks first, since Git records it as a link nobody can clone, and Stage all leaves such folders out and says so | Done | Yes | ? | ? |
| Publish any local branch (not only the checked-out one) to a remote from its context menu, making the remote branch its upstream | Done | Yes | ? | ? |
| Hunk staging, unstaging and discard | Done | Yes | Yes | Yes |
| Line-level staging | Done | Yes | Yes | Yes |
| Diff with old and new line numbers | Done | Yes | Yes | Yes |
| Side-by-side diff (a new or deleted file stays one column) | Done | Yes | Yes | No |
| Syntax highlighting in diffs | Done | Yes | Yes | ? |
| Image diff | Planned | Yes | Yes | ? |
| Ignore whitespace, context lines setting | Done | Yes | Yes | Yes |
| Commit composer with summary counter; follows the file lists, says why Commit is disabled | Done | Yes | Yes | Partial |
| Unsent commit message kept when its tab closes, until the repository opens again (same session) | Done | ? | ? | ? |
| Amend | Done | Yes | Yes | Yes |
| The commit template (`commit.template`) fills an empty message field once per tab in trusted folders, including after trusting an open tab; a draft or an amend keeps its text | Unreleased | Yes | ? | Yes |
| Commit signing (GPG/SSH) | Planned | Yes | Yes | Yes |
| Commit message checks: a summary or description line longer than 72 characters is reported under the fields (Settings → Git → Commits → Check commit messages, on by default); long URLs, indented and quoted lines are left alone, and a hint never blocks the commit or changes the draft | Unreleased | ? | Yes | ? |

Unstaged diffs refuse actual symbolic links when `core.symlinks=false`, because libgit2 would
otherwise read their targets as regular files. Git's ordinary-file link emulation still works.
Link diffs label their target paths; a file turned into a link (or back) shows the old side removed
and the new side added, as `git diff` does, with the link's target text and never the file it points
to, and its hunks offer no actions (stage or discard the whole file); mode
changes remain visible alongside content changes. A file or link replaced by a submodule (or
back) shows both text sides, including the recorded submodule commit, in the usual unified or
side-by-side diff. Changes between two submodule pointers retain the commit-list card.
Type-change previews omit content when either side exceeds 256 KiB, with an explicit size
notice instead of a binary label or zero-change counts. Commit file counts are independent
of that preview limit.

## Branches and merging

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Create (from any commit), checkout, rename, delete | Done | Yes | Yes | Yes |
| Delete with `git branch -d`, forcing only after Git refuses and you confirm | Done | ? | ? | ? |
| Checkout remote branch as tracking branch | Done | Yes | Yes | Yes |
| Ahead/behind badges | Done | Yes | Yes | Yes |
| Sidebar marks the branch, remote branch, tag, stash or worktree you clicked while History shows its commit (the checked-out branch on opening), apart from the checked-out branch (accent check, semibold) | Done | Yes | Yes | Yes |
| Sidebar rows sit indented under their section header; a new tab opens only Branches (Worktrees, Remotes, Tags, Stashes and Submodules start closed) and keeps the sections you open or close, saved per repository (its worktrees share them) and restored when reopened; Add remote… is in the palette and the local-only card | Done | Yes | ? | ? |
| A branch checked out in another worktree opens it (double-click or Open worktree) instead of a checkout Git would refuse | Done | Yes | Yes | ? |
| Merge into current | Done | Yes | Yes | Yes |
| Rebase current onto | Done | Yes | Yes | Yes |
| Merge, rebase, cherry-pick in progress: continue and abort | Done | Yes | Yes | Yes |
| Interactive rebase editor | Planned | Yes | Yes | Yes |
| Merge modes: fast-forward when possible, no fast-forward, fast-forward only, squash | Done | ? | ? | ? |
| Built-in 3-way conflict resolver | Done | Yes | Yes | No |
| Choices and unfinished edits stay with their conflicted file while another one is open in Changes, and come back when it is picked again | Done | Yes | Yes | No |
| Drag a branch onto another to merge or rebase | Planned | Yes | Yes | ? |
| git-flow | Planned (backend merged) | Yes | Yes | Yes |
| Bisect | Planned | ? | ? | ? |

The git-flow backend finishes a single-commit feature or bugfix by fast-forwarding when possible,
matching AVH. Diverged branches still merge; branches with multiple commits keep a merge commit.
The git-flow UI remains planned.

## Remotes

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Fetch (all remotes, prune) | Done | Yes | Yes | Yes |
| Pull (fast-forward or rebase per git config) | Done | Yes | Yes | Yes |
| Pull checks first: fast-forward only by default, asks Rebase (recommended) or Merge when branches diverged, remembered for the repository or, where Canopy's own setting asked for fast-forwards, for every repository, and names the files before a pull that would conflict | Done | ? | ? | ? |
| Git configuration: editable supported values in Settings → Git (global) and This repository (local), with values, scopes, files and recommendations; Use recommended writes the non-sensitive recommendations globally after confirmation; Unset/Inherit removes only the selected file's override, preserving included configuration and repository choices; canonical Git booleans and configuration errors are displayed accurately: a value Git refuses is shown as written, with the problem, on its own row; where a value is set reads "global: ~/.gitconfig", never Git's raw `file:` origin; a search shows rows only, never the editor's captions. The pull recommendation is one choice, fast-forward only (`pull.ff = only`): `pull.rebase` is not marked either way, and the diverged-pull dialog's "Rebase (recommended)" is the recommended way to combine when branches did diverge | Unreleased | ? | ? | ? |
| Repository and Git settings selects show each value once, mark recommended options in their lists (no "Recommended:" sentence in the help text), and distinguish a pinned value from an inherited one with Overridden and a named reset; Settings → Integrations lists the system's own editor, terminal or file manager once, as the Default entry; context lines and whitespace pin and reset together | Done | ? | ? | ? |
| Settings → This repository → Commit signing states what git will do (signs or not) with the format (OpenPGP, SSH, X.509), the key's id or file name (an inline SSH key is never shown), the program and where `commit.gpgsign` is set, and warns with its fix when signing would fail: the program is not installed, an SSH key is not set or its file is missing, or `gpg.format` is unknown; read from the configuration and `PATH` only, never by running the program; a button opens the file that sets it, preserving non-UTF-8 file names; identity, pull and repository settings share absolute, unquoted configuration origins | Done | ? | ? | ? |
| A pull fetches once: after its check it brings in what it fetched, so a credential prompt comes once | Done | ? | ? | ? |
| A rebase that stops on conflicts takes you to Changes, like a pull | Done | ? | ? | ? |
| A pull that stops on conflicts, or whose set-aside changes conflict when they come back, opens Changes to resolve them | Done | ? | ? | ? |
| Push, setting upstream when missing | Done | Yes | Yes | Yes |
| Force push with lease | Done | Yes | ? | ? |
| The force push confirmation names the branch and the remote branch it overwrites | Unreleased | ? | ? | ? |
| A pull asks again when the remote changed while you decided | Unreleased | ? | ? | ? |
| The status bar says when a branch is not tracking a remote branch of the same name that exists | Unreleased | ? | ? | ? |
| In-app credential and SSH passphrase prompts | Done | Yes | Yes | Yes |
| Pull request integration (GitHub) | Done | ? | Yes | ? |
| Merge request integration (GitLab, including self-hosted) | Done | ? | ? | ? |
| Open a commit, branch or remote on GitHub or GitLab | Done | ? | ? | ? |

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
| Pop and drop act on the stash you picked, by its commit, even if the list changed | Done | ? | ? | ? |
| Patches: Export as patch… in a commit's menu writes the commit, or it and up to 49 commits before it along first parents (How many commits), as numbered .patch files with their authors, skipping merge commits, which git has no patch for; Apply patch… in the palette applies a file or a folder of them as commits, stopping in "Applying patches" for conflicts | Unreleased | Yes | ? | Yes |

Requests supports Ctrl+Shift+A to approve and Ctrl+Shift+M to open the merge confirmation,
with the same state and busy guards as the buttons.

Without a GitHub or GitLab remote, the requests view offers Publish… for a repository with no
remote and Add remote… for one whose remotes are elsewhere.

Request list icons show lifecycle and draft state. A row leads with the request's number and
title (`#154 Title`, `!154` for a merge request), which wraps to a second line before it is cut,
then the author and when it was last updated with the labels at the right, then the first
lines of its description as plain text (Markdown, comments and tags flattened, paragraphs
separated by a middle dot; nothing when it has none). The branches are in the request's header and the row's tooltip. The description comes
with the list response, so a row costs no request of its own. The request list shows each
request's checks (passed, failed, running) and review (approved, changes requested, and on GitHub
review required) without opening it when Canopy has a token for the host; without a token a row
shows them once the request was opened in the session, and until then a faint question mark says
they are not loaded (a request with no checks shows nothing).

The list narrows and orders the 100 most recently updated requests, the ones the list holds,
with no further request: a filter field over number (`154`, `#154`, `!154`; a number matches
as part of a request's number, so `12` also finds 112), title, author and the start of the
description (every word must match; Ctrl+F or `/` in the Requests view puts the caret in it), a
Filters popover for
draft, checks (passed, failed, running), review (approved, changes requested, required) and
author, and a Sort menu (recently updated, newest, oldest, number, title, author). A line under
the field counts what is shown and clears the filters; a list with nothing left says so and
offers the same. The filters and the sort are kept per repository (its worktrees share them),
across restarts, and stay out of settings export and import. Up and Down, j and k move through
the list in the order shown.

The list starts at 45% of the pane (320 to 560 points) and follows the window until its width is
dragged or set with the arrow keys; it can be dragged to 60% of the pane or 800 points, and
double-clicking the handle makes it follow the window again.

A request's header actions are one group of buttons of the same size: the primary for its state
(Check out while it is open, Open on GitHub or GitLab otherwise), then the other, Copy link and
More. Nothing says where the token came from while it works (Settings → Hosting does); a missing
token, or one the host refused (expired, revoked or short of scope), shows as a warning under the
actions with Add a token… or Replace the token…, which opens Settings.

A request's details start with its summary line (commits, files changed, lines, conflict
state) with the lines in the added and removed colours, as the commit and file diffs show them.

The request list stays current without pressing Reload: a fetch (manual or the background one),
pull or push that succeeds refreshes it, and the open request, quietly (no spinner or toast; the
list, selection and open request stay on screen, and a failed refresh keeps them), opening
the Requests view reloads a list older than a minute, and while the view stays open and the window
is focused the list refreshes on a timer: at the background fetch's interval when that is on,
every minute when it is off, doubling after a failed refresh (up to fifteen minutes) and back to
normal once one succeeds. The timer needs a hosting token: without one the requests would spend
the host's small anonymous rate limit, which Reload, actions and author pictures share, so
without a token the list refreshes only on Reload, when the Requests view opens on a stale list
and after a fetch, pull or push. The timer pauses while another view is shown or the window is
unfocused, and while a fetch, pull or push runs (that one refreshes the list when it ends); a
Settings page or a dialog over the Requests view does not pause it. A rate limit holds it until
the host says the limit lifts (an hour at most), else for fifteen minutes; opening the Requests
view and a refresh after a Git sync wait out the same backoff. These use the same list and
details calls as Reload, and only
with hosting on and a list already loaded.

Going back to a request opened within the last minute (A, then B, then A again) shows its details
at once from a small cache of the last 16 loads, without asking the host again. The cache is keyed
by forge host, project and request number, so equal numbers in different projects never share an
entry. A Reload, an automatic refresh of the list, the result of an action (merge, close,
approve, comment and so on) and a change of token empty it, and a failed load is never kept.
Unsent comments are kept apart from it.

Hosting error messages hide the client token after normalizing invisible characters. Profile
lookup errors also hide private request paths and email addresses after that normalization.
A response with the wrong JSON shape reports its failure location without quoting the returned values.

## Interface

| Feature | Canopy v0.1 | Fork | GitKraken | Sourcetree |
|---|---|---|---|---|
| Command palette | Done | Yes | Yes | No |
| Keyboard shortcuts for common operations | Done | Yes | Yes | Yes |
| Keyboard cheatsheet and vim-style single-key actions | Done | ? | ? | ? |
| Settings screen (appearance, Git, integrations, accounts, updates, privacy, keyboard; relative/calendar History dates with English or ISO calendar format) | Done | ? | ? | ? |
| Compact layouts and 80–160% interface zoom | Done | ? | ? | ? |
| Shared raised controls, larger input targets and visible keyboard focus | Done | ? | ? | ? |
| Keyboard-only use: one Tab stop per list with the arrows inside, Shift+F10 context menus, modal palette, cheatsheet and dialogs that give focus back | Done | ? | ? | ? |
| Native folder chooser without blocking the app | Done | ? | ? | ? |
| Configurable keybindings: Settings → Keyboard changes any row's key (a key another row uses is refused, naming it), resets one or all, and keeps them in settings export and import | Done | ? | ? | ? |
| Dark and light themes (Midnight and Daylight) | Done | Yes | Yes | ? |
| Observatory theme (the previous Midnight) | Done | ? | ? | ? |
| High contrast switch for every theme (keeps the theme's backgrounds and colours, raises text, outlines and focus) | Done | ? | ? | ? |
| History graph options: commit marks (dots, rings, squares, diamonds) and lines (curves, metro, circuit) | Done | ? | ? | ? |
| Appearance options: Style (Standard, Deeper), Density (Comfortable, Dense), Motion (match system, reduced, full); Translucent surfaces (off by default; menus, select lists, popovers, dialogs, toasts, the command palette and the cheatsheet) | Done | ? | ? | ? |
| Command palette entries for Style, Density, Translucent surfaces, High contrast and Motion; High contrast also in the toolbar's theme menu | Done | ? | ? | ? |
| Custom theme: Midnight or Daylight recoloured from an accent (swatches, hue, intensity) and a tint (swatches, hue, strength); text and lines keep their contrast; works with High contrast | Done | No | Yes | No |
| Confirmation for destructive operations | Done | Yes | Yes | Yes |
| Toast notifications with git's error output and a viewport-bounded overflow stack | Done | ? | ? | ? |
| Notification cards that stay as long as their words take to read (5–10 s, errors 8–15 s, decisions 15 s then the center), paused while pointed at, focused or the window is away; decisions as two named buttons, no close button | Done | ? | ? | ? |
| Notification settings (Settings → Notifications): cards on or off, and per type — errors, needs your attention, updates, finished operations, confirmations; by default only the first three show as cards | Done | ? | ? | ? |
| Tooltip settings (Appearance → Tooltips): Helpful (default; only what the screen does not show), All, or Off (only why a control is unavailable), and a Quick, Normal or Slow delay | Done | ? | ? | ? |
| Links: a web link shows its address and offers Copy link; links in text are underlined; a cut-off field value shows whole in a tooltip | Unreleased | ? | ? | ? |
| A settings file cannot turn on tag pruning, automatic fetching or beta builds, fetch more often, stop following your git configuration or choose where a push goes | Unreleased | ? | ? | ? |
| Notification center: the status bar's bell lists the last 100 notifications, whatever shows as a card, with an unread count for those that need you; copy for failures, clear all, and the decisions a notification asks for (pull, accounts, git settings, review or restart into an update); entries, dismissals and unread counts kept across restarts, each row names its repository and dismisses on its own; the same notice again straight after itself from one repository (a fetch that found nothing, ten times) is one row with a count and the latest time; a repository whose git configuration decides against the recommendation is listed once | Done | ? | Yes | ? |
| Localisation | Planned | ? | ? | Yes |
| Update check, which can be turned off (nothing is sent before you accept the Terms) | Done | ? | ? | ? |
| Opt-in, scrubbed crash reports | Done | ? | ? | ? |
| Screen reader basics (AccessKit): window named for the repository, diff lines and conflict code read, unavailable buttons say why, no icon glyphs read, Settings scrolls to the focused control | Done | ? | ? | ? |
| Full screen reader support (AccessKit) | Planned | ? | ? | ? |

Author pictures are fetched after consent when enabled in Privacy, through the repository hosting worker. GitLab-provided Gravatar and Libravatar image URLs need no API keys. Pictures, and authors without one, are cached locally for 12 hours across restarts; failed refreshes retain the previous picture. A private repository without a token, an unpushed commit or a lost connection no longer hides pictures, and GitLab authors without a picture keep their initials instead of a generated pattern. Lookups take at most half of GitHub's anonymous request limit, and cache files unused for 30 days are deleted.

Request review and merge shortcuts use the selected request's refreshed details when available,
matching the state displayed by the action buttons. Merge controls wrap within narrow panes.

A clean Changes view uses one empty state with the branch’s sync status and Push, Pull or Fetch
as the next action (Publish for a local-only repository). Amend last commit is a secondary link
that opens the composer with the last message; an existing draft is preserved.
