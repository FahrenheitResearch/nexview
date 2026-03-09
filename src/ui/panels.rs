use egui::{self, Ui};
use crate::app::RadarApp;
use crate::nexrad::{RadarProduct, sites::RADAR_SITES};

pub struct SidePanel;

impl SidePanel {
    pub fn show(app: &mut RadarApp, ctx: &egui::Context) {
        egui::SidePanel::left("side_panel")
            .default_width(260.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("NexView");
                ui.separator();

                // Station selector
                Self::station_selector(app, ui);
                ui.separator();

                // Product selector
                Self::product_selector(app, ui);
                ui.separator();

                // Elevation selector
                Self::elevation_selector(app, ui);
                ui.separator();

                // View mode
                Self::view_controls(app, ui);
                ui.separator();

                // Data controls
                Self::data_controls(app, ui);
                ui.separator();

                // Info panel
                Self::info_panel(app, ui);

                // Color bar at the bottom
                ui.separator();
                Self::color_bar(app, ui);
            });
    }

    fn station_selector(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("Radar Site:");

        // Search/filter
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut app.station_filter);
        });

        let filter = app.station_filter.to_uppercase();

        egui::ScrollArea::vertical()
            .max_height(200.0)
            .id_salt("station_list")
            .show(ui, |ui| {
                for site in RADAR_SITES.iter() {
                    if !filter.is_empty()
                        && !site.id.contains(&filter)
                        && !site.name.to_uppercase().contains(&filter)
                        && !site.state.contains(&filter)
                    {
                        continue;
                    }

                    let label = format!("{} - {} ({})", site.id, site.name, site.state);
                    let selected = app.selected_station == site.id;

                    if ui.selectable_label(selected, &label).clicked() {
                        app.select_station(site.id);
                    }
                }
            });
    }

    fn product_selector(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("Product:");
        for product in RadarProduct::all_products() {
            let selected = app.selected_product == *product;
            if ui.selectable_label(selected, product.display_name()).clicked() {
                app.selected_product = *product;
                app.needs_render = true;
            }
        }
    }

    fn elevation_selector(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("Elevation:");
        if let Some(ref file) = app.current_file {
            for (i, sweep) in file.sweeps.iter().enumerate() {
                let label = format!("{:.1}°", sweep.elevation_angle);
                let selected = app.selected_elevation == i;
                if ui.selectable_label(selected, &label).clicked() {
                    app.selected_elevation = i;
                    app.needs_render = true;
                }
            }
        } else {
            ui.label("No data loaded");
        }
    }

    fn view_controls(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("View:");
        ui.checkbox(&mut app.quad_view, "Quad View (4 products)");

        if ui.button("Save as Default").clicked() {
            app.save_as_default();
        }
    }

    fn data_controls(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("Data:");

        if ui.button("Load Latest").clicked() {
            app.fetch_latest();
        }

        if let Some(progress) = app.fetcher.get_progress() {
            ui.spinner();
            ui.label(&progress);
        }

        if app.fetcher.is_fetching() {
            ui.spinner();
            ui.label("Loading file list...");
        }

        // Show available files
        let files = app.fetcher.available_files.lock().unwrap().clone();
        if !files.is_empty() {
            ui.label(format!("{} files available:", files.len()));
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .id_salt("file_list")
                .show(ui, |ui| {
                    for file in &files {
                        let size_kb = file.size / 1024;
                        let label = format!("{} ({}KB)", file.display_name, size_kb);
                        if ui.button(&label).clicked() {
                            app.fetcher.download_file(&file.key);
                        }
                    }
                });
        }
    }

    fn info_panel(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("Info:");

        if let Some(ref file) = app.current_file {
            ui.label(format!("Station: {}", file.station_id));
            ui.label(format!("Sweeps: {}", file.sweeps.len()));

            if let Some(sweep) = file.sweeps.get(app.selected_elevation) {
                ui.label(format!("Elevation: {:.1}°", sweep.elevation_angle));
                ui.label(format!("Radials: {}", sweep.radials.len()));

                // Count available products
                let products: std::collections::HashSet<RadarProduct> = sweep.radials.iter()
                    .flat_map(|r| r.moments.iter().map(|m| m.product))
                    .collect();
                ui.label(format!("Products: {:?}",
                    products.iter().map(|p| p.short_name()).collect::<Vec<_>>()
                ));
            }
        }

        ui.label(format!("Map tiles cached: {}", app.tile_manager.cache_size()));
        ui.label(format!("Zoom: {:.1}", app.map_view.zoom));
        ui.label(format!("Cursor: {:.3}°, {:.3}°", app.cursor_lat, app.cursor_lon));

        // Performance stats
        ui.separator();
        ui.label("Performance:");

        let perf = &app.perf;

        ui.label(format!("FPS: {:.0}", perf.fps));

        if let Some(dl) = perf.download_time {
            let size_mb = perf.parse_file_size as f64 / 1024.0 / 1024.0;
            let mb_s = size_mb / dl.as_secs_f64();
            ui.label(format!("Download: {:.0}ms ({:.1}MB, {:.1}MB/s)",
                dl.as_secs_f64() * 1000.0, size_mb, mb_s));
        }

        if let Some(pt) = perf.parse_time {
            ui.label(format!("Parse: {:.1}ms ({:.1}KB)",
                pt.as_secs_f64() * 1000.0, perf.parse_file_size as f64 / 1024.0));
        }

        if let Some(rt) = perf.render_time {
            ui.label(format!("Render: {:.1}ms total", rt.as_secs_f64() * 1000.0));
        }

        if app.quad_view {
            let names = ["REF", "VEL", "ZDR", "CC"];
            for (i, name) in names.iter().enumerate() {
                if let Some(qt) = perf.render_quad_times[i] {
                    ui.label(format!("  {}: {:.1}ms", name, qt.as_secs_f64() * 1000.0));
                }
            }
        }

        ui.label(format!("Radials: {}, Gates: {}",
            perf.total_radials, perf.total_gates));
    }

    fn color_bar(app: &mut RadarApp, ui: &mut Ui) {
        let color_table = crate::render::ColorTable::for_product(app.selected_product);

        ui.label(format!("{} ({})", color_table.name, app.selected_product.unit()));

        let bar_height = 200.0;
        let bar_width = 20.0;

        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(bar_width + 50.0, bar_height),
            egui::Sense::hover(),
        );

        let painter = ui.painter();
        let steps = 50;

        for i in 0..steps {
            let t = 1.0 - (i as f32 / steps as f32);
            let value = color_table.min_value + t * (color_table.max_value - color_table.min_value);
            let color = color_table.color_for_value(value);

            let y_start = rect.top() + (i as f32 / steps as f32) * bar_height;
            let y_end = rect.top() + ((i + 1) as f32 / steps as f32) * bar_height;

            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), y_start),
                    egui::pos2(rect.left() + bar_width, y_end),
                ),
                0.0,
                egui::Color32::from_rgba_premultiplied(color[0], color[1], color[2], color[3]),
            );
        }

        // Labels
        let label_count = 6;
        for i in 0..=label_count {
            let t = 1.0 - (i as f32 / label_count as f32);
            let value = color_table.min_value + t * (color_table.max_value - color_table.min_value);
            let y = rect.top() + (i as f32 / label_count as f32) * bar_height;

            painter.text(
                egui::pos2(rect.left() + bar_width + 4.0, y),
                egui::Align2::LEFT_CENTER,
                format!("{:.0}", value),
                egui::FontId::proportional(10.0),
                egui::Color32::WHITE,
            );
        }
    }
}
