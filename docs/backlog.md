# Backlog

Open tasks, in priority order. Take from the top of **Now**. Delete a task when it is done and
verified in the running app.

## Now

- [ ] Partial clones: trusted locations. Downloads (which run the repository's own git config)
      only for repositories under folders trusted in Settings; compare canonicalized paths in the
      worker; a "Trust…" button offering the repository's folder or its parent; privacy text.
- [ ] Partial clones: remember a failed download until Retry (no retry on every refresh); a fetch
      timeout; a Retry for failed diffs; a "not downloaded" marker per file.
- [ ] `cli::run_with_input`: show git's stderr, not "Broken pipe", when git exits early.
- [ ] `Repo::pull` reads `pull.rebase` through libgit2; read it through `git config`, and isolate
      the pull tests from the developer's git config.
- [ ] `packaging/test-install.sh`: the two "interrupted install exits 130" checks fail when
      `/bin/sh` is bash.
- [ ] Security review of the installer fixes (`85d2961`); review canopy-3's core when the app
      starts calling it.
- [ ] Wire canopy-2's core into the app: publish any branch, mode and type changes, collapsed
      untracked folders, ask before Stage all adds a nested repository; drop the compatibility
      wrappers.
- [ ] Persistent, secured storage: per-OS Canopy folders with a `.canopy-data` marker, one-time
      migration, atomic settings writes with recovery, 0700/0600, tokens in the OS keychain.
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
- [ ] Missing tools: detect git, git-lfs, ssh and signing tools; show install steps per OS.
- [ ] Architecture: shared widgets, file dialogs off the UI thread, tab-state and palette tests,
      a lint against colours outside `theme.rs`, the merge dialog's per-frame branch list.
- [ ] Settings: every preference in the UI, consent review, search/reset/import/export, Git
      identity, remappable keybindings.
- [ ] Design: colour tokens everywhere, typography tokens, controls, motion (reduce-motion
      setting), responsive 960×600 to 4K, polish and a design review.
- [ ] Stress readiness: lazy background tabs, bounded threads, staggered auto-fetch, one shared
      watcher, frame-time logging.
- [ ] Accessibility: AccessKit names and roles, focus order, Orca on history, changes and composer.
- [ ] Docs: `docs/design.md` for the other five themes, CONTRIBUTING, fresh screenshots, retire
      HANDOFF.md.
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
