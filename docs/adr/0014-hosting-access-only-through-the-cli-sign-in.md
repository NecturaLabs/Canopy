# ADR 0014: Hosting access only through the GitHub and GitLab CLI sign-in

**Status:** accepted
**Date:** 2026-10-14

## Context

ADR 0004 gave Canopy four places to find a hosting token (one saved in Canopy, the provider's
environment variable, git's credential helper) and a form to paste one; ADR 0013 put the sign-in of
the GitHub CLI (`gh`) and the GitLab CLI (`glab`) in front of them. Both CLIs already hold exactly
the credential Canopy needs, per host, and tell which account and scopes it has. The older sources
kept Canopy responsible for a secret: a plaintext `tokens.json` the privacy policy and installers
had to describe, environment rules per host and port, a password prompt on the Accounts page.
Issue #227 records the owner's decision of 2026-10-13: personal access tokens are gone.

## Decision drivers

- Canopy holds no secret it does not need to hold: nothing stored, nothing to paste.
- One source of credentials, so every "why did this request fail" has one place to look.
- The credential goes only to its own host and never reaches a URL, a log line, a file or a
  `Debug` output (ADR 0004, kept).
- A user whose CLI is missing, signed out, short of a scope or failing is told which, with the
  command that fixes it, and public repositories keep loading without any credential.

## Considered options

1. Keep the fallback chain of ADR 0013 (CLI, environment, saved token, credential helper).
2. Only the CLIs.
3. The CLIs plus an OAuth device flow of Canopy's own against each provider.
4. The CLIs plus a token saved in the operating system's credential store.

## Decision outcome

Chosen: **2, only the CLIs.**

- `hosting::resolve_token` asks `gh` or `glab` (`canopy_hosting::login_from_cli`, ADR 0013's
  runner, unchanged in how it runs the program) for the repository's own host and port, once when
  a tab's hosting thread starts. The token stays in memory and goes in an `Authorization: Bearer`
  header to that host only. `GH_TOKEN`, `GITHUB_TOKEN`, `GITLAB_TOKEN`, `GL_TOKEN`, git's
  credential helpers and the saved-token store are gone with their code (`TokenStore`,
  `token_from_env_for`, `token_from_git_credentials`). The CLI is still started without those
  variables, so it reports only its own stored sign-in and `gh auth token` cannot hand back an
  environment token. A test starts Canopy's hosting thread with those variables set and a stub
  API, and asserts that no request carries them.
- **`tokens.json` is deleted** (best effort, on its own thread, at every start; nothing happens
  once it is gone), so the plaintext tokens of earlier builds do not stay on disk while the
  privacy policy says Canopy saves none. The commit that ships this says so in the release note.
  The deletion does not erase the disk blocks, and the policy says that too. No settings migration
  is written for the removed fields: they are ignored.
- **Settings → Accounts** is a status page, one card per host (github.com, gitlab.com, each
  self-hosted GitLab host listed in Integrations, each host of an open repository): CLI not
  installed (a link to the official install page, opened as a web address through `platform.rs`),
  signed out or short of a scope (the exact command with a button that copies it), signed in (the
  account and the scopes the CLI lists), or failed (the CLI's words). Canopy does **not** run
  `gh auth login` itself: the login is interactive and Canopy never opens a terminal prompt.
- The page asks the CLI when it opens and on **Check again**, on its own threads (a host without a
  tab has no hosting thread). A check that finds another answer than an open tab's hosting thread
  started with restarts that tab's thread, so installing or signing in does not need a restart of
  Canopy. While Pull and merge requests are off, or before the start-up questions are answered,
  the page runs nothing: `gh auth status` contacts the host.
- The same state (`CliState`, the CLI's account or the reason it cannot be used) is what the
  requests view, the repository's Hosting block and the picture notes say, so they never disagree
  with the Accounts page.

### Consequences

- **Good:** Canopy stores no credential and asks for none; the privacy policy loses its saved-token
  and environment paragraphs, and the installers' purge text no longer promises tokens are removed.
- **Good:** one answer to "where does the credential come from", visible on one page.
- **Bad:** users who only had an environment token or a credential helper (CI, headless machines)
  lose hosting access until they sign in with a CLI. Public repositories still load.
- **Bad:** `gh auth status` contacts the host from the CLI, and the page runs it for every host it
  lists, so a slow or offline host costs up to ten seconds of a card showing "Checking".
- **Bad:** the CLI is found through `PATH`: whatever `gh` or `glab` comes first there is trusted with
  the question, as `git` is.
- **Bad:** `glab` does not list scopes, so a missing `api` scope shows only as the host refusing the
  sign-in, with the command that signs in again.
- The CLIs' output is not a stable interface. The parser was checked against `gh` 2.102 run for
  real and against `glab`'s source (`internal/commands/auth/status/status.go`); a live `glab` run is
  still open in issue #136.

## Related

- ADR 0004: superseded in full (token sources, saved-token store, environment rules). Its rule that a
  credential reaches only its own host and its redirect and loopback-override rules still hold.
- ADR 0013: the CLI runner and the account and scope reading stay; its fallback order (steps 2 to 4)
  is superseded.
- ADR 0003: the CLI is a local program, not a new service.
- Issues #227 (this change) and #136 (live checks of the CLIs, security pass).
- `docs/legal/privacy-policy.md`, "GitHub and GitLab".
