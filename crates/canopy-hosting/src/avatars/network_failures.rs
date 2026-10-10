//! Profile lookups and picture downloads against real local sockets that fail the way networks
//! do: nothing listening, a hang-up, an error status, a stall past the timeout and a redirect.
//! Each failure must name its cause and none may repeat the token, the author's email, the
//! private repository or the picture's address.

#![cfg(test)]

use canopy_core::forge::{Forge, ForgeKind};

use super::{Avatar, TOTAL_TIMEOUT};
use crate::test_server::{Reply, Scripted, refused};
use crate::{Client, Error};

const TOKEN: &str = "tok-SECRET-1234567890";
const EMAIL: &str = "private.author@example.com";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const PICTURE_PATH: &str = "/pictures/private-user/portrait.png?size=64&secret=sig-9f8e7d";

/// Pieces of private data no error text may contain.
const PRIVATE: [&str; 10] = [
    TOKEN,
    "SECRET",
    EMAIL,
    "private.author",
    "private.author%40example.com",
    "acme-secret",
    "private-repo",
    "private-user",
    "portrait.png",
    "sig-9f8e7d",
];

fn client(kind: ForgeKind, base: &str) -> Client {
    let (host, path, api) = match kind {
        ForgeKind::GitHub => ("github.com", "acme-secret/private-repo", base.to_owned()),
        ForgeKind::GitLab => (
            "gitlab.example.com",
            "acme-secret/private-repo",
            format!("{base}/api/v4"),
        ),
    };
    let forge = Forge {
        kind,
        host: host.into(),
        port: None,
        path: path.into(),
    };
    Client::with_api_base(&forge, Some(TOKEN.to_owned()), &api)
}

fn assert_private_data_hidden(message: &str, full_urls: &[&str]) {
    for secret in PRIVATE.into_iter().chain(full_urls.iter().copied()) {
        assert!(!message.contains(secret), "{secret} in: {message}");
    }
}

fn says(message: &str, causes: &[&str]) -> bool {
    let lower = message.to_ascii_lowercase();
    causes.iter().any(|c| lower.contains(c))
}

fn both_providers() -> [ForgeKind; 2] {
    [ForgeKind::GitHub, ForgeKind::GitLab]
}

fn lookup(kind: ForgeKind, base: &str) -> Error {
    client(kind, base)
        .author_avatar(SHA, EMAIL)
        .expect_err("the lookup fails")
}

#[test]
fn a_profile_lookup_on_a_closed_port_says_it_was_refused() {
    let closed = refused();
    let base = format!("http://{}", closed.address);
    for kind in both_providers() {
        let error = lookup(kind, &base);
        let message = error.to_string();
        assert!(matches!(error, Error::Network(_)), "{message}");
        assert!(says(&message, &["refused"]), "{message}");
        assert_private_data_hidden(&message, &[&base]);
    }
}

#[test]
fn a_profile_lookup_hung_up_on_says_the_connection_closed() {
    let server = Scripted::start(|_| Reply::HangUp);
    for kind in both_providers() {
        let error = lookup(kind, &server.base);
        let message = error.to_string();
        assert!(matches!(error, Error::Network(_)), "{message}");
        assert!(
            says(
                &message,
                &[
                    "closed",
                    "reset",
                    "end of file",
                    "eof",
                    "broken",
                    "disconnected"
                ]
            ),
            "{message}"
        );
        assert_private_data_hidden(&message, &[]);
    }
}

#[test]
fn a_profile_lookup_answered_with_500_says_the_status_and_hides_what_the_server_echoes() {
    // GitHub's lookup sends no email; GitLab's does, and its server may repeat it decoded.
    let server = Scripted::start(|target| {
        let author = if target.contains("/avatar") {
            EMAIL
        } else {
            "nobody"
        };
        Reply::Status(
            500,
            "Content-Type: application/json\r\n".into(),
            serde_json::json!({
                "message": format!("boom at {target} for {author} using {TOKEN}")
            })
            .to_string(),
        )
    });
    for kind in both_providers() {
        let error = lookup(kind, &server.base);
        let message = error.to_string();
        assert!(matches!(error, Error::Api { status: 500, .. }), "{message}");
        assert!(
            message.contains("500") && message.contains("boom"),
            "{message}"
        );
        assert_private_data_hidden(&message, &[&server.base]);
    }
}

#[test]
fn a_profile_lookup_that_stalls_says_it_timed_out() {
    let server = Scripted::start(|_| Reply::Stall);
    let started = std::time::Instant::now();
    let error = lookup(ForgeKind::GitHub, &server.base);
    let message = error.to_string();
    assert!(matches!(error, Error::Network(_)), "{message}");
    assert!(says(&message, &["timeout", "timed out"]), "{message}");
    assert!(started.elapsed() < std::time::Duration::from_secs(40));
    assert_private_data_hidden(&message, &[]);
}

#[test]
fn a_profile_lookup_redirect_is_refused_not_followed() {
    let elsewhere = Scripted::start(|_| Reply::Status(200, String::new(), "{}".into()));
    let location = format!("{}/private-repo/stolen?email={EMAIL}", elsewhere.base);
    let server = Scripted::start(move |_| {
        Reply::Status(302, format!("Location: {location}\r\n"), String::new())
    });
    for kind in both_providers() {
        let error = lookup(kind, &server.base);
        let message = error.to_string();
        assert!(says(&message, &["302"]), "{message}");
        assert_private_data_hidden(&message, &[&server.base, &elsewhere.base]);
    }
    assert!(elsewhere.seen().is_empty(), "{:?}", elsewhere.seen());
}

fn picture(base: &str) -> (Avatar, String) {
    let url = format!("{base}{PICTURE_PATH}");
    (
        Avatar {
            url: url.clone(),
            loopback: true,
        },
        url,
    )
}

#[test]
fn a_picture_on_a_closed_port_says_it_was_refused() {
    let closed = refused();
    let (avatar, url) = picture(&format!("http://{}", closed.address));
    let message = avatar.download().expect_err("refused").to_string();
    assert!(says(&message, &["refused"]), "{message}");
    assert_private_data_hidden(&message, &[&url]);
}

#[test]
fn a_picture_hung_up_on_says_the_connection_closed() {
    let server = Scripted::start(|_| Reply::HangUp);
    let (avatar, url) = picture(&server.base);
    let message = avatar.download().expect_err("hung up").to_string();
    assert!(
        says(
            &message,
            &[
                "closed",
                "reset",
                "end of file",
                "eof",
                "broken",
                "disconnected"
            ]
        ),
        "{message}"
    );
    assert_private_data_hidden(&message, &[&url]);
}

#[test]
fn a_picture_answered_with_500_says_the_status() {
    let server = Scripted::start(|target| {
        Reply::Status(
            500,
            String::new(),
            format!("boom at {target} for {EMAIL} using {TOKEN}"),
        )
    });
    let (avatar, url) = picture(&server.base);
    let error = avatar.download().expect_err("500");
    let message = error.to_string();
    assert!(matches!(error, Error::Api { status: 500, .. }), "{message}");
    assert!(message.contains("500"), "{message}");
    assert_private_data_hidden(&message, &[&url]);
}

#[test]
fn a_picture_that_stalls_says_it_timed_out() {
    let server = Scripted::start(|_| Reply::Stall);
    let (avatar, url) = picture(&server.base);
    let started = std::time::Instant::now();
    let message = avatar.download().expect_err("timeout").to_string();
    assert!(says(&message, &["timeout", "timed out"]), "{message}");
    assert!(started.elapsed() < TOTAL_TIMEOUT + std::time::Duration::from_secs(10));
    assert_private_data_hidden(&message, &[&url]);
}

#[test]
fn a_picture_redirect_is_refused_not_followed() {
    let elsewhere = Scripted::start(|_| Reply::Status(200, String::new(), "stolen".into()));
    let location = format!("{}{PICTURE_PATH}", elsewhere.base);
    let server = Scripted::start(move |_| {
        Reply::Status(302, format!("Location: {location}\r\n"), String::new())
    });
    let (avatar, url) = picture(&server.base);
    let message = avatar.download().expect_err("redirect").to_string();
    assert!(says(&message, &["302"]), "{message}");
    assert_private_data_hidden(&message, &[&url, &elsewhere.base]);
    assert!(elsewhere.seen().is_empty(), "{:?}", elsewhere.seen());
}
