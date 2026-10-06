//! Headless screenshots for `--screenshot` (tools/screenshot.sh).

use std::sync::Arc;

use egui::accesskit::Role;

use super::*;

#[derive(Debug)]
pub(super) struct Screenshot {
    pub(super) path: PathBuf,
    pub(super) ready_frames: u32,
    pub(super) ready_since: Option<f64>,
    pub(super) requested: bool,
    pub(super) started: f64,
    /// A popup kept open for the picture: popups close when a frame does not show them.
    pub(super) popup: Option<egui::Id>,
    /// The context menu `--overlay menu-*` opens for the picture.
    pub(super) menu: Option<MenuShot>,
}

impl Screenshot {
    /// A picture saved to `path`, of `overlay` when that is one this module opens itself.
    pub(super) fn new(path: PathBuf, overlay: Option<&str>) -> Self {
        Self {
            path,
            ready_frames: 0,
            ready_since: None,
            requested: false,
            started: 0.0,
            popup: (overlay == Some("diff-options"))
                .then(|| egui::Id::new(crate::views::diff::OPTIONS_POPUP)),
            menu: overlay.and_then(MenuShot::of),
        }
    }
}

/// `--overlay menu-*`: a context menu opened for the picture by a right click on the top-most
/// (then left-most) widget whose AccessKit role and name match, once the window has settled.
/// Found by what a screen reader hears, so the picture does not depend on any view's ids.
#[derive(Debug)]
pub(super) struct MenuShot {
    role: Option<Role>,
    name: &'static str,
    /// The view the widget is in.
    pub(super) view: View,
    /// Where the click goes once the widget is found, and how many of its two frames (press,
    /// then release) were sent.
    click: Option<(egui::Pos2, u8)>,
}

impl MenuShot {
    fn of(overlay: &str) -> Option<Self> {
        let (role, name, view) = match overlay {
            "menu-remotes" => (None, "Remotes", View::History),
            "menu-branch" => (Some(Role::ListItem), "feature/auth", View::History),
            "menu-commit" => (None, "Commit ", View::History),
            "menu-file" => (Some(Role::ListItem), "src/main.rs", View::Changes),
            _ => return None,
        };
        Some(Self {
            role,
            name,
            view,
            click: None,
        })
    }

    /// Whether the menu is open: finds its widget in this frame's AccessKit tree and aims the
    /// click at it, then waits for the click's two frames.
    fn opened(&mut self, ctx: &egui::Context) -> bool {
        if let Some((_, sent)) = self.click {
            return sent >= 2;
        }
        let found = ctx.viewport(|v| {
            let nodes = &v.this_pass.accesskit_state.as_ref()?.nodes;
            nodes
                .iter()
                .filter(|(_, n)| {
                    self.role.is_none_or(|r| n.role() == r)
                        && n.label().is_some_and(|l| l.starts_with(self.name))
                })
                .filter_map(|(id, _)| v.this_pass.widgets.get(*id).map(|w| w.rect))
                .filter(egui::Rect::is_positive)
                .min_by(|a, b| {
                    (a.top(), a.left())
                        .partial_cmp(&(b.top(), b.left()))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        if let Some(rect) = found {
            self.click = Some((rect.center(), 0));
        }
        false
    }
}

impl CanopyApp {
    /// Makes the active tab show the `--demo-requests` scenario (see `demo_requests`), and the
    /// settings the requests view needs to draw it. Whether the picture may be taken yet.
    fn show_demo_requests(
        &mut self,
        ctx: &egui::Context,
        scenario: demo_requests::Scenario,
    ) -> bool {
        let now = chrono::Utc::now().timestamp();
        let shown = match self.tabs.get_mut(self.active) {
            Some(tab) => scenario.show(ctx, tab, now),
            // Nothing is open (the welcome screen): there is nothing to wait for.
            None => return true,
        };
        // The view draws only when pull and merge requests are on. That is switched on only once
        // the tab holds the demo's API thread, so the next frame has no reason to start a real
        // one; the pictures it would fetch from the host stay off.
        self.settings.hosting_api = shown;
        self.settings.download_avatars = false;
        shown
    }

    pub(super) fn handle_screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_none() {
            return;
        }
        // `--demo-requests`: the active tab shows invented requests instead of loading them.
        let demo = demo_requests::chosen();
        let demo_shown = demo.is_none_or(|scenario| self.show_demo_requests(ctx, scenario));
        let Some(shot) = &mut self.screenshot else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if shot.started == 0.0 {
            shot.started = now;
        }
        let ready = demo_shown
            && self.pending_opens == 0
            && !crate::ui::art::growing(ctx)
            && self.tabs.iter().all(|t| {
                // A demo scene may be a loading or an error state on purpose: it is final.
                let requests_ready = demo.is_some()
                    || t.view != View::Requests
                    || t.host.is_none()
                    || (!t.requests.loading
                        && !t.requests.details_loading
                        && t.requests.list.is_some());
                t.snap.is_some()
                    && t.running.is_none()
                    && (t.diff.is_some() || t.file.is_none())
                    && requests_ready
                    && !t.avatars.pending()
            });
        // A menu opens only once everything under it is final, and the picture waits for it.
        let ready = ready && shot.menu.as_mut().is_none_or(|menu| menu.opened(ctx));
        if ready {
            if let Some(id) = shot.popup {
                egui::Popup::open_id(ctx, id);
            }
            shot.ready_frames += 1;
            shot.ready_since.get_or_insert(now);
        } else {
            shot.ready_frames = 0;
            shot.ready_since = None;
        }
        let settled =
            shot.ready_frames > 30 && shot.ready_since.is_some_and(|since| now - since > 0.3);
        if !shot.requested && (settled || now - shot.started > 12.0) {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(image) = image {
            match save_png(&shot.path, &image) {
                Ok(()) => tracing::info!(path = %shot.path.display(), "screenshot saved"),
                Err(e) => tracing::error!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// `--overlay` names for the dialogs no other overlay opens (also listed in `OVERLAYS`);
/// [`dialog_overlay`] says what each shows.
#[cfg(test)]
const DIALOG_OVERLAYS: &[&str] = &[
    "open",
    "open-error",
    "clone",
    "clone-running",
    "clone-error",
    "init",
    "new-branch",
    "rename",
    "new-tag",
    "stash",
    "add-remote",
    "add-remote-refused",
    "publish",
    "new-request",
    "confirm",
    "confirm-danger",
    "confirm-host",
    "confirm-merge-risky",
    "whats-new",
    "crash",
    "update-available",
    "update-failed",
    "terms",
    "consent-reading",
    "crash-reading",
    "username",
    "password",
    "approve",
    "notice",
];

impl CanopyApp {
    /// Opens the dialog `--overlay name` names (the names of `DIALOG_OVERLAYS`), with the state
    /// it shows.
    pub(super) fn open_dialog_overlay(&mut self, name: &str) {
        if name == "update-failed" {
            self.updater.install = updater::InstallState::Failed {
                build: 1001,
                error: "The download's checksum does not match the release (expected 9f2c…e1, got 41aa…07).".into(),
            };
        }
        if let Some(d) = dialog_overlay(name, &self.settings) {
            self.dialogs.open(d);
        }
    }
}

/// The dialog `--overlay name` opens for a screenshot, filled with fixed illustrative content so
/// the picture never shows a fixture's temporary path; `None` for a name that is not a dialog's.
fn dialog_overlay(name: &str, settings: &Settings) -> Option<Dialog> {
    use crate::askpass::{Prompt, PromptKind};
    use crate::ui::legal::{Document, Pending};
    use canopy_core::forge::ForgeKind;

    let commit = canopy_core::Oid::from_str("38b14e7c2d9a4f61b0e5a7c3d2f1e0b9a8c7d6e5").ok()?;
    let available = |notes: &str| updater::Available {
        build: 1001,
        manifest: canopy_hosting::updates::UpdateManifest {
            version: "1001".into(),
            notes: notes.to_owned(),
            pub_date: Some("2026-10-04T09:30:00Z".into()),
            release_page: None,
            platforms: std::collections::BTreeMap::default(),
        },
    };
    let notes = "### Added\n\n- Settings search finds any option by name, description or synonym.\n- Repository settings: clone protocol, pull strategy and identity per repository.\n\n### Fixed\n\n- Author pictures load again for public GitHub repositories.";
    Some(match name {
        "open" | "open-error" => Dialog::Open {
            path: if name == "open" {
                "~/src/".into()
            } else {
                "~/Downloads".into()
            },
            recent: ["tidepool", "canopy", "lighthouse-api", "dotfiles"]
                .into_iter()
                .map(|n| PathBuf::from("/home/ada/src").join(n))
                .collect(),
            pending: None,
            error: (name == "open-error").then(|| dialogs::OpenFailure {
                message: "/home/ada/Downloads is not a Git repository".into(),
                not_a_repository: true,
            }),
        },
        "clone" | "clone-running" | "clone-error" => {
            let mut d = Dialog::clone_default(settings);
            if let Dialog::Clone {
                builder,
                parent,
                pending,
                error,
            } = &mut d
            {
                builder.project = "rust-lang/rustlings".into();
                *parent = "~/src".into();
                *pending = name == "clone-running";
                if name == "clone-error" {
                    *error = Some("fatal: destination path 'rustlings' already exists and is not an empty directory.".into());
                }
            }
            d
        }
        "init" => Dialog::new_repository("~/src/tidepool".into()),
        "new-branch" => Dialog::new_branch_at(Start::Head),
        "rename" => Dialog::Rename {
            old: "feature/search".into(),
            new: "feature/search-v2".into(),
        },
        "new-tag" => Dialog::NewTag {
            name: "v1.4.0".into(),
            target: commit,
            message: String::new(),
        },
        "stash" => Dialog::Stash {
            message: String::new(),
            untracked: true,
        },
        "add-remote" | "add-remote-refused" | "publish" => {
            let mut d = Dialog::add_remote(settings);
            if name == "publish" {
                d = d.publishing();
            } else {
                // As the app names it beside the fixture's own origin.
                d.name_remote(&["origin".to_owned()]);
            }
            if name == "add-remote-refused"
                && let Dialog::AddRemote { builder, .. } = &mut d
            {
                // A URL with a password in it, which Canopy refuses to save.
                builder.source = dialogs::UrlSource::Url;
                builder.url = "https://ada:hunter2@git.example.com/acme/tidepool.git".into();
            }
            d
        }
        "new-request" => Dialog::NewRequest {
            kind: ForgeKind::GitHub,
            request: NewRequest {
                title: "Add settings search".into(),
                body: String::new(),
                source: "feature/settings-search".into(),
                target: "main".into(),
                draft: false,
            },
            branches: vec!["main".into(), "develop".into(), "release/2026.10".into()],
        },
        "confirm" => Dialog::Confirm {
            title: "Check out 38b14e7?".into(),
            body: "HEAD will point at this commit, not at a branch (a detached HEAD). Commits you make there belong to no branch until you create one.".into(),
            confirm: "Check out commit".into(),
            danger: false,
            op: Op::Checkout("38b14e7".into()),
        },
        "confirm-danger" => Dialog::Confirm {
            title: "Delete branch feature/dark-mode?".into(),
            body: "feature/dark-mode has 1 commit that is on no other branch. You can restore it from the reflog for 90 days.".into(),
            confirm: "Delete branch".into(),
            danger: true,
            op: Op::DeleteBranch {
                name: "feature/dark-mode".into(),
                force: false,
            },
        },
        "confirm-host" => Dialog::ConfirmHost {
            title: "Approve pull request #428?".into(),
            body: "GitHub records your approval of Add settings search under your account.".into(),
            confirm: "Approve".into(),
            danger: false,
            job: HostJob::Approve(428),
        },
        "confirm-merge-risky" => Dialog::ConfirmHost {
            title: "Merge pull request #428 anyway?".into(),
            body: "Checks failed on Add settings search and a reviewer requested changes. Merging now puts it on main as it is.".into(),
            confirm: "Merge anyway".into(),
            danger: true,
            job: HostJob::Merge(428, canopy_hosting::MergeMethod::Merge, None),
        },
        "whats-new" => Dialog::WhatsNew {
            version: "build 1001".into(),
            notes: notes.into(),
        },
        "crash" => Dialog::Crash(crate::logging::CrashReport {
            path: PathBuf::from("/home/ada/.local/share/canopy/crashes/2026-10-05T18-22-41Z.txt"),
            message: "called `Option::unwrap()` on a `None` value at crates/canopy-app/src/views/history.rs:412:31".into(),
        }),
        "update-available" | "update-failed" => Dialog::Update(Box::new(available(notes))),
        "terms" => Dialog::Legal(Document::Terms),
        "consent-reading" => Dialog::Consent {
            step: Pending::Terms,
            updated: false,
            reading: Some(Document::Terms),
        },
        "crash-reading" => Dialog::Consent {
            step: Pending::CrashReports,
            updated: false,
            reading: Some(Document::Privacy),
        },
        "username" => Dialog::credential(Prompt::preview("Username for 'https://gitlab.example.com': ")),
        "password" => Dialog::credential(Prompt::preview("Password for 'https://ada@gitlab.example.com': ")),
        // ssh asks these through SSH_ASKPASS_PROMPT, which a preview's text cannot say.
        "approve" | "notice" => {
            let mut prompt = Prompt::preview(if name == "approve" {
                "Allow use of key /home/ada/.ssh/id_ed25519?\nKey fingerprint SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU."
            } else {
                "Confirm user presence for key ED25519-SK SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"
            });
            prompt.kind = if name == "approve" {
                PromptKind::Approve
            } else {
                PromptKind::Notice
            };
            Dialog::credential(prompt)
        }
        _ => return None,
    })
}

impl CanopyApp {
    /// A screenshot's input: no pointer (see [`without_pointer`]) but the right click that opens
    /// an `--overlay menu-*` menu.
    pub(super) fn screenshot_input(&mut self, input: &mut egui::RawInput) {
        let Some(shot) = &mut self.screenshot else {
            return;
        };
        without_pointer(input);
        if let Some((at, sent)) = shot.menu.as_mut().and_then(|m| m.click.as_mut())
            && *sent < 2
        {
            input.events.retain(|e| *e != egui::Event::PointerGone);
            input.events.push(egui::Event::PointerMoved(*at));
            input.events.push(egui::Event::PointerButton {
                pos: *at,
                button: egui::PointerButton::Secondary,
                pressed: *sent == 0,
                modifiers: egui::Modifiers::NONE,
            });
            *sent += 1;
        }
    }
}

/// Takes the pointer out of a screenshot's input, so the picture never depends on where the
/// virtual screen left it: Xvfb starts the pointer in the middle of the screen, where it would
/// hover (and give a tooltip to) whatever lies there.
fn without_pointer(input: &mut egui::RawInput) {
    input.events.retain(|event| {
        !matches!(
            event,
            egui::Event::PointerMoved(_)
                | egui::Event::MouseMoved(_)
                | egui::Event::PointerButton { .. }
                | egui::Event::MouseWheel { .. }
                | egui::Event::Touch { .. }
                | egui::Event::PointerGone
        )
    });
    input.events.push(egui::Event::PointerGone);
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        u32::try_from(image.size[0])?,
        u32::try_from(image.size[1])?,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_srgba_unmultiplied)
        .collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CR-5: a screenshot never sees the pointer. Xvfb starts it in the middle of the screen,
    /// where it hovered whatever lay there and put a tooltip into some pictures and not others.
    #[test]
    fn a_screenshot_ignores_the_pointer() {
        use eframe::App as _;
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.screenshot = Some(Screenshot::new(PathBuf::from("shot.png"), None));
        let at = egui::pos2(480.0, 300.0);
        let mut input = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Text("kept".into()),
            ],
            ..Default::default()
        };
        app.raw_input_hook(&ctx, &mut input);
        assert_eq!(
            input.events,
            [egui::Event::Text("kept".into()), egui::Event::PointerGone]
        );
        // A normal run keeps its pointer.
        app.screenshot = None;
        let mut input = egui::RawInput {
            events: vec![egui::Event::PointerMoved(at)],
            ..Default::default()
        };
        app.raw_input_hook(&ctx, &mut input);
        assert_eq!(input.events, [egui::Event::PointerMoved(at)]);
    }

    /// CR-6: `--overlay menu-*` opens its context menu by a right click on the widget it names,
    /// once the window has settled, and the picture waits for the menu.
    #[test]
    fn a_menu_overlay_opens_its_context_menu() {
        use eframe::App as _;
        // (The other menus need a repository's commits, branches or a loaded diff: their
        // pictures are checked with tools/screenshot.sh.)
        let (overlay, item) = ("menu-remotes", "Add remote…");
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.enable_accesskit();
        let mut app = test_app();
        app.settings.accepted_terms = crate::ui::legal::TERMS_VERSION.to_owned();
        app.settings.accepted_privacy = crate::ui::legal::PRIVACY_VERSION.to_owned();
        app.settings.crash_reports = Some(false);
        app.settings.crash_reports_version = crate::crash_reports::DATA_VERSION;
        app.settings.seen_intro = true;
        // A remote, so "Add remote…" is only in the header's menu (not a local-only link).
        let mut tab = test_tab(canopy_core::RepoSnapshot {
            remotes: vec![canopy_core::RemoteInfo {
                name: "origin".into(),
                url: Some("https://example.com/r.git".into()),
                default_branch: None,
            }],
            ..Default::default()
        });
        app.screenshot = Some(Screenshot::new(PathBuf::from("shot.png"), Some(overlay)));
        if let Some(menu) = app.screenshot.as_ref().and_then(|s| s.menu.as_ref()) {
            tab.view = menu.view;
        }
        app.tabs.push(tab);
        let mut frame = eframe::Frame::_new_kittest();
        let mut shown = false;
        for i in 0..90 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(i) * 0.05),
                ..Default::default()
            };
            app.raw_input_hook(&ctx, &mut input);
            let mut out = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
            out.textures_delta.clear();
            shown = out.shapes.iter().any(|c| {
                fn has(shape: &egui::Shape, text: &str) -> bool {
                    match shape {
                        egui::Shape::Text(t) => t.galley.text() == text,
                        egui::Shape::Vec(v) => v.iter().any(|s| has(s, text)),
                        _ => false,
                    }
                }
                has(&c.shape, item)
            });
            if shown {
                break;
            }
        }
        assert!(shown, "--overlay {overlay} never showed {item:?}");
    }

    /// secfix2 F7: `--overlay add-remote-refused` shows Add remote with a URL Canopy refuses
    /// (credentials in it), so the refusal can be checked in a picture.
    #[test]
    fn the_refused_add_remote_overlay_shows_the_refusal() {
        let dialog = dialog_overlay("add-remote-refused", &Settings::default());
        let Some(Dialog::AddRemote { builder, .. }) = dialog else {
            panic!("--overlay add-remote-refused opens {dialog:?}");
        };
        assert!(builder.resolve().is_none());
        assert!(
            builder
                .problem()
                .is_some_and(|p| p.contains("password") || p.contains("credential")),
            "{:?}",
            builder.problem()
        );
        assert!(OVERLAYS.contains(&"add-remote-refused"));
    }

    #[test]
    fn every_dialog_overlay_opens_a_dialog_that_draws() {
        let settings = Settings::default();
        let updater = Updater::default();
        let mut picker = crate::ui::folder_picker::FolderPicker::default();
        for name in DIALOG_OVERLAYS {
            let mut dialog = dialog_overlay(name, &settings)
                .unwrap_or_else(|| panic!("--overlay {name} opens no dialog"));
            let ctx = egui::Context::default();
            crate::ui::fonts::install(&ctx);
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
                    dialogs::show(
                        &ctx,
                        &mut dialog,
                        None,
                        &mut picker,
                        false,
                        &updater,
                        "git version 2.55.0",
                    );
                });
                output.textures_delta.clear();
                assert!(!output.shapes.is_empty(), "--overlay {name} drew nothing");
            }
        }
        assert!(dialog_overlay("settings", &settings).is_none());
    }
}
