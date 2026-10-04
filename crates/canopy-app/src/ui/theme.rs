//! Design tokens. This is the only place colours are defined; see `docs/design.md`.
//!
//! Canopy ships eight themes. Each is a [`Palette`] with the same tokens, so views never need to
//! know which theme is active: they ask [`palette`] for the current one.

use std::sync::atomic::{AtomicU8, Ordering};

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Visuals,
};
use serde::{Deserialize, Serialize};

pub const XS: f32 = 4.0;
pub const SM: f32 = 8.0;
pub const MD: f32 = 12.0;
pub const LG: f32 = 16.0;
pub const XL: f32 = 24.0;
pub const COMPACT_WIDTH: f32 = 1000.0;
pub const SETTINGS_MAX_WIDTH: f32 = 1120.0;
pub const SETTINGS_MAX_HEIGHT: f32 = 900.0;
/// The widest a column of prose and its fields grow, so lines stay readable on wide windows.
pub const MEASURE: f32 = 920.0;
// Type scale (points, before interface zoom). Text sizes come only from these; icon glyphs are
// sized for their own optical weight.
/// Pills, badges and uppercase section labels.
pub const MICRO_SIZE: f32 = 10.5;
/// Dates, counts, hints and other secondary detail.
pub const SMALL_SIZE: f32 = 11.5;
/// Metadata beside a row's main text: author, date, palette detail.
pub const META_SIZE: f32 = 12.0;
/// Default interface text.
pub const BODY_SIZE: f32 = 13.0;
/// The text that leads a row, a field label or a button.
pub const LEAD_SIZE: f32 = 13.5;
/// Card and group titles, the repository name.
pub const SUBTITLE_SIZE: f32 = 14.0;
/// View and dialog titles, the palette query.
pub const TITLE_SIZE: f32 = 16.0;
/// The subject of a detail view and first-run dialog headings.
pub const HEADLINE_SIZE: f32 = 20.0;
/// The About dialog's name.
pub const DISPLAY_SIZE: f32 = 24.0;
/// The welcome screen's title.
pub const HERO_SIZE: f32 = 38.0;
/// Code, diffs and identifiers.
pub const MONO_SIZE: f32 = 12.5;
/// Short identifiers and key chords beside proportional text.
pub const MONO_SMALL_SIZE: f32 = 11.5;

// Icon sizes, for Phosphor glyphs drawn on their own (beside text, a glyph takes the text's size).
/// Disclosure chevrons and the close crosses of tabs and recent repositories.
pub const ICON_XS: f32 = 11.5;
/// Row glyphs in the sidebar, tab and segmented-control icons.
pub const ICON_SM: f32 = 14.0;
/// Glyphs on buttons, in palette rows, settings navigation and request status.
pub const ICON_MD: f32 = 16.0;
/// Toasts, account cards and the welcome tip.
pub const ICON_LG: f32 = 18.0;
/// Beside a dialog's title.
pub const ICON_TITLE: f32 = 20.0;
/// The welcome screen's action cards.
pub const ICON_XL: f32 = 24.0;
/// The centre of an empty or finished state.
pub const ICON_HERO: f32 = 32.0;
pub const MIN_WINDOW_SIZE: egui::Vec2 = egui::vec2(900.0, 560.0);
/// Inner margin of every framed text field, so fields match the 26-point control height.
pub const FIELD_MARGIN: Margin = Margin::symmetric(8, 5);

/// Colours of one theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Display name.
    pub name: &'static str,
    /// Window background, behind lists and the graph.
    pub bg: Color32,
    /// Panels: sidebar, toolbars, details.
    pub panel: Color32,
    /// Raised surfaces: dialogs, menus, cards.
    pub elevated: Color32,
    /// Hovered rows and controls.
    pub hover: Color32,
    /// Hairlines between regions: separators, card outlines. Decorative, so kept quiet.
    pub border: Color32,
    /// Outlines that identify a control (text fields, checkboxes, switch tracks): at least 3:1
    /// against every surface (WCAG 1.4.11).
    pub control_border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    /// Brand accent: selection, focus, primary actions.
    pub accent: Color32,
    /// Strength of the accent wash on a selected row.
    pub selection_alpha: u8,
    /// Second accent: links, remote refs, gradients.
    pub accent2: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub added_bg: Color32,
    pub removed_bg: Color32,
    /// Filled primary buttons and badges: gradient from `button` to `button2`, labelled in
    /// `on_button` (at least 4.5:1 against both ends).
    pub button: Color32,
    pub button2: Color32,
    pub on_button: Color32,
    /// Graph lane colours.
    pub lanes: [Color32; 10],
    /// Code colours in diffs (at least 4.5:1 on `bg`, `added_bg` and `removed_bg`).
    pub syntax: Syntax,
    /// Dark theme (egui's dark visuals as the base).
    pub dark: bool,
}

/// Syntax highlighting colours. Plain code uses `text`.
#[derive(Debug, Clone, Copy)]
pub struct Syntax {
    pub keyword: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub number: Color32,
    pub function: Color32,
    pub type_: Color32,
    pub constant: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const DARK_LANES: [Color32; 10] = [
    hex(0x8B5CF6),
    hex(0x22D3EE),
    hex(0xF472B6),
    hex(0x34D399),
    hex(0xFBBF24),
    hex(0x60A5FA),
    hex(0xF87171),
    hex(0xA3E635),
    hex(0xFB923C),
    hex(0xC084FC),
];

const LIGHT_LANES: [Color32; 10] = [
    hex(0x7C3AED),
    hex(0x088EAE),
    hex(0xDB2777),
    hex(0x059568),
    hex(0xB45309),
    hex(0x2563EB),
    hex(0xDC2626),
    hex(0x4D7C0F),
    hex(0xE1540C),
    hex(0x9333EA),
];

/// "Midnight", the default.
pub const MIDNIGHT: Palette = Palette {
    name: "Midnight",
    bg: hex(0x0D1016),
    panel: hex(0x12161E),
    elevated: hex(0x171C26),
    hover: hex(0x1E2431),
    border: hex(0x262D3B),
    control_border: hex(0x696F7A),
    text: hex(0xE6E9EF),
    text_dim: hex(0xA5ADBD),
    text_faint: hex(0x979EAD),
    accent: hex(0x9C71F8),
    selection_alpha: 46,
    accent2: hex(0x22D3EE),
    success: hex(0x34D399),
    warning: hex(0xFBBF24),
    danger: hex(0xF87171),
    added_bg: hex(0x0F2A21),
    removed_bg: hex(0x2E1518),
    button: hex(0x7C3AED),
    button2: hex(0x0E7490),
    on_button: hex(0xFFFFFF),
    lanes: DARK_LANES,
    syntax: Syntax {
        keyword: hex(0xC4A7FF),
        string: hex(0x7EE2B8),
        comment: hex(0x8A93A6),
        number: hex(0xF9A86B),
        function: hex(0x7CC4FF),
        type_: hex(0xFFD580),
        constant: hex(0xF59AC8),
    },
    dark: true,
};

/// "Daylight", the light default.
pub const DAYLIGHT: Palette = Palette {
    name: "Daylight",
    bg: hex(0xF7F8FA),
    panel: hex(0xFFFFFF),
    elevated: hex(0xFFFFFF),
    hover: hex(0xEEF1F6),
    border: hex(0xDDE2EA),
    control_border: hex(0x858A92),
    text: hex(0x161A22),
    text_dim: hex(0x4B5466),
    text_faint: hex(0x586071),
    accent: hex(0x7C3AED),
    selection_alpha: 30,
    accent2: hex(0x0D6F8A),
    success: hex(0x047555),
    warning: hex(0xA04A08),
    danger: hex(0xC72222),
    added_bg: hex(0xE3F7EC),
    removed_bg: hex(0xFDE7E7),
    button: hex(0x7C3AED),
    button2: hex(0x0E7490),
    on_button: hex(0xFFFFFF),
    lanes: LIGHT_LANES,
    syntax: Syntax {
        keyword: hex(0x6D28D9),
        string: hex(0x0B6E4F),
        comment: hex(0x5F6B7A),
        number: hex(0x9A3412),
        function: hex(0x1D4ED8),
        type_: hex(0x0E6A7E),
        constant: hex(0xBE185D),
    },
    dark: false,
};

/// "Aurora": cool arctic blues after the Nord palette.
pub const AURORA: Palette = Palette {
    name: "Aurora",
    bg: hex(0x1E222A),
    panel: hex(0x242933),
    elevated: hex(0x2B303B),
    hover: hex(0x333A47),
    border: hex(0x3B4252),
    control_border: hex(0x7E8490),
    text: hex(0xECEFF4),
    text_dim: hex(0xC2C8D3),
    text_faint: hex(0xB1B8C4),
    accent: hex(0x88C0D0),
    selection_alpha: 46,
    accent2: hex(0xCAAFC5),
    success: hex(0xA5C08F),
    warning: hex(0xEBCB8B),
    danger: hex(0xEAA5AB),
    added_bg: hex(0x253229),
    removed_bg: hex(0x3A262C),
    button: hex(0x88C0D0),
    button2: hex(0xB48EAD),
    on_button: hex(0x1E222A),
    lanes: [
        hex(0x88C0D0),
        hex(0xB48EAD),
        hex(0xA3BE8C),
        hex(0xEBCB8B),
        hex(0x81A1C1),
        hex(0xD08770),
        hex(0x8FBCBB),
        hex(0xE07A84),
        hex(0x698AB2),
        hex(0xC8A2C8),
    ],
    syntax: Syntax {
        keyword: hex(0x81A1C1),
        string: hex(0xA3BE8C),
        comment: hex(0x959FB3),
        number: hex(0xB48EAD),
        function: hex(0x88C0D0),
        type_: hex(0x8FBCBB),
        constant: hex(0xD08770),
    },
    dark: true,
};

/// "Ember": warm charcoal with orange and gold.
pub const EMBER: Palette = Palette {
    name: "Ember",
    bg: hex(0x16110F),
    panel: hex(0x1D1714),
    elevated: hex(0x241C18),
    hover: hex(0x2E241F),
    border: hex(0x3A2E27),
    control_border: hex(0x796E67),
    text: hex(0xF3E9E2),
    text_dim: hex(0xBCAB9F),
    text_faint: hex(0xAC9D93),
    accent: hex(0xF97316),
    selection_alpha: 30,
    accent2: hex(0xFACC15),
    success: hex(0x4ADE80),
    warning: hex(0xFBBF24),
    danger: hex(0xF87171),
    added_bg: hex(0x15281B),
    removed_bg: hex(0x331816),
    button: hex(0xC2410C),
    button2: hex(0xA16207),
    on_button: hex(0xFFFFFF),
    lanes: DARK_LANES,
    syntax: Syntax {
        keyword: hex(0xFB923C),
        string: hex(0xA3E635),
        comment: hex(0xA39184),
        number: hex(0xFACC15),
        function: hex(0x7DD3FC),
        type_: hex(0xF0ABFC),
        constant: hex(0xF472B6),
    },
    dark: true,
};

/// "Neon": deep violet night with electric cyan and hot pink. Cyan carries selection and focus so
/// they never read as the pink-red of removed lines and destructive actions.
pub const NEON: Palette = Palette {
    name: "Neon",
    bg: hex(0x0B0716),
    panel: hex(0x110B20),
    elevated: hex(0x170F2B),
    hover: hex(0x211638),
    border: hex(0x2D1F4A),
    control_border: hex(0x6F6386),
    text: hex(0xF5EEFF),
    text_dim: hex(0xCDBFE6),
    text_faint: hex(0xBBAEDC),
    accent: hex(0x00E5FF),
    selection_alpha: 46,
    accent2: hex(0xFF7ABD),
    success: hex(0x3DF5A0),
    warning: hex(0xFFD60A),
    danger: hex(0xFF6B4A),
    added_bg: hex(0x0B2A22),
    removed_bg: hex(0x2E0B1E),
    button: hex(0x0E7490),
    button2: hex(0x7C3AED),
    on_button: hex(0xFFFFFF),
    lanes: [
        hex(0xFF2E97),
        hex(0x00E5FF),
        hex(0xB026FF),
        hex(0xA3FF12),
        hex(0xFFB000),
        hex(0x4D7CFF),
        hex(0xFF4D6D),
        hex(0x00FFA3),
        hex(0xFF7A00),
        hex(0xE040FB),
    ],
    syntax: Syntax {
        keyword: hex(0xFF5CAD),
        string: hex(0x3DF5A0),
        comment: hex(0xA193CC),
        number: hex(0xFFD60A),
        function: hex(0x00E5FF),
        type_: hex(0xC77DFF),
        constant: hex(0xFF9E3D),
    },
    dark: true,
};

/// "Forest": deep greens lit with sunlit gold. Gold carries selection and focus and sky blue the
/// links, so neither is mistaken for the green of added lines and success.
pub const FOREST: Palette = Palette {
    name: "Forest",
    bg: hex(0x0E1411),
    panel: hex(0x131B17),
    elevated: hex(0x18221D),
    hover: hex(0x1F2B25),
    border: hex(0x2A3830),
    control_border: hex(0x67746D),
    text: hex(0xE4EEE8),
    text_dim: hex(0xB3C5BB),
    text_faint: hex(0xA7B4AD),
    accent: hex(0xE2B84F),
    selection_alpha: 46,
    accent2: hex(0x7DD3FC),
    success: hex(0x4ADE80),
    warning: hex(0xFB923C),
    danger: hex(0xF98282),
    added_bg: hex(0x12301F),
    removed_bg: hex(0x2E1618),
    button: hex(0x8A5A00),
    button2: hex(0x166534),
    on_button: hex(0xFFFFFF),
    lanes: DARK_LANES,
    syntax: Syntax {
        keyword: hex(0x6EE7B7),
        string: hex(0xD9F99D),
        comment: hex(0x91A79B),
        number: hex(0xFCD34D),
        function: hex(0x7DD3FC),
        type_: hex(0xC4B5FD),
        constant: hex(0xFDA4AF),
    },
    dark: true,
};

/// "Paper": warm off-white with rust and teal.
pub const PAPER: Palette = Palette {
    name: "Paper",
    bg: hex(0xF4EFE6),
    panel: hex(0xFBF8F2),
    elevated: hex(0xFFFDF8),
    hover: hex(0xEDE6DA),
    border: hex(0xDCD3C4),
    control_border: hex(0x8B8379),
    text: hex(0x2B2620),
    text_dim: hex(0x585044),
    text_faint: hex(0x655B50),
    accent: hex(0xA14A08),
    selection_alpha: 30,
    accent2: hex(0x0E6E67),
    success: hex(0x166534),
    warning: hex(0x825500),
    danger: hex(0xB91C1C),
    added_bg: hex(0xE3F0DC),
    removed_bg: hex(0xF8E1DC),
    button: hex(0xB45309),
    button2: hex(0x0F766E),
    on_button: hex(0xFFFFFF),
    lanes: [
        hex(0xB45309),
        hex(0x0F766E),
        hex(0x9D174D),
        hex(0x4D7C0F),
        hex(0x6D28D9),
        hex(0x1D4ED8),
        hex(0xB91C1C),
        hex(0x0E7490),
        hex(0xC2410C),
        hex(0x7E22CE),
    ],
    syntax: Syntax {
        keyword: hex(0xA13C07),
        string: hex(0x3F6212),
        comment: hex(0x62594B),
        number: hex(0x7E22CE),
        function: hex(0x2F4A8A),
        type_: hex(0x0E6B63),
        constant: hex(0x9D174D),
    },
    dark: false,
};

/// "Contrast": black and white with saturated signal colours and strong outlines, for low vision,
/// bright rooms and anyone who wants every edge unmistakable.
pub const CONTRAST: Palette = Palette {
    name: "Contrast",
    bg: hex(0x000000),
    panel: hex(0x0A0A0A),
    elevated: hex(0x121212),
    hover: hex(0x242424),
    border: hex(0x6B6B6B),
    control_border: hex(0xB3B3B3),
    text: hex(0xFFFFFF),
    text_dim: hex(0xE6E6E6),
    text_faint: hex(0xCCCCCC),
    accent: hex(0x4CC2FF),
    selection_alpha: 46,
    accent2: hex(0xFFE14D),
    success: hex(0x3DFF8A),
    warning: hex(0xFFA62B),
    danger: hex(0xFF6B6B),
    added_bg: hex(0x002B14),
    removed_bg: hex(0x3B0009),
    button: hex(0x005FB8),
    button2: hex(0x005FB8),
    on_button: hex(0xFFFFFF),
    lanes: [
        hex(0x4CC2FF),
        hex(0xFFE14D),
        hex(0xFF7AD9),
        hex(0x3DFF8A),
        hex(0xFFA62B),
        hex(0xB69CFF),
        hex(0xFF6B6B),
        hex(0x00F0D8),
        hex(0xD4FF3D),
        hex(0xFFFFFF),
    ],
    syntax: Syntax {
        keyword: hex(0x8FD6FF),
        string: hex(0x7DFFB0),
        comment: hex(0xC2C2C2),
        number: hex(0xFFC46B),
        function: hex(0xFFE14D),
        type_: hex(0xD2BFFF),
        constant: hex(0xFF9EE6),
    },
    dark: true,
};

/// A theme the user can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ThemeId {
    #[default]
    Midnight,
    Daylight,
    Aurora,
    Ember,
    Neon,
    Forest,
    Paper,
    Contrast,
}

impl ThemeId {
    /// Every theme, in menu order.
    pub const ALL: [ThemeId; 8] = [
        ThemeId::Midnight,
        ThemeId::Daylight,
        ThemeId::Aurora,
        ThemeId::Ember,
        ThemeId::Neon,
        ThemeId::Forest,
        ThemeId::Paper,
        ThemeId::Contrast,
    ];

    /// The theme's colours.
    pub fn palette(self) -> &'static Palette {
        match self {
            ThemeId::Midnight => &MIDNIGHT,
            ThemeId::Daylight => &DAYLIGHT,
            ThemeId::Aurora => &AURORA,
            ThemeId::Ember => &EMBER,
            ThemeId::Neon => &NEON,
            ThemeId::Forest => &FOREST,
            ThemeId::Paper => &PAPER,
            ThemeId::Contrast => &CONTRAST,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        self.palette().name
    }

    /// The theme after this one, wrapping around.
    pub fn next(self) -> ThemeId {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    fn index(self) -> u8 {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0) as u8
    }
}

/// The active theme. One window, one theme: a global keeps [`palette`] cheap to call.
static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// The theme currently shown.
pub fn active() -> ThemeId {
    ThemeId::ALL[usize::from(ACTIVE.load(Ordering::Relaxed)) % ThemeId::ALL.len()]
}

impl Palette {
    /// Lane colour for a graph colour index.
    pub fn lane(&self, index: u16) -> Color32 {
        self.lanes[usize::from(index) % self.lanes.len()]
    }

    /// The scrim behind modal dialogs, the settings screen, the palette and the cheatsheet: dims
    /// the window enough to separate the layer without hiding where the user was.
    pub fn backdrop(&self) -> Color32 {
        Color32::from_black_alpha(if self.dark { 176 } else { 60 })
    }

    /// Fill of the selected row in lists, the graph, the sidebar, the palette and diffs, and of
    /// egui's selected values. Every text colour stays readable on it (see the contrast test).
    pub fn selected(&self) -> Color32 {
        Palette::tint(self.accent, self.selection_alpha)
    }

    /// Fill behind warning text: conflict markers in diffs, in-progress banners.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "views adopt it in the controls pass")
    )]
    pub fn warning_tint(&self) -> Color32 {
        Palette::tint(self.warning, 40)
    }

    /// The surfaces a coloured component (pill, badge, avatar) can sit on: plain surfaces and a
    /// selected row.
    pub fn component_surfaces(&self) -> [Color32; 6] {
        let sel = self.selected();
        [
            self.bg,
            self.panel,
            self.elevated,
            self.hover,
            over(sel, self.bg),
            over(sel, self.panel),
        ]
    }

    /// Text or an icon in colour `fg` drawn on the translucent `fill`: `fg` moved towards the
    /// text colour just enough to read at 4.5:1 on that fill over every component surface, so
    /// it keeps its hue wherever the component is drawn.
    pub fn ink(&self, fg: Color32, fill: Color32) -> Color32 {
        let key = (self.name, fg, fill);
        if let Some(c) = INKS.with(|m| m.borrow().get(&key).copied()) {
            return c;
        }
        let behind = self.component_surfaces().map(|s| over(fill, s));
        let readable = |c: Color32| behind.iter().all(|b| contrast(c, *b) >= 4.5);
        let ink = (0..=20)
            .map(|step| mix(fg, self.text, step as f32 / 20.0))
            .find(|c| readable(*c))
            .unwrap_or(self.text);
        INKS.with(|m| m.borrow_mut().insert(key, ink));
        ink
    }

    /// Fill of a ref pill in colour `c`; the current branch and HEAD are stronger.
    pub fn pill_fill(c: Color32, strong: bool) -> Color32 {
        Palette::tint(c, if strong { 64 } else { 56 })
    }

    /// The outline reinforces ref identity while its text stays neutral.
    pub fn pill_border(c: Color32, strong: bool) -> Color32 {
        Palette::tint(c, if strong { 190 } else { 125 })
    }

    /// Fill of an initials avatar in colour `c`.
    pub fn avatar_fill(&self, c: Color32) -> Color32 {
        Palette::tint(c, if self.dark { 70 } else { 50 })
    }

    /// Fill of a small badge (change letter, count) in colour `c`.
    pub fn badge_fill(c: Color32) -> Color32 {
        Palette::tint(c, 40)
    }

    /// A translucent version of `c` for fills behind text.
    pub fn tint(c: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
    }
}

/// Relative luminance (WCAG 2.2) of an opaque colour.
fn luminance(c: Color32) -> f64 {
    let ch = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
}

/// WCAG contrast ratio between two opaque colours, from 1 to 21.
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Translucent `fg` drawn over opaque `bg`, as the GPU blends it (`Color32` is premultiplied).
pub fn over(fg: Color32, bg: Color32) -> Color32 {
    let a = f32::from(fg.a()) / 255.0;
    let mix = |f: u8, b: u8| (f32::from(f) + f32::from(b) * (1.0 - a)).round() as u8;
    Color32::from_rgb(
        mix(fg.r(), bg.r()),
        mix(fg.g(), bg.g()),
        mix(fg.b(), bg.b()),
    )
}

/// `a` moved towards `b` by `t` (0 to 1).
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

thread_local! {
    /// Inks already worked out, per theme, colour and fill: views ask for the same few every frame.
    static INKS: std::cell::RefCell<std::collections::HashMap<(&'static str, Color32, Color32), Color32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The palette of the active theme.
pub fn palette(_ctx: &egui::Context) -> &'static Palette {
    active().palette()
}

/// Corner radii.
pub const RADIUS_SM: u8 = 6;
pub const RADIUS: u8 = 8;
pub const RADIUS_LG: u8 = 12;

/// Family of the semibold UI face.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Family of the medium UI face.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.panel;
    v.window_fill = p.elevated;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.bg;
    v.hyperlink_color = p.accent2;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(RADIUS_LG);
    v.menu_corner_radius = CornerRadius::same(RADIUS);
    v.window_shadow = Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 140 } else { 40 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 120 } else { 30 }),
    };
    // Selected list rows and egui's selected values and text share one fill.
    v.selection.bg_fill = p.selected();
    // Also the keyboard focus ring: keep it fully opaque so focus is always visible.
    v.selection.stroke = Stroke::new(1.5, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.indent_has_left_vline = false;

    let w = &mut v.widgets;
    let r = CornerRadius::same(RADIUS_SM);
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = p.elevated;
    w.inactive.weak_bg_fill = p.elevated;
    w.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, Palette::tint(p.accent, 160));
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = Palette::tint(p.accent, 70);
    w.active.weak_bg_fill = Palette::tint(p.accent, 70);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    v
}

/// The shortest a scrollbar's handle gets, so a long document's handle is still easy to see and
/// to grab (egui's default is 12).
const SCROLL_HANDLE_MIN: f32 = 32.0;

/// Solid scrollbars use foreground ink so they remain visible on raised light surfaces.
pub fn solid_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        foreground_color: true,
        bar_width: SM,
        handle_min_length: SCROLL_HANDLE_MIN,
        ..egui::style::ScrollStyle::solid()
    }
}

/// Makes `id` the active theme and installs its style on `ctx`. `reduce_motion` turns off
/// animations.
pub fn apply(ctx: &egui::Context, id: ThemeId, reduce_motion: bool) {
    ACTIVE.store(id.index(), Ordering::Relaxed);
    super::motion::set_reduced(reduce_motion);
    let p = id.palette();
    let theme = if p.dark { Theme::Dark } else { Theme::Light };
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals(p);
    style.text_styles = [
        (TextStyle::Heading, FontId::new(TITLE_SIZE, semibold())),
        (
            TextStyle::Body,
            FontId::new(BODY_SIZE, FontFamily::Proportional),
        ),
        (TextStyle::Button, FontId::new(BODY_SIZE, medium())),
        (
            TextStyle::Small,
            FontId::new(SMALL_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(MONO_SIZE, FontFamily::Monospace),
        ),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.window_margin = Margin::same(18);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.floating = true;
    style.spacing.scroll.handle_min_length = SCROLL_HANDLE_MIN;
    style.interaction.selectable_labels = false;
    style.animation_time = super::motion::time(super::motion::BASE);
    ctx.set_style_of(theme, style);
    ctx.set_theme(theme);
}

/// How long a fade takes: `seconds`, or zero with reduced motion.
pub fn fade(_ctx: &egui::Context, seconds: f32) -> f32 {
    super::motion::time(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.2 for every pairing the views draw. Text (`text` 7:1, AAA; `text_dim` and
    /// `text_faint` 4.5:1) on every surface, including a selected row over each background and
    /// the diff backgrounds; links and SHAs (`accent2`) and status colours 4.5:1 on plain
    /// surfaces and selected rows, `success` on added lines, `danger` on removed lines and
    /// `warning` on its tint; `accent` 4.5:1 on plain surfaces; control outlines 3:1 and graph
    /// lanes 3:1 (non-text contrast), lanes also on a selected history row.
    #[test]
    fn every_theme_meets_contrast_targets() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            let sel = |bg| over(p.selected(), bg);
            let plain = [
                ("bg", p.bg),
                ("panel", p.panel),
                ("elevated", p.elevated),
                ("hover", p.hover),
            ];
            let selected = [
                ("selected row on bg", sel(p.bg)),
                ("selected row on panel", sel(p.panel)),
                ("selected row on elevated", sel(p.elevated)),
            ];
            let diff = [
                ("added_bg", p.added_bg),
                ("removed_bg", p.removed_bg),
                ("selected added line", sel(p.added_bg)),
                ("selected removed line", sel(p.removed_bg)),
                ("warning tint", over(p.warning_tint(), p.bg)),
            ];
            let mut check = |what: &str, fg: Color32, on: &str, bg: Color32, min: f64| {
                let c = contrast(fg, bg);
                if c < min {
                    failures.push(format!(
                        "{}: {what} on {on} is {c:.2}:1, needs {min}:1",
                        p.name
                    ));
                }
            };
            for (on, bg) in plain.iter().chain(&selected).chain(&diff) {
                check("text", p.text, on, *bg, 7.0);
                check("text_dim", p.text_dim, on, *bg, 4.5);
                check("text_faint", p.text_faint, on, *bg, 4.5);
            }
            for (on, bg) in &plain {
                check("accent", p.accent, on, *bg, 4.5);
                check("control_border", p.control_border, on, *bg, 3.0);
            }
            for (on, bg) in plain.iter().chain(&selected) {
                check("accent2", p.accent2, on, *bg, 4.5);
                check("success", p.success, on, *bg, 4.5);
                check("warning", p.warning, on, *bg, 4.5);
                check("danger", p.danger, on, *bg, 4.5);
            }
            check("success", p.success, "added_bg", p.added_bg, 4.5);
            check("danger", p.danger, "removed_bg", p.removed_bg, 4.5);
            check("warning", p.warning, "warning tint", diff[4].1, 4.5);
            let lane_surfaces = [plain[0], plain[1], plain[3], selected[0]];
            for (i, lane) in p.lanes.iter().enumerate() {
                for (on, bg) in lane_surfaces {
                    check(&format!("lane {i}"), *lane, on, bg, 3.0);
                }
            }
            check("on_button", p.on_button, "button", p.button, 4.5);
            check("on_button", p.on_button, "button2", p.button2, 4.5);
            let sx = p.syntax;
            let code = [
                ("keyword", sx.keyword),
                ("string", sx.string),
                ("comment", sx.comment),
                ("number", sx.number),
                ("function", sx.function),
                ("type", sx.type_),
                ("constant", sx.constant),
            ];
            for (what, fg) in code {
                for (on, bg) in [
                    ("bg", p.bg),
                    ("added_bg", p.added_bg),
                    ("removed_bg", p.removed_bg),
                ] {
                    check(&format!("syntax {what}"), fg, on, bg, 4.5);
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Pills, avatars and badges keep their hue and still read at 4.5:1 wherever they are drawn.
    #[test]
    fn coloured_components_are_readable() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            let mut colours: Vec<Color32> = p.lanes.to_vec();
            colours.extend([
                p.accent, p.accent2, p.success, p.warning, p.danger, p.text_dim,
            ]);
            for c in colours {
                let fills = [
                    Palette::pill_fill(c, false),
                    Palette::pill_fill(c, true),
                    p.avatar_fill(c),
                    Palette::badge_fill(c),
                ];
                for fill in fills {
                    let ink = p.ink(c, fill);
                    for s in p.component_surfaces() {
                        let r = contrast(ink, over(fill, s));
                        if r < 4.5 {
                            failures.push(format!("{}: {c:?} on {fill:?} is {r:.2}:1", p.name));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn next_cycles_through_all_themes() {
        let mut t = ThemeId::Midnight;
        for _ in 0..ThemeId::ALL.len() {
            t = t.next();
        }
        assert_eq!(t, ThemeId::Midnight);
    }
    #[test]
    fn open_menu_items_keep_the_same_surface_as_hovered_items() {
        for id in ThemeId::ALL {
            let v = visuals(id.palette());
            assert_eq!(
                v.widgets.open.weak_bg_fill,
                v.widgets.hovered.weak_bg_fill,
                "{} submenu must keep its hover fill when the pointer enters a child",
                id.palette().name
            );
            assert_eq!(
                v.widgets.open.fg_stroke.color,
                v.widgets.hovered.fg_stroke.color
            );
        }
    }
}
