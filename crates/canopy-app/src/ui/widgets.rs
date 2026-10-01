//! Small reusable widgets drawn with the design tokens.

use egui::{
    Align2, Color32, CornerRadius, FontId, Response, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::{ChangeKind, RefKind, RefLabel};

use super::theme::{self, Palette, palette};

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

/// Paints a ref pill with its left edge at `x`, vertically centred on `cy`. Returns its width.
pub fn paint_ref_pill(ui: &Ui, x: f32, cy: f32, label: &RefLabel, lane: Color32) -> f32 {
    let p = palette(ui.ctx());
    let color = ref_color(p, label.kind, lane);
    let text = format!("{} {}", ref_icon(label.kind), label.name);
    let galley = ui
        .painter()
        .layout_no_wrap(text, FontId::new(11.5, theme::medium()), color);
    let size = vec2(galley.size().x + 14.0, 19.0);
    let rect = egui::Rect::from_min_size(pos2(x, cy - size.y / 2.0), size);
    let strong = label.kind == RefKind::CurrentBranch || label.kind == RefKind::Head;
    let fill = Palette::pill_fill(color, strong);
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        fill,
        Stroke::new(1.0, Palette::tint(color, if strong { 220 } else { 110 })),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        pos2(rect.min.x + 7.0, cy - galley.size().y / 2.0),
        galley,
        p.ink(color, fill),
    );
    size.x
}

/// A stable colour for a person, from their e-mail address.
pub fn person_color(p: &Palette, email: &str) -> Color32 {
    let mut h: u32 = 2166136261;
    for b in email.to_lowercase().bytes() {
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
    let color = person_color(p, email);
    let painter = ui.painter();
    let fill = p.avatar_fill(color);
    painter.circle_filled(center, radius, fill);
    painter.circle_stroke(center, radius, Stroke::new(1.0, Palette::tint(color, 200)));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        initials(name),
        FontId::new(radius * 0.85, theme::semibold()),
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
        FontId::new(11.0, theme::semibold()),
        p.ink(color, fill),
    );
    resp.on_hover_text(kind.label())
}

/// A rounded count badge.
pub fn count_badge(ui: &mut Ui, n: usize, color: Color32) {
    let p = palette(ui.ctx());
    let fill = Palette::badge_fill(color);
    let text = n.to_string();
    let galley = ui.painter().layout_no_wrap(
        text,
        FontId::new(11.0, theme::semibold()),
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
    let p = palette(ui.ctx());
    ui.add(
        egui::Button::new(RichText::new(glyph).size(15.0).color(p.text_dim))
            .frame(false)
            .min_size(vec2(26.0, 26.0)),
    )
    .on_hover_text(tooltip)
}

/// A toolbar action: icon above a small label, like Fork and Sourcetree.
pub fn tool_button(
    ui: &mut Ui,
    glyph: &str,
    label: &str,
    tooltip: &str,
    enabled: bool,
) -> Response {
    let p = palette(ui.ctx());
    let size = vec2(58.0, 46.0);
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, enabled && resp.hovered(), 0.12);
    if hover > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            Palette::tint(p.text, (hover * if p.dark { 14.0 } else { 18.0 }) as u8),
        );
    }
    let color = if !enabled {
        p.text_faint
    } else if resp.is_pointer_button_down_on() {
        p.accent
    } else {
        p.text_dim.lerp_to_gamma(p.text, hover)
    };
    ui.painter().text(
        rect.center_top() + vec2(0.0, 15.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(19.0),
        color,
    );
    ui.painter().text(
        rect.center_bottom() - vec2(0.0, 10.0),
        Align2::CENTER_CENTER,
        label,
        FontId::new(11.0, theme::medium()),
        color,
    );
    let name = label.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    resp.on_hover_text(tooltip)
}

/// A button for a dangerous action: danger-coloured label on a tinted fill, readable in every
/// theme.
pub fn danger_button(ui: &mut Ui, text: &str, min_size: Vec2) -> Response {
    let p = palette(ui.ctx());
    ui.add(
        egui::Button::new(RichText::new(text).color(p.danger).strong())
            .fill(Palette::tint(p.danger, if p.dark { 40 } else { 26 }))
            .stroke(Stroke::new(1.0, Palette::tint(p.danger, 170)))
            .min_size(min_size),
    )
}

/// The filled accent button used outside gradients (confirmations).
pub fn accent_button(ui: &mut Ui, text: &str, min_size: Vec2) -> Response {
    let p = palette(ui.ctx());
    ui.add(
        egui::Button::new(RichText::new(text).color(p.on_button).strong())
            .fill(p.button)
            .min_size(min_size),
    )
}

/// The primary call to action: a gradient button.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool, width: f32) -> Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(
        vec2(width, 34.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hover = ui.ctx().animate_bool_with_time(
        resp.id,
        enabled && resp.hovered(),
        theme::fade(ui.ctx(), 0.12),
    );
    let (a, b) = if enabled {
        (p.button, p.button2)
    } else {
        (p.border, p.border)
    };
    let boost = 1.0 + 0.15 * hover;
    gradient_rect(
        ui.painter(),
        rect,
        a.gamma_multiply(boost.min(1.2)),
        b.gamma_multiply(boost.min(1.2)),
        theme::RADIUS,
    );
    if hover > 0.0 {
        ui.painter().rect_stroke(
            rect.expand(1.5),
            CornerRadius::same(theme::RADIUS + 1),
            Stroke::new(2.0, Palette::tint(p.accent, (hover * 90.0) as u8)),
            egui::StrokeKind::Outside,
        );
    }
    let fg = if enabled { p.on_button } else { p.text_faint };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::new(13.5, theme::semibold()),
        fg,
    );
    let label = text.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    resp
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

/// A section heading in small capitals.
pub fn section_label(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.label(
        RichText::new(text.to_uppercase())
            .font(FontId::new(10.5, theme::semibold()))
            .color(p.text_faint)
            .extra_letter_spacing(0.8),
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
        _ => absolute_date(seconds),
    }
}

/// A date such as "Sep 21, 2026".
pub fn absolute_date(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%b %-d, %Y")
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
                .format("%a %b %-d %Y, %H:%M (%:z)")
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
