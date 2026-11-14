//! Clean up merged branches: the branches that are merged into the base, to delete in one go.

use std::collections::HashSet;
use std::path::PathBuf;

use canopy_core::MergedReason;

use super::*;
use crate::ui::kit::overlay::TooltipExt;

/// The repository and reading this form asks the user to delete from.
#[derive(Debug, Clone)]
pub struct MergedForm {
    pub repo: PathBuf,
    serial: u64,
    selected: HashSet<String>,
    /// The proven rows are selected for the first time once the reading is in.
    seeded: bool,
    /// The user's confirmation to force-delete the branches that only look merged.
    unproven_ok: bool,
}

impl MergedForm {
    pub fn new(repo: PathBuf, serial: u64) -> Self {
        Self {
            repo,
            serial,
            selected: HashSet::new(),
            seeded: false,
            unproven_ok: false,
        }
    }
}

/// One branch the dialog offers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    name: String,
    reason: MergedReason,
    /// Why it cannot be picked: git refuses to delete a branch another worktree has checked out.
    blocked: Option<String>,
}

/// The merged branches to offer, proven ones first: never the checked-out branch, the base or the
/// long-lived branches ([`canopy_core::BranchBase::protected`]).
fn candidates(tab: &RepoTab, marks: &crate::tab::MergedMarks) -> Vec<Candidate> {
    let Some(snap) = tab.snap.as_ref() else {
        return Vec::new();
    };
    let kept: &[String] = snap.workflow.base.as_ref().map_or(&[], |b| &b.protected);
    let mut out: Vec<Candidate> = marks
        .rows
        .iter()
        .filter(|b| !b.is_head && !kept.contains(&b.name))
        .filter_map(|b| {
            let reason = b.reason?;
            let blocked = snap
                .branches
                .iter()
                .find(|i| i.name == b.name)
                .and_then(|i| i.worktree.as_ref())
                .map(|w| format!("Checked out in the worktree {}", w.display()));
            Some(Candidate {
                name: b.name.clone(),
                reason,
                blocked,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.reason
            .is_proof()
            .cmp(&a.reason.is_proof())
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Selected rows that can be deleted: (proven, unproven).
fn counts(form: &MergedForm, rows: &[Candidate]) -> (usize, usize) {
    rows.iter()
        .filter(|c| c.blocked.is_none() && form.selected.contains(&c.name))
        .fold((0, 0), |(proven, unproven), c| {
            if c.reason.is_proof() {
                (proven + 1, unproven)
            } else {
                (proven, unproven + 1)
            }
        })
}

/// Space for visible rows before the list scrolls.
const LIST_HEIGHT: f32 = 260.0;

pub(super) fn show(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut MergedForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let m = theme::metrics();
    let tab = tab.filter(|t| t.path == form.repo);
    // The reading this form asked for, or a newer one.
    let marks = tab
        .filter(|t| t.merged_applied >= form.serial)
        .and_then(|t| t.merged.as_ref());
    let ready = marks.and_then(|r| r.as_ref().ok());
    let rows = match (tab, ready) {
        (Some(tab), Some(marks)) => candidates(tab, marks),
        _ => Vec::new(),
    };
    if ready.is_some() && !form.seeded {
        form.seeded = true;
        form.selected = rows
            .iter()
            .filter(|c| c.reason.is_proof() && c.blocked.is_none())
            .map(|c| c.name.clone())
            .collect();
    }
    let busy = tab.is_none_or(|t| t.running.is_some());
    let (proven, unproven) = counts(form, &rows);
    let total = proven + unproven;
    let bases = ready.map(|m| m.bases.clone()).unwrap_or_default();
    let base = ready.map(|m| m.shown().to_owned()).unwrap_or_default();
    // Read by the footer after the body may have ticked it.
    let unproven_ok = std::cell::Cell::new(form.unproven_ok);
    let result = KitDialog::new(salt)
        .title("Clean up merged branches")
        .icon(icon::GIT_MERGE)
        .width(DialogWidth::Form)
        .destructive()
        .initial_focus(InitialFocus::Cancel)
        .body(|ui| {
            let Some(tab) = tab else {
                note(ui, "This list is no longer available. Reopen it from the repository.", p.danger);
                return;
            };
            match marks {
                None if tab.snap.as_ref().is_some_and(|s| s.workflow.base.is_none()) => note(
                    ui,
                    "There is no base branch to compare with: the repository has no main, master or remote default branch.",
                    p.text_dim,
                ),
                None => working(ui, "Checking which branches are merged…"),
                Some(Err(error)) => note(ui, error, p.danger),
                Some(Ok(_)) if rows.is_empty() => note(ui, "No merged branches", p.text_dim),
                Some(Ok(_)) => {
                    note(
                        ui,
                        format!("Local branches merged into {base}. The checked-out branch, {base} and the long-lived branches are never listed."),
                        p.text_dim,
                    );
                    let detail_h = ui.fonts_mut(|f| f.row_height(&FontId::proportional(m.meta_size)));
                    let row_h = m.control_sm + detail_h + theme::SM;
                    egui::ScrollArea::vertical()
                        .id_salt(("merged-candidates", form.serial))
                        .max_height(LIST_HEIGHT)
                        .show(ui, |ui| {
                            crate::views::commit_detail::virtual_rows(ui, rows.len(), row_h, |ui, i| {
                                let c = &rows[i];
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), row_h),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_min_height(row_h);
                                        let enabled = !busy && c.blocked.is_none();
                                        let mut on = form.selected.contains(&c.name);
                                        let mut checkbox = Checkbox::new(&mut on, c.name.as_str())
                                            .enabled(enabled)
                                            .id_salt(("merged-candidate", &c.name));
                                        if let Some(why) = &c.blocked {
                                            checkbox = checkbox.disabled_reason(why);
                                        }
                                        let response = checkbox.show(ui);
                                        if response.changed() {
                                            if on {
                                                form.selected.insert(c.name.clone());
                                            } else {
                                                form.selected.remove(&c.name);
                                            }
                                        }
                                        let (text, color) = match (&c.blocked, c.reason.is_proof()) {
                                            (Some(why), _) => (why.clone(), p.warning),
                                            (None, true) => (c.reason.label().to_owned(), p.text_dim),
                                            (None, false) => (format!("{} (not proven)", c.reason.label()), p.warning),
                                        };
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&text).size(m.meta_size).color(color),
                                            )
                                            .truncate(),
                                        )
                                        .kit_tooltip(&text)
                                        .show();
                                    },
                                );
                            });
                        });
                    if unproven > 0 {
                        let words = if unproven == 1 { "branch only looks" } else { "branches only look" };
                        Callout::new(
                            Tone::Warning,
                            format!("{unproven} selected {words} merged (squash-merged or upstream gone). Git cannot prove their commits are in {base}, so they are deleted with git branch -D; the commits stay reachable only from the reflog."),
                        )
                        .title("Not proven merged")
                        .show(ui);
                        let mut confirmed = unproven_ok.get();
                        Checkbox::new(&mut confirmed, "Delete them anyway")
                            .id_salt("merged-unproven")
                            .show(ui);
                        unproven_ok.set(confirmed);
                    }
                }
            }
        })
        .footer(|f| {
            let word = if total == 1 { "branch" } else { "branches" };
            f.cancel().confirm(format!("Delete {total} {word}"));
            if busy {
                f.confirm_disabled("Wait for repository operations to finish");
            } else if total == 0 {
                f.confirm_disabled("Select a merged branch");
            } else if unproven > 0 && !unproven_ok.get() {
                f.confirm_disabled("Confirm the branches that are not proven merged");
            }
        })
        .show(ctx);
    form.unproven_ok = unproven_ok.get();
    answer(result, || {
        if busy || total == 0 || (unproven > 0 && !form.unproven_ok) {
            return Outcome::Keep;
        }
        let names = rows
            .iter()
            .filter(|c| c.blocked.is_none() && form.selected.contains(&c.name))
            .map(|c| c.name.clone())
            .collect();
        Outcome::Run(vec![Action::OpIn {
            repo: form.repo.clone(),
            op: Op::CleanMerged {
                bases: bases.clone(),
                names,
                allow_unproven: unproven > 0,
            },
        }])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab_with(branches: &[(&str, bool)], reasons: &[(&str, Option<MergedReason>)]) -> RepoTab {
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.snap = Some(std::sync::Arc::new(canopy_core::RepoSnapshot {
            branches: branches
                .iter()
                .map(|(n, head)| canopy_core::BranchInfo {
                    name: (*n).to_owned(),
                    target: None,
                    is_head: *head,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    worktree: None,
                })
                .collect(),
            workflow: canopy_core::WorkflowInfo {
                base: Some(canopy_core::BranchBase {
                    references: vec!["main".into(), "origin/main".into()],
                    protected: vec!["main".into(), "develop".into()],
                }),
                ..canopy_core::WorkflowInfo::default()
            },
            ..canopy_core::RepoSnapshot::default()
        }));
        let rows: Vec<canopy_core::MergedBranch> = reasons
            .iter()
            .map(|(n, r)| canopy_core::MergedBranch {
                name: (*n).to_owned(),
                is_head: branches.iter().any(|(b, head)| b == n && *head),
                reason: *r,
            })
            .collect();
        tab.merged = Some(Ok(crate::tab::MergedMarks::new(
            vec!["main".into(), "origin/main".into()],
            rows.into(),
        )));
        tab.merged_serial = 1;
        tab.merged_applied = 1;
        tab
    }

    #[test]
    fn never_the_checked_out_branch_the_base_or_a_long_lived_branch() {
        let tab = tab_with(
            &[
                ("main", false),
                ("develop", false),
                ("wip", true),
                ("done", false),
                ("squashed", false),
                ("open", false),
            ],
            &[
                ("main", Some(MergedReason::Reachable)),
                ("develop", Some(MergedReason::Reachable)),
                ("wip", Some(MergedReason::Reachable)),
                ("done", Some(MergedReason::Reachable)),
                ("squashed", Some(MergedReason::SquashMerged)),
                ("open", None),
            ],
        );
        let Some(Ok(marks)) = &tab.merged else {
            panic!("marks");
        };
        let rows = candidates(&tab, marks);
        let names: Vec<(&str, bool)> = rows
            .iter()
            .map(|c| (c.name.as_str(), c.reason.is_proof()))
            .collect();
        assert_eq!(names, [("done", true), ("squashed", false)]);
    }

    #[test]
    fn a_proven_row_is_selected_to_begin_with_and_an_unproven_one_needs_the_confirmation() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let tab = tab_with(
            &[("main", true), ("done", false), ("squashed", false)],
            &[
                ("done", Some(MergedReason::Reachable)),
                ("squashed", Some(MergedReason::SquashMerged)),
            ],
        );
        let mut form = MergedForm::new(tab.path.clone(), 1);
        let salt = salt(&Dialog::CleanMerged(Box::new(form.clone())));
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {
                show(&ctx, salt, &mut form, Some(&tab));
            });
            out.textures_delta.clear();
        }
        assert!(form.seeded);
        assert!(form.selected.contains("done") && !form.selected.contains("squashed"));

        form.selected.insert("squashed".into());
        let (Some(Ok(marks)), Some(snap)) = (&tab.merged, &tab.snap) else {
            panic!("marks");
        };
        let _ = snap;
        let rows = candidates(&tab, marks);
        assert_eq!(counts(&form, &rows), (1, 1));
    }

    #[test]
    fn no_merged_branches_says_so() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let tab = tab_with(&[("main", true), ("open", false)], &[("open", None)]);
        let mut form = MergedForm::new(tab.path.clone(), 1);
        let salt = salt(&Dialog::CleanMerged(Box::new(form.clone())));
        let mut texts = Vec::new();
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {
                show(&ctx, salt, &mut form, Some(&tab));
            });
            out.textures_delta.clear();
            texts = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        assert!(texts.iter().any(|t| t == "No merged branches"), "{texts:?}");
    }
}
