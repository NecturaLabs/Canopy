//! The diff viewer: hunks with old/new line numbers, coloured additions and removals, syntax
//! highlighting, unified or side-by-side layout, and stage / unstage / discard per hunk or per
//! selected lines. Rows are virtualised so huge diffs stay fast.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Ui, UiBuilder,
    pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::ops::HunkAction;
use canopy_core::{ChangeKind, DiffLine, DiffSource, FileDiff, Hunk, LineKind};

use crate::action::Action;
use crate::highlight::{Highlights, Span, Token};
use crate::settings::WhitespaceMode;
use crate::tab::{LineSelection, RepoTab};
use crate::ui::kit::chip::{ChangeBadge, DiffStat};
use crate::ui::kit::choice::Segmented;
use crate::ui::kit::overlay::Popover;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::range::Slider;
use crate::ui::kit::surface::EmptyState;
use crate::ui::kit::{Button, IconButton, Size, Tone, paint};
use crate::ui::theme::{self, Palette, Syntax, palette};
use crate::ui::widgets;
use crate::worker::Op;

/// How far from the left edge the hunk actions may sit, so they stay near the code on very wide
/// panes.
const HUNK_ACTIONS_MAX_X: f32 = 960.0;
/// Below this pane width the hunk actions show their icons only.
const HUNK_ACTIONS_ICON_ONLY: f32 = 420.0;

/// Heights of a diff line and of a hunk header in the active density.
fn row_heights() -> (f32, f32) {
    let m = theme::metrics();
    (m.diff_line_height, m.hunk_header_height)
}

/// Bounds the initial inspector height without walking the lines of a large diff.
pub(super) fn inspector_height(d: &FileDiff, limit: f32) -> f32 {
    let (line_h, hunk_h) = row_heights();
    let mut height = theme::XL * 4.0;
    for hunk in &d.hunks {
        height += hunk_h + crate::ui::count_f32(hunk.lines.len()) * line_h;
        if height >= limit {
            return limit;
        }
    }
    height.min(limit)
}
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
    /// Line and hunk header heights the rows were placed with (they follow the density).
    line_h: f32,
    hunk_h: f32,
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
        let (line_h, hunk_h) = row_heights();
        let mut tops = Vec::with_capacity(rows.len() + 1);
        let mut y = 0.0;
        for r in &rows {
            tops.push(y);
            y += if matches!(r, Row::Hunk(_)) {
                hunk_h
            } else {
                line_h
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
            line_h,
            hunk_h,
            rows,
            tops,
            columns,
        }
    }

    /// Still valid for this diff and layout in the active density.
    fn is_for(&self, diff: &Arc<FileDiff>, split: bool) -> bool {
        Arc::ptr_eq(&self.diff, diff)
            && self.split == split
            && (self.line_h, self.hunk_h) == row_heights()
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
    let s = p.s();
    let m = theme::metrics();
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
        ui.vertical_centered(|ui| {
            Spinner::new().label("the diff").active(true).show(ui);
        });
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
    let num_w = crate::ui::count_f32(layout.digits) * 7.6 + 12.0;
    let gutter_w = num_w * crate::ui::count_f32(usize::from(show_old) + usize::from(show_new));
    let mono = FontId::monospace(m.mono_size);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&mono, 'm'));
    let longest = crate::ui::count_f32(layout.columns);
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
                        hunk_header(ui, &cx, &d, h, rect, origin, viewport, line_sel, actions);
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
                        paint::seam_v(
                            ui.painter(),
                            origin.x + gutter_w + 2.0,
                            rect.y_range(),
                            s.border,
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
                                    let mut cell = ui.new_child(UiBuilder::new().max_rect(r));
                                    cell.set_clip_rect(clip);
                                    if let Some(m) = line_cell(
                                        &cell,
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
                                        p.hover_strong(),
                                    );
                                }
                            }
                        }
                        paint::seam_v(ui.painter(), rect.center().x, rect.y_range(), s.border);
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
    let m = theme::metrics();
    let compact = ui.available_height() < theme::XL * 10.0;
    let single_row = ui.available_width() >= 720.0;
    // One row of small controls fills `file_header_height`; wrapped rows get the usual gaps.
    let vertical_padding = if single_row {
        ((m.file_header_height - m.control_sm) / 2.0).max(0.0)
    } else if compact {
        theme::XS
    } else {
        theme::SM
    };
    let frame = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(
            crate::ui::points_i8(theme::MD),
            crate::ui::points_i8(vertical_padding),
        ))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let identity = |ui: &mut Ui| {
                ChangeBadge::new(sel.entry.kind).show(ui);
                let (name, dir) = widgets::split_path(&sel.entry.path);
                ui.add(
                    egui::Label::new(
                        RichText::new(name)
                            .font(FontId::new(m.lead_size, theme::semibold()))
                            .color(p.text),
                    )
                    .truncate(),
                )
                .kit_tooltip(&sel.entry.path)
                .show();
                if !dir.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(dir).size(m.meta_size).color(p.text_faint))
                            .truncate(),
                    )
                    .kit_tooltip(dir)
                    .show();
                }
                if let Some(old) = &sel.entry.old_path {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("← {old}"))
                                .size(m.meta_size)
                                .color(p.text_faint),
                        )
                        .truncate(),
                    )
                    .kit_tooltip(old)
                    .show();
                }
            };
            let mut controls = |ui: &mut Ui| {
                diff_options(ui, tab, actions);
                if !sel.conflicted {
                    let one_column = tab
                        .diff
                        .as_deref()
                        .is_some_and(|d| gutters(d) != (true, true));
                    layout_toggle(ui, tab.split_diff, one_column, single_row, actions);
                }
                if let Some(d) = &tab.diff {
                    DiffStat::new(d.additions, d.deletions).show(ui);
                }
                let path = sel.entry.path.clone();
                let file_action = |label: &str, glyph: &str| {
                    Button::new(label)
                        .secondary()
                        .size(Size::Sm)
                        .icon(glyph)
                        .id_salt(("diff-file-action", label.to_owned()))
                };
                match (&sel.source, sel.conflicted) {
                    (_, true) => {
                        if file_action("Mark resolved", icon::CHECK)
                            .icon_tone(Tone::Success)
                            .show(ui)
                            .clicked()
                        {
                            actions.push(Action::Op(Op::Stage(vec![path])));
                        }
                    }
                    (DiffSource::Unstaged, false) => {
                        if file_action("Stage file", icon::PLUS).show(ui).clicked() {
                            actions.push(Action::Op(Op::Stage(vec![path])));
                        }
                    }
                    (DiffSource::Staged, false) => {
                        if file_action("Unstage file", icon::MINUS).show(ui).clicked() {
                            actions.push(Action::Op(Op::Unstage(vec![path])));
                        }
                    }
                    (DiffSource::Commit(_), _) => {}
                }
            };
            if single_row {
                ui.horizontal(|ui| {
                    // The room the controls need, learnt from how they laid out last frame: it
                    // grows while they wrap and settles on what they use, so a font
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
                        vec2(identity_width, m.control_sm),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            identity(ui);
                        },
                    );
                    let laid_out = ui.horizontal_wrapped(controls).response.rect;
                    let learnt = if laid_out.height() > m.control_sm * 1.5 {
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
    let rect = frame.response.rect;
    paint::seam_h(
        ui.painter(),
        rect.x_range(),
        rect.bottom() - paint::SEAM * 0.5,
        p.s().border,
    );
}

/// Unified / side-by-side switch, with labels when the header has room for them. For a new or
/// deleted file, which is always one column, it is shown off on Unified and the choice is kept
/// for the next modified file.
fn layout_toggle(
    ui: &mut Ui,
    split: bool,
    one_column: bool,
    labels: bool,
    actions: &mut Vec<Action>,
) {
    const OPTIONS: [(bool, &str, &str); 2] = [
        (false, icon::ROWS, "Unified"),
        (true, icon::COLUMNS, "Side by side"),
    ];
    let mut value = split && !one_column;
    let segmented = if labels {
        Segmented::with_icons(&mut value, &OPTIONS)
    } else {
        Segmented::icons(&mut value, &OPTIONS)
    };
    // A new or deleted file stays on Unified, chosen and lit; only Side by side is off.
    let changed = segmented
        .size(Size::Sm)
        .label("Diff layout")
        .id_salt("diff-layout")
        .option_enabled(
            1,
            !one_column,
            Some("A new or deleted file has nothing to set beside it, so it is one column"),
        )
        .show(ui)
        .changed();
    if changed {
        actions.push(Action::SetSplitDiff(value));
    }
}

/// Paints one line (or one side of a pair). Returns the click modifiers when the user picked it.
#[expect(
    clippy::too_many_arguments,
    reason = "a cell needs its line, spans, rectangle, gutter, sides, selection and id"
)]
fn line_cell(
    ui: &Ui,
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
    let s = p.s();
    let marker = cx.conflicted
        && ["<<<<<<<", "=======", ">>>>>>>", "|||||||"]
            .iter()
            .any(|m| line.text.starts_with(m));
    // The line's ground, its line-number gutter's ground, and the sign's colour.
    let (bg, gutter, fg, sign) = match line.kind {
        LineKind::Added => (Some(s.added_bg), s.added_gutter, p.success, "+"),
        LineKind::Removed => (Some(s.removed_bg), s.removed_gutter, p.danger, "−"),
        LineKind::Context if marker => {
            let tint = p.warning_tint();
            (Some(tint), tint, p.warning, "!")
        }
        LineKind::Context => (None, Color32::TRANSPARENT, p.text_faint, " "),
    };
    let selectable = cx.selectable(line);
    let resp = selectable.then(|| {
        ui.interact(rect, ui.id().with(("line", id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    // The code is painted, so each visible line is also a node a screen reader reads: a line
    // that can be picked as a toggle (picked or not), the others as text.
    match &resp {
        Some(r) => r.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                picked,
                spoken_line(line, marker),
            )
        }),
        None => ui
            .interact(rect, ui.id().with(("line", id)), Sense::hover())
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, spoken_line(line, marker))
            }),
    }
    let painter = ui.painter();
    if let Some(bg) = bg {
        // The line numbers sit on the stronger gutter tint. A whole new or deleted file keeps
        // that and gives its code a quiet wash of the line tint, so it still reads as added or
        // removed without a solid block of colour.
        let gutter_w = num_w * crate::ui::count_f32(usize::from(show_old) + usize::from(show_new));
        let body = if cx.one_sided {
            bg.gamma_multiply(0.4)
        } else {
            bg
        };
        painter.rect_filled(rect, CornerRadius::ZERO, body);
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(gutter_w, rect.height())),
            CornerRadius::ZERO,
            gutter,
        );
    }
    if resp.as_ref().is_some_and(egui::Response::hovered) {
        painter.rect_filled(rect, CornerRadius::ZERO, p.hover());
    }
    if picked {
        painter.rect_filled(rect, CornerRadius::ZERO, p.selected());
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(PICKED_MARK, rect.height())),
            CornerRadius::ZERO,
            p.accent,
        );
    }
    let cy = rect.center().y;
    let nf = FontId::monospace(theme::metrics().small_size);
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
    let text_right = x + 24.0 + galley.size().x;
    painter.galley(pos2(x + 24.0, cy - galley.size().y / 2.0), galley, base);
    // A line that runs past the visible edge fades out there, so it reads as continuing (scroll
    // sideways, or widen the pane) rather than as cut off.
    let edge = painter.clip_rect().max.x;
    if text_right > edge + 0.5 {
        let under = match bg {
            Some(bg) if cx.one_sided => theme::over(bg.gamma_multiply(0.4), s.pane),
            Some(bg) => theme::over(bg, s.pane),
            None => s.pane,
        };
        fade_edge(painter, rect.y_range(), edge, under);
    }
    let resp = resp?;
    if resp.clicked() {
        return Some(ui.input(|i| i.modifiers));
    }
    None
}

/// What a screen reader says for a diff line: how it changed, its number and its text.
fn spoken_line(line: &DiffLine, marker: bool) -> String {
    let text = if line.text.trim().is_empty() {
        "blank"
    } else {
        line.text.as_str()
    };
    let number = |n: Option<u32>| n.map_or_else(String::new, |n| format!(" {n}"));
    match line.kind {
        _ if marker => format!("Conflict marker: {text}"),
        LineKind::Added => format!("Added line{}: {text}", number(line.new_no)),
        LineKind::Removed => format!("Removed line{}: {text}", number(line.old_no)),
        LineKind::Context => format!("Line{}: {text}", number(line.new_no.or(line.old_no))),
    }
}

/// The first hunk's note on picking lines.
const PICK_HINT: &str = "Click lines to pick part of a hunk";

/// Width of the mark at the left of a picked line.
const PICKED_MARK: f32 = 4.0;

/// Fades from clear to `under` over the last pixels left of `edge`.
fn fade_edge(painter: &egui::Painter, y: egui::Rangef, edge: f32, under: Color32) {
    const FADE_W: f32 = 28.0;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(pos2(edge - FADE_W, y.min), Color32::TRANSPARENT);
    mesh.colored_vertex(pos2(edge, y.min), under);
    mesh.colored_vertex(pos2(edge, y.max), under);
    mesh.colored_vertex(pos2(edge - FADE_W, y.max), Color32::TRANSPARENT);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// A hunk header band with its actions (or the actions for the selected lines in it).
#[expect(
    clippy::too_many_arguments,
    reason = "the band needs its hunk, geometry, viewport, line selection and actions"
)]
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
    let m = theme::metrics();
    let hunk = &d.hunks[h];
    let band = rect;
    // The header is painted: its text reaches a screen reader through a node of its own.
    ui.interact(band, ui.id().with(("hunk-header", h)), Sense::hover())
        .widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                format!("Hunk {}: {}", h + 1, hunk.header),
            )
        });
    // Concept 1's `.hunk`: the accent at about 7%.
    ui.painter().rect_filled(
        band,
        CornerRadius::ZERO,
        p.accent_tint().gamma_multiply(0.45),
    );
    let coordinates = FontId::monospace(m.small_size);
    let note_font = FontId::proportional(m.small_size);
    let visible_left = viewport.min.x + origin.x;
    let right = origin.x + viewport.max.x - 10.0;
    if cx.conflicted || matches!(cx.source, DiffSource::Commit(_)) {
        ui.painter().text(
            pos2(visible_left + theme::MD, band.center().y),
            Align2::LEFT_CENTER,
            &hunk.header,
            coordinates,
            p.text_faint,
        );
        return;
    }
    if !cx.editable {
        let note = "Actions are off while whitespace is ignored";
        let note_width = ui
            .painter()
            .layout_no_wrap(note.into(), note_font.clone(), p.text_faint)
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
            coordinates,
            p.text_faint,
            header_right - (visible_left + theme::MD),
        );
        ui.painter().galley(
            pos2(
                visible_left + theme::MD,
                band.center().y - header.size().y / 2.0,
            ),
            header,
            p.text_faint,
        );
        if show_note {
            ui.painter().text(
                pos2(right, band.center().y),
                Align2::RIGHT_CENTER,
                note,
                note_font,
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
                "Discard hunk…".into(),
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
                format!("Discard {n} {lines_word}…"),
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
    let actions_right = right.min(visible_left + HUNK_ACTIONS_MAX_X);
    let area = Rect::from_min_max(
        pos2(visible_left, band.top()),
        pos2(actions_right.max(visible_left), band.bottom()),
    );
    let mut row = ui.new_child(
        UiBuilder::new()
            .id_salt(("hunk-actions", h))
            .max_rect(area)
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 0.0;
    let icon_only = viewport.width() < HUNK_ACTIONS_ICON_ONLY;
    let mut gap = 0.0;
    for (action, glyph, label) in buttons.iter().rev() {
        row.add_space(gap);
        let danger = *action == Some(HunkAction::Discard);
        let tone = danger.then_some(Tone::Danger);
        let salt = ("hunk", h, label.as_str());
        let resp = if icon_only {
            let mut b = IconButton::new(glyph, label.as_str())
                .size(Size::Sm)
                .id_salt(salt);
            if let Some(t) = tone {
                b = b.tone(t);
            }
            b.show(&mut row)
        } else {
            let mut b = Button::new(label.as_str())
                .ghost()
                .size(Size::Sm)
                .icon(glyph)
                .tooltip(label.as_str())
                .id_salt(salt);
            if let Some(t) = tone {
                b = b.icon_tone(t);
            }
            b.show(&mut row)
        };
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
                        let (title, confirm) = if n == 0 {
                            (
                                format!("Discard this hunk of {path}?"),
                                "Discard hunk".to_owned(),
                            )
                        } else {
                            (
                                format!("Discard {n} selected {lines_word} of {path}?"),
                                format!("Discard {lines_word}"),
                            )
                        };
                        actions.push(Action::Confirm {
                            title,
                            body: "The change is thrown away. It cannot be recovered.".into(),
                            confirm,
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
        gap = if danger { theme::LG } else { theme::SM };
    }
    let bx = if buttons.is_empty() {
        actions_right
    } else {
        row.min_rect().left()
    };
    let title_left = visible_left + 12.0;
    let title_room = bx - theme::SM - title_left;
    // The first hunk explains line picking, but only where the whole title leaves room for it:
    // the hint never paints over the title.
    let hint = (n == 0 && h == 0)
        .then(|| {
            ui.painter()
                .layout_no_wrap(PICK_HINT.to_owned(), note_font.clone(), p.text_faint)
        })
        .filter(|hint| {
            let title_w = ui
                .painter()
                .layout_no_wrap(hunk.header.clone(), coordinates.clone(), p.text_faint)
                .size()
                .x;
            title_w + theme::LG + hint.size().x <= title_room
        });
    let header = widgets::single_line_galley(
        ui,
        hunk.header.clone(),
        coordinates,
        p.text_faint,
        title_room,
    );
    ui.painter().galley(
        pos2(title_left, band.center().y - header.size().y / 2.0),
        header,
        p.text_faint,
    );
    if let Some(hint) = hint {
        ui.painter().galley(
            pos2(
                bx - 6.0 - hint.size().x,
                band.center().y - hint.size().y / 2.0,
            ),
            hint,
            p.text_faint,
        );
    }
}

/// The diff options popover's open state, and the popup id `--overlay diff-options` opens.
fn options_open_id() -> egui::Id {
    egui::Id::new(OPTIONS_POPUP)
}

/// Context lines and whitespace handling for this repository's diffs.
fn diff_options(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let prefs = &mut tab.diff_prefs;
    let mut changed = false;
    let lines = if prefs.context == 1 { "line" } else { "lines" };
    let anchor = Button::new(format!("{} {lines}", prefs.context))
        .ghost()
        .size(Size::Sm)
        .icon(icon::SLIDERS_HORIZONTAL)
        .tooltip("Diff options: context lines and whitespace (this repository)")
        .id_salt("diff-options-button")
        .show(ui);
    let state = options_open_id().with("open");
    let mut open = ui.data(|d| d.get_temp::<bool>(state)).unwrap_or(false);
    if anchor.clicked() {
        open = !open;
    }
    // Screenshots open it by the popup id (`--overlay diff-options`).
    if egui::Popup::is_id_open(ui.ctx(), options_open_id()) {
        open = true;
    }
    Popover::new(OPTIONS_POPUP, &anchor)
        .width(300.0)
        .open(&mut open)
        .show(|ui| {
            ui.label(
                RichText::new("Diff options")
                    .font(FontId::new(m.body_size, theme::semibold()))
                    .color(p.text),
            );
            ui.label(
                RichText::new("For this repository")
                    .size(m.meta_size)
                    .color(p.text_faint),
            );
            ui.add_space(theme::MD);
            changed |= Slider::new(&mut prefs.context, 0..=25)
                .step(1.0)
                .label("Context lines")
                .header(true)
                .width(ui.available_width())
                .id_salt("diff-context-lines")
                .show(ui)
                .changed();
            ui.add_space(theme::MD);
            ui.label(RichText::new("Whitespace").color(p.text_dim));
            ui.add_space(2.0);
            changed |= Segmented::new(
                &mut prefs.whitespace,
                &[
                    (WhitespaceMode::Show, "Show"),
                    (WhitespaceMode::IgnoreEol, "Ignore trailing"),
                    (WhitespaceMode::IgnoreAll, "Ignore all"),
                ],
            )
            .label("Whitespace")
            .id_salt("diff-whitespace")
            .show(ui)
            .changed();
            if prefs.whitespace != WhitespaceMode::Show {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Hunk staging is off while whitespace is ignored.")
                        .size(m.meta_size)
                        .color(p.text_faint),
                );
            }
        });
    ui.data_mut(|d| d.insert_temp(state, open));
    if changed {
        actions.push(Action::DiffPrefsChanged);
    }
}

fn empty(ui: &mut Ui, glyph: &str, title: &str, body: &str) {
    EmptyState::new(title)
        .icon(glyph)
        .body(body)
        .compact(true)
        .show(ui);
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

    /// Labels and bounds of the AccessKit nodes one frame of a working-copy hunk header made,
    /// `width` wide, and the actions it pushed.
    fn hunk_frame(
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> (Vec<(String, Rect)>, Vec<Action>) {
        let d = FileDiff {
            hunks: vec![hunk(" +")],
            ..Default::default()
        };
        let mut pushed = Vec::new();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                let cx = Ctx {
                    p: palette(ui.ctx()),
                    source: &DiffSource::Unstaged,
                    path: "file.rs",
                    conflicted: false,
                    one_sided: false,
                    editable: true,
                    context: 3,
                    highlights: None,
                    mono: FontId::monospace(theme::metrics().mono_size),
                };
                let rect = Rect::from_min_size(pos2(0.0, 40.0), vec2(width, row_heights().1));
                hunk_header(
                    ui,
                    &cx,
                    &d,
                    0,
                    rect,
                    egui::Pos2::ZERO,
                    rect,
                    &mut None,
                    &mut pushed,
                );
            },
        );
        output.textures_delta.clear();
        let nodes = output
            .platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| {
                        let b = n.bounds()?;
                        let r = Rect::from_min_max(
                            pos2(crate::ui::to_f32(b.x0), crate::ui::to_f32(b.y0)),
                            pos2(crate::ui::to_f32(b.x1), crate::ui::to_f32(b.y1)),
                        );
                        Some((n.label()?.to_owned(), r))
                    })
                    .collect()
            })
            .unwrap_or_default();
        (nodes, pushed)
    }

    fn themed() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    /// A11Y-1: the diff's text reaches assistive technology. Each visible line is a node that
    /// says how it changed and what it says; no line a user can pick is unnamed.
    #[test]
    fn diff_lines_are_spoken() {
        use egui::accesskit::{Action as AkAction, Role};
        let ctx = themed();
        ctx.enable_accesskit();
        let line = |kind, old_no, new_no, text: &str| DiffLine {
            kind,
            old_no,
            new_no,
            text: text.into(),
        };
        let d = Arc::new(FileDiff {
            path: "src/main.rs".into(),
            kind: Some(ChangeKind::Modified),
            hunks: vec![Hunk {
                header: "@@ -1,2 +1,2 @@".into(),
                old_start: 1,
                old_lines: 2,
                new_start: 1,
                new_lines: 2,
                lines: vec![
                    line(LineKind::Context, Some(1), Some(1), "fn main() {"),
                    line(LineKind::Removed, Some(2), None, "    old_call();"),
                    line(LineKind::Added, None, Some(2), "    new_call();"),
                ],
            }],
            additions: 1,
            deletions: 1,
            ..FileDiff::default()
        });
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.file = Some(crate::tab::FileSelection {
            source: DiffSource::Unstaged,
            entry: canopy_core::FileEntry {
                path: "src/main.rs".into(),
                old_path: None,
                kind: ChangeKind::Modified,
                additions: 1,
                deletions: 1,
                lfs: false,
                old_mode: None,
                new_mode: None,
            },
            conflicted: false,
        });
        tab.diff = Some(d);
        let mut nodes = Vec::new();
        for _ in 0..3 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default()
                        .show(ui, |ui| show(ui, &mut tab, &mut Vec::new()));
                },
            );
            out.textures_delta.clear();
            nodes = out
                .platform_output
                .accesskit_update
                .map(|u| u.nodes.into_iter().map(|(_, n)| n).collect())
                .unwrap_or_default();
        }
        let spoken: Vec<String> = nodes
            .iter()
            .filter_map(|n| n.label().or_else(|| n.value()).map(str::to_owned))
            .collect();
        for wanted in [
            "Line 1: fn main() {",
            "Removed line 2:     old_call();",
            "Added line 2:     new_call();",
            "Hunk 1: @@ -1,2 +1,2 @@",
        ] {
            assert!(
                spoken.iter().any(|s| s == wanted),
                "{wanted:?} not in {spoken:#?}"
            );
        }
        let unnamed: Vec<_> = nodes
            .iter()
            .filter(|n| n.supports_action(AkAction::Click) || n.supports_action(AkAction::Focus))
            .filter(|n| !matches!(n.role(), Role::Window | Role::GenericContainer))
            .filter(|n| n.label().is_none_or(str::is_empty) && n.value().is_none())
            .map(|n| format!("{:?}", n.role()))
            .collect();
        assert!(unnamed.is_empty(), "unnamed operable nodes: {unnamed:?}");
    }

    /// QVIS-1: the "Click lines" hint never paints over a long hunk title; it shows only where
    /// the whole title leaves room for it.
    #[test]
    fn the_pick_hint_never_overlaps_the_hunk_title() {
        let ctx = themed();
        for (header, hint_expected) in [
            ("@@ -1,2 +1,2 @@", true),
            (
                "@@ -15,3 +15,4 @@ Set TIDEPOOL_DATA to choose where pastes are stored (default: ./data).",
                false,
            ),
        ] {
            let d = FileDiff {
                hunks: vec![Hunk {
                    header: header.into(),
                    ..hunk(" +")
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let cx = Ctx {
                    p: palette(ui.ctx()),
                    source: &DiffSource::Unstaged,
                    path: "file.rs",
                    conflicted: false,
                    one_sided: false,
                    editable: true,
                    context: 3,
                    highlights: None,
                    mono: FontId::monospace(theme::metrics().mono_size),
                };
                let rect = Rect::from_min_size(pos2(0.0, 40.0), vec2(900.0, row_heights().1));
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
            let texts: Vec<(String, Rect)> = output
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.visual_bounding_rect()))
                    }
                    _ => None,
                })
                .collect();
            let title = texts
                .iter()
                .find(|(t, _)| t.starts_with("@@"))
                .map(|(_, r)| *r)
                .expect("the hunk title");
            let hint = texts
                .iter()
                .find(|(t, _)| t.starts_with("Click lines"))
                .map(|(_, r)| *r);
            assert_eq!(hint.is_some(), hint_expected, "{header}: {texts:?}");
            if let Some(hint) = hint {
                assert!(
                    title.right() < hint.left(),
                    "title {title:?} overlaps hint {hint:?}"
                );
            }
        }
    }

    #[test]
    fn hunk_actions_keep_their_names_when_icon_only() {
        for width in [800.0, 400.0] {
            let ctx = themed();
            let (nodes, _) = hunk_frame(&ctx, width, Vec::new());
            for name in ["Stage hunk", "Discard hunk…"] {
                assert!(
                    nodes.iter().any(|(l, _)| l == name),
                    "{name} at {width}: {nodes:?}"
                );
            }
            let stage = nodes.iter().find(|(l, _)| l == "Stage hunk").map(|n| n.1);
            let discard = nodes
                .iter()
                .find(|(l, _)| l == "Discard hunk…")
                .map(|n| n.1);
            let (Some(stage), Some(discard)) = (stage, discard) else {
                unreachable!("checked above");
            };
            assert!(
                discard.left() - stage.right() >= theme::LG - 0.5,
                "Discard keeps its distance from Stage at {width}"
            );
            if width < HUNK_ACTIONS_ICON_ONLY {
                // Icon buttons are square.
                assert!(
                    (discard.width() - discard.height()).abs() < 0.5,
                    "icon only: {discard:?}"
                );
            }
        }
    }

    #[test]
    fn discarding_a_hunk_asks_first() {
        let ctx = themed();
        let (nodes, _) = hunk_frame(&ctx, 800.0, Vec::new());
        let (_, at) = nodes
            .iter()
            .find(|(l, _)| l == "Discard hunk…")
            .expect("Discard is offered");
        let pos = at.center();
        hunk_frame(
            &ctx,
            800.0,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (_, pushed) = hunk_frame(
            &ctx,
            800.0,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            matches!(
                pushed.as_slice(),
                [Action::Confirm {
                    danger: true,
                    op: Op::Hunk {
                        action: HunkAction::Discard,
                        ..
                    },
                    ..
                }]
            ),
            "{pushed:?}"
        );
        // Mop-up D24: the title asks a question naming what goes, the answer is verb + object.
        let [Action::Confirm { title, confirm, .. }] = pushed.as_slice() else {
            unreachable!()
        };
        assert!(
            title.starts_with("Discard this hunk of ") && title.ends_with('?'),
            "{title}"
        );
        assert_eq!(confirm, "Discard hunk");
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
                mono: FontId::monospace(theme::metrics().mono_size),
            };
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, row_heights().1));
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
        let (line_h, hunk_h) = (l.line_h, l.hunk_h);
        assert_eq!((line_h, hunk_h), row_heights());
        assert_eq!(l.tops[1], hunk_h);
        assert_eq!(l.tops[8], hunk_h + 7.0 * line_h);
        assert_eq!(l.total_height(), 2.0 * hunk_h + 8.0 * line_h);
        assert_eq!(l.columns, 5, "a tab counts as four columns");
        assert_eq!(l.visible(0.0, 1.0), 0..1);
        // The bottom edge of a row is not inside it.
        assert_eq!(l.visible(hunk_h, hunk_h + line_h), 1..2);
        assert_eq!(
            l.visible(hunk_h + 0.5 * line_h, hunk_h + 2.5 * line_h),
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
        let syntax = theme::MIDNIGHT.syntax;
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

    /// H3 (kit-a F1): a new or deleted file keeps Unified chosen and live; only Side by side is
    /// off, and a click on it changes nothing.
    #[test]
    fn a_one_column_file_turns_off_only_side_by_side() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.enable_accesskit();
        let run = |events: Vec<egui::Event>, actions: &mut Vec<Action>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| layout_toggle(ui, true, true, true, actions),
            );
            out.textures_delta.clear();
            out.platform_output
                .accesskit_update
                .map(|u| u.nodes.into_iter().map(|(_, n)| n).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let mut actions = Vec::new();
        let _ = run(Vec::new(), &mut actions);
        let nodes = run(Vec::new(), &mut actions);
        let option = |label: &str| {
            nodes
                .iter()
                .find(|n| {
                    n.role() == egui::accesskit::Role::RadioButton && n.label() == Some(label)
                })
                .unwrap_or_else(|| panic!("{label}"))
                .clone()
        };
        let unified = option("Unified");
        let side = option("Side by side");
        assert!(!unified.is_disabled(), "Unified stays live");
        assert_eq!(unified.toggled(), Some(egui::accesskit::Toggled::True));
        assert!(side.is_disabled(), "Side by side is off");
        let b = side.bounds().expect("bounds");
        let at = pos2(
            crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
            crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
        );
        let mut events = vec![egui::Event::PointerMoved(at)];
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        let _ = run(events, &mut actions);
        assert!(actions.is_empty(), "{actions:?}");
    }
}
