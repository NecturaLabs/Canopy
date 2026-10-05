//! Canopy's own update feed: a small JSON manifest in the public `NecturaLabs/app-updates`
//! repository, fetched anonymously over HTTPS. Nothing about the user is sent (no identifiers,
//! no query string); the request carries only `User-Agent: Canopy/<version>`.
//!
//! Manifest format (`canopy/latest.json` for stable builds, `canopy/beta.json` for beta builds).
//! `version` is the build number, `git rev-list --count` of the build's commit on `main`:
//!
//! ```json
//! {
//!   "version": "412",
//!   "notes": "Markdown release notes",
//!   "pub_date": "2026-10-01T00:00:00Z",
//!   "release_page": "https://github.com/NecturaLabs/app-updates/releases/tag/canopy-build-412",
//!   "platforms": {
//!     "linux-x86_64": { "url": "https://…/canopy-build-412-x86_64-unknown-linux-gnu.tar.gz",
//!                       "sha256": "…" }
//!   }
//! }
//! ```

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig};

use crate::error::{Error, Result};

/// One downloadable build.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Artifact {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// A published build.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UpdateManifest {
    /// The build number as published. Anything that is not one (the semantic version of a feed
    /// from before builds were numbered) is older than every build; see [`UpdateManifest::build`].
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
        serde_json::from_slice(json).map_err(|e| Error::Parse(format!("update manifest: {e}")))
    }

    /// The build number, when `version` is one.
    pub fn build(&self) -> Option<u64> {
        canopy_core::update::parse_build(&self.version)
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
        .user_agent(crate::http::user_agent())
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

/// Where every build's release files are downloadable: `<this>/canopy-build-<n>/<file>`.
pub const RELEASE_DOWNLOADS: &str = "https://github.com/NecturaLabs/app-updates/releases/download";

/// Redirects followed, each of them to HTTPS.
const MAX_REDIRECTS: usize = 5;

/// What a download wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    pub bytes: u64,
    /// Lower-case hex SHA-256 of the bytes written.
    pub sha256: String,
}

fn is_loopback_url(url: &str) -> bool {
    url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A redirect's target: an absolute URL as it is, an absolute path on the same host. Anything
/// else (a relative path, a protocol-relative `//host`) is refused rather than guessed.
#[expect(
    clippy::string_slice,
    reason = "offsets come from find, match_indices and ASCII lengths, so they fall between characters"
)]
fn resolve_location(from: &str, location: &str) -> Result<String> {
    if location.contains("://") {
        return Ok(location.to_owned());
    }
    if location.starts_with('/') && !location.starts_with("//") {
        let after_scheme = from.find("://").map_or(0, |i| i + 3);
        let origin_end = from[after_scheme..]
            .find('/')
            .map_or(from.len(), |i| after_scheme + i);
        return Ok(format!("{}{location}", &from[..origin_end]));
    }
    Err(Error::Network(
        "a redirect to an address that cannot be followed".to_owned(),
    ))
}

/// Streams `url` into `sink`, at most `max_bytes`, calling `progress(done, total)` as it goes.
/// Redirects are followed by hand, at most five, and only to HTTPS (plain HTTP is accepted
/// nowhere but on a loopback address, for tests): a hop that is not HTTPS ends the download.
/// Returns the byte count and the SHA-256 of what `sink` received.
fn get(
    url: &str,
    max_bytes: u64,
    sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Downloaded> {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        // Redirects are checked here, hop by hop, instead of by ureq.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_mins(30)))
        .user_agent(crate::http::user_agent())
        .tls_config(tls);
    if is_loopback_url(url) {
        config = config.proxy(None);
    }
    let agent = ureq::Agent::new_with_config(config.build());
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        if !current.starts_with("https://") && !is_loopback_url(&current) {
            return Err(Error::Network(format!(
                "refusing a download that is not over HTTPS: {}",
                canopy_core::cli::redact_url(&current)
            )));
        }
        let mut response = agent
            .get(&current)
            .call()
            .map_err(|e| Error::Network(e.to_string()))?;
        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
                .ok_or_else(|| Error::Network("a redirect without a location".to_owned()))?;
            current = resolve_location(&current, &location)?;
            continue;
        }
        match status {
            200..=299 => {}
            404 => return Err(Error::NotFound(format!("nothing at {url}"))),
            _ => {
                let body = response
                    .body_mut()
                    .with_config()
                    .limit(4096)
                    .read_to_vec()
                    .unwrap_or_default();
                return Err(Error::Api {
                    status,
                    message: String::from_utf8_lossy(&body).chars().take(200).collect(),
                });
            }
        }
        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if total.is_some_and(|t| t > max_bytes) {
            return Err(Error::Network(format!(
                "the download is larger than the {max_bytes} bytes allowed"
            )));
        }
        let mut reader = response.body_mut().as_reader();
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| Error::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > max_bytes {
                return Err(Error::Network(format!(
                    "the download is larger than the {max_bytes} bytes allowed"
                )));
            }
            hasher.update(&buf[..n]);
            sink(&buf[..n])
                .map_err(|e| Error::Network(format!("could not save the download: {e}")))?;
            progress(done, total);
        }
        if total.is_some_and(|t| t != done) {
            return Err(Error::Network(
                "the download ended before its announced size".to_owned(),
            ));
        }
        return Ok(Downloaded {
            bytes: done,
            sha256: hex(&hasher.finalize()),
        });
    }
    Err(Error::Network(format!(
        "more than {MAX_REDIRECTS} redirects from {}",
        canopy_core::cli::redact_url(url)
    )))
}

/// Downloads `url` into a new file at `dest` (which must not exist) and returns its SHA-256. The
/// file is removed again when the download fails.
pub fn download(
    url: &str,
    dest: &Path,
    max_bytes: u64,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Downloaded> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|e| Error::Network(format!("could not create {}: {e}", dest.display())))?;
    let result = get(url, max_bytes, &mut |chunk| file.write_all(chunk), progress).and_then(|d| {
        file.flush()
            .map_err(|e| Error::Network(format!("could not save the download: {e}")))?;
        Ok(d)
    });
    if result.is_err() {
        drop(file);
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// Downloads a small file (at most `max_bytes`) into memory.
pub fn download_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    get(
        url,
        max_bytes,
        &mut |chunk| {
            out.extend_from_slice(chunk);
            Ok(())
        },
        &mut |_, _| {},
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const SAMPLE: &str = r#"{
        "version": "412",
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
        assert_eq!(m.version, "412");
        assert_eq!(m.build(), Some(412));
        assert_eq!(m.platforms.len(), 2);
        assert_eq!(m.platforms["windows-x86_64"].sha256, None);
        UpdateManifest::parse(b"not json").unwrap_err();
        UpdateManifest::parse(br#"{"notes":"no version"}"#).unwrap_err();
        let minimal = UpdateManifest::parse(br#"{"version":"1"}"#).expect("minimal");
        assert!(minimal.platforms.is_empty() && minimal.notes.is_empty());
    }

    #[test]
    fn a_semantic_version_is_a_manifest_without_a_build() {
        let old = UpdateManifest::parse(br#"{"version":"0.2.1-beta.1"}"#).expect("old feed");
        assert_eq!(old.build(), None);
        let odd = UpdateManifest::parse(br#"{"version":"soon"}"#).expect("odd feed");
        assert_eq!(odd.build(), None);
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
        assert_eq!(m.version, "412");
        let missing = serve_once("404 Not Found", "404: Not Found");
        assert!(matches!(fetch_manifest(&missing), Err(Error::NotFound(_))));
    }

    /// Serves each request from `routes` (path → raw response), `count` times, on a loopback port.
    fn serve_routes(routes: Vec<(&'static str, String)>, count: usize) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for _ in 0..count {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let reply = routes.iter().find(|(p, _)| *p == path).map_or_else(
                    || {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    },
                    |(_, r)| r.clone(),
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(to: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    /// SHA-256 of "hello world\n".
    const HELLO_SHA: &str = "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447";

    #[test]
    fn downloads_a_file_through_a_redirect_and_hashes_it() {
        let base = serve_routes(vec![("/a", redirect("/b")), ("/b", ok("hello world\n"))], 2);
        let dir = tempfile::tempdir().expect("dir");
        let dest = dir.path().join("f");
        let mut last = (0, None);
        let d = download(&format!("{base}/a"), &dest, 1024, &mut |done, total| {
            last = (done, total);
        })
        .expect("download");
        assert_eq!(d.bytes, 12);
        assert_eq!(d.sha256, HELLO_SHA);
        assert_eq!(last, (12, Some(12)));
        assert_eq!(std::fs::read(&dest).expect("file"), b"hello world\n");
    }

    #[test]
    fn a_download_over_the_limit_is_refused_and_leaves_no_file() {
        let base = serve_routes(vec![("/big", ok("hello world\n"))], 1);
        let dir = tempfile::tempdir().expect("dir");
        let dest = dir.path().join("f");
        let err = download(&format!("{base}/big"), &dest, 5, &mut |_, _| {}).expect_err("too big");
        assert!(err.to_string().contains("larger than"), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn a_missing_file_is_not_found_and_leaves_no_file() {
        let base = serve_routes(vec![], 1);
        let dir = tempfile::tempdir().expect("dir");
        let dest = dir.path().join("f");
        let err = download(&format!("{base}/nope"), &dest, 5, &mut |_, _| {}).expect_err("404");
        assert!(matches!(err, Error::NotFound(_)), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn plain_http_is_refused_except_on_loopback() {
        let dir = tempfile::tempdir().expect("dir");
        let err = download(
            "http://example.invalid/x",
            &dir.path().join("f"),
            5,
            &mut |_, _| {},
        )
        .expect_err("http");
        assert!(err.to_string().contains("not over HTTPS"), "{err}");
        // A redirect from loopback to a plain-HTTP host is refused too.
        let base = serve_routes(vec![("/a", redirect("http://example.invalid/b"))], 1);
        let err = download_bytes(&format!("{base}/a"), 5).expect_err("downgrade");
        assert!(err.to_string().contains("not over HTTPS"), "{err}");
    }

    #[test]
    fn redirect_targets_are_resolved_or_refused() {
        assert_eq!(
            resolve_location("https://a.example/x/y", "https://b.example/z").expect("absolute"),
            "https://b.example/z"
        );
        assert_eq!(
            resolve_location("https://a.example/x/y?q=1", "/z").expect("path"),
            "https://a.example/z"
        );
        assert_eq!(
            resolve_location("http://127.0.0.1:9", "/z").expect("no path in base"),
            "http://127.0.0.1:9/z"
        );
        resolve_location("https://a.example/x", "z").unwrap_err();
        resolve_location("https://a.example/x", "//evil.example/z").unwrap_err();
    }

    #[test]
    fn too_many_redirects_end_the_download() {
        let base = serve_routes(vec![("/loop", redirect("/loop"))], 6);
        let err = download_bytes(&format!("{base}/loop"), 5).expect_err("loop");
        assert!(err.to_string().contains("redirects"), "{err}");
    }

    #[test]
    fn a_small_file_downloads_into_memory() {
        let base = serve_routes(vec![("/s", ok("hello world\n"))], 1);
        assert_eq!(
            download_bytes(&format!("{base}/s"), 1024).expect("bytes"),
            b"hello world\n"
        );
    }
}
