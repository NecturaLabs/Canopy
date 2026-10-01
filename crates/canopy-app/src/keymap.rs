//! Every keyboard shortcut, in one table that both dispatches keys and renders the cheatsheet,
//! so the documentation cannot drift from the behaviour. See `docs/keybindings.md`.
//!
//! In the spirit of Omarchy and lazygit: a launcher-style palette, a cheatsheet one key away,
//! vim motions and single-key actions. Two rules: Super is never bound (it belongs to the window
//! manager), and bare keys never fire while a text field has focus.

use egui::{Key, Modifiers};

/// Something a key can do. The app turns commands into actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Palette,
    Cheatsheet,
    Dismiss,
    Quit,
    Open,
    Clone,
    CloseTab,
    NextTab,
    PrevTab,
    /// Jump to tab N (the matched chord's index, 0-based).
    JumpTab,
    Refresh,
    Fetch,
    Pull,
    Push,
    NewBranch,
    Stash,
    PopStash,
    Commit,
    Filter,
    Settings,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    History,
    Changes,
    Requests,
    Down,
    Up,
    PageDown,
    PageUp,
    First,
    Last,
    MergeDialog,
    Tag,
    FocusCommit,
    ToggleStage,
    StageAll,
    UnstageAll,
    Discard,
    CopySha,
    OpenOnForge,
    NextConflict,
    PrevConflict,
    TakeOurs,
    TakeTheirs,
    TakeBoth,
    TakeBothTheirsFirst,
    TakeBase,
    EditConflict,
    ClearChoice,
}

/// A key with modifiers. `ctrl` is Ctrl on Linux and Windows and Cmd on macOS (where the
/// Control key is accepted for it too).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    /// The key is a character (`?`, `+`, `[`) whatever modifiers the layout needs to type it:
    /// Shift and AltGr are not checked.
    pub symbol: bool,
}

const fn bare(key: Key) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: false,
        symbol: false,
    }
}

const fn shifted(key: Key) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: true,
        symbol: false,
    }
}

const fn symbol(key: Key) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: false,
        symbol: true,
    }
}

const fn ctrl(key: Key) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: false,
        symbol: false,
    }
}

const fn ctrl_shift(key: Key) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: true,
        symbol: false,
    }
}

const fn ctrl_symbol(key: Key) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: false,
        symbol: true,
    }
}

/// Where a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Works everywhere, also while typing.
    Anywhere,
    /// Moving through lists and switching views. Like every section below it, only when no text
    /// field has focus.
    Navigate,
    History,
    Changes,
    Resolve,
    Repository,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Anywhere,
        Section::Navigate,
        Section::History,
        Section::Changes,
        Section::Resolve,
        Section::Repository,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Anywhere => "Anywhere",
            Section::Navigate => "Lists and views",
            Section::History => "History",
            Section::Changes => "Changes",
            Section::Resolve => "Resolving a conflicted file",
            Section::Repository => "Repository actions",
        }
    }

    /// Bindings of this section do not fire while a text field has focus.
    pub fn is_bare(self) -> bool {
        self != Section::Anywhere
    }
}

/// One row of the table.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub command: Command,
    pub chords: &'static [Chord],
    /// What the cheatsheet shows for the keys; derived from `chords` when empty.
    pub keys_label: &'static str,
    pub description: &'static str,
    pub section: Section,
    /// Fires again while the key is held.
    pub repeat: bool,
    /// The first chord must be pressed twice in a row (vim's `gg`).
    pub double: bool,
}

const fn b(
    section: Section,
    command: Command,
    chords: &'static [Chord],
    description: &'static str,
) -> Binding {
    Binding {
        command,
        chords,
        keys_label: "",
        description,
        section,
        repeat: false,
        double: false,
    }
}

/// A binding that repeats while the key is held (motions).
const fn held(
    section: Section,
    command: Command,
    chords: &'static [Chord],
    description: &'static str,
) -> Binding {
    Binding {
        repeat: true,
        ..b(section, command, chords, description)
    }
}

const JUMP: [Chord; 9] = [
    ctrl(Key::Num1),
    ctrl(Key::Num2),
    ctrl(Key::Num3),
    ctrl(Key::Num4),
    ctrl(Key::Num5),
    ctrl(Key::Num6),
    ctrl(Key::Num7),
    ctrl(Key::Num8),
    ctrl(Key::Num9),
];

use Command as C;
use Section as S;

/// The keymap. Chords follow, in order of precedence: Omarchy's layering (one modifier for the
/// app's global actions, `?`-style help one key away), lazygit's single keys for Git actions,
/// vim's motions, and Fork's Ctrl+Shift chords for the same actions while typing.
pub const BINDINGS: &[Binding] = &[
    // Anywhere.
    b(
        S::Anywhere,
        C::Palette,
        &[ctrl(Key::K), ctrl(Key::P), ctrl(Key::Space)],
        "Open the command palette",
    ),
    b(
        S::Anywhere,
        C::Cheatsheet,
        &[ctrl(Key::Slash)],
        "Show keyboard shortcuts",
    ),
    b(
        S::Anywhere,
        C::Open,
        &[ctrl(Key::O), ctrl(Key::T)],
        "Open a repository in a new tab",
    ),
    b(
        S::Anywhere,
        C::Clone,
        &[ctrl_shift(Key::N)],
        "Clone a repository",
    ),
    b(
        S::Anywhere,
        C::CloseTab,
        &[ctrl(Key::W)],
        "Close the tab (not while typing)",
    ),
    b(
        S::Anywhere,
        C::PrevTab,
        &[ctrl_shift(Key::Tab)],
        "Previous tab",
    ),
    b(S::Anywhere, C::NextTab, &[ctrl(Key::Tab)], "Next tab"),
    Binding {
        keys_label: "Ctrl+1…9",
        ..b(S::Anywhere, C::JumpTab, &JUMP, "Go to tab 1–9")
    },
    b(
        S::Anywhere,
        C::Refresh,
        &[bare(Key::F5), ctrl(Key::R)],
        "Refresh",
    ),
    b(S::Anywhere, C::Fetch, &[ctrl_shift(Key::F)], "Fetch"),
    b(S::Anywhere, C::Pull, &[ctrl_shift(Key::L)], "Pull"),
    b(S::Anywhere, C::Push, &[ctrl_shift(Key::P)], "Push"),
    b(
        S::Anywhere,
        C::Stash,
        &[ctrl_shift(Key::H)],
        "Stash changes",
    ),
    b(
        S::Anywhere,
        C::StageAll,
        &[ctrl_shift(Key::S)],
        "Stage all changes",
    ),
    b(
        S::Anywhere,
        C::NewBranch,
        &[ctrl(Key::B)],
        "Create a branch",
    ),
    b(
        S::Anywhere,
        C::Commit,
        &[ctrl(Key::Enter)],
        "Commit staged changes",
    ),
    b(
        S::Anywhere,
        C::Filter,
        &[ctrl(Key::F)],
        "Toggle history search",
    ),
    b(
        S::Anywhere,
        C::Settings,
        &[ctrl(Key::Comma)],
        "Open settings",
    ),
    b(
        S::Anywhere,
        C::ZoomIn,
        &[ctrl_symbol(Key::Equals), ctrl_symbol(Key::Plus)],
        "Zoom in",
    ),
    b(S::Anywhere, C::ZoomOut, &[ctrl(Key::Minus)], "Zoom out"),
    b(S::Anywhere, C::ZoomReset, &[ctrl(Key::Num0)], "Reset zoom"),
    b(S::Anywhere, C::Quit, &[ctrl(Key::Q)], "Quit Canopy"),
    b(
        S::Anywhere,
        C::Dismiss,
        &[bare(Key::Escape)],
        "Close the dialog, palette, cheatsheet or search",
    ),
    // Lists and views.
    b(
        S::Navigate,
        C::Cheatsheet,
        &[symbol(Key::Questionmark)],
        "Show keyboard shortcuts",
    ),
    b(S::Navigate, C::History, &[bare(Key::Num1)], "Go to History"),
    b(S::Navigate, C::Changes, &[bare(Key::Num2)], "Go to Changes"),
    b(
        S::Navigate,
        C::Requests,
        &[bare(Key::Num3)],
        "Go to pull / merge requests",
    ),
    held(
        S::Navigate,
        C::Down,
        &[bare(Key::J), bare(Key::ArrowDown)],
        "Next commit, file or request",
    ),
    held(
        S::Navigate,
        C::Up,
        &[bare(Key::K), bare(Key::ArrowUp)],
        "Previous commit, file or request",
    ),
    held(
        S::Navigate,
        C::PageDown,
        &[bare(Key::PageDown), ctrl(Key::D)],
        "Down a page",
    ),
    held(
        S::Navigate,
        C::PageUp,
        &[bare(Key::PageUp), ctrl(Key::U)],
        "Up a page",
    ),
    Binding {
        double: true,
        keys_label: "gg  ·  Home",
        ..b(
            S::Navigate,
            C::First,
            &[bare(Key::G), bare(Key::Home)],
            "Go to the top",
        )
    },
    b(
        S::Navigate,
        C::Last,
        &[shifted(Key::G), bare(Key::End)],
        "Go to the bottom",
    ),
    b(
        S::Navigate,
        C::Filter,
        &[symbol(Key::Slash)],
        "Toggle history search",
    ),
    b(S::Navigate, C::Refresh, &[shifted(Key::R)], "Refresh"),
    // History.
    b(
        S::History,
        C::Tag,
        &[shifted(Key::T)],
        "Tag the selected commit",
    ),
    b(
        S::History,
        C::CopySha,
        &[bare(Key::Y)],
        "Copy the selected commit's SHA",
    ),
    b(
        S::History,
        C::OpenOnForge,
        &[bare(Key::O)],
        "Open the commit (or request) on GitHub / GitLab",
    ),
    // Changes.
    b(
        S::Changes,
        C::FocusCommit,
        &[bare(Key::C)],
        "Write the commit message",
    ),
    b(
        S::Changes,
        C::ToggleStage,
        &[bare(Key::Space)],
        "Stage or unstage the selected file",
    ),
    b(
        S::Changes,
        C::StageAll,
        &[bare(Key::A)],
        "Stage all changes",
    ),
    b(
        S::Changes,
        C::UnstageAll,
        &[bare(Key::U)],
        "Unstage all changes",
    ),
    b(
        S::Changes,
        C::Discard,
        &[bare(Key::D)],
        "Discard the selected file (asks first)",
    ),
    // Conflict resolver.
    b(
        S::Resolve,
        C::NextConflict,
        &[symbol(Key::CloseBracket)],
        "Next conflict",
    ),
    b(
        S::Resolve,
        C::PrevConflict,
        &[symbol(Key::OpenBracket)],
        "Previous conflict",
    ),
    b(S::Resolve, C::TakeOurs, &[bare(Key::H)], "Take ours (left)"),
    b(
        S::Resolve,
        C::TakeTheirs,
        &[bare(Key::L)],
        "Take theirs (right)",
    ),
    b(
        S::Resolve,
        C::TakeBoth,
        &[bare(Key::B)],
        "Take both, ours first",
    ),
    b(
        S::Resolve,
        C::TakeBothTheirsFirst,
        &[shifted(Key::B)],
        "Take both, theirs first",
    ),
    b(
        S::Resolve,
        C::TakeBase,
        &[shifted(Key::A)],
        "Take the common ancestor",
    ),
    b(
        S::Resolve,
        C::EditConflict,
        &[bare(Key::E)],
        "Edit the conflict by hand",
    ),
    b(
        S::Resolve,
        C::ClearChoice,
        &[bare(Key::Z)],
        "Undo the choice for this conflict",
    ),
    // Repository actions.
    b(S::Repository, C::Fetch, &[bare(Key::F)], "Fetch"),
    b(S::Repository, C::Pull, &[bare(Key::P)], "Pull"),
    b(S::Repository, C::Push, &[shifted(Key::P)], "Push"),
    b(
        S::Repository,
        C::NewBranch,
        &[bare(Key::N)],
        "Create a branch",
    ),
    b(
        S::Repository,
        C::MergeDialog,
        &[bare(Key::M)],
        "Merge or rebase",
    ),
    b(S::Repository, C::Stash, &[bare(Key::S)], "Stash changes"),
    b(
        S::Repository,
        C::PopStash,
        &[shifted(Key::S)],
        "Pop the latest stash",
    ),
];

/// The keys a focused button, checkbox or list keeps for itself: Space and Enter activate it,
/// the arrows move within it.
pub fn widget_key(key: Key) -> bool {
    matches!(
        key,
        Key::Space | Key::Enter | Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight
    )
}

fn mod_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    }
}

fn key_name(key: Key) -> &'static str {
    match key {
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Escape => "Esc",
        Key::Tab => "Tab",
        Key::ArrowDown => "↓",
        Key::ArrowUp => "↑",
        Key::PageDown => "PgDn",
        Key::PageUp => "PgUp",
        Key::Home => "Home",
        Key::End => "End",
        Key::Slash => "/",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Questionmark => "?",
        Key::Comma => ",",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::Minus => "−",
        other => other.symbol_or_name(),
    }
}

impl Chord {
    /// Human-readable form, e.g. `Ctrl+Shift+F` or `G` for Shift+G.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str(mod_label());
            s.push('+');
        }
        let name = key_name(self.key);
        let is_letter = name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic());
        if self.shift && is_letter && !self.ctrl {
            // A bare Shift+letter reads as the capital letter, lazygit style.
            s.push_str(name);
            return s;
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if is_letter && !self.ctrl {
            s.push_str(&name.to_lowercase());
        } else {
            s.push_str(name);
        }
        s
    }

    fn matches(&self, key: Key, m: &Modifiers) -> bool {
        if key != self.key {
            return false;
        }
        // Ctrl on Linux and Windows; Cmd, or the Control key, on macOS.
        let ctrl = m.command || m.ctrl;
        if self.symbol {
            // egui cannot tell AltGr from Alt: AltGr arrives as Alt, and on Windows as Ctrl+Alt.
            // So Alt alone, and on Windows Ctrl+Alt too, may just be how the layout types the
            // character (`[` on German and Nordic layouts). Such a press matches the plain
            // symbol chord and never a Ctrl chord; Ctrl+Alt elsewhere counts as Ctrl.
            let typed_with_altgr = m.alt && (!ctrl || cfg!(windows));
            return if typed_with_altgr {
                !self.ctrl
            } else {
                ctrl == self.ctrl
            };
        }
        !m.alt && ctrl == self.ctrl && m.shift == self.shift
    }
}

impl Binding {
    /// Keys as the cheatsheet and menus show them, e.g. `Ctrl+K  ·  Ctrl+Shift+P`.
    pub fn keys(&self) -> String {
        if !self.keys_label.is_empty() {
            // Written with "Ctrl"; macOS says Cmd.
            return self
                .keys_label
                .replace("Ctrl+", &format!("{}+", mod_label()));
        }
        self.chords
            .iter()
            .map(Chord::label)
            .collect::<Vec<_>>()
            .join("  ·  ")
    }
}

/// The keys for `command`, as a hint for menus, tooltips and the palette: the chord that works
/// everywhere, then the single key that works outside text fields ("Ctrl+Shift+F · f").
pub fn hint(command: Command) -> String {
    let first = |bare: bool| {
        BINDINGS
            .iter()
            .find(|b| b.command == command && b.section.is_bare() == bare)
            .and_then(|b| {
                if b.keys_label.is_empty() {
                    b.chords.first().map(Chord::label)
                } else {
                    Some(
                        b.keys_label
                            .split("  ·  ")
                            .next()
                            .unwrap_or_default()
                            .to_owned(),
                    )
                }
            })
    };
    [first(false), first(true)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A command that fired, with the index of the chord that matched (for [`Command::JumpTab`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fired {
    pub command: Command,
    pub chord: usize,
}

/// Every key for `command` across sections, the always-available chords first, for the
/// cheatsheet's one row per command.
pub fn all_keys(command: Command) -> Vec<String> {
    let mut bindings: Vec<&Binding> = BINDINGS.iter().filter(|b| b.command == command).collect();
    bindings.sort_by_key(|b| b.section.is_bare());
    bindings
        .into_iter()
        .flat_map(|b| {
            b.keys()
                .split("  ·  ")
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether the cheatsheet lists `binding`'s command under its section: a command with a bare
/// key is listed once, where that key works; its global chord is shown in the same row.
pub fn listed_here(binding: &Binding) -> bool {
    binding.section.is_bare()
        || !BINDINGS
            .iter()
            .any(|b| b.command == binding.command && b.section.is_bare())
}

/// How long the second key of a double press (`gg`) may take.
const DOUBLE_PRESS_SECONDS: f64 = 1.0;

/// Consumes this frame's key presses that match a binding that `active` allows for the pressed
/// chord. The caller turns bare-key sections off while a text field has focus, keeps the keys a
/// focused widget needs, and turns most bindings off while a dialog is open; keys of inactive
/// bindings reach the widgets untouched.
pub fn dispatch(ctx: &egui::Context, active: impl Fn(&Binding, &Chord) -> bool) -> Vec<Fired> {
    let mut fired = Vec::new();
    // Outside macOS egui's modifiers do not carry Super, so a window-manager shortcut such as
    // Super+H would also arrive as a bare `h`. Super belongs to the window manager: while it is
    // held nothing fires. (egui forgets held keys when the window loses focus, so a release the
    // window manager swallowed cannot leave the keyboard dead.)
    if !cfg!(target_os = "macos") && super_held(ctx) {
        return fired;
    }
    let pending_id = egui::Id::new("keymap-double-press");
    let mut pending: Option<(Key, f64)> = ctx.data(|d| d.get_temp(pending_id));
    ctx.input_mut(|i| {
        let now = i.time;
        i.events.retain(|e| {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = e
            else {
                return true;
            };
            let waiting = pending.take();
            for binding in BINDINGS {
                if *repeat && !binding.repeat {
                    continue;
                }
                let Some(index) = binding
                    .chords
                    .iter()
                    .position(|c| c.matches(*key, modifiers))
                else {
                    continue;
                };
                if !active(binding, &binding.chords[index]) {
                    continue;
                }
                if binding.double && index == 0 {
                    let second =
                        waiting.is_some_and(|(k, t)| k == *key && now - t <= DOUBLE_PRESS_SECONDS);
                    if !second {
                        // The first press only arms the binding.
                        pending = Some((*key, now));
                        return false;
                    }
                }
                fired.push(Fired {
                    command: binding.command,
                    chord: index,
                });
                return false;
            }
            true
        });
    });
    ctx.data_mut(|d| match pending {
        Some(p) => {
            d.insert_temp(pending_id, p);
        }
        None => d.remove::<(Key, f64)>(pending_id),
    });
    // A bare key also produces a text event; drop those so a widget focused by the command
    // (the filter box after `/`) does not receive the character.
    if fired.iter().any(|f| {
        BINDINGS
            .iter()
            .any(|b| b.command == f.command && b.section.is_bare())
    }) || pending.is_some()
    {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
    }
    fired
}

fn super_held(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.key_down(Key::SuperLeft) || i.key_down(Key::SuperRight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_naturally() {
        assert_eq!(
            ctrl_shift(Key::F).label(),
            format!("{}+Shift+F", mod_label())
        );
        assert_eq!(shifted(Key::G).label(), "G");
        assert_eq!(bare(Key::G).label(), "g");
        assert_eq!(symbol(Key::Questionmark).label(), "?");
        assert_eq!(bare(Key::Space).label(), "Space");
    }

    #[test]
    fn no_chord_is_bound_twice_in_overlapping_scopes() {
        let mut seen: Vec<(Chord, bool)> = Vec::new();
        for b in BINDINGS {
            for c in b.chords {
                let bare = b.section.is_bare();
                assert!(
                    !seen
                        .iter()
                        .any(|(s, sb)| s == c && (*sb == bare || !bare || !*sb)),
                    "{} is bound twice",
                    c.label()
                );
                seen.push((*c, bare));
            }
        }
    }

    #[test]
    fn bare_keys_have_no_ctrl_and_global_keys_need_ctrl_or_function_keys() {
        for b in BINDINGS {
            for c in b.chords {
                if b.section.is_bare() {
                    // Vim's half-page motions are the one Ctrl exception, and only in lists.
                    let half_page = matches!(b.command, Command::PageDown | Command::PageUp);
                    assert!(!c.ctrl || half_page, "{} in a bare section", c.label());
                } else {
                    assert!(
                        c.ctrl || matches!(c.key, Key::F5 | Key::Escape),
                        "{} would fire while typing",
                        c.label()
                    );
                }
            }
        }
    }

    #[test]
    fn matching_respects_shift() {
        let m = Modifiers::NONE;
        assert!(bare(Key::G).matches(Key::G, &m));
        assert!(!shifted(Key::G).matches(Key::G, &m));
        assert!(shifted(Key::G).matches(Key::G, &Modifiers::SHIFT));
        assert!(symbol(Key::Questionmark).matches(Key::Questionmark, &Modifiers::SHIFT));
        assert!(!bare(Key::G).matches(Key::G, &Modifiers::ALT));
        assert!(ctrl(Key::K).matches(Key::K, &Modifiers::COMMAND));
        // `[` typed with AltGr (German, Nordic layouts) is still `[`.
        assert!(symbol(Key::OpenBracket).matches(Key::OpenBracket, &Modifiers::ALT));
        assert!(!symbol(Key::OpenBracket).matches(Key::OpenBracket, &Modifiers::COMMAND));
    }

    #[test]
    fn altgr_presses_match_plain_symbols_never_ctrl_chords() {
        let alt = Modifiers::ALT;
        let ctrl_alt = Modifiers::ALT | Modifiers::COMMAND;
        // Alt alone types the character on every platform.
        assert!(symbol(Key::Plus).matches(Key::Plus, &alt));
        assert!(!ctrl_symbol(Key::Plus).matches(Key::Plus, &alt));
        // Ctrl+Alt is AltGr on Windows only; elsewhere it is a Ctrl press.
        assert_eq!(
            symbol(Key::Plus).matches(Key::Plus, &ctrl_alt),
            cfg!(windows)
        );
        assert_eq!(
            ctrl_symbol(Key::Plus).matches(Key::Plus, &ctrl_alt),
            !cfg!(windows)
        );
    }

    #[test]
    fn every_command_with_a_hint() {
        assert!(!hint(Command::Palette).is_empty());
        assert_eq!(hint(Command::Fetch), format!("{}+Shift+F · f", mod_label()));
        assert_eq!(hint(Command::TakeOurs), "h");
        assert_eq!(hint(Command::First), "gg");
    }

    fn press(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn release(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn fire(events: Vec<egui::Event>) -> Vec<Fired> {
        let ctx = egui::Context::default();
        let mut fired = Vec::new();
        let raw = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| fired = dispatch(ui.ctx(), |_, _| true));
        output.textures_delta.clear();
        fired
    }

    #[test]
    fn super_combinations_never_reach_bare_keys() {
        assert!(!fire(vec![press(Key::J)]).is_empty());
        let held = fire(vec![press(Key::SuperLeft), press(Key::J)]);
        if cfg!(target_os = "macos") {
            assert!(!held.is_empty());
        } else {
            assert!(held.is_empty());
        }
    }

    #[test]
    fn gg_needs_two_presses() {
        assert!(fire(vec![press(Key::G)]).is_empty());
        let both = fire(vec![press(Key::G), release(Key::G), press(Key::G)]);
        assert_eq!(both.first().map(|f| f.command), Some(Command::First));
        // Held down, the key repeats: that is not a double press.
        assert!(fire(vec![press(Key::G), press(Key::G)]).is_empty());
        assert!(
            fire(vec![
                press(Key::G),
                release(Key::G),
                press(Key::J),
                press(Key::G)
            ])
            .iter()
            .all(|f| f.command != Command::First)
        );
        let home = fire(vec![press(Key::Home)]);
        assert_eq!(home.first().map(|f| f.command), Some(Command::First));
    }

    #[test]
    fn the_cheatsheet_lists_each_command_once() {
        let listed: Vec<Command> = BINDINGS
            .iter()
            .filter(|b| listed_here(b))
            .map(|b| b.command)
            .collect();
        for b in BINDINGS {
            assert_eq!(
                listed.iter().filter(|c| **c == b.command).count(),
                1,
                "{:?}",
                b.command
            );
        }
        assert_eq!(
            all_keys(Command::Fetch),
            vec![format!("{}+Shift+F", mod_label()), "f".to_owned()]
        );
    }
}
