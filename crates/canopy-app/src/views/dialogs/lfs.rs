//! The Git LFS dialog: the files whose objects are not downloaded, the patterns stored with LFS
//! and the locks on the LFS server. Everything it runs is the user's click: the lists come from
//! the repository thread, and the locks from the server only when the dialog opens or refreshes.

use std::path::PathBuf;

use super::*;
use crate::lfs::{LISTED_FILES, human_size};
use crate::ui::kit::row::RowKind;

/// What the dialog shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfsPage {
    Files,
    Patterns,
    Locks,
}

/// A form belongs to the repository that opened it, including after a tab switch.
#[derive(Debug, Clone)]
pub struct LfsForm {
    pub repo: PathBuf,
    page: LfsPage,
    /// The pattern to track.
    pattern: String,
    /// The file to lock.
    lock_path: String,
}

impl LfsForm {
    pub fn new(repo: PathBuf, page: LfsPage) -> Self {
        Self {
            repo,
            page,
            pattern: String::new(),
            lock_path: String::new(),
        }
    }
}

/// Space for the rows of a list before it scrolls.
const LIST_HEIGHT: f32 = 240.0;

pub(super) fn show(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut LfsForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let mut actions = Vec::new();
    let result = KitDialog::new(salt)
        .title("Git LFS")
        .icon(icon::HARD_DRIVES)
        .width(DialogWidth::Reading)
        .initial_focus(InitialFocus::Cancel)
        .body(|ui| {
            Segmented::new(
                &mut form.page,
                &[
                    (LfsPage::Files, "Files"),
                    (LfsPage::Patterns, "Tracked patterns"),
                    (LfsPage::Locks, "Locks"),
                ],
            )
            .label("Git LFS")
            .show(ui);
            ui.add_space(theme::MD);
            let Some(tab) = tab.filter(|tab| tab.path == form.repo) else {
                note(
                    ui,
                    "This repository is no longer open. Reopen it to use Git LFS.",
                    p.danger,
                );
                return;
            };
            let unavailable = tab.lfs.unavailable();
            if let Some(reason) = &unavailable {
                note(ui, reason, p.warning);
                ui.add_space(theme::SM);
            }
            let can = unavailable.is_none() && tab.running.is_none();
            let why = unavailable
                .as_deref()
                .unwrap_or("Another operation is still running");
            match form.page {
                LfsPage::Files => files(ui, tab, can, why, &mut actions),
                LfsPage::Patterns => patterns(ui, form, tab, can, why, &mut actions),
                LfsPage::Locks => locks(ui, form, tab, can, why, &mut actions),
            }
        })
        .footer(|f| {
            f.cancel().cancel_label("Close");
        })
        .show(ctx);
    match result {
        DialogResult::Cancel | DialogResult::Confirm => Outcome::Close,
        DialogResult::Open if actions.is_empty() => Outcome::Keep,
        DialogResult::Open => Outcome::RunKeep(actions),
    }
}

fn files(ui: &mut Ui, tab: &RepoTab, can: bool, why: &str, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let all = tab.lfs.files();
    if all.is_empty() {
        note(
            ui,
            "No file of this repository is stored with Git LFS.",
            p.text_dim,
        );
        return;
    }
    let missing = tab.lfs.not_downloaded();
    note(
        ui,
        format!(
            "{} file{} stored with Git LFS; {missing} not downloaded.",
            all.len(),
            if all.len() == 1 { " is" } else { "s are" }
        ),
        p.text_dim,
    );
    if missing == 0 {
        return;
    }
    note(
        ui,
        "A file that is not downloaded holds only a small pointer text. Download writes the \
         real content into the working tree (git lfs pull); Fetch objects only stores it \
         without touching your files (git lfs fetch).",
        p.text_dim,
    );
    ui.add_space(theme::SM);
    egui::ScrollArea::vertical()
        .id_salt("lfs-files")
        .max_height(LIST_HEIGHT)
        .show(ui, |ui| {
            for f in all.iter().filter(|f| !f.downloaded).take(LISTED_FILES) {
                ListRow::new(("lfs-file", &f.path))
                    .kind(RowKind::File)
                    .icon(icon::CLOUD_SLASH)
                    .label(&f.path)
                    .trailing(human_size(f.pointer.size))
                    .ground(p.s().elevated)
                    .tooltip(&f.path)
                    .show(ui);
            }
        });
    if missing > LISTED_FILES {
        note(
            ui,
            format!("and {} more", missing - LISTED_FILES),
            p.text_faint,
        );
    }
    ui.add_space(theme::SM);
    ui.horizontal(|ui| {
        let download = Button::new("Download")
            .primary()
            .icon(icon::DOWNLOAD_SIMPLE)
            .enabled(can)
            .disabled_reason(why)
            .tooltip("git lfs pull: download the objects and write them into the files")
            .show(ui);
        if download.clicked() && can {
            actions.push(Action::Op(Op::LfsPull));
        }
        let fetch = Button::new("Fetch objects only")
            .secondary()
            .enabled(can)
            .disabled_reason(why)
            .tooltip(
                "git lfs fetch: download the objects of this branch, leave the files as they are",
            )
            .show(ui);
        if fetch.clicked() && can {
            actions.push(Action::Op(Op::LfsFetch));
        }
    });
}

fn patterns(
    ui: &mut Ui,
    form: &mut LfsForm,
    tab: &RepoTab,
    can: bool,
    why: &str,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    match &tab.lfs.patterns {
        None => working(ui, "Reading the tracked patterns…"),
        Some(Err(error)) => note(ui, error, p.danger),
        Some(Ok(list)) if list.is_empty() => {
            note(ui, "No pattern is stored with Git LFS yet.", p.text_dim);
        }
        Some(Ok(list)) => {
            egui::ScrollArea::vertical()
                .id_salt("lfs-patterns")
                .max_height(LIST_HEIGHT)
                .show(ui, |ui| {
                    for pattern in list {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&pattern.pattern)
                                    .font(FontId::monospace(theme::metrics().mono_size))
                                    .color(p.text),
                            );
                            ui.label(
                                RichText::new(&pattern.source)
                                    .size(theme::metrics().meta_size)
                                    .color(p.text_faint),
                            );
                            if pattern.lockable {
                                ui.label(
                                    RichText::new("lockable")
                                        .size(theme::metrics().meta_size)
                                        .color(p.text_dim),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let r = Button::new("Untrack")
                                        .ghost()
                                        .size(Size::Sm)
                                        .enabled(can)
                                        .disabled_reason(why)
                                        .tooltip("Remove the pattern from .gitattributes; committed files stay pointers")
                                        .show(ui);
                                    if r.clicked() && can {
                                        actions.push(Action::Op(Op::LfsUntrack(
                                            pattern.pattern.clone(),
                                        )));
                                    }
                                },
                            );
                        });
                    }
                });
        }
    }
    ui.add_space(theme::MD);
    field(
        ui,
        "Track a pattern",
        &mut form.pattern,
        "e.g. *.psd or assets/**",
        false,
    );
    let ready = !form.pattern.trim().is_empty() && can;
    let reason = if can { "Enter a pattern" } else { why };
    let track = Button::new("Track")
        .primary()
        .enabled(ready)
        .disabled_reason(reason)
        .tooltip("git lfs track: store the matching files with Git LFS from the next commit")
        .show(ui);
    if track.clicked() && ready {
        actions.push(Action::Op(Op::LfsTrack {
            pattern: form.pattern.trim().to_owned(),
            literal: false,
        }));
        form.pattern.clear();
    }
}

fn locks(
    ui: &mut Ui,
    form: &mut LfsForm,
    tab: &RepoTab,
    can: bool,
    why: &str,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    note(
        ui,
        "Locks live on the Git LFS server. This list asks it each time it opens or refreshes.",
        p.text_dim,
    );
    ui.add_space(theme::SM);
    match &tab.lfs.locks {
        None => working(ui, "Asking the LFS server for its locks…"),
        Some(Err(error)) => note(ui, error, p.danger),
        Some(Ok(list)) if list.is_empty() => note(ui, "No file is locked.", p.text_dim),
        Some(Ok(list)) => {
            egui::ScrollArea::vertical()
                .id_salt("lfs-locks")
                .max_height(LIST_HEIGHT)
                .show(ui, |ui| {
                    for lock in list {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&lock.path).color(p.text))
                                    .truncate(),
                            );
                            ui.label(
                                RichText::new(format!("locked by {}", lock.owner))
                                    .size(theme::metrics().meta_size)
                                    .color(p.text_faint),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let force = Button::new("Force unlock…")
                                        .ghost()
                                        .danger()
                                        .size(Size::Sm)
                                        .enabled(can)
                                        .disabled_reason(why)
                                        .show(ui);
                                    if force.clicked() && can {
                                        actions.push(Action::OpenDialog(Dialog::Confirm {
                                            title: format!("Force unlock {}?", lock.path),
                                            body: format!(
                                                "{} holds this lock. Releasing it by force can \
                                                 discard their claim on the file while they may \
                                                 still be changing it.",
                                                lock.owner
                                            ),
                                            confirm: "Force unlock".into(),
                                            danger: true,
                                            repo: form.repo.clone(),
                                            op: Op::LfsUnlock {
                                                path: lock.path.clone(),
                                                force: true,
                                            },
                                        }));
                                    }
                                    let unlock = Button::new("Unlock")
                                        .ghost()
                                        .size(Size::Sm)
                                        .enabled(can)
                                        .disabled_reason(why)
                                        .tooltip("Release your lock on this file")
                                        .show(ui);
                                    if unlock.clicked() && can {
                                        actions.push(Action::Op(Op::LfsUnlock {
                                            path: lock.path.clone(),
                                            force: false,
                                        }));
                                    }
                                },
                            );
                        });
                    }
                });
        }
    }
    ui.add_space(theme::MD);
    field(
        ui,
        "Lock a file",
        &mut form.lock_path,
        "e.g. assets/model.psd",
        false,
    );
    let ready = !form.lock_path.trim().is_empty() && can;
    let reason = if can { "Enter a file path" } else { why };
    ui.horizontal(|ui| {
        let lock = Button::new("Lock")
            .primary()
            .icon(icon::LOCK_SIMPLE)
            .enabled(ready)
            .disabled_reason(reason)
            .show(ui);
        if lock.clicked() && ready {
            actions.push(Action::Op(Op::LfsLock(form.lock_path.trim().to_owned())));
            form.lock_path.clear();
        }
        if Button::new("Refresh")
            .secondary()
            .icon(icon::ARROWS_CLOCKWISE)
            .show(ui)
            .clicked()
        {
            actions.push(Action::LfsLists);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use canopy_core::{LfsFile, LfsLock, LfsPattern, LfsPointer, MissingTool};

    use super::*;
    use crate::ui::kit::testkit::{Spoken, spoken};
    use crate::worker::LfsInfo;

    fn tab() -> RepoTab {
        let (worker, _, _) = crate::worker::Worker::detached();
        RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        )
    }

    fn draw(form: &mut LfsForm, tab: &RepoTab) -> Vec<String> {
        draw_nodes(form, tab)
            .into_iter()
            .filter_map(|s| s.label)
            .collect()
    }

    fn draw_nodes(form: &mut LfsForm, tab: &RepoTab) -> Vec<Spoken> {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let salt = salt(&Dialog::Lfs(Box::new(form.clone())));
        let mut said = Vec::new();
        for _ in 0..3 {
            said = spoken(&ctx, |_| {
                show(&ctx, salt, form, Some(tab));
            });
        }
        said
    }

    fn file(path: &str, downloaded: bool) -> LfsFile {
        LfsFile {
            path: path.into(),
            pointer: LfsPointer {
                oid: "a".repeat(64),
                size: 2048,
                extensions: false,
            },
            downloaded,
        }
    }

    #[test]
    fn every_page_draws_with_and_without_its_lists() {
        let mut tab = tab();
        tab.lfs.set(Arc::new(LfsInfo {
            in_use: true,
            files: vec![file("a.bin", false), file("b.bin", true)],
            missing: None,
        }));
        let serial = tab.lfs.next_patterns_serial();
        tab.lfs.accept_patterns(
            serial,
            Ok(vec![LfsPattern {
                pattern: "*.bin".into(),
                source: ".gitattributes".into(),
                lockable: false,
            }]),
        );
        let serial = tab.lfs.next_locks_serial();
        tab.lfs.accept_locks(
            serial,
            Ok(vec![LfsLock {
                id: "1".into(),
                path: "a.bin".into(),
                owner: "ada".into(),
            }]),
        );
        for page in [LfsPage::Files, LfsPage::Patterns, LfsPage::Locks] {
            let mut form = LfsForm::new(PathBuf::from("fixture"), page);
            draw(&mut form, &tab);
        }
        let said = draw(
            &mut LfsForm::new(PathBuf::from("fixture"), LfsPage::Files),
            &tab,
        );
        assert!(said.iter().any(|s| s.contains("Download")), "{said:?}");
    }

    #[test]
    fn without_git_lfs_the_actions_are_disabled() {
        let mut tab = tab();
        tab.lfs.set(Arc::new(LfsInfo {
            in_use: true,
            files: vec![file("a.bin", false)],
            missing: Some(MissingTool {
                program: "git-lfs".into(),
                needed_for: "Git LFS files",
                install: vec!["Install git-lfs with your package manager.".into()],
            }),
        }));
        let said = draw_nodes(
            &mut LfsForm::new(PathBuf::from("fixture"), LfsPage::Files),
            &tab,
        );
        // The reason is in the dialog's text (see `LfsState::unavailable`); what a screen reader
        // hears of the actions is that they cannot run, and why. Linux's adapter does not say a
        // button is disabled, so there the name says it.
        let suffix = if cfg!(all(unix, not(target_os = "macos"))) {
            ", unavailable"
        } else {
            ""
        };
        for action in ["Download", "Fetch objects only"] {
            let name = format!("{action}{suffix}");
            let node = said
                .iter()
                .find(|s| s.label.as_deref() == Some(name.as_str()))
                .unwrap_or_else(|| panic!("{name} in {said:?}"));
            assert!(node.disabled, "{node:?}");
            assert!(
                node.description
                    .as_deref()
                    .is_some_and(|d| d.contains("git-lfs is not installed")),
                "{node:?}"
            );
        }
    }
}
