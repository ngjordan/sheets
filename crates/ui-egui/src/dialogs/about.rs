use super::Dialog;

/// Help ▸ About Sheets: the app, its contributors and the AI models that helped. The credits
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
            ui.heading(iridium_core::brand::NAME);
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(4.0);
            ui.label("A clean-room, Rust-native spreadsheet by Iridium Technologies, Inc.");
            ui.hyperlink_to("iridium-sheets.ph", "https://iridium-sheets.ph");
            ui.label("MIT OR Apache-2.0. The upstream credits are in the NOTICE file.");
        }
    }
}
