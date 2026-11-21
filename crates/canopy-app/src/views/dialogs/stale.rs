//! Repository-bound stale cleanup preview and destructive confirmation.

use std::path::PathBuf;

use canopy_core::{StaleCandidate, StaleKind};

use super::*;
use crate::ui::kit::overlay::TooltipExt;

/// The repository and preview this form asks the user to remove from.
#[derive(Debug, Clone)]
pub struct StaleForm {
    pub repo: PathBuf,
    older_than_days: u32,
    serial: u64,
    selected: Vec<bool>,
}

impl StaleForm {
    pub fn new(repo: PathBuf, older_than_days: u32, serial: u64) -> Self {
        Self {
            repo,
            older_than_days,
            serial,
            selected: Vec::new(),
        }
    }

    fn counts(&self, candidates: &[StaleCandidate]) -> (usize, usize) {
        candidates
            .iter()
            .zip(&self.selected)
            .filter(|(candidate, selected)| **selected && candidate.protected.is_none())
            .fold(
                (0, 0),
                |(worktrees, branches), (candidate, _)| match candidate.kind {
                    StaleKind::Worktree => (worktrees + 1, branches),
                    StaleKind::Branch => (worktrees, branches + 1),
                },
            )
    }

    fn selection(&self, candidates: &[StaleCandidate]) -> Vec<StaleCandidate> {
        candidates
            .iter()
            .zip(&self.selected)
            .filter(|(candidate, selected)| **selected && candidate.protected.is_none())
            .map(|(candidate, _)| candidate.clone())
            .collect()
    }
}

/// Space for visible candidate rows before the list scrolls.
const CANDIDATE_LIST_HEIGHT: f32 = 240.0;

/// The row's one line of detail, at a fixed height so long names and paths cannot defeat list
/// virtualization. The tooltip holds everything the line leaves out.
fn detail(ui: &mut Ui, text: &str, tip: &str, color: egui::Color32) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(theme::metrics().meta_size)
                .color(color),
        )
        .truncate(),
    )
    .kit_tooltip(tip)
    .show();
}

fn candidate_row(
    ui: &mut Ui,
    candidate: &StaleCandidate,
    selected: &mut bool,
    index: usize,
    busy: bool,
) -> bool {
    let mut changed = false;
    let p = palette(ui.ctx());
    let enabled = !busy && candidate.protected.is_none();
    let kind = match candidate.kind {
        StaleKind::Worktree => "Worktree",
        StaleKind::Branch => "Branch",
    };
    let label = format!("{kind}: {}", candidate.name);
    ui.horizontal(|ui| {
        let mut checkbox = Checkbox::new(selected, "")
            .enabled(enabled)
            .tooltip(&label)
            .id_salt(("stale-candidate", index));
        if let Some(reason) = &candidate.protected {
            checkbox = checkbox.disabled_reason(reason);
        }
        let response = checkbox.show(ui);
        changed |= response.changed();
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *selected, &label)
        });
        if !enabled {
            crate::ui::kit::a11y::unavailable(&response, candidate.protected.as_deref());
        }
        let response = ui
            .add(
                egui::Label::new(RichText::new(&label).color(if enabled {
                    p.text
                } else {
                    p.text_dim
                }))
                .truncate()
                .sense(egui::Sense::click()),
            )
            .kit_tooltip(&label)
            .show();
        if enabled && response.clicked() {
            *selected = !*selected;
            changed = true;
        }
    });
    // One line of detail, as in Clean up merged: why the row is protected, or else when and by
    // whom it was last committed to and, for a worktree, its folder, shortened in the middle.
    let facts = format!(
        "{} · {}",
        crate::ui::widgets::absolute_date(candidate.last_commit_time),
        candidate.author.name
    );
    let path = candidate
        .path
        .as_ref()
        .map(|path| path.display().to_string());
    let full = path
        .as_ref()
        .map_or_else(|| facts.clone(), |path| format!("{facts} · {path}"));
    if let Some(reason) = &candidate.protected {
        detail(ui, reason, &format!("{reason}\n{full}"), p.warning);
    } else if let Some(path) = &path {
        let font = FontId::proportional(theme::metrics().meta_size);
        let lead = format!("{facts} · ");
        let room = ui.available_width() - crate::ui::widgets::text_width(ui, &lead, &font);
        let shown = format!(
            "{lead}{}",
            crate::ui::widgets::elide_middle(ui, path, &font, room)
        );
        detail(ui, &shown, &full, p.text_dim);
    } else {
        detail(ui, &facts, &full, p.text_dim);
    }
    changed
}

pub(super) fn show(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut StaleForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let tab = tab.filter(|tab| tab.path == form.repo && tab.stale_serial == form.serial);
    let candidates = tab
        .and_then(|tab| tab.stale_result.as_ref())
        .and_then(|result| result.as_ref().ok());
    if let Some(candidates) = candidates {
        form.selected.resize(candidates.len(), false);
    }
    let busy = tab.is_none_or(|tab| tab.running.is_some());
    let counts = candidates.map_or((0, 0), |candidates| form.counts(candidates));
    let changed = std::cell::Cell::new(false);
    let result = KitDialog::new(salt).title("Clean up stale worktrees and branches").icon(icon::TRASH)
        .width(DialogWidth::Form).destructive().initial_focus(InitialFocus::Cancel)
        .body(|ui| {
            note(ui, format!("Last commit older than {} days. Change the period in Settings → Git.", form.older_than_days), p.text_dim);
            let Some(tab) = tab else {
                note(ui, "This preview is no longer available. Reopen cleanup from the repository.", p.danger);
                return;
            };
            match &tab.stale_result {
                None => working(ui, "Checking worktrees and branches…"),
                Some(Err(error)) => note(ui, error, p.danger),
                Some(Ok(candidates)) if candidates.is_empty() => note(ui, "No stale worktrees or branches.", p.text_dim),
                Some(Ok(candidates)) => {
                    let m = theme::metrics();
                    let detail_height = ui.fonts_mut(|fonts| fonts.row_height(&FontId::proportional(m.meta_size)));
                    // The checkbox line and one line of detail, with a gap between rows: the row
                    // Clean up merged uses.
                    let row_height = m.control_sm + detail_height + theme::SM;
                    egui::ScrollArea::vertical().id_salt(("stale-candidates", form.serial))
                        .max_height(CANDIDATE_LIST_HEIGHT).show(ui, |ui| {
                            crate::views::commit_detail::virtual_rows(ui, candidates.len(), row_height, |ui, index| {
                                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), row_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_min_height(row_height);
                                    if candidate_row(ui, &candidates[index], &mut form.selected[index], index, busy) { changed.set(true); }
                                });
                            });
                        });
                    note(ui, "Selected worktree folders and local branch references will be removed. This cannot be undone here. Safety is checked again before removal.", p.text_dim);
                }
            }
        }).footer(|footer| {
            let (worktrees, branches) = counts;
            let worktree_word = if worktrees == 1 { "worktree" } else { "worktrees" };
            let branch_word = if branches == 1 { "branch" } else { "branches" };
            footer.cancel().confirm(format!("Remove {worktrees} {worktree_word}, {branches} {branch_word}"));
            if busy || worktrees + branches == 0 {
                footer.confirm_disabled(if busy { "Wait for repository operations to finish" } else { "Select an unprotected worktree or branch" });
            }
        }).show(ctx);
    match result {
        DialogResult::Confirm if !busy && !changed.get() && counts != (0, 0) => {
            let chosen = candidates.map_or_else(Vec::new, |candidates| form.selection(candidates));
            Outcome::Run(vec![Action::OpIn {
                repo: form.repo.clone(),
                op: Op::CleanStale {
                    candidates: chosen,
                    older_than_days: form.older_than_days,
                },
            }])
        }
        DialogResult::Cancel => Outcome::Close,
        _ => Outcome::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::testkit::spoken;

    fn candidate(kind: StaleKind, protected: Option<&str>) -> StaleCandidate {
        StaleCandidate {
            kind,
            name: "old-topic".into(),
            path: (kind == StaleKind::Worktree).then(|| PathBuf::from("old-worktree")),
            tip: Oid::ZERO_SHA1,
            last_commit_time: 1,
            author: canopy_core::Signature {
                name: "Test Author".into(),
                email: "test@example.invalid".into(),
                time: 1,
                offset_minutes: 0,
            },
            dirty: protected.is_some(),
            unmerged: 0,
            protected: protected.map(str::to_owned),
        }
    }

    #[test]
    fn cleanup_disables_protected_rows_and_names_selected_counts() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.stale_serial = 1;
        let rows = vec![
            candidate(StaleKind::Worktree, Some("Uncommitted changes")),
            candidate(StaleKind::Branch, None),
        ];
        tab.stale_result = Some(Ok(std::sync::Arc::from(rows.clone())));
        let mut form = StaleForm::new(tab.path.clone(), 90, 1);
        // Even a stale selection cannot include a protected candidate.
        form.selected = vec![true, true];
        assert_eq!(form.selection(&rows), vec![rows[1].clone()]);
        let salt = salt(&Dialog::CleanStale(Box::new(form.clone())));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes = spoken(&ctx, |_| {
                show(&ctx, salt, &mut form, Some(&tab));
            });
        }
        let protected = nodes
            .iter()
            .find(|node| {
                node.label
                    .as_deref()
                    .is_some_and(|label| label.starts_with("Worktree: old-topic"))
            })
            .expect("worktree row");
        assert!(protected.disabled);
        assert_eq!(
            protected.description.as_deref(),
            Some("Uncommitted changes")
        );
        assert_eq!(protected.label.as_deref(), Some("Worktree: old-topic"));
        let confirm = nodes
            .iter()
            .find(|node| {
                node.label
                    .as_deref()
                    .is_some_and(|label| label.starts_with("Remove 0 worktrees, 1 branch"))
            })
            .expect("confirm with counts");
        assert!(!confirm.disabled);
        tab.running = Some("Refreshing".into());
        for _ in 0..3 {
            nodes = spoken(&ctx, |_| {
                show(&ctx, salt, &mut form, Some(&tab));
            });
        }
        assert!(nodes.iter().any(|node| {
            node.disabled
                && node
                    .label
                    .as_deref()
                    .is_some_and(|label| label.starts_with("Remove 0 worktrees, 1 branch"))
        }));
    }
    #[test]
    fn stale_rows_are_as_tall_as_merged_rows() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.stale_serial = 1;
        let rows = vec![
            candidate(StaleKind::Worktree, None),
            candidate(StaleKind::Worktree, Some("Uncommitted changes")),
            candidate(StaleKind::Branch, None),
        ];
        tab.stale_result = Some(Ok(std::sync::Arc::from(rows)));
        let mut form = StaleForm::new(tab.path.clone(), 90, 1);
        let salt = salt(&Dialog::CleanStale(Box::new(form.clone())));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes = spoken(&ctx, |_| {
                show(&ctx, salt, &mut form, Some(&tab));
            });
        }
        let mut tops: Vec<f32> = nodes
            .iter()
            .filter(|node| node.role == egui::accesskit::Role::CheckBox)
            .filter_map(|node| node.bounds.map(|b| b.min.y))
            .collect();
        tops.sort_by(f32::total_cmp);
        assert_eq!(tops.len(), 3, "one checkbox per row");
        let m = theme::metrics();
        let detail = ctx.fonts_mut(|f| f.row_height(&FontId::proportional(m.meta_size)));
        // The row Clean up merged uses: the checkbox line, one line of detail, a gap.
        let merged_row = m.control_sm + detail + theme::SM;
        for pair in tops.windows(2) {
            let pitch = pair[1] - pair[0];
            assert!(
                (pitch - merged_row).abs() < 0.5,
                "row pitch {pitch} should be {merged_row}"
            );
        }
    }

    #[test]
    fn cleanup_only_lays_out_visible_rows_in_a_large_preview() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.stale_serial = 1;
        let rows: Vec<_> = (0..10_000)
            .map(|index| {
                let mut row = candidate(StaleKind::Branch, None);
                row.name = format!("old-topic-{index}");
                row
            })
            .collect();
        tab.stale_result = Some(Ok(std::sync::Arc::from(rows)));
        let mut form = StaleForm::new(tab.path.clone(), 90, 1);
        form.selected = vec![true; 10_000];
        let salt = salt(&Dialog::CleanStale(Box::new(form.clone())));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes = spoken(&ctx, |_| {
                show(&ctx, salt, &mut form, Some(&tab));
            });
        }
        let checkboxes = nodes
            .iter()
            .filter(|node| node.role == egui::accesskit::Role::CheckBox)
            .count();
        assert!(
            checkboxes > 0 && checkboxes < 20,
            "only viewport rows get widgets: {checkboxes}"
        );
        assert!(nodes.iter().any(|node| {
            node.label
                .as_deref()
                .is_some_and(|label| label.starts_with("Remove 0 worktrees, 10000 branches"))
        }));
    }
}
