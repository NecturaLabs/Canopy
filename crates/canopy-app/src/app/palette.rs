//! The command palette's entries.

use super::*;
use crate::ui::widgets::forge_icon;

impl CanopyApp {
    /// A digest of what [`Self::palette_entries`] depends on that can change while the palette
    /// is open (it is rebuilt on every opening anyway).
    pub(super) fn palette_inputs(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.active, self.tabs.len()).hash(&mut h);
        for t in &self.tabs {
            (&t.name, t.generation).hash(&mut h);
        }
        self.settings.recent.hash(&mut h);
        h.finish()
    }

    pub(super) fn palette_entries(&self) -> Vec<(Entry, Action)> {
        let e = |icon: &'static str, label: &str, detail: &str, cmd: Option<Command>| Entry {
            icon,
            label: label.to_owned(),
            detail: detail.to_owned(),
            shortcut: cmd.map(keymap::hint).unwrap_or_default(),
        };
        use Command as C;
        let mut v = vec![
            (
                e(
                    icon::FOLDER_OPEN,
                    "Open repository…",
                    "Repository",
                    Some(C::Open),
                ),
                Action::OpenDialog(Dialog::open()),
            ),
            (
                e(
                    icon::CLOUD_ARROW_DOWN,
                    "Clone repository…",
                    "Repository",
                    Some(C::Clone),
                ),
                Action::OpenDialog(Dialog::clone_default(&self.settings)),
            ),
            (
                e(
                    icon::PLANT,
                    "Init new repository…",
                    "Repository",
                    Some(C::Init),
                ),
                Action::OpenDialog(Dialog::Init {
                    path: String::new(),
                }),
            ),
            (
                e(icon::PALETTE, "Next theme", "Appearance", None),
                Action::NextTheme,
            ),
            (
                e(icon::GEAR_SIX, "Settings", "Canopy", Some(C::Settings)),
                Action::Settings,
            ),
            (
                e(
                    icon::KEYBOARD,
                    "Keyboard shortcuts",
                    "Help",
                    Some(C::Cheatsheet),
                ),
                Action::Cheatsheet,
            ),
            (
                e(icon::ARROW_CIRCLE_UP, "Check for updates", "Canopy", None),
                Action::CheckUpdates,
            ),
            (
                e(icon::SPARKLE, "What's new in Canopy", "Help", None),
                Action::WhatsNew,
            ),
            (
                e(icon::INFO, "About Canopy", "Help", None),
                Action::OpenDialog(Dialog::About),
            ),
            (
                e(icon::FILE_TEXT, "Open log folder", "Help", None),
                Action::OpenLogs,
            ),
            (
                e(icon::SIGN_OUT, "Quit Canopy", "Canopy", Some(C::Quit)),
                Action::Quit,
            ),
            (
                e(
                    icon::MAGNIFYING_GLASS_PLUS,
                    "Zoom in",
                    "Appearance",
                    Some(C::ZoomIn),
                ),
                Action::Command(C::ZoomIn),
            ),
            (
                e(
                    icon::MAGNIFYING_GLASS_MINUS,
                    "Zoom out",
                    "Appearance",
                    Some(C::ZoomOut),
                ),
                Action::Command(C::ZoomOut),
            ),
        ];
        for t in ThemeId::ALL {
            v.push((
                e(
                    icon::PALETTE,
                    &format!("Theme: {}", t.name()),
                    "Appearance",
                    None,
                ),
                Action::SetTheme(t),
            ));
        }
        let app_entries = v;
        let mut v = Vec::new();
        if let Some(tab) = self.tabs.get(self.active) {
            let local_only = tab.is_local_only();
            match tab.view {
                View::Changes => v.push((
                    e(
                        icon::PENCIL_SIMPLE_LINE,
                        "Write commit message",
                        "Changes",
                        Some(C::FocusCommit),
                    ),
                    Action::Command(C::FocusCommit),
                )),
                View::History => v.push((
                    e(
                        icon::FUNNEL_SIMPLE,
                        "Filter history",
                        "History",
                        Some(C::Filter),
                    ),
                    Action::Command(C::Filter),
                )),
                View::Requests => {}
            }
            v.extend([
                (
                    e(icon::GRAPH, "Show history", "View", Some(C::History)),
                    Action::SetView(View::History),
                ),
                (
                    e(
                        icon::PENCIL_SIMPLE_LINE,
                        "Show changes",
                        "View",
                        Some(C::Changes),
                    ),
                    Action::SetView(View::Changes),
                ),
                (
                    e(
                        icon::ARROWS_CLOCKWISE,
                        "Refresh",
                        "Repository",
                        Some(C::Refresh),
                    ),
                    Action::Refresh,
                ),
                (
                    e(
                        icon::GIT_BRANCH,
                        "New branch…",
                        "Branch",
                        Some(C::NewBranch),
                    ),
                    Action::OpenDialog(Dialog::new_branch_at(Start::Head)),
                ),
                (
                    e(icon::GIT_MERGE, "Merge…", "Branch", Some(C::MergeDialog)),
                    Action::OpenDialog(Dialog::Merge {
                        target: String::new(),
                        rebase: false,
                        mode: canopy_core::MergeMode::Default,
                    }),
                ),
                (
                    e(icon::GIT_PULL_REQUEST, "Rebase…", "Branch", None),
                    Action::OpenDialog(Dialog::Merge {
                        target: String::new(),
                        rebase: true,
                        mode: canopy_core::MergeMode::Default,
                    }),
                ),
                (
                    e(icon::ARCHIVE, "Stash changes…", "Stash", Some(C::Stash)),
                    Action::OpenDialog(Dialog::Stash {
                        message: String::new(),
                        untracked: true,
                    }),
                ),
                (
                    e(
                        icon::TRAY_ARROW_UP,
                        "Pop latest stash",
                        "Stash",
                        Some(C::PopStash),
                    ),
                    Action::Op(Op::StashApply {
                        index: 0,
                        pop: true,
                    }),
                ),
                (
                    e(
                        icon::PLUS,
                        "Stage all changes",
                        "Changes",
                        Some(C::StageAll),
                    ),
                    Action::Op(Op::StageAll),
                ),
                (
                    e(
                        icon::MINUS,
                        "Unstage all changes",
                        "Changes",
                        Some(C::UnstageAll),
                    ),
                    Action::Op(Op::UnstageAll),
                ),
                (
                    e(icon::CLOUD, "Add remote…", "Remote", None),
                    Action::OpenDialog(Dialog::add_remote(&self.settings)),
                ),
                (
                    e(
                        icon::TERMINAL_WINDOW,
                        "Open terminal here",
                        "Repository",
                        None,
                    ),
                    Action::OpenTerminal(tab.path.clone()),
                ),
                (
                    e(
                        icon::FOLDER_OPEN,
                        "Show in file manager",
                        "Repository",
                        None,
                    ),
                    Action::OpenPath(tab.path.clone()),
                ),
                (
                    e(icon::X, "Close repository", "Repository", Some(C::CloseTab)),
                    Action::CloseTab(self.active),
                ),
            ]);
            if local_only {
                v.push((
                    e(icon::CLOUD_ARROW_UP, "Publish repository…", "Remote", None),
                    Action::Publish,
                ));
            } else {
                v.extend([
                    (e(icon::ARROWS_CLOCKWISE, "Fetch all remotes", "Remote", Some(C::Fetch)), Action::Fetch),
                    (e(icon::ARROW_FAT_LINE_DOWN, "Pull", "Remote", Some(C::Pull)), Action::Pull),
                    (e(icon::ARROW_FAT_LINE_UP, "Push", "Remote", Some(C::Push)), Action::Push),
                    (e(icon::WARNING, "Force push (with lease)…", "Remote", None), Action::Confirm { title: "Force push".into(), body: "Overwrite the remote branch with your local one? --force-with-lease refuses if someone else pushed in the meantime.".into(), confirm: "Force push".into(), danger: true, op: Op::Push { force: true } }),
                ]);
            }
            if let Some((_, forge)) = tab.forge() {
                let kind = forge.kind;
                v.push((
                    e(
                        forge_icon(kind),
                        &format!("Open repository on {}", kind.name()),
                        "Remote",
                        None,
                    ),
                    Action::OpenUrl(forge.web_url()),
                ));
                v.push((
                    e(
                        icon::GIT_PULL_REQUEST,
                        &format!("Show {}s", kind.request_noun()),
                        "View",
                        Some(C::Requests),
                    ),
                    Action::SetView(View::Requests),
                ));
                if let Some(b) = tab.snap.as_ref().and_then(|s| s.current_branch())
                    && b.upstream.is_some()
                {
                    v.push((
                        e(
                            icon::GIT_PULL_REQUEST,
                            &format!("Create {} for {}…", kind.request_noun(), b.name),
                            "Remote",
                            None,
                        ),
                        Action::CreateRequest {
                            source: b.name.clone(),
                        },
                    ));
                }
            }
            if let Some(s) = &tab.snap {
                if let Some(op) = s.state.command().map(|_| s.state) {
                    v.push((
                        e(
                            icon::PLAY,
                            &format!("Continue {}", op.label().to_lowercase()),
                            "Repository",
                            None,
                        ),
                        Action::Op(Op::ContinueOrAbort {
                            state: op,
                            abort: false,
                        }),
                    ));
                    v.push((
                        e(
                            icon::X_CIRCLE,
                            &format!("Abort {}", op.label().to_lowercase()),
                            "Repository",
                            None,
                        ),
                        Action::Op(Op::ContinueOrAbort {
                            state: op,
                            abort: true,
                        }),
                    ));
                }
                for b in s.branches.iter().filter(|b| !b.is_head) {
                    v.push((
                        e(
                            icon::GIT_BRANCH,
                            &format!("Check out {}", b.name),
                            "Branch",
                            None,
                        ),
                        Action::Op(Op::Checkout(b.name.clone())),
                    ));
                }
                for b in &s.remote_branches {
                    v.push((
                        e(
                            icon::CLOUD,
                            &format!("Check out {}", b.full_name()),
                            "Remote branch",
                            None,
                        ),
                        Action::Op(Op::CheckoutRemote {
                            remote: b.remote.clone(),
                            branch: b.branch.clone(),
                        }),
                    ));
                }
                for b in s.branches.iter().filter(|b| !b.is_head) {
                    v.push((
                        e(
                            icon::GIT_MERGE,
                            &format!("Merge {} into current", b.name),
                            "Branch",
                            None,
                        ),
                        Action::Op(Op::Merge(b.name.clone(), canopy_core::MergeMode::Default)),
                    ));
                }
            }
            if let Some(Ok(list)) = &tab.requests.list
                && let Some((_, forge)) = tab.forge()
            {
                for r in list.iter() {
                    v.push((
                        e(
                            icon::GIT_PULL_REQUEST,
                            &format!("#{} {}", r.number, r.title),
                            forge.kind.request_noun(),
                            None,
                        ),
                        Action::Command(C::Requests),
                    ));
                }
            }
        }
        for (i, t) in self.tabs.iter().enumerate() {
            v.push((
                e(
                    icon::FOLDER_SIMPLE,
                    &format!("Switch to {}", t.name),
                    "Tab",
                    None,
                ),
                Action::SwitchTab(i),
            ));
        }
        for r in &self.settings.recent {
            let name = r
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            v.push((
                e(
                    icon::CLOCK_COUNTER_CLOCKWISE,
                    &format!("Open {name}"),
                    &r.display().to_string(),
                    None,
                ),
                Action::OpenRepo(r.clone()),
            ));
        }
        v.extend(app_entries);
        v
    }
}
