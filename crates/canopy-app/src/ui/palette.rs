//! The command palette (Ctrl+K): fuzzy search over every command, branch and recent repository.

use egui::{Align2, FontId, Id, Key, Order, RichText, Sense, vec2};
use egui_phosphor::regular as icon;

use super::kit::chip::Keycap;
use super::kit::overlay::{self, Layer};
use super::kit::row::{ListRow, RowKind};
use super::kit::{menu, paint};
use super::theme::{self, palette};

/// Space between two shortcut keycaps (alternatives).
pub(crate) const KEY_GAP: f32 = 6.0;

/// One entry the palette can run.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Icon glyph.
    pub icon: &'static str,
    /// Main text.
    pub label: String,
    /// Secondary text (a group such as "Branch" or a path).
    pub detail: String,
    /// Keyboard shortcut hint (from the keymap).
    pub shortcut: String,
}

fn category(entry: &Entry) -> Option<&str> {
    match entry.detail.as_str() {
        "Requests" | "History" | "Repository" | "Appearance" | "Canopy" | "Help" | "View"
        | "Branch" | "Stash" | "Changes" | "Remote" | "Remote branch" | "Tab" | "pull request"
        | "merge request" => Some(&entry.detail),
        _ => None,
    }
}

/// Scores how well `query` matches `text` as a subsequence. Consecutive and word-start matches
/// score higher. `None` when not all query characters occur in order.
#[cfg(test)]
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text_lower: Vec<char> = text.to_lowercase().chars().collect();
    score_lower(&query_chars(query), &text_lower)
}

/// The query's characters as matched: lowercase, without whitespace.
fn query_chars(query: &str) -> Vec<char> {
    query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// [`fuzzy_score`] for a non-empty query, over text that is already lowercased (the palette
/// lowercases its entries once, not on every keystroke).
fn score_lower(query: &[char], text_lower: &[char]) -> Option<i32> {
    let mut score = 0;
    let mut ti = 0;
    let mut last_match: Option<usize> = None;
    for &qc in query {
        let mut found = false;
        while ti < text_lower.len() {
            if text_lower[ti] == qc {
                score += 10;
                if last_match.is_some_and(|l| l + 1 == ti) {
                    score += 15;
                }
                if ti == 0 || matches!(text_lower[ti - 1], ' ' | '/' | '-' | '_' | ':' | '.') {
                    score += 20;
                }
                last_match = Some(ti);
                ti += 1;
                found = true;
                break;
            }
            ti += 1;
        }
        if !found {
            return None;
        }
    }
    Some(score - i32::try_from(text_lower.len()).unwrap_or(i32::MAX) / 4)
}

/// Entries shown at most.
const MAX_SHOWN: usize = 60;

/// Palette state. `T` is what an entry runs.
///
/// The entries are built by the caller only when the palette opens or what they are made from
/// changes ([`Self::needs_entries`]), and they are ranked only when the query changes, so an open
/// palette costs a frame almost nothing, however many branches the repository has.
#[derive(Debug)]
pub struct CommandPalette<T> {
    /// Shown this frame.
    pub open: bool,
    query: String,
    selected: usize,
    just_opened: bool,
    /// The entries and what each runs.
    items: Vec<(Entry, T)>,
    /// Lowercased label, and lowercased "detail label", of each entry.
    haystacks: Vec<(Vec<char>, Vec<char>)>,
    /// Identifies what `items` was built from; `None` until built for this opening.
    built_for: Option<u64>,
    /// Best matches (score, entry index) for `ranked_for`.
    ranked: Vec<(i32, usize)>,
    ranked_for: Option<String>,
}

impl<T> Default for CommandPalette<T> {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            just_opened: false,
            items: Vec::new(),
            haystacks: Vec::new(),
            built_for: None,
            ranked: Vec::new(),
            ranked_for: None,
        }
    }
}

/// The best `MAX_SHOWN` matches of `query` among `haystacks`, best first (ties in entry order).
fn rank(query: &str, haystacks: &[(Vec<char>, Vec<char>)]) -> Vec<(i32, usize)> {
    let q = query_chars(query);
    let mut ranked: Vec<(i32, usize)> = haystacks
        .iter()
        .enumerate()
        .filter_map(|(i, (label, both))| {
            if query.is_empty() {
                return Some((0, i));
            }
            let a = score_lower(&q, label);
            let b = score_lower(&q, both).map(|s| s - 5);
            a.max(b).map(|s| (s, i))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.truncate(MAX_SHOWN);
    ranked
}

impl<T: Clone> CommandPalette<T> {
    /// Opens with an empty query. The entries are built again for this opening.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.just_opened = true;
        self.built_for = None;
    }

    /// Whether the entries must be (re)built: the palette just opened, or `inputs` (a digest of
    /// what the entries are made from) changed.
    pub fn needs_entries(&self, inputs: u64) -> bool {
        self.built_for != Some(inputs)
    }

    /// Replaces the entries, built from `inputs`.
    pub fn set_entries(&mut self, inputs: u64, items: Vec<(Entry, T)>) {
        self.haystacks = items
            .iter()
            .map(|(e, _)| {
                let lower = |s: &str| s.to_lowercase().chars().collect::<Vec<char>>();
                (lower(&e.label), lower(&format!("{} {}", e.detail, e.label)))
            })
            .collect();
        self.items = items;
        self.built_for = Some(inputs);
        self.ranked_for = None;
    }

    /// Draws the palette; returns what the entry the user ran runs.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<T> {
        if !self.open {
            return None;
        }
        let p = palette(ctx);
        if self.ranked_for.as_deref() != Some(self.query.as_str()) {
            self.ranked = rank(&self.query, &self.haystacks);
            self.ranked_for = Some(self.query.clone());
        }
        let mut ranked = self.ranked.clone();
        if self.query.is_empty() {
            let mut groups = Vec::new();
            for (_, index) in &ranked {
                let group = category(&self.items[*index].0).unwrap_or("Recent repositories");
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            ranked.sort_by_key(|(_, index)| {
                let group = category(&self.items[*index].0).unwrap_or("Recent repositories");
                groups
                    .iter()
                    .position(|candidate| *candidate == group)
                    .unwrap_or(groups.len())
            });
        }

        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        if esc {
            self.open = false;
            return None;
        }
        if down && !ranked.is_empty() {
            self.selected = (self.selected + 1).min(ranked.len() - 1);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        self.selected = self.selected.min(ranked.len().saturating_sub(1));
        let mut chosen = if enter {
            ranked.get(self.selected).map(|r| r.1)
        } else {
            None
        };

        // Dim the window behind.
        let screen = ctx.content_rect();
        egui::Area::new(Id::new("palette-scrim"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let resp = ui.allocate_rect(screen, Sense::click());
                overlay::scrim(ui.painter(), screen, 1.0);
                if resp.clicked() {
                    self.open = false;
                }
            });

        let m = theme::metrics();
        let width = 620.0_f32.min(screen.width() - theme::XL * 2.0).max(0.0);
        let top = theme::XL.min(screen.height() * 0.08);
        egui::Area::new(Id::new("palette"))
            .order(Order::Tooltip)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(screen.center().x, screen.min.y + top))
            .show(ctx, |ui| {
                // The overlay surface (glass with Translucent surfaces on) under the card.
                overlay::floating(ui, Layer::Popover, egui::Margin::ZERO, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let edit = surface_search(
                        ui,
                        &mut self.query,
                        "Type a command, branch or repository…",
                        m.palette_input_height,
                    );
                    if self.just_opened {
                        edit.request_focus();
                        self.just_opened = false;
                    }
                    if edit.changed() {
                        self.selected = 0;
                    }
                    if ranked.is_empty() {
                        ui.add_space(theme::MD);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("No matches").color(p.text_faint))
                        });
                        ui.add_space(theme::MD);
                        return;
                    }
                    let list_margin = m.menu_pad;
                    let room = (screen.height() - top - theme::XL * 2.0 - m.palette_input_height)
                        .clamp(0.0, 400.0);
                    let list_h =
                        whole_rows_height(&ranked, &self.items, self.query.is_empty(), room);
                    let ground = overlay::ground(ctx, Layer::Popover);
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::same(crate::ui::points_i8(list_margin)))
                        .show(ui, |ui| {
                            overlay::scroll_body(ui, "palette-list", list_h, ground, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let mut previous_group = None;
                                for (row, (_, i)) in ranked.iter().enumerate() {
                                    let Some((e, _)) = self.items.get(*i) else {
                                        continue;
                                    };
                                    if self.query.is_empty() {
                                        let group = category(e).unwrap_or("Recent repositories");
                                        if previous_group != Some(group) {
                                            menu::heading(ui, group);
                                            previous_group = Some(group);
                                        }
                                    }
                                    let selected = row == self.selected;
                                    let resp = entry_row(ui, row, e, selected, ground);
                                    if resp.hovered()
                                        && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO)
                                    {
                                        self.selected = row;
                                    }
                                    if selected {
                                        ui.scroll_to_rect(resp.rect, None);
                                    }
                                    if resp.clicked() {
                                        chosen = Some(*i);
                                    }
                                }
                            });
                        });
                });
            });

        if chosen.is_some() {
            self.open = false;
        }
        chosen.and_then(|i| self.items.get(i).map(|(_, t)| t.clone()))
    }
}

/// The tallest list of whole entries (and their group headings, when grouped) that fits in
/// `room`, so the list never opens on a half-cut entry.
fn whole_rows_height<T>(
    ranked: &[(i32, usize)],
    items: &[(Entry, T)],
    grouped: bool,
    room: f32,
) -> f32 {
    let row = theme::metrics().palette_row_height;
    let mut height = 0.0;
    let mut previous_group = None;
    for (_, i) in ranked {
        let mut next = row;
        if grouped && let Some((e, _)) = items.get(*i) {
            let group = category(e).unwrap_or("Recent repositories");
            if previous_group != Some(group) {
                next += menu::HEADING_HEIGHT;
                previous_group = Some(group);
            }
        }
        if height + next > room {
            break;
        }
        height += next;
    }
    height.max(row.min(room))
}

/// Gap between a palette row's icon, label and detail, and before its keys.
const ROW_GAP: f32 = 10.0;

/// "The field that is the surface": the palette's and cheatsheet's search, with no ring and no
/// border of its own, only a seam under it. A magnifier leads; `height` tall, full width.
pub(crate) fn surface_search(
    ui: &mut egui::Ui,
    text: &mut String,
    placeholder: &str,
    height: f32,
) -> egui::Response {
    let p = palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let left = m.field_pad_x + m.icon_md + ROW_GAP;
    let glyph = ui.painter().layout_no_wrap(
        icon::MAGNIFYING_GLASS.to_owned(),
        FontId::proportional(m.icon_md),
        p.text_faint,
    );
    ui.painter().galley(
        egui::pos2(
            rect.left() + m.field_pad_x + theme::XS,
            rect.center().y - glyph.size().y / 2.0,
        ),
        glyph,
        p.text_faint,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    {
        let v = child.visuals_mut();
        v.weak_text_color = Some(p.text_faint);
        v.selection.bg_fill = p.selected();
    }
    let edit = egui::TextEdit::singleline(text)
        .hint_text(placeholder)
        .font(FontId::proportional(m.title_size))
        .text_color(p.text)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: crate::ui::points_i8(left + theme::XS),
            right: crate::ui::points_i8(m.field_pad_x),
            top: 0,
            bottom: 0,
        }))
        .min_size(vec2(rect.width(), height))
        .vertical_align(egui::Align::Center)
        .desired_width(rect.width())
        .show(&mut child)
        .response
        .response;
    paint::seam_h(ui.painter(), rect.x_range(), rect.bottom() - 0.5, s.border);
    edit
}

/// One palette entry as a list row: icon, label, detail, and its shortcut as keycaps. The
/// selection is the row's wash; the pointer moving over a row selects it.
fn entry_row(
    ui: &mut egui::Ui,
    row: usize,
    e: &Entry,
    selected: bool,
    ground: egui::Color32,
) -> egui::Response {
    let m = theme::metrics();
    let keys: Vec<Keycap> = e
        .shortcut
        .split('·')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(Keycap::new)
        .collect();
    let sizes: Vec<egui::Vec2> = keys.iter().map(|k| k.measure(ui)).collect();
    let keys_w: f32 = sizes.iter().map(|s| s.x + KEY_GAP).sum();
    let detail = (!e.detail.is_empty() && category(e).is_none()).then_some(e.detail.as_str());
    let r = ListRow::new(("palette-row", row))
        .kind(RowKind::Palette)
        .selected(selected)
        .ground(ground)
        .show_with(ui, |rc, painter| {
            let cy = rc.rect.center().y;
            let right = rc.content.right() - keys_w - ROW_GAP;
            let glyph =
                painter.layout_no_wrap(e.icon.to_owned(), FontId::proportional(m.icon_md), rc.icon);
            let mut x = rc.content.left();
            painter.galley(egui::pos2(x, cy - glyph.size().y / 2.0), glyph, rc.icon);
            x += m.icon_md + ROW_GAP;
            let clip = painter.with_clip_rect(egui::Rect::from_min_max(
                egui::pos2(x, rc.rect.top()),
                egui::pos2(right.max(x), rc.rect.bottom()),
            ));
            let label = clip.layout_no_wrap(
                e.label.clone(),
                FontId::new(m.body_size, theme::medium()),
                rc.text,
            );
            let label_w = label.size().x;
            clip.galley(egui::pos2(x, cy - label.size().y / 2.0), label, rc.text);
            if let Some(d) = detail {
                let g =
                    clip.layout_no_wrap(d.to_owned(), FontId::proportional(m.meta_size), rc.dim);
                clip.galley(
                    egui::pos2(x + label_w + ROW_GAP, cy - g.size().y / 2.0),
                    g,
                    rc.dim,
                );
            }
        });
    // The row paints its label itself; it is still the row's accessible name.
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &e.label)
    });
    ui.ctx().accesskit_node_builder(r.id, |n| {
        n.set_role(egui::accesskit::Role::ListItem);
        n.clear_toggled();
        n.set_selected(selected);
    });
    if ui.is_rect_visible(r.rect) {
        let mut x = r.rect.right() - theme::SM - keys_w + KEY_GAP;
        for (k, size) in keys.iter().zip(&sizes) {
            let at =
                egui::Rect::from_min_size(egui::pos2(x, r.rect.center().y - size.y / 2.0), *size);
            k.paint(ui, at);
            x += size.x + KEY_GAP;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, detail: &str) -> Entry {
        Entry {
            icon: "",
            label: label.into(),
            detail: detail.into(),
            shortcut: String::new(),
        }
    }

    fn items() -> Vec<(Entry, u32)> {
        vec![
            (entry("Push", "Remote"), 1),
            (entry("Pull", "Remote"), 2),
            (entry("New branch…", "Branch"), 3),
            (entry("Check out feature/login", "Branch"), 4),
            (entry("Unbind", "Help"), 5),
        ]
    }

    /// The ranking the palette used to compute every frame.
    fn reference(query: &str, entries: &[Entry]) -> Vec<(i32, usize)> {
        let mut ranked: Vec<(i32, usize)> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let a = fuzzy_score(query, &e.label);
                let b = fuzzy_score(query, &format!("{} {}", e.detail, e.label)).map(|s| s - 5);
                a.max(b).map(|s| (s, i))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked
    }

    #[test]
    fn cached_ranking_matches_the_direct_one() {
        let mut p = CommandPalette::default();
        p.set_entries(1, items());
        let entries: Vec<Entry> = items().into_iter().map(|(e, _)| e).collect();
        for q in ["", "  ", "p", "pu", "nb", "branch", "FEATURE", "zzz"] {
            assert_eq!(rank(q, &p.haystacks), reference(q, &entries), "query {q:?}");
        }
    }

    #[test]
    fn entries_are_rebuilt_on_opening_or_when_inputs_change() {
        let mut p: CommandPalette<u32> = CommandPalette::default();
        p.open();
        assert!(p.needs_entries(7));
        p.set_entries(7, items());
        assert!(!p.needs_entries(7), "kept while the inputs stay the same");
        assert!(p.needs_entries(8), "rebuilt when they change");
        p.open();
        assert!(p.needs_entries(7), "rebuilt on every opening");
    }

    #[test]
    fn enter_runs_the_best_match() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let mut p = CommandPalette::default();
        p.open();
        p.set_entries(1, items());
        let enter = egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut ran = None;
        ctx.run_ui(
            egui::RawInput {
                events: vec![enter],
                ..Default::default()
            },
            |ui| ran = p.show(ui.ctx()),
        )
        .drop_without_applying_deltas();
        assert_eq!(ran, Some(1), "the first entry for an empty query");
        assert!(!p.open);
    }

    #[test]
    fn fuzzy_prefers_word_starts_and_runs() {
        assert!(fuzzy_score("psh", "Push").is_some());
        assert!(fuzzy_score("xyz", "Push").is_none());
        let a = fuzzy_score("nb", "New branch").expect("match");
        let b = fuzzy_score("nb", "Unbind").expect("match");
        assert!(a > b, "{a} <= {b}");
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn the_list_opens_on_whole_entries_and_their_headings() {
        let row = theme::metrics().palette_row_height;
        let heading = menu::HEADING_HEIGHT;
        let items = items();
        let ranked: Vec<(i32, usize)> = (0..items.len()).map(|i| (0, i)).collect();
        // Grouped: Remote heading, Push, Pull, Branch heading, New branch, ...
        let room = heading + 2.0 * row + heading + row + row / 2.0;
        assert_eq!(
            whole_rows_height(&ranked, &items, true, room),
            heading + 2.0 * row + heading + row
        );
        // Ungrouped (a query): rows only.
        assert_eq!(
            whole_rows_height(&ranked, &items, false, 2.5 * row),
            2.0 * row
        );
    }

    #[test]
    fn a_long_list_fades_into_the_card_at_its_bottom() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut p = CommandPalette::default();
        p.open();
        let many: Vec<(Entry, u32)> = (0..40)
            .map(|i| (entry(&format!("Entry {i}"), "Branch"), i))
            .collect();
        p.set_entries(1, many);
        let mut meshes = 0;
        for _ in 0..4 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(1440.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let _ = p.show(ui.ctx());
                },
            );
            meshes = out
                .shapes
                .iter()
                .filter(|c| matches!(c.shape, egui::Shape::Mesh(_)))
                .count();
            out.drop_without_applying_deltas();
        }
        assert!(meshes > 0, "no fade");
    }
}
