//! Bottom-right notifications, with a bounded scroll stack when several need attention: the queue,
//! its timers and recovery actions. Each card is a `kit::toast::ToastCard`.
//!
//! A toast's timer runs only while the pointer is off the stack and the window has focus, so a
//! message being read, or arriving while the user is elsewhere, waits. When a card leaves, the
//! cards above it slide down into its place over `TOAST_SHIFT` instead of jumping.

#[cfg(test)]
use egui::CornerRadius;
use egui::emath::TSTransform;
use egui::{Align2, Id, Order, vec2};

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::kit::Tone;
use super::kit::overlay;
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

/// What a toast's card says under its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CardText<'a> {
    body: Option<&'a str>,
    /// The body in `text` rather than `text_dim` (a hint for an error).
    strong: bool,
    /// The output behind the card's Details disclosure.
    details: Option<&'a str>,
}

fn card_text(t: &Toast) -> CardText<'_> {
    if t.kind != ToastKind::Error {
        return CardText {
            body: Some(t.body.as_str()).filter(|b| !b.is_empty()),
            strong: false,
            details: None,
        };
    }
    if t.help.is_some() {
        return CardText {
            body: t.help,
            strong: true,
            details: Some(t.body.as_str()),
        };
    }
    // No hint: git's own first line says why, and Details holds the rest.
    let first = t.body.lines().map(str::trim).find(|l| !l.is_empty());
    let more = first.is_some_and(|f| t.body.trim() != f);
    CardText {
        body: first,
        strong: false,
        details: more.then_some(t.body.as_str()),
    }
}

/// Info and success toasts go after this many seconds of unpaused time; errors stay.
const TIMEOUT: f64 = 4.0;

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    kind: ToastKind,
    title: String,
    body: String,
    born: f64,
    /// Seconds of unpaused time left before it goes (infinite for errors).
    left: f64,
    help: Option<&'static str>,
    recovery: Option<Recovery>,
}

/// The stack of live toasts.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
    /// When the timers last ran.
    ticked: Option<f64>,
    /// Where each card sits, as its top's distance from the stack's bottom: the slide it is
    /// making towards its place.
    places: Vec<(u64, Slide)>,
}

/// A card's distance from the stack's bottom, easing from `from` to `to` since `start`.
#[derive(Debug, Clone, Copy)]
struct Slide {
    from: f32,
    to: f32,
    start: f64,
}

impl Slide {
    fn at(self, now: f64) -> f32 {
        let seconds = motion::time(motion::TOAST_SHIFT);
        if seconds <= 0.0 {
            return self.to;
        }
        let t = (super::to_f32(now - self.start) / seconds).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * motion::EASE_OUT.at(t)
    }

    fn moving(self, now: f64) -> bool {
        (self.from - self.to).abs() > 0.01
            && super::to_f32(now - self.start) < motion::time(motion::TOAST_SHIFT)
    }
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
        let left = if kind == ToastKind::Error {
            f64::INFINITY
        } else {
            TIMEOUT
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
            left,
            help,
            recovery,
        });
        if self.items.len() > 5 {
            // The oldest notice that is not an error goes first: an unread error stays.
            let oldest = self.items.iter().position(|t| t.kind != ToastKind::Error);
            let gone = self.items.remove(oldest.unwrap_or(0));
            if gone.kind == ToastKind::Error {
                tracing::warn!(title = %gone.title, "a sixth error pushed out the oldest");
            }
        }
        ctx.request_repaint();
    }

    /// Draws the toasts and expires old ones.
    pub fn show(&mut self, ctx: &egui::Context) -> Vec<Recovery> {
        let now = ctx.input(|i| i.time);
        // The timers wait while the pointer is on the stack or the window is in the background.
        let hovered = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|at| {
            ctx.memory(|m| m.area_rect(Id::new("notifications")))
                .is_some_and(|r| r.contains(at))
        });
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let paused = hovered || !focused;
        let elapsed = self.ticked.map_or(0.0, |t| (now - t).max(0.0));
        self.ticked = Some(now);
        if !paused {
            for t in &mut self.items {
                t.left -= elapsed;
            }
        }
        self.items.retain(|t| t.left > 0.0);
        if self.items.is_empty() {
            self.places.clear();
            return Vec::new();
        }
        let mut actions = Vec::new();
        let m = theme::metrics();
        let screen = ctx.content_rect();
        let mut dismiss = None;
        let mut cards = Vec::with_capacity(self.items.len());
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
                            let text = card_text(t);
                            if let Some(body) = text.body {
                                card = card.body(body).body_strong(text.strong);
                            }
                            if let Some(recovery) = &t.recovery {
                                let label = match recovery {
                                    Recovery::Pull(_) => "Pull",
                                    Recovery::Accounts => "Open Accounts",
                                };
                                card = card.action(label, None);
                            }
                            if let Some(details) = text.details {
                                card = card.details(details);
                            }
                            let layer = ui.layer_id();
                            let start = overlay::next_shape(ctx, layer);
                            let shown = ui.push_id(t.id, |ui| card.show(ui));
                            let out = shown.inner;
                            cards.push((
                                t.id,
                                layer,
                                start,
                                overlay::next_shape(ctx, layer),
                                shown.response.rect,
                            ));
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
        self.slide(ctx, now, &cards);
        if let Some(id) = dismiss {
            self.items.retain(|t| t.id != id);
        }
        // Wake when the next toast expires; nothing repaints in between, and nothing expires
        // while paused (leaving the stack or focusing the window repaints).
        if !paused
            && let Some(next) = self
                .items
                .iter()
                .map(|t| t.left)
                .filter(|left| left.is_finite())
                .reduce(f64::min)
        {
            ctx.request_repaint_after(Duration::from_secs_f64(next.max(0.0)));
        }
        actions
    }

    /// Moves each card from where it was towards where the stack now puts it: a card whose
    /// neighbour left (or arrived beneath it) slides into its new place instead of jumping.
    fn slide(
        &mut self,
        ctx: &egui::Context,
        now: f64,
        cards: &[(
            u64,
            egui::LayerId,
            egui::layers::ShapeIdx,
            egui::layers::ShapeIdx,
            egui::Rect,
        )],
    ) {
        let Some(bottom) = cards.iter().map(|c| c.4.bottom()).reduce(f32::max) else {
            return;
        };
        let mut places = Vec::with_capacity(cards.len());
        let mut moving = false;
        for &(id, layer, start, end, rect) in cards {
            let to = bottom - rect.top();
            let slide = match self.places.iter().find(|(i, _)| *i == id) {
                Some((_, s)) if (s.to - to).abs() > 0.5 => Slide {
                    from: s.at(now),
                    to,
                    start: now,
                },
                Some((_, s)) => Slide { to, ..*s },
                // A new card enters by itself (from the right); it does not slide.
                None => Slide {
                    from: to,
                    to,
                    start: now,
                },
            };
            let shown = slide.at(now);
            if (shown - to).abs() > 0.01 {
                // Further from the bottom than its place: drawn higher, by the difference.
                overlay::transform_range(
                    ctx,
                    layer,
                    start,
                    end,
                    TSTransform::from_translation(vec2(0.0, to - shown)),
                );
            }
            moving |= slide.moving(now);
            places.push((id, slide));
        }
        self.places = places;
        if moving {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REVIEW-KIT-5: a sixth notification pushes out the oldest one that is not an error; an
    /// unread error stays.
    #[test]
    fn a_sixth_notification_leaves_the_errors_alone() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Error, "Push failed", "fatal: refused");
        for i in 0..4 {
            toasts.push(&ctx, ToastKind::Success, format!("Done {i}"), "");
        }
        toasts.push(&ctx, ToastKind::Info, "Fetched", "");
        let titles: Vec<&str> = toasts.items.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Push failed", "Done 1", "Done 2", "Done 3", "Fetched"]
        );
    }

    /// REVIEW-KIT-7: an error Canopy has no hint for still shows git's own first line on the
    /// card, with the whole output behind Details when there is more.
    #[test]
    fn an_error_without_a_hint_shows_gits_first_line() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        let first =
            "error: Your local changes to the following files would be overwritten by checkout:";
        let stderr =
            format!("{first}\n\tsrc/main.rs\nPlease commit your changes or stash them.\nAborting");
        toasts.push(&ctx, ToastKind::Error, "Checkout failed", stderr.clone());
        let one = "fatal: couldn't find remote ref nope";
        toasts.push(&ctx, ToastKind::Error, "Fetch failed", one);
        let rejected = "! [rejected] main -> main (fetch first)";
        toasts.push(&ctx, ToastKind::Error, "Push failed", rejected);
        let text: Vec<CardText<'_>> = toasts.items.iter().map(card_text).collect();
        assert_eq!(
            text[0],
            CardText {
                body: Some(first),
                strong: false,
                details: Some(stderr.as_str()),
            }
        );
        assert_eq!(
            text[1],
            CardText {
                body: Some(one),
                strong: false,
                details: None,
            },
            "one line needs no Details"
        );
        assert_eq!(text[2].body, toasts.items[2].help, "a hint still leads");
        assert!(text[2].strong);
        assert_eq!(text[2].details, Some(rejected));
    }

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
        toasts.push(
            &ctx,
            ToastKind::Error,
            "Original error",
            "error: original\nOriginal output",
        );
        let mut pointer = egui::Pos2::ZERO;
        let mut detail_visible = false;
        for frame in 0..9 {
            if frame == 6 {
                assert!(detail_visible, "details must open before testing arrival");
                toasts.push(
                    &ctx,
                    ToastKind::Error,
                    "New error",
                    "error: new\nNew output",
                );
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
            detail_visible = output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "error: original\nOriginal output"));

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

    /// Runs one frame at `time` with the pointer at `pointer` and the window focused or not.
    fn frame_at(
        ctx: &egui::Context,
        toasts: &mut Toasts,
        time: f64,
        pointer: Option<egui::Pos2>,
        focused: bool,
    ) -> egui::FullOutput {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1024.0, 640.0),
            )),
            time: Some(time),
            events: pointer
                .map_or(egui::Event::PointerGone, egui::Event::PointerMoved)
                .into_iter_one(),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .focused = Some(focused);
        let mut output = ctx.run_ui(raw, |_| {
            toasts.show(ctx);
        });
        output.textures_delta.clear();
        output
    }

    trait IntoIterOne {
        fn into_iter_one(self) -> Vec<egui::Event>;
    }

    impl IntoIterOne for egui::Event {
        fn into_iter_one(self) -> Vec<egui::Event> {
            vec![self]
        }
    }

    #[test]
    fn timers_wait_while_the_stack_is_hovered_or_the_window_is_away() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut toasts = Toasts::default();
        let _ = frame_at(&ctx, &mut toasts, 0.0, None, true);
        toasts.push(&ctx, ToastKind::Success, "Pushed", "main to origin");
        let _ = frame_at(&ctx, &mut toasts, 0.1, None, true);
        let card = ctx
            .memory(|m| m.area_rect(Id::new("notifications")))
            .expect("stack");
        // Ten seconds with the pointer on the card: still there.
        for n in 1..=100 {
            let _ = frame_at(
                &ctx,
                &mut toasts,
                0.1 + f64::from(n) * 0.1,
                Some(card.center()),
                true,
            );
        }
        assert_eq!(toasts.items.len(), 1, "hovering keeps it");
        // Ten more seconds in the background: still there.
        for n in 1..=100 {
            let _ = frame_at(&ctx, &mut toasts, 10.1 + f64::from(n) * 0.1, None, false);
        }
        assert_eq!(toasts.items.len(), 1, "an unfocused window keeps it");
        // Its four seconds run only now.
        let _ = frame_at(&ctx, &mut toasts, 20.2, None, true);
        let _ = frame_at(&ctx, &mut toasts, 23.9, None, true);
        assert_eq!(toasts.items.len(), 1, "not before its time");
        let _ = frame_at(&ctx, &mut toasts, 24.4, None, true);
        assert!(toasts.items.is_empty(), "gone after four unpaused seconds");
    }

    #[test]
    fn cards_above_a_dismissed_one_slide_into_its_place() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        motion::set_reduced(false);
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Error, "Older", "first");
        toasts.push(&ctx, ToastKind::Error, "Newer", "second");
        let top_of = |out: &egui::FullOutput| {
            out.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "Newer" => Some(t.pos.y),
                _ => None,
            })
        };
        let mut before = None;
        for n in 0..40 {
            let out = frame_at(&ctx, &mut toasts, 1.0 + f64::from(n) * 0.05, None, true);
            before = top_of(&out).or(before);
        }
        let before = before.expect("newer card");
        // The older card (below) goes; the newer one moves down to the bottom, gradually.
        let older = toasts
            .items
            .iter()
            .find(|t| t.title == "Older")
            .map(|t| t.id);
        toasts.items.retain(|t| Some(t.id) != older);
        let mut path = Vec::new();
        for n in 0..12 {
            let out = frame_at(&ctx, &mut toasts, 3.0 + f64::from(n) * 0.04, None, true);
            path.push(top_of(&out).expect("newer card"));
        }
        let last = *path.last().expect("frames");
        assert!(last > before + 20.0, "it moved down: {before} -> {path:?}");
        assert!(
            path.iter().any(|y| *y > before + 1.0 && *y < last - 1.0),
            "it slid rather than jumped: {before} -> {path:?}"
        );
    }
}
