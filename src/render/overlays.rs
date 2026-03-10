use crate::render::map::MapView;

pub struct MapOverlays;

impl MapOverlays {
    /// Draw range rings centered on the radar site
    pub fn draw_range_rings(
        painter: &egui::Painter,
        site_lat: f64,
        site_lon: f64,
        map_view: &MapView,
        rect: egui::Rect,
    ) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let distances_km = [25.0, 50.0, 100.0, 150.0, 200.0, 300.0];

        for &dist_km in &distances_km {
            // Draw circle by plotting 72 points (every 5°)
            let mut points: Vec<egui::Pos2> = Vec::new();
            for angle_deg in (0..=360).step_by(5) {
                let angle_rad = (angle_deg as f64).to_radians();
                let lat = site_lat + (dist_km / 111.139) * angle_rad.cos();
                let lon = site_lon
                    + (dist_km / (111.139 * site_lat.to_radians().cos())) * angle_rad.sin();
                let (px, py) = map_view.lat_lon_to_pixel(lat, lon, screen_w, screen_h);
                points.push(egui::pos2(rect.left() + px as f32, rect.top() + py as f32));
            }

            // Draw ring segments
            for i in 0..points.len() - 1 {
                painter.line_segment(
                    [points[i], points[i + 1]],
                    egui::Stroke::new(0.8, egui::Color32::from_white_alpha(60)),
                );
            }

            // Label at north point
            let label_pos = points[0]; // 0° = north
            painter.text(
                label_pos + egui::vec2(4.0, -2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{}km", dist_km as i32),
                egui::FontId::proportional(10.0),
                egui::Color32::from_white_alpha(120),
            );
        }
    }

    /// Draw azimuth spokes every 30° from radar center
    pub fn draw_azimuth_lines(
        painter: &egui::Painter,
        site_lat: f64,
        site_lon: f64,
        map_view: &MapView,
        rect: egui::Rect,
        max_range_km: f64,
    ) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let (cx, cy) = map_view.lat_lon_to_pixel(site_lat, site_lon, screen_w, screen_h);
        let center = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);

        for angle_deg in (0..360).step_by(30) {
            let angle_rad = (angle_deg as f64).to_radians();
            let end_lat = site_lat + (max_range_km / 111.139) * angle_rad.cos();
            let end_lon = site_lon
                + (max_range_km / (111.139 * site_lat.to_radians().cos())) * angle_rad.sin();
            let (ex, ey) = map_view.lat_lon_to_pixel(end_lat, end_lon, screen_w, screen_h);
            let end = egui::pos2(rect.left() + ex as f32, rect.top() + ey as f32);

            painter.line_segment(
                [center, end],
                egui::Stroke::new(0.5, egui::Color32::from_white_alpha(40)),
            );

            // Label at spoke endpoint
            let label = match angle_deg {
                0 => "N",
                30 => "30",
                60 => "60",
                90 => "E",
                120 => "120",
                150 => "150",
                180 => "S",
                210 => "210",
                240 => "240",
                270 => "W",
                300 => "300",
                330 => "330",
                _ => "",
            };
            painter.text(
                end + egui::vec2(2.0, -2.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(10.0),
                egui::Color32::from_white_alpha(100),
            );
        }
    }

    /// Draw the radar site marker (small dot with label)
    pub fn draw_site_marker(
        painter: &egui::Painter,
        site_lat: f64,
        site_lon: f64,
        station_id: &str,
        map_view: &MapView,
        rect: egui::Rect,
    ) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;
        let (px, py) = map_view.lat_lon_to_pixel(site_lat, site_lon, screen_w, screen_h);
        let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);

        // Draw crosshair
        let size = 6.0;
        let color = egui::Color32::from_rgb(0, 200, 255);
        painter.line_segment(
            [pos - egui::vec2(size, 0.0), pos + egui::vec2(size, 0.0)],
            egui::Stroke::new(1.5, color),
        );
        painter.line_segment(
            [pos - egui::vec2(0.0, size), pos + egui::vec2(0.0, size)],
            egui::Stroke::new(1.5, color),
        );
        painter.circle_stroke(pos, 4.0, egui::Stroke::new(1.0, color));

        // Label
        painter.text(
            pos + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            station_id,
            egui::FontId::proportional(11.0),
            color,
        );
    }
}
