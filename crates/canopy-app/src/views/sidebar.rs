//! The left sidebar: branches, remotes, tags and stashes.

use egui::{FontId, RichText, Ui};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::RepoTab;
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::Badge;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Skeleton;
use crate::ui::kit::row::{ListRow, RowKind, SectionHeader, TreeRow};
use crate::ui::kit::{KitResponse, Tone, menu};
use crate::ui::theme::{self, palette};
use crate::ui::widgets::forge_icon;
use crate::views::commit_detail::virtual_rows;
use crate::views::dialogs::Dialog;
use crate::worker::Op;
use canopy_core::MergeMode;

/// Gap between a row's label and its badges, and between two badges.
const BADGE_GAP: f32 = theme::XS;

/// A collapsible section header bound to `tab.collapsed[key]`; returns whether it is open.
fn section(
    ui: &mut Ui,
    tab: &mut RepoTab,
    key: &str,
    title: &str,
    glyph: &str,
    count: usize,
    first: bool,
) -> bool {
    section_with_action(ui, tab, key, title, glyph, count, first, None).0
}

/// [`section`] with an icon button (glyph, tooltip) that shows on hover and focus; also returns
/// the header's response (for its context menu) and whether the button was clicked.
#[expect(
    clippy::too_many_arguments,
    reason = "a section header needs its tab, key, title, glyph, count, place and action"
)]
fn section_with_action(
    ui: &mut Ui,
    tab: &mut RepoTab,
    key: &str,
    title: &str,
    glyph: &str,
    count: usize,
    first: bool,
    action: Option<(&str, &str)>,
) -> (bool, KitResponse<bool>) {
    let mut open = !tab.collapsed.contains(key);
    let mut header = SectionHeader::new(title)
        .icon(glyph)
        .count(count)
        .open(&mut open)
        .first(first);
    if let Some((glyph, label)) = action {
        header = header.action(glyph, label);
    }
    let shown = header.show(ui);
    if open {
        tab.collapsed.remove(key);
    } else {
        tab.collapsed.insert(key.to_owned());
    }
    (open, shown)
}

/// The Remotes section of a repository with none: says so, and offers to publish it or add one.
fn no_remotes(ui: &mut Ui, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.add_space(theme::XS);
    ui.horizontal(|ui| {
        ui.add_space(theme::MD);
        ui.label(
            RichText::new(format!("{}  Local only — no remote", icon::HARD_DRIVES))
                .size(m.meta_size)
                .color(p.text_faint),
        )
        .kit_tooltip("Everything stays on this computer until you publish it or add a remote")
        .show();
    });
    ui.add_space(theme::XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SM;
        ui.add_space(theme::MD);
        if LinkButton::new("Publish…")
            .text_size(m.meta_size)
            .show(ui)
            .kit_tooltip("Create a repository on GitHub or GitLab and push this one to it")
            .show()
            .clicked()
        {
            actions.push(Action::Publish);
        }
        ui.label(RichText::new("·").size(m.meta_size).color(p.text_faint));
        if LinkButton::new("Add remote…")
            .text_size(m.meta_size)
            .show(ui)
            .kit_tooltip("Connect a repository that already exists elsewhere")
            .show()
            .clicked()
        {
            actions.push(Action::AddRemoteDialog);
        }
    });
    ui.add_space(theme::XS);
}

struct Row<'a> {
    /// Identifies the row across frames (hover, context menu); rows come and go while scrolling.
    id: egui::Id,
    glyph: &'a str,
    label: &'a str,
    /// The checked-out branch: shown selected, its icon in the accent.
    current: bool,
    ahead: usize,
    behind: usize,
    hint: Option<&'a str>,
}

/// A sidebar item: icon and label, the checked-out branch selected, ahead and behind counts as
/// badges at the right.
fn row(ui: &mut Ui, r: &Row<'_>) -> egui::Response {
    let m = theme::metrics();
    let badges: Vec<Badge> = [(r.ahead, "↑", Tone::Accent), (r.behind, "↓", Tone::Neutral)]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, arrow, tone)| Badge::new(format!("{arrow}{n}")).tone(tone).mono())
        .collect();
    let badges_w = badges
        .iter()
        .map(|b| b.measure(ui).x + BADGE_GAP)
        .sum::<f32>();
    // A shortened name is shown in full on hover.
    let label_w = ui
        .painter()
        .layout_no_wrap(
            r.label.to_owned(),
            FontId::new(m.body_size, theme::semibold()),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x;
    let room = ui.available_width() - 2.0 * theme::MD - m.icon_md - theme::SM;
    let tip = r
        .hint
        .map(str::to_owned)
        .or_else(|| (label_w > room - badges_w).then(|| r.label.to_owned()));
    let mut row = ListRow::new(r.id)
        .kind(RowKind::Sidebar)
        .selected(r.current)
        .icon(r.glyph)
        .label(r.label);
    if let Some(t) = tip {
        row = row.tooltip(t);
    }
    if !badges.is_empty() {
        // Right to left: the behind count at the edge, the ahead count before it.
        row = row.trailing_ui(badges_w, move |ui| {
            ui.spacing_mut().item_spacing.x = BADGE_GAP;
            for b in badges.into_iter().rev() {
                b.show(ui);
            }
        });
    }
    row.show(ui)
}

/// Draws the sidebar.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let row_h = RowKind::Sidebar.height();
    let Some(snap) = tab.snap.clone() else {
        // The first load: placeholder rows, after a moment so a quick load never flashes them.
        ui.add_space(theme::XS);
        Skeleton::rows(8)
            .row_height(row_h)
            .id_salt("sidebar-loading")
            .show(ui)
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Loading the repository")
            });
        return;
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(theme::XS);

        // Branches.
        let unborn = snap.head.branch.as_ref().filter(|name| {
            snap.head.oid.is_none() && !snap.branches.iter().any(|branch| &branch.name == *name)
        });
        if section(
            ui,
            tab,
            "branches",
            "Branches",
            icon::GIT_BRANCH,
            snap.branches.len() + usize::from(unborn.is_some()),
            true,
        ) {
            if let Some(name) = unborn {
                let label = format!("{name} (no commits yet)");
                let _ = row(ui, &Row { id: ui.id().with("unborn-branch"), glyph: icon::CHECK_CIRCLE, label: &label, current: true, ahead: 0, behind: 0, hint: Some("Current branch; make the first commit to create it") });
            }
            virtual_rows(ui, snap.branches.len(), row_h, |ui, i| {
                let b = &snap.branches[i];
                let glyph = if b.is_head { icon::CHECK_CIRCLE } else { icon::GIT_BRANCH };
                let hint = b.upstream.as_ref().map_or_else(|| "No upstream".to_owned(), |u| format!("Tracks {u}"));
                let resp = row(ui, &Row { id: ui.id().with(("branch", &b.name)), glyph, label: &b.name, current: b.is_head, ahead: b.ahead, behind: b.behind, hint: Some(&hint) });
                if resp.clicked()
                    && let Some(t) = b.target {
                        actions.push(Action::Reveal(t));
                    }
                if resp.double_clicked() && !b.is_head {
                    actions.push(Action::Op(Op::Checkout(b.name.clone())));
                }
                let name = b.name.clone();
                let is_head = b.is_head;
                // The branch's page on GitHub/GitLab, through its upstream remote.
                let upstream_forge = b.upstream.as_deref().and_then(|u| {
                    let (remote, rest) = u.split_once('/')?;
                    Some((tab.forge_of(remote)?.clone(), rest.to_owned()))
                });
                menu::context(&resp, |m| {
                    if let Some((f, remote_branch)) = &upstream_forge {
                        if m.item(forge_icon(f.kind), format!("Open on {}", f.kind.name())).show(m.ui).clicked() {
                            actions.push(Action::OpenUrl(f.branch_url(remote_branch)));
                        }
                        if m.item(icon::GIT_PULL_REQUEST, format!("Create {}…", f.kind.request_noun())).show(m.ui).clicked() {
                            actions.push(Action::CreateRequest { source: name.clone() });
                        }
                        m.separator();
                    }
                    if !is_head {
                        if m.item(icon::CHECK, "Check out").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::Checkout(name.clone())));
                        }
                        if m.item(icon::GIT_MERGE, "Merge into current branch").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::Merge(name.clone(), MergeMode::Default)));
                        }
                        if m.item(icon::GIT_PULL_REQUEST, "Rebase current branch onto this").show(m.ui).clicked() {
                            actions.push(Action::Confirm { title: "Rebase".into(), body: format!("Rebase the current branch onto {name}? This rewrites the current branch's commits."), confirm: "Rebase".into(), danger: false, op: Op::Rebase(name.clone()) });
                        }
                        m.separator();
                    }
                    if m.item(icon::GIT_BRANCH, "New branch from here…").show(m.ui).clicked() {
                        actions.push(Action::OpenDialog(Dialog::new_branch_at(crate::views::dialogs::Start::Branch(name.clone()))));
                    }
                    if m.item(icon::PENCIL_SIMPLE, "Rename…").show(m.ui).clicked() {
                        actions.push(Action::OpenDialog(Dialog::Rename { old: name.clone(), new: name.clone() }));
                    }
                    if is_head && m.item(icon::ARROW_FAT_LINE_UP, "Push").shortcut(Command::Push).show(m.ui).clicked() {
                        actions.push(Action::Op(Op::Push { force: false }));
                    }
                    if m.item(icon::COPY, "Copy name").show(m.ui).clicked() {
                        actions.push(Action::Copy(name.clone()));
                    }
                    m.separator();
                    if is_head {
                        if m.item(icon::WARNING, "Force push (with lease)…").danger().show(m.ui).clicked() {
                            actions.push(Action::Confirm { title: "Force push".into(), body: format!("Overwrite the remote copy of {name} with your local branch? --force-with-lease refuses if someone else pushed in the meantime."), confirm: "Force push".into(), danger: true, op: Op::Push { force: true } });
                        }
                    } else if m.item(icon::TRASH, "Delete…").danger().show(m.ui).clicked() {
                        actions.push(Action::Confirm { title: "Delete branch".into(), body: format!("Delete the local branch {name}? If it has commits that are not merged, Git refuses and Canopy asks before forcing it."), confirm: "Delete".into(), danger: true, op: Op::DeleteBranch { name: name.clone(), force: false } });
                    }
                });
            });
        }

        // Remotes: "+" on the header (hover or focus) and its context menu add one; a
        // repository with none says so and offers Publish and Add remote as links instead, so
        // the header has no second "Add remote…".
        let local_only = snap.remotes.is_empty();
        let (open, header) = section_with_action(ui, tab, "remotes", "Remotes", icon::CLOUD, snap.remotes.len(), false, (!local_only).then_some((icon::PLUS, "Add remote…")));
        if header.value {
            actions.push(Action::AddRemoteDialog);
        }
        menu::context(&header.response, |m| {
            if m.item(icon::PLUS, "Add remote…").show(m.ui).clicked() {
                actions.push(Action::AddRemoteDialog);
            }
            if local_only {
                if m.item(icon::CLOUD_ARROW_UP, "Publish…").show(m.ui).clicked() {
                    actions.push(Action::Publish);
                }
            } else if m.item(icon::ARROWS_CLOCKWISE, "Fetch all remotes").shortcut(Command::Fetch).show(m.ui).clicked() {
                actions.push(Action::Fetch);
            }
        });
        if open {
            if local_only {
                no_remotes(ui, actions);
            }
            for (ri, r) in snap.remotes.iter().enumerate() {
                let key = format!("remote:{}", r.name);
                let mut open = !tab.collapsed.contains(&key);
                let forge = tab.forge_of(&r.name).cloned();
                let label = match &forge {
                    Some(f) => format!("{}  {}", r.name, forge_icon(f.kind)),
                    None => r.name.clone(),
                };
                let shown = TreeRow::new(("remote", &r.name), 0)
                    .open(&mut open)
                    .toggle_on_click(true)
                    .label(label)
                    .show(ui);
                if open {
                    tab.collapsed.remove(&key);
                } else {
                    tab.collapsed.insert(key.clone());
                }
                let resp = match r.url.as_deref() {
                    Some(url) => shown.response.kit_tooltip(url).show(),
                    None => shown.response,
                };
                let rname = r.name.clone();
                let url = r.url.clone().unwrap_or_default();
                menu::context(&resp, |m| {
                    if let Some(f) = &forge
                        && m.item(forge_icon(f.kind), format!("Open on {}", f.kind.name())).show(m.ui).clicked() {
                            actions.push(Action::OpenUrl(f.web_url()));
                        }
                    if m.item(icon::ARROWS_CLOCKWISE, "Fetch").shortcut(Command::Fetch).show(m.ui).clicked() {
                        actions.push(Action::Fetch);
                    }
                    if m.item(icon::COPY, "Copy URL").show(m.ui).clicked() {
                        actions.push(Action::Copy(url.clone()));
                    }
                    m.separator();
                    if m.item(icon::TRASH, "Remove remote…").danger().show(m.ui).clicked() {
                        actions.push(Action::Confirm { title: "Remove remote".into(), body: format!("Remove the remote {rname} and its remote-tracking branches?"), confirm: "Remove".into(), danger: true, op: Op::RemoveRemote(rname.clone()) });
                    }
                });
                if !open {
                    continue;
                }
                let branches: &[usize] = tab.snap_index.remote_branches.get(ri).map_or(&[], Vec::as_slice);
                virtual_rows(ui, branches.len(), row_h, |ui, i| {
                    let b = &snap.remote_branches[branches[i]];
                    let resp = TreeRow::new(("remote-branch", &b.remote, &b.branch), 1)
                        .icon(icon::GIT_BRANCH)
                        .label(b.branch.clone())
                        .show(ui)
                        .response
                        .kit_tooltip("Double-click to check out").show();
                    if resp.clicked()
                        && let Some(t) = b.target {
                            actions.push(Action::Reveal(t));
                        }
                    let (remote, branch) = (b.remote.clone(), b.branch.clone());
                    if resp.double_clicked() {
                        actions.push(Action::Op(Op::CheckoutRemote { remote: remote.clone(), branch: branch.clone() }));
                    }
                    let rb_forge = forge.clone();
                    menu::context(&resp, |m| {
                        if let Some(f) = &rb_forge
                            && m.item(forge_icon(f.kind), format!("Open on {}", f.kind.name())).show(m.ui).clicked() {
                                actions.push(Action::OpenUrl(f.branch_url(&branch)));
                            }
                        if m.item(icon::CHECK, "Check out as local branch").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::CheckoutRemote { remote: remote.clone(), branch: branch.clone() }));
                        }
                        if m.item(icon::GIT_MERGE, "Merge into current branch").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::Merge(format!("{remote}/{branch}"), MergeMode::Default)));
                        }
                        if m.item(icon::COPY, "Copy name").show(m.ui).clicked() {
                            actions.push(Action::Copy(format!("{remote}/{branch}")));
                        }
                        m.separator();
                        if m.item(icon::TRASH, "Delete on remote…").danger().show(m.ui).clicked() {
                            actions.push(Action::Confirm { title: "Delete remote branch".into(), body: format!("Delete {branch} on {remote}? Everyone loses access to it."), confirm: "Delete".into(), danger: true, op: Op::DeleteRemoteBranch { remote: remote.clone(), branch: branch.clone() } });
                        }
                    });
                });
            }
        }

        // Tags.
        if section(ui, tab, "tags", "Tags", icon::TAG, snap.tags.len(), false) {
            let remote = snap.remotes.first().map(|r| r.name.clone());
            let tag_forge = tab.forge().map(|(_, f)| f.clone());
            virtual_rows(ui, snap.tags.len(), row_h, |ui, i| {
                let t = &snap.tags[i];
                let resp = row(ui, &Row { id: ui.id().with(("tag", &t.name)), glyph: icon::TAG, label: &t.name, current: false, ahead: 0, behind: 0, hint: t.message.as_deref() });
                if resp.clicked()
                    && let Some(target) = t.target {
                        actions.push(Action::Reveal(target));
                    }
                let name = t.name.clone();
                let target = t.target;
                menu::context(&resp, |m| {
                    if let Some(f) = &tag_forge {
                        let what = if f.kind == canopy_core::forge::ForgeKind::GitHub { "release" } else { "tag" };
                        if m.item(forge_icon(f.kind), format!("Open {what} on {}", f.kind.name())).show(m.ui).clicked() {
                            actions.push(Action::OpenUrl(f.tag_url(&name)));
                        }
                    }
                    if let Some(oid) = target
                        && m.item(icon::CHECK, "Check out (detached)").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::CheckoutDetached(oid)));
                        }
                    if let Some(r) = &remote
                        && m.item(icon::ARROW_FAT_LINE_UP, format!("Push to {r}")).show(m.ui).clicked() {
                            actions.push(Action::Op(Op::PushTag { remote: r.clone(), tag: name.clone() }));
                        }
                    if m.item(icon::COPY, "Copy name").show(m.ui).clicked() {
                        actions.push(Action::Copy(name.clone()));
                    }
                    m.separator();
                    if m.item(icon::TRASH, "Delete…").danger().show(m.ui).clicked() {
                        actions.push(Action::Confirm { title: "Delete tag".into(), body: format!("Delete the local tag {name}?"), confirm: "Delete".into(), danger: true, op: Op::DeleteTag(name.clone()) });
                    }
                });
            });
        }

        // Stashes.
        if section(ui, tab, "stashes", "Stashes", icon::ARCHIVE, snap.stashes.len(), false) {
            virtual_rows(ui, snap.stashes.len(), row_h, |ui, i| {
                let s = &snap.stashes[i];
                let label = s.message.trim_start_matches("On ").to_owned();
                let resp = row(ui, &Row { id: ui.id().with(("stash", s.oid)), glyph: icon::ARCHIVE, label: &label, current: false, ahead: 0, behind: 0, hint: Some(&s.message) });
                let index = s.index;
                if resp.clicked() {
                    actions.push(Action::Reveal(s.oid));
                }
                menu::context(&resp, |m| {
                    if m.item(icon::TRAY_ARROW_DOWN, "Apply").show(m.ui).clicked() {
                        actions.push(Action::Op(Op::StashApply { index, pop: false }));
                    }
                    if m.item(icon::TRAY_ARROW_UP, "Pop").show(m.ui).clicked() {
                        actions.push(Action::Op(Op::StashApply { index, pop: true }));
                    }
                    m.separator();
                    if m.item(icon::TRASH, "Drop…").danger().show(m.ui).clicked() {
                        actions.push(Action::Confirm { title: "Drop stash".into(), body: format!("Drop stash@{{{index}}}? Its changes are lost."), confirm: "Drop".into(), danger: true, op: Op::StashDrop(index) });
                    }
                });
            });
        }
        ui.add_space(theme::LG);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AccessKit labels of everything one frame of `add` shows.
    fn spoken(add: impl FnMut(&mut Ui)) -> Vec<String> {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        let mut add = add;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| add(ui));
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| n.label().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_row_is_announced_by_its_label() {
        let labels = spoken(|ui| {
            row(
                ui,
                &Row {
                    id: egui::Id::new("row"),
                    glyph: icon::GIT_BRANCH,
                    label: "feature/auth",
                    current: false,
                    ahead: 2,
                    behind: 1,
                    hint: None,
                },
            );
        });
        assert!(labels.iter().any(|l| l == "feature/auth"), "{labels:?}");
    }

    fn tab_with(remotes: Vec<canopy_core::RemoteInfo>) -> RepoTab {
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot {
            remotes,
            ..Default::default()
        }));
        tab
    }

    fn origin() -> canopy_core::RemoteInfo {
        canopy_core::RemoteInfo {
            name: "origin".into(),
            url: Some("https://example.com/r.git".into()),
            default_branch: None,
        }
    }

    /// One frame of the sidebar with `events`: the AccessKit nodes it reported.
    fn sidebar_frame(
        ctx: &egui::Context,
        tab: &mut RepoTab,
        events: Vec<egui::Event>,
        actions: &mut Vec<Action>,
    ) -> Vec<egui::accesskit::Node> {
        sidebar_frame_focus(ctx, tab, events, actions).0
    }

    /// [`sidebar_frame`], and the label of the node that has keyboard focus.
    fn sidebar_frame_focus(
        ctx: &egui::Context,
        tab: &mut RepoTab,
        events: Vec<egui::Event>,
        actions: &mut Vec<Action>,
    ) -> (Vec<egui::accesskit::Node>, Option<String>) {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ui| show(ui, tab, actions),
        );
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .map(|u| {
                let focused = u
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == u.focus)
                    .and_then(|(_, n)| n.label().map(str::to_owned));
                (u.nodes.into_iter().map(|(_, n)| n).collect(), focused)
            })
            .unwrap_or_default()
    }

    fn center(n: &egui::accesskit::Node) -> egui::Pos2 {
        let b = n.bounds().expect("bounds");
        egui::pos2(
            crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
            crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
        )
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let mut events = vec![egui::Event::PointerMoved(at)];
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        events
    }

    fn named<'a>(
        nodes: &'a [egui::accesskit::Node],
        label: &str,
    ) -> Option<&'a egui::accesskit::Node> {
        nodes.iter().find(|n| n.label() == Some(label))
    }

    /// O9: a repository with no remote offers Publish and Add remote as quiet links, not
    /// full-width buttons.
    #[test]
    fn a_repository_without_remotes_offers_publish_and_add_remote_links() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        let mut tab = tab_with(Vec::new());
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        for label in ["Publish…", "Add remote…"] {
            let n = named(&nodes, label).unwrap_or_else(|| panic!("{label}"));
            assert_eq!(n.role(), egui::accesskit::Role::Link, "{label}");
        }
        let at = center(named(&nodes, "Publish…").expect("publish"));
        let _ = sidebar_frame(&ctx, &mut tab, click(at), &mut actions);
        assert!(
            matches!(actions.as_slice(), [Action::Publish]),
            "{actions:?}"
        );
    }

    /// O9: with remotes the Remotes header's "+" is reachable at rest (keyboard and screen
    /// readers, kit-polish Override O2), shows on hover, and a click on it opens the dialog.
    #[test]
    fn the_remotes_header_adds_a_remote_from_its_hover_button() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = tab_with(vec![origin()]);
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert_eq!(
            named(&nodes, "Add remote…").map(egui::accesskit::Node::role),
            Some(egui::accesskit::Role::Button),
            "the header's + is in the tree at rest"
        );
        let header = center(named(&nodes, "Remotes").expect("remotes header"));
        let hover = vec![egui::Event::PointerMoved(header)];
        let _ = sidebar_frame(&ctx, &mut tab, hover.clone(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, hover, &mut actions);
        let add = center(named(&nodes, "Add remote…").expect("the header's + on hover"));
        let _ = sidebar_frame(&ctx, &mut tab, click(add), &mut actions);
        crate::ui::motion::set_reduced(false);
        assert!(
            matches!(actions.as_slice(), [Action::AddRemoteDialog]),
            "{actions:?}"
        );
    }
}
