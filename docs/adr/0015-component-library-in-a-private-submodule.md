# ADR 0015: The component library lives in a private submodule

**Status:** accepted
**Date:** 2026-10-17

## Context

Canopy's theme, fonts, motion, in-code illustrations and generic widget kit (`crates/canopy-app/src/ui`) are
not about Git, and the owner's other applications need the same look. Keeping them inside
`canopy-app` meant copying them or pulling Git-specific code into those applications.

## Decision

- **The library is `NecturaLabs/nectura-ui`, a private repository, mounted as the submodule
  `crates/nectura-ui`** and built as a workspace member, so it shares `Cargo.lock` and the `[patch]`
  entries. It knows nothing about Git or Canopy: the application passes what it needs through
  `nectura_ui::host::Host` and `kit::Shortcut`. Canopy keeps the Git components (change letters,
  ref pills, the history graph) in `ui/kit`.
- **A change to the library is made in its own repository**, then the submodule pointer here is
  updated in a Canopy commit.
- **CI and releases fetch it over SSH with a read-only deploy key** on `nectura-ui`, kept as the
  `SUBMODULE_DEPLOY_KEY` secret (Actions and Dependabot stores) and used by the composite action
  `.github/actions/fetch-ui` after checkout, with GitHub's host key pinned. A workflow's own token
  reads only the repository it runs in, even inside the same organization (a probe on 2026-10-15
  got "Repository not found"), and a personal access token would tie CI to one account. Deploy keys
  are enabled for the organization for this. Without the secret every job that builds or checks
  Canopy fails at the fetch step.
- **Checks read the submodule's files**: colours, source headers, numeric literals, docs, the
  unsafe-code policy and the third-party notices.

## Consequences

- Cloning Canopy needs read access to `nectura-ui` and `--recurse-submodules`.
- The licence rows for fonts and assets, and the theme tokens, are maintained in `nectura-ui`.
