//! Pull requests (GitHub) and merge requests (GitLab): a list on the left, the selected request
//! on the right with its checks, reviews, description and conversation, and the actions a
//! reviewer needs (check out, approve, comment, merge, close or reopen).

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontFamily, FontId, Galley, Id, Layout, Margin, Painter,
    Rect, Response, RichText, Stroke, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::RefKind;
use canopy_core::forge::ForgeKind;
use canopy_hosting::{
    CheckState, MergeMethod, RequestDetails, RequestState, RequestSummary, StateFilter,
};

use crate::action::Action;
use crate::hosting::{HostJob, TokenSource};
use crate::keymap::Command;
use crate::tab::RepoTab;
use crate::ui::art;
use crate::ui::kit::avatar::{Avatar, AvatarSize, Person};
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::{Badge, RefPill};
use crate::ui::kit::choice::Segmented;
use crate::ui::kit::field::TextArea;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::{Skeleton, Spinner};
use crate::ui::kit::row::{Disclosure, ListRow, RowKind, SectionHeader};
use crate::ui::kit::select::Select;
use crate::ui::kit::surface::{Callout, EmptyAction, EmptyState, InsetBox, NoticeAction};
use crate::ui::kit::{self, Button, IconButton, Size, Tone, paint};
use crate::ui::theme::{self, Elevation, Style, palette};
use crate::ui::widgets::{self, forge_icon};
use crate::views::dialogs::Dialog;
use crate::views::{commit_detail, state_banner};

/// Why an action waits.
const WAIT: &str = "Wait for the current operation";
/// Why an action waits for the details of the selected request.
const LOADING: &str = "Loading the request…";
/// Why an action is off when the details could not be loaded.
const DETAILS_FAILED: &str = "The request's details could not be loaded";
/// The widest a branch pill in the request header grows before it is cut.
const BRANCH_PILL_MAX: f32 = 240.0;
/// What the kit's select trigger spends besides its label and caret: 10 left of the label, 8
/// between it and the caret, 8 right of the caret (`kit::select`'s trigger padding).
const SELECT_PADDING: f32 = 26.0;

/// Draws the view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, api_enabled: bool, actions: &mut Vec<Action>) {
    state_banner::show(ui, tab, actions);
    let Some((_, forge)) = tab.forge().cloned() else {
        let local = tab.snap.as_ref().is_some_and(|s| s.remotes.is_empty());
        let (body, button) = no_forge(local);
        empty(
            ui,
            art::local_only(),
            "No GitHub or GitLab remote",
            body,
            Some(button),
            actions,
        );
        return;
    };
    let kind = forge.kind;
    if !api_enabled {
        empty(
            ui,
            art::local_only(),
            &format!("{}s are turned off", cap(kind.request_noun())),
            "Turn on pull and merge requests in Settings → Integrations to load them from the API.",
            Some((
                format!("{}  Open on {}", forge_icon(kind), kind.name()),
                Action::OpenUrl(format!(
                    "{}/{}",
                    forge.web_url(),
                    if kind == ForgeKind::GitHub {
                        "pulls"
                    } else {
                        "-/merge_requests"
                    }
                )),
            )),
            actions,
        );
        return;
    }

    let s = theme::surfaces(ui.ctx());
    let deeper = theme::style() == Style::Deeper;
    // Standard sets the list on the chrome tone beside the content; Deeper keeps it on the card.
    let list_ground = if deeper { s.pane } else { s.panel };
    // Deeper: each pane is a card, with half a gutter of ground on its side of the seam.
    let half_gutter = if deeper {
        crate::ui::points_i8(theme::metrics().pane_gutter / 2.0)
    } else {
        0
    };
    let pane_max = (ui.available_width() * 0.4).max(160.0);
    let list_panel = egui::Panel::left(LIST_PANEL)
        .resizable(true)
        .default_size(pane_max.min(380.0))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(640.0))
        .frame(egui::Frame::new().fill(list_ground).inner_margin(Margin {
            right: half_gutter,
            ..Margin::ZERO
        }))
        .show(ui, |ui| {
            let r = ui.max_rect();
            if !deeper {
                paint::seam_v(ui.painter(), r.right() - 0.5, r.y_range(), s.border);
            }
            list(ui, tab, kind, list_ground, actions);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(s.pane).inner_margin(Margin {
            left: half_gutter,
            ..Margin::ZERO
        }))
        .show(ui, |ui| detail(ui, tab, kind, actions));
    if deeper {
        let card = ui.max_rect();
        let seam = list_panel.response.rect.right();
        let gap = theme::metrics().pane_gutter;
        let cards = [
            Rect::from_x_y_ranges(card.left()..=seam - gap / 2.0, card.y_range()),
            Rect::from_x_y_ranges(seam + gap / 2.0..=card.right(), card.y_range()),
        ];
        two_cards(ui, cards);
    }
}

/// The list panel's id: its resize handle, `<id>/__resize`, marks the seam between the two cards.
const LIST_PANEL: &str = "requests-list";

fn list_handle() -> Id {
    Id::new(LIST_PANEL).with("__resize")
}

/// Deeper: the list and the details as two cards side by side, as History's table and details are
/// (`app/mod.rs` `split_gutter`, turned on its side). The content pane paints one card behind
/// both; this paints the gutter between them over the content, the ground, the shadows falling
/// into it, each card's inner corners and edges, and the seam handle's line while it is hovered or
/// dragged. The focused-pane edge the content pane draws still spans the pair.
fn two_cards(ui: &Ui, cards: [Rect; 2]) {
    let [left, right] = cards;
    let ctx = ui.ctx();
    let p = palette(ctx);
    let s = theme::surfaces(ctx);
    let m = theme::metrics();
    // Unclipped: the cards' edges lie on the content pane's clip.
    let painter = ctx.layer_painter(ui.layer_id());
    let radius = CornerRadius::same(m.pane_radius);
    let band = Rect::from_x_y_ranges(left.right()..=right.left(), left.y_range());
    painter.rect_filled(band, CornerRadius::ZERO, s.bg);
    let material = p.material(Style::Deeper, Elevation::Pane);
    for card in cards {
        painter
            .with_clip_rect(band)
            .extend(paint::shadow_shapes(card, radius, p, &material));
    }
    let r = f32::from(radius.nw);
    let quarter = std::f32::consts::FRAC_PI_2;
    for (corner, centre, from) in [
        (
            left.right_top(),
            left.right_top() + vec2(-r, r),
            3.0 * quarter,
        ),
        (left.right_bottom(), left.right_bottom() + vec2(-r, -r), 0.0),
        (
            right.left_top(),
            right.left_top() + vec2(r, r),
            2.0 * quarter,
        ),
        (
            right.left_bottom(),
            right.left_bottom() + vec2(r, -r),
            quarter,
        ),
    ] {
        corner_mask(&painter, corner, centre, r, from, s.bg);
    }
    if let Some(edge) = s.pane_edge {
        paint::rim(&painter, left, radius, edge);
        paint::rim(&painter, right, radius, edge);
    }
    if ctx
        .read_response(list_handle())
        .is_some_and(|handle| handle.hovered() || handle.dragged())
    {
        painter.vline(
            band.center().x,
            band.y_range().shrink(r),
            Stroke::new(2.0, theme::Palette::tint(p.focus, 150)),
        );
    }
}

/// Paints `color` over the part of a square corner outside its rounded arc: a fan from `corner`
/// over the quarter arc of radius `r` around `centre` that starts at angle `from`.
fn corner_mask(
    painter: &Painter,
    corner: egui::Pos2,
    centre: egui::Pos2,
    r: f32,
    from: f32,
    color: Color32,
) {
    const STEPS: u16 = 8;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(corner, color);
    for i in 0..=STEPS {
        let a = from + std::f32::consts::FRAC_PI_2 * f32::from(i) / f32::from(STEPS);
        mesh.colored_vertex(centre + r * vec2(a.cos(), a.sin()), color);
    }
    for i in 1..=u32::from(STEPS) {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(mesh);
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| format!("{}{}", f.to_uppercase(), c.as_str()))
        .unwrap_or_default()
}

/// A request's number as its host writes it: `#425` for a pull request, `!425` for a merge request.
fn number_text(kind: ForgeKind, number: u64) -> String {
    format!("{}{number}", kind_sigil(kind))
}

fn empty(
    ui: &mut Ui,
    image: art::Art,
    title: &str,
    body: &str,
    button: Option<(String, Action)>,
    actions: &mut Vec<Action>,
) {
    let mut state = EmptyState::new(title)
        .art(image)
        .icon(icon::GIT_PULL_REQUEST)
        .body(body);
    if let Some((label, _)) = &button {
        state = state.action(label.clone());
    }
    if state.show(ui).value == Some(EmptyAction::Primary)
        && let Some((_, action)) = button
    {
        actions.push(action);
    }
}

/// What the requests view says, and offers, for a repository with no GitHub or GitLab remote:
/// with no remote at all, publishing it; otherwise, adding a GitHub or GitLab remote.
fn no_forge(local: bool) -> (&'static str, (String, Action)) {
    // The empty state shows two lines of body text: keep each sentence inside them.
    if local {
        (
            "Publish it to GitHub or GitLab to work with its pull and merge requests here.",
            ("Publish…".to_owned(), Action::Publish),
        )
    } else {
        (
            "Add a GitHub or GitLab remote to see its pull and merge requests here.",
            ("Add remote…".to_owned(), Action::AddRemoteDialog),
        )
    }
}

/// A request's state: its glyph, tone and word.
fn state_style(r: &RequestSummary) -> (&'static str, Tone, &'static str) {
    match (r.state, r.draft) {
        (RequestState::Open, true) => (icon::PENCIL_SIMPLE_LINE, Tone::Neutral, "Draft"),
        (RequestState::Open, false) => (icon::GIT_PULL_REQUEST, Tone::Success, "Open"),
        (RequestState::Merged, _) => (icon::GIT_MERGE, Tone::Accent, "Merged"),
        (RequestState::Closed, _) => (icon::GIT_PULL_REQUEST, Tone::Danger, "Closed"),
    }
}

// ------------------------------------------------------------------------------------- signals

/// Where a request's checks stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ci {
    Passing { total: usize },
    Failing { failed: usize, total: usize },
    Running { pending: usize, total: usize },
}

/// The standing of a request's reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Approved,
    ChangesRequested,
    Commented,
}

/// What a reviewer scans a list for: the checks and the reviews of a request. The list API sends
/// neither, so a row has them once the request's details have been loaded in this session (they
/// stay until the app closes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signals {
    ci: Option<Ci>,
    review: Option<Verdict>,
}

impl Signals {
    fn of(d: &RequestDetails) -> Self {
        let total = d.checks.len();
        let count = |state| d.checks.iter().filter(|c| c.state == state).count();
        let (failed, pending) = (count(CheckState::Failure), count(CheckState::Pending));
        let ci = if failed > 0 {
            Some(Ci::Failing { failed, total })
        } else if pending > 0 {
            Some(Ci::Running { pending, total })
        } else {
            (total > 0).then_some(Ci::Passing { total })
        };
        let said = |verdict: &str| d.reviews.iter().any(|r| r.state == verdict);
        let review = if said(CHANGES_REQUESTED) {
            Some(Verdict::ChangesRequested)
        } else if said("Approved") {
            Some(Verdict::Approved)
        } else {
            (!d.reviews.is_empty()).then_some(Verdict::Commented)
        };
        Self { ci, review }
    }

    /// The glyphs of the row, in order, with their colour tone and what they say.
    fn glyphs(self) -> Vec<(&'static str, Tone, String)> {
        let mut out = Vec::new();
        match self.ci {
            Some(Ci::Passing { total }) => out.push((
                icon::CHECK_CIRCLE,
                Tone::Success,
                format!("All {total} checks passed"),
            )),
            Some(Ci::Failing { failed, total }) => out.push((
                icon::X_CIRCLE,
                Tone::Danger,
                format!("{failed} of {total} checks failed"),
            )),
            Some(Ci::Running { pending, total }) => out.push((
                icon::CIRCLE_DASHED,
                Tone::Warning,
                format!("{pending} of {total} checks running"),
            )),
            None => {}
        }
        match self.review {
            Some(Verdict::Approved) => {
                out.push((icon::THUMBS_UP, Tone::Success, "Approved".to_owned()));
            }
            Some(Verdict::ChangesRequested) => out.push((
                icon::NOTE_PENCIL,
                Tone::Danger,
                CHANGES_REQUESTED.to_owned(),
            )),
            Some(Verdict::Commented) => {
                out.push((icon::CHAT_CIRCLE, Tone::Neutral, "Reviewed".to_owned()));
            }
            None => {}
        }
        out
    }
}

/// A reviewer's verdict that asks for changes, as the hosting crate words it.
const CHANGES_REQUESTED: &str = "Changes requested";

fn signals_id(tab: &RepoTab, number: u64) -> Id {
    Id::new(("request-signals", tab.path.as_path(), number))
}

/// Keeps the checks and reviews of `d` for the list rows (see [`Signals`]).
pub(crate) fn remember_signals(ctx: &egui::Context, tab: &RepoTab, d: &RequestDetails) {
    let signals = Signals::of(d);
    ctx.data_mut(|data| data.insert_temp(signals_id(tab, d.summary.number), signals));
}

fn known_signals(ctx: &egui::Context, tab: &RepoTab, number: u64) -> Option<Signals> {
    ctx.data(|data| data.get_temp(signals_id(tab, number)))
}

// ------------------------------------------------------------------------------------- the list

fn list(
    ui: &mut Ui,
    tab: &mut RepoTab,
    kind: ForgeKind,
    ground: Color32,
    actions: &mut Vec<Action>,
) {
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    egui::Frame::new()
        .inner_margin(Margin::same(crate::ui::points_i8(theme::SM)))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let options = StateFilter::ALL.map(|f| (f, f.label()));
                if Segmented::new(&mut tab.requests.filter, &options)
                    .label("Show requests")
                    .size(Size::Sm)
                    .show(ui)
                    .changed()
                {
                    tab.requests.list = None;
                    tab.load_requests();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if tab.requests.loading {
                        Spinner::new()
                            .size(Size::Sm)
                            .label(format!("{}s", kind.request_noun()))
                            .show(ui);
                    } else if IconButton::new(icon::ARROWS_CLOCKWISE, "Reload")
                        .size(Size::Sm)
                        .show(ui)
                        .clicked()
                    {
                        tab.load_requests();
                    }
                    let current = tab
                        .snap
                        .as_ref()
                        .and_then(|s| s.current_branch())
                        .filter(|b| b.upstream.is_some())
                        .map(|b| b.name.clone());
                    if let Some(branch) = current
                        && !tab.is_default_branch(&branch)
                        && Button::new(format!("New {}", kind.request_abbrev()))
                            .icon(icon::PLUS)
                            .size(Size::Sm)
                            .tooltip(format!("Create a {} for {branch}", kind.request_noun()))
                            .show(ui)
                            .clicked()
                    {
                        actions.push(Action::CreateRequest { source: branch });
                    }
                });
            });
        });
    let r = ui.available_rect_before_wrap();
    paint::seam_h(ui.painter(), r.x_range(), r.min.y + 0.5, s.border);

    match tab.requests.list.clone() {
        None => {
            // Rows in the shape of the real ones, once the wait is long enough to notice.
            egui::Frame::new()
                .inner_margin(Margin::symmetric(
                    crate::ui::points_i8(m.sidebar_pad),
                    crate::ui::points_i8(theme::XS),
                ))
                .show(ui, |ui| {
                    Skeleton::rows(LOADING_ROWS)
                        .row_height(RowKind::Request.height())
                        .id_salt("requests-skeleton")
                        .show(ui);
                });
        }
        Some(Err(_)) => {
            // The details pane carries the message and the way out; the list only says it is empty.
            let no_token = tab.requests.token == Some(None);
            EmptyState::new(if no_token {
                "A token is needed".to_owned()
            } else {
                "Nothing to list".to_owned()
            })
            .icon(icon::WARNING_OCTAGON)
            .compact(true)
            .id_salt("request-list-error")
            .show(ui);
        }
        Some(Ok(items)) if items.is_empty() => {
            EmptyState::new(format!(
                "No {} {}s",
                tab.requests.filter.label().to_lowercase(),
                kind.request_noun()
            ))
            .icon(icon::CHECK_CIRCLE)
            .compact(true)
            .show(ui);
        }
        Some(Ok(items)) => {
            let now = chrono::Utc::now().timestamp();
            let out = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(
                            crate::ui::points_i8(m.sidebar_pad),
                            crate::ui::points_i8(theme::XS),
                        ))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            commit_detail::virtual_rows(
                                ui,
                                items.len(),
                                RowKind::Request.height(),
                                |ui, i| {
                                    let r = &items[i];
                                    let selected = tab.requests.selected == Some(r.number);
                                    let signals = known_signals(ui.ctx(), tab, r.number);
                                    if row(ui, r, kind, signals, selected, now, ground).clicked() {
                                        tab.select_request(r.number);
                                    }
                                },
                            );
                        });
                });
            paint::scroll_fade(ui, &out, ground);
        }
    }
}

/// How many placeholder rows a loading list shows.
const LOADING_ROWS: usize = 7;

/// `text` on one line, cut at `width`.
fn line_galley(
    painter: &Painter,
    text: String,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple(text, font, color, width.max(0.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    painter.layout_job(job)
}

/// `text` as it is, or with its middle left out when it is wider than `width`: a branch name
/// keeps the start that names its kind (`feature/`) and the end that names the work.
fn middle_galley(
    painter: &Painter,
    text: &str,
    font: &FontId,
    color: Color32,
    width: f32,
) -> Arc<Galley> {
    let whole = painter.layout_no_wrap(text.to_owned(), font.clone(), color);
    if whole.size().x <= width {
        return whole;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut best = painter.layout_no_wrap("…".to_owned(), font.clone(), color);
    let (mut low, mut high) = (1, chars.len().saturating_sub(1));
    while low <= high {
        let keep = usize::midpoint(low, high);
        let galley = painter.layout_no_wrap(elide_middle(&chars, keep), font.clone(), color);
        if galley.size().x <= width {
            best = galley;
            low = keep + 1;
        } else {
            high = keep - 1;
        }
    }
    best
}

/// The first and last of `keep` characters of `chars` around an ellipsis, the start a letter
/// longer when `keep` is odd.
fn elide_middle(chars: &[char], keep: usize) -> String {
    let head = keep.div_ceil(2).min(chars.len());
    let tail = (keep / 2).min(chars.len() - head);
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

/// How `room` is shared by two texts of natural widths `a` and `b`: both whole when they fit,
/// else the shorter one whole and the other the rest, else half each.
fn fit_pair(a: f32, b: f32, room: f32) -> (f32, f32) {
    let room = room.max(0.0);
    if a + b <= room {
        return (a, b);
    }
    let half = room / 2.0;
    if a <= half {
        (a, room - a)
    } else if b <= half {
        (room - b, b)
    } else {
        (half, half)
    }
}

/// A request in the list: state glyph, title, number, author and age, its branches, its labels,
/// and at the right of the title the checks and reviews when they are known.
fn row(
    ui: &mut Ui,
    r: &RequestSummary,
    kind: ForgeKind,
    signals: Option<Signals>,
    selected: bool,
    now: i64,
    ground: Color32,
) -> Response {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (glyph, tone, state) = state_style(r);
    let color = tone.color(p);
    let number = number_text(kind, r.number);
    let glyphs = signals.map(Signals::glyphs).unwrap_or_default();
    let mut tooltip = format!(
        "{state} · {}\n{number} · {}\n{} → {}",
        r.title, r.author, r.source_branch, r.target_branch
    );
    let mut info = format!("{state} request {number} {} by {}", r.title, r.author);
    for (_, _, said) in &glyphs {
        tooltip = format!("{tooltip}\n{said}");
        info = format!("{info}, {said}");
    }
    if !r.labels.is_empty() {
        tooltip = format!("{tooltip}\nLabels: {}", r.labels.join(", "));
    }
    let response = ListRow::new(("request", r.number))
        .kind(RowKind::Request)
        .selected(selected)
        .ground(ground)
        .tooltip(tooltip)
        .show_with(ui, |rc, painter| {
            let c = rc.content;
            let h = rc.rect.height();
            // Three lines: title, then number, author and age, then the branches and labels.
            let line = |k: f32| rc.rect.top() + h * k;
            let (title_y, meta_y, branch_y) = (line(0.27), line(0.52), line(0.76));
            painter.text(
                pos2(c.left() + m.icon_md / 2.0, title_y),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(m.icon_md),
                color,
            );
            let x = c.left() + m.icon_md + theme::SM;
            let width = (c.right() - x).max(0.0);
            let at = |g: &Arc<Galley>, x: f32, y: f32| pos2(x, y - g.size().y / 2.0);
            // The checks and the reviews at the right of the title line.
            let mut right = c.right();
            for (glyph, tone, _) in glyphs.iter().rev() {
                painter.text(
                    pos2(right, title_y),
                    Align2::RIGHT_CENTER,
                    *glyph,
                    FontId::proportional(m.icon_md),
                    tone.color(p),
                );
                right -= m.icon_md + theme::XS;
            }
            let title_width = if glyphs.is_empty() {
                width
            } else {
                (right - x - theme::XS).max(0.0)
            };
            let title = line_galley(
                painter,
                r.title.clone(),
                FontId::new(m.body_size, theme::medium()),
                rc.text,
                title_width,
            );
            painter.galley(at(&title, x, title_y), title, rc.text);
            let meta = line_galley(
                painter,
                format!(
                    "{number}  ·  {}  ·  {}",
                    r.author,
                    widgets::relative_time(r.updated_at, now)
                ),
                FontId::proportional(m.small_size),
                p.text_faint,
                width,
            );
            painter.galley(at(&meta, x, meta_y), meta, p.text_faint);

            // Both branch names whole when the row has the room, else shared fairly.
            let small = FontId::proportional(m.small_size);
            let medium = FontId::new(m.small_size, theme::medium());
            let natural = |text: &str, font: &FontId| {
                painter
                    .layout_no_wrap(text.to_owned(), font.clone(), p.text_dim)
                    .size()
                    .x
            };
            let (source_natural, target_natural) = (
                natural(&r.source_branch, &small),
                natural(&r.target_branch, &medium),
            );
            let (source_room, target_room) =
                fit_pair(source_natural, target_natural, (width - theme::XL).max(0.0));
            let source = middle_galley(painter, &r.source_branch, &small, p.text_dim, source_room);
            let source_width = source.size().x;
            painter.galley(at(&source, x, branch_y), source, p.text_dim);
            painter.text(
                pos2(x + source_width + theme::SM, branch_y),
                Align2::LEFT_CENTER,
                "→",
                small.clone(),
                p.text_faint,
            );
            let target = middle_galley(painter, &r.target_branch, &medium, p.text_dim, target_room);
            let target_width = target.size().x;
            painter.galley(
                at(&target, x + source_width + theme::XL, branch_y),
                target,
                p.text_dim,
            );
            // Labels take what the branches leave.
            let used = source_width + theme::XL + target_width + theme::MD;
            paint_labels(
                painter,
                &r.labels,
                &small,
                (width - used).max(0.0),
                c.right(),
                branch_y,
            );
        });
    name_row(ui, &response, selected, &info);
    response
}

/// The labels that fit in `room`, as small chips whose right end is at `right`; a last chip says
/// how many more there are.
fn paint_labels(
    painter: &Painter,
    labels: &[String],
    font: &FontId,
    room: f32,
    right: f32,
    y: f32,
) {
    let p = palette(painter.ctx());
    let pad = theme::XS + 2.0;
    let chip = |text: String| {
        let galley = painter.layout_no_wrap(text, font.clone(), p.text_dim);
        let width = galley.size().x + 2.0 * pad;
        (galley, width)
    };
    let mut fits = Vec::new();
    let mut used = 0.0;
    for label in labels {
        let (galley, width) = chip(label.clone());
        let gap = if fits.is_empty() { 0.0 } else { theme::XS };
        if used + gap + width > room {
            break;
        }
        used += gap + width;
        fits.push((galley, width));
    }
    let hidden = labels.len() - fits.len();
    if hidden > 0 {
        // A lone "+3" says nothing: when no label fits, the chip counts them instead.
        let (galley, width) = chip(if fits.is_empty() {
            format!("{hidden} label{}", if hidden == 1 { "" } else { "s" })
        } else {
            format!("+{hidden}")
        });
        let gap = if fits.is_empty() { 0.0 } else { theme::XS };
        if used + gap + width <= room {
            fits.push((galley, width));
            used += gap + width;
        } else if fits.is_empty() {
            return;
        }
    }
    let mut x = right - used;
    for (galley, width) in fits {
        let height = galley.size().y + 2.0;
        let rect = Rect::from_min_size(pos2(x, y - height / 2.0), vec2(width, height));
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::metrics().radius_sm),
            Tone::Neutral.tint(p),
        );
        painter.galley(pos2(x + pad, y - galley.size().y / 2.0), galley, p.text_dim);
        x += width + theme::XS;
    }
}

/// Names a custom-painted list row for screen readers (the kit row speaks its label slot).
fn name_row(ui: &Ui, response: &Response, selected: bool, name: &str) {
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, name)
    });
    ui.ctx().accesskit_node_builder(response.id, |n| {
        n.set_role(egui::accesskit::Role::ListItem);
        n.clear_toggled();
        n.set_selected(selected);
    });
}

// ------------------------------------------------------------------------------------ the details

/// The right pane when no request is selected: what the list is waiting for, or why it has nothing.
fn unselected(ui: &mut Ui, tab: &mut RepoTab, kind: ForgeKind, actions: &mut Vec<Action>) {
    let noun = kind.request_noun();
    match &tab.requests.list {
        Some(Err(message)) => {
            // Why, in the host's own words, and the way out; the list beside it only says "none".
            let no_token = tab.requests.token == Some(None);
            let title = if no_token {
                format!("Connect {} to see {noun}s", kind.name())
            } else {
                format!("Could not load {noun}s")
            };
            let body = message.clone();
            // The main action leads: a token when there is none, else another try.
            let (main, other) = if no_token {
                ("Add a token…", "Retry")
            } else {
                ("Retry", "Add a token…")
            };
            let clicked = EmptyState::new(title)
                .art(art::empty_error())
                .icon(icon::WARNING_OCTAGON)
                .body(body)
                .action(main)
                .link(other)
                .id_salt("request-error")
                .show(ui)
                .value;
            match clicked {
                Some(EmptyAction::Primary) if no_token => actions.push(Action::Settings),
                Some(EmptyAction::Link) if !no_token => actions.push(Action::Settings),
                Some(_) => tab.load_requests(),
                None => {}
            }
        }
        _ => {
            EmptyState::new(format!("Select a {noun} to see it"))
                .icon(icon::GIT_PULL_REQUEST)
                .compact(true)
                .show(ui);
        }
    }
}

fn detail(ui: &mut Ui, tab: &mut RepoTab, kind: ForgeKind, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let ground = theme::surfaces(ui.ctx()).pane;
    let Some(number) = tab.requests.selected else {
        unselected(ui, tab, kind, actions);
        return;
    };
    let summary = match &tab.requests.list {
        Some(Ok(l)) => l.iter().find(|r| r.number == number).cloned(),
        _ => None,
    };
    let details = match &tab.requests.details {
        Some(Ok(d)) if d.summary.number == number => Some(Arc::clone(d)),
        _ => None,
    };
    let Some(summary) = details.as_ref().map(|d| d.summary.clone()).or(summary) else {
        ui.centered_and_justified(|ui| {
            Spinner::new().label(kind.request_noun()).show(ui);
        });
        return;
    };
    if let Some(d) = &details {
        remember_signals(ui.ctx(), tab, d);
    }
    let now = chrono::Utc::now().timestamp();
    let out = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(
                    crate::ui::points_i8(theme::XL),
                    crate::ui::points_i8(theme::LG),
                ))
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(theme::MEASURE));
                    // Header.
                    let (glyph, tone, state) = state_style(&summary);
                    ui.horizontal(|ui| {
                        Badge::new(format!("{glyph} {state}")).tone(tone).show(ui);
                        ui.label(
                            RichText::new(number_text(kind, summary.number))
                                .size(m.subtitle_size)
                                .color(p.text_faint),
                        );
                    });
                    ui.add_space(theme::XS);
                    ui.label(
                        RichText::new(&summary.title)
                            .font(FontId::new(m.headline_size, theme::semibold()))
                            .color(p.text),
                    );
                    ui.add_space(theme::XS);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(&summary.author)
                                .font(FontId::new(m.body_size, theme::semibold()))
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new("wants to merge")
                                .size(m.body_size)
                                .color(p.text_dim),
                        );
                        branch_pill(ui, &summary.source_branch);
                        ui.label(RichText::new("into").size(m.body_size).color(p.text_dim));
                        branch_pill(ui, &summary.target_branch);
                        ui.label(
                            RichText::new(format!(
                                "· updated {}",
                                widgets::relative_time(summary.updated_at, now)
                            ))
                            .color(p.text_faint)
                            .size(m.meta_size),
                        );
                    });
                    if !summary.labels.is_empty() {
                        ui.add_space(theme::XS);
                        ui.horizontal_wrapped(|ui| {
                            for l in &summary.labels {
                                Badge::new(l.clone()).show(ui);
                            }
                        });
                    }
                    ui.add_space(theme::MD);
                    action_bar(ui, tab, kind, &summary, details.as_deref(), actions);
                    ui.add_space(theme::LG);

                    let error = match &tab.requests.details {
                        Some(Err(e)) => Some(e.clone()),
                        _ => None,
                    };
                    match (error, details.as_ref()) {
                        (Some(e), _) => {
                            let retry = Callout::new(Tone::Danger, e)
                                .title(format!("Could not load this {}", kind.request_noun()))
                                .action(NoticeAction::new("Retry").icon(icon::ARROW_CLOCKWISE))
                                .id_salt("details-error")
                                .show_actions(ui)
                                .value;
                            if retry.is_some() {
                                tab.requests.details = None;
                                tab.requests.details_loading = true;
                                tab.host_job(HostJob::Details(number));
                            }
                        }
                        (None, Some(d)) => {
                            body(ui, d, now, actions);
                            comment_box(ui, tab, number, actions);
                        }
                        (None, None) => {
                            Skeleton::rows(DETAILS_PLACEHOLDER_ROWS)
                                .row_height(theme::LG + theme::SM)
                                .id_salt("details-skeleton")
                                .show(ui);
                        }
                    }
                });
        });
    paint::scroll_fade(ui, &out, ground);
}

/// How many placeholder lines the body shows while the details load.
const DETAILS_PLACEHOLDER_ROWS: usize = 6;

fn branch_pill(ui: &mut Ui, name: &str) {
    RefPill::new(name, RefKind::LocalBranch)
        .max_width(BRANCH_PILL_MAX)
        .tooltip(name)
        .show(ui);
}

/// `#` for GitHub pull requests, `!` for GitLab merge requests.
fn kind_sigil(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "#",
        ForgeKind::GitLab => "!",
    }
}

/// A line under the merge controls that says what holds the merge back or puts it at risk.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    tone: Tone,
    glyph: &'static str,
    text: String,
}

/// Why the merge button is off: the tooltip, and a line under the controls when the reason is
/// worth reading without hovering.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Blocker {
    reason: &'static str,
    note: Option<Note>,
}

impl Blocker {
    const fn quiet(reason: &'static str) -> Self {
        Self { reason, note: None }
    }
}

/// Why `r` cannot be merged now, or `None` when it can. `d` is its loaded details, if any.
fn merge_blocker(tab: &RepoTab, r: &RequestSummary, d: Option<&RequestDetails>) -> Option<Blocker> {
    if tab.requests.busy.is_some() || tab.running.is_some() {
        return Some(Blocker::quiet(WAIT));
    }
    if r.state != RequestState::Open {
        return Some(Blocker::quiet("Only an open request can be merged"));
    }
    if r.draft {
        return Some(Blocker {
            reason: "Drafts cannot be merged",
            note: Some(Note {
                tone: Tone::Neutral,
                glyph: icon::PENCIL_SIMPLE_LINE,
                text: "Draft: mark it ready for review to merge it.".into(),
            }),
        });
    }
    if matches!(tab.requests.details, Some(Err(_))) {
        return Some(Blocker::quiet(DETAILS_FAILED));
    }
    let Some(details) = d.filter(|details| details.summary.number == r.number) else {
        return Some(Blocker::quiet(LOADING));
    };
    if details.mergeable == Some(false) {
        return Some(Blocker {
            reason: "This request has conflicts",
            note: Some(Note {
                tone: Tone::Danger,
                glyph: icon::WARNING_CIRCLE,
                text: "It has conflicts: resolve them on its branch before merging.".into(),
            }),
        });
    }
    if details.head_sha.as_ref().is_none_or(String::is_empty) {
        return Some(Blocker::quiet("The request's head commit is not known yet"));
    }
    None
}

/// What speaks against merging a request that can be merged: failed or running checks, a review
/// asking for changes, a conflict check still running. Each with how serious it is.
fn merge_risks(d: &RequestDetails) -> Vec<(Tone, String)> {
    let mut risks = Vec::new();
    let total = d.checks.len();
    let count = |state| d.checks.iter().filter(|c| c.state == state).count();
    let failed = count(CheckState::Failure);
    if failed > 0 {
        risks.push((Tone::Danger, format!("{failed} of {total} checks failed")));
    }
    let pending = count(CheckState::Pending);
    if pending > 0 {
        risks.push((
            Tone::Warning,
            format!("{pending} of {total} checks running"),
        ));
    }
    let askers: Vec<&str> = d
        .reviews
        .iter()
        .filter(|r| r.state == CHANGES_REQUESTED)
        .map(|r| r.author.as_str())
        .collect();
    if !askers.is_empty() {
        risks.push((
            Tone::Danger,
            format!("Changes requested by {}", askers.join(", ")),
        ));
    }
    if d.mergeable.is_none() {
        risks.push((Tone::Warning, "The conflict check is still running".into()));
    }
    risks
}

/// The risks as one line, in the most serious tone among them.
fn risk_note(risks: &[(Tone, String)]) -> Option<Note> {
    let tone = if risks.iter().any(|(tone, _)| *tone == Tone::Danger) {
        Tone::Danger
    } else if risks.is_empty() {
        return None;
    } else {
        Tone::Warning
    };
    let text: Vec<&str> = risks.iter().map(|(_, text)| text.as_str()).collect();
    Some(Note {
        tone,
        glyph: icon::WARNING,
        text: text.join("  ·  "),
    })
}

/// The width a select needs to show its longest label whole.
fn select_width(ui: &Ui, labels: impl Iterator<Item = &'static str>) -> f32 {
    let m = theme::metrics();
    let font = FontId::new(m.body_size, FontFamily::Proportional);
    let widest = labels
        .map(|label| {
            ui.painter()
                .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    (widest + SELECT_PADDING + m.icon_md).ceil()
}

/// The action buttons. Merge and close ask for confirmation in a dialog. `d` is the loaded
/// details of `r`; without them (still loading, or failed) nothing that needs them is on.
fn action_bar(
    ui: &mut Ui,
    tab: &mut RepoTab,
    kind: ForgeKind,
    r: &RequestSummary,
    d: Option<&RequestDetails>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let busy = tab.requests.busy.is_some() || tab.running.is_some();
    let failed = matches!(tab.requests.details, Some(Err(_)));
    let loading = d.is_none() && !failed;
    let (checkout_off, checkout_reason) = if busy {
        (true, WAIT)
    } else if loading {
        (true, LOADING)
    } else {
        (false, WAIT)
    };
    ui.horizontal_wrapped(|ui| {
        if Button::new("Check out")
            .icon(icon::DOWNLOAD_SIMPLE)
            .enabled(!checkout_off)
            .tooltip("Fetch the request's branch and check it out locally")
            .disabled_reason(checkout_reason)
            .show(ui)
            .clicked()
        {
            actions.push(Action::CheckoutRequest(r.clone()));
        }
        if Button::new(format!("{}  Open on {}", forge_icon(kind), kind.name()))
            .show(ui)
            .clicked()
        {
            actions.push(Action::OpenUrl(r.web_url.clone()));
        }
        if IconButton::new(icon::LINK, "Copy link").show(ui).clicked() {
            actions.push(Action::Copy(r.web_url.clone()));
        }
        let more = Button::new("More").caret().show(ui);
        kit::menu::menu(&more, |menu| match r.state {
            RequestState::Open => {
                if menu
                    .item(icon::X_CIRCLE, "Close request…")
                    .danger()
                    .enabled(!busy)
                    .disabled_reason(WAIT)
                    .show(menu.ui)
                    .clicked()
                {
                    actions.push(Action::OpenDialog(Dialog::ConfirmHost {
                        title: format!(
                            "Close {} {}?",
                            kind.request_abbrev(),
                            number_text(kind, r.number)
                        ),
                        body: format!(
                            "\"{}\" will be closed without merging. You can reopen it later.",
                            r.title
                        ),
                        confirm: "Close".into(),
                        danger: true,
                        job: HostJob::SetOpen(r.number, false),
                    }));
                }
            }
            RequestState::Closed => {
                if menu
                    .item(icon::ARROW_COUNTER_CLOCKWISE, "Reopen request")
                    .enabled(!busy)
                    .disabled_reason(WAIT)
                    .show(menu.ui)
                    .clicked()
                {
                    actions.push(Action::Host(HostJob::SetOpen(r.number, true)));
                }
            }
            RequestState::Merged => {
                menu.item(icon::GIT_MERGE, "Already merged")
                    .enabled(false)
                    .disabled_reason("A merged request cannot be closed or reopened")
                    .show(menu.ui);
            }
        });
    });
    if r.state == RequestState::Open {
        ui.add_space(theme::SM);
        let blocker = merge_blocker(tab, r, d);
        let risks = if blocker.is_none() {
            d.map(merge_risks).unwrap_or_default()
        } else {
            Vec::new()
        };
        let approve = approve_action(tab, r);
        let approve_reason = if busy {
            WAIT
        } else if r.draft {
            "Drafts cannot be approved yet"
        } else if failed {
            DETAILS_FAILED
        } else {
            LOADING
        };
        ui.horizontal_wrapped(|ui| {
            if Button::new("Approve")
                .icon(icon::CHECK)
                .icon_tone(Tone::Success)
                .enabled(approve.is_some())
                .tooltip("Approve this request")
                .shortcut(Command::ApproveRequest)
                .disabled_reason(approve_reason)
                .show(ui)
                .clicked()
                && let Some(action) = approve
            {
                actions.push(action);
            }
            if ui.available_size_before_wrap().x < 220.0 {
                ui.end_row();
            }
            InsetBox::new().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let methods = MergeMethod::ALL.map(|method| (method, method.label()));
                    let fit =
                        select_width(ui, MergeMethod::ALL.into_iter().map(MergeMethod::label));
                    Select::new(&mut tab.requests.merge_method, &methods)
                        .name("Merge method")
                        .width(fit.min(ui.available_width().max(80.0)))
                        .id_salt("merge-method")
                        .show(ui);
                    let dialog = merge_confirmation(tab, kind, r, d);
                    // Merge leads only when nothing speaks against it.
                    let mut merge = Button::new("Merge…")
                        .icon(icon::GIT_MERGE)
                        .enabled(dialog.is_some())
                        .tooltip("Confirm this merge before it runs")
                        .disabled_reason(blocker.as_ref().map_or(WAIT, |b| b.reason));
                    if risks.is_empty() {
                        merge = merge.primary();
                    } else {
                        merge = merge.icon_tone(Tone::Warning);
                    }
                    if merge.show(ui).clicked()
                        && let Some(dialog) = dialog
                    {
                        actions.push(Action::OpenDialog(dialog));
                    }
                });
            });
        });
        let note = blocker.and_then(|b| b.note).or_else(|| risk_note(&risks));
        if let Some(note) = note {
            ui.add_space(theme::XS);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(note.glyph)
                        .size(m.icon_md)
                        .color(note.tone.color(p)),
                );
                ui.label(RichText::new(note.text).size(m.meta_size).color(p.text_dim));
            });
        }
    }
    if loading {
        ui.add_space(theme::SM);
        ui.horizontal(|ui| {
            Spinner::new().size(Size::Sm).label("details").show(ui);
            ui.label(
                RichText::new("Loading details…")
                    .size(m.meta_size)
                    .color(p.text_dim),
            );
        });
    }
    if let Some(b) = &tab.requests.busy {
        ui.horizontal(|ui| {
            Spinner::new().size(Size::Sm).label(b.clone()).show(ui);
            ui.label(RichText::new(b).size(m.meta_size).color(p.accent));
        });
    }
    if let Some(Some(source)) = tab.requests.token
        && source != TokenSource::Saved
    {
        ui.label(
            RichText::new(format!("{}  Using the token {}", icon::KEY, source.label()))
                .size(m.small_size)
                .color(p.text_faint),
        );
    }
}

/// The selected request's details carry the state displayed by its action buttons.
pub(crate) fn selected_summary(tab: &RepoTab) -> Option<&RequestSummary> {
    let selected = tab.requests.selected?;
    tab.requests
        .details
        .as_ref()
        .and_then(|details| details.as_ref().ok())
        .filter(|details| details.summary.number == selected)
        .map(|details| &details.summary)
        .or_else(|| {
            tab.requests
                .list
                .as_ref()?
                .as_ref()
                .ok()?
                .iter()
                .find(|request| request.number == selected)
        })
}

/// Approving needs an open request that is not a draft, whose details have loaded.
pub(crate) fn approve_action(tab: &RepoTab, request: &RequestSummary) -> Option<Action> {
    let loaded = tab
        .requests
        .details
        .as_ref()
        .and_then(|details| details.as_ref().ok())
        .is_some_and(|details| details.summary.number == request.number);
    (request.state == RequestState::Open
        && !request.draft
        && loaded
        && tab.requests.busy.is_none()
        && tab.running.is_none())
    .then(|| Action::Host(HostJob::Approve(request.number)))
}

/// Shared by the pointer and keyboard paths; both confirm the reviewed head before merging.
pub(crate) fn merge_confirmation(
    tab: &RepoTab,
    kind: ForgeKind,
    r: &RequestSummary,
    d: Option<&RequestDetails>,
) -> Option<Dialog> {
    if merge_blocker(tab, r, d).is_some() {
        return None;
    }
    let details = d?;
    let head = details.head_sha.as_ref()?;
    let short: String = head.chars().take(7).collect();
    let method = tab.requests.merge_method;
    let risks = merge_risks(details);
    let warning = if risks.is_empty() {
        String::new()
    } else {
        let said: Vec<&str> = risks.iter().map(|(_, text)| text.as_str()).collect();
        format!("Heads up: {}. ", said.join("; "))
    };
    Some(Dialog::ConfirmHost {
        title: format!(
            "Merge {} {}?",
            kind.request_abbrev(),
            number_text(kind, r.number)
        ),
        body: format!(
            "{warning}{} \"{}\" ({}) into {} on {}, as reviewed at commit {short}. If anyone pushes to it before the merge runs, {} refuses the merge. This cannot be undone from Canopy.",
            method.label(),
            r.title,
            r.source_branch,
            r.target_branch,
            kind.name(),
            kind.name()
        ),
        confirm: if risks.is_empty() {
            "Merge"
        } else {
            "Merge anyway"
        }
        .into(),
        danger: risks.iter().any(|(tone, _)| *tone == Tone::Danger),
        job: HostJob::Merge(r.number, method, Some(head.clone())),
    })
}

fn body(ui: &mut Ui, d: &RequestDetails, now: i64, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    // Stats.
    let mut stats = Vec::new();
    if let Some(c) = d.commits {
        stats.push(format!("{c} commit{}", if c == 1 { "" } else { "s" }));
    }
    if let Some(f) = d.changed_files {
        stats.push(format!("{f} file{} changed", if f == 1 { "" } else { "s" }));
    }
    if let (Some(a), Some(r)) = (d.additions, d.deletions) {
        stats.push(format!("+{a} −{r}"));
    }
    if let Some(m) = d.mergeable {
        stats.push(if m {
            "no conflicts".into()
        } else {
            "has conflicts".into()
        });
    }
    if !stats.is_empty() {
        ui.label(
            RichText::new(stats.join("  ·  "))
                .size(m.meta_size)
                .color(p.text_dim),
        );
        ui.add_space(theme::MD);
    }

    // Description.
    InsetBox::new().show(ui, |ui| {
        ui.set_width(ui.available_width());
        if d.body.trim().is_empty() {
            ui.label(
                RichText::new("No description.")
                    .size(m.body_size)
                    .italics()
                    .color(p.text_faint),
            );
        } else {
            crate::ui::notes::markdown(ui, d.body.trim());
        }
    });
    ui.add_space(theme::LG);

    // Checks.
    if !d.checks.is_empty() {
        let failed = d
            .checks
            .iter()
            .filter(|c| c.state == CheckState::Failure)
            .count();
        let pending = d
            .checks
            .iter()
            .filter(|c| c.state == CheckState::Pending)
            .count();
        let (glyph, text) = if failed > 0 {
            (
                icon::X_CIRCLE,
                format!("{failed} of {} checks failed", d.checks.len()),
            )
        } else if pending > 0 {
            (
                icon::CIRCLE_DASHED,
                format!("{pending} of {} checks running", d.checks.len()),
            )
        } else {
            (
                icon::CHECK_CIRCLE,
                format!("All {} checks passed", d.checks.len()),
            )
        };
        // Open at first when something failed; then as the reviewer left it.
        let open_id = ui.id().with(("checks-open", d.summary.number));
        let mut open = ui.data(|data| data.get_temp(open_id)).unwrap_or(failed > 0);
        Disclosure::new("checks", text, &mut open)
            .icon(glyph)
            .show(ui, |ui| {
                for c in &d.checks {
                    let (g, col, word) = match c.state {
                        CheckState::Success => (icon::CHECK_CIRCLE, p.success, "passed"),
                        CheckState::Failure => (icon::X_CIRCLE, p.danger, "failed"),
                        CheckState::Pending => (icon::CIRCLE_DASHED, p.warning, "running"),
                        CheckState::Neutral => (icon::MINUS_CIRCLE, p.text_faint, "neutral"),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(g).size(m.icon_md).color(col))
                            .kit_tooltip(word)
                            .show();
                        match &c.url {
                            Some(u) => {
                                if LinkButton::new(&c.name).external(true).show(ui).clicked() {
                                    actions.push(Action::OpenUrl(u.clone()));
                                }
                            }
                            None => {
                                ui.label(RichText::new(&c.name).size(m.body_size).color(p.text));
                            }
                        }
                    });
                }
            });
        ui.data_mut(|data| data.insert_temp(open_id, open));
        ui.add_space(theme::SM);
    }

    // Reviews.
    if !d.reviews.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Reviews").size(m.meta_size).color(p.text_dim));
            for r in &d.reviews {
                let (g, c) = match r.state.as_str() {
                    "Approved" => (icon::CHECK_CIRCLE, p.success),
                    "Changes requested" => (icon::WARNING_CIRCLE, p.danger),
                    _ => (icon::CHAT_CIRCLE, p.text_dim),
                };
                ui.label(
                    RichText::new(format!("{g} {}: {}", r.author, r.state))
                        .size(m.meta_size)
                        .color(c),
                )
                .kit_tooltip(&r.state)
                .show();
            }
        });
        ui.add_space(theme::MD);
    }

    // Conversation.
    if !d.comments.is_empty() {
        SectionHeader::new("Conversation")
            .icon(icon::CHATS)
            .count(d.comments.len())
            .first(true)
            .show(ui);
        ui.add_space(theme::XS);
        for c in &d.comments {
            ui.horizontal_top(|ui| {
                Avatar::new(Person::new(&c.author, &c.author))
                    .size(AvatarSize::Detail)
                    .show(ui);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&c.author)
                                .font(FontId::new(m.body_size, theme::semibold()))
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(widgets::relative_time(c.created_at, now))
                                .size(m.small_size)
                                .color(p.text_faint),
                        );
                    });
                    crate::ui::notes::markdown(ui, c.body.trim());
                });
            });
            ui.add_space(theme::MD);
        }
    }
}

/// The comment box under the conversation. Ctrl+Enter posts the comment.
fn comment_box(ui: &mut Ui, tab: &mut RepoTab, number: u64, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let submitted = TextArea::new(&mut tab.requests.comment)
        .name("Comment")
        .placeholder("Leave a comment (Markdown)…")
        .rows(3..=12)
        .show(ui)
        .value;
    ui.add_space(theme::SM);
    let empty = tab.requests.comment.trim().is_empty();
    let can = !empty && tab.requests.busy.is_none();
    ui.horizontal(|ui| {
        let clicked = Button::new("Comment")
            .primary()
            .enabled(can)
            .tooltip("Post this comment")
            .disabled_reason(if empty {
                "Write a comment to enable"
            } else {
                WAIT
            })
            .show(ui)
            .clicked();
        if (clicked || submitted) && can {
            actions.push(Action::Host(HostJob::Comment(
                number,
                tab.requests.comment.trim().to_owned(),
            )));
        }
        if empty {
            ui.label(
                RichText::new("Write a comment to enable")
                    .size(m.small_size)
                    .color(p.text_faint),
            );
        }
    });
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::worker::Worker;

    /// qa-a F13: the empty state without a forge offers the way out, not only an explanation.
    #[test]
    fn without_a_forge_the_view_offers_publish_or_add_remote() {
        let (_, (label, action)) = no_forge(true);
        assert_eq!(label, "Publish…");
        assert!(matches!(action, Action::Publish));
        let (_, (label, action)) = no_forge(false);
        assert_eq!(label, "Add remote…");
        assert!(matches!(action, Action::AddRemoteDialog));
    }

    fn summary(state: RequestState) -> RequestSummary {
        RequestSummary {
            number: 7,
            title: "Request".into(),
            author: "Reviewer".into(),
            state,
            draft: false,
            source_branch: "topic".into(),
            target_branch: "main".into(),
            from_fork: false,
            created_at: 0,
            updated_at: 0,
            web_url: "https://github.com/example/project/pull/7".into(),
            labels: vec![],
        }
    }

    fn details(state: RequestState) -> RequestDetails {
        RequestDetails {
            summary: summary(state),
            body: String::new(),
            head_sha: Some("1234567".into()),
            mergeable: Some(true),
            commits: None,
            changed_files: None,
            additions: None,
            deletions: None,
            checks: vec![],
            reviews: vec![],
            comments: vec![],
        }
    }

    fn test_tab() -> RepoTab {
        let (worker, _, _) = Worker::detached();
        RepoTab::with_worker(
            "fixture".into(),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        )
    }

    /// Everything one frame of `f` writes: each text, whether it was cut short, and the
    /// AccessKit nodes of the frame.
    struct Frame {
        texts: Vec<(String, bool)>,
        /// Each AccessKit node's name and whether it is disabled.
        nodes: Vec<(String, bool)>,
    }

    impl Frame {
        fn has(&self, text: &str) -> bool {
            self.texts.iter().any(|(t, _)| t == text)
        }
        fn mentions(&self, part: &str) -> bool {
            self.texts.iter().any(|(t, _)| t.contains(part))
        }
        /// Whether the control named `part` is off.
        fn off(&self, part: &str) -> bool {
            self.nodes
                .iter()
                .find(|(name, _)| name.contains(part))
                .unwrap_or_else(|| panic!("no control named {part}: {:?}", self.nodes))
                .1
        }
    }

    /// Each text in `shape`, and whether it was cut short.
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, bool)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.galley.elided)),
            _ => {}
        }
    }

    fn render(theme_id: theme::ThemeId, size: egui::Vec2, mut f: impl FnMut(&mut Ui)) -> Frame {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme_id, theme::Appearance::STILL);
        ctx.enable_accesskit();
        let mut output = egui::Context::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| f(ui),
        );
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut texts);
        }
        let nodes = output
            .platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| Some((n.label()?.to_owned(), n.is_disabled())))
                    .collect()
            })
            .unwrap_or_default();
        Frame { texts, nodes }
    }

    const SCREEN: egui::Vec2 = vec2(900.0, 700.0);

    fn loaded_tab(state: RequestState) -> RepoTab {
        let mut tab = test_tab();
        tab.requests.selected = Some(7);
        tab.requests.list = Some(Ok(Arc::from([summary(state)])));
        tab.requests.details = Some(Ok(Arc::new(details(state))));
        tab
    }

    /// F3: a GitLab merge request is `!7` everywhere it is numbered; a GitHub pull request `#7`.
    #[test]
    fn requests_are_numbered_with_their_providers_sigil() {
        for (kind, sigil, other) in [
            (ForgeKind::GitLab, "!7", "#7"),
            (ForgeKind::GitHub, "#7", "!7"),
        ] {
            let mut tab = loaded_tab(RequestState::Open);
            let ground = Color32::BLACK;
            let list_frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
                list(ui, &mut tab, kind, ground, &mut Vec::new());
            });
            let detail_frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
                detail(ui, &mut tab, kind, &mut Vec::new());
            });
            for (name, frame) in [("list", &list_frame), ("detail", &detail_frame)] {
                assert!(frame.mentions(sigil), "{kind:?} {name} lacks {sigil}");
                assert!(!frame.mentions(other), "{kind:?} {name} shows {other}");
            }
        }
    }

    /// F5: while the details load there is nothing to approve or merge yet, and a draft is not
    /// approved at all.
    #[test]
    fn approving_waits_for_the_details_and_skips_drafts() {
        let mut tab = test_tab();
        tab.requests.selected = Some(7);
        let open = summary(RequestState::Open);
        assert!(approve_action(&tab, &open).is_none(), "no details yet");
        tab.requests.details = Some(Ok(Arc::new(details(RequestState::Open))));
        assert!(approve_action(&tab, &open).is_some());
        let mut draft = open;
        draft.draft = true;
        assert!(approve_action(&tab, &draft).is_none(), "drafts wait");
    }

    #[test]
    fn loading_details_leave_every_action_off_and_say_so() {
        let mut tab = test_tab();
        tab.requests.selected = Some(7);
        tab.requests.list = Some(Ok(Arc::from([summary(RequestState::Open)])));
        tab.requests.details_loading = true;
        let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            detail(ui, &mut tab, ForgeKind::GitHub, &mut Vec::new());
        });
        for name in ["Check out", "Approve", "Merge…"] {
            assert!(frame.off(name), "{name} is on while loading");
        }
        assert!(frame.mentions("Loading"), "{:?}", frame.texts);
    }

    /// F5: failed details show git's, or the host's, own message and a way to try again.
    #[test]
    fn failed_details_show_the_message_and_retry() {
        let mut tab = test_tab();
        tab.requests.selected = Some(7);
        tab.requests.list = Some(Ok(Arc::from([summary(RequestState::Open)])));
        tab.requests.details = Some(Err("connection timed out".into()));
        let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            detail(ui, &mut tab, ForgeKind::GitHub, &mut Vec::new());
        });
        assert!(frame.mentions("connection timed out"), "{:?}", frame.texts);
        assert!(frame.has("Retry"), "{:?}", frame.texts);
        assert!(frame.off("Approve"));
    }

    /// F6: when the list could not load, the right pane says why instead of "Select a ...".
    #[test]
    fn a_failed_list_is_explained_in_the_details_pane() {
        let mut tab = test_tab();
        tab.requests.list = Some(Err("Not Found".into()));
        tab.requests.token = Some(None);
        let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            detail(ui, &mut tab, ForgeKind::GitHub, &mut Vec::new());
        });
        assert!(
            !frame.mentions("Select a pull request"),
            "{:?}",
            frame.texts
        );
        assert!(frame.mentions("Not Found"), "{:?}", frame.texts);
        assert!(frame.has("Add a token…"), "{:?}", frame.texts);
    }

    /// F2: the merge method's full name fits its trigger.
    #[test]
    fn the_merge_method_is_not_cut_short() {
        let mut tab = test_tab();
        let details = details(RequestState::Open);
        // The panes the view really has: wide, and the narrowest the details pane gets at 1024.
        for (width, method) in [560.0, 360.0, 300.0]
            .into_iter()
            .flat_map(|width| MergeMethod::ALL.map(|method| (width, method)))
        {
            tab.requests.merge_method = method;
            let frame = render(theme::ThemeId::Midnight, vec2(width, 400.0), |ui| {
                action_bar(
                    ui,
                    &mut tab,
                    ForgeKind::GitHub,
                    &details.summary,
                    Some(&details),
                    &mut Vec::new(),
                );
            });
            let shown = frame
                .texts
                .iter()
                .find(|(t, _)| t == method.label())
                .unwrap_or_else(|| panic!("{} not shown: {:?}", method.label(), frame.texts));
            assert!(!shown.1, "{} is cut short at {width}", method.label());
        }
    }

    #[test]
    fn merge_controls_fit_a_narrow_details_pane() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = Worker::detached();
        let mut tab = RepoTab::with_worker(
            "fixture".into(),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let details = details(RequestState::Open);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(196.0, 700.0))),
                ..Default::default()
            },
            |ui| {
                action_bar(
                    ui,
                    &mut tab,
                    ForgeKind::GitHub,
                    &details.summary,
                    Some(&details),
                    &mut Vec::new(),
                );
            },
        );
        output.textures_delta.clear();
        let merge = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().contains("Merge…")
                {
                    Some((text.visual_bounding_rect(), shape.clip_rect))
                } else {
                    None
                }
            })
            .expect("merge control");
        assert!(
            merge.1.contains_rect(merge.0),
            "merge control {:?} outside {:?}",
            merge.0,
            merge.1
        );
    }

    #[test]
    fn refreshed_request_selection_preserves_action_eligibility() {
        let (worker, _, _) = Worker::detached();
        let mut tab = RepoTab::with_worker(
            "fixture".into(),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.requests.selected = Some(7);
        tab.requests.list = Some(Ok(Arc::from([summary(RequestState::Open)])));
        for state in [
            RequestState::Open,
            RequestState::Closed,
            RequestState::Merged,
        ] {
            tab.requests.details = Some(Ok(Arc::new(details(state))));
            let request = selected_summary(&tab).expect("selected request");
            assert_eq!(request.state, state);
            assert_eq!(
                approve_action(&tab, request).is_some(),
                state == RequestState::Open
            );
            for kind in [ForgeKind::GitHub, ForgeKind::GitLab] {
                let detail = details(state);
                assert_eq!(
                    merge_confirmation(&tab, kind, request, Some(&detail)).is_some(),
                    state == RequestState::Open
                );
            }
        }
        let mut other = details(RequestState::Closed);
        other.summary.number = 8;
        tab.requests.details = Some(Ok(Arc::new(other)));
        assert_eq!(
            selected_summary(&tab).expect("list fallback").state,
            RequestState::Open
        );
        tab.requests.details = Some(Err("offline".into()));
        assert_eq!(selected_summary(&tab).expect("error fallback").number, 7);
        tab.requests.selected = Some(99);
        assert!(selected_summary(&tab).is_none());
    }

    #[test]
    fn request_shortcuts_use_the_displayed_state_after_details_refresh() {
        let (worker, _, _) = Worker::detached();
        let mut tab = RepoTab::with_worker(
            "fixture".into(),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.requests.selected = Some(7);
        tab.requests.list = Some(Ok(Arc::from([summary(RequestState::Open)])));
        tab.requests.details = Some(Ok(Arc::new(details(RequestState::Merged))));
        let request = selected_summary(&tab).expect("selected request");
        assert!(approve_action(&tab, request).is_none());
        assert!(
            merge_confirmation(
                &tab,
                ForgeKind::GitHub,
                request,
                tab.requests
                    .details
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .ok()
                    .map(Arc::as_ref)
            )
            .is_none()
        );
    }

    fn check(state: CheckState) -> canopy_hosting::Check {
        canopy_hosting::Check {
            name: "build".into(),
            state,
            url: None,
        }
    }

    fn review(author: &str, state: &str) -> canopy_hosting::Review {
        canopy_hosting::Review {
            author: author.into(),
            state: state.into(),
        }
    }

    /// F11: the two branch names keep their whole width while the row has it, and share the
    /// room fairly when it does not.
    #[test]
    fn branch_names_share_the_room_fairly() {
        assert_eq!(fit_pair(100.0, 50.0, 200.0), (100.0, 50.0));
        assert_eq!(fit_pair(300.0, 50.0, 200.0), (150.0, 50.0));
        assert_eq!(fit_pair(50.0, 300.0, 200.0), (50.0, 150.0));
        assert_eq!(fit_pair(300.0, 300.0, 200.0), (100.0, 100.0));
        assert_eq!(fit_pair(300.0, 300.0, -4.0), (0.0, 0.0));
    }

    #[test]
    fn a_cut_branch_name_keeps_both_its_ends() {
        let name: Vec<char> = "feature/graph-lane-allocator".chars().collect();
        assert_eq!(elide_middle(&name, 10), "featu…cator");
        assert_eq!(elide_middle(&name, 9), "featu…ator");
        assert_eq!(elide_middle(&name, 0), "…");
    }

    /// F9: a row shows its labels and, once the request has been opened, its checks and review.
    #[test]
    fn a_row_carries_labels_checks_and_review() {
        let mut tab = loaded_tab(RequestState::Open);
        let mut open = details(RequestState::Open);
        open.summary.labels = vec!["bug".into(), "linux".into()];
        open.checks = vec![check(CheckState::Success), check(CheckState::Failure)];
        open.reviews = vec![review("maya", CHANGES_REQUESTED)];
        tab.requests.list = Some(Ok(Arc::from([open.summary.clone()])));
        let ctx_frame = |tab: &mut RepoTab, remember: bool| {
            let details = open.clone();
            render(theme::ThemeId::Midnight, SCREEN, |ui| {
                if remember {
                    remember_signals(ui.ctx(), tab, &details);
                }
                list(ui, tab, ForgeKind::GitHub, Color32::BLACK, &mut Vec::new());
            })
        };
        let unseen = ctx_frame(&mut tab, false);
        assert!(
            unseen.has("bug") && unseen.has("linux"),
            "{:?}",
            unseen.texts
        );
        assert!(
            !unseen.has(icon::X_CIRCLE),
            "checks shown before they are known"
        );
        // The memory lives in the context, so remember and draw in one frame.
        let seen = ctx_frame(&mut tab, true);
        let seen_again = ctx_frame(&mut tab, true);
        for frame in [&seen, &seen_again] {
            assert!(frame.has(icon::X_CIRCLE), "{:?}", frame.texts);
            assert!(frame.has(icon::NOTE_PENCIL), "{:?}", frame.texts);
        }
    }

    #[test]
    fn checks_and_reviews_become_row_signals() {
        let mut d = details(RequestState::Open);
        assert_eq!(
            Signals::of(&d),
            Signals {
                ci: None,
                review: None
            }
        );
        d.checks = vec![check(CheckState::Success), check(CheckState::Pending)];
        d.reviews = vec![review("a", "Commented"), review("b", "Approved")];
        assert_eq!(
            Signals::of(&d),
            Signals {
                ci: Some(Ci::Running {
                    pending: 1,
                    total: 2
                }),
                review: Some(Verdict::Approved)
            }
        );
        d.checks.push(check(CheckState::Failure));
        d.reviews.push(review("c", CHANGES_REQUESTED));
        assert_eq!(
            Signals::of(&d),
            Signals {
                ci: Some(Ci::Failing {
                    failed: 1,
                    total: 3
                }),
                review: Some(Verdict::ChangesRequested)
            }
        );
    }

    /// F6: with the list failed, the list pane stays short and leaves the way out to the details
    /// pane, so the two never repeat each other.
    #[test]
    fn a_failed_list_offers_its_retry_once() {
        let mut tab = test_tab();
        tab.requests.list = Some(Err("Bad Gateway".into()));
        let list_frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            list(
                ui,
                &mut tab,
                ForgeKind::GitHub,
                Color32::BLACK,
                &mut Vec::new(),
            );
        });
        let detail_frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            detail(ui, &mut tab, ForgeKind::GitHub, &mut Vec::new());
        });
        assert!(!list_frame.has("Retry"), "{:?}", list_frame.texts);
        assert!(detail_frame.has("Retry"), "{:?}", detail_frame.texts);
        assert!(detail_frame.mentions("Bad Gateway"));
    }

    /// F7: the first load shows placeholder rows, not a spinner and a sentence.
    #[test]
    fn the_first_load_shows_placeholder_rows() {
        let mut tab = test_tab();
        tab.requests.loading = true;
        let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            list(
                ui,
                &mut tab,
                ForgeKind::GitHub,
                Color32::BLACK,
                &mut Vec::new(),
            );
        });
        assert!(
            frame.nodes.iter().any(|(name, _)| name == "Loading"),
            "{:?}",
            frame.nodes
        );
        assert!(
            !frame.mentions("Loading pull requests"),
            "{:?}",
            frame.texts
        );
    }

    /// F1: a draft or a conflicted request cannot be merged, and says why where it can be read.
    #[test]
    fn a_blocked_merge_says_why_in_the_pane() {
        let mut tab = test_tab();
        let mut draft = details(RequestState::Open);
        draft.summary.draft = true;
        let mut conflicted = details(RequestState::Open);
        conflicted.mergeable = Some(false);
        for (details, said) in [
            (&draft, "mark it ready for review"),
            (&conflicted, "resolve them on its branch"),
        ] {
            tab.requests.details = Some(Ok(Arc::new(details.clone())));
            let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
                action_bar(
                    ui,
                    &mut tab,
                    ForgeKind::GitHub,
                    &details.summary,
                    Some(details),
                    &mut Vec::new(),
                );
            });
            assert!(frame.off("Merge…"));
            assert!(frame.mentions(said), "{said}: {:?}", frame.texts);
        }
        // A draft is not approved either.
        assert!(approve_action(&tab, &draft.summary).is_none());
    }

    /// F1: failed checks and a review asking for changes keep the merge possible but never
    /// make it the invitation: it is a warned secondary button, and the dialog says so.
    #[test]
    fn failing_checks_and_requested_changes_warn_before_a_merge() {
        let mut tab = test_tab();
        let mut risky = details(RequestState::Open);
        risky.checks = vec![check(CheckState::Success), check(CheckState::Failure)];
        risky.reviews = vec![review("maya", CHANGES_REQUESTED)];
        tab.requests.selected = Some(7);
        tab.requests.details = Some(Ok(Arc::new(risky.clone())));
        let risks = merge_risks(&risky);
        assert_eq!(risks[0], (Tone::Danger, "1 of 2 checks failed".to_owned()));
        assert_eq!(
            risks[1],
            (Tone::Danger, "Changes requested by maya".to_owned())
        );
        assert_eq!(risk_note(&risks).map(|n| n.tone), Some(Tone::Danger));
        let Some(Dialog::ConfirmHost {
            body,
            confirm,
            danger,
            ..
        }) = merge_confirmation(&tab, ForgeKind::GitHub, &risky.summary, Some(&risky))
        else {
            panic!("a risky merge can still be confirmed");
        };
        assert_eq!(confirm, "Merge anyway");
        assert!(danger);
        assert!(body.starts_with("Heads up: 1 of 2 checks failed"), "{body}");
        let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            action_bar(
                ui,
                &mut tab,
                ForgeKind::GitHub,
                &risky.summary,
                Some(&risky),
                &mut Vec::new(),
            );
        });
        assert!(!frame.off("Merge…"), "still possible");
        assert!(frame.mentions("1 of 2 checks failed"), "{:?}", frame.texts);
        // A clean request merges as before.
        let clean = details(RequestState::Open);
        tab.requests.details = Some(Ok(Arc::new(clean.clone())));
        assert_eq!(merge_risks(&clean), Vec::new());
        let Some(Dialog::ConfirmHost {
            confirm, danger, ..
        }) = merge_confirmation(&tab, ForgeKind::GitHub, &clean.summary, Some(&clean))
        else {
            panic!("a clean merge is confirmed");
        };
        assert_eq!(confirm, "Merge");
        assert!(!danger);
    }

    /// Deeper's seam between the cards follows the list panel's resize handle: egui registers
    /// it under `<panel id>/__resize`, centred on the panel's inner edge.
    #[test]
    fn the_list_handle_marks_the_edge_of_the_list_panel() {
        let ctx = egui::Context::default();
        let (mut edge, mut handle) = (None, None);
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let list = egui::Panel::left(LIST_PANEL)
                        .resizable(true)
                        .default_size(200.0)
                        .show(ui, |ui| ui.label("list"));
                    edge = Some(list.response.rect.right());
                    handle = ui
                        .ctx()
                        .read_response(list_handle())
                        .map(|r| r.rect.center().x);
                });
            });
            output.textures_delta.clear();
        }
        let (edge, handle) = (edge.unwrap_or(f32::NAN), handle.unwrap_or(f32::NAN));
        assert!(
            (edge - handle).abs() < 1.0,
            "panel edge {edge}, handle {handle}"
        );
    }
}
