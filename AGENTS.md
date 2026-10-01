# Canopy

A fast, beautiful, Linux-first Git client for Linux, macOS and Windows, written in Rust (edition
2024, MSRV 1.91). Two crates: `crates/canopy-core` is the UI-agnostic Git backend (libgit2 through
`git2` 0.21 built without HTTPS/SSH, for reads; every mutation runs the `git` CLI); `crates/canopy-app` is the GUI (`eframe`/`egui` 0.36, glow renderer), binary
`canopy`, app ID `io.necturalabs.Canopy`. Architecture is in `docs/architecture.md`, the visual
design system in `docs/design.md`, the feature matrix in `docs/features.md`.

**Start here: read `HANDOFF.md`.** It records the state of the work, the owner's requirements and
the prioritised queue of what to build next.

## Product rules

These come from the owner and override anything else in the repository.

- **Hosting**: GitHub and GitLab only (including self-hosted GitLab), plus local-only repositories
  with no remote at all. No other providers.
- **No external services**: no web APIs, backend, telemetry, update checks or avatar downloads.
  Canopy talks only to the local `git` executable and local files. Provider features are offline:
  recognise remote URLs locally and open web pages in the user's browser. Never put credentials
  into a URL Canopy builds or logs.
- **Toolkit**: egui stays (the owner considered React and chose egui).
- **Art**: placeholders are SVG files, to be replaced by art generated with Codex/OpenAI using the
  prompts in `docs/art-prompts.md`. Keep file names and view boxes stable when replacing.
- **Themes**: ship a handful of well-designed themes, all defined in `ui/theme.rs`, plus a settings
  screen covering every needed option. Every setting must change behaviour.
- **Keybindings** follow the spirit of Omarchy: keyboard-first, a launcher-style palette, a
  keybinding cheatsheet, vim motions and single-key lazygit-style actions. Never bind Super (it
  belongs to the window manager); bare keys never fire while a text field has focus.

## Commands

- Format: `cargo fmt --all`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Test: `cargo test --workspace` (one test: `cargo test -p canopy-core <name>`)
- Run: `cargo run -p canopy-app -- <repo>` (add `--release` for realistic speed)
- All checks, as CI runs them: `tools/check.sh`
- Demo repository: `tools/mkfixture.sh <dir> [showcase|conflict|linear|big]`
- Screenshot: `tools/screenshot.sh <repo> <out.png> [--light] [--view history|changes]`

CI (`.github/workflows/ci.yml`) runs fmt, clippy and tests (Linux, macOS, Windows) and
cargo-deny. Tests need `git` on `PATH` and a git identity; CI sets one. Linux and Windows jobs run
on the organisation's self-hosted runners (`.github/actions/setup-rust` gives each runner its own
toolchain); the organisation variable `USE_SELF_HOSTED` set to `0` moves them to GitHub-hosted ones.

## Workflow

- `docs/backlog.md` is the task board. Take entries from the top of "Now" down. Tick an entry only
  when the change is verified in the running app, with a screenshot taken through the
  `screenshot` skill, not when the code compiles.
- Any change to what the UI shows needs a screenshot check (dark and light where colours are
  involved). Unit tests cannot see layout or colour: look at the image with the Read tool.
- Adding a Git operation follows the `add-git-operation` skill end to end.
- Project agents in `.claude/agents/`: `canopy-review` (read-only diff review), `canopy-git`
  (Git semantics, canopy-core), `canopy-visual-qa` (headless rendering and visual defects),
  `canopy-ux-research` (how other clients solve a UX question).
- Run `tools/check.sh` before declaring work done.

## Conventions

- The UI thread never calls git, libgit2 or the filesystem for repository data. Add a `Job`
  variant in `crates/canopy-app/src/worker.rs`, handle it on the repository's worker thread and
  return a `Event`. Views render from tab state only.
- `canopy-core` has no UI dependencies. Every operation gets a test against a temporary
  repository (`tempfile`); compare behaviour with the `git` CLI when semantics are subtle.
- Colours, spacing and radii come only from `crates/canopy-app/src/ui/theme.rs`. No hex literals
  or `Color32::from_rgb` in views. Icons come from `egui-phosphor`.
- Destructive operations (discard, hard reset, delete branch or tag, drop stash, force push) go
  through a confirmation dialog. Never force-push without `--force-with-lease`.
- Network operations (clone, fetch, pull, push) and merge/rebase go through `cli.rs`, which sets
  `GIT_TERMINAL_PROMPT=0` so git never blocks waiting on a terminal.
- Errors reach the user as toasts carrying git's own stderr (or the libgit2 message), not a
  generic "operation failed". Log details with `tracing`.
- No `unwrap()` or `expect()` outside tests; propagate `canopy_core::Result` or handle the case.
  `unsafe` is forbidden workspace-wide.
- Paths are `Path`/`PathBuf`, joined with `join`, never by string concatenation with `/`; do not
  assume they are UTF-8.
- When a change adds or alters a feature, update `docs/features.md` and `CHANGELOG.md`
  (Unreleased) in the same change; when it adds or changes a module, update
  `docs/architecture.md`; when it changes a token or component, update `docs/design.md`.
- Fonts and other bundled assets need a licence row (`crates/canopy-app/assets/fonts/README.md`
  for fonts) and their licence text in the same change. Only OFL, MIT, Apache-2.0, CC0 or
  similarly permissive assets.
- Commit messages follow Conventional Commits (`feat(graph): ...`, `fix(worker): ...`).

## Boundaries

- Never commit credentials, tokens or personal repository paths.
- Never run destructive git commands (reset, checkout, clean, push, branch -D, stash drop)
  against the user's real repositories, in tests or by hand. Use temporary directories and
  `tools/mkfixture.sh`.
- Do not weaken or delete tests to make a change pass.
- This repository is proprietary (NecturaLabs). Do not copy code from GPL-licensed Git clients.
