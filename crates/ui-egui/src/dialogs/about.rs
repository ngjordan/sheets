use super::Dialog;

/// Help ▸ About Iridium Sheets: the app, its contributors and the AI models that helped. The credits
/// are compiled in (`crate::credits`, docs/contributors.md).
pub(super) fn about(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        for tab in ["About", "Contributors", "Models"] {
            if ui.selectable_label(d.tab == tab, tab).clicked() {
                d.tab = tab.to_string();
            }
        }
    });
    ui.separator();
    match d.tab.as_str() {
        "Contributors" => crate::credits::contributors_ui(ui),
        "Models" => crate::credits::models_ui(ui),
        _ => {
            ui.heading("Iridium Sheets");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(4.0);
            ui.label("A clean-room, Rust-native spreadsheet by Iridium Technologies, Inc.");
            ui.hyperlink_to("iridium.ph", "https://iridium.ph");
            ui.label("Based on GridCraft by the ArtCraft team. MIT OR Apache-2.0.");
        }
    }
}
