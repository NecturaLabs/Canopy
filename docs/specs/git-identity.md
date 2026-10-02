# Git identity (author details): findings and design

**Status:** implemented and driven in the running app (2 October 2026); the evidence is listed under
"Verification".
**Date:** 2 October 2026

Covers the three backlog items "Git identity in Settings", "Identity warning" and "First-launch
identity step", which shared one core and are now removed from the backlog. Git behaviour below was
probed with git 2.55.0 in an isolated `HOME` (`GIT_CONFIG_NOSYSTEM=1`), never against the developer's
real configuration.

## What the owner saw, and why

History shows the author each commit **stored**: `Repo::commits` and `Repo::commit_details` read
`git2::Commit::author()`. It never substitutes the current configuration, so changing today's
settings cannot change an old commit, and Canopy must not rewrite history to "fix" one. In the
repository checked, 28 of 121 reachable commits carried a placeholder-looking stored author (a
one-character name or an email domain without a dot). The part Canopy can fix is the future: show
what Git will use for the next commit, warn when that is unusable, guessed or a placeholder, and let
the user correct it in Settings. History's column header reads plainly "Author" (the owner's call);
the column always shows what the commit recorded.

## What git does (probed)

1. **Effective identity: ask git.** `git var GIT_AUTHOR_IDENT` and `git var GIT_COMMITTER_IDENT`
   print `Name <email> <epoch> <tz>`, or exit 128 with the "Author identity unknown" text when git
   cannot decide. They honour everything a commit honours: `GIT_AUTHOR_*` / `GIT_COMMITTER_*`,
   `EMAIL`, `author.*` / `committer.*` (these differ from `user.*`: author and committer idents
   differed in a probe), `user.*`, `includeIf`, `user.useConfigOnly` and the passwd/hostname guess.
2. **Where each value comes from.** One call:
   `git config -z --show-scope --show-origin --get-regexp '^(user|author|committer)\.(name|email)$'`.
   Each entry is `scope NUL origin NUL key NL value NUL`; scopes seen: `system`, `global`, `local`,
   `worktree`, `command`. An `includeIf` file reports scope `global` with the included file as
   origin. The last entry for a key wins. Exit 1 means no match. Environment variables are not
   listed; Canopy reads `GIT_AUTHOR_*` / `GIT_COMMITTER_*` / `EMAIL` from its own environment, which
   its git children inherit.
3. **Guessing.** With nothing configured git builds `user@host.fqdn`. Here the hostname has no
   domain (`omarchy`), so git refuses (exit 128). On a host with a domain the commit succeeds with
   an advice message, so "missing" must mean "no explicit source", not "`git var` failed".
4. **`user.useConfigOnly=true`** turns the guess off: with a name but no email `git var` exits 128.
5. **Placeholders pass.** `t` / `t@t` is accepted and produces `t <t@t>`. An empty name is refused.
   `<` and `>` in a name are silently stripped; a name with a newline is stored over two lines.
6. **Writing.** `git config --local|--global --replace-all -- user.name <value>` handles values that
   start with `-` and collapses duplicate values; plain `git config user.name -- -x` would add a
   second value. `--unset-all` clears (exit 5 when there was nothing). `--global` writes where git
   decides (`~/.gitconfig`, or `$XDG_CONFIG_HOME/git/config` when that exists and `~/.gitconfig`
   does not). `--local` in a linked worktree writes the shared `.git/config` (probed).
7. libgit2 is the wrong reader: it ignores `GIT_CONFIG_GLOBAL` and has no `includeIf`, environment or
   `EMAIL` view of the identity.

## Design, as built

- **Canopy stores no identity of its own.** It edits git's configuration, so what Canopy shows is
  what a terminal commit uses. Commits carry no `-c user.name=...` or `GIT_AUTHOR_*`. This fits the
  "Git settings" product rule: Canopy's choice and git's configuration are the same thing, so there
  is never a difference to report. It writes `user.name` and `user.email` only after the user
  presses a button, and never `author.*` or `committer.*`.
- **Core, `crates/canopy-core/src/identity.rs`** (no UI dependency): `Identity { author, committer }`,
  each a `Person { name, email, usable }` of `Field { value, source }`, where `Source` is
  `Environment(var)`, `Config { scope, origin }` or `Guessed`. `Identity::read(workdir)` runs one
  `git config --get-regexp` and one `git var` per person; `Identity::read_global()` (the first-launch
  view, no repository) reads global configuration only and has no guess. `Identity::warning()` is
  `Some` when git cannot use the identity, when part of it was guessed, or when it looks like a
  placeholder (`looks_like_placeholder`: name shorter than two characters, or an email domain with no
  dot, ending in `(none)`, `localhost` or `localdomain`; `example.com` is not flagged). `set` and
  `clear` take a `Scope` (`Local`, `Global`); `set` trims and refuses an empty value, control
  characters, `<`, `>`, and an email without an `@` and a domain, before git runs. Git calls with no
  repository run from the temporary directory, so a global-only view never depends on the
  repository Canopy was started in (`includeIf "gitdir:"` conditions look at it). An empty `EMAIL`
  variable is ignored, as git ignores it.
- **Two-step save.** `set` writes `user.name` and then `user.email` with two git calls. If the
  second fails (for example `.git/config.lock` appears between them) the name is put back to
  exactly the values it had at that scope (`store`, `restore`; a test injects the failure), and the
  git error is the toast. Only if the put-back fails too does the message say the new name was
  saved.
- **Clear.** It removes the keys from one file only; a value that also comes from another file
  (`~/.config/git/config`, an included file) keeps winning. The worker therefore reads the identity
  again right after a clear, and the toast says what Git uses now, or, when the cleared scope still
  supplies a value, which file still sets it (`cleared_body`). The confirmation dialog says the
  same in advance.
- **Timeout.** Every git call of the identity code goes through `cli::output_within`, which kills
  git after 15 seconds and fails with "git … did not answer", so a repository whose configuration
  makes git block cannot stall the shared `git-identity` thread for longer than that. Killing and
  reaping happen on a detached thread, and writes get 60 seconds so a killed `git config` does not
  leave `config.lock` behind. A clear that fails after removing the name puts the name back like a
  save does.
- **Refused repositories.** `git var` treats a repository git refuses to read ("dubious
  ownership") as no repository and answers from global configuration. `Identity::read` therefore
  first runs `git rev-parse --git-dir` (`check_repository`); a refusal is a read error, so Changes
  says "Could not read Git identity." and Settings → Git shows git's own message. Tested with
  `GIT_TEST_ASSUME_DIFFERENT_OWNER=1`.
- **`cli::loggable`** hides the value after `user.name`, `user.email`, `author.*` and `committer.*`
  in `git config` calls, so a debug log shows `<redacted>` (test in `cli.rs`).
- **App thread.** `identity_worker.rs`: one `IdentityWorker` owned by `CanopyApp`, jobs `ReadRepo`,
  `ReadGlobal`, `Set`, `Clear`, events `Repo { path, result }`, `Global`, `Changed { repo, cleared,
  result }`. The UI thread never calls git. A `Changed` event, successful or not, re-reads every
  open tab and the global view, so the screen shows what is true on disk. Reads also happen when a
  tab opens or becomes active, when Settings opens, and when the window regains focus (no file
  watcher covers `.git/config`).
- **Settings → Git** (`views/settings_screen.rs`, `identity_group`): Name and Email rows whose help
  text is "In use now: value (source)", a Committer row only when it differs, "Save to" (this
  repository, all repositories; the default is this repository when one is open), Save identity
  (disabled until both fields have text) and Clear…, which opens `Dialog::ConfirmClearIdentity`.
  The fields start with chosen values only: a guessed value is never offered for saving.
- **Warning** (`views/changes.rs`, `composer`): a `warning` line with an "Edit identity" button above
  the commit button, in a footer anchored to the bottom so the commit button stays visible. The
  button stays enabled: when no identity exists git's own error remains the toast, as the backlog
  item said.
- **First-launch step**: `Dialog::Identity`, opened by `offer_identity_step` after the consent
  dialogs (not a new consent `Pending`), only when `Settings.identity_asked` is false, the global
  identity has a warning, and no other identity dialog is open. `identity_asked` is set when the
  dialog opens, so dismissing it any way (Cancel, Escape, a click outside) never brings it back; a
  global identity that is already sensible sets it silently. "Save globally" writes `--global`, and
  the dialog stays open and shows git's error if the write fails. Screenshots skip the step.
- **History**: the column header reads "AUTHOR".

## Decisions made (the three open questions)

1. A guessed identity counts as missing and warns: the commits would carry an address the user never
   chose.
2. Settings scope defaults to this repository when one is open, else global.
3. Signing (`user.signingkey`, `commit.gpgsign`) stays out; it belongs to the backlog's "Git
   settings" item.

No privacy-policy change: nothing leaves the machine. Crash reports carry panics only, so git's
stderr (which can quote `user@host`) cannot reach one; no `crash_reports::DATA_VERSION` change.
No new dependency, host or token.

## Verification

- Core tests against temporary repositories in an isolated environment
  (`crates/canopy-core/tests/operations.rs`): scope and replacement, clear, immutable stored
  authors after a configuration change, environment and `author.*` precedence, missing and
  unusable identities (`useConfigOnly`), values starting with `-`, duplicate values, refused input,
  `includeIf`, global write and clear without a repository, `EMAIL` set and empty. Mutations
  (dropping `--` and `--replace-all`; honouring an empty `EMAIL`) turned their tests red. Unit tests: `loggable`, the placeholder rule, the put-back after a failed second write, the ownership/unknown distinction, `output_within` killing a git that outlasts its limit.
- App tests: the worker answers each job in order and a refused save writes nothing; the Settings
  editor never offers a guess (a mutation turned that test red) and chooses the scope by whether a
  repository is open; source labels, including an unknown home directory; the table of the
  first-launch decision (`identity_step`); the Changes banner text (`identity_notice`); the toast and
  dialog handling of a finished save or clear (`changed_toast`, `cleared_body`,
  `settle_identity_dialog`).
- Security pass (focused, by the author of the change, before the independent review): option
  injection (values follow `--`), config injection (git escapes values; control characters are
  refused), logging (`<redacted>` after `user.*` in a debug log), error text (never echoes input),
  crash reports (panics only), credentials (none of these commands prompt). No findings.
- Independent review (`canopy-review`): no blocking findings; its LOW findings were fixed or are
  listed above.
- Driven in the running app (private Xvfb display, isolated HOME and `GIT_CONFIG_GLOBAL`, a fixture
  repository): the real first-launch path (terms, crash reports, identity dialog, Save globally
  wrote `~/.gitconfig`, no second prompt after a relaunch); the Changes warning for `t` / `t@t`;
  "Edit identity" opening Settings → Git; Save for this repository; Cancel then confirmed Clear
  (fell back to the global identity); a held `.git/config.lock` gave git's own error in a toast and
  wrote nothing; a UI commit stored the shown identity and History displayed it; a debug-level log
  held no name or email; `GIT_AUTHOR_NAME` in the environment was shown as the source. Screenshots
  of Changes, Settings → Git and the dialog in the dark and light themes were inspected.

## Unconfirmed

- Behaviour on a host whose hostname has a domain (a guessed identity succeeds); the code handles
  both and the test accepts either.
- Several re-reads queue behind a hung repository (each costs up to 15 seconds); they are not
  deduplicated. The Windows launcher (`cmd\git.exe`) may survive a kill; the timeout test is Unix-only.
- A separate `/security-review` of the `cli.rs` addition (`output_within`) was not run; the
  independent review read it.
- Real-GPU rendering and a native Wayland window: all runs used software rendering on Xvfb.
- The confirmation dialog was rendered in both themes (`--overlay clear-identity`); the first-launch
  dialog in the light theme was covered before its buttons moved to the shared helper.
