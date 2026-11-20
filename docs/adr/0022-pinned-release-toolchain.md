# ADR 0022: How Canopy selects and records the release toolchain

**Status:** accepted
**Date:** 2026-11-20

## Context

The release workflow installed a checksum-verified rustup and built with `Cargo.lock`, but its
`stable` toolchain could resolve to a different compiler on a later run of the same commit. The
release assets gave readers no way to identify the compiler that built them. Issue #204 calls
for a toolchain pin that stays current.

## Decision

Keep the exact stable Rust channel in [rust-toolchain.toml](../../rust-toolchain.toml), with the
minimal profile, Clippy and rustfmt. Dependabot proposes weekly updates. Local rustup reads the
file automatically. CI and releases pass its channel to the setup action, disable rustup's
automatic toolchain installation and verify the active compiler before building.

CI checks the minimum supported Rust version separately, so moving the pin keeps the release
compiler current without raising the minimum. The release publishes the Rust version and
toolchain details from each runner whose archive ships, with the same checksums and provenance
as the other release assets.

## Alternatives

- Keep `stable` and log its resolved version. Readers could identify a build's compiler, but a
  rebuild of the same commit could use a newer one.
- Pin only the release workflow input. Local builds would still select their own compiler, and
  Dependabot's Rust toolchain updater would have no toolchain file to update.

## Consequences

- A release of a tag from before the pin cannot be published for the first time: the identity
  job and the release job refuse a commit without `rust-toolchain.toml`. Republishing an
  already published release of such a tag still works, because it ships the files it already
  has.
- The required `Tests` check now depends on `static.rust-lang.org`: the toolchain, and for the
  minimum-version job rustup itself, are downloaded on every run. An outage there fails the
  check.
- Each Dependabot update of the pin changes the compiler the release binaries are built with.
  Its commit is `build(deps)`, so it makes a build releasable and its pull request deserves the
  same review as a dependency bump.
- The minimum-version job installs `1.95`, which rustup resolves to the newest 1.95.x patch
  release, so it floats within that series and is not pinned like the release compiler.
