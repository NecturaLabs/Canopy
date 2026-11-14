//! Remove a linked working folder: what the removal would delete, listed before git runs, and a
//! second, destructive confirmation when the folder holds changes.

use std::path::{Path, PathBuf};

use canopy_core::{WorktreeLoss, WorktreeRemoval};

use super::*;

/// The folder and the reading this form asks the user to confirm.
#[derive(Debug, Clone)]
pub struct RemoveWorktreeForm {
    /// The repository the tab shows.
    pub repo: PathBuf,
    /// The linked working folder to remove.
    pub path: PathBuf,
    serial: u64,
    /// The title of the other tab that has the folder open, which keeps it from being removed.
    open_in: Option<String>,
    /// The folder holds changes: the first confirmation was given, the destructive one is asked.
    deleting: bool,
}

impl RemoveWorktreeForm {
    pub fn new(repo: PathBuf, path: PathBuf, serial: u64, open_in: Option<String>) -> Self {
        Self {
            repo,
            path,
            serial,
            open_in,
            deleting: false,
        }
    }
}

impl RemoveWorktreeForm {
    /// Shows the second, destructive question straight away (for `--overlay`).
    pub fn ask_destructive(&mut self) {
        self.deleting = true;
    }
}

/// The last part of `path`, for titles.
fn folder(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One kind of loss: how many, then the first of them.
fn loss(ui: &mut Ui, heading: &str, loss: &WorktreeLoss, color: egui::Color32) {
    if loss.count == 0 {
        return;
    }
    let p = palette(ui.ctx());
    ui.add_space(theme::XS);
    note(ui, heading, color);
    for path in &loss.first {
        note(ui, format!("    {path}"), p.text_dim);
    }
    if loss.count > loss.first.len() {
        note(
            ui,
            format!("    and {} more", loss.count - loss.first.len()),
            p.text_dim,
        );
    }
}

/// The confirmation button, by what the removal deletes.
fn confirm_label(removal: &WorktreeRemoval, deleting: bool) -> String {
    match (removal.needs_force(), deleting) {
        (false, _) => "Remove worktree".into(),
        (true, false) => "Continue".into(),
        (true, true) => format!(
            "Remove and delete {}",
            plural(removal.changed_files(), "changed file", "changed files")
        ),
    }
}

pub(super) fn show(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut RemoveWorktreeForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let tab = tab.filter(|t| t.path == form.repo && t.worktree_removal_serial == form.serial);
    let reading = tab.and_then(|t| t.worktree_removal.as_ref());
    let removal = reading.and_then(|r| r.as_ref().ok());
    let busy = tab.is_none_or(|t| t.running.is_some());
    let blocked = removal.and_then(|r| {
        form.open_in
            .as_ref()
            .map(|name| format!("It is open in the tab {name}. Close that tab first."))
            .or_else(|| r.blocked.clone())
    });
    // The second step exists only for a folder that holds changes.
    let deleting = form.deleting && removal.is_some_and(|r| r.needs_force());
    let name = folder(&form.path);
    let title = match removal {
        Some(r) if deleting => format!(
            "Delete {}?",
            plural(r.changed_files(), "changed file", "changed files")
        ),
        _ => format!("Remove worktree {name}?"),
    };
    let result = KitDialog::new(salt)
        .title(title)
        .icon(icon::TRASH)
        .width(DialogWidth::Form)
        .destructive()
        .initial_focus(InitialFocus::Cancel)
        .body(|ui| {
            note(ui, form.path.display().to_string(), p.text_dim);
            if tab.is_none() {
                note(ui, "This removal is no longer available. Open it again from the worktree.", p.danger);
                return;
            }
            match reading {
                None => working(ui, "Checking the folder…"),
                Some(Err(error)) => note(ui, error, p.danger),
                Some(Ok(removal)) => {
                    if let Some(reason) = &blocked {
                        Callout::new(Tone::Warning, reason.as_str()).show(ui);
                    }
                    if deleting {
                        Callout::new(
                            Tone::Danger,
                            format!(
                                "Removing {name} deletes {} that are not in any commit. This cannot be undone.",
                                plural(removal.changed_files(), "file", "files")
                            ),
                        )
                        .show(ui);
                    }
                    ui.add_space(theme::XS);
                    match (&removal.branch, removal.head) {
                        (Some(branch), _) => note(ui, format!("The branch {branch} stays, with its commits."), p.text),
                        (None, Some(head)) => note(ui, format!("Detached at {}.", short_id(head)), p.text),
                        (None, None) => note(ui, "Nothing is checked out there.", p.text),
                    }
                    loss(ui, &format!("{} deleted:", plural(removal.modified.count, "modified tracked file", "modified tracked files")), &removal.modified, p.danger);
                    loss(ui, &format!("{} deleted:", plural(removal.untracked.count, "untracked file", "untracked files")), &removal.untracked, p.danger);
                    loss(ui, &format!("{} deleted without asking:", plural(removal.ignored.count, "ignored file or folder", "ignored files and folders")), &removal.ignored, p.warning);
                    if removal.unreachable_commits > 0 {
                        ui.add_space(theme::XS);
                        Callout::new(
                            Tone::Danger,
                            format!(
                                "{} no branch contains: only this folder's detached HEAD reaches them, and the removal deletes it. They stay in the repository until git cleans up, but nothing points to them.",
                                plural(removal.unreachable_commits, "commit", "commits")
                            ),
                        )
                        .show(ui);
                    }
                    if removal.changed_files() + removal.ignored.count == 0 && removal.unreachable_commits == 0 && blocked.is_none() {
                        note(ui, "No changed, untracked or ignored files.", p.text_dim);
                    }
                    ui.add_space(theme::XS);
                    note(ui, "Safety is checked again before the folder is removed.", p.text_dim);
                }
            }
        })
        .footer(|footer| {
            footer.cancel();
            match removal {
                Some(removal) => footer.confirm(confirm_label(removal, deleting)),
                None => footer.confirm("Remove worktree"),
            };
            if busy {
                footer.confirm_disabled("Wait for repository operations to finish");
            } else if let Some(reason) = &blocked {
                footer.confirm_disabled(reason.clone());
            } else if removal.is_none() {
                footer.confirm_disabled("Wait for the check of the folder");
            }
        })
        .show(ctx);
    match result {
        DialogResult::Confirm if !busy && blocked.is_none() => match removal {
            Some(removal) if removal.needs_force() && !deleting => {
                form.deleting = true;
                Outcome::Keep
            }
            Some(removal) => Outcome::Run(vec![Action::OpIn {
                repo: form.repo.clone(),
                op: Op::RemoveWorktree(removal.clone()),
            }]),
            None => Outcome::Keep,
        },
        DialogResult::Cancel => Outcome::Close,
        _ => Outcome::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::testkit::spoken;

    fn removal() -> WorktreeRemoval {
        WorktreeRemoval {
            path: PathBuf::from("/work/canopy-wt"),
            branch: Some("feature/auth".into()),
            head: None,
            modified: WorktreeLoss {
                count: 1,
                first: vec!["src/lib.rs".into()],
            },
            untracked: WorktreeLoss {
                count: 2,
                first: vec!["notes.md".into(), "scratch/".into()],
            },
            ignored: WorktreeLoss::default(),
            unreachable_commits: 0,
            blocked: None,
        }
    }

    /// A tab that holds `removal` as the answer for serial 1.
    fn tab_with(removal: &WorktreeRemoval) -> RepoTab {
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("/work/canopy"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.worktree_removal_serial = 1;
        tab.worktree_removal = Some(Ok(Box::new(removal.clone())));
        tab
    }

    fn frames(
        ctx: &egui::Context,
        form: &mut RemoveWorktreeForm,
        tab: &RepoTab,
    ) -> (Vec<crate::ui::kit::testkit::Spoken>, Outcome) {
        let salt = salt(&Dialog::RemoveWorktree(Box::new(form.clone())));
        let mut nodes = Vec::new();
        let mut outcome = Outcome::Keep;
        for _ in 0..3 {
            nodes = spoken(ctx, |_| {
                outcome = show(ctx, salt, form, Some(tab));
            });
        }
        (nodes, outcome)
    }

    fn setup() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn button<'a>(
        nodes: &'a [crate::ui::kit::testkit::Spoken],
        label: &str,
    ) -> Option<&'a crate::ui::kit::testkit::Spoken> {
        nodes
            .iter()
            .find(|n| n.label.as_deref().is_some_and(|l| l.starts_with(label)))
    }

    #[test]
    fn a_folder_with_changes_asks_twice_and_names_the_files_it_deletes() {
        let ctx = setup();
        let removal = removal();
        let tab = tab_with(&removal);
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), removal.path, 1, None);
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        assert!(button(&nodes, "Continue").is_some_and(|n| !n.disabled));
        assert!(button(&nodes, "Remove and delete").is_none());
        // Confirming the first question asks the second, and runs nothing yet.
        form.deleting = true;
        let (nodes, outcome) = frames(&ctx, &mut form, &tab);
        assert!(matches!(outcome, Outcome::Keep));
        assert!(
            button(&nodes, "Remove and delete 3 changed files").is_some_and(|n| !n.disabled),
            "{:?}",
            nodes.iter().map(|n| n.label.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_clean_folder_asks_once() {
        let ctx = setup();
        let mut clean = removal();
        clean.modified = WorktreeLoss::default();
        clean.untracked = WorktreeLoss::default();
        let tab = tab_with(&clean);
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), clean.path, 1, None);
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        assert!(button(&nodes, "Remove worktree").is_some_and(|n| !n.disabled));
        assert!(button(&nodes, "Continue").is_none());
    }

    #[test]
    fn a_folder_open_in_another_tab_or_blocked_cannot_be_confirmed() {
        let ctx = setup();
        let removal = removal();
        let tab = tab_with(&removal);
        let mut form = RemoveWorktreeForm::new(
            tab.path.clone(),
            removal.path.clone(),
            1,
            Some("canopy-wt".into()),
        );
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        assert!(button(&nodes, "Continue").is_some_and(|n| n.disabled));

        let mut locked = removal;
        locked.blocked = Some("It is locked (on a USB drive).".into());
        let tab = tab_with(&locked);
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), locked.path, 1, None);
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        let confirm = button(&nodes, "Continue").expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("It is locked (on a USB drive).")
        );
    }
}
