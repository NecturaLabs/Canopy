//! Canopy's component library. See `docs/design.md`, "Component library".
//!
//! Every component is a struct built with `new(..)` (required inputs only), chained setters that
//! take and return `self`, and a terminal `show(self, ui) -> Response` (or [`KitResponse`] when it
//! returns a value too). Components also implement [`egui::Widget`], so `ui.add(Button::new(..))`
//! works. Common setters keep the same names everywhere: `size`, `icon`, `enabled`, `tooltip`,
//! `disabled_reason`, `shortcut`, `id_salt`, `full_width`, `min_width`.
//!
//! Components read their colours, geometry and options themselves ([`theme::palette`],
//! [`theme::surfaces`], [`theme::metrics`], [`theme::appearance`]); callers never pass colours or
//! sizes. Motion comes from [`anim`] and `ui::motion`, so reduced motion makes every transition
//! instant. Every interactive component reports its AccessKit role and label.
//!
//! The gallery (`--overlay gallery`) shows every component in every state: each module that has
//! components adds a `gallery(ui)` function and registers it in [`gallery::SECTIONS`].

pub mod anim;
pub mod button;
pub mod gallery;
pub mod glass;
pub mod paint;

// Filled by the component phase; each is declared now so its gallery section has a place.
pub mod avatar;
pub mod chip;
pub mod choice;
pub mod field;
pub mod graph;
pub mod menu;
pub mod overlay;
pub mod progress;
pub mod range;
pub mod row;
pub mod select;
pub mod split;
pub mod surface;
pub mod tabs;
pub mod toast;
pub mod toggle;

#[cfg(test)]
mod testkit;

pub use button::{Button, ButtonKind, IconButton, ToolButton};

use egui::{Color32, Id, Response};

use super::theme::{self, Palette};

/// Control size. `Md` is the default; `Lg` is for welcome calls to action only; `Xs` exists for
/// icon buttons inside other controls (a field's clear button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Size {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}

impl Size {
    pub const ALL: [Self; 4] = [Self::Xs, Self::Sm, Self::Md, Self::Lg];

    /// The control height of this size in the active metrics.
    pub fn height(self) -> f32 {
        let m = theme::metrics();
        match self {
            Self::Xs => 20.0_f32.min(m.control_sm),
            Self::Sm => m.control_sm,
            Self::Md => m.control,
            Self::Lg => m.control_lg,
        }
    }
}

/// The meaning a coloured component carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

impl Tone {
    pub const ALL: [Self; 5] = [
        Self::Neutral,
        Self::Accent,
        Self::Success,
        Self::Warning,
        Self::Danger,
    ];

    /// The text-grade colour of the tone.
    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Self::Neutral => p.text_dim,
            Self::Accent => p.accent,
            Self::Success => p.success,
            Self::Warning => p.warning,
            Self::Danger => p.danger,
        }
    }

    /// The tone's 16% tint (banners, callouts, dialog glyph discs).
    pub fn tint(self, p: &Palette) -> Color32 {
        match self {
            Self::Neutral => p.hover_strong(),
            Self::Accent => p.accent_tint(),
            Self::Success => p.success_tint(),
            Self::Warning => p.warning_tint(),
            Self::Danger => p.danger_tint(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Neutral => "Neutral",
            Self::Accent => "Accent",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::Danger => "Danger",
        }
    }
}

/// A component's response and the value it produced (a chosen option, an opened menu entry).
#[derive(Debug)]
pub struct KitResponse<T> {
    pub response: Response,
    pub value: T,
}

/// States forced on a component for the gallery and the kit's own tests, so a screenshot shows
/// hover, press, focus or an open popup without a pointer. Ordinary code never passes one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ForceState {
    pub hover: bool,
    pub pressed: bool,
    pub focus: bool,
    pub open: bool,
    /// A fixed transition progress (0 to 1) instead of the animated one.
    pub progress: Option<f32>,
}

impl ForceState {
    pub const HOVER: Self = Self {
        hover: true,
        pressed: false,
        focus: false,
        open: false,
        progress: None,
    };
    pub const PRESSED: Self = Self {
        hover: true,
        pressed: true,
        focus: false,
        open: false,
        progress: None,
    };
    pub const FOCUS: Self = Self {
        hover: false,
        pressed: false,
        focus: true,
        open: false,
        progress: None,
    };
    pub const OPEN: Self = Self {
        hover: false,
        pressed: false,
        focus: false,
        open: true,
        progress: None,
    };
}

/// Whether keyboard focus should be drawn: true after a key press, false again after the pointer
/// presses or moves (CSS `:focus-visible`). Text fields show focus whatever this says.
pub fn focus_visible(ctx: &egui::Context) -> bool {
    let id = Id::new("canopy-kit-focus-visible");
    let (key, pointer) = ctx.input(|i| {
        (
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        pressed: true,
                        key,
                        ..
                    } if matches!(key, egui::Key::Tab | egui::Key::ArrowUp | egui::Key::ArrowDown
                        | egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::Space
                        | egui::Key::Enter | egui::Key::Escape | egui::Key::Home | egui::Key::End)
                )
            }),
            i.pointer.any_pressed(),
        )
    });
    ctx.data_mut(|d| {
        let visible = d.get_temp_mut_or(id, false);
        if key {
            *visible = true;
        } else if pointer {
            *visible = false;
        }
        *visible
    })
}

/// What a component paints from: eased hover, press and focus, with the forced states applied.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Interact {
    /// Hover, eased in over `FAST` and out over `FAST_OUT` (0 to 1).
    pub hover: f32,
    /// Press, on at once and eased off over `FAST` (0 to 1).
    pub press: f32,
    /// Whether the press is held now (for the press scale).
    pub held: bool,
    /// Keyboard focus that should be drawn, eased in over `FAST`, out at once.
    pub focus: f32,
}

impl Interact {
    pub(crate) fn of(
        ctx: &egui::Context,
        response: &Response,
        enabled: bool,
        force: Option<ForceState>,
    ) -> Self {
        let f = force.unwrap_or_default();
        let id = response.id;
        let hovered = enabled && (f.hover || response.hovered());
        let held = enabled && (f.pressed || response.is_pointer_button_down_on());
        let focused = f.focus || (response.has_focus() && focus_visible(ctx));
        if force.is_some() {
            // A forced progress shows the transition part-way (the gallery's motion strip).
            let k = f.progress.unwrap_or(1.0);
            let on = |b: bool| f32::from(u8::from(b)) * k;
            return Self {
                hover: on(hovered),
                press: on(held),
                held,
                focus: on(focused),
            };
        }
        Self {
            hover: anim::ease_bool(
                ctx,
                id.with("kit-hover"),
                hovered,
                super::motion::FAST,
                super::motion::FAST_OUT,
            ),
            press: anim::ease_bool(ctx, id.with("kit-press"), held, 0.0, super::motion::FAST),
            held,
            focus: anim::ease_bool(ctx, id.with("kit-focus"), focused, super::motion::FAST, 0.0),
        }
    }
}
