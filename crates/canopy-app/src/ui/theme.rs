//! Design tokens. This is the only place colours are defined; see `docs/design.md`.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Visuals,
};

/// Colours of one theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub elevated: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub added_bg: Color32,
    pub removed_bg: Color32,
    pub lanes: [Color32; 10],
    pub dark: bool,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const LANES: [Color32; 10] = [
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

/// "Midnight", the default.
pub const DARK: Palette = Palette {
    bg: hex(0x0D1016),
    panel: hex(0x12161E),
    elevated: hex(0x171C26),
    hover: hex(0x1E2431),
    border: hex(0x262D3B),
    text: hex(0xE6E9EF),
    text_dim: hex(0x9AA3B5),
    text_faint: hex(0x636B7D),
    accent: hex(0x8B5CF6),
    accent2: hex(0x22D3EE),
    success: hex(0x34D399),
    warning: hex(0xFBBF24),
    danger: hex(0xF87171),
    added_bg: hex(0x0F2A21),
    removed_bg: hex(0x2E1518),
    lanes: LANES,
    dark: true,
};

/// "Daylight".
pub const LIGHT: Palette = Palette {
    bg: hex(0xF7F8FA),
    panel: hex(0xFFFFFF),
    elevated: hex(0xFFFFFF),
    hover: hex(0xEEF1F6),
    border: hex(0xDDE2EA),
    text: hex(0x161A22),
    text_dim: hex(0x566074),
    text_faint: hex(0x8A93A5),
    accent: hex(0x7C3AED),
    accent2: hex(0x0891B2),
    success: hex(0x059669),
    warning: hex(0xD97706),
    danger: hex(0xDC2626),
    added_bg: hex(0xE3F7EC),
    removed_bg: hex(0xFDE7E7),
    lanes: [
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
    ],
    dark: false,
};

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

/// The palette of the theme egui is currently showing.
pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if ctx.theme() == Theme::Dark {
        &DARK
    } else {
        &LIGHT
    }
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
    v.selection.stroke = Stroke::new(1.0, p.accent);
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

/// Installs both themes' styles on `ctx` and selects the dark or light one.
pub fn apply(ctx: &egui::Context, dark: bool) {
    for (theme, p) in [(Theme::Dark, &DARK), (Theme::Light, &LIGHT)] {
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
        style.animation_time = 0.18;
        ctx.set_style_of(theme, style);
    }
    ctx.set_theme(if dark { Theme::Dark } else { Theme::Light });
}
