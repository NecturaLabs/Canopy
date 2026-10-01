//! The HTTP layer: one `ureq` agent per client, the common headers, and mapping HTTP failures
//! to [`Error`] with the server's own message.

use std::fmt;
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
}

impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Http")
            .field("kind", &self.kind)
            .field("base", &self.base)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
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
            .user_agent(format!("Canopy/{}", env!("CARGO_PKG_VERSION")))
            // GitHub redirects renamed repositories; keep the token on the same host only.
            .redirect_auth_headers(RedirectAuthHeaders::SameHost)
            .tls_config(tls);
        if is_loopback(&base) {
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
        }
    }

    pub(crate) fn has_token(&self) -> bool {
        self.token.is_some()
    }

    pub(crate) fn base(&self) -> &str {
        &self.base
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
    /// of a 2xx response, or the mapped error.
    fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Vec<u8>> {
        let url = format!("{}{}", self.base, path);
        let payload = match body {
            Some(v) => Some(serde_json::to_vec(v).map_err(|e| Error::Parse(e.to_string()))?),
            None => None,
        };
        let result = match method {
            Method::Get => self.headers(self.agent.get(&url)).call(),
            Method::Post => self.send_with_body(self.agent.post(&url), payload.as_deref()),
            Method::Put => self.send_with_body(self.agent.put(&url), payload.as_deref()),
            Method::Patch => self.send_with_body(self.agent.patch(&url), payload.as_deref()),
        };
        let mut response = result.map_err(|e| {
            tracing::warn!(method = method.as_str(), path, error = %e, "hosting request failed");
            Error::Network(e.to_string())
        })?;
        let status = response.status().as_u16();
        tracing::debug!(method = method.as_str(), path, status, "hosting request");
        let bytes = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| Error::Network(e.to_string()))?;
        if (200..300).contains(&status) {
            Ok(bytes)
        } else {
            let err = map_status(status, response.headers(), &bytes, self.kind);
            tracing::warn!(method = method.as_str(), path, status, error = %err, "hosting request rejected");
            Err(err)
        }
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

fn is_loopback(base: &str) -> bool {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let host = rest.split('/').next().unwrap_or_default();
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub(crate) fn parse<T: DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| Error::Parse(format!("{what}: {e}")))
}

fn header_i64(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.trim().parse::<i64>().ok()
}

/// When a rate limit resets: the provider's reset header (unix seconds), else `Retry-After`
/// (seconds from now).
fn rate_limit_reset(headers: &HeaderMap) -> Option<i64> {
    header_i64(headers, "x-ratelimit-reset")
        .or_else(|| header_i64(headers, "ratelimit-reset"))
        .or_else(|| {
            header_i64(headers, "retry-after").map(|secs| chrono::Utc::now().timestamp() + secs)
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
        if text.is_empty() || text.starts_with('<') {
            return None;
        }
        return Some(shorten(text));
    };
    let mut parts: Vec<String> = Vec::new();
    for key in ["message", "error", "error_description", "errors"] {
        if let Some(v) = json.get(key) {
            flatten(v, None, &mut parts);
        }
    }
    parts.dedup();
    if parts.is_empty() {
        None
    } else {
        Some(shorten(&parts.join("; ")))
    }
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

fn shorten(s: &str) -> String {
    if s.chars().count() <= MAX_MESSAGE {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(MAX_MESSAGE).collect();
        out.push('…');
        out
    }
}

/// Percent-encodes everything outside RFC 3986's unreserved set, so `group/sub/project`
/// becomes `group%2Fsub%2Fproject`.
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::http::HeaderValue;

    #[test]
    fn encodes_gitlab_project_ids() {
        assert_eq!(encode("group/sub/project"), "group%2Fsub%2Fproject");
        assert_eq!(encode("a.b_c-d~e"), "a.b_c-d~e");
        assert_eq!(encode("x y+z"), "x%20y%2Bz");
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("http://127.0.0.1:4000"));
        assert!(is_loopback("http://localhost/api/v4"));
        assert!(is_loopback("http://[::1]:80"));
        assert!(!is_loopback("https://api.github.com"));
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
