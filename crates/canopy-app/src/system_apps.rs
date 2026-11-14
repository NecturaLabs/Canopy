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

/// The program the system would use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemChoice {
    /// What the user sees: an application's name, or the command.
    pub name: String,
    /// The command to run, in Settings' command syntax ([`crate::platform`]). `None` leaves it to
    /// the platform's own opener.
    pub command: Option<String>,
    /// Which installed program it is ([`App::id`]), when it is one the list can show.
    pub id: Option<String>,
}

impl SystemChoice {
    /// Whether `app` is this program: the list names the system's choice once, as the default,
    /// however it is started (`xdg-terminal-exec` and `kitty` are one program).
    pub fn is(&self, app: &App) -> bool {
        self.id.as_deref() == Some(app.id.as_str())
    }
}

/// An installed program Settings offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub kind: Kind,
    pub name: String,
    /// What Canopy saves and runs when it is chosen.
    pub command: String,
    /// What the program is, never what it is called or how it starts: its executable on Linux,
    /// its bundle on macOS, its App Paths name on Windows. One program found twice has one id.
    pub id: String,
}

/// A well-known program that is not installed: listed after the installed ones, marked, and never
/// started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub kind: Kind,
    pub name: String,
    /// What a saved command for it starts ([`command_key`]): its command names or its bundle.
    keys: Vec<String>,
}

impl Missing {
    pub fn new<'k>(kind: Kind, name: &str, keys: impl IntoIterator<Item = &'k str>) -> Self {
        Self {
            kind,
            name: name.to_owned(),
            keys: keys.into_iter().map(str::to_owned).collect(),
        }
    }

    /// Whether the saved `command` starts this program: by its bare name (or, on macOS, `open -a`),
    /// or by a path to a program file that is gone (`program_absent`, [`Apps::is_absent`]: what an
    /// entry in the list saved, such as `/usr/share/code/code --unity-launch %F`). A path to a file
    /// that is there is the user's own program and runs as typed.
    pub fn is_started_by(&self, command: &str, program_absent: bool) -> bool {
        command_key(command).is_some_and(|(key, by_path, ignore_case)| {
            (program_absent || !by_path)
                && if ignore_case {
                    self.keys.iter().any(|k| k.eq_ignore_ascii_case(&key))
                } else {
                    self.keys.contains(&key)
                }
        })
    }
}

/// Everything detected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Apps {
    pub editor: Option<SystemChoice>,
    pub terminal: Option<SystemChoice>,
    pub file_manager: Option<SystemChoice>,
    /// Installed editors, terminals and file managers, by name.
    pub installed: Vec<App>,
    /// Well-known ones that are not installed, by name; empty for a kind the platform cannot tell
    /// reliably (Windows: App Paths misses programs on `PATH` and Explorer).
    pub missing: Vec<Missing>,
    /// The saved commands (as given to [`refresh`]) that name their program by a path to a file
    /// that was not there when detection ran; read off the UI thread, so [`launch`] and Settings
    /// need no file access.
    pub absent: Vec<String>,
}

impl Apps {
    /// Whether `command` is a saved command that names its program by a path to a file that was
    /// not there at the last detection.
    pub fn is_absent(&self, command: &str) -> bool {
        self.absent.iter().any(|c| c == command.trim())
    }

    pub fn system(&self, kind: Kind) -> Option<&SystemChoice> {
        match kind {
            Kind::Editor => self.editor.as_ref(),
            Kind::Terminal => self.terminal.as_ref(),
            Kind::FileManager => self.file_manager.as_ref(),
        }
    }

    pub fn installed(&self, kind: Kind) -> impl Iterator<Item = &App> {
        self.installed.iter().filter(move |a| a.kind == kind)
    }

    pub fn missing(&self, kind: Kind) -> impl Iterator<Item = &Missing> {
        self.missing.iter().filter(move |m| m.kind == kind)
    }
}

/// The program a command starts, as [`Missing`] names it, and whether the command names its
/// program by a path (`/opt/zed/bin/zed`). Case is significant to Unix process launches; Windows
/// executable names and macOS's `open -a Name` application names are matched case-insensitively.
fn command_key(command: &str) -> Option<(String, bool, bool)> {
    let (program, args) = split_command(command.trim())?;
    let by_path = program.contains(['/', '\\']);
    let name = program.rsplit(['/', '\\']).next().unwrap_or(&program);
    if name == "open" && args.first().is_some_and(|a| a == "-a") {
        return args
            .get(1)
            .map(|app| (app.to_ascii_lowercase(), by_path, true));
    }
    #[cfg(windows)]
    let name = {
        let lower = name.to_ascii_lowercase();
        lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
    };
    #[cfg(not(windows))]
    let name = name.to_owned();
    Some((name, by_path, cfg!(windows)))
}

/// The `saved` commands that name their program by an absolute path to a file that is not there
/// (blocking file access: never on the UI thread). A `~` path or a relative one is left out:
/// starting a program never expands `~`, and a relative path is resolved against the folder the
/// command runs in, not Canopy's, so such a command runs as typed and fails with the system's
/// own error.
fn absent_commands(saved: &[String]) -> Vec<String> {
    saved
        .iter()
        .map(|c| c.trim())
        .filter(|c| {
            split_command(c).is_some_and(|(program, _)| {
                let path = Path::new(&program);
                path.is_absolute() && !path.is_file()
            })
        })
        .map(str::to_owned)
        .collect()
}

/// `apps` by name, each program once per kind (the first by name when several entries start it).
fn sorted(mut apps: Vec<App>) -> Vec<App> {
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.command.cmp(&b.command))
    });
    let mut seen = std::collections::HashSet::new();
    apps.retain(|a| seen.insert((a.kind, a.id.clone())));
    apps
}

/// `missing` by name.
fn sorted_missing(mut missing: Vec<Missing>) -> Vec<Missing> {
    missing.sort_by_cached_key(|m| m.name.to_lowercase());
    missing
}

struct Cache {
    apps: Option<Arc<Apps>>,
    running: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    apps: None,
    running: false,
});

/// Starts a detection unless one is running; `ctx` is woken when it ends. `saved` are the commands
/// Settings holds (editor, terminal, file manager): which of them name a program file that is gone
/// is part of the result.
pub fn refresh(ctx: Option<&egui::Context>, saved: &[&str]) {
    {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.running {
            return;
        }
        cache.running = true;
    }
    let ctx = ctx.cloned();
    let saved: Vec<String> = saved.iter().map(|c| (*c).to_owned()).collect();
    let spawned = std::thread::Builder::new()
        .name("system-apps".into())
        .spawn(move || {
            let publish = |apps: &Apps, finished: bool| {
                let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
                cache.apps = Some(Arc::new(apps.clone()));
                cache.running = !finished;
                drop(cache);
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            };
            let mut apps = detect();
            // The lists are published before the saved paths are checked, so a path on a mount
            // that hangs cannot delay them. Its limit: such a thread never ends, `absent` stays
            // empty (those commands run as typed) and no later refresh starts.
            publish(&apps, false);
            apps.absent = absent_commands(&saved);
            publish(&apps, true);
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

/// What an open action runs, and why when it is not what Settings chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub command: String,
    pub fallback: Option<Fallback>,
}

/// The chosen program is no longer installed: the system's is used instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    /// The well-known program that was chosen.
    pub missing: String,
    /// The program used instead.
    pub used: String,
}

/// The command an open action runs given what `apps` detected: the one chosen in Settings,
/// else the system's, else empty (the platform's own opener; also before the first detection
/// ends). A chosen well-known program that is no longer installed is not started: the system's
/// runs, and `fallback` says so.
pub fn launch(apps: Option<&Apps>, configured: &str, kind: Kind) -> Launch {
    let configured = configured.trim();
    let system = apps.and_then(|a| a.system(kind));
    let system_command = || system.and_then(|c| c.command.clone()).unwrap_or_default();
    if configured.is_empty() {
        return Launch {
            command: system_command(),
            fallback: None,
        };
    }
    let missing = apps.and_then(|a| {
        a.missing(kind)
            .find(|m| m.is_started_by(configured, a.is_absent(configured)))
    });
    match missing {
        Some(missing) => Launch {
            command: system_command(),
            fallback: Some(Fallback {
                missing: missing.name.clone(),
                used: system.map_or_else(|| "the system's own".to_owned(), |c| c.name.clone()),
            }),
        },
        None => Launch {
            command: configured.to_owned(),
            fallback: None,
        },
    }
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

/// Linux terminals tried in order when neither `$TERMINAL` nor `xdg-terminal-exec` is there: the
/// Debian alternative, then the well-known terminals in [`KNOWN`]'s order.
#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(dead_code, reason = "the list is Linux's")
)]
pub fn linux_terminals() -> impl Iterator<Item = &'static str> {
    std::iter::once("x-terminal-emulator").chain(
        KNOWN
            .iter()
            .filter(|k| k.kind == Kind::Terminal)
            .filter_map(|k| k.commands.first().copied()),
    )
}

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

/// The file a command runs, on `PATH` or by its path.
fn find_program(program: &str, path: &[PathBuf]) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    path.iter().find_map(|dir| {
        let full = dir.join(program);
        if full.is_file() {
            return Some(full);
        }
        let exe = full.with_extension("exe");
        (cfg!(windows) && exe.is_file()).then_some(exe)
    })
}

/// Whether the program a command runs is there, on `PATH` or by its path.
fn on_path(program: &str, path: &[PathBuf]) -> bool {
    find_program(program, path).is_some()
}

/// An executable's identity ([`App::id`]): its folder resolved, so `/bin/kitty` and
/// `/usr/bin/kitty` are one, under its own file name, so a link such as snap's `/snap/bin/code`
/// stays the program it names rather than the `snap` it leads to.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", test)),
    expect(
        dead_code,
        reason = "desktop entries are Linux's, a binary on PATH macOS's"
    )
)]
fn executable_id(program: &str, path: &[PathBuf]) -> String {
    let Some(file) = find_program(program, path) else {
        return program.to_owned();
    };
    let resolved = file
        .parent()
        .and_then(|dir| std::fs::canonicalize(dir).ok())
        .zip(file.file_name())
        .map_or(file.clone(), |(dir, name)| dir.join(name));
    resolved.to_string_lossy().into_owned()
}

/// `PATH`, split.
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// What `known` says of a graphical editor: its name, and its [`App::id`] when the list has it.
type KnownEditor = (String, Option<String>);

/// `$VISUAL`, then `$EDITOR`, when one names a graphical editor that is installed: `known` says
/// whether a program is one (from desktop entries; the [`GUI_EDITORS`] list otherwise).
fn editor_from_env(
    visual: Option<&str>,
    editor: Option<&str>,
    path: &[PathBuf],
    known: &dyn Fn(&str) -> Option<KnownEditor>,
) -> Option<SystemChoice> {
    [visual, editor].into_iter().flatten().find_map(|value| {
        let value = value.trim();
        let (program, _) = split_command(value)?;
        if !on_path(&program, path) {
            return None;
        }
        let (name, id) = known(&program_name(&program))?;
        Some(SystemChoice {
            name,
            command: Some(value.to_owned()),
            id,
        })
    })
}

/// What the [`GUI_EDITORS`] list knows about `program`: its own name, and no list entry.
fn listed_gui_editor(program: &str) -> Option<KnownEditor> {
    GUI_EDITORS
        .contains(&program.to_ascii_lowercase().as_str())
        .then(|| (program.to_owned(), None))
}

/// A well-known program Settings looks for, and what each platform calls it. Settings offers the
/// ones found as installed programs and, where a platform can tell reliably (Linux, macOS), the
/// others as "Not installed". Graphical editors only: a terminal editor (vim, helix) needs a
/// terminal, so it is listed only through a desktop entry that a terminal launcher can start.
/// Sources are in docs/adr/0016-launcher-lists.md.
struct Known {
    kind: Kind,
    name: &'static str,
    /// Linux: its command names on `PATH`, the usual one first.
    commands: &'static [&'static str],
    /// Linux: the ids of the desktop entries that install it.
    entries: &'static [&'static str],
    /// macOS: its application bundle's name, without `.app`.
    bundle: Option<&'static str>,
    /// Windows: its App Paths key.
    exe: Option<&'static str>,
}

impl Known {
    const fn new(kind: Kind, name: &'static str) -> Self {
        Self {
            kind,
            name,
            commands: &[],
            entries: &[],
            bundle: None,
            exe: None,
        }
    }

    const fn linux(
        mut self,
        commands: &'static [&'static str],
        entries: &'static [&'static str],
    ) -> Self {
        self.commands = commands;
        self.entries = entries;
        self
    }

    const fn mac(mut self, bundle: &'static str) -> Self {
        self.bundle = Some(bundle);
        self
    }

    const fn windows(mut self, exe: &'static str) -> Self {
        self.exe = Some(exe);
        self
    }
}

const KNOWN: &[Known] = &[
    Known::new(Kind::Editor, "Visual Studio Code")
        .linux(
            &["code"],
            &[
                "code.desktop",
                "visual-studio-code.desktop",
                "com.visualstudio.code.desktop",
            ],
        )
        .mac("Visual Studio Code")
        .windows("code.exe"),
    Known::new(Kind::Editor, "VSCodium").linux(
        &["codium"],
        &[
            "codium.desktop",
            "vscodium.desktop",
            "com.vscodium.codium.desktop",
        ],
    ),
    Known::new(Kind::Editor, "Cursor")
        .linux(&["cursor"], &["cursor.desktop"])
        .mac("Cursor")
        .windows("cursor.exe"),
    Known::new(Kind::Editor, "Zed")
        .linux(&["zed", "zeditor"], &["dev.zed.Zed.desktop", "zed.desktop"])
        .mac("Zed")
        .windows("zed.exe"),
    Known::new(Kind::Editor, "Sublime Text")
        .linux(
            &["subl", "sublime_text"],
            &[
                "sublime_text.desktop",
                "sublime-text.desktop",
                "com.sublimetext.three.desktop",
            ],
        )
        .mac("Sublime Text")
        .windows("sublime_text.exe"),
    Known::new(Kind::Editor, "Windsurf")
        .linux(&["windsurf"], &["windsurf.desktop"])
        .mac("Windsurf"),
    Known::new(Kind::Editor, "Kate").linux(&["kate"], &["org.kde.kate.desktop"]),
    Known::new(Kind::Editor, "Text Editor")
        .linux(&["gnome-text-editor"], &["org.gnome.TextEditor.desktop"]),
    Known::new(Kind::Editor, "gedit").linux(&["gedit"], &["org.gnome.gedit.desktop"]),
    Known::new(Kind::Editor, "Mousepad").linux(&["mousepad"], &["org.xfce.mousepad.desktop"]),
    Known::new(Kind::Editor, "Geany").linux(&["geany"], &["geany.desktop"]),
    Known::new(Kind::Editor, "Notepad++").windows("notepad++.exe"),
    Known::new(Kind::Editor, "BBEdit").mac("BBEdit"),
    Known::new(Kind::Editor, "CotEditor").mac("CotEditor"),
    Known::new(Kind::Editor, "Nova").mac("Nova"),
    Known::new(Kind::Editor, "TextMate").mac("TextMate"),
    Known::new(Kind::Editor, "MacVim").mac("MacVim"),
    Known::new(Kind::Editor, "TextEdit").mac("TextEdit"),
    Known::new(Kind::Terminal, "Terminal").mac("Terminal"),
    Known::new(Kind::Terminal, "Windows Terminal").windows("wt.exe"),
    Known::new(Kind::Terminal, "Console").linux(&["kgx"], &["org.gnome.Console.desktop"]),
    Known::new(Kind::Terminal, "GNOME Terminal")
        .linux(&["gnome-terminal"], &["org.gnome.Terminal.desktop"]),
    Known::new(Kind::Terminal, "Ptyxis").linux(&["ptyxis"], &["org.gnome.Ptyxis.desktop"]),
    Known::new(Kind::Terminal, "Konsole").linux(&["konsole"], &["org.kde.konsole.desktop"]),
    Known::new(Kind::Terminal, "Xfce Terminal")
        .linux(&["xfce4-terminal"], &["xfce4-terminal.desktop"]),
    Known::new(Kind::Terminal, "iTerm").mac("iTerm"),
    Known::new(Kind::Terminal, "Alacritty")
        .linux(
            &["alacritty"],
            &["Alacritty.desktop", "org.alacritty.Alacritty.desktop"],
        )
        .mac("Alacritty")
        .windows("alacritty.exe"),
    Known::new(Kind::Terminal, "kitty")
        .linux(
            &["kitty"],
            &["kitty.desktop", "net.kovidgoyal.kitty.desktop"],
        )
        .mac("kitty"),
    Known::new(Kind::Terminal, "Foot").linux(&["foot"], &["foot.desktop"]),
    Known::new(Kind::Terminal, "WezTerm")
        .linux(&["wezterm"], &["org.wezfurlong.wezterm.desktop"])
        .mac("WezTerm")
        .windows("wezterm-gui.exe"),
    Known::new(Kind::Terminal, "Ghostty")
        .linux(&["ghostty"], &["com.mitchellh.ghostty.desktop"])
        .mac("Ghostty"),
    Known::new(Kind::Terminal, "Warp").mac("Warp"),
    Known::new(Kind::Terminal, "Tilix").linux(&["tilix"], &["com.gexperts.Tilix.desktop"]),
    Known::new(Kind::Terminal, "Terminator").linux(&["terminator"], &["terminator.desktop"]),
    Known::new(Kind::Terminal, "xterm").linux(&["xterm"], &["xterm.desktop"]),
    Known::new(Kind::FileManager, "Finder").mac("Finder"),
    Known::new(Kind::FileManager, "Files").linux(&["nautilus"], &["org.gnome.Nautilus.desktop"]),
    Known::new(Kind::FileManager, "Dolphin").linux(&["dolphin"], &["org.kde.dolphin.desktop"]),
    Known::new(Kind::FileManager, "Thunar").linux(&["thunar"], &["thunar.desktop"]),
    Known::new(Kind::FileManager, "Nemo").linux(&["nemo"], &["nemo.desktop"]),
    Known::new(Kind::FileManager, "PCManFM").linux(&["pcmanfm"], &["pcmanfm.desktop"]),
    Known::new(Kind::FileManager, "Caja")
        .linux(&["caja"], &["org.mate.Caja.desktop", "caja.desktop"]),
    Known::new(Kind::FileManager, "COSMIC Files")
        .linux(&["cosmic-files"], &["com.system76.CosmicFiles.desktop"]),
    Known::new(Kind::FileManager, "Path Finder").mac("Path Finder"),
    Known::new(Kind::FileManager, "ForkLift").mac("ForkLift"),
    Known::new(Kind::FileManager, "Commander One").mac("Commander One"),
    Known::new(Kind::FileManager, "Total Commander").windows("totalcmd64.exe"),
    Known::new(Kind::FileManager, "Files").windows("files.exe"),
];

/// The known applications among `bundles` (names without `.app`), started with `open -a`, and
/// those only found on `path` (a binary from Homebrew or Nix), started by their command.
#[cfg_attr(
    not(any(target_os = "macos", test)),
    expect(dead_code, reason = "macOS's applications")
)]
fn mac_apps(bundles: &[String], path: &[PathBuf]) -> Vec<App> {
    let apps = KNOWN
        .iter()
        .filter_map(|k| Some((k, k.bundle?)))
        .filter_map(|(k, bundle)| {
            if bundles.iter().any(|b| b == bundle) {
                let open = format!("open -a {}", crate::platform::quote(bundle));
                return Some(App {
                    kind: k.kind,
                    name: k.name.to_owned(),
                    command: match k.kind {
                        Kind::Editor => open,
                        Kind::Terminal | Kind::FileManager => format!("{open} %d"),
                    },
                    id: bundle.to_owned(),
                });
            }
            let command = k.commands.iter().find(|c| on_path(c, path))?;
            Some(App {
                kind: k.kind,
                name: k.name.to_owned(),
                command: (*command).to_owned(),
                id: executable_id(command, path),
            })
        })
        .collect();
    sorted(apps)
}

/// The known applications that are neither among `bundles` nor on `path`: what a saved command
/// for one of them starts is nowhere to be found, by `open -a` or by its command name.
#[cfg_attr(
    not(any(target_os = "macos", test)),
    expect(dead_code, reason = "macOS's applications")
)]
fn mac_missing(bundles: &[String], path: &[PathBuf]) -> Vec<Missing> {
    let missing = KNOWN
        .iter()
        .filter_map(|k| Some((k, k.bundle?)))
        .filter(|(k, bundle)| {
            !bundles.iter().any(|b| b == bundle) && !k.commands.iter().any(|c| on_path(c, path))
        })
        .map(|(k, bundle)| {
            Missing::new(
                k.kind,
                k.name,
                std::iter::once(bundle).chain(k.commands.iter().copied()),
            )
        })
        .collect();
    sorted_missing(missing)
}

/// The application names in `mdfind` output: one path per line, `.app` bundles only.
#[cfg_attr(
    not(any(target_os = "macos", test)),
    expect(dead_code, reason = "macOS's applications")
)]
fn bundles_from_mdfind(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|line| {
            Path::new(line.trim())
                .file_name()?
                .to_str()?
                .strip_suffix(".app")
                .map(str::to_owned)
        })
        .collect()
}

/// `(program name, path)` from `reg query "…\App Paths" /s`: each key's default value.
#[cfg_attr(
    not(any(target_os = "windows", test)),
    expect(dead_code, reason = "Windows's programs")
)]
fn parse_app_paths(out: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut key: Option<String> = None;
    for line in out.lines() {
        if line.starts_with("HKEY_") {
            key = line.trim().rsplit('\\').next().map(str::to_ascii_lowercase);
            continue;
        }
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("(Default)") else {
            continue;
        };
        let value = rest
            .trim_start()
            .strip_prefix("REG_EXPAND_SZ")
            .or_else(|| rest.trim_start().strip_prefix("REG_SZ"))
            .map(|v| v.trim().trim_matches('"'));
        if let (Some(k), Some(v)) = (&key, value.filter(|v| !v.is_empty())) {
            found.push((k.clone(), v.to_owned()));
        }
    }
    found
}

/// The known programs among the App Paths entries.
#[cfg_attr(
    not(any(target_os = "windows", test)),
    expect(dead_code, reason = "Windows's programs")
)]
fn windows_apps(paths: &[(String, String)]) -> Vec<App> {
    let apps = KNOWN
        .iter()
        .filter_map(|k| {
            let exe = k.exe?;
            let (_, path) = paths.iter().find(|(key, _)| key == exe)?;
            let program = crate::platform::quote(path);
            Some(App {
                kind: k.kind,
                name: k.name.to_owned(),
                command: match (k.kind, exe) {
                    (Kind::Terminal, "wt.exe") => format!("{program} -d %d"),
                    (Kind::FileManager, _) => format!("{program} %d"),
                    _ => program,
                },
                id: exe.to_owned(),
            })
        })
        .collect();
    sorted(apps)
}

#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(dead_code, reason = "desktop entries are Linux's")
)]
mod linux {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{
        App, Apps, KNOWN, Kind, Missing, SystemChoice, editor_from_env, executable_id,
        listed_gui_editor, on_path, sorted, sorted_missing,
    };
    use crate::platform::{program_name, quote};

    /// Flags that make an entry a server or daemon of a program, which opens no window to work in
    /// (`foot --server`); `--flag=value` counts too.
    const HELPER_FLAGS: &[&str] = &["--server", "--daemon", "--gapplication-service"];

    /// Commands that only reach a program already running (`footclient` needs `foot --server`).
    const HELPER_COMMANDS: &[&str] = &["footclient", "urxvtc", "emacsclient"];

    /// Programs that only start another one: an entry using one is told apart by its own id.
    const LAUNCHERS: &[&str] = &[
        "env", "flatpak", "snap", "sh", "bash", "dash", "zsh", "fish",
    ];

    /// The largest desktop entry or terminal list read; the largest entries (many translations)
    /// are a few tens of kilobytes.
    const MAX_FILE_BYTES: u64 = 256 * 1024;

    /// A small text file's contents: `None` for anything but a file (a link to a device or a
    /// pipe never ends) and for a file larger than [`MAX_FILE_BYTES`], however large it claims to
    /// be.
    fn read_small(path: &Path) -> Option<String> {
        use std::io::Read;
        if !std::fs::metadata(path).ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return None;
        }
        String::from_utf8(bytes).ok()
    }

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

        /// A server, daemon or client of a program rather than a program to work in
        /// ([`HELPER_FLAGS`], [`HELPER_COMMANDS`]).
        fn helper(&self) -> bool {
            let mut words = self.exec.iter();
            let client = words
                .next()
                .is_some_and(|p| HELPER_COMMANDS.contains(&program_name(p).as_str()));
            client || words.any(|w| HELPER_FLAGS.contains(&w.split('=').next().unwrap_or_default()))
        }

        /// The program it starts ([`App::id`]): its executable, and for a launcher such as
        /// `flatpak`, `env` or `sh -c`, which says nothing of what it starts, the entry itself.
        pub(super) fn program_id(&self, path: &[PathBuf]) -> String {
            match self.exec.first() {
                Some(p) if !LAUNCHERS.contains(&program_name(p).as_str()) => executable_id(p, path),
                _ => self.id.clone(),
            }
        }

        /// The entry as a command: `file_code` (`%f` or `%d`) where the entry takes files or
        /// URLs, other field codes and Flatpak's `@@` markers left out.
        pub(super) fn command(&self, file_code: &str) -> Option<String> {
            let mut words = Vec::new();
            for arg in &self.exec {
                match arg.as_str() {
                    "%f" | "%F" | "%u" | "%U" => words.push(file_code.to_owned()),
                    "@@" | "@@u" | "@@f" | "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N"
                    | "%v" | "%m" => {}
                    _ => {
                        // Embedded field codes violate the desktop-entry specification and
                        // cannot safely become an argument containing the user's file path.
                        // Reject the entry: removing a word can change which argument a
                        // program interprets as code (for example, the script after sh -c).
                        let without_escapes = arg.replace("%%", "");
                        if ["%f", "%F", "%u", "%U", "%d", "%D"]
                            .iter()
                            .any(|code| without_escapes.contains(*code))
                        {
                            return None;
                        }
                        // Keep %% intact until platform::with_path has decided whether
                        // the word is a placeholder, then unescape it there.
                        words.push(quote(arg));
                    }
                }
            }
            Some(words.join(" "))
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
            if let Some(entry) = read_small(&path).and_then(|text| parse_entry(&id, &text)) {
                found.insert(id, entry);
            }
        }
    }

    /// The terminal `xdg-terminal-exec` starts: the first entry of `xdg-terminals.list` that is
    /// installed.
    fn xdg_terminal<'e>(env: &Env, entries: &'e HashMap<String, Entry>) -> Option<&'e Entry> {
        env.config_dirs.iter().find_map(|dir| {
            let list = read_small(&dir.join("xdg-terminals.list"))?;
            list.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .filter_map(|id| entries.get(id.trim_start_matches(['+', '-'])))
                .find(|e| e.installed(&env.path))
        })
    }

    /// An editor entry as a command, run through `xdg-terminal-exec` when it is a terminal
    /// program. `None` when it needs a terminal and there is no launcher for one.
    fn editor_command(entry: &Entry, launcher: bool) -> Option<(String, String)> {
        let command = entry.command("%f")?;
        if entry.terminal {
            launcher.then(|| {
                (
                    format!("{} in terminal", entry.name),
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
            // The program's own entry names it, not its server or client.
            matching.sort_by_key(|e| (e.hidden, e.helper(), e.id.clone()));
            matching.first().copied()
        };

        let known = |program: &str| match by_program(program) {
            Some(entry) if entry.terminal => None,
            Some(entry) => Some((entry.name.clone(), Some(entry.program_id(&env.path)))),
            None => listed_gui_editor(program),
        };
        let editor = editor_from_env(
            env.visual.as_deref(),
            env.editor.as_deref(),
            &env.path,
            &known,
        )
        .or_else(|| {
            let entry = entries
                .get(&(env.default_for)("text/plain")?)
                .filter(|e| e.installed(&env.path))?;
            let (name, command) = editor_command(entry, launcher)?;
            Some(SystemChoice {
                name,
                command: Some(command),
                id: Some(entry.program_id(&env.path)),
            })
        });

        let terminal_named = |command: &str| {
            let (program, _) = crate::platform::split_command(command)?;
            if !on_path(&program, &env.path) {
                return None;
            }
            let entry = if program_name(&program) == "xdg-terminal-exec" {
                xdg_terminal(env, &entries)
            } else {
                by_program(&program_name(&program))
            };
            let (name, id) = entry.map_or_else(
                || (program.clone(), executable_id(&program, &env.path)),
                |e| (e.name.clone(), e.program_id(&env.path)),
            );
            Some(SystemChoice {
                name,
                command: Some(command.to_owned()),
                id: Some(id),
            })
        };
        let terminal = env
            .terminal
            .as_deref()
            .and_then(|t| terminal_named(t.trim()))
            .or_else(|| terminal_named("xdg-terminal-exec"))
            .or_else(|| super::linux_terminals().find_map(terminal_named));

        let file_manager = (env.default_for)("inode/directory")
            .and_then(|id| entries.get(&id))
            .filter(|e| !e.terminal && e.installed(&env.path))
            .and_then(|e| {
                Some(SystemChoice {
                    name: e.name.clone(),
                    command: Some(e.command("%d")?),
                    id: Some(e.program_id(&env.path)),
                })
            });

        let mut installed = Vec::new();
        for entry in entries.values() {
            if entry.hidden || entry.helper() || !entry.installed(&env.path) {
                continue;
            }
            let id = entry.program_id(&env.path);
            let has = |c: &str| entry.categories.iter().any(|x| x == c);
            if has("TerminalEmulator")
                && !entry.terminal
                && let Some(command) = entry.command("%d")
            {
                installed.push(App {
                    kind: Kind::Terminal,
                    name: entry.name.clone(),
                    command,
                    id: id.clone(),
                });
            }
            if has("FileManager")
                && !entry.terminal
                && let Some(command) = entry.command("%d")
            {
                installed.push(App {
                    kind: Kind::FileManager,
                    name: entry.name.clone(),
                    command,
                    id: id.clone(),
                });
            }
            // Plain text or source code: a chat app that takes CSV is no editor.
            let edits_text = entry
                .mime
                .iter()
                .any(|m| m == "text/plain" || m.starts_with("text/x-"));
            if (has("TextEditor") || ((has("IDE") || has("Development")) && edits_text))
                && let Some((name, command)) = editor_command(entry, launcher)
            {
                installed.push(App {
                    kind: Kind::Editor,
                    name,
                    command,
                    id,
                });
            }
        }

        // The well-known programs: one found on PATH that no desktop entry listed is installed
        // too, and one found nowhere is the "Not installed" row.
        let mut missing = Vec::new();
        for known in KNOWN.iter().filter(|k| !k.commands.is_empty()) {
            let on_path = known.commands.iter().find(|c| on_path(c, &env.path));
            let mut ids: Vec<String> = known
                .entries
                .iter()
                .filter_map(|id| entries.get(*id))
                .filter(|e| e.installed(&env.path))
                .map(|e| e.program_id(&env.path))
                .collect();
            ids.extend(on_path.map(|c| executable_id(c, &env.path)));
            if ids.is_empty() {
                missing.push(Missing::new(
                    known.kind,
                    known.name,
                    known.commands.iter().copied(),
                ));
            } else if let Some(command) = on_path
                && !installed
                    .iter()
                    .any(|a| a.kind == known.kind && ids.contains(&a.id))
            {
                installed.push(App {
                    kind: known.kind,
                    name: known.name.to_owned(),
                    command: (*command).to_owned(),
                    id: executable_id(command, &env.path),
                });
            }
        }

        Apps {
            editor,
            terminal,
            file_manager,
            installed: sorted(installed),
            missing: sorted_missing(missing),
            ..Apps::default()
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;
    use std::process::Command;

    use super::{
        Apps, SystemChoice, bundles_from_mdfind, editor_from_env, listed_gui_editor, mac_apps,
        mac_missing, path_dirs,
    };

    /// Every application Spotlight knows, wherever it is installed (`open -a` finds those too).
    #[expect(
        clippy::disallowed_methods,
        reason = "asks Spotlight for the installed applications, never git"
    )]
    fn spotlight_bundles() -> Vec<String> {
        let mut cmd = Command::new("mdfind");
        cmd.arg("kMDItemContentType == 'com.apple.application-bundle'");
        crate::platform::output_within(cmd, crate::platform::QUERY_TIMEOUT)
            .map(|out| bundles_from_mdfind(&out))
            .unwrap_or_default()
    }

    pub(super) fn detect() -> Apps {
        let home = crate::settings::home_dir();
        let path = path_dirs();
        let mut bundles = spotlight_bundles();
        for dir in [
            PathBuf::from("/Applications"),
            PathBuf::from("/Applications/Utilities"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Applications/Utilities"),
            PathBuf::from("/System/Library/CoreServices"),
            home.join("Applications"),
        ] {
            let Ok(read) = std::fs::read_dir(dir) else {
                continue;
            };
            bundles.extend(read.flatten().filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".app"))
                    .map(str::to_owned)
            }));
        }
        let var = |name: &str| std::env::var(name).ok();
        let editor = editor_from_env(
            var("VISUAL").as_deref(),
            var("EDITOR").as_deref(),
            &path,
            &listed_gui_editor,
        )
        .or(Some(SystemChoice {
            name: "Default text editor".to_owned(),
            command: None,
            id: None,
        }));
        Apps {
            editor,
            terminal: Some(SystemChoice {
                name: "Terminal".to_owned(),
                command: None,
                id: Some("Terminal".to_owned()),
            }),
            file_manager: Some(SystemChoice {
                name: "Finder".to_owned(),
                command: None,
                id: Some("Finder".to_owned()),
            }),
            installed: mac_apps(&bundles, &path),
            missing: mac_missing(&bundles, &path),
            ..Apps::default()
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::process::Command;

    use super::{
        Apps, SystemChoice, editor_from_env, listed_gui_editor, on_path, parse_app_paths,
        path_dirs, windows_apps,
    };

    #[expect(
        clippy::disallowed_methods,
        reason = "reads the App Paths registry keys with reg.exe, never git"
    )]
    fn app_paths() -> Vec<(String, String)> {
        let mut found = Vec::new();
        for root in ["HKCU", "HKLM"] {
            let mut cmd = Command::new("reg");
            cmd.args([
                "query",
                &format!(r"{root}\Software\Microsoft\Windows\CurrentVersion\App Paths"),
                "/s",
            ]);
            if let Some(out) = crate::platform::output_within(cmd, crate::platform::QUERY_TIMEOUT) {
                found.extend(parse_app_paths(&out));
            }
        }
        found
    }

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
            id: None,
        }));
        let terminal = if on_path("wt.exe", &path) {
            SystemChoice {
                name: "Windows Terminal".to_owned(),
                command: Some("wt -d %d".to_owned()),
                id: Some("wt.exe".to_owned()),
            }
        } else {
            SystemChoice {
                name: "Command Prompt".to_owned(),
                command: None,
                id: None,
            }
        };
        Apps {
            editor,
            terminal: Some(terminal),
            file_manager: Some(SystemChoice {
                name: "File Explorer".to_owned(),
                command: None,
                id: None,
            }),
            installed: windows_apps(&app_paths()),
            // App Paths misses programs that are only on PATH (wt.exe is an alias) and Explorer,
            // so it cannot say that one is not installed.
            missing: Vec::new(),
            ..Apps::default()
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

    /// The id of the program at `relative` under `root` ([`App::id`]).
    fn exe(root: &Path, relative: &str) -> String {
        let file = root.join(relative);
        std::fs::canonicalize(file.parent().unwrap())
            .unwrap()
            .join(file.file_name().unwrap())
            .to_string_lossy()
            .into_owned()
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
    fn embedded_file_codes_never_turn_a_shell_script_into_a_path_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        program(&root.join("bin"), "editor");
        for exec in [
            r#"/bin/sh -c "editor --file=%f""#,
            r#"/bin/sh -c "editor --file=%f" ignored %f"#,
            "editor --file=%f",
            "editor --directory=%U",
            "editor --file=%%%f",
        ] {
            write(
                &root.join("home-data/applications"),
                "unsafe.desktop",
                &format!(
                    "[Desktop Entry]\nType=Application\nName=Editor\nExec={exec}\nCategories=TextEditor;FileManager;TerminalEmulator;\n"
                ),
            );
            let found = detect(&env(
                root,
                &[
                    ("text/plain", "unsafe.desktop"),
                    ("inode/directory", "unsafe.desktop"),
                ],
            ));
            assert!(found.editor.is_none(), "{exec}");
            assert!(found.file_manager.is_none(), "{exec}");
            assert!(found.installed.is_empty(), "{exec}");
        }
    }

    /// An executable `bin/<name>` that writes each of its arguments on a line of `bin/<name>.out`.
    #[cfg(unix)]
    fn recorder(root: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        write(
            &bin,
            name,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.tmp\" && mv \"$0.tmp\" \"$0.out\"\n",
        );
        let file = bin.join(name);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    /// The arguments `recorder` was started with, once it has run.
    #[cfg(unix)]
    fn recorded(program: &Path) -> Vec<String> {
        let out = program.with_extension("out");
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(&out) {
                return text.lines().map(str::to_owned).collect();
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("{} never ran", program.display());
    }

    /// Entries found on disk, chosen in Settings and launched: the file or folder reaches the
    /// program as one argument whatever its name, and an escaped `%%f` stays a literal `%f`.
    #[cfg(unix)]
    #[test]
    fn a_standalone_placeholder_launches_with_the_path_as_one_argument() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let editor = recorder(root, "editor");
        let escaped = recorder(root, "escaped");
        let files = recorder(root, "files");
        let term = recorder(root, "term");
        let apps = root.join("home-data/applications");
        for (id, kind, name, exec) in [
            (
                "editor",
                Kind::Editor,
                "Editor",
                format!("{} --wait %F", editor.display()),
            ),
            (
                "escaped",
                Kind::Editor,
                "Escaped",
                format!("{} --label %%f %f", escaped.display()),
            ),
            (
                "files",
                Kind::FileManager,
                "Files",
                format!("{} %U", files.display()),
            ),
            (
                "term",
                Kind::Terminal,
                "Term",
                format!("{} --cwd %U", term.display()),
            ),
        ] {
            write(
                &apps,
                &format!("{id}.desktop"),
                &entry_of(kind, name, &exec),
            );
        }
        let found = detect(&env(root, &[]));
        let command = |kind: Kind, name: &str| {
            found
                .installed(kind)
                .find(|a| a.name == name)
                .unwrap_or_else(|| panic!("{name} is not offered"))
                .command
                .clone()
        };

        let work = root.join("work dir");
        std::fs::create_dir_all(&work).unwrap();
        let file = work.join("my file $(touch pwned) %f.rs");
        std::fs::write(&file, "").unwrap();
        crate::platform::open_in_editor(&file, &command(Kind::Editor, "Editor")).unwrap();
        assert_eq!(recorded(&editor), ["--wait", file.to_str().unwrap()]);
        crate::platform::open_in_editor(&file, &command(Kind::Editor, "Escaped")).unwrap();
        assert_eq!(
            recorded(&escaped),
            ["--label", "%f", file.to_str().unwrap()]
        );
        crate::platform::open_folder(&work, &command(Kind::FileManager, "Files")).unwrap();
        assert_eq!(recorded(&files), [work.to_str().unwrap()]);
        crate::platform::open_terminal(&work, &command(Kind::Terminal, "Term")).unwrap();
        assert_eq!(recorded(&term), ["--cwd", work.to_str().unwrap()]);
        assert!(!work.join("pwned").exists() && !root.join("pwned").exists());
    }

    /// A command with a placeholder inside a larger word is refused when it is run, so a name
    /// can never become part of another argument; the program is not started.
    #[cfg(unix)]
    #[test]
    fn a_command_with_an_embedded_placeholder_is_refused_before_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let program = recorder(root, "editor");
        let shell = |script: &str| format!("/bin/sh -c {}", crate::platform::quote(script));
        let file = root.join("a b.rs");
        std::fs::write(&file, "").unwrap();
        for command in [
            format!("{} --file=%f", program.display()),
            format!("{} --file=%%%f", program.display()),
            shell(&format!("{} --file=%f", program.display())),
        ] {
            let refused = crate::platform::open_in_editor(&file, &command).unwrap_err();
            assert!(
                refused.contains("must be a word of its own"),
                "{command}: {refused}"
            );
            let refused =
                crate::platform::open_folder(root, &command.replace("%f", "%d")).unwrap_err();
            assert!(
                refused.contains("must be a word of its own"),
                "{command}: {refused}"
            );
            let refused =
                crate::platform::open_terminal(root, &command.replace("%f", "%d")).unwrap_err();
            assert!(
                refused.contains("must be a word of its own"),
                "{command}: {refused}"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!program.with_extension("out").exists());
    }

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
        assert_eq!(
            entry.command("%f").as_deref(),
            Some("/opt/code/bin/code --unity-launch %f")
        );
        let flatpak = parse_entry(
            "org.gnome.TextEditor.desktop",
            "[Desktop Entry]\nType=Application\nName=Text Editor\nExec=/usr/bin/flatpak run --file-forwarding org.gnome.TextEditor @@ %F @@\n",
        )
        .unwrap();
        assert_eq!(
            flatpak.command("%f").as_deref(),
            Some("/usr/bin/flatpak run --file-forwarding org.gnome.TextEditor %f")
        );
        let malformed = parse_entry(
            "broken.desktop",
            "[Desktop Entry]\nType=Application\nName=Broken\nExec=editor --file=%f --title %%f\n",
        )
        .unwrap();
        assert_eq!(
            malformed.command("%f"),
            None,
            "reject entries with unsafe embedded substitutions"
        );
        let escaped = parse_entry(
            "escaped.desktop",
            "[Desktop Entry]\nType=Application\nName=Escaped\nExec=editor %%f\n",
        )
        .unwrap();
        assert_eq!(
            escaped.command("%f").as_deref(),
            Some("editor %%f"),
            "an escaped percent is not a file placeholder"
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
    fn oversized_and_endless_entries_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("applications");
        let padding = "# padding\n".repeat(30_000);
        write(&apps, "big.desktop", &format!("{KITTY}{padding}"));
        write(&apps, "files.desktop", FILES);
        #[cfg(unix)]
        std::os::unix::fs::symlink("/dev/zero", apps.join("zero.desktop")).unwrap();
        let found = entries(&[dir.path().to_path_buf()]);
        assert!(!found.contains_key("big.desktop"), "read past the limit");
        assert!(!found.contains_key("zero.desktop"));
        assert_eq!(found["files.desktop"].name, "Files");
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
                name: "Neovim in terminal".to_owned(),
                command: Some("xdg-terminal-exec nvim %f".to_owned()),
                id: Some(exe(root, "bin/nvim")),
            })
        );
        assert_eq!(
            found.terminal,
            Some(SystemChoice {
                name: "kitty".to_owned(),
                command: Some("xdg-terminal-exec".to_owned()),
                id: Some(exe(root, "bin/kitty")),
            })
        );
        assert_eq!(
            found.file_manager,
            Some(SystemChoice {
                name: "Files".to_owned(),
                command: Some("nautilus --new-window %d".to_owned()),
                id: Some(exe(root, "bin/nautilus")),
            })
        );

        // A graphical editor in $VISUAL wins, named by its desktop entry.
        let mut e = env(root, &defaults);
        // Quoted, as a user would write a path with spaces (the temporary folder may have some).
        e.visual = Some(format!(
            "\"{}\" --wait",
            root.join("opt/code/bin/code").display()
        ));
        e.editor = Some("nvim".to_owned());
        let found = detect(&e);
        let editor = found.editor.unwrap();
        assert_eq!(editor.name, "Visual Studio Code");
        assert_eq!(editor.command, e.visual);
        // No $TERMINAL: xdg-terminal-exec is the desktop's.
        assert_eq!(
            found.terminal.unwrap().command.as_deref(),
            Some("xdg-terminal-exec")
        );

        // Nothing configured, no launcher: a terminal editor cannot be the default, and the
        // terminal is the first one found.
        std::fs::remove_file(root.join("bin/xdg-terminal-exec")).unwrap();
        let found = detect(&env(root, &defaults));
        assert_eq!(found.editor, None);
        let terminal = found.terminal.unwrap();
        assert_eq!(
            (terminal.name.as_str(), terminal.command.as_deref()),
            ("kitty", Some("kitty"))
        );
    }

    /// The program a missing default names, as a desktop entry and as a command, for `kind`.
    #[cfg(unix)]
    fn ghost_entry(kind: Kind) -> String {
        entry_of(kind, "Ghost", "ghost %F")
    }

    // Every source of a default: the environment, the desktop's text editor and file manager,
    // and `xdg-terminal-exec`. A program that is not installed is never the default.
    #[cfg(unix)]
    #[test]
    fn a_default_naming_a_program_that_is_not_installed_is_never_offered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let apps = root.join("usr-share/applications");
        write(&apps, "ghost-term.desktop", &ghost_entry(Kind::Terminal));
        write(&apps, "ghost-edit.desktop", &ghost_entry(Kind::Editor));
        write(
            &apps,
            "ghost-files.desktop",
            &ghost_entry(Kind::FileManager),
        );
        write(
            &root.join("config"),
            "xdg-terminals.list",
            "ghost-term.desktop\n",
        );
        let defaults = [
            ("text/plain", "ghost-edit.desktop"),
            ("inode/directory", "ghost-files.desktop"),
        ];
        let mut e = env(root, &defaults);
        e.terminal = Some("ghost".to_owned());
        e.visual = Some("ghost --wait".to_owned());
        e.editor = Some("ghost".to_owned());
        let found = detect(&e);
        assert_eq!(found.terminal, None, "$TERMINAL");
        assert_eq!(
            found.editor, None,
            "$VISUAL, $EDITOR and the desktop's text editor"
        );
        assert_eq!(found.file_manager, None, "the desktop's file manager");

        // xdg-terminal-exec starts the first terminal of its list that is installed.
        program(&root.join("bin"), "xdg-terminal-exec");
        program(&root.join("bin"), "kitty");
        write(&apps, "kitty.desktop", KITTY);
        write(
            &root.join("config"),
            "xdg-terminals.list",
            "ghost-term.desktop\nkitty.desktop\n",
        );
        e.terminal = None;
        assert_eq!(detect(&e).terminal.unwrap().name, "kitty");

        // The installed program after a missing one is the default.
        for p in ["code", "nautilus"] {
            program(&root.join("bin"), p);
        }
        write(
            &apps,
            "code.desktop",
            &CODE.replace("/opt/code/bin/code", "code"),
        );
        write(&apps, "org.gnome.Nautilus.desktop", FILES);
        e.visual = Some("ghost --wait".to_owned());
        e.editor = Some("code --wait".to_owned());
        let found = detect(&e);
        assert_eq!(
            found.editor.unwrap().command.as_deref(),
            Some("code --wait")
        );
        let mut e = env(root, &[("inode/directory", "org.gnome.Nautilus.desktop")]);
        e.terminal = Some("ghost".to_owned());
        assert_eq!(detect(&e).file_manager.unwrap().name, "Files");
    }

    const FOOT: &str = "[Desktop Entry]\nType=Application\nName=Foot\nExec=foot\nCategories=System;TerminalEmulator;\n";
    const FOOT_SERVER: &str = "[Desktop Entry]\nType=Application\nName=Foot Server\nExec=foot --server\nCategories=System;TerminalEmulator;\n";
    const FOOT_CLIENT: &str = "[Desktop Entry]\nType=Application\nName=Foot Client\nExec=footclient\nCategories=System;TerminalEmulator;\n";
    const GHOSTTY: &str = "[Desktop Entry]\nType=Application\nName=Ghostty\nExec=ghostty\nCategories=System;TerminalEmulator;\n";

    // The maintainer's machine: TERMINAL=xdg-terminal-exec, which resolves to kitty through
    // xdg-terminals.list, with kitty, foot, footclient and Ghostty installed.
    #[cfg(unix)]
    #[test]
    fn a_terminal_is_listed_once_and_its_helpers_not_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let apps = root.join("usr-share/applications");
        write(&apps, "kitty.desktop", KITTY);
        write(&apps, "foot.desktop", FOOT);
        write(&apps, "foot-server.desktop", FOOT_SERVER);
        write(&apps, "footclient.desktop", FOOT_CLIENT);
        write(&apps, "com.mitchellh.ghostty.desktop", GHOSTTY);
        write(
            &root.join("config"),
            "xdg-terminals.list",
            "kitty.desktop\n",
        );
        for p in [
            "kitty",
            "foot",
            "footclient",
            "ghostty",
            "xdg-terminal-exec",
        ] {
            program(&root.join("bin"), p);
        }
        let mut e = env(root, &[]);
        e.terminal = Some("xdg-terminal-exec".to_owned());
        let found = detect(&e);
        assert_eq!(
            names(&found, Kind::Terminal),
            [
                ("Foot".to_owned(), "foot".to_owned()),
                ("Ghostty".to_owned(), "ghostty".to_owned()),
                ("kitty".to_owned(), "kitty".to_owned()),
            ]
        );
        // The default is kitty under another command, and is one of the rows, not another.
        let default = found.terminal.as_ref().unwrap();
        assert_eq!(default.command.as_deref(), Some("xdg-terminal-exec"));
        assert_eq!(listed_as_default(&found, Kind::Terminal), ["kitty"]);

        // $TERMINAL naming a program with a server and a client entry is the program's own.
        for (terminal, name) in [("foot", "Foot"), ("footclient", "Foot Client")] {
            e.terminal = Some(terminal.to_owned());
            let found = detect(&e);
            assert_eq!(found.terminal.as_ref().unwrap().name, name);
        }
        e.terminal = Some("foot".to_owned());
        assert_eq!(listed_as_default(&detect(&e), Kind::Terminal), ["Foot"]);
    }

    #[test]
    fn a_chosen_command_comes_before_the_systems() {
        assert_eq!(
            launch(None, "  zed  ", Kind::Editor),
            Launch {
                command: "zed".to_owned(),
                fallback: None
            }
        );
    }

    /// Executable names stay case-sensitive where the platform's process launch is. A distinct
    /// custom `ZED` must not be mistaken for the well-known `zed` and replaced by the default.
    #[cfg(not(windows))]
    #[test]
    fn a_case_distinct_custom_command_runs_as_typed() {
        let apps = Apps {
            editor: Some(SystemChoice {
                name: "Default editor".into(),
                command: Some("default-editor".into()),
                id: None,
            }),
            missing: vec![Missing::new(Kind::Editor, "Zed", ["zed", "zeditor"])],
            ..Apps::default()
        };
        assert_eq!(
            launch(Some(&apps), "ZED", Kind::Editor),
            Launch {
                command: "ZED".into(),
                fallback: None
            }
        );
        assert_eq!(
            launch(Some(&apps), "zed", Kind::Editor).command,
            "default-editor"
        );
    }

    #[cfg(unix)]
    fn names(apps: &Apps, kind: Kind) -> Vec<(String, String)> {
        apps.installed(kind)
            .map(|a| (a.name.clone(), a.command.clone()))
            .collect()
    }

    /// The listed programs of `kind` that are the system's choice.
    #[cfg(unix)]
    fn listed_as_default(apps: &Apps, kind: Kind) -> Vec<String> {
        let system = apps.system(kind).unwrap();
        apps.installed(kind)
            .filter(|a| system.is(a))
            .map(|a| a.name.clone())
            .collect()
    }

    // A desktop entry's Exec line holds a Unix path; a Windows one is not valid there.
    #[cfg(unix)]
    #[test]
    fn installed_apps_are_offered_by_what_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let apps = root.join("usr-share/applications");
        write(&apps, "nvim.desktop", NVIM);
        let code = root.join("opt/code/bin/code");
        write(
            &apps,
            "code.desktop",
            &CODE.replace("/opt/code/bin/code", &format!("\"{}\"", code.display())),
        );
        write(&apps, "org.gnome.Nautilus.desktop", FILES);
        write(&apps, "kitty.desktop", KITTY);
        // Not offered: hidden, not installed, and a viewer that is not an editor.
        write(
            &apps,
            "hidden.desktop",
            &KITTY.replace("Name=kitty", "Name=Hidden\nNoDisplay=true"),
        );
        write(
            &apps,
            "gone.desktop",
            &KITTY.replace("Exec=kitty", "Exec=gone-term"),
        );
        write(
            &apps,
            "viewer.desktop",
            "[Desktop Entry]\nType=Application\nName=Viewer\nExec=nvim\nCategories=Graphics;\nMimeType=image/png;\n",
        );
        write(
            &apps,
            "chat.desktop",
            "[Desktop Entry]\nType=Application\nName=Chat\nExec=kitty %U\nCategories=Utility;Development;\nMimeType=text/csv;x-scheme-handler/https;\n",
        );
        for p in ["nvim", "nautilus", "kitty", "xdg-terminal-exec"] {
            program(&root.join("bin"), p);
        }
        program(&root.join("opt/code/bin"), "code");
        let found = detect(&env(root, &[]));
        assert_eq!(
            names(&found, Kind::Editor),
            [
                (
                    "Neovim in terminal".to_owned(),
                    "xdg-terminal-exec nvim %f".to_owned()
                ),
                (
                    "Visual Studio Code".to_owned(),
                    format!(
                        "{} --unity-launch %f",
                        crate::platform::quote(&code.display().to_string())
                    )
                ),
            ]
        );
        assert_eq!(
            names(&found, Kind::Terminal),
            [("kitty".to_owned(), "kitty".to_owned())]
        );
        assert_eq!(
            names(&found, Kind::FileManager),
            [("Files".to_owned(), "nautilus --new-window %d".to_owned())]
        );
        // Without a terminal launcher a terminal editor is not offered.
        std::fs::remove_file(root.join("bin/xdg-terminal-exec")).unwrap();
        let found = detect(&env(root, &[]));
        assert_eq!(names(&found, Kind::Editor).len(), 1);
    }

    /// A desktop entry offering `exec` as a program of `kind`.
    #[cfg(unix)]
    fn entry_of(kind: Kind, name: &str, exec: &str) -> String {
        let category = match kind {
            Kind::Terminal => "TerminalEmulator",
            Kind::FileManager => "FileManager",
            Kind::Editor => "TextEditor",
        };
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nCategories=Utility;{category};\n"
        )
    }

    const KINDS: [Kind; 3] = [Kind::Terminal, Kind::Editor, Kind::FileManager];

    #[cfg(unix)]
    #[test]
    fn one_program_found_twice_is_one_row_for_every_kind() {
        for kind in KINDS {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let apps = root.join("usr-share/applications");
            // The same file by its name and by its path, and through a folder that is a link
            // (`/bin` is `/usr/bin`).
            write(&apps, "one.desktop", &entry_of(kind, "Tool", "tool"));
            let by_path = crate::platform::quote(&root.join("bin/tool").display().to_string());
            write(
                &apps,
                "two.desktop",
                &entry_of(kind, "Tool again", &by_path),
            );
            std::os::unix::fs::symlink(root.join("bin"), root.join("sbin")).unwrap();
            let by_link = crate::platform::quote(&root.join("sbin/tool").display().to_string());
            write(
                &apps,
                "three.desktop",
                &entry_of(kind, "Tool once more", &by_link),
            );
            program(&root.join("bin"), "tool");
            let mut e = env(root, &[]);
            e.path.push(root.join("sbin"));
            let found = detect(&e);
            assert_eq!(
                names(&found, kind),
                [("Tool".to_owned(), "tool".to_owned())],
                "{kind:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn programs_are_told_apart_by_what_they_run_never_by_their_name() {
        for kind in KINDS {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let apps = root.join("usr-share/applications");
            // Two programs under one name, and two Flatpaks, which share the `flatpak` program.
            write(&apps, "a.desktop", &entry_of(kind, "Same", "alpha"));
            write(&apps, "b.desktop", &entry_of(kind, "Same", "beta"));
            write(
                &apps,
                "org.a.A.desktop",
                &entry_of(kind, "Flat A", "flatpak run org.a.A"),
            );
            write(
                &apps,
                "org.b.B.desktop",
                &entry_of(kind, "Flat B", "flatpak run org.b.B"),
            );
            for p in ["alpha", "beta", "flatpak"] {
                program(&root.join("bin"), p);
            }
            let found = detect(&env(root, &[]));
            let listed: Vec<String> = found.installed(kind).map(|a| a.command.clone()).collect();
            assert_eq!(
                listed,
                [
                    "flatpak run org.a.A",
                    "flatpak run org.b.B",
                    "alpha",
                    "beta"
                ],
                "{kind:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn servers_daemons_and_clients_are_not_programs_to_open() {
        // Each helper is its own program, so only the helper rule can leave it out.
        let helpers = [
            "termd --server",
            "termd --server=/run/user/1000/termd.sock",
            "termd --daemon",
            "termd --gapplication-service",
            "footclient",
            "@ROOT@/bin/urxvtc --no-fork",
            "emacsclient -c %F",
        ];
        for kind in KINDS {
            for helper in helpers {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path();
                let apps = root.join("usr-share/applications");
                let helper = helper.replace("@ROOT@", &root.display().to_string());
                write(&apps, "foot.desktop", &entry_of(kind, "Foot", "foot"));
                write(&apps, "helper.desktop", &entry_of(kind, "Helper", &helper));
                for p in ["foot", "termd", "footclient", "urxvtc", "emacsclient"] {
                    program(&root.join("bin"), p);
                }
                let found = detect(&env(root, &[]));
                assert_eq!(
                    names(&found, kind),
                    [("Foot".to_owned(), "foot".to_owned())],
                    "{kind:?}: {helper}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_system_choice_is_the_listed_program_it_starts_whatever_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let apps = root.join("usr-share/applications");
        write(&apps, "kitty.desktop", KITTY);
        write(&apps, "org.gnome.Nautilus.desktop", FILES);
        write(
            &apps,
            "org.gnome.TextEditor.desktop",
            &entry_of(Kind::Editor, "Text Editor", "gnome-text-editor %U"),
        );
        write(
            &apps,
            "code.desktop",
            &CODE.replace("/opt/code/bin/code", "code"),
        );
        for p in ["kitty", "nautilus", "gnome-text-editor", "code"] {
            program(&root.join("bin"), p);
        }
        let defaults = [
            ("text/plain", "org.gnome.TextEditor.desktop"),
            ("inode/directory", "org.gnome.Nautilus.desktop"),
        ];

        // The desktop's choices, and a terminal by its own command.
        let mut e = env(root, &defaults);
        e.terminal = Some("kitty".to_owned());
        let found = detect(&e);
        assert_eq!(listed_as_default(&found, Kind::Terminal), ["kitty"]);
        assert_eq!(listed_as_default(&found, Kind::Editor), ["Text Editor"]);
        assert_eq!(listed_as_default(&found, Kind::FileManager), ["Files"]);

        // $VISUAL and $EDITOR name a program, with flags, that has an entry.
        for (visual, editor) in [(Some("code --wait"), None), (None, Some("code -w"))] {
            let mut e = env(root, &defaults);
            e.visual = visual.map(str::to_owned);
            e.editor = editor.map(str::to_owned);
            let found = detect(&e);
            assert_eq!(
                listed_as_default(&found, Kind::Editor),
                ["Visual Studio Code"]
            );
        }

        // The first usual terminal found has an entry too; without one, a well-known program on
        // PATH is still an installed program, so the default is its row.
        let found = detect(&env(root, &defaults));
        assert_eq!(listed_as_default(&found, Kind::Terminal), ["kitty"]);
        std::fs::remove_file(apps.join("kitty.desktop")).unwrap();
        let found = detect(&env(root, &defaults));
        assert_eq!(found.terminal.as_ref().unwrap().name, "kitty");
        assert_eq!(listed_as_default(&found, Kind::Terminal), ["kitty"]);
        // A program nobody knows is the default but has no row.
        program(&root.join("bin"), "obscure-term");
        let mut e = env(root, &defaults);
        e.terminal = Some("obscure-term".to_owned());
        let found = detect(&e);
        assert_eq!(found.terminal.as_ref().unwrap().name, "obscure-term");
        assert_eq!(
            listed_as_default(&found, Kind::Terminal),
            Vec::<String>::new()
        );
    }

    fn missing_names(apps: &Apps, kind: Kind) -> Vec<&str> {
        apps.missing(kind).map(|m| m.name.as_str()).collect()
    }

    // Installed programs, each once, then the well-known ones that are not installed: found by
    // entry, by PATH, or both, and never both listed and missing.
    #[cfg(unix)]
    #[test]
    fn well_known_programs_are_installed_or_not_installed_never_both() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let apps = root.join("usr-share/applications");
        // kitty: an entry and PATH. Foot: PATH only, no entry. Code: PATH and an entry whose
        // program is another file (the packages put `code` in /usr/bin and the entry elsewhere).
        // Konsole: an entry whose program is gone.
        write(&apps, "kitty.desktop", KITTY);
        write(
            &apps,
            "code.desktop",
            &CODE.replace(
                "/opt/code/bin/code",
                &root.join("opt/code/code").display().to_string(),
            ),
        );
        write(
            &apps,
            "org.kde.konsole.desktop",
            &entry_of(Kind::Terminal, "Konsole", "konsole"),
        );
        // Not well known and not installed: not offered either way.
        write(
            &apps,
            "gone.desktop",
            &entry_of(Kind::Terminal, "Gone term", "gone-term"),
        );
        for p in ["kitty", "foot", "code"] {
            program(&root.join("bin"), p);
        }
        program(&root.join("opt/code"), "code");
        let found = detect(&env(root, &[]));
        assert_eq!(
            names(&found, Kind::Terminal),
            [
                ("Foot".to_owned(), "foot".to_owned()),
                ("kitty".to_owned(), "kitty".to_owned())
            ],
            "an installed well-known program found only on PATH is listed"
        );
        assert_eq!(
            found
                .installed(Kind::Editor)
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["Visual Studio Code"],
            "one row for code, though PATH and the entry name different files"
        );
        let terminals = missing_names(&found, Kind::Terminal);
        for installed in ["kitty", "Foot", "Gone term"] {
            assert!(
                !terminals.contains(&installed),
                "{installed}: {terminals:?}"
            );
        }
        for absent in ["Alacritty", "Konsole", "GNOME Terminal", "xterm", "Ghostty"] {
            assert!(terminals.contains(&absent), "{absent}: {terminals:?}");
        }
        let editors = missing_names(&found, Kind::Editor);
        assert!(!editors.contains(&"Visual Studio Code"), "{editors:?}");
        assert!(
            editors.contains(&"Zed") && editors.contains(&"Kate"),
            "{editors:?}"
        );
        assert!(
            !editors.contains(&"BBEdit") && !editors.contains(&"Notepad++"),
            "another platform's programs are not offered: {editors:?}"
        );
        // Sorted by name, and a saved command is matched to its program by what it starts.
        let sorted_names: Vec<String> = terminals.iter().map(|n| n.to_lowercase()).collect();
        assert!(sorted_names.is_sorted(), "{terminals:?}");
        let zed = found
            .missing(Kind::Editor)
            .find(|m| m.name == "Zed")
            .unwrap();
        for command in ["zed", "zed --wait %f", "zeditor"] {
            assert!(zed.is_started_by(command, false), "{command}");
        }
        #[cfg(windows)]
        assert!(zed.is_started_by("  ZED  ", false));
        #[cfg(not(windows))]
        assert!(!zed.is_started_by("  ZED  ", false));
        // A command that names its program by a path matches by that program's name only when the
        // file is gone (`program_absent`); a file that is there is the user's own program.
        for command in ["/opt/zed/bin/zed --wait %f", "./zed"] {
            assert!(!zed.is_started_by(command, false), "{command}");
            assert!(zed.is_started_by(command, true), "{command}");
        }
        #[cfg(windows)]
        {
            assert!(!zed.is_started_by("C:\\zed\\zed.exe", false));
            assert!(zed.is_started_by("C:\\zed\\zed.exe", true));
        }
        assert!(!zed.is_started_by("zedd", false) && !zed.is_started_by("", false));
        // Installing it moves it between the lists.
        program(&root.join("bin"), "zeditor");
        let found = detect(&env(root, &[]));
        assert!(!missing_names(&found, Kind::Editor).contains(&"Zed"));
        assert!(found.installed(Kind::Editor).any(|a| a.name == "Zed"));
    }

    // Every well-known program has a name, and its identifiers do not collide: one command or
    // entry belongs to one program, so a saved command names one.
    #[test]
    fn the_well_known_table_names_each_program_once() {
        let mut commands = std::collections::HashSet::new();
        let mut entries = std::collections::HashSet::new();
        for k in KNOWN {
            assert_ne!(k.name, "");
            for c in k.commands {
                assert!(commands.insert((k.kind, *c)), "{c} twice");
                assert_eq!(*c, c.to_ascii_lowercase(), "{c} is a lower-case command");
            }
            for e in k.entries {
                assert!(entries.insert((k.kind, *e)), "{e} twice");
                assert!(e.ends_with(".desktop"), "{e}");
            }
            assert!(
                !k.commands.is_empty() || k.bundle.is_some() || k.exe.is_some(),
                "{} is on no platform",
                k.name
            );
        }
        // Terminals tried as a last resort: the Debian alternative, then the table's order.
        let terminals: Vec<_> = linux_terminals().collect();
        assert_eq!(terminals[0], "x-terminal-emulator");
        assert!(terminals.contains(&"kitty") && terminals.contains(&"xterm"));
    }

    // A chosen well-known program that is no longer installed is never started: the system's
    // runs, and the fallback says which and why.
    #[test]
    fn a_chosen_program_that_is_not_installed_gives_way_to_the_systems() {
        let zed = Missing::new(Kind::Editor, "Zed", ["zed", "zeditor"]);
        let apps = Apps {
            editor: Some(SystemChoice {
                name: "Text Editor".to_owned(),
                command: Some("gnome-text-editor %f".to_owned()),
                id: None,
            }),
            missing: vec![zed],
            ..Apps::default()
        };
        let launch = |apps: Option<&Apps>, configured: &str, kind| launch(apps, configured, kind);
        assert_eq!(
            launch(Some(&apps), "zed --wait %f", Kind::Editor),
            Launch {
                command: "gnome-text-editor %f".to_owned(),
                fallback: Some(Fallback {
                    missing: "Zed".to_owned(),
                    used: "Text Editor".to_owned()
                }),
            }
        );
        // Without a system choice the platform's own opener runs (an empty command).
        let bare = Apps {
            missing: apps.missing.clone(),
            ..Apps::default()
        };
        let fallback = launch(Some(&bare), "zed", Kind::Editor);
        assert_eq!(fallback.command, "");
        assert_eq!(
            fallback.fallback.map(|f| f.used).as_deref(),
            Some("the system's own")
        );
        // Anything else runs as typed: a program that is installed, one nobody knows, another kind,
        // and before the first detection ends.
        for (apps, configured, kind) in [
            (Some(&apps), "code --wait", Kind::Editor),
            (Some(&apps), "my-editor %f", Kind::Editor),
            (Some(&apps), "zed", Kind::Terminal),
            (None, "zed", Kind::Editor),
        ] {
            let ran = launch(apps, configured, kind);
            assert_eq!(ran.command, configured);
            assert_eq!(ran.fallback, None, "{configured}");
        }
        // Nothing chosen is the system's, with no message.
        assert_eq!(
            launch(Some(&apps), "  ", Kind::Editor),
            Launch {
                command: "gnome-text-editor %f".to_owned(),
                fallback: None
            }
        );
        // macOS names its programs by `open -a`.
        let iterm = Missing::new(Kind::Terminal, "iTerm", ["iTerm"]);
        assert!(iterm.is_started_by("open -a iTerm %d", false));
        assert!(iterm.is_started_by("open -a \"iTerm\"", false));
        assert!(!iterm.is_started_by("open -a Terminal %d", false));
    }

    // A program that macOS finds on `PATH` (a Homebrew or Nix binary) is installed, not "Not
    // installed", so a saved `wezterm start --cwd %d` is never swapped for the system's terminal;
    // one nowhere to be found by its bundle or its command still is.
    #[test]
    fn a_mac_program_on_path_is_installed_and_one_nowhere_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        program(&bin, "wezterm");
        let path = [bin];
        let bundles = ["Photos"].map(str::to_owned);
        let apps = mac_apps(&bundles, &path);
        let wezterm = apps
            .iter()
            .find(|a| a.name == "WezTerm")
            .expect("WezTerm on PATH is listed as installed");
        assert_eq!(wezterm.command, "wezterm");
        let missing = mac_missing(&bundles, &path);
        assert!(missing.iter().all(|m| m.name != "WezTerm"), "{missing:?}");
        let apps = Apps {
            missing,
            ..Apps::default()
        };
        assert_eq!(
            launch(Some(&apps), "wezterm start --cwd %d", Kind::Terminal).fallback,
            None
        );
        // Not on PATH and no bundle: missing, by its bundle name and by its command.
        let missing = mac_missing(&bundles, &[]);
        let wezterm = missing.iter().find(|m| m.name == "WezTerm").unwrap();
        assert!(
            wezterm.is_started_by("wezterm start", false)
                && wezterm.is_started_by("open -a WezTerm", false)
        );
        // Its bundle can be anywhere Spotlight looks: names come from the `mdfind` paths.
        let names = bundles_from_mdfind(
            "/Users/me/Downloads/WezTerm.app\n/opt/homebrew/Caskroom/zed/Zed.app\nnot-an-app\n",
        );
        assert_eq!(names, ["WezTerm", "Zed"]);
        assert!(
            mac_missing(&names, &[])
                .iter()
                .all(|m| m.name != "WezTerm" && m.name != "Zed")
        );
    }

    // Which saved commands name an absolute path to a file that is not there: a file that exists
    // (also through a link) is there; a broken link, a folder or a missing file is gone; a quoted
    // path with spaces is read whole; `~` and relative paths are never judged.
    #[test]
    fn a_saved_command_is_absent_only_for_an_absolute_path_to_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let spaced = root.join("with space");
        write(&spaced, "code", "#!/bin/sh\n");
        write(root, "real", "#!/bin/sh\n");
        std::fs::create_dir(root.join("folder")).unwrap();
        let quoted = |p: &Path| format!("\"{}\" --wait %f", p.display());
        let mut saved = vec![
            quoted(&spaced.join("code")),
            quoted(&spaced.join("gone")),
            format!("{} %f", root.join("folder").display()),
            format!("{} %f", root.join("missing").display()),
            "~/bin/code %f".to_owned(),
            "./code %f".to_owned(),
            "bin/code %f".to_owned(),
            "code %f".to_owned(),
            String::new(),
        ];
        let mut absent = vec![
            quoted(&spaced.join("gone")),
            format!("{} %f", root.join("folder").display()),
            format!("{} %f", root.join("missing").display()),
        ];
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
            std::os::unix::fs::symlink(root.join("nowhere"), root.join("broken")).unwrap();
            saved.push(format!("{} %f", root.join("link").display()));
            saved.push(format!("  {}  ", root.join("broken").display()));
            absent.push(format!("{}", root.join("broken").display()));
        }
        assert_eq!(absent_commands(&saved), absent);
    }

    // A command saved from a desktop entry names its program by a path (`/usr/share/code/code`).
    // When that file is gone the program is not installed, so the system's runs and the fallback
    // says so; a file that is there, installed anywhere, is the user's own program and runs as
    // typed, as does a path that nothing known is called by.
    #[test]
    fn a_saved_path_is_a_gone_program_only_when_its_file_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let (gone, there) = (
            dir.path().join("code/code"),
            dir.path().join("alacritty/bin"),
        );
        program(&there, "alacritty");
        let there = there.join("alacritty");
        let saved = |program: &Path| format!("{} --unity-launch %F", program.display());
        let (gone_command, there_command) = (saved(&gone), saved(&there));
        let apps = Apps {
            editor: Some(SystemChoice {
                name: "Text Editor".to_owned(),
                command: Some("gnome-text-editor %f".to_owned()),
                id: None,
            }),
            missing: vec![
                Missing::new(Kind::Editor, "Visual Studio Code", ["code"]),
                Missing::new(Kind::Editor, "Alacritty", ["alacritty"]),
            ],
            absent: absent_commands(&[
                gone_command.clone(),
                format!(" {there_command} "),
                "/usr/share/nothing/else".to_owned(),
                "code".to_owned(),
            ]),
            ..Apps::default()
        };
        assert_eq!(
            apps.absent,
            [gone_command.clone(), "/usr/share/nothing/else".to_owned()],
            "only a path to a missing file is recorded"
        );
        let ran = launch(Some(&apps), &gone_command, Kind::Editor);
        assert_eq!(ran.command, "gnome-text-editor %f");
        assert_eq!(
            ran.fallback.map(|f| (f.missing, f.used)),
            Some(("Visual Studio Code".to_owned(), "Text Editor".to_owned()))
        );
        // Padding around the saved command does not hide it.
        assert!(apps.is_absent(&format!("  {gone_command}")));
        // The file exists: Alacritty is "missing" by name, but this program is there.
        let ran = launch(Some(&apps), &there_command, Kind::Editor);
        assert_eq!(ran.command, there_command);
        assert_eq!(ran.fallback, None);
        // Before any detection nothing is replaced.
        assert_eq!(
            launch(None, &gone_command, Kind::Editor).command,
            gone_command
        );
    }

    #[test]
    fn mac_and_windows_programs_are_recognised() {
        let bundles = ["Zed", "iTerm", "Photos", "Finder"].map(str::to_owned);
        let mac = mac_apps(&bundles, &[]);
        assert_eq!(
            mac.iter()
                .map(|a| (a.kind, a.command.as_str()))
                .collect::<Vec<_>>(),
            [
                (Kind::FileManager, "open -a Finder %d"),
                (Kind::Terminal, "open -a iTerm %d"),
                (Kind::Editor, "open -a Zed"),
            ]
        );
        let missing = mac_missing(&bundles, &[]);
        let names: Vec<_> = missing.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"Visual Studio Code") && names.contains(&"Ghostty"));
        assert!(
            !names.contains(&"Zed") && !names.contains(&"iTerm") && !names.contains(&"Finder"),
            "{names:?}"
        );
        assert!(
            !names.contains(&"Notepad++"),
            "Windows programs are not offered on macOS"
        );
        let code = missing
            .iter()
            .find(|m| m.name == "Visual Studio Code")
            .unwrap();
        assert!(code.is_started_by("open -a \"Visual Studio Code\"", false));
        let reg = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\Code.exe\r\n    (Default)    REG_SZ    C:\\Users\\me\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe\r\n    Path    REG_SZ    C:\\Users\\me\\AppData\\Local\\Programs\\Microsoft VS Code\r\n\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\wt.exe\r\n    (Default)    REG_EXPAND_SZ    \"%LOCALAPPDATA%\\Microsoft\\WindowsApps\\wt.exe\"\r\n\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\other.exe\r\n";
        let paths = parse_app_paths(reg);
        assert_eq!(paths.len(), 2, "{paths:?}");
        let windows = windows_apps(&paths);
        assert_eq!(
            windows
                .iter()
                .map(|a| (a.kind, a.command.as_str()))
                .collect::<Vec<_>>(),
            [
                (
                    Kind::Editor,
                    "\"C:\\Users\\me\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe\""
                ),
                (
                    Kind::Terminal,
                    "%LOCALAPPDATA%\\Microsoft\\WindowsApps\\wt.exe -d %d"
                ),
            ]
        );
    }
}
