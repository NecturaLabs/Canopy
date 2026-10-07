//! The reading dialog for the Terms of Service and the Privacy Policy: the kit's reading dialog,
//! as tall as a dialog may be, with both documents as tabs in its title row, a scrolling body
//! whose progress runs along the seam under the tabs, and a footer that either closes it or
//! carries the consent's own answers.

use egui::{Align, Layout, Rect, UiBuilder, pos2, vec2};

use super::*;
use crate::ui::kit::choice::UnderlineTabs;
use crate::ui::legal::{self, Link};
use crate::ui::motion;

/// A scroll far past either end of any document (Home and End).
const TO_THE_END: f32 = 1.0e6;

/// The footer of a reading dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Footer<'a> {
    /// A Close button; Esc and the scrim close it too.
    Close,
    /// The consent's two answers (`first`, rightmost, then `second`) beside a Back button. Equal
    /// answers (`neutral`) are both Secondary; otherwise `first` is Primary. Esc goes back;
    /// the scrim does nothing.
    Consent {
        first: &'a str,
        second: &'a str,
        neutral: bool,
    },
}

/// What the reader's controls answered this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Answer {
    Open,
    /// Close, Esc or the scrim (`Footer::Close`).
    Close,
    /// Back to the consent summary (`Footer::Consent`: the Back button or Esc).
    Back,
    /// The consent's first answer.
    First,
    /// The consent's second answer.
    Second,
    /// A link in the document.
    Link(Link),
}

/// Shows the reading dialog for `doc` (its tabs switch it) and returns what was answered.
pub(super) fn show(ctx: &egui::Context, doc: &mut Document, footer: Footer<'_>) -> Answer {
    let shown = *doc;
    let mut link = None;
    let mut answered = Answer::Open;
    // A link's menu open when the frame starts takes this frame's Esc (and closes before the
    // footer is drawn): Esc goes back only once nothing is above the reader.
    let menu_open = egui::Popup::is_any_open(ctx);
    let options = [
        (Document::Terms, Document::Terms.title()),
        (Document::Privacy, Document::Privacy.title()),
    ];
    let dialog = KitDialog::new("legal-reader")
        .label(shown.title())
        .icon(shown.glyph())
        .width(DialogWidth::Reading)
        .fill_height()
        .progress_lane()
        .scroll_id(shown)
        // Nothing focused: Space and the arrows scroll the document.
        .initial_focus(InitialFocus::Body)
        .header(|ui| {
            UnderlineTabs::new(doc, &options)
                .label("Legal documents")
                .id_salt("legal-reader-tabs")
                .show(ui);
        })
        .body(|ui| {
            start_at_top(ui, shown);
            scroll_keys(ui, ui.clip_rect().height());
            ui.add_space(theme::SM);
            link = legal::show(ui, shown);
            ui.add_space(theme::XL);
        });
    let result = match footer {
        Footer::Close => dialog
            .closable()
            .footer(|f| {
                f.confirm("Close");
            })
            .show(ctx),
        Footer::Consent {
            first,
            second,
            neutral,
        } => dialog
            .blocking()
            .custom_footer(|ui| {
                answered = consent_footer(ui, first, second, neutral, menu_open);
                match answered {
                    Answer::First => DialogResult::Confirm,
                    Answer::Back | Answer::Second => DialogResult::Cancel,
                    _ => DialogResult::Open,
                }
            })
            .show(ctx),
    };
    if let Some(link) = link {
        return Answer::Link(link);
    }
    match footer {
        Footer::Close if result != DialogResult::Open => Answer::Close,
        Footer::Close => Answer::Open,
        Footer::Consent { .. } => answered,
    }
}

/// The consent's footer: Back at the left (Esc too), the two answers at the right.
fn consent_footer(
    ui: &mut Ui,
    first: &str,
    second: &str,
    neutral: bool,
    menu_open: bool,
) -> Answer {
    let back = Button::new("Back")
        .ghost()
        .size(Size::Md)
        .icon(icon::ARROW_LEFT)
        .id_salt("legal-reader-back")
        .show(ui)
        .clicked()
        || (!menu_open && ui.input(|i| i.key_pressed(Key::Escape)));
    let mut answer = if back { Answer::Back } else { Answer::Open };
    // Made left to right (the second answer first) so Tab meets them in reading order; placed
    // at the right by their width last frame, as the kit's own footer.
    let row = ui.max_rect();
    let width_key = ui.id().with("answers-width");
    let last: f32 = ui.data(|d| d.get_temp(width_key)).unwrap_or(0.0);
    let slot = Rect::from_min_max(
        pos2((row.right() - last).max(ui.cursor().left()), row.top()),
        row.max,
    );
    let answers = ui.scope_builder(
        UiBuilder::new()
            .max_rect(slot)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::SM;
            if Button::new(second)
                .secondary()
                .size(Size::Md)
                .id_salt("legal-reader-second")
                .show(ui)
                .clicked()
            {
                answer = Answer::Second;
            }
            let b = Button::new(first)
                .size(Size::Md)
                .id_salt("legal-reader-first");
            let b = if neutral { b.secondary() } else { b.primary() };
            if b.show(ui).clicked() {
                answer = Answer::First;
            }
        },
    );
    let width = answers.response.rect.width();
    if (width - last).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(width_key, width));
        ui.ctx().request_repaint();
    }
    answer
}

/// Starts each document at the top the first time a freshly opened reader shows it. egui keeps a
/// document's scroll position (across restarts too); it is kept only while the reader stays open,
/// for flipping between its tabs, so the first-start consent never opens part-way down the Terms.
fn start_at_top(ui: &Ui, doc: Document) {
    let ctx = ui.ctx();
    let shown_key = egui::Id::new("legal-reader-shown");
    let unread_key = egui::Id::new("legal-reader-unread");
    let pass = ctx.cumulative_pass_nr();
    let open = ctx
        .data(|d| d.get_temp::<u64>(shown_key))
        .is_some_and(|last| last + 1 >= pass);
    let mut unread: Vec<Document> = if open {
        ctx.data(|d| d.get_temp(unread_key)).unwrap_or_default()
    } else {
        vec![Document::Terms, Document::Privacy]
    };
    if let Some(i) = unread.iter().position(|d| *d == doc) {
        unread.remove(i);
        ui.scroll_with_delta_animation(vec2(0.0, TO_THE_END), egui::style::ScrollAnimation::none());
    }
    ctx.data_mut(|d| {
        d.insert_temp(shown_key, pass);
        d.insert_temp(unread_key, unread);
    });
}

/// Scrolls the document from the keyboard: Page Up/Down and Home/End always, Space and the
/// arrows while no control has focus (focused, they belong to it).
fn scroll_keys(ui: &Ui, view: f32) {
    let page = (view - 3.0 * theme::XL).max(view * 0.5);
    let step = theme::XL * 2.0;
    let free = ui.memory(|m| m.focused().is_none());
    let delta = ui.input(|i| {
        let pressed = |k: Key| i.key_pressed(k) && i.modifiers.is_none();
        if pressed(Key::PageDown) || (free && pressed(Key::Space)) {
            -page
        } else if pressed(Key::PageUp) || (free && i.key_pressed(Key::Space) && i.modifiers.shift) {
            page
        } else if pressed(Key::End) {
            -TO_THE_END
        } else if pressed(Key::Home) {
            TO_THE_END
        } else if free && pressed(Key::ArrowDown) {
            -step
        } else if free && pressed(Key::ArrowUp) {
            step
        } else {
            0.0
        }
    });
    if delta != 0.0 {
        let seconds = motion::time(motion::BASE);
        let animation = if seconds > 0.0 {
            egui::style::ScrollAnimation::duration(seconds)
        } else {
            egui::style::ScrollAnimation::none()
        };
        ui.scroll_with_delta_animation(vec2(0.0, delta), animation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// polish F2: a reader opened afresh starts at the top of its document wherever it was left
    /// (egui keeps a scroll position, across restarts too), as the first-start consent must.
    #[test]
    fn a_reader_opened_afresh_starts_at_the_top() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let mut t = 0.0;
        // One frame, the reader open or not: whether the document's first line is in sight.
        let mut frame = |events: Vec<egui::Event>, open: bool| {
            t += 0.1;
            let mut doc = Document::Terms;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0))),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if open {
                        show(ui.ctx(), &mut doc, Footer::Close);
                    }
                },
            );
            out.textures_delta.clear();
            out.shapes.iter().any(|c| match &c.shape {
                egui::Shape::Text(text) => {
                    text.galley.text().starts_with("Last updated")
                        && c.clip_rect.contains_rect(text.visual_bounding_rect())
                }
                _ => false,
            })
        };
        let end = egui::Event::Key {
            key: Key::End,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for _ in 0..5 {
            frame(Vec::new(), true);
        }
        assert!(frame(Vec::new(), true), "the reader opens at the top");
        frame(vec![end], true);
        for _ in 0..5 {
            frame(Vec::new(), true);
        }
        assert!(!frame(Vec::new(), true), "End scrolls away from the top");
        for _ in 0..5 {
            frame(Vec::new(), false);
        }
        for _ in 0..5 {
            frame(Vec::new(), true);
        }
        assert!(
            frame(Vec::new(), true),
            "opened again, it starts at the top"
        );
    }

    /// usersim2 N1: Esc closes the topmost layer only. With a link's context menu open over the
    /// reader, the first Esc closes the menu and the reader stays; the next Esc closes the reader.
    #[test]
    fn esc_closes_a_links_context_menu_before_the_reader() {
        use crate::ui::kit::testkit;
        for footer in [
            Footer::Close,
            Footer::Consent {
                first: "Accept",
                second: "Decline",
                neutral: false,
            },
        ] {
            let ctx = testkit::ctx();
            ctx.enable_accesskit();
            let mut t = 0.0;
            let mut frame = |events: Vec<egui::Event>| {
                t += 0.1;
                let mut doc = Document::Terms;
                let mut answer = Answer::Open;
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(1200.0, 800.0),
                        )),
                        time: Some(t),
                        events,
                        ..Default::default()
                    },
                    |ui| answer = show(ui.ctx(), &mut doc, footer),
                );
                out.textures_delta.clear();
                (out, answer)
            };
            for _ in 0..5 {
                frame(Vec::new());
            }
            // Page down to the first link in view.
            let mut link = None;
            for _ in 0..20 {
                let (out, _) = frame(testkit::key(Key::PageDown));
                let (out2, _) = frame(Vec::new());
                drop(out);
                link = out2
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .and_then(|u| {
                        u.nodes
                            .iter()
                            .filter(|(_, n)| n.role() == egui::accesskit::Role::Link)
                            .filter_map(|(_, n)| n.bounds())
                            .map(|b| {
                                pos2(
                                    crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
                                    crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
                                )
                            })
                            .find(|p| p.y > 150.0 && p.y < 650.0)
                    });
                if link.is_some() {
                    break;
                }
            }
            let link = link.expect("a link in view");
            frame(testkit::secondary_click(link));
            for _ in 0..3 {
                frame(Vec::new());
            }
            assert!(
                egui::Popup::is_any_open(&ctx),
                "{footer:?}: the link's menu is open"
            );
            let (_, answer) = frame(testkit::key(Key::Escape));
            assert_eq!(
                answer,
                Answer::Open,
                "{footer:?}: the first Esc keeps the reader"
            );
            for _ in 0..3 {
                let (_, answer) = frame(Vec::new());
                assert_eq!(answer, Answer::Open, "{footer:?}");
            }
            assert!(
                !egui::Popup::is_any_open(&ctx),
                "{footer:?}: the first Esc closed the menu"
            );
            let (_, answer) = frame(testkit::key(Key::Escape));
            let wanted = match footer {
                Footer::Close => Answer::Close,
                Footer::Consent { .. } => Answer::Back,
            };
            assert_eq!(answer, wanted, "{footer:?}: the next Esc leaves the reader");
        }
    }
}
