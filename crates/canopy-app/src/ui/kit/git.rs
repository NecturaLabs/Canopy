//! Git components built on the library's chips: a file's change letter and a commit's ref labels.

use std::sync::Arc;

use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Galley, Painter, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::{ChangeKind, RefKind, RefLabel};
use nectura_ui::kit::chip::{Badge, label_response, layout, radius_for};
use nectura_ui::kit::overlay::TooltipExt;
use nectura_ui::kit::{ForceState, Interact, paint};
use nectura_ui::theme::{self, Palette};

use crate::ui::widgets;

/// `ChangeBadge`: radius 4, letter mono 10.
const CHANGE_RADIUS: u8 = 4;
const CHANGE_LETTER: f32 = 10.0;

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

    /// The badge's side (it is square).
    pub const SIDE: f32 = 17.0;

    /// Paints the badge in `rect` (`SIDE` square) with any painter (a list row's own painter).
    pub fn paint(self, painter: &Painter, rect: Rect) {
        let p = theme::palette(painter.ctx());
        let (fill, ink) = self.colors(p);
        painter.rect_filled(rect, CornerRadius::same(CHANGE_RADIUS), fill);
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            self.kind.letter(),
            FontId::new(CHANGE_LETTER, FontFamily::Monospace),
            ink,
        );
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(Self::SIDE), Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui.painter(), rect);
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

/// A branch, tag, stash or HEAD label beside a commit: one tonal pill for every kind in its
/// commit's lane colour, told apart by its glyph, the checked-out branch a little stronger.
/// Read-only unless [`RefPill::clickable`].
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

    /// The graph colour index of the commit's lane.
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

    /// Fill, ring and label colours ([`Palette::pill`] in the lane's colour): the same for every
    /// kind, a little stronger for the checked-out branch.
    pub fn colors(&self, p: &Palette) -> (Color32, Color32, Color32) {
        p.pill(p.lane(self.lane), self.kind == RefKind::CurrentBranch)
    }

    /// The glyphs' colour: the label's.
    pub fn glyph_color(&self, p: &Palette) -> Color32 {
        self.colors(p).2
    }

    fn label_font(&self) -> FontId {
        let m = theme::metrics();
        match self.kind {
            RefKind::Head => FontId::new(m.micro_size, FontFamily::Monospace),
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
            return 2.0 * m.ref_pill_pad;
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
            Stroke::new(paint::SEAM, ring),
            StrokeKind::Inside,
        );
        let glyphs = self.glyphs();
        let mut x = rect.left()
            + if glyphs.is_empty() {
                m.ref_pill_pad
            } else {
                m.ref_pill_pad_icon
            };
        let c = self.glyph_color(p);
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
            // The chip's text names it; its tooltip only describes it.
            match self.tooltip {
                Some(t) => response.kit_tooltip(t).extra(true).show(),
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
const GAP: f32 = theme::XS;

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

// ------------------------------------------------------------------------------------ gallery

/// The gallery's Git section: ref pills and change letters.
pub fn gallery(ui: &mut Ui) {
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
        RefPill::new("main", RefKind::CurrentBranch).show(ui);
        RefPill::new("origin/main", RefKind::RemoteBranch).show(ui);
        RefPill::new("v1.0.0", RefKind::Tag)
            .lane(2)
            .tooltip("Annotated tag")
            .show(ui);
        RefPill::new("HEAD", RefKind::Head).show(ui);
        RefPill::new("release/1.0", RefKind::LocalBranch)
            .upstream(true)
            .show(ui);
        RefOverflow::new(vec!["origin/feature/a".into(), "v0.9.0".into()]).show(ui);
        RefPill::new(
            "feature/a-very-long-branch-name-indeed",
            RefKind::LocalBranch,
        )
        .max_width(150.0)
        .show(ui);
        RefPill::new("feature/graph", RefKind::LocalBranch)
            .clickable()
            .show_forced(ui, ForceState::HOVER);
        RefPill::new("fix/typo", RefKind::LocalBranch)
            .clickable()
            .show_forced(ui, ForceState::FOCUS);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use nectura_ui::kit::chip::Keycap;
    use nectura_ui::theme::Style;

    /// Every kind of ref a pill shows.
    const KINDS: [RefKind; 6] = [
        RefKind::Head,
        RefKind::CurrentBranch,
        RefKind::LocalBranch,
        RefKind::RemoteBranch,
        RefKind::Tag,
        RefKind::Stash,
    ];

    fn ctx() -> egui::Context {
        nectura_ui::kit::testkit::ctx()
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, mut f: impl FnMut(&mut Ui)) {
        nectura_ui::kit::testkit::frame(ctx, events, |ui| f(ui));
    }

    /// Every ref pill shares one tonal design in its lane's colour: the badge surface tinted with
    /// it and a hairline of it, label and glyph white where white reads and in the lane colour
    /// otherwise; the checked-out branch is tinted more. Label and glyph read at 4.5:1 on the pill
    /// wherever it sits, in either style, and two lanes tell apart.
    #[test]
    fn every_ref_pill_is_tinted_in_its_lane() {
        for (id, style, contrast) in theme::ThemeId::ALL.into_iter().flat_map(|id| {
            Style::ALL
                .into_iter()
                .flat_map(move |style| [false, true].map(|contrast| (id, style, contrast)))
        }) {
            let _ctx = nectura_ui::kit::testkit::ctx_with(id, style, theme::Density::Comfortable);
            let p = id.shown_palette(contrast);
            for lane in (0u16..).take(p.lanes.len()) {
                let branch = RefPill::new("x", RefKind::LocalBranch).lane(lane);
                let badge = p.s().badge;
                let distance = |c: Color32| {
                    u32::from(badge.r().abs_diff(c.r()))
                        + u32::from(badge.g().abs_diff(c.g()))
                        + u32::from(badge.b().abs_diff(c.b()))
                };
                for kind in KINDS {
                    let pill = RefPill::new("x", kind).lane(lane);
                    let (fill, ring, ink) = pill.colors(p);
                    let strong = kind == RefKind::CurrentBranch;
                    assert_eq!(
                        (fill, ring, ink),
                        p.pill(p.lane(lane), strong),
                        "{} {kind:?} lane {lane}",
                        p.name
                    );
                    if !strong {
                        assert_eq!(fill, branch.colors(p).0, "{} {kind:?}", p.name);
                    }
                    assert_eq!(pill.glyph_color(p), ink, "{} {kind:?}", p.name);
                    assert_ne!(fill, p.s().badge, "{} lane {lane}: tinted", p.name);
                    assert!(
                        distance(ring) > distance(fill),
                        "{} {kind:?} lane {lane}: the hairline is more of the lane than the fill",
                        p.name
                    );
                    for bg in p.component_surfaces(p.s()) {
                        let r = theme::contrast(ink, theme::over(fill, bg));
                        assert!(
                            r >= 4.5,
                            "{} {style:?} {kind:?} lane {lane}: {r:.2}:1",
                            p.name
                        );
                    }
                }
                let current = RefPill::new("x", RefKind::CurrentBranch).lane(lane);
                assert!(
                    distance(current.colors(p).0) > distance(branch.colors(p).0),
                    "{} lane {lane}: the checked-out branch is tinted more",
                    p.name
                );
            }
            let one = RefPill::new("x", RefKind::LocalBranch).lane(0).colors(p).0;
            let two = RefPill::new("x", RefKind::LocalBranch).lane(1).colors(p).0;
            assert_ne!(one, two, "{}: lanes tell apart", p.name);
        }
    }

    #[test]
    fn pills_follow_the_metrics_in_every_density() {
        let ctx = ctx();
        for density in theme::Density::ALL {
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

    /// Spec 4.17: the detached-HEAD pill is padded `ref_pill_pad` either side (7, Dense 5), in
    /// the `micro_size` mono, like the other pills' density.
    #[test]
    fn the_head_pill_takes_its_padding_and_size_from_the_density() {
        for density in theme::Density::ALL {
            let ctx =
                super::super::testkit::ctx_with(theme::ThemeId::Midnight, Style::Standard, density);
            let m = theme::metrics();
            let mut width = 0.0;
            let mut label = 0.0;
            frame(&ctx, Vec::new(), |ui| {
                width = RefPill::new("HEAD", RefKind::Head).show(ui).rect.width();
                label = ui
                    .painter()
                    .layout_no_wrap(
                        "HEAD".to_owned(),
                        FontId::new(m.micro_size, FontFamily::Monospace),
                        Color32::PLACEHOLDER,
                    )
                    .size()
                    .x;
            });
            assert!(
                (width - (label + 2.0 * m.ref_pill_pad)).abs() < 0.01,
                "{density:?}: {width} vs {label} + 2 x {}",
                m.ref_pill_pad
            );
        }
    }
}
