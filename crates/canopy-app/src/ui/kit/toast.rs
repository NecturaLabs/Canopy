//! The toast card (concept `.k-toast`, design spec 4.15): the overlay surface `toast_width` wide,
//! a leading icon in the kind's colour, a semibold title, a dim body, optional actions, a progress
//! bar for running work, and the dismiss button. The queue, its
//! timers and recovery actions stay in `ui/toast.rs`, which lays the cards out as a stack.
//!
//! Cards slide in from the right while they grow and fade in (the second of two arriving together
//! a little later) and slide out to the right as they fade when they leave. Reduced motion shows
//! and hides them at once.

use egui::{Align, FontId, Id, Layout, Response, RichText, Sense, Ui, UiBuilder, Vec2, vec2};
use egui_phosphor::regular as icon;

use super::overlay::{self, Entrance, ExitMotion, Layer};
use super::progress::Progress;
use super::{Button, ForceState, IconButton, Size, Tone};
use crate::ui::motion;
use crate::ui::theme;

/// Gap between the icon, the text column and the dismiss button.
const COLUMN_GAP: f32 = 10.0;
/// Room under the title, and above the actions.
const TITLE_GAP: f32 = 3.0;
const ACTIONS_GAP: f32 = 8.0;
/// Body line height, as a multiple of its size.
const BODY_LEADING: f32 = 1.45;
/// Toasts enter from this far right at this scale and leave this far right.
const ENTER_SLIDE: f32 = 28.0;
const ENTER_SCALE: f32 = 0.98;
const EXIT_SLIDE: f32 = 24.0;

/// When the dismiss button shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dismiss {
    /// Errors: always.
    Always,
    /// Everything else: while the card is hovered or the button has focus.
    OnHover,
}

/// What a card's user did this frame.
#[derive(Debug, Default)]
pub struct ToastOutput {
    /// The card itself was clicked (only when it is [`ToastCard::clickable`]).
    pub clicked: bool,
    /// The dismiss button.
    pub dismissed: bool,
    /// The index of the action button pressed.
    pub action: Option<usize>,
}

/// One toast card. `ToastCard::new(id, Tone::Success, "Pushed main").body("…").show(ui)`.
#[must_use = "call .show(ui)"]
pub struct ToastCard<'a> {
    id: Id,
    tone: Tone,
    title: &'a str,
    body: Option<&'a str>,
    body_strong: bool,
    actions: Vec<(&'a str, Option<&'a str>)>,
    #[expect(
        clippy::option_option,
        reason = "no bar, an indeterminate bar or a fraction, as `progress` takes them"
    )]
    progress: Option<Option<f32>>,
    dismiss: Dismiss,
    clickable: bool,
    born: Option<f64>,
    stagger: u8,
    force: Option<ForceState>,
}

impl<'a> ToastCard<'a> {
    /// A card for `id` (stable across frames: it keeps the disclosure and the entrance) in the
    /// colour of `tone` (`Accent` informs, `Success`, `Warning`, `Danger` for errors).
    pub fn new(id: Id, tone: Tone, title: &'a str) -> Self {
        Self {
            id,
            tone,
            title,
            body: None,
            body_strong: false,
            actions: Vec::new(),
            progress: None,
            dismiss: if tone == Tone::Danger {
                Dismiss::Always
            } else {
                Dismiss::OnHover
            },
            clickable: false,
            born: None,
            stagger: 0,
            force: None,
        }
    }
    /// The message under the title, dim.
    pub fn body(mut self, text: &'a str) -> Self {
        self.body = Some(text);
        self
    }
    /// The body in `text` rather than dim (an error's advice).
    pub fn body_strong(mut self, strong: bool) -> Self {
        self.body_strong = strong;
        self
    }
    /// A ghost action button under the body, with an optional leading glyph.
    pub fn action(mut self, label: &'a str, glyph: Option<&'a str>) -> Self {
        self.actions.push((label, glyph));
        self
    }
    /// A progress bar under the body: `Some(f)` determinate, `None` indeterminate.
    pub fn progress(mut self, fraction: Option<f32>) -> Self {
        self.progress = Some(fraction);
        self
    }
    /// A click anywhere on the card reports `clicked` (non-error toasts dismiss that way).
    pub fn clickable(mut self, clickable: bool) -> Self {
        self.clickable = clickable;
        self
    }
    /// When the toast arrived (`ctx.input(|i| i.time)`), for its entrance.
    pub fn born(mut self, time: f64) -> Self {
        self.born = Some(time);
        self
    }
    /// Its place among toasts that arrived together: each enters `TOAST_STAGGER` after the last.
    pub fn stagger(mut self, index: u8) -> Self {
        self.stagger = index;
        self
    }
    /// Shows the card with forced states (`HOVER` shows the dismiss button, `progress` fixes the
    /// entrance; the gallery and tests only).
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> ToastOutput {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> ToastOutput {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        let id = self.id;
        let mut out = ToastOutput::default();
        let (glyph, color) = match self.tone {
            Tone::Success => (icon::CHECK_CIRCLE, p.success),
            Tone::Warning => (icon::WARNING, p.warning),
            Tone::Danger => (icon::WARNING_CIRCLE, p.danger),
            Tone::Accent | Tone::Neutral => (icon::INFO, p.accent),
        };
        let force = self.force.unwrap_or_default();
        let hover_key = id.with("kit-toast-hovered");
        let hovered_before =
            force.hover || ctx.data(|d| d.get_temp::<bool>(hover_key).unwrap_or(false));

        // Entrance: from the right, growing and fading in, `TOAST_STAGGER` after the one before.
        let delay = motion::time(motion::TOAST_STAGGER) * f32::from(self.stagger);
        let progress = force.progress.or_else(|| {
            let born = self.born?;
            let seconds = motion::time(motion::TOAST_IN);
            if seconds <= 0.0 {
                return None;
            }
            let age = crate::ui::to_f32(ctx.input(|i| i.time) - born) - delay;
            if age < seconds {
                ctx.request_repaint();
            }
            Some((age / seconds).clamp(0.0, 1.0))
        });
        let e = progress.map_or(Entrance::DONE, |t| Entrance::at(t, t));
        let layer = ui.layer_id();
        let start = overlay::next_shape(&ctx, layer);
        let width = ui.available_width();
        let pad = m.toast_pad;
        // The card senses clicks under its children (registered first), so its buttons win.
        let sense = if self.clickable {
            Sense::click()
        } else {
            Sense::hover()
        };
        let card = ui.scope_builder(UiBuilder::new().id_salt(id).sense(sense), |ui| {
            ui.multiply_opacity(e.fade);
            overlay::floating(ui, Layer::Toast, pad, |ui| {
                ui.set_width(width - f32::from(pad.left + pad.right));
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                    let (slot, _) =
                        ui.allocate_exact_size(Vec2::splat(m.icon_toast), Sense::hover());
                    ui.painter().text(
                        slot.center(),
                        egui::Align2::CENTER_CENTER,
                        glyph,
                        FontId::proportional(m.icon_toast),
                        color,
                    );
                    let close_w = Size::Xs.height();
                    let text_w = (ui.available_width() - close_w - COLUMN_GAP).max(0.0);
                    ui.allocate_ui_with_layout(
                        vec2(text_w, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(text_w);
                            ui.spacing_mut().item_spacing.y = TITLE_GAP;
                            ui.add(
                                egui::Label::new(
                                    RichText::new(self.title)
                                        .font(FontId::new(m.body_size, theme::semibold()))
                                        .color(p.text),
                                )
                                .truncate(),
                            );
                            if let Some(b) = self.body {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(b)
                                            .size(m.meta_size)
                                            .line_height(Some((m.meta_size * BODY_LEADING).round()))
                                            .color(if self.body_strong {
                                                p.text
                                            } else {
                                                p.text_dim
                                            }),
                                    )
                                    .wrap(),
                                );
                            }
                            if let Some(f) = self.progress {
                                ui.add_space(ACTIONS_GAP - TITLE_GAP);
                                let bar = match f {
                                    Some(f) => Progress::new(f),
                                    None => Progress::indeterminate(),
                                };
                                bar.width(text_w).id_salt(id.with("progress")).show(ui);
                            }
                            if !self.actions.is_empty() {
                                ui.add_space(ACTIONS_GAP - TITLE_GAP);
                                // Ghost buttons pulled left so their labels line up with the text.
                                let mut row = ui.available_rect_before_wrap();
                                row.min.x -= m.button_pad_x_sm - 2.0;
                                row.set_height(Size::Sm.height());
                                ui.scope_builder(
                                    UiBuilder::new()
                                        .max_rect(row)
                                        .layout(Layout::left_to_right(Align::Center)),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        for (i, (label, glyph)) in self.actions.iter().enumerate() {
                                            let mut b = Button::new(*label)
                                                .ghost()
                                                .size(Size::Sm)
                                                .id_salt(id.with(("action", i)));
                                            if let Some(g) = glyph {
                                                b = b.icon(g);
                                            }
                                            if b.show(ui).clicked() {
                                                out.action = Some(i);
                                            }
                                        }
                                    },
                                );
                            }
                        },
                    );
                    let show_close = self.dismiss == Dismiss::Always || hovered_before;
                    // Hidden until hovered, but always there for the keyboard (and shown while
                    // it has focus).
                    let focus_key = id.with("kit-toast-dismiss-focus");
                    let focused = ctx.data(|d| d.get_temp::<bool>(focus_key).unwrap_or(false));
                    ui.scope(|ui| {
                        if !(show_close || focused) {
                            ui.multiply_opacity(0.0);
                        }
                        let r = IconButton::new(icon::X, "Dismiss notification")
                            .size(Size::Xs)
                            .id_salt(id.with("dismiss"))
                            .show(ui);
                        out.dismissed = r.clicked();
                        let has_focus = r.has_focus();
                        ctx.data_mut(|d| d.insert_temp(focus_key, has_focus));
                    });
                });
            })
        });
        let response: Response = card.response;
        let rect = response.rect;
        out.clicked = self.clickable && response.clicked();
        let hovered = ui.rect_contains_pointer(rect);
        ctx.data_mut(|d| d.insert_temp(hover_key, hovered));
        let origin = rect.right_center();
        overlay::transform_since(
            &ctx,
            layer,
            start,
            e.transform(origin, ENTER_SCALE, vec2(ENTER_SLIDE, 0.0)),
        );
        if self.force.is_none() {
            let end = overlay::next_shape(&ctx, layer);
            overlay::record_exit(
                &ctx,
                id,
                layer,
                0..0,
                start.0..end.0,
                ExitMotion {
                    seconds: motion::EXIT_TOAST,
                    scale: 1.0,
                    origin,
                    offset: vec2(EXIT_SLIDE, 0.0),
                    instant_on_input: false,
                },
            );
        }
        // A live region: errors interrupt, the rest wait their turn. The announcement reads the
        // title and the body (git's message, or the hint for it) as the description.
        let title = self.title.to_owned();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, &title));
        let tone = self.tone;
        let body = self.body;
        ctx.accesskit_node_builder(response.id, |n| {
            use egui::accesskit::{Live, Role};
            if let Some(b) = body {
                n.set_description(b);
            }
            if tone == Tone::Danger {
                n.set_role(Role::Alert);
                n.set_live(Live::Assertive);
            } else {
                n.set_role(Role::Status);
                n.set_live(Live::Polite);
            }
        });
        out
    }
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's toasts: info, success, warning, an error with its details open and an action,
/// a running operation, and one mid-entrance.
pub(crate) fn gallery(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    ui.label(
        RichText::new("Toasts: info, success (dismiss on hover), warning, error with details, progress, entering")
            .size(m.small_size)
            .color(p.text_faint),
    );
    let width = m.toast_width.min(ui.available_width());
    let id = ui.id().with("gallery-toasts");
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = theme::SM;
        let still = ForceState::default();
        ToastCard::new(id.with(0), Tone::Accent, "Update available")
            .body("Build 214 is ready to install.")
            .action("Update", Some(icon::DOWNLOAD_SIMPLE))
            .show_forced(ui, still);
        ToastCard::new(id.with(1), Tone::Success, "Pushed main to origin")
            .body("2 commits, 38b14e7 is now on origin/main.")
            .show_forced(ui, ForceState::HOVER);
        ToastCard::new(id.with(2), Tone::Warning, "Fetch skipped a remote")
            .body("upstream could not be reached; origin was fetched.")
            .show_forced(ui, still);
        ToastCard::new(id.with(3), Tone::Danger, "Push failed")
            .body("The remote has commits you do not have yet. Pull, resolve any conflicts, then push again.")
            .action("Pull", Some(icon::ARROW_DOWN))
            .show_forced(ui, ForceState::OPEN);
        ToastCard::new(id.with(4), Tone::Accent, "Fetching origin")
            .body("Receiving objects")
            .progress(Some(0.62))
            .show_forced(ui, still);
        ToastCard::new(id.with(5), Tone::Success, "Committed")
            .body("Entering, half way")
            .show_forced(
                ui,
                ForceState {
                    progress: Some(0.5),
                    ..ForceState::default()
                },
            );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Rect;

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn run(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1024.0, 640.0))),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    #[test]
    fn cards_announce_errors_as_alerts_and_the_rest_as_status() {
        use egui::accesskit::{Live, Role};

        let ctx = context();
        ctx.enable_accesskit();
        let out = run(&ctx, Vec::new(), |ui| {
            ui.set_width(380.0);
            ToastCard::new(Id::new(1), Tone::Danger, "Push failed").show(ui);
            ToastCard::new(Id::new(2), Tone::Success, "Pushed").show(ui);
        });
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        let role = |label: &str| {
            nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .map(|(_, n)| (n.role(), n.live()))
        };
        assert_eq!(
            role("Push failed"),
            Some((Role::Alert, Some(Live::Assertive)))
        );
        assert_eq!(role("Pushed"), Some((Role::Status, Some(Live::Polite))));
    }

    /// A11Y-14: an error's alert speaks git's message with its title, not the title alone.
    #[test]
    fn an_error_alert_speaks_its_message() {
        use egui::accesskit::Role;

        let ctx = context();
        ctx.enable_accesskit();
        let out = run(&ctx, Vec::new(), |ui| {
            ui.set_width(380.0);
            ToastCard::new(Id::new(1), Tone::Danger, "Push failed")
                .body("fatal: unable to access the remote")
                .show(ui);
        });
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        let (_, alert) = nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Alert)
            .expect("the alert");
        assert_eq!(alert.label(), Some("Push failed"));
        assert_eq!(
            alert.description(),
            Some("fatal: unable to access the remote"),
            "the message is the alert's description"
        );
    }

    #[test]
    fn toasts_enter_at_once_with_reduced_motion() {
        let ctx = context();
        let out = run(&ctx, Vec::new(), |ui| {
            ToastCard::new(Id::new(1), Tone::Success, "Pushed")
                .born(ui.ctx().input(|i| i.time))
                .show(ui);
        });
        let s = theme::surfaces(&ctx);
        let mut opaque = false;
        for c in &out.shapes {
            if let egui::Shape::Vec(v) = &c.shape {
                opaque |= v
                    .iter()
                    .any(|s2| matches!(s2, egui::Shape::Rect(r) if r.fill == s.overlay));
            }
        }
        assert!(opaque, "fully shown on the first frame");
    }

    #[test]
    fn the_dismiss_button_and_actions_report_what_was_pressed() {
        let ctx = context();
        let mut close = Rect::NOTHING;
        let mut action = Rect::NOTHING;
        let mut got = ToastOutput::default();
        for frame in 0..6 {
            let at = if frame < 3 {
                close.center()
            } else {
                action.center()
            };
            let events = match frame {
                1 | 4 => vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                2 | 5 => vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }],
                _ => Vec::new(),
            };
            let out = run(&ctx, events, |ui| {
                ui.set_width(380.0);
                let o = ToastCard::new(Id::new("t"), Tone::Danger, "Push failed")
                    .body("Pull first.")
                    .action("Pull", None)
                    .show(ui);
                got.dismissed |= o.dismissed;
                if o.action.is_some() {
                    got.action = o.action;
                }
            });
            for c in &out.shapes {
                if let egui::Shape::Text(t) = &c.shape {
                    let r = t.visual_bounding_rect();
                    if t.galley.text() == "Pull" {
                        action = r;
                    }
                    if t.galley.text() == icon::X {
                        close = r;
                    }
                }
            }
        }
        assert!(got.dismissed, "dismiss");
        assert_eq!(got.action, Some(0), "action");
    }
}
