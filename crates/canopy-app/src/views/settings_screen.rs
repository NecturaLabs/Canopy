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

use canopy_core::forge::{Forge, ForgeKind};
use canopy_core::identity::{Field, Identity, Scope, Source};

use crate::action::Action;
use crate::hosting::{self, TokenStore};
use crate::keymap::{Command, Section as KeySection};
use crate::repo_config::{ConfigValue, GitFacts};
use crate::settings::{CloneProtocol, DiffPrefs, PullChoice, PullMode, Settings, WhitespaceMode};
use crate::system_apps::Kind;
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::Badge;
use crate::ui::kit::choice::Segmented;
use crate::ui::kit::field::{NumberField as KitNumber, SearchField, TextField};
use crate::ui::kit::overlay;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::paint;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::range::Slider;
use crate::ui::kit::row::{ListRow, RowKind};
use crate::ui::kit::select::{self, Select};
use crate::ui::kit::surface::{Callout, Card, EmptyAction, EmptyState, column_label};
use crate::ui::kit::toggle::Switch;
use crate::ui::kit::{Button, IconButton, Size, Tone};
use crate::ui::motion::{self, MotionPref};
use crate::ui::theme::{
    self, Density, Elevation, Fill, GlassBlocked, Palette, Style, Surfaces, ThemeId, palette,
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
    const ALL: [Self; 8] = [
        Self::Appearance,
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

#[derive(Debug)]
enum TokenTest {
    Running(Receiver<Result<String, String>>),
    Done(Result<String, String>),
}

/// State of the open settings screen.
#[derive(Default)]
pub struct SettingsScreen {
    section: Section,
    zoom_preview: Option<f32>,
    picker: crate::ui::folder_picker::FolderPicker,
    token_input: HashMap<String, String>,
    tests: HashMap<String, TokenTest>,
    token_error: Option<String>,
    identity_for: Option<PathBuf>,
    loaded_identity: Option<Identity>,
    identity_name: String,
    identity_email: String,
    identity_scope: Scope,
    search: Search,
    facts: FactsCache,
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
    /// The URL last saved for a remote, and when, until the configuration shows it.
    remote_saved: Option<(String, String, Instant)>,
    /// An export or import waiting on its file chooser.
    file_task: Option<crate::settings_file::FileTask>,
    /// What the last export, import or reset did: `(failed, message)`.
    file_status: Option<(bool, String)>,
    /// Stay on This repository until a repository is open (the screenshot overlay opens
    /// Settings before its repository's tab); cleared once one is.
    awaiting_repository: bool,
}

/// Why the active repository's authors show initials instead of pictures
/// (`crate::avatars::explain`, contracts/avatars.md), shown under "Download profile pictures".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureNote {
    pub title: String,
    pub detail: String,
    /// A token under Accounts would bring the pictures (`Fix::Accounts`).
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
    facts: Option<GitFacts>,
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
    fn get(&mut self, ctx: &egui::Context, path: Option<&Path>) -> Option<&GitFacts> {
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
        if let Some(facts) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.facts = Some(facts);
            self.rx = None;
            self.read_at = Some(Instant::now());
        }
        self.facts.as_ref()
    }
}

impl FactsCache {
    fn global(&mut self, ctx: &egui::Context) -> Option<&GitFacts> {
        self.global.get(ctx, None)
    }

    fn repo(&mut self, ctx: &egui::Context, path: &Path) -> Option<&GitFacts> {
        self.repo.get(ctx, Some(path))
    }
}

/// The search over every section: what is typed, and what the last frame found.
#[derive(Debug, Default)]
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
    // The token fields hold secrets being typed: show only which hosts have one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsScreen")
            .field("section", &self.section)
            .field("token_input", &self.token_input.keys().collect::<Vec<_>>())
            .field("tests", &self.tests)
            .field("token_error", &self.token_error)
            .field("identity_scope", &self.identity_scope)
            .field("search", &self.search)
            .finish_non_exhaustive()
    }
}

impl SettingsScreen {
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

    /// The screen opened at This repository, staying there until the repository's tab opens
    /// (the settings-repository screenshot).
    pub fn awaiting_repository() -> Self {
        Self {
            section: Section::Repository,
            awaiting_repository: true,
            ..Self::default()
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
    pub tokens_changed: bool,
}

pub struct IdentityView<'a> {
    pub repo: Option<&'a Path>,
    pub identity: Option<&'a Identity>,
    pub error: Option<&'a str>,
}

pub struct SettingsData<'a> {
    pub forges: &'a [Forge],
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
    tokens: &mut TokenStore,
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
        crate::system_apps::refresh(Some(ctx));
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
                    .max_rect(body)
                    .layout(Layout::top_down(Align::Min)),
            );
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            let words = screen.search.words();
            let scroll_salt = words.is_empty().then_some(screen.section);
            overlay::scroll_body(
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
                                tokens,
                                data,
                                actions,
                                tokens_changed: false,
                            };
                            if words.is_empty() {
                                Filter::set(ui.ctx(), Filter::default());
                                page.section(ui, page.screen.section);
                            } else {
                                page.results(ui, &words, body.height());
                            }
                            result.tokens_changed |= page.tokens_changed;
                        });
                },
            );
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
    result
}

/// Everything a section's page draws from and writes to.
struct Page<'a, 'd> {
    screen: &'a mut SettingsScreen,
    settings: &'a mut Settings,
    tokens: &'a mut TokenStore,
    data: &'a SettingsData<'d>,
    actions: &'a mut Vec<Action>,
    tokens_changed: bool,
}

impl Page<'_, '_> {
    /// One section's rows (all of them, or while searching those that match).
    fn section(&mut self, ui: &mut Ui, section: Section) {
        let Self {
            screen,
            settings,
            tokens,
            data,
            actions,
            tokens_changed,
        } = self;
        match section {
            Section::Appearance => appearance(ui, settings, &mut screen.zoom_preview),
            Section::Git => git(ui, screen, settings, &data.identity, actions),
            Section::Integrations => integrations(
                ui,
                settings,
                &mut screen.picker,
                &mut screen.custom_launchers,
            ),
            Section::Accounts => {
                *tokens_changed |= accounts(ui, screen, settings, tokens, data.forges, actions);
            }
            Section::Updates => updates(ui, settings, data.update, actions),
            Section::Privacy => {
                let open_accounts = privacy(
                    ui,
                    settings,
                    actions,
                    &mut screen.picker,
                    screen.picture_note.as_ref(),
                );
                if open_accounts {
                    screen.section = Section::Accounts;
                }
            }
            Section::Keyboard => keyboard(ui),
            Section::Data => backup(ui, screen, settings, actions),
            Section::Repository => repository(ui, screen, settings, tokens, data, actions),
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

/// Lower-case words of a query.
fn search_words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Whether every word occurs in one of `texts` (case-insensitive).
fn matches_all(words: &[String], texts: &[&str]) -> bool {
    let haystack = texts.join("\n").to_lowercase();
    words.iter().all(|w| haystack.contains(w.as_str()))
}

/// Other words people use for a setting, so a search finds it by them too.
fn synonyms(title: &str) -> &'static str {
    match title {
        "Theme" => {
            "colour color dark light mode midnight daylight aurora ember neon forest paper contrast"
        }
        "Style" => "standard deeper flat depth shadows look",
        "Translucent surfaces" => "glass blur transparency transparent frosted",
        "Density" => "compact dense comfortable spacing row height",
        "Motion" => "animation animations reduce reduced motion",
        "Zoom" => "scale size bigger smaller font text",
        "Dates" | "Commit date format" => "time relative calendar iso",
        "Author avatars" | "Download profile pictures" => {
            "avatar picture photo gravatar image profile"
        }
        "Name" | "Email" | "Apply identity" | "Save to" | "Committer" => {
            "identity user author git config user.name user.email"
        }
        "Pull strategy" => "pull rebase merge fast-forward ff only",
        "Follow my git config" => "pull rebase pull.rebase pull.ff configuration follow default",
        "Prune on fetch" => "prune stale remote branches delete",
        "Fetch in the background" => "auto fetch automatic interval minutes",
        "Add Signed-off-by" => "signoff sign-off dco trailer",
        "Commits per page" => "history load more limit",
        "Layout" => "diff side by side unified split",
        "Syntax highlighting" => "colour color code language",
        "Context lines" => "diff context unchanged lines",
        "Whitespace" => "spaces tabs ignore diff",
        "Default clone folder" => "clone directory path folder location",
        "Clone protocol" => "ssh https url remote",
        "Self-hosted GitLab" => "gitlab host enterprise server",
        "Pull and merge requests" => "pr mr github gitlab api hosting offline",
        "Terminal" => "console shell command line $terminal kitty alacritty",
        "Editor" => "ide vscode code zed sublime vim open file $editor $visual",
        "File manager" => "folder explorer finder nautilus files dolphin show reveal",
        "Trusted folders" => "trust partial clone security",
        "Send crash reports" => "crash sentry telemetry error report",
        "Log detail" => "log logging debug verbose diagnostics",
        "Logs and crash reports" => "log logs diagnostics folder copy",
        "Privacy Policy" | "Terms of Service" => {
            "legal privacy terms document consent accepted agreement"
        }
        "Export settings" | "Import settings" => {
            "backup file json move computer copy transfer save load"
        }
        "Reset all settings" => "defaults factory start over restore clear",
        "Check for updates" | "Include beta builds" => "update upgrade version release beta",
        "Tokens" => "token account sign in personal access github gitlab api pat credential",
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

/// Counts `n` matches found by other means (the shortcut list) and draws what they sit under.
fn reveal_rows(ui: &mut Ui, n: usize) {
    let mut f = Filter::get(ui.ctx());
    f.count += n;
    f.shown = true;
    let f = open_group(ui, f);
    Filter::set(ui.ctx(), f);
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
                    .font(FontId::new(m.subtitle_size, theme::semibold()))
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
        FontId::new(m.body_size, theme::semibold()),
        p.text,
    );
    egui::ScrollArea::vertical()
        .id_salt("settings-sections")
        .show(&mut ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let searching = !screen.search.words().is_empty();
            for section in Section::shown(repo.is_some()) {
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
                .font(FontId::new(m.body_size, theme::semibold()))
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
    if std::mem::take(&mut screen.search.focus) {
        ui.memory_mut(|m| m.request_focus(crate::ui::kit::field::edit_id(id)));
    }
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

/// The widest an explanation runs: a comfortable line length, however wide the column.
const READING_WIDTH: f32 = 560.0;

/// A preference and its controls. Compact columns stack the controls below the help.
fn row(ui: &mut Ui, title: &str, help: &str, control: impl FnOnce(&mut Ui)) {
    row_more(ui, title, help, "", control);
    row_end(ui);
}

/// Every setting's default, for the buttons that go back to it.
static DEFAULTS: LazyLock<Settings> = LazyLock::new(Settings::default);

/// A button back to a setting's default, shown while the setting differs from it (drawn right
/// to left, so it sits before the control). Returns true when it was clicked.
fn revert<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, default: &T, name: &str) -> bool {
    if value == default {
        return false;
    }
    let clicked = IconButton::new(
        icon::ARROW_COUNTER_CLOCKWISE,
        format!("Reset {name} to its default"),
    )
    .size(Size::Sm)
    .id_salt(("revert", name))
    .show(ui)
    .clicked();
    if clicked {
        value.clone_from(default);
    }
    clicked
}

/// The tag on a global setting that each repository can override.
const DEFAULT_TAG: &str = "Default for repositories";
const OVERRIDDEN_TIP: &str = "This repository has its own value.";
const TAG_TIP: &str =
    "Each repository follows this unless it chooses otherwise in Settings → This repository.";

/// [`row`] for a setting that is the default for repositories: tagged as such.
fn default_row(
    ui: &mut Ui,
    title: &str,
    help: &str,
    note: Option<&Note>,
    control: impl FnOnce(&mut Ui),
) {
    let help_text = |ui: &mut Ui| {
        help_label(ui, help);
        if let Some(note) = note {
            note.show(ui);
        }
    };
    row_layout(
        ui,
        title,
        Some(DEFAULT_TAG),
        &format!("{help} {DEFAULT_TAG}"),
        &help_text,
        "",
        control,
    );
    row_end(ui);
}

/// A line under a setting's help about what git's own configuration says: a warning when git
/// will act otherwise than the setting, else a quiet remark.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    text: String,
    warn: bool,
}

impl Note {
    fn show(&self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let m = theme::metrics();
        let (glyph, color) = if self.warn {
            (icon::WARNING, p.warning)
        } else {
            (icon::INFO, p.text_faint)
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = theme::XS;
            ui.label(RichText::new(glyph).size(m.meta_size).color(color));
            ui.add(
                egui::Label::new(
                    RichText::new(&self.text)
                        .size(m.meta_size)
                        .color(if self.warn { p.text_dim } else { p.text_faint }),
                )
                .wrap(),
            );
        });
    }
}

/// `key = value (where it is set)`, for a note.
fn config_phrase(v: &ConfigValue) -> String {
    format!(
        "{} = {} ({})",
        v.key,
        v.value,
        source_label(Some(&v.source()))
    )
}

/// What git's configuration says about pulls, against Canopy's `choice`. Canopy always passes a
/// strategy (`canopy_core::Repo::pull`): the configuration's when it follows it and one is set,
/// else its own, and then a configuration that differs does not apply.
fn pull_note(facts: &GitFacts, choice: PullChoice) -> Option<Note> {
    let config = facts.pull.as_ref()?;
    let ours = choice.mode.strategy();
    let Some((configured, described)) = config.strategy.zip(config.describe()) else {
        return choice.follow_config.then(|| Note {
            text: format!(
                "Your git configuration sets no pull strategy, so pulls use {}.",
                lowercase_label(ours.label())
            ),
            warn: false,
        });
    };
    Some(if choice.follow_config {
        Note {
            text: format!(
                "From your git configuration: {described}, so pulls use {}.",
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
                "Canopy passes {}, so your git configuration's {described} does not apply to \
                 pulls from Canopy.",
                ours.git_args().join(" ")
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

/// What git's configuration says about pruning, against Canopy's choice. Canopy always passes
/// `--prune` or `--no-prune`, so a `fetch.prune` that differs does not apply to its fetches.
fn prune_note(facts: &GitFacts, prune: bool) -> Option<Note> {
    let v = facts.fetch_prune.as_ref()?;
    let git_prunes = v.as_bool().unwrap_or(false);
    Some(if git_prunes == prune {
        Note {
            text: format!("Matches your git configuration: {}.", config_phrase(v)),
            warn: false,
        }
    } else {
        Note {
            text: format!(
                "Canopy passes {}, so your git configuration's {} does not apply to fetches \
                 from Canopy.",
                if prune { "--prune" } else { "--no-prune" },
                config_phrase(v)
            ),
            warn: true,
        }
    })
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

/// A row whose `help` is one sentence. A longer `detail` sits behind a "More" link and, once
/// opened, runs beneath the row at reading width. The caller ends the row with [`row_end`].
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
    if !reveal(ui, title, &[text, detail]) {
        return;
    }
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let words = Filter::get(ui.ctx()).words;
    let width = ui.available_width();
    let more_id = ui.make_persistent_id(("settings-more", title));
    let mut open = !detail.is_empty()
        && ui
            .ctx()
            .data(|d| d.get_temp::<bool>(more_id))
            .unwrap_or(false);
    let label = |ui: &mut Ui, open: &mut bool| {
        ui.spacing_mut().item_spacing.y = theme::XS;
        let title = marked(
            title,
            &words,
            &FontId::new(m.body_size, theme::semibold()),
            p.text,
            p.accent,
            p.accent_tint(),
        );
        match tag {
            Some(tag) => {
                let line = Layout::left_to_right(Align::Center).with_main_wrap(true);
                ui.with_layout(line, |ui| {
                    ui.spacing_mut().item_spacing.x = theme::SM;
                    ui.label(title);
                    let (tip, tone) = if tag == DEFAULT_TAG {
                        (TAG_TIP, Tone::Neutral)
                    } else {
                        (OVERRIDDEN_TIP, Tone::Accent)
                    };
                    Badge::new(tag).tone(tone).tooltip(tip).show(ui);
                });
            }
            None => {
                ui.label(title);
            }
        }
        help(ui);
        if !detail.is_empty()
            && LinkButton::new(if *open { "Less" } else { "More" })
                .text_size(m.meta_size)
                .show(ui)
                .clicked()
        {
            *open = !*open;
        }
    };
    ui.push_id(title, |ui| {
        if width < 620.0 {
            label(ui, &mut open);
            ui.add_space(theme::XS);
            ui.allocate_ui_with_layout(
                vec2(width, 0.0),
                Layout::right_to_left(Align::Center).with_main_wrap(true),
                |ui| {
                    ui.add_space(theme::SM);
                    control(ui);
                },
            );
        } else {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(width * 0.50, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        // Without this the column is only as wide as its text, and the controls
                        // of rows with short help text sit at different distances from the edge.
                        ui.set_min_width(width * 0.50);
                        label(ui, &mut open);
                    },
                );
                ui.allocate_ui_with_layout(
                    vec2(width * 0.50 - 2.0 * theme::SM, 0.0),
                    Layout::right_to_left(Align::Center).with_main_wrap(true),
                    |ui| {
                        // Keeps controls clear of the scroll bar.
                        ui.add_space(theme::SM);
                        control(ui);
                    },
                );
            });
        }
        if open {
            ui.add_space(theme::XS);
            ui.scope(|ui| {
                ui.set_max_width((width - 2.0 * theme::SM).min(READING_WIDTH));
                help_label(ui, detail);
            });
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(more_id, open));
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
    column_label(ui, text);
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

/// The fewest and most commits a history page loads, and the stepper's step.
const COMMIT_PAGE: RangeInclusive<f64> = 200.0..=50_000.0;
const COMMIT_PAGE_STEP: f64 = 100.0;

fn appearance(ui: &mut Ui, s: &mut Settings, preview: &mut Option<f32>) {
    heading(ui, "Theme");
    if block(ui, "Theme", "") {
        theme_cards(ui, s);
        if s.theme != DEFAULTS.theme {
            let m = theme::metrics();
            ui.add_space(theme::SM);
            if LinkButton::new(format!(
                "Back to the default theme, {}",
                DEFAULTS.theme.name()
            ))
            .text_size(m.meta_size)
            .show(ui)
            .clicked()
            {
                s.theme = DEFAULTS.theme;
            }
        }
    }
    group_gap(ui, theme::LG);
    heading(ui, "Look");
    row(
        ui,
        "Style",
        "Standard: flat, two-tone chrome. Deeper: floating panes, gradient controls and layered shadows.",
        |ui| {
            let options = Style::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.style, &options)
                .label("Style")
                .show(ui);
            revert(ui, &mut s.style, &DEFAULTS.style, "Style");
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
            None => "Blur what is behind menus, dialogs, toasts and the palette.",
        },
        |ui| {
            let on = s.translucent;
            switch_with(
                Switch::new(&mut s.translucent, "")
                    .enabled(blocked.is_none())
                    .disabled_reason(blocked.map_or("", GlassBlocked::reason)),
                ui,
                on,
                "Translucent surfaces",
            );
            revert(
                ui,
                &mut s.translucent,
                &DEFAULTS.translucent,
                "Translucent surfaces",
            );
        },
    );
    row(
        ui,
        "Density",
        "Dense fits about a quarter more history: 24-point rows, a single-row toolbar and smaller text.",
        |ui| {
            let options = Density::ALL.map(|v| (v, v.label()));
            Segmented::new(&mut s.density, &options)
                .label("Density")
                .show(ui);
            revert(ui, &mut s.density, &DEFAULTS.density, "Density");
        },
    );
    let system = match motion::system() {
        Some(true) => "Your system asks for reduced motion.",
        Some(false) => "Your system asks for full motion.",
        None => "Your system did not say whether to reduce motion.",
    };
    row(ui, "Motion", system, |ui| {
        let options = MotionPref::ALL.map(|v| (v, v.label()));
        Segmented::new(&mut s.motion, &options)
            .label("Motion")
            .show(ui);
        revert(ui, &mut s.motion, &DEFAULTS.motion, "Motion");
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
            zoom_slider(ui, s, preview);
            if revert(ui, &mut s.zoom, &DEFAULTS.zoom, "Zoom") {
                *preview = None;
            }
        },
    );
    row(
        ui,
        "Dates",
        "How the history shows when a commit was made. The exact time is in the tooltip.",
        |ui| {
            Segmented::new(
                &mut s.relative_dates,
                &[(true, "Relative"), (false, "Calendar")],
            )
            .label("Dates")
            .show(ui);
            revert(ui, &mut s.relative_dates, &DEFAULTS.relative_dates, "Dates");
        },
    );
    row(
        ui,
        "Commit date format",
        "Calendar dates in History and commit details, including older relative dates. Exact timestamp tooltips always include the author's UTC offset.",
        |ui| {
            Segmented::new(
                &mut s.iso_dates,
                &[(false, "Sep 21, 2026"), (true, "2026-09-21")],
            )
            .label("Commit date format")
            .show(ui);
            revert(
                ui,
                &mut s.iso_dates,
                &DEFAULTS.iso_dates,
                "Commit date format",
            );
        },
    );
    row(
        ui,
        "Author avatars",
        "Pictures or coloured initials next to authors.",
        |ui| {
            switch(ui, &mut s.show_avatars, "Author avatars");
            revert(
                ui,
                &mut s.show_avatars,
                &DEFAULTS.show_avatars,
                "Author avatars",
            );
        },
    );
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
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for themes in ThemeId::ALL.chunks(columns) {
            ui.horizontal(|ui| {
                for &t in themes {
                    if theme_card(ui, t, s.theme == t, card_width).clicked() {
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
const THEME_CARD_MIN: f32 = 150.0;
const THEME_CARD_MAX: f32 = 200.0;
/// The miniature's height, and the size of the window it stands for (concept `.tcard svg`).
const MINI_HEIGHT: f32 = 58.0;
const MINI_VIEW: Vec2 = vec2(200.0, 62.0);
/// The selected card's tick: its diameter and its inset from the miniature's corner.
const TICK: f32 = 18.0;
const TICK_INSET: f32 = 5.0;

/// A theme card: a miniature of the window drawn from that theme's palette in the current
/// style, its name and whether it is dark or light. Selected: the card's focus ring and a tick.
fn theme_card(ui: &mut Ui, id: ThemeId, selected: bool, width: f32) -> Response {
    let tp = id.palette();
    let card = Card::new()
        .interactive(true)
        .selected(selected)
        .width(width)
        .label(format!("{} theme", id.name()))
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
                if tp.dark { "Dark" } else { "Light" },
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
            // An unknown home directory is an empty path, which every path starts with.
            let file = Path::new(file)
                .strip_prefix(home)
                .ok()
                .filter(|_| !home.as_os_str().is_empty())
                .map_or_else(|| file.to_owned(), |rest| format!("~/{}", rest.display()));
            let scope = match scope.as_str() {
                "local" => "this repository",
                "worktree" => "this worktree",
                other => other,
            };
            format!("{scope}: {file}")
        }
        Some(Source::Guessed) => "guessed by Git from this computer".into(),
        None => "not set".into(),
    }
}

/// What Git would use now for one part of an identity, and why: the value, then where it is
/// set, cut in the middle to the column with the whole of it in a tooltip.
fn in_use(ui: &mut Ui, field: &Field) {
    let p = palette(ui.ctx());
    let size = theme::metrics().small_size;
    if field.value.is_empty() {
        ui.label(RichText::new("Not set.").size(size).color(p.text_dim));
        return;
    }
    ui.add(
        egui::Label::new(
            RichText::new(format!("In use now: {}", field.value))
                .size(size)
                .color(p.text_dim),
        )
        .truncate(),
    );
    let source = format!("From {}", source_label(field.source.as_ref()));
    let shown = elide_middle(
        ui,
        &source,
        &FontId::proportional(size),
        ui.available_width(),
    );
    let cut = shown != source;
    let resp = ui.add(egui::Label::new(
        RichText::new(shown).size(size).color(p.text_faint),
    ));
    if cut {
        resp.kit_tooltip(source).show();
    }
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
/// The width of a select in a settings row.
const SELECT_WIDTH: f32 = 220.0;

/// A select's width in a settings row: [`SELECT_WIDTH`], so a column of rows lines up, or wider
/// when its longest option needs it (`kit::select::fit_width`).
fn select_width<'l>(ui: &Ui, labels: impl IntoIterator<Item = &'l str>) -> f32 {
    SELECT_WIDTH.max(select::fit_width(ui, labels))
}

/// The width of the commits-per-page field, its steppers included.
const COUNT_FIELD_WIDTH: f32 = 160.0;

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
    let current = |pick: fn(&Identity) -> &Field| {
        move |ui: &mut Ui| {
            if let Some(field) = identity.map(pick) {
                in_use(ui, field);
            }
        }
    };
    // What git would use while the field is empty (a guess from the user and host names, or a
    // value nobody can save here) shows as its placeholder, never as a value to save.
    let unchosen = |pick: fn(&Identity) -> &Field| {
        identity
            .map(pick)
            .filter(|f| !f.explicit() && !f.value.is_empty())
            .map_or_else(String::new, |f| format!("{} (guessed by git)", f.value))
    };
    row_layout(
        ui,
        "Name",
        None,
        "",
        &current(|i| &i.author.name),
        "",
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
    row_layout(
        ui,
        "Email",
        None,
        "",
        &current(|i| &i.author.email),
        "",
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
            "This repository writes its own .git/config; all repositories write your global Git configuration.",
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
        "Save these fields to Git configuration. Changes apply to future commits only; existing commits keep their authors. Environment variables or author settings may override this identity.",
        |ui| {
            // Right to left: the primary action is rightmost.
            if Button::new("Save identity")
                .primary()
                .enabled(can_save)
                .tooltip("Write this identity to the selected Git configuration.")
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
    heading(ui, "Remote");
    facts_error(ui, facts.as_ref());
    default_row(
        ui,
        "Follow my git config",
        "Pull with the strategy your git configuration sets (branch.<name>.rebase, pull.rebase, pull.ff) when it sets one.",
        facts
            .as_ref()
            .and_then(|f| pull_note(f, s.pull_default()))
            .as_ref(),
        |ui| {
            switch(ui, &mut s.pull_follow_config, "Follow my git config");
            revert(
                ui,
                &mut s.pull_follow_config,
                &DEFAULTS.pull_follow_config,
                "Follow my git config",
            );
        },
    );
    default_row(
        ui,
        "Pull strategy",
        "How a pull brings in the remote's commits: a merge commit, a rebase of yours on top, or only a fast-forward. While Follow my git config is on, only where your git configuration sets none.",
        None,
        |ui| {
            let options = PullMode::ALL.map(|m| (m, m.label()));
            Select::new(&mut s.pull_mode, &options)
                .width(select_width(ui, options.iter().map(|(_, l)| *l)))
                .name("Pull strategy")
                .id_salt("pull-strategy")
                .show(ui);
            revert(ui, &mut s.pull_mode, &DEFAULTS.pull_mode, "Pull strategy");
        },
    );
    default_row(
        ui,
        "Prune on fetch",
        "Remove remote-tracking branches that were deleted on the remote.",
        facts
            .as_ref()
            .and_then(|f| prune_note(f, s.prune_on_fetch))
            .as_ref(),
        |ui| {
            switch(ui, &mut s.prune_on_fetch, "Prune on fetch");
            revert(
                ui,
                &mut s.prune_on_fetch,
                &DEFAULTS.prune_on_fetch,
                "Prune on fetch",
            );
        },
    );
    default_row(
        ui,
        "Fetch in the background",
        "Fetches every remote of open repositories while Canopy is idle.",
        None,
        |ui| {
            let labels = fetch_intervals();
            let options: Vec<(u32, &str)> = labels.iter().map(|(m, l)| (*m, l.as_str())).collect();
            // A stored interval the list does not offer still shows as itself.
            let current = fetch_interval_label(s.auto_fetch_minutes);
            Select::new(&mut s.auto_fetch_minutes, &options)
                .placeholder(current)
                .width(SELECT_WIDTH)
                .name("Fetch in the background")
                .id_salt("auto-fetch")
                .show(ui);
            revert(
                ui,
                &mut s.auto_fetch_minutes,
                &DEFAULTS.auto_fetch_minutes,
                "Fetch in the background",
            );
        },
    );
    heading(ui, "Commits");
    default_row(
        ui,
        "Add Signed-off-by",
        "Appends a Signed-off-by trailer (git commit --signoff), as DCO projects require.",
        None,
        |ui| {
            switch(ui, &mut s.signoff, "Add Signed-off-by");
            revert(ui, &mut s.signoff, &DEFAULTS.signoff, "Add Signed-off-by");
        },
    );
    row(
        ui,
        "Commits per page",
        "How many commits the history loads before \"Load more\".",
        |ui| {
            commit_page(ui, s);
            revert(
                ui,
                &mut s.commit_page,
                &DEFAULTS.commit_page,
                "Commits per page",
            );
        },
    );
    heading(ui, "Diffs");
    row(
        ui,
        "Layout",
        "Unified shows old and new lines in one column; side by side puts the old file on the left and the new on the right.",
        |ui| {
            Segmented::new(
                &mut s.split_diff,
                &[(false, "Unified"), (true, "Side by side")],
            )
            .label("Diff layout")
            .show(ui);
            revert(ui, &mut s.split_diff, &DEFAULTS.split_diff, "Diff layout");
        },
    );
    row(
        ui,
        "Syntax highlighting",
        "Colours code in diffs by language (Rust, TypeScript, Python, Markdown and about 200 more).",
        |ui| {
            switch(ui, &mut s.syntax_highlight, "Syntax highlighting");
            revert(
                ui,
                &mut s.syntax_highlight,
                &DEFAULTS.syntax_highlight,
                "Syntax highlighting",
            );
        },
    );
    default_row(
        ui,
        "Context lines",
        "Unchanged lines shown around each change. A repository can also change it from its diff header.",
        None,
        |ui| {
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
            revert(
                ui,
                &mut s.diff.context,
                &DEFAULTS.diff.context,
                "Context lines",
            );
        },
    );
    default_row(
        ui,
        "Whitespace",
        "Ignoring whitespace turns off hunk staging, since such a diff cannot be applied.",
        None,
        |ui| {
            let options = WhitespaceMode::ALL.map(|m| (m, m.label()));
            Select::new(&mut s.diff.whitespace, &options)
                .width(SELECT_WIDTH)
                .name("Whitespace")
                .id_salt("whitespace")
                .show(ui);
            revert(
                ui,
                &mut s.diff.whitespace,
                &DEFAULTS.diff.whitespace,
                "Whitespace",
            );
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
    tokens: &TokenStore,
    data: &SettingsData<'_>,
    actions: &mut Vec<Action>,
) {
    let Some(path) = data.identity.repo else {
        return;
    };
    let facts = screen.facts.repo(ui.ctx(), path).cloned();
    if block(ui, "This repository", &path.display().to_string()) {
        repo_header(ui, path, &RepoLook::of(s, path));
        facts_error(ui, facts.as_ref());
    }
    repo_tab(ui, s, path);
    repo_remotes(ui, screen, path, facts.as_ref(), actions);
    repo_identity(ui, screen, path, &data.identity, actions);

    let mut own = s.repo(path);
    group_gap(ui, theme::LG);
    heading(ui, "Pull and fetch");
    let pull = s.pull_for(path);
    repo_row(
        ui,
        "Follow my git config",
        "Pull with the strategy this repository's git configuration sets, when it sets one.",
        facts.as_ref().and_then(|f| pull_note(f, pull)).as_ref(),
        |ui| {
            override_select(
                ui,
                "repo-pull-follow",
                "Follow my git config",
                &mut own.pull_follow_config,
                s.pull_follow_config,
                &on_off(),
            );
        },
    );
    repo_row(
        ui,
        "Pull strategy",
        "How a pull brings in the remote's commits; while it follows the git configuration, only where that sets none.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-pull",
                "Pull strategy",
                &mut own.pull_mode,
                s.pull_mode,
                &PullMode::ALL.map(|m| (m, m.label().to_owned())),
            );
        },
    );
    let prune = own.prune_on_fetch.unwrap_or(s.prune_on_fetch);
    repo_row(
        ui,
        "Prune on fetch",
        "Remove remote-tracking branches that were deleted on the remote.",
        facts.as_ref().and_then(|f| prune_note(f, prune)).as_ref(),
        |ui| {
            override_select(
                ui,
                "repo-prune",
                "Prune on fetch",
                &mut own.prune_on_fetch,
                s.prune_on_fetch,
                &on_off(),
            );
        },
    );
    repo_row(
        ui,
        "Fetch in the background",
        "Fetches this repository's remotes while Canopy is idle.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-auto-fetch",
                "Fetch in the background",
                &mut own.auto_fetch_minutes,
                s.auto_fetch_minutes,
                &fetch_intervals(),
            );
        },
    );
    repo_row(
        ui,
        "Remote protocol",
        "For remote URLs built from owner/repository when you add a remote or publish.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-protocol",
                "Remote protocol",
                &mut own.protocol,
                s.clone_protocol,
                &CloneProtocol::ALL.map(|p| (p, p.label().to_owned())),
            );
        },
    );
    heading(ui, "Commits");
    repo_row(
        ui,
        "Add Signed-off-by",
        "Appends a Signed-off-by trailer (git commit --signoff), as DCO projects require.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-signoff",
                "Add Signed-off-by",
                &mut own.signoff,
                s.signoff,
                &on_off(),
            );
        },
    );
    if let Some(facts) = &facts {
        signing_row(ui, facts);
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
        "hosting account token github gitlab",
    ) {
        hosting_account(ui, screen, s, tokens, facts.as_ref());
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
    row(
        ui,
        "Name",
        "What its tab calls this repository. Empty uses the folder's name.",
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut name)
                    .placeholder(folder.clone())
                    .width(FIELD_WIDTH)
                    .name("Tab name")
                    .id_salt("repo-tab-name")
                    .show(ui)
            });
        },
    );
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
            .width(SELECT_WIDTH)
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
        let help = format!(
            "Where {} fetches from and pushes to. {}",
            remote.name,
            if changed { "Not saved yet." } else { "" }
        );
        let mut save = false;
        row_layout(
            ui,
            &remote.name,
            None,
            "remote url origin upstream set-url",
            &|ui: &mut Ui| {
                help_label(ui, help.trim_end());
                if let Some(problem) = problem.filter(|_| changed) {
                    Note {
                        text: problem.to_owned(),
                        warn: true,
                    }
                    .show(ui);
                }
            },
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
                    .font(FontId::new(m.subtitle_size, theme::semibold()))
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
    help_label(
        ui,
        "These settings apply to this repository only. Anything left at Default follows Settings → Git and Integrations.",
    );
    ui.add_space(theme::LG);
}

/// Why git's configuration could not be read, in git's words, when it could not.
fn facts_error(ui: &mut Ui, facts: Option<&GitFacts>) {
    if let Some(e) = facts
        .and_then(|f| f.error.as_deref())
        .filter(|_| !Filter::searching(ui.ctx()))
    {
        Note {
            text: format!("Could not read your git configuration: {e}"),
            warn: true,
        }
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
    let help_text = |ui: &mut Ui| {
        help_label(ui, help);
        if let Some(note) = note {
            note.show(ui);
        }
    };
    row_layout(ui, title, None, help, &help_text, "", control);
    row_end(ui);
}

fn on_off() -> [(bool, String); 2] {
    [(true, "On".to_owned()), (false, "Off".to_owned())]
}

/// A repository's choice: a select whose first option follows the default (and names it), and,
/// once the repository has its own value, an "Overridden" tag and a button back to the default.
/// Drawn right to left (the select rightmost).
fn override_select<T: PartialEq + Copy>(
    ui: &mut Ui,
    salt: &str,
    name: &str,
    value: &mut Option<T>,
    default: T,
    options: &[(T, String)],
) {
    let label = |v: T| {
        options
            .iter()
            .find(|(o, _)| *o == v)
            .map_or_else(String::new, |(_, l)| l.clone())
    };
    let items: Vec<(Option<T>, String)> =
        std::iter::once((None, format!("Default · {}", label(default))))
            .chain(options.iter().map(|(v, l)| (Some(*v), l.clone())))
            .collect();
    let refs: Vec<(Option<T>, &str)> = items.iter().map(|(v, l)| (*v, l.as_str())).collect();
    Select::new(value, &refs)
        .width(select_width(ui, refs.iter().map(|(_, l)| *l)))
        .name(name)
        .id_salt(salt)
        .show(ui);
    if value.is_some() {
        if IconButton::new(
            icon::ARROW_COUNTER_CLOCKWISE,
            format!("Use the default ({})", label(default)),
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

/// Whether git signs this repository's commits, and where that is set: shown, not changed
/// (overriding signing is the owner's call, docs/backlog.md "Git settings").
fn signing_row(ui: &mut Ui, facts: &GitFacts) {
    let with = facts
        .gpg_format
        .as_ref()
        .filter(|_| facts.signs_commits())
        .map_or_else(String::new, |f| format!(" with {} keys", f.value));
    let help = match &facts.gpg_sign {
        Some(v) => format!(
            "Git {}{with} here: {}. Canopy commits with git, so this applies; change it in your git configuration.",
            if facts.signs_commits() { "signs commits" } else { "does not sign commits" },
            config_phrase(v)
        ),
        None => "Git does not sign commits here (commit.gpgsign is not set). Canopy commits with git, so setting it in your git configuration applies.".to_owned(),
    };
    row(ui, "Commit signing", &help, |ui| {
        let on = facts.signs_commits();
        Badge::new(if on { "On" } else { "Off" })
            .tone(if on { Tone::Success } else { Tone::Neutral })
            .show(ui);
    });
}

/// Context lines and whitespace for this repository (the same options as its diff header).
fn repo_diff_rows(ui: &mut Ui, s: &Settings, path: &Path, actions: &mut Vec<Action>) {
    let own = s.repo_diff.get(path).copied();
    let current = own.unwrap_or(s.diff);
    let mut context = own.map(|d| d.context).filter(|c| *c != s.diff.context);
    let mut whitespace = own
        .map(|d| d.whitespace)
        .filter(|w| *w != s.diff.whitespace);
    repo_row(
        ui,
        "Context lines",
        "Unchanged lines shown around each change. The diff header changes it too.",
        None,
        |ui| {
            let counts = [0, 1, 2, 3, 5, 8, 10, 15, 25].map(|n| (n, format!("{n} lines")));
            override_select(
                ui,
                "repo-context",
                "Context lines",
                &mut context,
                s.diff.context,
                &counts,
            );
        },
    );
    repo_row(
        ui,
        "Whitespace",
        "Ignoring whitespace turns off hunk staging, since such a diff cannot be applied.",
        None,
        |ui| {
            override_select(
                ui,
                "repo-whitespace",
                "Whitespace",
                &mut whitespace,
                s.diff.whitespace,
                &WhitespaceMode::ALL.map(|w| (w, w.label().to_owned())),
            );
        },
    );
    let chosen = DiffPrefs {
        context: context.unwrap_or(s.diff.context),
        whitespace: whitespace.unwrap_or(s.diff.whitespace),
    };
    if chosen != current {
        actions.push(Action::SetRepoDiff {
            path: path.to_path_buf(),
            prefs: (chosen != s.diff).then_some(chosen),
        });
    }
}

/// The repository's hosting account: its GitHub or GitLab host and whether Canopy has a token
/// for it (tokens belong to hosts, so they are managed under Accounts).
fn hosting_account(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    s: &Settings,
    tokens: &TokenStore,
    facts: Option<&GitFacts>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let hosts = s.gitlab_host_list();
    let forge = facts.and_then(|f| {
        f.remotes
            .iter()
            .find_map(|r| Forge::parse(&r.url, &hosts).map(|forge| (r.name.clone(), forge)))
    });
    let Some((remote, forge)) = forge else {
        let text = if facts.is_some() {
            "No remote of this repository is on GitHub or GitLab, so there are no pull or merge requests to load."
        } else {
            "Reading the repository's remotes…"
        };
        help_label(ui, text);
        row_end(ui);
        return;
    };
    let host = forge.authority();
    let saved = tokens.get_for(&forge).is_some();
    let env = canopy_hosting::token_from_env_for(forge.kind, &forge.host, &hosts).is_some();
    row_layout(
        ui,
        "Pull and merge requests",
        None,
        "",
        &|ui: &mut Ui| {
            help_label(
                ui,
                &format!(
                    "{} {} on {host} (remote {remote}).",
                    forge.kind.name(),
                    forge.kind.request_noun(),
                ),
            );
            let (text, color) = if !s.hosting_api {
                (
                    "Turned off in Integrations: Canopy does not contact the host.".to_owned(),
                    p.text_faint,
                )
            } else if saved {
                (
                    format!("{}  Token saved for {host}", icon::CHECK_CIRCLE),
                    p.success,
                )
            } else if env {
                (
                    format!(
                        "{}  Using the token from the environment",
                        icon::CHECK_CIRCLE
                    ),
                    p.success,
                )
            } else {
                (
                    format!(
                        "{}  No token saved: the git credential helper is tried",
                        icon::KEY
                    ),
                    p.text_faint,
                )
            };
            ui.label(RichText::new(text).size(m.meta_size).color(color));
        },
        "",
        |ui| {
            if Button::new("Open Accounts")
                .icon(forge_icon(forge.kind))
                .id_salt("repo-open-accounts")
                .show(ui)
                .clicked()
            {
                screen.section = Section::Accounts;
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
    let current = |pick: fn(&Identity) -> &Field| {
        move |ui: &mut Ui| {
            if let Some(field) = identity.map(pick) {
                in_use(ui, field);
            }
        }
    };
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
    row_layout(
        ui,
        "Name",
        name_tag,
        "identity user.name author",
        &current(|i| &i.author.name),
        "",
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
    row_layout(
        ui,
        "Email",
        email_tag,
        "identity user.email author",
        &current(|i| &i.author.email),
        "",
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
        "Saves the name and email to this repository's .git/config, where they win over your global identity for future commits here.",
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
) {
    heading(ui, "Cloning");
    row(
        ui,
        "Default clone folder",
        "Where the clone dialog puts new repositories. Empty uses ~/src when it exists, else your home folder.",
        |ui| {
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
            let default = s.clone_parent();
            let width = (FIELD_WIDTH - button).max(80.0);
            fixed_width(ui, width, |ui| {
                TextField::new(&mut s.clone_dir)
                    .placeholder(default)
                    .width(width)
                    .name("Default clone folder")
                    .id_salt("clone-dir")
                    .show(ui)
            });
            if s.clone_dir.trim().is_empty() {
                Badge::new("Default")
                    .tooltip("Canopy's default: ~/src when it exists, else your home folder.")
                    .show(ui);
            }
        },
    );
    default_row(
        ui,
        "Clone protocol",
        "For URLs built from owner/repository when you clone, add a remote or publish. SSH needs a key added to your account.",
        None,
        |ui| {
            Segmented::new(
                &mut s.clone_protocol,
                &[(CloneProtocol::Https, "HTTPS"), (CloneProtocol::Ssh, "SSH")],
            )
            .label("Clone protocol")
            .show(ui);
            revert(
                ui,
                &mut s.clone_protocol,
                &DEFAULTS.clone_protocol,
                "Clone protocol",
            );
        },
    );
    heading(ui, "GitHub and GitLab");
    row(
        ui,
        "Self-hosted GitLab",
        "Hosts to treat as GitLab, comma-separated. Hosts whose name starts with \"gitlab\" are recognised anyway.",
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
    row(
        ui,
        "Pull and merge requests",
        "Load, review and act on pull/merge requests through the GitHub and GitLab APIs. Requests load when you open a repository hosted there. Off keeps Canopy offline apart from git itself.",
        |ui| {
            switch(ui, &mut s.hosting_api, "Pull and merge requests");
            revert(
                ui,
                &mut s.hosting_api,
                &DEFAULTS.hosting_api,
                "Pull and merge requests",
            );
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
            command_help: "Runs in the repository folder; %d stands for the folder.",
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
            command_help: "%f stands for the file; without it the file is added at the end. Quote a path with spaces.",
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
            command_help: "%d stands for the folder; without it the folder is added at the end.",
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
    Custom,
}

/// What a saved command shows as: the system's when empty, an installed app whose command it
/// is, else a custom command (also while the user has the command field open).
fn launch_choice(
    value: &str,
    editing: bool,
    installed: &[&crate::system_apps::App],
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
            .map_or(LaunchChoice::Custom, LaunchChoice::App)
    }
}

/// An editor, terminal or file manager: the system's (named, with where that comes from), one of
/// the installed ones, or a command of the user's in a field that opens below.
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
    let system_label = match (apps, system) {
        (None, _) => "Default (looking…)".to_owned(),
        (Some(_), Some(choice)) => format!("Default · {}", choice.name),
        (Some(_), None) => "Default".to_owned(),
    };
    let current = launch_choice(value, custom.contains(&kind), &installed);
    let mut labels = vec![(LaunchChoice::System, system_label)];
    labels.extend(
        installed
            .iter()
            .enumerate()
            .map(|(i, a)| (LaunchChoice::App(i), a.name.clone())),
    );
    labels.push((LaunchChoice::Custom, "Custom command…".to_owned()));
    let options: Vec<(LaunchChoice, &str)> = labels.iter().map(|(c, l)| (*c, l.as_str())).collect();
    let mut chosen = current;
    // Where the system's choice comes from, under the help (the control column is narrow).
    let origin = (current == LaunchChoice::System)
        .then_some(system)
        .flatten();
    let help_text = |ui: &mut Ui| {
        help_label(ui, help);
        if let Some(choice) = origin {
            let tip = choice.command.as_ref().map_or_else(
                || "Canopy asks the system to open it.".to_owned(),
                |cmd| format!("Runs: {cmd}"),
            );
            Badge::new(choice.origin.tag()).tooltip(tip).show(ui);
        }
    };
    row_layout(ui, title, None, help, &help_text, "", |ui| {
        Select::new(&mut chosen, &options)
            .width(FIELD_WIDTH)
            .name(title)
            .id_salt(("launcher", title))
            .show(ui);
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

/// Returns true when a token was saved or removed.
fn accounts(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    settings: &Settings,
    tokens: &mut TokenStore,
    forges: &[Forge],
    actions: &mut Vec<Action>,
) -> bool {
    const ABOUT: &str = "Canopy uses a personal access token to list and act on pull/merge requests. Tokens are sent only to the host they belong to. Without one, Canopy falls back to GH_TOKEN / GITLAB_TOKEN and then to your git credential helper.";
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let mut changed = false;
    let hosts_text = std::iter::once("github.com gitlab.com".to_owned())
        .chain(settings.gitlab_host_list())
        .chain(forges.iter().map(|f| f.host.clone()))
        .collect::<Vec<_>>()
        .join(" ");
    // The page is one block: a search for any host or for tokens shows all of it.
    if !block(ui, "Tokens", &format!("{ABOUT} {hosts_text}")) {
        return false;
    }
    help_label(ui, ABOUT);
    if !settings.hosting_api {
        ui.add_space(theme::SM);
        Callout::new(
            Tone::Warning,
            "Pull and merge requests are turned off in Integrations.",
        )
        .icon(icon::INFO)
        .show(ui);
    }
    ui.add_space(theme::MD);
    // Hosts: the two public ones, configured GitLab hosts and those of open repositories. Tokens
    // are kept per authority, so a host on a non-default port gets its own entry.
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
    for h in settings.gitlab_host_list() {
        hosts.push(account(ForgeKind::GitLab, &h));
    }
    for f in forges {
        hosts.push(Forge {
            path: String::new(),
            ..f.clone()
        });
    }
    let mut seen = HashSet::new();
    hosts.retain(|f| seen.insert(f.authority()));

    // Collect finished token tests.
    for test in screen.tests.values_mut() {
        if let TokenTest::Running(rx) = test
            && let Ok(r) = rx.try_recv()
        {
            *test = TokenTest::Done(r);
        }
    }

    let width = ui.available_width() - theme::SM;
    for forge in hosts {
        let kind = forge.kind;
        let host = forge.authority();
        Card::new()
            .width(width)
            .id_salt(("token-card", &host))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = theme::SM;
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(forge_icon(kind)).size(m.icon_lg).color(p.text));
                    ui.label(RichText::new(&host).font(FontId::new(m.subtitle_size, theme::semibold())).color(p.text));
                    let saved = tokens.get_for(&forge).is_some();
                    let env = canopy_hosting::token_from_env_for(kind, &forge.host, &settings.gitlab_host_list()).is_some();
                    let (status, color) = if saved {
                        (format!("{}  Token saved", icon::CHECK_CIRCLE), p.success)
                    } else if env {
                        (format!("{}  Using the {} environment variable", icon::CHECK_CIRCLE, if kind == ForgeKind::GitHub { "GH_TOKEN" } else { "GITLAB_TOKEN" }), p.success)
                    } else {
                        (format!("{}  No saved token (the git credential helper is tried)", icon::KEY), p.text_faint)
                    };
                    ui.label(RichText::new(status).size(m.meta_size).color(color));
                });
                let input = screen.token_input.entry(host.clone()).or_default();
                ui.horizontal_wrapped(|ui| {
                    TextField::new(input)
                        .password(true)
                        .placeholder("Paste a personal access token…")
                        .width(FIELD_WIDTH)
                        .name(format!("Token for {host}"))
                        .id_salt(("token", &host))
                        .show(ui);
                    let has_input = !input.trim().is_empty();
                    if Button::new("Save")
                        .icon(icon::FLOPPY_DISK)
                        .enabled(has_input)
                        .disabled_reason("Paste a token first.")
                        .id_salt(("token-save", &host))
                        .show(ui)
                        .clicked()
                    {
                        match tokens.set(&host, input) {
                            Ok(()) => {
                                changed = true;
                                screen.token_error = None;
                                let rx = hosting::test_token(forge.clone(), input.trim().to_owned(), ui.ctx().clone());
                                screen.tests.insert(host.clone(), TokenTest::Running(rx));
                                input.clear();
                            }
                            Err(e) => screen.token_error = Some(format!("Could not save the token: {e}")),
                        }
                    }
                    if tokens.get_for(&forge).is_some() {
                        if Button::new("Test")
                            .icon(icon::PLUGS)
                            .id_salt(("token-test", &host))
                            .show(ui)
                            .clicked()
                            && let Some(t) = tokens.get_for(&forge)
                        {
                            let rx = hosting::test_token(forge.clone(), t.to_owned(), ui.ctx().clone());
                            screen.tests.insert(host.clone(), TokenTest::Running(rx));
                        }
                        if Button::new("Remove")
                            .icon(icon::TRASH)
                            .icon_tone(Tone::Danger)
                            .id_salt(("token-remove", &host))
                            .show(ui)
                            .clicked()
                        {
                            match tokens.set(&host, "") {
                                Ok(()) => {
                                    changed = true;
                                    screen.tests.remove(&host);
                                }
                                Err(e) => screen.token_error = Some(format!("Could not remove the token: {e}")),
                            }
                        }
                    }
                });
                match screen.tests.get(&host) {
                    Some(TokenTest::Running(_)) => {
                        ui.horizontal(|ui| {
                            Spinner::new()
                                .size(Size::Sm)
                                .label("the token check")
                                .show(ui);
                            ui.label(RichText::new("Checking the token…").size(m.meta_size).color(p.text_dim));
                        });
                    }
                    Some(TokenTest::Done(Ok(user))) => {
                        ui.label(RichText::new(format!("{}  Signed in as {user}", icon::CHECK)).size(m.meta_size).color(p.success));
                    }
                    Some(TokenTest::Done(Err(e))) => {
                        ui.label(RichText::new(format!("{}  {e}", icon::WARNING)).size(m.meta_size).color(p.danger));
                    }
                    None => {}
                }
                let scopes = match kind {
                    ForgeKind::GitHub => "Needs the repo scope (classic token), or Pull requests: read and write plus Contents: read (fine-grained).",
                    ForgeKind::GitLab => "Needs the api scope (read_api is enough to only browse).",
                };
                ui.add(egui::Label::new(RichText::new(scopes).size(m.small_size).color(p.text_faint)).wrap());
                if LinkButton::new("Create a token")
                    .external(true)
                    .text_size(m.small_size)
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::OpenUrl(hosting::token_page(kind, &host)));
                }
            });
        ui.add_space(m.card_gap);
    }
    if let Some(e) = &screen.token_error {
        Callout::new(Tone::Danger, e.as_str()).show(ui);
    }
    changed
}

/// The status line under "Download profile pictures": why the open repository's authors show
/// initials, and the way to Accounts when a token would fix it.
fn picture_note(ui: &mut Ui, note: &PictureNote, open_accounts: &std::cell::Cell<bool>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.add_space(theme::XS);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = theme::XS;
        ui.label(
            RichText::new(icon::INFO)
                .size(m.meta_size)
                .color(p.text_dim),
        );
        ui.label(
            RichText::new(&note.title)
                .size(m.meta_size)
                .color(p.text_dim),
        );
    });
    ui.add(
        egui::Label::new(
            RichText::new(&note.detail)
                .size(m.meta_size)
                .color(p.text_faint),
        )
        .wrap(),
    );
    if note.open_accounts
        && LinkButton::new("Open Accounts")
            .text_size(m.meta_size)
            .show(ui)
            .clicked()
    {
        open_accounts.set(true);
    }
}

/// What the user agreed to: the Terms and the Privacy Policy accepted, and the crash report
/// answer, with the way to change it. Reading a document here accepts nothing.
fn consent(ui: &mut Ui, s: &Settings, actions: &mut Vec<Action>) {
    use crate::ui::legal::{Document, PRIVACY_VERSION, TERMS_VERSION};
    let p = palette(ui.ctx());
    heading(ui, "Consent");
    let status = |accepted: &str, current: &str| {
        if accepted == current {
            format!("You accepted this version (last updated {current}).")
        } else if accepted.is_empty() {
            "Not accepted yet. Canopy asks before it contacts any server.".to_owned()
        } else {
            format!(
                "You accepted the version of {accepted}; it was updated on {current}. Canopy asks you to accept the new one when it starts."
            )
        }
    };
    let documents = [
        (
            Document::Terms,
            "Terms of Service",
            "The agreement for using Canopy.",
            status(&s.accepted_terms, TERMS_VERSION),
            icon::SCROLL,
            "read-terms",
        ),
        (
            Document::Privacy,
            "Privacy Policy",
            "What Canopy collects and your rights.",
            status(&s.accepted_privacy, PRIVACY_VERSION),
            icon::SHIELD_CHECK,
            "read-privacy",
        ),
    ];
    for (document, title, about, state, glyph, salt) in documents {
        row(ui, title, &format!("{about} {state}"), |ui| {
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
        let answer = match s.crash_reports {
            None => "You have not answered yet, so nothing is sent.",
            Some(true) => "Your answer: send them.",
            Some(false) => "Your answer: do not send them.",
        };
        row_more(
            ui,
            "Send crash reports",
            &format!(
                "When Canopy crashes, send the error and a stack trace to NecturaLabs. {answer}"
            ),
            "Also sent: the Canopy version, your operating system and a short record of each session. Stored by Sentry in the EU and deleted after 90 days. Takes effect immediately.",
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

/// Returns true when "Open Accounts" was clicked.
fn privacy(
    ui: &mut Ui,
    s: &mut Settings,
    actions: &mut Vec<Action>,
    picker: &mut crate::ui::folder_picker::FolderPicker,
    pictures: Option<&PictureNote>,
) -> bool {
    consent(ui, s, actions);
    heading(ui, "Partial clones");
    row_more(
        ui,
        "Trusted folders",
        "Canopy downloads a partial clone's missing files on its own only inside these folders.",
        "In a partial clone, opening a commit or a diff downloads the files the clone left on the server. git then follows the repository's own configuration, which can run programs and contact servers, so Canopy does this on its own only for repositories inside these folders. Trust a folder only if everything in it comes from you or from people you trust. Elsewhere you see what is already on disk, with a Trust… button in the status bar.",
        |ui| {
            if let Some(dir) = picker.show_labelled(
                ui,
                "trusted-folder",
                &crate::settings::expand_home(&s.clone_parent()),
                "Add folder…",
            ) && crate::trust::add(&mut s.trusted_folders, &dir)
            {
                crate::trust::set(&s.trusted_folders);
                ui.ctx().request_repaint();
            }
        },
    );
    if last_row_shown(ui) {
        trusted_folder_list(ui, s);
    }
    row_end(ui);
    heading(ui, "Profile pictures");
    let help = "Fetch GitHub or GitLab account pictures for visible authors.";
    let open_accounts = std::cell::Cell::new(false);
    let help_text = |ui: &mut Ui| {
        help_label(ui, help);
        if let Some(note) = pictures {
            picture_note(ui, note, &open_accounts);
        }
    };
    row_layout(
        ui,
        "Download profile pictures",
        None,
        help,
        &help_text,
        "Sends commit IDs to GitHub or author emails to GitLab. Pictures, and authors without one, are remembered for 12 hours. A failed refresh keeps the previous picture. Turn off to use initials and stop new requests. Also requires Pull and merge requests to be on.",
        |ui| {
            switch(ui, &mut s.download_avatars, "Download profile pictures");
            revert(
                ui,
                &mut s.download_avatars,
                &DEFAULTS.download_avatars,
                "Download profile pictures",
            );
        },
    );
    row_end(ui);
    heading(ui, "Diagnostics");
    row_more(
        ui,
        "Log detail",
        "Normal records operations, warnings and errors. Detailed adds every git command and API call with its timing.",
        "Turn Detailed on while you reproduce a problem. The CANOPY_LOG environment variable overrides this.",
        |ui| {
            let before = s.log_level;
            let options = crate::logging::LogLevel::ALL.map(|l| (l, l.label()));
            Select::new(&mut s.log_level, &options)
                .width(SELECT_WIDTH)
                .name("Log detail")
                .id_salt("log-level")
                .show(ui);
            revert(ui, &mut s.log_level, &DEFAULTS.log_level, "Log detail");
            if s.log_level != before {
                crate::logging::set_level(s.log_level);
            }
        },
    );
    row_end(ui);
    row_more(
        ui,
        "Logs and crash reports",
        "Kept on this computer, one log per run, and never uploaded.",
        "Copy diagnostics puts your versions, this run's log file and its latest warnings on the clipboard, for a bug report.",
        |ui| {
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
        },
    );
    row_end(ui);
    open_accounts.get()
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
        crate::trust::set(&s.trusted_folders);
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
    row(
        ui,
        "Check for updates",
        "At most once a day, when Canopy starts, it reads a small public file on GitHub naming the latest build. GitHub sees your IP address and your Canopy build number, nothing else. Nothing is downloaded or installed until you choose Update.",
        |ui| {
            switch(ui, &mut s.check_updates, "Check for updates");
            revert(
                ui,
                &mut s.check_updates,
                &DEFAULTS.check_updates,
                "Check for updates",
            );
        },
    );
    row(
        ui,
        "Include beta builds",
        "Also offer beta builds, which are tried out less before they are published.",
        |ui| {
            switch(ui, &mut s.include_prereleases, "Include beta builds");
            revert(
                ui,
                &mut s.include_prereleases,
                &DEFAULTS.include_prereleases,
                "Include beta builds",
            );
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

/// How many rows the cheatsheet's list shows for `section` filtered by `query` (the same test
/// `ui::cheatsheet` applies: the description or one of the keys contains the query).
fn shortcut_matches(section: KeySection, query: &str) -> usize {
    crate::keymap::BINDINGS
        .iter()
        .filter(|b| b.section == section && crate::keymap::listed_here(b))
        .filter(|b| {
            b.description.to_lowercase().contains(query)
                || crate::keymap::all_keys(b.command)
                    .iter()
                    .any(|k| k.to_lowercase().contains(query))
        })
        .count()
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
                *s = imported;
                actions.push(Action::SettingsReplaced);
                Some((false, format!("Imported from {}.", path.display())))
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
        Note {
            text: message.clone(),
            warn: *failed,
        }
        .show(ui);
        ui.add_space(theme::SM);
    }
    heading(ui, "Settings file");
    row(
        ui,
        "Export settings",
        "Saves your preferences to a file you choose, to keep or to use on another computer. Tokens, trusted folders and your answers to the Terms, the Privacy Policy and crash reports are never in it.",
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
    row(
        ui,
        "Import settings",
        "Takes the preferences from a settings file. Anything the file does not name stays as it is; a file that is not Canopy's changes nothing.",
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
        "Every preference back to its default, which turns pull and merge requests, profile pictures and update checks back on. Tokens, trusted folders, recent repositories and your answers to the Terms, the Privacy Policy and crash reports stay.",
        |ui| {
            if confirm_button(ui, "reset-all-settings", "Reset all", "Reset every setting") {
                match crate::settings_file::reset(s) {
                    Ok(fresh) => {
                        *s = fresh;
                        actions.push(Action::SettingsReplaced);
                        screen.file_status =
                            Some((false, "Every setting is back to its default.".to_owned()));
                    }
                    Err(e) => screen.file_status = Some((true, e)),
                }
            }
        },
    );
}

fn keyboard(ui: &mut Ui) {
    let words = Filter::get(ui.ctx()).words;
    if !words.is_empty() {
        // The shortcuts the cheatsheet's own filter finds for the whole query.
        let query = words.join(" ");
        for section in KeySection::ALL {
            let found = shortcut_matches(section, &query);
            if found > 0 {
                reveal_rows(ui, found);
                crate::ui::cheatsheet::rows(ui, section, &query);
            }
        }
        return;
    }
    ui.scope(|ui| {
        // Clear of the scrollbar, which floats over the list's right edge.
        ui.set_max_width(ui.available_width() - theme::MD);
        help_label(
            ui,
            &format!(
                "Press ? (or {}) anywhere for this list as an overlay. Single keys work when no text field has focus; Super is never used.",
                crate::keymap::hint(Command::Cheatsheet)
                    .split(" · ")
                    .next()
                    .unwrap_or_default()
            ),
        );
    });
    ui.add_space(theme::MD);
    for section in KeySection::ALL {
        crate::ui::cheatsheet::rows(ui, section, "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn settings_fit_a_compact_zoomed_window() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, theme::Appearance::STILL);
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(562.5, 350.0));
        let mut settings = Settings::default();
        let mut tokens = TokenStore::default();
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
                            &mut tokens,
                            &SettingsData {
                                forges: &[],
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
        let mut tokens = TokenStore::default();
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
                        &mut tokens,
                        &SettingsData {
                            forges: &[],
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

    fn fact(key: &str, value: &str) -> ConfigValue {
        ConfigValue {
            key: key.to_owned(),
            value: value.to_owned(),
            scope: "global".to_owned(),
            origin: "file:/home/me/.gitconfig".to_owned(),
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
        let nothing = pull_note(&facts, follow).unwrap();
        assert!(!nothing.warn);
        assert!(
            nothing.text.contains("sets no pull strategy"),
            "{nothing:?}"
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
            "From your git configuration: pull.rebase=true (your global git config), so pulls \
             use rebase."
        );
        assert!(!pull_note(&facts, own(PullMode::Rebase)).unwrap().warn);
        let overridden = pull_note(&facts, own(PullMode::Merge)).unwrap();
        assert!(overridden.warn);
        assert_eq!(
            overridden.text,
            "Canopy passes --no-rebase --ff, so your git configuration's pull.rebase=true (your \
             global git config) does not apply to pulls from Canopy."
        );
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
            title: "No token for github.com".to_owned(),
            detail: "Add a token under Accounts to load authors' pictures.".to_owned(),
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
        };
        let (zed, code) = (app("Zed", "zed"), app("Code", "code %f"));
        let installed = [&zed, &code];
        assert_eq!(launch_choice("  ", false, &installed), LaunchChoice::System);
        assert_eq!(
            launch_choice("code %f", false, &installed),
            LaunchChoice::App(1)
        );
        assert_eq!(
            launch_choice(" zed ", false, &installed),
            LaunchChoice::App(0)
        );
        assert_eq!(
            launch_choice("zed --wait", false, &installed),
            LaunchChoice::Custom
        );
        assert_eq!(
            launch_choice("zed", true, &installed),
            LaunchChoice::Custom,
            "an open command field stays open"
        );
    }

    /// Canopy passes its pruning choice, so a configuration that differs is reported as not
    /// applying, either way round.
    #[test]
    fn notes_say_when_the_prune_choice_overrides_git() {
        let mut facts = GitFacts::default();
        assert_eq!(prune_note(&facts, false), None);
        facts.fetch_prune = Some(fact("fetch.prune", "true"));
        let off = prune_note(&facts, false).unwrap();
        assert!(off.warn);
        assert!(
            off.text.starts_with("Canopy passes --no-prune"),
            "{}",
            off.text
        );
        assert!(!prune_note(&facts, true).unwrap().warn);
        facts.fetch_prune = Some(fact("fetch.prune", "false"));
        assert!(!prune_note(&facts, false).unwrap().warn);
        let on = prune_note(&facts, true).unwrap();
        assert!(on.warn);
        assert!(on.text.starts_with("Canopy passes --prune"), "{}", on.text);
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
        let paper = found("paper", &mut settings);
        assert_eq!(paper.count(Section::Appearance), 1, "{paper:?}");
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
        let mut tokens = TokenStore::default();
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
                            &mut tokens,
                            &SettingsData {
                                forges: &[],
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
        assert_eq!(label(&local_source()), "this repository: .git/config");
        assert_eq!(
            label(&Source::Config {
                scope: "global".into(),
                origin: "file:/home/ada/.gitconfig".into(),
            }),
            "global: ~/.gitconfig"
        );
        assert_eq!(
            label(&Source::Config {
                scope: "global".into(),
                origin: "file:/etc/elsewhere/gitconfig".into(),
            }),
            "global: /etc/elsewhere/gitconfig"
        );
        assert_eq!(label(&Source::Guessed), "guessed by Git from this computer");
        assert_eq!(source_label_in(None, home), "not set");
    }

    #[test]
    fn a_long_path_is_cut_in_the_middle_to_the_width() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let path =
            "From global: /run/media/nectura/Programming SSD/tmp/canopy-polish/tmpdir/gitconfig";
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
            scope: "global".into(),
            origin: "file:/etc/gitconfig".into(),
        };
        assert_eq!(
            source_label_in(Some(&source), Path::new("")),
            "global: /etc/gitconfig"
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
}
