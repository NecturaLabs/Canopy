//! Pull requests (GitHub) and merge requests (GitLab): a list on the left, the selected request
//! on the right with its checks, reviews, description and conversation, and the actions a
//! reviewer needs (check out, approve, comment, merge, close or reopen).

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::forge::ForgeKind;
use canopy_hosting::{
    CheckState, MergeMethod, RequestDetails, RequestState, RequestSummary, StateFilter,
};

use crate::action::Action;
use crate::hosting::{HostJob, TokenSource};
use crate::tab::RepoTab;
use crate::ui::theme::{self, Palette, palette};
use crate::ui::widgets::forge_icon;
use crate::ui::{art, widgets};
use crate::views::commit_detail;
use crate::views::dialogs::Dialog;

/// Draws the view.
pub fn show(ui: &mut Ui, tab: &mut RepoTab, api_enabled: bool, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    widgets::repository_state_banner(ui, tab, actions);
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

    let pane_max = (ui.available_width() * 0.4).max(160.0);
    egui::Panel::left("requests-list")
        .resizable(true)
        .default_size(pane_max.min(380.0))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(640.0))
        .frame(
            egui::Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border)),
        )
        .show(ui, |ui| list(ui, tab, kind, actions));
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.bg))
        .show(ui, |ui| detail(ui, tab, kind, actions));
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
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
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(30.0));
        art::show(ui, image, vec2(160.0, 120.0));
        ui.add_space(8.0);
        ui.label(RichText::new(title).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
        ui.add_space(2.0);
        ui.label(RichText::new(body).color(p.text_dim));
        if let Some((label, action)) = button {
            ui.add_space(12.0);
            if ui.button(label).clicked() {
                actions.push(action);
            }
        }
    });
}

fn state_style(p: &Palette, r: &RequestSummary) -> (&'static str, egui::Color32, &'static str) {
    match (r.state, r.draft) {
        (RequestState::Open, true) => (icon::PENCIL_SIMPLE_LINE, p.text_dim, "Draft"),
        (RequestState::Open, false) => (icon::GIT_PULL_REQUEST, p.success, "Open"),
        (RequestState::Merged, _) => (icon::GIT_MERGE, p.accent, "Merged"),
        (RequestState::Closed, _) => (icon::GIT_PULL_REQUEST, p.danger, "Closed"),
    }
}

fn list(ui: &mut Ui, tab: &mut RepoTab, kind: ForgeKind, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(10, 10))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let options = StateFilter::ALL.map(|f| (f, f.label()));
                if widgets::segmented(ui, &mut tab.requests.filter, &options) {
                    tab.requests.list = None;
                    tab.load_requests();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if tab.requests.loading {
                        ui.spinner();
                    } else if widgets::icon_button(ui, icon::ARROWS_CLOCKWISE, "Reload").clicked() {
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
                        && ui
                            .button(format!("{}  New {}", icon::PLUS, kind.request_abbrev()))
                            .on_hover_text(format!("Create a {} for {branch}", kind.request_noun()))
                            .clicked()
                    {
                        actions.push(Action::CreateRequest { source: branch });
                    }
                });
            });
        });
    let r = ui.available_rect_before_wrap();
    ui.painter()
        .hline(r.x_range(), r.min.y, Stroke::new(1.0, p.border));

    match tab.requests.list.clone() {
        None => {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(format!("Loading {}s…", kind.request_noun())).color(p.text_dim),
                );
            });
        }
        Some(Err(e)) => {
            let no_token = tab.requests.token == Some(None);
            egui::ScrollArea::vertical()
                .id_salt("request-error")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(ui.available_width() - 32.0);
                        art::show(ui, art::empty_error(), vec2(120.0, 90.0));
                        ui.label(
                            RichText::new(if no_token {
                                format!("Connect {} to see {}s", kind.name(), kind.request_noun())
                            } else {
                                format!("Could not load {}s", kind.request_noun())
                            })
                            .font(FontId::new(theme::SUBTITLE_SIZE, theme::semibold())),
                        );
                        ui.label(RichText::new(&e).size(theme::META_SIZE).color(p.text_dim));
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space((ui.available_width() - 230.0).max(0.0) / 2.0);
                            if ui.button(format!("{}  Add a token…", icon::KEY)).clicked() {
                                actions.push(Action::Settings);
                            }
                            if ui
                                .button(format!("{}  Retry", icon::ARROWS_CLOCKWISE))
                                .clicked()
                            {
                                tab.load_requests();
                            }
                        });
                    });
                });
        }
        Some(Ok(items)) if items.is_empty() => {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(icon::CHECK_CIRCLE)
                        .size(theme::ICON_HERO)
                        .color(p.success),
                );
                ui.label(
                    RichText::new(format!(
                        "No {} {}s",
                        tab.requests.filter.label().to_lowercase(),
                        kind.request_noun()
                    ))
                    .color(p.text_dim),
                );
            });
        }
        Some(Ok(items)) => {
            let now = chrono::Utc::now().timestamp();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    commit_detail::virtual_rows(ui, items.len(), REQUEST_ROW_H, |ui, i| {
                        let r = &items[i];
                        if row(ui, r, tab.requests.selected == Some(r.number), now).clicked() {
                            tab.select_request(r.number);
                        }
                    });
                });
        }
    }
}

const REQUEST_ROW_H: f32 = 74.0;

fn row(ui: &mut Ui, r: &RequestSummary, selected: bool, now: i64) -> egui::Response {
    let p = palette(ui.ctx());
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), REQUEST_ROW_H), Sense::hover());
    let resp = ui.interact(rect, ui.id().with(("request", r.number)), Sense::click());
    if selected {
        ui.painter().rect_filled(
            rect,
            CornerRadius::ZERO,
            Palette::tint(p.accent, if p.dark { 46 } else { 30 }),
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            CornerRadius::ZERO,
            p.accent,
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.hover);
    }
    let (glyph, color, state) = state_style(p, r);
    ui.painter().text(
        rect.left_top() + vec2(18.0, 18.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(theme::ICON_MD),
        color,
    );
    let clip = egui::Rect::from_min_max(
        rect.left_top() + vec2(34.0, 0.0),
        rect.right_bottom() - vec2(10.0, 0.0),
    );
    let painter = ui.painter().with_clip_rect(clip);
    let title = widgets::single_line_galley(
        ui,
        r.title.clone(),
        FontId::new(theme::LEAD_SIZE, theme::medium()),
        p.text,
        clip.width(),
    );
    painter.galley(
        egui::pos2(clip.min.x, rect.min.y + 18.0 - title.size().y / 2.0),
        title,
        p.text,
    );
    let meta = format!(
        "#{}  ·  {}  ·  {}",
        r.number,
        r.author,
        widgets::relative_time(r.updated_at, now)
    );
    let meta = widgets::single_line_galley(
        ui,
        meta,
        FontId::proportional(theme::SMALL_SIZE),
        p.text_faint,
        clip.width(),
    );
    painter.galley(
        egui::pos2(clip.min.x, rect.min.y + 37.0 - meta.size().y / 2.0),
        meta,
        p.text_faint,
    );
    let target_width = ui
        .painter()
        .layout_no_wrap(
            r.target_branch.clone(),
            FontId::proportional(theme::SMALL_SIZE),
            p.text_dim,
        )
        .size()
        .x
        .min(clip.width() * 0.45);
    let source = widgets::single_line_galley(
        ui,
        r.source_branch.clone(),
        FontId::proportional(theme::SMALL_SIZE),
        p.text_dim,
        (clip.width() - target_width - theme::XL).max(0.0),
    );
    let source_width = source.size().x;
    painter.galley(
        egui::pos2(clip.min.x, rect.min.y + 56.0 - source.size().y / 2.0),
        source,
        p.text_dim,
    );
    painter.text(
        egui::pos2(clip.min.x + source_width + theme::SM, rect.min.y + 56.0),
        Align2::LEFT_CENTER,
        "→",
        FontId::proportional(theme::SMALL_SIZE),
        p.text_faint,
    );
    let target = widgets::single_line_galley(
        ui,
        r.target_branch.clone(),
        FontId::new(theme::SMALL_SIZE, theme::medium()),
        p.text_dim,
        target_width,
    );
    painter.galley(
        egui::pos2(
            clip.min.x + source_width + theme::XL,
            rect.min.y + 56.0 - target.size().y / 2.0,
        ),
        target,
        p.text_dim,
    );
    let info = format!("{state} request #{} {} by {}", r.number, r.title, r.author);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &info)
    });
    resp.on_hover_text(format!(
        "{state} · {}\n#{} · {}\n{} → {}",
        r.title, r.number, r.author, r.source_branch, r.target_branch
    ))
}

fn detail(ui: &mut Ui, tab: &mut RepoTab, kind: ForgeKind, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some(number) = tab.requests.selected else {
        ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new(format!("Select a {} to see it", kind.request_noun()))
                    .color(p.text_faint),
            )
        });
        return;
    };
    let summary = match &tab.requests.list {
        Some(Ok(l)) => l.iter().find(|r| r.number == number).cloned(),
        _ => None,
    };
    let details = match &tab.requests.details {
        Some(Ok(d)) => Some(d.clone()),
        _ => None,
    };
    let Some(summary) = details.as_ref().map(|d| d.summary.clone()).or(summary) else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };
    let now = chrono::Utc::now().timestamp();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(22, 18))
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(theme::MEASURE));
                    // Header.
                    let (glyph, color, state) = state_style(p, &summary);
                    ui.horizontal(|ui| {
                        egui::Frame::new()
                            .fill(Palette::tint(color, 40))
                            .corner_radius(CornerRadius::same(11))
                            .inner_margin(egui::Margin::symmetric(9, 3))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("{glyph}  {state}"))
                                        .size(theme::META_SIZE)
                                        .color(color)
                                        .strong(),
                                )
                            });
                        ui.label(
                            RichText::new(format!("#{}", summary.number))
                                .size(theme::SUBTITLE_SIZE)
                                .color(p.text_faint),
                        );
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(&summary.title)
                            .font(FontId::new(theme::HEADLINE_SIZE, theme::semibold())),
                    );
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&summary.author).color(p.text).strong());
                        ui.label(RichText::new("wants to merge").color(p.text_dim));
                        branch_pill(ui, &summary.source_branch);
                        ui.label(RichText::new("into").color(p.text_dim));
                        branch_pill(ui, &summary.target_branch);
                        ui.label(
                            RichText::new(format!(
                                "· updated {}",
                                widgets::relative_time(summary.updated_at, now)
                            ))
                            .color(p.text_faint)
                            .size(theme::META_SIZE),
                        );
                    });
                    if !summary.labels.is_empty() {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            for l in &summary.labels {
                                egui::Frame::new()
                                    .fill(Palette::tint(p.accent2, 30))
                                    .corner_radius(CornerRadius::same(9))
                                    .inner_margin(egui::Margin::symmetric(8, 1))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(l)
                                                .size(theme::SMALL_SIZE)
                                                .color(p.accent2),
                                        )
                                    });
                            }
                        });
                    }
                    ui.add_space(12.0);
                    action_bar(ui, tab, kind, &summary, details.as_deref(), actions);
                    ui.add_space(16.0);

                    let error = match &tab.requests.details {
                        Some(Err(e)) => Some(e.clone()),
                        _ => None,
                    };
                    match (error, details.as_ref()) {
                        (Some(e), _) => {
                            ui.label(
                                RichText::new(format!("{}  {e}", icon::WARNING)).color(p.danger),
                            );
                        }
                        (None, Some(d)) => {
                            body(ui, d, now);
                            comment_box(ui, tab, number, actions);
                        }
                        (None, None) => {
                            ui.spinner();
                        }
                    }
                });
        });
}

fn branch_pill(ui: &mut Ui, name: &str) {
    let p = palette(ui.ctx());
    let fill = Palette::pill_fill(p.accent2, false);
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{}  {name}", icon::GIT_BRANCH))
                        .font(egui::FontId::new(theme::SMALL_SIZE, theme::medium()))
                        .color(p.ink(p.text, fill)),
                )
                .truncate(),
            )
            .on_hover_text(name);
        });
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
    let busy = tab.requests.busy.is_some() || tab.running.is_some();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(format!("{}  Check out", icon::DOWNLOAD_SIMPLE)),
            )
            .on_hover_text("Fetch the request's branch and check it out locally")
            .clicked()
        {
            actions.push(Action::CheckoutRequest(r.clone()));
        }
        if ui
            .button(format!("{}  Open on {}", forge_icon(kind), kind.name()))
            .clicked()
        {
            actions.push(Action::OpenUrl(r.web_url.clone()));
        }
        if widgets::icon_button(ui, icon::LINK, "Copy link").clicked() {
            actions.push(Action::Copy(r.web_url.clone()));
        }
        ui.menu_button("More", |ui| {
            if r.state == RequestState::Open
                && ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(RichText::new("Close request…").color(p.danger)),
                    )
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
                ui.close();
            } else if r.state == RequestState::Closed
                && ui
                    .add_enabled(!busy, egui::Button::new("Reopen request"))
                    .clicked()
            {
                actions.push(Action::Host(HostJob::SetOpen(r.number, true)));
                ui.close();
            } else if r.state == RequestState::Merged {
                ui.label("Already merged");
            }
        });
    });
    if r.state == RequestState::Open {
        ui.add_space(theme::SM);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    approve_action(tab, r).is_some(),
                    egui::Button::new(
                        RichText::new(format!("{}  Approve", icon::CHECK)).color(p.success),
                    ),
                )
                .on_hover_text(format!(
                    "Approve this request  ({})",
                    crate::keymap::hint(crate::keymap::Command::ApproveRequest)
                ))
                .clicked()
                && let Some(action) = approve_action(tab, r)
            {
                actions.push(action);
            }
            if ui.available_size_before_wrap().x < 220.0 {
                ui.end_row();
            }
            egui::Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(theme::RADIUS_SM))
                .inner_margin(egui::Margin::same(theme::XS as i8))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("merge-method")
                            .icon(widgets::combo_caret)
                            .selected_text(tab.requests.merge_method.label())
                            .width(170.0_f32.min((ui.available_width() - 120.0).max(80.0)))
                            .wrap_mode(egui::TextWrapMode::Truncate)
                            .show_ui(ui, |ui| {
                                for method in MergeMethod::ALL {
                                    ui.selectable_value(
                                        &mut tab.requests.merge_method,
                                        method,
                                        method.label(),
                                    );
                                }
                            });
                        let dialog = merge_confirmation(tab, kind, r, d);
                        let reason = if busy {
                            "Wait for the current operation"
                        } else if r.draft {
                            "Drafts cannot be merged"
                        } else if d.is_none_or(|details| details.head_sha.is_none()) {
                            "Loading the request…"
                        } else {
                            "This request has conflicts or is blocked"
                        };
                        if widgets::primary_button(
                            ui,
                            &format!("{}  Merge…", icon::GIT_MERGE),
                            dialog.is_some(),
                            110.0,
                        )
                        .on_hover_text(if dialog.is_some() {
                            "Confirm this merge before it runs"
                        } else {
                            reason
                        })
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
            ui.spinner();
            ui.label(RichText::new(b).size(theme::META_SIZE).color(p.accent));
        });
    }
    if let Some(Some(source)) = tab.requests.token
        && source != TokenSource::Saved
    {
        ui.label(
            RichText::new(format!("{}  Using the token {}", icon::KEY, source.label()))
                .size(theme::SMALL_SIZE)
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

fn body(ui: &mut Ui, d: &RequestDetails, now: i64) {
    let p = palette(ui.ctx());
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
                .size(theme::META_SIZE)
                .color(p.text_dim),
        );
        ui.add_space(12.0);
    }

    // Description.
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if d.body.trim().is_empty() {
                ui.label(
                    RichText::new("No description.")
                        .italics()
                        .color(p.text_faint),
                );
            } else {
                crate::ui::notes::markdown(ui, d.body.trim());
            }
        });
    ui.add_space(16.0);

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
        let (glyph, color, text) = if failed > 0 {
            (
                icon::X_CIRCLE,
                p.danger,
                format!("{failed} of {} checks failed", d.checks.len()),
            )
        } else if pending > 0 {
            (
                icon::CIRCLE_DASHED,
                p.warning,
                format!("{pending} of {} checks running", d.checks.len()),
            )
        } else {
            (
                icon::CHECK_CIRCLE,
                p.success,
                format!("All {} checks passed", d.checks.len()),
            )
        };
        egui::CollapsingHeader::new(
            RichText::new(format!("{glyph}  {text}"))
                .color(color)
                .strong(),
        )
        .id_salt("checks")
        .icon(crate::ui::widgets::collapse_caret)
        .default_open(failed > 0)
        .show(ui, |ui| {
            for c in &d.checks {
                let (g, col, word) = match c.state {
                    CheckState::Success => (icon::CHECK_CIRCLE, p.success, "passed"),
                    CheckState::Failure => (icon::X_CIRCLE, p.danger, "failed"),
                    CheckState::Pending => (icon::CIRCLE_DASHED, p.warning, "running"),
                    CheckState::Neutral => (icon::MINUS_CIRCLE, p.text_faint, "neutral"),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(g).color(col)).on_hover_text(word);
                    match &c.url {
                        Some(u) => {
                            ui.hyperlink_to(&c.name, u);
                        }
                        None => {
                            ui.label(&c.name);
                        }
                    }
                });
            }
        });
        ui.add_space(8.0);
    }

    // Reviews.
    if !d.reviews.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Reviews")
                    .size(theme::META_SIZE)
                    .color(p.text_dim),
            );
            for r in &d.reviews {
                let (g, c) = match r.state.as_str() {
                    "Approved" => (icon::CHECK_CIRCLE, p.success),
                    "Changes requested" => (icon::WARNING_CIRCLE, p.danger),
                    _ => (icon::CHAT_CIRCLE, p.text_dim),
                };
                ui.label(
                    RichText::new(format!("{g} {}: {}", r.author, r.state))
                        .size(theme::META_SIZE)
                        .color(c),
                )
                .on_hover_text(&r.state);
            }
        });
        ui.add_space(12.0);
    }

    // Conversation.
    if !d.comments.is_empty() {
        widgets::section_label(ui, &format!("Conversation · {}", d.comments.len()));
        ui.add_space(6.0);
        for c in &d.comments {
            ui.horizontal_top(|ui| {
                widgets::avatar(ui, 26.0, &c.author, &c.author);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&c.author).strong());
                        ui.label(
                            RichText::new(widgets::relative_time(c.created_at, now))
                                .size(theme::SMALL_SIZE)
                                .color(p.text_faint),
                        );
                    });
                    crate::ui::notes::markdown(ui, c.body.trim());
                });
            });
            ui.add_space(10.0);
        }
    }
}

/// The comment box under the conversation.
fn comment_box(ui: &mut Ui, tab: &mut RepoTab, number: u64, actions: &mut Vec<Action>) {
    ui.add(
        egui::TextEdit::multiline(&mut tab.requests.comment)
            .hint_text("Leave a comment (Markdown)…")
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .margin(theme::FIELD_MARGIN),
    );
    ui.add_space(6.0);
    let can = !tab.requests.comment.trim().is_empty() && tab.requests.busy.is_none();
    ui.horizontal(|ui| {
        if widgets::primary_button(ui, "Comment", can, 112.0).clicked() {
            actions.push(Action::Host(HostJob::Comment(
                number,
                tab.requests.comment.trim().to_owned(),
            )));
        }
        if tab.requests.comment.trim().is_empty() {
            ui.label(
                RichText::new("Write a comment to enable")
                    .size(theme::SMALL_SIZE)
                    .color(palette(ui.ctx()).text_faint),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::Worker;
    use std::sync::Arc;

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
        crate::ui::theme::apply(&ctx, crate::ui::theme::ThemeId::Midnight, true);
        let (worker, _, _) = Worker::detached();
        let mut tab = RepoTab::with_worker("fixture".into(), 100, Default::default(), worker);
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
    fn request_shortcuts_use_the_displayed_state_after_details_refresh() {
        let (worker, _, _) = Worker::detached();
        let mut tab = RepoTab::with_worker("fixture".into(), 100, Default::default(), worker);
        tab.requests.selected = Some(7);
        tab.requests.list = Some(Ok(Arc::new(vec![summary(RequestState::Open)])));
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
