//! Keyboard handling: bare keys, chords and the commands they run.

use super::*;

impl CanopyApp {
    /// Runs the keyboard table for this frame.
    pub(super) fn keyboard(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let modal =
            self.dialogs.current().is_some() || self.settings_screen.is_some() || self.palette.open;
        let overlay = modal || self.cheatsheet.open;
        let menu_open = egui::Popup::is_any_open(ctx);
        // Settings on top of everything: its search takes the find shortcut.
        let settings_on_top = self.settings_screen.is_some()
            && self.dialogs.current().is_none()
            && !self.palette.open
            && !self.cheatsheet.open;
        let search_open = self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.view == View::History && tab.history_filter_open);
        // Only a text field, or an open list's type-ahead, takes letters; any other focused widget
        // (a button the pointer or Tab left focus on, after Esc closed its menu) keeps just Space,
        // Enter and the arrows.
        let typing = ctx.text_edit_focused() || crate::ui::kit::select::takes_text(ctx);
        let widget_focused = !typing && ctx.memory(|m| m.focused().is_some());
        // Settings → Keyboard may be waiting for a row's new key: it takes the press first.
        keymap::capture_key(ctx);
        let fired = keymap::dispatch(ctx, |b, chord| {
            if b.command == Command::ContextMenu {
                // Listed for the cheatsheet; the focused row consumes the key itself.
                return false;
            }
            if b.command == Command::Dismiss {
                // An open menu is the topmost layer: it takes this Escape and closes itself.
                return (overlay || search_open) && !menu_open;
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
            // Opened by the focused row (`keyboard` never fires it).
            C::ContextMenu => {}
            C::Cheatsheet => self.cheatsheet.toggle(),
            C::Dismiss => {
                if self.cheatsheet.open {
                    self.cheatsheet.open = false;
                } else if self.palette.open {
                    self.palette.open = false;
                } else if self.dialogs.current().is_some() {
                    // The consent stays (it must be answered); a credential prompt is declined.
                    self.dialogs.dismiss_with_focus(ctx);
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
            // Settings' own search, with or without a repository open.
            C::Filter if self.settings_screen.is_some() => {
                if let Some(screen) = self.settings_screen.as_mut() {
                    screen.focus_search();
                }
            }
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
            C::PopStash => {
                if let Some(op) = self.tabs.get(self.active).and_then(toolbar::pop_latest) {
                    actions.push(Action::Op(op));
                }
            }
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
                            title: format!("Discard changes to {path}?"),
                            body: "Its unstaged changes are thrown away. This cannot be undone."
                                .into(),
                            confirm: "Discard changes".into(),
                            danger: true,
                            op: Op::Discard(vec![sel.entry]),
                        });
                    }
                } else if sel.source == canopy_core::DiffSource::Staged {
                    actions.push(Action::Op(Op::Unstage(vec![path])));
                } else {
                    // Every file picked with Ctrl-click or Shift-click, else the open one.
                    let mut paths = t.stage_targets();
                    if paths.is_empty() {
                        paths.push(path);
                    }
                    actions.push(Action::Op(Op::Stage(paths)));
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
                    .filter(|t| {
                        t.view == View::Changes
                            && t.file.as_ref().zip(t.resolver.as_ref()).is_some_and(
                                |(file, resolver)| {
                                    file.conflicted && file.entry.path == resolver.path
                                },
                            )
                    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(app: &mut CanopyApp, command: Command) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        app.command(&ctx, command, 0, &mut actions);
        actions
    }

    #[test]
    fn conflict_shortcuts_leave_a_hidden_resolution_draft_untouched() {
        use crate::tab::{FileSelection, Resolver};
        use crate::worker::Worker;
        use canopy_core::{ChangeKind, Choice, ConflictFile, DiffSource, FileEntry};
        let mut app = test_app();
        let (worker, _, _) = Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            settings::DiffPrefs::default(),
            worker,
        );
        tab.view = View::Changes;
        let mut resolver = Resolver::new("conflict.txt".into());
        resolver.reload(
            None,
            Ok(std::sync::Arc::new(ConflictFile::parse(
                "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n",
            ))),
        );
        resolver.choices[0] = Some(Choice::Custom("saved choice".into()));
        resolver.editing = Some((0, "unfinished text".into()));
        tab.resolver = Some(resolver);
        tab.file = Some(FileSelection {
            source: DiffSource::Unstaged,
            entry: FileEntry {
                path: "ordinary.txt".into(),
                old_path: None,
                kind: ChangeKind::Modified,
                additions: 0,
                deletions: 0,
                lfs: false,
                old_mode: None,
                new_mode: None,
            },
            conflicted: false,
        });
        app.tabs.push(tab);
        run(&mut app, Command::ClearChoice);
        let resolver = app.tabs[0].resolver.as_ref().expect("hidden draft");
        assert_eq!(
            resolver.choices,
            vec![Some(Choice::Custom("saved choice".into()))]
        );
        assert_eq!(resolver.editing, Some((0, "unfinished text".into())));

        let file = app.tabs[0].file.as_mut().expect("selected file");
        file.conflicted = true;
        file.entry.path = "conflict.txt".into();
        run(&mut app, Command::ClearChoice);
        let resolver = app.tabs[0].resolver.as_ref().expect("visible resolver");
        assert_eq!(resolver.choices, vec![None]);
        assert_eq!(resolver.editing, None);
    }

    #[test]
    fn the_stage_key_stages_every_selected_unstaged_file() {
        use crate::tab::FileSelection;
        use crate::worker::Worker;
        use canopy_core::{ChangeKind, DiffSource, FileEntry};
        let entry = |path: &str| FileEntry {
            path: path.into(),
            old_path: None,
            kind: ChangeKind::Modified,
            additions: 0,
            deletions: 0,
            lfs: false,
            old_mode: None,
            new_mode: None,
        };
        let mut app = test_app();
        let (worker, _, _) = Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            settings::DiffPrefs::default(),
            worker,
        );
        tab.view = View::Changes;
        let mut snap = canopy_core::RepoSnapshot::default();
        snap.status.unstaged = vec![entry("a.rs"), entry("b.rs"), entry("c.rs")];
        tab.snap = Some(std::sync::Arc::new(snap));
        tab.file = Some(FileSelection {
            source: DiffSource::Unstaged,
            entry: entry("a.rs"),
            conflicted: false,
        });
        app.tabs.push(tab);
        let staged = |app: &mut CanopyApp| match run(app, Command::ToggleStage).as_slice() {
            [Action::Op(Op::Stage(paths))] => paths.clone(),
            other => panic!("one Stage expected, got {other:?}"),
        };
        assert_eq!(staged(&mut app), ["a.rs"]);
        app.tabs[0].marked.insert("c.rs".into());
        assert_eq!(staged(&mut app), ["a.rs", "c.rs"]);
    }

    /// Shift+F10 is in the keymap table for the cheatsheet, but the focused row opens its menu
    /// itself: the keyboard table must leave the key press for the widget.
    #[test]
    fn shift_f10_is_left_for_the_focused_row() {
        let mut app = test_app();
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::F10,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
            }],
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut left = false;
        let mut output = ctx.run_ui(raw, |ui| {
            app.keyboard(ui.ctx(), &mut actions);
            left = ui.ctx().input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::F10,
                            ..
                        }
                    )
                })
            });
        });
        output.textures_delta.clear();
        assert!(left, "the table consumed Shift+F10 before the row saw it");
        assert!(actions.is_empty(), "{actions:?}");
    }

    /// review-views-1: the find shortcut reaches Settings' search on the welcome screen too.
    #[test]
    fn settings_search_takes_the_find_shortcut_with_no_repository_open() {
        let mut app = test_app();
        app.settings_screen = Some(SettingsScreen::at(settings_screen::Section::Appearance));
        let actions = run(&mut app, Command::Filter);
        assert!(actions.is_empty(), "{actions:?}");
        assert!(
            app.settings_screen
                .as_ref()
                .is_some_and(SettingsScreen::search_focus_pending),
            "the search field takes the caret"
        );
    }

    #[test]
    fn app_escape_returns_focus_to_the_dialog_trigger_each_time() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut app = test_app();
        let mut trigger = egui::Id::NULL;
        let mut draw = |events| {
            frame(&ctx, events, |ui| {
                app.keyboard(ui.ctx(), &mut Vec::new());
                let button = crate::ui::kit::Button::new("Stash").show(ui);
                trigger = button.id;
                if button.clicked() {
                    app.dialogs.open(Dialog::Stash {
                        message: String::new(),
                        untracked: true,
                    });
                }
                if let Some(dialog) = app.dialogs.current() {
                    let _ = crate::ui::kit::overlay::Dialog::new(dialogs::salt(dialog))
                        .title("Stash changes")
                        .initial_focus(crate::ui::kit::overlay::InitialFocus::Body)
                        .body(|ui| {
                            ui.text_edit_singleline(&mut String::new());
                        })
                        .footer(|f| {
                            f.cancel().confirm("Stash");
                        })
                        .show(ui.ctx());
                }
            });
            (trigger, app.dialogs.current().is_some())
        };
        draw(Vec::new());
        let (trigger, _) = draw(crate::ui::kit::testkit::key(egui::Key::Tab));
        for _ in 0..2 {
            assert!(
                draw(crate::ui::kit::testkit::key(egui::Key::Enter)).1,
                "trigger opens the dialog"
            );
            draw(Vec::new());
            assert!(
                !draw(crate::ui::kit::testkit::key(egui::Key::Escape)).1,
                "app handler closes it"
            );
            draw(Vec::new());
            draw(Vec::new());
            assert_eq!(ctx.memory(egui::Memory::focused), Some(trigger));
        }
    }

    /// Escape closes the topmost thing first: the cheatsheet, the palette, a dialog, then a
    /// search in Settings, then Settings itself.
    #[test]
    fn escape_closes_the_topmost_overlay_first() {
        let mut app = test_app();
        app.settings_screen = Some(SettingsScreen::searching("theme"));
        app.dialogs.open(Dialog::open());
        app.palette.open = true;
        app.cheatsheet.open = true;
        run(&mut app, Command::Dismiss);
        assert!(!app.cheatsheet.open && app.palette.open);
        run(&mut app, Command::Dismiss);
        assert!(!app.palette.open && app.dialogs.current().is_some());
        run(&mut app, Command::Dismiss);
        assert!(app.dialogs.current().is_none() && app.settings_screen.is_some());
        run(&mut app, Command::Dismiss);
        assert!(
            app.settings_screen.is_some(),
            "the first Escape in Settings clears its search"
        );
        run(&mut app, Command::Dismiss);
        assert!(app.settings_screen.is_none());
    }

    /// usersim2 N1: with a menu open over a dialog (a link's context menu in the legal reader),
    /// the first Escape closes the menu only; the next one dismisses the dialog.
    #[test]
    fn escape_closes_an_open_menu_before_the_dialog_behind_it() {
        let mut app = test_app();
        app.dialogs.open(Dialog::open());
        let ctx = egui::Context::default();
        let esc = || crate::ui::kit::testkit::key(egui::Key::Escape);
        // As the app draws: the keyboard table first, then the interface with the menu.
        let run = |app: &mut CanopyApp, events: Vec<egui::Event>, open: bool| {
            let mut actions = Vec::new();
            frame(&ctx, events, |ui| {
                app.keyboard(ui.ctx(), &mut actions);
                let button = ui.button("Row");
                let _ = egui::Popup::menu(&button)
                    .open_memory(open.then_some(egui::SetOpenCommand::Bool(true)))
                    .show(|ui| ui.label("Copy link"));
            });
        };
        run(&mut app, Vec::new(), true);
        run(&mut app, Vec::new(), false);
        assert!(egui::Popup::is_any_open(&ctx), "the menu is open");
        run(&mut app, esc(), false);
        assert!(
            !egui::Popup::is_any_open(&ctx),
            "the first Escape closes the menu"
        );
        assert!(app.dialogs.current().is_some(), "and leaves the dialog");
        run(&mut app, esc(), false);
        assert!(
            app.dialogs.current().is_none(),
            "the next Escape dismisses the dialog"
        );
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, f: impl FnMut(&mut egui::Ui)) {
        let mut f = f;
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
    }

    /// Runs `keyboard` for one frame in which `key` is pressed while the widget `focused` built by
    /// `build` has keyboard focus.
    fn keyboard_with_focus(
        app: &mut CanopyApp,
        key: egui::Key,
        build: impl Fn(&mut egui::Ui) -> egui::Id,
    ) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut focused = None;
        // The first frame builds the widget and gives it focus; the key arrives in the second.
        frame(&ctx, Vec::new(), |ui| {
            let id = build(ui);
            ui.memory_mut(|m| m.request_focus(id));
            focused = Some(id);
        });
        frame(&ctx, Vec::new(), |ui| {
            build(ui);
        });
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            focused,
            "the widget has focus"
        );
        let mut actions = Vec::new();
        let press = egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, vec![press], |ui| {
            build(ui);
            app.keyboard(ui.ctx(), &mut actions);
        });
        actions
    }

    /// usersim F2: a button left focused (the bell after Esc closes the center) keeps Space, Enter
    /// and the arrows for itself but must not swallow the bare repository keys.
    #[test]
    fn a_focused_button_leaves_the_bare_repository_keys_working() {
        let button = |ui: &mut egui::Ui| ui.button("Bell").id;
        let mut app = test_app();
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        let actions = keyboard_with_focus(&mut app, egui::Key::F, button);
        assert!(matches!(actions.as_slice(), [Action::Fetch]), "{actions:?}");
        let actions = keyboard_with_focus(&mut app, egui::Key::P, button);
        assert!(matches!(actions.as_slice(), [Action::Pull]), "{actions:?}");
        // The arrows and Space stay with the button.
        for key in [egui::Key::ArrowDown, egui::Key::Space, egui::Key::Enter] {
            let actions = keyboard_with_focus(&mut app, key, button);
            assert!(actions.is_empty(), "{key:?} fired {actions:?}");
        }
    }

    /// Letters typed into an open list jump to an option (type-ahead): they are not the bare
    /// repository keys (`s` stashes, `m` merges), and the text event stays for the list.
    #[test]
    fn letters_typed_into_an_open_select_are_not_repository_keys() {
        use crate::ui::kit::testkit::{frame, key};
        let ctx = crate::ui::kit::testkit::ctx();
        let options = [(0_u8, "Merge"), (1, "Squash"), (2, "Rebase")];
        let mut value = 0_u8;
        let mut trigger = egui::Id::NULL;
        let mut show = |ui: &mut egui::Ui| {
            trigger = crate::ui::kit::select::Select::new(&mut value, &options)
                .id_salt("method")
                .show(ui)
                .id;
            trigger
        };
        let mut app = test_app();
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        frame(&ctx, Vec::new(), |ui| {
            let id = show(ui);
            ui.memory_mut(|m| m.request_focus(id));
        });
        frame(&ctx, Vec::new(), |ui| {
            show(ui);
        });
        frame(&ctx, key(egui::Key::ArrowDown), |ui| {
            show(ui);
        });
        frame(&ctx, Vec::new(), |ui| {
            show(ui);
        });
        assert!(egui::Popup::is_any_open(&ctx), "the list is open");
        let mut events = key(egui::Key::S);
        events.push(egui::Event::Text("s".into()));
        let mut actions = Vec::new();
        let mut text_left = false;
        frame(&ctx, events, |ui| {
            app.keyboard(ui.ctx(), &mut actions);
            text_left = ui.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Text(t) if t == "s"))
            });
            show(ui);
        });
        assert!(actions.is_empty(), "{actions:?}");
        assert!(text_left, "the list still gets the letter");
        // Closed again, the same key is the repository's.
        frame(&ctx, key(egui::Key::Escape), |ui| {
            show(ui);
        });
        frame(&ctx, Vec::new(), |ui| {
            show(ui);
        });
        assert!(!egui::Popup::is_any_open(&ctx), "Escape closed the list");
        let mut actions = Vec::new();
        frame(&ctx, key(egui::Key::S), |ui| {
            app.keyboard(ui.ctx(), &mut actions);
            show(ui);
        });
        assert!(!actions.is_empty(), "a closed list leaves the key alone");
    }

    /// While a text field has focus a bare letter is text, not a command.
    #[test]
    fn a_focused_text_field_keeps_the_bare_keys() {
        let field = |ui: &mut egui::Ui| ui.text_edit_singleline(&mut String::new()).id;
        let mut app = test_app();
        app.tabs
            .push(test_tab(canopy_core::RepoSnapshot::default()));
        let actions = keyboard_with_focus(&mut app, egui::Key::F, field);
        assert!(actions.is_empty(), "{actions:?}");
    }

    /// secfix2 F5: the Pop shortcut pops the stash the snapshot shows as latest, by its commit.
    #[test]
    fn the_pop_shortcut_pops_the_latest_stash_by_its_commit() {
        let oid = canopy_core::Oid::from_str("0123456789abcdef0123456789abcdef01234567")
            .expect("an object id");
        let mut app = test_app();
        app.tabs.push(test_tab(canopy_core::RepoSnapshot {
            stashes: vec![canopy_core::StashInfo {
                index: 0,
                message: "On main: wip".into(),
                oid,
            }],
            ..Default::default()
        }));
        let actions = run(&mut app, Command::PopStash);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Op(Op::StashApplyCommit { oid: o, pop: true })] if *o == oid
            ),
            "{actions:?}"
        );
        // With no stash known, git's own `stash pop` says why nothing happened.
        app.tabs[0] = test_tab(canopy_core::RepoSnapshot::default());
        let actions = run(&mut app, Command::PopStash);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Op(Op::StashApply {
                    index: 0,
                    pop: true
                })]
            ),
            "{actions:?}"
        );
    }
}
