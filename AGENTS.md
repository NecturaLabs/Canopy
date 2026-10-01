# Canopy

A fast, beautiful, Linux-first Git client for Linux, macOS and Windows, written in Rust (edition
2024, MSRV 1.91). Three crates: `crates/canopy-core`, the UI-agnostic Git backend (libgit2 via
`git2`, built without HTTPS/SSH, for reads; the `git` CLI for every mutation);
`crates/canopy-hosting`, the GitHub/GitLab REST client (`ureq`, rustls); `crates/canopy-app`, the
GUI (`eframe`/`egui` 0.36, glow), binary `canopy`. Docs: `docs/architecture.md`, decisions in
`docs/adr/`, `docs/design.md`, `docs/features.md`.

**Then read `HANDOFF.md`** for the queue. It is a session note and can lag: where it disagrees
with this file or the code, they win; fix the note (`memory-hygiene`).

## Product rules

These come from the owner and override anything else in the repository.

- **Hosting**: GitHub and GitLab only (including self-hosted GitLab), plus local-only repositories
  with no remote at all.
- **Network**: Canopy talks to exactly four things: the local `git` executable (all Git
  traffic); the REST API of a repository's own GitHub or GitLab host for pull/merge requests,
  with a user-supplied token (`crates/canopy-hosting`); Canopy's update check (`updater.rs`);
  opt-in, scrubbed crash reports to Sentry (`crash_reports.rs`). The last three have an off
  switch and send nothing before the user accepted the Terms and Privacy Policy and answered the
  crash question. No other telemetry, analytics, avatar or image downloads, backend or service.
- **Credentials**: never put credentials into a URL Canopy builds or logs. A hosting token goes
  only to its own host. Git and ssh prompts go through Canopy's own askpass
  (`crates/canopy-app/src/askpass.rs`), never a terminal.
- **Legal and data**: `docs/legal/privacy-policy.md` and `terms-of-service.md` state what leaves
  the machine. A change to that updates them and their `Last updated` date (matching
  `ui::legal::PRIVACY_VERSION`/`TERMS_VERSION`); one to what a crash report carries also bumps
  `crash_reports::DATA_VERSION`. After a dependency change run `python3 tools/notices.py` (writes
  `docs/legal/third-party-notices.txt`; CI checks it).
- **Logging**: JSON-lines logs per run (`logging.rs`). Never log secrets or prompt answers; pass
  URLs through `canopy_core::cli::redact_url`, git arguments through `cli::loggable`.
- **Toolkit**: egui stays (the owner chose it over React).
- **Art**: SVG placeholders until Codex/OpenAI art (`docs/art-prompts.md`); keep file names and
  view boxes stable.
- **Themes**: a handful of well-designed themes, all in `ui/theme.rs` (`add-theme` skill), and a
  settings screen covering every needed option. Every setting must change behaviour.
- **Keybindings** follow the spirit of Omarchy: keyboard-first, a launcher-style palette, a
  cheatsheet, vim motions and single-key lazygit-style actions, from one table in `keymap.rs`
  (`add-keybinding` skill). Never bind Super (the window manager's); bare keys never fire while a
  text field has focus.

## Security-sensitive areas

`askpass.rs`; saved tokens (`tokens.json`, `hosting.rs`) and all of `crates/canopy-hosting/`;
crash-report scrubbing (`crash_reports.rs`); URL opening in `platform.rs` (web addresses only);
log redaction (`logging.rs`, `cli.rs`); URL building in `forge.rs`; the update feed; `Cargo.toml`,
`Cargo.lock`, `deny.toml`; `.github/` (self-hosted runners run code). A change there gets a
security pass (`/security-review`) before the general review. Ask the owner before storing a
token anywhere new, contacting a new host or adding a dependency.

## Commands

- Format: `cargo fmt --all`
- Lint: `cargo clippy --workspace --all-targets --locked -- -D warnings`
- Test: `cargo test --workspace --locked` (one: `cargo test -p canopy-core <name>`)
- Run: `cargo run -p canopy-app -- <repo>` (`--release` for realistic speed)
- `tools/check.sh`: `fmt --check`, clippy, workspace tests, `tools/release.py selftest` and
  `tools/notices.py --check`, on this platform. CI (`ci.yml`) adds `cargo deny check` (run it when
  `Cargo.toml`, `Cargo.lock` or `deny.toml` change) and tests on Linux (macOS and Windows jobs are off until the repository variable
  `CI_MACOS_WINDOWS` is set to `1`).
- Demo repository: `tools/mkfixture.sh <dir> [kind]` (`fixture-repo` skill)
- Screenshot: `tools/screenshot.sh <repo> <out.png> [--light|--theme NAME|--view V|--overlay NAME]`
  (`-h` lists the options)

## Workflow

- `docs/backlog.md` is the task board: take entries from the top of "Now" down, and tick one only
  when verified in the running app, not when the code compiles.
- Any change to what the UI shows needs a screenshot check (`screenshot` skill), dark and light
  where colours are involved; unit tests cannot see layout or colour.
- Recipes to follow end to end: `add-git-operation`, `hosting-api`, `add-theme`, `add-keybinding`.
- Project agents in `.claude/agents/`: `canopy-review`, `canopy-git`, `canopy-visual-qa`,
  `canopy-ux-research`. Model tier: `opus` for costly, security-sensitive or cross-cutting work,
  `sonnet` otherwise.
- Before declaring work done: `tools/check.sh` passes and the `canopy-review` agent has reviewed
  the diff (brief it per the `independent-review` skill); fix or answer every Blocking finding.
  Only a typo or formatting-only change skips this.

## Conventions

- The UI thread never calls git, libgit2, the hosting API, the network or the filesystem for
  repository data. Add a `Job` or `Op` variant in `crates/canopy-app/src/worker.rs` (hosting: a `HostJob`
  in `hosting.rs`), handle it off the UI thread and return an `Event`. Views render from tab state
  only.
- `canopy-core` has no UI dependencies. Every operation gets a test against a temporary
  repository (`tempfile`), compared with the `git` CLI where semantics are subtle.
- Every repository mutation and all Git network traffic run the `git` executable through
  `canopy_core::cli`, which sets `GIT_TERMINAL_PROMPT=0` and `GIT_EDITOR=true`. Arguments are
  separate argv entries, with `--` before user-supplied paths and refs. HTTP happens only in
  `crates/canopy-hosting` (plus Sentry's transport).
- Colours, spacing and radii come only from `crates/canopy-app/src/ui/theme.rs`. No hex literals
  or `Color32::from_rgb` in views. Icons come from `egui-phosphor`.
- Destructive operations (discard, hard reset, delete branch or tag, drop stash, force push) go
  through a confirmation dialog. Never force-push without `--force-with-lease`.
- Errors reach the user as toasts carrying git's own stderr (or the libgit2 or server message),
  never a generic "operation failed"; log details with `tracing`.
- Propagate `canopy_core::Result`; clippy rejects `unwrap`/`expect` outside tests; no `unsafe`.
- Paths are `Path`/`PathBuf`, joined with `join`, never concatenated with `/`; not assumed UTF-8.
- In the same change: a feature updates `docs/features.md` and `CHANGELOG.md` (Unreleased); a
  module, `docs/architecture.md`; a token or component, `docs/design.md`. A consequential,
  hard-to-reverse decision gets an ADR in `docs/adr/` (`project-docs` skill).
- Bundled fonts and assets need a licence row (fonts: `crates/canopy-app/assets/fonts/README.md`)
  and text in the same change; only OFL, MIT, Apache-2.0, CC0 or similar.
- Commit messages follow Conventional Commits (`feat(graph): ...`); the type decides the release
  bump (`tools/release.py`). Never bump versions or tag by hand: releases come from the manual
  "Create release" workflow (`release` skill).

## Boundaries

- Never commit credentials, tokens or personal repository paths.
- Never run destructive git commands (reset, checkout, clean, push, branch -D, stash drop)
  against the user's real repositories, in tests or by hand. Use temporary directories and
  `tools/mkfixture.sh`.
- Do not weaken or delete tests to make a change pass.
- This repository is proprietary (NecturaLabs). Do not copy code from GPL-licensed Git clients.
