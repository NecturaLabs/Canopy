//! Canopy's own update check. It reads a small JSON manifest from the public
//! `NecturaLabs/app-updates` repository (stable releases in `canopy/latest.json`, pre-releases in
//! `canopy/beta.json`), compares its version with this build and, when newer, offers the release
//! notes and the download for this platform. It never downloads or installs anything by itself
//! and sends nothing about the user.

use crossbeam_channel::{Receiver, unbounded};

use canopy_core::update::Version;
use canopy_hosting::updates::{self, UpdateManifest};

/// Folder of the manifests. Packagers can point a build elsewhere, or disable the check with an
/// empty value, through the `CANOPY_UPDATE_FEED` environment variable at compile time.
pub const UPDATE_FEED: &str = match option_env!("CANOPY_UPDATE_FEED") {
    Some(url) => url,
    None => "https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy",
};

/// Where people browse releases when a manifest names no page of its own.
pub const RELEASES_PAGE: &str = "https://github.com/NecturaLabs/app-updates/releases";

/// How often the automatic check runs at most.
pub const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// This build's version.
pub fn current() -> Option<Version> {
    Version::parse(env!("CARGO_PKG_VERSION"))
}

/// A newer version than this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub version: Version,
    pub manifest: UpdateManifest,
}

impl Available {
    /// The release page, else the list of releases.
    pub fn page(&self) -> String {
        self.manifest
            .release_page
            .clone()
            .unwrap_or_else(|| RELEASES_PAGE.to_owned())
    }

    /// The download for this platform, else the release page.
    pub fn download(&self) -> String {
        self.manifest
            .artifact()
            .map(|a| a.url.clone())
            .unwrap_or_else(|| self.page())
    }
}

/// Where the check stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Box<Available>),
    Failed(String),
}

/// Runs checks on a background thread.
#[derive(Debug, Default)]
pub struct Updater {
    pub state: UpdateState,
    rx: Option<Receiver<Result<Option<Available>, String>>>,
    /// The user asked (show the result even when up to date or failed).
    pub manual: bool,
}

/// Fetches the stable manifest (and the beta one when asked) and returns the newest version
/// above this build.
fn check(include_prerelease: bool) -> Result<Option<Available>, String> {
    let feed = UPDATE_FEED.trim_end_matches('/');
    let mut channels = vec![format!("{feed}/latest.json")];
    if include_prerelease {
        channels.push(format!("{feed}/beta.json"));
    }
    let mut best: Option<Available> = None;
    let mut first_error = None;
    for url in channels {
        match updates::fetch_manifest(&url) {
            Ok(manifest) => {
                let Some(version) = Version::parse(&manifest.version) else {
                    continue;
                };
                if best.as_ref().is_none_or(|b| version > b.version) {
                    best = Some(Available { version, manifest });
                }
            }
            // No beta channel published is not an error.
            Err(canopy_hosting::Error::NotFound(_)) if url.ends_with("beta.json") => {}
            Err(e) => {
                first_error.get_or_insert(e.to_string());
            }
        }
    }
    match (best, first_error) {
        (Some(b), _) => Ok(current().is_some_and(|c| b.version > c).then_some(b)),
        (None, Some(e)) => Err(e),
        (None, None) => Ok(None),
    }
}

impl Updater {
    /// Whether checks are possible in this build.
    pub fn enabled() -> bool {
        !UPDATE_FEED.trim().is_empty()
    }

    /// Starts a check unless one is running.
    pub fn start(&mut self, ctx: &egui::Context, include_prerelease: bool, manual: bool) {
        if !Self::enabled() {
            return;
        }
        if self.rx.is_some() {
            // A check is already running: a manual request rides on it and still gets an answer.
            self.manual |= manual;
            return;
        }
        let (tx, rx) = unbounded();
        let failed = tx.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = tx.send(check(include_prerelease));
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            // Reported through the next poll like any other failure.
            tracing::warn!("could not start the update check: {e}");
            let _ = failed.send(Err(format!("Could not start the update check: {e}")));
        }
        self.rx = Some(rx);
        self.state = UpdateState::Checking;
        self.manual = manual;
    }

    /// Collects a finished check. Returns true when the state changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return false,
            // The thread ended without an answer (it panicked): stop showing "Checking".
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Err("The update check stopped unexpectedly.".to_owned())
            }
        };
        self.rx = None;
        self.state = match result {
            Ok(Some(a)) => UpdateState::Available(Box::new(a)),
            Ok(None) => UpdateState::UpToDate,
            Err(e) => {
                tracing::info!("update check failed: {e}");
                UpdateState::Failed(e)
            }
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_has_a_version_and_a_feed() {
        assert!(current().is_some(), "the crate version parses");
        assert!(UPDATE_FEED.starts_with("https://"));
    }

    #[test]
    fn available_falls_back_to_the_release_list() {
        let manifest = UpdateManifest::parse(br#"{"version":"9.0.0"}"#).expect("manifest");
        let a = Available {
            version: Version::parse("9.0.0").expect("version"),
            manifest,
        };
        assert_eq!(a.page(), RELEASES_PAGE);
        assert_eq!(a.download(), RELEASES_PAGE);
    }

    #[test]
    fn a_check_whose_thread_died_fails_instead_of_spinning() {
        let (tx, rx) = unbounded();
        let mut u = Updater {
            state: UpdateState::Checking,
            rx: Some(rx),
            manual: true,
        };
        assert!(!u.poll(), "still running");
        assert_eq!(u.state, UpdateState::Checking);
        drop(tx);
        assert!(u.poll());
        assert!(matches!(u.state, UpdateState::Failed(_)), "{:?}", u.state);
        assert!(!u.poll(), "reported once");
    }

    #[test]
    fn a_finished_check_is_collected() {
        let (tx, rx) = unbounded();
        let mut u = Updater {
            state: UpdateState::Checking,
            rx: Some(rx),
            manual: false,
        };
        let _ = tx.send(Ok(None));
        assert!(u.poll());
        assert_eq!(u.state, UpdateState::UpToDate);
    }
}
