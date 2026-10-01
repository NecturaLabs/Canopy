//! The SVG art in `assets/art/` (placeholders until the generated art lands; see
//! `docs/art-prompts.md`). Rendered by egui_extras' resvg loader, so file names and view boxes
//! must stay stable when the art is replaced.

use egui::{ImageSource, include_image};

/// The Canopy mark, 24×24.
pub fn logo() -> ImageSource<'static> {
    include_image!("../../assets/art/logo.svg")
}

/// The welcome screen illustration, 640×360.
pub fn welcome() -> ImageSource<'static> {
    include_image!("../../assets/art/welcome.svg")
}

/// "Working tree clean", 160×120.
pub fn empty_clean() -> ImageSource<'static> {
    include_image!("../../assets/art/empty-clean.svg")
}

/// "No commits yet", 160×120.
pub fn empty_history() -> ImageSource<'static> {
    include_image!("../../assets/art/empty-history.svg")
}

/// A repository with no remote, 160×120.
pub fn local_only() -> ImageSource<'static> {
    include_image!("../../assets/art/local-only.svg")
}

/// Something went wrong, 160×120.
pub fn empty_error() -> ImageSource<'static> {
    include_image!("../../assets/art/empty-error.svg")
}

/// Draws `source` at `size` points, centred in the available width.
pub fn show(ui: &mut egui::Ui, source: ImageSource<'static>, size: egui::Vec2) -> egui::Response {
    ui.add(egui::Image::new(source).fit_to_exact_size(size))
}
