//! The tab strip and the action bar.

use egui::{Align, Align2, CornerRadius, FontId, Layout, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::keymap::{Command, hint};
use crate::tab::{RepoTab, View};
use crate::ui::theme::{self, Palette, ThemeId, palette};
use crate::ui::widgets::forge_icon;
use crate::ui::{art, widgets};
use crate::worker::Op;

/// "Fetch (Ctrl+Shift+F)".
fn tip(text: &str, command: Command) -> String {
    let keys = hint(command);
    if keys.is_empty() {
        text.to_owned()
    } else {
        format!("{text}  ({keys})")
    }
}

/// Repository tabs, browser style.
pub fn tabs(
    ui: &mut Ui,
    tabs: &[RepoTab],
    active: usize,
    current_theme: ThemeId,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    ui.horizontal_centered(|ui| {
        ui.add_space(10.0);
        art::show(ui, art::logo(), vec2(22.0, 22.0));
        ui.add_space(4.0);
        ui.label(
            RichText::new("Canopy")
                .font(FontId::new(theme::SUBTITLE_SIZE, theme::semibold()))
                .color(p.text),
        );
        ui.add_space(14.0);
        let compact = ui.ctx().content_rect().width() < theme::COMPACT_WIDTH;
        let reserve = if compact { 150.0 } else { 390.0 };
        egui::ScrollArea::horizontal()
            .id_salt("repository-tabs")
            .max_width((ui.available_width() - reserve).max(60.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, tab) in tabs.iter().enumerate() {
                        let selected = i == active;
                        let text = tab.name.clone();
                        let galley = ui.painter().layout_no_wrap(
                            text,
                            FontId::new(theme::BODY_SIZE, theme::medium()),
                            p.text,
                        );
                        // Commits to push and pull, like the status bar, so every tab shows its sync state.
                        let sync = tab
                            .snap
                            .as_ref()
                            .and_then(|s| s.current_branch())
                            .filter(|b| b.upstream.is_some() && (b.ahead > 0 || b.behind > 0))
                            .map(|b| {
                                let mut job = egui::text::LayoutJob::default();
                                let font = FontId::new(theme::SMALL_SIZE, theme::medium());
                                if b.ahead > 0 {
                                    job.append(
                                        &format!("↑{}", b.ahead),
                                        0.0,
                                        egui::TextFormat::simple(font.clone(), p.success),
                                    );
                                }
                                if b.behind > 0 {
                                    job.append(
                                        &format!("↓{}", b.behind),
                                        if b.ahead > 0 { 4.0 } else { 0.0 },
                                        egui::TextFormat::simple(font, p.accent2),
                                    );
                                }
                                (ui.painter().layout_job(job), b.ahead, b.behind)
                            });
                        let sync_w = sync.as_ref().map_or(0.0, |(g, _, _)| g.size().x + 8.0);
                        let w = galley.size().x + 58.0 + sync_w;
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                        let hover = ui
                            .ctx()
                            .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                        let fill = if selected {
                            p.panel()
                        } else {
                            Palette::tint(p.text, (hover * 12.0) as u8)
                        };
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius {
                                nw: 8,
                                ne: 8,
                                sw: 0,
                                se: 0,
                            },
                            fill,
                        );
                        if selected {
                            let bar = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 2.0));
                            widgets::gradient_rect(ui.painter(), bar, p.accent, p.accent2, 1);
                        }
                        let busy = tab.running.is_some();
                        let glyph = if busy {
                            icon::CIRCLE_NOTCH
                        } else {
                            icon::FOLDER_SIMPLE
                        };
                        ui.painter().text(
                            rect.left_center() + vec2(14.0, 0.0),
                            Align2::CENTER_CENTER,
                            glyph,
                            FontId::proportional(theme::ICON_SM),
                            if selected { p.accent } else { p.text_faint },
                        );
                        let name_w = galley.size().x;
                        ui.painter().galley(
                            rect.left_center() + vec2(26.0, -galley.size().y / 2.0),
                            galley,
                            if selected { p.text } else { p.text_dim },
                        );
                        let mut sync_text = String::new();
                        if let Some((g, ahead, behind)) = sync {
                            ui.painter().galley(
                                rect.left_center() + vec2(26.0 + name_w + 8.0, -g.size().y / 2.0),
                                g,
                                p.text_dim,
                            );
                            sync_text = format!(", {ahead} to push, {behind} to pull");
                        }
                        let close = egui::Rect::from_center_size(
                            rect.right_center() - vec2(15.0, 0.0),
                            vec2(18.0, 18.0),
                        );
                        let close_resp = ui.interact(close, resp.id.with("close"), Sense::click());
                        if selected || resp.hovered() || close_resp.hovered() {
                            if close_resp.hovered() {
                                ui.painter()
                                    .rect_filled(close, CornerRadius::same(4), p.hover());
                            }
                            ui.painter().text(
                                close.center(),
                                Align2::CENTER_CENTER,
                                icon::X,
                                FontId::proportional(theme::ICON_XS),
                                p.text_dim,
                            );
                        }
                        if close_resp.clicked() {
                            actions.push(Action::CloseTab(i));
                        } else if resp.clicked() {
                            actions.push(Action::SwitchTab(i));
                        } else if resp.middle_clicked() {
                            actions.push(Action::CloseTab(i));
                        }
                        let label = format!("Repository tab {}{sync_text}", tab.name);
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::Button,
                                true,
                                selected,
                                &label,
                            )
                        });
                        resp.on_hover_text(format!(
                            "{}\n{}",
                            tab.path.display(),
                            if i < 9 {
                                format!(
                                    "{}+{}",
                                    if cfg!(target_os = "macos") {
                                        "Cmd"
                                    } else {
                                        "Ctrl"
                                    },
                                    i + 1
                                )
                            } else {
                                String::new()
                            }
                        ));
                    }
                    if widgets::icon_button(ui, icon::PLUS, &tip("Open repository", Command::Open))
                        .clicked()
                    {
                        actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::open()));
                    }
                });
            });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            if widgets::icon_button(
                ui,
                icon::KEYBOARD,
                &tip("Keyboard shortcuts", Command::Cheatsheet),
            )
            .clicked()
            {
                actions.push(Action::Cheatsheet);
            }
            if widgets::icon_button(ui, icon::GEAR_SIX, &tip("Settings", Command::Settings))
                .clicked()
            {
                actions.push(Action::Settings);
            }
            let theme_resp = widgets::icon_button(ui, icon::PALETTE, "Theme");
            egui::Popup::menu(&theme_resp).show(|ui| {
                ui.set_min_width(170.0);
                for t in ThemeId::ALL {
                    let tp = t.palette();
                    let resp = ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(28.0, 16.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(4), tp.bg());
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(r.min + vec2(14.0, 0.0), vec2(14.0, 16.0)),
                            CornerRadius {
                                nw: 0,
                                sw: 0,
                                ne: 4,
                                se: 4,
                            },
                            tp.accent,
                        );
                        ui.painter().rect_stroke(
                            r,
                            CornerRadius::same(4),
                            Stroke::new(1.0, p.border()),
                            egui::StrokeKind::Inside,
                        );
                        ui.selectable_label(t == current_theme, t.name())
                    });
                    if resp.inner.clicked() {
                        actions.push(Action::SetTheme(t));
                    }
                }
            });
            // Search stays reachable when repository tabs take the available width.
            if compact {
                if widgets::icon_button(
                    ui,
                    icon::MAGNIFYING_GLASS,
                    &tip("Search commands", Command::Palette),
                )
                .clicked()
                {
                    actions.push(Action::Palette);
                }
                return;
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(260.0, 26.0), Sense::click());
            let hover = ui
                .ctx()
                .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            ui.painter().rect(
                rect,
                CornerRadius::same(13),
                p.bg(),
                Stroke::new(1.0, p.border().lerp_to_gamma(p.accent, hover * 0.6)),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.left_center() + vec2(14.0, 0.0),
                Align2::CENTER_CENTER,
                icon::MAGNIFYING_GLASS,
                FontId::proportional(theme::BODY_SIZE),
                p.text_faint,
            );
            ui.painter().text(
                rect.left_center() + vec2(28.0, 0.0),
                Align2::LEFT_CENTER,
                "Search commands",
                FontId::proportional(theme::META_SIZE),
                p.text_faint,
            );
            ui.painter().text(
                rect.right_center() - vec2(12.0, 0.0),
                Align2::RIGHT_CENTER,
                hint(Command::Palette),
                FontId::monospace(theme::MONO_SMALL_SIZE),
                p.text_faint,
            );
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Search commands")
            });
            if resp.clicked() {
                actions.push(Action::Palette);
            }
        });
    });
}

/// The action bar of the active repository.
pub fn actions_bar(ui: &mut Ui, tab: &RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let snap = tab.snap.as_ref();
    let has_remote = snap.is_some_and(|s| !s.remotes.is_empty());
    let local_only = tab.is_local_only();
    let busy = tab.running.is_some();
    let branch = snap.and_then(|s| s.current_branch());
    let forge = tab.forge();
    let width = ui.available_width();
    ui.style_mut().spacing.scroll.floating = false;
    egui::ScrollArea::horizontal()
        .id_salt("toolbar-actions")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(width);
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);
                // View switcher (segmented control).
                let changes = snap.map(|s| s.status.len()).unwrap_or(0);
                let open_requests = match &tab.requests.list {
                    Some(Ok(l)) if tab.requests.filter == canopy_hosting::StateFilter::Open => {
                        l.len()
                    }
                    _ => 0,
                };
                segmented(
                    ui,
                    tab.view,
                    changes,
                    forge.map(|(_, f)| (f.kind, open_requests)),
                    actions,
                );
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(4.0);
                if widgets::tool_button(
                    ui,
                    icon::ARROWS_CLOCKWISE,
                    "Fetch",
                    &tip("Fetch all remotes", Command::Fetch),
                    has_remote && !busy,
                )
                .clicked()
                {
                    actions.push(Action::Fetch);
                }
                let behind = branch.map(|b| b.behind).unwrap_or(0);
                let pull_label = if behind > 0 {
                    format!("Pull ↓{behind}")
                } else {
                    "Pull".to_owned()
                };
                if widgets::tool_button(
                    ui,
                    icon::ARROW_FAT_LINE_DOWN,
                    &pull_label,
                    &tip("Pull the current branch", Command::Pull),
                    has_remote && !busy,
                )
                .clicked()
                {
                    actions.push(Action::Pull);
                }
                let ahead = branch.map(|b| b.ahead).unwrap_or(0);
                if local_only {
                    if widgets::tool_button(
                        ui,
                        icon::CLOUD_ARROW_UP,
                        "Publish",
                        "This repository has no remote. Add one on GitHub or GitLab and push",
                        !busy && branch.is_some(),
                    )
                    .clicked()
                    {
                        actions.push(Action::Publish);
                    }
                } else {
                    let push_label = if ahead > 0 {
                        format!("Push ↑{ahead}")
                    } else {
                        "Push".to_owned()
                    };
                    if widgets::tool_button(
                        ui,
                        icon::ARROW_FAT_LINE_UP,
                        &push_label,
                        &tip("Push the current branch", Command::Push),
                        has_remote && !busy && branch.is_some(),
                    )
                    .clicked()
                    {
                        actions.push(Action::Push);
                    }
                }
                // Pull/merge request for a pushed branch.
                if let (Some((_, f)), Some(b)) = (forge, branch)
                    && b.upstream.is_some()
                    && !tab.is_default_branch(&b.name)
                {
                    let label = f.kind.request_abbrev();
                    if widgets::tool_button(
                        ui,
                        icon::GIT_PULL_REQUEST,
                        label,
                        &format!(
                            "Create a {} for {} on {}",
                            f.kind.request_noun(),
                            b.name,
                            f.kind.name()
                        ),
                        !busy,
                    )
                    .clicked()
                    {
                        actions.push(Action::CreateRequest {
                            source: b.name.clone(),
                        });
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                if widgets::tool_button(
                    ui,
                    icon::GIT_BRANCH,
                    "Branch",
                    &tip("New branch", Command::NewBranch),
                    snap.is_some(),
                )
                .clicked()
                {
                    actions.push(Action::OpenDialog(
                        crate::views::dialogs::Dialog::new_branch_at(
                            crate::views::dialogs::Start::Head,
                        ),
                    ));
                }
                if widgets::tool_button(
                    ui,
                    icon::GIT_MERGE,
                    "Merge",
                    &tip("Merge or rebase", Command::MergeDialog),
                    branch.is_some() && !busy,
                )
                .clicked()
                {
                    actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Merge {
                        target: String::new(),
                        rebase: false,
                        mode: canopy_core::MergeMode::Default,
                    }));
                }
                let dirty = snap.is_some_and(|s| !s.status.is_clean());
                if widgets::tool_button(
                    ui,
                    icon::ARCHIVE,
                    "Stash",
                    &tip("Stash changes", Command::Stash),
                    dirty && !busy,
                )
                .clicked()
                {
                    actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Stash {
                        message: String::new(),
                        untracked: true,
                    }));
                }
                let latest = snap.and_then(|s| s.stashes.first());
                let pop_tip = match latest {
                    Some(st) => format!(
                        "Apply \u{201c}{}\u{201d} and remove it from the stash list",
                        st.message
                    ),
                    None => "Apply the latest stash and remove it from the stash list".to_owned(),
                };
                if widgets::tool_button(
                    ui,
                    icon::TRAY_ARROW_UP,
                    "Pop",
                    &tip(&pop_tip, Command::PopStash),
                    latest.is_some() && !busy,
                )
                .clicked()
                {
                    actions.push(Action::Op(Op::StashApply {
                        index: 0,
                        pop: true,
                    }));
                }
                let tool_width = if ui.ctx().content_rect().width() < theme::COMPACT_WIDTH {
                    48.0
                } else {
                    58.0
                };
                let show_local =
                    local_only && ui.ctx().content_rect().width() >= theme::COMPACT_WIDTH;
                let local_width = if show_local {
                    ui.painter()
                        .layout_no_wrap(
                            format!("{}  Local only", icon::HARD_DRIVES),
                            FontId::proportional(theme::META_SIZE),
                            p.text_faint,
                        )
                        .size()
                        .x
                        + theme::SM
                } else {
                    0.0
                };
                let right_width = theme::SM
                    + (tool_width + ui.spacing().item_spacing.x)
                        * if forge.is_some() { 3.0 } else { 2.0 }
                    + local_width;
                ui.add_space(
                    (width - (ui.cursor().min.x - ui.max_rect().min.x) - right_width).max(0.0),
                );
                ui.add_space(theme::SM);
                if widgets::tool_button(
                    ui,
                    icon::TERMINAL_WINDOW,
                    "Terminal",
                    "Open a terminal here",
                    true,
                )
                .clicked()
                {
                    actions.push(Action::OpenTerminal(tab.path.clone()));
                }
                if widgets::tool_button(
                    ui,
                    icon::FOLDER_OPEN,
                    "Files",
                    "Show in file manager",
                    true,
                )
                .clicked()
                {
                    actions.push(Action::OpenPath(tab.path.clone()));
                }
                if let Some((_, f)) = forge
                    && widgets::tool_button(
                        ui,
                        forge_icon(f.kind),
                        f.kind.name(),
                        &format!("Open {} on {}", f.path, f.kind.name()),
                        true,
                    )
                    .clicked()
                {
                    actions.push(Action::OpenUrl(f.web_url()));
                }
                if show_local {
                    ui.label(
                        RichText::new(format!("{}  Local only", icon::HARD_DRIVES))
                            .size(theme::META_SIZE)
                            .color(p.text_faint),
                    )
                    .on_hover_text(
                        "No remote: everything stays on this computer until you publish",
                    );
                }
            });
        });
}

fn segmented(
    ui: &mut Ui,
    view: View,
    changes: usize,
    requests: Option<(canopy_core::forge::ForgeKind, usize)>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let items = vec![
        (View::History, icon::GRAPH, "History", 0, Command::History),
        (
            View::Changes,
            icon::PENCIL_SIMPLE_LINE,
            "Changes",
            changes,
            Command::Changes,
        ),
    ];
    let (kind, n) = requests.unwrap_or((canopy_core::forge::ForgeKind::GitHub, 0));
    let mut items = items;
    items.push((
        View::Requests,
        icon::GIT_PULL_REQUEST,
        match kind {
            canopy_core::forge::ForgeKind::GitHub => "Pull requests",
            canopy_core::forge::ForgeKind::GitLab => "Merge requests",
        },
        n,
        Command::Requests,
    ));
    let compact = ui.ctx().content_rect().width() < theme::COMPACT_WIDTH;
    // Compact: only the active view keeps its name; the others show their glyph and count.
    let caption = |v: View, glyph: &str, label: &str| {
        if compact && v != view {
            glyph.to_owned()
        } else if compact && v == View::Requests {
            format!("{glyph}  Requests")
        } else {
            format!("{glyph}  {label}")
        }
    };
    let widths: Vec<f32> = items
        .iter()
        .map(|(v, glyph, label, count, _)| {
            let text = ui.painter().layout_no_wrap(
                caption(*v, glyph, label),
                FontId::new(theme::BODY_SIZE, theme::medium()),
                p.text,
            );
            text.size().x + if *count > 0 { 26.0 } else { 0.0 } + theme::SM * 2.0
        })
        .collect();
    let (outer, _) =
        ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 6.0, 34.0), Sense::hover());
    ui.painter().rect(
        outer,
        CornerRadius::same(10),
        p.bg(),
        Stroke::new(1.0, p.border()),
        egui::StrokeKind::Inside,
    );
    let index = items.iter().position(|item| item.0 == view).unwrap_or(0);
    let x = ui.ctx().animate_value_with_time(
        egui::Id::new("seg-view-x"),
        widths[..index].iter().sum(),
        theme::fade(ui.ctx(), 0.2),
    );
    let w = ui.ctx().animate_value_with_time(
        egui::Id::new("seg-view-width"),
        widths[index],
        theme::fade(ui.ctx(), 0.2),
    );
    let thumb = egui::Rect::from_min_size(
        outer.min + vec2(3.0 + x, 3.0),
        vec2(w, outer.height() - 6.0),
    );
    ui.painter()
        .rect_filled(thumb, CornerRadius::same(8), p.elevated());
    ui.painter().rect_stroke(
        thumb,
        CornerRadius::same(8),
        Stroke::new(1.0, Palette::tint(p.accent, 120)),
        egui::StrokeKind::Inside,
    );
    let mut x = 3.0;
    for (i, (v, glyph, label, count, command)) in items.into_iter().enumerate() {
        let w = widths[i];
        let r = egui::Rect::from_min_size(outer.min + vec2(x, 3.0), vec2(w, outer.height() - 6.0));
        x += w;
        let enabled = v != View::Requests || requests.is_some();
        let resp = ui.interact(
            r,
            egui::Id::new(("seg", i)),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let active = v == view;
        let color = if !enabled {
            p.text_faint
        } else if active {
            p.text
        } else if resp.hovered() {
            p.text_dim.lerp_to_gamma(p.text, 0.5)
        } else {
            p.text_dim
        };
        let text = caption(v, glyph, label);
        let galley = ui.painter().layout_no_wrap(
            text,
            FontId::new(theme::BODY_SIZE, theme::medium()),
            color,
        );
        let badge_w = if count > 0 { 26.0 } else { 0.0 };
        let start = r.center().x - (galley.size().x + badge_w) / 2.0;
        ui.painter().galley(
            egui::pos2(start, r.center().y - galley.size().y / 2.0),
            galley.clone(),
            color,
        );
        if count > 0 {
            let br = egui::Rect::from_center_size(
                egui::pos2(start + galley.size().x + 15.0, r.center().y),
                vec2(22.0, 16.0),
            );
            ui.painter()
                .rect_filled(br, CornerRadius::same(8), p.button());
            ui.painter().text(
                br.center(),
                Align2::CENTER_CENTER,
                if count > 99 {
                    "99+".to_owned()
                } else {
                    count.to_string()
                },
                FontId::new(theme::MICRO_SIZE, theme::semibold()),
                p.on_button(),
            );
        }
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, active, label)
        });
        let resp = resp.on_hover_text(if enabled {
            tip(label, command)
        } else {
            "Add a GitHub or GitLab remote to view pull or merge requests.".to_owned()
        });
        if enabled && resp.clicked() {
            actions.push(Action::SetView(v));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_labels_fit_their_controls_with_counts() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1200.0, 100.0),
                )),
                ..Default::default()
            },
            |ui| {
                segmented(
                    ui,
                    View::Requests,
                    123,
                    Some((canopy_core::forge::ForgeKind::GitLab, 123)),
                    &mut Vec::new(),
                )
            },
        );
        output.textures_delta.clear();
        for (index, label) in ["History", "Changes", "Merge requests"]
            .into_iter()
            .enumerate()
        {
            let rect = ctx
                .read_response(egui::Id::new(("seg", index)))
                .expect("view control")
                .rect;
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.text().contains(label) => {
                        Some(text)
                    }
                    _ => None,
                })
                .expect("view label");
            assert!(
                rect.contains_rect(text.galley.rect.translate(text.pos.to_vec2())),
                "{label} overflows its control"
            );
        }
    }
}
