use std::collections::HashMap;

use egui;

use crate::nexrad::sites::{self, RadarSite, RADAR_SITES};
use crate::render::map::MapView;

pub struct NationalView {
    /// Texture handles for each station's thumbnail
    thumbnails: HashMap<String, egui::TextureHandle>,
}

impl NationalView {
    pub fn new() -> Self {
        Self {
            thumbnails: HashMap::new(),
        }
    }

    /// Update/create a thumbnail texture for a station.
    /// Called when preload cache has new data.
    /// `pixels` must be 256x256 RGBA (256*256*4 = 262144 bytes).
    pub fn update_thumbnail(
        &mut self,
        ctx: &egui::Context,
        station_id: &str,
        pixels: &[u8], // 256x256 RGBA
    ) {
        let image = egui::ColorImage::from_rgba_unmultiplied([256, 256], pixels);
        let texture = ctx.load_texture(
            format!("national_thumb_{station_id}"),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.thumbnails.insert(station_id.to_string(), texture);
    }

    /// Remove a station's thumbnail (e.g. when data expires).
    pub fn remove_thumbnail(&mut self, station_id: &str) {
        self.thumbnails.remove(station_id);
    }

    /// Draw all cached thumbnails on the map, geo-referenced.
    /// Each thumbnail is positioned at the radar site's lat/lon
    /// and sized based on zoom level to approximate coverage area.
    pub fn draw(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        map_view: &MapView,
        loaded_stations: &[String],
    ) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        // Calculate thumbnail display size based on zoom.
        // At zoom 4-5: small (~60-80px), show many stations at once.
        // At zoom 6-7: medium (~120-200px), fewer visible but larger.
        // At zoom 8+: should transition to single-radar view.
        let base_size = match map_view.zoom as u32 {
            0..=3 => 40.0_f32,
            4 => 60.0,
            5 => 100.0,
            6 => 160.0,
            7 => 250.0,
            _ => 350.0,
        };

        for station_id in loaded_stations {
            let site = match sites::find_site(station_id) {
                Some(s) => s,
                None => continue,
            };

            let (cx, cy) = map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
            let center = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);

            // Skip if off-screen (with margin)
            let margin = base_size;
            if center.x < rect.left() - margin
                || center.x > rect.right() + margin
                || center.y < rect.top() - margin
                || center.y > rect.bottom() + margin
            {
                continue;
            }

            let half = base_size / 2.0;
            let thumb_rect =
                egui::Rect::from_center_size(center, egui::vec2(base_size, base_size));

            if let Some(tex) = self.thumbnails.get(station_id.as_str()) {
                // Draw the thumbnail with slight transparency so the map shows through
                ui.painter().image(
                    tex.id(),
                    thumb_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha(200),
                );
            }

            // Draw station label below the thumbnail
            ui.painter().text(
                egui::pos2(center.x, center.y + half + 2.0),
                egui::Align2::CENTER_TOP,
                station_id,
                egui::FontId::proportional(9.0),
                egui::Color32::from_gray(180),
            );
        }
    }

    /// Returns true if the given zoom level should use national overview mode.
    /// Below zoom 7 the view is zoomed out enough to show multiple stations.
    pub fn should_show(zoom: f64) -> bool {
        zoom < 7.0
    }

    /// How many thumbnails are currently loaded.
    pub fn loaded_count(&self) -> usize {
        self.thumbnails.len()
    }

    /// Find the nearest radar site to a screen click position.
    /// Returns the site and the screen-space distance, useful for transitioning
    /// from national overview to single-station view when the user clicks a thumbnail.
    pub fn nearest_site_to_click(
        click_pos: egui::Pos2,
        rect: egui::Rect,
        map_view: &MapView,
        loaded_stations: &[String],
    ) -> Option<(&'static RadarSite, f32)> {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let mut best: Option<(&'static RadarSite, f32)> = None;

        for station_id in loaded_stations {
            let site = match sites::find_site(station_id) {
                Some(s) => s,
                None => continue,
            };

            let (cx, cy) = map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
            let center = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);
            let dist = center.distance(click_pos);

            match &best {
                Some((_, best_dist)) if dist < *best_dist => {
                    best = Some((site, dist));
                }
                None => {
                    best = Some((site, dist));
                }
                _ => {}
            }
        }

        best
    }

    /// Find the nearest radar site to a screen click, considering all known sites
    /// (not just those with loaded thumbnails). Useful for picking a station to
    /// load when the user clicks an empty area.
    pub fn nearest_any_site_to_click(
        click_pos: egui::Pos2,
        rect: egui::Rect,
        map_view: &MapView,
    ) -> Option<(&'static RadarSite, f32)> {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let mut best: Option<(&'static RadarSite, f32)> = None;

        for site in RADAR_SITES {
            let (cx, cy) = map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
            let center = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);
            let dist = center.distance(click_pos);

            match &best {
                Some((_, best_dist)) if dist < *best_dist => {
                    best = Some((site, dist));
                }
                None => {
                    best = Some((site, dist));
                }
                _ => {}
            }
        }

        best
    }
}
