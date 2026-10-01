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
    NextTheme,
}

/// A key with modifiers. `ctrl` is Ctrl on Linux and Windows and Cmd on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    /// Shift is part of the character (`?`, `+`) and is not checked.
    pub any_shift: bool,
}

const fn bare(key: Key) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: false,
        any_shift: false,
    }
}

const fn shifted(key: Key) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: true,
        any_shift: false,
    }
}

const fn symbol(key: Key) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: false,
        any_shift: true,
    }
}

const fn ctrl(key: Key) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: false,
        any_shift: false,
    }
}

const fn ctrl_shift(key: Key) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: true,
        any_shift: false,
    }
}

const fn ctrl_symbol(key: Key) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: false,
        any_shift: true,
    }
}

/// Where a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Works everywhere, also while typing.
    Anywhere,
    /// Bare keys, when no text field has focus.
    Navigate,
    History,
    Changes,
    Remote,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Anywhere,
        Section::Navigate,
        Section::History,
        Section::Changes,
        Section::Remote,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Anywhere => "Anywhere",
            Section::Navigate => "Navigate (no text field focused)",
            Section::History => "History",
            Section::Changes => "Changes",
            Section::Remote => "Branches and remotes",
        }
    }

    /// Bindings of this section are bare keys.
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

/// The keymap. Order matters only within a section (more specific chords first).
pub const BINDINGS: &[Binding] = &[
    // Anywhere.
    b(
        S::Anywhere,
        C::Palette,
        &[ctrl(Key::K), ctrl_shift(Key::P), ctrl(Key::Space)],
        "Command palette",
    ),
    b(
        S::Anywhere,
        C::Cheatsheet,
        &[ctrl(Key::Slash)],
        "Keyboard shortcuts",
    ),
    b(S::Anywhere, C::Open, &[ctrl(Key::O)], "Open repository"),
    b(
        S::Anywhere,
        C::Clone,
        &[ctrl_shift(Key::N)],
        "Clone repository",
    ),
    b(S::Anywhere, C::CloseTab, &[ctrl(Key::W)], "Close tab"),
    b(
        S::Anywhere,
        C::PrevTab,
        &[ctrl_shift(Key::Tab)],
        "Previous tab",
    ),
    b(S::Anywhere, C::NextTab, &[ctrl(Key::Tab)], "Next tab"),
    Binding {
        command: C::JumpTab,
        chords: &JUMP,
        keys_label: "Ctrl+1…9",
        description: "Jump to tab 1–9",
        section: S::Anywhere,
        repeat: false,
    },
    b(
        S::Anywhere,
        C::Refresh,
        &[bare(Key::F5), ctrl(Key::R)],
        "Refresh",
    ),
    b(S::Anywhere, C::Fetch, &[ctrl_shift(Key::F)], "Fetch"),
    b(S::Anywhere, C::Pull, &[ctrl_shift(Key::L)], "Pull"),
    b(S::Anywhere, C::Push, &[ctrl_shift(Key::K)], "Push"),
    b(
        S::Anywhere,
        C::Stash,
        &[ctrl_shift(Key::S)],
        "Stash changes",
    ),
    b(S::Anywhere, C::NewBranch, &[ctrl(Key::B)], "New branch"),
    b(
        S::Anywhere,
        C::Commit,
        &[ctrl(Key::Enter)],
        "Commit (in Changes)",
    ),
    b(S::Anywhere, C::Filter, &[ctrl(Key::F)], "Filter history"),
    b(S::Anywhere, C::Settings, &[ctrl(Key::Comma)], "Settings"),
    b(
        S::Anywhere,
        C::ZoomIn,
        &[ctrl_symbol(Key::Equals), ctrl_symbol(Key::Plus)],
        "Zoom in",
    ),
    b(S::Anywhere, C::ZoomOut, &[ctrl(Key::Minus)], "Zoom out"),
    b(S::Anywhere, C::ZoomReset, &[ctrl(Key::Num0)], "Reset zoom"),
    b(
        S::Anywhere,
        C::Dismiss,
        &[bare(Key::Escape)],
        "Close dialog, palette or cheatsheet",
    ),
    // Navigate.
    b(
        S::Navigate,
        C::Cheatsheet,
        &[symbol(Key::Questionmark)],
        "Keyboard shortcuts",
    ),
    b(S::Navigate, C::History, &[bare(Key::Num1)], "History"),
    b(S::Navigate, C::Changes, &[bare(Key::Num2)], "Changes"),
    b(
        S::Navigate,
        C::Requests,
        &[bare(Key::Num3)],
        "Pull / merge requests",
    ),
    Binding {
        command: C::Down,
        chords: &[bare(Key::J), bare(Key::ArrowDown)],
        keys_label: "",
        description: "Next commit, file or request",
        section: S::Navigate,
        repeat: true,
    },
    Binding {
        command: C::Up,
        chords: &[bare(Key::K), bare(Key::ArrowUp)],
        keys_label: "",
        description: "Previous commit, file or request",
        section: S::Navigate,
        repeat: true,
    },
    b(S::Navigate, C::Refresh, &[shifted(Key::R)], "Refresh"),
    b(S::Navigate, C::NextTheme, &[shifted(Key::T)], "Next theme"),
    // History.
    b(S::History, C::First, &[bare(Key::G)], "First commit"),
    b(
        S::History,
        C::Last,
        &[shifted(Key::G)],
        "Last loaded commit",
    ),
    b(
        S::History,
        C::Filter,
        &[symbol(Key::Slash)],
        "Filter history",
    ),
    b(S::History, C::Tag, &[bare(Key::T)], "Tag selected commit"),
    b(S::History, C::CopySha, &[bare(Key::Y)], "Copy selected SHA"),
    b(
        S::History,
        C::OpenOnForge,
        &[bare(Key::O)],
        "Open commit (or request) on GitHub / GitLab",
    ),
    // Changes.
    b(
        S::Changes,
        C::FocusCommit,
        &[bare(Key::C)],
        "Write commit message",
    ),
    b(
        S::Changes,
        C::ToggleStage,
        &[bare(Key::Space)],
        "Stage / unstage selected file",
    ),
    b(S::Changes, C::StageAll, &[bare(Key::A)], "Stage all"),
    b(S::Changes, C::UnstageAll, &[bare(Key::U)], "Unstage all"),
    b(
        S::Changes,
        C::Discard,
        &[bare(Key::D)],
        "Discard selected file (asks first)",
    ),
    // Remote and branches.
    b(S::Remote, C::Fetch, &[bare(Key::F)], "Fetch"),
    b(S::Remote, C::Pull, &[bare(Key::P)], "Pull"),
    b(S::Remote, C::Push, &[shifted(Key::P)], "Push"),
    b(S::Remote, C::NewBranch, &[bare(Key::B)], "New branch"),
    b(
        S::Remote,
        C::MergeDialog,
        &[bare(Key::M)],
        "Merge or rebase",
    ),
    b(S::Remote, C::Stash, &[bare(Key::S)], "Stash changes"),
    b(
        S::Remote,
        C::PopStash,
        &[shifted(Key::S)],
        "Pop latest stash",
    ),
];

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
        Key::Slash => "/",
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
        if key != self.key || m.alt {
            return false;
        }
        let ctrl = m.command || m.ctrl;
        ctrl == self.ctrl && (self.any_shift || m.shift == self.shift)
    }
}

impl Binding {
    /// Keys as the cheatsheet and menus show them, e.g. `Ctrl+K  ·  Ctrl+Shift+P`.
    pub fn keys(&self) -> String {
        if !self.keys_label.is_empty() {
            return self.keys_label.to_owned();
        }
        self.chords
            .iter()
            .map(Chord::label)
            .collect::<Vec<_>>()
            .join("  ·  ")
    }
}

/// The first chord bound to `command`, as a hint for menus, tooltips and the palette
/// ("Ctrl+Shift+F"). Prefers the always-available chord over a bare key.
pub fn hint(command: Command) -> String {
    BINDINGS
        .iter()
        .filter(|b| b.command == command)
        .min_by_key(|b| b.section.is_bare())
        .and_then(|b| {
            if b.keys_label.is_empty() {
                b.chords.first().map(Chord::label)
            } else {
                Some(b.keys_label.to_owned())
            }
        })
        .unwrap_or_default()
}

/// A command that fired, with the index of the chord that matched (for [`Command::JumpTab`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fired {
    pub command: Command,
    pub chord: usize,
}

/// Consumes this frame's key presses that match a binding that `active` allows. The caller turns
/// bare-key sections off while a text field has focus, and most bindings off while a dialog is
/// open; keys of inactive bindings reach the widgets untouched.
pub fn dispatch(ctx: &egui::Context, active: impl Fn(&Binding) -> bool) -> Vec<Fired> {
    let mut fired = Vec::new();
    ctx.input_mut(|i| {
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
            for binding in BINDINGS {
                if !active(binding) {
                    continue;
                }
                if *repeat && !binding.repeat {
                    continue;
                }
                if let Some(chord) = binding
                    .chords
                    .iter()
                    .position(|c| c.matches(*key, modifiers))
                {
                    fired.push(Fired {
                        command: binding.command,
                        chord,
                    });
                    return false;
                }
            }
            true
        });
    });
    // A bare key also produces a text event; drop those so a widget focused by the command
    // (the filter box after `/`) does not receive the character.
    if fired.iter().any(|f| {
        BINDINGS
            .iter()
            .any(|b| b.command == f.command && b.section.is_bare())
    }) {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
    }
    fired
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
                    assert!(!c.ctrl, "{} in a bare section", c.label());
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
    }

    #[test]
    fn every_command_with_a_hint() {
        assert!(!hint(Command::Palette).is_empty());
        assert_eq!(hint(Command::Fetch), format!("{}+Shift+F", mod_label()));
    }
}
