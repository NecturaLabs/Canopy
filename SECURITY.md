# Security policy

## Reporting a vulnerability

Report security problems privately, never in a public issue or pull request:

- on GitHub, through private vulnerability reporting:
  <https://github.com/NecturaLabs/Canopy/security/advisories/new> (**Security → Report a
  vulnerability**), or
- by e-mail to support@necturalabs.com, with "Security" in the subject.

Say what is affected, how to reproduce it and what an attacker gains. Leave out real credentials,
tokens and private repository contents.

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

From build 1683 on, every file a release publishes (the archives, `install.sh`, `install.ps1`
and `SHA256SUMS`) carries signed build provenance. To check that a file was built by the Release
workflow from the release's tag on a GitHub-hosted runner:

```sh
gh attestation verify <file> --repo NecturaLabs/Canopy \
  --signer-workflow NecturaLabs/Canopy/.github/workflows/release.yml \
  --source-ref refs/tags/build-<number> --deny-self-hosted-runners
```

`SHA256SUMS` in the release lists each file's checksum.

## Scope

Canopy runs `git` on your computer and talks only to the hosts its Privacy Policy lists
(`docs/legal/privacy-policy.md`). Of most interest: running a program or reading a credential
through a repository or its configuration, settings import, the askpass helper, the update feed
and installers, URL and file opening, and what crash reports or logs carry. `AGENTS.md`
("Security-sensitive areas") lists the code involved.
