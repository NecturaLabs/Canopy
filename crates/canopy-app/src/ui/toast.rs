//! Transient notifications in the bottom-right corner that slide in and fade out.

use egui::{Align2, CornerRadius, FontId, Id, Order, RichText, Stroke, vec2};
use egui_phosphor::regular as icon;

use std::path::PathBuf;

use super::theme::{self, Palette, palette};
use super::widgets;

/// Severity of a toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub enum Recovery {
    Pull(PathBuf),
    Accounts,
}

fn error_help(
    title: &str,
    body: &str,
    repo: Option<&PathBuf>,
) -> Option<(&'static str, Option<Recovery>)> {
    let lower = body.to_lowercase();
    if title.to_lowercase().contains("push")
        && (lower.contains("fetch first") || lower.contains("non-fast-forward"))
    {
        Some((
            "The remote has commits you do not have yet. Pull and resolve any conflicts before pushing again.",
            repo.cloned().map(Recovery::Pull),
        ))
    } else if lower.contains("authentication failed")
        || lower.contains("http basic: access denied")
        || lower.contains("bad credentials")
    {
        Some((
            "The host did not accept your credentials. Check your account access and try again.",
            Some(Recovery::Accounts),
        ))
    } else if lower.contains("could not resolve host")
        || lower.contains("could not resolve hostname")
    {
        Some((
            "The host could not be reached. Check your connection and the remote address, then try again.",
            None,
        ))
    } else if lower.contains("permission denied (publickey)") {
        Some((
            "SSH did not accept your key. Check the key's access to this repository, then try again.",
            None,
        ))
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    kind: ToastKind,
    title: String,
    body: String,
    born: f64,
    ttl: f64,
    help: Option<&'static str>,
    recovery: Option<Recovery>,
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
        self.push_for(ctx, kind, title, body, None);
    }

    /// A repository notice keeps its recovery action attached to that repository.
    pub fn push_for(
        &mut self,
        ctx: &egui::Context,
        kind: ToastKind,
        title: impl Into<String>,
        body: impl Into<String>,
        repo: Option<PathBuf>,
    ) {
        let ttl = if kind == ToastKind::Error {
            f64::INFINITY
        } else {
            4.0
        };
        self.next += 1;
        let title = title.into();
        let body = body.into();
        let (help, recovery) = if kind == ToastKind::Error {
            error_help(&title, &body, repo.as_ref())
                .map_or((None, None), |(help, recovery)| (Some(help), recovery))
        } else {
            (None, None)
        };
        self.items.push(Toast {
            id: self.next,
            kind,
            title,
            body,
            born: ctx.input(|i| i.time),
            ttl,
            help,
            recovery,
        });
        if self.items.len() > 5 {
            self.items.remove(0);
        }
        ctx.request_repaint();
    }

    /// Draws the toasts and expires old ones.
    pub fn show(&mut self, ctx: &egui::Context) -> Vec<Recovery> {
        let now = ctx.input(|i| i.time);
        self.items.retain(|t| now - t.born < t.ttl);
        if self.items.is_empty() {
            return Vec::new();
        }
        let mut actions = Vec::new();
        let p = palette(ctx);
        let screen = ctx.content_rect();
        let mut y = screen.max.y - 40.0;
        let mut dismiss = None;
        for t in self.items.iter().rev() {
            let age = now - t.born;
            let enter_time = f64::from(theme::fade(ctx, 0.25));
            let leave_time = f64::from(theme::fade(ctx, 0.35));
            let enter = if enter_time > 0.0 {
                (age / enter_time).min(1.0) as f32
            } else {
                1.0
            };
            let leave = if leave_time > 0.0 {
                ((t.ttl - age) / leave_time).clamp(0.0, 1.0) as f32
            } else {
                1.0
            };
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
                            ui.set_width(
                                (380.0_f32).min((screen.width() - theme::XL * 3.0).max(0.0)),
                            );
                            ui.horizontal_top(|ui| {
                                ui.label(RichText::new(glyph).size(theme::ICON_LG).color(color));
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&t.title)
                                            .font(FontId::new(theme::LEAD_SIZE, theme::semibold()))
                                            .color(p.text),
                                    )
                                    .wrap(),
                                );
                                ui.allocate_ui_with_layout(
                                    vec2(ui.available_width(), 26.0),
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if widgets::icon_button(ui, icon::X, "Dismiss notification")
                                            .clicked()
                                        {
                                            dismiss = Some(t.id);
                                        }
                                    },
                                );
                            });
                            if t.kind == ToastKind::Error {
                                if let Some(help) = t.help {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(help)
                                                .size(theme::META_SIZE)
                                                .color(p.text),
                                        )
                                        .wrap(),
                                    );
                                }
                                if let Some(recovery) = &t.recovery {
                                    let label = match recovery {
                                        Recovery::Pull(_) => "Pull",
                                        Recovery::Accounts => "Open Accounts",
                                    };
                                    if widgets::compact_button(ui, label).clicked() {
                                        actions.push(recovery.clone());
                                        dismiss = Some(t.id);
                                    }
                                }
                                egui::CollapsingHeader::new("Details").id_salt(t.id).show(
                                    ui,
                                    |ui| {
                                        egui::ScrollArea::vertical().max_height(140.0).show(
                                            ui,
                                            |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(&t.body)
                                                            .font(FontId::monospace(
                                                                theme::MONO_SMALL_SIZE,
                                                            ))
                                                            .color(p.text_dim),
                                                    )
                                                    .wrap(),
                                                );
                                            },
                                        );
                                        if ui
                                            .button(format!("{}  Copy output", icon::COPY))
                                            .clicked()
                                        {
                                            ctx.copy_text(t.body.clone());
                                        }
                                    },
                                );
                            } else if !t.body.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&t.body)
                                            .size(theme::META_SIZE)
                                            .color(p.text_dim),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                })
                .response;
            if t.kind != ToastKind::Error && resp.clicked() {
                dismiss = Some(t.id);
            }
            y -= resp.rect.height() + 10.0;
        }
        if let Some(id) = dismiss {
            self.items.retain(|t| t.id != id);
        }
        let _ = vec2(0.0, 0.0);
        if self
            .items
            .iter()
            .any(|toast| toast.kind != ToastKind::Error || now - toast.born < 0.25)
        {
            ctx.request_repaint();
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_stack_fits_the_viewport() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Error, "Push failed", "fetch first");
        toasts.push(&ctx, ToastKind::Success, "Committed", "Update README");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1024.0, 640.0));
        for frame in 0..20 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(0.4 + frame as f64 * 0.01),
                    ..Default::default()
                },
                |_| {
                    toasts.show(&ctx);
                },
            );
            output.textures_delta.clear();
            let error = ctx
                .memory(|memory| memory.area_rect(Id::new(("toast", 1_u64))))
                .expect("error notice");
            let success = ctx
                .memory(|memory| memory.area_rect(Id::new(("toast", 2_u64))))
                .expect("success notice");
            assert!(
                screen.contains_rect(error),
                "frame {frame}: error notice {error:?}"
            );
            assert!(
                screen.contains_rect(success),
                "frame {frame}: success notice {success:?}"
            );
            assert!(
                !error.intersects(success),
                "frame {frame}: notifications overlap"
            );
        }
    }
}
