//! Keyboard handling: bare keys, chords and the commands they run.

use super::*;

impl CanopyApp {
    /// Runs the keyboard table for this frame.
    pub(super) fn keyboard(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let modal =
            self.dialogs.current().is_some() || self.settings_screen.is_some() || self.palette.open;
        let overlay = modal || self.cheatsheet.open;
        // Settings on top of everything: its search takes the find shortcut.
        let settings_on_top = self.settings_screen.is_some()
            && self.dialogs.current().is_none()
            && !self.palette.open
            && !self.cheatsheet.open;
        let search_open = self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.view == View::History && tab.history_filter_open);
        let typing = ctx.egui_wants_keyboard_input();
        // A button or checkbox reached with Tab keeps Space, Enter and the arrows for itself.
        let widget_focused = !typing && ctx.memory(|m| m.focused().is_some());
        let fired = keymap::dispatch(ctx, |b, chord| {
            if b.command == Command::Dismiss {
                return overlay || search_open;
            }
            if modal {
                return settings_on_top
                    && b.command == Command::Filter
                    && (!b.section.is_bare() || !typing);
            }
            if b.command == Command::CloseTab {
                // Ctrl+W while typing a commit message must not throw the draft away.
                return !typing;
            }
            if !b.section.is_bare() {
                return true;
            }
            !(typing || (widget_focused && keymap::widget_key(chord.key)))
        });
        for f in fired {
            self.command(ctx, f.command, f.chord, actions);
        }
    }

    /// Turns a command into actions. `chord` is the matched chord's index (for tab jumps).
    pub(super) fn command(
        &mut self,
        ctx: &egui::Context,
        command: Command,
        chord: usize,
        actions: &mut Vec<Action>,
    ) {
        use Command as C;
        let has_tab = !self.tabs.is_empty();
        let view = self.tabs.get(self.active).map(|t| t.view);
        match command {
            C::Palette => actions.push(Action::Palette),
            C::Cheatsheet => self.cheatsheet.toggle(),
            C::Dismiss => {
                if self.cheatsheet.open {
                    self.cheatsheet.open = false;
                } else if self.palette.open {
                    self.palette.open = false;
                } else if self.dialogs.current().is_some() {
                    // The consent stays (it must be answered); a credential prompt is declined.
                    self.dialogs.dismiss();
                } else if let Some(screen) = self.settings_screen.as_mut() {
                    // Settings covers the history search: a search in Settings is cleared
                    // first, and the next Escape closes Settings.
                    if !screen.dismiss_search() {
                        self.settings_screen = None;
                    }
                } else if let Some(tab) = self.tab()
                    && tab.view == View::History
                    && tab.history_filter_open
                {
                    tab.close_history_filter(ctx);
                }
            }
            C::Open => actions.push(Action::OpenDialog(Dialog::open())),
            C::Clone => actions.push(Action::OpenDialog(Dialog::clone_default(&self.settings))),
            C::Init => actions.push(Action::OpenDialog(Dialog::new_repository(String::new()))),
            C::Settings => actions.push(Action::Settings),
            C::ZoomIn | C::ZoomOut | C::ZoomReset => {
                self.settings.zoom = match command {
                    C::ZoomIn => (self.settings.zoom + 0.1).min(1.6),
                    C::ZoomOut => (self.settings.zoom - 0.1).max(0.8),
                    _ => 1.0,
                };
                apply_zoom(ctx, self.settings.zoom);
            }
            C::Quit => actions.push(Action::Quit),
            _ if !has_tab => {}
            C::CloseTab => actions.push(Action::CloseTab(self.active)),
            C::NextTab => actions.push(Action::SwitchTab((self.active + 1) % self.tabs.len())),
            C::PrevTab => actions.push(Action::SwitchTab(
                (self.active + self.tabs.len() - 1) % self.tabs.len(),
            )),
            C::JumpTab => {
                if chord < self.tabs.len() {
                    actions.push(Action::SwitchTab(chord));
                }
            }
            C::Refresh => actions.push(Action::Refresh),
            C::Fetch => actions.push(Action::Fetch),
            C::Pull => actions.push(Action::Pull),
            C::Push => actions.push(Action::Push),
            C::NewBranch => actions.push(Action::OpenDialog(Dialog::new_branch_at(Start::Head))),
            C::Stash => actions.push(Action::OpenDialog(Dialog::Stash {
                message: String::new(),
                untracked: true,
            })),
            C::PopStash => actions.push(Action::Op(Op::StashApply {
                index: 0,
                pop: true,
            })),
            C::MergeDialog => actions.push(Action::OpenDialog(Dialog::Merge {
                target: String::new(),
                rebase: false,
                mode: canopy_core::MergeMode::Default,
            })),
            C::Commit => {
                if view == Some(View::Changes) {
                    actions.push(Action::Commit);
                }
            }
            C::Filter if self.settings_screen.is_some() => {
                if let Some(screen) = self.settings_screen.as_mut() {
                    screen.focus_search();
                }
            }
            C::Filter => {
                actions.push(Action::SetView(View::History));
                if let Some(tab) = self.tab() {
                    tab.filter_history(ctx);
                }
            }
            C::History => actions.push(Action::SetView(View::History)),
            C::Changes => actions.push(Action::SetView(View::Changes)),
            C::Requests => actions.push(Action::SetView(View::Requests)),
            C::ApproveRequest | C::MergeRequest => {
                if view == Some(View::Requests)
                    && self.settings.hosting_api
                    && let Some(tab) = self.tabs.get(self.active)
                    && let Some((_, forge)) = tab.forge()
                    && let Some(request) = requests::selected_summary(tab)
                {
                    if command == C::ApproveRequest {
                        if let Some(action) = requests::approve_action(tab, request) {
                            actions.push(action);
                        }
                    } else {
                        let details = tab
                            .requests
                            .details
                            .as_ref()
                            .and_then(|details| details.as_ref().ok())
                            .map(std::sync::Arc::as_ref);
                        if let Some(dialog) =
                            requests::merge_confirmation(tab, forge.kind, request, details)
                        {
                            actions.push(Action::OpenDialog(dialog));
                        }
                    }
                }
            }
            C::Down | C::Up | C::PageDown | C::PageUp | C::First | C::Last => {
                let delta = match command {
                    C::Down => 1,
                    C::Up => -1,
                    C::PageDown => PAGE_ROWS,
                    C::PageUp => -PAGE_ROWS,
                    C::First => isize::MIN,
                    _ => isize::MAX,
                };
                if let Some(t) = self.tab() {
                    match t.view {
                        View::History => t.move_commit(delta),
                        View::Changes => t.move_file(delta),
                        View::Requests => t.move_request(delta),
                    }
                }
            }
            C::Tag => {
                if let Some(oid) = self.tabs.get(self.active).and_then(|t| t.selected_commit) {
                    actions.push(Action::OpenDialog(Dialog::NewTag {
                        name: String::new(),
                        target: oid,
                        message: String::new(),
                    }));
                }
            }
            C::CopySha => {
                if let Some(oid) = self.tabs.get(self.active).and_then(|t| t.selected_commit) {
                    actions.push(Action::Copy(oid.to_string()));
                }
            }
            C::OpenOnForge => {
                if let Some(t) = self.tabs.get(self.active)
                    && let Some((_, forge)) = t.forge()
                {
                    let url = match (t.view, t.selected_commit, &t.requests.list) {
                        (View::Requests, _, Some(Ok(list))) => t
                            .requests
                            .selected
                            .and_then(|n| list.iter().find(|r| r.number == n))
                            .map_or_else(|| forge.web_url(), |r| r.web_url.clone()),
                        (View::History, Some(oid), _) => forge.commit_url(&oid.to_string()),
                        _ => forge.web_url(),
                    };
                    actions.push(Action::OpenUrl(url));
                }
            }
            C::FocusCommit => {
                if let Some(t) = self.tab() {
                    t.view = View::Changes;
                    t.focus_summary = true;
                }
            }
            C::ToggleStage | C::Discard => {
                let Some(t) = self.tabs.get(self.active) else {
                    return;
                };
                let Some(sel) = t.file.clone().filter(|_| t.view == View::Changes) else {
                    return;
                };
                let path = sel.entry.path.clone();
                if command == C::Discard {
                    if sel.source == canopy_core::DiffSource::Unstaged && !sel.conflicted {
                        actions.push(Action::Confirm {
                            title: "Discard changes".into(),
                            body: format!(
                                "Throw away the unstaged changes to {path}? This cannot be undone."
                            ),
                            confirm: "Discard".into(),
                            danger: true,
                            op: Op::Discard(vec![sel.entry]),
                        });
                    }
                } else if sel.source == canopy_core::DiffSource::Staged {
                    actions.push(Action::Op(Op::Unstage(vec![path])));
                } else {
                    actions.push(Action::Op(Op::Stage(vec![path])));
                }
            }
            C::NextConflict
            | C::PrevConflict
            | C::TakeOurs
            | C::TakeTheirs
            | C::TakeBoth
            | C::TakeBothTheirsFirst
            | C::TakeBase
            | C::EditConflict
            | C::ClearChoice => {
                if let Some(r) = self
                    .tabs
                    .get_mut(self.active)
                    .filter(|t| t.view == View::Changes)
                    .and_then(|t| t.resolver.as_mut())
                {
                    match command {
                        C::NextConflict => r.move_focus(1),
                        C::PrevConflict => r.move_focus(-1),
                        C::TakeOurs => r.choose(canopy_core::Choice::Ours),
                        C::TakeTheirs => r.choose(canopy_core::Choice::Theirs),
                        C::TakeBoth => r.choose(canopy_core::Choice::OursThenTheirs),
                        C::TakeBothTheirsFirst => r.choose(canopy_core::Choice::TheirsThenOurs),
                        C::TakeBase => r.choose(canopy_core::Choice::Base),
                        C::EditConflict => r.edit_focused(),
                        _ => r.clear_focused(),
                    }
                }
            }
            C::StageAll => {
                if view == Some(View::Changes) {
                    actions.push(Action::Op(Op::StageAll));
                }
            }
            C::UnstageAll => {
                if view == Some(View::Changes) {
                    actions.push(Action::Op(Op::UnstageAll));
                }
            }
        }
    }
}
