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

/// Standard WSR-88D beamwidth in degrees.
const BEAMWIDTH_DEG: f64 = 0.95;

impl CrossSectionRenderer {
    /// Render a vertical cross-section along a line from `start` to `end` (lat, lon).
    ///
    /// For each pixel, finds the two elevation sweeps that bracket it in altitude
    /// and interpolates between them, producing a smooth, gap-free image.
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
        let total_ground_km = ground_distance_km(start.0, start.1, end.0, end.1);
        if total_ground_km < 0.01 {
            return None;
        }

        let w = width as usize;
        let h = height as usize;
        let mut pixels = vec![0u8; w * h * 4];

        // For each pixel column, sample at the desired azimuth
        for x in 0..width {
            let t = x as f64 / (width - 1).max(1) as f64;
            let lat = start.0 + t * (end.0 - start.0);
            let lon = start.1 + t * (end.1 - start.1);
            let ground_dist_km = ground_distance_km(site.lat, site.lon, lat, lon);
            let desired_az = azimuth_deg(site.lat, site.lon, lat, lon);

            if ground_dist_km < 0.1 { continue; }

            // Collect (altitude_km, value) samples from all sweeps at this column
            let mut samples: Vec<(f64, f32)> = Vec::new();

            for sweep in &file.sweeps {
                let radial = match find_nearest_radial(sweep, desired_az) {
                    Some(r) => r,
                    None => continue,
                };

                let moment = match radial.moments.iter().find(|m| m.product == product) {
                    Some(m) => m,
                    None => continue,
                };

                let elev_rad = (radial.elevation as f64).to_radians();
                if elev_rad.cos().abs() <= 1e-6 { continue; }
                let slant_range_km = ground_dist_km / elev_rad.cos();
                if slant_range_km < 0.0 { continue; }

                let value = sample_gate(moment, slant_range_km);
                if value.is_nan() || value < color_table.min_value { continue; }

                let alt_km = beam_altitude_km(slant_range_km, elev_rad);
                if alt_km < 0.0 || alt_km > max_altitude_km { continue; }

                // Also compute the beam's vertical extent at this range
                let half_bw = (BEAMWIDTH_DEG / 2.0).to_radians();
                let alt_top = beam_altitude_km(slant_range_km, elev_rad + half_bw);
                let alt_bot = beam_altitude_km(slant_range_km, elev_rad - half_bw);
                let beam_half_km = (alt_top - alt_bot).abs() / 2.0;
                // Expand beam to fill gaps: use at least half the spacing to next tilt
                let expanded_half = beam_half_km.max(0.3);

                samples.push((alt_km, value.min(color_table.max_value)));
                // Also store the beam extent for gap filling
                let top = (alt_km + expanded_half).min(max_altitude_km);
                let bot = (alt_km - expanded_half).max(0.0);

                // Fill pixels in this beam's vertical extent
                let py_top = ((1.0 - top / max_altitude_km) * (h - 1) as f64) as i32;
                let py_bot = ((1.0 - bot / max_altitude_km) * (h - 1) as f64) as i32;
                let y_min = py_top.max(0) as usize;
                let y_max = py_bot.min(h as i32 - 1) as usize;

                let color = color_table.color_for_value(value.min(color_table.max_value));
                for y in y_min..=y_max {
                    let idx = (y * w + x as usize) * 4;
                    if pixels[idx + 3] == 0 {
                        pixels[idx] = color[0];
                        pixels[idx + 1] = color[1];
                        pixels[idx + 2] = color[2];
                        pixels[idx + 3] = color[3];
                    }
                }
            }

            // Second pass: interpolate between adjacent sweep samples to fill any remaining gaps
            if samples.len() >= 2 {
                samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                for pair in samples.windows(2) {
                    let (alt0, val0) = pair[0];
                    let (alt1, val1) = pair[1];
                    let py0 = ((1.0 - alt0 / max_altitude_km) * (h - 1) as f64) as usize;
                    let py1 = ((1.0 - alt1 / max_altitude_km) * (h - 1) as f64) as usize;
                    let (y_top, y_bot) = if py0 < py1 { (py0, py1) } else { (py1, py0) };
                    for y in y_top..=y_bot.min(h - 1) {
                        let idx = (y * w + x as usize) * 4;
                        if pixels[idx + 3] != 0 { continue; } // already filled
                        // Interpolate
                        let alt_here = max_altitude_km * (1.0 - y as f64 / (h - 1) as f64);
                        let t_interp = if (alt1 - alt0).abs() > 0.001 {
                            ((alt_here - alt0) / (alt1 - alt0)).clamp(0.0, 1.0) as f32
                        } else {
                            0.5
                        };
                        let val = val0 + (val1 - val0) * t_interp;
                        let color = color_table.color_for_value(val);
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
    /// Uses inverse mapping: for each screen pixel, ray-casts back into the 3D scene
    /// to find the corresponding data grid cell, producing a fully filled image.
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

        // Sample the volume data into a 2D grid (distance x altitude)
        let grid_w: usize = 600;
        let grid_h: usize = 300;
        let mut grid = vec![f32::NAN; grid_w * grid_h];

        for gx in 0..grid_w {
            let t = gx as f64 / (grid_w - 1).max(1) as f64;
            let lat = start.0 + t * (end.0 - start.0);
            let lon = start.1 + t * (end.1 - start.1);
            let ground_dist_km = ground_distance_km(site.lat, site.lon, lat, lon);
            let az = azimuth_deg(site.lat, site.lon, lat, lon);

            if ground_dist_km < 0.1 { continue; }

            // Collect samples for interpolation
            let mut col_samples: Vec<(f64, f32)> = Vec::new();

            for sweep in &file.sweeps {
                let radial = match find_nearest_radial(sweep, az) {
                    Some(r) => r,
                    None => continue,
                };
                let moment = match radial.moments.iter().find(|m| m.product == product) {
                    Some(m) => m, None => continue,
                };
                let elev_rad = (radial.elevation as f64).to_radians();
                if elev_rad.cos().abs() <= 1e-6 { continue; }
                let slant_range_km = ground_dist_km / elev_rad.cos();
                if slant_range_km < 0.0 { continue; }
                let value = sample_gate(moment, slant_range_km);
                if value.is_nan() || value < color_table.min_value { continue; }

                let alt_km = beam_altitude_km(slant_range_km, elev_rad);
                if alt_km < 0.0 || alt_km > max_altitude_km { continue; }

                // Paint beam extent
                let half_bw = (BEAMWIDTH_DEG / 2.0).to_radians();
                let alt_top = beam_altitude_km(slant_range_km, elev_rad + half_bw);
                let alt_bot = beam_altitude_km(slant_range_km, elev_rad - half_bw);
                let beam_half = ((alt_top - alt_bot).abs() / 2.0).max(0.3);
                let top_gy = ((1.0 - (alt_km + beam_half).min(max_altitude_km) / max_altitude_km) * (grid_h - 1) as f64) as usize;
                let bot_gy = ((1.0 - (alt_km - beam_half).max(0.0) / max_altitude_km) * (grid_h - 1) as f64) as usize;
                for gy in top_gy..=bot_gy.min(grid_h - 1) {
                    let gi = gy * grid_w + gx;
                    if grid[gi].is_nan() {
                        grid[gi] = value.min(color_table.max_value);
                    }
                }
                col_samples.push((alt_km, value.min(color_table.max_value)));
            }

            // Interpolate between sweep samples
            if col_samples.len() >= 2 {
                col_samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                for pair in col_samples.windows(2) {
                    let (alt0, v0) = pair[0];
                    let (alt1, v1) = pair[1];
                    let gy0 = ((1.0 - alt0 / max_altitude_km) * (grid_h - 1) as f64) as usize;
                    let gy1 = ((1.0 - alt1 / max_altitude_km) * (grid_h - 1) as f64) as usize;
                    let (top, bot) = if gy0 < gy1 { (gy0, gy1) } else { (gy1, gy0) };
                    for gy in top..=bot.min(grid_h - 1) {
                        let gi = gy * grid_w + gx;
                        if !grid[gi].is_nan() { continue; }
                        let alt = max_altitude_km * (1.0 - gy as f64 / (grid_h - 1) as f64);
                        let t_interp = if (alt1 - alt0).abs() > 0.001 {
                            ((alt - alt0) / (alt1 - alt0)).clamp(0.0, 1.0) as f32
                        } else { 0.5 };
                        grid[gi] = v0 + (v1 - v0) * t_interp;
                    }
                }
            }
        }

        // Render 3D using inverse mapping
        let w = width as usize;
        let h = height as usize;
        let mut pixels = vec![0u8; w * h * 4];

        // Dark gradient background
        for py in 0..h {
            let t = py as f64 / h as f64;
            let r = (12.0 + t * 8.0) as u8;
            let g = (12.0 + t * 6.0) as u8;
            let b = (20.0 + t * 10.0) as u8;
            for px in 0..w {
                let idx = (py * w + px) * 4;
                pixels[idx] = r;
                pixels[idx + 1] = g;
                pixels[idx + 2] = b;
                pixels[idx + 3] = 255;
            }
        }

        let angle_rad = view_angle.to_radians();
        let pitch_rad = view_pitch.to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let cos_p = pitch_rad.cos();
        let sin_p = pitch_rad.sin();

        // Vertical exaggeration so the slab isn't paper-thin
        let vert_exag = 2.5;
        let aspect = total_ground_km / max_altitude_km;
        let y_scale = vert_exag / aspect.max(1.0);

        let camera_dist = 3.5;

        // Forward project: 3D point -> screen
        let project = |x3d: f64, y3d: f64, z3d: f64| -> Option<(f64, f64, f64)> {
            let rx = x3d * cos_a + z3d * sin_a;
            let ry = y3d;
            let rz = -x3d * sin_a + z3d * cos_a;
            let fy = ry * cos_p - rz * sin_p;
            let fz = ry * sin_p + rz * cos_p;
            let depth = fz + camera_dist;
            if depth < 0.1 { return None; }
            let px = rx / depth;
            let py = -fy / depth;
            let sx = (px * 2.0 + 0.5) * w as f64;
            let sy = (py * 2.0 + 0.5) * h as f64;
            Some((sx, sy, depth))
        };

        // Inverse: screen -> ray direction, intersect with z=0 plane in world space
        // We need the inverse of the rotation to get world-space ray
        let unproject = |sx: f64, sy: f64| -> Option<(f64, f64)> {
            let px = (sx / w as f64 - 0.5) / 2.0;
            let py = (sy / h as f64 - 0.5) / 2.0;

            // Ray from camera through this pixel
            // Camera is at (0, 0, -camera_dist) in rotated space
            // Pixel is at (px * depth, -py * depth, 0) in rotated space for depth=1 plane
            // But we need to find the intersection with the z3d=0 plane in world space

            // In rotated space, camera is at origin looking along +Z
            // A point at screen (px, py) with perspective corresponds to direction (px, -py, 1)
            // We need to un-rotate this ray and find where z_world = 0

            // The forward transform is:
            //   rx = x*cos_a + z*sin_a
            //   ry = y
            //   rz = -x*sin_a + z*cos_a
            // Then pitch:
            //   fy = ry*cos_p - rz*sin_p
            //   fz = ry*sin_p + rz*cos_p
            // Screen: sx = rx/depth, sy = -fy/depth, depth = fz + camera_dist

            // For z_world = 0:
            //   rx = x * cos_a
            //   rz = -x * sin_a
            //   fy = y * cos_p - (-x*sin_a) * sin_p = y*cos_p + x*sin_a*sin_p
            //   fz = y * sin_p + (-x*sin_a) * cos_p = y*sin_p - x*sin_a*cos_p
            //   depth = fz + camera_dist

            // px = rx / depth = x*cos_a / depth
            // py = -fy / depth = -(y*cos_p + x*sin_a*sin_p) / depth

            // This is 2 equations, 3 unknowns (x, y, depth). With depth = y*sin_p - x*sin_a*cos_p + camera_dist:
            // px * depth = x * cos_a
            // py * depth = -(y * cos_p + x * sin_a * sin_p)

            // Substitute depth:
            // px * (y*sin_p - x*sin_a*cos_p + camera_dist) = x * cos_a
            // Expand: px*y*sin_p - px*x*sin_a*cos_p + px*camera_dist = x*cos_a
            // x*(cos_a + px*sin_a*cos_p) = px*y*sin_p + px*camera_dist  ... (eq1)

            // py * (y*sin_p - x*sin_a*cos_p + camera_dist) = -(y*cos_p + x*sin_a*sin_p)
            // py*y*sin_p - py*x*sin_a*cos_p + py*camera_dist = -y*cos_p - x*sin_a*sin_p
            // y*(py*sin_p + cos_p) + x*(-py*sin_a*cos_p + sin_a*sin_p) = -py*camera_dist
            // y*(py*sin_p + cos_p) + x*sin_a*(sin_p - py*cos_p) = -py*camera_dist  ... (eq2)

            // From eq1: x = (px*y*sin_p + px*camera_dist) / (cos_a + px*sin_a*cos_p)
            // Substitute into eq2...

            // This is getting complex. Let's just solve numerically using the 2x2 system.
            // Let A = cos_a + px*sin_a*cos_p, B = px*sin_p, C = px*camera_dist
            // eq1: A*x - B*y = C
            // Let D = sin_a*(sin_p - py*cos_p), E = py*sin_p + cos_p, F = -py*camera_dist
            // eq2: D*x + E*y = F

            let a_coeff = cos_a + px * sin_a * cos_p;
            let b_coeff = -px * sin_p;
            let c_val = px * camera_dist;
            let d_coeff = sin_a * (sin_p - py * cos_p);
            let e_coeff = py * sin_p + cos_p;
            let f_val = -py * camera_dist;

            let det = a_coeff * e_coeff - b_coeff * d_coeff;
            if det.abs() < 1e-10 { return None; }

            let x_world = (c_val * e_coeff - b_coeff * f_val) / det;
            let y_world = (a_coeff * f_val - c_val * d_coeff) / det;

            // x_world is in [-1, 1] range (normalized cross-section distance)
            // y_world is in [-y_scale, y_scale] range (normalized altitude)
            // Map to grid coordinates
            let gx_f = (x_world + 1.0) / 2.0; // 0 to 1
            let gy_f = (1.0 - y_world / y_scale) / 2.0; // 0 (top) to 1 (bottom)

            if gx_f < 0.0 || gx_f > 1.0 || gy_f < 0.0 || gy_f > 1.0 {
                return None;
            }

            Some((gx_f, gy_f))
        };

        // Inverse map every screen pixel
        for py in 0..h {
            for px in 0..w {
                if let Some((gx_f, gy_f)) = unproject(px as f64, py as f64) {
                    let gx = (gx_f * (grid_w - 1) as f64) as usize;
                    let gy = (gy_f * (grid_h - 1) as f64) as usize;
                    if gx < grid_w && gy < grid_h {
                        let gi = gy * grid_w + gx;
                        let value = grid[gi];
                        if !value.is_nan() {
                            let color = color_table.color_for_value(value);
                            if color[3] > 0 {
                                let idx = (py * w + px) * 4;
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

        // Draw grid lines over the data for reference
        let frame_color = [150, 200, 240, 160];
        let ground_color = [50, 80, 60, 140];

        // Vertical grid lines (distance markers)
        for i in 0..=10 {
            let x3d = (i as f64 / 10.0 - 0.5) * 2.0;
            for seg in 0..100 {
                let t0 = seg as f64 / 100.0;
                let t1 = (seg + 1) as f64 / 100.0;
                let y0 = (-1.0 + 2.0 * t0) * y_scale;
                let y1 = (-1.0 + 2.0 * t1) * y_scale;
                if let (Some((sx0, sy0, _)), Some((sx1, sy1, _))) = (
                    project(x3d, y0, 0.0), project(x3d, y1, 0.0),
                ) {
                    draw_line_aa(&mut pixels, w, h, sx0, sy0, sx1, sy1,
                        if i == 0 || i == 10 { frame_color } else { [30, 50, 70, 80] });
                }
            }
        }

        // Horizontal grid lines (altitude markers)
        for i in 0..=4 {
            let y3d = (-1.0 + 2.0 * i as f64 / 4.0) * y_scale;
            for seg in 0..100 {
                let t0 = seg as f64 / 100.0;
                let t1 = (seg + 1) as f64 / 100.0;
                let x0 = (t0 - 0.5) * 2.0;
                let x1 = (t1 - 0.5) * 2.0;
                if let (Some((sx0, sy0, _)), Some((sx1, sy1, _))) = (
                    project(x0, y3d, 0.0), project(x1, y3d, 0.0),
                ) {
                    draw_line_aa(&mut pixels, w, h, sx0, sy0, sx1, sy1,
                        if i == 0 { ground_color } else { [30, 50, 70, 80] });
                }
            }
        }

        // Border frame
        let edges = [
            ((-1.0, -y_scale, 0.0), (1.0, -y_scale, 0.0)),
            ((1.0, -y_scale, 0.0), (1.0, y_scale, 0.0)),
            ((1.0, y_scale, 0.0), (-1.0, y_scale, 0.0)),
            ((-1.0, y_scale, 0.0), (-1.0, -y_scale, 0.0)),
        ];
        for &((x0, y0, z0), (x1, y1, z1)) in &edges {
            if let (Some((sx0, sy0, _)), Some((sx1, sy1, _))) = (
                project(x0, y0, z0), project(x1, y1, z1),
            ) {
                draw_line_aa(&mut pixels, w, h, sx0, sy0, sx1, sy1, frame_color);
            }
        }

        // Ground plane grid (gives depth perception)
        for iz in 0..=6 {
            let z = (iz as f64 / 6.0 - 0.5) * 0.6;
            if (z - 0.0).abs() < 0.01 { continue; } // skip z=0, that's the data plane
            for seg in 0..50 {
                let t0 = seg as f64 / 50.0;
                let t1 = (seg + 1) as f64 / 50.0;
                let x0 = (t0 - 0.5) * 2.0;
                let x1 = (t1 - 0.5) * 2.0;
                if let (Some((sx0, sy0, _)), Some((sx1, sy1, _))) = (
                    project(x0, -y_scale, z), project(x1, -y_scale, z),
                ) {
                    draw_line_aa(&mut pixels, w, h, sx0, sy0, sx1, sy1, [25, 40, 55, 100]);
                }
            }
        }
        for ix in 0..=10 {
            let x = (ix as f64 / 10.0 - 0.5) * 2.0;
            if let (Some((sx0, sy0, _)), Some((sx1, sy1, _))) = (
                project(x, -y_scale, -0.3), project(x, -y_scale, 0.3),
            ) {
                draw_line_aa(&mut pixels, w, h, sx0, sy0, sx1, sy1, [25, 40, 55, 100]);
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
// Helpers
// ---------------------------------------------------------------------------

/// Find the nearest radial in a sweep to the desired azimuth, rejecting if too far.
fn find_nearest_radial<'a>(
    sweep: &'a crate::nexrad::level2::Level2Sweep,
    desired_az: f64,
) -> Option<&'a crate::nexrad::level2::RadialData> {
    let radial = sweep.radials.iter().min_by(|a, b| {
        let da = azimuth_difference(a.azimuth as f64, desired_az);
        let db = azimuth_difference(b.azimuth as f64, desired_az);
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let az_diff = azimuth_difference(radial.azimuth as f64, desired_az);
    if az_diff > (radial.azimuth_spacing as f64 * 1.5).max(2.0) {
        return None;
    }
    Some(radial)
}

/// Sample a gate value from moment data at a given slant range.
fn sample_gate(moment: &crate::nexrad::level2::MomentData, slant_range_km: f64) -> f32 {
    let first_gate_km = moment.first_gate_range as f64 / 1000.0;
    let gate_size_km = moment.gate_size as f64 / 1000.0;
    if gate_size_km <= 0.0 { return f32::NAN; }
    let gate_f = (slant_range_km - first_gate_km) / gate_size_km;
    let gate_idx = gate_f.round() as i64;
    if gate_idx < 0 || gate_idx >= moment.gate_count as i64 { return f32::NAN; }
    moment.data[gate_idx as usize]
}

/// Beam center altitude in km using 4/3 Earth radius refraction model.
fn beam_altitude_km(slant_range_km: f64, elev_rad: f64) -> f64 {
    let r = slant_range_km * 1000.0;
    let re = RE_PRIME_KM * 1000.0;
    let alt_m = ((r * r) + (re * re) + (2.0 * r * re * elev_rad.sin())).sqrt() - re;
    alt_m / 1000.0
}

/// Draw a line with alpha blending.
fn draw_line_aa(pixels: &mut [u8], w: usize, h: usize, x0: f64, y0: f64, x1: f64, y1: f64, color: [u8; 4]) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    let steps = (len * 1.5) as usize;
    if steps == 0 { return; }
    let alpha = color[3] as f32 / 255.0;
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let px = (x0 + dx * t) as i32;
        let py = (y0 + dy * t) as i32;
        if px >= 0 && px < w as i32 && py >= 0 && py < h as i32 {
            let idx = (py as usize * w + px as usize) * 4;
            pixels[idx] = (pixels[idx] as f32 * (1.0 - alpha) + color[0] as f32 * alpha) as u8;
            pixels[idx + 1] = (pixels[idx + 1] as f32 * (1.0 - alpha) + color[1] as f32 * alpha) as u8;
            pixels[idx + 2] = (pixels[idx + 2] as f32 * (1.0 - alpha) + color[2] as f32 * alpha) as u8;
            pixels[idx + 3] = 255;
        }
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

/// Absolute angular difference between two azimuths in degrees, in the range [0, 180].
fn azimuth_difference(a: f64, b: f64) -> f64 {
    let diff = (a - b).abs() % 360.0;
    if diff > 180.0 {
        360.0 - diff
    } else {
        diff
    }
}
