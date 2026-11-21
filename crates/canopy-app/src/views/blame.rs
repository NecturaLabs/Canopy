//! The blame view: the selected file's lines, each with the commit and author that last changed
//! it, in place of the file's diff. A line's commit opens in History. Rows are virtualised so a
//! long file stays fast.

use egui::{Align, Align2, FontId, Layout, Rect, RichText, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

use canopy_core::{Blame, BlameCommit, short_id};

use crate::action::Action;
use crate::highlight::Highlights;
use crate::tab::{BlameRead, FileSelection, RepoTab};
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::surface::EmptyState;
use crate::ui::kit::{Button, Size, paint};
use crate::ui::theme::{self, palette};

/// Column widths: the commit id, the author, the date and the line number.
const ID_W: f32 = 76.0;
const AUTHOR_W: f32 = 128.0;
const DATE_W: f32 = 92.0;
const NUMBER_W: f32 = 52.0;
/// Room before the code, after the line number.
const CODE_GAP: f32 = 12.0;
/// What an uncommitted line says in the commit column.
const UNCOMMITTED: &str = "Not committed";

/// The file's blame, if `tab` is showing it for `sel`.
pub fn shown<'a>(tab: &'a RepoTab, sel: &FileSelection) -> Option<&'a BlameRead> {
    tab.blame
        .as_ref()
        .filter(|b| b.path == sel.entry.path && b.rev == sel.blame_rev())
}

/// Draws the blame of `sel`, which `tab` is showing ([`shown`]).
pub fn show(ui: &mut Ui, tab: &mut RepoTab, sel: &FileSelection, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(read) = shown(tab, sel).cloned() else {
        return;
    };
    if header(ui, sel, read.rev) {
        tab.close_blame();
        return;
    }
    match &read.result {
        None => {
            ui.add_space(theme::XL);
            ui.vertical_centered(|ui| {
                Spinner::new().label("the blame").active(true).show(ui);
            });
        }
        Some(Err(error)) => {
            ui.add_space(theme::XL);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(error).color(p.danger));
            });
        }
        Some(Ok(blame)) if blame.lines.is_empty() => {
            EmptyState::new("Nothing to blame")
                .icon(icon::FILE_DASHED)
                .body("The file is empty.")
                .compact(true)
                .show(ui);
        }
        Some(Ok(blame)) => lines(
            ui,
            &read.path,
            blame,
            read.longest,
            read.highlights.as_deref(),
            tab.iso_dates,
            actions,
        ),
    }
}

/// The title row: what is blamed, and the way back to the diff. True when that was chosen.
fn header(ui: &mut Ui, sel: &FileSelection, rev: Option<canopy_core::Oid>) -> bool {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let mut back = false;
    let frame = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(
            crate::ui::points_i8(theme::MD),
            crate::ui::points_i8(((m.file_header_height - m.control_sm) / 2.0).max(0.0)),
        ))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let row = vec2(ui.available_width(), m.control_sm);
            ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
                back = Button::new("Show diff")
                    .secondary()
                    .size(Size::Sm)
                    .icon(icon::ARROW_LEFT)
                    .id_salt("blame-back")
                    .show(ui)
                    .clicked();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(RichText::new(icon::USER_LIST).color(p.text_dim));
                    ui.label(
                        RichText::new("Blame")
                            .font(FontId::new(m.lead_size, theme::semibold()))
                            .color(p.text),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&sel.entry.path)
                                .size(m.meta_size)
                                .color(p.text_dim),
                        )
                        .truncate(),
                    )
                    .kit_tooltip(&sel.entry.path)
                    .show();
                    let at = rev.map_or_else(|| "working tree".to_owned(), short_id);
                    ui.label(
                        RichText::new(format!("at {at}"))
                            .size(m.meta_size)
                            .color(p.text_faint),
                    );
                });
            });
        });
    let rect = frame.response.rect;
    paint::seam_h(
        ui.painter(),
        rect.x_range(),
        rect.bottom() - paint::SEAM * 0.5,
        p.s().border,
    );
    back
}

/// The date of a commit in Settings' calendar format (`iso`), as the History table writes it.
fn date(commit: &BlameCommit, iso: bool) -> String {
    Some(crate::ui::widgets::absolute_date_format(commit.time, iso))
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "unknown date".into())
}

/// What a screen reader says for a line.
fn spoken(number: usize, commit: &BlameCommit, text: &str, iso: bool) -> String {
    let who = if commit.is_uncommitted() {
        UNCOMMITTED.to_owned()
    } else {
        format!(
            "{} by {} on {}",
            short_id(commit.oid),
            commit.author,
            date(commit, iso)
        )
    };
    let text = if text.trim().is_empty() {
        "blank"
    } else {
        text
    };
    format!("Line {number}, {who}: {text}")
}

fn tooltip(commit: &BlameCommit, iso: bool) -> String {
    if commit.is_uncommitted() {
        return "This line is changed in the working tree and not committed yet.".into();
    }
    format!(
        "{}\n{} <{}>\n{}\nOpen the commit in History",
        commit.summary,
        commit.author,
        commit.email,
        date(commit, iso)
    )
}

fn lines(
    ui: &mut Ui,
    path: &str,
    blame: &Blame,
    longest: usize,
    highlights: Option<&Highlights>,
    iso: bool,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let row_h = m.diff_line_height;
    let mono = FontId::monospace(m.mono_size);
    let small = FontId::monospace(m.small_size);
    let char_w = ui
        .painter()
        .layout_no_wrap("M".into(), mono.clone(), p.text)
        .size()
        .x;
    let gutter = ID_W + AUTHOR_W + DATE_W + NUMBER_W + CODE_GAP;
    let content_w = gutter + crate::ui::count_f32(longest) * char_w + theme::XL;
    // Rows touch: the height `show_rows` lays them out by is the row's alone.
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .id_salt(("blame", path))
        .show_rows(ui, row_h, blame.lines.len(), |ui, rows| {
            ui.set_min_width(content_w);
            for i in rows {
                let line = &blame.lines[i];
                let commit = &blame.commits[line.commit];
                let (rect, _) = ui.allocate_exact_size(
                    vec2(content_w.max(ui.available_width()), row_h),
                    Sense::hover(),
                );
                let starts = i == 0 || blame.lines[i - 1].commit != line.commit;
                let commit_cell =
                    Rect::from_min_size(rect.min, vec2(ID_W + AUTHOR_W + DATE_W, row_h));
                let response = ui.interact(
                    commit_cell,
                    ui.id().with(("blame-line", i)),
                    if commit.is_uncommitted() {
                        Sense::hover()
                    } else {
                        Sense::click()
                    },
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        spoken(i + 1, commit, &line.text, iso),
                    )
                });
                if !commit.is_uncommitted() {
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(commit_cell, egui::CornerRadius::ZERO, p.hover());
                    }
                    if response.clicked() {
                        actions.push(Action::Reveal(commit.oid));
                    }
                }
                let response = response.on_hover_cursor(if commit.is_uncommitted() {
                    egui::CursorIcon::Default
                } else {
                    egui::CursorIcon::PointingHand
                });
                if response.hovered() {
                    response.kit_tooltip(tooltip(commit, iso)).show();
                }

                let painter = ui.painter();
                if starts {
                    paint::seam_h(painter, rect.x_range(), rect.top(), p.s().border);
                }
                let cy = rect.center().y;
                let mut x = rect.min.x + theme::SM;
                if commit.is_uncommitted() {
                    if starts {
                        painter.text(
                            pos2(x, cy),
                            Align2::LEFT_CENTER,
                            UNCOMMITTED,
                            small.clone(),
                            p.warning,
                        );
                    }
                } else {
                    painter.text(
                        pos2(x, cy),
                        Align2::LEFT_CENTER,
                        short_id(commit.oid),
                        small.clone(),
                        if starts { p.accent } else { p.text_faint },
                    );
                    if starts {
                        let author = painter.layout(
                            commit.author.clone(),
                            FontId::proportional(m.meta_size),
                            p.text,
                            AUTHOR_W - theme::SM,
                        );
                        let clip = painter.with_clip_rect(Rect::from_min_size(
                            pos2(rect.min.x + ID_W, rect.top()),
                            vec2(AUTHOR_W - theme::XS, row_h),
                        ));
                        clip.galley(
                            pos2(rect.min.x + ID_W, cy - author.size().y / 2.0),
                            author,
                            p.text,
                        );
                        painter.text(
                            pos2(rect.min.x + ID_W + AUTHOR_W, cy),
                            Align2::LEFT_CENTER,
                            date(commit, iso),
                            small.clone(),
                            p.text_dim,
                        );
                    }
                }
                x = rect.min.x + ID_W + AUTHOR_W + DATE_W + NUMBER_W;
                painter.text(
                    pos2(x - theme::SM, cy),
                    Align2::RIGHT_CENTER,
                    (i + 1).to_string(),
                    small.clone(),
                    p.text_faint,
                );
                // Coloured by language like the file's diff, once the highlighter has it.
                let spans = highlights
                    .and_then(|h| h.lines.first())
                    .and_then(|lines| lines.get(i))
                    .map_or(&[][..], Vec::as_slice);
                let code = painter.layout_job(crate::views::diff::code_job(
                    &line.text, spans, p.text, &p.syntax, &mono,
                ));
                painter.galley(pos2(x + CODE_GAP, cy - code.size().y / 2.0), code, p.text);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #194: blame writes dates in Settings' calendar format, in the column, the hover and what
    /// a screen reader hears.
    #[test]
    fn blame_dates_follow_the_calendar_setting() {
        let commit = BlameCommit {
            oid: git2::Oid::from_bytes(&[1; 20]).expect("an id"),
            author: "Ada".into(),
            email: "ada@example.com".into(),
            // 14 August 2026, 12:00 UTC: the 14th or 15th in every time zone.
            time: 1_786_708_800,
            summary: "Add metrics".into(),
        };
        let iso = date(&commit, true);
        assert!(
            iso.len() == 10 && iso.starts_with("2026-08-1") && iso.as_bytes()[4] == b'-',
            "{iso}"
        );
        let written = date(&commit, false);
        assert!(
            written.starts_with("Aug 1") && written.ends_with(", 2026"),
            "{written}"
        );
        assert!(tooltip(&commit, false).contains(&written));
        assert!(spoken(1, &commit, "x", true).contains(&iso));
    }

    /// J7: with syntax highlighting on, a blamed TOML line paints its code in more than one
    /// colour, as the file's diff does; with it off, in one.
    #[test]
    fn blamed_code_is_coloured_by_language_when_highlighting_is_on() {
        let text = ["[package]", "name = \"canopy\" # the app", "edition = 2024"];
        let blame = Blame {
            commits: vec![BlameCommit {
                oid: git2::Oid::from_bytes(&[1; 20]).expect("an id"),
                author: "Ada".into(),
                email: "ada@example.com".into(),
                time: 1_786_708_800,
                summary: "Add metrics".into(),
            }],
            lines: text
                .iter()
                .map(|t| canopy_core::BlameLine {
                    commit: 0,
                    text: (*t).into(),
                })
                .collect(),
        };
        let h = crate::highlight::highlight(&crate::highlight::whole_file("Cargo.toml", text))
            .expect("TOML is highlighted");
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let colours = |highlights: Option<&Highlights>| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                lines(
                    ui,
                    "Cargo.toml",
                    &blame,
                    30,
                    highlights,
                    false,
                    &mut Vec::new(),
                );
            });
            out.textures_delta.clear();
            let mut colours = std::collections::BTreeSet::new();
            for c in &out.shapes {
                if let egui::Shape::Text(t) = &c.shape
                    && t.galley.text() == text[1]
                {
                    for section in &t.galley.job.sections {
                        let colour = if section.format.color == egui::Color32::PLACEHOLDER {
                            t.fallback_color
                        } else {
                            section.format.color
                        };
                        colours.insert(colour.to_array());
                    }
                }
            }
            colours
        };
        assert!(
            colours(Some(&h)).len() > 1,
            "highlighted code is one colour"
        );
        assert_eq!(colours(None).len(), 1, "plain code is not one colour");
    }
}
