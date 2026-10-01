//! The command palette (Ctrl+K): fuzzy search over every command, branch and recent repository.

use egui::{Align2, CornerRadius, FontId, Id, Key, Order, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;

use super::theme::{self, Palette, palette};

/// One entry the palette can run.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Icon glyph.
    pub icon: &'static str,
    /// Main text.
    pub label: String,
    /// Secondary text (a group such as "Branch" or a path).
    pub detail: String,
    /// Keyboard shortcut hint.
    pub shortcut: &'static str,
}

/// Scores how well `query` matches `text` as a subsequence. Consecutive and word-start matches
/// score higher. `None` when not all query characters occur in order.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text_lower: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut last_match: Option<usize> = None;
    for qc in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
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

/// Palette state.
#[derive(Debug, Default)]
pub struct CommandPalette {
    /// Shown this frame.
    pub open: bool,
    query: String,
    selected: usize,
    just_opened: bool,
}

impl CommandPalette {
    /// Opens with an empty query.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.just_opened = true;
    }

    /// Draws the palette over `entries`; returns the index of the entry the user ran.
    pub fn show(&mut self, ctx: &egui::Context, entries: &[Entry]) -> Option<usize> {
        if !self.open {
            return None;
        }
        let p = palette(ctx);
        let mut ranked: Vec<(i32, usize)> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let a = fuzzy_score(&self.query, &e.label);
                let b =
                    fuzzy_score(&self.query, &format!("{} {}", e.detail, e.label)).map(|s| s - 5);
                a.max(b).map(|s| (s, i))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked.truncate(60);

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
                ui.painter().rect_filled(
                    screen,
                    CornerRadius::ZERO,
                    egui::Color32::from_black_alpha(if p.dark { 110 } else { 50 }),
                );
                if resp.clicked() {
                    self.open = false;
                }
            });

        let width = 620.0_f32.min(screen.width() - 40.0);
        egui::Area::new(Id::new("palette"))
            .order(Order::Tooltip)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(screen.center().x, screen.min.y + 90.0))
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
                                    .size(17.0)
                                    .color(p.accent),
                            );
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .hint_text("Type a command, branch or repository…")
                                    .font(FontId::proportional(15.0))
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
                        egui::ScrollArea::vertical()
                            .max_height(380.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (row, (_, i)) in ranked.iter().enumerate() {
                                    let e = &entries[*i];
                                    let selected = row == self.selected;
                                    let (rect, resp) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), 34.0),
                                        Sense::click(),
                                    );
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
                                        FontId::proportional(16.0),
                                        if selected { p.accent } else { p.text_dim },
                                    );
                                    let label = ui.painter().text(
                                        c + vec2(34.0, 0.0),
                                        Align2::LEFT_CENTER,
                                        &e.label,
                                        FontId::new(13.5, theme::medium()),
                                        p.text,
                                    );
                                    if !e.detail.is_empty() {
                                        ui.painter().text(
                                            egui::pos2(label.max.x + 10.0, c.y),
                                            Align2::LEFT_CENTER,
                                            &e.detail,
                                            FontId::proportional(12.0),
                                            p.text_faint,
                                        );
                                    }
                                    if !e.shortcut.is_empty() {
                                        ui.painter().text(
                                            rect.right_center() - vec2(10.0, 0.0),
                                            Align2::RIGHT_CENTER,
                                            e.shortcut,
                                            FontId::monospace(11.5),
                                            p.text_faint,
                                        );
                                    }
                                    if resp.clicked() {
                                        chosen = Some(*i);
                                    }
                                }
                            });
                    });
            });

        if chosen.is_some() {
            self.open = false;
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

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
