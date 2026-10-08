//! Modal dialogs. Each returns the actions to run when the user confirms.

use egui::{FontId, Key, RichText, Ui};
use egui_phosphor::regular as icon;

use canopy_core::forge::{self, ForgeKind};
use canopy_core::{Oid, short_id};
use canopy_hosting::NewRequest;

use crate::action::Action;
use crate::askpass::{Prompt, PromptKind};
use crate::hosting::HostJob;
use crate::settings::{CloneProtocol, Settings};
use crate::tab::RepoTab;
use crate::ui::art;
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::choice::Segmented;
use crate::ui::kit::field::{TextArea, TextField};
use crate::ui::kit::overlay::{
    Dialog as KitDialog, DialogResult, DialogWidth, Footer, InitialFocus,
};
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{Disclosure, ListRow};
use crate::ui::kit::select::Select;
use crate::ui::kit::surface::{Callout, InsetBox};
use crate::ui::kit::toggle::{Checkbox, RadioGroup};
use crate::ui::kit::{Button, Size, Tone, paint};
use crate::ui::legal::{Document, Pending};
use crate::ui::theme::{self, palette};
use crate::worker::Op;

mod common;
pub(crate) use common::{tilde_in, valid_ref_name};
mod credential;
mod ignore;
mod reader;
mod stack;
mod start;
mod update;
mod url;

use common::{
    FolderField, alternatives, answer, beside, bullet, confirm, document, field, field_with_error,
    folder_row, home, identity_in_scope, inset_scroll, note, recent_rows, remote_name_problem,
    swallow_enter, tilde, working,
};
use credential::credential;
pub use ignore::IgnoreForm;
pub use stack::DialogStack;
pub use start::Start;
pub use url::{UrlBuilder, UrlSource};

/// Every dialog Canopy shows.
#[derive(Debug, Clone)]
pub enum Dialog {
    Ignore(Box<IgnoreForm>),
    Open {
        path: String,
        /// Recently opened repositories, offered under the field.
        recent: Vec<std::path::PathBuf>,
        /// The folder asked for, while the opener looks at it.
        pending: Option<std::path::PathBuf>,
        /// Why the last folder asked for did not open.
        error: Option<OpenFailure>,
    },
    Clone {
        builder: UrlBuilder,
        parent: String,
        pending: bool,
        /// Why the last clone from this form failed, in git's words.
        error: Option<String>,
    },
    NewRepository {
        path: String,
        ignore_template: canopy_core::IgnoreTemplate,
        /// The folder asked for, while it is created and opened.
        pending: Option<std::path::PathBuf>,
        /// Why it could not be created, in git's words.
        error: Option<String>,
    },
    NewBranch {
        name: String,
        start: Start,
        checkout: bool,
    },
    Rename {
        old: String,
        new: String,
    },
    AddSubmodule {
        url: String,
        /// The folder; empty means the one the URL suggests.
        path: String,
        branch: String,
    },
    NewTag {
        name: String,
        target: Oid,
        message: String,
    },
    Stash {
        message: String,
        untracked: bool,
    },
    Merge {
        target: String,
        rebase: bool,
        mode: canopy_core::MergeMode,
    },
    AddRemote {
        name: String,
        builder: UrlBuilder,
        /// Push the current branch after adding (publishing a local-only repository).
        publish: bool,
        /// Fetch after adding, so the remote's branches show at once.
        fetch: bool,
    },
    NewRequest {
        kind: ForgeKind,
        request: NewRequest,
        /// Branches on the remote, for the target picker.
        branches: Vec<String>,
    },
    Confirm {
        title: String,
        body: String,
        confirm: String,
        danger: bool,
        /// The repository `op` runs in: the one that asked, even after a switch to another tab.
        repo: std::path::PathBuf,
        op: Op,
    },
    /// A fast-forward pull found that the branch and its upstream both moved on: how to bring
    /// them together, and whether to pull this repository that way from now on.
    Diverged {
        /// The repository that pulled.
        repo: std::path::PathBuf,
        mode: crate::settings::PullMode,
        remember: crate::action::Remember,
        /// Whether "every repository" is offered: Canopy's setting, not the git configuration,
        /// asked for fast-forwards only.
        everywhere: bool,
        /// What the check before the pull found (empty when git refused without one).
        preview: canopy_core::PullPreview,
    },
    /// A pull would likely stop on conflicts: pull anyway (and resolve them) or not.
    PullConflicts {
        /// The repository that pulled.
        repo: std::path::PathBuf,
        strategy: canopy_core::PullStrategy,
        preview: canopy_core::PullPreview,
        /// The remote moved after the user's last answer: this asks about the new state.
        moved: bool,
    },
    About,
    Identity {
        name: String,
        email: String,
        pending: bool,
    },
    /// Removing the identity from one Git configuration file.
    ConfirmClearIdentity {
        repo: Option<std::path::PathBuf>,
        scope: canopy_core::identity::Scope,
    },
    /// Allow downloads in the partial clones of a folder: the repository's own, or its parent.
    TrustFolder {
        repo: std::path::PathBuf,
        /// The promisor remote the downloads would come from.
        remote: String,
    },
    /// What's new in an available update.
    Update(Box<crate::updater::Available>),
    /// What's new in this version (after an upgrade, or from the palette).
    WhatsNew {
        version: String,
        notes: String,
    },
    /// Canopy crashed last time; offer the local report.
    Crash(crate::logging::CrashReport),
    /// First start (or changed terms): accept the terms, then answer about crash reports. Cannot
    /// be dismissed.
    Consent {
        step: Pending,
        /// The terms changed since the user last accepted them.
        updated: bool,
        /// A document expanded inline.
        reading: Option<Document>,
    },
    /// The Terms of Service or the Privacy Policy.
    Legal(Document),
    /// Confirm a pull/merge request action (merge, close).
    ConfirmHost {
        title: String,
        body: String,
        confirm: String,
        danger: bool,
        job: HostJob,
    },
    /// A question from git or ssh: a user name, password, passphrase or host key.
    Credential {
        prompt: Prompt,
        answer: String,
        /// This prompt among every prompt so far: its answer field is its own, so a secret
        /// shown in one prompt is hidden again in the next.
        serial: u64,
    },
}

/// Why a folder did not open: the message as git or libgit2 gave it, and whether the folder is
/// simply not in a repository (the dialog then offers to create one there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFailure {
    pub message: String,
    pub not_a_repository: bool,
}

/// What a dialog wants after this frame.
pub enum Outcome {
    Keep,
    Close,
    Run(Vec<Action>),
    RunKeep(Vec<Action>),
}

/// The folder a clone of `url` would get by default: its last segment without `.git`.
fn folder_from_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let last = url.rsplit(['/', ':']).next().unwrap_or_default();
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

impl Dialog {
    /// The dialog answering a credential prompt.
    pub fn credential(prompt: Prompt) -> Self {
        static PROMPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self::Credential {
            prompt,
            answer: String::new(),
            serial: PROMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// The open-repository dialog, prefilled with the home directory. The app adds the recent
    /// repositories when it opens it.
    pub fn open() -> Self {
        Self::Open {
            path: home(),
            recent: Vec::new(),
            pending: None,
            error: None,
        }
    }

    /// The new-repository dialog with `path` filled in.
    pub fn new_repository(path: String) -> Self {
        Self::NewRepository {
            path,
            ignore_template: canopy_core::IgnoreTemplate::None,
            pending: None,
            error: None,
        }
    }

    /// The clone dialog with the default clone folder as the destination.
    pub fn clone_default(settings: &Settings) -> Self {
        Self::Clone {
            builder: UrlBuilder::new(settings),
            parent: settings.clone_parent(),
            pending: false,
            error: None,
        }
    }

    /// The add-remote dialog. The app names the remote for the repository when it opens it
    /// ([`Dialog::name_remote`]).
    pub fn add_remote(settings: &Settings) -> Self {
        Self::AddRemote {
            name: "origin".into(),
            builder: UrlBuilder::new(settings),
            publish: false,
            fetch: true,
        }
    }

    /// Suggests a name no remote of the repository has yet in an add-remote dialog: `origin`,
    /// then `upstream`, then `remote2`, `remote3`…
    pub fn name_remote(&mut self, existing: &[String]) {
        if let Self::AddRemote { name, .. } = self {
            // One more candidate than there are remotes: one of them is free.
            let free = ["origin", "upstream"]
                .into_iter()
                .map(str::to_owned)
                .chain((2..=existing.len() + 2).map(|i| format!("remote{i}")))
                .find(|n| !existing.contains(n));
            if let Some(free) = free {
                *name = free;
            }
        }
    }

    /// Turns an add-remote dialog into "Publish": add, then push the current branch.
    pub fn publishing(mut self) -> Self {
        if let Self::AddRemote { publish, .. } = &mut self {
            *publish = true;
        }
        self
    }

    /// New branch starting at `start`.
    pub fn new_branch_at(start: Start) -> Self {
        Self::NewBranch {
            name: String::new(),
            start,
            checkout: true,
        }
    }
}

/// Draws the dialog. `tab` is the active repository, for branch lists and defaults.
pub fn show(
    ctx: &egui::Context,
    dialog: &mut Dialog,
    tab: Option<&RepoTab>,
    picker: &mut crate::ui::folder_picker::FolderPicker,
    clone_running: bool,
    updater: &crate::updater::Updater,
    git_version: &str,
) -> Outcome {
    let p = palette(ctx);
    let m = theme::metrics();
    let salt = salt(dialog);
    // What a control inside the body asked for (a link, a secondary action); it wins over the
    // footer.
    let mut extra: Option<Outcome> = None;
    let outcome = match dialog {
        Dialog::Ignore(form) => return ignore::show(ctx, salt, form, tab),
        Dialog::Identity {
            name,
            email,
            pending,
        } => {
            let ok = !name.trim().is_empty() && !email.trim().is_empty();
            let missing = if name.trim().is_empty() && email.trim().is_empty() {
                "Enter a name and an email to save."
            } else if name.trim().is_empty() {
                "Enter a name to save."
            } else {
                "Enter an email to save."
            };
            let busy = *pending;
            let result = KitDialog::new(salt)
                .title("Set your Git identity")
                .icon(icon::IDENTIFICATION_CARD)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .description("Git records this name and email as the author of every new commit. Commits already made keep their authors.")
                .body(|ui| {
                    field(ui, "Name", name, "e.g. Ada Lovelace", true);
                    field(ui, "Email", email, "e.g. ada@example.com", false);
                    note(ui, "Every repository without its own identity uses it. You can change it later in Settings → Git.", p.text_dim);
                })
                .footer(|f| {
                    confirm(f, "Save globally", ok, missing);
                    f.confirm_loading(busy);
                })
                .show(ctx);
            answer(result, || {
                if !name.trim().is_empty() && !email.trim().is_empty() && !*pending {
                    Outcome::RunKeep(vec![Action::SaveIdentity {
                        repo: None,
                        scope: canopy_core::identity::Scope::Global,
                        name: name.clone(),
                        email: email.clone(),
                    }])
                } else {
                    Outcome::Keep
                }
            })
        }
        Dialog::Open {
            path,
            recent,
            pending,
            error,
        } => {
            let busy = pending.is_some();
            let ok = !path.trim().is_empty() && !busy;
            let mut chosen: Option<std::path::PathBuf> = None;
            let mut create_here = false;
            let result = KitDialog::new(salt)
                .title("Open repository")
                .icon(icon::FOLDER_OPEN)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    let start = crate::settings::expand_home(path);
                    let typed = path.clone();
                    ui.add_enabled_ui(!busy, |ui| {
                        folder_row(
                            ui,
                            picker,
                            "open",
                            path,
                            &start,
                            FolderField {
                                label: "Folder",
                                hint: "~/src/project",
                                helper: Some("Any folder inside a Git repository."),
                                error: error.as_ref().map(|e| e.message.as_str()),
                                focus: true,
                            },
                        );
                    });
                    // A new folder clears what was said about the last one.
                    if *path != typed {
                        *error = None;
                    }
                    if error.as_ref().is_some_and(|e| e.not_a_repository)
                        && Button::new("Create a new repository in this folder…")
                            .ghost()
                            .size(Size::Sm)
                            .icon(icon::PLUS)
                            .show(ui)
                            .clicked()
                    {
                        create_here = true;
                    }
                    if let Some(asked) = pending.as_deref() {
                        working(ui, &format!("Opening {}…", tilde(asked)));
                    }
                    if !recent.is_empty() {
                        ui.add_space(theme::XS);
                        ui.add_enabled_ui(!busy, |ui| {
                            chosen = recent_rows(ui, recent);
                        });
                    }
                    alternatives(ui, &mut extra);
                })
                .footer(|f| {
                    confirm(f, "Open", ok, "Enter a folder");
                    f.confirm_loading(busy);
                })
                .show(ctx);
            if create_here {
                Outcome::Run(vec![Action::OpenDialog(Dialog::new_repository(
                    path.trim().to_owned(),
                ))])
            } else if let Some(folder) = chosen {
                *path = tilde(&folder);
                *error = None;
                *pending = Some(folder.clone());
                Outcome::RunKeep(vec![Action::OpenRepo(folder)])
            } else {
                answer(result, || {
                    if path.trim().is_empty() || pending.is_some() {
                        Outcome::Keep
                    } else {
                        let folder = std::path::PathBuf::from(path.trim());
                        *error = None;
                        *pending = Some(folder.clone());
                        Outcome::RunKeep(vec![Action::OpenRepo(folder)])
                    }
                })
            }
        }
        Dialog::Clone {
            builder,
            parent,
            pending,
            error,
        } => {
            let busy = *pending;
            let mut target: Option<(String, std::path::PathBuf)> = None;
            let resolved = clone_target(builder, parent);
            let ok = resolved.is_some() && !clone_running;
            let result = KitDialog::new(salt)
                .title("Clone repository")
                .icon(icon::CLOUD_ARROW_DOWN)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        builder.ui(ui, true);
                        let start = crate::settings::expand_home(parent);
                        let into = clone_target(builder, parent)
                            .map(|(_, dest)| format!("Creates {}", tilde(&dest)));
                        folder_row(
                            ui,
                            picker,
                            "clone",
                            parent,
                            &start,
                            FolderField {
                                label: "Clone into",
                                hint: "~/src",
                                helper: Some(
                                    into.as_deref()
                                        .unwrap_or("The repository gets a folder of its own here."),
                                ),
                                error: None,
                                focus: false,
                            },
                        );
                        target = clone_target(builder, parent);
                    });
                    // A changed repository or folder clears what was said about the last try.
                    if target != resolved {
                        *error = None;
                    }
                    if let Some(message) = error.as_deref() {
                        Callout::new(Tone::Danger, message)
                            .title("The clone failed")
                            .show(ui);
                    }
                    if busy {
                        working(ui, "Cloning repository…");
                    }
                })
                .footer(|f| {
                    if busy {
                        f.cancel_label("Continue in background");
                    } else {
                        confirm(
                            f,
                            "Clone",
                            ok,
                            if clone_running {
                                "Another clone is running"
                            } else {
                                "Enter a repository"
                            },
                        );
                    }
                })
                .show(ctx);
            answer(result, || match target {
                Some((url, dest)) if !*pending && !clone_running => {
                    *pending = true;
                    *error = None;
                    Outcome::RunKeep(vec![Action::Clone { url, dest }])
                }
                _ => Outcome::Keep,
            })
        }
        Dialog::NewRepository {
            path,
            ignore_template,
            pending,
            error,
        } => {
            let busy = pending.is_some();
            let ok = !path.trim().is_empty() && !busy;
            let result = KitDialog::new(salt)
                .title("New repository")
                .icon(icon::PLANT)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .description("Runs git init in the folder, creating it if needed, and opens the repository in a tab.")
                .body(|ui| {
                    let start = crate::settings::expand_home(path)
                        .parent()
                        .map_or_else(|| crate::settings::expand_home(&home()), std::path::Path::to_path_buf);
                    let typed = path.clone();
                    ui.add_enabled_ui(!busy, |ui| {
                        folder_row(
                            ui,
                            picker,
                            "init",
                            path,
                            &start,
                            FolderField {
                                label: "Folder",
                                hint: "~/src/my-project",
                                helper: Some("A new or empty folder."),
                                error: error.as_deref(),
                                focus: true,
                            },
                        );
                    });
                    if *path != typed {
                        *error = None;
                    }
                    let templates = canopy_core::IgnoreTemplate::ALL.map(|template| (template, template.label()));
                    Select::new(ignore_template, &templates).label(".gitignore template")
                        .enabled(!busy).show(ui);
                    if !ignore_template.patterns().is_empty() {
                        note(ui, ignore_template.patterns().join(" · "), p.text_dim);
                    }
                    if let Some(asked) = pending.as_deref() {
                        working(ui, &format!("Creating {}…", tilde(asked)));
                    }
                })
                .footer(|f| {
                    confirm(f, "Create repository", ok, "Enter a folder");
                    f.confirm_loading(busy);
                })
                .show(ctx);
            answer(result, || {
                if path.trim().is_empty() || pending.is_some() {
                    Outcome::Keep
                } else {
                    let folder = crate::settings::expand_home(path.trim());
                    *error = None;
                    *pending = Some(folder.clone());
                    Outcome::RunKeep(vec![Action::Init(folder, *ignore_template)])
                }
            })
        }
        Dialog::NewBranch {
            name,
            start,
            checkout,
        } => {
            let ok = valid_ref_name(name.trim());
            let from = start.label(|| {
                tab.and_then(|t| t.snap.as_ref())
                    .map_or_else(|| "HEAD".into(), |s| s.head.label())
            });
            let result = KitDialog::new(salt)
                .title("New branch")
                .icon(icon::GIT_BRANCH)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    let error = (!name.is_empty() && !valid_ref_name(name.trim()))
                        .then_some("Not a valid branch name");
                    field_with_error(ui, "Name", name, "feature/something", true, error);
                    note(
                        ui,
                        format!("{}  Starts at {from}", icon::GIT_COMMIT),
                        p.text_faint,
                    );
                    Checkbox::new(checkout, "Check out after creating").show(ui);
                })
                .footer(|f| confirm(f, "Create branch", ok, "Enter a valid branch name"))
                .show(ctx);
            answer(result, || {
                if valid_ref_name(name.trim()) {
                    Outcome::Run(vec![Action::Op(Op::CreateBranch {
                        name: name.trim().to_owned(),
                        start: start.rev(),
                        checkout: *checkout,
                    })])
                } else {
                    Outcome::Keep
                }
            })
        }
        Dialog::Rename { old, new } => {
            let valid = |new: &str| valid_ref_name(new.trim()) && new.trim() != old;
            let ok = valid(new);
            let result = KitDialog::new(salt)
                .title(format!("Rename {old}"))
                .icon(icon::PENCIL_SIMPLE)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .description("The remote branch it tracks keeps its name.")
                .body(|ui| {
                    let error = (!new.trim().is_empty() && !valid_ref_name(new.trim()))
                        .then_some("Not a valid branch name");
                    field_with_error(ui, "New name", new, old.as_str(), true, error);
                })
                .footer(|f| confirm(f, "Rename", ok, "Enter a new, valid branch name"))
                .show(ctx);
            answer(result, || {
                if valid(new) {
                    Outcome::Run(vec![Action::Op(Op::RenameBranch {
                        old: old.clone(),
                        new: new.trim().to_owned(),
                    })])
                } else {
                    Outcome::Keep
                }
            })
        }
        Dialog::AddSubmodule { url, path, branch } => {
            let folder = if path.trim().is_empty() {
                folder_from_url(url)
            } else {
                path.trim().to_owned()
            };
            let problem = canopy_core::submodule_add_problem(url, &folder);
            let result = KitDialog::new(salt)
                .title("Add submodule")
                .icon(icon::PACKAGE)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    let url_error = (!url.trim().is_empty())
                        .then(|| canopy_core::submodule_add_problem(url, "x"))
                        .flatten();
                    field_with_error(
                        ui,
                        "URL",
                        url,
                        "https://github.com/owner/repo.git",
                        true,
                        url_error,
                    );
                    let path_error = (!path.trim().is_empty() && url_error.is_none())
                        .then_some(problem)
                        .flatten();
                    let hint = if folder.is_empty() {
                        "libs/name"
                    } else {
                        folder.as_str()
                    };
                    field_with_error(ui, "Folder", path, hint, false, path_error);
                    field_with_error(
                        ui,
                        "Branch (optional)",
                        branch,
                        "Follow the default branch",
                        false,
                        None,
                    );
                })
                .footer(|f| {
                    confirm(
                        f,
                        "Add submodule",
                        problem.is_none(),
                        problem.unwrap_or("Enter a URL"),
                    );
                })
                .show(ctx);
            answer(result, || {
                if problem.is_some() {
                    Outcome::Keep
                } else {
                    Outcome::Run(vec![Action::Op(Op::SubmoduleAdd {
                        url: url.trim().to_owned(),
                        path: folder.clone(),
                        branch: Some(branch.trim().to_owned()).filter(|b| !b.is_empty()),
                    })])
                }
            })
        }
        Dialog::NewTag {
            name,
            target,
            message,
        } => {
            let ok = valid_ref_name(name.trim());
            let mut submitted = false;
            let summary = tab
                .and_then(|t| t.snap.as_ref())
                .and_then(|s| s.commits.iter().find(|c| c.oid == *target))
                .map(|c| c.summary.clone());
            let result = KitDialog::new(salt)
                .title(format!("Tag {}", short_id(*target)))
                .icon(icon::TAG)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    if let Some(summary) = &summary {
                        note(
                            ui,
                            format!("{}  {}  {summary}", icon::GIT_COMMIT, short_id(*target)),
                            p.text_dim,
                        );
                    }
                    let error = (!name.trim().is_empty() && !valid_ref_name(name.trim()))
                        .then_some("Not a valid tag name");
                    field_with_error(ui, "Name", name, "v1.2.0", true, error);
                    submitted = TextArea::new(message)
                        .label("Message (makes an annotated tag)")
                        .rows(3..=8)
                        .id_salt("tag-message")
                        .show(ui)
                        .value;
                })
                .footer(|f| confirm(f, "Create tag", ok, "Enter a valid tag name"))
                .show(ctx);
            let result = if submitted {
                DialogResult::Confirm
            } else {
                result
            };
            answer(result, || {
                if valid_ref_name(name.trim()) {
                    let m = (!message.trim().is_empty()).then(|| message.trim().to_owned());
                    Outcome::Run(vec![Action::Op(Op::CreateTag {
                        name: name.trim().to_owned(),
                        target: *target,
                        message: m,
                    })])
                } else {
                    Outcome::Keep
                }
            })
        }
        Dialog::Stash { message, untracked } => {
            let result = KitDialog::new(salt)
                .title("Stash changes")
                .icon(icon::ARCHIVE)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .description("Puts your uncommitted changes aside until you apply them from Stashes in the sidebar.")
                .body(|ui| {
                    field(
                        ui,
                        "Message (optional)",
                        message,
                        "WIP: what you were doing",
                        true,
                    );
                    Checkbox::new(untracked, "Include untracked files").show(ui);
                })
                .footer(|f| {
                    f.cancel().confirm("Stash");
                })
                .show(ctx);
            answer(result, || {
                let m = (!message.trim().is_empty()).then(|| message.trim().to_owned());
                Outcome::Run(vec![Action::Op(Op::StashSave {
                    message: m,
                    untracked: *untracked,
                })])
            })
        }
        Dialog::Merge {
            target,
            rebase,
            mode,
        } => {
            let current = tab
                .and_then(|t| t.snap.as_ref())
                .map(|s| s.head.label())
                .unwrap_or_default();
            let verb = |rebase: bool, mode: canopy_core::MergeMode| match (rebase, mode) {
                (true, _) => "Rebase",
                (false, canopy_core::MergeMode::Squash) => "Squash",
                _ => "Merge",
            };
            let ok = !target.is_empty();
            let label = verb(*rebase, *mode);
            let result = KitDialog::new(salt)
                .title(if *rebase { "Rebase" } else { "Merge" })
                .icon(icon::GIT_MERGE)
                .width(DialogWidth::Form)
                .body(|ui| merge_body(ui, tab, &current, target, rebase, mode))
                .footer(|f| confirm(f, label, ok, "Choose a branch"))
                .show(ctx);
            answer(result, || {
                if target.is_empty() {
                    return Outcome::Keep;
                }
                let op = if *rebase {
                    Op::Rebase(target.clone())
                } else {
                    Op::Merge(target.clone(), *mode)
                };
                Outcome::Run(vec![Action::Op(op)])
            })
        }
        Dialog::AddRemote {
            name,
            builder,
            publish,
            fetch,
        } => {
            let remotes: Vec<&str> = tab
                .and_then(|t| t.snap.as_ref())
                .map(|s| s.remotes.iter().map(|r| r.name.as_str()).collect())
                .unwrap_or_default();
            let problem = remote_name_problem(name.trim(), &remotes);
            let ok = problem.is_none() && builder.resolve().is_some();
            let reason = if problem.is_some() {
                "Choose a remote name that is free and valid"
            } else {
                "Enter the repository"
            };
            let mut dialog = KitDialog::new(salt)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body);
            dialog = if *publish {
                dialog
                    .title("Publish repository")
                    .icon(icon::CLOUD_ARROW_UP)
                    .description("This repository is local only. Create an empty repository on GitHub or GitLab, enter it here, and Canopy adds it as a remote and pushes the current branch.")
            } else {
                dialog
                    .title("Add remote")
                    .icon(icon::CLOUD)
                    .description("A remote is another copy of this repository, usually on GitHub or GitLab, that you fetch from and push to.")
            };
            let label = if *publish {
                "Add and push"
            } else {
                "Add remote"
            };
            let result = dialog
                .body(|ui| {
                    // An empty name is said by the disabled button, not as an error.
                    let shown = problem.as_deref().filter(|_| !name.is_empty());
                    field_with_error(ui, "Name", name, "origin", false, shown);
                    builder.ui(ui, true);
                    if !*publish {
                        Checkbox::new(fetch, "Fetch its branches after adding").show(ui);
                    }
                })
                .footer(|f| confirm(f, label, ok, reason))
                .show(ctx);
            answer(result, || match builder.resolve() {
                Some(url) if remote_name_problem(name.trim(), &remotes).is_none() => {
                    let mut run = vec![Action::Op(Op::AddRemote {
                        name: name.trim().to_owned(),
                        url,
                    })];
                    if *publish {
                        run.push(Action::Op(Op::Push {
                            force: false,
                            expect: None,
                        }));
                    } else if *fetch {
                        run.push(Action::Fetch);
                    }
                    Outcome::Run(run)
                }
                _ => Outcome::Keep,
            })
        }
        Dialog::NewRequest {
            kind,
            request,
            branches,
        } => {
            let valid = |r: &NewRequest| {
                !r.title.trim().is_empty() && !r.target.is_empty() && r.target != r.source
            };
            let ok = valid(request);
            let reason = if request.title.trim().is_empty() {
                "Enter a title"
            } else {
                "Choose a target branch"
            };
            let label = format!("Create {}", kind.request_abbrev());
            let mut submitted = false;
            let result = KitDialog::new(salt)
                .title(format!("New {}", kind.request_noun()))
                .icon(icon::GIT_PULL_REQUEST)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    let options: Vec<(&str, &str)> = branches
                        .iter()
                        .filter(|b| **b != request.source)
                        .map(|b| (b.as_str(), b.as_str()))
                        .collect();
                    let mut pick = options
                        .iter()
                        .find(|(b, _)| *b == request.target)
                        .map_or("", |(b, _)| *b);
                    let mut target = Select::new(&mut pick, &options)
                        .label(format!("Merge {} into", request.source))
                        .placeholder("Choose a branch…")
                        .enabled(!options.is_empty())
                        .disabled_reason("No other branches on the remote yet")
                        .id_salt("request-target");
                    if options.is_empty() {
                        target = target.helper("Fetch to refresh the list.");
                    }
                    target.show(ui);
                    if pick != request.target && !pick.is_empty() {
                        pick.clone_into(&mut request.target);
                    }
                    field(
                        ui,
                        "Title",
                        &mut request.title,
                        "What this change does…",
                        true,
                    );
                    submitted = TextArea::new(&mut request.body)
                        .label("Description")
                        .placeholder(
                            "Why, how it was tested, anything reviewers should know (Markdown)…",
                        )
                        .rows(6..=12)
                        .id_salt("request-body")
                        .show(ui)
                        .value;
                    Checkbox::new(&mut request.draft, "Create as draft").show(ui);
                })
                .footer(|f| confirm(f, &label, ok, reason))
                .show(ctx);
            let result = if submitted {
                DialogResult::Confirm
            } else {
                result
            };
            answer(result, || {
                if valid(request) {
                    let mut r = request.clone();
                    #[expect(
                        clippy::assigning_clones,
                        reason = "the source borrows the target, so clone_into cannot take it"
                    )]
                    {
                        r.title = r.title.trim().to_owned();
                    }
                    Outcome::Run(vec![Action::Host(HostJob::Create(r))])
                } else {
                    Outcome::Keep
                }
            })
        }
        Dialog::Confirm {
            title,
            body,
            confirm,
            danger,
            repo,
            op,
        } => {
            let result =
                confirmation(salt, title, body, confirm, *danger, icon::QUESTION, |_| {}).show(ctx);
            answer(result, || {
                Outcome::Run(vec![Action::OpIn {
                    repo: repo.clone(),
                    op: op.clone(),
                }])
            })
        }
        Dialog::Diverged {
            repo,
            mode,
            remember,
            everywhere,
            preview,
        } => {
            use crate::action::Remember;
            use crate::settings::PullMode;
            let options = [
                (
                    PullMode::Rebase,
                    "Rebase (recommended)",
                    Some("Replays your commits on top of the remote's, with new IDs."),
                ),
                (
                    PullMode::Merge,
                    "Merge",
                    Some("Joins both lines with a merge commit; every commit keeps its ID."),
                ),
            ];

            let confirm = match mode {
                PullMode::Rebase => "Pull with rebase",
                _ => "Pull with merge",
            };
            let result = KitDialog::new(salt)
                .title("Your branch and the remote have diverged")
                .icon(icon::GIT_FORK)
                .width(DialogWidth::Form)
                .body(|ui| {
                    RadioGroup::new(mode, &options)
                        .label("Combine them with")
                        .id_salt("diverged-mode")
                        .show(ui);
                    ui.add_space(theme::SM);
                    let mut scopes = vec![
                        (Remember::No, "Don't remember"),
                        (Remember::Repository, "For this repository (recommended)"),
                    ];
                    if *everywhere {
                        scopes.push((Remember::Everywhere, "For every repository"));
                    }
                    Select::new(remember, &scopes)
                        .label("Remember my decision")
                        .fit_to_options()
                        .id_salt("diverged-remember")
                        .show(ui);
                    ui.add_space(theme::SM);
                    predicted_conflicts(ui, preview);
                })
                .footer(|f| {
                    f.cancel().confirm(confirm);
                })
                .show(ctx);
            answer(result, || {
                Outcome::Run(vec![Action::PullAs {
                    repo: repo.clone(),
                    mode: *mode,
                    remember: *remember,
                    seen: preview.tip,
                }])
            })
        }
        Dialog::PullConflicts {
            repo,
            strategy,
            preview,
            moved,
        } => {
            let (title, description, confirm) = if *moved {
                let incoming = match preview.behind {
                    1 => "1 new commit".to_owned(),
                    n => format!("{n} new commits"),
                };
                (
                    "The remote changed while you decided",
                    format!(
                        "{} received new commits since Canopy checked it and now has {incoming} for you. Pull them?",
                        preview.upstream
                    ),
                    moved_pull_confirm(*strategy),
                )
            } else {
                (
                    "This pull will likely conflict",
                    likely_conflict_description(&preview.upstream, preview.behind),
                    "Pull anyway",
                )
            };
            let result = KitDialog::new(salt)
                .title(title)
                .icon(icon::WARNING)
                .width(DialogWidth::Form)
                .description(description)
                .body(|ui| predicted_conflicts(ui, preview))
                .footer(|f| {
                    f.cancel().confirm(confirm);
                })
                .show(ctx);
            answer(result, || {
                Outcome::Run(vec![Action::PullAnyway {
                    repo: repo.clone(),
                    strategy: *strategy,
                    seen: preview.tip,
                }])
            })
        }
        Dialog::ConfirmClearIdentity { repo, scope } => {
            let target = match scope {
                canopy_core::identity::Scope::Local => "this repository's configuration",
                canopy_core::identity::Scope::Global => {
                    "your global Git configuration, which every repository without its own identity uses"
                }
            };
            let body = format!(
                "This removes user.name and user.email from {target}. Commits already made keep their authors. If the same value is also set in another file, that one stays in use."
            );
            let values = identity_in_scope(tab, *scope);
            let result = confirmation(
                salt,
                "Clear Git identity?",
                &body,
                "Clear identity",
                true,
                icon::QUESTION,
                |ui| {
                    if let Some(values) = &values {
                        InsetBox::new().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            note(ui, "Removing", p.text_dim);
                            ui.label(
                                RichText::new(values)
                                    .font(FontId::monospace(m.mono_size))
                                    .color(p.text),
                            );
                        });
                    }
                },
            )
            .show(ctx);
            answer(result, || {
                Outcome::Run(vec![Action::ClearIdentity {
                    repo: repo.clone(),
                    scope: *scope,
                }])
            })
        }
        Dialog::TrustFolder { repo, remote } => {
            let name = |path: &std::path::Path| {
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            };
            swallow_enter(ctx);
            let parent = crate::trust::parent_offer(repo);
            let result = KitDialog::new(salt)
                .title("Trust this folder?")
                .icon(icon::SHIELD_CHECK)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .description(format!("{} is a partial clone: some of its files were left on the server. To show them, Canopy has to download them from {remote} with git, and git then follows this repository's own configuration, which can run programs and contact servers.", name(repo)))
                .body(|ui| {
                    ui.label(
                        RichText::new("Trust a folder only if everything in it was made by you or by people you trust. You can remove it again in Settings.")
                            .size(m.body_size)
                            .color(p.text_dim),
                    );
                    if let Some(parent) = &parent {
                        paint::seam_h(
                            ui.painter(),
                            ui.max_rect().x_range(),
                            ui.cursor().top(),
                            theme::surfaces(ui.ctx()).border,
                        );
                        ui.add_space(theme::XS);
                        note(ui, format!("Trusting {} covers every repository inside it, including ones you clone later.", name(parent)), p.text_faint);
                        if Button::new(format!("Trust everything in {}", name(parent)))
                            .ghost()
                            .size(Size::Sm)
                            .icon(icon::WARNING)
                            .icon_tone(Tone::Warning)
                            .tooltip(parent.display().to_string())
                            .show(ui)
                            .clicked()
                        {
                            extra = Some(Outcome::Run(vec![Action::TrustFolder(parent.clone())]));
                        }
                    }
                })
                .footer(|f| {
                    f.cancel_label("Not now")
                        .confirm(format!("Trust {}", name(repo)));
                })
                .show(ctx);
            answer(result, || {
                Outcome::Run(vec![Action::TrustFolder(repo.clone())])
            })
        }
        Dialog::ConfirmHost {
            title,
            body,
            confirm,
            danger,
            job,
        } => {
            let result = confirmation(salt, title, body, confirm, *danger, icon::GIT_MERGE, |_| {})
                .show(ctx);
            answer(result, || Outcome::Run(vec![Action::Host(job.clone())]))
        }
        Dialog::Credential {
            prompt,
            answer,
            serial,
        } => credential(ctx, salt, prompt, answer, *serial),
        Dialog::Update(a) => update::update(ctx, salt, a, updater),
        Dialog::WhatsNew { version, notes } => {
            let result = KitDialog::new(salt)
                .title(format!("What's new in Canopy {version}"))
                .icon(icon::SPARKLE)
                .width(DialogWidth::Form)
                .body(|ui| {
                    document(ui, "whats-new", NOTES_HEIGHT, notes);
                    if LinkButton::new("All releases")
                        .url(crate::updater::RELEASES_PAGE)
                        .text_size(m.meta_size)
                        .show(ui)
                        .clicked()
                    {
                        extra = Some(Outcome::Run(vec![Action::OpenUrl(
                            crate::updater::RELEASES_PAGE.to_owned(),
                        )]));
                    }
                })
                .footer(|f| {
                    f.confirm("Continue");
                })
                .show(ctx);
            answer(result, || Outcome::Close)
        }
        Dialog::Crash(report) => {
            let result = KitDialog::new(salt)
                .title("Canopy closed unexpectedly")
                .icon(icon::FIRST_AID_KIT)
                .width(DialogWidth::Form)
                .description("A crash report was saved on this computer. Nothing was sent.")
                .body(|ui| {
                    if !report.message.is_empty() {
                        InsetBox::new().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&report.message)
                                        .font(FontId::monospace(m.mono_size))
                                        .color(p.text),
                                )
                                .wrap(),
                            );
                        });
                    }
                    if let Some(dir) = report.path.parent()
                        && Button::new("Show folder")
                            .ghost()
                            .size(Size::Sm)
                            .icon(icon::FOLDER)
                            .show(ui)
                            .clicked()
                    {
                        extra = Some(Outcome::Run(vec![Action::OpenPath(dir.to_path_buf())]));
                    }
                })
                .footer(|f| {
                    f.cancel_label("Dismiss").confirm("Open report");
                })
                .show(ctx);
            answer(result, || {
                Outcome::Run(vec![Action::OpenInEditor(report.path.clone())])
            })
        }
        Dialog::Consent {
            step,
            updated,
            reading,
        } => consent(ctx, salt, *step, *updated, reading),
        Dialog::Legal(doc) => match reader::show(ctx, doc, reader::Footer::Close) {
            reader::Answer::Close => Outcome::Close,
            reader::Answer::Link(link) => Outcome::RunKeep(vec![follow(link)]),
            reader::Answer::Open
            | reader::Answer::Back
            | reader::Answer::First
            | reader::Answer::Second => Outcome::Keep,
        },
        Dialog::About => {
            let result = KitDialog::new(salt)
                .title("About Canopy")
                .initial_focus(InitialFocus::Cancel)
                .body(|ui| extra = about(ui, git_version))
                .footer(|f| {
                    f.cancel_label("Close")
                        .confirm("Check for updates")
                        .confirm_icon(icon::ARROW_CIRCLE_UP);
                })
                .show(ctx);
            answer(result, || Outcome::Run(vec![Action::CheckUpdates]))
        }
    };
    match (extra, outcome) {
        (Some(e), Outcome::Keep) => e,
        (_, o) => o,
    }
}

/// The tallest release notes grow in What's new before they scroll.
const NOTES_HEIGHT: f32 = 360.0;

/// The id every dialog of `dialog`'s kind shares, so switching to another kind animates it in.
fn salt(dialog: &Dialog) -> (&'static str, std::mem::Discriminant<Dialog>) {
    ("dialog", std::mem::discriminant(dialog))
}

/// The modal area the kit shows `dialog` in (the kit derives it from the dialog's id).
#[cfg(test)]
fn area_id(dialog: &Dialog) -> egui::Id {
    egui::Id::new(("kit-dialog", egui::Id::new(salt(dialog)))).with("modal")
}

/// Where a clone goes: the URL and `parent`/`name`, once both are known.
fn clone_target(builder: &UrlBuilder, parent: &str) -> Option<(String, std::path::PathBuf)> {
    let url = builder.resolve()?;
    let name = url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("")
        .to_owned();
    if url.trim().is_empty() || name.is_empty() {
        return None;
    }
    let dest = crate::settings::expand_home(parent).join(&name);
    Some((url.trim().to_owned(), dest))
}

/// The confirm button of the question about a remote that moved after the user chose how to
/// combine the branches: it names that choice, as the diverged-branches dialog does.
fn moved_pull_confirm(strategy: canopy_core::PullStrategy) -> &'static str {
    use canopy_core::PullStrategy as S;
    match strategy {
        S::Rebase | S::RebaseMerges => "Pull with rebase",
        S::Merge | S::MergeNoFastForward => "Pull with merge",
        S::FastForwardOnly => "Pull",
    }
}

/// The likely-conflict dialog's text: what the upstream has and what happens on "Pull anyway".
fn likely_conflict_description(upstream: &str, behind: usize) -> String {
    let (incoming, verb) = match behind {
        1 => ("1 new commit".to_owned(), "touches"),
        n => (format!("{n} new commits"), "touch"),
    };
    format!(
        "{upstream} has {incoming} that {verb} what you are working on. If you pull anyway, Canopy takes you to Changes to resolve the conflicts."
    )
}

/// How many predicted files a pull's dialog lists before "and N more".
const PREDICTED_SHOWN: usize = 8;

/// The files a pull is predicted to conflict in, and the uncommitted ones it also changes.
fn predicted_conflicts(ui: &mut Ui, preview: &canopy_core::PullPreview) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let conflicts = preview.conflicts.as_deref().unwrap_or_default();
    for (files, says) in [
        (conflicts, "Likely conflicts"),
        (
            preview.overlap.as_slice(),
            "Uncommitted files the pull also changes",
        ),
    ] {
        if files.is_empty() {
            continue;
        }
        ui.add_space(theme::SM);
        note(ui, says, p.text_dim);
        InsetBox::new().show(ui, |ui| {
            ui.set_width(ui.available_width());
            for file in files.iter().take(PREDICTED_SHOWN) {
                ui.label(
                    RichText::new(file)
                        .font(FontId::monospace(m.mono_size))
                        .color(p.text),
                );
            }
            if files.len() > PREDICTED_SHOWN {
                note(
                    ui,
                    format!("and {} more", files.len() - PREDICTED_SHOWN),
                    p.text_faint,
                );
            }
        });
    }
}

/// A yes-or-no question: destructive ones get the danger disc and button and keep Enter from
/// confirming.
fn confirmation<'a>(
    salt: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    body: &str,
    confirm: &str,
    danger: bool,
    glyph: &str,
    extra: impl FnOnce(&mut Ui) + 'a,
) -> KitDialog<'a> {
    let mut d = KitDialog::new(salt)
        .title(title)
        .description(body)
        .body(extra)
        .footer(|f| {
            f.cancel().confirm(confirm);
        });
    d = if danger {
        d.destructive().icon(icon::WARNING)
    } else {
        d.icon(glyph)
    };
    d
}

/// The merge dialog's body: merge or rebase, the branch, the merge mode and what will happen.
fn merge_body(
    ui: &mut Ui,
    tab: Option<&RepoTab>,
    current: &str,
    target: &mut String,
    rebase: &mut bool,
    mode: &mut canopy_core::MergeMode,
) {
    use canopy_core::MergeMode as M;
    let p = palette(ui.ctx());
    // The branch name keeps the two choices from reading as "current" in the abstract.
    let shown: String = if current.chars().count() > 20 {
        current.chars().take(19).chain(['…']).collect()
    } else {
        current.to_owned()
    };
    let (merge_label, rebase_label) = if shown.is_empty() {
        (
            "Merge into current".to_owned(),
            "Rebase current onto".to_owned(),
        )
    } else {
        (
            format!("Merge into {shown}"),
            format!("Rebase {shown} onto"),
        )
    };
    let kinds = [(false, merge_label.as_str()), (true, rebase_label.as_str())];
    Segmented::new(rebase, &kinds)
        .label("Merge or rebase")
        .id_salt("merge-kind")
        .show(ui);
    let mut names: Vec<String> = tab
        .and_then(|t| t.snap.as_ref())
        .map(|s| {
            s.branches
                .iter()
                .filter(|b| !b.is_head)
                .map(|b| b.name.clone())
                .chain(
                    s.remote_branches
                        .iter()
                        .map(canopy_core::RemoteBranchInfo::full_name),
                )
                .collect()
        })
        .unwrap_or_default();
    // A branch chosen before the dialog opened stays listed even if this list lacks it.
    if !target.is_empty() && !names.contains(target) {
        names.insert(0, target.clone());
    }
    let options: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), n.as_str())).collect();
    let mut pick = names
        .iter()
        .find(|n| *n == target)
        .map_or("", String::as_str);
    let mut branch = Select::new(&mut pick, &options)
        .label("Branch")
        .placeholder("Choose a branch…")
        .enabled(!options.is_empty())
        .id_salt("merge-target");
    if options.is_empty() {
        branch = branch.helper("This repository has no other branch to merge or rebase onto.");
    }
    branch.show(ui);
    if pick != target && !pick.is_empty() {
        *target = pick.to_owned();
    }
    if !*rebase {
        let modes: Vec<(M, &str, Option<&str>)> = M::ALL
            .iter()
            .map(|m| {
                let explanation = match m {
                    M::Default => "Otherwise creates a merge commit.",
                    M::NoFastForward => "Keeps the branch join visible in history.",
                    M::FastForwardOnly => "Stops without changes if the branches diverged.",
                    M::Squash => "Leaves one combined change to review and commit.",
                };
                (*m, m.label(), Some(explanation))
            })
            .collect();
        RadioGroup::new(mode, &modes)
            .label("How")
            .id_salt("merge-mode")
            .show(ui);
    }
    if !target.is_empty() {
        let text = match (*rebase, *mode) {
            (true, _) => format!("Replay {current}'s commits on top of {target}."),
            (false, M::Default) => format!(
                "Merge {target} into {current}, fast-forwarding when {current} has no commits of its own."
            ),
            (false, M::NoFastForward) => format!(
                "Merge {target} into {current} with a merge commit, even when a fast-forward is possible."
            ),
            (false, M::FastForwardOnly) => format!(
                "Move {current} forward to {target}; stops without changes if they have diverged."
            ),
            (false, M::Squash) => format!(
                "Stage all of {target}'s changes on {current} as one change, without committing. Review them in Changes, then commit."
            ),
        };
        note(ui, text, p.text_faint);
    }
}

/// The first-start consent: the terms, then the crash-report question. Answered only with its
/// buttons; bare Enter accepts nothing.
fn consent(
    ctx: &egui::Context,
    salt: impl std::hash::Hash + std::fmt::Debug,
    step: Pending,
    updated: bool,
    reading: &mut Option<Document>,
) -> Outcome {
    let p = palette(ctx);
    swallow_enter(ctx);
    // Reading a document: the reader at full size, with the step's answers in its footer so the
    // decision never leaves the screen.
    if let Some(doc) = reading {
        let footer = match step {
            Pending::Terms => reader::Footer::Consent {
                first: "I agree",
                second: "Quit",
                neutral: false,
            },
            Pending::CrashReports => reader::Footer::Consent {
                first: "Send crash reports",
                second: "Don't send",
                neutral: true,
            },
        };
        return match reader::show(ctx, doc, footer) {
            reader::Answer::Open | reader::Answer::Close => Outcome::Keep,
            reader::Answer::Back => {
                *reading = None;
                Outcome::Keep
            }
            reader::Answer::First => Outcome::Run(vec![match step {
                Pending::Terms => Action::AcceptTerms,
                Pending::CrashReports => Action::SetCrashReports(true),
            }]),
            reader::Answer::Second => Outcome::Run(vec![match step {
                Pending::Terms => Action::Quit,
                Pending::CrashReports => Action::SetCrashReports(false),
            }]),
            reader::Answer::Link(link) => Outcome::RunKeep(vec![follow(link)]),
        };
    }
    match step {
        Pending::Terms => {
            let result = KitDialog::new(salt)
                .title(if updated {
                    "We've updated our terms"
                } else {
                    "Welcome to Canopy"
                })
                .icon(icon::TREE)
                .width(DialogWidth::Form)
                .blocking()
                .initial_focus(InitialFocus::Body)
                .description(if updated {
                    "Accept the new Terms of Service and Privacy Policy to keep using Canopy."
                } else {
                    "Accept the Terms of Service and the Privacy Policy to start."
                })
                .body(|ui| {
                    for (glyph, text) in [
                        (icon::LOCK_SIMPLE, "Your repositories and tokens are never sent to NecturaLabs. Requests to GitHub and GitLab go straight to them."),
                        (icon::ARROW_CIRCLE_UP, "At most once a day, Canopy checks GitHub for a new version. You can turn this off."),
                        (icon::FIRST_AID_KIT, "Crash reports are sent only if you agree. You choose next, and can change it any time."),
                    ] {
                        bullet(ui, glyph, p.accent, text);
                    }
                    documents(ui, &[Document::Terms, Document::Privacy], reading);
                })
                .footer(|f| {
                    f.cancel_label("Quit").confirm("I agree");
                })
                .show(ctx);
            match result {
                DialogResult::Open => Outcome::Keep,
                DialogResult::Cancel => Outcome::Run(vec![Action::Quit]),
                DialogResult::Confirm => Outcome::Run(vec![Action::AcceptTerms]),
            }
        }
        Pending::CrashReports => {
            let result = KitDialog::new(salt)
                .title("Send crash reports?")
                .icon(icon::FIRST_AID_KIT)
                .width(DialogWidth::Form)
                .blocking()
                .description("If Canopy crashes, it can send a report to NecturaLabs so we can fix the problem. It also sends a short record of each session: when it started, how long it ran and how it ended.")
                .body(|ui| {
                    for (glyph, text, color) in [
                        (icon::CHECK, "A report holds the error message, a stack trace, your Canopy version and your operating system.", p.success),
                        (icon::SHIELD_CHECK, "Canopy does not add your name, tokens, logs or repository files. An error message can name a file or branch.", p.accent),
                        (icon::GLOBE_HEMISPHERE_WEST, "Reports are stored in the EU (Germany) by Sentry and deleted after 90 days.", p.accent2),
                    ] {
                        bullet(ui, glyph, color, text);
                    }
                    note(ui, "Canopy works the same either way. You can change this at any time in Settings → Privacy.", p.text_faint);
                    documents(ui, &[Document::Privacy], reading);
                })
                .footer(|f| {
                    f.choice("Send crash reports", "Don't send");
                })
                .show(ctx);
            match result {
                DialogResult::Open => Outcome::Keep,
                DialogResult::Cancel => Outcome::Run(vec![Action::SetCrashReports(false)]),
                DialogResult::Confirm => Outcome::Run(vec![Action::SetCrashReports(true)]),
            }
        }
    }
}

/// The documents the consent asks about, one row each: its glyph, title and date, opening it in
/// the reader.
fn documents(ui: &mut Ui, docs: &[Document], reading: &mut Option<Document>) {
    let m = theme::metrics();
    InsetBox::new().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        // The rows sit on the box's fill.
        let ground = theme::surfaces(ui.ctx()).field;
        for doc in docs {
            let row = ListRow::new(("consent-document", *doc))
                .icon(doc.glyph())
                .label(doc.title())
                .detail(format!("Updated {}", doc.updated()))
                .ground(ground)
                .accessible_name(format!("Read the {}", doc.title()))
                .show_with(ui, |rc, painter| {
                    let g = painter.layout_no_wrap(
                        format!("Read  {}", icon::ARROW_RIGHT),
                        FontId::proportional(m.meta_size),
                        rc.accent,
                    );
                    painter.galley(
                        egui::pos2(
                            rc.content.right() - g.size().x,
                            rc.rect.center().y - g.size().y / 2.0,
                        ),
                        g,
                        rc.accent,
                    );
                });
            if row.clicked() {
                *reading = Some(*doc);
            }
        }
    });
}

/// The action a link in a legal document asks for: a web page opens, an email address is copied.
fn follow(link: crate::ui::legal::Link) -> Action {
    match link {
        crate::ui::legal::Link::Web(url) => Action::OpenUrl(url),
        crate::ui::legal::Link::Email(address) => Action::Copy(address),
    }
}

/// The About dialog's body: the logo, the version, and where to read more.
fn about(ui: &mut Ui, git_version: &str) -> Option<Outcome> {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let mut outcome = None;
    ui.vertical_centered(|ui| {
        art::show(ui, art::logo(), egui::Vec2::splat(64.0));
        ui.label(
            RichText::new("Canopy")
                .font(theme::heading(m.display_size))
                .color(p.text),
        );
        ui.label(RichText::new(crate::build_info::title_label()).color(p.text_dim));
        ui.label(
            RichText::new(
                git_version
                    .strip_prefix("git version ")
                    .map_or_else(|| git_version.to_owned(), |v| format!("Git {v}")),
            )
            .size(m.meta_size)
            .color(p.text_faint),
        );
        ui.add_space(theme::SM);
        ui.label(RichText::new("A fast, beautiful, Linux-first Git client.").color(p.text_dim));
        ui.label(
            RichText::new("© 2026 NecturaLabs.")
                .size(m.meta_size)
                .color(p.text_faint),
        );
        ui.add_space(theme::MD);
        // Where to read more: links, so the one action in the footer stays the main one. The row
        // is centred on its width from the frame before.
        let links: [(&str, Option<&str>, Outcome); 4] = [
            ("What's new", None, Outcome::Run(vec![Action::WhatsNew])),
            (
                "Releases",
                Some(crate::updater::RELEASES_PAGE),
                Outcome::Run(vec![Action::OpenUrl(
                    crate::updater::RELEASES_PAGE.to_owned(),
                )]),
            ),
            (
                "Privacy Policy",
                None,
                Outcome::Run(vec![Action::OpenDialog(Dialog::Legal(Document::Privacy))]),
            ),
            (
                "Terms of Service",
                None,
                Outcome::Run(vec![Action::OpenDialog(Dialog::Legal(Document::Terms))]),
            ),
        ];
        let width_id = ui.id().with("about-links-width");
        let last: f32 = ui.data(|d| d.get_temp(width_id)).unwrap_or(0.0);
        let row = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::MD;
            ui.add_space(((ui.available_width() - last) / 2.0).max(0.0));
            let start = ui.cursor().left();
            for (label, url, target) in links {
                let mut link = LinkButton::new(label).text_size(m.meta_size);
                if let Some(url) = url {
                    link = link.url(url);
                }
                if link.show(ui).clicked() {
                    outcome = Some(target);
                }
            }
            ui.min_rect().right() - start
        });
        let width = row.inner;
        if (width - last).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(width_id, width));
            ui.ctx().request_repaint();
        }
    });
    outcome
}

#[cfg(test)]
mod tests {
    use super::folder_from_url;

    #[test]
    fn a_submodule_folder_is_suggested_from_its_url() {
        for (url, folder) in [
            ("https://github.com/o/repo.git", "repo"),
            ("git@github.com:o/repo.git/", "repo"),
            ("/srv/git/lib", "lib"),
            ("", ""),
        ] {
            assert_eq!(folder_from_url(url), folder, "{url}");
        }
    }

    use super::{
        Dialog, DialogStack, Oid, Start, likely_conflict_description, moved_pull_confirm,
        valid_ref_name,
    };
    use crate::askpass::Prompt;
    use crate::ui::legal::{Document, Pending};
    use crossbeam_channel::Receiver;

    fn consent() -> Dialog {
        Dialog::Consent {
            step: Pending::Terms,
            updated: false,
            reading: None,
        }
    }

    fn credential(text: &str) -> (Dialog, Receiver<Option<String>>) {
        let (prompt, answers) = Prompt::with_reply(text);
        (Dialog::credential(prompt), answers)
    }

    /// A short name for the dialog on screen.
    fn shown(stack: &DialogStack) -> String {
        match stack.current() {
            None => "none".into(),
            Some(Dialog::About) => "about".into(),
            Some(Dialog::Consent { .. }) => "consent".into(),
            Some(Dialog::Legal(_)) => "legal".into(),
            Some(Dialog::Credential { prompt, .. }) => prompt.text.clone(),
            Some(Dialog::Stash { message, .. }) => message.clone(),
            Some(other) => format!("{other:?}"),
        }
    }

    fn stash(name: &str) -> Dialog {
        Dialog::Stash {
            message: name.into(),
            untracked: false,
        }
    }

    #[test]
    fn the_likely_conflict_text_agrees_with_the_number_of_commits() {
        for (behind, says) in [
            (1, "origin/main has 1 new commit that touches what you are"),
            (2, "origin/main has 2 new commits that touch what you are"),
        ] {
            let text = likely_conflict_description("origin/main", behind);
            assert!(text.starts_with(says), "{behind}: {text}");
        }
    }

    /// The question about a remote that moved keeps the user's earlier choice in view: the
    /// button names what the pull will do, in the words of the diverged-branches dialog.
    #[test]
    fn the_moved_remote_question_names_the_way_the_branches_combine() {
        use canopy_core::PullStrategy as S;
        for (strategy, says) in [
            (S::Rebase, "Pull with rebase"),
            (S::RebaseMerges, "Pull with rebase"),
            (S::Merge, "Pull with merge"),
            (S::MergeNoFastForward, "Pull with merge"),
            (S::FastForwardOnly, "Pull"),
        ] {
            assert_eq!(moved_pull_confirm(strategy), says, "{strategy:?}");
        }
    }

    #[test]
    fn ordinary_dialogs_stack_newest_first_and_none_is_lost() {
        let mut s = DialogStack::default();
        assert_eq!(shown(&s), "none");
        s.open(stash("a"));
        s.open(stash("b"));
        assert_eq!(shown(&s), "b");
        s.close();
        assert_eq!(shown(&s), "a");
        s.dismiss();
        assert_eq!(shown(&s), "none");
        s.close();
        s.dismiss();
        assert_eq!(shown(&s), "none");
    }

    #[test]
    fn consent_cannot_be_replaced_or_dismissed() {
        let mut s = DialogStack::default();
        s.open(stash("notice"));
        s.open(consent());
        s.open(Dialog::About);
        assert_eq!(shown(&s), "consent");
        s.dismiss();
        assert_eq!(shown(&s), "consent");
        s.close_where(|d| matches!(d, Dialog::Consent { .. }));
        assert_eq!(shown(&s), "about");
        s.close();
        assert_eq!(shown(&s), "notice");
    }

    /// The app's Escape binding reaches the stack before the dialog draws, so going back from a
    /// document read inside the consent has to happen here.
    #[test]
    fn escape_goes_back_from_a_document_read_in_the_consent() {
        let mut s = DialogStack::default();
        s.open(Dialog::Consent {
            step: Pending::Terms,
            updated: false,
            reading: Some(Document::Terms),
        });
        s.dismiss();
        assert!(
            matches!(s.current(), Some(Dialog::Consent { reading: None, .. })),
            "back to the consent summary, still open"
        );
        s.dismiss();
        assert_eq!(shown(&s), "consent");
    }

    #[test]
    fn legal_documents_are_not_replaced() {
        let mut s = DialogStack::default();
        s.open(Dialog::Legal(Document::Privacy));
        s.open(Dialog::About);
        assert_eq!(shown(&s), "legal");
        s.close();
        assert_eq!(shown(&s), "about");
    }

    #[test]
    fn credentials_go_in_front_one_at_a_time_in_arrival_order() {
        let mut s = DialogStack::default();
        s.open(consent());
        let (first, first_answers) = credential("first");
        let (second, _second_answers) = credential("second");
        s.open(first);
        assert_eq!(shown(&s), "first", "a prompt goes in front of the consent");
        s.open(second);
        assert_eq!(shown(&s), "first", "a second prompt waits behind the first");
        s.open(Dialog::About);
        s.open(consent());
        assert_eq!(shown(&s), "first", "nothing hides a prompt");
        s.close();
        assert_eq!(first_answers.try_recv().ok(), Some(None), "closing cancels");
        assert_eq!(shown(&s), "second");
        s.close();
        assert_eq!(shown(&s), "consent");
    }

    #[test]
    fn a_closed_prompt_is_always_answered() {
        let mut s = DialogStack::default();
        // Answered in the dialog, then closed: the answer stands.
        let (d, answers) = credential("Password: ");
        if let Dialog::Credential { prompt, .. } = &d {
            prompt.answer(Some("hunter2".into()));
        }
        s.open(d);
        s.close();
        assert_eq!(answers.try_recv().ok(), Some(Some("hunter2".into())));
        // Escape on a host key says no; on a password it cancels.
        let (host, host_answers) = credential("Continue connecting (yes/no/[fingerprint])? ");
        s.open(host);
        s.dismiss();
        assert_eq!(host_answers.try_recv().ok(), Some(Some("no".into())));
        let (pw, pw_answers) = credential("Password: ");
        s.open(pw);
        s.dismiss();
        assert_eq!(pw_answers.try_recv().ok(), Some(None));
        // Closed from behind another dialog.
        let (queued, queued_answers) = credential("Username: ");
        let (front, _front_answers) = credential("Password: ");
        s.open(front);
        s.open(queued);
        s.close_where(
            |d| matches!(d, Dialog::Credential { prompt, .. } if prompt.text == "Username: "),
        );
        assert_eq!(queued_answers.try_recv().ok(), Some(None));
        assert_eq!(shown(&s), "Password: ");
    }

    /// One frame of `dialog` with `events`: the text it painted and where its "Show secret"
    /// button is.
    fn credential_frame(
        ctx: &egui::Context,
        dialog: &mut Dialog,
        events: Vec<egui::Event>,
    ) -> (String, Option<egui::Pos2>) {
        fn walk(shape: &egui::Shape, into: &mut String) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, into)),
                egui::Shape::Text(t) => {
                    into.push_str(t.galley.text());
                    into.push('\n');
                }
                _ => {}
            }
        }
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        let updater = crate::updater::Updater::default();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let _ = super::show(ui.ctx(), dialog, None, &mut picker, false, &updater, "");
            },
        );
        out.textures_delta.clear();
        let mut painted = String::new();
        for c in &out.shapes {
            walk(&c.shape, &mut painted);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "AccessKit bounds are points on a 1200 x 800 screen"
        )]
        let reveal = out.platform_output.accesskit_update.and_then(|u| {
            u.nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Show secret"))
                .and_then(|(_, n)| n.bounds())
                .map(|b| {
                    egui::pos2(
                        f64::midpoint(b.x0, b.x1) as f32,
                        f64::midpoint(b.y0, b.y1) as f32,
                    )
                })
        });
        (painted, reveal)
    }

    /// review-views-7: a password shown in one prompt is hidden again in the next one.
    #[test]
    fn each_credential_prompt_starts_with_its_secret_hidden() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        crate::ui::theme::apply(
            &ctx,
            crate::ui::theme::ThemeId::Midnight,
            crate::ui::theme::Appearance::STILL,
        );
        ctx.enable_accesskit();
        let (mut first, _first_answers) = credential("Password for 'https://example.com': ");
        if let Dialog::Credential { answer, .. } = &mut first {
            "hunter2".clone_into(answer);
        }
        let mut reveal = None;
        for _ in 0..3 {
            reveal = credential_frame(&ctx, &mut first, Vec::new()).1;
        }
        let at = reveal.expect("a Show secret button");
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        credential_frame(
            &ctx,
            &mut first,
            vec![egui::Event::PointerMoved(at), button(true)],
        );
        credential_frame(&ctx, &mut first, vec![button(false)]);
        let (painted, _) = credential_frame(&ctx, &mut first, Vec::new());
        assert!(painted.contains("hunter2"), "revealed: {painted}");

        let (mut second, _second_answers) = credential("Password for 'https://example.com': ");
        if let Dialog::Credential { answer, .. } = &mut second {
            "s3cret".clone_into(answer);
        }
        for _ in 0..2 {
            let (painted, _) = credential_frame(&ctx, &mut second, Vec::new());
            assert!(
                !painted.contains("s3cret"),
                "the next prompt shows: {painted}"
            );
        }
    }

    #[test]
    fn withdrawn_prompts_leave_the_stack() {
        let mut s = DialogStack::default();
        s.open(stash("under"));
        let (d, _answers) = credential("Password: ");
        let prompt = match &d {
            Dialog::Credential { prompt, .. } => prompt.clone(),
            _ => unreachable!(),
        };
        s.open(d);
        s.drop_withdrawn();
        assert_eq!(shown(&s), "Password: ");
        prompt.withdraw();
        s.drop_withdrawn();
        assert_eq!(shown(&s), "under");
    }

    #[test]
    fn branch_starts_are_typed_and_shortened_safely() -> Result<(), git2::Error> {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let oid = Oid::from_str(sha)?;
        assert_eq!(Start::Commit(oid).label(String::new), "commit 0123456");
        assert_eq!(Start::Commit(oid).rev().as_deref(), Some(sha));
        // Names, however long or short and whatever their characters, are shown whole.
        for name in [
            "main",
            "v1",
            "feature/ünïcödé-forty-bytes-long-name",
            "release/2026-10-01-hotfix-for-the-thing",
            "0123456789abcdef",
        ] {
            let start = Start::Branch(name.to_owned());
            assert_eq!(start.label(String::new), name);
            assert_eq!(start.rev().as_deref(), Some(name));
        }
        assert_eq!(Start::Head.label(|| "main".into()), "main");
        assert_eq!(Start::Head.rev(), None);
        assert!(matches!(
            Dialog::new_branch_at(Start::Head),
            Dialog::NewBranch {
                start: Start::Head,
                ..
            }
        ));
        assert!(matches!(
            Dialog::new_branch_at(Start::Commit(oid)),
            Dialog::NewBranch { start: Start::Commit(o), .. } if o == oid
        ));
        Ok(())
    }

    #[test]
    fn the_opener_answers_only_the_dialog_that_asked() {
        use super::OpenFailure;
        use std::path::{Path, PathBuf};
        let failure = OpenFailure {
            message: "/tmp/x is not a Git repository".into(),
            not_a_repository: true,
        };
        let mut s = DialogStack::default();
        let mut open = Dialog::open();
        if let Dialog::Open { pending, .. } = &mut open {
            *pending = Some(PathBuf::from("/tmp/x"));
        }
        s.open(open);
        // Another folder (a restored session) and a creation of the same path are not its own.
        assert!(!s.finish_open(Path::new("/tmp/y"), false, Err(&failure)));
        assert!(!s.finish_open(Path::new("/tmp/x"), true, Err(&failure)));
        assert!(s.finish_open(Path::new("/tmp/x"), false, Err(&failure)));
        assert!(matches!(
            s.current(),
            Some(Dialog::Open { pending: None, error: Some(e), .. }) if *e == failure
        ));
        // Asked again, it closes once the repository opened.
        if let Some(Dialog::Open { pending, .. }) = s.current_mut() {
            *pending = Some(PathBuf::from("/tmp/x"));
        }
        assert!(s.finish_open(Path::new("/tmp/x"), false, Ok(())));
        assert!(s.current().is_none());

        let mut create = Dialog::new_repository("~/new".into());
        if let Dialog::NewRepository { pending, .. } = &mut create {
            *pending = Some(PathBuf::from("/home/ada/new"));
        }
        s.open(create);
        assert!(s.finish_open(Path::new("/home/ada/new"), true, Err(&failure)));
        assert!(matches!(
            s.current(),
            Some(Dialog::NewRepository { pending: None, error: Some(e), .. }) if e == &failure.message
        ));
    }

    #[test]
    fn new_remotes_get_a_free_valid_name() {
        use super::remote_name_problem;
        let names = |existing: &[&str]| {
            let existing: Vec<String> = existing.iter().map(|s| (*s).to_owned()).collect();
            let mut d = Dialog::add_remote(&crate::settings::Settings::default());
            d.name_remote(&existing);
            match d {
                Dialog::AddRemote { name, .. } => name,
                _ => unreachable!(),
            }
        };
        assert_eq!(names(&[]), "origin");
        assert_eq!(names(&["origin"]), "upstream");
        assert_eq!(names(&["origin", "upstream", "remote2"]), "remote3");
        assert!(remote_name_problem("origin", &["origin"]).is_some());
        assert!(remote_name_problem("", &[]).is_some());
        assert!(remote_name_problem("a/b", &[]).is_some());
        assert!(remote_name_problem("has space", &[]).is_some());
        assert!(remote_name_problem("upstream", &["origin"]).is_none());
    }

    /// Add remote and Clone take no URL with credentials, and none that git could read as an
    /// option.
    #[test]
    fn a_typed_url_with_credentials_or_a_leading_dash_is_refused() {
        use super::url::{UrlBuilder, UrlSource};
        let mut b = UrlBuilder::new(&crate::settings::Settings::default());
        b.source = UrlSource::Url;
        for url in [
            "https://user:secret@github.com/o/r.git",
            "https:///ghp_secret@github.com/o/r.git",
            "--mirror=push",
        ] {
            b.url = url.to_owned();
            assert_eq!(b.resolve(), None, "{url}");
        }
        b.url = " git@github.com:o/r.git ".to_owned();
        assert_eq!(b.resolve().as_deref(), Some("git@github.com:o/r.git"));
        // A GitLab host typed with a user name and password never builds a URL carrying them.
        b.source = UrlSource::GitLab;
        b.host = "user:secret@gitlab.example.com".to_owned();
        b.project = "g/p".to_owned();
        assert!(
            b.resolve().is_none_or(|u| !u.contains("secret")),
            "{:?}",
            b.resolve()
        );
    }

    /// The Add remote form says under the URL field why a typed URL is refused.
    #[test]
    fn the_add_remote_form_says_why_a_url_is_refused() {
        use super::url::UrlSource;
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut dialog = Dialog::add_remote(&crate::settings::Settings::default());
        if let Dialog::AddRemote { builder, .. } = &mut dialog {
            builder.source = UrlSource::Url;
            builder.url = "https://user:secret@github.com/o/r.git".into();
        }
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        let mut texts = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
                super::show(
                    &ctx,
                    &mut dialog,
                    None,
                    &mut picker,
                    false,
                    &crate::updater::Updater::default(),
                    "git version 2.55.0",
                );
            });
            output.textures_delta.clear();
            texts = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Remove the user name and password")),
            "{texts:?}"
        );
    }

    /// The legal documents read in a card that fills most of the room a modal has (O1), at the
    /// reading measure, in both the plain dialog and the consent.
    #[test]
    fn legal_documents_read_at_full_size() {
        for (w, h) in [(1440.0, 900.0), (1024.0, 700.0), (1920.0, 1080.0)] {
            for mut dialog in [
                Dialog::Legal(Document::Terms),
                Dialog::Consent {
                    step: Pending::Terms,
                    updated: false,
                    reading: Some(Document::Privacy),
                },
            ] {
                let ctx = egui::Context::default();
                crate::ui::fonts::install(&ctx);
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
                let mut picker = crate::ui::folder_picker::FolderPicker::default();
                let mut card = egui::Rect::NOTHING;
                for _ in 0..3 {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            ..Default::default()
                        },
                        |_| {
                            super::show(
                                &ctx,
                                &mut dialog,
                                None,
                                &mut picker,
                                false,
                                &crate::updater::Updater::default(),
                                "git version 2.55.0",
                            );
                        },
                    );
                    output.textures_delta.clear();
                    // Everything painted but the scrim is the card and its content.
                    card = output
                        .shapes
                        .iter()
                        .map(|s| s.shape.visual_bounding_rect().intersect(s.clip_rect))
                        .filter(|r| r.is_positive() && r.width() < w * 0.99)
                        .fold(egui::Rect::NOTHING, egui::Rect::union);
                }
                let room = crate::ui::kit::overlay::modal_room(&ctx);
                assert!(
                    card.height() >= room.height() * 0.85,
                    "{dialog:?} at {w}x{h} is {card:?}, room {room:?}"
                );
                assert!(
                    card.width()
                        >= crate::ui::kit::overlay::reading_measure(&ctx).min(room.width() * 0.9)
                        && card.width() <= room.width() + 1.0,
                    "{dialog:?} at {w}x{h} is {card:?}"
                );
            }
        }
    }

    /// Consent is never given by Enter, Escape or a click on the scrim, reading or not; Escape
    /// while reading goes back to the summary.
    #[test]
    fn consent_is_only_given_with_its_buttons() {
        use super::{Key, Outcome, show};
        for step in [Pending::Terms, Pending::CrashReports] {
            for reading in [None, Some(Document::Terms)] {
                let ctx = egui::Context::default();
                crate::ui::fonts::install(&ctx);
                let mut dialog = Dialog::Consent {
                    step,
                    updated: false,
                    reading,
                };
                let mut picker = crate::ui::folder_picker::FolderPicker::default();
                for key in [None, Some(Key::Enter), Some(Key::Escape), None] {
                    let mut events = vec![egui::Event::PointerButton {
                        pos: egui::pos2(2.0, 2.0),
                        button: egui::PointerButton::Primary,
                        pressed: key.is_none(),
                        modifiers: egui::Modifiers::NONE,
                    }];
                    if let Some(key) = key {
                        events.push(egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            events,
                            ..Default::default()
                        },
                        |_| {
                            let outcome = show(
                                &ctx,
                                &mut dialog,
                                None,
                                &mut picker,
                                false,
                                &crate::updater::Updater::default(),
                                "git version 2.55.0",
                            );
                            assert!(
                                matches!(outcome, Outcome::Keep),
                                "{step:?} reading {reading:?} answered on {key:?}"
                            );
                        },
                    );
                    output.textures_delta.clear();
                }
                assert!(
                    matches!(dialog, Dialog::Consent { reading: None, .. }),
                    "Escape goes back from reading"
                );
            }
        }
    }

    /// A risky merge's confirmation ("Merge anyway", danger) keeps both answers on one line
    /// and in view at the confirmation width, down to the smallest window.
    #[test]
    fn risky_merge_answers_never_wrap_or_clip() {
        for (w, h) in [(1440.0, 900.0), (562.5, 350.0)] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            let mut dialog = Dialog::ConfirmHost {
                title: "Merge pull request #428 anyway?".into(),
                body: "Checks failed and a reviewer requested changes.".into(),
                confirm: "Merge anyway".into(),
                danger: true,
                job: crate::hosting::HostJob::Merge(428, canopy_hosting::MergeMethod::Squash, None),
            };
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
            let mut picker = crate::ui::folder_picker::FolderPicker::default();
            let mut seen = Vec::new();
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |_| {
                        super::show(
                            &ctx,
                            &mut dialog,
                            None,
                            &mut picker,
                            false,
                            &crate::updater::Updater::default(),
                            "git version 2.55.0",
                        );
                    },
                );
                output.textures_delta.clear();
                seen = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t)
                            if ["Merge anyway", "Cancel"].contains(&t.galley.text()) =>
                        {
                            let r = t.visual_bounding_rect();
                            Some((
                                t.galley.text().to_owned(),
                                t.galley.rows.len() == 1
                                    && screen.contains_rect(r)
                                    && s.clip_rect.contains_rect(r),
                            ))
                        }
                        _ => None,
                    })
                    .collect();
            }
            assert_eq!(seen.len(), 2, "both answers drawn at {w}x{h}: {seen:?}");
            assert!(seen.iter().all(|(_, ok)| *ok), "{w}x{h}: {seen:?}");
        }
    }

    /// DL-F8: with no other branch, the Merge dialog says why under its select.
    #[test]
    fn an_empty_merge_branch_list_says_why() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut dialog = Dialog::Merge {
            target: String::new(),
            rebase: false,
            mode: canopy_core::MergeMode::Default,
        };
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        let mut texts = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
                super::show(
                    &ctx,
                    &mut dialog,
                    None,
                    &mut picker,
                    false,
                    &crate::updater::Updater::default(),
                    "git version 2.55.0",
                );
            });
            output.textures_delta.clear();
            texts = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        assert!(
            texts
                .iter()
                .any(|t| t == "This repository has no other branch to merge or rebase onto."),
            "{texts:?}"
        );
    }

    #[test]
    fn ref_names() {
        assert!(valid_ref_name("feature/login"));
        assert!(valid_ref_name("v1.2.0"));
        assert!(!valid_ref_name(""));
        assert!(!valid_ref_name("has space"));
        assert!(!valid_ref_name("a..b"));
        assert!(!valid_ref_name("-x"));
        assert!(!valid_ref_name("x.lock"));
        assert!(!valid_ref_name("x~1"));
    }

    /// review-views-11: the check agrees with `git check-ref-format`, so a name the dialog
    /// accepts is one git accepts. The name is checked as a whole (`--allow-onelevel`), as
    /// `--branch` would read `@` as the current branch; git's refusal of a branch starting with
    /// `-` is the one rule beyond it.
    #[test]
    fn ref_names_are_checked_as_git_checks_them() {
        let dir = tempfile::tempdir().expect("temporary folder");
        canopy_core::cli::run(dir.path(), &["init", "-q"]).expect("git init");
        let names = [
            "feature/login",
            "v1.2.0",
            "a@b",
            "@a",
            "x.lockx",
            "a.b/c",
            "\u{fc}ber",
            "a\u{a0}b",
            "a//b",
            "feature/.x",
            "x/y.lock/z",
            "a/.lock",
            "@",
            "HEAD",
            ".a",
            "a.",
            "a.lock",
            "a..b",
            "a@{b",
            "a b",
            "a\tb",
            "a\u{7f}b",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
            "a\\b",
            "/a",
            "a/",
            "-a",
            "",
        ];
        // `git check-ref-format --allow-onelevel` allows HEAD, but `HEAD` itself is
        // reserved and cannot safely name a new branch or tag in Canopy.
        for name in names {
            let out = canopy_core::cli::run_unchecked(
                dir.path(),
                &["check-ref-format", "--allow-onelevel", name],
            )
            .expect("git runs");
            assert_eq!(
                valid_ref_name(name),
                out.code == Some(0) && !name.starts_with('-') && name != "HEAD",
                "{name:?}: git says {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    #[test]
    fn clone_submission_keeps_its_form_until_the_worker_answers() {
        use super::{Key, Outcome, Settings, show};
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut dialog = Dialog::clone_default(&Settings::default());
        if let Dialog::Clone {
            builder, parent, ..
        } = &mut dialog
        {
            builder.project = "example/repository".into();
            *parent = "existing-parent".into();
        }
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        let mut kept = false;
        for pressed in [false, true] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: if pressed {
                        vec![egui::Event::Key {
                            key: Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::default(),
                        }]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                |_| {
                    let result = show(
                        &ctx,
                        &mut dialog,
                        None,
                        &mut picker,
                        false,
                        &crate::updater::Updater::default(),
                        "git version 2.55.0",
                    );
                    if pressed {
                        kept = matches!(result, Outcome::RunKeep(_));
                    }
                },
            );
            output.textures_delta.clear();
        }
        assert!(
            kept,
            "clone input must remain available after a destination refusal"
        );
    }
    #[test]
    fn clone_completion_preserves_failed_input_and_only_closes_its_pending_form() {
        let mut stack = DialogStack::default();
        let mut pending = Dialog::clone_default(&crate::settings::Settings::default());
        if let Dialog::Clone {
            builder,
            parent,
            pending,
            ..
        } = &mut pending
        {
            builder.project = "example/repository".into();
            *parent = "chosen-parent".into();
            *pending = true;
        }
        stack.open(pending);
        stack.open(Dialog::Legal(Document::Privacy));
        stack.finish_clone_with(Err(""));
        stack.close();
        assert!(
            matches!(stack.current(), Some(Dialog::Clone { builder, parent, pending: false, .. }) if builder.project == "example/repository" && parent == "chosen-parent")
        );
        if let Some(Dialog::Clone { pending, .. }) = stack.current_mut() {
            *pending = true;
        }
        stack.open(Dialog::clone_default(&crate::settings::Settings::default()));
        stack.finish_clone_with(Ok(()));
        stack.close();
        assert!(
            stack.current().is_none(),
            "only the completed pending form closes"
        );
    }
    #[test]
    fn a_failed_clone_says_why_in_its_own_form() {
        let mut stack = DialogStack::default();
        assert!(
            !stack.finish_clone_with(Err("fatal: no form")),
            "with no form waiting the caller reports the failure"
        );
        let mut form = Dialog::clone_default(&crate::settings::Settings::default());
        if let Dialog::Clone { pending, .. } = &mut form {
            *pending = true;
        }
        stack.open(form);
        let message = "fatal: repository 'https://github.com/example/missing/' not found";
        assert!(stack.finish_clone_with(Err(message)));
        assert!(matches!(
            stack.current(),
            Some(Dialog::Clone { pending: false, error: Some(e), .. }) if e == message
        ));
        if let Some(Dialog::Clone { pending, .. }) = stack.current_mut() {
            *pending = true;
        }
        assert!(stack.finish_clone_with(Ok(())));
        assert!(
            stack.current().is_none(),
            "a successful clone closes its form"
        );
    }
    #[test]
    fn update_progress_and_completion_dialogs_fit_their_content() {
        use super::show;
        use crate::selfupdate::{Installed, Phase};
        use crate::updater::{Available, InstallState, Updater};

        let available = Available {
            build: 1001,
            manifest: canopy_hosting::updates::UpdateManifest {
                version: "1001".into(),
                notes: String::new(),
                pub_date: None,
                release_page: None,
                platforms: std::collections::BTreeMap::default(),
            },
        };
        for state in [
            InstallState::Running {
                build: 1001,
                phase: Phase::Installing,
            },
            InstallState::Done {
                build: 1001,
                installed: Installed {
                    exe: Some("canopy".into()),
                    notes: Vec::new(),
                },
            },
        ] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            let mut updater = Updater::default();
            updater.install = state;
            let mut dialog = Dialog::Update(Box::new(available.clone()));
            let mut picker = crate::ui::folder_picker::FolderPicker::default();
            for _ in 0..5 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |_| {
                        show(
                            &ctx,
                            &mut dialog,
                            None,
                            &mut picker,
                            false,
                            &updater,
                            "git version 2.55.0",
                        );
                    },
                );
                output.textures_delta.clear();
            }
            let bounds = egui::containers::AreaState::load(&ctx, super::area_id(&dialog))
                .expect("update dialog area")
                .rect();
            assert!(
                bounds.height() < 300.0,
                "update actions must stay beside their content, not consume the viewport: {bounds:?}"
            );
        }
    }

    #[test]
    fn clone_form_title_and_actions_are_reachable_at_minimum_window_scale() {
        use super::{Settings, show};
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        crate::ui::theme::apply(
            &ctx,
            crate::ui::theme::ThemeId::Midnight,
            crate::ui::theme::Appearance::STILL,
        );
        let mut dialog = Dialog::clone_default(&Settings::default());
        if let Dialog::Clone { builder, .. } = &mut dialog {
            builder.project = "example/repository".into();
        }
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(562.5, 350.0));
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        for scrolling in [false, true] {
            let mut visible = false;
            for frame in 0..15 {
                let mut events = vec![egui::Event::PointerMoved(viewport.center())];
                if scrolling && frame == 0 {
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0.0, -1000.0),
                        modifiers: egui::Modifiers::default(),
                    });
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        events,
                        ..Default::default()
                    },
                    |_| {
                        show(
                            &ctx,
                            &mut dialog,
                            None,
                            &mut picker,
                            false,
                            &crate::updater::Updater::default(),
                            "git version 2.55.0",
                        );
                    },
                );
                output.textures_delta.clear();
                let label = if scrolling {
                    "Clone"
                } else {
                    "Clone repository"
                };
                visible = output.shapes.iter().any(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == label
                    {
                        let bounds = text.visual_bounding_rect();
                        return viewport.contains_rect(bounds)
                            && shape.clip_rect.contains_rect(bounds);
                    }
                    false
                });
            }
            let bounds = egui::containers::AreaState::load(&ctx, super::area_id(&dialog))
                .expect("clone dialog area")
                .rect();
            assert!(
                viewport.contains_rect(bounds),
                "dialog frame must fit the viewport: {bounds:?}"
            );
            assert!(
                visible,
                "Clone title and scrolled action must fit the minimum scaled window (scrolling={scrolling})"
            );
        }
    }
    #[test]
    fn consent_choices_are_visible_without_scrolling_at_maximum_zoom() {
        for step in [Pending::Terms, Pending::CrashReports] {
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            crate::ui::theme::apply(
                &ctx,
                crate::ui::theme::ThemeId::Daylight,
                crate::ui::theme::Appearance::STILL,
            );
            let mut dialog = Dialog::Consent {
                step,
                updated: false,
                reading: Some(Document::Privacy),
            };
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(562.5, 350.0));
            let mut picker = crate::ui::folder_picker::FolderPicker::default();
            let mut visible = 0;
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |_| {
                        super::show(
                            &ctx,
                            &mut dialog,
                            None,
                            &mut picker,
                            false,
                            &crate::updater::Updater::default(),
                            "git version 2.55.0",
                        );
                    },
                );
                output.textures_delta.clear();
                let choices = match step {
                    Pending::Terms => ["I agree", "Quit"],
                    Pending::CrashReports => ["Send crash reports", "Don't send"],
                };
                visible = output
                    .shapes
                    .iter()
                    .filter(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && choices.contains(&text.galley.text())
                        {
                            let rect = text.visual_bounding_rect();
                            viewport.contains_rect(rect) && shape.clip_rect.contains_rect(rect)
                        } else {
                            false
                        }
                    })
                    .count();
            }
            assert_eq!(
                visible, 2,
                "both consent choices must stay visible while reading"
            );
        }
    }
}
