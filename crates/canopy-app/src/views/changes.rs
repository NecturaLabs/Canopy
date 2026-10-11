//! The staging area: conflicted, unstaged and staged files, the commit composer and the diff.

use egui::{Align, FontId, Layout, Rect, Response, RichText, Ui, UiBuilder, pos2, vec2};
use egui_phosphor::regular as icon;

use canopy_core::identity::Identity;
use canopy_core::{DiffSource, FileEntry};

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::{FileSelection, RepoTab};
use crate::ui::art;
use crate::ui::kit::anim::Flip;
use crate::ui::kit::chip::CountBadge;
use crate::ui::kit::field::{Join, TextArea, TextField};
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{RowKind, SectionHeader, Selection, caps_galley};
use crate::ui::kit::split::{self, SplitAxis, SplitHandle};
use crate::ui::kit::surface::{EmptyAction, EmptyState};
use crate::ui::kit::toggle::Checkbox;
use crate::ui::kit::{Button, IconButton, Size, Tone, menu, paint};
use crate::ui::theme::{self, palette};
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

    if snap.status_unavailable.is_none()
        && snap.state == canopy_core::RepoState::Clean
        && snap.status.is_clean()
        && !tab.amend
    {
        tab.focus_summary = false;
        tab.blur_composer = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(s.pane))
            .show(ui, |ui| clean_home(ui, tab, &snap, actions));
        return None;
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
            let seam = paint::pane_seam(palette(ui.ctx()), s.border);
            let w = seam.width;
            paint::seam_line_v(ui.painter(), edge.right() - w * 0.5, edge.y_range(), seam);
            let list_frame = egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 8));
            // The composer follows the lists while both fit; once the lists would crowd it, it
            // pins to the bottom and the lists scroll above it.
            let gap = if snap.status.is_clean() {
                0.0
            } else {
                COMPOSER_GAP
            };
            let need = lists_height(&snap.status) + gap + composer_height(ui, tab);
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
                                    if !snap.status.is_clean() {
                                        ui.add_space(COMPOSER_GAP / 2.0);
                                        let rule = ui.max_rect().x_range();
                                        paint::seam_h(
                                            ui.painter(),
                                            rule,
                                            ui.cursor().min.y,
                                            s.border,
                                        );
                                        ui.add_space(COMPOSER_GAP / 2.0);
                                    }
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::symmetric(4, 0))
                                        .show(ui, |ui| composer(ui, tab, &snap, actions, true));
                                });
                                flip.end(ui, |key| listed(&snap.status, key));
                            });
                    });
                return;
            }
            // Short windows show a two-row description (`composer_fields`), so the composer
            // needs less room and the file lists keep it.
            let cap = if ui.ctx().content_rect().height() < 600.0 {
                260.0
            } else {
                300.0
            };
            let (default, min, max) =
                pinned_composer_sizes(ui.available_height(), lists_height(&snap.status), cap);
            egui::Panel::bottom("composer")
                .default_size(default)
                .min_size(min)
                .max_size(max)
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

/// The smallest a pinned composer gets: the commit button, its hint and a little of the fields,
/// which scroll.
const PINNED_COMPOSER_MIN: f32 = 150.0;
/// The share of the column the file lists keep, at most, when the composer is pinned below them.
const LISTS_SHARE: f32 = 0.6;

/// The default, smallest and largest height of the composer pinned below the lists, in a column
/// `available` points tall whose lists would take `lists` to show whole: the lists keep what
/// they need up to [`LISTS_SHARE`] of the column, so every section shows its first files, and
/// the composer takes the rest up to `cap`, its fields scrolling when it is squeezed.
fn pinned_composer_sizes(available: f32, lists: f32, cap: f32) -> (f32, f32, f32) {
    let kept = lists.min(available * LISTS_SHARE);
    let max = (available - kept).clamp(PINNED_COMPOSER_MIN, cap);
    let default = (available * 0.60).clamp(PINNED_COMPOSER_MIN, max);
    (default, max.min(250.0), max)
}

/// Gap between the last list and the composer when the composer follows the lists.
const COMPOSER_GAP: f32 = 28.0;
/// Gap between two file lists, and the height of an empty list's placeholder line.
const LIST_GAP: f32 = theme::MD;
const EMPTY_LIST: f32 = theme::XL;
/// The clean state’s primary action stays a proper button even with a short label.
const CTA_MIN_WIDTH: f32 = 120.0;

/// Height of the sections `lists` draws, close enough to decide whether the composer fits
/// below them. An overestimate only pins the composer; an underestimate scrolls.
fn lists_height(st: &canopy_core::WorkingStatus) -> f32 {
    if st.is_clean() {
        return 0.0;
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
fn composer_height(ui: &Ui, tab: &RepoTab) -> f32 {
    let short = ui.ctx().content_rect().height() < 600.0;
    let mut h = if short { 224.0 } else { 262.0 };
    if identity_notice(tab.identity.as_ref(), tab.identity_error.is_some()).is_some() {
        h += 56.0;
    }
    h += message_hints(tab)
        .iter()
        .map(|_| MESSAGE_HINT_HEIGHT)
        .sum::<f32>();
    h + 24.0
}

/// What one message hint takes in the composer: a small line and the gap under it. A hint
/// that wraps in a narrow column takes more; the composer scrolls, so none is cut off.
const MESSAGE_HINT_HEIGHT: f32 = 20.0;

/// The hints for the draft in the composer: none when the check is off. They only warn; the
/// commit button and the draft are never touched.
fn message_hints(tab: &RepoTab) -> Vec<String> {
    if !tab.check_message {
        return Vec::new();
    }
    canopy_core::message::check(&tab.commit_summary, &tab.commit_body)
        .iter()
        .map(canopy_core::message::Problem::describe)
        .collect()
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
    } else if !st.is_clean() {
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
        if !st.untracked_dirs.is_empty() {
            ui.add_space(LIST_GAP);
            untracked_folders(ui, &st.untracked_dirs, actions);
        }
    }
    ui.spacing_mut().item_spacing.y = spacing;
    ui.spacing_mut().interact_size.y = row_height;
}

/// The untracked folders the status lists as one entry each (a folder holding a repository of
/// its own, or any top-most folder when there are very many untracked files), with a Stage
/// action. A folder with a repository in it is staged only after a question: Git would record
/// the repository as a link nobody can clone.
fn untracked_folders(ui: &mut Ui, dirs: &[canopy_core::UntrackedDir], actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    list_header(ui, "Untracked folders", dirs.len(), Tone::Warning, |_| {});
    ui.add_space(theme::XS);
    for d in dirs {
        let id = ui.id().with(("untracked-folder", &d.path));
        let detail = if d.repository {
            "repository".to_owned()
        } else if d.files == 1 {
            "1 file".to_owned()
        } else {
            format!("{} files", d.files)
        };
        let tip = if d.repository {
            format!(
                "{} holds a Git repository of its own; Stage all leaves it out.",
                d.path
            )
        } else {
            d.path.clone()
        };
        let mut clicked = false;
        let row = crate::ui::kit::row::ListRow::new(id)
            .kind(RowKind::File)
            .tooltip(tip)
            .actions(|ui| {
                let r = IconButton::new(icon::PLUS, "Stage folder")
                    .size(Size::Sm)
                    .id_salt(("stage-folder", &d.path))
                    .show(ui);
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Stage folder {}", d.path),
                    )
                });
                clicked = r.clicked();
            })
            .show_with(ui, |rc, painter| {
                let cy = rc.rect.center().y;
                let glyph = painter.layout_no_wrap(
                    icon::FOLDER_SIMPLE.to_owned(),
                    FontId::proportional(m.body_size),
                    p.text_dim,
                );
                let glyph_width = glyph.size().x;
                painter.galley(
                    pos2(rc.content.left(), cy - glyph.size().y / 2.0),
                    glyph,
                    p.text_dim,
                );
                let side = painter.layout_no_wrap(
                    detail.clone(),
                    FontId::proportional(m.meta_size),
                    p.text_faint,
                );
                let right =
                    rc.content.right() - commit_detail::icon_actions_width() - side.size().x;
                painter.galley(pos2(right, cy - side.size().y / 2.0), side, p.text_faint);
                commit_detail::paint_line(
                    painter,
                    rc.content.left() + glyph_width + commit_detail::ROW_GAP,
                    right - commit_detail::ROW_GAP,
                    cy,
                    &d.path,
                    FontId::new(m.body_size, theme::medium()),
                    rc.text,
                );
            });
        ui.ctx().accesskit_node_builder(row.id, |n| {
            n.set_label(format!("{}, untracked folder, {detail}", d.path));
        });
        if clicked {
            actions.push(stage_folder(d));
        }
    }
}

/// Staging an untracked folder: at once, or after a question when a repository of its own is
/// in it.
fn stage_folder(d: &canopy_core::UntrackedDir) -> Action {
    let op = Op::Stage(vec![d.path.clone()]);
    if d.repository || d.contains_repository {
        Action::Confirm {
            title: format!("Stage {}?", d.path),
            body: if d.repository {
                format!(
                    "{} is a Git repository of its own. Staging it records a link to one commit, \
                     not its files, and nobody can clone that without a submodule entry. Add it as \
                     a submodule, or ignore it, instead.",
                    d.path
                )
            } else {
                format!(
                    "{} holds a Git repository of its own. Staging the folder records that \
                     repository as a link to one commit, which nobody can clone without a \
                     submodule entry.",
                    d.path
                )
            },
            confirm: "Stage anyway".into(),
            danger: false,
            op,
        }
    } else {
        Action::Op(op)
    }
}

/// Shared entry point from the Changes header and overflow menu.
fn ignore_rules(tab: &RepoTab) -> Action {
    Action::OpenDialog(crate::views::dialogs::Dialog::Ignore(Box::new(
        crate::views::dialogs::IgnoreForm::new(tab.path.clone(), None),
    )))
}

/// A clean tree's next sync action and a quiet entry into the amend composer.
fn clean_home(
    ui: &mut Ui,
    tab: &mut RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
) {
    let branch = snap
        .head
        .branch
        .as_ref()
        .and_then(|name| snap.branches.iter().find(|b| b.is_head && &b.name == name));
    let (status, label, action) = match (branch, snap.remotes.is_empty()) {
        (_, true) => (
            "Local only, no remote".to_owned(),
            "Publish…",
            Action::Publish,
        ),
        (Some(b), false) if b.upstream.is_some() => {
            let upstream = b.upstream.as_deref().unwrap_or_default();
            match (b.ahead, b.behind) {
                (0, 0) => (
                    format!("Up to date with {upstream}"),
                    "Fetch",
                    Action::Fetch,
                ),
                (a, 0) => (
                    format!(
                        "{a} commit{} to push to {upstream}",
                        if a == 1 { "" } else { "s" }
                    ),
                    "Push",
                    Action::Push,
                ),
                (0, n) => (
                    format!(
                        "{n} commit{} to pull from {upstream}",
                        if n == 1 { "" } else { "s" }
                    ),
                    "Pull",
                    Action::Pull,
                ),
                (a, n) => (
                    format!("{a} ahead, {n} behind {upstream}"),
                    "Pull",
                    Action::Pull,
                ),
            }
        }
        (Some(_), false) => ("Not pushed yet".to_owned(), "Push", Action::Push),
        (None, false) => (snap.head.label(), "Fetch", Action::Fetch),
    };
    let mut state = EmptyState::new("Working tree clean")
        .art(art::empty_clean())
        .icon(icon::CHECK_CIRCLE)
        .body(status)
        .action(label)
        .action_min_width(CTA_MIN_WIDTH)
        .id_salt("clean-working-tree");
    if snap.head.oid.is_some() && snap.state == canopy_core::RepoState::Clean && !tab.amend {
        state = state.link("Amend last commit");
    }
    let clicked = ui
        .add_enabled_ui(tab.running.is_none(), |ui| state.show(ui).value)
        .inner;
    match clicked {
        Some(EmptyAction::Primary) => actions.push(action),
        Some(EmptyAction::Link) => {
            tab.amend = true;
            if tab.message_is_unwritten() {
                actions.push(Action::PrefillAmend);
            }
        }
        None => {}
    }
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
/// section (aliased by its path, so it slides between sections), and the
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
        return false;
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

/// The list header's width from which "Stage selected" keeps its label beside "Stage all".
const STAGE_SELECTED_LABEL_WIDTH: f32 = 372.0;

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
    // Narrower columns drop the labels, Stage selected first: it sits beside Stage all, and both
    // keep their tooltips.
    let width = ui.available_width();
    let compact = width < 280.0;
    let labelled_selected = width >= STAGE_SELECTED_LABEL_WIDTH;
    // What "Stage selected" stages: the unstaged file open in the diff and the ones marked.
    let staging = tab.stage_targets();
    let stage_tip = match staging.len() {
        0 | 1 => "Stage the selected file".to_owned(),
        n => format!("Stage the {n} selected files"),
    };
    flip.item(ui, header_key(which), None, |ui| {
        list_header(ui, title, files.len(), tone, |ui| match which {
            List::Unstaged if !files.is_empty() => {
                // The header lays out right to left: Stage selected is added first, so it sits
                // to the right of Stage all.
                let selected = if labelled_selected {
                    Button::new("Stage selected")
                        .icon(icon::PLUS_CIRCLE)
                        .size(Size::Sm)
                        .tooltip(&stage_tip)
                        .shortcut(Command::ToggleStage)
                        .id_salt("stage-selected")
                        .enabled(!staging.is_empty())
                        .disabled_reason("Select an unstaged file to stage it")
                        .show(ui)
                } else {
                    IconButton::new(icon::PLUS_CIRCLE, &stage_tip)
                        .size(Size::Sm)
                        .shortcut(Command::ToggleStage)
                        .id_salt("stage-selected")
                        .enabled(!staging.is_empty())
                        .disabled_reason("Select an unstaged file to stage it")
                        .show(ui)
                };
                if !staging.is_empty() && selected.clicked() {
                    actions.push(Action::Op(Op::Stage(staging.clone())));
                }
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
                    if m.item(icon::EYE_SLASH, "Ignore rules…")
                        .show(m.ui)
                        .clicked()
                    {
                        actions.push(ignore_rules(tab));
                    }
                    m.separator();
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
        }) || (which == List::Unstaged && tab.marked.contains(&f.path));
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
        let missing = tab.not_downloaded.contains(&(source, f.path.clone()));
        let (resp, acted) = flip.item(ui, row_key(which, &f.path), Some(path_key(&f.path)), |ui| {
            file_row(
                ui,
                f,
                selected,
                missing,
                ("change", which as u8, &f.path),
                action,
            )
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
            let modifiers = ui.input(|i| i.modifiers);
            if which == List::Unstaged && (modifiers.command || modifiers.shift) {
                pick(tab, files, i, modifiers);
            } else {
                // A plain click is one file again, also when it is the one already open.
                tab.marked.clear();
                tab.select_file(FileSelection {
                    source,
                    entry: f.clone(),
                    conflicted,
                });
            }
        }
        let entry = f.clone();
        // Several files are selected and this is one of them: the menu stages them all.
        let group = Some(tab.stage_targets())
            .filter(|t| which == List::Unstaged && t.len() > 1 && t.contains(&f.path));
        menu::context(&resp, |m| {
            let first = match &group {
                Some(group) => m.item(glyph, format!("Stage {} files", group.len())),
                None => m.item(glyph, tip),
            };
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
                } else if let Some(group) = &group {
                    actions.push(Action::Op(Op::Stage(group.clone())));
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
            // A working-tree rename is in `HEAD` under its old name.
            if entry.kind != canopy_core::ChangeKind::Untracked
                && entry.kind != canopy_core::ChangeKind::Added
                && !conflicted
                && m.item(icon::CLOCK_COUNTER_CLOCKWISE, "File history…")
                    .show(m.ui)
                    .clicked()
            {
                actions.push(Action::FileHistory {
                    from: None,
                    path: entry.old_path.clone().unwrap_or_else(|| entry.path.clone()),
                });
            }
            m.separator();
            if m.item(icon::EYE_SLASH, "Ignore…").show(m.ui).clicked() {
                actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Ignore(
                    Box::new(crate::views::dialogs::IgnoreForm::new(
                        tab.path.clone(),
                        Some(entry.path.clone()),
                    )),
                )));
            }
            if m.item(icon::QUESTION, "Why ignored…").show(m.ui).clicked() {
                actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Ignore(
                    Box::new(
                        crate::views::dialogs::IgnoreForm::new(
                            tab.path.clone(),
                            Some(entry.path.clone()),
                        )
                        .why(),
                    ),
                )));
            }
            if entry.kind != canopy_core::ChangeKind::Untracked
                && !conflicted
                && m.item(icon::MINUS_CIRCLE, "Stop tracking…")
                    .danger()
                    .show(m.ui)
                    .clicked()
            {
                actions.push(Action::Confirm {
                    title: format!("Stop tracking {}?", entry.path),
                    body: "Files on disk are kept. Their removal from Git is staged for commit; collaborators will lose the tracked copy when they pull that commit. Add an ignore rule separately to keep it out of future commits.".into(),
                    confirm: "Stop tracking".into(), danger: true,
                    op: Op::StopTracking(entry.path.clone()),
                });
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

/// Ctrl-click or Shift-click on the unstaged file `index`: marks or unmarks it, or marks the
/// range from the open file ([`Selection::click`]). The open file stays open and always counts.
fn pick(tab: &mut RepoTab, files: &[FileEntry], index: usize, modifiers: egui::Modifiers) {
    let open = tab
        .file
        .as_ref()
        .filter(|s| s.source == DiffSource::Unstaged && !s.conflicted)
        .and_then(|s| files.iter().position(|f| f.path == s.entry.path));
    let mut selection = Selection {
        rows: files
            .iter()
            .enumerate()
            .filter(|(i, f)| Some(*i) == open || tab.marked.contains(&f.path))
            .map(|(i, _)| i)
            .collect(),
        anchor: open,
    };
    selection.click(index, modifiers);
    tab.marked = selection
        .rows
        .iter()
        .filter(|i| Some(**i) != open)
        .map(|&i| files[i].path.clone())
        .collect();
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
    not_downloaded: bool,
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
    let row = commit_detail::file_list_row_with(ui, f, selected, not_downloaded, id, width, |ui| {
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
    // Always-visible bars with their own room, so a handle never covers the fields' text.
    ui.spacing_mut().scroll = theme::solid_scroll();
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
    let tip = warning.map(|w| format!("{w} Open Settings → Git to correct it. Git's own error will appear if you commit without a usable identity."));
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
            .shortcut(Command::Commit)
            .show_shortcut(keys);
        if let Some(tip) = &tip {
            commit = commit.tooltip(tip.as_str());
        }
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

fn composer_fields(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let _ = SectionHeader::new("Commit").first(true).show(ui);
    ui.add_space(theme::XS);
    let summary = TextField::new(&mut tab.commit_summary)
        .placeholder("Summary")
        .name("Commit summary")
        .joined(Join::Above)
        .counter(true)
        .tooltip("Keep the summary under 50 characters; 72 at most.")
        .id_salt("commit-summary")
        .show(ui);
    if tab.focus_summary {
        summary.request_focus();
        tab.focus_summary = false;
    }
    // The description sits under the summary as the second half of one message box: its top edge
    // is the summary's bottom edge.
    ui.add_space(-ui.spacing().item_spacing.y);
    let rows = if ui.ctx().content_rect().height() < 600.0 {
        2..=6
    } else {
        3..=12
    };
    let body = TextArea::new(&mut tab.commit_body)
        .placeholder("Description (optional)")
        .name("Commit description")
        .joined(Join::Below)
        .rows(rows)
        .guide(72)
        .id_salt("commit-body")
        .show(ui);
    // A single-line field lets go of focus on Enter; pass it on rather than drop it, or the next
    // keys run as bare shortcuts. Ctrl+Enter is the commit chord and stays out of it.
    if summary.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.command)
    {
        body.response.request_focus();
    }
    if std::mem::take(&mut tab.blur_composer) {
        ui.memory_mut(|m| {
            m.surrender_focus(summary.id);
            m.surrender_focus(body.response.id);
        });
    }
    let hints = message_hints(tab);
    if !hints.is_empty() {
        let p = palette(ui.ctx());
        let m = theme::metrics();
        ui.add_space(theme::XS);
        for hint in &hints {
            commit_detail::without_icons(
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}  {hint}", icon::WARNING))
                            .color(p.warning)
                            .size(m.small_size),
                    )
                    .wrap(),
                ),
            );
        }
    }
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
    if tab.amend && !was && tab.message_is_unwritten() {
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
        // same glyphs as their icons, after a label rather than a file name.
        let list_right = rest
            .iter()
            .filter(|(t, _)| t == "Stage all" || t == "Stage selected" || t == icon::PLUS_CIRCLE)
            .map(|(_, r)| r.max.x + theme::LG)
            .reduce(f32::max)
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

    /// Frames of `show` at 960x600 with a click (press and release in one frame) at `at`;
    /// returns what the click asked for.
    fn click_at(ctx: &egui::Context, tab: &mut RepoTab, at: egui::Pos2) -> Vec<Action> {
        click_with(ctx, tab, at, egui::Modifiers::NONE)
    }

    /// [`click_at`] with `modifiers` held.
    fn click_with(
        ctx: &egui::Context,
        tab: &mut RepoTab,
        at: egui::Pos2,
        modifiers: egui::Modifiers,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        for events in [
            Vec::new(),
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                },
            ],
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    show(ui, tab, &mut actions);
                },
            );
            output.textures_delta.clear();
        }
        actions
    }

    #[test]
    fn ctrl_and_shift_click_pick_several_unstaged_files_for_stage_selected() {
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
        let mut snap = canopy_core::RepoSnapshot::default();
        snap.status.unstaged = ["a.rs", "b.rs", "c.rs", "d.rs"].map(entry).to_vec();
        tab.snap = Some(std::sync::Arc::new(snap));
        let texts = painted_texts(&ctx, &mut tab, None);
        let row = |name: &str| {
            texts.iter().find(|(t, _)| t == name).map_or_else(
                || panic!("row {name}"),
                |(_, r)| pos2(r.min.x + 4.0, r.center().y),
            )
        };
        let stage_selected = texts
            .iter()
            .find(|(t, _)| t == icon::PLUS_CIRCLE || t == "Stage selected")
            .map(|(_, r)| r.center())
            .expect("Stage selected");
        let staged = |actions: &[Action]| {
            actions.iter().find_map(|a| match a {
                Action::Op(Op::Stage(paths)) => Some(paths.clone()),
                _ => None,
            })
        };

        // a.rs is open; Ctrl-click adds d.rs to the selection.
        click_with(&ctx, &mut tab, row("d.rs"), egui::Modifiers::COMMAND);
        assert_eq!(tab.marked.iter().collect::<Vec<_>>(), ["d.rs"]);
        assert_eq!(
            tab.file.as_ref().map(|f| f.entry.path.as_str()),
            Some("a.rs"),
            "the open diff stays"
        );
        let actions = click_at(&ctx, &mut tab, stage_selected);
        assert_eq!(staged(&actions), Some(vec!["a.rs".into(), "d.rs".into()]));

        // Ctrl-click again takes it out.
        click_with(&ctx, &mut tab, row("d.rs"), egui::Modifiers::COMMAND);
        assert!(tab.marked.is_empty());

        // Shift-click picks the range from the open file.
        click_with(&ctx, &mut tab, row("c.rs"), egui::Modifiers::SHIFT);
        let actions = click_at(&ctx, &mut tab, stage_selected);
        assert_eq!(
            staged(&actions),
            Some(vec!["a.rs".into(), "b.rs".into(), "c.rs".into()])
        );

        // A plain click opens one file and drops the others.
        click_at(&ctx, &mut tab, row("b.rs"));
        assert!(tab.marked.is_empty());
        let actions = click_at(&ctx, &mut tab, stage_selected);
        assert_eq!(staged(&actions), Some(vec!["b.rs".into()]));

        // ...also when it is the file that is already open.
        click_with(&ctx, &mut tab, row("d.rs"), egui::Modifiers::COMMAND);
        assert_eq!(tab.marked.iter().collect::<Vec<_>>(), ["d.rs"]);
        click_at(&ctx, &mut tab, row("b.rs"));
        assert!(tab.marked.is_empty(), "{:?}", tab.marked);
    }

    #[test]
    fn stage_selected_sits_right_of_stage_all_and_stages_the_selected_file() {
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
        snap.status.staged = vec![entry("README.md")];
        tab.snap = Some(std::sync::Arc::new(snap));
        let texts = painted_texts(&ctx, &mut tab, None);
        let find = |label: &str| texts.iter().find(|(t, _)| t == label).map(|(_, r)| *r);
        let all = find("Stage all").expect("Stage all");
        // This column is too narrow for the label, so the button is its icon.
        let selected = find(icon::PLUS_CIRCLE).expect("Stage selected");
        assert!(
            selected.min.x > all.max.x && (selected.center().y - all.center().y).abs() < 4.0,
            "Stage selected follows Stage all on its row: {selected:?} after {all:?}"
        );
        assert!(
            find("Stage file").is_none(),
            "the diff header no longer carries Stage file"
        );
        let staged = tab
            .file
            .clone()
            .expect("the first unstaged file is selected");
        let actions = click_at(&ctx, &mut tab, selected.center());
        assert!(
            actions.iter().any(|a| matches!(
                a,
                Action::Op(Op::Stage(paths)) if paths == std::slice::from_ref(&staged.entry.path)
            )),
            "Stage selected stages the selected file: {actions:?}"
        );
        // With a staged file selected there is nothing to stage, and the button says so.
        tab.select_file(FileSelection {
            source: DiffSource::Staged,
            entry: entry("README.md"),
            conflicted: false,
        });
        painted_texts(&ctx, &mut tab, None);
        let actions = click_at(&ctx, &mut tab, selected.center());
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::Op(Op::Stage(_)))),
            "Stage selected is disabled for a staged file: {actions:?}"
        );
    }

    #[test]
    fn a_short_column_leaves_the_lists_room_for_every_section() {
        // 960x600: about 470 points of column, lists of 250 for three unstaged and one staged file.
        let (default, min, max) = pinned_composer_sizes(470.0, 250.0, 260.0);
        assert!(max <= 470.0 - 250.0, "the lists keep 250: {max}");
        assert!(min <= default && default <= max, "{min} {default} {max}");
        // Very many files: the lists keep their share and the composer stays usable.
        let (_, _, max) = pinned_composer_sizes(470.0, 5000.0, 260.0);
        assert!((PINNED_COMPOSER_MIN..=260.0).contains(&max), "{max}");
        // A tall window is unchanged: the composer takes its cap.
        let (default, min, max) = pinned_composer_sizes(970.0, 5000.0, 300.0);
        assert_eq!((min, max), (250.0, 300.0));
        assert!(default <= max);
    }

    #[test]
    fn a_clean_tree_has_one_state_and_keyboard_amend() {
        let ctx = crate::ui::kit::testkit::ctx();
        ctx.enable_accesskit();
        let (worker, events, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            "fixture".into(),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let mut snap = canopy_core::RepoSnapshot::default();
        snap.head.oid = Some(canopy_core::Oid::from_bytes(&[1; 20]).expect("head"));
        tab.snap = Some(std::sync::Arc::new(snap));
        let mut actions = Vec::new();
        let mut draw = |keys: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 400.0))),
                    events: keys,
                    ..Default::default()
                },
                |ui| {
                    let split = show(ui, &mut tab, &mut actions);
                    assert!(
                        split.is_none(),
                        "a clean tree has no empty file-list column"
                    );
                },
            );
            out.textures_delta.clear();
            let text = out
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (
                out.platform_output
                    .accesskit_update
                    .expect("accessible tree"),
                text,
            )
        };
        let (mut update, mut text) = draw(Vec::new());
        for _ in 0..3 {
            (update, text) = draw(Vec::new());
        }
        assert!(
            text.iter().any(|text| text == "Working tree clean"),
            "{text:?}"
        );
        assert!(
            !update
                .nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Commit summary"))
        );
        let amend = update
            .nodes
            .iter()
            .filter(|(_, n)| n.label() == Some("Amend last commit"))
            .collect::<Vec<_>>();
        assert_eq!(amend.len(), 1);
        assert_eq!(amend[0].1.role(), egui::accesskit::Role::Link);
        let mut reached = false;
        for _ in 0..10 {
            let (update, _) = draw(crate::ui::kit::testkit::key(egui::Key::Tab));
            if update
                .nodes
                .iter()
                .any(|(id, n)| *id == update.focus && n.label() == Some("Amend last commit"))
            {
                draw(crate::ui::kit::testkit::key(egui::Key::Enter));
                reached = true;
                break;
            }
        }
        assert!(reached, "amend is keyboard reachable");
        assert!(tab.amend);
        assert!(actions.iter().any(|a| matches!(a, Action::PrefillAmend)));
        events
            .send(crate::worker::Event::HeadMessage(
                "Last message\n\nLast description".into(),
            ))
            .expect("message");
        tab.poll(&[]);
        assert_eq!(tab.commit_summary, "Last message");
        assert_eq!(tab.commit_body, "Last description");
        let nodes = crate::ui::kit::testkit::spoken(&ctx, |ui| {
            show(ui, &mut tab, &mut actions);
        });
        assert!(
            nodes
                .iter()
                .any(|n| n.label.as_deref() == Some("Commit summary"))
        );
    }

    #[test]
    fn clean_branches_offer_the_next_sync_action() {
        for (ahead, behind, expected) in [
            (0, 0, "Fetch"),
            (2, 0, "Push"),
            (0, 2, "Pull"),
            (2, 2, "Pull"),
        ] {
            let ctx = crate::ui::kit::testkit::ctx();
            ctx.enable_accesskit();
            let (worker, _, _) = crate::worker::Worker::detached();
            let mut tab = RepoTab::with_worker(
                "fixture".into(),
                100,
                crate::settings::DiffPrefs::default(),
                worker,
            );
            let mut snap = canopy_core::RepoSnapshot::default();
            snap.head.branch = Some("main".into());
            snap.branches.push(canopy_core::BranchInfo {
                name: "main".into(),
                target: None,
                is_head: true,
                upstream: Some("origin/main".into()),
                ahead,
                behind,
                worktree: None,
            });
            snap.remotes.push(canopy_core::RemoteInfo {
                name: "origin".into(),
                url: None,
                default_branch: Some("main".into()),
            });
            tab.snap = Some(std::sync::Arc::new(snap));
            let mut actions = Vec::new();
            let mut reached = false;
            for _ in 0..10 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        events: crate::ui::kit::testkit::key(egui::Key::Tab),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui, &mut tab, &mut actions);
                    },
                );
                out.textures_delta.clear();
                let update = out.platform_output.accesskit_update.expect("tree");
                if update
                    .nodes
                    .iter()
                    .any(|(id, n)| *id == update.focus && n.label() == Some(expected))
                {
                    crate::ui::kit::testkit::frame(
                        &ctx,
                        crate::ui::kit::testkit::key(egui::Key::Enter),
                        |ui| {
                            show(ui, &mut tab, &mut actions);
                        },
                    );
                    reached = true;
                    break;
                }
            }
            assert!(
                reached,
                "{ahead}/{behind}: {expected} is keyboard reachable"
            );
            assert!(actions.iter().any(|a| matches!(
                (expected, a),
                ("Fetch", Action::Fetch) | ("Push", Action::Push) | ("Pull", Action::Pull)
            )));
        }
    }

    #[test]
    fn compact_clean_summary_keeps_publish_reachable_by_scrolling() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
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
                            clean_home(ui, &mut tab, &snap, &mut Vec::new());
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

    /// The commit fields' scroll bar is the kit's solid bar (a thin handle at 45% of the text
    /// colour), never egui's 8-point slab in the full text colour, which read as a black bar in
    /// Daylight beside a short composer.
    #[test]
    fn the_commit_fields_scroll_bar_is_a_thin_handle() {
        for id in [theme::ThemeId::Daylight, theme::ThemeId::Midnight] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, id, theme::Appearance::STILL);
            let text = palette(&ctx).text;
            let (worker, _, _) = crate::worker::Worker::detached();
            let mut tab = RepoTab::with_worker(
                std::path::PathBuf::from("fixture"),
                100,
                crate::settings::DiffPrefs::default(),
                worker,
            );
            tab.commit_summary = "Add metrics".into();
            tab.commit_body = "A body long enough to scroll.\n".repeat(30);
            let mut snap = unstaged_snapshot();
            snap.status.staged = vec![entry("src/lib.rs")];
            let mut handles = Vec::new();
            for _ in 0..3 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(340.0, 260.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            composer(ui, &mut tab, &snap, &mut Vec::new(), false);
                        });
                    },
                );
                out.textures_delta.clear();
                handles = out
                    .shapes
                    .iter()
                    .filter_map(|c| match &c.shape {
                        egui::Shape::Rect(r)
                            if r.rect.height() > 20.0 && r.rect.width() <= 10.0 =>
                        {
                            Some((r.rect, r.fill))
                        }
                        _ => None,
                    })
                    .collect();
            }
            assert!(!handles.is_empty(), "{id:?}: the fields do not scroll");
            for (rect, fill) in handles {
                assert!(
                    rect.width() <= 4.5 && fill.a() < text.a(),
                    "{id:?}: a {}-point handle in {fill:?} (text {text:?})",
                    rect.width()
                );
            }
        }
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
                .find(|(t, _)| t == "Summary")
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

    /// #43: an untracked folder the status lists as one entry is on the list, named, with a
    /// Stage action; one with a repository in it is staged only after the question.
    #[test]
    fn collapsed_untracked_folders_are_listed_and_staged_after_a_question_when_they_hold_a_repository()
     {
        use canopy_core::UntrackedDir;
        let dir = |path: &str, files, repository, contains_repository| UntrackedDir {
            path: path.into(),
            files,
            repository,
            contains_repository,
        };
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let folders = vec![
            dir("vendor/", 0, true, false),
            dir("build/", 120, false, false),
        ];
        let mut actions = Vec::new();
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes = crate::ui::kit::testkit::spoken(&ctx, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    untracked_folders(ui, &folders, &mut actions);
                });
            });
        }
        let named = |what: &str| {
            nodes
                .iter()
                .any(|n| n.label.as_deref().is_some_and(|l| l.contains(what)))
        };
        assert!(named("vendor/, untracked folder, repository"));
        assert!(named("build/, untracked folder, 120 files"));
        assert!(actions.is_empty(), "listing stages nothing");

        assert!(matches!(
            stage_folder(&folders[0]),
            Action::Confirm { confirm, op: Op::Stage(p), .. }
                if confirm == "Stage anyway" && p == ["vendor/"]
        ));
        assert!(matches!(
            stage_folder(&dir("tools/", 3, false, true)),
            Action::Confirm { .. }
        ));
        assert!(matches!(
            stage_folder(&folders[1]),
            Action::Op(Op::Stage(p)) if p == ["build/"]
        ));
    }

    /// #63: every control the Changes view draws (the lists, their buttons and the commit
    /// composer) is named for a screen reader, and the composer's message is a text field.
    #[test]
    fn every_control_in_the_changes_view_has_a_name() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(
            dir.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        tab.identity = Some(identity("Ada", "ada@example.com", None, true));
        let mut snap = unstaged_snapshot();
        snap.status.staged = vec![entry("README.md")];
        tab.snap = Some(std::sync::Arc::new(snap));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes = crate::ui::kit::testkit::spoken(&ctx, |ui| {
                egui::Panel::top("test-actions")
                    .exact_size(84.0)
                    .show(ui, |_| {});
                show(ui, &mut tab, &mut Vec::new());
            });
        }
        let controls = [
            Role::Button,
            Role::CheckBox,
            Role::Link,
            Role::MenuItem,
            Role::RadioButton,
            Role::Switch,
            Role::Tab,
            Role::TextInput,
            Role::MultilineTextInput,
        ];
        let unnamed: Vec<_> = nodes
            .iter()
            .filter(|n| controls.contains(&n.role))
            .filter(|n| n.label.as_deref().is_none_or(|l| l.trim().is_empty()))
            .map(|n| format!("{:?} at {:?}", n.role, n.bounds))
            .collect();
        assert!(unnamed.is_empty(), "controls with no name: {unnamed:?}");
        assert!(
            nodes
                .iter()
                .any(|n| matches!(n.role, Role::TextInput | Role::MultilineTextInput)),
            "the commit message field is exposed as a text field"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.label.as_deref().is_some_and(|l| l.contains("main.rs"))),
            "the files are named"
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
                        && text.galley.text() == "Summary"
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

    /// Every text the Changes view paints for `tab` in a window of `size`, after a few frames.
    fn composer_texts(ctx: &egui::Context, tab: &mut RepoTab, size: egui::Vec2) -> Vec<String> {
        let mut texts = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::top("test-actions")
                        .exact_size(84.0)
                        .show(ui, |_| {});
                    show(ui, tab, &mut Vec::new());
                },
            );
            output.textures_delta.clear();
            texts = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        texts
    }

    /// Issue 92: a summary or description line that is too wide is reported under the fields
    /// while the setting is on, and the draft and the commit button are never touched.
    #[test]
    fn message_hints_follow_the_setting_and_leave_the_draft_alone() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let summary = "Rework the commit composer so that it reports every long line it finds";
        let summary = format!("{summary} before the commit");
        let body = format!("Short.\n{}", "word ".repeat(20));
        tab.commit_summary.clone_from(&summary);
        tab.commit_body.clone_from(&body);
        let mut snap = unstaged_snapshot();
        snap.status.staged = vec![entry("src/lib.rs")];
        tab.snap = Some(std::sync::Arc::new(snap));
        let can = tab.can_commit();
        let size = vec2(562.5, 700.0);
        let hinted = |texts: &[String], needle: &str| texts.iter().any(|t| t.contains(needle));

        tab.check_message = true;
        let texts = composer_texts(&ctx, &mut tab, size);
        assert!(
            hinted(
                &texts,
                &format!("Summary is {} characters", summary.chars().count())
            ),
            "summary hint missing in {texts:?}"
        );
        assert!(
            hinted(&texts, "Description line 2 is 99 characters"),
            "description hint missing in {texts:?}"
        );
        assert_eq!((&tab.commit_summary, &tab.commit_body), (&summary, &body));
        assert_eq!(tab.can_commit(), can, "a hint never blocks the commit");

        tab.check_message = false;
        let texts = composer_texts(&ctx, &mut tab, size);
        assert!(
            !hinted(&texts, "characters;"),
            "no hints when off: {texts:?}"
        );
        assert!(!hinted(&texts, "description line"), "{texts:?}");
        assert_eq!((&tab.commit_summary, &tab.commit_body), (&summary, &body));

        // A message that fits is never hinted.
        tab.check_message = true;
        tab.commit_summary = "Add metrics".into();
        tab.commit_body = "Explains why.".into();
        let texts = composer_texts(&ctx, &mut tab, size);
        assert!(!hinted(&texts, "characters;"), "{texts:?}");
    }

    /// The composer reserves room for the hints, in a short window as in a tall one.
    #[test]
    fn message_hints_make_the_composer_taller() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let mut snap = unstaged_snapshot();
        snap.status.staged = vec![entry("src/lib.rs")];
        let snap = std::sync::Arc::new(snap);
        tab.snap = Some(std::sync::Arc::clone(&snap));
        tab.commit_summary = "x".repeat(80);
        tab.commit_body = "word ".repeat(20);
        for height in [400.0, 900.0] {
            let mut heights = Vec::new();
            for on in [false, true] {
                tab.check_message = on;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(562.5, height),
                        )),
                        ..Default::default()
                    },
                    |ui| heights.push(composer_height(ui, &tab)),
                );
                output.textures_delta.clear();
            }
            assert!(
                (heights[1] - heights[0] - 2.0 * MESSAGE_HINT_HEIGHT).abs() < f32::EPSILON,
                "{heights:?} at {height}"
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
            let composer0 = painted_at(&before, "Summary");
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
            let composer1 = painted_at(&rest, "Summary");
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
                painted_at(&change, "Summary"),
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

    /// Draws the composer fields for one frame and names the field that has keyboard focus.
    fn composer_focus(
        ctx: &egui::Context,
        tab: &mut RepoTab,
        events: Vec<egui::Event>,
    ) -> Option<String> {
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 700.0))),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    composer_fields(ui, tab, &mut Vec::new());
                });
            },
        );
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.expect("tree");
        update
            .nodes
            .iter()
            .find(|(id, _)| *id == update.focus)
            .and_then(|(_, n)| n.label().map(str::to_owned))
    }

    fn composer_ctx() -> (egui::Context, RepoTab) {
        let ctx = crate::ui::kit::testkit::ctx();
        ctx.enable_accesskit();
        let (worker, _, _) = crate::worker::Worker::detached();
        let tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        (ctx, tab)
    }

    /// Usersim: Enter in the summary dropped all focus, so the next keys ran as bare shortcuts.
    /// It moves to the description; Ctrl+Enter leaves the fields to the commit shortcut.
    #[test]
    fn enter_in_the_summary_moves_focus_to_the_description() {
        let (ctx, mut tab) = composer_ctx();
        tab.focus_summary = true;
        for _ in 0..3 {
            composer_focus(&ctx, &mut tab, Vec::new());
        }
        assert_eq!(
            composer_focus(&ctx, &mut tab, Vec::new()).as_deref(),
            Some("Commit summary")
        );
        composer_focus(
            &ctx,
            &mut tab,
            crate::ui::kit::testkit::key(egui::Key::Enter),
        );
        assert_eq!(
            composer_focus(&ctx, &mut tab, Vec::new()).as_deref(),
            Some("Commit description")
        );
    }

    /// After a commit the composer is cleared and neither text field keeps focus.
    #[test]
    fn a_finished_commit_takes_focus_out_of_the_text_fields() {
        let (ctx, mut tab) = composer_ctx();
        tab.focus_summary = true;
        for _ in 0..3 {
            composer_focus(&ctx, &mut tab, Vec::new());
        }
        composer_focus(
            &ctx,
            &mut tab,
            crate::ui::kit::testkit::key(egui::Key::Enter),
        );
        composer_focus(&ctx, &mut tab, Vec::new());
        let (worker, events, _jobs) = crate::worker::Worker::detached();
        tab.worker = worker;
        events
            .send(crate::worker::Event::OpFinished {
                label: "Committing".into(),
                kind: crate::worker::OpKind::Commit,
                announce: false,
                result: Ok("a1b2c3d".into()),
            })
            .expect("commit completion");
        tab.poll(&[]);
        for _ in 0..2 {
            composer_focus(&ctx, &mut tab, Vec::new());
        }
        assert_eq!(composer_focus(&ctx, &mut tab, Vec::new()), None);
    }
}
