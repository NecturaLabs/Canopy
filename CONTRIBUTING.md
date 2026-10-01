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

`tools/check.sh` runs what CI runs: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`. It must
pass before you open a pull request.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):
`<type>(<scope>): <summary>`, summary in the imperative and under 72 characters.

- Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `style`.
- Scopes: a crate (`core`, `app`) or a module (`graph`, `worker`, `diff`, `history`, `theme`, ...).
- Breaking changes: `!` after the scope and a `BREAKING CHANGE:` footer.

Example: `feat(changes): stage individual lines from the diff viewer`.

## Pull requests

- [ ] One logical change; the branch is up to date with `main`.
- [ ] `tools/check.sh` passes.
- [ ] New or changed Git operations have tests against a temporary repository.
- [ ] UI changes include before and after screenshots (dark, and light where colours change),
      taken with `tools/screenshot.sh` on a fixture.
- [ ] `docs/features.md`, `docs/architecture.md`, `docs/design.md` and `CHANGELOG.md`
      (Unreleased) are updated where the change touches them.
- [ ] Destructive actions go through a confirmation dialog.
- [ ] No credentials, tokens or personal paths in code, tests or screenshots.
