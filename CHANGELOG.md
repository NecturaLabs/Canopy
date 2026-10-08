# Changelog

All notable changes to Canopy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Releases up to 0.2.1-beta.1 used
[Semantic Versioning](https://semver.org/spec/v2.0.0.html); from the first numbered build on, a
release is a build named by its number (see Unreleased), and its notes are generated from the
commits since the previous build.

## [Unreleased]

### Fixed

- A repository's own configuration can no longer run a command through `core.fsmonitor` when
  Canopy reads it, and auto-fetch no longer runs the programs a repository names (SSH command,
  credential helper, upload-pack, hooks) outside a trusted folder.
- Historical submodule diffs refresh after fetching missing commits. Submodule cards omit
  text diff controls, and a differing checkout no longer claims to contain new commits.
- Long submodule sidebar lists render only visible rows, with status derived when it changes.
- Typing an option's prefix in a mouse-opened select scrolls its match into view.

### Changed

- The Privacy Policy describes automatic submodule downloads and their Git settings controls;
  accepting the updated policy is required on the next launch.

### Added

- Stage all says which folders it left out because they hold a repository of their own, and
  what to do instead.
- Canopy names the programs a repository needs and the system lacks (`git-lfs`, `ssh`, the signing
  program) when it is opened, with install steps for the system.


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
