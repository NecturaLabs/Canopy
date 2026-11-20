//! Small reusable widgets drawn with the design tokens.

use egui::Color32;
use egui_phosphor::regular as icon;

use canopy_core::ChangeKind;

use super::theme::Palette;

pub use nectura_ui::text::{dismiss_underlying_tooltips, layout_tidy, single_line_galley};

/// Colour and letter of a change kind.
pub fn change_style(p: &Palette, kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => p.success,
        ChangeKind::Modified | ChangeKind::Typechange => p.warning,
        ChangeKind::Deleted | ChangeKind::Conflicted => p.danger,
        ChangeKind::Renamed => p.accent2,
    }
}

/// Splits a path into (file name, parent directory).
pub fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/')
        .map_or((path, ""), |(parent, name)| (name, parent))
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
    date_in_zone(seconds, iso, jiff::tz::TimeZone::system())
}

/// A UTC calendar date in ISO or English month format (the first day of a UTC week).
pub fn utc_date_format(seconds: i64, iso: bool) -> String {
    date_in_zone(seconds, iso, jiff::tz::TimeZone::UTC)
}

fn date_in_zone(seconds: i64, iso: bool, zone: jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_second(seconds)
        .map(|d| {
            d.to_zoned_datetime(zone)
                .strftime(if iso { "%Y-%m-%d" } else { "%b %-d, %Y" })
                .to_string()
        })
        .unwrap_or_default()
}

/// A full timestamp in the author's offset, for tooltips and the details pane.
pub fn full_date(seconds: i64, offset_minutes: i32) -> String {
    // Git allows offsets up to 99:59; past a day, show the local date, as before.
    if offset_minutes.unsigned_abs() >= 24 * 60 {
        return absolute_date(seconds);
    }
    let Ok(offset) = jiff::tz::Offset::from_seconds(offset_minutes.saturating_mul(60)) else {
        return absolute_date(seconds);
    };
    jiff::Timestamp::from_second(seconds)
        .map(|d| {
            d.to_zoned_datetime(offset.to_time_zone())
                .strftime("%Y-%m-%d %H:%M (%:z)")
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

/// How wide `text` is on one line in `font`.
pub fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x
}

/// `text` cut in its middle with an ellipsis until it fits in `max_width` when drawn in
/// `font`: a path or branch name keeps its start and its end, the parts that name it.
pub fn elide_middle(ui: &egui::Ui, text: &str, font: &egui::FontId, max_width: f32) -> String {
    let width = |t: &str| text_width(ui, t, font);
    if width(text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| {
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let start: String = chars[..head].iter().collect();
        let end: String = chars[chars.len() - tail..].iter().collect();
        format!("{start}…{end}")
    };
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        if width(&build(mid)) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    build(low)
}

/// A path shown as the placeholder of a one-line text field `width` points wide, cut in its
/// middle to fit inside the field's padding, so its last folder stays in view.
pub fn field_placeholder(ui: &egui::Ui, path: &str, width: f32) -> String {
    let m = super::theme::metrics();
    let font = egui::FontId::proportional(m.body_size);
    elide_middle(ui, path, &font, width - 2.0 * m.field_pad_x)
}

/// Adds `text` to a `horizontal_wrapped` row after a "·" separator, as one unit: whole on the
/// current line when it fits there, else at the start of the next line without the separator,
/// so no separator is left hanging at a line's end.
pub fn wrapped_fact(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    let joined = format!("· {text}");
    let room = ui.available_size_before_wrap().x - ui.spacing().item_spacing.x;
    if text_width(ui, &joined, &egui::FontId::proportional(size)) <= room {
        ui.add(egui::Label::new(egui::RichText::new(joined).size(size).color(color)).extend());
    } else {
        ui.end_row();
        ui.add(egui::Label::new(egui::RichText::new(text).size(size).color(color)).wrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F12: "· updated …" moves to the next line as one unit and drops its separator there.
    #[test]
    fn a_fact_wraps_whole_and_drops_its_separator() {
        let ctx = crate::ui::kit::testkit::ctx();
        let place = |width: f32| {
            let nodes = crate::ui::kit::testkit::spoken(&ctx, |ui| {
                ui.set_width(width);
                ui.horizontal_wrapped(|ui| {
                    ui.label("lucia-ortega wants to merge feature into main");
                    wrapped_fact(ui, "updated 18 minutes ago", 12.0, Color32::GRAY);
                });
            });
            let rect = |label: &str| {
                nodes
                    .iter()
                    .find(|n| n.label.as_deref().or(n.value.as_deref()) == Some(label))
                    .and_then(|n| n.bounds)
            };
            let lead = rect("lucia-ortega wants to merge feature into main").expect("lead");
            (
                lead,
                rect("· updated 18 minutes ago"),
                rect("updated 18 minutes ago"),
            )
        };
        let (lead, joined, alone) = place(600.0);
        assert!(alone.is_none(), "whole, with its separator");
        assert!(
            joined.is_some_and(|r| lead.y_range().contains(r.center().y)),
            "on the lead's line when it fits: {lead:?} {joined:?}"
        );
        let (lead, joined, alone) = place(320.0);
        assert!(joined.is_none(), "no separator after a wrap");
        assert!(
            alone.is_some_and(|r| r.top() >= lead.bottom()),
            "on the next line: {lead:?} {alone:?}"
        );
    }

    #[test]
    fn dates_keep_their_formats() {
        // 2024-01-02T03:04:05Z.
        let at = 1_704_164_645;
        assert_eq!(
            date_in_zone(at, false, jiff::tz::TimeZone::UTC),
            "Jan 2, 2024"
        );
        assert_eq!(
            date_in_zone(at, true, jiff::tz::TimeZone::UTC),
            "2024-01-02"
        );
        assert_eq!(full_date(at, 330), "2024-01-02 08:34 (+05:30)");
        assert_eq!(full_date(at, -480), "2024-01-01 19:04 (-08:00)");
        assert_eq!(full_date(at, 1500), absolute_date(at));
        assert_eq!(date_in_zone(i64::MAX, true, jiff::tz::TimeZone::UTC), "");
    }

    #[test]
    fn paths() {
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
    fn a_long_path_is_cut_in_the_middle_to_the_width() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let path = "From global: /home/ada/projects/scratch/canopy-polish/tmpdir/gitconfig";
        let font = egui::FontId::proportional(crate::ui::theme::metrics().small_size);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                assert_eq!(elide_middle(ui, path, &font, 10_000.0), path);
                let cut = elide_middle(ui, path, &font, 200.0);
                assert!(cut.contains('…') && cut.starts_with("From") && cut.ends_with("gitconfig"));
                let width = ui
                    .painter()
                    .layout_no_wrap(cut, font.clone(), Color32::WHITE)
                    .size()
                    .x;
                assert!(width <= 200.0, "{width}");
                assert_eq!(elide_middle(ui, path, &font, 0.0), "…");
            });
        });
        output.textures_delta.clear();
    }
}
