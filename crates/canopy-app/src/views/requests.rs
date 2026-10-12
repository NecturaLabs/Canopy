//! Pull requests (GitHub) and merge requests (GitLab): a list on the left, the selected request
//! on the right with its checks, reviews, description and conversation, and the actions a
//! reviewer needs (check out, approve, comment, merge, close or reopen).

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Galley, Id, Layout, Margin, Painter, Rect,
    Response, RichText, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::RefKind;
use canopy_core::forge::ForgeKind;
use canopy_hosting::{
    CheckState, MergeMethod, RequestDetails, RequestState, RequestSummary, ReviewDecision, Rollup,
    StateFilter,
};

use crate::action::Action;
use crate::hosting::HostJob;
use crate::keymap::Command;
use crate::request_view::{
    self, ChecksChoice, DraftChoice, RequestSort, RequestView, ReviewChoice, Standing,
};
use crate::tab::RepoTab;
use crate::ui::art;
use crate::ui::kit::avatar::{Avatar, AvatarSize, Person};
use crate::ui::kit::button::{LinkButton, icon_button_side};
use crate::ui::kit::chip::{Badge, DiffStat, RefPill};
use crate::ui::kit::choice::Segmented;
use crate::ui::kit::field::{SearchField, TextArea};
use crate::ui::kit::overlay::{Popover, TooltipExt};
use crate::ui::kit::progress::{Skeleton, Spinner};
use crate::ui::kit::row::{self, Disclosure, ListRow, Roving, RowKind, SectionHeader};
use crate::ui::kit::select::{self, Select};
use crate::ui::kit::split::{self, SplitAxis, SplitHandle};
use crate::ui::kit::surface::{Callout, EmptyAction, EmptyState, InsetBox, NoticeAction};
use crate::ui::kit::{self, Button, IconButton, Size, Tone, a11y, paint};
use crate::ui::theme::{self, Style, palette};
use crate::ui::widgets::{self, forge_icon};
use crate::views::dialogs::Dialog;
use crate::views::state_banner;

/// Why an action waits.
const WAIT: &str = "Wait for the current operation";
/// Why an action waits for the details of the selected request.
const LOADING: &str = "Loading the request…";
/// Why an action is off when the details could not be loaded.
const DETAILS_FAILED: &str = "The request's details could not be loaded";
/// The widest a branch pill in the request header grows before it is cut.
const BRANCH_PILL_MAX: f32 = 240.0;
/// Shows the requests view. Returns the list panel's rectangle and the id of the handle between
/// it and the details when they sit side by side, for the content pane's two cards
/// (`app::PaneSplit::beside`).
pub fn show(
    ui: &mut Ui,
    tab: &mut RepoTab,
    api_enabled: bool,
    actions: &mut Vec<Action>,
) -> Option<(Rect, Id)> {
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
        return None;
    };
    let kind = forge.kind;
    if !api_enabled {
        empty(
            ui,
            art::local_only(),
            &format!("{}s are turned off", cap(kind.request_noun())),
            "Turn them on in Settings → Integrations.",
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
        return None;
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
    // The list takes a share of the pane that grows with the window, up to a limit; the user's own
    // width, once they set one, replaces it.
    let room = ui.available_width();
    let pane_max = (room * LIST_MAX_SHARE).max(LIST_FLOOR);
    let (lo, hi) = (pane_max.min(LIST_MIN), pane_max.min(LIST_MAX));
    let default_width = (room * LIST_SHARE)
        .clamp(LIST_DEFAULT_MIN, LIST_DEFAULT_MAX)
        .clamp(lo, hi);
    // The list's width: where its handle is being dragged to, else the user's saved one (the
    // panel's own state, kept across restarts), else the default.
    let panel_id = Id::new(LIST_PANEL);
    let handle_id = SplitHandle::id_in(ui, LIST_SPLIT);
    let saved = egui::containers::panel::PanelState::load(ui.ctx(), panel_id);
    let own_id = Id::new(LIST_OWN_WIDTH);
    // A width saved before this was recorded is the user's own, unless it is the old default,
    // which every list that was never resized carries; a first run is on the default.
    let own: bool = ui.ctx().data_mut(|d| {
        *d.get_persisted_mut_or_insert_with(own_id, || {
            saved.is_some_and(|s| (s.size().x - LIST_OLD_DEFAULT).abs() > 0.5)
        })
    });
    let stored = match saved {
        Some(state) if own => state.size().x,
        _ => default_width,
    };
    let mut width = split::dragged_size(ui.ctx(), handle_id, SplitAxis::Columns, false, lo..=hi)
        .unwrap_or(stored)
        .clamp(lo, hi);
    let list_panel = egui::Panel::left(panel_id)
        .resizable(false)
        .exact_size(width)
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
    // Shown over both panes, so their scrolling cannot take the drag from it.
    let rect = list_panel.response.rect;
    let handle = SplitHandle::new(LIST_SPLIT, &mut width, SplitAxis::Columns)
        .range(lo..=hi)
        .default_size(default_width)
        .label("Resize the request list")
        .seam(false)
        .show(ui, rect.right(), rect.y_range());
    if handle.changed()
        && let Some(mut state) = egui::containers::panel::PanelState::load(ui.ctx(), panel_id)
    {
        state.outer_rect.max.x = state.outer_rect.min.x + width;
        ui.ctx().data_mut(|data| {
            data.insert_persisted(panel_id, state);
            // Back at the default (a double click or Home), the list follows the window again.
            data.insert_persisted(own_id, (width - default_width).abs() > f32::EPSILON);
        });
        ui.ctx().request_repaint();
    }
    Some((rect, handle.id))
}

/// The id of the request list's panel, beside the details (its width is kept under it).
const LIST_PANEL: &str = "requests-list";
/// The salt of the handle between the request list and the details.
const LIST_SPLIT: &str = "requests-list";
/// Whether the list's saved width is one the user set (else it follows the window).
const LIST_OWN_WIDTH: &str = "requests-list-own-width";
/// The list's default share of the pane, and the narrowest and widest that default is.
const LIST_SHARE: f32 = 0.45;
const LIST_DEFAULT_MIN: f32 = 320.0;
const LIST_DEFAULT_MAX: f32 = 560.0;
/// The default width before the list took a share of the pane.
const LIST_OLD_DEFAULT: f32 = 380.0;
/// The most of the pane the list can be dragged to, its widest and its narrowest, and the least
/// room a small window leaves it.
const LIST_MAX_SHARE: f32 = 0.6;
const LIST_MAX: f32 = 800.0;
const LIST_MIN: f32 = 220.0;
const LIST_FLOOR: f32 = 160.0;

/// The checks' disclosure line, its glyph and its tone (V22): red when any failed, amber while
/// any run, green when all passed.
fn checks_summary(failed: usize, pending: usize, total: usize) -> (&'static str, Tone, String) {
    if failed > 0 {
        (
            icon::X_CIRCLE,
            Tone::Danger,
            format!("{failed} of {total} checks failed"),
        )
    } else if pending > 0 {
        (
            icon::CIRCLE_DASHED,
            Tone::Warning,
            format!("{pending} of {total} checks running"),
        )
    } else {
        (
            icon::CHECK_CIRCLE,
            Tone::Success,
            format!("All {total} checks passed"),
        )
    }
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
    Passing {
        total: usize,
    },
    Failing {
        failed: usize,
        total: usize,
    },
    Running {
        pending: usize,
        total: usize,
    },
    /// The provider's combined state from the list's rollup, which carries no counts: success,
    /// failure or pending (a neutral state says nothing and shows nothing).
    Overall(CheckState),
}

/// The standing of a request's reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Approved,
    ChangesRequested,
    Commented,
    /// The branch rules want a review nobody has given yet (GitHub, from the list's rollup).
    Required,
}

/// What a reviewer scans a list for: the checks and the reviews of a request. With a token the
/// list comes with them (the rollup, [`RequestSummary::rollup`]); otherwise a row has them once
/// the request's details have been loaded in this session (they stay until the app closes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signals {
    ci: Option<Ci>,
    review: Option<Verdict>,
}

impl Signals {
    fn of_rollup(r: Rollup) -> Self {
        Self {
            ci: r
                .checks
                .filter(|&s| s != CheckState::Neutral)
                .map(Ci::Overall),
            review: r.review.map(|d| match d {
                ReviewDecision::Approved => Verdict::Approved,
                ReviewDecision::ChangesRequested => Verdict::ChangesRequested,
                ReviewDecision::Required => Verdict::Required,
            }),
        }
    }

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

    /// Where the checks and the review stand, for the list's filters.
    fn standing(self) -> Standing {
        Standing {
            checks: match self.ci {
                Some(Ci::Passing { .. } | Ci::Overall(CheckState::Success)) => {
                    Some(ChecksChoice::Passed)
                }
                Some(Ci::Failing { .. } | Ci::Overall(CheckState::Failure)) => {
                    Some(ChecksChoice::Failed)
                }
                Some(Ci::Running { .. } | Ci::Overall(CheckState::Pending)) => {
                    Some(ChecksChoice::Running)
                }
                Some(Ci::Overall(CheckState::Neutral)) | None => None,
            },
            review: match self.review {
                Some(Verdict::Approved) => Some(ReviewChoice::Approved),
                Some(Verdict::ChangesRequested) => Some(ReviewChoice::ChangesRequested),
                Some(Verdict::Required) => Some(ReviewChoice::Required),
                Some(Verdict::Commented) | None => None,
            },
        }
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
            Some(Ci::Overall(CheckState::Success)) => {
                out.push((
                    icon::CHECK_CIRCLE,
                    Tone::Success,
                    "Checks passed".to_owned(),
                ));
            }
            Some(Ci::Overall(CheckState::Failure)) => {
                out.push((icon::X_CIRCLE, Tone::Danger, "Checks failed".to_owned()));
            }
            Some(Ci::Overall(CheckState::Pending)) => {
                out.push((
                    icon::CIRCLE_DASHED,
                    Tone::Warning,
                    "Checks running".to_owned(),
                ));
            }
            Some(Ci::Overall(CheckState::Neutral)) | None => {}
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
            Some(Verdict::Required) => out.push((
                icon::USER_CIRCLE_DASHED,
                Tone::Neutral,
                "Review required".to_owned(),
            )),
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

/// What row `r` shows: the rollup the list came with, part by part, and where it says nothing
/// (no token, or a part it does not know), what the request's details said when they were last
/// opened. The rollup is as fresh as the list, and the details can lack a part the token may not
/// read (check runs, for one), so the list's word stands.
fn row_signals(ctx: &egui::Context, tab: &RepoTab, r: &RequestSummary) -> Option<Signals> {
    let remembered = known_signals(ctx, tab, r.number);
    let Some(listed) = r.rollup.map(Signals::of_rollup) else {
        return remembered;
    };
    Some(Signals {
        ci: listed.ci.or_else(|| remembered.and_then(|s| s.ci)),
        review: listed.review.or_else(|| remembered.and_then(|s| s.review)),
    })
}

/// Where row `r`'s checks and review stand as the row shows them, for the filters and for the
/// request a list opens on.
pub(crate) fn row_standing(ctx: &egui::Context, tab: &RepoTab, r: &RequestSummary) -> Standing {
    row_signals(ctx, tab, r)
        .map(Signals::standing)
        .unwrap_or_default()
}

// ------------------------------------------------------------------------------------- the list

/// The id of the filter field, which the filter command focuses.
const FILTER_FIELD: &str = "request-filter";
/// The popover with the filters other than the text.
const FILTERS_POPUP: &str = "request-filters";
/// Its width.
const FILTERS_WIDTH: f32 = 280.0;
/// The filter field's narrowest, beside the two buttons.
const FILTER_FIELD_MIN: f32 = 96.0;
/// How many lines a row's title wraps to before it is cut.
const TITLE_ROWS: usize = 2;
/// How many lines of the description a row shows.
const DESCRIPTION_ROWS: usize = 2;
/// How much of the description a screen reader hears; the rest is in the details.
const DESCRIPTION_SPOKEN: usize = 200;

fn list(
    ui: &mut Ui,
    tab: &mut RepoTab,
    kind: ForgeKind,
    ground: Color32,
    actions: &mut Vec<Action>,
) {
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    if let Some(wait) = tab.run_search_if_due() {
        ui.ctx().request_repaint_after(wait);
    }
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
            // A search that found nothing keeps its field, so it can be changed.
            if let Some(Ok(items)) = &tab.requests.list.clone()
                && (!items.is_empty() || !tab.requests.query.is_empty())
            {
                ui.add_space(theme::XS);
                filter_bar(ui, tab, items, actions);
            } else {
                // No field to take the caret: the request must not wait for a later list's.
                tab.requests.focus_filter = false;
            }
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
        Some(Ok(items)) if items.is_empty() && !tab.requests.query.is_empty() => {
            let clicked = EmptyState::filtered("No requests match these filters")
                .compact(true)
                .id_salt("request-list-searched")
                .show(ui)
                .value;
            if clicked == Some(EmptyAction::Primary) {
                tab.request_view.clear_filters();
                tab.request_view_changed();
                remember_view(tab, actions);
            }
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
        Some(Ok(items)) => rows(ui, tab, kind, &items, ground, actions),
    }
}

/// The filter field, the filters' popover and the sort menu, and what the filters have left.
fn filter_bar(ui: &mut Ui, tab: &mut RepoTab, items: &[RequestSummary], actions: &mut Vec<Action>) {
    let m = theme::metrics();
    let p = palette(ui.ctx());
    let before = tab.request_view.clone();
    let mut view = before.clone();
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let buttons = 2.0 * (icon_button_side(Size::Sm) + spacing);
        let field = SearchField::new(&mut view.text)
            .placeholder(format!("Filter {}s…", tab_noun(tab)))
            .name(format!("Filter {}s", tab_noun(tab)))
            .shortcut(Command::Filter)
            .width((ui.available_width() - buttons).max(FILTER_FIELD_MIN))
            .id_salt(FILTER_FIELD)
            .show(ui);
        if tab.requests.focus_filter {
            field.request_focus();
            tab.requests.focus_filter = false;
        }
        let narrowing = view.menu_filters();
        let name = if narrowing == 0 {
            "Filter by draft, checks, review or author".to_owned()
        } else {
            format!("Filters: {narrowing} on")
        };
        let anchor = IconButton::new(icon::FUNNEL, name)
            .size(Size::Sm)
            .active(narrowing > 0)
            .id_salt("request-filters-button")
            .show(ui);
        // The open state is kept here, not in egui's one popup slot: a choice's own list opens
        // in that slot and would close the popover under it.
        let open_id = Id::new((FILTERS_POPUP, "open"));
        let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
        if anchor.clicked() {
            open = !open;
        }
        let authors = if open {
            request_view::authors(items)
        } else {
            Vec::new()
        };
        let list_was_open = egui::Popup::is_any_open(ui.ctx());
        let was_open = open;
        Popover::new(FILTERS_POPUP, &anchor)
            .width(FILTERS_WIDTH)
            .open(&mut open)
            .show(|ui| filter_choices(ui, &mut view, &authors));
        // A click on an option of a choice's list is outside the popover for egui, and so is the
        // Esc that closes the list: the list takes it, and the popover stays for the next filter.
        if was_open && !open && list_was_open {
            open = true;
        }
        ui.data_mut(|d| d.insert_temp(open_id, open));
        let sort = IconButton::new(
            icon::ARROWS_DOWN_UP,
            format!("Sort requests: {}", view.sort.label()),
        )
        .size(Size::Sm)
        .active(view.sort != RequestSort::default())
        .id_salt("request-sort-button")
        .show(ui);
        let _ = kit::menu::menu(&sort, |menu| {
            menu.heading("Sort by");
            for choice in RequestSort::ALL {
                if menu
                    .item("", choice.label())
                    .checked(view.sort == choice)
                    .show(menu.ui)
                    .clicked()
                {
                    view.sort = choice;
                }
            }
        });
    });
    let shown = tab.requests.shown.as_ref().map(Vec::len);
    // With nothing left the empty state below carries the same way out.
    if view.filters() > 0
        && let Some(shown) = shown.filter(|&n| n > 0)
    {
        ui.add_space(theme::XS);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if tab.requests.query.is_empty() {
                    format!("{shown} of {} shown", items.len())
                } else {
                    format!("{shown} of {} found", items.len())
                })
                .size(m.meta_size)
                .color(p.text_faint),
            );
            if LinkButton::new("Clear filters").show(ui).clicked() {
                view.clear_filters();
            }
        });
    }
    if view != before {
        tab.request_view = view;
        tab.request_view_changed();
        remember_view(tab, actions);
    }
}

/// "pull request" or "merge request", for the filter's words.
fn tab_noun(tab: &RepoTab) -> &'static str {
    tab.forge()
        .map_or("request", |(_, forge)| forge.kind.request_noun())
}

/// Queues the request view for the repository's saved settings.
fn remember_view(tab: &RepoTab, actions: &mut Vec<Action>) {
    if let Some(repo) = tab.sidebar_key() {
        actions.push(Action::RememberRequestView {
            repo: repo.to_path_buf(),
            view: tab.request_view.clone(),
        });
    }
}

/// The popover's choices: draft, checks, review and author, each from a list, and the way out.
fn filter_choices(ui: &mut Ui, view: &mut RequestView, authors: &[String]) {
    let m = theme::metrics();
    let p = palette(ui.ctx());
    ui.spacing_mut().item_spacing.y = theme::SM;
    ui.label(
        RichText::new("Filters")
            .font(FontId::new(m.body_size, theme::semibold()))
            .color(p.text),
    );
    let width = ui.available_width();
    let any = "Any";
    let drafts = [
        (None, any),
        (Some(DraftChoice::Drafts), DraftChoice::Drafts.label()),
        (Some(DraftChoice::Ready), DraftChoice::Ready.label()),
    ];
    Select::new(&mut view.draft, &drafts)
        .label("Draft")
        .size(Size::Md)
        .width(width)
        .id_salt("request-filter-draft")
        .show(ui);
    let checks = [
        (None, any),
        (Some(ChecksChoice::Passed), ChecksChoice::Passed.label()),
        (Some(ChecksChoice::Failed), ChecksChoice::Failed.label()),
        (Some(ChecksChoice::Running), ChecksChoice::Running.label()),
    ];
    Select::new(&mut view.checks, &checks)
        .label("Checks")
        .size(Size::Md)
        .width(width)
        .id_salt("request-filter-checks")
        .show(ui);
    let reviews = [
        (None, any),
        (Some(ReviewChoice::Approved), ReviewChoice::Approved.label()),
        (
            Some(ReviewChoice::ChangesRequested),
            ReviewChoice::ChangesRequested.label(),
        ),
        (Some(ReviewChoice::Required), ReviewChoice::Required.label()),
    ];
    Select::new(&mut view.review, &reviews)
        .label("Review")
        .size(Size::Md)
        .width(width)
        .id_salt("request-filter-review")
        .show(ui);
    // A chosen author who is no longer in the list (it was reloaded) stays choosable.
    let mut names = authors.to_vec();
    if let Some(chosen) = &view.author
        && !names.iter().any(|n| n.eq_ignore_ascii_case(chosen))
    {
        names.push(chosen.clone());
    }
    let people: Vec<(Option<String>, &str)> = std::iter::once((None, "Anyone"))
        .chain(names.iter().map(|n| (Some(n.clone()), n.as_str())))
        .collect();
    Select::new(&mut view.author, &people)
        .label("Author")
        .size(Size::Md)
        .width(width)
        .id_salt("request-filter-author")
        .show(ui);
    if view.menu_filters() > 0
        && Button::new("Clear filters")
            .ghost()
            .id_salt("request-filters-clear")
            .show(ui)
            .clicked()
    {
        let (text, sort) = (std::mem::take(&mut view.text), view.sort);
        view.clear_filters();
        view.text = text;
        view.sort = sort;
    }
}

/// The rows of the list: the requests that pass the view's filters, in its order, each as tall
/// as its text needs.
fn rows(
    ui: &mut Ui,
    tab: &mut RepoTab,
    kind: ForgeKind,
    items: &Arc<[RequestSummary]>,
    ground: Color32,
    actions: &mut Vec<Action>,
) {
    let m = theme::metrics();
    let ctx = ui.ctx().clone();
    let order = tab
        .local_request_view()
        .visible(items, |r| row_standing(&ctx, tab, r));
    let numbers: Vec<u64> = order.iter().map(|&i| items[i].number).collect();
    if tab.requests.shown.as_ref() != Some(&numbers) {
        tab.requests.shown = Some(numbers);
    }
    if order.is_empty() {
        let clicked = EmptyState::filtered("No requests match these filters")
            .compact(true)
            .id_salt("request-list-filtered")
            .show(ui)
            .value;
        if clicked == Some(EmptyAction::Primary) {
            tab.request_view.clear_filters();
            tab.request_view_changed();
            remember_view(tab, actions);
        }
        return;
    }
    let now = chrono::Utc::now().timestamp();
    let selected = tab.requests.selected;
    let roving = Roving::new(
        &ctx,
        ("request-rows", &tab.path),
        order.len(),
        selected.and_then(|n| order.iter().position(|&i| items[i].number == n)),
    );
    let scroll_to = scroll_target(&ctx, tab, selected);
    // The bar is always visible and keeps its own lane beside the rows (`solid_scroll`), so the
    // handle never covers a status glyph or a label chip, as in the notification center.
    let out = ui
        .scope(|ui| {
            ui.spacing_mut().scroll = theme::solid_scroll();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("requests-rows")
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin {
                            left: crate::ui::points_i8(m.sidebar_pad),
                            right: crate::ui::points_i8(m.sidebar_pad),
                            top: crate::ui::points_i8(theme::XS),
                            // The last row scrolls clear of the edge (design.md, Scroll edges).
                            bottom: crate::ui::points_i8(theme::SM),
                        })
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            a11y::group(ui, egui::accesskit::Role::List, "Requests");
                            let content = ui.available_width() - 2.0 * row::PAD;
                            let laid: Vec<RowLayout> = order
                                .iter()
                                .map(|&i| {
                                    let r = &items[i];
                                    let glyphs =
                                        row_signals(&ctx, tab, r).map_or(1, |s| s.glyphs().len());
                                    RowLayout::new(ui.painter(), r, kind, glyphs, content, now)
                                })
                                .collect();
                            let heights: Vec<f32> = laid.iter().map(|l| l.height).collect();
                            let top = ui.cursor().min.y;
                            if let Some(at) = scroll_to.filter(|&at| at < heights.len()) {
                                let (y, h) = (heights[..at].iter().sum::<f32>(), heights[at]);
                                let width = ui.available_width();
                                ui.scroll_to_rect(
                                    Rect::from_min_size(pos2(0.0, top + y), vec2(width, h)),
                                    None,
                                );
                            }
                            let span = visible_span(&heights, top, ui.clip_rect().y_range());
                            let width = ui.available_width();
                            if span.before > 0.0 {
                                ui.allocate_space(vec2(width, span.before));
                            }
                            for at in span.range.clone() {
                                let r = &items[order[at]];
                                let is_selected = selected == Some(r.number);
                                if is_selected {
                                    roving.follow(&ctx, at);
                                }
                                let signals = row_signals(&ctx, tab, r);
                                let response = row(
                                    ui,
                                    r,
                                    RowDraw {
                                        kind,
                                        layout: &laid[at],
                                        signals,
                                        selected: is_selected,
                                        ground,
                                        index: at,
                                        list: roving,
                                        now,
                                    },
                                );
                                if response.clicked() {
                                    tab.select_request(r.number);
                                }
                            }
                            if span.after > 0.0 {
                                ui.allocate_space(vec2(width, span.after));
                            }
                            if tab.requests.more {
                                load_more(ui, tab, items.len(), actions);
                            }
                        });
                })
        })
        .inner;
    paint::scroll_fade(ui, &out, ground);
    // An arrow on a focused row moves the selection; focus follows it next frame.
    if let Some((from, to)) = roving.moved(&ctx) {
        let delta = if to >= from {
            isize::try_from(to - from).unwrap_or(isize::MAX)
        } else {
            -isize::try_from(from - to).unwrap_or(isize::MAX)
        };
        tab.move_request(delta);
    }
}

/// The end of a list the host holds more of: how much of it is loaded, and the next page on
/// demand (the pages already loaded are not fetched again).
fn load_more(ui: &mut Ui, tab: &RepoTab, loaded: usize, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.add_space(theme::SM);
    // Level with the text of the rows above.
    egui::Frame::new()
        .inner_margin(Margin::symmetric(crate::ui::points_i8(row::PAD), 0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(if tab.requests.query.is_empty() {
                    format!("Showing the {loaded} most recently updated")
                } else {
                    format!("Showing {loaded} matches; the host holds more")
                })
                .size(m.meta_size)
                .color(p.text_faint),
            );
            ui.add_space(theme::XS);
            if Button::new("Load more")
                .id_salt("request-load-more")
                .size(Size::Sm)
                .loading(tab.requests.loading_more)
                .show(ui)
                .clicked()
            {
                actions.push(Action::LoadMoreRequests);
            }
        });
}

/// The row to scroll into view because the selection moved since the last frame (a key or a
/// reload, not a click on a row that is already in view); remembers the selection.
fn scroll_target(ctx: &egui::Context, tab: &RepoTab, selected: Option<u64>) -> Option<usize> {
    let id = Id::new(("request-scrolled-to", tab.path.as_path()));
    let last: Option<Option<u64>> = ctx.data(|d| d.get_temp(id));
    ctx.data_mut(|d| d.insert_temp(id, selected));
    let (Some(old), Some(new)) = (last, selected) else {
        return None;
    };
    if old == Some(new) {
        return None;
    }
    tab.requests
        .shown
        .as_ref()
        .and_then(|shown| shown.iter().position(|&n| n == new))
}

/// How many placeholder rows a loading list shows.
const LOADING_ROWS: usize = 7;

/// Which rows of a list of `heights` (no spacing, its top at `top`) intersect `y`, and the room
/// of the rows before and after them.
#[derive(Debug, PartialEq)]
struct Span {
    range: std::ops::Range<usize>,
    before: f32,
    after: f32,
}

fn visible_span(heights: &[f32], top: f32, y: egui::Rangef) -> Span {
    let (mut first, mut before) = (0, 0.0);
    while first < heights.len() && top + before + heights[first] <= y.min {
        before += heights[first];
        first += 1;
    }
    let (mut end, mut at) = (first, top + before);
    while end < heights.len() && at < y.max {
        at += heights[end];
        end += 1;
    }
    Span {
        range: first..end,
        before,
        after: heights[end..].iter().sum(),
    }
}

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

/// A request's text laid out for the width its row has: the number and title (up to
/// [`TITLE_ROWS`] lines), the author and age, and the description (up to [`DESCRIPTION_ROWS`]),
/// and the height they make.
struct RowLayout {
    title: Arc<Galley>,
    meta: Arc<Galley>,
    description: Option<Arc<Galley>>,
    /// The text's height, blocks and the gaps between them.
    text_height: f32,
    /// The row's height: the text's with its padding, never less than a row of the kind.
    height: f32,
}

impl RowLayout {
    /// `glyphs` is how many checks and review glyphs the row has at the right of its title;
    /// `content` the row's width inside its padding.
    fn new(
        painter: &Painter,
        r: &RequestSummary,
        kind: ForgeKind,
        glyphs: usize,
        content: f32,
        now: i64,
    ) -> Self {
        let p = palette(painter.ctx());
        let m = theme::metrics();
        let text_width = (content - m.icon_md - theme::SM).max(0.0);
        let signals = if glyphs == 0 {
            0.0
        } else {
            crate::ui::count_f32(glyphs) * (m.icon_md + theme::XS) + theme::XS
        };
        let title_width = (text_width - signals).max(0.0);
        let font = FontId::new(m.body_size, theme::medium());
        let mut job = LayoutJob::default();
        job.wrap.max_width = title_width;
        job.wrap.max_rows = TITLE_ROWS;
        for (text, color) in [
            (number_text(kind, r.number), p.text_faint),
            (format!(" {}", r.title), Color32::PLACEHOLDER),
        ] {
            job.append(
                &text,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                },
            );
        }
        let title = painter.layout_job(job);
        let small = FontId::proportional(m.small_size);
        let meta = line_galley(
            painter,
            format!(
                "{}  ·  {}",
                r.author,
                widgets::relative_time(r.updated_at, now)
            ),
            small.clone(),
            p.text_faint,
            text_width,
        );
        let description = (!r.description.is_empty()).then(|| {
            let mut job = LayoutJob::simple(r.description.clone(), small, p.text_dim, text_width);
            job.wrap.max_rows = DESCRIPTION_ROWS;
            painter.layout_job(job)
        });
        let text_height = title.size().y
            + theme::XS
            + meta.size().y
            + description.as_ref().map_or(0.0, |d| theme::XS + d.size().y);
        Self {
            title,
            meta,
            description,
            text_height,
            height: (text_height + 2.0 * theme::SM).max(RowKind::Request.height()),
        }
    }
}

/// What a row is drawn with besides the request.
#[derive(Clone, Copy)]
struct RowDraw<'a> {
    kind: ForgeKind,
    layout: &'a RowLayout,
    signals: Option<Signals>,
    selected: bool,
    ground: Color32,
    index: usize,
    list: Roving,
    now: i64,
}

/// A request in the list: state glyph, number and title, author and age with its labels at the
/// right, the start of its description, and at the right of the title the checks and reviews when
/// they are known.
fn row(ui: &mut Ui, r: &RequestSummary, d: RowDraw<'_>) -> Response {
    let RowDraw {
        kind,
        layout,
        signals,
        selected,
        ground,
        index,
        list,
        now,
    } = d;
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (glyph, tone, state) = state_style(r);
    let color = tone.color(p);
    let number = number_text(kind, r.number);
    // Nothing known (no rollup yet or at all, and never opened): said, rather than left to look
    // like a request without checks.
    let glyphs = signals.map_or_else(
        || {
            vec![(
                icon::QUESTION,
                Tone::Neutral,
                "Checks and reviews not loaded".to_owned(),
            )]
        },
        Signals::glyphs,
    );
    let when = widgets::relative_time(r.updated_at, now);
    let mut tooltip = format!(
        "{state} · {}\n{number} · {} · {when}\n{} → {}",
        r.title, r.author, r.source_branch, r.target_branch
    );
    let mut info = format!(
        "{state} request {number} {} by {}, {when}",
        r.title, r.author
    );
    for (_, _, said) in &glyphs {
        tooltip = format!("{tooltip}\n{said}");
        info = format!("{info}, {said}");
    }
    if !r.labels.is_empty() {
        tooltip = format!("{tooltip}\nLabels: {}", r.labels.join(", "));
    }
    if !r.description.is_empty() {
        let spoken: String = r.description.chars().take(DESCRIPTION_SPOKEN).collect();
        info = format!("{info}. {spoken}");
    }
    let response = ListRow::new(("request", r.number))
        .kind(RowKind::Request)
        .height(layout.height)
        .roving(list, index)
        .selected(selected)
        .ground(ground)
        .tooltip(tooltip)
        .show_with(ui, |rc, painter| {
            let c = rc.content;
            let x = c.left() + m.icon_md + theme::SM;
            let mut y = rc.rect.top() + (rc.rect.height() - layout.text_height) / 2.0;
            let first_line = layout
                .title
                .rows
                .first()
                .map_or(layout.title.size().y, |row| row.height());
            let title_y = y + first_line / 2.0;
            painter.text(
                pos2(c.left() + m.icon_md / 2.0, title_y),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(m.icon_md),
                color,
            );
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
            let title_height = layout.title.size().y;
            painter.galley(pos2(x, y), Arc::clone(&layout.title), rc.text);
            y += title_height + theme::XS;
            let meta_height = layout.meta.size().y;
            painter.galley(pos2(x, y), Arc::clone(&layout.meta), p.text_faint);
            // Labels take what the author and age leave.
            let small = FontId::proportional(m.small_size);
            let used = layout.meta.size().x + theme::MD;
            paint_labels(
                painter,
                &r.labels,
                &small,
                (c.right() - x - used).max(0.0),
                c.right(),
                y + meta_height / 2.0,
            );
            y += meta_height + theme::XS;
            if let Some(description) = &layout.description {
                painter.galley(pos2(x, y), Arc::clone(description), p.text_dim);
            }
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
                            .font(theme::heading(m.headline_size))
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
                                tab.request_details(number);
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
    let pill = RefPill::new(name, RefKind::LocalBranch);
    // The whole name on hover only when the pill cuts it.
    let pill = if pill.width(ui) > BRANCH_PILL_MAX {
        pill.tooltip(name)
    } else {
        pill
    };
    pill.max_width(BRANCH_PILL_MAX).show(ui);
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
    // One primary for the state: Check out while the request is open, else Open on the host.
    // The rest follow as one group of secondary buttons of the same size and spacing.
    let open = r.state == RequestState::Open;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::SM, theme::SM);
        let check_out = |ui: &mut Ui, actions: &mut Vec<Action>| {
            let button = Button::new("Check out")
                .icon(icon::DOWNLOAD_SIMPLE)
                .enabled(!checkout_off)
                .tooltip("Fetch the request's branch and check it out locally")
                .disabled_reason(checkout_reason);
            let button = if open {
                button.primary()
            } else {
                button.secondary()
            };
            if button.show(ui).clicked() {
                actions.push(Action::CheckoutRequest(r.clone()));
            }
        };
        let open_on_host = |ui: &mut Ui, actions: &mut Vec<Action>| {
            let button = Button::new(format!("Open on {}", kind.name()))
                .icon(forge_icon(kind))
                .shortcut(Command::OpenOnForge)
                .tooltip(format!(
                    "Open this {} on {}",
                    kind.request_noun(),
                    kind.name()
                ));
            let button = if open {
                button.secondary()
            } else {
                button.primary()
            };
            if button.show(ui).clicked() {
                actions.push(Action::OpenUrl(r.web_url.clone()));
            }
        };
        if open {
            check_out(ui, actions);
            open_on_host(ui, actions);
        } else {
            open_on_host(ui, actions);
            check_out(ui, actions);
        }
        if Button::new("Copy link")
            .icon(icon::LINK)
            .tooltip(format!("Copy the address of this {}", kind.request_noun()))
            .show(ui)
            .clicked()
        {
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
                        select::fit_width(ui, MergeMethod::ALL.into_iter().map(MergeMethod::label));
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
    if let Some(problem) = credential_problem(tab) {
        ui.add_space(theme::SM);
        let fix = Callout::new(Tone::Warning, problem.text)
            .title(problem.title)
            .action(NoticeAction::new(problem.fix).icon(icon::KEY))
            .id_salt("credential-problem")
            .show_actions(ui)
            .value;
        if fix.is_some() {
            actions.push(Action::Settings);
        }
    }
}

/// A credential that cannot do what the request's actions need, and what fixes it.
struct CredentialProblem {
    title: &'static str,
    text: String,
    fix: &'static str,
}

/// The credential problem to show under the request's actions: none while the credential works
/// (its source is Settings' to say), else no token or a token the host refused. Without a token
/// nothing was refused, whatever the host answered (a GitLab that wants a sign-in answers 401).
fn credential_problem(tab: &RepoTab) -> Option<CredentialProblem> {
    if tab.requests.token == Some(None) {
        const NO_TOKEN: &str = "Without one Canopy can only read this request. Approving, merging and commenting need a token.";
        // A CLI that is installed but cannot be used says why, before anything else.
        let text = tab
            .requests
            .cli
            .as_ref()
            .and_then(|cli| cli.problem.as_deref())
            .map_or_else(|| NO_TOKEN.to_owned(), |why| format!("{why} {NO_TOKEN}"));
        Some(CredentialProblem {
            title: "No token",
            text,
            fix: "Add a token…",
        })
    } else if tab.requests.token_rejected {
        Some(CredentialProblem {
            title: "The host refused the token",
            // Which one, so the fix goes to the right place.
            text: match tab.requests.token.flatten() {
                Some(source @ crate::hosting::TokenSource::Cli(kind)) => format!(
                    "The token {} is expired, revoked or lacks the scope this needs. Sign in to the {} CLI again, or add a token, to approve, merge or comment.",
                    source.label(),
                    kind.name(),
                ),
                source => format!(
                    "The token {} is expired, revoked or lacks the scope this needs. Replace it to approve, merge or comment.",
                    source.map_or("Canopy sends", |source| source.label()),
                ),
            },
            fix: "Replace the token…",
        })
    } else {
        None
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
        format!("{}. ", said.join("; "))
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

/// One part of the summary line under a request's header.
#[derive(Debug, PartialEq, Eq)]
enum Stat {
    Text(String),
    /// Lines added and removed, drawn as the app's other diff stats.
    Lines(usize, usize),
}

/// The summary line's parts: commits, files, lines and the conflict state, as far as known.
fn summary_stats(d: &RequestDetails) -> Vec<Stat> {
    let plural = |n: u32| if n == 1 { "" } else { "s" };
    let mut stats = Vec::new();
    if let Some(c) = d.commits {
        stats.push(Stat::Text(format!("{c} commit{}", plural(c))));
    }
    if let Some(f) = d.changed_files {
        stats.push(Stat::Text(format!("{f} file{} changed", plural(f))));
    }
    if let (Some(a), Some(r)) = (d.additions, d.deletions) {
        let count = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
        stats.push(Stat::Lines(count(a), count(r)));
    }
    if let Some(m) = d.mergeable {
        stats.push(Stat::Text(
            if m { "no conflicts" } else { "has conflicts" }.into(),
        ));
    }
    stats
}

fn body(ui: &mut Ui, d: &RequestDetails, now: i64, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    // Stats: the counts in the dim text, the lines in the added and removed colours.
    let stats = summary_stats(d);
    if !stats.is_empty() {
        ui.scope(|ui| {
            // As tall as its text, like the commit details' summary.
            ui.spacing_mut().interact_size.y = 0.0;
            ui.horizontal_wrapped(|ui| {
                for (i, stat) in stats.iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("·").size(m.meta_size).color(p.text_faint));
                    }
                    match stat {
                        Stat::Text(text) => {
                            ui.label(RichText::new(text).size(m.meta_size).color(p.text_dim));
                        }
                        Stat::Lines(added, removed) => {
                            DiffStat::new(*added, *removed).show(ui);
                        }
                    }
                }
            });
        });
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
        let (glyph, tone, text) = checks_summary(failed, pending, d.checks.len());
        // Open at first when something failed; then as the reviewer left it.
        let open_id = ui.id().with(("checks-open", d.summary.number));
        let mut open = ui.data(|data| data.get_temp(open_id)).unwrap_or(failed > 0);
        Disclosure::new("checks", text, &mut open)
            .icon(glyph)
            .icon_tone(tone)
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
                                if LinkButton::new(&c.name).url(u).show(ui).clicked() {
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
                .extra(true)
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
            .disabled_reason(if empty { "Write a comment first" } else { WAIT })
            .show(ui)
            .clicked();
        if (clicked || submitted) && can {
            actions.push(Action::Host(HostJob::Comment(
                number,
                tab.requests.comment.trim().to_owned(),
            )));
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
            description: String::new(),
            rollup: None,
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

    /// #143: the summary line reads "8 commits · 8 files changed · +607 −98", with the lines in
    /// the theme's added and removed colours and the rest in the dim text.
    #[test]
    fn the_summary_line_colours_the_lines_like_other_diff_stats() {
        let mut d = details(RequestState::Open);
        d.commits = Some(8);
        d.changed_files = Some(1);
        d.additions = Some(607);
        d.deletions = Some(98);
        assert_eq!(
            summary_stats(&d),
            [
                Stat::Text("8 commits".into()),
                Stat::Text("1 file changed".into()),
                Stat::Lines(607, 98),
                Stat::Text("no conflicts".into()),
            ]
        );
        for theme_id in theme::ThemeId::ALL {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme_id, theme::Appearance::STILL);
            let mut p = None;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
                    ..Default::default()
                },
                |ui| {
                    p = Some(palette(ui.ctx()));
                    body(ui, &d, 0, &mut Vec::new());
                },
            );
            out.textures_delta.clear();
            let p = p.expect("palette");
            // Each painted text with the colour of each of its sections.
            let mut inks = Vec::new();
            for c in &out.shapes {
                colours_of(&c.shape, &mut inks);
            }
            let colours = |text: &str| {
                let (_, colours) = inks
                    .iter()
                    .find(|(t, _)| t == text)
                    .unwrap_or_else(|| panic!("{text} not painted: {inks:?}"));
                colours.clone()
            };
            assert_eq!(
                colours("+607\u{2212}98"),
                [p.success, p.danger.gamma_multiply(0.85)],
                "{theme_id:?}"
            );
            assert_eq!(colours("8 commits"), [p.text_dim], "{theme_id:?}");
            assert_eq!(colours("1 file changed"), [p.text_dim], "{theme_id:?}");
        }
    }

    /// Each text in `shape` with the colour of each of its sections.
    fn colours_of(shape: &egui::Shape, out: &mut Vec<(String, Vec<Color32>)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| colours_of(s, out)),
            egui::Shape::Text(t) => out.push((
                t.galley.text().to_owned(),
                t.galley
                    .job
                    .sections
                    .iter()
                    .map(|s| s.format.color)
                    .collect(),
            )),
            _ => {}
        }
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

    /// The shapes one frame of `f` paints on a pane of `size`.
    fn painted(
        theme_id: theme::ThemeId,
        size: egui::Vec2,
        mut f: impl FnMut(&mut Ui),
    ) -> Vec<egui::epaint::ClippedShape> {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme_id, theme::Appearance::STILL);
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out.shapes
    }

    /// The colour `text` is painted in, and the left edge it starts at.
    fn ink(shapes: &[egui::epaint::ClippedShape], text: &str) -> (Color32, f32) {
        fn find(shape: &egui::Shape, text: &str) -> Option<(Color32, f32)> {
            match shape {
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                egui::Shape::Text(t) if t.galley.text() == text => Some((
                    t.override_text_color.unwrap_or(t.fallback_color),
                    t.visual_bounding_rect().left(),
                )),
                _ => None,
            }
        }
        shapes
            .iter()
            .find_map(|c| find(&c.shape, text))
            .unwrap_or_else(|| panic!("{text:?} not painted"))
    }

    fn action_row(
        theme_id: theme::ThemeId,
        kind: ForgeKind,
        state: RequestState,
        width: f32,
        tab: &mut RepoTab,
    ) -> Vec<egui::epaint::ClippedShape> {
        let details = details(state);
        painted(theme_id, vec2(width, 400.0), |ui| {
            action_bar(
                ui,
                tab,
                kind,
                &details.summary,
                Some(&details),
                &mut Vec::new(),
            );
        })
    }

    /// #142: the header actions are one group of kit buttons with a single primary for the
    /// state: Check out while the request is open, Open on the host otherwise.
    #[test]
    fn the_action_row_has_one_primary_for_the_state() {
        for theme_id in theme::ThemeId::ALL {
            let mut tab = test_tab();
            let open = action_row(
                theme_id,
                ForgeKind::GitHub,
                RequestState::Open,
                560.0,
                &mut tab,
            );
            let merged = action_row(
                theme_id,
                ForgeKind::GitHub,
                RequestState::Merged,
                560.0,
                &mut tab,
            );
            let (check_out_open, _) = ink(&open, "Check out");
            let (host_open, _) = ink(&open, "Open on GitHub");
            let (check_out_merged, _) = ink(&merged, "Check out");
            let (host_merged, _) = ink(&merged, "Open on GitHub");
            let (copy, _) = ink(&open, "Copy link");
            let (more, _) = ink(&open, "More");
            assert_ne!(
                check_out_open, host_open,
                "{theme_id:?}: a primary and a secondary"
            );
            assert_eq!(check_out_open, host_merged, "{theme_id:?}: the primary");
            assert_eq!(host_open, check_out_merged, "{theme_id:?}: the secondary");
            assert_eq!(
                [host_open, copy, more],
                [host_open; 3],
                "{theme_id:?}: one group"
            );
            // Left to right: the primary first.
            assert!(ink(&open, "Check out").1 < ink(&open, "Open on GitHub").1);
            assert!(ink(&merged, "Open on GitHub").1 < ink(&merged, "Check out").1);
            assert!(ink(&open, "Open on GitHub").1 < ink(&open, "Copy link").1);
            assert!(ink(&open, "Copy link").1 < ink(&open, "More").1);
        }
    }

    /// #142: nothing says where the token came from while the credential works.
    #[test]
    fn no_credential_line_while_the_token_works() {
        for source in [
            crate::hosting::TokenSource::Cli(ForgeKind::GitHub),
            crate::hosting::TokenSource::Cli(ForgeKind::GitLab),
            crate::hosting::TokenSource::Saved,
            crate::hosting::TokenSource::Environment,
            crate::hosting::TokenSource::GitCredentials,
        ] {
            let mut tab = test_tab();
            tab.requests.token = Some(Some(source));
            let frame = render(theme::ThemeId::Midnight, vec2(560.0, 400.0), |ui| {
                let details = details(RequestState::Open);
                action_bar(
                    ui,
                    &mut tab,
                    ForgeKind::GitHub,
                    &details.summary,
                    Some(&details),
                    &mut Vec::new(),
                );
            });
            for word in ["token", "credential", "Using"] {
                assert!(
                    !frame.mentions(word),
                    "{source:?} shows {word}: {:?}",
                    frame.texts
                );
            }
        }
    }

    /// #136: a CLI that is installed but cannot be used says why where the missing token is
    /// reported; a CLI that is not installed adds nothing.
    #[test]
    fn a_cli_that_could_not_be_used_is_named_in_the_no_token_warning() {
        let mut tab = test_tab();
        tab.requests.token = Some(None);
        let plain = credential_problem(&tab).unwrap().text;
        tab.requests.cli = Some(crate::hosting::CliReport {
            user: None,
            problem: Some("The GitHub CLI (gh) is not signed in to github.com.".into()),
        });
        let text = credential_problem(&tab).unwrap().text;
        assert!(
            text.starts_with("The GitHub CLI (gh) is not signed in"),
            "{text}"
        );
        assert!(text.ends_with(&plain), "{text}");
    }

    /// #142: a missing or refused token shows as a warning with its fix, and the fix opens
    /// Settings.
    #[test]
    fn a_missing_or_refused_token_is_shown_with_its_fix() {
        for (rejected, token, fix) in [
            (false, Some(None), "Add a token…"),
            // A host that wants a sign-in refuses a tokenless read; there is nothing to replace.
            (true, Some(None), "Add a token…"),
            (
                true,
                Some(Some(crate::hosting::TokenSource::Saved)),
                "Replace the token…",
            ),
        ] {
            let mut tab = test_tab();
            tab.requests.token = token;
            tab.requests.token_rejected = rejected;
            let details = details(RequestState::Open);
            let mut actions = Vec::new();
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
            ctx.enable_accesskit();
            let mut run = |events: Vec<egui::Event>, actions: &mut Vec<Action>| {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(560.0, 700.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        action_bar(
                            ui,
                            &mut tab,
                            ForgeKind::GitHub,
                            &details.summary,
                            Some(&details),
                            actions,
                        );
                    },
                );
                out.textures_delta.clear();
                out.platform_output.accesskit_update
            };
            let _ = run(Vec::new(), &mut actions);
            let nodes = run(Vec::new(), &mut actions).expect("accesskit").nodes;
            let button = nodes
                .iter()
                .find(|(_, n)| n.label() == Some(fix))
                .unwrap_or_else(|| panic!("no {fix:?} button"));
            let b = button.1.bounds().expect("bounds");
            let at = pos2(
                crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
                crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
            );
            let mut events = vec![egui::Event::PointerMoved(at)];
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let _ = run(events, &mut actions);
            assert!(
                actions.iter().any(|a| matches!(a, Action::Settings)),
                "{fix}: {actions:?}"
            );
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

    #[test]
    fn a_rollup_becomes_row_signals() {
        let signals = |checks, review| Signals::of_rollup(Rollup { checks, review });
        assert_eq!(
            signals(Some(CheckState::Failure), Some(ReviewDecision::Required)),
            Signals {
                ci: Some(Ci::Overall(CheckState::Failure)),
                review: Some(Verdict::Required)
            }
        );
        // A neutral state (a skipped or manual pipeline) claims nothing.
        assert_eq!(
            signals(Some(CheckState::Neutral), None),
            Signals {
                ci: None,
                review: None
            }
        );
        let said = |s: Signals| -> Vec<String> { s.glyphs().into_iter().map(|g| g.2).collect() };
        assert_eq!(
            said(signals(
                Some(CheckState::Success),
                Some(ReviewDecision::ChangesRequested)
            )),
            ["Checks passed", CHANGES_REQUESTED]
        );
        assert_eq!(
            said(signals(
                Some(CheckState::Pending),
                Some(ReviewDecision::Approved)
            )),
            ["Checks running", "Approved"]
        );
        assert_eq!(
            said(signals(None, Some(ReviewDecision::Required))),
            ["Review required"]
        );
    }

    /// Rollup (requests F2): with a token every row shows its checks and review straight from
    /// the list, opened or not; where the list says nothing, what the details said fills in.
    #[test]
    fn rows_show_the_lists_rollup_and_details_fill_its_gaps() {
        let mut tab = loaded_tab(RequestState::Open);
        let mut open = details(RequestState::Open);
        open.checks = vec![check(CheckState::Success)];
        open.reviews = vec![review("maya", "Approved")];
        let mut seven = open.summary.clone();
        seven.rollup = Some(Rollup {
            checks: Some(CheckState::Failure),
            review: None,
        });
        let mut eight = summary(RequestState::Open);
        eight.number = 8;
        eight.title = "Second".into();
        eight.rollup = Some(Rollup {
            checks: Some(CheckState::Pending),
            review: Some(ReviewDecision::Required),
        });
        let mut draw = |rows: Vec<RequestSummary>| {
            tab.requests.list = Some(Ok(Arc::from(rows)));
            render(theme::ThemeId::Midnight, SCREEN, |ui| {
                remember_signals(ui.ctx(), &tab, &open);
                list(
                    ui,
                    &mut tab,
                    ForgeKind::GitHub,
                    Color32::BLACK,
                    &mut Vec::new(),
                );
            })
        };
        let frame = draw(vec![seven, eight]);
        // #8 was never opened: its glyphs come from the list.
        assert!(frame.has(icon::CIRCLE_DASHED), "{:?}", frame.texts);
        assert!(frame.has(icon::USER_CIRCLE_DASHED), "{:?}", frame.texts);
        // #7: the list's failed checks stand over older details; its review comes from them.
        assert!(frame.has(icon::X_CIRCLE), "{:?}", frame.texts);
        assert!(!frame.has(icon::CHECK_CIRCLE), "{:?}", frame.texts);
        assert!(frame.has(icon::THUMBS_UP), "{:?}", frame.texts);
        // Without a rollup (no token) the row shows what its details said, as before.
        let frame = draw(vec![open.summary.clone()]);
        assert!(frame.has(icon::CHECK_CIRCLE), "{:?}", frame.texts);
        assert!(frame.has(icon::THUMBS_UP), "{:?}", frame.texts);
        assert!(!frame.has(icon::X_CIRCLE), "{:?}", frame.texts);
        // A request nothing is known about (no rollup, never opened) says so instead of showing
        // what a request without checks shows.
        let mut nine = summary(RequestState::Open);
        nine.number = 9;
        let frame = draw(vec![nine]);
        assert!(frame.has(icon::QUESTION), "{:?}", frame.texts);
        let mut ten = summary(RequestState::Open);
        ten.number = 10;
        ten.rollup = Some(Rollup {
            checks: None,
            review: None,
        });
        let frame = draw(vec![ten]);
        assert!(
            !frame.has(icon::QUESTION),
            "a rollup of nothing is known: {:?}",
            frame.texts
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
        assert!(body.starts_with("1 of 2 checks failed"), "{body}");
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

    /// KPL-F6 (V22): the checks line's icon is red, amber or green by the checks' state.
    #[test]
    fn the_checks_line_is_toned_by_their_state() {
        let tone = |failed, pending| checks_summary(failed, pending, 4).1;
        assert_eq!(tone(1, 2), Tone::Danger);
        assert_eq!(tone(0, 2), Tone::Warning);
        assert_eq!(tone(0, 0), Tone::Success);
        assert_eq!(checks_summary(0, 0, 4).2, "All 4 checks passed");
    }

    /// polish F1: the request list is resized by the kit's split handle, like every other pane
    /// split: it sits on the list's edge (where Deeper's cards meet), keeps a saved width, and
    /// moves with the arrow keys.
    #[test]
    fn the_list_split_handle_keeps_a_saved_width_and_resizes_from_the_keyboard() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut tab = test_tab();
        let forge = canopy_core::forge::Forge::parse("https://github.com/acme/widgets.git", &[])
            .expect("a GitHub remote");
        tab.forges = vec![("origin".to_owned(), forge)];
        let panel = Id::new(LIST_PANEL);
        // Saved by an earlier run: 300 points wide.
        ctx.data_mut(|d| {
            d.insert_persisted(
                panel,
                egui::containers::panel::PanelState {
                    outer_rect: Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 600.0)),
                },
            );
        });
        let mut t = 0.0;
        let mut frame = |events: Vec<egui::Event>, tab: &mut RepoTab| {
            t += 0.05;
            let mut shown = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 700.0))),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    shown = show(ui, tab, true, &mut Vec::new())
                        .map(|(list, handle)| (list, ui.ctx().read_response(handle)));
                },
            );
            output.textures_delta.clear();
            shown.expect("the list is shown")
        };
        let (list, handle) = frame(Vec::new(), &mut tab);
        assert!((list.width() - 300.0).abs() < 1.0, "{list:?}");
        let handle = handle.expect("the handle is shown");
        assert!((handle.rect.center().x - list.right()).abs() < 1.0);

        ctx.memory_mut(|m| m.request_focus(handle.id));
        let right = vec![egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        frame(right, &mut tab);
        let (list, _) = frame(Vec::new(), &mut tab);
        let wanted = 300.0 + split::KEY_STEP;
        assert!((list.width() - wanted).abs() < 1.0, "{list:?}");
        let saved = egui::containers::panel::PanelState::load(&ctx, panel).expect("saved");
        assert!((saved.size().x - wanted).abs() < 1.0, "{saved:?}");
    }

    // ---------------------------------------------------------------------- number, preview, view

    fn request(number: u64, title: &str, author: &str, description: &str) -> RequestSummary {
        RequestSummary {
            number,
            title: title.into(),
            author: author.into(),
            source_branch: format!("topic-{number}"),
            target_branch: "release-branch".into(),
            updated_at: 1_000 - i64::try_from(number).unwrap_or(0),
            created_at: i64::try_from(number).unwrap_or(0),
            description: description.into(),
            ..summary(RequestState::Open)
        }
    }

    fn tab_with(rows: Vec<RequestSummary>) -> RepoTab {
        let mut tab = test_tab();
        tab.requests.list = Some(Ok(rows.into()));
        tab
    }

    #[test]
    fn a_row_leads_with_its_number_shows_a_preview_and_no_branches() {
        let mut tab = tab_with(vec![
            request(
                7,
                "Add keyboard help",
                "ann",
                "Explains the single-key shortcuts.",
            ),
            request(8, "Tidy settings", "bob", ""),
        ]);
        let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
            list(
                ui,
                &mut tab,
                ForgeKind::GitHub,
                Color32::BLACK,
                &mut Vec::new(),
            );
        });
        assert!(frame.has("#7 Add keyboard help"), "{:?}", frame.texts);
        assert!(frame.has("#8 Tidy settings"), "{:?}", frame.texts);
        assert!(
            frame.has("Explains the single-key shortcuts."),
            "{:?}",
            frame.texts
        );
        assert!(
            frame.mentions("ann  ·  "),
            "author and age on the second line"
        );
        assert!(
            !frame.mentions("topic-7") && !frame.mentions("release-branch") && !frame.mentions("→"),
            "the branch line is gone from the row: {:?}",
            frame.texts
        );
        let described = frame
            .nodes
            .iter()
            .find(|(name, _)| name.starts_with("Open request #7 Add keyboard help by ann"))
            .unwrap_or_else(|| panic!("no row named for #7: {:?}", frame.nodes));
        assert!(described.0.contains("Explains the single-key shortcuts."));
        let plain = frame
            .nodes
            .iter()
            .find(|(name, _)| name.starts_with("Open request #8"))
            .expect("a row for #8");
        assert!(
            !plain.0.contains(". "),
            "no description, nothing extra: {}",
            plain.0
        );
    }

    #[test]
    fn a_long_title_wraps_to_two_lines_before_it_is_cut_and_a_description_adds_height() {
        let ctx = kit::testkit::ctx();
        let kinds = ForgeKind::GitHub;
        let long_title =
            "Stream large diffs through the virtual list without materialising every hunk";
        let very_long = "word ".repeat(80);
        let measure = |title: &str, description: &str, width: f32| {
            let mut out = None;
            let r = request(12, title, "ann", description);
            kit::testkit::frame(&ctx, Vec::new(), |ui| {
                out = Some(RowLayout::new(ui.painter(), &r, kinds, 2, width, 0));
            });
            out.expect("laid out")
        };
        let base = RowKind::Request.height();
        let short = measure("Fix it", "", 480.0);
        assert_eq!(short.title.rows.len(), 1);
        assert!(!short.title.elided);
        assert!(
            (short.height - base).abs() < f32::EPSILON,
            "a plain row keeps its height"
        );
        assert!(short.description.is_none());

        let wide = measure(long_title, "", 700.0);
        assert_eq!(
            wide.title.rows.len(),
            1,
            "it fits on one line when there is room"
        );
        let narrow = measure(long_title, "", 400.0);
        assert_eq!(narrow.title.rows.len(), 2, "else it takes a second line");
        assert!(!narrow.title.elided, "and is not cut while it fits in two");
        assert!(narrow.height > wide.height);
        let cut = measure(&very_long, "", 400.0);
        assert_eq!(cut.title.rows.len(), 2);
        assert!(
            cut.title.elided,
            "only a title that fills both lines is cut"
        );

        let described = measure("Fix it", &"Explains why. ".repeat(40), 480.0);
        let description = described.description.as_ref().expect("a description");
        assert_eq!(description.rows.len(), 2, "at most two lines of it");
        assert!(description.elided, "with an ellipsis at the end");
        assert!(described.height > base, "the row grows to hold it");
    }

    #[test]
    fn the_visible_rows_follow_the_heights_they_have() {
        let heights = [64.0, 100.0, 64.0, 64.0, 100.0];
        let span = |top, min, max| visible_span(&heights, top, egui::Rangef::new(min, max));
        let all = span(0.0, 0.0, 1_000.0);
        assert_eq!((all.range.clone(), all.before, all.after), (0..5, 0.0, 0.0));
        // Rows 1 and 2 are in a window of 70..200; the first is above, the rest below.
        let middle = span(0.0, 70.0, 200.0);
        assert_eq!(middle.range, 1..3);
        assert_eq!((middle.before, middle.after), (64.0, 164.0));
        // The list's top is above the window by what has scrolled.
        let scrolled = span(-300.0, 0.0, 100.0);
        assert_eq!(
            scrolled.range.start, 4,
            "only the last row is still in view"
        );
        assert!(span(0.0, 0.0, 0.0).range.is_empty());
        assert!(
            visible_span(&[], 0.0, egui::Rangef::new(0.0, 100.0))
                .range
                .is_empty()
        );
    }

    #[test]
    fn the_list_shows_what_the_view_keeps_in_the_order_it_asks_for() {
        let mut rows = vec![
            request(
                31,
                "Stream large diffs",
                "lucia",
                "Large diffs no longer allocate.",
            ),
            request(12, "Add keyboard help", "maya", ""),
            request(27, "Fix the graph lanes", "sam", "Closes #12"),
        ];
        // The host lists them most recently updated first.
        for (row, updated) in rows.iter_mut().zip([900, 800, 700]) {
            row.updated_at = updated;
        }
        let mut tab = tab_with(rows);
        let numbers_shown = |tab: &mut RepoTab| {
            let frame = render(theme::ThemeId::Midnight, SCREEN, |ui| {
                list(ui, tab, ForgeKind::GitHub, Color32::BLACK, &mut Vec::new());
            });
            let order: Vec<String> = frame
                .texts
                .iter()
                .filter(|(t, _)| t.starts_with('#'))
                .map(|(t, _)| t.split(' ').next().unwrap_or_default().to_owned())
                .collect();
            (order, tab.requests.shown.clone())
        };
        let (order, shown) = numbers_shown(&mut tab);
        assert_eq!(order, ["#31", "#12", "#27"]);
        assert_eq!(shown, Some(vec![31, 12, 27]), "the host's order by default");

        tab.request_view.sort = RequestSort::Number;
        let (order, shown) = numbers_shown(&mut tab);
        assert_eq!(order, ["#12", "#27", "#31"]);
        assert_eq!(
            shown,
            Some(vec![12, 27, 31]),
            "the arrow keys walk this order"
        );

        tab.request_view.text = "allocate".into();
        let (order, shown) = numbers_shown(&mut tab);
        assert_eq!(order, ["#31"], "the description is searched");
        assert_eq!(shown, Some(vec![31]));

        tab.request_view.text.clear();
        tab.request_view.author = Some("sam".into());
        let (order, _) = numbers_shown(&mut tab);
        assert_eq!(order, ["#27"]);
    }

    #[test]
    fn a_request_to_focus_the_filter_does_not_wait_for_a_field() {
        let draw = |tab: &mut RepoTab| {
            render(theme::ThemeId::Midnight, SCREEN, |ui| {
                list(ui, tab, ForgeKind::GitHub, Color32::BLACK, &mut Vec::new());
            });
        };
        let mut tab = tab_with(Vec::new());
        tab.requests.focus_filter = true;
        draw(&mut tab);
        assert!(!tab.requests.focus_filter, "no field was drawn to take it");

        tab.requests.list = Some(Ok(vec![request(1, "Add help", "ann", "")].into()));
        draw(&mut tab);
        assert!(
            !tab.requests.focus_filter,
            "a later list's field is not stolen"
        );
    }

    #[test]
    fn the_arrow_keys_walk_the_shown_order() {
        let mut tab = tab_with(vec![
            request(31, "Stream", "lucia", ""),
            request(12, "Help", "maya", ""),
            request(27, "Lanes", "sam", ""),
        ]);
        tab.request_view.sort = RequestSort::Number;
        render(theme::ThemeId::Midnight, SCREEN, |ui| {
            list(
                ui,
                &mut tab,
                ForgeKind::GitHub,
                Color32::BLACK,
                &mut Vec::new(),
            );
        });
        tab.requests.selected = Some(12);
        tab.move_request(1);
        assert_eq!(
            tab.requests.selected,
            Some(27),
            "the next row, not the next loaded"
        );
        tab.move_request(isize::MAX);
        assert_eq!(tab.requests.selected, Some(31));
        tab.move_request(isize::MIN);
        assert_eq!(tab.requests.selected, Some(12));
    }

    #[test]
    fn the_status_of_a_request_follows_the_rollup_for_the_filters() {
        let ctx = egui::Context::default();
        let tab = test_tab();
        let mut r = request(5, "Checked", "ann", "");
        r.rollup = Some(Rollup {
            checks: Some(CheckState::Failure),
            review: Some(ReviewDecision::Required),
        });
        let standing = row_signals(&ctx, &tab, &r)
            .map(Signals::standing)
            .expect("the rollup is known");
        assert_eq!(standing.checks, Some(ChecksChoice::Failed));
        assert_eq!(standing.review, Some(ReviewChoice::Required));
        r.rollup = Some(Rollup {
            checks: Some(CheckState::Neutral),
            review: None,
        });
        let standing = row_signals(&ctx, &tab, &r)
            .map(Signals::standing)
            .expect("known");
        assert_eq!(standing, Standing::default(), "neutral checks say nothing");
        r.rollup = None;
        assert!(
            row_signals(&ctx, &tab, &r).is_none(),
            "unknown stays unknown"
        );
    }

    #[test]
    fn an_empty_result_says_so_and_clearing_the_filters_brings_the_rows_back() {
        let mut tab = tab_with(vec![request(7, "Add keyboard help", "ann", "")]);
        tab.request_view.text = "zzz".into();
        tab.request_view.sort = RequestSort::Title;
        tab.forges = vec![(
            "origin".to_owned(),
            canopy_core::forge::Forge::parse("https://github.com/acme/widgets.git", &[])
                .expect("a GitHub remote"),
        )];
        tab.snap = Some(Arc::new(canopy_core::RepoSnapshot {
            common_dir: "/fixture/.git".into(),
            ..Default::default()
        }));
        let ctx = kit::testkit::ctx();
        let mut actions = Vec::new();
        let draw = |tab: &mut RepoTab, actions: &mut Vec<Action>, events| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
                    events,
                    ..Default::default()
                },
                |ui| list(ui, tab, ForgeKind::GitHub, Color32::BLACK, actions),
            );
            out.textures_delta.clear();
        };
        ctx.enable_accesskit();
        draw(&mut tab, &mut actions, Vec::new());
        let spoken_now = |ctx: &egui::Context, tab: &mut RepoTab, actions: &mut Vec<Action>| {
            kit::testkit::spoken(ctx, |ui| {
                list(ui, tab, ForgeKind::GitHub, Color32::BLACK, actions);
            })
        };
        let nodes = spoken_now(&ctx, &mut tab, &mut actions);
        let texts: Vec<_> = nodes.iter().filter_map(|n| n.label.clone()).collect();
        assert!(
            texts.iter().any(|t| t == "Clear filters"),
            "the empty state offers the way out: {texts:?}"
        );
        let target = nodes
            .iter()
            .filter(|n| n.label.as_deref() == Some("Clear filters"))
            .find_map(|n| n.bounds)
            .expect("the button has a place");
        draw(&mut tab, &mut actions, kit::testkit::press(target.center()));
        draw(
            &mut tab,
            &mut actions,
            kit::testkit::release(target.center()),
        );
        assert!(tab.request_view.text.is_empty(), "{:?}", tab.request_view);
        assert_eq!(
            tab.request_view.sort,
            RequestSort::Title,
            "the sort is a choice of its own and stays"
        );
        assert!(
            actions.iter().any(|a| matches!(
                a,
                Action::RememberRequestView { view, .. } if view.filters() == 0
            )),
            "the cleared view is queued for saving: {actions:?}"
        );
    }

    #[test]
    fn the_filter_controls_are_named_and_reachable() {
        let mut tab = tab_with(vec![request(7, "Add keyboard help", "ann", "")]);
        tab.forges = vec![(
            "origin".to_owned(),
            canopy_core::forge::Forge::parse("https://github.com/acme/widgets.git", &[])
                .expect("a GitHub remote"),
        )];
        let ctx = kit::testkit::ctx();
        let nodes = kit::testkit::spoken(&ctx, |ui| {
            list(
                ui,
                &mut tab,
                ForgeKind::GitHub,
                Color32::BLACK,
                &mut Vec::new(),
            );
        });
        let named = |part: &str| {
            nodes
                .iter()
                .any(|n| n.label.as_deref().is_some_and(|l| l.contains(part)))
        };
        assert!(named("Filter pull requests"), "the text field");
        assert!(
            named("Filter by draft, checks, review or author"),
            "the filters"
        );
        assert!(
            named("Sort requests: Recently updated"),
            "the sort, with its choice"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.role == egui::accesskit::Role::List
                    && n.label.as_deref() == Some("Requests")),
            "the rows are a named list"
        );
        tab.request_view.draft = Some(DraftChoice::Drafts);
        tab.request_view.sort = RequestSort::Author;
        let nodes = kit::testkit::spoken(&ctx, |ui| {
            list(
                ui,
                &mut tab,
                ForgeKind::GitHub,
                Color32::BLACK,
                &mut Vec::new(),
            );
        });
        assert!(
            nodes
                .iter()
                .any(|n| n.label.as_deref() == Some("Filters: 1 on")),
            "the active filters are counted in the button's name"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.label.as_deref() == Some("Sort requests: Author")),
        );
    }

    #[test]
    fn a_filter_chosen_in_the_popover_narrows_the_list_and_keeps_the_popover_open() {
        let mut tab = tab_with(vec![
            request(31, "Stream large diffs", "lucia", ""),
            request(12, "Add keyboard help", "maya", ""),
        ]);
        tab.requests.list = Some(Ok(Arc::from([
            request(31, "Stream large diffs", "lucia", ""),
            RequestSummary {
                draft: true,
                ..request(12, "Add keyboard help", "maya", "")
            },
        ])));
        tab.forges = vec![(
            "origin".to_owned(),
            canopy_core::forge::Forge::parse("https://github.com/acme/widgets.git", &[])
                .expect("a GitHub remote"),
        )];
        let ctx = kit::testkit::ctx();
        ctx.enable_accesskit();
        let mut actions = Vec::new();
        let mut draw = |tab: &mut RepoTab, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
                    events,
                    ..Default::default()
                },
                |ui| list(ui, tab, ForgeKind::GitHub, Color32::BLACK, &mut actions),
            );
            out.textures_delta.clear();
            out.platform_output
                .accesskit_update
                .map(|u| {
                    u.nodes
                        .into_iter()
                        .filter_map(|(_, n)| {
                            let at = n.bounds()?;
                            let at = Rect::from_min_max(
                                pos2(crate::ui::to_f32(at.x0), crate::ui::to_f32(at.y0)),
                                pos2(crate::ui::to_f32(at.x1), crate::ui::to_f32(at.y1)),
                            );
                            Some((n.label()?.to_owned(), at))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let click = |nodes: &[(String, Rect)], name: &str| {
            let (_, at) = nodes
                .iter()
                .find(|(label, _)| label == name)
                .unwrap_or_else(|| panic!("no {name}: {nodes:?}"));
            at.center()
        };
        let nodes = draw(&mut tab, Vec::new());
        let at = click(&nodes, "Filter by draft, checks, review or author");
        draw(&mut tab, kit::testkit::press(at));
        draw(&mut tab, kit::testkit::release(at));
        let nodes = draw(&mut tab, Vec::new());
        let nodes = if nodes.iter().any(|(l, _)| l == "Draft") {
            nodes
        } else {
            draw(&mut tab, Vec::new())
        };
        let at = click(&nodes, "Draft");
        draw(&mut tab, kit::testkit::press(at));
        draw(&mut tab, kit::testkit::release(at));
        let nodes = draw(&mut tab, Vec::new());
        let at = click(&nodes, "Drafts only");
        draw(&mut tab, kit::testkit::press(at));
        draw(&mut tab, kit::testkit::release(at));
        draw(&mut tab, Vec::new());
        let nodes = draw(&mut tab, Vec::new());
        assert_eq!(tab.request_view.draft, Some(DraftChoice::Drafts));
        assert_eq!(tab.requests.shown, Some(vec![12]), "only the draft is left");
        assert!(
            nodes.iter().any(|(l, _)| l == "Review"),
            "the popover is still open after a choice: {nodes:?}"
        );
        // Esc closes the popover (no list is open to take it).
        draw(&mut tab, kit::testkit::key(egui::Key::Escape));
        let nodes = draw(&mut tab, Vec::new());
        assert!(!nodes.iter().any(|(l, _)| l == "Review"), "{nodes:?}");
    }

    // ---------------------------------------------------------------------------- the scroll bar

    /// `count` requests with the busiest right edge a row has: a title that wraps, a description,
    /// three labels, and a checks glyph and a review glyph.
    fn overflowing(count: u64) -> RepoTab {
        let rows = (1..=count)
            .map(|n| RequestSummary {
                labels: vec!["bug".into(), "accessibility".into(), "needs design".into()],
                rollup: Some(Rollup {
                    checks: Some(CheckState::Failure),
                    review: Some(ReviewDecision::ChangesRequested),
                }),
                ..request(
                    n,
                    "A title that is long enough to wrap onto a second line in a narrow list",
                    "maya-chen",
                    "What the change does, in a sentence or two of plain text.",
                )
            })
            .collect();
        tab_with(rows)
    }

    /// What `list` paints at `size` after a few frames (the bar needs one to know the rows
    /// overflow), with the pointer over the rows and, when `to_the_end`, the wheel turned far
    /// enough to reach the last row; and the right edge of the panel it ran in.
    fn list_painted(
        tab: &mut RepoTab,
        size: egui::Vec2,
        to_the_end: bool,
    ) -> (Vec<egui::epaint::ClippedShape>, f32) {
        let ctx = kit::testkit::ctx();
        let (mut right, mut shapes) = (0.0, Vec::new());
        for step in 0..6_u8 {
            let mut events = Vec::new();
            if step == 1 {
                events.push(egui::Event::PointerMoved(pos2(size.x / 2.0, size.y / 2.0)));
            }
            if to_the_end && step == 2 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -1.0e6),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(f64::from(step) * 0.05),
                    events,
                    ..Default::default()
                },
                |ui| {
                    right = ui.max_rect().right();
                    list(ui, tab, ForgeKind::GitHub, Color32::BLACK, &mut Vec::new());
                },
            );
            output.textures_delta.clear();
            shapes = output.shapes;
        }
        (shapes, right)
    }

    /// The owner's report: the bar must be visible and must never sit over a status glyph or a
    /// label chip. The bar keeps its own lane (`solid_scroll`), the handle is the kit's, and
    /// every part of a row ends at least a handle, its margin and a gutter short of it.
    #[test]
    fn the_scroll_bar_is_visible_and_keeps_its_lane_clear_of_every_row() {
        let solid = theme::solid_scroll();
        // The handle at its widest (hovered), its margin and a gutter: the least a row may leave.
        let clear = solid.bar_width + solid.bar_inner_margin + theme::XS;
        for size in [vec2(420.0, 600.0), vec2(320.0, 480.0), vec2(800.0, 900.0)] {
            let mut tab = overflowing(40);
            let (shapes, right) = list_painted(&mut tab, size, false);
            let lane = right - solid.floating_allocated_width;
            let (fills, inks) = kit::testkit::painted_colors(&shapes);
            let handle = fills
                .iter()
                .map(|(rect, _)| *rect)
                .find(|r| r.left() >= lane - 0.5 && r.width() <= solid.bar_width + 0.5)
                .unwrap_or_else(|| panic!("no scroll bar handle in the lane at {size:?}"));
            assert!(
                handle.height() >= theme::SCROLL_HANDLE_MIN,
                "{size:?}: a handle of {} is hard to grab",
                handle.height()
            );
            // What a row paints: its text, and its label chips (the row's own band, a hover or a
            // selection, is taller than a line and ends at the list's edge by design).
            let rows = |r: &Rect| r.top() >= handle.top() - 0.5 && r.left() < lane;
            let furthest = inks
                .iter()
                .map(|(r, _)| *r)
                .chain(fills.iter().map(|(r, _)| *r).filter(|r| r.height() < 24.0))
                .filter(rows)
                .map(|r| r.right())
                .fold(f32::MIN, f32::max);
            assert!(
                lane - furthest >= clear,
                "{size:?}: a row reaches {furthest}, {} short of the lane at {lane}; needs {clear}",
                lane - furthest
            );
        }
    }

    /// The last row scrolls clear of the bottom edge: its last line of text ends the row's own
    /// padding and the list's room short of it.
    /// The filter bar fits the panel it is in: its buttons took room the field did not leave, and
    /// the overflow widened the list, so the scroll bar's lane sat 4 points past the panel's edge.
    #[test]
    fn the_filter_bar_does_not_outgrow_its_panel() {
        let ctx = kit::testkit::ctx();
        for width in [220.0_f32, 320.0, 560.0, 800.0] {
            let mut tab = overflowing(3);
            let items = match &tab.requests.list {
                Some(Ok(rows)) => Arc::clone(rows),
                _ => unreachable!("the tab has a list"),
            };
            let mut used = 0.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    filter_bar(ui, &mut tab, &items, &mut Vec::new());
                    used = ui.min_rect().right();
                },
            );
            output.textures_delta.clear();
            assert!(
                used <= width + 0.5,
                "the filter bar is {used} wide in a panel {width} wide"
            );
        }
    }

    #[test]
    fn the_last_row_scrolls_clear_of_the_bottom_edge() {
        let size = vec2(420.0, 600.0);
        let mut tab = overflowing(40);
        let (shapes, _) = list_painted(&mut tab, size, true);
        let (_, inks) = kit::testkit::painted_colors(&shapes);
        let lowest = inks
            .iter()
            .map(|(r, _)| r.bottom())
            .fold(f32::MIN, f32::max);
        assert!(
            lowest <= size.y - 2.0 * theme::SM,
            "the last line ends at {lowest} in a list {} high",
            size.y
        );
    }

    #[test]
    fn the_list_follows_the_window_until_the_user_sets_a_width() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut tab = test_tab();
        tab.forges = vec![(
            "origin".to_owned(),
            canopy_core::forge::Forge::parse("https://github.com/acme/widgets.git", &[])
                .expect("a GitHub remote"),
        )];
        let mut t = 0.0;
        let mut frame = |width: f32, events: Vec<egui::Event>, tab: &mut RepoTab| {
            t += 0.05;
            let mut shown = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 700.0))),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    shown = show(ui, tab, true, &mut Vec::new())
                        .map(|(list, handle)| (list, ui.ctx().read_response(handle)));
                },
            );
            output.textures_delta.clear();
            shown.expect("the list is shown")
        };
        let width_at = |room: f32| (room * LIST_SHARE).clamp(LIST_DEFAULT_MIN, LIST_DEFAULT_MAX);
        for room in [1200.0, 1000.0, 1400.0] {
            let (list, _) = frame(room, Vec::new(), &mut tab);
            let (list_again, _) = frame(room, Vec::new(), &mut tab);
            assert!(
                (list.width() - width_at(room)).abs() < 1.0
                    && (list_again.width() - list.width()).abs() < 1.0,
                "an unset width follows the window: {room} gave {}",
                list.width()
            );
        }
        // The user sets one: it stays when the window changes.
        let (_, handle) = frame(1200.0, Vec::new(), &mut tab);
        let handle = handle.expect("the handle is shown");
        ctx.memory_mut(|m| m.request_focus(handle.id));
        let right = vec![egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        frame(1200.0, right, &mut tab);
        let (own, _) = frame(1200.0, Vec::new(), &mut tab);
        let wanted = width_at(1200.0) + split::KEY_STEP;
        assert!((own.width() - wanted).abs() < 1.0, "{own:?}");
        let (smaller, _) = frame(1000.0, Vec::new(), &mut tab);
        assert!((smaller.width() - wanted).abs() < 1.0, "{smaller:?}");
    }
}
