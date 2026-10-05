//! Release notes: the bundled `CHANGELOG.md` for "What's new", and a small Markdown renderer
//! for notes, pull request descriptions and comments (headings, bullets, paragraphs, inline
//! code). Anything fancier is shown as plain text.

use std::sync::Arc;

use egui::{FontId, RichText, Ui};

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
            ui.horizontal_top(|ui| {
                ui.add_space(theme::XS);
                ui.label(RichText::new("•").size(m.body_size).color(p.accent));
                ui.add(egui::Label::new(paragraph(clean_inline(item))).wrap());
            });
        } else {
            ui.add(egui::Label::new(paragraph(clean_inline(trimmed))).wrap());
            ui.add_space(theme::XS);
        }
    }
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
    fn bundled_changelog_has_an_unreleased_section() {
        assert!(changelog_section(CHANGELOG, "Unreleased").is_some());
    }
}
