//! Small labels: [`Badge`], [`CountBadge`], [`ChangeBadge`], [`RefPill`] and [`RefOverflow`],
//! [`Keycap`] and [`KeyChord`], [`StatusDot`], [`DiffStat`] and the interactive status [`Chip`]
//! (design spec 4.17, 4.18).
//!
//! Everything here but [`Chip`] (and a ref pill given `.clickable()`) is read-only: it senses
//! hover for its tooltip and reports itself to screen readers as a label.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Galley, Id, Painter, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, Ui, Vec2, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::{ChangeKind, RefKind, RefLabel};

use super::{ForceState, Interact, Tone, paint};
use crate::keymap::{self, Command};
use crate::ui::motion;
use crate::ui::theme::{self, Density, FULL, Fill, Palette};
use crate::ui::widgets;

/// A radius that `FULL` turns into half the height.
fn radius_for(radius: u8, height: f32) -> CornerRadius {
    if radius == FULL {
        CornerRadius::same(crate::ui::saturate_u8(
            (height / 2.0).round().min(f32::from(u8::MAX)),
        ))
    } else {
        CornerRadius::same(radius)
    }
}

fn layout(ui: &Ui, text: String, font: FontId) -> Arc<Galley> {
    ui.painter()
        .layout_no_wrap(text, font, Color32::PLACEHOLDER)
}

/// A read-only label's response: its tooltip and its AccessKit label.
fn label_response(response: Response, label: &str, tooltip: Option<&str>) -> Response {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
    match tooltip {
        Some(t) => response.on_hover_text(t),
        None => response,
    }
}

// -------------------------------------------------------------------------------------- Badge

/// A small rounded count or status label. Neutral by default; a [`Tone`] tints it.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Badge {
    text: String,
    tone: Tone,
    mono: bool,
    strong: bool,
    tooltip: Option<String>,
}

impl Badge {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Neutral,
            mono: false,
            strong: false,
            tooltip: None,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Monospaced digits (counts, SHAs).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The `primary` fill with `on_accent` text (the view switcher's counts).
    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    fn font(&self) -> FontId {
        let mono = self.mono || self.tone == Tone::Accent;
        let family = if mono {
            FontFamily::Monospace
        } else {
            theme::semibold()
        };
        FontId::new(theme::metrics().micro_size, family)
    }

    /// The badge's fill and text colours.
    pub fn colors(&self, p: &Palette) -> (Fill, Color32) {
        let s = p.s();
        if self.strong {
            return (s.primary, p.on_accent);
        }
        match self.tone {
            Tone::Neutral => (Fill::flat(s.badge), p.text),
            Tone::Danger => {
                let fill = p.danger_badge_fill();
                (Fill::flat(fill), p.ink(p.danger, fill))
            }
            tone => {
                let c = tone.color(p);
                let fill = Palette::badge_fill(c);
                (Fill::flat(fill), p.ink(c, fill))
            }
        }
    }

    /// Its size: `badge_height` tall, at least as wide, 5 pt either side of the text.
    pub fn measure(&self, ui: &Ui) -> Vec2 {
        let m = theme::metrics();
        let g = layout(ui, self.text.clone(), self.font());
        vec2((g.size().x + 10.0).max(m.badge_height), m.badge_height)
    }

    /// Paints the badge in `rect` (sized by [`Badge::measure`]).
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        let p = theme::palette(ui.ctx());
        let (fill, ink) = self.colors(p);
        let g = layout(ui, self.text.clone(), self.font());
        let painter = ui.painter();
        paint::fill(painter, rect, CornerRadius::same(9), fill);
        painter.galley(rect.center() - g.size() / 2.0, g, ink);
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let size = self.measure(ui);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui, rect);
        }
        label_response(response, &self.text, self.tooltip.as_deref())
    }
}

impl Widget for Badge {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// A [`Badge`] with a count: mono digits, capped at `99+`.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct CountBadge {
    badge: Badge,
}

impl CountBadge {
    pub fn new(count: usize) -> Self {
        Self {
            badge: Badge::new(count_text(count)).mono(),
        }
    }
    pub fn tone(mut self, tone: Tone) -> Self {
        self.badge = self.badge.tone(tone);
        self
    }
    pub fn strong(mut self) -> Self {
        self.badge = self.badge.strong();
        self
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.badge = self.badge.tooltip(text);
        self
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        self.badge.show(ui)
    }
}

impl Widget for CountBadge {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// A count as a badge shows it.
pub fn count_text(count: usize) -> String {
    if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    }
}

// -------------------------------------------------------------------------------- ChangeBadge

/// A file's status letter (A, M, D, R, C, U) on an 18% tint of its colour.
#[derive(Debug, Clone, Copy)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct ChangeBadge {
    kind: ChangeKind,
}

impl ChangeBadge {
    pub fn new(kind: ChangeKind) -> Self {
        Self { kind }
    }

    /// The badge's fill and letter colours.
    pub fn colors(self, p: &Palette) -> (Color32, Color32) {
        let c = widgets::change_style(p, self.kind);
        let fill = Palette::change_fill(c);
        (fill, p.ink(c, fill))
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = theme::palette(ui.ctx());
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(17.0), Sense::hover());
        if ui.is_rect_visible(rect) {
            let (fill, ink) = self.colors(p);
            ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                self.kind.letter(),
                FontId::new(10.0, FontFamily::Monospace),
                ink,
            );
        }
        let tip = match self.kind {
            ChangeKind::Untracked => "Untracked: a new file Git does not track yet",
            other => other.label(),
        };
        label_response(response, tip, Some(tip))
    }
}

impl Widget for ChangeBadge {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------ RefPill

/// A branch, tag or HEAD label beside a commit. Colour: the branch's lane (local and remote), the
/// tag colour `warning`. Read-only unless [`RefPill::clickable`].
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct RefPill {
    name: String,
    kind: RefKind,
    lane: u16,
    upstream: bool,
    max_width: Option<f32>,
    clickable: bool,
    tooltip: Option<String>,
    force: Option<ForceState>,
}

impl RefPill {
    pub fn new(name: impl Into<String>, kind: RefKind) -> Self {
        Self {
            name: name.into(),
            kind,
            lane: 0,
            upstream: false,
            max_width: None,
            clickable: false,
            tooltip: None,
            force: None,
        }
    }

    /// A pill for a ref label the history shows, in its commit's lane colour.
    pub fn from_label(label: &RefLabel, lane: u16) -> Self {
        Self::new(label.name.clone(), label.kind).lane(lane)
    }

    /// The graph colour index of the branch's lane.
    pub fn lane(mut self, lane: u16) -> Self {
        self.lane = lane;
        self
    }

    /// A local branch merged with its upstream into one pill: both icons.
    pub fn upstream(mut self, merged: bool) -> Self {
        self.upstream = merged;
        self
    }

    /// Ellipsizes the label past this width.
    pub fn max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }

    /// Senses clicks (History opens the ref's context menu from it).
    pub fn clickable(mut self) -> Self {
        self.clickable = true;
        self
    }

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// The pill's colour (spec 4.17).
    pub fn color(&self, p: &Palette) -> Color32 {
        match self.kind {
            RefKind::Head | RefKind::Stash => p.text_dim,
            RefKind::CurrentBranch | RefKind::LocalBranch | RefKind::RemoteBranch => {
                p.lane(self.lane)
            }
            RefKind::Tag => p.warning,
        }
    }

    fn glyphs(&self) -> Vec<&'static str> {
        let main = match self.kind {
            RefKind::Head => return Vec::new(),
            RefKind::CurrentBranch => icon::RECORD,
            RefKind::LocalBranch => icon::GIT_BRANCH,
            RefKind::RemoteBranch => icon::CLOUD,
            RefKind::Tag => icon::TAG,
            RefKind::Stash => icon::ARCHIVE,
        };
        if self.upstream && matches!(self.kind, RefKind::CurrentBranch | RefKind::LocalBranch) {
            vec![main, icon::CLOUD]
        } else {
            vec![main]
        }
    }

    /// Fill, ring and label colours.
    pub fn colors(&self, p: &Palette) -> (Color32, Color32, Color32) {
        let s = p.s();
        let c = self.color(p);
        match self.kind {
            RefKind::Head => (s.elevated, s.edge, p.text_dim),
            RefKind::RemoteBranch => (Color32::TRANSPARENT, Palette::pill_border(c), p.text_dim),
            RefKind::CurrentBranch => {
                let fill = Palette::pill_fill(c, true);
                (fill, Palette::pill_border(c), p.ink(p.text, fill))
            }
            _ => {
                let fill = Palette::pill_fill(c, false);
                (fill, Palette::pill_border(c), p.ink(p.text, fill))
            }
        }
    }

    fn label_font(&self) -> FontId {
        let m = theme::metrics();
        match self.kind {
            RefKind::Head => FontId::new(10.5, FontFamily::Monospace),
            RefKind::CurrentBranch => FontId::new(m.small_size, theme::semibold()),
            _ => FontId::new(m.small_size, theme::medium()),
        }
    }

    fn text(&self) -> &str {
        if self.kind == RefKind::Head {
            "HEAD"
        } else {
            &self.name
        }
    }

    fn label_galley(&self, ui: &Ui, ink: Color32) -> Arc<Galley> {
        let width = self.max_width.map_or(f32::INFINITY, |w| {
            (w - self.chrome_width(ui)).max(theme::LG)
        });
        widgets::single_line_galley(ui, self.text().to_owned(), self.label_font(), ink, width)
    }

    /// Everything but the label: padding, icons and their gaps.
    fn chrome_width(&self, ui: &Ui) -> f32 {
        let m = theme::metrics();
        let glyphs = self.glyphs();
        if glyphs.is_empty() {
            return 2.0 * 7.0;
        }
        let icon_w: f32 = glyphs
            .iter()
            .map(|g| {
                layout(ui, (*g).to_owned(), FontId::proportional(m.icon_xs))
                    .size()
                    .x
            })
            .sum();
        m.ref_pill_pad_icon + icon_w + GAP * crate::ui::count_f32(glyphs.len()) + m.ref_pill_pad
    }

    /// The pill's width (the history sizes its description column from it).
    pub fn width(&self, ui: &Ui) -> f32 {
        let p = theme::palette(ui.ctx());
        let g = self.label_galley(ui, self.colors(p).2);
        self.chrome_width(ui) + g.size().x
    }

    /// Paints the pill with its left edge at `left`, centred on `cy`; returns its rect.
    pub fn paint(&self, ui: &Ui, painter: &Painter, left: f32, cy: f32) -> Rect {
        let p = theme::palette(ui.ctx());
        let m = theme::metrics();
        let (fill, ring, ink) = self.colors(p);
        let g = self.label_galley(ui, ink);
        let rect = Rect::from_min_size(
            pos2(left, cy - m.ref_pill_height / 2.0),
            vec2(self.chrome_width(ui) + g.size().x, m.ref_pill_height),
        );
        let radius = radius_for(m.pill_radius, m.ref_pill_height);
        painter.rect(
            rect,
            radius,
            fill,
            Stroke::new(1.0, ring),
            StrokeKind::Inside,
        );
        let glyphs = self.glyphs();
        let mut x = rect.left()
            + if glyphs.is_empty() {
                7.0
            } else {
                m.ref_pill_pad_icon
            };
        let c = self.color(p);
        for glyph in glyphs {
            let gg = painter.layout_no_wrap(glyph.to_owned(), FontId::proportional(m.icon_xs), c);
            painter.galley(pos2(x, cy - gg.size().y / 2.0), Arc::clone(&gg), c);
            x += gg.size().x + GAP;
        }
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, ink);
        rect
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let m = theme::metrics();
        let width = self.width(ui);
        let sense = if self.clickable {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, m.ref_pill_height), sense);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter().clone();
            self.paint(ui, &painter, rect.left(), rect.center().y);
            if self.clickable {
                let p = theme::palette(ui.ctx());
                let st = Interact::of(ui.ctx(), &response, true, self.force);
                let radius = radius_for(m.pill_radius, m.ref_pill_height);
                let over = Color32::TRANSPARENT
                    .lerp_to_gamma(p.hover(), st.hover)
                    .lerp_to_gamma(p.pressed(), st.press);
                if over.a() > 0 {
                    painter.rect_filled(rect, radius, over);
                }
                if st.focus > 0.0 {
                    paint::focus_ring(
                        &painter,
                        rect,
                        radius,
                        p.focus.gamma_multiply(st.focus),
                        Color32::TRANSPARENT,
                    );
                }
            }
        }
        let spoken = match self.kind {
            RefKind::Head => "Detached HEAD".to_owned(),
            RefKind::CurrentBranch => format!("{} (current branch)", self.name),
            RefKind::Tag => format!("Tag {}", self.name),
            _ => self.name.clone(),
        };
        if self.clickable {
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &spoken));
            match &self.tooltip {
                Some(t) => response.on_hover_text(t),
                None => response,
            }
        } else {
            label_response(response, &spoken, self.tooltip.as_deref())
        }
    }
}

impl Widget for RefPill {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// Space between a ref pill's icon and its label.
const GAP: f32 = 4.0;

/// The `+N` badge for refs that did not fit; its tooltip lists them.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct RefOverflow {
    hidden: Vec<String>,
}

impl RefOverflow {
    pub fn new(hidden: Vec<String>) -> Self {
        Self { hidden }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let tip = self.hidden.join("\n");
        Badge::new(format!("+{}", self.hidden.len()))
            .tooltip(tip)
            .show(ui)
    }
}

impl Widget for RefOverflow {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ---------------------------------------------------------------------------- Keycap, KeyChord

/// One key as a cap: `Ctrl`, `P`, `↵`.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Keycap {
    label: String,
}

impl Keycap {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }

    fn galley(&self, ui: &Ui) -> Arc<Galley> {
        let m = theme::metrics();
        layout(
            ui,
            self.label.clone(),
            FontId::new(m.chord_size, FontFamily::Monospace),
        )
    }

    pub fn measure(&self, ui: &Ui) -> Vec2 {
        let m = theme::metrics();
        let g = self.galley(ui);
        vec2((g.size().x + 10.0).max(m.keycap_height), m.keycap_height)
    }

    /// Paints the cap in `rect` (sized by [`Keycap::measure`]).
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        let p = theme::palette(ui.ctx());
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let painter = ui.painter();
        let radius = CornerRadius::same(m.keycap_radius);
        paint::fill(painter, rect, radius, s.keycap);
        // The inner bottom line sits inside the 1 pt ring.
        let inset = f32::from(m.keycap_radius);
        paint::seam_h(
            painter,
            egui::Rangef::new(rect.left() + inset, rect.right() - inset),
            rect.bottom() - 1.5,
            s.keycap_edge,
        );
        paint::rim(painter, rect, radius, s.edge);
        let g = self.galley(ui);
        painter.galley(rect.center() - g.size() / 2.0, g, p.text_dim);
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let size = self.measure(ui);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui, rect);
        }
        label_response(response, &self.label, None)
    }
}

impl Widget for Keycap {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// Splits a chord label (`Ctrl+Shift+P`, `Ctrl++`) into its keys.
pub fn chord_keys(chord: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut current = String::new();
    for c in chord.chars() {
        if c == '+' && !current.is_empty() {
            keys.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        keys.push(current);
    }
    keys
}

/// Splits a keys label (`Ctrl+K  ·  Ctrl+Shift+P`, `g g`) into alternatives, each a sequence of
/// chords, each a list of keys.
pub fn parse_keys(text: &str) -> Vec<Vec<Vec<String>>> {
    text.split('·')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|alt| alt.split_whitespace().map(chord_keys).collect())
        .collect()
}

/// A shortcut as keycaps: keys of a chord 3 apart, chords of a sequence 6 apart, alternatives
/// separated by a faint `·`. Labels come from `keymap.rs`.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct KeyChord {
    keys: Vec<Vec<Vec<String>>>,
    spoken: String,
}

impl KeyChord {
    /// The keys of `command` as the keymap lists them.
    pub fn command(command: Command) -> Self {
        Self::new(keymap::hint(command))
    }

    /// Keys written as the keymap writes them (`Ctrl+Shift+P · f`).
    pub fn new(text: impl Into<String>) -> Self {
        let spoken = text.into();
        Self {
            keys: parse_keys(&spoken),
            spoken,
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = theme::palette(ui.ctx());
        let m = theme::metrics();
        let dot = layout(ui, "·".to_owned(), FontId::proportional(m.small_size));
        let mut items: Vec<(Option<Keycap>, f32)> = Vec::new();
        for (a, alt) in self.keys.iter().enumerate() {
            if a > 0 {
                items.push((None, 6.0));
            }
            for (c, chord) in alt.iter().enumerate() {
                for (k, key) in chord.iter().enumerate() {
                    let gap = if k > 0 {
                        3.0
                    } else if c > 0 {
                        6.0
                    } else {
                        0.0
                    };
                    items.push((Some(Keycap::new(key.clone())), gap));
                }
            }
        }
        let widths: Vec<f32> = items
            .iter()
            .map(|(cap, gap)| gap + cap.as_ref().map_or(dot.size().x + 6.0, |k| k.measure(ui).x))
            .collect();
        let size = vec2(widths.iter().sum(), m.keycap_height);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            let mut x = rect.left();
            for ((cap, gap), w) in items.iter().zip(&widths) {
                x += gap;
                match cap {
                    Some(k) => {
                        let at = Rect::from_min_size(pos2(x, rect.top()), k.measure(ui));
                        k.paint(ui, at);
                    }
                    None => {
                        ui.painter().galley(
                            pos2(x, rect.center().y - dot.size().y / 2.0),
                            Arc::clone(&dot),
                            p.text_faint,
                        );
                    }
                }
                x += w - gap;
            }
        }
        label_response(response, &self.spoken, None)
    }
}

impl Widget for KeyChord {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ---------------------------------------------------------------------------------- StatusDot

/// A small status circle with a ring of the surface behind it; `.pulse()` breathes while
/// something runs (still with reduced motion).
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct StatusDot {
    tone: Tone,
    pulse: bool,
    ground: Option<Color32>,
    label: String,
}

impl StatusDot {
    pub fn new(tone: Tone, label: impl Into<String>) -> Self {
        Self {
            tone,
            pulse: false,
            ground: None,
            label: label.into(),
        }
    }

    /// Breathes (alpha 0.5 to 1 over 1.6 s) while shown.
    pub fn pulse(mut self, on: bool) -> Self {
        self.pulse = on;
        self
    }

    /// The surface it sits on (default `pane`).
    pub fn ground(mut self, ground: Color32) -> Self {
        self.ground = Some(ground);
        self
    }

    /// The dot's diameter: 8, 6 in Dense.
    pub fn diameter() -> f32 {
        if theme::density() == Density::Dense {
            6.0
        } else {
            8.0
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = theme::palette(ui.ctx());
        let d = Self::diameter();
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(d + 4.0), Sense::hover());
        if ui.is_rect_visible(rect) {
            let ground = self.ground.unwrap_or(theme::surfaces(ui.ctx()).pane);
            let alpha = if self.pulse {
                pulse_alpha(ui.ctx())
            } else {
                1.0
            };
            let c = rect.center();
            ui.painter().circle_filled(c, d / 2.0 + 2.0, ground);
            ui.painter()
                .circle_filled(c, d / 2.0, self.tone.color(p).gamma_multiply(alpha));
        }
        label_response(response, &self.label, Some(&self.label))
    }
}

impl Widget for StatusDot {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The breathing alpha of a pulsing dot now (0.5 to 1 over 1.6 s), repainting while shown; 1 and
/// still with reduced motion.
fn pulse_alpha(ctx: &egui::Context) -> f32 {
    if motion::reduced() {
        return 1.0;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    let t = crate::ui::to_f32(ctx.input(|i| i.time) % 1.6) / 1.6;
    0.75 + 0.25 * (t * std::f32::consts::TAU).cos()
}

// ----------------------------------------------------------------------------------- DiffStat

/// Lines added and removed: `+12 −3` in `success` and `danger`, with an optional five-block bar.
#[derive(Debug, Clone, Copy)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct DiffStat {
    added: usize,
    removed: usize,
    bar: bool,
}

impl DiffStat {
    pub fn new(added: usize, removed: usize) -> Self {
        Self {
            added,
            removed,
            bar: false,
        }
    }

    /// Adds the five-block bar after the counts.
    pub fn bar(mut self, on: bool) -> Self {
        self.bar = on;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = theme::palette(ui.ctx());
        let s = theme::surfaces(ui.ctx());
        let font = FontId::new(theme::metrics().sha_size, FontFamily::Monospace);
        let mut job = LayoutJob::default();
        let fmt = |c: Color32| TextFormat::simple(font.clone(), c);
        job.append(&format!("+{}", self.added), 0.0, fmt(p.success));
        job.append(
            &format!("−{}", self.removed),
            6.0,
            fmt(p.danger.gamma_multiply(0.85)),
        );
        let g = ui.painter().layout_job(job);
        let bar_w = if self.bar {
            6.0 + 5.0 * 6.0 + 4.0 * 2.0
        } else {
            0.0
        };
        let (rect, response) =
            ui.allocate_exact_size(vec2(g.size().x + bar_w, g.size().y), Sense::hover());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            painter.galley(rect.left_top(), Arc::clone(&g), p.text);
            if self.bar {
                let (green, red, _) = blocks(self.added, self.removed);
                let mut x = rect.left() + g.size().x + 6.0;
                for i in 0..5 {
                    let c = if i < green {
                        p.success
                    } else if i < green + red {
                        p.danger
                    } else {
                        s.switch_off
                    };
                    let b =
                        Rect::from_center_size(pos2(x + 3.0, rect.center().y), Vec2::splat(6.0));
                    painter.rect_filled(b, CornerRadius::same(2), c);
                    x += 8.0;
                }
            }
        }
        let spoken = format!(
            "{} addition{}, {} deletion{}",
            self.added,
            if self.added == 1 { "" } else { "s" },
            self.removed,
            if self.removed == 1 { "" } else { "s" }
        );
        label_response(response, &spoken, None)
    }
}

impl Widget for DiffStat {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The five blocks of a diffstat bar: (added, removed, neutral). Up to five lines fill one block
/// each; more fill all five in proportion, and a side with any change keeps at least one block.
pub fn blocks(added: usize, removed: usize) -> (usize, usize, usize) {
    let total = added + removed;
    if total == 0 {
        return (0, 0, 5);
    }
    let filled = total.min(5);
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "line counts are exact in f64, and the rounded share is between 0 and 5"
    )]
    let mut green = ((added * filled) as f64 / total as f64).round() as usize;
    if added > 0 {
        green = green.max(1);
    }
    if removed > 0 {
        green = green.min(filled - 1);
    }
    let red = filled - green;
    (green, red, 5 - filled)
}

// --------------------------------------------------------------------------------------- Chip

/// An interactive status chip (the status bar's trust, restart and update chips): a tinted pill
/// with an icon and a label, the ghost hover and press, and a focus ring.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Chip {
    text: String,
    icon: Option<String>,
    tone: Tone,
    enabled: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl Chip {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            tone: Tone::Neutral,
            enabled: true,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            force: None,
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// The chip's fill (the tone's tint; none when neutral) and its label colour.
    pub fn colors(&self, p: &Palette) -> (Color32, Color32) {
        match self.tone {
            Tone::Neutral => (Color32::TRANSPARENT, p.text_dim),
            tone => {
                let fill = tone.tint(p);
                (fill, p.ink(tone.color(p), fill))
            }
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let font = FontId::new(m.small_size, theme::medium());
        let text = layout(ui, self.text.clone(), font);
        let glyph = self
            .icon
            .as_ref()
            .map(|g| layout(ui, g.clone(), FontId::proportional(m.icon_xs)));
        let height = m.keycap_height;
        let gap = 5.0;
        let content = text.size().x + glyph.as_ref().map_or(0.0, |g| g.size().x + gap);
        let size = vec2(content + 2.0 * theme::SM, height);
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = match self.id_salt {
            Some(id) => {
                let rect = ui.allocate_space(size).1;
                (rect, ui.interact(rect, ui.id().with(id), sense))
            }
            None => ui.allocate_exact_size(size, sense),
        };
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            let mut painter = ui.painter().clone();
            if !enabled && ui.is_enabled() {
                painter.multiply_opacity(theme::DISABLED_ALPHA);
            }
            let radius = radius_for(FULL, height);
            let (fill, ink) = self.colors(p);
            if fill.a() > 0 {
                painter.rect_filled(rect, radius, fill);
            }
            let over = Color32::TRANSPARENT
                .lerp_to_gamma(p.hover_strong(), st.hover)
                .lerp_to_gamma(p.pressed(), st.press);
            if over.a() > 0 {
                painter.rect_filled(rect, radius, over);
            }
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    rect,
                    radius,
                    p.focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            let (painter, ink) = if !enabled {
                (
                    super::content_painter(ui),
                    super::disabled_text(p, theme::style()),
                )
            } else if self.tone == Tone::Neutral {
                (painter, ink.lerp_to_gamma(p.text, st.hover.max(st.press)))
            } else {
                (painter, ink)
            };
            let mut x = rect.left() + theme::SM;
            let mut shapes: Vec<Shape> = Vec::new();
            if let Some(g) = glyph {
                shapes.push(Shape::galley(
                    pos2(x, rect.center().y - g.size().y / 2.0),
                    Arc::clone(&g),
                    ink,
                ));
                x += g.size().x + gap;
            }
            shapes.push(Shape::galley(
                pos2(x, rect.center().y - text.size().y / 2.0),
                text,
                ink,
            ));
            painter.extend(shapes);
        }
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &self.text));
        let tip = if enabled {
            self.tooltip
        } else {
            self.disabled_reason.or(self.tooltip)
        };
        match tip {
            Some(t) => response.on_hover_text(&t).on_disabled_hover_text(&t),
            None => response,
        }
    }
}

impl Widget for Chip {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------ gallery

/// The gallery's Chips section: ref pills, badges per tone, counts, change letters, keycaps and
/// chords, status dots, diffstats and status chips.
pub(crate) fn gallery(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let small = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .size(m.small_size)
                .color(p.text_faint),
        );
    };
    let row = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui)| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, theme::SM);
            f(ui);
        });
    };
    small(
        ui,
        "Ref pills: branch, current, remote, tag, HEAD, merged, +N, clickable",
    );
    row(ui, &mut |ui| {
        RefPill::from_label(
            &RefLabel {
                name: "feature/auth".into(),
                kind: RefKind::LocalBranch,
            },
            1,
        )
        .show(ui);
        RefPill::new("main", RefKind::CurrentBranch)
            .lane(0)
            .show(ui);
        RefPill::new("origin/main", RefKind::RemoteBranch)
            .lane(0)
            .show(ui);
        RefPill::new("v1.0.0", RefKind::Tag)
            .tooltip("Annotated tag")
            .show(ui);
        RefPill::new("HEAD", RefKind::Head).show(ui);
        RefPill::new("release/1.0", RefKind::LocalBranch)
            .lane(2)
            .upstream(true)
            .show(ui);
        RefOverflow::new(vec!["origin/feature/a".into(), "v0.9.0".into()]).show(ui);
        RefPill::new(
            "feature/a-very-long-branch-name-indeed",
            RefKind::LocalBranch,
        )
        .lane(3)
        .max_width(150.0)
        .show(ui);
        RefPill::new("feature/graph", RefKind::LocalBranch)
            .lane(4)
            .clickable()
            .show_forced(ui, ForceState::HOVER);
        RefPill::new("fix/typo", RefKind::LocalBranch)
            .lane(5)
            .clickable()
            .show_forced(ui, ForceState::FOCUS);
    });
    small(ui, "Badges per tone, strong, counts (1, 42, 99+)");
    row(ui, &mut |ui| {
        for tone in Tone::ALL {
            Badge::new(tone.label()).tone(tone).show(ui);
        }
        Badge::new("12").strong().mono().show(ui);
        for n in [1, 42, 140] {
            CountBadge::new(n).show(ui);
        }
        CountBadge::new(4).tone(Tone::Accent).show(ui);
        CountBadge::new(3).tone(Tone::Danger).show(ui);
        CountBadge::new(7)
            .strong()
            .tooltip("7 changed files")
            .show(ui);
    });
    small(ui, "Change letters");
    row(ui, &mut |ui| {
        for kind in [
            ChangeKind::Added,
            ChangeKind::Modified,
            ChangeKind::Deleted,
            ChangeKind::Renamed,
            ChangeKind::Typechange,
            ChangeKind::Untracked,
            ChangeKind::Conflicted,
        ] {
            ChangeBadge::new(kind).show(ui);
        }
    });
    small(ui, "Keycaps and chords");
    row(ui, &mut |ui| {
        Keycap::new("Esc").show(ui);
        Keycap::new("↵").show(ui);
        KeyChord::new("Ctrl+Shift+P").show(ui);
        KeyChord::command(Command::Palette).show(ui);
        KeyChord::new("g g").show(ui);
        KeyChord::new("Ctrl++").show(ui);
    });
    small(ui, "Status dots (static, pulsing), diffstats");
    row(ui, &mut |ui| {
        for tone in Tone::ALL {
            StatusDot::new(tone, tone.label()).show(ui);
        }
        StatusDot::new(Tone::Accent, "Running").pulse(true).show(ui);
        StatusDot::new(Tone::Success, "On the chrome")
            .ground(theme::surfaces(ui.ctx()).panel)
            .show(ui);
        ui.add_space(theme::MD);
        DiffStat::new(12, 3).show(ui);
        DiffStat::new(12, 3).bar(true).show(ui);
        DiffStat::new(2, 0).bar(true).show(ui);
        DiffStat::new(0, 40).bar(true).show(ui);
    });
    small(
        ui,
        "Status chips: neutral, warning, accent, hover, focus, disabled",
    );
    row(ui, &mut |ui| {
        Chip::new("4 changed")
            .icon(icon::PENCIL_SIMPLE)
            .id_salt("gc0")
            .show(ui);
        Chip::new("Downloads off · Trust…")
            .icon(icon::SHIELD_WARNING)
            .tone(Tone::Warning)
            .tooltip("Trust this folder")
            .id_salt("gc1")
            .show(ui);
        Chip::new("Restart to update")
            .icon(icon::ARROW_CLOCKWISE)
            .tone(Tone::Accent)
            .id_salt("gc2")
            .show(ui);
        Chip::new("Hover")
            .icon(icon::GIT_BRANCH)
            .id_salt("gc3")
            .show_forced(ui, ForceState::HOVER);
        Chip::new("Focus")
            .tone(Tone::Success)
            .id_salt("gc4")
            .show_forced(ui, ForceState::FOCUS);
        Chip::new("Disabled")
            .enabled(false)
            .disabled_reason("Nothing to do")
            .id_salt("gc5")
            .show(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Style;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, mut f: impl FnMut(&mut Ui)) {
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
    }

    #[test]
    fn counts_cap_at_99() {
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(99), "99");
        assert_eq!(count_text(100), "99+");
    }

    #[test]
    fn chords_split_into_keys() {
        assert_eq!(chord_keys("Ctrl+Shift+P"), ["Ctrl", "Shift", "P"]);
        assert_eq!(chord_keys("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(chord_keys("+"), ["+"]);
        assert_eq!(
            parse_keys("Ctrl+K  ·  Ctrl+Shift+P"),
            vec![
                vec![vec!["Ctrl".to_owned(), "K".to_owned()]],
                vec![vec!["Ctrl".to_owned(), "Shift".to_owned(), "P".to_owned()]],
            ]
        );
        assert_eq!(parse_keys("g g").first().map(Vec::len), Some(2));
    }

    #[test]
    fn keymap_chords_never_show_super() {
        for b in keymap::BINDINGS {
            for alt in parse_keys(&b.keys()) {
                for chord in alt {
                    for key in chord {
                        assert!(!matches!(key.as_str(), "Super" | "Win" | "Meta"), "{key}");
                    }
                }
            }
        }
    }

    #[test]
    fn diffstat_blocks_fill_in_proportion() {
        assert_eq!(blocks(0, 0), (0, 0, 5));
        assert_eq!(blocks(2, 0), (2, 0, 3));
        assert_eq!(blocks(12, 3), (4, 1, 0));
        assert_eq!(blocks(100, 1), (4, 1, 0));
        assert_eq!(blocks(1, 100), (1, 4, 0));
        assert_eq!(blocks(0, 40), (0, 5, 0));
        for a in 0..30 {
            for r in 0..30 {
                let (g, d, n) = blocks(a, r);
                assert_eq!(g + d + n, 5);
                assert_eq!(g > 0, a > 0, "{a} {r}");
                assert_eq!(d > 0, r > 0, "{a} {r}");
            }
        }
    }

    #[test]
    fn coloured_labels_read_on_their_fills() {
        for id in theme::ThemeId::ALL {
            let p = id.palette();
            for style in Style::ALL {
                theme::with_look(style, Density::Comfortable, || {
                    let s = p.surfaces(style);
                    let grounds = [s.pane, s.panel];
                    for tone in Tone::ALL {
                        let (fill, ink) = Badge::new("1").tone(tone).colors(p);
                        for g in grounds {
                            let behind = theme::over(fill.top, g);
                            assert!(
                                theme::contrast(ink, behind) >= 4.5,
                                "{} {style:?} badge {tone:?}",
                                p.name
                            );
                        }
                        let (fill, ink) = Chip::new("").tone(tone).colors(p);
                        for g in grounds {
                            assert!(
                                theme::contrast(ink, theme::over(fill, g)) >= 4.5,
                                "{} {style:?} chip {tone:?}",
                                p.name
                            );
                        }
                    }
                    for kind in [RefKind::LocalBranch, RefKind::CurrentBranch, RefKind::Tag] {
                        for lane in 0..10 {
                            let pill = RefPill::new("x", kind).lane(lane);
                            let (fill, _, ink) = pill.colors(p);
                            for g in grounds {
                                assert!(
                                    theme::contrast(ink, theme::over(fill, g)) >= 4.5,
                                    "{} {style:?} pill {kind:?} lane {lane}",
                                    p.name
                                );
                            }
                        }
                    }
                });
            }
        }
    }

    #[test]
    fn pills_follow_the_metrics_in_every_density() {
        let ctx = ctx();
        for density in Density::ALL {
            for style in Style::ALL {
                theme::apply(
                    &ctx,
                    theme::ThemeId::Midnight,
                    theme::Appearance {
                        style,
                        density,
                        ..theme::Appearance::STILL
                    },
                );
                let m = theme::metrics();
                let mut heights = Vec::new();
                frame(&ctx, Vec::new(), |ui| {
                    heights = vec![
                        RefPill::new("main", RefKind::CurrentBranch)
                            .show(ui)
                            .rect
                            .height(),
                        Badge::new("3").show(ui).rect.height(),
                        Keycap::new("K").show(ui).rect.height(),
                    ];
                });
                assert_eq!(
                    heights,
                    vec![m.ref_pill_height, m.badge_height, m.keycap_height],
                    "{density:?} {style:?}"
                );
            }
        }
    }

    #[test]
    fn a_chip_clicks_from_the_keyboard_and_not_when_disabled() {
        let ctx = ctx();
        for enabled in [true, false] {
            let mut clicked = false;
            for i in 0..3 {
                let events = if i == 1 {
                    vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::default(),
                    }]
                } else {
                    Vec::new()
                };
                frame(&ctx, events, |ui| {
                    let r = Chip::new("Trust")
                        .enabled(enabled)
                        .id_salt("chip-test")
                        .show(ui);
                    if i == 0 {
                        ui.memory_mut(|m| m.request_focus(r.id));
                    }
                    clicked |= r.clicked();
                });
            }
            assert_eq!(clicked, enabled, "enabled {enabled}");
        }
    }

    #[test]
    fn labels_reach_screen_readers() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            RefPill::new("main", RefKind::CurrentBranch).show(ui);
            DiffStat::new(1, 2).show(ui);
            KeyChord::new("Ctrl+K").show(ui);
            Chip::new("Restart to update").show(ui);
        });
        out.textures_delta.clear();
        let labels: Vec<String> = out
            .platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| n.label().or(n.value()).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        for want in [
            "main (current branch)",
            "1 addition, 2 deletions",
            "Ctrl+K",
            "Restart to update",
        ] {
            assert!(labels.iter().any(|l| l == want), "{want}: {labels:?}");
        }
    }

    #[test]
    fn pulsing_dots_hold_still_with_reduced_motion() {
        let ctx = ctx();
        assert_eq!(pulse_alpha(&ctx), 1.0);
    }

    #[test]
    fn keycaps_are_at_least_square() {
        let ctx = ctx();
        frame(&ctx, Vec::new(), |ui| {
            let m = theme::metrics();
            assert_eq!(Keycap::new("K").measure(ui).x, m.keycap_height);
            assert!(Keycap::new("Shift").measure(ui).x > m.keycap_height);
        });
    }
}
