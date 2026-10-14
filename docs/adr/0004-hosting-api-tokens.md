# ADR 0004: Where hosting API tokens come from, where they are kept and where they may go

**Status:** superseded by [ADR 0014](0014-hosting-access-only-through-the-cli-sign-in.md) (tokens are gone; the order of its sources was already superseded by [ADR 0013](0013-hosting-credentials-from-the-cli-first.md))
**Date:** 2026-10-01

Reconstructed from the shipped code and the Privacy Policy on the day the hosting API landed; the
owner has not recorded separate reasoning for the storage choice.

## Context and problem statement

Pull and merge request features (ADR 0003) call the GitHub and GitLab REST APIs, which need a
personal access token for private projects and for every action (comment, approve, merge, create).
Canopy never had to hold a credential before: `git` did. A token is the most sensitive thing
Canopy touches, and three questions need one answer every later change follows: where the token
comes from, whether and where Canopy stores it, and which hosts may receive it.

## Decision drivers

- A token must never leak into a URL, log line, toast, error message, `Debug` output, screenshot
  or crash report, and must never reach a host other than the one it belongs to.
- Users who already work with `gh`, `glab` or a credential helper should get the feature without
  setup; users without any of them need a way to enter one.
- Self-hosted GitLab hosts are arbitrary names: a host whose name merely contains "gitlab" must not
  be trusted with a token meant for gitlab.com.
- The same behaviour on Linux, macOS and Windows, with few dependencies that must pass
  `cargo deny`.

## Considered options

Token source:

1. Only what the user already has: provider environment variables and git's credential helpers;
   Canopy stores nothing.
2. Also a token typed into Canopy and saved by Canopy in its own data folder.
3. A token typed into Canopy and saved in the OS credential store (Secret Service, Keychain,
   Windows Credential Manager).
4. An OAuth device flow against each provider.

Where an environment token may go:

1. Any host recognised as GitHub or GitLab.
2. Only the host the variable is meant for.

## Decision outcome

Chosen: sources **1 and 2 together**, with environment tokens restricted per **2**.

`hosting::resolve_token` (in `crates/canopy-app/src/hosting.rs`) takes the first that exists:

1. **A token saved in Canopy** for that host, in `tokens.json` in Canopy's data folder
   (`hosting::TokenStore`). The file is plain text, written to a temporary file that only the user
   can read from the moment it exists (mode 0600 on Unix; the user profile's permissions on
   Windows) and renamed into place. The Accounts section of Settings adds, tests and removes
   tokens; `TokenStore`'s `Debug` output shows hosts only.
2. **The provider's environment variable**: `GH_TOKEN` then `GITHUB_TOKEN` for github.com;
   `GITLAB_TOKEN` then `GL_TOKEN` for gitlab.com, for the host named by `GITLAB_HOST`, and for
   the self-hosted GitLab hosts the user listed in Settings
   (`canopy_hosting::token_from_env_for`). A host recognised as GitLab only by its name never gets
   an environment token; the older host-blind `token_from_env` is deprecated.
3. **git's credential helpers**: `git credential fill` for the host's authority, with no prompt,
   no askpass and a 10 s kill timer (`canopy_hosting::token_from_git_credentials`). The hosting
   thread always tries this step; `resolve_token`'s `use_git_credentials` flag exists so tests
   can leave the user's helpers alone.

Whatever the source, the token travels only in an `Authorization: Bearer` header to the API of the
repository's own host (`canopy_hosting::Client`); the HTTP agent forwards authorization headers
only to the same host on a redirect (`RedirectAuthHeaders::SameHost`). Without a token, reads on
public projects still work and actions fail with the provider's own message. The development
override `CANOPY_HOSTING_API_BASE` accepts only a plain loopback `http://` URL
(`canopy_hosting::is_loopback_base`), so it cannot redirect a token off the machine.

The OS credential store (option 3) is the better home for a typed token and is not ruled out; it
was not chosen because it adds a platform-specific dependency tree, and the shipped design keeps
the same three-step lookup on every platform. Option 4 needs an OAuth application
registered with each provider and does not cover self-hosted GitLab without per-instance setup.

### Consequences

- **Good:** most users need no setup; environment and helper tokens are never written by Canopy.
- **Good:** the Privacy Policy states plainly where a saved token lives and that it is not
  encrypted; users can delete it from Settings.
- **Bad:** a token saved in Canopy is readable by anything running as the user. Moving it to the
  OS credential store is the follow-up if the owner wants it; it would change the Privacy Policy.
- **Bad:** `crates/canopy-hosting/src/token.rs`, `crates/canopy-hosting/src/http.rs`,
  `crates/canopy-app/src/hosting.rs` and anything that builds a URL or a log line near a token are
  security-sensitive; changes get a security pass.
- **Bad:** the redirect rule is configured but no test pins it; a test that serves a redirect to
  a second loopback port and asserts the header is not forwarded would.

## Related

- ADR 0003: the services Canopy contacts and the consent that gates them.
- ADR 0005: Canopy's own credential prompts, which are for `git` and `ssh`, never for these tokens.
- `docs/legal/privacy-policy.md`, "GitHub and GitLab".
- `.claude/skills/hosting-api/SKILL.md`.
