//! The details pane under the graph: commit metadata, changed files and the selected file's diff.

use std::ops::Range;

use egui::{
    Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::forge::Forge;
use canopy_core::{CommitDetails, DiffSource, FileEntry, SignatureStatus, short_id};

use crate::action::Action;
use crate::tab::{DetailsError, FileSelection, RepoTab};
use crate::ui::kit::avatar::{Avatar, AvatarSize, Person};
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::DiffStat;
use crate::ui::kit::git::ChangeBadge;
use crate::ui::kit::overlay::{Popover, TooltipExt};
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{ListRow, RowKind, caps_galley};
use crate::ui::kit::{Button, IconButton, Size, menu, paint};
use crate::ui::theme::{self, Style, palette};
use crate::ui::{art, widgets};
use crate::views::diff;

/// Draws the details pane.
pub fn show(
    ui: &mut Ui,
    tab: &mut RepoTab,
    settings: &crate::settings::Settings,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let s = p.s();
    let m = theme::metrics();
    let headed = header(ui, tab, settings, actions);
    let Some(d) = tab.details.clone() else {
        // A failed load stays on screen with its reason, so selecting the same commit again is
        // not the only way out.
        if let Some(error) = tab
            .details_error
            .clone()
            .filter(|e| tab.selected_commit == Some(e.oid))
        {
            failed(ui, tab, &error);
            return;
        }
        ui.centered_and_justified(|ui| {
            if tab.selected_commit.is_some() {
                Spinner::new().label("commit details").active(true).show(ui);
            } else if tab.snap.as_ref().is_some_and(|s| !s.commits.is_empty()) {
                ui.label(RichText::new("Select a commit to see its changes").color(p.text_faint));
            }
        });
        return;
    };
    let pane_max = (ui.available_width() * 0.4).max(160.0);
    // A compact window keeps the changed files in view: the tighter inset of its controls.
    let pad = if tab.history_layout.compact {
        egui::Margin::same(crate::ui::points_i8(theme::SM))
    } else {
        egui::Margin::symmetric(
            crate::ui::points_i8(m.detail_pad.x),
            crate::ui::points_i8(m.detail_pad.y),
        )
    };
    egui::Panel::left("detail-meta")
        .resizable(true)
        .default_size(pane_max.min(m.detail_info_width))
        .min_size(pane_max.min(220.0))
        .max_size(pane_max.min(640.0))
        .frame(egui::Frame::new().fill(s.pane).inner_margin(pad))
        .show(ui, |ui| {
            ui.spacing_mut().scroll = theme::solid_scroll();
            ui.spacing_mut().item_spacing.y = if tab.history_layout.compact {
                theme::CONTROL_INSET
            } else {
                theme::XS
            };
            if !headed {
                let note = crate::avatars::explain_tab(settings, tab).map(PictureNote::from);
                commit_facts(ui, &d, tab, note.as_ref(), actions);
                ui.add_space(theme::SM);
            }
            // Up to two whole file rows, the "N files changed" title above them and the gaps
            // between them.
            let files_min = file_row_height() * crate::ui::count_f32(d.files.len().min(2))
                + m.section_header_height
                + theme::XS * 2.0
                + ui.spacing().item_spacing.y * 3.0;
            // What is left for the message, less the gaps around it.
            let room = (ui.available_height()
                - files_min
                - theme::XS * 2.0
                - ui.spacing().item_spacing.y * 2.0)
                .max(0.0);
            // The pane header owns the summary; compact windows render it here.
            let summary = !headed;
            if summary || !body_of(&d).is_empty() {
                let first_line = if summary {
                    theme::heading(m.headline_size)
                } else {
                    FontId::proportional(m.meta_size)
                };
                let first_line = ui.fonts_mut(|f| f.row_height(&first_line));
                ui.add_space(theme::XS);
                message_block(ui, room.max(first_line), |ui| message(ui, &d, summary));
            }
            ui.add_space(theme::XS);
            file_stats(ui, &d);
            ui.add_space(theme::XS);
            let list = egui::ScrollArea::vertical()
                .id_salt("commit-file-list")
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| files(ui, &d, tab, actions));
            paint::scroll_fade(ui, &list, s.pane);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(s.pane))
        .show(ui, |ui| diff::show(ui, tab, actions));
}

/// Lines of a commit message's body a details pane that sizes itself keeps room for; a longer
/// body scrolls in its block.
const BODY_LINES: usize = 2;

/// The header, description's first lines and two file rows kept visible by History's automatic
/// pane sizing. Long descriptions and additional files scroll independently.
pub fn natural_height(ui: &Ui, d: Option<&CommitDetails>) -> f32 {
    let m = theme::metrics();
    let (body_lines, files) = d.map_or((0, 2), |d| {
        (
            body_of(d)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(BODY_LINES)
                .count(),
            d.files.len().min(2),
        )
    });
    let body = if body_lines > 0 {
        crate::ui::count_f32(body_lines) * text_row(ui, m.meta_size) + theme::XS * 2.0
    } else {
        0.0
    };
    let header = d.map_or(m.pane_header_height, |d| header_height(ui, d));
    header
        + m.detail_pad.y * 2.0
        + body
        + m.section_header_height
        + file_row_height() * crate::ui::count_f32(files)
        + theme::XS * 2.0
        + ui.spacing().item_spacing.y * 3.0
}

fn header_height(ui: &Ui, d: &CommitDetails) -> f32 {
    ui.ctx()
        .data(|data| data.get_temp::<f32>(egui::Id::new(("commit-header-height", d.oid))))
        .unwrap_or(theme::metrics().pane_header_height)
}

/// The height of one line of text at `size` (an icon row of the same size included).
fn text_row(ui: &Ui, size: f32) -> f32 {
    let m = theme::metrics();
    ui.fonts_mut(|f| {
        f.row_height(&FontId::proportional(size))
            .max(f.row_height(&FontId::proportional(m.icon_sm)))
    })
}

/// The summary and navigation lead the header; identity and ids share its row when they fit,
/// otherwise they wrap on a second row. Compact windows move these facts into the card.
fn header(
    ui: &mut Ui,
    tab: &mut RepoTab,
    settings: &crate::settings::Settings,
    actions: &mut Vec<Action>,
) -> bool {
    if tab.history_layout.compact {
        return false;
    }
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let details = tab.details.clone();
    let height = details
        .as_deref()
        .map_or(m.pane_header_height, |d| header_height(ui, d));
    egui::Panel::top("commit-pane-header")
        .exact_size(height)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let row = ui.max_rect();
            paint_header_strip(ui, row);
            let top = Rect::from_min_max(
                pos2(row.left() + PANE_HEADER_PAD.0, row.top()),
                pos2(
                    row.right() - PANE_HEADER_PAD.1,
                    row.top() + m.pane_header_height,
                ),
            );
            let mut buttons = ui.new_child(
                UiBuilder::new()
                    .max_rect(top)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            let (glyph, label) = if tab.history_layout.expanded {
                (icon::ARROWS_IN_SIMPLE, "Back to history")
            } else {
                (icon::ARROWS_OUT_SIMPLE, "Expand")
            };
            let toggle = Button::new(label)
                .secondary()
                .size(Size::Sm)
                .icon(glyph)
                .id_salt("commit-pane-expand")
                .enabled(tab.selected_commit.is_some())
                .show(&mut buttons);
            let mut summary = Rect::from_min_max(
                top.min,
                pos2(
                    (toggle.rect.left() - theme::SM).max(top.left()),
                    top.bottom(),
                ),
            );
            let mut measured = m.pane_header_height;
            if let Some(d) = details.as_deref() {
                let width = facts_width(ui, d);
                let available = summary.width();
                let inline = facts_fit_inline(width, available, m.body_size * 4.0);
                let facts = if inline {
                    Rect::from_min_size(top.min, vec2(width, top.height()))
                } else {
                    Rect::from_min_max(
                        top.min,
                        pos2(summary.right(), row.bottom().max(top.bottom())),
                    )
                };
                let mut facts_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(facts)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                let note = crate::avatars::explain_tab(settings, tab).map(PictureNote::from);
                let used = commit_facts(&mut facts_ui, d, tab, note.as_ref(), actions);
                if inline {
                    summary.min.x = facts.right() + theme::SM;
                } else {
                    // The ids take the first row; the subject follows below them.
                    let below = used.height().max(m.pane_header_height) + theme::XS;
                    summary = Rect::from_min_max(
                        pos2(top.left(), top.top() + below),
                        pos2(top.right(), top.top() + below + m.pane_header_height),
                    );
                    measured = below + m.pane_header_height;
                }
                let id = egui::Id::new(("commit-header-height", d.oid));
                if (measured - height).abs() > 0.5 {
                    ui.ctx().data_mut(|data| data.insert_temp(id, measured));
                    ui.ctx().request_repaint();
                }
            }
            let summary_rect = summary;
            let mut summary_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(summary_rect)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            summary_ui.set_clip_rect(summary_rect.intersect(ui.clip_rect()));
            let summary = details.as_ref().map_or("Commit details", |d| {
                d.message.lines().next().unwrap_or("").trim()
            });
            let font = FontId::new(m.body_size, theme::semibold());
            let (_, label) =
                summary_ui.allocate_space(vec2(summary_rect.width(), summary_rect.height()));
            let galley = widgets::single_line_galley(
                &summary_ui,
                summary.to_owned(),
                font,
                p.text,
                label.width(),
            );
            summary_ui.painter().galley(
                pos2(label.left(), label.center().y - galley.size().y / 2.0),
                galley,
                p.text,
            );
            let band = summary_ui.interact(
                label,
                summary_ui.id().with("commit-summary"),
                egui::Sense::hover(),
            );
            band.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, summary));
            band.kit_tooltip(summary).show();
            if toggle.clicked() {
                tab.history_layout.expanded = !tab.history_layout.expanded;
                tab.history_layout.compact_details = true;
            }
        });
    details.is_some()
}

/// Whether the person and ids share the header's row with the subject: they do when the subject
/// keeps at least `min_summary` of the `available` width after them. Otherwise they take the row
/// and the subject the one below; the ids never shrink, only the subject truncates.
fn facts_fit_inline(facts: f32, available: f32, min_summary: f32) -> bool {
    facts + min_summary <= available
}

/// The width of the author, commit id and parents (they never truncate; the subject does).
fn facts_width(ui: &Ui, d: &CommitDetails) -> f32 {
    let m = theme::metrics();
    let ink = palette(ui.ctx()).text;
    ui.fonts_mut(|fonts| {
        let mut width = |text: String, size| {
            fonts
                .layout_no_wrap(text, FontId::new(size, theme::medium()), ink)
                .size()
                .x
        };
        let name = width(d.author.name.clone(), m.body_size).min(m.detail_info_width / 3.0);
        let ids = width("Commit".to_owned(), m.body_size)
            + width(short_id(d.oid), m.body_size)
            + m.button_pad_x_sm * 2.0
            + if d.parents.is_empty() {
                0.0
            } else {
                width(parents_label(d).to_owned(), m.body_size)
                    + d.parents
                        .iter()
                        .map(|oid| width(short_id(*oid), m.sha_size))
                        .sum::<f32>()
            };
        m.avatar_detail
            + name
            + m.icon_md
            + ids
            + theme::SM * crate::ui::count_f32(d.parents.len() + 5)
    })
}

fn commit_facts(
    ui: &mut Ui,
    d: &CommitDetails,
    tab: &mut RepoTab,
    note: Option<&PictureNote>,
    actions: &mut Vec<Action>,
) -> Rect {
    // The author and the ids share a row when both fit; otherwise the ids take their own row
    // under the author, wrapping between ids, so no id is ever cut. The widths are those of the
    // last frame, settled in a discarded pass when they change.
    let id = egui::Id::new(("commit-facts-widths", d.oid));
    let known: (f32, f32) = ui.ctx().data(|m| m.get_temp(id)).unwrap_or_default();
    let one_row = known.0 + theme::SM + known.1 <= ui.available_width() + 0.5;
    let mut measured = known;
    let rect = if one_row {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::SM;
            measured.0 = ui
                .horizontal(|ui| person(ui, d, tab, note, actions))
                .response
                .rect
                .width();
            measured.1 = ui.horizontal(|ui| commit_ids(ui, d, tab, actions)).inner;
        })
        .response
        .rect
    } else {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = theme::XS;
            measured.0 = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SM;
                    person(ui, d, tab, note, actions);
                })
                .response
                .rect
                .width();
            measured.1 = ui
                .horizontal_wrapped(|ui| commit_ids(ui, d, tab, actions))
                .inner;
        })
        .response
        .rect
    };
    if (measured.0 - known.0).abs() > 0.5 || (measured.1 - known.1).abs() > 0.5 {
        ui.ctx().data_mut(|m| m.insert_temp(id, measured));
        ui.ctx().request_discard("the commit facts changed width");
    }
    rect
}

/// The commit id (a button that copies it) and the parent ids (links to them); returns their
/// width on one line, whether or not they wrapped.
fn commit_ids(ui: &mut Ui, d: &CommitDetails, tab: &mut RepoTab, actions: &mut Vec<Action>) -> f32 {
    let m = theme::metrics();
    ui.spacing_mut().item_spacing.x = theme::SM;
    let mut parts: Vec<f32> = Vec::new();
    parts.push(fact_label(ui, "Commit").width());
    // The button's own padding, so the id sits as close to its label as the parents' do.
    ui.add_space(-m.button_pad_x_sm);
    let copy = Button::new(short_id(d.oid))
        .ghost()
        .plain_text()
        .size(Size::Xs)
        .show(ui)
        .kit_tooltip("Copy the commit id")
        .detail(d.oid.to_string())
        .show();
    parts.push(copy.rect.width());
    copy.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Copy commit {}", d.oid),
        )
    });
    if copy.clicked() {
        actions.push(Action::Copy(d.oid.to_string()));
    }
    if !d.parents.is_empty() {
        parts.push(fact_label(ui, parents_label(d)).width());
    }
    for (i, parent) in d.parents.iter().enumerate() {
        // The link sits in its own scope for a stable id, and a scope does not wrap by itself:
        // start the next row when the id would not fit whole.
        let text = short_id(*parent);
        let needs = widgets::text_width(ui, &text, &FontId::new(m.sha_size, theme::medium()));
        if ui.layout().main_wrap && needs > ui.available_size_before_wrap().x {
            ui.end_row();
        }
        let link = ui
            .push_id(("parent", i, parent), |ui| {
                LinkButton::new(text).text_size(m.sha_size).show(ui)
            })
            .inner
            .kit_tooltip("Show this parent commit")
            .detail(parent.to_string())
            .show();
        parts.push(link.rect.width());
        link.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Show parent {} commit {parent}", i + 1),
            )
        });
        if link.clicked() {
            tab.select_commit(*parent);
            tab.scroll_to_selected = true;
        }
    }
    // Their widths and the gaps between them, less the copy button's pulled-in padding.
    parts.iter().sum::<f32>() + theme::SM * crate::ui::count_f32(parts.len().saturating_sub(1))
        - m.button_pad_x_sm
}

/// The word before the parent ids: one label for all of them.
fn parents_label(d: &CommitDetails) -> &'static str {
    if d.parents.len() == 1 {
        "Parent"
    } else {
        "Parents"
    }
}

/// A muted word naming the value that follows it in the header's facts; returns its rect.
fn fact_label(ui: &mut Ui, text: &str) -> Rect {
    let m = theme::metrics();
    ui.label(
        RichText::new(text)
            .font(FontId::new(m.body_size, theme::medium()))
            .color(palette(ui.ctx()).text_dim),
    )
    .rect
}

/// Left and right insets of the detail header's content (concept 1's `.dh`).
const PANE_HEADER_PAD: (f32, f32) = (14.0, 10.0);

/// The detail header's ground: the chrome step over a seam in Standard; in Deeper the card's own
/// ground (the header sits inside the floating pane) over the same seam.
pub(super) fn paint_header_strip(ui: &Ui, rect: Rect) {
    let p = palette(ui.ctx());
    let s = p.s();
    let fill = if theme::style() == Style::Deeper {
        s.pane
    } else {
        s.panel
    };
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    // The header's own fill parts it from the pane in Standard; the seam follows the theme's edges.
    let seam = paint::edge_seam(p, s.border);
    paint::seam_line_h(
        ui.painter(),
        rect.x_range(),
        rect.bottom() - seam.width * 0.5,
        seam,
    );
}

/// The description above the file section, flush with the column: its natural height up to
/// `room` (what the changed files' reserved rows leave, at least one line), scrolling past it
/// behind a faded edge so no line is cut through its glyphs.
fn message_block(ui: &mut Ui, room: f32, body: impl FnOnce(&mut Ui) -> bool) {
    let s = palette(ui.ctx()).s();
    let out = egui::ScrollArea::vertical()
        .id_salt("commit-message")
        .max_height(room)
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // As tall as its text, like the rest of the column.
            ui.spacing_mut().interact_size.y = 0.0;
            body(ui)
        });
    paint::scroll_fade(ui, &out, s.pane);
}

/// The commit message after its summary line, trimmed.
fn body_of(d: &CommitDetails) -> &str {
    d.message
        .trim()
        .split_once('\n')
        .map_or("", |(_, rest)| rest.trim())
}

/// The commit message: its summary as a headline in compact mode, then the rest of the message. Returns whether it drew anything.
fn message(ui: &mut Ui, d: &CommitDetails, summary: bool) -> bool {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let head = d.message.trim().lines().next().unwrap_or("").trim();
    let body = body_of(d);
    if summary {
        ui.add(
            egui::Label::new(
                RichText::new(head)
                    .font(theme::heading(m.headline_size))
                    .color(p.text),
            )
            .wrap(),
        );
    }
    if !body.is_empty() {
        if summary {
            ui.add_space(theme::XS);
        }
        ui.add(egui::Label::new(RichText::new(body).color(p.text_dim).size(m.meta_size)).wrap());
    }
    summary || !body.is_empty()
}

/// Why the repository's authors show initials instead of pictures, for the author line: the view's
/// side of `avatars::Explanation` (contracts/avatars.md, "Picture status UI" 1).
#[derive(Debug, Clone)]
pub struct PictureNote {
    /// A few words: the tooltip's first line and the button's accessible name.
    pub title: String,
    /// The cause and what to do, under the title.
    pub detail: String,
    /// What a click does: open Settings where the cause can be changed.
    pub fix: Option<Action>,
}

impl From<crate::avatars::Explanation> for PictureNote {
    fn from(why: crate::avatars::Explanation) -> Self {
        use crate::avatars::Fix;
        use crate::views::settings_screen::Section;
        Self {
            title: why.title.to_owned(),
            detail: why.detail,
            fix: why.fix.map(|f| {
                Action::SettingsAt(match f {
                    Fix::Accounts => Section::Accounts,
                    Fix::Integrations => Section::Integrations,
                    Fix::Privacy => Section::Privacy,
                })
            }),
        }
    }
}

/// The commit's author, who opens the identity card; the committer joins the card and the
/// tooltip only when someone else committed.
fn person(
    ui: &mut Ui,
    d: &CommitDetails,
    tab: &mut RepoTab,
    note: Option<&PictureNote>,
    actions: &mut Vec<Action>,
) {
    let forge = tab.forge().map(|(_, forge)| forge.clone());
    tab.request_avatar(d.oid, &d.author.email);
    let picture = tab
        .avatars
        .get(&d.author.email)
        .map(egui::TextureHandle::id);
    Avatar::new(Person::new(&d.author.name, &d.author.email).picture(picture))
        .size(AvatarSize::Detail)
        .id_salt(("commit-author", &d.author.email))
        .show(ui);
    let name = LinkButton::new(&d.author.name)
        .max_text_width(theme::metrics().detail_info_width / 3.0)
        .show(ui);
    let mut tip = name.kit_tooltip(&d.author.name);
    if let Some(by) = committed_by(d) {
        tip = tip.detail(by);
    }
    let name = tip.show();
    name.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} (commit identity card)", d.author.name),
        )
    });
    let status = tab
        .signature
        .as_ref()
        .filter(|(oid, _)| *oid == d.oid)
        .map(|(_, status)| status.clone());
    if let Some(status) = &status {
        signature_mark(ui, status, false);
    }
    Popover::new("commit-identity", &name)
        .width(IDENTITY_CARD_WIDTH)
        .show(|ui| identity_card(ui, d, forge.as_ref(), status.as_ref(), note, actions));
}

/// The author's identity, the committer when different, signature verification and profile link,
/// plus the explanation and settings action when pictures are kept away.
fn identity_card(
    ui: &mut Ui,
    d: &CommitDetails,
    forge: Option<&Forge>,
    status: Option<&SignatureStatus>,
    note: Option<&PictureNote>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.spacing_mut().item_spacing.y = theme::XS;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&d.author.name)
                .font(FontId::new(m.body_size, theme::semibold()))
                .color(p.text),
        );
    });
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&d.author.email)
                .size(m.meta_size)
                .color(p.text_dim),
        );
        if IconButton::new(icon::COPY, "Copy email")
            .size(Size::Xs)
            .show(ui)
            .clicked()
        {
            actions.push(Action::Copy(d.author.email.clone()));
        }
    });
    if let Some(by) = committed_by(d) {
        ui.label(RichText::new(by).size(m.meta_size).color(p.text_dim));
    }
    if let Some(status) = status {
        signature_mark(ui, status, true);
    }
    if let Some(forge) = forge {
        let page = forge.author_page(&d.author.email);
        let name = if matches!(forge.host.as_str(), "github.com" | "gitlab.com") {
            forge.kind.name().to_owned()
        } else {
            forge.host.clone()
        };
        let label = if page.profile {
            format!("View profile on {name}")
        } else {
            format!("Find on {name}")
        };
        if LinkButton::new(label).url(&page.url).show(ui).clicked() {
            actions.push(Action::OpenUrl(page.url));
        }
    }
    if let Some(note) = note {
        ui.add_space(theme::XS);
        ui.label(RichText::new(&note.title).size(m.meta_size).color(p.text));
        ui.label(
            RichText::new(&note.detail)
                .size(m.meta_size)
                .color(p.text_dim),
        );
        if let Some(fix) = &note.fix
            && LinkButton::new("Open settings").show(ui).clicked()
        {
            actions.push(fix.clone());
        }
    }
}

/// Signature results use different glyphs, tones and spoken names; only Verified is trusted.
fn signature_mark(ui: &mut Ui, status: &SignatureStatus, text: bool) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (glyph, label, color, signer) = match status {
        SignatureStatus::Unsigned => return,
        SignatureStatus::Verified(signer) => (
            icon::SEAL_CHECK,
            "Signature verified",
            p.success,
            Some(signer),
        ),
        SignatureStatus::Invalid(signer) => {
            (icon::X_CIRCLE, "Signature invalid", p.danger, Some(signer))
        }
        SignatureStatus::Unavailable => (
            icon::QUESTION,
            "Signature verification unavailable",
            p.text_dim,
            None,
        ),
        SignatureStatus::Untrusted(signer) => (
            icon::WARNING_CIRCLE,
            "Signature signer not trusted",
            p.warning,
            Some(signer),
        ),
        SignatureStatus::Expired(signer) => (
            icon::WARNING_CIRCLE,
            "Signature expired or revoked",
            p.warning,
            Some(signer),
        ),
    };
    let content = if text {
        format!("{glyph} {label}")
    } else {
        glyph.to_owned()
    };
    let response = ui.add(
        egui::Label::new(
            RichText::new(content)
                .size(if text { m.meta_size } else { m.icon_md })
                .color(color),
        )
        .sense(egui::Sense::hover()),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    let detail = signer
        .map(|s| {
            [s.name.as_str(), s.key.as_str()]
                .into_iter()
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if text && !detail.is_empty() {
        ui.label(RichText::new(&detail).size(m.meta_size).color(p.text_dim));
    }
    let tip = response.kit_tooltip(label);
    if detail.is_empty() {
        tip.show();
    } else {
        tip.detail(detail).show();
    }
}

/// The width of the commit identity card.
const IDENTITY_CARD_WIDTH: f32 = 280.0;

/// The committer when someone other than the author committed the work.
fn committed_by(d: &CommitDetails) -> Option<String> {
    (d.committer.email != d.author.email || d.committer.name != d.author.name)
        .then(|| format!("Committed by {} <{}>", d.committer.name, d.committer.email))
}

/// The section's title, centred: "N files changed" in the section-header label style and its
/// line counts, in one band as tall as a section header. A partial clone that has not downloaded
/// the file contents says so in the band's tooltip, and in the band while the words fit.
fn file_stats(ui: &mut Ui, d: &CommitDetails) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (adds, dels) = d
        .files
        .iter()
        .fold((0, 0), |(a, r), f| (a + f.additions, r + f.deletions));
    let text = format!(
        "{} file{} changed",
        d.files.len(),
        if d.files.len() == 1 { "" } else { "s" }
    );
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), m.section_header_height));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let title = caps_galley(ui, &text, p.text_dim);
    let stat = DiffStat::new(adds, dels);
    let words = (!d.stats_known).then(|| {
        painter.layout_no_wrap(
            "line counts not downloaded".to_owned(),
            FontId::proportional(m.small_size),
            p.text_faint,
        )
    });
    let after = if d.stats_known {
        stat.size(&painter).x
    } else {
        words.as_ref().map_or(0.0, |w| w.size().x)
    };
    let title_w = title.size().x;
    let mut total = title_w + theme::SM + after;
    // Too narrow for both: the title alone, centred; the tooltip keeps the explanation.
    let show_after = total <= rect.width() || d.stats_known;
    if !show_after {
        total = title_w;
    }
    let left = (rect.center().x - total / 2.0).max(rect.left());
    let cy = rect.center().y;
    painter.galley(pos2(left, cy - title.size().y / 2.0), title, p.text_dim);
    let at = left + title_w + theme::SM;
    if d.stats_known {
        stat.paint(&painter, pos2(at, cy - stat.size(&painter).y / 2.0));
    } else if let (Some(words), true) = (words, show_after) {
        painter.galley(pos2(at, cy - words.size().y / 2.0), words, p.text_faint);
    }
    let band = ui.interact(
        rect,
        ui.id().with("commit-files-title"),
        egui::Sense::hover(),
    );
    let name = if d.stats_known {
        format!("{text}, {adds} added, {dels} removed")
    } else {
        format!("{text}, line counts not downloaded")
    };
    band.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &name));
    if !d.stats_known {
        // A partial clone: the line counts and renamed files need file contents that were not
        // downloaded.
        band.kit_tooltip(
            "This is a partial clone: the line counts and renamed files \
             need file contents that were not downloaded. Open a file \
             to download it.",
        )
        .show();
    }
}

fn files(ui: &mut Ui, d: &CommitDetails, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    virtual_rows(ui, d.files.len(), file_row_height(), |ui, i| {
        let f = &d.files[i];
        let selected = tab.file.as_ref().is_some_and(|s| s.entry.path == f.path);
        let id = ui.id().with(("commit-file", &f.path, &f.old_path, f.kind));
        let missing = tab
            .not_downloaded
            .contains(&(DiffSource::Commit(d.oid), f.path.clone()));
        let row = file_list_row(ui, f, selected, missing, id);
        if row.clicked() {
            tab.select_file(FileSelection {
                source: DiffSource::Commit(d.oid),
                entry: f.clone(),
                conflicted: false,
            });
        }
        menu::context(&row, |m| {
            if m.item(icon::CLOCK_COUNTER_CLOCKWISE, "File history…")
                .show(m.ui)
                .clicked()
            {
                actions.push(Action::FileHistory {
                    from: Some(d.oid),
                    path: f.path.clone(),
                });
            }
        });
    });
}

/// The details could not be loaded: git's reason and, when downloading can fix it (a partial
/// clone that has not fetched the commit), a Retry button.
fn failed(ui: &mut Ui, tab: &mut RepoTab, error: &DetailsError) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    // Git's message can be long and the pane short: scroll rather than clip the button.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(theme::XL);
            ui.vertical_centered(|ui| {
                ui.set_max_width(ui.available_width() - theme::XXL);
                art::show(ui, art::empty_error(), vec2(120.0, 90.0));
                ui.label(
                    RichText::new(format!("Could not load commit {}", short_id(error.oid)))
                        .font(theme::heading(m.subtitle_size)),
                );
                ui.label(
                    RichText::new(&error.message)
                        .size(m.meta_size)
                        .color(p.text_dim),
                );
                if error.can_download {
                    ui.add_space(theme::SM);
                    if Button::new("Retry")
                        .secondary()
                        .icon(icon::ARROWS_CLOCKWISE)
                        .show(ui)
                        .clicked()
                    {
                        tab.retry_details();
                    }
                }
                ui.add_space(theme::MD);
            });
        });
}

/// Keeps icon glyphs out of what a screen reader says for a label drawn as "<glyph>  text":
/// Phosphor's glyphs are private-use characters, read out as noise. A label that is only a glyph
/// is hidden from screen readers. Returns `response` for chaining.
pub fn without_icons(response: egui::Response) -> egui::Response {
    fn plain(text: &str) -> String {
        text.chars()
            .filter(|c| !('\u{E000}'..='\u{F8FF}').contains(c))
            .collect::<String>()
            .trim()
            .to_owned()
    }
    response.ctx.accesskit_node_builder(response.id, |node| {
        let label = node.label().map(plain);
        let value = node.value().map(plain);
        match label {
            Some(l) if l.is_empty() => node.clear_label(),
            Some(l) => node.set_label(l),
            None => {}
        }
        match value {
            Some(v) if v.is_empty() => node.clear_value(),
            Some(v) => node.set_value(v),
            None => {}
        }
        if node.label().is_none() && node.value().is_none() {
            node.set_hidden();
        }
    });
    response
}

/// Says in a disabled button's name that it is unavailable, and why in its description. The
/// AT-SPI adapter (`accesskit_atspi_common` 0.18) reports every button as enabled whatever its
/// disabled state, so on Linux a screen reader would otherwise offer a button that does nothing.
/// Returns `response` for chaining.
pub fn say_unavailable(response: egui::Response, reason: &str) -> egui::Response {
    response.ctx.accesskit_node_builder(response.id, |node| {
        if node.is_disabled()
            && let Some(label) = node.label().map(str::to_owned)
            && !label.ends_with(", unavailable")
        {
            node.set_label(format!("{label}, unavailable"));
            node.set_description(reason);
        }
    });
    response
}

/// The rows of a fixed-height list (`row_h` apart, no spacing) whose top is at `top`, that
/// intersect `min_y..max_y`.
pub fn visible_rows(n: usize, row_h: f32, top: f32, min_y: f32, max_y: f32) -> Range<usize> {
    if n == 0 || row_h <= 0.0 || max_y <= min_y {
        return 0..0;
    }
    let first = ((min_y - top) / row_h).floor().max(0.0);
    let end = ((max_y - top) / row_h).ceil().max(0.0);
    // Float to usize saturates; clamp to the list.
    let first = crate::ui::saturate_usize(first).min(n);
    first..crate::ui::saturate_usize(end).clamp(first, n)
}

/// Draws a list of `n` rows of height `row_h` at the cursor, calling `row` only for the rows in
/// view and reserving the space of the others, so a list of thousands costs only its visible
/// rows. Rows must allocate exactly `row_h` and give themselves ids that do not depend on their
/// position (they come and go while scrolling).
pub fn virtual_rows(ui: &mut Ui, n: usize, row_h: f32, mut row: impl FnMut(&mut Ui, usize)) {
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let clip = ui.clip_rect();
    let range = visible_rows(n, row_h, ui.cursor().min.y, clip.min.y, clip.max.y);
    let width = ui.available_width();
    if range.start > 0 {
        ui.allocate_space(vec2(width, crate::ui::count_f32(range.start) * row_h));
    }
    for i in range.clone() {
        row(ui, i);
    }
    if range.end < n {
        ui.allocate_space(vec2(width, crate::ui::count_f32(n - range.end) * row_h));
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

/// Height of [`file_list_row`] in the active density (`row_height`).
pub fn file_row_height() -> f32 {
    RowKind::File.height()
}

/// A changed-file row: the status letter, the name, its folder and the line counts, as a kit
/// [`ListRow`] (selection is the wash, with no side bar). `id` identifies the row across frames
/// (for hover and its context menu). Lists of these are [`file_row_height`] apart.
///
/// `not_downloaded` marks a file a partial clone could not download.
pub fn file_list_row(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    not_downloaded: impl Into<Missing>,
    id: egui::Id,
) -> egui::Response {
    file_list_row_inner(ui, f, selected, not_downloaded.into(), id, 0.0, None)
}

/// [`file_list_row`] with actions revealed at its right end while it is hovered, selected or
/// focused: `actions` adds them right to left (`IconButton::size(Size::Sm)`s, or a labelled
/// `Button::size(Size::Sm)`), and the row keeps `actions_width` for them
/// ([`icon_actions_width`] for icon buttons).
pub fn file_list_row_with<'a>(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    not_downloaded: impl Into<Missing>,
    id: egui::Id,
    actions_width: f32,
    actions: impl FnOnce(&mut Ui) + 'a,
) -> egui::Response {
    file_list_row_inner(
        ui,
        f,
        selected,
        not_downloaded.into(),
        id,
        actions_width,
        Some(Box::new(actions)),
    )
}

/// Room a file row keeps at its right end for two small icon buttons, as `ListRow::actions`
/// reserves.
pub fn icon_actions_width() -> f32 {
    (theme::metrics().control_sm + 2.0) * 2.0
}

/// Why a file row says its content is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    No,
    /// A partial clone could not download the file.
    PartialClone,
    /// The file is stored with Git LFS and its object is not downloaded: the working tree holds
    /// the pointer.
    LfsObject,
}

impl From<bool> for Missing {
    fn from(not_downloaded: bool) -> Self {
        if not_downloaded {
            Self::PartialClone
        } else {
            Self::No
        }
    }
}

/// A file row's hover-revealed actions.
type RowActions<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

fn file_list_row_inner(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    missing: Missing,
    id: egui::Id,
    actions_width: f32,
    actions: Option<RowActions<'_>>,
) -> egui::Response {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (name, dir) = widgets::split_path(&f.path);
    let (badge_fill, badge_ink) = ChangeBadge::new(f.kind).colors(p);
    let stats = (f.additions + f.deletions > 0).then(|| diffstat_galley(ui, f));
    let name_font = FontId::new(m.body_size, theme::medium());
    let dir_font = FontId::proportional(m.meta_size);
    let reserve = if actions.is_some() {
        actions_width
    } else {
        0.0
    };
    let mut tip = match &f.old_path {
        Some(old) => format!("{} → {}", old, f.path),
        None => f.path.clone(),
    };
    match missing {
        Missing::No => {}
        Missing::PartialClone => tip.push_str(" (not downloaded; open it to retry)"),
        Missing::LfsObject => {
            tip.push_str(" (stored with Git LFS, not downloaded: the file holds only its pointer)");
        }
    }
    if f.lfs && missing != Missing::LfsObject {
        tip.push_str(" (stored with Git LFS)");
    }
    let mut row = ListRow::new(id)
        .kind(RowKind::File)
        .selected(selected)
        .tooltip(tip);
    if let Some(add) = actions {
        row = row.actions(add);
    }
    let response = row.show_with(ui, |rc, painter| {
        let cy = rc.rect.center().y;
        let badge = Rect::from_min_size(
            pos2(rc.content.left(), cy - CHANGE_BADGE / 2.0),
            vec2(CHANGE_BADGE, CHANGE_BADGE),
        );
        painter.rect_filled(badge, CornerRadius::same(4), badge_fill);
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            f.kind.letter(),
            FontId::new(CHANGE_LETTER, egui::FontFamily::Monospace),
            badge_ink,
        );
        let mut right = rc.content.right() - reserve;
        if missing != Missing::No {
            let glyph = painter.layout_no_wrap(
                icon::CLOUD_SLASH.to_owned(),
                FontId::proportional(m.body_size),
                p.warning,
            );
            right -= glyph.size().x;
            painter.galley(pos2(right, cy - glyph.size().y / 2.0), glyph, p.warning);
            right -= ROW_GAP;
        }
        if let Some(g) = stats {
            right -= g.size().x;
            painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text);
            right -= ROW_GAP;
        }
        if f.lfs {
            right = paint_lfs_tag(painter, right, cy, p) - ROW_GAP;
        }
        let x = badge.right() + ROW_GAP;
        let end = paint_line(painter, x, right, cy, name, name_font, rc.text);
        if !dir.is_empty() {
            paint_line(
                painter,
                end + ROW_GAP,
                right,
                cy,
                dir,
                dir_font,
                p.text_faint,
            );
        }
    });
    // The row paints its own columns; screen readers get the file's path.
    let mut spoken = f.path.clone();
    if f.lfs {
        spoken.push_str(", stored with Git LFS");
    }
    if missing != Missing::No {
        spoken.push_str(", not downloaded");
    }
    ui.ctx()
        .accesskit_node_builder(response.id, |n| n.set_label(spoken));
    response
}

/// The text of the badge on a file stored with Git LFS.
pub(crate) const LFS_TAG: &str = "LFS";

/// Paints the "LFS" tag with its right edge at `right`; returns its left edge. The letters are
/// text, so the tag does not rest on colour alone.
fn paint_lfs_tag(painter: &egui::Painter, right: f32, cy: f32, p: &theme::Palette) -> f32 {
    let m = theme::metrics();
    let g = painter.layout_no_wrap(
        LFS_TAG.to_owned(),
        FontId::new(m.sha_size, egui::FontFamily::Monospace),
        p.text_dim,
    );
    let pad = theme::XS;
    let rect = Rect::from_min_max(
        pos2(right - g.size().x - 2.0 * pad, cy - g.size().y / 2.0 - 1.0),
        pos2(right, cy + g.size().y / 2.0 + 1.0),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        egui::Stroke::new(1.0, p.text_faint),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        pos2(rect.left() + pad, cy - g.size().y / 2.0),
        g,
        p.text_dim,
    );
    rect.left()
}

/// The status letter's square (concept 1's `.fs`, as `kit::git::ChangeBadge`) and its letter.
const CHANGE_BADGE: f32 = 17.0;
const CHANGE_LETTER: f32 = 10.0;
/// Gap between a file row's badge, name, folder and counts (concept 1's `.frow`).
pub(crate) const ROW_GAP: f32 = 8.0;

/// `+n −n` as `kit::chip::DiffStat` paints it: mono in the diff inks.
fn diffstat_galley(ui: &Ui, f: &FileEntry) -> std::sync::Arc<egui::Galley> {
    let p = palette(ui.ctx());
    let font = FontId::new(theme::metrics().sha_size, egui::FontFamily::Monospace);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c| egui::TextFormat::simple(font.clone(), c);
    job.append(&format!("+{}", f.additions), 0.0, fmt(p.diff_added));
    job.append(&format!("−{}", f.deletions), 6.0, fmt(p.diff_removed));
    ui.painter().layout_job(job)
}

/// Paints `text` on one line from `x`, ellipsized before `right`; returns where it ends.
pub(crate) fn paint_line(
    painter: &egui::Painter,
    x: f32,
    right: f32,
    cy: f32,
    text: &str,
    font: FontId,
    color: egui::Color32,
) -> f32 {
    if right <= x {
        return x;
    }
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, right - x);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let g = painter.layout_job(job);
    let end = x + g.size().x;
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
    end
}

#[cfg(test)]
mod tests {
    use canopy_core::{ChangeKind, FileEntry};
    use egui_phosphor::regular as icon;

    use super::{file_list_row, file_list_row_with, file_row_height, visible_rows};
    use crate::ui::kit::{IconButton, Size};
    use crate::ui::theme::{self, Appearance, Density, ThemeId};

    fn entry() -> FileEntry {
        FileEntry {
            path: "src/views/history.rs".into(),
            old_path: None,
            kind: ChangeKind::Modified,
            additions: 12,
            deletions: 3,
            lfs: false,
            old_mode: Some(0o100_644),
            new_mode: Some(0o100_644),
        }
    }

    fn ctx(density: Density) -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(
            &ctx,
            ThemeId::Midnight,
            Appearance {
                density,
                ..Appearance::STILL
            },
        );
        ctx
    }

    /// Labels and selection of the AccessKit nodes one frame of `f` produced.
    fn spoken(ctx: &egui::Context, mut f: impl FnMut(&mut egui::Ui)) -> Vec<(String, bool)> {
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(ui));
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| {
                        n.label()
                            .or_else(|| n.value())
                            .map(|l| (l.to_owned(), n.is_selected().unwrap_or(false)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn file_rows_follow_the_density() {
        for (density, height) in [(Density::Comfortable, 30.0), (Density::Dense, 24.0)] {
            let ctx = ctx(density);
            let mut drawn = 0.0;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(file_row_height(), height, "{density:?}");
                drawn = file_list_row(ui, &entry(), false, false, egui::Id::new("row"))
                    .rect
                    .height();
            });
            out.textures_delta.clear();
            assert_eq!(drawn, height, "{density:?}");
        }
    }

    #[test]
    fn a_file_row_speaks_its_path_and_selection() {
        let ctx = ctx(Density::Comfortable);
        let f = entry();
        let nodes = spoken(&ctx, |ui| {
            file_list_row(ui, &f, true, false, egui::Id::new("row"));
        });
        assert!(
            nodes.contains(&("src/views/history.rs".to_owned(), true)),
            "{nodes:?}"
        );
    }

    #[test]
    fn the_header_exposes_commit_parent_and_committer_controls() {
        let ctx = ctx(Density::Comfortable);
        let root = tempfile::tempdir().expect("tab directory");
        let mut tab = crate::tab::RepoTab::new(
            root.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        let sig = |name: &str, email: &str| canopy_core::Signature {
            name: name.into(),
            email: email.into(),
            time: 0,
            offset_minutes: 0,
        };
        let oid = canopy_core::Oid::from_bytes(&[1; 20]).expect("oid");
        let parent = canopy_core::Oid::from_bytes(&[2; 20]).expect("parent");
        tab.details = Some(std::sync::Arc::new(canopy_core::CommitDetails {
            oid,
            message: "A commit summary".into(),
            author: sig("Author", "author@example.com"),
            committer: sig("Committer", "committer@example.com"),
            parents: vec![parent],
            files: Vec::new(),
            stats_known: true,
        }));
        tab.selected_commit = Some(oid);
        let settings = crate::settings::Settings::default();
        let mut actions = Vec::new();
        ctx.enable_accesskit();
        let mut last = None;
        for width in [360.0, 600.0, 900.0, 1200.0] {
            for _ in 0..3 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 400.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        super::header(ui, &mut tab, &settings, &mut actions);
                    },
                );
                out.textures_delta.clear();
                last = out.platform_output.accesskit_update;
            }
            let nodes = &last.as_ref().expect("accessible tree").nodes;
            for name in [
                format!("Copy commit {oid}"),
                format!("Show parent 1 commit {parent}"),
                "Author (commit identity card)".into(),
            ] {
                let matching: Vec<_> = nodes
                    .iter()
                    .filter(|(_, node)| node.label() == Some(name.as_str()))
                    .collect();
                assert_eq!(matching.len(), 1, "unique {name} at {width}");
                let bounds = matching[0].1.bounds().expect("bounds");
                assert!(
                    bounds.x0 >= 0.0 && bounds.x1 <= f64::from(width),
                    "{name}: {bounds:?}"
                );
            }
        }
        // Reach the controls through ordinary Tab navigation; Enter performs their actual actions.
        for target in [
            format!("Copy commit {oid}"),
            format!("Show parent 1 commit {parent}"),
        ] {
            let mut reached = false;
            for _ in 0..20 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        events: crate::ui::kit::testkit::key(egui::Key::Tab),
                        ..Default::default()
                    },
                    |ui| {
                        super::header(ui, &mut tab, &settings, &mut actions);
                    },
                );
                out.textures_delta.clear();
                let update = out
                    .platform_output
                    .accesskit_update
                    .expect("accessible tree");
                if update
                    .nodes
                    .iter()
                    .any(|(id, node)| *id == update.focus && node.label() == Some(target.as_str()))
                {
                    reached = true;
                    let mut activated = ctx.run_ui(
                        egui::RawInput {
                            events: crate::ui::kit::testkit::key(egui::Key::Enter),
                            ..Default::default()
                        },
                        |ui| {
                            super::header(ui, &mut tab, &settings, &mut actions);
                        },
                    );
                    activated.textures_delta.clear();
                    break;
                }
            }
            assert!(reached, "keyboard reaches {target}");
        }
        assert!(actions.iter().any(|action| matches!(action, crate::action::Action::Copy(value) if value == &oid.to_string())));
        assert_eq!(tab.selected_commit, Some(parent));
    }

    fn details_with(message: &str, parents: usize, committer: &str) -> canopy_core::CommitDetails {
        let sig = |name: &str, email: &str| canopy_core::Signature {
            name: name.into(),
            email: email.into(),
            time: 0,
            offset_minutes: 0,
        };
        canopy_core::CommitDetails {
            oid: canopy_core::Oid::from_bytes(&[1; 20]).expect("oid"),
            message: message.into(),
            author: sig("Author", "author@example.com"),
            committer: sig(committer, "committer@example.com"),
            parents: (0..parents)
                .map(|i| canopy_core::Oid::from_bytes(&[2 + u8::try_from(i).expect("small"); 20]))
                .collect::<Result<_, _>>()
                .expect("parents"),
            files: Vec::new(),
            stats_known: true,
        }
    }

    #[test]
    fn the_header_width_follows_the_ids_never_the_subject() {
        let ctx = ctx(Density::Comfortable);
        let mut widths = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let long = "A very long subject line that would never fit beside the ids ".repeat(8);
            let one = super::facts_width(ui, &details_with("Short", 1, "Author"));
            let long_one = super::facts_width(ui, &details_with(&long, 1, "Author"));
            let merge = super::facts_width(ui, &details_with("Merge", 3, "Author"));
            // The committer is not shown in the row, so only the author sets the name's width.
            let other = super::facts_width(ui, &details_with("Short", 1, "Someone Else Entirely"));
            widths = vec![one, long_one, merge, other];
        });
        out.textures_delta.clear();
        let [one, long_one, merge, other] = widths[..] else {
            panic!("measured four widths");
        };
        assert!(
            (one - long_one).abs() < 0.01,
            "a long subject adds no width"
        );
        assert!((one - other).abs() < 0.01, "the committer adds no width");
        assert!(merge > one, "every parent adds width: {merge} vs {one}");
    }

    #[test]
    fn the_files_title_speaks_its_counts_or_that_they_are_missing() {
        let ctx = ctx(Density::Comfortable);
        for (known, files, expected) in [
            (true, 1, "1 file changed, 12 added, 3 removed"),
            (true, 3, "3 files changed, 36 added, 9 removed"),
            (false, 3, "3 files changed, line counts not downloaded"),
        ] {
            let mut d = details_with("Subject", 1, "Author");
            d.stats_known = known;
            d.files = vec![entry(); files];
            let names = spoken(&ctx, |ui| super::file_stats(ui, &d));
            assert!(
                names.iter().any(|(name, _)| name == expected),
                "{expected}: {names:?}"
            );
        }
    }

    #[test]
    fn several_parents_share_one_label_and_each_is_a_link() {
        let ctx = ctx(Density::Comfortable);
        let root = tempfile::tempdir().expect("tab directory");
        for (parents, label) in [(1, "Parent"), (2, "Parents"), (3, "Parents")] {
            let mut tab = crate::tab::RepoTab::new(
                root.path().into(),
                &ctx,
                100,
                crate::settings::DiffPrefs::default(),
            );
            let d = details_with("Merge", parents, "Author");
            let mut actions = Vec::new();
            let names = spoken(&ctx, |ui| {
                super::commit_facts(ui, &d, &mut tab, None, &mut actions);
            });
            let count = |wanted: &str| names.iter().filter(|(name, _)| name == wanted).count();
            assert_eq!(count(label), 1, "{parents} parents: {names:?}");
            assert_eq!(count("Commit"), 1, "{names:?}");
            for i in 1..=parents {
                let prefix = format!("Show parent {i} commit ");
                assert_eq!(
                    names.iter().filter(|(n, _)| n.starts_with(&prefix)).count(),
                    1,
                    "{names:?}"
                );
            }
        }
    }

    /// In a pane too narrow for the author and the ids on one row, the ids move under the author
    /// and wrap between ids: none is cut at the pane's edge (audit F4).
    #[test]
    fn narrow_commit_facts_wrap_the_ids_instead_of_cutting_them() {
        let ctx = ctx(Density::Comfortable);
        ctx.enable_accesskit();
        let root = tempfile::tempdir().expect("tab directory");
        let mut tab = crate::tab::RepoTab::new(
            root.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        let d = details_with("Merge", 3, "Author");
        let mut actions = Vec::new();
        for width in [200.0_f32, 300.0, 420.0] {
            let mut last = None;
            for _ in 0..3 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 400.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        super::commit_facts(ui, &d, &mut tab, None, &mut actions);
                    },
                );
                out.textures_delta.clear();
                last = out.platform_output.accesskit_update;
            }
            let nodes = &last.as_ref().expect("accessible tree").nodes;
            let ids: Vec<_> = nodes
                .iter()
                .filter_map(|(_, n)| Some((n.label()?, n.bounds()?)))
                .filter(|(l, _)| l.starts_with("Copy commit") || l.starts_with("Show parent"))
                .collect();
            assert_eq!(ids.len(), 4, "{width}: {ids:?}");
            for (name, at) in ids {
                assert!(
                    at.x0 >= 0.0 && at.x1 <= f64::from(width),
                    "{width}: {name} at {at:?}"
                );
            }
        }
    }

    #[test]
    fn the_ids_share_the_row_only_while_the_subject_keeps_room() {
        let (one, merge) = (300.0, 480.0);
        let room = 120.0;
        // A wide header keeps everything on one row, merge commit included.
        assert!(super::facts_fit_inline(merge, 900.0, room));
        // As it narrows the merge commit's ids wrap below first; a single parent still fits.
        assert!(super::facts_fit_inline(one, 440.0, room));
        assert!(!super::facts_fit_inline(merge, 440.0, room));
        // The subject keeps its minimum room, and not a point less.
        assert!(super::facts_fit_inline(one, one + room, room));
        assert!(!super::facts_fit_inline(one, one + room - 1.0, room));
    }

    #[test]
    fn a_selected_file_row_reveals_its_actions() {
        let ctx = ctx(Density::Comfortable);
        let f = entry();
        let mut nodes = Vec::new();
        for _ in 0..2 {
            nodes = spoken(&ctx, |ui| {
                let width = super::icon_actions_width();
                file_list_row_with(ui, &f, true, false, egui::Id::new("row"), width, |ui| {
                    IconButton::new(icon::PLUS, "Stage file")
                        .size(Size::Sm)
                        .show(ui);
                });
            });
        }
        assert!(
            nodes.iter().any(|(l, _)| l == "Stage file"),
            "the action is shown on a selected row: {nodes:?}"
        );
        assert!(nodes.iter().any(|(l, _)| l == "src/views/history.rs"));
    }

    /// The author line of a commit by `Author`, with `note` after the name; the actions it asked
    /// for and the AccessKit labels of the last frame. `keys` are pressed one per frame after
    /// the first.
    fn author_line(
        note: Option<&super::PictureNote>,
        keys: &[egui::Key],
        status: Option<canopy_core::SignatureStatus>,
    ) -> (Vec<crate::action::Action>, Vec<String>) {
        let ctx = ctx(Density::Comfortable);
        ctx.enable_accesskit();
        let dir = tempfile::tempdir().expect("temporary tab");
        let mut tab = crate::tab::RepoTab::new(
            dir.path().into(),
            &ctx,
            100,
            crate::settings::DiffPrefs::default(),
        );
        let author = canopy_core::Signature {
            name: "Author".into(),
            email: "author@example.com".into(),
            time: 0,
            offset_minutes: 0,
        };
        let d = canopy_core::CommitDetails {
            oid: canopy_core::Oid::from_bytes(&[1; 20]).expect("fixture oid"),
            message: "Selected commit".into(),
            author: author.clone(),
            committer: author,
            parents: Vec::new(),
            files: Vec::new(),
            stats_known: true,
        };
        tab.signature = status.map(|status| (d.oid, status));
        let mut actions = Vec::new();
        let mut labels = Vec::new();
        let frames = std::iter::once(None).chain(keys.iter().copied().map(Some));
        for key in frames {
            let events = key.map_or_else(Vec::new, |key| {
                [true, false]
                    .map(|pressed| egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .to_vec()
            });
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.horizontal(|ui| super::person(ui, &d, &mut tab, note, &mut actions));
                },
            );
            out.textures_delta.clear();
            labels = out
                .platform_output
                .accesskit_update
                .map(|u| {
                    u.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label().or(n.value()).map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
        }
        (actions, labels)
    }

    #[test]
    fn signature_badges_distinguish_verification_outcomes() {
        use canopy_core::{SignatureStatus, Signer};
        for (status, expected) in [
            (
                SignatureStatus::Verified(Signer::default()),
                "Signature verified",
            ),
            (
                SignatureStatus::Invalid(Signer::default()),
                "Signature invalid",
            ),
            (
                SignatureStatus::Unavailable,
                "Signature verification unavailable",
            ),
            (
                SignatureStatus::Untrusted(Signer::default()),
                "Signature signer not trusted",
            ),
            (
                SignatureStatus::Expired(Signer::default()),
                "Signature expired or revoked",
            ),
        ] {
            let (_, labels) = author_line(None, &[], Some(status));
            assert!(
                labels.iter().any(|label| label == expected),
                "{expected}: {labels:?}"
            );
        }
        let (_, labels) = author_line(None, &[], Some(SignatureStatus::Unsigned));
        assert!(!labels.iter().any(|label| label.starts_with("Signature")));
    }

    fn note(fix: Option<crate::action::Action>) -> super::PictureNote {
        super::PictureNote {
            title: "Profile pictures are off".into(),
            detail: "Turn on Download profile pictures in Settings → Privacy.".into(),
            fix,
        }
    }

    /// Each fix `avatars::explain` names opens its own Settings section.
    #[test]
    fn a_picture_explanation_opens_the_section_that_fixes_it() {
        use crate::action::Action;
        use crate::avatars::{Explanation, Fix};
        use crate::views::settings_screen::Section;
        let open = |fix| {
            super::PictureNote::from(Explanation {
                title: "t",
                detail: "d".into(),
                fix,
            })
            .fix
        };
        assert!(matches!(
            open(Some(Fix::Accounts)),
            Some(Action::SettingsAt(Section::Accounts))
        ));
        assert!(matches!(
            open(Some(Fix::Integrations)),
            Some(Action::SettingsAt(Section::Integrations))
        ));
        assert!(matches!(
            open(Some(Fix::Privacy)),
            Some(Action::SettingsAt(Section::Privacy))
        ));
        assert!(open(None).is_none());
    }

    /// The author line has no info button: the name is a link that opens the author card, and the
    /// setting that fixes missing pictures is a link in that card.
    #[test]
    fn the_author_card_opens_the_fix_for_missing_pictures() {
        let fix = Some(crate::action::Action::Settings);
        let (_, labels) = author_line(Some(&note(fix.clone())), &[], None);
        assert!(
            labels.iter().any(|l| l == "Author (commit identity card)"),
            "{labels:?}"
        );
        assert!(!labels.iter().any(|l| l == "Profile pictures are off"));
        assert!(!labels.iter().any(|l| l == "Open settings"), "{labels:?}");

        // Tab to the name, Enter opens the card, a frame later it is drawn.
        let open = [egui::Key::Tab, egui::Key::Enter, egui::Key::F13];
        let (_, labels) = author_line(Some(&note(fix)), &open, None);
        assert!(labels.iter().any(|l| l == "Copy email"), "{labels:?}");
        assert!(labels.iter().any(|l| l == "Open settings"), "{labels:?}");

        let (_, labels) = author_line(Some(&note(None)), &open, None);
        assert!(!labels.iter().any(|l| l == "Open settings"), "{labels:?}");
        let (_, labels) = author_line(None, &open, None);
        assert!(!labels.iter().any(|l| l == "Open settings"), "{labels:?}");
    }

    /// One frame-settled draw of the message block with `room`: the first line's rectangle, the
    /// column's left edge and the height used.
    fn message_at(ctx: &egui::Context, room: f32) -> (egui::Rect, f32, f32) {
        let mut first = egui::Rect::NOTHING;
        let mut column = 0.0;
        let mut used = 0.0;
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                column = ui.max_rect().left();
                super::message_block(ui, room, |ui| {
                    first = ui.label("fix(retry): tune defaults").rect;
                    for _ in 0..20 {
                        ui.label("more of the body");
                    }
                    true
                });
                used = ui.min_rect().height();
            });
            out.textures_delta.clear();
        }
        (first, column, used)
    }

    /// The message sits flush with the column, with no disclosure row above it, and takes no
    /// more than its room, scrolling past it.
    #[test]
    fn the_message_shows_flush_and_within_its_room() {
        for density in [Density::Comfortable, Density::Dense] {
            let ctx = ctx(density);
            let (first, column, used) = message_at(&ctx, 80.0);
            assert!(
                (first.left() - column).abs() < 0.5,
                "{density:?}: message at {} against the column at {column}",
                first.left()
            );
            assert!(
                first.top() < 1.0,
                "{density:?}: nothing above it, at {}",
                first.top()
            );
            assert!(used <= 80.5, "{density:?}: within its room, got {used}");
        }
    }

    #[test]
    fn visible_rows_cover_the_viewport_only() {
        // Ten rows of 10 starting at y 100.
        assert_eq!(
            visible_rows(10, 10.0, 100.0, 0.0, 50.0),
            0..0,
            "above the list"
        );
        assert_eq!(visible_rows(10, 10.0, 100.0, 0.0, 115.0), 0..2);
        assert_eq!(visible_rows(10, 10.0, 100.0, 125.0, 145.0), 2..5);
        assert_eq!(visible_rows(10, 10.0, 100.0, 190.0, 400.0), 9..10);
        assert_eq!(
            visible_rows(10, 10.0, 100.0, 300.0, 400.0),
            10..10,
            "below the list"
        );
        assert_eq!(visible_rows(0, 10.0, 0.0, 0.0, 100.0), 0..0);
        // A huge list costs only the rows in view.
        let r = visible_rows(1_000_000, 26.0, 0.0, 5_000_000.0, 5_000_900.0);
        assert!(r.len() <= 36, "{r:?}");
    }
}
