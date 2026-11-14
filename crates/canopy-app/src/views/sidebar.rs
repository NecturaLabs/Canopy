//! The left sidebar: branches, worktrees, remotes, tags, stashes and submodules.

use egui::accesskit::Role;
use egui::{FontId, RichText, Ui};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::{RepoTab, SidebarItem};
use crate::ui::kit::chip::Badge;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Skeleton;
use crate::ui::kit::row::{
    ListRow, Roving, RowKind, SectionHeader, TreeNav, TreeRow, section_item_indent,
    section_tree_indent,
};
use crate::ui::kit::{Button, KitResponse, Size, Tone, a11y, menu};
use crate::ui::theme::{self, palette};
use crate::ui::widgets::forge_icon;
use crate::views::commit_detail::virtual_rows;
use crate::views::dialogs::Dialog;
use crate::worker::Op;
use canopy_core::{MergeMode, SubmoduleState};

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
    tab.set_sidebar_open(key, open);
    (open, shown)
}

/// A quiet empty Remotes section, with the same compact outlined actions as the tree's tools.
fn no_remotes(ui: &mut Ui, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.add_space(theme::XS);
    egui::Frame::new()
        .inner_margin(egui::Margin::same(crate::ui::points_i8(theme::MD)))
        .outer_margin(egui::Margin::symmetric(crate::ui::points_i8(theme::SM), 0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = theme::SM;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SM;
                crate::views::commit_detail::without_icons(
                    ui.label(
                        RichText::new(icon::CLOUD_SLASH)
                            .size(m.icon_md)
                            .color(p.text_dim),
                    ),
                );
                ui.label(
                    RichText::new("Local only")
                        .size(m.body_size)
                        .color(p.text_dim),
                );
            });
            ui.add(
                egui::Label::new(
                    RichText::new("No remote connected to this repository.")
                        .size(m.meta_size)
                        .color(p.text_dim),
                )
                .wrap(),
            );
            ui.add_space(theme::XS);
            if Button::new("Publish…")
                .secondary()
                .size(Size::Sm)
                .icon(icon::CLOUD_ARROW_UP)
                .full_width()
                .tooltip("Connect an existing remote repository and push your commits")
                .show(ui)
                .clicked()
            {
                actions.push(Action::Publish);
            }
            if Button::new("Add remote…")
                .secondary()
                .size(Size::Sm)
                .icon(icon::PLUS)
                .full_width()
                .tooltip("Connect a repository that already exists elsewhere")
                .show(ui)
                .clicked()
            {
                actions.push(Action::AddRemoteDialog);
            }
        });
    ui.add_space(theme::SM);
}

struct Row<'a> {
    /// Identifies the row across frames (hover, context menu); rows come and go while scrolling.
    id: egui::Id,
    glyph: &'a str,
    label: &'a str,
    /// The row last clicked, while History shows its commit.
    selected: bool,
    /// The checked-out branch: its icon in the accent and its label semibold.
    current: bool,
    ahead: usize,
    behind: usize,
    hint: Option<&'a str>,
    /// Dim text after the label (a worktree's branch).
    detail: Option<&'a str>,
    /// What screen readers call the row when the label alone says too little.
    spoken: Option<&'a str>,
}

/// A sidebar item: icon and label, the picked row selected and the checked-out branch marked,
/// ahead and behind counts as badges at the right; row `index` of `list` when its section is one
/// Tab stop.
fn row(ui: &mut Ui, r: &Row<'_>, list: Option<(Roving, usize)>) -> egui::Response {
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
    let room =
        ui.available_width() - section_item_indent() - 2.0 * theme::MD - m.icon_md - theme::SM;
    let tip = r
        .hint
        .map(str::to_owned)
        .or_else(|| (label_w > room - badges_w).then(|| r.label.to_owned()));
    let mut row = ListRow::new(r.id)
        .kind(RowKind::Sidebar)
        .indent(section_item_indent())
        .selected(r.selected)
        .current(r.current)
        .icon(r.glyph)
        .label(r.label);
    if let Some(d) = r.detail {
        row = row.detail(d).whole_detail();
    }
    if let Some(name) = r.spoken {
        row = row.accessible_name(name);
    }
    if let Some(t) = tip {
        row = row.tooltip(t);
    }
    if let Some((list, index)) = list {
        row = row.roving(list, index);
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
    // The row clicked this frame, picked once the sections are drawn.
    let mut pick = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(theme::XS);

        // Branches.
        let unborn = snap.head.branch.as_ref().filter(|name| {
            snap.head.oid.is_none() && !snap.branches.iter().any(|branch| &branch.name == *name)
        });
        let (branches_open, branches_header) = section_with_action(
            ui,
            tab,
            "branches",
            "Branches",
            icon::GIT_BRANCH,
            snap.branches.len() + usize::from(unborn.is_some()),
            true,
            None,
        );
        let flow = snap.workflow.flow.as_ref();
        let head_flow = snap
            .head
            .branch
            .as_deref()
            .zip(flow)
            .and_then(|(name, f)| f.classify(name).map(|(kind, _)| (kind, name.to_owned())));
        menu::context(&branches_header.response, |m| {
            if m.item(icon::GIT_MERGE, "Clean up merged branches…").show(m.ui).clicked() {
                actions.push(Action::CleanMerged);
            }
            if m.item(icon::TRASH, "Clean up stale…").show(m.ui).clicked() {
                actions.push(Action::CleanStale);
            }
            m.separator();
            if flow.is_none() {
                if m.item(icon::TREE_STRUCTURE, "Set up git-flow…").show(m.ui).clicked() {
                    actions.push(Action::FlowInit);
                }
            } else {
                if m.item(icon::TREE_STRUCTURE, "Start a git-flow branch…").show(m.ui).clicked() {
                    actions.push(Action::FlowStart(None));
                }
                if let Some((kind, name)) = &head_flow
                    && m.item(icon::CHECK_CIRCLE, format!("Finish {} {name}…", kind.label().to_lowercase())).show(m.ui).clicked() {
                        actions.push(Action::FlowFinish(Some(name.clone())));
                    }
            }
        });
        if branches_open {
            let tree = &tab.snap_index.branch_tree;
            let shown = crate::branch_tree::visible(tree, &tab.collapsed);
            let merged = tab.merged.as_ref().and_then(|m| m.as_ref().ok());
            let kept: &[String] = snap.workflow.base.as_ref().map_or(&[], |b| &b.protected);
            // One Tab stop for the branches: the checked-out one until the user moves.
            let first = usize::from(unborn.is_some());
            let head = shown.iter().position(|s| matches!(&tree[s.node], crate::branch_tree::Node::Branch { index, .. } if snap.branches[*index].is_head)).map(|i| i + first);
            let list = Roving::new(ui.ctx(), ("sidebar-branches", &tab.path), shown.len() + first, head.or(Some(0)));
            let mut toggles: Vec<(String, bool)> = Vec::new();
            ui.scope(|ui| {
            if shown.len() + first > 0 { a11y::group(ui, if tree.is_empty() { Role::List } else { Role::Tree }, "Branches"); }
            if let Some(name) = unborn {
                let label = format!("{name} (no commits yet)");
                let _ = row(ui, &Row { id: ui.id().with("unborn-branch"), glyph: icon::CHECK_CIRCLE, label: &label, selected: false, current: true, ahead: 0, behind: 0, hint: Some("Current branch; make the first commit to create it"), detail: None, spoken: None }, Some((list, 0)));
            }
            virtual_rows(ui, shown.len(), row_h, |ui, vi| {
                let at = vi + first;
                let (node, parent) = (&tree[shown[vi].node], shown[vi].parent);
                let (b, depth, leaf) = match node {
                    crate::branch_tree::Node::Folder { path, name, depth, branches, has_head, .. } => {
                        let key = crate::branch_tree::folder_key(path);
                        let mut open = !tab.collapsed.contains(&key);
                        let glyph = if open { icon::FOLDER_OPEN } else { icon::FOLDER_SIMPLE };
                        let spoken = format!("{path}, folder of {branches} {}", if *branches == 1 { "branch" } else { "branches" });
                        let mut row = TreeRow::new(("branch-folder", path), *depth)
                            .indent(section_tree_indent())
                            .open(&mut open)
                            .toggle_on_click(true)
                            .icon(glyph)
                            .label(name.clone())
                            .trailing(branches.to_string())
                            .accessible_name(spoken)
                            .tooltip(path.clone())
                            .roving(list, at);
                        if *has_head {
                            row = row.icon_tone(Tone::Accent);
                        }
                        let shown_row = row.show(ui);
                        if shown_row.value == Some(TreeNav::FirstChild) {
                            list.focus(ui.ctx(), at + 1);
                        }
                        if shown_row.value == Some(TreeNav::Parent)
                            && let Some(parent) = parent {
                                list.focus(ui.ctx(), parent + first);
                            }
                        if open == tab.collapsed.contains(&key) {
                            toggles.push((key, open));
                        }
                        return;
                    }
                    crate::branch_tree::Node::Branch { index, name, depth } => (&snap.branches[*index], *depth, name),
                };
                // A branch checked out in another worktree (listed under Worktrees) lives there:
                // git refuses to check it out here, so it opens that worktree instead.
                let worktree = b.worktree.as_ref().filter(|_| !b.is_head);
                // The base and the long-lived branches never carry a merged marker.
                let reason = merged.and_then(|m| m.reason(&b.name)).filter(|_| !kept.contains(&b.name));
                let mut hint = b.upstream.as_ref().map_or_else(|| "No upstream".to_owned(), |u| format!("Tracks {u}"));
                if let Some(w) = worktree {
                    hint = format!("Checked out in the worktree {} · {hint}", folder_name(w));
                }
                if let Some(r) = reason {
                    hint = format!("{} · {hint}", r.label());
                }
                let (glyph, tone) = if b.is_head {
                    (icon::CHECK_CIRCLE, Tone::Accent)
                } else {
                    match reason {
                        Some(canopy_core::MergedReason::Reachable) => (icon::GIT_MERGE, Tone::Success),
                        Some(canopy_core::MergedReason::SquashMerged) => (icon::GIT_MERGE, Tone::Warning),
                        Some(canopy_core::MergedReason::UpstreamGone) => (icon::CLOUD_SLASH, Tone::Warning),
                        None => (icon::GIT_BRANCH, Tone::Neutral),
                    }
                };
                let mut notes: Vec<String> = Vec::new();
                if b.ahead > 0 { notes.push(format!("↑{}", b.ahead)); }
                if b.behind > 0 { notes.push(format!("↓{}", b.behind)); }
                if let Some(r) = reason { notes.push(merged_word(r).to_owned()); }
                let mut spoken = b.name.clone();
                if let Some(r) = reason { spoken = format!("{spoken}, {}", merged_word(r)); }
                let selected = tab.sidebar_shows(|p| matches!(p, SidebarItem::Branch(n) if *n == b.name), b.target);
                let mut tree_row = TreeRow::new(("branch", &b.name), depth)
                    .indent(section_tree_indent())
                    .icon(glyph)
                    .icon_tone(tone)
                    .label(leaf.clone())
                    .accessible_name(spoken)
                    .selected(selected)
                    .tooltip(hint)
                    .roving(list, at);
                if !notes.is_empty() {
                    tree_row = tree_row.trailing(notes.join(" "));
                }
                let shown_row = tree_row.show(ui);
                // Left goes back up to the folder.
                if shown_row.value == Some(TreeNav::Parent)
                    && let Some(parent) = parent {
                        list.focus(ui.ctx(), parent + first);
                    }
                let resp = shown_row.response;
                if b.is_head {
                    // Announced as the current branch, apart from being selected.
                    ui.ctx().accesskit_node_builder(resp.id, |n| n.set_aria_current(egui::accesskit::AriaCurrent::True));
                }
                if resp.clicked()
                    && let Some(t) = b.target {
                        pick = Some(SidebarItem::Branch(b.name.clone()));
                        actions.push(Action::Reveal(t));
                    }
                if resp.double_clicked() && !b.is_head {
                    actions.push(match worktree {
                        Some(w) => Action::SwitchWorktree(w.clone()),
                        None => Action::Op(Op::Checkout(b.name.clone())),
                    });
                }
                let name = b.name.clone();
                let is_head = b.is_head;
                let head_base = is_head.then(|| snap.workflow.head_base.clone()).flatten();
                let finish_kind = snap.workflow.flow.as_ref().and_then(|f| f.classify(&b.name)).map(|(kind, _)| kind);
                let has_upstream = b.upstream.is_some();
                let remotes: Vec<String> = snap.remotes.iter().map(|r| r.name.clone()).collect();
                let worktree = worktree.cloned();
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
                        if let Some(w) = &worktree {
                            if m.item(icon::FOLDER_OPEN, "Open worktree").show(m.ui).clicked() {
                                actions.push(Action::SwitchWorktree(w.clone()));
                            }
                        } else if m.item(icon::CHECK, "Check out").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::Checkout(name.clone())));
                        }
                        if m.item(icon::GIT_MERGE, "Merge into current branch").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::Merge(name.clone(), MergeMode::Default)));
                        }
                        if m.item(icon::GIT_PULL_REQUEST, "Rebase current branch onto this").show(m.ui).clicked() {
                            actions.push(Action::Confirm { title: format!("Rebase onto {name}?"), body: format!("The current branch's commits are rewritten on top of {name}."), confirm: "Rebase branch".into(), danger: false, op: Op::Rebase(name.clone()) });
                        }
                        m.separator();
                    }
                    if let Some(base) = &head_base
                        && m.item(icon::ARROW_FAT_LINES_DOWN, format!("Update from {base}…")).show(m.ui).clicked() {
                            actions.push(Action::UpdateFromBase);
                        }
                    if let Some(kind) = finish_kind
                        && m.item(icon::FLAG_CHECKERED, format!("Finish {}…", kind.label().to_lowercase())).show(m.ui).clicked() {
                            actions.push(Action::FlowFinish(Some(name.clone())));
                        }
                    if m.item(icon::GIT_BRANCH, "New branch from here…").show(m.ui).clicked() {
                        actions.push(Action::OpenDialog(Dialog::new_branch_at(crate::views::dialogs::Start::Branch(name.clone()))));
                    }
                    if m.item(icon::PENCIL_SIMPLE, "Rename…").show(m.ui).clicked() {
                        actions.push(Action::OpenDialog(Dialog::Rename { old: name.clone(), new: name.clone() }));
                    }
                    if is_head && m.item(icon::ARROW_FAT_LINE_UP, "Push").shortcut(Command::Push).show(m.ui).clicked() {
                        actions.push(Action::Op(Op::Push { force: false, expect: None }));
                    }
                    // Any local branch without an upstream, checked out or not.
                    if !has_upstream {
                        for r in &remotes {
                            let label = format!("Publish to {r}");
                            if m.item(icon::CLOUD_ARROW_UP, label).show(m.ui).clicked() {
                                actions.push(Action::Op(Op::PublishBranch { name: name.clone(), remote: r.clone() }));
                            }
                        }
                    }
                    if m.item(icon::COPY, "Copy name").show(m.ui).clicked() {
                        actions.push(Action::Copy(name.clone()));
                    }
                    m.separator();
                    if is_head {
                        if m.item(icon::WARNING, "Force push (with lease)…").danger().show(m.ui).clicked() {
                            actions.push(Action::ForcePush);
                        }
                    } else if worktree.is_none() && m.item(icon::TRASH, "Delete…").danger().show(m.ui).clicked() {
                        actions.push(Action::Confirm { title: format!("Delete branch {name}?"), body: "Only the local branch is deleted. If it has commits that are not merged, Git refuses and Canopy asks before forcing it.".into(), confirm: "Delete branch".into(), danger: true, op: Op::DeleteBranch { name: name.clone(), force: false } });
                    }
                });
            });
            });
            for (key, open) in toggles {
                tab.set_sidebar_open(&key, open);
            }
        }

        // Worktrees: the repository's working folders, once `git worktree add` made a second.
        if snap.worktrees.len() > 1
            && section(ui, tab, "worktrees", "Worktrees", icon::FOLDERS, snap.worktrees.len(), true)
        {
            let here = snap.worktrees.iter().position(|w| w.current);
            let list = Roving::new(ui.ctx(), ("sidebar-worktrees", &tab.path), snap.worktrees.len(), here.or(Some(0)));
            ui.scope(|ui| {
            a11y::group(ui, Role::List, "Worktrees");
            for (i, w) in snap.worktrees.iter().enumerate() {
                let name = folder_name(&w.path);
                let on = w.branch.clone().unwrap_or_else(|| match w.head {
                    Some(oid) => format!("detached at {}", canopy_core::short_id(oid)),
                    None => "detached".to_owned(),
                });
                let mut hint = format!("{} · on {on}", w.path.display());
                if w.main {
                    hint.push_str(" · main working folder");
                }
                if w.current {
                    hint.push_str(" · open here");
                }
                let spoken = format!("{name}, worktree on {on}");
                let glyph = if w.current { icon::CHECK_CIRCLE } else { icon::FOLDER_SIMPLE };
                let selected = tab.sidebar_shows(|p| matches!(p, SidebarItem::Worktree(path) if *path == w.path), w.head);
                let resp = row(ui, &Row { id: ui.id().with(("worktree", &w.path)), glyph, label: &name, selected, current: w.current, ahead: 0, behind: 0, hint: Some(&hint), detail: Some(&on), spoken: Some(&spoken) }, Some((list, i)));
                if resp.clicked()
                    && let Some(head) = w.head {
                        pick = Some(SidebarItem::Worktree(w.path.clone()));
                        actions.push(Action::Reveal(head));
                    }
                if resp.double_clicked() && !w.current {
                    actions.push(Action::SwitchWorktree(w.path.clone()));
                }
                let path = w.path.clone();
                let current = w.current;
                menu::context(&resp, |m| {
                    if !current && m.item(icon::FOLDER_OPEN, "Open worktree").show(m.ui).clicked() {
                        actions.push(Action::SwitchWorktree(path.clone()));
                    }
                    if m.item(icon::COPY, "Copy path").show(m.ui).clicked() {
                        actions.push(Action::Copy(path.display().to_string()));
                    }
                });
            }
            });
        }

        // Remotes: a repository with none says so and offers Publish and Add remote…; the header
        // has no action or menu of its own (Add remote… is in the palette too).
        let local_only = snap.remotes.is_empty();
        let open = section(ui, tab, "remotes", "Remotes", icon::CLOUD, snap.remotes.len(), false);
        if open {
            if local_only {
                no_remotes(ui, actions);
            }
            // The remotes and their branches are one tree and one Tab stop; the rows are
            // numbered in display order.
            let shown_rows: usize = snap.remotes.iter().enumerate().map(|(ri, r)| {
                1 + if tab.collapsed.contains(&format!("remote:{}", r.name)) { 0 } else { tab.snap_index.remote_branches.get(ri).map_or(0, Vec::len) }
            }).sum();
            let list = Roving::new(ui.ctx(), ("sidebar-remotes", &tab.path), shown_rows, Some(0));
            let mut at = 0;
            ui.scope(|ui| {
            if !local_only { a11y::group(ui, Role::Tree, "Remotes"); }
            for (ri, r) in snap.remotes.iter().enumerate() {
                let key = format!("remote:{}", r.name);
                let mut open = !tab.collapsed.contains(&key);
                let forge = tab.forge_of(&r.name).cloned();
                let (label, spoken) = match &forge {
                    Some(f) => (format!("{}  {}", r.name, forge_icon(f.kind)), format!("{}, {}", r.name, f.kind.name())),
                    None => (r.name.clone(), r.name.clone()),
                };
                let remote_at = at;
                let shown = TreeRow::new(("remote", &r.name), 0)
                    .indent(section_tree_indent())
                    .open(&mut open)
                    .toggle_on_click(true)
                    .label(label)
                    .accessible_name(spoken)
                    .roving(list, remote_at)
                    .show(ui);
                at += 1;
                if shown.value == Some(TreeNav::FirstChild) {
                    list.focus(ui.ctx(), remote_at + 1);
                }
                tab.set_sidebar_open(&key, open);
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
                        actions.push(Action::Confirm { title: format!("Remove remote {rname}?"), body: "Its remote-tracking branches go too. The repository on the server is not touched.".into(), confirm: "Remove remote".into(), danger: true, op: Op::RemoveRemote(rname.clone()) });
                    }
                });
                if !open {
                    continue;
                }
                let branches: &[usize] = tab.snap_index.remote_branches.get(ri).map_or(&[], Vec::as_slice);
                let base = at;
                at += branches.len();
                virtual_rows(ui, branches.len(), row_h, |ui, i| {
                    let b = &snap.remote_branches[branches[i]];
                    let selected = tab.sidebar_shows(|p| matches!(p, SidebarItem::RemoteBranch { remote, branch } if *remote == b.remote && *branch == b.branch), b.target);
                    let shown = TreeRow::new(("remote-branch", &b.remote, &b.branch), 1)
                        .indent(section_tree_indent())
                        .icon(icon::GIT_BRANCH)
                        .label(b.branch.clone())
                        .selected(selected)
                        .roving(list, base + i)
                        .show(ui);
                    // Left goes back up to the remote.
                    if shown.value == Some(TreeNav::Parent) {
                        list.focus(ui.ctx(), remote_at);
                    }
                    let resp = shown.response;
                    if resp.clicked()
                        && let Some(t) = b.target {
                            pick = Some(SidebarItem::RemoteBranch { remote: b.remote.clone(), branch: b.branch.clone() });
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
                            actions.push(Action::Confirm { title: format!("Delete {branch} on {remote}?"), body: "The branch is deleted on the server, and everyone loses access to it.".into(), confirm: "Delete remote branch".into(), danger: true, op: Op::DeleteRemoteBranch { remote: remote.clone(), branch: branch.clone() } });
                        }
                    });
                });
            }
            });
        }

        // Tags.
        if section(ui, tab, "tags", "Tags", icon::TAG, snap.tags.len(), false) {
            let remote = snap.remotes.first().map(|r| r.name.clone());
            let tag_forge = tab.forge().map(|(_, f)| f.clone());
            let list = Roving::new(ui.ctx(), ("sidebar-tags", &tab.path), snap.tags.len(), Some(0));
            ui.scope(|ui| {
            if !snap.tags.is_empty() { a11y::group(ui, Role::List, "Tags"); }
            virtual_rows(ui, snap.tags.len(), row_h, |ui, i| {
                let t = &snap.tags[i];
                let selected = tab.sidebar_shows(|p| matches!(p, SidebarItem::Tag(n) if *n == t.name), t.target);
                let resp = row(ui, &Row { id: ui.id().with(("tag", &t.name)), glyph: icon::TAG, label: &t.name, selected, current: false, ahead: 0, behind: 0, hint: t.message.as_deref(), detail: None, spoken: None }, Some((list, i)));
                if resp.clicked()
                    && let Some(target) = t.target {
                        pick = Some(SidebarItem::Tag(t.name.clone()));
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
                        actions.push(Action::Confirm { title: format!("Delete tag {name}?"), body: "Only the local tag is deleted; a copy already pushed stays on the remote.".into(), confirm: "Delete tag".into(), danger: true, op: Op::DeleteTag(name.clone()) });
                    }
                });
            });
            });
        }

        // Stashes.
        if section(ui, tab, "stashes", "Stashes", icon::ARCHIVE, snap.stashes.len(), false) {
            let list = Roving::new(ui.ctx(), ("sidebar-stashes", &tab.path), snap.stashes.len(), Some(0));
            ui.scope(|ui| {
            if !snap.stashes.is_empty() { a11y::group(ui, Role::List, "Stashes"); }
            virtual_rows(ui, snap.stashes.len(), row_h, |ui, i| {
                let s = &snap.stashes[i];
                let label = s.message.trim_start_matches("On ").to_owned();
                let selected = tab.sidebar_shows(|p| *p == SidebarItem::Stash(s.oid), Some(s.oid));
                let resp = row(ui, &Row { id: ui.id().with(("stash", s.oid)), glyph: icon::ARCHIVE, label: &label, selected, current: false, ahead: 0, behind: 0, hint: Some(&s.message), detail: None, spoken: None }, Some((list, i)));
                let oid = s.oid;
                if resp.clicked() {
                    pick = Some(SidebarItem::Stash(oid));
                    actions.push(Action::Reveal(oid));
                }
                // By the stash's commit, never its place in the list, which can change before
                // the operation runs.
                menu::context(&resp, |m| {
                    if m.item(icon::TRAY_ARROW_DOWN, "Apply").show(m.ui).clicked() {
                        actions.push(Action::Op(Op::StashApplyCommit { oid, pop: false }));
                    }
                    if m.item(icon::TRAY_ARROW_UP, "Pop").show(m.ui).clicked() {
                        actions.push(Action::Op(Op::StashApplyCommit { oid, pop: true }));
                    }
                    m.separator();
                    if m.item(icon::TRASH, "Drop…").danger().show(m.ui).clicked() {
                        actions.push(drop_stash(s));
                    }
                });
            });
            });
        }

        // Submodules: only in a repository that has some; "+" adds one, the palette too.
        // A removal that is staged but not committed is no longer a submodule to act on.
        let submodules = tab.snap_index.submodules.len();
        if submodules > 0 {
            let (open, header) = section_with_action(ui, tab, "submodules", "Submodules", icon::PACKAGE, submodules, false, Some((icon::PLUS, "Add submodule…")));
            if header.value {
                actions.push(Action::OpenDialog(Dialog::AddSubmodule { url: String::new(), path: String::new(), branch: String::new() }));
            }
            menu::context(&header.response, |m| {
                if m.item(icon::PLUS, "Add submodule…").show(m.ui).clicked() {
                    actions.push(Action::OpenDialog(Dialog::AddSubmodule { url: String::new(), path: String::new(), branch: String::new() }));
                }
                if m.item(icon::ARROWS_CLOCKWISE, "Update all").show(m.ui).clicked() {
                    actions.push(Action::Op(Op::SubmoduleUpdate { path: None, init: true }));
                }
                if m.item(icon::ARROWS_CLOCKWISE, "Sync all URLs").show(m.ui).clicked() {
                    actions.push(Action::Op(Op::SubmoduleSync(None)));
                }
            });
            if open {
                let list = Roving::new(ui.ctx(), ("sidebar-submodules", &tab.path), submodules, Some(0));
                ui.scope(|ui| {
                a11y::group(ui, Role::List, "Submodules");
                virtual_rows(ui, submodules, row_h, |ui, i| {
                    let (index, state) = tab.snap_index.submodules[i];
                    let sm = &snap.submodules[index];
                    let commit = sm.workdir_id.or(sm.index_id).map(canopy_core::short_id);
                    let detail = match state {
                        SubmoduleState::Clean => commit.clone().unwrap_or_default(),
                        _ => state.label().to_owned(),
                    };
                    let glyph = if state == SubmoduleState::Conflicted { icon::WARNING } else { icon::PACKAGE };
                    let name = folder_name(std::path::Path::new(&sm.path));
                    let mut hint = format!("{} · {}", sm.path, sm.url.as_deref().unwrap_or("no URL"));
                    if let Some(c) = &commit {
                        hint = format!("{hint} · {c}");
                    }
                    let spoken = format!("{}, submodule, {}", sm.path, state.label());
                    let resp = row(ui, &Row { id: ui.id().with(("submodule", &sm.path)), glyph, label: &name, selected: false, current: false, ahead: 0, behind: 0, hint: Some(&hint), detail: Some(&detail), spoken: Some(&spoken) }, Some((list, i)));
                    let initialized = sm.is_initialized();
                    let folder = tab.path.join(&sm.path);
                    if resp.double_clicked() && initialized {
                        actions.push(Action::OpenRepo(folder.clone()));
                    }
                    let (path, url) = (sm.path.clone(), sm.url.clone());
                    menu::context(&resp, |m| {
                        if initialized && m.item(icon::FOLDER_OPEN, "Open in a tab").show(m.ui).clicked() {
                            actions.push(Action::OpenRepo(folder.clone()));
                        }
                        let verb = if initialized { "Update to the recorded commit" } else { "Initialize and update" };
                        if m.item(icon::ARROWS_CLOCKWISE, verb).show(m.ui).clicked() {
                            actions.push(Action::Op(Op::SubmoduleUpdate { path: Some(path.clone()), init: !initialized }));
                        }
                        if m.item(icon::ARROWS_CLOCKWISE, "Sync URL").show(m.ui).clicked() {
                            actions.push(Action::Op(Op::SubmoduleSync(Some(path.clone()))));
                        }
                        if m.item(icon::COPY, "Copy path").show(m.ui).clicked() {
                            actions.push(Action::Copy(path.clone()));
                        }
                        if let Some(url) = &url
                            && m.item(icon::COPY, "Copy URL").show(m.ui).clicked() {
                                actions.push(Action::Copy(url.clone()));
                            }
                        m.separator();
                        if m.item(icon::TRASH, "Remove…").danger().show(m.ui).clicked() {
                            actions.push(remove_submodule(&path));
                        }
                    });
                });
                });
            }
        }
        ui.add_space(theme::LG);
    });
    if pick.is_some() {
        tab.sidebar_pick = pick;
    }
    if tab.sidebar_changed {
        if let Some(repo) = tab.sidebar_key() {
            actions.push(Action::RememberSidebar {
                repo: repo.to_path_buf(),
                closed: tab.collapsed.iter().cloned().collect(),
            });
        }
        tab.sidebar_changed = false;
    }
}

/// The short word a merged branch's row shows after its name; the tooltip has the full check.
fn merged_word(reason: canopy_core::MergedReason) -> &'static str {
    match reason {
        canopy_core::MergedReason::Reachable => "merged",
        canopy_core::MergedReason::UpstreamGone => "gone",
        canopy_core::MergedReason::SquashMerged => "squash?",
    }
}

/// A worktree's folder name, as its row and tooltips call it.
fn folder_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The confirmation for removing the submodule at `path`: it deletes the folder and the
/// submodule's own repository.
fn remove_submodule(path: &str) -> Action {
    Action::Confirm {
        title: format!("Remove submodule {path}?"),
        body: "The folder and the submodule's own repository are deleted, including uncommitted changes and commits that were never pushed. The removal is staged, not committed.".into(),
        confirm: "Remove submodule".into(),
        danger: true,
        op: Op::SubmoduleRemove(path.to_owned()),
    }
}

/// The confirmation for dropping `stash`: it names the stash by its message and drops it by its
/// commit.
fn drop_stash(stash: &canopy_core::StashInfo) -> Action {
    Action::Confirm {
        title: format!(
            "Drop stash \u{201c}{}\u{201d}?",
            stash.message.trim_start_matches("On ")
        ),
        body: format!(
            "Its changes are lost. The stash is commit {}.",
            canopy_core::short_id(stash.oid)
        ),
        confirm: "Drop stash".into(),
        danger: true,
        op: Op::StashDrop(stash.oid),
    }
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
                    selected: false,
                    current: false,
                    ahead: 2,
                    behind: 1,
                    hint: None,
                    detail: None,
                    spoken: None,
                },
                None,
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
        // A new tab starts with all but Branches closed; these fixtures show every section.
        tab.collapsed.clear();
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
        // These fixtures assign snapshots directly instead of receiving a worker event.
        if let Some(snap) = &tab.snap {
            tab.snap_index = crate::tab::SnapIndex::new(snap);
        }
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

    /// A repository with no remote says so in a card offering Publish and Add remote as buttons
    /// (the owner overruled O9's quiet links as low effort).
    #[test]
    fn a_repository_without_remotes_offers_publish_and_add_remote() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        let mut tab = tab_with(Vec::new());
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        for label in ["Publish…", "Add remote…"] {
            let n = named(&nodes, label).unwrap_or_else(|| panic!("{label}"));
            assert_eq!(n.role(), Role::Button, "{label}");
        }
        let at = center(named(&nodes, "Publish…").expect("publish"));
        let _ = sidebar_frame(&ctx, &mut tab, click(at), &mut actions);
        assert!(
            matches!(actions.as_slice(), [Action::Publish]),
            "{actions:?}"
        );
    }

    /// #138: the Remotes header has no "+" and no context menu; "Add remote…" stays in the
    /// palette and the local-only block.
    #[test]
    fn the_remotes_header_has_no_button_and_no_menu() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = tab_with(vec![origin()]);
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let header =
            center(named(&nodes, &a11y::disclosure_name("Remotes", true)).expect("remotes header"));
        assert!(
            named(&nodes, "Add remote…").is_none(),
            "no + on the header at rest"
        );
        let hover = vec![egui::Event::PointerMoved(header)];
        let _ = sidebar_frame(&ctx, &mut tab, hover.clone(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, hover, &mut actions);
        assert!(named(&nodes, "Add remote…").is_none(), "none on hover");
        let right_click = [true, false].map(|pressed| egui::Event::PointerButton {
            pos: header,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        let _ = sidebar_frame(&ctx, &mut tab, right_click.to_vec(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        crate::ui::motion::set_reduced(false);
        for label in ["Add remote…", "Fetch all remotes", "Publish…"] {
            assert!(named(&nodes, label).is_none(), "a menu opened with {label}");
        }
        assert!(actions.is_empty(), "{actions:?}");
    }

    /// #138: a new tab starts with every section closed except Branches, and a section the user
    /// opens stays open.
    #[test]
    fn a_new_tab_opens_only_branches_and_keeps_the_users_choice() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.snap = branch_tab().snap;
        let sub = canopy_core::SubmoduleInfo {
            name: "deps/lib".into(),
            path: "deps/lib".into(),
            path_os: "deps/lib".into(),
            url: None,
            head_id: Some(oid(7)),
            index_id: Some(oid(7)),
            workdir_id: Some(oid(7)),
            conflicted: false,
            dirty: None,
        };
        if let Some(snap) = tab.snap.as_mut() {
            std::sync::Arc::make_mut(snap).submodules = vec![sub];
        }
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        // Where the adapter speaks the expanded state, the name is the bare label for both
        // states, so the node's own expanded property is what tells them apart.
        let state = |nodes: &[egui::accesskit::Node], title: &str| {
            [true, false].into_iter().find_map(|open| {
                named(nodes, &a11y::disclosure_name(title, open))
                    .map(|n| n.is_expanded().unwrap_or(open))
            })
        };
        assert_eq!(state(&nodes, "Branches"), Some(true));
        for title in ["Worktrees", "Remotes", "Tags", "Stashes", "Submodules"] {
            assert_eq!(state(&nodes, title), Some(false), "{title}");
        }

        // The user opens Tags and closes Branches: both stay as chosen.
        for title in ["Tags", "Branches"] {
            let open = state(&nodes, title) == Some(true);
            let at = center(named(&nodes, &a11y::disclosure_name(title, open)).expect(title));
            let _ = sidebar_frame(&ctx, &mut tab, click(at), &mut actions);
            let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
            assert_eq!(state(&nodes, title), Some(!open), "{title} toggled");
        }
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        crate::ui::motion::set_reduced(false);
        assert_eq!(state(&nodes, "Tags"), Some(true));
        assert_eq!(state(&nodes, "Branches"), Some(false));
        assert_eq!(state(&nodes, "Remotes"), Some(false));
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::RememberSidebar { closed, .. }
            if closed.contains("branches") && !closed.contains("tags"))),
            "the user's toggle is queued for persistence"
        );
        actions.clear();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert!(
            actions.is_empty(),
            "drawing steady state does not write preferences"
        );
    }

    /// The text a painted shape holds, with where it is.
    fn painted_text(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                out.push((t.galley.text().to_owned(), t.visual_bounding_rect()));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| painted_text(s, out)),
            _ => {}
        }
    }

    /// #139: the rows of a section sit right of its header's icon in every section, and a
    /// remote's branches one level right of the remote.
    #[test]
    fn items_sit_right_of_their_section_headers() {
        let ctx = crate::ui::kit::testkit::ctx();
        ctx.enable_accesskit();
        let mut tab = branch_tab();
        tab.collapsed.clear();
        let remote = tab_with(vec![origin()]).snap.expect("snapshot");
        if let Some(snap) = tab.snap.as_mut() {
            let snap = std::sync::Arc::make_mut(snap);
            snap.remotes = remote.remotes.clone();
            snap.remote_branches = vec![canopy_core::RemoteBranchInfo {
                remote: "origin".into(),
                branch: "main".into(),
                target: Some(oid(1)),
            }];
            snap.tags = vec![canopy_core::TagInfo {
                name: "v1".into(),
                target: Some(oid(1)),
                message: None,
            }];
            snap.stashes = vec![canopy_core::StashInfo {
                index: 0,
                message: "On main: wip".into(),
                oid: oid(5),
            }];
        }
        tab.snap_index = crate::tab::SnapIndex::new(tab.snap.as_ref().expect("snapshot"));
        let mut actions = Vec::new();
        let mut texts = Vec::new();
        for _ in 0..2 {
            let shapes = crate::ui::kit::testkit::frame(&ctx, Vec::new(), |ui| {
                ui.set_max_size(egui::vec2(260.0, 700.0));
                show(ui, &mut tab, &mut actions);
            });
            texts.clear();
            for c in &shapes {
                painted_text(&c.shape, &mut texts);
            }
        }
        // The glyphs of `glyph`, top to bottom.
        let lefts = |glyph: &str| -> Vec<f32> {
            let mut found: Vec<_> = texts.iter().filter(|(t, _)| t == glyph).collect();
            found.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
            found.into_iter().map(|(_, r)| r.left()).collect()
        };
        let left_of = |label: &str| {
            texts
                .iter()
                .find(|(t, _)| t == label)
                .unwrap_or_else(|| panic!("{label}: {texts:?}"))
                .1
                .left()
        };
        let step = theme::metrics().tree_indent / 2.0;
        // Branches: the header's glyph first, then the rows' (branch and remote branch).
        let branch = lefts(icon::GIT_BRANCH);
        assert!(branch.len() >= 3, "{branch:?}");
        let (header, rows) = branch.split_first().expect("glyphs");
        assert!(rows[0] > header + step, "branches {header} {rows:?}");
        let worktrees = lefts(icon::FOLDERS)[0];
        assert!(
            lefts(icon::FOLDER_SIMPLE)[0] > worktrees + step,
            "worktrees"
        );
        let tags = lefts(icon::TAG);
        assert!(tags[1] > tags[0] + step, "tags {tags:?}");
        let stashes = lefts(icon::ARCHIVE);
        assert!(stashes[1] > stashes[0] + step, "stashes {stashes:?}");
        // Remotes: the remote's name lines up with the items, its branch is one level further.
        let cloud = lefts(icon::CLOUD)[0];
        let origin = left_of("origin");
        assert!(origin > cloud + step, "remote {cloud} {origin}");
        let remote_branch = rows.last().copied().expect("remote branch glyph");
        assert!(
            remote_branch > origin + step,
            "remote branch {origin} {remote_branch}"
        );
    }

    /// A11Y-3: each section's rows are one Tab stop (the checked-out branch for Branches), the
    /// arrows move between them, and the rows are announced inside a list named after the section.
    #[test]
    fn each_section_is_one_tab_stop_with_arrows_inside() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = tab_with(Vec::new());
        let branch = |name: &str, is_head: bool| canopy_core::BranchInfo {
            name: name.into(),
            target: None,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            worktree: None,
        };
        let tag = |name: &str| canopy_core::TagInfo {
            name: name.into(),
            target: None,
            message: None,
        };
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot {
            branches: vec![
                branch("feature/a", false),
                branch("main", true),
                branch("zeta", false),
            ],
            tags: vec![tag("v2"), tag("v1")],
            ..Default::default()
        }));
        tab.snap_index = crate::tab::SnapIndex::new(tab.snap.as_deref().expect("snapshot"));
        let mut actions = Vec::new();
        let press = |key: egui::Key| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }]
        };
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let mut path = Vec::new();
        for key in [
            egui::Key::Tab,
            egui::Key::Tab,
            egui::Key::ArrowDown,
            egui::Key::Tab,
            egui::Key::Tab,
            egui::Key::Tab,
            egui::Key::Tab,
            egui::Key::Tab,
            egui::Key::ArrowDown,
            egui::Key::Tab,
        ] {
            let _ = sidebar_frame(&ctx, &mut tab, press(key), &mut actions);
            let (nodes, focused) = sidebar_frame_focus(&ctx, &mut tab, Vec::new(), &mut actions);
            path.push(focused.unwrap_or_default());
            if key == egui::Key::Tab && path.len() == 2 {
                let list = nodes
                    .iter()
                    .find(|n| n.role() == Role::Tree && n.label() == Some("Branches"))
                    .expect("the branches tree");
                assert_eq!(list.label(), Some("Branches"));
                // main, zeta, the feature folder and feature/a inside it.
                assert_eq!(list.children().len(), 4, "the branch rows are its parts");
            }
        }
        crate::ui::motion::set_reduced(false);
        let open = |s: &str| a11y::disclosure_name(s, true);
        assert_eq!(
            path,
            [
                open("Branches"),
                "main".to_owned(),
                "zeta".to_owned(),
                open("Remotes"),
                "Publish…".to_owned(),
                "Add remote…".to_owned(),
                open("Tags"),
                "v2".to_owned(),
                "v1".to_owned(),
                open("Stashes"),
            ],
            "{path:?}"
        );
        assert!(actions.is_empty(), "{actions:?}");
    }

    /// Branches group into folders by their prefixes: a folder is a tree item with its expanded
    /// state, a click closes it (hiding what it holds) and the choice is saved with the
    /// repository's other sidebar choices.
    #[test]
    fn a_prefix_folder_collapses_and_the_choice_is_remembered() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = tab_with(Vec::new());
        let branch = |name: &str, is_head: bool| canopy_core::BranchInfo {
            name: name.into(),
            target: None,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            worktree: None,
        };
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot {
            branches: vec![
                branch("main", true),
                branch("feature/a", false),
                branch("feature/b", false),
            ],
            ..Default::default()
        }));
        tab.snap_index = crate::tab::SnapIndex::new(tab.snap.as_deref().expect("snapshot"));
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let open = a11y::disclosure_name("feature, folder of 2 branches", true);
        let folder = named(&nodes, &open).expect("the open folder");
        assert_eq!(folder.role(), Role::TreeItem);
        assert_eq!(folder.is_expanded(), Some(true));
        // A branch is announced by its whole name; the branch without a prefix stays at the top.
        assert!(named(&nodes, "feature/a").is_some() && named(&nodes, "feature/b").is_some());
        assert!(named(&nodes, "main").is_some());

        let at = center(folder);
        let _ = sidebar_frame(&ctx, &mut tab, click(at), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert!(tab.collapsed.contains("folder:feature"));
        assert!(
            named(&nodes, "feature/a").is_none(),
            "closed folders hide their branches"
        );
        let closed = a11y::disclosure_name("feature, folder of 2 branches", false);
        assert!(named(&nodes, &closed).is_some());
        assert!(
            actions.iter().any(|a| matches!(a, Action::RememberSidebar { closed, .. } if closed.contains("folder:feature"))),
            "{actions:?}"
        );
    }

    fn oid(n: u8) -> canopy_core::Oid {
        canopy_core::Oid::from_bytes(&[n; 20]).expect("oid")
    }

    /// A tab whose branches are main (checked out), feature and wt (checked out in another
    /// worktree), each at its own commit.
    fn branch_tab() -> RepoTab {
        let mut tab = tab_with(Vec::new());
        let branch =
            |name: &str, n: u8, is_head: bool, worktree: Option<&str>| canopy_core::BranchInfo {
                name: name.into(),
                target: Some(oid(n)),
                is_head,
                upstream: None,
                ahead: 0,
                behind: 0,
                worktree: worktree.map(std::path::PathBuf::from),
            };
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot {
            branches: vec![
                branch("main", 1, true, None),
                branch("feature", 2, false, None),
                branch("wt", 3, false, Some("/work/canopy-wt")),
            ],
            worktrees: vec![
                canopy_core::WorktreeInfo {
                    path: "/work/canopy".into(),
                    branch: Some("main".into()),
                    head: Some(oid(1)),
                    current: true,
                    main: true,
                },
                canopy_core::WorktreeInfo {
                    path: "/work/canopy-wt".into(),
                    branch: Some("wt".into()),
                    head: Some(oid(3)),
                    current: false,
                    main: false,
                },
            ],
            ..Default::default()
        }));
        tab.snap_index = crate::tab::SnapIndex::new(tab.snap.as_deref().expect("snapshot"));
        tab
    }

    /// The selected state of the row named `label`, and whether it is announced as current.
    fn row_state(nodes: &[egui::accesskit::Node], label: &str) -> (Option<bool>, bool) {
        let n = named(nodes, label).unwrap_or_else(|| panic!("{label}"));
        (n.is_selected(), n.aria_current().is_some())
    }

    /// The branch clicked shows selected while History shows its commit; the checked-out branch
    /// is announced as current, not as selected.
    #[test]
    fn a_clicked_branch_shows_selected_apart_from_the_checked_out_one() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = branch_tab();
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert_eq!(row_state(&nodes, "main"), (Some(false), true));
        assert_eq!(row_state(&nodes, "feature"), (Some(false), false));

        let at = center(named(&nodes, "feature").expect("feature"));
        let _ = sidebar_frame(&ctx, &mut tab, click(at), &mut actions);
        assert!(
            matches!(actions.as_slice(), [Action::Reveal(o)] if *o == oid(2)),
            "{actions:?}"
        );
        // What the app does with the Reveal.
        tab.select_commit(oid(2));
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert_eq!(row_state(&nodes, "feature"), (Some(true), false));
        assert_eq!(row_state(&nodes, "main"), (Some(false), true));

        // Another commit chosen in History lets go of the branch.
        tab.select_commit(oid(9));
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        crate::ui::motion::set_reduced(false);
        assert_eq!(row_state(&nodes, "feature"), (Some(false), false));
    }

    /// A branch checked out in another worktree looks like any branch (its folder is under
    /// Worktrees) and opens that worktree on a double click instead of trying a checkout git
    /// would refuse.
    #[test]
    fn a_branch_in_another_worktree_opens_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = branch_tab();
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let at = center(named(&nodes, "wt").expect("the branch is listed by its name"));
        let mut events = click(at);
        events.extend(click(at));
        let _ = sidebar_frame(&ctx, &mut tab, events, &mut actions);
        crate::ui::motion::set_reduced(false);
        assert!(
            actions.iter().any(
                |a| matches!(a, Action::SwitchWorktree(p) if p == std::path::Path::new("/work/canopy-wt"))
            ),
            "{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::Op(Op::Checkout(_)))),
            "{actions:?}"
        );
    }

    /// Worktrees list the working folders apart from the branches: the open one marked current,
    /// a click showing a folder's commit, a double click opening it.
    #[test]
    fn worktrees_have_their_own_section() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = branch_tab();
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert_eq!(
            row_state(&nodes, "canopy, worktree on main"),
            (Some(false), true)
        );
        let other = "canopy-wt, worktree on wt";
        assert_eq!(row_state(&nodes, other), (Some(false), false));
        assert!(named(&nodes, "Worktrees").is_some(), "its own section");

        let at = center(named(&nodes, other).expect(other));
        let _ = sidebar_frame(&ctx, &mut tab, click(at), &mut actions);
        assert!(
            matches!(actions.as_slice(), [Action::Reveal(o)] if *o == oid(3)),
            "{actions:?}"
        );
        tab.select_commit(oid(3));
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert_eq!(row_state(&nodes, other), (Some(true), false));
        assert_eq!(
            row_state(&nodes, "wt"),
            (Some(false), false),
            "the branch at the same commit stays unpicked"
        );

        actions.clear();
        let mut events = click(at);
        events.extend(click(at));
        let _ = sidebar_frame(&ctx, &mut tab, events, &mut actions);
        crate::ui::motion::set_reduced(false);
        assert!(
            actions.iter().any(
                |a| matches!(a, Action::SwitchWorktree(p) if p == std::path::Path::new("/work/canopy-wt"))
            ),
            "{actions:?}"
        );
    }

    /// Submodules have their own section: an initialized one opens in a tab on a double click, one
    /// never initialized does not, and removal asks first.
    #[test]
    fn submodules_have_their_own_section() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = branch_tab();
        let sub = |path: &str, checked_out: bool| canopy_core::SubmoduleInfo {
            name: path.into(),
            path: path.into(),
            path_os: path.into(),
            url: Some(format!("https://example.com/{path}.git")),
            head_id: Some(oid(7)),
            index_id: Some(oid(7)),
            workdir_id: checked_out.then(|| oid(7)),
            conflicted: false,
            dirty: None,
        };
        if let Some(snap) = tab.snap.as_mut() {
            std::sync::Arc::make_mut(snap).submodules =
                vec![sub("deps/lib", true), sub("deps/idle", false)];
        }
        // The other sections out of the way, so the rows are inside the frame.
        for key in ["branches", "worktrees", "remotes", "tags", "stashes"] {
            tab.collapsed.insert(key.to_owned());
        }
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert!(named(&nodes, "Submodules").is_some(), "its own section");
        let idle = "deps/idle, submodule, Not initialized";
        let lib = "deps/lib, submodule, Up to date";
        let at = center(named(&nodes, lib).expect(lib));
        let mut events = click(at);
        events.extend(click(at));
        let _ = sidebar_frame(&ctx, &mut tab, events, &mut actions);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::OpenRepo(p) if p.ends_with("deps/lib"))),
            "{actions:?}"
        );

        actions.clear();
        let at = center(named(&nodes, idle).expect(idle));
        let mut events = click(at);
        events.extend(click(at));
        let _ = sidebar_frame(&ctx, &mut tab, events, &mut actions);
        crate::ui::motion::set_reduced(false);
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn a_large_submodule_list_only_builds_visible_rows() {
        let ctx = crate::ui::kit::testkit::ctx();
        ctx.enable_accesskit();
        let mut tab = branch_tab();
        let snap = std::sync::Arc::make_mut(tab.snap.as_mut().expect("snapshot"));
        snap.submodules = (0..1000)
            .map(|i| canopy_core::SubmoduleInfo {
                name: format!("deps/lib-{i}"),
                path: format!("deps/lib-{i}"),
                path_os: format!("deps/lib-{i}").into(),
                url: None,
                head_id: Some(oid(7)),
                index_id: Some(oid(7)),
                workdir_id: Some(oid(7)),
                conflicted: false,
                dirty: None,
            })
            .collect();
        for key in ["branches", "worktrees", "remotes", "tags", "stashes"] {
            tab.collapsed.insert(key.into());
        }
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let rows = nodes
            .iter()
            .filter(|n| n.label().is_some_and(|s| s.contains(", submodule,")))
            .count();
        assert!(
            rows > 0 && rows < 30,
            "built {rows} submodule rows for a 700-point viewport"
        );
    }

    /// A removed submodule is staged for deletion, not committed yet; it leaves the list at
    /// once instead of staying behind as "Not initialized" with an Initialize action.
    #[test]
    fn a_submodule_whose_removal_is_staged_leaves_the_list() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = branch_tab();
        let sub = |path: &str, index: Option<u8>| canopy_core::SubmoduleInfo {
            name: path.into(),
            path: path.into(),
            path_os: path.into(),
            url: Some(format!("https://example.com/{path}.git")),
            head_id: Some(oid(7)),
            index_id: index.map(oid),
            workdir_id: index.map(oid),
            conflicted: false,
            dirty: None,
        };
        if let Some(snap) = tab.snap.as_mut() {
            std::sync::Arc::make_mut(snap).submodules =
                vec![sub("deps/lib", Some(7)), sub("deps/gone", None)];
        }
        for key in ["branches", "worktrees", "remotes", "tags", "stashes"] {
            tab.collapsed.insert(key.to_owned());
        }
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        crate::ui::motion::set_reduced(false);
        assert!(
            named(&nodes, "deps/lib, submodule, Up to date").is_some(),
            "the kept submodule is listed"
        );
        assert!(
            named(&nodes, "deps/gone, submodule, Not initialized").is_none(),
            "the removed one is not"
        );
    }

    #[test]
    fn removing_a_submodule_asks_and_names_the_danger() {
        let Action::Confirm {
            danger, body, op, ..
        } = remove_submodule("deps/lib")
        else {
            panic!("a confirmation")
        };
        assert!(danger && body.contains("never pushed"), "{body}");
        assert!(matches!(op, Op::SubmoduleRemove(p) if p == "deps/lib"));
    }

    /// A repository with one working folder has no Worktrees section.
    #[test]
    fn a_single_working_folder_has_no_worktrees_section() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        let mut tab = branch_tab();
        if let Some(snap) = &mut tab.snap {
            std::sync::Arc::make_mut(snap).worktrees.truncate(1);
        }
        let mut actions = Vec::new();
        let _ = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        let nodes = sidebar_frame(&ctx, &mut tab, Vec::new(), &mut actions);
        assert!(named(&nodes, "Worktrees").is_none());
        assert!(named(&nodes, "canopy, worktree on main").is_none());
    }

    /// review-views-3: Drop names the stash and acts on its commit, not its place in the list.
    #[test]
    fn dropping_a_stash_names_it_and_acts_on_its_commit() {
        let oid = canopy_core::Oid::from_bytes(&[7; 20]).expect("oid");
        let stash = canopy_core::StashInfo {
            index: 0,
            message: "On main: wip login".into(),
            oid,
        };
        let Action::Confirm {
            title, body, op, ..
        } = drop_stash(&stash)
        else {
            panic!("a confirmation");
        };
        assert!(title.contains("main: wip login"), "{title}");
        assert!(body.contains(&canopy_core::short_id(oid)), "{body}");
        assert!(matches!(op, Op::StashDrop(o) if o == oid), "{op:?}");
    }
}
