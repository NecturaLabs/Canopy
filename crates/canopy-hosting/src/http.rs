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

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest server message kept from a non-JSON error body.
const MAX_MESSAGE: usize = 300;
/// Largest API response body read (ureq's own default for a whole-body read, stated here so
/// every request, the GraphQL query included, visibly shares it).
const MAX_BODY: u64 = 10 * 1024 * 1024;

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
        let agent = Agent::new_with_config(config.build());
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

    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    /// Profile lookup: no redirects, bounded JSON, and no author email in logs. Sends nothing
    /// while the switch is off.
    #[expect(
        clippy::map_err_ignore,
        reason = "kept as it was; docs/backlog.md has the entry to carry the transport's message"
    )]
    pub(crate) fn avatar_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        if self.stopped() {
            return Err(Error::Stopped);
        }
        let mut response = self
            .headers(self.agent.get(format!("{}{}", self.base, path)))
            .config()
            .max_redirects(0)
            .build()
            .call()
            .map_err(|_| {
                Error::Network("Profile lookup could not reach the hosting service".into())
            })?;
        let status = response.status().as_u16();
        let bytes = response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_vec()
            .map_err(|_| Error::Network("Could not read the profile response".into()))?;
        if !(200..300).contains(&status) {
            return Err(map_status(status, response.headers(), &bytes, self.kind));
        }
        parse(&bytes, "profile response")
    }

    /// Sends a request and parses a successful JSON response.
    pub(crate) fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let bytes = self.send(method, path, body)?;
        parse(&bytes, path)
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
            let error = shorten(&e.to_string());
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
            .map(str::to_owned);
        tracing::debug!(method = method.as_str(), path, status, ms, rate_limit_remaining = ?remaining, "hosting request");
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|e| Error::Network(shorten(&e.to_string())))?;
        if (200..300).contains(&status) {
            Ok((response.headers().clone(), bytes))
        } else {
            let err = map_status(status, response.headers(), &bytes, self.kind);
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
    /// successful response, errors that come with no requests left, a `retry-after`, or a
    /// message or type saying a rate limit was hit become [`Error::RateLimited`]; other errors
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
        let envelope: GraphqlEnvelope<T> = parse(&bytes, "GraphQL response")?;
        // GitHub answers an exhausted GraphQL quota with 200, an error and no requests left, and
        // a secondary limit with 200 or 403, an error saying so and perhaps `retry-after`.
        let limited = !envelope.errors.is_empty()
            && (header_i64(&headers, "x-ratelimit-remaining") == Some(0)
                || headers.contains_key("retry-after")
                || envelope.errors.iter().any(|e| {
                    e.kind.as_deref() == Some("RATE_LIMITED")
                        || e.message.to_ascii_lowercase().contains("rate limit")
                }));
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
            let message = shorten(&messages.join("; "));
            Error::Api {
                status: 200,
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
                .header("X-GitHub-Api-Version", "2022-11-28"),
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

/// Parses a JSON response. The parser's message can quote the server's own text (a string where
/// a number belongs), so it is cleaned like any server message.
pub(crate) fn parse<T: DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| Error::Parse(shorten(&format!("{what}: {e}"))))
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

/// The longest `Retry-After` taken at its word: a day.
const MAX_RETRY_AFTER: i64 = 86_400;

/// When a rate limit resets: the provider's reset header (unix seconds), else `Retry-After`
/// (seconds from now, between none and [`MAX_RETRY_AFTER`]).
fn rate_limit_reset(headers: &HeaderMap) -> Option<i64> {
    header_i64(headers, "x-ratelimit-reset")
        .or_else(|| header_i64(headers, "ratelimit-reset"))
        .or_else(|| {
            header_i64(headers, "retry-after").map(|secs| {
                chrono::Utc::now()
                    .timestamp()
                    .saturating_add(secs.clamp(0, MAX_RETRY_AFTER))
            })
        })
}

/// Maps an unsuccessful response to an [`Error`].
pub(crate) fn map_status(status: u16, headers: &HeaderMap, body: &[u8], kind: ForgeKind) -> Error {
    let message = server_message(body).unwrap_or_else(|| default_reason(status).to_owned());
    match status {
        401 => Error::Unauthorized(message),
        403 => {
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
        404 => Error::NotFound(message),
        429 => Error::RateLimited {
            reset: rate_limit_reset(headers),
        },
        _ => Error::Api { status, message },
    }
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
/// A non-JSON body is used as is, shortened.
pub(crate) fn server_message(body: &[u8]) -> Option<String> {
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        let text = String::from_utf8_lossy(body);
        let text = text.trim();
        if text.starts_with('<') {
            return None;
        }
        return Some(shorten(text)).filter(|t| !t.is_empty());
    };
    let mut parts: Vec<String> = Vec::new();
    for key in ["message", "error", "error_description", "errors"] {
        if let Some(v) = json.get(key) {
            flatten(v, None, &mut parts);
        }
    }
    parts.dedup();
    Some(shorten(&parts.join("; "))).filter(|t| !t.is_empty())
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

/// A server's own words made safe to show among Canopy's and to log.
///
/// Control characters become spaces, characters that reorder or hide text are dropped
/// (bidirectional controls and every other default-ignorable code point, such as zero-width
/// spaces, variation selectors and tags), whitespace collapses to single spaces, and the result
/// is at most `max` characters, ending in an ellipsis when it was cut.
pub fn plain_text(text: &str, max: usize) -> String {
    let visible: String = text
        .chars()
        .filter(|&c| !hides_text(c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().count() <= max {
        return words;
    }
    let mut short: String = words.chars().take(max.saturating_sub(1)).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    short
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
    }

    #[test]
    fn huge_retry_after_values_neither_overflow_nor_wait_for_ever() {
        let now = chrono::Utc::now().timestamp();
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
        h.insert("x-ratelimit-reset", HeaderValue::from_static("1800000000"));
        assert_eq!(
            map_status(403, &h, b"", ForgeKind::GitHub),
            Error::RateLimited {
                reset: Some(1_800_000_000)
            }
        );
        let mut gl = HeaderMap::new();
        gl.insert("ratelimit-reset", HeaderValue::from_static("1800000001"));
        assert_eq!(
            map_status(429, &gl, b"Retry later", ForgeKind::GitLab),
            Error::RateLimited {
                reset: Some(1_800_000_001)
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
