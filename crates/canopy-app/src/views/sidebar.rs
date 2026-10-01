//! The left sidebar: branches, remotes, tags and stashes.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::app::Action;
use crate::tab::RepoTab;
use crate::ui::theme::{self, Palette, palette};
use crate::views::dialogs::Dialog;
use crate::worker::Op;

const ROW_H: f32 = 26.0;

fn section(
    ui: &mut Ui,
    tab: &mut RepoTab,
    key: &str,
    title: &str,
    glyph: &str,
    count: usize,
) -> bool {
    let p = palette(ui.ctx());
    let open = !tab.collapsed.contains(key);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let chevron = if open {
        icon::CARET_DOWN
    } else {
        icon::CARET_RIGHT
    };
    let color = if resp.hovered() {
        p.text_dim
    } else {
        p.text_faint
    };
    ui.painter().text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::CENTER_CENTER,
        chevron,
        FontId::proportional(11.0),
        color,
    );
    ui.painter().text(
        rect.left_center() + vec2(22.0, 0.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(13.0),
        color,
    );
    ui.painter().text(
        rect.left_center() + vec2(34.0, 0.0),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        FontId::new(10.5, theme::semibold()),
        color,
    );
    ui.painter().text(
        rect.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        count.to_string(),
        FontId::new(11.0, theme::medium()),
        p.text_faint,
    );
    if resp.clicked() {
        if open {
            tab.collapsed.insert(key.to_owned());
        } else {
            tab.collapsed.remove(key);
        }
    }
    open
}

struct Row<'a> {
    glyph: &'a str,
    label: &'a str,
    indent: f32,
    strong: bool,
    color: egui::Color32,
    ahead: usize,
    behind: usize,
    hint: Option<&'a str>,
}

fn row(ui: &mut Ui, r: Row<'_>) -> egui::Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    if r.strong {
        ui.painter().rect_filled(
            rect.shrink2(vec2(4.0, 1.0)),
            CornerRadius::same(theme::RADIUS_SM),
            Palette::tint(p.accent, if p.dark { 40 } else { 28 }),
        );
    } else if hover > 0.0 {
        ui.painter().rect_filled(
            rect.shrink2(vec2(4.0, 1.0)),
            CornerRadius::same(theme::RADIUS_SM),
            Palette::tint(p.text, (hover * 12.0) as u8),
        );
    }
    let x = rect.min.x + 14.0 + r.indent;
    ui.painter().text(
        egui::pos2(x + 6.0, rect.center().y),
        Align2::CENTER_CENTER,
        r.glyph,
        FontId::proportional(13.5),
        r.color,
    );
    let mut right = rect.max.x - 10.0;
    for (n, arrow, c) in [(r.behind, "↓", p.accent2), (r.ahead, "↑", p.success)] {
        if n > 0 {
            let t = format!("{arrow}{n}");
            let g = ui
                .painter()
                .layout_no_wrap(t, FontId::new(10.5, theme::semibold()), c);
            let br = egui::Rect::from_min_max(
                egui::pos2(right - g.size().x - 10.0, rect.center().y - 8.0),
                egui::pos2(right, rect.center().y + 8.0),
            );
            ui.painter()
                .rect_filled(br, CornerRadius::same(8), Palette::tint(c, 36));
            ui.painter().galley(br.center() - g.size() / 2.0, g, c);
            right = br.min.x - 4.0;
        }
    }
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(x + 18.0, rect.min.y),
        egui::pos2(right, rect.max.y),
    );
    let font = if r.strong {
        FontId::new(13.0, theme::semibold())
    } else {
        FontId::proportional(13.0)
    };
    let g = ui.painter().layout_no_wrap(
        r.label.to_owned(),
        font,
        if r.strong {
            p.text
        } else {
            p.text_dim.lerp_to_gamma(p.text, 0.45 + hover * 0.4)
        },
    );
    ui.painter().with_clip_rect(text_rect).galley(
        egui::pos2(text_rect.min.x, rect.center().y - g.size().y / 2.0),
        g,
        p.text,
    );
    match r.hint {
        Some(h) => resp.on_hover_text(h),
        None => resp,
    }
}

/// Draws the sidebar.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(snap) = tab.snap.clone() else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.spinner());
        return;
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(8.0);

        // Branches.
        if section(ui, tab, "branches", "Branches", icon::GIT_BRANCH, snap.branches.len()) {
            for b in &snap.branches {
                let color = if b.is_head { p.accent } else { p.text_dim };
                let glyph = if b.is_head { icon::CHECK_CIRCLE } else { icon::GIT_BRANCH };
                let hint = b.upstream.as_ref().map(|u| format!("Tracks {u}")).unwrap_or_else(|| "No upstream".to_owned());
                let resp = row(ui, Row { glyph, label: &b.name, indent: 6.0, strong: b.is_head, color, ahead: b.ahead, behind: b.behind, hint: Some(&hint) });
                if resp.clicked()
                    && let Some(t) = b.target {
                        actions.push(Action::Reveal(t));
                    }
                if resp.double_clicked() && !b.is_head {
                    actions.push(Action::Op(Op::Checkout(b.name.clone())));
                }
                let name = b.name.clone();
                let is_head = b.is_head;
                resp.context_menu(|ui| {
                    if !is_head && ui.button(format!("{}  Check out", icon::CHECK)).clicked() {
                        actions.push(Action::Op(Op::Checkout(name.clone())));
                    }
                    if !is_head && ui.button(format!("{}  Merge into current branch", icon::GIT_MERGE)).clicked() {
                        actions.push(Action::Op(Op::Merge(name.clone())));
                    }
                    if !is_head && ui.button(format!("{}  Rebase current branch onto this", icon::GIT_PULL_REQUEST)).clicked() {
                        actions.push(Action::Confirm { title: "Rebase".into(), body: format!("Rebase the current branch onto {name}? This rewrites the current branch's commits."), confirm: "Rebase".into(), danger: false, op: Op::Rebase(name.clone()) });
                    }
                    ui.separator();
                    if ui.button(format!("{}  New branch from here…", icon::GIT_BRANCH)).clicked() {
                        actions.push(Action::OpenDialog(Dialog::new_branch(Some(name.clone()))));
                    }
                    if ui.button(format!("{}  Rename…", icon::PENCIL_SIMPLE)).clicked() {
                        actions.push(Action::OpenDialog(Dialog::Rename { old: name.clone(), new: name.clone() }));
                    }
                    if is_head && ui.button(format!("{}  Push", icon::ARROW_FAT_LINE_UP)).clicked() {
                        actions.push(Action::Op(Op::Push { force: false }));
                    }
                    if is_head && ui.button(format!("{}  Force push (with lease)…", icon::WARNING)).clicked() {
                        actions.push(Action::Confirm { title: "Force push".into(), body: format!("Overwrite the remote copy of {name} with your local branch? --force-with-lease refuses if someone else pushed in the meantime."), confirm: "Force push".into(), danger: true, op: Op::Push { force: true } });
                    }
                    if ui.button(format!("{}  Copy name", icon::COPY)).clicked() {
                        actions.push(Action::Copy(name.clone()));
                    }
                    if !is_head {
                        ui.separator();
                        if ui.button(RichText::new(format!("{}  Delete…", icon::TRASH)).color(p.danger)).clicked() {
                            actions.push(Action::Confirm { title: "Delete branch".into(), body: format!("Delete the local branch {name}? Unmerged commits stay reachable only through the reflog."), confirm: "Delete".into(), danger: true, op: Op::DeleteBranch { name: name.clone(), force: true } });
                        }
                    }
                });
            }
            ui.add_space(8.0);
        }

        // Remotes.
        if section(ui, tab, "remotes", "Remotes", icon::CLOUD, snap.remotes.len()) {
            for r in &snap.remotes {
                let key = format!("remote:{}", r.name);
                let open = !tab.collapsed.contains(&key);
                let resp = row(ui, Row { glyph: if open { icon::CARET_DOWN } else { icon::CARET_RIGHT }, label: &r.name, indent: 6.0, strong: false, color: p.accent2, ahead: 0, behind: 0, hint: r.url.as_deref() });
                if resp.clicked() {
                    if open {
                        tab.collapsed.insert(key.clone());
                    } else {
                        tab.collapsed.remove(&key);
                    }
                }
                let rname = r.name.clone();
                let url = r.url.clone().unwrap_or_default();
                resp.context_menu(|ui| {
                    if ui.button(format!("{}  Fetch", icon::ARROWS_CLOCKWISE)).clicked() {
                        actions.push(Action::Op(Op::Fetch));
                    }
                    if ui.button(format!("{}  Copy URL", icon::COPY)).clicked() {
                        actions.push(Action::Copy(url.clone()));
                    }
                    ui.separator();
                    if ui.button(RichText::new(format!("{}  Remove remote…", icon::TRASH)).color(p.danger)).clicked() {
                        actions.push(Action::Confirm { title: "Remove remote".into(), body: format!("Remove the remote {rname} and its remote-tracking branches?"), confirm: "Remove".into(), danger: true, op: Op::RemoveRemote(rname.clone()) });
                    }
                });
                if !open {
                    continue;
                }
                for b in snap.remote_branches.iter().filter(|b| b.remote == r.name) {
                    let resp = row(ui, Row { glyph: icon::GIT_BRANCH, label: &b.branch, indent: 22.0, strong: false, color: p.text_faint, ahead: 0, behind: 0, hint: Some("Double-click to check out") });
                    if resp.clicked()
                        && let Some(t) = b.target {
                            actions.push(Action::Reveal(t));
                        }
                    let (remote, branch) = (b.remote.clone(), b.branch.clone());
                    if resp.double_clicked() {
                        actions.push(Action::Op(Op::CheckoutRemote { remote: remote.clone(), branch: branch.clone() }));
                    }
                    resp.context_menu(|ui| {
                        if ui.button(format!("{}  Check out as local branch", icon::CHECK)).clicked() {
                            actions.push(Action::Op(Op::CheckoutRemote { remote: remote.clone(), branch: branch.clone() }));
                        }
                        if ui.button(format!("{}  Merge into current branch", icon::GIT_MERGE)).clicked() {
                            actions.push(Action::Op(Op::Merge(format!("{remote}/{branch}"))));
                        }
                        if ui.button(format!("{}  Copy name", icon::COPY)).clicked() {
                            actions.push(Action::Copy(format!("{remote}/{branch}")));
                        }
                        ui.separator();
                        if ui.button(RichText::new(format!("{}  Delete on remote…", icon::TRASH)).color(p.danger)).clicked() {
                            actions.push(Action::Confirm { title: "Delete remote branch".into(), body: format!("Delete {branch} on {remote}? Everyone loses access to it."), confirm: "Delete".into(), danger: true, op: Op::DeleteRemoteBranch { remote: remote.clone(), branch: branch.clone() } });
                        }
                    });
                }
            }
            if ui.add(egui::Button::new(RichText::new(format!("{}  Add remote", icon::PLUS)).size(12.0).color(p.text_faint)).frame(false)).clicked() {
                actions.push(Action::OpenDialog(Dialog::AddRemote { name: "origin".into(), url: String::new() }));
            }
            ui.add_space(8.0);
        }

        // Tags.
        if section(ui, tab, "tags", "Tags", icon::TAG, snap.tags.len()) {
            for t in &snap.tags {
                let resp = row(ui, Row { glyph: icon::TAG, label: &t.name, indent: 6.0, strong: false, color: p.warning, ahead: 0, behind: 0, hint: t.message.as_deref() });
                if resp.clicked()
                    && let Some(target) = t.target {
                        actions.push(Action::Reveal(target));
                    }
                let name = t.name.clone();
                let target = t.target;
                let remote = snap.remotes.first().map(|r| r.name.clone());
                resp.context_menu(|ui| {
                    if let Some(oid) = target
                        && ui.button(format!("{}  Check out (detached)", icon::CHECK)).clicked() {
                            actions.push(Action::Op(Op::CheckoutDetached(oid)));
                        }
                    if let Some(r) = &remote
                        && ui.button(format!("{}  Push to {r}", icon::ARROW_FAT_LINE_UP)).clicked() {
                            actions.push(Action::Op(Op::PushTag { remote: r.clone(), tag: name.clone() }));
                        }
                    if ui.button(format!("{}  Copy name", icon::COPY)).clicked() {
                        actions.push(Action::Copy(name.clone()));
                    }
                    ui.separator();
                    if ui.button(RichText::new(format!("{}  Delete…", icon::TRASH)).color(p.danger)).clicked() {
                        actions.push(Action::Confirm { title: "Delete tag".into(), body: format!("Delete the local tag {name}?"), confirm: "Delete".into(), danger: true, op: Op::DeleteTag(name.clone()) });
                    }
                });
            }
            ui.add_space(8.0);
        }

        // Stashes.
        if section(ui, tab, "stashes", "Stashes", icon::ARCHIVE, snap.stashes.len()) {
            for s in &snap.stashes {
                let label = s.message.trim_start_matches("On ").to_owned();
                let resp = row(ui, Row { glyph: icon::ARCHIVE, label: &label, indent: 6.0, strong: false, color: p.text_dim, ahead: 0, behind: 0, hint: Some(&s.message) });
                let index = s.index;
                if resp.clicked() {
                    actions.push(Action::Reveal(s.oid));
                }
                resp.context_menu(|ui| {
                    if ui.button(format!("{}  Apply", icon::TRAY_ARROW_DOWN)).clicked() {
                        actions.push(Action::Op(Op::StashApply { index, pop: false }));
                    }
                    if ui.button(format!("{}  Pop", icon::TRAY_ARROW_UP)).clicked() {
                        actions.push(Action::Op(Op::StashApply { index, pop: true }));
                    }
                    ui.separator();
                    if ui.button(RichText::new(format!("{}  Drop…", icon::TRASH)).color(p.danger)).clicked() {
                        actions.push(Action::Confirm { title: "Drop stash".into(), body: format!("Drop stash@{{{index}}}? Its changes are lost."), confirm: "Drop".into(), danger: true, op: Op::StashDrop(index) });
                    }
                });
            }
        }
        ui.add_space(16.0);
    });
}
