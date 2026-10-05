//! [`Segmented`] (with its sliding thumb), [`ViewTabs`] and [`UnderlineTabs`] (with its sliding
//! indicator). Design spec 4.7 and 4.8.
//!
//! The thumb and the indicator are painted once, under the labels, and slide to the chosen option
//! on the indicator springs, measured from the control's own left edge so scrolling never animates
//! them. Only the chosen option is a Tab stop; arrow keys move the choice.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Galley, Id, Key, Modifiers, PointerButton, Rect,
    Response, Sense, Ui, UiBuilder, Vec2, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::button::{disabled_painter, tooltip_text, with_tooltip};
use super::toggle::{
    arrow_step, claim_arrows, component_id, gallery_caption, gallery_row, group_node, remembered,
    step,
};
use super::{ForceState, IconButton, Interact, KitResponse, Size, anim, paint};
use crate::ui::motion;
use crate::ui::theme::{self, Elevation};

/// Gap between a segment's or tab's icon, label, count and close button.
const ITEM_GAP: f32 = 7.0;
/// The underline indicator's thickness.
const INDICATOR: f32 = 2.0;
/// Space between underline tabs.
const TAB_GAP: f32 = 2.0;
/// Underline tabs: padding left of the content, and right of it with and without a close button.
const TAB_PAD_LEFT: f32 = 10.0;
const TAB_PAD_RIGHT_CLOSE: f32 = 6.0;
/// The ahead/behind count on a tab (mono, `accent`).
const TAB_COUNT_SIZE: f32 = 11.5;

/// A slid rectangle, measured from `origin` so moving the whole control never animates it. A
/// forced progress places it part-way from the previous option (`from`) instead.
fn slide(
    ctx: &egui::Context,
    id: Id,
    origin: egui::Pos2,
    target: Rect,
    from: Option<Rect>,
    force: Option<ForceState>,
) -> Rect {
    let local = target.translate(-origin.to_vec2());
    let r = match (force.and_then(|f| f.progress), from) {
        (Some(k), Some(from)) => {
            let from = from.translate(-origin.to_vec2());
            Rect::from_min_max(from.min.lerp(local.min, k), from.max.lerp(local.max, k))
        }
        (Some(_), None) | (None, _) if force.is_some() => local,
        _ => anim::slide_rect(ctx, id, local),
    };
    r.translate(origin.to_vec2())
}

/// The number badge on a segment (concept 1's neutral count).
fn badge_galley(ui: &Ui, n: usize) -> Arc<Galley> {
    let m = theme::metrics();
    let text = if n > 99 {
        "99+".to_owned()
    } else {
        n.to_string()
    };
    ui.painter().layout_no_wrap(
        text,
        FontId::new(m.small_size, theme::medium()),
        Color32::PLACEHOLDER,
    )
}

fn badge_width(g: &Galley) -> f32 {
    let m = theme::metrics();
    (g.size().x + 10.0).max(m.badge_height)
}

// ----------------------------------------------------------------------------------- Segmented

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    Text,
    IconAndText,
    IconOnly,
}

/// Mutually exclusive options in a recessed track; a raised thumb slides to the chosen one.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Segmented<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: Vec<(T, Option<&'a str>, &'a str)>,
    content: Content,
    badges: Vec<(usize, usize)>,
    label: String,
    equal: bool,
    size: Size,
    enabled: bool,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    force: Option<(usize, ForceState)>,
}

impl<'a, T: PartialEq + Clone> Segmented<'a, T> {
    /// Text segments: (value, label).
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        let options = options.iter().map(|(v, l)| (v.clone(), None, *l)).collect();
        Self::build(value, options, Content::Text)
    }

    /// Icon-only segments: (value, glyph, name); the name is the tooltip and accessible label.
    pub fn icons(value: &'a mut T, options: &'a [(T, &'a str, &'a str)]) -> Self {
        let options = options
            .iter()
            .map(|(v, g, l)| (v.clone(), Some(*g), *l))
            .collect();
        Self::build(value, options, Content::IconOnly)
    }

    /// Segments with a leading icon and a label: (value, glyph, label).
    pub fn with_icons(value: &'a mut T, options: &'a [(T, &'a str, &'a str)]) -> Self {
        let options = options
            .iter()
            .map(|(v, g, l)| (v.clone(), Some(*g), *l))
            .collect();
        Self::build(value, options, Content::IconAndText)
    }

    fn build(value: &'a mut T, options: Vec<(T, Option<&'a str>, &'a str)>, c: Content) -> Self {
        Self {
            value,
            options,
            content: c,
            badges: Vec::new(),
            label: String::new(),
            equal: false,
            size: Size::Md,
            enabled: true,
            disabled_reason: None,
            id_salt: None,
            force: None,
        }
    }

    /// A count badge on segment `index` (hidden when `n` is 0).
    pub fn badge(mut self, index: usize, n: usize) -> Self {
        if n > 0 {
            self.badges.push((index, n));
        }
        self
    }
    /// The group's accessible name (what the options choose).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    /// Every segment as wide as the widest.
    pub fn equal(mut self) -> Self {
        self.equal = true;
        self
    }
    /// `Sm` for headers (diff file header); `Md` by default.
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
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
    /// Forces a state on segment `index` (the gallery and tests only). A forced progress shows
    /// the thumb part-way from the first segment to the chosen one.
    pub(crate) fn show_forced(mut self, ui: &mut Ui, index: usize, force: ForceState) -> Response {
        self.force = Some((index, force));
        self.show(ui)
    }

    /// Shows the control; the response is the group's and reports `changed` when the choice
    /// moved.
    pub fn show(self, ui: &mut Ui) -> Response {
        struct Seg {
            icon: Option<Arc<Galley>>,
            text: Option<Arc<Galley>>,
            badge: Option<Arc<Galley>>,
            width: f32,
        }

        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let group_id = component_id(ui, "kit-segmented", self.id_salt);
        let seg_h = self.size.height() - 2.0;
        let inset = m.segment_track_pad + paint::SEAM;
        let font = FontId::new(m.body_size, theme::medium());
        let icon_font = FontId::proportional(m.icon_md);
        let pad = if self.size == Size::Md {
            m.segment_pad
        } else {
            m.segment_pad - 3.0
        };

        let segs: Vec<Seg> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, (_, glyph, label))| {
                let icon = glyph.map(|g| {
                    ui.painter().layout_no_wrap(
                        g.to_owned(),
                        icon_font.clone(),
                        Color32::PLACEHOLDER,
                    )
                });
                let text = (self.content != Content::IconOnly).then(|| {
                    ui.painter().layout_no_wrap(
                        (*label).to_owned(),
                        font.clone(),
                        Color32::PLACEHOLDER,
                    )
                });
                let badge = self
                    .badges
                    .iter()
                    .find(|(at, _)| *at == i)
                    .map(|(_, n)| badge_galley(ui, *n));
                let parts = [
                    icon.as_ref().map(|g| g.size().x),
                    text.as_ref().map(|g| g.size().x),
                    badge.as_ref().map(|g| badge_width(g)),
                ];
                let n = parts.iter().flatten().count();
                let content: f32 =
                    parts.iter().flatten().sum::<f32>() + ITEM_GAP * n.saturating_sub(1) as f32;
                let width = if self.content == Content::IconOnly && badge.is_none() {
                    (content + 2.0 * (pad - 3.0)).max(seg_h + 4.0)
                } else {
                    content + 2.0 * pad
                };
                Seg {
                    icon,
                    text,
                    badge,
                    width,
                }
            })
            .collect();
        let widest = segs.iter().map(|s| s.width).fold(0.0, f32::max);
        let widths: Vec<f32> = segs
            .iter()
            .map(|s| if self.equal { widest } else { s.width })
            .collect();
        let total = vec2(
            widths.iter().sum::<f32>() + 2.0 * inset,
            seg_h + 2.0 * inset,
        );
        let (track, _) = ui.allocate_exact_size(total, Sense::hover());
        let mut rects = Vec::with_capacity(segs.len());
        let mut x = track.left() + inset;
        for w in &widths {
            rects.push(Rect::from_min_size(
                pos2(x, track.top() + inset),
                vec2(*w, seg_h),
            ));
            x += w;
        }
        let chosen = self.options.iter().position(|(v, _, _)| v == self.value);

        let mut responses = Vec::with_capacity(rects.len());
        let inner = ui.new_child(
            UiBuilder::new()
                .max_rect(track)
                .accessibility_parent(group_id),
        );
        for (i, rect) in rects.iter().enumerate() {
            let sense = match (enabled, chosen.unwrap_or(0) == i) {
                (false, _) => Sense::hover(),
                (true, true) => Sense::click(),
                (true, false) => Sense::CLICK,
            };
            responses.push(inner.interact(*rect, group_id.with(i), sense));
        }

        let mut changed = false;
        let mut now = chosen;
        for (i, r) in responses.iter().enumerate() {
            if r.clicked() && now != Some(i) {
                now = Some(i);
                changed = true;
            }
        }
        let focused = responses.iter().position(Response::has_focus);
        if let Some(at) = focused {
            claim_arrows(ui, responses[at].id, true, false);
        }
        if enabled
            && let Some(at) = focused
            && let Some(delta) = arrow_step(ui, false, true)
        {
            let next = step(at, delta, responses.len());
            now = Some(next);
            responses[next].request_focus();
            changed = true;
        }
        if changed && let Some(i) = now {
            *self.value = self.options[i].0.clone();
        }

        if ui.is_rect_visible(track) {
            let painter = disabled_painter(ui, enabled, theme::DISABLED_ALPHA);
            let radius = CornerRadius::same(m.radius);
            paint::fill(
                &painter,
                track,
                CornerRadius::same(m.radius.saturating_add(2)),
                theme::Fill::flat(s.track),
            );
            paint::rim(
                &painter,
                track,
                CornerRadius::same(m.radius.saturating_add(2)),
                s.track_edge,
            );
            if let Some(i) = now {
                let force = self.force.map(|(_, f)| f);
                let thumb = slide(
                    &ctx,
                    group_id.with("kit-seg-thumb"),
                    track.min,
                    rects[i],
                    rects.first().copied(),
                    force,
                );
                paint::surface(
                    &painter,
                    thumb,
                    radius,
                    p,
                    style,
                    Elevation::Thumb,
                    s.thumb,
                    None,
                );
            }
            for (i, (seg, rect)) in segs.iter().zip(&rects).enumerate() {
                let force = self.force.filter(|(at, _)| *at == i).map(|(_, f)| f);
                let st = Interact::of(&ctx, &responses[i], enabled, force);
                let selected = anim::ease(
                    &ctx,
                    group_id.with(("kit-seg-on", i)),
                    now == Some(i),
                    motion::BASE,
                );
                let color = p.text_dim.lerp_to_gamma(p.text, selected.max(st.hover));
                let parts = [
                    seg.icon.as_ref().map(|g| g.size().x),
                    seg.text.as_ref().map(|g| g.size().x),
                    seg.badge.as_ref().map(|g| badge_width(g)),
                ];
                let n = parts.iter().flatten().count();
                let content: f32 =
                    parts.iter().flatten().sum::<f32>() + ITEM_GAP * n.saturating_sub(1) as f32;
                let mut x = rect.center().x - content / 2.0;
                let cy = rect.center().y;
                if let Some(g) = &seg.icon {
                    painter.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), color);
                    x += g.size().x + ITEM_GAP;
                }
                if let Some(g) = &seg.text {
                    painter.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), color);
                    x += g.size().x + ITEM_GAP;
                }
                if let Some(g) = &seg.badge {
                    let r = Rect::from_min_size(
                        pos2(x, cy - m.badge_height / 2.0),
                        vec2(badge_width(g), m.badge_height),
                    );
                    painter.rect_filled(r, CornerRadius::same(theme::FULL), s.badge);
                    painter.galley(r.center() - g.size() / 2.0, Arc::clone(g), p.text);
                }
                if st.focus > 0.0 {
                    paint::inset_ring(&painter, *rect, radius, p.focus.gamma_multiply(st.focus));
                }
            }
        }

        for (i, r) in responses.into_iter().enumerate() {
            let label = self.options[i].2;
            let on = now == Some(i);
            r.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, on, label));
            if self.content == Content::IconOnly {
                let tip = tooltip_text(enabled, Some(label), self.disabled_reason.as_deref(), None);
                with_tooltip(r, tip);
            } else if !enabled {
                with_tooltip(r, self.disabled_reason.clone());
            }
        }
        let mut group = group_node(ui, group_id, track, &self.label, enabled);
        if changed {
            group.mark_changed();
        }
        group
    }
}

impl<T: PartialEq + Clone> Widget for Segmented<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The History / Changes / Requests switcher in the action bar: the segmented control with icons
/// and labels (concept 1 puts a `.k-seg` at the start of the toolbar).
pub type ViewTabs<'a, T> = Segmented<'a, T>;

// ------------------------------------------------------------------------------- UnderlineTabs

/// In-pane tabs (legal documents, detail sub-views): labels over a seam, with a 2 pt indicator
/// that slides to the active tab. Tabs may carry an icon, a count and a close button.
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct UnderlineTabs<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: Vec<(T, Option<&'a str>, &'a str)>,
    counts: Vec<(usize, String)>,
    label: String,
    closable: bool,
    enabled: bool,
    id_salt: Option<Id>,
    force: Option<(usize, ForceState)>,
}

impl<'a, T: PartialEq + Clone> UnderlineTabs<'a, T> {
    /// Text tabs: (value, label).
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        let options = options.iter().map(|(v, l)| (v.clone(), None, *l)).collect();
        Self::build(value, options)
    }

    /// Tabs with a leading icon: (value, glyph, label).
    pub fn with_icons(value: &'a mut T, options: &'a [(T, &'a str, &'a str)]) -> Self {
        let options = options
            .iter()
            .map(|(v, g, l)| (v.clone(), Some(*g), *l))
            .collect();
        Self::build(value, options)
    }

    fn build(value: &'a mut T, options: Vec<(T, Option<&'a str>, &'a str)>) -> Self {
        Self {
            value,
            options,
            counts: Vec::new(),
            label: String::new(),
            closable: false,
            enabled: true,
            id_salt: None,
            force: None,
        }
    }

    /// A short count after tab `index`'s label, such as ahead/behind (`↑2`).
    pub fn count(mut self, index: usize, text: impl Into<String>) -> Self {
        self.counts.push((index, text.into()));
        self
    }
    /// The tab list's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    /// Each tab gets a close button (and closes on a middle click); `show` returns which.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    pub(crate) fn show_forced(
        mut self,
        ui: &mut Ui,
        index: usize,
        force: ForceState,
    ) -> KitResponse<Option<T>> {
        self.force = Some((index, force));
        self.show(ui)
    }

    /// Shows the tabs. The response is the tab list's (`changed` when the active tab moved); the
    /// value is the tab whose close button was pressed, if any.
    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<T>> {
        struct Tab {
            icon: Option<Arc<Galley>>,
            text: Arc<Galley>,
            count: Option<Arc<Galley>>,
            width: f32,
        }

        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let list_id = component_id(ui, "kit-underline-tabs", self.id_salt);
        let h = m.underline_tab_height;
        let font = FontId::new(m.body_size, theme::medium());
        let icon_font = FontId::proportional(m.icon_md);
        let close_side = Size::Xs.height();
        let pad_right = if self.closable {
            TAB_PAD_RIGHT_CLOSE
        } else {
            TAB_PAD_LEFT
        };

        let tabs: Vec<Tab> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, (_, glyph, label))| {
                let icon = glyph.map(|g| {
                    ui.painter().layout_no_wrap(
                        g.to_owned(),
                        icon_font.clone(),
                        Color32::PLACEHOLDER,
                    )
                });
                let text = ui.painter().layout_no_wrap(
                    (*label).to_owned(),
                    font.clone(),
                    Color32::PLACEHOLDER,
                );
                let count = self.counts.iter().find(|(at, _)| *at == i).map(|(_, c)| {
                    ui.painter().layout_no_wrap(
                        c.clone(),
                        FontId::new(TAB_COUNT_SIZE, FontFamily::Monospace),
                        Color32::PLACEHOLDER,
                    )
                });
                let mut width = TAB_PAD_LEFT + text.size().x + pad_right;
                if let Some(g) = &icon {
                    width += g.size().x + ITEM_GAP;
                }
                if let Some(g) = &count {
                    width += g.size().x + ITEM_GAP;
                }
                if self.closable {
                    width += close_side + ITEM_GAP;
                }
                Tab {
                    icon,
                    text,
                    count,
                    width,
                }
            })
            .collect();
        let row_w = ui
            .available_width()
            .max(tabs.iter().map(|t| t.width).sum::<f32>() + TAB_GAP * tabs.len() as f32);
        let (row, _) = ui.allocate_exact_size(vec2(row_w, h), Sense::hover());
        let mut rects = Vec::with_capacity(tabs.len());
        let mut x = row.left();
        for t in &tabs {
            rects.push(Rect::from_min_size(pos2(x, row.top()), vec2(t.width, h)));
            x += t.width + TAB_GAP;
        }
        let chosen = self.options.iter().position(|(v, _, _)| v == self.value);
        let mut responses = Vec::with_capacity(rects.len());
        let mut closes = Vec::new();
        let mut closed = None;
        let inner = ui.new_child(UiBuilder::new().max_rect(row).accessibility_parent(list_id));
        for (i, rect) in rects.iter().enumerate() {
            let sense = match (enabled, chosen.unwrap_or(0) == i) {
                (false, _) => Sense::hover(),
                (true, true) => Sense::click(),
                (true, false) => Sense::CLICK,
            };
            let r = inner.interact(*rect, list_id.with(i), sense);
            if self.closable && enabled && r.clicked_by(PointerButton::Middle) {
                closed = Some(i);
            }
            responses.push(r);
        }

        let mut changed = false;
        let mut now = chosen;
        for (i, r) in responses.iter().enumerate() {
            if r.clicked() && now != Some(i) {
                now = Some(i);
                changed = true;
            }
        }
        let focused = responses.iter().position(Response::has_focus);
        if let Some(at) = focused {
            claim_arrows(ui, responses[at].id, true, false);
        }
        if enabled && let Some(at) = focused {
            let jump = ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::Home) {
                    Some(0)
                } else if i.consume_key(Modifiers::NONE, Key::End) {
                    Some(responses.len().saturating_sub(1))
                } else {
                    None
                }
            });
            let next =
                jump.or_else(|| arrow_step(ui, false, true).map(|d| step(at, d, responses.len())));
            if let Some(next) = next {
                now = Some(next);
                responses[next].request_focus();
                changed = true;
            }
        }
        if changed && let Some(i) = now {
            *self.value = self.options[i].0.clone();
        }

        if ui.is_rect_visible(row) {
            let painter = disabled_painter(ui, enabled, theme::DISABLED_ALPHA);
            paint::seam_h(
                &painter,
                row.x_range(),
                row.bottom() - paint::SEAM * 0.5,
                s.border,
            );
            let top_radius = CornerRadius {
                nw: m.radius,
                ne: m.radius,
                sw: 0,
                se: 0,
            };
            for (i, (tab, rect)) in tabs.iter().zip(&rects).enumerate() {
                let force = self.force.filter(|(at, _)| *at == i).map(|(_, f)| f);
                let st = Interact::of(&ctx, &responses[i], enabled, force);
                let active = now == Some(i);
                if st.hover > 0.0 {
                    painter.rect_filled(*rect, top_radius, p.hover().gamma_multiply(st.hover));
                }
                let color = if active {
                    p.text
                } else {
                    p.text_dim.lerp_to_gamma(p.text, st.hover)
                };
                let mut x = rect.left() + TAB_PAD_LEFT;
                let cy = rect.center().y;
                if let Some(g) = &tab.icon {
                    let c = if active { p.accent } else { p.text_faint };
                    painter.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), c);
                    x += g.size().x + ITEM_GAP;
                }
                painter.galley(
                    pos2(x, cy - tab.text.size().y / 2.0),
                    Arc::clone(&tab.text),
                    color,
                );
                x += tab.text.size().x + ITEM_GAP;
                if let Some(g) = &tab.count {
                    painter.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), p.accent);
                    x += g.size().x + ITEM_GAP;
                }
                if self.closable {
                    closes.push(Rect::from_center_size(
                        pos2(x + close_side / 2.0, cy),
                        Vec2::splat(close_side),
                    ));
                }
                if st.focus > 0.0 {
                    paint::inset_ring(
                        &painter,
                        *rect,
                        top_radius,
                        p.focus.gamma_multiply(st.focus),
                    );
                }
            }
            if let Some(i) = now {
                let bar = Rect::from_min_max(
                    pos2(rects[i].left(), row.bottom() - INDICATOR),
                    pos2(rects[i].right(), row.bottom()),
                );
                let from = rects.first().map(|r| {
                    Rect::from_min_max(pos2(r.left(), bar.top()), pos2(r.right(), bar.bottom()))
                });
                let bar = slide(
                    &ctx,
                    list_id.with("kit-tab-indicator"),
                    row.min,
                    bar,
                    from,
                    self.force.map(|(_, f)| f),
                );
                painter.rect_filled(
                    bar,
                    CornerRadius {
                        nw: 2,
                        ne: 2,
                        sw: 0,
                        se: 0,
                    },
                    p.focus,
                );
            }
        }
        for (i, rect) in closes.into_iter().enumerate() {
            let name = format!("Close {}", self.options[i].2);
            let r = ui.put(
                rect,
                IconButton::new(icon::X, name)
                    .size(Size::Xs)
                    .enabled(enabled)
                    .id_salt(list_id.with(("close", i))),
            );
            if r.clicked() {
                closed = Some(i);
            }
        }

        for (i, r) in responses.into_iter().enumerate() {
            let label = self.options[i].2;
            let on = now == Some(i);
            r.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, on, label));
            ctx.accesskit_node_builder(r.id, |n| {
                n.set_role(Role::Tab);
                n.clear_toggled();
                n.set_selected(on);
            });
        }
        let mut list = ui.interact(row, list_id, Sense::hover());
        list.widget_info(|| WidgetInfo::labeled(WidgetType::Other, enabled, &self.label));
        ctx.accesskit_node_builder(list_id, |n| n.set_role(Role::TabList));
        if changed {
            list.mark_changed();
        }
        KitResponse {
            response: list,
            value: closed.map(|i| self.options[i].0.clone()),
        }
    }
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's segmented controls and tabs.
pub(crate) fn gallery(ui: &mut Ui) {
    gallery_caption(ui, "Segmented: text, live (click or arrows)");
    remembered(ui, "seg-text", 0_u8, |ui, v| {
        Segmented::new(v, &[(0, "Unified"), (1, "Split")])
            .label("Diff layout")
            .id_salt("g-seg0")
            .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "View tabs: icons, labels and a count badge");
    remembered(ui, "seg-view", 0_u8, |ui, v| {
        ViewTabs::with_icons(
            v,
            &[
                (0, icon::CLOCK_COUNTER_CLOCKWISE, "History"),
                (1, icon::PENCIL_SIMPLE, "Changes"),
                (2, icon::GIT_PULL_REQUEST, "Pull requests"),
            ],
        )
        .badge(1, 4)
        .label("View")
        .id_salt("g-seg1")
        .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "Icons only, small, equal; hover, focus, disabled");
    gallery_row(ui, |ui| {
        let layouts = [
            (0_u8, icon::ROWS, "Unified"),
            (1, icon::COLUMNS, "Side by side"),
        ];
        remembered(ui, "seg-icons", 0_u8, |ui, v| {
            Segmented::icons(v, &layouts)
                .label("Layout")
                .id_salt("g-seg2")
                .show(ui);
        });
        Segmented::new(&mut 1_u8, &[(0, "All"), (1, "Mine"), (2, "Open")])
            .size(Size::Sm)
            .equal()
            .id_salt("g-seg3")
            .show(ui);
        Segmented::new(&mut 0_u8, &[(0, "Hover"), (1, "Other")])
            .id_salt("g-seg4")
            .show_forced(ui, 1, ForceState::HOVER);
        Segmented::new(&mut 0_u8, &[(0, "Focus"), (1, "Other")])
            .id_salt("g-seg5")
            .show_forced(ui, 0, ForceState::FOCUS);
        Segmented::new(&mut 0_u8, &[(0, "Disabled"), (1, "Other")])
            .enabled(false)
            .disabled_reason("Open a repository first")
            .id_salt("g-seg6")
            .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "Thumb sliding to the third segment at t = 0, 0.5, 1");
    gallery_row(ui, |ui| {
        for (i, t) in [0.0_f32, 0.5, 1.0].into_iter().enumerate() {
            Segmented::new(&mut 2_u8, &[(0, "One"), (1, "Two"), (2, "Three")])
                .id_salt(("g-seg-m", i))
                .show_forced(
                    ui,
                    2,
                    ForceState {
                        progress: Some(t),
                        ..ForceState::default()
                    },
                );
        }
    });
    ui.add_space(theme::SM);
    gallery_caption(
        ui,
        "Underline tabs: icons, count, close; live (close a tab, + to reopen)",
    );
    remembered(ui, "tabs-docs", (0_u8, 3_u8), |ui, (v, open)| {
        let all = [
            (0_u8, icon::FOLDER, "showcase"),
            (1, icon::FOLDER, "canopy"),
            (2, icon::FOLDER, "dotfiles"),
        ];
        let shown: Vec<(u8, &str, &str)> = all.iter().take(usize::from(*open)).copied().collect();
        let out = UnderlineTabs::with_icons(v, &shown)
            .count(0, "↑2")
            .closable(true)
            .label("Repositories")
            .id_salt("g-tabs0")
            .show(ui);
        if out.value.is_some() && *open > 1 {
            *open -= 1;
            *v = (*v).min(*open - 1);
        }
        out.response.on_hover_text("Middle-click a tab to close it");
        if *open < 3 {
            let reopen = IconButton::new(icon::PLUS, "Reopen a tab")
                .size(Size::Sm)
                .id_salt("g-tabs-reopen")
                .show(ui);
            if reopen.clicked() {
                *open += 1;
            }
        }
    });
    ui.add_space(theme::XS);
    gallery_caption(
        ui,
        "Text tabs: hover on the second, focus on the first, disabled",
    );
    UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms"), (2, "Notices")])
        .id_salt("g-tabs1")
        .show_forced(ui, 1, ForceState::HOVER);
    UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms"), (2, "Notices")])
        .id_salt("g-tabs2")
        .show_forced(ui, 0, ForceState::FOCUS);
    UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms")])
        .enabled(false)
        .id_salt("g-tabs3")
        .show(ui);
    gallery_caption(ui, "Indicator sliding to the third tab at t = 0.5");
    UnderlineTabs::new(&mut 2_u8, &[(0, "Privacy"), (1, "Terms"), (2, "Notices")])
        .id_salt("g-tabs4")
        .show_forced(
            ui,
            2,
            ForceState {
                progress: Some(0.5),
                ..ForceState::default()
            },
        );
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Toggled;

    use super::super::testkit::{self, find, frame, key, spoken};
    use super::*;

    const OPTS: [(u8, &str); 3] = [(0, "History"), (1, "Changes"), (2, "Requests")];

    #[test]
    fn segmented_chooses_by_click_and_arrows() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut track = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            track = Segmented::new(&mut v, &OPTS).show(ui).rect;
        });
        // Click the right third.
        let at = pos2(track.right() - 8.0, track.center().y);
        testkit::click(&ctx, at, |ui| {
            Segmented::new(&mut v, &OPTS).show(ui);
        });
        assert_eq!(v, 2);
        // A key press and its release, with a button after the control that egui's own arrow
        // navigation could move focus to.
        let press = |k: Key, v: &mut u8| {
            let mut changed = false;
            frame(&ctx, key(k), |ui| {
                changed = Segmented::new(v, &OPTS).show(ui).changed();
                super::super::Button::new("After").show(ui);
            });
            frame(&ctx, Vec::new(), |ui| {
                Segmented::new(v, &OPTS).show(ui);
                super::super::Button::new("After").show(ui);
            });
            changed
        };
        press(Key::Tab, &mut v);
        assert!(press(Key::ArrowLeft, &mut v));
        assert_eq!(v, 1, "Left moves to the previous segment");
        press(Key::ArrowLeft, &mut v);
        assert_eq!(v, 0, "focus followed the choice");
        press(Key::ArrowRight, &mut v);
        assert_eq!(v, 1);
    }

    #[test]
    fn unsalted_segmented_controls_keep_their_own_choice() {
        let ctx = testkit::ctx();
        let (mut a, mut b) = (0_u8, 0_u8);
        let mut second = Rect::NOTHING;
        let show = |ui: &mut Ui, a: &mut u8, b: &mut u8| {
            Segmented::new(a, &OPTS).show(ui);
            Segmented::new(b, &OPTS).show(ui).rect
        };
        frame(&ctx, Vec::new(), |ui| second = show(ui, &mut a, &mut b));
        testkit::click(&ctx, pos2(second.right() - 8.0, second.center().y), |ui| {
            show(ui, &mut a, &mut b);
        });
        assert_eq!((a, b), (0, 2));
    }

    #[test]
    fn disabled_segmented_ignores_input() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut track = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            track = Segmented::new(&mut v, &OPTS).enabled(false).show(ui).rect;
        });
        testkit::click(&ctx, pos2(track.right() - 8.0, track.center().y), |ui| {
            Segmented::new(&mut v, &OPTS).enabled(false).show(ui);
        });
        frame(&ctx, key(Key::Tab), |ui| {
            Segmented::new(&mut v, &OPTS).enabled(false).show(ui);
        });
        frame(&ctx, key(Key::ArrowRight), |ui| {
            Segmented::new(&mut v, &OPTS).enabled(false).show(ui);
        });
        assert_eq!(v, 0);
    }

    #[test]
    fn segmented_speaks_as_a_radio_group() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let nodes = spoken(&ctx, |ui| {
            Segmented::new(&mut v, &OPTS).label("View").show(ui);
        });
        assert_eq!(find(&nodes, "View").map(|n| n.role), Some(Role::RadioGroup));
        let on = find(&nodes, "Changes").expect("spoken");
        assert_eq!(
            (on.role, on.toggled),
            (Role::RadioButton, Some(Toggled::True))
        );
        let off = find(&nodes, "History").expect("spoken");
        assert_eq!(off.toggled, Some(Toggled::False));
        let mut layout = 0_u8;
        let nodes = spoken(&ctx, |ui| {
            Segmented::icons(&mut layout, &[(0, icon::ROWS, "Unified")]).show(ui);
        });
        assert!(find(&nodes, "Unified").is_some(), "icon segments are named");
    }

    #[test]
    fn segmented_height_follows_the_metrics() {
        for density in theme::Density::ALL {
            for style in theme::Style::ALL {
                let ctx = testkit::ctx_with(theme::ThemeId::Midnight, style, density);
                let m = theme::metrics();
                let mut h = 0.0;
                frame(&ctx, Vec::new(), |ui| {
                    h = Segmented::new(&mut 0_u8, &OPTS).show(ui).rect.height();
                });
                assert_eq!(h, m.control - 2.0 + 2.0 * (m.segment_track_pad + 1.0));
            }
        }
    }

    #[test]
    fn thumb_lands_at_once_with_reduced_motion() {
        let ctx = testkit::ctx();
        let thumb = |v: u8| {
            let mut v = v;
            let s = theme::surfaces(&ctx);
            let shapes = frame(&ctx, Vec::new(), |ui| {
                Segmented::new(&mut v, &OPTS).id_salt("t").show(ui);
            });
            shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Rect(r) if r.fill == s.thumb.top && r.blur_width == 0.0 => {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .expect("thumb painted")
        };
        let a = thumb(0);
        let b = thumb(2);
        assert!(
            b.left() > a.right(),
            "the thumb is on the third segment the next frame"
        );
    }

    #[test]
    fn underline_tabs_switch_close_and_speak_as_tabs() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut row = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            row = UnderlineTabs::new(&mut v, &OPTS).show(ui).response.rect;
        });
        frame(&ctx, key(Key::Tab), |ui| {
            UnderlineTabs::new(&mut v, &OPTS).show(ui);
        });
        frame(&ctx, key(Key::End), |ui| {
            UnderlineTabs::new(&mut v, &OPTS).show(ui);
        });
        assert_eq!(v, 2, "End jumps to the last tab");
        frame(&ctx, key(Key::ArrowRight), |ui| {
            UnderlineTabs::new(&mut v, &OPTS).show(ui);
        });
        assert_eq!(v, 0, "Right from the last tab wraps");
        let nodes = spoken(&ctx, |ui| {
            UnderlineTabs::new(&mut v, &OPTS)
                .label("Documents")
                .closable(true)
                .show(ui);
        });
        assert_eq!(
            find(&nodes, "Documents").map(|n| n.role),
            Some(Role::TabList)
        );
        let tab = find(&nodes, "History").expect("spoken");
        assert_eq!((tab.role, tab.selected), (Role::Tab, Some(true)));
        assert!(find(&nodes, "Close Changes").is_some());
        // Middle click closes.
        let at = pos2(row.left() + 20.0, row.center().y);
        let mut closed = None;
        let events = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Middle,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ];
        let release = vec![egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Middle,
            pressed: false,
            modifiers: Modifiers::NONE,
        }];
        for events in [events, release] {
            frame(&ctx, events, |ui| {
                let r = UnderlineTabs::new(&mut v, &OPTS).closable(true).show(ui);
                closed = closed.or(r.value);
            });
        }
        assert_eq!(closed, Some(0));
    }
}
