# ADR 0013: Hosting credentials come from the GitHub or GitLab CLI first

**Status:** accepted
**Date:** 2026-10-12

## Context

ADR 0004 gave Canopy three places to look for a hosting token: one saved in Canopy, the provider's
environment variable and git's credential helper. A user who already has the GitHub CLI (`gh`) or
the GitLab CLI (`glab`) signed in, with the scopes Canopy needs, still had to create a second token
and paste it (issue #136). Both CLIs hold exactly the credential Canopy wants, per host, and tell
which account and scopes it has.

## Decision drivers

- No second token for a user who is already signed in; no new place a token is stored.
- The credential still goes only to its own host, and never into a URL, a log line, a file or a
  `Debug` output (ADR 0004).
- A CLI that is missing, signed out, short of a scope or failing must be told apart, with a message
  that says what to do, and must never block the other sources.
- Running a program is new for `canopy-hosting`; it must be as contained as `git credential fill`.

## Decision outcome

`hosting::resolve_token` takes the first source that gives a token, in this order:

1. **The CLI's sign-in** (`gh` for GitHub, `glab` for GitLab), for the repository's own host and
   port. The token is held in memory (`canopy_hosting::CliLogin`, whose `Debug` hides it) and
   handed to the HTTP client; Canopy never saves it. `TokenSource::Cli` labels it "from the GitHub
   CLI" or "from the GitLab CLI".
2. **The provider's environment variable**, under ADR 0004's per-host rules.
3. **The token saved in Canopy** (`tokens.json`).
4. **git's credential helpers.**

This moves the environment ahead of the saved token (ADR 0004 had it behind): a variable set for
one shell is a deliberate, current choice, and the order now reads from most to least ambient
authority of the user's own tooling. Nothing is migrated: the saved token is simply the fallback.

How the CLI is run (`canopy-hosting/src/cli_token.rs`):

- Through the executable on `PATH`, without a shell: `gh auth status --hostname <host>` and
  `gh auth token --hostname <host>`; `glab auth status --hostname <host> --show-token`, which is
  the one `glab` command that prints the token. The host is one argument, rejected when empty,
  starting with `-` or holding whitespace or control characters.
- Without a terminal prompt, with colour off, with standard input closed, with the output capped,
  and killed after ten seconds in all.
- Without the token variables (`GH_TOKEN`, `GITHUB_TOKEN`, `GITLAB_TOKEN`, `GL_TOKEN`, enterprise
  and access-token variants, `GITLAB_HOST`), so a CLI reports only its own stored sign-in and
  never answers for a host an environment token was not meant for (ADR 0004).
- On the hosting thread, once when it starts, off the UI thread.
- A token is accepted only as one run of visible ASCII characters of at most 4096. Words from the
  CLI that reach the user or a log go through `http::shorten`, which also hides token shapes.

Outcomes: not installed (nothing shown, the next source applies), signed out, signed in but
lacking the scope Canopy needs (`repo` for GitHub, `api` for GitLab, checked only where the CLI
lists scopes), failed or timed out. Each of the last three skips the CLI and moves on to the next
source, and the requests view says why when no source gave a token (`HostEvent::Cli`).

### Consequences

- **Good:** a signed-in `gh` or `glab` works with no setup, including self-hosted GitLab hosts the
  CLI knows; the token has no new home.
- **Good:** the privacy policy gains one line: Canopy runs the CLI to read the signed-in token.
- **Bad:** `gh auth status` makes the CLI contact the host to test its token, from the CLI and not
  from Canopy; a slow or offline host costs up to the ten seconds.
- **Bad:** the CLI is found through `PATH`: whatever `gh` or `glab` comes first there is trusted
  with the question, as `git` is.
- **Bad:** the token is read once per hosting thread; a CLI sign-in renewed later is picked up when
  the tab's hosting thread restarts.
- The CLIs' output formats are not a stable interface; the parser reads the lines it needs and
  everything else is ignored. `glab auth status --show-token` and its `Token found:` line are as
  of recent releases and unverified against every version.

## Related

- ADR 0004: the token sources this one reorders, and where tokens may go.
- ADR 0003: the network services; the CLI is a local program, not a new service.
- `docs/legal/privacy-policy.md`, "GitHub and GitLab".
