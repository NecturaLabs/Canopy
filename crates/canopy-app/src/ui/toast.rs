//! Bottom-right notifications, with a bounded scroll stack when several need attention: the queue,
//! its timers and recovery actions. Each card is a `kit::toast::ToastCard`. Every notification also
//! goes to the notification center (the status bar's bell), which lists the recent ones whether
//! or not their [`Topic`] shows as a card.
//!
//! A toast's timer runs only while the pointer is off the stack and the window has focus, so a
//! message being read, or arriving while the user is elsewhere, waits. When a card leaves, the
//! cards above it slide down into its place over `TOAST_SHIFT` instead of jumping.

#[cfg(test)]
use egui::CornerRadius;
use egui::emath::TSTransform;
use egui::{Align2, Id, Order, vec2};

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use super::kit::Tone;
use super::kit::button::IconButton;
use super::kit::chip::Badge;
use super::kit::overlay::{self, Popover, TooltipExt};
use super::kit::toast::ToastCard;
use super::kit::{Button, KitResponse, Size};
use super::motion;
use super::theme;
use crate::settings::NotificationPrefs;

/// Severity of a toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

/// What a notification is about, which decides whether it shows as a card
/// ([`NotificationPrefs`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    /// Something failed, with git's, the host's or the system's own message.
    Error,
    /// Something waits on the user: ssh asking for a security key, an update to restart into.
    Attention,
    /// A new Canopy build, or the answer to checking for one.
    Update,
    /// Git work finished: a commit, push, pull, fetch or clone.
    Finished,
    /// A small confirmation: text copied, the theme switched, a setting saved.
    Confirmation,
}

impl Topic {
    pub const ALL: [Self; 5] = [
        Self::Error,
        Self::Attention,
        Self::Update,
        Self::Finished,
        Self::Confirmation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Errors",
            Self::Attention => "Needs your attention",
            Self::Update => "Updates",
            Self::Finished => "Finished operations",
            Self::Confirmation => "Confirmations",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::Error => {
                "An operation, the host or opening something failed, with the reason it gave."
            }
            Self::Attention => {
                "ssh waits for your security key, or an installed update waits for a restart."
            }
            Self::Update => "A new Canopy build is out, or the result of checking for one.",
            Self::Finished => "A commit, push, pull, fetch, clone or other Git work finished.",
            Self::Confirmation => {
                "Text copied, the theme switched, a folder trusted or an identity saved."
            }
        }
    }

    /// Whether a notification about it the user has not seen counts on the bell: the routine
    /// ones are listed but never ask for attention.
    fn counts(self) -> bool {
        matches!(self, Self::Error | Self::Attention | Self::Update)
    }
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

/// What a notification of `kind` with `body` (and, for an error, its `help`) says under its
/// title, on its card and in the center alike.
fn card_text<'a>(kind: ToastKind, body: &'a str, help: Option<&'static str>) -> CardText<'a> {
    if kind != ToastKind::Error {
        return CardText {
            body: Some(body).filter(|b| !b.is_empty()),
            strong: false,
            details: None,
        };
    }
    if help.is_some() {
        return CardText {
            body: help,
            strong: true,
            details: Some(body),
        };
    }
    // No hint: git's own first line says why, and Details holds the rest.
    let first = body.lines().map(str::trim).find(|l| !l.is_empty());
    let more = first.is_some_and(|f| body.trim() != f);
    CardText {
        body: first,
        strong: false,
        details: more.then_some(body),
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

/// How many notifications the notification center keeps.
const HISTORY: usize = 100;
/// The notification center's width and the height its list scrolls beyond.
const CENTER_WIDTH: f32 = 360.0;
const CENTER_LIST_HEIGHT: f32 = 380.0;
/// The lines of a notification's text the center shows; the rest is in its tooltip.
const CENTER_BODY_LINES: usize = 4;

/// A notification as the notification center lists it.
#[derive(Debug, Clone)]
struct Entry {
    id: u64,
    kind: ToastKind,
    title: String,
    body: String,
    /// When it arrived, in Unix seconds.
    at: i64,
    /// What an error means, before git's own words.
    help: Option<&'static str>,
    recovery: Option<Recovery>,
}

/// The stack of live toasts, and the notification center's history.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
    /// Which topics show as cards (kept current by the app every frame).
    prefs: NotificationPrefs,
    /// The recent notifications, newest first.
    history: VecDeque<Entry>,
    /// Notifications that count on the bell and were seen neither as a card nor in the center.
    unread: usize,
    /// Whether the notification center was open last frame (no cards show meanwhile).
    center_open: bool,
    /// Recovery actions chosen in the center, handed out by [`Toasts::show`].
    chosen: Vec<Recovery>,
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
    /// Which topics show as cards from now on.
    pub fn set_prefs(&mut self, prefs: NotificationPrefs) {
        self.prefs = prefs;
    }

    /// Notifies: into the center, and as a card when its topic shows as one.
    pub fn push(
        &mut self,
        ctx: &egui::Context,
        kind: ToastKind,
        topic: Topic,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.push_for(ctx, kind, topic, title, body, None);
    }

    /// A repository notice keeps its recovery action attached to that repository.
    pub fn push_for(
        &mut self,
        ctx: &egui::Context,
        kind: ToastKind,
        topic: Topic,
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
        self.history.push_front(Entry {
            id: self.next,
            kind,
            title: title.clone(),
            body: body.clone(),
            at: chrono::Utc::now().timestamp(),
            help,
            recovery: recovery.clone(),
        });
        self.history.truncate(HISTORY);
        let card = self.prefs.shows(topic) && !self.center_open;
        if !card {
            if topic.counts() && !self.center_open {
                self.unread += 1;
            }
            tracing::debug!(title = %title, ?topic, "notification kept for the center");
            ctx.request_repaint();
            return;
        }
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

    /// Draws the toasts and expires old ones; returns the recovery actions chosen on a card or in
    /// the notification center.
    pub fn show(&mut self, ctx: &egui::Context) -> Vec<Recovery> {
        let chosen = std::mem::take(&mut self.chosen);
        let mut actions = self.show_cards(ctx);
        actions.extend(chosen);
        actions
    }

    fn show_cards(&mut self, ctx: &egui::Context) -> Vec<Recovery> {
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
                            let text = card_text(t.kind, &t.body, t.help);
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

impl Toasts {
    /// The status bar's bell, with the count of unseen notifications that need the user, and the
    /// notification center it opens. Returns whether the user asked for the notification
    /// settings.
    pub fn bell(&mut self, ui: &mut Ui) -> KitResponse<bool> {
        let unread = self.unread;
        let (glyph, name) = match (self.prefs.cards, unread) {
            (false, 0) => (icon::BELL_SLASH, "Notifications (cards off)".to_owned()),
            (false, n) => (
                icon::BELL_SLASH,
                format!("Notifications (cards off), {n} unread"),
            ),
            (true, 0) => (icon::BELL, "Notifications".to_owned()),
            (true, n) => (icon::BELL, format!("Notifications, {n} unread")),
        };
        let response = IconButton::new(glyph, name)
            .size(Size::Xs)
            .active(self.center_open)
            .id_salt("notification-center")
            .show(ui);
        if unread > 0 {
            let errors = self
                .history
                .iter()
                .take(unread)
                .any(|e| e.kind == ToastKind::Error);
            let badge = Badge::new(super::kit::chip::count_text(unread))
                .tone(if errors { Tone::Danger } else { Tone::Accent })
                .strong();
            let size = badge.measure(ui);
            let at = response.rect.right_top() + vec2(-size.x * 0.4, -size.y * 0.3);
            badge.paint(ui, egui::Rect::from_min_size(at, size));
        }
        let mut settings = false;
        let popover = Popover::new("notification-center", &response).width(CENTER_WIDTH);
        let popover_id = popover.id();
        let chosen = self.chosen.len();
        let shown = popover.show(|ui| settings = self.center(ui));
        // What these open (Settings, a pull) takes over; the center would linger behind it.
        let leave = settings || self.chosen.len() > chosen;
        if leave {
            egui::Popup::close_id(ui.ctx(), popover_id);
        }
        self.center_open = shown.is_some() && !leave;
        if self.center_open {
            self.unread = 0;
            // What is open in the center needs no card as well.
            self.items.clear();
        }
        KitResponse {
            response,
            value: settings,
        }
    }

    /// The notification center's content; returns whether the settings link was clicked.
    fn center(&mut self, ui: &mut Ui) -> bool {
        let p = theme::palette(ui.ctx());
        let m = theme::metrics();
        let mut settings = false;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Notifications")
                    .size(m.body_size)
                    .family(theme::semibold())
                    .color(p.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if IconButton::new(icon::GEAR_SIX, "Notification settings")
                    .size(Size::Xs)
                    .show(ui)
                    .clicked()
                {
                    settings = true;
                }
                if !self.history.is_empty()
                    && Button::new("Clear all")
                        .ghost()
                        .size(Size::Xs)
                        .show(ui)
                        .clicked()
                {
                    self.history.clear();
                }
            });
        });
        ui.add_space(theme::SM);
        if self.history.is_empty() {
            ui.add_space(theme::LG);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(icon::BELL)
                        .size(m.icon_lg)
                        .color(p.text_faint),
                );
                ui.add_space(theme::XS);
                ui.label(
                    RichText::new("No notifications yet")
                        .size(m.meta_size)
                        .color(p.text_dim),
                );
            });
            ui.add_space(theme::LG);
            return settings;
        }
        let now = chrono::Utc::now().timestamp();
        let mut gone = None;
        egui::ScrollArea::vertical()
            .id_salt("notification-center-list")
            .max_height(CENTER_LIST_HEIGHT)
            .min_scrolled_height(CENTER_LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, e) in self.history.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(theme::XS);
                        ui.separator();
                        ui.add_space(theme::XS);
                    }
                    let (glyph, color) = match e.kind {
                        ToastKind::Error => (icon::WARNING_CIRCLE, p.danger),
                        ToastKind::Success => (icon::CHECK_CIRCLE, p.success),
                        ToastKind::Info => (icon::INFO, p.accent),
                    };
                    ui.push_id(e.id, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = theme::SM;
                            ui.label(RichText::new(glyph).size(m.icon_md).color(color));
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = theme::XS;
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&e.title)
                                            .size(m.body_size)
                                            .family(theme::medium())
                                            .color(p.text),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(super::widgets::relative_time(
                                                    e.at, now,
                                                ))
                                                .size(m.meta_size)
                                                .color(p.text_faint),
                                            )
                                            .kit_tooltip(super::widgets::full_date(
                                                e.at,
                                                local_offset_minutes(e.at),
                                            ))
                                            .show();
                                        },
                                    );
                                });
                                let body = e.body.trim();
                                // What the card says (a hint before git's own words), the full
                                // output in the tooltip.
                                let said = card_text(e.kind, &e.body, e.help);
                                let text = said.body.map_or("", str::trim);
                                if !text.is_empty() {
                                    let lines: Vec<&str> = text.lines().collect();
                                    let shown = lines
                                        .iter()
                                        .take(CENTER_BODY_LINES)
                                        .copied()
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    let label = ui.add(
                                        egui::Label::new(
                                            RichText::new(shown)
                                                .size(m.meta_size)
                                                .color(p.text_dim),
                                        )
                                        .wrap(),
                                    );
                                    if lines.len() > CENTER_BODY_LINES || said.details.is_some() {
                                        label.kit_tooltip(body).show();
                                    }
                                }
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = theme::XS;
                                    // The buttons' labels, not their padding, line up with the
                                    // text above.
                                    ui.add_space(-theme::metrics().button_pad_x_sm);
                                    if let Some(recovery) = &e.recovery {
                                        let label = match recovery {
                                            Recovery::Pull(_) => "Pull",
                                            Recovery::Accounts => "Open Accounts",
                                        };
                                        if Button::new(label)
                                            .secondary()
                                            .size(Size::Xs)
                                            .show(ui)
                                            .clicked()
                                        {
                                            self.chosen.push(recovery.clone());
                                        }
                                    }
                                    if Button::new("Copy")
                                        .ghost()
                                        .size(Size::Xs)
                                        .icon(icon::COPY)
                                        .show(ui)
                                        .clicked()
                                    {
                                        let text = if body.is_empty() {
                                            e.title.clone()
                                        } else {
                                            format!("{}\n{body}", e.title)
                                        };
                                        ui.ctx().copy_text(text);
                                    }
                                    if Button::new("Dismiss")
                                        .ghost()
                                        .size(Size::Xs)
                                        .show(ui)
                                        .clicked()
                                    {
                                        gone = Some(e.id);
                                    }
                                });
                            });
                        });
                    });
                }
            });
        if let Some(id) = gone {
            self.history.retain(|e| e.id != id);
        }
        settings
    }
}

/// The local zone's offset from UTC at `seconds`, in minutes.
fn local_offset_minutes(seconds: i64) -> i32 {
    use chrono::Offset;
    chrono::DateTime::from_timestamp(seconds, 0).map_or(0, |d| {
        d.with_timezone(&chrono::Local)
            .offset()
            .fix()
            .local_minus_utc()
            / 60
    })
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
        toasts.push(
            &ctx,
            ToastKind::Error,
            Topic::Error,
            "Push failed",
            "fatal: refused",
        );
        for i in 0..4 {
            toasts.push(
                &ctx,
                ToastKind::Success,
                Topic::Update,
                format!("Done {i}"),
                "",
            );
        }
        toasts.push(&ctx, ToastKind::Info, Topic::Update, "Fetched", "");
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
        toasts.push(
            &ctx,
            ToastKind::Error,
            Topic::Error,
            "Checkout failed",
            stderr.clone(),
        );
        let one = "fatal: couldn't find remote ref nope";
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Fetch failed", one);
        let rejected = "! [rejected] main -> main (fetch first)";
        toasts.push(
            &ctx,
            ToastKind::Error,
            Topic::Error,
            "Push failed",
            rejected,
        );
        let text: Vec<CardText<'_>> = toasts
            .items
            .iter()
            .map(|t| card_text(t.kind, &t.body, t.help))
            .collect();
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
                Topic::Error,
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
        toasts.push(
            &ctx,
            ToastKind::Success,
            Topic::Update,
            "Committed",
            "Finished safely",
        );
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
    fn notification_stack_fits_the_viewport() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut toasts = Toasts::default();
        toasts.push(
            &ctx,
            ToastKind::Error,
            Topic::Error,
            "Push failed",
            "fetch first",
        );
        toasts.push(
            &ctx,
            ToastKind::Success,
            Topic::Update,
            "Committed",
            "Update README",
        );
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
        toasts.push(
            &ctx,
            ToastKind::Success,
            Topic::Update,
            "Pushed",
            "main to origin",
        );
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
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Older", "first");
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Newer", "second");
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

    fn titles(toasts: &Toasts) -> (Vec<&str>, Vec<&str>) {
        (
            toasts.items.iter().map(|t| t.title.as_str()).collect(),
            toasts.history.iter().map(|e| e.title.as_str()).collect(),
        )
    }

    #[test]
    fn by_default_only_notifications_that_need_the_user_show_as_cards() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Success, Topic::Finished, "Pushed", "");
        toasts.push(&ctx, ToastKind::Info, Topic::Confirmation, "Copied", "abc");
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Fetch failed", "no");
        toasts.push(&ctx, ToastKind::Info, Topic::Attention, "SSH", "touch it");
        toasts.push(&ctx, ToastKind::Info, Topic::Update, "Canopy 9 is out", "");
        let (cards, center) = titles(&toasts);
        assert_eq!(cards, ["Fetch failed", "SSH", "Canopy 9 is out"]);
        assert_eq!(
            center,
            ["Canopy 9 is out", "SSH", "Fetch failed", "Copied", "Pushed"],
            "the center lists every notification, newest first"
        );
        assert_eq!(toasts.unread, 0, "every one that needs the user was a card");
    }

    #[test]
    fn with_cards_off_nothing_pops_up_and_the_bell_counts_what_needs_the_user() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.set_prefs(NotificationPrefs {
            cards: false,
            finished: true,
            ..Default::default()
        });
        toasts.push(&ctx, ToastKind::Success, Topic::Finished, "Pushed", "");
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Fetch failed", "no");
        toasts.push(&ctx, ToastKind::Info, Topic::Attention, "SSH", "touch it");
        let (cards, center) = titles(&toasts);
        assert_eq!(cards, Vec::<&str>::new());
        assert_eq!(center.len(), 3);
        assert_eq!(toasts.unread, 2, "a finished push asks for no attention");
    }

    #[test]
    fn a_topic_switched_off_still_reaches_the_center() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.set_prefs(NotificationPrefs {
            errors: false,
            ..Default::default()
        });
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Fetch failed", "no");
        let (cards, center) = titles(&toasts);
        assert_eq!(cards, Vec::<&str>::new());
        assert_eq!(center, ["Fetch failed"]);
        assert_eq!(toasts.unread, 1);
    }

    #[test]
    fn the_center_keeps_the_newest_hundred() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        for i in 0..HISTORY + 5 {
            toasts.push(
                &ctx,
                ToastKind::Info,
                Topic::Confirmation,
                format!("n{i}"),
                "",
            );
        }
        assert_eq!(toasts.history.len(), HISTORY);
        assert_eq!(
            toasts.history.front().map(|e| e.title.as_str()),
            Some(format!("n{}", HISTORY + 4).as_str())
        );
    }

    /// The bell opens the center, which marks everything read and lists the notifications; while
    /// it is open, new ones go to the list rather than a card.
    #[test]
    fn opening_the_bell_lists_notifications_and_clears_the_count() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        motion::set_reduced(true);
        let mut toasts = Toasts::default();
        toasts.set_prefs(NotificationPrefs {
            cards: false,
            ..Default::default()
        });
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Fetch failed", "no");
        let frame = |toasts: &mut Toasts, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                        let _ = toasts.bell(ui);
                    });
                },
            );
            out.textures_delta.clear();
            out.platform_output
                .accesskit_update
                .map(|u| u.nodes.into_iter().map(|(_, n)| n).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let nodes = frame(&mut toasts, Vec::new());
        let bell = nodes
            .iter()
            .find(|n| n.label() == Some("Notifications (cards off), 1 unread"))
            .expect("the bell names the unread count");
        let b = bell.bounds().expect("bounds");
        let at = egui::pos2(
            crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
            crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
        );
        let mut click = vec![egui::Event::PointerMoved(at)];
        for pressed in [true, false] {
            click.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        let _ = frame(&mut toasts, click);
        let nodes = frame(&mut toasts, Vec::new());
        motion::set_reduced(false);
        assert!(toasts.center_open, "the click opened the center");
        assert_eq!(toasts.unread, 0);
        assert!(
            nodes
                .iter()
                .any(|n| n.label().or(n.value()) == Some("Fetch failed")),
            "the center lists it"
        );
        toasts.set_prefs(NotificationPrefs::default());
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Push failed", "no");
        assert!(toasts.items.is_empty(), "no card while the center is open");
        assert_eq!(toasts.history.len(), 2);
    }

    /// The center says what the card says: a known git failure as its hint, git's own output in
    /// the tooltip and the copy, not as the text.
    #[test]
    fn the_center_explains_a_failure_like_its_card() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        motion::set_reduced(true);
        let mut toasts = Toasts::default();
        let raw = "! [rejected]        main -> main (fetch first)\nerror: failed to push some refs";
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Push failed", raw);
        let e = &toasts.history[0];
        let hint = card_text(e.kind, &e.body, e.help)
            .body
            .expect("a hint")
            .to_owned();
        assert_ne!(hint, raw.lines().next().unwrap_or_default());
        let mut labels = Vec::new();
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let _ = toasts.center(ui);
            });
            out.textures_delta.clear();
            labels = out
                .platform_output
                .accesskit_update
                .map(|u| {
                    u.nodes
                        .into_iter()
                        .filter_map(|(_, n)| n.label().or(n.value()).map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
        }
        motion::set_reduced(false);
        assert!(labels.contains(&hint), "{labels:?}");
        assert!(
            !labels.iter().any(|l| l.contains("[rejected]")),
            "{labels:?}"
        );
    }

    #[test]
    fn the_center_closes_when_its_settings_link_opens_settings() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        motion::set_reduced(true);
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Fetch failed", "no");
        let frame = |toasts: &mut Toasts, events: Vec<egui::Event>| {
            let mut settings = false;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                        settings = toasts.bell(ui).value;
                    });
                },
            );
            out.textures_delta.clear();
            let nodes = out
                .platform_output
                .accesskit_update
                .map(|u| u.nodes.into_iter().map(|(_, n)| n).collect::<Vec<_>>())
                .unwrap_or_default();
            (nodes, settings)
        };
        let click = |nodes: &[egui::accesskit::Node], label: &str| {
            let b = nodes
                .iter()
                .find(|n| n.label() == Some(label))
                .and_then(egui::accesskit::Node::bounds)
                .unwrap_or_else(|| panic!("{label} is drawn"));
            let at = egui::pos2(
                crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
                crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
            );
            let mut events = vec![egui::Event::PointerMoved(at)];
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
            events
        };
        let (nodes, _) = frame(&mut toasts, Vec::new());
        let (_, _) = frame(&mut toasts, click(&nodes, "Notifications"));
        let (nodes, _) = frame(&mut toasts, Vec::new());
        assert!(toasts.center_open, "the click opened the center");
        let (_, settings) = frame(&mut toasts, click(&nodes, "Notification settings"));
        let (nodes, _) = frame(&mut toasts, Vec::new());
        motion::set_reduced(false);
        assert!(settings, "the link asks for Settings");
        assert!(!toasts.center_open, "the center closed for it");
        assert!(
            !nodes
                .iter()
                .any(|n| n.label() == Some("Notification settings")),
            "and is no longer drawn"
        );
    }
}
