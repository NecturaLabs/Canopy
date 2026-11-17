# Security policy

## Reporting a vulnerability

Report security problems privately, never in a public issue or pull request:

- on GitHub, through **Security → Report a vulnerability** in this repository (private
  vulnerability reporting), or
- by e-mail to support@necturalabs.com, with "Security" in the subject.

Say what is affected, how to reproduce it and what an attacker gains. Leave out real credentials,
tokens and private repository contents. We acknowledge a report within a week, keep you informed
while we fix it, and credit you in the release notes unless you ask us not to.

## Supported versions

Only the latest published build receives security fixes. Canopy offers updates in the app
(Settings → Updates); update before reporting if you can.

## Scope

Canopy runs `git` on your computer and talks only to the hosts its Privacy Policy lists
(`docs/legal/privacy-policy.md`). Of most interest: running a program or reading a credential
through a repository or its configuration, settings import, the askpass helper, the update feed
and installers, URL and file opening, and what crash reports or logs carry. `AGENTS.md`
("Security-sensitive areas") lists the code involved.
