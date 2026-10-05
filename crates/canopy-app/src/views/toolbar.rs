//! The tab strip and the action bar. Both sit on the top deck that `app` paints (flat chrome in
//! Standard, one lifted surface in Deeper), so nothing here paints a background of its own.

use egui::accesskit::Role;
use egui::text::LayoutJob;
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Id, Layout, Rect, RichText, Sense, Ui,
    UiBuilder, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::keymap::{Command, hint};
use crate::tab::{RepoTab, View};
use crate::ui::art;
use crate::ui::kit::button::tool_divider;
use crate::ui::kit::chip::{Badge, count_text};
use crate::ui::kit::field::SearchField;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::{IconButton, Size, ToolButton, anim, menu, paint};
use crate::ui::theme::{self, Elevation, ThemeId, palette};
use crate::ui::widgets::forge_icon;
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
                .font(FontId::new(m.subtitle_size, theme::semibold()))
                .color(p.text),
        );
        ui.add_space(theme::MD);
        // The tools on the right keep their room; the tabs scroll in the rest.
        let tools_id = ui.id().with("tab-strip-tools");
        let tools_w: f32 = ui.data(|d| d.get_temp(tools_id)).unwrap_or(0.0);
        egui::ScrollArea::horizontal()
            .id_salt("repository-tabs")
            .max_width((ui.available_width() - tools_w - theme::LG).max(m.control * 2.0))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::XS;
                    if let Some(event) = DocTabs::new(tabs, active).show(ui) {
                        actions.push(event);
                    }
                    if IconButton::new(icon::PLUS, "Open repository")
                        .shortcut(Command::Open)
                        .size(Size::Sm)
                        .show(ui)
                        .clicked()
                    {
                        actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::open()));
                    }
                });
            });
        let tools = ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = theme::XS;
            let start = ui.cursor().right();
            ui.add_space(theme::SM);
            if IconButton::new(icon::KEYBOARD, "Keyboard shortcuts")
                .shortcut(Command::Cheatsheet)
                .show(ui)
                .clicked()
            {
                actions.push(Action::Cheatsheet);
            }
            if IconButton::new(icon::GEAR_SIX, "Settings")
                .shortcut(Command::Settings)
                .show(ui)
                .clicked()
            {
                actions.push(Action::Settings);
            }
            let theme_button = IconButton::new(icon::PALETTE, "Theme").show(ui);
            if let Some(Some(t)) = menu::menu(&theme_button, |m| theme_items(m, current_theme)) {
                actions.push(Action::SetTheme(t));
            }
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
            start - ui.min_rect().left()
        });
        let width = tools.inner;
        if (width - tools_w).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(tools_id, width));
            ui.ctx().request_repaint();
        }
    });
}

/// The theme menu: every theme with its swatch (ground and accent), the active one checked.
fn theme_items(m: &mut menu::Menu<'_>, current: ThemeId) -> Option<ThemeId> {
    let metrics = theme::metrics();
    let s = theme::surfaces(m.ui.ctx());
    let mut picked = None;
    for t in ThemeId::ALL {
        // A blank glyph keeps the icon column, where the swatch is painted.
        let r = m.item(" ", t.name()).checked(t == current).show(m.ui);
        let tp = t.palette();
        let size = vec2(metrics.icon_md, metrics.icon_md * 0.75);
        let swatch = Rect::from_min_size(
            pos2(r.rect.left() + theme::SM, r.rect.center().y - size.y / 2.0),
            size,
        );
        let radius = CornerRadius::same(metrics.radius_sm.min(3));
        let painter = m.ui.painter();
        painter.rect_filled(swatch, radius, tp.bg());
        painter.rect_filled(
            Rect::from_min_max(swatch.center_top(), swatch.max),
            CornerRadius {
                nw: 0,
                sw: 0,
                ne: radius.ne,
                se: radius.se,
            },
            tp.accent,
        );
        paint::rim(painter, swatch, radius, s.edge);
        if r.clicked() {
            picked = Some(t);
        }
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
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip("Search commands", Command::Palette));
    r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Search commands"));
    r
}

// ------------------------------------------------------------------------------------ DocTabs

/// Padding inside a document tab: left of the icon, right of the close button.
const TAB_PAD_LEFT: f32 = 10.0;
const TAB_PAD_RIGHT: f32 = 6.0;
/// Gap between a tab's icon, label, counts and close button.
const TAB_GAP: f32 = 7.0;
const TAB_MIN_WIDTH: f32 = 100.0;
const TAB_MAX_WIDTH: f32 = 220.0;
/// The ahead/behind counts (mono, `accent`).
const TAB_COUNT_SIZE: f32 = 11.5;

/// The repository tabs (concept `.h-tab`, design spec 4.8): the active tab is the `tab` fill
/// with its edge, the others transparent until hovered. A busy repository shows a spinner in
/// place of its folder; ahead/behind counts follow the name; the close button shows on hover and
/// on the active tab; a middle click closes too.
struct DocTabs<'a> {
    tabs: &'a [RepoTab],
    active: usize,
}

impl<'a> DocTabs<'a> {
    fn new(tabs: &'a [RepoTab], active: usize) -> Self {
        Self { tabs, active }
    }

    /// Shows the tabs; returns the switch or close the user asked for.
    fn show(self, ui: &mut Ui) -> Option<Action> {
        let mut event = None;
        let list = ui
            .scope_builder(UiBuilder::new().id_salt("doc-tabs"), |ui| {
                ui.spacing_mut().item_spacing.x = theme::XS;
                for (i, tab) in self.tabs.iter().enumerate() {
                    if let Some(e) = doc_tab(ui, i, tab, i == self.active) {
                        event = Some(e);
                    }
                }
            })
            .response;
        ui.ctx().accesskit_node_builder(list.id, |n| {
            n.set_role(Role::TabList);
            n.set_label("Repositories");
        });
        event
    }
}

fn doc_tab(ui: &mut Ui, i: usize, tab: &RepoTab, selected: bool) -> Option<Action> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let style = theme::style();
    let s = p.surfaces(style);
    let m = theme::metrics();
    let font = FontId::new(m.body_size, theme::medium());
    // Commits to push and pull, like the status bar, so every tab shows its sync state.
    let sync = tab
        .snap
        .as_ref()
        .and_then(|s| s.current_branch())
        .filter(|b| b.upstream.is_some() && (b.ahead > 0 || b.behind > 0))
        .map(|b| (b.ahead, b.behind));
    let counts = sync.map(|(ahead, behind)| {
        let mut parts = Vec::new();
        if ahead > 0 {
            parts.push(format!("↑{ahead}"));
        }
        if behind > 0 {
            parts.push(format!("↓{behind}"));
        }
        ui.painter().layout_no_wrap(
            parts.join(" "),
            FontId::new(TAB_COUNT_SIZE, FontFamily::Monospace),
            p.accent,
        )
    });
    let close_side = Size::Xs.height();
    let name_w = ui
        .painter()
        .layout_no_wrap(tab.name.clone(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    let fixed = TAB_PAD_LEFT
        + m.icon_sm
        + TAB_GAP
        + counts.as_ref().map_or(0.0, |g| g.size().x + TAB_GAP)
        + TAB_GAP
        + close_side
        + TAB_PAD_RIGHT;
    let width = (fixed + name_w).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, m.doc_tab_height), Sense::click());
    let hover = anim::ease_bool(
        &ctx,
        resp.id.with("doc-tab-hover"),
        resp.hovered() || ui.rect_contains_pointer(rect),
        crate::ui::motion::FAST,
        crate::ui::motion::FAST_OUT,
    );
    let radius = CornerRadius::same(m.doc_tab_radius);
    let mut event = None;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().clone();
        if selected {
            paint::surface(
                &painter,
                rect,
                radius,
                p,
                style,
                Elevation::Tab,
                s.tab,
                Some(s.tab_edge),
            );
        } else if hover > 0.0 {
            painter.rect_filled(rect, radius, p.hover().gamma_multiply(hover));
        }
        let cy = rect.center().y;
        let mut x = rect.left() + TAB_PAD_LEFT;
        let icon_rect =
            Rect::from_center_size(pos2(x + m.icon_sm / 2.0, cy), Vec2::splat(m.icon_sm));
        if tab.running.is_some() {
            ui.put(
                icon_rect,
                Spinner::new()
                    .size(Size::Xs)
                    .label(tab.name.clone())
                    .id_salt(("doc-tab-busy", i)),
            );
        } else {
            let color = if selected {
                p.text_dim
            } else {
                p.text_faint.lerp_to_gamma(p.text_dim, hover)
            };
            let g = painter.layout_no_wrap(
                icon::FOLDER_SIMPLE.to_owned(),
                FontId::proportional(m.icon_sm),
                color,
            );
            painter.galley(icon_rect.center() - g.size() / 2.0, g, color);
        }
        x += m.icon_sm + TAB_GAP;
        let close_left = rect.right() - TAB_PAD_RIGHT - close_side;
        let counts_w = counts.as_ref().map_or(0.0, |g| g.size().x + TAB_GAP);
        let label_right = close_left - TAB_GAP - counts_w;
        let label_color = if selected {
            p.text
        } else {
            p.text_dim.lerp_to_gamma(p.text, hover)
        };
        let mut job = LayoutJob::simple(
            tab.name.clone(),
            font,
            label_color,
            (label_right - x).max(1.0),
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let g = painter.layout_job(job);
        let label_w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, label_color);
        if let Some(g) = counts {
            painter.galley(
                pos2(x + label_w + TAB_GAP, cy - g.size().y / 2.0),
                g,
                p.accent,
            );
        }
        if selected || hover > 0.0 {
            let at = Rect::from_min_size(
                pos2(close_left, cy - close_side / 2.0),
                Vec2::splat(close_side),
            );
            let mut slot = ui.new_child(UiBuilder::new().max_rect(at));
            if !selected {
                slot.multiply_opacity(hover);
            }
            if slot
                .add(
                    IconButton::new(icon::X, format!("Close {}", tab.name))
                        .size(Size::Xs)
                        .id_salt(("doc-tab-close", i)),
                )
                .clicked()
            {
                event = Some(Action::CloseTab(i));
            }
        }
    }
    if event.is_none() {
        if resp.clicked() {
            event = Some(Action::SwitchTab(i));
        } else if resp.middle_clicked() {
            event = Some(Action::CloseTab(i));
        }
    }
    let sync_text = sync.map_or(String::new(), |(ahead, behind)| {
        format!(", {ahead} to push, {behind} to pull")
    });
    let label = format!("Repository tab {}{sync_text}", tab.name);
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &label));
    ctx.accesskit_node_builder(resp.id, |n| {
        n.set_role(Role::Tab);
        n.clear_toggled();
        n.set_selected(selected);
    });
    if resp.has_focus() && crate::ui::kit::focus_visible(&ctx) {
        paint::inset_ring(ui.painter(), rect, radius, p.focus);
    }
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
    event
}

// ---------------------------------------------------------------------------------- action bar

/// The action bar of the active repository: the view switcher, then the tool groups.
pub fn actions_bar(ui: &mut Ui, tab: &RepoTab, actions: &mut Vec<Action>) {
    let m = theme::metrics();
    let snap = tab.snap.as_ref();
    let has_remote = snap.is_some_and(|s| !s.remotes.is_empty());
    let local_only = tab.is_local_only();
    let busy = tab.running.is_some();
    let branch = snap.and_then(|s| s.current_branch());
    let forge = tab.forge();
    let width = ui.available_width();
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
    ui.style_mut().spacing.scroll.floating = false;
    egui::ScrollArea::horizontal()
        .id_salt("toolbar-actions")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(width);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = m.toolbar_gap;
                ui.add_space(theme::SM);
                let changes = snap.map(|s| s.status.len()).unwrap_or(0);
                let open_requests = match &tab.requests.list {
                    Some(Ok(l)) if tab.requests.filter == canopy_hosting::StateFilter::Open => {
                        l.len()
                    }
                    _ => 0,
                };
                view_tabs(
                    ui,
                    tab.view,
                    changes,
                    forge.map(|(_, f)| (f.kind, open_requests)),
                    actions,
                );
                ui.add_space(theme::SM);
                tool_divider(ui);
                if ToolButton::new(icon::ARROWS_CLOCKWISE, "Fetch")
                    .tooltip("Fetch all remotes")
                    .shortcut(Command::Fetch)
                    .enabled(has_remote && !busy)
                    .disabled_reason(reason(true))
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::Fetch);
                }
                let behind = branch.map(|b| b.behind).unwrap_or(0);
                let mut pull = ToolButton::new(icon::ARROW_FAT_LINE_DOWN, "Pull")
                    .tooltip("Pull the current branch")
                    .shortcut(Command::Pull)
                    .enabled(has_remote && !busy)
                    .disabled_reason(reason(true));
                if behind > 0 {
                    pull = pull.badge(format!("↓{behind}"));
                }
                if pull.show(ui).clicked() {
                    actions.push(Action::Pull);
                }
                let ahead = branch.map(|b| b.ahead).unwrap_or(0);
                if local_only {
                    if ToolButton::new(icon::CLOUD_ARROW_UP, "Publish")
                        .tooltip(
                            "This repository has no remote. Add one on GitHub or GitLab and push",
                        )
                        .enabled(!busy && branch.is_some())
                        .disabled_reason(if busy {
                            busy_reason
                        } else {
                            "Check out a branch to publish"
                        })
                        .show(ui)
                        .clicked()
                    {
                        actions.push(Action::Publish);
                    }
                } else {
                    let mut push = ToolButton::new(icon::ARROW_FAT_LINE_UP, "Push")
                        .tooltip("Push the current branch")
                        .shortcut(Command::Push)
                        .enabled(has_remote && !busy && branch.is_some())
                        .disabled_reason(if branch.is_none() && has_remote && !busy {
                            "Check out a branch to push"
                        } else {
                            reason(true)
                        });
                    if ahead > 0 {
                        push = push.badge(format!("↑{ahead}"));
                    }
                    if push.show(ui).clicked() {
                        actions.push(Action::Push);
                    }
                }
                // Pull/merge request for a pushed branch.
                if let (Some((_, f)), Some(b)) = (forge, branch)
                    && b.upstream.is_some()
                    && !tab.is_default_branch(&b.name)
                    && ToolButton::new(icon::GIT_PULL_REQUEST, f.kind.request_abbrev())
                        .tooltip(format!(
                            "Create a {} for {} on {}",
                            f.kind.request_noun(),
                            b.name,
                            f.kind.name()
                        ))
                        .enabled(!busy)
                        .disabled_reason(busy_reason)
                        .show(ui)
                        .clicked()
                {
                    actions.push(Action::CreateRequest {
                        source: b.name.clone(),
                    });
                }
                tool_divider(ui);
                if ToolButton::new(icon::GIT_BRANCH, "Branch")
                    .tooltip("New branch")
                    .shortcut(Command::NewBranch)
                    .enabled(snap.is_some())
                    .disabled_reason("The repository is still loading")
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::OpenDialog(
                        crate::views::dialogs::Dialog::new_branch_at(
                            crate::views::dialogs::Start::Head,
                        ),
                    ));
                }
                if ToolButton::new(icon::GIT_MERGE, "Merge")
                    .tooltip("Merge or rebase")
                    .shortcut(Command::MergeDialog)
                    .enabled(branch.is_some() && !busy)
                    .disabled_reason(if busy {
                        busy_reason
                    } else {
                        "Check out a branch to merge into"
                    })
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Merge {
                        target: String::new(),
                        rebase: false,
                        mode: canopy_core::MergeMode::Default,
                    }));
                }
                let dirty = snap.is_some_and(|s| !s.status.is_clean());
                if ToolButton::new(icon::ARCHIVE, "Stash")
                    .tooltip("Stash changes")
                    .shortcut(Command::Stash)
                    .enabled(dirty && !busy)
                    .disabled_reason(if busy {
                        busy_reason
                    } else {
                        "Nothing to stash"
                    })
                    .show(ui)
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
                if ToolButton::new(icon::TRAY_ARROW_UP, "Pop")
                    .tooltip(pop_tip)
                    .shortcut(Command::PopStash)
                    .enabled(latest.is_some() && !busy)
                    .disabled_reason(if busy { busy_reason } else { "No stashes" })
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::Op(Op::StashApply {
                        index: 0,
                        pop: true,
                    }));
                }
                // The last group sits at the right end, on its width from the frame before.
                let right_id = ui.id().with("toolbar-right-group");
                let right_w: f32 = ui.data(|d| d.get_temp(right_id)).unwrap_or(0.0);
                ui.add_space(
                    (width - (ui.cursor().min.x - ui.max_rect().min.x) - right_w).max(theme::SM),
                );
                let start = ui.cursor().min.x;
                if ToolButton::new(icon::TERMINAL_WINDOW, "Terminal")
                    .tooltip("Open a terminal here")
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::OpenTerminal(tab.path.clone()));
                }
                if ToolButton::new(icon::FOLDER_OPEN, "Files")
                    .tooltip("Show in file manager")
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::OpenPath(tab.path.clone()));
                }
                if let Some((_, f)) = forge
                    && ToolButton::new(forge_icon(f.kind), f.kind.name())
                        .tooltip(format!("Open {} on {}", f.path, f.kind.name()))
                        .show(ui)
                        .clicked()
                {
                    actions.push(Action::OpenUrl(f.web_url()));
                }
                if local_only && ui.ctx().content_rect().width() >= theme::COMPACT_WIDTH {
                    Badge::new(format!("{}  Local only", icon::HARD_DRIVES))
                        .tooltip("No remote: everything stays on this computer until you publish")
                        .show(ui);
                }
                ui.add_space(theme::SM);
                let used = ui.cursor().min.x - start;
                if (used - right_w).abs() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(right_id, used));
                    ui.ctx().request_repaint();
                }
            });
        });
}

/// One view of the switcher.
struct ViewTab {
    view: View,
    glyph: &'static str,
    label: &'static str,
    count: usize,
    command: Command,
    enabled: bool,
}

/// Id of view tab `index` (its response, for tests).
fn view_tab_id(index: usize) -> Id {
    Id::new(("view-tab", index))
}

/// The History / Changes / Requests switcher (design spec 4.8 ViewTabs): the kit's segmented
/// control, drawn here because its options differ in ways the kit's `Segmented` has no slot for:
/// Requests is disabled without a GitHub or GitLab remote (saying why), each view has its own
/// shortcut tooltip, and in a narrow window only the active view keeps its name.
fn view_tabs(
    ui: &mut Ui,
    view: View,
    changes: usize,
    requests: Option<(canopy_core::forge::ForgeKind, usize)>,
    actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let style = theme::style();
    let s = p.surfaces(style);
    let m = theme::metrics();
    let (kind, n) = requests.unwrap_or((canopy_core::forge::ForgeKind::GitHub, 0));
    let compact = ctx.content_rect().width() < theme::COMPACT_WIDTH;
    let items = [
        ViewTab {
            view: View::History,
            glyph: icon::GRAPH,
            label: "History",
            count: 0,
            command: Command::History,
            enabled: true,
        },
        ViewTab {
            view: View::Changes,
            glyph: icon::PENCIL_SIMPLE_LINE,
            label: "Changes",
            count: changes,
            command: Command::Changes,
            enabled: true,
        },
        ViewTab {
            view: View::Requests,
            glyph: icon::GIT_PULL_REQUEST,
            label: match (compact, kind) {
                (true, _) => "Requests",
                (false, canopy_core::forge::ForgeKind::GitHub) => "Pull requests",
                (false, canopy_core::forge::ForgeKind::GitLab) => "Merge requests",
            },
            count: n,
            command: Command::Requests,
            enabled: requests.is_some(),
        },
    ];
    let font = FontId::new(m.body_size, theme::medium());
    let seg_h = m.control - 2.0;
    let inset = m.segment_track_pad + paint::SEAM;
    struct Laid {
        glyph: std::sync::Arc<egui::Galley>,
        text: Option<std::sync::Arc<egui::Galley>>,
        badge: Option<Badge>,
        width: f32,
    }
    let laid: Vec<Laid> = items
        .iter()
        .map(|it| {
            let glyph = ui.painter().layout_no_wrap(
                it.glyph.to_owned(),
                FontId::proportional(m.icon_md),
                Color32::PLACEHOLDER,
            );
            // Compact: only the active view keeps its name; the others show their glyph and
            // count.
            let text = (!compact || it.view == view).then(|| {
                ui.painter()
                    .layout_no_wrap(it.label.to_owned(), font.clone(), Color32::PLACEHOLDER)
            });
            let badge = (it.count > 0).then(|| Badge::new(count_text(it.count)).mono().strong());
            let parts = [
                Some(glyph.size().x),
                text.as_ref().map(|g| g.size().x),
                badge.as_ref().map(|b| b.measure(ui).x),
            ];
            let k = parts.iter().flatten().count();
            let content: f32 =
                parts.iter().flatten().sum::<f32>() + TAB_GAP * k.saturating_sub(1) as f32;
            Laid {
                glyph,
                text,
                badge,
                width: content + 2.0 * m.segment_pad,
            }
        })
        .collect();
    let total = vec2(
        laid.iter().map(|l| l.width).sum::<f32>() + 2.0 * inset,
        seg_h + 2.0 * inset,
    );
    let (track, group) = ui.allocate_exact_size(total, Sense::hover());
    let mut x = track.left() + inset;
    let rects: Vec<Rect> = laid
        .iter()
        .map(|l| {
            let r = Rect::from_min_size(pos2(x, track.top() + inset), vec2(l.width, seg_h));
            x += l.width;
            r
        })
        .collect();
    let chosen = items.iter().position(|it| it.view == view).unwrap_or(0);
    let responses: Vec<egui::Response> = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let sense = if it.enabled {
                Sense::click()
            } else {
                Sense::hover()
            };
            ui.interact(rects[i], view_tab_id(i), sense)
        })
        .collect();
    if ui.is_rect_visible(track) {
        let painter = ui.painter();
        let track_radius = CornerRadius::same(m.radius.saturating_add(2));
        paint::fill(painter, track, track_radius, theme::Fill::flat(s.track));
        paint::rim(painter, track, track_radius, s.track_edge);
        let radius = CornerRadius::same(m.radius);
        let local = rects[chosen].translate(-track.min.to_vec2());
        let thumb = anim::slide_rect(&ctx, group.id.with("view-thumb"), local)
            .translate(track.min.to_vec2());
        paint::surface(
            painter,
            thumb,
            radius,
            p,
            style,
            Elevation::Thumb,
            s.thumb,
            None,
        );
        for (i, (l, rect)) in laid.iter().zip(&rects).enumerate() {
            let hovered = anim::ease_bool(
                &ctx,
                responses[i].id.with("view-tab-hover"),
                responses[i].hovered() && items[i].enabled,
                crate::ui::motion::FAST,
                crate::ui::motion::FAST_OUT,
            );
            let on = anim::ease(
                &ctx,
                responses[i].id.with("view-tab-on"),
                i == chosen,
                crate::ui::motion::BASE,
            );
            let mut color = p.text_dim.lerp_to_gamma(p.text, on.max(hovered));
            if !items[i].enabled {
                color = color.gamma_multiply(theme::DISABLED_ALPHA);
            }
            let parts_w = l.width - 2.0 * m.segment_pad;
            let cy = rect.center().y;
            let mut x = rect.center().x - parts_w / 2.0;
            painter.galley(pos2(x, cy - l.glyph.size().y / 2.0), l.glyph.clone(), color);
            x += l.glyph.size().x + TAB_GAP;
            if let Some(g) = &l.text {
                painter.galley(pos2(x, cy - g.size().y / 2.0), g.clone(), color);
                x += g.size().x + TAB_GAP;
            }
            if let Some(b) = &l.badge {
                let size = b.measure(ui);
                b.paint(ui, Rect::from_min_size(pos2(x, cy - size.y / 2.0), size));
            }
            if responses[i].has_focus() && crate::ui::kit::focus_visible(&ctx) {
                paint::inset_ring(painter, *rect, radius, p.focus);
            }
        }
    }
    for (i, (it, r)) in items.iter().zip(responses).enumerate() {
        r.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, it.enabled, i == chosen, it.label)
        });
        let tip_text = if it.enabled {
            tip(it.label, it.command)
        } else {
            "Add a GitHub or GitLab remote to view pull or merge requests.".to_owned()
        };
        let r = r.on_hover_text(&tip_text).on_disabled_hover_text(&tip_text);
        if it.enabled && r.clicked() && it.view != view {
            actions.push(Action::SetView(it.view));
        }
    }
    group.widget_info(|| WidgetInfo::labeled(WidgetType::RadioGroup, true, "View"));
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
                view_tabs(
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
                .read_response(view_tab_id(index))
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

    #[test]
    fn requests_without_a_forge_cannot_be_chosen() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 100.0));
        let mut actions = Vec::new();
        for events in [Vec::new(), Vec::new()] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| view_tabs(ui, View::History, 0, None, &mut actions),
            );
            out.textures_delta.clear();
        }
        let requests = ctx
            .read_response(view_tab_id(2))
            .expect("requests tab")
            .rect;
        for pressed in [true, false] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events: vec![
                        egui::Event::PointerMoved(requests.center()),
                        egui::Event::PointerButton {
                            pos: requests.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ui| view_tabs(ui, View::History, 0, None, &mut actions),
            );
            out.textures_delta.clear();
        }
        assert!(actions.is_empty(), "{actions:?}");
    }
}
