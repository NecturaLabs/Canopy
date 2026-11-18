# Practices audit

Each area of Canopy's work is held to the current practices of the sources listed for it. An area
is audited at least weekly; its date is recorded only when the gaps found are fixed or filed as
issues. `never` means it has not been audited against this table yet.

| Area | Held to | Last audit |
|---|---|---|
| Rust code, lints and API design | The Rust API Guidelines, the Clippy lint documentation, the Rust Reference and edition guide, the Rustonomicon (for `unsafe`) | never |
| Git semantics and the `git` CLI | The Git documentation (git-scm.com, `git help`), the Git source and release notes, libgit2 documentation | never |
| User interface, interaction and accessibility | The egui and eframe documentation, WCAG 2.2, the platform guidelines (GNOME HIG, Apple HIG, Windows design guidance), AccessKit | never |
| Hosting API clients | The GitHub REST and GraphQL documentation, the GitLab REST and GraphQL documentation, both providers' rate-limit and authentication guides | never |
| Security and supply chain | The OWASP guides that apply to desktop clients, RustSec advisories, `cargo deny` documentation, the OpenSSF Scorecard checks | never |
| CI and GitHub configuration | The GitHub Actions security hardening guide, GitHub's repository security and ruleset documentation, the OpenSSF Scorecard checks | never |
| Release, packaging and updates | The Cargo book, the freedesktop.org desktop entry and AppStream specifications, Apple's notarization and Windows signing documentation, The Update Framework | never |
| Testing | The Rust book's testing chapter, the Cargo test documentation, the project's `testing` skill | never |
| Documentation | Diátaxis, the ADR guidance at adr.github.io, the project's `project-docs` skill | never |
| Open-source governance | GitHub's community health file documentation, the Open Source Guides (opensource.guide), the Contributor Covenant, SPDX licence identifiers, REUSE | never |
| Landing site | WCAG 2.2, web.dev guidance, MDN | never |
