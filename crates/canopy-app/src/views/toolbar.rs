//! The tab strip and the action bar. `app` paints what they sit on (Standard: flat chrome over a
//! seam each; Deeper: the tab strip on the ground and the action bar on a lifted card), so nothing
//! here paints a background of its own.

use std::fmt::Write as _;
use std::sync::Arc;

use canopy_core::RemoteInfo;
use canopy_core::forge::ForgeKind;
use canopy_hosting::{RequestState, RequestSummary, StateFilter};
use egui::{
    Align, Id, Layout, RichText, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use crate::action::{Action, AppearanceChoice};
use crate::keymap::Command;
use crate::tab::{Finished, RepoTab, View};
use crate::ui::art;
use crate::ui::kit::button::{Operation, tool_divider};
use crate::ui::kit::choice::ViewTabs;
use crate::ui::kit::field::SearchField;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::tabs::{DocTab, DocTabs, DocTabsEvent};
use crate::ui::kit::{IconButton, Size, ToolButton, focus_visible, menu, paint};
use crate::ui::motion;
use crate::ui::theme::{self, ThemeId, palette};
use crate::ui::widgets::forge_icon;
use crate::worker::{Op, OpKind};

/// Repository tabs: the wordmark, the document tabs, and the strip's tools on the right.
pub fn tabs(
    ui: &mut Ui,
    tabs: &[RepoTab],
    active: usize,
    current_theme: ThemeId,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let compact = ui.ctx().content_rect().width() < theme::COMPACT_WIDTH;
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = theme::XS;
        ui.add_space(theme::MD);
        art::show(ui, art::logo(), Vec2::splat(m.icon_xl));
        ui.label(
            RichText::new("Canopy")
                .font(theme::heading(m.subtitle_size))
                .color(p.text),
        );
        ui.add_space(theme::MD);
        // The tools on the right keep their room; the tabs scroll in the rest, and the "+" after
        // them stays in sight.
        let tools_id = ui.id().with("tab-strip-tools");
        let tools_w: f32 = ui.data(|d| d.get_temp(tools_id)).unwrap_or(0.0);
        let open_w = Size::Sm.height() + theme::XS;
        let room = (ui.available_width() - tools_w - theme::LG - open_w).max(m.control * 2.0);
        let docs = doc_tabs(tabs);
        let shown = DocTabs::new(&docs, active)
            .max_width(room)
            .id_salt("repository-tabs")
            .show(ui);
        match shown.value {
            Some(DocTabsEvent::Select(i)) => actions.push(Action::SwitchTab(i)),
            Some(DocTabsEvent::Close(i)) => actions.push(Action::CloseTab(i)),
            None => {}
        }
        if IconButton::new(icon::PLUS, "Open repository")
            .shortcut(Command::Open)
            .size(Size::Sm)
            .show(ui)
            .clicked()
        {
            actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::open()));
        }
        // The tools sit at the right edge but are laid out left to right, so Tab visits them in
        // the order they read. Their width is the last frame's; the first frame, with none, is
        // laid out unseen.
        let bar = ui.max_rect();
        let mut place = egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                pos2(bar.right() - tools_w, bar.top()),
                bar.right_bottom(),
            ))
            .layout(Layout::left_to_right(Align::Center));
        if tools_w == 0.0 {
            place = place.invisible();
        }
        let tools = ui.scope_builder(place, |ui| {
            ui.spacing_mut().item_spacing.x = theme::XS;
            // Search stays reachable when repository tabs take the available width.
            if compact {
                if IconButton::new(icon::MAGNIFYING_GLASS, "Search commands")
                    .shortcut(Command::Palette)
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::Palette);
                }
            } else if search_trigger(ui).clicked() {
                actions.push(Action::Palette);
            }
            let theme_button = IconButton::new(icon::PALETTE, "Theme").show(ui);
            if let Some(Some(action)) = menu::menu(&theme_button, |m| theme_items(m, current_theme))
            {
                actions.push(action);
            }
            if IconButton::new(icon::GEAR_SIX, "Settings")
                .shortcut(Command::Settings)
                .show(ui)
                .clicked()
            {
                actions.push(Action::Settings);
            }
            if IconButton::new(icon::KEYBOARD, "Keyboard shortcuts")
                .shortcut(Command::Cheatsheet)
                .show(ui)
                .clicked()
            {
                actions.push(Action::Cheatsheet);
            }
            ui.add_space(theme::SM);
            ui.min_rect().width()
        });
        let width = tools.inner;
        if (width - tools_w).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(tools_id, width));
            ui.ctx().request_repaint();
        }
    });
}

/// What a tab's tooltip says besides its name: its path, the shortcut that switches to it (the
/// first nine), and, for a tab that has not been shown yet and so has no counts, that it is not
/// loaded, so that nothing about it reads as "in sync".
fn tab_detail(tab: &RepoTab, index: usize) -> String {
    let mut detail = tab.path.display().to_string();
    if index < 9 {
        let modifier = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        let _ = write!(detail, "  ·  {modifier}+{}", index + 1);
    }
    if !tab.is_loaded() {
        detail.push_str("  ·  Not loaded yet: it loads when you switch to it");
    }
    detail
}

/// The kit's document tabs for the open repositories: keyed by path, with each one's sync state,
/// whether it is working, and its path and switch shortcut in the tooltip.
fn doc_tabs(tabs: &[RepoTab]) -> Vec<DocTab<'_>> {
    tabs.iter()
        .enumerate()
        .map(|(i, tab)| {
            // Commits to push and pull, like the status bar, so every tab shows its sync state.
            let (ahead, behind) = tab
                .snap
                .as_ref()
                .and_then(|s| s.current_branch())
                .filter(|b| b.upstream.is_some())
                .map_or((0, 0), |b| (b.ahead, b.behind));
            let detail = tab_detail(tab, i);
            let doc = DocTab::new(&tab.path, &tab.name)
                .icon(tab.icon)
                .sync(ahead, behind)
                .busy(tab.running.is_some())
                .detail(detail);
            match &tab.running {
                Some(job) => doc.tooltip(format!("{}  ·  {job}…", tab.name)),
                None => doc,
            }
        })
        .collect()
}

/// The theme menu: every theme with its swatch (ground and accent), the active one checked, then
/// the High contrast switch.
fn theme_items(m: &mut menu::Menu<'_>, current: ThemeId) -> Option<Action> {
    let metrics = theme::metrics();
    let s = theme::surfaces(m.ui.ctx());
    let mut picked = None;
    for t in ThemeId::ALL {
        // A blank glyph keeps the icon column, where the swatch is painted.
        let r = m.item(" ", t.name()).checked(t == current).show(m.ui);
        // A theme circle, like Settings' theme cards: the theme's pane with its accent at the
        // centre, as a graph node sits on it.
        let tp = t.palette();
        let radius = metrics.icon_md / 2.0;
        let center = pos2(r.rect.left() + theme::SM + radius, r.rect.center().y);
        let painter = m.ui.painter();
        painter.circle_filled(center, radius, tp.s().pane);
        painter.circle_filled(center, radius * 0.45, tp.accent);
        painter.circle_stroke(
            center,
            radius - paint::SEAM / 2.0,
            Stroke::new(paint::SEAM, s.edge),
        );
        if r.clicked() {
            picked = Some(Action::SetTheme(t));
        }
    }
    m.separator();
    let on = theme::high_contrast();
    if m.item(icon::CIRCLE_HALF, "High contrast")
        .checked(on)
        .show(m.ui)
        .clicked()
    {
        picked = Some(Action::SetAppearance(AppearanceChoice::HighContrast(!on)));
    }
    picked
}

/// The title-bar search: a read-only search field that opens the command palette.
fn search_trigger(ui: &mut Ui) -> egui::Response {
    let m = theme::metrics();
    let size = if m.search_trigger.y <= Size::Sm.height() {
        Size::Sm
    } else {
        Size::Md
    };
    let mut nothing = String::new();
    // Its own slot, so the field lays out from its left edge whatever the strip's direction.
    let field = ui
        .allocate_ui_with_layout(
            vec2(m.search_trigger.x, size.height()),
            Layout::left_to_right(Align::Center),
            |ui| {
                SearchField::new(&mut nothing)
                    .placeholder("Search commands…")
                    .shortcut(Command::Palette)
                    .read_only(true)
                    .size(size)
                    .width(m.search_trigger.x)
                    .id_salt("title-search")
                    .show(ui)
            },
        )
        .inner;
    let r = ui
        .interact(
            field.rect,
            ui.id().with("title-search-open"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Search commands"));
    // The read-only field and its click target are separate widgets. Whichever
    // receives Tab focus needs the same visible ring as the other title-bar tools.
    if (r.has_focus() || field.has_focus()) && focus_visible(ui.ctx()) {
        paint::focus_ring(
            ui.painter(),
            field.rect,
            egui::CornerRadius::same(m.radius_sm),
            palette(ui.ctx()).focus,
            egui::Color32::TRANSPARENT,
        );
    }
    r.kit_tooltip("Search commands")
        .extra(true)
        .shortcut(Command::Palette)
        .show()
}

// ---------------------------------------------------------------------------------- action bar

/// The action bar of the active repository: the view switcher, then the tool groups.
pub fn actions_bar(ui: &mut Ui, tab: &RepoTab, actions: &mut Vec<Action>) {
    let m = theme::metrics();
    let snap = tab.snap.as_ref();
    let has_remote = snap.is_some_and(|s| !s.remotes.is_empty());
    let local_only = tab.is_local_only();
    let busy = tab.running.is_some();
    let running = Running::of(ui.ctx(), tab);
    let branch = snap.and_then(|s| s.current_branch());
    let forge = tab.forge();
    let width = ui.available_width();
    // Deeper floats the bar on a card inset by the pane gutter: the tools keep their padding
    // inside it, in line with the panes' content below.
    let card_inset = if theme::style() == theme::Style::Deeper {
        m.pane_gutter
    } else {
        0.0
    };
    let at_left_edge = ui.max_rect().left() <= ui.ctx().content_rect().left() + 0.5;
    let left_pad = theme::SM + if at_left_edge { card_inset } else { 0.0 };
    let right_pad = theme::SM + card_inset;
    let no_remote = "Add a remote first";
    let busy_reason = "Wait for the running operation to finish";
    let reason = |remote_needed: bool| {
        if busy {
            busy_reason
        } else if remote_needed && !has_remote {
            no_remote
        } else {
            "Not available here"
        }
    };
    // Too narrow for every tool at full size: only the active view keeps its name and the Local
    // only badge goes; past that the tools that still do not fit move, in order, into a More menu
    // at the right end. The bar never scrolls.
    let full_id = Id::new(("toolbar-full-width", &tab.path));
    let full_w: f32 = ui.data(|d| d.get_temp(full_id)).unwrap_or(0.0);
    let narrow_window = ui.ctx().content_rect().width() < theme::COMPACT_WIDTH;
    let squeezed = full_w > width + 0.5;
    let bar = ui.max_rect();
    ui.scope(|ui| {
        // A tool measured for the first time may run past the edge for the one pass before the
        // bar lays out again; nothing shows outside the bar meanwhile.
        ui.set_clip_rect(ui.clip_rect().intersect(bar));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = m.toolbar_gap;
            let bar_start = ui.cursor().min.x;
            ui.add_space(left_pad);
            let changes = snap.map_or(0, |s| s.status.len());
            let open_requests = open_requests(ui.ctx(), tab);
            view_tabs(
                ui,
                tab.view,
                changes,
                forge.map_or_else(
                    || Err(snap.map(|s| s.remotes.as_slice())),
                    |(_, f)| Ok((f.kind, open_requests)),
                ),
                narrow_window || squeezed,
                actions,
            );
            ui.add_space(theme::SM);
            tool_divider(ui);
            let fetch = ToolButton::new(icon::ARROWS_CLOCKWISE, "Fetch")
                .tooltip("Fetch all remotes")
                .enabled(has_remote && !busy)
                .disabled_reason(reason(true));
            if running.show(ui, fetch, Tool::Fetch, Some(Command::Fetch)) {
                actions.push(Action::Fetch);
            }
            let behind = branch.map_or(0, |b| b.behind);
            let (strategy, _) = tab.pull_default();
            let mut pull = ToolButton::new(icon::ARROW_FAT_LINE_DOWN, "Pull")
                .tooltip(format!("Pull the current branch: {}", strategy.label()))
                .enabled(has_remote && !busy)
                .disabled_reason(reason(true));
            if behind > 0 {
                pull = pull.badge(format!("↓{behind}"));
            }
            if running.show(ui, pull, Tool::Pull, Some(Command::Pull)) {
                actions.push(Action::Pull);
            }
            let ahead = branch.map_or(0, |b| b.ahead);
            if local_only {
                let publish = ToolButton::new(icon::CLOUD_ARROW_UP, "Publish")
                    .tooltip("This repository has no remote. Add one on GitHub or GitLab and push")
                    .enabled(!busy && branch.is_some())
                    .disabled_reason(if busy {
                        busy_reason
                    } else {
                        "Check out a branch to publish"
                    });
                if running.show(ui, publish, Tool::Publish, None) {
                    actions.push(Action::Publish);
                }
            } else {
                let mut push = ToolButton::new(icon::ARROW_FAT_LINE_UP, "Push")
                    .tooltip("Push the current branch")
                    .enabled(has_remote && !busy && branch.is_some())
                    .disabled_reason(if branch.is_none() && has_remote && !busy {
                        "Check out a branch to push"
                    } else {
                        reason(true)
                    });
                if ahead > 0 {
                    push = push.badge(format!("↑{ahead}"));
                }
                if running.show(ui, push, Tool::Push, Some(Command::Push)) {
                    actions.push(Action::Push);
                }
            }
            let tools = spare_tools(tab, busy, busy_reason);
            let fixed = ui.cursor().min.x - bar_start;
            let badge = local_only && !narrow_window && !squeezed;
            let plan = Shelf::plan(ui.ctx(), &tools, badge, fixed, width - right_pad);
            if !squeezed && let Some(needed) = plan.full {
                // What the bar needs at full size; measured only then, so it is known while
                // squeezed too and the bar opens up again once the window has room.
                let needed = needed + right_pad;
                if (needed - full_w).abs() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(full_id, needed));
                    ui.ctx().request_repaint();
                }
            }
            let mut remeasured = false;
            let mut group = None;
            for t in tools.iter().take(plan.shown) {
                if group != Some(t.group) {
                    if t.group == SpareGroup::Places && !plan.overflow {
                        // The places group sits at the right end, on its width from the
                        // frame before.
                        ui.add_space(plan.spacer);
                    } else if t.group != SpareGroup::Remote {
                        remeasured |= measured(ui, Shelf::divider_id(), tool_divider);
                    }
                    group = Some(t.group);
                }
                remeasured |= measured(ui, Shelf::tool_id(t), |ui| {
                    // Its own id, whichever tools come before it: focus stays on the tool
                    // it was given to as tools move in and out of the More menu.
                    ui.push_id(("toolbar-tool", &t.label), |ui| {
                        if t.show(ui, &running)
                            && let Some(action) = &t.action
                        {
                            actions.push(action.clone());
                        }
                    });
                });
            }
            if plan.badge {
                remeasured |= measured(ui, Shelf::divider_id(), tool_divider);
                remeasured |= measured(ui, Shelf::badge_id(), local_status);
            }
            if plan.overflow {
                let more_left = bar_start + width - right_pad - plan.more;
                ui.add_space((more_left - ui.cursor().min.x).max(0.0));
                let mut more = None;
                remeasured |= measured(ui, Shelf::more_id(), |ui| {
                    let r = ui
                        .push_id("toolbar-more", |ui| {
                            ToolButton::new(icon::DOTS_THREE, "More")
                                .tooltip("More actions")
                                .show(ui)
                        })
                        .inner;
                    ui.ctx()
                        .accesskit_node_builder(r.id, |n| n.set_label("More actions"));
                    more = Some(r);
                });
                if let Some(more) = more
                    && let Some(Some(action)) =
                        menu::menu(&more, |m| more_items(m, &tools[plan.shown..], &running))
                {
                    actions.push(action);
                }
            }
            if remeasured {
                ui.ctx().request_discard("a toolbar tool changed width");
            }
        });
    });
}

/// Paints `add` and remembers how far it moved the cursor under `id`; returns whether that
/// changed.
fn measured(ui: &mut Ui, id: Id, add: impl FnOnce(&mut Ui)) -> bool {
    let start = ui.cursor().min.x;
    add(ui);
    let advance = ui.cursor().min.x - start;
    let before: Option<f32> = ui.data(|d| d.get_temp(id));
    let changed = before.is_none_or(|b| (b - advance).abs() > 0.5);
    if changed {
        ui.data_mut(|d| d.insert_temp(id, advance));
    }
    changed
}

/// The tool groups after Push, which make room on a narrow bar by moving into the More menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpareGroup {
    /// The pull or merge request tool, beside Push.
    Remote,
    /// Branch to Pop.
    Work,
    /// Terminal, Files and the forge, at the right end.
    Places,
}

/// A tool after Push: on the bar while it fits, else a row of the More menu with the same icon,
/// label, shortcut and disabled reason.
struct Spare {
    glyph: &'static str,
    label: String,
    tooltip: String,
    enabled: bool,
    reason: String,
    shortcut: Option<Command>,
    /// The tool whose operation can run (and spin on it).
    tool: Option<Tool>,
    action: Option<Action>,
    group: SpareGroup,
}

impl Spare {
    fn new(group: SpareGroup, glyph: &'static str, label: impl Into<String>) -> Self {
        Self {
            glyph,
            label: label.into(),
            tooltip: String::new(),
            enabled: true,
            reason: String::new(),
            shortcut: None,
            tool: None,
            action: None,
            group,
        }
    }
    fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = text.into();
        self
    }
    fn enabled(mut self, enabled: bool, reason: &str) -> Self {
        self.enabled = enabled;
        reason.clone_into(&mut self.reason);
        self
    }
    fn shortcut(mut self, command: Command) -> Self {
        self.shortcut = Some(command);
        self
    }
    fn tool(mut self, tool: Tool) -> Self {
        self.tool = Some(tool);
        self
    }
    fn action(mut self, action: Action) -> Self {
        self.action = Some(action);
        self
    }

    /// Shows it on the bar; returns whether it was clicked.
    fn show(&self, ui: &mut Ui, running: &Running) -> bool {
        let button = ToolButton::new(self.glyph, self.label.clone())
            .tooltip(self.tooltip.clone())
            .enabled(self.enabled)
            .disabled_reason(self.reason.clone());
        match self.tool {
            Some(tool) => running.show(ui, button, tool, self.shortcut),
            None => match self.shortcut {
                Some(c) => button.shortcut(c).show(ui).clicked(),
                None => button.show(ui).clicked(),
            },
        }
    }
}

/// The tools after Push, in bar order.
fn spare_tools(tab: &RepoTab, busy: bool, busy_reason: &str) -> Vec<Spare> {
    use crate::views::dialogs::{Dialog, Start};
    let snap = tab.snap.as_ref();
    let branch = snap.and_then(|s| s.current_branch());
    let forge = tab.forge();
    let loading = "The repository is still loading";
    let or_busy = |reason: &'static str| if busy { busy_reason } else { reason };
    let mut tools = Vec::new();
    // Pull/merge request for a pushed branch.
    if let (Some((_, f)), Some(b)) = (forge, branch)
        && b.upstream.is_some()
        && !tab.is_default_branch(&b.name)
    {
        tools.push(
            Spare::new(
                SpareGroup::Remote,
                icon::GIT_PULL_REQUEST,
                f.kind.request_abbrev(),
            )
            .tooltip(format!(
                "Create a {} for {} on {}",
                f.kind.request_noun(),
                b.name,
                f.kind.name()
            ))
            .enabled(!busy, busy_reason)
            .action(Action::CreateRequest {
                source: b.name.clone(),
            }),
        );
    }
    let work = SpareGroup::Work;
    tools.push(
        Spare::new(work, icon::GIT_BRANCH, "Branch")
            .tooltip("New branch: a line of work in this folder. Use it for most changes")
            .enabled(snap.is_some(), loading)
            .shortcut(Command::NewBranch)
            .tool(Tool::Branch)
            .action(Action::OpenDialog(Dialog::new_branch_at(Start::Head))),
    );
    tools.push(
        Spare::new(work, icon::FOLDERS, "Worktree")
            .tooltip(
                "New worktree: a branch in a second folder, so this one stays as it is. Use it \
                 to work on two branches at once",
            )
            .enabled(snap.is_some() && !busy, or_busy(loading))
            .action(Action::OpenDialog(Dialog::new_worktree())),
    );
    tools.push(
        Spare::new(work, icon::GIT_MERGE, "Merge")
            .tooltip("Merge or rebase")
            .enabled(
                branch.is_some() && !busy,
                or_busy("Check out a branch to merge into"),
            )
            .shortcut(Command::MergeDialog)
            .tool(Tool::Merge)
            .action(Action::OpenDialog(Dialog::Merge {
                target: String::new(),
                rebase: false,
                mode: canopy_core::MergeMode::Default,
            })),
    );
    let dirty = snap.is_some_and(|s| !s.status.is_clean());
    tools.push(
        Spare::new(work, icon::ARCHIVE, "Stash")
            .tooltip("Stash changes")
            .enabled(dirty && !busy, or_busy("Nothing to stash"))
            .shortcut(Command::Stash)
            .tool(Tool::Stash)
            .action(Action::OpenDialog(Dialog::Stash {
                message: String::new(),
                untracked: true,
            })),
    );
    let latest = snap.and_then(|s| s.stashes.first());
    let pop_tip = match latest {
        Some(st) => format!(
            "Apply \u{201c}{}\u{201d} and remove it from the stash list",
            st.message
        ),
        None => "Apply the latest stash and remove it from the stash list".to_owned(),
    };
    let mut pop = Spare::new(work, icon::TRAY_ARROW_UP, "Pop")
        .tooltip(pop_tip)
        .enabled(latest.is_some() && !busy, or_busy("No stashes"))
        .shortcut(Command::PopStash)
        .tool(Tool::Pop);
    if let Some(st) = latest {
        // By its commit: the stash the tooltip named, even if the list changed.
        pop = pop.action(Action::Op(Op::StashApplyCommit {
            oid: st.oid,
            pop: true,
        }));
    }
    tools.push(pop);
    let places = SpareGroup::Places;
    tools.push(
        Spare::new(places, icon::TERMINAL_WINDOW, "Terminal")
            .tooltip("Open a terminal here")
            .action(Action::OpenTerminal(tab.path.clone())),
    );
    tools.push(
        Spare::new(places, icon::FOLDER_OPEN, "Files")
            .tooltip("Show in file manager")
            .action(Action::OpenPath(tab.path.clone())),
    );
    if let Some((_, f)) = forge {
        tools.push(
            Spare::new(places, forge_icon(f.kind), f.kind.name())
                .tooltip(format!("Open {} on {}", f.path, f.kind.name()))
                .action(Action::OpenUrl(f.web_url())),
        );
    }
    tools
}

/// The rows of the More menu: the `hidden` tools in bar order, a separator between groups.
/// Returns the chosen tool's action.
fn more_items(m: &mut menu::Menu<'_>, hidden: &[Spare], running: &Running) -> Option<Action> {
    let mut picked = None;
    let mut group = None;
    for t in hidden {
        if group.is_some_and(|g| g != t.group) {
            m.separator();
        }
        group = Some(t.group);
        let spinning = t.tool.is_some() && running.tool == t.tool;
        let mut item = m
            .item(t.glyph, t.label.clone())
            .enabled(t.enabled && !spinning)
            .disabled_reason(if spinning {
                running.tip.clone()
            } else {
                t.reason.clone()
            });
        if let Some(c) = t.shortcut {
            item = item.shortcut(c);
        }
        if item.show(m.ui).clicked() {
            picked.clone_from(&t.action);
        }
    }
    picked
}

/// Where the tools after Push go this frame, from the widths they took when last shown.
struct Shelf {
    /// How many of the spare tools are on the bar; the rest are in the More menu.
    shown: usize,
    overflow: bool,
    /// The gap before the places group when everything fits.
    spacer: f32,
    /// The More button's width with its gap.
    more: f32,
    badge: bool,
    /// The whole bar's width with every tool shown, once every tool has been measured.
    full: Option<f32>,
}

/// Whether tools stand icon over caption (Comfortable), which sets their widths.
fn stacked() -> bool {
    theme::metrics().tool_layout == theme::ToolLayout::Stacked
}

impl Shelf {
    fn tool_id(t: &Spare) -> Id {
        Id::new(("toolbar-tool-width", &t.label, stacked()))
    }
    fn divider_id() -> Id {
        Id::new(("toolbar-divider-width", stacked()))
    }
    fn badge_id() -> Id {
        Id::new(("toolbar-badge-width", stacked()))
    }
    fn more_id() -> Id {
        Id::new(("toolbar-more-width", stacked()))
    }

    /// Lays out `tools` after `fixed` points of the bar, up to `end`; `badge` asks for the Local
    /// only badge after them. A tool not yet measured is shown, so it can be.
    fn plan(ctx: &egui::Context, tools: &[Spare], badge: bool, fixed: f32, end: f32) -> Self {
        let width = |id: Id| ctx.data(|d| d.get_temp::<f32>(id));
        let divider = width(Self::divider_id());
        let widths: Vec<Option<f32>> = tools.iter().map(|t| width(Self::tool_id(t))).collect();
        let badge_w = if badge {
            width(Self::badge_id()).zip(divider).map(|(b, d)| b + d)
        } else {
            Some(0.0)
        };
        let has_work = tools.iter().any(|t| t.group == SpareGroup::Work);
        let all: Option<f32> = widths
            .iter()
            .try_fold(0.0, |sum, w| w.map(|w| sum + w))
            .zip(if has_work { divider } else { Some(0.0) })
            .zip(badge_w)
            .map(|((tools, divider), badge)| tools + divider + badge + theme::SM);
        let Some(all) = all else {
            // Something has not been measured yet: show it all once (the pass is discarded).
            return Self {
                shown: tools.len(),
                overflow: false,
                spacer: theme::SM,
                more: 0.0,
                badge,
                full: None,
            };
        };
        let full = Some(fixed + all);
        if fixed + all <= end + 0.5 {
            let places: f32 = tools
                .iter()
                .zip(&widths)
                .filter(|(t, _)| t.group == SpareGroup::Places)
                .filter_map(|(_, w)| *w)
                .sum();
            let before = fixed + all - theme::SM - places - badge_w.unwrap_or(0.0);
            return Self {
                shown: tools.len(),
                overflow: false,
                spacer: (end - before - places - badge_w.unwrap_or(0.0)).max(theme::SM),
                more: 0.0,
                badge,
                full,
            };
        }
        let more = width(Self::more_id()).unwrap_or(0.0);
        let limit = end - more;
        let mut x = fixed;
        let mut group = None;
        let mut shown = 0;
        for (t, w) in tools.iter().zip(&widths) {
            let lead = if group != Some(t.group) && t.group != SpareGroup::Remote {
                divider.unwrap_or(0.0)
            } else {
                0.0
            };
            let w = w.unwrap_or(0.0);
            if x + lead + w > limit + 0.5 {
                break;
            }
            x += lead + w;
            group = Some(t.group);
            shown += 1;
        }
        Self {
            shown,
            overflow: true,
            spacer: 0.0,
            more,
            badge: false,
            full,
        }
    }
}

/// Repository status shares the tools' density and alignment, without a button or badge ground.
fn local_status(ui: &mut Ui) {
    let m = theme::metrics();
    let p = palette(ui.ctx());
    let stacked = m.tool_layout == theme::ToolLayout::Stacked;
    let glyph = ui.painter().layout_no_wrap(
        icon::CLOUD_SLASH.to_owned(),
        theme::icon_font(m.icon_tool, false),
        p.text_dim,
    );
    let caption = ui.painter().layout_no_wrap(
        "Local only".to_owned(),
        egui::FontId::new(m.tool_caption, theme::medium()),
        p.text_dim,
    );
    let width = if stacked {
        glyph.size().x.max(caption.size().x)
    } else {
        glyph.size().x + m.tool_gap + caption.size().x
    } + 2.0 * m.tool_pad;
    let (rect, response) = ui.allocate_exact_size(vec2(width, m.tool_height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Local only"));
    let (glyph_pos, caption_pos) = if stacked {
        let top = rect.center().y - (glyph.size().y + m.tool_gap + caption.size().y) / 2.0;
        (
            pos2(rect.center().x - glyph.size().x / 2.0, top),
            pos2(
                rect.center().x - caption.size().x / 2.0,
                top + glyph.size().y + m.tool_gap,
            ),
        )
    } else {
        (
            pos2(
                rect.left() + m.tool_pad,
                rect.center().y - glyph.size().y / 2.0,
            ),
            pos2(
                rect.left() + m.tool_pad + glyph.size().x + m.tool_gap,
                rect.center().y - caption.size().y / 2.0,
            ),
        )
    };
    ui.painter().galley(glyph_pos, glyph, p.text_dim);
    ui.painter().galley(caption_pos, caption, p.text_dim);
    response
        .kit_tooltip("This repository has no remotes. Use Publish or Add remote to connect one.")
        .show();
}

/// The History / Changes / Requests switcher (design spec 4.8 ViewTabs): the kit's view tabs with
/// counts on the strong badge, each view's shortcut in its tooltip, Requests off (saying why)
/// without a GitHub or GitLab remote (`Err` holds the repository's remotes, `None` until they are
/// loaded), and when `compact` only the active view named.
fn view_tabs(
    ui: &mut Ui,
    view: View,
    changes: usize,
    requests: Result<(ForgeKind, usize), Option<&[RemoteInfo]>>,
    compact: bool,
    actions: &mut Vec<Action>,
) {
    let (kind, n) = requests.unwrap_or((ForgeKind::GitHub, 0));
    let reason = requests.err().map(requests_reason);
    let requests_label = match (compact, kind) {
        (true, _) => "Requests",
        (false, ForgeKind::GitHub) => "Pull requests",
        (false, ForgeKind::GitLab) => "Merge requests",
    };
    let options = [
        (View::History, icon::GRAPH, "History"),
        (View::Changes, icon::PENCIL_SIMPLE_LINE, "Changes"),
        (View::Requests, icon::GIT_PULL_REQUEST, requests_label),
    ];
    let commands = [Command::History, Command::Changes, Command::Requests];
    let mut chosen = view;
    let mut tabs = ViewTabs::with_icons(&mut chosen, &options)
        .compact(compact)
        .strong_badges(true)
        .badge(1, changes)
        .badge(2, n)
        // The tools after the switcher keep their place as the counts come and go (a commit
        // empties Changes while the pointer is on Push).
        .badge_room(1)
        .label("View")
        .id_salt(VIEW_TABS)
        .option_enabled(2, requests.is_ok(), reason.as_deref());
    if requests.is_ok() {
        tabs = tabs.badge_room(2);
    }
    for (i, ((_, _, label), command)) in options.iter().zip(commands).enumerate() {
        tabs = tabs.option_tooltip(i, *label).option_shortcut(i, command);
    }
    if tabs.show(ui).changed() && chosen != view {
        actions.push(Action::SetView(chosen));
    }
}

/// Why Requests is off: the repository's `remotes` (`None` before they are loaded) include no
/// GitHub or GitLab remote.
fn requests_reason(remotes: Option<&[RemoteInfo]>) -> String {
    const SELF_HOSTED: &str = "For a self-hosted GitLab, list its host in Settings → Integrations.";
    match remotes {
        None => "Loading the repository's remotes.".into(),
        Some([]) => {
            "No remote. Add a GitHub or GitLab remote to use pull and merge requests.".into()
        }
        Some([only]) => format!(
            "Remote {} is not on GitHub or GitLab. {SELF_HOSTED}",
            only.name
        ),
        Some(several) => {
            let (shown, hidden) = several.split_at(several.len().min(3));
            let mut names: Vec<&str> = shown.iter().map(|r| r.name.as_str()).collect();
            let more = format!("{} more", hidden.len());
            if !hidden.is_empty() {
                names.push(&more);
            }
            let last = names.pop().unwrap_or_default();
            format!(
                "Remotes {} and {last} are not on GitHub or GitLab. {SELF_HOSTED}",
                names.join(", ")
            )
        }
    }
}

/// The view switcher's id salt.
const VIEW_TABS: &str = "view-tabs";

/// The open requests the list shows, when it says: all of it under Open, its open ones under All.
/// Under Closed, while it loads and after a failure it says nothing.
fn open_in_list(
    filter: StateFilter,
    list: Option<&Result<Arc<[RequestSummary]>, String>>,
) -> Option<usize> {
    match (filter, list?.as_ref().ok()?) {
        (StateFilter::Open, l) => Some(l.len()),
        (StateFilter::All, l) => Some(l.iter().filter(|r| r.state == RequestState::Open).count()),
        (StateFilter::Closed, _) => None,
    }
}

/// The operation that pops the latest stash, as the keyboard and the palette run it: by the
/// stash's commit, so a stash list that changed since the snapshot never pops another stash. With
/// no stash known, plain `git stash pop`, so git says why nothing happened.
pub fn pop_latest(tab: &RepoTab) -> Option<Op> {
    Some(match tab.snap.as_ref()?.stashes.first() {
        Some(latest) => Op::StashApplyCommit {
            oid: latest.oid,
            pop: true,
        },
        None => Op::StashApply {
            index: 0,
            pop: true,
        },
    })
}

/// The count on the Requests view: what the list says, else the last count it gave for this
/// repository, so the badge stays while the list reloads or shows closed requests. A failed list
/// clears it.
fn open_requests(ctx: &egui::Context, tab: &RepoTab) -> usize {
    let id = Id::new(("toolbar-open-requests", &tab.path));
    let list = tab.requests.list.as_ref();
    if matches!(list, Some(Err(_))) {
        ctx.data_mut(|d| d.remove_temp::<usize>(id));
        return 0;
    }
    match open_in_list(tab.requests.filter, list) {
        Some(n) => {
            ctx.data_mut(|d| d.insert_temp(id, n));
            n
        }
        None => ctx.data(|d| d.get_temp(id)).unwrap_or(0),
    }
}

/// An action-bar tool whose operation can be under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Fetch,
    Pull,
    Push,
    Publish,
    Branch,
    Merge,
    Stash,
    Pop,
}

/// The toolbar action that owns an operation, independent of its display wording.
fn tool_of(kind: &OpKind) -> Option<Tool> {
    Some(match kind {
        OpKind::Fetch => Tool::Fetch,
        OpKind::Pull => Tool::Pull,
        OpKind::Push | OpKind::ForcePush(_) => Tool::Push,
        OpKind::Stash => Tool::Stash,
        OpKind::Pop => Tool::Pop,
        OpKind::Merge | OpKind::Squash(_) => Tool::Merge,
        OpKind::Branch => Tool::Branch,
        OpKind::Publish => Tool::Publish,
        _ => return None,
    })
}

/// How a tool's operation ended, while the tool still shows it.
#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    tool: Tool,
    ok: bool,
    /// When it ended, on the context's clock (`InputState::time`), as the kit's
    /// [`Operation::finished`] takes it.
    at: f64,
    /// "Fetched", "Could not push".
    tip: String,
}

impl Outcome {
    /// The outcome `finished` still shows `seconds` after it, at `now` on the context's clock,
    /// or `None` once [`motion::OUTCOME_HOLD`] has passed.
    fn after(finished: &Finished, seconds: f32, now: f64) -> Option<Self> {
        let tool = tool_of(&finished.kind)?;
        if seconds >= motion::OUTCOME_HOLD {
            return None;
        }
        let tip = crate::tab::outcome_title(&finished.label, finished.ok);
        Some(Self {
            tool,
            ok: finished.ok,
            at: now - f64::from(seconds),
            tip,
        })
    }

    fn operation(&self) -> Operation {
        Operation::finished(self.ok, self.at)
    }
}

/// The operation the action bar is running, or how the last one ended, as its tools show it
/// through the kit's [`Operation`] (a spinner, then a check or a warning mark, in place).
struct Running {
    tool: Option<Tool>,
    /// "Fetching…  Receiving objects: 45%".
    tip: String,
    outcome: Option<Outcome>,
}

impl Running {
    fn of(ctx: &egui::Context, tab: &RepoTab) -> Self {
        let job = tab.running.as_deref();
        let mut tip = job.map_or_else(String::new, |j| format!("{j}…"));
        if let Some(progress) = tab.progress.as_deref().filter(|_| job.is_some()) {
            let _ = write!(tip, "  {progress}");
        }
        let now = ctx.input(|i| i.time);
        let outcome = tab
            .finished
            .as_ref()
            .filter(|_| job.is_none())
            .and_then(|f| Outcome::after(f, f.at.elapsed().as_secs_f32(), now));
        Self {
            tool: job.and(tab.running_kind.as_ref()).and_then(tool_of),
            tip,
            outcome,
        }
    }

    /// Shows `button`, the `tool` with `shortcut`; returns whether it was clicked. While the
    /// tool's own operation runs it spins on the accent ground, its tooltip is the progress, and a
    /// click does nothing. Just after, the kit shows how it ended in the icon's place.
    fn show(&self, ui: &mut Ui, button: ToolButton, tool: Tool, shortcut: Option<Command>) -> bool {
        if self.tool == Some(tool) {
            let _ = button
                .enabled(true)
                .toggled(true)
                .operation(Operation::Running)
                .tooltip(self.tip.clone())
                .show(ui);
            return false;
        }
        let button = match shortcut {
            Some(c) => button.shortcut(c),
            None => button,
        };
        match self.outcome.as_ref().filter(|o| o.tool == tool) {
            Some(outcome) => button
                .operation(outcome.operation())
                .tooltip(outcome.tip.clone())
                .show(ui)
                .clicked(),
            None => button.show(ui).clicked(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use egui::Rect;
    use egui::accesskit::{Node, Role};

    use super::*;
    use crate::settings::DiffPrefs;
    use crate::worker::Worker;

    const SCREEN: Rect = Rect::from_min_max(egui::Pos2::ZERO, pos2(1200.0, 100.0));

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.enable_accesskit();
        ctx
    }

    /// One frame of `add` with `events`: what it painted and the AccessKit nodes it reported.
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        add: impl FnMut(&mut Ui),
    ) -> (Vec<egui::epaint::ClippedShape>, Vec<Node>) {
        frame_in(ctx, SCREEN, events, add)
    }

    /// [`frame`] on a `screen` of its own (room for a menu under the bar).
    fn frame_in(
        ctx: &egui::Context,
        screen: Rect,
        events: Vec<egui::Event>,
        add: impl FnMut(&mut Ui),
    ) -> (Vec<egui::epaint::ClippedShape>, Vec<Node>) {
        let mut add = add;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ui| add(ui),
        );
        out.textures_delta.clear();
        let nodes = out
            .platform_output
            .accesskit_update
            .map(|u| u.nodes.into_iter().map(|(_, n)| n).collect())
            .unwrap_or_default();
        (out.shapes, nodes)
    }

    fn node<'a>(nodes: &'a [Node], role: Role, label: &str) -> &'a Node {
        nodes
            .iter()
            .find(|n| n.role() == role && n.label() == Some(label))
            .unwrap_or_else(|| panic!("no {role:?} named {label}"))
    }

    fn node_rect(n: &Node) -> Rect {
        let b = n.bounds().expect("bounds");
        Rect::from_min_max(
            pos2(crate::ui::to_f32(b.x0), crate::ui::to_f32(b.y0)),
            pos2(crate::ui::to_f32(b.x1), crate::ui::to_f32(b.y1)),
        )
    }

    /// Where `text` was painted.
    fn text_rect(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<Rect> {
        fn find(shape: &egui::Shape, text: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        shapes.iter().find_map(|c| find(&c.shape, text))
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

    fn repo_tab(name: &str) -> RepoTab {
        let (worker, _, _) = Worker::detached();
        RepoTab::with_worker(PathBuf::from(name), 100, DiffPrefs::default(), worker)
    }

    #[test]
    fn view_labels_fit_their_controls_with_counts() {
        let ctx = context();
        let (shapes, nodes) = frame(&ctx, Vec::new(), |ui| {
            view_tabs(
                ui,
                View::Requests,
                123,
                Ok((ForgeKind::GitLab, 123)),
                false,
                &mut Vec::new(),
            );
        });
        for label in ["History", "Changes", "Merge requests"] {
            let control = node_rect(node(&nodes, Role::RadioButton, label));
            let text = text_rect(&shapes, label).expect("view label");
            assert!(control.contains_rect(text), "{label} overflows its control");
        }
    }

    /// usersim F6: the tools after the switcher stay put as the Changes and Requests counts come
    /// and go (a commit empties Changes while the pointer is on Push).
    #[test]
    fn the_switcher_keeps_its_width_as_the_counts_come_and_go() {
        let ctx = context();
        let right = |changes: usize, requests: usize| {
            let mut edge = 0.0;
            for _ in 0..2 {
                frame(&ctx, Vec::new(), |ui| {
                    view_tabs(
                        ui,
                        View::History,
                        changes,
                        Ok((ForgeKind::GitHub, requests)),
                        false,
                        &mut Vec::new(),
                    );
                    edge = ui.min_rect().right();
                });
            }
            edge
        };
        let edges: Vec<f32> = [(0, 0), (4, 0), (12, 3), (0, 12), (99, 99), (150, 150)]
            .iter()
            .map(|(c, r)| right(*c, *r))
            .collect();
        assert!(
            edges.iter().all(|e| (e - edges[0]).abs() < 0.5),
            "the switcher's right edge moved: {edges:?}"
        );
    }

    #[test]
    fn requests_without_a_forge_cannot_be_chosen() {
        let ctx = context();
        let mut actions = Vec::new();
        let mut nodes = Vec::new();
        for _ in 0..2 {
            nodes = frame(&ctx, Vec::new(), |ui| {
                view_tabs(ui, View::History, 0, Err(Some(&[])), false, &mut actions);
            })
            .1;
        }
        let requests = node(&nodes, Role::RadioButton, "Pull requests");
        assert!(requests.is_disabled(), "Requests is spoken as unavailable");
        let at = node_rect(requests).center();
        let _ = frame(&ctx, click(at), |ui| {
            view_tabs(ui, View::History, 0, Err(Some(&[])), false, &mut actions);
        });
        assert!(actions.is_empty(), "{actions:?}");
    }

    fn remote(name: &str) -> RemoteInfo {
        RemoteInfo {
            name: name.into(),
            url: Some(format!("https://example.com/{name}.git")),
            default_branch: None,
        }
    }

    /// #218, #222: Requests says once what is missing and what to do, naming the remotes it looked
    /// at, and does not claim there is no remote before they are loaded.
    #[test]
    fn a_disabled_requests_tab_says_what_is_missing() {
        let ctx = context();
        let hint = "list its host in Settings → Integrations.";
        let four = ["origin", "upstream", "fork", "backup"].map(remote);
        for (remotes, says) in [
            (None, "Loading the repository's remotes.".to_owned()),
            (
                Some(&[][..]),
                "No remote. Add a GitHub or GitLab remote to use pull and merge requests."
                    .to_owned(),
            ),
            (
                Some(&four[..1]),
                format!(
                    "Remote origin is not on GitHub or GitLab. For a self-hosted GitLab, {hint}"
                ),
            ),
            (
                Some(&four[..2]),
                format!(
                    "Remotes origin and upstream are not on GitHub or GitLab. For a self-hosted GitLab, {hint}"
                ),
            ),
            (
                Some(&four[..]),
                format!(
                    "Remotes origin, upstream, fork and 1 more are not on GitHub or GitLab. For a self-hosted GitLab, {hint}"
                ),
            ),
        ] {
            let mut nodes = Vec::new();
            for _ in 0..2 {
                nodes = frame(&ctx, Vec::new(), |ui| {
                    view_tabs(ui, View::History, 0, Err(remotes), false, &mut Vec::new());
                })
                .1;
            }
            let why = node(&nodes, Role::RadioButton, "Pull requests")
                .description()
                .unwrap_or_default();
            assert_eq!(why, says, "{remotes:?}");
        }
    }

    #[test]
    fn a_click_on_another_view_switches_to_it() {
        let ctx = context();
        let mut actions = Vec::new();
        let mut nodes = Vec::new();
        for _ in 0..2 {
            nodes = frame(&ctx, Vec::new(), |ui| {
                view_tabs(ui, View::History, 4, Err(Some(&[])), false, &mut actions);
            })
            .1;
        }
        let at = node_rect(node(&nodes, Role::RadioButton, "Changes")).center();
        let _ = frame(&ctx, click(at), |ui| {
            view_tabs(ui, View::History, 4, Err(Some(&[])), false, &mut actions);
        });
        assert!(
            matches!(actions.as_slice(), [Action::SetView(View::Changes)]),
            "{actions:?}"
        );
    }

    fn summary(number: u64, state: RequestState) -> RequestSummary {
        RequestSummary {
            number,
            title: format!("Request {number}"),
            author: "maya".into(),
            state,
            draft: false,
            source_branch: "topic".into(),
            target_branch: "main".into(),
            from_fork: false,
            created_at: 0,
            updated_at: 0,
            web_url: String::new(),
            labels: Vec::new(),
            description: String::new(),
            rollup: None,
        }
    }

    /// Fixture F7: the Requests count is the open requests, whichever filter the list shows.
    #[test]
    fn the_open_count_comes_from_open_and_all_lists_only() {
        let list: Arc<[RequestSummary]> = vec![
            summary(1, RequestState::Open),
            summary(2, RequestState::Merged),
            summary(3, RequestState::Open),
            summary(4, RequestState::Closed),
        ]
        .into();
        let ok = Ok(Arc::clone(&list));
        assert_eq!(open_in_list(StateFilter::Open, Some(&ok)), Some(4));
        assert_eq!(open_in_list(StateFilter::All, Some(&ok)), Some(2));
        assert_eq!(open_in_list(StateFilter::Closed, Some(&ok)), None);
        assert_eq!(open_in_list(StateFilter::Open, None), None);
        assert_eq!(
            open_in_list(StateFilter::Open, Some(&Err("401".into()))),
            None
        );
    }

    /// Fixture F7: the badge keeps the last count while the list reloads or shows closed ones,
    /// and a failed list clears it.
    #[test]
    fn the_requests_badge_survives_loading_and_the_closed_filter() {
        let ctx = context();
        let mut tab = repo_tab("alpha");
        let open: Arc<[RequestSummary]> = vec![
            summary(1, RequestState::Open),
            summary(2, RequestState::Open),
        ]
        .into();
        tab.requests.filter = StateFilter::Open;
        tab.requests.list = Some(Ok(open));
        assert_eq!(open_requests(&ctx, &tab), 2);
        tab.requests.list = None;
        assert_eq!(open_requests(&ctx, &tab), 2, "loading");
        tab.requests.filter = StateFilter::Closed;
        tab.requests.list = Some(Ok(vec![summary(3, RequestState::Merged)].into()));
        assert_eq!(open_requests(&ctx, &tab), 2, "closed filter");
        tab.requests.list = Some(Err("Not Found".into()));
        assert_eq!(open_requests(&ctx, &tab), 0, "failed");
        tab.requests.list = None;
        assert_eq!(open_requests(&ctx, &tab), 0, "a failure forgets the count");
    }

    /// Each running operation's kind lights the tool that started it, independent of its label.
    #[test]
    fn running_operations_map_to_their_tools() {
        use canopy_core::MergeMode;
        use canopy_core::ops::PullStrategy;
        let cases = [
            (
                Op::Fetch {
                    prune: false,
                    quiet: false,
                },
                Some(Tool::Fetch),
            ),
            (
                Op::Pull {
                    strategy: PullStrategy::default(),
                    follow_config: true,
                    prune: true,
                    checked: None,
                },
                Some(Tool::Pull),
            ),
            (Op::Push, Some(Tool::Push)),
            (
                Op::ForcePush(canopy_core::PushTarget {
                    branch: "main".into(),
                    remote: "origin".into(),
                    dest: "main".into(),
                    dest_ref: "refs/heads/main".into(),
                    seen: None,
                }),
                Some(Tool::Push),
            ),
            (
                Op::StashSave {
                    message: None,
                    untracked: true,
                },
                Some(Tool::Stash),
            ),
            (
                Op::StashApply {
                    index: 0,
                    pop: true,
                },
                Some(Tool::Pop),
            ),
            (
                Op::StashApply {
                    index: 0,
                    pop: false,
                },
                None,
            ),
            (
                Op::StashApplyCommit {
                    oid: canopy_core::Oid::ZERO_SHA1,
                    pop: true,
                },
                Some(Tool::Pop),
            ),
            (
                Op::StashApplyCommit {
                    oid: canopy_core::Oid::ZERO_SHA1,
                    pop: false,
                },
                None,
            ),
            (
                Op::Merge("topic".into(), MergeMode::Default),
                Some(Tool::Merge),
            ),
            (
                Op::Merge("topic".into(), MergeMode::Squash),
                Some(Tool::Merge),
            ),
            (Op::Rebase("main".into()), Some(Tool::Merge)),
            (
                Op::CreateBranch {
                    name: "topic".into(),
                    start: None,
                    checkout: true,
                },
                Some(Tool::Branch),
            ),
            (
                Op::AddRemote {
                    name: "origin".into(),
                    url: "https://example.com/r.git".into(),
                },
                Some(Tool::Publish),
            ),
            (
                Op::PushTag {
                    remote: "origin".into(),
                    tag: "v1".into(),
                },
                None,
            ),
            (
                Op::Commit {
                    message: "m".into(),
                    amend: false,
                    signoff: false,
                },
                None,
            ),
        ];
        for (op, tool) in cases {
            assert_eq!(tool_of(&op.kind()), tool, "{}", op.label());
        }
    }

    /// V30: the running tool spins and swallows clicks; the others act as usual.
    #[test]
    fn the_running_tool_spins_and_ignores_clicks() {
        let ctx = context();
        let running = Running {
            tool: Some(Tool::Fetch),
            tip: "Fetching…".into(),
            outcome: None,
        };
        let idle = Running {
            tool: None,
            tip: String::new(),
            outcome: None,
        };
        let fetch = || ToolButton::new(icon::ARROWS_CLOCKWISE, "Fetch");
        for (state, expected) in [(&running, false), (&idle, true)] {
            let mut nodes = Vec::new();
            for _ in 0..2 {
                nodes = frame(&ctx, Vec::new(), |ui| {
                    let _ = state.show(ui, fetch(), Tool::Fetch, None);
                })
                .1;
            }
            let at = node_rect(node(&nodes, Role::Button, "Fetch")).center();
            let mut clicked = false;
            let _ = frame(&ctx, click(at), |ui| {
                clicked |= state.show(ui, fetch(), Tool::Fetch, None);
            });
            assert_eq!(clicked, expected);
            // Let the click end before the next case.
            let _ = frame(&ctx, Vec::new(), |_| {});
        }
    }

    /// H1 (kit-b F1): the title bar's tabs are the kit's: a tab list with each repository, and a
    /// busy one keeps its width instead of overlapping the next.
    #[test]
    fn the_title_bar_tabs_announce_each_repository_and_a_busy_one_keeps_its_room() {
        let ctx = context();
        let mut alpha = repo_tab("alpha");
        alpha.running = Some("Fetching".into());
        let tabs = [alpha, repo_tab("beta")];
        let mut actions = Vec::new();
        let (shapes, nodes) = frame(&ctx, Vec::new(), |ui| {
            super::tabs(ui, &tabs, 1, ThemeId::Midnight, &mut actions);
        });
        assert!(
            nodes
                .iter()
                .any(|n| n.role() == Role::TabList && n.label() == Some("Repositories"))
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.role() == Role::Tab && n.label().is_some_and(|l| l.starts_with("beta"))),
            "the tab is spoken"
        );
        let a = text_rect(&shapes, "alpha").expect("alpha");
        let b = text_rect(&shapes, "beta").expect("beta");
        assert!(
            a.right() + 40.0 < b.left(),
            "the busy tab overlaps the next: {a:?} {b:?}"
        );
    }

    /// The strip's tools take Tab from left to right, the way they read: search, theme, settings,
    /// keyboard shortcuts.
    #[test]
    fn the_title_bar_tools_take_tab_from_left_to_right() {
        let ctx = context();
        let tabs = [repo_tab("alpha")];
        let mut show = |ui: &mut Ui| super::tabs(ui, &tabs, 0, ThemeId::Midnight, &mut Vec::new());
        for _ in 0..3 {
            frame(&ctx, Vec::new(), &mut show);
        }
        let mut stops = Vec::new();
        for _ in 0..12 {
            frame(
                &ctx,
                crate::ui::kit::testkit::key(egui::Key::Tab),
                &mut show,
            );
            frame(&ctx, Vec::new(), &mut show);
            if let Some(r) = ctx
                .memory(egui::Memory::focused)
                .and_then(|id| ctx.read_response(id))
            {
                stops.push(r.rect.center().x);
            }
        }
        // The tools sit at the right; the tabs and the + are to their left.
        let tools: Vec<_> = stops
            .iter()
            .copied()
            .filter(|x| *x > 500.0)
            .take(4)
            .collect();
        assert!(
            tools.len() == 4 && tools.windows(2).all(|w| w[0] < w[1]),
            "Tab runs against the reading order: {stops:?}"
        );
    }

    /// A repository's tab shows the icon chosen for it in its settings, not always the folder.
    #[test]
    fn a_title_bar_tab_shows_the_repository_icon() {
        let ctx = context();
        let mut alpha = repo_tab("alpha");
        alpha.icon = crate::tab::tab_icon(Some("code"));
        let tabs = [alpha];
        let mut actions = Vec::new();
        let (shapes, _) = frame(&ctx, Vec::new(), |ui| {
            super::tabs(ui, &tabs, 0, ThemeId::Midnight, &mut actions);
        });
        assert!(
            text_rect(&shapes, egui_phosphor::regular::CODE).is_some(),
            "the chosen icon"
        );
        assert!(
            text_rect(&shapes, egui_phosphor::regular::FOLDER_SIMPLE).is_none(),
            "the default folder instead of the chosen icon"
        );
    }

    /// Override O1: how an operation ended holds on its tool for `motion::OUTCOME_HOLD`, as
    /// the kit's operation state, then is gone.
    #[test]
    fn an_outcome_holds_on_its_tool_then_ends() {
        let done = Finished {
            label: "Fetching".into(),
            kind: OpKind::Fetch,
            ok: true,
            at: std::time::Instant::now(),
        };
        let held = Outcome::after(&done, motion::OUTCOME_HOLD / 2.0, 10.0).expect("held");
        assert_eq!(held.tool, Tool::Fetch);
        assert_eq!(
            held.operation(),
            Operation::Succeeded {
                at: 10.0 - f64::from(motion::OUTCOME_HOLD / 2.0)
            }
        );
        assert_eq!(held.tip, "Fetched");
        assert!(
            Outcome::after(&done, motion::OUTCOME_HOLD, 10.0).is_none(),
            "gone"
        );
        let failed = Finished {
            label: "Pushing".into(),
            kind: OpKind::Push,
            ok: false,
            ..done
        };
        let shown = Outcome::after(&failed, 0.0, 3.0).expect("shown");
        assert_eq!(shown.tool, Tool::Push);
        assert_eq!(shown.operation(), Operation::Failed { at: 3.0 });
        assert_eq!(shown.tip, "Could not push");
        let other = Finished {
            label: "Committing".into(),
            kind: OpKind::Commit,
            ok: true,
            at: std::time::Instant::now(),
        };
        assert!(Outcome::after(&other, 0.0, 0.0).is_none(), "not a tool");
    }

    /// Override O1, end to end from the worker's events: busy while it runs, then failed on the
    /// same tool; a quiet (auto) fetch leaves no mark.
    #[test]
    fn the_worker_events_drive_the_tool_from_busy_to_its_outcome() {
        use crate::worker::{Event, OpKind};
        let ctx = context();
        let (worker, events, _jobs) = Worker::detached();
        let mut tab =
            RepoTab::with_worker(PathBuf::from("alpha"), 100, DiffPrefs::default(), worker);
        events
            .send(Event::OpStarted {
                label: "Fetching".into(),
                kind: OpKind::Fetch,
            })
            .expect("started");
        tab.poll(&[]);
        let busy = Running::of(&ctx, &tab);
        assert_eq!(busy.tool, Some(Tool::Fetch));
        assert!(busy.outcome.is_none());
        events
            .send(Event::OpFinished {
                label: "Fetching".into(),
                kind: OpKind::Fetch,
                announce: true,
                result: Err("fatal: unable to access".into()),
            })
            .expect("finished");
        tab.poll(&[]);
        let after = Running::of(&ctx, &tab);
        assert_eq!(after.tool, None);
        let outcome = after.outcome.expect("outcome");
        assert_eq!(outcome.tool, Tool::Fetch);
        assert!(matches!(outcome.operation(), Operation::Failed { .. }));

        let (worker, events, _jobs) = Worker::detached();
        let mut quiet =
            RepoTab::with_worker(PathBuf::from("beta"), 100, DiffPrefs::default(), worker);
        events
            .send(Event::OpFinished {
                label: "Fetching".into(),
                kind: OpKind::Fetch,
                announce: false,
                result: Ok(String::new()),
            })
            .expect("finished");
        quiet.poll(&[]);
        assert!(Running::of(&ctx, &quiet).outcome.is_none());
    }

    /// F15: in a narrow window every action is either shown whole on the bar or listed in the
    /// More menu at its right end, never cut by the edge; the view switcher and the remote tools
    /// stay on the bar, and a wide bar needs no menu.
    #[test]
    fn every_action_is_on_the_bar_or_in_the_more_menu() {
        const MOVABLE: [&str; 7] = [
            "Branch", "Worktree", "Merge", "Stash", "Pop", "Terminal", "Files",
        ];
        const STAYING: [&str; 4] = ["History", "Fetch", "Pull", "Publish"];
        let screen = Rect::from_min_max(egui::Pos2::ZERO, pos2(1500.0, 700.0));
        for density in [theme::Density::Comfortable, theme::Density::Dense] {
            for width in [1440.0, 900.0, 760.0, 640.0] {
                let ctx = context();
                theme::apply(
                    &ctx,
                    ThemeId::Midnight,
                    theme::Appearance {
                        density,
                        ..theme::Appearance::STILL
                    },
                );
                let mut tab = repo_tab("alpha");
                tab.snap = Some(Arc::new(canopy_core::RepoSnapshot::default()));
                let mut show = |ui: &mut Ui| {
                    let slot = Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 60.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                        actions_bar(ui, &tab, &mut Vec::new());
                    });
                };
                let mut out = (Vec::new(), Vec::new());
                for _ in 0..3 {
                    out = frame_in(&ctx, screen, Vec::new(), &mut show);
                }
                let (shapes, nodes) = out;
                // A disabled tool is announced as "Pop, unavailable".
                let named = |n: &Node, label: &str| {
                    n.label()
                        .is_some_and(|l| l == label || l.starts_with(&format!("{label},")))
                };
                let on_bar = |label: &str| {
                    nodes.iter().any(|n| {
                        named(n, label)
                            && n.bounds().is_some()
                            && node_rect(n).right() <= width + 0.5
                            && node_rect(n).bottom() <= 60.5
                    })
                };
                for label in STAYING {
                    assert!(
                        on_bar(label),
                        "{label} left the bar at {width} ({density:?})"
                    );
                }
                let more = nodes
                    .iter()
                    .find(|n| n.label() == Some("More actions"))
                    .map(node_rect);
                let listed: Vec<Node> = more.map_or_else(Vec::new, |more| {
                    assert!(
                        more.right() <= width + 0.5,
                        "More is cut at {width} ({density:?})"
                    );
                    frame_in(&ctx, screen, click(more.center()), &mut show);
                    let (_, open) = frame_in(&ctx, screen, Vec::new(), &mut show);
                    open.into_iter()
                        .filter(|n| n.role() == Role::MenuItem)
                        .collect()
                });
                for label in MOVABLE {
                    let shown = on_bar(label)
                        && text_rect(&shapes, label).is_some_and(|r| r.right() <= width + 0.5);
                    let in_menu = listed.iter().any(|n| named(n, label));
                    assert!(
                        shown != in_menu,
                        "{label} at {width} ({density:?}): on the bar {shown}, in the menu \
                         {in_menu}"
                    );
                }
                if width >= 1440.0 {
                    assert!(more.is_none(), "a wide bar has a More menu ({density:?})");
                }
                if width <= 640.0 {
                    assert!(more.is_some(), "no More menu at {width} ({density:?})");
                }
            }
        }
    }

    /// A tool keeps its own id whatever the width, and More never takes a tool's: focus on More
    /// must not pass to Pop when the window widens and Pop comes back (Enter would pop a stash).
    #[test]
    fn every_tool_keeps_its_own_id_at_every_width() {
        let screen = Rect::from_min_max(egui::Pos2::ZERO, pos2(1500.0, 700.0));
        let mut owners: std::collections::HashMap<egui::accesskit::NodeId, String> =
            std::collections::HashMap::new();
        let ctx = context();
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let mut tab = repo_tab("alpha");
        tab.snap = Some(Arc::new(canopy_core::RepoSnapshot::default()));
        for width in [1440.0, 900.0, 760.0, 640.0, 560.0, 900.0, 1440.0] {
            let mut out = None;
            for _ in 0..3 {
                if let Some(mut old) = out.take() {
                    let old: &mut egui::FullOutput = &mut old;
                    old.textures_delta.clear();
                }
                out = Some(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| {
                        let slot = Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 60.0));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                            actions_bar(ui, &tab, &mut Vec::new());
                        });
                    },
                ));
            }
            let mut out = out.expect("a frame");
            out.textures_delta.clear();
            for (id, n) in out.platform_output.accesskit_update.expect("nodes").nodes {
                if n.role() != Role::Button {
                    continue;
                }
                let Some(label) = n.label() else { continue };
                let name = label.split(',').next().unwrap_or(label).to_owned();
                let owner = owners.entry(id).or_insert_with(|| name.clone());
                assert_eq!(*owner, name, "{name} took {owner}'s id at {width}");
            }
        }
    }

    /// Override O2: an action bar too narrow for every tool names only the active view and drops
    /// the Local only badge (instead of overflowing under a scroll bar), and opens up again when
    /// the room comes back.
    #[test]
    fn a_narrow_action_bar_gives_up_view_names_and_the_badge() {
        let ctx = context();
        let mut tab = repo_tab("alpha");
        tab.snap = Some(Arc::new(canopy_core::RepoSnapshot::default()));
        let bar = |width: f32| {
            let mut shapes = Vec::new();
            for _ in 0..3 {
                shapes = frame(&ctx, Vec::new(), |ui| {
                    let slot = Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 60.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                        actions_bar(ui, &tab, &mut Vec::new());
                    });
                })
                .0;
            }
            shapes
        };
        let painted = |shapes: &[egui::epaint::ClippedShape], text: &str| {
            fn find(shape: &egui::Shape, text: &str) -> bool {
                match shape {
                    egui::Shape::Text(t) => t.galley.text().contains(text),
                    egui::Shape::Vec(v) => v.iter().any(|s| find(s, text)),
                    _ => false,
                }
            }
            shapes.iter().any(|c| find(&c.shape, text))
        };
        let wide = bar(1240.0);
        assert!(painted(&wide, "Changes") && painted(&wide, "Local only"));
        let narrow = bar(560.0);
        assert!(
            painted(&narrow, "History"),
            "the active view keeps its name"
        );
        assert!(
            !painted(&narrow, "Changes"),
            "the other views give up theirs"
        );
        assert!(!painted(&narrow, "Local only"), "the badge goes");
        let wide = bar(1240.0);
        assert!(painted(&wide, "Changes") && painted(&wide, "Local only"));
    }
    /// F1: with Tooltips at the default Helpful, resting the pointer on the toolbar's Fetch and
    /// Pull shows what they do and their shortcuts.
    #[test]
    fn the_fetch_and_pull_tools_show_their_tooltips_at_the_default_level() {
        use crate::ui::kit::overlay::{TooltipLevel, set_tooltip_level};
        fn painted(shapes: &[egui::epaint::ClippedShape], text: &str) -> bool {
            fn find(shape: &egui::Shape, text: &str) -> bool {
                match shape {
                    egui::Shape::Text(t) => t.galley.text() == text,
                    egui::Shape::Vec(v) => v.iter().any(|s| find(s, text)),
                    _ => false,
                }
            }
            shapes.iter().any(|c| find(&c.shape, text))
        }
        set_tooltip_level(TooltipLevel::Helpful);
        let mut tab = repo_tab("alpha");
        tab.snap = Some(Arc::new(canopy_core::RepoSnapshot {
            remotes: vec![RemoteInfo {
                name: "origin".into(),
                url: Some("https://example.com/a.git".into()),
                default_branch: None,
            }],
            ..canopy_core::RepoSnapshot::default()
        }));
        let bar = |ctx: &egui::Context, time: f64, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(SCREEN),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| actions_bar(ui, &tab, &mut Vec::new()),
            );
            out.textures_delta.clear();
            let nodes: Vec<Node> = out
                .platform_output
                .accesskit_update
                .map(|u| u.nodes.into_iter().map(|(_, n)| n).collect())
                .unwrap_or_default();
            (out.shapes, nodes)
        };
        for (tool, tip) in [
            ("Fetch", "Fetch all remotes"),
            ("Pull", "Pull the current branch: Fast-forward only"),
        ] {
            let ctx = context();
            let mut nodes = Vec::new();
            for i in 0..3 {
                nodes = bar(&ctx, f64::from(i) * 0.1, Vec::new()).1;
            }
            let at = node_rect(node(&nodes, Role::Button, tool)).center();
            let mut shapes = Vec::new();
            for i in 0..12 {
                let events = if i == 0 {
                    vec![egui::Event::PointerMoved(at)]
                } else {
                    Vec::new()
                };
                shapes = bar(&ctx, 1.0 + f64::from(i) * 0.2, events).0;
            }
            assert!(painted(&shapes, tip), "{tool} shows {tip:?} at Helpful");
        }
    }

    /// #62: the tooltip of a tab that has not loaded says so; a loaded one does not.
    #[test]
    fn the_tooltip_of_a_tab_that_is_not_loaded_says_so() {
        let ctx = egui::Context::default();
        let unloaded = RepoTab::unloaded(PathBuf::from("alpha"), &ctx, 100, DiffPrefs::default());
        let loaded = repo_tab("beta");
        let note = "Not loaded yet: it loads when you switch to it";
        let tabs = [unloaded, loaded];
        assert!(
            tab_detail(&tabs[0], 0).contains(note),
            "{}",
            tab_detail(&tabs[0], 0)
        );
        assert!(tab_detail(&tabs[0], 0).starts_with("alpha"));
        assert!(!tab_detail(&tabs[1], 1).contains(note));
        assert_eq!(doc_tabs(&tabs).len(), 2);
    }
}
