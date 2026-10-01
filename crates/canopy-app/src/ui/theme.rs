//! Design tokens. This is the only place colours are defined; see `docs/design.md`.
//!
//! Canopy ships seven themes. Each is a [`Palette`] with the same tokens, so views never need to
//! know which theme is active: they ask [`palette`] for the current one.

use std::sync::atomic::{AtomicU8, Ordering};

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Visuals,
};
use serde::{Deserialize, Serialize};

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
    /// Hairlines and outlines.
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    /// Brand accent: selection, focus, primary actions.
    pub accent: Color32,
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
    /// Dark theme (egui's dark visuals as the base).
    pub dark: bool,
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
    hex(0x0891B2),
    hex(0xDB2777),
    hex(0x059669),
    hex(0xD97706),
    hex(0x2563EB),
    hex(0xDC2626),
    hex(0x65A30D),
    hex(0xEA580C),
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
    text: hex(0xE6E9EF),
    text_dim: hex(0x9AA3B5),
    text_faint: hex(0x767F92),
    accent: hex(0x9161F7),
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
    text: hex(0x161A22),
    text_dim: hex(0x566074),
    text_faint: hex(0x687185),
    accent: hex(0x7C3AED),
    accent2: hex(0x0891B2),
    success: hex(0x047857),
    warning: hex(0xB45309),
    danger: hex(0xDC2626),
    added_bg: hex(0xE3F7EC),
    removed_bg: hex(0xFDE7E7),
    button: hex(0x7C3AED),
    button2: hex(0x0E7490),
    on_button: hex(0xFFFFFF),
    lanes: LIGHT_LANES,
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
    text: hex(0xECEFF4),
    text_dim: hex(0xA9B1C1),
    text_faint: hex(0x8A93A5),
    accent: hex(0x88C0D0),
    accent2: hex(0xB48EAD),
    success: hex(0xA3BE8C),
    warning: hex(0xEBCB8B),
    danger: hex(0xE07A84),
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
        hex(0x5E81AC),
        hex(0xC8A2C8),
    ],
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
    text: hex(0xF3E9E2),
    text_dim: hex(0xB9A79B),
    text_faint: hex(0x8F7D71),
    accent: hex(0xF97316),
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
    dark: true,
};

/// "Neon": deep violet night with hot pink and cyan.
pub const NEON: Palette = Palette {
    name: "Neon",
    bg: hex(0x0B0716),
    panel: hex(0x110B20),
    elevated: hex(0x170F2B),
    hover: hex(0x211638),
    border: hex(0x2D1F4A),
    text: hex(0xF5EEFF),
    text_dim: hex(0xB3A3D1),
    text_faint: hex(0x8575B3),
    accent: hex(0xFF2E97),
    accent2: hex(0x00E5FF),
    success: hex(0x3DF5A0),
    warning: hex(0xFFD60A),
    danger: hex(0xFF4D6D),
    added_bg: hex(0x0B2A22),
    removed_bg: hex(0x2E0B1E),
    button: hex(0xD6006F),
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
    dark: true,
};

/// "Forest": deep greens with emerald and lime.
pub const FOREST: Palette = Palette {
    name: "Forest",
    bg: hex(0x0E1411),
    panel: hex(0x131B17),
    elevated: hex(0x18221D),
    hover: hex(0x1F2B25),
    border: hex(0x2A3830),
    text: hex(0xE4EEE8),
    text_dim: hex(0x9FB3A8),
    text_faint: hex(0x76897E),
    accent: hex(0x34D399),
    accent2: hex(0xA3E635),
    success: hex(0x4ADE80),
    warning: hex(0xFBBF24),
    danger: hex(0xF87171),
    added_bg: hex(0x12301F),
    removed_bg: hex(0x2E1618),
    button: hex(0x047857),
    button2: hex(0x4D7C0F),
    on_button: hex(0xFFFFFF),
    lanes: DARK_LANES,
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
    text: hex(0x2B2620),
    text_dim: hex(0x6B6255),
    text_faint: hex(0x766B5D),
    accent: hex(0xA14A08),
    accent2: hex(0x0F766E),
    success: hex(0x166534),
    warning: hex(0x8A5A00),
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
    dark: false,
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
}

impl ThemeId {
    /// Every theme, in menu order.
    pub const ALL: [ThemeId; 7] = [
        ThemeId::Midnight,
        ThemeId::Daylight,
        ThemeId::Aurora,
        ThemeId::Ember,
        ThemeId::Neon,
        ThemeId::Forest,
        ThemeId::Paper,
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

    /// A translucent version of `c` for fills behind text.
    pub fn tint(c: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
    }
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
    v.selection.bg_fill = Palette::tint(p.accent, if p.dark { 90 } else { 60 });
    // Also the keyboard focus ring: keep it fully opaque so focus is always visible.
    v.selection.stroke = Stroke::new(1.5, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.slider_trailing_fill = true;
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
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
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
    w.open = w.active;
    v
}

/// Makes `id` the active theme and installs its style on `ctx`. `reduce_motion` turns off
/// animations.
pub fn apply(ctx: &egui::Context, id: ThemeId, reduce_motion: bool) {
    ACTIVE.store(id.index(), Ordering::Relaxed);
    let p = id.palette();
    let theme = if p.dark { Theme::Dark } else { Theme::Light };
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals(p);
    style.text_styles = [
        (TextStyle::Heading, FontId::new(20.0, semibold())),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
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
    style.interaction.selectable_labels = false;
    style.animation_time = if reduce_motion { 0.0 } else { 0.18 };
    ctx.set_style_of(theme, style);
    ctx.set_theme(theme);
}

/// How long hover and selection fades take; zero with reduced motion.
pub fn fade(ctx: &egui::Context, seconds: f32) -> f32 {
    if ctx.global_style().animation_time == 0.0 {
        0.0
    } else {
        seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// WCAG 2.2: body text 7:1 (AAA); secondary and faint text (line numbers, dates), status
    /// colours and labels on filled buttons 4.5:1 (AA).
    #[test]
    fn every_theme_meets_contrast_targets() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            let checks = [
                ("text on bg", p.text, p.bg, 7.0),
                ("text on panel", p.text, p.panel, 7.0),
                ("text_dim on bg", p.text_dim, p.bg, 4.5),
                ("text_dim on panel", p.text_dim, p.panel, 4.5),
                ("text_faint on bg", p.text_faint, p.bg, 4.5),
                ("text_faint on panel", p.text_faint, p.panel, 4.5),
                ("accent on bg", p.accent, p.bg, 4.5),
                ("success on bg", p.success, p.bg, 4.5),
                ("warning on bg", p.warning, p.bg, 4.5),
                ("danger on bg", p.danger, p.bg, 4.5),
                ("on_button on button", p.on_button, p.button, 4.5),
                ("on_button on button2", p.on_button, p.button2, 4.5),
            ];
            for (what, fg, bg, min) in checks {
                let c = contrast(fg, bg);
                if c < min {
                    failures.push(format!("{}: {what} is {c:.2}:1, needs {min}:1", p.name));
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
}
