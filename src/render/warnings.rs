use egui::{Color32, Painter, Pos2, Rect, Stroke};

use crate::data::alerts::WeatherAlert;
use crate::render::map::MapView;

/// Renders NWS watch/warning/advisory polygons on the map
pub struct WarningRenderer;

struct WarningStyle {
    stroke_color: Color32,
    stroke_width: f32,
    fill_alpha: u8, // 0 = no fill
}

impl WarningRenderer {
    /// Draw warning polygons on the map
    pub fn draw_warnings(
        alerts: &[WeatherAlert],
        painter: &Painter,
        map_view: &MapView,
        rect: Rect,
    ) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;
        let offset = rect.min;

        for alert in alerts {
            if alert.polygon.len() < 3 {
                continue;
            }

            let style = Self::style_for_event(&alert.event);

            // Convert polygon points to screen coordinates
            let screen_points: Vec<Pos2> = alert
                .polygon
                .iter()
                .map(|&(lat, lon)| {
                    let (px, py) = map_view.lat_lon_to_pixel(lat, lon, screen_w, screen_h);
                    Pos2::new(px as f32 + offset.x, py as f32 + offset.y)
                })
                .collect();

            // Draw filled polygon if fill_alpha > 0
            if style.fill_alpha > 0 {
                let fill_color = Color32::from_rgba_unmultiplied(
                    style.stroke_color.r(),
                    style.stroke_color.g(),
                    style.stroke_color.b(),
                    style.fill_alpha,
                );
                // Use triangle fan to fill the polygon
                Self::fill_polygon(painter, &screen_points, fill_color, rect);
            }

            // Draw stroke outline using line segments
            let stroke = Stroke::new(style.stroke_width, style.stroke_color);
            for i in 0..screen_points.len() {
                let j = (i + 1) % screen_points.len();
                painter.line_segment([screen_points[i], screen_points[j]], stroke);
            }
        }
    }

    fn style_for_event(event: &str) -> WarningStyle {
        let event_lower = event.to_lowercase();

        if event_lower.contains("tornado") && event_lower.contains("warning") {
            WarningStyle {
                stroke_color: Color32::from_rgb(0xFF, 0x00, 0x00),
                stroke_width: 3.0,
                fill_alpha: 40,
            }
        } else if event_lower.contains("severe thunderstorm")
            && event_lower.contains("warning")
        {
            WarningStyle {
                stroke_color: Color32::from_rgb(0xFF, 0xA5, 0x00),
                stroke_width: 3.0,
                fill_alpha: 40,
            }
        } else if event_lower.contains("tornado") && event_lower.contains("watch") {
            WarningStyle {
                stroke_color: Color32::from_rgb(0xFF, 0xFF, 0x00),
                stroke_width: 2.0,
                fill_alpha: 0,
            }
        } else if event_lower.contains("severe thunderstorm")
            && event_lower.contains("watch")
        {
            WarningStyle {
                stroke_color: Color32::from_rgb(0xCC, 0xCC, 0x00),
                stroke_width: 2.0,
                fill_alpha: 0,
            }
        } else if event_lower.contains("flash flood") && event_lower.contains("warning") {
            WarningStyle {
                stroke_color: Color32::from_rgb(0x00, 0xFF, 0x00),
                stroke_width: 2.0,
                fill_alpha: 40,
            }
        } else {
            WarningStyle {
                stroke_color: Color32::from_rgb(0xFF, 0xFF, 0xFF),
                stroke_width: 1.0,
                fill_alpha: 0,
            }
        }
    }

    /// Fill a polygon using triangle fan decomposition
    fn fill_polygon(painter: &Painter, points: &[Pos2], color: Color32, _clip_rect: Rect) {
        if points.len() < 3 {
            return;
        }

        // Simple ear/fan triangulation from first vertex
        let mesh = {
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(points[0], color);
            for p in &points[1..] {
                mesh.colored_vertex(*p, color);
            }
            // Triangle fan: (0, 1, 2), (0, 2, 3), (0, 3, 4), ...
            for i in 1..(points.len() as u32 - 1) {
                mesh.add_triangle(0, i, i + 1);
            }
            mesh
        };

        painter.add(egui::Shape::mesh(mesh));
    }
}
