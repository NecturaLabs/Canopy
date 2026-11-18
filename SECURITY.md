# Security policy

## Reporting a vulnerability

Report security problems privately, never in a public issue or pull request:

- on GitHub, through private vulnerability reporting:
  <https://github.com/NecturaLabs/Canopy/security/advisories/new> (**Security → Report a
  vulnerability**), or
- by e-mail to support@necturalabs.com, with "Security" in the subject.

Say what is affected, how to reproduce it and what an attacker gains. Leave out real credentials,
tokens and private repository contents.

## What counts as a vulnerability

A bug is a vulnerability when it crosses a security boundary: something untrusted, such as a
repository you open, its configuration, a remote, a hosting server or a file you import, gains
access, permissions or control it should not have. Examples: running a program through a
repository's settings, reading a credential, or sending data to a host the Privacy Policy does
not list. Code that could be more robust without crossing a boundary is a normal bug: report it
in Discussions (collaborators open issues directly; see CONTRIBUTING.md).

## Research in good faith

Test only your own computer, repositories and accounts, or ones you have permission to test.
Avoid privacy violations, data destruction and service disruption, including to GitHub, GitLab
and the other hosts Canopy contacts. Do not exploit a problem beyond what shows it, and give us
the time described below before publishing.

## What happens next

1. We acknowledge the report within 7 days.
2. We confirm or rule out the vulnerability and tell you which, within 14 days of the report where
   we can.
3. We fix it privately in a GitHub security advisory for this repository, asking GitHub for a CVE
   when the issue affects published builds.
4. We publish the fixed build, then the advisory. The build's release notes list the fix under
   Security, and the advisory and notes credit you unless you ask us not to.

We follow coordinated disclosure: please keep the details private until the advisory is published,
or for 90 days from your report if that comes first. If a fix needs longer, we agree a date with
you.

## Supported versions

Only the latest published build receives security fixes. Canopy offers updates in the app
(Settings → Updates); update before reporting if you can.

## Verifying a download

Release archives carry signed build provenance from build 1683 on, and from the next build the
installers (`install.sh`, `install.ps1`) and `SHA256SUMS` do too. A release is built by the
Release workflow, which Create release runs from `main`, so the provenance names `main` and the
release's commit. To check that a file was built that way, from build `<number>`'s commit, on a
GitHub-hosted runner:

```sh
gh attestation verify <file> --repo NecturaLabs/Canopy \
  --signer-workflow NecturaLabs/Canopy/.github/workflows/release.yml \
  --source-ref refs/heads/main \
  --source-digest "$(git rev-parse 'build-<number>^{commit}')" \
  --deny-self-hosted-runners
```

Run the `git rev-parse` in a clone with the tags fetched, or put the commit from the release page
in its place.

`SHA256SUMS` in the release lists each file's checksum.

Releases published after build 1683 are immutable, in this repository and in
`NecturaLabs/app-updates`: their files and tags cannot change once published, and GitHub signs a
release attestation. `gh release verify-asset build-<number> <file> --repo NecturaLabs/Canopy`
checks that a file is the one that release published.

## Scope

Canopy runs `git` on your computer and talks only to the hosts its Privacy Policy lists
(`docs/legal/privacy-policy.md`). Of most interest: running a program or reading a credential
through a repository or its configuration, settings import, the askpass helper, the update feed
and installers, URL and file opening, and what crash reports or logs carry. `AGENTS.md`
("Security-sensitive areas") lists the code involved.
