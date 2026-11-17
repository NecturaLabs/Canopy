# ADR 0019: Canopy is open source under MIT OR Apache-2.0

**Status:** accepted
**Date:** 2026-11-18

## Context

Canopy was a private, proprietary repository: its `LICENSE` granted no rights, the Terms of Service
forbade copying, modifying and reverse engineering, and the component library `NecturaLabs/nectura-ui`
was a private submodule that CI fetched with a deploy key. The product stays free; the owner chose to
publish the source after an audit of the repository, its history and its issue tracker for anything
personal, private or exploitable.

## Decision

- **Licence: MIT OR Apache-2.0, at the user's option** (`LICENSE-MIT`, `LICENSE-APACHE`), the Rust
  ecosystem's usual pair, which egui and most of Canopy's dependencies use. Apache-2.0 adds a patent
  grant; MIT keeps reuse simple. Contributions come in under the same terms (Apache-2.0 section 5),
  with a Developer Certificate of Origin sign-off (`CONTRIBUTING.md`).
- **The component library goes public under the same licence**, so anyone can build and test Canopy.
  CI checks it out with `actions/checkout` (`submodules: true`); the deploy key and
  `.github/actions/fetch-ui` are gone (supersedes that part of ADR 0015).
- **The Terms of Service cover NecturaLabs' published builds and their services**, not the code:
  the copy, modify, reverse-engineering and AI-training limits are gone, and the Canopy name and
  icon stay NecturaLabs' marks for its own builds. `TERMS_VERSION` moved, so every user accepts the
  new terms once.
- **A fresh public repository with a cleaned history.** GitHub keeps every pull request's commits
  (`refs/pull/*`) and cannot transfer issues from a private repository to a public one, so the
  private repository was renamed and archived, and `NecturaLabs/Canopy` was created from a rewritten
  history: every author and committer is the organisation's account, AI attribution trailers and
  session links are removed from messages, deleted session handoffs and early screenshots are
  purged. The commit count is unchanged, so build numbers (`git rev-list --count` on `main`) and
  the `build-*` tags still line up. Issue and pull request numbers are preserved: open issues are
  recreated with their text, closed ones as closed placeholders.
- **Every workflow runs on GitHub-hosted runners.** A public repository's pull requests can change
  workflow files, so a self-hosted runner would run a stranger's code on a persistent machine. A
  pull request from a fork runs CI through `pull_request` with a read-only token and no secrets, and
  needs a maintainer's approval to start.

## Consequences

- Anyone may fork, change and redistribute Canopy, including commercially, under either licence.
- Security problems are reported privately (`SECURITY.md`, GitHub private vulnerability reporting),
  never in a public issue.
- Nothing personal, machine-specific or private goes into files, commits, issues or pull requests
  (`AGENTS.md`).
- Old commit hashes, and links to them from before 2026-11-18, do not resolve in the public
  repository.
