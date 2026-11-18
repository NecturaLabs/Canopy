//! The settings screen (Ctrl+,): sections on the left, live-applied controls on the right.
//! Every setting changes behaviour; nothing here is decorative.

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use egui::epaint::CubicBezierShape;
use egui::{
    Align, Align2, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, Rect, Response,
    RichText, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::config::Setting;
use canopy_core::forge::{Forge, ForgeKind};
use canopy_core::identity::{Field, Identity, Scope, Source};
use canopy_hosting::CliError;

use crate::action::Action;
use crate::hosting;
use crate::keymap::{Command, Section as KeySection};
use crate::repo_config::{ConfigValue, GitFacts, SigningProblem};
use crate::settings::{
    CleanupMode, CloneProtocol, ConflictMode, PullChoice, PullMode, PushMode, Settings,
    WhitespaceMode,
};
use crate::system_apps::Kind;
use crate::ui::custom_theme::{self, CustomTheme};
use crate::ui::kit::a11y;
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::Badge;
use crate::ui::kit::chip::Keycap;
use crate::ui::kit::choice::{Segmented, Swatches};
use crate::ui::kit::field::{NumberField as KitNumber, SearchField, TextField};
use crate::ui::kit::overlay;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::paint;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::range::Slider;
use crate::ui::kit::row::{ListRow, Roving, RowKind};
use crate::ui::kit::select::{self, Select};
use crate::ui::kit::surface::{Callout, Card, EmptyAction, EmptyState, column_label};
use crate::ui::kit::toggle::Switch;
use crate::ui::kit::{Button, HelpMark, IconButton, Size, Tone};
use crate::ui::motion::{self, MotionPref};
use crate::ui::theme::{
    self, Curve, Density, Elevation, Fill, GlassBlocked, NodeShape, Palette, Style, Surfaces,
    ThemeId, palette,
};
use crate::ui::widgets::forge_icon;
use crate::updater::{self, UpdateState, Updater};
use crate::views::dialogs::Dialog;
use crate::worker::Op;

/// The sections, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Section {
    #[default]
    Appearance,
    Notifications,
    Git,
    Integrations,
    Accounts,
    Updates,
    Privacy,
    Keyboard,
    /// Export, import and reset.
    Data,
    /// The active repository's own settings (shown while a repository is open).
    Repository,
}

impl Section {
    const ALL: [Self; 9] = [
        Self::Appearance,
        Self::Notifications,
        Self::Git,
        Self::Integrations,
        Self::Accounts,
        Self::Updates,
        Self::Privacy,
        Self::Keyboard,
        Self::Data,
    ];

    fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Appearance => (icon::PALETTE, "Appearance"),
            Self::Notifications => (icon::BELL, "Notifications"),
            Self::Git => (icon::GIT_BRANCH, "Git"),
            Self::Integrations => (icon::PLUGS_CONNECTED, "Integrations"),
            Self::Accounts => (icon::KEY, "Accounts"),
            Self::Updates => (icon::ARROW_CIRCLE_UP, "Updates"),
            Self::Privacy => (icon::SHIELD_CHECK, "Privacy"),
            Self::Keyboard => (icon::KEYBOARD, "Keyboard"),
            Self::Data => (icon::ARCHIVE, "Backup and reset"),
            Self::Repository => (icon::FOLDER_SIMPLE, "This repository"),
        }
    }

    /// The sections to offer: the app's, then the repository's when one is open.
    fn shown(has_repo: bool) -> Vec<Self> {
        let mut v = Self::ALL.to_vec();
        if has_repo {
            v.push(Self::Repository);
        }
        v
    }
}

/// State of the open settings screen.
#[derive(Default)]
pub struct SettingsScreen {
    section: Section,
    zoom_preview: Option<f32>,
    picker: crate::ui::folder_picker::FolderPicker,
    /// What the CLIs said about each host on the Accounts page, by `host[:port]` in lower case.
    checks: HashMap<String, CliCheck>,
    /// Hosts whose completed check was explicitly requested with Check again. A renewed token can
    /// have the same account and scopes, so an open tab is restarted even when `CliState` matches.
    rechecks: HashSet<String>,
    /// The Accounts page's hosts, with what they were built from (`AccountHosts::source`).
    account_hosts: Option<(u64, Arc<AccountHosts>)>,
    identity_for: Option<PathBuf>,
    loaded_identity: Option<Identity>,
    identity_name: String,
    identity_email: String,
    identity_scope: Scope,
    search: Search,
    facts: FactsCache,
    git_config_pending: bool,
    /// The repository page's identity fields: the repository's own values only.
    local_identity_for: Option<PathBuf>,
    local_loaded: Option<Identity>,
    local_name: String,
    local_email: String,
    /// Whether this screen asked for the system's programs to be looked up again.
    apps_asked: bool,
    /// Launchers whose command field is open although their command is an installed app's.
    custom_launchers: HashSet<Kind>,
    /// Why the active repository's authors show initials, for Privacy.
    picture_note: Option<PictureNote>,
    /// Remote URLs being edited on the repository page, by remote name, for this repository.
    remote_drafts_for: Option<PathBuf>,
    remote_drafts: HashMap<String, String>,
    /// Scroll the page to this group heading on the next frame.
    reveal: Option<&'static str>,
    /// The URL last saved for a remote, and when, until the configuration shows it.
    remote_saved: Option<(String, String, Instant)>,
    /// An export or import waiting on its file chooser.
    file_task: Option<crate::settings_file::FileTask>,
    /// What the last export, import or reset did: `(failed, message)`.
    file_status: Option<(bool, String)>,
    /// Stay on This repository until a repository is open (the screenshot overlay opens
    /// Settings before its repository's tab); cleared once one is.
    awaiting_repository: bool,
    /// Where clones go when no folder is chosen (~/src when it exists, else home), looked up
    /// once while the screen is open rather than every frame.
    default_clone_parent: Option<String>,
    /// The keyboard row waiting for its new key (`keymap::binding_id`).
    rebinding: Option<String>,
    /// Why the last key was refused, with the row it was for.
    key_note: Option<(String, String)>,
}

/// Why the active repository's authors show initials instead of pictures
/// (`crate::avatars::explain`, contracts/avatars.md), shown under "Download profile pictures".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureNote {
    pub title: String,
    pub detail: String,
    /// A sign-in under Accounts would bring the pictures (`Fix::Accounts`).
    pub open_accounts: bool,
}

impl From<crate::avatars::Explanation> for PictureNote {
    fn from(why: crate::avatars::Explanation) -> Self {
        Self {
            title: why.title.to_owned(),
            detail: why.detail,
            open_accounts: why.fix == Some(crate::avatars::Fix::Accounts),
        }
    }
}

/// What the user's version-control configuration says, read off the UI thread for the notes
/// beside Canopy's choices: the global configuration and the active repository's.
#[derive(Debug, Default)]
struct FactsCache {
    global: FactsSlot,
    repo: FactsSlot,
}

#[derive(Debug, Default)]
struct FactsSlot {
    /// Whether a read started, and for which repository (`None`: the global configuration).
    asked: bool,
    path: Option<PathBuf>,
    rx: Option<Receiver<GitFacts>>,
    /// Shared, so a page can hold them while it changes the screen without copying them.
    facts: Option<Arc<GitFacts>>,
    /// When the last read ended.
    read_at: Option<Instant>,
}

impl FactsSlot {
    /// Reads again on the next [`FactsSlot::get`].
    fn reread(&mut self) {
        self.asked = false;
    }

    /// Whether no read is running and the last one ended at least `gap` ago.
    fn idle_for(&self, gap: Duration) -> bool {
        self.rx.is_none() && self.read_at.is_none_or(|at| at.elapsed() >= gap)
    }

    /// The facts for `path`, starting a read the first time and when `path` changes.
    fn get(&mut self, ctx: &egui::Context, path: Option<&Path>) -> Option<&Arc<GitFacts>> {
        let path = path.map(Path::to_path_buf);
        if !self.asked || self.path != path {
            if self.path != path {
                // Another repository's facts must not show while this one's are read.
                self.facts = None;
            }
            self.rx = Some(crate::repo_config::read_async(ctx, path.clone()));
            self.asked = true;
            self.path = path;
        }
        if let Some(rx) = &self.rx {
            let result = match rx.try_recv() {
                Ok(facts) => Some(facts),
                Err(crossbeam_channel::TryRecvError::Disconnected) => Some(GitFacts {
                    error: Some(
                        "The Git configuration reader stopped. Reopen Settings to retry.".into(),
                    ),
                    ..GitFacts::default()
                }),
                Err(crossbeam_channel::TryRecvError::Empty) => None,
            };
            if let Some(facts) = result {
                self.facts = Some(Arc::new(facts));
                self.rx = None;
                self.read_at = Some(Instant::now());
            }
        }
        self.facts.as_ref()
    }
}

impl FactsCache {
    fn global(&mut self, ctx: &egui::Context) -> Option<&Arc<GitFacts>> {
        self.global.get(ctx, None)
    }

    fn repo(&mut self, ctx: &egui::Context, path: &Path) -> Option<&Arc<GitFacts>> {
        self.repo.get(ctx, Some(path))
    }
}

/// The search over every section: what is typed, and what the last frame found.
#[derive(Debug)]
struct Search {
    query: String,
    /// Put the caret in the search field on the next frame.
    focus: bool,
    /// Scroll the results to this section's header on the next frame.
    jump: Option<Section>,
    /// Move the focus from the search field to the first result's control on the next frame.
    to_results: bool,
    /// Matches per section in the last frame drawn, in section order.
    counts: Vec<(Section, usize)>,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            query: String::new(),
            // A newly opened modal must take keyboard focus away from the underlying screen.
            focus: true,
            jump: None,
            to_results: false,
            counts: Vec::new(),
        }
    }
}

impl Search {
    /// The query's words, lower case; empty when nothing is searched for.
    fn words(&self) -> Vec<String> {
        search_words(&self.query)
    }

    fn total(&self) -> usize {
        self.counts.iter().map(|(_, n)| n).sum()
    }

    fn count(&self, section: Section) -> usize {
        self.counts
            .iter()
            .find(|(s, _)| *s == section)
            .map_or(0, |(_, n)| *n)
    }
}

impl std::fmt::Debug for SettingsScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsScreen")
            .field("section", &self.section)
            .field("checks", &self.checks)
            .field("identity_scope", &self.identity_scope)
            .field("search", &self.search)
            .finish_non_exhaustive()
    }
}

impl SettingsScreen {
    pub(crate) fn git_config_finished(&mut self) {
        self.git_config_pending = false;
        self.facts = FactsCache::default();
    }

    /// What Privacy says about the active repository's profile pictures, set each frame.
    pub fn set_picture_note(&mut self, note: Option<PictureNote>) {
        self.picture_note = note;
    }

    /// The screen opened at `section`.
    pub fn at(section: Section) -> Self {
        Self {
            section,
            ..Self::default()
        }
    }

    /// The screen opened at `section`, scrolled to its group headed `group`.
    pub fn at_group(section: Section, group: &'static str) -> Self {
        Self {
            section,
            reveal: Some(group),
            ..Self::default()
        }
    }

    /// The screen opened at This repository, staying there until the repository's tab opens
    /// (the settings-repository screenshot).
    pub fn awaiting_repository() -> Self {
        Self {
            section: Section::Repository,
            awaiting_repository: true,
            ..Self::default()
        }
    }

    /// This repository, scrolled to a group once its tab opens.
    pub fn awaiting_repository_at(group: &'static str) -> Self {
        Self {
            reveal: Some(group),
            ..Self::awaiting_repository()
        }
    }

    /// The screen showing what `query` finds (screenshots and tests).
    pub fn searching(query: &str) -> Self {
        let mut screen = Self::default();
        query.clone_into(&mut screen.search.query);
        screen
    }

    /// Puts the caret in the search field (the find shortcut while Settings is open).
    pub fn focus_search(&mut self) {
        self.search.focus = true;
    }

    /// Where clones go: the chosen folder, else the default, whose folder check runs once per
    /// opening of the screen.
    fn clone_parent(&mut self, s: &Settings) -> String {
        if s.clone_dir.trim().is_empty() {
            self.default_clone_parent
                .get_or_insert_with(|| Settings::default().clone_parent())
                .clone()
        } else {
            s.clone_parent()
        }
    }

    /// Whether the search field takes the caret on the next frame.
    #[cfg(test)]
    pub fn search_focus_pending(&self) -> bool {
        self.search.focus
    }

    /// Escape: clears the search and returns true, or returns false when nothing is searched for
    /// (Settings then closes).
    pub fn dismiss_search(&mut self) -> bool {
        let searching = !self.search.words().is_empty();
        self.search.query.clear();
        self.search.jump = None;
        searching
    }
}

/// What the app must do after this frame.
#[derive(Debug, Default)]
pub struct SettingsResult {
    pub close: bool,
    /// Hosts (`host[:port]`, lower case) whose CLI sign-in is not what an open tab on them
    /// started with.
    pub sign_in_changed: Vec<String>,
}

pub struct IdentityView<'a> {
    pub repo: Option<&'a Path>,
    pub identity: Option<&'a Identity>,
    pub error: Option<&'a str>,
}

pub struct SettingsData<'a> {
    /// The open repositories' tabs: Accounts lists the hosts of their GitHub/GitLab remotes.
    pub tabs: &'a [crate::tab::RepoTab],
    pub update: &'a UpdateState,
    pub identity: IdentityView<'a>,
}

/// The section list's width (concept `.snav`), shown once the screen is at least
/// [`NAV_MIN_WIDTH`] wide; narrower screens pick the section from a select in the header.
const NAV_WIDTH: f32 = 220.0;
const NAV_MIN_WIDTH: f32 = 720.0;
/// The width of the section select in a narrow screen's header.
const SECTION_SELECT_WIDTH: f32 = 200.0;

/// The shell's size: the room a modal card may fill (clear of the tab strip and the status bar),
/// centred on the window as dialogs are, and no larger than the settings' maximum.
fn shell_size(ctx: &egui::Context) -> (f32, f32) {
    let room = overlay::modal_room(ctx);
    let centre = ctx.content_rect().center().y;
    let tall = 2.0 * (centre - room.top()).min(room.bottom() - centre);
    (
        room.width().clamp(0.0, theme::SETTINGS_MAX_WIDTH),
        tall.clamp(0.0, theme::SETTINGS_MAX_HEIGHT),
    )
}

/// Draws the screen. `forges` are the GitHub/GitLab remotes of open repositories, so their hosts
/// show up under Accounts.
pub fn show(
    ctx: &egui::Context,
    screen: &mut SettingsScreen,
    settings: &mut Settings,
    data: &SettingsData<'_>,
    actions: &mut Vec<Action>,
) -> SettingsResult {
    let p = palette(ctx);
    let style = theme::style();
    let s = p.surfaces(style);
    let m = theme::metrics();
    let mut result = SettingsResult::default();
    screen.picker.begin_frame();
    poll_file_task(screen, settings, actions);
    if !screen.apps_asked {
        // Picks up an editor or terminal installed since the last look.
        screen.apps_asked = true;
        crate::system_apps::refresh(Some(ctx), &settings.launcher_commands());
    }
    let (width, height) = shell_size(ctx);
    let sidebar = width >= NAV_MIN_WIDTH;
    let repo = data.identity.repo;
    if repo.is_some() {
        screen.awaiting_repository = false;
    }
    if screen.section == Section::Repository && repo.is_none() && !screen.awaiting_repository {
        // The repository's tab was closed while its page was open.
        screen.section = Section::Appearance;
    }
    let id = egui::Id::new("settings");
    // The dialog's motion: the scrim fades in while the card grows from a little smaller.
    let entrance = overlay::Entrance::of(ctx, id, motion::SLOW, motion::SLOW, false, None);
    let modal = egui::Modal::new(id)
        .frame(egui::Frame::NONE)
        .backdrop_color(p.scrim.gamma_multiply(entrance.fade))
        .show(ctx, |ui| {
            let layer = ui.layer_id();
            let start = overlay::next_shape(ui.ctx(), layer);
            ui.multiply_opacity(entrance.fade);
            let (shell, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let r = m.radius_lg;
            // The dialog's surface, opaque: the settings screen is never glass.
            ui.painter().extend(paint::surface_shapes(
                ui.painter(),
                shell,
                CornerRadius::same(r),
                p,
                style,
                Elevation::Floating,
                Fill::flat(s.pane),
                None,
            ));
            let content = if sidebar {
                let nav = Rect::from_min_size(shell.min, vec2(NAV_WIDTH, shell.height()));
                let look = repo.map(|path| RepoLook::of(settings, path));
                navigation(ui, nav, r, screen, look.as_ref());
                shell.with_min_x(nav.right())
            } else {
                shell
            };
            let header =
                Rect::from_min_size(content.min, vec2(content.width(), m.pane_header_height));
            let corner = CornerRadius {
                nw: if sidebar { 0 } else { r },
                ne: r,
                sw: 0,
                se: 0,
            };
            result.close |= header_bar(ui, header, corner, screen, sidebar, repo.is_some());
            let body = content.with_min_y(header.bottom());
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt("settings-body")
                    .ui_stack_info(
                        egui::UiStackInfo::default()
                            .with_tag_value(overlay::POPUP_BOUNDS_TAG, shell),
                    )
                    .max_rect(body)
                    .layout(Layout::top_down(Align::Min)),
            );
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            let words = screen.search.words();
            let scroll_salt = words.is_empty().then_some(screen.section);
            let page = overlay::scroll_body(
                &mut child,
                ("settings-content", scroll_salt),
                body.height(),
                s.pane,
                |ui| {
                    egui::Frame::NONE
                        .inner_margin(Margin {
                            left: crate::ui::points_i8(theme::XL),
                            right: crate::ui::points_i8(theme::LG),
                            top: crate::ui::points_i8(theme::LG),
                            bottom: crate::ui::points_i8(theme::XL),
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width());
                            ui.spacing_mut().item_spacing.x = theme::SM;
                            let mut page = Page {
                                screen,
                                settings,
                                data,
                                actions,
                                sign_in_changed: Vec::new(),
                            };
                            page.trust_notice(ui);
                            if words.is_empty() {
                                Filter::set(
                                    ui.ctx(),
                                    Filter {
                                        reveal: if page.screen.awaiting_repository {
                                            None
                                        } else {
                                            page.screen.reveal.take()
                                        },
                                        ..Filter::default()
                                    },
                                );
                                page.section(ui, page.screen.section);
                            } else {
                                page.results(ui, &words, body.height());
                            }
                            result.sign_in_changed.append(&mut page.sign_in_changed);
                            reveal_focus(ui);
                        });
                },
            );
            page_fade(&child, &page, s.pane);
            // The ring goes over the chrome painted inside the card.
            paint::rim(
                ui.painter(),
                shell,
                CornerRadius::same(r),
                p.floating_ring(s),
            );
            let center = shell.center();
            overlay::transform_since(
                ui.ctx(),
                layer,
                start,
                entrance.transform(center, overlay::DIALOG_SCALE, Vec2::ZERO),
            );
            let end = overlay::next_shape(ui.ctx(), layer);
            overlay::record_exit(
                ui.ctx(),
                id,
                layer,
                0..start.0,
                start.0..end.0,
                overlay::ExitMotion {
                    seconds: motion::EXIT_DIALOG,
                    scale: overlay::DIALOG_EXIT_SCALE,
                    origin: center,
                    offset: Vec2::ZERO,
                    instant_on_input: false,
                },
            );
        });
    screen.picker.end_frame();
    result.close |= modal.should_close();
    // egui's modal sets its initial focus after drawing the body. Requesting focus
    // inside the search field can therefore be overwritten on the opening frame.
    if !result.close && std::mem::take(&mut screen.search.focus) {
        ctx.memory_mut(|m| m.request_focus(crate::ui::kit::field::edit_id(search_id())));
        ctx.request_repaint();
    }
    result
}

/// How far a Settings page fades into the card at an edge where more of it lies: a full line of
/// row text, so the line the card's edge cuts through is never left legible but half drawn (the
/// kit's 12-point scroll fade suits short lists, not a page of paragraphs).
const PAGE_FADE: f32 = theme::XL + theme::XS;

/// Fades the page into `ground` at its top and bottom where more of it lies beyond, over the
/// kit's own scroll fade.
fn page_fade<R>(ui: &Ui, page: &egui::scroll_area::ScrollAreaOutput<R>, ground: Color32) {
    let rect = page.inner_rect;
    let above = page.state.offset.y;
    let below = page.content_size.y - rect.height() - page.state.offset.y;
    let painter = ui.painter_at(rect);
    for (hidden, edge) in [
        (above, paint::FadeEdge::Top),
        (below, paint::FadeEdge::Bottom),
    ] {
        if hidden > 1.0 {
            painter.add(paint::edge_fade(
                rect,
                edge,
                PAGE_FADE.min(hidden),
                Fill::flat(ground),
                1.0,
            ));
        }
    }
}

/// Scrolls the page to a control of it that just took keyboard focus out of sight (Tab onto a
/// row below the fold), so the focus ring is never hidden. Runs inside the page's scroll area.
fn reveal_focus(ui: &Ui) {
    let key = egui::Id::new("settings-revealed-focus");
    let focused = ui.ctx().memory(egui::Memory::focused);
    let seen = ui
        .ctx()
        .data(|d| d.get_temp::<Option<egui::Id>>(key))
        .flatten();
    if focused == seen {
        return;
    }
    ui.ctx().data_mut(|d| d.insert_temp(key, focused));
    let Some(rect) = focused
        .and_then(|id| ui.ctx().read_response(id))
        .map(|r| r.rect)
    else {
        return;
    };
    let page = ui.min_rect();
    let inside =
        page.x_range().contains(rect.center().x) && page.y_range().contains(rect.center().y);
    if inside && !ui.clip_rect().contains_rect(rect) {
        ui.scroll_to_rect(rect.expand(theme::LG), None);
    }
}

/// Everything a section's page draws from and writes to.
struct Page<'a, 'd> {
    screen: &'a mut SettingsScreen,
    settings: &'a mut Settings,
    data: &'a SettingsData<'d>,
    actions: &'a mut Vec<Action>,
    sign_in_changed: Vec<String>,
}

impl Page<'_, '_> {
    /// What stays at the top of every page while trust safety is off, until it is dismissed.
    fn trust_notice(&mut self, ui: &mut Ui) {
        if self.settings.trust_safety {
            self.settings.trust_off_dismissed = false;
            return;
        }
        if self.settings.trust_off_dismissed {
            return;
        }
        match trust_off_notice(ui) {
            Some(TrustNotice::TurnOn) => self.actions.push(Action::SetTrustSafety(true)),
            Some(TrustNotice::Dismiss) => self.settings.trust_off_dismissed = true,
            None => {}
        }
    }

    /// One section's rows (all of them, or while searching those that match).
    fn section(&mut self, ui: &mut Ui, section: Section) {
        let Self {
            screen,
            settings,
            data,
            actions,
            sign_in_changed,
        } = self;
        match section {
            Section::Appearance => appearance(ui, settings, &mut screen.zoom_preview),
            Section::Notifications => notifications(ui, &mut settings.notifications),
            Section::Git => git(ui, screen, settings, &data.identity, actions),
            Section::Integrations => {
                let parent = screen.clone_parent(settings);
                integrations(
                    ui,
                    settings,
                    &mut screen.picker,
                    &mut screen.custom_launchers,
                    &parent,
                );
            }
            Section::Accounts => {
                sign_in_changed.append(&mut accounts(ui, screen, settings, data.tabs, actions));
            }
            Section::Updates => updates(ui, settings, data.update, actions),
            Section::Privacy => {
                let parent = screen.clone_parent(settings);
                let open_accounts = privacy(
                    ui,
                    settings,
                    actions,
                    &mut screen.picker,
                    screen.picture_note.as_ref(),
                    &parent,
                );
                if open_accounts {
                    screen.section = Section::Accounts;
                }
            }
            Section::Keyboard => keyboard(ui, screen, settings),
            Section::Data => backup(ui, screen, settings, actions),
            Section::Repository => repository(ui, screen, settings, data, actions),
        }
    }

    /// What the search finds: every section's matching rows in place, each section under its
    /// own header, or an empty state. Records the matches per section for the navigation.
    fn results(&mut self, ui: &mut Ui, words: &[String], height: f32) {
        if std::mem::take(&mut self.screen.search.to_results) {
            // egui hands focus on to the next control while the focused one registers: a
            // focus stop at the top of the results, there for this frame only, passes it to
            // the first control of the results.
            let start = ui.id().with("settings-results-start");
            ui.memory_mut(|m| {
                m.request_focus(start);
                m.move_focus(egui::FocusDirection::Next);
            });
            let at = Rect::from_min_size(ui.cursor().min, Vec2::ZERO);
            ui.interact(at, start, Sense::focusable_noninteractive());
        }
        let sections = Section::shown(self.data.identity.repo.is_some());
        let mut counts = Vec::with_capacity(sections.len());
        let jump = self.screen.search.jump.take();
        for section in sections {
            Filter::set(
                ui.ctx(),
                Filter {
                    words: words.to_vec(),
                    section: Some(section),
                    header: true,
                    jump: jump == Some(section),
                    first: counts.iter().all(|&(_, n)| n == 0),
                    ..Filter::default()
                },
            );
            self.section(ui, section);
            counts.push((section, Filter::get(ui.ctx()).count));
        }
        Filter::set(ui.ctx(), Filter::default());
        if counts != self.screen.search.counts {
            self.screen.search.counts = counts;
            ui.ctx().request_repaint();
        }
        if self.screen.search.total() == 0 {
            let query = self.screen.search.query.trim().to_owned();
            // The body less its margins, so the empty state fills it without scrolling.
            let room = (height - theme::LG - 2.0 * theme::XL).max(theme::XL * 4.0);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), room),
                Layout::top_down(Align::Center),
                |ui| {
                    let clicked = EmptyState::new(format!("No settings match “{query}”"))
                        .icon(icon::MAGNIFYING_GLASS)
                        .body(
                            "Try fewer words, or another name for it: “theme”, “rebase”, “editor”.",
                        )
                        .action("Clear search")
                        .id_salt("settings-no-match")
                        .show(ui)
                        .value;
                    if clicked == Some(EmptyAction::Primary) {
                        self.screen.search.query.clear();
                        self.screen.search.focus = true;
                    }
                },
            );
        }
    }
}

/// While searching: which rows a section draws. Kept in the context's memory for the frame, so
/// the row helpers ([`row_layout`], [`heading`], [`block`]) can ask without new parameters.
#[derive(Debug, Clone, Default)]
struct Filter {
    /// The query's lower-case words, all of which a row must contain; empty shows every row.
    words: Vec<String>,
    /// The section being drawn.
    section: Option<Section>,
    /// The section's header is still to be drawn, before its first match.
    header: bool,
    /// Scroll the results to this section's header once drawn.
    jump: bool,
    /// No earlier section had a match (its header needs no space above it).
    first: bool,
    /// The group heading over the next rows, still to be drawn, if any.
    group: Option<String>,
    /// The group heading itself matches, so its whole group does.
    group_matches: bool,
    /// Whether the last row was drawn (its seam and anything under it follow only then).
    shown: bool,
    /// Rows drawn in this section.
    count: usize,
    /// The group heading to scroll to once drawn.
    reveal: Option<&'static str>,
}

impl Filter {
    fn id() -> egui::Id {
        egui::Id::new("settings-filter")
    }

    fn get(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<Self>(Self::id()).unwrap_or_default())
    }

    fn set(ctx: &egui::Context, filter: Self) {
        ctx.data_mut(|d| d.insert_temp(Self::id(), filter));
    }

    fn searching(ctx: &egui::Context) -> bool {
        !Self::get(ctx).words.is_empty()
    }
}

/// Words that join a query's terms ("pull and merge") and would match nearly every row.
const CONNECTIVES: &[&str] = &[
    "a", "an", "and", "as", "at", "by", "for", "from", "in", "into", "is", "of", "on", "or", "the",
    "to", "with",
];

/// Lower-case words of a query, without its connective words unless it has nothing else.
fn search_words(query: &str) -> Vec<String> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let meaningful: Vec<String> = words
        .iter()
        .filter(|w| !CONNECTIVES.contains(&w.as_str()))
        .cloned()
        .collect();
    if meaningful.is_empty() {
        words
    } else {
        meaningful
    }
}

/// Whether every word occurs in one of `texts` (case-insensitive).
fn matches_all(words: &[String], texts: &[&str]) -> bool {
    let haystack = texts.join("\n").to_lowercase();
    words.iter().all(|w| haystack.contains(w.as_str()))
}

/// Other words people use for a setting, so a search finds it by them too.
fn synonyms(title: &str) -> &'static str {
    match title {
        "Theme" => "colour color dark light mode midnight daylight observatory contrast",
        "Style" => "standard deeper flat depth shadows look",
        "High contrast" => "contrast accessibility low vision glare legibility readable bold",
        "Translucent surfaces" => "glass blur transparency transparent frosted",
        "Density" => "compact dense comfortable spacing row height",
        "Request list" => "pull merge requests compact dense comfortable one line rows items",
        "Graph nodes" => "commit graph dots rings squares diamonds shape nodes history",
        "Graph lines" => "commit graph curves metro circuit lines edges branches history",
        "Motion" => "animation animations reduce reduced motion",
        "Zoom" => "scale size bigger smaller font text",
        "Dates" | "Commit date format" => "time relative calendar iso",
        "Author avatars" | "Download profile pictures" => {
            "avatar picture photo gravatar image profile"
        }
        "Name" | "Email" | "Apply identity" | "Save to" | "Committer" => {
            "identity user author git config user.name user.email"
        }
        "Pull strategy" | "Fast-forward when pulling" => {
            "pull strategy rebase merge fast-forward ff only pull.ff"
        }
        "Follow my git config" => "pull rebase pull.rebase pull.ff configuration follow default",
        "Edit my git config" => "write change global config gitconfig recommended permission",
        "Prune on fetch" => "prune stale remote branches delete",
        "Update submodules" => "submodule recurse pull checkout clone recursive",
        "Discard in a submodule" => "submodule discard untracked clean delete reset",
        "Recurse into submodules" => "submodule.recurse submodule recursive git config",
        "Fetch in the background" => "auto fetch automatic interval minutes",
        "Remove deleted branches" => "prune gone stale merged local branches delete cleanup",
        "Add Signed-off-by" => "signoff sign-off dco trailer",
        "Check commit messages" => {
            "message lint linting summary subject length 72 wrap description"
        }
        "Commits per page" => "history load more limit",
        "Layout" => "diff side by side unified split",
        "Syntax highlighting" => "colour color code language",
        "Context lines" => "diff context unchanged lines",
        "Whitespace" => "spaces tabs ignore diff",
        "Default clone folder" => "clone directory path folder location",
        "Clone protocol" => "ssh https url remote",
        "Self-hosted GitLab" => "gitlab host enterprise server",
        "Pull and merge requests" => "pr mr github gitlab api hosting offline",
        "Terminal" => "default console shell command line $terminal kitty alacritty",
        "Editor" => "default ide vscode code zed sublime vim open file $editor $visual",
        "File manager" => "default folder explorer finder nautilus files dolphin show reveal",
        "Trusted folders" => "trust trusted repository worktree partial clone security folder",
        "Trust safety" => "trust untrusted read-only hooks programs security safety off",
        "Send crash reports" => "crash sentry telemetry error report",
        "Log detail" => {
            "log logging debug verbose trace normal detailed everything canopy_log rust_log environment"
        }
        "Logs and crash files" => {
            "log logs crash crashes diagnostics folder copy clipboard bug report"
        }
        "Privacy Policy" | "Terms of Service" => {
            "legal privacy terms document consent accepted agreement"
        }
        "Export settings" | "Import settings" => {
            "backup file json move computer copy transfer save load"
        }
        "Reset all settings" => "defaults factory start over restore clear",
        "Check for updates" | "Include beta builds" => "update upgrade version release beta",
        "Sign-in" => "account sign in login github gitlab gh glab cli credential",
        "Version" => "update upgrade version release check now what's new",
        _ => "",
    }
}

/// Whether a row (or block) titled `title` with `text` is drawn: always when not searching;
/// while searching, when the query matches it, its group heading or its section. A match first
/// draws the section header and group heading it sits under.
fn reveal(ui: &mut Ui, title: &str, text: &[&str]) -> bool {
    let mut f = Filter::get(ui.ctx());
    if f.words.is_empty() {
        return true;
    }
    let section = f.section.map_or("", |s| s.label().1);
    let mut texts = vec![title, synonyms(title), section];
    texts.extend_from_slice(text);
    let shown = f.group_matches || matches_all(&f.words, &texts);
    f.shown = shown;
    if shown {
        f.count += 1;
        f = open_group(ui, f);
    }
    Filter::set(ui.ctx(), f);
    shown
}

/// Draws the section header and group heading a match sits under, the first time.
fn open_group(ui: &mut Ui, mut f: Filter) -> Filter {
    if f.header {
        f.header = false;
        if let Some(section) = f.section {
            result_header(ui, section, f.first, f.jump);
        }
    }
    if let Some(group) = f.group.take() {
        heading_now(ui, &group);
    }
    f
}

/// A search result's section header: the section's icon and name.
fn result_header(ui: &mut Ui, section: Section, first: bool, jump: bool) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    if !first {
        ui.add_space(theme::XL);
    }
    let (glyph, label) = section.label();
    let response = ui
        .horizontal(|ui| {
            ui.label(RichText::new(glyph).size(m.icon_md).color(p.text_faint));
            ui.label(
                RichText::new(label)
                    .font(theme::heading(m.subtitle_size))
                    .color(p.text),
            );
        })
        .response;
    if jump {
        ui.scroll_to_rect(response.rect.expand(theme::LG), Some(Align::TOP));
    }
    ui.add_space(theme::MD);
}

/// `text` in `font` and `color`, with each occurrence of `words` in `hit` over `mark`.
fn marked(
    text: &str,
    words: &[String],
    font: &FontId,
    color: Color32,
    hit: Color32,
    mark: Color32,
) -> egui::text::LayoutJob {
    let lower = text.to_lowercase();
    // Which bytes of `text` a word covers. Lower-casing can change byte lengths outside ASCII;
    // then nothing is marked.
    let mut covered = vec![false; text.len()];
    if lower.len() == text.len() {
        for word in words.iter().filter(|w| !w.is_empty()) {
            for (at, _) in lower.match_indices(word.as_str()) {
                for b in covered.iter_mut().skip(at).take(word.len()) {
                    *b = true;
                }
            }
        }
    }
    let format = |marked: bool| egui::TextFormat {
        font_id: font.clone(),
        color: if marked { hit } else { color },
        background: if marked { mark } else { Color32::TRANSPARENT },
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let mut run = String::new();
    let mut run_marked = false;
    for (i, ch) in text.char_indices() {
        let marked = covered.get(i).copied().unwrap_or(false);
        if marked != run_marked && !run.is_empty() {
            job.append(&run, 0.0, format(run_marked));
            run.clear();
        }
        run_marked = marked;
        run.push(ch);
    }
    job.append(&run, 0.0, format(run_marked));
    job
}

/// The section list on the chrome: the screen's title, then one row per section. While
/// searching, each row counts its section's matches and a click scrolls the results to it.
fn navigation(
    ui: &mut Ui,
    rect: Rect,
    radius: u8,
    screen: &mut SettingsScreen,
    repo: Option<&RepoLook>,
) {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius {
            nw: radius,
            sw: radius,
            ne: 0,
            se: 0,
        },
        s.panel,
    );
    paint::seam_v(painter, rect.right() - 0.5, rect.y_range(), s.border);
    let inner = rect.shrink(m.sidebar_pad);
    let mut ui = ui.new_child(
        UiBuilder::new()
            .id_salt("settings-navigation")
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    // The title shares the header's centre line.
    let (title, _) = ui.allocate_exact_size(
        vec2(inner.width(), m.pane_header_height - 2.0 * m.sidebar_pad),
        Sense::hover(),
    );
    let glyph = ui.painter().layout_no_wrap(
        icon::GEAR_SIX.to_owned(),
        FontId::proportional(m.icon_md),
        p.text_faint,
    );
    let x = title.left() + theme::SM;
    let cy = title.center().y;
    ui.painter().galley(
        pos2(x, cy - glyph.size().y / 2.0),
        Arc::clone(&glyph),
        p.text_faint,
    );
    ui.painter().text(
        pos2(x + glyph.size().x + theme::SM, cy),
        Align2::LEFT_CENTER,
        "Settings",
        theme::heading(m.body_size),
        p.text,
    );
    egui::ScrollArea::vertical()
        .id_salt("settings-sections")
        .show(&mut ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let searching = !screen.search.words().is_empty();
            // One Tab stop for the list, the arrows between its sections.
            let sections = Section::shown(repo.is_some());
            let list = Roving::new(
                ui.ctx(),
                "settings-sections",
                sections.len(),
                sections.iter().position(|s| *s == screen.section),
            );
            for (index, section) in sections.into_iter().enumerate() {
                let (mut glyph, mut label) = section.label();
                let name = if let (Section::Repository, Some(look)) = (section, repo) {
                    // Its own group, named and drawn as the repository's tab.
                    ui.add_space(theme::LG);
                    column_label(ui, "This repository");
                    ui.add_space(theme::XS);
                    label = &look.name;
                    glyph = look.icon;
                    format!("This repository: {label}")
                } else {
                    label.to_owned()
                };
                let found = screen.search.count(section);
                let mut row = ListRow::new(("settings-section", section))
                    .kind(RowKind::Sidebar)
                    .icon(glyph)
                    .label(label)
                    .accessible_name(name)
                    .roving(list, index)
                    .selected(!searching && screen.section == section);
                if searching {
                    row = row.enabled(found > 0);
                    if found > 0 {
                        row = row.trailing_badge(Badge::new(found.to_string()));
                    }
                }
                if row.show(ui).clicked() {
                    screen.section = section;
                    if searching {
                        screen.search.jump = Some(section);
                    }
                }
            }
        });
}

/// The search field's widest and narrowest.
const SEARCH_WIDTH: f32 = 280.0;
const SEARCH_MIN_WIDTH: f32 = 140.0;

/// The id of the settings search field.
fn search_id() -> egui::Id {
    egui::Id::new("settings-search")
}

/// The strip over the section: its name (or, in a narrow screen, the section select; while
/// searching, the number of results), the search field and the close button. Returns whether
/// close was clicked.
fn header_bar(
    ui: &mut Ui,
    rect: Rect,
    corner: CornerRadius,
    screen: &mut SettingsScreen,
    sidebar: bool,
    has_repo: bool,
) -> bool {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    if theme::style() == Style::Standard {
        ui.painter().rect_filled(rect, corner, s.panel);
    }
    paint::seam_h(ui.painter(), rect.x_range(), rect.bottom() - 0.5, s.border);
    let mut ui = ui.new_child(
        UiBuilder::new()
            .id_salt("settings-header")
            .max_rect(rect.shrink2(vec2(theme::MD, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let searching = !screen.search.words().is_empty();
    if sidebar {
        let title = if searching {
            match screen.search.total() {
                0 => "No results".to_owned(),
                1 => "1 result".to_owned(),
                n => format!("{n} results"),
            }
        } else {
            screen.section.label().1.to_owned()
        };
        ui.label(
            RichText::new(title)
                .font(theme::heading(m.body_size))
                .color(p.text),
        );
    } else {
        let options: Vec<(Section, &str)> = Section::shown(has_repo)
            .into_iter()
            .map(|s| (s, s.label().1))
            .collect();
        let before = screen.section;
        Select::new(&mut screen.section, &options)
            .width(SECTION_SELECT_WIDTH.min(ui.available_width() - SEARCH_MIN_WIDTH))
            .name("Settings section")
            .size(Size::Sm)
            .id_salt("settings-section")
            .show(&mut ui);
        if searching && screen.section != before {
            screen.search.jump = Some(screen.section);
        }
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        // Drawn before the search field, so Tab goes from the search straight to the results.
        let close = IconButton::new(icon::X, "Close settings (Esc)")
            .size(Size::Sm)
            .id_salt("settings-close")
            .show(ui)
            .clicked();
        search_field(ui, screen);
        close
    })
    .inner
}

/// The search field: the find shortcut puts the caret in it, Escape clears it (then closes
/// Settings, through the Dismiss command), and Down or Enter moves on to the first result.
fn search_field(ui: &mut Ui, screen: &mut SettingsScreen) {
    let id = search_id();
    let width = (ui.available_width() - theme::SM).clamp(SEARCH_MIN_WIDTH, SEARCH_WIDTH);
    let before = screen.search.query.clone();
    let response = ui
        .allocate_ui_with_layout(
            vec2(width, Size::Sm.height()),
            Layout::top_down(Align::Min),
            |ui| {
                SearchField::new(&mut screen.search.query)
                    .placeholder("Search settings")
                    .shortcut(Command::Filter)
                    .size(Size::Sm)
                    .width(width)
                    .name("Search settings")
                    .id(id)
                    .show(ui)
            },
        )
        .inner;
    if screen.search.query != before {
        screen.search.jump = None;
    }
    let (down, enter) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Enter),
        )
    });
    let leave = (response.has_focus() && down) || (response.lost_focus() && enter);
    if leave && !screen.search.words().is_empty() && screen.search.total() > 0 {
        screen.search.to_results = true;
        ui.ctx().request_repaint();
    }
}

/// What the identity rows say under their titles.
const NAME_HELP: &str = "The author name recorded in your commits.";
const EMAIL_HELP: &str = "Recorded in your commits; GitHub and GitLab link it to your account.";

/// A preference and its controls. Compact columns stack the controls below the help.
fn row(ui: &mut Ui, title: &str, help: &str, control: impl FnOnce(&mut Ui)) {
    row_explained(ui, title, help, "", control);
}

/// [`row`] with a longer explanation behind a help mark after its title.
fn row_explained(
    ui: &mut Ui,
    title: &str,
    help: &str,
    detail: &str,
    control: impl FnOnce(&mut Ui),
) {
    row_more(ui, title, help, detail, control);
    row_end(ui);
}

/// What a Git setting row says under its title; the recommended option is marked in its list.
fn git_setting_help(setting: Setting) -> &'static str {
    match setting {
        Setting::PullRebase => {
            "Replays your commits on top of the incoming ones instead of merging."
        }
        Setting::PullFastForward => {
            "With fast-forward only, Canopy asks Rebase or Merge when branches diverged."
        }
        Setting::PullAutostash => {
            "Stashes local changes before a pull and restores them after. Git before 2.51 ignores it."
        }
        Setting::FetchPrune => "Removes remote branches that were deleted on the remote.",
        Setting::FetchPruneTags => {
            "Deletes local tags the remote doesn't have, even unpushed ones."
        }
        Setting::RebaseAutostash => {
            "Stashes local changes before a rebase and restores them after."
        }
        Setting::MergeAutostash => {
            "Stashes local changes before a merge, never a squash, and restores them after."
        }
        Setting::PushFollowTags => "Pushes the annotated tags on the commits being pushed.",
        Setting::SubmoduleRecurse => {
            "Checkout, reset and restore also reset submodules and can lose edits; a push includes their commits."
        }
        Setting::PushDefault => "Simple pushes to the upstream branch only when the names match.",
        Setting::ConflictStyle => "zdiff3 and diff3 also show the original text between the sides.",
        Setting::InitialBranch => "The branch a new repository starts on.",
        Setting::CommitCleanup => {
            "Default removes # lines from a message you edit. Spaces and blank lines keeps them."
        }
        Setting::Autocrlf => "Converts CRLF to LF on commit; CRLF on checkout converts back.",
        Setting::CommitSigning => "Signs every commit with your configured key.",
        Setting::SigningFormat => "SSH signs with an SSH key; OpenPGP and X.509 need GnuPG.",
    }
}

/// Why a value the user's configuration holds is not one of the row's choices, after the note's
/// "Set in …": empty for a value the list offers (see [`Setting::shown_only`]).
fn shown_only_reason(setting: Setting, value: &str) -> &'static str {
    let value = value.trim();
    match setting {
        _ if !setting
            .shown_only()
            .iter()
            .any(|v| v.eq_ignore_ascii_case(value)) =>
        {
            ""
        }
        Setting::PushDefault if value.eq_ignore_ascii_case("matching") => {
            " Canopy does not offer matching: a push sends every branch that has a branch of the same name on the remote."
        }
        Setting::PushDefault => {
            " Canopy does not offer nothing: a push that names no branch is refused."
        }
        _ => "",
    }
}

/// What an unset git setting shows: the value that applies, which is its recommendation except for
/// line endings and signing, which Canopy leaves to git's built-in value.
fn unset_value(setting: Setting, recommended: &str) -> String {
    let builtin = match setting {
        Setting::Autocrlf | Setting::CommitSigning => "false",
        Setting::SigningFormat => "openpgp",
        _ => recommended,
    };
    let label = git_value_label(setting, builtin);
    if builtin == recommended {
        format!("{label} (Recommended)")
    } else {
        label
    }
}

/// [`row`] with a note under its help about what git's own configuration says.
fn row_with_note(
    ui: &mut Ui,
    title: &str,
    help: &str,
    note: Option<&Note>,
    control: impl FnOnce(&mut Ui),
) {
    let help_text = |ui: &mut Ui| {
        if !help.is_empty() {
            help_label(ui, help);
        }
    };
    noted_row_layout(ui, title, None, help, &help_text, "", note, control);
    row_end(ui);
}

/// Every setting's default, for the buttons that go back to it.
static DEFAULTS: LazyLock<Settings> = LazyLock::new(Settings::default);

/// What the control of a row took last frame, for the reset button that goes before it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ControlRoom {
    /// From the row's right edge to the control's left edge, spacing included.
    width: f32,
    /// The control's middle, below the row's top.
    middle: f32,
    /// The control did not fit on one line: the reset button then flows with it.
    wrapped: bool,
}

/// The row being drawn: where [`revert`] finds the room its control took last frame (`None` before
/// the first frame has measured it), and where this frame's control starts.
#[derive(Debug, Clone, Copy)]
struct RowRoom {
    last: Option<ControlRoom>,
    top: f32,
    right: f32,
}

fn row_room_id() -> egui::Id {
    egui::Id::new("settings-row-room")
}

/// Runs a row's `control` and measures what it took, for the next frame's [`revert`].
fn run_control(ui: &mut Ui, control: impl FnOnce(&mut Ui)) {
    let key = ui.id().with("control-room");
    let last: Option<ControlRoom> = ui.data(|d| d.get_temp(key));
    let start = ui.cursor();
    let top = ui.max_rect().top();
    ui.data_mut(|d| {
        d.insert_temp(
            row_room_id(),
            RowRoom {
                last,
                top,
                right: start.right(),
            },
        );
    });
    control(ui);
    let end = ui.cursor();
    ui.data_mut(|d| d.remove::<RowRoom>(row_room_id()));
    let room = ControlRoom {
        width: start.right() - end.right(),
        middle: ui.min_rect().center().y - top,
        wrapped: end.min.y > start.min.y + 0.5,
    };
    let changed = last.is_none_or(|l| {
        l.wrapped != room.wrapped
            || (l.width - room.width).abs() > 0.5
            || (l.middle - room.middle).abs() > 0.5
    });
    if changed {
        ui.data_mut(|d| d.insert_temp(key, room));
        ui.ctx().request_repaint();
    }
}

/// A button back to a setting's default, shown while the setting differs from it. It sits before
/// the row's control (to its left) and so is drawn, and met by Tab and by a screen reader, before
/// it: call it first in the row's control. Returns true when it was clicked.
fn revert<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, default: &T, name: &str) -> bool {
    if value == default {
        return false;
    }
    let side = crate::ui::kit::button::icon_button_side(Size::Sm);
    let button = IconButton::new(
        icon::ARROW_COUNTER_CLOCKWISE,
        format!("Reset {name} to recommended"),
    )
    .size(Size::Sm)
    .id_salt(("revert", name));
    // The row draws right to left, so the control's place is known from last frame only: the
    // button is put where it ended, and drawn first. A row's first frame draws without it, and a
    // control that wraps lets the button flow with it.
    let row = ui.ctx().data(|d| d.get_temp::<RowRoom>(row_room_id()));
    let clicked = match row {
        Some(RowRoom { last: None, .. }) => false,
        Some(RowRoom {
            last:
                Some(ControlRoom {
                    width,
                    middle,
                    wrapped: false,
                }),
            top,
            right,
        }) if right - width - side >= ui.max_rect().left() => {
            let at = pos2(right - width - side * 0.5, top + middle);
            let mut slot = ui.new_child(
                UiBuilder::new()
                    .id_salt("revert-slot")
                    .max_rect(Rect::from_center_size(at, Vec2::splat(side)))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            button.show(&mut slot).clicked()
        }
        _ => button.show(ui).clicked(),
    };
    if clicked {
        value.clone_from(default);
    }
    clicked
}

const OVERRIDDEN_TIP: &str = "This repository has its own value.";

/// [`row`] for a setting that is the default for repositories (each can override it in This
/// repository, which tags its overrides).
fn default_row(
    ui: &mut Ui,
    title: &str,
    help: &str,
    note: Option<&Note>,
    control: impl FnOnce(&mut Ui),
) {
    let help_text = |ui: &mut Ui| help_label(ui, help);
    noted_row_layout(ui, title, None, help, &help_text, "", note, control);
    row_end(ui);
}

/// A line under a setting's help about what git's own configuration says: a warning when git
/// will act otherwise than the setting, else a quiet remark.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    text: String,
    warn: bool,
}

/// How a note opens when git's own configuration decides a choice.
const DECIDES: &str = "Your git configuration decides";

/// `key = value (where it is set)`, for a note.
fn config_phrase(v: &ConfigValue) -> String {
    let source = source_label(Some(&v.source()));
    let (setting, owner) = row_of(&v.key);
    let Some(setting) = setting else {
        return format!("{} in {source}", v.value);
    };
    let label = owner.map_or_else(
        || setting.label().to_owned(),
        |owner| format!("{} for {owner}", setting.label()),
    );
    format!(
        "{label} set to {} in {source}",
        value_phrase(setting, &v.value)
    )
}

/// A value inside a sentence. Most names are single words that read as they are in lower case;
/// a cleanup's name is a phrase of its own ("Nothing", "Also lines starting with #"), so it is
/// quoted as Settings shows it.
fn value_phrase(setting: Setting, value: &str) -> String {
    let label = git_value_label(setting, value);
    if setting == Setting::CommitCleanup {
        format!("\"{label}\"")
    } else {
        lowercase_label(&label)
    }
}

/// The row a configuration key belongs to, and the remote or branch whose own value it is: a
/// remote's own fetch value, or a branch's own pull value, is its row's setting for that one.
fn row_of(key: &str) -> (Option<Setting>, Option<&str>) {
    let row = |key: &str| {
        Setting::ALL
            .into_iter()
            .find(|s| s.key().eq_ignore_ascii_case(key))
    };
    match key.split_once('.') {
        Some(("remote", rest)) => rest.rsplit_once('.').map_or((None, None), |(name, var)| {
            (row(&format!("fetch.{var}")), Some(name))
        }),
        Some(("branch", rest)) => rest.rsplit_once('.').map_or((None, None), |(name, var)| {
            (row(&format!("pull.{var}")), Some(name))
        }),
        _ => (row(key), None),
    }
}

/// The settings behind a pull strategy as configuration values.
fn pull_values(config: &canopy_core::PullConfig) -> Vec<ConfigValue> {
    use canopy_core::ConfigScope;
    config
        .sources
        .iter()
        .map(|s| ConfigValue {
            key: s.key.clone(),
            value: s.value.clone(),
            scope: match s.scope {
                ConfigScope::System => "system",
                ConfigScope::Global => "global",
                ConfigScope::Local => "local",
                ConfigScope::Worktree => "worktree",
                ConfigScope::Command => "command",
                ConfigScope::Other => "",
            }
            .to_owned(),
            origin: s.origin.clone(),
            file: s.file.clone(),
        })
        .collect()
}

/// The settings behind a pull strategy, phrased as every other row's: [`config_phrase`].
fn pull_phrase(config: &canopy_core::PullConfig) -> Option<String> {
    let phrases: Vec<String> = pull_values(config).iter().map(config_phrase).collect();
    (!phrases.is_empty()).then(|| and_list(&phrases))
}

/// What git's configuration says about pulls, against Canopy's `choice`. Canopy always passes a
/// strategy (`canopy_core::Repo::pull`): the configuration's when it follows it and one is set,
/// else its own, and then a configuration that differs does not apply.
fn pull_note(facts: &GitFacts, choice: PullChoice) -> Option<Note> {
    let config = facts.pull.as_ref()?;
    let ours = choice.mode.strategy();
    let (configured, described) = config.strategy.zip(pull_phrase(config))?;
    Some(if choice.follow_config {
        Note {
            text: format!(
                "Your git configuration decides: {described}, so pulls use {}.",
                lowercase_label(configured.label())
            ),
            warn: false,
        }
    } else if configured == ours {
        Note {
            text: format!("Matches your git configuration: {described}."),
            warn: false,
        }
    } else {
        Note {
            text: format!(
                "Canopy's pulls use this choice instead of your git configuration: {described}."
            ),
            warn: true,
        }
    })
}

/// A strategy's label inside a sentence: "Rebase, keeping merges" -> "rebase, keeping merges".
fn lowercase_label(label: &str) -> String {
    let mut chars = label.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

/// A phrase as a sentence's start: "rebase when pulling …" -> "Rebase when pulling …".
fn upper_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The values of `fetch.<key>` and `remote.<name>.<key>` that decide a fetch: a remote's own
/// value comes before `fetch.<key>`, which still decides for a remote without one (or with no
/// remote yet).
fn fetch_values<'a>(
    facts: &GitFacts,
    fetch: Option<&'a ConfigValue>,
    remote: &'a [ConfigValue],
    key: &str,
) -> Vec<&'a ConfigValue> {
    let fetch_applies = facts.remotes.is_empty()
        || facts.remotes.iter().any(|r| {
            let own = format!("remote.{}.{key}", r.name);
            !remote.iter().any(|v| v.key == own)
        });
    fetch
        .filter(|_| fetch_applies)
        .into_iter()
        .chain(remote)
        .collect()
}

/// What git's configuration says about pruning remote-tracking branches: `fetch.prune` and a
/// remote's own `remote.<name>.prune`.
fn prune_note(facts: &GitFacts, prune: bool, follow: bool) -> Option<Note> {
    let set = fetch_values(
        facts,
        facts.fetch_prune.as_ref(),
        &facts.remote_prune,
        "prune",
    );
    config_note(
        &set,
        follow,
        |v| v.as_bool().unwrap_or(false) == prune,
        "fetches",
    )
}

/// What git's configuration says about pruning tags: `fetch.pruneTags` and a remote's own
/// `remote.<name>.pruneTags`.
fn prune_tags_note(facts: &GitFacts, prune_tags: bool, follow: bool) -> Option<Note> {
    let (fetch, remote): (Vec<ConfigValue>, Vec<ConfigValue>) = facts
        .prune_tags
        .iter()
        .cloned()
        .partition(|v| v.key.eq_ignore_ascii_case("fetch.pruneTags"));
    let set = fetch_values(facts, fetch.first(), &remote, "pruneTags");
    config_note(
        &set,
        follow,
        |v| v.as_bool().unwrap_or(false) == prune_tags,
        "fetches",
    )
}

/// What Canopy's Message cleanup row says under its title.
const MESSAGE_CLEANUP_HELP: &str =
    "What git removes from commit messages. Spaces and blank lines keeps # lines.";

/// What the Conflict style row says under its title; one line even at zoom 1.6, also in the
/// smallest window (`help_fits_on_one_line`).
const CONFLICT_STYLE_HELP: &str = "Showing the original too makes each side's change clear.";

/// The Autostash row's help. A squash merge is left out: git would keep its stash unapplied and
/// hidden, so `Repo::merge` never autostashes one, whatever Canopy's choice or git's key says.
const AUTOSTASH_HELP: &str = "Sets uncommitted changes aside around a pull, rebase or merge (never a squash) and restores them after.";

/// What git's configuration says about autostash: `rebase.autoStash` decides rebases and
/// `merge.autoStash` merges, so with only one set Canopy's switch still decides the other kind.
fn autostash_note(facts: &GitFacts, autostash: bool, follow: bool) -> Option<Note> {
    let values: Vec<&ConfigValue> = facts.autostash.iter().collect();
    let mut note = config_note(
        &values,
        follow,
        |v| v.as_bool() == Some(autostash),
        "pulls, rebases and merges",
    )?;
    if let (true, [only]) = (follow, values.as_slice()) {
        let rest = if only.key.eq_ignore_ascii_case("rebase.autoStash") {
            "merges"
        } else {
            "rebases"
        };
        note.text = format!("{} Canopy's switch decides {rest}.", note.text);
    }
    Some(note)
}

/// What git's configuration says about one of Canopy's choices, set in `values`:
/// - while Canopy follows it (`follow`), that it decides;
/// - otherwise, that it matches Canopy's choice (`ours`), or that Canopy's own choice applies to its
///   does not apply to Canopy's `what`.
///
/// `None` when the configuration sets nothing.
fn config_note(
    values: &[&ConfigValue],
    follow: bool,
    ours: impl Fn(&ConfigValue) -> bool,
    what: &str,
) -> Option<Note> {
    if values.is_empty() {
        return None;
    }
    let phrases = |values: &[&ConfigValue]| {
        and_list(&values.iter().map(|v| config_phrase(v)).collect::<Vec<_>>())
    };
    if follow {
        return Some(Note {
            text: format!("{DECIDES}: {}.", phrases(values)),
            warn: false,
        });
    }
    let differ: Vec<&ConfigValue> = values.iter().copied().filter(|v| !ours(v)).collect();
    Some(if differ.is_empty() {
        Note {
            text: format!("Matches your git configuration: {}.", phrases(values)),
            warn: false,
        }
    } else {
        Note {
            text: format!(
                "Canopy's {what} use this choice instead of your git configuration: {}.",
                phrases(&differ),
            ),
            warn: true,
        }
    })
}

/// `items` joined as a sentence lists them: "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The background fetch intervals offered, in minutes (0 = off), with their labels.
fn fetch_intervals() -> Vec<(u32, String)> {
    [0, 5, 10, 15, 30]
        .into_iter()
        .map(|m| (m, fetch_interval_label(m)))
        .collect()
}

fn fetch_interval_label(minutes: u32) -> String {
    if minutes == 0 {
        "Off".to_owned()
    } else {
        format!("Every {minutes} minutes")
    }
}

/// A button that asks to be confirmed in place: the first click turns it into Cancel and a
/// danger button labelled `confirm`; returns true once that is clicked.
fn confirm_button(ui: &mut Ui, salt: &str, label: &str, confirm: &str) -> bool {
    let id = ui.make_persistent_id(("settings-confirm", salt));
    let armed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let mut confirmed = false;
    let mut next = armed;
    // Right to left: the confirming button is rightmost.
    if armed {
        if Button::new(confirm)
            .danger()
            .id_salt(("settings-confirm-yes", salt))
            .show(ui)
            .clicked()
        {
            confirmed = true;
            next = false;
        }
        if Button::new("Cancel")
            .ghost()
            .id_salt(("settings-confirm-no", salt))
            .show(ui)
            .clicked()
        {
            next = false;
        }
    } else if Button::new(format!("{label}…"))
        .icon(icon::ARROW_COUNTER_CLOCKWISE)
        .id_salt(("settings-confirm", salt))
        .show(ui)
        .clicked()
    {
        next = true;
    }
    ui.data_mut(|d| d.insert_temp(id, next));
    confirmed
}

/// The seam between two rows (nothing when the search left the row out).
fn row_end(ui: &mut Ui) {
    if !last_row_shown(ui) {
        return;
    }
    let s = theme::surfaces(ui.ctx());
    ui.add_space(theme::SM);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), paint::SEAM), Sense::hover());
    paint::seam_h(ui.painter(), rect.x_range(), rect.center().y, s.border);
    ui.add_space(theme::SM);
}

/// Help or detail text under a row's title.
fn help_label(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let words = Filter::get(ui.ctx()).words;
    let job = marked(
        text,
        &words,
        &FontId::proportional(m.meta_size),
        p.text_dim,
        p.accent,
        p.accent_tint(),
    );
    ui.add(egui::Label::new(job).wrap());
}

/// A row whose `help` is one or two sentences. A longer `detail` shows as the tooltip of a help
/// mark after the title. The caller ends the row with [`row_end`].
fn row_more(ui: &mut Ui, title: &str, help: &str, detail: &str, control: impl FnOnce(&mut Ui)) {
    let help_text = |ui: &mut Ui| {
        if !help.is_empty() {
            help_label(ui, help);
        }
    };
    row_layout(ui, title, None, help, &help_text, detail, control);
}

/// Whether the row drawn last was shown (a search can leave rows out); what belongs to a row
/// beneath it (its seam, a list) is drawn only then.
fn last_row_shown(ui: &Ui) -> bool {
    let f = Filter::get(ui.ctx());
    f.words.is_empty() || f.shown
}

/// The widest a row's title and help may run beside its control, about 90 characters.
const HELP_MAX_WIDTH: f32 = 640.0;

/// [`row_more`] with the help drawn by `help`; `text` is what a search finds it by besides its
/// title and detail. While searching, a row that does not match is not drawn at all.
fn row_layout(
    ui: &mut Ui,
    title: &str,
    tag: Option<&str>,
    text: &str,
    help: &dyn Fn(&mut Ui),
    detail: &str,
    control: impl FnOnce(&mut Ui),
) {
    noted_row_layout(ui, title, tag, text, help, detail, None, control);
}

/// [`row_layout`] with a remark about the row's current state as a mark after its title.
#[expect(
    clippy::too_many_arguments,
    reason = "row_layout's parts plus the note"
)]
fn noted_row_layout(
    ui: &mut Ui,
    title: &str,
    tag: Option<&str>,
    text: &str,
    help: &dyn Fn(&mut Ui),
    detail: &str,
    note: Option<&Note>,
    control: impl FnOnce(&mut Ui),
) {
    if !reveal(ui, title, &[text, detail]) {
        return;
    }
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let words = Filter::get(ui.ctx()).words;
    let width = ui.available_width();
    let label = |ui: &mut Ui| {
        ui.spacing_mut().item_spacing.y = theme::XS;
        let shown = marked(
            title,
            &words,
            &FontId::new(m.body_size, theme::semibold()),
            p.text,
            p.accent,
            p.accent_tint(),
        );
        ui.scope(|ui| {
            // As tall as the title, not as a control.
            ui.spacing_mut().interact_size.y = 0.0;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = theme::XS;
                ui.label(shown);
                if !detail.is_empty() {
                    HelpMark::new(title, detail).show(ui);
                }
                if let Some(note) = note {
                    // Beside an explanation mark, the state remark is set apart from it.
                    let mark = HelpMark::new(title, &note.text).notice(note.warn);
                    if detail.is_empty() {
                        mark.show(ui);
                    } else {
                        ui.add_space(theme::XS);
                        mark.apart().show(ui);
                    }
                }
                if let Some(tag) = tag {
                    ui.add_space(theme::XS);
                    Badge::new(tag)
                        .tone(Tone::Accent)
                        .tooltip(OVERRIDDEN_TIP)
                        .show(ui);
                }
            });
        });
        help(ui);
    };
    ui.push_id(title, |ui| {
        if width < 620.0 {
            label(ui);
            ui.add_space(theme::XS);
            ui.allocate_ui_with_layout(
                vec2(width, 0.0),
                Layout::right_to_left(Align::Center).with_main_wrap(true),
                |ui| {
                    ui.add_space(theme::SM);
                    run_control(ui, control);
                },
            );
        } else {
            // The text takes the room the control left last frame, so help stays on one line
            // where it can; at least half the row, at most a readable measure.
            let key = ui.id().with("control-width");
            let control_width: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(width * 0.5);
            // Room for the reset button, which sits left of the control while it shows.
            let reset =
                crate::ui::kit::button::icon_button_side(Size::Sm) + ui.spacing().item_spacing.x;
            let text_width = (width - control_width - reset - theme::XL)
                .clamp(width * 0.5, HELP_MAX_WIDTH.max(width * 0.5));
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(text_width, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        // Without this the column is only as wide as its text, and the controls
                        // of rows with short help text sit at different distances from the edge.
                        ui.set_min_width(text_width);
                        label(ui);
                    },
                );
                ui.allocate_ui_with_layout(
                    vec2(width - text_width - 2.0 * theme::SM, 0.0),
                    Layout::right_to_left(Align::Center).with_main_wrap(true),
                    |ui| {
                        // Keeps controls clear of the scroll bar.
                        ui.add_space(theme::SM);
                        run_control(ui, control);
                        // A wrapped control measures only its widest line: give it half the row
                        // again, so the next frame measures it on one line.
                        let wrapped = ui
                            .data(|d| d.get_temp::<ControlRoom>(ui.id().with("control-room")))
                            .is_some_and(|room| room.wrapped);
                        let used = if wrapped {
                            width * 0.5
                        } else {
                            ui.min_rect().width()
                        };
                        if (used - control_width).abs() > 0.5 {
                            ui.data_mut(|d| d.insert_temp(key, used));
                            ui.ctx().request_repaint();
                        }
                    },
                );
            });
        }
    });
}

/// An uppercase group label over a few rows. While searching it waits until a row under it
/// matches (and when it matches the query itself, every row under it does).
fn heading(ui: &mut Ui, text: &str) {
    let mut f = Filter::get(ui.ctx());
    if f.words.is_empty() {
        heading_now(ui, text);
        return;
    }
    f.group_matches = matches_all(&f.words, &[text]);
    f.group = Some(text.to_owned());
    Filter::set(ui.ctx(), f);
}

fn heading_now(ui: &mut Ui, text: &str) {
    ui.add_space(theme::XS);
    let label = column_label(ui, text);
    if Filter::get(ui.ctx()).reveal == Some(text) {
        label.scroll_to_me_animation(Some(Align::TOP), egui::style::ScrollAnimation::none());
    }
    ui.add_space(theme::MD);
}

/// Space before a group heading; none while searching, where the row above ends in a seam.
fn group_gap(ui: &mut Ui, space: f32) {
    if !Filter::searching(ui.ctx()) {
        ui.add_space(space);
    }
}

/// A part of a page that is not a row (theme cards, token cards, the version line): drawn
/// always, or while searching when `title` or `text` matches.
fn block(ui: &mut Ui, title: &str, text: &str) -> bool {
    reveal(ui, title, &[text])
}

/// Lays `add` out in a `width`-wide column, so a kit field sits where a right-to-left row puts
/// it (a field lays itself out top-down from the left of what is left).
fn fixed_width(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    // A field's height, so a centred row centres the field with its neighbours.
    ui.allocate_ui_with_layout(
        vec2(width, Size::Md.height()),
        Layout::top_down(Align::Min),
        add,
    )
    .inner
}

/// Why `text` cannot be the first branch of a new repository: git must accept it as a branch
/// name (`refs/heads/<name>`, so no leading `-`). Empty is fine: the default is used.
fn initial_branch_problem(text: &str) -> Option<&'static str> {
    let name = text.trim();
    (!name.is_empty() && !crate::views::dialogs::valid_ref_name(name)).then_some(
        "Not a valid branch name: git refuses spaces, .., ~ ^ : ? * [ \\ and a leading -",
    )
}

/// What was typed in the first-branch field that is not a branch name yet, and the setting it
/// was typed over.
#[derive(Debug, Clone, PartialEq)]
struct BranchDraft {
    text: String,
    saved: String,
}

/// The first-branch field. A name git refuses shows its error as it is typed and is not kept:
/// `saved` holds the last name that was good until the text is one again.
fn initial_branch_field(ui: &mut Ui, saved: &mut String) {
    let id = ui.id().with("initial-branch-draft");
    // A draft over a setting that has since changed (reset, imported) is stale.
    let draft: Option<BranchDraft> = ui
        .data(|d| d.get_temp(id))
        .filter(|d: &BranchDraft| d.saved == *saved);
    let mut text = draft.map_or_else(|| saved.clone(), |d| d.text);
    let before = text.clone();
    let problem = initial_branch_problem(&text);
    fixed_width(ui, FIELD_WIDTH, |ui| {
        TextField::new(&mut text)
            .placeholder(crate::settings::INITIAL_BRANCH)
            .width(FIELD_WIDTH)
            .name("First branch of a new repository")
            .error(problem)
            .id_salt("initial-branch")
            .show(ui)
    });
    if text == before {
        return;
    }
    if initial_branch_problem(&text).is_none() {
        *saved = text;
        ui.data_mut(|d| d.remove::<BranchDraft>(id));
    } else {
        let saved = saved.clone();
        ui.data_mut(|d| d.insert_temp(id, BranchDraft { text, saved }));
    }
}

/// A switch whose name is its row's title, so it draws no label of its own.
fn switch(ui: &mut Ui, on: &mut bool, name: &str) -> Response {
    let was = *on;
    switch_with(Switch::new(on, ""), ui, was, name)
}

fn switch_with(switch: Switch<'_>, ui: &mut Ui, on: bool, name: &str) -> Response {
    let enabled = ui.is_enabled();
    let response = switch.show(ui);
    let on = on ^ response.changed();
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, name));
    response
}

/// The custom theme's choices: its base, the accent and the tint. Each applies as it changes.
fn custom_theme_rows(ui: &mut Ui, c: &mut CustomTheme) {
    let defaults = CustomTheme::default();
    let m = theme::metrics();
    let p = palette(ui.ctx());
    let font = FontId::new(m.mono_size, FontFamily::Monospace);
    // A slot as wide as the widest value, so the rail beside it stays put while dragging.
    let slot = ui
        .painter()
        .layout_no_wrap("000%".to_owned(), font.clone(), p.text_dim)
        .size()
        .x;
    let value = |ui: &mut Ui, text: String| {
        let height = ui.spacing().interact_size.y;
        ui.add_sized(
            [slot, height],
            egui::Label::new(RichText::new(text).font(font.clone()).color(p.text_dim)),
        );
    };
    row(ui, "Based on", "The theme your colours build on.", |ui| {
        revert(ui, &mut c.light, &defaults.light, "Based on");
        let options = [(false, "Midnight"), (true, "Daylight")];
        Segmented::new(&mut c.light, &options)
            .label("Based on")
            .show(ui);
    });
    let accents = custom_theme::HUES.map(|(hue, name)| (hue, c.accent_sample(hue), name));
    row(
        ui,
        "Accent",
        "The colour of buttons, links and selections.",
        |ui| {
            revert(ui, &mut c.accent_hue, &defaults.accent_hue, "Accent");
            Swatches::new(&mut c.accent_hue, &accents)
                .label("Accent")
                .id_salt("custom-accent")
                .show(ui);
        },
    );
    row(
        ui,
        "Accent hue",
        "Any hue between the colours above.",
        |ui| {
            value(ui, format!("{}°", c.accent_hue));
            Slider::new(&mut c.accent_hue, 0..=359)
                .label("Accent hue")
                .value_text(|v| format!("{v}°"))
                .width(ZOOM_RAIL_WIDTH)
                .id_salt("custom-accent-hue")
                .show(ui);
        },
    );
    row(ui, "Accent intensity", "How vivid the accent is.", |ui| {
        revert(
            ui,
            &mut c.accent_intensity,
            &defaults.accent_intensity,
            "Accent intensity",
        );
        value(ui, format!("{}%", c.accent_intensity));
        Slider::new(&mut c.accent_intensity, CustomTheme::ACCENT_INTENSITY)
            .label("Accent intensity")
            .value_text(|v| format!("{v}%"))
            .width(ZOOM_RAIL_WIDTH)
            .id_salt("custom-accent-intensity")
            .show(ui);
    });
    let tints = custom_theme::HUES.map(|(hue, name)| (hue, CustomTheme::tint_sample(hue), name));
    row(
        ui,
        "Tint",
        "Colours backgrounds, lines and text without losing contrast.",
        |ui| {
            revert(ui, &mut c.tint_hue, &defaults.tint_hue, "Tint");
            Swatches::new(&mut c.tint_hue, &tints)
                .label("Tint")
                .id_salt("custom-tint")
                .show(ui);
        },
    );
    row(ui, "Tint hue", "Any hue between the colours above.", |ui| {
        value(ui, format!("{}°", c.tint_hue));
        Slider::new(&mut c.tint_hue, 0..=359)
            .label("Tint hue")
            .value_text(|v| format!("{v}°"))
            .width(ZOOM_RAIL_WIDTH)
            .id_salt("custom-tint-hue")
            .show(ui);
    });
    row(
        ui,
        "Tint strength",
        "How strongly the tint shows. 0% keeps neutral backgrounds.",
        |ui| {
            revert(ui, &mut c.tint, &defaults.tint, "Tint strength");
            value(ui, format!("{}%", c.tint));
            Slider::new(&mut c.tint, CustomTheme::TINT)
                .label("Tint strength")
                .value_text(|v| format!("{v}%"))
                .width(ZOOM_RAIL_WIDTH)
                .id_salt("custom-tint-strength")
                .show(ui);
        },
    );
}

/// The fewest and most commits a history page loads, and the stepper's step.
const COMMIT_PAGE: RangeInclusive<f64> = 200.0..=50_000.0;
const COMMIT_PAGE_STEP: f64 = 100.0;
/// Maximum inactivity period offered by the cleanup setting (ten years).
const STALE_DAYS: RangeInclusive<f64> = 1.0..=3650.0;

fn appearance(ui: &mut Ui, s: &mut Settings, preview: &mut Option<f32>) {
    heading(ui, "Theme");
    if block(ui, "Theme", "") {
        theme_cards(ui, s);
    }
    if s.theme == ThemeId::Custom {
        group_gap(ui, theme::LG);
        heading(ui, "Custom theme");
        custom_theme_rows(ui, &mut s.custom_theme);
    }
    group_gap(ui, theme::LG);
    heading(ui, "Look");
    row(
        ui,
        "Style",
        "Standard is flat. Deeper adds floating panes, gradients and shadows.",
        |ui| {
            revert(ui, &mut s.style, &DEFAULTS.style, "Style");
            let options = Style::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.style, &options)
                .label("Style")
                .show(ui);
        },
    );
    row_explained(
        ui,
        "High contrast",
        "Stronger text, lines and focus rings, with outlines instead of shadows.",
        "For low vision and bright light. It also turns off translucency.",
        |ui| {
            revert(
                ui,
                &mut s.high_contrast,
                &DEFAULTS.high_contrast,
                "High contrast",
            );
            let on = s.high_contrast;
            switch_with(
                Switch::new(&mut s.high_contrast, ""),
                ui,
                on,
                "High contrast",
            );
        },
    );
    let blocked = theme::glass_blocked(
        s.theme,
        crate::ui::kit::glass::available(),
        crate::ui::kit::glass::reduced_transparency(),
    );
    row(
        ui,
        "Translucent surfaces",
        match blocked {
            Some(why) => why.reason(),
            None => "Blurs what is behind menus, dialogs and notifications.",
        },
        |ui| {
            revert(
                ui,
                &mut s.translucent,
                &DEFAULTS.translucent,
                "Translucent surfaces",
            );
            let on = s.translucent;
            switch_with(
                Switch::new(&mut s.translucent, "")
                    .enabled(blocked.is_none())
                    .disabled_reason(blocked.map_or("", GlassBlocked::reason)),
                ui,
                on,
                "Translucent surfaces",
            );
        },
    );
    row_explained(
        ui,
        "Density",
        "Dense fits about a quarter more history with smaller rows and text.",
        "Dense uses 24-point rows, a single-row toolbar and smaller text.",
        |ui| {
            revert(ui, &mut s.density, &DEFAULTS.density, "Density");
            let options = Density::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.density, &options)
                .label("Density")
                .show(ui);
        },
    );
    row(
        ui,
        "Request list",
        "Dense shows one line per pull or merge request, without the author line and the preview.",
        |ui| {
            revert(
                ui,
                &mut s.request_density,
                &DEFAULTS.request_density,
                "Request list",
            );
            let options = crate::request_view::RequestDensity::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.request_density, &options)
                .label("Request list")
                .show(ui);
        },
    );
    row(
        ui,
        "Graph nodes",
        "How commits are drawn in the history graph.",
        |ui| {
            revert(ui, &mut s.graph_node, &DEFAULTS.graph_node, "Graph nodes");
            let options = NodeShape::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.graph_node, &options)
                .label("Graph nodes")
                .show(ui);
        },
    );
    row(
        ui,
        "Graph lines",
        "The shape of the lines joining branches in the history graph.",
        |ui| {
            revert(ui, &mut s.graph_curve, &DEFAULTS.graph_curve, "Graph lines");
            let options = Curve::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.graph_curve, &options)
                .label("Graph lines")
                .show(ui);
        },
    );
    let system = match motion::system() {
        Some(true) => "Your system asks for reduced motion.",
        Some(false) => "Your system asks for full motion.",
        None => "Your system sets no preference, so Match system means full motion.",
    };
    row(ui, "Motion", system, |ui| {
        revert(ui, &mut s.motion, &DEFAULTS.motion, "Motion");
        let options = MotionPref::ALL.map(|v| (v, v.label()));
        Segmented::new(&mut s.motion, &options)
            .label("Motion")
            .show(ui);
    });
    group_gap(ui, theme::LG);
    heading(ui, "Interface");
    row(
        ui,
        "Zoom",
        &format!(
            "Scales the whole interface. {} and {} also work.",
            crate::keymap::hint(Command::ZoomIn),
            crate::keymap::hint(Command::ZoomOut)
        ),
        |ui| {
            if revert(ui, &mut s.zoom, &DEFAULTS.zoom, "Zoom") {
                *preview = None;
            }
            zoom_slider(ui, s, preview);
        },
    );
    row(
        ui,
        "Dates",
        "How the history shows when a commit was made.",
        |ui| {
            revert(ui, &mut s.relative_dates, &DEFAULTS.relative_dates, "Dates");
            Segmented::new(
                &mut s.relative_dates,
                &[(true, "Relative"), (false, "Calendar")],
            )
            .label("Dates")
            .show(ui);
        },
    );
    row_explained(
        ui,
        "Commit date format",
        "How calendar dates appear in History and commit details.",
        "Older commits shown with relative dates use it too. Exact-time tooltips always include the author's UTC offset.",
        |ui| {
            revert(
                ui,
                &mut s.iso_dates,
                &DEFAULTS.iso_dates,
                "Commit date format",
            );
            Segmented::new(
                &mut s.iso_dates,
                &[(false, "Sep 21, 2026"), (true, "2026-09-21")],
            )
            .label("Commit date format")
            .show(ui);
        },
    );
    row(
        ui,
        "Author avatars",
        "Pictures or coloured initials next to authors.",
        |ui| {
            revert(
                ui,
                &mut s.show_avatars,
                &DEFAULTS.show_avatars,
                "Author avatars",
            );
            switch(ui, &mut s.show_avatars, "Author avatars");
        },
    );
    tooltips(ui, s);
}

/// Which tooltips show, and how soon.
fn tooltips(ui: &mut Ui, s: &mut Settings) {
    use crate::ui::kit::overlay::{TooltipDelay, TooltipLevel};
    group_gap(ui, theme::LG);
    heading(ui, "Tooltips");
    row_explained(
        ui,
        "Show tooltips",
        "Helpful explains only what the screen doesn't show. All also describes every control.",
        "Helpful shows an icon's name, why something is unavailable, cut-off text, shortcuts, full dates and addresses. Off still says why something is unavailable.",
        |ui| {
            revert(ui, &mut s.tooltips, &DEFAULTS.tooltips, "Show tooltips");
            let options = TooltipLevel::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.tooltips, &options)
                .label("Show tooltips")
                .show(ui);
        },
    );
    // Off still explains why something is unavailable, after this delay too.
    row(
        ui,
        "Tooltip delay",
        "How long to hover before a tooltip appears.",
        |ui| {
            revert(
                ui,
                &mut s.tooltip_delay,
                &DEFAULTS.tooltip_delay,
                "Tooltip delay",
            );
            let options = TooltipDelay::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.tooltip_delay, &options)
                .label("Tooltip delay")
                .show(ui);
        },
    );
}

/// The notification cards: whether they show at all, and for which kinds of notification. The
/// notification center lists every notification either way.
fn notifications(ui: &mut Ui, n: &mut crate::settings::NotificationPrefs) {
    heading(ui, "Notifications");
    row(
        ui,
        "Notification cards",
        "Pop-up cards for new events. The bell in the status bar always lists every notification.",
        |ui| {
            revert(
                ui,
                &mut n.cards,
                &DEFAULTS.notifications.cards,
                "Notification cards",
            );
            switch(ui, &mut n.cards, "Notification cards");
        },
    );
    let cards = n.cards;
    for topic in crate::ui::toast::Topic::ALL {
        let default = *DEFAULTS.notifications.clone().topic_mut(topic);
        let on = n.topic_mut(topic);
        ui.add_enabled_ui(cards, |ui| {
            row(ui, topic.label(), topic.help(), |ui| {
                revert(ui, on, &default, topic.label());
                switch(ui, on, topic.label());
            });
        });
    }
}

/// The theme cards, whole cards per row.
fn theme_cards(ui: &mut Ui, s: &mut Settings) {
    let m = theme::metrics();
    // Whole cards per row, stretched to fill it, with a gutter clear of the scroll bar.
    let gap = m.card_gap;
    let usable = ui.available_width() - theme::LG;
    let columns =
        crate::ui::saturate_usize(((usable + gap) / (THEME_CARD_MIN + gap)).floor()).max(1);
    let card_width = ((usable - gap * crate::ui::count_f32(columns - 1))
        / crate::ui::count_f32(columns))
    .clamp(THEME_CARD_MIN - 20.0, THEME_CARD_MAX);
    // One Tab stop for the grid, the arrows between its cards.
    let grid = Roving::new(
        ui.ctx(),
        "settings-theme-cards",
        ThemeId::ALL.len(),
        ThemeId::ALL.iter().position(|t| *t == s.theme),
    )
    .columns(columns);
    ui.scope(|ui| {
        a11y::group(ui, egui::accesskit::Role::RadioGroup, "Theme");
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for (row, themes) in ThemeId::ALL.chunks(columns).enumerate() {
            ui.horizontal(|ui| {
                for (n, &t) in themes.iter().enumerate() {
                    if theme_card(
                        ui,
                        t,
                        s.theme == t,
                        card_width,
                        Some((grid, row * columns + n)),
                    )
                    .clicked()
                    {
                        s.theme = t;
                    }
                }
            });
        }
    });
}

/// The zoom range and step.
const ZOOM: RangeInclusive<f32> = 0.8..=1.6;
const ZOOM_STEP: f64 = 0.05;
/// The zoom slider's rail and the typed percentage beside it.
const ZOOM_RAIL_WIDTH: f32 = 150.0;
const ZOOM_FIELD_WIDTH: f32 = 72.0;

/// The zoom slider and the percentage beside it. Dragging previews the value; the interface
/// rescales once the drag ends (rescaling under the pointer would move the slider away from it).
/// A typed percentage applies on Enter.
fn zoom_slider(ui: &mut Ui, s: &mut Settings, draft: &mut Option<f32>) -> Response {
    let percent = |v: f64| format!("{:.0}%", v * 100.0);
    // Left to right, so the value follows the rail, wherever the row puts the pair.
    ui.allocate_ui_with_layout(
        vec2(ZOOM_RAIL_WIDTH + ZOOM_FIELD_WIDTH + theme::SM, 0.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            let mut preview = draft.unwrap_or(s.zoom);
            let response = Slider::new(&mut preview, ZOOM)
                .step(ZOOM_STEP)
                .label("Zoom")
                .value_text(|v| percent(f64::from(v)))
                .width(ZOOM_RAIL_WIDTH)
                .id_salt("settings-zoom")
                .show(ui);
            if response.dragged() || response.is_pointer_button_down_on() {
                *draft = Some(preview);
            } else {
                if response.changed() || draft.is_some() {
                    s.zoom = preview;
                }
                *draft = None;
            }
            // The slider steps; the field takes a typed percentage, so it has no steppers.
            let mut typed = f64::from(draft.unwrap_or(s.zoom));
            let field = KitNumber::new(
                &mut typed,
                f64::from(*ZOOM.start())..=f64::from(*ZOOM.end()),
            )
            .step(0.0)
            .format(percent, |text| {
                text.trim_end_matches('%')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|v| v / 100.0)
            })
            .name("Zoom percentage")
            .width(ZOOM_FIELD_WIDTH)
            .id_salt("zoom")
            .show(ui);
            if field.changed() {
                s.zoom = crate::ui::to_f32(typed);
                *draft = None;
            }
        },
    )
    .response
}

/// The narrowest and widest a theme card is laid out.
const THEME_CARD_MIN: f32 = 180.0;
const THEME_CARD_MAX: f32 = 200.0;
/// The miniature's height, and the size of the window it stands for (concept `.tcard svg`).
const MINI_HEIGHT: f32 = 58.0;
const MINI_VIEW: Vec2 = vec2(200.0, 62.0);
/// The selected card's tick: its diameter and its inset from the miniature's corner.
const TICK: f32 = 18.0;
const TICK_INSET: f32 = 5.0;

/// A theme card: a miniature of the window drawn from that theme's palette in the current
/// style, its name and whether it is dark or light. Selected: the card's focus ring and a tick.
fn theme_card(
    ui: &mut Ui,
    id: ThemeId,
    selected: bool,
    width: f32,
    roving: Option<(Roving, usize)>,
) -> Response {
    let tp = id.palette();
    let mut card = Card::new().interactive(true).selected(selected);
    if let Some((list, index)) = roving {
        card = card.roving(list, index);
    }
    let card = card
        .width(width)
        .label(if id == DEFAULTS.theme {
            format!("{} theme (recommended)", id.name())
        } else {
            format!("{} theme", id.name())
        })
        .tooltip(tp.concept)
        .id_salt(("theme-card", id))
        .show(ui, |ui| {
            let p = palette(ui.ctx());
            let m = theme::metrics();
            let w = ui.available_width();
            let (mini, _) = ui.allocate_exact_size(vec2(w, MINI_HEIGHT), Sense::hover());
            miniature(ui.painter(), mini, tp, tp.surfaces(theme::style()));
            if selected {
                let c = pos2(
                    mini.right() - TICK_INSET - TICK / 2.0,
                    mini.top() + TICK_INSET + TICK / 2.0,
                );
                // The miniatures have different backgrounds. A ring from the preview
                // palette keeps the selected mark legible on both light and dark cards.
                ui.painter().circle_filled(c, TICK / 2.0 + 2.0, tp.text);
                ui.painter().circle_filled(c, TICK / 2.0, p.accent);
                ui.painter().text(
                    c,
                    Align2::CENTER_CENTER,
                    icon::CHECK,
                    FontId::proportional(m.icon_xs),
                    p.on_accent,
                );
            }
            ui.add_space(theme::XS);
            let name = ui.painter().layout_no_wrap(
                id.name().to_owned(),
                FontId::new(m.body_size, theme::medium()),
                p.text,
            );
            let (caption, _) = ui.allocate_exact_size(vec2(w, name.size().y), Sense::hover());
            ui.painter().galley(caption.left_top(), name, p.text);
            ui.painter().text(
                caption.right_center(),
                Align2::RIGHT_CENTER,
                if id == DEFAULTS.theme {
                    "Recommended"
                } else if tp.dark {
                    "Dark"
                } else {
                    "Light"
                },
                FontId::proportional(m.small_size),
                p.text_faint,
            );
        });
    card.response
}

/// The window in miniature, as the approved preview draws it: the ground, the chrome column with
/// a few labels, the content pane (flush in Standard, a card in Deeper), two graph lanes with
/// circle nodes, message lines and a primary button. Coordinates are in a 200 x 62 view.
fn miniature(painter: &egui::Painter, rect: Rect, tp: &Palette, s: &Surfaces) {
    let m = theme::metrics();
    let deeper = s.pane_edge.is_some();
    let k = vec2(rect.width() / MINI_VIEW.x, rect.height() / MINI_VIEW.y);
    let at = |x: f32, y: f32| pos2(rect.left() + x * k.x, rect.top() + y * k.y);
    let area = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_max(at(x, y), at(x + w, y + h));
    let radius = CornerRadius::same(m.card_radius.saturating_sub(5).max(3));
    let small = CornerRadius::same(m.radius_sm.min(2));
    painter.rect_filled(rect, radius, s.bg);
    painter.rect_filled(
        area(0.0, 0.0, 56.0, MINI_VIEW.y),
        CornerRadius {
            nw: radius.nw,
            sw: radius.sw,
            ne: 0,
            se: 0,
        },
        s.panel,
    );
    if deeper {
        let pane = area(58.0, 4.0, 138.0, 54.0);
        painter.rect_filled(pane, small, s.pane);
        paint::rim(painter, pane, small, s.pane_edge.unwrap_or(s.border));
    } else {
        painter.rect_filled(
            area(56.0, 0.0, 144.0, MINI_VIEW.y),
            CornerRadius {
                ne: radius.ne,
                se: radius.se,
                nw: 0,
                sw: 0,
            },
            s.pane,
        );
        paint::seam_v(painter, at(56.0, 0.0).x, rect.y_range(), s.border);
    }
    let bar = |x: f32, y: f32, w: f32, h: f32, c: Color32| {
        painter.rect_filled(area(x, y, w, h), CornerRadius::same(theme::FULL), c);
    };
    bar(10.0, 12.0, 28.0, 4.0, tp.accent);
    bar(10.0, 22.0, 36.0, 3.0, tp.text_faint);
    bar(10.0, 30.0, 30.0, 3.0, tp.text_faint);
    let edge = Stroke::new(m.graph_edge, tp.lane(0));
    painter.line_segment([at(74.0, 8.0), at(74.0, 54.0)], edge);
    let branch = Stroke::new(m.graph_edge, tp.lane(1));
    painter.add(CubicBezierShape::from_points_stroke(
        [
            at(74.0, 20.0),
            at(74.0, 26.0),
            at(86.0, 26.0),
            at(86.0, 32.0),
        ],
        false,
        Color32::TRANSPARENT,
        branch,
    ));
    painter.line_segment([at(86.0, 32.0), at(86.0, 50.0)], branch);
    for (x, y, lane) in [(74.0, 14.0, 0), (86.0, 40.0, 1)] {
        let c = at(x, y);
        painter.circle_filled(c, m.graph_node_r + m.graph_ring, s.pane);
        painter.circle_filled(c, m.graph_node_r, tp.lane(lane));
    }
    bar(98.0, 12.0, 78.0, 3.0, tp.text);
    bar(98.0, 25.0, 60.0, 3.0, tp.text_dim);
    bar(98.0, 38.0, 70.0, 3.0, tp.text_dim);
    paint::fill(painter, area(146.0, 45.0, 42.0, 9.0), small, s.primary);
}

/// Where an identity value comes from, as a short phrase.
pub(crate) fn source_label(source: Option<&Source>) -> String {
    source_label_in(source, &crate::settings::home_dir())
}

/// [`source_label`], with `home` shown as `~`.
fn source_label_in(source: Option<&Source>, home: &Path) -> String {
    match source {
        Some(Source::Environment(key)) => format!("the {key} environment variable"),
        Some(Source::Config { scope, .. }) if scope == "command" => "the git command line".into(),
        Some(Source::Config { scope, origin }) => {
            let file = origin.strip_prefix("file:").unwrap_or(origin);
            // The usual files are named by what they cover, then by path; an included file by its
            // path alone.
            let usual = Path::new(file).file_name().is_some_and(|name| {
                ["config", "config.worktree", ".gitconfig", "gitconfig"]
                    .iter()
                    .any(|usual| name == *usual)
            });
            // An unknown home directory is an empty path, which every path starts with.
            let path = Path::new(file)
                .strip_prefix(home)
                .ok()
                .filter(|_| !home.as_os_str().is_empty())
                .map_or_else(|| file.to_owned(), |rest| format!("~/{}", rest.display()));
            let covers = match scope.as_str() {
                "local" if usual => "this repository",
                "worktree" if usual => "this worktree",
                "global" if usual => "your global git config",
                "system" if usual => "the system git config",
                _ => return path,
            };
            format!("{covers} ({path})")
        }
        Some(Source::Guessed) => "guessed by Git from this computer".into(),
        None => "not set".into(),
    }
}

/// Where Git takes one part of an identity from, cut in the middle to the column with the whole
/// of it in a tooltip. The field above shows a configured or guessed value already; one from the
/// environment is named here, since no field holds it.
fn in_use(field: &Field) -> Option<Note> {
    if field.value.is_empty() {
        return None;
    }
    let from = source_label(field.source.as_ref());
    Some(Note {
        text: if matches!(field.source, Some(Source::Environment(_))) {
            format!("In use: {}, from {from}.", field.value)
        } else {
            format!("From {from}.")
        },
        warn: false,
    })
}

/// `text` cut in its middle with an ellipsis until it fits in `max_width`.
fn elide_middle(ui: &Ui, text: &str, font: &FontId, max_width: f32) -> String {
    let width = |t: &str| {
        ui.painter()
            .layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| {
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let start: String = chars[..head].iter().collect();
        let end: String = chars[chars.len() - tail..].iter().collect();
        format!("{start}…{end}")
    };
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        if width(&build(mid)) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    build(low)
}

/// The width of a text field in a settings row.
const FIELD_WIDTH: f32 = 280.0;
/// The narrowest a settings select gets, so a short one is still an easy target.
const SELECT_MIN_WIDTH: f32 = 120.0;
/// The first-branch field: room for a branch name such as "main" or "develop".
const BRANCH_FIELD_WIDTH: f32 = 160.0;

/// A select's width in a settings row: as wide as its longest option needs
/// (`kit::select::fit_width`), as desktop pop-up buttons size to their content.
fn select_width<'l>(ui: &Ui, labels: impl IntoIterator<Item = &'l str>) -> f32 {
    SELECT_MIN_WIDTH.max(select::fit_width(ui, labels))
}

/// The width of the commits-per-page field, its steppers included.
const COUNT_FIELD_WIDTH: f32 = 120.0;

/// What is wrong with the identity, or that it is still being read.
fn identity_status(ui: &mut Ui, identity: Option<&Identity>, error: Option<&str>) {
    let p = palette(ui.ctx());
    if let Some(error) = error {
        Callout::new(Tone::Warning, error).show(ui);
        ui.add_space(theme::SM);
    } else if let Some(identity) = identity {
        if let Some(warning) = identity.warning() {
            Callout::new(Tone::Warning, warning).show(ui);
            ui.add_space(theme::SM);
        }
    } else {
        ui.label(RichText::new("Reading Git identity…").color(p.text_dim));
    }
}

fn identity_group(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    repo: Option<&Path>,
    identity: Option<&Identity>,
    error: Option<&str>,
    actions: &mut Vec<Action>,
) {
    let repo_path = repo.map(Path::to_path_buf);
    if screen.identity_for != repo_path {
        screen.identity_scope = if repo.is_some() {
            Scope::Local
        } else {
            Scope::Global
        };
        screen.loaded_identity = None;
    }
    if screen.identity_for != repo_path || screen.loaded_identity.as_ref() != identity {
        screen.identity_for.clone_from(&repo_path);
        screen.loaded_identity = identity.cloned();
        // Only values someone chose are offered for editing: a guess built from the user and host
        // names would be saved as if it were a choice.
        let chosen = |field: &Field| {
            if field.explicit() {
                field.value.clone()
            } else {
                String::new()
            }
        };
        screen.identity_name = identity.map_or_else(String::new, |i| chosen(&i.author.name));
        screen.identity_email = identity.map_or_else(String::new, |i| chosen(&i.author.email));
    }
    heading(ui, "Identity for future commits");
    // Status lines are not settings: a search shows only the rows.
    if !Filter::searching(ui.ctx()) {
        identity_status(ui, identity, error);
    }
    let current = |pick: fn(&Identity) -> &Field| identity.map(pick).and_then(in_use);
    // What git would use while the field is empty (a guess from the user and host names, or a
    // value nobody can save here) shows as its placeholder, never as a value to save.
    let unchosen = |pick: fn(&Identity) -> &Field| {
        identity
            .map(pick)
            .filter(|f| !f.explicit() && !f.value.is_empty())
            .map_or_else(String::new, |f| format!("{} (guessed by git)", f.value))
    };
    noted_row_layout(
        ui,
        "Name",
        None,
        "",
        &|ui: &mut Ui| help_label(ui, NAME_HELP),
        "",
        current(|i| &i.author.name).as_ref(),
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut screen.identity_name)
                    .placeholder(unchosen(|i| &i.author.name))
                    .width(FIELD_WIDTH)
                    .name("Name")
                    .id_salt("identity-name")
                    .show(ui)
            });
        },
    );
    row_end(ui);
    noted_row_layout(
        ui,
        "Email",
        None,
        "",
        &|ui: &mut Ui| help_label(ui, EMAIL_HELP),
        "",
        current(|i| &i.author.email).as_ref(),
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut screen.identity_email)
                    .placeholder(unchosen(|i| &i.author.email))
                    .width(FIELD_WIDTH)
                    .name("Email")
                    .id_salt("identity-email")
                    .show(ui)
            });
        },
    );
    row_end(ui);
    if let Some(i) = identity.filter(|i| i.committer != i.author) {
        row(
            ui,
            "Committer",
            &format!(
                "Git records {} <{}> as the committer ({}). Saving here does not change it.",
                i.committer.name.value,
                i.committer.email.value,
                source_label(i.committer.name.source.as_ref())
            ),
            |_| {},
        );
    }
    if repo.is_some() {
        row(
            ui,
            "Save to",
            "Whether the identity applies to all repositories or only this one.",
            |ui| {
                Segmented::new(
                    &mut screen.identity_scope,
                    &[
                        (Scope::Global, "All repositories"),
                        (Scope::Local, "This repository"),
                    ],
                )
                .label("Save to")
                .show(ui);
            },
        );
    }
    let can_save =
        !screen.identity_name.trim().is_empty() && !screen.identity_email.trim().is_empty();
    row(
        ui,
        "Apply identity",
        "Existing commits keep their authors.",
        |ui| {
            // Right to left: the primary action is rightmost.
            if Button::new("Save identity")
                .primary()
                .enabled(can_save)
                .disabled_reason("Enter both a name and an email before saving.")
                .show(ui)
                .clicked()
            {
                actions.push(Action::SaveIdentity {
                    repo: repo_path.clone(),
                    scope: screen.identity_scope,
                    name: screen.identity_name.clone(),
                    email: screen.identity_email.clone(),
                });
            }
            if Button::new("Clear…")
                .icon(icon::ERASER)
                .icon_tone(Tone::Danger)
                .show(ui)
                .clicked()
            {
                actions.push(Action::OpenDialog(Dialog::ConfirmClearIdentity {
                    repo: repo_path.clone(),
                    scope: screen.identity_scope,
                }));
            }
        },
    );
}

/// Edits the selected Git file; inherited and included values remain visible and removable
/// only from their own source. All writes go through the serial background worker.
fn git_config_editor(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    repo: Option<&Path>,
    scope: Scope,
    facts: Option<&GitFacts>,
    editable: bool,
    actions: &mut Vec<Action>,
) {
    heading(
        ui,
        if scope == Scope::Global {
            "Global Git configuration"
        } else {
            "Repository Git configuration"
        },
    );
    // Captions are not settings: a search shows only the rows.
    if !Filter::searching(ui.ctx()) {
        help_label(
            ui,
            match (editable, scope) {
                (false, Scope::Global) => {
                    "Read only. Turn on Edit my git config above to change these."
                }
                (false, _) => {
                    "Read only. Turn on Edit my git config in Settings → Git to change these."
                }
                (true, Scope::Global) => "Changes apply to Git everywhere, not only in Canopy.",
                (true, _) => "Changes apply to this repository everywhere, not only in Canopy.",
            },
        );
        ui.add_space(theme::SM);
    }
    // A read error stays editable: saving a valid value is how the user repairs it.
    let ready = editable && facts.is_some() && !screen.git_config_pending;
    if screen.git_config_pending && !Filter::searching(ui.ctx()) {
        help_label(ui, "Saving Git configuration…");
    }
    for setting in Setting::ALL {
        // Git reads it only to create a repository, never from one that exists.
        if scope == Scope::Local && setting == Setting::InitialBranch {
            continue;
        }
        let value = facts.and_then(|facts| editor_value(facts, setting));
        let recommended = setting.recommendation_for(facts.and_then(|facts| facts.git));
        let invalid = facts.and_then(|facts| {
            facts
                .invalid
                .iter()
                .find(|value| value.key.eq_ignore_ascii_case(setting.key()))
        });
        let note = match (invalid, value) {
            (Some(raw), _) => Some(Note {
                text: format!(
                    "Git does not accept {} in {}.",
                    raw.value,
                    source_label(Some(&raw.source()))
                ),
                warn: true,
            }),
            (None, Some(value)) => Some(Note {
                text: format!(
                    "Set in {}.{}",
                    source_label(Some(&value.source())),
                    shown_only_reason(setting, &value.value)
                ),
                warn: false,
            }),
            (None, None) => None,
        };
        let help = git_setting_help(setting);
        row_with_note(ui, setting.label(), help, note.as_ref(), |ui| {
            // Read only: the value as text, so nothing looks like a control that does nothing.
            if !editable {
                ui.label(invalid.or(value).map_or_else(
                    || unset_value(setting, recommended),
                    |value| git_value_label(setting, &value.value),
                ));
                return;
            }
            ui.add_enabled_ui(ready, |ui| {
                let mut selected = value.map(|value| value.value.clone());
                if setting == Setting::InitialBranch {
                    let id = ui
                        .id()
                        .with(("git-branch-draft", repo, scope == Scope::Global));
                    let saved = selected.clone().unwrap_or_default();
                    let draft: Option<BranchDraft> = ui
                        .data(|d| d.get_temp(id))
                        .filter(|draft: &BranchDraft| draft.saved == saved);
                    let mut text = draft.map_or_else(|| saved.clone(), |draft| draft.text);
                    let problem = initial_branch_problem(&text);
                    // A select says it after its value; a field's text is the user's own, so
                    // the mark goes under it, where a field's messages go.
                    let marked = text.trim() == recommended;
                    let mut field = TextField::new(&mut text)
                        .width(BRANCH_FIELD_WIDTH)
                        .name(setting.label())
                        .placeholder(format!("{recommended} (Recommended)"));
                    if marked {
                        field = field.helper("(Recommended)");
                    }
                    field.error(problem).id_salt("git-first-branch").show(ui);
                    if Button::new("Save")
                        .enabled(!text.trim().is_empty() && problem.is_none())
                        .id_salt("save-git-first-branch")
                        .show(ui)
                        .clicked()
                    {
                        selected = Some(text.trim().to_owned());
                        actions.push(Action::SaveGitConfig {
                            repo: repo.map(Path::to_path_buf),
                            scope,
                            setting,
                            value: selected,
                        });
                        screen.git_config_pending = true;
                    }
                    ui.data_mut(|d| d.insert_temp(id, BranchDraft { text, saved }));
                } else {
                    let labels: Vec<_> = setting
                        .values()
                        .iter()
                        .map(|value| git_value_label(setting, value))
                        .collect();
                    let options: Vec<_> = setting
                        .values()
                        .iter()
                        .zip(&labels)
                        .map(|(value, label)| (Some((*value).to_owned()), label.as_str()))
                        .collect();
                    let select = Select::new(&mut selected, &options)
                        .fit_to_options()
                        .name(setting.label())
                        .report_reselection()
                        .placeholder(invalid.or(value).map_or_else(
                            || unset_value(setting, recommended),
                            |value| git_value_label(setting, &value.value),
                        ))
                        .id_salt(("git-setting", setting, scope == Scope::Global))
                        .recommended(Some(recommended.to_owned()));
                    if select.show(ui).changed() {
                        actions.push(Action::SaveGitConfig {
                            repo: repo.map(Path::to_path_buf),
                            scope,
                            setting,
                            value: selected,
                        });
                        screen.git_config_pending = true;
                    }
                }
                let direct = facts.is_some_and(|facts| {
                    facts
                        .direct
                        .iter()
                        .any(|value| value.key.eq_ignore_ascii_case(setting.key()))
                });
                // A repository's own value can go back to the global one; a global value is
                // changed by choosing another, never cleared.
                if direct
                    && scope == Scope::Local
                    && IconButton::new(
                        icon::ARROW_COUNTER_CLOCKWISE,
                        format!("Reset {} to the global value", setting.label()),
                    )
                    .size(Size::Sm)
                    .id_salt(("unset-git", setting))
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::SaveGitConfig {
                        repo: repo.map(Path::to_path_buf),
                        scope,
                        setting,
                        value: None,
                    });
                    screen.git_config_pending = true;
                }
            });
        });
    }
    if let Some(hooks) = facts.and_then(|facts| facts.hooks_path.as_ref()) {
        let note = Note {
            text: format!("{}.", config_phrase(hooks)),
            warn: false,
        };
        row_with_note(
            ui,
            "Hooks path",
            "Git runs the hooks in this folder for the operations you start.",
            Some(&note),
            |_| {},
        );
    }
}

fn editor_value(facts: &GitFacts, setting: Setting) -> Option<&ConfigValue> {
    facts
        .values
        .iter()
        .find(|value| value.key.eq_ignore_ascii_case(setting.key()))
}

fn git_value_label(setting: Setting, value: &str) -> String {
    // A setting that takes a boolean or a word reads `yes` or `1` as `true`, as git does.
    let value = match setting {
        Setting::PullRebase | Setting::PullFastForward | Setting::Autocrlf if !value.is_empty() => {
            match crate::repo_config::git_bool(value) {
                Some(true) => "true",
                Some(false) => "false",
                None => value,
            }
        }
        _ => value,
    };
    // One name for each cleanup, as Canopy's own Message cleanup row has it.
    if setting == Setting::CommitCleanup
        && let Some(cleanup) = canopy_core::Cleanup::from_value(value)
    {
        return cleanup.label().to_owned();
    }
    match (setting, value) {
        (Setting::PullRebase, "false") => "Don't rebase",
        (Setting::PullRebase, "true") => "Rebase",
        (Setting::PullRebase, "merges") => "Rebase preserving merges",
        (Setting::PullRebase, "interactive") => "Interactive rebase",
        (Setting::PullFastForward, "only") => "Fast-forward only",
        (Setting::PullFastForward, "true") => "Allow fast-forward",
        (Setting::PullFastForward, "false") => "Always merge",
        (Setting::Autocrlf, "input") => "LF on commit",
        (Setting::Autocrlf, "true") => "CRLF on checkout",
        (Setting::SigningFormat, "openpgp") => "OpenPGP",
        (Setting::SigningFormat, "ssh") => "SSH",
        (Setting::SigningFormat, "x509") => "X.509",
        (Setting::CommitCleanup, "scissors") => "Cut at the scissors line",
        (_, "true") => "On",
        (_, "false") => "Off",
        // Branch names and the diff3 styles are names, not words.
        (Setting::InitialBranch, _) => value,
        (Setting::ConflictStyle, _) if value.contains("diff3") => value,
        _ => return upper_first(value),
    }
    .to_owned()
}

fn git(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    s: &mut Settings,
    identity: &IdentityView<'_>,
    actions: &mut Vec<Action>,
) {
    identity_group(
        ui,
        screen,
        identity.repo,
        identity.identity,
        identity.error,
        actions,
    );
    group_gap(ui, theme::LG);
    let facts = screen.facts.global(ui.ctx()).cloned();
    let f = facts.as_deref();
    let follow = s.follow_git_config;
    // While Canopy follows the git configuration, a choice it sets is not Canopy's to make.
    heading(ui, "Your git configuration");
    facts_error(ui, f);
    row(
        ui,
        "Follow my git config",
        "Where your git configuration sets one of these choices, it wins over Canopy's.",
        |ui| {
            revert(
                ui,
                &mut s.follow_git_config,
                &DEFAULTS.follow_git_config,
                "Follow my git config",
            );
            switch(ui, &mut s.follow_git_config, "Follow my git config");
        },
    );
    row(
        ui,
        "Edit my git config",
        "Lets Canopy write the git settings you choose here. Canopy works without it.",
        |ui| {
            revert(
                ui,
                &mut s.edit_git_config,
                &DEFAULTS.edit_git_config,
                "Edit my git config",
            );
            switch(ui, &mut s.edit_git_config, "Edit my git config");
        },
    );
    let editable = s.edit_git_config;
    if editable {
        row(
            ui,
            "Recommended settings",
            "Writes the recommended values to your global git config. Signing and line endings stay.",
            |ui| {
                ui.add_enabled_ui(!screen.git_config_pending && f.is_some(), |ui| {
                    if confirm_button(
                        ui,
                        "use-recommended-git",
                        "Use recommended",
                        "Apply to global Git config",
                    ) {
                        screen.git_config_pending = true;
                        actions.push(Action::UseRecommendedGitConfig);
                    }
                });
            },
        );
    }
    if follow {
        git_config_editor(ui, screen, None, Scope::Global, f, editable, actions);
    }
    if !follow {
        heading(ui, "Canopy choices");
        heading(ui, "Remote");
        default_row(
            ui,
            "Pull strategy",
            "How a pull combines the remote's commits with yours.",
            f.and_then(|f| pull_note(f, s.pull_default())).as_ref(),
            |ui| {
                revert(ui, &mut s.pull_mode, &DEFAULTS.pull_mode, "Pull strategy");
                let options = PullMode::ALL.map(|m| (m, m.label()));
                Select::new(&mut s.pull_mode, &options)
                    .width(select_width(ui, options.iter().map(|(_, l)| *l)))
                    .name("Pull strategy")
                    .recommended(DEFAULTS.pull_mode)
                    .id_salt("pull-strategy")
                    .show(ui);
            },
        );
        default_row(
            ui,
            "Prune on fetch",
            "Removes remote branches that were deleted on the remote.",
            f.and_then(|f| prune_note(f, s.prune_on_fetch, follow))
                .as_ref(),
            |ui| {
                revert(
                    ui,
                    &mut s.prune_on_fetch,
                    &DEFAULTS.prune_on_fetch,
                    "Prune on fetch",
                );
                switch(ui, &mut s.prune_on_fetch, "Prune on fetch");
            },
        );
        row_with_note(
            ui,
            "Prune tags too",
            "Also deletes local tags the remote doesn't have, even ones you never pushed.",
            f.and_then(|f| prune_tags_note(f, s.prune_tags, follow))
                .as_ref(),
            |ui| {
                revert(
                    ui,
                    &mut s.prune_tags,
                    &DEFAULTS.prune_tags,
                    "Prune tags too",
                );
                switch(ui, &mut s.prune_tags, "Prune tags too");
            },
        );
        row_with_note(
            ui,
            "Autostash",
            AUTOSTASH_HELP,
            f.and_then(|f| autostash_note(f, s.autostash, follow))
                .as_ref(),
            |ui| {
                revert(ui, &mut s.autostash, &DEFAULTS.autostash, "Autostash");
                switch(ui, &mut s.autostash, "Autostash");
            },
        );
        row_with_note(
            ui,
            "Push tags with their commits",
            "Pushes the annotated tags on the commits you push.",
            f.and_then(|f| {
                config_note(
                    &f.follow_tags.iter().collect::<Vec<_>>(),
                    follow,
                    |v| v.as_bool() == Some(s.follow_tags),
                    "pushes",
                )
            })
            .as_ref(),
            |ui| {
                revert(
                    ui,
                    &mut s.follow_tags,
                    &DEFAULTS.follow_tags,
                    "Push tags with their commits",
                );
                switch(ui, &mut s.follow_tags, "Push tags with their commits");
            },
        );
        row_with_note(
            ui,
            "Update submodules",
            "Pulls, checkouts and new clones also update submodules.",
            f.and_then(|f| {
                config_note(
                    &f.submodule_recurse.iter().collect::<Vec<_>>(),
                    follow,
                    |v| v.as_bool() == Some(s.recurse_submodules),
                    "pulls and checkouts",
                )
            })
            .as_ref(),
            |ui| {
                revert(
                    ui,
                    &mut s.recurse_submodules,
                    &DEFAULTS.recurse_submodules,
                    "Update submodules",
                );
                switch(ui, &mut s.recurse_submodules, "Update submodules");
            },
        );
        row_with_note(
            ui,
            "Push to",
            "Where Push sends a branch that has an upstream.",
            f.and_then(|f| {
                let ours = s.push_default.push_default().value();
                config_note(
                    &f.push_default.iter().collect::<Vec<_>>(),
                    follow,
                    |v| v.value.trim().eq_ignore_ascii_case(ours),
                    "pushes",
                )
            })
            .as_ref(),
            |ui| {
                revert(ui, &mut s.push_default, &DEFAULTS.push_default, "Push to");
                let options = PushMode::ALL.map(|m| (m, m.label()));
                Select::new(&mut s.push_default, &options)
                    .width(select_width(ui, options.iter().map(|(_, l)| *l)))
                    .name("Push to")
                    .recommended(DEFAULTS.push_default)
                    .id_salt("push-default")
                    .show(ui);
            },
        );
    }
    heading(ui, "Fetch");
    default_row(
        ui,
        "Fetch in the background",
        "Fetches the remotes of open repositories while Canopy is idle.",
        None,
        |ui| {
            revert(
                ui,
                &mut s.auto_fetch_minutes,
                &DEFAULTS.auto_fetch_minutes,
                "Fetch in the background",
            );
            let labels = fetch_intervals();
            let options: Vec<(u32, &str)> = labels.iter().map(|(m, l)| (*m, l.as_str())).collect();
            // A stored interval the list does not offer still shows as itself.
            let current = fetch_interval_label(s.auto_fetch_minutes);
            Select::new(&mut s.auto_fetch_minutes, &options)
                .placeholder(current)
                .fit_to_options()
                .name("Fetch in the background")
                .recommended(DEFAULTS.auto_fetch_minutes)
                .id_salt("auto-fetch")
                .show(ui);
        },
    );
    default_row(
        ui,
        "Remove deleted branches",
        "After a fetch that prunes, deletes your merged branches that were deleted on the remote.",
        None,
        |ui| {
            revert(
                ui,
                &mut s.remove_gone_branches,
                &DEFAULTS.remove_gone_branches,
                "Remove deleted branches",
            );
            switch(ui, &mut s.remove_gone_branches, "Remove deleted branches");
        },
    );
    heading(ui, "Cleanup");
    row(
        ui,
        "Stale cleanup after (days)",
        "Clean up stale… lists worktrees and branches idle this long, and asks before removing any.",
        |ui| {
            revert(
                ui,
                &mut s.stale_inactivity_days,
                &DEFAULTS.stale_inactivity_days,
                "Stale cleanup after (days)",
            );
            stale_days(ui, s);
        },
    );
    if !follow {
        heading(ui, "Conflicts");
        row_with_note(
            ui,
            "Conflict style",
            CONFLICT_STYLE_HELP,
            f.and_then(|f| {
                let ours = s.conflict_style.style();
                config_note(
                    &f.conflict_style.iter().collect::<Vec<_>>(),
                    follow,
                    |v| canopy_core::ConflictStyle::from_value(&v.value) == Some(ours),
                    "merges, rebases, cherry-picks, reverts and stashes",
                )
            })
            .as_ref(),
            |ui| {
                revert(
                    ui,
                    &mut s.conflict_style,
                    &DEFAULTS.conflict_style,
                    "Conflict style",
                );
                let options = ConflictMode::ALL.map(|m| (m, m.label()));
                Select::new(&mut s.conflict_style, &options)
                    .width(select_width(ui, options.iter().map(|(_, l)| *l)))
                    .name("Conflict style")
                    .recommended(DEFAULTS.conflict_style)
                    .id_salt("conflict-style")
                    .show(ui);
            },
        );
    }
    heading(ui, "Commits");
    if !follow {
        row_with_note(
            ui,
            "Message cleanup",
            MESSAGE_CLEANUP_HELP,
            f.and_then(|f| {
                let ours = s.cleanup.cleanup().value();
                config_note(
                    &f.cleanup.iter().collect::<Vec<_>>(),
                    follow,
                    |v| v.value.trim().eq_ignore_ascii_case(ours),
                    "commits",
                )
            })
            .as_ref(),
            |ui| {
                revert(ui, &mut s.cleanup, &DEFAULTS.cleanup, "Message cleanup");
                let options = CleanupMode::ALL.map(|m| (m, m.label()));
                Select::new(&mut s.cleanup, &options)
                    .width(select_width(ui, options.iter().map(|(_, l)| *l)))
                    .name("Message cleanup")
                    .recommended(DEFAULTS.cleanup)
                    .id_salt("commit-cleanup")
                    .show(ui);
            },
        );
        row_with_note(
            ui,
            "First branch of a new repository",
            "The branch a repository created in Canopy starts on.",
            f.and_then(|f| {
                let ours = s.initial_branch().to_owned();
                config_note(
                    &f.initial_branch.iter().collect::<Vec<_>>(),
                    follow,
                    |v| v.value.trim() == ours,
                    "new repositories",
                )
            })
            .as_ref(),
            |ui| {
                revert(
                    ui,
                    &mut s.initial_branch,
                    &DEFAULTS.initial_branch,
                    "First branch of a new repository",
                );
                initial_branch_field(ui, &mut s.initial_branch);
            },
        );
    }
    default_row(
        ui,
        "Add Signed-off-by",
        "Adds a Signed-off-by line to commits, as DCO projects require.",
        None,
        |ui| {
            revert(ui, &mut s.signoff, &DEFAULTS.signoff, "Add Signed-off-by");
            switch(ui, &mut s.signoff, "Add Signed-off-by");
        },
    );
    row_explained(
        ui,
        "Check commit messages",
        "Warns when the subject or a body line is longer than 72 characters. It never blocks a commit.",
        "Most tools cut off or wrap longer lines. A hint never changes what you wrote.",
        |ui| {
            revert(
                ui,
                &mut s.check_message,
                &DEFAULTS.check_message,
                "Check commit messages",
            );
            switch(ui, &mut s.check_message, "Check commit messages");
        },
    );
    row(
        ui,
        "Commits per page",
        "How many commits the history loads before \"Load more\".",
        |ui| {
            revert(
                ui,
                &mut s.commit_page,
                &DEFAULTS.commit_page,
                "Commits per page",
            );
            commit_page(ui, s);
        },
    );
    heading(ui, "Submodules");
    row(
        ui,
        "Discard in a submodule",
        "What Discard does with untracked files inside a submodule.",
        |ui| {
            revert(
                ui,
                &mut s.discard_untracked_in_submodules,
                &DEFAULTS.discard_untracked_in_submodules,
                "Discard in a submodule",
            );
            let options = [
                (false, "Keep untracked files"),
                (true, "Delete untracked files too"),
            ];
            Select::new(&mut s.discard_untracked_in_submodules, &options)
                .width(select_width(ui, options.iter().map(|(_, l)| *l)))
                .name("Discard in a submodule")
                .recommended(DEFAULTS.discard_untracked_in_submodules)
                .id_salt("discard-untracked-in-submodules")
                .show(ui);
        },
    );
    heading(ui, "Diffs");
    row(
        ui,
        "Layout",
        "Old and new lines in one column, or in two.",
        |ui| {
            revert(ui, &mut s.split_diff, &DEFAULTS.split_diff, "Diff layout");
            Segmented::new(
                &mut s.split_diff,
                &[(false, "Unified"), (true, "Side by side")],
            )
            .label("Diff layout")
            .show(ui);
        },
    );
    row(
        ui,
        "Syntax highlighting",
        "Colours code in diffs by language.",
        |ui| {
            revert(
                ui,
                &mut s.syntax_highlight,
                &DEFAULTS.syntax_highlight,
                "Syntax highlighting",
            );
            switch(ui, &mut s.syntax_highlight, "Syntax highlighting");
        },
    );
    default_row(
        ui,
        "Context lines",
        "Unchanged lines shown around each change.",
        None,
        |ui| {
            revert(
                ui,
                &mut s.diff.context,
                &DEFAULTS.diff.context,
                "Context lines",
            );
            let p = palette(ui.ctx());
            let m = theme::metrics();
            // Right to left: the value, then the rail.
            ui.label(
                RichText::new(s.diff.context.to_string())
                    .font(FontId::new(m.mono_size, FontFamily::Monospace))
                    .color(p.text_dim),
            );
            Slider::new(&mut s.diff.context, 0..=25)
                .label("Context lines")
                .value_text(|v| format!("{v} lines"))
                .width(ZOOM_RAIL_WIDTH)
                .id_salt("diff-context")
                .show(ui);
        },
    );
    default_row(
        ui,
        "Whitespace",
        "Ignoring whitespace hides space-only changes but turns off hunk staging.",
        None,
        |ui| {
            revert(
                ui,
                &mut s.diff.whitespace,
                &DEFAULTS.diff.whitespace,
                "Whitespace",
            );
            let options = WhitespaceMode::ALL.map(|m| (m, m.label()));
            Select::new(&mut s.diff.whitespace, &options)
                .fit_to_options()
                .name("Whitespace")
                .recommended(DEFAULTS.diff.whitespace)
                .id_salt("whitespace")
                .show(ui);
        },
    );
    let overridden = s.repos_with_overrides();
    if overridden > 0 {
        row(
            ui,
            "Repository choices",
            &match overridden {
                1 => "1 repository has settings of its own.".to_owned(),
                n => format!("{n} repositories have settings of their own."),
            },
            |ui| {
                if confirm_button(
                    ui,
                    "reset-repositories",
                    "Reset all",
                    "Reset every repository",
                ) {
                    actions.push(Action::ResetRepoSettings);
                }
            },
        );
    }
}

/// The active repository's own settings: each one follows the default from the app's settings
/// until the repository chooses otherwise, and shows which it does.
fn repository(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    s: &mut Settings,
    data: &SettingsData<'_>,
    actions: &mut Vec<Action>,
) {
    let Some(path) = data.identity.repo else {
        return;
    };
    let facts = screen.facts.repo(ui.ctx(), path).cloned();
    if block(ui, "This repository", &path.display().to_string()) {
        repo_header(ui, path, &RepoLook::of(s, path));
        facts_error(ui, facts.as_deref());
    }
    repo_tab(ui, s, path);
    repo_remotes(ui, screen, path, facts.as_deref(), actions);
    repo_identity(ui, screen, path, &data.identity, actions);

    git_config_editor(
        ui,
        screen,
        Some(path),
        Scope::Local,
        facts.as_deref(),
        s.edit_git_config,
        actions,
    );

    let mut own = s.repo(path);
    group_gap(ui, theme::LG);
    heading(ui, "Pull and fetch");
    let pull = s.pull_for(path);
    repo_row(
        ui,
        "Follow my git config for pulls",
        "Pulls use this repository's git configuration when it sets a strategy.",
        facts.as_ref().and_then(|f| pull_note(f, pull)).as_ref(),
        |ui| {
            override_select(
                ui,
                "repo-pull-follow",
                "Follow my git config for pulls",
                &mut own.pull_follow_config,
                s.follow_git_config,
                DEFAULTS.follow_git_config,
                &on_off(),
            );
        },
    );
    // While the git configuration decides a choice, Canopy's own is not offered: the
    // configuration's row below is where it changes, as on the Git page.
    let configured_pull = facts
        .as_ref()
        .and_then(|f| f.pull.as_ref())
        .is_some_and(|p| p.strategy.is_some());
    if !(pull.follow_config && configured_pull) {
        repo_row(
            ui,
            "Pull strategy",
            "How a pull combines the remote's commits with yours.",
            None,
            |ui| {
                override_select(
                    ui,
                    "repo-pull",
                    "Pull strategy",
                    &mut own.pull_mode,
                    s.pull_mode,
                    DEFAULTS.pull_mode,
                    &PullMode::ALL.map(|m| (m, m.label().to_owned())),
                );
            },
        );
    }
    let prune = own.prune_on_fetch.unwrap_or(s.prune_on_fetch);
    let configured_prune = facts.as_ref().is_some_and(|f| {
        !fetch_values(f, f.fetch_prune.as_ref(), &f.remote_prune, "prune").is_empty()
    });
    if !(s.follow_git_config && configured_prune) {
        repo_row(
            ui,
            "Prune on fetch",
            "Removes remote branches that were deleted on the remote.",
            facts
                .as_ref()
                .and_then(|f| prune_note(f, prune, s.follow_git_config))
                .as_ref(),
            |ui| {
                override_select(
                    ui,
                    "repo-prune",
                    "Prune on fetch",
                    &mut own.prune_on_fetch,
                    s.prune_on_fetch,
                    DEFAULTS.prune_on_fetch,
                    &on_off(),
                );
            },
        );
    }
    repo_row(
        ui,
        "Fetch in the background",
        "Fetches this repository's remotes and requests while Canopy is idle.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-auto-fetch",
                "Fetch in the background",
                &mut own.auto_fetch_minutes,
                s.auto_fetch_minutes,
                DEFAULTS.auto_fetch_minutes,
                &fetch_intervals(),
            );
        },
    );
    repo_row(
        ui,
        "Remote protocol",
        "The protocol for remote URLs Canopy builds when you add a remote or publish.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-protocol",
                "Remote protocol",
                &mut own.protocol,
                s.clone_protocol,
                DEFAULTS.clone_protocol,
                &CloneProtocol::ALL.map(|p| (p, p.label().to_owned())),
            );
        },
    );
    heading(ui, "Commits");
    repo_row(
        ui,
        "Add Signed-off-by",
        "Adds a Signed-off-by line to commits, as DCO projects require.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-signoff",
                "Add Signed-off-by",
                &mut own.signoff,
                s.signoff,
                DEFAULTS.signoff,
                &on_off(),
            );
        },
    );
    if let Some(facts) = &facts {
        signing_row(ui, facts, actions);
    }
    if own != s.repo(path) {
        *s.repo_mut(path) = own;
    }

    heading(ui, "Diffs");
    repo_diff_rows(ui, s, path, actions);

    heading(ui, "Hosting");
    if block(
        ui,
        "Pull and merge requests",
        "hosting account sign in github gitlab",
    ) && let Some(tab) = data.tabs.iter().find(|t| t.path == path)
    {
        hosting_account(ui, screen, s, tab);
    }
}

/// How the repository's tab looks: its own name and icon, else the folder's name and a folder.
struct RepoLook {
    name: String,
    icon: &'static str,
}

impl RepoLook {
    fn of(s: &Settings, path: &Path) -> Self {
        let own = s.repos.get(path);
        Self {
            name: own
                .and_then(|r| r.display_name.clone())
                .unwrap_or_else(|| crate::tab::folder_name(path)),
            icon: crate::tab::tab_icon(own.and_then(|r| r.icon.as_deref())),
        }
    }
}

/// The repository tab's name and icon.
fn repo_tab(ui: &mut Ui, s: &mut Settings, path: &Path) {
    heading(ui, "Tab");
    let folder = crate::tab::folder_name(path);
    let saved = s
        .repos
        .get(path)
        .and_then(|r| r.display_name.clone())
        .unwrap_or_default();
    let mut name = saved.clone();
    let about = format!(
        "What its tab calls this repository, up to {} characters.",
        crate::settings::MAX_TAB_NAME
    );
    row(ui, "Name", &about, |ui| {
        fixed_width(ui, FIELD_WIDTH, |ui| {
            TextField::new(&mut name)
                .placeholder(folder.clone())
                .width(FIELD_WIDTH)
                .name("Tab name")
                .id_salt("repo-tab-name")
                .show(ui)
        });
    });
    if name != saved {
        let name: String = name.chars().take(crate::settings::MAX_TAB_NAME).collect();
        s.repo_mut(path).display_name = (!name.trim().is_empty()).then_some(name);
    }
    let icons: Vec<String> = crate::tab::TAB_ICONS
        .iter()
        .map(|(_, glyph, label)| format!("{glyph}  {label}"))
        .collect();
    let options: Vec<(usize, &str)> = icons.iter().map(String::as_str).enumerate().collect();
    let key = s.repos.get(path).and_then(|r| r.icon.clone());
    let current = crate::tab::TAB_ICONS
        .iter()
        .position(|(k, _, _)| Some(*k) == key.as_deref())
        .unwrap_or(0);
    let mut chosen = current;
    row(ui, "Icon", "Shown in the tab, before its name.", |ui| {
        Select::new(&mut chosen, &options)
            .fit_to_options()
            .name("Tab icon")
            .id_salt("repo-tab-icon")
            .show(ui);
    });
    if chosen != current {
        s.repo_mut(path).icon = (chosen != 0)
            .then(|| {
                crate::tab::TAB_ICONS
                    .get(chosen)
                    .map(|(k, _, _)| (*k).to_owned())
            })
            .flatten();
    }
}

/// The repository's remotes, each URL editable (`git remote set-url`).
fn repo_remotes(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    path: &Path,
    facts: Option<&GitFacts>,
    actions: &mut Vec<Action>,
) {
    let Some(facts) = facts else {
        return;
    };
    if screen.remote_drafts_for.as_deref() != Some(path) {
        screen.remote_drafts_for = Some(path.to_path_buf());
        screen.remote_drafts.clear();
        screen.remote_saved = None;
    }
    // After a save, read the configuration again until it shows the new URL (or for 10 s, when
    // git refused it: its message is in a toast).
    if let Some((name, url, at)) = &screen.remote_saved {
        let landed = facts
            .remotes
            .iter()
            .any(|r| &r.name == name && &r.url == url);
        if landed || at.elapsed() > Duration::from_secs(10) {
            screen.remote_saved = None;
        } else if screen.facts.repo.idle_for(Duration::from_millis(300)) {
            screen.facts.repo.reread();
        }
        ui.ctx().request_repaint_after(Duration::from_millis(300));
    }
    heading(ui, "Remotes");
    if facts.remotes.is_empty() {
        if block(ui, "Remotes", "remote url origin") {
            help_label(
                ui,
                "This repository has no remotes. Add one from the sidebar or the command palette.",
            );
            row_end(ui);
        }
        return;
    }
    for remote in &facts.remotes {
        let draft = screen
            .remote_drafts
            .entry(remote.name.clone())
            .or_insert_with(|| remote.url.clone());
        let typed = draft.trim().to_owned();
        let problem = if typed.is_empty() {
            Some("Enter the remote's URL.")
        } else if canopy_core::ops::url_has_credentials(&typed) {
            Some("Remove the user name and password: git asks for them when it needs them.")
        } else {
            None
        };
        let changed = typed != remote.url;
        let help = format!("Where {} fetches from and pushes to.", remote.name);
        let mut save = false;
        row_layout(
            ui,
            &remote.name,
            None,
            "remote url origin upstream set-url",
            &|ui: &mut Ui| help_label(ui, &help),
            "",
            |ui| {
                // Right to left: Save, then the field, together as wide as the other fields.
                let right = ui.cursor().max.x;
                save = Button::new("Save")
                    .primary()
                    .enabled(changed && problem.is_none())
                    .disabled_reason(problem.unwrap_or("The URL is unchanged."))
                    .id_salt(("remote-save", &remote.name))
                    .show(ui)
                    .clicked();
                let width = (FIELD_WIDTH - (right - ui.cursor().max.x)).max(120.0);
                fixed_width(ui, width, |ui| {
                    TextField::new(draft)
                        .width(width)
                        .name(format!("URL of {}", remote.name))
                        .error(problem.filter(|_| changed))
                        .id_salt(("remote-url", &remote.name))
                        .show(ui)
                });
            },
        );
        row_end(ui);
        if save {
            actions.push(Action::Op(Op::SetRemoteUrl {
                name: remote.name.clone(),
                url: typed.clone(),
            }));
            screen.remote_saved = Some((remote.name.clone(), typed, Instant::now()));
        }
    }
}

/// The repository's name and where it is.
fn repo_header(ui: &mut Ui, path: &Path, look: &RepoLook) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let name = &look.name;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::MD;
        let (disc, _) = ui.allocate_exact_size(Vec2::splat(m.icon_lg * 2.0), Sense::hover());
        ui.painter()
            .circle_filled(disc.center(), disc.width() / 2.0, p.accent_tint());
        ui.painter().text(
            disc.center(),
            Align2::CENTER_CENTER,
            look.icon,
            FontId::proportional(m.icon_lg),
            p.accent,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = theme::XS;
            ui.label(
                RichText::new(name)
                    .font(theme::heading(m.subtitle_size))
                    .color(p.text),
            );
            let shown = path.display().to_string();
            let font = FontId::new(m.small_size, FontFamily::Monospace);
            let cut = elide_middle(ui, &shown, &font, ui.available_width() - theme::LG);
            let response = ui.label(RichText::new(&cut).font(font).color(p.text_dim));
            if cut != shown {
                response.kit_tooltip(shown).show();
            }
        });
    });
    ui.add_space(theme::SM);
    help_label(ui, "Settings here apply to this repository only.");
    ui.add_space(theme::LG);
}

/// Why git's configuration could not be read, in git's words, when it could not.
fn facts_error(ui: &mut Ui, facts: Option<&GitFacts>) {
    if let Some(e) = facts
        .and_then(|f| f.error.as_deref())
        .filter(|_| !Filter::searching(ui.ctx()))
    {
        Callout::new(
            Tone::Warning,
            format!("Could not read your git configuration: {e}"),
        )
        .show(ui);
        ui.add_space(theme::SM);
    }
}

/// A repository row: [`row`] with a note under its help.
fn repo_row(
    ui: &mut Ui,
    title: &str,
    help: &str,
    note: Option<&Note>,
    control: impl FnOnce(&mut Ui),
) {
    let help_text = |ui: &mut Ui| help_label(ui, help);
    noted_row_layout(ui, title, None, help, &help_text, "", note, control);
    row_end(ui);
}

fn on_off() -> [(bool, String); 2] {
    [(true, "On".to_owned()), (false, "Off".to_owned())]
}

/// A repository's effective choice, with one option per value. An explicit selection pins it;
/// an Overridden tag and a reset distinguish it from following the app's default.
/// Drawn right to left (the select rightmost).
fn override_select<T: PartialEq + Copy>(
    ui: &mut Ui,
    salt: &str,
    name: &str,
    value: &mut Option<T>,
    default: T,
    recommended: T,
    options: &[(T, String)],
) {
    let label = |v: T| {
        options
            .iter()
            .find(|(o, _)| *o == v)
            .map_or_else(String::new, |(_, l)| l.clone())
    };
    let refs: Vec<(T, &str)> = options.iter().map(|(v, l)| (*v, l.as_str())).collect();
    let mut shown = value.unwrap_or(default);
    if Select::new(&mut shown, &refs)
        .recommended(recommended)
        .report_reselection()
        .width(select_width(ui, refs.iter().map(|(_, l)| *l)))
        .name(name)
        .id_salt(salt)
        .show(ui)
        .changed()
    {
        *value = Some(shown);
    }
    if value.is_some() {
        if IconButton::new(
            icon::ARROW_COUNTER_CLOCKWISE,
            format!("Use the default for {name} ({})", label(default)),
        )
        .size(Size::Sm)
        .id_salt((salt, "reset"))
        .show(ui)
        .clicked()
        {
            *value = None;
        }
        Badge::new("Overridden")
            .tone(Tone::Accent)
            .tooltip(OVERRIDDEN_TIP)
            .show(ui);
    }
}

/// What the commit signing row states: whether commits get signed, with which key and program,
/// and what would make signing fail.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SigningLook {
    badge: &'static str,
    tone: Tone,
    help: String,
    /// A warning with the fix, when signing is on and would fail.
    note: Option<Note>,
}

/// The program as a person would name it: a configured path shows as its file name.
fn program_label(program: &str) -> String {
    Path::new(program)
        .file_name()
        .map_or_else(|| program.to_owned(), |n| n.to_string_lossy().into_owned())
}

fn signing_look(facts: &GitFacts) -> SigningLook {
    // Where signing is turned on or off, as a remark beside the title.
    let setting = facts.gpg_sign.as_ref().map(|v| Note {
        text: format!("{}.", upper_first(&config_phrase(v))),
        warn: false,
    });
    if !facts.signs_commits() {
        return SigningLook {
            badge: "Not signing",
            tone: Tone::Neutral,
            help: "Commits are not signed here. To sign, turn on Sign commits under Git configuration."
                .to_owned(),
            note: setting,
        };
    }
    let Some(signing) = &facts.signing else {
        return SigningLook {
            badge: "Signing",
            tone: Tone::Success,
            help: "Git signs commits here.".to_owned(),
            note: None,
        };
    };
    let program = program_label(&signing.program);
    let key = signing.key.as_deref().map_or_else(
        || {
            if signing.problem == Some(SigningProblem::KeyFileMissing) {
                "the configured key file".to_owned()
            } else {
                "git's default key for your committer identity".to_owned()
            }
        },
        |k| {
            if k == "an SSH key" {
                k.to_owned()
            } else {
                format!("key {k}")
            }
        },
    );
    let help = format!(
        "Commits are signed as {} with {key}, by {program}.",
        signing.format_label()
    );
    let problem = signing.problem.map(|problem| Note {
        text: match problem {
            SigningProblem::ProgramMissing => format!(
                "{program} is not installed or not on the PATH, so every commit will fail. Install it, or set another signing program in your git configuration."
            ),
            SigningProblem::KeyNotSet => "SSH signing needs a signing key and none is set, so every commit will fail. Set your public key file as the signing key in your git configuration.".to_owned(),
            SigningProblem::KeyFileMissing => {
                let file = signing.key.as_deref().filter(|key| *key != "an SSH key")
                    .unwrap_or("of the signing key");
                format!("The key file {file} does not exist, so every commit will fail. Correct the signing key in your git configuration.")
            },
            SigningProblem::UnknownFormat => format!(
                "\"{}\" is not a signing format git knows (OpenPGP, SSH or X.509), so every commit will fail. Choose one under Signing format.",
                signing.format
            ),
        },
        warn: true,
    });
    let failing = problem.is_some();
    let note = problem.or(setting);
    SigningLook {
        badge: if failing { "Will fail" } else { "Signing" },
        tone: if failing { Tone::Danger } else { Tone::Success },
        help,
        note,
    }
}

/// Whether git signs this repository's commits, with which key and program, and what would make
/// it fail: shown, not changed (overriding signing is the owner's call, GitHub issue #42), with a
/// button to open the file that sets it.
fn signing_row(ui: &mut Ui, facts: &GitFacts, actions: &mut Vec<Action>) {
    let look = signing_look(facts);
    let file = facts
        .gpg_sign
        .as_ref()
        .filter(|v| v.scope != "command")
        .and_then(|v| v.file.clone());
    row_with_note(ui, "Commit signing", &look.help, look.note.as_ref(), |ui| {
        // Right to left.
        Badge::new(look.badge).tone(look.tone).show(ui);
        if let Some(file) = file
            && Button::new("Open git configuration")
                .ghost()
                .size(Size::Sm)
                .tooltip(format!("Edit {}", file.display()))
                .show(ui)
                .clicked()
        {
            actions.push(Action::OpenInEditor(file));
        }
    });
}

/// Context lines and whitespace for this repository (the same options as its diff header).
fn repo_diff_rows(ui: &mut Ui, s: &Settings, path: &Path, actions: &mut Vec<Action>) {
    let own = s.repo_diff.get(path).copied();
    let mut chosen = own.unwrap_or(s.diff);
    let mut selected = false;
    repo_row(
        ui,
        "Context lines",
        "Unchanged lines shown around each change in this repository.",
        None,
        |ui| {
            let labels = (0..=25)
                .map(|n| (n, format!("{n} lines")))
                .collect::<Vec<_>>();
            let options = labels
                .iter()
                .map(|(n, label)| (*n, label.as_str()))
                .collect::<Vec<_>>();
            selected |= Select::new(&mut chosen.context, &options)
                .recommended(DEFAULTS.diff.context)
                .report_reselection()
                .fit_to_options()
                .name("Context lines")
                .id_salt("repo-context")
                .show(ui)
                .changed();
        },
    );
    repo_row(
        ui,
        "Whitespace",
        "Ignoring whitespace hides space-only changes but turns off hunk staging.",
        None,
        |ui| {
            let options = WhitespaceMode::ALL.map(|w| (w, w.label()));
            selected |= Select::new(&mut chosen.whitespace, &options)
                .recommended(DEFAULTS.diff.whitespace)
                .report_reselection()
                .width(select_width(ui, options.iter().map(|(_, label)| *label)))
                .name("Whitespace")
                .id_salt("repo-whitespace")
                .show(ui)
                .changed();
        },
    );
    let mut desired = if selected { Some(chosen) } else { own };
    if own.is_some() {
        repo_row(
            ui,
            "Diff defaults",
            "Returns context lines and whitespace to the values in Settings → Git.",
            None,
            |ui| {
                if IconButton::new(icon::ARROW_COUNTER_CLOCKWISE, "Use default diff options")
                    .size(Size::Sm)
                    .id_salt("repo-diff-reset")
                    .show(ui)
                    .clicked()
                {
                    desired = None;
                }
                Badge::new("Overridden")
                    .tone(Tone::Accent)
                    .tooltip(OVERRIDDEN_TIP)
                    .show(ui);
            },
        );
    }
    if desired != own {
        actions.push(Action::SetRepoDiff {
            path: path.to_path_buf(),
            prefs: desired,
        });
    }
}

/// What Settings can say about loading the repository's pull or merge requests right now.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostingStatus {
    /// Integrations → Pull and merge requests is off.
    Off,
    /// The host refused the CLI's sign-in on the newest read.
    Rejected,
    /// The host asked Canopy to wait; the timed refresh resumes when the limit lifts.
    RateLimited,
    /// Reads work with the CLI's sign-in, as this account when the CLI said.
    Working(Option<String>),
    /// The CLI gave no credential: public repositories load, private ones do not.
    NoCredential(CliError),
    /// The tab has not asked the host yet.
    Unknown,
}

/// The state of the tab's host, outcome first: an off switch beats a refusal, a refusal beats a
/// rate limit, and only then does the sign-in in use matter.
fn hosting_status(
    enabled: bool,
    cli: Option<&hosting::CliState>,
    rejected: bool,
    limited: bool,
) -> HostingStatus {
    if !enabled {
        HostingStatus::Off
    } else if rejected {
        HostingStatus::Rejected
    } else if limited {
        HostingStatus::RateLimited
    } else {
        match cli {
            Some(Ok(account)) => HostingStatus::Working(account.user.clone()),
            Some(Err(error)) => HostingStatus::NoCredential(error.clone()),
            None => HostingStatus::Unknown,
        }
    }
}

/// One block for the repository's requests: which host and remote, whether the API is on, which
/// credential serves it and whether reads work now, with the one action that fits the state.
fn hosting_account(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    s: &Settings,
    tab: &crate::tab::RepoTab,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let Some((remote, forge)) = tab.forge() else {
        let text = if tab.snap.is_some() {
            "No remote of this repository is on GitHub or GitLab, so there are no pull or merge requests to load."
        } else {
            "Reading the repository's remotes…"
        };
        help_label(ui, text);
        row_end(ui);
        return;
    };
    let host = forge.authority();
    let cli = format!("{} CLI", forge.kind.name());
    let plural = format!("{}s", forge.kind.request_noun());
    let sentence = {
        let mut chars = plural.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    };
    let limited = tab
        .requests
        .refresh
        .held_until
        .is_some_and(|until| Instant::now() < until);
    let status = hosting_status(
        s.hosting_api,
        tab.requests.cli.as_ref(),
        tab.requests.token_rejected,
        limited,
    );
    let (text, color, action) = match status {
        HostingStatus::Off => (
            format!("{sentence} are off: Canopy does not contact {host}."),
            p.text_faint,
            Some(("Open Integrations", Section::Integrations)),
        ),
        HostingStatus::Rejected => (
            format!("{host} refused the {cli}'s sign-in, so {plural} cannot load."),
            p.danger,
            Some(("Open Accounts", Section::Accounts)),
        ),
        HostingStatus::RateLimited => (
            format!("{host} is limiting requests; {plural} refresh again when the limit lifts."),
            p.warning,
            Some(("Open Accounts", Section::Accounts)),
        ),
        HostingStatus::Working(user) => (
            format!(
                "{sentence} load from {host} using the {cli}'s sign-in{}.",
                user.map_or_else(String::new, |user| format!(" as {user}"))
            ),
            p.success,
            Some(("Open Accounts", Section::Accounts)),
        ),
        HostingStatus::NoCredential(error) => {
            let (status, _) = account_status(forge.kind, &Err(error.clone()));
            let why = match &error {
                CliError::Failed { message, .. } => {
                    format!("{status}: {}", message.trim_end_matches('.'))
                }
                _ => status,
            };
            (
                format!("{why}. Public repositories load; private ones do not."),
                p.text_faint,
                Some(("Open Accounts", Section::Accounts)),
            )
        }
        HostingStatus::Unknown => (
            format!("{sentence} have not been requested from {host} yet."),
            p.text_faint,
            Some(("Open Accounts", Section::Accounts)),
        ),
    };
    let others: Vec<String> = tab
        .forges
        .iter()
        .filter(|(name, _)| name != remote)
        .map(|(name, other)| format!("{name} ({})", other.authority()))
        .collect();
    row_layout(
        ui,
        "Pull and merge requests",
        None,
        "",
        &|ui: &mut Ui| {
            ui.label(RichText::new(&text).size(m.meta_size).color(color));
            help_label(ui, &format!("{} remote {remote}.", forge.kind.name()));
            if !others.is_empty() {
                help_label(
                    ui,
                    &format!(
                        "Other GitHub or GitLab remotes, not loaded: {}.",
                        others.join(", ")
                    ),
                );
            }
        },
        "",
        |ui| {
            if let Some((label, section)) = action
                && Button::new(label)
                    .icon(forge_icon(forge.kind))
                    .id_salt("repo-open-accounts")
                    .show(ui)
                    .clicked()
            {
                screen.section = section;
            }
        },
    );
    row_end(ui);
}

/// This repository's identity: what git uses now and where it comes from, and the repository's
/// own name and email, which win over the global ones while they are set.
fn repo_identity(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    path: &Path,
    view: &IdentityView<'_>,
    actions: &mut Vec<Action>,
) {
    let identity = view.identity;
    let local = |field: &Field| matches!(&field.source, Some(Source::Config { scope, .. }) if scope == "local" || scope == "worktree");
    if screen.local_identity_for.as_deref() != Some(path)
        || screen.local_loaded.as_ref() != identity
    {
        screen.local_identity_for = Some(path.to_path_buf());
        screen.local_loaded = identity.cloned();
        let own = |field: &Field| {
            if local(field) {
                field.value.clone()
            } else {
                String::new()
            }
        };
        screen.local_name = identity.map_or_else(String::new, |i| own(&i.author.name));
        screen.local_email = identity.map_or_else(String::new, |i| own(&i.author.email));
    }
    heading(ui, "Identity in this repository");
    if !Filter::searching(ui.ctx()) {
        identity_status(ui, identity, view.error);
    }
    let has_own = identity.is_some_and(|i| local(&i.author.name) || local(&i.author.email));
    let current = |pick: fn(&Identity) -> &Field| identity.map(pick).and_then(in_use);
    // What the repository inherits shows in an empty field, so the field is never blank.
    let inherited = |pick: fn(&Identity) -> &Field| {
        identity
            .map(pick)
            .filter(|f| !local(f) && !f.value.is_empty())
            .map_or_else(
                || "Not set".to_owned(),
                |f| format!("{} (inherited)", f.value),
            )
    };
    let name_tag = identity
        .filter(|i| local(&i.author.name))
        .map(|_| "Overridden");
    noted_row_layout(
        ui,
        "Name",
        name_tag,
        "identity user.name author",
        &|ui: &mut Ui| help_label(ui, NAME_HELP),
        "",
        current(|i| &i.author.name).as_ref(),
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut screen.local_name)
                    .placeholder(inherited(|i| &i.author.name))
                    .width(FIELD_WIDTH)
                    .name("Name in this repository")
                    .id_salt("repo-identity-name")
                    .show(ui)
            });
        },
    );
    row_end(ui);
    let email_tag = identity
        .filter(|i| local(&i.author.email))
        .map(|_| "Overridden");
    noted_row_layout(
        ui,
        "Email",
        email_tag,
        "identity user.email author",
        &|ui: &mut Ui| help_label(ui, EMAIL_HELP),
        "",
        current(|i| &i.author.email).as_ref(),
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut screen.local_email)
                    .placeholder(inherited(|i| &i.author.email))
                    .width(FIELD_WIDTH)
                    .name("Email in this repository")
                    .id_salt("repo-identity-email")
                    .show(ui)
            });
        },
    );
    row_end(ui);
    let can_save = !screen.local_name.trim().is_empty() && !screen.local_email.trim().is_empty();
    row(
        ui,
        "Apply identity",
        "Saves the name and email for this repository only.",
        |ui| {
            if Button::new("Save to this repository")
                .primary()
                .enabled(can_save)
                .disabled_reason("Enter both a name and an email before saving.")
                .id_salt("repo-identity-save")
                .show(ui)
                .clicked()
            {
                actions.push(Action::SaveIdentity {
                    repo: Some(path.to_path_buf()),
                    scope: Scope::Local,
                    name: screen.local_name.clone(),
                    email: screen.local_email.clone(),
                });
            }
            if has_own
                && Button::new("Use global identity…")
                    .icon(icon::ARROW_COUNTER_CLOCKWISE)
                    .tooltip("Remove this repository's own name and email (asks first).")
                    .id_salt("repo-identity-clear")
                    .show(ui)
                    .clicked()
            {
                actions.push(Action::OpenDialog(Dialog::ConfirmClearIdentity {
                    repo: Some(path.to_path_buf()),
                    scope: Scope::Local,
                }));
            }
        },
    );
}

/// The inactivity period for manual cleanup, entered as a whole number of days.
fn stale_days(ui: &mut Ui, s: &mut Settings) {
    let mut days = f64::from(s.stale_inactivity_days);
    if fixed_width(ui, COUNT_FIELD_WIDTH, |ui| {
        KitNumber::new(&mut days, STALE_DAYS)
            .step(1.0)
            .name("Stale cleanup after (days)")
            .width(COUNT_FIELD_WIDTH)
            .id_salt("stale-inactivity-days")
            .show(ui)
    })
    .changed()
    {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the number field clamps to a positive whole-day range within u32"
        )]
        let days = days.round() as u32;
        s.stale_inactivity_days = days;
    }
}

/// Commits per page: the kit's number field, with − and + steppers of [`COMMIT_PAGE_STEP`].
fn commit_page(ui: &mut Ui, s: &mut Settings) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a page size of at most 50 000, exact in f64"
    )]
    let mut page = s.commit_page as f64;
    let changed = fixed_width(ui, COUNT_FIELD_WIDTH, |ui| {
        KitNumber::new(&mut page, COMMIT_PAGE)
            .step(COMMIT_PAGE_STEP)
            .name("Commits per page")
            .step_names("Fewer commits per page", "More commits per page")
            .width(COUNT_FIELD_WIDTH)
            .id_salt("commit-page")
            .show(ui)
    })
    .changed();
    if changed {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the field clamps the value to COMMIT_PAGE, so it is a whole positive count once rounded"
        )]
        let page = page.round() as usize;
        s.commit_page = page;
    }
}

fn integrations(
    ui: &mut Ui,
    s: &mut Settings,
    picker: &mut crate::ui::folder_picker::FolderPicker,
    custom: &mut HashSet<Kind>,
    clone_parent: &str,
) {
    heading(ui, "Cloning");
    row(ui, "Default clone folder", "Where new clones go.", |ui| {
        let right = ui.cursor().max.x;
        if let Some(dir) = picker.show(
            ui,
            "default-clone-folder",
            &crate::settings::expand_home(&s.clone_dir),
        ) {
            s.clone_dir = dir.display().to_string();
        }
        // The button and the field together are as wide as the other fields, so every
        // field starts at the same edge.
        let button = right - ui.cursor().max.x;
        let default = clone_parent.to_owned();
        let width = (FIELD_WIDTH - button).max(80.0);
        fixed_width(ui, width, |ui| {
            TextField::new(&mut s.clone_dir)
                .placeholder(default)
                .width(width)
                .name("Default clone folder")
                .id_salt("clone-dir")
                .show(ui)
        });
    });
    default_row(
        ui,
        "Clone protocol",
        "For the URLs Canopy builds. SSH needs a key on your account.",
        None,
        |ui| {
            revert(
                ui,
                &mut s.clone_protocol,
                &DEFAULTS.clone_protocol,
                "Clone protocol",
            );
            Segmented::new(
                &mut s.clone_protocol,
                &[(CloneProtocol::Https, "HTTPS"), (CloneProtocol::Ssh, "SSH")],
            )
            .label("Clone protocol")
            .show(ui);
        },
    );
    heading(ui, "GitHub and GitLab");
    row(
        ui,
        "Self-hosted GitLab",
        "Other hosts to treat as GitLab, separated by commas.",
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut s.gitlab_hosts)
                    .placeholder("git.example.com, code.corp.io")
                    .width(FIELD_WIDTH)
                    .name("Self-hosted GitLab")
                    .id_salt("gitlab-hosts")
                    .show(ui)
            });
        },
    );
    row_explained(
        ui,
        "Pull and merge requests",
        "Loads pull and merge requests through the GitHub and GitLab APIs. Off keeps Canopy offline apart from git.",
        "Requests load when you open a repository hosted on GitHub or GitLab, using the sign-in of the gh or glab CLI.",
        |ui| {
            revert(
                ui,
                &mut s.hosting_api,
                &DEFAULTS.hosting_api,
                "Pull and merge requests",
            );
            switch(ui, &mut s.hosting_api, "Pull and merge requests");
        },
    );
    heading(ui, "Tools");
    let apps = crate::system_apps::get();
    let apps = apps.as_deref();
    launcher_row(
        ui,
        Launcher {
            title: "Terminal",
            help: "What \"Open terminal here\" runs, in the repository folder.",
            command_help: "Runs in the repository folder; %d, as a word of its own, stands for the folder.",
            kind: Kind::Terminal,
        },
        &mut s.terminal,
        apps,
        custom,
    );
    launcher_row(
        ui,
        Launcher {
            title: "Editor",
            help: "What \"Open in editor\" opens files with.",
            command_help: "%f, as a word of its own, stands for the file; without it the file is added at the end. Quote a path with spaces.",
            kind: Kind::Editor,
        },
        &mut s.editor,
        apps,
        custom,
    );
    launcher_row(
        ui,
        Launcher {
            title: "File manager",
            help: "What \"Show in file manager\" opens folders with.",
            command_help: "%d, as a word of its own, stands for the folder; without it the folder is added at the end.",
            kind: Kind::FileManager,
        },
        &mut s.file_manager,
        apps,
        custom,
    );
}

/// The words of one editor, terminal or file manager setting.
#[derive(Clone, Copy)]
struct Launcher {
    title: &'static str,
    help: &'static str,
    command_help: &'static str,
    kind: Kind,
}

/// What a launcher setting holds, as its select shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchChoice {
    System,
    App(usize),
    /// A well-known program that is not installed: shown when it is the saved choice, never
    /// chosen from the list.
    Missing(usize),
    Custom,
}

/// What a saved command shows as: the system's when empty, an installed app whose command it
/// is, a well-known program that is no longer installed (`absent`: the command names a program
/// file that is gone, [`crate::system_apps::Apps::is_absent`]), else a custom command (also while
/// the user has the command field open).
fn launch_choice(
    value: &str,
    editing: bool,
    installed: &[&crate::system_apps::App],
    missing: &[&crate::system_apps::Missing],
    absent: bool,
) -> LaunchChoice {
    let value = value.trim();
    if editing {
        LaunchChoice::Custom
    } else if value.is_empty() {
        LaunchChoice::System
    } else {
        installed
            .iter()
            .position(|a| a.command == value)
            .map(LaunchChoice::App)
            .or_else(|| {
                missing
                    .iter()
                    .position(|m| m.is_started_by(value, absent))
                    .map(LaunchChoice::Missing)
            })
            .unwrap_or(LaunchChoice::Custom)
    }
}

/// Whether `app` is what the system would open: it is listed as the default, not again by name.
fn is_system_choice(
    system: Option<&crate::system_apps::SystemChoice>,
    app: &crate::system_apps::App,
) -> bool {
    system.is_some_and(|s| s.is(app))
}

/// The entries of a launcher's list: the system's own program as the Default (named once, so not
/// again among the installed ones), the other installed programs, the well-known ones that are
/// not installed, then a command of the user's.
fn launcher_options(
    system: Option<&crate::system_apps::SystemChoice>,
    installed: &[&crate::system_apps::App],
    missing: &[&crate::system_apps::Missing],
) -> Vec<(LaunchChoice, String)> {
    let mut options = vec![(
        LaunchChoice::System,
        system.map_or_else(
            || "Default".to_owned(),
            |choice| format!("{} (Default)", choice.name),
        ),
    )];
    options.extend(
        installed
            .iter()
            .enumerate()
            .filter(|(_, a)| !is_system_choice(system, a))
            .map(|(i, a)| (LaunchChoice::App(i), a.name.clone())),
    );
    options.extend(
        missing
            .iter()
            .enumerate()
            .map(|(i, m)| (LaunchChoice::Missing(i), m.name.clone())),
    );
    options.push((LaunchChoice::Custom, "Custom command…".to_owned()));
    options
}

/// An editor, terminal or file manager: the system's (named), one of the installed ones, or a
/// command of the user's in a field that opens below.
fn launcher_row(
    ui: &mut Ui,
    launcher: Launcher,
    value: &mut String,
    apps: Option<&crate::system_apps::Apps>,
    custom: &mut HashSet<Kind>,
) {
    let Launcher {
        title,
        help,
        command_help,
        kind,
    } = launcher;
    let system = apps.and_then(|a| a.system(kind));
    let installed: Vec<&crate::system_apps::App> = apps
        .map(|a| a.installed(kind).collect())
        .unwrap_or_default();
    let missing: Vec<&crate::system_apps::Missing> =
        apps.map(|a| a.missing(kind).collect()).unwrap_or_default();
    let absent = apps.is_some_and(|a| a.is_absent(value));
    let current = match launch_choice(value, custom.contains(&kind), &installed, &missing, absent) {
        LaunchChoice::App(i)
            if installed
                .get(i)
                .is_some_and(|a| is_system_choice(system, a)) =>
        {
            LaunchChoice::System
        }
        other => other,
    };
    let labels = launcher_options(system, &installed, &missing);
    let options: Vec<(LaunchChoice, &str)> = labels.iter().map(|(c, l)| (*c, l.as_str())).collect();
    let mut chosen = current;
    row_more(ui, title, help, "", |ui| {
        let mut select = Select::new(&mut chosen, &options)
            .described(LaunchChoice::System, "Default")
            .width(select_width(ui, options.iter().map(|(_, l)| *l)))
            .name(title)
            .id_salt(("launcher", title))
            // Custom command… stays in view below a long list of programs (#280).
            .pin_last();
        for i in 0..missing.len() {
            select = select.unavailable(LaunchChoice::Missing(i), "Not installed");
        }
        select.show(ui);
    });
    row_end(ui);
    if chosen != current {
        match chosen {
            LaunchChoice::System => {
                value.clear();
                custom.remove(&kind);
            }
            LaunchChoice::App(i) => {
                if let Some(app) = installed.get(i) {
                    value.clone_from(&app.command);
                }
                custom.remove(&kind);
            }
            // A program that is not installed cannot be chosen (the list passes over it).
            LaunchChoice::Missing(_) => {}
            // The field starts from what was chosen, to adjust.
            LaunchChoice::Custom => {
                if value.trim().is_empty()
                    && let Some(cmd) = system.and_then(|c| c.command.as_ref())
                {
                    value.clone_from(cmd);
                }
                custom.insert(kind);
            }
        }
    }
    if chosen == LaunchChoice::Custom {
        let name = format!("{title} command");
        row(ui, &name, command_help, |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(value)
                    .placeholder(match kind {
                        Kind::Editor => "code --wait %f",
                        Kind::Terminal => "wezterm start --cwd %d",
                        Kind::FileManager => "nautilus %d",
                    })
                    .width(FIELD_WIDTH)
                    .name(name.clone())
                    .id_salt(("launcher-command", title))
                    .show(ui)
            });
        });
    }
}

/// A host's check of the CLI sign-in: under way, or what it found.
#[derive(Debug)]
enum CliCheck {
    Running(Receiver<hosting::CliState>),
    Done(hosting::CliState),
}

/// The program that signs in to `kind`'s hosts.
const fn cli_program(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "gh",
        ForgeKind::GitLab => "glab",
    }
}

/// What a host's card says about the CLI's sign-in, and the tone of the line.
pub(crate) fn account_status(kind: ForgeKind, state: &hosting::CliState) -> (String, Tone) {
    let cli = format!("{} CLI", kind.name());
    match state {
        Ok(account) => (
            format!(
                "Using the {cli}'s sign-in{}",
                account
                    .user
                    .as_ref()
                    .map_or_else(String::new, |user| format!(" as {user}"))
            ),
            Tone::Success,
        ),
        Err(CliError::NotInstalled { .. }) => {
            (format!("The {cli} is not installed"), Tone::Neutral)
        }
        Err(CliError::SignedOut { .. }) => (format!("Not signed in with the {cli}"), Tone::Warning),
        Err(CliError::MissingScope { scope, user, .. }) => (
            format!(
                "Signed in{} with the {cli}, without the {scope} scope",
                user.as_ref()
                    .map_or_else(String::new, |user| format!(" as {user}"))
            ),
            Tone::Warning,
        ),
        Err(CliError::Failed { .. }) => (format!("The {cli} could not be used"), Tone::Danger),
    }
}

/// The line under the status of a signed-in card: the scopes the sign-in has, when the CLI lists
/// them.
fn scopes_line(account: &canopy_hosting::CliAccount) -> Option<String> {
    account
        .scopes
        .as_ref()
        .map(|scopes| format!("Scopes: {}", scopes.join(", ")))
}

/// A command to run in a terminal, on a line of its own with a button that copies it.
fn command_line(ui: &mut Ui, command: &str, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::Label::new(
                RichText::new(command)
                    .monospace()
                    .size(m.mono_size)
                    .color(p.text),
            )
            .selectable(true),
        );
        if IconButton::new(icon::COPY, "Copy the command")
            .size(Size::Xs)
            .id_salt(("account-copy", command))
            .show(ui)
            .clicked()
        {
            actions.push(Action::Copy(command.to_owned()));
        }
    });
}

/// What a host's card shows under its status line for each state, and what the user can do.
fn account_actions(
    ui: &mut Ui,
    kind: ForgeKind,
    state: &hosting::CliState,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let detail = |ui: &mut Ui, text: &str| {
        ui.add(egui::Label::new(RichText::new(text).size(m.small_size).color(p.text_faint)).wrap());
    };
    match state {
        Ok(account) => {
            if let Some(line) = scopes_line(account) {
                detail(ui, &line);
            }
        }
        Err(CliError::NotInstalled { .. }) => {
            detail(
                ui,
                &format!(
                    "Install it, sign in with it ({} auth login), then check again.",
                    cli_program(kind)
                ),
            );
            let page = canopy_hosting::install_page(kind);
            if LinkButton::new(format!("Install {} CLI", kind.name()))
                .url(page)
                .text_size(m.small_size)
                .show(ui)
                .clicked()
            {
                actions.push(Action::OpenUrl(page.to_owned()));
            }
        }
        Err(error @ (CliError::SignedOut { .. } | CliError::MissingScope { .. })) => {
            detail(
                ui,
                &error.fix_note().map_or_else(
                    || "Run this in a terminal, then check again.".to_owned(),
                    |note| format!("Run this in a terminal and {note}, then check again."),
                ),
            );
            if let Some(command) = error.fix_command() {
                command_line(ui, &command, actions);
            }
        }
        Err(CliError::Failed { message, .. }) => detail(ui, message),
    }
}

/// The most sign-in checks the Accounts page runs at once; the other hosts wait their turn.
const MAX_CHECKS: usize = 4;
/// The most cards for hosts that only open repositories' remotes name; the rest are counted.
const MAX_REPO_HOSTS: usize = 4;

/// The hosts the Accounts page shows: github.com, gitlab.com and the GitLab hosts listed in
/// Integrations always, then those of open repositories up to [`MAX_REPO_HOSTS`]. A host on a
/// non-default port is its own entry.
#[derive(Debug, Default)]
struct AccountHosts {
    hosts: Vec<Forge>,
    /// Hosts of open repositories left out.
    more: usize,
    /// What a settings search finds the page by.
    search: String,
}

impl AccountHosts {
    fn of(settings: &Settings, tabs: &[crate::tab::RepoTab]) -> Self {
        let account = |kind, host: &str| Forge {
            kind,
            host: host.to_lowercase(),
            port: None,
            path: String::new(),
        };
        let mut hosts: Vec<Forge> = vec![
            account(ForgeKind::GitHub, "github.com"),
            account(ForgeKind::GitLab, "gitlab.com"),
        ];
        for (host, port) in settings
            .gitlab_host_list()
            .iter()
            .filter_map(|h| canopy_core::forge::normalise_host_with_port(h))
        {
            hosts.push(Forge {
                kind: ForgeKind::GitLab,
                host,
                port,
                path: String::new(),
            });
        }
        let mut seen = HashSet::new();
        hosts.retain(|f| seen.insert(f.authority().to_lowercase()));
        let listed = hosts.len();
        let mut more = 0;
        for f in tabs.iter().flat_map(|t| t.forges.iter().map(|(_, f)| f)) {
            if !seen.insert(f.authority().to_lowercase()) {
                continue;
            }
            if hosts.len() - listed < MAX_REPO_HOSTS {
                hosts.push(Forge {
                    path: String::new(),
                    ..f.clone()
                });
            } else {
                more += 1;
            }
        }
        let search = hosts
            .iter()
            .map(|f| f.host.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            hosts,
            more,
            search,
        }
    }

    /// What the list is made from, so it is built again only when that changes.
    fn source(settings: &Settings, tabs: &[crate::tab::RepoTab]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        settings.gitlab_hosts.hash(&mut h);
        for (_, f) in tabs.iter().flat_map(|t| t.forges.iter()) {
            (f.kind, &f.host, f.port).hash(&mut h);
        }
        h.finish()
    }
}

/// The hosts whose check starts now: those not checked yet, in page order, while fewer than
/// [`MAX_CHECKS`] run process-wide, including checks started by a Settings screen since closed.
fn checks_to_start<'a>(
    hosts: &'a [Forge],
    checks: &HashMap<String, CliCheck>,
    running: usize,
) -> Vec<&'a Forge> {
    hosts
        .iter()
        .filter(|f| !checks.contains_key(&f.authority().to_lowercase()))
        .take(MAX_CHECKS.saturating_sub(running))
        .collect()
}

/// The hosts' sign-in under Accounts. Returns the hosts (`host[:port]`, lower case) whose sign-in
/// is not what an open tab on that host started with, so the app can restart those tabs' hosting
/// threads.
fn accounts(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    settings: &Settings,
    tabs: &[crate::tab::RepoTab],
    actions: &mut Vec<Action>,
) -> Vec<String> {
    const ABOUT: [&str; 2] = [
        "Pull and merge requests use the sign-in of the GitHub CLI (gh) and the GitLab CLI (glab).",
        "Canopy stores no token. Public repositories load without signing in.",
    ];
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let mut changed = Vec::new();
    let source = AccountHosts::source(settings, tabs);
    if screen
        .account_hosts
        .as_ref()
        .is_none_or(|(s, _)| *s != source)
    {
        screen.account_hosts = Some((source, Arc::new(AccountHosts::of(settings, tabs))));
    }
    let Some(page) = screen.account_hosts.as_ref().map(|(_, h)| Arc::clone(h)) else {
        return changed;
    };
    // The page is one block: a search for any host or for the CLIs shows all of it.
    if !block(
        ui,
        "Sign-in",
        &format!("{} {}", ABOUT.join(" "), page.search),
    ) {
        return changed;
    }
    for line in ABOUT {
        help_label(ui, line);
    }
    // The CLI is run (and contacts its host) only while pull and merge requests are on and the
    // start-up questions are answered, as for a tab's own hosting thread.
    let live = settings.hosting_api && crate::ui::legal::pending(settings).is_none();
    if !settings.hosting_api {
        ui.add_space(theme::SM);
        Callout::new(
            Tone::Neutral,
            "Pull and merge requests are turned off in Integrations, so Canopy does not run the CLIs.",
        )
        .icon(icon::INFO)
        .show(ui);
    }
    ui.add_space(theme::MD);

    // Collect every finished check first, including one whose host left the page, so a freed slot
    // is taken in this frame and an old Settings screen cannot strand a running check.
    let finished: Vec<(String, Option<hosting::CliState>)> = screen
        .checks
        .iter()
        .filter_map(|(key, check)| {
            let CliCheck::Running(rx) = check else {
                return None;
            };
            match rx.try_recv() {
                Ok(state) => Some((key.clone(), Some(state))),
                Err(crossbeam_channel::TryRecvError::Empty) => None,
                Err(crossbeam_channel::TryRecvError::Disconnected) => Some((key.clone(), None)),
            }
        })
        .collect();
    for (key, state) in finished {
        let state = if let Some(state) = state {
            state
        } else {
            let Some(forge) = page
                .hosts
                .iter()
                .find(|forge| forge.authority().eq_ignore_ascii_case(&key))
            else {
                screen.checks.remove(&key);
                screen.rechecks.remove(&key);
                continue;
            };
            Err(CliError::Failed {
                kind: forge.kind,
                message: "the check could not be run".into(),
            })
        };
        // A tab that started with another answer holds a stale credential, even when its host is
        // no longer one of the few repository hosts the page has room to show. An explicit recheck
        // also replaces a renewed token, which the account-and-scopes state cannot distinguish.
        let rechecked = screen.rechecks.remove(&key);
        let stale = tabs.iter().any(|t| {
            t.host
                .as_ref()
                .is_some_and(|h| h.forge.authority().to_lowercase() == key)
                && (rechecked
                    || t.requests.token_rejected
                    || t.requests.cli.as_ref().is_some_and(|had| *had != state))
        });
        if stale {
            changed.push(key.clone());
        }
        screen.checks.insert(key, CliCheck::Done(state));
    }
    // Ask the CLI when the page first shows a host, and again on "Check again".
    if live {
        for forge in checks_to_start(&page.hosts, &screen.checks, hosting::cli_checks_running()) {
            let rx = hosting::check_cli(forge.clone(), ui.ctx().clone());
            screen
                .checks
                .insert(forge.authority().to_lowercase(), CliCheck::Running(rx));
        }
    }

    let width = ui.available_width() - theme::SM;
    for forge in &page.hosts {
        let kind = forge.kind;
        let host = forge.authority();
        let key = host.to_lowercase();
        Card::new()
            .width(width)
            .id_salt(("account-card", &host))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = theme::SM;
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(forge_icon(kind))
                            .size(m.icon_lg)
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new(&host)
                            .font(theme::heading(m.subtitle_size))
                            .color(p.text),
                    );
                });
                match screen.checks.get(&key) {
                    // Waiting for a free slot reads the same as running: the check is on its way.
                    Some(CliCheck::Running(_)) | None
                        if live || screen.checks.contains_key(&key) =>
                    {
                        ui.horizontal(|ui| {
                            Spinner::new()
                                .size(Size::Sm)
                                .label("the sign-in check")
                                .show(ui);
                            ui.label(
                                RichText::new(format!("Checking the {} CLI…", kind.name()))
                                    .size(m.meta_size)
                                    .color(p.text_dim),
                            );
                        });
                    }
                    Some(CliCheck::Done(state)) => {
                        let (status, tone) = account_status(kind, state);
                        let glyph = match tone {
                            Tone::Success => icon::CHECK_CIRCLE,
                            Tone::Danger => icon::WARNING_OCTAGON,
                            Tone::Warning => icon::WARNING,
                            _ => icon::KEY,
                        };
                        ui.label(
                            RichText::new(format!("{glyph}  {status}"))
                                .size(m.meta_size)
                                .color(tone.color(p)),
                        );
                        account_actions(ui, kind, state, actions);
                    }
                    _ => {
                        ui.label(
                            RichText::new("Not checked while pull and merge requests are off.")
                                .size(m.meta_size)
                                .color(p.text_faint),
                        );
                    }
                }
                let busy = !matches!(screen.checks.get(&key), Some(CliCheck::Done(_)));
                if live
                    && Button::new("Check again")
                        .icon(icon::ARROW_CLOCKWISE)
                        .enabled(!busy)
                        .disabled_reason("Checking the sign-in now.")
                        .id_salt(("account-recheck", &host))
                        .show(ui)
                        .clicked()
                {
                    // Queued: it starts on the next frame, when a slot is free. Its completion
                    // restarts tabs even if the account and scopes are unchanged (token renewal).
                    screen.rechecks.insert(key.clone());
                    screen.checks.remove(&key);
                    ui.ctx().request_repaint();
                }
            });
        ui.add_space(m.card_gap);
    }
    if page.more > 0 {
        help_label(
            ui,
            &format!("And {} more hosts of open repositories.", page.more),
        );
    }
    changed
}

/// What the user agreed to: the Terms and the Privacy Policy accepted, and the crash report
/// answer, with the way to change it. Reading a document here accepts nothing.
fn consent(ui: &mut Ui, s: &Settings, actions: &mut Vec<Action>) {
    use crate::ui::legal::{Document, PRIVACY_VERSION, TERMS_VERSION};
    let p = palette(ui.ctx());
    heading(ui, "Consent");
    let status = |accepted: &str, current: &str, document: Document| {
        if accepted == current {
            format!(
                "You accepted this version (last updated {}).",
                document.updated()
            )
        } else if accepted.is_empty() {
            "Not accepted yet. Canopy asks before it contacts any server.".to_owned()
        } else {
            format!(
                "You accepted the version of {accepted}; it was updated on {}. Canopy asks you to accept the new one when it starts.",
                document.updated()
            )
        }
    };
    let documents = [
        (
            Document::Terms,
            "Terms of Service",
            "",
            status(&s.accepted_terms, TERMS_VERSION, Document::Terms),
            icon::SCROLL,
            "read-terms",
        ),
        (
            Document::Privacy,
            "Privacy Policy",
            "What Canopy collects and your rights.",
            status(&s.accepted_privacy, PRIVACY_VERSION, Document::Privacy),
            icon::SHIELD_CHECK,
            "read-privacy",
        ),
    ];
    for (document, title, about, state, glyph, salt) in documents {
        row(ui, title, format!("{about} {state}").trim_start(), |ui| {
            if Button::new("Read")
                .icon(glyph)
                .id_salt(salt)
                .show(ui)
                .clicked()
            {
                actions.push(Action::OpenDialog(Dialog::Legal(document)));
            }
        });
    }
    if crate::crash_reports::available() {
        let before = s.crash_reports;
        let mut on = s.crash_reports == Some(true);
        row_more(
            ui,
            "Send crash reports",
            "When Canopy crashes, send the error and a stack trace to NecturaLabs.",
            "A report also holds the Canopy version and your operating system. Stored by Sentry in the EU and deleted after 30 days. Takes effect immediately.",
            |ui| {
                switch(ui, &mut on, "Send crash reports");
            },
        );
        row_end(ui);
        if before != Some(on) && (before.is_some() || on) {
            actions.push(Action::SetCrashReports(on));
        }
    } else if block(
        ui,
        "Send crash reports",
        "This build of Canopy cannot send crash reports.",
    ) {
        ui.label(
            RichText::new("This build of Canopy cannot send crash reports.").color(p.text_dim),
        );
    }
}

/// What the user did with the trust-off notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrustNotice {
    TurnOn,
    Dismiss,
}

/// The notice shown while trust safety is off, and which of its buttons was pressed. Bordered
/// buttons, not the callout's ghost actions, which show no edge until hovered: this notice must be
/// answerable at a glance, with the kit button's focus ring and keys.
fn trust_off_notice(ui: &mut Ui) -> Option<TrustNotice> {
    let mut pressed = None;
    Callout::new(
        Tone::Danger,
        "Canopy no longer asks before it changes or runs anything in a repository. Any repository can run programs on this computer through its hooks and settings.",
    )
    .title("Trust safety is off")
    .icon(icon::SHIELD_WARNING)
    .id_salt("trust-safety-off")
    .show_with(ui, |ui| {
        ui.horizontal(|ui| {
            if Button::new("Turn on")
                .size(Size::Sm)
                .icon(icon::SHIELD_CHECK)
                .id_salt("trust-safety-on")
                .show(ui)
                .clicked()
            {
                pressed = Some(TrustNotice::TurnOn);
            }
            if Button::new("Dismiss")
                .size(Size::Sm)
                .icon(icon::X)
                .id_salt("trust-safety-dismiss")
                .show(ui)
                .clicked()
            {
                pressed = Some(TrustNotice::Dismiss);
            }
        });
    });
    ui.add_space(theme::MD);
    pressed
}

/// Returns true when "Open Accounts" was clicked.
fn privacy(
    ui: &mut Ui,
    s: &mut Settings,
    actions: &mut Vec<Action>,
    picker: &mut crate::ui::folder_picker::FolderPicker,
    pictures: Option<&PictureNote>,
    clone_parent: &str,
) -> bool {
    consent(ui, s, actions);
    heading(ui, "Trust");
    row_more(
        ui,
        "Trusted folders",
        "A repository inside one of these folders is trusted, and so is everything below each folder.",
        "A repository's hooks and settings can run programs on this computer, and git follows them when it downloads a partial clone's missing files. Canopy opens a repository outside these folders read-only and asks before trusting it. Trusting a folder trusts every repository inside it, linked worktrees and ones you clone later included, so the list keeps only the outer folder. Trust a folder only if everything in it comes from you or people you trust.",
        |ui| {
            if let Some(dir) = picker.show_labelled(
                ui,
                "trusted-folder",
                &crate::settings::expand_home(clone_parent),
                "Add folder…",
            ) && crate::trust::add(&mut s.trusted_folders, &dir)
            {
                crate::trust::publish(s);
                ui.ctx().request_repaint();
            }
        },
    );
    if last_row_shown(ui) {
        trusted_folder_list(ui, s);
    }
    row_end(ui);
    let before = s.trust_safety;
    let mut safe = before;
    row_explained(
        ui,
        "Trust safety",
        "Asks before a repository can change anything or run programs. Off, every repository is trusted.",
        "With it on, a repository outside the trusted folders opens read-only: Canopy shows its history and diffs but stages, commits, fetches and downloads nothing, because those run git with the repository's own hooks and settings. Turning it off asks first, applies to every open repository at once, and keeps your trusted folders for when you turn it back on.",
        |ui| {
            switch(ui, &mut safe, "Trust safety");
        },
    );
    if safe != before {
        // Off asks first; the row shows the old state until the answer comes.
        actions.push(if safe {
            Action::SetTrustSafety(true)
        } else {
            Action::OpenDialog(Dialog::ConfirmTrustSafetyOff)
        });
    }
    heading(ui, "Profile pictures");
    let help = "Shows GitHub or GitLab account pictures next to authors.";
    let mut open_accounts = false;
    let note = pictures.map(|why| Note {
        text: format!("{}. {}", why.title, why.detail),
        warn: false,
    });
    noted_row_layout(
        ui,
        "Download profile pictures",
        None,
        help,
        &|ui: &mut Ui| help_label(ui, help),
        "Sends commit IDs to GitHub or author emails to GitLab. Pictures, and authors without one, are remembered for 12 hours, and a failed refresh keeps the previous picture. Off shows initials and needs Pull and merge requests on to turn back on.",
        note.as_ref(),
        |ui| {
            revert(
                ui,
                &mut s.download_avatars,
                &DEFAULTS.download_avatars,
                "Download profile pictures",
            );
            switch(ui, &mut s.download_avatars, "Download profile pictures");
            // Right to left: the way to fix it sits before the switch.
            if pictures.is_some_and(|why| why.open_accounts)
                && Button::new("Open Accounts")
                    .ghost()
                    .size(Size::Sm)
                    .show(ui)
                    .clicked()
            {
                open_accounts = true;
            }
        },
    );
    row_end(ui);
    heading(ui, "Diagnostics");
    row_more(
        ui,
        "Log detail",
        "Normal records operations and problems; Detailed adds every git command and API call.",
        "Normal: start-up, each operation and its outcome, warnings and errors.\nDetailed: also every git command, API call and repository job, with timings.\nEverything: also the libraries' own messages, which Canopy does not filter.\n\nCanopy never logs tokens, passwords or answers to prompts, and removes URL credentials and commit messages from the git commands it logs.\n\nThe CANOPY_LOG or RUST_LOG environment variable overrides this setting.",
        |ui| {
            revert(ui, &mut s.log_level, &DEFAULTS.log_level, "Log detail");
            let before = s.log_level;
            let options = crate::logging::LogLevel::ALL.map(|l| (l, l.label()));
            Select::new(&mut s.log_level, &options)
                .fit_to_options()
                .name("Log detail")
                .id_salt("log-level")
                .show(ui);
            if s.log_level != before {
                crate::logging::set_level(s.log_level);
            }
        },
    );
    row_end(ui);
    let logs = format!(
        "One log file per run on this computer; the newest {} are kept. Nothing is uploaded.",
        crate::logging::KEEP_RUNS,
    );
    let more = format!(
        "Open folder opens the logs folder. Each log stops at {} MB.\n\nCrash files are written when Canopy crashes, whether or not you send reports. They are in the crashes folder beside it, the newest {} kept. Send crash reports above is a separate, opt-in upload.\n\nCopy diagnostics puts this on the clipboard and sends nothing: the Canopy, Git and system versions, the run id, the log file's path, your log detail, theme, contrast and translucency settings, the number of open repositories, your crash-report answer and the last {} warnings and errors, with your home folder shown as ~.",
        crate::logging::MAX_RUN_BYTES / (1024 * 1024),
        crate::logging::KEEP_CRASHES,
        crate::logging::RECENT,
    );
    row_more(ui, "Logs and crash files", &logs, &more, |ui| {
        // Right to left.
        if Button::new("Copy diagnostics")
            .icon(icon::CLIPBOARD_TEXT)
            .show(ui)
            .clicked()
        {
            actions.push(Action::CopyDiagnostics);
        }
        if Button::new("Open folder")
            .icon(icon::FOLDER_OPEN)
            .show(ui)
            .clicked()
        {
            actions.push(Action::OpenLogs);
        }
    });
    row_end(ui);
    open_accounts
}

/// The trusted folders, each with a button that stops trusting it.
fn trusted_folder_list(ui: &mut Ui, s: &mut Settings) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.add_space(theme::XS);
    let mut remove = None;
    for (i, folder) in s.trusted_folders.iter().enumerate() {
        ui.horizontal(|ui| {
            if IconButton::new(icon::X, "Stop trusting this folder")
                .size(Size::Sm)
                .id_salt(("untrust-folder", i))
                .show(ui)
                .clicked()
            {
                remove = Some(i);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(folder.display().to_string())
                        .font(FontId::new(m.small_size, FontFamily::Monospace))
                        .color(p.text_dim),
                )
                .truncate(),
            )
            .kit_tooltip(folder.display().to_string())
            .show();
        });
    }
    if let Some(i) = remove {
        s.trusted_folders.remove(i);
        // At once, not on the next frame: a download queued on a repository thread must see it.
        crate::trust::publish(s);
    }
    if s.trusted_folders.is_empty() {
        ui.label(
            RichText::new("No folder is trusted yet.")
                .size(m.small_size)
                .color(p.text_faint),
        );
    }
}

fn updates(ui: &mut Ui, s: &mut Settings, update: &UpdateState, actions: &mut Vec<Action>) {
    if block(ui, "Version", "Canopy") {
        update_status(ui, s, update, actions);
    }
    if !Updater::enabled() {
        return;
    }
    group_gap(ui, theme::XL);
    heading(ui, "Preferences");
    row_explained(
        ui,
        "Check for updates",
        "Once a day, Canopy checks GitHub for a newer build.",
        "Canopy reads a small public file on GitHub, which sees your IP address and build number. Nothing downloads until you choose Update.",
        |ui| {
            revert(
                ui,
                &mut s.check_updates,
                &DEFAULTS.check_updates,
                "Check for updates",
            );
            switch(ui, &mut s.check_updates, "Check for updates");
        },
    );
    row(
        ui,
        "Include beta builds",
        "Also offer beta builds, which are less tested than stable ones.",
        |ui| {
            revert(
                ui,
                &mut s.include_prereleases,
                &DEFAULTS.include_prereleases,
                "Include beta builds",
            );
            switch(ui, &mut s.include_prereleases, "Include beta builds");
        },
    );
}

/// The running version, the last check's answer and the update actions.
fn update_status(ui: &mut Ui, s: &mut Settings, update: &UpdateState, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    if !Updater::enabled() {
        ui.label(
            RichText::new("This build of Canopy is updated by your package manager.")
                .color(p.text_dim),
        );
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("Canopy {}", crate::build_info::label()))
                .font(FontId::new(m.lead_size, theme::medium()))
                .color(p.text),
        );
        let (text, color) = match update {
            UpdateState::Idle => (String::new(), p.text_dim),
            UpdateState::Checking => ("Checking…".to_owned(), p.text_dim),
            UpdateState::UpToDate => (format!("{}  Up to date", icon::CHECK), p.success),
            UpdateState::Available(a) => (
                format!(
                    "{}  Canopy {} is available",
                    icon::ARROW_CIRCLE_UP,
                    a.label()
                ),
                p.accent,
            ),
            UpdateState::Failed(e) => (format!("{}  {e}", icon::WARNING), p.danger),
        };
        ui.label(RichText::new(text).size(m.meta_size).color(color));
    });
    ui.add_space(theme::SM);
    ui.horizontal_wrapped(|ui| {
        if Button::new("Check now")
            .icon(icon::ARROWS_CLOCKWISE)
            .loading(*update == UpdateState::Checking)
            .show(ui)
            .clicked()
        {
            actions.push(Action::CheckUpdates);
        }
        if matches!(update, UpdateState::Available(_))
            && Button::new("What's new")
                .primary()
                .icon(icon::SPARKLE)
                .show(ui)
                .clicked()
        {
            actions.push(Action::ShowUpdate);
        }
        if Button::new("All releases")
            .ghost()
            .icon(icon::LIST)
            .show(ui)
            .clicked()
        {
            actions.push(Action::OpenUrl(updater::RELEASES_PAGE.to_owned()));
        }
    });
    if let Some(v) = s.skipped_version.clone() {
        ui.add_space(theme::SM);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("Skipping version {v}."))
                    .size(m.meta_size)
                    .color(p.text_dim),
            );
            if Button::new("Stop skipping")
                .ghost()
                .size(Size::Sm)
                .show(ui)
                .clicked()
            {
                s.skipped_version = None;
            }
        });
    }
}

/// Takes an export's or import's result once its file chooser and file access are done.
fn poll_file_task(screen: &mut SettingsScreen, s: &mut Settings, actions: &mut Vec<Action>) {
    use crate::settings_file::Outcome;
    let Some(outcome) = screen
        .file_task
        .as_ref()
        .and_then(crate::settings_file::FileTask::poll)
    else {
        return;
    };
    screen.file_task = None;
    screen.file_status = match outcome {
        Outcome::Cancelled => None,
        Outcome::Failed(e) => Some((true, e)),
        Outcome::Exported(path) => Some((false, format!("Exported to {}.", path.display()))),
        Outcome::Read(path, text) => match crate::settings_file::import(s, &text) {
            Ok(imported) => {
                let message = imported.message(&path);
                *s = imported.settings;
                actions.push(Action::SettingsReplaced);
                Some((false, message))
            }
            Err(e) => Some((true, e)),
        },
    };
}

/// Backup and reset: settings to and from a file, and every preference back to its default.
fn backup(ui: &mut Ui, screen: &mut SettingsScreen, s: &mut Settings, actions: &mut Vec<Action>) {
    let busy = screen.file_task.is_some();
    if let Some((failed, message)) = &screen.file_status
        && !Filter::searching(ui.ctx())
    {
        Callout::new(
            if *failed {
                Tone::Warning
            } else {
                Tone::Success
            },
            message.as_str(),
        )
        .show(ui);
        ui.add_space(theme::SM);
    }
    heading(ui, "Settings file");
    row_explained(
        ui,
        "Export settings",
        "Saves your preferences to a file, without consent.",
        "Left out: consent, trusted folders, the programs Canopy runs, GitLab hosts, the clone folder and per-repository settings.",
        |ui| {
            if Button::new(if busy { "Choosing…" } else { "Export…" })
                .icon(icon::EXPORT)
                .enabled(!busy)
                .id_salt("settings-export")
                .show(ui)
                .clicked()
            {
                match crate::settings_file::export(s) {
                    Ok(text) => {
                        screen.file_task =
                            Some(crate::settings_file::FileTask::export(ui.ctx(), text));
                        screen.file_status = None;
                    }
                    Err(e) => screen.file_status = Some((true, e)),
                }
            }
        },
    );
    row_explained(
        ui,
        "Import settings",
        "Loads preferences from a settings file.",
        "What the file leaves out stays as it is. A file never chooses the programs Canopy runs or the GitLab hosts Canopy signs in to, and never turns a network feature on.",
        |ui| {
            if Button::new(if busy { "Choosing…" } else { "Import…" })
                .icon(icon::DOWNLOAD_SIMPLE)
                .enabled(!busy)
                .id_salt("settings-import")
                .show(ui)
                .clicked()
            {
                screen.file_task = Some(crate::settings_file::FileTask::import(ui.ctx()));
                screen.file_status = None;
            }
        },
    );
    heading(ui, "Start over");
    row(
        ui,
        "Reset all settings",
        "Restores every preference to its recommended value. Trusted folders, repositories and consent stay.",
        |ui| {
            if confirm_button(ui, "reset-all-settings", "Reset all", "Reset every setting") {
                match crate::settings_file::reset(s) {
                    Ok(fresh) => {
                        *s = fresh;
                        actions.push(Action::SettingsReplaced);
                        screen.file_status = Some((
                            false,
                            "Every setting is back to its recommended value.".to_owned(),
                        ));
                    }
                    Err(e) => screen.file_status = Some((true, e)),
                }
            }
        },
    );
}

/// Settings → Keyboard: every key of the keymap by where it applies, each with a button that
/// listens for its new key, and Reset where the user changed it.
fn keyboard(ui: &mut Ui, screen: &mut SettingsScreen, s: &mut Settings) {
    use crate::keymap::{self, Binding};
    // The key a waiting row was given, or that Esc cancelled.
    keymap::keep_capturing(ui.ctx());
    if let Some(taken) = keymap::take_capture()
        && let Some(id) = screen.rebinding.take()
        && let keymap::Captured::Key(chord) = taken
        && let Some(binding) = keymap::binding_by_id(&id)
    {
        if let Some(why) = keymap::rebind_problem(binding, chord) {
            screen.key_note = Some((id, why));
        } else {
            screen.key_note = None;
            if binding.chords == [chord] {
                s.keybindings.remove(&id);
            } else {
                s.keybindings.insert(id, chord.to_text());
            }
        }
    }
    if screen.rebinding.is_some() && !keymap::is_capturing() {
        screen.rebinding = None;
    }
    let p = palette(ui.ctx());
    if !Filter::searching(ui.ctx()) {
        ui.scope(|ui| {
            // Clear of the scrollbar, which floats over the list's right edge.
            ui.set_max_width(ui.available_width() - theme::MD);
            help_label(
                ui,
                concat!(
                    "Anywhere keys also work while typing; single keys only outside text fields. ",
                    "Change replaces a row's keys with the one you press; Esc cancels."
                ),
            );
        });
        ui.add_space(theme::MD);
    }
    for section in KeySection::ALL {
        let rows: Vec<&Binding> = keymap::BINDINGS
            .iter()
            .filter(|b| b.section == section)
            .collect();
        ui.push_id(("keyboard", section.title()), |ui| {
            heading(ui, section.title());
            for b in rows {
                let id = keymap::binding_id(b);
                let waiting = screen.rebinding.as_deref() == Some(id.as_str());
                let note =
                    screen
                        .key_note
                        .as_ref()
                        .filter(|(row, _)| *row == id)
                        .map(|(_, why)| Note {
                            text: why.clone(),
                            warn: true,
                        });
                let keys = b.keys();
                noted_row_layout(
                    ui,
                    b.description,
                    None,
                    &keys,
                    &|_| {},
                    "",
                    note.as_ref(),
                    |ui| {
                        if !keymap::is_remappable(b) {
                            Keycap::new(keys.clone()).show(ui);
                            return;
                        }
                        // Right to left: Change is rightmost.
                        if waiting {
                            if Button::new("Cancel")
                                .ghost()
                                .id_salt(("keys-cancel", &id))
                                .show(ui)
                                .clicked()
                            {
                                keymap::cancel_capture();
                                screen.rebinding = None;
                            }
                            ui.label(
                                RichText::new("Press the new key")
                                    .size(theme::metrics().meta_size)
                                    .color(p.accent),
                            );
                            return;
                        }
                        if Button::new("Change")
                            .icon(icon::KEYBOARD)
                            .id_salt(("keys-change", &id))
                            .show(ui)
                            .clicked()
                        {
                            keymap::start_capture(ui.ctx());
                            screen.rebinding = Some(id.clone());
                            screen.key_note = None;
                        }
                        if keymap::is_customised(b)
                            && Button::new("Reset")
                                .ghost()
                                .id_salt(("keys-reset", &id))
                                .show(ui)
                                .clicked()
                        {
                            s.keybindings.remove(&id);
                            screen.key_note = None;
                        }
                        for key in keys.rsplit("  ·  ") {
                            Keycap::new(key.to_owned()).show(ui);
                        }
                    },
                );
                row_end(ui);
            }
        });
    }
    if !s.keybindings.is_empty() {
        heading(ui, "Start over");
        row(
            ui,
            "Reset all shortcuts",
            "Every key back to the one Canopy ships with.",
            |ui| {
                if confirm_button(ui, "reset-keys", "Reset all", "Reset every shortcut") {
                    s.keybindings.clear();
                    screen.key_note = None;
                }
            },
        );
    }
}

/// What the repository's git configuration decides against Canopy's recommendation, one line per
/// choice, for the notification center. Empty unless Canopy follows the configuration (when it
/// does not, Canopy's own choice applies and nothing is overruled) and it sets such a value.
pub(crate) fn against_recommendation(
    facts: &GitFacts,
    settings: &Settings,
    repo: &Path,
) -> Vec<String> {
    if !settings.follow_git_config {
        return Vec::new();
    }
    let follow = true;
    let pull = settings.pull_for(repo);
    let follow_tags = config_note(
        &facts.follow_tags.iter().collect::<Vec<_>>(),
        follow,
        |v| v.as_bool() == Some(settings.follow_tags),
        "pushes",
    );
    // A pull setting is judged against its own row's recommendation, as the editor marks it.
    let mut lines: Vec<String> = facts
        .pull
        .iter()
        .filter(|_| pull.follow_config)
        .flat_map(pull_values)
        .filter_map(|value| {
            let (row, _) = row_of(&value.key);
            against_line(row?, &value, facts.git)
        })
        .collect();
    for (setting, note) in [
        (
            Setting::FetchPrune,
            prune_note(facts, settings.prune_on_fetch, follow).filter(|_| {
                fetch_values(
                    facts,
                    facts.fetch_prune.as_ref(),
                    &facts.remote_prune,
                    "prune",
                )
                .iter()
                .any(|v| !v.as_bool().unwrap_or(false))
            }),
        ),
        (
            Setting::FetchPruneTags,
            prune_tags_note(facts, settings.prune_tags, follow).filter(|_| {
                let (fetch, remote): (Vec<ConfigValue>, Vec<ConfigValue>) = facts
                    .prune_tags
                    .iter()
                    .cloned()
                    .partition(|v| v.key.eq_ignore_ascii_case("fetch.pruneTags"));
                fetch_values(facts, fetch.first(), &remote, "pruneTags")
                    .iter()
                    .any(|v| v.as_bool() == Some(true))
            }),
        ),
        (
            Setting::PullAutostash,
            autostash_note(facts, settings.autostash, follow)
                .filter(|_| facts.autostash.iter().any(|v| v.as_bool() != Some(true))),
        ),
        (
            Setting::PushFollowTags,
            follow_tags.filter(|_| {
                facts
                    .follow_tags
                    .as_ref()
                    .is_some_and(|v| v.as_bool() != Some(true))
            }),
        ),
    ] {
        if let Some(note) = note {
            let text = note
                .text
                .trim_start_matches(DECIDES)
                .trim_start_matches(": ");
            lines.push(format!(
                "{} Recommended: {}.",
                upper_first(text),
                recommended_label(setting, facts.git)
            ));
        }
    }
    for (setting, value) in [
        (Setting::PushDefault, facts.push_default.as_ref()),
        (Setting::ConflictStyle, facts.conflict_style.as_ref()),
        (Setting::CommitCleanup, facts.cleanup.as_ref()),
        (Setting::InitialBranch, facts.initial_branch.as_ref()),
        (Setting::SubmoduleRecurse, facts.submodule_recurse.as_ref()),
    ] {
        lines.extend(value.and_then(|value| against_line(setting, value, facts.git)));
    }
    lines
}

/// The recommendation for `setting`, named as its row's list names it.
fn recommended_label(setting: Setting, git: Option<(u32, u32)>) -> String {
    git_value_label(setting, setting.recommendation_for(git))
}

/// What `value` says against `setting`'s recommendation, in the words of the editor's row:
/// where it is set, then what Canopy recommends for that same row. `None` when it is the
/// recommendation.
fn against_line(setting: Setting, value: &ConfigValue, git: Option<(u32, u32)>) -> Option<String> {
    let recommended = setting.recommendation_for(git);
    let is_recommended = match recommended {
        "true" | "false" => value.as_bool() == Some(recommended == "true"),
        _ => value.value == recommended,
    };
    (!is_recommended).then(|| {
        format!(
            "{}. Recommended: {}.",
            upper_first(&config_phrase(value)),
            recommended_label(setting, git)
        )
    })
}

/// How many findings the notice lists in full; more are cut to the first few and a count.
const NOTICE_FINDINGS: usize = 4;

/// The notice's text from the findings of [`against_recommendation`]: one paragraph each, so
/// the card reads as a list, and past [`NOTICE_FINDINGS`] the first few and how many more
/// the settings screen shows.
pub(crate) fn notice_body(findings: &[String]) -> String {
    let mut paragraphs = findings.to_vec();
    if findings.len() > NOTICE_FINDINGS {
        paragraphs.truncate(NOTICE_FINDINGS - 1);
        paragraphs.push(format!(
            "And {} more.",
            findings.len() - (NOTICE_FINDINGS - 1)
        ));
    }
    paragraphs.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_config::Signing;
    use canopy_core::{cli, config};

    /// Each state of a host's card has a line of its own, in a tone that says how it stands.
    #[test]
    fn each_cli_state_has_its_own_status_line() {
        let (gh, glab) = (ForgeKind::GitHub, ForgeKind::GitLab);
        let account = |user: Option<&str>| {
            Ok(canopy_hosting::CliAccount {
                user: user.map(str::to_owned),
                scopes: None,
            })
        };
        let missing = CliError::MissingScope {
            kind: gh,
            host: "github.com".into(),
            user: Some("octocat".into()),
            scope: "repo",
        };
        let cases = [
            (
                account(Some("tanuki")),
                glab,
                "Using the GitLab CLI's sign-in as tanuki",
                Tone::Success,
            ),
            (
                account(None),
                gh,
                "Using the GitHub CLI's sign-in",
                Tone::Success,
            ),
            (
                Err(CliError::NotInstalled { kind: gh }),
                gh,
                "The GitHub CLI is not installed",
                Tone::Neutral,
            ),
            (
                hosting::signed_out(glab, "gitlab.corp"),
                glab,
                "Not signed in with the GitLab CLI",
                Tone::Warning,
            ),
            (
                Err(missing),
                gh,
                "Signed in as octocat with the GitHub CLI, without the repo scope",
                Tone::Warning,
            ),
            (
                Err(CliError::Failed {
                    kind: glab,
                    message: "it took longer than 10 seconds".into(),
                }),
                glab,
                "The GitLab CLI could not be used",
                Tone::Danger,
            ),
        ];
        for (state, kind, line, tone) in cases {
            assert_eq!(account_status(kind, &state), (line.to_owned(), tone));
        }
    }

    /// The scopes line of a signed-in card lists what the CLI listed, and nothing for a CLI that
    /// lists none (`glab`).
    #[test]
    fn a_signed_in_card_lists_the_scopes_the_cli_listed() {
        let scoped = canopy_hosting::CliAccount {
            user: None,
            scopes: Some(vec!["gist".into(), "repo".into()]),
        };
        assert_eq!(scopes_line(&scoped).as_deref(), Some("Scopes: gist, repo"));
        let bare = canopy_hosting::CliAccount {
            scopes: None,
            ..scoped
        };
        assert_eq!(scopes_line(&bare), None);
    }

    /// Settings with the start-up questions answered, so the Accounts page may run the CLIs.
    fn answered() -> Settings {
        Settings {
            accepted_terms: crate::ui::legal::TERMS_VERSION.to_owned(),
            accepted_privacy: crate::ui::legal::PRIVACY_VERSION.to_owned(),
            crash_reports: Some(false),
            crash_reports_version: crate::crash_reports::DATA_VERSION,
            ..Settings::default()
        }
    }

    /// One frame of the Accounts page: what it speaks, and the hosts it says are stale. Every
    /// host the page lists needs an entry in `screen.checks` first, or the page would run the real
    /// CLI for it.
    fn accounts_frame(
        ctx: &egui::Context,
        screen: &mut SettingsScreen,
        settings: &Settings,
        tabs: &[crate::tab::RepoTab],
        actions: &mut Vec<Action>,
    ) -> (Vec<crate::ui::kit::testkit::Spoken>, Vec<String>) {
        let mut changed = Vec::new();
        let spoken = crate::ui::kit::testkit::spoken(ctx, |ui| {
            changed = accounts(ui, screen, settings, tabs, actions);
        });
        (spoken, changed)
    }

    /// Whether any node's name or words contain `part`.
    fn said(spoken: &[crate::ui::kit::testkit::Spoken], part: &str) -> bool {
        spoken.iter().any(|n| {
            [&n.label, &n.value]
                .into_iter()
                .flatten()
                .any(|t| t.contains(part))
        })
    }

    fn seeded(states: Vec<(&str, hosting::CliState)>) -> SettingsScreen {
        let mut screen = SettingsScreen::at(Section::Accounts);
        for (host, state) in states {
            screen.checks.insert(host.to_owned(), CliCheck::Done(state));
        }
        screen
    }

    /// Each state of a host's card says what is wrong in words of its own and offers the one
    /// action that fixes it; the actions work.
    #[test]
    fn every_account_state_has_its_words_and_its_action() {
        use crate::ui::kit::testkit::{self, find};
        let ctx = testkit::ctx();
        let settings = answered();
        let mut screen = seeded(vec![
            (
                "github.com",
                Err(CliError::NotInstalled {
                    kind: ForgeKind::GitHub,
                }),
            ),
            (
                "gitlab.com",
                hosting::signed_out(ForgeKind::GitLab, "gitlab.com"),
            ),
        ]);
        let mut actions = Vec::new();
        accounts_frame(&ctx, &mut screen, &settings, &[], &mut actions);
        let (words, changed) = accounts_frame(&ctx, &mut screen, &settings, &[], &mut actions);
        assert_eq!(changed, Vec::<String>::new());
        assert!(said(&words, "The GitHub CLI is not installed"));
        assert!(said(&words, "Not signed in with the GitLab CLI"));
        assert!(said(&words, "glab auth login --hostname gitlab.com"));
        assert!(find(&words, "Install GitHub CLI").is_some());
        assert!(find(&words, "Copy the command").is_some());
        let rechecks = words
            .iter()
            .filter(|n| n.label.as_deref() == Some("Check again"))
            .count();
        assert_eq!(rechecks, 2, "one on each card");
        // No token is asked for or accepted anywhere on the page.
        for word in ["Paste", "paste", "Save", "Create a token"] {
            assert!(!said(&words, word), "{word}");
        }
        let centre = |label: &str| find(&words, label).and_then(|n| n.bounds).unwrap().center();
        for label in ["Install GitHub CLI", "Copy the command"] {
            testkit::click(&ctx, centre(label), |ui| {
                accounts(ui, &mut screen, &settings, &[], &mut actions);
            });
        }
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::OpenUrl(url) if url == "https://cli.github.com")),
            "{actions:?}"
        );
        assert!(
            actions.iter().any(|a| matches!(
                a,
                Action::Copy(text) if text == "glab auth login --hostname gitlab.com"
            )),
            "{actions:?}"
        );
    }

    #[test]
    fn a_signed_in_card_names_the_account_and_its_scopes() {
        use crate::ui::kit::testkit;
        let ctx = testkit::ctx();
        let settings = answered();
        let mut screen = seeded(vec![
            (
                "github.com",
                Ok(canopy_hosting::CliAccount {
                    user: Some("octocat".into()),
                    scopes: Some(vec!["gist".into(), "repo".into()]),
                }),
            ),
            (
                "gitlab.com",
                Err(CliError::MissingScope {
                    kind: ForgeKind::GitLab,
                    host: "gitlab.com".into(),
                    user: Some("tanuki".into()),
                    scope: "api",
                }),
            ),
        ]);
        let mut actions = Vec::new();
        accounts_frame(&ctx, &mut screen, &settings, &[], &mut actions);
        let (words, _) = accounts_frame(&ctx, &mut screen, &settings, &[], &mut actions);
        assert!(said(&words, "Using the GitHub CLI's sign-in as octocat"));
        assert!(said(&words, "Scopes: gist, repo"));
        assert!(said(
            &words,
            "Signed in as tanuki with the GitLab CLI, without the api scope"
        ));
        assert!(said(&words, "glab auth login --hostname gitlab.com"));
        // GitLab's sign-in asks which scopes to grant: the card says to allow the missing one.
        assert!(
            said(
                &words,
                "Run this in a terminal and allow the api scope, then check again."
            ),
            "{words:#?}"
        );
    }

    /// While pull and merge requests are off, or before the start-up questions are answered, the
    /// page runs no CLI (it would contact the host) and says so.
    #[test]
    fn the_page_runs_no_cli_while_requests_are_off_or_unanswered() {
        use crate::ui::kit::testkit;
        for settings in [
            Settings {
                hosting_api: false,
                ..answered()
            },
            Settings::default(),
        ] {
            let ctx = testkit::ctx();
            let mut screen = SettingsScreen::at(Section::Accounts);
            let mut actions = Vec::new();
            let (words, changed) = accounts_frame(&ctx, &mut screen, &settings, &[], &mut actions);
            assert!(screen.checks.is_empty(), "a CLI was started");
            assert_eq!(changed, Vec::<String>::new());
            assert!(said(
                &words,
                "Not checked while pull and merge requests are off"
            ));
            assert!(
                !words
                    .iter()
                    .any(|n| n.label.as_deref() == Some("Check again")),
                "nothing to check again"
            );
        }
    }

    /// A tab with 200 remotes on 200 GitLab hosts.
    fn tab_with_many_hosts() -> crate::tab::RepoTab {
        let (worker, _events, _jobs) = crate::worker::Worker::detached();
        let mut tab = crate::tab::RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.forges = (0..200)
            .map(|n| {
                (
                    format!("r{n}"),
                    Forge {
                        kind: ForgeKind::GitLab,
                        host: format!("gitlab-{n}.example"),
                        port: None,
                        path: "x/y".into(),
                    },
                )
            })
            .collect();
        tab
    }

    /// A repository with hundreds of remotes on as many hosts starts at most [`MAX_CHECKS`] CLIs
    /// at once, and only for the hosts it shows: the listed ones and [`MAX_REPO_HOSTS`] of the
    /// repository's, the rest counted in one line.
    #[test]
    fn many_repository_hosts_are_capped() {
        use crate::ui::kit::testkit;
        let tab = tab_with_many_hosts();
        let tabs = std::slice::from_ref(&tab);
        let page = AccountHosts::of(&answered(), tabs);
        assert_eq!(page.hosts.len(), 2 + MAX_REPO_HOSTS);
        assert_eq!(page.more, 200 - MAX_REPO_HOSTS);
        // Nothing checked yet: the first MAX_CHECKS start, then one per slot that frees.
        let mut checks = HashMap::new();
        assert_eq!(checks_to_start(&page.hosts, &checks, 0).len(), MAX_CHECKS);
        let running: Vec<_> = (0..MAX_CHECKS - 1)
            .map(|_| crossbeam_channel::unbounded::<hosting::CliState>())
            .collect();
        for (n, (_, rx)) in running.iter().enumerate() {
            checks.insert(page.hosts[n].authority(), CliCheck::Running(rx.clone()));
        }
        let next = checks_to_start(&page.hosts, &checks, MAX_CHECKS - 1);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].host, page.hosts[MAX_CHECKS - 1].host);
        let (_, rx) = crossbeam_channel::unbounded::<hosting::CliState>();
        checks.insert(next[0].authority(), CliCheck::Running(rx));
        assert_eq!(
            checks_to_start(&page.hosts, &checks, MAX_CHECKS),
            Vec::<&Forge>::new()
        );

        // Entries left by a page that closed are not how availability is measured: the counter of
        // real threads is. Stale map entries alone must neither consume nor invent free slots.
        let mut orphaned = HashMap::new();
        for (_, rx) in running {
            orphaned.insert(format!("orphan-{}", orphaned.len()), CliCheck::Running(rx));
        }
        assert_eq!(checks_to_start(&page.hosts, &orphaned, 0).len(), MAX_CHECKS);
        assert_eq!(
            checks_to_start(&page.hosts, &orphaned, MAX_CHECKS),
            Vec::<&Forge>::new()
        );

        // The page shows those cards and counts the rest; with every shown host answered it
        // starts nothing for the hidden ones.
        let ctx = testkit::ctx();
        let settings = Settings {
            hosting_api: true,
            ..answered()
        };
        let mut screen = SettingsScreen::at(Section::Accounts);
        for forge in &page.hosts {
            screen.checks.insert(
                forge.authority(),
                CliCheck::Done(hosting::signed_out(forge.kind, &forge.host)),
            );
        }
        let (words, _) = accounts_frame(&ctx, &mut screen, &settings, tabs, &mut Vec::new());
        assert_eq!(
            screen.checks.len(),
            2 + MAX_REPO_HOSTS,
            "no check for a hidden host"
        );
        assert!(said(&words, "gitlab-3.example"));
        assert!(!said(&words, "gitlab-4.example"));
        assert!(said(&words, "And 196 more hosts of open repositories."));
        let cards = words
            .iter()
            .filter(|n| n.label.as_deref() == Some("Check again"))
            .count();
        assert_eq!(cards, 2 + MAX_REPO_HOSTS);
    }

    /// A configured GitLab authority includes its web port in the CLI check, including when no
    /// open repository can supply a correctly ported remote.
    #[test]
    fn configured_gitlab_hosts_keep_their_non_default_ports() {
        let settings = Settings {
            gitlab_hosts: "GitLab.Corp:8443, https://gitlab.other:9443/root".into(),
            ..answered()
        };
        let page = AccountHosts::of(&settings, &[]);
        let corp = page
            .hosts
            .iter()
            .find(|forge| forge.host == "gitlab.corp")
            .expect("configured host");
        assert_eq!(corp.port, Some(8443));
        assert_eq!(corp.authority(), "gitlab.corp:8443");
        let other = page
            .hosts
            .iter()
            .find(|forge| forge.host == "gitlab.other")
            .expect("configured host with a path");
        assert_eq!(other.port, Some(9443));
    }

    /// The page's host list is built once and kept while the settings and the tabs' remotes stay
    /// the same, not every frame.
    #[test]
    fn the_account_hosts_are_built_again_only_when_they_change() {
        use crate::ui::kit::testkit;
        let ctx = testkit::ctx();
        let tab = tab_with_many_hosts();
        let tabs = std::slice::from_ref(&tab);
        // Off, so no CLI runs.
        let mut settings = Settings {
            hosting_api: false,
            ..answered()
        };
        let mut screen = SettingsScreen::at(Section::Accounts);
        let built = |screen: &SettingsScreen| Arc::clone(&screen.account_hosts.as_ref().unwrap().1);
        accounts_frame(&ctx, &mut screen, &settings, tabs, &mut Vec::new());
        let first = built(&screen);
        accounts_frame(&ctx, &mut screen, &settings, tabs, &mut Vec::new());
        assert!(Arc::ptr_eq(&first, &built(&screen)), "kept between frames");
        settings.gitlab_hosts = "gitlab.corp".into();
        accounts_frame(&ctx, &mut screen, &settings, tabs, &mut Vec::new());
        let second = built(&screen);
        assert!(!Arc::ptr_eq(&first, &second), "built again for a new host");
        assert!(second.hosts.iter().any(|f| f.host == "gitlab.corp"));
    }

    /// A check that finds another answer than the one a tab's hosting thread started with marks
    /// that host, so the app restarts the tab; an answer that agrees marks nothing.
    #[test]
    fn a_changed_sign_in_marks_the_host_of_the_tab_that_holds_the_old_one() {
        use crate::ui::kit::testkit;
        let ctx = testkit::ctx();
        let settings = answered();
        let forge = Forge::parse("https://github.com/acme/demo.git", &[]).unwrap();
        let (worker, _events, _jobs) = crate::worker::Worker::detached();
        let mut tab = crate::tab::RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.forges = vec![("origin".to_owned(), forge.clone())];
        tab.host = Some(hosting::HostWorker::detached(forge).0);
        tab.requests.cli = Some(hosting::signed_out(ForgeKind::GitHub, "github.com"));
        let signed_in: hosting::CliState = Ok(hosting::demo_account());
        for (answer, stale) in [
            (hosting::signed_out(ForgeKind::GitHub, "github.com"), false),
            (signed_in, true),
        ] {
            let mut screen = SettingsScreen::at(Section::Accounts);
            let (tx, rx) = crossbeam_channel::unbounded();
            screen
                .checks
                .insert("github.com".into(), CliCheck::Running(rx));
            screen.checks.insert(
                "gitlab.com".into(),
                CliCheck::Done(hosting::signed_out(ForgeKind::GitLab, "gitlab.com")),
            );
            tx.send(answer).unwrap();
            let mut actions = Vec::new();
            let (_, changed) = accounts_frame(
                &ctx,
                &mut screen,
                &settings,
                std::slice::from_ref(&tab),
                &mut actions,
            );
            assert_eq!(
                changed,
                if stale {
                    vec!["github.com".to_owned()]
                } else {
                    vec![]
                }
            );
            assert!(matches!(
                screen.checks.get("github.com"),
                Some(CliCheck::Done(_))
            ));
        }
    }

    /// A rejected credential must be replaced even when the CLI comes back with the same account
    /// and scopes: the token itself may have been renewed.
    #[test]
    fn a_rejected_credential_is_retried_when_the_account_did_not_change() {
        use crate::ui::kit::testkit;
        let ctx = testkit::ctx();
        let settings = answered();
        let forge = Forge::parse("https://github.com/acme/demo.git", &[]).unwrap();
        let (worker, _events, _jobs) = crate::worker::Worker::detached();
        let mut tab = crate::tab::RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.forges = vec![("origin".to_owned(), forge.clone())];
        tab.host = Some(hosting::HostWorker::detached(forge).0);
        let signed_in: hosting::CliState = Ok(hosting::demo_account());
        tab.requests.cli = Some(signed_in.clone());
        tab.requests.token_rejected = true;
        let mut screen = SettingsScreen::at(Section::Accounts);
        let (tx, rx) = crossbeam_channel::unbounded();
        screen
            .checks
            .insert("github.com".into(), CliCheck::Running(rx));
        tx.send(signed_in).unwrap();

        let (_, changed) = accounts_frame(
            &ctx,
            &mut screen,
            &settings,
            std::slice::from_ref(&tab),
            &mut Vec::new(),
        );

        assert_eq!(changed, vec!["github.com".to_owned()]);
    }

    /// Check again replaces the worker's token even when reauthentication returned the identical
    /// account metadata, which is the usual shape of a token renewal.
    #[test]
    fn check_again_restarts_a_tab_with_the_same_account_and_scopes() {
        use crate::ui::kit::testkit;
        let ctx = testkit::ctx();
        let settings = answered();
        let forge = Forge::parse("https://github.com/acme/demo.git", &[]).unwrap();
        let (worker, _events, _jobs) = crate::worker::Worker::detached();
        let mut tab = crate::tab::RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.forges = vec![("origin".to_owned(), forge.clone())];
        tab.host = Some(hosting::HostWorker::detached(forge).0);
        let signed_in: hosting::CliState = Ok(hosting::demo_account());
        tab.requests.cli = Some(signed_in.clone());
        let mut screen = SettingsScreen::at(Section::Accounts);
        screen.rechecks.insert("github.com".into());
        let (tx, rx) = crossbeam_channel::unbounded();
        screen
            .checks
            .insert("github.com".into(), CliCheck::Running(rx));
        tx.send(signed_in).unwrap();

        let (_, changed) = accounts_frame(
            &ctx,
            &mut screen,
            &settings,
            std::slice::from_ref(&tab),
            &mut Vec::new(),
        );

        assert_eq!(changed, vec!["github.com".to_owned()]);
        assert!(!screen.rechecks.contains("github.com"));
    }

    /// A host that drops off the page while its check runs still has its answer collected, so its
    /// tab restarts and an orphaned check cannot hold a slot forever.
    #[test]
    fn a_finished_check_is_collected_after_its_host_leaves_the_page() {
        use crate::ui::kit::testkit;
        let ctx = testkit::ctx();
        let settings = Settings {
            hosting_api: false,
            ..answered()
        };
        let mut tab = tab_with_many_hosts();
        let hidden = tab.forges[100].1.clone();
        tab.host = Some(hosting::HostWorker::detached(hidden.clone()).0);
        tab.requests.cli = Some(hosting::signed_out(hidden.kind, &hidden.host));
        let key = hidden.authority().to_lowercase();
        assert!(
            !AccountHosts::of(&settings, std::slice::from_ref(&tab))
                .hosts
                .iter()
                .any(|forge| forge.authority().eq_ignore_ascii_case(&key))
        );

        let mut screen = SettingsScreen::at(Section::Accounts);
        let (tx, rx) = crossbeam_channel::unbounded();
        screen.checks.insert(key.clone(), CliCheck::Running(rx));
        tx.send(Ok(hosting::demo_account())).unwrap();
        let (_, changed) = accounts_frame(
            &ctx,
            &mut screen,
            &settings,
            std::slice::from_ref(&tab),
            &mut Vec::new(),
        );
        assert_eq!(changed, vec![key.clone()]);
        assert!(matches!(screen.checks.get(&key), Some(CliCheck::Done(_))));
    }

    /// Every preview theme needs an outlined selection marker; merely changing the
    /// card border is not enough to distinguish it from an unselected miniature.
    #[test]
    fn selected_theme_tick_is_outlined_in_every_preview() {
        fn has_outline(shape: &egui::Shape, color: Color32) -> bool {
            match shape {
                egui::Shape::Circle(circle) => {
                    (circle.radius - (TICK / 2.0 + 2.0)).abs() < 0.1 && circle.fill == color
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_outline(shape, color)),
                _ => false,
            }
        }

        for theme_id in ThemeId::ALL {
            for selected in [false, true] {
                let ctx = crate::ui::kit::testkit::ctx_with(
                    ThemeId::Midnight,
                    Style::Standard,
                    Density::Comfortable,
                );
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(400.0, 300.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        theme_card(ui, theme_id, selected, 220.0, None);
                    },
                );
                output.textures_delta.clear();
                let outlined = output
                    .shapes
                    .iter()
                    .any(|clipped| has_outline(&clipped.shape, theme_id.palette().text));
                assert_eq!(outlined, selected, "{theme_id:?}: selected={selected}");
            }
        }
    }

    /// review-views-9: the default clone folder is looked up once while Settings is open, not
    /// on every frame of Integrations or Privacy; a chosen folder is used as typed.
    #[test]
    fn the_default_clone_folder_is_looked_up_once() {
        let mut screen = SettingsScreen::default();
        let mut s = Settings::default();
        assert!(screen.default_clone_parent.is_none());
        let first = screen.clone_parent(&s);
        assert_eq!(screen.default_clone_parent.as_deref(), Some(first.as_str()));
        "  /home/ada/code ".clone_into(&mut s.clone_dir);
        assert_eq!(screen.clone_parent(&s), "/home/ada/code");
    }

    #[test]
    fn narrow_setting_controls_stay_beside_their_help() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let help_bottom = std::cell::Cell::new(0.0);
        let mut control_top = 0.0;
        let mut row_bottom = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(500.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    row_layout(
                        ui,
                        "Name",
                        None,
                        "",
                        &|ui: &mut Ui| {
                            help_bottom.set(ui.label("Used for future commits").rect.bottom());
                        },
                        "",
                        |ui| {
                            control_top = ui.button("Save identity").rect.top();
                        },
                    );
                    row_bottom = ui.cursor().top();
                });
            },
        );
        output.textures_delta.clear();
        assert!(
            control_top <= help_bottom.get() + theme::LG,
            "control top {control_top}, help bottom {}",
            help_bottom.get()
        );
        assert!(row_bottom < 150.0, "row consumed {row_bottom} points");
    }

    #[test]
    fn zoom_changes_only_after_the_drag_ends() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut settings = Settings::default();
        let mut draft = None;
        let mut rect = Rect::NOTHING;
        let mut draw = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 400.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        rect = zoom_slider(ui, &mut settings, &mut draft).rect;
                    });
                },
            );
            output.textures_delta.clear();
            (settings.zoom, rect)
        };
        let (_, rect) = draw(Vec::new());
        let pos = rect.left_center() + vec2(80.0, 0.0);
        let (zoom, _) = draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        assert_eq!(zoom, 1.0, "applying zoom while dragging moves the control");
        let (zoom, _) = draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        }]);
        assert!(zoom > 1.0, "released slider must apply its preview");
    }

    #[test]
    fn typing_a_zoom_percentage_waits_for_enter() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut settings = Settings::default();
        let mut draft = None;
        let mut time = 1.0;
        let mut draw = |events| {
            time += 0.05;
            let mut rect = Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    rect = zoom_slider(ui, &mut settings, &mut draft).rect;
                },
            );
            output.textures_delta.clear();
            (settings.zoom, rect)
        };
        let (_, rect) = draw(Vec::new());
        let pos = rect.right_center() - vec2(16.0, 0.0);
        for _ in 0..2 {
            draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ]);
            draw(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }]);
        }
        assert!(
            ctx.egui_wants_keyboard_input(),
            "zoom value must have editing focus"
        );
        let (zoom, _) = draw(vec![egui::Event::Text("1".into())]);
        assert_eq!(zoom, 1.0, "a partial percentage must not rescale the app");
        draw(vec![egui::Event::Text("50".into())]);
        let (zoom, _) = draw(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]);
        assert_eq!(zoom, 1.5, "Enter must apply the complete percentage");
    }

    /// A11Y-12: Tab onto a control below the visible part of a page scrolls the page to it, so
    /// keyboard focus is never out of sight.
    #[test]
    fn tab_scrolls_the_page_to_the_focused_control() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 560.0));
        let mut settings = Settings::default();
        let mut screen = SettingsScreen::at(Section::Appearance);
        // The clock moves a second a frame, so a scroll's glide ends between frames.
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 1.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    events,
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui.ctx(),
                        &mut screen,
                        &mut settings,
                        &SettingsData {
                            tabs: &[],
                            update: &UpdateState::Idle,
                            identity: IdentityView {
                                repo: None,
                                identity: None,
                                error: None,
                            },
                        },
                        &mut Vec::new(),
                    );
                },
            );
            output.textures_delta.clear();
        };
        for _ in 0..3 {
            frame(Vec::new());
        }
        let tab = |pressed| egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut checked = 0;
        for _ in 0..60 {
            frame(vec![tab(true), tab(false)]);
            // The scroll starts the frame after it is asked for and is read a frame later.
            for _ in 0..4 {
                frame(Vec::new());
            }
            let Some(id) = ctx.memory(egui::Memory::focused) else {
                continue;
            };
            let (Some(focused), Some(body)) = (
                ctx.read_response(id),
                ctx.memory(|m| m.area_rect(egui::Id::new("settings"))),
            ) else {
                continue;
            };
            // Only controls of the page body (right of the navigation, under the header).
            if focused.rect.left() < body.left() + NAV_WIDTH
                || focused.rect.top() < body.top() + theme::metrics().pane_header_height - 1.0
            {
                continue;
            }
            checked += 1;
            assert!(
                focused.rect.bottom() <= body.bottom() + 0.5,
                "focused {id:?} at {:?} is below the card {body:?}",
                focused.rect
            );
        }
        assert!(checked > 10, "Tab reached only {checked} page controls");
    }

    /// usersim2 N3: a row's reset button sits before its control, and is drawn and reached
    /// before it too: Tab and a screen reader meet them in the order they are seen (WCAG 2.4.3).
    #[test]
    fn tab_follows_the_order_of_a_row_with_a_reset_button() {
        let mut met = 0;
        for section in [Section::Git, Section::Appearance] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            ctx.enable_accesskit();
            theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
            let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 4000.0));
            // This test exercises Canopy fallback controls, used when following Git is off.
            let mut settings = Settings {
                initial_branch: "trunk".to_owned(),
                signoff: true,
                commit_page: 77,
                relative_dates: !DEFAULTS.relative_dates,
                style: Style::Deeper,
                prune_on_fetch: !DEFAULTS.prune_on_fetch,
                autostash: !DEFAULTS.autostash,
                syntax_highlight: !DEFAULTS.syntax_highlight,
                high_contrast: !DEFAULTS.high_contrast,
                show_avatars: !DEFAULTS.show_avatars,
                ..Settings {
                    follow_git_config: false,
                    ..Settings::default()
                }
            };
            let mut screen = SettingsScreen::at(section);
            // No git configuration is read: what it sets would decide the rows.
            screen.facts.global.asked = true;
            screen.facts.global.facts = Some(Arc::new(GitFacts::default()));
            let mut time = 0.0;
            let mut frame = |events: Vec<egui::Event>| {
                time += 1.0;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(bounds),
                        events,
                        time: Some(time),
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui.ctx(),
                            &mut screen,
                            &mut settings,
                            &SettingsData {
                                tabs: &[],
                                update: &UpdateState::Idle,
                                identity: IdentityView {
                                    repo: None,
                                    identity: None,
                                    error: None,
                                },
                            },
                            &mut Vec::new(),
                        );
                    },
                );
                output.textures_delta.clear();
                output
            };
            for _ in 0..4 {
                frame(Vec::new());
            }
            let update = frame(Vec::new())
                .platform_output
                .accesskit_update
                .expect("accesskit");
            // Every node in reading order (the order the controls were drawn in).
            let nodes: HashMap<_, _> = update.nodes.iter().cloned().collect();
            let mut order = Vec::new();
            let mut pending = vec![update.tree.as_ref().expect("a tree").root];
            while let Some(id) = pending.pop() {
                order.push(id);
                if let Some(node) = nodes.get(&id) {
                    pending.extend(node.children().iter().rev());
                }
            }
            let place = |label: &str| {
                order.iter().enumerate().find_map(|(i, id)| {
                    let node = nodes.get(id)?;
                    (node.label() == Some(label) && node.bounds().is_some())
                        .then(|| (i, node.bounds().expect("bounds")))
                })
            };
            for node in nodes.values() {
                let Some(name) = node
                    .label()
                    .and_then(|l| l.strip_prefix("Reset "))
                    .and_then(|l| l.strip_suffix(" to recommended"))
                else {
                    continue;
                };
                let (reset, control) = (place(node.label().expect("label")), place(name));
                let (Some((at, reset)), Some((then, control))) = (reset, control) else {
                    continue;
                };
                met += 1;
                assert!(
                    at < then,
                    "{section:?}: {name}: the reset button is read after it"
                );
                assert!(
                    reset.x1 <= control.x0 + 1.0,
                    "{section:?}: {name}: the reset button {reset:?} is not left of {control:?}"
                );
            }
            // And Tab, from the top: the first stop on the First branch row is its reset button.
            if section == Section::Git {
                let tab = |pressed| egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                };
                let mut stops = Vec::new();
                for _ in 0..100 {
                    frame(vec![tab(true), tab(false)]);
                    frame(Vec::new());
                    stops.push(format!("{:?}", ctx.memory(egui::Memory::focused)));
                }
                let at = |needle: &str| stops.iter().position(|s| s.contains(needle));
                let (reset, field) = (at("First branch of a new repository"), at("initial-branch"));
                assert!(
                    matches!((reset, field), (Some(r), Some(f)) if r + 1 == f),
                    "Tab reaches the reset button ({reset:?}) just before the field ({field:?})"
                );
            }
        }
        assert!(met >= 8, "only {met} reset buttons were checked");
    }

    /// usersim2 N2: the first-branch field says as it is typed that git refuses a name, shows
    /// nothing before anything is typed, and keeps only a name git accepts.
    #[test]
    fn the_first_branch_field_refuses_a_name_git_refuses_as_it_is_typed() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.enable_accesskit();
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 4000.0));
        // This test exercises Canopy fallback controls, used when following Git is off.
        let mut settings = Settings {
            follow_git_config: false,
            ..Settings::default()
        };
        let mut screen = SettingsScreen::at(Section::Git);
        // No git configuration is read: what it sets would decide the row.
        screen.facts.global.asked = true;
        screen.facts.global.facts = Some(Arc::new(GitFacts::default()));
        let mut time = 0.0;
        let mut frame = |settings: &mut Settings, events: Vec<egui::Event>| {
            time += 1.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    events,
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui.ctx(),
                        &mut screen,
                        settings,
                        &SettingsData {
                            tabs: &[],
                            update: &UpdateState::Idle,
                            identity: IdentityView {
                                repo: None,
                                identity: None,
                                error: None,
                            },
                        },
                        &mut Vec::new(),
                    );
                },
            );
            output.textures_delta.clear();
            output
        };
        let said = |out: &egui::FullOutput| {
            out.platform_output
                .accesskit_update
                .as_ref()
                .is_some_and(|u| {
                    u.nodes.iter().any(|(_, n)| {
                        [n.label(), n.description(), n.value()]
                            .into_iter()
                            .flatten()
                            .any(|t| t.starts_with("Not a valid branch name"))
                    })
                })
        };
        for _ in 0..4 {
            frame(&mut settings, Vec::new());
        }
        // Tab to the field.
        let tab = |pressed| egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut found = false;
        for _ in 0..100 {
            frame(&mut settings, vec![tab(true), tab(false)]);
            frame(&mut settings, Vec::new());
            if ctx
                .memory(egui::Memory::focused)
                .is_some_and(|id| format!("{id:?}").contains("initial-branch"))
            {
                found = true;
                break;
            }
        }
        assert!(found, "Tab reaches the field");
        let out = frame(&mut settings, Vec::new());
        assert!(!said(&out), "no error before anything is typed");
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let mut typed = |settings: &mut Settings, text: &str| {
            frame(settings, vec![select_all.clone()]);
            frame(settings, vec![egui::Event::Text(text.to_owned())]);
            frame(settings, Vec::new())
        };
        let out = typed(&mut settings, "bad name~");
        assert!(said(&out), "the error shows as the bad name is typed");
        assert_eq!(settings.initial_branch, "main", "a bad name is not kept");
        let out = typed(&mut settings, "-x");
        assert!(said(&out), "a leading dash is refused");
        assert_eq!(settings.initial_branch, "main");
        let out = typed(&mut settings, "trunk");
        assert!(!said(&out), "a good name clears the error");
        assert_eq!(settings.initial_branch, "trunk", "a good name is kept");
        let out = typed(&mut settings, "release/1.0");
        assert!(!said(&out));
        assert_eq!(settings.initial_branch, "release/1.0");
        let out = typed(&mut settings, "");
        assert!(!said(&out), "empty means the default, not an error");
    }

    #[test]
    fn settings_fit_a_compact_zoomed_window() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(562.5, 350.0));
        let mut settings = Settings::default();
        for section in Section::ALL {
            let mut screen = SettingsScreen::at(section);
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(bounds),
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui.ctx(),
                            &mut screen,
                            &mut settings,
                            &SettingsData {
                                tabs: &[],
                                update: &UpdateState::Idle,
                                identity: IdentityView {
                                    repo: None,
                                    identity: None,
                                    error: None,
                                },
                            },
                            &mut Vec::new(),
                        );
                    },
                );
                output.textures_delta.clear();
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("settings")))
                .expect("settings area");
            assert!(
                rect.width() - 2.0 <= (bounds.width() - theme::XL * 2.0).ceil(),
                "{section:?}: modal must retain its viewport margins: {rect:?}"
            );
            assert!(
                bounds.contains_rect(rect),
                "{section:?}: {rect:?} exceeds {bounds:?}"
            );
        }
    }
    /// Draws the whole screen for a few frames at 1280 x 800 and returns it.
    fn draw_screen(screen: SettingsScreen, settings: &mut Settings) -> SettingsScreen {
        draw_screen_in(screen, settings, None).0
    }

    /// Draws Settings with `repo` as the active repository; returns the screen and the actions.
    fn draw_screen_in(
        mut screen: SettingsScreen,
        settings: &mut Settings,
        repo: Option<&Path>,
    ) -> (SettingsScreen, Vec<Action>) {
        let mut actions = Vec::new();
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui.ctx(),
                        &mut screen,
                        settings,
                        &SettingsData {
                            tabs: &[],
                            update: &UpdateState::Idle,
                            identity: IdentityView {
                                repo,
                                identity: None,
                                error: None,
                            },
                        },
                        &mut actions,
                    );
                },
            );
            output.textures_delta.clear();
        }
        (screen, actions)
    }

    /// Where the group heading `group` is drawn, top edge, after opening `screen`.
    fn heading_top(screen: SettingsScreen, group: &str) -> f64 {
        heading_top_with(screen, Settings::default(), group)
    }

    /// [`heading_top`] with `settings` (the theme in them is the one shown).
    fn heading_top_with(mut screen: SettingsScreen, mut settings: Settings, group: &str) -> f64 {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let mut actions = Vec::new();
        let mut top = None;
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui.ctx(),
                        &mut screen,
                        &mut settings,
                        &SettingsData {
                            tabs: &[],
                            update: &UpdateState::Idle,
                            identity: IdentityView {
                                repo: None,
                                identity: None,
                                error: None,
                            },
                        },
                        &mut actions,
                    );
                },
            );
            output.textures_delta.clear();
            top = output
                .platform_output
                .accesskit_update
                .and_then(|u| {
                    u.nodes
                        .into_iter()
                        .map(|(_, n)| n)
                        .find(|n| n.label().or(n.value()) == Some(group))
                })
                .and_then(|n| n.bounds())
                .map(|b| b.y0);
        }
        top.expect("the heading is drawn")
    }

    #[test]
    fn newly_opened_settings_focuses_its_search() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let mut screen = SettingsScreen::default();
        let mut settings = Settings::default();
        let mut actions = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui.ctx(),
                        &mut screen,
                        &mut settings,
                        &SettingsData {
                            tabs: &[],
                            update: &UpdateState::Idle,
                            identity: IdentityView {
                                repo: None,
                                identity: None,
                                error: None,
                            },
                        },
                        &mut actions,
                    );
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            Some(crate::ui::kit::field::edit_id(search_id())),
            "Settings must take keyboard focus when opened"
        );
    }

    /// The section list and the theme cards are each one Tab stop, however many entries they
    /// hold; the arrows move inside them.
    #[test]
    fn the_section_list_and_the_theme_cards_are_one_tab_stop_each() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let mut screen = SettingsScreen::default();
        let mut settings = Settings::default();
        let mut frame = |events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui.ctx(),
                        &mut screen,
                        &mut settings,
                        &SettingsData {
                            tabs: &[],
                            update: &UpdateState::Idle,
                            identity: IdentityView {
                                repo: None,
                                identity: None,
                                error: None,
                            },
                        },
                        &mut Vec::new(),
                    );
                },
            );
            output.textures_delta.clear();
        };
        let key = |key| crate::ui::kit::testkit::key(key);
        for _ in 0..3 {
            frame(Vec::new());
        }
        let mut stops = HashSet::new();
        let focused_rect = |ctx: &egui::Context| {
            ctx.memory(egui::Memory::focused)
                .and_then(|id| ctx.read_response(id))
                .map(|r| (r.id, r.rect))
        };
        for _ in 0..40 {
            frame(key(egui::Key::Tab));
            frame(Vec::new());
            if let Some((id, rect)) = focused_rect(&ctx) {
                stops.insert((id, rect.left() < 240.0, rect.height() > 80.0));
            }
        }
        let in_list = stops.iter().filter(|(_, nav, _)| *nav).count();
        let cards = stops.iter().filter(|(_, _, card)| *card).count();
        assert_eq!(in_list, 1, "the section list is one stop: {stops:?}");
        assert_eq!(cards, 1, "the theme cards are one stop: {stops:?}");
    }

    #[test]
    fn settings_opened_at_a_group_shows_its_heading() {
        // Tooltips is below the initial Appearance fold. Once Notifications moved
        // to its own page, less content follows Tooltips: the scroll area cannot
        // always align it to the very top, but it must bring it into the viewport.
        let plain = heading_top(SettingsScreen::at(Section::Appearance), "Tooltips");
        assert!(plain > 800.0, "below the fold by default: {plain}");
        let revealed = heading_top(
            SettingsScreen::at_group(Section::Appearance, "Tooltips"),
            "Tooltips",
        );
        assert!(
            (0.0..700.0).contains(&revealed) && revealed < plain,
            "group heading should be visible after scrolling: {revealed} (initially {plain})"
        );
    }

    fn fact(key: &str, value: &str) -> ConfigValue {
        ConfigValue {
            key: key.to_owned(),
            value: value.to_owned(),
            scope: "global".to_owned(),
            origin: "file:/home/me/.gitconfig".to_owned(),
            file: Some(PathBuf::from("/home/me/.gitconfig")),
        }
    }

    #[test]
    fn the_repository_page_is_there_only_with_a_repository() {
        let mut settings = Settings::default();
        let repo = Path::new("/work/canopy");
        let (with, _) = draw_screen_in(
            SettingsScreen::at(Section::Repository),
            &mut settings,
            Some(repo),
        );
        assert_eq!(with.section, Section::Repository);
        let (without, _) =
            draw_screen_in(SettingsScreen::at(Section::Repository), &mut settings, None);
        assert_ne!(
            without.section,
            Section::Repository,
            "with no repository open, Settings opens elsewhere"
        );
        // A search covers the repository's settings too, and only while one is open.
        let (found, _) = draw_screen_in(
            SettingsScreen::searching("pull strategy"),
            &mut settings,
            Some(repo),
        );
        assert!(
            found.search.count(Section::Repository) >= 1,
            "{:?}",
            found.search
        );
        assert!(found.search.count(Section::Git) >= 1, "{:?}", found.search);
        let (alone, _) = draw_screen_in(
            SettingsScreen::searching("pull strategy"),
            &mut settings,
            None,
        );
        assert_eq!(alone.search.count(Section::Repository), 0);
    }

    #[test]
    fn drawing_the_repository_page_changes_nothing() {
        let mut settings = Settings::default();
        let repo = Path::new("/work/canopy");
        settings.repo_mut(repo).signoff = Some(true);
        let before = settings.clone();
        let (_, actions) = draw_screen_in(
            SettingsScreen::at(Section::Repository),
            &mut settings,
            Some(repo),
        );
        assert_eq!(settings.repos, before.repos);
        assert_eq!(settings.repo_diff, before.repo_diff);
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::SetRepoDiff { .. })),
            "{actions:?}"
        );
    }

    /// ST-F1, Override O3: pulls pass a strategy, so the note says which one applies and, when
    /// Canopy's own choice differs from the configuration, that the configuration does not.
    #[test]
    fn notes_say_which_pull_strategy_applies() {
        use canopy_core::{ConfigScope, ConfigSource, PullConfig, PullStrategy};
        let follow = PullChoice::default();
        let own = |mode| PullChoice {
            mode,
            follow_config: false,
        };
        let mut facts = GitFacts::default();
        assert_eq!(pull_note(&facts, follow), None, "not read");
        facts.pull = Some(PullConfig::default());
        assert_eq!(
            pull_note(&facts, follow),
            None,
            "nothing to say when nothing is set"
        );
        assert_eq!(pull_note(&facts, own(PullMode::Rebase)), None);

        facts.pull = Some(PullConfig {
            strategy: Some(PullStrategy::Rebase),
            sources: vec![ConfigSource {
                key: "pull.rebase".into(),
                value: "true".into(),
                scope: ConfigScope::Global,
                origin: "file:/home/me/.gitconfig".into(),
                file: None,
            }],
            invalid: Vec::new(),
        });
        let followed = pull_note(&facts, follow).unwrap();
        assert!(!followed.warn);
        assert_eq!(
            followed.text,
            "Your git configuration decides: Rebase when pulling set to rebase in your global \
             git config (/home/me/.gitconfig), so pulls use rebase."
        );
        assert!(!pull_note(&facts, own(PullMode::Rebase)).unwrap().warn);
        let overridden = pull_note(&facts, own(PullMode::Merge)).unwrap();
        assert!(overridden.warn);
        assert_eq!(
            overridden.text,
            "Canopy's pulls use this choice instead of your git configuration: Rebase when \
             pulling set to rebase in your global git config (/home/me/.gitconfig)."
        );
    }

    #[test]
    fn notification_preferences_have_their_own_page_and_search_section() {
        assert!(Section::ALL.contains(&Section::Notifications));
        assert_eq!(Section::Notifications.label().1, "Notifications");
        let mut settings = Settings::default();
        settings.notifications.finished = true;
        settings.notifications.errors = false;
        settings.notifications.confirmations = true;
        let original = settings.notifications;
        let page = draw_screen(SettingsScreen::at(Section::Notifications), &mut settings);
        assert_eq!(page.section, Section::Notifications);
        assert_eq!(
            settings.notifications, original,
            "opening the page must not reset filters"
        );
        let matches = draw_screen(
            SettingsScreen::searching("Finished operations"),
            &mut settings,
        );
        assert!(matches.search.count(Section::Notifications) > 0);
        assert_eq!(matches.search.count(Section::Appearance), 0);
        assert_eq!(settings.notifications, original);
    }

    #[test]
    fn backup_reset_and_consent_are_found() {
        let mut settings = Settings::default();
        let found = |query: &str, settings: &mut Settings| {
            draw_screen(SettingsScreen::searching(query), settings).search
        };
        let backup = found("backup", &mut settings);
        assert!(backup.count(Section::Data) >= 3, "{backup:?}");
        let defaults = found("factory defaults", &mut settings);
        assert_eq!(defaults.count(Section::Data), 1, "{defaults:?}");
        let consent = found("consent", &mut settings);
        assert!(consent.count(Section::Privacy) >= 2, "{consent:?}");
    }

    #[test]
    fn a_setting_goes_back_to_its_default_only_when_asked() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut value = 7_u32;
        let frame = |events: Vec<egui::Event>, value: &mut u32| {
            let mut clicked = false;
            let mut button = Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    clicked = revert(ui, value, &3, "Test");
                    button = ui.min_rect();
                },
            );
            output.textures_delta.clear();
            (clicked, button)
        };
        let (clicked, button) = frame(Vec::new(), &mut value);
        assert!(!clicked, "drawing alone changes nothing");
        assert_eq!(value, 7);
        let at = button.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at), press(true)], &mut value);
        let (clicked, _) = frame(vec![press(false)], &mut value);
        assert!(clicked, "the button at {button:?} was clicked");
        assert_eq!(value, 3, "back to the default");
        // At the default there is nothing to go back to.
        let (_, after) = frame(Vec::new(), &mut value);
        assert!(after.width() <= 0.0, "no button at the default: {after:?}");
    }

    #[test]
    fn privacy_draws_why_pictures_are_missing() {
        let mut settings = Settings::default();
        let mut screen = SettingsScreen::at(Section::Privacy);
        screen.set_picture_note(Some(PictureNote {
            title: "No sign-in for github.com".to_owned(),
            detail: "Sign in under Accounts to load authors' pictures.".to_owned(),
            open_accounts: true,
        }));
        let screen = draw_screen(screen, &mut settings);
        assert_eq!(screen.section, Section::Privacy, "nothing was clicked");
        assert!(screen.picture_note.is_some());
        // The new wording is what a search finds.
        let found = draw_screen(SettingsScreen::searching("remembered"), &mut settings);
        assert_eq!(
            found.search.count(Section::Privacy),
            1,
            "{:?}",
            found.search
        );
    }

    #[test]
    fn a_saved_command_shows_as_the_app_it_starts() {
        use crate::system_apps::App;
        let app = |name: &str, command: &str| App {
            kind: Kind::Editor,
            name: name.to_owned(),
            command: command.to_owned(),
            id: command.to_owned(),
        };
        let (zed, code) = (app("Zed", "zed"), app("Code", "code %f"));
        let installed = [&zed, &code];
        assert_eq!(
            launch_choice("  ", false, &installed, &[], false),
            LaunchChoice::System
        );
        assert_eq!(
            launch_choice("code %f", false, &installed, &[], false),
            LaunchChoice::App(1)
        );
        assert_eq!(
            launch_choice(" zed ", false, &installed, &[], false),
            LaunchChoice::App(0)
        );
        assert_eq!(
            launch_choice("zed --wait", false, &installed, &[], false),
            LaunchChoice::Custom
        );
        assert_eq!(
            launch_choice("zed", true, &installed, &[], false),
            LaunchChoice::Custom,
            "an open command field stays open"
        );
    }

    #[test]
    fn repository_diff_defaults_can_be_pinned_and_reset_together() {
        let ctx = crate::ui::kit::testkit::ctx();
        ctx.enable_accesskit();
        let mut initial = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 900.0))),
                ..Default::default()
            },
            |_| {},
        );
        initial.textures_delta.clear();
        let mut settings = Settings::default();
        settings.diff.context = 4;
        let path = Path::new("fixture");
        let mut actions = Vec::new();
        let frame = |key, settings: &Settings, actions: &mut Vec<Action>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events: crate::ui::kit::testkit::key(key),
                    ..Default::default()
                },
                |ui| repo_diff_rows(ui, settings, path, actions),
            );
            out.textures_delta.clear();
            out.platform_output.accesskit_update.expect("tree")
        };
        let update = frame(egui::Key::Tab, &settings, &mut actions);
        let context = update
            .nodes
            .iter()
            .map(|(_, n)| n)
            .find(|n| {
                n.role() == egui::accesskit::Role::ComboBox && n.label() == Some("Context lines")
            })
            .expect("context selector");
        assert_eq!(
            context.value(),
            Some("4 lines"),
            "the inherited value is shown"
        );
        crate::ui::kit::testkit::frame(&ctx, Vec::new(), |ui| {
            repo_diff_rows(ui, &settings, path, &mut actions);
        });
        frame(egui::Key::ArrowDown, &settings, &mut actions);
        crate::ui::kit::testkit::frame(&ctx, Vec::new(), |ui| {
            repo_diff_rows(ui, &settings, path, &mut actions);
        });
        frame(egui::Key::Enter, &settings, &mut actions);
        assert!(actions.iter().any(|a| matches!(a, Action::SetRepoDiff { prefs: Some(prefs), .. } if *prefs == settings.diff)), "the default diff values can be pinned");
        settings.repo_diff.insert(path.to_path_buf(), settings.diff);
        actions.clear();
        let update = frame(egui::Key::Tab, &settings, &mut actions);
        let context = update
            .nodes
            .iter()
            .map(|(_, n)| n)
            .find(|n| {
                n.role() == egui::accesskit::Role::ComboBox && n.label() == Some("Context lines")
            })
            .expect("pinned context selector");
        assert_eq!(
            context.value(),
            Some("4 lines"),
            "the pinned value is shown"
        );
        let mut reached = false;
        for _ in 0..15 {
            let update = frame(egui::Key::Tab, &settings, &mut actions);
            if update
                .nodes
                .iter()
                .any(|(id, n)| *id == update.focus && n.label() == Some("Use default diff options"))
            {
                frame(egui::Key::Enter, &settings, &mut actions);
                reached = true;
                break;
            }
        }
        assert!(reached, "the shared diff reset is keyboard reachable");
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::SetRepoDiff { prefs: None, .. }))
        );
    }

    #[test]
    fn repository_selects_offer_each_value_once_and_pin_the_default() {
        let ctx = crate::ui::kit::testkit::ctx();
        ctx.enable_accesskit();
        let mut initial = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 900.0))),
                ..Default::default()
            },
            |_| {},
        );
        initial.textures_delta.clear();
        let mut value = None;
        let draw = |ui: &mut Ui, value: &mut Option<bool>| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                override_select(
                    ui,
                    "sample-own",
                    "Sample setting",
                    value,
                    true,
                    true,
                    &on_off(),
                );
            });
        };
        let frame = |key, value: &mut Option<bool>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events: crate::ui::kit::testkit::key(key),
                    ..Default::default()
                },
                |ui| draw(ui, value),
            );
            out.textures_delta.clear();
            out.platform_output.accesskit_update.expect("tree")
        };
        for _ in 0..3 {
            crate::ui::kit::testkit::frame(&ctx, Vec::new(), |ui| draw(ui, &mut value));
        }
        frame(egui::Key::Tab, &mut value);
        crate::ui::kit::testkit::frame(&ctx, Vec::new(), |ui| draw(ui, &mut value));
        let update = frame(egui::Key::ArrowDown, &mut value);
        let options = update
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::ListBoxOption)
            .collect::<Vec<_>>();
        assert_eq!(options.len(), 2, "one entry per value");
        assert!(
            options
                .iter()
                .any(|(_, n)| n.label() == Some("On") && n.description() == Some("Recommended"))
        );
        assert!(options.iter().any(|(_, n)| n.label() == Some("Off")));
        crate::ui::kit::testkit::frame(&ctx, Vec::new(), |ui| draw(ui, &mut value));
        frame(egui::Key::Home, &mut value);
        crate::ui::kit::testkit::frame(&ctx, Vec::new(), |ui| draw(ui, &mut value));
        frame(egui::Key::Enter, &mut value);
        assert_eq!(value, Some(true), "selecting the effective default pins it");
        let mut reached = false;
        for _ in 0..10 {
            let update = frame(egui::Key::Tab, &mut value);
            if update.nodes.iter().any(|(id, n)| {
                *id == update.focus && n.label() == Some("Use the default for Sample setting (On)")
            }) {
                frame(egui::Key::Enter, &mut value);
                reached = true;
                break;
            }
        }
        assert!(reached, "reset has a unique keyboard name");
        assert_eq!(value, None, "reset follows the default again");
    }

    #[test]
    fn notes_say_when_the_git_configuration_decides() {
        let mut facts = GitFacts::default();
        assert_eq!(prune_note(&facts, false, true), None, "nothing set");
        facts.fetch_prune = Some(fact("fetch.prune", "true"));
        let recommended = prune_note(&facts, false, true).expect("a note");
        assert!(!recommended.warn);
        assert!(
            recommended
                .text
                .starts_with("Your git configuration decides: Prune remote branches set to on")
                && !recommended.text.contains("Recommended"),
            "{}",
            recommended.text
        );
        assert_eq!(prune_note(&facts, true, true), Some(recommended));
        facts.prune_tags = vec![fact("fetch.pruneTags", "true")];
        let tags = prune_tags_note(&facts, false, true).expect("a note");
        assert!(!tags.warn);
        assert!(
            tags.text.contains("Prune local tags set to on") && !tags.text.contains("Recommended:"),
            "{}",
            tags.text
        );
        // Not followed, the same value is one Canopy's choice replaces.
        let replaced = prune_tags_note(&facts, false, false).expect("a note");
        assert!(replaced.warn);
        assert!(
            replaced
                .text
                .starts_with("Canopy's fetches use this choice"),
            "{}",
            replaced.text
        );
    }

    /// With only one of the two autostash keys set, the note says which kind of operation
    /// Canopy's switch still decides.
    #[test]
    fn the_autostash_note_names_what_canopys_switch_still_decides() {
        let mut facts = GitFacts {
            autostash: vec![fact("rebase.autoStash", "false")],
            ..GitFacts::default()
        };
        let note = autostash_note(&facts, true, true).expect("a note");
        assert!(note.text.ends_with("decides merges."), "{}", note.text);
        facts.autostash = vec![fact("merge.autoStash", "true")];
        let note = autostash_note(&facts, true, true).expect("a note");
        assert!(note.text.ends_with("decides rebases."), "{}", note.text);
        facts.autostash.push(fact("rebase.autoStash", "true"));
        let note = autostash_note(&facts, true, true).expect("a note");
        assert!(!note.text.contains("switch"), "{}", note.text);
    }

    /// When Canopy does not follow the git configuration it passes its pruning choice, so a
    /// configuration that differs is reported as not applying, either way round.
    #[test]
    fn notes_say_when_the_prune_choice_overrides_git() {
        let mut facts = GitFacts::default();
        assert_eq!(prune_note(&facts, false, false), None);
        facts.fetch_prune = Some(fact("fetch.prune", "true"));
        let off = prune_note(&facts, false, false).unwrap();
        assert!(off.warn);
        assert!(
            off.text.starts_with("Canopy's fetches use this choice")
                && off.text.contains("set to on"),
            "{}",
            off.text
        );
        assert!(!prune_note(&facts, true, false).unwrap().warn);
        facts.fetch_prune = Some(fact("fetch.prune", "false"));
        assert!(!prune_note(&facts, false, false).unwrap().warn);
        let on = prune_note(&facts, true, false).unwrap();
        assert!(on.warn);
        assert!(on.text.contains("set to off"), "{}", on.text);
    }

    fn signing(format: &str, key: Option<&str>, problem: Option<SigningProblem>) -> GitFacts {
        GitFacts {
            gpg_sign: Some(fact("commit.gpgsign", "true")),
            signing: Some(Signing {
                format: format.to_owned(),
                key: key.map(str::to_owned),
                program: match format {
                    "ssh" => "/usr/bin/ssh-keygen".to_owned(),
                    "x509" => "gpgsm".to_owned(),
                    _ => "gpg".to_owned(),
                },
                problem,
            }),
            ..GitFacts::default()
        }
    }

    /// Off, on and broken read differently in words, not only in colour.
    #[test]
    fn the_signing_row_tells_off_on_and_broken_apart() {
        let off = signing_look(&GitFacts::default());
        assert_eq!((off.badge, off.tone), ("Not signing", Tone::Neutral));
        assert!(
            off.help.starts_with("Commits are not signed here. "),
            "{}",
            off.help
        );
        assert_eq!(off.note, None);

        let explicit = GitFacts {
            gpg_sign: Some(fact("commit.gpgsign", "false")),
            ..GitFacts::default()
        };
        let note = signing_look(&explicit).note.expect("where it is set");
        assert!(!note.warn);
        assert!(
            note.text
                .starts_with("Sign commits set to off in your global git config"),
            "{}",
            note.text
        );

        let on = signing_look(&signing("openpgp", Some("3AA5C34371567BD2"), None));
        assert_eq!((on.badge, on.tone), ("Signing", Tone::Success));
        for part in ["OpenPGP", "key 3AA5C34371567BD2", "by gpg"] {
            assert!(on.help.contains(part), "{part}: {}", on.help);
        }
        assert!(
            on.note.is_some_and(|note| !note.warn),
            "where it is turned on"
        );

        let ssh = signing_look(&signing("ssh", Some("id.pub"), None));
        assert!(
            ssh.help.contains("SSH with key id.pub, by ssh-keygen"),
            "{}",
            ssh.help
        );
        assert!(!ssh.help.contains("/usr/bin"), "{}", ssh.help);

        let x509 = signing_look(&signing("x509", None, None));
        assert!(x509.help.contains("X.509"), "{}", x509.help);
        assert!(x509.help.contains("default key"), "{}", x509.help);
    }

    #[test]
    fn a_signing_problem_is_a_warning_with_its_fix() {
        let missing = signing_look(&signing(
            "ssh",
            Some("id.pub"),
            Some(SigningProblem::ProgramMissing),
        ));
        assert_eq!((missing.badge, missing.tone), ("Will fail", Tone::Danger));
        let note = missing.note.expect("a warning");
        assert!(note.warn);
        assert!(
            note.text.contains("ssh-keygen is not installed")
                && note.text.contains("another signing program"),
            "{}",
            note.text
        );
        for (problem, part) in [
            (
                SigningProblem::KeyNotSet,
                "needs a signing key and none is set",
            ),
            (SigningProblem::KeyFileMissing, "id.pub does not exist"),
            (
                SigningProblem::UnknownFormat,
                "is not a signing format git knows",
            ),
        ] {
            let look = signing_look(&signing("ssh", Some("id.pub"), Some(problem)));
            let text = look.note.expect("a warning").text;
            assert!(text.contains(part), "{part}: {text}");
        }
    }

    /// A remote's own `remote.<name>.prune` is named beside `fetch.prune`, and replaces it once
    /// every remote sets its own.
    #[test]
    fn notes_name_a_remote_s_own_prune_setting() {
        let remote = |name: &str| crate::repo_config::Remote {
            name: name.to_owned(),
            url: format!("https://example.com/{name}.git"),
        };
        let mut facts = GitFacts {
            remotes: vec![remote("origin")],
            remote_prune: vec![fact("remote.origin.prune", "true")],
            ..GitFacts::default()
        };
        let off = prune_note(&facts, false, false).expect("a note");
        assert!(off.warn);
        assert!(
            off.text
                .contains("Prune remote branches for origin set to on"),
            "{}",
            off.text
        );
        assert!(
            off.text.contains("instead of your git configuration"),
            "{}",
            off.text
        );
        assert!(!prune_note(&facts, true, false).expect("a note").warn);

        // Every remote sets its own, so fetch.prune decides nothing and is not named.
        facts.fetch_prune = Some(fact("fetch.prune", "false"));
        let on = prune_note(&facts, true, false).expect("a note");
        assert!(!on.warn, "{}", on.text);
        assert!(!on.text.contains("fetch.prune"), "{}", on.text);

        // A second remote without its own setting brings fetch.prune back.
        facts.remotes.push(remote("mirror"));
        let on = prune_note(&facts, true, false).expect("a note");
        assert!(on.warn);
        assert!(
            on.text.contains("Prune remote branches set to off"),
            "{}",
            on.text
        );
        assert!(!on.text.contains("for origin"), "{}", on.text);
        let off = prune_note(&facts, false, false).expect("a note");
        assert!(off.warn);
        assert!(
            off.text
                .contains("Prune remote branches for origin set to on")
                && !off.text.contains("set to off"),
            "{}",
            off.text
        );
        facts.remote_prune.push(fact("remote.mirror.prune", "true"));
        let off = prune_note(&facts, false, false).expect("a note");
        assert!(
            off.text
                .contains("Prune remote branches for origin set to on in ")
                && off
                    .text
                    .contains(" and Prune remote branches for mirror set to on in "),
            "{}",
            off.text
        );
    }

    #[test]
    fn a_search_finds_rows_by_title_help_synonym_and_section() {
        let mut settings = Settings::default();
        let found = |query: &str, settings: &mut Settings| {
            draw_screen(SettingsScreen::searching(query), settings).search
        };
        // A synonym: nothing says "rebase" but the pull strategy and its shortcuts.
        let rebase = found("rebase", &mut settings);
        assert!(rebase.count(Section::Git) >= 1, "{rebase:?}");
        assert_eq!(rebase.count(Section::Appearance), 0, "{rebase:?}");
        // Help text, any case, words in any order.
        let terminal = found("TERMINAL here", &mut settings);
        assert_eq!(terminal.count(Section::Integrations), 1, "{terminal:?}");
        // The theme cards are found by a theme's name.
        let observatory = found("observatory", &mut settings);
        assert_eq!(observatory.count(Section::Appearance), 1, "{observatory:?}");
        // A section's name finds all of its rows.
        let updates = found("updates", &mut settings);
        assert!(updates.count(Section::Updates) >= 3, "{updates:?}");
        // Shortcuts are searched like the cheatsheet does.
        let push = found("push", &mut settings);
        assert!(push.count(Section::Keyboard) >= 1, "{push:?}");
        // Every word must match.
        let none = found("theme rebase", &mut settings);
        assert_eq!(none.total(), 0, "{none:?}");
        assert_eq!(found("qqqzzz", &mut settings).total(), 0);
    }

    /// #229: the Diagnostics rows are found by the words their new copy and synonyms use.
    #[test]
    fn a_search_finds_the_diagnostics_rows_by_their_words() {
        let mut settings = Settings::default();
        for (query, rows) in [
            ("RUST_LOG", 1),
            ("everything libraries", 1),
            ("crashes folder", 1),
            ("clipboard", 1),
            ("log detail", 2),
            ("crash files", 1),
        ] {
            let found = draw_screen(SettingsScreen::searching(query), &mut settings).search;
            assert_eq!(found.count(Section::Privacy), rows, "{query}: {found:?}");
        }
    }

    #[test]
    fn escape_clears_a_search_before_it_closes_settings() {
        let mut screen = SettingsScreen::searching("zoom");
        assert!(
            screen.dismiss_search(),
            "the first Escape clears the search"
        );
        assert_eq!(screen.search.query, "");
        assert!(
            !screen.dismiss_search(),
            "with nothing searched, Escape closes"
        );
        let mut blank = SettingsScreen::searching("   ");
        assert!(!blank.dismiss_search(), "spaces are not a search");
    }

    /// #195: connective words neither match on their own nor get marked; a query made only of
    /// them is still searched as typed.
    #[test]
    fn a_search_skips_connective_words() {
        assert_eq!(search_words("Pull AND merge"), ["pull", "merge"]);
        assert_eq!(
            search_words("the  for to of or"),
            ["the", "for", "to", "of", "or"]
        );
        let mut settings = Settings::default();
        let found = |query: &str, settings: &mut Settings| {
            draw_screen(SettingsScreen::searching(query), settings).search
        };
        assert_eq!(
            found("pull and merge", &mut settings).total(),
            found("pull merge", &mut settings).total()
        );
    }

    #[test]
    fn found_words_are_marked_in_titles() {
        let font = FontId::proportional(13.0);
        let mark = Color32::RED;
        let job = marked(
            "Pull strategy",
            &search_words("PULL str"),
            &font,
            Color32::WHITE,
            Color32::BLUE,
            mark,
        );
        let parts: Vec<(&str, bool)> = job
            .sections
            .iter()
            .map(|s| {
                (
                    job.text
                        .get(s.byte_range.start.0..s.byte_range.end.0)
                        .unwrap_or_default(),
                    s.format.background == mark,
                )
            })
            .collect();
        assert_eq!(
            parts,
            [
                ("Pull", true),
                (" ", false),
                ("str", true),
                ("ategy", false)
            ]
        );
        let none = marked("Zoom", &[], &font, Color32::WHITE, Color32::BLUE, mark);
        assert_eq!(none.sections.len(), 1);
        assert_eq!(none.text, "Zoom");
    }

    /// qa-b F13: at 1024 x 700 the shell covered the title bar's tab strip and the status bar.
    #[test]
    fn the_shell_stays_clear_of_the_tab_strip_and_the_status_bar() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let m = theme::metrics();
        let mut settings = Settings::default();
        for size in [
            vec2(1024.0, 700.0),
            vec2(1440.0, 900.0),
            vec2(1920.0, 1080.0),
        ] {
            let bounds = Rect::from_min_size(egui::Pos2::ZERO, size);
            let mut screen = SettingsScreen::default();
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(bounds),
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui.ctx(),
                            &mut screen,
                            &mut settings,
                            &SettingsData {
                                tabs: &[],
                                update: &UpdateState::Idle,
                                identity: IdentityView {
                                    repo: None,
                                    identity: None,
                                    error: None,
                                },
                            },
                            &mut Vec::new(),
                        );
                    },
                );
                output.textures_delta.clear();
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("settings")))
                .expect("settings area");
            assert!(
                rect.top() >= m.tab_strip_height,
                "{size:?}: the shell's top {} covers the tab strip ({})",
                rect.top(),
                m.tab_strip_height
            );
            assert!(
                rect.bottom() <= size.y - m.status_bar_height,
                "{size:?}: the shell's bottom {} covers the status bar",
                rect.bottom()
            );
        }
    }

    fn field(value: &str, source: Option<Source>) -> Field {
        Field {
            value: value.into(),
            source,
        }
    }

    fn identity(name: Field, email: Field) -> Identity {
        let person = canopy_core::identity::Person {
            name,
            email,
            usable: true,
        };
        Identity {
            author: person.clone(),
            committer: person,
        }
    }

    fn local_source() -> Source {
        Source::Config {
            scope: "local".into(),
            origin: "file:.git/config".into(),
        }
    }

    #[test]
    fn sources_say_where_a_value_comes_from() {
        let home = Path::new("/home/ada");
        let label = |source: &Source| source_label_in(Some(source), home);
        assert_eq!(
            label(&Source::Environment("GIT_AUTHOR_NAME")),
            "the GIT_AUTHOR_NAME environment variable"
        );
        assert_eq!(
            label(&Source::Config {
                scope: "command".into(),
                origin: "command line:".into(),
            }),
            "the git command line"
        );
        assert_eq!(label(&local_source()), "this repository (.git/config)");
        assert_eq!(
            label(&Source::Config {
                scope: "global".into(),
                origin: "file:/home/ada/.gitconfig".into(),
            }),
            "your global git config (~/.gitconfig)"
        );
        assert_eq!(
            label(&Source::Config {
                scope: "system".into(),
                origin: "file:/etc/gitconfig".into(),
            }),
            "the system git config (/etc/gitconfig)"
        );
        assert_eq!(
            label(&Source::Config {
                scope: String::new(),
                origin: "file:/home/ada/.config/git/work".into(),
            }),
            "~/.config/git/work"
        );
        assert_eq!(label(&Source::Guessed), "guessed by Git from this computer");
        assert_eq!(source_label_in(None, home), "not set");
    }

    #[test]
    fn a_long_path_is_cut_in_the_middle_to_the_width() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let path =
            "From global: /home/ada/projects/scratch/canopy-polish/tmpdir/gitconfig";
        let font = FontId::proportional(theme::metrics().small_size);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                assert_eq!(elide_middle(ui, path, &font, 10_000.0), path);
                let cut = elide_middle(ui, path, &font, 200.0);
                assert!(cut.contains('…') && cut.starts_with("From") && cut.ends_with("gitconfig"));
                let width = ui
                    .painter()
                    .layout_no_wrap(cut, font.clone(), Color32::WHITE)
                    .size()
                    .x;
                assert!(width <= 200.0, "{width}");
                assert_eq!(elide_middle(ui, path, &font, 0.0), "…");
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn an_unknown_home_directory_is_never_shown_as_a_tilde() {
        let source = Source::Config {
            scope: String::new(),
            origin: "file:/etc/gitconfig".into(),
        };
        assert_eq!(
            source_label_in(Some(&source), Path::new("")),
            "/etc/gitconfig"
        );
    }

    #[test]
    fn the_editor_offers_chosen_values_and_never_a_guess() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut settings = Settings::default();
        let mut screen = SettingsScreen::default();
        let repo = PathBuf::from("/repo");
        let mut draw = |screen: &mut SettingsScreen, repo: Option<&Path>, id: &Identity| {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    git(
                        ui,
                        screen,
                        &mut settings,
                        &IdentityView {
                            repo,
                            identity: Some(id),
                            error: None,
                        },
                        &mut actions,
                    );
                });
            });
            output.textures_delta.clear();
            actions
        };

        let guessed = identity(
            field("someone", Some(Source::Guessed)),
            field("someone@host.(none)", Some(Source::Guessed)),
        );
        draw(&mut screen, Some(&repo), &guessed);
        assert!(
            screen.identity_name.is_empty() && screen.identity_email.is_empty(),
            "a value Git made up must not be offered as if someone chose it"
        );
        assert_eq!(screen.identity_scope, Scope::Local);

        let chosen = identity(
            field("Ada Lovelace", Some(local_source())),
            field("ada@example.com", Some(local_source())),
        );
        let actions = draw(&mut screen, Some(&repo), &chosen);
        assert_eq!(screen.identity_name, "Ada Lovelace");
        assert_eq!(screen.identity_email, "ada@example.com");
        assert!(actions.is_empty(), "drawing alone changes nothing");

        draw(&mut screen, None, &chosen);
        assert_eq!(
            screen.identity_scope,
            Scope::Global,
            "without a repository only the global configuration can be written"
        );
    }
    /// The Git page drawn with `facts` as the global configuration and Edit my git config on (the
    /// read-only page: `the_git_configuration_is_read_only_until_editing_is_turned_on`); the
    /// AccessKit nodes it spoke.
    fn git_page(facts: GitFacts, follow: bool) -> Vec<crate::ui::kit::testkit::Spoken> {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let mut settings = Settings {
            follow_git_config: follow,
            edit_git_config: true,
            ..Settings::default()
        };
        let mut screen = SettingsScreen::default();
        screen.facts.global.asked = true;
        screen.facts.global.facts = Some(Arc::new(facts));
        crate::ui::kit::testkit::spoken(&ctx, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                git(
                    ui,
                    &mut screen,
                    &mut settings,
                    &IdentityView {
                        repo: None,
                        identity: None,
                        error: None,
                    },
                    &mut Vec::new(),
                );
            });
        })
    }

    /// #158 deliberately replaces the former locked controls with editable Git values.
    #[test]
    fn configured_git_choices_stay_editable() {
        use canopy_core::{ConfigScope, ConfigSource, PullConfig, PullStrategy};
        let mut facts = GitFacts {
            pull: Some(PullConfig {
                strategy: Some(PullStrategy::Rebase),
                sources: vec![ConfigSource {
                    key: "pull.rebase".into(),
                    value: "true".into(),
                    scope: ConfigScope::Global,
                    origin: "file:/home/me/.gitconfig".into(),
                    file: None,
                }],
                invalid: Vec::new(),
            }),
            fetch_prune: Some(fact("fetch.prune", "true")),
            conflict_style: Some(fact("merge.conflictStyle", "merge")),
            ..GitFacts::default()
        };
        facts.values = vec![
            fact("pull.rebase", "true"),
            fact("fetch.prune", "true"),
            fact("merge.conflictStyle", "merge"),
        ];
        let nodes = git_page(facts, true);
        for name in [
            "Rebase when pulling",
            "Prune remote branches",
            "Conflict markers",
        ] {
            let node = crate::ui::kit::testkit::find(&nodes, name)
                .unwrap_or_else(|| panic!("{name} is on the page"));
            assert!(!node.disabled, "{name} can change the configuration");
        }
        assert!(
            nodes
                .iter()
                .filter_map(|node| node.value.as_deref())
                .any(|text| text.contains("your global git config")),
            "the source is available: {nodes:?}"
        );
    }
    /// The editable select shows the configured value, rather than a stored fallback.
    #[test]
    fn a_choice_the_git_configuration_decides_shows_its_value() {
        let mut facts = GitFacts {
            fetch_prune: Some(fact("fetch.prune", "false")),
            follow_tags: Some(fact("push.followTags", "false")),
            ..GitFacts::default()
        };
        facts.values = vec![
            fact("fetch.prune", "false"),
            fact("push.followTags", "false"),
        ];
        let nodes = git_page(facts, true);
        for name in ["Prune remote branches", "Push annotated tags"] {
            let node = crate::ui::kit::testkit::find(&nodes, name).expect(name);
            assert!(!node.disabled, "{name}");
            assert_eq!(node.value.as_deref(), Some("Off"), "{name} is off in git");
        }
        // Canopy's own choice for both is on.
        assert!(Settings::default().prune_on_fetch && Settings::default().follow_tags);
    }

    #[test]
    fn git_setting_help_does_not_type_the_recommendation() {
        for setting in Setting::ALL {
            assert!(
                !git_setting_help(setting).is_empty(),
                "{setting:?} has no help"
            );
            assert!(
                !git_setting_help(setting).contains("Recommended"),
                "{setting:?}: the list marks the recommended option"
            );
        }
    }

    /// One recommendation for how a pull combines: the editor's marks, Use recommended, the
    /// notification center and the diverged-pull dialog all read it from the same places
    /// (ADR 0012: fast-forward only, then Rebase when the branches diverged).
    #[test]
    fn the_pull_recommendation_is_one_choice_everywhere() {
        use canopy_core::PullStrategy;
        let git = Some((2, 55));
        // The editor marks Don't rebase on pull.rebase and fast-forward only on pull.ff.
        assert_eq!(Setting::PullRebase.recommendation_for(git), "false");
        assert_eq!(Setting::PullFastForward.recommendation_for(git), "only");
        assert_eq!(
            git_value_label(Setting::PullFastForward, "only"),
            DEFAULTS.pull_mode.label()
        );
        assert_eq!(DEFAULTS.pull_mode, PullMode::FastForwardOnly);
        // Use recommended writes pull.rebase only so that fast-forward only takes effect.
        assert_eq!(Setting::PullRebase.recommended_write(), Some("false"));
        // The dialog recommends one way to combine diverged branches, and marks only that one.
        let marked = [PullMode::Rebase, PullMode::Merge]
            .into_iter()
            .filter(|mode| crate::views::dialogs::diverged_label(*mode).contains("(recommended)"))
            .collect::<Vec<_>>();
        assert_eq!(marked, [PullMode::RECOMMENDED_WHEN_DIVERGED]);
        // The notification center judges each pull setting against its own row's recommendation,
        // named as that row's list names it.
        let pulls_with = |strategy, key: &str, value: &str| {
            let facts = GitFacts {
                pull: Some(canopy_core::PullConfig {
                    strategy: Some(strategy),
                    sources: vec![canopy_core::ConfigSource {
                        key: key.into(),
                        value: value.into(),
                        scope: canopy_core::ConfigScope::Global,
                        origin: "file:/home/me/.gitconfig".into(),
                        file: None,
                    }],
                    invalid: Vec::new(),
                }),
                git,
                ..GitFacts::default()
            };
            against_recommendation(&facts, &Settings::default(), Path::new("fixture"))
        };
        for (strategy, key, value) in [
            (PullStrategy::FastForwardOnly, "pull.ff", "only"),
            (PullStrategy::Merge, "pull.rebase", "false"),
            (PullStrategy::Merge, "pull.rebase", "no"),
        ] {
            assert_eq!(
                pulls_with(strategy, key, value),
                Vec::<String>::new(),
                "{key}={value} is the recommendation for its row"
            );
        }
        // The path in a line depends on the home folder, so only its ends are compared.
        for (strategy, key, value, start, end) in [
            (
                PullStrategy::Rebase,
                "pull.rebase",
                "true",
                "Rebase when pulling set to rebase in your global git config",
                "Recommended: Don't rebase.",
            ),
            (
                PullStrategy::Rebase,
                "branch.main.rebase",
                "true",
                "Rebase when pulling for main set to rebase in your global git config",
                "Recommended: Don't rebase.",
            ),
            (
                PullStrategy::Merge,
                "pull.ff",
                "true",
                "Fast-forward when pulling set to allow fast-forward in your global git config",
                "Recommended: Fast-forward only.",
            ),
        ] {
            let lines = pulls_with(strategy, key, value);
            assert!(
                matches!(lines.as_slice(), [line] if line.starts_with(start) && line.ends_with(end)),
                "{key}={value}: {lines:?}"
            );
        }
    }

    /// A search shows rows only: the editor's captions would sit apart from every result.
    #[test]
    fn a_search_leaves_the_git_editors_captions_out() {
        for scope in [Scope::Global, Scope::Local] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
            let mut screen = SettingsScreen::default();
            let drawn = |ctx: &egui::Context, screen: &mut SettingsScreen| {
                crate::ui::kit::testkit::spoken(ctx, |ui| {
                    git_config_editor(ui, screen, None, scope, None, true, &mut Vec::new());
                })
            };
            let shown = format!("{:?}", drawn(&ctx, &mut screen));
            assert!(
                shown.contains("Changes apply"),
                "{scope:?}: captions without a search"
            );
            Filter::set(
                &ctx,
                Filter {
                    words: vec!["xyzzy".to_owned()],
                    section: Some(Section::Git),
                    ..Filter::default()
                },
            );
            let found = format!("{:?}", drawn(&ctx, &mut screen));
            assert!(!found.contains("Changes apply"), "{scope:?}: {found}");
        }
    }

    /// A value git refuses is shown as written, with the problem, on its own row.
    #[test]
    fn an_invalid_git_value_shows_its_value_and_problem_on_its_row() {
        let mut facts = GitFacts {
            error: Some("fetch.prune: bad boolean config value 'maybe'".to_owned()),
            ..GitFacts::default()
        };
        facts.invalid = vec![fact("fetch.prune", "maybe")];
        facts.direct = vec![fact("fetch.prune", "maybe")];
        let nodes = git_page(facts, true);
        let row = format!("{nodes:?}");
        assert!(row.contains("Git does not accept maybe in"), "{row}");
        assert!(!row.contains("fetch.prune is unset"), "{row}");
    }

    /// Rows name settings by their labels, never by git's keys.
    /// A boolean written as `yes` or `1` names the option `true` does, in a row and in a note.
    #[test]
    fn a_spelled_out_boolean_has_the_label_of_its_option() {
        for (setting, written, label) in [
            (Setting::PullRebase, "1", "Rebase"),
            (Setting::PullRebase, "off", "Don't rebase"),
            (Setting::PullRebase, "merges", "Rebase preserving merges"),
            (Setting::PullFastForward, "yes", "Allow fast-forward"),
            (Setting::PullFastForward, "only", "Fast-forward only"),
            (Setting::Autocrlf, "on", "CRLF on checkout"),
            (Setting::Autocrlf, "input", "LF on commit"),
        ] {
            assert_eq!(git_value_label(setting, written), label, "{written}");
        }
    }

    #[test]
    fn the_git_page_never_shows_a_configuration_key() {
        let facts = GitFacts {
            direct: vec![fact("pull.rebase", "true"), fact("fetch.prune", "false")],
            fetch_prune: Some(fact("fetch.prune", "false")),
            push_default: Some(fact("push.default", "matching")),
            ..GitFacts::default()
        };
        let page = format!("{:?}", git_page(facts, true));
        for setting in Setting::ALL {
            assert!(!page.contains(setting.key()), "{} in {page}", setting.key());
        }
    }

    /// A remark about a row's state is a mark beside its title with the remark as its
    /// description, never another line under the help; a problem is a warning mark.
    #[test]
    fn a_rows_note_is_a_mark_beside_its_title() {
        let facts = GitFacts {
            values: vec![fact("fetch.prune", "false")],
            invalid: vec![fact("push.followTags", "maybe")],
            ..GitFacts::default()
        };
        let nodes = git_page(facts, true);
        let note = crate::ui::kit::testkit::find(&nodes, "Note on Prune remote branches")
            .expect("a note mark");
        assert!(
            note.description
                .as_deref()
                .is_some_and(|d| d.starts_with("Set in your global git config")),
            "{note:?}"
        );
        assert!(
            crate::ui::kit::testkit::find(&nodes, "Warning on Push annotated tags").is_some(),
            "a value git refuses is a warning"
        );
        assert!(
            !nodes
                .iter()
                .any(|n| n.value.as_deref().is_some_and(|v| v.starts_with("Set in"))),
            "the note is not drawn as text"
        );
    }

    /// Where a value is set reads as the signing row says it, never as git's raw `file:` origin.
    #[test]
    fn a_configured_values_origin_is_a_friendly_path() {
        let facts = GitFacts {
            values: vec![fact("fetch.prune", "false")],
            ..GitFacts::default()
        };
        let row = format!("{:?}", git_page(facts, true));
        assert!(row.contains("your global git config"), "{row}");
        assert!(!row.contains("file:/home/me/.gitconfig"), "{row}");
    }

    /// A global value is changed, never cleared; a repository's own value can go back to the
    /// global one, and only where the repository sets one.
    #[test]
    fn only_a_repositorys_own_value_goes_back_to_the_global_one() {
        let facts = GitFacts {
            values: vec![
                fact("fetch.prune", "false"),
                fact("push.followTags", "true"),
            ],
            direct: vec![fact("push.followTags", "true")],
            ..GitFacts::default()
        };
        let resets = |scope| {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            let mut screen = SettingsScreen::default();
            crate::ui::kit::testkit::spoken(&ctx, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    git_config_editor(
                        ui,
                        &mut screen,
                        Some(Path::new("fixture")),
                        scope,
                        Some(&facts),
                        true,
                        &mut Vec::new(),
                    );
                });
            })
            .into_iter()
            .filter(|n| {
                n.label
                    .as_deref()
                    .is_some_and(|l| l.ends_with("to the global value"))
            })
            .collect::<Vec<_>>()
        };
        assert_eq!(resets(Scope::Global).len(), 0);
        let local = resets(Scope::Local);
        assert_eq!(local.len(), 1, "{local:?}");
        assert!(!local[0].disabled, "{local:?}");
    }

    /// Edit my git config is off by default; until it is on, the editor shows its values as text
    /// with no control and the caption says how to change that, at both scopes.
    #[test]
    fn the_git_configuration_is_read_only_until_editing_is_turned_on() {
        assert!(!Settings::default().edit_git_config);
        let facts = GitFacts {
            values: vec![fact("fetch.prune", "false")],
            direct: vec![fact("fetch.prune", "false")],
            ..GitFacts::default()
        };
        for scope in [Scope::Global, Scope::Local] {
            for editable in [false, true] {
                let ctx = egui::Context::default();
                crate::ui::fonts::install(&ctx);
                let mut screen = SettingsScreen::default();
                let mut actions = Vec::new();
                let nodes = crate::ui::kit::testkit::spoken(&ctx, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        git_config_editor(
                            ui,
                            &mut screen,
                            Some(Path::new("fixture")),
                            scope,
                            Some(&facts),
                            editable,
                            &mut actions,
                        );
                    });
                });
                let controls = nodes
                    .iter()
                    .filter(|n| n.role != egui::accesskit::Role::Label)
                    .filter(|n| {
                        n.label
                            .as_deref()
                            .is_some_and(|l| Setting::ALL.iter().any(|s| s.label() == l))
                    })
                    .count();
                assert_eq!(controls > 0, editable, "{scope:?}: {controls} controls");
                let shown = |text: &str| {
                    nodes.iter().any(|n| {
                        [n.label.as_deref(), n.value.as_deref()]
                            .into_iter()
                            .flatten()
                            .any(|t| t == text)
                    })
                };
                assert!(
                    editable || shown("Off"),
                    "{scope:?}: the configured value is shown as text"
                );
                let caption = nodes.iter().any(|n| {
                    [n.label.as_deref(), n.value.as_deref()]
                        .into_iter()
                        .flatten()
                        .any(|t| t.starts_with("Read only."))
                });
                assert_eq!(caption, !editable, "{scope:?} editable {editable}");
                assert!(actions.is_empty());
            }
        }
    }

    /// Each setting a repository can override, with a global value and a different value for the
    /// repository. The first branch is global only: [`only_the_global_editor_offers_the_first_branch`].
    const CASES: [(Setting, &str, &str); 15] = [
        (Setting::PullRebase, "false", "true"),
        (Setting::PullFastForward, "true", "only"),
        (Setting::PullAutostash, "true", "false"),
        (Setting::FetchPrune, "true", "false"),
        (Setting::FetchPruneTags, "false", "true"),
        (Setting::RebaseAutostash, "true", "false"),
        (Setting::MergeAutostash, "true", "false"),
        (Setting::PushFollowTags, "true", "false"),
        (Setting::SubmoduleRecurse, "true", "false"),
        (Setting::PushDefault, "simple", "current"),
        (Setting::ConflictStyle, "merge", "diff3"),
        (Setting::CommitCleanup, "whitespace", "strip"),
        (Setting::Autocrlf, "false", "input"),
        (Setting::CommitSigning, "false", "true"),
        (Setting::SigningFormat, "openpgp", "ssh"),
    ];

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("repository");
        cli::run(dir.path(), &["init", "-q", "-b", "main"]).expect("init");
        dir
    }

    fn git_output(dir: &Path, args: &[&str]) -> String {
        cli::run_unchecked(dir, args)
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .expect("git runs")
    }

    /// The Git configuration editor for `repo` (or the global one), drawn from facts read by git now.
    struct Editor {
        ctx: egui::Context,
        screen: SettingsScreen,
        repo: Option<PathBuf>,
        scope: Scope,
        facts: GitFacts,
    }

    impl Editor {
        fn open(repo: Option<&Path>) -> Self {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
            let mut editor = Self {
                ctx,
                screen: SettingsScreen::default(),
                repo: repo.map(Path::to_path_buf),
                scope: repo.map_or(Scope::Global, |_| Scope::Local),
                facts: GitFacts::default(),
            };
            editor.reread();
            editor
        }

        fn reread(&mut self) {
            self.facts = crate::repo_config::read(self.repo.as_deref());
            assert_eq!(self.facts.error, None, "git's configuration reads cleanly");
        }

        fn draw(&mut self, events: Vec<egui::Event>, actions: &mut Vec<Action>) {
            let Self {
                ctx,
                screen,
                repo,
                scope,
                facts,
            } = self;
            crate::ui::kit::testkit::frame(ctx, events, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    git_config_editor(
                        ui,
                        screen,
                        repo.as_deref(),
                        *scope,
                        Some(facts),
                        true,
                        actions,
                    );
                });
            });
        }

        /// What the editor speaks once its layout has settled.
        fn nodes(&mut self) -> Vec<crate::ui::kit::testkit::Spoken> {
            let mut nodes = Vec::new();
            for _ in 0..3 {
                let Self {
                    ctx,
                    screen,
                    repo,
                    scope,
                    facts,
                } = self;
                nodes = crate::ui::kit::testkit::spoken(ctx, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        git_config_editor(
                            ui,
                            screen,
                            repo.as_deref(),
                            *scope,
                            Some(facts),
                            true,
                            &mut Vec::new(),
                        );
                    });
                });
            }
            nodes
        }

        /// What the row of `setting` shows now.
        fn shown(&mut self, setting: Setting) -> Option<String> {
            let nodes = self.nodes();
            crate::ui::kit::testkit::find(&nodes, setting.label())
                .and_then(|node| node.value.clone())
        }

        /// Presses the control named `name`; the actions it pushed.
        fn press(&mut self, name: &str) -> Vec<Action> {
            let nodes = self.nodes();
            let at = crate::ui::kit::testkit::find(&nodes, name)
                .unwrap_or_else(|| panic!("{name} is on the page"))
                .bounds
                .expect("it is drawn")
                .center();
            let mut actions = Vec::new();
            self.draw(crate::ui::kit::testkit::press(at), &mut actions);
            self.draw(crate::ui::kit::testkit::release(at), &mut actions);
            actions
        }

        fn reset_name(setting: Setting) -> String {
            format!("Reset {} to the global value", setting.label())
        }

        fn has_reset(&mut self, setting: Setting) -> bool {
            let nodes = self.nodes();
            crate::ui::kit::testkit::find(&nodes, &Self::reset_name(setting)).is_some()
        }
    }

    /// The keys the repository's own file holds, with their values, one per line.
    fn local_entries(dir: &Path) -> String {
        git_output(dir, &["config", "--local", "--list"])
    }

    /// A row shows the value git uses, wherever it is set; a choice writes this repository's file and
    /// nothing else; and the reset control returns the global value, leaving no key behind.
    #[test]
    fn every_repository_git_setting_is_shown_written_in_effect_and_reset() {
        if !crate::repo_config::in_isolated_git_env(
            "views::settings_screen::tests::every_repository_git_setting_is_shown_written_in_effect_and_reset",
        ) {
            return;
        }
        let dir = repository();
        let repo = dir.path();
        let global_file =
            PathBuf::from(std::env::var_os("GIT_CONFIG_GLOBAL").expect("private global config"));
        for (setting, global, local) in CASES {
            let key = setting.key();
            // A closed select says Recommended after the value that is the recommendation (#232).
            let label = |value: &str| {
                let label = git_value_label(setting, value);
                if value == setting.recommendation() {
                    format!("{label} (Recommended)")
                } else {
                    label
                }
            };
            config::set(repo, Scope::Global, setting, Some(global)).expect("global write");
            let global_bytes = std::fs::read(&global_file).expect("global file");

            // Set globally only: the repository shows the global value and has nothing to reset.
            let mut editor = Editor::open(Some(repo));
            assert_eq!(
                editor.shown(setting),
                Some(label(global)),
                "{key}: inherited"
            );
            assert!(
                !editor.has_reset(setting),
                "{key}: nothing of its own to reset"
            );
            assert_eq!(git_output(repo, &["config", "--get", key]), global, "{key}");

            // Chosen for the repository: one key in its own file, and it is the value in effect.
            let before = local_entries(repo);
            config::set(repo, Scope::Local, setting, Some(local)).expect("repository write");
            let written: Vec<String> = local_entries(repo)
                .lines()
                .filter(|line| !before.lines().any(|old| old == *line))
                .map(str::to_owned)
                .collect();
            assert_eq!(
                written,
                [format!("{}={local}", key.to_lowercase())],
                "{key}"
            );
            assert_eq!(
                std::fs::read(&global_file).expect("global file"),
                global_bytes,
                "{key}: the global file is untouched"
            );
            assert_eq!(git_output(repo, &["config", "--get", key]), local, "{key}");
            let mut editor = Editor::open(Some(repo));
            assert_eq!(
                editor.shown(setting),
                Some(label(local)),
                "{key}: own value"
            );

            // Reset: the control asks for the key to be removed from this repository only.
            let actions = editor.press(&Editor::reset_name(setting));
            let [
                Action::SaveGitConfig {
                    repo: at,
                    scope,
                    setting: chosen,
                    value: None,
                },
            ] = actions.as_slice()
            else {
                panic!("{key}: the reset pushes one removal, not {}", actions.len());
            };
            assert_eq!(
                (at.as_deref(), *scope, *chosen),
                (Some(repo), Scope::Local, setting)
            );
            config::set(repo, *scope, *chosen, None).expect("repository reset");
            assert_eq!(local_entries(repo), before, "{key}: the file is as it was");
            assert_eq!(git_output(repo, &["config", "--get", key]), global, "{key}");
            let mut editor = Editor::open(Some(repo));
            assert_eq!(editor.shown(setting), Some(label(global)), "{key}: back");
            assert!(!editor.has_reset(setting), "{key}: reset is done");
            assert_eq!(
                std::fs::read(&global_file).expect("global file"),
                global_bytes,
                "{key}: the global file is untouched"
            );
        }
    }

    /// The Push destination list offers `simple`, `upstream` and `current`, which Canopy's own push
    /// understands. A `matching` or `nothing` the user's own configuration holds is still the row's
    /// value, with where it is set and why it is not a choice, and is not rewritten (#261).
    #[test]
    fn push_destinations_git_holds_but_canopy_does_not_offer_are_shown() {
        if !crate::repo_config::in_isolated_git_env(
            "views::settings_screen::tests::push_destinations_git_holds_but_canopy_does_not_offer_are_shown",
        ) {
            return;
        }
        let home = crate::settings::home_dir();
        let global_file =
            PathBuf::from(std::env::var_os("GIT_CONFIG_GLOBAL").expect("private global config"));
        let setting = Setting::PushDefault;
        let offered = ["Current", "Simple", "Upstream"];
        for value in ["simple", "upstream", "current", "matching", "nothing"] {
            cli::run(&home, &["config", "--global", setting.key(), value]).expect("global write");
            let written = std::fs::read(&global_file).expect("global file");
            let mut editor = Editor::open(None);
            let nodes = editor.nodes();
            let said = crate::ui::kit::testkit::find(&nodes, setting.label())
                .and_then(|node| node.value.clone());
            let label = git_value_label(setting, value);
            let expected = if value == "simple" {
                format!("{label} (Recommended)")
            } else {
                label
            };
            assert_eq!(said, Some(expected), "{value}");
            let note = crate::ui::kit::testkit::find(&nodes, "Note on Push destination")
                .and_then(|n| n.description.clone())
                .expect("the row says where the value is set");
            assert!(
                note.starts_with("Set in your global git config"),
                "{value}: {note}"
            );
            assert_eq!(
                note.contains("Canopy does not offer"),
                matches!(value, "matching" | "nothing"),
                "{value}: {note}"
            );
            // Opening the list lists the three, whatever the row holds.
            editor.press(setting.label());
            let mut options: Vec<_> = editor
                .nodes()
                .into_iter()
                .filter(|n| n.role == egui::accesskit::Role::ListBoxOption)
                .filter_map(|n| n.label)
                .collect();
            options.sort();
            assert_eq!(options, offered, "{value}");
            assert_eq!(
                std::fs::read(&global_file).expect("global file"),
                written,
                "{value}: looking changes nothing"
            );
        }
    }

    /// Git reads `init.defaultBranch` only to create a repository, never from one that exists, so a
    /// repository's own value would change nothing: its editor leaves the row out (#230).
    #[test]
    fn only_the_global_editor_offers_the_first_branch() {
        if !crate::repo_config::in_isolated_git_env(
            "views::settings_screen::tests::only_the_global_editor_offers_the_first_branch",
        ) {
            return;
        }
        let dir = repository();
        let first_branch = Setting::InitialBranch.label();
        let repo = Editor::open(Some(dir.path())).nodes();
        assert!(
            crate::ui::kit::testkit::find(&repo, first_branch).is_none(),
            "the repository page offers a setting nothing reads"
        );
        assert!(crate::ui::kit::testkit::find(&repo, Setting::PushDefault.label()).is_some());
        let global = Editor::open(None).nodes();
        assert!(crate::ui::kit::testkit::find(&global, first_branch).is_some());
    }

    /// The labels spoken by Settings → This repository for `/work/canopy`, with `facts` as what git's
    /// configuration says there and `settings` as Canopy's own.
    fn repository_labels(facts: GitFacts, mut settings: Settings) -> Vec<String> {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.enable_accesskit();
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let repo = Some(Path::new("/work/canopy"));
        let mut screen = SettingsScreen::at(Section::Repository);
        screen.facts.global.asked = true;
        screen.facts.global.facts = Some(Arc::new(GitFacts::default()));
        screen.facts.repo.asked = true;
        screen.facts.repo.path = repo.map(Path::to_path_buf);
        screen.facts.repo.facts = Some(Arc::new(facts));
        screen.facts.repo.read_at = Some(Instant::now());
        let mut last = None;
        for time in 1..=3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 6000.0))),
                    time: Some(f64::from(time)),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui.ctx(),
                        &mut screen,
                        &mut settings,
                        &SettingsData {
                            tabs: &[],
                            update: &UpdateState::Idle,
                            identity: IdentityView {
                                repo,
                                identity: None,
                                error: None,
                            },
                        },
                        &mut Vec::new(),
                    );
                },
            );
            output.textures_delta.clear();
            last = output.platform_output.accesskit_update;
        }
        last.expect("accesskit")
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label().map(str::to_owned))
            .collect()
    }

    fn config_value(key: &str, value: &str) -> ConfigValue {
        ConfigValue {
            key: key.to_owned(),
            value: value.to_owned(),
            scope: "local".to_owned(),
            origin: "file:/work/canopy/.git/config".to_owned(),
            file: None,
        }
    }

    /// Canopy's Pull strategy follows the repository's own "Follow my git config for pulls" and its
    /// Prune on fetch the global Follow my git config: a row is offered exactly when Canopy's choice,
    /// not git's configuration, decides what the operation does (#230).
    #[test]
    fn canopys_pull_and_prune_choices_show_only_where_canopy_decides() {
        use canopy_core::{ConfigScope, ConfigSource, PullConfig, PullStrategy};
        let pull_set = || PullConfig {
            strategy: Some(PullStrategy::Rebase),
            sources: vec![ConfigSource {
                key: "pull.rebase".into(),
                value: "true".into(),
                scope: ConfigScope::Local,
                origin: "file:/work/canopy/.git/config".into(),
                file: None,
            }],
            invalid: Vec::new(),
        };
        let repo = PathBuf::from("/work/canopy");
        let offered = |facts: GitFacts, global_follow: bool, repo_pull_follow: Option<bool>| {
            let mut settings = Settings {
                follow_git_config: global_follow,
                ..Settings::default()
            };
            settings.repo_mut(&repo).pull_follow_config = repo_pull_follow;
            let labels = repository_labels(facts, settings);
            let has = |name: &str| labels.iter().any(|label| label == name);
            (has("Pull strategy"), has("Prune on fetch"))
        };
        let nothing = GitFacts::default;
        let pull = || GitFacts {
            pull: Some(pull_set()),
            ..GitFacts::default()
        };
        let prune = || GitFacts {
            fetch_prune: Some(config_value("fetch.prune", "false")),
            ..GitFacts::default()
        };

        // Git sets nothing: Canopy's choices decide, whatever the switches say.
        for (global, own) in [
            (true, None),
            (false, None),
            (true, Some(false)),
            (false, Some(true)),
        ] {
            assert_eq!(
                offered(nothing(), global, own),
                (true, true),
                "{global} {own:?}"
            );
        }
        // A pull strategy in git's configuration: hidden while this repository follows it, whether
        // that is its own switch or the global one it inherits.
        assert_eq!(
            offered(pull(), true, None),
            (false, true),
            "inherited follow"
        );
        assert_eq!(
            offered(pull(), false, Some(true)),
            (false, true),
            "own follow on"
        );
        assert_eq!(
            offered(pull(), true, Some(false)),
            (true, true),
            "own follow off"
        );
        assert_eq!(
            offered(pull(), false, None),
            (true, true),
            "global follow off"
        );
        // Pruning: only the global switch decides; a repository has no switch of its own for it, so
        // the pull switch changes nothing here.
        assert_eq!(
            offered(prune(), true, None),
            (true, false),
            "global follow on"
        );
        assert_eq!(
            offered(prune(), true, Some(false)),
            (true, false),
            "pull switch is not prune's"
        );
        assert_eq!(
            offered(prune(), false, None),
            (true, true),
            "global follow off"
        );
        assert_eq!(
            offered(prune(), false, Some(true)),
            (true, true),
            "pull switch is not prune's"
        );
    }

    /// The system's own program is the Default entry of a launcher's list, and only that.
    #[test]
    fn the_system_program_is_listed_once_in_a_launchers_list() {
        use crate::system_apps::{App, Kind, SystemChoice};
        let app = |name: &str, command: &str, id: &str| App {
            kind: Kind::Terminal,
            name: name.to_owned(),
            command: command.to_owned(),
            id: id.to_owned(),
        };
        // xdg-terminal-exec starts kitty: the default is kitty's own row, not a second one.
        let system = SystemChoice {
            name: "kitty".to_owned(),
            command: Some("xdg-terminal-exec".to_owned()),
            id: Some("/usr/bin/kitty".to_owned()),
        };
        assert!(is_system_choice(
            Some(&system),
            &app("kitty", "kitty", "/usr/bin/kitty")
        ));
        assert!(
            !is_system_choice(Some(&system), &app("kitty", "kitty", "/opt/kitty")),
            "a name or command in common is not the program"
        );
        assert!(!is_system_choice(
            Some(&system),
            &app("Foot", "foot", "/usr/bin/foot")
        ));
        assert!(!is_system_choice(
            None,
            &app("kitty", "kitty", "/usr/bin/kitty")
        ));
        let unlisted = SystemChoice { id: None, ..system };
        assert!(!is_system_choice(
            Some(&unlisted),
            &app("kitty", "kitty", "/usr/bin/kitty")
        ));
    }

    fn terminal(name: &str, command: &str, id: &str) -> crate::system_apps::App {
        crate::system_apps::App {
            kind: Kind::Terminal,
            name: name.to_owned(),
            command: command.to_owned(),
            id: id.to_owned(),
        }
    }

    fn labels(options: &[(LaunchChoice, String)]) -> Vec<&str> {
        options.iter().map(|(_, l)| l.as_str()).collect()
    }

    /// The system's own program is "<name> (Default)", or plain "Default" before one is known;
    /// nothing in a launcher's list says "Recommended" or names an environment variable.
    #[test]
    fn a_launchers_list_marks_the_systems_program_default() {
        use crate::system_apps::SystemChoice;
        let (foot, kitty) = (
            terminal("Foot", "foot", "/usr/bin/foot"),
            terminal("kitty", "kitty", "/usr/bin/kitty"),
        );
        let installed = [&foot, &kitty];
        let system = SystemChoice {
            name: "kitty".to_owned(),
            command: Some("xdg-terminal-exec".to_owned()),
            id: Some("/usr/bin/kitty".to_owned()),
        };
        let detected = launcher_options(Some(&system), &installed, &[]);
        assert_eq!(
            labels(&detected),
            ["kitty (Default)", "Foot", "Custom command…"],
            "the default is listed once, under its own name"
        );
        assert_eq!(
            detected[1].0,
            LaunchChoice::App(0),
            "Foot is the first installed"
        );
        let undetected = launcher_options(None, &installed, &[]);
        assert_eq!(
            labels(&undetected),
            ["Default", "Foot", "kitty", "Custom command…"]
        );
        for label in labels(&detected).into_iter().chain(labels(&undetected)) {
            assert!(
                !label.contains("Recommended") && !label.contains('$'),
                "{label}"
            );
        }
    }

    /// Every launcher row, closed and open, says Default and never Recommended or `$VARIABLE`; the
    /// Default entry's description is "Default".
    #[test]
    fn the_integrations_launchers_show_default_never_recommended() {
        use crate::system_apps::{Apps, SystemChoice};
        use crate::ui::kit::testkit::{frame, key, spoken};
        let apps = Apps {
            terminal: Some(SystemChoice {
                name: "kitty".to_owned(),
                command: Some("xdg-terminal-exec".to_owned()),
                id: Some("/usr/bin/kitty".to_owned()),
            }),
            installed: vec![
                terminal("Foot", "foot", "/usr/bin/foot"),
                terminal("kitty", "kitty", "/usr/bin/kitty"),
            ],
            ..Apps::default()
        };
        let ctx = crate::ui::kit::testkit::ctx();
        let (mut value, mut custom) = (String::new(), HashSet::new());
        let mut draw = |ui: &mut Ui| {
            for (title, kind) in [
                ("Terminal", Kind::Terminal),
                ("Editor", Kind::Editor),
                ("File manager", Kind::FileManager),
            ] {
                let launcher = Launcher {
                    title,
                    help: "",
                    command_help: "",
                    kind,
                };
                launcher_row(ui, launcher, &mut value, Some(&apps), &mut custom);
            }
        };
        let closed = spoken(&ctx, &mut draw);
        let shown: Vec<_> = closed
            .iter()
            .filter(|n| n.role == egui::accesskit::Role::ComboBox)
            .map(|n| n.value.as_deref())
            .collect();
        assert_eq!(
            shown,
            [Some("kitty (Default)"), Some("Default"), Some("Default")],
            "the detected default; plain Default where nothing is detected"
        );
        // Tab reaches the terminal's select, and the arrow opens its list.
        frame(&ctx, Vec::new(), &mut draw);
        frame(&ctx, key(egui::Key::Tab), &mut draw);
        frame(&ctx, Vec::new(), &mut draw);
        frame(&ctx, key(egui::Key::ArrowDown), &mut draw);
        let open = spoken(&ctx, &mut draw);
        let mut options: Vec<_> = open
            .iter()
            .filter(|n| n.role == egui::accesskit::Role::ListBoxOption)
            .map(|n| (n.label.as_deref(), n.description.as_deref()))
            .collect();
        options.sort_unstable();
        assert_eq!(
            options,
            [
                (Some("Custom command…"), None),
                (Some("Foot"), None),
                (Some("kitty (Default)"), Some("Default")),
            ]
        );
        for n in closed.iter().chain(&open) {
            for text in [&n.label, &n.value, &n.description].into_iter().flatten() {
                assert!(
                    !text.contains("Recommended") && !text.contains('$'),
                    "{text}"
                );
            }
        }
    }

    /// Well-known programs that are not installed follow the installed ones, before Custom
    /// command…, marked "Not installed" and never chosen; a saved choice that has gone shows as
    /// one of them.
    #[test]
    fn missing_programs_are_listed_marked_and_never_chosen() {
        use crate::system_apps::{Apps, Missing, SystemChoice};
        use crate::ui::kit::testkit::{frame, key, spoken};
        let apps = Apps {
            terminal: Some(SystemChoice {
                name: "kitty".to_owned(),
                command: Some("kitty".to_owned()),
                id: Some("/usr/bin/kitty".to_owned()),
            }),
            installed: vec![
                terminal("Foot", "foot", "/usr/bin/foot"),
                terminal("kitty", "kitty", "/usr/bin/kitty"),
            ],
            missing: vec![
                Missing::new(Kind::Terminal, "Alacritty", ["alacritty"]),
                Missing::new(Kind::Terminal, "Konsole", ["konsole"]),
            ],
            ..Apps::default()
        };
        let installed: Vec<_> = apps.installed(Kind::Terminal).collect();
        let missing: Vec<_> = apps.missing(Kind::Terminal).collect();
        assert_eq!(
            labels(&launcher_options(
                apps.terminal.as_ref(),
                &installed,
                &missing
            )),
            [
                "kitty (Default)",
                "Foot",
                "Alacritty",
                "Konsole",
                "Custom command…"
            ]
        );
        // A saved command of a program that is not installed shows as that program, an installed
        // one as itself, and anything else as a custom command.
        let shown = |value: &str| launch_choice(value, false, &installed, &missing, false);
        assert_eq!(
            shown("alacritty --working-directory %d"),
            LaunchChoice::Missing(0)
        );
        assert_eq!(shown("konsole"), LaunchChoice::Missing(1));
        assert_eq!(shown("foot"), LaunchChoice::App(0));
        assert_eq!(shown("my-terminal"), LaunchChoice::Custom);
        assert_eq!(
            shown("/opt/alacritty/bin/alacritty"),
            LaunchChoice::Custom,
            "a command with a path to a file that is there is the user's own program"
        );
        assert_eq!(
            launch_choice(
                "/opt/alacritty/bin/alacritty",
                false,
                &installed,
                &missing,
                true
            ),
            LaunchChoice::Missing(0),
            "a path to a file that is gone is the program that was saved from the list"
        );

        let ctx = crate::ui::kit::testkit::ctx();
        let (mut value, mut custom) = (
            "alacritty --working-directory %d".to_owned(),
            HashSet::new(),
        );
        let mut draw = |ui: &mut Ui| {
            let launcher = Launcher {
                title: "Terminal",
                help: "",
                command_help: "",
                kind: Kind::Terminal,
            };
            launcher_row(ui, launcher, &mut value, Some(&apps), &mut custom);
        };
        let closed = spoken(&ctx, &mut draw);
        let combo = closed
            .iter()
            .find(|n| n.role == egui::accesskit::Role::ComboBox);
        let shown_value = combo.and_then(|n| n.value.as_deref());
        assert_eq!(
            combo.and_then(|n| n.description.as_deref()),
            Some("Not installed"),
            "the closed control says the saved choice is gone"
        );
        assert_eq!(
            shown_value,
            Some("Alacritty"),
            "the saved choice, though gone"
        );
        // Open the list: the missing programs are disabled with their tag as description.
        frame(&ctx, Vec::new(), &mut draw);
        frame(&ctx, key(egui::Key::Tab), &mut draw);
        frame(&ctx, Vec::new(), &mut draw);
        frame(&ctx, key(egui::Key::ArrowDown), &mut draw);
        let open = spoken(&ctx, &mut draw);
        let mut options: Vec<_> = open
            .iter()
            .filter(|n| n.role == egui::accesskit::Role::ListBoxOption)
            .map(|n| (n.label.as_deref(), n.disabled, n.description.as_deref()))
            .collect();
        options.sort_unstable();
        assert_eq!(
            options,
            [
                (Some("Alacritty"), true, Some("Not installed")),
                (Some("Custom command…"), false, None),
                (Some("Foot"), false, None),
                (Some("Konsole"), true, Some("Not installed")),
                (Some("kitty (Default)"), false, Some("Default")),
            ]
        );
        assert_eq!(
            value, "alacritty --working-directory %d",
            "nothing was chosen"
        );
    }

    /// Each launcher row is found by its kind and by "default <kind>" (its help never says it).
    #[test]
    fn each_launcher_row_is_found_by_its_kind_and_as_the_default() {
        let mut settings = Settings::default();
        for query in [
            "terminal",
            "default terminal",
            "editor",
            "default editor",
            "file manager",
            "default file manager",
        ] {
            let (found, _) = draw_screen_in(SettingsScreen::searching(query), &mut settings, None);
            assert_eq!(
                found.search.count(Section::Integrations),
                1,
                "{query}: {:?}",
                found.search
            );
        }
    }

    #[test]
    fn notifications_cover_all_remaining_recommended_git_choices() {
        let facts = GitFacts {
            push_default: Some(fact("push.default", "matching")),
            conflict_style: Some(fact("merge.conflictStyle", "merge")),
            cleanup: Some(fact("commit.cleanup", "strip")),
            initial_branch: Some(fact("init.defaultBranch", "trunk")),
            submodule_recurse: Some(fact("submodule.recurse", "true")),
            git: Some((2, 55)),
            ..GitFacts::default()
        };
        let lines = against_recommendation(&facts, &Settings::default(), Path::new("fixture"));
        for key in [
            Setting::PushDefault,
            Setting::ConflictStyle,
            Setting::CommitCleanup,
            Setting::InitialBranch,
            Setting::SubmoduleRecurse,
        ]
        .map(Setting::label)
        {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(key) && line.contains("your global git config")),
                "missing {key}: {lines:?}"
            );
        }
    }

    /// A boolean written another way is still the recommendation, and the card names the
    /// recommendation as the editor's list does (On, Off, Default), the same word everywhere.
    #[test]
    fn a_finding_names_the_recommendation_as_the_list_does() {
        let spelled = GitFacts {
            submodule_recurse: Some(fact("submodule.recurse", "no")),
            git: Some((2, 55)),
            ..GitFacts::default()
        };
        assert_eq!(
            against_recommendation(&spelled, &Settings::default(), Path::new("fixture")),
            Vec::<String>::new(),
            "no is false"
        );
        let facts = GitFacts {
            fetch_prune: Some(fact("fetch.prune", "false")),
            cleanup: Some(fact("commit.cleanup", "whitespace")),
            submodule_recurse: Some(fact("submodule.recurse", "yes")),
            git: Some((2, 55)),
            ..GitFacts::default()
        };
        let lines = against_recommendation(&facts, &Settings::default(), Path::new("fixture"));
        for (row, name) in [
            ("Prune remote branches", "On"),
            ("Commit message cleanup", "Default"),
            ("Recurse into submodules", "Off"),
        ] {
            assert!(
                lines
                    .iter()
                    .any(|l| l.starts_with(row) && l.ends_with(&format!("Recommended: {name}."))),
                "{row}: {lines:?}"
            );
        }
    }

    /// Git before 2.51 does not read `pull.autoStash`: once the facts are read, neither the
    /// Settings row's note nor the notice says the configuration decides by it; the keys git does
    /// read still are.
    #[test]
    fn a_pull_autostash_value_is_reported_only_where_git_reads_it() {
        let read = |git, keys: &[&str]| {
            let mut facts = GitFacts {
                autostash: keys.iter().map(|k| fact(k, "false")).collect(),
                git,
                ..GitFacts::default()
            };
            facts.forget_ignored();
            facts
        };
        let notice = |facts: &GitFacts| {
            against_recommendation(facts, &Settings::default(), Path::new("fixture"))
        };
        for git in [Some((2, 50)), None] {
            let facts = read(git, &["pull.autoStash"]);
            assert!(autostash_note(&facts, true, true).is_none(), "{git:?}");
            assert_eq!(notice(&facts), Vec::<String>::new(), "{git:?}");
        }
        for git in [Some((2, 51)), Some((2, 55))] {
            let facts = read(git, &["pull.autoStash"]);
            assert!(autostash_note(&facts, true, true).is_some(), "{git:?}");
            assert_eq!(notice(&facts).len(), 1, "{git:?}");
        }
        let old = read(Some((2, 50)), &["pull.autoStash", "merge.autoStash"]);
        let note = autostash_note(&old, true, true).expect("a note").text;
        assert!(
            note.contains("Autostash before merge") && !note.contains("Autostash for all pulls"),
            "{note}"
        );
        let lines = notice(&old);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(!lines[0].contains("Autostash for all pulls"), "{lines:?}");
    }

    /// The Autostash row says a squash merge is never stashed, and, with Canopy's own choice
    /// applying, that its merges use that choice instead of the configuration.
    #[test]
    fn the_autostash_row_says_a_squash_never_autostashes() {
        assert!(
            AUTOSTASH_HELP.contains("never a squash"),
            "{AUTOSTASH_HELP}"
        );
        let facts = GitFacts {
            autostash: vec![fact("merge.autoStash", "true")],
            git: Some((2, 55)),
            ..GitFacts::default()
        };
        let note = autostash_note(&facts, false, false).expect("a note").text;
        assert!(
            note.contains("Canopy's pulls, rebases and merges"),
            "{note}"
        );
    }

    #[test]
    fn the_notice_lists_findings_as_paragraphs_and_counts_the_rest() {
        let findings = |n: usize| (1..=n).map(|i| format!("Finding {i}.")).collect::<Vec<_>>();
        assert_eq!(notice_body(&findings(1)), "Finding 1.");
        assert_eq!(
            notice_body(&findings(4)),
            "Finding 1.\n\nFinding 2.\n\nFinding 3.\n\nFinding 4.",
            "four fit"
        );
        assert_eq!(
            notice_body(&findings(6)),
            "Finding 1.\n\nFinding 2.\n\nFinding 3.\n\nAnd 3 more.",
            "more than four: the first three and the count of the rest"
        );
    }

    /// #42: the notification center lists what the git configuration decides against the
    /// recommendation, only while Canopy follows it.
    #[test]
    fn tag_pruning_notifications_use_the_remote_override() {
        let mut facts = GitFacts {
            remotes: vec![crate::repo_config::Remote {
                name: "origin".into(),
                url: "https://github.com/example/repo".into(),
            }],
            prune_tags: vec![
                fact("fetch.pruneTags", "true"),
                fact("remote.origin.pruneTags", "false"),
            ],
            ..GitFacts::default()
        };
        assert_eq!(
            against_recommendation(&facts, &Settings::default(), Path::new("fixture")),
            Vec::<String>::new()
        );
        facts.prune_tags.pop();
        assert_eq!(
            against_recommendation(&facts, &Settings::default(), Path::new("fixture")).len(),
            1
        );
    }

    #[test]
    fn the_configuration_against_the_recommendation_is_listed_while_following_it() {
        let facts = GitFacts {
            fetch_prune: Some(fact("fetch.prune", "false")),
            follow_tags: Some(fact("push.followTags", "true")),
            ..GitFacts::default()
        };
        let repo = Path::new("/work/canopy");
        let follow = Settings {
            follow_git_config: true,
            ..Settings::default()
        };
        let lines = against_recommendation(&facts, &follow, repo);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("Prune remote branches set to off")
                && lines[0].contains("Recommended"),
            "{lines:?}"
        );
        let ours = Settings {
            follow_git_config: false,
            ..Settings::default()
        };
        assert!(
            against_recommendation(&facts, &ours, repo).is_empty(),
            "Canopy's own choice applies, so nothing is overruled"
        );
        assert!(
            against_recommendation(&GitFacts::default(), &follow, repo).is_empty(),
            "a configuration that sets nothing is not listed"
        );
    }

    /// #55: Change listens for the next key, a free key is saved under the row and moves the
    /// command, a taken key is refused with the row that has it, and Reset removes it.
    #[test]
    fn the_keyboard_page_changes_and_resets_a_key() {
        use crate::keymap;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let mut settings = Settings::default();
        let mut screen = SettingsScreen::default();
        let frame =
            |screen: &mut SettingsScreen, settings: &mut Settings, key: Option<egui::Key>| {
                let events = key
                    .map(|key| egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .into_iter()
                    .collect();
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        keymap::capture_key(ui.ctx());
                        egui::CentralPanel::default().show(ui, |ui| keyboard(ui, screen, settings));
                        keymap::apply_saved(&settings.keybindings);
                    },
                );
                out.textures_delta.clear();
            };
        let id = "Fetch.Repository".to_owned();
        let fetch = keymap::binding_by_id(&id).expect("a row");
        // Change on that row.
        keymap::start_capture(&ctx);
        screen.rebinding = Some(id.clone());
        frame(&mut screen, &mut settings, Some(egui::Key::J));
        assert!(settings.keybindings.is_empty(), "j is taken");
        let note = screen
            .key_note
            .as_ref()
            .map(|(row, why)| (row.as_str(), why.as_str()));
        assert!(
            note.is_some_and(|(row, why)| row == id && why.ends_with("already uses j.")),
            "{note:?}"
        );
        keymap::start_capture(&ctx);
        screen.rebinding = Some(id.clone());
        frame(&mut screen, &mut settings, Some(egui::Key::V));
        assert_eq!(
            settings.keybindings.get(&id).map(String::as_str),
            Some("V"),
            "saved as text"
        );
        assert_eq!(screen.key_note, None);
        assert_eq!(fetch.current_chords().len(), 1);
        assert!(keymap::is_customised(fetch));
        // Esc cancels and keeps what is there.
        keymap::start_capture(&ctx);
        screen.rebinding = Some(id.clone());
        frame(&mut screen, &mut settings, Some(egui::Key::Escape));
        assert_eq!(settings.keybindings.len(), 1);
        assert_eq!(screen.rebinding, None);
        // Giving the row its own table key again leaves no entry.
        keymap::start_capture(&ctx);
        screen.rebinding = Some(id.clone());
        frame(&mut screen, &mut settings, Some(egui::Key::F));
        assert!(settings.keybindings.is_empty());
        assert!(!keymap::is_customised(fetch));
        keymap::apply_saved(&std::collections::BTreeMap::new());
    }

    /// Selecting another predefined theme leaves the appearance controls at the same position.
    #[test]
    fn choosing_a_theme_does_not_shift_the_page() {
        let at = |theme: ThemeId| {
            let settings = Settings {
                theme,
                ..Settings::default()
            };
            heading_top_with(SettingsScreen::at(Section::Appearance), settings, "Look")
        };
        let default = at(DEFAULTS.theme);
        // Custom opens its own group of colour rows, which is what choosing it asks for.
        for theme in ThemeId::ALL.into_iter().filter(|t| *t != ThemeId::Custom) {
            assert!(
                (at(theme) - default).abs() < 0.5,
                "{theme:?} moves the page by {}",
                at(theme) - default
            );
        }
        let nodes = {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
            let mut settings = Settings {
                theme: ThemeId::Daylight,
                ..Settings::default()
            };
            crate::ui::kit::testkit::spoken(&ctx, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    appearance(ui, &mut settings, &mut None);
                });
            })
        };
        assert!(
            nodes
                .iter()
                .any(|n| n.label.as_deref() == Some("Midnight theme (recommended)")),
            "Midnight is identified as the default even when Daylight is selected"
        );
        assert!(
            !nodes.iter().any(|n| n
                .label
                .as_deref()
                .is_some_and(|l| l.starts_with("Back to the default theme"))),
            "the separate default-theme button was removed"
        );
    }

    #[test]
    fn the_hosting_status_names_one_outcome_per_state() {
        let status = hosting_status;
        let working: hosting::CliState = Ok(hosting::demo_account());
        let signed_out = hosting::signed_out(ForgeKind::GitHub, "github.com");
        assert_eq!(
            status(false, Some(&working), true, true),
            HostingStatus::Off
        );
        assert_eq!(
            status(true, Some(&working), true, true),
            HostingStatus::Rejected
        );
        assert_eq!(
            status(true, Some(&working), false, true),
            HostingStatus::RateLimited
        );
        assert_eq!(
            status(true, Some(&working), false, false),
            HostingStatus::Working(Some("octocat".into()))
        );
        assert_eq!(
            status(true, Some(&signed_out), false, false),
            HostingStatus::NoCredential(CliError::SignedOut {
                kind: ForgeKind::GitHub,
                host: "github.com".into()
            })
        );
        assert_eq!(status(true, None, false, false), HostingStatus::Unknown);
    }

    /// The words the hosting block shows for a tab whose requests are in the given state.
    fn hosting_words(enabled: bool, cli: Option<hosting::CliState>, rejected: bool) -> String {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _events, _jobs) = crate::worker::Worker::detached();
        let mut tab = crate::tab::RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.snap = Some(Arc::new(canopy_core::RepoSnapshot::default()));
        tab.forges = [
            "origin|https://github.com/acme/demo.git",
            "up|https://gitlab.com/acme/demo.git",
        ]
        .iter()
        .filter_map(|r| {
            let (name, url) = r.split_once('|')?;
            Some((name.to_owned(), Forge::parse(url, &[])?))
        })
        .collect();
        tab.requests.cli = cli;
        tab.requests.token_rejected = rejected;
        let settings = Settings {
            hosting_api: enabled,
            ..Settings::default()
        };
        let mut screen = SettingsScreen::default();
        let mut words = String::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
                    ..Default::default()
                },
                |ui| hosting_account(ui, &mut screen, &settings, &tab),
            );
            output.textures_delta.clear();
            words = output
                .platform_output
                .accesskit_update
                .map(|u| {
                    u.nodes
                        .into_iter()
                        .filter_map(|(_, n)| n.label().or(n.value()).map(str::to_owned))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
        }
        words
    }

    #[test]
    fn the_hosting_block_says_the_outcome_the_remote_and_the_other_remotes() {
        let working = hosting_words(true, Some(Ok(hosting::demo_account())), false);
        assert!(
            working.contains("load from github.com using the GitHub CLI's sign-in as octocat"),
            "{working}"
        );
        assert!(working.contains("remote origin"), "{working}");
        assert!(working.contains("up (gitlab.com)"), "{working}");
        assert!(working.contains("Open Accounts"), "{working}");
        let rejected = hosting_words(true, Some(Ok(hosting::demo_account())), true);
        assert!(
            rejected.contains(
                "github.com refused the GitHub CLI's sign-in, so pull requests cannot load."
            ),
            "{rejected}"
        );
        let bare = hosting_words(
            true,
            Some(hosting::signed_out(ForgeKind::GitHub, "github.com")),
            false,
        );
        assert!(
            bare.contains(
                "Not signed in with the GitHub CLI. Public repositories load; private ones do not."
            ),
            "{bare}"
        );
        let missing = hosting_words(
            true,
            Some(Err(CliError::NotInstalled {
                kind: ForgeKind::GitHub,
            })),
            false,
        );
        assert!(
            missing.contains("The GitHub CLI is not installed. Public repositories load"),
            "{missing}"
        );
        assert!(!bare.contains("token") && !missing.contains("token"));
        let off = hosting_words(false, Some(Ok(hosting::demo_account())), false);
        assert!(
            off.contains("Pull requests are off: Canopy does not contact github.com."),
            "{off}"
        );
        assert!(off.contains("Open Integrations"), "{off}");
    }

    /// Every row's help reads on one line at the usual window size: a subtitle that wraps is
    /// too long for its row.
    #[test]
    fn help_fits_on_one_line() {
        let mut autostash_drawn = false;
        let mut conflict_drawn = false;
        let mut wrapped = Vec::new();
        // With a repository open, so This repository is drawn too.
        let repo = Some(Path::new("/work/canopy"));
        // With Canopy following git's configuration or choosing for itself: the rows differ.
        // At the default window (1440 points), at zoom 1.6 (900) and at the smallest window at zoom 1.6
        // (562).
        for (follow, section, width) in [true, false].into_iter().flat_map(|follow| {
            Section::ALL
                .into_iter()
                .chain([Section::Repository])
                .flat_map(move |section| {
                    [1440.0_f32, 900.0, 562.0].map(move |width| (follow, section, width))
                })
        }) {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            ctx.enable_accesskit();
            theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
            let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 6000.0));
            let mut settings = Settings {
                follow_git_config: follow,
                ..Settings::default()
            };
            let mut screen = SettingsScreen::at(section);
            screen.facts.global.asked = true;
            screen.facts.global.facts = Some(Arc::new(GitFacts::default()));
            // No git runs: the repository's configuration reads as empty, and stays read.
            screen.facts.repo.asked = true;
            screen.facts.repo.path = repo.map(Path::to_path_buf);
            screen.facts.repo.facts = Some(Arc::new(GitFacts::default()));
            screen.facts.repo.read_at = Some(Instant::now());
            let mut last = None;
            // Rows measure their controls on the first frames.
            for time in 1..=4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(bounds),
                        time: Some(f64::from(time)),
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui.ctx(),
                            &mut screen,
                            &mut settings,
                            &SettingsData {
                                tabs: &[],
                                update: &UpdateState::Idle,
                                identity: IdentityView {
                                    repo,
                                    identity: None,
                                    error: None,
                                },
                            },
                            &mut Vec::new(),
                        );
                    },
                );
                output.textures_delta.clear();
                last = output.platform_output.accesskit_update;
            }
            // Two lines of help text; a one-line label in a row of controls is a little less.
            let two_lines = f64::from(
                1.9 * ctx
                    .fonts_mut(|f| f.row_height(&FontId::proportional(theme::metrics().meta_size))),
            );
            for node in last.expect("accesskit").nodes.iter().map(|(_, n)| n) {
                let (Some(text), Some(rect)) = (node.value(), node.bounds()) else {
                    continue;
                };
                // In the narrower windows only this row is held to one line; the page's longer helps wrap.
                if width < 1440.0 && text != CONFLICT_STYLE_HELP {
                    continue;
                }
                conflict_drawn |= width < 600.0 && !follow && text == CONFLICT_STYLE_HELP;
                autostash_drawn |= text.starts_with(AUTOSTASH_HELP);
                if node.role() == egui::accesskit::Role::Label && rect.y1 - rect.y0 > two_lines {
                    wrapped.push(format!(
                        "{section:?} (follow {follow}, {width} points): {text}"
                    ));
                }
            }
        }
        assert!(wrapped.is_empty(), "{wrapped:#?}");
        assert!(autostash_drawn, "the Autostash row was not drawn");
        assert!(conflict_drawn, "the Conflict style row was not drawn");
    }

    /// While this repository's git configuration decides its pulls or pruning, Canopy's own
    /// choice is not offered beside the configuration's row, as on the Git page.
    #[test]
    fn a_repository_hides_canopys_choice_while_git_decides_it() {
        use canopy_core::{ConfigScope, ConfigSource, PullConfig, PullStrategy};
        let page = |facts: GitFacts| repository_labels(facts, Settings::default());
        let own = page(GitFacts::default());
        assert!(own.iter().any(|l| l == "Pull strategy"), "{own:?}");
        assert!(own.iter().any(|l| l == "Prune on fetch"), "{own:?}");
        let source = |key: &str, value: &str| ConfigSource {
            key: key.into(),
            value: value.into(),
            scope: ConfigScope::Local,
            origin: "file:/work/canopy/.git/config".into(),
            file: None,
        };
        let decided = page(GitFacts {
            pull: Some(PullConfig {
                strategy: Some(PullStrategy::Rebase),
                sources: vec![source("pull.rebase", "true")],
                invalid: Vec::new(),
            }),
            fetch_prune: Some(fact("fetch.prune", "false")),
            ..GitFacts::default()
        });
        assert!(!decided.iter().any(|l| l == "Pull strategy"), "{decided:?}");
        assert!(
            !decided.iter().any(|l| l == "Prune on fetch"),
            "{decided:?}"
        );
    }

    /// Turn on, in the notice that stays while trust safety is off, is reached with Tab and
    /// pressed with Enter, like any kit button.
    #[test]
    fn the_trust_off_notice_turns_on_from_the_keyboard() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let pressed = std::cell::Cell::new(None);
        let draw = |events| {
            crate::ui::kit::testkit::frame(&ctx, events, |ui| {
                if let Some(p) = trust_off_notice(ui) {
                    pressed.set(Some(p));
                }
            });
        };
        draw(Vec::new());
        draw(crate::ui::kit::testkit::key(egui::Key::Tab));
        draw(Vec::new());
        assert_eq!(pressed.get(), None, "reaching it does not press it");
        draw(crate::ui::kit::testkit::key(egui::Key::Enter));
        assert_eq!(
            pressed.get(),
            Some(TrustNotice::TurnOn),
            "Enter presses the focused button"
        );
        draw(crate::ui::kit::testkit::key(egui::Key::Tab));
        draw(Vec::new());
        draw(crate::ui::kit::testkit::key(egui::Key::Enter));
        assert_eq!(
            pressed.get(),
            Some(TrustNotice::Dismiss),
            "the next button dismisses it"
        );
    }

    /// Dismissing the trust-off notice hides it until trust safety is turned on again (owner: no
    /// notice that cannot be dismissed).
    #[test]
    fn the_trust_off_notice_can_be_dismissed_until_trust_comes_back() {
        let mut s = Settings {
            trust_safety: false,
            trust_off_dismissed: true,
            ..Settings::default()
        };
        let shown = |s: &mut Settings| {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
            let nodes = crate::ui::kit::testkit::spoken(&ctx, |ui| {
                let mut screen = SettingsScreen::default();
                let data = SettingsData {
                    tabs: &[],
                    update: &UpdateState::Idle,
                    identity: IdentityView {
                        repo: None,
                        identity: None,
                        error: None,
                    },
                };
                let mut actions = Vec::new();
                let mut page = Page {
                    screen: &mut screen,
                    settings: s,
                    data: &data,
                    actions: &mut actions,
                    sign_in_changed: Vec::new(),
                };
                page.trust_notice(ui);
            });
            crate::ui::kit::testkit::find(&nodes, "Turn on").is_some()
        };
        assert!(!shown(&mut s), "a dismissed notice stays hidden");
        s.trust_safety = true;
        assert!(!shown(&mut s));
        assert!(
            !s.trust_off_dismissed,
            "turning trust safety on clears the dismissal"
        );
        s.trust_safety = false;
        assert!(shown(&mut s), "turning it off again shows the notice again");
    }

    /// Facts whose editable values are `value_of(setting)` for every setting that has one.
    fn facts_with(value_of: impl Fn(Setting) -> Option<String>) -> GitFacts {
        GitFacts {
            git: Some((2, 55)),
            values: Setting::ALL
                .into_iter()
                .filter_map(|setting| Some(fact(setting.key(), &value_of(setting)?)))
                .collect(),
            ..GitFacts::default()
        }
    }

    /// What every select of the editor says closed, by the row's name (#232).
    fn shown(facts: GitFacts) -> Vec<(Setting, String)> {
        let nodes = git_page(facts, true);
        Setting::ALL
            .into_iter()
            .filter(|setting| *setting != Setting::InitialBranch)
            .map(|setting| {
                let node = crate::ui::kit::testkit::find(&nodes, setting.label())
                    .unwrap_or_else(|| panic!("{} is on the page", setting.label()));
                (
                    setting,
                    node.value.clone().expect("a select speaks its value"),
                )
            })
            .collect()
    }

    /// After Use recommended, and after choosing the value by hand, the value is the
    /// recommended one and the closed select says so, as it does for an unset one.
    #[test]
    fn a_selected_recommended_value_says_recommended() {
        let facts =
            facts_with(|setting| Some(setting.recommendation_for(Some((2, 55))).to_owned()));
        for (setting, said) in shown(facts) {
            let value = git_value_label(setting, setting.recommendation_for(Some((2, 55))));
            assert_eq!(
                said,
                format!("{value} (Recommended)"),
                "{}",
                setting.label()
            );
        }
    }

    /// The Git page names one group once: with Canopy choosing, Commits is one heading over its
    /// rows (#283).
    #[test]
    fn the_git_page_has_one_commits_heading_in_each_mode() {
        for follow in [true, false] {
            let nodes = git_page(GitFacts::default(), follow);
            let headings = nodes
                .iter()
                .filter(|n| {
                    [n.label.as_deref(), n.value.as_deref()]
                        .into_iter()
                        .flatten()
                        .any(|t| t.eq_ignore_ascii_case("Commits"))
                })
                .count();
            assert_eq!(headings, 1, "follow {follow}");
        }
    }

    /// A commit cleanup value has one name wherever it appears: the list of git values, the note
    /// and notice that say where it is set, and Canopy's own Message cleanup row (#283).
    #[test]
    fn a_commit_cleanup_value_has_one_name() {
        for cleanup in canopy_core::Cleanup::ALL {
            let name = git_value_label(Setting::CommitCleanup, cleanup.value());
            assert_eq!(name, cleanup.label(), "{}", cleanup.value());
            assert!(
                config_phrase(&fact("commit.cleanup", cleanup.value())).starts_with(&format!(
                    "Commit message cleanup set to \"{}\" in ",
                    cleanup.label()
                )),
                "{}",
                cleanup.value()
            );
        }
        // The help under the rows says the value by that name too, never by another.
        for help in [
            git_setting_help(Setting::CommitCleanup),
            MESSAGE_CLEANUP_HELP,
        ] {
            for other in [
                "Whitespace",
                "whitespace",
                "Strip",
                "strip",
                "Verbatim",
                "verbatim",
            ] {
                assert!(!help.contains(other), "{help}: {other}");
            }
        }
        assert!(
            git_setting_help(Setting::CommitCleanup)
                .contains(canopy_core::Cleanup::Whitespace.label()),
            "the Git list's help names the value as the row does"
        );
        // Git's own two have names of their own, and none of the five repeats another.
        let names: HashSet<_> = Setting::CommitCleanup
            .values()
            .iter()
            .map(|v| git_value_label(Setting::CommitCleanup, v))
            .collect();
        assert_eq!(
            names.len(),
            Setting::CommitCleanup.values().len(),
            "{names:?}"
        );
    }

    /// Any other value shows bare, and the list still marks the recommended entry.
    #[test]
    fn a_selected_value_that_is_not_recommended_says_nothing_more() {
        let recommended = |setting: Setting| setting.recommendation_for(Some((2, 55)));
        let facts = facts_with(|setting| {
            setting
                .values()
                .iter()
                .find(|value| **value != recommended(setting))
                .map(|value| (*value).to_owned())
        });
        for (setting, said) in shown(facts) {
            let other = setting
                .values()
                .iter()
                .find(|value| **value != recommended(setting))
                .expect("a second value");
            assert_eq!(said, git_value_label(setting, other), "{}", setting.label());
        }
    }

    /// The first branch is a text field: "(Recommended)" sits under it while it holds `main`.
    #[test]
    fn the_first_branch_is_marked_while_it_holds_the_recommendation() {
        for (branch, marked) in [("main", true), ("trunk", false)] {
            let facts = facts_with(|setting| {
                (setting == Setting::InitialBranch).then(|| branch.to_owned())
            });
            let nodes = git_page(facts, true);
            assert_eq!(
                nodes
                    .iter()
                    .any(|node| node.description.as_deref() == Some("(Recommended)")),
                marked,
                "{branch}: {nodes:?}"
            );
        }
    }

    /// What the selects show, what the global file holds and what Git uses agree after Use
    /// recommended (temporary global configuration).
    #[test]
    fn the_selects_show_what_git_uses_after_the_recommendations_are_written() {
        if !crate::repo_config::in_isolated_git_env(
            "views::settings_screen::tests::the_selects_show_what_git_uses_after_the_recommendations_are_written",
        ) {
            return;
        }
        let home = crate::settings::home_dir();
        config::apply_recommended(&home).expect("written");
        let facts = crate::repo_config::read(None);
        assert_eq!(facts.error, None);
        for (setting, said) in shown(facts) {
            let effective = cli::run(&home, &["config", "--get", setting.key()]);
            match setting.recommended_write() {
                Some(value) => {
                    assert_eq!(effective.expect("set").trim(), value, "{}", setting.key());
                    assert_eq!(
                        said,
                        format!("{} (Recommended)", git_value_label(setting, value)),
                        "{}",
                        setting.label()
                    );
                }
                // Left to Git: unset, and the row says the value Git uses.
                None => assert!(effective.is_err(), "{} stays unset", setting.key()),
            }
        }
    }
}
