//! The conflict resolver, shown in Changes for a conflicted file: every conflict as a card with
//! our and their side next to each other (and the common ancestor for diff3 files), a choice per
//! conflict (ours, theirs, both in either order, the ancestor, or a hand edit), the resulting text,
//! and Save progress / Save resolved file. Files Canopy cannot show line by line (binary, deleted on
//! one side) get a whole-file choice instead.

use std::collections::HashMap;

use egui::{FontId, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::{Choice, ConflictKind, ConflictRegion, Segment, Side};

use crate::action::Action;
use crate::tab::{RepoTab, Resolver};
use crate::ui::kit::chip::Badge;
use crate::ui::kit::field::CodeArea;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::surface::{Card, InsetBox};
use crate::ui::kit::toggle::Switch;
use crate::ui::kit::{Button, Size, Tone, menu, paint};
use crate::ui::theme::{self, palette};
use crate::ui::widgets;
use crate::views::commit_detail;
use crate::worker::Op;

/// Unchanged stretches longer than this are folded.
const FOLD: usize = 8;
/// Lines kept around a fold.
const KEEP: usize = 3;

fn choice_label(c: &Choice) -> &'static str {
    match c {
        Choice::Ours => "Current version",
        Choice::Theirs => "Incoming version",
        Choice::OursThenTheirs => "Current, then incoming",
        Choice::TheirsThenOurs => "Incoming, then current",
        Choice::Base => "Ancestor",
        Choice::Custom(_) => "Edited",
    }
}

/// Draws the resolver for the selected conflicted file.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let rebasing = tab
        .snap
        .as_ref()
        .is_some_and(|snap| snap.state == canopy_core::RepoState::Rebase);
    let Some(r) = tab.resolver.as_mut() else {
        return;
    };
    let abs = tab.path.join(&r.path);
    header(ui, r, &abs, actions);
    if rebasing {
        ui.add(egui::Label::new(RichText::new("During a rebase, Current is the version being rebased onto; Incoming is the commit being replayed.").size(m.small_size).color(p.text_dim)).wrap());
    }
    match &r.file {
        None => {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| Spinner::new().label("the conflicted file").show(ui));
        }
        Some(Err(e)) => {
            let e = e.clone();
            whole_file(ui, r, Some(&e), actions);
        }
        Some(Ok(_))
            if !matches!(
                r.kind,
                None | Some(ConflictKind::BothModified | ConflictKind::BothAdded)
            ) =>
        {
            whole_file(ui, r, None, actions);
        }
        Some(Ok(f)) if f.conflicts() == 0 => {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(icon::CHECK_CIRCLE)
                        .size(m.icon_hero)
                        .color(p.success),
                );
                ui.label(
                    RichText::new("No conflict markers left").font(theme::heading(m.subtitle_size)),
                );
                ui.label(
                    RichText::new("The file looks resolved. Mark it resolved to stage it.")
                        .color(p.text_dim),
                );
                ui.add_space(theme::SM);
                if Button::new("Mark resolved").primary().show(ui).clicked() {
                    actions.push(Action::Op(Op::Stage(vec![r.path.clone()])));
                }
            });
        }
        Some(Ok(_)) => {
            footer(ui, r, actions);
            body(ui, r);
        }
    }
}

fn header(ui: &mut Ui, r: &mut Resolver, abs: &std::path::Path, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    let width = ui.available_width();
    egui::Frame::new()
        .fill(s.pane_header)
        .inner_margin(egui::Margin::symmetric(
            crate::ui::points_i8(theme::LG),
            crate::ui::points_i8(theme::SM),
        ))
        .show(ui, |ui| {
            ui.set_min_width((width - theme::LG * 2.0).max(0.0));
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(icon::GIT_DIFF)
                        .size(m.icon_md)
                        .color(p.warning),
                );
                let (name, dir) = widgets::split_path(&r.path);
                ui.label(
                    RichText::new(name)
                        .font(FontId::new(m.body_size, theme::semibold()))
                        .color(p.text),
                );
                if !dir.is_empty() {
                    ui.label(RichText::new(dir).size(m.meta_size).color(p.text_faint));
                }
                if let Some(kind) = r.kind {
                    Badge::new(kind.label()).tone(Tone::Warning).show(ui);
                }
                ui.horizontal_wrapped(|ui| {
                    if Button::new("Open in editor")
                        .icon(icon::ARROW_SQUARE_OUT)
                        .size(Size::Sm)
                        .tooltip("Open the file in your editor (Settings → Integrations)")
                        .show(ui)
                        .clicked()
                    {
                        actions.push(Action::OpenInEditor(abs.to_path_buf()));
                    }
                    let n = r.conflicts();
                    if n > 0 {
                        if r.file.as_ref().is_some_and(|f| {
                            f.as_ref()
                                .is_ok_and(|f| f.regions().any(|c| c.base.is_some()))
                        }) {
                            Switch::new(&mut r.show_base, "Show ancestor")
                                .tooltip("Show the common ancestor's version of each conflict")
                                .show(ui);
                        }
                        let done = r.resolved();
                        let color = if done == n { p.success } else { p.text_dim };
                        ui.label(
                            RichText::new(format!("{done} of {n} resolved"))
                                .size(m.meta_size)
                                .color(color),
                        );
                    }
                });
            });
        });
    paint::seam_h(
        ui.painter(),
        ui.min_rect().x_range(),
        ui.min_rect().max.y - 0.5,
        s.pane_header_seam,
    );
}

/// What a whole-file choice says for a conflict of `kind` (`error`: the file cannot be parsed):
/// its title and explanation.
fn whole_file_text(
    kind: Option<ConflictKind>,
    error: Option<&str>,
) -> (&'static str, &'static str) {
    match (kind, error) {
        (Some(ConflictKind::DeletedByThem), _) => (
            "The incoming side deleted this file; the current side changed it",
            "Keep the current version, or accept the incoming deletion.",
        ),
        (Some(ConflictKind::DeletedByUs), _) => (
            "The current side deleted this file; the incoming side changed it",
            "Keep it deleted, or restore the incoming version.",
        ),
        (Some(ConflictKind::AddedByUs), _) => (
            "Only the current side added this file",
            "Keep the current file, or accept its absence on the incoming side.",
        ),
        (Some(ConflictKind::AddedByThem), _) => (
            "Only the incoming side added this file",
            "Leave it out, or take the incoming file.",
        ),
        (Some(ConflictKind::BothDeleted), _) => (
            "Both sides deleted this file",
            "It was renamed differently on each side. Delete it here.",
        ),
        (_, Some(_)) => (
            "Canopy cannot show this file line by line",
            "It is binary, a link, or not UTF-8 text. Choose a whole side, or open it in an editor.",
        ),
        _ => ("Choose a side", ""),
    }
}

/// The sides a whole-file choice offers for a conflict of `kind`, each with its button label. A
/// file both sides deleted has one answer: either side removes it.
fn whole_file_sides(kind: Option<ConflictKind>) -> Vec<(Side, &'static str)> {
    match kind {
        Some(ConflictKind::DeletedByThem) => vec![
            (Side::Ours, "Keep current version"),
            (Side::Theirs, "Delete the file"),
        ],
        Some(ConflictKind::DeletedByUs) => vec![
            (Side::Ours, "Keep it deleted"),
            (Side::Theirs, "Restore incoming version"),
        ],
        Some(ConflictKind::AddedByUs) => {
            vec![(Side::Ours, "Keep it"), (Side::Theirs, "Leave it out")]
        }
        Some(ConflictKind::AddedByThem) => vec![
            (Side::Ours, "Leave it out"),
            (Side::Theirs, "Take incoming file"),
        ],
        Some(ConflictKind::BothDeleted) => vec![(Side::Ours, "Delete it")],
        _ => vec![
            (Side::Ours, "Keep current version"),
            (Side::Theirs, "Keep incoming version"),
        ],
    }
}

/// Whole-file choice: binary or non-UTF-8 files, and deleted-on-one-side conflicts.
fn whole_file(ui: &mut Ui, r: &Resolver, error: Option<&str>, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let path = r.path.clone();
    let (title, body) = whole_file_text(r.kind, error);
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(RichText::new(icon::GIT_DIFF).size(m.icon_hero).color(p.warning));
        ui.label(RichText::new(title).font(theme::heading(m.subtitle_size)));
        if !body.is_empty() {
            ui.label(RichText::new(body).color(p.text_dim));
        }
        if let Some(e) = error {
            ui.label(RichText::new(e).size(m.small_size).color(p.text_faint));
        }
        ui.add_space(theme::MD);
        // The two choices side by side, centred by the width they took last frame.
        let width_id = ui.id().with("whole-file-choices");
        let last: f32 = ui.ctx().data(|d| d.get_temp(width_id)).unwrap_or(0.0);
        let row = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::SM;
            ui.add_space(((ui.available_width() - last) / 2.0).max(0.0));
            let start = ui.cursor().left();
            for (side, label) in whole_file_sides(r.kind) {
                if Button::new(label)
                    .size(Size::Lg)
                    .id_salt(("whole-file", side_name(side)))
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::Confirm {
                        title: format!("{label}?"),
                        body: format!("This replaces {path} in the working tree with that version and marks it resolved. Edits you made to the file are lost."),
                        confirm: label.to_owned(),
                        danger: true,
                        op: Op::ResolveSide { path: path.clone(), side },
                    });
                }
            }
            ui.min_rect().right() - start
        });
        let used = row.inner;
        if (used - last).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(width_id, used));
            ui.ctx().request_repaint();
        }
    });
}

/// A side's name in widget ids.
fn side_name(side: Side) -> &'static str {
    match side {
        Side::Ours => "ours",
        Side::Theirs => "theirs",
    }
}

fn footer(ui: &mut Ui, r: &mut Resolver, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    let Some(Ok(file)) = &r.file else { return };
    let n = file.conflicts();
    let done = r.resolved();
    egui::Panel::bottom(egui::Id::new(("resolve-footer", &r.path)))
        .frame(egui::Frame::new().fill(s.panel).inner_margin(egui::Margin::symmetric(14, 10)))
        .show(ui, |ui| {
            let top = ui.max_rect().top() - 10.0;
            paint::seam_h(ui.painter(), ui.max_rect().expand2(vec2(14.0, 0.0)).x_range(), top + 0.5, s.border);
            ui.vertical(|ui| {
                use crate::keymap::{Command, hint};
                let keys = format!(
                    "{}  {} {} previous/next · {} current · {} incoming · {} both · {} undo",
                    icon::KEYBOARD,
                    hint(Command::PrevConflict),
                    hint(Command::NextConflict),
                    hint(Command::TakeOurs),
                    hint(Command::TakeTheirs),
                    hint(Command::TakeBoth),
                    hint(Command::ClearChoice),
                );
                commit_detail::without_icons(ui.add(egui::Label::new(RichText::new(keys).size(m.small_size).color(p.text_faint)).truncate()))
                    .kit_tooltip("Press ? for every shortcut").show();
                ui.label(RichText::new(if done == n {
                    "All conflicts have a choice. Save the resolved file to stage it."
                } else {
                    "Choose a version for each conflict. Save progress keeps markers for unresolved parts."
                }).size(m.small_size).color(p.text_dim));
                ui.add_space(theme::XS);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SM;
                    let all = done == n;
                    let why = "Choose a resolution for every conflict first";
                    let save = Button::new("Save resolved file")
                        .primary()
                        .icon(icon::CHECK)
                        .enabled(all)
                        .tooltip("Write this file and stage it as resolved")
                        .disabled_reason(why)
                        .show(ui);
                    if commit_detail::say_unavailable(save, why).clicked() {
                        actions.push(Action::Op(Op::Resolve { path: r.path.clone(), content: file.render(&r.choices), mark: true, expected: file.fingerprint() }));
                    }
                    let why = "Choose a resolution for a conflict first";
                    let progress = Button::new("Save progress")
                        .icon(icon::FLOPPY_DISK)
                        .enabled(done > 0)
                        .tooltip("Write the chosen resolutions to the file; unresolved conflicts keep their markers")
                        .disabled_reason(why)
                        .show(ui);
                    if commit_detail::say_unavailable(progress, why).clicked() {
                        actions.push(Action::Op(Op::Resolve { path: r.path.clone(), content: file.render(&r.choices), mark: false, expected: file.fingerprint() }));
                    }
                    if done > 0
                        && Button::new("Undo all choices")
                            .ghost()
                            .tooltip("Clear every choice before saving")
                            .show(ui)
                            .clicked()
                    {
                        r.choices.iter_mut().for_each(|c| *c = None);
                        r.editing = None;
                    }
                });
            });
        });
}

fn code_font() -> FontId {
    FontId::monospace(theme::metrics().mono_size)
}

/// The line numbers beside code lines.
fn number_font() -> FontId {
    FontId::monospace(theme::metrics().small_size)
}

/// Height of one laid-out code line: what a label of it takes (the galley's height, rounded the
/// way egui rounds rows), so the lines sit exactly where one label per line would put them.
fn code_row_height(ui: &Ui) -> f32 {
    ui.painter()
        .layout_no_wrap("M".to_owned(), code_font(), egui::Color32::PLACEHOLDER)
        .size()
        .y
}

/// Height of `n` code lines stacked `spacing` apart (`row_h` each), as [`code_lines`] lays them
/// out; an empty list shows one "(nothing)" line.
fn code_height(n: usize, row_h: f32, spacing: f32) -> f32 {
    let n = crate::ui::count_f32(n.max(1));
    n * row_h + (n - 1.0) * spacing
}

/// Code lines, one per row, truncated to the width. The block reserves its full height but only
/// the lines inside the visible area are laid out and painted, so a side with thousands of lines
/// costs no more than a short one. A side's preview drops the indentation all its lines share, so
/// a nested snippet uses the box's width for its code; `name` is what a screen reader calls it.
fn code_lines(ui: &mut Ui, lines: &[String], color: egui::Color32, name: &str) {
    code_lines_at(ui, lines, color, None, name);
}

/// Columns of indentation every non-blank line of `lines` shares, tabs counted as four.
fn common_indent(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum::<usize>()
        })
        .min()
        .unwrap_or(0)
}

/// `line` as painted: tabs as four spaces, its first `indent` columns dropped.
fn shown_line(line: &str, indent: usize) -> String {
    let expanded = line.replace('\t', "    ");
    let skip = expanded
        .char_indices()
        .take(indent)
        .take_while(|(_, c)| *c == ' ')
        .count();
    expanded.chars().skip(skip).collect()
}

/// Where a line too long for its box is cut: before the last whole word or number that fits,
/// so a cut never leaves part of a name or a number looking whole ("25…" of "250"). A word
/// longer than a quarter of what fits is cut where it must be.
fn cut_at_boundary(text: &str, fits: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    if fits >= chars.len() {
        return chars.len();
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = fits;
    while at > 0 && word(chars[at - 1]) && word(chars[at]) {
        at -= 1;
    }
    if at == 0 || fits - at > fits / 4 {
        fits
    } else {
        at
    }
}

/// What a screen reader reads for a block of code lines: its first lines, and how many more.
fn spoken_block(lines: &[String]) -> String {
    const SPOKEN: usize = 100;
    let mut text = lines
        .iter()
        .take(SPOKEN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > SPOKEN {
        use std::fmt::Write as _;
        let _ = write!(text, "\n… and {} more lines", lines.len() - SPOKEN);
    }
    text
}

fn code_lines_at(
    ui: &mut Ui,
    lines: &[String],
    color: egui::Color32,
    first_line: Option<usize>,
    name: &str,
) {
    if lines.is_empty() {
        ui.label(
            RichText::new("(nothing)")
                .size(theme::metrics().small_size)
                .italics()
                .color(palette(ui.ctx()).text_faint),
        );
        return;
    }
    let font = code_font();
    let row_h = code_row_height(ui);
    let pitch = row_h + ui.spacing().item_spacing.y;
    let width = ui.available_width();
    let gutter = first_line.map_or(0.0, |line| {
        ui.painter()
            .layout_no_wrap(
                (line + lines.len()).to_string(),
                number_font(),
                palette(ui.ctx()).text_faint,
            )
            .size()
            .x
            + theme::LG
    });
    let (rect, resp) = ui.allocate_exact_size(
        vec2(
            width,
            code_height(lines.len(), row_h, ui.spacing().item_spacing.y),
        ),
        Sense::click(),
    );
    // The lines are painted, not labels: a screen reader reads them from the block's node, and
    // copying goes through the context menu.
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, spoken_block(lines))
    });
    ui.ctx()
        .accesskit_node_builder(resp.id, |node| node.set_label(name));
    menu::context(&resp, |m| {
        if m.item(icon::COPY, "Copy lines").show(m.ui).clicked() {
            m.ui.ctx().copy_text(lines.join("\n"));
        }
    });
    // A side's preview (no line numbers) drops the indentation its lines share, measured once
    // per block rather than every frame.
    let indent = if first_line.is_none() {
        let key = resp
            .id
            .with(("common-indent", lines.as_ptr() as usize, lines.len()));
        ui.ctx()
            .data(|d| d.get_temp::<usize>(key))
            .unwrap_or_else(|| {
                let indent = common_indent(lines);
                ui.ctx().data_mut(|d| d.insert_temp(key, indent));
                indent
            })
    } else {
        0
    };
    let room = (width - gutter).max(0.0);
    let clip = ui.clip_rect();
    let hover = resp.hover_pos();
    for i in commit_detail::visible_rows(lines.len(), pitch, rect.min.y, clip.min.y, clip.max.y) {
        let text = shown_line(&lines[i], indent);
        let full = ui
            .painter()
            .layout_no_wrap(text.clone(), font.clone(), color);
        let elided = full.size().x > room;
        let galley = if elided {
            // The characters that fit beside an ellipsis, then back to a word boundary.
            let ellipsis = ui
                .painter()
                .layout_no_wrap("…".to_owned(), font.clone(), color)
                .size()
                .x;
            let fits = full.rows.first().map_or(0, |row| {
                row.glyphs
                    .iter()
                    .take_while(|g| g.pos.x + g.advance_width <= room - ellipsis)
                    .count()
            });
            let cut = cut_at_boundary(&text, fits);
            let shown: String = text.chars().take(cut).collect();
            ui.painter()
                .layout_no_wrap(format!("{}…", shown.trim_end()), font.clone(), color)
        } else {
            full
        };
        let top = rect.min.y + crate::ui::count_f32(i) * pitch;
        // Like a truncated label: the whole line on hover when it does not fit.
        if elided
            && hover.is_some_and(|h| (top..top + row_h).contains(&h.y))
            && let Some(line) = lines.get(i)
        {
            resp.clone().kit_tooltip(line.as_str()).show();
        }
        if let Some(line) = first_line {
            ui.painter().text(
                egui::pos2(rect.min.x + gutter - theme::SM, top + row_h / 2.0),
                egui::Align2::RIGHT_CENTER,
                (line + i).to_string(),
                number_font(),
                palette(ui.ctx()).text_faint,
            );
        }
        ui.painter()
            .galley(egui::pos2(rect.min.x + gutter, top), galley, color);
    }
}

/// Measured heights of the resolver's conflict cards, so cards out of view are skipped instead
/// of laid out every frame. Common text between cards is always drawn: it only paints its
/// visible lines.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    /// Width the heights were measured at.
    width: f32,
    /// Per segment index: the card's state signature and its measured height.
    heights: HashMap<usize, (u64, f32)>,
    /// Card height minus its code lines, learned per shape (has a result, shows the ancestor).
    overhead: HashMap<(bool, bool), f32>,
}

impl Layout {
    /// A layout with one measured card.
    #[cfg(test)]
    pub fn with_height(segment: usize, signature: u64, height: f32) -> Self {
        let mut l = Self::default();
        l.heights.insert(segment, (signature, height));
        l
    }

    /// Nothing measured.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty() && self.overhead.is_empty()
    }

    /// The height a card is known to have, if it was measured in the same state.
    fn known(&self, segment: usize, signature: u64) -> Option<f32> {
        self.heights
            .get(&segment)
            .filter(|(s, _)| *s == signature)
            .map(|(_, h)| *h)
    }
}

/// Space around each card in the body.
const CARD_GAP: f32 = 8.0;
/// Cards this close to the visible area are drawn anyway.
const OVERSCAN: f32 = 200.0;

/// Lines in the result of `choice`, without building it.
fn result_len(region: &ConflictRegion, choice: &Choice) -> usize {
    let base = region.base.as_ref().map_or(0, Vec::len);
    match choice {
        Choice::Ours => region.ours.len(),
        Choice::Theirs => region.theirs.len(),
        Choice::OursThenTheirs | Choice::TheirsThenOurs => region.ours.len() + region.theirs.len(),
        Choice::Base => base,
        Choice::Custom(t) => t.lines().count(),
    }
}

/// Everything about a card that changes its height.
fn card_signature(r: &Resolver, k: usize, region: &ConflictRegion) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (r.focus == k).hash(&mut h);
    r.choices
        .get(k)
        .and_then(Option::as_ref)
        .map(|c| (choice_label(c), result_len(region, c)))
        .hash(&mut h);
    (r.show_base && region.base.is_some()).hash(&mut h);
    r.editing
        .as_ref()
        .filter(|(i, _)| *i == k)
        .map(|(_, t)| t.lines().count())
        .hash(&mut h);
    h.finish()
}

/// The code lines of a card: (lines inside the card, card shape).
fn card_lines(r: &Resolver, k: usize, region: &ConflictRegion) -> (Vec<usize>, (bool, bool)) {
    let choice = r.choices.get(k).cloned().flatten();
    let base_shown = r.show_base && region.base.is_some();
    let mut blocks = vec![region.ours.len().max(region.theirs.len())];
    if base_shown {
        blocks.push(region.base.as_ref().map_or(0, Vec::len));
    }
    if let Some(c) = &choice {
        blocks.push(result_len(region, c));
    }
    (blocks, (choice.is_some(), base_shown))
}

/// A card's height from its code lines and the learned overhead of its shape.
fn estimate_card(layout: &Layout, blocks: &[usize], shape: (bool, bool), row_h: f32) -> f32 {
    let lines: f32 = blocks.iter().map(|n| code_height(*n, row_h, 4.0)).sum();
    let default = 110.0 + if shape.0 { 26.0 } else { 0.0 } + if shape.1 { 26.0 } else { 0.0 };
    lines + layout.overhead.get(&shape).copied().unwrap_or(default) + 2.0 * CARD_GAP
}

fn body(ui: &mut Ui, r: &mut Resolver) {
    let Some(Ok(file)) = r.file.clone() else {
        return;
    };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .id_salt(("resolve", &r.path))
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let width = ui.available_width();
                    #[expect(
                        clippy::float_cmp,
                        reason = "the cached heights belong to this exact width"
                    )]
                    let resized = r.layout.width != width;
                    if resized {
                        r.layout.width = width;
                        r.layout.heights.clear();
                    }
                    let row_h = code_row_height(ui);
                    let near = ui.clip_rect().expand2(vec2(0.0, OVERSCAN));
                    let mut k = 0;
                    let mut original_line = 1;
                    for (i, seg) in file.segments.iter().enumerate() {
                        match seg {
                            Segment::Common(lines) => {
                                // Its own id scope: ids inside must not depend on which cards
                                // above were drawn this frame.
                                ui.push_id(("common", i), |ui| {
                                    common(
                                        ui,
                                        r,
                                        i,
                                        lines,
                                        i + 1 == file.segments.len(),
                                        original_line,
                                    );
                                });
                                original_line += lines.len();
                            }
                            Segment::Conflict(region) => {
                                original_line += region.ours.len()
                                    + region.theirs.len()
                                    + 3
                                    + region.base.as_ref().map_or(0, |base| base.len() + 1);
                                let signature = card_signature(r, k, region);
                                let (blocks, shape) = card_lines(r, k, region);
                                let known = r.layout.known(i, signature);
                                let height = known.unwrap_or_else(|| {
                                    estimate_card(&r.layout, &blocks, shape, row_h)
                                });
                                let top = ui.cursor().min.y;
                                let reveal = r.reveal && r.focus == k;
                                // A card being edited by hand is always drawn: egui drops the
                                // focus of a text field that is not drawn in a frame.
                                let editing = r.editing.as_ref().is_some_and(|(e, _)| *e == k);
                                if !reveal
                                    && !editing
                                    && (top + height < near.min.y || top > near.max.y)
                                {
                                    ui.allocate_space(vec2(width, height));
                                } else {
                                    ui.push_id(("conflict", i), |ui| {
                                        ui.add_space(CARD_GAP);
                                        conflict_card(ui, r, k, region);
                                        ui.add_space(CARD_GAP);
                                    });
                                    let measured = ui.cursor().min.y - top;
                                    // A click inside the card can change its state while it is
                                    // drawn; the height then belongs to neither state, so it is
                                    // measured again next frame.
                                    if card_signature(r, k, region) != signature {
                                        k += 1;
                                        continue;
                                    }
                                    r.layout.heights.insert(i, (signature, measured));
                                    let editing = r.editing.as_ref().is_some_and(|(e, _)| *e == k);
                                    if !editing {
                                        let lines: f32 = blocks
                                            .iter()
                                            .map(|n| code_height(*n, row_h, 4.0))
                                            .sum();
                                        r.layout
                                            .overhead
                                            .insert(shape, measured - lines - 2.0 * CARD_GAP);
                                    }
                                }
                                k += 1;
                            }
                        }
                    }
                });
        });
}

/// The name of a block of unchanged lines: which lines of the file it holds.
fn unchanged_name(first: usize, count: usize) -> String {
    if count <= 1 {
        format!("Unchanged line {first}")
    } else {
        format!("Unchanged lines {first} to {}", first + count - 1)
    }
}

/// Unchanged lines between conflicts, long stretches folded.
fn common(
    ui: &mut Ui,
    r: &mut Resolver,
    i: usize,
    lines: &[String],
    last_segment: bool,
    first_line: usize,
) {
    let p = palette(ui.ctx());
    if lines.len() > FOLD && !r.expanded.contains(&i) {
        let first = if i == 0 { &[][..] } else { &lines[..KEEP] };
        let last = if last_segment {
            &[][..]
        } else {
            &lines[lines.len() - KEEP..]
        };
        if !first.is_empty() {
            code_lines_at(
                ui,
                first,
                p.text,
                Some(first_line),
                &unchanged_name(first_line, first.len()),
            );
        }
        let hidden = lines.len() - first.len() - last.len();
        ui.add_space(2.0);
        if Button::new(format!("{hidden} unchanged lines"))
            .ghost()
            .size(Size::Sm)
            .icon(icon::DOTS_THREE)
            .tooltip("Show the unchanged lines")
            .show(ui)
            .clicked()
        {
            r.expanded.insert(i);
        }
        ui.add_space(2.0);
        if !last.is_empty() {
            let from = first_line + lines.len() - last.len();
            code_lines_at(
                ui,
                last,
                p.text,
                Some(from),
                &unchanged_name(from, last.len()),
            );
        }
    } else if !lines.is_empty() {
        code_lines_at(
            ui,
            lines,
            p.text,
            Some(first_line),
            &unchanged_name(first_line, lines.len()),
        );
    }
}

/// The button that picks (or, when it is already picked, clears) one choice: a check in front of
/// the chosen one. The two sides are the main choices (Secondary); the others are `quiet`
/// (Ghost) until chosen, so a card reads as "keep one side" first.
fn choice_button(
    label: &str,
    on: bool,
    quiet: bool,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> Button {
    let b = Button::new(label).size(Size::Sm).id_salt(salt);
    if on {
        b.icon(icon::CHECK)
    } else if quiet {
        b.ghost()
    } else {
        b
    }
}

fn conflict_card(ui: &mut Ui, r: &mut Resolver, k: usize, region: &ConflictRegion) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let focused = r.focus == k;
    let choice = r.choices.get(k).cloned().flatten();
    let tone = if choice.is_some() {
        Tone::Success
    } else {
        Tone::Warning
    };
    let small =
        |text: String, color: egui::Color32| RichText::new(text).size(m.small_size).color(color);
    let resp = Card::new()
        .ring(tone)
        .selected(focused)
        .id_salt(("conflict-card", k))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = theme::XS;
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Conflict {}", k + 1))
                        .font(FontId::new(m.body_size, theme::semibold())),
                );
                commit_detail::without_icons(match &choice {
                    Some(c) => ui.label(small(
                        format!("{}  {}", icon::CHECK, choice_label(c)),
                        p.success,
                    )),
                    None => ui.label(small(format!("{}  Unresolved", icon::WARNING), p.warning)),
                });
            });
            ui.add_space(theme::XS);
            let mut picked: Option<Option<Choice>> = None;
            // Both sides next to each other.
            let w = (ui.available_width() - theme::MD) / 2.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = theme::MD;
                for (title, label, lines, color, side, command) in [
                    (
                        "Current version",
                        &region.ours_label,
                        &region.ours,
                        p.accent,
                        Choice::Ours,
                        crate::keymap::Command::TakeOurs,
                    ),
                    (
                        "Incoming version",
                        &region.theirs_label,
                        &region.theirs,
                        p.accent2,
                        Choice::Theirs,
                        crate::keymap::Command::TakeTheirs,
                    ),
                ] {
                    ui.vertical(|ui| {
                        ui.set_width(w);
                        ui.label(small(
                            if label.is_empty() || label == "HEAD" {
                                title.to_owned()
                            } else {
                                format!("Version from {label}")
                            },
                            color,
                        ));
                        ui.label(small(
                            format!(
                                "Git: {} · {label}",
                                if side == Choice::Ours {
                                    "ours"
                                } else {
                                    "theirs"
                                }
                            ),
                            p.text_faint,
                        ));
                        InsetBox::new().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            code_lines(ui, lines, p.text, title);
                        });
                        let on = choice.as_ref() == Some(&side);
                        if choice_button("Keep this version", on, false, ("keep", title))
                            .tooltip("Keep this version")
                            .shortcut(command)
                            .show_shortcut(true)
                            .show(ui)
                            .clicked()
                        {
                            picked = Some(if on { None } else { Some(side) });
                        }
                    });
                }
            });
            if r.show_base
                && let Some(base) = &region.base
            {
                ui.label(small(
                    format!("Ancestor · {}", region.base_label),
                    p.text_dim,
                ));
                InsetBox::new().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    code_lines(ui, base, p.text_dim, "Ancestor");
                });
            }
            ui.add_space(theme::XS);
            // Combined and custom choices.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SM;
                let mut options = vec![
                    (Choice::OursThenTheirs, "Both: current first"),
                    (Choice::TheirsThenOurs, "Both: incoming first"),
                ];
                if region.base.is_some() {
                    options.push((Choice::Base, "Ancestor"));
                }
                for (c, label) in options {
                    let on = choice.as_ref() == Some(&c);
                    if choice_button(label, on, true, ("combine", label))
                        .show(ui)
                        .clicked()
                    {
                        picked = Some(if on { None } else { Some(c) });
                    }
                }
                let editing = r.editing.as_ref().is_some_and(|(i, _)| *i == k);
                let custom = matches!(choice, Some(Choice::Custom(_)));
                let edit = Button::new("Edit").size(Size::Sm);
                let edit = if custom { edit } else { edit.ghost() };
                let edit = edit
                    .icon(if custom {
                        icon::CHECK
                    } else {
                        icon::PENCIL_SIMPLE
                    })
                    .tooltip("Write the result by hand")
                    .id_salt("edit")
                    .enabled(!editing)
                    .disabled_reason("Editing below");
                if edit.show(ui).clicked() && !editing {
                    let start = match &choice {
                        Some(c) => region.resolve(c),
                        None => region.resolve(&Choice::OursThenTheirs),
                    };
                    r.editing = Some((k, start.join("\n")));
                }
            });
            if let Some(c) = picked {
                if let Some(slot) = r.choices.get_mut(k) {
                    *slot = c;
                }
                r.focus = k;
                r.editing = None;
            }
            // Hand edit.
            let mut apply = None;
            let mut cancel = false;
            if let Some((i, text)) = r.editing.as_mut()
                && *i == k
            {
                ui.add_space(theme::XS);
                let submitted = CodeArea::new(text)
                    .rows(3..=16)
                    .name("Resolved text")
                    .id_salt(("hand-edit", k))
                    .show(ui)
                    .value;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SM;
                    if Button::new("Use this text")
                        .primary()
                        .size(Size::Sm)
                        .show(ui)
                        .clicked()
                        || submitted
                    {
                        apply = Some(text.clone());
                    }
                    if Button::new("Cancel")
                        .ghost()
                        .size(Size::Sm)
                        .show(ui)
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            }
            if let Some(text) = apply {
                if let Some(slot) = r.choices.get_mut(k) {
                    *slot = Some(Choice::Custom(text));
                }
                r.editing = None;
            } else if cancel {
                r.editing = None;
            }
            // Result.
            if let Some(c) = r.choices.get(k).cloned().flatten()
                && r.editing.as_ref().is_none_or(|(i, _)| *i != k)
            {
                ui.add_space(theme::XS);
                ui.label(small("Result".to_owned(), p.success));
                InsetBox::new().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    code_lines(ui, &region.resolve(&c), p.text, "Result");
                });
            }
        });
    if resp.response.clicked() {
        r.focus = k;
    }
    if focused && r.reveal {
        resp.response.scroll_to_me(Some(egui::Align::Center));
        r.reveal = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::sync::Arc;

    use canopy_core::ConflictFile;

    /// `n` conflicts of `lines` lines a side, each after a few common lines.
    fn text(n: usize, lines: usize) -> String {
        let mut s = String::new();
        for c in 0..n {
            let _ = writeln!(s, "common {c}");
            s.push_str("<<<<<<< HEAD\n");
            for l in 0..lines {
                let _ = writeln!(s, "ours {c}.{l}");
            }
            s.push_str("=======\n");
            for l in 0..lines {
                let _ = writeln!(s, "theirs {c}.{l}");
            }
            s.push_str(">>>>>>> topic\n");
        }
        s
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx
    }

    fn frame(ctx: &egui::Context, r: &mut Resolver) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1000.0, 800.0),
            )),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| body(ui, r));
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn compact_resolver_code_never_paints_over_its_footer() {
        let ctx = context();
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(ConflictFile::parse(&text(1, 5)))));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 200.0));
        for frame in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| {
                            header(ui, &mut r, std::path::Path::new("f"), &mut Vec::new());
                            footer(ui, &mut r, &mut Vec::new());
                            body(ui, &mut r);
                        });
                },
            );
            output.textures_delta.clear();
            let footer = egui::containers::panel::PanelState::load(
                &ctx,
                egui::Id::new(("resolve-footer", &r.path)),
            )
            .expect("resolver footer");
            if frame >= 2 {
                assert!(
                    screen.contains_rect(footer.outer_rect),
                    "frame {frame}: {:?} outside {screen:?}",
                    footer.outer_rect
                );
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape
                        && (text.galley.text().starts_with("common ")
                            || text.galley.text().starts_with("ours ")
                            || text.galley.text().starts_with("theirs "))
                    {
                        assert!(
                            shape.clip_rect.max.y <= footer.outer_rect.min.y,
                            "code clip {:?} enters footer {:?}",
                            shape.clip_rect,
                            footer.outer_rect
                        );
                    }
                }
            }
        }
    }

    /// What a whole-file choice for a conflict of `kind` shows: the button labels painted, and
    /// the buttons a screen reader finds (one node per widget id).
    fn whole_file_buttons(kind: ConflictKind) -> (Vec<String>, usize) {
        let ctx = context();
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx.enable_accesskit();
        let mut r = Resolver::new("f".into());
        r.reload(Some(kind), Ok(Arc::new(ConflictFile::parse("a\n"))));
        let mut painted = Vec::new();
        let mut nodes = 0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(1000.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        whole_file(ui, &r, None, &mut Vec::new());
                    });
                },
            );
            output.textures_delta.clear();
            painted = output
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .filter(|t| (t.starts_with("Delete") || t.starts_with("Keep")) && !t.ends_with('.'))
                .collect();
            nodes = output.platform_output.accesskit_update.map_or(0, |u| {
                u.nodes
                    .iter()
                    .filter(|(_, n)| n.role() == egui::accesskit::Role::Button)
                    .count()
            });
        }
        (painted, nodes)
    }

    #[test]
    fn a_file_both_sides_deleted_offers_one_delete() {
        let (painted, nodes) = whole_file_buttons(ConflictKind::BothDeleted);
        assert_eq!(painted, ["Delete it"], "one button, not two the same");
        assert_eq!(nodes, 1);
        let (painted, nodes) = whole_file_buttons(ConflictKind::DeletedByThem);
        assert_eq!(
            painted.len(),
            2,
            "a real choice keeps both sides: {painted:?}"
        );
        assert_eq!(nodes, 2, "each side its own widget");
    }

    /// QV2-4: a side's preview drops the indentation its lines share, and a line that does not
    /// fit is cut before a word or number, never inside one.
    #[test]
    fn previews_drop_shared_indentation_and_cut_between_words() {
        let lines: Vec<String> = [
            "        Self { attempts: 4, backoff_ms: 250 }",
            "",
            "\t    nested",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(
            common_indent(&lines),
            8,
            "blank lines do not count; a tab is four"
        );
        assert_eq!(
            shown_line(&lines[0], 8),
            "Self { attempts: 4, backoff_ms: 250 }"
        );
        assert_eq!(shown_line(&lines[2], 8), "nested");
        let text = "backoff_ms: 250 }";
        // 14 characters fit: "backoff_ms: 25" would cut the number; the cut goes before it.
        assert_eq!(cut_at_boundary(text, 14), 12);
        assert_eq!(cut_at_boundary(text, 15), 15, "after the whole number");
        assert_eq!(cut_at_boundary(text, 40), text.chars().count());
        // One long word is cut where it must be.
        assert_eq!(cut_at_boundary("aaaaaaaaaaaaaaaaaaaa", 10), 10);
    }

    /// A11Y-1: a conflict's code blocks are named and their text reaches a screen reader.
    #[test]
    fn conflict_code_blocks_are_named_and_spoken() {
        use egui::accesskit::{Action as AkAction, Role};
        let ctx = context();
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx.enable_accesskit();
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(ConflictFile::parse(&text(1, 2)))));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(1000.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| body(ui, &mut r));
                },
            );
            out.textures_delta.clear();
            nodes = out
                .platform_output
                .accesskit_update
                .map(|u| u.nodes.into_iter().map(|(_, n)| n).collect())
                .unwrap_or_default();
        }
        let block = |name: &str| {
            nodes
                .iter()
                .find(|n| n.label() == Some(name))
                .and_then(|n| n.value().map(str::to_owned))
        };
        assert_eq!(
            block("Current version").as_deref(),
            Some("ours 0.0\nours 0.1")
        );
        assert_eq!(
            block("Incoming version").as_deref(),
            Some("theirs 0.0\ntheirs 0.1")
        );
        assert_eq!(block("Unchanged line 1").as_deref(), Some("common 0"));
        let unnamed: Vec<_> = nodes
            .iter()
            .filter(|n| n.supports_action(AkAction::Click) || n.supports_action(AkAction::Focus))
            .filter(|n| !matches!(n.role(), Role::Window | Role::GenericContainer))
            .filter(|n| n.label().is_none_or(str::is_empty) && n.value().is_none())
            .map(|n| format!("{:?}", n.role()))
            .collect();
        assert!(unnamed.is_empty(), "unnamed operable nodes: {unnamed:?}");
    }

    #[test]
    fn only_cards_near_the_view_are_laid_out() {
        let ctx = context();
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(ConflictFile::parse(&text(300, 5)))));
        frame(&ctx, &mut r);
        frame(&ctx, &mut r);
        let measured = r.layout.heights.len();
        assert!(measured > 0 && measured < 20, "{measured} cards laid out");
        // A card far below is laid out when keyboard focus reveals it, so it can scroll there.
        r.focus = 250;
        r.reveal = true;
        frame(&ctx, &mut r);
        assert!(r.layout.heights.contains_key(&(2 * 250 + 1)));
        assert!(!r.reveal);
    }

    #[test]
    fn huge_sides_cost_only_their_visible_lines() {
        let ctx = context();
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(ConflictFile::parse(&text(1, 50_000)))));
        let started = std::time::Instant::now();
        frame(&ctx, &mut r);
        frame(&ctx, &mut r);
        // The card reserves the height of all its lines.
        let height = r.layout.known(1, card_signature(&r, 0, first_region(&r)));
        assert!(height.is_some_and(|h| h > 50_000.0 * 10.0), "{height:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "laid out every line"
        );
    }

    fn first_region(r: &Resolver) -> &ConflictRegion {
        match &r.file {
            Some(Ok(f)) => f.regions().next().expect("a conflict"),
            _ => panic!("no file"),
        }
    }

    #[test]
    fn card_estimates_follow_the_lines_shown() {
        assert_eq!(code_height(0, 15.0, 4.0), 15.0, "\"(nothing)\"");
        assert_eq!(code_height(3, 15.0, 4.0), 53.0);
        let f = ConflictFile::parse(&text(1, 3));
        let region = f.regions().next().expect("conflict");
        assert_eq!(result_len(region, &Choice::Ours), 3);
        assert_eq!(result_len(region, &Choice::OursThenTheirs), 6);
        assert_eq!(result_len(region, &Choice::Custom("a\nb".into())), 2);
        let mut r = Resolver::new("f".into());
        r.reload(None, Ok(Arc::new(f.clone())));
        let before = card_signature(&r, 0, region);
        let (blocks, shape) = card_lines(&r, 0, region);
        assert_eq!((blocks, shape), (vec![3], (false, false)));
        r.choices[0] = Some(Choice::OursThenTheirs);
        assert_ne!(
            card_signature(&r, 0, region),
            before,
            "a choice changes the height"
        );
        let (blocks, shape) = card_lines(&r, 0, region);
        assert_eq!((blocks, shape), (vec![3, 6], (true, false)));
        let layout = Layout::default();
        assert!(
            estimate_card(&layout, &[3, 6], shape, 15.0)
                > estimate_card(&layout, &[3], (false, false), 15.0)
        );
    }
}
