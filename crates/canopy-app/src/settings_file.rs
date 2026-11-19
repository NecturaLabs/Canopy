//! Settings as a file: export to and import from a file the user chooses, and resetting every
//! preference.
//!
//! Only preferences travel. What the user agreed to (the Terms, the Privacy Policy, the crash
//! report answer), the folders trusted to run a repository's configuration and Canopy's memory
//! of the session ([`NOT_PREFERENCES`]) never leave the machine and are never taken from a file:
//! a file must not be able to give consent or trust. Credentials are not settings at all
//! (the GitHub and GitLab CLIs hold them), so they are never in the file. The programs Canopy
//! runs, the list of self-hosted GitLab hosts and everything tied to a folder on this computer
//! ([`THIS_COMPUTER`]) are neither written nor read either, and a file may switch a network
//! feature off but never on ([`NETWORK_SWITCHES`]) nor fetch more often (`AUTO_FETCH`). Git
//! choices that could delete local tags, override the user's own git configuration or send a push
//! somewhere unexpected take a file's value only when it is the safe one ([`GIT_GUARDS`]).
//!
//! The file is JSON: `{"format": "canopy-settings", "version": 1, "exported": "<UTC>",
//! "settings": {…}}`. An import is checked as a whole before anything changes: a file that is
//! not Canopy's, comes from a newer format, or has a value of the wrong kind is refused with a
//! message saying why. The file is read only up to [`MAX_BYTES`], whatever size it claims.

use std::io::Read;
use std::path::PathBuf;

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use serde_json::{Map, Value};

use crate::settings::Settings;

/// The `format` of a settings file.
pub const FORMAT: &str = "canopy-settings";
/// The newest settings file version this build reads and the one it writes.
pub const VERSION: u64 = 1;
/// The largest settings file Canopy reads.
pub const MAX_BYTES: u64 = 1024 * 1024;

/// Settings fields that are not preferences: consent, trust and session memory.
pub const NOT_PREFERENCES: &[&str] = &[
    "accepted_terms",
    "accepted_privacy",
    "crash_reports",
    "crash_reports_version",
    "trusted_folders",
    "trust_safety",
    "identity_asked",
    "trust_off_dismissed",
    "last_update_check",
    "skipped_version",
    "last_seen_version",
    "recent",
    "open_tabs",
    "repo_sidebar",
    "repo_requests",
    "seen_intro",
    "last_crash_seen",
    "dark",
    "reduce_motion",
];

/// Preferences that stay on this computer, with the words the import message uses for them. The
/// editor, terminal and file manager are programs Canopy runs and `gitlab_hosts` is the list of
/// self-hosted hosts Canopy recognises as GitLab and signs in to through `glab`: a file from
/// someone else must not choose either. The clone folder and the per-repository entries are paths on this computer.
pub const THIS_COMPUTER: &[(&str, &str)] = &[
    ("editor", "editor"),
    ("terminal", "terminal"),
    ("file_manager", "file manager"),
    ("gitlab_hosts", "GitLab hosts"),
    ("clone_dir", "clone folder"),
    ("repos", "per-repository settings"),
    ("repo_diff", "per-repository settings"),
];

/// Network features (each its off switch) a file may turn off but never on, with the import
/// message's words for them.
pub const NETWORK_SWITCHES: &[(&str, &str)] = &[
    ("hosting_api", "pull and merge requests"),
    ("download_avatars", "profile pictures"),
    ("check_updates", "update checks"),
    ("include_prereleases", "beta builds"),
];

/// How often a file may have Canopy fetch on its own, with the import message's words: never more
/// often than now, and never when automatic fetching is off here (0 turns it off).
const AUTO_FETCH: (&str, &str) = ("auto_fetch_minutes", "automatic fetching");

/// Whether a file's `auto_fetch_minutes` asks for no more fetching than `current` does.
fn fetches_no_more(current: Option<&Value>, value: &Value) -> bool {
    let now = current.and_then(Value::as_u64).unwrap_or(0);
    value
        .as_u64()
        .is_some_and(|v| v == 0 || (now != 0 && v >= now))
}

/// The one value of a guarded setting a file may set.
#[derive(Debug, Clone, Copy)]
pub enum Safe {
    Bool(bool),
    Text(&'static str),
}

impl Safe {
    fn is(self, value: &Value) -> bool {
        match self {
            Self::Bool(b) => value == &Value::Bool(b),
            Self::Text(t) => value.as_str() == Some(t),
        }
    }
}

/// Git choices where one value can lose data or send it, with the value a file may set and the
/// import message's words. Any other value in a file leaves the current one alone, so a file
/// cannot have a pruning fetch delete local tags that were never pushed (`prune_tags`), take
/// decisions away from the user's git configuration (`follow_git_config`) or have a push without
/// a destination create or update a branch the user did not choose (`push_default`), and a file
/// cannot let Settings write the git configuration (`edit_git_config`) or have Discard delete
/// untracked files inside a submodule (`discard_untracked_in_submodules`). The other
/// git choices are Canopy's defaults or lose nothing, and a file sets them freely.
pub const GIT_GUARDS: &[(&str, Safe, &str)] = &[
    ("prune_tags", Safe::Bool(false), "tag pruning"),
    (
        "follow_git_config",
        Safe::Bool(true),
        "following the git configuration",
    ),
    ("push_default", Safe::Text("Simple"), "where a push goes"),
    (
        "edit_git_config",
        Safe::Bool(false),
        "editing your git config",
    ),
    (
        "discard_untracked_in_submodules",
        Safe::Bool(false),
        "deleting untracked files in submodules",
    ),
];

/// An imported file's settings, and what of the file was not taken.
#[derive(Debug)]
pub struct Imported {
    pub settings: Settings,
    /// What the file named but this computer keeps, in the words of [`THIS_COMPUTER`] and
    /// [`NETWORK_SWITCHES`], each once.
    pub kept: Vec<&'static str>,
}

impl Imported {
    /// The message after an import from `path`.
    pub fn message(&self, path: &std::path::Path) -> String {
        if self.kept.is_empty() {
            format!("Imported from {}.", path.display())
        } else {
            format!(
                "Imported from {}. Kept as they are on this computer: {}.",
                path.display(),
                self.kept.join(", ")
            )
        }
    }
}

fn to_map(s: &Settings) -> Result<Map<String, Value>, String> {
    match serde_json::to_value(s) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err("Settings are not an object.".to_owned()),
        Err(e) => Err(format!("Could not write the settings: {e}")),
    }
}

/// The preferences in `s` as a settings file.
pub fn export(s: &Settings) -> Result<String, String> {
    let mut prefs = to_map(s)?;
    for key in NOT_PREFERENCES {
        prefs.remove(*key);
    }
    for (key, _) in THIS_COMPUTER {
        prefs.remove(*key);
    }
    let file = serde_json::json!({
        "format": FORMAT,
        "version": VERSION,
        "exported": crate::logging::utc_seconds(jiff::Timestamp::now()),
        "settings": prefs,
    });
    serde_json::to_string_pretty(&file).map_err(|e| format!("Could not write the settings: {e}"))
}

/// `current` with the preferences from a settings file; nothing else changes. Preferences the
/// file does not name keep their current value, and so do [`THIS_COMPUTER`]'s, a
/// [`NETWORK_SWITCHES`] feature that is off and a [`GIT_GUARDS`] choice the file sets unsafely.
pub fn import(current: &Settings, text: &str) -> Result<Imported, String> {
    let file: Value =
        serde_json::from_str(text).map_err(|e| format!("This is not a settings file: {e}."))?;
    let Value::Object(mut file) = file else {
        return Err("This is not a settings file: it is not a JSON object.".to_owned());
    };
    if file.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(
            "This is not a Canopy settings file (its \"format\" is not \"canopy-settings\")."
                .to_owned(),
        );
    }
    let version = file
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("The settings file has no version number.")?;
    if version > VERSION {
        return Err(format!(
            "This settings file is version {version}, from a newer Canopy. Update Canopy to import it."
        ));
    }
    let Some(Value::Object(prefs)) = file.remove("settings") else {
        return Err("The settings file has no \"settings\" object.".to_owned());
    };
    let mut merged = to_map(current)?;
    let mut kept = Vec::new();
    let mut keep = |words: &'static str| {
        if !kept.contains(&words) {
            kept.push(words);
        }
    };
    for (key, value) in prefs {
        if NOT_PREFERENCES.contains(&key.as_str()) {
            continue;
        }
        if let Some((_, words)) = THIS_COMPUTER.iter().find(|(k, _)| *k == key) {
            if merged.get(&key) != Some(&value) {
                keep(words);
            }
            continue;
        }
        if let Some((_, words)) = NETWORK_SWITCHES.iter().find(|(k, _)| *k == key)
            && merged.get(&key) == Some(&Value::Bool(false))
        {
            if value != Value::Bool(false) {
                keep(words);
            }
            continue;
        }
        if key == AUTO_FETCH.0 && !fetches_no_more(merged.get(&key), &value) {
            if merged.get(&key) != Some(&value) {
                keep(AUTO_FETCH.1);
            }
            continue;
        }
        if let Some((_, safe, words)) = GIT_GUARDS.iter().find(|(k, ..)| *k == key)
            && !safe.is(&value)
        {
            if merged.get(&key) != Some(&value) {
                keep(words);
            }
            continue;
        }
        merged.insert(key, value);
    }
    let mut settings: Settings = serde_json::from_value(Value::Object(merged))
        .map_err(|e| format!("The settings file has a value Canopy cannot use: {e}."))?;
    settings.migrate();
    Ok(Imported { settings, kept })
}

/// `current` with every preference back to its default; consent, trust and session memory
/// stay.
pub fn reset(current: &Settings) -> Result<Settings, String> {
    let kept = to_map(current)?;
    let mut fresh = to_map(&Settings::default())?;
    for key in NOT_PREFERENCES {
        if let Some(value) = kept.get(*key) {
            fresh.insert((*key).to_owned(), value.clone());
        }
    }
    serde_json::from_value(Value::Object(fresh)).map_err(|e| format!("Could not reset: {e}."))
}

/// What a file chooser and its file access ended with.
#[derive(Debug)]
pub enum Outcome {
    Exported(PathBuf),
    /// A file's text, to [`import`].
    Read(PathBuf, String),
    Cancelled,
    Failed(String),
}

/// A file chooser and the reading or writing after it, on its own thread.
#[derive(Debug)]
pub struct FileTask {
    rx: Receiver<Outcome>,
}

impl FileTask {
    fn start(ctx: &egui::Context, work: impl FnOnce() -> Outcome + Send + 'static) -> Self {
        let (tx, rx) = bounded(1);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("settings-file".into())
            .spawn(move || {
                let _ = tx.send(work());
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            let (tx, rx) = bounded(1);
            let _ = tx.send(Outcome::Failed(format!(
                "Could not open a file chooser: {e}"
            )));
            return Self { rx };
        }
        Self { rx }
    }

    /// Asks where to save `text`, then writes it there.
    pub fn export(ctx: &egui::Context, text: String) -> Self {
        Self::start(ctx, move || {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Export Canopy settings")
                .add_filter("Canopy settings", &["json"])
                .set_file_name("canopy-settings.json")
                .save_file()
            else {
                return Outcome::Cancelled;
            };
            match std::fs::write(&path, text) {
                Ok(()) => Outcome::Exported(path),
                Err(e) => Outcome::Failed(format!("Could not write {}: {e}", path.display())),
            }
        })
    }

    /// Asks for a settings file, then reads it (up to [`MAX_BYTES`]).
    pub fn import(ctx: &egui::Context) -> Self {
        Self::start(ctx, || {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Import Canopy settings")
                .add_filter("Canopy settings", &["json"])
                .pick_file()
            else {
                return Outcome::Cancelled;
            };
            read(path)
        })
    }

    /// The outcome once there is one.
    pub fn poll(&self) -> Option<Outcome> {
        match self.rx.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Outcome::Failed(
                "The file chooser closed unexpectedly. Try again.".to_owned(),
            )),
        }
    }
}

/// A settings file's text, refusing anything but a file and one too large to be settings.
fn read(path: PathBuf) -> Outcome {
    let failed = |e: &dyn std::fmt::Display| {
        Outcome::Failed(format!("Could not read {}: {e}", path.display()))
    };
    // A device or a pipe has no end to wait for.
    match std::fs::metadata(&path) {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Outcome::Failed(format!("{} is not a file.", path.display())),
        Err(e) => return failed(&e),
    }
    match std::fs::File::open(&path)
        .map_err(|e| e.to_string())
        .and_then(read_text)
    {
        Ok(text) => Outcome::Read(path, text),
        Err(e) => failed(&e),
    }
}

/// At most [`MAX_BYTES`] of text: a file that grew after it was chosen is cut off, not read on.
fn read_text(reader: impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!(
            "it is larger than {} KB, too large to be a settings file",
            MAX_BYTES / 1024
        ));
    }
    String::from_utf8(bytes).map_err(|e| format!("it is not text ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CloneProtocol, PullMode, PushMode};
    use crate::ui::theme::ThemeId;

    /// A folder trusted on this machine. Settings keep only absolute folders, and `/srv/trusted`
    /// is not one on Windows.
    fn trusted_folder() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\srv\trusted")
        } else {
            PathBuf::from("/srv/trusted")
        }
    }

    fn customised() -> Settings {
        Settings {
            theme: ThemeId::Daylight,
            match_system: true,
            dark_theme: ThemeId::Observatory,
            pull_mode: PullMode::Rebase,
            clone_protocol: CloneProtocol::Ssh,
            editor: "zed --wait %f".to_owned(),
            check_message: false,
            remove_gone_branches: false,
            accepted_terms: "2026-10-03".to_owned(),
            accepted_privacy: "2026-10-03".to_owned(),
            crash_reports: Some(false),
            trusted_folders: vec![trusted_folder()],
            trust_safety: false,
            recent: vec![PathBuf::from("/srv/repo")],
            ..Settings::default()
        }
    }

    #[test]
    fn an_export_carries_preferences_only_and_imports_back() {
        let s = customised();
        let text = export(&s).unwrap();
        for key in [
            "accepted_terms",
            "crash_reports",
            "trusted_folders",
            "trust_safety",
            "recent",
            "/srv/",
        ] {
            assert!(!text.contains(key), "{key} must not be exported:\n{text}");
        }
        assert!(text.contains("\"format\": \"canopy-settings\""), "{text}");
        let fresh = Settings::default();
        let back = import(&fresh, &text).unwrap().settings;
        assert_eq!(back.theme, ThemeId::Daylight);
        assert!(back.match_system, "Match system travels with its themes");
        assert_eq!(back.dark_theme, ThemeId::Observatory);
        assert_eq!(back.pull_mode, PullMode::Rebase);
        assert!(!back.check_message, "the message checks setting travels");
        assert!(
            !back.remove_gone_branches,
            "removing branches deleted on the remote travels"
        );
        assert_eq!(
            back.editor, "",
            "the programs Canopy runs stay this computer's"
        );
        // Consent and trust stay what they were on this machine.
        assert_eq!(back.accepted_terms, fresh.accepted_terms);
        assert_eq!(back.crash_reports, None);
        assert_eq!(back.trusted_folders, Vec::<PathBuf>::new());
        assert!(back.trust_safety, "the switch is this computer's own");
    }

    #[test]
    fn a_file_cannot_give_consent_or_trust() {
        let fresh = Settings::default();
        let text = r#"{"format": "canopy-settings", "version": 1, "settings": {
            "accepted_terms": "2026-10-03", "accepted_privacy": "2026-10-03",
            "crash_reports": true, "trusted_folders": ["/"], "trust_safety": false,
            "zoom": 1.2}}"#;
        let back = import(&fresh, text).unwrap().settings;
        assert_eq!(back.accepted_terms, fresh.accepted_terms);
        assert_eq!(back.crash_reports, None);
        assert_eq!(back.trusted_folders, Vec::<PathBuf>::new());
        assert!(back.trust_safety, "a file cannot turn trust safety off");
        assert!((back.zoom - 1.2).abs() < f32::EPSILON);
    }

    #[test]
    fn a_file_cannot_turn_trust_safety_on_either() {
        let off = Settings {
            trust_safety: false,
            trusted_folders: vec![trusted_folder()],
            ..Settings::default()
        };
        let text = r#"{"format": "canopy-settings", "version": 1, "settings": {
            "trust_safety": true, "trusted_folders": []}}"#;
        let back = import(&off, text).unwrap().settings;
        assert!(!back.trust_safety, "the choice stays the one made here");
        assert_eq!(back.trusted_folders, off.trusted_folders);
        let reset = reset(&off).unwrap();
        assert!(!reset.trust_safety, "resetting preferences keeps it");
        assert_eq!(reset.trusted_folders, off.trusted_folders);
    }

    #[test]
    fn a_file_cannot_choose_programs_gitlab_hosts_or_network_features() {
        let here = Settings {
            editor: "code --wait %f".to_owned(),
            gitlab_hosts: "gitlab.internal".to_owned(),
            hosting_api: false,
            download_avatars: false,
            check_updates: true,
            ..Settings::default()
        };
        let text = r#"{"format": "canopy-settings", "version": 1, "settings": {
            "gitlab_hosts": "git.evil.example", "editor": "sh -c id %f",
            "terminal": "sh -c id", "file_manager": "sh -c id %d", "clone_dir": "/srv/x",
            "repos": {"/srv/secret": {"display_name": "x"}},
            "repo_diff": {"/srv/secret": {}},
            "repo_sidebar": {"/srv/private/.git": ["branches"]},
            "repo_requests": {"/srv/private/.git": {"text": "secret"}},
            "hosting_api": true, "download_avatars": true, "check_updates": false,
            "zoom": 1.2}}"#;
        let back = import(&here, text).unwrap().settings;
        assert_eq!(back.gitlab_hosts, "gitlab.internal");
        assert_eq!(back.editor, "code --wait %f");
        assert_eq!(back.terminal, "");
        assert_eq!(back.file_manager, "");
        assert_eq!(back.clone_dir, here.clone_dir);
        assert!(back.repos.is_empty() && back.repo_diff.is_empty());
        assert!(back.repo_sidebar.is_empty() && back.repo_requests.is_empty());
        // A network feature switched off here stays off; one the file switches off goes off.
        assert!(!back.hosting_api && !back.download_avatars && !back.check_updates);
        assert!(
            (back.zoom - 1.2).abs() < f32::EPSILON,
            "preferences still come in"
        );
    }

    fn file_with(settings: &str) -> String {
        format!(r#"{{"format": "canopy-settings", "version": 1, "settings": {settings}}}"#)
    }

    #[test]
    fn a_file_cannot_turn_on_tag_pruning() {
        let off = Settings::default();
        let on = Settings {
            prune_tags: true,
            ..Settings::default()
        };
        let imported = import(
            &off,
            &file_with(r#"{"prune_tags": true, "follow_git_config": false}"#),
        )
        .unwrap();
        assert!(
            !imported.settings.prune_tags,
            "a file must not turn on deleting local tags"
        );
        assert!(
            imported.kept.contains(&"tag pruning"),
            "{:?}",
            imported.kept
        );
        // A file may switch it off, and a switch already on stays on.
        assert!(
            !import(&on, &file_with(r#"{"prune_tags": false}"#))
                .unwrap()
                .settings
                .prune_tags
        );
        let same = import(&on, &file_with(r#"{"prune_tags": true}"#)).unwrap();
        assert!(same.settings.prune_tags && same.kept.is_empty());
    }

    #[test]
    fn a_file_cannot_fetch_more_often_or_turn_on_beta_builds() {
        let off = Settings::default();
        let every_ten = Settings {
            auto_fetch_minutes: 10,
            ..Settings::default()
        };
        let imported = import(
            &off,
            &file_with(r#"{"auto_fetch_minutes": 1, "include_prereleases": true}"#),
        )
        .unwrap();
        assert_eq!(
            imported.settings.auto_fetch_minutes, 0,
            "fetching stays off"
        );
        assert!(
            !imported.settings.include_prereleases,
            "beta builds stay off"
        );
        assert!(
            imported.kept.contains(&"automatic fetching") && imported.kept.contains(&"beta builds"),
            "{:?}",
            imported.kept
        );
        let faster = import(&every_ten, &file_with(r#"{"auto_fetch_minutes": 1}"#)).unwrap();
        assert_eq!(
            faster.settings.auto_fetch_minutes, 10,
            "no more often than now"
        );
        // Less often, or off, comes in.
        for minutes in [30, 0] {
            let fewer = import(
                &every_ten,
                &file_with(&format!(r#"{{"auto_fetch_minutes": {minutes}}}"#)),
            )
            .unwrap();
            assert_eq!(fewer.settings.auto_fetch_minutes, minutes);
            assert!(fewer.kept.is_empty(), "{:?}", fewer.kept);
        }
    }

    #[test]
    fn a_file_cannot_stop_following_the_git_configuration() {
        let follows = Settings::default();
        assert!(follows.follow_git_config);
        let imported = import(&follows, &file_with(r#"{"follow_git_config": false}"#)).unwrap();
        assert!(imported.settings.follow_git_config);
        assert!(imported.kept.contains(&"following the git configuration"));
        let apart = Settings {
            follow_git_config: false,
            ..Settings::default()
        };
        let back = import(&apart, &file_with(r#"{"follow_git_config": true}"#)).unwrap();
        assert!(
            back.settings.follow_git_config,
            "a file may hand decisions back to git"
        );
    }

    #[test]
    fn a_file_cannot_choose_where_a_push_goes() {
        let simple = Settings::default();
        for mode in ["Current", "Upstream"] {
            let imported = import(
                &simple,
                &file_with(&format!(r#"{{"push_default": "{mode}"}}"#)),
            )
            .unwrap();
            assert_eq!(imported.settings.push_default, PushMode::Simple, "{mode}");
            assert!(imported.kept.contains(&"where a push goes"));
        }
        let current = Settings {
            push_default: PushMode::Current,
            ..Settings::default()
        };
        let back = import(&current, &file_with(r#"{"push_default": "Upstream"}"#)).unwrap();
        assert_eq!(back.settings.push_default, PushMode::Current);
        let back = import(&current, &file_with(r#"{"push_default": "Simple"}"#)).unwrap();
        assert_eq!(back.settings.push_default, PushMode::Simple);
    }

    #[test]
    fn a_file_cannot_let_settings_write_the_git_configuration() {
        let off = Settings::default();
        let imported = import(&off, &file_with(r#"{"edit_git_config": true}"#)).unwrap();
        assert!(!imported.settings.edit_git_config);
        assert!(imported.kept.contains(&"editing your git config"));
        let on = Settings {
            edit_git_config: true,
            ..Settings::default()
        };
        let back = import(&on, &file_with(r#"{"edit_git_config": false}"#)).unwrap();
        assert!(!back.settings.edit_git_config, "a file may turn it off");
    }

    #[test]
    fn a_file_cannot_have_discard_delete_untracked_files_in_submodules() {
        let keep = Settings::default();
        let imported = import(
            &keep,
            &file_with(r#"{"discard_untracked_in_submodules": true}"#),
        )
        .unwrap();
        assert!(!imported.settings.discard_untracked_in_submodules);
        assert!(
            imported
                .kept
                .contains(&"deleting untracked files in submodules")
        );
    }

    #[test]
    fn a_file_still_sets_the_harmless_git_choices() {
        let here = Settings::default();
        let imported = import(
            &here,
            &file_with(
                r#"{"prune_on_fetch": false, "autostash": false, "follow_tags": false,
                    "remove_gone_branches": false, "pull_mode": "Rebase",
                    "initial_branch": "trunk"}"#,
            ),
        )
        .unwrap();
        let s = imported.settings;
        assert!(!s.prune_on_fetch && !s.autostash && !s.follow_tags);
        assert!(!s.remove_gone_branches);
        assert_eq!(s.initial_branch, "trunk");
        assert!(imported.kept.is_empty(), "{:?}", imported.kept);
    }

    #[test]
    fn an_export_leaves_out_what_belongs_to_this_computer() {
        let mut s = customised();
        s.gitlab_hosts = "gitlab.internal".to_owned();
        s.terminal = "kitty".to_owned();
        s.repos.insert(
            PathBuf::from("/home/ada/secret-project"),
            crate::settings::RepoSettings {
                display_name: Some("Secret".to_owned()),
                ..Default::default()
            },
        );
        s.repo_diff.insert(PathBuf::from("/home/ada/other"), s.diff);
        s.repo_sidebar.insert(
            PathBuf::from("/private/repository/.git"),
            std::collections::BTreeSet::new(),
        );
        s.repo_requests.insert(
            PathBuf::from("/private/repository/.git"),
            crate::request_view::RequestView {
                text: "secret-project".into(),
                ..Default::default()
            },
        );
        let text = export(&s).unwrap();
        for key in [
            "secret-project",
            "/home/ada",
            "gitlab_hosts",
            "\"editor\"",
            "\"terminal\"",
            "\"repos\"",
            "\"repo_diff\"",
            "\"repo_sidebar\"",
            "\"repo_requests\"",
        ] {
            assert!(!text.contains(key), "{key} must not be exported:\n{text}");
        }
    }

    #[test]
    fn a_bad_file_is_refused_with_the_reason() {
        let s = Settings::default();
        let refused = |text: &str| import(&s, text).unwrap_err();
        assert!(refused("not json").contains("not a settings file"));
        assert!(refused("[1, 2]").contains("not a JSON object"));
        assert!(
            refused(r#"{"format": "other", "version": 1, "settings": {}}"#)
                .contains("not a Canopy settings file")
        );
        assert!(refused(r#"{"format": "canopy-settings", "settings": {}}"#).contains("no version"));
        assert!(
            refused(r#"{"format": "canopy-settings", "version": 9, "settings": {}}"#)
                .contains("newer Canopy")
        );
        assert!(
            refused(r#"{"format": "canopy-settings", "version": 1}"#).contains("no \"settings\"")
        );
        let wrong =
            refused(r#"{"format": "canopy-settings", "version": 1, "settings": {"zoom": "big"}}"#);
        assert!(wrong.contains("cannot use"), "{wrong}");
        // Out-of-range values are brought into range, as when Canopy loads its own settings.
        let big = import(
            &s,
            r#"{"format": "canopy-settings", "version": 1, "settings": {"zoom": 9.0}}"#,
        )
        .unwrap()
        .settings;
        assert!(big.zoom <= 1.6);
    }

    #[test]
    fn reset_keeps_consent_trust_and_memory() {
        let s = customised();
        let back = reset(&s).unwrap();
        assert_eq!(back.theme, Settings::default().theme);
        assert_eq!(back.pull_mode, Settings::default().pull_mode);
        assert_eq!(back.editor, "");
        assert_eq!(back.accepted_terms, "2026-10-03");
        assert_eq!(back.crash_reports, Some(false));
        assert_eq!(back.trusted_folders, s.trusted_folders);
        assert_eq!(back.recent, s.recent);
    }

    #[test]
    fn a_large_file_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        std::fs::write(&path, vec![b' '; usize::try_from(MAX_BYTES).unwrap() + 1]).unwrap();
        assert!(matches!(read(path), Outcome::Failed(m) if m.contains("too large")));
    }

    /// A device reports no size, so only a bounded read stops it.
    #[cfg(unix)]
    #[test]
    fn an_endless_file_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
        let outcome = read(path);
        assert!(
            matches!(&outcome, Outcome::Failed(m) if m.contains("not a file")),
            "{outcome:?}"
        );
    }

    /// A file that grows after its size was checked is cut off at the limit.
    #[test]
    fn reading_stops_at_the_limit() {
        let endless = std::io::repeat(b' ');
        assert!(matches!(read_text(endless), Err(m) if m.contains("too large")));
        assert_eq!(read_text(&b"{}"[..]).as_deref(), Ok("{}"));
    }
}
