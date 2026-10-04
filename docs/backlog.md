# Backlog

Open tasks, in priority order: security and data safety first, then the owner's rules that are not
yet true, then correctness and the features people meet first, then polish. Take from the top of
**Now**. Delete a task when it is done and verified in the running app.

## Now

- [ ] Partial clones: remember a failed download until Retry (no retry on every refresh); a fetch
      timeout; a Retry for failed diffs; a "not downloaded" marker per file; offer to convert a
      `LegacyPartialClone`, behind a confirmation.
- [ ] Trusted folders (ADR 0009): a linked worktree needs its own folder and the folder of its main
      repository trusted, but the Trust… dialog offers only the opened folder and its parent
      (pass the common git directory with `Event::DownloadsRefused` to offer both). Check on macOS
      and Windows that paths compare by the same spelling on a case-insensitive volume. From the
      ninth external review, not yet fixed: the gate checks the common git directory and the
      working folder but not a linked worktree's own git directory (`repo.path()`), which git
      reads `config.worktree` from under `extensions.worktreeConfig`; add it to `download_allowed`
      and `is_in_trusted_folder` (a crafted `.git` file would have to sit in the trusted folder
      already, but the gate should cover what git reads). The Downloads off chip also stays after
      switching from History to Changes while line counts are undone, and after a status-only
      refusal that ends in `Event::Failed` (`tab.rs`, `clear_obsolete_refusal`).
- [ ] App icon: an update made by build 192 or older drops the scalable SVG it registered from the
      receipt but leaves the copy in `~/.local/share/icons/hicolor/scalable/apps`; nothing removes
      it now (`selfupdate.rs` `integrate` and `install.sh` remove only a copy the receipt lists).
      Both check that the copy is a plain file and then remove it: a process of the same user
      could swap in a symlink between the two, which would then be removed (never its target).
- [ ] Installer follow-ups from the security review (`install.sh` is fixed; `install.ps1` could not
      be tested here and is untouched, and nothing in CI runs it): bring `install.ps1` to parity
      (an upgrade must move only the installer's own entries aside, around lines 495 and 649; an
      uninstall must act only on the entries it creates; its purge text, lines 416-417, still says
      tokens are in the keychain, but they are `tokens.json` in the data folder); write the
      receipt before `SWAP_PHASE` is cleared, so a first install interrupted after the swap does
      not leave files that both install and uninstall then refuse; clean up `.previous.*`
      leftovers on install; check the saved `uninstall.sh` against a checksum; refuse archive
      members that are symlinks. Review canopy-3's core when the app starts calling it.
- [ ] Persistent, secured storage: per-OS Canopy folders with a `.canopy-data` marker (the
      contract `install.sh --purge` relies on is in `packaging/README.md`), one-time migration,
      atomic debounced settings writes off the UI thread with a schema version and recovery,
      0700/0600, tokens in the OS keychain (`keyring` crate, approved by the owner; the 0600 file
      as fallback), a test that redirects never forward `Authorization`, an ADR, privacy text.
- [ ] Notification center: one place where Canopy reports what the user should know beyond a
      transient toast (a git setting Canopy overrode, failed background jobs, warnings, update
      and security notices): a persistent, dismissable list with unread count, severity, source
      and timestamp, actions such as "Open setting", kept across restarts. Replace bottom-right
      toast notifications with this center; let users choose notification kinds in Settings.
- [ ] Pull strategy: the app is the authority. The chosen strategy is always passed to git, so
      the menu never says Merge while git rebases. The user's setting, read through `git config`
      in git's order (`branch.<name>.rebase`, then `pull.rebase`, `pull.ff`), becomes the menu's
      default selection, marked as coming from their git config; a pull that goes against it is
      reported in the notification center with the setting, its value and where it is set. Today
      `pull` checks only `pull.rebase`, so it overrides a `branch.<name>.rebase`.
- [ ] Git settings: Canopy is the authority (AGENTS.md, Git settings). List every action whose
      result git configuration can change (`pull.rebase`/`pull.ff`, `merge.ff`, `push.default`,
      `fetch.prune`, `rebase.autoStash`, ...), pass Canopy's choice explicitly for each, and
      report every difference from their configuration in the notification center. Whether
      Canopy may override `commit.gpgsign` or `core.hooksPath` (signing, hooks) is the owner's
      call: ask before changing either.
- [ ] Wire canopy-2's core into the app: publish any branch, mode and type changes, collapsed
      untracked folders, ask before Stage all adds a nested repository; drop the compatibility
      wrappers.
- [ ] Symlinks and type changes: show and diff them; never follow links; Windows
      `core.symlinks=false`.
- [ ] Missing tools: detect git, git-lfs, ssh and signing tools; show install steps per OS
      (Finder-launched macOS apps lack the shell `PATH`).
- [ ] Settings: every preference in the UI, consent review, search/reset/import/export,
      remappable keybindings.
- [ ] Repository configuration: change the remote URL or repository, rename the repository tab,
      and choose its icon.
- [ ] Git LFS UI: indicator, pointer-only state, diffs, pull/fetch, track/untrack, locks, prompt
      when `git-lfs` is missing (the status fix needs `sha2`: owner's call).
- [ ] Feature-branch workflows UI: git-flow dialogs, branch folders by prefix, merged and
      squash-merged indicators, delete merged, "Update from base".
- [ ] .gitignore UI: ignore file/extension/folder/pattern, local exclude, stop tracking, show
      ignored, "why ignored", templates on init.
- [ ] Submodules UI: sidebar list, open in a tab, init/update/sync/add/remove, pointer diffs,
      recursion on pull, checkout and clone.
- [ ] In-app update, what is left: a native macOS bundle swap and a Windows folder swap with the
      running executable renamed aside (both run the installer today). Update and Restart are
      verified in the running app on a private Xvfb display (3 Oct 2026, `docs/playbook.md`).
- [ ] Stress readiness: lazy background tabs, bounded threads, staggered auto-fetch, one shared
      watcher, frame-time and first-frame logging. Run the stress test itself
      (`docs/stress-test.md`) only when the owner asks.
- [ ] Accessibility: AccessKit names and roles, focus order, Orca on history, changes and composer.
- [ ] Architecture: shared widgets, tab-state and palette tests,
      a lint against colours outside `theme.rs`.
- [ ] Design, in order: colour tokens everywhere, typography and controls
      (`docs/specs/ui-spec.md`), motion (reduce-motion setting), responsive 960×600 to 4K, polish
      and a design review.
- [ ] Motion: smooth real-time animations as repository activity changes the UI, respecting
      the reduce-motion setting.
- [ ] Operation feedback: animate button icons during operations and their completion,
      respecting the reduce-motion setting.
- [ ] Docs: a CONTRIBUTING review.
- [ ] `tests/safety.rs` non-UTF-8 folder test: when the filesystem refuses the name (NTFS under
      `TMPDIR`), fail with a message naming that cause instead of an `unwrap` panic.
- [ ] Verify the one-line installers on real machines after the first stable release.

## Next

- [ ] Reflog and operation-specific undo: show the recoverable action and its effect before
      applying it; never imply discarded unstaged or untracked content can be restored.
- [ ] Interactive rebase editor.
- [ ] Repositories of connected accounts: list the repositories of the GitHub or GitLab account a
      token belongs to (own, organisations, optionally starred; searchable, paged, with the
      visibility and fork markers) in the start screen and clone one into the default folder with
      the account's protocol. New data category: ask the owner first (`hosting-api` skill), then an
      ADR, privacy text and an off switch.
- [ ] File history (following renames).
- [ ] Blame view.
- [ ] Search across history (pickaxe).
- [ ] Performance budget tests on the `big` fixture.
- [ ] Incremental graph for large repositories.
- [ ] Create a repository on GitHub or GitLab from Canopy: name, description, visibility, owner or
      organisation or group, and optionally the first push of a local-only repository (joins
      Publish). Confirmation dialog, token scope explained, errors as git or the server states them.
- [ ] Repository details from the host, beside the sidebar: about text, topics, visibility
      (public, private, internal), fork-of link, stars, forks, watchers, open issues and pull
      requests, default branch, licence, main language, last push. Cached like pictures, behind
      the same switches, nothing sent without the hosting API on.
- [ ] Repository and branch metrics, local first (libgit2, no network, no privacy change): commits
      per week for the last year, top authors, last commit, repository size and object count;
      per branch ahead/behind the default branch and its upstream, unique commits, last commit
      author and date, age and a stale mark, merged or squash-merged state, and CI status where the
      host provides it (check runs are already read for requests).
- [ ] Signature badges (GPG and SSH).
- [ ] Worktrees.
- [ ] Stale worktree and branch cleanup: configurable inactivity period, preview candidates and
      confirm removal; protect uncommitted work and branches with unmerged commits.
- [ ] Commit templates and message linting.
- [ ] Image diff.
- [ ] Drag and drop branches to merge or rebase.

## Later

- [ ] Flatpak, AUR, .deb/.rpm, AppImage.
- [ ] Linux arm64 and Intel macOS builds out of experimental.
- [ ] Signed and notarised macOS app, signed Windows binary.
- [ ] Custom themes.
- [ ] Patches (format-patch and apply).
- [ ] Bisect.
- [ ] Graph minimap.
- [ ] gitoxide for history walks.
- [ ] Localisation.

## Awaiting in-app verification (owner)

- [ ] Credential and SSH passphrase prompts through Canopy's askpass.
- [ ] Author pictures on a long history against `tools/mock-hosting.py`: lookups on their own
      thread (a pull request action never waits for one), the hourly budget (30 anonymous),
      eviction past 256 authors, and cache files older than 30 days pruned. Unit-tested only.
- [ ] Ahead/behind counts in tab headers.
- [ ] Branch delete with a forced delete only when Git refuses.
- [ ] Local-only badge and Publish; "Open on" links.
- [ ] Settings screen, keymap cheatsheet, update check and "What's new".
- [ ] Merge modes.
- [ ] Line-level stage, unstage and discard.
- [ ] Logs and opt-in crash reports.
- [ ] Diff settings, syntax highlighting, side-by-side diffs.
- [ ] Three-way conflict resolver.
- [ ] GitHub and GitLab pull and merge requests.
- [ ] Seven themes.
- [ ] Partial clones on real `--filter=blob:none` and `--filter=tree:0` clones.

## Owner to-dos

Things only the owner can do; agents remind, never act.

- [ ] Sentry: turn off IP address storage, set 90-day retention.
- [ ] Legal: add the legal name and postal address to `docs/legal/privacy-policy.md`; confirm the
      Terms assumptions (the app is free; the governing law is the registered office's).
- [ ] Set the GitHub "About" description (suggested: "A fast, beautiful, Linux-first Git client
      written in Rust.").
- [ ] Signing, once macOS or Windows matter: an Apple Developer ID for notarisation and a Windows
      code-signing certificate.
- [ ] Landing site (a separate project): the install one-liners, `docs/legal/`, screenshots and
      `docs/features.md`, the README's system requirements, `docs/legal/third-party-notices.txt`.
