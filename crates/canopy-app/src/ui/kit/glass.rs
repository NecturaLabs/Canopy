//! Translucent surfaces: the safe facade over the backdrop blur (design spec 7).
//!
//! Floating layers (menus, select lists, popovers, toasts, dialog cards, palette, cheatsheet) ask
//! [`layer_fill`] for their fill. With the blur available and Translucent surfaces on, it adds the
//! blurred backdrop under the layer and returns the overlay at the theme's glass opacity; otherwise
//! it returns the opaque overlay, the fallback that keeps the same layout. The OpenGL renderer
//! (`glass/gl.rs`, the only `unsafe` code) is not built yet, so [`available`] is false and every
//! layer takes the opaque path.

use egui::{Color32, CornerRadius, Painter, Rect};

use crate::ui::theme::{self, Palette};

/// Whether this build and graphics backend can blur the backdrop.
pub fn available() -> bool {
    false
}

/// Adds the blurred backdrop under a floating layer at `rect`, if glass is on and available, and
/// returns whether it did (the caller then paints the translucent overlay).
pub fn backdrop(_painter: &Painter, _rect: Rect, _radius: CornerRadius) -> bool {
    theme::appearance().glass && available()
}

/// The fill a floating layer paints at `rect`: the overlay over the blurred backdrop when glass is
/// on, else the opaque overlay. Call after the layer's shadow and before its content.
pub fn layer_fill(painter: &Painter, rect: Rect, radius: CornerRadius) -> Color32 {
    let p = theme::palette(painter.ctx());
    let s = theme::surfaces(painter.ctx());
    if backdrop(painter, rect, radius) {
        Palette::tint(s.overlay, p.glass_alpha)
    } else {
        s.overlay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_the_blur_layers_are_opaque() {
        let ctx = egui::Context::default();
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            &theme::Appearance {
                glass: true,
                ..theme::Appearance::STILL
            },
        );
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let fill = layer_fill(ui.painter(), Rect::ZERO, CornerRadius::ZERO);
            assert_eq!(fill, theme::surfaces(ui.ctx()).overlay);
            assert_eq!(fill.a(), 255);
        });
        out.textures_delta.clear();
        theme::apply(&ctx, theme::ThemeId::Midnight, &theme::Appearance::STILL);
    }
}
