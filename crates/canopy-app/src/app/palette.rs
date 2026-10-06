//! The command palette's entries.

use super::*;
use crate::action::AppearanceChoice;
use crate::ui::motion::MotionPref;
use crate::ui::theme::{Density, Style};
use crate::ui::widgets::forge_icon;

impl CanopyApp {
    /// Sets an Appearance option the way Settings → Appearance does: the setting changes and the
    /// look follows on this frame (and is saved with the settings). Choosing "Match system" asks
    /// the system again, as its answer may have changed.
    pub(super) fn set_appearance(&mut self, ctx: &egui::Context, choice: AppearanceChoice) {
        let s = &mut self.settings;
        match choice {
            AppearanceChoice::Style(v) => s.style = v,
            AppearanceChoice::Density(v) => s.density = v,
            AppearanceChoice::Translucent(on) => s.translucent = on,
            AppearanceChoice::Motion(v) => {
                s.motion = v;
                if v == MotionPref::System {
                    crate::ui::motion::query_system(ctx);
                }
            }
        }
        self.apply_look(ctx);
    }

    /// The palette's Appearance entries: Style, Density, Translucent surfaces and Motion (spec
    /// 6.2). Turning translucency on is offered only where it can work.
    fn appearance_entries(&self) -> Vec<(Entry, Action)> {
        let e = |icon: &'static str, label: String, choice: AppearanceChoice| {
            (
                Entry {
                    icon,
                    label,
                    detail: "Appearance".to_owned(),
                    shortcut: String::new(),
                },
                Action::SetAppearance(choice),
            )
        };
        let mut v = Vec::new();
        for style in Style::ALL {
            v.push(e(
                icon::STACK,
                format!("Style: {}", style.label()),
                AppearanceChoice::Style(style),
            ));
        }
        for density in Density::ALL {
            v.push(e(
                icon::ROWS,
                format!("Density: {}", density.label()),
                AppearanceChoice::Density(density),
            ));
        }
        let blocked = theme::glass_blocked(
            self.settings.theme,
            crate::ui::kit::glass::available(),
            crate::ui::kit::glass::reduced_transparency(),
        );
        for on in [true, false] {
            if on && blocked.is_some() {
                continue;
            }
            v.push(e(
                icon::DROP_HALF,
                format!("Translucent surfaces: {}", if on { "On" } else { "Off" }),
                AppearanceChoice::Translucent(on),
            ));
        }
        for motion in MotionPref::ALL {
            v.push(e(
                icon::WAVE_SINE,
                format!("Motion: {}", motion.label()),
                AppearanceChoice::Motion(motion),
            ));
        }
        v
    }
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
        use Command as C;

        let e = |icon: &'static str, label: &str, detail: &str, cmd: Option<Command>| Entry {
            icon,
            label: label.to_owned(),
            detail: detail.to_owned(),
            shortcut: cmd.map(keymap::hint).unwrap_or_default(),
        };
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
                Action::OpenDialog(Dialog::new_repository(String::new())),
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
        v.extend(self.appearance_entries());
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
                View::Requests => {
                    v.push((
                        e(
                            icon::CHECK,
                            "Approve selected request",
                            "Requests",
                            Some(C::ApproveRequest),
                        ),
                        Action::Command(C::ApproveRequest),
                    ));
                    v.push((
                        e(
                            icon::GIT_MERGE,
                            "Merge selected request…",
                            "Requests",
                            Some(C::MergeRequest),
                        ),
                        Action::Command(C::MergeRequest),
                    ));
                }
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
                    // Resolved when it runs, by the latest stash's commit.
                    Action::Command(C::PopStash),
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
                    Action::AddRemoteDialog,
                ),
                (
                    e(
                        icon::SLIDERS_HORIZONTAL,
                        "Repository settings…",
                        "Repository",
                        None,
                    ),
                    Action::RepositorySettings(tab.path.clone()),
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
                    (e(icon::WARNING, "Force push (with lease)…", "Remote", None), Action::Confirm { title: "Force push the current branch?".into(), body: "The remote branch is overwritten with your local one. --force-with-lease refuses if someone else pushed in the meantime.".into(), confirm: "Force push branch".into(), danger: true, op: Op::Push { force: true } }),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(entries: &[(Entry, Action)]) -> Vec<&str> {
        entries.iter().map(|(e, _)| e.label.as_str()).collect()
    }

    /// With no repository open the palette offers only what works without one.
    #[test]
    fn the_palette_offers_repository_commands_only_with_a_tab() {
        let mut app = test_app();
        let alone = app.palette_entries();
        assert!(labels(&alone).contains(&"Open repository…"));
        assert!(
            !labels(&alone).contains(&"Pop latest stash"),
            "{:?}",
            labels(&alone)
        );
        let before = app.palette_inputs();
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        assert_ne!(
            app.palette_inputs(),
            before,
            "a new tab rebuilds the entries"
        );
        let with_tab = app.palette_entries();
        let pop = with_tab
            .iter()
            .find(|(e, _)| e.label == "Pop latest stash")
            .map(|(_, a)| a);
        assert!(
            matches!(pop, Some(Action::Command(Command::PopStash))),
            "resolved by the latest stash's commit when chosen: {pop:?}"
        );
    }

    /// Every Style and Density choice and the Motion choices are offered.
    #[test]
    fn every_appearance_choice_is_in_the_palette() {
        let app = test_app();
        let entries = app.appearance_entries();
        let all = labels(&entries);
        for style in Style::ALL {
            let label = format!("Style: {}", style.label());
            assert!(all.contains(&label.as_str()), "{all:?}");
        }
        for density in Density::ALL {
            let label = format!("Density: {}", density.label());
            assert!(all.contains(&label.as_str()), "{all:?}");
        }
        assert!(all.iter().any(|l| l.starts_with("Motion: ")), "{all:?}");
    }
}
