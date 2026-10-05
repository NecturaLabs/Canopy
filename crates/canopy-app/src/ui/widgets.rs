//! Small reusable widgets drawn with the design tokens.

use egui::{
    Align2, Color32, CornerRadius, FontId, Response, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::{ChangeKind, RefKind, RefLabel};

use super::kit;
use super::theme::{self, Palette, palette};

/// A painted one-line label that ends in an ellipsis instead of cutting through a glyph.
pub fn single_line_galley(
    ui: &Ui,
    text: String,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text, font, color, width.max(0.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// Removes tooltips from content below a newly drawn blocking overlay.
/// egui can keep an existing tooltip open while the pointer stays over its old widget.
pub fn dismiss_underlying_tooltips(ctx: &egui::Context) {
    let layers = ctx.memory(|memory| {
        memory
            .layer_ids()
            .filter(|layer| layer.order == egui::Order::Tooltip)
            .collect::<Vec<_>>()
    });
    ctx.graphics_mut(|graphics| {
        for layer in layers {
            if let Some(list) = graphics.get_mut(layer) {
                *list = Default::default();
            }
        }
    });
}

/// Shows context actions above their containing navigation popup.
pub fn context_menu(
    response: &Response,
    content: impl FnOnce(&mut Ui),
) -> Option<egui::InnerResponse<()>> {
    let menu = response.context_menu(content);
    if let Some(menu) = &menu
        && response.layer_id.order == egui::Order::Foreground
    {
        response
            .ctx
            .set_sublayer(response.layer_id, menu.response.layer_id);
    }
    menu
}

fn ref_icon(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Head => icon::CROSSHAIR,
        RefKind::CurrentBranch => icon::CHECK_CIRCLE,
        RefKind::LocalBranch => icon::GIT_BRANCH,
        RefKind::RemoteBranch => icon::CLOUD,
        RefKind::Tag => icon::TAG,
        RefKind::Stash => icon::ARCHIVE,
    }
}

/// Colour a ref pill uses; branches take their lane colour.
pub fn ref_color(p: &Palette, kind: RefKind, lane: Color32) -> Color32 {
    match kind {
        RefKind::Head => p.danger,
        RefKind::CurrentBranch | RefKind::LocalBranch => lane,
        RefKind::RemoteBranch => p.accent2,
        RefKind::Tag => p.warning,
        RefKind::Stash => p.text_dim,
    }
}

/// The untruncated width of a ref pill, for content-based table sizing.
pub fn ref_pill_width(ui: &Ui, label: &RefLabel) -> f32 {
    ui.painter()
        .layout_no_wrap(
            format!("{} {}", ref_icon(label.kind), label.name),
            FontId::new(theme::SMALL_SIZE, theme::medium()),
            Color32::PLACEHOLDER,
        )
        .size()
        .x
        + 14.0
}

/// Paints a ref pill with its left edge at `x`, vertically centred on `cy`. Returns its width.
pub fn paint_ref_pill(ui: &Ui, x: f32, cy: f32, label: &RefLabel, lane: Color32) -> f32 {
    paint_ref_pill_with_palette(ui, x, cy, label, lane, palette(ui.ctx()))
}

fn paint_ref_pill_with_palette(
    ui: &Ui,
    x: f32,
    cy: f32,
    label: &RefLabel,
    lane: Color32,
    p: &Palette,
) -> f32 {
    let color = ref_color(p, label.kind, lane);
    let strong = label.kind == RefKind::CurrentBranch || label.kind == RefKind::Head;
    let fill = Palette::pill_fill(color, strong);
    let ink = p.ink(p.text, fill);
    let text = format!("{} {}", ref_icon(label.kind), label.name);
    let galley =
        ui.painter()
            .layout_no_wrap(text, FontId::new(theme::SMALL_SIZE, theme::medium()), ink);
    let size = vec2(galley.size().x + 14.0, 19.0);
    let rect = egui::Rect::from_min_size(pos2(x, cy - size.y / 2.0), size);
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        fill,
        Stroke::new(1.0, Palette::pill_border(color)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        pos2(rect.min.x + 7.0, cy - galley.size().y / 2.0),
        galley,
        ink,
    );
    size.x
}

/// A stable colour for a displayed identity, shared by names and matching host handles.
pub fn person_color(p: &Palette, identity: &str) -> Color32 {
    let mut h: u32 = 2166136261;
    let key: String = identity
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    for b in key.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(16777619);
    }
    p.lanes[(h as usize) % p.lanes.len()]
}

/// Up to two initials of a name.
pub fn initials(name: &str) -> String {
    let mut out: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect();
    if out.is_empty() {
        out.push('?');
    }
    out.to_uppercase()
}

/// Paints a round avatar with initials centred on `center`.
pub fn paint_avatar(ui: &Ui, center: egui::Pos2, radius: f32, name: &str, email: &str) {
    let p = palette(ui.ctx());
    let color = person_color(p, if name.trim().is_empty() { email } else { name });
    let painter = ui.painter();
    let fill = p.avatar_fill(color);
    painter.circle_filled(center, radius, fill);
    painter.circle_stroke(center, radius, Stroke::new(1.0, Palette::tint(color, 90)));
    let letters = initials(name);
    let shown = if radius <= 13.0 {
        letters.chars().take(1).collect::<String>()
    } else {
        letters
    };
    painter.text(
        center,
        Align2::CENTER_CENTER,
        shown,
        FontId::new(radius, theme::semibold()),
        p.ink(color, fill),
    );
}

/// Allocates and paints an avatar.
pub fn avatar(ui: &mut Ui, size: f32, name: &str, email: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_avatar(ui, rect.center(), size / 2.0, name, email);
    resp
}

/// Colour and letter of a change kind.
pub fn change_style(p: &Palette, kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => p.success,
        ChangeKind::Modified | ChangeKind::Typechange => p.warning,
        ChangeKind::Deleted | ChangeKind::Conflicted => p.danger,
        ChangeKind::Renamed => p.accent2,
    }
}

/// A small square badge with the change letter.
pub fn change_badge(ui: &mut Ui, kind: ChangeKind) -> Response {
    let p = palette(ui.ctx());
    let color = change_style(p, kind);
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    let fill = Palette::badge_fill(color);
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        kind.letter(),
        FontId::new(theme::SMALL_SIZE, theme::semibold()),
        p.ink(color, fill),
    );
    resp.on_hover_text(match kind {
        ChangeKind::Untracked => "Untracked: a new file Git does not track yet",
        _ => kind.label(),
    })
}

/// A rounded count badge.
pub fn count_badge(ui: &mut Ui, n: usize, color: Color32) {
    let p = palette(ui.ctx());
    let fill = Palette::badge_fill(color);
    let text = n.to_string();
    let galley = ui.painter().layout_no_wrap(
        text,
        FontId::new(theme::SMALL_SIZE, theme::semibold()),
        p.ink(color, fill),
    );
    let w = (galley.size().x + 12.0).max(20.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        Color32::PLACEHOLDER,
    );
}

/// An icon-only button with a tooltip.
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    kit::IconButton::new(glyph, tooltip).show(ui)
}

/// A toolbar action: icon over caption (Comfortable) or beside it (Dense).
pub fn tool_button(
    ui: &mut Ui,
    glyph: &str,
    label: &str,
    tooltip: &str,
    enabled: bool,
) -> Response {
    kit::ToolButton::new(glyph, label)
        .tooltip(tooltip)
        .enabled(enabled)
        .show(ui)
}

/// Mutually exclusive options in one recessed track; the chosen one is a raised segment, the others
/// stay quiet but still read as options. `options` are in visual order, left to right, whatever the
/// surrounding layout's direction. Returns whether the value changed.
pub fn segmented<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let font = FontId::new(theme::BODY_SIZE, theme::medium());
    let segments: Vec<Segment<'_>> = options
        .iter()
        .map(|(option, label)| Segment {
            text: label,
            name: label,
            selected: *value == *option,
            font: font.clone(),
        })
        .collect();
    match segment_track(ui, &segments, ui.spacing().interact_size.y, theme::MD) {
        Some(i) => {
            *value = options[i].0.clone();
            true
        }
        None => false,
    }
}

/// [`segmented`] with icons; each option is `(glyph, name)` and shows its name as a tooltip.
/// Returns the index of a newly chosen option.
pub fn icon_segmented(ui: &mut Ui, selected: usize, options: &[(&str, &str)]) -> Option<usize> {
    let segments: Vec<Segment<'_>> = options
        .iter()
        .enumerate()
        .map(|(i, (glyph, name))| Segment {
            text: glyph,
            name,
            selected: i == selected,
            font: FontId::proportional(theme::ICON_SM),
        })
        .collect();
    segment_track(ui, &segments, ui.spacing().interact_size.y, theme::SM)
}

struct Segment<'a> {
    text: &'a str,
    name: &'a str,
    selected: bool,
    font: FontId,
}

/// The caret of every drop-down (`ComboBox::icon`), from the icon font like the rest of the app.
pub fn combo_caret(ui: &Ui, rect: egui::Rect, visuals: &egui::style::WidgetVisuals, open: bool) {
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        if open {
            icon::CARET_UP
        } else {
            icon::CARET_DOWN
        },
        FontId::proportional(theme::BODY_SIZE),
        visuals.fg_stroke.color,
    );
}

/// The caret of every `CollapsingHeader` (`icon`), matching the app's other expanders.
pub fn collapse_caret(ui: &mut Ui, openness: f32, response: &Response) {
    ui.painter().text(
        response.rect.center(),
        Align2::CENTER_CENTER,
        if openness > 0.5 {
            icon::CARET_DOWN
        } else {
            icon::CARET_RIGHT
        },
        FontId::proportional(theme::BODY_SIZE),
        ui.style().interact(response).fg_stroke.color,
    );
}

/// egui paints a slider's rail in the inactive widget fill, which is often the surface behind it:
/// recess it like the segmented control's track so the whole range shows, and fill the chosen
/// part with the accent rather than the pale selection tint.
pub fn recessed_rail(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let v = ui.visuals_mut();
    v.widgets.inactive.bg_fill = if p.dark { p.bg() } else { p.border() };
    v.selection.bg_fill = p.accent;
}

/// Paints the track and its segments; returns the index of a segment clicked while not selected.
fn segment_track(ui: &mut Ui, segments: &[Segment<'_>], height: f32, pad: f32) -> Option<usize> {
    let p = palette(ui.ctx());
    let galleys: Vec<_> = segments
        .iter()
        .map(|s| {
            ui.painter().layout_no_wrap(
                s.text.to_owned(),
                s.font.clone(),
                egui::Color32::PLACEHOLDER,
            )
        })
        .collect();
    let enabled = ui.is_enabled();
    let inset = theme::CONTROL_INSET;
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 2.0 * pad).collect();
    let (outer, _) = ui.allocate_exact_size(
        vec2(widths.iter().sum::<f32>() + 2.0 * inset, height),
        Sense::hover(),
    );
    ui.painter().rect(
        outer,
        CornerRadius::same(theme::RADIUS_SM + 1),
        p.bg(),
        Stroke::new(1.0, p.border()),
        egui::StrokeKind::Inside,
    );
    let mut chosen = None;
    let mut x = outer.min.x + inset;
    let id = ui.next_auto_id();
    for (i, (segment, galley)) in segments.iter().zip(galleys).enumerate() {
        let rect = egui::Rect::from_min_size(
            pos2(x, outer.min.y + inset),
            vec2(widths[i], height - 2.0 * inset),
        );
        x += widths[i];
        let mut resp = ui.interact(rect, id.with(i), Sense::click());
        if segment.text != segment.name {
            resp = resp.on_hover_text(segment.name);
        }
        if resp.clicked() && !segment.selected {
            chosen = Some(i);
        }
        let selected = chosen.map_or(segment.selected, |c| c == i);
        let hover = ui.ctx().animate_bool_with_time(
            resp.id,
            enabled && resp.hovered() && !selected,
            super::motion::time(super::motion::FAST),
        );
        let radius = CornerRadius::same(theme::RADIUS_SM - 1);
        if selected {
            if enabled {
                ui.painter().add(p.control_shadow().as_shape(rect, radius));
            }
            ui.painter().rect(
                rect,
                radius,
                p.elevated(),
                Stroke::new(1.0, p.control_edge()),
                egui::StrokeKind::Inside,
            );
            if enabled {
                ui.painter().line_segment(
                    [
                        pos2(rect.left() + theme::SM, rect.bottom() - inset),
                        pos2(rect.right() - theme::SM, rect.bottom() - inset),
                    ],
                    Stroke::new(1.5, p.accent),
                );
            }
        } else if hover > 0.0 {
            ui.painter().rect_filled(
                rect,
                radius,
                Palette::tint(p.text, (hover * if p.dark { 12.0 } else { 14.0 }) as u8),
            );
        }
        let color = if !enabled {
            p.text_faint
        } else if selected {
            p.text
        } else {
            p.text_dim.lerp_to_gamma(p.text, hover)
        };
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, color);
        if resp.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(1.0),
                CornerRadius::same(theme::RADIUS_SM),
                Stroke::new(1.5, p.accent),
                egui::StrokeKind::Outside,
            );
        }
        let name = segment.name.to_owned();
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, selected, &name)
        });
    }
    chosen
}

/// A square checkbox with its label, distinct from radio options and switches.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: &str) -> Response {
    let p = palette(ui.ctx());
    let box_size = theme::CHECKBOX_SIZE;
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(theme::BODY_SIZE, egui::FontFamily::Proportional),
        Color32::PLACEHOLDER,
    );
    let size = vec2(
        box_size + theme::SM + galley.size().x,
        ui.spacing().interact_size.y,
    );
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *checked = !*checked;
        resp.mark_changed();
    }
    let enabled = ui.is_enabled();
    let t = ui.ctx().animate_bool_with_time(
        resp.id,
        *checked,
        super::motion::time(super::motion::FAST),
    );
    let b = egui::Rect::from_center_size(
        pos2(rect.left() + box_size / 2.0, rect.center().y),
        Vec2::splat(box_size),
    );
    let off_fill = if resp.hovered() && enabled {
        p.hover()
    } else {
        p.bg()
    };
    let fill = off_fill.lerp_to_gamma(p.button(), t);
    let stroke = if resp.hovered() && enabled && t < 1.0 {
        Palette::tint(p.accent, 200)
    } else {
        p.control_border.lerp_to_gamma(p.button(), t)
    };
    ui.painter().rect(
        b,
        CornerRadius::same(4),
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    if t > 0.0 {
        let c = b.shrink(4.0);
        let ink = Palette::tint(p.on_button(), (t * 255.0) as u8);
        ui.painter().add(egui::Shape::line(
            vec![
                pos2(c.left(), c.center().y + 0.5),
                pos2(c.left() + c.width() * 0.38, c.bottom() - 0.5),
                pos2(c.right(), c.top() + 1.0),
            ],
            Stroke::new(1.8, ink),
        ));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            b.expand(2.0),
            CornerRadius::same(6),
            Stroke::new(1.5, p.accent),
            egui::StrokeKind::Outside,
        );
    }
    ui.painter().galley(
        pos2(
            b.right() + theme::SM,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        if enabled { p.text } else { p.text_faint },
    );
    let name = label.to_owned();
    let state = *checked;
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, &name)
    });
    resp
}

/// An on/off switch named `label` for assistive technology (the visible label is the setting's
/// row). Returns whether it was flipped.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let size = theme::SWITCH_SIZE;
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let flipped = resp.clicked();
    if flipped {
        *on = !*on;
    }
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, *on, super::motion::time(super::motion::FAST));
    let hovered = enabled && resp.hovered();
    let radius = CornerRadius::same(theme::RADIUS_LG);
    let off_fill = if hovered { p.hover() } else { p.bg() };
    let fill = off_fill.lerp_to_gamma(p.button(), t);
    let stroke = p.control_border.lerp_to_gamma(p.button(), t);
    ui.painter().rect(
        rect,
        radius,
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    let r = (size.y - 2.0 * theme::CONTROL_INSET - 2.0) / 2.0;
    let cx = egui::lerp((rect.left() + 3.0 + r)..=(rect.right() - 3.0 - r), t);
    let thumb = if enabled {
        p.text_dim.lerp_to_gamma(p.on_button(), t)
    } else {
        p.text_faint
    };
    ui.painter()
        .circle_filled(pos2(cx, rect.center().y + 1.0), r, p.control_shadow().color);
    ui.painter()
        .circle_filled(pos2(cx, rect.center().y), r, thumb);
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(12),
            Stroke::new(1.5, p.accent),
            egui::StrokeKind::Outside,
        );
    }
    let name = label.to_owned();
    let state = *on;
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, &name)
    });
    flipped
}

/// Height of the buttons in a list header, so "Stage all" and a menu beside it match.
pub fn compact_h() -> f32 {
    kit::Size::Sm.height()
}

/// A small secondary button: the buttons in list headers.
pub fn compact_button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    kit::Button::new(text).size(kit::Size::Sm).show(ui)
}

/// A neutral action: the secondary button, at least `min_size.x` wide.
pub fn secondary_button(
    ui: &mut Ui,
    text: impl Into<egui::WidgetText>,
    min_size: Vec2,
) -> Response {
    kit::Button::new(text).min_width(min_size.x).show(ui)
}

/// A destructive action outside a confirmation: a secondary button whose icon is `danger`.
pub fn danger_button(ui: &mut Ui, text: &str, min_size: Vec2) -> Response {
    let button = kit::Button::new(text)
        .min_width(min_size.x)
        .icon_tone(kit::Tone::Danger);
    let button = if kit::button::split_glyph(text).0.is_none() {
        button.icon(icon::TRASH)
    } else {
        button
    };
    button.show(ui)
}

/// The primary call to action sized to its label, at least `min_size.x` wide.
pub fn accent_button(ui: &mut Ui, text: &str, min_size: Vec2) -> Response {
    kit::Button::new(text)
        .primary()
        .min_width(min_size.x)
        .show(ui)
}

/// The primary call to action, at least `width` wide.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool, width: f32) -> Response {
    kit::Button::new(text)
        .primary()
        .enabled(enabled)
        .min_width(width)
        .show(ui)
}

/// Fills `rect` with a horizontal gradient from `left` to `right` and rounded corners.
pub fn gradient_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    left: Color32,
    right: Color32,
    radius: u8,
) {
    // Draw the gradient as a mesh clipped to a rounded shape by painting rounded caps first.
    let r = f32::from(radius).min(rect.height() / 2.0);
    painter.rect_filled(
        egui::Rect::from_min_max(rect.min, pos2(rect.min.x + 2.0 * r, rect.max.y)),
        CornerRadius::same(radius),
        left,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(pos2(rect.max.x - 2.0 * r, rect.min.y), rect.max),
        CornerRadius::same(radius),
        right,
    );
    let inner = egui::Rect::from_min_max(
        pos2(rect.min.x + r, rect.min.y),
        pos2(rect.max.x - r, rect.max.y),
    );
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(inner.left_top(), left);
    mesh.colored_vertex(inner.right_top(), right);
    mesh.colored_vertex(inner.right_bottom(), right);
    mesh.colored_vertex(inner.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// Fades the top and bottom edges of a vertical scroll area into `surface` where more content
/// lies beyond them ([`kit::paint::scroll_fade`]).
pub fn scroll_fade<R>(ui: &Ui, output: &egui::scroll_area::ScrollAreaOutput<R>, surface: Color32) {
    kit::paint::scroll_fade(ui, output, surface);
}

/// A soft radial glow centred on `center`, used behind the welcome screen.
pub fn glow(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    let mut mesh = egui::Mesh::default();
    let segments = 64;
    mesh.colored_vertex(center, color);
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(
            center + vec2(a.cos(), a.sin()) * radius,
            Color32::TRANSPARENT,
        );
    }
    for i in 1..=segments {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(mesh);
}

/// Repository operation state stays visible in every work view.
pub fn repository_state_banner(
    ui: &mut Ui,
    tab: &crate::tab::RepoTab,
    actions: &mut Vec<crate::action::Action>,
) {
    let Some(snap) = &tab.snap else {
        return;
    };
    if snap.state == canopy_core::RepoState::Clean {
        return;
    }
    let p = palette(ui.ctx());
    let state = snap.state;
    let conflicts = snap.status.conflicted.len();
    let busy = tab.running.is_some();
    egui::Panel::top("repository-operation-state")
        .frame(egui::Frame::new().fill(Palette::tint(p.warning, if p.dark { 28 } else { 18 })).inner_margin(egui::Margin::symmetric(theme::LG as i8, theme::SM as i8)))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{}  {} in progress on {}", icon::WARNING, state.label(), snap.head.label())).color(p.warning).strong());
                ui.label(RichText::new(if conflicts > 0 { format!("{conflicts} conflict{} left", if conflicts == 1 { "" } else { "s" }) } else { "No conflicts left".to_owned() }).size(theme::META_SIZE).color(p.text_dim));
                if let Some(command) = state.command() {
                    let can_continue = conflicts == 0 && !busy;
                    if primary_button(ui, "Continue", can_continue, 110.0).on_hover_text(if busy { "Wait for the current operation" } else if conflicts > 0 { "Resolve and stage every conflicted file first" } else { "Continue this operation using the staged resolutions" }).clicked() {
                        actions.push(crate::action::Action::Op(crate::worker::Op::ContinueOrAbort { state, abort: false }));
                    }
                    ui.add_space(theme::LG);
                    if ui.add_enabled(!busy, egui::Button::new(RichText::new(format!("Abort {command}…")).color(p.danger))).clicked() {
                        actions.push(crate::action::Action::Confirm { title: format!("Abort {command}"), body: "Return the repository to where it was before the operation started? Resolved conflicts are discarded.".into(), confirm: "Abort".into(), danger: true, op: crate::worker::Op::ContinueOrAbort { state, abort: true } });
                    }
                }
            });
        });
}

/// A sentence-case section heading.
pub fn section_label(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.label(
        RichText::new(text)
            .font(FontId::new(theme::SMALL_SIZE, theme::semibold()))
            .color(p.text_dim),
    );
}

/// Splits a path into (file name, parent directory).
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[i + 1..], &path[..i]),
        None => (path, ""),
    }
}

/// A short relative time such as "3 hours ago".
pub fn relative_time(seconds: i64, now: i64) -> String {
    relative_time_format(seconds, now, false)
}

/// Relative time with the chosen calendar format for older dates.
pub fn relative_time_format(seconds: i64, now: i64, iso: bool) -> String {
    let d = now - seconds;
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match d {
        i64::MIN..=59 => "just now".to_owned(),
        60..=3599 => plural(d / 60, "minute"),
        3600..=86_399 => plural(d / 3600, "hour"),
        86_400..=604_799 => plural(d / 86_400, "day"),
        _ => absolute_date_format(seconds, iso),
    }
}

/// A date such as "Sep 21, 2026".
pub fn absolute_date(seconds: i64) -> String {
    absolute_date_format(seconds, false)
}

/// A local calendar date in ISO or English month format.
pub fn absolute_date_format(seconds: i64, iso: bool) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format(if iso { "%Y-%m-%d" } else { "%b %-d, %Y" })
                .to_string()
        })
        .unwrap_or_default()
}

/// A full timestamp in the author's offset, for tooltips and the details pane.
pub fn full_date(seconds: i64, offset_minutes: i32) -> String {
    let Some(offset) = chrono::FixedOffset::east_opt(offset_minutes * 60) else {
        return absolute_date(seconds);
    };
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|d| {
            d.with_timezone(&offset)
                .format("%Y-%m-%d %H:%M (%:z)")
                .to_string()
        })
        .unwrap_or_default()
}

/// The Phosphor icon of a forge.
pub fn forge_icon(kind: canopy_core::forge::ForgeKind) -> &'static str {
    match kind {
        canopy_core::forge::ForgeKind::GitHub => icon::GITHUB_LOGO,
        canopy_core::forge::ForgeKind::GitLab => icon::GITLAB_LOGO_SIMPLE,
    }
}

/// The destructive call to action in a confirmation: the solid danger button.
pub fn danger_primary_button(ui: &mut Ui, text: &str, min_size: Vec2) -> Response {
    kit::Button::new(text)
        .danger()
        .min_width(min_size.x)
        .show(ui)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_keep_keyboard_activation_and_disabled_guards() {
        for kind in 0..4 {
            for enabled in [false, true] {
                let ctx = egui::Context::default();
                crate::ui::fonts::install(&ctx);
                theme::apply(
                    &ctx,
                    theme::ThemeId::Midnight,
                    &crate::ui::theme::Appearance::STILL,
                );
                let mut checked = false;
                let mut activated = false;
                let mut focused = false;
                for frame in 0..5 {
                    let events = if frame == 2 {
                        vec![egui::Event::Key {
                            key: egui::Key::Space,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }]
                    } else {
                        Vec::new()
                    };
                    ctx.run_ui(
                        egui::RawInput {
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            ui.add_enabled_ui(enabled, |ui| {
                                let response = match kind {
                                    0 => checkbox(ui, &mut checked, "Checkbox"),
                                    1 => primary_button(ui, "Primary", true, 120.0),
                                    2 => tool_button(ui, icon::CHECK, "Tool", "Tool action", true),
                                    _ => icon_button(ui, icon::CHECK, "Icon action"),
                                };
                                if frame == 0 && enabled {
                                    response.request_focus();
                                }
                                activated |= response.clicked();
                                focused |= response.has_focus();
                            });
                        },
                    )
                    .drop_without_applying_deltas();
                }
                assert_eq!(activated, enabled, "control {kind}, enabled {enabled}");
                assert_eq!(focused, enabled, "control {kind}, enabled {enabled}");
                if kind == 0 {
                    assert_eq!(checked, enabled);
                }
            }
        }
    }

    #[test]
    fn blocking_overlay_hides_underlying_tooltips_and_keeps_its_own() {
        let ctx = egui::Context::default();
        for frame in 0..3 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                egui::Tooltip::always_open(
                    ctx.clone(),
                    ui.layer_id(),
                    egui::Id::new("underlying"),
                    egui::pos2(20.0, 20.0),
                )
                .show(|ui| {
                    ui.label("Underlying tooltip");
                });
                dismiss_underlying_tooltips(&ctx);
                egui::Modal::new(egui::Id::new("blocking-overlay")).show(&ctx, |ui| {
                    ui.label("Overlay content");
                    egui::Tooltip::always_open(
                        ctx.clone(),
                        ui.layer_id(),
                        egui::Id::new("overlay-tooltip"),
                        egui::pos2(100.0, 100.0),
                    )
                    .show(|ui| {
                        ui.label("Overlay tooltip");
                    });
                });
            });
            output.textures_delta.clear();
            if frame < 2 {
                continue;
            }
            let text: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.text())
                    } else {
                        None
                    }
                })
                .collect();
            assert!(text.contains(&"Overlay content"));
            assert!(text.contains(&"Overlay tooltip"));
            assert!(
                !text.contains(&"Underlying tooltip"),
                "background tooltip painted above overlay"
            );
        }
    }

    /// Design spec 2.4: a disabled button is the whole control at `DISABLED_ALPHA`, never the
    /// enabled fill at full strength.
    #[test]
    fn disabled_primary_controls_do_not_use_the_enabled_fill() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            &crate::ui::theme::Appearance::STILL,
        );
        for frame in 0..3 {
            let mut button_rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(Default::default(), |ui| {
                ui.add_enabled_ui(false, |ui| {
                    button_rect = primary_button(ui, "Commit", true, 120.0).rect;
                });
            });
            if frame == 2 {
                let rectangle = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.rect == button_rect => Some(rect),
                        _ => None,
                    })
                    .expect("painted disabled primary");
                let limit = (255.0 * theme::DISABLED_ALPHA).ceil() as u8;
                assert!(
                    rectangle.fill.a() > 0 && rectangle.fill.a() <= limit,
                    "a disabled primary is faded to the disabled opacity, got {:?}",
                    rectangle.fill
                );
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn segmented_dims_the_options_that_are_not_selected() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut value = 0;
        for frame in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                segmented(ui, &mut value, &[(0, "One"), (1, "Two")]);
            });
            output.textures_delta.clear();
            if frame == 0 {
                continue;
            }
            let ink = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(t) if t.galley.text() == label => {
                            let own = t.galley.job.sections[0].format.color;
                            Some(if own == Color32::PLACEHOLDER {
                                t.fallback_color
                            } else {
                                own
                            })
                        }
                        _ => None,
                    })
                    .expect(label)
            };
            let p = palette(&ctx);
            assert_eq!(ink("One"), p.text);
            assert_eq!(ink("Two"), p.text_dim);
        }
    }

    #[test]
    fn initials_and_paths() {
        assert_eq!(initials("ada lovelace"), "AL");
        assert_eq!(initials("Linus"), "L");
        assert_eq!(initials(""), "?");
        assert_eq!(split_path("src/ui/mod.rs"), ("mod.rs", "src/ui"));
        assert_eq!(split_path("README.md"), ("README.md", ""));
    }

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(1000, 1010), "just now");
        assert_eq!(relative_time(0, 120), "2 minutes ago");
        assert_eq!(relative_time(0, 3600), "1 hour ago");
        assert_eq!(relative_time(0, 86_400 * 3), "3 days ago");
    }
    #[test]
    fn rendered_ref_pills_use_readable_neutral_text_in_every_theme() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        for id in theme::ThemeId::ALL {
            let p = id.palette();
            for kind in [
                RefKind::CurrentBranch,
                RefKind::LocalBranch,
                RefKind::RemoteBranch,
                RefKind::Tag,
                RefKind::Head,
            ] {
                for lane in p.lanes {
                    let label = RefLabel {
                        name: "feature/readability".into(),
                        kind,
                    };
                    let mut output = ctx.run_ui(Default::default(), |ui| {
                        paint_ref_pill_with_palette(ui, 20.0, 30.0, &label, lane, p);
                    });
                    output.textures_delta.clear();
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| {
                            if let egui::Shape::Text(text) = &shape.shape {
                                Some(text)
                            } else {
                                None
                            }
                        })
                        .expect("painted ref text");
                    let actual = text
                        .override_text_color
                        .unwrap_or(text.galley.job.sections[0].format.color);
                    let fill = Palette::pill_fill(
                        ref_color(p, kind, lane),
                        matches!(kind, RefKind::CurrentBranch | RefKind::Head),
                    );
                    for surface in p.component_surfaces(p.s()) {
                        let ratio = theme::contrast(actual, theme::over(fill, surface));
                        assert!(
                            ratio >= 4.5,
                            "{} {kind:?}: painted ink {actual:?} contrast {ratio:.2}:1",
                            p.name
                        );
                    }
                    assert_eq!(
                        actual, p.text,
                        "{} ref text keeps a consistent neutral tone",
                        p.name
                    );
                }
            }
        }
    }
}
