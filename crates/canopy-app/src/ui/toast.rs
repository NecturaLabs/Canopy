//! Bottom-right notifications, with a bounded scroll stack when several need attention: the queue,
//! its timers and the decisions notifications ask for ([`Offer`]). Each card is a
//! `kit::toast::ToastCard`. Every notification also
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
use serde::{Deserialize, Serialize};

use super::kit::Tone;
use super::kit::button::IconButton;
use super::kit::chip::Badge;
use super::kit::overlay::{self, Popover, TooltipExt};
use super::kit::toast::{ACTIONS_GAP, BODY_LEADING, COLUMN_GAP, Choice, TITLE_GAP, ToastCard};
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
                "SSH waits for your security key, or an installed update waits for a restart."
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

/// A decision a notification asks for; the app carries out the one chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// A push was refused because the remote moved on: pull the repository at this path.
    Pull(PathBuf),
    /// The host refused the credentials: open Settings at Accounts.
    Accounts,
    /// Git's own configuration sets a choice against Canopy's recommendation: open Settings at
    /// Git, where each choice names where it is set.
    GitSettings,
    /// A new build is out: open its update dialog.
    ReviewUpdate,
    /// A new build is installed at this path: restart into it.
    Restart(PathBuf),
}

impl Offer {
    /// The buttons that ask for it, named for what they do.
    fn choice(&self) -> Choice<'static> {
        let (confirm, glyph, decline) = match self {
            Self::Pull(_) => ("Pull", icon::ARROW_DOWN, "Not now"),
            Self::Accounts => ("Open Accounts", icon::USER_CIRCLE, "Not now"),
            Self::GitSettings => ("Open Git settings", icon::GIT_BRANCH, "Later"),
            Self::ReviewUpdate => ("Review update", icon::DOWNLOAD_SIMPLE, "Later"),
            Self::Restart(_) => ("Restart now", icon::ARROW_CLOCKWISE, "Later"),
        };
        Choice {
            confirm,
            glyph: Some(glyph),
            decline,
        }
    }
}

fn error_help(
    title: &str,
    body: &str,
    repo: Option<&Path>,
) -> Option<(&'static str, Option<Offer>)> {
    let lower = body.to_lowercase();
    if title.to_lowercase().contains("push")
        && (lower.contains("fetch first") || lower.contains("non-fast-forward"))
    {
        Some((
            "The remote has commits you do not have yet. Pull and resolve any conflicts before pushing again.",
            repo.map(|r| Offer::Pull(r.to_path_buf())),
        ))
    } else if lower.contains("authentication failed")
        || lower.contains("http basic: access denied")
        || lower.contains("bad credentials")
    {
        Some((
            "The host did not accept your credentials.",
            Some(Offer::Accounts),
        ))
    } else if lower.contains("unable to read askpass response") {
        Some(("Sign-in was cancelled.", None))
    } else if lower.contains("could not resolve host")
        || lower.contains("could not resolve hostname")
    {
        Some((
            "The host could not be reached. Check your connection and the remote address.",
            None,
        ))
    } else if lower.contains("permission denied (publickey)") {
        Some((
            "SSH did not accept your key. Check the key's access to this repository.",
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
    /// Git's full output when the body says less than it (a hint, or its first line): the
    /// center's tooltip shows it.
    details: Option<&'a str>,
}

/// What a notification of `kind` with `body` (and, for an error, its `help`) says under its
/// title, on its card and in the center alike.
fn card_text<'a>(kind: ToastKind, body: &'a str, help: Option<&'static str>) -> CardText<'a> {
    if kind != ToastKind::Error {
        return CardText {
            body: Some(body).filter(|b| !b.is_empty()),
            details: None,
        };
    }
    if help.is_some() {
        return CardText {
            body: help,
            details: Some(body),
        };
    }
    // No hint: git's own words say why, and the details hold the rest. Git opens some commands
    // with progress. A remote hook's explanation is more useful than git's generic push error.
    let lines = || body.lines().map(str::trim);
    let first = lines()
        .find(|l| {
            l.starts_with("fatal:")
                || l.starts_with("remote: fatal:")
                || l.starts_with("remote: error:")
        })
        .or_else(|| {
            lines().find(|l| {
                l.strip_prefix("remote:").is_some_and(|text| {
                    let text = text.trim();
                    !text.is_empty()
                        && ![
                            "Enumerating objects:",
                            "Counting objects:",
                            "Compressing objects:",
                            "Total ",
                            "Resolving deltas:",
                            "Receiving objects:",
                            "Writing objects:",
                        ]
                        .iter()
                        .any(|prefix| text.starts_with(prefix))
                })
            })
        })
        .or_else(|| lines().find(|l| l.starts_with("error:")))
        .or_else(|| lines().find(|l| !l.is_empty()));
    let more = first.is_some_and(|f| body.trim() != f);
    CardText {
        body: first,
        details: more.then_some(body),
    }
}

/// How long a card stays, in seconds of unpaused time (the center keeps every notification).
/// Design systems settle on about 5 s for a short message and cap it at 10 to 15 s (Atlassian
/// flags 5 to 15 s, Canva 5 s up to 10, SAP Fiori 4 to 10); WCAG 2.2.1 asks that the timer stop
/// while the user attends to it, which `Toasts::show` does. So a card lasts as long as its words
/// take to read, at about four words a second after two seconds to notice it, within a range
/// per kind: an error is read more carefully, and a decision waits longest before it moves to
/// the center (and the bell's count).
const NOTICE_SECONDS: f64 = 2.0;
const SECONDS_PER_WORD: f64 = 0.25;
const INFO_SECONDS: (f64, f64) = (5.0, 10.0);
const ERROR_SECONDS: (f64, f64) = (8.0, 15.0);
const DECISION_SECONDS: f64 = 15.0;

/// How long a card with `title` and `body` (what it says, not git's raw output) stays.
fn lifetime(kind: ToastKind, decision: bool, title: &str, body: Option<&str>) -> f64 {
    if decision {
        return DECISION_SECONDS;
    }
    let words = title.split_whitespace().count() + body.map_or(0, |b| b.split_whitespace().count());
    let reading =
        NOTICE_SECONDS + SECONDS_PER_WORD * f64::from(u32::try_from(words).unwrap_or(u32::MAX));
    let (least, most) = if kind == ToastKind::Error {
        ERROR_SECONDS
    } else {
        INFO_SECONDS
    };
    reading.clamp(least, most)
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    kind: ToastKind,
    topic: Topic,
    title: String,
    body: String,
    born: f64,
    /// Seconds of unpaused time left before it goes.
    left: f64,
    help: Option<&'static str>,
    offer: Option<Offer>,
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
    /// The repository it is about, by folder name.
    source: Option<String>,
    /// What an error means, before git's own words.
    help: Option<&'static str>,
    offer: Option<Offer>,
    unread: bool,
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
    /// How many of the `unread` are errors: the bell turns danger while there is one.
    unread_errors: usize,
    /// Whether the notification center was open last frame (no cards show meanwhile).
    center_open: bool,
    /// Offers taken in the center, handed out by [`Toasts::show`].
    chosen: Vec<Offer>,
    /// When the timers last ran.
    ticked: Option<f64>,
    /// A card's button had keyboard focus last frame (the timers wait, as for the pointer).
    focus_held: bool,
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

/// The key the notification center is kept under between runs.
pub const STORAGE_KEY: &str = "canopy-notifications-v1";

/// A notification as it is kept on disk: what the center lists, and the decision it still asks
/// for when that one survives a restart.
#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    kind: SavedKind,
    title: String,
    body: String,
    at: i64,
    source: Option<String>,
    offer: Option<SavedOffer>,
    #[serde(default)]
    unread: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum SavedKind {
    Info,
    Success,
    Error,
}

/// The decisions that still mean something after a restart: an update to review or to restart
/// into does not.
#[derive(Debug, Serialize, Deserialize)]
enum SavedOffer {
    Pull(PathBuf),
    Accounts,
    GitSettings,
}

impl Toasts {
    /// The center's notifications as text to keep between runs.
    pub fn to_json(&self) -> String {
        let saved: Vec<Saved> = self
            .history
            .iter()
            .map(|e| Saved {
                kind: match e.kind {
                    ToastKind::Info => SavedKind::Info,
                    ToastKind::Success => SavedKind::Success,
                    ToastKind::Error => SavedKind::Error,
                },
                title: e.title.clone(),
                body: e.body.clone(),
                at: e.at,
                source: e.source.clone(),
                unread: e.unread
                    || (matches!(
                        e.offer,
                        Some(Offer::Pull(_) | Offer::Accounts | Offer::GitSettings)
                    ) && self.items.iter().any(|t| t.id == e.id)),
                offer: match &e.offer {
                    Some(Offer::Pull(path)) => Some(SavedOffer::Pull(path.clone())),
                    Some(Offer::Accounts) => Some(SavedOffer::Accounts),
                    Some(Offer::GitSettings) => Some(SavedOffer::GitSettings),
                    Some(Offer::ReviewUpdate | Offer::Restart(_)) | None => None,
                },
            })
            .collect();
        serde_json::to_string(&saved).unwrap_or_default()
    }

    /// Puts back the center and its unread state. Legacy entries without that state are read;
    /// text that does not parse is ignored.
    pub fn restore_json(&mut self, json: &str) {
        let Ok(saved) = serde_json::from_str::<Vec<Saved>>(json) else {
            return;
        };
        self.history.clear();
        for s in saved.into_iter().take(HISTORY) {
            self.next += 1;
            self.history.push_back(Entry {
                id: self.next,
                kind: match s.kind {
                    SavedKind::Info => ToastKind::Info,
                    SavedKind::Success => ToastKind::Success,
                    SavedKind::Error => ToastKind::Error,
                },
                title: s.title,
                body: s.body,
                at: s.at,
                source: s.source,
                unread: s.unread,
                help: None,
                offer: s.offer.map(|o| match o {
                    SavedOffer::Pull(path) => Offer::Pull(path),
                    SavedOffer::Accounts => Offer::Accounts,
                    SavedOffer::GitSettings => Offer::GitSettings,
                }),
            });
        }
        self.count_unread();
    }

    fn count_unread(&mut self) {
        self.unread = self.history.iter().filter(|e| e.unread).count();
        self.unread_errors = self
            .history
            .iter()
            .filter(|e| e.unread && e.kind == ToastKind::Error)
            .count();
    }

    fn mark_unread(&mut self, id: u64) {
        if let Some(entry) = self.history.iter_mut().find(|e| e.id == id) {
            entry.unread = true;
        }
        self.count_unread();
    }

    /// Applies new card filters immediately; hidden cards remain in the notification center.
    pub fn set_prefs(&mut self, prefs: NotificationPrefs) {
        self.prefs = prefs;
        self.items.retain(|item| prefs.shows(item.topic));
        self.places
            .retain(|(id, _)| self.items.iter().any(|item| item.id == *id));
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

    /// A repository notice keeps the recovery it offers (a pull) attached to that repository.
    pub fn push_for(
        &mut self,
        ctx: &egui::Context,
        kind: ToastKind,
        topic: Topic,
        title: impl Into<String>,
        body: impl Into<String>,
        repo: Option<&Path>,
    ) {
        let title = title.into();
        let body = body.into();
        let (help, offer) = if kind == ToastKind::Error {
            error_help(&title, &body, repo)
                .map_or((None, None), |(help, offer)| (Some(help), offer))
        } else {
            (None, None)
        };
        let source = repo
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned());
        self.notify(ctx, kind, topic, title, body, help, offer, source);
    }

    /// Notifies with a decision to make: the card shows buttons for it for a decision's time
    /// (`DECISION_SECONDS`), and the center offers it until it is made or declined.
    pub fn push_offer(
        &mut self,
        ctx: &egui::Context,
        kind: ToastKind,
        topic: Topic,
        title: impl Into<String>,
        body: impl Into<String>,
        offer: Offer,
    ) {
        self.notify(
            ctx,
            kind,
            topic,
            title.into(),
            body.into(),
            None,
            Some(offer),
            None,
        );
    }

    /// Like [`Self::push_offer`] (an Info notice), about `repo`, and only when the center does not already hold
    /// the same notice for it (one kept from an earlier run included).
    pub fn push_offer_for(
        &mut self,
        ctx: &egui::Context,
        topic: Topic,
        title: &str,
        body: String,
        offer: Offer,
        repo: &Path,
    ) {
        let source = repo
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if self
            .history
            .iter()
            .any(|e| e.title == title && e.source == source)
        {
            return;
        }
        self.notify(
            ctx,
            ToastKind::Info,
            topic,
            title.into(),
            body,
            None,
            Some(offer),
            source,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the one place every kind of notification comes together"
    )]
    fn notify(
        &mut self,
        ctx: &egui::Context,
        kind: ToastKind,
        topic: Topic,
        title: String,
        body: String,
        help: Option<&'static str>,
        offer: Option<Offer>,
        source: Option<String>,
    ) {
        let left = lifetime(
            kind,
            offer.is_some(),
            &title,
            card_text(kind, &body, help).body,
        );
        self.next += 1;
        let card = self.prefs.shows(topic) && !self.center_open;
        self.history.push_front(Entry {
            id: self.next,
            kind,
            title: title.clone(),
            body: body.clone(),
            at: chrono::Utc::now().timestamp(),
            source,
            help,
            offer: offer.clone(),
            unread: !card && topic.counts() && !self.center_open,
        });
        self.history.truncate(HISTORY);
        self.count_unread();
        if !card {
            tracing::debug!(title = %title, ?topic, "notification kept for the center");
            ctx.request_repaint();
            return;
        }
        self.items.push(Toast {
            id: self.next,
            kind,
            topic,
            title,
            body,
            born: ctx.input(|i| i.time),
            left,
            help,
            offer,
        });
        if self.items.len() > 5 {
            // Evict an older card, never the one that just arrived. Prefer an older routine
            // card to an error, and an error to an unanswered decision.
            let older = &self.items[..self.items.len() - 1];
            let oldest = older
                .iter()
                .position(|t| t.offer.is_none() && t.kind != ToastKind::Error)
                .or_else(|| older.iter().position(|t| t.offer.is_none()))
                .unwrap_or(0);
            let gone = self.items.remove(oldest);
            if gone.offer.is_some() {
                // Not made: it waits in the center, counted on the bell as when it runs out.
                tracing::warn!(title = %gone.title, "a sixth decision pushed out the oldest card");
                self.mark_unread(gone.id);
            }
        }
        ctx.request_repaint();
    }

    /// Draws the toasts and expires old ones; returns the offers taken on a card or in the
    /// notification center.
    pub fn show(&mut self, ctx: &egui::Context) -> Vec<Offer> {
        let chosen = std::mem::take(&mut self.chosen);
        let mut actions = self.show_cards(ctx);
        actions.extend(chosen);
        actions
    }

    fn show_cards(&mut self, ctx: &egui::Context) -> Vec<Offer> {
        let now = ctx.input(|i| i.time);
        // The timers wait while the pointer is on the stack or the window is in the background.
        let hovered = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|at| {
            ctx.memory(|m| m.area_rect(Id::new("notifications")))
                .is_some_and(|r| r.contains(at))
        });
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let paused = hovered || self.focus_held || !focused;
        let elapsed = self.ticked.map_or(0.0, |t| (now - t).max(0.0));
        self.ticked = Some(now);
        if !paused {
            for t in &mut self.items {
                t.left -= elapsed;
            }
        }
        // A decision nobody made waits in the center, counted on the bell.
        let undecided: Vec<_> = self
            .items
            .iter()
            .filter(|t| t.left <= 0.0 && t.offer.is_some())
            .map(|t| t.id)
            .collect();
        for id in undecided {
            self.mark_unread(id);
        }
        self.items.retain(|t| t.left > 0.0);
        self.focus_held = false;
        if self.items.is_empty() {
            self.places.clear();
            return Vec::new();
        }
        let mut actions = Vec::new();
        let m = theme::metrics();
        let screen = ctx.content_rect();
        let mut dismiss = None;
        let mut taken = None;
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
                                    .clickable(t.offer.is_none());
                            let text = card_text(t.kind, &t.body, t.help);
                            if let Some(body) = text.body {
                                card = card.body(body);
                            }
                            if let Some(offer) = &t.offer {
                                card = card.choice(offer.choice());
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
                            // Declined, the decision stays in the center for later.
                            self.focus_held |= out.focused;
                            if out.declined || out.clicked {
                                dismiss = Some(t.id);
                            }
                            if out.confirmed
                                && let Some(offer) = &t.offer
                            {
                                actions.push(offer.clone());
                                dismiss = Some(t.id);
                                taken = Some(t.id);
                            }
                        }
                    });
            });
        self.slide(ctx, now, &cards);
        if let Some(id) = dismiss {
            self.items.retain(|t| t.id != id);
        }
        if let Some(id) = taken {
            self.history.retain(|e| e.id != id);
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

/// Where the bell's badge goes: over the button's top right corner, but never outside `clip`,
/// which would cut it off.
fn badge_origin(button: egui::Rect, size: egui::Vec2, clip: egui::Rect) -> egui::Pos2 {
    let at = button.right_top() + vec2(-size.x * 0.4, -size.y * 0.3);
    egui::pos2(
        at.x.min(clip.right() - size.x).max(clip.left()),
        at.y.min(clip.bottom() - size.y).max(clip.top()),
    )
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
        if let Some(badge) = self.bell_badge() {
            let size = badge.measure(ui);
            let at = badge_origin(response.rect, size, ui.clip_rect());
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
            for entry in &mut self.history {
                entry.unread = false;
            }
            self.count_unread();
            // What is open in the center needs no card as well.
            self.items.clear();
        }
        KitResponse {
            response,
            value: settings,
        }
    }

    /// The bell's count: `danger` when one of the unread notifications is an error, else the
    /// accent.
    fn bell_badge(&self) -> Option<Badge> {
        if self.unread == 0 {
            return None;
        }
        let tone = if self.unread_errors > 0 {
            Tone::Danger
        } else {
            Tone::Accent
        };
        Some(
            Badge::new(super::kit::chip::count_text(self.unread))
                .tone(tone)
                .strong(),
        )
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
        let divider = theme::mix(theme::surfaces(ui.ctx()).overlay, p.text, 0.08);
        egui::ScrollArea::vertical()
            .id_salt("notification-center-list")
            .max_height(CENTER_LIST_HEIGHT)
            .min_scrolled_height(CENTER_LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, e) in self.history.iter().enumerate() {
                    if index > 0 {
                        // The same quiet hairline in every theme, contrast included: rows part
                        // by space first, the line only steadies the eye.
                        ui.add_space(theme::SM);
                        let (line, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 1.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().hline(
                            line.x_range(),
                            line.center().y,
                            egui::Stroke::new(1.0, divider),
                        );
                        ui.add_space(theme::SM);
                    }
                    let (glyph, color) = match e.kind {
                        ToastKind::Error => (icon::WARNING_CIRCLE, p.danger),
                        ToastKind::Success => (icon::CHECK_CIRCLE, p.success),
                        ToastKind::Info => (icon::INFO, p.accent),
                    };
                    ui.push_id(e.id, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                            // The card's glyph, centred on the title's line once it is laid out.
                            let (slot, _) = ui.allocate_exact_size(
                                vec2(m.icon_toast, m.icon_toast),
                                egui::Sense::hover(),
                            );
                            let painter = ui.painter().clone();
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = TITLE_GAP;
                                // Rows as tall as their content, so the body follows the title
                                // as on a card.
                                ui.spacing_mut().interact_size.y = 0.0;
                                ui.horizontal(|ui| {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::TOP),
                                        |ui| {
                                            if IconButton::new(icon::X, "Dismiss notification")
                                                .size(Size::Xs)
                                                .id_salt(("dismiss", e.id))
                                                .show(ui)
                                                .clicked()
                                            {
                                                gone = Some(e.id);
                                            }
                                            let title = ui
                                                .allocate_ui_with_layout(
                                                    vec2(ui.available_width(), 0.0),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(&e.title)
                                                                    .size(m.body_size)
                                                                    .family(theme::semibold())
                                                                    .color(p.text),
                                                            )
                                                            .wrap(),
                                                        )
                                                    },
                                                )
                                                .inner;
                                            painter.text(
                                                egui::pos2(
                                                    slot.center().x,
                                                    title.rect.top() + m.body_size * 0.5,
                                                ),
                                                Align2::CENTER_CENTER,
                                                glyph,
                                                egui::FontId::proportional(m.icon_toast),
                                                color,
                                            );
                                        },
                                    );
                                });
                                let when = super::widgets::relative_time(e.at, now);
                                let when = match &e.source {
                                    Some(source) => format!("{source}  ·  {when}"),
                                    None => when,
                                };
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(when).size(m.meta_size).color(p.text_faint),
                                    )
                                    .wrap(),
                                )
                                .kit_tooltip(super::widgets::full_date(
                                    e.at,
                                    local_offset_minutes(e.at),
                                ))
                                .show();
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
                                                .line_height(Some(
                                                    (m.meta_size * BODY_LEADING).round(),
                                                ))
                                                .color(p.text_dim),
                                        )
                                        .wrap(),
                                    );
                                    if lines.len() > CENTER_BODY_LINES || said.details.is_some() {
                                        label.kit_tooltip(body).show();
                                    }
                                }
                                // Only what can be done about it: the decision it asks for, and a
                                // failure's output to take elsewhere. Clear all empties the rest.
                                let copy = e.kind == ToastKind::Error;
                                if e.offer.is_none() && !copy {
                                    return;
                                }
                                ui.add_space(ACTIONS_GAP - TITLE_GAP);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = theme::XS;
                                    if let Some(offer) = &e.offer {
                                        let choice = offer.choice();
                                        let mut confirm =
                                            Button::new(choice.confirm).primary().size(Size::Xs);
                                        if let Some(g) = choice.glyph {
                                            confirm = confirm.icon(g);
                                        }
                                        if confirm.show(ui).clicked() {
                                            self.chosen.push(offer.clone());
                                            gone = Some(e.id);
                                        }
                                        if Button::new(choice.decline)
                                            .ghost()
                                            .size(Size::Xs)
                                            .show(ui)
                                            .clicked()
                                        {
                                            gone = Some(e.id);
                                        }
                                    } else {
                                        // A ghost button's label, not its padding, lines up with
                                        // the text above.
                                        ui.add_space(-m.button_pad_x_sm);
                                    }
                                    if copy
                                        && Button::new("Copy")
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

    #[test]
    fn cancelling_a_sign_in_prompt_is_not_reported_as_git_failing() {
        let raw =
            "git fetch failed: error: unable to read askpass response from '/opt/canopy/canopy'";
        let (help, offer) = error_help("Could not fetch", raw, None).expect("a plain explanation");
        assert_eq!(help, "Sign-in was cancelled.");
        assert!(offer.is_none());
    }

    #[test]
    fn topic_filters_apply_to_each_category_and_live_cards() {
        let ctx = egui::Context::default();
        for enabled_topic in Topic::ALL {
            let mut prefs = NotificationPrefs {
                cards: true,
                errors: false,
                attention: false,
                updates: false,
                finished: false,
                confirmations: false,
            };
            *prefs.topic_mut(enabled_topic) = true;
            let mut toasts = Toasts::default();
            toasts.set_prefs(prefs);
            for topic in Topic::ALL {
                let kind = if topic == Topic::Error {
                    ToastKind::Error
                } else {
                    ToastKind::Success
                };
                toasts.push(&ctx, kind, topic, topic.label(), "completed");
            }
            assert_eq!(toasts.items.len(), 1, "only {enabled_topic:?} shows a card");
            assert_eq!(toasts.items[0].topic, enabled_topic);
            assert_eq!(
                toasts.history.len(),
                Topic::ALL.len(),
                "all topics remain in the center"
            );

            *prefs.topic_mut(enabled_topic) = false;
            toasts.set_prefs(prefs);
            assert!(
                toasts.items.is_empty(),
                "switching off {enabled_topic:?} hides its card"
            );
            assert_eq!(toasts.history.len(), Topic::ALL.len());
        }
    }

    #[test]
    fn successful_fetch_respects_disabled_finished_operations() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.set_prefs(NotificationPrefs::default());
        // The same completed-operation topic is used by foreground and background fetches.
        for title in ["Fetched", "Background fetch complete"] {
            toasts.push(
                &ctx,
                ToastKind::Success,
                Topic::Finished,
                title,
                "up to date",
            );
        }
        assert!(
            toasts.items.is_empty(),
            "finished-operation cards are disabled by default"
        );
        assert_eq!(
            toasts.history.len(),
            2,
            "the notification center retains both results"
        );
    }

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

    /// Six decisions at once: the oldest card goes, and like one that ran out it waits in the
    /// center and counts on the bell.
    #[test]
    fn a_decision_pushed_out_counts_on_the_bell() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        for i in 0..6 {
            toasts.push_offer(
                &ctx,
                ToastKind::Info,
                Topic::Update,
                format!("Decide {i}"),
                "",
                Offer::ReviewUpdate,
            );
        }
        assert_eq!(toasts.items.len(), 5);
        assert_eq!(toasts.items[0].title, "Decide 1");
        assert_eq!(toasts.unread, 1);
    }

    /// A decision still waiting outlasts errors too: with nothing routine left to push out, the
    /// oldest error goes.
    #[test]
    fn a_sixth_notification_leaves_a_pending_decision_alone() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.push_offer(
            &ctx,
            ToastKind::Info,
            Topic::Update,
            "Canopy 9 is out",
            "",
            Offer::ReviewUpdate,
        );
        for i in 0..5 {
            toasts.push(
                &ctx,
                ToastKind::Error,
                Topic::Error,
                format!("Fetch failed {i}"),
                "no",
            );
        }
        let titles: Vec<&str> = toasts.items.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Canopy 9 is out",
                "Fetch failed 1",
                "Fetch failed 2",
                "Fetch failed 3",
                "Fetch failed 4"
            ]
        );
    }

    /// REVIEW-KIT-7: an error Canopy has no hint for still shows git's own first line on the
    /// card, with the whole output in the center's tooltip when there is more.
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
                details: Some(stderr.as_str()),
            }
        );
        assert_eq!(
            text[1],
            CardText {
                body: Some(one),
                details: None,
            },
            "one line has nothing more to show"
        );
        assert_eq!(text[2].body, toasts.items[2].help, "a hint still leads");
        assert_eq!(text[2].details, Some(rejected));
    }

    #[test]
    fn a_new_routine_card_does_not_evict_itself_when_five_errors_are_visible() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.set_prefs(NotificationPrefs {
            confirmations: true,
            ..Default::default()
        });
        for index in 0..5 {
            toasts.push(
                &ctx,
                ToastKind::Error,
                Topic::Error,
                format!("Error {index}"),
                "failure",
            );
        }
        toasts.push(&ctx, ToastKind::Success, Topic::Update, "Pushed", "main");
        assert_eq!(toasts.items.len(), 5);
        assert_eq!(
            toasts.items.last().map(|t| t.title.as_str()),
            Some("Pushed")
        );
        assert!(!toasts.items.iter().any(|t| t.title == "Error 0"));
        assert_eq!(
            toasts
                .items
                .iter()
                .filter(|t| t.kind == ToastKind::Error)
                .count(),
            4
        );

        toasts.push(&ctx, ToastKind::Info, Topic::Confirmation, "Copied", "SHA");
        assert_eq!(
            toasts.items.last().map(|t| t.title.as_str()),
            Some("Copied")
        );
        assert_eq!(toasts.items.len(), 5);
        assert!(!toasts.items.iter().any(|t| t.title == "Pushed"));
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
        // Its five seconds (four words: the shortest a card stays) run only now.
        let _ = frame_at(&ctx, &mut toasts, 20.2, None, true);
        let _ = frame_at(&ctx, &mut toasts, 24.9, None, true);
        assert_eq!(toasts.items.len(), 1, "not before its time");
        let _ = frame_at(&ctx, &mut toasts, 25.4, None, true);
        assert!(toasts.items.is_empty(), "gone after five unpaused seconds");
    }

    #[test]
    fn a_card_stays_as_long_as_its_words_take_within_its_kinds_range() {
        let many = ["word"; 60].join(" ");
        let cases = [
            ("short info", ToastKind::Info, false, "Copied", None, 5.0),
            (
                "short success",
                ToastKind::Success,
                false,
                "Pushed",
                None,
                5.0,
            ),
            (
                "twelve words",
                ToastKind::Success,
                false,
                "Pulled main",
                Some("ten more words to read here so it takes longer"),
                5.0,
            ),
            (
                "twenty words",
                ToastKind::Info,
                false,
                "a b c d e",
                Some("f g h i j k l m n o p q r s t"),
                7.0,
            ),
            (
                "long info",
                ToastKind::Info,
                false,
                "Note",
                Some(&many),
                10.0,
            ),
            (
                "short error",
                ToastKind::Error,
                false,
                "Fetch failed",
                None,
                8.0,
            ),
            (
                "long error",
                ToastKind::Error,
                false,
                "Failed",
                Some(&many),
                15.0,
            ),
            ("decision", ToastKind::Info, true, "Update", None, 15.0),
            (
                "long decision",
                ToastKind::Error,
                true,
                "x",
                Some(&many),
                15.0,
            ),
        ];
        for (name, kind, decision, title, body, expected) in cases {
            let got = lifetime(kind, decision, title, body);
            assert!((got - expected).abs() < 1e-9, "{name}: {got} s");
        }
    }

    /// A decision whose card ran out was not made: it waits in the center and the bell counts
    /// it. A routine card that ran out was seen and counts for nothing.
    #[test]
    fn a_decision_nobody_made_counts_on_the_bell() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut toasts = Toasts::default();
        let _ = frame_at(&ctx, &mut toasts, 0.0, None, true);
        toasts.push_offer(
            &ctx,
            ToastKind::Info,
            Topic::Update,
            "Canopy 9 is out",
            "",
            Offer::ReviewUpdate,
        );
        toasts.push(&ctx, ToastKind::Info, Topic::Update, "Up to date", "");
        for n in 1..=170 {
            let _ = frame_at(&ctx, &mut toasts, f64::from(n) * 0.1, None, true);
        }
        assert!(toasts.items.is_empty(), "both cards ran out");
        assert_eq!(toasts.unread, 1, "only the decision counts");
        assert_eq!(
            toasts.history.front().map(|e| e.offer.clone()),
            Some(None),
            "the newest entry is the routine one"
        );
        assert_eq!(toasts.history[1].offer, Some(Offer::ReviewUpdate));
    }

    /// Runs one frame at `time` that clicks the text `label` where the last frame drew it, and
    /// returns the offers taken and where `label` is drawn now.
    fn click_text(
        ctx: &egui::Context,
        toasts: &mut Toasts,
        time: f64,
        at: Option<egui::Pos2>,
    ) -> (Vec<Offer>, egui::FullOutput) {
        let mut events = Vec::new();
        if let Some(at) = at {
            events.push(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        let mut offers = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1024.0, 640.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |_| offers = toasts.show(ctx),
        );
        output.textures_delta.clear();
        (offers, output)
    }

    fn text_at(output: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
        output.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
    }

    /// Taking a card's offer hands it to the app and clears it from the center too; declining
    /// it only closes the card, and the center still offers it.
    #[test]
    fn a_cards_choice_is_taken_or_left_for_the_center() {
        for (button, taken) in [("Review update", true), ("Later", false)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
            motion::set_reduced(true);
            let mut toasts = Toasts::default();
            toasts.push_offer(
                &ctx,
                ToastKind::Info,
                Topic::Update,
                "Canopy 9 is out",
                "See what's new.",
                Offer::ReviewUpdate,
            );
            let mut at = None;
            for n in 0..3 {
                let (_, out) = click_text(&ctx, &mut toasts, 0.1 * f64::from(n), None);
                at = text_at(&out, button).or(at);
            }
            let (offers, _) = click_text(&ctx, &mut toasts, 0.4, at);
            motion::set_reduced(false);
            assert!(toasts.items.is_empty(), "{button}: the card closed");
            if taken {
                assert_eq!(offers, [Offer::ReviewUpdate], "{button}");
                assert!(toasts.history.is_empty(), "{button}: done, not kept");
            } else {
                assert!(offers.is_empty(), "{button}");
                assert_eq!(
                    toasts.history.front().and_then(|e| e.offer.clone()),
                    Some(Offer::ReviewUpdate),
                    "{button}: still offered in the center"
                );
            }
        }
    }

    /// The center's rows carry buttons only for what can be done: a decision's own two, a
    /// failure's Copy, and none on a plain row. Taking the decision there hands it to the app.
    #[test]
    fn the_center_offers_a_decision_with_its_own_buttons() {
        use egui::accesskit::Role;

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        motion::set_reduced(true);
        let mut toasts = Toasts::default();
        toasts.set_prefs(NotificationPrefs {
            cards: false,
            ..Default::default()
        });
        toasts.push(&ctx, ToastKind::Success, Topic::Finished, "Pushed", "main");
        let frame = |toasts: &mut Toasts, events: Vec<egui::Event>| {
            let mut buttons = Vec::new();
            for _ in 0..3 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        events: events.clone(),
                        ..Default::default()
                    },
                    |ui| {
                        let _ = toasts.center(ui);
                    },
                );
                out.textures_delta.clear();
                buttons = out
                    .platform_output
                    .accesskit_update
                    .map_or(Vec::new(), |u| {
                        u.nodes
                            .into_iter()
                            .filter(|(_, n)| n.role() == Role::Button)
                            .filter_map(|(_, n)| Some((n.label()?.to_owned(), n.bounds()?)))
                            .collect::<Vec<_>>()
                    });
            }
            buttons
        };
        let names = |b: &[(String, egui::accesskit::Rect)]| {
            let mut n: Vec<String> = b.iter().map(|(l, _)| l.clone()).collect();
            n.sort();
            n
        };
        let header = ["Clear all", "Dismiss notification", "Notification settings"];
        assert_eq!(names(&frame(&mut toasts, Vec::new())), header);
        toasts.push_offer(
            &ctx,
            ToastKind::Info,
            Topic::Attention,
            "Canopy 9 is installed",
            "",
            Offer::Restart(PathBuf::from("canopy")),
        );
        let buttons = frame(&mut toasts, Vec::new());
        assert_eq!(
            names(&buttons),
            [
                "Clear all",
                "Dismiss notification",
                "Dismiss notification",
                "Later",
                "Notification settings",
                "Restart now"
            ]
        );
        let (_, b) = buttons
            .iter()
            .find(|(l, _)| l == "Restart now")
            .expect("Restart now");
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
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: click,
                ..Default::default()
            },
            |ui| {
                let _ = toasts.center(ui);
            },
        );
        out.textures_delta.clear();
        motion::set_reduced(false);
        assert_eq!(toasts.show(&ctx), [Offer::Restart(PathBuf::from("canopy"))]);
        assert_eq!(
            toasts
                .history
                .iter()
                .map(|e| e.title.as_str())
                .collect::<Vec<_>>(),
            ["Pushed"],
            "taken, it leaves the center"
        );
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
    fn the_bell_count_is_danger_when_an_unread_notification_is_an_error() {
        let ctx = egui::Context::default();
        let p = theme::palette(&ctx);
        let mut toasts = Toasts::default();
        toasts.set_prefs(NotificationPrefs {
            errors: false,
            attention: false,
            ..Default::default()
        });
        assert!(toasts.bell_badge().is_none(), "nothing unread, no badge");
        toasts.push(&ctx, ToastKind::Info, Topic::Attention, "Decide", "");
        let (fill, ink) = toasts.bell_badge().expect("one unread").colors(p);
        assert_eq!((fill, ink), (p.s().primary, p.on_accent), "an accent count");
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Could not pull", "no");
        let (fill, ink) = toasts.bell_badge().expect("two unread").colors(p);
        assert_eq!((fill, ink), (p.s().danger_solid, p.on_danger), "an error");
        // A newer notification that is not counted does not hide the error.
        toasts.push(&ctx, ToastKind::Success, Topic::Finished, "Pushed", "");
        let (fill, _) = toasts.bell_badge().expect("still unread").colors(p);
        assert_eq!(
            fill,
            p.s().danger_solid,
            "an error under a newer notification"
        );
    }

    #[test]
    fn the_bell_badge_stays_inside_the_bar_that_clips_it() {
        let button = egui::Rect::from_min_size(egui::pos2(780.0, 580.0), vec2(16.0, 16.0));
        let size = vec2(14.0, 14.0);
        let bar = egui::Rect::from_min_max(egui::pos2(0.0, 577.0), egui::pos2(800.0, 600.0));
        let at = badge_origin(button, size, bar);
        assert!(
            bar.contains_rect(egui::Rect::from_min_size(at, size)),
            "{at:?}"
        );
        let roomy = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(900.0, 700.0));
        let at = badge_origin(button, size, roomy);
        assert_eq!(at, button.right_top() + vec2(-size.x * 0.4, -size.y * 0.3));
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

    #[test]
    fn the_center_survives_a_restart_without_what_cannot() {
        let ctx = egui::Context::default();
        let mut before = Toasts::default();
        before.push_for(
            &ctx,
            ToastKind::Error,
            Topic::Error,
            "Fetch failed",
            "no route",
            Some(Path::new("/work/canopy")),
        );
        before.push_offer(
            &ctx,
            ToastKind::Info,
            Topic::Attention,
            "Git decides",
            "pull.rebase is set",
            Offer::GitSettings,
        );
        before.push_offer(
            &ctx,
            ToastKind::Info,
            Topic::Update,
            "Canopy 9 is out",
            "",
            Offer::ReviewUpdate,
        );
        let mut after = Toasts::default();
        after.restore_json(&before.to_json());
        let kept: Vec<_> = after
            .history
            .iter()
            .map(|e| (e.title.as_str(), e.source.as_deref(), e.offer.clone()))
            .collect();
        assert_eq!(
            kept,
            [
                ("Canopy 9 is out", None, None),
                ("Git decides", None, Some(Offer::GitSettings)),
                ("Fetch failed", Some("canopy"), None),
            ],
            "the update to review is gone, the rest is back, newest first"
        );
        assert_eq!(
            after.unread, 1,
            "the unanswered Git settings offer still needs attention"
        );
        after.restore_json("not json");
        assert_eq!(after.history.len(), 3, "unreadable text is ignored");
    }

    #[test]
    fn unread_notifications_and_error_severity_survive_restart() {
        let ctx = egui::Context::default();
        let mut before = Toasts::default();
        before.set_prefs(NotificationPrefs {
            cards: false,
            ..Default::default()
        });
        before.push(&ctx, ToastKind::Error, Topic::Error, "failure", "why");
        before.push(&ctx, ToastKind::Info, Topic::Attention, "attention", "why");
        before.push(&ctx, ToastKind::Success, Topic::Finished, "finished", "");
        let mut after = Toasts::default();
        after.restore_json(&before.to_json());
        assert_eq!(after.unread, 2);
        assert_eq!(after.unread_errors, 1);
        assert!(after.items.is_empty(), "a restart does not replay cards");
        let read = Toasts::default();
        let mut empty = Toasts::default();
        empty.restore_json(&read.to_json());
        assert_eq!(empty.unread, 0);
    }

    #[test]
    fn a_notice_about_a_repository_is_kept_once() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        for _ in 0..2 {
            toasts.push_offer_for(
                &ctx,
                Topic::Attention,
                "Git settings differ",
                "Prune: x".into(),
                Offer::GitSettings,
                Path::new("/work/canopy"),
            );
        }
        toasts.push_offer_for(
            &ctx,
            Topic::Attention,
            "Git settings differ",
            "Prune: x".into(),
            Offer::GitSettings,
            Path::new("/work/other"),
        );
        assert_eq!(toasts.history.len(), 2, "once per repository");
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

    /// Git opens a clone with a progress line; the card must say why it failed, not that.
    #[test]
    fn a_long_notification_title_leaves_room_for_its_source_and_time() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut toasts = Toasts::default();
        let title = "Push failed for the feature branch with a long descriptive name";
        toasts.push_for(
            &ctx,
            ToastKind::Error,
            Topic::Error,
            title,
            "denied",
            Some(Path::new("/work/repository")),
        );
        let mut rects = Vec::new();
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_width(CENTER_WIDTH);
                let _ = toasts.center(ui);
            });
            out.textures_delta.clear();
            rects = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t)
                        if t.galley.text() == title
                            || t.galley.text().starts_with("repository  ") =>
                    {
                        Some((t.galley.text().to_owned(), t.visual_bounding_rect()))
                    }
                    _ => None,
                })
                .collect();
        }
        assert_eq!(rects.len(), 2, "{rects:?}");
        assert!(!rects[0].1.intersects(rects[1].1), "{rects:?}");
        assert!(
            rects.iter().all(|(_, r)| r.right() <= CENTER_WIDTH),
            "{rects:?}"
        );
    }

    #[test]
    fn a_failure_card_gives_the_remote_hook_reason_before_generic_push_errors() {
        let raw = "Enumerating objects: 27, done.\nremote: Counting objects: 100% (27/27), done.\nremote: error: Changes must be made through a pull request.\n! [remote rejected] main -> main (pre-receive hook declined)\nerror: failed to push some refs";
        let said = card_text(ToastKind::Error, raw, None);
        assert_eq!(
            said.body,
            Some("remote: error: Changes must be made through a pull request.")
        );
        assert_eq!(said.details, Some(raw));
        let raw = "Enumerating objects: 27, done.\nremote: Push denied by policy\nerror: failed to push some refs";
        assert_eq!(
            card_text(ToastKind::Error, raw, None).body,
            Some("remote: Push denied by policy")
        );
    }

    #[test]
    fn a_failure_card_gives_gits_fatal_line_not_its_progress_line() {
        let raw = "git submodule failed: Cloning into 'libs/three'...\n\
                   fatal: unable to access 'https://127.0.0.1:9/o/r.git/': Failed to connect";
        let said = card_text(ToastKind::Error, raw, None);
        assert_eq!(
            said.body,
            Some("fatal: unable to access 'https://127.0.0.1:9/o/r.git/': Failed to connect")
        );
        assert_eq!(said.details, Some(raw), "the tooltip keeps all of it");

        let plain = card_text(ToastKind::Error, "something went wrong\nand more", None);
        assert_eq!(plain.body, Some("something went wrong"));
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
    fn only_a_failure_in_the_center_offers_copy() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        motion::set_reduced(true);
        let mut toasts = Toasts::default();
        toasts.push(&ctx, ToastKind::Success, Topic::Finished, "Pushed", "main");
        toasts.push(&ctx, ToastKind::Info, Topic::Confirmation, "Copied", "abc");
        let copies = |toasts: &mut Toasts| {
            let mut n = 0;
            for _ in 0..3 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let _ = toasts.center(ui);
                });
                out.textures_delta.clear();
                n = out.platform_output.accesskit_update.map_or(0, |u| {
                    u.nodes
                        .iter()
                        .filter(|(_, n)| n.label() == Some("Copy"))
                        .count()
                });
            }
            n
        };
        assert_eq!(copies(&mut toasts), 0);
        toasts.push(&ctx, ToastKind::Error, Topic::Error, "Fetch failed", "no");
        assert_eq!(copies(&mut toasts), 1);
        motion::set_reduced(false);
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
