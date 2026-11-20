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
    /// The folder holds changes: the first confirmation was given, the destructive one is asked.
    deleting: bool,
}

impl RemoveWorktreeForm {
    pub fn new(repo: PathBuf, path: PathBuf, serial: u64) -> Self {
        Self {
            repo,
            path,
            serial,
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
    let blocked = removal.and_then(|r| r.blocked.clone());
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
                    if let Some(operation) = &removal.operation {
                        note(
                            ui,
                            format!("A {operation} is in progress in this folder; removing it drops that state."),
                            p.warning,
                        );
                    }
                    loss(ui, &format!("{} deleted:", plural(removal.modified.count, "modified tracked file", "modified tracked files")), &removal.modified, p.danger);
                    loss(ui, &format!("{} deleted:", plural(removal.untracked.count, "untracked file", "untracked files")), &removal.untracked, p.danger);
                    loss(ui, &format!("{} deleted without asking:", plural(removal.ignored.count, "ignored file or folder", "ignored files and folders")), &removal.ignored, p.warning);
                    if removal.unreachable_commits > 0 {
                        ui.add_space(theme::XS);
                        Callout::new(
                            Tone::Danger,
                            format!(
                                "{} reachable only from this folder's detached HEAD; once it is removed nothing points to {}.",
                                plural(removal.unreachable_commits, "commit is", "commits are"),
                                if removal.unreachable_commits == 1 { "it" } else { "them" },
                            ),
                        )
                        .show(ui);
                    }
                    if removal.changed_files() + removal.ignored.count == 0 && removal.unreachable_commits == 0 && blocked.is_none() {
                        note(ui, "No changed, untracked or ignored files.", p.text_dim);
                    }
                    ui.add_space(theme::XS);
                    note(ui, "Its HEAD reflog, config.worktree and refs/worktree/* are deleted too.", p.text_dim);
                    note(ui, "Commits only that reflog holds are lost.", p.text_dim);
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
            } else if let Some(Err(_)) = reading {
                footer.confirm_disabled("The check of the folder failed");
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
                op: Op::RemoveWorktree {
                    removal: removal.clone(),
                    open_in: Vec::new(),
                },
            }]),
            None => Outcome::Keep,
        },
        DialogResult::Cancel => Outcome::Close,
        _ => Outcome::Keep,
    }
}

/// The repository and reading this form asks the user to confirm.
#[derive(Debug, Clone)]
pub struct ForgetWorktreesForm {
    /// The repository the tab shows.
    pub repo: PathBuf,
    serial: u64,
}

impl ForgetWorktreesForm {
    pub fn new(repo: PathBuf, serial: u64) -> Self {
        Self { repo, serial }
    }
}

/// What forgetting one worktree deletes besides the working folder it no longer has.
fn forgotten(ui: &mut Ui, entry: &canopy_core::ForgottenWorktree) {
    let p = palette(ui.ctx());
    ui.add_space(theme::XS);
    note(ui, entry.path.display().to_string(), p.text);
    let on = match (&entry.branch, entry.head) {
        (Some(branch), _) => format!("on {branch}, which stays"),
        (None, Some(head)) => format!("detached at {}", short_id(head)),
        (None, None) => "nothing checked out".to_owned(),
    };
    note(ui, format!("    {on}"), p.text_dim);
    if !entry.reason.is_empty() {
        note(ui, format!("    {}", entry.reason), p.text_dim);
    }
    if entry.unreachable_commits > 0 {
        note(
            ui,
            format!(
                "    {} that no branch, remote branch or tag contains: nothing points to {} once its HEAD is deleted",
                plural(entry.unreachable_commits, "commit", "commits"),
                if entry.unreachable_commits == 1 {
                    "it"
                } else {
                    "them"
                }
            ),
            p.danger,
        );
    }
    if let Some(operation) = &entry.operation {
        note(
            ui,
            format!("    a {operation} in progress is lost"),
            p.danger,
        );
    }
    if entry.submodule_repositories {
        note(
            ui,
            "    its submodule repositories are deleted, with any commits in them that were never pushed",
            p.danger,
        );
    }
}

pub(super) fn show_forget(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &ForgetWorktreesForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let tab = tab.filter(|t| t.path == form.repo && t.worktree_forget_serial == form.serial);
    let reading = tab.and_then(|t| t.worktree_forget.as_ref());
    let forget = reading.and_then(|r| r.as_ref().ok());
    let busy = tab.is_none_or(|t| t.running.is_some());
    let blocked = forget.and_then(|f| {
        if f.entries.is_empty() {
            Some("Git would forget no worktree: none is missing now.".to_owned())
        } else {
            f.blocked.clone()
        }
    });
    let count = forget.map_or(0, |f| f.entries.len());
    let result = KitDialog::new(salt)
        .title("Forget missing worktrees?")
        .icon(icon::TRASH)
        .width(DialogWidth::Form)
        .destructive()
        .initial_focus(InitialFocus::Cancel)
        .body(|ui| {
            if tab.is_none() {
                note(ui, "This is no longer available. Open it again from the worktree.", p.danger);
                return;
            }
            match reading {
                None => working(ui, "Asking git which worktrees it would forget…"),
                Some(Err(error)) => note(ui, format!("The check failed: {error}"), p.danger),
                Some(Ok(forget)) => {
                    if let Some(reason) = &blocked {
                        Callout::new(Tone::Warning, reason.as_str()).show(ui);
                    }
                    note(
                        ui,
                        "Removes git's record of worktrees whose folders are gone. No folder is deleted and their branches stay. Commits that only such a worktree's HEAD log or its own refs hold are lost.",
                        p.text,
                    );
                    for entry in &forget.entries {
                        forgotten(ui, entry);
                    }
                }
            }
        })
        .footer(|footer| {
            // A number only once git has named the worktrees it would forget.
            footer.cancel().confirm(if count == 0 {
                "Forget worktrees".to_owned()
            } else {
                format!("Forget {}", plural(count, "worktree", "worktrees"))
            });
            if tab.is_none() {
                footer.confirm_disabled("This is no longer available");
            } else if busy {
                footer.confirm_disabled("Wait for repository operations to finish");
            } else if let Some(reason) = &blocked {
                footer.confirm_disabled(reason.clone());
            } else if let Some(Err(_)) = reading {
                footer.confirm_disabled("The check of the worktrees failed");
            } else if forget.is_none() {
                footer.confirm_disabled("Wait for the check of the worktrees");
            }
        })
        .show(ctx);
    match result {
        DialogResult::Confirm if !busy && blocked.is_none() => match forget {
            Some(forget) => Outcome::Run(vec![Action::OpIn {
                repo: form.repo.clone(),
                op: Op::ForgetWorktrees(forget.clone()),
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
            operation: None,
            fingerprint: 0,
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
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), removal.path, 1);
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

    fn entry(id: &str) -> canopy_core::ForgottenWorktree {
        canopy_core::ForgottenWorktree {
            id: id.into(),
            path: PathBuf::from("/work").join(id),
            reason: "gitdir file points to non-existent location".into(),
            gone: true,
            branch: None,
            head: None,
            unreachable_commits: 0,
            submodule_repositories: false,
            operation: None,
        }
    }

    fn forget_frames(
        ctx: &egui::Context,
        forget: Option<Result<Box<canopy_core::WorktreeForget>, String>>,
    ) -> Vec<crate::ui::kit::testkit::Spoken> {
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("/work/canopy"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        tab.worktree_forget_serial = 1;
        tab.worktree_forget = forget;
        let form = ForgetWorktreesForm::new(tab.path.clone(), 1);
        let salt = salt(&Dialog::ForgetWorktrees(Box::new(form.clone())));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes = spoken(ctx, |_| {
                show_forget(ctx, salt, &form, Some(&tab));
            });
        }
        nodes
    }

    /// The confirmation says what forgetting removes, plainly, and names each worktree that
    /// holds commits nothing else reaches.
    #[test]
    fn forgetting_names_what_it_deletes_and_the_commits_it_orphans() {
        let ctx = setup();
        let mut lonely = entry("lonely");
        lonely.unreachable_commits = 2;
        lonely.operation = Some("rebase".into());
        lonely.submodule_repositories = true;
        let forget = canopy_core::WorktreeForget {
            entries: vec![lonely, entry("plain")],
            blocked: None,
        };
        let nodes = forget_frames(&ctx, Some(Ok(Box::new(forget))));
        let said = |part: &str| {
            nodes.iter().any(|n| {
                [&n.label, &n.value]
                    .into_iter()
                    .flatten()
                    .any(|text| text.contains(part))
            })
        };
        // F23: in plain words, without git's file names.
        assert!(
            said("Removes git's record of worktrees whose folders are gone"),
            "{nodes:?}"
        );
        assert!(said("No folder is deleted and their branches stay"));
        // The worktree's own refs (refs/worktree/*, a bisect's refs) go with its entry too.
        assert!(said("HEAD log or its own refs hold are lost"));
        assert!(!said("config.worktree"), "no plumbing: {nodes:?}");
        assert!(said(
            "2 commits that no branch, remote branch or tag contains"
        ));
        assert!(said("a rebase in progress is lost"));
        assert!(said(
            "its submodule repositories are deleted, with any commits in them that were never pushed"
        ));
        assert!(!said("deletes nothing"), "no claim that nothing is deleted");
        assert!(button(&nodes, "Forget 2 worktrees").is_some_and(|n| !n.disabled));
        // A worktree without submodules says nothing about them.
        let plain = canopy_core::WorktreeForget {
            entries: vec![entry("plain")],
            blocked: None,
        };
        let nodes = forget_frames(&ctx, Some(Ok(Box::new(plain))));
        let mentions_submodules = nodes.iter().any(|n| {
            [&n.label, &n.value]
                .into_iter()
                .flatten()
                .any(|text| text.contains("submodule"))
        });
        assert!(!mentions_submodules, "{nodes:?}");
    }

    /// A confirmation left open after its tab closed says so on its button too, not that
    /// repository operations are running.
    #[test]
    fn a_forget_left_without_its_tab_says_it_is_gone() {
        let ctx = setup();
        let form = ForgetWorktreesForm::new(PathBuf::from("/work/canopy"), 1);
        let salt = salt(&Dialog::ForgetWorktrees(Box::new(form.clone())));
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes = spoken(&ctx, |_| {
                show_forget(&ctx, salt, &form, None);
            });
        }
        let confirm = button(&nodes, "Forget worktrees").expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("This is no longer available")
        );
    }

    #[test]
    fn forgetting_is_disabled_when_blocked_failed_or_empty() {
        let ctx = setup();
        let blocked = canopy_core::WorktreeForget {
            entries: vec![entry("gone")],
            blocked: Some("Git would also forget /work/lo.".into()),
        };
        let nodes = forget_frames(&ctx, Some(Ok(Box::new(blocked))));
        let confirm = button(&nodes, "Forget 1 worktree").expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("Git would also forget /work/lo.")
        );
        // With nothing to forget, or no count, the button names no number: "Forget 1
        // worktree" beside "none is missing now" contradicted itself.
        let nodes = forget_frames(&ctx, Some(Ok(Box::default())));
        let confirm = button(&nodes, "Forget worktrees").expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("Git would forget no worktree: none is missing now.")
        );
        assert!(button(&nodes, "Forget 1 worktree").is_none(), "{nodes:?}");
        let nodes = forget_frames(&ctx, None);
        let confirm = button(&nodes, "Forget worktrees").expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("Wait for the check of the worktrees")
        );
        let nodes = forget_frames(&ctx, Some(Err("git went away".into())));
        let confirm = button(&nodes, "Forget worktrees").expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("The check of the worktrees failed")
        );
    }

    /// What else the removal deletes is said plainly, and the commits only a detached `HEAD`
    /// reaches are one fact, with no filler about checks.
    #[test]
    fn the_removal_says_what_else_it_deletes() {
        let ctx = setup();
        let mut lonely = removal();
        lonely.unreachable_commits = 2;
        let tab = tab_with(&lonely);
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), lonely.path, 1);
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        let said = |part: &str| {
            nodes.iter().any(|n| {
                [&n.label, &n.value]
                    .into_iter()
                    .flatten()
                    .any(|text| text.contains(part))
            })
        };
        assert!(
            said("HEAD reflog, config.worktree and refs/worktree"),
            "{nodes:?}"
        );
        assert!(said("Commits only that reflog holds are lost"));
        assert!(said(
            "2 commits are reachable only from this folder's detached HEAD"
        ));
        assert!(!said("Safety is checked again"));
    }

    #[test]
    fn a_clean_folder_asks_once() {
        let ctx = setup();
        let mut clean = removal();
        clean.modified = WorktreeLoss::default();
        clean.untracked = WorktreeLoss::default();
        let tab = tab_with(&clean);
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), clean.path, 1);
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        assert!(button(&nodes, "Remove worktree").is_some_and(|n| !n.disabled));
        assert!(button(&nodes, "Continue").is_none());
    }

    #[test]
    fn a_failed_check_says_it_failed_and_a_rebase_in_progress_is_named() {
        let ctx = setup();
        let mut busy = removal();
        busy.operation = Some("rebase".into());
        let tab = tab_with(&busy);
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), busy.path.clone(), 1);
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        assert!(nodes.iter().any(|n| {
            [&n.label, &n.value]
                .into_iter()
                .flatten()
                .any(|t| t.contains("A rebase is in progress in this folder"))
        }));

        let (worker, _, _) = crate::worker::Worker::detached();
        let mut failed = RepoTab::with_worker(
            PathBuf::from("/work/canopy"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        failed.worktree_removal_serial = 1;
        failed.worktree_removal = Some(Err("git went away".into()));
        let mut form = RemoveWorktreeForm::new(failed.path.clone(), busy.path, 1);
        let (nodes, _) = frames(&ctx, &mut form, &failed);
        let confirm = nodes
            .iter()
            .find(|n| {
                n.role == egui::accesskit::Role::Button
                    && n.label
                        .as_deref()
                        .is_some_and(|l| l.starts_with("Remove worktree"))
            })
            .expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("The check of the folder failed")
        );
    }

    #[test]
    fn a_blocked_folder_cannot_be_confirmed() {
        let ctx = setup();
        let removal = removal();
        let mut locked = removal;
        // Core says why: locked, open in another tab, a submodule, another repository inside.
        locked.blocked = Some("It is locked (on a USB drive).".into());
        let tab = tab_with(&locked);
        let mut form = RemoveWorktreeForm::new(tab.path.clone(), locked.path, 1);
        let (nodes, _) = frames(&ctx, &mut form, &tab);
        let confirm = button(&nodes, "Continue").expect("confirm");
        assert!(confirm.disabled);
        assert_eq!(
            confirm.description.as_deref(),
            Some("It is locked (on a USB drive).")
        );
    }
}
