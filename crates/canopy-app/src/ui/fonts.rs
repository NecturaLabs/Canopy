//! Bundled fonts: Inter for the interface, JetBrains Mono for code and ids, Phosphor for icons.
//! Licences: `crates/canopy-app/assets/fonts/README.md`.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

/// Installs the fonts on `ctx`.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        "inter",
        include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    );
    add(
        "inter-medium",
        include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    );
    add(
        "inter-semibold",
        include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    );
    add(
        "jetbrains-mono",
        include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"),
    );

    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    // Phosphor icons live in the Unicode private-use area. The bundled Inter files have their own
    // private-use mappings removed (see assets/fonts/README.md) and Phosphor comes straight after
    // Inter, ahead of egui's emoji font, which also uses that area. egui's defaults stay as
    // fallbacks for scripts and emoji Inter lacks.
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f != "phosphor")
        .collect();
    let with = |primary: &str| {
        let mut v = vec![primary.to_owned(), "phosphor".to_owned()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with("inter"));
    fonts
        .families
        .insert(FontFamily::Name("medium".into()), with("inter-medium"));
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), with("inter-semibold"));
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        // Icons inside monospace text (branch names in pills) come from Phosphor, ahead of the
        // default monospace font's own private-use glyphs.
        mono.retain(|f| f != "phosphor");
        mono.insert(0, "jetbrains-mono".to_owned());
        mono.insert(1, "phosphor".to_owned());
    }
    ctx.set_fonts(fonts);
}
