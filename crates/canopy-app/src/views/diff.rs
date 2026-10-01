//! The diff viewer: hunks with old/new line numbers, coloured additions and removals, and
//! stage / unstage / discard actions per hunk. Rows are virtualised so huge diffs stay fast.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

use canopy_core::ops::HunkAction;
use canopy_core::{DiffSource, LineKind};

use crate::app::Action;
use crate::tab::RepoTab;
use crate::ui::theme::{self, Palette, palette};
use crate::ui::widgets;
use crate::worker::Op;

const LINE_H: f32 = 20.0;
const HUNK_H: f32 = 32.0;

/// Draws the diff of the selected file.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(sel) = tab.file.clone() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Select a file to see its diff").color(p.text_faint))
        });
        return;
    };
    // Header.
    egui::Frame::new()
        .fill(p.panel)
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::change_badge(ui, sel.entry.kind);
                let (name, dir) = widgets::split_path(&sel.entry.path);
                ui.label(
                    RichText::new(name)
                        .font(FontId::new(13.5, theme::semibold()))
                        .color(p.text),
                );
                if !dir.is_empty() {
                    ui.label(RichText::new(dir).size(12.0).color(p.text_faint));
                }
                if let Some(old) = &sel.entry.old_path {
                    ui.label(
                        RichText::new(format!("← {old}"))
                            .size(12.0)
                            .color(p.text_faint),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(d) = &tab.diff {
                        ui.label(
                            RichText::new(format!("−{}", d.deletions))
                                .color(p.danger)
                                .size(12.0),
                        );
                        ui.label(
                            RichText::new(format!("+{}", d.additions))
                                .color(p.success)
                                .size(12.0),
                        );
                    }
                    match (&sel.source, sel.conflicted) {
                        (_, true) => {
                            if ui
                                .button(
                                    RichText::new(format!("{}  Mark resolved", icon::CHECK))
                                        .color(p.success),
                                )
                                .clicked()
                            {
                                actions.push(Action::Op(Op::Stage(vec![sel.entry.path.clone()])));
                            }
                        }
                        (DiffSource::Unstaged, false) => {
                            if ui.button(format!("{}  Stage file", icon::PLUS)).clicked() {
                                actions.push(Action::Op(Op::Stage(vec![sel.entry.path.clone()])));
                            }
                        }
                        (DiffSource::Staged, false) => {
                            if ui
                                .button(format!("{}  Unstage file", icon::MINUS))
                                .clicked()
                            {
                                actions.push(Action::Op(Op::Unstage(vec![sel.entry.path.clone()])));
                            }
                        }
                        (DiffSource::Commit(_), _) => {}
                    }
                });
            });
        });
    ui.painter().hline(
        ui.min_rect().x_range(),
        ui.min_rect().max.y,
        Stroke::new(1.0, p.border),
    );

    if let Some(e) = &tab.diff_error {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(e).color(p.danger)));
        return;
    }
    let Some(d) = &tab.diff else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.spinner());
        return;
    };
    if d.binary {
        empty(
            ui,
            icon::FILE_DASHED,
            "Binary file",
            "Git does not show line changes for binary files.",
        );
        return;
    }
    if d.hunks.is_empty() {
        empty(
            ui,
            icon::EQUALS,
            "No line changes",
            "Only the file mode or metadata changed.",
        );
        return;
    }

    // Flatten to rows: (hunk, None) is a hunk header, (hunk, Some(line)) a line.
    let rows: Vec<(usize, Option<usize>)> = d
        .hunks
        .iter()
        .enumerate()
        .flat_map(|(h, hunk)| {
            std::iter::once((h, None)).chain((0..hunk.lines.len()).map(move |l| (h, Some(l))))
        })
        .collect();
    let heights: Vec<f32> = rows
        .iter()
        .map(|(_, l)| if l.is_none() { HUNK_H } else { LINE_H })
        .collect();
    let max_no = d
        .hunks
        .iter()
        .map(|h| (h.old_start + h.old_lines).max(h.new_start + h.new_lines))
        .max()
        .unwrap_or(1);
    let digits = max_no.to_string().len().max(3) as f32;
    let num_w = digits * 7.6 + 12.0;
    let mono = FontId::monospace(12.5);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&mono, 'm'));
    let longest = d
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .map(|l| l.text.chars().count())
        .max()
        .unwrap_or(0) as f32;
    let content_w = (num_w * 2.0 + 24.0 + longest * char_w + 40.0).max(ui.available_width());

    let source = sel.source.clone();
    let path = sel.entry.path.clone();
    let conflicted = sel.conflicted;
    egui::ScrollArea::both().auto_shrink([false, false]).id_salt(("diff", &path)).show_viewport(ui, |ui, viewport| {
        let total: f32 = heights.iter().sum();
        ui.set_min_size(vec2(content_w, total + 16.0));
        let origin = ui.min_rect().min;
        let mut y = 0.0;
        for (i, (h, l)) in rows.iter().enumerate() {
            let rh = heights[i];
            if y + rh < viewport.min.y {
                y += rh;
                continue;
            }
            if y > viewport.max.y {
                break;
            }
            let rect = egui::Rect::from_min_size(origin + vec2(0.0, y), vec2(content_w, rh));
            let hunk = &d.hunks[*h];
            match l {
                None => {
                    let band = rect.shrink2(vec2(0.0, 4.0));
                    ui.painter().rect_filled(band, CornerRadius::ZERO, Palette::tint(p.accent, if p.dark { 22 } else { 16 }));
                    let visible_left = viewport.min.x + origin.x;
                    ui.painter().text(pos2(visible_left + 12.0, band.center().y), Align2::LEFT_CENTER, &hunk.header, FontId::monospace(11.5), p.accent.lerp_to_gamma(p.text_dim, 0.4));
                    if !conflicted {
                        let buttons: &[(HunkAction, &str, &str)] = match source {
                            DiffSource::Unstaged => &[(HunkAction::Discard, icon::ARROW_COUNTER_CLOCKWISE, "Discard hunk"), (HunkAction::Stage, icon::PLUS, "Stage hunk")],
                            DiffSource::Staged => &[(HunkAction::Unstage, icon::MINUS, "Unstage hunk")],
                            DiffSource::Commit(_) => &[],
                        };
                        let mut bx = origin.x + viewport.max.x - 10.0;
                        for (action, glyph, label) in buttons {
                            let text = format!("{glyph}  {label}");
                            let g = ui.painter().layout_no_wrap(text, FontId::new(12.0, theme::medium()), p.text);
                            let br = egui::Rect::from_min_max(pos2(bx - g.size().x - 16.0, band.min.y + 2.0), pos2(bx, band.max.y - 2.0));
                            let resp = ui.interact(br, ui.id().with(("hunk", *h, *label)), Sense::click());
                            let danger = *action == HunkAction::Discard;
                            let base = if danger { p.danger } else { p.accent };
                            let fill = Palette::tint(base, if resp.hovered() { 80 } else { 36 });
                            ui.painter().rect(br, CornerRadius::same(theme::RADIUS_SM), fill, Stroke::new(1.0, Palette::tint(base, 140)), egui::StrokeKind::Inside);
                            ui.painter().galley(br.center() - g.size() / 2.0, g, if danger { p.danger } else { p.text });
                            if resp.clicked() {
                                let op = Op::Hunk { action: *action, path: path.clone(), index: *h, key: hunk.key() };
                                if danger {
                                    actions.push(Action::Confirm { title: "Discard hunk".into(), body: format!("Throw away this change to {path}? It cannot be recovered."), confirm: "Discard".into(), danger: true, op });
                                } else {
                                    actions.push(Action::Op(op));
                                }
                            }
                            bx = br.min.x - 8.0;
                        }
                    }
                }
                Some(li) => {
                    let line = &hunk.lines[*li];
                    let marker = conflicted && (line.text.starts_with("<<<<<<<") || line.text.starts_with("=======") || line.text.starts_with(">>>>>>>") || line.text.starts_with("|||||||"));
                    let (bg, fg, sign) = match line.kind {
                        LineKind::Added => (Some(p.added_bg), p.success, "+"),
                        LineKind::Removed => (Some(p.removed_bg), p.danger, "−"),
                        LineKind::Context if marker => (Some(Palette::tint(p.warning, 40)), p.warning, "!"),
                        LineKind::Context => (None, p.text_faint, " "),
                    };
                    if let Some(bg) = bg {
                        ui.painter().rect_filled(rect, CornerRadius::ZERO, bg);
                        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(3.0, rh)), CornerRadius::ZERO, fg);
                    }
                    let cy = rect.center().y;
                    let nf = FontId::monospace(11.5);
                    if let Some(n) = line.old_no {
                        ui.painter().text(pos2(rect.min.x + num_w - 4.0, cy), Align2::RIGHT_CENTER, n.to_string(), nf.clone(), p.text_faint);
                    }
                    if let Some(n) = line.new_no {
                        ui.painter().text(pos2(rect.min.x + num_w * 2.0 - 4.0, cy), Align2::RIGHT_CENTER, n.to_string(), nf, p.text_faint);
                    }
                    ui.painter().text(pos2(rect.min.x + num_w * 2.0 + 8.0, cy), Align2::LEFT_CENTER, sign, mono.clone(), fg);
                    let text_color = match line.kind {
                        LineKind::Context if !marker => p.text_dim,
                        _ => p.text,
                    };
                    let text = line.text.replace('\t', "    ");
                    ui.painter().text(pos2(rect.min.x + num_w * 2.0 + 24.0, cy), Align2::LEFT_CENTER, text, mono.clone(), text_color);
                }
            }
            y += rh;
        }
        // Gutter separator.
        let gx = origin.x + num_w * 2.0 + 2.0;
        ui.painter().vline(gx, (origin.y + viewport.min.y)..=(origin.y + viewport.max.y), Stroke::new(1.0, p.border));
    });
}

fn empty(ui: &mut Ui, glyph: &str, title: &str, body: &str) {
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(RichText::new(glyph).size(34.0).color(p.text_faint));
        ui.label(RichText::new(title).font(FontId::new(15.0, theme::semibold())));
        ui.label(RichText::new(body).color(p.text_dim));
    });
}
