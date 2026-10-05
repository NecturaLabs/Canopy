//! Small pieces shared by the dialogs: fields, folder rows, results and ref-name checks.

use super::*;

pub(super) fn home() -> String {
    crate::settings::home_dir().display().to_string()
}

#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "git refuses a ref name ending in `.lock`, compared as written"
)]
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

/// What the dialog's buttons answered: `Close` on Cancel, the scrim or the close button, the
/// confirming outcome on the confirming button (or Enter).
pub(super) fn answer(result: DialogResult, confirm: impl FnOnce() -> Outcome) -> Outcome {
    match result {
        DialogResult::Open => Outcome::Keep,
        DialogResult::Cancel => Outcome::Close,
        DialogResult::Confirm => confirm(),
    }
}

/// The confirming button: enabled with `label`, or disabled saying `reason`.
pub(super) fn confirm(f: &mut Footer, label: &str, ok: bool, reason: &str) {
    f.cancel().confirm(label);
    if !ok {
        f.confirm_disabled(reason);
    }
}

/// Bare Enter on a dialog whose answer should be a deliberate click (consent, trust): with
/// nothing focused, Enter does nothing, as in a dialog without a default button.
pub(super) fn swallow_enter(ctx: &egui::Context) {
    if ctx.memory(|m| m.focused().is_none()) {
        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
    }
}

/// An icon and a line of text that wraps within the dialog, aligned under itself.
pub(super) fn bullet(ui: &mut Ui, glyph: &str, color: egui::Color32, text: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal_top(|ui| {
        ui.label(RichText::new(glyph).size(m.icon_md).color(color));
        ui.add(egui::Label::new(RichText::new(text).size(m.meta_size).color(p.text)).wrap());
    });
}

/// A quiet line of text (`meta_size`) in `color`.
pub(super) fn note(ui: &mut Ui, text: impl Into<String>, color: egui::Color32) {
    let m = theme::metrics();
    ui.add(egui::Label::new(RichText::new(text).size(m.meta_size).color(color)).wrap());
}

/// A labelled one-line field; `focus` takes the keyboard when nothing else has it (but not in
/// the frame Enter let go of it, so Enter reaches the dialog and submits it).
pub(super) fn field(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
) -> egui::Response {
    field_with_error(ui, label, value, hint, focus, None)
}

pub(super) fn field_with_error(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
    error: Option<&str>,
) -> egui::Response {
    let r = TextField::new(value)
        .label(label)
        .placeholder(hint)
        .error(error)
        .id_salt(("dialog-field", label))
        .show(ui);
    if focus && !r.lost_focus() && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    r
}

/// A labelled folder field with the folder picker beside it.
#[expect(
    clippy::too_many_arguments,
    reason = "one row: label, value, picker and its start"
)]
pub(super) fn folder_row(
    ui: &mut Ui,
    picker: &mut crate::ui::folder_picker::FolderPicker,
    salt: &str,
    label: &str,
    value: &mut String,
    hint: &str,
    start: &std::path::Path,
    focus: bool,
) {
    // The picker's button lines up with the field under the label.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
            if let Some(picked) = picker.show(ui, salt, start) {
                *value = picked.display().to_string();
            }
            let r = TextField::new(value)
                .label(label)
                .placeholder(hint)
                .width(ui.available_width())
                .id_salt(("dialog-folder", salt))
                .show(ui);
            if focus && !r.lost_focus() && ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
        });
    });
}

/// A document shown inline, in a recessed box that scrolls on its own.
pub(super) fn document(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    max_height: f32,
    text: &str,
) {
    InsetBox::new().show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical()
            .id_salt(salt)
            .max_height(max_height)
            .show(ui, |ui| crate::ui::notes::markdown(ui, text));
    });
}

/// The name and email that `scope`'s configuration file sets for the author, as the active
/// repository reports them, for a dialog that is about to remove them. `None` when the repository
/// has not read them or they come from elsewhere.
pub(super) fn identity_in_scope(
    tab: Option<&RepoTab>,
    scope: canopy_core::identity::Scope,
) -> Option<String> {
    use canopy_core::identity::{Scope, Source};

    fn set_here<'a>(field: &'a canopy_core::identity::Field, wanted: &str) -> Option<&'a str> {
        matches!(&field.source, Some(Source::Config { scope, .. }) if scope == wanted)
            .then(|| field.value.trim())
            .filter(|v| !v.is_empty())
    }

    let wanted = match scope {
        Scope::Local => "local",
        Scope::Global => "global",
    };
    let author = &tab?.identity.as_ref()?.author;
    match (
        set_here(&author.name, wanted),
        set_here(&author.email, wanted),
    ) {
        (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
        (Some(value), None) | (None, Some(value)) => Some(value.to_owned()),
        (None, None) => None,
    }
}
