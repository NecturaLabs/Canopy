//! The conflict resolver, shown in Changes for a conflicted file: every conflict as a card with
//! our and their side next to each other (and the common ancestor for diff3 files), a choice per
//! conflict (ours, theirs, both in either order, the ancestor, or a hand edit), the resulting text,
//! and Save / Save and mark resolved. Files Canopy cannot show line by line (binary, deleted on
//! one side) get a whole-file choice instead.

use egui::{FontId, RichText, Stroke, Ui};
use egui_phosphor::regular as icon;

use canopy_core::{Choice, ConflictKind, ConflictRegion, Segment, Side};

use crate::app::Action;
use crate::tab::{RepoTab, Resolver};
use crate::ui::theme::{self, Palette, palette};
use crate::ui::widgets;
use crate::worker::Op;

/// Unchanged stretches longer than this are folded.
const FOLD: usize = 8;
/// Lines kept around a fold.
const KEEP: usize = 3;

fn choice_label(c: &Choice) -> &'static str {
    match c {
        Choice::Ours => "Ours",
        Choice::Theirs => "Theirs",
        Choice::OursThenTheirs => "Ours, then theirs",
        Choice::TheirsThenOurs => "Theirs, then ours",
        Choice::Base => "Ancestor",
        Choice::Custom(_) => "Edited",
    }
}

/// Draws the resolver for the selected conflicted file.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(r) = tab.resolver.as_mut() else {
        return;
    };
    let abs = tab.path.join(&r.path);
    header(ui, r, &abs, actions);
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
                        .size(34.0)
                        .color(p.success),
                );
                ui.label(
                    RichText::new("No conflict markers left")
                        .font(FontId::new(15.0, theme::semibold())),
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
    egui::Frame::new()
        .fill(p.panel)
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::GIT_DIFF).size(16.0).color(p.warning));
                let (name, dir) = widgets::split_path(&r.path);
                ui.label(
                    RichText::new(name)
                        .font(FontId::new(13.5, theme::semibold()))
                        .color(p.text),
                );
                if !dir.is_empty() {
                    ui.label(RichText::new(dir).size(12.0).color(p.text_faint));
                }
                if let Some(kind) = r.kind {
                    ui.label(
                        RichText::new(kind.label())
                            .size(11.5)
                            .color(p.warning)
                            .background_color(Palette::tint(p.warning, 30)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(format!("{}  Open in editor", icon::ARROW_SQUARE_OUT))
                        .on_hover_text("Open the file in your default editor")
                        .clicked()
                    {
                        actions.push(Action::OpenExternal(abs.display().to_string()));
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
                                .size(12.0)
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
            "They deleted this file; you changed it",
            "Keep your version, or delete the file as they did.",
            "Keep my version",
            "Delete the file",
        ),
        (Some(ConflictKind::DeletedByUs), _) => (
            "You deleted this file; they changed it",
            "Keep it deleted, or restore their version.",
            "Keep it deleted",
            "Restore their version",
        ),
        (Some(ConflictKind::AddedByUs), _) => (
            "Only you added this file",
            "Keep it, or leave it out as they did.",
            "Keep it",
            "Leave it out",
        ),
        (Some(ConflictKind::AddedByThem), _) => (
            "Only they added this file",
            "Leave it out, or take their file.",
            "Leave it out",
            "Take their file",
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
            "Use ours",
            "Use theirs",
        ),
        _ => ("Choose a side", "", "Use ours", "Use theirs"),
    };
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(RichText::new(icon::GIT_DIFF).size(34.0).color(p.warning));
        ui.label(RichText::new(title).font(FontId::new(15.0, theme::semibold())));
        if !body.is_empty() {
            ui.label(RichText::new(body).color(p.text_dim));
        }
        if let Some(e) = error {
            ui.label(RichText::new(e).size(11.5).color(p.text_faint));
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
            ui.horizontal(|ui| {
                use crate::keymap::{Command, hint};
                let keys = format!(
                    "{}  {} {} previous/next · {} ours · {} theirs · {} both · {} undo",
                    icon::KEYBOARD,
                    hint(Command::PrevConflict),
                    hint(Command::NextConflict),
                    hint(Command::TakeOurs),
                    hint(Command::TakeTheirs),
                    hint(Command::TakeBoth),
                    hint(Command::ClearChoice),
                );
                ui.label(RichText::new(keys).size(11.5).color(p.text_faint))
                    .on_hover_text("Press ? for every shortcut");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let all = done == n;
                    let mark = widgets::primary_button(ui, "Save and mark resolved", all, 200.0)
                        .on_disabled_hover_text("Choose a resolution for every conflict first");
                    if mark.clicked() {
                        actions.push(Action::Op(Op::Resolve { path: r.path.clone(), content: file.render(&r.choices), mark: true }));
                    }
                    if ui.add_enabled(done > 0, egui::Button::new(format!("{}  Save", icon::FLOPPY_DISK)))
                        .on_hover_text("Write the chosen resolutions to the file; unresolved conflicts keep their markers")
                        .clicked()
                    {
                        actions.push(Action::Op(Op::Resolve { path: r.path.clone(), content: file.render(&r.choices), mark: false }));
                    }
                    if done > 0 && ui.button("Reset").on_hover_text("Clear every choice").clicked() {
                        r.choices.iter_mut().for_each(|c| *c = None);
                        r.editing = None;
                    }
                });
            });
        });
}

fn code_lines(ui: &mut Ui, lines: &[String], color: egui::Color32) {
    for l in lines {
        let text = if l.is_empty() {
            " ".to_owned()
        } else {
            l.replace('\t', "    ")
        };
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .font(FontId::monospace(12.5))
                    .color(color),
            )
            .truncate(),
        );
    }
    if lines.is_empty() {
        ui.label(
            RichText::new("(nothing)")
                .size(11.5)
                .italics()
                .color(palette(ui.ctx()).text_faint),
        );
    }
}

fn body(ui: &mut Ui, r: &mut Resolver) {
    let p = palette(ui.ctx());
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
                    let mut k = 0;
                    for (i, seg) in file.segments.iter().enumerate() {
                        match seg {
                            Segment::Common(lines) => {
                                if lines.len() > FOLD && !r.expanded.contains(&i) {
                                    let first = if i == 0 { &[][..] } else { &lines[..KEEP] };
                                    let last = if i + 1 == file.segments.len() {
                                        &[][..]
                                    } else {
                                        &lines[lines.len() - KEEP..]
                                    };
                                    code_lines(ui, first, p.text_faint);
                                    let hidden = lines.len() - first.len() - last.len();
                                    ui.add_space(2.0);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!(
                                                    "{}  {hidden} unchanged lines",
                                                    icon::DOTS_THREE
                                                ))
                                                .size(11.5)
                                                .color(p.text_faint),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        r.expanded.insert(i);
                                    }
                                    ui.add_space(2.0);
                                    code_lines(ui, last, p.text_faint);
                                } else {
                                    code_lines(ui, lines, p.text_faint);
                                }
                            }
                            Segment::Conflict(region) => {
                                ui.add_space(8.0);
                                conflict_card(ui, r, k, region);
                                ui.add_space(8.0);
                                k += 1;
                            }
                        }
                    }
                });
        });
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
                        .font(FontId::new(13.0, theme::semibold())),
                );
                match &choice {
                    Some(c) => ui.label(
                        RichText::new(format!("{}  {}", icon::CHECK, choice_label(c)))
                            .size(11.5)
                            .color(p.success),
                    ),
                    None => ui.label(
                        RichText::new(format!("{}  Unresolved", icon::WARNING))
                            .size(11.5)
                            .color(p.warning),
                    ),
                };
            });
            ui.add_space(4.0);
            // Both sides next to each other.
            let w = (ui.available_width() - 12.0) / 2.0;
            ui.horizontal_top(|ui| {
                for (title, label, lines, color) in [
                    ("Ours", &region.ours_label, &region.ours, p.accent),
                    ("Theirs", &region.theirs_label, &region.theirs, p.accent2),
                ] {
                    ui.vertical(|ui| {
                        ui.set_width(w);
                        ui.label(
                            RichText::new(format!("{title} · {label}"))
                                .size(11.5)
                                .color(color),
                        );
                        egui::Frame::new()
                            .fill(Palette::tint(color, if p.dark { 22 } else { 16 }))
                            .corner_radius(egui::CornerRadius::same(theme::RADIUS_SM))
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                code_lines(ui, lines, p.text);
                            });
                    });
                }
            });
            if r.show_base
                && let Some(base) = &region.base
            {
                ui.label(
                    RichText::new(format!("Ancestor · {}", region.base_label))
                        .size(11.5)
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
            // Choices.
            let mut picked: Option<Option<Choice>> = None;
            ui.horizontal_wrapped(|ui| {
                let mut options = vec![
                    (Choice::Ours, "Take ours"),
                    (Choice::Theirs, "Take theirs"),
                    (Choice::OursThenTheirs, "Both: ours first"),
                    (Choice::TheirsThenOurs, "Both: theirs first"),
                ];
                if region.base.is_some() {
                    options.push((Choice::Base, "Ancestor"));
                }
                for (c, label) in options {
                    let on = choice.as_ref() == Some(&c);
                    if ui.selectable_label(on, label).clicked() {
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
                        .desired_width(f32::INFINITY),
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
                ui.label(RichText::new("Result").size(11.5).color(p.success));
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
