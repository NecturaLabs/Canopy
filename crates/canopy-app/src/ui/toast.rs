//! Bottom-right notifications, with a bounded scroll stack when several need attention: the queue,
//! its timers and recovery actions. Each card is a `kit::toast::ToastCard`.

#[cfg(test)]
use egui::CornerRadius;
use egui::{Align2, Id, Order, vec2};

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::kit::Tone;
use super::kit::toast::ToastCard;
use super::motion;
use super::theme;

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
    repo: Option<&Path>,
) -> Option<(&'static str, Option<Recovery>)> {
    let lower = body.to_lowercase();
    if title.to_lowercase().contains("push")
        && (lower.contains("fetch first") || lower.contains("non-fast-forward"))
    {
        Some((
            "The remote has commits you do not have yet. Pull and resolve any conflicts before pushing again.",
            repo.map(|r| Recovery::Pull(r.to_path_buf())),
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
        repo: Option<&Path>,
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
            error_help(&title, &body, repo)
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
        let m = theme::metrics();
        let screen = ctx.content_rect();
        let mut dismiss = None;
        // Bottom-right, 16 from the window's right edge and above the status bar.
        let corner = screen.right_bottom() - vec2(theme::LG, m.status_bar_height + theme::LG);
        egui::Area::new(Id::new("notifications"))
            .order(Order::Tooltip)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(corner)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_width(
                    m.toast_width
                        .min((screen.width() - 2.0 * theme::XL).max(0.0)),
                );
                // An Area begins with its previous frame's size. Do not feed that shrunken
                // height back into the scroll viewport: arrivals and disclosures must grow.
                let max_height = (screen.height() - m.status_bar_height - 2.0 * theme::LG).max(0.0);
                ui.set_max_height(max_height);
                ui.spacing_mut().item_spacing.y = theme::SM;
                ui.spacing_mut().scroll = theme::solid_scroll();
                egui::ScrollArea::vertical()
                    .id_salt("notification-stack")
                    .auto_shrink([false, true])
                    .min_scrolled_height(0.0)
                    .max_height(max_height)
                    .show(ui, |ui| {
                        for (index, t) in self.items.iter().enumerate().rev() {
                            // Toasts that arrived together enter one after another.
                            let stagger = self.items[..index]
                                .iter()
                                .rev()
                                .take_while(|o| t.born - o.born < f64::from(motion::TOAST_STAGGER))
                                .count();
                            let tone = match t.kind {
                                ToastKind::Info => Tone::Accent,
                                ToastKind::Success => Tone::Success,
                                ToastKind::Error => Tone::Danger,
                            };
                            let mut card =
                                ToastCard::new(Id::new(("notification", t.id)), tone, &t.title)
                                    .born(t.born)
                                    .stagger(u8::try_from(stagger).unwrap_or(u8::MAX))
                                    .clickable(t.kind != ToastKind::Error);
                            if t.kind == ToastKind::Error {
                                if let Some(help) = t.help {
                                    card = card.body(help).body_strong(true);
                                }
                                if let Some(recovery) = &t.recovery {
                                    let label = match recovery {
                                        Recovery::Pull(_) => "Pull",
                                        Recovery::Accounts => "Open Accounts",
                                    };
                                    card = card.action(label, None);
                                }
                                card = card.details(&t.body);
                            } else if !t.body.is_empty() {
                                card = card.body(&t.body);
                            }
                            let out = ui.push_id(t.id, |ui| card.show(ui)).inner;
                            if out.dismissed || out.clicked {
                                dismiss = Some(t.id);
                            }
                            if out.action.is_some()
                                && let Some(recovery) = &t.recovery
                            {
                                actions.push(recovery.clone());
                                dismiss = Some(t.id);
                            }
                        }
                    });
            });
        if let Some(id) = dismiss {
            self.items.retain(|t| t.id != id);
        }
        // Wake when the next toast expires; nothing repaints in between.
        if let Some(next) = self
            .items
            .iter()
            .map(|t| t.born + t.ttl - now)
            .filter(|left| left.is_finite())
            .reduce(f64::min)
        {
            ctx.request_repaint_after(Duration::from_secs_f64(next.max(0.0)));
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_error_notifications_fit_and_remain_reachable_at_max_zoom() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut toasts = Toasts::default();
        for index in 1..=5 {
            toasts.push(
                &ctx,
                ToastKind::Error,
                format!("Push failed {index}"),
                "fetch first",
            );
        }
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 375.0));
        let mut seen = std::collections::HashSet::new();
        for frame in 0..60 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(400.0, 180.0))];
            if frame > 10 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -30.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(0.4 + f64::from(frame) * 0.01),
                    events,
                    ..Default::default()
                },
                |_| {
                    toasts.show(&ctx);
                },
            );
            output.textures_delta.clear();
            let bounds = ctx
                .memory(|memory| memory.area_rect(Id::new("notifications")))
                .expect("notification stack");
            assert!(screen.contains_rect(bounds), "{bounds:?}");
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().starts_with("Push failed ")
                    && shape
                        .clip_rect
                        .intersects(text.galley.rect.translate(text.pos.to_vec2()))
                {
                    seen.insert(text.galley.text().to_owned());
                }
            }
            let cards = visible_cards(&output);
            if frame >= 2 {
                assert!(!cards.is_empty(), "notifications must be painted");
            }
            assert!(
                cards.iter().all(|rect| screen.contains_rect(*rect)),
                "{cards:?}"
            );
            for (index, rectangle) in cards.iter().enumerate() {
                assert!(
                    cards[index + 1..]
                        .iter()
                        .all(|other| !rectangle.intersects(*other)),
                    "{cards:?}"
                );
            }
            output.drop_without_applying_deltas();
        }
        assert_eq!(toasts.items.len(), 5, "overflow must not drop errors");
        assert_eq!(
            seen.len(),
            5,
            "every notification must be reachable by scrolling: {seen:?}"
        );
    }

    fn visible_cards(output: &egui::FullOutput) -> Vec<egui::Rect> {
        fn collect(shape: &egui::Shape, clip: egui::Rect, cards: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, clip, cards);
                    }
                }
                egui::Shape::Rect(rect)
                    if rect.corner_radius == CornerRadius::same(theme::metrics().radius_lg)
                        && rect.stroke.width > 0.0 =>
                {
                    let visible = rect.rect.intersect(clip);
                    if visible.is_positive() {
                        cards.push(visible);
                    }
                }
                _ => {}
            }
        }
        let mut cards = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, shape.clip_rect, &mut cards);
        }
        cards
    }

    #[test]
    fn clicking_a_success_notification_dismisses_it() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Success, "Committed", "Finished safely");
        let mut pointer = egui::Pos2::ZERO;
        for frame in 0..6 {
            let mut events = vec![egui::Event::PointerMoved(pointer)];
            if matches!(frame, 3 | 4) {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 3,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(1024.0, 640.0),
                    )),
                    time: Some(0.4 + f64::from(frame) * 0.01),
                    events,
                    ..Default::default()
                },
                |_| {
                    toasts.show(&ctx);
                },
            );
            output.textures_delta.clear();
            if frame == 2 {
                pointer = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "Finished safely" => {
                            Some(text.visual_bounding_rect().center())
                        }
                        _ => None,
                    })
                    .expect("success message");
            }
        }
        assert!(
            toasts.items.is_empty(),
            "clicking a non-error card must dismiss it"
        );
    }

    #[test]
    fn expanded_notification_details_survive_a_new_arrival() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Error, "Original error", "Original output");
        let mut pointer = egui::Pos2::ZERO;
        let mut detail_visible = false;
        for frame in 0..9 {
            if frame == 6 {
                assert!(detail_visible, "details must open before testing arrival");
                toasts.push(&ctx, ToastKind::Error, "New error", "New output");
            }
            let mut events = vec![egui::Event::PointerMoved(pointer)];
            if matches!(frame, 3 | 4) {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 3,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(1024.0, 640.0),
                    )),
                    time: Some(0.4 + f64::from(frame) * 0.01),
                    events,
                    ..Default::default()
                },
                |_| {
                    toasts.show(&ctx);
                },
            );
            output.textures_delta.clear();
            detail_visible = output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Original output"));

            if frame == 2 {
                pointer = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "Details" => {
                            Some(text.visual_bounding_rect().center())
                        }
                        _ => None,
                    })
                    .expect("details disclosure");
            }
        }
        assert!(
            detail_visible,
            "a new card must not reset another card's disclosure"
        );
    }

    #[test]
    fn notification_stack_fits_the_viewport() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Error, "Push failed", "fetch first");
        toasts.push(&ctx, ToastKind::Success, "Committed", "Update README");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1024.0, 640.0));
        for frame in 0..20 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(0.4 + f64::from(frame) * 0.01),
                    ..Default::default()
                },
                |_| {
                    toasts.show(&ctx);
                },
            );
            output.textures_delta.clear();
            let bounds = ctx
                .memory(|memory| memory.area_rect(Id::new("notifications")))
                .expect("notification stack");
            assert!(screen.contains_rect(bounds), "frame {frame}: {bounds:?}");
            if frame >= 2 {
                let cards = visible_cards(&output);
                assert_eq!(cards.len(), 2);
                assert!(cards.iter().all(|rect| screen.contains_rect(*rect)));
                assert!(!cards[0].intersects(cards[1]), "notifications overlap");
            }
        }
    }
}
