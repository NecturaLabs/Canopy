//! The programs this computer would use for what Canopy hands off: the editor, the terminal and
//! the file manager. Settings shows them when nothing is chosen, and the open actions run them.
//!
//! Detection reads environment variables, desktop entries and the desktop's default handlers, so
//! it runs on its own thread: once at startup and again whenever Settings opens (an editor
//! installed meanwhile shows up). The result is cached for the run.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::platform::{program_name, split_command};

/// What a program is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Editor,
    Terminal,
    FileManager,
}

/// Where the system's choice comes from: the quiet tag beside it in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// An environment variable (`VISUAL`, `EDITOR`, `TERMINAL`).
    Env(&'static str),
    /// The desktop's default application (Linux `xdg-mime`, `xdg-terminal-exec`).
    Desktop,
    /// The first of the usual programs found on this computer.
    Found,
    /// What the platform itself opens (TextEdit's default, Notepad, Finder, Explorer).
    #[cfg_attr(
        not(any(target_os = "macos", target_os = "windows")),
        expect(dead_code, reason = "Linux names the desktop's choices")
    )]
    Platform,
}

impl Origin {
    pub fn tag(self) -> String {
        match self {
            Self::Env(var) => format!("from ${var}"),
            Self::Desktop | Self::Platform => "System default".to_owned(),
            Self::Found => "Found on this computer".to_owned(),
        }
    }
}

/// The program the system would use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemChoice {
    /// What the user sees: an application's name, or the command.
    pub name: String,
    /// The command to run, in Settings' command syntax ([`crate::platform`]). `None` leaves it to
    /// the platform's own opener.
    pub command: Option<String>,
    pub origin: Origin,
}

/// Everything detected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Apps {
    pub editor: Option<SystemChoice>,
    pub terminal: Option<SystemChoice>,
    pub file_manager: Option<SystemChoice>,
}

impl Apps {
    pub fn system(&self, kind: Kind) -> Option<&SystemChoice> {
        match kind {
            Kind::Editor => self.editor.as_ref(),
            Kind::Terminal => self.terminal.as_ref(),
            Kind::FileManager => self.file_manager.as_ref(),
        }
    }
}

struct Cache {
    apps: Option<Arc<Apps>>,
    running: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    apps: None,
    running: false,
});

/// Starts a detection unless one is running; `ctx` is woken when it ends.
pub fn refresh(ctx: Option<&egui::Context>) {
    {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.running {
            return;
        }
        cache.running = true;
    }
    let ctx = ctx.cloned();
    let spawned = std::thread::Builder::new()
        .name("system-apps".into())
        .spawn(move || {
            let apps = Arc::new(detect());
            let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
            cache.apps = Some(apps);
            cache.running = false;
            drop(cache);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start detecting the editor and terminal");
        CACHE.lock().unwrap_or_else(PoisonError::into_inner).running = false;
    }
}

/// The last detection, once one finished.
pub fn get() -> Option<Arc<Apps>> {
    CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .apps
        .clone()
}

/// The command an open action runs: the one chosen in Settings, else the system's, else empty
/// (the platform's own opener; also before the first detection ends).
pub fn command_for(configured: &str, kind: Kind) -> String {
    let configured = configured.trim();
    if !configured.is_empty() {
        return configured.to_owned();
    }
    get()
        .and_then(|apps| apps.system(kind).and_then(|c| c.command.clone()))
        .unwrap_or_default()
}

/// Detects for this computer (blocking: never on the UI thread).
fn detect() -> Apps {
    #[cfg(target_os = "linux")]
    {
        linux::detect(&linux::Env::current())
    }
    #[cfg(target_os = "macos")]
    {
        macos::detect()
    }
    #[cfg(target_os = "windows")]
    {
        windows::detect()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Apps::default()
    }
}

/// Linux terminals tried in order when neither `$TERMINAL` nor `xdg-terminal-exec` is there.
#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(dead_code, reason = "the list is Linux's")
)]
pub const LINUX_TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "kgx",
    "gnome-terminal",
    "konsole",
    "alacritty",
    "kitty",
    "foot",
    "wezterm",
    "ghostty",
    "xterm",
];

/// Command-line names of graphical editors, for `$VISUAL`/`$EDITOR` where no desktop entry
/// says what the program is. Anything else there is taken to be a terminal editor, which
/// cannot open a window of its own.
const GUI_EDITORS: &[&str] = &[
    "code",
    "codium",
    "code-insiders",
    "cursor",
    "zed",
    "zeditor",
    "subl",
    "sublime_text",
    "gedit",
    "gnome-text-editor",
    "kate",
    "kwrite",
    "mousepad",
    "pluma",
    "xed",
    "geany",
    "gvim",
    "mvim",
    "bbedit",
    "mate",
    "notepad++",
    "notepad",
];

/// The program a command runs, on `PATH` or by its path.
fn on_path(program: &str, path: &[PathBuf]) -> bool {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file();
    }
    path.iter().any(|dir| {
        let full = dir.join(program);
        full.is_file() || (cfg!(windows) && full.with_extension("exe").is_file())
    })
}

/// `PATH`, split.
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// `$VISUAL`, then `$EDITOR`, when one names a graphical editor that is installed: `known` says
/// whether a program is one (from desktop entries; the [`GUI_EDITORS`] list otherwise).
fn editor_from_env(
    visual: Option<&str>,
    editor: Option<&str>,
    path: &[PathBuf],
    known: &dyn Fn(&str) -> Option<String>,
) -> Option<SystemChoice> {
    [("VISUAL", visual), ("EDITOR", editor)]
        .into_iter()
        .find_map(|(var, value)| {
            let value = value?.trim();
            let (program, _) = split_command(value)?;
            if !on_path(&program, path) {
                return None;
            }
            let name = known(&program_name(&program))?;
            Some(SystemChoice {
                name,
                command: Some(value.to_owned()),
                origin: Origin::Env(var),
            })
        })
}

/// What the [`GUI_EDITORS`] list knows about `program`: its own name.
fn listed_gui_editor(program: &str) -> Option<String> {
    GUI_EDITORS
        .contains(&program.to_ascii_lowercase().as_str())
        .then(|| program.to_owned())
}

#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(dead_code, reason = "desktop entries are Linux's")
)]
mod linux {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{Apps, Origin, SystemChoice, editor_from_env, listed_gui_editor, on_path};
    use crate::platform::{program_name, quote};

    /// The desktop entry id that opens a MIME type.
    pub(super) type DefaultFor = Box<dyn Fn(&str) -> Option<String>>;

    /// What detection reads from the computer, so tests can supply their own.
    pub(super) struct Env {
        pub visual: Option<String>,
        pub editor: Option<String>,
        pub terminal: Option<String>,
        pub path: Vec<PathBuf>,
        /// Data folders, most important first (`$XDG_DATA_HOME`, then `$XDG_DATA_DIRS`).
        pub data_dirs: Vec<PathBuf>,
        /// Configuration folders, most important first (`$XDG_CONFIG_HOME`, `$XDG_CONFIG_DIRS`).
        pub config_dirs: Vec<PathBuf>,
        /// The desktop entry id that opens a MIME type (`xdg-mime query default`).
        pub default_for: DefaultFor,
    }

    impl Env {
        pub(super) fn current() -> Self {
            let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
            let home = crate::settings::home_dir();
            let dirs = |own: &str, own_default: &str, system: &str, system_default: &str| {
                let mut v = vec![var(own).map_or_else(|| home.join(own_default), PathBuf::from)];
                v.extend(
                    var(system)
                        .unwrap_or_else(|| system_default.to_owned())
                        .split(':')
                        .filter(|d| !d.is_empty())
                        .map(PathBuf::from),
                );
                v
            };
            Self {
                visual: var("VISUAL"),
                editor: var("EDITOR"),
                terminal: var("TERMINAL"),
                path: super::path_dirs(),
                data_dirs: dirs(
                    "XDG_DATA_HOME",
                    ".local/share",
                    "XDG_DATA_DIRS",
                    "/usr/local/share:/usr/share",
                ),
                config_dirs: dirs("XDG_CONFIG_HOME", ".config", "XDG_CONFIG_DIRS", "/etc/xdg"),
                default_for: Box::new(xdg_mime_default),
            }
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "asks xdg-mime for the desktop's default application, never git"
    )]
    fn xdg_mime_default(mime: &str) -> Option<String> {
        let mut cmd = Command::new("xdg-mime");
        cmd.args(["query", "default", mime]);
        crate::platform::output_within(cmd, crate::platform::QUERY_TIMEOUT)
            .map(|out| out.trim().to_owned())
            .filter(|id| !id.is_empty())
    }

    /// A desktop entry's fields Canopy uses.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub(super) struct Entry {
        pub id: String,
        pub name: String,
        pub exec: Vec<String>,
        pub try_exec: Option<String>,
        pub terminal: bool,
        /// `NoDisplay` or `Hidden`.
        pub hidden: bool,
        pub categories: Vec<String>,
        pub mime: Vec<String>,
    }

    impl Entry {
        /// Whether its program is on this computer.
        fn installed(&self, path: &[PathBuf]) -> bool {
            self.try_exec
                .as_deref()
                .or_else(|| self.exec.first().map(String::as_str))
                .is_some_and(|p| on_path(p, path))
        }

        /// The entry as a command: `file_code` (`%f` or `%d`) where the entry takes files or
        /// URLs, other field codes and Flatpak's `@@` markers left out.
        pub(super) fn command(&self, file_code: &str) -> String {
            let mut words = Vec::new();
            for arg in &self.exec {
                match arg.as_str() {
                    "%f" | "%F" | "%u" | "%U" => words.push(file_code.to_owned()),
                    "@@" | "@@u" | "@@f" | "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N"
                    | "%v" | "%m" => {}
                    _ => words.push(quote(&arg.replace("%%", "%"))),
                }
            }
            words.join(" ")
        }
    }

    /// The desktop entry string escapes (`\s`, `\n`, `\t`, `\r`, `\\`).
    fn unescape(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        out
    }

    /// An `Exec` value's arguments: separated by spaces, double quotes around an argument with
    /// `\"`, `` \` ``, `\$` and `\\` escaped inside. `None` when a quote is not closed.
    pub(super) fn parse_exec(value: &str) -> Option<Vec<String>> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut started = false;
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    started = true;
                    loop {
                        match chars.next()? {
                            '"' => break,
                            '\\' => current.push(chars.next()?),
                            other => current.push(other),
                        }
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                other => {
                    started = true;
                    current.push(other);
                }
            }
        }
        if started {
            args.push(current);
        }
        Some(args)
    }

    /// The `[Desktop Entry]` group of a desktop file, when it is an application.
    pub(super) fn parse_entry(id: &str, text: &str) -> Option<Entry> {
        let mut entry = Entry {
            id: id.to_owned(),
            ..Entry::default()
        };
        let mut in_group = false;
        let mut is_app = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_group = line == "[Desktop Entry]";
                continue;
            }
            if !in_group || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = unescape(value.trim());
            let list = || {
                value
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            match key.trim() {
                "Type" => is_app = value == "Application",
                "Name" => entry.name.clone_from(&value),
                "Exec" => entry.exec = parse_exec(&value)?,
                "TryExec" => entry.try_exec = Some(value.clone()),
                "Terminal" => entry.terminal = value == "true",
                "NoDisplay" | "Hidden" => entry.hidden |= value == "true",
                "Categories" => entry.categories = list(),
                "MimeType" => entry.mime = list(),
                _ => {}
            }
        }
        (is_app && !entry.name.is_empty() && !entry.exec.is_empty()).then_some(entry)
    }

    /// Every application entry under the data folders' `applications`, by id (a file in a
    /// subfolder gets `folder-name.desktop`); the first folder's entry wins.
    pub(super) fn entries(data_dirs: &[PathBuf]) -> HashMap<String, Entry> {
        let mut found = HashMap::new();
        for dir in data_dirs {
            collect(&dir.join("applications"), "", 0, &mut found);
        }
        found
    }

    fn collect(dir: &Path, prefix: &str, depth: usize, found: &mut HashMap<String, Entry>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for item in read.flatten() {
            let path = item.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.is_dir() {
                if depth < 3 {
                    collect(&path, &format!("{prefix}{name}-"), depth + 1, found);
                }
                continue;
            }
            let Some(stem) = name.strip_suffix(".desktop") else {
                continue;
            };
            let id = format!("{prefix}{stem}.desktop");
            if found.contains_key(&id) {
                continue;
            }
            if let Some(entry) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| parse_entry(&id, &text))
            {
                found.insert(id, entry);
            }
        }
    }

    /// The terminal `xdg-terminal-exec` starts: the first entry of `xdg-terminals.list` that is
    /// installed. Its name, for Settings.
    fn xdg_terminal_name(env: &Env, entries: &HashMap<String, Entry>) -> Option<String> {
        env.config_dirs.iter().find_map(|dir| {
            let list = std::fs::read_to_string(dir.join("xdg-terminals.list")).ok()?;
            list.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .filter_map(|id| entries.get(id.trim_start_matches(['+', '-'])))
                .find(|e| e.installed(&env.path))
                .map(|e| e.name.clone())
        })
    }

    /// An editor entry as a command, run through `xdg-terminal-exec` when it is a terminal
    /// program. `None` when it needs a terminal and there is no launcher for one.
    fn editor_command(entry: &Entry, launcher: bool) -> Option<(String, String)> {
        let command = entry.command("%f");
        if entry.terminal {
            launcher.then(|| {
                (
                    format!("{} (in terminal)", entry.name),
                    format!("xdg-terminal-exec {command}"),
                )
            })
        } else {
            Some((entry.name.clone(), command))
        }
    }

    pub(super) fn detect(env: &Env) -> Apps {
        let entries = entries(&env.data_dirs);
        let launcher = on_path("xdg-terminal-exec", &env.path);
        let by_program = |program: &str| {
            let mut matching: Vec<&Entry> = entries
                .values()
                .filter(|e| e.exec.first().is_some_and(|p| program_name(p) == program))
                .collect();
            matching.sort_by_key(|e| (e.hidden, e.id.clone()));
            matching.first().copied()
        };

        let known = |program: &str| match by_program(program) {
            Some(entry) if entry.terminal => None,
            Some(entry) => Some(entry.name.clone()),
            None => listed_gui_editor(program),
        };
        let editor = editor_from_env(
            env.visual.as_deref(),
            env.editor.as_deref(),
            &env.path,
            &known,
        )
        .or_else(|| {
            let entry = entries.get(&(env.default_for)("text/plain")?)?;
            let (name, command) = editor_command(entry, launcher)?;
            Some(SystemChoice {
                name,
                command: Some(command),
                origin: Origin::Desktop,
            })
        });

        let terminal_named = |command: &str, origin: Origin| {
            let (program, _) = crate::platform::split_command(command)?;
            if !on_path(&program, &env.path) {
                return None;
            }
            let name = if program_name(&program) == "xdg-terminal-exec" {
                xdg_terminal_name(env, &entries)
                    .unwrap_or_else(|| "Desktop terminal (xdg-terminal-exec)".to_owned())
            } else {
                by_program(&program_name(&program))
                    .map_or_else(|| program.clone(), |e| e.name.clone())
            };
            Some(SystemChoice {
                name,
                command: Some(command.to_owned()),
                origin,
            })
        };
        let terminal = env
            .terminal
            .as_deref()
            .and_then(|t| terminal_named(t.trim(), Origin::Env("TERMINAL")))
            .or_else(|| terminal_named("xdg-terminal-exec", Origin::Desktop))
            .or_else(|| {
                super::LINUX_TERMINALS
                    .iter()
                    .find_map(|t| terminal_named(t, Origin::Found))
            });

        let file_manager = (env.default_for)("inode/directory")
            .and_then(|id| entries.get(&id))
            .filter(|e| !e.terminal && e.installed(&env.path))
            .map(|e| SystemChoice {
                name: e.name.clone(),
                command: Some(e.command("%d")),
                origin: Origin::Desktop,
            });

        Apps {
            editor,
            terminal,
            file_manager,
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{Apps, Origin, SystemChoice, editor_from_env, listed_gui_editor, path_dirs};

    pub(super) fn detect() -> Apps {
        let var = |name: &str| std::env::var(name).ok();
        let editor = editor_from_env(
            var("VISUAL").as_deref(),
            var("EDITOR").as_deref(),
            &path_dirs(),
            &listed_gui_editor,
        )
        .or(Some(SystemChoice {
            name: "Default text editor".to_owned(),
            command: None,
            origin: Origin::Platform,
        }));
        Apps {
            editor,
            terminal: Some(SystemChoice {
                name: "Terminal".to_owned(),
                command: None,
                origin: Origin::Platform,
            }),
            file_manager: Some(SystemChoice {
                name: "Finder".to_owned(),
                command: None,
                origin: Origin::Platform,
            }),
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::{
        Apps, Origin, SystemChoice, editor_from_env, listed_gui_editor, on_path, path_dirs,
    };

    pub(super) fn detect() -> Apps {
        let var = |name: &str| std::env::var(name).ok();
        let path = path_dirs();
        let editor = editor_from_env(
            var("VISUAL").as_deref(),
            var("EDITOR").as_deref(),
            &path,
            &listed_gui_editor,
        )
        .or(Some(SystemChoice {
            name: "Notepad".to_owned(),
            command: None,
            origin: Origin::Platform,
        }));
        let terminal = if on_path("wt.exe", &path) {
            SystemChoice {
                name: "Windows Terminal".to_owned(),
                command: Some("wt -d %d".to_owned()),
                origin: Origin::Platform,
            }
        } else {
            SystemChoice {
                name: "Command Prompt".to_owned(),
                command: None,
                origin: Origin::Platform,
            }
        };
        Apps {
            editor,
            terminal: Some(terminal),
            file_manager: Some(SystemChoice {
                name: "File Explorer".to_owned(),
                command: None,
                origin: Origin::Platform,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::linux::{Env, detect, entries, parse_entry, parse_exec};
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn program(dir: &Path, name: &str) {
        write(dir, name, "#!/bin/sh\n");
    }

    fn env(root: &Path, defaults: &[(&str, &str)]) -> Env {
        let defaults: HashMap<String, String> = defaults
            .iter()
            .map(|(m, id)| ((*m).to_owned(), (*id).to_owned()))
            .collect();
        Env {
            visual: None,
            editor: None,
            terminal: None,
            path: vec![root.join("bin")],
            data_dirs: vec![root.join("home-data"), root.join("usr-share")],
            config_dirs: vec![root.join("config")],
            default_for: Box::new(move |mime| defaults.get(mime).cloned()),
        }
    }

    const NVIM: &str = "[Desktop Entry]\nType=Application\nName=Neovim\nExec=nvim %F\nTerminal=true\nCategories=Utility;TextEditor;\nMimeType=text/plain;\n";
    const CODE: &str = "[Desktop Entry]\nType=Application\nName=Visual Studio Code\nExec=/opt/code/bin/code --unity-launch %F\nCategories=Development;IDE;\nMimeType=text/plain;inode/directory;\n[Desktop Action new-empty-window]\nName=New Empty Window\nExec=code --new-window\n";
    const FILES: &str = "[Desktop Entry]\nType=Application\nName=Files\nExec=nautilus --new-window %U\nCategories=GNOME;Utility;Core;FileManager;\n";
    const KITTY: &str = "[Desktop Entry]\nType=Application\nName=kitty\nExec=kitty\nCategories=System;TerminalEmulator;\n";

    #[test]
    fn exec_lines_are_split_as_the_desktop_entry_spec_says() {
        assert_eq!(
            parse_exec("code --unity-launch %F").unwrap(),
            ["code", "--unity-launch", "%F"]
        );
        assert_eq!(
            parse_exec(r#""/opt/My Editor/edit" --title "say \"hi\"" %f"#).unwrap(),
            ["/opt/My Editor/edit", "--title", "say \"hi\"", "%f"]
        );
        assert_eq!(parse_exec(r#"edit "unclosed"#), None);
        let entry = parse_entry("code.desktop", CODE).unwrap();
        assert_eq!(
            entry.name, "Visual Studio Code",
            "the action group is not the entry"
        );
        assert_eq!(entry.command("%f"), "/opt/code/bin/code --unity-launch %f");
        let flatpak = parse_entry(
            "org.gnome.TextEditor.desktop",
            "[Desktop Entry]\nType=Application\nName=Text Editor\nExec=/usr/bin/flatpak run --file-forwarding org.gnome.TextEditor @@ %F @@\n",
        )
        .unwrap();
        assert_eq!(
            flatpak.command("%f"),
            "/usr/bin/flatpak run --file-forwarding org.gnome.TextEditor %f"
        );
        assert_eq!(
            parse_entry("x.desktop", "[Desktop Entry]\nType=Link\nName=X\nExec=x\n"),
            None
        );
    }

    #[test]
    fn the_first_data_folder_wins_and_subfolders_name_entries() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("home-data/applications"),
            "kitty.desktop",
            KITTY,
        );
        write(
            &dir.path().join("usr-share/applications"),
            "kitty.desktop",
            &KITTY.replace("Name=kitty", "Name=Old kitty"),
        );
        write(
            &dir.path().join("usr-share/applications/kde"),
            "files.desktop",
            FILES,
        );
        let found = entries(&[dir.path().join("home-data"), dir.path().join("usr-share")]);
        assert_eq!(found["kitty.desktop"].name, "kitty");
        assert_eq!(found["kde-files.desktop"].name, "Files");
    }

    #[test]
    fn the_system_choices_follow_the_environment_then_the_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let apps = root.join("usr-share/applications");
        write(&apps, "nvim.desktop", NVIM);
        write(&apps, "code.desktop", CODE);
        write(&apps, "org.gnome.Nautilus.desktop", FILES);
        write(&apps, "kitty.desktop", KITTY);
        write(
            &root.join("config"),
            "xdg-terminals.list",
            "# preferred\nkitty.desktop\n",
        );
        for p in [
            "nvim",
            "nautilus",
            "kitty",
            "xdg-terminal-exec",
            "omarchy-launch-editor",
        ] {
            program(&root.join("bin"), p);
        }
        program(&root.join("opt/code/bin"), "code");
        let defaults = [
            ("text/plain", "nvim.desktop"),
            ("inode/directory", "org.gnome.Nautilus.desktop"),
        ];

        // $EDITOR names a script nothing says is graphical: the desktop's text editor, which
        // runs in a terminal, opens through xdg-terminal-exec.
        let mut e = env(root, &defaults);
        e.editor = Some("omarchy-launch-editor --inline".to_owned());
        e.terminal = Some("xdg-terminal-exec".to_owned());
        let found = detect(&e);
        assert_eq!(
            found.editor,
            Some(SystemChoice {
                name: "Neovim (in terminal)".to_owned(),
                command: Some("xdg-terminal-exec nvim %f".to_owned()),
                origin: Origin::Desktop,
            })
        );
        assert_eq!(
            found.terminal,
            Some(SystemChoice {
                name: "kitty".to_owned(),
                command: Some("xdg-terminal-exec".to_owned()),
                origin: Origin::Env("TERMINAL"),
            })
        );
        assert_eq!(
            found.file_manager,
            Some(SystemChoice {
                name: "Files".to_owned(),
                command: Some("nautilus --new-window %d".to_owned()),
                origin: Origin::Desktop,
            })
        );

        // A graphical editor in $VISUAL wins, named by its desktop entry.
        let mut e = env(root, &defaults);
        e.visual = Some(format!(
            "{} --wait",
            root.join("opt/code/bin/code").display()
        ));
        e.editor = Some("nvim".to_owned());
        let found = detect(&e);
        let editor = found.editor.unwrap();
        assert_eq!(
            (editor.name.as_str(), editor.origin),
            ("Visual Studio Code", Origin::Env("VISUAL"))
        );
        assert_eq!(editor.command, e.visual);
        // No $TERMINAL: xdg-terminal-exec is the desktop's.
        assert_eq!(found.terminal.unwrap().origin, Origin::Desktop);

        // Nothing configured, no launcher: a terminal editor cannot be the default, and the
        // terminal is the first one found.
        std::fs::remove_file(root.join("bin/xdg-terminal-exec")).unwrap();
        let found = detect(&env(root, &defaults));
        assert_eq!(found.editor, None);
        let terminal = found.terminal.unwrap();
        assert_eq!(
            (
                terminal.name.as_str(),
                terminal.command.as_deref(),
                terminal.origin
            ),
            ("kitty", Some("kitty"), Origin::Found)
        );
        // A $TERMINAL that is not installed is passed over.
        let mut e = env(root, &defaults);
        e.terminal = Some("wezterm".to_owned());
        assert_eq!(detect(&e).terminal.unwrap().origin, Origin::Found);
    }

    #[test]
    fn a_chosen_command_comes_before_the_systems() {
        assert_eq!(command_for("  zed  ", Kind::Editor), "zed");
    }
}
