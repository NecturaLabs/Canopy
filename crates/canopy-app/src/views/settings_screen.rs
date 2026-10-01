//! The settings screen (Ctrl+,): sections on the left, live-applied controls on the right.
//! Every setting changes behaviour; nothing here is decorative.

use std::collections::HashMap;

use crossbeam_channel::Receiver;
use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::forge::{Forge, ForgeKind};

use crate::app::{Action, forge_icon};
use crate::hosting::{self, TokenStore};
use crate::keymap::{BINDINGS, Section as KeySection};
use crate::settings::{CloneProtocol, PullMode, Settings, WhitespaceMode};
use crate::ui::theme::{self, Palette, ThemeId, palette};
use crate::ui::widgets;
use crate::updater::{self, UpdateState, Updater};

/// The sections, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Appearance,
    Git,
    Integrations,
    Accounts,
    Updates,
    Privacy,
    Keyboard,
}

impl Section {
    const ALL: [Section; 7] = [
        Section::Appearance,
        Section::Git,
        Section::Integrations,
        Section::Accounts,
        Section::Updates,
        Section::Privacy,
        Section::Keyboard,
    ];

    fn label(self) -> (&'static str, &'static str) {
        match self {
            Section::Appearance => (icon::PALETTE, "Appearance"),
            Section::Git => (icon::GIT_BRANCH, "Git"),
            Section::Integrations => (icon::PLUGS_CONNECTED, "Integrations"),
            Section::Accounts => (icon::KEY, "Accounts"),
            Section::Updates => (icon::ARROW_CIRCLE_UP, "Updates"),
            Section::Privacy => (icon::SHIELD_CHECK, "Privacy"),
            Section::Keyboard => (icon::KEYBOARD, "Keyboard"),
        }
    }
}

#[derive(Debug)]
enum TokenTest {
    Running(Receiver<Result<String, String>>),
    Done(Result<String, String>),
}

/// State of the open settings screen.
#[derive(Default)]
pub struct SettingsScreen {
    section: Section,
    token_input: HashMap<String, String>,
    tests: HashMap<String, TokenTest>,
    token_error: Option<String>,
}

impl std::fmt::Debug for SettingsScreen {
    // The token fields hold secrets being typed: show only which hosts have one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsScreen")
            .field("section", &self.section)
            .field("token_input", &self.token_input.keys().collect::<Vec<_>>())
            .field("tests", &self.tests)
            .field("token_error", &self.token_error)
            .finish()
    }
}

impl SettingsScreen {
    /// The screen opened at `section`.
    pub fn at(section: Section) -> Self {
        Self {
            section,
            ..Self::default()
        }
    }
}

/// What the app must do after this frame.
#[derive(Debug, Default)]
pub struct SettingsResult {
    pub close: bool,
    pub tokens_changed: bool,
}

/// Draws the screen. `forges` are the GitHub/GitLab remotes of open repositories, so their hosts
/// show up under Accounts.
pub fn show(
    ctx: &egui::Context,
    screen: &mut SettingsScreen,
    settings: &mut Settings,
    tokens: &mut TokenStore,
    forges: &[Forge],
    update: &UpdateState,
    actions: &mut Vec<Action>,
) -> SettingsResult {
    let p = palette(ctx);
    let mut result = SettingsResult::default();
    let screen_rect = ctx.content_rect();
    let width = (screen_rect.width() - 80.0).clamp(560.0, 900.0);
    let height = (screen_rect.height() - 120.0).clamp(380.0, 640.0);
    let modal = egui::Modal::new(egui::Id::new("settings"))
        .frame(
            egui::Frame::new()
                .fill(p.elevated)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
                .shadow(ctx.global_style().visuals.window_shadow)
                .inner_margin(egui::Margin::same(0)),
        )
        .backdrop_color(p.backdrop())
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_height(height);
            ui.horizontal_top(|ui| {
                // Section list.
                egui::Frame::new()
                    .fill(p.panel)
                    .corner_radius(CornerRadius {
                        nw: theme::RADIUS_LG,
                        sw: theme::RADIUS_LG,
                        ne: 0,
                        se: 0,
                    })
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(170.0);
                            ui.set_height(height - 24.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon::GEAR_SIX).size(18.0).color(p.accent));
                                ui.label(
                                    RichText::new("Settings")
                                        .font(FontId::new(16.0, theme::semibold())),
                                );
                            });
                            ui.add_space(12.0);
                            for s in Section::ALL {
                                nav_item(ui, &mut screen.section, s);
                            }
                        });
                    });
                // Content.
                ui.vertical(|ui| {
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        let (glyph, title) = screen.section.label();
                        ui.label(RichText::new(glyph).size(18.0).color(p.accent));
                        ui.label(RichText::new(title).font(FontId::new(17.0, theme::semibold())));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(14.0);
                            if widgets::icon_button(ui, icon::X, "Close (Esc)").clicked() {
                                result.close = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(height - 60.0)
                        .show(ui, |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: 6,
                                    right: 18,
                                    top: 0,
                                    bottom: 14,
                                })
                                .show(ui, |ui| match screen.section {
                                    Section::Appearance => appearance(ui, settings),
                                    Section::Git => git(ui, settings),
                                    Section::Integrations => integrations(ui, settings),
                                    Section::Accounts => {
                                        result.tokens_changed |=
                                            accounts(ui, screen, settings, tokens, forges, actions)
                                    }
                                    Section::Updates => updates(ui, settings, update, actions),
                                    Section::Privacy => privacy(ui, settings, actions),
                                    Section::Keyboard => keyboard(ui),
                                });
                        });
                });
            });
        });
    result.close |= modal.should_close();
    result
}

fn nav_item(ui: &mut Ui, current: &mut Section, s: Section) {
    let p = palette(ui.ctx());
    let selected = *current == s;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if selected {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            Palette::tint(p.accent, if p.dark { 50 } else { 32 }),
        );
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS), p.hover);
    }
    let (glyph, label) = s.label();
    ui.painter().text(
        rect.left_center() + vec2(14.0, 0.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(15.0),
        if selected { p.accent } else { p.text_dim },
    );
    ui.painter().text(
        rect.left_center() + vec2(30.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::new(13.5, theme::medium()),
        if selected { p.text } else { p.text_dim },
    );
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    if resp.clicked() {
        *current = s;
    }
}

/// A labelled row: the title and help on the left, the control on the right.
fn row(ui: &mut Ui, title: &str, help: &str, control: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 260.0).max(200.0));
            ui.label(RichText::new(title).font(FontId::new(13.5, theme::medium())));
            if !help.is_empty() {
                ui.label(RichText::new(help).size(12.0).color(p.text_dim));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter()
        .hline(r.x_range(), r.min.y, Stroke::new(1.0, p.border));
    ui.add_space(10.0);
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    widgets::section_label(ui, text);
    ui.add_space(8.0);
}

fn appearance(ui: &mut Ui, s: &mut Settings) {
    heading(ui, "Theme");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        for t in ThemeId::ALL {
            if theme_card(ui, t, s.theme == t).clicked() {
                s.theme = t;
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Interface");
    row(
        ui,
        "Zoom",
        &format!(
            "Scales the whole interface. {} and {} also work.",
            crate::keymap::hint(crate::keymap::Command::ZoomIn),
            crate::keymap::hint(crate::keymap::Command::ZoomOut)
        ),
        |ui| {
            ui.add(
                egui::Slider::new(&mut s.zoom, 0.8..=1.6)
                    .step_by(0.05)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        },
    );
    row(
        ui,
        "Dates",
        "How the history shows when a commit was made. The exact time is in the tooltip.",
        |ui| {
            ui.selectable_value(&mut s.relative_dates, false, "Sep 21, 2026");
            ui.selectable_value(&mut s.relative_dates, true, "3 hours ago");
        },
    );
    row(
        ui,
        "Author avatars",
        "Coloured initials next to authors. Canopy never downloads pictures.",
        |ui| {
            toggle(ui, &mut s.show_avatars);
        },
    );
    row(
        ui,
        "Reduce motion",
        "Turns off fades and the welcome screen's drifting glow.",
        |ui| {
            toggle(ui, &mut s.reduce_motion);
        },
    );
}

/// A theme preview: a miniature of the window drawn from the theme's own palette.
fn theme_card(ui: &mut Ui, id: ThemeId, selected: bool) -> egui::Response {
    let p = palette(ui.ctx());
    let tp = id.palette();
    let (rect, resp) = ui.allocate_exact_size(vec2(150.0, 112.0), Sense::click());
    let hover = resp.hovered();
    let painter = ui.painter();
    let mock = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 82.0));
    painter.rect_filled(mock, CornerRadius::same(theme::RADIUS), tp.bg);
    // Sidebar.
    let side = egui::Rect::from_min_size(mock.min, vec2(38.0, mock.height()));
    painter.rect_filled(
        side,
        CornerRadius {
            nw: theme::RADIUS,
            sw: theme::RADIUS,
            ne: 0,
            se: 0,
        },
        tp.panel,
    );
    for (i, w) in [22.0, 16.0, 26.0].into_iter().enumerate() {
        let y = side.min.y + 12.0 + i as f32 * 10.0;
        painter.line_segment(
            [
                egui::pos2(side.min.x + 7.0, y),
                egui::pos2(side.min.x + 7.0 + w, y),
            ],
            Stroke::new(3.0, if i == 0 { tp.accent } else { tp.text_faint }),
        );
    }
    // Graph lanes and rows.
    for i in 0..5 {
        let y = mock.min.y + 12.0 + i as f32 * 13.0;
        let lane = tp.lane(i % 3);
        let x = mock.min.x + 50.0 + f32::from(i % 3) * 8.0;
        painter.circle_filled(egui::pos2(x, y), 3.0, lane);
        painter.line_segment(
            [
                egui::pos2(mock.min.x + 76.0, y),
                egui::pos2(mock.max.x - 12.0 - (i as f32 * 7.0) % 20.0, y),
            ],
            Stroke::new(2.5, if i == 1 { tp.text } else { tp.text_dim }),
        );
    }
    for l in 0..3u16 {
        let x = mock.min.x + 50.0 + f32::from(l) * 8.0;
        painter.line_segment(
            [
                egui::pos2(x, mock.min.y + 8.0),
                egui::pos2(x, mock.max.y - 8.0),
            ],
            Stroke::new(1.5, Palette::tint(tp.lane(l), 160)),
        );
    }
    // Primary button.
    let button = egui::Rect::from_min_size(
        egui::pos2(mock.max.x - 46.0, mock.max.y - 16.0),
        vec2(38.0, 10.0),
    );
    widgets::gradient_rect(painter, button, tp.button, tp.button2, 4);
    painter.rect_stroke(
        mock,
        CornerRadius::same(theme::RADIUS),
        Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                p.accent
            } else if hover {
                Palette::tint(p.accent, 140)
            } else {
                p.border
            },
        ),
        egui::StrokeKind::Outside,
    );
    let label = if selected {
        format!("{}  {}", icon::CHECK_CIRCLE, id.name())
    } else {
        id.name().to_owned()
    };
    painter.text(
        egui::pos2(rect.min.x + 2.0, mock.max.y + 16.0),
        Align2::LEFT_CENTER,
        label,
        FontId::new(13.0, theme::medium()),
        if selected { p.text } else { p.text_dim },
    );
    painter.text(
        egui::pos2(rect.max.x - 2.0, mock.max.y + 16.0),
        Align2::RIGHT_CENTER,
        if tp.dark { "dark" } else { "light" },
        FontId::proportional(11.0),
        p.text_faint,
    );
    let name = format!("{} theme", id.name());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, &name)
    });
    resp
}

fn toggle(ui: &mut Ui, on: &mut bool) {
    ui.selectable_value(on, true, "On");
    ui.selectable_value(on, false, "Off");
}

fn git(ui: &mut Ui, s: &mut Settings) {
    heading(ui, "Remote");
    row(
        ui,
        "Pull strategy",
        "Used when git's pull.rebase is not configured for the repository.",
        |ui| {
            for m in PullMode::ALL.into_iter().rev() {
                ui.selectable_value(&mut s.pull_mode, m, m.label());
            }
        },
    );
    row(
        ui,
        "Prune on fetch",
        "Remove remote-tracking branches that were deleted on the remote.",
        |ui| {
            toggle(ui, &mut s.prune_on_fetch);
        },
    );
    row(
        ui,
        "Fetch in the background",
        "Fetches every remote of open repositories while Canopy is idle.",
        |ui| {
            egui::ComboBox::from_id_salt("auto-fetch")
                .selected_text(match s.auto_fetch_minutes {
                    0 => "Off".to_owned(),
                    m => format!("Every {m} minutes"),
                })
                .show_ui(ui, |ui| {
                    for m in [0, 5, 10, 15, 30] {
                        let label = if m == 0 {
                            "Off".to_owned()
                        } else {
                            format!("Every {m} minutes")
                        };
                        ui.selectable_value(&mut s.auto_fetch_minutes, m, label);
                    }
                });
        },
    );
    heading(ui, "Commits");
    row(
        ui,
        "Add Signed-off-by",
        "Appends a Signed-off-by trailer (git commit --signoff), as DCO projects require.",
        |ui| {
            toggle(ui, &mut s.signoff);
        },
    );
    row(
        ui,
        "Commits per page",
        "How many commits the history loads before \"Load more\".",
        |ui| {
            ui.add(
                egui::DragValue::new(&mut s.commit_page)
                    .range(200..=50_000)
                    .speed(50),
            );
        },
    );
    heading(ui, "Diffs");
    row(
        ui,
        "Layout",
        "Unified shows old and new lines in one column; side by side puts the old file on the left and the new on the right.",
        |ui| {
            ui.selectable_value(&mut s.split_diff, true, "Side by side");
            ui.selectable_value(&mut s.split_diff, false, "Unified");
        },
    );
    row(
        ui,
        "Syntax highlighting",
        "Colours code in diffs by language (Rust, TypeScript, Python, Markdown and about 200 more).",
        |ui| {
            toggle(ui, &mut s.syntax_highlight);
        },
    );
    heading(ui, "Diff defaults (every repository)");
    row(
        ui,
        "Context lines",
        "Unchanged lines shown around each change. Each repository can override this from the diff header.",
        |ui| {
            ui.add(egui::Slider::new(&mut s.diff.context, 0..=25));
        },
    );
    row(
        ui,
        "Whitespace",
        "Ignoring whitespace turns off hunk staging, since such a diff cannot be applied.",
        |ui| {
            egui::ComboBox::from_id_salt("whitespace")
                .selected_text(s.diff.whitespace.label())
                .show_ui(ui, |ui| {
                    for m in WhitespaceMode::ALL {
                        ui.selectable_value(&mut s.diff.whitespace, m, m.label());
                    }
                });
        },
    );
    if !s.repo_diff.is_empty() {
        row(
            ui,
            "Repository overrides",
            &format!(
                "{} repositories use their own diff options.",
                s.repo_diff.len()
            ),
            |ui| {
                if ui.button("Reset all").clicked() {
                    s.repo_diff.clear();
                }
            },
        );
    }
}

fn integrations(ui: &mut Ui, s: &mut Settings) {
    heading(ui, "Cloning");
    row(
        ui,
        "Default clone folder",
        "Where the clone dialog puts new repositories. Empty uses ~/src when it exists, else your home folder.",
        |ui| {
            if ui.button(format!("{}  Browse…", icon::FOLDER)).clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                s.clone_dir = dir.display().to_string();
            }
            let hint = s.clone_parent();
            ui.add(
                egui::TextEdit::singleline(&mut s.clone_dir)
                    .hint_text(hint)
                    .desired_width(200.0),
            );
        },
    );
    row(
        ui,
        "Clone protocol",
        "For URLs built from owner/repository. SSH needs a key added to your account.",
        |ui| {
            ui.selectable_value(&mut s.clone_protocol, CloneProtocol::Ssh, "SSH");
            ui.selectable_value(&mut s.clone_protocol, CloneProtocol::Https, "HTTPS");
        },
    );
    heading(ui, "GitHub and GitLab");
    row(
        ui,
        "Self-hosted GitLab",
        "Hosts to treat as GitLab, comma-separated. Hosts whose name starts with \"gitlab\" are recognised anyway.",
        |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut s.gitlab_hosts)
                    .hint_text("git.example.com, code.corp.io")
                    .desired_width(240.0),
            );
        },
    );
    row(
        ui,
        "Pull and merge requests",
        "Load, review and act on pull/merge requests through the GitHub and GitLab APIs. Requests load when you open a repository hosted there. Off keeps Canopy offline apart from git itself.",
        |ui| {
            toggle(ui, &mut s.hosting_api);
        },
    );
    heading(ui, "Tools");
    row(
        ui,
        "Terminal",
        "The command \"Open terminal here\" runs, in the repository folder. Empty tries $TERMINAL and common terminals.",
        |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut s.terminal)
                    .hint_text("kitty, alacritty, wezterm start --cwd .")
                    .desired_width(240.0),
            );
        },
    );
}

/// Returns true when a token was saved or removed.
fn accounts(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    settings: &Settings,
    tokens: &mut TokenStore,
    forges: &[Forge],
    actions: &mut Vec<Action>,
) -> bool {
    let p = palette(ui.ctx());
    let mut changed = false;
    ui.label(
        RichText::new(
            "Canopy uses a personal access token to list and act on pull/merge requests. Tokens are sent only to the host they belong to. Without one, Canopy falls back to GH_TOKEN / GITLAB_TOKEN and then to your git credential helper.",
        )
        .size(12.5)
        .color(p.text_dim),
    );
    if !settings.hosting_api {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "{}  Pull and merge requests are turned off in Integrations.",
                icon::INFO
            ))
            .size(12.5)
            .color(p.warning),
        );
    }
    ui.add_space(12.0);
    // Hosts: the two public ones, configured GitLab hosts and those of open repositories.
    let mut hosts: Vec<(ForgeKind, String)> = vec![
        (ForgeKind::GitHub, "github.com".into()),
        (ForgeKind::GitLab, "gitlab.com".into()),
    ];
    for h in settings.gitlab_host_list() {
        hosts.push((ForgeKind::GitLab, h.to_lowercase()));
    }
    for f in forges {
        hosts.push((f.kind, f.host.clone()));
    }
    let mut seen = std::collections::HashSet::new();
    hosts.retain(|(_, h)| seen.insert(h.clone()));

    // Collect finished token tests.
    for test in screen.tests.values_mut() {
        if let TokenTest::Running(rx) = test
            && let Ok(r) = rx.try_recv()
        {
            *test = TokenTest::Done(r);
        }
    }

    for (kind, host) in hosts {
        egui::Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(forge_icon(kind)).size(18.0).color(p.text));
                    ui.label(RichText::new(&host).font(FontId::new(14.0, theme::semibold())));
                    let saved = tokens.get(&host).is_some();
                    let env = canopy_hosting::token_from_env_for(kind, &host, &settings.gitlab_host_list()).is_some();
                    let (status, color) = if saved {
                        (format!("{}  Token saved", icon::CHECK_CIRCLE), p.success)
                    } else if env {
                        (format!("{}  Using the {} environment variable", icon::CHECK_CIRCLE, if kind == ForgeKind::GitHub { "GH_TOKEN" } else { "GITLAB_TOKEN" }), p.success)
                    } else {
                        (format!("{}  No saved token (the git credential helper is tried)", icon::CIRCLE_DASHED), p.text_faint)
                    };
                    ui.label(RichText::new(status).size(12.0).color(color));
                });
                ui.add_space(6.0);
                let input = screen.token_input.entry(host.clone()).or_default();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(input)
                            .password(true)
                            .hint_text("Paste a personal access token")
                            .desired_width(280.0),
                    );
                    let has_input = !input.trim().is_empty();
                    if ui.add_enabled(has_input, egui::Button::new(format!("{}  Save", icon::FLOPPY_DISK))).clicked() {
                        match tokens.set(&host, input) {
                            Ok(()) => {
                                changed = true;
                                screen.token_error = None;
                                let forge = Forge { kind, host: host.clone(), port: None, path: String::new() };
                                let rx = hosting::test_token(forge, input.trim().to_owned(), ui.ctx().clone());
                                screen.tests.insert(host.clone(), TokenTest::Running(rx));
                                input.clear();
                            }
                            Err(e) => screen.token_error = Some(format!("Could not save the token: {e}")),
                        }
                    }
                    if tokens.get(&host).is_some() {
                        if ui.button(format!("{}  Test", icon::PLUGS)).clicked()
                            && let Some(t) = tokens.get(&host)
                        {
                            let forge = Forge { kind, host: host.clone(), port: None, path: String::new() };
                            let rx = hosting::test_token(forge, t.to_owned(), ui.ctx().clone());
                            screen.tests.insert(host.clone(), TokenTest::Running(rx));
                        }
                        if widgets::danger_button(ui, &format!("{}  Remove", icon::TRASH), vec2(0.0, 0.0)).clicked() {
                            match tokens.set(&host, "") {
                                Ok(()) => {
                                    changed = true;
                                    screen.tests.remove(&host);
                                }
                                Err(e) => screen.token_error = Some(format!("Could not remove the token: {e}")),
                            }
                        }
                    }
                });
                match screen.tests.get(&host) {
                    Some(TokenTest::Running(_)) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Checking the token…").size(12.0).color(p.text_dim));
                        });
                    }
                    Some(TokenTest::Done(Ok(user))) => {
                        ui.label(RichText::new(format!("{}  Signed in as {user}", icon::CHECK)).size(12.0).color(p.success));
                    }
                    Some(TokenTest::Done(Err(e))) => {
                        ui.label(RichText::new(format!("{}  {e}", icon::WARNING)).size(12.0).color(p.danger));
                    }
                    None => {}
                }
                ui.add_space(4.0);
                let scopes = match kind {
                    ForgeKind::GitHub => "Needs the repo scope (classic token), or Pull requests: read and write plus Contents: read (fine-grained).",
                    ForgeKind::GitLab => "Needs the api scope (read_api is enough to only browse).",
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(scopes).size(11.5).color(p.text_faint));
                    if ui.link(RichText::new(format!("Create a token {}", icon::ARROW_SQUARE_OUT)).size(11.5)).clicked() {
                        actions.push(Action::OpenExternal(hosting::token_page(kind, &host)));
                    }
                });
            });
        ui.add_space(10.0);
    }
    if let Some(e) = &screen.token_error {
        ui.label(RichText::new(e).color(p.danger));
    }
    changed
}

fn privacy(ui: &mut Ui, s: &mut Settings, actions: &mut Vec<Action>) {
    use crate::ui::legal::Document;
    let p = palette(ui.ctx());
    heading(ui, "Crash reports");
    if crate::crash_reports::available() {
        let before = s.crash_reports;
        let mut on = s.crash_reports == Some(true);
        row(
            ui,
            "Send crash reports",
            "When Canopy crashes, send the error, a stack trace, the Canopy version and your operating system to NecturaLabs, and a short record of each session. Stored by Sentry in the EU and deleted after 90 days. Takes effect immediately.",
            |ui| {
                toggle(ui, &mut on);
            },
        );
        if before != Some(on) && (before.is_some() || on) {
            actions.push(Action::SetCrashReports(on));
        }
    } else {
        ui.label(
            RichText::new("This build of Canopy cannot send crash reports.").color(p.text_dim),
        );
    }
    row(
        ui,
        "Local logs and crash reports",
        "Canopy keeps its log and a copy of every crash report on this computer. They are never uploaded automatically.",
        |ui| {
            if ui
                .button(format!("{}  Open folder", icon::FOLDER_OPEN))
                .clicked()
            {
                actions.push(Action::OpenLogs);
            }
        },
    );
    heading(ui, "Documents");
    row(
        ui,
        "Privacy Policy",
        &format!(
            "What Canopy collects and your rights. Last updated {}.",
            crate::ui::legal::PRIVACY_VERSION
        ),
        |ui| {
            if ui.button(format!("{}  Read", icon::SHIELD_CHECK)).clicked() {
                actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Legal(
                    Document::Privacy,
                )));
            }
        },
    );
    row(
        ui,
        "Terms of Service",
        &format!(
            "The agreement for using Canopy. Last updated {}.",
            crate::ui::legal::TERMS_VERSION
        ),
        |ui| {
            if ui.button(format!("{}  Read", icon::SCROLL)).clicked() {
                actions.push(Action::OpenDialog(crate::views::dialogs::Dialog::Legal(
                    Document::Terms,
                )));
            }
        },
    );
}

fn updates(ui: &mut Ui, s: &mut Settings, update: &UpdateState, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    if !Updater::enabled() {
        ui.label(
            RichText::new("This build of Canopy is updated by your package manager.")
                .color(p.text_dim),
        );
        return;
    }
    row(
        ui,
        "Check for updates",
        "At most once a day, when Canopy starts, it reads a small public file on GitHub naming the latest version. GitHub sees your IP address and your Canopy version, nothing else. Nothing is downloaded or installed by itself.",
        |ui| {
            toggle(ui, &mut s.check_updates);
        },
    );
    row(
        ui,
        "Include pre-releases",
        "Also offer beta and release-candidate versions.",
        |ui| {
            toggle(ui, &mut s.include_prereleases);
        },
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Canopy {}", env!("CARGO_PKG_VERSION")))
                .font(FontId::new(13.5, theme::medium())),
        );
        let (text, color) = match update {
            UpdateState::Idle => (String::new(), p.text_dim),
            UpdateState::Checking => ("Checking…".to_owned(), p.text_dim),
            UpdateState::UpToDate => (format!("{}  Up to date", icon::CHECK), p.success),
            UpdateState::Available(a) => (
                format!(
                    "{}  Canopy {} is available",
                    icon::ARROW_CIRCLE_UP,
                    a.version
                ),
                p.accent,
            ),
            UpdateState::Failed(e) => (format!("{}  {e}", icon::WARNING), p.danger),
        };
        ui.label(RichText::new(text).size(12.5).color(color));
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                *update != UpdateState::Checking,
                egui::Button::new(format!("{}  Check now", icon::ARROWS_CLOCKWISE)),
            )
            .clicked()
        {
            actions.push(Action::CheckUpdates);
        }
        if matches!(update, UpdateState::Available(_))
            && widgets::accent_button(
                ui,
                &format!("{}  What's new", icon::SPARKLE),
                vec2(0.0, 0.0),
            )
            .clicked()
        {
            actions.push(Action::ShowUpdate);
        }
        if ui.button(format!("{}  All releases", icon::LIST)).clicked() {
            actions.push(Action::OpenExternal(updater::RELEASES_PAGE.to_owned()));
        }
    });
    if let Some(v) = s.skipped_version.clone() {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Skipping version {v}."))
                    .size(12.0)
                    .color(p.text_dim),
            );
            if ui.small_button("Stop skipping").clicked() {
                s.skipped_version = None;
            }
        });
    }
}

fn keyboard(ui: &mut Ui) {
    let p = palette(ui.ctx());
    ui.label(
        RichText::new(format!("Press ? (or {}) anywhere for this list as an overlay. Single keys work when no text field has focus; Super is never used.", crate::keymap::hint(crate::keymap::Command::Cheatsheet).split(" · ").next().unwrap_or_default()))
            .size(12.5)
            .color(p.text_dim),
    );
    ui.add_space(10.0);
    for section in KeySection::ALL {
        heading(ui, section.title());
        egui::Grid::new(("keys", section.title()))
            .num_columns(2)
            .spacing([24.0, 6.0])
            .striped(false)
            .show(ui, |ui| {
                for b in BINDINGS.iter().filter(|b| b.section == section) {
                    ui.label(RichText::new(b.keys()).monospace().size(12.0).color(p.text));
                    ui.label(RichText::new(b.description).size(12.5).color(p.text_dim));
                    ui.end_row();
                }
            });
        ui.add_space(8.0);
    }
}
