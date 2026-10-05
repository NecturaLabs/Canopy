//! The staging area: conflicted, unstaged and staged files, the commit composer and the diff.

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, FontFamily, FontId, Layout, Rect, Response, RichText, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::identity::Identity;
use canopy_core::{DiffSource, FileEntry};

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::{FileSelection, RepoTab};
use crate::ui::kit::chip::{ChangeBadge, CountBadge, DiffStat};
use crate::ui::kit::field::{TextArea, TextField};
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{ListRow, RowCtx, RowKind, SectionHeader, caps_galley};
use crate::ui::kit::surface::Banner;
use crate::ui::kit::toggle::Checkbox;
use crate::ui::kit::{Button, IconButton, Size, Tone, anim, menu, paint};
use crate::ui::theme::{self, palette};
use crate::ui::{art, motion, widgets};
use crate::views::{commit_detail, diff, resolve};
use crate::worker::Op;

#[derive(Clone, Copy, PartialEq, Eq)]
enum List {
    Conflicted,
    Unstaged,
    Staged,
}

/// Draws the changes view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let s = theme::surfaces(ui.ctx());
    let Some(snap) = tab.snap.clone() else {
        ui.centered_and_justified(|ui| Spinner::new().label("changes").show(ui));
        return;
    };
    state_banner(ui, tab, actions);
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
    egui::Panel::left("changes-list")
        .resizable(true)
        .default_size(pane_max.min(400.0))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(700.0))
        .frame(egui::Frame::new().fill(s.pane))
        .show(ui, |ui| {
            // The seam to the diff; the pane's other edges are the card's or the window's.
            let edge = ui.max_rect();
            paint::seam_v(ui.painter(), edge.right() - 0.5, edge.y_range(), s.border);
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
                                lists(ui, tab, &snap, actions);
                                ui.add_space(COMPOSER_GAP / 2.0);
                                let rule = ui.max_rect().x_range();
                                paint::seam_h(ui.painter(), rule, ui.cursor().min.y, s.border);
                                ui.add_space(COMPOSER_GAP / 2.0);
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(4, 0))
                                    .show(ui, |ui| composer(ui, tab, &snap, actions, true));
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
                            lists(ui, tab, &snap, actions);
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
}

/// The operation in progress (a merge, rebase, cherry-pick or revert) across the top of the
/// view, with Continue and Abort.
fn state_banner(ui: &mut Ui, tab: &RepoTab, actions: &mut Vec<Action>) {
    let Some(snap) = &tab.snap else {
        return;
    };
    if snap.state == canopy_core::RepoState::Clean {
        return;
    }
    let state = snap.state;
    let conflicts = snap.status.conflicted.len();
    let busy = tab.running.is_some();
    egui::Panel::top("repository-operation-state")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let left = if conflicts > 0 {
                format!(
                    "{conflicts} conflict{} left",
                    if conflicts == 1 { "" } else { "s" }
                )
            } else {
                "No conflicts left".to_owned()
            };
            let banner = Banner::new(
                Tone::Warning,
                format!("{} in progress on {}", state.label(), snap.head.label()),
            )
            .body(left)
            .id_salt("repository-operation-state")
            .show(ui);
            let Some(command) = state.command() else {
                return;
            };
            // The banner's own actions cannot be disabled or explain themselves, so Continue and
            // Abort sit over its right end, where its actions would be.
            let rect = banner.response.rect;
            if rect.height() <= 0.0 {
                return;
            }
            let full = theme::metrics().pane_header_height;
            let row = Rect::from_min_size(
                pos2(rect.left(), rect.bottom() - full),
                vec2(rect.width(), full),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(row.shrink2(vec2(theme::SM, 0.0)))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            child.spacing_mut().item_spacing.x = theme::SM;
            if Button::new(format!("Abort {command}…"))
                .ghost()
                .size(Size::Sm)
                .icon(icon::X_CIRCLE)
                .icon_tone(Tone::Danger)
                .enabled(!busy)
                .disabled_reason("Wait for the current operation")
                .id_salt("state-abort")
                .show(&mut child)
                .clicked()
            {
                actions.push(Action::Confirm {
                    title: format!("Abort {command}"),
                    body: "Return the repository to where it was before the operation \
                           started? Resolved conflicts are discarded."
                        .into(),
                    confirm: "Abort".into(),
                    danger: true,
                    op: Op::ContinueOrAbort { state, abort: true },
                });
            }
            let can_continue = conflicts == 0 && !busy;
            let why = if busy {
                "Wait for the current operation"
            } else {
                "Resolve and stage every conflicted file first"
            };
            if Button::new("Continue")
                .ghost()
                .size(Size::Sm)
                .icon(icon::ARROW_RIGHT)
                .enabled(can_continue)
                .tooltip("Continue this operation using the staged resolutions")
                .disabled_reason(why)
                .id_salt("state-continue")
                .show(&mut child)
                .clicked()
            {
                actions.push(Action::Op(Op::ContinueOrAbort {
                    state,
                    abort: false,
                }));
            }
        });
}

/// Gap between the last list and the composer when the composer follows the lists.
const COMPOSER_GAP: f32 = 28.0;
/// Gap between two file lists, and the height of an empty list's placeholder line.
const LIST_GAP: f32 = theme::MD;
const EMPTY_LIST: f32 = theme::XL;

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
        let body = if n == 0 { EMPTY_LIST } else { n as f32 * row_h };
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
            ui.label(
                RichText::new("Changes not available")
                    .font(FontId::new(m.subtitle_size, theme::semibold())),
            );
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
        list_header(ui, "Changes", 0, Tone::Neutral, |_| {});
    } else {
        let (conflicts, unstaged, staged) = visible_lists(st);
        if conflicts {
            list(ui, tab, List::Conflicted, &st.conflicted, actions);
            ui.add_space(LIST_GAP);
        }
        if unstaged {
            list(ui, tab, List::Unstaged, &st.unstaged, actions);
            ui.add_space(LIST_GAP);
        }
        if staged {
            list(ui, tab, List::Staged, &st.staged, actions);
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
                ui.label(
                    RichText::new("Working tree clean")
                        .font(FontId::new(m.display_size, theme::semibold())),
                );
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
                ui.label(RichText::new(status).size(m.body_size).color(p.text));
                if let Some((label, action)) = next {
                    ui.add_space(theme::SM);
                    if Button::new(label)
                        .primary()
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

fn list(
    ui: &mut Ui,
    tab: &mut RepoTab,
    which: List,
    files: &[FileEntry],
    actions: &mut Vec<Action>,
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
                    actions.push(Action::Confirm {
                        title: "Discard all changes".into(),
                        body: format!(
                            "Throw away every unstaged change in {} files, including untracked \
                             files? This cannot be undone.",
                            files.len()
                        ),
                        confirm: "Discard all".into(),
                        danger: true,
                        op: Op::Discard(files.to_vec()),
                    });
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
    if files.is_empty() {
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
        // The row's action, revealed under the pointer. A conflicted file's is a labelled
        // "Resolve conflicts…", shown only under the pointer: a bare check would read as
        // "already done". The selected row keeps its Stage or Unstage button.
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
            pinned: selected && !conflicted,
        };
        let (resp, acted) = file_row(ui, f, selected, ("change", which as u8, &f.path), action);
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
                    source: source.clone(),
                    entry: f.clone(),
                    conflicted,
                });
            } else {
                actions.push(Action::Op(toggle()));
            }
        } else if resp.clicked() {
            tab.select_file(FileSelection {
                source: source.clone(),
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
                        source: source.clone(),
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
                    actions.push(Action::Confirm {
                        title: "Discard changes".into(),
                        body: format!(
                            "Throw away the unstaged changes to {}? This cannot be undone.",
                            entry.path
                        ),
                        confirm: "Discard".into(),
                        danger: true,
                        op: Op::Discard(vec![entry.clone()]),
                    });
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
    /// Shown without the pointer too (the selected row).
    pinned: bool,
}

/// A changed file: its status badge, name, folder and line counts, and `action` revealed left of
/// the counts while the row is hovered (or pinned). Returns the row and whether the action was
/// clicked.
fn file_row(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    action: RowAction,
) -> (Response, bool) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let m = theme::metrics();
    let height = RowKind::File.height();
    // The row lands at the cursor, full width (as `ListRow` allocates it).
    let ahead = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
    let hovered = ui.rect_contains_pointer(ahead);
    let mut state: Option<RowCtx> = None;
    let row = ListRow::new(id_salt)
        .kind(RowKind::File)
        .selected(selected)
        .show_with(ui, |rc, _| state = Some(*rc));
    row.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &f.path)
    });
    let reveal = anim::ease_bool(
        &ctx,
        row.id.with("file-row-action"),
        hovered || action.pinned,
        motion::FAST,
        motion::FAST,
    );
    let mut acted = false;
    if let Some(rc) = state {
        let cy = rc.rect.center().y;
        let badge_rect =
            Rect::from_center_size(pos2(rc.content.left() + 8.5, cy), vec2(17.0, 17.0));
        let mut badge = ui.new_child(UiBuilder::new().max_rect(badge_rect));
        ChangeBadge::new(f.kind).show(&mut badge);
        let mut end = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(badge_rect.right(), rc.rect.top()),
                    pos2(rc.content.right(), rc.rect.bottom()),
                ))
                .layout(Layout::right_to_left(Align::Center)),
        );
        end.spacing_mut().item_spacing.x = theme::SM;
        if f.additions + f.deletions > 0 {
            DiffStat::new(f.additions, f.deletions).show(&mut end);
        }
        if reveal > 0.0 {
            end.scope(|ui| {
                ui.multiply_opacity(reveal);
                let r = if action.labelled {
                    Button::new(action.tip)
                        .ghost()
                        .size(Size::Sm)
                        .id_salt(("file-row-action", &f.path))
                        .show(ui)
                } else {
                    IconButton::new(action.glyph, action.tip)
                        .size(Size::Sm)
                        .id_salt(("file-row-action", &f.path))
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
        }
        let right = end.min_rect().left() - theme::SM;
        let left = badge_rect.right() + theme::SM;
        if right > left {
            let painter = ui.painter().with_clip_rect(
                Rect::from_x_y_ranges(left..=right, rc.rect.y_range()).intersect(ui.clip_rect()),
            );
            let (name, dir) = widgets::split_path(&f.path);
            let name = painter.layout_no_wrap(
                name.to_owned(),
                FontId::new(m.body_size, theme::medium()),
                rc.text,
            );
            let name_w = name.size().x;
            painter.galley(pos2(left, cy - name.size().y / 2.0), name, rc.text);
            if !dir.is_empty() {
                let mut job = LayoutJob::default();
                job.append(
                    dir,
                    0.0,
                    TextFormat::simple(
                        FontId::new(m.meta_size, FontFamily::Proportional),
                        p.text_faint,
                    ),
                );
                let dir = painter.layout_job(job);
                painter.galley(
                    pos2(left + name_w + theme::SM, cy - dir.size().y / 2.0),
                    dir,
                    p.text_faint,
                );
            }
        }
    }
    let tip = match &f.old_path {
        Some(old) => format!("{} → {}", old, f.path),
        None => f.path.clone(),
    };
    (row.on_hover_text(tip), acted)
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
    let label = if merging {
        format!("Conclude merge on {branch}")
    } else if tab.amend {
        format!("Amend {branch}")
    } else if staged > 0 {
        format!(
            "Commit {staged} file{} to {branch}",
            if staged == 1 { "" } else { "s" }
        )
    } else {
        "Stage files to commit".to_owned()
    };
    let warning = identity_notice(tab.identity.as_ref(), tab.identity_error.is_some());
    let tip = warning.map_or_else(
        || "Commit the staged changes".to_owned(),
        |w| format!("{w} Open Settings → Git to correct it. Git's own error will appear if you commit without a usable identity."),
    );
    let footer = |ui: &mut Ui, actions: &mut Vec<Action>| {
        if let Some(warning) = warning {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}  {warning}", icon::WARNING))
                            .color(p.warning)
                            .size(m.small_size),
                    )
                    .wrap(),
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
        let mut commit = Button::new(&label)
            .primary()
            .icon(icon::CHECK)
            .full_width()
            .enabled(can)
            .tooltip(tip.as_str())
            .shortcut(Command::Commit)
            .show_shortcut(true);
        if let Some(reason) = blocker {
            commit = commit.disabled_reason(reason);
        }
        if commit.show(ui).clicked() && can {
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

    fn identity(name: &str, email: &str, source: Option<Source>, usable: bool) -> Identity {
        let field = |value: &str| Field {
            value: value.into(),
            source: source.clone(),
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
                |ui| show(ui, tab, &mut Vec::new()),
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
        theme::apply(&ctx, theme::ThemeId::Midnight, &theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
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
        theme::apply(&ctx, theme::ThemeId::Midnight, &theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let tab = RepoTab::with_worker("fixture".into(), 100, Default::default(), worker);
        let snap = canopy_core::RepoSnapshot::default();
        let mut publish_visible = false;
        for frame in 0..40 {
            let mut events = vec![egui::Event::PointerMoved(pos2(180.0, 100.0))];
            if frame > 5 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -30.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
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
        theme::apply(&ctx, theme::ThemeId::Midnight, &theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
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
        let good = identity("Ada Lovelace", "ada@example.com", configured.clone(), true);
        assert_eq!(identity_notice(Some(&good), false), None);
        assert_eq!(
            identity_notice(None, false),
            None,
            "still reading: no warning yet"
        );

        let placeholder = identity("t", "t@t", configured.clone(), true);
        assert_eq!(
            identity_notice(Some(&placeholder), false),
            Some("This identity looks like a placeholder.")
        );
        let guessed = identity("ada", "ada@host.example", Some(Source::Guessed), true);
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
        theme::apply(&ctx, theme::ThemeId::Midnight, &theme::Appearance::STILL);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
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

    #[test]
    fn commit_button_is_visible_in_default_desktop_layouts() {
        for size in [vec2(600.0, 375.0), vec2(960.0, 600.0), vec2(1440.0, 900.0)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, &theme::Appearance::STILL);
            let dir = tempfile::tempdir().expect("temporary tab");
            let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
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
}
