//! The folders whose repositories Canopy trusts, and the switch that turns trust checks off.
//!
//! A repository's own configuration, hooks and filters can make `git` run programs, and in a
//! partial clone a download runs `git fetch`, which obeys that configuration too. Canopy acts on
//! a repository (anything but reading it) only when it lies inside a folder the user trusted
//! in Settings (`Settings::trusted_folders`), unless the user turned trust safety off
//! (`Settings::trust_safety`). Repository threads read both from here, so a change applies to
//! every open tab at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};

use crate::settings::Settings;

/// What the user decided: the trusted folders and whether trust is checked at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// How often the decision has changed, as it stood when it was read.
    pub version: u64,
    pub folders: Vec<PathBuf>,
    /// `false` when the user turned trust safety off: nothing is then asked or refused.
    pub safety: bool,
}

/// The decision and how often it has changed. A change is counted under the same lock that
/// replaces the decision, so a reader never pairs a list with another list's count.
struct Trust {
    state: RwLock<State>,
    changes: AtomicU64,
}

struct State {
    folders: Vec<PathBuf>,
    safety: bool,
    /// Folders tests trusted for themselves ([`add_for_tests`]): kept apart from the published
    /// list, so a test that publishes settings (any that runs the app's frame) cannot take them
    /// from tests running beside it (#254).
    #[cfg(test)]
    test_folders: Vec<PathBuf>,
}

static TRUST: Trust = Trust::new();

impl Trust {
    const fn new() -> Self {
        Self {
            state: RwLock::new(State {
                folders: Vec::new(),
                safety: true,
                #[cfg(test)]
                test_folders: Vec::new(),
            }),
            changes: AtomicU64::new(0),
        }
    }

    #[expect(
        clippy::significant_drop_tightening,
        reason = "the change count moves while the state is still locked, so readers never see one without the other"
    )]
    fn set(&self, folders: &[PathBuf], safety: bool) -> bool {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        if state.folders == folders && state.safety == safety {
            return false;
        }
        state.folders = folders.to_vec();
        state.safety = safety;
        self.changes.fetch_add(1, Ordering::SeqCst);
        true
    }

    fn changes(&self) -> u64 {
        self.changes.load(Ordering::SeqCst)
    }

    fn snapshot(&self) -> Snapshot {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        #[cfg(test)]
        let folders = state
            .folders
            .iter()
            .chain(&state.test_folders)
            .cloned()
            .collect();
        #[cfg(not(test))]
        let folders = state.folders.clone();
        Snapshot {
            version: self.changes(),
            folders,
            safety: state.safety,
        }
    }

    /// What `check` says about the decision as it is when the answer is given. Resolving paths
    /// takes time with no lock held (the UI thread must never wait for it), so a decision that
    /// changed meanwhile makes the answer void and `check` runs again on the new one.
    fn decide(&self, check: impl Fn(&Snapshot) -> bool) -> bool {
        loop {
            let seen = self.snapshot();
            let answer = check(&seen);
            if self.changes() == seen.version {
                return answer;
            }
        }
    }
}

/// Publishes the user's trust decision from `settings`, for every repository thread at once:
/// call it the moment the list or the switch changes, so work that has not started yet sees a
/// removal. Returns whether anything changed.
pub fn publish(settings: &Settings) -> bool {
    TRUST.set(&settings.trusted_folders, settings.trust_safety)
}

/// How many times the decision has changed, for whoever must react to a change once.
pub fn changes() -> u64 {
    TRUST.changes()
}

/// Trusts `folder` for the rest of the test run, whatever settings tests publish meanwhile.
#[cfg(test)]
pub fn add_for_tests(folder: PathBuf) {
    let mut state = TRUST.state.write().unwrap_or_else(PoisonError::into_inner);
    state.test_folders.push(folder);
}

/// The trusted folders, whether trust is checked, and the version of that decision.
pub fn snapshot() -> Snapshot {
    TRUST.snapshot()
}

/// The folders trusting a repository must cover: its working folder, common Git directory and
/// the worktree's private Git directory. Resolve links on the repository thread before omitting
/// a Git folder already covered by the working folder; a linked `.git` directory may point
/// outside it. On a resolution error keep the original path for the gate to recheck.
pub fn repository_folders(workdir: &Path, common_dir: &Path, git_dir: &Path) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for path in [workdir, common_dir, git_dir] {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !folders.iter().any(|folder| path.starts_with(folder)) {
            folders.push(path);
        }
    }
    folders
}

/// Whether `path` names a filesystem root once `.` and `..` are read lexically.
fn is_root(path: &Path) -> bool {
    use std::path::Component;
    let mut depth = 0usize;
    for part in path.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => depth = depth.saturating_sub(1),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    depth == 0
}

/// Whether a download for the repository with these folders is allowed under `trust`: always when
/// trust safety is off, else when the folders lie inside a trusted one.
fn allows_download(
    trust: &Snapshot,
    common_dir: &Path,
    git_dir: &Path,
    workdir: Option<&Path>,
) -> bool {
    !trust.safety
        || canopy_core::partial::paths_in_trusted_folder(
            common_dir,
            git_dir,
            workdir,
            &trust.folders,
        )
}

/// Gives core the rule every download passes right where `git fetch` would start, so that no
/// read path reaches a fetch around the trust list. Safe to call again.
pub fn install_gate() {
    canopy_core::partial::set_download_gate(|common_dir, git_dir, workdir| {
        TRUST.decide(|trust| allows_download(trust, common_dir, git_dir, workdir))
    });
}

/// Whether trusting `parent` covers `folder`: it is `parent` or lies inside it. A reading of the
/// two spellings, without touching the disk (it runs on the UI thread): a path with `..` in it
/// may lead anywhere once links are followed, so it is never taken to cover or to be covered.
/// A spelling this misses only keeps a redundant entry; the gate (core) compares what the
/// folders really are.
fn covers(parent: &Path, folder: &Path) -> bool {
    let plain = |path: &Path| {
        !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    };
    plain(parent) && plain(folder) && folder.starts_with(parent)
}

/// Adds `folder`, made absolute, to `list`, which keeps only the outermost folders: one already
/// covered by an entry is not added, and the entries a new folder covers are dropped. A relative
/// entry would mean another folder from another working directory, so settings never keep one.
/// A filesystem root is refused, whether spelled `/`, `C:\` or `/a/../..`. This is a reading of
/// the path, without touching the repository's disk (it runs on the UI thread); core resolves
/// links when it checks, and a link that leads to a root trusts nothing there. Returns whether
/// the list changed.
pub fn add(list: &mut Vec<PathBuf>, folder: &Path) -> bool {
    let Ok(folder) = std::path::absolute(folder) else {
        return false;
    };
    if is_root(&folder) || list.iter().any(|kept| covers(kept, &folder)) {
        return false;
    }
    list.retain(|kept| !covers(&folder, kept));
    list.push(folder);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #254: a test that publishes settings trusting nothing (any test that runs the app's
    /// frame does) leaves the folders other tests trusted for themselves, whichever tests run
    /// together and in whatever order.
    #[test]
    fn publishing_settings_keeps_the_folders_tests_trusted_for_themselves() {
        let dir = PathBuf::from("/canopy-tests/trusted-by-one-test");
        add_for_tests(dir.clone());
        publish(&Settings::default());
        assert!(
            snapshot().folders.contains(&dir),
            "{:?}",
            snapshot().folders
        );
    }

    #[test]
    fn linked_worktree_offers_cover_every_folder_the_gate_checks()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let workdir = dir.path().join("linked");
        let common = dir.path().join("main").join(".git");
        let private = common.join("worktrees").join("linked");
        std::fs::create_dir(&workdir)?;
        std::fs::create_dir_all(&private)?;
        let folders = repository_folders(&workdir, &common, &private);
        assert_eq!(folders, [workdir.canonicalize()?, common.canonicalize()?]);
        assert!(canopy_core::partial::paths_in_trusted_folder(
            &common,
            &private,
            Some(&workdir),
            &folders
        ));
        assert!(!canopy_core::partial::paths_in_trusted_folder(
            &common,
            &private,
            Some(&workdir),
            &folders[..1]
        ));
        assert_eq!(
            repository_folders(&workdir, &workdir, &workdir),
            [workdir.canonicalize()?]
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_git_directory_link_outside_the_worktree_is_offered()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let workdir = dir.path().join("repo");
        let external = dir.path().join("external");
        std::fs::create_dir(&workdir)?;
        std::fs::create_dir(&external)?;
        let git_dir = workdir.join(".git");
        std::os::unix::fs::symlink(&external, &git_dir)?;
        assert_eq!(
            repository_folders(&workdir, &git_dir, &git_dir),
            [workdir.canonicalize()?, external.canonicalize()?]
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn folders_are_added_once_and_a_root_is_refused() {
        let mut list = Vec::new();
        assert!(add(&mut list, &PathBuf::from("/home/ada/src")));
        assert!(!add(&mut list, &PathBuf::from("/home/ada/src")));
        assert!(!add(&mut list, &PathBuf::from("/")));
        assert_eq!(list, [PathBuf::from("/home/ada/src")]);
    }

    #[test]
    fn a_relative_folder_is_kept_as_an_absolute_one() {
        let mut list = Vec::new();
        assert!(add(&mut list, &PathBuf::from("repo")));
        assert!(list.iter().all(|folder| folder.is_absolute()));
        assert!(list[0].ends_with("repo"));
        let kept = list[0].clone();
        assert!(!add(&mut list, &kept), "already there");
        assert!(!add(&mut list, &PathBuf::new()), "nothing to trust");
    }

    fn snapshot_of(trust: &Trust) -> (u64, Vec<PathBuf>, bool) {
        let seen = trust.snapshot();
        (seen.version, seen.folders, seen.safety)
    }

    #[test]
    fn only_a_real_change_is_counted() {
        let trust = Trust::new();
        let src = PathBuf::from("src");
        assert!(trust.set(std::slice::from_ref(&src), true));
        assert!(!trust.set(std::slice::from_ref(&src), true));
        assert_eq!(trust.changes(), 1);
        assert!(trust.set(&[], true));
        assert_eq!(snapshot_of(&trust), (2, Vec::new(), true));
    }

    #[test]
    fn switching_safety_is_a_change_that_keeps_the_list() {
        let trust = Trust::new();
        let src = PathBuf::from("src");
        trust.set(std::slice::from_ref(&src), true);
        assert!(trust.set(std::slice::from_ref(&src), false));
        assert_eq!(snapshot_of(&trust), (2, vec![src.clone()], false));
        assert!(trust.set(std::slice::from_ref(&src), true));
        assert_eq!(snapshot_of(&trust), (3, vec![src], true));
        assert!(Trust::new().snapshot().safety, "on until the user says");
    }

    #[test]
    fn a_folder_removed_while_the_gate_decides_is_not_trusted() {
        let trust = Trust::new();
        trust.set(&[PathBuf::from("src")], true);
        let asked = std::cell::Cell::new(0);
        let answer = trust.decide(|seen| {
            asked.set(asked.get() + 1);
            if asked.get() == 1 {
                // The user removes the folder while the paths are being resolved.
                trust.set(&[], true);
            }
            !seen.folders.is_empty()
        });
        assert!(!answer, "the answer of a list that no longer holds");
        assert_eq!(asked.get(), 2);
    }

    #[test]
    fn a_list_nobody_changes_is_asked_once() {
        let trust = Trust::new();
        trust.set(&[PathBuf::from("src")], true);
        let asked = std::cell::Cell::new(0);
        assert!(trust.decide(|seen| {
            asked.set(asked.get() + 1);
            !seen.folders.is_empty()
        }));
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn a_download_is_allowed_inside_a_trusted_folder_or_with_safety_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("repo");
        let allowed = |safety: bool, folders: &[&Path]| {
            let trust = Snapshot {
                version: 1,
                folders: folders.iter().map(|folder| folder.to_path_buf()).collect(),
                safety,
            };
            allows_download(&trust, &repo, &repo, Some(&repo))
        };
        assert!(!allowed(true, &[]), "nothing is trusted by default");
        assert!(allowed(false, &[]), "off: nothing is checked");
        assert!(allowed(true, &[dir.path()]), "a trusted parent covers it");
        assert!(
            !allowed(true, &[]),
            "and the switch back on closes it again"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_covers_what_is_below_it_and_nothing_else() {
        let covered = |parent: &str, folder: &str| covers(Path::new(parent), Path::new(folder));
        assert!(covered("/home/ada/src", "/home/ada/src"));
        assert!(covered("/home/ada/src", "/home/ada/src/tide/pool"));
        assert!(covered("/home/ada/src", "/home/ada/src/"), "trailing slash");
        assert!(covered("/home/ada/src/.", "/home/ada/src/tide"), "a dot");
        assert!(!covered("/home/ada/src", "/home/ada/src-old"), "a sibling");
        assert!(!covered("/home/ada/src/tide", "/home/ada/src"), "the child");
        assert!(
            !covered("/home/ada/src", "/home/ada/SRC/tide"),
            "no folding"
        );
        assert!(
            !covered("/home/ada/src", "/home/ada/src/../x"),
            "a link may lead out"
        );
        assert!(!covered("/home/ada/x/..", "/home/ada/x/../src"), "or in");
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_replaces_the_folders_below_it_and_a_covered_one_is_not_added() {
        let mut list = Vec::new();
        assert!(add(&mut list, Path::new("/home/ada/src/tide")));
        assert!(add(&mut list, Path::new("/home/ada/src/reef")));
        assert!(add(&mut list, Path::new("/srv/git")));
        assert!(add(&mut list, Path::new("/home/ada/src")), "the parent");
        assert_eq!(
            list,
            [PathBuf::from("/srv/git"), PathBuf::from("/home/ada/src")],
            "its children are gone, an unrelated entry stays"
        );
        assert!(!add(&mut list, Path::new("/home/ada/src/tide")), "covered");
        assert!(!add(&mut list, Path::new("/home/ada/src")), "already there");
        assert!(add(&mut list, Path::new("/home/ada/src-old")), "a sibling");
        // Where a `..` leads depends on links, which only core can follow: it is kept as given.
        assert!(add(&mut list, Path::new("/home/ada/src/tide/../reef")));
        assert_eq!(list.len(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_reads_as_the_root_is_refused_like_the_root() {
        let mut list = Vec::new();
        assert!(!add(&mut list, &PathBuf::from("/..")));
        assert!(!add(&mut list, &PathBuf::from("/a/../..")));
        assert!(!add(&mut list, &PathBuf::from("/./")));
        assert!(list.is_empty(), "{list:?}");
        assert!(add(&mut list, &PathBuf::from("/a/../b")));
    }
}
