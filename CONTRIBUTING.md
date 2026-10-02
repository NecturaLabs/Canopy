# Contributing

Canopy is a private NecturaLabs repository. These notes apply to everyone working in it, human or
agent; [AGENTS.md](AGENTS.md) has the full conventions.

## Setup

1. Install Rust with [rustup](https://rustup.rs); `rust-toolchain.toml` selects stable with
   rustfmt and clippy. The minimum supported Rust version is 1.91.
2. Install `git` 2.30 or newer and, on Linux, the runtime libraries listed in the
   [README](README.md#install-from-source).
3. Optional: `cargo install cargo-deny` to run the licence and advisory checks locally, and
   `xvfb` to take headless screenshots.

```sh
tools/mkfixture.sh /tmp/canopy-demo
cargo run -p canopy-app -- /tmp/canopy-demo
```

Use fixtures for anything destructive, never a repository you care about.

## Checks

`tools/check.sh` runs CI's checks for this platform (`cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the release
tooling self-test and the licence-notice check, plus `shellcheck` and `cargo deny check` when they
are installed) and one CI does not: the installer test, on Linux and macOS. It must pass before
you merge; a failure already listed in `docs/backlog.md` is reported, not hidden. Keep `TMPDIR` on
a filesystem that accepts non-UTF-8 file names (the default `/tmp` does; NTFS does not).

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):
`<type>(<scope>): <summary>`, summary in the imperative and under 72 characters.

- Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `style`.
- Scopes: a crate (`core`, `app`) or a module (`graph`, `worker`, `diff`, `history`, `theme`, ...).
- Breaking changes: `!` after the scope and a `BREAKING CHANGE:` footer.

Example: `feat(changes): stage individual lines from the diff viewer`.

## Before merging

- [ ] One logical change, on top of the current `main`.
- [ ] `tools/check.sh` passes.
- [ ] New or changed Git operations have tests against a temporary repository.
- [ ] UI changes include before and after screenshots (dark, and light where colours change),
      taken with `tools/screenshot.sh` on a fixture.
- [ ] `docs/features.md`, `docs/architecture.md`, `docs/design.md` and `CHANGELOG.md`
      (Unreleased) are updated where the change touches them.
- [ ] Destructive actions go through a confirmation dialog.
- [ ] No credentials, tokens or personal paths in code, tests or screenshots.

## Library notes

API details of the pinned versions that older examples get wrong:

- **egui 0.36**: the app implements `eframe::App::ui(&mut self, ui, frame)`; panels are
  `egui::Panel::left/right/top/bottom(id).show(ui, …)` and take a `Ui`, not a `Context`;
  `CentralPanel::default().show(ui, …)`; `ctx.global_style()` and
  `ctx.egui_wants_keyboard_input()`; `Shadow`, `Margin` and `CornerRadius` have integer fields.
- **git2 0.21** returns `Result` from many string getters (`Reference::name`, `Remote::url`;
  `StringArray` iteration yields `Result<Option<&str>>`); prefer the `*_bytes` variants.
- **Inter**: the bundled files have their private-use code points removed so Phosphor icons
  render; re-apply that when updating Inter (`crates/canopy-app/assets/fonts/README.md`).

Update this section when one of these crates is upgraded.
