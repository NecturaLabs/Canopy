//! The staging area: conflicted, unstaged and staged files, the commit composer and the diff.

use egui::{Align, FontId, Layout, Rect, Response, RichText, Ui, UiBuilder, pos2, vec2};
use egui_phosphor::regular as icon;

use canopy_core::identity::Identity;
use canopy_core::{DiffSource, FileEntry};

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::{FileSelection, RepoTab};
use crate::ui::kit::anim::Flip;
use crate::ui::kit::chip::CountBadge;
use crate::ui::kit::field::{TextArea, TextField};
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{RowKind, SectionHeader, caps_galley};
use crate::ui::kit::split::{self, SplitAxis, SplitHandle};
use crate::ui::kit::toggle::Checkbox;
use crate::ui::kit::{Button, IconButton, Size, Tone, menu, paint};
use crate::ui::theme::{self, palette};
use crate::ui::{art, widgets};
use crate::views::{commit_detail, diff, resolve, state_banner};
use crate::worker::Op;

#[derive(Clone, Copy, PartialEq, Eq)]
enum List {
    Conflicted,
    Unstaged,
    Staged,
}

/// The id of the file lists' panel, beside the diff (its size is kept under it).
const LIST_PANEL: &str = "changes-list";
/// The salt of the handle between the file lists and the diff.
const LIST_SPLIT: &str = "changes-list";

/// Draws the changes view. Returns the file lists' panel rectangle and the id of the handle
/// between them and the diff when it showed them (Deeper gives the lists and the diff a card
/// each).
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) -> Option<(Rect, egui::Id)> {
    let s = theme::surfaces(ui.ctx());
    let Some(snap) = tab.snap.clone() else {
        ui.centered_and_justified(|ui| Spinner::new().label("changes").show(ui));
        return None;
    };
    state_banner::show(ui, tab, actions);
    // Default selection: the first changed file.
    if tab
        .file
        .as_ref()
        .is_none_or(|f| matches!(f.source, DiffSource::Commit(_)))
    {
        let st = &snap.status;
        let first = st
            .conflicted
            .first()
            .map(|e| (DiffSource::Unstaged, e, true))
            .or_else(|| {
                st.unstaged
                    .first()
                    .map(|e| (DiffSource::Unstaged, e, false))
            })
            .or_else(|| st.staged.first().map(|e| (DiffSource::Staged, e, false)));
        if let Some((source, entry, conflicted)) = first {
            tab.select_file(FileSelection {
                source,
                entry: entry.clone(),
                conflicted,
            });
        } else if tab.file.is_some() {
            tab.deselect_file();
        }
    }

    let pane_max = (ui.available_width() * 0.4).max(160.0);
    let (lo, hi, default_width) = (
        pane_max.min(220.0),
        pane_max.min(700.0),
        pane_max.min(400.0),
    );
    // The lists' width: where their handle is being dragged to, else the panel's own (kept across
    // restarts).
    let panel_id = egui::Id::new(LIST_PANEL);
    let handle_id = SplitHandle::id_in(ui, LIST_SPLIT);
    let stored = egui::containers::panel::PanelState::load(ui.ctx(), panel_id)
        .map_or(default_width, |state| state.size().x);
    let mut width = split::dragged_size(ui.ctx(), handle_id, SplitAxis::Columns, false, lo..=hi)
        .unwrap_or(stored)
        .clamp(lo, hi);
    let list = egui::Panel::left(panel_id)
        .resizable(false)
        .exact_size(width)
        .frame(egui::Frame::new().fill(s.pane))
        .show(ui, |ui| {
            // The seam to the diff; the pane's other edges are the card's or the window's.
            let edge = ui.max_rect();
            let seam = paint::pane_seam(palette(ui.ctx()), s.border, false);
            let w = seam.map_or(0.0, |st| st.width);
            paint::seam_line_v(ui.painter(), edge.right() - w * 0.5, edge.y_range(), seam);
            let list_frame = egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 8));
            // The composer follows the lists while both fit; once the lists would crowd it, it
            // pins to the bottom and the lists scroll above it.
            let need = lists_height(&snap.status) + COMPOSER_GAP + composer_height(ui, tab, &snap);
            let follows = snap.status_unavailable.is_none() && need <= ui.available_height();
            if follows {
                egui::CentralPanel::default()
                    .frame(list_frame)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let mut flip = list_motion(ui, tab);
                                lists(ui, tab, &snap, actions, &mut flip);
                                // The composer follows the lists as they change.
                                flip.item(ui, composer_key(), None, |ui| {
                                    ui.add_space(COMPOSER_GAP / 2.0);
                                    let rule = ui.max_rect().x_range();
                                    paint::seam_h(ui.painter(), rule, ui.cursor().min.y, s.border);
                                    ui.add_space(COMPOSER_GAP / 2.0);
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::symmetric(4, 0))
                                        .show(ui, |ui| composer(ui, tab, &snap, actions, true));
                                });
                                flip.end(ui, |key| listed(&snap.status, key));
                            });
                    });
                return;
            }
            let composer_height = (ui.available_height() * 0.60).max(60.0);
            // Short windows show a two-row description (`composer_fields`), so the composer
            // needs less room and the file lists keep it.
            let cap = if ui.ctx().content_rect().height() < 600.0 {
                260.0
            } else {
                300.0
            };
            egui::Panel::bottom("composer")
                .default_size(composer_height.min(cap))
                .min_size(composer_height.min(cap.min(250.0)))
                .max_size(composer_height.min(cap))
                .frame(
                    egui::Frame::new()
                        .fill(s.pane)
                        .inner_margin(egui::Margin::same(12)),
                )
                .show(ui, |ui| composer(ui, tab, &snap, actions, false));
            egui::CentralPanel::default()
                .frame(list_frame)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let mut flip = list_motion(ui, tab);
                            lists(ui, tab, &snap, actions, &mut flip);
                            flip.end(ui, |key| listed(&snap.status, key));
                        });
                });
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(s.pane))
        .show(ui, |ui| {
            if snap.status_unavailable.is_none() && snap.status.is_clean() {
                clean_home(ui, tab, &snap, actions);
            } else if tab.resolver.is_some() && tab.file.as_ref().is_some_and(|f| f.conflicted) {
                resolve::show(ui, tab, actions);
            } else {
                diff::show(ui, tab, actions);
            }
        });
    // Shown over both panes, so their scrolling cannot take the drag from it.
    let rect = list.response.rect;
    let handle = SplitHandle::new(LIST_SPLIT, &mut width, SplitAxis::Columns)
        .range(lo..=hi)
        .default_size(default_width)
        .label("Resize the file lists")
        .seam(false)
        .show(ui, rect.right(), rect.y_range());
    if handle.changed()
        && let Some(mut state) = egui::containers::panel::PanelState::load(ui.ctx(), panel_id)
    {
        state.outer_rect.max.x = state.outer_rect.min.x + width;
        ui.ctx()
            .data_mut(|data| data.insert_persisted(panel_id, state));
        ui.ctx().request_repaint();
    }
    Some((rect, handle.id))
}

/// Gap between the last list and the composer when the composer follows the lists.
const COMPOSER_GAP: f32 = 28.0;
/// Gap between two file lists, and the height of an empty list's placeholder line.
const LIST_GAP: f32 = theme::MD;
const EMPTY_LIST: f32 = theme::XL;
/// The clean tree's one call to action ("Push", "Pull", "Publish…") stays a proper button under
/// the illustration however short its label.
const CTA_MIN_WIDTH: f32 = 120.0;

/// Height of the sections `lists` draws, close enough to decide whether the composer fits
/// below them. An overestimate only pins the composer; an underestimate scrolls.
fn lists_height(st: &canopy_core::WorkingStatus) -> f32 {
    if st.is_clean() {
        return 40.0;
    }
    let (conflicts, unstaged, staged) = visible_lists(st);
    let header = theme::metrics().section_header_height;
    let row_h = RowKind::File.height();
    let section = |n: usize| {
        let body = if n == 0 {
            EMPTY_LIST
        } else {
            crate::ui::count_f32(n) * row_h
        };
        header + theme::XS + body + LIST_GAP
    };
    let mut h = 16.0;
    if conflicts {
        h += section(st.conflicted.len());
    }
    if unstaged {
        h += section(st.unstaged.len());
    }
    if staged {
        h += section(st.staged.len());
    }
    h
}

/// Height of the composer as `composer` lays it out in `follows` mode.
fn composer_height(ui: &Ui, tab: &RepoTab, snap: &canopy_core::RepoSnapshot) -> f32 {
    if composer_is_quiet(tab, snap) {
        return 76.0;
    }
    let short = ui.ctx().content_rect().height() < 600.0;
    let mut h = if short { 224.0 } else { 262.0 };
    if identity_notice(tab.identity.as_ref(), tab.identity_error.is_some()).is_some() {
        h += 56.0;
    }
    h + 24.0
}

/// A list's header: the uppercase title, its count in a badge beside it, and `trailing`
/// controls laid out right to left at its end.
fn list_header(ui: &mut Ui, title: &str, count: usize, tone: Tone, trailing: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    let rect = SectionHeader::new(title).first(true).show(ui).response.rect;
    // The section header insets its title by 6.
    let title_w = caps_galley(ui, title, p.text_faint).size().x;
    let mut badge = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.left() + 6.0 + title_w + theme::SM, rect.top()),
                rect.max,
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    CountBadge::new(count).tone(tone).show(&mut badge);
    let mut end = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    end.spacing_mut().item_spacing.x = theme::XS;
    trailing(&mut end);
}

/// The sections of the file column: the not-available notice, the clean placeholder, or the
/// conflicted, unstaged and staged lists.
fn lists(
    ui: &mut Ui,
    tab: &mut RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
    flip: &mut Flip,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let spacing = ui.spacing().item_spacing.y;
    let row_height = ui.spacing().interact_size.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    // Passive list headings and placeholders need no button-sized minimum row.
    // Actual actions keep their explicit compact-control target size.
    ui.spacing_mut().interact_size.y = theme::XL;
    let st = &snap.status;
    if let Some(u) = &snap.status_unavailable {
        // A partial clone that lacks the tree of HEAD: the list is empty because
        // it could not be read, not because nothing changed.
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            art::show(ui, art::empty_error(), vec2(160.0, 120.0));
            ui.add_space(6.0);
            ui.label(RichText::new("Changes not available").font(theme::heading(m.subtitle_size)));
            ui.label(
                RichText::new(&u.message)
                    .color(p.text_dim)
                    .size(m.meta_size),
            );
            if u.can_download {
                ui.add_space(theme::SM);
                if Button::new("Retry")
                    .icon(icon::ARROWS_CLOCKWISE)
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::Refresh);
                }
            }
        });
    } else if st.is_clean() {
        // The illustration and the summary live in the main pane (`clean_home`); here the
        // column only says there is nothing to list.
        flip.item(ui, clean_key(), None, |ui| {
            list_header(ui, "Changes", 0, Tone::Neutral, |_| {});
        });
    } else {
        let (conflicts, unstaged, staged) = visible_lists(st);
        if conflicts {
            list(ui, tab, List::Conflicted, &st.conflicted, actions, flip);
            ui.add_space(LIST_GAP);
        }
        if unstaged {
            list(ui, tab, List::Unstaged, &st.unstaged, actions, flip);
            ui.add_space(LIST_GAP);
        }
        if staged {
            list(ui, tab, List::Staged, &st.staged, actions, flip);
        }
    }
    ui.spacing_mut().item_spacing.y = spacing;
    ui.spacing_mut().interact_size.y = row_height;
}

/// The main pane of a clean working tree: the illustration and where the branch stands, with
/// the next step (Push, Pull, Publish) when there is one.
fn clean_home(
    ui: &mut Ui,
    tab: &RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let busy = tab.running.is_some();
    let branch = snap
        .head
        .branch
        .as_ref()
        .and_then(|name| snap.branches.iter().find(|b| b.is_head && &b.name == name));
    let last = snap
        .head
        .oid
        .and_then(|oid| snap.commits.iter().find(|c| c.oid == oid));
    let top = ((ui.available_height() - 380.0) / 2.0).max(24.0);
    egui::ScrollArea::vertical()
        .id_salt("clean-summary")
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(top);
                art::show(ui, art::empty_clean(), vec2(200.0, 150.0));
                ui.add_space(theme::SM);
                ui.label(RichText::new("Working tree clean").font(theme::heading(m.display_size)));
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!(
                        "Nothing to commit on {}. Edit some files and they show up here.",
                        snap.head.label()
                    ))
                    .color(p.text_dim)
                    .size(m.meta_size),
                );
                ui.add_space(theme::LG);
                let (status, next) = match (branch, snap.remotes.is_empty()) {
                    (_, true) => (
                        format!("{}  Local only, no remote", icon::HARD_DRIVES),
                        Some(("Publish…", Action::Publish)),
                    ),
                    (Some(b), false) if b.upstream.is_some() => {
                        let upstream = b.upstream.as_deref().unwrap_or_default();
                        match (b.ahead, b.behind) {
                            (0, 0) => (
                                format!("{}  Up to date with {upstream}", icon::CHECK_CIRCLE),
                                None,
                            ),
                            (a, 0) => (
                                format!(
                                    "{}  {a} commit{} to push to {upstream}",
                                    icon::ARROW_FAT_LINE_UP,
                                    if a == 1 { "" } else { "s" }
                                ),
                                Some(("Push", Action::Push)),
                            ),
                            (0, n) => (
                                format!(
                                    "{}  {n} commit{} to pull from {upstream}",
                                    icon::ARROW_FAT_LINE_DOWN,
                                    if n == 1 { "" } else { "s" }
                                ),
                                Some(("Pull", Action::Pull)),
                            ),
                            (a, n) => (
                                format!("{}  {a} ahead, {n} behind {upstream}", icon::GIT_DIFF),
                                Some(("Pull", Action::Pull)),
                            ),
                        }
                    }
                    (Some(_), false) => (
                        format!("{}  Not pushed yet", icon::CLOUD_ARROW_UP),
                        Some(("Push", Action::Push)),
                    ),
                    (None, false) => (format!("{}  {}", icon::GIT_BRANCH, snap.head.label()), None),
                };
                commit_detail::without_icons(
                    ui.label(RichText::new(status).size(m.body_size).color(p.text)),
                );
                if let Some((label, action)) = next {
                    ui.add_space(theme::SM);
                    if Button::new(label)
                        .primary()
                        .min_width(CTA_MIN_WIDTH)
                        .enabled(!busy)
                        .disabled_reason("Another operation is still running")
                        .show(ui)
                        .clicked()
                    {
                        actions.push(action);
                    }
                }
                if let Some(c) = last {
                    ui.add_space(theme::LG);
                    let now = chrono::Utc::now().timestamp();
                    ui.label(
                        RichText::new(format!(
                            "Last commit {}  ·  {}",
                            canopy_core::short_id(c.oid),
                            widgets::relative_time(c.author.time, now)
                        ))
                        .size(m.small_size)
                        .color(p.text_faint),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&c.summary)
                                .size(m.meta_size)
                                .color(p.text_dim),
                        )
                        .truncate(),
                    );
                }
            });
        });
}

/// Which of the conflicted, unstaged and staged sections to draw. An empty section is left out
/// while another has content (a merge with every file conflicted has no use for "Unstaged 0");
/// with nothing to show, or only an empty staging area, it stays so the column is never blank.
fn visible_lists(st: &canopy_core::WorkingStatus) -> (bool, bool, bool) {
    let conflicts = !st.conflicted.is_empty();
    let unstaged = !st.unstaged.is_empty() || (!conflicts && st.staged.is_empty());
    let staged = !st.staged.is_empty() || (!conflicts && !st.unstaged.is_empty());
    (conflicts, unstaged, staged)
}

/// The file lists' rows as [`Flip`] items for the tab's current data: a file staged or unstaged
/// slides to its new section, new files fade in and the rest close up smoothly.
fn list_motion(ui: &Ui, tab: &RepoTab) -> Flip {
    let id = egui::Id::new(("changes-list-motion", &tab.path));
    Flip::begin(ui, id, tab.generation, ui.cursor().min).ground(theme::surfaces(ui.ctx()).pane)
}

/// Keys of the file lists' [`Flip`] items: a section's header and empty line, a file's row in a
/// section (aliased by its path, so it slides between sections), the clean tree's header and the
/// composer that follows the lists.
fn header_key(which: List) -> egui::Id {
    egui::Id::new(("changes-header", which as u8))
}
fn empty_key(which: List) -> egui::Id {
    egui::Id::new(("changes-empty", which as u8))
}
fn row_key(which: List, path: &str) -> egui::Id {
    egui::Id::new(("changes-row", which as u8, path))
}
fn path_key(path: &str) -> egui::Id {
    egui::Id::new(("changes-file", path))
}
fn clean_key() -> egui::Id {
    egui::Id::new("changes-clean")
}
fn composer_key() -> egui::Id {
    egui::Id::new("changes-composer")
}

/// Whether the item `key` is still in the lists of `st` (a row out of view is, and is not faded
/// out). Asked only for rows that were on screen and are not now, when the status changed.
fn listed(st: &canopy_core::WorkingStatus, key: egui::Id) -> bool {
    if key == composer_key() {
        return true;
    }
    if st.is_clean() {
        return key == clean_key();
    }
    let (conflicts, unstaged, staged) = visible_lists(st);
    [
        (List::Conflicted, conflicts, &st.conflicted),
        (List::Unstaged, unstaged, &st.unstaged),
        (List::Staged, staged, &st.staged),
    ]
    .into_iter()
    .any(|(which, shown, files)| {
        shown
            && (key == header_key(which)
                || (files.is_empty() && key == empty_key(which))
                || files.iter().any(|f| key == row_key(which, &f.path)))
    })
}

/// The confirmation before the unstaged changes to `entry` are thrown away.
fn discard_one(entry: &FileEntry) -> Action {
    Action::Confirm {
        title: format!("Discard changes to {}?", entry.path),
        body: "Its unstaged changes are thrown away. This cannot be undone.".into(),
        confirm: "Discard changes".into(),
        danger: true,
        op: Op::Discard(vec![entry.clone()]),
    }
}

/// The confirmation before every unstaged change in `files` is thrown away.
fn discard_all(files: &[FileEntry]) -> Action {
    let n = files.len();
    Action::Confirm {
        title: if n == 1 {
            "Discard changes to 1 file?".to_owned()
        } else {
            format!("Discard changes to all {n} files?")
        },
        body: "Every unstaged change is thrown away, untracked files included. This cannot be \
               undone."
            .into(),
        confirm: "Discard all changes".into(),
        danger: true,
        op: Op::Discard(files.to_vec()),
    }
}

fn list(
    ui: &mut Ui,
    tab: &mut RepoTab,
    which: List,
    files: &[FileEntry],
    actions: &mut Vec<Action>,
    flip: &mut Flip,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (title, tone) = match which {
        List::Conflicted => ("Conflicts", Tone::Danger),
        List::Unstaged => ("Unstaged", Tone::Warning),
        List::Staged => ("Staged", Tone::Success),
    };
    // An empty list is not news: its count stays neutral instead of taking the section colour.
    let tone = if files.is_empty() {
        Tone::Neutral
    } else {
        tone
    };
    let compact = ui.available_width() < 280.0;
    flip.item(ui, header_key(which), None, |ui| {
        list_header(ui, title, files.len(), tone, |ui| match which {
            List::Unstaged if !files.is_empty() => {
                let stage = if compact {
                    IconButton::new(icon::PLUS, "Stage all files")
                        .size(Size::Sm)
                        .shortcut(Command::StageAll)
                        .show(ui)
                } else {
                    Button::new("Stage all")
                        .icon(icon::PLUS)
                        .size(Size::Sm)
                        .tooltip("Stage all files")
                        .shortcut(Command::StageAll)
                        .show(ui)
                };
                if stage.clicked() {
                    actions.push(Action::Op(Op::StageAll));
                }
                // Discarding everything lives in an overflow menu, a gap away from Stage.
                ui.add_space(theme::MD);
                let more = IconButton::new(icon::DOTS_THREE, "More actions")
                    .size(Size::Sm)
                    .show(ui);
                menu::menu(&more, |m| {
                    if m.item(icon::ARROW_COUNTER_CLOCKWISE, "Discard all changes…")
                        .danger()
                        .show(m.ui)
                        .clicked()
                    {
                        actions.push(discard_all(files));
                    }
                });
            }
            List::Staged if !files.is_empty() => {
                let unstage = if compact {
                    IconButton::new(icon::MINUS, "Unstage all files")
                        .size(Size::Sm)
                        .shortcut(Command::UnstageAll)
                        .show(ui)
                } else {
                    Button::new("Unstage all")
                        .icon(icon::MINUS)
                        .size(Size::Sm)
                        .tooltip("Unstage all files")
                        .shortcut(Command::UnstageAll)
                        .show(ui)
                };
                if unstage.clicked() {
                    actions.push(Action::Op(Op::UnstageAll));
                }
            }
            _ => {}
        });
        ui.add_space(theme::XS);
    });
    if files.is_empty() {
        flip.item(ui, empty_key(which), None, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(theme::MD);
                ui.label(
                    RichText::new(if which == List::Staged {
                        "Nothing staged yet"
                    } else {
                        "No unstaged changes"
                    })
                    .size(m.meta_size)
                    .color(p.text_faint),
                );
            });
        });
        return;
    }
    commit_detail::virtual_rows(ui, files.len(), RowKind::File.height(), |ui, i| {
        let f = &files[i];
        let source = if which == List::Staged {
            DiffSource::Staged
        } else {
            DiffSource::Unstaged
        };
        let conflicted = which == List::Conflicted;
        let selected = tab.file.as_ref().is_some_and(|s| {
            s.entry.path == f.path && s.source == source && s.conflicted == conflicted
        });
        // The row's action, revealed under the pointer and on the selected row. A conflicted
        // file's is a labelled "Resolve conflicts…": a bare icon would read as "already done".
        let (glyph, tip) = if which == List::Staged {
            (icon::MINUS, "Unstage")
        } else if conflicted {
            (icon::GIT_DIFF, "Resolve conflicts…")
        } else {
            (icon::PLUS, "Stage")
        };
        let action = RowAction {
            glyph,
            tip,
            labelled: conflicted,
        };
        let (resp, acted) = flip.item(ui, row_key(which, &f.path), Some(path_key(&f.path)), |ui| {
            file_row(ui, f, selected, ("change", which as u8, &f.path), action)
        });
        let toggle = || {
            if which == List::Staged {
                Op::Unstage(vec![f.path.clone()])
            } else {
                Op::Stage(vec![f.path.clone()])
            }
        };
        if acted || resp.double_clicked() {
            if conflicted {
                tab.select_file(FileSelection {
                    source,
                    entry: f.clone(),
                    conflicted,
                });
            } else {
                actions.push(Action::Op(toggle()));
            }
        } else if resp.clicked() {
            tab.select_file(FileSelection {
                source,
                entry: f.clone(),
                conflicted,
            });
        }
        let entry = f.clone();
        menu::context(&resp, |m| {
            let first = m.item(glyph, tip);
            let first = if conflicted {
                first
            } else {
                first.shortcut(Command::ToggleStage)
            };
            if first.show(m.ui).clicked() {
                if conflicted {
                    tab.select_file(FileSelection {
                        source,
                        entry: f.clone(),
                        conflicted,
                    });
                } else {
                    actions.push(Action::Op(toggle()));
                }
            }
            m.separator();
            if m.item(icon::ARROW_SQUARE_OUT, "Open file")
                .show(m.ui)
                .clicked()
            {
                actions.push(Action::OpenInEditor(tab.path.join(&entry.path)));
            }
            if m.item(icon::COPY, "Copy path").show(m.ui).clicked() {
                actions.push(Action::Copy(entry.path.clone()));
            }
            if which == List::Unstaged {
                m.separator();
                if m.item(icon::ARROW_COUNTER_CLOCKWISE, "Discard changes…")
                    .shortcut(Command::Discard)
                    .danger()
                    .show(m.ui)
                    .clicked()
                {
                    actions.push(discard_one(&entry));
                }
            }
        });
    });
}

/// The button a file row reveals at its right end.
#[derive(Clone, Copy)]
struct RowAction {
    glyph: &'static str,
    /// Its tooltip, and its label when `labelled`.
    tip: &'static str,
    /// A Ghost button with the label instead of an icon button.
    labelled: bool,
}

/// A changed file on the shared file-row path ([`commit_detail::file_list_row_with`]): its
/// status badge, name, folder and line counts, and `action` revealed at its right end while the
/// row is hovered, selected or focused. Returns the row and whether the action was clicked.
fn file_row(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    action: RowAction,
) -> (Response, bool) {
    let id = ui.id().with(id_salt);
    let width = if action.labelled {
        labelled_action_width(ui, action.tip)
    } else {
        commit_detail::icon_actions_width()
    };
    let mut acted = false;
    let row = commit_detail::file_list_row_with(ui, f, selected, id, width, |ui| {
        let salt = ("file-row-action", &f.path);
        let r = if action.labelled {
            Button::new(action.tip)
                .ghost()
                .size(Size::Sm)
                .id_salt(salt)
                .show(ui)
        } else {
            IconButton::new(action.glyph, action.tip)
                .size(Size::Sm)
                .id_salt(salt)
                .show(ui)
        };
        r.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{} {}", action.tip, f.path),
            )
        });
        acted = r.clicked();
    });
    (row, acted)
}

/// Room a labelled Ghost `Size::Sm` action takes at a row's end: its text, the button's side
/// padding and a gap before the line counts.
fn labelled_action_width(ui: &Ui, label: &str) -> f32 {
    let m = theme::metrics();
    let text = ui
        .painter()
        .layout_no_wrap(
            label.to_owned(),
            FontId::new(m.body_size, theme::medium()),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x;
    text + 2.0 * m.button_pad_x_sm + theme::XS
}

/// The commit composer. `follows` is true when it sits under the file lists in their scroll
/// area; otherwise it fills the panel pinned to the bottom of the column, with the fields
/// scrolling above the commit button.
fn composer(
    ui: &mut Ui,
    tab: &mut RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
    follows: bool,
) {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    ui.style_mut().spacing.scroll.floating = false;
    if composer_is_quiet(tab, snap) {
        quiet_composer(ui, tab, snap, actions);
        return;
    }
    let staged = snap.status.staged.len();
    let merging = snap.state == canopy_core::RepoState::Merge;
    let can = tab.can_commit();
    let blocker = if can {
        None
    } else {
        commit_blocker(
            !snap.status.conflicted.is_empty(),
            tab.running.is_some(),
            staged > 0 || tab.amend || merging,
            tab.commit_summary.trim().is_empty(),
        )
    };
    let branch = snap.head.label();
    // The full label first, then shorter ones for a column too narrow for it.
    let labels = if merging {
        vec![
            format!("Conclude merge on {branch}"),
            "Conclude merge".to_owned(),
        ]
    } else if tab.amend {
        vec![format!("Amend {branch}"), "Amend".to_owned()]
    } else if staged > 0 {
        let files = format!("{staged} file{}", if staged == 1 { "" } else { "s" });
        vec![
            format!("Commit {files} to {branch}"),
            format!("Commit {files}"),
            "Commit".to_owned(),
        ]
    } else {
        vec!["Stage files to commit".to_owned(), "Stage files".to_owned()]
    };
    let warning = identity_notice(tab.identity.as_ref(), tab.identity_error.is_some());
    let tip = warning.map_or_else(
        || "Commit the staged changes".to_owned(),
        |w| format!("{w} Open Settings → Git to correct it. Git's own error will appear if you commit without a usable identity."),
    );
    let footer = |ui: &mut Ui, actions: &mut Vec<Action>| {
        if let Some(warning) = warning {
            ui.horizontal_wrapped(|ui| {
                commit_detail::without_icons(
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}  {warning}", icon::WARNING))
                                .color(p.warning)
                                .size(m.small_size),
                        )
                        .wrap(),
                    ),
                );
                if Button::new("Edit identity")
                    .size(Size::Sm)
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::IdentitySettings);
                }
            });
            ui.add_space(theme::XS);
        }
        if let Some(reason) = blocker {
            ui.label(RichText::new(reason).size(m.small_size).color(p.text_dim));
            ui.add_space(theme::XS);
        }
        let (label, keys) = fitting_label(ui, &labels, ui.available_width());
        let mut commit = Button::new(label)
            .primary()
            .icon(icon::CHECK)
            .full_width()
            .enabled(can)
            .tooltip(tip.as_str())
            .shortcut(Command::Commit)
            .show_shortcut(keys);
        if let Some(reason) = blocker {
            commit = commit.disabled_reason(reason);
        }
        let commit = commit.show(ui);
        if let Some(full) = labels.first().filter(|full| full.as_str() != label) {
            commit
                .ctx
                .accesskit_node_builder(commit.id, |node| node.set_label(full.as_str()));
        }
        if commit_detail::say_unavailable(commit, blocker.unwrap_or_default()).clicked() && can {
            actions.push(Action::Commit);
        }
    };
    if follows {
        composer_fields(ui, tab, actions);
        footer(ui, actions);
        return;
    }
    // The button and the identity warning sit at the bottom; the fields take the rest, so a
    // warning that wraps never pushes the button out of the panel.
    egui::Panel::bottom("commit-footer")
        .frame(egui::Frame::new())
        .show(ui, |ui| footer(ui, actions));
    egui::ScrollArea::vertical()
        .id_salt("commit-fields")
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            paint::seam_h(
                ui.painter(),
                ui.max_rect().x_range(),
                ui.max_rect().min.y - 12.0,
                s.border,
            );
            composer_fields(ui, tab, actions);
        });
}

/// The commit button's label for a column `room` points wide, and whether its keys fit beside
/// it: the first of `labels` with its keys, else the first label that fits alone (the keys stay
/// in the tooltip), else the last, so the button never cuts its text at a large zoom.
fn fitting_label<'a>(ui: &Ui, labels: &'a [String], room: f32) -> (&'a str, bool) {
    let m = theme::metrics();
    let width = |text: &str, font: FontId| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    let font = FontId::new(m.body_size, theme::medium());
    let frame =
        2.0 * m.button_pad_x + width(icon::CHECK, FontId::proportional(m.icon_md)) + m.button_gap;
    let keys = theme::SM + width(&crate::keymap::hint(Command::Commit), font.clone());
    let fits = |label: &String, extra: f32| frame + width(label, font.clone()) + extra <= room;
    if let Some(full) = labels.first().filter(|l| fits(l, keys)) {
        return (full, true);
    }
    let label = labels
        .iter()
        .find(|l| fits(l, 0.0))
        .or_else(|| labels.last());
    (label.map_or("", String::as_str), false)
}

/// A clean tree with nothing typed: the composer shrinks to a line and the Amend choice, so an
/// empty form and a disabled button do not compete with the rest of the view.
fn composer_is_quiet(tab: &RepoTab, snap: &canopy_core::RepoSnapshot) -> bool {
    snap.state == canopy_core::RepoState::Clean
        && snap.status.is_clean()
        && !tab.amend
        && tab.commit_summary.is_empty()
        && tab.commit_body.is_empty()
}

fn quiet_composer(
    ui: &mut Ui,
    tab: &mut RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    // Nothing to commit yet, so a pending request to focus the summary would fire later,
    // when files appear.
    tab.focus_summary = false;
    let _ = SectionHeader::new("Commit").first(true).show(ui);
    ui.add_space(theme::XS);
    ui.label(
        RichText::new("Nothing to commit")
            .size(m.meta_size)
            .color(p.text_faint),
    );
    if snap.head.oid.is_some() {
        ui.add_space(theme::SM);
        Checkbox::new(&mut tab.amend, "Amend last commit")
            .tooltip("Replace the last commit with a new message")
            .show(ui);
        if tab.amend {
            actions.push(Action::PrefillAmend);
        }
    }
}

fn composer_fields(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let _ = SectionHeader::new("Commit").first(true).show(ui);
    ui.add_space(theme::XS);
    let summary = TextField::new(&mut tab.commit_summary)
        .placeholder("Summary (required)")
        .name("Commit summary")
        .counter(true)
        .tooltip("Keep the summary under 50 characters; 72 at most.")
        .id_salt("commit-summary")
        .show(ui);
    if tab.focus_summary {
        summary.request_focus();
        tab.focus_summary = false;
    }
    ui.add_space(6.0);
    let rows = if ui.ctx().content_rect().height() < 600.0 {
        2..=6
    } else {
        3..=12
    };
    TextArea::new(&mut tab.commit_body)
        .placeholder("Description")
        .name("Commit description")
        .rows(rows)
        .guide(72)
        .id_salt("commit-body")
        .show(ui);
    ui.add_space(theme::SM);
    let was = tab.amend;
    let clean = tab
        .snap
        .as_ref()
        .is_some_and(|snap| snap.state == canopy_core::RepoState::Clean);
    Checkbox::new(&mut tab.amend, "Amend last commit")
        .enabled(clean)
        .tooltip("Replace the last commit with the staged changes and this message")
        .disabled_reason("Finish or abort the operation in progress first")
        .show(ui);
    if tab.amend && !was && tab.commit_summary.is_empty() {
        actions.push(Action::PrefillAmend);
    }
    ui.add_space(theme::SM);
}

/// Why the commit button is disabled, when its label does not already say ("Stage files to
/// commit" is its own reason).
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "each flag is a separate reason the button can be disabled"
)]
fn commit_blocker(
    conflicted: bool,
    running: bool,
    has_content: bool,
    summary_empty: bool,
) -> Option<&'static str> {
    if conflicted {
        Some("Resolve the conflicts to commit")
    } else if running {
        Some("Another operation is still running")
    } else if has_content && summary_empty {
        Some("Write a summary to commit")
    } else {
        None
    }
}

/// The line above the commit button when Git's identity needs attention.
fn identity_notice(identity: Option<&Identity>, read_failed: bool) -> Option<&'static str> {
    identity
        .and_then(Identity::warning)
        .or(read_failed.then_some("Could not read Git identity."))
}

#[cfg(test)]
mod tests {
    use canopy_core::identity::{Field, Person, Source};

    use super::*;

    fn identity(name: &str, email: &str, source: Option<&Source>, usable: bool) -> Identity {
        let field = |value: &str| Field {
            value: value.into(),
            source: source.cloned(),
        };
        let person = Person {
            name: field(name),
            email: field(email),
            usable,
        };
        Identity {
            author: person.clone(),
            committer: person,
        }
    }

    /// Mop-up D24: the titles ask a question naming what goes, the answers are verb + object.
    #[test]
    fn discarding_asks_about_the_files_by_name() {
        let confirm_of = |a: Action| match a {
            Action::Confirm { title, confirm, .. } => (title, confirm),
            other => panic!("not a confirmation: {other:?}"),
        };
        assert_eq!(
            confirm_of(discard_one(&entry("src/main.rs"))),
            (
                "Discard changes to src/main.rs?".to_owned(),
                "Discard changes".to_owned()
            )
        );
        assert_eq!(
            confirm_of(discard_all(&[entry("a"), entry("b")])),
            (
                "Discard changes to all 2 files?".to_owned(),
                "Discard all changes".to_owned()
            )
        );
        assert_eq!(
            confirm_of(discard_all(&[entry("a")])).0,
            "Discard changes to 1 file?"
        );
    }

    fn entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            old_path: None,
            kind: canopy_core::ChangeKind::Modified,
            additions: 1,
            deletions: 0,
            lfs: false,
            old_mode: None,
            new_mode: None,
        }
    }

    /// A tree with one unstaged file, so the composer is not in its quiet clean-tree form.
    fn unstaged_snapshot() -> canopy_core::RepoSnapshot {
        let mut snap = canopy_core::RepoSnapshot::default();
        snap.status.unstaged = vec![entry("src/main.rs")];
        snap
    }

    /// KP-F7: the file lists are resized with the kit's split handle. A width kept from an
    /// earlier run is the one shown, the handle sits on the lists' edge, moves from the keyboard
    /// and keeps the new width in the panel's persisted state.
    #[test]
    fn the_list_split_handle_keeps_a_saved_width_and_resizes_from_the_keyboard() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(
            dir.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        tab.snap = Some(std::sync::Arc::new(unstaged_snapshot()));
        let panel = egui::Id::new(LIST_PANEL);
        // Saved by an earlier run: 300 points wide.
        ctx.data_mut(|d| {
            d.insert_persisted(
                panel,
                egui::containers::panel::PanelState {
                    outer_rect: Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 600.0)),
                },
            );
        });
        let mut t = 0.0;
        let mut frame = |events: Vec<egui::Event>, tab: &mut RepoTab| {
            t += 0.05;
            let mut shown = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 700.0))),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    shown = show(ui, tab, &mut Vec::new())
                        .map(|(list, handle)| (list, ui.ctx().read_response(handle)));
                },
            );
            output.textures_delta.clear();
            shown.expect("the lists are shown")
        };
        let (list, handle) = frame(Vec::new(), &mut tab);
        assert!((list.width() - 300.0).abs() < 1.0, "{list:?}");
        let handle = handle.expect("the handle is shown");
        assert!((handle.rect.center().x - list.right()).abs() < 1.0);

        ctx.memory_mut(|m| m.request_focus(handle.id));
        let right = vec![egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        frame(right, &mut tab);
        let (list, _) = frame(Vec::new(), &mut tab);
        let wanted = 300.0 + split::KEY_STEP;
        assert!((list.width() - wanted).abs() < 1.0, "{list:?}");
        let saved = egui::containers::panel::PanelState::load(&ctx, panel).expect("saved");
        assert!((saved.size().x - wanted).abs() < 1.0, "{saved:?}");
    }

    #[test]
    fn empty_sections_are_left_out_while_another_has_content() {
        let mut st = canopy_core::WorkingStatus::default();
        assert_eq!(
            visible_lists(&st),
            (false, true, false),
            "nothing: one empty list stays"
        );
        st.unstaged = vec![entry("a")];
        assert_eq!(
            visible_lists(&st),
            (false, true, true),
            "unstaged only: staged says so"
        );
        st.staged = vec![entry("b")];
        assert_eq!(visible_lists(&st), (false, true, true));
        st.unstaged.clear();
        assert_eq!(
            visible_lists(&st),
            (false, false, true),
            "staged only: no Unstaged 0"
        );
        st.conflicted = vec![entry("c")];
        st.staged.clear();
        assert_eq!(
            visible_lists(&st),
            (true, false, false),
            "a conflict hides the empties"
        );
    }

    /// Frames of `show` at 960x600 with the pointer at `pointer`; returns every painted text.
    fn painted_texts(
        ctx: &egui::Context,
        tab: &mut RepoTab,
        pointer: Option<egui::Pos2>,
    ) -> Vec<(String, Rect)> {
        let mut texts = Vec::new();
        for _ in 0..3 {
            let events = pointer.map(egui::Event::PointerMoved).into_iter().collect();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    show(ui, tab, &mut Vec::new());
                },
            );
            output.textures_delta.clear();
            texts = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.visual_bounding_rect()))
                    }
                    _ => None,
                })
                .collect();
        }
        texts
    }

    #[test]
    fn every_row_offers_its_action_under_the_pointer() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(
            dir.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        let mut snap = unstaged_snapshot();
        snap.status.unstaged.push(entry("src/lib.rs"));
        snap.status.staged = vec![entry("README.md")];
        tab.snap = Some(std::sync::Arc::new(snap));
        // The first unstaged row is selected by default and shows its action; the others
        // show theirs only under the pointer.
        let rest = painted_texts(&ctx, &mut tab, None);
        // Row actions only: the list headers' "Stage all" and "Unstage all" buttons draw the
        // same glyphs as their icons, after a label rather than a file name, and the diff's
        // file header beside the list has its own "Stage file" button after the file name.
        let list_right = rest
            .iter()
            .find(|(t, _)| t == "Stage all")
            .map(|(_, r)| r.max.x + theme::LG)
            .expect("painted list header");
        let shown = |texts: &[(String, Rect)], glyph: &str| {
            texts
                .iter()
                .filter(|(t, r)| {
                    t == glyph
                        && r.max.x <= list_right
                        && texts.iter().any(|(name, at)| {
                            name.contains('.')
                                && at.max.x < r.min.x
                                && (at.center().y - r.center().y).abs() < 4.0
                        })
                })
                .count()
        };
        assert_eq!(shown(&rest, icon::PLUS), 1, "only the selected row");
        assert_eq!(shown(&rest, icon::MINUS), 0);
        for (name, glyph) in [("lib.rs", icon::PLUS), ("README.md", icon::MINUS)] {
            let (_, at) = rest
                .iter()
                .find(|(t, _)| t == name)
                .expect("painted row name");
            let pointer = pos2(at.min.x + 20.0, at.center().y);
            let hovered = painted_texts(&ctx, &mut tab, Some(pointer));
            assert!(
                hovered
                    .iter()
                    .any(|(t, r)| t == glyph && (r.center().y - at.center().y).abs() < 4.0),
                "hovering {name} must show {glyph} on its row"
            );
            painted_texts(&ctx, &mut tab, Some(pos2(5.0, 595.0)));
        }
    }

    #[test]
    fn compact_clean_summary_keeps_publish_reachable_by_scrolling() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let tab = RepoTab::with_worker(
            "fixture".into(),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let snap = canopy_core::RepoSnapshot::default();
        let mut publish_visible = false;
        for frame in 0..40 {
            let mut events = vec![egui::Event::PointerMoved(pos2(180.0, 100.0))];
            if frame > 5 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -30.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 200.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| {
                            clean_home(ui, &tab, &snap, &mut Vec::new());
                        });
                },
            );
            output.textures_delta.clear();
            publish_visible |= output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Publish…" => {
                    shape.clip_rect.contains_rect(text.visual_bounding_rect())
                }
                _ => false,
            });
        }
        assert!(
            publish_visible,
            "the clean-state primary action must remain reachable in a short pane"
        );
    }

    #[test]
    fn the_composer_follows_a_short_list_and_pins_below_a_long_one() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(
            dir.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        let summary_y = |tab: &mut RepoTab| {
            let texts = painted_texts(&ctx, tab, None);
            let (_, at) = texts
                .iter()
                .find(|(t, _)| t == "Summary (required)")
                .expect("painted summary hint");
            at.min.y
        };
        tab.snap = Some(std::sync::Arc::new(unstaged_snapshot()));
        assert!(
            summary_y(&mut tab) < 220.0,
            "one file: the composer sits under it"
        );

        let mut snap = unstaged_snapshot();
        snap.status.unstaged = (0..40).map(|i| entry(&format!("src/f{i}.rs"))).collect();
        tab.snap = Some(std::sync::Arc::new(snap));
        assert!(
            summary_y(&mut tab) > 300.0,
            "forty files: the composer pins to the bottom"
        );
    }

    /// A11Y-19: a Commit button that cannot commit yet is disabled for a screen reader too, and
    /// says so in its name (the Linux adapter reports every button as enabled); its reason is its
    /// description.
    #[test]
    fn a_commit_that_cannot_run_is_disabled_for_screen_readers() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx.enable_accesskit();
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let mut snap = unstaged_snapshot();
        snap.status.staged = vec![entry("src/lib.rs")];
        snap.head.branch = Some("main".into());
        let mut commit = None;
        for _ in 0..3 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 700.0))),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        composer(ui, &mut tab, &snap, &mut Vec::new(), true);
                    });
                },
            );
            out.textures_delta.clear();
            commit = out.platform_output.accesskit_update.and_then(|u| {
                u.nodes
                    .into_iter()
                    .map(|(_, n)| n)
                    .find(|n| n.label().is_some_and(|l| l.starts_with("Commit 1 file")))
            });
        }
        let commit = commit.expect("the Commit button");
        assert!(commit.is_disabled(), "no summary yet: {commit:?}");
        assert_eq!(commit.label(), Some("Commit 1 file to main, unavailable"));
        assert_eq!(commit.description(), Some("Write a summary to commit"));
        assert!(
            !commit.supports_action(egui::accesskit::Action::Click),
            "{commit:?}"
        );
    }

    #[test]
    fn a_disabled_commit_button_says_why() {
        assert_eq!(
            commit_blocker(true, false, true, false),
            Some("Resolve the conflicts to commit")
        );
        assert_eq!(
            commit_blocker(false, true, true, false),
            Some("Another operation is still running")
        );
        assert_eq!(
            commit_blocker(false, false, true, true),
            Some("Write a summary to commit")
        );
        assert_eq!(
            commit_blocker(false, false, false, true),
            None,
            "the label asks for staged files"
        );
    }

    #[test]
    fn the_commit_panel_warns_only_when_the_identity_needs_attention() {
        let configured = Some(Source::Config {
            scope: "global".into(),
            origin: "file:/home/ada/.gitconfig".into(),
        });
        let good = identity("Ada Lovelace", "ada@example.com", configured.as_ref(), true);
        assert_eq!(identity_notice(Some(&good), false), None);
        assert_eq!(
            identity_notice(None, false),
            None,
            "still reading: no warning yet"
        );

        let placeholder = identity("t", "t@t", configured.as_ref(), true);
        assert_eq!(
            identity_notice(Some(&placeholder), false),
            Some("This identity looks like a placeholder.")
        );
        let guessed = identity("ada", "ada@host.example", Some(&Source::Guessed), true);
        assert_eq!(
            identity_notice(Some(&guessed), false),
            Some("Git guessed part of this identity from the computer.")
        );
        let unusable = identity("", "", None, false);
        assert_eq!(
            identity_notice(Some(&unusable), false),
            Some("Git cannot use this identity for a commit.")
        );
        assert_eq!(
            identity_notice(None, true),
            Some("Could not read Git identity.")
        );
    }

    #[test]
    fn wrapped_identity_notice_does_not_overlap_commit_fields() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(
            dir.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        let unusable = identity("", "", None, false);
        let warning = unusable.warning().expect("identity warning");
        tab.identity = Some(unusable);
        tab.snap = Some(std::sync::Arc::new(unstaged_snapshot()));
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(562.5, 350.0))),
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::top("test-actions")
                        .exact_size(84.0)
                        .show(ui, |_| {});
                    egui::Panel::bottom("test-status")
                        .exact_size(26.0)
                        .show(ui, |_| {});
                    show(ui, &mut tab, &mut Vec::new());
                },
            );
            output.textures_delta.clear();
            let warning_rect = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text().contains(warning)
                    {
                        Some(text.visual_bounding_rect())
                    } else {
                        None
                    }
                })
                .expect("painted identity notice");
            let summary = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text().contains("Summary (required)")
                    {
                        Some(text.visual_bounding_rect().intersect(shape.clip_rect))
                    } else {
                        None
                    }
                })
                .expect("painted summary hint");
            assert!(
                !summary.is_positive() || !warning_rect.intersects(summary),
                "identity notice {warning_rect:?} overlaps painted summary {summary:?}",
            );
        }
    }

    /// QVIS-5: in a narrow column (a small window at a large zoom) the commit button never
    /// clips its label or keys: it drops the keys, then the branch, then the count, and the
    /// screen reader still hears the whole label.
    #[test]
    fn the_commit_button_fits_its_label_in_a_narrow_column() {
        let keys = crate::keymap::hint(Command::Commit);
        for width in [180.0, 240.0, 300.0, 420.0] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
            ctx.enable_accesskit();
            let (worker, _, _) = crate::worker::Worker::detached();
            let mut tab = RepoTab::with_worker(
                std::path::PathBuf::from("fixture"),
                100,
                crate::settings::DiffPrefs::default(),
                worker,
            );
            tab.commit_summary = "Add metrics".into();
            let mut snap = unstaged_snapshot();
            snap.status.staged = vec![entry("src/lib.rs"), entry("src/main.rs")];
            snap.head.branch = Some("feature/dark-mode".into());
            tab.snap = Some(std::sync::Arc::new(snap.clone()));
            let mut texts = Vec::new();
            let mut spoken = None;
            for _ in 0..3 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(width, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            composer(ui, &mut tab, &snap, &mut Vec::new(), true);
                        });
                    },
                );
                out.textures_delta.clear();
                texts = out
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text)
                            if text.galley.text().starts_with("Commit")
                                || text.galley.text() == keys =>
                        {
                            Some((
                                text.galley.text().to_owned(),
                                shape.clip_rect,
                                text.visual_bounding_rect(),
                            ))
                        }
                        _ => None,
                    })
                    .collect();
                spoken = out.platform_output.accesskit_update.and_then(|u| {
                    u.nodes
                        .into_iter()
                        .map(|(_, n)| n)
                        .find(|n| {
                            n.role() == egui::accesskit::Role::Button
                                && n.label().is_some_and(|l| l.starts_with("Commit"))
                        })
                        .and_then(|n| n.label().map(str::to_owned))
                });
            }
            let label = texts
                .iter()
                .find(|(t, _, _)| t.starts_with("Commit"))
                .unwrap_or_else(|| panic!("no commit label at {width}: {texts:?}"));
            for (text, clip, bounds) in &texts {
                assert!(
                    bounds.right() <= clip.right().min(width) + 0.5,
                    "{text:?} at {bounds:?} is cut by {clip:?} in a {width}-point column"
                );
            }
            assert_eq!(
                spoken.as_deref(),
                Some("Commit 2 files to feature/dark-mode"),
                "at {width}, painted {:?}",
                label.0
            );
            if width >= 420.0 {
                assert_eq!(label.0, "Commit 2 files to feature/dark-mode");
                assert!(texts.iter().any(|(t, _, _)| *t == keys), "{texts:?}");
            }
        }
    }

    #[test]
    fn commit_button_is_visible_in_default_desktop_layouts() {
        for size in [vec2(600.0, 375.0), vec2(960.0, 600.0), vec2(1440.0, 900.0)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
            let dir = tempfile::tempdir().expect("temporary tab");
            let mut tab = RepoTab::new(
                dir.path().into(),
                &ctx,
                100,
                crate::settings::DiffPrefs::default(),
            );
            tab.snap = Some(std::sync::Arc::new(unstaged_snapshot()));
            let mut visible = false;
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::top("test-actions")
                            .exact_size(96.0)
                            .show(ui, |_| {});
                        egui::Panel::bottom("test-status")
                            .exact_size(26.0)
                            .show(ui, |_| {});
                        show(ui, &mut tab, &mut Vec::new());
                    },
                );
                output.textures_delta.clear();
                visible = output.shapes.iter().any(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == "Stage files to commit"
                    {
                        return shape.clip_rect.contains_rect(text.visual_bounding_rect());
                    }
                    false
                });
            }
            assert!(visible, "Commit button must be visible at {size:?}");
        }
    }

    /// One frame at time `t`: each text painted and the top of its galley.
    fn timed_texts(ctx: &egui::Context, tab: &mut RepoTab, t: f64) -> Vec<(String, f32)> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0))),
                time: Some(t),
                ..Default::default()
            },
            |ui| {
                show(ui, tab, &mut Vec::new());
            },
        );
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                // Only what shows: a sliding row's text inside its (moving) clip.
                egui::Shape::Text(text)
                    if shape.clip_rect.intersects(text.visual_bounding_rect()) =>
                {
                    Some((text.galley.text().to_owned(), text.pos.y))
                }
                _ => None,
            })
            .collect()
    }

    fn painted_at(texts: &[(String, f32)], text: &str) -> Option<f32> {
        texts.iter().find(|(t, _)| t == text).map(|(_, y)| *y)
    }

    /// O1 (owner): staging a file moves its row from Unstaged to Staged by sliding it there, the
    /// rows below close up smoothly and the composer under the lists follows without a jump;
    /// with reduced motion everything lands at once.
    #[test]
    fn staging_a_file_slides_its_row_to_the_staged_list() {
        for reduced in [false, true] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    reduce_motion: reduced,
                    ..theme::Appearance::STILL
                },
            );
            let dir = tempfile::tempdir().expect("temporary tab");
            let mut tab = RepoTab::new(
                dir.path().into(),
                &ctx,
                100,
                crate::settings::DiffPrefs::default(),
            );
            let mut snap = canopy_core::RepoSnapshot::default();
            snap.status.unstaged = vec![entry("a.rs"), entry("b.rs"), entry("c.rs")];
            tab.snap = Some(std::sync::Arc::new(snap.clone()));
            let mut before = Vec::new();
            for i in 0..3 {
                before = timed_texts(&ctx, &mut tab, f64::from(i) * 0.1);
            }
            let (b0, c0) = (painted_at(&before, "b.rs"), painted_at(&before, "c.rs"));
            let composer0 = painted_at(&before, "Summary (required)");
            // b.rs is staged: the worker sends the new status.
            snap.status.unstaged = vec![entry("a.rs"), entry("c.rs")];
            snap.status.staged = vec![entry("b.rs")];
            tab.snap = Some(std::sync::Arc::new(snap));
            tab.generation += 1;
            let change = timed_texts(&ctx, &mut tab, 1.0);
            let mid = timed_texts(&ctx, &mut tab, 1.08);
            let mut rest = Vec::new();
            for i in 0..3 {
                rest = timed_texts(&ctx, &mut tab, 2.0 + f64::from(i) * 0.1);
            }
            let (b1, c1) = (painted_at(&rest, "b.rs"), painted_at(&rest, "c.rs"));
            let composer1 = painted_at(&rest, "Summary (required)");
            let (b0, b1, c0, c1) = (
                b0.expect("b.rs before"),
                b1.expect("b.rs after"),
                c0.expect("c.rs before"),
                c1.expect("c.rs after"),
            );
            assert!(b1 > b0 && c1 < c0, "b.rs moves down to Staged, c.rs up");
            let at = |texts: &[(String, f32)], text: &str| {
                painted_at(texts, text).unwrap_or_else(|| panic!("{text} painted"))
            };
            if reduced {
                assert_eq!(at(&change, "b.rs"), b1, "reduced: b.rs lands at once");
                assert_eq!(at(&change, "c.rs"), c1);
                continue;
            }
            assert_eq!(at(&change, "b.rs"), b0, "b.rs starts where it was");
            assert_eq!(at(&change, "c.rs"), c0, "c.rs starts where it was");
            assert_eq!(
                painted_at(&change, "Summary (required)"),
                composer0,
                "the composer does not jump"
            );
            let b = at(&mid, "b.rs");
            assert!(b > b0 && b < b1, "b.rs slides: {b0} -> {b} -> {b1}");
            let c = at(&mid, "c.rs");
            assert!(c < c0 && c > c1, "c.rs slides: {c0} -> {c} -> {c1}");
            assert!(composer1.is_some(), "the composer is there at rest");
        }
    }
}
