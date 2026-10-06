//! The details pane under the graph: commit metadata, changed files and the selected file's diff.

use std::ops::Range;

use egui::{
    Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;

use canopy_core::{CommitDetails, DiffSource, FileEntry, short_id};

use crate::action::Action;
use crate::keymap::Command;
use crate::tab::{DetailsError, FileSelection, RepoTab};
use crate::ui::kit::avatar::{Avatar, AvatarSize, Person};
use crate::ui::kit::button::LinkButton;
use crate::ui::kit::chip::{ChangeBadge, DiffStat};
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::progress::Spinner;
use crate::ui::kit::row::{ListRow, RowKind, TreeRow, tree_children};
use crate::ui::kit::{Button, IconButton, Size, paint};
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
    let headed = header(ui, tab);
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
            let note = crate::avatars::explain_tab(settings, tab).map(PictureNote::from);
            author(ui, &d, tab, note.as_ref(), actions);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(short_id(d.oid))
                        .font(FontId::new(m.sha_size, egui::FontFamily::Monospace))
                        .color(p.accent),
                )
                .kit_tooltip(d.oid.to_string())
                .show();
                if IconButton::new(icon::COPY, "Copy SHA")
                    .size(Size::Xs)
                    .shortcut(Command::CopySha)
                    .show(ui)
                    .clicked()
                {
                    actions.push(Action::Copy(d.oid.to_string()));
                }
                ui.label(
                    RichText::new(widgets::absolute_date_format(
                        d.author.time,
                        settings.iso_dates,
                    ))
                    .size(m.meta_size)
                    .color(p.text_dim),
                )
                .kit_tooltip(widgets::full_date(d.author.time, d.author.offset_minutes))
                .show();
            });
            // Up to two whole file rows, the "N files changed" line above them (as tall as it
            // was last frame, at first one line of its text) and the gaps between them.
            let stats_key = ui.id().with("commit-file-stats-height");
            let stats_h = ui
                .data(|data| data.get_temp::<f32>(stats_key))
                .unwrap_or_else(|| text_row(ui, m.meta_size));
            let files_min = file_row_height() * crate::ui::count_f32(d.files.len().min(2))
                + stats_h
                + theme::XS * 2.0
                + ui.spacing().item_spacing.y * 3.0;
            // What is left for the body under the disclosure row, less the body's padding and
            // the gaps around it.
            let info_room = (ui.available_height()
                - files_min
                - RowKind::List.height()
                - theme::XS * 2.0
                - ui.spacing().item_spacing.y * 2.0)
                .max(0.0);
            let mut open = tab.history_layout.information_open;
            // The pane header already shows a summary that fits it; the block then starts with
            // the rest of the message.
            let summary = !headed;
            let first_line = if summary {
                theme::heading(m.headline_size)
            } else {
                FontId::proportional(m.meta_size)
            };
            let first_line = ui.fonts_mut(|f| f.row_height(&first_line));
            information(ui, &mut open, info_room, first_line, |ui| {
                // Metadata rows as tall as their text, like a list of facts.
                ui.spacing_mut().interact_size.y = 0.0;
                if message(ui, &d, summary) {
                    ui.add_space(theme::XS);
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(&d.author.email)
                            .size(m.small_size)
                            .color(p.text_dim),
                    )
                    .wrap(),
                );
                metadata(ui, &d, actions);
            });
            tab.history_layout.information_open = open;
            ui.add_space(theme::XS);
            let measured = file_stats(ui, &d);
            if (measured - stats_h).abs() > 0.5 {
                ui.data_mut(|data| data.insert_temp(stats_key, measured));
                ui.ctx().request_repaint();
            }
            ui.add_space(theme::XS);
            let list = egui::ScrollArea::vertical()
                .id_salt("commit-file-list")
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| files(ui, &d, tab));
            paint::scroll_fade(ui, &list, s.pane);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(s.pane))
        .show(ui, |ui| diff::show(ui, tab, actions));
}

/// Lines of a commit message's body a details pane that sizes itself keeps room for; a longer
/// body scrolls in its block.
const BODY_LINES: usize = 2;

/// The least height History gives the details pane when it sizes the pane itself: the pane header
/// and the commit column as [`show`] lays it out for `d` (any commit while it loads): the author
/// and SHA rows, the "Commit information" row and its body (the first lines of the message, the
/// email, the date, the committer and the parents), the changed-files line and the file rows it
/// keeps in view. So the commit's information shows at the pane's natural size in every density;
/// a pane made shorter by hand shuts the block and says why.
pub fn natural_height(ui: &Ui, d: Option<&CommitDetails>) -> f32 {
    let m = theme::metrics();
    let gap = theme::XS;
    let row = ui.spacing().interact_size.y;
    let author = row.max(m.avatar_detail);
    let sha = row.max(Size::Xs.height());
    let disclosure = RowKind::List.height() + theme::XS * 2.0 + gap * 2.0;
    let fact = text_row(ui, m.meta_size);
    // The email and the date always; the committer and the parents when the commit has them;
    // the body's first lines when the message has one.
    let (facts, body_lines, files) = d.map_or((3, 0, 2), |d| {
        let committer = d.committer.email != d.author.email || d.committer.name != d.author.name;
        let body = d
            .message
            .trim()
            .lines()
            .skip(1)
            .filter(|l| !l.trim().is_empty());
        (
            1 + usize::from(committer) + usize::from(!d.parents.is_empty()),
            body.take(BODY_LINES).count(),
            d.files.len().min(2),
        )
    });
    let body = if body_lines > 0 {
        crate::ui::count_f32(body_lines) * fact + theme::XS + gap
    } else {
        0.0
    };
    let information =
        text_row(ui, m.small_size) + crate::ui::count_f32(facts) * (fact + gap) + body;
    let files = file_row_height() * crate::ui::count_f32(files)
        + text_row(ui, m.meta_size)
        + theme::XS * 2.0
        + gap * 3.0;
    m.pane_header_height
        + m.detail_pad.y * 2.0
        + author
        + gap
        + sha
        + gap
        + disclosure
        + information
        + files
}

/// The height of one line of text at `size` (an icon row of the same size included).
fn text_row(ui: &Ui, size: f32) -> f32 {
    let m = theme::metrics();
    ui.fonts_mut(|f| {
        f.row_height(&FontId::proportional(size))
            .max(f.row_height(&FontId::proportional(m.icon_sm)))
    })
}

/// Pane navigation belongs to the commit, including when its files have not loaded: the commit's
/// summary and the Expand / Back to history button on one header row. Returns whether it showed
/// the whole summary (false in a compact window, which has no header, or when it was cut).
fn header(ui: &mut Ui, tab: &mut RepoTab) -> bool {
    if tab.history_layout.compact {
        return false;
    }
    let p = palette(ui.ctx());
    let m = theme::metrics();
    egui::Panel::top("commit-pane-header")
        .exact_size(m.pane_header_height)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let font = FontId::new(m.body_size, theme::semibold());
            let row = ui.max_rect();
            paint_header_strip(ui, row);
            let inner = Rect::from_min_max(
                pos2(row.left() + PANE_HEADER_PAD.0, row.top()),
                pos2(row.right() - PANE_HEADER_PAD.1, row.bottom()),
            );
            let mut buttons = ui.new_child(
                UiBuilder::new()
                    .max_rect(inner)
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
            let summary_rect = Rect::from_min_max(
                inner.min,
                pos2(toggle.rect.left() - theme::SM, inner.bottom()),
            );
            let mut summary_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(summary_rect)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            summary_ui.set_clip_rect(summary_rect.intersect(ui.clip_rect()));
            let summary = tab.details.as_ref().map_or("Commit details", |d| {
                d.message.lines().next().unwrap_or("").trim()
            });
            let whole = ui
                .fonts_mut(|f| f.layout_no_wrap(summary.to_owned(), font.clone(), p.text))
                .size()
                .x
                <= summary_rect.width();
            summary_ui
                .add(egui::Label::new(RichText::new(summary).font(font).color(p.text)).truncate())
                .kit_tooltip(summary)
                .show();
            if toggle.clicked() {
                tab.history_layout.expanded = !tab.history_layout.expanded;
                tab.history_layout.compact_details = true;
            }
            whole && tab.details.is_some()
        })
        .inner
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

/// The "Commit information" disclosure, one rule in every density: it keeps the user's choice
/// (open by default) and shows its body whenever `room` (what the changed files' reserved rows
/// leave) holds the body's `first_line`. The body sits flush with the author row, takes its
/// natural height up to that room and scrolls past it behind a faded edge, so no line is cut
/// through its glyphs. Without that room it stays shut and its tooltip says how to make room.
fn information(
    ui: &mut Ui,
    open: &mut bool,
    room: f32,
    first_line: f32,
    body: impl FnOnce(&mut Ui),
) {
    const LABEL: &str = "Commit information";
    let s = palette(ui.ctx()).s();
    let id = ui.id().with("commit-information");
    let fits = room >= first_line;
    let mut shown = *open && fits;
    let row = TreeRow::new("commit-information", 0)
        .kind(RowKind::List)
        .label(LABEL)
        .toggle_on_click(true)
        .enabled(fits)
        .open(&mut shown)
        .show(ui)
        .response;
    if fits {
        *open = shown;
    }
    // A disclosure is a collapsing header, not an item of a tree.
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, fits, LABEL));
    ui.ctx().accesskit_node_builder(row.id, |n| {
        n.clear_selected();
        n.set_expanded(shown);
    });
    if !fits {
        row.kit_tooltip("No room for it under the changed files")
            .detail("Make the details pane taller, or choose Expand")
            .show();
    }
    tree_children(ui, id, shown, 0, |ui| {
        ui.add_space(theme::XS);
        let out = egui::ScrollArea::vertical()
            .id_salt("commit-metadata")
            .max_height(room)
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, body);
        paint::scroll_fade(ui, &out, s.pane);
        ui.add_space(theme::XS);
    });
}

/// The commit message: its summary as a headline when `summary` (the pane header does not show
/// it whole), then the rest of the message. Returns whether it drew anything.
fn message(ui: &mut Ui, d: &CommitDetails, summary: bool) -> bool {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let mut lines = d.message.trim().splitn(2, '\n');
    let head = lines.next().unwrap_or("").trim();
    let body = lines.next().unwrap_or("").trim();
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

fn author(
    ui: &mut Ui,
    d: &CommitDetails,
    tab: &mut RepoTab,
    note: Option<&PictureNote>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal(|ui| {
        tab.request_avatar(d.oid, &d.author.email);
        let picture = tab
            .avatars
            .get(&d.author.email)
            .map(egui::TextureHandle::id);
        Avatar::new(Person::new(&d.author.name, &d.author.email).picture(picture))
            .size(AvatarSize::Detail)
            .id_salt(("commit-author", &d.author.email))
            .show(ui);
        ui.add(
            egui::Label::new(
                RichText::new(&d.author.name)
                    .font(FontId::new(m.body_size, theme::semibold()))
                    .color(p.text),
            )
            .truncate(),
        )
        .kit_tooltip(format!("{} <{}>", d.author.name, d.author.email))
        .show();
        if let Some(note) = note {
            picture_note(ui, note, actions);
        }
    });
}

/// The info mark after the author's name when pictures are kept away: faint at rest, dim on hover,
/// no fill; its tooltip gives the title and the cause, and a click opens the setting that fixes it.
fn picture_note(ui: &mut Ui, note: &PictureNote, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let m = theme::metrics();
    let side = Size::Xs.height();
    let sense = if note.fix.is_some() {
        egui::Sense::click()
    } else {
        egui::Sense::focusable_noninteractive()
    };
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(side), sense);
    let st = crate::ui::kit::Interact::of(&ctx, &response, true, None);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if st.focus > 0.0 {
            paint::focus_ring(
                painter,
                rect,
                CornerRadius::same(m.radius_sm),
                p.focus.gamma_multiply(st.focus),
                egui::Color32::TRANSPARENT,
            );
        }
        let color = p.text_faint.lerp_to_gamma(p.text_dim, st.hover);
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon::INFO,
            FontId::proportional(m.icon_sm),
            color,
        );
    }
    let title = note.title.clone();
    let response = response.on_hover_cursor(if note.fix.is_some() {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
    let response = response
        .kit_tooltip(note.title.clone())
        .detail(note.detail.clone())
        .show();
    if response.clicked()
        && let Some(fix) = &note.fix
    {
        actions.push(fix.clone());
    }
}

fn metadata(ui: &mut Ui, d: &CommitDetails, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    meta_row(
        ui,
        icon::CALENDAR_BLANK,
        &widgets::full_date(d.author.time, d.author.offset_minutes),
    );
    if d.committer.email != d.author.email || d.committer.name != d.author.name {
        meta_row(
            ui,
            icon::USER_SWITCH,
            &format!("Committed by {}", d.committer.name),
        );
    }
    if !d.parents.is_empty() {
        ui.horizontal_wrapped(|ui| {
            without_icons(
                ui.label(
                    RichText::new(icon::GIT_COMMIT)
                        .size(m.icon_sm)
                        .color(p.text_faint),
                ),
            );
            ui.label(
                RichText::new(if d.parents.len() > 1 {
                    "Parents"
                } else {
                    "Parent"
                })
                .size(m.meta_size)
                .color(p.text_faint),
            );
            for parent in &d.parents {
                if LinkButton::new(short_id(*parent))
                    .text_size(m.meta_size)
                    .show(ui)
                    .kit_tooltip(format!("Show commit {parent}"))
                    .show()
                    .clicked()
                {
                    actions.push(Action::Reveal(*parent));
                }
            }
        });
    }
}

/// The "N files changed" line and its line counts; returns the height it took.
fn file_stats(ui: &mut Ui, d: &CommitDetails) -> f32 {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let (adds, dels) = d
        .files
        .iter()
        .fold((0, 0), |(a, r), f| (a + f.additions, r + f.deletions));
    ui.scope(|ui| {
        // As tall as its text: the height of a control would only push the files down.
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} file{} changed",
                    d.files.len(),
                    if d.files.len() == 1 { "" } else { "s" }
                ))
                .size(m.meta_size)
                .color(p.text_dim),
            );
            if d.stats_known {
                DiffStat::new(adds, dels).show(ui);
            } else {
                // A partial clone that has not downloaded the file contents.
                ui.label(
                    RichText::new("line counts not downloaded")
                        .color(p.text_faint)
                        .size(m.small_size),
                )
                .kit_tooltip(
                    "This is a partial clone: the line counts and renamed files \
                 need file contents that were not downloaded. Open a file \
                 to download it.",
                )
                .show();
            }
        })
    })
    .response
    .rect
    .height()
}

fn files(ui: &mut Ui, d: &CommitDetails, tab: &mut RepoTab) {
    virtual_rows(ui, d.files.len(), file_row_height(), |ui, i| {
        let f = &d.files[i];
        let selected = tab.file.as_ref().is_some_and(|s| s.entry.path == f.path);
        let id = ui.id().with(("commit-file", &f.path, &f.old_path, f.kind));
        if file_list_row(ui, f, selected, id).clicked() {
            tab.select_file(FileSelection {
                source: DiffSource::Commit(d.oid),
                entry: f.clone(),
                conflicted: false,
            });
        }
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

fn meta_row(ui: &mut Ui, glyph: &str, text: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal(|ui| {
        without_icons(ui.label(RichText::new(glyph).size(m.icon_sm).color(p.text_faint)));
        ui.add(egui::Label::new(RichText::new(text).size(m.meta_size).color(p.text_dim)).wrap());
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
pub fn file_list_row(ui: &mut Ui, f: &FileEntry, selected: bool, id: egui::Id) -> egui::Response {
    file_list_row_inner(ui, f, selected, id, 0.0, None)
}

/// [`file_list_row`] with actions revealed at its right end while it is hovered, selected or
/// focused: `actions` adds them right to left (`IconButton::size(Size::Sm)`s, or a labelled
/// `Button::size(Size::Sm)`), and the row keeps `actions_width` for them
/// ([`icon_actions_width`] for icon buttons).
pub fn file_list_row_with<'a>(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
    id: egui::Id,
    actions_width: f32,
    actions: impl FnOnce(&mut Ui) + 'a,
) -> egui::Response {
    file_list_row_inner(ui, f, selected, id, actions_width, Some(Box::new(actions)))
}

/// Room a file row keeps at its right end for two small icon buttons, as `ListRow::actions`
/// reserves.
pub fn icon_actions_width() -> f32 {
    (theme::metrics().control_sm + 2.0) * 2.0
}

/// A file row's hover-revealed actions.
type RowActions<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

fn file_list_row_inner(
    ui: &mut Ui,
    f: &FileEntry,
    selected: bool,
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
    let tip = match &f.old_path {
        Some(old) => format!("{} → {}", old, f.path),
        None => f.path.clone(),
    };
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
        if let Some(g) = stats {
            right -= g.size().x;
            painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text);
            right -= ROW_GAP;
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
    ui.ctx()
        .accesskit_node_builder(response.id, |n| n.set_label(f.path.clone()));
    response
}

/// The status letter's square (concept 1's `.fs`, as `kit::chip::ChangeBadge`) and its letter.
const CHANGE_BADGE: f32 = 17.0;
const CHANGE_LETTER: f32 = 10.0;
/// Gap between a file row's badge, name, folder and counts (concept 1's `.frow`).
const ROW_GAP: f32 = 8.0;

/// `+n −n` as `kit::chip::DiffStat` paints it: mono in `success` and `danger` at 85%.
fn diffstat_galley(ui: &Ui, f: &FileEntry) -> std::sync::Arc<egui::Galley> {
    let p = palette(ui.ctx());
    let font = FontId::new(theme::metrics().sha_size, egui::FontFamily::Monospace);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c| egui::TextFormat::simple(font.clone(), c);
    job.append(&format!("+{}", f.additions), 0.0, fmt(p.success));
    job.append(
        &format!("−{}", f.deletions),
        6.0,
        fmt(p.danger.gamma_multiply(0.85)),
    );
    ui.painter().layout_job(job)
}

/// Paints `text` on one line from `x`, ellipsized before `right`; returns where it ends.
fn paint_line(
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
                drawn = file_list_row(ui, &entry(), false, egui::Id::new("row"))
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
            file_list_row(ui, &f, true, egui::Id::new("row"));
        });
        assert!(
            nodes.contains(&("src/views/history.rs".to_owned(), true)),
            "{nodes:?}"
        );
    }

    #[test]
    fn a_selected_file_row_reveals_its_actions() {
        let ctx = ctx(Density::Comfortable);
        let f = entry();
        let mut nodes = Vec::new();
        for _ in 0..2 {
            nodes = spoken(&ctx, |ui| {
                let width = super::icon_actions_width();
                file_list_row_with(ui, &f, true, egui::Id::new("row"), width, |ui| {
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
                |ui| super::author(ui, &d, &mut tab, note, &mut actions),
            );
            out.textures_delta.clear();
            labels = out
                .platform_output
                .accesskit_update
                .map(|u| {
                    u.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
        }
        (actions, labels)
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

    /// Override O2 (contracts/avatars.md, "Picture status UI" 1): when pictures are kept away the
    /// author line carries an info button named by the reason, and activating it opens the
    /// setting that fixes it; with nothing to explain the line is unchanged.
    #[test]
    fn the_author_line_explains_missing_pictures_and_opens_the_fix() {
        let (actions, labels) = author_line(None, &[]);
        assert!(actions.is_empty());
        assert!(
            !labels.iter().any(|l| l == "Profile pictures are off"),
            "{labels:?}"
        );

        let fix = Some(crate::action::Action::Settings);
        let (actions, labels) = author_line(Some(&note(fix)), &[egui::Key::Tab, egui::Key::Enter]);
        assert!(
            labels.iter().any(|l| l == "Profile pictures are off"),
            "{labels:?}"
        );
        assert!(
            matches!(actions.as_slice(), [crate::action::Action::Settings]),
            "{actions:?}"
        );

        let (actions, labels) = author_line(Some(&note(None)), &[egui::Key::Tab, egui::Key::Enter]);
        assert!(labels.iter().any(|l| l == "Profile pictures are off"));
        assert!(actions.is_empty(), "nothing to open: {actions:?}");
    }

    /// One frame-settled draw of the commit information block with `room` to spare: the body's
    /// rectangle (None while shut), the column's left edge, the height used and the user's
    /// choice afterwards.
    fn information_at(ctx: &egui::Context, room: f32) -> (Option<egui::Rect>, f32, f32, bool) {
        let mut body = None;
        let mut column = 0.0;
        let mut used = 0.0;
        let mut open = true;
        for _ in 0..3 {
            body = None;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                column = ui.max_rect().left();
                let line = ui.text_style_height(&egui::TextStyle::Body);
                super::information(ui, &mut open, room, line, |ui| {
                    body = Some(ui.label("fix(retry): tune defaults").rect);
                });
                used = ui.min_rect().height();
            });
            out.textures_delta.clear();
        }
        (body, column, used, open)
    }

    /// qa-a F1/F15, qa-b F2: one rule in every density. With room for its first line the block
    /// is open, shows that line whole and starts at the column's edge, not under the disclosure
    /// label; without it the block stays shut, the changed files keep their rows, and the user's
    /// choice is kept for a taller pane.
    #[test]
    fn commit_information_follows_one_rule_flush_with_the_column() {
        for density in [Density::Comfortable, Density::Dense] {
            let ctx = ctx(density);
            let (body, column, used, _) = information_at(&ctx, 80.0);
            let body = body.unwrap_or_else(|| panic!("{density:?}: open with room for a line"));
            assert!(
                (body.left() - column).abs() < 0.5,
                "{density:?}: body at {} against the column at {column}",
                body.left()
            );
            assert!(
                used >= super::RowKind::List.height() + body.height(),
                "{density:?}: the row and a whole first line show, got {used}"
            );
            let (body, _, used, open) = information_at(&ctx, 0.0);
            assert!(body.is_none(), "{density:?}: no room, no body");
            assert!(
                used <= super::RowKind::List.height() + 0.5,
                "{density:?}: only the row, got {used}"
            );
            assert!(open, "{density:?}: the choice survives a short pane");
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
