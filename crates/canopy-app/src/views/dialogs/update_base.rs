//! Update from base: bring what the base branch gained into the checked-out branch.

use std::path::PathBuf;

use canopy_core::PullStrategy;

use super::*;
use crate::settings::PullMode;

/// The repository and comparison this form asks the user to act on.
#[derive(Debug, Clone)]
pub struct UpdateBaseForm {
    pub repo: PathBuf,
    /// The base, as `origin/main`.
    pub base: String,
    serial: u64,
    /// How to combine diverged branches; the recommendation until the user picks.
    mode: PullMode,
}

impl UpdateBaseForm {
    pub fn new(repo: PathBuf, base: String, serial: u64) -> Self {
        Self {
            repo,
            base,
            serial,
            mode: PullMode::RECOMMENDED_WHEN_DIVERGED,
        }
    }
}

/// What the update does for `preview`: `None` when there is nothing to bring in.
fn strategy_for(preview: &canopy_core::PullPreview, mode: PullMode) -> Option<PullStrategy> {
    if preview.behind == 0 {
        None
    } else if preview.ahead == 0 {
        Some(PullStrategy::FastForwardOnly)
    } else if mode == PullMode::Rebase {
        Some(PullStrategy::Rebase)
    } else {
        Some(PullStrategy::Merge)
    }
}

fn commits(n: usize) -> String {
    if n == 1 {
        "1 commit".to_owned()
    } else {
        format!("{n} commits")
    }
}

pub(super) fn show(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut UpdateBaseForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let tab = tab.filter(|t| t.path == form.repo && t.base_serial == form.serial);
    let branch = tab
        .and_then(|t| t.snap.as_ref())
        .map(|s| s.head.label())
        .unwrap_or_default();
    let reading = tab.and_then(|t| t.base_result.as_ref());
    let preview = reading.and_then(|r| r.as_ref().ok());
    let strategy = preview.and_then(|pv| strategy_for(pv, form.mode));
    let busy = tab.is_none_or(|t| t.running.is_some());
    let confirm_label = match strategy {
        Some(PullStrategy::Rebase) => "Rebase",
        Some(PullStrategy::Merge) => "Merge",
        _ => "Fast-forward",
    };
    let result = KitDialog::new(salt)
        .title(format!("Update {branch} from {}", form.base))
        .icon(icon::ARROW_FAT_LINES_DOWN)
        .width(DialogWidth::Form)
        .body(|ui| {
            let Some(tab) = tab else {
                note(ui, "This comparison is no longer available. Reopen it from the branch.", p.danger);
                return;
            };
            match &tab.base_result {
                None => working(ui, &format!("Comparing with {}…", form.base)),
                Some(Err(error)) => note(ui, error, p.danger),
                Some(Ok(pv)) if pv.behind == 0 => note(
                    ui,
                    format!("{branch} already has everything {} has.", form.base),
                    p.text_dim,
                ),
                Some(Ok(pv)) => {
                    let ahead = if pv.ahead == 0 {
                        String::new()
                    } else {
                        format!(" and has {} of its own", commits(pv.ahead))
                    };
                    note(
                        ui,
                        format!(
                            "{} has {} that {branch} lacks{ahead}. Canopy compares with {} as last fetched; fetch first to see newer commits.",
                            form.base,
                            commits(pv.behind),
                            form.base
                        ),
                        p.text_dim,
                    );
                    if pv.diverged() {
                        ui.add_space(theme::SM);
                        let options = [
                            (
                                PullMode::Rebase,
                                diverged_label(PullMode::Rebase),
                                Some("Replays your commits on top of the base, with new IDs."),
                            ),
                            (
                                PullMode::Merge,
                                diverged_label(PullMode::Merge),
                                Some("Joins both lines with a merge commit; every commit keeps its ID."),
                            ),
                        ];
                        RadioGroup::new(&mut form.mode, &options)
                            .label("Combine them with")
                            .id_salt("update-base-mode")
                            .show(ui);
                    } else {
                        note(ui, "Your branch only fell behind, so it moves forward without a merge commit.", p.text_dim);
                    }
                    predicted_conflicts(ui, pv);
                }
            }
        })
        .footer(|f| {
            f.cancel().confirm(confirm_label);
            if busy {
                f.confirm_disabled("Wait for repository operations to finish");
            } else if strategy.is_none() {
                f.confirm_disabled(match reading {
                    Some(Ok(_)) => "Nothing to update",
                    Some(Err(_)) => "The comparison failed",
                    None => "Comparing…",
                });
            }
        })
        .show(ctx);
    answer(result, || {
        let (Some(pv), Some(strategy)) = (preview, strategy) else {
            return Outcome::Keep;
        };
        let Some(tip) = pv.tip else {
            return Outcome::Keep;
        };
        if busy {
            return Outcome::Keep;
        }
        Outcome::Run(vec![Action::OpIn {
            repo: form.repo.clone(),
            op: Op::UpdateFromBase {
                base: form.base.clone(),
                strategy,
                tip,
            },
        }])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(ahead: usize, behind: usize) -> canopy_core::PullPreview {
        canopy_core::PullPreview {
            upstream: "origin/main".into(),
            tip: Some(Oid::ZERO_SHA1),
            ahead,
            behind,
            ..canopy_core::PullPreview::default()
        }
    }

    #[test]
    fn the_strategy_follows_what_the_two_branches_did() {
        use PullMode::{Merge, Rebase};
        assert_eq!(strategy_for(&preview(0, 0), Rebase), None);
        assert_eq!(strategy_for(&preview(3, 0), Rebase), None);
        // Behind only: a fast-forward whatever the choice.
        assert_eq!(
            strategy_for(&preview(0, 2), Rebase),
            Some(PullStrategy::FastForwardOnly)
        );
        assert_eq!(
            strategy_for(&preview(0, 2), Merge),
            Some(PullStrategy::FastForwardOnly)
        );
        // Diverged: the user's choice, Rebase being the recommended one.
        assert_eq!(
            strategy_for(&preview(1, 2), Rebase),
            Some(PullStrategy::Rebase)
        );
        assert_eq!(
            strategy_for(&preview(1, 2), Merge),
            Some(PullStrategy::Merge)
        );
        assert_eq!(
            UpdateBaseForm::new("r".into(), "origin/main".into(), 1).mode,
            Rebase
        );
    }
}
