//! Settings as a file: export to and import from a file the user chooses, and resetting every
//! preference.
//!
//! Only preferences travel. What the user agreed to (the Terms, the Privacy Policy, the crash
//! report answer), the folders trusted to run a repository's configuration and Canopy's memory
//! of the session ([`NOT_PREFERENCES`]) never leave the machine and are never taken from a file:
//! a file must not be able to give consent or trust. Tokens are not settings at all
//! (`hosting.rs` keeps them), so they are never in the file.
//!
//! The file is JSON: `{"format": "canopy-settings", "version": 1, "exported": "<UTC>",
//! "settings": {…}}`. An import is checked as a whole before anything changes: a file that is
//! not Canopy's, comes from a newer format, or has a value of the wrong kind is refused with a
//! message saying why.

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
    "identity_asked",
    "last_update_check",
    "skipped_version",
    "last_seen_version",
    "recent",
    "open_tabs",
    "seen_intro",
    "last_crash_seen",
    "dark",
    "reduce_motion",
];

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
    let file = serde_json::json!({
        "format": FORMAT,
        "version": VERSION,
        "exported": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "settings": prefs,
    });
    serde_json::to_string_pretty(&file).map_err(|e| format!("Could not write the settings: {e}"))
}

/// `current` with the preferences from a settings file; nothing else changes. Preferences the
/// file does not name keep their current value.
pub fn import(current: &Settings, text: &str) -> Result<Settings, String> {
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
    for (key, value) in prefs {
        if NOT_PREFERENCES.contains(&key.as_str()) {
            continue;
        }
        merged.insert(key, value);
    }
    let mut imported: Settings = serde_json::from_value(Value::Object(merged))
        .map_err(|e| format!("The settings file has a value Canopy cannot use: {e}."))?;
    imported.migrate();
    Ok(imported)
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

/// A settings file's text, refusing one too large to be settings.
fn read(path: PathBuf) -> Outcome {
    let size = match std::fs::metadata(&path) {
        Ok(m) => m.len(),
        Err(e) => return Outcome::Failed(format!("Could not read {}: {e}", path.display())),
    };
    if size > MAX_BYTES {
        return Outcome::Failed(format!(
            "{} is {} KB, too large to be a settings file.",
            path.display(),
            size / 1024
        ));
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => Outcome::Read(path, text),
        Err(e) => Outcome::Failed(format!("Could not read {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CloneProtocol, PullMode};
    use crate::ui::theme::ThemeId;

    fn customised() -> Settings {
        Settings {
            theme: ThemeId::Daylight,
            pull_mode: PullMode::Rebase,
            clone_protocol: CloneProtocol::Ssh,
            editor: "zed --wait %f".to_owned(),
            accepted_terms: "2026-10-03".to_owned(),
            accepted_privacy: "2026-10-03".to_owned(),
            crash_reports: Some(false),
            trusted_folders: vec![PathBuf::from("/srv/trusted")],
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
            "recent",
            "/srv/",
        ] {
            assert!(!text.contains(key), "{key} must not be exported:\n{text}");
        }
        assert!(text.contains("\"format\": \"canopy-settings\""), "{text}");
        let fresh = Settings::default();
        let back = import(&fresh, &text).unwrap();
        assert_eq!(back.theme, ThemeId::Daylight);
        assert_eq!(back.pull_mode, PullMode::Rebase);
        assert_eq!(back.editor, "zed --wait %f");
        // Consent and trust stay what they were on this machine.
        assert_eq!(back.accepted_terms, fresh.accepted_terms);
        assert_eq!(back.crash_reports, None);
        assert_eq!(back.trusted_folders, Vec::<PathBuf>::new());
    }

    #[test]
    fn a_file_cannot_give_consent_or_trust() {
        let fresh = Settings::default();
        let text = r#"{"format": "canopy-settings", "version": 1, "settings": {
            "accepted_terms": "2026-10-03", "accepted_privacy": "2026-10-03",
            "crash_reports": true, "trusted_folders": ["/"], "zoom": 1.2}}"#;
        let back = import(&fresh, text).unwrap();
        assert_eq!(back.accepted_terms, fresh.accepted_terms);
        assert_eq!(back.crash_reports, None);
        assert_eq!(back.trusted_folders, Vec::<PathBuf>::new());
        assert!((back.zoom - 1.2).abs() < f32::EPSILON);
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
        .unwrap();
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
}
