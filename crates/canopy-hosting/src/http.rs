//! The HTTP layer: one `ureq` agent per client, the common headers, and mapping HTTP failures
//! to [`Error`] with the server's own message.

use std::fmt;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use canopy_core::forge::ForgeKind;
use serde::de::DeserializeOwned;
use serde_json::Value;
use ureq::config::RedirectAuthHeaders;
use ureq::http::{HeaderMap, Response};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Body, RequestBuilder};

use crate::error::{Error, Result};
use crate::token::TOKEN_PREFIXES;

/// The HTTP status codes the clients branch on, and the class of success.
pub(crate) mod status {
    pub const OK: u16 = 200;
    pub const ACCEPTED: u16 = 202;
    pub const UNAUTHORIZED: u16 = 401;
    pub const FORBIDDEN: u16 = 403;
    pub const NOT_FOUND: u16 = 404;
    pub const CONFLICT: u16 = 409;
    pub const TOO_MANY_REQUESTS: u16 = 429;
    /// The 2xx codes: from 200 up to, not including, 300.
    pub const SUCCESS: std::ops::Range<u16> = 200..300;
}

/// Largest profile lookup response read.
const MAX_PROFILE_BODY: u64 = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest server message kept from a non-JSON error body.
const MAX_MESSAGE: usize = 300;
/// Largest API response body read (ureq's own default for a whole-body read, stated here so
/// every request, the GraphQL query included, visibly shares it).
const MAX_BODY: u64 = 10 * 1024 * 1024;

/// The REST API version `api.github.com` is asked for, the latest GitHub publishes. None of its
/// breaking changes touches an endpoint or a field Canopy uses.
const GITHUB_API_VERSION: &str = "2026-03-10";

/// The REST API version sent to every GitHub host but `api.github.com`. GitHub Enterprise Server
/// 3.20 and older know only this one and answer another with an error; GitHub supports it until
/// 2028-03-10. Move a server to [`GITHUB_API_VERSION`] only once its releases accept it.
const GITHUB_ENTERPRISE_API_VERSION: &str = "2022-11-28";

/// The API root of github.com itself; any other GitHub root is Enterprise Server (`/api/v3`) or a
/// local test server.
const GITHUB_API_ROOT: &str = "https://api.github.com";

/// The `X-GitHub-Api-Version` for requests to the API at `base` (no trailing slash).
fn github_api_version(base: &str) -> &'static str {
    if base.eq_ignore_ascii_case(GITHUB_API_ROOT) {
        GITHUB_API_VERSION
    } else {
        GITHUB_ENTERPRISE_API_VERSION
    }
}

/// The running build, announced in the `User-Agent` of every request.
static BUILD: OnceLock<String> = OnceLock::new();

/// Tells this crate which Canopy build is running (its build number, or `dev`). The application
/// calls it once at start-up; without it requests carry this crate's own version.
pub fn set_build(build: &str) {
    let _ = BUILD.set(build.to_owned());
}

/// `Canopy/<build>`, the only identification a request carries.
pub(crate) fn user_agent() -> String {
    let build = BUILD
        .get()
        .map_or(env!("CARGO_PKG_VERSION"), String::as_str);
    format!("Canopy/{build}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Patch,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
        }
    }
}

/// An authenticated connection to one provider's API.
pub(crate) struct Http {
    agent: Agent,
    kind: ForgeKind,
    /// API root without a trailing slash, e.g. `https://api.github.com`.
    base: String,
    token: Option<String>,
    /// While this holds `false`, nothing is sent ([`Self::set_switch`]).
    switch: Option<Arc<AtomicBool>>,
}

/// Never prints the token, nor credentials embedded in the API root (see [`redact_userinfo`]).
impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Http")
            .field("kind", &self.kind)
            .field("base", &redact_userinfo(&self.base))
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

/// `url` with any credentials in its authority replaced: `https://user:secret@host/x` becomes
/// `https://<redacted>@host/x`. For `Debug` output and logs of a user-supplied API root.
pub(crate) fn redact_userinfo(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, url),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let Some((_, host)) = authority.rsplit_once('@') else {
        return url.to_owned();
    };
    match scheme {
        Some(scheme) => format!("{scheme}://<redacted>@{host}{tail}"),
        None => format!("<redacted>@{host}{tail}"),
    }
}

impl Http {
    pub(crate) fn new(kind: ForgeKind, base: &str, token: Option<String>) -> Self {
        let base = base.trim_end_matches('/').to_owned();
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let mut config = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(TOTAL_TIMEOUT))
            .user_agent(user_agent())
            // GitHub redirects renamed repositories; keep the token on the same host only.
            .redirect_auth_headers(RedirectAuthHeaders::SameHost)
            .tls_config(tls);
        if is_loopback_base(&base) {
            // A local server (tests, a developer's stub) never goes through a proxy.
            config = config.proxy(None);
        }
        let agent = crate::transport::agent(config.build());
        let token = token.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        Self {
            agent,
            kind,
            base,
            token,
            switch: None,
        }
    }

    /// From now on, sends a request only while `on` holds `true`; otherwise the call fails with
    /// [`Error::Stopped`] and nothing leaves the machine.
    pub(crate) fn set_switch(&mut self, on: Arc<AtomicBool>) {
        self.switch = Some(on);
    }

    /// Whether the switch is off, so nothing may be sent.
    fn stopped(&self) -> bool {
        self.switch
            .as_ref()
            .is_some_and(|on| !on.load(Ordering::SeqCst))
    }

    pub(crate) fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// Normalizes diagnostic text before replacing this client's token, without truncation.
    fn hide_token(&self, text: &str) -> String {
        let text = plain_text(text, usize::MAX);
        match &self.token {
            Some(token) if token.len() >= MIN_TOKEN_LEN => text.replace(token.as_str(), REDACTED),
            _ => text,
        }
    }

    /// Parses JSON without quoting response values in type errors; syntax errors retain their
    /// parser diagnostic. Both include the location of the failure.
    fn parse<T: DeserializeOwned>(&self, bytes: &[u8], what: &str) -> Result<T> {
        serde_json::from_slice(bytes).map_err(|e| {
            // serde Debug-escapes quoted strings, so stripping invisible characters from
            // its message cannot recover an echoed token for redaction.
            let detail = if e.is_data() {
                format!(
                    "response data has an unexpected format (values <redacted>) at line {} column {}",
                    e.line(),
                    e.column()
                )
            } else {
                e.to_string()
            };
            Error::Parse(shorten(&self.hide_token(&format!("{what}: {detail}"))))
        })
    }

    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    /// `text` with the profile request's URL and path, and anything else private to it (an
    /// author's email address), replaced. Profile paths may contain an author's email, which must
    /// not appear in logs or notifications.
    fn hide_request(&self, text: &str, path: &str, private: &[&str]) -> String {
        let url = plain_text(&format!("{}{}", self.base, path), usize::MAX);
        let mut text = plain_text(text, usize::MAX).replace(&url, "[profile URL]");
        if !path.is_empty() {
            let path = plain_text(path, usize::MAX);
            if !path.is_empty() {
                text = text.replace(&path, "[profile path]");
            }
        }
        for value in private.iter().filter(|v| !v.is_empty()) {
            let value = plain_text(value, usize::MAX);
            if !value.is_empty() {
                text = text.replace(&value, "[private]");
            }
        }
        text
    }

    /// A lookup diagnostic with the private request URL and token hidden.
    fn profile_error(&self, context: &str, detail: &str, path: &str, private: &[&str]) -> Error {
        let detail = self.hide_request(detail, path, private);
        Error::Network(shorten(&self.hide_token(&format!("{context}: {detail}"))))
    }

    /// Profile lookup: no redirects, bounded JSON, and no author email in logs. Sends nothing
    /// while the switch is off. `private` lists text (the author's email, plain and encoded) that
    /// an error must not repeat, even when the server echoes it.
    pub(crate) fn avatar_json<T: DeserializeOwned>(
        &self,
        path: &str,
        private: &[&str],
    ) -> Result<T> {
        if self.stopped() {
            return Err(Error::Stopped);
        }
        let mut response = self
            .headers(self.agent.get(format!("{}{}", self.base, path)))
            .config()
            .max_redirects(0)
            .build()
            .call()
            .map_err(|e| {
                self.profile_error("Profile lookup failed", &e.to_string(), path, private)
            })?;
        let status = response.status().as_u16();
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_PROFILE_BODY)
            .read_to_vec()
            .map_err(|e| {
                self.profile_error(
                    "Could not read the profile response",
                    &e.to_string(),
                    path,
                    private,
                )
            })?;
        if !status::SUCCESS.contains(&status) {
            return Err(map_status_with_token(
                status,
                response.headers(),
                &bytes,
                self.kind,
                self.token.as_deref(),
                &|message| self.hide_request(message, path, private),
            ));
        }
        self.parse(&bytes, "profile response")
    }

    /// Sends a request and parses a successful JSON response.
    pub(crate) fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let bytes = self.send(method, path, body)?;
        self.parse(&bytes, path)
    }

    /// Sends a request whose response body does not matter.
    pub(crate) fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<()> {
        self.send(method, path, body).map(drop)
    }

    /// Sends `method path` (path relative to the API root, query included) and returns the body
    /// of a 2xx response, or the mapped error. Sends nothing while the switch is off.
    fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Vec<u8>> {
        if self.stopped() {
            return Err(Error::Stopped);
        }
        let url = format!("{}{}", self.base, path);
        let payload = match body {
            Some(v) => Some(serde_json::to_vec(v).map_err(|e| Error::Parse(e.to_string()))?),
            None => None,
        };
        let started = std::time::Instant::now();
        let result = match method {
            Method::Get => self.headers(self.agent.get(&url)).call(),
            Method::Post => self.send_with_body(self.agent.post(&url), payload.as_deref()),
            Method::Put => self.send_with_body(self.agent.put(&url), payload.as_deref()),
            Method::Patch => self.send_with_body(self.agent.patch(&url), payload.as_deref()),
        };
        self.receive(method, path, started, result)
            .map(|(_, bytes)| bytes)
    }

    /// Reads the response to `method path`, sent at `started`: the headers and body of a 2xx
    /// response, or the mapped error. Logs method, path, status and duration, never a body.
    fn receive(
        &self,
        method: Method,
        path: &str,
        started: std::time::Instant,
        result: Result<Response<Body>, ureq::Error>,
    ) -> Result<(HeaderMap, Vec<u8>)> {
        let mut response = result.map_err(|e| {
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            // The transport's message can quote the server (a malformed redirect target).
            let error = shorten(&self.hide_token(&e.to_string()));
            tracing::warn!(
                method = method.as_str(),
                path,
                ms,
                error,
                "hosting request failed"
            );
            Error::Network(error)
        })?;
        let status = response.status().as_u16();
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let remaining = response
            .headers()
            .get("x-ratelimit-remaining")
            .or_else(|| response.headers().get("ratelimit-remaining"))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        tracing::debug!(method = method.as_str(), path, status, ms, rate_limit_remaining = ?remaining, "hosting request");
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|e| Error::Network(shorten(&self.hide_token(&e.to_string()))))?;
        if status::SUCCESS.contains(&status) {
            Ok((response.headers().clone(), bytes))
        } else {
            let err = map_status_with_token(
                status,
                response.headers(),
                &bytes,
                self.kind,
                self.token.as_deref(),
                &str::to_owned,
            );
            tracing::warn!(method = method.as_str(), path, status, ms, error = %err, "hosting request rejected");
            Err(err)
        }
    }

    /// The GraphQL endpoint on the API's own host: `/graphql` beside a root API
    /// (`https://api.github.com`), `/api/graphql` in place of a versioned REST root
    /// (`https://gitlab.example.com/api/v4`, GitHub Enterprise's `/api/v3`). Returns the full URL
    /// and its path from the host's root, which is all a log line names.
    fn graphql_endpoint(&self) -> (String, &'static str) {
        let versioned = self
            .base
            .strip_suffix("/api/v4")
            .or_else(|| self.base.strip_suffix("/api/v3"));
        let (root, path) = match versioned {
            Some(root) => (root, "/api/graphql"),
            None => (self.base.as_str(), "/graphql"),
        };
        (format!("{root}{path}"), path)
    }

    /// Sends one GraphQL `query` with `variables` to the API's GraphQL endpoint on the same host
    /// ([`Self::graphql_endpoint`]) and returns the response's `data`.
    ///
    /// The request carries the same `User-Agent` and token as every other, follows no redirect,
    /// reads the body with the same limit as [`Self::json`] and logs the same fields (method,
    /// path, status, duration). A failed status maps as for any request ([`map_status`]). In a
    /// successful response, errors with a message or type saying a rate limit was hit, or that
    /// come without data and with no requests left or a `retry-after`, become
    /// [`Error::RateLimited`]; other errors
    /// fail the call only when no `data` came back (GitHub answers a missing pull request with
    /// an error beside the rest of the data). Sends nothing while the switch is off.
    pub(crate) fn graphql<T: DeserializeOwned>(&self, query: &str, variables: &Value) -> Result<T> {
        let (url, path) = self.graphql_endpoint();
        let payload = serde_json::to_vec(&serde_json::json!({
            "query": query,
            "variables": variables,
        }))
        .map_err(|e| Error::Parse(e.to_string()))?;
        let mut req = self
            .agent
            .post(&url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json");
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        if self.stopped() {
            return Err(Error::Stopped);
        }
        let started = std::time::Instant::now();
        let result = req.config().max_redirects(0).build().send(&*payload);
        let (headers, bytes) = self.receive(Method::Post, path, started, result)?;
        let envelope: GraphqlEnvelope<T> = self.parse(&bytes, "GraphQL response")?;
        // GitHub answers an exhausted GraphQL quota with 200, an error and no requests left, and
        // a secondary limit with 200 or 403, an error saying so and perhaps `retry-after`. An
        // answer with data took the last request but is still good: its errors are about
        // something else unless one says it is the limit.
        let says_limit = envelope.errors.iter().any(|e| {
            e.kind.as_deref() == Some("RATE_LIMITED")
                || e.message.to_ascii_lowercase().contains("rate limit")
        });
        let out_of_requests = header_i64(&headers, "x-ratelimit-remaining") == Some(0)
            || headers.contains_key("retry-after");
        let limited = !envelope.errors.is_empty()
            && (says_limit || (out_of_requests && envelope.data.is_none()));
        if limited {
            return Err(Error::RateLimited {
                reset: rate_limit_reset(&headers),
            });
        }
        envelope.data.ok_or_else(|| {
            let messages: Vec<&str> = envelope
                .errors
                .iter()
                .map(|e| e.message.trim())
                .filter(|m| !m.is_empty())
                .collect();
            let message = shorten(&self.hide_token(&messages.join("; ")));
            Error::Api {
                status: status::OK,
                message: if message.is_empty() {
                    "The GraphQL response carried no data".to_owned()
                } else {
                    message
                },
            }
        })
    }

    fn send_with_body(
        &self,
        req: RequestBuilder<ureq::typestate::WithBody>,
        payload: Option<&[u8]>,
    ) -> Result<Response<Body>, ureq::Error> {
        let req = self.headers(req);
        match payload {
            Some(bytes) => req.header("Content-Type", "application/json").send(bytes),
            None => req.send_empty(),
        }
    }

    fn headers<B>(&self, mut req: RequestBuilder<B>) -> RequestBuilder<B> {
        req = match self.kind {
            ForgeKind::GitHub => req
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", github_api_version(&self.base)),
            ForgeKind::GitLab => req.header("Accept", "application/json"),
        };
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        req
    }
}

/// Whether `url` is a plain-HTTP API root on this machine, such as `http://127.0.0.1:8080` or
/// `http://[::1]:8787/api` (a test server or a developer's stub).
///
/// Deliberately strict, so a URL that only looks local cannot pass: the scheme must be exactly
/// `http`; the URL must contain no `@` at all, so there are no credentials in the authority
/// (which defeats `http://localhost:@evil.example/`, whose real host is `evil.example`); the
/// authority (everything up to the first `/`, `?`, `#` or `\`) must be exactly `127.0.0.1`,
/// `localhost` or `[::1]`, optionally followed by `:` and a port number; whatever follows must
/// be empty or a path starting with `/`. Upper case, trailing dots,
/// `https` and anything else answer `false`, which is the safe side.
pub fn is_loopback_base(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let end = rest.find(['/', '?', '#', '\\']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if !(tail.is_empty() || tail.starts_with('/')) || url.contains('@') {
        return false;
    }
    let (host, port) = if let Some(after) = authority.strip_prefix("[::1]") {
        ("[::1]", after)
    } else {
        match authority.find(':') {
            Some(i) => authority.split_at(i),
            None => (authority, ""),
        }
    };
    let port_ok = match port.strip_prefix(':') {
        None => port.is_empty(),
        Some(digits) => {
            !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && digits.parse::<u16>().is_ok()
        }
    };
    port_ok && matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// A GraphQL response: `data`, `errors`, or both.
#[derive(serde::Deserialize)]
struct GraphqlEnvelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct GraphqlError {
    message: String,
    /// GitHub's error type, such as `RATE_LIMITED` or `NOT_FOUND`.
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn header_i64(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.trim().parse::<i64>().ok()
}

/// The longest wait for a rate limit taken at the server's word: a day.
const MAX_RETRY_AFTER: i64 = 86_400;

/// When a rate limit resets, in unix seconds between now and [`MAX_RETRY_AFTER`] ahead: the
/// provider's reset header (unix seconds), else `Retry-After` (seconds from now).
pub(crate) fn rate_limit_reset(headers: &HeaderMap) -> Option<i64> {
    let now = jiff::Timestamp::now().as_second();
    let latest = now.saturating_add(MAX_RETRY_AFTER);
    header_i64(headers, "x-ratelimit-reset")
        .or_else(|| header_i64(headers, "ratelimit-reset"))
        .map(|at| at.clamp(now, latest))
        .or_else(|| {
            header_i64(headers, "retry-after")
                .map(|secs| now.saturating_add(secs.clamp(0, MAX_RETRY_AFTER)))
        })
}

/// Maps an unsuccessful response to an [`Error`], redacting the client's token only after
/// JSON escapes and invisible characters are normalized. Never truncate before token removal.
fn map_status_with_token(
    status: u16,
    headers: &HeaderMap,
    body: &[u8],
    kind: ForgeKind,
    token: Option<&str>,
    hide: &dyn Fn(&str) -> String,
) -> Error {
    let message =
        server_message_unbounded(body).unwrap_or_else(|| default_reason(status).to_owned());
    let message = match token {
        Some(token) if token.len() >= MIN_TOKEN_LEN => message.replace(token, REDACTED),
        _ => message,
    };
    // Before the cut, so a request path that straddles it cannot be left half hidden.
    let message = hide(&message);
    let message = shorten(&message);
    match status {
        status::UNAUTHORIZED => Error::Unauthorized(message),
        status::FORBIDDEN => {
            let quota_exhausted = header_i64(headers, "x-ratelimit-remaining") == Some(0);
            // GitHub's secondary rate limits answer 403 with Retry-After.
            let secondary = kind == ForgeKind::GitHub
                && (headers.contains_key("retry-after")
                    || message.to_ascii_lowercase().contains("rate limit"));
            if quota_exhausted || secondary {
                Error::RateLimited {
                    reset: rate_limit_reset(headers),
                }
            } else {
                Error::Unauthorized(message)
            }
        }
        status::NOT_FOUND => Error::NotFound(message),
        status::TOO_MANY_REQUESTS => Error::RateLimited {
            reset: rate_limit_reset(headers),
        },
        _ => Error::Api { status, message },
    }
}

/// Message extraction without a client token, used for diagnostic tests.
#[cfg(test)]
pub(crate) fn server_message(body: &[u8]) -> Option<String> {
    server_message_unbounded(body).map(|message| shorten(&message))
}

#[cfg(test)]
pub(crate) fn map_status(status: u16, headers: &HeaderMap, body: &[u8], kind: ForgeKind) -> Error {
    map_status_with_token(status, headers, body, kind, None, &str::to_owned)
}

fn default_reason(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Request failed",
    }
}

/// The human-readable message in an error body: GitHub's `message` plus `errors[]`, GitLab's
/// `message` (a string, a list, or a field → messages map), `error` and `error_description`.
/// Normalize the full text before redacting client-specific tokens or truncating it.
fn server_message_unbounded(body: &[u8]) -> Option<String> {
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        let text = String::from_utf8_lossy(body);
        let text = text.trim();
        if text.starts_with('<') {
            return None;
        }
        return Some(plain_text(text, usize::MAX)).filter(|t| !t.is_empty());
    };
    let mut parts: Vec<String> = Vec::new();
    for key in ["message", "error", "error_description", "errors"] {
        if let Some(v) = json.get(key) {
            flatten(v, None, &mut parts);
        }
    }
    parts.dedup();
    Some(plain_text(&parts.join("; "), usize::MAX)).filter(|t| !t.is_empty())
}

/// Collects the strings in an error value. Objects in GitHub's `errors` array contribute their
/// `message`, or `field code` when there is none; GitLab's `{field: [messages]}` maps become
/// `field message`.
fn flatten(v: &Value, field: Option<&str>, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            let s = s.trim();
            if !s.is_empty() {
                out.push(match field {
                    Some(f) => format!("{f} {s}"),
                    None => s.to_owned(),
                });
            }
        }
        Value::Array(items) => items.iter().for_each(|i| flatten(i, field, out)),
        Value::Object(map) => {
            if let Some(m) = map.get("message") {
                flatten(m, None, out);
            } else if let Some(code) = map.get("code").and_then(Value::as_str) {
                let f = map.get("field").and_then(Value::as_str);
                let resource = map.get("resource").and_then(Value::as_str);
                let subject = f.or(resource).unwrap_or_default();
                out.push(format!("{subject} {code}").trim().to_owned());
            } else {
                for (k, inner) in map {
                    flatten(inner, Some(k), out);
                }
            }
        }
        _ => {}
    }
}

/// Server-derived text as an error carries it: [`plain_text`], at most 300 characters.
pub(crate) fn shorten(s: &str) -> String {
    plain_text(s, MAX_MESSAGE)
}

/// What replaces a secret in a server's words.
const REDACTED: &str = "<redacted>";

/// The shortest token [`Http::hide_token`] looks for: shorter text is too likely to be ordinary
/// words, and no real token is that short.
const MIN_TOKEN_LEN: usize = 8;

/// A server's own words made safe to show among Canopy's and to log.
///
/// Control characters become spaces, characters that reorder or hide text are dropped
/// (bidirectional controls and every other default-ignorable code point, such as zero-width
/// spaces, variation selectors and tags), GitHub and GitLab tokens ([`TOKEN_PREFIXES`]) and the
/// user name and password in URLs are replaced, whitespace collapses to single spaces, and the
/// result is at most `max` characters, ending in an ellipsis when it was cut.
pub fn plain_text(text: &str, max: usize) -> String {
    let visible: String = text
        .chars()
        .filter(|&c| !hides_text(c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let visible = canopy_core::cli::redact_url(&hide_tokens(&visible));
    let words = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().count() <= max {
        return words;
    }
    let mut short: String = words.chars().take(max.saturating_sub(1)).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    short
}

/// `text` with every word that starts with a [`TOKEN_PREFIXES`] prefix replaced.
fn hide_tokens(text: &str) -> String {
    let in_token = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(text.len());
    let mut rest = text.chars();
    let mut after_word = false;
    loop {
        if !after_word && TOKEN_PREFIXES.iter().any(|p| rest.as_str().starts_with(p)) {
            out.push_str(REDACTED);
            rest = rest.as_str().trim_start_matches(in_token).chars();
            continue;
        }
        let Some(c) = rest.next() else {
            return out;
        };
        out.push(c);
        after_word = in_token(c);
    }
}

/// Unicode 16's `Default_Ignorable_Code_Point` characters (which include every bidirectional
/// control) and the interlinear annotation controls: none of them shows, and some reorder text.
fn hides_text(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// Percent-encodes everything outside RFC 3986's unreserved set, so `group/sub/project`
/// becomes `group%2Fsub%2Fproject`.
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::http::HeaderValue;

    /// The `X-GitHub-Api-Version` a GitHub request to `base` carries.
    fn sent_github_version(base: &str) -> String {
        let http = Http::new(ForgeKind::GitHub, base, None);
        let req = http.headers(http.agent.get(format!("{base}/rate_limit")));
        let value = req
            .headers_ref()
            .and_then(|h| h.get("x-github-api-version"));
        value.and_then(|v| v.to_str().ok()).unwrap_or("").to_owned()
    }

    #[test]
    fn github_dot_com_gets_the_latest_api_version_and_enterprise_servers_the_old_one() {
        assert_eq!(
            sent_github_version("https://api.github.com"),
            GITHUB_API_VERSION
        );
        assert_eq!(
            sent_github_version("https://api.github.com/"),
            GITHUB_API_VERSION,
            "a trailing slash is trimmed"
        );
        assert_eq!(GITHUB_API_VERSION, "2026-03-10");
        for ghes in [
            "https://ghe.example.com/api/v3",
            "https://ghe.example.com:8443/api/v3",
            "https://api.github.com.evil.example",
            "http://127.0.0.1:8080",
        ] {
            assert_eq!(
                sent_github_version(ghes),
                GITHUB_ENTERPRISE_API_VERSION,
                "{ghes}"
            );
        }
        assert_eq!(GITHUB_ENTERPRISE_API_VERSION, "2022-11-28");
    }

    #[test]
    fn transport_error_tokens_are_cleaned_before_redaction() {
        let http = Http::new(
            ForgeKind::GitLab,
            "https://gitlab.example/api/v4",
            Some("arbitrarySecretToken1234567".to_owned()),
        );
        let message = shorten(&http.hide_token("bad header: arbitrarySecret\u{200b}Token1234567"));
        assert!(!message.contains("Secret"), "{message}");
        assert!(message.contains("bad header"), "{message}");
    }

    #[test]
    fn parse_errors_redact_normalized_client_tokens() {
        let token = "arbitrarySecretToken1234567";
        let http = Http::new(
            ForgeKind::GitLab,
            "https://gitlab.example/api/v4",
            Some(token.to_owned()),
        );
        for body in [
            br#""arbitrarySecretToken1234567""#.as_slice(),
            br#""arbitrarySecret\u0054oken1234567""#.as_slice(),
            b"\"arbitrarySecret\xe2\x80\x8bToken1234567\"".as_slice(),
        ] {
            let message = http.parse::<u64>(body, "response").unwrap_err().to_string();
            assert!(!message.contains("Secret"), "{message}");
            assert!(message.contains("line 1 column"), "{message}");
        }
    }

    #[test]
    fn profile_errors_preserve_the_cause_without_exposing_request_data() {
        let http = Http::new(
            ForgeKind::GitLab,
            "https://gitlab.example/api/v4",
            Some("private-auth-token-123".to_owned()),
        );
        let path = "/avatar?email=private@example.com&size=64";
        let full_url = format!("{}{path}", http.base());
        let diagnostic =
            format!("connection refused at {full_url}: private-auth-token-123 (path {path})");
        let message = http
            .profile_error("Profile lookup failed", &diagnostic, path, &[])
            .to_string();
        assert!(message.contains("connection refused"), "{message}");
        assert!(!message.contains("private@example.com"), "{message}");
        assert!(!message.contains("private-auth-token-123"), "{message}");
        assert!(!message.contains(full_url.as_str()), "{message}");
    }

    #[test]
    fn profile_errors_redact_private_data_after_removing_invisible_characters() {
        let http = Http::new(ForgeKind::GitLab, "https://gitlab.example/api/v4", None);
        let email = "private@example.com";
        let path = "/avatar?email=private%40example.com&size=64";
        for hidden in ['\u{200B}', '\u{2060}', '\u{FEFF}'] {
            let detail = format!(
                "connection refused: pri{hidden}vate@example.com at https://gitlab.example/api/v4/ava{hidden}tar?email=private%40example.com&size=64"
            );
            let message = http
                .profile_error("Profile lookup failed", &detail, path, &[email])
                .to_string();
            assert!(message.contains("connection refused"), "{message}");
            assert!(!message.contains(email), "{message}");
            assert!(!message.contains("private%40example.com"), "{message}");
            assert!(!message.contains("/avatar?"), "{message}");
        }
    }

    #[test]
    fn plain_text_drops_what_hides_or_reorders_text() {
        // One of each: bidirectional controls, zero-width and other invisible formatting, the
        // Khmer, Mongolian, shorthand and musical format characters, specials and tags.
        let hidden = [
            '\u{00AD}',
            '\u{034F}',
            '\u{061C}',
            '\u{115F}',
            '\u{1160}',
            '\u{17B4}',
            '\u{17B5}',
            '\u{180B}',
            '\u{180E}',
            '\u{180F}',
            '\u{200B}',
            '\u{200F}',
            '\u{202A}',
            '\u{202E}',
            '\u{2060}',
            '\u{2066}',
            '\u{2069}',
            '\u{206F}',
            '\u{3164}',
            '\u{FE00}',
            '\u{FE0F}',
            '\u{FEFF}',
            '\u{FFA0}',
            '\u{FFF0}',
            '\u{FFF8}',
            '\u{FFFB}',
            '\u{1BCA0}',
            '\u{1BCA3}',
            '\u{1D173}',
            '\u{1D17A}',
            '\u{E0001}',
            '\u{E007F}',
            '\u{E0FFF}',
        ];
        for c in hidden {
            assert_eq!(
                plain_text(&format!("a{c}b"), 10),
                "ab",
                "U+{:04X}",
                u32::from(c)
            );
        }
        assert_eq!(
            plain_text("a\u{0}b\tc\u{85}d\u{2028}e\r\n f", 20),
            "a b c d e f"
        );
        assert_eq!(plain_text(" \u{200B}\n", 10), "");
        // The ellipsis counts towards the limit, and never follows a space.
        assert_eq!(plain_text("abcdefghij", 10), "abcdefghij");
        assert_eq!(plain_text("abcdefghijk", 10), "abcdefghi…");
        assert_eq!(plain_text("abcd efghijk", 6), "abcd…");
        assert_eq!(plain_text("ééééé", 3), "éé…");
        // Tokens and URL credentials are replaced, even split by a character that hides.
        assert_eq!(
            plain_text("bad glpat-ab\u{200B}cd_9, ghp_x1 (github_pat_1A_b) é", 100),
            "bad <redacted>, <redacted> (<redacted>) é"
        );
        assert_eq!(
            plain_text("see https://user:secret@host/x", 100),
            "see https://<redacted>@host/x"
        );
        assert_eq!(plain_text("xghp_word", 100), "xghp_word", "not a token");
    }

    #[test]
    fn huge_retry_after_values_neither_overflow_nor_wait_for_ever() {
        let now = jiff::Timestamp::now().as_second();
        let mut headers = HeaderMap::new();
        headers.insert(
            "retry-after",
            HeaderValue::from_static("9223372036854775807"),
        );
        let reset = rate_limit_reset(&headers).expect("a reset");
        assert!((now..=now + 86_400 + 60).contains(&reset), "{reset}");
        // REST answers reach it too.
        for status in [403, 429] {
            assert!(matches!(
                map_status(status, &headers, b"", ForgeKind::GitHub),
                Error::RateLimited { reset: Some(r) } if (now..=now + 86_400 + 60).contains(&r)
            ));
        }
        headers.insert(
            "retry-after",
            HeaderValue::from_static("-9223372036854775808"),
        );
        let reset = rate_limit_reset(&headers).expect("a reset");
        assert!((now..=now + 60).contains(&reset), "{reset}");
    }

    #[test]
    fn reset_times_are_kept_between_now_and_a_day_ahead() {
        let now = jiff::Timestamp::now().as_second();
        for name in ["x-ratelimit-reset", "ratelimit-reset"] {
            let reset = |value: &'static str| {
                let mut headers = HeaderMap::new();
                headers.insert(name, HeaderValue::from_static(value));
                rate_limit_reset(&headers).expect("a reset")
            };
            let far = reset("9223372036854775807");
            assert!((now..=now + 86_400 + 60).contains(&far), "{name}: {far}");
            let past = reset("-9223372036854775808");
            assert!((now..=now + 60).contains(&past), "{name}: {past}");
            assert!((now..=now + 60).contains(&reset("0")), "{name}");
            let soon = now + 600;
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from(soon));
            assert_eq!(rate_limit_reset(&headers), Some(soon), "{name}");
        }
    }

    #[test]
    fn encodes_gitlab_project_ids() {
        assert_eq!(encode("group/sub/project"), "group%2Fsub%2Fproject");
        assert_eq!(encode("a.b_c-d~e"), "a.b_c-d~e");
        assert_eq!(encode("x y+z"), "x%20y%2Bz");
    }

    #[test]
    fn graphql_lives_on_the_api_host() {
        let endpoint = |kind, base: &str| Http::new(kind, base, None).graphql_endpoint();
        assert_eq!(
            endpoint(ForgeKind::GitHub, "https://api.github.com"),
            ("https://api.github.com/graphql".to_owned(), "/graphql")
        );
        assert_eq!(
            endpoint(ForgeKind::GitHub, "https://ghe.corp/api/v3/"),
            ("https://ghe.corp/api/graphql".to_owned(), "/api/graphql")
        );
        assert_eq!(
            endpoint(ForgeKind::GitLab, "https://gitlab.corp:8443/api/v4"),
            (
                "https://gitlab.corp:8443/api/graphql".to_owned(),
                "/api/graphql"
            )
        );
        assert_eq!(
            endpoint(ForgeKind::GitLab, "http://127.0.0.1:4000"),
            ("http://127.0.0.1:4000/graphql".to_owned(), "/graphql")
        );
        // Whatever the root, the endpoint keeps its scheme, host and port: only the path after
        // the authority changes, and nothing but the API root goes into it.
        for (kind, base) in [
            (ForgeKind::GitHub, "https://api.github.com"),
            (ForgeKind::GitLab, "https://gitlab.com/api/v4"),
            (ForgeKind::GitLab, "https://gitlab.corp:8443/api/v4"),
            (ForgeKind::GitLab, "https://gitlab.corp/sub/api/v4"),
            (ForgeKind::GitHub, "https://ghe.corp/api/v3"),
            (ForgeKind::GitLab, "https://evil.example/api/v4/api/v4"),
        ] {
            let (url, _) = endpoint(kind, base);
            // `scheme://host:port`: everything before the third `/`.
            let authority = |u: &str| u.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
            assert_eq!(authority(&url), authority(base), "{base} -> {url}");
            assert!(
                url.starts_with(base.trim_end_matches("/api/v4").trim_end_matches("/api/v3")),
                "{base} -> {url}"
            );
        }
    }

    #[test]
    fn redacts_credentials_in_urls() {
        let cases = [
            ("https://gitlab.corp/api/v4", "https://gitlab.corp/api/v4"),
            (
                "https://oauth2:glpat-x@gitlab.corp:8443/api/v4",
                "https://<redacted>@gitlab.corp:8443/api/v4",
            ),
            ("https://tok@host", "https://<redacted>@host"),
            ("https://a@b:c@host/p@q", "https://<redacted>@host/p@q"),
            ("http://host/path@x", "http://host/path@x"),
            ("user:pw@host/x", "<redacted>@host/x"),
        ];
        for (url, want) in cases {
            assert_eq!(redact_userinfo(url), want, "{url}");
        }
    }

    #[test]
    fn loopback_detection() {
        for url in [
            "http://127.0.0.1",
            "http://127.0.0.1/",
            "http://127.0.0.1:4000",
            "http://localhost/api/v4",
            "http://localhost:8080/api/v4/",
            "http://[::1]",
            "http://[::1]:80",
            "http://[::1]:8787/api",
            "http://127.0.0.1:65535/x",
        ] {
            assert!(is_loopback_base(url), "{url} should be loopback");
        }
        for url in [
            // Credentials: the real host is after the `@`.
            "http://localhost:@evil.example/",
            "http://localhost:@evil",
            "http://localhost@evil.example",
            "http://user:pw@localhost/",
            "http://127.0.0.1:80@evil.example/api",
            "http://localhost/@evil.example",
            "http://[::1]@evil.example/",
            // Look-alike hosts.
            "http://127.0.0.1.evil.com",
            "http://127.0.0.1.evil.com/api",
            "http://localhost.evil.com:80",
            "http://localhost.",
            "http://LOCALHOST/",
            "http://evil.example/localhost",
            "http://127.0.0.2/",
            "http://0.0.0.0/",
            "http://[::1].evil.com/",
            "http://[::2]/",
            "http://[0:0:0:0:0:0:0:1]/",
            // Not plain http, or malformed.
            "https://127.0.0.1",
            "https://localhost:8080/api",
            "HTTP://localhost/",
            "ftp://localhost/",
            "localhost:8080",
            " http://localhost/",
            "http://",
            "http://:8080/",
            "http://localhost:/",
            "http://localhost:99999/",
            "http://localhost:80:80/",
            "http://localhost:8o/",
            "http://localhost?x=1",
            "http://localhost#frag",
            "http://localhost\\@evil.example/",
            "http://localhost%40evil.example/",
            "https://api.github.com",
        ] {
            assert!(!is_loopback_base(url), "{url} must not be loopback");
        }
    }

    #[test]
    fn server_messages() {
        assert_eq!(
            server_message(br#"{"message":"Bad credentials"}"#).as_deref(),
            Some("Bad credentials")
        );
        assert_eq!(
            server_message(
                br#"{"message":"Validation Failed","errors":[{"resource":"PullRequest","code":"custom","message":"A pull request already exists"}]}"#
            )
            .as_deref(),
            Some("Validation Failed; A pull request already exists")
        );
        assert_eq!(
            server_message(br#"{"message":"Validation Failed","errors":[{"resource":"PullRequest","field":"head","code":"invalid"}]}"#)
                .as_deref(),
            Some("Validation Failed; head invalid")
        );
        assert_eq!(
            server_message(br#"{"message":{"source_branch":["is invalid"]}}"#).as_deref(),
            Some("source_branch is invalid")
        );
        assert_eq!(
            server_message(br#"{"message":["Another open merge request already exists"]}"#)
                .as_deref(),
            Some("Another open merge request already exists")
        );
        assert_eq!(
            server_message(br#"{"error":"insufficient_scope","error_description":"The request requires higher privileges"}"#)
                .as_deref(),
            Some("insufficient_scope; The request requires higher privileges")
        );
        assert_eq!(server_message(b"<html>bad gateway</html>"), None);
        assert_eq!(server_message(b"plain text").as_deref(), Some("plain text"));
        assert_eq!(server_message(b"{}"), None);
    }

    #[test]
    fn status_messages_redact_normalized_client_tokens() {
        let headers = HeaderMap::new();
        let token = "arbitrarySecretToken1234567";
        for body in [
            br#"{"message":"arbitrarySecretToken1234567"}"#.as_slice(),
            br#"{"message":"arbitrarySecret\u0054oken1234567"}"#.as_slice(),
            b"{\"message\":\"arbitrarySecret\xe2\x80\x8bToken1234567\"}".as_slice(),
        ] {
            let error = map_status_with_token(
                422,
                &headers,
                body,
                ForgeKind::GitLab,
                Some(token),
                &str::to_owned,
            );
            let message = error.to_string();
            assert!(message.contains("<redacted>"), "{message}");
            assert!(!message.contains(token), "{message}");
            assert!(!message.contains("Secret"), "{message}");
        }
    }

    #[test]
    fn status_mapping() {
        let mut h = HeaderMap::new();
        assert_eq!(
            map_status(
                401,
                &h,
                br#"{"message":"Bad credentials"}"#,
                ForgeKind::GitHub
            ),
            Error::Unauthorized("Bad credentials".into())
        );
        assert_eq!(
            map_status(403, &h, b"", ForgeKind::GitLab),
            Error::Unauthorized("Forbidden".into())
        );
        h.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        let soon = jiff::Timestamp::now().as_second() + 600;
        h.insert("x-ratelimit-reset", HeaderValue::from(soon));
        assert_eq!(
            map_status(403, &h, b"", ForgeKind::GitHub),
            Error::RateLimited { reset: Some(soon) }
        );
        let mut gl = HeaderMap::new();
        gl.insert("ratelimit-reset", HeaderValue::from(soon + 1));
        assert_eq!(
            map_status(429, &gl, b"Retry later", ForgeKind::GitLab),
            Error::RateLimited {
                reset: Some(soon + 1)
            }
        );
        assert_eq!(
            map_status(404, &HeaderMap::new(), b"", ForgeKind::GitLab),
            Error::NotFound("Not Found".into())
        );
        assert_eq!(
            map_status(
                405,
                &HeaderMap::new(),
                br#"{"message":"Pull Request is not mergeable"}"#,
                ForgeKind::GitHub
            ),
            Error::Api {
                status: 405,
                message: "Pull Request is not mergeable".into()
            }
        );
    }
}
