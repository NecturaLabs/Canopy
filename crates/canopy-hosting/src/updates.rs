//! Canopy's own update feed: a small JSON manifest in the public `NecturaLabs/app-updates`
//! repository, fetched anonymously over HTTPS. Nothing about the user is sent (no identifiers,
//! no query string); the request carries only `User-Agent: Canopy/<version>`.
//!
//! Manifest format (`canopy/latest.json` for stable releases, `canopy/beta.json` for
//! pre-releases):
//!
//! ```json
//! {
//!   "version": "0.2.0",
//!   "notes": "Markdown release notes",
//!   "pub_date": "2026-10-01T00:00:00Z",
//!   "release_page": "https://github.com/NecturaLabs/app-updates/releases/tag/canopy-v0.2.0",
//!   "platforms": {
//!     "linux-x86_64": { "url": "https://…/canopy-0.2.0-x86_64-unknown-linux-gnu.tar.gz",
//!                       "sha256": "…" }
//!   }
//! }
//! ```

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use ureq::tls::{RootCerts, TlsConfig};

use crate::error::{Error, Result};

/// One downloadable build.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Artifact {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// A published version.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: Option<String>,
    #[serde(default)]
    pub release_page: Option<String>,
    #[serde(default)]
    pub platforms: BTreeMap<String, Artifact>,
}

impl UpdateManifest {
    /// Parses a manifest.
    pub fn parse(json: &[u8]) -> Result<Self> {
        let m: UpdateManifest = serde_json::from_slice(json)
            .map_err(|e| Error::Parse(format!("update manifest: {e}")))?;
        if canopy_core::update::Version::parse(&m.version).is_none() {
            return Err(Error::Parse(format!(
                "update manifest: \"{}\" is not a version",
                m.version
            )));
        }
        Ok(m)
    }

    /// The build for the platform Canopy runs on.
    pub fn artifact(&self) -> Option<&Artifact> {
        self.platforms.get(&platform_key())
    }
}

#[cfg(target_os = "linux")]
const OS: &str = "linux";
#[cfg(target_os = "windows")]
const OS: &str = "windows";
#[cfg(target_os = "macos")]
const OS: &str = "darwin";
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
const OS: &str = std::env::consts::OS;

/// This build's key in a manifest's `platforms`: `linux-x86_64`, `windows-x86_64`,
/// `darwin-aarch64`, …
pub fn platform_key() -> String {
    format!("{OS}-{}", std::env::consts::ARCH)
}

/// Fetches and parses the manifest at `url`.
pub fn fetch_manifest(url: &str) -> Result<UpdateManifest> {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(format!("Canopy/{}", env!("CARGO_PKG_VERSION")))
        .tls_config(tls);
    if url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost") {
        config = config.proxy(None);
    }
    let agent = ureq::Agent::new_with_config(config.build());
    let mut response = agent
        .get(url)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| Error::Network(e.to_string()))?;
    let status = response.status().as_u16();
    let bytes = response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_vec()
        .map_err(|e| Error::Network(e.to_string()))?;
    match status {
        200..=299 => UpdateManifest::parse(&bytes),
        404 => Err(Error::NotFound(format!("no update manifest at {url}"))),
        _ => Err(Error::Api {
            status,
            message: String::from_utf8_lossy(&bytes).chars().take(200).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const SAMPLE: &str = r#"{
        "version": "0.2.0",
        "notes": "New themes",
        "pub_date": "2026-10-01T00:00:00Z",
        "release_page": "https://example.invalid/r",
        "platforms": {
            "linux-x86_64": { "url": "https://example.invalid/l.tar.gz", "sha256": "ab" },
            "windows-x86_64": { "url": "https://example.invalid/w.zip" }
        }
    }"#;

    #[test]
    fn parses_manifest() {
        let m = UpdateManifest::parse(SAMPLE.as_bytes()).expect("parse");
        assert_eq!(m.version, "0.2.0");
        assert_eq!(m.platforms.len(), 2);
        assert_eq!(m.platforms["windows-x86_64"].sha256, None);
        assert!(UpdateManifest::parse(br#"{"version":"soon"}"#).is_err());
        assert!(UpdateManifest::parse(b"not json").is_err());
        let minimal = UpdateManifest::parse(br#"{"version":"1.0.0"}"#).expect("minimal");
        assert!(minimal.platforms.is_empty() && minimal.notes.is_empty());
    }

    #[test]
    fn platform_key_has_os_and_arch() {
        let key = platform_key();
        assert!(key.contains('-'), "{key}");
        assert!(key.ends_with(std::env::consts::ARCH), "{key}");
    }

    /// Serves one canned HTTP response on a loopback port and returns its URL.
    fn serve_once(status: &'static str, body: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}/canopy/latest.json")
    }

    #[test]
    fn fetches_over_http() {
        let url = serve_once("200 OK", SAMPLE);
        let m = fetch_manifest(&url).expect("fetch");
        assert_eq!(m.version, "0.2.0");
        let missing = serve_once("404 Not Found", "404: Not Found");
        assert!(matches!(fetch_manifest(&missing), Err(Error::NotFound(_))));
    }
}
