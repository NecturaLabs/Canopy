//! Small reusable widgets drawn with the design tokens.

use egui::{Color32, FontId, Ui};
use egui_phosphor::regular as icon;

use canopy_core::ChangeKind;

use super::theme::Palette;

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
                *list = egui::layers::PaintList::default();
            }
        }
    });
}

/// A stable colour for a displayed identity, shared by names and matching host handles.
pub fn person_color(p: &Palette, identity: &str) -> Color32 {
    let mut h: u32 = 2_166_136_261;
    let key: String = identity
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    for b in key.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit;
    use crate::ui::theme::{self, palette};

    #[test]
    fn controls_keep_keyboard_activation_and_disabled_guards() {
        for kind in 0..4 {
            for enabled in [false, true] {
                let ctx = egui::Context::default();
                crate::ui::fonts::install(&ctx);
                theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
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
                            modifiers: egui::Modifiers::default(),
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
                                    0 => kit::toggle::Checkbox::new(&mut checked, "Checkbox")
                                        .show(ui),
                                    1 => kit::Button::new("Primary")
                                        .primary()
                                        .min_width(120.0)
                                        .show(ui),
                                    2 => kit::ToolButton::new(icon::CHECK, "Tool")
                                        .tooltip("Tool action")
                                        .show(ui),
                                    _ => kit::IconButton::new(icon::CHECK, "Icon action").show(ui),
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
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
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
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        for frame in 0..3 {
            let mut button_rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.add_enabled_ui(false, |ui| {
                    button_rect = kit::Button::new("Commit")
                        .primary()
                        .min_width(120.0)
                        .show(ui)
                        .rect;
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
                let limit = crate::ui::saturate_u8((255.0 * theme::DISABLED_ALPHA).ceil());
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
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                kit::choice::Segmented::new(&mut value, &[(0, "One"), (1, "Two")]).show(ui);
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
}
