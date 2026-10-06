//! Document tabs (concept `.h-tab`, design spec 4.8): the repository tabs in the title bar.
//!
//! The active tab is marked in the theme's tab style ([`active_tab`]: lifted, underlined or ruled),
//! the others transparent until hovered. A tab shows a folder icon (a spinner while its
//! repository is busy), its name ellipsised, the ahead/behind counts in mono `accent`, and a close
//! button on hover and on the active tab; a middle click closes too. When the tabs need more room
//! than they have they scroll sideways (the mouse wheel scrolls them) and fade out at the edges
//! that hide more. A tab that goes away collapses to nothing over `TAB_CLOSE` instead of
//! vanishing, so its neighbours slide into its place; reduced motion removes it at once.
//!
//! `DocTabs` reports what the user asked for and changes nothing itself: the caller switches or
//! closes, and the tab it removes animates out on the next frames.

use std::fmt::Write as _;

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Id, Rect, Response, Sense, Ui, UiBuilder, Vec2,
    WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::overlay::TooltipExt;
use super::progress::Spinner;
use super::{ForceState, IconButton, Interact, KitResponse, Size, anim, paint};
use crate::ui::motion;
use crate::ui::theme::{self, Elevation, Fill, Style, TabStyle};

/// Padding inside a tab: left of the icon, right of the close button.
const PAD_LEFT: f32 = 10.0;
const PAD_RIGHT: f32 = 6.0;
/// Gap between a tab's icon, label, counts and close button.
const GAP: f32 = 7.0;
const MIN_WIDTH: f32 = 100.0;
const MAX_WIDTH: f32 = 220.0;
/// Gap between two tabs.
const BETWEEN: f32 = 4.0;
/// The ahead/behind counts (mono, `accent`).
const COUNT_SIZE: f32 = 11.5;

/// One tab: a stable key (it keeps the tab's identity when others open or close), a name, and
/// what it shows besides.
#[derive(Debug, Clone)]
pub struct DocTab<'a> {
    key: Id,
    name: &'a str,
    icon: &'a str,
    tooltip: Option<String>,
    detail: Option<String>,
    ahead: usize,
    behind: usize,
    busy: bool,
}

impl<'a> DocTab<'a> {
    /// A tab for `key` (the repository's path, say) named `name`.
    pub fn new(key: impl std::hash::Hash + std::fmt::Debug, name: &'a str) -> Self {
        Self {
            key: Id::new(("kit-doc-tab", key)),
            name,
            icon: icon::FOLDER_SIMPLE,
            tooltip: None,
            detail: None,
            ahead: 0,
            behind: 0,
            busy: false,
        }
    }
    /// The Phosphor glyph before the name (default: a folder), such as the repository's own icon.
    pub fn icon(mut self, glyph: &'a str) -> Self {
        self.icon = glyph;
        self
    }
    /// The tooltip's first line (default: the name).
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    /// A dim second tooltip line (the path, the shortcut that switches to it).
    pub fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = Some(text.into());
        self
    }
    /// Commits to push and to pull; nothing shows when both are zero.
    pub fn sync(mut self, ahead: usize, behind: usize) -> Self {
        self.ahead = ahead;
        self.behind = behind;
        self
    }
    /// The repository is working: a spinner takes the folder's place.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    fn counts(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.ahead > 0 {
            parts.push(format!("↑{}", self.ahead));
        }
        if self.behind > 0 {
            parts.push(format!("↓{}", self.behind));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn spoken(&self) -> String {
        let mut label = self.name.to_owned();
        if self.ahead > 0 || self.behind > 0 {
            let _ = write!(label, ", {} to push, {} to pull", self.ahead, self.behind);
        }
        if self.busy {
            label.push_str(", working");
        }
        label
    }
}

/// What the user asked of the tabs this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocTabsEvent {
    /// Switch to the tab at this index (a click, or Enter or Space on a focused tab).
    Select(usize),
    /// Close the tab at this index (its close button, or a middle click).
    Close(usize),
}

/// The tab strip: `DocTabs::new(&tabs, active).max_width(w).show(ui)`.
#[must_use = "call .show(ui)"]
pub struct DocTabs<'a> {
    tabs: &'a [DocTab<'a>],
    active: usize,
    id_salt: Id,
    max_width: Option<f32>,
    force: Option<(usize, ForceState)>,
    offset: Option<f32>,
}

/// A tab as it was drawn last frame, so one that disappears can collapse where it stood.
#[derive(Debug, Clone)]
struct Drawn {
    key: Id,
    name: String,
    width: f32,
}

/// A closed tab collapsing to nothing: before the tab now at `index`, from `width`, since `start`.
#[derive(Debug, Clone)]
struct Closing {
    key: Id,
    name: String,
    width: f32,
    index: usize,
    start: f64,
}

#[derive(Debug, Clone, Default)]
struct StripState {
    drawn: Vec<Drawn>,
    closing: Vec<Closing>,
    active: Option<Id>,
}

impl<'a> DocTabs<'a> {
    pub fn new(tabs: &'a [DocTab<'a>], active: usize) -> Self {
        Self {
            tabs,
            active,
            id_salt: Id::new("kit-doc-tabs"),
            max_width: None,
            force: None,
            offset: None,
        }
    }
    /// The widest the strip may be; past it the tabs scroll (default: all the room there is).
    pub fn max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Id::new(salt);
        self
    }
    /// Forces a state on the tab at `index` (the gallery only).
    pub(crate) fn force(mut self, index: usize, force: ForceState) -> Self {
        self.force = Some((index, force));
        self
    }

    /// Scrolls the strip to `x` points from its start (the gallery only).
    pub(crate) fn offset(mut self, x: f32) -> Self {
        self.offset = Some(x);
        self
    }

    /// Shows the strip; the value is what the user asked for, if anything.
    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<DocTabsEvent>> {
        let ctx = ui.ctx().clone();
        let m = theme::metrics();
        let id = ui.id().with(self.id_salt);
        let state_id = id.with("state");
        let mut state: StripState = ctx.data(|d| d.get_temp(state_id)).unwrap_or_default();
        let now = ctx.input(|i| i.time);
        let seconds = motion::time(motion::TAB_CLOSE);

        // Tabs drawn last frame that are gone now collapse where they stood.
        let keys: Vec<Id> = self.tabs.iter().map(|t| t.key).collect();
        if seconds > 0.0 {
            for (i, d) in state.drawn.iter().enumerate() {
                if !keys.contains(&d.key) && !state.closing.iter().any(|c| c.key == d.key) {
                    // Its place: before the first tab that followed it and is still here.
                    let index = state.drawn[i + 1..]
                        .iter()
                        .find_map(|after| keys.iter().position(|k| *k == after.key))
                        .unwrap_or(keys.len());
                    state.closing.push(Closing {
                        key: d.key,
                        name: d.name.clone(),
                        width: d.width,
                        index,
                        start: now,
                    });
                }
            }
        }
        state.closing.retain(|c| {
            seconds > 0.0 && crate::ui::to_f32(now - c.start) < seconds && !keys.contains(&c.key)
        });
        if !state.closing.is_empty() {
            ctx.request_repaint();
        }

        let active_key = self.tabs.get(self.active).map(|t| t.key);
        let reveal_active = active_key.is_some() && active_key != state.active;
        state.active = active_key;

        let mut event = None;
        let mut drawn = Vec::with_capacity(self.tabs.len());
        let max_width = self.max_width.unwrap_or_else(|| ui.available_width());
        let height = m.doc_tab_height;
        let mut area = egui::ScrollArea::horizontal()
            .id_salt(id.with("scroll"))
            .max_width(max_width.max(0.0))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        if let Some(x) = self.offset {
            area = area.horizontal_scroll_offset(x);
        }
        let scroll = area.show(ui, |ui| {
            let list = ui.scope_builder(
                UiBuilder::new()
                    .id_salt(id.with("list"))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = BETWEEN;
                    for i in 0..=self.tabs.len() {
                        for c in state.closing.iter().filter(|c| c.index == i) {
                            closing_tab(ui, c, now, seconds, height);
                        }
                        let Some(tab) = self.tabs.get(i) else {
                            break;
                        };
                        let force = self.force.filter(|(at, _)| *at == i).map(|(_, f)| f);
                        let (r, e) = doc_tab(ui, tab, i == self.active, force);
                        if reveal_active && i == self.active {
                            r.scroll_to_me(None);
                        }
                        if e.is_some() {
                            event = e.map(|e| match e {
                                TabEvent::Select => DocTabsEvent::Select(i),
                                TabEvent::Close => DocTabsEvent::Close(i),
                            });
                        }
                        drawn.push(Drawn {
                            key: tab.key,
                            name: tab.name.to_owned(),
                            width: r.rect.width(),
                        });
                    }
                },
            );
            ctx.accesskit_node_builder(list.response.id, |n| {
                n.set_role(Role::TabList);
                n.set_label("Repositories");
            });
            list.response
        });

        // Fade the edges that hide more tabs.
        paint::scroll_fade_x(ui, &scroll, strip_ground(&ctx));

        state.drawn = drawn;
        ctx.data_mut(|d| d.insert_temp(state_id, state));
        KitResponse {
            response: scroll.inner,
            value: event,
        }
    }
}

/// What the tab strip sits on: the deck's chrome in Standard, the window's ground in Deeper (whose
/// tab strip is not a bar of its own).
pub fn strip_ground(ctx: &egui::Context) -> Fill {
    let s = theme::surfaces(ctx);
    match theme::style() {
        Style::Standard => s.deck,
        Style::Deeper => Fill::flat(s.bg),
    }
}

enum TabEvent {
    Select,
    Close,
}

/// The width a tab needs for its name and counts, within `MIN_WIDTH` and `MAX_WIDTH`.
fn tab_width(ui: &Ui, tab: &DocTab<'_>, counts: Option<&std::sync::Arc<egui::Galley>>) -> f32 {
    let m = theme::metrics();
    let name_w = ui
        .painter()
        .layout_no_wrap(
            tab.name.to_owned(),
            FontId::new(m.body_size, theme::medium()),
            Color32::PLACEHOLDER,
        )
        .size()
        .x;
    let fixed = PAD_LEFT
        + m.icon_sm
        + GAP
        + counts.map_or(0.0, |g| g.size().x + GAP)
        + GAP
        + Size::Xs.height()
        + PAD_RIGHT;
    (fixed + name_w).clamp(MIN_WIDTH, MAX_WIDTH)
}

fn doc_tab(
    ui: &mut Ui,
    tab: &DocTab<'_>,
    selected: bool,
    force: Option<ForceState>,
) -> (Response, Option<TabEvent>) {
    let ctx = ui.ctx().clone();
    let p = theme::palette(&ctx);
    let m = theme::metrics();
    let counts = tab.counts().map(|text| {
        ui.painter().layout_no_wrap(
            text,
            FontId::new(COUNT_SIZE, FontFamily::Monospace),
            p.accent,
        )
    });
    let width = tab_width(ui, tab, counts.as_ref());
    let close_side = Size::Xs.height();
    let (_, rect) = ui.allocate_space(vec2(width, m.doc_tab_height));
    let resp = ui.interact(rect, tab.key, Sense::click());
    let st = Interact::of(&ctx, &resp, true, force);
    // The pointer over the close button still hovers the tab.
    let hover = st
        .hover
        .max(if force.is_none() && ui.rect_contains_pointer(rect) {
            anim::ease_bool(
                &ctx,
                tab.key.with("kit-doc-tab-over"),
                true,
                motion::FAST,
                motion::FAST_OUT,
            )
        } else {
            0.0
        });
    let radius = CornerRadius::same(m.doc_tab_radius);
    let mut event = None;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().clone();
        if selected {
            active_tab(&painter, rect, radius, hover);
        } else if hover > 0.0 {
            painter.rect_filled(rect, radius, p.hover().gamma_multiply(hover));
        }
        let cy = rect.center().y;
        let mut x = rect.left() + PAD_LEFT;
        let icon_rect =
            Rect::from_center_size(pos2(x + m.icon_sm / 2.0, cy), Vec2::splat(m.icon_sm));
        if tab.busy {
            // A child, so the spinner takes no room from the strip's layout.
            ui.new_child(UiBuilder::new().max_rect(icon_rect)).add(
                Spinner::new()
                    .size(Size::Xs)
                    .label(tab.name.to_owned())
                    .id_salt(tab.key.with("busy")),
            );
        } else {
            let color = if selected {
                p.text_dim
            } else {
                p.text_faint.lerp_to_gamma(p.text_dim, hover)
            };
            let g = painter.layout_no_wrap(
                tab.icon.to_owned(),
                theme::icon_font(m.icon_sm, selected),
                color,
            );
            painter.galley(icon_rect.center() - g.size() / 2.0, g, color);
        }
        x += m.icon_sm + GAP;
        let close_left = rect.right() - PAD_RIGHT - close_side;
        let counts_w = counts.as_ref().map_or(0.0, |g| g.size().x + GAP);
        let label_right = close_left - GAP - counts_w;
        let label_color = if selected {
            p.text
        } else {
            p.text_dim.lerp_to_gamma(p.text, hover)
        };
        let g = painter.layout_job(name_job(tab.name, label_color, label_right - x));
        let label_w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, label_color);
        if let Some(g) = counts {
            painter.galley(pos2(x + label_w + GAP, cy - g.size().y / 2.0), g, p.accent);
        }
        if selected || hover > 0.0 {
            let at = Rect::from_min_size(
                pos2(close_left, cy - close_side / 2.0),
                Vec2::splat(close_side),
            );
            let mut slot = ui.new_child(UiBuilder::new().max_rect(at));
            if !selected {
                slot.multiply_opacity(hover);
            }
            if IconButton::new(icon::X, format!("Close {}", tab.name))
                .size(Size::Xs)
                .id_salt(tab.key.with("close"))
                .show(&mut slot)
                .clicked()
            {
                event = Some(TabEvent::Close);
            }
        }
        if st.focus > 0.0 {
            paint::inset_ring(&painter, rect, radius, p.focus.gamma_multiply(st.focus));
        }
    }
    if event.is_none() {
        if resp.clicked() {
            event = Some(TabEvent::Select);
        } else if resp.middle_clicked() {
            event = Some(TabEvent::Close);
        }
    }
    let spoken = tab.spoken();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &spoken));
    ctx.accesskit_node_builder(resp.id, |n| {
        n.set_role(Role::Tab);
        n.clear_toggled();
        n.set_selected(selected);
    });
    let mut tip = resp.kit_tooltip(tab.tooltip.clone().unwrap_or_else(|| tab.name.to_owned()));
    if let Some(d) = &tab.detail {
        tip = tip.detail(d.clone());
    }
    let resp = tip.show();
    (resp, event)
}

/// The active tab's mark in the active theme's [`TabStyle`]: `Lifted` the `tab` fill with its
/// `tab_edge` ring (and `Elevation::Tab` in Deeper); `Underline` no fill, a `focus` underline (the
/// hover wash while hovered).
pub(crate) fn active_tab(painter: &egui::Painter, rect: Rect, radius: CornerRadius, hover: f32) {
    let p = theme::palette(painter.ctx());
    let style = theme::style();
    let s = p.surfaces(style);
    let lifted = || {
        paint::surface(
            painter,
            rect,
            radius,
            p,
            style,
            Elevation::Tab,
            s.tab,
            Some(s.tab_edge),
        );
    };
    match theme::active_design().active_tab {
        TabStyle::Lifted => lifted(),
        TabStyle::Underline { width } => {
            if hover > 0.0 {
                painter.rect_filled(rect, radius, p.hover().gamma_multiply(hover));
            }
            let inset = f32::from(radius.sw).max(theme::XS);
            let bar = Rect::from_min_max(
                pos2(rect.left() + inset, rect.bottom() - width),
                pos2(rect.right() - inset, rect.bottom()),
            );
            let round = CornerRadius::same(crate::ui::saturate_u8((width / 2.0).round()));
            painter.rect_filled(bar, round, p.focus);
        }
    }
}

/// The name on one line, ellipsised to `width`.
fn name_job(name: &str, color: Color32, width: f32) -> LayoutJob {
    let m = theme::metrics();
    let mut job = LayoutJob::simple(
        name.to_owned(),
        FontId::new(m.body_size, theme::medium()),
        color,
        width.max(1.0),
    );
    job.wrap = TextWrapping {
        max_width: width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job
}

/// A closed tab collapsing: its width eases to zero while its name fades.
fn closing_tab(ui: &mut Ui, c: &Closing, now: f64, seconds: f32, height: f32) {
    let t = (crate::ui::to_f32(now - c.start) / seconds).clamp(0.0, 1.0);
    let k = motion::EASE_OUT.at(t);
    let width = c.width * (1.0 - k);
    // The gap after it closes with it.
    let (rect, _) =
        ui.allocate_exact_size(vec2((width - BETWEEN * k).max(0.0), height), Sense::hover());
    let p = theme::palette(ui.ctx());
    let m = theme::metrics();
    let painter = ui.painter_at(rect);
    let color = p.text_dim.gamma_multiply(1.0 - k);
    let g = painter.layout_job(name_job(
        &c.name,
        color,
        (c.width - PAD_LEFT - m.icon_sm - GAP - PAD_RIGHT).max(1.0),
    ));
    painter.galley(
        pos2(
            rect.left() + PAD_LEFT + m.icon_sm + GAP,
            rect.center().y - g.size().y / 2.0,
        ),
        g,
        color,
    );
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's tabs: an active tab with counts, a busy one, a hovered one, a long name, and a
/// strip too narrow for its tabs (scrolled part-way, both edges fading).
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
    let tabs = [
        DocTab::new("showcase", "showcase")
            .sync(2, 0)
            .detail("~/src/showcase · Ctrl+1"),
        DocTab::new("canopy", "canopy")
            .busy(true)
            .detail("~/src/canopy · Ctrl+2"),
        DocTab::new("website", "website")
            .icon(icon::GLOBE)
            .sync(0, 3)
            .detail("~/src/website · Ctrl+3"),
        DocTab::new("long", "a-repository-with-a-very-long-name")
            .tooltip("a-repository-with-a-very-long-name")
            .detail("~/src/a-repository-with-a-very-long-name · Ctrl+4"),
    ];
    small(
        ui,
        "On the title bar: active with counts, busy, hovered, long name ellipsised",
    );
    on_deck(ui, |ui| {
        let _ = DocTabs::new(&tabs, 0)
            .id_salt("gallery-doc-tabs")
            .force(2, ForceState::HOVER)
            .show(ui);
    });
    ui.add_space(theme::SM);
    small(ui, "Overflow: scrolled part-way, both edges fade");
    let width = (ui.available_width() * 0.6).max(MIN_WIDTH);
    on_deck(ui, |ui| {
        let _ = DocTabs::new(&tabs, 1)
            .id_salt("gallery-doc-tabs-overflow")
            .max_width(width)
            .offset(MIN_WIDTH * 0.6)
            .show(ui);
    });
}

/// A strip of the title bar's deck, the tabs' own ground, for the gallery.
fn on_deck(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let m = theme::metrics();
    let s = theme::surfaces(ui.ctx());
    let slot = ui.painter().add(egui::Shape::Noop);
    let r = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(crate::ui::points_i8(theme::SM), 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(m.doc_tab_height);
            add(ui);
        });
    let rect = r.response.rect;
    let radius = CornerRadius::same(m.radius);
    ui.painter().set(
        slot,
        egui::Shape::Vec(vec![
            paint::fill_shape(ui.ctx(), rect, radius, strip_ground(ui.ctx())),
            egui::epaint::RectShape::stroke(
                rect,
                radius,
                egui::Stroke::new(paint::SEAM, s.deck_edge),
                egui::StrokeKind::Inside,
            )
            .into(),
        ]),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn frame(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut f = f;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 200.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    fn text_rect(out: &egui::FullOutput, text: &str) -> Option<Rect> {
        fn find(shape: &egui::Shape, text: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        out.shapes.iter().find_map(|c| find(&c.shape, text))
    }

    #[test]
    fn a_click_selects_and_a_middle_click_closes() {
        let ctx = context();
        let tabs = [DocTab::new(1, "alpha"), DocTab::new(2, "beta")];
        let mut at = egui::Pos2::ZERO;
        let mut got = Vec::new();
        for (n, button) in [
            (0, None),
            (1, Some(egui::PointerButton::Primary)),
            (2, None),
            (3, Some(egui::PointerButton::Middle)),
        ] {
            let mut events = vec![egui::Event::PointerMoved(at)];
            if let Some(b) = button {
                for pressed in [true, false] {
                    events.push(egui::Event::PointerButton {
                        pos: at,
                        button: b,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    });
                }
            }
            let out = frame(&ctx, f64::from(n) * 0.1, events, |ui| {
                if let Some(e) = DocTabs::new(&tabs, 0).show(ui).value {
                    got.push(e);
                }
            });
            if n == 0 {
                at = text_rect(&out, "beta").expect("beta tab").center();
            }
        }
        assert_eq!(got, vec![DocTabsEvent::Select(1), DocTabsEvent::Close(1)]);
    }

    #[test]
    fn a_tab_paints_its_own_icon() {
        let ctx = context();
        let tabs = [
            DocTab::new(1, "alpha").icon(icon::ROCKET_LAUNCH),
            DocTab::new(2, "beta"),
        ];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 0).show(ui);
        });
        assert!(
            text_rect(&out, icon::ROCKET_LAUNCH).is_some(),
            "chosen icon"
        );
        assert!(
            text_rect(&out, icon::FOLDER_SIMPLE).is_some(),
            "default folder"
        );
    }

    #[test]
    fn the_tabs_announce_a_tab_list_with_the_selected_tab() {
        let ctx = context();
        ctx.enable_accesskit();
        let tabs = [DocTab::new(1, "alpha").sync(2, 1), DocTab::new(2, "beta")];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 0).show(ui);
        });
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.role() == Role::TabList && n.label() == Some("Repositories"))
        );
        let tab = |name: &str| {
            nodes
                .iter()
                .find(|(_, n)| {
                    n.role() == Role::Tab && n.label().is_some_and(|l| l.starts_with(name))
                })
                .map(|(_, n)| (n.is_selected(), n.label().map(str::to_owned)))
        };
        assert_eq!(
            tab("alpha"),
            Some((Some(true), Some("alpha, 2 to push, 1 to pull".to_owned())))
        );
        assert_eq!(tab("beta").map(|t| t.0), Some(Some(false)));
        assert!(
            nodes.iter().any(|(_, n)| n.label() == Some("Close alpha")),
            "the active tab's close button is named"
        );
    }

    #[test]
    fn a_closed_tab_collapses_before_it_is_gone() {
        let ctx = context();
        motion::set_reduced(false);
        let both = [DocTab::new(1, "alpha"), DocTab::new(2, "beta")];
        let one = [DocTab::new(2, "beta")];
        let left_of_beta = |out: &egui::FullOutput| text_rect(out, "beta").map(|r| r.left());
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&both, 0).show(ui);
        });
        let before = left_of_beta(&out).expect("beta");
        // Half-way through the collapse, alpha still takes some room and beta has not arrived.
        let _ = frame(&ctx, 1.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&one, 0).show(ui);
        });
        let out = frame(
            &ctx,
            1.0 + f64::from(motion::TAB_CLOSE) / 2.0,
            Vec::new(),
            |ui| {
                let _ = DocTabs::new(&one, 0).show(ui);
            },
        );
        let middle = left_of_beta(&out).expect("beta");
        let out = frame(&ctx, 2.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&one, 0).show(ui);
        });
        let after = left_of_beta(&out).expect("beta");
        assert!(
            after < middle && middle < before,
            "{before} {middle} {after}"
        );
        assert!(
            text_rect(&out, "alpha").is_none(),
            "the collapsed tab is gone"
        );
    }

    #[test]
    fn a_closed_tab_goes_at_once_with_reduced_motion() {
        let ctx = context();
        motion::set_reduced(true);
        let both = [DocTab::new(1, "alpha"), DocTab::new(2, "beta")];
        let one = [DocTab::new(2, "beta")];
        let _ = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&both, 0).show(ui);
        });
        let out = frame(&ctx, 0.05, Vec::new(), |ui| {
            let _ = DocTabs::new(&one, 0).show(ui);
        });
        assert!(text_rect(&out, "alpha").is_none());
        motion::set_reduced(false);
    }

    #[test]
    fn long_names_are_ellipsised_within_the_widest_tab() {
        let ctx = context();
        let name = "a-repository-with-a-name-far-longer-than-any-tab-can-show";
        let tabs = [DocTab::new(1, name)];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let r = DocTabs::new(&tabs, 0).show(ui);
            assert!(r.response.rect.width() <= MAX_WIDTH + 0.5);
        });
        let shown = out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("a-repository") => {
                Some(std::sync::Arc::clone(&t.galley))
            }
            _ => None,
        });
        let g = shown.expect("the name");
        assert!(g.size().x < MAX_WIDTH);
        assert!(g.rows.iter().any(|r| r.text().ends_with('…')), "ellipsised");
    }

    #[test]
    fn a_busy_tab_keeps_its_width() {
        let ctx = context();
        let tabs = [DocTab::new(1, "alpha").busy(true), DocTab::new(2, "beta")];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 1).show(ui);
        });
        let alpha = text_rect(&out, "alpha").expect("alpha");
        let beta = text_rect(&out, "beta").expect("beta");
        assert!(
            alpha.right() + MIN_WIDTH / 2.0 < beta.left(),
            "the busy tab overlaps the next: {alpha:?} {beta:?}"
        );
    }

    /// The overflow fades melt into what the strip stands on: the deck in Standard, the bare
    /// ground in Deeper (whose tab strip is not a bar).
    #[test]
    fn overflow_fades_use_the_ground_the_strip_stands_on() {
        let tabs: Vec<DocTab<'_>> = (0..12).map(|i| DocTab::new(i, "repository")).collect();
        for style in [Style::Standard, Style::Deeper] {
            let ctx = context();
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    style,
                    ..theme::Appearance::STILL
                },
            );
            let s = theme::surfaces(&ctx);
            let ground = match style {
                Style::Standard => s.deck.top,
                Style::Deeper => s.bg,
            };
            let edge = ground.gamma_multiply(paint::SCROLL_FADE_ALPHA);
            let out = frame(&ctx, 0.0, Vec::new(), |ui| {
                let _ = DocTabs::new(&tabs, 0)
                    .max_width(400.0)
                    .offset(300.0)
                    .show(ui);
            });
            let fades = out
                .shapes
                .iter()
                .filter(|c| match &c.shape {
                    egui::Shape::Mesh(m) => m.vertices.iter().any(|v| v.color == edge),
                    _ => false,
                })
                .count();
            assert_eq!(
                fades, 2,
                "{style:?}: both edges hide tabs and fade into the ground"
            );
        }
    }
}
