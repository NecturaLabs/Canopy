//! Errors from the hosting API, carrying the server's own message where it sent one.

use chrono::{DateTime, Utc};

/// A hosting API failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The server could not be reached: DNS, connection, TLS, timeout or a broken response
    /// stream.
    #[error("Could not reach the server: {0}")]
    Network(String),
    /// HTTP 401, or a 403 that is not a rate limit: the CLI's sign-in is missing, invalid,
    /// expired or lacks the scope for this request.
    #[error(
        "The server refused the request (check the sign-in of the GitHub or GitLab CLI and its scopes): {0}"
    )]
    Unauthorized(String),
    /// HTTP 429, or GitHub's 403 with an exhausted quota. `reset` is when the quota refills, in
    /// unix seconds, when the server said.
    #[error("{}", rate_limited(*reset))]
    RateLimited {
        /// When the limit resets, in unix seconds, at most a day ahead of when the answer came.
        reset: Option<i64>,
    },
    /// HTTP 404. GitHub and GitLab answer 404 rather than 403 for private projects the signed-in
    /// account cannot see, or that nobody is signed in to see.
    #[error("{}", not_found(.0))]
    NotFound(String),
    /// Any other unsuccessful status, with the message from the response's `message`, `error`
    /// or `errors` fields when present.
    #[error("The server rejected the request (HTTP {status}): {message}")]
    Api {
        /// The HTTP status code.
        status: u16,
        /// The server's explanation.
        message: String,
    },
    /// The response was not the JSON Canopy expected.
    #[error("Unexpected response from the server: {0}")]
    Parse(String),
    /// Not sent: the client's switch ([`crate::Client::with_switch`]) was off.
    #[error("Not sent: pull and merge requests were turned off or reconnected")]
    Stopped,
}

/// Result alias for hosting API calls.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The error for a merge refused because the request's head is no longer the commit the user
/// reviewed (GitHub and GitLab both answer 409 when the `sha` sent with a merge does not match).
pub(crate) fn head_changed(noun: &str) -> Error {
    Error::Api {
        status: crate::http::status::CONFLICT,
        message: format!(
            "The {noun} changed since you loaded it. Refresh and review before merging."
        ),
    }
}

/// Replaces a 409 from a merge that carried an expected head SHA with [`head_changed`].
pub(crate) fn map_head_conflict<T>(result: Result<T>, sha_sent: bool, noun: &str) -> Result<T> {
    match result {
        Err(Error::Api {
            status: crate::http::status::CONFLICT,
            message,
        }) if sha_sent => {
            tracing::info!(server = %message, "merge refused: the request head moved");
            Err(head_changed(noun))
        }
        other => other,
    }
}

/// The sentence for a 404, with the server's words only when they add something: GitHub's are
/// just "Not Found".
fn not_found(message: &str) -> String {
    const SAID: &str = "Not found, or the repository is private";
    let message = message.trim();
    if message.is_empty() || message.eq_ignore_ascii_case("not found") {
        SAID.to_owned()
    } else {
        format!("{SAID} ({message})")
    }
}

fn rate_limited(reset: Option<i64>) -> String {
    match reset.and_then(|t| DateTime::<Utc>::from_timestamp(t, 0)) {
        Some(at) => format!(
            "API rate limit reached; it resets at {} UTC",
            at.format("%Y-%m-%d %H:%M")
        ),
        None => "API rate limit reached; try again later".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_server_message() {
        let e = Error::Api {
            status: 422,
            message: "Validation Failed: A pull request already exists".into(),
        };
        assert_eq!(
            e.to_string(),
            "The server rejected the request (HTTP 422): Validation Failed: A pull request already exists"
        );
        assert_eq!(
            Error::NotFound("Not Found".into()).to_string(),
            "Not found, or the repository is private",
            "the server's \"Not Found\" adds nothing"
        );
        assert_eq!(
            Error::NotFound("404 Project Not Found".into()).to_string(),
            "Not found, or the repository is private (404 Project Not Found)"
        );
        assert_eq!(
            Error::RateLimited {
                reset: Some(1_790_000_000)
            }
            .to_string(),
            "API rate limit reached; it resets at 2026-09-21 14:13 UTC"
        );
        assert!(
            Error::RateLimited { reset: None }
                .to_string()
                .contains("try again later")
        );
    }
}
