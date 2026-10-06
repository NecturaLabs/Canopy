//! The left sidebar: branches, remotes, tags and stashes.

use egui::accesskit::Role;
use egui::{FontId, RichText, Ui};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::{RepoTab, SidebarItem};
use crate::ui::kit::chip::Badge;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Skeleton;
use crate::ui::kit::row::{ListRow, Roving, RowKind, SectionHeader, TreeNav, TreeRow};
use crate::ui::kit::{Button, KitResponse, Size, Tone, a11y, menu};
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

/// The Remotes section of a repository with none: a small card that says so, with Publish as the
/// main way on and Add remote for a repository that already exists elsewhere.
fn no_remotes(ui: &mut Ui, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let s = p.s();
    let m = theme::metrics();
    ui.add_space(theme::XS);
    egui::Frame::new()
        .fill(s.elevated)
        .stroke(egui::Stroke::new(crate::ui::kit::paint::SEAM, s.edge))
        .corner_radius(egui::CornerRadius::same(m.radius))
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
                        .family(theme::medium())
                        .color(p.text),
                );
            });
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Nothing leaves this computer until you publish it or connect a remote.",
                    )
                    .size(m.meta_size)
                    .color(p.text_dim),
                )
                .wrap(),
            );
            ui.add_space(theme::XS);
            if Button::new("Publish…")
                .primary()
                .size(Size::Sm)
                .icon(icon::CLOUD_ARROW_UP)
                .full_width()
                .tooltip("Create a repository on GitHub or GitLab and push this one to it")
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
    let room = ui.available_width() - 2.0 * theme::MD - m.icon_md - theme::SM;
    let tip = r
        .hint
        .map(str::to_owned)
        .or_else(|| (label_w > room - badges_w).then(|| r.label.to_owned()));
    let mut row = ListRow::new(r.id)
        .kind(RowKind::Sidebar)
        .selected(r.selected)
        .current(r.current)
        .icon(r.glyph)
        .label(r.label);
    if let Some(d) = r.detail {
        row = row.detail(d);
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
        if section(
            ui,
            tab,
            "branches",
            "Branches",
            icon::GIT_BRANCH,
            snap.branches.len() + usize::from(unborn.is_some()),
            true,
        ) {
            // One Tab stop for the branches: the checked-out one until the user moves.
            let first = usize::from(unborn.is_some());
            let head = snap.branches.iter().position(|b| b.is_head).map(|i| i + first);
            let list = Roving::new(ui.ctx(), ("sidebar-branches", &tab.path), snap.branches.len() + first, head.or(Some(0)));
            ui.scope(|ui| {
            if snap.branches.len() + first > 0 { a11y::group(ui, Role::List, "Branches"); }
            if let Some(name) = unborn {
                let label = format!("{name} (no commits yet)");
                let _ = row(ui, &Row { id: ui.id().with("unborn-branch"), glyph: icon::CHECK_CIRCLE, label: &label, selected: false, current: true, ahead: 0, behind: 0, hint: Some("Current branch; make the first commit to create it"), detail: None, spoken: None }, Some((list, 0)));
            }
            virtual_rows(ui, snap.branches.len(), row_h, |ui, i| {
                let b = &snap.branches[i];
                // A branch checked out in another worktree (listed under Worktrees) lives there:
                // git refuses to check it out here, so it opens that worktree instead.
                let worktree = b.worktree.as_ref().filter(|_| !b.is_head);
                let glyph = if b.is_head { icon::CHECK_CIRCLE } else { icon::GIT_BRANCH };
                let mut hint = b.upstream.as_ref().map_or_else(|| "No upstream".to_owned(), |u| format!("Tracks {u}"));
                if let Some(w) = worktree {
                    hint = format!("Checked out in the worktree {} · {hint}", folder_name(w));
                }
                let selected = tab.sidebar_shows(|p| matches!(p, SidebarItem::Branch(n) if *n == b.name), b.target);
                let resp = row(ui, &Row { id: ui.id().with(("branch", &b.name)), glyph, label: &b.name, selected, current: b.is_head, ahead: b.ahead, behind: b.behind, hint: Some(&hint), detail: None, spoken: None }, Some((list, i + first)));
                if resp.clicked()
                    && let Some(t) = b.target {
                        pick = Some(SidebarItem::Branch(b.name.clone()));
                        actions.push(Action::Reveal(t));
                    }
                if resp.double_clicked() && !b.is_head {
                    actions.push(match worktree {
                        Some(w) => Action::OpenRepo(w.clone()),
                        None => Action::Op(Op::Checkout(b.name.clone())),
                    });
                }
                let name = b.name.clone();
                let is_head = b.is_head;
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
                                actions.push(Action::OpenRepo(w.clone()));
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
                            actions.push(Action::Confirm { title: format!("Force push {name}?"), body: format!("The remote copy of {name} is overwritten with your local branch. --force-with-lease refuses if someone else pushed in the meantime."), confirm: "Force push branch".into(), danger: true, op: Op::Push { force: true } });
                        }
                    } else if worktree.is_none() && m.item(icon::TRASH, "Delete…").danger().show(m.ui).clicked() {
                        actions.push(Action::Confirm { title: format!("Delete branch {name}?"), body: "Only the local branch is deleted. If it has commits that are not merged, Git refuses and Canopy asks before forcing it.".into(), confirm: "Delete branch".into(), danger: true, op: Op::DeleteBranch { name: name.clone(), force: false } });
                    }
                });
            });
            });
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
                    actions.push(Action::OpenRepo(w.path.clone()));
                }
                let path = w.path.clone();
                let current = w.current;
                menu::context(&resp, |m| {
                    if !current && m.item(icon::FOLDER_OPEN, "Open worktree").show(m.ui).clicked() {
                        actions.push(Action::OpenRepo(path.clone()));
                    }
                    if m.item(icon::COPY, "Copy path").show(m.ui).clicked() {
                        actions.push(Action::Copy(path.display().to_string()));
                    }
                });
            }
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
        ui.add_space(theme::LG);
    });
    if pick.is_some() {
        tab.sidebar_pick = pick;
    }
}

/// A worktree's folder name, as its row and tooltips call it.
fn folder_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
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
            Some(Role::Button),
            "the header's + is in the tree at rest"
        );
        let header =
            center(named(&nodes, &a11y::disclosure_name("Remotes", true)).expect("remotes header"));
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

    /// O9, KPL-F13: the Remotes header's "+" is reachable from the keyboard: Tab from the header
    /// lands on it, and Shift+Tab goes back to the header.
    #[test]
    fn the_remotes_header_button_is_reachable_by_tab_and_shift_tab() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        crate::ui::motion::set_reduced(true);
        let mut tab = tab_with(vec![origin()]);
        tab.collapsed.insert("branches".into());
        let mut actions = Vec::new();
        let key = |shift: bool| egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: if shift {
                egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::default()
            },
        };
        let mut focused = Vec::new();
        for _ in 0..6 {
            let _ = sidebar_frame(&ctx, &mut tab, vec![key(false)], &mut actions);
            let now = sidebar_frame_focus(&ctx, &mut tab, Vec::new(), &mut actions).1;
            let reached = now.as_deref() == Some("Add remote…");
            focused.push(now);
            if reached {
                break;
            }
        }
        assert_eq!(
            focused.last().cloned().flatten().as_deref(),
            Some("Add remote…"),
            "Tab never reached the header's +: {focused:?}"
        );
        assert!(
            focused
                .iter()
                .rev()
                .nth(1)
                .is_some_and(|l| l.as_deref().is_some_and(|l| l.starts_with("Remotes"))),
            "the + comes right after its header: {focused:?}"
        );
        let _ = sidebar_frame(&ctx, &mut tab, vec![key(true)], &mut actions);
        let back = sidebar_frame_focus(&ctx, &mut tab, Vec::new(), &mut actions).1;
        crate::ui::motion::set_reduced(false);
        assert!(
            back.as_deref().is_some_and(|l| l.starts_with("Remotes")),
            "Shift+Tab goes back to the header: {back:?}"
        );
        assert!(actions.is_empty(), "{actions:?}");
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
                    .find(|n| n.role() == Role::List && n.label() == Some("Branches"))
                    .expect("the branches list");
                assert_eq!(list.label(), Some("Branches"));
                assert_eq!(list.children().len(), 3, "the branch rows are its parts");
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
                |a| matches!(a, Action::OpenRepo(p) if p == std::path::Path::new("/work/canopy-wt"))
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
                |a| matches!(a, Action::OpenRepo(p) if p == std::path::Path::new("/work/canopy-wt"))
            ),
            "{actions:?}"
        );
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
