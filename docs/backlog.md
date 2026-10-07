# Backlog

Open tasks, in priority order: security and data safety first, then the owner's rules that are not
yet true, then correctness and the features people meet first, then polish. Take from the top of
**Now**. Delete a task when it is done and verified in the running app.

## Now

- [ ] Operation cleanup: audit failed, cancelled and interrupted operations for files, temporary
      directories and processes Canopy leaves on the user's PC; clean up only Canopy-owned
      artifacts, preserve user work and recoverable Git states, and verify failure paths in
      isolated fixtures leave no orphaned artifacts or processes.
- [ ] Crash reports: `crash_reports.rs` `redact_url_credentials` ends the user part at `?`/`#`,
      so `https://u:secret?x@host/r` keeps `secret` in a report; use `canopy_core::cli::userinfo`
      (git splits at the first `/` only). Its `TOKEN_PREFIXES` (11) lacks the six GitLab prefixes
      `http.rs` has (`glcbt-`, `glft-`, `glimt-`, `glagent-`, `glsoat-`, `glffct-`): share one list.
- [ ] Hosting errors: `http.rs` hides the client's own token in raw response bytes before JSON
      decoding and before `plain_text` strips hidden characters, so a server echo that is escaped
      or split by a zero-width character reaches the toast and the log; replace it on the decoded,
      cleaned text (`map_status`, GraphQL `errors[].message`).
- [ ] Argv rule: `ops.rs` `delete_remote_branch` runs `git push --progress <remote> --delete
      <branch>` without `--` (a server refname may start with `-`); add `--` there and in
      `push_tag`, `rename_branch`, `checkout_remote_branch`.
- [ ] Installed apps: `system_apps.rs` `Entry::command` keeps `--file=%f` as a literal word that
      `platform::with_path` then refuses, so such editors cannot launch; and `%%f` becomes the
      file placeholder. Settings help (`settings_screen.rs:3286,3298,3310`) should say `%f` must
      be a word of its own.
- [ ] Small fixes from the component library review: `repo_config.rs:136` drops a failed read of
      `remote.<name>.prune`; `views/history.rs:796` stacked doc comments; `valid_ref_name`
      (`dialogs/common.rs:17`) accepts `HEAD`; `dialogs/url.rs:104` shows the credentialed-host
      message under Project; the legal reader's scroll offset survives restarts (consent opens
      part-way down); toolbar/palette "Pop latest stash" still pops index 0, not the commit the
      tooltip named.
- [ ] `ops.rs:206` `url_has_credentials` needs `://`; check with a real git whether a one-slash
      helper URL (`http::https:/user:pw@host/r`) prints the password unredacted.
- [ ] Component library: triage the rest of its working record (kept outside the repository with
      the session's orchestration notes: the follow-up ledger, about 90 open lines, and the
      workers' deferred items and Docs notes) into lines here; the security pass, the last
      review and the UI round's leftovers are already the lines above.
- [ ] Component library docs pass: apply every worker's Docs notes and the docs review (RD-1 to
      RD-8, in the working record): `design.md` rewritten from the shipped kit
      and the Design record, features, CHANGELOG Unreleased, architecture, README and
      `tools/README.md` options, the `add-theme` and `screenshot` skills, ADR 0003/0004 wording.
- [ ] Component library final QA: re-render the full matrix (4 themes and High contrast, both styles
      and densities, 960x600 to 4K) and read every image; frame-capture the commit and branch clicks
      (O2); Motion Match system / Reduced / Full live; owner before | after page per theme.
- [ ] Accessibility: `accesskit_unix` 0.21.1 unwraps a failed AT-SPI registry call in a thread,
      so a missing accessibility bus writes a crash report; report upstream or guard it.
- [ ] Daylight: check the Changes scroll thumb against the light composer (reported near-black
      on the wave-2 palette); `docs/design.md` gives the action bar as 52/40 in Layout but
      58/46 in its table and `theme.rs`.
- [ ] Legal reader links: a link that starts at a line wrap gets the previous row's end as its
      rect (`legal.rs:348` `pos_from_cursor`), so hover, focus underline and click land wrong.
- [ ] Accessibility, kit: kit `Callout` speaks title and body twice and its icon as a
      private-use glyph; kit `Button` could name every disabled button's reason the way
      `commit_detail::say_unavailable` does.
- [ ] Toolbar maps a running or finished operation to its tool by `Op::label()` text
      (`toolbar.rs` `tool_of`) because `RepoTab::running` stores only the label; store the op kind.
- [ ] Keyboard focus: the palette never gives focus back after it closes (`app/mod.rs:964`
      calls `palette.show` only while open, so its close frame never runs); opening Settings
      does not move focus into it; the title bar's Tab order runs right to left and its search
      shows no focus ring; three panels' resize handles have no accessible name; Settings
      navigation and theme cards should use the kit `Roving` focus; `app/input.rs:20-22` blocks
      every bare key whenever any widget has focus, and list rows now hold focus.
- [ ] `app/mod.rs` should use `paint::pane_focus` instead of its own copy; `theme.rs`
      `clamp_backdrop` comment is stale.
- [ ] Toasts: with five unread errors, a new Success or Info toast evicts itself at once
      (`ui/toast.rs:205-211`), so "Pushed" or "Fetched" never shows; evict the oldest older
      non-error, or replace the oldest error. Smaller: the conflict indent cache keys on a
      pointer and never drops entries (`views/resolve.rs` `code_lines_at`); an arrow onto a
      disabled roving row sticks (`kit/row.rs`); `apply_repo_looks` reassigns the folder name
      every frame for a root path (`app/dispatch.rs:432`); `gl.rs:1` uses `allow`, not `expect`.
- [ ] Flaky test: canopy-core `checkout_request_leaves_a_real_remote_branch_of_the_same_name_alone`
      failed once, then passed three reruns; find the race.
- [ ] Repository config can run code: Canopy's git calls (`canopy_core::cli`, `cli.rs:52-100`)
      set no `core.fsmonitor` override, so a cloned repository's `.git/config` could name a
      command that git runs on `git status` (the GitSpawn class, September 2026). Pass
      `-c core.fsmonitor=false` on every call, add a poisoned-repository regression test, and
      review `core.sshCommand`, `diff.external` and `core.pager` in the same pass, then
      `/security-review`. Found by the AI and monetization research (5 Oct 2026); exploitability
      not yet demonstrated.
- [ ] Request content updates: thoroughly investigate stale pull/merge request content in the
      app across GitHub and GitLab; trace refreshes, caching, selection/tab changes, responses
      arriving out of order, and edits/actions; compare displayed content with host API data.
- [ ] Branch/worktree checkout: switching to a different branch or worktree should reuse the
      current repository tab rather than open a new one; verify both paths in the running app.
- [ ] Appearance default theme: remove the "Back to the default theme, Midnight" button and
      mark Midnight as "Default" on its theme selection card, independently of selection.
- [ ] Appearance theme selection: make the top-right checkmark clearly visible and distinct
      from unselected cards; verify selected states across themes and contrast settings.
- [ ] Appearance custom colors: add a color picker alongside the predefined custom-theme
      color choices; verify arbitrary colors apply and persist across restarts.
- [ ] Notifications settings: move notification preferences out of Appearance into their own
      Notifications tab; verify all existing preferences remain accessible and effective.
- [ ] Notification filters: audit every kind filter and fix ignored preferences; disabling
      "Finished operations" must suppress successful fetch notifications, including background
      fetches, while each other category follows its own configured filter.
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
      it now (`selfupdate/mod.rs` `integrate` and `install.sh` remove only a copy the receipt lists).
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
- [ ] Git settings: where the user's git configuration sets one of Canopy's git choices it
      decides (Follow my git config, on by default), else Canopy passes its recommended choice
      (AGENTS.md, Git settings; ADR 0012). Left: report a configuration value that goes against
      Canopy's recommendation in the notification center too (Settings shows it). Whether
      Canopy may override `commit.gpgsign` or `core.hooksPath` is the owner's call: ask first.
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
      The core (`flow.rs`) always finishes with `--no-ff` (AVH fast-forwards a single-commit
      feature), offers no rebase-before-finish or hotfix into an open release, and refuses on git
      older than 2.38.
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
- [ ] Log redaction: `canopy_core::cli::redact_url` removes credentials from the first URL in a
      text only, and it also runs on git's stderr and error text (`cli.rs` `record`, `worker.rs`),
      so `user:token@` in a second URL reaches the log; redact every URL, as
      `crash_reports::redact_url_credentials` does, with a test.

## Next

- [ ] `canopy --screenshot` run directly (not through `tools/screenshot.sh`) opens a real window on
      the desktop session and writes logs into the user's own data folder; isolation lives only in
      the script. Make the binary refuse `--screenshot` unless the isolated environment is set, or
      move the isolation into the binary.
- [ ] A project check for what clippy cannot enforce from the quality standard: numeric literals
      without a named constant or comment, and a header comment on every source file.
- [ ] Errors that drop their cause (found by `clippy::map_err_ignore`): the merged-branches base
      lookup (`merged.rs`) says "does not exist" for any libgit2 failure, and the hosting picture
      and profile lookups (`canopy-hosting` `avatars.rs`, `http.rs` `avatar_json`) replace the
      transport's error with a fixed message; carry the libgit2 or ureq message where it helps.
- [ ] `canopy_hosting::token_from_env` is deprecated (it ignores the host and can hand a token to
      the wrong server) and nothing calls it; remove it and its re-export.
- [ ] Crate package metadata: reconcile the legacy `0.2.1-beta.1` workspace version inherited
      by all three crates with numbered-build releases; document an internal version convention
      and align local lockfile entries and tool/doc assumptions without per-release version bumps.
- [ ] Request list check/review badges: bulk or bounded cached status loading through the
      existing host API; show unknown while unavailable, respect its off switch and consent.
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
- [ ] Three themes (Midnight, Daylight, Observatory) and High contrast on each.
- [ ] Partial clones on real `--filter=blob:none` and `--filter=tree:0` clones.
- [ ] Checks the development machine cannot make, left open by the visual polish review (PR #5,
      merged 4 Oct 2026): real hosting credentials and account persistence, external links and
      update downloads, hardware HiDPI, Wayland, screen readers, macOS and Windows. Do not report
      any of them as verified until someone has run them.

## Owner to-dos

Things only the owner can do; agents remind, never act.

- [ ] Decide D1-D7 of the AI, agent access and pricing research (5 Oct 2026): the private
      artifact "Canopy AI & Pricing" on claude.ai saves the choices with the page; the reports
      are kept outside the repository. Nothing there is built until decided; two of its findings
      also touch the Terms (lines 11 and 19 conflict with a paid tier).
- [ ] Sentry: turn off IP address storage, set 90-day retention.
- [ ] Legal: add the legal name and postal address to `docs/legal/privacy-policy.md`; confirm the
      Terms assumptions (the app is free; the governing law is the registered office's).
- [ ] Set the GitHub "About" description (suggested: "A fast, beautiful, Linux-first Git client
      written in Rust.").
- [ ] Signing, once macOS or Windows matter: an Apple Developer ID for notarisation and a Windows
      code-signing certificate.
- [ ] Landing site (a separate project): the install one-liners, `docs/legal/`, screenshots and
      `docs/features.md`, the README's system requirements, `docs/legal/third-party-notices.txt`.
