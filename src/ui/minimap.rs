use egui;
use crate::render::map::MapView;

pub struct Minimap {
    size: f32, // 150.0 default
}

impl Minimap {
    pub fn new() -> Self {
        Self { size: 150.0 }
    }

    /// Draw the minimap in the bottom-right corner of the given rect.
    /// Shows:
    /// - Dark background
    /// - All NEXRAD site dots (colored by whether they have data)
    /// - A rectangle showing the current viewport extent
    /// - Current station highlighted
    /// Returns Some((lat, lon)) if the user clicked on the minimap (for jumping)
    pub fn draw(
        &self,
        ui: &mut egui::Ui,
        parent_rect: egui::Rect,
        main_map_view: &MapView,
        current_station: &str,
        loaded_stations: &[String], // stations with preloaded data
    ) -> Option<(f64, f64)> {
        // Position in bottom-right corner with 10px margin
        let minimap_rect = egui::Rect::from_min_size(
            egui::pos2(
                parent_rect.right() - self.size - 10.0,
                parent_rect.bottom() - self.size - 10.0,
            ),
            egui::vec2(self.size, self.size),
        );

        // Dark semi-transparent background
        ui.painter().rect_filled(
            minimap_rect,
            4.0,
            egui::Color32::from_rgba_premultiplied(20, 20, 30, 200),
        );
        ui.painter().rect_stroke(
            minimap_rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
            egui::StrokeKind::Outside,
        );

        // Create a minimap MapView centered on CONUS
        // CONUS bounds roughly: lat 24-50, lon -125 to -66
        let mini_view = MapView {
            center_lat: 38.0,
            center_lon: -96.0,
            zoom: 3.5,
        };

        let mini_w = self.size as f64;
        let mini_h = self.size as f64;

        // Draw all radar sites as dots
        for site in crate::nexrad::sites::RADAR_SITES.iter() {
            let (px, py) = mini_view.lat_lon_to_pixel(site.lat, site.lon, mini_w, mini_h);
            let pos = egui::pos2(
                minimap_rect.left() + px as f32,
                minimap_rect.top() + py as f32,
            );

            if !minimap_rect.contains(pos) {
                continue;
            }

            let color = if site.id == current_station {
                egui::Color32::from_rgb(0, 229, 255) // accent cyan
            } else if loaded_stations.contains(&site.id.to_string()) {
                egui::Color32::from_rgb(0, 200, 100) // green = loaded
            } else {
                egui::Color32::from_gray(80) // dim = not loaded
            };

            let radius = if site.id == current_station { 3.0 } else { 1.5 };
            ui.painter().circle_filled(pos, radius, color);
        }

        // Draw viewport rectangle showing where the main view is looking
        // Convert main map view corners to minimap pixels
        let main_w = parent_rect.width() as f64;
        let main_h = parent_rect.height() as f64;
        let (tl_lat, tl_lon) = main_map_view.pixel_to_lat_lon(0.0, 0.0, main_w, main_h);
        let (br_lat, br_lon) = main_map_view.pixel_to_lat_lon(main_w, main_h, main_w, main_h);

        let (tl_px, tl_py) = mini_view.lat_lon_to_pixel(tl_lat, tl_lon, mini_w, mini_h);
        let (br_px, br_py) = mini_view.lat_lon_to_pixel(br_lat, br_lon, mini_w, mini_h);

        let viewport_rect = egui::Rect::from_min_max(
            egui::pos2(
                minimap_rect.left() + tl_px as f32,
                minimap_rect.top() + tl_py as f32,
            ),
            egui::pos2(
                minimap_rect.left() + br_px as f32,
                minimap_rect.top() + br_py as f32,
            ),
        );

        // Clamp to minimap bounds
        let viewport_rect = viewport_rect.intersect(minimap_rect);

        if viewport_rect.width() > 2.0 && viewport_rect.height() > 2.0 {
            ui.painter().rect_stroke(
                viewport_rect,
                0.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 255, 0)),
                egui::StrokeKind::Outside,
            );
        }

        // Handle click on minimap to jump to location
        // Use a separate sense rect for the minimap area
        let minimap_response = ui.allocate_rect(minimap_rect, egui::Sense::click());
        if minimap_response.clicked() {
            if let Some(pos) = minimap_response.interact_pointer_pos() {
                let mx = (pos.x - minimap_rect.left()) as f64;
                let my = (pos.y - minimap_rect.top()) as f64;
                let (lat, lon) = mini_view.pixel_to_lat_lon(mx, my, mini_w, mini_h);
                return Some((lat, lon));
            }
        }

        None
    }

    /// Only show minimap when zoomed in enough that national context is useful
    pub fn should_show(zoom: f64) -> bool {
        zoom >= 6.0
    }
}
