//! Small pieces shared by the dialogs: fields, buttons, folder picking and ref-name checks.

use super::*;

pub(super) fn home() -> String {
    crate::settings::home_dir().display().to_string()
}

pub(super) fn valid_ref_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with(".lock")
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
}

/// Cancel and confirm buttons with keyboard hints. The safe choice has the initial focus: Cancel
/// when the action destroys something, otherwise the action, so Enter does what is least
/// surprising. `Some(true)` confirms, `Some(false)` cancels.
pub(super) fn confirm_buttons(ui: &mut Ui, confirm: &str, danger: bool) -> Option<bool> {
    let p = palette(ui.ctx());
    let mut answer = None;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if danger {
                "Enter cancels · Esc closes"
            } else {
                "Enter confirms · Esc cancels"
            })
            .size(theme::META_SIZE)
            .color(p.text_faint),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let size = egui::vec2(120.0, 34.0);
            let yes = if danger {
                widgets::danger_button(ui, confirm, size)
            } else {
                widgets::accent_button(ui, confirm, size)
            };
            let no = ui.add_sized([90.0, 34.0], egui::Button::new("Cancel"));
            if ui.memory(|m| m.focused().is_none()) {
                if danger {
                    no.request_focus()
                } else {
                    yes.request_focus()
                }
            }
            if yes.clicked() {
                answer = Some(true);
            }
            if no.clicked() {
                answer = Some(false);
            }
        });
    });
    answer
}

/// An icon and a line of text that wraps within the dialog, aligned under itself.
pub(super) fn bullet(ui: &mut Ui, glyph: &str, color: egui::Color32, text: &str) {
    let p = palette(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.label(RichText::new(glyph).color(color));
        ui.add(egui::Label::new(RichText::new(text).size(theme::META_SIZE).color(p.text)).wrap());
    });
    ui.add_space(2.0);
}

pub(super) fn multiline_key() -> Id {
    Id::new("dialog-multiline")
}

/// Records the dialog's multi-line field so Enter there inserts a newline instead of submitting.
pub(super) fn remember_multiline(ui: &Ui, id: Id) {
    ui.ctx().data_mut(|d| d.insert_temp(multiline_key(), id));
}

pub(super) fn field(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
) -> egui::Response {
    let p = palette(ui.ctx());
    ui.label(
        RichText::new(label)
            .size(theme::META_SIZE)
            .color(p.text_dim),
    );
    let r = ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(theme::FIELD_MARGIN),
    );
    if focus && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    ui.add_space(8.0);
    r
}

/// Document choices stay outside the scrolling body.
pub(super) fn document_actions(ui: &mut Ui, dialog: &Dialog) -> Outcome {
    let mut outcome = Outcome::Keep;
    ui.add_space(theme::SM);
    ui.horizontal(|ui| {
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| match dialog {
                Dialog::Consent {
                    step: Pending::Terms,
                    ..
                } => {
                    if widgets::primary_button(ui, "I agree", true, 130.0).clicked() {
                        outcome = Outcome::Run(vec![Action::AcceptTerms]);
                    }
                    if ui
                        .add_sized([90.0, 34.0], egui::Button::new("Quit"))
                        .clicked()
                    {
                        outcome = Outcome::Run(vec![Action::Quit]);
                    }
                }
                Dialog::Consent {
                    step: Pending::CrashReports,
                    ..
                } => {
                    for (label, enabled) in [("Send crash reports", true), ("Don't send", false)] {
                        if ui
                            .add_sized(
                                [170.0, 34.0],
                                egui::Button::new(
                                    RichText::new(label)
                                        .font(FontId::new(theme::LEAD_SIZE, theme::medium())),
                                ),
                            )
                            .clicked()
                        {
                            outcome = Outcome::Run(vec![Action::SetCrashReports(enabled)]);
                        }
                    }
                }
                Dialog::Legal(_)
                    if ui
                        .add(egui::Button::new("Close").min_size(egui::vec2(110.0, 34.0)))
                        .clicked()
                        || ui.input(|i| i.key_pressed(Key::Enter)) =>
                {
                    outcome = Outcome::Close;
                }
                _ => {}
            },
        );
    });
    outcome
}
