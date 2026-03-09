use egui::{self};
use crate::app::RadarApp;

pub struct ControlBar;

impl ControlBar {
    pub fn show(app: &mut RadarApp, ctx: &egui::Context) {
        egui::TopBottomPanel::top("control_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Station:");
                let station_label = format!("{}", app.selected_station);
                ui.strong(&station_label);

                ui.separator();

                ui.label("Product:");
                ui.strong(app.selected_product.short_name());

                ui.separator();

                if let Some(ref file) = app.current_file {
                    if let Some(sweep) = file.sweeps.get(app.selected_elevation) {
                        ui.label(format!("Elev: {:.1}°", sweep.elevation_angle));
                    }
                }

                ui.separator();

                // Tilt up/down buttons
                if ui.button("▲ Tilt Up").clicked() {
                    if let Some(ref file) = app.current_file {
                        if app.selected_elevation + 1 < file.sweeps.len() {
                            app.selected_elevation += 1;
                            app.needs_render = true;
                        }
                    }
                }
                if ui.button("▼ Tilt Down").clicked() {
                    if app.selected_elevation > 0 {
                        app.selected_elevation -= 1;
                        app.needs_render = true;
                    }
                }

                ui.separator();

                // Product quick-select
                if ui.button("REF").clicked() {
                    app.selected_product = crate::nexrad::RadarProduct::Reflectivity;
                    app.needs_render = true;
                }
                if ui.button("VEL").clicked() {
                    app.selected_product = crate::nexrad::RadarProduct::Velocity;
                    app.needs_render = true;
                }
                if ui.button("SW").clicked() {
                    app.selected_product = crate::nexrad::RadarProduct::SpectrumWidth;
                    app.needs_render = true;
                }
                if ui.button("ZDR").clicked() {
                    app.selected_product = crate::nexrad::RadarProduct::DifferentialReflectivity;
                    app.needs_render = true;
                }
                if ui.button("CC").clicked() {
                    app.selected_product = crate::nexrad::RadarProduct::CorrelationCoefficient;
                    app.needs_render = true;
                }
                if ui.button("KDP").clicked() {
                    app.selected_product = crate::nexrad::RadarProduct::SpecificDiffPhase;
                    app.needs_render = true;
                }

                ui.separator();
                ui.checkbox(&mut app.quad_view, "Quad");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.4}°N {:.4}°W",
                        app.cursor_lat, -app.cursor_lon));
                });
            });
        });
    }
}
