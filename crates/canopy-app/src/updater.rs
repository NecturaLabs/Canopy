//! Canopy's own update check. It reads a small JSON manifest from the public
//! `NecturaLabs/app-updates` repository (stable builds in `canopy/latest.json`, beta builds in
//! `canopy/beta.json`), compares its build number with this build's and, when newer, offers the
//! release notes and the download for this platform. It never downloads or installs anything by
//! itself and sends nothing about the user. Installing is a separate step the user asks for in
//! the update dialog (`Updater::install`, which runs `selfupdate/mod.rs`).

use crossbeam_channel::{Receiver, unbounded};

use canopy_core::update::is_newer;
use canopy_hosting::updates::{self, UpdateManifest};

use crate::selfupdate::{self, Install, Installed, Phase, Plan};

/// Folder of the manifests. Packagers can point a build elsewhere, or disable the check with an
/// empty value, through the `CANOPY_UPDATE_FEED` environment variable at compile time.
pub const UPDATE_FEED: &str = match option_env!("CANOPY_UPDATE_FEED") {
    Some(url) => url,
    None => "https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy",
};

/// Folder of the release downloads an update may come from (`<this>/canopy-build-<n>/<file>`).
/// Packagers and the tests of the updater can point a build elsewhere through the
/// `CANOPY_UPDATE_DOWNLOADS` environment variable at compile time.
pub const UPDATE_DOWNLOADS: &str = match option_env!("CANOPY_UPDATE_DOWNLOADS") {
    Some(url) => url,
    None => updates::RELEASE_DOWNLOADS,
};

/// Where people browse releases when a manifest names no page of its own.
pub const RELEASES_PAGE: &str = "https://github.com/NecturaLabs/app-updates/releases";

/// How often the automatic check runs at most.
pub const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// A newer build than this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub build: u64,
    pub manifest: UpdateManifest,
}

impl Available {
    /// The build as the interface names it: `build 412`.
    pub fn label(&self) -> String {
        crate::build_info::name(self.build)
    }

    /// What "Skip this version" stores in `skipped_version`: the build number, `412`.
    pub fn skip_key(&self) -> String {
        self.build.to_string()
    }

    /// Whether `skipped` (the saved `skipped_version`) skips this build. Also reads `build 412`,
    /// which the status bar's menu once stored.
    pub fn is_skipped(&self, skipped: Option<&str>) -> bool {
        skipped.is_some_and(|s| s.trim().trim_start_matches("build ") == self.skip_key())
    }

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
            .map_or_else(|| self.page(), |a| a.url.clone())
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

/// Where installing an update stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InstallState {
    #[default]
    Idle,
    Running {
        build: u64,
        phase: Phase,
    },
    Done {
        build: u64,
        installed: Installed,
    },
    Failed {
        build: u64,
        error: String,
    },
}

/// What the installing thread reports.
#[derive(Debug)]
enum InstallMsg {
    Phase(Phase),
    Finished(Result<Installed, String>),
}

/// Runs checks, and installs, on background threads.
#[derive(Debug)]
pub struct Updater {
    pub state: UpdateState,
    rx: Option<Receiver<Result<Option<Available>, String>>>,
    /// The user asked (show the result even when up to date or failed).
    pub manual: bool,
    /// How this copy of Canopy was installed: decides how an update is applied.
    pub kind: Install,
    pub install: InstallState,
    install_rx: Option<Receiver<InstallMsg>>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            state: UpdateState::default(),
            rx: None,
            manual: false,
            kind: selfupdate::detect(),
            install: InstallState::default(),
            install_rx: None,
        }
    }
}

/// Fetches the stable manifest (and the beta one when asked) and returns the newest build above
/// this one.
fn check(include_beta: bool) -> Result<Option<Available>, String> {
    let feed = UPDATE_FEED.trim_end_matches('/');
    let mut channels = vec![format!("{feed}/latest.json")];
    if include_beta {
        channels.push(format!("{feed}/beta.json"));
    }
    let mut best: Option<Available> = None;
    let mut first_error = None;
    for url in channels {
        match updates::fetch_manifest(&url) {
            Ok(manifest) => {
                // A manifest without a build number (a feed from before builds were numbered)
                // is older than every build.
                let Some(build) = manifest.build() else {
                    continue;
                };
                if best.as_ref().is_none_or(|b| build > b.build) {
                    best = Some(Available { build, manifest });
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
        (Some(b), _) => Ok(crate::build_info::number()
            .is_some_and(|running| is_newer(Some(b.build), running))
            .then_some(b)),
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
    pub fn start(&mut self, ctx: &egui::Context, include_beta: bool, manual: bool) {
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
                let _ = tx.send(check(include_beta));
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

    /// Installs `available` on a background thread, unless an install is running: in place where
    /// the one-line installer put Canopy, else by running the installer. Progress and the result
    /// show in `install`.
    pub fn install(&mut self, ctx: &egui::Context, available: &Available) {
        if self.install_rx.is_some() {
            return;
        }
        let build = available.build;
        let (tx, rx) = unbounded();
        let kind = self.kind.clone();
        let plan = Plan::from_manifest(&available.manifest, UPDATE_DOWNLOADS);
        let thread_ctx = ctx.clone();
        let thread_tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let mut last = std::time::Instant::now();
                let mut report = |phase: Phase| {
                    // The download reports every chunk: the window needs a few updates a second.
                    let progress = matches!(phase, Phase::Downloading { .. });
                    if progress && last.elapsed() < std::time::Duration::from_millis(100) {
                        return;
                    }
                    last = std::time::Instant::now();
                    let _ = thread_tx.send(InstallMsg::Phase(phase));
                    thread_ctx.request_repaint();
                };
                let result = match (&kind, plan) {
                    (Install::Managed { root }, Ok(plan)) => {
                        selfupdate::install_managed(root, &plan, &mut report)
                    }
                    (Install::Managed { .. }, Err(e)) => Err(e),
                    (Install::Other, _) => {
                        selfupdate::install_with_installer(UPDATE_FEED, build, &mut report)
                    }
                };
                let _ = thread_tx.send(InstallMsg::Finished(result));
                thread_ctx.request_repaint();
            });
        if let Err(e) = spawned {
            tracing::warn!("could not start the update: {e}");
            let _ = tx.send(InstallMsg::Finished(Err(format!(
                "Could not start the update: {e}"
            ))));
        }
        self.install_rx = Some(rx);
        self.install = InstallState::Running {
            build,
            phase: Phase::Downloading {
                done: 0,
                total: None,
            },
        };
    }

    /// Collects what the installing thread reported. Returns true when the state changed.
    pub fn poll_install(&mut self) -> bool {
        let Some(rx) = &self.install_rx else {
            return false;
        };
        let InstallState::Running { build, .. } = self.install else {
            self.install_rx = None;
            return false;
        };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(InstallMsg::Phase(phase)) => {
                    self.install = InstallState::Running { build, phase };
                    changed = true;
                }
                Ok(InstallMsg::Finished(result)) => {
                    self.install_rx = None;
                    self.install = match result {
                        Ok(installed) => {
                            tracing::info!(build, notes = ?installed.notes, "update installed");
                            // The running copy is the old build until it restarts, but there is
                            // nothing left to offer.
                            self.state = UpdateState::UpToDate;
                            InstallState::Done { build, installed }
                        }
                        Err(error) => {
                            tracing::warn!(build, "update failed: {error}");
                            InstallState::Failed { build, error }
                        }
                    };
                    return true;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => return changed,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.install_rx = None;
                    self.install = InstallState::Failed {
                        build,
                        error: "The update stopped unexpectedly.".to_owned(),
                    };
                    return true;
                }
            }
        }
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
            // A build installed this run waits for a restart: do not offer it again.
            Ok(Some(a)) if matches!(&self.install, InstallState::Done { build, .. } if *build >= a.build) => {
                UpdateState::UpToDate
            }
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
    fn the_feed_and_the_downloads_are_https() {
        assert!(UPDATE_FEED.starts_with("https://"));
        assert!(UPDATE_DOWNLOADS.starts_with("https://"));
    }

    #[test]
    fn available_falls_back_to_the_release_list() {
        let manifest = UpdateManifest::parse(br#"{"version":"900"}"#).expect("manifest");
        let a = Available {
            build: 900,
            manifest,
        };
        assert_eq!(a.page(), RELEASES_PAGE);
        assert_eq!(a.download(), RELEASES_PAGE);
        assert_eq!(a.label(), "build 900");
    }

    /// review-views-4: the status bar stored "build 900" where the check compared "900".
    #[test]
    fn a_skipped_build_is_recognised_whichever_menu_skipped_it() {
        let manifest = UpdateManifest::parse(br#"{"version":"900"}"#).expect("manifest");
        let a = Available {
            build: 900,
            manifest,
        };
        assert_eq!(a.skip_key(), "900");
        assert!(a.is_skipped(Some(&a.skip_key())));
        assert!(a.is_skipped(Some("build 900")), "saved by an earlier build");
        assert!(!a.is_skipped(Some("899")));
        assert!(!a.is_skipped(Some("build 9000")));
        assert!(!a.is_skipped(None));
    }

    #[test]
    fn a_check_whose_thread_died_fails_instead_of_spinning() {
        let (tx, rx) = unbounded();
        let mut u = Updater {
            state: UpdateState::Checking,
            rx: Some(rx),
            manual: true,
            ..Updater::default()
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
            ..Updater::default()
        };
        let _ = tx.send(Ok(None));
        assert!(u.poll());
        assert_eq!(u.state, UpdateState::UpToDate);
    }

    fn running(rx: Receiver<InstallMsg>) -> Updater {
        Updater {
            install: InstallState::Running {
                build: 413,
                phase: Phase::Verifying,
            },
            install_rx: Some(rx),
            ..Updater::default()
        }
    }

    #[test]
    fn install_progress_and_the_result_are_collected() {
        let (tx, rx) = unbounded();
        let mut u = running(rx);
        u.state = UpdateState::Checking;
        assert!(!u.poll_install(), "nothing yet");
        let _ = tx.send(InstallMsg::Phase(Phase::Unpacking));
        assert!(u.poll_install());
        assert_eq!(
            u.install,
            InstallState::Running {
                build: 413,
                phase: Phase::Unpacking
            }
        );
        let installed = Installed {
            exe: None,
            notes: vec![],
        };
        let _ = tx.send(InstallMsg::Finished(Ok(installed.clone())));
        assert!(u.poll_install());
        assert_eq!(
            u.install,
            InstallState::Done {
                build: 413,
                installed
            }
        );
        assert_eq!(u.state, UpdateState::UpToDate, "nothing left to offer");
        assert!(!u.poll_install(), "reported once");
    }

    #[test]
    fn a_failed_install_is_reported_with_its_message() {
        let (tx, rx) = unbounded();
        let mut u = running(rx);
        let _ = tx.send(InstallMsg::Finished(Err("checksum mismatch".to_owned())));
        assert!(u.poll_install());
        assert_eq!(
            u.install,
            InstallState::Failed {
                build: 413,
                error: "checksum mismatch".to_owned()
            }
        );
    }

    #[test]
    fn an_install_thread_that_died_fails_instead_of_spinning() {
        let (tx, rx) = unbounded();
        let mut u = running(rx);
        drop(tx);
        assert!(u.poll_install());
        assert!(
            matches!(u.install, InstallState::Failed { .. }),
            "{:?}",
            u.install
        );
    }
}
