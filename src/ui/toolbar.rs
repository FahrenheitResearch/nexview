use egui::{self, Color32, RichText};
use crate::app::RadarApp;
use crate::nexrad::{RadarProduct, sites::RADAR_SITES};

/// Products shown in the toolbar tab bar.
const TOOLBAR_PRODUCTS: &[(RadarProduct, &str)] = &[
    (RadarProduct::SuperResReflectivity, "SR-R"),
    (RadarProduct::SuperResVelocity, "SR-V"),
    (RadarProduct::Reflectivity, "REF"),
    (RadarProduct::Velocity, "VEL"),
    (RadarProduct::SpectrumWidth, "SW"),
    (RadarProduct::DifferentialReflectivity, "ZDR"),
    (RadarProduct::CorrelationCoefficient, "CC"),
    (RadarProduct::SpecificDiffPhase, "KDP"),
    (RadarProduct::StormRelativeVelocity, "SRV"),
    (RadarProduct::VIL, "VIL"),
    (RadarProduct::EchoTops, "ET"),
];

pub struct Toolbar;

impl Toolbar {
    pub fn show(app: &mut RadarApp, ctx: &egui::Context) {
        let accent = Color32::from_rgb(0x00, 0xE5, 0xFF);
        let bg_panel = Color32::from_rgb(0x25, 0x25, 0x35);
        let text_primary = Color32::from_rgb(0xE0, 0xE0, 0xE0);
        let text_secondary = Color32::from_rgb(0x80, 0x80, 0x90);
        let border = Color32::from_rgb(0x35, 0x35, 0x45);

        egui::TopBottomPanel::top("toolbar_top")
            .exact_height(36.0)
            .frame(egui::Frame::new()
                .fill(bg_panel)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .stroke(egui::Stroke::new(1.0, border)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;

                    // 1. App label
                    ui.label(
                        RichText::new("NexView")
                            .color(accent)
                            .strong()
                            .size(15.0),
                    );

                    ui.separator();

                    // 2. Station dropdown (compact combo box with search)
                    Self::station_combo(app, ui, accent, text_primary, text_secondary);

                    ui.separator();

                    // 3. Product tab bar
                    Self::product_tabs(app, ui, accent, bg_panel, text_primary, text_secondary);

                    ui.separator();

                    // 4. Elevation selector (compact up/down + angle)
                    Self::elevation_compact(app, ui, text_primary, text_secondary);

                    ui.separator();

                    // 5. View mode buttons
                    Self::view_mode_buttons(app, ui, accent, text_primary);

                    ui.separator();

                    // 6. Settings gear (placeholder)
                    if ui.button(RichText::new("\u{2699}").size(16.0)).on_hover_text("Settings").clicked() {
                        // Placeholder — will be wired later
                    }
                });
            });
    }

    fn station_combo(
        app: &mut RadarApp,
        ui: &mut egui::Ui,
        accent: Color32,
        text_primary: Color32,
        _text_secondary: Color32,
    ) {
        let station_text = RichText::new(&app.selected_station)
            .color(accent)
            .strong()
            .size(13.0);

        egui::ComboBox::from_id_salt("toolbar_station")
            .selected_text(station_text)
            .width(180.0)
            .show_ui(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut app.station_filter);
                });

                let filter = app.station_filter.to_uppercase();

                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .id_salt("toolbar_station_list")
                    .show(ui, |ui| {
                        for site in RADAR_SITES.iter() {
                            if !filter.is_empty()
                                && !site.id.contains(&filter)
                                && !site.name.to_uppercase().contains(&filter)
                                && !site.state.contains(&filter)
                            {
                                continue;
                            }

                            let label = format!("{} - {}", site.id, site.name);
                            let selected = app.selected_station == site.id;
                            if ui.selectable_label(selected, &label).clicked() {
                                app.select_station(site.id);
                            }
                        }
                    });
            });
    }

    fn product_tabs(
        app: &mut RadarApp,
        ui: &mut egui::Ui,
        accent: Color32,
        bg_panel: Color32,
        text_primary: Color32,
        text_secondary: Color32,
    ) {
        for &(product, label) in TOOLBAR_PRODUCTS {
            let is_active = app.selected_product == product;

            let text = if is_active {
                RichText::new(label).color(bg_panel).strong().size(11.0)
            } else {
                RichText::new(label).color(text_secondary).size(11.0)
            };

            let button = egui::Button::new(text)
                .corner_radius(egui::CornerRadius::same(3))
                .min_size(egui::vec2(28.0, 22.0));

            let button = if is_active {
                button.fill(accent)
            } else {
                button.fill(Color32::TRANSPARENT)
            };

            let response = ui.add(button);

            if response.clicked() {
                if product == crate::nexrad::RadarProduct::StormRelativeVelocity
                    && app.selected_product != crate::nexrad::RadarProduct::StormRelativeVelocity
                {
                    app.estimate_storm_motion();
                }
                app.selected_product = product;
                app.needs_render = true;
            }

            if response.hovered() && !is_active {
                // Tooltip with full name
                response.on_hover_text(product.display_name());
            }
        }
    }

    #[allow(unused_variables)]
    fn elevation_compact(
        app: &mut RadarApp,
        ui: &mut egui::Ui,
        text_primary: Color32,
        text_secondary: Color32,
    ) {
        let angle_text = if let Some(ref file) = app.current_file {
            if let Some(sweep) = file.sweeps.get(app.selected_elevation) {
                format!("{:.1}\u{00B0}", sweep.elevation_angle)
            } else {
                "--.-\u{00B0}".to_string()
            }
        } else {
            "--.-\u{00B0}".to_string()
        };

        let max_elev = app.current_file
            .as_ref()
            .map(|f| f.sweeps.len().saturating_sub(1))
            .unwrap_or(0);

        // Down arrow
        if ui.add_enabled(
            app.selected_elevation > 0,
            egui::Button::new(RichText::new("\u{25BC}").size(10.0))
                .min_size(egui::vec2(18.0, 22.0)),
        ).clicked() {
            app.selected_elevation = app.selected_elevation.saturating_sub(1);
            app.needs_render = true;
        }

        ui.label(
            RichText::new(&angle_text)
                .color(text_primary)
                .size(12.0)
                .monospace(),
        );

        // Up arrow
        if ui.add_enabled(
            app.selected_elevation < max_elev,
            egui::Button::new(RichText::new("\u{25B2}").size(10.0))
                .min_size(egui::vec2(18.0, 22.0)),
        ).clicked() {
            app.selected_elevation += 1;
            app.needs_render = true;
        }
    }

    fn view_mode_buttons(
        app: &mut RadarApp,
        ui: &mut egui::Ui,
        accent: Color32,
        text_primary: Color32,
    ) {
        #[derive(PartialEq)]
        enum ViewMode { Single, Quad, Dual, Wall }

        let current = if app.wall_mode {
            ViewMode::Wall
        } else if app.quad_view {
            ViewMode::Quad
        } else if app.dual_pane {
            ViewMode::Dual
        } else {
            ViewMode::Single
        };

        let modes = [
            (ViewMode::Single, "1"),
            (ViewMode::Quad, "4"),
            (ViewMode::Dual, "2"),
            (ViewMode::Wall, "W"),
        ];

        for (mode, label) in &modes {
            let is_active = current == *mode;
            let text = if is_active {
                RichText::new(*label).color(Color32::BLACK).strong().size(11.0)
            } else {
                RichText::new(*label).color(text_primary).size(11.0)
            };

            let button = egui::Button::new(text)
                .corner_radius(egui::CornerRadius::same(3))
                .min_size(egui::vec2(22.0, 22.0));

            let button = if is_active {
                button.fill(accent)
            } else {
                button.fill(Color32::TRANSPARENT)
            };

            if ui.add(button).clicked() {
                match mode {
                    ViewMode::Single => {
                        app.quad_view = false;
                        app.dual_pane = false;
                        app.wall_mode = false;
                    }
                    ViewMode::Quad => {
                        app.quad_view = true;
                        app.dual_pane = false;
                        app.wall_mode = false;
                    }
                    ViewMode::Dual => {
                        app.quad_view = false;
                        app.dual_pane = true;
                        app.wall_mode = false;
                    }
                    ViewMode::Wall => {
                        if !app.wall_mode {
                            app.start_wall_mode();
                        }
                        app.quad_view = false;
                        app.dual_pane = false;
                    }
                }
                app.needs_render = true;
            }
        }
    }
}
