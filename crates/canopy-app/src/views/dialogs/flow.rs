//! git-flow: set it up, start a branch, finish one.

use canopy_core::{FlowConfig, FlowFinish, FlowKind, RepoSnapshot};

use super::*;
use crate::worker::FlowPush;

/// The tracked files have changes (staged, or unstaged beyond untracked files): what `git flow`
/// and [`canopy_core::Repo::flow_finish`] refuse to start from.
fn has_tracked_changes(snap: &RepoSnapshot) -> bool {
    !snap.status.staged.is_empty()
        || !snap.status.conflicted.is_empty()
        || snap
            .status
            .unstaged
            .iter()
            .any(|f| f.kind != canopy_core::ChangeKind::Untracked)
}

fn has_branch(snap: &RepoSnapshot, name: &str) -> bool {
    snap.branches.iter().any(|b| b.name == name)
}

// ------------------------------------------------------------------------------------- init

/// What the set-up dialog edits: the production and development branches, the prefixes and the
/// version tag prefix.
#[derive(Debug, Clone)]
pub struct FlowInitForm {
    pub config: FlowConfig,
}

impl FlowInitForm {
    /// Prefilled from the stored configuration, else the AVH defaults with `main` or `master`
    /// (whichever exists) as production.
    pub fn new(snap: Option<&RepoSnapshot>) -> Self {
        let config = snap.map_or_else(FlowConfig::default, |s| {
            s.workflow.flow.clone().unwrap_or_else(|| {
                FlowConfig::suggested(has_branch(s, "main"), has_branch(s, "master"))
            })
        });
        Self { config }
    }

    /// The values as typed, without surrounding blanks.
    fn trimmed(&self) -> FlowConfig {
        let c = &self.config;
        FlowConfig {
            master: c.master.trim().to_owned(),
            develop: c.develop.trim().to_owned(),
            feature: c.feature.trim().to_owned(),
            bugfix: c.bugfix.trim().to_owned(),
            release: c.release.trim().to_owned(),
            hotfix: c.hotfix.trim().to_owned(),
            support: c.support.clone(),
            version_tag: c.version_tag.trim().to_owned(),
        }
    }
}

/// Why `config` cannot be written yet, in the core's words where it has them.
fn init_problem(config: &FlowConfig, snap: Option<&RepoSnapshot>) -> Option<String> {
    if snap.is_some_and(|s| s.head.oid.is_none()) {
        return Some(
            "The repository has no commit yet. git-flow needs at least one commit on the \
             production branch."
                .to_owned(),
        );
    }
    for branch in [&config.master, &config.develop] {
        if !valid_ref_name(branch) {
            return Some(format!("'{branch}' is not a valid branch name."));
        }
    }
    if config.master == config.develop {
        return Some("The production and development branches must differ.".to_owned());
    }
    for kind in FlowKind::ALL {
        if !valid_ref_name(&config.branch(kind, "name")) {
            return Some(format!(
                "The {} prefix '{}' does not make valid branch names.",
                kind.label().to_lowercase(),
                config.prefix(kind)
            ));
        }
    }
    if !valid_ref_name(&config.tag("1.0")) {
        return Some(format!(
            "The version tag prefix '{}' does not make valid tag names.",
            config.version_tag
        ));
    }
    if let Some(s) = snap
        && !has_branch(s, &config.master)
    {
        return Some(format!(
            "The production branch '{}' does not exist. git-flow needs at least one commit on it.",
            config.master
        ));
    }
    None
}

pub(super) fn show_init(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut FlowInitForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let snap = tab.and_then(|t| t.snap.as_deref());
    let config = form.trimmed();
    let problem = init_problem(&config, snap);
    let creates = snap.is_some_and(|s| !has_branch(s, &config.develop));
    let result = KitDialog::new(salt)
        .title("Set up git-flow")
        .icon(icon::TREE_STRUCTURE)
        .width(DialogWidth::Form)
        .initial_focus(InitialFocus::Body)
        .description(
            "Names the long-lived branches and the prefixes of feature, bugfix, release and \
             hotfix branches. It is stored in this repository's Git configuration, where \
             `git flow` reads it too.",
        )
        .body(|ui| {
            let c = &mut form.config;
            ui.columns(2, |cols| {
                field(
                    &mut cols[0],
                    "Production branch",
                    &mut c.master,
                    "main",
                    true,
                );
                field(
                    &mut cols[1],
                    "Development branch",
                    &mut c.develop,
                    "develop",
                    false,
                );
            });
            ui.columns(2, |cols| {
                field(
                    &mut cols[0],
                    "Feature prefix",
                    &mut c.feature,
                    "feature/",
                    false,
                );
                field(
                    &mut cols[1],
                    "Bugfix prefix",
                    &mut c.bugfix,
                    "bugfix/",
                    false,
                );
            });
            ui.columns(2, |cols| {
                field(
                    &mut cols[0],
                    "Release prefix",
                    &mut c.release,
                    "release/",
                    false,
                );
                field(
                    &mut cols[1],
                    "Hotfix prefix",
                    &mut c.hotfix,
                    "hotfix/",
                    false,
                );
            });
            field(
                ui,
                "Version tag prefix",
                &mut c.version_tag,
                "v (or empty)",
                false,
            );
            if let Some(problem) = &problem {
                note(ui, problem, p.danger);
            } else if creates {
                note(
                    ui,
                    format!(
                        "{} does not exist yet; it is created from {}.",
                        config.develop, config.master
                    ),
                    p.text_dim,
                );
            }
        })
        .footer(|f| {
            confirm(
                f,
                "Set up git-flow",
                problem.is_none(),
                problem.as_deref().unwrap_or(""),
            );
        })
        .show(ctx);
    answer(result, || {
        if problem.is_some() {
            return Outcome::Keep;
        }
        Outcome::Run(vec![Action::Op(Op::FlowInit(config.clone()))])
    })
}

// ------------------------------------------------------------------------------------ start

/// What the start dialog edits.
#[derive(Debug, Clone)]
pub struct FlowStartForm {
    pub kind: FlowKind,
    pub name: String,
    /// The branch to start from; the kind's own (`FlowConfig::default_base`) until picked.
    pub base: Option<String>,
}

impl FlowStartForm {
    pub fn new(kind: Option<FlowKind>) -> Self {
        Self {
            kind: kind.unwrap_or(FlowKind::Feature),
            name: String::new(),
            base: None,
        }
    }
}

/// Why `name` of `kind` cannot start from `base` yet: what [`canopy_core::Repo::flow_start`]
/// refuses, worked out from the snapshot.
fn start_problem(
    snap: &RepoSnapshot,
    config: &FlowConfig,
    kind: FlowKind,
    name: &str,
    base: &str,
) -> Option<String> {
    let branch = config.branch(kind, name);
    if !valid_ref_name(&branch) {
        return Some(format!("'{branch}' is not a valid branch name."));
    }
    if has_branch(snap, &branch) {
        return Some(format!("Branch '{branch}' already exists."));
    }
    if !has_branch(snap, base) {
        return Some(format!("The base '{base}' does not exist."));
    }
    if matches!(kind, FlowKind::Release | FlowKind::Hotfix) {
        if has_tracked_changes(snap) {
            return Some("The working tree has changes. Commit or stash them first.".to_owned());
        }
        if let Some(other) = snap
            .branches
            .iter()
            .find(|b| config.classify(&b.name).is_some_and(|(k, _)| k == kind))
        {
            return Some(format!(
                "Finish '{}' first: git-flow allows one {} branch at a time.",
                other.name,
                kind.label().to_lowercase()
            ));
        }
        let tag = config.tag(name);
        if snap.tags.iter().any(|t| t.name == tag) {
            return Some(format!("Tag '{tag}' already exists."));
        }
    }
    None
}

pub(super) fn show_start(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut FlowStartForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let snap = tab.and_then(|t| t.snap.as_deref());
    let config = snap.and_then(|s| s.workflow.flow.as_ref());
    let (Some(snap), Some(config)) = (snap, config) else {
        let result = KitDialog::new(salt)
            .title("Start a git-flow branch")
            .icon(icon::TREE_STRUCTURE)
            .width(DialogWidth::Form)
            .body(|ui| {
                note(
                    ui,
                    "git-flow is not set up in this repository. Set it up first.",
                    p.text_dim,
                );
            })
            .footer(|f| {
                f.cancel();
            })
            .show(ctx);
        return answer(result, || Outcome::Keep);
    };
    let default = config.default_base(form.kind);
    let base = form.base.as_deref().unwrap_or(default).to_owned();
    let name = form.name.trim().to_owned();
    let problem = (!name.is_empty())
        .then(|| start_problem(snap, config, form.kind, &name, &base))
        .flatten();
    let ok = !name.is_empty() && problem.is_none();
    let kinds = FlowKind::ALL.map(|k| (k, k.label()));
    let verb = form.kind.label().to_lowercase();
    let result = KitDialog::new(salt)
        .title("Start a git-flow branch")
        .icon(icon::TREE_STRUCTURE)
        .width(DialogWidth::Form)
        .initial_focus(InitialFocus::Body)
        .body(|ui| {
            let before = form.kind;
            Segmented::new(&mut form.kind, &kinds)
                .label("Kind")
                .id_salt("flow-kind")
                .show(ui);
            if form.kind != before {
                form.base = None;
            }
            field_with_error(
                ui,
                "Name",
                &mut form.name,
                "login",
                true,
                problem.as_deref(),
            );
            note(
                ui,
                format!(
                    "{}  Creates {} and checks it out",
                    icon::GIT_BRANCH,
                    config.branch(form.kind, if name.is_empty() { "…" } else { &name })
                ),
                p.text_faint,
            );
            let names: Vec<&str> = snap.branches.iter().map(|b| b.name.as_str()).collect();
            let options: Vec<(&str, &str)> = names.iter().map(|n| (*n, *n)).collect();
            let mut pick = names.iter().copied().find(|n| *n == base).unwrap_or("");
            if Select::new(&mut pick, &options)
                .label("Start from")
                .placeholder("Choose a branch…")
                .id_salt("flow-base")
                .show(ui)
                .changed()
            {
                form.base = Some(pick.to_owned());
            }
            if matches!(form.kind, FlowKind::Release | FlowKind::Hotfix) {
                note(
                    ui,
                    "A release or hotfix needs a clean working tree, and only one of each can be open.",
                    p.text_dim,
                );
            }
        })
        .footer(|f| {
            confirm(
                f,
                &format!("Start {verb}"),
                ok,
                if name.is_empty() {
                    "Enter a name"
                } else {
                    "Fix the problem above"
                },
            );
        })
        .show(ctx);
    answer(result, || {
        if !ok {
            return Outcome::Keep;
        }
        Outcome::Run(vec![Action::Op(Op::FlowStart {
            kind: form.kind,
            name: name.clone(),
            base: Some(base.clone()),
        })])
    })
}

// ------------------------------------------------------------------------------------ finish

/// What the finish dialog edits.
#[derive(Debug, Clone)]
pub struct FlowFinishForm {
    /// The branch being finished, with its prefix.
    pub branch: String,
    pub delete_branch: bool,
    pub squash: bool,
    pub rebase: bool,
    pub tag: bool,
    pub tag_message: String,
    pub push: bool,
    pub remote: String,
    pub delete_remote: bool,
}

impl FlowFinishForm {
    /// For `branch`, else the checked-out branch when it is a git-flow branch, else the first
    /// one there is.
    pub fn new(branch: Option<String>, snap: Option<&RepoSnapshot>) -> Self {
        let flow = snap.and_then(|s| s.workflow.flow.as_ref());
        let is_flow = |name: &str| flow.is_some_and(|f| f.classify(name).is_some());
        let branch = branch
            .or_else(|| {
                snap.and_then(|s| s.head.branch.clone())
                    .filter(|b| is_flow(b))
            })
            .or_else(|| {
                snap.and_then(|s| s.branches.iter().find(|b| is_flow(&b.name)))
                    .map(|b| b.name.clone())
            })
            .unwrap_or_default();
        let remote = snap
            .and_then(|s| {
                s.remotes
                    .iter()
                    .find(|r| r.name == "origin")
                    .or_else(|| s.remotes.first())
            })
            .map(|r| r.name.clone())
            .unwrap_or_default();
        let defaults = FlowFinish::default();
        Self {
            branch,
            delete_branch: defaults.delete_branch,
            squash: defaults.squash,
            rebase: defaults.rebase,
            tag: defaults.tag,
            tag_message: String::new(),
            push: false,
            remote,
            delete_remote: false,
        }
    }
}

/// The branch a finished release or hotfix merges back into: develop, except for a hotfix while
/// a release branch is open ([`canopy_core::Repo::flow_back_merge_target`]).
fn back_merge_target(snap: &RepoSnapshot, config: &FlowConfig, kind: FlowKind) -> String {
    if kind == FlowKind::Hotfix
        && let Some(release) = snap.branches.iter().find(|b| {
            config
                .classify(&b.name)
                .is_some_and(|(k, _)| k == FlowKind::Release)
        })
    {
        return release.name.clone();
    }
    config.develop.clone()
}

/// What finishing does, as sentences for the dialog.
fn finish_summary(
    snap: &RepoSnapshot,
    config: &FlowConfig,
    kind: FlowKind,
    name: &str,
    form: &FlowFinishForm,
) -> Vec<String> {
    if kind.is_release_like() {
        let back = back_merge_target(snap, config, kind);
        let mut lines = vec![format!("Merges into {}.", config.master)];
        if form.tag {
            lines.push(format!("Tags it {}.", config.tag(name)));
        }
        lines.push(if back == config.develop {
            format!(
                "Merges {} back into {}.",
                if form.tag { "the tag" } else { &config.master },
                back
            )
        } else {
            format!(
                "Merges {} into the open release {back}, not {}: the release carries it to {}.",
                if form.tag { "the tag" } else { &config.master },
                config.develop,
                config.develop
            )
        });
        lines
    } else {
        let how = if form.squash {
            "as one commit"
        } else {
            "with a merge commit, or a fast-forward when the branch has one commit and develop has not moved"
        };
        vec![format!("Merges into {} {how}.", config.develop)]
    }
}

pub(super) fn show_finish(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut FlowFinishForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let snap = tab.and_then(|t| t.snap.as_deref());
    let config = snap.and_then(|s| s.workflow.flow.as_ref());
    let (Some(snap), Some(config)) = (snap, config) else {
        let result = KitDialog::new(salt)
            .title("Finish a git-flow branch")
            .icon(icon::FLAG_CHECKERED)
            .width(DialogWidth::Form)
            .body(|ui| {
                note(
                    ui,
                    "git-flow is not set up in this repository. Set it up first.",
                    p.text_dim,
                );
            })
            .footer(|f| {
                f.cancel();
            })
            .show(ctx);
        return answer(result, || Outcome::Keep);
    };
    let flow_branches: Vec<&str> = snap
        .branches
        .iter()
        .filter(|b| config.classify(&b.name).is_some())
        .map(|b| b.name.as_str())
        .collect();
    let classified = config
        .classify(&form.branch)
        .map(|(k, n)| (k, n.to_owned()));
    let unavailable = snap.workflow.finish_unavailable;
    let problem: Option<String> = match &classified {
        None => Some("Choose the git-flow branch to finish.".to_owned()),
        Some(_) if unavailable.is_some() => unavailable.map(str::to_owned),
        Some(_) if has_tracked_changes(snap) => {
            Some("The working tree has changes. Commit or stash them first.".to_owned())
        }
        Some((kind, name)) if kind.is_release_like() && form.tag => {
            let tag = config.tag(name);
            snap.tags
                .iter()
                .any(|t| t.name == tag)
                .then(|| format!("Tag '{tag}' already exists."))
        }
        Some(_) => None,
    };
    let destructive = form.push && form.delete_remote;
    let confirm_label = match (&classified, form.push) {
        (Some((kind, _)), true) => format!("Finish {} and push", kind.label().to_lowercase()),
        (Some((kind, _)), false) => format!("Finish {}", kind.label().to_lowercase()),
        _ => "Finish".to_owned(),
    };
    let mut dialog = KitDialog::new(salt)
        .title("Finish a git-flow branch")
        .icon(icon::FLAG_CHECKERED)
        .width(DialogWidth::Form)
        .initial_focus(InitialFocus::Body);
    if destructive {
        dialog = dialog.destructive();
    }
    let result = dialog
        .body(|ui| {
            let options: Vec<(&str, &str)> = flow_branches.iter().map(|n| (*n, *n)).collect();
            let mut pick = flow_branches
                .iter()
                .copied()
                .find(|n| *n == form.branch)
                .unwrap_or("");
            if Select::new(&mut pick, &options)
                .label("Branch")
                .placeholder("Choose a branch…")
                .enabled(!options.is_empty())
                .id_salt("flow-finish-branch")
                .show(ui)
                .changed()
            {
                pick.clone_into(&mut form.branch);
            }
            let Some((kind, name)) = &classified else {
                note(
                    ui,
                    "This repository has no feature, bugfix, release or hotfix branch.",
                    p.text_dim,
                );
                return;
            };
            if let Some(reason) = unavailable {
                Callout::new(Tone::Warning, reason)
                    .title("Finishing is unavailable")
                    .show(ui);
            }
            for line in finish_summary(snap, config, *kind, name, form) {
                note(ui, format!("{}  {line}", icon::ARROW_RIGHT), p.text_dim);
            }
            ui.add_space(theme::SM);
            Checkbox::new(&mut form.delete_branch, "Delete the branch afterwards").show(ui);
            if kind.is_release_like() {
                Checkbox::new(&mut form.tag, format!("Tag {}", config.tag(name))).show(ui);
                if form.tag {
                    field(
                        ui,
                        "Tag message",
                        &mut form.tag_message,
                        &config.tag(name),
                        false,
                    );
                }
            } else {
                Checkbox::new(&mut form.squash, "Squash into one commit").show(ui);
                Checkbox::new(
                    &mut form.rebase,
                    format!("Rebase onto {} first (rewrites the branch)", config.develop),
                )
                .show(ui);
            }
            ui.add_space(theme::SM);
            if snap.remotes.is_empty() {
                note(ui, "No remote to push to.", p.text_faint);
            } else {
                Checkbox::new(&mut form.push, "Push afterwards").show(ui);
                if form.push {
                    if snap.remotes.len() > 1 {
                        let remotes: Vec<(&str, &str)> = snap
                            .remotes
                            .iter()
                            .map(|r| (r.name.as_str(), r.name.as_str()))
                            .collect();
                        let mut pick = remotes
                            .iter()
                            .map(|(n, _)| *n)
                            .find(|n| *n == form.remote)
                            .unwrap_or("");
                        if Select::new(&mut pick, &remotes)
                            .label("Push to")
                            .id_salt("flow-finish-remote")
                            .show(ui)
                            .changed()
                        {
                            pick.clone_into(&mut form.remote);
                        }
                    }
                    let pushes = if kind.is_release_like() {
                        format!(
                            "Pushes {}, {}{} to {}.",
                            config.develop,
                            config.master,
                            if form.tag { " and the tag" } else { "" },
                            form.remote
                        )
                    } else {
                        format!("Pushes {} to {}.", config.develop, form.remote)
                    };
                    note(ui, pushes, p.text_dim);
                    Checkbox::new(
                        &mut form.delete_remote,
                        format!("Also delete {} on {}", form.branch, form.remote),
                    )
                    .show(ui);
                    if form.delete_remote {
                        note(
                            ui,
                            "The branch is deleted on the server, and everyone loses access to it.",
                            p.danger,
                        );
                    }
                }
            }
            if let Some(problem) = &problem
                && unavailable.is_none()
            {
                ui.add_space(theme::SM);
                note(ui, problem, p.danger);
            }
        })
        .footer(|f| {
            confirm(
                f,
                &confirm_label,
                problem.is_none(),
                problem.as_deref().unwrap_or(""),
            );
        })
        .show(ctx);
    answer(result, || {
        let Some((kind, name)) = classified.clone() else {
            return Outcome::Keep;
        };
        if problem.is_some() {
            return Outcome::Keep;
        }
        let release_like = kind.is_release_like();
        let options = FlowFinish {
            delete_branch: form.delete_branch,
            squash: !release_like && form.squash,
            rebase: !release_like && form.rebase,
            tag: release_like && form.tag,
            tag_message: Some(form.tag_message.trim().to_owned()).filter(|m| !m.is_empty()),
        };
        let push = (form.push && !form.remote.is_empty()).then(|| FlowPush {
            remote: form.remote.clone(),
            delete_remote: form.delete_remote,
        });
        Outcome::Run(vec![Action::Op(Op::FlowFinish {
            kind,
            name,
            options,
            push,
        })])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use canopy_core::{BranchInfo, FileEntry, HeadInfo};

    fn branch(name: &str, is_head: bool) -> BranchInfo {
        BranchInfo {
            name: name.to_owned(),
            target: None,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            worktree: None,
        }
    }

    fn snap(names: &[&str]) -> RepoSnapshot {
        RepoSnapshot {
            head: HeadInfo {
                branch: Some("develop".into()),
                oid: Some(Oid::ZERO_SHA1),
                detached: false,
            },
            branches: names.iter().map(|n| branch(n, *n == "develop")).collect(),
            ..RepoSnapshot::default()
        }
    }

    #[test]
    fn the_init_form_starts_from_main_or_master_whichever_exists() {
        let s = snap(&["master", "develop"]);
        assert_eq!(FlowInitForm::new(Some(&s)).config.master, "master");
        let s = snap(&["main", "master"]);
        assert_eq!(FlowInitForm::new(Some(&s)).config.master, "main");
        assert_eq!(FlowInitForm::new(None).config.master, "master");
    }

    #[test]
    fn the_init_form_says_what_the_core_would_refuse() {
        let s = snap(&["main"]);
        let config = FlowConfig::suggested(true, false);
        assert_eq!(init_problem(&config, Some(&s)), None);

        let mut empty = snap(&[]);
        empty.head.oid = None;
        assert!(init_problem(&config, Some(&empty)).is_some_and(|p| p.contains("no commit")));

        let mut bad = config.clone();
        "bad..name".clone_into(&mut bad.develop);
        assert!(init_problem(&bad, Some(&s)).is_some_and(|p| p.contains("not a valid branch")));
        let mut same = config.clone();
        same.develop.clone_from(&same.master);
        assert!(init_problem(&same, Some(&s)).is_some_and(|p| p.contains("must differ")));
        let mut prefix = config.clone();
        "fe at/".clone_into(&mut prefix.feature);
        assert!(init_problem(&prefix, Some(&s)).is_some_and(|p| p.contains("feature prefix")));
        let mut tag = config.clone();
        "v ~".clone_into(&mut tag.version_tag);
        assert!(init_problem(&tag, Some(&s)).is_some_and(|p| p.contains("version tag")));
        let mut missing = config;
        "trunk".clone_into(&mut missing.master);
        assert!(init_problem(&missing, Some(&s)).is_some_and(|p| p.contains("does not exist")));
    }

    #[test]
    fn a_start_is_refused_for_what_the_core_refuses() {
        let config = FlowConfig::suggested(true, false);
        let mut s = snap(&["main", "develop", "feature/login", "release/1.0"]);
        let check =
            |s: &RepoSnapshot, kind, name, base| start_problem(s, &config, kind, name, base);
        assert_eq!(check(&s, FlowKind::Feature, "new", "develop"), None);
        assert!(
            check(&s, FlowKind::Feature, "login", "develop")
                .is_some_and(|p| p.contains("already exists"))
        );
        assert!(
            check(&s, FlowKind::Feature, "a b", "develop")
                .is_some_and(|p| p.contains("not a valid"))
        );
        assert!(
            check(&s, FlowKind::Feature, "new", "nope")
                .is_some_and(|p| p.contains("does not exist"))
        );
        assert!(
            check(&s, FlowKind::Release, "2.0", "develop")
                .is_some_and(|p| p.contains("Finish 'release/1.0' first"))
        );
        s.branches.retain(|b| b.name != "release/1.0");
        assert_eq!(check(&s, FlowKind::Release, "2.0", "develop"), None);
        s.tags.push(canopy_core::TagInfo {
            name: "2.0".into(),
            target: None,
            message: None,
        });
        assert!(
            check(&s, FlowKind::Release, "2.0", "develop")
                .is_some_and(|p| p.contains("Tag '2.0' already exists"))
        );
        s.status.staged.push(FileEntry {
            path: "a".into(),
            old_path: None,
            kind: canopy_core::ChangeKind::Modified,
            additions: 0,
            deletions: 0,
            lfs: false,
            old_mode: None,
            new_mode: None,
        });
        assert!(
            check(&s, FlowKind::Hotfix, "1.0.1", "main")
                .is_some_and(|p| p.contains("working tree has changes"))
        );
        // A feature does not mind the changes.
        assert_eq!(check(&s, FlowKind::Feature, "new", "develop"), None);
    }

    #[test]
    fn a_hotfix_summary_names_the_open_release() {
        let config = FlowConfig::suggested(true, false);
        let s = snap(&["main", "develop", "hotfix/1.0.1", "release/1.1"]);
        let form = FlowFinishForm::new(Some("hotfix/1.0.1".into()), Some(&s));
        let lines = finish_summary(&s, &config, FlowKind::Hotfix, "1.0.1", &form);
        assert!(
            lines.iter().any(|l| l.contains("open release release/1.1")),
            "{lines:?}"
        );
        let s = snap(&["main", "develop", "hotfix/1.0.1"]);
        let lines = finish_summary(&s, &config, FlowKind::Hotfix, "1.0.1", &form);
        assert!(
            lines.iter().any(|l| l.contains("back into develop")),
            "{lines:?}"
        );
    }

    #[test]
    fn the_finish_form_picks_the_current_flow_branch_then_the_first() {
        let mut s = snap(&["main", "develop", "feature/a", "feature/b"]);
        s.workflow.flow = Some(FlowConfig::suggested(true, false));
        assert_eq!(FlowFinishForm::new(None, Some(&s)).branch, "feature/a");
        s.head.branch = Some("feature/b".into());
        assert_eq!(FlowFinishForm::new(None, Some(&s)).branch, "feature/b");
        assert_eq!(
            FlowFinishForm::new(Some("feature/a".into()), Some(&s)).branch,
            "feature/a"
        );
    }

    /// With a Git older than 2.38 the finish dialog says so and its button stays disabled, with
    /// the core's sentence as the reason; with a newer one the button works.
    #[test]
    fn finish_is_disabled_with_the_cores_sentence_on_an_old_git() {
        use crate::ui::kit::testkit::spoken;
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            std::path::PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let mut s = snap(&["main", "develop", "feature/a"]);
        s.workflow.flow = Some(FlowConfig::suggested(true, false));
        s.workflow.finish_unavailable = canopy_core::flow_finish_requirement(Some((2, 37)));
        tab.snap = Some(std::sync::Arc::new(s.clone()));
        let confirm = |tab: &RepoTab| {
            let mut form = FlowFinishForm::new(Some("feature/a".into()), tab.snap.as_deref());
            let salt = salt(&Dialog::FlowFinish(Box::new(form.clone())));
            let mut nodes = Vec::new();
            for _ in 0..3 {
                nodes = spoken(&ctx, |_| {
                    show_finish(&ctx, salt, &mut form, Some(tab));
                });
            }
            nodes
                .into_iter()
                .find(|n| {
                    n.label
                        .as_deref()
                        .is_some_and(|l| l.starts_with("Finish feature"))
                })
                .expect("the confirming button")
        };
        let old = confirm(&tab);
        assert!(old.disabled);
        assert_eq!(
            old.description.as_deref(),
            Some("Finishing a git-flow branch needs Git 2.38 or later.")
        );
        s.workflow.finish_unavailable = None;
        tab.snap = Some(std::sync::Arc::new(s));
        assert!(!confirm(&tab).disabled);
    }
}
