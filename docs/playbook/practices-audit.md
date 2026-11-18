# Practices audit

Each area of Canopy's work is held to the current practices of the sources listed for it. An area
is audited at least weekly; its date is recorded only when the gaps found are fixed or filed as
issues. `never` means it has not been audited against this table yet.

| Area | Held to | Last audit |
|---|---|---|
| Rust code, lints and API design | The Rust API Guidelines, the Clippy lint documentation, the Rust Reference and edition guide, the Rustonomicon (for `unsafe`) | 2026-11-18 |
| Git semantics and the `git` CLI | The Git documentation (git-scm.com, `git help`), the Git source and release notes, libgit2 documentation | 2026-11-18 |
| User interface, interaction and accessibility | The egui and eframe documentation, WCAG 2.2, the platform guidelines (GNOME HIG, Apple HIG, Windows design guidance), AccessKit | 2026-11-18 |
| Hosting API clients | The GitHub REST and GraphQL documentation, the GitLab REST and GraphQL documentation, both providers' rate-limit and authentication guides | 2026-11-18 |
| Security and supply chain | The OWASP guides that apply to desktop clients, RustSec advisories, `cargo deny` documentation, the OpenSSF Scorecard checks | 2026-11-18 |
| CI and GitHub configuration | The GitHub Actions security hardening guide, GitHub's repository security and ruleset documentation, the OpenSSF Scorecard checks | 2026-11-18 |
| Release, packaging and updates | The Cargo book, the freedesktop.org desktop entry and AppStream specifications, Apple's notarization and Windows signing documentation, The Update Framework | 2026-11-18 |
| Testing | The Rust book's testing chapter, the Cargo test documentation, the project's `testing` skill | 2026-11-18 |
| Documentation | Diátaxis, the ADR guidance at adr.github.io, the project's `project-docs` skill | 2026-11-18 |
| Open-source governance | GitHub's community health file documentation, the Open Source Guides (opensource.guide), the Contributor Covenant, SPDX licence identifiers, REUSE | 2026-11-18 |
| Branches, pull requests and review | GitHub's ruleset, protected branch and pull request documentation, the OpenSSF Scorecard Branch-Protection and Code-Review checks, Conventional Commits, opensource.guide | 2026-11-19 |
| Landing site | WCAG 2.2, web.dev guidance, MDN | 2026-11-18 |

## 2026-11-18

All areas audited. Fixed in the same change: release jobs publish from the verified commit, a
weekly RustSec advisory check, the AppStream metadata, the code of conduct (Contributor Covenant
3.0), `SUPPORT.md`, `ACCESSIBILITY.md` and README drift; secret scanning's non-provider patterns
and validity checks and the required SHA pinning of actions were turned on. Filed: #196 to #209;
added to #42, #95 and #188.

## 2026-11-19

Branches, pull requests and review, added as an area. Fixed: one `Tests` check for the test matrix,
to be required on `dev` and `main` once #214 makes the suite pass on hosted runners; `dev` requires an up-to-date branch; the approval rule asks
for a code owner and the latest push; only merge commits are allowed in the repository settings
too; the pull request template asks for the issue, review focus and what was not verified; a
Pull request title check; branch naming and promotion titles documented. Kept: the maintainer's
admin bypass of the approval rule (one maintainer cannot approve their own pull requests; the
Scorecard Code-Review check stays at 0 until there is a second reviewer), and `main`'s release
merge commits not being on `dev`.
