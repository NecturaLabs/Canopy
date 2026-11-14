# Changelog

All notable changes to Canopy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Releases up to 0.2.1-beta.1 used
[Semantic Versioning](https://semver.org/spec/v2.0.0.html); from the first numbered build on, a
release is a build named by its number (see Unreleased), and its notes are generated from the
commits since the previous build.

## [Unreleased]

### Added

- Tabs that are open but not shown load only when you first show them. Restoring a session, or
  opening several repositories from the command line, loads the tab you see at once and lists the
  others by name in the tab strip; each one reads its repository, starts watching its files and
  joins background fetching the first time you switch to it. Before, every restored tab started
  its own threads and read its repository at launch, so many tabs made start-up slow and heavy.
  A tab not yet shown has no ahead/behind counts in the strip (its tooltip says it is not loaded
  yet), and the notices that come with opening a repository (Git settings that differ from the
  recommendation, leftover files from an interrupted save) appear when you first show the tab.
- Settings → Integrations lists the well-known editors, terminals and file managers that are not
  installed after the installed ones, dimmed and tagged Not installed, so you see what exists; they
  cannot be picked, and the keyboard passes over them (Linux and macOS; Windows lists the installed
  programs only). A program you chose that has since been uninstalled shows the same way (also when its
  saved command is a path to a file that is gone; a path to a file that is there runs as typed),
  and opening falls back to your system's program and says so. A well-known program found only on `PATH`
  is now listed as installed (Linux and macOS).
- Git LFS is visible and usable. Files stored with LFS carry an `LFS` tag in the Changes lists and
  commit details, and a file that holds only its pointer says "not downloaded", with one entry in
  the status bar that downloads the missing files. The diff of an LFS file is a card with each
  side's size and object id, old to new (and the pointer text on request) instead of a diff of the
  file; the content is compared only when it is local, small and text, and is never staged by hunk
  or line. The palette has LFS: Pull and Fetch objects, and a Git LFS dialog lists the files not
  downloaded, the tracked patterns (track and untrack) and the locks on the LFS server (lock,
  unlock, confirmed force unlock); a file's menu can track it, lock it or unlock it. These run
  only when asked, show git-lfs's own message when they fail, and are disabled with the install
  steps when `git-lfs` is missing. An LFS file whose content is exactly its pointer is no longer
  listed as modified because its timestamp changed.
- Canopy asks before it acts on a repository it does not trust: opening one outside every trusted
  folder shows a prompt that says a repository can run programs on this computer through its hooks
  and settings, with Trust and Don't trust. Don't trust opens it read-only: history and diffs show,
  every action is refused with a toast that says why, background fetches skip it, and a banner offers
  Trust…. A trusted folder
  trusts everything inside it, linked worktrees included, and the list in Settings → Privacy keeps
  only the outer folders. Trust safety can be turned off there, after a confirmation; a settings file
  never changes the list or the switch. Folders are compared by what they are on disk, so
  capitalization and links never decide.
- The pull and merge request list has a comfortable / dense toggle in its toolbar (and in Settings,
  Appearance, Request list). Dense shows one line per request, with the number, title, labels and
  checks and no preview; the choice is remembered and is separate from the whole-app density.
  A request's number, author, age and labels each have their own colour in the list, and the
  request's header adds its comment count in one too, in every theme.
- Pull and merge requests use the signed-in GitHub CLI (`gh`) or GitLab CLI (`glab`) as their only
  credential. Canopy no longer stores or accepts personal access tokens, and no longer reads
  `GH_TOKEN`, `GITLAB_TOKEN` or git's credential helper; a `tokens.json` saved by an earlier build
  is deleted at start. Settings → Accounts shows, for each host, whether the CLI is installed,
  signed in (the account and its scopes), signed out or short of a scope, with the command that fixes
  it and a button to copy it, a link to the CLI's install page, and Check again; the requests view
  and the repository's Hosting block say the same. The Privacy Policy and the Terms say so.
- File history: right-click a file in Changes or in a commit's file list, or use the command palette,
  to list the commits that changed it, followed back through renames. Choosing one opens that
  commit with the file's diff; the working tree is not touched.
- Blame: the Blame button in a file's diff header (or "Blame selected file" in the command palette)
  shows who last changed each line, with the commit id, author and date, in place of the diff; click
  a line's commit to open it in History.
- A pull or merge request whose checks and reviews are not loaded (before they arrive, or without a
  token and never opened) shows a faint question mark with that tooltip, instead of looking like a
  request that has none.
- A file replaced by a symbolic link (or the other way round) now shows its lines in the diff: the old
  content or target removed and the new one added, as `git diff` prints them. A link is read, never
  followed, so what it points to is not shown; its hunks offer no actions, since only the whole file
  can be staged or discarded.
- Preview and safely clean up stale linked worktrees and local branches from the command palette
  or Branches header menu. Settings → Git controls the inactivity period (90 days by default);
  uncommitted work and unmerged commits are protected, with safety checked again before removal.

### Fixed

- Custom command… stays in view below the programs in Settings → Integrations, past a separator,
  however many programs the list shows; before, a long list hid it below the fold.
- A push, pull or fetch the host refuses to sign in no longer offers Open Accounts, which cannot
  change how git signs in: the notice says the host did not accept the user name and password git
  sent, and that GitHub and GitLab take an access token as the password. Open Accounts now
  follows every refusal of the GitHub or GitLab CLI's sign-in by the host's API, not only GitHub's.
- Settings → Git and This repository no longer offer `matching` and `nothing` as Push destination
  choices: Canopy's own push understands `simple`, `upstream` and `current`, and `matching` pushes
  every branch with a same-named branch on the remote. A value your own git configuration holds
  still shows on the row with where it is set and why Canopy does not offer it, and is left alone
  until you pick another one.
- Settings → Git with Canopy choosing the Git settings shows one Commits heading over its rows
  instead of two, and a commit cleanup value has one name in the list of Git values, in the notes
  and notices that say where it is set, and in Canopy's own Message cleanup row (Spaces and blank
  lines, Also lines starting with #, Nothing; before, the same value was Whitespace, whitespace and
  Spaces and blank lines).
- The Conflict style help in Settings → Git, with Canopy choosing the Git settings, reads on one
  line, also at zoom 1.6.
- A select never cuts a label short (#225). The closed control widens to the value it shows (a long
  program name in Settings → Integrations is no longer limited to the text-field width) and, when
  the window has no more room, wraps it onto a second line; the open list is as wide as its
  longest entry, and wraps the entries that still do not fit, so the recommended entry keeps its
  whole label beside the Recommended badge even in a window narrower than the two together (#255).
- A select's open list lines up with its control instead of sticking out past its right edge: it
  starts at the control's left edge (its right edge when it is wider and would not fit to the
  right), is as wide as the control or its longest entry, whichever is more, with no 200-point
  minimum of its own, and stays inside the window or dialog (#225).
- In the Observatory dark theme, the dimmed name of a program that is not installed reads at
  4.5:1 or more in an open list, glass included (it was 4.2:1 to 4.4:1 on the plain list).
- The notification center keeps its margin above and below at the smallest window size and zoom, scrolling its list when the window is short.
- A Settings row with both an explanation mark and a state remark (Download profile pictures) draws the remark in the accent colour with a gap, so the two marks no longer look alike.
- Settings are written to a temporary file and renamed over the old one, on a thread of their own,
  so a crash or a full disk while saving no longer leaves a truncated file. A settings file Canopy
  cannot read is kept as `settings.json.bad` instead of being overwritten, Canopy starts with its
  default settings and the notification center says so once.
- What a killed or failed Canopy leaves behind is cleaned up or reported, and nothing of yours is
  deleted. At the next start Canopy removes its own staging files (the saved-token and avatar-cache
  staging files, the patch export's and the update's folders in the temporary folder, an
  interrupted update's staging in the install folder) once the process that made them is gone. An
  update killed halfway through its swap is undone and the old version is back, with a
  notification; the lock a killed update or install leaves in the install folder is taken over on
  Linux, where Canopy can tell its process is gone. Where it cannot (no process id in the lock,
  another system) it clears nothing and says so in a notification; so does an update it refuses
  to clear because no Canopy would be left. A repository you open with a staging file an
  interrupted save left in its working tree gets one notification with the path; Canopy never
  deletes it, nor Git's own lock files. An update whose roll-back could not put every old file
  back keeps them, and the next start refuses to delete them instead of mistaking them for a
  finished update's leftovers. The leftovers of a lock takeover that was itself killed are cleared
  once their process is gone. Exporting patches now stages in the temporary folder
  instead of the folder you chose.
- A Git setting that holds the recommended value says "(Recommended)" after it in its closed select,
  whether it was chosen by hand, written by Use recommended or read from your config, not only while
  it is unset; one that does not shows the bare value. The first branch field says "(Recommended)"
  under it while it holds `main`.
- Discard on a submodule in Changes now works: the submodule goes back to the commit this
  repository records and changes to its tracked files are thrown away, while untracked files inside
  it stay (Settings → Git → Discard in a submodule can delete them too). It refuses when that would
  leave a commit on no branch. Before, it did nothing.
- Opening a repository that stores its references in Git's reftable format says so plainly;
  Canopy cannot open such a repository yet.
- A pull or checkout with `submodule.recurse` on in your git configuration no longer throws away
  uncommitted edits inside a submodule: Canopy updates submodules itself, the way it does when the
  key is unset, and that keeps them.
- A push whose commits record a submodule commit that is on no remote now stops before sending
  anything and names the submodules to push first, instead of failing with git's "src refspec must
  name a ref" while Update submodules is on, or publishing a commit nobody can fetch while it is
  off. A `push.recurseSubmodules` in your git configuration decides instead.
- Upgrades preserve an edited or custom legacy SVG app icon even when an old installer receipt
  still lists it: only a regular file whose current bytes match an icon Canopy shipped is removed.

- Settings → Integrations marks the program your system would use as "<name> (Default)" instead of
  "Recommended", and no longer names the `$TERMINAL`, `$VISUAL` or `$EDITOR` it came from. The desktop's
  default text editor is offered only when it is installed, like the other defaults. Searching
  Settings for "default terminal", "default editor" or "default file manager" finds the row.

- Settings → Integrations lists each terminal, editor and file manager once: the program the system
  starts (kitty behind `xdg-terminal-exec`, Terminal or Finder on macOS, Windows Terminal) is no
  longer repeated beside its own entry, two entries of one program are one row, and a program's
  server, daemon and client entries (`foot --server`, `footclient`) are not offered.

- Oversized file-type changes say the diff exceeds the preview limit, not that the file is
  binary. Their type change and whole-file actions remain visible without false zero counts.

- File-to-submodule diffs and their reverse show the removed and added text, instead of
  hiding it behind a submodule card. Whole-file staging and unstaging remain available.

- Commit file counts include every text line in a file-type change, including files larger
  than the diff preview limit and changes to or from a submodule pointer.

- A repository's own `sequence.editor` setting can no longer start a program through Canopy's git
  commands; the security audit of the other configuration keys is in the architecture notes.
- The installer and the in-app updater now remove the old scalable icon an earlier build left in your
  icon folder, even when no install record lists it, but only when it is byte for byte the icon
  Canopy shipped; a custom, edited or linked icon is left alone.
- In a small window the Changes view keeps room for the file lists: the staged files are no longer
  pushed out of sight by the commit form, which scrolls its fields instead.
- A download of a commit's files that runs past ten minutes now stops with a message naming the
  limit and a Retry button, and the failure is remembered, instead of leaving the commit
  without a way to ask again.
- The notification center saved between runs no longer keeps a user name or password that sat in
  a URL in a title or in Git's error text, and the Privacy Policy now lists those saved
  notifications among the data that stays on your computer (updated 9 October 2026).

- A repository that is not trusted is read-only, and the refusal now names what its own files would
  run (a hook, a clean or smudge filter, a merge driver, `core.sshCommand`, a credential helper,
  `gpg.program`, a submodule update command) instead of saying only that programs could run, and
  says only what is true when it finds none. The checks that only the automatic fetch made now
  cover the whole repository. In a repository that is not trusted the branch cleanup no longer
  checks for squash merges (git's merge machinery runs merge drivers) and the base comparison is
  refused; both work as before once the folder is trusted.

- Feature-branch workflows. The sidebar's Branches group names by their `/` prefixes into
  collapsible folders (nested for deeper prefixes, open by default, remembered per repository,
  with the Remotes tree's arrow keys). A branch merged into the base (`develop` in a git-flow
  repository, else the remote's default branch) shows a merged marker, a branch whose upstream was
  deleted shows `gone`, and one that looks squash-merged shows `squash?`; the tooltip names the
  check, and the readings run once per change of branches, off the UI thread. "Clean up merged
  branches…" deletes the proven ones with `git branch -d` and the unproven ones with `-D` only after
  a confirmation, never the checked-out branch, the base or `main`, `master` and `develop`.
  "Update from base" brings the base's new commits into the checked-out branch (fast-forward, or
  Rebase (recommended) or Merge when they diverged, naming the files that would conflict). Set up,
  Start and Finish for git-flow work from the palette and the Branches menu: Finish can rebase a
  feature or bugfix onto develop first, merges a hotfix into an open release instead of develop,
  and pushes the result (deleting the remote branch only when asked).
- Windows uninstall help and purge confirmation now say that saved hosting tokens are deleted
  along with settings, logs and caches.
- Finishing a single-commit git-flow feature or bugfix fast-forwards when possible, matching
  AVH git-flow. A diverged branch still merges; larger branches keep a merge commit.
- Linked worktrees can trust their shared Git folders from the Downloads off dialog. The chip
  clears when its refused commit details are hidden or a failed status refresh leaves no refusal
  on screen.
- A failed avatar-cache write removes its temporary file and preserves the previous cache.
- With `core.symlinks=false`, unstaged diffs refuse real symbolic links before reading their
  targets; ordinary files used to emulate links still support diffing and staging.
- Symbolic-link diffs identify their old and new targets. Mode changes remain visible when
  there are also content changes.
- A clone whose submodules cannot be downloaded is kept and opened with git's message, instead of
  reporting a failure and blocking a second try because the folder exists.
- A failed download in a partial clone is no longer tried again on every file-watcher refresh;
  Retry or Refresh tries again, and a download that hangs is stopped after ten minutes.
- The title bar's tools take Tab from left to right.
- Settings' section list and theme cards are one Tab stop each, moved with the arrow keys.
  Closing Settings gives keyboard focus back to the control that had it.
- A repository's own configuration can no longer run a command through `core.fsmonitor` when
  Canopy reads it, and auto-fetch no longer runs the programs a repository names (SSH command,
  credential helper, upload-pack, hooks) outside a trusted folder.
- Historical submodule diffs refresh after fetching missing commits. Submodule cards omit
  text diff controls, and a differing checkout no longer claims to contain new commits.
- Long submodule sidebar lists render only visible rows, with status derived when it changes.
- Typing an option's prefix in a mouse-opened select scrolls its match into view.
- A custom pattern saved at the root is no longer checked against a path left in the Ignore form by
  another kind of rule.
- A merge message put into the commit boxes is dropped again when the merge is aborted, unless it was
  edited.
- Cancelling a sign-in prompt says "Sign-in was cancelled." instead of showing git's askpass error.
- Fewer notification cards: a failure that repeats while its card shows raises no second card, an
  automatic fetch that fails reaches the notification center and the bell but not a card, and a pull,
  rebase or merge that stops on conflicts, or a missing program, is a Needs your attention notice, not an error.
- A cherry-pick, a revert, or a stash applied or popped that stops on conflicts is a Needs your
  attention notice that takes you to Changes, not a Git error card, like a pull, rebase or merge.
  For a stash it says the stash stays in the stash list until you drop it.

### Changed

- The conflict shortcuts are named like the resolver's buttons: the shortcuts sheet and Settings →
  Keyboard now say "Accept current version", "Accept incoming version", "Accept both" and "Accept
  the common ancestor" instead of "Take …".
- Background work no longer grows with the number of tabs. At most four background fetches run
  at once across all tabs: a tab that falls due while four run waits for one to finish, then
  fetches, so a long session fetches everything within each interval without starting a `git`
  and `ssh` process per tab at the same time. A fetch you start yourself never waits. The
  refreshes that file changes cause in tabs you are not looking at share a limit of two to four
  at a time (by the number of cores) instead of every tab reading its repository at once when a
  build or another tool touches many; the tab you are looking at always refreshes at once.
- One file watcher serves every open repository instead of one per tab, so many open tabs no
  longer run into the system's limit on watcher instances (128 per user by default on Linux). A
  tab whose files cannot be watched says so once in the notification center: changes on disk then
  show after a Refresh.
- Background fetching no longer fires in every tab at the same moment. When a tab first has
  background fetching on (it opens, or you turn the setting on or change the interval with tabs
  open), its first fetch comes between half the interval and a whole one later, a different time
  for each repository; after a fetch it waits a whole interval.
- The log records frame times and the first frame: a `frame stats` event about every 5 seconds
  while the window is drawing (frames, p50, p95, p99 and worst frame in milliseconds, frames over
  33 and 100 ms; Detailed log level) and a `first frame` event with the start-up time. They help
  tell whether the window stutters; the stress-test report reads them.
- Canopy keeps its files in a folder for each kind of data (settings, logs and crash reports,
  picture cache) following each system's convention, owner-only on Linux and macOS and marked `.canopy-data`
  (unless a folder already holds someone else's files) so the installers' `--purge` finds them, and
  moves what earlier builds kept in two other folders the first time it starts, never replacing a
  file. The privacy policy lists the folders.
- Canopy no longer changes your git configuration unless you turn on Settings → Git → Edit my git
  config (off by default). Until then the git configuration in Settings is read only and Use
  recommended is hidden; Canopy still passes its own choices to every git command. A settings file
  cannot turn it on.
- Settings recommends leaving git's `submodule.recurse` off (Recurse into submodules): with it on,
  git's own checkout, reset and restore can drop edits inside submodules. Use recommended removes it
  from your global git config. Canopy's own Update submodules stays on.
- The commit details header now shows the author, the commit id and the parents before the commit
  subject, and the subject takes the rest of the row. The commit identity card describes the
  author and names the committer only when someone else committed.
- The "Changed files" label is gone from the commit details; the "N files changed" line with its
  line counts is now the section's centred title.
- `canopy-core` no longer has `Repo::stage_all` and `Repo::delete_branch`, which only wrapped other
  functions: use `stage_all_except_repositories`, `delete_merged_branch` and `force_delete_branch`
  instead.
- Text fields in light themes are white plates with a clear edge instead of a grey well, so they
  no longer look disabled; the commit summary shows its character count only once you type.
- The Privacy Policy describes automatic submodule downloads and their Git settings controls;
  accepting the updated policy is required on the next launch.
- Crash reporting sends only crash reports: the session records (when Canopy started, how long it
  ran and how it ended) are no longer sent. The Privacy Policy, the crash report prompt and
  Settings say Sentry deletes crash reports 30 days after it receives them (it said 90), and the
  Terms name the law and courts of the country where NecturaLabs is established; accepting the
  updated documents is required on the next launch.

### Added

- The author's name in the commit details is a link that opens a card with their email (copy), who
  committed it when that is someone else, whether the signature is verified, a link to their
  profile (from a GitHub or GitLab no-reply address) or to the forge's user search for them, and,
  when pictures are kept away, why and where to turn them on; this replaces the info button. The
  commit's parents follow its id as links that select them. A green check after the name marks a
  signature Git trusts (GPG, SSH or X.509), checked in trusted folders only; invalid, unavailable,
  untrusted and expired signatures have distinct marks and names in the author card.
- The commit message field starts with your Git `commit.template` when it is empty (once per
  open repository, never over a draft or an amend); a template that cannot be read is reported.
- The commit fields now say when the summary or a description line is longer than 72 characters
  ("Summary is 80 characters; 72 is the most tools show."). The hint only warns: it never stops
  the commit or changes what you wrote, and URLs, indented and quoted lines are not counted.
  Settings → Git → Commits → Check commit messages turns it off.
- Export as patch… in a commit's menu writes the commit as a `.patch` file in a folder you pick;
  How many commits (1 to 50) adds the commits before it along first parents, oldest first as
  `0001-…patch`, `0002-…patch`, and the dialog says what it will write. A merge commit has no
  patch in git, so merges in the range are skipped and named, and a merge alone is refused.
  Apply patch… in the palette applies a patch file, or the `.patch` files of a folder, as
  commits that keep their authors; a patch that does not fit stops in "Applying patches" with
  Continue and Abort, and the message names the conflicting files.
- Any local branch without an upstream can be published to a remote from its context menu in the
  sidebar, and the Changes list shows untracked folders the status collapsed (a folder that is a
  Git repository, or any top folder when there are very many untracked files) with a Stage
  action that asks first when a repository is inside.

- A partial clone made by Git before 2.24 can be converted from the Open error: Canopy asks, sets
  `remote.<name>.promisor` and removes `extensions.partialClone`, then opens it. A diff a partial
  clone could not download shows a Retry button, is not asked for again on every refresh, and its
  file row carries a "not downloaded" mark until it loads.

- Keyboard shortcuts can be changed in Settings → Keyboard: Change takes the next key you press
  for a row, a key another row uses is refused (naming that row), Reset returns a row or all of
  them to the default, and the choices are kept in a settings export.

- The notification center is kept across restarts (the last 100, shown as read; the decisions
  that need the running session are dropped), each row names the repository it is about and can
  be dismissed on its own, and a repository whose git configuration decides against Canopy's
  recommendation is listed there once, with a button to Git settings.

- A test fails when `src/views` or `src/ui` code names a filesystem, process, network, libgit2 or
  git-CLI API, keeping repository and disk reads in worker jobs.

- Repository insights from the command palette: weekly commits, top authors, Git object storage
  and local branch activity, comparisons and stale/merged hints, without network access.

- Canopy names the programs a repository needs and the system lacks (`git-lfs`, `ssh`, the signing
  program) when it is opened, with install steps for the system.

- Stage all says which folders it left out because they hold a repository of their own, and
  what to do instead.

- A diff that only changes a file's mode now says what happened: made executable, no longer
  executable, or a file turned into a symbolic link and back.

- Without git installed, Canopy opens a dialog with install steps for the system and a download
  link instead of only a toast.

- Submodules: a sidebar section shows each one's state; open one in a tab, initialize and
  update, sync URLs, add (Add submodule… in the palette or the section's +) and remove after a
  confirmation. A submodule's diff lists the commits between its two pointers. Pull, checkout and
  clone update submodules too (Settings → Git → Update submodules, on by default).

- Changes has a Stage selected button right of Stage all, enabled when an unstaged file is
  selected. Ctrl-click and Shift-click pick several unstaged files; Stage selected, Space and
  the row menu stage them together. The diff header no longer repeats it as Stage file; Space
  and the row action still stage the file.

- Ignore management in Changes and the command palette: file, extension, folder and custom rules
  with previews; repository-local and global excludes; ignored-path listings and matching-rule
  explanations. Stop tracking confirms the staged removal while keeping the local file. New
  repositories can start with optional Rust, Node or Python `.gitignore` rules.

- Local-only repository status follows the toolbar's icon and caption layout; the sidebar
  uses quiet labels and compact outlined Publish/Add remote actions.

- A pull looks before it changes anything: it fetches, and when your branch and the remote have
  both moved on it asks whether to rebase (recommended) or merge, and can remember the answer
  for the repository or for every repository; when a pull would likely conflict it names the files first and lets you cancel.
  It then brings in what it fetched, so it asks the remote, and you for credentials, only once.
  A pull or rebase that stops on conflicts, or whose uncommitted changes conflict when they come
  back, takes you to Changes to resolve them.
- Settings → Git: Follow my git config, and Prune tags too, Autostash, Push tags with their
  commits, Push to, Conflict style, Message cleanup and First branch of a new repository, each
  with Canopy's recommendation and, where your git configuration sets it, its value and file.
- A notification center: the bell at the right end of the status bar lists the last 100
  notifications, whether or not they showed as a card, with copy and clear all, and an
  unread count for errors and other notices that need you. Settings → Notifications
  turns the bottom-right cards off or chooses them per type (errors, needs your attention,
  updates, finished operations, confirmations); by default only the first three pop up.
- Settings → Appearance → Tooltips: Helpful (the default) shows only what the screen does not
  already say, such as an icon's name, why something is unavailable, cut-off text, shortcuts and
  full dates; All adds descriptions and usage hints; Off keeps only why something is unavailable.
  A Quick, Normal or Slow delay sets how soon they appear.
- The sidebar marks the branch, remote branch, tag, stash or worktree you clicked while History
  shows its commit, apart from the checked-out branch (accent check, bold name); on opening, the
  checked-out branch is the one marked.
- A Worktrees section in the sidebar lists the repository's working folders, once it has more than
  one: each folder with its branch, the open one marked; double-click (or Open worktree)
  switches the current tab to that worktree. A branch checked out in another worktree switches
  to it instead of attempting a checkout Git would refuse.
- Pull and merge request lists show each request's checks and review without opening it, when
  Canopy has a token for the server: passed, failed or running checks, and approved, changes
  requested or (GitHub) review required. Loading the list sends one more request to the same
  server.
- Redesigned themes and appearance options. Three themes take the new look: Midnight (the
  default) and Daylight, with quiet two-tone chrome in neutral blue-black or white, an indigo
  signature, clean coloured banners, graph lines that never look alike and soft shadows only
  under menus and dialogs; and Observatory, the previous Midnight, kept as it was. Aurora, Ember,
  Neon, Forest and Paper are gone, and Contrast is now a High contrast switch that works with
  every theme: it keeps the theme's backgrounds and colours and raises text, outlines and focus
  to the contrast Midnight's high-contrast version has. Settings → Appearance adds Style
  (Standard, or Deeper, where the action bar and panes float as cards, History splits into a
  commit card and a details card, and the focused pane is lit along its top), Density
  (Comfortable, or Dense with 24-point rows and a one-row toolbar), Motion (match the system,
  reduced or full; reduced also makes scrolling jump instead of glide), which replaces the Reduce
  motion switch (an earlier "reduce motion" choice carries over as Reduced), and the history graph's commit marks (dots, rings, squares or diamonds) and
  lines (curves, metro or circuit).
- A Custom theme, a fourth choice in Settings → Appearance and the toolbar's theme menu (also
  `--theme custom`). Start from Midnight or Daylight and pick an accent (a swatch, any hue, and how
  vivid it is) and a tint for backgrounds, lines and text. Text and lines keep at least the
  base theme's contrast, status, graph, syntax and diff colours keep their hues, and High
  contrast works over it. `--custom-theme` sets its colours from the command line.
- High contrast is in the toolbar's theme menu and the command palette ("High contrast: On" and
  "Off"), as well as in Settings. `--contrast on|off` starts Canopy with it on or off.
- Translucent surfaces in Settings → Appearance: the command palette blurs what is behind it.
  Off by default; unavailable with high contrast, when the system asks for reduced
  transparency, or when the graphics driver cannot do it.
- Git identity in Settings shows the effective author and its source for the open repository, and
  lets you save or clear repository or global Git configuration. A warning in Changes links to it
  when Git cannot use the identity, guessed it, or it looks like a placeholder. First launch asks
  for a global identity after consent when no sensible one is configured. Existing commit authors
  remain exactly as stored in Git history.
- Update from inside Canopy. The update dialog's Update button downloads the new build over HTTPS
  from Canopy's release, checks its SHA-256 against the update feed and installs it. Where the
  one-line installer put Canopy (Linux) it replaces its own folder in place, keeps the previous
  version if anything fails, keeps the command link, launcher entry, icons and `uninstall.sh`
  consistent, and never touches settings, tokens, logs or caches. Any other installation runs the
  one-line installer pinned to the new build. A restart button follows. Nothing is downloaded until
  you press Update; the Privacy Policy describes it.
- GitHub and GitLab (self-hosted GitLab included): forge icons on remotes, "Open on" links for
  commits, branches and remotes, and a pull and merge request view with checks, reviews and the
  conversation, plus create, check out, comment, approve, merge and close, through the host's REST
  API with a token you provide (Settings → Accounts, kept per host and port; an environment
  variable or git's credential helper also works). Requests can be turned off in Settings →
  Integrations.
- Local-only repositories show a "local only" badge, and Publish adds a remote and pushes. The
  clone and add-remote dialogs build the address from `owner/repo` with an HTTPS or SSH choice.
- Seven themes (Midnight, Daylight, Aurora, Ember, Neon, Forest and Paper), interface zoom, reduced
  motion and a settings screen (Ctrl+,) for appearance, Git, integrations, accounts, updates and
  privacy. Every option applies at once.
- A keyboard map in one table: palette, a searchable cheatsheet (`?` or Ctrl+/), vim motions and
  single-key Git actions, never Super and never while typing. `docs/keybindings.md` lists it.
- Credential and SSH passphrase prompts, and host-key questions, in Canopy's own dialog instead of
  failing, whenever git or ssh needs one.
- Staging, unstaging and discarding single lines of a hunk, side-by-side diffs, syntax highlighting,
  and context-line and whitespace options for diffs, global or per repository (Use for every
  repository makes one repository's options the default).
- A three-way conflict resolver (ours, theirs, both, the common ancestor or an edit by hand), and
  merge modes: default, no fast-forward, fast-forward only and squash.
- Each repository tab shows its commits to push and pull.
- An update check against Canopy's own feed, a "What's new" dialog after an upgrade, and the option
  to turn the check off.
- Structured JSON-lines logs per run and local crash reports in Canopy's data folder, with Settings
  → Copy diagnostics. Opt-in, scrubbed crash reports to NecturaLabs, asked on first start together
  with the Terms of Service and Privacy Policy, which can be read in the app.
- SVG illustrations for the empty states and the welcome screen (placeholders until the final
  art).
- One-line installers for Linux and macOS (`curl -fsSL …/canopy/install.sh | sh`) and Windows
  (`irm …/canopy/install.ps1 | iex`): per-user, no sudo, a folder of Canopy's own, SHA-256
  checked downloads, in-place upgrades and a clean uninstall. macOS builds are now a
  `Canopy.app` bundle.

### Security

- Hosting diagnostics hide tokens after normalizing invisible characters. JSON shape errors
  report the failure location without quoting response values that could contain a token.
- Files Canopy reads or writes inside a repository (the conflict resolver) refuse paths that leave
  the working tree, name `.git`, or cross a symbolic link; a conflicted file behind a symlinked
  folder now gives an error instead of opening.
- A credential or host-key prompt can no longer be replaced or dropped by another dialog; dialogs
  queue behind it, and Escape on a host-key question answers "no".
- Crash reports also scrub credentials in every URL, GitHub and GitLab token patterns, `Bearer`
  values, and home folders only at whole-path boundaries.
- Saved hosting tokens are kept per host and port, so a GitLab instance on another port no longer
  shares a token with the default one.
- Programs Canopy opens (terminal, editor, browser) never inherit the credential prompt's secret.
- The installers' test-only feed override accepts nothing but a loopback host and a port, so a
  value such as `http://127.0.0.1:1@elsewhere.example` is refused. `install.ps1` follows
  redirects itself and refuses any that is not HTTPS; `install.sh` ignores `~/.curlrc` and
  `~/.wgetrc`.
- The installers never write through a symlink or over a desktop entry, icon, Start Menu shortcut
  or Apps & features entry that another install owns, and `--purge` deletes a generically named
  folder (`canopy`) only when it is marked as Canopy's (`.canopy-data`) or holds Canopy's own
  files.

### Fixed

- On a desktop session without an accessibility bus, Canopy no longer sends a crash report for
  the accessibility library's failed startup; the app runs on without screen-reader support.

- Pressing a button, including the top bar's operation tools, eases in and stays visible for at
  least a moment, so a quick click that starts an operation no longer flashes.

- Select menus stay within their containing dialog or panel, open above the field when needed
  and scroll through longer lists. Keyboard navigation also works after opening with the mouse,
  and the highlighted option scrolls into view in both directions. A quick click that opens a
  select in Settings or a dialog no longer leaves the list hidden behind it.

- With a screen reader running, closing a dialog (such as the force push confirmation) no
  longer crashes Canopy.
- Notification cards follow the Dense density like the rest of the window, with tighter padding
  and a smaller icon, instead of keeping Comfortable's.
- A commit keeps message lines that start with `#` (an issue number, a Markdown heading) unless
  your git configuration sets `commit.cleanup`.
- Submenus in context menus (Reset current branch here in History, and others) no longer close
  as soon as the pointer leaves their item on the way to them.
- The pull and merge request list refreshes by itself after a fetch, pull or push, and when you
  open it after a minute away, instead of only when you press Refresh.
- Repository tabs show each name in full for as long as the title bar has room, instead of
  cutting names at a fixed width. When the tabs outgrow the bar the longest names give up width
  first (the active tab keeps its name), and when that is not enough the tabs scroll: scroll
  buttons at both ends (dimmed at the ends), a list of every open repository, the mouse wheel,
  and the active tab always kept in view. A tab's tooltip shows only when it adds something.
- A saved theme or option this build no longer has, such as a removed theme, now reads as its
  default instead of resetting every setting. Settings the build no longer has are dropped at the
  first save after launch (Canopy saves every 30 seconds).
- Keyboard use: each list (History's commits, every sidebar section) is one Tab stop and the
  arrows move inside it, History's commit list comes before the details, and a row's buttons stay
  visible while one of them has focus. Shift+F10 opens the focused row's context menu; in a menu
  the arrows stay inside it and wrap, and Escape returns to the row. The command palette and the
  cheatsheet keep Tab inside them and give focus back when they close, and dragging a divider no
  longer takes the arrows away from the lists.
- Screen readers hear lists, trees, menus, choices, tabs and a select's open list as groups of
  their items, a commit by its refs, author and date, whether a section is open (Linux), why a
  control is unavailable, and an error notification's message, and no longer hear callouts twice
  or icons as symbols.
- An error notification without a known explanation shows git's message instead of only its
  title, and a new notification never pushes out an unread error.
- A typed number setting is stored exactly as shown, a revealed password is hidden again at the
  next prompt, a select listing every branch opens quickly, and the sliding segment and tab
  indicators stretch the right way when moving left.
- Author pictures appear after adding a token for a private repository, for your own unpushed
  commits, and after a lost connection, instead of staying initials for 12 hours. GitLab authors
  without a picture show initials instead of a generated pattern.
- Scroll bars in the commit details, toasts and code boxes are a thin, quiet handle beside the
  text instead of a thick full-colour bar. The soft glow inside the illustrations fades smoothly instead of in rings.
- Pull uses the chosen strategy (merge, rebase or fast-forward only) whenever `git` itself has
  no `pull.rebase` setting. Canopy read the setting through libgit2, which ignores
  `GIT_CONFIG_GLOBAL`, so it could skip the strategy, and recent versions of Git refuse to
  pull diverged branches without one.
- Commit details restore a readable height after a small window is enlarged. Expand and a
  compact History/Commit details switch give the author, changed files and diff useful space.
  A shorter initial pane leaves less empty space under short diffs.
- History search now appears only when invoked with Ctrl+F or `/`; those shortcuts, Esc and the
  close button hide it and restore the full graph. The redundant commit count and status-bar
  repository path are removed.
- Commit detail Expand and Back sit at the right end of the filename row instead of adding another
  header row. Ref and request pills use clearer color fills and legible text; opened context submenu
  items match hovered menu items rather than flashing purple.
- Interface zoom no longer moves the Settings slider during a drag; percentage entry now parses
  correctly; increasing zoom no longer increases the native window minimum size.
- Clone destination validation runs off the UI thread and keeps the entered values on failure.
- Native folder selection keeps the app event loop running while browsing instead of causing
  operating-system unresponsive warnings.
- Compact settings, legal text, shortcuts, palette, diff controls and history labels stay within
  their viewport; long paths no longer overlap status counts.
- Staging, unstaging and discarding use each path literally: a file named `*` or `[ab].txt` no
  longer touches other files (discarding an untracked `[ab].txt` used to delete an untracked
  `b.txt` too).
- Deleting a branch from the sidebar runs `git branch -d`: a branch with unmerged commits is no
  longer deleted silently, and Canopy asks before forcing it.
- Opening a large repository no longer freezes the window: the repository is opened and its file
  watcher set up off the UI thread.
- Commits, checkouts and fetches in a linked worktree or submodule now refresh Canopy: their git
  folder (and the shared common folder) is watched too. The watcher no longer follows symbolic
  links.
- Installers: a failed or interrupted (Ctrl+C) upgrade puts the previous version back without
  ever deleting part of it; uninstall removes only the installer's own files and keeps the folder
  when it holds anything else; re-running no longer silently replaces a newer version (such as a
  beta) with an older one; a failed first install leaves no empty folders behind and does not
  claim an existing empty folder; a symlinked home folder no longer breaks uninstall; errors
  during downloads, including a missing `curl` and `wget`, are always shown; a script cut short
  in download runs nothing; and through `irm | iex` the Windows installer leaves the PowerShell
  session as it found it.
- Partial clones (`git clone --filter=blob:none` or `--filter=tree:0`) no longer fail with
  "object not found" when you open an old commit or its diff: Canopy downloads the missing
  objects through your `git` (the status bar says from where) and shows git's own message, with a
  Retry button, when that fails. The commit's file list appears at once, and its line counts
  follow once downloaded (commits over 200 files keep the list without counts). Selecting several
  commits quickly downloads only for the last one. After a soft reset in a treeless clone the
  history still loads, and the Changes view says why it cannot list changes yet instead of
  showing every file as added. A remote whose name looks like a git option is refused, never
  run. Partial clones made by Git before 2.24 get a message with the commands that convert them.
- Secondary and faint text, links, status colours and graph lanes are readable on every surface
  they are drawn on, selected rows and diff lines included, in all seven themes (WCAG AA; body
  text AAA). Selected values in Settings use the same calmer selection fill as lists. Branch and
  tag pills, avatar initials and change and count badges keep their colour but are always at
  least 4.5:1 on their own fill, which they were not in the light themes.
- Syntax highlighting restarts at every hunk, so a hunk that begins inside a block comment or
  string no longer colours the hunks after it.
- Screen readers: the window is named after the open repository ("tidepool — Canopy"); diff
  lines are read with their kind and number ("Added line 12: …") and hunk headers by number;
  conflict code blocks are named (Current version, Incoming version, Result) and read; the status
  bar, commit details and Changes no longer read icon glyphs, and ahead/behind reads "2 ahead,
  1 behind origin/main"; Commit, Continue, Abort and the resolver's Save buttons say when they are
  unavailable and why; each welcome action is read once, with its help and its keys; and Tab in
  Settings scrolls the page to the control it reaches.
- A file both sides deleted offers one "Delete it" instead of two. Conflict side previews drop the
  indentation their lines share and never cut a number in two.
- Ctrl+F opens Settings search when no repository is open.
- Pop, on the toolbar, its shortcut and the palette, pops the stash the toolbar names, even when
  the stash list changed since.
- The Open and New repository forms show an error and stay usable when Canopy cannot start
  opening the folder, instead of waiting forever.
- Each password or passphrase prompt starts with its answer hidden, even after you showed the
  previous one's.
- Closing a tab or switching worktrees keeps its unsent commit message, conflict choices and
  hand edits, and request comments until you open the repository again in the same session.
  Comments return only to their own host, project and request.
- A build skipped in the update dialog by an earlier version stays skipped.
- The Accounts link in a toast opens Settings the same way the Settings button does, and Open logs
  folder says when the folder cannot be created.
- The diff's "Click lines to pick part of a hunk" hint no longer covers a long hunk title.
- In a small window or at a large zoom, the commit button drops its keys, then the branch, then the
  file count instead of cutting its label; the action bar fades out a tool it cuts at its edge;
  and a Settings page fades into its card where it continues instead of cutting a line in half.
- The commit composer's scroll bar is a thin handle like the others, not a black bar in light
  themes.
- The request list can be resized from the keyboard and reset with a double-click, like the other
  panes.
- The Terms and the Privacy Policy open at the top, also on first start after a restart.
- The help for a custom editor, terminal or file manager command says that `%f` or `%d` must be a
  word of its own.

### Changed

- Text fields, number fields and selects are taller (32 pt, 28 in Dense) so forms and the commit
  composer read less cramped and line up with their buttons; search fields stay compact.

- What your git configuration sets still decides; everywhere else Canopy's git choices follow
  today's recommended practice and are passed to git explicitly: a pull only fast-forwards
  unless you choose otherwise, uncommitted changes are set aside around a pull or rebase, a push
  sends the annotated tags on its commits, conflicts show the original text too, local tags are
  never pruned, and new repositories start on `main`. Pull options live in Settings only; the
  menu beside the Pull button is gone.
- Notification cards no longer have an expandable Details section, and the notification center
  explains a failure the same way its card does, with git's full output in its tooltip and Copy.
- Notifications look the same in every theme and have no close button: one that asks for a
  decision says so with two buttons named for it (Pull or Not now, Review update or Later,
  Restart now or Later, Open Accounts or Not now), and the notification center offers it until it
  is taken. A new build and an installed update now ask this way. Cards stay as long as their
  words take to read, 5 to 10 seconds (8 to 15 for an error, 15 for a decision, which then waits
  in the notification center and counts on the bell), and pause while you point at them.
- Primary and destructive buttons are tonal instead of solid: a ground tinted with the accent (or
  red), a fine edge in the same colour and a bold label in it, so the main action stands out
  without a heavy block of colour.
- Commit details no longer have a "Commit information" block repeating the email, date and
  parents: the message shows directly under the author, the email (and the committer, when someone
  else made the commit) is in the author's tooltip and the full date in the date's.
- Branch, tag and stash pills in History are redesigned as one consistent style: every pill is
  softly tinted in its commit's graph colour with a hairline in that colour and a white label,
  told apart by its icon; in light themes, where white would not read, the label takes the
  graph colour. The checked-out branch is tinted a little more.
- Prompts mark Canopy's recommended choice and leave out what goes without saying, such as "Esc
  cancels"; notifications of finished work say what happened ("Pulled", "Could not push").
- Select boxes look like the buttons beside them, with an up-down caret; text fields have one even
  edge; links brighten and underline under the pointer, and links to the web end in an arrow.
- Settings rows now keep their controls in one aligned column.
- Releases are numbered builds, named by `git rev-list --count` of their commit on `main` and its
  short hash (`build 412 (a1b2c3d)`), instead of semantic versions. The number is shown in About,
  `canopy --version`, the logs and the update check; the update check, the one-line installers
  (`--version N`) and the update feed compare builds as integers, and a version such as
  `0.2.1-beta.1` counts as older than every build. Nothing is committed or bumped to release: the
  Create release workflow tags `build-N`. Installations older than the first numbered build do
  not understand the new feed: their update check reports an error until Canopy is reinstalled with
  the one-line installer.
- Compact windows move repository navigation into a menu, adapt pane widths and history columns,
  and scroll tabs and toolbar actions. Shared typography, control outlines and keyboard focus
  indicators are more consistent across themes.
- Screenshot tooling supports native Hyprland capture when Xvfb is unavailable, workspace
  selection and `--zoom` for scaled-layout checks; captures reset saved panel layout.
- Stage all leaves out a folder that holds its own Git repository instead of adding it as a
  submodule entry.
- A new branch never tracks its start point on its own (pushing sets the upstream), and a branch
  or start point whose name begins with `-` is refused instead of being read by git as an option.
- Interface transitions are shorter (150 ms instead of 180 ms), in line with the new motion
  tokens.
- After accepting the Privacy Policy and Terms on first run, start-up notices and the update
  check now start without a restart.
- `~` in clone and init destinations and opened paths expands to the home folder.
- A repository, clone or update thread that fails to start reports an error instead of loading
  forever.
- Smooth with huge repositories and many tabs: frames no longer copy the snapshot, diff or
  conflict file; long sidebar, file and request lists draw only the rows in view; the diff keeps
  its layout and scrolls a 50,000-line diff as cheaply as a short one; the conflict resolver
  skips cards out of view; syntax highlighting runs on its own threads; the command palette
  builds its entries once per opening.

## [0.2.1-beta.1] - 2026-10-01

### Added

- GitHub and GitLab (self-hosted GitLab included): forge icons on remotes, "Open on" links for
  commits, branches and remotes, and a pull and merge request view with checks, reviews and the
  conversation, plus create, check out, comment, approve, merge and close, through the host's REST
  API with a token you provide (Settings → Accounts, kept per host and port; an environment
  variable or git's credential helper also works). Requests can be turned off in Settings →
  Integrations.
- Local-only repositories show a "local only" badge, and Publish adds a remote and pushes. The
  clone and add-remote dialogs build the address from `owner/repo` with an HTTPS or SSH choice.
- Seven themes (Midnight, Daylight, Aurora, Ember, Neon, Forest and Paper), interface zoom, reduced
  motion and a settings screen (Ctrl+,) for appearance, Git, integrations, accounts, updates and
  privacy. Every option applies at once.
- A keyboard map in one table: palette, a searchable cheatsheet (`?` or Ctrl+/), vim motions and
  single-key Git actions, never Super and never while typing. `docs/keybindings.md` lists it.
- Credential and SSH passphrase prompts, and host-key questions, in Canopy's own dialog instead of
  failing, whenever git or ssh needs one.
- Staging, unstaging and discarding single lines of a hunk, side-by-side diffs, syntax highlighting,
  and context-line and whitespace options for diffs, global or per repository.
- A three-way conflict resolver (ours, theirs, both, the common ancestor or an edit by hand), and
  merge modes: default, no fast-forward, fast-forward only and squash.
- Each repository tab shows its commits to push and pull.
- An update check against Canopy's own feed, a "What's new" dialog after an upgrade, and the option
  to turn the check off.
- Structured JSON-lines logs per run and local crash reports in Canopy's data folder, with Settings
  → Copy diagnostics. Opt-in, scrubbed crash reports to NecturaLabs, asked on first start together
  with the Terms of Service and Privacy Policy, which can be read in the app.
- SVG illustrations for the empty states and the welcome screen (placeholders until the final
  art).
- One-line installers for Linux and macOS (`curl -fsSL …/canopy/install.sh | sh`) and Windows
  (`irm …/canopy/install.ps1 | iex`): per-user, no sudo, a folder of Canopy's own, SHA-256
  checked downloads, in-place upgrades and a clean uninstall. macOS builds are now a
  `Canopy.app` bundle.

### Security

- Files Canopy reads or writes inside a repository (the conflict resolver) refuse paths that leave
  the working tree, name `.git`, or cross a symbolic link; a conflicted file behind a symlinked
  folder now gives an error instead of opening.
- A credential or host-key prompt can no longer be replaced or dropped by another dialog; dialogs
  queue behind it, and Escape on a host-key question answers "no".
- Crash reports also scrub credentials in every URL, GitHub and GitLab token patterns, `Bearer`
  values, and home folders only at whole-path boundaries.
- Saved hosting tokens are kept per host and port, so a GitLab instance on another port no longer
  shares a token with the default one.
- Programs Canopy opens (terminal, editor, browser) never inherit the credential prompt's secret.
- The installers' test-only feed override accepts nothing but a loopback host and a port, so a
  value such as `http://127.0.0.1:1@elsewhere.example` is refused. `install.ps1` follows
  redirects itself and refuses any that is not HTTPS; `install.sh` ignores `~/.curlrc` and
  `~/.wgetrc`.
- The installers never write through a symlink or over a desktop entry, icon, Start Menu shortcut
  or Apps & features entry that another install owns, and `--purge` deletes a generically named
  folder (`canopy`) only when it is marked as Canopy's (`.canopy-data`) or holds Canopy's own
  files.

### Fixed

- Commit details restore a readable height after a small window is enlarged. Expand and a
  compact History/Commit details switch give the author, changed files and diff useful space.
  A shorter initial pane leaves less empty space under short diffs.
- History search now appears only when invoked with Ctrl+F or `/`; those shortcuts, Esc and the
  close button hide it and restore the full graph. The redundant commit count and status-bar
  repository path are removed.
- Commit detail Expand and Back sit at the right end of the filename row instead of adding another
  header row. Ref and request pills use clearer color fills and legible text; opened context submenu
  items match hovered menu items rather than flashing purple.

- Interface zoom no longer moves the Settings slider during a drag; percentage entry now parses
  correctly; increasing zoom no longer increases the native window minimum size.
- Clone destination validation runs off the UI thread and keeps the entered values on failure.
- Native folder selection keeps the app event loop running while browsing instead of causing
  operating-system unresponsive warnings.
- Compact settings, legal text, shortcuts, palette, diff controls and history labels stay within
  their viewport; long paths no longer overlap status counts.
- Staging, unstaging and discarding use each path literally: a file named `*` or `[ab].txt` no
  longer touches other files (discarding an untracked `[ab].txt` used to delete an untracked
  `b.txt` too).
- Deleting a branch from the sidebar runs `git branch -d`: a branch with unmerged commits is no
  longer deleted silently, and Canopy asks before forcing it.
- Opening a large repository no longer freezes the window: the repository is opened and its file
  watcher set up off the UI thread.
- Commits, checkouts and fetches in a linked worktree or submodule now refresh Canopy: their git
  folder (and the shared common folder) is watched too. The watcher no longer follows symbolic
  links.
- Installers: a failed or interrupted (Ctrl+C) upgrade puts the previous version back without
  ever deleting part of it; uninstall removes only the installer's own files and keeps the folder
  when it holds anything else; re-running no longer silently replaces a newer version (such as a
  beta) with an older one; a failed first install leaves no empty folders behind and does not
  claim an existing empty folder; a symlinked home folder no longer breaks uninstall; errors
  during downloads, including a missing `curl` and `wget`, are always shown; a script cut short
  in download runs nothing; and through `irm | iex` the Windows installer leaves the PowerShell
  session as it found it.
- Partial clones (`git clone --filter=blob:none` or `--filter=tree:0`) no longer fail with
  "object not found" when you open an old commit or its diff: Canopy downloads the missing
  objects through your `git` (the status bar says from where) and shows git's own message, with a
  Retry button, when that fails. The commit's file list appears at once, and its line counts
  follow once downloaded (commits over 200 files keep the list without counts). Selecting several
  commits quickly downloads only for the last one. After a soft reset in a treeless clone the
  history still loads, and the Changes view says why it cannot list changes yet instead of
  showing every file as added. A remote whose name looks like a git option is refused, never
  run. Partial clones made by Git before 2.24 get a message with the commands that convert them.
- Secondary and faint text, links, status colours and graph lanes are readable on every surface
  they are drawn on, selected rows and diff lines included, in all seven themes (WCAG AA; body
  text AAA). Selected values in Settings use the same calmer selection fill as lists. Branch and
  tag pills, avatar initials and change and count badges keep their colour but are always at
  least 4.5:1 on their own fill, which they were not in the light themes.
- Syntax highlighting restarts at every hunk, so a hunk that begins inside a block comment or
  string no longer colours the hunks after it.

### Changed

- Compact windows move repository navigation into a menu, adapt pane widths and history columns,
  and scroll tabs and toolbar actions. Shared typography, control outlines and keyboard focus
  indicators are more consistent across themes.
- Screenshot tooling supports native Hyprland capture when Xvfb is unavailable, workspace
  selection and `--zoom` for scaled-layout checks; captures reset saved panel layout.
- Stage all leaves out a folder that holds its own Git repository instead of adding it as a
  submodule entry.
- A new branch never tracks its start point on its own (pushing sets the upstream), and a branch
  or start point whose name begins with `-` is refused instead of being read by git as an option.
- Interface transitions are shorter (150 ms instead of 180 ms), in line with the new motion
  tokens.
- After accepting the Privacy Policy and Terms on first run, start-up notices and the update
  check now start without a restart.
- `~` in clone and init destinations and opened paths expands to the home folder.
- A repository, clone or update thread that fails to start reports an error instead of loading
  forever.
- Smooth with huge repositories and many tabs: frames no longer copy the snapshot, diff or
  conflict file; long sidebar, file and request lists draw only the rows in view; the diff keeps
  its layout and scrolls a 50,000-line diff as cheaply as a short one; the conflict resolver
  skips cards out of view; syntax highlighting runs on its own threads; the command palette
  builds its entries once per opening.

## [0.2.0-beta.1] - 2026-10-01

### Added

- GitHub and GitLab (self-hosted GitLab included): forge icons on remotes, "Open on" links for
  commits, branches and remotes, and a pull and merge request view with checks, reviews and the
  conversation, plus create, check out, comment, approve, merge and close, through the host's REST
  API with a token you provide (Settings → Accounts, kept per host and port; an environment
  variable or git's credential helper also works). Requests can be turned off in Settings →
  Integrations.
- Local-only repositories show a "local only" badge, and Publish adds a remote and pushes. The
  clone and add-remote dialogs build the address from `owner/repo` with an HTTPS or SSH choice.
- Seven themes (Midnight, Daylight, Aurora, Ember, Neon, Forest and Paper), interface zoom, reduced
  motion and a settings screen (Ctrl+,) for appearance, Git, integrations, accounts, updates and
  privacy. Every option applies at once.
- A keyboard map in one table: palette, a searchable cheatsheet (`?` or Ctrl+/), vim motions and
  single-key Git actions, never Super and never while typing. `docs/keybindings.md` lists it.
- Credential and SSH passphrase prompts, and host-key questions, in Canopy's own dialog instead of
  failing, whenever git or ssh needs one.
- Staging, unstaging and discarding single lines of a hunk, side-by-side diffs, syntax highlighting,
  and context-line and whitespace options for diffs, global or per repository.
- A three-way conflict resolver (ours, theirs, both, the common ancestor or an edit by hand), and
  merge modes: default, no fast-forward, fast-forward only and squash.
- Each repository tab shows its commits to push and pull.
- An update check against Canopy's own feed, a "What's new" dialog after an upgrade, and the option
  to turn the check off.
- Structured JSON-lines logs per run and local crash reports in Canopy's data folder, with Settings
  → Copy diagnostics. Opt-in, scrubbed crash reports to NecturaLabs, asked on first start together
  with the Terms of Service and Privacy Policy, which can be read in the app.
- SVG illustrations for the empty states and the welcome screen (placeholders until the final
  art).
- One-line installers for Linux and macOS (`curl -fsSL …/canopy/install.sh | sh`) and Windows
  (`irm …/canopy/install.ps1 | iex`): per-user, no sudo, a folder of Canopy's own, SHA-256
  checked downloads, in-place upgrades and a clean uninstall. macOS builds are now a
  `Canopy.app` bundle.

### Security

- Files Canopy reads or writes inside a repository (the conflict resolver) refuse paths that leave
  the working tree, name `.git`, or cross a symbolic link; a conflicted file behind a symlinked
  folder now gives an error instead of opening.
- A credential or host-key prompt can no longer be replaced or dropped by another dialog; dialogs
  queue behind it, and Escape on a host-key question answers "no".
- Crash reports also scrub credentials in every URL, GitHub and GitLab token patterns, `Bearer`
  values, and home folders only at whole-path boundaries.
- Saved hosting tokens are kept per host and port, so a GitLab instance on another port no longer
  shares a token with the default one.
- Programs Canopy opens (terminal, editor, browser) never inherit the credential prompt's secret.
- The installers' test-only feed override accepts nothing but a loopback host and a port, so a
  value such as `http://127.0.0.1:1@elsewhere.example` is refused. `install.ps1` follows
  redirects itself and refuses any that is not HTTPS; `install.sh` ignores `~/.curlrc` and
  `~/.wgetrc`.
- The installers never write through a symlink or over a desktop entry, icon, Start Menu shortcut
  or Apps & features entry that another install owns, and `--purge` deletes a generically named
  folder (`canopy`) only when it is marked as Canopy's (`.canopy-data`) or holds Canopy's own
  files.

### Fixed

- Staging, unstaging and discarding use each path literally: a file named `*` or `[ab].txt` no
  longer touches other files (discarding an untracked `[ab].txt` used to delete an untracked
  `b.txt` too).
- Deleting a branch from the sidebar runs `git branch -d`: a branch with unmerged commits is no
  longer deleted silently, and Canopy asks before forcing it.
- Opening a large repository no longer freezes the window: the repository is opened and its file
  watcher set up off the UI thread.
- Commits, checkouts and fetches in a linked worktree or submodule now refresh Canopy: their git
  folder (and the shared common folder) is watched too. The watcher no longer follows symbolic
  links.
- Installers: a failed or interrupted (Ctrl+C) upgrade puts the previous version back without
  ever deleting part of it; uninstall removes only the installer's own files and keeps the folder
  when it holds anything else; re-running no longer silently replaces a newer version (such as a
  beta) with an older one; a failed first install leaves no empty folders behind and does not
  claim an existing empty folder; a symlinked home folder no longer breaks uninstall; errors
  during downloads, including a missing `curl` and `wget`, are always shown; a script cut short
  in download runs nothing; and through `irm | iex` the Windows installer leaves the PowerShell
  session as it found it.
- Partial clones (`git clone --filter=blob:none` or `--filter=tree:0`) no longer fail with
  "object not found" when you open an old commit or its diff: Canopy downloads the missing
  objects through your `git` (the status bar says from where) and shows git's own message, with a
  Retry button, when that fails. The commit's file list appears at once, and its line counts
  follow once downloaded (commits over 200 files keep the list without counts). Selecting several
  commits quickly downloads only for the last one. After a soft reset in a treeless clone the
  history still loads, and the Changes view says why it cannot list changes yet instead of
  showing every file as added. A remote whose name looks like a git option is refused, never
  run. Partial clones made by Git before 2.24 get a message with the commands that convert them.
- Secondary and faint text, links, status colours and graph lanes are readable on every surface
  they are drawn on, selected rows and diff lines included, in all seven themes (WCAG AA; body
  text AAA). Selected values in Settings use the same calmer selection fill as lists. Branch and
  tag pills, avatar initials and change and count badges keep their colour but are always at
  least 4.5:1 on their own fill, which they were not in the light themes.
- Syntax highlighting restarts at every hunk, so a hunk that begins inside a block comment or
  string no longer colours the hunks after it.

### Changed

- Stage all leaves out a folder that holds its own Git repository instead of adding it as a
  submodule entry.
- A new branch never tracks its start point on its own (pushing sets the upstream), and a branch
  or start point whose name begins with `-` is refused instead of being read by git as an option.
- Interface transitions are shorter (150 ms instead of 180 ms), in line with the new motion
  tokens.
- After accepting the Privacy Policy and Terms on first run, start-up notices and the update
  check now start without a restart.
- `~` in clone and init destinations and opened paths expands to the home folder.
- A repository, clone or update thread that fails to start reports an error instead of loading
  forever.
- Smooth with huge repositories and many tabs: frames no longer copy the snapshot, diff or
  conflict file; long sidebar, file and request lists draw only the rows in view; the diff keeps
  its layout and scrolls a 50,000-line diff as cheaply as a short one; the conflict resolver
  skips cards out of view; syntax highlighting runs on its own threads; the command palette
  builds its entries once per opening.

## [0.1.0] - 2026-10-01

### Added

- Welcome screen with recent repositories; open, clone with live progress, and init.
- Several repositories open at once in tabs.
- Commit graph with coloured lanes, curved merge edges, ref labels for HEAD, local and remote
  branches and tags, initials avatars, relative dates, and filtering by message, author or SHA.
- Commit details with metadata, parents, changed files with added and removed line counts, and a
  diff per file.
- Working copy with unstaged, staged and conflicted files; stage, unstage and discard per file,
  for all files, and per hunk; diff viewer with old and new line numbers.
- Commit composer with summary and description, a character counter, and amend.
- Branches: create (from any commit), checkout, rename, delete, merge into current, rebase current
  onto, ahead/behind badges, and checkout of remote branches as tracking branches.
- Remotes: fetch (all remotes, prune), pull (fast-forward or rebase per git config), and push
  (sets the upstream when missing; optional force-with-lease), all through the system `git`.
- Tags (lightweight and annotated; delete; push) and stashes (save with message and optionally
  untracked files; apply; pop; drop).
- Commit context menu: checkout (detached), new branch or tag here, cherry-pick, revert, reset
  the current branch (soft, mixed, hard), copy SHA or message.
- Banner for merge, rebase and cherry-pick in progress with continue and abort.
- Confirmation dialogs for every destructive operation.
- Automatic refresh from a file watcher and on window focus.
- Command palette (Ctrl+K, Ctrl+Shift+P) for every command and branch checkout, keyboard
  shortcuts, toasts, and a status bar with branch, ahead/behind, repository state and the running
  job.
- Midnight (dark, default) and Daylight (light) themes with bundled Inter and JetBrains Mono.
- Command-line options `--light`, `--view`, `--size` and `--screenshot`.
- Linux desktop entry, AppStream metainfo and application icon.

[Unreleased]: https://github.com/NecturaLabs/Canopy/compare/v0.2.1-beta.1...HEAD
[0.2.1-beta.1]: https://github.com/NecturaLabs/Canopy/releases/tag/v0.2.1-beta.1
[0.2.0-beta.1]: https://github.com/NecturaLabs/Canopy/releases/tag/v0.2.0-beta.1
[0.1.0]: https://github.com/NecturaLabs/Canopy/releases/tag/v0.1.0
