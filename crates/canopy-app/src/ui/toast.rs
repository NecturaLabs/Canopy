//! Transient notifications in the bottom-right corner that slide in and fade out.

use egui::{Align2, CornerRadius, FontId, Id, Order, RichText, Stroke, vec2};
use egui_phosphor::regular as icon;

use super::theme::{self, Palette, palette};

/// Severity of a toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    kind: ToastKind,
    title: String,
    body: String,
    born: f64,
    ttl: f64,
}

/// The stack of live toasts.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    /// Queues a toast.
    pub fn push(
        &mut self,
        ctx: &egui::Context,
        kind: ToastKind,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        let ttl = match kind {
            ToastKind::Error => 9.0,
            _ => 4.0,
        };
        self.next += 1;
        let body: String = body.into();
        let body = if body.len() > 600 {
            format!("{}…", &body[..body.floor_char_boundary(600)])
        } else {
            body
        };
        self.items.push(Toast {
            id: self.next,
            kind,
            title: title.into(),
            body,
            born: ctx.input(|i| i.time),
            ttl,
        });
        if self.items.len() > 5 {
            self.items.remove(0);
        }
        ctx.request_repaint();
    }

    /// Draws the toasts and expires old ones.
    pub fn show(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.items.retain(|t| now - t.born < t.ttl);
        if self.items.is_empty() {
            return;
        }
        let p = palette(ctx);
        let screen = ctx.content_rect();
        let mut y = screen.max.y - 40.0;
        let mut dismiss = None;
        for t in self.items.iter().rev() {
            let age = now - t.born;
            let enter = ((age / 0.25).min(1.0)) as f32;
            let leave = (((t.ttl - age) / 0.35).clamp(0.0, 1.0)) as f32;
            let ease = 1.0 - (1.0 - enter).powi(3);
            let alpha = ease.min(leave);
            let (glyph, color) = match t.kind {
                ToastKind::Info => (icon::INFO, p.accent2),
                ToastKind::Success => (icon::CHECK_CIRCLE, p.success),
                ToastKind::Error => (icon::WARNING_CIRCLE, p.danger),
            };
            let x = screen.max.x - 20.0 + (1.0 - ease) * 60.0;
            let resp = egui::Area::new(Id::new(("toast", t.id)))
                .order(Order::Tooltip)
                .pivot(Align2::RIGHT_BOTTOM)
                .fixed_pos(egui::pos2(x, y))
                .interactable(true)
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    egui::Frame::new()
                        .fill(p.elevated)
                        .stroke(Stroke::new(1.0, Palette::tint(color, 120)))
                        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                        .shadow(ctx.global_style().visuals.popup_shadow)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_max_width(380.0);
                            ui.horizontal_top(|ui| {
                                ui.label(RichText::new(glyph).size(18.0).color(color));
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(&t.title)
                                            .font(FontId::new(13.5, theme::semibold()))
                                            .color(p.text),
                                    );
                                    if !t.body.is_empty() {
                                        ui.label(
                                            RichText::new(&t.body).size(12.0).color(p.text_dim),
                                        );
                                    }
                                });
                            });
                        });
                })
                .response;
            if resp.clicked() {
                dismiss = Some(t.id);
            }
            y -= resp.rect.height() + 10.0;
        }
        if let Some(id) = dismiss {
            self.items.retain(|t| t.id != id);
        }
        let _ = vec2(0.0, 0.0);
        ctx.request_repaint();
    }
}
