//! Small pieces shared by the dialogs: fields, folder rows, results and ref-name checks.

use super::*;

pub(super) fn home() -> String {
    crate::settings::home_dir().display().to_string()
}

/// Whether git takes `name` as a branch name, by `git check-ref-format --branch`'s rules: no
/// empty, `.`-led or `.lock`-ended part between slashes, no `..`, `@{`, ASCII control
/// character, space or any of `~^:?*[\`, not `@` alone, not ending in `.` and not starting with
/// `-`, and not the reserved `HEAD`.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "git refuses a ref name ending in `.lock`, compared as written"
)]
pub(crate) fn valid_ref_name(name: &str) -> bool {
    !name.is_empty()
        && name != "@"
        && name != "HEAD"
        && !name.starts_with('-')
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name
            .bytes()
            .any(|b| b < 0x20 || b == 0x7f || b" ~^:?*[\\".contains(&b))
        && name
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}

/// Why `name` cannot name a new remote among `existing`, if it cannot.
pub(super) fn remote_name_problem(name: &str, existing: &[&str]) -> Option<String> {
    if name.is_empty() {
        Some("Enter a name for the remote".to_owned())
    } else if !valid_ref_name(name) || name.contains('/') {
        Some("A remote name cannot contain spaces, slashes, .. or ~^:?*[\\".to_owned())
    } else if existing.contains(&name) {
        Some(format!("This repository already has a remote named {name}"))
    } else {
        None
    }
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

/// A field (laid out by `field` at the width it is given) with a control beside its box: the
/// control is centred on the box itself, whatever label sits above the box or message below
/// it. The control's width is remembered from the frame before.
pub(super) fn beside<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    field: impl FnOnce(&mut Ui, f32) -> egui::Response,
    control: impl FnOnce(&mut Ui) -> R,
) -> (egui::Response, R) {
    let m = theme::metrics();
    let key = ui.id().with(("dialog-beside", salt));
    let last: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(m.control * 3.0);
    let width = (ui.available_width() - last - theme::SM).max(m.control * 4.0);
    let r = field(ui, width);
    let slot = egui::Rect::from_min_size(
        egui::pos2(r.rect.right() + theme::SM, r.rect.top()),
        egui::vec2(last.max(1.0), r.rect.height()),
    )
    .expand2(egui::vec2(0.0, theme::XS));
    // A child that does not move the dialog's cursor: the row's height is the field's.
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(slot)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let inner = control(&mut child);
    let used = child.min_rect().width();
    if (used - last).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(key, used));
        ui.ctx().request_repaint();
    }
    (r, inner)
}

/// What a folder field says: its label and placeholder, and the helper or error under it.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FolderField<'a> {
    pub label: &'a str,
    pub hint: &'a str,
    pub helper: Option<&'a str>,
    pub error: Option<&'a str>,
    /// Take the keyboard when nothing else has it.
    pub focus: bool,
}

/// A labelled folder field with the folder picker's Browse beside it (`salt` names both). A
/// chooser that fails says so under the field.
pub(super) fn folder_row(
    ui: &mut Ui,
    picker: &mut crate::ui::folder_picker::FolderPicker,
    salt: &str,
    value: &mut String,
    start: &std::path::Path,
    f: FolderField<'_>,
) -> egui::Response {
    let chooser_error = picker.error(salt).map(str::to_owned);
    let error = f.error.or(chooser_error.as_deref());
    let mut picked = None;
    let (r, ()) = beside(
        ui,
        ("folder", salt),
        |ui, width| {
            let mut field = TextField::new(value)
                .label(f.label)
                .placeholder(f.hint)
                .error(error)
                .width(width)
                .id_salt(("dialog-folder", salt));
            if let Some(h) = f.helper {
                field = field.helper(h);
            }
            field.show(ui)
        },
        |ui| picked = picker.show_button(ui, salt, start),
    );
    if let Some(p) = picked {
        *value = p.display().to_string();
    }
    if f.focus && !r.lost_focus() && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    r
}

/// A spinner and what is running, while a dialog waits for its work.
pub(super) fn working(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SM;
        Spinner::new().size(Size::Sm).label(text).show(ui);
        ui.add(
            egui::Label::new(RichText::new(text).size(m.meta_size).color(p.text_dim)).truncate(),
        );
    });
}

/// How many recent repositories the open dialog offers.
const RECENT_SHOWN: usize = 5;

/// The recent repositories under the open dialog's field; returns the one clicked.
pub(super) fn recent_rows(
    ui: &mut Ui,
    recent: &[std::path::PathBuf],
) -> Option<std::path::PathBuf> {
    use crate::ui::kit::overlay::{Layer, ground};
    use crate::ui::kit::row::SectionHeader;

    let ground = ground(ui.ctx(), Layer::Dialog);
    let mut chosen = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        SectionHeader::new("Recent").first(true).show(ui);
        for path in recent.iter().take(RECENT_SHOWN) {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let r = ListRow::new(("open-recent", path))
                .icon(icon::GIT_BRANCH)
                .label(name)
                .detail(tilde(path))
                .tooltip(path.display().to_string())
                .ground(ground)
                .show(ui);
            if r.clicked() {
                chosen = Some(path.clone());
            }
        }
    });
    chosen
}

/// The open dialog's way out when the folder is not a repository yet: clone one, or start one.
pub(super) fn alternatives(ui: &mut Ui, extra: &mut Option<Outcome>) {
    use crate::keymap::Command;

    let p = palette(ui.ctx());
    let m = theme::metrics();
    ui.add_space(theme::XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SM;
        ui.label(
            RichText::new("Not a repository yet?")
                .size(m.meta_size)
                .color(p.text_faint),
        );
        for (glyph, label, command) in [
            (
                icon::CLOUD_ARROW_DOWN,
                "Clone a repository…",
                Command::Clone,
            ),
            (icon::PLANT, "New repository…", Command::Init),
        ] {
            if Button::new(label)
                .ghost()
                .size(Size::Sm)
                .icon(glyph)
                .id_salt(("open-alternative", label))
                .show(ui)
                .clicked()
            {
                *extra = Some(Outcome::Run(vec![Action::Command(command)]));
            }
        }
    });
}

/// `path` with the home folder written as `~`, as people type it.
pub(super) fn tilde(path: &std::path::Path) -> String {
    tilde_in(path, &crate::settings::home_dir())
}

/// [`tilde`] against `home`: a path in `home` starts with `~`; any other path, or every path
/// when the home folder is unknown (empty), is shown whole. The one place paths are shortened,
/// for dialogs and the welcome screen alike.
pub(crate) fn tilde_in(path: &std::path::Path, home: &std::path::Path) -> String {
    if home.as_os_str().is_empty() {
        return path.display().to_string();
    }
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => std::path::Path::new("~").join(rest).display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// Text shown inline in a recessed box that scrolls on its own (release notes, git's question),
/// fading at the edge where more is scrolled out of sight.
pub(super) fn inset_scroll(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    max_height: f32,
    add: impl FnOnce(&mut Ui),
) {
    InsetBox::new().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let ground = theme::surfaces(ui.ctx()).field;
        crate::ui::kit::overlay::scroll_body(ui, salt, max_height, ground, add);
    });
}

/// Release notes shown inline: Markdown in an [`inset_scroll`] box.
pub(super) fn document(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    max_height: f32,
    text: &str,
) {
    inset_scroll(ui, salt, max_height, |ui| {
        crate::ui::notes::markdown(ui, text);
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

#[cfg(test)]
mod tests {
    use super::valid_ref_name;

    #[test]
    fn reserved_head_cannot_name_a_new_ref() {
        assert!(!valid_ref_name("HEAD"));
        assert!(valid_ref_name("head"));
        assert!(valid_ref_name("topic/HEAD"));
        assert!(valid_ref_name("release/v1.2"));
    }
}
