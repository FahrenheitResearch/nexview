use crate::nexrad::{Level2File, RadarProduct, RadarSite};
use crate::render::ColorTable;

/// Renders a vertical cross-section (RHI-style slice) through radar volume data.
pub struct CrossSectionRenderer;

/// Output of rendering a cross-section.
pub struct CrossSectionResult {
    pub pixels: Vec<u8>, // RGBA
    pub width: u32,
    pub height: u32,
    pub max_range_km: f64,
    pub max_altitude_km: f64,
}

/// Effective Earth radius using 4/3 refraction model (km).
const RE_PRIME_KM: f64 = 6371.0 * 4.0 / 3.0;

impl CrossSectionRenderer {
    /// Render a vertical cross-section along a line from `start` to `end` (lat, lon).
    ///
    /// Samples every sweep in the volume file along the specified ground track and
    /// builds a 2-D image where the horizontal axis is ground distance along the
    /// line and the vertical axis is altitude (0 at the bottom, `max_altitude_km`
    /// at the top).
    pub fn render_cross_section(
        file: &Level2File,
        product: RadarProduct,
        color_table: &ColorTable,
        site: &RadarSite,
        start: (f64, f64),
        end: (f64, f64),
        width: u32,
        height: u32,
    ) -> Option<CrossSectionResult> {
        if file.sweeps.is_empty() || width == 0 || height == 0 {
            return None;
        }

        let max_altitude_km: f64 = 20.0;

        // Total ground distance of the cross-section line.
        let total_ground_km = ground_distance_km(start.0, start.1, end.0, end.1);
        if total_ground_km < 0.01 {
            return None;
        }

        // Vertical spread of each data point in pixels – ensures we fill gaps
        // between elevation tilts.
        let beam_half_height_px = (height as f64 / file.sweeps.len().max(1) as f64 / 2.0)
            .ceil() as i32;

        let w = width as usize;
        let h = height as usize;
        let mut pixels = vec![0u8; w * h * 4]; // transparent black

        // For every horizontal column, interpolate a geographic point along the
        // line, then sample each sweep at that azimuth / range.
        for x in 0..width {
            let t = x as f64 / (width - 1).max(1) as f64;
            let lat = start.0 + t * (end.0 - start.0);
            let lon = start.1 + t * (end.1 - start.1);

            let ground_dist_km = ground_distance_km(site.lat, site.lon, lat, lon);
            let azimuth_deg = azimuth_deg(site.lat, site.lon, lat, lon);

            for sweep in &file.sweeps {
                // Find the radial whose azimuth is closest to the desired azimuth.
                let radial = sweep.radials.iter().min_by(|a, b| {
                    let da = azimuth_difference(a.azimuth as f64, azimuth_deg);
                    let db = azimuth_difference(b.azimuth as f64, azimuth_deg);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                });

                let radial = match radial {
                    Some(r) => r,
                    None => continue,
                };

                // Reject if the nearest radial is more than ~1.5 azimuth spacings away.
                let az_diff = azimuth_difference(radial.azimuth as f64, azimuth_deg);
                let az_limit = (radial.azimuth_spacing as f64 * 1.5).max(2.0);
                if az_diff > az_limit {
                    continue;
                }

                // Look up the moment data for the requested product.
                let moment = match radial.moments.iter().find(|m| m.product == product) {
                    Some(m) => m,
                    None => continue,
                };

                // Compute slant range from ground distance and elevation angle.
                let elev_rad = (radial.elevation as f64).to_radians();
                let slant_range_km = if elev_rad.cos().abs() > 1e-6 {
                    ground_dist_km / elev_rad.cos()
                } else {
                    continue;
                };

                if slant_range_km < 0.0 {
                    continue;
                }

                // Which gate index does this slant range correspond to?
                let first_gate_km = moment.first_gate_range as f64 / 1000.0;
                let gate_size_km = moment.gate_size as f64 / 1000.0;
                if gate_size_km <= 0.0 {
                    continue;
                }
                let gate_idx =
                    ((slant_range_km - first_gate_km) / gate_size_km).round() as i64;
                if gate_idx < 0 || gate_idx >= moment.gate_count as i64 {
                    continue;
                }

                let value = moment.data[gate_idx as usize];
                if value.is_nan() || value < color_table.min_value {
                    continue;
                }
                let value = value.min(color_table.max_value);

                // Beam altitude using standard atmospheric refraction formula:
                //   h = sqrt(r² + Re'² + 2·r·Re'·sin(θ)) − Re'
                let slant_range_m = slant_range_km * 1000.0;
                let re_m = RE_PRIME_KM * 1000.0;
                let altitude_m = ((slant_range_m * slant_range_m)
                    + (re_m * re_m)
                    + (2.0 * slant_range_m * re_m * elev_rad.sin()))
                .sqrt()
                    - re_m;
                let altitude_km = altitude_m / 1000.0;

                if altitude_km < 0.0 || altitude_km > max_altitude_km {
                    continue;
                }

                // Map altitude to a vertical pixel coordinate (0 = top row, h-1 = bottom).
                let y_center =
                    ((1.0 - altitude_km / max_altitude_km) * (h - 1) as f64).round() as i32;

                let color = color_table.color_for_value(value);

                // Paint a small vertical strip to avoid gaps between tilts.
                let y_min = (y_center - beam_half_height_px).max(0) as usize;
                let y_max = (y_center + beam_half_height_px).min(h as i32 - 1) as usize;

                for y in y_min..=y_max {
                    let idx = (y * w + x as usize) * 4;
                    // Overwrite only if this pixel is still transparent, or if this
                    // tilt is higher-resolution (prefer data closer to the ground
                    // which tends to be more detailed).  A simple "first-write-wins"
                    // approach from lowest elevation up works well when sweeps are
                    // sorted ascending by elevation.
                    if pixels[idx + 3] == 0 {
                        pixels[idx] = color[0];
                        pixels[idx + 1] = color[1];
                        pixels[idx + 2] = color[2];
                        pixels[idx + 3] = color[3];
                    }
                }
            }
        }

        Some(CrossSectionResult {
            pixels,
            width,
            height,
            max_range_km: total_ground_km,
            max_altitude_km,
        })
    }

    /// Render a 3D perspective view of the radar volume along the cross-section line.
    ///
    /// Creates an isometric-style 3D rendering showing the cross-section data
    /// as a vertical slab with a ground plane grid, viewed from a configurable angle.
    /// `view_angle` is the horizontal rotation in degrees (0 = head-on, 45 = angled).
    /// `view_pitch` is the vertical tilt in degrees (0 = level, 30 = looking down).
    pub fn render_cross_section_3d(
        file: &Level2File,
        product: RadarProduct,
        color_table: &ColorTable,
        site: &RadarSite,
        start: (f64, f64),
        end: (f64, f64),
        width: u32,
        height: u32,
        view_angle: f64,
        view_pitch: f64,
    ) -> Option<CrossSectionResult> {
        if file.sweeps.is_empty() || width == 0 || height == 0 {
            return None;
        }

        let max_altitude_km: f64 = 20.0;
        let total_ground_km = ground_distance_km(start.0, start.1, end.0, end.1);
        if total_ground_km < 0.01 {
            return None;
        }

        // First, sample the volume data into a 2D grid (distance x altitude)
        let grid_w: usize = 400;
        let grid_h: usize = 200;
        let mut grid = vec![f32::NAN; grid_w * grid_h];

        for gx in 0..grid_w {
            let t = gx as f64 / (grid_w - 1).max(1) as f64;
            let lat = start.0 + t * (end.0 - start.0);
            let lon = start.1 + t * (end.1 - start.1);
            let ground_dist_km = ground_distance_km(site.lat, site.lon, lat, lon);
            let az = azimuth_deg(site.lat, site.lon, lat, lon);

            for sweep in &file.sweeps {
                let radial = sweep.radials.iter().min_by(|a, b| {
                    let da = azimuth_difference(a.azimuth as f64, az);
                    let db = azimuth_difference(b.azimuth as f64, az);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                });
                let radial = match radial { Some(r) => r, None => continue };
                let az_diff = azimuth_difference(radial.azimuth as f64, az);
                if az_diff > (radial.azimuth_spacing as f64 * 1.5).max(2.0) { continue; }
                let moment = match radial.moments.iter().find(|m| m.product == product) {
                    Some(m) => m, None => continue,
                };
                let elev_rad = (radial.elevation as f64).to_radians();
                if elev_rad.cos().abs() <= 1e-6 { continue; }
                let slant_range_km = ground_dist_km / elev_rad.cos();
                if slant_range_km < 0.0 { continue; }
                let first_gate_km = moment.first_gate_range as f64 / 1000.0;
                let gate_size_km = moment.gate_size as f64 / 1000.0;
                if gate_size_km <= 0.0 { continue; }
                let gate_idx = ((slant_range_km - first_gate_km) / gate_size_km).round() as i64;
                if gate_idx < 0 || gate_idx >= moment.gate_count as i64 { continue; }
                let value = moment.data[gate_idx as usize];
                if value.is_nan() || value < color_table.min_value { continue; }

                let slant_m = slant_range_km * 1000.0;
                let re_m = RE_PRIME_KM * 1000.0;
                let alt_m = ((slant_m * slant_m) + (re_m * re_m) + (2.0 * slant_m * re_m * elev_rad.sin())).sqrt() - re_m;
                let alt_km = alt_m / 1000.0;
                if alt_km < 0.0 || alt_km > max_altitude_km { continue; }

                let gy = ((1.0 - alt_km / max_altitude_km) * (grid_h - 1) as f64).round() as usize;
                if gy < grid_h {
                    let gi = gy * grid_w + gx;
                    if grid[gi].is_nan() {
                        grid[gi] = value.min(color_table.max_value);
                    }
                }
            }
        }

        // Now render the 3D perspective view
        let w = width as usize;
        let h = height as usize;
        let mut pixels = vec![0u8; w * h * 4];
        // Dark background
        for i in 0..w * h {
            pixels[i * 4] = 12;
            pixels[i * 4 + 1] = 12;
            pixels[i * 4 + 2] = 20;
            pixels[i * 4 + 3] = 255;
        }

        let angle_rad = view_angle.to_radians();
        let pitch_rad = view_pitch.to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let cos_p = pitch_rad.cos();
        let sin_p = pitch_rad.sin();

        // 3D coordinate system: X = along cross-section, Y = altitude, Z = depth (perpendicular)
        // Normalize coordinates to [-1, 1] range for projection
        let scale_x = total_ground_km;
        let scale_y = max_altitude_km;

        // Exaggerate vertical to make structure visible
        let vert_exaggeration = 3.0;

        // Project a 3D point to 2D screen coordinates (simple perspective)
        let project = |x3d: f64, y3d: f64, z3d: f64| -> Option<(f64, f64)> {
            // Rotate around Y axis by view_angle
            let rx = x3d * cos_a + z3d * sin_a;
            let ry = y3d;
            let rz = -x3d * sin_a + z3d * cos_a;
            // Rotate around X axis by pitch
            let fy = ry * cos_p - rz * sin_p;
            let fz = ry * sin_p + rz * cos_p;
            // Perspective projection
            let camera_dist = 4.0;
            let depth = fz + camera_dist;
            if depth < 0.1 { return None; }
            let px = rx / depth;
            let py = -fy / depth; // flip Y so altitude goes up
            // Map to screen
            let sx = (px * 1.8 + 0.5) * w as f64;
            let sy = (py * 1.8 + 0.55) * h as f64;
            Some((sx, sy))
        };

        // Helper to draw a line on the pixel buffer
        let draw_line = |pixels: &mut Vec<u8>, x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 4]| {
            let dx = (x1 - x0).abs();
            let dy = (y1 - y0).abs();
            let sx = if x0 < x1 { 1 } else { -1 };
            let sy = if y0 < y1 { 1 } else { -1 };
            let mut err = dx - dy;
            let mut cx = x0;
            let mut cy = y0;
            let steps = (dx + dy).max(1);
            for _ in 0..=steps {
                if cx >= 0 && cx < w as i32 && cy >= 0 && cy < h as i32 {
                    let idx = (cy as usize * w + cx as usize) * 4;
                    // Alpha blend
                    let alpha = color[3] as f32 / 255.0;
                    pixels[idx] = (pixels[idx] as f32 * (1.0 - alpha) + color[0] as f32 * alpha) as u8;
                    pixels[idx + 1] = (pixels[idx + 1] as f32 * (1.0 - alpha) + color[1] as f32 * alpha) as u8;
                    pixels[idx + 2] = (pixels[idx + 2] as f32 * (1.0 - alpha) + color[2] as f32 * alpha) as u8;
                    pixels[idx + 3] = 255;
                }
                if cx == x1 && cy == y1 { break; }
                let e2 = 2 * err;
                if e2 > -dy { err -= dy; cx += sx; }
                if e2 < dx { err += dx; cy += sy; }
            }
        };

        // 1. Draw ground plane grid
        let grid_color = [40, 60, 80, 180];
        let grid_lines = 10;
        // Lines along the cross-section direction (constant Z)
        for iz in 0..=4 {
            let z = (iz as f64 / 4.0 - 0.5) * 0.4; // shallow depth range
            for i in 0..grid_lines {
                let t0 = i as f64 / grid_lines as f64;
                let t1 = (i + 1) as f64 / grid_lines as f64;
                let x0_3d = (t0 - 0.5) * 2.0;
                let x1_3d = (t1 - 0.5) * 2.0;
                if let (Some((sx0, sy0)), Some((sx1, sy1))) = (
                    project(x0_3d, -1.0 * vert_exaggeration / (scale_y / scale_x).max(1.0), z),
                    project(x1_3d, -1.0 * vert_exaggeration / (scale_y / scale_x).max(1.0), z),
                ) {
                    draw_line(&mut pixels, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, grid_color);
                }
            }
        }
        // Lines perpendicular to cross-section (constant X)
        for ix in 0..=grid_lines {
            let x = (ix as f64 / grid_lines as f64 - 0.5) * 2.0;
            let ground_y = -1.0 * vert_exaggeration / (scale_y / scale_x).max(1.0);
            if let (Some((sx0, sy0)), Some((sx1, sy1))) = (
                project(x, ground_y, -0.2),
                project(x, ground_y, 0.2),
            ) {
                draw_line(&mut pixels, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, grid_color);
            }
        }

        // 2. Draw the cross-section slab — use a Z-buffer approach
        // For each column in the grid, project each cell and paint it
        let mut zbuf = vec![f64::MAX; w * h];
        let norm_y_range = vert_exaggeration / (scale_y / scale_x).max(1.0);

        for gx in 0..grid_w {
            let x3d = (gx as f64 / (grid_w - 1) as f64 - 0.5) * 2.0;
            for gy in 0..grid_h {
                let gi = gy * grid_w + gx;
                let value = grid[gi];
                if value.is_nan() { continue; }

                // Map grid Y to 3D Y coordinate
                let t_y = gy as f64 / (grid_h - 1) as f64; // 0=top, 1=bottom
                let y3d = (1.0 - 2.0 * t_y) * norm_y_range;

                let z3d = 0.0; // cross-section is at Z=0

                if let Some((sx, sy)) = project(x3d, y3d, z3d) {
                    let px = sx as i32;
                    let py = sy as i32;
                    // Paint a small area to fill gaps
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let fx = px + dx;
                            let fy = py + dy;
                            if fx >= 0 && fx < w as i32 && fy >= 0 && fy < h as i32 {
                                let si = fy as usize * w + fx as usize;
                                // Simple depth test (closer to camera = lower fz)
                                let rx = x3d * cos_a + z3d * sin_a;
                                let _ry = y3d;
                                let rz = -x3d * sin_a + z3d * cos_a;
                                let fz = y3d * sin_p + rz * cos_p + 4.0;
                                if fz < zbuf[si] {
                                    zbuf[si] = fz;
                                    let color = color_table.color_for_value(value);
                                    let idx = si * 4;
                                    pixels[idx] = color[0];
                                    pixels[idx + 1] = color[1];
                                    pixels[idx + 2] = color[2];
                                    pixels[idx + 3] = 255;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Draw border frame around the cross-section slab
        let frame_color = [120, 180, 220, 200];
        let corners = [
            (-1.0, -norm_y_range, 0.0),
            (1.0, -norm_y_range, 0.0),
            (1.0, norm_y_range, 0.0),
            (-1.0, norm_y_range, 0.0),
        ];
        for i in 0..4 {
            let (x0, y0, z0) = corners[i];
            let (x1, y1, z1) = corners[(i + 1) % 4];
            if let (Some((sx0, sy0)), Some((sx1, sy1))) = (
                project(x0, y0, z0), project(x1, y1, z1),
            ) {
                draw_line(&mut pixels, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, frame_color);
            }
        }

        // 4. Draw altitude labels on the left edge
        for i in 0..=4 {
            let t = i as f64 / 4.0;
            let alt_km = max_altitude_km * t;
            let y3d = (-1.0 + 2.0 * t) * norm_y_range;
            if let Some((sx, sy)) = project(-1.0, y3d, 0.0) {
                // Draw a small tick mark
                let tick_x = (sx as i32 - 5).max(0);
                draw_line(&mut pixels, tick_x, sy as i32, sx as i32, sy as i32, frame_color);
                // We can't easily draw text in raw pixels, but the tick marks help
            }
        }

        // 5. Draw distance labels along the bottom edge
        for i in 0..=4 {
            let t = i as f64 / 4.0;
            let x3d = (t - 0.5) * 2.0;
            let y3d = -norm_y_range;
            if let Some((sx, sy)) = project(x3d, y3d, 0.0) {
                let tick_y = (sy as i32 + 5).min(h as i32 - 1);
                draw_line(&mut pixels, sx as i32, sy as i32, sx as i32, tick_y, frame_color);
            }
        }

        Some(CrossSectionResult {
            pixels,
            width,
            height,
            max_range_km: total_ground_km,
            max_altitude_km,
        })
    }
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/// Ground distance between two lat/lon points in kilometers (flat-Earth approx).
fn ground_distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat_avg_rad = ((lat1 + lat2) / 2.0).to_radians();
    let dx = (lon2 - lon1) * lat_avg_rad.cos() * 111.139;
    let dy = (lat2 - lat1) * 111.139;
    (dx * dx + dy * dy).sqrt()
}

/// Azimuth (bearing) in degrees [0, 360) from point 1 to point 2.
fn azimuth_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat_avg_rad = ((lat1 + lat2) / 2.0).to_radians();
    let dx = (lon2 - lon1) * lat_avg_rad.cos() * 111139.0;
    let dy = (lat2 - lat1) * 111139.0;
    let az = dx.atan2(dy).to_degrees();
    if az < 0.0 {
        az + 360.0
    } else {
        az
    }
}

/// Absolute angular difference between two azimuths in degrees, in the range
/// [0, 180].
fn azimuth_difference(a: f64, b: f64) -> f64 {
    let diff = (a - b).abs() % 360.0;
    if diff > 180.0 {
        360.0 - diff
    } else {
        diff
    }
}
