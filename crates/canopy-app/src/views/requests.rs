//! Pull requests (GitHub) and merge requests (GitLab): a list on the left, the selected request
//! on the right with its checks, reviews, description and conversation, and the actions a
//! reviewer needs (check out, approve, comment, merge, close or reopen).

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, FontId, Galley, Layout, Painter, Response, RichText, Ui, UiBuilder,
    pos2, vec2,
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
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{Disclosure, ListRow, RowKind, SectionHeader};
use crate::ui::kit::select::Select;
use crate::ui::kit::surface::{Banner, EmptyAction, EmptyState, InsetBox};
use crate::ui::kit::{self, Button, IconButton, Size, Tone};
use crate::ui::theme::{self, Style, palette};
use crate::ui::widgets::{self, forge_icon};
use crate::views::commit_detail;
use crate::views::dialogs::Dialog;

/// Why an action waits.
const WAIT: &str = "Wait for the current operation";
/// The widest a branch pill in the request header grows before it is cut.
const BRANCH_PILL_MAX: f32 = 240.0;

/// Draws the view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, api_enabled: bool, actions: &mut Vec<Action>) {
    operation_banner(ui, tab, actions);
    let Some((_, forge)) = tab.forge().cloned() else {
        empty(
            ui,
            art::local_only(),
            "No GitHub or GitLab remote",
            "Pull and merge requests appear here for repositories hosted on GitHub or GitLab.",
            None,
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
    // Standard sets the list on the chrome tone beside the content; Deeper keeps it on the card.
    let list_ground = if theme::style() == Style::Deeper {
        s.pane
    } else {
        s.panel
    };
    let pane_max = (ui.available_width() * 0.4).max(160.0);
    egui::Panel::left("requests-list")
        .resizable(true)
        .default_size(pane_max.min(380.0))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(640.0))
        .frame(egui::Frame::new().fill(list_ground))
        .show(ui, |ui| {
            let r = ui.max_rect();
            kit::paint::seam_v(ui.painter(), r.right() - 0.5, r.y_range(), s.border);
            list(ui, tab, kind, list_ground, actions);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(s.pane))
        .show(ui, |ui| detail(ui, tab, kind, actions));
}

/// The merge, rebase, cherry-pick or revert in progress, with Continue and Abort.
fn operation_banner(ui: &mut Ui, tab: &RepoTab, actions: &mut Vec<Action>) {
    let Some(snap) = &tab.snap else {
        return;
    };
    if snap.state == canopy_core::RepoState::Clean {
        return;
    }
    let m = theme::metrics();
    let state = snap.state;
    let conflicts = snap.status.conflicted.len();
    let busy = tab.running.is_some();
    egui::Panel::top("repository-operation-state")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let left = if conflicts > 0 {
                format!(
                    "{conflicts} conflict{} left",
                    if conflicts == 1 { "" } else { "s" }
                )
            } else {
                "No conflicts left".to_owned()
            };
            let banner = Banner::new(
                Tone::Warning,
                format!("{} in progress on {}", state.label(), snap.head.label()),
            )
            .body(left)
            .show(ui);
            let Some(command) = state.command() else {
                return;
            };
            let rect = banner.response.rect;
            if rect.height() <= 0.0 {
                return;
            }
            // The banner grows in from its top edge: its content sits on the bottom.
            let full = egui::Rect::from_min_size(
                pos2(rect.left(), rect.bottom() - m.pane_header_height),
                vec2(rect.width(), m.pane_header_height),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(full.shrink2(vec2(theme::SM, 0.0)))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            if Button::new(format!("Abort {command}…"))
                .ghost()
                .size(Size::Sm)
                .icon(icon::X_CIRCLE)
                .icon_tone(Tone::Danger)
                .enabled(!busy)
                .disabled_reason(WAIT)
                .id_salt("operation-abort")
                .show(&mut child)
                .clicked()
            {
                actions.push(Action::Confirm {
                    title: format!("Abort {command}"),
                    body: "Return the repository to where it was before the operation started? Resolved conflicts are discarded.".into(),
                    confirm: "Abort".into(),
                    danger: true,
                    op: crate::worker::Op::ContinueOrAbort { state, abort: true },
                });
            }
            let can_continue = conflicts == 0 && !busy;
            if Button::new("Continue")
                .ghost()
                .size(Size::Sm)
                .icon(icon::ARROW_RIGHT)
                .enabled(can_continue)
                .tooltip("Continue this operation using the staged resolutions")
                .disabled_reason(if busy {
                    WAIT
                } else {
                    "Resolve and stage every conflicted file first"
                })
                .id_salt("operation-continue")
                .show(&mut child)
                .clicked()
            {
                actions.push(Action::Op(crate::worker::Op::ContinueOrAbort {
                    state,
                    abort: false,
                }));
            }
        });
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| format!("{}{}", f.to_uppercase(), c.as_str()))
        .unwrap_or_default()
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

/// A request's state: its glyph, tone and word.
fn state_style(r: &RequestSummary) -> (&'static str, Tone, &'static str) {
    match (r.state, r.draft) {
        (RequestState::Open, true) => (icon::PENCIL_SIMPLE_LINE, Tone::Neutral, "Draft"),
        (RequestState::Open, false) => (icon::GIT_PULL_REQUEST, Tone::Success, "Open"),
        (RequestState::Merged, _) => (icon::GIT_MERGE, Tone::Accent, "Merged"),
        (RequestState::Closed, _) => (icon::GIT_PULL_REQUEST, Tone::Danger, "Closed"),
    }
}

fn list(
    ui: &mut Ui,
    tab: &mut RepoTab,
    kind: ForgeKind,
    ground: Color32,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    egui::Frame::new()
        .inner_margin(egui::Margin::same(theme::SM as i8))
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
    kit::paint::seam_h(ui.painter(), r.x_range(), r.min.y + 0.5, s.border);

    match tab.requests.list.clone() {
        None => {
            ui.add_space(theme::XXL);
            ui.vertical_centered(|ui| {
                Spinner::new()
                    .label(format!("{}s", kind.request_noun()))
                    .show(ui);
                ui.label(
                    RichText::new(format!("Loading {}s…", kind.request_noun()))
                        .size(m.body_size)
                        .color(p.text_dim),
                );
            });
        }
        Some(Err(e)) => {
            let no_token = tab.requests.token == Some(None);
            let title = if no_token {
                format!("Connect {} to see {}s", kind.name(), kind.request_noun())
            } else {
                format!("Could not load {}s", kind.request_noun())
            };
            // The main action leads: a token when there is none, else another try.
            let (main, other) = if no_token {
                ("Add a token…", "Retry")
            } else {
                ("Retry", "Add a token…")
            };
            let clicked = EmptyState::error(title, e)
                .action(main)
                .link(other)
                .id_salt("request-error")
                .show(ui)
                .value;
            let token = match clicked {
                Some(EmptyAction::Primary) => Some(no_token),
                Some(EmptyAction::Link) => Some(!no_token),
                None => None,
            };
            match token {
                Some(true) => actions.push(Action::Settings),
                Some(false) => tab.load_requests(),
                None => {}
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
        Some(Ok(items)) => {
            let now = chrono::Utc::now().timestamp();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(
                            m.sidebar_pad as i8,
                            theme::XS as i8,
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
                                    if row(ui, r, selected, now, ground).clicked() {
                                        tab.select_request(r.number);
                                    }
                                },
                            );
                        });
                });
        }
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

/// A request in the list: state glyph, title, number, author and age, and its branches.
fn row(ui: &mut Ui, r: &RequestSummary, selected: bool, now: i64, ground: Color32) -> Response {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (glyph, tone, state) = state_style(r);
    let color = tone.color(p);
    let info = format!("{state} request #{} {} by {}", r.number, r.title, r.author);
    let response = ListRow::new(("request", r.number))
        .kind(RowKind::Request)
        .selected(selected)
        .ground(ground)
        .tooltip(format!(
            "{state} · {}\n#{} · {}\n{} → {}",
            r.title, r.number, r.author, r.source_branch, r.target_branch
        ))
        .show_with(ui, |rc, painter| {
            let c = rc.content;
            let h = rc.rect.height();
            // Three lines: title, then number, author and age, then the branches.
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
            let title = line_galley(
                painter,
                r.title.clone(),
                FontId::new(m.body_size, theme::medium()),
                rc.text,
                width,
            );
            painter.galley(at(&title, x, title_y), title, rc.text);
            let meta = line_galley(
                painter,
                format!(
                    "#{}  ·  {}  ·  {}",
                    r.number,
                    r.author,
                    widgets::relative_time(r.updated_at, now)
                ),
                FontId::proportional(m.small_size),
                p.text_faint,
                width,
            );
            painter.galley(at(&meta, x, meta_y), meta, p.text_faint);
            let small = FontId::proportional(m.small_size);
            let target_width = painter
                .layout_no_wrap(r.target_branch.clone(), small.clone(), p.text_dim)
                .size()
                .x
                .min(width * 0.45);
            let source = line_galley(
                painter,
                r.source_branch.clone(),
                small.clone(),
                p.text_dim,
                (width - target_width - theme::XL).max(0.0),
            );
            let source_width = source.size().x;
            painter.galley(at(&source, x, branch_y), source, p.text_dim);
            painter.text(
                pos2(x + source_width + theme::SM, branch_y),
                Align2::LEFT_CENTER,
                "→",
                small,
                p.text_faint,
            );
            let target = line_galley(
                painter,
                r.target_branch.clone(),
                FontId::new(m.small_size, theme::medium()),
                p.text_dim,
                target_width,
            );
            painter.galley(
                at(&target, x + source_width + theme::XL, branch_y),
                target,
                p.text_dim,
            );
        });
    name_row(ui, &response, selected, &info);
    response
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

fn detail(ui: &mut Ui, tab: &mut RepoTab, kind: ForgeKind, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let Some(number) = tab.requests.selected else {
        EmptyState::new(format!("Select a {} to see it", kind.request_noun()))
            .icon(icon::GIT_PULL_REQUEST)
            .compact(true)
            .show(ui);
        return;
    };
    let summary = match &tab.requests.list {
        Some(Ok(l)) => l.iter().find(|r| r.number == number).cloned(),
        _ => None,
    };
    let details = match &tab.requests.details {
        Some(Ok(d)) => Some(Arc::clone(d)),
        _ => None,
    };
    let Some(summary) = details.as_ref().map(|d| d.summary.clone()).or(summary) else {
        ui.centered_and_justified(|ui| {
            Spinner::new().label(kind.request_noun()).show(ui);
        });
        return;
    };
    let now = chrono::Utc::now().timestamp();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(theme::XL as i8, theme::LG as i8))
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(theme::MEASURE));
                    // Header.
                    let (glyph, tone, state) = state_style(&summary);
                    ui.horizontal(|ui| {
                        Badge::new(format!("{glyph} {state}")).tone(tone).show(ui);
                        ui.label(
                            RichText::new(format!("#{}", summary.number))
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
                            ui.label(
                                RichText::new(format!("{}  {e}", icon::WARNING_CIRCLE))
                                    .size(m.body_size)
                                    .color(p.danger),
                            );
                        }
                        (None, Some(d)) => {
                            body(ui, d, now, actions);
                            comment_box(ui, tab, number, actions);
                        }
                        (None, None) => {
                            Spinner::new().label(kind.request_noun()).show(ui);
                        }
                    }
                });
        });
}

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

/// The action buttons. Merge and close ask for confirmation in a dialog.
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
    ui.horizontal_wrapped(|ui| {
        if Button::new("Check out")
            .icon(icon::DOWNLOAD_SIMPLE)
            .enabled(!busy)
            .tooltip("Fetch the request's branch and check it out locally")
            .disabled_reason(WAIT)
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
                            "Close {} {}{}?",
                            kind.request_abbrev(),
                            kind_sigil(kind),
                            r.number
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
        ui.horizontal_wrapped(|ui| {
            if Button::new("Approve")
                .icon(icon::CHECK)
                .icon_tone(Tone::Success)
                .enabled(approve_action(tab, r).is_some())
                .tooltip("Approve this request")
                .shortcut(Command::ApproveRequest)
                .disabled_reason(WAIT)
                .show(ui)
                .clicked()
                && let Some(action) = approve_action(tab, r)
            {
                actions.push(action);
            }
            if ui.available_size_before_wrap().x < 220.0 {
                ui.end_row();
            }
            InsetBox::new().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let methods = MergeMethod::ALL.map(|method| (method, method.label()));
                    Select::new(&mut tab.requests.merge_method, &methods)
                        .name("Merge method")
                        .width(170.0_f32.min((ui.available_width() - 120.0).max(80.0)))
                        .id_salt("merge-method")
                        .show(ui);
                    let dialog = merge_confirmation(tab, kind, r, d);
                    let reason = if busy {
                        WAIT
                    } else if r.draft {
                        "Drafts cannot be merged"
                    } else if d.is_none_or(|details| details.head_sha.is_none()) {
                        "Loading the request…"
                    } else {
                        "This request has conflicts or is blocked"
                    };
                    if Button::new("Merge…")
                        .primary()
                        .icon(icon::GIT_MERGE)
                        .enabled(dialog.is_some())
                        .tooltip("Confirm this merge before it runs")
                        .disabled_reason(reason)
                        .show(ui)
                        .clicked()
                        && let Some(dialog) = dialog
                    {
                        actions.push(Action::OpenDialog(dialog));
                    }
                });
            });
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

pub(crate) fn approve_action(tab: &RepoTab, request: &RequestSummary) -> Option<Action> {
    (request.state == RequestState::Open && tab.requests.busy.is_none() && tab.running.is_none())
        .then(|| Action::Host(HostJob::Approve(request.number)))
}

/// Shared by the pointer and keyboard paths; both confirm the reviewed head before merging.
pub(crate) fn merge_confirmation(
    tab: &RepoTab,
    kind: ForgeKind,
    r: &RequestSummary,
    d: Option<&RequestDetails>,
) -> Option<Dialog> {
    if tab.requests.busy.is_some()
        || tab.running.is_some()
        || r.state != RequestState::Open
        || r.draft
    {
        return None;
    }
    let details = d.filter(|details| details.summary.number == r.number)?;
    if details.mergeable == Some(false) {
        return None;
    }
    let head = details.head_sha.as_ref().filter(|head| !head.is_empty())?;
    let short: String = head.chars().take(7).collect();
    let method = tab.requests.merge_method;
    Some(Dialog::ConfirmHost {
        title: format!(
            "Merge {} {}{}?",
            kind.request_abbrev(),
            kind_sigil(kind),
            r.number
        ),
        body: format!(
            "{} \"{}\" ({}) into {} on {}, as reviewed at commit {short}. If anyone pushes to it before the merge runs, {} refuses the merge. This cannot be undone from Canopy.",
            method.label(),
            r.title,
            r.source_branch,
            r.target_branch,
            kind.name(),
            kind.name()
        ),
        confirm: "Merge".into(),
        danger: false,
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
                            .on_hover_text(word);
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
                .on_hover_text(&r.state);
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
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(196.0, 700.0),
                )),
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
}
