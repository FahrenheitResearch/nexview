use egui::{self, Ui};
use crate::app::RadarApp;
use crate::nexrad::{RadarProduct, sites::RADAR_SITES};
use crate::render::color_table::ColorTablePreset;
use crate::render::map::TileProvider;

#[derive(PartialEq, Clone, Copy)]
pub enum SidebarSection {
    Station,
    Date,
    Overlays,
    Tools,
    Performance,
}

pub struct CollapsibleSidebar;

impl CollapsibleSidebar {
    pub fn show(app: &mut RadarApp, ctx: &egui::Context) {
        let width = if app.sidebar_expanded { 280.0 } else { 44.0 };

        egui::SidePanel::left("sidebar")
            .exact_width(width)
            .resizable(false)
            .show(ctx, |ui| {
                if app.sidebar_expanded {
                    Self::draw_expanded(app, ui);
                } else {
                    Self::draw_collapsed(app, ui);
                }
            });
    }

    fn draw_collapsed(app: &mut RadarApp, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);

            let sections = [
                (SidebarSection::Station, "S", "Stations"),
                (SidebarSection::Date, "D", "Date / Archive"),
                (SidebarSection::Overlays, "O", "Overlays"),
                (SidebarSection::Tools, "T", "Tools"),
                (SidebarSection::Performance, "P", "Performance"),
            ];

            for (section, icon, tooltip) in sections {
                let is_active = app.sidebar_expanded && app.sidebar_section == section;
                let btn = egui::Button::new(
                    egui::RichText::new(icon).size(16.0).strong(),
                )
                .min_size(egui::vec2(32.0, 32.0))
                .fill(if is_active {
                    egui::Color32::from_rgb(0, 100, 120)
                } else {
                    egui::Color32::TRANSPARENT
                });

                let response = ui.add(btn);
                if response.clicked() {
                    if app.sidebar_expanded && app.sidebar_section == section {
                        app.sidebar_expanded = false;
                    } else {
                        app.sidebar_expanded = true;
                        app.sidebar_section = section;
                    }
                }
                if response.hovered() {
                    response.on_hover_text(tooltip);
                }
                ui.add_space(4.0);
            }
        });
    }

    fn draw_expanded(app: &mut RadarApp, ui: &mut egui::Ui) {
        // Use the full available height for the sidebar layout
        let available = ui.available_rect_before_wrap();
        let rail_width = 36.0;
        let separator_width = 6.0;

        // Icon rail: fixed-width strip on the left, full height
        let rail_rect = egui::Rect::from_min_size(
            available.min,
            egui::vec2(rail_width, available.height()),
        );
        let mut rail_ui = ui.new_child(egui::UiBuilder::new().max_rect(rail_rect));
        rail_ui.vertical(|ui| {
            ui.set_width(rail_width);
            ui.add_space(8.0);

            let sections = [
                (SidebarSection::Station, "S", "Stations"),
                (SidebarSection::Date, "D", "Date / Archive"),
                (SidebarSection::Overlays, "O", "Overlays"),
                (SidebarSection::Tools, "T", "Tools"),
                (SidebarSection::Performance, "P", "Performance"),
            ];

            for (section, icon, tooltip) in sections {
                let is_active = app.sidebar_section == section;
                let btn = egui::Button::new(
                    egui::RichText::new(icon).size(16.0).strong(),
                )
                .min_size(egui::vec2(32.0, 32.0))
                .fill(if is_active {
                    egui::Color32::from_rgb(0, 100, 120)
                } else {
                    egui::Color32::TRANSPARENT
                });

                let response = ui.add(btn);
                if response.clicked() {
                    if app.sidebar_section == section {
                        app.sidebar_expanded = false;
                    } else {
                        app.sidebar_section = section;
                    }
                }
                if response.hovered() {
                    response.on_hover_text(tooltip);
                }
                ui.add_space(4.0);
            }
        });

        // Separator line between rail and content
        let sep_x = available.left() + rail_width;
        ui.painter().line_segment(
            [egui::pos2(sep_x, available.top()), egui::pos2(sep_x, available.bottom())],
            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
        );

        // Content area: remaining width, full height
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(available.left() + rail_width + separator_width, available.top()),
            available.max,
        );
        let mut content_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
        content_ui.vertical(|ui| {
            // Header with section name and collapse button
            ui.horizontal(|ui| {
                let section_name = match app.sidebar_section {
                    SidebarSection::Station => "Stations",
                    SidebarSection::Date => "Date / Archive",
                    SidebarSection::Overlays => "Overlays",
                    SidebarSection::Tools => "Tools",
                    SidebarSection::Performance => "Performance",
                };
                ui.strong(section_name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("\u{25C0}").clicked() {
                        app.sidebar_expanded = false;
                    }
                });
            });
            ui.separator();

            // Single scroll area for all content, using remaining vertical space
            egui::ScrollArea::vertical()
                .id_salt("sidebar_content")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    match app.sidebar_section {
                        SidebarSection::Station => Self::station_section(app, ui),
                        SidebarSection::Date => Self::date_section(app, ui),
                        SidebarSection::Overlays => Self::overlays_section(app, ui),
                        SidebarSection::Tools => Self::tools_section(app, ui),
                        SidebarSection::Performance => Self::perf_section(app, ui),
                    }
                });
        });

        // Advance the parent UI cursor past the full area we used
        ui.allocate_rect(available, egui::Sense::hover());
    }

    // ---- Station section ----

    fn station_section(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("Radar Site:");

        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut app.station_filter);
        });

        let filter = app.station_filter.to_uppercase();

        egui::ScrollArea::vertical()
            .max_height(400.0)
            .id_salt("sidebar_station_list")
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

    // ---- Date / Archive section ----

    fn date_section(app: &mut RadarApp, ui: &mut Ui) {
        ui.label("Date (archived data):");

        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut app.date_year).range(2000..=2030).prefix("Y:"));
            ui.add(egui::DragValue::new(&mut app.date_month).range(1..=12).prefix("M:"));
            ui.add(egui::DragValue::new(&mut app.date_day).range(1..=31).prefix("D:"));
        });

        ui.horizontal(|ui| {
            if ui.button("Load Date").clicked() {
                app.fetch_for_date();
            }
            if ui.button("Latest").clicked() {
                let now = chrono::Utc::now();
                app.date_year = chrono::Datelike::year(&now);
                app.date_month = chrono::Datelike::month(&now);
                app.date_day = chrono::Datelike::day(&now);
                app.fetch_latest();
            }
        });

        ui.add_space(8.0);

        // Historic event presets
        egui::ComboBox::from_id_salt("sidebar_historic_events")
            .selected_text("Historic Events...")
            .show_ui(ui, |ui| {
                let events: &[(&str, &str, i32, u32, u32)] = &[
                    ("2013 Moore EF5", "KTLX", 2013, 5, 20),
                    ("2011 Joplin EF5", "KSGF", 2011, 5, 22),
                    ("2011 Tuscaloosa EF4", "KBMX", 2011, 4, 27),
                    ("2013 El Reno EF3", "KTLX", 2013, 5, 31),
                    ("1999 Bridge Creek F5", "KTLX", 1999, 5, 3),
                    ("2011 April 27 Super Outbreak", "KHTX", 2011, 4, 27),
                    ("2005 Hurricane Katrina", "KLIX", 2005, 8, 29),
                    ("2017 Hurricane Harvey", "KHGX", 2017, 8, 25),
                    ("2012 Hurricane Sandy", "KDIX", 2012, 10, 29),
                    ("2019 Nashville EF3", "KOHX", 2020, 3, 3),
                    ("2021 Quad State Tornado", "KPAH", 2021, 12, 11),
                    ("2024 Greenfield EF4", "KOAX", 2024, 4, 26),
                    ("2022 Rolling Fork EF4", "KDGX", 2023, 3, 24),
                    ("2025 Super Tuesday Outbreak", "KHTX", 2025, 1, 21),
                    ("2008 Super Tuesday Outbreak", "KHTX", 2008, 2, 5),
                    ("2011 Hackleburg EF5", "KGWX", 2011, 4, 27),
                ];
                for (name, station, year, month, day) in events {
                    if ui.button(format!("{} ({})", name, station)).clicked() {
                        app.date_year = *year;
                        app.date_month = *month;
                        app.date_day = *day;
                        app.selected_station = station.to_string();
                        if let Some(site) = crate::nexrad::sites::find_site(station) {
                            app.map_view.center_lat = site.lat;
                            app.map_view.center_lon = site.lon;
                        }
                        app.anim_frames.clear();
                        app.anim_frame_names.clear();
                        app.anim_playing = false;
                        app.anim_loading = false;
                        app.fetch_for_date();
                    }
                }
            });

        ui.add_space(8.0);
        ui.separator();

        // Data loading / file list (moved from data_controls)
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

        let files = app.fetcher.available_files.lock().unwrap().clone();
        if !files.is_empty() {
            ui.label(format!("{} files available:", files.len()));
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .id_salt("sidebar_file_list")
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

        ui.add_space(8.0);
        ui.separator();

        // Animation controls
        ui.label("Animation:");

        let available = app.fetcher.available_files.lock().unwrap().len();

        ui.horizontal(|ui| {
            ui.label("Frames:");
            let max = if available > 0 { available } else { 300 };
            ui.add(egui::DragValue::new(&mut app.anim_frame_count).range(3..=max));
            if available > 0 {
                if ui.button(format!("All ({})", available)).clicked() {
                    app.anim_frame_count = available;
                }
            }
        });

        ui.horizontal(|ui| {
            if app.anim_loading {
                ui.spinner();
                ui.label(format!(
                    "Loading {}/{}...",
                    app.anim_frames.len(),
                    app.anim_download_queue.len()
                ));
            } else if ui.button("Load Loop").clicked() {
                app.load_animation_frames();
            }
        });

        if !app.anim_frames.is_empty() {
            let mut frame_idx = app.anim_index as i32;
            let max_idx = (app.anim_frames.len() as i32 - 1).max(0);
            if ui
                .add(egui::Slider::new(&mut frame_idx, 0..=max_idx).show_value(false))
                .changed()
            {
                app.anim_playing = false;
                app.anim_index = frame_idx as usize;
                app.current_file = Some(app.anim_frames[app.anim_index].clone());
                app.needs_render = true;
            }

            ui.horizontal(|ui| {
                if ui
                    .button(if app.anim_playing { "Pause" } else { "Play" })
                    .clicked()
                {
                    app.anim_playing = !app.anim_playing;
                    if app.anim_playing {
                        app.anim_last_advance = Some(std::time::Instant::now());
                    }
                }

                if ui.button("<").clicked() {
                    app.anim_playing = false;
                    app.anim_index = if app.anim_index == 0 {
                        app.anim_frames.len() - 1
                    } else {
                        app.anim_index - 1
                    };
                    app.current_file = Some(app.anim_frames[app.anim_index].clone());
                    app.needs_render = true;
                }
                if ui.button(">").clicked() {
                    app.anim_playing = false;
                    app.anim_index = (app.anim_index + 1) % app.anim_frames.len();
                    app.current_file = Some(app.anim_frames[app.anim_index].clone());
                    app.needs_render = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Speed:");
                ui.add(egui::Slider::new(&mut app.anim_speed_ms, 50..=1000).suffix("ms"));
            });

            let name = app
                .anim_frame_names
                .get(app.anim_index)
                .map(|s| s.as_str())
                .unwrap_or("?");
            ui.label(format!(
                "Frame {}/{}: {}",
                app.anim_index + 1,
                app.anim_frames.len(),
                name
            ));

            ui.horizontal(|ui| {
                if ui.button("Export GIF").clicked() {
                    app.export_loop_gif();
                }
                if let Some(status) = &app.gif_export_status {
                    ui.label(status.as_str());
                }
            });
        }
    }

    // ---- Overlays section ----

    fn overlays_section(app: &mut RadarApp, ui: &mut Ui) {
        // Product selector
        ui.label("Product:");
        for product in RadarProduct::all_products() {
            let selected = app.selected_product == *product;
            if ui
                .selectable_label(selected, product.display_name())
                .clicked()
            {
                if *product == RadarProduct::StormRelativeVelocity
                    && app.selected_product != RadarProduct::StormRelativeVelocity
                {
                    app.estimate_storm_motion();
                }
                app.selected_product = *product;
                app.mark_all_needs_render();
            }
        }

        ui.add_space(4.0);
        ui.separator();

        // Elevation selector
        ui.label("Elevation:");
        if let Some(ref file) = app.current_file {
            for (i, sweep) in file.sweeps.iter().enumerate() {
                let label = format!("{:.1}\u{b0}", sweep.elevation_angle);
                let selected = app.selected_elevation == i;
                if ui.selectable_label(selected, &label).clicked() {
                    app.selected_elevation = i;
                    app.needs_render = true;
                }
            }
        } else {
            ui.label("No data loaded");
        }

        ui.add_space(4.0);
        ui.separator();

        // View mode
        ui.label("View:");
        if ui
            .checkbox(&mut app.quad_view, "Quad View (4 products)")
            .changed()
        {
            if app.quad_view {
                app.dual_pane = false; // mutually exclusive
            }
            app.needs_render = true;
        }
        if ui
            .checkbox(&mut app.dual_pane, "Dual Pane (side-by-side)")
            .changed()
        {
            if app.dual_pane {
                app.quad_view = false; // mutually exclusive
            }
            app.needs_render = true;
        }
        if app.dual_pane {
            ui.horizontal(|ui| {
                ui.label("Right pane:");
                egui::ComboBox::from_id_salt("sidebar_dual_pane_product")
                    .selected_text(app.dual_pane_product.display_name())
                    .show_ui(ui, |ui| {
                        for product in RadarProduct::all_products() {
                            if ui
                                .selectable_label(
                                    app.dual_pane_product == *product,
                                    product.display_name(),
                                )
                                .clicked()
                            {
                                app.dual_pane_product = *product;
                                app.needs_render = true;
                            }
                        }
                    });
            });
        }

        if ui
            .button(if app.wall_mode {
                "Exit Wall Mode"
            } else {
                "Wall Mode (20 radars)"
            })
            .clicked()
        {
            if app.wall_mode {
                app.wall_mode = false;
            } else {
                app.start_wall_mode();
            }
        }

        ui.add_space(4.0);
        ui.separator();

        // Map style
        ui.horizontal(|ui| {
            ui.label("Map:");
            let current = app.tile_manager.provider();
            egui::ComboBox::from_id_salt("sidebar_map_style")
                .selected_text(current.label())
                .show_ui(ui, |ui| {
                    for provider in TileProvider::all() {
                        if ui
                            .selectable_label(current == *provider, provider.label())
                            .clicked()
                        {
                            app.set_tile_provider(*provider);
                        }
                    }
                });
        });

        // Color table preset
        ui.horizontal(|ui| {
            ui.label("Colors:");
            egui::ComboBox::from_id_salt("sidebar_color_preset")
                .selected_text(app.color_preset.label())
                .show_ui(ui, |ui| {
                    for preset in ColorTablePreset::all() {
                        if ui
                            .selectable_label(app.color_preset == *preset, preset.label())
                            .clicked()
                        {
                            app.color_preset = *preset;
                            app.mark_all_needs_render();
                        }
                    }
                });
        });

        // GPU rendering toggle
        if app.gpu_renderer.is_some() {
            if ui
                .checkbox(&mut app.gpu_rendering, "GPU Rendering")
                .changed()
            {
                app.needs_render = true;
            }
        }

        ui.add_space(4.0);
        ui.separator();

        // Opacity
        ui.label("Opacity:");
        ui.horizontal(|ui| {
            ui.label("Radar:");
            ui.add(egui::Slider::new(&mut app.radar_opacity, 0.0..=1.0).step_by(0.01));
        });
        ui.horizontal(|ui| {
            ui.label("Map:");
            ui.add(egui::Slider::new(&mut app.map_opacity, 0.0..=1.0).step_by(0.01));
        });
        ui.checkbox(&mut app.dark_mode, "Dark Mode");

        ui.add_space(4.0);
        ui.separator();

        // Overlay toggles
        ui.label("Overlays:");
        ui.checkbox(&mut app.show_range_rings, "Range Rings");
        ui.checkbox(&mut app.show_azimuth_lines, "Azimuth Lines");
        ui.checkbox(&mut app.show_cities, "City Labels");
        ui.checkbox(&mut app.show_warnings, "NWS Warnings");
        ui.checkbox(&mut app.show_detections, "Meso/TVS Detection");

        if ui
            .checkbox(&mut app.sounding_mode, "Sounding Mode (click map)")
            .changed()
        {
            if !app.sounding_mode {
                app.sounding_texture = None;
            }
        }

        ui.add_space(8.0);
        ui.separator();

        // Color bar
        Self::color_bar(app, ui);
    }

    // ---- Tools section ----

    fn tools_section(app: &mut RadarApp, ui: &mut Ui) {
        // Measure distance
        ui.label("Measure Distance:");
        let measure_label = if app.measure_mode {
            if app.measure_start.is_none() {
                "Measuring... (click start)"
            } else {
                "Measuring... (click end)"
            }
        } else {
            "Measure Distance (M)"
        };
        if ui.button(measure_label).clicked() {
            app.measure_mode = true;
            app.measure_start = None;
            app.measure_end = None;
        }
        if app.measure_start.is_some() && app.measure_end.is_some() {
            if ui.button("Clear Measurement").clicked() {
                app.measure_start = None;
                app.measure_end = None;
            }
        }

        ui.add_space(8.0);
        ui.separator();

        // Cross section
        ui.label("Cross Section:");

        let mode_label = if app.cross_section_mode {
            if app.cross_section_start.is_some() {
                "Click end point..."
            } else {
                "Click start point..."
            }
        } else {
            "Draw Cross Section"
        };

        if ui.button(mode_label).clicked() {
            app.cross_section_mode = !app.cross_section_mode;
            if app.cross_section_mode {
                app.cross_section_start = None;
                app.cross_section_end = None;
            }
        }

        if app.cross_section_start.is_some() && app.cross_section_end.is_some() {
            if ui.button("Clear").clicked() {
                app.cross_section_start = None;
                app.cross_section_end = None;
                app.cross_section_texture = None;
            }
        }

        ui.add_space(8.0);
        ui.separator();

        // Storm motion
        egui::CollapsingHeader::new("Storm Motion")
            .default_open(false)
            .show(ui, |ui| {
                let is_srv = app.selected_product == RadarProduct::StormRelativeVelocity;

                let dir_response = ui.add(
                    egui::Slider::new(&mut app.storm_motion_dir, 0.0..=360.0)
                        .text("Dir")
                        .suffix("\u{b0}"),
                );
                let spd_response = ui.add(
                    egui::Slider::new(&mut app.storm_motion_speed, 0.0..=80.0)
                        .text("Speed")
                        .suffix(" kts"),
                );

                if is_srv && (dir_response.changed() || spd_response.changed()) {
                    app.needs_render = true;
                }

                if ui.button("Auto-estimate").clicked() {
                    app.estimate_storm_motion();
                    if is_srv {
                        app.needs_render = true;
                    }
                }
            });

        ui.add_space(8.0);
        ui.separator();

        if ui.button("Save as Default").clicked() {
            app.save_as_default();
        }
    }

    // ---- Performance section ----

    fn perf_section(app: &mut RadarApp, ui: &mut Ui) {
        if let Some(ref file) = app.current_file {
            ui.label(format!("Station: {}", file.station_id));
            ui.label(format!("Sweeps: {}", file.sweeps.len()));

            if let Some(sweep) = file.sweeps.get(app.selected_elevation) {
                ui.label(format!("Elevation: {:.1}\u{b0}", sweep.elevation_angle));
                ui.label(format!("Radials: {}", sweep.radials.len()));

                let products: std::collections::HashSet<RadarProduct> = sweep
                    .radials
                    .iter()
                    .flat_map(|r| r.moments.iter().map(|m| m.product))
                    .collect();
                ui.label(format!(
                    "Products: {:?}",
                    products.iter().map(|p| p.short_name()).collect::<Vec<_>>()
                ));
            }
        }

        ui.label(format!("Map tiles cached: {}", app.tile_manager.cache_size()));
        ui.label(format!("Zoom: {:.1}", app.map_view.zoom));
        ui.label(format!(
            "Cursor: {:.3}\u{b0}, {:.3}\u{b0}",
            app.cursor_lat, app.cursor_lon
        ));

        ui.add_space(4.0);
        ui.separator();
        ui.label("Performance:");

        let perf = &app.perf;

        ui.label(format!("FPS: {:.0}", perf.fps));

        if let Some(dl) = perf.download_time {
            let size_mb = perf.parse_file_size as f64 / 1024.0 / 1024.0;
            let mb_s = size_mb / dl.as_secs_f64();
            ui.label(format!(
                "Download: {:.0}ms ({:.1}MB, {:.1}MB/s)",
                dl.as_secs_f64() * 1000.0,
                size_mb,
                mb_s
            ));
        }

        if let Some(pt) = perf.parse_time {
            ui.label(format!(
                "Parse: {:.1}ms ({:.1}KB)",
                pt.as_secs_f64() * 1000.0,
                perf.parse_file_size as f64 / 1024.0
            ));
        }

        if let Some(rt) = perf.render_time {
            ui.label(format!(
                "Render: {:.1}ms total",
                rt.as_secs_f64() * 1000.0
            ));
        }

        if app.quad_view {
            let names = ["REF", "VEL", "ZDR", "CC"];
            for (i, name) in names.iter().enumerate() {
                if let Some(qt) = perf.render_quad_times[i] {
                    ui.label(format!("  {}: {:.1}ms", name, qt.as_secs_f64() * 1000.0));
                }
            }
        }

        ui.label(format!(
            "Radials: {}, Gates: {}",
            perf.total_radials, perf.total_gates
        ));
    }

    // ---- Color bar ----

    fn color_bar(app: &mut RadarApp, ui: &mut Ui) {
        let color_table =
            crate::render::ColorTable::for_product_preset(app.selected_product, app.color_preset);

        ui.label(format!(
            "{} ({})",
            color_table.name,
            app.selected_product.unit()
        ));

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
                egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
            );
        }

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
