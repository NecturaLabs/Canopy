//! The diff viewer: hunks with old/new line numbers, coloured additions and removals, syntax
//! highlighting, unified or side-by-side layout, and stage / unstage / discard per hunk or per
//! selected lines. Rows are virtualised so huge diffs stay fast.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

use canopy_core::ops::HunkAction;
use canopy_core::{ChangeKind, DiffLine, DiffSource, FileDiff, Hunk, LineKind};

use crate::action::Action;
use crate::highlight::{Highlights, Span, Token};
use crate::settings::WhitespaceMode;
use crate::tab::{LineSelection, RepoTab};
use crate::ui::theme::{self, Palette, Syntax, palette};
use crate::ui::widgets;
use crate::worker::Op;

const LINE_H: f32 = 20.0;
const HUNK_H: f32 = 36.0;
/// Height of the hunk action buttons.
const HUNK_BUTTON_H: f32 = 26.0;
/// How far from the left edge the hunk actions may sit, so they stay near the code on very wide
/// panes.
const HUNK_ACTIONS_MAX_X: f32 = 960.0;
/// The id of the diff options popup, which `--overlay diff-options` keeps open for screenshots.
pub const OPTIONS_POPUP: &str = "diff-options";

/// One row of the diff as laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// A hunk header.
    Hunk(usize),
    /// Unified: one line of a hunk.
    Line(usize, usize),
    /// Side by side: the old line (left) and the new line (right) shown on one row.
    Pair(usize, Option<usize>, Option<usize>),
}

/// Lines of a hunk paired for the side-by-side view: context lines face themselves, and each
/// block of removals is matched line by line with the additions that follow it, leaving blanks
/// where one side is longer.
fn pair_lines(hunk: &Hunk) -> Vec<(Option<usize>, Option<usize>)> {
    let mut out = Vec::with_capacity(hunk.lines.len());
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let flush = |out: &mut Vec<(Option<usize>, Option<usize>)>,
                 removed: &mut Vec<usize>,
                 added: &mut Vec<usize>| {
        for i in 0..removed.len().max(added.len()) {
            out.push((removed.get(i).copied(), added.get(i).copied()));
        }
        removed.clear();
        added.clear();
    };
    for (i, line) in hunk.lines.iter().enumerate() {
        match line.kind {
            LineKind::Context => {
                flush(&mut out, &mut removed, &mut added);
                out.push((Some(i), Some(i)));
            }
            LineKind::Removed => {
                // A removal after additions starts a new block.
                if !added.is_empty() {
                    flush(&mut out, &mut removed, &mut added);
                }
                removed.push(i);
            }
            LineKind::Added => added.push(i),
        }
    }
    flush(&mut out, &mut removed, &mut added);
    out
}

fn rows(d: &FileDiff, split: bool) -> Vec<Row> {
    let mut v = Vec::new();
    for (h, hunk) in d.hunks.iter().enumerate() {
        v.push(Row::Hunk(h));
        if split {
            v.extend(
                pair_lines(hunk)
                    .into_iter()
                    .map(|(l, r)| Row::Pair(h, l, r)),
            );
        } else {
            v.extend((0..hunk.lines.len()).map(|l| Row::Line(h, l)));
        }
    }
    v
}

/// A diff's rows and their positions, built once per diff and layout (unified or side by side)
/// instead of every frame. A frame then finds the visible rows by binary search and lays out only
/// those, so its cost does not depend on the diff's size. Galleys are not kept here: egui's own
/// galley cache reuses them while they stay on screen and survives font-atlas rebuilds.
#[derive(Debug)]
pub struct DiffLayout {
    /// The diff this layout is for (compared by pointer).
    diff: Arc<FileDiff>,
    split: bool,
    rows: Vec<Row>,
    /// Top of each row, plus the total height as the last entry.
    tops: Vec<f32>,
    /// The widest line in monospace columns, tabs counted as four.
    columns: usize,
    /// Digits of the largest line number (at least three).
    digits: usize,
}

impl DiffLayout {
    pub fn new(diff: Arc<FileDiff>, split: bool) -> Self {
        let rows = rows(&diff, split);
        let mut tops = Vec::with_capacity(rows.len() + 1);
        let mut y = 0.0;
        for r in &rows {
            tops.push(y);
            y += if matches!(r, Row::Hunk(_)) {
                HUNK_H
            } else {
                LINE_H
            };
        }
        tops.push(y);
        let columns = diff
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .map(|l| l.text.chars().count() + l.text.matches('\t').count() * 3)
            .max()
            .unwrap_or(0);
        let max_no = diff
            .hunks
            .iter()
            .map(|h| (h.old_start + h.old_lines).max(h.new_start + h.new_lines))
            .max()
            .unwrap_or(1);
        Self {
            digits: max_no.to_string().len().max(3),
            diff,
            split,
            rows,
            tops,
            columns,
        }
    }

    /// Still valid for this diff and layout.
    fn is_for(&self, diff: &Arc<FileDiff>, split: bool) -> bool {
        Arc::ptr_eq(&self.diff, diff) && self.split == split
    }

    fn total_height(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.0)
    }

    /// Indices of the rows that intersect `min_y..max_y` (content coordinates).
    fn visible(&self, min_y: f32, max_y: f32) -> std::ops::Range<usize> {
        let n = self.rows.len();
        // Row i spans tops[i]..tops[i + 1].
        let start = self.tops[1..].partition_point(|&bottom| bottom <= min_y);
        let end = self.tops[..n].partition_point(|&top| top < max_y);
        start.min(n)..end.max(start).min(n)
    }
}

fn token_color(s: &Syntax, t: Token) -> Color32 {
    match t {
        Token::Keyword => s.keyword,
        Token::String => s.string,
        Token::Comment => s.comment,
        Token::Number => s.number,
        Token::Function => s.function,
        Token::Type => s.type_,
        Token::Constant => s.constant,
    }
}

/// The line's text as a layout job, coloured by `spans` (byte ranges), tabs as four spaces.
fn code_job(
    text: &str,
    spans: &[Span],
    base: Color32,
    syntax: &Syntax,
    font: &FontId,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut add = |piece: &str, color: Color32| {
        if !piece.is_empty() {
            job.append(
                &piece.replace('\t', "    "),
                0.0,
                TextFormat::simple(font.clone(), color),
            );
        }
    };
    let mut pos = 0;
    for s in spans {
        let (a, b) = (s.start as usize, s.end as usize);
        let (Some(before), Some(inside)) = (text.get(pos..a), text.get(a..b)) else {
            continue;
        };
        add(before, base);
        add(inside, token_color(syntax, s.token));
        pos = b;
    }
    add(text.get(pos..).unwrap_or(""), base);
    job
}

/// What can be done with lines and hunks of this diff.
struct Ctx<'a> {
    p: &'static Palette,
    source: &'a DiffSource,
    path: &'a str,
    conflicted: bool,
    /// Every line is on one side (a new or deleted file): the tint is softened so the whole file
    /// is not one solid slab of colour.
    one_sided: bool,
    /// Hunk and line actions are possible (working copy, whitespace shown).
    editable: bool,
    context: u32,
    highlights: Option<&'a Highlights>,
    mono: FontId,
}

impl Ctx<'_> {
    fn spans(&self, h: usize, l: usize) -> &[Span] {
        self.highlights
            .and_then(|x| x.lines.get(h))
            .and_then(|x| x.get(l))
            .map_or(&[], Vec::as_slice)
    }

    fn selectable(&self, line: &DiffLine) -> bool {
        self.editable && line.kind != LineKind::Context
    }
}

/// Updates the line selection after a click on line `l` of hunk `h`.
fn click_line(
    sel: &mut Option<LineSelection>,
    hunk: &Hunk,
    h: usize,
    l: usize,
    m: egui::Modifiers,
) {
    let changed = |i: &usize| {
        hunk.lines
            .get(*i)
            .is_some_and(|x| x.kind != LineKind::Context)
    };
    match sel {
        Some(s) if s.hunk == h && m.shift => {
            let (a, b) = (s.anchor.min(l), s.anchor.max(l));
            s.lines = (a..=b).filter(changed).collect();
        }
        Some(s) if s.hunk == h && m.command => {
            if !s.lines.remove(&l) {
                s.lines.insert(l);
            }
            s.anchor = l;
            if s.lines.is_empty() {
                *sel = None;
            }
        }
        Some(s) if s.hunk == h && s.lines.len() == 1 && s.lines.contains(&l) => *sel = None,
        _ => {
            *sel = Some(LineSelection {
                hunk: h,
                lines: std::iter::once(l).collect(),
                anchor: l,
            });
        }
    }
}

/// Draws the diff of the selected file.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(sel) = tab.file.clone() else {
        // A clean tree has nothing to select: the list beside this already says so.
        if !tab.snap.as_ref().is_some_and(|s| s.status.is_clean()) {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Select a file to see its diff").color(p.text_faint))
            });
        }
        return;
    };
    header(ui, tab, &sel, actions);

    if let Some(e) = &tab.diff_error {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(e).color(p.danger)));
        return;
    }
    let Some(d) = tab.diff.clone() else {
        tab.diff_layout = None;
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

    let (show_old, show_new) = gutters(&d);
    // A new or deleted file has nothing to set beside its lines: one column, whatever the toggle.
    let split = tab.split_diff && !sel.conflicted && show_old && show_new;
    if !tab
        .diff_layout
        .as_ref()
        .is_some_and(|l| l.is_for(&d, split))
    {
        tab.diff_layout = Some(DiffLayout::new(Arc::clone(&d), split));
    }
    let Some(layout) = tab.diff_layout.as_ref() else {
        return;
    };
    let num_w = layout.digits as f32 * 7.6 + 12.0;
    let gutter_w = num_w * (usize::from(show_old) + usize::from(show_new)) as f32;
    let mono = FontId::monospace(theme::MONO_SIZE);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&mono, 'm'));
    let longest = layout.columns as f32;
    let content_w = if split {
        ui.available_width()
    } else {
        (gutter_w + 24.0 + longest * char_w + 40.0).max(ui.available_width())
    };

    let highlights = if tab.highlight_diff {
        tab.highlights.clone()
    } else {
        None
    };
    let cx = Ctx {
        p,
        source: &sel.source,
        path: &sel.entry.path,
        conflicted: sel.conflicted,
        one_sided: !(show_old && show_new),
        // A whitespace-insensitive diff cannot be applied as a patch.
        editable: !sel.conflicted
            && !matches!(sel.source, DiffSource::Commit(_))
            && tab.diff_prefs.whitespace == WhitespaceMode::Show,
        context: tab.diff_prefs.context,
        highlights: highlights.as_deref(),
        mono,
    };
    let line_sel = &mut tab.line_sel;
    let scroll = if split {
        egui::ScrollArea::vertical()
    } else {
        egui::ScrollArea::both()
    };
    scroll
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .id_salt(("diff", &sel.entry.path, split))
        .show_viewport(ui, |ui, viewport| {
            ui.set_min_size(vec2(content_w, layout.total_height() + 16.0));
            let origin = ui.min_rect().min;
            for i in layout.visible(viewport.min.y, viewport.max.y) {
                let y = layout.tops[i];
                let rh = layout.tops[i + 1] - y;
                let rect = Rect::from_min_size(origin + vec2(0.0, y), vec2(content_w, rh));
                match layout.rows[i] {
                    Row::Hunk(h) => {
                        hunk_header(ui, &cx, &d, h, rect, origin, viewport, line_sel, actions)
                    }
                    Row::Line(h, l) => {
                        let hunk = &d.hunks[h];
                        let line = &hunk.lines[l];
                        let picked = line_sel
                            .as_ref()
                            .is_some_and(|s| s.hunk == h && s.lines.contains(&l));
                        if let Some(m) = line_cell(
                            ui,
                            &cx,
                            line,
                            cx.spans(h, l),
                            rect,
                            num_w,
                            (show_old, show_new),
                            picked,
                            (h, l),
                        ) {
                            click_line(line_sel, hunk, h, l, m);
                        }
                        // Gutter separator, per line so it never crosses a hunk header.
                        ui.painter().vline(
                            origin.x + gutter_w + 2.0,
                            rect.y_range(),
                            Stroke::new(1.0, p.border),
                        );
                    }
                    Row::Pair(h, left, right) => {
                        let hunk = &d.hunks[h];
                        let half = rect.width() / 2.0;
                        let lrect = Rect::from_min_size(rect.min, vec2(half, rh));
                        let rrect = Rect::from_min_size(rect.min + vec2(half, 0.0), vec2(half, rh));
                        for (side, slot, r, nums) in [
                            (0, left, lrect, (true, false)),
                            (1, right, rrect, (false, true)),
                        ] {
                            match slot {
                                Some(l) => {
                                    let line = &hunk.lines[l];
                                    let picked = line_sel
                                        .as_ref()
                                        .is_some_and(|s| s.hunk == h && s.lines.contains(&l));
                                    let clip = ui.painter().clip_rect().intersect(r);
                                    let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(r));
                                    cell.set_clip_rect(clip);
                                    if let Some(m) = line_cell(
                                        &mut cell,
                                        &cx,
                                        line,
                                        cx.spans(h, l),
                                        r,
                                        num_w,
                                        nums,
                                        picked,
                                        (h, l + side * 1_000_000),
                                    ) {
                                        click_line(line_sel, hunk, h, l, m);
                                    }
                                }
                                None => {
                                    // Filler for the shorter side of a change block.
                                    ui.painter().rect_filled(
                                        r,
                                        CornerRadius::ZERO,
                                        Palette::tint(p.text_faint, 14),
                                    );
                                }
                            }
                        }
                        ui.painter().vline(
                            rect.center().x,
                            rect.y_range(),
                            Stroke::new(1.0, p.border),
                        );
                    }
                }
            }
        });
}

/// Which line-number columns a diff shows: a new file has no old numbers and a deleted one no new
/// numbers, so that column is dropped. Hunks alone cannot tell: without context lines a modified
/// file's hunks can all be insertions.
fn gutters(d: &FileDiff) -> (bool, bool) {
    match d.kind {
        Some(ChangeKind::Added | ChangeKind::Untracked) => (false, true),
        Some(ChangeKind::Deleted) => (true, false),
        _ => (true, true),
    }
}

/// The file header: name, counts, layout and options, and the file-level action.
fn header(
    ui: &mut Ui,
    tab: &mut RepoTab,
    sel: &crate::tab::FileSelection,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let compact = ui.available_height() < theme::XL * 10.0;
    let single_row = ui.available_width() >= 720.0;
    let vertical_padding = if compact { theme::XS } else { theme::SM };
    egui::Frame::new()
        .fill(p.panel)
        .inner_margin(egui::Margin::symmetric(
            theme::MD as i8,
            vertical_padding as i8,
        ))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let identity = |ui: &mut Ui| {
                widgets::change_badge(ui, sel.entry.kind);
                let (name, dir) = widgets::split_path(&sel.entry.path);
                ui.add(
                    egui::Label::new(
                        RichText::new(name)
                            .font(FontId::new(theme::LEAD_SIZE, theme::semibold()))
                            .color(p.text),
                    )
                    .truncate(),
                )
                .on_hover_text(&sel.entry.path);
                if !dir.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(dir)
                                .size(theme::META_SIZE)
                                .color(p.text_faint),
                        )
                        .truncate(),
                    )
                    .on_hover_text(dir);
                }
                if let Some(old) = &sel.entry.old_path {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("← {old}"))
                                .size(theme::META_SIZE)
                                .color(p.text_faint),
                        )
                        .truncate(),
                    )
                    .on_hover_text(old);
                }
            };
            let mut controls = |ui: &mut Ui| {
                diff_options(ui, tab, actions);
                if !sel.conflicted {
                    let one_column = tab
                        .diff
                        .as_deref()
                        .is_some_and(|d| gutters(d) != (true, true));
                    layout_toggle(ui, tab.split_diff, one_column, actions);
                }
                if let Some(lang) = tab
                    .highlights
                    .as_ref()
                    .filter(|_| tab.highlight_diff)
                    .map(|h| h.language.clone())
                {
                    ui.label(
                        RichText::new(lang)
                            .size(theme::SMALL_SIZE)
                            .color(p.text_faint),
                    );
                }
                if let Some(d) = &tab.diff {
                    ui.label(
                        RichText::new(format!("+{}", d.additions))
                            .color(p.success)
                            .size(theme::META_SIZE),
                    );
                    ui.label(
                        RichText::new(format!("−{}", d.deletions))
                            .color(p.danger)
                            .size(theme::META_SIZE),
                    );
                }
                let path = sel.entry.path.clone();
                match (&sel.source, sel.conflicted) {
                    (_, true) => {
                        if ui
                            .button(
                                RichText::new(format!("{}  Mark resolved", icon::CHECK))
                                    .color(p.success),
                            )
                            .clicked()
                        {
                            actions.push(Action::Op(Op::Stage(vec![path])));
                        }
                    }
                    (DiffSource::Unstaged, false) => {
                        if ui.button(format!("{}  Stage file", icon::PLUS)).clicked() {
                            actions.push(Action::Op(Op::Stage(vec![path])));
                        }
                    }
                    (DiffSource::Staged, false) => {
                        if ui
                            .button(format!("{}  Unstage file", icon::MINUS))
                            .clicked()
                        {
                            actions.push(Action::Op(Op::Unstage(vec![path])));
                        }
                    }
                    (DiffSource::Commit(_), _) => {}
                }
            };
            if single_row {
                ui.horizontal(|ui| {
                    // The room the controls need, learnt from how they laid out last frame: it
                    // grows while they wrap and settles on what they use, so a font, language
                    // or button change needs no new guess. The numbers are only the first one.
                    let width_id = ui.id().with("diff-controls-width");
                    let controls_width = ui.data(|d| d.get_temp::<f32>(width_id)).unwrap_or(
                        if matches!(sel.source, DiffSource::Commit(_)) {
                            280.0
                        } else {
                            400.0
                        },
                    );
                    let identity_width =
                        (ui.available_width() - controls_width - theme::SM).max(0.0);
                    ui.allocate_ui_with_layout(
                        vec2(identity_width, ui.spacing().interact_size.y),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            identity(ui);
                        },
                    );
                    let laid_out = ui.horizontal_wrapped(controls).response.rect;
                    let learnt = if laid_out.height() > ui.spacing().interact_size.y * 1.5 {
                        controls_width + 40.0
                    } else {
                        laid_out.width().max(1.0)
                    };
                    if (learnt - controls_width).abs() > 0.5 {
                        ui.data_mut(|d| d.insert_temp(width_id, learnt));
                        ui.ctx().request_repaint();
                    }
                });
            } else if compact {
                ui.horizontal_wrapped(|ui| {
                    identity(ui);
                    controls(ui);
                });
            } else {
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(identity);
                    ui.horizontal_wrapped(controls);
                });
            }
        });
    ui.painter().hline(
        ui.min_rect().x_range(),
        ui.min_rect().max.y,
        Stroke::new(1.0, p.border),
    );
}

/// Unified / side-by-side switch. For a new or deleted file, which is always one column, it is
/// shown off on Unified and the choice is kept for the next modified file.
fn layout_toggle(ui: &mut Ui, split: bool, one_column: bool, actions: &mut Vec<Action>) {
    let options = [(icon::ROWS, "Unified"), (icon::COLUMNS, "Side by side")];
    if one_column {
        ui.add_enabled_ui(false, |ui| {
            widgets::icon_segmented(ui, 0, &options);
        })
        .response
        .on_hover_text("A new or deleted file has nothing to set beside it, so it is one column");
    } else if let Some(i) = widgets::icon_segmented(ui, usize::from(split), &options) {
        actions.push(Action::SetSplitDiff(i == 1));
    }
}

/// Paints one line (or one side of a pair). Returns the click modifiers when the user picked it.
#[allow(clippy::too_many_arguments)]
fn line_cell(
    ui: &mut Ui,
    cx: &Ctx<'_>,
    line: &DiffLine,
    spans: &[Span],
    rect: Rect,
    num_w: f32,
    (show_old, show_new): (bool, bool),
    picked: bool,
    id: (usize, usize),
) -> Option<egui::Modifiers> {
    let p = cx.p;
    let marker = cx.conflicted
        && ["<<<<<<<", "=======", ">>>>>>>", "|||||||"]
            .iter()
            .any(|m| line.text.starts_with(m));
    let (bg, fg, sign) = match line.kind {
        LineKind::Added => (Some(p.added_bg), p.success, "+"),
        LineKind::Removed => (Some(p.removed_bg), p.danger, "−"),
        LineKind::Context if marker => (Some(Palette::tint(p.warning, 40)), p.warning, "!"),
        LineKind::Context => (None, p.text_faint, " "),
    };
    let selectable = cx.selectable(line);
    let resp = selectable.then(|| {
        ui.interact(rect, ui.id().with(("line", id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    let painter = ui.painter();
    if let Some(bg) = bg {
        if cx.one_sided {
            // The gutter keeps the full tint and the code a quiet wash of it, so a whole new or
            // deleted file still reads as added or removed without a solid block of colour.
            let gutter_w = num_w * (usize::from(show_old) + usize::from(show_new)) as f32;
            painter.rect_filled(rect, CornerRadius::ZERO, bg.gamma_multiply(0.4));
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(gutter_w, rect.height())),
                CornerRadius::ZERO,
                bg,
            );
        } else {
            painter.rect_filled(rect, CornerRadius::ZERO, bg);
        }
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            CornerRadius::ZERO,
            fg,
        );
    }
    if resp.as_ref().is_some_and(egui::Response::hovered) {
        painter.rect_filled(rect, CornerRadius::ZERO, Palette::tint(p.text, 10));
    }
    if picked {
        painter.rect_filled(
            rect,
            CornerRadius::ZERO,
            Palette::tint(p.accent, if p.dark { 46 } else { 34 }),
        );
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(4.0, rect.height())),
            CornerRadius::ZERO,
            p.accent,
        );
    }
    let cy = rect.center().y;
    let nf = FontId::monospace(theme::MONO_SMALL_SIZE);
    let mut x = rect.min.x;
    if show_old {
        if let Some(n) = line.old_no {
            painter.text(
                pos2(x + num_w - 4.0, cy),
                Align2::RIGHT_CENTER,
                n.to_string(),
                nf.clone(),
                p.text_faint,
            );
        }
        x += num_w;
    }
    if show_new {
        if let Some(n) = line.new_no {
            painter.text(
                pos2(x + num_w - 4.0, cy),
                Align2::RIGHT_CENTER,
                n.to_string(),
                nf,
                p.text_faint,
            );
        }
        x += num_w;
    }
    painter.text(
        pos2(x + 8.0, cy),
        Align2::LEFT_CENTER,
        sign,
        cx.mono.clone(),
        fg,
    );
    let base = match line.kind {
        LineKind::Context if !marker => p.text_dim,
        _ => p.text,
    };
    let job = if marker {
        code_job(&line.text, &[], base, &p.syntax, &cx.mono)
    } else {
        code_job(&line.text, spans, base, &p.syntax, &cx.mono)
    };
    let galley = painter.layout_job(job);
    painter.galley(pos2(x + 24.0, cy - galley.size().y / 2.0), galley, base);
    let resp = resp?;
    if resp.clicked() {
        return Some(ui.input(|i| i.modifiers));
    }
    None
}

/// A hunk header band with its actions (or the actions for the selected lines in it).
#[allow(clippy::too_many_arguments)]
fn hunk_header(
    ui: &mut Ui,
    cx: &Ctx<'_>,
    d: &FileDiff,
    h: usize,
    rect: Rect,
    origin: egui::Pos2,
    viewport: Rect,
    line_sel: &mut Option<LineSelection>,
    actions: &mut Vec<Action>,
) {
    let p = cx.p;
    let hunk = &d.hunks[h];
    let band = rect.shrink2(vec2(0.0, 4.0));
    ui.painter().rect_filled(
        band,
        CornerRadius::ZERO,
        Palette::tint(p.accent, if p.dark { 22 } else { 16 }),
    );
    let visible_left = viewport.min.x + origin.x;
    let right = origin.x + viewport.max.x - 10.0;
    if cx.conflicted || matches!(cx.source, DiffSource::Commit(_)) {
        ui.painter().text(
            pos2(visible_left + theme::MD, band.center().y),
            Align2::LEFT_CENTER,
            &hunk.header,
            FontId::monospace(theme::MONO_SMALL_SIZE),
            p.text_dim,
        );
        return;
    }
    if !cx.editable {
        let note = "Actions are off while whitespace is ignored";
        let note_width = ui
            .painter()
            .layout_no_wrap(
                note.into(),
                FontId::proportional(theme::SMALL_SIZE),
                p.text_faint,
            )
            .size()
            .x;
        let show_note = right - visible_left > note_width + 160.0;
        let header_right = if show_note {
            right - note_width - theme::LG
        } else {
            right
        };
        let header = widgets::single_line_galley(
            ui,
            hunk.header.clone(),
            FontId::monospace(theme::MONO_SMALL_SIZE),
            p.text_dim,
            header_right - (visible_left + theme::MD),
        );
        ui.painter().galley(
            pos2(
                visible_left + theme::MD,
                band.center().y - header.size().y / 2.0,
            ),
            header,
            p.text_dim,
        );
        if show_note {
            ui.painter().text(
                pos2(right, band.center().y),
                Align2::RIGHT_CENTER,
                note,
                FontId::proportional(theme::SMALL_SIZE),
                p.text_faint,
            );
        }
        return;
    }
    let picked: Vec<usize> = line_sel
        .as_ref()
        .filter(|s| s.hunk == h)
        .map(|s| s.lines.iter().copied().collect())
        .unwrap_or_default();
    let n = picked.len();
    let lines_word = if n == 1 { "line" } else { "lines" };
    let buttons: Vec<(Option<HunkAction>, &str, String)> = match (cx.source, n) {
        (DiffSource::Unstaged, 0) => vec![
            (Some(HunkAction::Stage), icon::PLUS, "Stage hunk".into()),
            (
                Some(HunkAction::Discard),
                icon::ARROW_COUNTER_CLOCKWISE,
                "Discard hunk".into(),
            ),
        ],
        (DiffSource::Staged, 0) => vec![(
            Some(HunkAction::Unstage),
            icon::MINUS,
            "Unstage hunk".into(),
        )],
        (DiffSource::Unstaged, _) => vec![
            (None, icon::X, "Clear".into()),
            (
                Some(HunkAction::Stage),
                icon::PLUS,
                format!("Stage {n} {lines_word}"),
            ),
            (
                Some(HunkAction::Discard),
                icon::ARROW_COUNTER_CLOCKWISE,
                format!("Discard {n} {lines_word}"),
            ),
        ],
        (DiffSource::Staged, _) => vec![
            (None, icon::X, "Clear".into()),
            (
                Some(HunkAction::Unstage),
                icon::MINUS,
                format!("Unstage {n} {lines_word}"),
            ),
        ],
        (DiffSource::Commit(_), _) => Vec::new(),
    };
    let mut bx = right.min(visible_left + HUNK_ACTIONS_MAX_X);
    for (action, glyph, label) in buttons.iter().rev() {
        let text = if viewport.width() < 420.0 {
            glyph.to_string()
        } else {
            format!("{glyph}  {label}")
        };
        let g = ui.painter().layout_no_wrap(
            text,
            FontId::new(theme::META_SIZE, theme::medium()),
            Color32::PLACEHOLDER,
        );
        let br = Rect::from_min_max(
            pos2(
                bx - g.size().x - 16.0,
                band.center().y - HUNK_BUTTON_H / 2.0,
            ),
            pos2(bx, band.center().y + HUNK_BUTTON_H / 2.0),
        );
        let resp = ui
            .interact(
                br,
                ui.id().with(("hunk", h, label.as_str())),
                Sense::click(),
            )
            .on_hover_text(label);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let danger = *action == Some(HunkAction::Discard);
        let hover = resp.hovered();
        // Stage is the one filled button, Discard a ghost that only turns red under the pointer,
        // and the rest are neutral, so the two never read as the same warm pill.
        let (fill, edge, ink) = match action {
            Some(HunkAction::Stage) => (
                Palette::tint(p.success, if hover { 90 } else { 48 }),
                Palette::tint(p.success, 150),
                p.text,
            ),
            Some(HunkAction::Discard) if hover => (
                Palette::tint(p.danger, 70),
                Palette::tint(p.danger, 150),
                p.danger,
            ),
            Some(HunkAction::Discard) => (
                Color32::TRANSPARENT,
                Color32::TRANSPARENT,
                p.text_dim.lerp_to_gamma(p.danger, 0.3),
            ),
            _ => (
                Palette::tint(p.text, if hover { 30 } else { 14 }),
                Palette::tint(p.text, 60),
                p.text,
            ),
        };
        ui.painter().rect(
            br,
            CornerRadius::same(theme::RADIUS_SM),
            fill,
            Stroke::new(1.0, edge),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(br.center() - g.size() / 2.0, g, ink);
        if resp.clicked() {
            match action {
                None => *line_sel = None,
                Some(action) => {
                    let path = cx.path.to_owned();
                    let op = if n == 0 {
                        Op::Hunk {
                            action: *action,
                            path: path.clone(),
                            index: h,
                            key: hunk.key(),
                            context: cx.context,
                        }
                    } else {
                        Op::Lines {
                            action: *action,
                            path: path.clone(),
                            index: h,
                            key: hunk.key(),
                            lines: picked.clone(),
                            context: cx.context,
                        }
                    };
                    if danger {
                        let what = if n == 0 {
                            "this change".to_owned()
                        } else {
                            format!("the {n} selected {lines_word}")
                        };
                        actions.push(Action::Confirm {
                            title: if n == 0 {
                                "Discard hunk".into()
                            } else {
                                format!("Discard {lines_word}")
                            },
                            body: format!("Throw away {what} in {path}? It cannot be recovered."),
                            confirm: "Discard".into(),
                            danger: true,
                            op,
                        });
                    } else {
                        actions.push(Action::Op(op));
                    }
                }
            }
        }
        // Discard sits apart from Stage so a slip toward one never lands on the other.
        bx = br.min.x - if danger { theme::LG } else { theme::SM };
    }
    let ink = p.accent.lerp_to_gamma(p.text_dim, 0.4);
    let header = widgets::single_line_galley(
        ui,
        hunk.header.clone(),
        FontId::monospace(theme::MONO_SMALL_SIZE),
        ink,
        bx - theme::SM - (visible_left + 12.0),
    );
    ui.painter().galley(
        pos2(visible_left + 12.0, band.center().y - header.size().y / 2.0),
        header,
        ink,
    );
    if n == 0 && h == 0 && bx - visible_left > 360.0 {
        ui.painter().text(
            pos2(bx - 6.0, band.center().y),
            Align2::RIGHT_CENTER,
            "Click lines to pick part of a hunk",
            FontId::proportional(theme::SMALL_SIZE),
            p.text_faint,
        );
    }
}

/// Context lines and whitespace handling for this repository's diffs.
fn diff_options(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let prefs = &mut tab.diff_prefs;
    let mut changed = false;
    let lines = if prefs.context == 1 { "line" } else { "lines" };
    let resp = ui
        .add(egui::Button::new(
            RichText::new(format!(
                "{}  {} {lines}",
                icon::SLIDERS_HORIZONTAL,
                prefs.context
            ))
            .size(theme::META_SIZE)
            .color(p.text_dim),
        ))
        .on_hover_text("Diff options: context lines and whitespace (this repository)");
    egui::Popup::menu(&resp)
        .id(egui::Id::new(OPTIONS_POPUP))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(300.0);
            ui.add_space(4.0);
            ui.label(
                RichText::new("Diff options")
                    .font(FontId::new(theme::BODY_SIZE, theme::semibold())),
            );
            ui.label(
                RichText::new("For this repository")
                    .size(theme::META_SIZE)
                    .color(p.text_faint),
            );
            ui.add_space(theme::MD);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Context lines").color(p.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(prefs.context.to_string())
                            .font(FontId::new(theme::BODY_SIZE, theme::semibold())),
                    );
                });
            });
            changed |= ui
                .scope(|ui| {
                    widgets::recessed_rail(ui);
                    ui.spacing_mut().slider_width = ui.available_width();
                    ui.add(egui::Slider::new(&mut prefs.context, 0..=25).show_value(false))
                })
                .inner
                .changed();
            ui.add_space(theme::MD);
            ui.label(RichText::new("Whitespace").color(p.text_dim));
            ui.add_space(2.0);
            changed |= widgets::segmented(
                ui,
                &mut prefs.whitespace,
                &[
                    (WhitespaceMode::Show, "Show"),
                    (WhitespaceMode::IgnoreEol, "Ignore trailing"),
                    (WhitespaceMode::IgnoreAll, "Ignore all"),
                ],
            );
            if prefs.whitespace != WhitespaceMode::Show {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Hunk staging is off while whitespace is ignored.")
                        .size(theme::META_SIZE)
                        .color(p.text_faint),
                );
            }
            ui.add_space(4.0);
        });
    if changed {
        actions.push(Action::DiffPrefsChanged);
    }
}

fn empty(ui: &mut Ui, glyph: &str, title: &str, body: &str) {
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(
            RichText::new(glyph)
                .size(theme::ICON_HERO)
                .color(p.text_faint),
        );
        ui.label(RichText::new(title).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
        ui.label(RichText::new(body).color(p.text_dim));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(kinds: &str) -> Hunk {
        Hunk {
            header: "@@".into(),
            old_start: 1,
            old_lines: 0,
            new_start: 1,
            new_lines: 0,
            lines: kinds
                .chars()
                .map(|c| DiffLine {
                    kind: match c {
                        '+' => LineKind::Added,
                        '-' => LineKind::Removed,
                        _ => LineKind::Context,
                    },
                    old_no: None,
                    new_no: None,
                    text: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn only_a_new_or_deleted_file_drops_a_line_number_column() {
        let diff = |kind| FileDiff {
            kind: Some(kind),
            hunks: vec![Hunk {
                header: String::new(),
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: 3,
                lines: Vec::new(),
            }],
            ..FileDiff::default()
        };
        assert_eq!(gutters(&diff(ChangeKind::Added)), (false, true));
        assert_eq!(gutters(&diff(ChangeKind::Untracked)), (false, true));
        assert_eq!(gutters(&diff(ChangeKind::Deleted)), (true, false));
        // Without context lines, a modified file's hunks can all be pure insertions.
        assert_eq!(gutters(&diff(ChangeKind::Modified)), (true, true));
    }

    #[test]
    fn ignoring_whitespace_keeps_the_hunk_coordinates_visible() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let d = FileDiff {
            hunks: vec![hunk("+")],
            ..Default::default()
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let cx = Ctx {
                p: palette(ui.ctx()),
                source: &DiffSource::Unstaged,
                path: "file.rs",
                conflicted: false,
                one_sided: false,
                editable: false,
                context: 3,
                highlights: None,
                mono: FontId::monospace(theme::MONO_SIZE),
            };
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, HUNK_H));
            hunk_header(
                ui,
                &cx,
                &d,
                0,
                rect,
                egui::Pos2::ZERO,
                rect,
                &mut None,
                &mut Vec::new(),
            );
        });
        output.textures_delta.clear();
        assert!(
            output.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Text(t) if t.galley.text() == "@@")),
            "hunk coordinates disappeared"
        );
    }

    #[test]
    fn pairs_align_changes_with_fillers() {
        // context, 2 removed, 3 added, context
        let h = hunk(" --+++ ");
        assert_eq!(
            pair_lines(&h),
            vec![
                (Some(0), Some(0)),
                (Some(1), Some(3)),
                (Some(2), Some(4)),
                (None, Some(5)),
                (Some(6), Some(6)),
            ]
        );
        // Removal after an addition starts a new block.
        assert_eq!(
            pair_lines(&hunk("+-")),
            vec![(None, Some(0)), (Some(1), None)]
        );
        assert_eq!(pair_lines(&hunk("---")).len(), 3);
    }

    #[test]
    fn layout_positions_rows_and_finds_the_visible_ones() {
        let mut a = hunk(" --+++ ");
        a.lines[0].text = "\tx".into();
        let d = Arc::new(FileDiff {
            hunks: vec![a, hunk("+")],
            ..FileDiff::default()
        });
        let l = DiffLayout::new(Arc::clone(&d), false);
        // Hunk, 7 lines, hunk, 1 line.
        assert_eq!(l.rows.len(), 10);
        assert_eq!(l.tops[1], HUNK_H);
        assert_eq!(l.tops[8], HUNK_H + 7.0 * LINE_H);
        assert_eq!(l.total_height(), 2.0 * HUNK_H + 8.0 * LINE_H);
        assert_eq!(l.columns, 5, "a tab counts as four columns");
        assert_eq!(l.visible(0.0, 1.0), 0..1);
        // The bottom edge of a row is not inside it.
        assert_eq!(l.visible(HUNK_H, HUNK_H + LINE_H), 1..2);
        assert_eq!(
            l.visible(HUNK_H + 0.5 * LINE_H, HUNK_H + 2.5 * LINE_H),
            1..4
        );
        assert_eq!(l.visible(0.0, 10_000.0), 0..10);
        assert_eq!(l.visible(10_000.0, 20_000.0), 10..10);
        // Side by side pairs the change block.
        let s = DiffLayout::new(Arc::clone(&d), true);
        assert_eq!(s.rows.len(), 1 + 5 + 1 + 1);
        // Reused only for the same diff and layout.
        assert!(l.is_for(&d, false));
        assert!(!l.is_for(&d, true));
        assert!(!l.is_for(&Arc::new((*d).clone()), false));
    }

    #[test]
    fn huge_diffs_lay_out_once_and_scroll_by_lookup() {
        let lines = hunk(&" +-".repeat(20_000));
        let d = Arc::new(FileDiff {
            hunks: vec![lines.clone(), lines],
            ..FileDiff::default()
        });
        let l = DiffLayout::new(d, false);
        assert_eq!(l.rows.len(), 120_002);
        let mid = l.total_height() / 2.0;
        let r = l.visible(mid, mid + 800.0);
        assert!(r.len() <= 42, "{r:?}");
        assert!(l.tops[r.start] <= mid && l.tops[r.end] >= mid + 800.0);
    }

    #[test]
    fn clicks_select_ranges_within_one_hunk() {
        let h = hunk(" ++ ++");
        let none = egui::Modifiers::NONE;
        let shift = egui::Modifiers::SHIFT;
        let ctrl = egui::Modifiers::COMMAND;
        let mut sel = None;
        click_line(&mut sel, &h, 0, 1, none);
        click_line(&mut sel, &h, 0, 5, shift);
        let s = sel.clone().expect("selection");
        assert_eq!(
            s.lines.into_iter().collect::<Vec<_>>(),
            vec![1, 2, 4, 5],
            "context skipped"
        );
        click_line(&mut sel, &h, 0, 2, ctrl);
        assert!(!sel.as_ref().expect("still").lines.contains(&2));
        // A click in another hunk starts over.
        click_line(&mut sel, &h, 1, 4, none);
        assert_eq!(sel.as_ref().map(|s| s.hunk), Some(1));
        // Clicking the only selected line clears it.
        click_line(&mut sel, &h, 1, 4, none);
        assert!(sel.is_none());
    }

    #[test]
    fn code_job_colours_spans_and_expands_tabs() {
        let syntax = crate::ui::theme::MIDNIGHT.syntax;
        let spans = [Span {
            start: 1,
            end: 3,
            token: Token::Keyword,
        }];
        let job = code_job(
            "\tfn x",
            &spans,
            Color32::WHITE,
            &syntax,
            &FontId::monospace(12.0),
        );
        assert_eq!(job.text, "    fn x");
        assert_eq!(job.sections.len(), 3);
        assert_eq!(job.sections[1].format.color, syntax.keyword);
    }
}
