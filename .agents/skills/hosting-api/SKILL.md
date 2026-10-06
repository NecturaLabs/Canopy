---
name: hosting-api
description: Recipe for adding or changing a GitHub or GitLab API call in Canopy (crates/canopy-hosting and the hosting thread in crates/canopy-app/src/hosting.rs), such as a new pull or merge request action or field. Covers what the product rules allow, the provider functions and Client method, tokens and the consent and settings gates, rate limits, timeouts, redaction, running off the UI thread, tests against a fake local server, and the privacy policy update. Use whenever a change makes Canopy send an HTTP request to a hosting provider or reads a new part of an API response.
license: Proprietary (NecturaLabs); see LICENSE
---

# Add a hosting API call

`crates/canopy-hosting` is Canopy's only HTTP client (plus Sentry's transport in
`crash_reports.rs`). It is security-sensitive: it holds the user's token. Read ADR 0003 (what may
be contacted) and ADR 0004 (tokens) in `docs/adr/` before the first change in a session. Work
through the steps in order; a pull or merge request feature touches both providers.

## 0. Is it allowed?

- Allowed without asking: another endpoint of the pull request (GitHub) or merge request (GitLab)
  API of the repository's own host, called with the user's token, on behalf of a user action.
- Ask the owner first: any other host, another provider, a new category of data (issues, user
  profiles beyond the approved author pictures, other image downloads, organisation data), a call that runs without a user
  action or the approved picture/update checks, polling, or storing a token somewhere new. Telemetry and
  analytics are out entirely. Author pictures are approved under ADR 0008: the forge, GitHub's
  avatar CDN, and exact Gravatar/Libravatar image hosts returned by GitLab, after consent and while
  download_avatars and hosting_api are on. Follow the existing validators and twelve-hour cache
  in avatars.rs; never send a token to an image URL (AGENTS.md, Product rules).
- Anything sent to a third party must be true in `docs/legal/privacy-policy.md` (step 6).

## 1. Data (`crates/canopy-hosting/src/model.rs`)

Add provider-neutral plain data the UI needs (`RequestSummary`, `RequestDetails`, `Review`,
`Check`, `Comment`, `NewRequest` are the examples). Keep provider JSON in private `serde`
structs in `github.rs` or `gitlab.rs` and map them: unknown fields are ignored, missing optional
ones default, timestamps go through `client::unix`. A model type never holds the token.

## 2. Provider functions (`github.rs`, `gitlab.rs`)

- One `pub(crate) fn name(http: &Http, repo_or_project: &str, ...) -> Result<T>` per provider,
  using `http.json(Method, path, body)` for a response body and `http.call(...)` for none.
  `repo`/`project` is the path prefix the `Client` built (`/repos/owner/repo`,
  `/projects/group%2Fproject`); encode user-supplied path segments with `http::encode`.
- Lists use `PER_PAGE` (100) and page until a short page, bounded by the caller's `limit`.
- A secondary request (reviews, checks) goes through `client::optional` so one forbidden endpoint
  does not hide the rest; the primary request failing is an error.
- A write that acts on something the user looked at carries its precondition (as `merge` carries
  the head SHA the user reviewed and refuses with `error::head_changed` when it moved).
- Both providers get the call, or the missing one is a stated, documented limitation.

## 3. The `Client` method (`client.rs`)

Add a method on `Client` that matches on `ForgeKind` and calls the two functions. Its doc comment
says what it sends and what it needs (a token for any write). Do not give `Client` a way to take
a full URL, a different host or a header: every request goes through `Http`, which sends the
token only as `Authorization: Bearer` to the API base of the repository's own host.

## 4. HTTP rules (all in `http.rs`; do not work around them)

- **Timeouts**: 10 s to connect, 30 s in total, blocking. Every call may take that long, so it
  runs on a background thread, never the UI thread (step 5).
- **Token**: only in the header, never in a URL, a log line, a toast, an error string, a `Debug`
  impl or a file. A new type that holds an `Http`, a token or an API base gets a hand-written
  `Debug` that redacts them (see `Client` and `Http`; `redact_userinfo` hides credentials a user
  put in a configured API root). Log method, path, status and duration only, never bodies or
  headers; pass any user-supplied URL through `canopy_core::cli::redact_url`.
- **Redirects**: authorization headers are forwarded to the same host only
  (`RedirectAuthHeaders::SameHost`); do not change that.
- **GraphQL** goes only through `Http::graphql` (same host as the API base, no redirects, same
  headers and limits), only with a token. A new GraphQL field must exist in every edition the
  provider ships (GitLab CE has no `approvalsLeft`: an unknown field fails the whole query).
- **Rate limits**: `map_status` turns a 429, or a GitHub 403 with an exhausted quota or a
  `Retry-After`, into `Error::RateLimited { reset }`, from `x-ratelimit-reset`, `ratelimit-reset`
  or `retry-after`. Show the reset time (the error's text does); never retry in
  a loop, and never poll faster than a user action.
- **Errors**: `Error` carries the server's own message (`server_message`), made plain text
  (`plain_text`); a new variant's `Display` must not echo a header or token. The app shows
  `e.to_string()` in a toast.
- A link in an API response is data. Open it only through `platform::open_url`, which accepts web
  addresses only.

## 5. The app side (`crates/canopy-app/src/hosting.rs`, `tab.rs`, `app/`, `views/requests.rs`)

- Add a `HostJob` variant (and, for a result the UI shows, a `HostEvent` variant), handle it in
  `hosting::serve` on the per-forge hosting thread, and apply the event in `tab.rs`. Views and
  `app/` only send jobs and render tab state; they never call `Client`.
- The consent and settings gates are structural: `app/mod.rs` starts the hosting thread
  (`RepoTab::ensure_host` in `tab.rs`) only after `ui::legal::pending(&settings)` is `None`
  (Terms and Privacy Policy accepted, crash-report question answered) and only while
  `settings.hosting_api` is on. A call outside the `HostWorker` thread would bypass both: do not
  add one. A change that keeps a request alive after the setting is turned off is a defect:
  the thread's `Client` carries the worker's switch (`Client::with_switch`), which every request
  checks right before it is sent.
- Tokens come from `hosting::resolve_token` (saved in `tokens.json`, then the host-matched
  environment variable, then git's credential helper); the requests view shows an "Add a token…"
  prompt when the server refuses without one. Do not read tokens anywhere else, and do not hand
  one to any code outside the hosting thread.
- Destructive or hard-to-undo actions (merge, close) confirm first, like any other
  (AGENTS.md, Conventions).

## 6. Tests

- `crates/canopy-hosting/tests/api.rs` drives a real `Client` against a local HTTP server bound to
  `127.0.0.1` (`server::Server::start`, then `route`/`route_with_headers`; `requests()` and
  `writes()` return what it received). Use `github(&server, Some(TOKEN))` and
  `gitlab(&server, ...)` with the fake `TOKEN`. Never call the real network or use a real token.
- Cover, for each provider: field mapping from a canned response including an unknown field;
  pagination; the recorded request (method, path, JSON body, `authorization` equal to
  `Bearer <TOKEN>`, and absent without a token); each error mapping (401, 403 with and without
  an exhausted quota, 404, 429 with a reset header, 5xx, malformed JSON giving `Error::Parse`);
  and `assert_no_token_leak(&client)` for any new `Debug` surface.
- Pure mapping helpers get unit tests in their module. Run `cargo test -p canopy-hosting`, then
  `cargo test -p canopy-app` for the app side.
- To see the UI without a network or token, run `python3 tools/mock-hosting.py <port>` and start
  Canopy with `CANOPY_HOSTING_API_BASE=http://127.0.0.1:<port>` (only a plain loopback address is
  honoured). The mock speaks GitHub's shape, including `POST /graphql` for the list's checks and
  reviews; a new endpoint or GraphQL field needs a route or canned data there too. Screenshot the
  result with `--view requests` (`screenshot` skill).

## 7. Privacy, docs and release

- If the call sends or reads anything the Privacy Policy does not already describe (its "GitHub
  and GitLab" section lists what is sent, to whom, and where tokens are stored), update
  `docs/legal/privacy-policy.md` and, when the terms change, `terms-of-service.md`; change the
  `Last updated` line and `ui::legal::PRIVACY_VERSION`/`TERMS_VERSION` together (the
  `versions_match_the_documents` test checks it), which asks users again.
- A new dependency needs `cargo deny check` and `python3 tools/notices.py`; ask the owner first.
- `docs/features.md`, `CHANGELOG.md` and, for a new module or type, `docs/architecture.md`.
- Run `/security-review` on the diff, then `tools/check.sh`, then the `canopy-review` agent
  (`independent-review` skill).

## Done when

Both providers are implemented and tested against the fake server, including the error and
no-token cases; nothing outside `HostWorker` calls the client; the setting off or the consent
unanswered sends nothing; the legal documents are accurate; and the security pass and review are
clean.
