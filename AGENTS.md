# Canopy

A fast, beautiful, Linux-first Git client for Linux, macOS and Windows, written in Rust (edition
2024, MSRV 1.91). Three crates: `crates/canopy-core`, the UI-agnostic Git backend (libgit2 via
`git2`, built without HTTPS/SSH, for reads; the `git` CLI for every mutation);
`crates/canopy-hosting`, the GitHub/GitLab REST client (`ureq`, rustls); `crates/canopy-app`, the
GUI (`eframe`/`egui` 0.36, glow), binary `canopy`. Docs: `docs/README.md` maps them (architecture,
decisions in `docs/adr/`, design, features, playbook) and says which can lag.

## Product rules

These come from the owner and override anything else in the repository.

- **Hosting**: GitHub and GitLab only (including self-hosted GitLab), plus local-only repositories
  with no remote at all.
- **Network**: Canopy contacts the local `git` executable for Git traffic; the repository's own
  GitHub or GitLab REST API for pull/merge requests and author profile lookup; profile pictures
  on that host, GitHub's `avatars.githubusercontent.com`, and exact Gravatar/Libravatar image
  hosts returned by GitLab (`canopy-hosting::avatars`, ADR 0008); Canopy's update check and its
  download only when the user chooses Update; opt-in, scrubbed crash reports to Sentry.
  Every HTTP feature has an off switch and sends nothing before the user accepted the Terms and
  Privacy Policy and answered the crash question. Pictures are cached locally for 12 hours,
  including failures; a failed refresh keeps the previous picture. Image requests carry no token
  and follow no redirects. No other telemetry, analytics, backend or service.
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
- **Git settings**: Canopy is the authority on what it does to a repository. Every action runs
  with the option chosen in Canopy, passed to git explicitly, even where the user's git
  configuration says otherwise; that configuration is the default Canopy's choices start from.
  Whenever Canopy's choice differs from the user's git configuration, the UI says so: which
  setting, its value and where it is set. Not yet true: `pull` still defers to the user's
  `pull.rebase`, and nothing reports differences yet (`docs/backlog.md`, "Git settings" and
  "Pull strategy").
- **Toolkit**: egui stays (the owner chose it over React).
- **Art**: in-app illustrations are drawn in code from the active theme (`ui/art.rs`,
  `docs/art-prompts.md`), never shipped as SVG; the app icon is `assets/icon.svg` and its PNGs.
- **Themes**: a handful of well-designed themes, all in `ui/theme.rs` (`add-theme` skill), and a
  settings screen covering every needed option. Every setting must change behaviour.
- **Keybindings** follow the spirit of Omarchy: keyboard-first, a launcher-style palette, a
  cheatsheet, vim motions and single-key lazygit-style actions, from one table in `keymap.rs`
  (`add-keybinding` skill). Never bind Super (the window manager's); bare keys never fire while a
  text field has focus.

## Security-sensitive areas

`vendor/` (third-party code we build, with our patch); `askpass.rs`; saved tokens (`tokens.json`,
`hosting.rs`) and all of `crates/canopy-hosting/`; crash-report scrubbing (`crash_reports.rs`); URL
opening in `platform.rs` (web addresses only); log redaction (`logging.rs`, `cli.rs`); URL building
in `forge.rs`; the update feed; `Cargo.toml`,
`Cargo.lock`, `deny.toml`; `.github/` (self-hosted runners run code). A change there gets a
security pass (`/security-review`) before the general review. Ask the owner before storing a
token anywhere new, contacting a new host or adding a dependency.

## Commands

- Format: `cargo fmt --all`
- Lint: `cargo clippy --workspace --all-targets --locked -- -D warnings`
- Test: `cargo test --workspace --locked` (one: `cargo test -p canopy-core <name>`)
- Run: `cargo run -p canopy-app -- <repo>` (`--release` for realistic speed)
- `tools/check.sh`, on this platform: `fmt --check`, clippy, workspace tests,
  `tools/release.py selftest`, `tools/notices.py --check`, `tools/check-docs.py` (the docs map,
  paths in docs, module table and skill mirror), `tools/vendor-eframe.sh --check` (`vendor/eframe`
  is the published crate plus our patch and the build uses it; needs network access),
  `tools/stall-check.py` (a hidden window must keep answering its compositor; Linux, skipped
  without a C compiler, `pkg-config` and `libwayland-server`), `packaging/test-install.sh` on Linux
  and macOS,
  and `shellcheck` and `cargo deny check` only when installed (CI runs ShellCheck and the vendor
  check on every push, `stall-check.py` in its Linux test job (required on GitHub-hosted runners,
  advisory on the self-hosted one) and `cargo deny check` whenever a file outside the docs changes; run `cargo deny
  check` yourself when `Cargo.toml`, `Cargo.lock` or `deny.toml` change). CI (`ci.yml`) adds
  tests per platform, one repository variable each: Linux runs unless `CI_LINUX`
  is `0`; macOS and Windows run only when `CI_MACOS` / `CI_WINDOWS` is `1` (off for now).
- Demo repository: `tools/mkfixture.sh <dir> [kind]` (`fixture-repo` skill)
- Screenshot: `tools/screenshot.sh <repo> <out.png> [--light|--theme NAME|--view V|--overlay NAME]`
  (`-h` lists the options)

## Workflow

- `docs/backlog.md` is the task board, one short line per open task: take them from the top of
  "Now" down, and delete one only when verified in the running app, not when the code compiles.
- Any change to what the UI shows needs a screenshot check (`screenshot` skill), dark and light
  where colours are involved; unit tests cannot see layout or colour.
- `docs/playbook.md`: running the app live on a private Xvfb display, testing an update end to end,
  releasing and the app-updates feed. Read it before any of those.
- Recipes to follow end to end: `add-git-operation`, `hosting-api`, `add-theme`, `add-keybinding`.
- The owner has given full authority: ask only for decisions that are truly theirs
  (destructive and outward-facing actions still need the owner).
- Project agents in `.claude/agents/` (Codex: `.codex/agents/`): `canopy-review`, `canopy-git`,
  `canopy-visual-qa`, `canopy-ux-research`.
- Subagents (the owner's rule): at most 3 at a time. Small things are done directly. Each builder
  gets its own `CARGO_TARGET_DIR` in a scratch directory, deleted afterwards; keep 5 GB of disk
  free.
- Branches: no pull requests and no long-lived branches. Small fixes go straight to `main`; larger
  work on a short branch merged with `git merge --no-ff` or a fast-forward. Push `main` when the
  owner asks. `tools/check.sh` must pass before merging; do not wait for CI. Where a check fails
  for a reason already in `docs/backlog.md`, report the actual run.
- Before declaring work done: `tools/check.sh` passes and the `canopy-review` agent has reviewed
  the diff (brief it per the `independent-review` skill); fix or answer every Blocking finding.
  Only a typo or formatting-only change skips this.

## Conventions

- The UI thread never calls git, libgit2, the hosting API, the network or the filesystem for
  repository data. Add a `Job` or `Op` variant in `crates/canopy-app/src/worker.rs` (hosting: a
  `HostJob` in `hosting.rs`), handle it off the UI thread and return an `Event`. Views render from
  tab state only.
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
- In the same change: a feature updates `docs/features.md` and, until the first numbered build is released,
  `CHANGELOG.md` (Unreleased; afterwards the commit subject is the release note); a
  module, `docs/architecture.md`; a token or component, `docs/design.md`. A consequential,
  hard-to-reverse decision gets an ADR in `docs/adr/` (`project-docs` skill).
- Bundled fonts and assets need a licence row (fonts: `crates/canopy-app/assets/fonts/README.md`)
  and text in the same change; only OFL, MIT, Apache-2.0, CC0 or similar.
- Commit messages follow Conventional Commits (`feat(graph): ...`); a `feat`, `fix`, `perf`,
  `revert` or `build(deps)` commit makes a build releasable and its subject becomes the release
  note (`tools/release.py`), so write it for users. A release is a numbered build (`git rev-list
  --count` on `main`): there is no version to bump. Never tag by hand: releases come from the
  manual "Create release" workflow (`release` skill). No model attribution in commits or files.

## Boundaries

- Never commit credentials, tokens or personal repository paths.
- Never run destructive git commands (reset, checkout, clean, push, branch -D, stash drop)
  against the user's real repositories, in tests or by hand. Use temporary directories and
  `tools/mkfixture.sh`.
- Do not weaken or delete tests to make a change pass.
- This repository is proprietary (NecturaLabs). Do not copy code from GPL-licensed Git clients.
