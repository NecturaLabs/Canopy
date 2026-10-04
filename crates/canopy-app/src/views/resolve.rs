//! The conflict resolver, shown in Changes for a conflicted file: every conflict as a card with
//! our and their side next to each other (and the common ancestor for diff3 files), a choice per
//! conflict (ours, theirs, both in either order, the ancestor, or a hand edit), the resulting text,
//! and Save progress / Save resolved file. Files Canopy cannot show line by line (binary, deleted on
//! one side) get a whole-file choice instead.

use std::collections::HashMap;

use egui::text::LayoutJob;
use egui::{FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::{Choice, ConflictKind, ConflictRegion, Segment, Side};

use crate::action::Action;
use crate::tab::{RepoTab, Resolver};
use crate::ui::theme::{self, Palette, palette};
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
        ui.add(egui::Label::new(RichText::new("During a rebase, Current is the version being rebased onto; Incoming is the commit being replayed.").size(theme::SMALL_SIZE).color(p.text_dim)).wrap());
    }
    match &r.file {
        None => {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.spinner());
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
                        .size(theme::ICON_HERO)
                        .color(p.success),
                );
                ui.label(
                    RichText::new("No conflict markers left")
                        .font(FontId::new(theme::TITLE_SIZE, theme::semibold())),
                );
                ui.label(
                    RichText::new("The file looks resolved. Mark it resolved to stage it.")
                        .color(p.text_dim),
                );
                ui.add_space(8.0);
                if widgets::primary_button(ui, "Mark resolved", true, 150.0).clicked() {
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
    let width = ui.available_width();
    egui::Frame::new()
        .fill(p.panel)
        .inner_margin(egui::Margin::symmetric(theme::LG as i8, theme::SM as i8))
        .show(ui, |ui| {
            ui.set_min_width((width - theme::LG * 2.0).max(0.0));
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(icon::GIT_DIFF)
                        .size(theme::ICON_MD)
                        .color(p.warning),
                );
                let (name, dir) = widgets::split_path(&r.path);
                ui.label(
                    RichText::new(name)
                        .font(FontId::new(theme::LEAD_SIZE, theme::semibold()))
                        .color(p.text),
                );
                if !dir.is_empty() {
                    ui.label(
                        RichText::new(dir)
                            .size(theme::META_SIZE)
                            .color(p.text_faint),
                    );
                }
                if let Some(kind) = r.kind {
                    ui.label(
                        RichText::new(kind.label())
                            .size(theme::SMALL_SIZE)
                            .color(p.warning)
                            .background_color(Palette::tint(p.warning, 30)),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(format!("{}  Open in editor", icon::ARROW_SQUARE_OUT))
                        .on_hover_text("Open the file in your editor (Settings → Integrations)")
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
                            ui.toggle_value(&mut r.show_base, "Show ancestor")
                                .on_hover_text(
                                    "Show the common ancestor's version of each conflict",
                                );
                        }
                        let done = r.resolved();
                        let color = if done == n { p.success } else { p.text_dim };
                        ui.label(
                            RichText::new(format!("{done} of {n} resolved"))
                                .size(theme::META_SIZE)
                                .color(color),
                        );
                    }
                });
            });
        });
    ui.painter().hline(
        ui.min_rect().x_range(),
        ui.min_rect().max.y,
        Stroke::new(1.0, p.border),
    );
}

/// Whole-file choice: binary or non-UTF-8 files, and deleted-on-one-side conflicts.
fn whole_file(ui: &mut Ui, r: &Resolver, error: Option<&str>, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let path = r.path.clone();
    let (title, body, ours, theirs) = match (r.kind, error) {
        (Some(ConflictKind::DeletedByThem), _) => (
            "The incoming side deleted this file; the current side changed it",
            "Keep the current version, or accept the incoming deletion.",
            "Keep current version",
            "Delete the file",
        ),
        (Some(ConflictKind::DeletedByUs), _) => (
            "The current side deleted this file; the incoming side changed it",
            "Keep it deleted, or restore the incoming version.",
            "Keep it deleted",
            "Restore incoming version",
        ),
        (Some(ConflictKind::AddedByUs), _) => (
            "Only the current side added this file",
            "Keep the current file, or accept its absence on the incoming side.",
            "Keep it",
            "Leave it out",
        ),
        (Some(ConflictKind::AddedByThem), _) => (
            "Only the incoming side added this file",
            "Leave it out, or take the incoming file.",
            "Leave it out",
            "Take incoming file",
        ),
        (Some(ConflictKind::BothDeleted), _) => (
            "Both sides deleted this file",
            "It was renamed differently on each side. Delete it here.",
            "Delete it",
            "Delete it",
        ),
        (_, Some(_)) => (
            "Canopy cannot show this file line by line",
            "It is binary, a link, or not UTF-8 text. Choose a whole side, or open it in an editor.",
            "Keep current version",
            "Keep incoming version",
        ),
        _ => (
            "Choose a side",
            "",
            "Keep current version",
            "Keep incoming version",
        ),
    };
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(RichText::new(icon::GIT_DIFF).size(theme::ICON_HERO).color(p.warning));
        ui.label(RichText::new(title).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
        if !body.is_empty() {
            ui.label(RichText::new(body).color(p.text_dim));
        }
        if let Some(e) = error {
            ui.label(RichText::new(e).size(theme::SMALL_SIZE).color(p.text_faint));
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 340.0).max(0.0) / 2.0);
            for (side, label) in [(Side::Ours, ours), (Side::Theirs, theirs)] {
                if ui.add_sized([165.0, 34.0], egui::Button::new(label)).clicked() {
                    actions.push(Action::Confirm {
                        title: format!("{label}?"),
                        body: format!("This replaces {path} in the working tree with that version and marks it resolved. Edits you made to the file are lost."),
                        confirm: label.to_owned(),
                        danger: true,
                        op: Op::ResolveSide { path: path.clone(), side },
                    });
                }
            }
        });
    });
}

fn footer(ui: &mut Ui, r: &mut Resolver, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(Ok(file)) = &r.file else { return };
    let n = file.conflicts();
    let done = r.resolved();
    egui::Panel::bottom(egui::Id::new(("resolve-footer", &r.path)))
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(14, 10)).stroke(Stroke::new(1.0, p.border)))
        .show(ui, |ui| {
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
                ui.add(egui::Label::new(RichText::new(keys).size(theme::SMALL_SIZE).color(p.text_faint)).truncate())
                    .on_hover_text("Press ? for every shortcut");
                ui.label(RichText::new(if done == n {
                    "All conflicts have a choice. Save the resolved file to stage it."
                } else {
                    "Choose a version for each conflict. Save progress keeps markers for unresolved parts."
                }).size(theme::SMALL_SIZE).color(p.text_dim));
                ui.horizontal_wrapped(|ui| {
                    let all = done == n;
                    let mark = widgets::primary_button(ui, "Save resolved file", all, 180.0)
                        .on_hover_text(if all { "Write this file and stage it as resolved" } else { "Choose a resolution for every conflict first" });
                    if mark.clicked() {
                        actions.push(Action::Op(Op::Resolve { path: r.path.clone(), content: file.render(&r.choices), mark: true, expected: file.fingerprint() }));
                    }
                    if ui.add_enabled(done > 0, egui::Button::new(format!("{}  Save progress", icon::FLOPPY_DISK)))
                        .on_hover_text("Write the chosen resolutions to the file; unresolved conflicts keep their markers")
                        .clicked()
                    {
                        actions.push(Action::Op(Op::Resolve { path: r.path.clone(), content: file.render(&r.choices), mark: false, expected: file.fingerprint() }));
                    }
                    if done > 0 && ui.button("Undo all choices").on_hover_text("Clear every choice before saving").clicked() {
                        r.choices.iter_mut().for_each(|c| *c = None);
                        r.editing = None;
                    }
                });
            });
        });
}

fn code_font() -> FontId {
    FontId::monospace(theme::MONO_SIZE)
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
    let n = n.max(1) as f32;
    n * row_h + (n - 1.0) * spacing
}

/// Code lines, one per row, truncated to the width. The block reserves its full height but only
/// the lines inside the visible area are laid out and painted, so a side with thousands of lines
/// costs no more than a short one.
fn code_lines(ui: &mut Ui, lines: &[String], color: egui::Color32) {
    code_lines_at(ui, lines, color, None);
}

fn code_lines_at(ui: &mut Ui, lines: &[String], color: egui::Color32, first_line: Option<usize>) {
    if lines.is_empty() {
        ui.label(
            RichText::new("(nothing)")
                .size(theme::SMALL_SIZE)
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
                FontId::monospace(theme::MONO_SMALL_SIZE),
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
    // The lines are painted, not labels, so copying goes through the context menu.
    resp.context_menu(|ui| {
        if ui.button("Copy lines").clicked() {
            ui.ctx().copy_text(lines.join("\n"));
            ui.close();
        }
    });
    let clip = ui.clip_rect();
    let hover = resp.hover_pos();
    for i in commit_detail::visible_rows(lines.len(), pitch, rect.min.y, clip.min.y, clip.max.y) {
        let text = lines[i].replace('\t', "    ");
        let mut job =
            LayoutJob::single_section(text, egui::TextFormat::simple(font.clone(), color));
        job.wrap = egui::text::TextWrapping::truncate_at_width((width - gutter).max(0.0));
        let galley = ui.painter().layout_job(job);
        let top = rect.min.y + i as f32 * pitch;
        // Like a truncated label: the whole line on hover when it does not fit.
        if galley.elided
            && hover.is_some_and(|h| (top..top + row_h).contains(&h.y))
            && let Some(line) = lines.get(i)
        {
            resp.clone().on_hover_text(line.as_str());
        }
        if let Some(line) = first_line {
            ui.painter().text(
                egui::pos2(rect.min.x + gutter - theme::SM, top + row_h / 2.0),
                egui::Align2::RIGHT_CENTER,
                (line + i).to_string(),
                FontId::monospace(theme::MONO_SMALL_SIZE),
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
        .id_salt(("resolve", &r.path))
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let width = ui.available_width();
                    if r.layout.width != width {
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
            code_lines_at(ui, first, p.text, Some(first_line));
        }
        let hidden = lines.len() - first.len() - last.len();
        ui.add_space(2.0);
        if ui
            .add(
                egui::Button::new(
                    RichText::new(format!("{}  {hidden} unchanged lines", icon::DOTS_THREE))
                        .size(theme::SMALL_SIZE)
                        .color(p.text_faint),
                )
                .frame(false),
            )
            .clicked()
        {
            r.expanded.insert(i);
        }
        ui.add_space(2.0);
        if !last.is_empty() {
            code_lines_at(
                ui,
                last,
                p.text,
                Some(first_line + lines.len() - last.len()),
            );
        }
    } else if !lines.is_empty() {
        code_lines_at(ui, lines, p.text, Some(first_line));
    }
}

fn conflict_card(ui: &mut Ui, r: &mut Resolver, k: usize, region: &ConflictRegion) {
    let p = palette(ui.ctx());
    let focused = r.focus == k;
    let choice = r.choices.get(k).cloned().flatten();
    let border = match (&choice, focused) {
        (_, true) => p.accent,
        (Some(_), false) => Palette::tint(p.success, 160),
        (None, false) => Palette::tint(p.warning, 160),
    };
    let resp = egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(if focused { 2.0 } else { 1.0 }, border))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Conflict {}", k + 1))
                        .font(FontId::new(theme::BODY_SIZE, theme::semibold())),
                );
                match &choice {
                    Some(c) => ui.label(
                        RichText::new(format!("{}  {}", icon::CHECK, choice_label(c)))
                            .size(theme::SMALL_SIZE)
                            .color(p.success),
                    ),
                    None => ui.label(
                        RichText::new(format!("{}  Unresolved", icon::WARNING))
                            .size(theme::SMALL_SIZE)
                            .color(p.warning),
                    ),
                };
            });
            ui.add_space(4.0);
            let mut picked: Option<Option<Choice>> = None;
            // Both sides next to each other.
            let w = (ui.available_width() - 12.0) / 2.0;
            ui.horizontal_top(|ui| {
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
                        ui.label(
                            RichText::new(if label.is_empty() || label == "HEAD" {
                                title.to_owned()
                            } else {
                                format!("Version from {label}")
                            })
                            .size(theme::SMALL_SIZE)
                            .color(color),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Git: {} · {label}",
                                if side == Choice::Ours {
                                    "ours"
                                } else {
                                    "theirs"
                                }
                            ))
                            .size(theme::SMALL_SIZE)
                            .color(p.text_faint),
                        );
                        egui::Frame::new()
                            .fill(Palette::tint(color, if p.dark { 22 } else { 16 }))
                            .corner_radius(egui::CornerRadius::same(theme::RADIUS_SM))
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                code_lines(ui, lines, p.text);
                            });
                        let on = choice.as_ref() == Some(&side);
                        let label = format!(
                            "{}Keep this version  ({})",
                            if on { "✓ " } else { "" },
                            crate::keymap::hint(command)
                        );
                        if widgets::compact_button(ui, label).clicked() {
                            picked = Some(if on { None } else { Some(side) });
                        }
                    });
                }
            });
            if r.show_base
                && let Some(base) = &region.base
            {
                ui.label(
                    RichText::new(format!("Ancestor · {}", region.base_label))
                        .size(theme::SMALL_SIZE)
                        .color(p.text_dim),
                );
                egui::Frame::new()
                    .fill(p.hover)
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS_SM))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        code_lines(ui, base, p.text_dim);
                    });
            }
            ui.add_space(4.0);
            // Combined and custom choices.
            ui.horizontal_wrapped(|ui| {
                let mut options = vec![
                    (Choice::OursThenTheirs, "Both: current first"),
                    (Choice::TheirsThenOurs, "Both: incoming first"),
                ];
                if region.base.is_some() {
                    options.push((Choice::Base, "Ancestor"));
                }
                for (c, label) in options {
                    let on = choice.as_ref() == Some(&c);
                    if widgets::compact_button(ui, format!("{}{label}", if on { "✓ " } else { "" }))
                        .clicked()
                    {
                        picked = Some(if on { None } else { Some(c) });
                    }
                }
                let editing = r.editing.as_ref().is_some_and(|(i, _)| *i == k);
                if ui
                    .selectable_label(
                        editing || matches!(choice, Some(Choice::Custom(_))),
                        format!("{}  Edit", icon::PENCIL_SIMPLE),
                    )
                    .clicked()
                    && !editing
                {
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
                ui.add_space(4.0);
                let rows = text.lines().count().clamp(3, 16);
                ui.add(
                    egui::TextEdit::multiline(text)
                        .code_editor()
                        .desired_rows(rows)
                        .desired_width(f32::INFINITY)
                        .margin(theme::FIELD_MARGIN),
                );
                ui.horizontal(|ui| {
                    if widgets::accent_button(ui, "Use this text", egui::vec2(120.0, 28.0))
                        .clicked()
                    {
                        apply = Some(text.clone());
                    }
                    if ui.button("Cancel").clicked() {
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
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Result")
                        .size(theme::SMALL_SIZE)
                        .color(p.success),
                );
                egui::Frame::new()
                    .fill(Palette::tint(p.success, if p.dark { 18 } else { 14 }))
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS_SM))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        code_lines(ui, &region.resolve(&c), p.text);
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
    use std::sync::Arc;

    use canopy_core::ConflictFile;

    /// `n` conflicts of `lines` lines a side, each after a few common lines.
    fn text(n: usize, lines: usize) -> String {
        let mut s = String::new();
        for c in 0..n {
            s.push_str(&format!("common {c}\n"));
            s.push_str("<<<<<<< HEAD\n");
            for l in 0..lines {
                s.push_str(&format!("ours {c}.{l}\n"));
            }
            s.push_str("=======\n");
            for l in 0..lines {
                s.push_str(&format!("theirs {c}.{l}\n"));
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
