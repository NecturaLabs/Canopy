//! The component kit: `nectura-ui`'s, plus the Git components that need `canopy-core`.

pub use nectura_ui::kit::*;

pub mod git;
pub mod graph;

/// The gallery's sections the library does not have.
pub const GALLERY_SECTIONS: &[gallery::Section] =
    &[("Git", git::gallery), ("Graph", graph::gallery)];

#[cfg(test)]
mod tests {
    /// The Git components, the cheatsheet, the palette and the history, changes, diff and
    /// conflict views show the kit tooltip (`overlay::TooltipExt`), never egui's stock one, so
    /// every tooltip looks and moves the same.
    #[test]
    fn the_kit_uses_its_own_tooltip() {
        const STOCK: [&str; 5] = [
            ".on_hover_text(",
            ".on_disabled_hover_text(",
            ".on_hover_text_at_pointer(",
            ".on_hover_ui(",
            ".on_hover_ui_at_pointer(",
        ];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        let mut files: Vec<_> = [
            "kit/git.rs",
            "kit/graph.rs",
            "cheatsheet.rs",
            "palette.rs",
            "widgets.rs",
        ]
        .map(|f| src.join(f))
        .into();
        files.extend(
            [
                "history.rs",
                "changes.rs",
                "diff.rs",
                "statusbar.rs",
                "state_banner.rs",
                "commit_detail.rs",
                "resolve.rs",
            ]
            .map(|f| src.with_file_name("views").join(f)),
        );
        let mut found = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path).expect("kit source");
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
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
        assert!(found.is_empty(), "stock tooltips:\n{}", found.join("\n"));
    }
}
