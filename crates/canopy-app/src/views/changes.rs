//! The staging area: conflicted, unstaged and staged files, the commit composer and the diff.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::identity::Identity;
use canopy_core::{DiffSource, FileEntry};

use crate::action::Action;
use crate::tab::{FileSelection, RepoTab};
use crate::ui::theme::{self, Palette, palette};
use crate::ui::{art, widgets};
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
    let p = palette(ui.ctx());
    let Some(snap) = tab.snap.clone() else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };
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
        .frame(
            egui::Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border)),
        )
        .show(ui, |ui| {
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
                                ui.painter().hline(
                                    rule,
                                    ui.cursor().min.y,
                                    Stroke::new(1.0, p.border),
                                );
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
                        .fill(p.panel)
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
        .frame(egui::Frame::new().fill(p.bg))
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

/// Gap between the last list and the composer when the composer follows the lists.
const COMPOSER_GAP: f32 = 28.0;

/// Height of the sections `lists` draws, close enough to decide whether the composer fits
/// below them. An overestimate only pins the composer; an underestimate scrolls.
fn lists_height(st: &canopy_core::WorkingStatus) -> f32 {
    if st.is_clean() {
        return 40.0;
    }
    let (conflicts, unstaged, staged) = visible_lists(st);
    let section = |n: usize| {
        let body = if n == 0 {
            24.0
        } else {
            n as f32 * commit_detail::FILE_ROW_H
        };
        widgets::COMPACT_H + 4.0 + body + 10.0
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

/// The sections of the file column: the not-available notice, the clean placeholder, or the
/// conflicted, unstaged and staged lists.
fn lists(
    ui: &mut Ui,
    tab: &mut RepoTab,
    snap: &canopy_core::RepoSnapshot,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
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
                    .font(FontId::new(theme::TITLE_SIZE, theme::semibold())),
            );
            ui.label(
                RichText::new(&u.message)
                    .color(p.text_dim)
                    .size(theme::META_SIZE),
            );
            if u.can_download {
                ui.add_space(8.0);
                if ui
                    .button(format!("{}  Retry", icon::ARROWS_CLOCKWISE))
                    .clicked()
                {
                    actions.push(Action::Refresh);
                }
            }
        });
    } else if st.is_clean() {
        // The illustration and the summary live in the main pane (`clean_home`); here the
        // column only says there is nothing to list.
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            widgets::section_label(ui, "Changes");
            widgets::count_badge(ui, 0, p.text_faint);
        });
    } else {
        let (conflicts, unstaged, staged) = visible_lists(st);
        if conflicts {
            list(ui, tab, List::Conflicted, &st.conflicted, actions);
            ui.add_space(10.0);
        }
        if unstaged {
            list(ui, tab, List::Unstaged, &st.unstaged, actions);
            ui.add_space(10.0);
        }
        if staged {
            list(ui, tab, List::Staged, &st.staged, actions);
        }
    }
    ui.spacing_mut().item_spacing.y = spacing;
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
    let width = ui.available_width().min(420.0);
    let top = ((ui.available_height() - 380.0) / 2.0).max(24.0);
    ui.vertical_centered(|ui| {
        ui.add_space(top);
        art::show(ui, art::empty_clean(), vec2(200.0, 150.0));
        ui.add_space(8.0);
        ui.label(
            RichText::new("Working tree clean")
                .font(FontId::new(theme::HEADLINE_SIZE, theme::semibold())),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(format!(
                "Nothing to commit on {}. Edit some files and they show up here.",
                snap.head.label()
            ))
            .color(p.text_dim)
            .size(theme::META_SIZE),
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
        ui.label(RichText::new(status).size(theme::BODY_SIZE).color(p.text));
        if let Some((label, action)) = next {
            ui.add_space(theme::SM);
            if widgets::primary_button(ui, label, !busy, 160.0_f32.min(width)).clicked() {
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
                .size(theme::SMALL_SIZE)
                .color(p.text_faint),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&c.summary)
                        .size(theme::META_SIZE)
                        .color(p.text_dim),
                )
                .truncate(),
            );
        }
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
    let (title, color) = match which {
        List::Conflicted => ("Conflicts", p.danger),
        List::Unstaged => ("Unstaged", p.warning),
        List::Staged => ("Staged", p.success),
    };
    let compact = ui.available_width() < 280.0;
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        widgets::section_label(ui, title);
        // An empty list is not news: its count stays neutral instead of taking the section colour.
        widgets::count_badge(ui, files.len(), if files.is_empty() { p.text_faint } else { color });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match which {
            List::Unstaged if !files.is_empty() => {
                if widgets::compact_button(ui, if compact { icon::PLUS.to_owned() } else { format!("{}  Stage all", icon::PLUS) }).on_hover_text("Stage all files").clicked() {
                    actions.push(Action::Op(Op::StageAll));
                }
                // Discarding everything lives in an overflow menu, a gap away from Stage.
                ui.add_space(theme::MD);
                ui.scope(|ui| {
                    ui.spacing_mut().button_padding = vec2(theme::SM, 4.0);
                    ui.spacing_mut().interact_size.y = widgets::COMPACT_H;
                    ui.menu_button(RichText::new(icon::DOTS_THREE).size(theme::ICON_MD), |ui| {
                        ui.set_min_width(200.0);
                        if ui.button(RichText::new(format!("{}  Discard all changes…", icon::ARROW_COUNTER_CLOCKWISE)).color(p.danger)).clicked() {
                            actions.push(Action::Confirm { title: "Discard all changes".into(), body: format!("Throw away every unstaged change in {} files, including untracked files? This cannot be undone.", files.len()), confirm: "Discard all".into(), danger: true, op: Op::Discard(files.to_vec()) });
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("More actions");
                });
            }
            List::Staged if !files.is_empty()
                && widgets::compact_button(ui, if compact { icon::MINUS.to_owned() } else { format!("{}  Unstage all", icon::MINUS) }).on_hover_text("Unstage all files").clicked() => {
                    actions.push(Action::Op(Op::UnstageAll));
                }
            _ => {}
        });
    });
    ui.add_space(4.0);
    if files.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new(if which == List::Staged {
                    "Nothing staged yet"
                } else {
                    "No unstaged changes"
                })
                .size(theme::META_SIZE)
                .color(p.text_faint),
            );
        });
        return;
    }
    commit_detail::virtual_rows(ui, files.len(), commit_detail::FILE_ROW_H, |ui, i| {
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
        let id = ui.id().with(("change", which as u8, &f.path));
        let resp = commit_detail::file_row(ui, f, selected, id);
        // Hover action button on the right.
        let btn = egui::Rect::from_center_size(
            resp.rect.right_center() - vec2(18.0, 0.0),
            vec2(24.0, 22.0),
        );
        let (glyph, tip) = if which == List::Staged {
            (icon::MINUS, "Unstage")
        } else if conflicted {
            (icon::CHECK, "Mark resolved")
        } else {
            (icon::PLUS, "Stage")
        };
        let bresp = ui.interact(btn, resp.id.with("act"), Sense::click());
        bresp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{tip} {}", f.path))
        });
        if resp.hovered() || bresp.hovered() || selected {
            ui.painter().rect(
                btn,
                CornerRadius::same(theme::RADIUS_SM),
                if bresp.hovered() {
                    Palette::tint(p.accent, 90)
                } else {
                    p.elevated
                },
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                btn.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(theme::BODY_SIZE),
                p.text,
            );
        }
        let toggle = || {
            if which == List::Staged {
                Op::Unstage(vec![f.path.clone()])
            } else {
                Op::Stage(vec![f.path.clone()])
            }
        };
        if bresp.on_hover_text(tip).clicked() || resp.double_clicked() {
            actions.push(Action::Op(toggle()));
        } else if resp.clicked() {
            tab.select_file(FileSelection {
                source,
                entry: f.clone(),
                conflicted,
            });
        }
        let entry = f.clone();
        resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            if ui.button(format!("{}  {}", glyph, tip)).clicked() {
                actions.push(Action::Op(toggle()));
            }
            if which == List::Unstaged
                && ui
                    .button(
                        RichText::new(format!(
                            "{}  Discard changes…",
                            icon::ARROW_COUNTER_CLOCKWISE
                        ))
                        .color(p.danger),
                    )
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
            ui.separator();
            if ui
                .button(format!("{}  Open file", icon::ARROW_SQUARE_OUT))
                .clicked()
            {
                actions.push(Action::OpenInEditor(tab.path.join(&entry.path)));
            }
            if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
                actions.push(Action::Copy(entry.path.clone()));
            }
        });
    });
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
    ui.style_mut().spacing.scroll.floating = false;
    if composer_is_quiet(tab, snap) {
        quiet_composer(ui, tab, snap, actions);
        return;
    }
    let staged = snap.status.staged.len();
    let can = (staged > 0 || tab.amend)
        && !tab.commit_summary.trim().is_empty()
        && tab.running.is_none()
        && snap.status.conflicted.is_empty();
    let blocker = if can {
        None
    } else {
        commit_blocker(
            !snap.status.conflicted.is_empty(),
            tab.running.is_some(),
            staged > 0 || tab.amend,
            tab.commit_summary.trim().is_empty(),
        )
    };
    let branch = snap.head.label();
    let label = if tab.amend {
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
    let hint = warning.map_or_else(
        || crate::keymap::hint(crate::keymap::Command::Commit).to_owned(),
        |w| format!("{w} Open Settings → Git to correct it. Git's own error will appear if you commit without a usable identity."),
    );
    let footer = |ui: &mut Ui, actions: &mut Vec<Action>| {
        if let Some(warning) = warning {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}  {warning}", icon::WARNING))
                            .color(p.warning)
                            .size(theme::SMALL_SIZE),
                    )
                    .wrap(),
                );
                if ui.small_button("Edit identity").clicked() {
                    actions.push(Action::IdentitySettings);
                }
            });
            ui.add_space(theme::XS);
        }
        if let Some(reason) = blocker {
            ui.label(
                RichText::new(reason)
                    .size(theme::SMALL_SIZE)
                    .color(p.text_dim),
            );
            ui.add_space(theme::XS);
        }
        if widgets::primary_button(ui, &label, can, ui.available_width())
            .on_hover_text(hint)
            .clicked()
            && can
        {
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
            ui.painter().hline(
                ui.max_rect().x_range(),
                ui.max_rect().min.y - 12.0,
                Stroke::new(1.0, p.border),
            );
            composer_fields(ui, tab, actions);
        });
}

/// A clean tree with nothing typed: the composer shrinks to a line and the Amend choice, so an
/// empty form and a disabled button do not compete with the rest of the view.
fn composer_is_quiet(tab: &RepoTab, snap: &canopy_core::RepoSnapshot) -> bool {
    snap.status.is_clean()
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
    // Nothing to commit yet, so a pending request to focus the summary would fire later,
    // when files appear.
    tab.focus_summary = false;
    widgets::section_label(ui, "Commit");
    ui.add_space(4.0);
    ui.label(
        RichText::new("Nothing to commit")
            .size(theme::META_SIZE)
            .color(p.text_faint),
    );
    if snap.head.oid.is_some() {
        ui.add_space(8.0);
        widgets::checkbox(ui, &mut tab.amend, "Amend last commit")
            .on_hover_text("Replace the last commit with a new message");
        if tab.amend {
            actions.push(Action::PrefillAmend);
        }
    }
}

fn composer_fields(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        widgets::section_label(ui, "Commit");
        let n = tab.commit_summary.chars().count();
        // Counted from the first character: an empty field has nothing to measure.
        if n > 0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let color = if n > 72 {
                    p.danger
                } else if n > 50 {
                    p.warning
                } else {
                    p.text_faint
                };
                ui.label(
                    RichText::new(format!("{n}/72"))
                        .size(theme::SMALL_SIZE)
                        .color(color),
                )
                .on_hover_text("Keep the summary under 50 characters; 72 at most.");
            });
        }
    });
    ui.add_space(4.0);
    let summary = ui.add(
        egui::TextEdit::singleline(&mut tab.commit_summary)
            .id(egui::Id::new("commit-summary"))
            .hint_text("Summary (required)")
            .font(FontId::new(theme::LEAD_SIZE, theme::medium()))
            .desired_width(f32::INFINITY)
            .margin(theme::FIELD_MARGIN),
    );
    if tab.focus_summary {
        summary.request_focus();
        tab.focus_summary = false;
    }
    ui.add_space(6.0);
    ui.add(
        egui::TextEdit::multiline(&mut tab.commit_body)
            .hint_text("Description")
            .desired_rows(if ui.ctx().content_rect().height() < 600.0 {
                2
            } else {
                4
            })
            .desired_width(f32::INFINITY)
            .margin(theme::FIELD_MARGIN),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let was = tab.amend;
        widgets::checkbox(ui, &mut tab.amend, "Amend last commit")
            .on_hover_text("Replace the last commit with the staged changes and this message");
        if tab.amend && !was && tab.commit_summary.is_empty() {
            actions.push(Action::PrefillAmend);
        }
    });
    ui.add_space(8.0);
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
    ) -> Vec<(String, egui::Rect)> {
        let mut texts = Vec::new();
        for _ in 0..3 {
            let events = pointer.map(egui::Event::PointerMoved).into_iter().collect();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(960.0, 600.0),
                    )),
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
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
        let mut snap = unstaged_snapshot();
        snap.status.unstaged.push(entry("src/lib.rs"));
        snap.status.staged = vec![entry("README.md")];
        tab.snap = Some(std::sync::Arc::new(snap));
        // The first unstaged row is selected by default and shows its action; the others
        // show theirs only under the pointer.
        let rest = painted_texts(&ctx, &mut tab, None);
        let shown = |texts: &[(String, egui::Rect)], glyph: &str| {
            texts.iter().filter(|(t, _)| t == glyph).count()
        };
        assert_eq!(shown(&rest, icon::PLUS), 1, "only the selected row");
        assert_eq!(shown(&rest, icon::MINUS), 0);
        for (name, glyph) in [("lib.rs", icon::PLUS), ("README.md", icon::MINUS)] {
            let (_, at) = rest
                .iter()
                .find(|(t, _)| t == name)
                .expect("painted row name");
            let pointer = egui::pos2(at.min.x + 20.0, at.center().y);
            let hovered = painted_texts(&ctx, &mut tab, Some(pointer));
            assert!(
                hovered
                    .iter()
                    .any(|(t, r)| t == glyph && (r.center().y - at.center().y).abs() < 4.0),
                "hovering {name} must show {glyph} on its row"
            );
            painted_texts(&ctx, &mut tab, Some(egui::pos2(5.0, 595.0)));
        }
    }

    #[test]
    fn the_composer_follows_a_short_list_and_pins_below_a_long_one() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
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
        theme::apply(&ctx, theme::ThemeId::Midnight, true);
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
        let unusable = identity("", "", None, false);
        let warning = unusable.warning().expect("identity warning");
        tab.identity = Some(unusable);
        tab.snap = Some(std::sync::Arc::new(unstaged_snapshot()));
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(562.5, 350.0),
                    )),
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
            theme::apply(&ctx, theme::ThemeId::Midnight, true);
            let dir = tempfile::tempdir().expect("temporary tab");
            let mut tab = RepoTab::new(dir.path().into(), &ctx, 100, Default::default());
            tab.snap = Some(std::sync::Arc::new(unstaged_snapshot()));
            let mut visible = false;
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
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
