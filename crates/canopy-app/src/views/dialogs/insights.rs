//! Repository insights rendered exclusively from a tab's worker-loaded metrics.

use egui::{RichText, Sense, Ui};

use super::{Dialog, DialogWidth, InitialFocus, KitDialog, Outcome, answer};
use crate::action::Action;
use crate::tab::RepoTab;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::Disclosure;
use crate::ui::theme::{self, palette};
use crate::worker::Insights;

pub(super) fn show(ctx: &egui::Context, dialog: &Dialog, tab: Option<&RepoTab>) -> Outcome {
    let Dialog::Insights { repo } = dialog else {
        return Outcome::Close;
    };
    let repo = repo.clone();
    let result = KitDialog::new(super::salt(dialog))
        .title("Repository insights")
        .width(DialogWidth::Reading)
        .initial_focus(InitialFocus::Cancel)
        .body(|ui| match tab.and_then(|t| t.insights.as_ref()) {
            None => {
                Spinner::new().show(ui);
                ui.label("Loading insights…");
            }
            Some(Err(error)) => {
                ui.colored_label(palette(ctx).danger, error);
            }
            Some(Ok(data)) => body(ui, data),
        })
        .footer(|f| {
            f.cancel_label("Close").confirm("Refresh");
        })
        .show(ctx);
    answer(result, || {
        Outcome::RunKeep(vec![Action::RepositoryInsights(repo)])
    })
}

fn date(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0).map_or_else(
        || "Unknown date".into(),
        |date| date.format("%Y-%m-%d").to_string(),
    )
}

fn body(ui: &mut Ui, data: &Insights) {
    let m = theme::metrics();
    let p = palette(ui.ctx());
    let repo = &data.repository;
    ui.label(format!(
        "{} objects · {} KiB of Git objects",
        repo.objects,
        repo.size_bytes / 1024
    ));
    if let Some(commit) = &repo.last_commit {
        ui.label(format!(
            "Last commit · {} · {}",
            date(commit.committed_at),
            commit.author.name
        ))
        .kit_tooltip(&commit.summary)
        .show();
    } else {
        ui.label("No commits yet");
    }
    ui.add_space(m.section_gap);
    ui.label(RichText::new("Weekly commits").strong());
    ui.label(
        RichText::new("52 UTC weeks · current week included · all local refs and HEAD")
            .color(p.text_dim),
    );
    chart(ui, &repo.weeks);
    weekly_counts(ui, &repo.weeks);
    ui.add_space(m.section_gap);
    ui.label(RichText::new("Top authors").strong());
    if repo.authors.is_empty() {
        ui.label("No authors yet");
    }
    for author in repo.authors.iter().take(10) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(author.commits.to_string()).color(p.accent));
            ui.label(&author.name);
            ui.label(RichText::new(&author.email).color(p.text_dim));
        });
    }
    ui.add_space(m.section_gap);
    ui.label(RichText::new("Local branches").strong());
    let default = data
        .branches
        .first()
        .and_then(|b| b.default_branch.as_deref());
    ui.label(
        RichText::new(default.map_or_else(
            || "Default branch unknown".into(),
            |name| format!("Compared with {name} · upstream refs are cached"),
        ))
        .color(p.text_dim),
    );
    ui.label(
        RichText::new(
            "Unique: commits on no other local branch · stale: no commit for over 90 days",
        )
        .color(p.text_dim),
    );
    if data.branches.is_empty() {
        ui.label("No local branches yet");
    }
    branch_table(ui, &data.branches);
    ui.add_space(m.dialog_gap);
    ui.label(RichText::new("Local history only. Shallow clones omit older commits; squash-merged branches are not detected.").color(p.text_dim));
}

fn divergence(value: Option<canopy_core::Divergence>) -> String {
    value.map_or_else(
        || "—".into(),
        |value| format!("{} / {}", value.ahead, value.behind),
    )
}

fn chart(ui: &mut Ui, weeks: &[canopy_core::WeekActivity]) {
    let m = theme::metrics();
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), m.control_lg * 2.0),
        Sense::hover(),
    );
    let max = weeks.iter().map(|w| w.commits).max().unwrap_or(0);
    let width = rect.width() / crate::ui::count_f32(weeks.len().max(1));
    for (i, week) in weeks.iter().enumerate() {
        let x = rect.left() + crate::ui::count_f32(i) * width;
        let hit = egui::Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(x + width, rect.bottom()),
        );
        let height = rect.height()
            * crate::ui::count_f32(usize::try_from(week.commits).unwrap_or(usize::MAX))
            / crate::ui::count_f32(usize::try_from(max.max(1)).unwrap_or(usize::MAX));
        let bar = egui::Rect::from_min_max(
            egui::pos2(x, rect.bottom() - height),
            egui::pos2(x + (width - m.item_spacing.x / 4.0).max(1.0), rect.bottom()),
        );
        ui.painter().rect_filled(
            bar,
            0.0,
            if week.commits == 0 {
                p.text_faint
            } else {
                p.accent
            },
        );
        ui.interact(hit, ui.id().with(("week", i)), Sense::hover())
            .kit_tooltip(format!(
                "Week of {} · {} commits",
                date(week.start),
                week.commits
            ))
            .show();
    }
    ui.horizontal(|ui| {
        if let Some(first) = weeks.first() {
            ui.label(date(first.start));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("Peak: {max} commits/week"));
        });
    });
}

/// The width of `text` on one line in the body font.
fn text_width(ui: &Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
            .size()
            .x
    })
}

/// One row per branch in columns that always fit the dialog: the branch name takes what the
/// other columns leave and is cut short when it must be (its tooltip has the full name), and the
/// last commit's author, date and age are in that tooltip rather than a column of their own.
fn branch_table(ui: &mut Ui, branches: &[canopy_core::BranchMetrics]) {
    let m = theme::metrics();
    let spacing = m.item_spacing.x;
    let columns = ["Default ↑/↓", "Upstream ↑/↓", "Unique"];
    let widths: Vec<f32> = columns
        .iter()
        .map(|header| text_width(ui, header))
        .collect();
    let state_width = branches
        .iter()
        .map(|b| text_width(ui, &state_label(b)))
        .fold(text_width(ui, "State"), f32::max);
    // The scroll bar's room is kept so a long list does not squeeze the last column.
    let name_width = (ui.available_width()
        - widths.iter().sum::<f32>()
        - state_width
        - 4.0 * spacing
        - ui.spacing().scroll.allocated_width())
    .max(m.row_height * 2.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = m.item_spacing;
        egui::ScrollArea::vertical()
            .id_salt("insights-branches")
            .max_height(m.row_height * 8.0)
            .show_rows(ui, m.row_height, branches.len() + 1, |ui, rows| {
                egui::Grid::new("insights-branch-table")
                    .striped(true)
                    .start_row(rows.start)
                    .spacing(m.item_spacing)
                    .min_row_height(m.row_height)
                    .show(ui, |ui| {
                        for row in rows {
                            if row == 0 {
                                for header in
                                    ["Branch", columns[0], columns[1], columns[2], "State"]
                                {
                                    ui.label(RichText::new(header).strong());
                                }
                            } else {
                                branch_row(ui, &branches[row - 1], name_width);
                            }
                            ui.end_row();
                        }
                    });
            });
    });
}

fn branch_row(ui: &mut Ui, branch: &canopy_core::BranchMetrics, name_width: f32) {
    let last = &branch.last_commit;
    ui.scope(|ui| {
        ui.set_max_width(name_width);
        ui.add(egui::Label::new(&branch.name).truncate().selectable(false))
            .kit_tooltip(format!(
                "{}\nLast commit: {} <{}> · {} · {} days ago\n{}",
                branch.name,
                last.author.name,
                last.author.email,
                date(last.committed_at),
                branch.age_days,
                last.summary
            ))
            .show();
    });
    ui.label(divergence(branch.default_divergence));
    ui.label(divergence(branch.upstream_divergence))
        .kit_tooltip(branch.upstream.as_deref().unwrap_or("No upstream"))
        .show();
    ui.label(branch.unique_commits.to_string());
    ui.label(state_label(branch))
        .kit_tooltip(branch.merged.map_or("", canopy_core::MergedReason::label))
        .show();
}

fn weekly_counts(ui: &mut Ui, weeks: &[canopy_core::WeekActivity]) {
    let id = ui.id().with("insights-weekly-counts");
    let mut open = ui.data(|data| data.get_temp::<bool>(id).unwrap_or(false));
    Disclosure::new("weekly-counts", "Weekly counts", &mut open).show(ui, |ui| {
        for week in weeks {
            let response = ui.add(
                egui::Label::new(format!(
                    "Week of {} (UTC) · {} commits",
                    date(week.start),
                    week.commits
                ))
                .sense(Sense::focusable_noninteractive()),
            );
            if response.gained_focus() {
                response.scroll_to_me(Some(egui::Align::Center));
            }
        }
    });
    ui.data_mut(|data| data.insert_temp(id, open));
}

/// What the State column says of a branch.
fn state_label(branch: &canopy_core::BranchMetrics) -> String {
    let state = match branch.merged {
        Some(canopy_core::MergedReason::Reachable) => "Merged",
        Some(canopy_core::MergedReason::SquashMerged) => "Squash merged?",
        Some(canopy_core::MergedReason::UpstreamGone) => "Upstream gone",
        None => "—",
    };
    if branch.stale {
        format!("{state} · stale")
    } else {
        state.into()
    }
}
