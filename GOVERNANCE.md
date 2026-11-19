# Governance

Canopy is a NecturaLabs project with a single maintainer, who decides what is merged and released
and settles disagreements. The maintainer explains decisions in the issue or pull request where
they come up; consequential ones are recorded as ADRs in [docs/adr/](docs/adr/).

## Roles

- **Contributors**: anyone who files an issue, takes part in a discussion or opens a pull request.
  Contributions follow [CONTRIBUTING.md](CONTRIBUTING.md) and the
  [code of conduct](CODE_OF_CONDUCT.md).
- **Maintainers**: people with write access, who triage issues, review and merge pull requests and
  cut releases. Today that is the NecturaLabs maintainer.

## Review

Every pull request into `dev` or `main` needs passing required checks and the approval of a code
owner for its latest push. GitHub does not let an author approve their own pull request, so while
there is one maintainer, the maintainer's own pull requests are merged through the rulesets'
administrator bypass, after CI passes and an independent review, done apart from the change's
author, whose findings are answered in the pull request. A contributor's pull request is approved
by the maintainer. The OpenSSF Scorecard Code-Review check counts only GitHub approvals, so it
stays low until a second maintainer reviews.

## Becoming a maintainer

A contributor with a record of sustained, careful contributions and reviews may be invited to
become a maintainer. They are added to `.github/CODEOWNERS` and this file is updated.

## Changes to this document

Changes to governance are made by pull request and approved by the maintainer.
