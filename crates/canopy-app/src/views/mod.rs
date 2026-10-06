//! Screens and panels. Views read tab state and push [`crate::action::Action`]s; they never call git.

pub mod changes;
pub mod commit_detail;
pub mod dialogs;
pub mod diff;
pub mod history;
pub mod requests;
pub mod resolve;
pub mod settings_screen;
pub mod sidebar;
pub mod state_banner;
pub mod statusbar;
pub mod toolbar;
pub mod welcome;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Stock egui widgets and tooltips that have a kit component (design spec 8.1). A view uses
    /// the kit one instead, so every control looks, moves and speaks the same.
    const STOCK: [&str; 16] = [
        ".on_hover_text(",
        ".on_disabled_hover_text(",
        "egui::Button",
        "egui::ComboBox",
        "egui::CollapsingHeader",
        ".spinner(",
        "egui::Spinner",
        "ui.button(",
        ".small_button(",
        ".checkbox(",
        "selectable_label",
        "egui::Slider",
        "DragValue",
        "ProgressBar",
        "hyperlink",
        "ui.separator(",
    ];

    fn view_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("views folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                view_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// M5: no stock egui widget is left in a view's own code (its tests may use them).
    #[test]
    fn views_use_kit_components_not_stock_widgets() {
        let mut files = Vec::new();
        view_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/views"),
            &mut files,
        );
        let mut found = Vec::new();
        for path in files {
            let src = std::fs::read_to_string(&path).expect("view source");
            // The file up to its test module; a test-only helper above it is still scanned.
            let code = src
                .split("\n#[cfg(test)]\nmod tests")
                .next()
                .unwrap_or_default();
            for (i, line) in code.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for stock in STOCK {
                    if line.contains(stock) {
                        found.push(format!("{}:{}: {stock}", path.display(), i + 1));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "stock widgets in views:\n{}",
            found.join("\n")
        );
    }
}
