//! The settings screen (Ctrl+,): sections on the left, live-applied controls on the right.
//! Every setting changes behaviour; nothing here is decorative.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

use canopy_core::forge::{Forge, ForgeKind};
use canopy_core::identity::{Field, Identity, Scope, Source};

use crate::action::Action;
use crate::hosting::{self, TokenStore};
use crate::keymap::{BINDINGS, Section as KeySection};
use crate::settings::{CloneProtocol, PullMode, Settings, WhitespaceMode};
use crate::ui::theme::{self, Palette, ThemeId, palette};
use crate::ui::widgets;
use crate::ui::widgets::forge_icon;
use crate::updater::{self, UpdateState, Updater};
use crate::views::dialogs::Dialog;

/// The sections, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
    zoom_preview: Option<f32>,
    picker: crate::ui::folder_picker::FolderPicker,
    token_input: HashMap<String, String>,
    tests: HashMap<String, TokenTest>,
    token_error: Option<String>,
    identity_for: Option<PathBuf>,
    loaded_identity: Option<Identity>,
    identity_name: String,
    identity_email: String,
    identity_scope: Scope,
}

impl std::fmt::Debug for SettingsScreen {
    // The token fields hold secrets being typed: show only which hosts have one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsScreen")
            .field("section", &self.section)
            .field("token_input", &self.token_input.keys().collect::<Vec<_>>())
            .field("tests", &self.tests)
            .field("token_error", &self.token_error)
            .field("identity_scope", &self.identity_scope)
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

pub struct IdentityView<'a> {
    pub repo: Option<&'a Path>,
    pub identity: Option<&'a Identity>,
    pub error: Option<&'a str>,
}

pub struct SettingsData<'a> {
    pub forges: &'a [Forge],
    pub update: &'a UpdateState,
    pub identity: IdentityView<'a>,
}

/// Draws the screen. `forges` are the GitHub/GitLab remotes of open repositories, so their hosts
/// show up under Accounts.
pub fn show(
    ctx: &egui::Context,
    screen: &mut SettingsScreen,
    settings: &mut Settings,
    tokens: &mut TokenStore,
    data: SettingsData<'_>,
    actions: &mut Vec<Action>,
) -> SettingsResult {
    let p = palette(ctx);
    let mut result = SettingsResult::default();
    screen.picker.begin_frame();
    let screen_rect = ctx.content_rect();
    let width = (screen_rect.width() - theme::XL * 2.0).clamp(0.0, 900.0);
    let height = (screen_rect.height() - theme::XL * 2.0).clamp(0.0, 640.0);
    let sidebar = width >= 720.0;
    let nav_width = 180.0;
    let modal = egui::Modal::new(egui::Id::new("settings"))
        .frame(
            egui::Frame::new()
                .fill(p.elevated)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                .shadow(ctx.global_style().visuals.window_shadow)
                .inner_margin(egui::Margin::ZERO),
        )
        .backdrop_color(p.backdrop())
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_height(height);
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                if sidebar {
                    egui::Frame::new()
                        .fill(p.panel)
                        .corner_radius(CornerRadius {
                            nw: theme::RADIUS_LG,
                            sw: theme::RADIUS_LG,
                            ne: 0,
                            se: 0,
                        })
                        .inner_margin(egui::Margin::same(theme::MD as i8))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(nav_width - theme::MD * 2.0);
                                ui.set_height(height - theme::MD * 2.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(icon::GEAR_SIX)
                                            .size(theme::TITLE_SIZE)
                                            .color(p.accent),
                                    );
                                    ui.label(
                                        RichText::new("Settings").font(FontId::new(
                                            theme::TITLE_SIZE,
                                            theme::semibold(),
                                        )),
                                    );
                                });
                                ui.add_space(theme::LG);
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    for s in Section::ALL {
                                        nav_item(ui, &mut screen.section, s);
                                    }
                                });
                            })
                        });
                }
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(theme::LG as i8))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(
                                width - if sidebar { nav_width } else { 0.0 } - theme::LG * 2.0,
                            );
                            ui.spacing_mut().item_spacing.x = theme::SM;
                            ui.horizontal(|ui| {
                                let (glyph, title) = screen.section.label();
                                ui.label(
                                    RichText::new(glyph).size(theme::TITLE_SIZE).color(p.accent),
                                );
                                if sidebar {
                                    ui.label(
                                        RichText::new(title).font(FontId::new(
                                            theme::TITLE_SIZE,
                                            theme::semibold(),
                                        )),
                                    );
                                } else {
                                    egui::ComboBox::from_id_salt("settings-section")
                                        .selected_text(title)
                                        .show_ui(ui, |ui| {
                                            for section in Section::ALL {
                                                ui.selectable_value(
                                                    &mut screen.section,
                                                    section,
                                                    section.label().1,
                                                );
                                            }
                                        });
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if widgets::icon_button(ui, icon::X, "Close settings (Esc)")
                                            .clicked()
                                        {
                                            result.close = true;
                                        }
                                    },
                                );
                            });
                            ui.add_space(theme::MD);
                            egui::ScrollArea::vertical()
                                .id_salt(("settings-content", screen.section))
                                .auto_shrink([false, false])
                                .max_height((height - theme::LG * 2.0 - 26.0 - theme::MD).max(0.0))
                                .show(ui, |ui| {
                                    ui.set_max_width(ui.available_width());
                                    match screen.section {
                                        Section::Appearance => {
                                            appearance(ui, settings, &mut screen.zoom_preview)
                                        }
                                        Section::Git => {
                                            git(ui, screen, settings, &data.identity, actions)
                                        }
                                        Section::Integrations => {
                                            integrations(ui, settings, &mut screen.picker)
                                        }
                                        Section::Accounts => {
                                            result.tokens_changed |= accounts(
                                                ui,
                                                screen,
                                                settings,
                                                tokens,
                                                data.forges,
                                                actions,
                                            )
                                        }
                                        Section::Updates => {
                                            updates(ui, settings, data.update, actions)
                                        }
                                        Section::Privacy => privacy(ui, settings, actions),
                                        Section::Keyboard => keyboard(ui),
                                    }
                                });
                        })
                    });
            });
        });
    screen.picker.end_frame();
    result.close |= modal.should_close();
    result
}

fn nav_item(ui: &mut Ui, current: &mut Section, s: Section) {
    let p = palette(ui.ctx());
    let selected = *current == s;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::RADIUS),
            Stroke::new(1.5, p.accent),
            egui::StrokeKind::Inside,
        );
    }
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

/// A preference and its controls. Compact columns stack the controls below the help.
fn row(ui: &mut Ui, title: &str, help: &str, control: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    let width = ui.available_width();
    let label = |ui: &mut Ui| {
        ui.label(RichText::new(title).font(FontId::new(theme::BODY_SIZE, theme::medium())));
        if !help.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(help)
                        .size(theme::SMALL_SIZE)
                        .color(p.text_dim),
                )
                .wrap(),
            );
        }
    };
    ui.push_id(title, |ui| {
        if width < 620.0 {
            label(ui);
            ui.add_space(theme::XS);
            ui.horizontal_wrapped(control);
        } else {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(width * 0.50, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        // Without this the column is only as wide as its text, and the controls
                        // of rows with short help text sit at different distances from the edge.
                        ui.set_min_width(width * 0.50);
                        label(ui);
                    },
                );
                ui.allocate_ui_with_layout(
                    vec2(width * 0.50 - 2.0 * theme::SM, 0.0),
                    egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
                    control,
                );
            });
        }
    });
    ui.add_space(theme::SM);
    ui.separator();
    ui.add_space(theme::SM);
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    widgets::section_label(ui, text);
    ui.add_space(8.0);
}

fn appearance(ui: &mut Ui, s: &mut Settings, preview: &mut Option<f32>) {
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
            zoom_slider(ui, s, preview);
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

fn zoom_slider(ui: &mut Ui, s: &mut Settings, draft: &mut Option<f32>) -> egui::Response {
    let mut preview = draft.unwrap_or(s.zoom);
    ui.spacing_mut().slider_width = 160.0_f32.min((ui.available_width() - 72.0).max(48.0));
    let response = ui.add(
        egui::Slider::new(&mut preview, 0.8..=1.6)
            .step_by(0.05)
            .update_while_editing(false)
            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
            .custom_parser(|text| {
                text.trim()
                    .trim_end_matches('%')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|v| v / 100.0)
            }),
    );
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(preview), "Zoom"));
    if response.dragged() || response.is_pointer_button_down_on() {
        *draft = Some(preview);
    } else {
        if response.changed() || draft.is_some() {
            s.zoom = preview;
        }
        *draft = None;
    }
    response
}

/// A theme preview: a miniature of the window drawn from the theme's own palette.
fn theme_card(ui: &mut Ui, id: ThemeId, selected: bool) -> egui::Response {
    let p = palette(ui.ctx());
    let tp = id.palette();
    let (rect, resp) = ui.allocate_exact_size(
        vec2(
            150.0_f32.min((ui.available_width() - theme::MD).max(120.0)),
            112.0,
        ),
        Sense::click(),
    );
    let hover = resp.hovered() || resp.has_focus();
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

/// Where an identity value comes from, as a short phrase.
pub(crate) fn source_label(source: Option<&Source>) -> String {
    source_label_in(source, &crate::settings::home_dir())
}

/// [`source_label`], with `home` shown as `~`.
fn source_label_in(source: Option<&Source>, home: &Path) -> String {
    match source {
        Some(Source::Environment(key)) => format!("the {key} environment variable"),
        Some(Source::Config { scope, .. }) if scope == "command" => "the git command line".into(),
        Some(Source::Config { scope, origin }) => {
            let file = origin.strip_prefix("file:").unwrap_or(origin);
            // An unknown home directory is an empty path, which every path starts with.
            let file = Path::new(file)
                .strip_prefix(home)
                .ok()
                .filter(|_| !home.as_os_str().is_empty())
                .map_or_else(|| file.to_owned(), |rest| format!("~/{}", rest.display()));
            let scope = match scope.as_str() {
                "local" => "this repository",
                "worktree" => "this worktree",
                other => other,
            };
            format!("{scope}: {file}")
        }
        Some(Source::Guessed) => "guessed by Git from this computer".into(),
        None => "not set".into(),
    }
}

/// What Git would use now for one part of an identity, and why.
fn in_use(field: &Field) -> String {
    if field.value.is_empty() {
        "Not set.".into()
    } else {
        format!(
            "In use now: {} ({}).",
            field.value,
            source_label(field.source.as_ref())
        )
    }
}

fn identity_group(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    repo: Option<&Path>,
    identity: Option<&Identity>,
    error: Option<&str>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let repo_path = repo.map(Path::to_path_buf);
    if screen.identity_for != repo_path {
        screen.identity_scope = if repo.is_some() {
            Scope::Local
        } else {
            Scope::Global
        };
        screen.loaded_identity = None;
    }
    if screen.identity_for != repo_path || screen.loaded_identity.as_ref() != identity {
        screen.identity_for = repo_path.clone();
        screen.loaded_identity = identity.cloned();
        // Only values someone chose are offered for editing: a guess built from the user and host
        // names would be saved as if it were a choice.
        let chosen = |field: &Field| {
            if field.explicit() {
                field.value.clone()
            } else {
                String::new()
            }
        };
        screen.identity_name = identity.map_or_else(String::new, |i| chosen(&i.author.name));
        screen.identity_email = identity.map_or_else(String::new, |i| chosen(&i.author.email));
    }
    heading(ui, "Identity for future commits");
    if let Some(error) = error {
        ui.label(RichText::new(error).color(p.warning));
    } else if let Some(identity) = identity {
        if let Some(warning) = identity.warning() {
            ui.label(RichText::new(format!("{}  {warning}", icon::WARNING)).color(p.warning));
            ui.add_space(theme::SM);
        }
    } else {
        ui.label(RichText::new("Reading Git identity…").color(p.text_dim));
    }
    let current =
        |pick: fn(&Identity) -> &Field| identity.map(pick).map(in_use).unwrap_or_default();
    row(ui, "Name", &current(|i| &i.author.name), |ui| {
        ui.add(egui::TextEdit::singleline(&mut screen.identity_name).desired_width(260.0));
    });
    row(ui, "Email", &current(|i| &i.author.email), |ui| {
        ui.add(egui::TextEdit::singleline(&mut screen.identity_email).desired_width(260.0));
    });
    if let Some(i) = identity.filter(|i| i.committer != i.author) {
        row(
            ui,
            "Committer",
            &format!(
                "Git records {} <{}> as the committer ({}). Saving here does not change it.",
                i.committer.name.value,
                i.committer.email.value,
                source_label(i.committer.name.source.as_ref())
            ),
            |_| {},
        );
    }
    if repo.is_some() {
        row(
            ui,
            "Save to",
            "This repository writes its own .git/config; all repositories write your global Git configuration.",
            |ui| {
                ui.selectable_value(&mut screen.identity_scope, Scope::Local, "This repository");
                ui.selectable_value(
                    &mut screen.identity_scope,
                    Scope::Global,
                    "All repositories",
                );
            },
        );
    }
    let can_save =
        !screen.identity_name.trim().is_empty() && !screen.identity_email.trim().is_empty();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_save, egui::Button::new("Save identity"))
            .clicked()
        {
            actions.push(Action::SaveIdentity {
                repo: repo_path.clone(),
                scope: screen.identity_scope,
                name: screen.identity_name.clone(),
                email: screen.identity_email.clone(),
            });
        }
        if ui.button("Clear…").clicked() {
            actions.push(Action::OpenDialog(Dialog::ConfirmClearIdentity {
                repo: repo_path.clone(),
                scope: screen.identity_scope,
            }));
        }
    });
    ui.add_space(theme::XS);
    ui.label(
        RichText::new("Changes apply to future commits only; commits already made keep their authors. An environment variable or an author setting can still override what you save.")
            .size(theme::SMALL_SIZE)
            .color(p.text_faint),
    );
}

fn git(
    ui: &mut Ui,
    screen: &mut SettingsScreen,
    s: &mut Settings,
    identity: &IdentityView<'_>,
    actions: &mut Vec<Action>,
) {
    identity_group(
        ui,
        screen,
        identity.repo,
        identity.identity,
        identity.error,
        actions,
    );
    ui.add_space(theme::LG);
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

fn integrations(
    ui: &mut Ui,
    s: &mut Settings,
    picker: &mut crate::ui::folder_picker::FolderPicker,
) {
    heading(ui, "Cloning");
    row(
        ui,
        "Default clone folder",
        "Where the clone dialog puts new repositories. Empty uses ~/src when it exists, else your home folder.",
        |ui| {
            if let Some(dir) = picker.show(
                ui,
                "default-clone-folder",
                &crate::settings::expand_home(&s.clone_dir),
            ) {
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
    row(
        ui,
        "Editor",
        "The command \"Open in editor\" runs, with the file added at the end. Empty uses your system's text editor.",
        |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut s.editor)
                    .hint_text("code, zed, gnome-text-editor")
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
    // Hosts: the two public ones, configured GitLab hosts and those of open repositories. Tokens
    // are kept per authority, so a host on a non-default port gets its own entry.
    let account = |kind, host: &str| Forge {
        kind,
        host: host.to_lowercase(),
        port: None,
        path: String::new(),
    };
    let mut hosts: Vec<Forge> = vec![
        account(ForgeKind::GitHub, "github.com"),
        account(ForgeKind::GitLab, "gitlab.com"),
    ];
    for h in settings.gitlab_host_list() {
        hosts.push(account(ForgeKind::GitLab, &h));
    }
    for f in forges {
        hosts.push(Forge {
            path: String::new(),
            ..f.clone()
        });
    }
    let mut seen = std::collections::HashSet::new();
    hosts.retain(|f| seen.insert(f.authority()));

    // Collect finished token tests.
    for test in screen.tests.values_mut() {
        if let TokenTest::Running(rx) = test
            && let Ok(r) = rx.try_recv()
        {
            *test = TokenTest::Done(r);
        }
    }

    for forge in hosts {
        let kind = forge.kind;
        let host = forge.authority();
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
                    let saved = tokens.get_for(&forge).is_some();
                    let env = canopy_hosting::token_from_env_for(kind, &forge.host, &settings.gitlab_host_list()).is_some();
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
                                let rx = hosting::test_token(forge.clone(), input.trim().to_owned(), ui.ctx().clone());
                                screen.tests.insert(host.clone(), TokenTest::Running(rx));
                                input.clear();
                            }
                            Err(e) => screen.token_error = Some(format!("Could not save the token: {e}")),
                        }
                    }
                    if tokens.get_for(&forge).is_some() {
                        if ui.button(format!("{}  Test", icon::PLUGS)).clicked()
                            && let Some(t) = tokens.get_for(&forge)
                        {
                            let rx = hosting::test_token(forge.clone(), t.to_owned(), ui.ctx().clone());
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
                        actions.push(Action::OpenUrl(hosting::token_page(kind, &host)));
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
    heading(ui, "Diagnostics");
    row(
        ui,
        "Log detail",
        "Normal records operations, warnings and errors. Detailed adds every git command and API call with its timing: turn it on while you reproduce a problem. The CANOPY_LOG environment variable overrides this.",
        |ui| {
            let before = s.log_level;
            egui::ComboBox::from_id_salt("log-level")
                .selected_text(s.log_level.label())
                .show_ui(ui, |ui| {
                    for level in crate::logging::LogLevel::ALL {
                        ui.selectable_value(&mut s.log_level, level, level.label());
                    }
                });
            if s.log_level != before {
                crate::logging::set_level(s.log_level);
            }
        },
    );
    row(
        ui,
        "Logs and crash reports",
        "Kept on this computer, one log per run, and never uploaded. Copy diagnostics puts your versions, this run's log file and its latest warnings on the clipboard, for a bug report.",
        |ui| {
            if ui
                .button(format!("{}  Open folder", icon::FOLDER_OPEN))
                .clicked()
            {
                actions.push(Action::OpenLogs);
            }
            if ui
                .button(format!("{}  Copy diagnostics", icon::CLIPBOARD_TEXT))
                .clicked()
            {
                actions.push(Action::CopyDiagnostics);
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
        "At most once a day, when Canopy starts, it reads a small public file on GitHub naming the latest build. GitHub sees your IP address and your Canopy build number, nothing else. Nothing is downloaded or installed until you choose Update.",
        |ui| {
            toggle(ui, &mut s.check_updates);
        },
    );
    row(
        ui,
        "Include beta builds",
        "Also offer beta builds, which are tried out less before they are published.",
        |ui| {
            toggle(ui, &mut s.include_prereleases);
        },
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Canopy {}", crate::build_info::label()))
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
                    a.label()
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
            actions.push(Action::OpenUrl(updater::RELEASES_PAGE.to_owned()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_changes_only_after_the_drag_ends() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut settings = Settings::default();
        let mut draft = None;
        let mut rect = egui::Rect::NOTHING;
        let mut draw = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        rect = zoom_slider(ui, &mut settings, &mut draft).rect;
                    });
                },
            );
            output.textures_delta.clear();
            (settings.zoom, rect)
        };
        let (_, rect) = draw(Vec::new());
        let pos = rect.left_center() + vec2(80.0, 0.0);
        let (zoom, _) = draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        assert_eq!(zoom, 1.0, "applying zoom while dragging moves the control");
        let (zoom, _) = draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        assert!(zoom > 1.0, "released slider must apply its preview");
    }

    #[test]
    fn typing_a_zoom_percentage_waits_for_enter() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut settings = Settings::default();
        let mut draft = None;
        let mut time = 1.0;
        let mut draw = |events| {
            time += 0.05;
            let mut rect = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    rect = zoom_slider(ui, &mut settings, &mut draft).rect;
                },
            );
            output.textures_delta.clear();
            (settings.zoom, rect)
        };
        let (_, rect) = draw(Vec::new());
        let pos = rect.right_center() - vec2(16.0, 0.0);
        for _ in 0..2 {
            draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]);
            draw(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
        }
        assert!(
            ctx.egui_wants_keyboard_input(),
            "zoom value must have editing focus"
        );
        let (zoom, _) = draw(vec![egui::Event::Text("1".into())]);
        assert_eq!(zoom, 1.0, "a partial percentage must not rescale the app");
        draw(vec![egui::Event::Text("50".into())]);
        let (zoom, _) = draw(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(zoom, 1.5, "Enter must apply the complete percentage");
    }

    #[test]
    fn settings_fit_a_compact_zoomed_window() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, ThemeId::Midnight, true);
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 375.0));
        let mut settings = Settings::default();
        let mut tokens = TokenStore::default();
        for section in Section::ALL {
            let mut screen = SettingsScreen::at(section);
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(bounds),
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui.ctx(),
                            &mut screen,
                            &mut settings,
                            &mut tokens,
                            SettingsData {
                                forges: &[],
                                update: &UpdateState::Idle,
                                identity: IdentityView {
                                    repo: None,
                                    identity: None,
                                    error: None,
                                },
                            },
                            &mut Vec::new(),
                        );
                    },
                );
                output.textures_delta.clear();
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("settings")))
                .expect("settings area");
            assert!(
                bounds.contains_rect(rect),
                "{section:?}: {rect:?} exceeds {bounds:?}"
            );
        }
    }
    fn field(value: &str, source: Option<Source>) -> Field {
        Field {
            value: value.into(),
            source,
        }
    }

    fn identity(name: Field, email: Field) -> Identity {
        let person = canopy_core::identity::Person {
            name,
            email,
            usable: true,
        };
        Identity {
            author: person.clone(),
            committer: person,
        }
    }

    fn local_source() -> Option<Source> {
        Some(Source::Config {
            scope: "local".into(),
            origin: "file:.git/config".into(),
        })
    }

    #[test]
    fn sources_say_where_a_value_comes_from() {
        let home = Path::new("/home/ada");
        let label = |source: &Source| source_label_in(Some(source), home);
        assert_eq!(
            label(&Source::Environment("GIT_AUTHOR_NAME")),
            "the GIT_AUTHOR_NAME environment variable"
        );
        assert_eq!(
            label(&Source::Config {
                scope: "command".into(),
                origin: "command line:".into(),
            }),
            "the git command line"
        );
        assert_eq!(
            label(&local_source().expect("a source")),
            "this repository: .git/config"
        );
        assert_eq!(
            label(&Source::Config {
                scope: "global".into(),
                origin: "file:/home/ada/.gitconfig".into(),
            }),
            "global: ~/.gitconfig"
        );
        assert_eq!(
            label(&Source::Config {
                scope: "global".into(),
                origin: "file:/etc/elsewhere/gitconfig".into(),
            }),
            "global: /etc/elsewhere/gitconfig"
        );
        assert_eq!(label(&Source::Guessed), "guessed by Git from this computer");
        assert_eq!(source_label_in(None, home), "not set");
    }

    #[test]
    fn an_unknown_home_directory_is_never_shown_as_a_tilde() {
        let source = Source::Config {
            scope: "global".into(),
            origin: "file:/etc/gitconfig".into(),
        };
        assert_eq!(
            source_label_in(Some(&source), Path::new("")),
            "global: /etc/gitconfig"
        );
    }

    #[test]
    fn the_editor_offers_chosen_values_and_never_a_guess() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut settings = Settings::default();
        let mut screen = SettingsScreen::default();
        let repo = std::path::PathBuf::from("/repo");
        let mut draw = |screen: &mut SettingsScreen, repo: Option<&Path>, id: &Identity| {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    git(
                        ui,
                        screen,
                        &mut settings,
                        &IdentityView {
                            repo,
                            identity: Some(id),
                            error: None,
                        },
                        &mut actions,
                    );
                });
            });
            output.textures_delta.clear();
            actions
        };

        let guessed = identity(
            field("someone", Some(Source::Guessed)),
            field("someone@host.(none)", Some(Source::Guessed)),
        );
        draw(&mut screen, Some(&repo), &guessed);
        assert!(
            screen.identity_name.is_empty() && screen.identity_email.is_empty(),
            "a value Git made up must not be offered as if someone chose it"
        );
        assert_eq!(screen.identity_scope, Scope::Local);

        let chosen = identity(
            field("Ada Lovelace", local_source()),
            field("ada@example.com", local_source()),
        );
        let actions = draw(&mut screen, Some(&repo), &chosen);
        assert_eq!(screen.identity_name, "Ada Lovelace");
        assert_eq!(screen.identity_email, "ada@example.com");
        assert!(actions.is_empty(), "drawing alone changes nothing");

        draw(&mut screen, None, &chosen);
        assert_eq!(
            screen.identity_scope,
            Scope::Global,
            "without a repository only the global configuration can be written"
        );
    }
}
