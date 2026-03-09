use eframe::egui;
use std::time::{Duration, Instant};
use crate::nexrad::{Level2File, RadarProduct, sites};
use crate::render::{RadarRenderer, MapTileManager};
use crate::render::map::MapView;
use crate::data::NexradFetcher;
use crate::ui::{SidePanel, ControlBar};

const SETTINGS_PATH: &str = "nexview_settings.json";

/// Quad-view panel products
pub const QUAD_PRODUCTS: [RadarProduct; 4] = [
    RadarProduct::Reflectivity,
    RadarProduct::Velocity,
    RadarProduct::DifferentialReflectivity,
    RadarProduct::CorrelationCoefficient,
];

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AppSettings {
    pub default_station: String,
    pub default_zoom: f64,
    pub quad_view: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_station: "KTLX".into(),
            default_zoom: 7.0,
            quad_view: true,
        }
    }
}

impl AppSettings {
    fn load() -> Self {
        let path = std::path::Path::new(SETTINGS_PATH);
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(settings) = serde_json::from_str(&data) {
                    return settings;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(SETTINGS_PATH, data);
        }
    }
}

pub struct RadarApp {
    // Data
    pub current_file: Option<Level2File>,
    pub selected_station: String,
    pub selected_product: RadarProduct,
    pub selected_elevation: usize,
    pub station_filter: String,

    // Rendering - quad view textures
    pub quad_textures: [Option<egui::TextureHandle>; 4],
    pub single_texture: Option<egui::TextureHandle>,
    pub needs_render: bool,
    pub quad_view: bool,
    pub map_view: MapView,
    pub tile_manager: MapTileManager,
    pub tile_textures: std::collections::HashMap<crate::render::map::TileKey, egui::TextureHandle>,

    // Data fetching
    pub fetcher: NexradFetcher,

    // Interaction
    pub cursor_lat: f64,
    pub cursor_lon: f64,

    // Settings
    pub settings: AppSettings,

    // Performance stats
    pub perf: PerfStats,

    // Runtime
    runtime: tokio::runtime::Runtime,
}

#[derive(Default)]
pub struct PerfStats {
    pub parse_time: Option<Duration>,
    pub parse_file_size: usize,
    pub render_time: Option<Duration>,
    pub render_quad_times: [Option<Duration>; 4],
    pub download_time: Option<Duration>,
    pub decompress_time: Option<Duration>,
    pub total_radials: usize,
    pub total_gates: usize,
    pub frame_times: Vec<Duration>,
    pub last_frame_start: Option<Instant>,
    pub fps: f64,
}

impl RadarApp {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let handle = runtime.handle().clone();

        let settings = AppSettings::load();

        let fetcher = NexradFetcher::new(handle.clone());
        let tile_manager = MapTileManager::new(handle);

        let station = settings.default_station.clone();
        let zoom = settings.default_zoom;
        let quad = settings.quad_view;

        let (center_lat, center_lon) = sites::find_site(&station)
            .map(|s| (s.lat, s.lon))
            .unwrap_or((35.333, -97.278));

        let mut app = Self {
            current_file: None,
            selected_station: station,
            selected_product: RadarProduct::Reflectivity,
            selected_elevation: 0,
            station_filter: String::new(),

            quad_textures: [None, None, None, None],
            single_texture: None,
            needs_render: false,
            quad_view: quad,
            map_view: MapView {
                center_lat,
                center_lon,
                zoom,
            },
            tile_manager,
            tile_textures: std::collections::HashMap::new(),

            fetcher,

            cursor_lat: 0.0,
            cursor_lon: 0.0,

            settings: AppSettings::load(),

            perf: PerfStats::default(),

            runtime,
        };

        // Auto-load latest data on startup
        app.fetch_latest();
        app
    }

    pub fn select_station(&mut self, station_id: &str) {
        self.selected_station = station_id.to_string();
        if let Some(site) = sites::find_site(station_id) {
            self.map_view.center_lat = site.lat;
            self.map_view.center_lon = site.lon;
        }
        self.fetch_latest();
    }

    pub fn save_as_default(&mut self) {
        self.settings.default_station = self.selected_station.clone();
        self.settings.default_zoom = self.map_view.zoom;
        self.settings.quad_view = self.quad_view;
        self.settings.save();
        log::info!("Saved default settings: station={}", self.selected_station);
    }

    pub fn fetch_latest(&mut self) {
        self.fetcher.list_recent_files(&self.selected_station);
    }

    /// Find the best sweep for a given product.
    /// NEXRAD splits products across different sweeps at the same elevation.
    pub fn find_sweep_for_product(&self, product: RadarProduct) -> Option<usize> {
        let file = self.current_file.as_ref()?;

        // First, try to find a sweep at the selected elevation that has this product
        if let Some(sweep) = file.sweeps.get(self.selected_elevation) {
            let has_product = sweep.radials.iter().any(|r| {
                r.moments.iter().any(|m| m.product == product)
            });
            if has_product {
                return Some(self.selected_elevation);
            }
        }

        // Otherwise, find the lowest elevation sweep that has this product
        for (i, sweep) in file.sweeps.iter().enumerate() {
            let has_product = sweep.radials.iter().any(|r| {
                r.moments.iter().any(|m| m.product == product)
            });
            if has_product {
                return Some(i);
            }
        }
        None
    }

    fn check_downloads(&mut self, ctx: &egui::Context) {
        if let Some(data) = self.fetcher.take_downloaded_data() {
            let file_size = data.len();
            let parse_start = Instant::now();
            match Level2File::parse(&data) {
                Ok(file) => {
                    let parse_time = parse_start.elapsed();
                    log::info!(
                        "Parsed Level2 file: station={}, sweeps={}, parse={:.1}ms, size={}KB",
                        file.station_id,
                        file.sweeps.len(),
                        parse_time.as_secs_f64() * 1000.0,
                        file_size / 1024,
                    );

                    // Collect stats
                    let total_radials: usize = file.sweeps.iter().map(|s| s.radials.len()).sum();
                    let total_gates: usize = file.sweeps.iter()
                        .flat_map(|s| s.radials.iter())
                        .flat_map(|r| r.moments.iter())
                        .map(|m| m.gate_count as usize)
                        .sum();

                    self.perf.parse_time = Some(parse_time);
                    self.perf.parse_file_size = file_size;
                    self.perf.total_radials = total_radials;
                    self.perf.total_gates = total_gates;

                    self.current_file = Some(file);
                    self.selected_elevation = 0;
                    self.needs_render = true;
                }
                Err(e) => {
                    log::error!("Failed to parse Level2 file: {}", e);
                }
            }
        }

        // Track download time from fetcher
        if let Some(dl_time) = self.fetcher.take_download_time() {
            self.perf.download_time = Some(dl_time);
        }

        ctx.request_repaint();
    }

    fn render_radar(&mut self, ctx: &egui::Context) {
        if !self.needs_render {
            return;
        }
        self.needs_render = false;

        let render_start = Instant::now();

        let file = match &self.current_file {
            Some(f) => f,
            None => return,
        };

        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        if self.quad_view {
            // Render all 4 quad products
            for (i, &product) in QUAD_PRODUCTS.iter().enumerate() {
                let sweep_idx = self.find_sweep_for_product(product);
                let sweep = sweep_idx.and_then(|idx| file.sweeps.get(idx));

                if let Some(sweep) = sweep {
                    let t0 = Instant::now();
                    let rendered = RadarRenderer::render_sweep(sweep, product, site, 512);
                    self.perf.render_quad_times[i] = Some(t0.elapsed());

                    if let Some(rendered) = rendered {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [rendered.width as usize, rendered.height as usize],
                            &rendered.pixels,
                        );
                        self.quad_textures[i] = Some(ctx.load_texture(
                            format!("radar_quad_{}", i),
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    } else {
                        self.quad_textures[i] = None;
                    }
                } else {
                    self.quad_textures[i] = None;
                    self.perf.render_quad_times[i] = None;
                }
            }
        }

        // Always render the selected single product too (for single view / overlay)
        let sweep_idx = self.find_sweep_for_product(self.selected_product)
            .unwrap_or(self.selected_elevation);
        if let Some(sweep) = file.sweeps.get(sweep_idx) {
            let rendered = RadarRenderer::render_sweep(sweep, self.selected_product, site, 1024);
            if let Some(rendered) = rendered {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [rendered.width as usize, rendered.height as usize],
                    &rendered.pixels,
                );
                self.single_texture = Some(ctx.load_texture(
                    "radar_single",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            } else {
                self.single_texture = None;
            }
        }

        self.perf.render_time = Some(render_start.elapsed());
        log::info!("Render total: {:.1}ms", render_start.elapsed().as_secs_f64() * 1000.0);
    }

    fn draw_map(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let visible = self.map_view.visible_tiles(screen_w, screen_h);
        for key in &visible {
            self.tile_manager.request_tile(*key);

            if let Some(tile_data) = self.tile_manager.get_tile(key) {
                let tex = self.tile_textures.entry(*key).or_insert_with(|| {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [tile_data.width as usize, tile_data.height as usize],
                        &tile_data.pixels,
                    );
                    ui.ctx().load_texture(
                        format!("tile_{}_{}_{}", key.z, key.x, key.y),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                });

                let (px, py) = self.map_view.tile_screen_pos(key, screen_w, screen_h);
                let tile_size = self.map_view.tile_size_on_screen(key.z) as f32;

                let tile_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + px as f32, rect.top() + py as f32),
                    egui::vec2(tile_size, tile_size),
                );

                if tile_rect.intersects(rect) {
                    ui.painter().image(
                        tex.id(),
                        tile_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
        }

        let visible_set: std::collections::HashSet<_> = visible.iter().copied().collect();
        self.tile_textures.retain(|k, _| visible_set.contains(k));
    }

    fn get_radar_rect(&self, rect: egui::Rect, product: RadarProduct) -> Option<egui::Rect> {
        let file = self.current_file.as_ref()?;
        let site = sites::find_site(&self.selected_station)?;
        let sweep_idx = self.find_sweep_for_product(product)?;
        let sweep = file.sweeps.get(sweep_idx)?;

        let max_range_m = sweep.radials.iter()
            .filter_map(|r| {
                r.moments.iter()
                    .filter(|m| m.product == product)
                    .map(|m| m.first_gate_range as f64 + m.gate_count as f64 * m.gate_size as f64)
                    .next()
            })
            .fold(0.0f64, f64::max);

        if max_range_m <= 0.0 {
            return None;
        }

        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let range_deg_lat = max_range_m / 111139.0;
        let range_deg_lon = max_range_m / (111139.0 * site.lat.to_radians().cos());

        let (top_x, top_y) = self.map_view.lat_lon_to_pixel(
            site.lat + range_deg_lat, site.lon - range_deg_lon,
            screen_w, screen_h,
        );
        let (bot_x, bot_y) = self.map_view.lat_lon_to_pixel(
            site.lat - range_deg_lat, site.lon + range_deg_lon,
            screen_w, screen_h,
        );

        Some(egui::Rect::from_min_max(
            egui::pos2(rect.left() + top_x as f32, rect.top() + top_y as f32),
            egui::pos2(rect.left() + bot_x as f32, rect.top() + bot_y as f32),
        ))
    }

    fn draw_radar_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let tex = match &self.single_texture {
            Some(t) => t,
            None => return,
        };

        let radar_rect = match self.get_radar_rect(rect, self.selected_product) {
            Some(r) => r,
            None => return,
        };

        ui.painter().image(
            tex.id(),
            radar_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::from_white_alpha(220),
        );

        // Draw radar site marker
        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;
        let (cx, cy) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
        let marker_pos = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);
        ui.painter().circle_filled(marker_pos, 4.0, egui::Color32::WHITE);
        ui.painter().circle_stroke(marker_pos, 4.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
        ui.painter().text(
            marker_pos + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            &self.selected_station,
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }

    fn draw_quad_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let half_w = rect.width() / 2.0;
        let half_h = rect.height() / 2.0;

        let rects = [
            egui::Rect::from_min_size(rect.min, egui::vec2(half_w, half_h)),
            egui::Rect::from_min_size(egui::pos2(rect.left() + half_w, rect.top()), egui::vec2(half_w, half_h)),
            egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + half_h), egui::vec2(half_w, half_h)),
            egui::Rect::from_min_size(egui::pos2(rect.left() + half_w, rect.top() + half_h), egui::vec2(half_w, half_h)),
        ];

        for (i, &product) in QUAD_PRODUCTS.iter().enumerate() {
            let quad_rect = rects[i];

            // Draw map tiles in this quadrant
            self.draw_map_in_rect(ui, quad_rect);

            // Draw radar overlay
            if let Some(tex) = &self.quad_textures[i] {
                if let Some(radar_rect) = self.get_radar_rect(quad_rect, product) {
                    ui.painter().image(
                        tex.id(),
                        radar_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::from_white_alpha(220),
                    );
                }
            }

            // Label
            ui.painter().rect_filled(
                egui::Rect::from_min_size(quad_rect.min, egui::vec2(80.0, 22.0)),
                4.0,
                egui::Color32::from_black_alpha(180),
            );
            ui.painter().text(
                quad_rect.min + egui::vec2(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                product.short_name(),
                egui::FontId::proportional(14.0),
                egui::Color32::WHITE,
            );

            // Border between quadrants
            ui.painter().rect_stroke(quad_rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_gray(60)), egui::StrokeKind::Outside);
        }
    }

    fn draw_map_in_rect(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let visible = self.map_view.visible_tiles(screen_w, screen_h);
        for key in &visible {
            if let Some(tex) = self.tile_textures.get(key) {
                let (px, py) = self.map_view.tile_screen_pos(key, screen_w, screen_h);
                let tile_size = self.map_view.tile_size_on_screen(key.z) as f32;

                let tile_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + px as f32, rect.top() + py as f32),
                    egui::vec2(tile_size, tile_size),
                );

                if tile_rect.intersects(rect) {
                    ui.painter().image(
                        tex.id(),
                        tile_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
        }
    }

    fn draw_radar_sites(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        if self.map_view.zoom < 5.0 {
            return;
        }

        for site in sites::RADAR_SITES.iter() {
            let (px, py) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
            let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);

            if !rect.contains(pos) {
                continue;
            }

            let is_selected = site.id == self.selected_station;
            let color = if is_selected {
                egui::Color32::from_rgb(255, 255, 0)
            } else {
                egui::Color32::from_rgb(200, 200, 200)
            };

            let radius = if is_selected { 5.0 } else { 3.0 };
            ui.painter().circle_filled(pos, radius, color);

            if self.map_view.zoom >= 7.0 || is_selected {
                ui.painter().text(
                    pos + egui::vec2(6.0, -6.0),
                    egui::Align2::LEFT_BOTTOM,
                    site.id,
                    egui::FontId::proportional(10.0),
                    color,
                );
            }
        }
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let products = RadarProduct::all_products();

        ctx.input(|i| {
            // Up/Down arrows: tilt up/down
            if i.key_pressed(egui::Key::ArrowUp) {
                if let Some(ref file) = self.current_file {
                    if self.selected_elevation + 1 < file.sweeps.len() {
                        self.selected_elevation += 1;
                        self.needs_render = true;
                    }
                }
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                if self.selected_elevation > 0 {
                    self.selected_elevation -= 1;
                    self.needs_render = true;
                }
            }

            // Left/Right arrows: cycle products
            if i.key_pressed(egui::Key::ArrowRight) {
                if let Some(idx) = products.iter().position(|&p| p == self.selected_product) {
                    let next = (idx + 1) % products.len();
                    self.selected_product = products[next];
                    self.needs_render = true;
                }
            }
            if i.key_pressed(egui::Key::ArrowLeft) {
                if let Some(idx) = products.iter().position(|&p| p == self.selected_product) {
                    let prev = if idx == 0 { products.len() - 1 } else { idx - 1 };
                    self.selected_product = products[prev];
                    self.needs_render = true;
                }
            }

            // Q: toggle quad view
            if i.key_pressed(egui::Key::Q) {
                self.quad_view = !self.quad_view;
                self.needs_render = true;
            }
        });
    }

    fn handle_interaction(&mut self, response: &egui::Response, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        if let Some(pos) = response.hover_pos() {
            let rel_x = (pos.x - rect.left()) as f64;
            let rel_y = (pos.y - rect.top()) as f64;
            let (lat, lon) = self.map_view.pixel_to_lat_lon(rel_x, rel_y, screen_w, screen_h);
            self.cursor_lat = lat;
            self.cursor_lon = lon;
        }

        let scroll = response.ctx.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 && response.hovered() {
            if let Some(pos) = response.hover_pos() {
                let rel_x = (pos.x - rect.left()) as f64;
                let rel_y = (pos.y - rect.top()) as f64;
                let delta = scroll as f64 * 0.003;
                self.map_view.zoom_at(delta, rel_x, rel_y, screen_w, screen_h);
            }
        }

        if response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            self.map_view.pan(delta.x as f64, delta.y as f64, screen_w, screen_h);
        }

        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let click_x = (pos.x - rect.left()) as f64;
                let click_y = (pos.y - rect.top()) as f64;

                for site in sites::RADAR_SITES.iter() {
                    let (sx, sy) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
                    let dist = ((click_x - sx).powi(2) + (click_y - sy).powi(2)).sqrt();
                    if dist < 10.0 {
                        self.select_station(site.id);
                        break;
                    }
                }
            }
        }
    }
}

impl eframe::App for RadarApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // FPS tracking
        let frame_start = Instant::now();
        if let Some(last) = self.perf.last_frame_start {
            let dt = last.elapsed();
            self.perf.frame_times.push(dt);
            if self.perf.frame_times.len() > 60 {
                self.perf.frame_times.remove(0);
            }
            if !self.perf.frame_times.is_empty() {
                let avg: f64 = self.perf.frame_times.iter()
                    .map(|d| d.as_secs_f64())
                    .sum::<f64>() / self.perf.frame_times.len() as f64;
                self.perf.fps = if avg > 0.0 { 1.0 / avg } else { 0.0 };
            }
        }
        self.perf.last_frame_start = Some(frame_start);

        self.check_downloads(ctx);
        self.handle_keyboard(ctx);
        self.render_radar(ctx);

        ControlBar::show(self, ctx);
        SidePanel::show(self, ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_rgb(20, 20, 30)))
            .show(ctx, |ui| {
                let available_rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(available_rect, egui::Sense::click_and_drag());

                ui.painter().rect_filled(available_rect, 0.0, egui::Color32::from_rgb(20, 20, 30));

                if self.quad_view {
                    // Pre-fetch tiles for all quadrants
                    let screen_w = (available_rect.width() / 2.0) as f64;
                    let screen_h = (available_rect.height() / 2.0) as f64;
                    let visible = self.map_view.visible_tiles(screen_w, screen_h);
                    for key in &visible {
                        self.tile_manager.request_tile(*key);
                        if let Some(tile_data) = self.tile_manager.get_tile(key) {
                            self.tile_textures.entry(*key).or_insert_with(|| {
                                let image = egui::ColorImage::from_rgba_unmultiplied(
                                    [tile_data.width as usize, tile_data.height as usize],
                                    &tile_data.pixels,
                                );
                                ui.ctx().load_texture(
                                    format!("tile_{}_{}_{}", key.z, key.x, key.y),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                )
                            });
                        }
                    }
                    self.draw_quad_overlay(ui, available_rect);
                } else {
                    self.draw_map(ui, available_rect);
                    self.draw_radar_overlay(ui, available_rect);
                    self.draw_radar_sites(ui, available_rect);
                }

                self.handle_interaction(&response, available_rect);
            });

        ctx.request_repaint();
    }
}
