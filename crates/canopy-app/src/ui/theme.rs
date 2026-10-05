//! Design tokens. This is the only place colours, geometry and type sizes are defined; see
//! `docs/design.md`.
//!
//! Canopy ships eight themes. Each is a [`Palette`]: the colours both styles share, plus one
//! [`Surfaces`] set per [`Style`] (Standard, the default, and Deeper). Geometry and type come from
//! [`Metrics`], one table per ([`Density`], [`Style`]) pair. The active theme, style, density and
//! translucency are globals set by [`apply`], so views ask [`palette`], [`surfaces`], [`metrics`]
//! and [`appearance`] without knowing which is active.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Visuals,
    vec2,
};
use serde::{Deserialize, Serialize};

pub const XS: f32 = 4.0;
pub const SM: f32 = 8.0;
pub const MD: f32 = 12.0;
pub const LG: f32 = 16.0;
pub const XL: f32 = 24.0;
pub const XXL: f32 = 32.0;
pub const COMPACT_WIDTH: f32 = 1000.0;
pub const SETTINGS_MAX_WIDTH: f32 = 1120.0;
pub const SETTINGS_MAX_HEIGHT: f32 = 900.0;
/// The widest a column of prose and its fields grow, so lines stay readable on wide windows.
pub const MEASURE: f32 = 920.0;
/// A disabled control's opacity (the whole control, label included).
pub const DISABLED_ALPHA: f32 = 0.42;
/// A disabled icon-only button's opacity.
pub const DISABLED_ICON_ALPHA: f32 = 0.40;

// Type scale and control sizes used by views written before the component library. The
// component library reads [`metrics`] instead, and the migration moves views onto it and deletes
// these constants.
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
/// The centre of an empty or finished state.
pub const ICON_HERO: f32 = 32.0;
pub const MIN_WINDOW_SIZE: egui::Vec2 = egui::vec2(900.0, 560.0);
/// Shared control geometry, in logical points before interface zoom.
pub const COMPACT_CONTROL_HEIGHT: f32 = 28.0;
pub const PRIMARY_CONTROL_HEIGHT: f32 = 36.0;
pub const CONTROL_INSET: f32 = 2.0;
pub const CHECKBOX_SIZE: f32 = 18.0;
/// Inner margin of every framed text field, matching the standard control height.
pub const FIELD_MARGIN: Margin = Margin::symmetric(8, 7);
/// Tallest raw-output viewport of a notification card.
pub const TOAST_DETAILS_HEIGHT: f32 = 140.0;

/// Two ends of a vertical fill, top first. Standard surfaces are flat (`top == bottom`) and paint
/// as one rect; Deeper's are painted with `kit::paint::fill` (a ramp texture brush).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fill {
    pub top: Color32,
    pub bottom: Color32,
}

impl Fill {
    pub const fn flat(c: Color32) -> Self {
        Self { top: c, bottom: c }
    }

    pub fn is_flat(self) -> bool {
        self.top == self.bottom
    }

    /// Both ends as a pair, for checks that run on each.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the contrast tests check both ends")
    )]
    pub fn ends(self) -> [Color32; 2] {
        [self.top, self.bottom]
    }
}

/// The surfaces and material of one [`Style`] (Standard = round-2 concept 1, Deeper = concept 4).
#[derive(Debug, Clone, Copy)]
pub struct Surfaces {
    /// Window ground. Standard: also the content ground. Deeper: the gutter colour around cards.
    pub bg: Color32,
    /// Chrome: sidebar, status bar, pane headers, settings navigation. Deeper: equal to `bg`.
    pub panel: Color32,
    /// Content ground: History table, file lists, diff, welcome. Standard `== bg`; Deeper: the card.
    pub pane: Color32,
    /// Raised flat surface: inset boxes, the HEAD pill, toast detail blocks.
    pub elevated: Color32,
    /// Floating layers: menus, select lists, popovers, toasts, dialogs, palette, cheatsheet.
    pub overlay: Color32,
    /// Recessed input fill (text fields, search).
    pub field: Color32,
    /// Recessed tracks: the segmented track.
    pub track: Color32,
    /// Switch off track, slider rail, progress track.
    pub switch_off: Color32,
    /// Neutral count-badge fill.
    pub badge: Color32,
    /// Seam: 1 pt lines between regions and under headers.
    pub border: Color32,
    /// Edge: rings of fields, keycaps and cards' hover.
    pub edge: Color32,
    /// Ring of secondary buttons and select triggers.
    pub raised_edge: Color32,
    /// Ring of floating layers and tooltips.
    pub overlay_edge: Color32,
    /// Ring of the active document tab.
    pub tab_edge: Color32,
    /// Ring of the segmented track.
    pub track_edge: Color32,
    /// Ring of floating panes and cards; `None` in Standard (flush panes).
    pub pane_edge: Option<Color32>,
    /// 1 px inner bottom line of keycaps.
    pub keycap_edge: Color32,
    /// Secondary button, select trigger and their states.
    pub raised: Fill,
    pub raised_hover: Fill,
    pub raised_pressed: Fill,
    /// Segmented thumb.
    pub thumb: Fill,
    /// Keycap.
    pub keycap: Fill,
    /// Active document tab.
    pub tab: Fill,
    /// Tooltip (light themes use a dark tooltip).
    pub tooltip: Fill,
    /// Switch thumb, on and off.
    pub knob: Fill,
    /// Primary button, checked checkbox and radio, switch on, slider range, accent count badge.
    pub primary: Fill,
    pub primary_hover: Fill,
    pub primary_pressed: Fill,
    /// Danger button (confirmations and destructive primary actions) and its states.
    pub danger_solid: Fill,
    pub danger_solid_hover: Fill,
    pub danger_solid_pressed: Fill,
    /// Diff lines and gutters, composited over `pane`.
    pub added_bg: Color32,
    pub removed_bg: Color32,
    pub added_gutter: Color32,
    pub removed_gutter: Color32,
    /// The top deck: the repository tab strip and the action bar as one surface. Standard: flat
    /// `panel` (concept 1's two-tone chrome). Deeper: a lifted gradient with its own highlight
    /// and shadow, so it reads apart from the cards below.
    pub deck: Fill,
    /// The deck's bottom edge: Standard's seam under the tab strip and action bar; Deeper's outer
    /// line under the deck.
    pub deck_edge: Color32,
    /// Header strip of a table pane (History's column header). Standard: the pane itself.
    /// Deeper: a lifted tone.
    pub pane_header: Color32,
    /// The seam under a table pane's header. Standard: `border`. Deeper: tinted with `focus`, so
    /// it never reads as one of the neutral pane edges.
    pub pane_header_seam: Color32,
    /// 1 pt ring around the switch thumb (light themes, where white on the off track is 1.5:1):
    /// 3:1 against the thumb. `None` in dark themes.
    pub knob_edge: Option<Color32>,
}

/// Colours of one theme. Opaque unless stated; translucent values are made with
/// [`Palette::tint`] (premultiplied).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Display name.
    pub name: &'static str,
    /// One-line concept, shown with the theme's card.
    pub concept: &'static str,
    /// Dark theme (egui's dark visuals as the base).
    pub dark: bool,
    /// Stricter contrast targets (Contrast): text 10:1, outlines and focus 4.5:1.
    pub strict: bool,
    pub standard: Surfaces,
    pub deeper: Surfaces,
    /// Primary text: 7:1 on every surface, row state and diff background (10:1 in Contrast).
    pub text: Color32,
    /// Secondary text: authors, dates, paths, inactive labels. 4.5:1 (7:1 in Contrast).
    pub text_dim: Color32,
    /// Tertiary text: placeholders, line numbers, shortcuts, section labels. 4.5:1 (5.5:1).
    pub text_faint: Color32,
    /// Text-grade accent: selected SHA, active icons, checkmarks, matched characters, counts.
    pub accent: Color32,
    /// Links and remote-ref accents.
    pub accent2: Color32,
    /// Keyboard focus ring, focused field border, active underline-tab indicator, Deeper's
    /// focused-pane edge and table header seam.
    pub focus: Color32,
    /// Hue of the selection wash ([`Palette::selected`]).
    pub selection: Color32,
    pub selection_alpha: u8,
    /// Label on `primary*`: 4.5:1 on every end of every state, both styles.
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    /// Text-grade danger: danger menu items, error text, deletions.
    pub danger: Color32,
    /// Concept 1's danger red (`--danger`): error field border and halo, the danger button's focus
    /// ring, the danger badge fill, the danger menu highlight and the destructive dialog disc.
    pub danger_base: Color32,
    /// Label on `danger_solid*`: 4.5:1 on every end of every state, both styles.
    pub on_danger: Color32,
    /// Outlines that identify an input: text fields, checkbox and radio rims. 3:1 on every surface
    /// (4.5:1 in Contrast).
    pub control_border: Color32,
    /// Tooltip text and its second line.
    pub on_tooltip: Color32,
    pub on_tooltip_dim: Color32,
    /// Base of the hover, strong-hover, pressed and zebra overlays.
    pub hover_ink: Color32,
    /// Shadow colours: `shadow` for neutral layers, `primary_shadow` and `danger_shadow` for the
    /// tinted contact shadows of light themes (black in dark themes).
    pub shadow: Color32,
    pub primary_shadow: Color32,
    pub danger_shadow: Color32,
    /// Scrim behind dialogs, settings, palette and cheatsheet (premultiplied).
    pub scrim: Color32,
    /// Translucent surfaces: alpha of `overlay` painted over the blurred backdrop; 0 = never glass.
    pub glass_alpha: u8,
    /// Translucent surfaces: the blurred backdrop's relative luminance is clamped to at most this
    /// (dark) or at least this (light) so the contrast pairings hold over any content.
    pub glass_luminance: f32,
    /// Graph lanes, ref pills, avatar fills. 3:1 on pane, chrome, hovered and selected rows.
    pub lanes: [Color32; 10],
    /// Code colours in diffs. 4.5:1 on `pane`, `added_bg`, `removed_bg`.
    pub syntax: Syntax,
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

// Generated from the design spec's `themes.json` (spec/themes.py, contrast-fitted) plus the
// tokens added after the owner's preview review (danger_base, deck, pane header, knob ring).
/// "Midnight": Neutral blue-black with two-tone chrome and an indigo signature: concept 1, the default.
pub const MIDNIGHT: Palette = Palette {
    name: "Midnight",
    concept: "Neutral blue-black with two-tone chrome and an indigo signature: concept 1, the default.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x0F1012),
        panel: hex(0x141518),
        pane: hex(0x0F1012),
        elevated: hex(0x1B1C20),
        overlay: hex(0x1B1C20),
        field: hex(0x111214),
        track: hex(0x0C0D0F),
        switch_off: hex(0x30323A),
        badge: hex(0x2B2D33),
        border: hex(0x24262B),
        edge: hex(0x2B2D33),
        raised_edge: hex(0x2B2D33),
        overlay_edge: hex(0x2B2D33),
        tab_edge: hex(0x2B2D33),
        track_edge: hex(0x24262B),
        pane_edge: None,
        keycap_edge: hex(0x0B0B0D),
        raised: Fill::flat(hex(0x1B1C20)),
        raised_hover: Fill::flat(hex(0x222329)),
        raised_pressed: Fill::flat(hex(0x17181B)),
        thumb: Fill::flat(hex(0x26282E)),
        keycap: Fill::flat(hex(0x24262B)),
        tab: Fill::flat(hex(0x1B1C20)),
        tooltip: Fill::flat(hex(0x2A2C32)),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x5E6AD2)),
        primary_hover: Fill::flat(hex(0x5964CC)),
        primary_pressed: Fill::flat(hex(0x5560C4)),
        danger_solid: Fill::flat(hex(0xD63C42)),
        danger_solid_hover: Fill::flat(hex(0xD0353D)),
        danger_solid_pressed: Fill::flat(hex(0xC2353B)),
        added_bg: hex(0x15211C),
        removed_bg: hex(0x231517),
        added_gutter: hex(0x192E26),
        removed_gutter: hex(0x33191C),
        deck: Fill::flat(hex(0x141518)),
        deck_edge: hex(0x24262B),
        pane_header: hex(0x0F1012),
        pane_header_seam: hex(0x24262B),
        knob_edge: None,
    },
    deeper: Surfaces {
        bg: hex(0x0B0C0E),
        panel: hex(0x0B0C0E),
        pane: hex(0x141518),
        elevated: hex(0x1D1E23),
        overlay: hex(0x1D1E23),
        field: hex(0x0F1013),
        track: hex(0x0B0C0E),
        switch_off: hex(0x30323A),
        badge: hex(0x2A2C32),
        border: hex(0x22242A),
        edge: hex(0x2D2F36),
        raised_edge: hex(0x2F3138),
        overlay_edge: hex(0x2E3037),
        tab_edge: hex(0x2A2C32),
        track_edge: hex(0x1F2126),
        pane_edge: Some(hex(0x1F2126)),
        keycap_edge: hex(0x08080A),
        raised: Fill {
            top: hex(0x26282E),
            bottom: hex(0x1D1E23),
        },
        raised_hover: Fill {
            top: hex(0x2C2E35),
            bottom: hex(0x222329),
        },
        raised_pressed: Fill {
            top: hex(0x1A1B1F),
            bottom: hex(0x1F2025),
        },
        thumb: Fill {
            top: hex(0x2D2F36),
            bottom: hex(0x25272D),
        },
        keycap: Fill {
            top: hex(0x2A2C32),
            bottom: hex(0x202126),
        },
        tab: Fill {
            top: hex(0x202227),
            bottom: hex(0x1A1B20),
        },
        tooltip: Fill {
            top: hex(0x2C2E35),
            bottom: hex(0x24262C),
        },
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xE6E7EC),
        },
        primary: Fill {
            top: hex(0x616CD6),
            bottom: hex(0x4753BE),
        },
        primary_hover: Fill {
            top: hex(0x5C66D0),
            bottom: hex(0x434DB8),
        },
        primary_pressed: Fill {
            top: hex(0x5260C8),
            bottom: hex(0x4F5AC2),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC2353B),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0x192522),
        removed_bg: hex(0x27191D),
        added_gutter: hex(0x1E322B),
        removed_gutter: hex(0x381D21),
        deck: Fill {
            top: hex(0x1C1D20),
            bottom: hex(0x17181B),
        },
        deck_edge: hex(0x060608),
        pane_header: hex(0x191A1D),
        pane_header_seam: hex(0x323556),
        knob_edge: None,
    },
    text: hex(0xECECEF),
    text_dim: hex(0xA4A6AD),
    text_faint: hex(0x9698A0),
    accent: hex(0x9AA2FF),
    accent2: hex(0x7EC8F2),
    focus: hex(0x7C86FF),
    selection: hex(0x7C86FF),
    selection_alpha: 33,
    on_accent: hex(0xFFFFFF),
    success: hex(0x4CC38A),
    warning: hex(0xD9A54A),
    danger: hex(0xFF7D81),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    control_border: hex(0x787A81),
    on_tooltip: hex(0xECECEF),
    on_tooltip_dim: hex(0xB6B6BA),
    hover_ink: hex(0xFFFFFF),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(4, 4, 5, 158),
    glass_alpha: 204,
    glass_luminance: 0.03,
    lanes: [
        hex(0x7C86FF),
        hex(0x4CC38A),
        hex(0xE5739F),
        hex(0xD9A54A),
        hex(0x4EB8E9),
        hex(0xEE8266),
        hex(0xBB82E3),
        hex(0x43C3C3),
        hex(0x9EBF5F),
        hex(0xED914C),
    ],
    syntax: Syntax {
        keyword: hex(0xADB8FF),
        string: hex(0x7BD4A4),
        comment: hex(0x9698A0),
        number: hex(0xF3B17A),
        function: hex(0x82CBF6),
        type_: hex(0x7BDAD9),
        constant: hex(0xF49CBB),
    },
};

/// "Daylight": White canvas, light-grey chrome and the same indigo: Midnight by day.
pub const DAYLIGHT: Palette = Palette {
    name: "Daylight",
    concept: "White canvas, light-grey chrome and the same indigo: Midnight by day.",
    dark: false,
    strict: false,
    standard: Surfaces {
        bg: hex(0xFFFFFF),
        panel: hex(0xF7F7F8),
        pane: hex(0xFFFFFF),
        elevated: hex(0xFFFFFF),
        overlay: hex(0xFFFFFF),
        field: hex(0xFFFFFF),
        track: hex(0xEFEFF1),
        switch_off: hex(0xD4D4DA),
        badge: hex(0xECECEF),
        border: hex(0xE6E6EA),
        edge: hex(0xD9D9DF),
        raised_edge: hex(0xD9D9DF),
        overlay_edge: hex(0xE1E1E6),
        tab_edge: hex(0xE1E1E6),
        track_edge: hex(0xE3E3E8),
        pane_edge: None,
        keycap_edge: hex(0xCDCED5),
        raised: Fill::flat(hex(0xFFFFFF)),
        raised_hover: Fill::flat(hex(0xF6F6F8)),
        raised_pressed: Fill::flat(hex(0xEFEFF2)),
        thumb: Fill::flat(hex(0xFFFFFF)),
        keycap: Fill::flat(hex(0xF3F3F5)),
        tab: Fill::flat(hex(0xFFFFFF)),
        tooltip: Fill::flat(hex(0x18181B)),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x5E6AD2)),
        primary_hover: Fill::flat(hex(0x5560C8)),
        primary_pressed: Fill::flat(hex(0x4B56B8)),
        danger_solid: Fill::flat(hex(0xD83C41)),
        danger_solid_hover: Fill::flat(hex(0xCC3238)),
        danger_solid_pressed: Fill::flat(hex(0xB92B31)),
        added_bg: hex(0xEDF7F2),
        removed_bg: hex(0xFCF0F0),
        added_gutter: hex(0xDDF0E7),
        removed_gutter: hex(0xF9E2E3),
        deck: Fill::flat(hex(0xF7F7F8)),
        deck_edge: hex(0xE6E6EA),
        pane_header: hex(0xFFFFFF),
        pane_header_seam: hex(0xE6E6EA),
        knob_edge: Some(hex(0x94949A)),
    },
    deeper: Surfaces {
        bg: hex(0xECEEF1),
        panel: hex(0xECEEF1),
        pane: hex(0xFFFFFF),
        elevated: hex(0xFFFFFF),
        overlay: hex(0xFFFFFF),
        field: hex(0xFFFFFF),
        track: hex(0xE2E4E8),
        switch_off: hex(0xD4D4DA),
        badge: hex(0xECECEF),
        border: hex(0xE3E5EA),
        edge: hex(0xD3D6DC),
        raised_edge: hex(0xD3D6DC),
        overlay_edge: hex(0xDFE1E6),
        tab_edge: hex(0xD9DCE1),
        track_edge: hex(0xD9DCE1),
        pane_edge: Some(hex(0xE3E5EA)),
        keycap_edge: hex(0xC9CCD3),
        raised: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF4F5F7),
        },
        raised_hover: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xEEF0F3),
        },
        raised_pressed: Fill {
            top: hex(0xECEEF1),
            bottom: hex(0xF2F3F5),
        },
        thumb: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF6F7F9),
        },
        keycap: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF0F1F4),
        },
        tab: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF6F7F9),
        },
        tooltip: Fill::flat(hex(0x18181B)),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF1F2F5),
        },
        primary: Fill {
            top: hex(0x616CD6),
            bottom: hex(0x4753BE),
        },
        primary_hover: Fill {
            top: hex(0x5C66D0),
            bottom: hex(0x434DB8),
        },
        primary_pressed: Fill {
            top: hex(0x4F5BC4),
            bottom: hex(0x5561CC),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xBF3036),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0xEDF7F2),
        removed_bg: hex(0xFCF0F0),
        added_gutter: hex(0xDDF0E7),
        removed_gutter: hex(0xF9E2E3),
        deck: Fill {
            top: hex(0xF8FAFD),
            bottom: hex(0xF3F5F8),
        },
        deck_edge: hex(0xD3D5D8),
        pane_header: hex(0xFAFAFA),
        pane_header_seam: hex(0xCED1EE),
        knob_edge: Some(hex(0x898C92)),
    },
    text: hex(0x18181B),
    text_dim: hex(0x575962),
    text_faint: hex(0x5D5F69),
    accent: hex(0x4651C0),
    accent2: hex(0x006790),
    focus: hex(0x5E6AD2),
    selection: hex(0x5E6AD2),
    selection_alpha: 26,
    on_accent: hex(0xFFFFFF),
    success: hex(0x006A4B),
    warning: hex(0x805500),
    danger: hex(0xB2191F),
    danger_base: hex(0xD93D42),
    on_danger: hex(0xFFFFFF),
    control_border: hex(0x7C7D85),
    on_tooltip: hex(0xF4F4F5),
    on_tooltip_dim: hex(0xB6B6B8),
    hover_ink: hex(0x14141E),
    shadow: hex(0x101018),
    primary_shadow: hex(0x1E1E50),
    danger_shadow: hex(0x781414),
    scrim: Color32::from_rgba_premultiplied(6, 6, 9, 82),
    glass_alpha: 214,
    glass_luminance: 0.8,
    lanes: [
        hex(0x5E6AD2),
        hex(0x1D9A5F),
        hex(0xCC477C),
        hex(0xB07A1C),
        hex(0x017EAF),
        hex(0xC64E31),
        hex(0x904EBA),
        hex(0x02888E),
        hex(0x648828),
        hex(0xBE6517),
    ],
    syntax: Syntax {
        keyword: hex(0x4B50B0),
        string: hex(0x116F42),
        comment: hex(0x696B75),
        number: hex(0x974C01),
        function: hex(0x006495),
        type_: hex(0x006B70),
        constant: hex(0xA22C5E),
    },
};

/// "Aurora": Deep marine navy, the one saturated ground: polar-sky blue with an aurora violet.
pub const AURORA: Palette = Palette {
    name: "Aurora",
    concept: "Deep marine navy, the one saturated ground: polar-sky blue with an aurora violet.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x061220),
        panel: hex(0x091826),
        pane: hex(0x061220),
        elevated: hex(0x0F1F30),
        overlay: hex(0x0F1F30),
        field: hex(0x071523),
        track: hex(0x040F1D),
        switch_off: hex(0x22364B),
        badge: hex(0x1D3145),
        border: hex(0x17293C),
        edge: hex(0x1D3145),
        raised_edge: hex(0x1D3145),
        overlay_edge: hex(0x1D3145),
        tab_edge: hex(0x1D3145),
        track_edge: hex(0x17293C),
        pane_edge: None,
        keycap_edge: hex(0x030D1B),
        raised: Fill::flat(hex(0x0F1F30)),
        raised_hover: Fill::flat(hex(0x152739)),
        raised_pressed: Fill::flat(hex(0x0B1B2A)),
        thumb: Fill::flat(hex(0x192B3F)),
        keycap: Fill::flat(hex(0x17293C)),
        tab: Fill::flat(hex(0x0F1F30)),
        tooltip: Fill::flat(hex(0x1C3044)),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x0169B6)),
        primary_hover: Fill::flat(hex(0x1175C5)),
        primary_pressed: Fill::flat(hex(0x015FA5)),
        danger_solid: Fill::flat(hex(0xD63C42)),
        danger_solid_hover: Fill::flat(hex(0xD0353D)),
        danger_solid_pressed: Fill::flat(hex(0xC2353B)),
        added_bg: hex(0x0E242C),
        removed_bg: hex(0x1A1624),
        added_gutter: hex(0x163538),
        removed_gutter: hex(0x2C1B28),
        deck: Fill::flat(hex(0x091826)),
        deck_edge: hex(0x17293C),
        pane_header: hex(0x061220),
        pane_header_seam: hex(0x17293C),
        knob_edge: None,
    },
    deeper: Surfaces {
        bg: hex(0x030E1B),
        panel: hex(0x030E1B),
        pane: hex(0x091826),
        elevated: hex(0x112132),
        overlay: hex(0x112132),
        field: hex(0x061320),
        track: hex(0x030E1B),
        switch_off: hex(0x22364B),
        badge: hex(0x1C3044),
        border: hex(0x15273A),
        edge: hex(0x1F3347),
        raised_edge: hex(0x213549),
        overlay_edge: hex(0x203448),
        tab_edge: hex(0x1C3044),
        track_edge: hex(0x132436),
        pane_edge: Some(hex(0x132436)),
        keycap_edge: hex(0x010A17),
        raised: Fill {
            top: hex(0x192B3F),
            bottom: hex(0x112132),
        },
        raised_hover: Fill {
            top: hex(0x1E3246),
            bottom: hex(0x152739),
        },
        raised_pressed: Fill {
            top: hex(0x0E1E2E),
            bottom: hex(0x122335),
        },
        thumb: Fill {
            top: hex(0x1F3347),
            bottom: hex(0x182A3D),
        },
        keycap: Fill {
            top: hex(0x1C3044),
            bottom: hex(0x132436),
        },
        tab: Fill {
            top: hex(0x142537),
            bottom: hex(0x0E1E2E),
        },
        tooltip: Fill {
            top: hex(0x1E3246),
            bottom: hex(0x17293C),
        },
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xE3EBF4),
        },
        primary: Fill {
            top: hex(0x1979CA),
            bottom: hex(0x0061A9),
        },
        primary_hover: Fill {
            top: hex(0x0F73C4),
            bottom: hex(0x005CA1),
        },
        primary_pressed: Fill {
            top: hex(0x005FA6),
            bottom: hex(0x005A9D),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC2353B),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0x112A32),
        removed_bg: hex(0x1D1C2A),
        added_gutter: hex(0x183A3D),
        removed_gutter: hex(0x2E202D),
        deck: Fill {
            top: hex(0x11202F),
            bottom: hex(0x0C1B29),
        },
        deck_edge: hex(0x010814),
        pane_header: hex(0x0E1D2B),
        pane_header_seam: hex(0x24485F),
        knob_edge: None,
    },
    text: hex(0xE7F0F5),
    text_dim: hex(0xB3C2CC),
    text_faint: hex(0x9CADB9),
    accent: hex(0x67C4FD),
    accent2: hex(0xCEACF7),
    focus: hex(0x66C6FD),
    selection: hex(0x67C4FD),
    selection_alpha: 33,
    on_accent: hex(0xFFFFFF),
    success: hex(0x64E3AB),
    warning: hex(0xF5CE6C),
    danger: hex(0xFD8A8F),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    control_border: hex(0x6A7D8D),
    on_tooltip: hex(0xE7F0F5),
    on_tooltip_dim: hex(0xAEBAC3),
    hover_ink: hex(0xE7F0F5),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(4, 4, 5, 158),
    glass_alpha: 196,
    glass_luminance: 0.03,
    lanes: [
        hex(0x67C4FD),
        hex(0xF6A26F),
        hex(0xCBA5F7),
        hex(0x69D1A1),
        hex(0xD9B457),
        hex(0xA4B3FF),
        hex(0xFC989C),
        hex(0x41D1CA),
        hex(0x9FC874),
        hex(0xEC9AD1),
    ],
    syntax: Syntax {
        keyword: hex(0xD1B2F7),
        string: hex(0x84E1B5),
        comment: hex(0x96A7B7),
        number: hex(0xFABC86),
        function: hex(0x85D3FB),
        type_: hex(0x8DDFDB),
        constant: hex(0xF6A9D4),
    },
};

/// "Ember": Warm charcoal and cream text with a bright copper signature and steel-blue links.
pub const EMBER: Palette = Palette {
    name: "Ember",
    concept: "Warm charcoal and cream text with a bright copper signature and steel-blue links.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x110D0B),
        panel: hex(0x17120F),
        pane: hex(0x110D0B),
        elevated: hex(0x1F1915),
        overlay: hex(0x1F1915),
        field: hex(0x130F0D),
        track: hex(0x0E0B08),
        switch_off: hex(0x372E29),
        badge: hex(0x322924),
        border: hex(0x2A231E),
        edge: hex(0x322924),
        raised_edge: hex(0x322924),
        overlay_edge: hex(0x322924),
        tab_edge: hex(0x322924),
        track_edge: hex(0x2A231E),
        pane_edge: None,
        keycap_edge: hex(0x0C0907),
        raised: Fill::flat(hex(0x1F1915)),
        raised_hover: Fill::flat(hex(0x27201C)),
        raised_pressed: Fill::flat(hex(0x1A1512)),
        thumb: Fill::flat(hex(0x2C2520)),
        keycap: Fill::flat(hex(0x2A231E)),
        tab: Fill::flat(hex(0x1F1915)),
        tooltip: Fill::flat(hex(0x312823)),
        knob: Fill::flat(hex(0xFEF9F3)),
        primary: Fill::flat(hex(0xF59344)),
        primary_hover: Fill::flat(hex(0xFFA15A)),
        primary_pressed: Fill::flat(hex(0xE98838)),
        danger_solid: Fill::flat(hex(0xD63C42)),
        danger_solid_hover: Fill::flat(hex(0xD0353D)),
        danger_solid_pressed: Fill::flat(hex(0xC2353B)),
        added_bg: hex(0x1B1F15),
        removed_bg: hex(0x241211),
        added_gutter: hex(0x252F1E),
        removed_gutter: hex(0x351716),
        deck: Fill::flat(hex(0x17120F)),
        deck_edge: hex(0x2A231E),
        pane_header: hex(0x110D0B),
        pane_header_seam: hex(0x2A231E),
        knob_edge: None,
    },
    deeper: Surfaces {
        bg: hex(0x0D0A07),
        panel: hex(0x0D0A07),
        pane: hex(0x17120F),
        elevated: hex(0x211B17),
        overlay: hex(0x211B17),
        field: hex(0x110E0B),
        track: hex(0x0D0A07),
        switch_off: hex(0x372E29),
        badge: hex(0x312823),
        border: hex(0x28211C),
        edge: hex(0x342B26),
        raised_edge: hex(0x362D28),
        overlay_edge: hex(0x352C27),
        tab_edge: hex(0x312823),
        track_edge: hex(0x241E1A),
        pane_edge: Some(hex(0x241E1A)),
        keycap_edge: hex(0x090605),
        raised: Fill {
            top: hex(0x2C2520),
            bottom: hex(0x211B17),
        },
        raised_hover: Fill {
            top: hex(0x332A25),
            bottom: hex(0x27201C),
        },
        raised_pressed: Fill {
            top: hex(0x1E1815),
            bottom: hex(0x231D19),
        },
        thumb: Fill {
            top: hex(0x342B26),
            bottom: hex(0x2B241F),
        },
        keycap: Fill {
            top: hex(0x312823),
            bottom: hex(0x251E1A),
        },
        tab: Fill {
            top: hex(0x251F1A),
            bottom: hex(0x1E1815),
        },
        tooltip: Fill {
            top: hex(0x332A25),
            bottom: hex(0x2A231E),
        },
        knob: Fill {
            top: hex(0xFEF9F3),
            bottom: hex(0xEDE2DC),
        },
        primary: Fill {
            top: hex(0xFFA969),
            bottom: hex(0xEC8A3A),
        },
        primary_hover: Fill {
            top: hex(0xFFB987),
            bottom: hex(0xF59244),
        },
        primary_pressed: Fill {
            top: hex(0xE98838),
            bottom: hex(0xE38231),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC2353B),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0x212319),
        removed_bg: hex(0x2A1615),
        added_gutter: hex(0x2A3321),
        removed_gutter: hex(0x3A1B19),
        deck: Fill {
            top: hex(0x1F1A17),
            bottom: hex(0x1A1512),
        },
        deck_edge: hex(0x070503),
        pane_header: hex(0x1C1613),
        pane_header_seam: hex(0x533822),
        knob_edge: None,
    },
    text: hex(0xF2ECE3),
    text_dim: hex(0xC4B8AC),
    text_faint: hex(0xAFA295),
    accent: hex(0xF19C52),
    accent2: hex(0x89C6EE),
    focus: hex(0xF69F50),
    selection: hex(0xF19C52),
    selection_alpha: 33,
    on_accent: hex(0x21140D),
    success: hex(0x88D87F),
    warning: hex(0xEED558),
    danger: hex(0xFB7C89),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    control_border: hex(0x80756D),
    on_tooltip: hex(0xF2ECE3),
    on_tooltip_dim: hex(0xBCB5AD),
    hover_ink: hex(0xF2ECE3),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(4, 4, 5, 158),
    glass_alpha: 204,
    glass_luminance: 0.03,
    lanes: [
        hex(0xEEA05D),
        hex(0x5DBFFA),
        hex(0xF092BF),
        hex(0x85C87D),
        hex(0xC9B54D),
        hex(0xA5ABFF),
        hex(0xFA9296),
        hex(0x2DCDC6),
        hex(0xAAC060),
        hex(0xD899E6),
    ],
    syntax: Syntax {
        keyword: hex(0xF8AE7B),
        string: hex(0xA8D88F),
        comment: hex(0xAC9D91),
        number: hex(0xE7D17A),
        function: hex(0x8ECBF3),
        type_: hex(0xF0ABCB),
        constant: hex(0xD3B1ED),
    },
};

/// "Neon": Black-violet night with electric cyan and hot magenta; colour only in the signals.
pub const NEON: Palette = Palette {
    name: "Neon",
    concept: "Black-violet night with electric cyan and hot magenta; colour only in the signals.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x0A080D),
        panel: hex(0x0F0C14),
        pane: hex(0x0A080D),
        elevated: hex(0x16131D),
        overlay: hex(0x16131D),
        field: hex(0x0C0A10),
        track: hex(0x07060A),
        switch_off: hex(0x2D2738),
        badge: hex(0x272233),
        border: hex(0x201B29),
        edge: hex(0x272233),
        raised_edge: hex(0x272233),
        overlay_edge: hex(0x272233),
        tab_edge: hex(0x272233),
        track_edge: hex(0x201B29),
        pane_edge: None,
        keycap_edge: hex(0x060509),
        raised: Fill::flat(hex(0x16131D)),
        raised_hover: Fill::flat(hex(0x1D1926)),
        raised_pressed: Fill::flat(hex(0x120F18)),
        thumb: Fill::flat(hex(0x221D2C)),
        keycap: Fill::flat(hex(0x201B29)),
        tab: Fill::flat(hex(0x16131D)),
        tooltip: Fill::flat(hex(0x262131)),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x00E0F3)),
        primary_hover: Fill::flat(hex(0x3CECFF)),
        primary_pressed: Fill::flat(hex(0x00D3E6)),
        danger_solid: Fill::flat(hex(0xD63C42)),
        danger_solid_hover: Fill::flat(hex(0xD0353D)),
        danger_solid_pressed: Fill::flat(hex(0xC2353B)),
        added_bg: hex(0x0F1D1B),
        removed_bg: hex(0x1E0D13),
        added_gutter: hex(0x143026),
        removed_gutter: hex(0x2F1318),
        deck: Fill::flat(hex(0x0F0C14)),
        deck_edge: hex(0x201B29),
        pane_header: hex(0x0A080D),
        pane_header_seam: hex(0x201B29),
        knob_edge: None,
    },
    deeper: Surfaces {
        bg: hex(0x060509),
        panel: hex(0x060509),
        pane: hex(0x0F0C14),
        elevated: hex(0x18151F),
        overlay: hex(0x18151F),
        field: hex(0x0A080E),
        track: hex(0x060509),
        switch_off: hex(0x2D2738),
        badge: hex(0x262131),
        border: hex(0x1E1A27),
        edge: hex(0x2A2435),
        raised_edge: hex(0x2B2637),
        overlay_edge: hex(0x2A2536),
        tab_edge: hex(0x262131),
        track_edge: hex(0x1B1723),
        pane_edge: Some(hex(0x1B1723)),
        keycap_edge: hex(0x040306),
        raised: Fill {
            top: hex(0x221D2C),
            bottom: hex(0x18151F),
        },
        raised_hover: Fill {
            top: hex(0x292334),
            bottom: hex(0x1D1926),
        },
        raised_pressed: Fill {
            top: hex(0x15121C),
            bottom: hex(0x1A1622),
        },
        thumb: Fill {
            top: hex(0x2A2435),
            bottom: hex(0x211C2B),
        },
        keycap: Fill {
            top: hex(0x262131),
            bottom: hex(0x1B1723),
        },
        tab: Fill {
            top: hex(0x1C1824),
            bottom: hex(0x15121C),
        },
        tooltip: Fill {
            top: hex(0x292334),
            bottom: hex(0x201B2A),
        },
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xDEDAE5),
        },
        primary: Fill {
            top: hex(0x64EEFF),
            bottom: hex(0x03D6E8),
        },
        primary_hover: Fill {
            top: hex(0x9CF4FF),
            bottom: hex(0x03DFF3),
        },
        primary_pressed: Fill {
            top: hex(0x02D3E6),
            bottom: hex(0x01CDDF),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC2353B),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0x142121),
        removed_bg: hex(0x231119),
        added_gutter: hex(0x18332C),
        removed_gutter: hex(0x33161E),
        deck: Fill {
            top: hex(0x16131C),
            bottom: hex(0x120F17),
        },
        deck_edge: hex(0x030204),
        pane_header: hex(0x131019),
        pane_header_seam: hex(0x194751),
        knob_edge: None,
    },
    text: hex(0xF1EEF8),
    text_dim: hex(0xC1B9D0),
    text_faint: hex(0xAAA2BB),
    accent: hex(0x2CECF8),
    accent2: hex(0xFD71CF),
    focus: hex(0x2CECF8),
    selection: hex(0x2CECF8),
    selection_alpha: 33,
    on_accent: hex(0x001018),
    success: hex(0x47F4A3),
    warning: hex(0xF7E343),
    danger: hex(0xFF7F75),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    control_border: hex(0x756F83),
    on_tooltip: hex(0xF1EEF8),
    on_tooltip_dim: hex(0xB8B5C0),
    hover_ink: hex(0xF1EEF8),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(4, 4, 5, 158),
    glass_alpha: 204,
    glass_luminance: 0.03,
    lanes: [
        hex(0x05D3DF),
        hex(0xFF8ED7),
        hex(0xC7A8FF),
        hex(0x3FDA91),
        hex(0xD4BC02),
        hex(0x8BBEFF),
        hex(0xFF9B92),
        hex(0x04D8BE),
        hex(0xFFA14C),
        hex(0x9CCE4F),
    ],
    syntax: Syntax {
        keyword: hex(0xFF8ED7),
        string: hex(0x63F0A8),
        comment: hex(0xA79EB9),
        number: hex(0xF5E05D),
        function: hex(0x49EAF5),
        type_: hex(0xD1A9FF),
        constant: hex(0xFFAF78),
    },
};

/// "Forest": Evergreen shade under warm cream text, lit by a sunlit-gold signature and lake-blue links.
pub const FOREST: Palette = Palette {
    name: "Forest",
    concept: "Evergreen shade under warm cream text, lit by a sunlit-gold signature and lake-blue links.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x08110D),
        panel: hex(0x0B1611),
        pane: hex(0x08110D),
        elevated: hex(0x101E18),
        overlay: hex(0x101E18),
        field: hex(0x09130E),
        track: hex(0x050E0A),
        switch_off: hex(0x22352D),
        badge: hex(0x1D3028),
        border: hex(0x172821),
        edge: hex(0x1D3028),
        raised_edge: hex(0x1D3028),
        overlay_edge: hex(0x1D3028),
        tab_edge: hex(0x1D3028),
        track_edge: hex(0x172821),
        pane_edge: None,
        keycap_edge: hex(0x040C08),
        raised: Fill::flat(hex(0x101E18)),
        raised_hover: Fill::flat(hex(0x16261F)),
        raised_pressed: Fill::flat(hex(0x0D1914)),
        thumb: Fill::flat(hex(0x192B23)),
        keycap: Fill::flat(hex(0x172821)),
        tab: Fill::flat(hex(0x101E18)),
        tooltip: Fill::flat(hex(0x1C2F27)),
        knob: Fill::flat(hex(0xFCFAF1)),
        primary: Fill::flat(hex(0xE5B340)),
        primary_hover: Fill::flat(hex(0xF2BF4F)),
        primary_pressed: Fill::flat(hex(0xDAA732)),
        danger_solid: Fill::flat(hex(0xD63C42)),
        danger_solid_hover: Fill::flat(hex(0xD0353D)),
        danger_solid_pressed: Fill::flat(hex(0xC2353B)),
        added_bg: hex(0x122318),
        removed_bg: hex(0x1C1513),
        added_gutter: hex(0x1C3322),
        removed_gutter: hex(0x2E1A18),
        deck: Fill::flat(hex(0x0B1611)),
        deck_edge: hex(0x172821),
        pane_header: hex(0x08110D),
        pane_header_seam: hex(0x172821),
        knob_edge: None,
    },
    deeper: Surfaces {
        bg: hex(0x050D09),
        panel: hex(0x050D09),
        pane: hex(0x0B1611),
        elevated: hex(0x12201A),
        overlay: hex(0x12201A),
        field: hex(0x08110D),
        track: hex(0x050D09),
        switch_off: hex(0x22352D),
        badge: hex(0x1C2F27),
        border: hex(0x16261F),
        edge: hex(0x1F322A),
        raised_edge: hex(0x21342C),
        overlay_edge: hex(0x20332B),
        tab_edge: hex(0x1C2F27),
        track_edge: hex(0x14231C),
        pane_edge: Some(hex(0x14231C)),
        keycap_edge: hex(0x030906),
        raised: Fill {
            top: hex(0x192B23),
            bottom: hex(0x12201A),
        },
        raised_hover: Fill {
            top: hex(0x1E3129),
            bottom: hex(0x16261F),
        },
        raised_pressed: Fill {
            top: hex(0x101D17),
            bottom: hex(0x13221C),
        },
        thumb: Fill {
            top: hex(0x1F322A),
            bottom: hex(0x182A22),
        },
        keycap: Fill {
            top: hex(0x1C2F27),
            bottom: hex(0x14231D),
        },
        tab: Fill {
            top: hex(0x15241D),
            bottom: hex(0x101D17),
        },
        tooltip: Fill {
            top: hex(0x1E3129),
            bottom: hex(0x182921),
        },
        knob: Fill {
            top: hex(0xFCFAF1),
            bottom: hex(0xDCE8E2),
        },
        primary: Fill {
            top: hex(0xF7C454),
            bottom: hex(0xDCAA35),
        },
        primary_hover: Fill {
            top: hex(0xFFD06E),
            bottom: hex(0xE5B23F),
        },
        primary_pressed: Fill {
            top: hex(0xDAA832),
            bottom: hex(0xD4A22A),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC2353B),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0x15281B),
        removed_bg: hex(0x1F1A16),
        added_gutter: hex(0x1E3725),
        removed_gutter: hex(0x301E1B),
        deck: Fill {
            top: hex(0x131E19),
            bottom: hex(0x0E1914),
        },
        deck_edge: hex(0x020704),
        pane_header: hex(0x0F1B15),
        pane_header_seam: hex(0x464527),
        knob_edge: None,
    },
    text: hex(0xF0EDDE),
    text_dim: hex(0xBDBEAA),
    text_faint: hex(0xA2AA96),
    accent: hex(0xE7BF57),
    accent2: hex(0x81C9E9),
    focus: hex(0xEAC25A),
    selection: hex(0xE7BF57),
    selection_alpha: 33,
    on_accent: hex(0x1E1908),
    success: hex(0x7DDC86),
    warning: hex(0xFFA05C),
    danger: hex(0xFB8087),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    control_border: hex(0x6D7B6E),
    on_tooltip: hex(0xF0EDDE),
    on_tooltip_dim: hex(0xB5B8AB),
    hover_ink: hex(0xF0EDDE),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(4, 4, 5, 158),
    glass_alpha: 204,
    glass_luminance: 0.03,
    lanes: [
        hex(0xD9B24F),
        hex(0x4FC7F6),
        hex(0xF395C3),
        hex(0x80CD86),
        hex(0xF5A166),
        hex(0xA9AFFF),
        hex(0xFD9599),
        hex(0x39D1C2),
        hex(0xADC363),
        hex(0xD99DEB),
    ],
    syntax: Syntax {
        keyword: hex(0x92DCA1),
        string: hex(0xD2D982),
        comment: hex(0x9AA896),
        number: hex(0xF4C677),
        function: hex(0x87CFF0),
        type_: hex(0xCCB9F1),
        constant: hex(0xF2ACCA),
    },
};

/// "Paper": Warm cream paper with fountain-pen ink blue and sepia, set like a printed page.
pub const PAPER: Palette = Palette {
    name: "Paper",
    concept: "Warm cream paper with fountain-pen ink blue and sepia, set like a printed page.",
    dark: false,
    strict: false,
    standard: Surfaces {
        bg: hex(0xFAF5ED),
        panel: hex(0xF3EDE4),
        pane: hex(0xFAF5ED),
        elevated: hex(0xFAF5ED),
        overlay: hex(0xFAF5ED),
        field: hex(0xFAF5ED),
        track: hex(0xEBE5DB),
        switch_off: hex(0xD3CBBD),
        badge: hex(0xE9E2D7),
        border: hex(0xE3DCD0),
        edge: hex(0xD8D0C2),
        raised_edge: hex(0xD8D0C2),
        overlay_edge: hex(0xDFD8CB),
        tab_edge: hex(0xDFD8CB),
        track_edge: hex(0xE1DACD),
        pane_edge: None,
        keycap_edge: hex(0xCCC5B7),
        raised: Fill::flat(hex(0xFAF5ED)),
        raised_hover: Fill::flat(hex(0xF2ECE3)),
        raised_pressed: Fill::flat(hex(0xEBE5DB)),
        thumb: Fill::flat(hex(0xFAF5ED)),
        keycap: Fill::flat(hex(0xEFE9DF)),
        tab: Fill::flat(hex(0xFAF5ED)),
        tooltip: Fill::flat(hex(0x291F18)),
        knob: Fill::flat(hex(0xFFFEFD)),
        primary: Fill::flat(hex(0x2B56B3)),
        primary_hover: Fill::flat(hex(0x224CA8)),
        primary_pressed: Fill::flat(hex(0x18419C)),
        danger_solid: Fill::flat(hex(0xD73B40)),
        danger_solid_hover: Fill::flat(hex(0xCC3238)),
        danger_solid_pressed: Fill::flat(hex(0xB92B31)),
        added_bg: hex(0xE9EADD),
        removed_bg: hex(0xF7E7DF),
        added_gutter: hex(0xDCDFD0),
        removed_gutter: hex(0xF5D9D4),
        deck: Fill::flat(hex(0xF3EDE4)),
        deck_edge: hex(0xE3DCD0),
        pane_header: hex(0xFAF5ED),
        pane_header_seam: hex(0xE3DCD0),
        knob_edge: Some(hex(0x9B9385)),
    },
    deeper: Surfaces {
        bg: hex(0xEAE4D9),
        panel: hex(0xEAE4D9),
        pane: hex(0xFAF5ED),
        elevated: hex(0xFAF5ED),
        overlay: hex(0xFAF5ED),
        field: hex(0xFAF5ED),
        track: hex(0xE1DACE),
        switch_off: hex(0xD3CBBD),
        badge: hex(0xE9E2D7),
        border: hex(0xE2DBCF),
        edge: hex(0xD4CCBE),
        raised_edge: hex(0xD4CCBE),
        overlay_edge: hex(0xDED7CA),
        tab_edge: hex(0xDAD2C4),
        track_edge: hex(0xDAD2C4),
        pane_edge: Some(hex(0xE2DBCF)),
        keycap_edge: hex(0xCAC2B4),
        raised: Fill {
            top: hex(0xFAF5ED),
            bottom: hex(0xF1EBE1),
        },
        raised_hover: Fill {
            top: hex(0xFAF5ED),
            bottom: hex(0xECE6DB),
        },
        raised_pressed: Fill {
            top: hex(0xEAE4D9),
            bottom: hex(0xEFE9DF),
        },
        thumb: Fill {
            top: hex(0xFAF5ED),
            bottom: hex(0xF2EDE4),
        },
        keycap: Fill {
            top: hex(0xFAF5ED),
            bottom: hex(0xEDE7DD),
        },
        tab: Fill {
            top: hex(0xFAF5ED),
            bottom: hex(0xF2EDE4),
        },
        tooltip: Fill::flat(hex(0x291F18)),
        knob: Fill {
            top: hex(0xFFFEFD),
            bottom: hex(0xEEE8DE),
        },
        primary: Fill {
            top: hex(0x3A67C4),
            bottom: hex(0x234EAA),
        },
        primary_hover: Fill {
            top: hex(0x335FBC),
            bottom: hex(0x1E47A3),
        },
        primary_pressed: Fill {
            top: hex(0x1E47A3),
            bottom: hex(0x234EAA),
        },
        danger_solid: Fill {
            top: hex(0xD53D43),
            bottom: hex(0xBA272D),
        },
        danger_solid_hover: Fill {
            top: hex(0xCF363E),
            bottom: hex(0xB41F28),
        },
        danger_solid_pressed: Fill {
            top: hex(0xBF3036),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0xE9EADD),
        removed_bg: hex(0xF7E7DF),
        added_gutter: hex(0xDCDFD0),
        removed_gutter: hex(0xF5D9D4),
        deck: Fill {
            top: hex(0xF6F0E5),
            bottom: hex(0xF1EBE0),
        },
        deck_edge: hex(0xD1CCC1),
        pane_header: hex(0xF5F0E8),
        pane_header_seam: hex(0xBEC8DD),
        knob_edge: Some(hex(0x8C8476)),
    },
    text: hex(0x291F18),
    text_dim: hex(0x625448),
    text_faint: hex(0x635649),
    accent: hex(0x2451A9),
    accent2: hex(0x764627),
    focus: hex(0x3263C3),
    selection: hex(0x2451A9),
    selection_alpha: 26,
    on_accent: hex(0xFFFDFA),
    success: hex(0x286323),
    warning: hex(0x775000),
    danger: hex(0xA3212C),
    danger_base: hex(0xD93D42),
    on_danger: hex(0xFFFDFA),
    control_border: hex(0x807668),
    on_tooltip: hex(0xF6F3EC),
    on_tooltip_dim: hex(0xBDB8B1),
    hover_ink: hex(0x291F18),
    shadow: hex(0x120F0B),
    primary_shadow: hex(0x0C1F47),
    danger_shadow: hex(0x781414),
    scrim: Color32::from_rgba_premultiplied(8, 7, 5, 82),
    glass_alpha: 214,
    glass_luminance: 0.8,
    lanes: [
        hex(0x3962AE),
        hex(0x9C4A00),
        hex(0x93427C),
        hex(0x31762B),
        hex(0x805F00),
        hex(0x7150A3),
        hex(0xA24042),
        hex(0x027378),
        hex(0x636C00),
        hex(0xA24138),
    ],
    syntax: Syntax {
        keyword: hex(0x2852A3),
        string: hex(0x38692D),
        comment: hex(0x726354),
        number: hex(0x924413),
        function: hex(0x644792),
        type_: hex(0x02686C),
        constant: hex(0x8F3662),
    },
};

/// "Contrast": Pure black, white text, strong outlines and a yellow focus ring, for low vision and glare.
pub const CONTRAST: Palette = Palette {
    name: "Contrast",
    concept: "Pure black, white text, strong outlines and a yellow focus ring, for low vision and glare.",
    dark: true,
    strict: true,
    standard: Surfaces {
        bg: hex(0x000000),
        panel: hex(0x0A0A0A),
        pane: hex(0x000000),
        elevated: hex(0x141414),
        overlay: hex(0x141414),
        field: hex(0x050505),
        track: hex(0x000000),
        switch_off: hex(0x333333),
        badge: hex(0x2A2A2A),
        border: hex(0x747474),
        edge: hex(0x989898),
        raised_edge: hex(0x989898),
        overlay_edge: hex(0xCACACA),
        tab_edge: hex(0x989898),
        track_edge: hex(0x747474),
        pane_edge: None,
        keycap_edge: hex(0x747474),
        raised: Fill::flat(hex(0x1A1A1A)),
        raised_hover: Fill::flat(hex(0x242424)),
        raised_pressed: Fill::flat(hex(0x111111)),
        thumb: Fill::flat(hex(0x2C2C2C)),
        keycap: Fill::flat(hex(0x1D1D1D)),
        tab: Fill::flat(hex(0x1A1A1A)),
        tooltip: Fill::flat(hex(0x1D1D1D)),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x71C8FF)),
        primary_hover: Fill::flat(hex(0x91D3FF)),
        primary_pressed: Fill::flat(hex(0x56BEFB)),
        danger_solid: Fill::flat(hex(0xFF8987)),
        danger_solid_hover: Fill::flat(hex(0xFF9C99)),
        danger_solid_pressed: Fill::flat(hex(0xF37A79)),
        added_bg: hex(0x08170D),
        removed_bg: hex(0x150607),
        added_gutter: hex(0x0E2A19),
        removed_gutter: hex(0x270C0D),
        deck: Fill::flat(hex(0x0A0A0A)),
        deck_edge: hex(0x747474),
        pane_header: hex(0x000000),
        pane_header_seam: hex(0x747474),
        knob_edge: None,
    },
    deeper: Surfaces {
        bg: hex(0x000000),
        panel: hex(0x000000),
        pane: hex(0x000000),
        elevated: hex(0x161616),
        overlay: hex(0x161616),
        field: hex(0x050505),
        track: hex(0x000000),
        switch_off: hex(0x333333),
        badge: hex(0x2A2A2A),
        border: hex(0x747474),
        edge: hex(0x989898),
        raised_edge: hex(0x989898),
        overlay_edge: hex(0xCACACA),
        tab_edge: hex(0x989898),
        track_edge: hex(0x747474),
        pane_edge: Some(hex(0x747474)),
        keycap_edge: hex(0x747474),
        raised: Fill {
            top: hex(0x262626),
            bottom: hex(0x1A1A1A),
        },
        raised_hover: Fill {
            top: hex(0x2E2E2E),
            bottom: hex(0x242424),
        },
        raised_pressed: Fill {
            top: hex(0x111111),
            bottom: hex(0x161616),
        },
        thumb: Fill {
            top: hex(0x333333),
            bottom: hex(0x2A2A2A),
        },
        keycap: Fill {
            top: hex(0x2A2A2A),
            bottom: hex(0x1D1D1D),
        },
        tab: Fill {
            top: hex(0x202020),
            bottom: hex(0x1A1A1A),
        },
        tooltip: Fill {
            top: hex(0x262626),
            bottom: hex(0x1D1D1D),
        },
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xE8E8E8),
        },
        primary: Fill {
            top: hex(0x9CD7FF),
            bottom: hex(0x59C0FE),
        },
        primary_hover: Fill {
            top: hex(0xB4E1FF),
            bottom: hex(0x70C8FF),
        },
        primary_pressed: Fill {
            top: hex(0x57BEFC),
            bottom: hex(0x50B8F6),
        },
        danger_solid: Fill {
            top: hex(0xFFA19E),
            bottom: hex(0xF87E7D),
        },
        danger_solid_hover: Fill {
            top: hex(0xFFB1AD),
            bottom: hex(0xFF8886),
        },
        danger_solid_pressed: Fill {
            top: hex(0xF37A79),
            bottom: hex(0xF87E7D),
        },
        added_bg: hex(0x08170D),
        removed_bg: hex(0x150607),
        added_gutter: hex(0x0E2A19),
        removed_gutter: hex(0x270C0D),
        deck: Fill {
            top: hex(0x141414),
            bottom: hex(0x0A0A0A),
        },
        deck_edge: hex(0x747474),
        pane_header: hex(0x0F0F0F),
        pane_header_seam: hex(0xFDEA4C),
        knob_edge: None,
    },
    text: hex(0xFFFFFF),
    text_dim: hex(0xE9E9E9),
    text_faint: hex(0xD6D6D6),
    accent: hex(0x7ECFFF),
    accent2: hex(0xFDEA4C),
    focus: hex(0xFDEA4C),
    selection: hex(0x7ECFFF),
    selection_alpha: 64,
    on_accent: hex(0x000000),
    success: hex(0x54FC93),
    warning: hex(0xFFB573),
    danger: hex(0xFF8C8A),
    danger_base: hex(0xF3565B),
    on_danger: hex(0x000000),
    control_border: hex(0xCACACA),
    on_tooltip: hex(0xFFFFFF),
    on_tooltip_dim: hex(0xC0C0C0),
    hover_ink: hex(0xFFFFFF),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(0, 0, 0, 200),
    glass_alpha: 0,
    glass_luminance: 0.03,
    lanes: [
        hex(0x7ECFFF),
        hex(0xD8C740),
        hex(0xFFA0E0),
        hex(0x70E093),
        hex(0xFFB169),
        hex(0xC3BAFF),
        hex(0xFFAAA7),
        hex(0x01E1D8),
        hex(0xAED561),
        hex(0xFFA6C1),
    ],
    syntax: Syntax {
        keyword: hex(0x8ED4FF),
        string: hex(0x87F7A9),
        comment: hex(0xC4C4C4),
        number: hex(0xFFC38F),
        function: hex(0xFDEA4C),
        type_: hex(0xD4BDFF),
        constant: hex(0xFEADDB),
    },
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

    /// The theme for a `--theme` argument: the display name or the variant name, any case.
    pub fn from_name(name: &str) -> Option<ThemeId> {
        Self::ALL.into_iter().find(|t| {
            t.name().eq_ignore_ascii_case(name) || format!("{t:?}").eq_ignore_ascii_case(name)
        })
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

/// Surfaces and material: Standard (concept 1, flat two-tone chrome, flush panes) or Deeper
/// (concept 4, floating card panes, gradient controls and layered shadows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Standard,
    Deeper,
}

impl Style {
    pub const ALL: [Style; 2] = [Style::Standard, Style::Deeper];

    pub fn label(self) -> &'static str {
        match self {
            Style::Standard => "Standard",
            Style::Deeper => "Deeper",
        }
    }

    /// The style for a `--style` argument.
    pub fn from_name(name: &str) -> Option<Style> {
        Self::ALL
            .into_iter()
            .find(|s| s.label().eq_ignore_ascii_case(name))
    }
}

/// Row heights, radii, toolbar layout and type size: Comfortable (concept 1) or Dense (concept 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Density {
    #[default]
    Comfortable,
    Dense,
}

impl Density {
    pub const ALL: [Density; 2] = [Density::Comfortable, Density::Dense];

    pub fn label(self) -> &'static str {
        match self {
            Density::Comfortable => "Comfortable",
            Density::Dense => "Dense",
        }
    }

    /// The density for a `--density` argument.
    pub fn from_name(name: &str) -> Option<Density> {
        Self::ALL
            .into_iter()
            .find(|d| d.label().eq_ignore_ascii_case(name))
    }
}

/// What renders, resolved from the settings, the theme and the graphics backend each time any of
/// them changes ([`Appearance::resolve`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Appearance {
    pub style: Style,
    pub density: Density,
    /// Blurred translucent floating layers.
    pub glass: bool,
    pub reduce_motion: bool,
}

/// Why Translucent surfaces cannot be on right now, for the setting's disabled state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassBlocked {
    /// The theme never uses glass (Contrast).
    Theme,
    /// The operating system asks for reduced transparency.
    System,
    /// The graphics backend cannot blur (or this build has no blur yet).
    Backend,
}

impl GlassBlocked {
    pub fn reason(self) -> &'static str {
        match self {
            GlassBlocked::Theme => "Not available with the Contrast theme",
            GlassBlocked::System => "Your system asks for reduced transparency",
            GlassBlocked::Backend => "Not supported by this graphics driver",
        }
    }
}

impl Appearance {
    /// The default options with motion off: screenshots and tests, where nothing may animate.
    #[cfg_attr(not(test), expect(dead_code, reason = "tests install it"))]
    pub const STILL: Self = Self {
        style: Style::Standard,
        density: Density::Comfortable,
        glass: false,
        reduce_motion: true,
    };

    /// What renders for these choices. `translucent` is the user's switch; glass also needs a
    /// theme that allows it, a backend that can blur and no system request for reduced
    /// transparency (`None`: the system did not say).
    pub fn resolve(
        theme: ThemeId,
        style: Style,
        density: Density,
        translucent: bool,
        reduce_motion: bool,
        glass_available: bool,
        reduced_transparency: Option<bool>,
    ) -> Self {
        let blocked = glass_blocked(theme, glass_available, reduced_transparency);
        Self {
            style,
            density,
            glass: translucent && blocked.is_none(),
            reduce_motion,
        }
    }
}

/// Why glass is off whatever the switch says, if it is.
pub fn glass_blocked(
    theme: ThemeId,
    glass_available: bool,
    reduced_transparency: Option<bool>,
) -> Option<GlassBlocked> {
    if theme.palette().glass_alpha == 0 {
        Some(GlassBlocked::Theme)
    } else if reduced_transparency == Some(true) {
        Some(GlassBlocked::System)
    } else if !glass_available {
        Some(GlassBlocked::Backend)
    } else {
        None
    }
}

/// The active theme. One window, one look: globals keep [`palette`] and [`metrics`] cheap.
struct Look {
    theme: AtomicU8,
    style: AtomicU8,
    density: AtomicU8,
    glass: AtomicBool,
}

impl Look {
    const fn new() -> Self {
        Self {
            theme: AtomicU8::new(0),
            style: AtomicU8::new(0),
            density: AtomicU8::new(0),
            glass: AtomicBool::new(false),
        }
    }
}

#[cfg(not(test))]
static LOOK: Look = Look::new();

#[cfg(not(test))]
fn look<R>(f: impl FnOnce(&Look) -> R) -> R {
    f(&LOOK)
}

// Tests run in parallel threads, each with its own look, so one test's theme or density never
// leaks into another's.
#[cfg(test)]
thread_local! {
    static LOOK: Look = const { Look::new() };
}

#[cfg(test)]
fn look<R>(f: impl FnOnce(&Look) -> R) -> R {
    LOOK.with(f)
}

/// The theme currently shown.
pub fn active() -> ThemeId {
    ThemeId::ALL[usize::from(look(|l| l.theme.load(Ordering::Relaxed))) % ThemeId::ALL.len()]
}

/// The style currently shown.
pub fn style() -> Style {
    if look(|l| l.style.load(Ordering::Relaxed)) == 1 {
        Style::Deeper
    } else {
        Style::Standard
    }
}

/// The density currently shown.
pub fn density() -> Density {
    if look(|l| l.density.load(Ordering::Relaxed)) == 1 {
        Density::Dense
    } else {
        Density::Comfortable
    }
}

/// The options currently rendering.
pub fn appearance() -> Appearance {
    Appearance {
        style: style(),
        density: density(),
        glass: look(|l| l.glass.load(Ordering::Relaxed)),
        reduce_motion: super::motion::reduced(),
    }
}

impl Palette {
    /// The surfaces of `style`.
    pub fn surfaces(&self, style: Style) -> &Surfaces {
        match style {
            Style::Standard => &self.standard,
            Style::Deeper => &self.deeper,
        }
    }

    /// The surfaces of the active style.
    pub fn s(&self) -> &Surfaces {
        self.surfaces(style())
    }

    /// Lane colour for a graph colour index.
    pub fn lane(&self, index: u16) -> Color32 {
        self.lanes[usize::from(index) % self.lanes.len()]
    }

    fn overlay_alpha(&self, dark: u8, light: u8) -> Color32 {
        Palette::tint(self.hover_ink, if self.dark { dark } else { light })
    }

    /// Hovered rows and quiet controls: a translucent tint drawn over any surface.
    pub fn hover(&self) -> Color32 {
        self.overlay_alpha(9, 9)
    }

    /// Hovered tool and icon buttons, menu highlight, ghost buttons.
    pub fn hover_strong(&self) -> Color32 {
        self.overlay_alpha(15, 14)
    }

    /// Pressed rows and quiet controls.
    pub fn pressed(&self) -> Color32 {
        self.overlay_alpha(22, 20)
    }

    /// Even history rows in Dense.
    pub fn zebra(&self) -> Color32 {
        self.overlay_alpha(5, 6)
    }

    /// Fill of the selected row in lists, the graph, the sidebar, the palette and diffs, and of
    /// egui's selected values. Every text colour stays readable on it (see the contrast test).
    pub fn selected(&self) -> Color32 {
        Palette::tint(self.selection, self.selection_alpha)
    }

    /// Status tints (16%): dialog glyph discs, banners, callouts.
    pub fn success_tint(&self) -> Color32 {
        Palette::tint(self.success, 41)
    }

    pub fn warning_tint(&self) -> Color32 {
        Palette::tint(self.warning, 41)
    }

    /// Concept 1's danger tint, from `danger_base`: the destructive dialog's glyph disc.
    pub fn danger_tint(&self) -> Color32 {
        Palette::tint(self.danger_base, 41)
    }

    pub fn accent_tint(&self) -> Color32 {
        Palette::tint(self.accent, 41)
    }

    /// Highlight of a danger menu item (15% of `danger_base`).
    pub fn danger_highlight(&self) -> Color32 {
        Palette::tint(self.danger_base, 38)
    }

    /// The surfaces a coloured component (pill, badge, avatar) can sit on in `s`: plain surfaces,
    /// hovered rows and selected rows.
    pub fn component_surfaces(&self, s: &Surfaces) -> [Color32; 8] {
        let sel = self.selected();
        let hover = self.hover();
        [
            s.pane,
            s.panel,
            s.elevated,
            s.overlay,
            over(hover, s.pane),
            over(hover, s.panel),
            over(sel, s.pane),
            over(sel, s.panel),
        ]
    }

    /// Text or an icon in colour `fg` drawn on the translucent `fill` in the active style: `fg`
    /// moved towards the text colour just enough to read at 4.5:1 on that fill over every
    /// component surface, so it keeps its hue wherever the component is drawn.
    pub fn ink(&self, fg: Color32, fill: Color32) -> Color32 {
        let style = style();
        let key = (self.name, style, fg, fill);
        if let Some(c) = INKS.with(|m| m.borrow().get(&key).copied()) {
            return c;
        }
        let ink = self.ink_on(self.surfaces(style), fg, fill);
        INKS.with(|m| m.borrow_mut().insert(key, ink));
        ink
    }

    /// [`Palette::ink`] for the surfaces `s`, uncached.
    pub fn ink_on(&self, s: &Surfaces, fg: Color32, fill: Color32) -> Color32 {
        let behind = self.component_surfaces(s).map(|b| over(fill, b));
        let readable = |c: Color32| behind.iter().all(|b| contrast(c, *b) >= 4.5);
        (0..=20)
            .map(|step| mix(fg, self.text, step as f32 / 20.0))
            .find(|c| readable(*c))
            .unwrap_or(self.text)
    }

    /// Fill of a ref pill in colour `c` (15%); the current branch is stronger (26%).
    pub const fn pill_fill(c: Color32, current: bool) -> Color32 {
        Palette::tint(c, if current { 66 } else { 38 })
    }

    /// The ring of a ref pill (42%), the same for the current branch.
    pub const fn pill_border(c: Color32) -> Color32 {
        Palette::tint(c, 107)
    }

    /// Fill of an initials avatar in colour `c` (24%).
    pub fn avatar_fill(&self, c: Color32) -> Color32 {
        Palette::tint(c, 61)
    }

    /// Fill of a count badge in colour `c` (24%).
    pub const fn badge_fill(c: Color32) -> Color32 {
        Palette::tint(c, 61)
    }

    /// Fill of a danger count badge (22% of `danger_base`), labelled in `ink(danger, ..)`.
    pub fn danger_badge_fill(&self) -> Color32 {
        Palette::tint(self.danger_base, 56)
    }

    /// Fill of a file status letter in colour `c` (18%).
    pub const fn change_fill(c: Color32) -> Color32 {
        Palette::tint(c, 46)
    }

    /// A translucent version of `c` for fills behind text (premultiplied).
    pub const fn tint(c: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied_const(c.r(), c.g(), c.b(), alpha)
    }

    /// The material of `level` in `style` for this theme: Contrast has no highlights.
    pub fn material(&self, style: Style, level: Elevation) -> Material {
        let m = *material(style, self.dark, level);
        if self.strict {
            Material {
                highlight: 0,
                bottom_line: 0,
                ..m
            }
        } else {
            m
        }
    }

    /// The colour a shadow layer's tint resolves to in `style`.
    pub fn shadow_color(&self, tint: ShadowTint) -> Color32 {
        match tint {
            ShadowTint::Shadow => self.shadow,
            ShadowTint::Black => Color32::BLACK,
            ShadowTint::Primary => self.primary_shadow,
            ShadowTint::Danger => self.danger_shadow,
            ShadowTint::PrimaryGlow => self.standard.primary.top,
            ShadowTint::DangerGlow => self.standard.danger_solid.top,
        }
    }

    /// Ring of floating layers: `overlay_edge`, or `control_border` in Contrast.
    pub fn floating_ring(&self, s: &Surfaces) -> Color32 {
        if self.strict {
            self.control_border
        } else {
            s.overlay_edge
        }
    }

    // Accessors kept for views written before the component library; the migration moves them
    // onto `s()` and the kit, then deletes these.

    /// The ground views painted their content on before the kit (`Surfaces::pane`: the window
    /// ground in Standard, the floating card in Deeper).
    #[expect(
        clippy::misnamed_getters,
        reason = "views say bg for their content ground until the migration"
    )]
    pub fn bg(&self) -> Color32 {
        self.s().pane
    }

    /// Chrome in the active style (`Surfaces::panel`).
    pub fn panel(&self) -> Color32 {
        self.s().panel
    }

    /// Raised surface in the active style (`Surfaces::elevated`).
    pub fn elevated(&self) -> Color32 {
        self.s().elevated
    }

    /// Seam in the active style (`Surfaces::border`).
    pub fn border(&self) -> Color32 {
        self.s().border
    }

    pub fn added_bg(&self) -> Color32 {
        self.s().added_bg
    }

    pub fn removed_bg(&self) -> Color32 {
        self.s().removed_bg
    }

    /// Primary fill, top end (`Surfaces::primary`).
    pub fn button(&self) -> Color32 {
        self.s().primary.top
    }

    /// Label on the primary fill (`on_accent`).
    pub fn on_button(&self) -> Color32 {
        self.on_accent
    }

    /// The scrim behind modal dialogs, the settings screen, the palette and the cheatsheet.
    pub fn backdrop(&self) -> Color32 {
        self.scrim
    }

    /// Ring of a raised control (`Surfaces::raised_edge`).
    pub fn control_edge(&self) -> Color32 {
        self.s().raised_edge
    }

    /// Contact shadow of a raised control, as one egui shadow.
    pub fn control_shadow(&self) -> Shadow {
        let style = style();
        let m = self.material(style, Elevation::Control);
        m.layers.first().map_or(Shadow::NONE, |l| Shadow {
            offset: [0, l.dy],
            blur: l.blur,
            spread: 0,
            color: Palette::tint(self.shadow_color(l.tint), l.alpha),
        })
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
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Translucent surfaces: a blurred backdrop sample after the composite shader's luminance clamp.
/// Dark themes scale it down to at most `cap` relative luminance, light themes lift it towards
/// white to at least `cap`, in linear light. Shared by the glass shader's uniform and the test.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the glass renderer's luminance clamp uniform comes with the blur"
    )
)]
pub fn clamp_backdrop(c: Color32, dark: bool, cap: f32) -> Color32 {
    let to_lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let to_srgb = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0 + 0.5).floor() as u8
    };
    let mut lin = [to_lin(c.r()), to_lin(c.g()), to_lin(c.b())];
    let y = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
    let cap = f64::from(cap);
    if dark && y > cap {
        lin = lin.map(|x| x * cap / y);
    } else if !dark && y < cap {
        let t = (cap - y) / (1.0 - y);
        lin = lin.map(|x| x + (1.0 - x) * t);
    }
    Color32::from_rgb(to_srgb(lin[0]), to_srgb(lin[1]), to_srgb(lin[2]))
}

/// Theme, style, colour and fill of a worked-out ink.
type InkKey = (&'static str, Style, Color32, Color32);

thread_local! {
    /// Inks already worked out, per theme, style, colour and fill: views ask for the same few
    /// every frame.
    static INKS: std::cell::RefCell<std::collections::HashMap<InkKey, Color32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The palette of the active theme.
pub fn palette(_ctx: &egui::Context) -> &'static Palette {
    active().palette()
}

/// The surfaces of the active theme and style.
pub fn surfaces(_ctx: &egui::Context) -> &'static Surfaces {
    active().palette().surfaces(style())
}

// ------------------------------------------------------------------------------------ material

/// One soft shadow layer in egui units, painted bottom layer first, before the surface. `spread`
/// may be negative (CSS `0 10px 28px -8px`): `kit::paint::shadow` shrinks the rect by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowLayer {
    pub dy: i8,
    pub blur: u8,
    pub spread: i8,
    pub alpha: u8,
    pub tint: ShadowTint,
}

/// The colour a shadow layer takes ([`Palette::shadow_color`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowTint {
    /// `Palette::shadow`.
    Shadow,
    Black,
    /// `Palette::primary_shadow`.
    Primary,
    /// `Palette::danger_shadow`.
    Danger,
    /// The theme's Standard primary rest colour (Deeper's coloured glow).
    PrimaryGlow,
    /// The theme's Standard danger rest colour.
    DangerGlow,
}

/// What a level paints besides its fill: a 1 px inner top highlight (white at `highlight` alpha),
/// a 1 px inner bottom line (`bottom_line`, light Deeper controls), an inner recess shadow
/// (fields in Deeper) and the outer layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Material {
    pub highlight: u8,
    pub bottom_line: u8,
    pub recess: u8,
    pub layers: &'static [ShadowLayer],
}

/// Elevation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Elevation {
    /// Secondary button, select trigger.
    Control,
    /// Segmented thumb.
    Thumb,
    /// Switch thumb.
    Knob,
    /// Active document tab.
    Tab,
    /// Primary button.
    Primary,
    /// Danger button.
    Danger,
    /// Menus, select list, popover, tooltip, toast, dialog, palette, cheatsheet.
    Floating,
    /// Floating panes and cards (Deeper).
    Pane,
    /// Text fields (Deeper's recess).
    Field,
    /// The top deck (Deeper).
    Deck,
}

impl Elevation {
    pub const ALL: [Elevation; 10] = [
        Elevation::Control,
        Elevation::Thumb,
        Elevation::Knob,
        Elevation::Tab,
        Elevation::Primary,
        Elevation::Danger,
        Elevation::Floating,
        Elevation::Pane,
        Elevation::Field,
        Elevation::Deck,
    ];
}

const fn layer(dy: i8, blur: u8, spread: i8, alpha: u8) -> ShadowLayer {
    ShadowLayer {
        dy,
        blur,
        spread,
        alpha,
        tint: ShadowTint::Shadow,
    }
}

const fn tinted(dy: i8, blur: u8, spread: i8, alpha: u8, tint: ShadowTint) -> ShadowLayer {
    ShadowLayer {
        dy,
        blur,
        spread,
        alpha,
        tint,
    }
}

const fn mat(highlight: u8, layers: &'static [ShadowLayer]) -> Material {
    Material {
        highlight,
        bottom_line: 0,
        recess: 0,
        layers,
    }
}

const NONE: Material = mat(0, &[]);

/// The material of `level` in `style`, for dark or light themes (concept 1 and concept 4).
pub fn material(style: Style, dark: bool, level: Elevation) -> &'static Material {
    use Elevation as E;
    use ShadowTint as T;
    static STD_DARK: [Material; 10] = [
        mat(11, &[layer(1, 1, 0, 77)]),
        mat(13, &[layer(1, 2, 0, 115)]),
        mat(0, &[layer(1, 2, 0, 115)]),
        NONE,
        mat(46, &[layer(1, 2, 0, 102)]),
        mat(41, &[layer(1, 2, 0, 102)]),
        mat(0, &[layer(10, 28, -8, 166), layer(2, 6, 0, 77)]),
        NONE,
        NONE,
        NONE,
    ];
    static STD_LIGHT: [Material; 10] = [
        mat(0, &[layer(1, 2, 0, 15)]),
        mat(0, &[layer(1, 2, 0, 26), layer(0, 0, 1, 13)]),
        // Concept 1's contact shadow plus a soft halo (owner's amendment, with the knob ring).
        mat(
            0,
            &[tinted(1, 2, 0, 56, T::Black), tinted(2, 6, 0, 20, T::Black)],
        ),
        NONE,
        mat(46, &[tinted(1, 2, 0, 51, T::Primary)]),
        mat(0, &[tinted(1, 2, 0, 51, T::Danger)]),
        mat(0, &[layer(12, 32, -10, 56), layer(2, 6, 0, 15)]),
        NONE,
        NONE,
        NONE,
    ];
    static DEEP_DARK: [Material; 10] = [
        mat(19, &[layer(1, 1, 0, 102), layer(3, 8, -2, 115)]),
        mat(23, &[layer(1, 2, 0, 128), layer(2, 6, 0, 89)]),
        mat(0, &[layer(1, 2, 0, 128), layer(2, 6, 0, 77)]),
        mat(15, &[layer(2, 6, -2, 128)]),
        mat(
            66,
            &[layer(1, 1, 0, 102), tinted(5, 14, -5, 166, T::PrimaryGlow)],
        ),
        mat(
            56,
            &[layer(1, 1, 0, 102), tinted(5, 14, -6, 153, T::DangerGlow)],
        ),
        mat(13, &[layer(20, 44, -14, 217), layer(4, 12, 0, 102)]),
        mat(9, &[layer(1, 2, 0, 115), layer(8, 22, -12, 179)]),
        Material {
            highlight: 0,
            bottom_line: 0,
            recess: 71,
            layers: &[],
        },
        mat(13, &[layer(1, 2, 0, 115), layer(6, 18, -8, 153)]),
    ];
    static DEEP_LIGHT: [Material; 10] = [
        Material {
            highlight: 0,
            bottom_line: 10,
            recess: 0,
            layers: &[layer(1, 1, 0, 15), layer(2, 6, -2, 31)],
        },
        mat(0, &[layer(1, 1, 0, 20), layer(2, 6, -2, 38)]),
        mat(
            0,
            &[tinted(1, 2, 0, 51, T::Black), tinted(2, 5, 0, 26, T::Black)],
        ),
        mat(0, &[layer(1, 2, 0, 20)]),
        mat(
            64,
            &[
                tinted(1, 1, 0, 51, T::Primary),
                tinted(5, 12, -5, 128, T::PrimaryGlow),
            ],
        ),
        mat(
            56,
            &[
                tinted(1, 1, 0, 51, T::Danger),
                tinted(5, 12, -6, 115, T::DangerGlow),
            ],
        ),
        mat(0, &[layer(20, 44, -16, 71), layer(3, 10, 0, 20)]),
        mat(0, &[layer(1, 2, 0, 15), layer(8, 22, -14, 56)]),
        Material {
            highlight: 0,
            bottom_line: 0,
            recess: 18,
            layers: &[],
        },
        mat(0, &[layer(1, 2, 0, 15), layer(6, 18, -10, 46)]),
    ];
    let table = match (style, dark) {
        (Style::Standard, true) => &STD_DARK,
        (Style::Standard, false) => &STD_LIGHT,
        (Style::Deeper, true) => &DEEP_DARK,
        (Style::Deeper, false) => &DEEP_LIGHT,
    };
    let i = match level {
        E::Control => 0,
        E::Thumb => 1,
        E::Knob => 2,
        E::Tab => 3,
        E::Primary => 4,
        E::Danger => 5,
        E::Floating => 6,
        E::Pane => 7,
        E::Field => 8,
        E::Deck => 9,
    };
    &table[i]
}

/// Deeper's focused-pane edge: `focus` at these alphas at the top, at 38% of the height, and gone
/// from 70% (concept 4's `.c-diff::before`).
pub const FOCUS_EDGE: [(f32, u8); 3] = [(0.0, 191), (0.38, 46), (0.70, 0)];

// ------------------------------------------------------------------------------------- metrics

/// How tool buttons lay out their icon and caption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLayout {
    /// Icon over caption (Comfortable).
    Stacked,
    /// Icon beside the label (Dense).
    Inline,
}

/// Radius meaning "fully rounded" (half the shorter side).
pub const FULL: u8 = u8::MAX;

/// Geometry and type for one ([`Density`], [`Style`]) pair, read with [`metrics`] and installed
/// into egui by [`apply`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub hero_size: f32,
    pub display_size: f32,
    /// Detail-pane commit subject.
    pub headline_size: f32,
    /// Dialog titles.
    pub title_size: f32,
    /// Card titles.
    pub subtitle_size: f32,
    /// Body, row text, buttons.
    pub body_size: f32,
    pub lead_size: f32,
    /// Metadata, toast body, field labels.
    pub meta_size: f32,
    /// Ref pills, hints.
    pub small_size: f32,
    /// Uppercase section labels.
    pub micro_size: f32,
    /// Diff text.
    pub mono_size: f32,
    pub sha_size: f32,
    /// Keycaps.
    pub chord_size: f32,
    pub icon_xs: f32,
    pub icon_sm: f32,
    pub icon_md: f32,
    pub icon_lg: f32,
    pub icon_tool: f32,
    pub icon_toast: f32,
    pub icon_xl: f32,
    pub icon_hero: f32,
    /// Rows, chips, checkboxes.
    pub radius_sm: u8,
    /// Controls.
    pub radius: u8,
    /// Floating layers and cards.
    pub radius_lg: u8,
    pub pill_radius: u8,
    pub doc_tab_radius: u8,
    pub card_radius: u8,
    /// Deeper's floating panes (0: flush).
    pub pane_radius: u8,
    /// Deeper's gap between panes and around them (0: flush).
    pub pane_gutter: f32,
    pub control_sm: f32,
    pub control: f32,
    pub control_lg: f32,
    pub button_pad_x: f32,
    pub button_pad_x_sm: f32,
    pub button_pad_y: f32,
    pub button_gap: f32,
    pub item_spacing: egui::Vec2,
    pub field_pad_x: f32,
    pub field_label_gap: f32,
    pub tab_strip_height: f32,
    pub doc_tab_height: f32,
    pub search_trigger: egui::Vec2,
    pub action_bar_height: f32,
    pub tool_height: f32,
    pub tool_layout: ToolLayout,
    pub tool_min_width: f32,
    pub tool_pad: f32,
    pub tool_gap: f32,
    pub tool_caption: f32,
    pub toolbar_gap: f32,
    pub tool_divider_height: f32,
    pub tool_divider_margin: f32,
    pub status_bar_height: f32,
    pub pane_header_height: f32,
    pub file_header_height: f32,
    pub sidebar_width: f32,
    pub sidebar_pad: f32,
    pub sidebar_row_height: f32,
    pub section_header_height: f32,
    pub section_gap: f32,
    pub tree_indent: f32,
    pub history_row_height: f32,
    pub table_header_height: f32,
    /// Graph, author, date and SHA columns; the message takes the rest.
    pub history_columns: [f32; 4],
    pub row_height: f32,
    pub detail_info_width: f32,
    pub detail_pad: egui::Vec2,
    pub detail_gap: f32,
    pub menu_item_height: f32,
    pub menu_pad: f32,
    pub palette_row_height: f32,
    pub palette_input_height: f32,
    pub diff_line_height: f32,
    pub hunk_header_height: f32,
    pub request_row_height: f32,
    pub card_pad: f32,
    pub card_gap: f32,
    pub dialog_pad: f32,
    pub dialog_gap: f32,
    pub checkbox: f32,
    pub radio: f32,
    pub switch: egui::Vec2,
    pub switch_knob: f32,
    pub switch_travel: f32,
    pub slider_rail: f32,
    pub slider_thumb: f32,
    pub slider_hit: f32,
    pub progress_height: f32,
    pub spinner: f32,
    pub avatar_row: f32,
    pub avatar_detail: f32,
    pub avatar_lg: f32,
    pub ref_pill_height: f32,
    /// Ref pill padding on the icon side, and on the other side.
    pub ref_pill_pad_icon: f32,
    pub ref_pill_pad: f32,
    pub badge_height: f32,
    pub keycap_height: f32,
    pub keycap_radius: u8,
    pub segment_pad: f32,
    pub segment_track_pad: f32,
    pub underline_tab_height: f32,
    pub tooltip_pad: Margin,
    pub tooltip_size: f32,
    pub toast_pad: Margin,
    pub toast_width: f32,
    pub dialog_width: f32,
    pub dialog_width_form: f32,
    pub dialog_width_wide: f32,
    pub graph_lane: f32,
    pub graph_x0: f32,
    pub graph_node_r: f32,
    pub graph_ring: f32,
    pub graph_edge: f32,
    pub graph_merge_stroke: f32,
    pub graph_head_halo: f32,
    pub graph_head_halo_alpha: u8,
    /// Faint stripes on even history rows.
    pub zebra: bool,
}

const fn margin(top: i8, right: i8, bottom: i8, left: i8) -> Margin {
    Margin {
        left,
        right,
        top,
        bottom,
    }
}

/// Concept 1's metrics.
pub const COMFORTABLE: Metrics = Metrics {
    hero_size: 38.0,
    display_size: 24.0,
    headline_size: 15.5,
    title_size: 15.0,
    subtitle_size: 14.0,
    body_size: 13.0,
    lead_size: 13.0,
    meta_size: 12.5,
    small_size: 11.5,
    micro_size: 10.5,
    mono_size: 12.5,
    sha_size: 12.0,
    chord_size: 11.0,
    icon_xs: 12.0,
    icon_sm: 13.0,
    icon_md: 15.0,
    icon_lg: 16.0,
    icon_tool: 17.0,
    icon_toast: 18.0,
    icon_xl: 24.0,
    icon_hero: 32.0,
    radius_sm: 5,
    radius: 7,
    radius_lg: 12,
    pill_radius: FULL,
    doc_tab_radius: 7,
    card_radius: 12,
    pane_radius: 0,
    pane_gutter: 0.0,
    control_sm: 24.0,
    control: 28.0,
    control_lg: 32.0,
    button_pad_x: 12.0,
    button_pad_x_sm: 10.0,
    button_pad_y: 6.0,
    button_gap: 6.0,
    item_spacing: vec2(8.0, 6.0),
    field_pad_x: 10.0,
    field_label_gap: 6.0,
    tab_strip_height: 42.0,
    doc_tab_height: 30.0,
    search_trigger: vec2(280.0, 30.0),
    action_bar_height: 58.0,
    tool_height: 46.0,
    tool_layout: ToolLayout::Stacked,
    tool_min_width: 54.0,
    tool_pad: 9.0,
    tool_gap: 4.0,
    tool_caption: 12.5,
    toolbar_gap: 4.0,
    tool_divider_height: 30.0,
    tool_divider_margin: 8.0,
    status_bar_height: 28.0,
    pane_header_height: 40.0,
    file_header_height: 36.0,
    sidebar_width: 236.0,
    sidebar_pad: 6.0,
    sidebar_row_height: 26.0,
    section_header_height: 28.0,
    section_gap: 4.0,
    tree_indent: 20.0,
    history_row_height: 28.0,
    table_header_height: 30.0,
    history_columns: [76.0, 162.0, 116.0, 82.0],
    row_height: 30.0,
    detail_info_width: 300.0,
    detail_pad: vec2(14.0, 14.0),
    detail_gap: 10.0,
    menu_item_height: 30.0,
    menu_pad: 4.0,
    palette_row_height: 40.0,
    palette_input_height: 48.0,
    diff_line_height: 20.0,
    hunk_header_height: 24.0,
    request_row_height: 64.0,
    card_pad: 16.0,
    card_gap: 14.0,
    dialog_pad: 18.0,
    dialog_gap: 12.0,
    checkbox: 16.0,
    radio: 16.0,
    switch: vec2(32.0, 18.0),
    switch_knob: 14.0,
    switch_travel: 14.0,
    slider_rail: 4.0,
    slider_thumb: 16.0,
    slider_hit: 18.0,
    progress_height: 6.0,
    spinner: 16.0,
    avatar_row: 18.0,
    avatar_detail: 24.0,
    avatar_lg: 30.0,
    ref_pill_height: 19.0,
    ref_pill_pad_icon: 5.0,
    ref_pill_pad: 7.0,
    badge_height: 17.0,
    keycap_height: 20.0,
    keycap_radius: 5,
    segment_pad: 12.0,
    segment_track_pad: 2.0,
    underline_tab_height: 34.0,
    tooltip_pad: margin(5, 6, 5, 9),
    tooltip_size: 12.0,
    toast_pad: margin(11, 8, 11, 12),
    toast_width: 380.0,
    dialog_width: 420.0,
    dialog_width_form: 520.0,
    dialog_width_wide: 720.0,
    graph_lane: 14.0,
    graph_x0: 18.0,
    graph_node_r: 4.2,
    graph_ring: 2.0,
    graph_edge: 2.0,
    graph_merge_stroke: 2.0,
    graph_head_halo: 5.2,
    graph_head_halo_alpha: 61,
    zebra: false,
};

/// Concept 5's metrics.
pub const DENSE: Metrics = Metrics {
    headline_size: 14.0,
    subtitle_size: 13.5,
    body_size: 12.0,
    lead_size: 12.0,
    meta_size: 12.0,
    small_size: 11.0,
    mono_size: 12.0,
    chord_size: 10.5,
    icon_lg: 15.0,
    icon_tool: 15.0,
    icon_xl: 22.0,
    icon_hero: 28.0,
    radius_sm: 3,
    radius: 5,
    radius_lg: 6,
    pill_radius: 3,
    doc_tab_radius: 5,
    card_radius: 6,
    control_sm: 20.0,
    control: 24.0,
    control_lg: 28.0,
    button_pad_y: 4.0,
    item_spacing: vec2(6.0, 4.0),
    tab_strip_height: 34.0,
    doc_tab_height: 24.0,
    search_trigger: vec2(240.0, 24.0),
    action_bar_height: 36.0,
    tool_height: 26.0,
    tool_layout: ToolLayout::Inline,
    tool_min_width: 0.0,
    tool_pad: 8.0,
    tool_gap: 6.0,
    tool_caption: 12.0,
    toolbar_gap: 2.0,
    tool_divider_height: 18.0,
    tool_divider_margin: 6.0,
    status_bar_height: 24.0,
    pane_header_height: 32.0,
    file_header_height: 30.0,
    sidebar_width: 210.0,
    sidebar_row_height: 22.0,
    section_header_height: 24.0,
    section_gap: 2.0,
    history_row_height: 24.0,
    table_header_height: 26.0,
    history_columns: [66.0, 150.0, 104.0, 74.0],
    row_height: 24.0,
    detail_info_width: 270.0,
    detail_pad: vec2(12.0, 10.0),
    detail_gap: 8.0,
    menu_item_height: 24.0,
    palette_row_height: 32.0,
    palette_input_height: 40.0,
    diff_line_height: 18.0,
    request_row_height: 52.0,
    card_pad: 12.0,
    card_gap: 10.0,
    checkbox: 14.0,
    radio: 14.0,
    switch: vec2(28.0, 16.0),
    switch_knob: 12.0,
    switch_travel: 12.0,
    avatar_row: 16.0,
    ref_pill_height: 17.0,
    ref_pill_pad_icon: 4.0,
    ref_pill_pad: 5.0,
    keycap_height: 18.0,
    keycap_radius: 4,
    segment_pad: 9.0,
    toast_width: 360.0,
    dialog_width_form: 500.0,
    dialog_width_wide: 680.0,
    graph_lane: 12.0,
    graph_x0: 15.0,
    graph_node_r: 3.6,
    graph_edge: 1.8,
    zebra: true,
    ..COMFORTABLE
};

/// Concept 4's geometry over Comfortable.
pub const DEEPER_COMFORTABLE: Metrics = Metrics {
    radius_sm: 6,
    radius: 10,
    radius_lg: 14,
    pill_radius: 6,
    control: 30.0,
    doc_tab_radius: 10,
    card_radius: 14,
    pane_radius: 10,
    pane_gutter: 6.0,
    action_bar_height: 56.0,
    ..COMFORTABLE
};

/// Concept 4's step scaled by concept 5's, over Dense (no concept drew this pair).
pub const DEEPER_DENSE: Metrics = Metrics {
    radius_sm: 4,
    radius: 7,
    radius_lg: 8,
    pill_radius: 4,
    control: 26.0,
    doc_tab_radius: 7,
    card_radius: 8,
    pane_radius: 8,
    pane_gutter: 4.0,
    action_bar_height: 36.0,
    ..DENSE
};

/// The metrics of a (density, style) pair.
pub fn metrics_for(density: Density, style: Style) -> &'static Metrics {
    match (density, style) {
        (Density::Comfortable, Style::Standard) => &COMFORTABLE,
        (Density::Dense, Style::Standard) => &DENSE,
        (Density::Comfortable, Style::Deeper) => &DEEPER_COMFORTABLE,
        (Density::Dense, Style::Deeper) => &DEEPER_DENSE,
    }
}

/// The active metrics.
pub fn metrics() -> &'static Metrics {
    metrics_for(density(), style())
}

impl Metrics {
    /// Every token as (name, value), for the gallery's token sheet.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let f = |v: f32| format!("{v}");
        let r = |v: u8| {
            if v == FULL {
                "full".to_owned()
            } else {
                v.to_string()
            }
        };
        let v2 = |v: egui::Vec2| format!("{} x {}", v.x, v.y);
        let m = |v: Margin| format!("{} {} {} {}", v.top, v.right, v.bottom, v.left);
        vec![
            (
                "hero / display",
                format!("{} / {}", self.hero_size, self.display_size),
            ),
            (
                "headline / title / subtitle",
                format!(
                    "{} / {} / {}",
                    self.headline_size, self.title_size, self.subtitle_size
                ),
            ),
            (
                "body / lead / meta",
                format!(
                    "{} / {} / {}",
                    self.body_size, self.lead_size, self.meta_size
                ),
            ),
            (
                "small / micro",
                format!("{} / {}", self.small_size, self.micro_size),
            ),
            (
                "mono / sha / chord",
                format!(
                    "{} / {} / {}",
                    self.mono_size, self.sha_size, self.chord_size
                ),
            ),
            (
                "icon xs / sm / md / lg",
                format!(
                    "{} / {} / {} / {}",
                    self.icon_xs, self.icon_sm, self.icon_md, self.icon_lg
                ),
            ),
            (
                "icon tool / toast / xl / hero",
                format!(
                    "{} / {} / {} / {}",
                    self.icon_tool, self.icon_toast, self.icon_xl, self.icon_hero
                ),
            ),
            (
                "radius sm / md / lg / pill",
                format!(
                    "{} / {} / {} / {}",
                    r(self.radius_sm),
                    r(self.radius),
                    r(self.radius_lg),
                    r(self.pill_radius)
                ),
            ),
            (
                "doc tab / card / pane radius",
                format!(
                    "{} / {} / {}",
                    r(self.doc_tab_radius),
                    r(self.card_radius),
                    r(self.pane_radius)
                ),
            ),
            ("pane gutter", f(self.pane_gutter)),
            (
                "control sm / md / lg",
                format!(
                    "{} / {} / {}",
                    self.control_sm, self.control, self.control_lg
                ),
            ),
            (
                "button pad x / x sm / y / gap",
                format!(
                    "{} / {} / {} / {}",
                    self.button_pad_x, self.button_pad_x_sm, self.button_pad_y, self.button_gap
                ),
            ),
            ("item spacing", v2(self.item_spacing)),
            (
                "field pad x / label gap",
                format!("{} / {}", self.field_pad_x, self.field_label_gap),
            ),
            (
                "tab strip / doc tab",
                format!("{} / {}", self.tab_strip_height, self.doc_tab_height),
            ),
            ("search trigger", v2(self.search_trigger)),
            (
                "action bar / tool",
                format!(
                    "{} / {} ({:?})",
                    self.action_bar_height, self.tool_height, self.tool_layout
                ),
            ),
            (
                "tool min w / pad / gap / caption",
                format!(
                    "{} / {} / {} / {}",
                    self.tool_min_width, self.tool_pad, self.tool_gap, self.tool_caption
                ),
            ),
            (
                "toolbar gap / divider h / margin",
                format!(
                    "{} / {} / {}",
                    self.toolbar_gap, self.tool_divider_height, self.tool_divider_margin
                ),
            ),
            ("status bar", f(self.status_bar_height)),
            (
                "pane header / file header",
                format!("{} / {}", self.pane_header_height, self.file_header_height),
            ),
            (
                "sidebar width / pad",
                format!("{} / {}", self.sidebar_width, self.sidebar_pad),
            ),
            (
                "sidebar row / section / gap",
                format!(
                    "{} / {} / {}",
                    self.sidebar_row_height, self.section_header_height, self.section_gap
                ),
            ),
            ("tree indent", f(self.tree_indent)),
            (
                "history row / table header",
                format!("{} / {}", self.history_row_height, self.table_header_height),
            ),
            ("history columns", format!("{:?}", self.history_columns)),
            ("row", f(self.row_height)),
            (
                "detail info / pad / gap",
                format!(
                    "{} / {} / {}",
                    self.detail_info_width,
                    v2(self.detail_pad),
                    self.detail_gap
                ),
            ),
            (
                "menu item / pad",
                format!("{} / {}", self.menu_item_height, self.menu_pad),
            ),
            (
                "palette row / input",
                format!(
                    "{} / {}",
                    self.palette_row_height, self.palette_input_height
                ),
            ),
            (
                "diff line / hunk header",
                format!("{} / {}", self.diff_line_height, self.hunk_header_height),
            ),
            ("request row", f(self.request_row_height)),
            (
                "card pad / gap",
                format!("{} / {}", self.card_pad, self.card_gap),
            ),
            (
                "dialog pad / gap",
                format!("{} / {}", self.dialog_pad, self.dialog_gap),
            ),
            (
                "checkbox / radio",
                format!("{} / {}", self.checkbox, self.radio),
            ),
            (
                "switch / knob / travel",
                format!(
                    "{} / {} / {}",
                    v2(self.switch),
                    self.switch_knob,
                    self.switch_travel
                ),
            ),
            (
                "slider rail / thumb / hit",
                format!(
                    "{} / {} / {}",
                    self.slider_rail, self.slider_thumb, self.slider_hit
                ),
            ),
            (
                "progress / spinner",
                format!("{} / {}", self.progress_height, self.spinner),
            ),
            (
                "avatar row / detail / lg",
                format!(
                    "{} / {} / {}",
                    self.avatar_row, self.avatar_detail, self.avatar_lg
                ),
            ),
            (
                "ref pill h / pad icon / pad",
                format!(
                    "{} / {} / {}",
                    self.ref_pill_height, self.ref_pill_pad_icon, self.ref_pill_pad
                ),
            ),
            (
                "badge / keycap / keycap radius",
                format!(
                    "{} / {} / {}",
                    self.badge_height,
                    self.keycap_height,
                    r(self.keycap_radius)
                ),
            ),
            (
                "segment pad / track pad",
                format!("{} / {}", self.segment_pad, self.segment_track_pad),
            ),
            ("underline tab", f(self.underline_tab_height)),
            (
                "tooltip pad / size",
                format!("{} / {}", m(self.tooltip_pad), self.tooltip_size),
            ),
            (
                "toast pad / width",
                format!("{} / {}", m(self.toast_pad), self.toast_width),
            ),
            (
                "dialog widths",
                format!(
                    "{} / {} / {}",
                    self.dialog_width, self.dialog_width_form, self.dialog_width_wide
                ),
            ),
            (
                "graph lane / x0",
                format!("{} / {}", self.graph_lane, self.graph_x0),
            ),
            (
                "graph node / ring / edge / merge",
                format!(
                    "{} / {} / {} / {}",
                    self.graph_node_r, self.graph_ring, self.graph_edge, self.graph_merge_stroke
                ),
            ),
            (
                "graph head halo",
                format!(
                    "+{} at {}",
                    self.graph_head_halo, self.graph_head_halo_alpha
                ),
            ),
            ("zebra", self.zebra.to_string()),
        ]
    }
}

// ------------------------------------------------------------------------------------------ type

/// Family of the semibold UI face.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Family of the medium UI face.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

/// Corner radii kept for views written before the component library (Comfortable Standard).
pub const RADIUS_SM: u8 = COMFORTABLE.radius_sm;
pub const RADIUS: u8 = COMFORTABLE.radius;
pub const RADIUS_LG: u8 = COMFORTABLE.radius_lg;

// ------------------------------------------------------------------------------- egui mapping

/// The largest `Floating` layer of `style` as one egui shadow (egui's own windows and popups).
fn floating_shadow(p: &Palette, style: Style) -> Shadow {
    let m = p.material(style, Elevation::Floating);
    m.layers
        .iter()
        .max_by_key(|l| l.blur)
        .map_or(Shadow::NONE, |l| Shadow {
            offset: [0, l.dy],
            blur: l.blur,
            spread: 0,
            color: Palette::tint(p.shadow_color(l.tint), l.alpha),
        })
}

/// egui's visuals for `p` in `style` with metrics `m`, so any stock widget left over (and egui's
/// own submenus, tooltips, scrollbars and resize handles) matches the kit.
fn visuals(p: &Palette, style: Style, m: &Metrics) -> Visuals {
    let s = p.surfaces(style);
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.dark_mode = p.dark;
    v.text_options.color_transfer_function = if p.dark {
        egui::epaint::FontColorTransferFunction::TwoCoverageMinusCoverageSq
    } else {
        egui::epaint::FontColorTransferFunction::Off
    };
    v.panel_fill = s.panel;
    v.window_fill = s.overlay;
    v.extreme_bg_color = s.field;
    v.text_edit_bg_color = Some(s.field);
    v.faint_bg_color = over(p.hover(), s.pane);
    v.code_bg_color = s.field;
    v.hyperlink_color = p.accent2;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_stroke = Stroke::new(1.0, p.floating_ring(s));
    v.window_corner_radius = CornerRadius::same(m.radius_lg);
    v.menu_corner_radius = CornerRadius::same(m.radius_lg);
    v.window_shadow = floating_shadow(p, style);
    v.popup_shadow = floating_shadow(p, style);
    // Selected list rows and egui's selected values and text share one fill.
    v.selection.bg_fill = p.selected();
    // Also egui's keyboard focus ring: fully opaque so focus is always visible.
    v.selection.stroke = Stroke::new(2.0, p.focus);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.text_cursor.blink = true;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v.button_frame = true;
    v.interact_cursor = None;
    v.image_loading_spinners = false;
    v.disabled_alpha = DISABLED_ALPHA;

    let w = &mut v.widgets;
    let r = CornerRadius::same(m.radius);
    w.noninteractive.bg_fill = s.panel;
    w.noninteractive.weak_bg_fill = s.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, s.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = s.raised.bottom;
    w.inactive.weak_bg_fill = s.raised.bottom;
    w.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;
    w.hovered.bg_fill = s.raised_hover.bottom;
    w.hovered.weak_bg_fill = s.raised_hover.bottom;
    w.hovered.bg_stroke = Stroke::new(1.0, p.control_border);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = s.raised_pressed.bottom;
    w.active.weak_bg_fill = s.raised_pressed.bottom;
    w.active.bg_stroke = Stroke::new(1.0, p.focus);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    v
}

/// Floating scroll bars (section 4.21): thin until hovered, no gutter, no built-in fade.
fn floating_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating: true,
        floating_width: 4.0,
        bar_width: 8.0,
        floating_allocated_width: 0.0,
        handle_min_length: SCROLL_HANDLE_MIN,
        bar_inner_margin: 2.0,
        bar_outer_margin: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.45,
        interact_handle_opacity: 0.75,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.4,
        foreground_color: true,
        fade: egui::style::ScrollFadeStyle {
            strength: 0.0,
            ..Default::default()
        },
        ..egui::style::ScrollStyle::floating()
    }
}

/// The shortest a scrollbar's handle gets, so a long document's handle is still easy to see and
/// to grab (egui's default is 12).
const SCROLL_HANDLE_MIN: f32 = 32.0;

/// Solid scrollbars use foreground ink so they remain visible on raised light surfaces (code and
/// diff panes, for position awareness).
pub fn solid_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        foreground_color: true,
        bar_width: SM,
        handle_min_length: SCROLL_HANDLE_MIN,
        ..egui::style::ScrollStyle::solid()
    }
}

fn look_set(id: ThemeId, style: Style, density: Density, glass: bool) {
    look(|l| {
        l.theme.store(id.index(), Ordering::Relaxed);
        l.style
            .store(u8::from(style == Style::Deeper), Ordering::Relaxed);
        l.density
            .store(u8::from(density == Density::Dense), Ordering::Relaxed);
        l.glass.store(glass, Ordering::Relaxed);
    });
}

/// Runs `f` with `style` and `density` active, then restores the installed ones: the gallery's
/// side-by-side strips paint both styles in one frame. egui's own visuals stay as installed.
pub fn with_look<R>(style: Style, density: Density, f: impl FnOnce() -> R) -> R {
    let (id, before) = (active(), appearance());
    look_set(id, style, density, before.glass);
    let r = f();
    look_set(id, before.style, before.density, before.glass);
    r
}

/// Makes `id` the active theme with `look`'s style, density, glass and motion, and installs the
/// matching egui style on `ctx`.
pub fn apply(ctx: &egui::Context, id: ThemeId, look: &Appearance) {
    look_set(id, look.style, look.density, look.glass);
    super::motion::set_reduced(look.reduce_motion);
    let p = id.palette();
    let m = metrics_for(look.density, look.style);
    let theme = if p.dark { Theme::Dark } else { Theme::Light };
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals(p, look.style, m);
    style.text_styles = [
        (TextStyle::Heading, FontId::new(m.title_size, semibold())),
        (
            TextStyle::Body,
            FontId::new(m.body_size, FontFamily::Proportional),
        ),
        (TextStyle::Button, FontId::new(m.body_size, medium())),
        (
            TextStyle::Small,
            FontId::new(m.small_size, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(m.mono_size, FontFamily::Monospace),
        ),
    ]
    .into();
    let sp = &mut style.spacing;
    sp.item_spacing = m.item_spacing;
    sp.button_padding = vec2(m.button_pad_x, m.button_pad_y);
    sp.interact_size.y = m.control;
    sp.window_margin = Margin::same(m.dialog_pad as i8);
    sp.menu_margin = Margin::same(m.menu_pad as i8);
    sp.menu_spacing = 1.0;
    sp.indent = m.tree_indent;
    sp.icon_width = m.checkbox;
    sp.icon_width_inner = m.checkbox / 2.0;
    sp.icon_spacing = 9.0;
    sp.tooltip_width = 260.0;
    sp.menu_width = 360.0;
    sp.scroll = floating_scroll();
    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = super::motion::TOOLTIP_DELAY;
    style.interaction.tooltip_grace_time = super::motion::TOOLTIP_SKIP;
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

    /// Collects failed pairings so one run lists all of them.
    struct Gate {
        failures: Vec<String>,
        name: &'static str,
        style: Style,
    }

    impl Gate {
        fn new(p: &Palette, style: Style) -> Self {
            Self {
                failures: Vec::new(),
                name: p.name,
                style,
            }
        }

        fn need(&mut self, what: &str, fg: Color32, on: &str, bg: Color32, min: f64) {
            let r = contrast(fg, bg);
            if r < min - 1e-9 {
                self.failures.push(format!(
                    "{}/{:?}: {what} on {on} is {r:.2}:1, needs {min}:1",
                    self.name, self.style
                ));
            }
        }
    }

    /// (text, text_dim, text_faint, outline and focus) minimums: Contrast is stricter.
    fn targets(p: &Palette) -> (f64, f64, f64, f64) {
        if p.strict {
            (10.0, 7.0, 5.5, 4.5)
        } else {
            (7.0, 4.5, 4.5, 3.0)
        }
    }

    /// The surfaces text sits on in `style`: the four grounds (and Deeper's window ground) plus
    /// field, track, a Dense zebra row and every hover state over each ground.
    fn grounds(s: &Surfaces, style: Style) -> Vec<(String, Color32)> {
        let mut g = vec![
            ("pane".to_owned(), s.pane),
            ("panel".to_owned(), s.panel),
            ("elevated".to_owned(), s.elevated),
            ("overlay".to_owned(), s.overlay),
        ];
        if style == Style::Deeper {
            g.push(("deeper ground".to_owned(), s.bg));
        }
        g
    }

    fn with_hovers(p: &Palette, base: &[(String, Color32)]) -> Vec<(String, Color32)> {
        let mut out = base.to_vec();
        for (on, s) in base {
            for (k, ink) in [
                ("hover", p.hover()),
                ("hover_strong", p.hover_strong()),
                ("pressed", p.pressed()),
            ] {
                out.push((format!("{k} over {on}"), over(ink, *s)));
            }
        }
        out
    }

    /// WCAG 2.2 for every pairing the views and the kit draw, in every theme and both styles
    /// (design spec 3.3 items 1, 3 and 5; `spec/themes.py` runs the same checks).
    #[test]
    fn every_theme_meets_contrast_targets() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, rim_min) = targets(p);
                let sel = p.selected();
                let surfaces = grounds(s, style);
                let mut plain = surfaces.clone();
                plain.extend([
                    ("field".to_owned(), s.field),
                    ("track".to_owned(), s.track),
                    ("zebra row".to_owned(), over(p.zebra(), s.pane)),
                ]);
                for (on, bg) in &surfaces {
                    for (k, ink) in [
                        ("hover", p.hover()),
                        ("hover_strong", p.hover_strong()),
                        ("pressed", p.pressed()),
                    ] {
                        plain.push((format!("{k} over {on}"), over(ink, *bg)));
                    }
                }
                let mut selrows: Vec<(String, Color32)> = surfaces
                    .iter()
                    .map(|(on, bg)| (format!("selected row on {on}"), over(sel, *bg)))
                    .collect();
                selrows.extend(surfaces[..2].iter().map(|(on, bg)| {
                    (
                        format!("hovered selected row on {on}"),
                        over(p.hover(), over(sel, *bg)),
                    )
                }));
                let warning_tint = over(p.warning_tint(), s.pane);
                let diff = [
                    ("added_bg".to_owned(), s.added_bg),
                    ("removed_bg".to_owned(), s.removed_bg),
                    ("added_gutter".to_owned(), s.added_gutter),
                    ("removed_gutter".to_owned(), s.removed_gutter),
                    ("selected added line".to_owned(), over(sel, s.added_bg)),
                    ("selected removed line".to_owned(), over(sel, s.removed_bg)),
                    ("hovered added line".to_owned(), over(p.hover(), s.added_bg)),
                    (
                        "hovered removed line".to_owned(),
                        over(p.hover(), s.removed_bg),
                    ),
                    ("warning tint".to_owned(), warning_tint),
                ];
                for (on, bg) in plain.iter().chain(&selrows).chain(&diff) {
                    gate.need("text", p.text, on, *bg, text_min);
                    gate.need("text_dim", p.text_dim, on, *bg, dim_min);
                    gate.need("text_faint", p.text_faint, on, *bg, faint_min);
                }
                for (on, bg) in &plain {
                    gate.need("accent", p.accent, on, *bg, 4.5);
                    gate.need("control_border", p.control_border, on, *bg, rim_min);
                }
                for (on, bg) in plain.iter().chain(&selrows) {
                    gate.need("accent2", p.accent2, on, *bg, 4.5);
                    gate.need("success", p.success, on, *bg, 4.5);
                    gate.need("warning", p.warning, on, *bg, 4.5);
                    gate.need("danger", p.danger, on, *bg, 4.5);
                }
                gate.need("success", p.success, "added_bg", s.added_bg, 4.5);
                gate.need("danger", p.danger, "removed_bg", s.removed_bg, 4.5);
                gate.need("success", p.success, "added_gutter", s.added_gutter, 4.5);
                gate.need("danger", p.danger, "removed_gutter", s.removed_gutter, 4.5);
                gate.need("warning", p.warning, "warning tint", warning_tint, 4.5);
                let lane_bgs = [
                    ("pane", s.pane),
                    ("panel", s.panel),
                    ("hover over pane", over(p.hover(), s.pane)),
                    ("selected row on pane", over(sel, s.pane)),
                ];
                for (i, lane) in p.lanes.iter().enumerate() {
                    for (on, bg) in lane_bgs {
                        gate.need(&format!("lane {i}"), *lane, on, bg, 3.0);
                    }
                }
                let sx = p.syntax;
                for (what, fg) in [
                    ("keyword", sx.keyword),
                    ("string", sx.string),
                    ("comment", sx.comment),
                    ("number", sx.number),
                    ("function", sx.function),
                    ("type", sx.type_),
                    ("constant", sx.constant),
                ] {
                    for (on, bg) in [
                        ("pane", s.pane),
                        ("added_bg", s.added_bg),
                        ("removed_bg", s.removed_bg),
                    ] {
                        gate.need(&format!("syntax {what}"), fg, on, bg, 4.5);
                    }
                }

                // Kit pairings.
                for (name, fill) in [
                    ("primary", s.primary),
                    ("primary_hover", s.primary_hover),
                    ("primary_pressed", s.primary_pressed),
                ] {
                    for c in fill.ends() {
                        gate.need("on_accent", p.on_accent, name, c, 4.5);
                    }
                }
                for (name, fill) in [
                    ("danger_solid", s.danger_solid),
                    ("danger_solid_hover", s.danger_solid_hover),
                    ("danger_solid_pressed", s.danger_solid_pressed),
                ] {
                    for c in fill.ends() {
                        gate.need("on_danger", p.on_danger, name, c, 4.5);
                    }
                }
                for (name, fill) in [
                    ("raised", s.raised),
                    ("raised_hover", s.raised_hover),
                    ("raised_pressed", s.raised_pressed),
                    ("thumb", s.thumb),
                    ("tab", s.tab),
                ] {
                    for c in fill.ends() {
                        gate.need("text", p.text, name, c, text_min);
                        gate.need("text_dim", p.text_dim, name, c, dim_min);
                    }
                }
                for c in s.keycap.ends() {
                    gate.need("text_dim", p.text_dim, "keycap", c, dim_min);
                }
                gate.need("text", p.text, "badge", s.badge, text_min);
                for c in s.tooltip.ends() {
                    gate.need("on_tooltip", p.on_tooltip, "tooltip", c, text_min);
                    gate.need("on_tooltip_dim", p.on_tooltip_dim, "tooltip", c, dim_min);
                }
                let mut focus_bgs: Vec<(String, Color32)> =
                    plain.iter().chain(&selrows).cloned().collect();
                focus_bgs.extend([
                    ("added_bg".to_owned(), s.added_bg),
                    ("removed_bg".to_owned(), s.removed_bg),
                    ("raised.top".to_owned(), s.raised.top),
                    ("raised.bottom".to_owned(), s.raised.bottom),
                ]);
                for (on, bg) in &focus_bgs {
                    gate.need("focus", p.focus, on, *bg, rim_min);
                }
                if p.dark {
                    for c in s.knob.ends() {
                        gate.need("switch knob", c, "switch_off", s.switch_off, 3.0);
                    }
                }
                for (name, c) in [
                    ("accent", p.accent),
                    ("success", p.success),
                    ("warning", p.warning),
                    ("danger", p.danger),
                ] {
                    let tint = Palette::tint(c, 41);
                    for (on, bg) in &surfaces[..4] {
                        let b = over(tint, *bg);
                        gate.need(
                            "text",
                            p.text,
                            &format!("{name} tint over {on}"),
                            b,
                            text_min,
                        );
                        gate.need(name, c, &format!("its tint over {on}"), b, 4.5);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The tokens added after the owner's preview review: concept 1's danger red, the Deeper top
    /// deck, the table header strip and its seam, and the light switch knob's ring.
    #[test]
    fn amendment_tokens_meet_their_targets() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, _, _, rim_min) = targets(p);
                // `danger_base` draws error rings and the danger button's focus ring.
                let mut rims = vec![
                    ("pane", s.pane),
                    ("panel", s.panel),
                    ("elevated", s.elevated),
                    ("overlay", s.overlay),
                    ("field", s.field),
                ];
                rims.extend([
                    ("raised.top", s.raised.top),
                    ("raised.bottom", s.raised.bottom),
                ]);
                for (on, bg) in rims {
                    gate.need("danger_base", p.danger_base, on, bg, rim_min);
                }
                for (on, bg) in [
                    ("overlay", s.overlay),
                    ("elevated", s.elevated),
                    ("pane", s.pane),
                    ("panel", s.panel),
                ] {
                    for (what, fill) in [
                        ("danger highlight", p.danger_highlight()),
                        ("danger tint", p.danger_tint()),
                    ] {
                        let b = over(fill, bg);
                        gate.need("danger", p.danger, &format!("{what} over {on}"), b, 4.5);
                        gate.need("text", p.text, &format!("{what} over {on}"), b, text_min);
                    }
                }
                // Text and controls sit on the deck and the header strip as on any chrome.
                let deck = [
                    ("deck.top".to_owned(), s.deck.top),
                    ("deck.bottom".to_owned(), s.deck.bottom),
                    ("pane_header".to_owned(), s.pane_header),
                ];
                let (_, dim_min, faint_min, _) = targets(p);
                for (on, bg) in with_hovers(p, &deck) {
                    gate.need("text", p.text, &on, bg, text_min);
                    gate.need("text_dim", p.text_dim, &on, bg, dim_min);
                    gate.need("text_faint", p.text_faint, &on, bg, faint_min);
                    gate.need("accent", p.accent, &on, bg, 4.5);
                    gate.need("focus", p.focus, &on, bg, rim_min);
                    gate.need("control_border", p.control_border, &on, bg, rim_min);
                }
                match s.knob_edge {
                    Some(ring) => {
                        for c in s.knob.ends() {
                            gate.need("knob_edge", ring, "knob", c, 3.0);
                        }
                    }
                    None => assert!(p.dark, "{}: light themes ring the switch knob", p.name),
                }
                failures.extend(gate.failures);
            }
            // Owner: Deeper's table header seam is not one more neutral pane edge, and Standard
            // keeps concept 1's flat two-tone chrome.
            let d = &p.deeper;
            if !p.strict {
                assert_ne!(Some(d.pane_header_seam), d.pane_edge, "{}", p.name);
                assert_ne!(d.pane_header_seam, d.border, "{}", p.name);
                assert_ne!(d.pane_header, d.pane, "{}", p.name);
                assert!(
                    !d.deck.is_flat(),
                    "{}: the Deeper deck is a gradient",
                    p.name
                );
            }
            let st = &p.standard;
            assert_eq!(st.deck, Fill::flat(st.panel), "{}", p.name);
            assert_eq!(st.pane_header_seam, st.border, "{}", p.name);
            assert_eq!(st.pane_header, st.pane, "{}", p.name);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Pills, avatars and badges keep their hue and still read at 4.5:1 wherever they are drawn,
    /// in both styles.
    #[test]
    fn coloured_components_are_readable() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut colours: Vec<Color32> = p.lanes.to_vec();
                colours.extend([
                    p.accent, p.accent2, p.success, p.warning, p.danger, p.text_dim,
                ]);
                for c in colours {
                    let mut fills = vec![
                        Palette::pill_fill(c, false),
                        Palette::pill_fill(c, true),
                        p.avatar_fill(c),
                        Palette::badge_fill(c),
                        Palette::change_fill(c),
                    ];
                    if c == p.danger {
                        fills.extend([Palette::tint(c, 56), p.danger_badge_fill()]);
                    }
                    for fill in fills {
                        let ink = p.ink_on(s, c, fill);
                        for bg in p.component_surfaces(s) {
                            let r = contrast(ink, over(fill, bg));
                            if r < 4.5 {
                                failures.push(format!(
                                    "{}/{style:?}: {c:?} on {fill:?} is {r:.2}:1",
                                    p.name
                                ));
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Translucent surfaces keep the contrast targets over any content: the overlay at its glass
    /// opacity over nine worst-case backdrops after the shader's luminance clamp.
    #[test]
    fn glass_keeps_contrast_over_any_backdrop() {
        let probes = [
            ("white", Color32::WHITE),
            ("black", Color32::BLACK),
            ("red", Color32::from_rgb(255, 0, 0)),
            ("green", Color32::from_rgb(0, 255, 0)),
            ("blue", Color32::from_rgb(0, 0, 255)),
            ("yellow", Color32::from_rgb(255, 255, 0)),
            ("cyan", Color32::from_rgb(0, 255, 255)),
            ("magenta", Color32::from_rgb(255, 0, 255)),
            ("grey", Color32::from_gray(128)),
        ];
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            if p.glass_alpha == 0 {
                assert!(p.strict, "{}: only Contrast never uses glass", p.name);
                continue;
            }
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, _) = targets(p);
                for (name, probe) in probes {
                    let backdrop = clamp_backdrop(probe, p.dark, p.glass_luminance);
                    let glass = over(Palette::tint(s.overlay, p.glass_alpha), backdrop);
                    for (on, bg) in [
                        (format!("glass over {name}"), glass),
                        (
                            format!("hover_strong on glass over {name}"),
                            over(p.hover_strong(), glass),
                        ),
                    ] {
                        gate.need("text", p.text, &on, bg, text_min);
                        gate.need("text_dim", p.text_dim, &on, bg, dim_min);
                        gate.need("text_faint", p.text_faint, &on, bg, faint_min);
                        gate.need("accent", p.accent, &on, bg, 4.5);
                        gate.need("danger", p.danger, &on, bg, 4.5);
                        gate.need("control_border", p.control_border, &on, bg, 3.0);
                        gate.need("focus", p.focus, &on, bg, 3.0);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn backdrop_clamp_caps_dark_and_lifts_light() {
        let lum = |c: Color32| luminance(c);
        let dark = clamp_backdrop(Color32::WHITE, true, 0.03);
        assert!(lum(dark) <= 0.031, "{dark:?}");
        assert_eq!(clamp_backdrop(Color32::BLACK, true, 0.03), Color32::BLACK);
        let light = clamp_backdrop(Color32::BLACK, false, 0.80);
        assert!(lum(light) >= 0.79, "{light:?}");
        assert_eq!(clamp_backdrop(Color32::WHITE, false, 0.80), Color32::WHITE);
    }

    /// OKLCH hue of an opaque colour, in degrees.
    fn oklch_hue(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
        let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
        bb.atan2(a).to_degrees().rem_euclid(360.0)
    }

    /// The accent never reads as a status: at least 40 degrees of OKLCH hue from success and
    /// danger in every theme.
    #[test]
    fn accent_clears_status_hues() {
        let dist = |a: f64, b: f64| {
            let d = (a - b).abs() % 360.0;
            d.min(360.0 - d)
        };
        for id in ThemeId::ALL {
            let p = id.palette();
            let accent = oklch_hue(p.accent);
            for (what, c) in [("success", p.success), ("danger", p.danger)] {
                let d = dist(accent, oklch_hue(c));
                assert!(d >= 40.0, "{}: accent is {d:.0} deg from {what}", p.name);
            }
        }
    }

    /// Standard is concept 1: flat fills, flush panes, the content ground is the window ground.
    #[test]
    fn standard_is_flat_and_flush() {
        for id in ThemeId::ALL {
            let p = id.palette();
            let s = &p.standard;
            for (name, fill) in [
                ("raised", s.raised),
                ("raised_hover", s.raised_hover),
                ("raised_pressed", s.raised_pressed),
                ("thumb", s.thumb),
                ("keycap", s.keycap),
                ("tab", s.tab),
                ("tooltip", s.tooltip),
                ("knob", s.knob),
                ("primary", s.primary),
                ("primary_hover", s.primary_hover),
                ("primary_pressed", s.primary_pressed),
                ("danger_solid", s.danger_solid),
                ("danger_solid_hover", s.danger_solid_hover),
                ("danger_solid_pressed", s.danger_solid_pressed),
                ("deck", s.deck),
            ] {
                assert!(fill.is_flat(), "{}: Standard {name} is flat", p.name);
            }
            assert_eq!(s.pane, s.bg, "{}", p.name);
            assert_eq!(s.pane_edge, None, "{}", p.name);
            assert!(p.deeper.pane_edge.is_some(), "{}", p.name);
            for level in Elevation::ALL {
                assert_eq!(
                    material(Style::Standard, p.dark, level).recess,
                    0,
                    "Standard has no field recess"
                );
            }
        }
    }

    #[test]
    fn contrast_has_no_highlights() {
        let p = ThemeId::Contrast.palette();
        for style in Style::ALL {
            for level in Elevation::ALL {
                assert_eq!(p.material(style, level).highlight, 0);
            }
            assert_eq!(p.floating_ring(p.surfaces(style)), p.control_border);
        }
    }

    #[test]
    fn interact_size_follows_the_metrics() {
        let ctx = egui::Context::default();
        for density in Density::ALL {
            for style in Style::ALL {
                let look = Appearance {
                    style,
                    density,
                    glass: false,
                    reduce_motion: true,
                };
                apply(&ctx, ThemeId::Midnight, &look);
                let m = metrics_for(density, style);
                let s = ctx.style_of(Theme::Dark);
                assert_eq!(
                    s.spacing.interact_size.y, m.control,
                    "{density:?} {style:?}"
                );
                assert_eq!(metrics(), m);
                assert_eq!(appearance(), look);
            }
        }
        apply(&ctx, ThemeId::Midnight, &Appearance::STILL);
        assert_eq!(FIELD_MARGIN.top + FIELD_MARGIN.bottom, 14);
    }

    #[test]
    fn metrics_match_the_concepts() {
        assert_eq!(COMFORTABLE.control, 28.0);
        assert_eq!(COMFORTABLE.history_row_height, 28.0);
        assert_eq!(COMFORTABLE.radius, 7);
        assert_eq!(DENSE.control, 24.0);
        assert_eq!(DENSE.history_row_height, 24.0);
        assert_eq!(DENSE.radius_lg, 6);
        const { assert!(DENSE.zebra && !COMFORTABLE.zebra) };
        assert_eq!(DEEPER_COMFORTABLE.control, 30.0);
        assert_eq!(DEEPER_COMFORTABLE.radius, 10);
        assert_eq!(DEEPER_COMFORTABLE.pane_gutter, 6.0);
        assert_eq!(DEEPER_DENSE.control, 26.0);
        // Style overrides geometry only; type and rows come from the density.
        assert_eq!(DEEPER_DENSE.body_size, DENSE.body_size);
        assert_eq!(DEEPER_DENSE.history_row_height, DENSE.history_row_height);
        assert_eq!(COMFORTABLE.pane_gutter, 0.0);
        assert_eq!(DENSE.pane_radius, 0);
    }

    #[test]
    fn theme_names_accept_display_and_variant_names() {
        assert_eq!(ThemeId::from_name("midnight"), Some(ThemeId::Midnight));
        assert_eq!(ThemeId::from_name("CONTRAST"), Some(ThemeId::Contrast));
        assert_eq!(ThemeId::from_name("nope"), None);
        assert_eq!(Style::from_name("deeper"), Some(Style::Deeper));
        assert_eq!(Density::from_name("Dense"), Some(Density::Dense));
    }

    #[test]
    fn glass_needs_theme_backend_and_system_consent() {
        let on = |theme, available, system| {
            Appearance::resolve(
                theme,
                Style::Standard,
                Density::Comfortable,
                true,
                false,
                available,
                system,
            )
            .glass
        };
        assert!(on(ThemeId::Midnight, true, None));
        assert!(on(ThemeId::Midnight, true, Some(false)));
        assert!(!on(ThemeId::Midnight, true, Some(true)));
        assert!(!on(ThemeId::Midnight, false, None));
        assert!(!on(ThemeId::Contrast, true, None));
        let off = Appearance::resolve(
            ThemeId::Midnight,
            Style::Standard,
            Density::Comfortable,
            false,
            false,
            true,
            None,
        );
        assert!(!off.glass);
        assert_eq!(
            glass_blocked(ThemeId::Contrast, true, None),
            Some(GlassBlocked::Theme)
        );
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
            for style in Style::ALL {
                let v = visuals(
                    id.palette(),
                    style,
                    metrics_for(Density::Comfortable, style),
                );
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
}
