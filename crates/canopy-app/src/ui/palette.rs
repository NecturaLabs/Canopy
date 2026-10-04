//! The command palette (Ctrl+K): fuzzy search over every command, branch and recent repository.

use egui::{Align2, CornerRadius, FontId, Id, Key, Order, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;

use super::theme::{self, Palette, palette};
use super::widgets;

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
        let ranked = self.ranked.clone();

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
                ui.painter()
                    .rect_filled(screen, CornerRadius::ZERO, p.backdrop());
                if resp.clicked() {
                    self.open = false;
                }
            });

        let width = 620.0_f32.min(screen.width() - theme::XL * 2.0).max(0.0);
        let top = theme::XL.min(screen.height() * 0.08);
        egui::Area::new(Id::new("palette"))
            .order(Order::Tooltip)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(screen.center().x, screen.min.y + top))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.elevated)
                    .stroke(Stroke::new(1.0, Palette::tint(p.accent, 120)))
                    .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(icon::MAGNIFYING_GLASS)
                                    .size(theme::ICON_MD)
                                    .color(p.accent),
                            );
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .hint_text("Type a command, branch or repository…")
                                    .font(FontId::proportional(theme::TITLE_SIZE))
                                    .frame(egui::Frame::NONE)
                                    .desired_width(f32::INFINITY),
                            );
                            if self.just_opened {
                                edit.request_focus();
                                self.just_opened = false;
                            }
                            if edit.changed() {
                                self.selected = 0;
                            }
                        });
                        ui.add_space(6.0);
                        ui.painter().hline(
                            ui.min_rect().x_range(),
                            ui.cursor().min.y,
                            Stroke::new(1.0, p.border),
                        );
                        ui.add_space(6.0);
                        if ranked.is_empty() {
                            ui.add_space(10.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("No matches").color(p.text_faint))
                            });
                            ui.add_space(10.0);
                            return;
                        }
                        // Whole rows only, so the list never ends in a half-cut entry.
                        let pitch = 34.0 + ui.spacing().item_spacing.y;
                        let room =
                            (screen.height() - top - theme::XL * 2.0 - 60.0).clamp(0.0, 380.0);
                        let list_h = ((room + ui.spacing().item_spacing.y) / pitch)
                            .floor()
                            .max(1.0)
                            * pitch
                            - ui.spacing().item_spacing.y;
                        // Both bounds: the area's own size comes from its last frame and would
                        // otherwise cap the list part-way through a row.
                        let list = egui::ScrollArea::vertical()
                            .max_height(list_h)
                            .min_scrolled_height(list_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (row, (_, i)) in ranked.iter().enumerate() {
                                    let Some((e, _)) = self.items.get(*i) else {
                                        continue;
                                    };
                                    let selected = row == self.selected;
                                    let (rect, resp) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), 34.0),
                                        Sense::click(),
                                    );
                                    resp.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::SelectableLabel,
                                            true,
                                            selected,
                                            &e.label,
                                        )
                                    });
                                    if resp.hovered()
                                        && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO)
                                    {
                                        self.selected = row;
                                    }
                                    if selected {
                                        ui.painter().rect_filled(
                                            rect,
                                            CornerRadius::same(theme::RADIUS),
                                            Palette::tint(p.accent, if p.dark { 55 } else { 35 }),
                                        );
                                        ui.painter().rect_filled(
                                            egui::Rect::from_min_size(
                                                rect.min + vec2(0.0, 8.0),
                                                vec2(3.0, 18.0),
                                            ),
                                            CornerRadius::same(2),
                                            p.accent,
                                        );
                                        ui.scroll_to_rect(rect, None);
                                    }
                                    let c = rect.left_center();
                                    ui.painter().text(
                                        c + vec2(16.0, 0.0),
                                        Align2::CENTER_CENTER,
                                        e.icon,
                                        FontId::proportional(theme::ICON_MD),
                                        if selected { p.accent } else { p.text_dim },
                                    );
                                    let shortcut_width = ui
                                        .painter()
                                        .layout_no_wrap(
                                            e.shortcut.clone(),
                                            FontId::monospace(theme::MONO_SMALL_SIZE),
                                            p.text_faint,
                                        )
                                        .size()
                                        .x;
                                    let label_clip = egui::Rect::from_min_max(
                                        rect.min + vec2(30.0, 0.0),
                                        rect.max - vec2(shortcut_width + theme::XL, 0.0),
                                    );
                                    let painter = ui.painter().with_clip_rect(label_clip);
                                    let label = painter.text(
                                        c + vec2(34.0, 0.0),
                                        Align2::LEFT_CENTER,
                                        &e.label,
                                        FontId::new(theme::LEAD_SIZE, theme::medium()),
                                        p.text,
                                    );
                                    if !e.detail.is_empty() {
                                        painter.text(
                                            egui::pos2(label.max.x + 10.0, c.y),
                                            Align2::LEFT_CENTER,
                                            &e.detail,
                                            FontId::proportional(theme::META_SIZE),
                                            p.text_faint,
                                        );
                                    }
                                    if !e.shortcut.is_empty() {
                                        ui.painter().text(
                                            rect.right_center() - vec2(10.0, 0.0),
                                            Align2::RIGHT_CENTER,
                                            &e.shortcut,
                                            FontId::monospace(theme::MONO_SMALL_SIZE),
                                            p.text_faint,
                                        );
                                    }
                                    if resp.clicked() {
                                        chosen = Some(*i);
                                    }
                                }
                            });
                        widgets::scroll_fade(ui, &list, p.elevated);
                    });
            });

        if chosen.is_some() {
            self.open = false;
        }
        chosen.and_then(|i| self.items.get(i).map(|(_, t)| t.clone()))
    }
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
}
