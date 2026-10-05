//! Helpers the kit's own tests share: a themed context, one frame with input events, keyboard and
//! pointer events, and the AccessKit nodes a frame produced.

use egui::accesskit::{Node, Role, Toggled};
use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Ui};

use crate::ui::theme::{self, Appearance, Density, Style, ThemeId};

/// A context with Canopy's fonts and `theme` in `style` and `density`, motion off.
pub fn ctx_with(theme_id: ThemeId, style: Style, density: Density) -> Context {
    let ctx = Context::default();
    crate::ui::fonts::install(&ctx);
    theme::apply(
        &ctx,
        theme_id,
        &Appearance {
            style,
            density,
            ..Appearance::STILL
        },
    );
    ctx
}

/// Midnight, Standard, Comfortable, motion off.
pub fn ctx() -> Context {
    ctx_with(ThemeId::Midnight, Style::Standard, Density::Comfortable)
}

/// Runs one frame with `events`; returns what it painted.
pub fn frame(
    ctx: &Context,
    events: Vec<Event>,
    mut f: impl FnMut(&mut Ui),
) -> Vec<egui::epaint::ClippedShape> {
    let mut out = ctx.run_ui(
        RawInput {
            events,
            ..Default::default()
        },
        |ui| f(ui),
    );
    out.textures_delta.clear();
    out.shapes
}

/// A key press and its release.
pub fn key(k: Key) -> Vec<Event> {
    key_with(k, Modifiers::NONE)
}

pub fn key_with(k: Key, modifiers: Modifiers) -> Vec<Event> {
    vec![
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
        Event::Key {
            key: k,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        },
    ]
}

/// The pointer moving to `pos` and pressing there.
pub fn press(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]
}

/// The pointer releasing at `pos`.
pub fn release(pos: Pos2) -> Vec<Event> {
    vec![Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]
}

/// A full click at `pos` over two frames of `f`; the second frame's result is returned.
pub fn click(ctx: &Context, pos: Pos2, mut f: impl FnMut(&mut Ui)) {
    frame(ctx, press(pos), &mut f);
    frame(ctx, release(pos), &mut f);
}

/// One AccessKit node as the tests read it.
#[derive(Debug, Clone)]
pub struct Spoken {
    pub role: Role,
    pub label: Option<String>,
    pub toggled: Option<Toggled>,
    pub selected: Option<bool>,
    pub value: Option<String>,
    pub numeric: Option<f64>,
    pub disabled: bool,
}

impl Spoken {
    fn of(n: &Node) -> Self {
        Self {
            role: n.role(),
            label: n.label().map(str::to_owned),
            toggled: n.toggled(),
            selected: n.is_selected(),
            value: n.value().map(str::to_owned),
            numeric: n.numeric_value(),
            disabled: n.is_disabled(),
        }
    }
}

/// The AccessKit nodes one frame of `f` produced.
pub fn spoken(ctx: &Context, mut f: impl FnMut(&mut Ui)) -> Vec<Spoken> {
    ctx.enable_accesskit();
    let mut out = ctx.run_ui(RawInput::default(), |ui| f(ui));
    out.textures_delta.clear();
    out.platform_output
        .accesskit_update
        .map(|u| u.nodes.iter().map(|(_, n)| Spoken::of(n)).collect())
        .unwrap_or_default()
}

/// The node labelled `label`, if one was spoken.
pub fn find<'a>(nodes: &'a [Spoken], label: &str) -> Option<&'a Spoken> {
    nodes.iter().find(|n| n.label.as_deref() == Some(label))
}
