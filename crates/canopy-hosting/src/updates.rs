//! Canopy's own update feed.
//!
//! A small JSON manifest in the public `NecturaLabs/app-updates` repository, fetched anonymously
//! over HTTPS. Nothing about the user is sent (no identifiers, no query string); the request
//! carries only `User-Agent: Canopy/<version>`.
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
//!
//! Each manifest is signed: `latest.json.minisig` beside it holds a minisign signature
//! (prehashed Ed25519) made by the release workflow with a key only the `release` environment
//! holds. A manifest is trusted only when its signature verifies against a key in
//! [`MANIFEST_KEYS`], so whoever can write the feed repository still cannot point Canopy at an
//! archive of their own: the archive's SHA-256 comes from the signed manifest.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig};

use crate::error::{Error, Result};
use crate::http::{plain_text, shorten};

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
    /// The SHA-256 of each installer script in the build's release (`install.sh`,
    /// `install.ps1`): an update that runs one, or refreshes `uninstall.sh` from it, checks the
    /// script against this signed list.
    #[serde(default)]
    pub installers: BTreeMap<String, String>,
}

impl UpdateManifest {
    /// Parses a manifest.
    pub fn parse(json: &[u8]) -> Result<Self> {
        serde_json::from_slice(json)
            .map_err(|e| Error::Parse(shorten(&format!("update manifest: {e}"))))
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

/// The minisign public keys a manifest may be signed with (`packaging/update-signing.pub`). A new
/// key is added here, and released, before the workflow signs with it.
pub const MANIFEST_KEYS: &[&str] = &["RWS8kMOXkmLyv2FOTa9wPxIv1i3pWYghSD/4ba+BI0zuHKj2DD7Zxw6L"];

/// A key that replaces [`MANIFEST_KEYS`] in a build made with `CANOPY_UPDATE_KEY` set, for
/// testing an update end to end against a local feed signed with a key of one's own
/// (`docs/playbook.md`). Release builds never set it.
const KEY_OVERRIDE: Option<&str> = option_env!("CANOPY_UPDATE_KEY");

/// Fetches the manifest at `url` and its signature at `<url>.minisig`, and parses the manifest
/// only once the signature verifies against one of [`MANIFEST_KEYS`].
pub fn fetch_manifest(url: &str) -> Result<UpdateManifest> {
    match KEY_OVERRIDE {
        Some(key) => fetch_signed_manifest(url, &[key]),
        None => fetch_signed_manifest(url, MANIFEST_KEYS),
    }
}

/// [`fetch_manifest`] with the keys given. The channel is the manifest's file name
/// (`latest.json`, `beta.json`), which the signature must name.
pub fn fetch_signed_manifest(url: &str, keys: &[&str]) -> Result<UpdateManifest> {
    let channel = url
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".json"))
        .unwrap_or_default();
    let manifest = fetch_feed_file(url, Some("application/json"))?;
    let signature = fetch_feed_file(&format!("{url}.minisig"), None).map_err(|e| match e {
        Error::NotFound(_) => Error::Parse(format!("the update manifest at {url} is not signed")),
        other => other,
    })?;
    verify_manifest(&manifest, &signature, keys, channel)
}

/// The trusted comment the release workflow signs with (`tools/sign-manifest.py`).
fn trusted_comment(channel: &str, build: &str) -> String {
    format!("Canopy update manifest: channel={channel} build={build}")
}

/// Checks a manifest's minisign signature against `keys` (prehashed signatures only) and that it
/// was signed for `channel` and the build the manifest names, then parses the manifest.
pub fn verify_manifest(
    manifest: &[u8],
    signature: &[u8],
    keys: &[&str],
    channel: &str,
) -> Result<UpdateManifest> {
    // Right after a release the feed's cached copies of the manifest and its signature can
    // briefly disagree.
    let refused = || {
        Error::Parse(
            "the update manifest's signature does not verify (if a build was just released, \
             try again in a few minutes)"
                .to_owned(),
        )
    };
    let unreadable = |e: &dyn std::fmt::Display| {
        Error::Parse(shorten(&format!(
            "the update manifest's signature is unreadable: {e}"
        )))
    };
    let text = std::str::from_utf8(signature).map_err(|e| unreadable(&e))?;
    let signature = minisign_verify::Signature::decode(text).map_err(|e| unreadable(&e))?;
    let verifies = |legacy: bool| {
        keys.iter().any(|key| {
            minisign_verify::PublicKey::from_base64(key)
                .is_ok_and(|key| key.verify(manifest, &signature, legacy).is_ok())
        })
    };
    if !verifies(false) {
        if verifies(true) {
            return Err(Error::Parse(
                "the update manifest is signed in an older format that Canopy no longer accepts"
                    .to_owned(),
            ));
        }
        return Err(refused());
    }
    let parsed = UpdateManifest::parse(manifest)?;
    if signature.trusted_comment() != trusted_comment(channel, &parsed.version) {
        return Err(Error::Parse(format!(
            "the update manifest's signature is for another channel or build, not for build {} on the {channel} channel",
            plain_text(&parsed.version, MAX_ERROR_TEXT)
        )));
    }
    Ok(parsed)
}

/// Fetches one file of the update feed, asking for `accept` when given.
fn fetch_feed_file(url: &str, accept: Option<&str>) -> Result<Vec<u8>> {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(FEED_CONNECT_TIMEOUT))
        .timeout_global(Some(FEED_TIMEOUT))
        .user_agent(crate::http::user_agent())
        .tls_config(tls);
    if url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost") {
        config = config.proxy(None);
    }
    let agent = crate::transport::agent(config.build());
    let mut request = agent.get(url);
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }
    let mut response = request
        .call()
        .map_err(|e| Error::Network(shorten(&e.to_string())))?;
    let status = response.status().as_u16();
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_MANIFEST)
        .read_to_vec()
        .map_err(|e| Error::Network(shorten(&e.to_string())))?;
    match status {
        s if crate::http::status::SUCCESS.contains(&s) => Ok(bytes),
        crate::http::status::NOT_FOUND => {
            Err(Error::NotFound(format!("no update manifest at {url}")))
        }
        _ => Err(Error::Api {
            status,
            message: plain_text(&String::from_utf8_lossy(&bytes), MAX_ERROR_TEXT),
        }),
    }
}

/// Where every build's release files are downloadable: `<this>/canopy-build-<n>/<file>`.
pub const RELEASE_DOWNLOADS: &str = "https://github.com/NecturaLabs/app-updates/releases/download";

/// Redirects followed, each of them to HTTPS.
const MAX_REDIRECTS: usize = 5;
/// Connecting to the update feed gives up after this long, and the whole fetch after the longer one.
const FEED_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FEED_TIMEOUT: Duration = Duration::from_secs(20);
/// A release download connects within this long and may take up to half an hour.
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(30);
/// Largest update manifest read.
const MAX_MANIFEST: u64 = 1024 * 1024;
/// Largest error body read from a failed download.
const MAX_ERROR_BODY: u64 = 4096;
/// The most characters of a server's error text kept.
const MAX_ERROR_TEXT: usize = 200;
/// How many bytes a download is read in at a time.
const READ_CHUNK: usize = 64 * 1024;
/// The length of `://`, which ends a URL's scheme.
const SCHEME_SEPARATOR_LEN: usize = 3;
/// The redirect statuses followed: moved permanently, found, see other, and the two that keep
/// the method.
const REDIRECTS: [u16; 5] = [301, 302, 303, 307, 308];

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
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
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
        let after_scheme = from.find("://").map_or(0, |i| i + SCHEME_SEPARATOR_LEN);
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
        .timeout_connect(Some(DOWNLOAD_CONNECT_TIMEOUT))
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .user_agent(crate::http::user_agent())
        .tls_config(tls);
    if is_loopback_url(url) {
        config = config.proxy(None);
    }
    let agent = crate::transport::agent(config.build());
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        if !current.starts_with("https://") && !is_loopback_url(&current) {
            // After a redirect, the address is the server's choice.
            return Err(Error::Network(shorten(&format!(
                "refusing a download that is not over HTTPS: {}",
                canopy_core::cli::redact_url(&current)
            ))));
        }
        let mut response = agent
            .get(&current)
            .call()
            .map_err(|e| Error::Network(shorten(&e.to_string())))?;
        let status = response.status().as_u16();
        if REDIRECTS.contains(&status) {
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
            s if crate::http::status::SUCCESS.contains(&s) => {}
            crate::http::status::NOT_FOUND => {
                return Err(Error::NotFound(format!("nothing at {url}")));
            }
            _ => {
                let body = response
                    .body_mut()
                    .with_config()
                    .limit(MAX_ERROR_BODY)
                    .read_to_vec()
                    .unwrap_or_default();
                return Err(Error::Api {
                    status,
                    message: plain_text(&String::from_utf8_lossy(&body), MAX_ERROR_TEXT),
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
        let mut buf = vec![0u8; READ_CHUNK];
        let mut done = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| Error::Network(shorten(&e.to_string())))?;
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

    const SAMPLE: &str = include_str!("../testdata/sample-manifest.json");
    const SAMPLE_SIG: &str = include_str!("../testdata/sample-manifest.json.minisig");
    /// The public key of a throwaway key that signed `SAMPLE`; never a release key.
    fn test_key() -> String {
        include_str!("../testdata/update-test.pub")
            .lines()
            .last()
            .expect("key line")
            .to_owned()
    }

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
    fn fetches_a_signed_manifest_over_http() {
        let key = test_key();
        let base = serve_routes(
            vec![
                ("/latest.json", ok(SAMPLE)),
                ("/latest.json.minisig", ok(SAMPLE_SIG)),
            ],
            2,
        );
        let m = fetch_signed_manifest(&format!("{base}/latest.json"), &[&key]).expect("fetch");
        assert_eq!(m.version, "412");
        let missing = serve_once("404 Not Found", "404: Not Found");
        assert!(matches!(
            fetch_signed_manifest(&missing, &[&key]),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn an_unsigned_or_altered_manifest_is_refused() {
        let key = test_key();
        // No signature beside it.
        let base = serve_routes(vec![("/latest.json", ok(SAMPLE))], 2);
        let err =
            fetch_signed_manifest(&format!("{base}/latest.json"), &[&key]).expect_err("unsigned");
        assert!(err.to_string().contains("not signed"), "{err}");
        // One changed byte: another archive checksum.
        let altered = SAMPLE.replace("\"ab\"", "\"ac\"");
        assert_ne!(altered, SAMPLE);
        verify_manifest(altered.as_bytes(), SAMPLE_SIG.as_bytes(), &[&key], "latest")
            .expect_err("altered");
        // A key Canopy does not trust, and the release keys for a test signature.
        verify_manifest(
            SAMPLE.as_bytes(),
            SAMPLE_SIG.as_bytes(),
            MANIFEST_KEYS,
            "latest",
        )
        .expect_err("other key");
        verify_manifest(SAMPLE.as_bytes(), b"not a signature", &[&key], "latest")
            .expect_err("garbage");
        verify_manifest(SAMPLE.as_bytes(), SAMPLE_SIG.as_bytes(), &[&key], "latest")
            .expect("intact");
    }

    /// The signature vouches for the channel and the build: a signed stable manifest is no beta
    /// manifest, and a manifest served under another channel's name is refused.
    #[test]
    fn a_signature_binds_the_channel_and_build() {
        let key = test_key();
        assert_eq!(
            verify_manifest(SAMPLE.as_bytes(), SAMPLE_SIG.as_bytes(), &[&key], "latest")
                .expect("latest")
                .build(),
            Some(412)
        );
        let err = verify_manifest(SAMPLE.as_bytes(), SAMPLE_SIG.as_bytes(), &[&key], "beta")
            .expect_err("another channel");
        assert!(
            err.to_string().contains("another channel or build"),
            "{err}"
        );
        let base = serve_routes(
            vec![
                ("/beta.json", ok(SAMPLE)),
                ("/beta.json.minisig", ok(SAMPLE_SIG)),
            ],
            2,
        );
        fetch_signed_manifest(&format!("{base}/beta.json"), &[&key]).expect_err("served as beta");
    }

    /// minisign's legacy, not prehashed, signatures are refused even when they are genuine.
    #[test]
    fn a_legacy_signature_is_refused() {
        let key = test_key();
        let legacy = include_str!("../testdata/sample-manifest.json.legacy-minisig");
        let decoded = minisign_verify::Signature::decode(legacy).expect("decode");
        minisign_verify::PublicKey::from_base64(&key)
            .expect("key")
            .verify(SAMPLE.as_bytes(), &decoded, true)
            .expect("a genuine legacy signature");
        let err = verify_manifest(SAMPLE.as_bytes(), legacy.as_bytes(), &[&key], "latest")
            .expect_err("legacy");
        assert!(err.to_string().contains("older format"), "{err}");
    }

    /// What `tools/sign-manifest.py` signs, Canopy accepts: the two implementations agree.
    /// Skipped where python3 or openssl is missing.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "runs the release's signer")]
    fn the_release_signer_makes_signatures_canopy_accepts() {
        let present = |program: &str, arg: &str| {
            std::process::Command::new(program)
                .arg(arg)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !present("python3", "--version") || !present("openssl", "version") {
            return;
        }
        let dir = tempfile::tempdir().expect("tmp");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/sign-manifest.py");
        let out = std::process::Command::new("python3")
            .arg(&script)
            .arg("emit")
            .arg(dir.path())
            .output()
            .expect("run the signer");
        assert!(
            out.status.success(),
            "the signer failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let read = |name: &str| std::fs::read(dir.path().join(name)).expect(name);
        let public = String::from_utf8(read("test.pub")).expect("utf-8");
        let key = public.lines().last().expect("key line").to_owned();
        let manifest = verify_manifest(
            &read("latest.json"),
            &read("latest.json.minisig"),
            &[&key],
            "latest",
        )
        .expect("Canopy accepts the signer's signature");
        assert_eq!(manifest.build(), Some(1700));
        verify_manifest(
            &read("latest.json"),
            &read("latest.json.minisig"),
            &[&key],
            "beta",
        )
        .expect_err("another channel");
    }

    #[test]
    fn a_manifest_names_its_installers_checksums() {
        let m = UpdateManifest::parse(SAMPLE.as_bytes()).expect("parse");
        assert_eq!(
            m.installers.get("install.sh").map(String::as_str),
            Some("c".repeat(64).as_str())
        );
        let none = UpdateManifest::parse(br#"{"version":"1"}"#).expect("parse");
        assert!(none.installers.is_empty());
    }

    #[test]
    fn the_release_key_is_the_published_one() {
        let published = include_str!("../../../packaging/update-signing.pub");
        assert_eq!(MANIFEST_KEYS, [published.lines().last().expect("key line")]);
        minisign_verify::PublicKey::from_base64(MANIFEST_KEYS[0]).expect("a minisign key");
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
    fn the_feed_servers_words_are_reported_as_plain_text() {
        let plain = |e: &Error| {
            let text = e.to_string();
            text.chars().count() <= 400
                && !text
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '\u{202E}' | '\u{3164}'))
        };
        let dirty = format!("x\u{202E}y\n{}", "z".repeat(1_000));
        // A refusal's body, a malformed redirect target, and a manifest field of the wrong type.
        let failed = format!(
            "HTTP/1.1 500 Oops\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{dirty}",
            dirty.len()
        );
        let moved = format!(
            "HTTP/1.1 301 Moved\r\nLocation: /a\u{202E}b{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "c".repeat(1_000)
        );
        let base = serve_routes(vec![("/failed", failed), ("/moved", moved)], 2);
        for path in ["/failed", "/moved"] {
            let err = fetch_signed_manifest(&format!("{base}{path}"), &[]).expect_err(path);
            assert!(plain(&err), "{path}: {err:?}");
        }
        let wrong = format!(
            r#"{{"version":"1","platforms":"{}"}}"#,
            "\u{3164}".repeat(10_000)
        );
        let err = UpdateManifest::parse(wrong.as_bytes()).expect_err("wrong type");
        assert!(plain(&err), "{err:?}");
        // A redirect to plain HTTP names its target, which the server chose.
        let far = format!("http://example.invalid/{}", "a".repeat(5_000));
        let base = serve_routes(vec![("/a", redirect(&far))], 1);
        let err = download_bytes(&format!("{base}/a"), 5).expect_err("downgrade");
        assert!(plain(&err), "{err:?}");
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
