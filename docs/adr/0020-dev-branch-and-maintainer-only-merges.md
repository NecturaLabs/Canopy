# ADR 0020: Work lands in dev; main takes only dev; only the maintainer merges

**Status:** accepted
**Date:** 2026-11-18

## Context

Canopy and `NecturaLabs/nectura-ui` are public. Pull requests went straight into `main`, the
branch builds are released from (a build number is `git rev-list --count` on `main`), and `main`
was protected only against deletion and force pushes. Anyone could open issues, though the issue
tracker is the project's backlog (AGENTS.md). The owner asked that nobody else can open issues or
get a pull request merged into `main`, and that pull requests go into a `dev` branch only the
maintainer merges.

## Decision

- **`dev` is the default branch and takes all work.** Contributors and Dependabot open pull
  requests against it; `Closes #N` closes issues when a pull request merges into it.
- **`main` takes only `dev`**, through a pull request, when a build is released. The required
  check "Source branch" (`.github/workflows/branch-policy.yml`) fails a pull request into `main`
  from any other branch or fork, and a pull request opened against `main` is moved to `dev`
  (`pull_request_target`, which changes the base through the API and never checks out or runs the
  pull request's code). Releases stay on `main` (`create-release.yml`), so build numbers are
  unchanged.
- **Three rulesets on each branch**. "Protect": no deletion or force push, with no bypass.
  "CI for": the required CI checks (What changed, Clippy, Tests, and Format, docs,
  tooling and dependencies) must pass,
  with no bypass, so nothing reaches either branch with failing CI; on `dev` the branch must
  also be up to date, so the tested code is the merged code (not on `main`,
  whose release merge commits `dev` never takes). "Pull request for": a pull request with one
  approval, from a code owner, of its latest push, and updates restricted;
  an organisation admin bypasses it, so the maintainer merges without a second reviewer and may
  fast-forward `main` to `dev` directly (that skips only "Source branch", which runs on pull
  requests). No other account has write access. Release tags (`build-*`, and `v*` in
  `nectura-ui`) cannot be moved or deleted by anyone (amended 2026-11-18 after a configuration
  audit: CI used to share the bypassable ruleset; amended 2026-11-19: Tests (Linux), up-to-date branches, code
  owner and latest-push approval added, merge commits the only merge method).
- **Issues: collaborators only** (GitHub's issue creation policy). Everyone else reports bugs and
  proposes features in Discussions; a maintainer files the issue. Pull requests stay open to
  everyone, as in other open-source projects, and outside contributors' workflow runs need the
  maintainer's approval first.
- **Security reports go through private vulnerability reporting** (SECURITY.md).

The same settings apply to `nectura-ui`.

## Consequences

- A release takes one more step: a pull request from `dev` into `main` (release skill).
- Outside contributors cannot open issues; their bug reports start as discussions.
- The maintainer approves, with an explicit merge, everything that reaches `dev` or `main`.
