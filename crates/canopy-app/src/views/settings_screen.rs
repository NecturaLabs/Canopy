//! The settings screen (Ctrl+,): sections on the left, live-applied controls on the right.
//! Every setting changes behaviour; nothing here is decorative.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
use crate::settings::{CloneProtocol, PullMode, Settings, WhitespaceMode};
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::Badge;
use crate::ui::kit::choice::Segmented;
use crate::ui::kit::field::{SearchField, TextField};
use crate::ui::kit::overlay;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::paint;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::range::Slider;
use crate::ui::kit::row::{ListRow, RowKind};
use crate::ui::kit::select::Select;
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
}

impl Section {
    const ALL: [Self; 7] = [
        Self::Appearance,
        Self::Git,
        Self::Integrations,
        Self::Accounts,
        Self::Updates,
        Self::Privacy,
        Self::Keyboard,
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
        }
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
    /// The screen opened at `section`.
    pub fn at(section: Section) -> Self {
        Self {
            section,
            ..Self::default()
        }
    }

    /// The screen showing what `query` finds (screenshots and tests).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "for the settings-search screenshot overlay the integrator adds (contracts/settings.md)"
        )
    )]
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
    let (width, height) = shell_size(ctx);
    let sidebar = width >= NAV_MIN_WIDTH;
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
                navigation(ui, nav, r, screen);
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
            result.close |= header_bar(ui, header, corner, screen, sidebar);
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
            Section::Integrations => integrations(ui, settings, &mut screen.picker),
            Section::Accounts => {
                *tokens_changed |= accounts(ui, screen, settings, tokens, data.forges, actions);
            }
            Section::Updates => updates(ui, settings, data.update, actions),
            Section::Privacy => privacy(ui, settings, actions, &mut screen.picker),
            Section::Keyboard => keyboard(ui),
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
        let mut counts = Vec::with_capacity(Section::ALL.len());
        let jump = self.screen.search.jump.take();
        for section in Section::ALL {
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
        "Terminal" => "console shell command line",
        "Editor" => "ide vscode code zed sublime open file",
        "Trusted folders" => "trust partial clone security",
        "Send crash reports" => "crash sentry telemetry error report",
        "Log detail" => "log logging debug verbose diagnostics",
        "Logs and crash reports" => "log logs diagnostics folder copy",
        "Privacy Policy" | "Terms of Service" => "legal privacy terms document",
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
fn navigation(ui: &mut Ui, rect: Rect, radius: u8, screen: &mut SettingsScreen) {
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
            for section in Section::ALL {
                let (glyph, label) = section.label();
                let found = screen.search.count(section);
                let mut row = ListRow::new(("settings-section", section))
                    .kind(RowKind::Sidebar)
                    .icon(glyph)
                    .label(label)
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
        let options = Section::ALL.map(|s| (s, s.label().1));
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
    row_layout(ui, title, help, &help_text, detail, control);
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
        ui.label(marked(
            title,
            &words,
            &FontId::new(m.body_size, theme::semibold()),
            p.text,
            p.accent,
            p.accent_tint(),
        ));
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

/// Text typed into a number field that has not been applied yet, and the error it showed.
#[derive(Debug, Clone, Default)]
struct NumberDraft {
    text: String,
    error: Option<String>,
}

/// Where a [`number_field`] keeps its draft, what it accepts, how wide it is and its name.
struct NumberField<'a> {
    salt: &'a str,
    range: RangeInclusive<f64>,
    width: f32,
    name: &'a str,
}

/// A number typed into a field: what is typed applies on Enter or when the field loses focus
/// (clamped to `range`), Esc restores the value, and text that is not a number shows an error.
/// A click selects the whole value, so typing replaces it. Returns the new value once applied.
fn number_field(
    ui: &mut Ui,
    field: NumberField<'_>,
    value: f64,
    shown: impl Fn(f64) -> String,
    parse: impl Fn(&str) -> Option<f64>,
) -> Option<f64> {
    let NumberField {
        salt,
        range,
        width,
        name,
    } = field;
    let key = ui.id().with(("settings-number", salt));
    let draft: Option<NumberDraft> = ui.data(|d| d.get_temp(key));
    let mut text = draft
        .as_ref()
        .map_or_else(|| shown(value), |d| d.text.clone());
    let error = draft.as_ref().and_then(|d| d.error.clone());
    let response = fixed_width(ui, width, |ui| {
        TextField::new(&mut text)
            .width(width)
            .name(name)
            .error(error.as_deref())
            .id_salt(("settings-number", salt))
            .show(ui)
    });
    if response.gained_focus() || response.clicked() {
        let ctx = ui.ctx();
        if let Some(mut state) = egui::text_edit::TextEditState::load(ctx, response.id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ctx, response.id);
        }
    }
    let mut applied = None;
    let mut next = None;
    if response.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            next = None;
        } else {
            match parse(text.trim()) {
                Some(v) if v.is_finite() => {
                    applied = Some(v.clamp(*range.start(), *range.end()));
                }
                _ => {
                    next = Some(NumberDraft {
                        text: text.clone(),
                        error: Some(format!(
                            "Enter a number from {} to {}.",
                            shown(*range.start()),
                            shown(*range.end())
                        )),
                    });
                }
            }
        }
    } else if response.has_focus() {
        next = Some(NumberDraft {
            text: text.clone(),
            // The error stays until the text is applied again.
            error,
        });
    } else {
        next = draft;
    }
    ui.data_mut(|d| match next {
        Some(n) => {
            d.insert_temp(key, n);
        }
        None => d.remove::<NumberDraft>(key),
    });
    applied
}

/// The fewest and most commits a history page loads, and the stepper's step.
const COMMIT_PAGE: RangeInclusive<f64> = 200.0..=50_000.0;
const COMMIT_PAGE_STEP: usize = 100;

fn appearance(ui: &mut Ui, s: &mut Settings, preview: &mut Option<f32>) {
    heading(ui, "Theme");
    if block(ui, "Theme", "") {
        theme_cards(ui, s);
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
        },
    );
    row(
        ui,
        "Author avatars",
        "Pictures or coloured initials next to authors.",
        |ui| {
            switch(ui, &mut s.show_avatars, "Author avatars");
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
            let typed = number_field(
                ui,
                NumberField {
                    salt: "zoom",
                    range: f64::from(*ZOOM.start())..=f64::from(*ZOOM.end()),
                    width: ZOOM_FIELD_WIDTH,
                    name: "Zoom percentage",
                },
                f64::from(draft.unwrap_or(s.zoom)),
                percent,
                |text| {
                    text.trim_end_matches('%')
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .map(|v| v / 100.0)
                },
            );
            if let Some(v) = typed {
                s.zoom = crate::ui::to_f32(v);
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
/// The width of the commits-per-page field between its steppers.
const COUNT_FIELD_WIDTH: f32 = 96.0;

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
    row_layout(ui, "Name", "", &current(|i| &i.author.name), "", |ui| {
        fixed_width(ui, FIELD_WIDTH, |ui| {
            TextField::new(&mut screen.identity_name)
                .width(FIELD_WIDTH)
                .name("Name")
                .id_salt("identity-name")
                .show(ui)
        });
    });
    row_end(ui);
    row_layout(ui, "Email", "", &current(|i| &i.author.email), "", |ui| {
        fixed_width(ui, FIELD_WIDTH, |ui| {
            TextField::new(&mut screen.identity_email)
                .width(FIELD_WIDTH)
                .name("Email")
                .id_salt("identity-email")
                .show(ui)
        });
    });
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
    heading(ui, "Remote");
    row(
        ui,
        "Pull strategy",
        "Used when git's pull.rebase is not configured for the repository.",
        |ui| {
            let options = PullMode::ALL.map(|m| (m, m.label()));
            Segmented::new(&mut s.pull_mode, &options)
                .label("Pull strategy")
                .show(ui);
        },
    );
    row(
        ui,
        "Prune on fetch",
        "Remove remote-tracking branches that were deleted on the remote.",
        |ui| {
            switch(ui, &mut s.prune_on_fetch, "Prune on fetch");
        },
    );
    row(
        ui,
        "Fetch in the background",
        "Fetches every remote of open repositories while Canopy is idle.",
        |ui| {
            let labels = [0, 5, 10, 15, 30].map(|m| {
                (
                    m,
                    if m == 0 {
                        "Off".to_owned()
                    } else {
                        format!("Every {m} minutes")
                    },
                )
            });
            let options: Vec<(u32, &str)> = labels.iter().map(|(m, l)| (*m, l.as_str())).collect();
            let other = format!("Every {} minutes", s.auto_fetch_minutes);
            Select::new(&mut s.auto_fetch_minutes, &options)
                .placeholder(other)
                .width(SELECT_WIDTH)
                .name("Fetch in the background")
                .id_salt("auto-fetch")
                .show(ui);
        },
    );
    heading(ui, "Commits");
    row(
        ui,
        "Add Signed-off-by",
        "Appends a Signed-off-by trailer (git commit --signoff), as DCO projects require.",
        |ui| {
            switch(ui, &mut s.signoff, "Add Signed-off-by");
        },
    );
    row(
        ui,
        "Commits per page",
        "How many commits the history loads before \"Load more\".",
        |ui| {
            commit_page(ui, s);
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
        },
    );
    row(
        ui,
        "Syntax highlighting",
        "Colours code in diffs by language (Rust, TypeScript, Python, Markdown and about 200 more).",
        |ui| {
            switch(ui, &mut s.syntax_highlight, "Syntax highlighting");
        },
    );
    heading(ui, "Diff defaults (every repository)");
    row(
        ui,
        "Context lines",
        "Unchanged lines shown around each change. Each repository can override this from the diff header.",
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
        },
    );
    row(
        ui,
        "Whitespace",
        "Ignoring whitespace turns off hunk staging, since such a diff cannot be applied.",
        |ui| {
            let options = WhitespaceMode::ALL.map(|m| (m, m.label()));
            Select::new(&mut s.diff.whitespace, &options)
                .width(SELECT_WIDTH)
                .name("Whitespace")
                .id_salt("whitespace")
                .show(ui);
        },
    );
    if !s.repo_diff.is_empty() {
        row(
            ui,
            "Repository overrides",
            &format!(
                "{} repositories use their own diff options.",
                s.repo_diff.len()
            ),
            |ui| {
                if Button::new("Reset all").show(ui).clicked() {
                    s.repo_diff.clear();
                }
            },
        );
    }
}

/// Commits per page: a number field between − and + steppers.
fn commit_page(ui: &mut Ui, s: &mut Settings) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the range's ends are the whole numbers 200 and 50 000"
    )]
    let (lo, hi) = (*COMMIT_PAGE.start() as usize, *COMMIT_PAGE.end() as usize);
    // The steppers stand a gap away from the field and as tall as it, like the other rows'
    // controls, rather than pressed against it.
    ui.spacing_mut().item_spacing.x = theme::SM;
    // Right to left: +, the field, −.
    if IconButton::new(icon::PLUS, "More commits per page")
        .size(Size::Md)
        .enabled(s.commit_page < hi)
        .id_salt("commit-page-more")
        .show(ui)
        .clicked()
    {
        s.commit_page = (s.commit_page + COMMIT_PAGE_STEP).min(hi);
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a page size of at most 50 000, exact in f64"
    )]
    let current = s.commit_page as f64;
    if let Some(v) = number_field(
        ui,
        NumberField {
            salt: "commit-page",
            range: COMMIT_PAGE,
            width: COUNT_FIELD_WIDTH,
            name: "Commits per page",
        },
        current,
        |v| format!("{v:.0}"),
        |text| text.replace([',', '_', ' '], "").parse::<f64>().ok(),
    ) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "number_field clamps the value to COMMIT_PAGE, so it is a whole positive count once rounded"
        )]
        let page = v.round() as usize;
        s.commit_page = page;
    }
    if IconButton::new(icon::MINUS, "Fewer commits per page")
        .size(Size::Md)
        .enabled(s.commit_page > lo)
        .id_salt("commit-page-fewer")
        .show(ui)
        .clicked()
    {
        s.commit_page = s.commit_page.saturating_sub(COMMIT_PAGE_STEP).max(lo);
    }
}

fn integrations(
    ui: &mut Ui,
    s: &mut Settings,
    picker: &mut crate::ui::folder_picker::FolderPicker,
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
        },
    );
    row(
        ui,
        "Clone protocol",
        "For URLs built from owner/repository. SSH needs a key added to your account.",
        |ui| {
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
        },
    );
    heading(ui, "Tools");
    row(
        ui,
        "Terminal",
        "The command \"Open terminal here\" runs, in the repository folder. Empty tries $TERMINAL and common terminals.",
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut s.terminal)
                    .placeholder("kitty, alacritty, wezterm start --cwd .")
                    .width(FIELD_WIDTH)
                    .name("Terminal")
                    .id_salt("terminal")
                    .show(ui)
            });
        },
    );
    row(
        ui,
        "Editor",
        "The command \"Open in editor\" runs, with the file added at the end. Empty uses your system's text editor.",
        |ui| {
            fixed_width(ui, FIELD_WIDTH, |ui| {
                TextField::new(&mut s.editor)
                    .placeholder("code, zed, gnome-text-editor")
                    .width(FIELD_WIDTH)
                    .name("Editor")
                    .id_salt("editor")
                    .show(ui)
            });
        },
    );
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
    let mut seen = std::collections::HashSet::new();
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

fn privacy(
    ui: &mut Ui,
    s: &mut Settings,
    actions: &mut Vec<Action>,
    picker: &mut crate::ui::folder_picker::FolderPicker,
) {
    use crate::ui::legal::Document;
    let p = palette(ui.ctx());
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
    row_more(
        ui,
        "Download profile pictures",
        "Fetch GitHub or GitLab account pictures for visible authors.",
        "Sends commit IDs to GitHub or author emails to GitLab. Pictures and failed lookups are cached locally for 12 hours. A failed refresh keeps the previous picture. Turn off to use initials and stop new requests. Also requires Pull and merge requests to be on.",
        |ui| {
            switch(ui, &mut s.download_avatars, "Download profile pictures");
        },
    );
    row_end(ui);
    heading(ui, "Crash reports");
    if crate::crash_reports::available() {
        let before = s.crash_reports;
        let mut on = s.crash_reports == Some(true);
        row_more(
            ui,
            "Send crash reports",
            "When Canopy crashes, send the error and a stack trace to NecturaLabs.",
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
    heading(ui, "Documents");
    row(
        ui,
        "Privacy Policy",
        &format!(
            "What Canopy collects and your rights. Last updated {}.",
            crate::ui::legal::PRIVACY_VERSION
        ),
        |ui| {
            if Button::new("Read")
                .icon(icon::SHIELD_CHECK)
                .id_salt("read-privacy")
                .show(ui)
                .clicked()
            {
                actions.push(Action::OpenDialog(Dialog::Legal(Document::Privacy)));
            }
        },
    );
    row(
        ui,
        "Terms of Service",
        &format!(
            "The agreement for using Canopy. Last updated {}.",
            crate::ui::legal::TERMS_VERSION
        ),
        |ui| {
            if Button::new("Read")
                .icon(icon::SCROLL)
                .id_salt("read-terms")
                .show(ui)
                .clicked()
            {
                actions.push(Action::OpenDialog(Dialog::Legal(Document::Terms)));
            }
        },
    );
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
        },
    );
    row(
        ui,
        "Include beta builds",
        "Also offer beta builds, which are tried out less before they are published.",
        |ui| {
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
    fn draw_screen(mut screen: SettingsScreen, settings: &mut Settings) -> SettingsScreen {
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
        screen
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
