# Backlog

Open tasks, in priority order. Take from the top of **Now**. Delete a task when it is done and
verified in the running app.

## Now

- [ ] Build-number versions instead of semver: build N = `git rev-list --count` on main plus the
      short SHA ("build 412 (a1b2c3d)"), embedded at compile time; tags `build-N`, no version-bump
      commit; feed carries the integer build; updater and installers compare integers and treat
      an old semver as older. Releases stay manual (`workflow_dispatch` only). A partial attempt
      (installers, `build.rs`, test fixtures; not finished) is saved on the maintainer's machine as
      `canopy-build-versions-partial.patch` in the shared scratch folder.
- [ ] In-app update: today "Download" only opens the archive URL in a browser
      (`views/dialogs/mod.rs`, `updater.rs` never installs). Owner's design: a native updater for
      installer-managed installs (`.canopy-install` receipt whose `root=` holds the running exe):
      download over HTTPS (installer's URL rule), check the manifest's SHA-256, stage into
      `<root>/.staging.*` and swap with the installer's two-phase rollback, update the receipt,
      `VERSION`, `uninstall.sh` and the registered desktop entry/icon copies (uninstall needs them
      identical), take `.lock`, then offer Restart. Any other install runs the one-line installer
      with `--version X`. Both remove every old file and `.previous.*`/`.staging.*` leftover and
      never touch settings, tokens, logs or caches. Owner approval: `sha2` + tar/zip crates,
      privacy policy text; Windows needs the rename-running-exe swap.
- [ ] Git identity in Settings: show the effective `user.name` / `user.email` and where each comes
      from (local, global, env); edit them for this repository or globally through `git config`
      via `canopy_core::cli` on a worker `Op`; core tests against temporary repositories.
- [ ] Identity warning: a banner and a hint on the commit button when the identity is missing or
      looks like a placeholder (for example `t` / `t@t`, an email whose domain has no dot);
      git's own "who you are" error stays the toast when none is set.
- [ ] First-launch identity step: after the consent screen, a one-time prompt pre-filled from the
      current git config; skipped when a sensible identity is already configured. Canopy never
      writes `user.*` without the user confirming it.
- [ ] Partial clones: trusted locations. Downloads (which run the repository's own git config)
      only for repositories under folders trusted in Settings; compare canonicalized paths in the
      worker; a "Trust…" button offering the repository's folder or its parent; privacy text.
- [ ] Partial clones: remember a failed download until Retry (no retry on every refresh); a fetch
      timeout; a Retry for failed diffs; a "not downloaded" marker per file; offer to convert a
      `LegacyPartialClone`, behind a confirmation.
- [ ] Notification center: one place where Canopy reports what the user should know beyond a
      transient toast (a git setting Canopy overrode, failed background jobs, warnings, update
      and security notices): a persistent, dismissable list with unread count, severity, source
      and timestamp, actions such as "Open setting", kept across restarts; toasts link to it.
- [ ] Git settings: Canopy is the authority (AGENTS.md, Git settings). List every action whose
      result git configuration can change (`pull.rebase`/`pull.ff`, `merge.ff`, `push.default`,
      `fetch.prune`, `rebase.autoStash`, ...), pass Canopy's choice explicitly for each, and
      report every difference from their configuration in the notification center. Whether
      Canopy may override `commit.gpgsign` or `core.hooksPath` (signing, hooks) is the owner's
      call: ask before changing either.
- [ ] Pull strategy: the app is the authority. The chosen strategy is always passed to git, so
      the menu never says Merge while git rebases. The user's setting, read through `git config`
      in git's order (`branch.<name>.rebase`, then `pull.rebase`, `pull.ff`), becomes the menu's
      default selection, marked as coming from their git config; a pull that goes against it is
      reported in the notification center with the setting, its value and where it is set. Today
      `pull` checks only `pull.rebase`, so it overrides a `branch.<name>.rebase`.
- [ ] `cli::run_with_input`: show git's stderr, not "Broken pipe", when git exits early.
- [ ] Security review of the installer fixes (`85d2961`); review canopy-3's core when the app
      starts calling it.
- [ ] Wire canopy-2's core into the app: publish any branch, mode and type changes, collapsed
      untracked folders, ask before Stage all adds a nested repository; drop the compatibility
      wrappers.
- [ ] Persistent, secured storage: per-OS Canopy folders with a `.canopy-data` marker (the
      contract `install.sh --purge` relies on is in `packaging/README.md`), one-time migration,
      atomic debounced settings writes off the UI thread with a schema version and recovery,
      0700/0600, tokens in the OS keychain (`keyring` crate, approved by the owner; the 0600 file
      as fallback), a test that redirects never forward `Authorization`, an ADR, privacy text.
- [ ] Git LFS UI: indicator, pointer-only state, diffs, pull/fetch, track/untrack, locks, prompt
      when `git-lfs` is missing (the status fix needs `sha2`: owner's call).
- [ ] Feature-branch workflows UI: git-flow dialogs, branch folders by prefix, merged and
      squash-merged indicators, delete merged, "Update from base".
- [ ] .gitignore UI: ignore file/extension/folder/pattern, local exclude, stop tracking, show
      ignored, "why ignored", templates on init.
- [ ] Submodules UI: sidebar list, open in a tab, init/update/sync/add/remove, pointer diffs,
      recursion on pull, checkout and clone.
- [ ] Symlinks and type changes: show and diff them; never follow links; Windows
      `core.symlinks=false`.
- [ ] Missing tools: detect git, git-lfs, ssh and signing tools; show install steps per OS
      (Finder-launched macOS apps lack the shell `PATH`).
- [ ] Architecture: shared widgets, tab-state and palette tests,
      a lint against colours outside `theme.rs`, the merge dialog's per-frame branch list.
- [ ] Settings: every preference in the UI, consent review, search/reset/import/export,
      remappable keybindings (Git identity has its own entries above).
- [ ] Design, in order: colour tokens everywhere, typography and controls
      (`docs/specs/ui-spec.md`), motion (reduce-motion setting), responsive 960×600 to 4K, polish
      and a design review.
- [ ] Stress readiness: lazy background tabs, bounded threads, staggered auto-fetch, one shared
      watcher, frame-time and first-frame logging. Run the stress test itself
      (`docs/stress-test.md`) only when the owner asks.
- [ ] Accessibility: AccessKit names and roles, focus order, Orca on history, changes and composer.
- [ ] Docs: `docs/design.md` for the other five themes, fresh screenshots, a CONTRIBUTING review.
- [ ] `tests/safety.rs` non-UTF-8 folder test: when the filesystem refuses the name (NTFS under
      `TMPDIR`), fail with a message naming that cause instead of an `unwrap` panic.
- [ ] Verify the one-line installers on real machines after the first stable release.

## Next

- [ ] Interactive rebase editor.
- [ ] File history (following renames).
- [ ] Blame view.
- [ ] Reflog and undo.
- [ ] Search across history (pickaxe).
- [ ] Drag and drop branches to merge or rebase.
- [ ] Signature badges (GPG and SSH).
- [ ] Image diff.
- [ ] Worktrees.
- [ ] Commit templates and message linting.
- [ ] Performance budget tests on the `big` fixture.
- [ ] Incremental graph for large repositories.

## Later

- [ ] Linux arm64 and Intel macOS builds out of experimental.
- [ ] Signed and notarised macOS app, signed Windows binary.
- [ ] Flatpak, AUR, .deb/.rpm, AppImage.
- [ ] Custom themes.
- [ ] Patches (format-patch and apply).
- [ ] Bisect.
- [ ] Graph minimap.
- [ ] gitoxide for history walks.
- [ ] Localisation.

## Awaiting in-app verification (owner)

- [ ] Credential and SSH passphrase prompts through Canopy's askpass.
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
