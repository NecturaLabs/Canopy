# ADR 0003: Which network services Canopy contacts, and the consent that gates them

**Status:** partially superseded by [ADR 0008](0008-author-profile-pictures.md) for author pictures
**Date:** 2026-10-01

## Context and problem statement

Canopy v0.1 followed a strict rule: no external services. It talked only to the local `git`
executable and local files; GitHub and GitLab features were limited to recognising remote URLs
offline and opening web pages in the browser. The owner then asked for three things that need the
network: pull and merge requests inside Canopy, a way to learn about new releases, and a way to
hear about crashes. Telemetry, analytics, avatar downloads and every other service stay out.

That makes Canopy a program that sends HTTP requests, handles credentials itself and can send
data about its users to a third party. One closed list of what may leave the machine, and one
consent mechanism, has to hold for every later change.

## Decision drivers

- Users trust a Git client with their credentials and repositories: nothing leaves the machine
  that they did not ask for or agree to, and the documents they agreed to must stay true.
- No backend of our own to run, secure or pay for.
- Self-hosted GitLab, including behind a corporate CA, must work.
- Every network feature has a setting that turns it off, and Canopy works fully without all of
  them.
- Git traffic stays on the user's `git` (ADR 0001); this decision adds, it does not move it.

## Considered options

Pull and merge requests:

1. Stay offline: web links only (the v0.1 rule).
2. The REST API of the repository's own host (GitHub, GitLab), called from Canopy with a token the
   user supplies.
3. Shell out to `gh` and `glab` and reuse their authentication.
4. A NecturaLabs backend or OAuth proxy between Canopy and the providers.

Update check:

1. None.
2. The GitHub Releases API of the project repository.
3. `git ls-remote --tags` against the project repository.
4. A small JSON manifest per channel in a public repository, fetched anonymously.
5. An update server of our own.

Crash reports:

1. None; the user attaches the local crash file to a bug report by hand.
2. Opt-in reports to Sentry, scrubbed before sending.
3. Opt-out reports.
4. A crash-report service of our own.

## Decision outcome

Chosen: pull and merge requests **2**, update check **4**, crash reports **2**. Together they are
the complete list; anything else is a product-rule change that needs the owner.

| Service | What is sent | Where | Gate |
|---|---|---|---|
| GitHub or GitLab REST API | Requests for the repository's own project, with the user's token (ADR 0004) | The API host of the repository's remote only: `api.github.com` for github.com, `https://<host>/api/v4` for gitlab.com and self-hosted GitLab | Setting `hosting_api` (Pull and merge requests), default on; works without a token for public projects |
| Update manifest | An anonymous GET, `User-Agent: Canopy/<version>` | `raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/{latest,beta}.json` (`updater::UPDATE_FEED`) | Setting `check_updates`, default on; at most once a day; nothing is downloaded or installed |
| Sentry crash reports | Panic message and location, stack trace, version, OS and architecture, a random session id; scrubbed by `crash_reports::scrub` | Canopy's Sentry project (EU region) | Opt-in, asked once; off until the user says yes; `crash_reports::DATA_VERSION` re-asks when reports carry more |

- **Option 2 for requests** is the smallest change that meets the owner's request: the only new
  endpoints are the API of the host a remote already points at. Option 3 makes the feature depend
  on two extra programs being installed and logged in, and on parsing their output across
  versions; option 4 is a backend, which the product rules exclude.
- **Update check 4** needs no HTTP API quota, no credentials and no parsing of a provider's
  schema, and the manifest carries what the app needs (version, notes, per-platform URL and
  SHA-256). Options 2 and 3 depend on the project repository being readable by every user; the
  release workflow publishes archives and the manifest to the public `NecturaLabs/app-updates`
  repository instead (`.github/workflows/release.yml`, `tools/update-manifest.sh`). Option 5 is a
  server.
- **Crash reports 2** gives the team crash data without a service of our own, and keeps consent
  honest: the policy says nothing is sent before the user answers. No logging integration feeds
  Sentry, so logs never leave the machine.

Rules every change follows:

- **Consent first.** Until the user has accepted the Terms of Service and Privacy Policy and
  answered the crash-report question, Canopy contacts no service (`ui::legal::pending`; the
  hosting thread and the start-up update check both wait for it).
- **The legal documents are part of the feature.** `docs/legal/privacy-policy.md` and
  `terms-of-service.md` describe exactly what leaves the machine. A change to that updates them,
  and the `Last updated` date with `ui::legal::TERMS_VERSION` / `PRIVACY_VERSION` (a test checks
  they match), so users are asked again. A change to what a crash report carries also bumps
  `crash_reports::DATA_VERSION`.
- **All HTTP lives in `crates/canopy-hosting`** (`ureq` with rustls and the platform verifier, so
  a corporate CA in the OS trust store works; API calls time out after 10 s to connect and 30 s
  in total, the update manifest after 10 s and 20 s), the update manifest included, except Sentry's own transport, which is configured in
  `crates/canopy-app/src/crash_reports.rs`. HTTP calls run on a background thread, never the UI
  thread.
- **No credential in a URL Canopy builds or logs;** URLs and git arguments are redacted before
  logging (`canopy_core::cli::redact_url`, `cli::loggable`).
- Every setting that turns a feature off makes the feature send nothing, immediately.

### Consequences

- **Good:** pull and merge request work happens inside Canopy; no service of ours sits in the
  path of the user's code or tokens; the network surface is small enough to review completely.
- **Good:** users can read, in plain language, what is sent, and the code refuses to send before
  they have.
- **Bad:** `canopy-hosting`, token handling, `forge.rs`, the updater and `crash_reports.rs` become
  security-sensitive and need a security pass before review.
- **Bad:** a TLS and HTTP dependency tree (rustls, ureq, the Sentry SDK) to keep current under
  `cargo deny`, and a new third-party notice entry for each change (`tools/notices.py`).
- **Bad:** unauthenticated GitHub API use is tightly rate-limited; the client reports the reset
  time instead of a bare failure.
- **Bad:** the update feed is a second repository (`NecturaLabs/app-updates`) that the release
  workflow must be able to push to (`APP_UPDATES_TOKEN`); without it a release is published but
  running copies are not told.

## Related

- ADR 0001: Git network traffic stays on the `git` executable.
- ADR 0004: where hosting tokens come from and where they may go.
- `docs/legal/privacy-policy.md`, `docs/legal/terms-of-service.md`: what users agreed to.
- `crates/canopy-hosting/src/lib.rs`, `crates/canopy-hosting/src/updates.rs`,
  `crates/canopy-app/src/updater.rs`, `crates/canopy-app/src/crash_reports.rs`,
  `crates/canopy-app/src/ui/legal.rs`.
- `AGENTS.md`, Product rules, "Network": the maintained statement of the rule; it must match this
  record.
