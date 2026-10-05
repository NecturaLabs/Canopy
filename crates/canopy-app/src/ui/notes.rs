//! Release notes: the bundled `CHANGELOG.md` for "What's new", and a small Markdown renderer
//! for notes, pull request descriptions and comments (headings, bullets, task lists, paragraphs,
//! inline code). Anything fancier is shown as plain text.

use std::sync::Arc;

use egui::{
    CornerRadius, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, vec2,
};

use super::kit::paint;
use super::theme::{self, palette};

/// Canopy's changelog, as shipped.
pub const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

/// The body of `version`'s section (`## [x.y.z]`) of a Keep a Changelog file.
pub fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let header = format!("## [{version}]");
    let mut lines = changelog.lines().skip_while(|l| !l.starts_with(&header));
    lines.next()?;
    let body: Vec<&str> = lines
        .take_while(|l| !(l.starts_with("## [") || (l.starts_with('[') && l.contains("]: "))))
        .collect();
    let text = body.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Joins wrapped continuation lines into the bullet or paragraph they continue.
fn blocks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let t = line.trim_end();
        if t.trim().is_empty() {
            open = false;
            continue;
        }
        let starts_block = t.starts_with('#')
            || t.trim_start().starts_with("- ")
            || t.trim_start().starts_with("* ");
        if open && !starts_block {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(t.trim());
            }
        } else {
            out.push(t.to_owned());
            open = true;
        }
    }
    out
}

#[derive(Default)]
struct Blocks;

impl egui::cache::ComputerMut<&str, Arc<Vec<String>>> for Blocks {
    fn compute(&mut self, text: &str) -> Arc<Vec<String>> {
        Arc::new(blocks(text))
    }
}

/// Renders a little Markdown. The parsed blocks are cached while the text stays on screen.
pub fn markdown(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let parsed = ui.memory_mut(|mem| {
        Arc::clone(
            mem.caches
                .cache::<egui::cache::FrameCache<Arc<Vec<String>>, Blocks>>()
                .get(text),
        )
    });
    let paragraph = |text: String| RichText::new(text).size(m.body_size).color(p.text_dim);
    for block in parsed.iter() {
        let trimmed = block.trim_start();
        if let Some(h) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
        {
            ui.add_space(theme::XS);
            ui.label(
                RichText::new(h.trim())
                    .font(FontId::new(m.body_size, theme::semibold()))
                    .color(p.text),
            );
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            ui.label(
                RichText::new(h.trim())
                    .font(FontId::new(m.title_size, theme::semibold()))
                    .color(p.text),
            );
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let task = task_item(item);
            ui.horizontal_top(|ui| {
                ui.add_space(theme::XS);
                match task {
                    Some((done, _)) => task_box(ui, done),
                    None => {
                        ui.label(RichText::new("•").size(m.body_size).color(p.accent));
                    }
                }
                let text = task.map_or(item, |(_, text)| text);
                ui.add(egui::Label::new(paragraph(clean_inline(text))).wrap());
            });
        } else {
            ui.add(egui::Label::new(paragraph(clean_inline(trimmed))).wrap());
            ui.add_space(theme::XS);
        }
    }
}

/// A task list item's state and text: `[ ] text` is open, `[x] text` (or `[X]`) is done.
fn task_item(item: &str) -> Option<(bool, &str)> {
    let rest = item.strip_prefix('[')?;
    let done = match rest.chars().next()? {
        ' ' => false,
        'x' | 'X' => true,
        _ => return None,
    };
    // GitHub needs a space after the brackets: a bare `[x]` is not a task.
    let text = rest.get(1..)?.strip_prefix("] ")?;
    Some((done, text))
}

/// A read-only checkbox in the line of a task list item, drawn like the kit's checkbox (box, rim,
/// fill and tick) without the control: a description's tasks can only be ticked on the host.
fn task_box(ui: &mut Ui, done: bool) {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    let side = m.checkbox;
    // Centred on the first line of the text beside it.
    let line = ui.text_style_height(&egui::TextStyle::Body).max(side);
    let (rect, response) = ui.allocate_exact_size(vec2(side, line), Sense::hover());
    let boxed = Rect::from_center_size(rect.center(), vec2(side, side));
    let painter = ui.painter();
    let radius = CornerRadius::same(m.radius_sm);
    if done {
        paint::fill(painter, boxed, radius, s.primary);
        let unit = side / 24.0 * 1.4;
        let at = |x: f32, y: f32| -> Pos2 { boxed.center() + vec2(x - 12.0, y - 12.0) * unit };
        let points = [at(5.0, 12.5), at(9.5, 17.0), at(19.0, 7.5)];
        let width = unit * 2.6;
        painter.add(Shape::line(
            points.to_vec(),
            Stroke::new(width, p.on_accent),
        ));
        for point in points {
            painter.circle_filled(point, width / 2.0, p.on_accent);
        }
    } else {
        painter.rect_filled(boxed, radius, s.field);
        painter.rect_stroke(
            boxed,
            radius,
            Stroke::new(1.5, p.control_border),
            StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Checkbox,
            false,
            if done { "Task done" } else { "Task open" },
        )
    });
}

/// Drops emphasis markers that would otherwise show literally.
fn clean_inline(s: &str) -> String {
    s.replace("**", "").replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Changelog\n\n## [Unreleased]\n\n- next\n\n## [0.2.0] - 2026-10-01\n\n### Added\n\n- One\n  continued.\n- Two\n\n## [0.1.0] - 2026-09-01\n\n- First\n\n[0.2.0]: https://x\n";

    #[test]
    fn finds_a_version_section() {
        let s = changelog_section(LOG, "0.2.0").expect("section");
        assert!(s.starts_with("### Added"));
        assert!(!s.contains("First"));
        assert_eq!(changelog_section(LOG, "0.1.0").as_deref(), Some("- First"));
        assert_eq!(changelog_section(LOG, "9.9.9"), None);
    }

    #[test]
    fn joins_wrapped_bullets() {
        let b = blocks("- One\n  continued.\n- Two\n\nPara\nline");
        assert_eq!(b, vec!["- One continued.", "- Two", "Para line"]);
    }

    #[test]
    fn markdown_wraps_bullets_within_its_column() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut width = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(296.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.set_width(280.0);
                    markdown(ui, crate::ui::legal::Document::Privacy.body());
                    width = ui.min_rect().width();
                });
            },
        );
        output.textures_delta.clear();
        assert!(width <= 280.0, "legal text grew to {width}");
    }

    #[test]
    fn task_items_are_told_from_plain_bullets() {
        assert_eq!(task_item("[ ] write it"), Some((false, "write it")));
        assert_eq!(task_item("[x] ship it"), Some((true, "ship it")));
        assert_eq!(task_item("[X] ship it"), Some((true, "ship it")));
        for not_a_task in [
            "[x]",
            "[y] other",
            "[ ]glued",
            "[link](https://x) text",
            "plain",
        ] {
            assert_eq!(task_item(not_a_task), None, "{not_a_task}");
        }
    }

    /// F4: a description's task list reads as checkboxes, not bullets with literal brackets.
    #[test]
    fn task_lists_show_boxes_not_brackets() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0))),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    markdown(
                        ui,
                        "- [x] Update the docs\n- [ ] Add a migration\n- Plain bullet",
                    );
                });
            },
        );
        output.textures_delta.clear();
        let texts: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t == "Update the docs"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Add a migration"), "{texts:?}");
        assert!(
            !texts.iter().any(|t| t.contains('[')),
            "literal brackets shown: {texts:?}"
        );
        // The two tasks draw a box each; the plain bullet keeps its dot.
        assert_eq!(texts.iter().filter(|t| t.as_str() == "•").count(), 1);
    }

    #[test]
    fn bundled_changelog_has_an_unreleased_section() {
        assert!(changelog_section(CHANGELOG, "Unreleased").is_some());
    }
}
