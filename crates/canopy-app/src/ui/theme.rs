//! Design tokens. This is the only place colours, geometry and type sizes are defined; see
//! `docs/design.md`.
//!
//! Canopy ships eight themes. Each is a [`Palette`]: the colours both styles share, plus one
//! [`Surfaces`] set per [`Style`] (Standard, the default, and Deeper). Geometry and type come from
//! [`Metrics`], one table per ([`Density`], [`Style`]) pair. The active theme, style, density and
//! translucency are globals set by [`apply`], so views ask [`palette`], [`surfaces`], [`metrics`]
//! and [`appearance`] without knowing which is active.
#![expect(
    clippy::disallowed_methods,
    reason = "the palette: the one module that makes colours"
)]
#![expect(
    clippy::unreadable_literal,
    reason = "colours are written 0xRRGGBB, as design tools show them"
)]

use std::collections::HashMap;
use std::sync::LazyLock;
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
/// Opacity of the status washes over their [`Tones`] (16%).
const TONE_ALPHA: u8 = 41;

pub const MIN_WINDOW_SIZE: egui::Vec2 = vec2(900.0, 560.0);
/// Shared control geometry, in logical points before interface zoom.
pub const CONTROL_INSET: f32 = 2.0;
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
    /// A keycap inside a tooltip (its shortcut): fill, 1 pt ring, inner bottom line (`None` for
    /// a flat key) and label. Dark themes use their ordinary keycap; light themes, whose tooltip
    /// is dark, a key a step lighter than the tooltip.
    pub tooltip_key: Color32,
    pub tooltip_key_edge: Color32,
    pub tooltip_key_base: Option<Color32>,
    pub on_tooltip_key: Color32,
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
    /// The top bars ([`crate::ui::kit::surface::Deck`]). Standard: the flat `panel` chrome of
    /// the repository tab strip and the action bar (concept 1's two-tone chrome). Deeper: the
    /// action bar's floating card, a lifted gradient with its own highlight and shadow; the tab
    /// strip stays on the window's ground.
    pub deck: Fill,
    /// The deck's bottom edge: Standard's seam under the tab strip and the action bar (none over
    /// the content where the theme parts panes by tone); Deeper's outer line under the action
    /// bar's card.
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
    /// Fill of a disabled primary, danger or secondary button: a flat, neutral, dimmed step (no
    /// gradient, highlight, glow or shadow), labelled in [`Palette::disabled_text`].
    pub disabled_fill: Color32,
    /// 1 pt ring around `disabled_fill`, so a disabled button keeps its shape on any surface.
    pub disabled_edge: Color32,
}

/// The base colours of the status washes ([`Palette::success_tint`] and the rest). Dark themes
/// use their text-grade status colours; light themes a brighter, more chromatic version of the
/// same hue, so a 16% wash over a pale ground reads as a clean colour rather than a muddy grey.
#[derive(Debug, Clone, Copy)]
pub struct Tones {
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
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
    /// Label and icon of a disabled control: 3:1 (4.5:1 in Contrast) on `disabled_fill`, every
    /// `raised*` fill, the grounds and their hover tints, dimmer than `text_faint`. Controls drawn
    /// with it are not also faded by [`DISABLED_ALPHA`].
    pub disabled_text: Color32,
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
    /// Bases of the status washes (banners, callouts, dialog discs, diff lines).
    pub tones: Tones,
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
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

// Each theme is an OKLCH surface ladder in its own neutral hue (Standard: flat chrome one clear
// step off the ground; Deeper: a receding ground under lifted cards), its signature colours, status
// tones and ten lanes at least dE 25 apart, contrast-fitted against the tests at the end of this file.
/// "Midnight" (Observatory: navy-ink night, periwinkle signature, starlight links): Observatory: navy-ink night, a periwinkle signature and starlight links, precise as an instrument.
pub const MIDNIGHT: Palette = Palette {
    name: "Midnight",
    concept: "Observatory: navy-ink night, a periwinkle signature and starlight links, precise as an instrument.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x0B0F19),
        panel: hex(0x111723),
        pane: hex(0x0B0F19),
        elevated: hex(0x161D2B),
        overlay: hex(0x181E2D),
        field: hex(0x0E131E),
        track: hex(0x080D17),
        switch_off: hex(0x2A3346),
        badge: hex(0x232B3E),
        border: hex(0x1B2233),
        edge: hex(0x21293B),
        raised_edge: hex(0x21293B),
        overlay_edge: hex(0x20283B),
        tab_edge: hex(0x20283A),
        track_edge: hex(0x181F2E),
        pane_edge: None,
        keycap_edge: hex(0x050914),
        raised: Fill::flat(hex(0x161D2B)),
        raised_hover: Fill::flat(hex(0x1A2131)),
        raised_pressed: Fill::flat(hex(0x141927)),
        thumb: Fill::flat(hex(0x1D2435)),
        keycap: Fill::flat(hex(0x1A2131)),
        tab: Fill::flat(hex(0x161D2B)),
        tooltip: Fill::flat(hex(0x232B3E)),
        tooltip_key: hex(0x1A2131),
        tooltip_key_edge: hex(0x21293B),
        tooltip_key_base: Some(hex(0x050914)),
        on_tooltip_key: hex(0xB2B9C6),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x646CD6)),
        primary_hover: Fill::flat(hex(0x6067D1)),
        primary_pressed: Fill::flat(hex(0x5D63CD)),
        danger_solid: Fill::flat(hex(0xD73E41)),
        danger_solid_hover: Fill::flat(hex(0xD2383D)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0x112023),
        removed_bg: hex(0x1F141E),
        added_gutter: hex(0x162D2C),
        removed_gutter: hex(0x301822),
        deck: Fill::flat(hex(0x111723)),
        deck_edge: hex(0x1B2233),
        pane_header: hex(0x0B0F19),
        pane_header_seam: hex(0x1B2233),
        knob_edge: None,
        disabled_fill: hex(0x141A28),
        disabled_edge: hex(0x1C2334),
    },
    deeper: Surfaces {
        bg: hex(0x040813),
        panel: hex(0x040813),
        pane: hex(0x111622),
        elevated: hex(0x1A2131),
        overlay: hex(0x1B2233),
        field: hex(0x0D111C),
        track: hex(0x030612),
        switch_off: hex(0x2A3346),
        badge: hex(0x232B3E),
        border: hex(0x1B2233),
        edge: hex(0x232B3E),
        raised_edge: hex(0x252D40),
        overlay_edge: hex(0x232B3E),
        tab_edge: hex(0x222A3D),
        track_edge: hex(0x171D2C),
        pane_edge: Some(hex(0x1E2637)),
        keycap_edge: hex(0x02040E),
        raised: Fill {
            top: hex(0x21293B),
            bottom: hex(0x1A2131),
        },
        raised_hover: Fill {
            top: hex(0x262E41),
            bottom: hex(0x1E2638),
        },
        raised_pressed: Fill {
            top: hex(0x181E2D),
            bottom: hex(0x1A2030),
        },
        thumb: Fill {
            top: hex(0x252D40),
            bottom: hex(0x1E2638),
        },
        keycap: Fill {
            top: hex(0x222A3D),
            bottom: hex(0x1B2233),
        },
        tab: Fill {
            top: hex(0x1E2638),
            bottom: hex(0x1A2030),
        },
        tooltip: Fill {
            top: hex(0x272F42),
            bottom: hex(0x21293C),
        },
        tooltip_key: hex(0x222A3D),
        tooltip_key_edge: hex(0x232B3E),
        tooltip_key_base: Some(hex(0x02040E)),
        on_tooltip_key: hex(0xB2B9C6),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xEDEEF1),
        },
        primary: Fill {
            top: hex(0x626BD7),
            bottom: hex(0x5157C1),
        },
        primary_hover: Fill {
            top: hex(0x5D65D1),
            bottom: hex(0x4C51BB),
        },
        primary_pressed: Fill {
            top: hex(0x5960C8),
            bottom: hex(0x5E65CE),
        },
        danger_solid: Fill {
            top: hex(0xD73E40),
            bottom: hex(0xC2282F),
        },
        danger_solid_hover: Fill {
            top: hex(0xD1373B),
            bottom: hex(0xBC202A),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0x16262B),
        removed_bg: hex(0x241A26),
        added_gutter: hex(0x1B3333),
        removed_gutter: hex(0x351E29),
        deck: Fill {
            top: hex(0x1D2435),
            bottom: hex(0x171D2C),
        },
        deck_edge: hex(0x02040E),
        pane_header: hex(0x1C2234),
        pane_header_seam: hex(0x454C8A),
        knob_edge: None,
        disabled_fill: hex(0x171D2C),
        disabled_edge: hex(0x1E2638),
    },
    text: hex(0xEAEDF2),
    text_dim: hex(0xB2B9C6),
    text_faint: hex(0x9FA6B4),
    disabled_text: hex(0x7E8490),
    accent: hex(0xA3ABFF),
    accent2: hex(0x90D9F2),
    focus: hex(0x7F89FF),
    selection: hex(0x7F89FF),
    selection_alpha: 34,
    on_accent: hex(0xFFFFFF),
    success: hex(0x55D69F),
    warning: hex(0xEDB65C),
    danger: hex(0xFD8284),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0x7F89FF),
        success: hex(0x4CC38A),
        warning: hex(0xD9A54A),
        danger: hex(0xE5484D),
    },
    control_border: hex(0x798090),
    on_tooltip: hex(0xEAEDF2),
    on_tooltip_dim: hex(0xAEB3BC),
    hover_ink: hex(0xFFFFFF),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(2, 2, 3, 158),
    glass_alpha: 153,
    glass_luminance: 0.04,
    lanes: [
        hex(0xA9B4FF),
        hex(0x25CEFA),
        hex(0x55D69F),
        hex(0xF6A6F8),
        hex(0xBFB83E),
        hex(0x12E3E0),
        hex(0xFD8BA1),
        hex(0x97CE6D),
        hex(0xFF9B78),
        hex(0xEBA540),
    ],
    syntax: Syntax {
        keyword: hex(0xAEB6FF),
        string: hex(0x83D9AE),
        comment: hex(0x8D93A8),
        number: hex(0xF2B582),
        function: hex(0x8FD3F7),
        type_: hex(0xCDB8FF),
        constant: hex(0xF5A1C0),
    },
};

/// "Daylight" (Drafting studio: white sheets on slate, cobalt ink, teal pen): Drafting studio: cool white sheets on a slate desk, cobalt ink and a teal second pen.
pub const DAYLIGHT: Palette = Palette {
    name: "Daylight",
    concept: "Drafting studio: cool white sheets on a slate desk, cobalt ink and a teal second pen.",
    dark: false,
    strict: false,
    standard: Surfaces {
        bg: hex(0xF7F9FC),
        panel: hex(0xE4EBF5),
        pane: hex(0xF7F9FC),
        elevated: hex(0xFFFFFF),
        overlay: hex(0xFFFFFF),
        field: hex(0xFFFFFF),
        track: hex(0xE6ECF4),
        switch_off: hex(0xC3CEDD),
        badge: hex(0xDEE5EF),
        border: hex(0xD5DEEA),
        edge: hex(0xC9D4E3),
        raised_edge: hex(0xC9D4E3),
        overlay_edge: hex(0xD1DBE8),
        tab_edge: hex(0xD1DBE8),
        track_edge: hex(0xD8E1EC),
        pane_edge: None,
        keycap_edge: hex(0xB7C2D1),
        raised: Fill::flat(hex(0xFFFFFF)),
        raised_hover: Fill::flat(hex(0xF0F3F9)),
        raised_pressed: Fill::flat(hex(0xE9EEF5)),
        thumb: Fill::flat(hex(0xFFFFFF)),
        keycap: Fill::flat(hex(0xECF1F7)),
        tab: Fill::flat(hex(0xFFFFFF)),
        tooltip: Fill::flat(hex(0x181E29)),
        tooltip_key: hex(0x2C313C),
        tooltip_key_edge: hex(0x3B404A),
        tooltip_key_base: None,
        on_tooltip_key: hex(0xD1D3D9),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x2C5FDD)),
        primary_hover: Fill::flat(hex(0x2658D5)),
        primary_pressed: Fill::flat(hex(0x2354D1)),
        danger_solid: Fill::flat(hex(0xD73E41)),
        danger_solid_hover: Fill::flat(hex(0xD2383D)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0xE7F4F2),
        removed_bg: hex(0xF7ECEF),
        added_gutter: hex(0xD7F0E9),
        removed_gutter: hex(0xF6E1E3),
        deck: Fill::flat(hex(0xE4EBF5)),
        deck_edge: hex(0xD5DEEA),
        pane_header: hex(0xF7F9FC),
        pane_header_seam: hex(0xD5DEEA),
        knob_edge: Some(hex(0x94928B)),
        disabled_fill: hex(0xE2E8F1),
        disabled_edge: hex(0xD3DCE9),
    },
    deeper: Surfaces {
        bg: hex(0xDCE3EE),
        panel: hex(0xDCE3EE),
        pane: hex(0xFFFFFF),
        elevated: hex(0xFFFFFF),
        overlay: hex(0xFFFFFF),
        field: hex(0xFFFFFF),
        track: hex(0xD8E1EC),
        switch_off: hex(0xC3CEDD),
        badge: hex(0xDEE5EF),
        border: hex(0xD8E1EC),
        edge: hex(0xC8D3E2),
        raised_edge: hex(0xC8D3E2),
        overlay_edge: hex(0xD3DCE9),
        tab_edge: hex(0xCFD9E7),
        track_edge: hex(0xCFD9E7),
        pane_edge: Some(hex(0xD3DCE9)),
        keycap_edge: hex(0xB6C1D0),
        raised: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF0F4F9),
        },
        raised_hover: Fill {
            top: hex(0xF7F9FC),
            bottom: hex(0xECF0F7),
        },
        raised_pressed: Fill {
            top: hex(0xE7ECF4),
            bottom: hex(0xEBF0F6),
        },
        thumb: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF3F6FA),
        },
        keycap: Fill {
            top: hex(0xF7F9FC),
            bottom: hex(0xEEF2F8),
        },
        tab: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF2F5FA),
        },
        tooltip: Fill::flat(hex(0x181E29)),
        tooltip_key: hex(0x2C313C),
        tooltip_key_edge: hex(0x3B404A),
        tooltip_key_base: None,
        on_tooltip_key: hex(0xD1D3D9),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF3F5F8),
        },
        primary: Fill {
            top: hex(0x3569E7),
            bottom: hex(0x2354D1),
        },
        primary_hover: Fill {
            top: hex(0x2F62E0),
            bottom: hex(0x1D4DC9),
        },
        primary_pressed: Fill {
            top: hex(0x2050CD),
            bottom: hex(0x2455D2),
        },
        danger_solid: Fill {
            top: hex(0xD73E40),
            bottom: hex(0xC2282F),
        },
        danger_solid_hover: Fill {
            top: hex(0xD1373B),
            bottom: hex(0xBC202A),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0xEEFAF5),
        removed_bg: hex(0xFEF2F2),
        added_gutter: hex(0xDEF5EC),
        removed_gutter: hex(0xFDE6E6),
        deck: Fill {
            top: hex(0xF7F9FC),
            bottom: hex(0xEFF3F8),
        },
        deck_edge: hex(0xC4CFDE),
        pane_header: hex(0xF2F4FA),
        pane_header_seam: hex(0xA9BFF2),
        knob_edge: Some(hex(0x878B91)),
        disabled_fill: hex(0xE3E9F2),
        disabled_edge: hex(0xD1DBE8),
    },
    text: hex(0x131720),
    text_dim: hex(0x4E5767),
    text_faint: hex(0x515A67),
    disabled_text: hex(0x626A7A),
    accent: hex(0x1C4BC1),
    accent2: hex(0x006273),
    focus: hex(0x386BE2),
    selection: hex(0x5280E8),
    selection_alpha: 30,
    on_accent: hex(0xFFFFFF),
    success: hex(0x00624B),
    warning: hex(0x744F00),
    danger: hex(0xA70E14),
    danger_base: hex(0xD93D42),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0x628EED),
        success: hex(0x23BA7D),
        warning: hex(0xF8AC3D),
        danger: hex(0xF05656),
    },
    control_border: hex(0x6D7785),
    on_tooltip: hex(0xF1F3F8),
    on_tooltip_dim: hex(0xB0B3BA),
    hover_ink: hex(0x10161F),
    shadow: hex(0x16243C),
    primary_shadow: hex(0x062372),
    danger_shadow: hex(0x781414),
    scrim: Color32::from_rgba_premultiplied(5, 7, 12, 82),
    glass_alpha: 178,
    glass_luminance: 0.38,
    lanes: [
        hex(0x3764CD),
        hex(0x02798B),
        hex(0xB73E00),
        hex(0x077D0A),
        hex(0x913DA1),
        hex(0x7E5C01),
        hex(0x0181C4),
        hex(0xB83241),
        hex(0x007A54),
        hex(0xA02366),
    ],
    syntax: Syntax {
        keyword: hex(0x2D4FCB),
        string: hex(0x13703F),
        comment: hex(0x656C7C),
        number: hex(0xA04800),
        function: hex(0x00658C),
        type_: hex(0x7440BC),
        constant: hex(0xB0295C),
    },
};

/// "Aurora" (Polar night: deep marine sky, ice-teal and aurora violet): Polar night: deep marine sky lit by ice-teal and aurora violet, with green lights in the lanes.
pub const AURORA: Palette = Palette {
    name: "Aurora",
    concept: "Polar night: deep marine sky lit by ice-teal and aurora violet, with green lights in the lanes.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x001B26),
        panel: hex(0x001121),
        pane: hex(0x001B26),
        elevated: hex(0x032A38),
        overlay: hex(0x042B3A),
        field: hex(0x011F2A),
        track: hex(0x001822),
        switch_off: hex(0x164253),
        badge: hex(0x0C3A4B),
        border: hex(0x063040),
        edge: hex(0x093849),
        raised_edge: hex(0x093849),
        overlay_edge: hex(0x093748),
        tab_edge: hex(0x083747),
        track_edge: hex(0x042C3B),
        pane_edge: None,
        keycap_edge: hex(0x00131D),
        raised: Fill::flat(hex(0x032A38)),
        raised_hover: Fill::flat(hex(0x052F3E)),
        raised_pressed: Fill::flat(hex(0x022634)),
        thumb: Fill::flat(hex(0x063242)),
        keycap: Fill::flat(hex(0x052E3E)),
        tab: Fill::flat(hex(0x032A38)),
        tooltip: Fill::flat(hex(0x0C3A4B)),
        tooltip_key: hex(0x052E3E),
        tooltip_key_edge: hex(0x093849),
        tooltip_key_base: Some(hex(0x00131D)),
        on_tooltip_key: hex(0xACC8D0),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x2BD7D6)),
        primary_hover: Fill::flat(hex(0x38DFDD)),
        primary_pressed: Fill::flat(hex(0x08CCCB)),
        danger_solid: Fill::flat(hex(0xD73E41)),
        danger_solid_hover: Fill::flat(hex(0xD2383D)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0x0A2D2E),
        removed_bg: hex(0x151F2A),
        added_gutter: hex(0x133B35),
        removed_gutter: hex(0x27222D),
        deck: Fill::flat(hex(0x001121)),
        deck_edge: hex(0x063040),
        pane_header: hex(0x001B26),
        pane_header_seam: hex(0x063040),
        knob_edge: None,
        disabled_fill: hex(0x032735),
        disabled_edge: hex(0x063141),
    },
    deeper: Surfaces {
        bg: hex(0x001121),
        panel: hex(0x001121),
        pane: hex(0x02232F),
        elevated: hex(0x052E3E),
        overlay: hex(0x063040),
        field: hex(0x001D29),
        track: hex(0x001119),
        switch_off: hex(0x164253),
        badge: hex(0x0D3A4B),
        border: hex(0x063040),
        edge: hex(0x0D3A4B),
        raised_edge: hex(0x103C4E),
        overlay_edge: hex(0x0D3A4B),
        tab_edge: hex(0x0B394A),
        track_edge: hex(0x042A39),
        pane_edge: Some(hex(0x073444)),
        keycap_edge: hex(0x000C13),
        raised: Fill {
            top: hex(0x093849),
            bottom: hex(0x052F3E),
        },
        raised_hover: Fill {
            top: hex(0x103D4E),
            bottom: hex(0x073445),
        },
        raised_pressed: Fill {
            top: hex(0x042B3A),
            bottom: hex(0x052E3D),
        },
        thumb: Fill {
            top: hex(0x0F3C4D),
            bottom: hex(0x073445),
        },
        keycap: Fill {
            top: hex(0x0B394A),
            bottom: hex(0x063040),
        },
        tab: Fill {
            top: hex(0x073445),
            bottom: hex(0x052E3D),
        },
        tooltip: Fill {
            top: hex(0x123E50),
            bottom: hex(0x0A3849),
        },
        tooltip_key: hex(0x0B394A),
        tooltip_key_edge: hex(0x0D3A4B),
        tooltip_key_base: Some(hex(0x000C13)),
        on_tooltip_key: hex(0xACC8D0),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xECEFF0),
        },
        primary: Fill {
            top: hex(0x3CE1E0),
            bottom: hex(0x08CCCB),
        },
        primary_hover: Fill {
            top: hex(0x47E9E7),
            bottom: hex(0x21D3D2),
        },
        primary_pressed: Fill {
            top: hex(0x02C8C7),
            bottom: hex(0x0FCDCC),
        },
        danger_solid: Fill {
            top: hex(0xD73E40),
            bottom: hex(0xC2282F),
        },
        danger_solid_hover: Fill {
            top: hex(0xD1373B),
            bottom: hex(0xBC202A),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0x0C3436),
        removed_bg: hex(0x172632),
        added_gutter: hex(0x15423C),
        removed_gutter: hex(0x292934),
        deck: Fill {
            top: hex(0x063242),
            bottom: hex(0x042A39),
        },
        deck_edge: hex(0x000C14),
        pane_header: hex(0x0B333E),
        pane_header_seam: hex(0x237D81),
        knob_edge: None,
        disabled_fill: hex(0x042A39),
        disabled_edge: hex(0x073445),
    },
    text: hex(0xE6F3F5),
    text_dim: hex(0xACC8D0),
    text_faint: hex(0x97B5BF),
    disabled_text: hex(0x7B959C),
    accent: hex(0x56E3E1),
    accent2: hex(0xCBACFC),
    focus: hex(0x49E1DC),
    selection: hex(0x56D4D5),
    selection_alpha: 30,
    on_accent: hex(0x001925),
    success: hex(0x7EE888),
    warning: hex(0xF9C468),
    danger: hex(0xFE8291),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0x45D6D4),
        success: hex(0x72D97C),
        warning: hex(0xFBA541),
        danger: hex(0xE5484D),
    },
    control_border: hex(0x698F9D),
    on_tooltip: hex(0xE6F3F5),
    on_tooltip_dim: hex(0xA5BCC2),
    hover_ink: hex(0xE6F3F5),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(2, 2, 3, 158),
    glass_alpha: 153,
    glass_luminance: 0.05,
    lanes: [
        hex(0x01DAD9),
        hex(0xCDADFF),
        hex(0x7AD983),
        hex(0xFF9BCB),
        hex(0xD7AB1B),
        hex(0x01CEFE),
        hex(0xFE9C4C),
        hex(0xB2CFFF),
        hex(0xC9DA5D),
        hex(0xFFBA93),
    ],
    syntax: Syntax {
        keyword: hex(0xCBAAFF),
        string: hex(0x82E8B4),
        comment: hex(0x8FA6BA),
        number: hex(0xF6C98C),
        function: hex(0x72DDE8),
        type_: hex(0x9EC6FF),
        constant: hex(0xF6A6CA),
    },
};

/// "Ember" (Forge at dusk: smoked oak, cream type, molten copper): Forge at dusk: smoked oak and cream type around a molten copper glow, with cool steel links.
pub const EMBER: Palette = Palette {
    name: "Ember",
    concept: "Forge at dusk: smoked oak and cream type around a molten copper glow, with cool steel links.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x1A120C),
        panel: hex(0x25190F),
        pane: hex(0x1A120C),
        elevated: hex(0x2B1E16),
        overlay: hex(0x2D2017),
        field: hex(0x1F150E),
        track: hex(0x180F09),
        switch_off: hex(0x453428),
        badge: hex(0x3D2C20),
        border: hex(0x32241A),
        edge: hex(0x3B2A1F),
        raised_edge: hex(0x3B2A1F),
        overlay_edge: hex(0x3A2A1E),
        tab_edge: hex(0x3A291E),
        track_edge: hex(0x2E2117),
        pane_edge: None,
        keycap_edge: hex(0x140B05),
        raised: Fill::flat(hex(0x2B1E16)),
        raised_hover: Fill::flat(hex(0x312319)),
        raised_pressed: Fill::flat(hex(0x271B13)),
        thumb: Fill::flat(hex(0x35261B)),
        keycap: Fill::flat(hex(0x312219)),
        tab: Fill::flat(hex(0x2B1E16)),
        tooltip: Fill::flat(hex(0x3D2C20)),
        tooltip_key: hex(0x312219),
        tooltip_key_edge: hex(0x3B2A1F),
        tooltip_key_base: Some(hex(0x140B05)),
        on_tooltip_key: hex(0xCCBCAB),
        knob: Fill::flat(hex(0xFEF9F1)),
        primary: Fill::flat(hex(0xF69445)),
        primary_hover: Fill::flat(hex(0xFE9B4D)),
        primary_pressed: Fill::flat(hex(0xEA8839)),
        danger_solid: Fill::flat(hex(0xD73E41)),
        danger_solid_hover: Fill::flat(hex(0xD2383D)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0x272213),
        removed_bg: hex(0x2D1713),
        added_gutter: hex(0x31311A),
        removed_gutter: hex(0x3D1C19),
        deck: Fill::flat(hex(0x25190F)),
        deck_edge: hex(0x32241A),
        pane_header: hex(0x1A120C),
        pane_header_seam: hex(0x32241A),
        knob_edge: None,
        disabled_fill: hex(0x281C14),
        disabled_edge: hex(0x34251A),
    },
    deeper: Surfaces {
        bg: hex(0x130904),
        panel: hex(0x130904),
        pane: hex(0x231811),
        elevated: hex(0x312219),
        overlay: hex(0x32241A),
        field: hex(0x1D140E),
        track: hex(0x120803),
        switch_off: hex(0x453428),
        badge: hex(0x3E2D21),
        border: hex(0x32241A),
        edge: hex(0x3E2D21),
        raised_edge: hex(0x402F23),
        overlay_edge: hex(0x3E2D21),
        tab_edge: hex(0x3D2C20),
        track_edge: hex(0x2C1F16),
        pane_edge: Some(hex(0x37271C)),
        keycap_edge: hex(0x0E0501),
        raised: Fill {
            top: hex(0x3B2A1F),
            bottom: hex(0x312319),
        },
        raised_hover: Fill {
            top: hex(0x402F23),
            bottom: hex(0x37271C),
        },
        raised_pressed: Fill {
            top: hex(0x2D2017),
            bottom: hex(0x302218),
        },
        thumb: Fill {
            top: hex(0x3F2E22),
            bottom: hex(0x37271C),
        },
        keycap: Fill {
            top: hex(0x3D2C20),
            bottom: hex(0x32241A),
        },
        tab: Fill {
            top: hex(0x37271C),
            bottom: hex(0x302218),
        },
        tooltip: Fill {
            top: hex(0x423125),
            bottom: hex(0x3B2B1F),
        },
        tooltip_key: hex(0x3D2C20),
        tooltip_key_edge: hex(0x3E2D21),
        tooltip_key_base: Some(hex(0x0E0501)),
        on_tooltip_key: hex(0xCCBCAB),
        knob: Fill {
            top: hex(0xFEF9F1),
            bottom: hex(0xEEE8DE),
        },
        primary: Fill {
            top: hex(0xFF9E54),
            bottom: hex(0xEA8839),
        },
        primary_hover: Fill {
            top: hex(0xFFA96A),
            bottom: hex(0xF28F41),
        },
        primary_pressed: Fill {
            top: hex(0xE68534),
            bottom: hex(0xEC8A3A),
        },
        danger_solid: Fill {
            top: hex(0xD73E40),
            bottom: hex(0xC2282F),
        },
        danger_solid_hover: Fill {
            top: hex(0xD1373B),
            bottom: hex(0xBC202A),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0x2F2817),
        removed_bg: hex(0x351D17),
        added_gutter: hex(0x38361E),
        removed_gutter: hex(0x44211D),
        deck: Fill {
            top: hex(0x35261B),
            bottom: hex(0x2C1F16),
        },
        deck_edge: hex(0x0F0501),
        pane_header: hex(0x352519),
        pane_header_seam: hex(0x8B562E),
        knob_edge: None,
        disabled_fill: hex(0x2C1F16),
        disabled_edge: hex(0x37271C),
    },
    text: hex(0xF4EDE1),
    text_dim: hex(0xCCBCAB),
    text_faint: hex(0xBAA796),
    disabled_text: hex(0x9A8B7A),
    accent: hex(0xFBA156),
    accent2: hex(0x94C3E1),
    focus: hex(0xFF9B4E),
    selection: hex(0xFBC499),
    selection_alpha: 33,
    on_accent: hex(0x22130B),
    success: hex(0xB1D372),
    warning: hex(0xEDD46E),
    danger: hex(0xFC8191),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0xFBA156),
        success: hex(0xA3C75D),
        warning: hex(0xEFCA55),
        danger: hex(0xE94E5A),
    },
    control_border: hex(0x928072),
    on_tooltip: hex(0xF4EDE1),
    on_tooltip_dim: hex(0xBDB3A7),
    hover_ink: hex(0xF4EDE1),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(2, 2, 3, 158),
    glass_alpha: 153,
    glass_luminance: 0.043,
    lanes: [
        hex(0xFDA258),
        hex(0x62C6FF),
        hex(0xA6CA60),
        hex(0xFF98A5),
        hex(0x37E5CA),
        hex(0xE789CC),
        hex(0xDDBF47),
        hex(0x03C4D5),
        hex(0xBAB4FF),
        hex(0x61C780),
    ],
    syntax: Syntax {
        keyword: hex(0xF8AF7C),
        string: hex(0xC3D98F),
        comment: hex(0xA99A8B),
        number: hex(0xECCB72),
        function: hex(0x93C8EB),
        type_: hex(0xF0ABCB),
        constant: hex(0xD9B5F0),
    },
};

/// "Neon" (Night city: black-violet, hot magenta and electric cyan): Night city: black-violet streets under hot magenta and electric cyan signs.
pub const NEON: Palette = Palette {
    name: "Neon",
    concept: "Night city: black-violet streets under hot magenta and electric cyan signs.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x0E091A),
        panel: hex(0x020108),
        pane: hex(0x0E091A),
        elevated: hex(0x1D1330),
        overlay: hex(0x1F1433),
        field: hex(0x120B1F),
        track: hex(0x0C0618),
        switch_off: hex(0x342550),
        badge: hex(0x2D1E47),
        border: hex(0x23173A),
        edge: hex(0x2B1C45),
        raised_edge: hex(0x2B1C45),
        overlay_edge: hex(0x2A1B45),
        tab_edge: hex(0x2A1B44),
        track_edge: hex(0x201434),
        pane_edge: None,
        keycap_edge: hex(0x090316),
        raised: Fill::flat(hex(0x1D1330)),
        raised_hover: Fill::flat(hex(0x221638)),
        raised_pressed: Fill::flat(hex(0x1A102B)),
        thumb: Fill::flat(hex(0x26183D)),
        keycap: Fill::flat(hex(0x221637)),
        tab: Fill::flat(hex(0x1D1330)),
        tooltip: Fill::flat(hex(0x2D1E47)),
        tooltip_key: hex(0x221637),
        tooltip_key_edge: hex(0x2B1C45),
        tooltip_key_base: Some(hex(0x090316)),
        on_tooltip_key: hex(0xC8BCD9),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0xFD53BE)),
        primary_hover: Fill::flat(hex(0xFF63C2)),
        primary_pressed: Fill::flat(hex(0xF046B2)),
        danger_solid: Fill::flat(hex(0xD73E41)),
        danger_solid_hover: Fill::flat(hex(0xD2383D)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0x161E24),
        removed_bg: hex(0x220E1F),
        added_gutter: hex(0x1D312C),
        removed_gutter: hex(0x331323),
        deck: Fill::flat(hex(0x020108)),
        deck_edge: hex(0x23173A),
        pane_header: hex(0x0E091A),
        pane_header_seam: hex(0x23173A),
        knob_edge: None,
        disabled_fill: hex(0x1B112C),
        disabled_edge: hex(0x25173C),
    },
    deeper: Surfaces {
        bg: hex(0x04010D),
        panel: hex(0x04010D),
        pane: hex(0x160E25),
        elevated: hex(0x221637),
        overlay: hex(0x23173A),
        field: hex(0x110A1D),
        track: hex(0x070115),
        switch_off: hex(0x342550),
        badge: hex(0x2D1E48),
        border: hex(0x23173A),
        edge: hex(0x2D1E48),
        raised_edge: hex(0x2F204A),
        overlay_edge: hex(0x2D1E48),
        tab_edge: hex(0x2C1D47),
        track_edge: hex(0x1E1331),
        pane_edge: Some(hex(0x271940)),
        keycap_edge: hex(0x050011),
        raised: Fill {
            top: hex(0x2B1C45),
            bottom: hex(0x221638),
        },
        raised_hover: Fill {
            top: hex(0x30214B),
            bottom: hex(0x281A40),
        },
        raised_pressed: Fill {
            top: hex(0x1F1433),
            bottom: hex(0x211537),
        },
        thumb: Fill {
            top: hex(0x2F204A),
            bottom: hex(0x281A40),
        },
        keycap: Fill {
            top: hex(0x2C1D47),
            bottom: hex(0x23173A),
        },
        tab: Fill {
            top: hex(0x281A40),
            bottom: hex(0x211537),
        },
        tooltip: Fill {
            top: hex(0x31224C),
            bottom: hex(0x2B1C46),
        },
        tooltip_key: hex(0x2C1D47),
        tooltip_key_edge: hex(0x2D1E48),
        tooltip_key_base: Some(hex(0x050011)),
        on_tooltip_key: hex(0xC8BCD9),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xEFEEF1),
        },
        primary: Fill {
            top: hex(0xFF6AC4),
            bottom: hex(0xF046B2),
        },
        primary_hover: Fill {
            top: hex(0xFF7AC8),
            bottom: hex(0xF84EB9),
        },
        primary_pressed: Fill {
            top: hex(0xEC41AF),
            bottom: hex(0xF247B4),
        },
        danger_solid: Fill {
            top: hex(0xD73E40),
            bottom: hex(0xC2282F),
        },
        danger_solid_hover: Fill {
            top: hex(0xD1373B),
            bottom: hex(0xBC202A),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0x1D232E),
        removed_bg: hex(0x291329),
        added_gutter: hex(0x233635),
        removed_gutter: hex(0x39182C),
        deck: Fill {
            top: hex(0x26183D),
            bottom: hex(0x1E1331),
        },
        deck_edge: hex(0x050012),
        pane_header: hex(0x1D1F37),
        pane_header_seam: hex(0x27798B),
        knob_edge: None,
        disabled_fill: hex(0x1E1331),
        disabled_edge: hex(0x281A40),
    },
    text: hex(0xF3F0FB),
    text_dim: hex(0xC8BCD9),
    text_faint: hex(0xB4A7C7),
    disabled_text: hex(0x8D849A),
    accent: hex(0xFF6EC5),
    accent2: hex(0x39F3FC),
    focus: hex(0x39F3FC),
    selection: hex(0xF160B8),
    selection_alpha: 34,
    on_accent: hex(0x10091A),
    success: hex(0x60F882),
    warning: hex(0xF7E343),
    danger: hex(0xFF7455),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0xFB51BC),
        success: hex(0x64F784),
        warning: hex(0xF7E343),
        danger: hex(0xE5484D),
    },
    control_border: hex(0x837599),
    on_tooltip: hex(0xF3F0FB),
    on_tooltip_dim: hex(0xB8B1C5),
    hover_ink: hex(0xF3F0FB),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(2, 2, 3, 158),
    glass_alpha: 153,
    glass_luminance: 0.036,
    lanes: [
        hex(0xFF9CD3),
        hex(0x02DBE4),
        hex(0x57E176),
        hex(0xD7C301),
        hex(0xCF92FF),
        hex(0xB0E649),
        hex(0x9DBAFF),
        hex(0xFA9D01),
        hex(0xFFBEB0),
        hex(0xFF8E61),
    ],
    syntax: Syntax {
        keyword: hex(0xFF7DD8),
        string: hex(0x6BF29A),
        comment: hex(0xA597BE),
        number: hex(0xF5E05D),
        function: hex(0x49EAF5),
        type_: hex(0xC6A4FF),
        constant: hex(0xFFAF78),
    },
};

/// "Forest" (Evergreen at dusk: moss and shade, sunlit gold, lake water): Evergreen at dusk: moss and deep shade under cream type, sunlit gold and lake water.
pub const FOREST: Palette = Palette {
    name: "Forest",
    concept: "Evergreen at dusk: moss and deep shade under cream type, sunlit gold and lake water.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x071B10),
        panel: hex(0x00140A),
        pane: hex(0x071B10),
        elevated: hex(0x0F2B1C),
        overlay: hex(0x102D1D),
        field: hex(0x091F14),
        track: hex(0x04190D),
        switch_off: hex(0x214331),
        badge: hex(0x193B29),
        border: hex(0x133121),
        edge: hex(0x173927),
        raised_edge: hex(0x173927),
        overlay_edge: hex(0x173927),
        tab_edge: hex(0x163826),
        track_edge: hex(0x112E1E),
        pane_edge: None,
        keycap_edge: hex(0x011509),
        raised: Fill::flat(hex(0x0F2B1C)),
        raised_hover: Fill::flat(hex(0x123020)),
        raised_pressed: Fill::flat(hex(0x0D2719)),
        thumb: Fill::flat(hex(0x143423)),
        keycap: Fill::flat(hex(0x123020)),
        tab: Fill::flat(hex(0x0F2B1C)),
        tooltip: Fill::flat(hex(0x193B29)),
        tooltip_key: hex(0x123020),
        tooltip_key_edge: hex(0x173927),
        tooltip_key_base: Some(hex(0x011509)),
        on_tooltip_key: hex(0xC0C2AA),
        knob: Fill::flat(hex(0xFCFAF0)),
        primary: Fill::flat(hex(0xE6B441)),
        primary_hover: Fill::flat(hex(0xEEBB49)),
        primary_pressed: Fill::flat(hex(0xDBA833)),
        danger_solid: Fill::flat(hex(0xD73E41)),
        danger_solid_hover: Fill::flat(hex(0xD2383D)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0x102C18),
        removed_bg: hex(0x1A2115),
        added_gutter: hex(0x19391F),
        removed_gutter: hex(0x2B2419),
        deck: Fill::flat(hex(0x00140A)),
        deck_edge: hex(0x133121),
        pane_header: hex(0x071B10),
        pane_header_seam: hex(0x133121),
        knob_edge: None,
        disabled_fill: hex(0x0E281A),
        disabled_edge: hex(0x133322),
    },
    deeper: Surfaces {
        bg: hex(0x001309),
        panel: hex(0x001309),
        pane: hex(0x0B2317),
        elevated: hex(0x123020),
        overlay: hex(0x133121),
        field: hex(0x081E12),
        track: hex(0x001207),
        switch_off: hex(0x214331),
        badge: hex(0x1A3C2A),
        border: hex(0x133121),
        edge: hex(0x1A3C2A),
        raised_edge: hex(0x1C3E2B),
        overlay_edge: hex(0x1A3C2A),
        tab_edge: hex(0x193B29),
        track_edge: hex(0x102B1D),
        pane_edge: Some(hex(0x153524)),
        keycap_edge: hex(0x000D05),
        raised: Fill {
            top: hex(0x173927),
            bottom: hex(0x123020),
        },
        raised_hover: Fill {
            top: hex(0x1D3F2C),
            bottom: hex(0x153624),
        },
        raised_pressed: Fill {
            top: hex(0x102D1D),
            bottom: hex(0x122F1F),
        },
        thumb: Fill {
            top: hex(0x1C3D2B),
            bottom: hex(0x153624),
        },
        keycap: Fill {
            top: hex(0x193B29),
            bottom: hex(0x133121),
        },
        tab: Fill {
            top: hex(0x153624),
            bottom: hex(0x122F1F),
        },
        tooltip: Fill {
            top: hex(0x1E402D),
            bottom: hex(0x183A28),
        },
        tooltip_key: hex(0x193B29),
        tooltip_key_edge: hex(0x1A3C2A),
        tooltip_key_base: Some(hex(0x000D05)),
        on_tooltip_key: hex(0xC0C2AA),
        knob: Fill {
            top: hex(0xFCFAF0),
            bottom: hex(0xECE9DE),
        },
        primary: Fill {
            top: hex(0xF0BD4C),
            bottom: hex(0xDBA833),
        },
        primary_hover: Fill {
            top: hex(0xF8C454),
            bottom: hex(0xE2AF3C),
        },
        primary_pressed: Fill {
            top: hex(0xD7A52E),
            bottom: hex(0xDCAA35),
        },
        danger_solid: Fill {
            top: hex(0xD73E40),
            bottom: hex(0xC2282F),
        },
        danger_solid_hover: Fill {
            top: hex(0xD1373B),
            bottom: hex(0xBC202A),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0x14331E),
        removed_bg: hex(0x1E281B),
        added_gutter: hex(0x1C4025),
        removed_gutter: hex(0x2E2B1F),
        deck: Fill {
            top: hex(0x143423),
            bottom: hex(0x102B1D),
        },
        deck_edge: hex(0x000E05),
        pane_header: hex(0x1D3220),
        pane_header_seam: hex(0x766E36),
        knob_edge: None,
        disabled_fill: hex(0x102B1D),
        disabled_edge: hex(0x153624),
    },
    text: hex(0xF2EDDB),
    text_dim: hex(0xC0C2AA),
    text_faint: hex(0xA6AF99),
    disabled_text: hex(0x8E917C),
    accent: hex(0xEDC15A),
    accent2: hex(0x83C7E4),
    focus: hex(0xEDC15A),
    selection: hex(0x7AC7E2),
    selection_alpha: 34,
    on_accent: hex(0x1E1908),
    success: hex(0x87E17E),
    warning: hex(0xFFA963),
    danger: hex(0xFA8798),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0xEDC15A),
        success: hex(0x71D268),
        warning: hex(0xFE9D4A),
        danger: hex(0xD95448),
    },
    control_border: hex(0x798E79),
    on_tooltip: hex(0xF2EDDB),
    on_tooltip_dim: hex(0xB1B8A6),
    hover_ink: hex(0xF2EDDB),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(2, 2, 3, 158),
    glass_alpha: 153,
    glass_luminance: 0.05,
    lanes: [
        hex(0xE2B33F),
        hex(0x33CCFF),
        hex(0x84D17C),
        hex(0xFF9A97),
        hex(0x7BAFFF),
        hex(0xA9B945),
        hex(0xC296F5),
        hex(0xF39254),
        hex(0x00C7C2),
        hex(0xFFA4D5),
    ],
    syntax: Syntax {
        keyword: hex(0x92DCA1),
        string: hex(0xD9D682),
        comment: hex(0x93A493),
        number: hex(0xF4C677),
        function: hex(0x87CFF0),
        type_: hex(0xCCB9F1),
        constant: hex(0xF2ACCA),
    },
};

/// "Paper" (Letterpress: cream stock on kraft, Prussian ink, highlighter): Letterpress: cream stock on a kraft desk, Prussian ink, sepia notes and a yellow highlighter.
pub const PAPER: Palette = Palette {
    name: "Paper",
    concept: "Letterpress: cream stock on a kraft desk, Prussian ink, sepia notes and a yellow highlighter.",
    dark: false,
    strict: false,
    standard: Surfaces {
        bg: hex(0xF9F2E4),
        panel: hex(0xEEE0CB),
        pane: hex(0xF9F2E4),
        elevated: hex(0xFFF9EE),
        overlay: hex(0xFFF9EE),
        field: hex(0xFFF9EE),
        track: hex(0xECE4D3),
        switch_off: hex(0xD0C5AF),
        badge: hex(0xE6DDCB),
        border: hex(0xE0D5C1),
        edge: hex(0xD6CBB5),
        raised_edge: hex(0xD6CBB5),
        overlay_edge: hex(0xDDD2BE),
        tab_edge: hex(0xDDD2BE),
        track_edge: hex(0xE2D8C5),
        pane_edge: None,
        keycap_edge: hex(0xC4B9A4),
        raised: Fill::flat(hex(0xFFF9EE)),
        raised_hover: Fill::flat(hex(0xF3ECDC)),
        raised_pressed: Fill::flat(hex(0xEFE6D6)),
        thumb: Fill::flat(hex(0xFFF9EE)),
        keycap: Fill::flat(hex(0xF1E9D9)),
        tab: Fill::flat(hex(0xFFF9EE)),
        tooltip: Fill::flat(hex(0x2C2017)),
        tooltip_key: hex(0x3E332A),
        tooltip_key_edge: hex(0x4C4239),
        tooltip_key_base: None,
        on_tooltip_key: hex(0xD9D4CB),
        knob: Fill::flat(hex(0xFFFEFD)),
        primary: Fill::flat(hex(0x125CA1)),
        primary_hover: Fill::flat(hex(0x07559A)),
        primary_pressed: Fill::flat(hex(0x005295)),
        danger_solid: Fill::flat(hex(0xD53C3F)),
        danger_solid_hover: Fill::flat(hex(0xCF353A)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0xEEEDDA),
        removed_bg: hex(0xF7E7D9),
        added_gutter: hex(0xE5E8D1),
        removed_gutter: hex(0xF6DDD0),
        deck: Fill::flat(hex(0xEEE0CB)),
        deck_edge: hex(0xE0D5C1),
        pane_header: hex(0xF9F2E4),
        pane_header_seam: hex(0xE0D5C1),
        knob_edge: Some(hex(0x96908B)),
        disabled_fill: hex(0xE9E0CF),
        disabled_edge: hex(0xDED4BF),
    },
    deeper: Surfaces {
        bg: hex(0xE5DBC8),
        panel: hex(0xE5DBC8),
        pane: hex(0xFFF9EE),
        elevated: hex(0xFFF9EE),
        overlay: hex(0xFFF9EE),
        field: hex(0xFFF9EE),
        track: hex(0xE2D8C5),
        switch_off: hex(0xD0C5AF),
        badge: hex(0xE6DDCB),
        border: hex(0xE2D8C5),
        edge: hex(0xD6CBB5),
        raised_edge: hex(0xD6CBB5),
        overlay_edge: hex(0xDED4BF),
        tab_edge: hex(0xDBD0BB),
        track_edge: hex(0xDBD0BB),
        pane_edge: Some(hex(0xDED4BF)),
        keycap_edge: hex(0xC4B9A3),
        raised: Fill {
            top: hex(0xFFF9EE),
            bottom: hex(0xF4ECDD),
        },
        raised_hover: Fill {
            top: hex(0xF9F2E4),
            bottom: hex(0xF1E8D8),
        },
        raised_pressed: Fill {
            top: hex(0xEDE4D3),
            bottom: hex(0xF0E8D7),
        },
        thumb: Fill {
            top: hex(0xFFF9EE),
            bottom: hex(0xF6EEE0),
        },
        keycap: Fill {
            top: hex(0xF9F2E4),
            bottom: hex(0xF2EADB),
        },
        tab: Fill {
            top: hex(0xFFF9EE),
            bottom: hex(0xF5EEDF),
        },
        tooltip: Fill::flat(hex(0x2C2017)),
        tooltip_key: hex(0x3E332A),
        tooltip_key_edge: hex(0x4C4239),
        tooltip_key_base: None,
        on_tooltip_key: hex(0xD9D4CB),
        knob: Fill {
            top: hex(0xFFFEFD),
            bottom: hex(0xF6F4F0),
        },
        primary: Fill {
            top: hex(0x1E65AA),
            bottom: hex(0x005295),
        },
        primary_hover: Fill {
            top: hex(0x165EA3),
            bottom: hex(0x004C8B),
        },
        primary_pressed: Fill {
            top: hex(0x004E90),
            bottom: hex(0x025397),
        },
        danger_solid: Fill {
            top: hex(0xD53C3E),
            bottom: hex(0xC0262D),
        },
        danger_solid_hover: Fill {
            top: hex(0xCF3539),
            bottom: hex(0xBA1E28),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0xF4F3E3),
        removed_bg: hex(0xFDEDE2),
        added_gutter: hex(0xEAEEDA),
        removed_gutter: hex(0xFBE3D9),
        deck: Fill {
            top: hex(0xF9F2E4),
            bottom: hex(0xF3EBDC),
        },
        deck_edge: hex(0xD2C7B1),
        pane_header: hex(0xF0EEE8),
        pane_header_seam: hex(0x9EBDD6),
        knob_edge: Some(hex(0x8D8A83)),
        disabled_fill: hex(0xEAE1CF),
        disabled_edge: hex(0xDDD2BD),
    },
    text: hex(0x2A1F17),
    text_dim: hex(0x615246),
    text_faint: hex(0x635547),
    disabled_text: hex(0x736356),
    accent: hex(0x005393),
    accent2: hex(0x764627),
    focus: hex(0x1D6CB8),
    selection: hex(0xF8E048),
    selection_alpha: 100,
    on_accent: hex(0xFFFDFA),
    success: hex(0x235E1E),
    warning: hex(0x6E4C00),
    danger: hex(0x9C1A25),
    danger_base: hex(0xD93D42),
    on_danger: hex(0xFFFDFA),
    tones: Tones {
        accent: hex(0x2A80E2),
        success: hex(0x75B168),
        warning: hex(0xEBAE42),
        danger: hex(0xE3645C),
    },
    control_border: hex(0x7D705E),
    on_tooltip: hex(0xF7F3EB),
    on_tooltip_dim: hex(0xBAB4AB),
    hover_ink: hex(0x1A150B),
    shadow: hex(0x311C09),
    primary_shadow: hex(0x03294F),
    danger_shadow: hex(0x781414),
    scrim: Color32::from_rgba_premultiplied(11, 8, 5, 82),
    glass_alpha: 178,
    glass_luminance: 0.35,
    lanes: [
        hex(0x005DA1),
        hex(0x9E2E21),
        hex(0x426800),
        hex(0x6955B8),
        hex(0x007E45),
        hex(0xAB3A60),
        hex(0x02796F),
        hex(0x822A75),
        hex(0x644A00),
        hex(0x01556E),
    ],
    syntax: Syntax {
        keyword: hex(0x2852A3),
        string: hex(0x466A1E),
        comment: hex(0x766759),
        number: hex(0xA2470F),
        function: hex(0x644792),
        type_: hex(0x02686C),
        constant: hex(0x8F3662),
    },
};

/// "Contrast" (Signal: black, white type, strong outlines and a yellow focus ring): Pure black, white type, strong outlines and a yellow focus ring, for low vision and glare.
pub const CONTRAST: Palette = Palette {
    name: "Contrast",
    concept: "Pure black, white type, strong outlines and a yellow focus ring, for low vision and glare.",
    dark: true,
    strict: true,
    standard: Surfaces {
        bg: hex(0x000000),
        panel: hex(0x0E0E0E),
        pane: hex(0x000000),
        elevated: hex(0x161616),
        overlay: hex(0x161616),
        field: hex(0x0A0A0A),
        track: hex(0x000000),
        switch_off: hex(0x3A3A3A),
        badge: hex(0x2A2A2A),
        border: hex(0x767676),
        edge: hex(0x9A9A9A),
        raised_edge: hex(0x9A9A9A),
        overlay_edge: hex(0xCACACA),
        tab_edge: hex(0x9A9A9A),
        track_edge: hex(0x767676),
        pane_edge: None,
        keycap_edge: hex(0x767676),
        raised: Fill::flat(hex(0x1C1C1C)),
        raised_hover: Fill::flat(hex(0x262626)),
        raised_pressed: Fill::flat(hex(0x121212)),
        thumb: Fill::flat(hex(0x2E2E2E)),
        keycap: Fill::flat(hex(0x1E1E1E)),
        tab: Fill::flat(hex(0x1C1C1C)),
        tooltip: Fill::flat(hex(0x1E1E1E)),
        tooltip_key: hex(0x1E1E1E),
        tooltip_key_edge: hex(0x9A9A9A),
        tooltip_key_base: Some(hex(0x767676)),
        on_tooltip_key: hex(0xE9E9E9),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x71C8FF)),
        primary_hover: Fill::flat(hex(0x91D3FF)),
        primary_pressed: Fill::flat(hex(0x56BEFB)),
        danger_solid: Fill::flat(hex(0xFF8987)),
        danger_solid_hover: Fill::flat(hex(0xFF9C99)),
        danger_solid_pressed: Fill::flat(hex(0xF37A79)),
        added_bg: hex(0x08180E),
        removed_bg: hex(0x190909),
        added_gutter: hex(0x0E2B19),
        removed_gutter: hex(0x2A0F10),
        deck: Fill::flat(hex(0x0E0E0E)),
        deck_edge: hex(0x767676),
        pane_header: hex(0x000000),
        pane_header_seam: hex(0x767676),
        knob_edge: None,
        disabled_fill: hex(0x141414),
        disabled_edge: hex(0x5C5C5C),
    },
    deeper: Surfaces {
        bg: hex(0x000000),
        panel: hex(0x000000),
        pane: hex(0x0A0A0A),
        elevated: hex(0x181818),
        overlay: hex(0x181818),
        field: hex(0x000000),
        track: hex(0x000000),
        switch_off: hex(0x3A3A3A),
        badge: hex(0x2A2A2A),
        border: hex(0x767676),
        edge: hex(0x9A9A9A),
        raised_edge: hex(0x9A9A9A),
        overlay_edge: hex(0xCACACA),
        tab_edge: hex(0x9A9A9A),
        track_edge: hex(0x767676),
        pane_edge: Some(hex(0x8A8A8A)),
        keycap_edge: hex(0x767676),
        raised: Fill {
            top: hex(0x262626),
            bottom: hex(0x1C1C1C),
        },
        raised_hover: Fill {
            top: hex(0x323232),
            bottom: hex(0x262626),
        },
        raised_pressed: Fill {
            top: hex(0x121212),
            bottom: hex(0x181818),
        },
        thumb: Fill {
            top: hex(0x363636),
            bottom: hex(0x2C2C2C),
        },
        keycap: Fill {
            top: hex(0x2C2C2C),
            bottom: hex(0x1E1E1E),
        },
        tab: Fill {
            top: hex(0x242424),
            bottom: hex(0x1C1C1C),
        },
        tooltip: Fill {
            top: hex(0x282828),
            bottom: hex(0x1E1E1E),
        },
        tooltip_key: hex(0x2C2C2C),
        tooltip_key_edge: hex(0x9A9A9A),
        tooltip_key_base: Some(hex(0x767676)),
        on_tooltip_key: hex(0xE9E9E9),
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
        added_bg: hex(0x112117),
        removed_bg: hex(0x221212),
        added_gutter: hex(0x163321),
        removed_gutter: hex(0x321718),
        deck: Fill {
            top: hex(0x1A1A1A),
            bottom: hex(0x101010),
        },
        deck_edge: hex(0x767676),
        pane_header: hex(0x141414),
        pane_header_seam: hex(0xFDEA4C),
        knob_edge: None,
        disabled_fill: hex(0x161616),
        disabled_edge: hex(0x5C5C5C),
    },
    text: hex(0xFFFFFF),
    text_dim: hex(0xE9E9E9),
    text_faint: hex(0xD6D6D6),
    disabled_text: hex(0xA1A1A1),
    accent: hex(0x7ECFFF),
    accent2: hex(0xFDEA4C),
    focus: hex(0xFDEA4C),
    selection: hex(0xFFFFFF),
    selection_alpha: 40,
    on_accent: hex(0x000000),
    success: hex(0x54FC93),
    warning: hex(0xFFB573),
    danger: hex(0xFF8C8A),
    danger_base: hex(0xF3565B),
    on_danger: hex(0x000000),
    tones: Tones {
        accent: hex(0x7ECFFF),
        success: hex(0x54FC93),
        warning: hex(0xFFB573),
        danger: hex(0xF3565B),
    },
    control_border: hex(0xCACACA),
    on_tooltip: hex(0xFFFFFF),
    on_tooltip_dim: hex(0xC4C4C4),
    hover_ink: hex(0xFFFFFF),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(0, 0, 0, 200),
    glass_alpha: 0,
    glass_luminance: 0.03,
    lanes: [
        hex(0x96D7FF),
        hex(0xE4D127),
        hex(0xEF96FF),
        hex(0x57FCB4),
        hex(0xFFA83D),
        hex(0x01F1F1),
        hex(0xFFA09A),
        hex(0x9EE064),
        hex(0xFFCBF8),
        hex(0xFFD799),
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
    pub const ALL: [Self; 8] = [
        Self::Midnight,
        Self::Daylight,
        Self::Aurora,
        Self::Ember,
        Self::Neon,
        Self::Forest,
        Self::Paper,
        Self::Contrast,
    ];

    /// The theme's colours.
    pub fn palette(self) -> &'static Palette {
        match self {
            Self::Midnight => &MIDNIGHT,
            Self::Daylight => &DAYLIGHT,
            Self::Aurora => &AURORA,
            Self::Ember => &EMBER,
            Self::Neon => &NEON,
            Self::Forest => &FOREST,
            Self::Paper => &PAPER,
            Self::Contrast => &CONTRAST,
        }
    }

    /// The theme's structure.
    pub fn design(self) -> &'static Design {
        match self {
            Self::Midnight => &MIDNIGHT_DESIGN,
            Self::Daylight => &DAYLIGHT_DESIGN,
            Self::Aurora => &AURORA_DESIGN,
            Self::Ember => &EMBER_DESIGN,
            Self::Neon => &NEON_DESIGN,
            Self::Forest => &FOREST_DESIGN,
            Self::Paper => &PAPER_DESIGN,
            Self::Contrast => &CONTRAST_DESIGN,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        self.palette().name
    }

    /// The theme for a `--theme` argument: the display name or the variant name, any case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| {
            t.name().eq_ignore_ascii_case(name) || format!("{t:?}").eq_ignore_ascii_case(name)
        })
    }

    /// The theme after this one, wrapping around.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    fn index(self) -> u8 {
        Self::ALL
            .iter()
            .position(|t| *t == self)
            .and_then(|i| u8::try_from(i).ok())
            .unwrap_or(0)
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
    pub const ALL: [Self; 2] = [Self::Standard, Self::Deeper];

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Deeper => "Deeper",
        }
    }

    /// The style for a `--style` argument.
    pub fn from_name(name: &str) -> Option<Self> {
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
    pub const ALL: [Self; 2] = [Self::Comfortable, Self::Dense];

    pub fn label(self) -> &'static str {
        match self {
            Self::Comfortable => "Comfortable",
            Self::Dense => "Dense",
        }
    }

    /// The density for a `--density` argument.
    pub fn from_name(name: &str) -> Option<Self> {
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
            Self::Theme => "Not available with the Contrast theme",
            Self::System => "Your system asks for reduced transparency",
            Self::Backend => "Not supported by this graphics driver",
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
        Self::tint(self.hover_ink, if self.dark { dark } else { light })
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
        Self::tint(self.selection, self.selection_alpha)
    }

    /// Status tints (16% of the theme's [`Tones`]): dialog glyph discs, banners, callouts.
    pub fn success_tint(&self) -> Color32 {
        Self::tint(self.tones.success, TONE_ALPHA)
    }

    pub fn warning_tint(&self) -> Color32 {
        Self::tint(self.tones.warning, TONE_ALPHA)
    }

    /// The danger wash: the destructive dialog's glyph disc, danger banners and callouts.
    pub fn danger_tint(&self) -> Color32 {
        Self::tint(self.tones.danger, TONE_ALPHA)
    }

    pub fn accent_tint(&self) -> Color32 {
        Self::tint(self.tones.accent, TONE_ALPHA)
    }

    /// Highlight of a danger menu item (15% of `danger_base`).
    pub fn danger_highlight(&self) -> Color32 {
        Self::tint(self.danger_base, 38)
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
            .map(|step| mix(fg, self.text, crate::ui::int_f32(step) / 20.0))
            .find(|c| readable(*c))
            .unwrap_or(self.text)
    }

    /// Fill of a ref pill in colour `c` (15%); the current branch is stronger (26%).
    pub const fn pill_fill(c: Color32, current: bool) -> Color32 {
        Self::tint(c, if current { 66 } else { 38 })
    }

    /// The ring of a ref pill (42%), the same for the current branch.
    pub const fn pill_border(c: Color32) -> Color32 {
        Self::tint(c, 107)
    }

    /// Fill of an initials avatar in colour `c` (24%).
    pub const fn avatar_fill(c: Color32) -> Color32 {
        Self::tint(c, 61)
    }

    /// Fill of a count badge in colour `c` (24%).
    pub const fn badge_fill(c: Color32) -> Color32 {
        Self::tint(c, 61)
    }

    /// Fill of a danger count badge (22% of `danger_base`), labelled in `ink(danger, ..)`.
    pub fn danger_badge_fill(&self) -> Color32 {
        Self::tint(self.danger_base, 56)
    }

    /// Fill of a file status letter in colour `c` (18%).
    pub const fn change_fill(c: Color32) -> Color32 {
        Self::tint(c, 46)
    }

    /// A translucent version of `c` for fills behind text (premultiplied).
    pub const fn tint(c: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied_const(c.r(), c.g(), c.b(), alpha)
    }

    /// The material of `level` in `style` for this theme: its shadow layers in the theme's
    /// [`ShadowCharacter`] (`Design::shadows`); Contrast has no highlights.
    pub fn material(&self, style: Style, level: Elevation) -> Material {
        let m = *material(style, self.dark, level);
        let m = Material {
            layers: shaped_layers(self.design().shadows, style, self.dark, level, m.layers),
            ..m
        };
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

    /// The structure of the theme this palette belongs to (Midnight's for a palette no theme
    /// owns).
    pub fn design(&self) -> &'static Design {
        ThemeId::ALL
            .into_iter()
            .find(|t| t.palette().name == self.name)
            .map_or(&MIDNIGHT_DESIGN, ThemeId::design)
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

    /// The fill of a translucent floating layer over `s`: the overlay leaning to the theme's light
    /// (`Design::glass_tint` over `overlay`) at `glass_alpha`.
    pub fn glass(&self, s: &Surfaces) -> Color32 {
        Self::tint(over(self.design().glass_tint, s.overlay), self.glass_alpha)
    }

    /// Ring of floating layers: `overlay_edge`, or `control_border` in Contrast.
    pub fn floating_ring(&self, s: &Surfaces) -> Color32 {
        if self.strict {
            self.control_border
        } else {
            s.overlay_edge
        }
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
    let mix =
        |f: u8, b: u8| crate::ui::saturate_u8((f32::from(f) + f32::from(b) * (1.0 - a)).round());
    Color32::from_rgb(
        mix(fg.r(), bg.r()),
        mix(fg.g(), bg.g()),
        mix(fg.b(), bg.b()),
    )
}

/// `a` moved towards `b` by `t` (0 to 1).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| {
        crate::ui::saturate_u8((f32::from(x) + (f32::from(y) - f32::from(x)) * t).round())
    };
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
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "v is clamped to 0..=1 first, so the channel is 0..=255"
    )]
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
    static INKS: std::cell::RefCell<HashMap<InkKey, Color32>> =
        std::cell::RefCell::new(HashMap::new());
}

/// The palette of the active theme.
pub fn palette(_ctx: &egui::Context) -> &'static Palette {
    active().palette()
}

/// The surfaces of the active theme and style.
pub fn surfaces(_ctx: &egui::Context) -> &'static Surfaces {
    active().palette().surfaces(style())
}

/// The structure of the active theme: how it draws corners, edges, shadows, selection, focus,
/// labels, headings, the graph and its art.
pub fn design(_ctx: &egui::Context) -> &'static Design {
    active().design()
}

// -------------------------------------------------------------------------------------- design

pub use design::*;

/// The per-theme design record (wave 3): the structure half of each theme.
mod design {
    use egui::{Color32, FontFamily};

    use super::{Elevation, FULL, Palette, ShadowLayer, hex, medium, semibold};

    /// A theme's structure, the half of its identity colour cannot carry: corners, edges, shadows,
    /// how panes separate, the ground, the interaction voice, type, the graph, icons, art and motion.
    /// Every field changes what the kit and the views paint (`contracts/theme-2.md` says how); the
    /// colours stay in the [`Palette`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Design {
        /// Factor on every radius of the active [`Metrics`] ([`Design::radius`]): below 1 sharper and
        /// more technical, above 1 softer and more rounded. [`FULL`] stays full.
        pub radius_scale: f32,
        /// Width in points of seams, card edges and control rings.
        pub edge_width: f32,
        pub edges: EdgeMode,
        pub shadows: ShadowCharacter,
        /// How Standard's flush regions part (Deeper always floats cards).
        pub panes: PaneSeparation,
        /// Treatment of the window ground where no content sits: Deeper's gutter, the welcome screen
        /// and empty states. Content grounds stay flat.
        pub ground: Ground,
        pub selection: SelectionStyle,
        pub focus: FocusStyle,
        pub hover: HoverStyle,
        /// The active repository tab and the active view tab.
        pub active_tab: TabStyle,
        /// Links are underlined at rest, not only when hovered or focused: where the link colour
        /// alone does not set a link apart from body text (Paper) or for low vision (Contrast).
        pub link_underline: bool,
        /// Section labels (sidebar sections, settings groups, table headers, menu headings).
        pub labels: LabelStyle,
        /// Face of titles and headlines; body text stays Inter.
        pub heading: HeadingFace,
        pub graph: GraphStyle,
        pub icons: IconWeight,
        /// The motif of the welcome backdrop and the empty-state illustrations (`ui/art.rs`).
        pub art: ArtMotif,
        /// Easing and pace within the user's Motion setting (Reduced still turns motion off).
        pub motion: MotionCharacter,
        /// Translucent surfaces: a premultiplied tint laid over `Surfaces::overlay` before the glass
        /// opacity, so the glass leans to the theme's light. Transparent: plain overlay.
        pub glass_tint: Color32,
    }

    /// How region edges and control rings are drawn.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum EdgeMode {
        /// 1 px seams in `Surfaces::border` between regions and under headers; controls ringed.
        Hairline,
        /// No seams between regions of different tone: the tonal step alone parts them (a seam only
        /// where both sides share one tone). Controls keep their rings.
        Tonal,
        /// Strong lines: seams and card edges in `Palette::control_border` at `edge_width`.
        Outline,
        /// Hairlines, plus edges that light up: the focused pane, the active tab and primary buttons
        /// carry a 1 px `focus` edge with a soft glow of it (neon tubes).
        Lit,
    }

    /// Character of shadows, applied to every [`Material`] layer by [`Palette::material`]
    /// ([`ShadowCharacter::shape`]).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ShadowCharacter {
        /// The material tables as they are: long, soft, low-alpha layers.
        Soft,
        /// Short and close like a sheet on a desk: blur under half, offset halved, slightly darker.
        Crisp,
        /// No shadows below floating layers; floating layers keep a faint, short one so they lift.
        Flat,
        /// Light, not shade: every layer centred under its surface (no drop) and a little wider.
        Glow,
    }

    /// How Standard's flush regions (sidebar, content, details) part.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PaneSeparation {
        /// A 1 px seam in `Surfaces::border`.
        Seams,
        /// The tone step alone (sidebar and content differ by `Surfaces::panel`).
        Tonal,
        /// A strong rule in `Palette::control_border` at `Design::edge_width`.
        Outline,
    }

    /// The window ground's treatment where no content sits. Every variant is a translucent `ink`
    /// laid over the style's ground (`Surfaces::bg`, and `pane` on the welcome screen in Standard),
    /// so text and controls painted over it keep their contrast (tested).
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum Ground {
        Flat,
        /// `ink` at `top` alpha at the top edge, fading linearly to `bottom` alpha at the bottom.
        Vertical {
            ink: Color32,
            top: u8,
            bottom: u8,
        },
        /// A radial glow of `ink` at `alpha`, centred at `center` (fractions of the rect, may lie
        /// outside it) with a radius of `radius` times the rect's longer side, fading smoothly
        /// (`kit::paint`'s banding-free glow).
        Glow {
            ink: Color32,
            alpha: u8,
            center: (f32, f32),
            radius: f32,
        },
        /// The edges darkened by `ink` at `alpha` (aged stock), clear in the middle.
        Vignette {
            ink: Color32,
            alpha: u8,
        },
    }

    /// How a selected row, item or chip is marked. Every style keeps the [`Palette::selected`] wash
    /// under it, so the contrast tests hold for all of them.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum SelectionStyle {
        /// The wash alone.
        Wash,
        /// The wash plus a leading bar of `width` points in `Palette::focus`, full row height,
        /// inset by the row's radius.
        Bar { width: f32 },
        /// The wash plus a ring of `width` points in `Palette::focus` at 70%.
        Outline { width: f32 },
        /// The wash plus a soft inner glow of `Palette::selection` along its edges (6 pt, 40% to 0).
        Glow,
    }

    /// The keyboard focus ring.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum FocusStyle {
        /// A ring of `width` in `Palette::focus`, `offset` points outside the control.
        Ring { width: f32, offset: f32 },
        /// A 1.5 pt ring of `Palette::focus` plus a soft glow of it `width` points wide (35% to 0).
        Glow { width: f32 },
        /// A ring of `width` in `Palette::focus` with a 1 pt ring of the ground inside it, so it
        /// reads on any fill.
        Double { width: f32 },
    }

    /// What a hovered row or quiet control shows.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HoverStyle {
        /// The translucent [`Palette::hover`] wash.
        Tint,
        /// The wash plus the raised fill's top highlight and a contact shadow: the item lifts.
        Lift,
        /// The wash plus a 1 px ring in `Surfaces::edge` (`control_border` in Contrast).
        Edge,
    }

    /// The active repository tab and view tab.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum TabStyle {
        /// A filled, ringed tab lifted off the strip (`Surfaces::tab`, `tab_edge`).
        Lifted,
        /// No fill: an underline of `width` points in `Palette::focus` under the label.
        Underline { width: f32 },
        /// A filled tab with a rule of `width` points in `Palette::accent` along its top edge
        /// (a bookmark).
        TopRule { width: f32 },
    }

    /// Letter case of section labels.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum LabelCase {
        /// ALL CAPS at `Metrics::micro_size`.
        Upper,
        /// As written (labels are authored in title case), at `Metrics::small_size`.
        Title,
    }

    /// A weight of the UI face.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Weight {
        Medium,
        Semibold,
    }

    impl Weight {
        /// The bundled face of this weight.
        pub fn family(self) -> FontFamily {
            match self {
                Self::Medium => medium(),
                Self::Semibold => semibold(),
            }
        }
    }

    /// Section labels: case, letter-spacing (em, times the label's size) and weight.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct LabelStyle {
        pub case: LabelCase,
        pub tracking: f32,
        pub weight: Weight,
    }

    impl LabelStyle {
        /// `text` in this style's case.
        pub fn cased(self, text: &str) -> String {
            match self.case {
                LabelCase::Upper => text.to_uppercase(),
                LabelCase::Title => text.to_owned(),
            }
        }
    }

    /// Face of titles and headlines.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HeadingFace {
        /// Inter Semibold, like the rest of the interface.
        Ui,
        /// Canopy Serif Bold (a Latin subset of PT Serif Bold): a book face for Paper's titles.
        Serif,
        /// Atkinson Hyperlegible Bold: designed for low vision, for Contrast's titles.
        Hyperlegible,
    }

    impl HeadingFace {
        /// The egui family name the face is registered under (`ui/fonts.rs`).
        pub const fn family_name(self) -> &'static str {
            match self {
                Self::Ui => "semibold",
                Self::Serif => "heading-serif",
                Self::Hyperlegible => "heading-hyperlegible",
            }
        }

        pub fn family(self) -> FontFamily {
            FontFamily::Name(self.family_name().into())
        }

        /// Size factor that gives the face Inter's apparent size (x-height), applied to heading sizes.
        pub const fn size_scale(self) -> f32 {
            match self {
                Self::Ui => 1.0,
                Self::Serif => 1.06,
                Self::Hyperlegible => 1.04,
            }
        }
    }

    /// Shape of a commit node in the graph.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum NodeShape {
        /// A filled circle on a ring of the row's ground.
        Dot,
        /// A hollow circle: a ring in the lane colour around the row's ground.
        Ring,
        /// A filled square with `Metrics::radius_sm` scaled corners.
        Square,
        /// A filled diamond (a square turned 45 degrees).
        Diamond,
    }

    /// How an edge between lanes is drawn.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Curve {
        /// A cubic curve leaving and entering vertically.
        Smooth,
        /// Vertical, then a rounded corner, horizontal, a rounded corner, vertical (a metro map).
        Rounded,
        /// Vertical, then a straight diagonal across one row, then vertical (a circuit trace).
        Angular,
    }

    /// The history graph's drawing.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct GraphStyle {
        pub node: NodeShape,
        /// Factor on `Metrics::graph_node_r` and `graph_head_halo`.
        pub node_scale: f32,
        /// Factor on `Metrics::graph_edge` and `graph_merge_stroke`.
        pub line_scale: f32,
        pub curve: Curve,
    }

    /// Phosphor weight for icons.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IconWeight {
        /// Regular everywhere.
        Regular,
        /// Regular, with the `fill` glyph for active and selected icons (the active view tab, a
        /// toggled tool, the selected sidebar row, the checked menu item).
        FillAccents,
    }

    /// The motif `ui/art.rs` draws the welcome backdrop and the empty states from.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ArtMotif {
        /// Midnight: stars joined into constellations, the commits as stars.
        Constellation,
        /// Daylight: drafting geometry: circles, construction lines and a low sun.
        Drafting,
        /// Aurora: curtains of light in bands across the top.
        Aurora,
        /// Ember: sparks rising from a warm glow below.
        Embers,
        /// Neon: a perspective grid running to a lit horizon.
        NeonGrid,
        /// Forest: layered branching silhouettes, a canopy over the tree.
        Canopy,
        /// Paper: ink strokes and fibres on the stock, a fountain-pen line.
        Ink,
        /// Contrast: bold line art, no glows or gradients.
        Signal,
    }

    /// Easing and pace of the theme's motion.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MotionCharacter {
        /// Quick and exact: 0.8 times the durations.
        Crisp,
        Standard,
        /// Unhurried: 1.2 times the durations, ease-in-out instead of ease-out for enters.
        Soft,
    }

    impl MotionCharacter {
        /// Factor on every `ui::motion` duration.
        pub const fn duration_scale(self) -> f32 {
            match self {
                Self::Crisp => 0.8,
                Self::Standard => 1.0,
                Self::Soft => 1.2,
            }
        }
    }

    impl Design {
        /// `r` (a [`Metrics`] radius) in this theme: scaled and rounded, [`FULL`] kept full, and a
        /// rounded corner never squared off entirely.
        pub fn radius(&self, r: u8) -> u8 {
            if r == FULL || r == 0 {
                return r;
            }
            crate::ui::saturate_u8((f32::from(r) * self.radius_scale).round()).max(1)
        }
    }

    impl ShadowCharacter {
        pub const ALL: [Self; 4] = [Self::Soft, Self::Crisp, Self::Flat, Self::Glow];

        /// Shadow `layer` of `level` in this character, or `None` when it draws no shadow there.
        pub fn shape(self, level: Elevation, layer: ShadowLayer) -> Option<ShadowLayer> {
            let scale = |v: u8, f: f32| crate::ui::saturate_u8((f32::from(v) * f).round());
            match self {
                Self::Soft => Some(layer),
                Self::Crisp => Some(ShadowLayer {
                    dy: layer.dy.signum() * (layer.dy.abs() / 2).max(1),
                    blur: scale(layer.blur, 0.45).max(1),
                    alpha: scale(layer.alpha, 1.1),
                    ..layer
                }),
                Self::Flat => (level == Elevation::Floating).then_some(ShadowLayer {
                    dy: layer.dy.min(4),
                    blur: scale(layer.blur, 0.4),
                    alpha: scale(layer.alpha, 0.6),
                    ..layer
                }),
                Self::Glow => Some(ShadowLayer {
                    dy: 0,
                    blur: scale(layer.blur, 1.2),
                    spread: layer.spread.max(0),
                    alpha: scale(layer.alpha, 0.8),
                    ..layer
                }),
            }
        }
    }

    const fn labels(case: LabelCase, tracking: f32, weight: Weight) -> LabelStyle {
        LabelStyle {
            case,
            tracking,
            weight,
        }
    }

    const fn graph(node: NodeShape, node_scale: f32, line_scale: f32, curve: Curve) -> GraphStyle {
        GraphStyle {
            node,
            node_scale,
            line_scale,
            curve,
        }
    }

    /// Midnight, "Observatory": precise as an instrument. Hairlines, soft long shadows, a selection
    /// bar like an indicator, a faint zenith-to-horizon lift, constellations.
    pub const MIDNIGHT_DESIGN: Design = Design {
        radius_scale: 1.0,
        edge_width: 1.0,
        edges: EdgeMode::Hairline,
        shadows: ShadowCharacter::Soft,
        panes: PaneSeparation::Seams,
        ground: Ground::Vertical {
            ink: hex(0x7F89FF),
            top: 0,
            bottom: 12,
        },
        selection: SelectionStyle::Bar { width: 2.0 },
        focus: FocusStyle::Ring {
            width: 2.0,
            offset: 1.0,
        },
        hover: HoverStyle::Tint,
        active_tab: TabStyle::Lifted,
        link_underline: false,
        labels: labels(LabelCase::Upper, 0.07, Weight::Semibold),
        heading: HeadingFace::Ui,
        graph: graph(NodeShape::Dot, 1.0, 1.0, Curve::Smooth),
        icons: IconWeight::Regular,
        art: ArtMotif::Constellation,
        motion: MotionCharacter::Standard,
        glass_tint: Palette::tint(hex(0x1A1F4A), 28),
    };

    /// Daylight, "Drafting studio": crisp daylight geometry. Tighter corners, hairlines, short cool
    /// shadows, a clean cobalt wash, an underlined active tab, hollow nodes on a metro map.
    pub const DAYLIGHT_DESIGN: Design = Design {
        radius_scale: 0.72,
        edge_width: 1.0,
        edges: EdgeMode::Hairline,
        shadows: ShadowCharacter::Crisp,
        panes: PaneSeparation::Seams,
        ground: Ground::Flat,
        selection: SelectionStyle::Wash,
        focus: FocusStyle::Ring {
            width: 2.0,
            offset: 1.0,
        },
        hover: HoverStyle::Tint,
        active_tab: TabStyle::Underline { width: 2.0 },
        link_underline: false,
        labels: labels(LabelCase::Title, 0.0, Weight::Semibold),
        heading: HeadingFace::Ui,
        graph: graph(NodeShape::Ring, 1.05, 0.85, Curve::Rounded),
        icons: IconWeight::Regular,
        art: ArtMotif::Drafting,
        motion: MotionCharacter::Crisp,
        glass_tint: Palette::tint(hex(0xDCE5FF), 30),
    };

    /// Aurora, "Polar night": luminous and soft. Rounder corners, no hard seams, a glow from the top
    /// of the sky, a lit selection band, a glowing focus ring, a band of light under the active
    /// tab, hollow nodes like stars, curtains of light.
    pub const AURORA_DESIGN: Design = Design {
        radius_scale: 1.1,
        edge_width: 1.0,
        edges: EdgeMode::Tonal,
        shadows: ShadowCharacter::Soft,
        panes: PaneSeparation::Tonal,
        ground: Ground::Glow {
            ink: hex(0x5FD6E0),
            alpha: 20,
            center: (0.5, -0.25),
            radius: 1.1,
        },
        selection: SelectionStyle::Glow,
        focus: FocusStyle::Glow { width: 4.0 },
        hover: HoverStyle::Tint,
        active_tab: TabStyle::Underline { width: 2.0 },
        link_underline: false,
        labels: labels(LabelCase::Upper, 0.09, Weight::Medium),
        heading: HeadingFace::Ui,
        graph: graph(NodeShape::Ring, 1.05, 1.1, Curve::Smooth),
        icons: IconWeight::Regular,
        art: ArtMotif::Aurora,
        motion: MotionCharacter::Soft,
        glass_tint: Palette::tint(hex(0x0B3A44), 30),
    };

    /// Ember, "Forge at dusk": warm and hand-made. The softest corners, tonal fields instead of
    /// lines, a hearth glow rising from below, items that lift when hovered, sparks.
    pub const EMBER_DESIGN: Design = Design {
        radius_scale: 1.3,
        edge_width: 1.0,
        edges: EdgeMode::Tonal,
        shadows: ShadowCharacter::Soft,
        panes: PaneSeparation::Tonal,
        ground: Ground::Glow {
            ink: hex(0xF0954A),
            alpha: 18,
            center: (0.5, 1.2),
            radius: 1.0,
        },
        selection: SelectionStyle::Wash,
        focus: FocusStyle::Glow { width: 4.0 },
        hover: HoverStyle::Lift,
        active_tab: TabStyle::Lifted,
        link_underline: false,
        labels: labels(LabelCase::Title, 0.01, Weight::Semibold),
        heading: HeadingFace::Ui,
        graph: graph(NodeShape::Dot, 1.1, 1.2, Curve::Smooth),
        icons: IconWeight::FillAccents,
        art: ArtMotif::Embers,
        motion: MotionCharacter::Soft,
        glass_tint: Palette::tint(hex(0x4A2410), 26),
    };

    /// Neon, "Night city": sharp and lit. Near-square corners, edges that glow instead of shadows
    /// that drop, an outlined selection, wide-tracked signage labels, diamonds on circuit traces, a
    /// grid running to a magenta horizon.
    pub const NEON_DESIGN: Design = Design {
        radius_scale: 0.4,
        edge_width: 1.0,
        edges: EdgeMode::Lit,
        shadows: ShadowCharacter::Glow,
        panes: PaneSeparation::Seams,
        ground: Ground::Vertical {
            ink: hex(0xFF4FCB),
            top: 0,
            bottom: 16,
        },
        selection: SelectionStyle::Outline { width: 1.0 },
        focus: FocusStyle::Glow { width: 5.0 },
        hover: HoverStyle::Edge,
        active_tab: TabStyle::Underline { width: 2.0 },
        link_underline: false,
        labels: labels(LabelCase::Upper, 0.14, Weight::Semibold),
        heading: HeadingFace::Ui,
        graph: graph(NodeShape::Diamond, 1.1, 1.0, Curve::Angular),
        icons: IconWeight::Regular,
        art: ArtMotif::NeonGrid,
        motion: MotionCharacter::Crisp,
        glass_tint: Palette::tint(hex(0x3A0F4A), 30),
    };

    /// Forest, "Evergreen at dusk": organic and calm. Rounded corners, tonal fields, sunlight
    /// filtering down from above, a trunk-like selection bar, thick branch-like lines, a canopy.
    pub const FOREST_DESIGN: Design = Design {
        radius_scale: 1.2,
        edge_width: 1.0,
        edges: EdgeMode::Tonal,
        shadows: ShadowCharacter::Soft,
        panes: PaneSeparation::Tonal,
        ground: Ground::Vertical {
            ink: hex(0xE7BF57),
            top: 12,
            bottom: 0,
        },
        selection: SelectionStyle::Bar { width: 3.0 },
        focus: FocusStyle::Ring {
            width: 2.0,
            offset: 1.0,
        },
        hover: HoverStyle::Tint,
        active_tab: TabStyle::Lifted,
        link_underline: false,
        labels: labels(LabelCase::Title, 0.02, Weight::Medium),
        heading: HeadingFace::Ui,
        graph: graph(NodeShape::Dot, 1.0, 1.3, Curve::Smooth),
        icons: IconWeight::Regular,
        art: ArtMotif::Canopy,
        motion: MotionCharacter::Soft,
        glass_tint: Palette::tint(hex(0x123A20), 26),
    };

    /// Paper, "Letterpress": printed matter. Sharp corners, ruled hairlines, short warm sheet
    /// shadows, aged edges on the stock, a highlighter selection, a bookmark on the active tab,
    /// small-caps labels, serif titles, ink rings on the graph.
    pub const PAPER_DESIGN: Design = Design {
        radius_scale: 0.45,
        edge_width: 1.0,
        edges: EdgeMode::Hairline,
        shadows: ShadowCharacter::Crisp,
        panes: PaneSeparation::Seams,
        ground: Ground::Vignette {
            ink: hex(0x6B4A22),
            alpha: 18,
        },
        selection: SelectionStyle::Wash,
        focus: FocusStyle::Ring {
            width: 1.5,
            offset: 1.0,
        },
        hover: HoverStyle::Tint,
        active_tab: TabStyle::TopRule { width: 2.0 },
        link_underline: true,
        labels: labels(LabelCase::Upper, 0.11, Weight::Semibold),
        heading: HeadingFace::Serif,
        graph: graph(NodeShape::Ring, 1.0, 0.9, Curve::Smooth),
        icons: IconWeight::Regular,
        art: ArtMotif::Ink,
        motion: MotionCharacter::Standard,
        glass_tint: Palette::tint(hex(0xFFF3D6), 30),
    };

    /// Contrast, "Signal": maximum legibility, still designed. Strong outlines instead of shadows,
    /// an outlined selection, a double focus ring, a thick underline, bold labels, Atkinson
    /// Hyperlegible titles, square nodes on thick lines, line art without glows.
    pub const CONTRAST_DESIGN: Design = Design {
        radius_scale: 0.6,
        edge_width: 1.5,
        edges: EdgeMode::Outline,
        shadows: ShadowCharacter::Flat,
        panes: PaneSeparation::Outline,
        ground: Ground::Flat,
        selection: SelectionStyle::Outline { width: 2.0 },
        focus: FocusStyle::Double { width: 2.5 },
        hover: HoverStyle::Edge,
        active_tab: TabStyle::Underline { width: 3.0 },
        link_underline: true,
        labels: labels(LabelCase::Upper, 0.06, Weight::Semibold),
        heading: HeadingFace::Hyperlegible,
        graph: graph(NodeShape::Square, 1.15, 1.25, Curve::Rounded),
        icons: IconWeight::FillAccents,
        art: ArtMotif::Signal,
        motion: MotionCharacter::Crisp,
        glass_tint: Color32::TRANSPARENT,
    };
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

/// The colour of a material's top highlight line: white at `alpha` (`Material::highlight`).
pub fn highlight_line(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
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
    pub const ALL: [Self; 10] = [
        Self::Control,
        Self::Thumb,
        Self::Knob,
        Self::Tab,
        Self::Primary,
        Self::Danger,
        Self::Floating,
        Self::Pane,
        Self::Field,
        Self::Deck,
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
        mat(0, &[layer(14, 36, -10, 64), layer(2, 6, 0, 20)]),
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
        mat(16, &[layer(1, 2, 0, 140), layer(10, 26, -10, 205)]),
        Material {
            highlight: 0,
            bottom_line: 0,
            recess: 71,
            layers: &[],
        },
        mat(20, &[layer(1, 2, 0, 128), layer(8, 22, -10, 179)]),
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
        mat(0, &[layer(1, 2, 0, 20), layer(8, 24, -12, 66)]),
        Material {
            highlight: 0,
            bottom_line: 0,
            recess: 18,
            layers: &[],
        },
        mat(0, &[layer(1, 2, 0, 20), layer(6, 18, -10, 56)]),
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

/// `layers` (the material of `level` in `style`, dark or light) in shadow character `shadows`,
/// worked out once for every combination: `Soft` is the tables as they are.
fn shaped_layers(
    shadows: ShadowCharacter,
    style: Style,
    dark: bool,
    level: Elevation,
    layers: &'static [ShadowLayer],
) -> &'static [ShadowLayer] {
    type Key = (ShadowCharacter, Style, bool, Elevation);
    static SHAPED: LazyLock<HashMap<Key, Vec<ShadowLayer>>> = LazyLock::new(|| {
        let mut out = HashMap::new();
        for shadows in ShadowCharacter::ALL {
            for style in [Style::Standard, Style::Deeper] {
                for dark in [true, false] {
                    for level in Elevation::ALL {
                        let shaped = material(style, dark, level)
                            .layers
                            .iter()
                            .filter_map(|l| shadows.shape(level, *l))
                            .collect();
                        out.insert((shadows, style, dark, level), shaped);
                    }
                }
            }
        }
        out
    });
    if shadows == ShadowCharacter::Soft {
        return layers;
    }
    SHAPED
        .get(&(shadows, style, dark, level))
        .map_or(layers, Vec::as_slice)
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
    /// A status dot's diameter (spec 4.18: 8, 6 in Dense).
    pub status_dot: f32,
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
    status_dot: 8.0,
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
    status_dot: 6.0,
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

/// The metrics of a (density, style) pair in `theme`: every corner through the theme's radius
/// scale ([`Design::radius`]), so the kit and the views draw the theme's corners by reading
/// [`metrics`] as they always have.
pub fn metrics_in(theme: ThemeId, density: Density, style: Style) -> &'static Metrics {
    const PAIRS: [(Density, Style); 4] = [
        (Density::Comfortable, Style::Standard),
        (Density::Dense, Style::Standard),
        (Density::Comfortable, Style::Deeper),
        (Density::Dense, Style::Deeper),
    ];
    static SHAPED: LazyLock<Vec<Metrics>> = LazyLock::new(|| {
        ThemeId::ALL
            .into_iter()
            .flat_map(|t| PAIRS.map(|(d, s)| metrics_for(d, s).shaped(t.design())))
            .collect()
    });
    let pair = PAIRS
        .iter()
        .position(|&pair| pair == (density, style))
        .unwrap_or(0);
    SHAPED
        .get(usize::from(theme.index()) * PAIRS.len() + pair)
        .unwrap_or_else(|| metrics_for(density, style))
}

/// The active metrics, in the active theme's corners.
pub fn metrics() -> &'static Metrics {
    metrics_in(active(), density(), style())
}

impl Metrics {
    /// These metrics with every radius in `design`'s scale.
    pub fn shaped(&self, design: &Design) -> Self {
        Self {
            radius_sm: design.radius(self.radius_sm),
            radius: design.radius(self.radius),
            radius_lg: design.radius(self.radius_lg),
            pill_radius: design.radius(self.pill_radius),
            doc_tab_radius: design.radius(self.doc_tab_radius),
            card_radius: design.radius(self.card_radius),
            pane_radius: design.radius(self.pane_radius),
            keycap_radius: design.radius(self.keycap_radius),
            ..*self
        }
    }

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

/// A section label (sidebar sections, settings groups, table column headers, menu headings) in
/// the active theme's [`LabelStyle`]: its case, size (`micro_size` in capitals, `small_size` in
/// title case), weight and tracking.
pub fn section_label(text: &str, color: Color32) -> egui::text::LayoutJob {
    let style = active().design().labels;
    let m = metrics();
    let size = match style.case {
        LabelCase::Upper => m.micro_size,
        LabelCase::Title => m.small_size,
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &style.cased(text),
        0.0,
        egui::TextFormat {
            font_id: FontId::new(size, style.weight.family()),
            color,
            extra_letter_spacing: style.tracking * size,
            ..Default::default()
        },
    );
    job
}

/// A title or headline at `size` (`Metrics::title_size`, `headline_size`, `subtitle_size` for card
/// titles, `display_size`, `hero_size`) in the active theme's heading face ([`HeadingFace`]).
pub fn heading(size: f32) -> FontId {
    let face = active().design().heading;
    FontId::new(size * face.size_scale(), face.family())
}

/// An icon glyph's font at `size`: the filled Phosphor glyph when `lit` (active, selected or
/// toggled) and the theme fills its active icons ([`IconWeight::FillAccents`]), else the regular
/// one.
pub fn icon_font(size: f32, lit: bool) -> FontId {
    if lit && active().design().icons == IconWeight::FillAccents {
        FontId::new(size, FontFamily::Name(super::fonts::FILL.into()))
    } else {
        FontId::proportional(size)
    }
}

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

/// Always-visible scroll bars for code, diff and detail boxes, for position awareness (section
/// 4.21): the floating bars' thin handle in the text colour at 45% (75% while hovered or dragged),
/// with its room kept beside the content so the handle never covers text.
pub fn solid_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating_allocated_width: SM,
        dormant_handle_opacity: 0.45,
        active_handle_opacity: 0.45,
        ..floating_scroll()
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
pub fn apply(ctx: &egui::Context, id: ThemeId, look: Appearance) {
    look_set(id, look.style, look.density, look.glass);
    super::motion::set_reduced(look.reduce_motion);
    let p = id.palette();
    let m = metrics_in(id, look.density, look.style);
    let theme = if p.dark { Theme::Dark } else { Theme::Light };
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals(p, look.style, m);
    let face = id.design().heading;
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(m.title_size * face.size_scale(), face.family()),
        ),
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
    sp.window_margin = Margin::same(crate::ui::points_i8(m.dialog_pad));
    sp.menu_margin = Margin::same(crate::ui::points_i8(m.menu_pad));
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
    style.scroll_animation = super::motion::scroll_animation();
    ctx.set_style_of(theme, style);
    ctx.set_theme(theme);
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

    /// (`text`, `text_dim`, `text_faint`, `outline` and `focus`) minimums: Contrast is stricter.
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
                gate.need(
                    "on_tooltip_key",
                    s.on_tooltip_key,
                    "tooltip_key",
                    s.tooltip_key,
                    dim_min,
                );
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
                        Palette::avatar_fill(c),
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

    /// A disabled control stays readable (QA wave 1: labels at 1.3 to 2.3:1): `disabled_text` at
    /// 3:1 (4.5:1 in Contrast) on every surface a disabled control is painted on, dimmer than
    /// `text_faint` so it still reads as disabled, on a neutral fill with a ring of its own.
    #[test]
    fn disabled_tokens_are_readable() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            let min = if p.strict { 4.5 } else { 3.0 };
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let mut bgs = vec![
                    ("disabled_fill".to_owned(), s.disabled_fill),
                    ("pane".to_owned(), s.pane),
                    ("panel".to_owned(), s.panel),
                    ("elevated".to_owned(), s.elevated),
                    ("overlay".to_owned(), s.overlay),
                    ("field".to_owned(), s.field),
                ];
                for (name, fill) in [
                    ("raised", s.raised),
                    ("raised_hover", s.raised_hover),
                    ("raised_pressed", s.raised_pressed),
                ] {
                    for (end, c) in ["top", "bottom"].into_iter().zip(fill.ends()) {
                        bgs.push((format!("{name}.{end}"), c));
                    }
                }
                for (on, bg) in with_hovers(p, &bgs[..6]) {
                    gate.need("disabled_text", p.disabled_text, &on, bg, min);
                }
                for (on, bg) in &bgs[6..] {
                    gate.need("disabled_text", p.disabled_text, on, *bg, min);
                }
                failures.extend(gate.failures);
                assert!(
                    contrast(p.disabled_text, s.pane) < contrast(p.text_faint, s.pane),
                    "{}/{style:?}: disabled_text reads dimmer than text_faint",
                    p.name
                );
                assert_ne!(s.disabled_edge, s.disabled_fill, "{}", p.name);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// CIELAB (D65) of an opaque colour.
    fn lab(c: Color32) -> [f64; 3] {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.950_47;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.088_83;
        let f = |t: f64| {
            if t > 216.0 / 24389.0 {
                t.cbrt()
            } else {
                (24389.0 / 27.0 * t + 16.0) / 116.0
            }
        };
        let (fx, fy, fz) = (f(x), f(y), f(z));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    /// CIE76 colour difference.
    fn delta_e(a: Color32, b: Color32) -> f64 {
        let (a, b) = (lab(a), lab(b));
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    /// Two branches never draw in look-alike colours: every pair of a theme's lanes at least
    /// dE 25 apart (QA wave 1 found pairs down to dE 6).
    #[test]
    fn graph_lanes_are_distinct() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            for (i, a) in p.lanes.iter().enumerate() {
                for (j, b) in p.lanes.iter().enumerate().skip(i + 1) {
                    let d = delta_e(*a, *b);
                    if d < 25.0 {
                        failures.push(format!("{}: lanes {i} and {j} are dE {d:.1}", p.name));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The status washes the kit paints (banners, callouts, discs) keep text and their own status
    /// colour readable, whatever tone a theme gives them.
    #[test]
    fn status_washes_keep_text_readable() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, ..) = targets(p);
                for (name, wash, fg) in [
                    ("accent", p.accent_tint(), p.accent),
                    ("success", p.success_tint(), p.success),
                    ("warning", p.warning_tint(), p.warning),
                    ("danger", p.danger_tint(), p.danger),
                ] {
                    for (on, bg) in grounds(s, Style::Standard) {
                        let b = over(wash, bg);
                        gate.need(
                            "text",
                            p.text,
                            &format!("{name} wash over {on}"),
                            b,
                            text_min,
                        );
                        gate.need(name, fg, &format!("its wash over {on}"), b, 4.5);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Paper's sepia links share body text's hue family and Contrast serves low vision, so there a
    /// link carries a cue beyond colour: it is underlined at rest (QV2-3, WCAG 1.4.1). Elsewhere
    /// the underline appears on hover.
    #[test]
    fn links_are_underlined_at_rest_in_paper_and_contrast() {
        use crate::ui::kit::button::LinkButton;
        for id in ThemeId::ALL {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            apply(&ctx, id, Appearance::STILL);
            let mut text = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                text = LinkButton::new("Create a token").show(ui).rect;
            });
            out.textures_delta.clear();
            let p = id.palette();
            let underlined = out.shapes.iter().any(|c| match &c.shape {
                egui::Shape::LineSegment { points, stroke } => {
                    stroke.color == p.accent2
                        && (points[0].y - points[1].y).abs() < 0.01
                        && points[0].y > text.center().y
                        && points[0].y <= text.bottom() + 2.0
                }
                _ => false,
            });
            let want = matches!(id, ThemeId::Paper | ThemeId::Contrast);
            assert_eq!(underlined, want, "{id:?}");
            assert_eq!(id.design().link_underline, want, "{id:?}");
        }
    }

    /// A banner's or callout's action can be disabled ("Continue" while conflicts remain): disabled
    /// text keeps the 3:1 floor on every status wash over every ground, and a clear margin, 3.5:1,
    /// on the grounds banners sit on (pane, panel, Deeper's window ground), so it still reads as
    /// the step that will come (QV2-9: Ember was 3.16:1; washes over dialogs were down to 2.71:1).
    /// Contrast keeps 4.5:1 everywhere.
    #[test]
    fn disabled_actions_read_on_banners() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            let floor = if p.strict { 4.5 } else { 3.0 };
            let banner = if p.strict { 4.5 } else { 3.5 };
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                for (name, wash) in [
                    ("accent", p.accent_tint()),
                    ("success", p.success_tint()),
                    ("warning", p.warning_tint()),
                    ("danger", p.danger_tint()),
                ] {
                    for (on, bg) in grounds(s, style) {
                        let min = if matches!(on.as_str(), "pane" | "panel" | "deeper ground") {
                            banner
                        } else {
                            floor
                        };
                        gate.need(
                            "disabled_text",
                            p.disabled_text,
                            &format!("{name} wash over {on}"),
                            over(wash, bg),
                            min,
                        );
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Structure you can see (QA wave 1: every ladder was one uniform 1 to 2 L* step): Standard's
    /// chrome sits at least 2.5 L* off the content ground, and Deeper's cards at least 4 L* off
    /// their ground (Contrast outlines its cards instead).
    #[test]
    fn solid_scroll_bars_stay_thin_and_quiet() {
        let solid = solid_scroll();
        let floating = floating_scroll();
        assert!(solid.floating, "a solid bar is the thin floating handle");
        assert_eq!(solid.floating_width, floating.floating_width);
        assert!(
            solid.floating_allocated_width > 0.0,
            "the bar keeps its room beside the content"
        );
        assert!(
            solid.dormant_handle_opacity > 0.0 && solid.dormant_handle_opacity < 1.0,
            "the handle is always visible but never a solid slab"
        );
        assert_eq!(solid.dormant_handle_opacity, solid.active_handle_opacity);
        assert!(solid.interact_handle_opacity > solid.active_handle_opacity);
    }

    #[test]
    fn surface_steps_are_visible() {
        for id in ThemeId::ALL {
            let p = id.palette();
            let l = |c: Color32| lab(c)[0];
            let st = &p.standard;
            let chrome = (l(st.panel) - l(st.pane)).abs();
            assert!(chrome >= 2.5, "{}: chrome step {chrome:.1} L*", p.name);
            if !p.strict {
                let d = &p.deeper;
                let card = (l(d.pane) - l(d.bg)).abs();
                assert!(card >= 4.0, "{}: Deeper card step {card:.1} L*", p.name);
            }
        }
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
                apply(&ctx, ThemeId::Midnight, look);
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

    /// Owner (6 Oct 2026): each theme is a design of its own, not a recolour. Every pair of themes
    /// differs in at least four structural choices, and no two share an art motif.
    #[test]
    fn every_theme_is_a_design_of_its_own() {
        use std::mem::discriminant as kind;
        let traits = |d: &Design| {
            [
                format!("{:.1}", (d.radius_scale * 4.0).round() / 4.0),
                format!("{:?}", d.edges),
                format!("{:?}", d.shadows),
                format!("{:?}", d.panes),
                format!("{:?}", kind(&d.ground)),
                format!("{:?}", kind(&d.selection)),
                format!("{:?}", kind(&d.focus)),
                format!("{:?}", d.hover),
                format!("{:?}", kind(&d.active_tab)),
                format!("{:?}{:?}", d.labels.case, d.labels.weight),
                format!("{:?}", d.heading),
                format!("{:?}{:?}", d.graph.node, d.graph.curve),
                format!("{:?}", d.icons),
                format!("{:?}", d.motion),
            ]
        };
        let mut motifs = std::collections::HashSet::new();
        for (i, a) in ThemeId::ALL.iter().enumerate() {
            assert!(
                motifs.insert(a.design().art),
                "{}: art motif shared",
                a.name()
            );
            for b in ThemeId::ALL.iter().skip(i + 1) {
                let (x, y) = (traits(a.design()), traits(b.design()));
                let differ = x.iter().zip(&y).filter(|(p, q)| p != q).count();
                assert!(
                    differ >= 4,
                    "{} and {} differ in only {differ} structural choices",
                    a.name(),
                    b.name()
                );
            }
        }
    }

    /// A theme's radius scale reshapes every corner but keeps pills full, square corners square
    /// and a rounded corner from collapsing; sharp themes stay sharper than soft ones.
    #[test]
    fn design_radii_scale_and_keep_their_shape() {
        for id in ThemeId::ALL {
            let d = id.design();
            assert!((0.3..=1.5).contains(&d.radius_scale), "{}", id.name());
            assert_eq!(d.radius(FULL), FULL);
            assert_eq!(d.radius(0), 0);
            let mut last = 0;
            for r in 1..=40 {
                let v = d.radius(r);
                assert!(v >= 1 && v >= last, "{}: radius {r} -> {v}", id.name());
                last = v;
            }
        }
        let r = |id: ThemeId| id.design().radius(COMFORTABLE.radius);
        for sharp in [ThemeId::Neon, ThemeId::Paper, ThemeId::Daylight] {
            for soft in [ThemeId::Ember, ThemeId::Forest, ThemeId::Aurora] {
                assert!(r(sharp) < r(soft), "{sharp:?} sharper than {soft:?}");
            }
        }
    }

    /// Lines, rings and markers the record sizes stay visible and never heavy.
    #[test]
    fn design_strokes_are_sized_sensibly() {
        for id in ThemeId::ALL {
            let d = id.design();
            let mut widths = vec![("edge", d.edge_width)];
            match d.selection {
                SelectionStyle::Bar { width } | SelectionStyle::Outline { width } => {
                    widths.push(("selection", width));
                }
                SelectionStyle::Wash | SelectionStyle::Glow => {}
            }
            match d.focus {
                FocusStyle::Ring { width, offset } => {
                    widths.push(("focus", width));
                    assert!((0.0..=2.0).contains(&offset), "{}", id.name());
                }
                FocusStyle::Glow { width } | FocusStyle::Double { width } => {
                    widths.push(("focus", width));
                }
            }
            match d.active_tab {
                TabStyle::Underline { width } | TabStyle::TopRule { width } => {
                    widths.push(("tab", width));
                }
                TabStyle::Lifted => {}
            }
            for (what, w) in widths {
                assert!((1.0..=6.0).contains(&w), "{}: {what} width {w}", id.name());
            }
            let g = d.graph;
            assert!((0.8..=1.4).contains(&g.node_scale), "{}", id.name());
            assert!((0.8..=1.4).contains(&g.line_scale), "{}", id.name());
            assert!((0.0..=0.2).contains(&d.labels.tracking), "{}", id.name());
            assert!((0.7..=1.3).contains(&d.motion.duration_scale()));
            assert!((1.0..=1.1).contains(&d.heading.size_scale()));
        }
        // Contrast keeps the strongest structure: outlines, a double ring, no shadows.
        let c = ThemeId::Contrast.design();
        assert_eq!(c.edges, EdgeMode::Outline);
        assert!(matches!(c.focus, FocusStyle::Double { .. }));
        assert_eq!(c.shadows, ShadowCharacter::Flat);
        assert_eq!(c.ground, Ground::Flat);
    }

    /// Ground treatments are washes laid over the ground: text, links, the focus ring and input
    /// outlines keep their targets on the strongest point of the wash, on both styles' grounds.
    #[test]
    fn ground_treatments_keep_text_readable() {
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            let wash = match id.design().ground {
                Ground::Flat => continue,
                Ground::Vertical { ink, top, bottom } => Palette::tint(ink, top.max(bottom)),
                Ground::Glow { ink, alpha, .. } | Ground::Vignette { ink, alpha } => {
                    Palette::tint(ink, alpha)
                }
            };
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, rim_min) = targets(p);
                for (on, g) in [("ground", s.bg), ("pane", s.pane)] {
                    let b = over(wash, g);
                    let on = format!("ground treatment over {on}");
                    gate.need("text", p.text, &on, b, text_min);
                    gate.need("text_dim", p.text_dim, &on, b, dim_min);
                    gate.need("text_faint", p.text_faint, &on, b, faint_min);
                    gate.need("accent", p.accent, &on, b, 4.5);
                    gate.need("focus", p.focus, &on, b, rim_min);
                    gate.need("control_border", p.control_border, &on, b, rim_min);
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The theme's glass tint keeps every pairing `glass_keeps_contrast_over_any_backdrop` checks;
    /// Contrast never tints (it never uses glass).
    #[test]
    fn glass_tint_keeps_contrast_over_any_backdrop() {
        let probes = [
            Color32::WHITE,
            Color32::BLACK,
            Color32::from_rgb(255, 0, 0),
            Color32::from_rgb(0, 255, 0),
            Color32::from_rgb(0, 0, 255),
            Color32::from_rgb(255, 255, 0),
            Color32::from_rgb(0, 255, 255),
            Color32::from_rgb(255, 0, 255),
            Color32::from_gray(128),
        ];
        let mut failures = Vec::new();
        for id in ThemeId::ALL {
            let p = id.palette();
            let tint = id.design().glass_tint;
            if p.glass_alpha == 0 {
                assert_eq!(tint, Color32::TRANSPARENT, "{}", p.name);
                continue;
            }
            assert!(
                tint.a() <= 40,
                "{}: the tint leans, it does not repaint",
                p.name
            );
            for style in Style::ALL {
                let s = p.surfaces(style);
                let fill = over(tint, s.overlay);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, _) = targets(p);
                for probe in probes {
                    let backdrop = clamp_backdrop(probe, p.dark, p.glass_luminance);
                    let glass = over(Palette::tint(fill, p.glass_alpha), backdrop);
                    for bg in [glass, over(p.hover_strong(), glass)] {
                        gate.need("text", p.text, "tinted glass", bg, text_min);
                        gate.need("text_dim", p.text_dim, "tinted glass", bg, dim_min);
                        gate.need("text_faint", p.text_faint, "tinted glass", bg, faint_min);
                        gate.need("accent", p.accent, "tinted glass", bg, 4.5);
                        gate.need("danger", p.danger, "tinted glass", bg, 4.5);
                        gate.need("control_border", p.control_border, "tinted glass", bg, 3.0);
                        gate.need("focus", p.focus, "tinted glass", bg, 3.0);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Shadow characters: Soft keeps the tables, Crisp shortens, Flat keeps only a faint floating
    /// lift, Glow centres every layer.
    #[test]
    fn shadow_characters_reshape_the_layers() {
        use ShadowCharacter as C;
        let drop = ShadowLayer {
            dy: 10,
            blur: 28,
            spread: -8,
            alpha: 166,
            tint: ShadowTint::Shadow,
        };
        assert_eq!(C::Soft.shape(Elevation::Pane, drop), Some(drop));
        let crisp = C::Crisp.shape(Elevation::Floating, drop).unwrap();
        assert!(crisp.blur < drop.blur / 2 && crisp.dy < drop.dy && crisp.dy > 0);
        assert_eq!(C::Flat.shape(Elevation::Control, drop), None);
        assert_eq!(C::Flat.shape(Elevation::Pane, drop), None);
        let lift = C::Flat.shape(Elevation::Floating, drop).unwrap();
        assert!(lift.alpha < drop.alpha && lift.blur < drop.blur);
        let glow = C::Glow.shape(Elevation::Primary, drop).unwrap();
        assert_eq!(glow.dy, 0);
        assert!(glow.spread >= 0 && glow.blur > drop.blur);
    }

    /// Every material a theme hands the kit is already in its shadow character: Contrast's
    /// controls cast nothing, Neon's layers glow around their surface, Daylight's are short.
    #[test]
    fn materials_come_in_the_themes_shadow_character() {
        for style in [Style::Standard, Style::Deeper] {
            for level in Elevation::ALL {
                let contrast = ThemeId::Contrast.palette().material(style, level);
                if level != Elevation::Floating {
                    assert!(contrast.layers.is_empty(), "Contrast {style:?} {level:?}");
                }
                let neon = ThemeId::Neon.palette().material(style, level);
                assert!(
                    neon.layers.iter().all(|l| l.dy == 0),
                    "Neon {style:?} {level:?}"
                );
                let soft = material(style, false, level);
                let crisp = ThemeId::Daylight.palette().material(style, level);
                assert_eq!(crisp.layers.len(), soft.layers.len());
                for (c, s) in crisp.layers.iter().zip(soft.layers) {
                    assert!(c.blur <= s.blur.max(1), "Daylight {style:?} {level:?}");
                }
                assert_eq!(
                    ThemeId::Midnight.palette().material(style, level).layers,
                    material(style, true, level).layers
                );
            }
        }
    }

    /// Heading faces and label weights resolve to the families `ui/fonts.rs` registers, and label
    /// case is applied as the record says.
    #[test]
    fn type_tokens_resolve() {
        assert_eq!(HeadingFace::Ui.family(), semibold());
        assert_ne!(
            HeadingFace::Serif.family(),
            HeadingFace::Hyperlegible.family()
        );
        assert_eq!(Weight::Semibold.family(), semibold());
        assert_eq!(Weight::Medium.family(), medium());
        let upper = LabelStyle {
            case: LabelCase::Upper,
            tracking: 0.1,
            weight: Weight::Semibold,
        };
        assert_eq!(upper.cased("Remotes"), "REMOTES");
        let title = LabelStyle {
            case: LabelCase::Title,
            ..upper
        };
        assert_eq!(title.cased("Remotes"), "Remotes");
        let ctx = egui::Context::default();
        for id in ThemeId::ALL {
            apply(&ctx, id, Appearance::STILL);
            assert_eq!(design(&ctx), id.design());
        }
        assert_ne!(
            IconWeight::Regular,
            ThemeId::Contrast.design().icons,
            "Contrast's active icons are filled"
        );
    }

    /// The active metrics carry the active theme's corners: every radius through its scale,
    /// everything else (sizes, gutters, type) as the density and style set it.
    #[test]
    fn metrics_come_in_the_themes_corners() {
        for id in ThemeId::ALL {
            for density in Density::ALL {
                for style in [Style::Standard, Style::Deeper] {
                    let raw = metrics_for(density, style);
                    let m = metrics_in(id, density, style);
                    let d = id.design();
                    assert_eq!(m.radius, d.radius(raw.radius), "{id:?}");
                    assert_eq!(m.radius_sm, d.radius(raw.radius_sm));
                    assert_eq!(m.pane_radius, d.radius(raw.pane_radius));
                    assert_eq!(m.keycap_radius, d.radius(raw.keycap_radius));
                    assert_eq!(m.pill_radius, d.radius(raw.pill_radius));
                    assert_eq!(
                        (m.control, m.pane_gutter, m.body_size),
                        (raw.control, raw.pane_gutter, raw.body_size)
                    );
                }
            }
        }
        look_set(ThemeId::Neon, Style::Deeper, Density::Comfortable, false);
        assert_eq!(
            metrics().radius,
            ThemeId::Neon.design().radius(DEEPER_COMFORTABLE.radius)
        );
        assert!(
            metrics().radius < DEEPER_COMFORTABLE.radius,
            "Neon is sharper"
        );
        look_set(ThemeId::Ember, Style::Deeper, Density::Comfortable, false);
        assert!(
            metrics().radius > DEEPER_COMFORTABLE.radius,
            "Ember is softer"
        );
    }

    /// Section labels take the theme's case, size, weight and tracking; titles its heading face;
    /// active icons its icon weight.
    #[test]
    fn labels_titles_and_icons_follow_the_theme() {
        let job_of = |id: ThemeId| {
            look_set(id, Style::Standard, Density::Comfortable, false);
            section_label("Branches", Color32::WHITE)
        };
        let paper = job_of(ThemeId::Paper);
        assert_eq!(paper.text, "BRANCHES");
        let format = &paper.sections[0].format;
        assert_eq!(format.font_id.size, COMFORTABLE.micro_size);
        assert!((format.extra_letter_spacing - 0.11 * COMFORTABLE.micro_size).abs() < 1e-4);
        let daylight = job_of(ThemeId::Daylight);
        assert_eq!(daylight.text, "Branches");
        assert_eq!(
            daylight.sections[0].format.font_id,
            FontId::new(COMFORTABLE.small_size, semibold())
        );
        look_set(ThemeId::Paper, Style::Standard, Density::Comfortable, false);
        assert_eq!(
            heading(20.0),
            FontId::new(20.0 * 1.06, HeadingFace::Serif.family())
        );
        assert_eq!(icon_font(14.0, true), FontId::proportional(14.0));
        look_set(
            ThemeId::Contrast,
            Style::Standard,
            Density::Comfortable,
            false,
        );
        assert_eq!(heading(20.0).family, HeadingFace::Hyperlegible.family());
        assert_ne!(icon_font(14.0, true), FontId::proportional(14.0));
        assert_eq!(icon_font(14.0, false), FontId::proportional(14.0));
        look_set(
            ThemeId::Midnight,
            Style::Standard,
            Density::Comfortable,
            false,
        );
        assert_eq!(heading(20.0), FontId::new(20.0, semibold()));
    }

    /// The glass fill leans to the theme's light; Contrast has none.
    #[test]
    fn glass_leans_to_the_theme() {
        for id in ThemeId::ALL {
            let p = id.palette();
            let s = p.surfaces(Style::Standard);
            let glass = p.glass(s);
            assert_eq!(glass.a(), p.glass_alpha, "{id:?}");
            if id.design().glass_tint == Color32::TRANSPARENT {
                assert_eq!(glass, Palette::tint(s.overlay, p.glass_alpha));
            } else {
                assert_ne!(glass, Palette::tint(s.overlay, p.glass_alpha), "{id:?}");
            }
        }
    }
}
