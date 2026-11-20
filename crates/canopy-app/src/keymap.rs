//! Every keyboard shortcut, in one table that both dispatches keys and renders the cheatsheet,
//! so the documentation cannot drift from the behaviour. See `docs/keybindings.md`.
//!
//! In the spirit of Omarchy and lazygit: a launcher-style palette, a cheatsheet one key away,
//! vim motions and single-key actions. Omarchy gives every layer its own modifiers and never
//! lets a lower layer use a higher one's: Super is the window manager's, Alt and Ctrl+Alt belong
//! to window cycling and tmux, Print, F9 and the media keys to the system. Canopy lives in the
//! Ctrl layer, so Super, Alt and those keys are never bound, and bare keys never fire while a
//! text field has focus. In a family Shift turns the action around (`p` pull, `P` push).

use std::cell::RefCell;
use std::collections::BTreeMap;

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
    Init,
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
    ApproveRequest,
    MergeRequest,
    Down,
    Up,
    PageDown,
    PageUp,
    First,
    Last,
    MergeDialog,
    Tag,
    SearchChanges,
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
    /// Shift+F10 on a focused row: its context menu. The row opens it itself
    /// (`ui/kit/menu.rs`); the table lists the key and never dispatches it.
    ContextMenu,
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
    pub const ALL: [Self; 6] = [
        Self::Anywhere,
        Self::Navigate,
        Self::History,
        Self::Changes,
        Self::Resolve,
        Self::Repository,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Anywhere => "Anywhere",
            Self::Navigate => "Lists and views",
            Self::History => "History",
            Self::Changes => "Changes",
            Self::Resolve => "Resolving a conflicted file",
            Self::Repository => "Repository actions",
        }
    }

    /// Bindings of this section do not fire while a text field has focus.
    pub fn is_bare(self) -> bool {
        self != Self::Anywhere
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
        C::Init,
        &[ctrl_shift(Key::I)],
        "Create a new repository",
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
        C::ApproveRequest,
        &[ctrl_shift(Key::A)],
        "Approve the selected request",
    ),
    b(
        S::Anywhere,
        C::MergeRequest,
        &[ctrl_shift(Key::M)],
        "Merge the selected request (asks first)",
    ),
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
        "Search History, or filter the requests",
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
        "Search History, or filter the requests",
    ),
    b(S::Navigate, C::Refresh, &[shifted(Key::R)], "Refresh"),
    b(
        S::Navigate,
        C::ContextMenu,
        &[shifted(Key::F10)],
        "Open the menu of the focused row",
    ),
    // History.
    b(
        S::History,
        C::Tag,
        &[shifted(Key::T)],
        "Tag the selected commit",
    ),
    b(
        S::History,
        C::SearchChanges,
        &[shifted(Key::F)],
        "Search the history for changed text",
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
    b(
        S::Resolve,
        C::TakeOurs,
        &[bare(Key::H)],
        "Accept current version (left)",
    ),
    b(
        S::Resolve,
        C::TakeTheirs,
        &[bare(Key::L)],
        "Accept incoming version (right)",
    ),
    b(
        S::Resolve,
        C::TakeBoth,
        &[bare(Key::B)],
        "Accept both, current first",
    ),
    b(
        S::Resolve,
        C::TakeBothTheirsFirst,
        &[shifted(Key::B)],
        "Accept both, incoming first",
    ),
    b(
        S::Resolve,
        C::TakeBase,
        &[shifted(Key::A)],
        "Accept the common ancestor",
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
        Key::Comma => "Comma",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::Minus => "−",
        other => other.symbol_or_name(),
    }
}

impl Chord {
    /// Human-readable form, e.g. `Ctrl+Shift+F` or `G` for Shift+G.
    pub fn label(self) -> String {
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

    fn matches(self, key: Key, m: Modifiers) -> bool {
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
    /// The chords that fire this binding: the ones the user chose in Settings, else the table's.
    pub fn current_chords(&self) -> Vec<Chord> {
        custom(self).unwrap_or_else(|| self.chords.to_vec())
    }

    /// Keys as the cheatsheet and menus show them, e.g. `Ctrl+K  ·  Ctrl+Shift+P`.
    pub fn keys(&self) -> String {
        if !self.keys_label.is_empty() && custom(self).is_none() {
            // Written with "Ctrl"; macOS says Cmd.
            return self
                .keys_label
                .replace("Ctrl+", &format!("{}+", mod_label()));
        }
        self.current_chords()
            .into_iter()
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
                if b.keys_label.is_empty() || custom(b).is_some() {
                    b.current_chords().first().copied().map(Chord::label)
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

impl From<Command> for nectura_ui::kit::Shortcut {
    fn from(command: Command) -> Self {
        Self::new(hint(command))
    }
}

/// Consumes a key press that matches one of the chords the table lists for `command`, for a
/// widget that handles the command itself (the table only lists it); true if there was one.
pub fn consume(input: &mut egui::InputState, command: Command) -> bool {
    let mut found = false;
    input.events.retain(|e| {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = e
        else {
            return true;
        };
        let owned = !found
            && BINDINGS
                .iter()
                .filter(|b| b.command == command)
                .flat_map(Binding::current_chords)
                .any(|c| c.matches(*key, *modifiers));
        found |= owned;
        !owned
    });
    found
}

/// What the user changed in Settings → Keyboard, for the UI thread (the only one that draws or
/// dispatches keys): the saved text as the settings hold it, and the chords it came to.
#[derive(Debug, Default)]
struct Custom {
    saved: BTreeMap<String, String>,
    chords: Vec<(Command, Section, Chord)>,
}

thread_local! {
    static CUSTOM: RefCell<Custom> = RefCell::new(Custom::default());
}

/// The chord the user gave `binding`, if any.
fn custom(binding: &Binding) -> Option<Vec<Chord>> {
    CUSTOM.with_borrow(|c| {
        c.chords
            .iter()
            .find(|(command, section, _)| {
                *command == binding.command && *section == binding.section
            })
            .map(|(_, _, chord)| vec![*chord])
    })
}

/// Whether the user changed `binding`'s key.
pub fn is_customised(binding: &Binding) -> bool {
    custom(binding).is_some()
}

/// What `Settings::keybindings` calls a row: its command and where it applies.
pub fn binding_id(binding: &Binding) -> String {
    format!("{:?}.{:?}", binding.command, binding.section)
}

/// The row [`binding_id`] names.
pub fn binding_by_id(id: &str) -> Option<&'static Binding> {
    BINDINGS.iter().find(|b| binding_id(b) == id)
}

/// Whether the key of `binding` can be changed: every row but the tab jumps, whose digits
/// are positions.
pub fn is_remappable(binding: &Binding) -> bool {
    binding.command != Command::JumpTab
}

/// Takes the keys the settings hold: `saved` maps a row ([`binding_id`]) to a chord's text.
/// An entry that is not a row, does not read as a chord, breaks a rule of
/// [`shape_problem`] or would share its key with another row is ignored, so a saved or
/// imported file can never leave a key doing two things.
pub fn apply_saved(saved: &BTreeMap<String, String>) {
    if CUSTOM.with_borrow(|c| &c.saved == saved) {
        return;
    }
    let mut chords: Vec<(Command, Section, Chord)> = BINDINGS
        .iter()
        .filter(|b| is_remappable(b))
        .filter_map(|b| {
            let chord = Chord::from_text(saved.get(&binding_id(b))?)?;
            shape_problem(b.section, chord)
                .is_none()
                .then_some((b.command, b.section, chord))
        })
        .collect();
    // Drop the changes that collide until none do; a swap of two keys is not a collision.
    while let Some(clash) = (0..chords.len()).find(|i| {
        let (command, section, chord) = chords[*i];
        BINDINGS.iter().any(|other| {
            (other.command, other.section) != (command, section)
                && effective(other, &chords)
                    .iter()
                    .any(|c| c.same_key_as(chord))
        })
    }) {
        chords.remove(clash);
    }
    CUSTOM.set(Custom {
        saved: saved.clone(),
        chords,
    });
}

/// `binding`'s chords given `chords` as the user's changes.
fn effective(binding: &Binding, chords: &[(Command, Section, Chord)]) -> Vec<Chord> {
    chords
        .iter()
        .find(|(command, section, _)| *command == binding.command && *section == binding.section)
        .map_or_else(|| binding.chords.to_vec(), |(_, _, chord)| vec![*chord])
}

/// Why `chord` cannot be a key of a row in `section`, in words for the user.
pub fn shape_problem(section: Section, chord: Chord) -> Option<&'static str> {
    if matches!(chord.key, Key::SuperLeft | Key::SuperRight) {
        return Some("Super belongs to the window manager.");
    }
    if chord.key == Key::F9 {
        return Some("F9 belongs to the system.");
    }
    if chord.key == Key::Tab && !chord.ctrl {
        return Some("Tab moves between controls.");
    }
    if section.is_bare() && chord.ctrl {
        return Some(
            "A single key works without Ctrl; Ctrl keys are for the shortcuts that also work while typing.",
        );
    }
    if !section.is_bare() && !chord.ctrl && !matches!(chord.key, Key::F5 | Key::Escape) {
        return Some("This shortcut also works while typing, so it needs Ctrl.");
    }
    None
}

/// Whether `binding` could take `chord` as its key: the rows other than itself must not
/// already use it. The error says what does.
pub fn rebind_problem(binding: &Binding, chord: Chord) -> Option<String> {
    if let Some(why) = shape_problem(binding.section, chord) {
        return Some(why.to_owned());
    }
    BINDINGS
        .iter()
        .filter(|other| (other.command, other.section) != (binding.command, binding.section))
        .find(|other| other.current_chords().iter().any(|c| c.same_key_as(chord)))
        .map(|other| format!("{} already uses {}.", other.description, chord.label()))
}

/// The text a chord is saved as: `ctrl+shift+symbol+Slash` (the modifiers that apply, then the
/// key's name).
impl Chord {
    pub fn to_text(self) -> String {
        let mut text = String::new();
        for (on, word) in [
            (self.ctrl, "ctrl+"),
            (self.shift, "shift+"),
            (self.symbol, "symbol+"),
        ] {
            if on {
                text.push_str(word);
            }
        }
        text.push_str(self.key.name());
        text
    }

    /// The chord a text from [`Self::to_text`] says; `None` when it is anything else.
    pub fn from_text(text: &str) -> Option<Self> {
        let mut chord = Self {
            key: Key::A,
            ctrl: false,
            shift: false,
            symbol: false,
        };
        let mut rest = text;
        loop {
            if let Some(after) = rest.strip_prefix("ctrl+").filter(|_| !chord.ctrl) {
                chord.ctrl = true;
                rest = after;
            } else if let Some(after) = rest.strip_prefix("shift+").filter(|_| !chord.shift) {
                chord.shift = true;
                rest = after;
            } else if let Some(after) = rest.strip_prefix("symbol+").filter(|_| !chord.symbol) {
                chord.symbol = true;
                rest = after;
            } else {
                break;
            }
        }
        chord.key = Key::from_name(rest).filter(|key| key.name() == rest)?;
        Some(chord)
    }

    /// Both chords are the same key press: a symbol is typed with whatever Shift its layout
    /// needs, so Shift does not tell it apart.
    fn same_key_as(self, other: Self) -> bool {
        self.key == other.key
            && self.ctrl == other.ctrl
            && (self.symbol || other.symbol || self.shift == other.shift)
    }

    /// The chord of a key press: a punctuation key is a symbol, whose Shift is dropped.
    pub fn from_press(key: Key, modifiers: Modifiers) -> Self {
        let symbol = {
            let mut chars = key.symbol_or_name().chars();
            chars
                .next()
                .is_some_and(|c| chars.next().is_none() && !c.is_alphanumeric())
        };
        Self {
            key,
            ctrl: modifiers.command || modifiers.ctrl,
            shift: modifiers.shift && !symbol,
            symbol,
        }
    }
}

/// Where Settings → Keyboard listens for the new key of a row.
#[derive(Debug, Default)]
enum Capture {
    #[default]
    Off,
    /// Waiting for a key; `seen` is the last frame Settings asked for it, so it stops by
    /// itself when the page goes away.
    Waiting {
        seen: u64,
    },
    Cancelled,
    Got(Chord),
}

thread_local! {
    static CAPTURE: RefCell<Capture> = const { RefCell::new(Capture::Off) };
}

/// Settings asks for the next key press to be taken as a row's new key, instead of acting.
pub fn start_capture(ctx: &egui::Context) {
    CAPTURE.set(Capture::Waiting {
        seen: ctx.cumulative_pass_nr(),
    });
}

/// Called every frame the page that started a capture is drawn.
pub fn keep_capturing(ctx: &egui::Context) {
    CAPTURE.with_borrow_mut(|c| {
        if let Capture::Waiting { seen } = c {
            *seen = ctx.cumulative_pass_nr();
        }
    });
}

/// Whether the next key press goes to Settings.
pub fn is_capturing() -> bool {
    CAPTURE.with_borrow(|c| matches!(c, Capture::Waiting { .. }))
}

/// Stops a capture.
pub fn cancel_capture() {
    CAPTURE.set(Capture::Off);
}

/// How a capture ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Captured {
    /// Esc.
    Cancelled,
    /// The key that was pressed.
    Key(Chord),
}

/// What a finished capture came to; `None` while it is still waiting or off. Reading it ends
/// the capture.
pub fn take_capture() -> Option<Captured> {
    CAPTURE.with_borrow_mut(|c| match std::mem::take(c) {
        Capture::Cancelled => Some(Captured::Cancelled),
        Capture::Got(chord) => Some(Captured::Key(chord)),
        waiting => {
            *c = waiting;
            None
        }
    })
}

/// Whether `key` is a modifier's own key: egui sends a press of it (left and right apart) when
/// a modifier goes down, before the key it modifies.
const fn is_modifier_key(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// Takes this frame's first key press for a waiting capture, before any binding sees it: Esc
/// cancels, a modifier's own key is skipped (the capture keeps waiting for the key it
/// modifies, which carries the modifiers), and Alt chords are refused (the window manager's).
pub fn capture_key(ctx: &egui::Context) {
    let waiting = CAPTURE.with_borrow_mut(|c| {
        match c {
            // The page was not drawn last frame: it is gone.
            Capture::Waiting { seen } if ctx.cumulative_pass_nr() > *seen + 1 => {
                *c = Capture::Off;
                false
            }
            Capture::Waiting { .. } => true,
            _ => false,
        }
    });
    if !waiting {
        return;
    }
    let mut result = None;
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            if result.is_some() {
                return true;
            }
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = e
            else {
                return !matches!(e, egui::Event::Text(_));
            };
            if is_modifier_key(*key) {
                // Pressing a chord sends the modifier's own press first; the key it
                // modifies comes next, and a modifier alone is never a binding.
            } else if *key == Key::Escape {
                result = Some(Capture::Cancelled);
            } else {
                let chord = Chord::from_press(*key, *modifiers);
                if !modifiers.alt || chord.symbol {
                    result = Some(Capture::Got(chord));
                }
            }
            false
        });
    });
    if let Some(done) = result {
        CAPTURE.set(done);
    }
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
                let chords = binding.current_chords();
                let Some(index) = chords.iter().position(|c| c.matches(*key, *modifiers)) else {
                    continue;
                };
                if !active(binding, &chords[index]) {
                    continue;
                }
                if binding.double && index == 0 && custom(binding).is_none() {
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
    fn alt_belongs_to_the_window_manager_and_the_terminal() {
        for b in BINDINGS {
            for c in b.chords.iter().filter(|c| !c.symbol) {
                let held = Modifiers {
                    alt: true,
                    ctrl: c.ctrl,
                    shift: c.shift,
                    command: c.ctrl,
                    ..Modifiers::NONE
                };
                assert!(!c.matches(c.key, held), "{} fires with Alt held", c.label());
            }
        }
    }

    /// Omarchy's chords that are not Super chords: window cycling, monitor focus, close all
    /// windows, and the push-to-talk dictation key (`hypr/bindings/*.lua`).
    #[test]
    fn nothing_fires_on_a_chord_omarchy_owns() {
        let alt = Modifiers::ALT;
        let ctrl_alt = Modifiers::ALT | Modifiers::COMMAND;
        let owned = [
            (Key::Tab, alt),
            (Key::Tab, alt | Modifiers::SHIFT),
            (Key::Tab, ctrl_alt),
            (Key::Tab, ctrl_alt | Modifiers::SHIFT),
            (Key::Delete, ctrl_alt),
            (Key::F9, Modifiers::NONE),
        ];
        for (key, held) in owned {
            for b in BINDINGS {
                assert!(
                    !b.chords.iter().any(|c| c.matches(key, held)),
                    "{key:?} with {held:?} would fire {:?}",
                    b.command
                );
            }
        }
    }

    #[test]
    fn shift_turns_a_family_around() {
        let chord = |command: Command| {
            BINDINGS
                .iter()
                .filter(|b| b.command == command)
                .max_by_key(|b| b.section.is_bare())
                .map(|b| b.chords[0])
                .expect("bound")
        };
        for (forward, back) in [
            (Command::Pull, Command::Push),
            (Command::Stash, Command::PopStash),
            (Command::TakeBoth, Command::TakeBothTheirsFirst),
            (Command::NextTab, Command::PrevTab),
            (Command::First, Command::Last),
        ] {
            let (f, r) = (chord(forward), chord(back));
            assert!(
                f.key == r.key && f.ctrl == r.ctrl && !f.shift && r.shift,
                "{forward:?} and {back:?} must differ by Shift only"
            );
        }
    }

    /// Shift+F10 opens a focused row's context menu (`ui/kit/menu.rs`), so the table lists it for
    /// the cheatsheet and Settings.
    #[test]
    fn shift_f10_is_in_the_table_and_the_cheatsheet() {
        let rows: Vec<&Binding> = BINDINGS
            .iter()
            .filter(|b| b.chords.contains(&shifted(Key::F10)))
            .collect();
        assert_eq!(rows.len(), 1, "one row owns Shift+F10");
        assert!(listed_here(rows[0]));
        assert_eq!(all_keys(rows[0].command), ["Shift+F10"]);
        assert_eq!(hint(rows[0].command), "Shift+F10");
        assert!(rows[0].section.is_bare(), "not while typing");
    }

    /// A widget that owns a command's key takes it from the table: whatever chord the table lists
    /// for `ContextMenu`, and no other key.
    #[test]
    fn a_widget_consumes_the_chords_the_table_lists_for_its_command() {
        let press = |key: Key, modifiers: Modifiers| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        };
        let ctx = egui::Context::default();
        let mut consumed = (false, false);
        let mut left = 0;
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    press(Key::F10, Modifiers::NONE),
                    press(Key::F10, Modifiers::SHIFT),
                ],
                ..Default::default()
            },
            |ui| {
                ui.input_mut(|i| {
                    consumed.0 = consume(i, Command::ContextMenu);
                    consumed.1 = consume(i, Command::ContextMenu);
                    left = i.events.len();
                });
            },
        );
        out.textures_delta.clear();
        assert_eq!(consumed, (true, false), "the Shift+F10 press, once");
        assert_eq!(left, 1, "the bare F10 press is left alone");
    }

    #[test]
    fn matching_respects_shift() {
        let m = Modifiers::NONE;
        assert!(bare(Key::G).matches(Key::G, m));
        assert!(!shifted(Key::G).matches(Key::G, m));
        assert!(shifted(Key::G).matches(Key::G, Modifiers::SHIFT));
        assert!(symbol(Key::Questionmark).matches(Key::Questionmark, Modifiers::SHIFT));
        assert!(!bare(Key::G).matches(Key::G, Modifiers::ALT));
        assert!(ctrl(Key::K).matches(Key::K, Modifiers::COMMAND));
        // `[` typed with AltGr (German, Nordic layouts) is still `[`.
        assert!(symbol(Key::OpenBracket).matches(Key::OpenBracket, Modifiers::ALT));
        assert!(!symbol(Key::OpenBracket).matches(Key::OpenBracket, Modifiers::COMMAND));
    }

    #[test]
    fn altgr_presses_match_plain_symbols_never_ctrl_chords() {
        let alt = Modifiers::ALT;
        let ctrl_alt = Modifiers::ALT | Modifiers::COMMAND;
        // Alt alone types the character on every platform.
        assert!(symbol(Key::Plus).matches(Key::Plus, alt));
        assert!(!ctrl_symbol(Key::Plus).matches(Key::Plus, alt));
        // Ctrl+Alt is AltGr on Windows only; elsewhere it is a Ctrl press.
        assert_eq!(
            symbol(Key::Plus).matches(Key::Plus, ctrl_alt),
            cfg!(windows)
        );
        assert_eq!(
            ctrl_symbol(Key::Plus).matches(Key::Plus, ctrl_alt),
            !cfg!(windows)
        );
    }

    #[test]
    fn every_command_with_a_hint() {
        assert!(
            !hint(Command::Palette).is_empty(),
            "the palette has no hint"
        );
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
        assert!(!fire(vec![press(Key::J)]).is_empty(), "j fires nothing");
        let held = fire(vec![press(Key::SuperLeft), press(Key::J)]);
        if cfg!(target_os = "macos") {
            assert!(!held.is_empty(), "Cmd+J fires nothing");
        } else {
            assert!(held.is_empty(), "{held:?}");
        }
    }

    #[test]
    fn gg_needs_two_presses() {
        let fired = fire(vec![press(Key::G)]);
        assert!(fired.is_empty(), "{fired:?}");
        let both = fire(vec![press(Key::G), release(Key::G), press(Key::G)]);
        assert_eq!(both.first().map(|f| f.command), Some(Command::First));
        // Held down, the key repeats: that is not a double press.
        let fired = fire(vec![press(Key::G), press(Key::G)]);
        assert!(fired.is_empty(), "{fired:?}");
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

    /// Settings → Keyboard: a row's key saved as text moves the command to it, the old key stops
    /// working, hints and the cheatsheet follow, and clearing the entry restores the table.
    #[test]
    fn a_changed_key_fires_the_command_and_the_old_one_does_not() {
        let fetch = binding_by_id("Fetch.Repository").expect("a row");
        assert_eq!(fetch.chords, [bare(Key::F)]);
        let saved = BTreeMap::from([("Fetch.Repository".to_owned(), bare(Key::V).to_text())]);
        apply_saved(&saved);
        let fired = |key| {
            fire(vec![press(key)])
                .iter()
                .any(|f| f.command == Command::Fetch)
        };
        assert!(fired(Key::V), "the new key fires it");
        assert!(!fired(Key::F), "the old key does not");
        assert_eq!(hint(Command::Fetch), format!("{}+Shift+F · v", mod_label()));
        assert_eq!(
            all_keys(Command::Fetch),
            vec![format!("{}+Shift+F", mod_label()), "v".to_owned()],
            "the cheatsheet shows it"
        );
        apply_saved(&BTreeMap::new());
        assert!(
            fired(Key::F) && !fired(Key::V),
            "cleared, the table is back"
        );
    }

    /// A saved or imported file cannot leave a key doing two things or take one that is not
    /// ours to give; swapping two keys is fine.
    #[test]
    fn saved_keys_that_break_a_rule_are_ignored() {
        let text = |c: Chord| c.to_text();
        let fetch = |c: Chord| ("Fetch.Repository".to_owned(), text(c));
        let key_of = |id: &str| binding_by_id(id).map(Binding::current_chords);
        let ours = || key_of("Fetch.Repository").expect("a row");
        for (why, saved) in [
            ("another row's key", BTreeMap::from([fetch(bare(Key::J))])),
            ("Super", BTreeMap::from([fetch(bare(Key::SuperLeft))])),
            (
                "Ctrl in a single-key row",
                BTreeMap::from([fetch(ctrl(Key::V))]),
            ),
            (
                "not a chord",
                BTreeMap::from([("Fetch.Repository".into(), "nonsense".into())]),
            ),
            (
                "no such row",
                BTreeMap::from([("Nothing.Anywhere".into(), text(bare(Key::V)))]),
            ),
            (
                "bare key that works while typing",
                BTreeMap::from([("Palette.Anywhere".into(), text(bare(Key::V)))]),
            ),
        ] {
            apply_saved(&saved);
            assert_eq!(ours(), [bare(Key::F)], "{why}");
        }
        let pull = ("Pull.Repository".to_owned(), text(bare(Key::F)));
        apply_saved(&BTreeMap::from([fetch(bare(Key::P)), pull]));
        assert_eq!(ours(), [bare(Key::P)], "a swap holds");
        assert_eq!(key_of("Pull.Repository"), Some(vec![bare(Key::F)]));
        // Two rows asking for the same free key: neither gets it.
        apply_saved(&BTreeMap::from([
            fetch(bare(Key::V)),
            ("Pull.Repository".to_owned(), text(bare(Key::V))),
        ]));
        assert_eq!(ours(), [bare(Key::F)]);
        apply_saved(&BTreeMap::new());
    }

    #[test]
    fn rebinding_names_the_row_that_has_the_key() {
        let fetch = binding_by_id("Fetch.Repository").expect("a row");
        let why = rebind_problem(fetch, bare(Key::J)).expect("taken");
        assert!(why.ends_with("already uses j."), "{why}");
        assert_eq!(rebind_problem(fetch, bare(Key::V)), None);
        assert_eq!(rebind_problem(fetch, bare(Key::F)), None, "its own key");
        assert!(rebind_problem(fetch, bare(Key::Tab)).is_some());
    }

    #[test]
    fn every_chord_of_the_table_reads_back_from_its_text() {
        for b in BINDINGS {
            for c in b.chords {
                assert_eq!(Chord::from_text(&c.to_text()), Some(*c), "{}", c.label());
            }
        }
        assert_eq!(Chord::from_text("ctrl+ctrl+K"), None);
        assert_eq!(Chord::from_text("ctrl+"), None);
    }

    /// Settings waits for the next press: it takes it before any binding does, Esc cancels,
    /// and a capture the page stopped asking for ends.
    #[test]
    fn a_waiting_capture_takes_the_next_key_before_the_keymap() {
        let run = |ctx: &egui::Context, events: Vec<egui::Event>| {
            let mut fired = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    capture_key(ui.ctx());
                    fired = dispatch(ui.ctx(), |_, _| true);
                },
            );
            output.textures_delta.clear();
            fired
        };
        let ctx = egui::Context::default();
        start_capture(&ctx);
        assert!(is_capturing());
        let fired = run(&ctx, vec![press(Key::J)]);
        assert!(fired.is_empty(), "j went to Settings: {fired:?}");
        assert_eq!(take_capture(), Some(Captured::Key(bare(Key::J))));
        assert!(!is_capturing());

        start_capture(&ctx);
        run(&ctx, vec![press(Key::Escape)]);
        assert_eq!(take_capture(), Some(Captured::Cancelled), "Esc cancels");

        start_capture(&ctx);
        for _ in 0..3 {
            run(&ctx, Vec::new());
        }
        assert!(!is_capturing(), "nobody kept it alive");
        assert_eq!(take_capture(), None);
    }

    /// Pressing a chord sends the modifier's own key press first (`ControlLeft`), then the key
    /// with the modifiers held: the capture waits for the key, and a modifier alone never
    /// becomes a binding.
    #[test]
    fn a_capture_waits_through_modifier_presses_for_the_key() {
        let key_press = |key: Key, modifiers: Modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let run = |ctx: &egui::Context, events: Vec<egui::Event>| {
            let mut fired = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    capture_key(ui.ctx());
                    fired = dispatch(ui.ctx(), |_, _| true);
                },
            );
            output.textures_delta.clear();
            fired
        };
        let ctx = egui::Context::default();
        let held = Modifiers::COMMAND | Modifiers::SHIFT;

        // The modifiers and the key arrive together in one frame.
        start_capture(&ctx);
        run(
            &ctx,
            vec![
                key_press(Key::ControlLeft, Modifiers::COMMAND),
                key_press(Key::ShiftLeft, held),
                key_press(Key::G, held),
            ],
        );
        assert_eq!(
            take_capture(),
            Some(Captured::Key(ctrl_shift(Key::G))),
            "Ctrl+Shift+G, not the Ctrl key"
        );

        // They arrive in frames of their own.
        start_capture(&ctx);
        run(&ctx, vec![key_press(Key::ControlLeft, Modifiers::COMMAND)]);
        assert!(is_capturing(), "Ctrl alone keeps waiting");
        assert_eq!(take_capture(), None);
        run(&ctx, vec![key_press(Key::G, held)]);
        assert_eq!(take_capture(), Some(Captured::Key(ctrl_shift(Key::G))));

        // No modifier key alone ever becomes the binding.
        for key in [
            Key::ShiftLeft,
            Key::ShiftRight,
            Key::ControlLeft,
            Key::ControlRight,
            Key::AltLeft,
            Key::AltRight,
            Key::SuperLeft,
            Key::SuperRight,
        ] {
            start_capture(&ctx);
            run(&ctx, vec![key_press(key, Modifiers::NONE)]);
            assert!(is_capturing(), "{key:?} alone keeps waiting");
            assert_eq!(take_capture(), None, "{key:?} is not a binding");
            cancel_capture();
        }

        // Once the key comes, a bare modifier before it does not stop it.
        start_capture(&ctx);
        run(
            &ctx,
            vec![
                key_press(Key::ShiftLeft, Modifiers::SHIFT),
                key_press(Key::J, Modifiers::NONE),
            ],
        );
        assert_eq!(take_capture(), Some(Captured::Key(bare(Key::J))));
    }
}
