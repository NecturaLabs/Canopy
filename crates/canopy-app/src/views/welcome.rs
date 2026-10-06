//! The start screen: brand, quick actions, first-run keys and recent repositories.

use std::path::Path;

use egui::{FontId, Id, RichText, Ui, vec2};
use egui_phosphor::regular as icon;

use crate::action::Action;
use crate::keymap::{Command, hint};
use crate::settings::{self, Settings};
use crate::ui::art;
use crate::ui::kit::chip::KeyChord;
use crate::ui::kit::row::{ListRow, SectionHeader};
use crate::ui::kit::surface::{Callout, Card, NoticeAction};
use crate::ui::kit::{IconButton, Size, Tone};
use crate::ui::theme::{self, palette};
use crate::views::dialogs::Dialog;

/// Widest the action cards, the first-run keys and the recent list grow.
const COLUMN_WIDTH: f32 = 660.0;
/// Height of an action card.
const CARD_HEIGHT: f32 = 128.0;
/// Side of the tinted tile behind an action card's icon.
const ICON_TILE: f32 = 40.0;
/// Width of the keys column in the first-run list, so the descriptions line up.
const KEYS_COLUMN: f32 = 104.0;

/// Draws the welcome screen.
pub fn show(ui: &mut Ui, settings: &Settings, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let rect = ui.max_rect();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Sit the whole stack a little above the middle, from its height last frame; a tall
            // window otherwise leaves everything in its upper half.
            let height_id = Id::new("welcome-content-height");
            let last: f32 = ui.ctx().data(|d| d.get_temp(height_id)).unwrap_or(0.0);
            let top = if rect.height() < 400.0 {
                theme::XS
            } else {
                ((rect.height() - last) * 0.5).max((rect.height() * 0.04).max(theme::LG))
            };
            let start = ui.cursor().top();
            ui.vertical_centered(|ui| {
                ui.add_space(top);
                // The first-run tips take room: the picture gives way so the actions stay in view.
                let art_share = if settings.seen_intro { 0.46 } else { 0.34 };
                let art_w = (rect.width() * 0.5)
                    .min(
                        rect.height()
                            * if rect.height() < 400.0 {
                                0.30
                            } else {
                                art_share
                            },
                    )
                    .clamp(120.0, 520.0);
                art::show(ui, art::welcome(), vec2(art_w, art_w * 25.0 / 64.0));
                ui.add_space(theme::XS);
                ui.label(
                    RichText::new("Canopy")
                        .font(theme::heading(m.hero_size))
                        .color(p.text),
                );
                ui.label(
                    RichText::new("A fast, beautiful, Linux-first Git client")
                        .size(m.title_size)
                        .color(p.text_dim),
                );
                ui.add_space(theme::XL);

                let total = (ui.available_width() - theme::XL * 2.0).min(COLUMN_WIDTH);
                let card_w = (total - theme::MD * 2.0) / 3.0;
                ui.allocate_ui_with_layout(
                    vec2(total, CARD_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = theme::MD;
                        if card(
                            ui,
                            icon::FOLDER_OPEN,
                            "Open",
                            "A repository on this computer",
                            Command::Open,
                            card_w,
                        )
                        .clicked()
                        {
                            actions.push(Action::OpenDialog(Dialog::open()));
                        }
                        if card(
                            ui,
                            icon::CLOUD_ARROW_DOWN,
                            "Clone",
                            "From GitHub, GitLab or any URL",
                            Command::Clone,
                            card_w,
                        )
                        .clicked()
                        {
                            actions.push(Action::OpenDialog(Dialog::clone_default(settings)));
                        }
                        if card(
                            ui,
                            icon::PLANT,
                            "Init",
                            "Start a new repository in a folder",
                            Command::Init,
                            card_w,
                        )
                        .clicked()
                        {
                            actions.push(Action::OpenDialog(Dialog::new_repository(String::new())));
                        }
                    },
                );

                if !settings.seen_intro {
                    ui.add_space(theme::LG);
                    intro(ui, total, actions);
                }

                if !settings.recent.is_empty() {
                    ui.add_space(theme::XL);
                    ui.allocate_ui_with_layout(
                        vec2(total, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(total);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            SectionHeader::new("Recent repositories")
                                .count(settings.recent.len())
                                .first(true)
                                .show(ui);
                            let home = settings::home_dir();
                            for path in &settings.recent {
                                recent(ui, path, &home, actions);
                            }
                        },
                    );
                }
                ui.add_space(theme::XL);
                if settings.seen_intro {
                    footer(ui);
                }
            });
            let used = ui.cursor().top() - start - top;
            if (used - last).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(height_id, used));
                ui.ctx().request_repaint();
            }
        });
}

/// The single key of `command` where it has one (the keymap lists the chord first, then the bare
/// key: `Ctrl+Shift+F · f` gives `f`), else its chord.
fn key(command: Command) -> String {
    let all = hint(command);
    all.rsplit(" · ").next().unwrap_or_default().to_owned()
}

/// The two keys worth knowing first, as keycaps: the command palette and the shortcut sheet.
fn footer(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let hint_text = |ui: &mut Ui, text: &str| {
        ui.label(RichText::new(text).size(m.small_size).color(p.text_faint));
    };
    // Centred: lay it out once at its width.
    let width_id = Id::new("welcome-footer-width");
    let width: f32 = ui.ctx().data(|d| d.get_temp(width_id)).unwrap_or(0.0);
    let pad = ((ui.available_width() - width) / 2.0).max(0.0);
    let row = ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.spacing_mut().item_spacing.x = theme::SM;
        let start = ui.cursor().left();
        KeyChord::command(Command::Palette).show(ui);
        hint_text(ui, "Command palette");
        ui.add_space(theme::MD);
        KeyChord::new(key(Command::Cheatsheet)).show(ui);
        hint_text(ui, "Keyboard shortcuts");
        ui.cursor().left() - start - theme::SM
    });
    if (row.inner - width).abs() > 0.5 {
        ui.ctx().data_mut(|d| d.insert_temp(width_id, row.inner));
        ui.ctx().request_repaint();
    }
}

/// A recent repository: its name, the folder it is in (home as `~`), opening on click, with a
/// button to forget it.
fn recent(ui: &mut Ui, path: &Path, home: &Path, actions: &mut Vec<Action>) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shown = path.display().to_string();
    let mut forget = false;
    let mut row = ListRow::new(("recent", path))
        .icon(icon::GIT_BRANCH)
        .label(name.clone())
        .tooltip(shown)
        .actions(|ui| {
            forget = IconButton::new(icon::X, format!("Forget {name}"))
                .size(Size::Sm)
                .id_salt(("forget", path))
                .show(ui)
                .clicked();
        });
    if let Some(folder) = folder_label(path, home) {
        row = row.detail(folder);
    }
    let row = row.show(ui);
    if forget {
        actions.push(Action::ForgetRecent(path.to_path_buf()));
    } else if row.clicked() {
        actions.push(Action::OpenRepo(path.to_path_buf()));
    }
}

/// The folder `path` is in, as people say it: the home folder as `~`. `None` at the top of the
/// file system.
fn folder_label(path: &Path, home: &Path) -> Option<String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
    Some(crate::views::dialogs::tilde_in(parent, home))
}

/// First-run tips: how Canopy is meant to be driven, as keys and what they do.
fn intro(ui: &mut Ui, width: f32, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let keys: [(String, &str); 5] = [
        (
            hint(Command::Palette),
            "Command palette: every action, branch and repository",
        ),
        (key(Command::Cheatsheet), "Every keyboard shortcut"),
        (
            format!(
                "{} · {} · {}",
                key(Command::History),
                key(Command::Changes),
                key(Command::Requests)
            ),
            "History, Changes and Pull requests",
        ),
        (
            format!(
                "{} · {} · {}",
                key(Command::Fetch),
                key(Command::Pull),
                key(Command::Push)
            ),
            "Fetch, pull and push",
        ),
        (
            hint(Command::Settings),
            "Themes, Git defaults and your GitHub and GitLab accounts",
        ),
    ];
    ui.allocate_ui_with_layout(
        vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            let shown = Callout::new(Tone::Accent, "Everything is a few keys away:")
                .title("Canopy is keyboard-first")
                .icon(icon::LIGHTBULB)
                .id_salt("welcome-intro")
                .action(NoticeAction::new("Got it"))
                .show_with(ui, |ui| {
                    // The tips span the column, like the cards above them.
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = theme::XS;
                    for (chord, what) in &keys {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = theme::MD;
                            ui.allocate_ui_with_layout(
                                vec2(KEYS_COLUMN, Size::Sm.height()),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_width(KEYS_COLUMN);
                                    KeyChord::new(chord.clone()).show(ui);
                                },
                            );
                            ui.label(RichText::new(*what).size(m.body_size).color(p.text));
                        });
                    }
                });
            if shown.value == Some(0) {
                actions.push(Action::DismissIntro);
            }
        },
    );
}

/// A quick action: its icon on a tinted tile, the command's keys, a title and one line of help.
/// Returns its response.
fn card(
    ui: &mut Ui,
    glyph: &str,
    title: &str,
    sub: &str,
    command: Command,
    w: f32,
) -> egui::Response {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    Card::new()
        .interactive(true)
        .width(w)
        .label(title)
        .id_salt(("welcome-card", title))
        .show(ui, |ui| {
            ui.set_min_height(CARD_HEIGHT - 2.0 * m.card_pad);
            ui.spacing_mut().item_spacing.y = theme::XS;
            ui.horizontal(|ui| {
                let (tile, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(ICON_TILE), egui::Sense::hover());
                ui.painter().rect_filled(
                    tile,
                    egui::CornerRadius::same(m.radius),
                    Tone::Accent.tint(p),
                );
                ui.painter().text(
                    tile.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    FontId::proportional(m.icon_xl),
                    p.accent,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    KeyChord::command(command).show(ui);
                });
            });
            ui.add_space(theme::SM);
            ui.label(
                RichText::new(title)
                    .font(theme::heading(m.subtitle_size))
                    .color(p.text),
            );
            let mut job = egui::text::LayoutJob::simple(
                sub.to_owned(),
                FontId::proportional(m.small_size),
                p.text_dim,
                ui.available_width(),
            );
            job.wrap.max_rows = 2;
            ui.label(job);
        })
        .response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_action_text_fits_a_minimum_zoomed_window() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut card_rect = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            card_rect = card(
                ui,
                icon::CLOUD_ARROW_DOWN,
                "Clone",
                "GitHub, GitLab or any URL",
                Command::Clone,
                160.0,
            )
            .rect;
        });
        output.textures_delta.clear();
        assert!(card_rect.width() <= 160.5, "card grew to {card_rect:?}");
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                assert!(
                    card_rect.contains_rect(text.visual_bounding_rect()),
                    "quick action text overflows: {:?}",
                    text.visual_bounding_rect()
                );
            }
        }
    }

    #[test]
    fn recent_repositories_name_their_folder_with_home_as_a_tilde() {
        let home = Path::new("/home/ada");
        let at = |p: &str| folder_label(Path::new(p), home);
        assert_eq!(at("/home/ada/src/canopy").as_deref(), Some("~/src"));
        assert_eq!(at("/home/ada/canopy").as_deref(), Some("~"));
        assert_eq!(at("/srv/git/canopy").as_deref(), Some("/srv/git"));
        // A folder merely starting with the home folder's name is not inside it.
        assert_eq!(at("/home/adam/canopy").as_deref(), Some("/home/adam"));
        assert_eq!(at("/canopy").as_deref(), Some("/"));
        assert_eq!(folder_label(Path::new("canopy"), home), None);
        assert_eq!(
            folder_label(Path::new("/home/ada/src/x"), Path::new("")).as_deref(),
            Some("/home/ada/src")
        );
    }

    /// The tips teach the single keys where a command has one, not its long chord too.
    #[test]
    fn tips_show_the_single_key_of_a_command() {
        assert_eq!(key(Command::Fetch), "f");
        assert_eq!(key(Command::Cheatsheet), "?");
        assert_eq!(key(Command::Palette), hint(Command::Palette));
        for c in [
            Command::Fetch,
            Command::Pull,
            Command::Push,
            Command::History,
        ] {
            assert!(!key(c).contains('·'), "{c:?}: {}", key(c));
        }
    }

    /// The first-run tips end in "Got it", which dismisses them.
    #[test]
    fn got_it_dismisses_the_first_run_tips() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut actions = Vec::new();
        let run = |events: Vec<egui::Event>, actions: &mut Vec<Action>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| intro(ui, 600.0, actions),
            );
            out.textures_delta.clear();
            out
        };
        let out = run(Vec::new(), &mut actions);
        let got_it = out
            .shapes
            .iter()
            .find_map(|c| find_text(&c.shape, "Got it"))
            .expect("a Got it button");
        let at = got_it.center();
        let press = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        let release = vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }];
        let _ = run(press, &mut actions);
        let _ = run(release, &mut actions);
        assert!(
            matches!(actions.as_slice(), [Action::DismissIntro]),
            "{actions:?}"
        );
    }

    fn find_text(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
            egui::Shape::Vec(v) => v.iter().find_map(|s| find_text(s, text)),
            _ => None,
        }
    }
}
