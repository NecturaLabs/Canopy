# Changelog

All notable changes to Canopy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/NecturaLabs/Canopy/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/NecturaLabs/Canopy/releases/tag/v0.1.0
