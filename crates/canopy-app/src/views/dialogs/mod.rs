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
use crate::ui::kit::row::Disclosure;
use crate::ui::kit::select::Select;
use crate::ui::kit::surface::InsetBox;
use crate::ui::kit::toggle::{Checkbox, RadioGroup};
use crate::ui::kit::{Button, Size, Tone, paint};
use crate::ui::legal::{Document, Pending};
use crate::ui::theme::{self, palette};
use crate::worker::Op;

mod common;
mod credential;
mod stack;
mod start;
mod update;
mod url;

use common::{
    answer, bullet, confirm, document, field, field_with_error, folder_row, home,
    identity_in_scope, note, swallow_enter, valid_ref_name,
};
use credential::credential;
pub use stack::DialogStack;
pub use start::Start;
pub use url::UrlBuilder;

/// Every dialog Canopy shows.
#[derive(Debug, Clone)]
pub enum Dialog {
    Open {
        path: String,
    },
    Clone {
        builder: UrlBuilder,
        parent: String,
        pending: bool,
    },
    Init {
        path: String,
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
        op: Op,
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
    },
}

/// What a dialog wants after this frame.
pub enum Outcome {
    Keep,
    Close,
    Run(Vec<Action>),
    RunKeep(Vec<Action>),
}

impl Dialog {
    /// The dialog answering a credential prompt.
    pub fn credential(prompt: Prompt) -> Self {
        Self::Credential {
            prompt,
            answer: String::new(),
        }
    }

    /// The open-repository dialog, prefilled with the home directory.
    pub fn open() -> Self {
        Self::Open { path: home() }
    }

    /// The clone dialog with the default clone folder as the destination.
    pub fn clone_default(settings: &Settings) -> Self {
        Self::Clone {
            builder: UrlBuilder::new(settings),
            parent: settings.clone_parent(),
            pending: false,
        }
    }

    /// The add-remote dialog.
    pub fn add_remote(settings: &Settings) -> Self {
        Self::AddRemote {
            name: "origin".into(),
            builder: UrlBuilder::new(settings),
            publish: false,
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
                .description("Git stores an author name and email in every new commit. Enter the details you want to use. Canopy will not guess them, and commits already made keep their authors.")
                .body(|ui| {
                    field(ui, "Name", name, "e.g. Ada Lovelace", true);
                    field(ui, "Email", email, "e.g. ada@example.com", false);
                    note(ui, "Saved to your global Git configuration, so every repository without its own identity uses it. You can change it later in Settings → Git.", p.text_dim);
                    if !ok && !busy {
                        note(ui, missing, p.text_faint);
                    }
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
        Dialog::Open { path } => {
            let ok = !path.trim().is_empty();
            let result = KitDialog::new(salt)
                .title("Open repository")
                .icon(icon::FOLDER_OPEN)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    let start = crate::settings::expand_home(path);
                    folder_row(ui, picker, "open", "Folder", path, "", &start, true);
                })
                .footer(|f| confirm(f, "Open", ok, "Enter a folder"))
                .show(ctx);
            answer(result, || {
                if path.trim().is_empty() {
                    Outcome::Keep
                } else {
                    Outcome::Run(vec![Action::OpenRepo(path.trim().into())])
                }
            })
        }
        Dialog::Clone {
            builder,
            parent,
            pending,
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
                        folder_row(ui, picker, "clone", "Clone into", parent, "", &start, false);
                        target = clone_target(builder, parent);
                        if let Some((_, dest)) = &target {
                            note(ui, format!("→ {}", dest.display()), p.text_faint);
                        }
                    });
                    if busy {
                        ui.horizontal(|ui| {
                            Spinner::new().size(Size::Sm).label("the clone").show(ui);
                            ui.label(RichText::new("Cloning repository…").size(m.body_size));
                        });
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
                    Outcome::RunKeep(vec![Action::Clone { url, dest }])
                }
                _ => Outcome::Keep,
            })
        }
        Dialog::Init { path } => {
            let ok = !path.trim().is_empty();
            let result = KitDialog::new(salt)
                .title("New repository")
                .icon(icon::PLANT)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    let start = crate::settings::expand_home(&home());
                    folder_row(
                        ui,
                        picker,
                        "init",
                        "Folder (created if missing)",
                        path,
                        "~/src/my-project",
                        &start,
                        true,
                    );
                })
                .footer(|f| confirm(f, "Create", ok, "Enter a folder"))
                .show(ctx);
            answer(result, || {
                if path.trim().is_empty() {
                    Outcome::Keep
                } else {
                    Outcome::Run(vec![Action::Init(crate::settings::expand_home(path))])
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
                .body(|ui| {
                    field(ui, "New name", new, "", true);
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
        Dialog::NewTag {
            name,
            target,
            message,
        } => {
            let ok = valid_ref_name(name.trim());
            let mut submitted = false;
            let result = KitDialog::new(salt)
                .title(format!("Tag {}", short_id(*target)))
                .icon(icon::TAG)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    field(ui, "Name", name, "v1.2.0", true);
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
        } => {
            let ok = valid_ref_name(name.trim()) && builder.resolve().is_some();
            let mut dialog = KitDialog::new(salt)
                .width(DialogWidth::Form)
                .initial_focus(InitialFocus::Body);
            dialog = if *publish {
                dialog
                    .title("Publish repository")
                    .icon(icon::CLOUD_ARROW_UP)
                    .description("This repository is local only. Create an empty repository on GitHub or GitLab, enter it here, and Canopy adds it as a remote and pushes the current branch.")
            } else {
                dialog.title("Add remote").icon(icon::CLOUD)
            };
            let label = if *publish {
                "Add and push"
            } else {
                "Add remote"
            };
            let result = dialog
                .body(|ui| {
                    field(ui, "Name", name, "origin", false);
                    builder.ui(ui, true);
                })
                .footer(|f| confirm(f, label, ok, "Enter a remote name and a repository"))
                .show(ctx);
            answer(result, || match builder.resolve() {
                Some(url) if valid_ref_name(name.trim()) => {
                    let mut run = vec![Action::Op(Op::AddRemote {
                        name: name.trim().to_owned(),
                        url,
                    })];
                    if *publish {
                        run.push(Action::Op(Op::Push { force: false }));
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
                    note(
                        ui,
                        format!("{}  {}  →", icon::GIT_BRANCH, request.source),
                        p.text_dim,
                    );
                    let options: Vec<(&str, &str)> = branches
                        .iter()
                        .filter(|b| **b != request.source)
                        .map(|b| (b.as_str(), b.as_str()))
                        .collect();
                    let mut pick = options
                        .iter()
                        .find(|(b, _)| *b == request.target)
                        .map_or("", |(b, _)| *b);
                    Select::new(&mut pick, &options)
                        .name("Target branch")
                        .placeholder("Choose a branch…")
                        .id_salt("request-target")
                        .show(ui);
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
            op,
        } => {
            let result =
                confirmation(salt, title, body, confirm, *danger, icon::QUESTION, |_| {}).show(ctx);
            answer(result, || Outcome::Run(vec![Action::Op(op.clone())]))
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
                        note(ui, format!("Trusting {} covers every repository inside it, including ones you clone later. Only do that for a folder that is entirely yours.", name(parent)), p.text_faint);
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
        Dialog::Credential { prompt, answer } => credential(ctx, salt, prompt, answer),
        Dialog::Update(a) => update::update(ctx, salt, a, updater),
        Dialog::WhatsNew { version, notes } => {
            let result = KitDialog::new(salt)
                .title(format!("What's new in Canopy {version}"))
                .icon(icon::SPARKLE)
                .width(DialogWidth::Form)
                .body(|ui| {
                    document(ui, "whats-new", 360.0, notes);
                    if LinkButton::new("All releases")
                        .external(true)
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
                .description("Sorry about that. A crash report was saved on this computer; nothing was sent anywhere. Sharing it with the developers helps fix the problem.")
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
        Dialog::Legal(doc) => {
            let glyph = match doc {
                Document::Privacy => icon::SHIELD_CHECK,
                Document::Terms => icon::SCROLL,
            };
            let height = (ctx.content_rect().height() - 180.0).clamp(0.0, 420.0);
            let result = KitDialog::new(salt)
                .title(doc.title())
                .icon(glyph)
                .width(DialogWidth::Form)
                .body(|ui| document(ui, ("legal", doc.title()), height, doc.body()))
                .footer(|f| {
                    f.cancel_label("Close");
                })
                .show(ctx);
            answer(result, || Outcome::Close)
        }
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
            f.cancel().confirm(confirm).hint(if danger {
                "Esc cancels"
            } else {
                "Enter confirms · Esc cancels"
            });
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
    Select::new(&mut pick, &options)
        .name("Branch")
        .placeholder("Choose a branch…")
        .id_salt("merge-target")
        .show(ui);
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
                    "Please read and accept the new Terms of Service and Privacy Policy to keep using Canopy."
                } else {
                    "Before you start, please read and accept the Terms of Service and the Privacy Policy."
                })
                .body(|ui| {
                    for (glyph, text) in [
                        (icon::LOCK_SIMPLE, "Your repositories and tokens are never sent to NecturaLabs. Requests to GitHub and GitLab go straight to them."),
                        (icon::ARROW_CIRCLE_UP, "At most once a day, Canopy checks GitHub for a new version. You can turn this off."),
                        (icon::FIRST_AID_KIT, "Crash reports are sent only if you agree. You choose next, and can change it any time."),
                    ] {
                        bullet(ui, glyph, p.accent, text);
                    }
                    for doc in [Document::Terms, Document::Privacy] {
                        read(ui, doc, reading, 260.0);
                    }
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
            if *reading != Some(Document::Privacy) {
                *reading = None;
            }
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
                    read(ui, Document::Privacy, reading, 240.0);
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

/// A legal document that opens inline under its title (one at a time).
fn read(ui: &mut Ui, doc: Document, reading: &mut Option<Document>, height: f32) {
    let mut open = *reading == Some(doc);
    let was = open;
    Disclosure::new(("read", doc.title()), doc.title(), &mut open)
        .icon(icon::SCROLL)
        .show(ui, |ui| {
            document(ui, ("legal", doc.title()), height, doc.body());
        });
    if open != was {
        *reading = open.then_some(doc);
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
                .font(FontId::new(m.display_size, theme::semibold()))
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
        let links: [(&str, bool, Outcome); 4] = [
            ("What's new", false, Outcome::Run(vec![Action::WhatsNew])),
            (
                "Releases",
                true,
                Outcome::Run(vec![Action::OpenUrl(
                    crate::updater::RELEASES_PAGE.to_owned(),
                )]),
            ),
            (
                "Privacy Policy",
                false,
                Outcome::Run(vec![Action::OpenDialog(Dialog::Legal(Document::Privacy))]),
            ),
            (
                "Terms of Service",
                false,
                Outcome::Run(vec![Action::OpenDialog(Dialog::Legal(Document::Terms))]),
            ),
        ];
        let width_id = ui.id().with("about-links-width");
        let last: f32 = ui.data(|d| d.get_temp(width_id)).unwrap_or(0.0);
        let row = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::MD;
            ui.add_space(((ui.available_width() - last) / 2.0).max(0.0));
            let start = ui.cursor().left();
            for (label, external, target) in links {
                if LinkButton::new(label)
                    .external(external)
                    .text_size(m.meta_size)
                    .show(ui)
                    .clicked()
                {
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
    use super::{Dialog, DialogStack, Oid, Start, valid_ref_name};
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
        } = &mut pending
        {
            builder.project = "example/repository".into();
            *parent = "chosen-parent".into();
            *pending = true;
        }
        stack.open(pending);
        stack.open(Dialog::Legal(Document::Privacy));
        stack.finish_clone(false);
        stack.close();
        assert!(
            matches!(stack.current(), Some(Dialog::Clone { builder, parent, pending: false }) if builder.project == "example/repository" && parent == "chosen-parent")
        );
        if let Some(Dialog::Clone { pending, .. }) = stack.current_mut() {
            *pending = true;
        }
        stack.open(Dialog::clone_default(&crate::settings::Settings::default()));
        stack.finish_clone(true);
        stack.close();
        assert!(
            stack.current().is_none(),
            "only the completed pending form closes"
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
