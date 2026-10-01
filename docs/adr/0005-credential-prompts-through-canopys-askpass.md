# ADR 0005: How git and ssh ask the user for credentials

**Status:** accepted
**Date:** 2026-10-01

## Context and problem statement

Every mutation and all Git network traffic run the user's `git` (ADR 0001) with
`GIT_TERMINAL_PROMPT=0`, because Canopy has no terminal and a process waiting on one hangs the
job. A remote that needs a user name, password, passphrase or an SSH host-key decision then fails
unless something else answers. Credential helpers cover stored secrets; they do not cover the
first sign-in, a passphrase-protected key without an agent, or an unknown host key.

## Decision drivers

- A prompt must appear inside Canopy, in the window the user is looking at, with the password
  typed hidden.
- It must work for `git` and for the `ssh` that `git` starts, on Linux, macOS and Windows.
- A user's own `GIT_ASKPASS` or `SSH_ASKPASS` setting must keep working.
- The channel that carries a password must not be reachable by other users or by a remote server.
- Answers are never logged.

## Considered options

1. Leave prompts off: credential helpers and agents only; failures say so.
2. Run git in a pseudo-terminal and parse its prompts.
3. Register Canopy's own executable as `GIT_ASKPASS` and `SSH_ASKPASS`; it asks the running app
   over a local socket and prints the answer.
4. Write a Git credential helper and an SSH agent.

## Decision outcome

Chosen option: "3. Canopy's own askpass", because both `git` and OpenSSH already define it as the
extension point for non-terminal prompts, it needs no pseudo-terminal parsing, and it is one
mechanism for passwords, passphrases and host-key questions. Option 1 leaves first sign-in and
passphrases broken; option 2 is fragile across git versions and platforms; option 4 is two
products.

How it works (`crates/canopy-app/src/askpass.rs`, `canopy_core::cli::set_askpass`):

- The running app opens a listener on `127.0.0.1` and registers its own executable as
  `GIT_ASKPASS` and `SSH_ASKPASS` (`SSH_ASKPASS_REQUIRE=force`) for every later `git` command, with
  `CANOPY_ASKPASS_PORT` and a random `CANOPY_ASKPASS_TOKEN` in the environment. A `GIT_ASKPASS`,
  `SSH_ASKPASS` or `SSH_ASKPASS_REQUIRE` the user already set wins.
- When git or ssh runs `canopy "<prompt>"`, `main` detects that (`askpass::is_client`), connects,
  and the app shows a dialog; the answer goes back on standard output. Prompts are classified by
  `PromptKind`; anything not known to be harmless (only user-name questions are) is typed hidden.
- The listener accepts only loopback connections that present the token, serves at most
  `MAX_CONNECTIONS` prompts at once, requires a request within `REQUEST_DEADLINE`, and withdraws
  a prompt whose git process went away. Answers are never logged.

### Consequences

- **Good:** HTTPS sign-in, SSH passphrases and unknown host keys work inside Canopy for every
  operation, with no terminal.
- **Bad:** the token is in the environment of every `git` command Canopy runs, so processes git
  starts (hooks, credential helpers, `core.sshCommand`, Git LFS) can read it and could show a
  question in Canopy: the same trust a terminal gives them. Other local users cannot.
- **Bad:** the listener and its protocol are a local attack surface and are security-sensitive;
  changes to `askpass.rs` get a security pass.
- **Bad:** `SSH_ASKPASS_REQUIRE` needs OpenSSH 8.4 or later; older ssh falls back to its own
  behaviour.

## Related

- ADR 0001: why `git` runs as a child process at all.
- ADR 0004: hosting API tokens are a separate path and never go through this prompt.
- `crates/canopy-core/src/cli.rs`: `Askpass`, `set_askpass`, `apply_env`.
