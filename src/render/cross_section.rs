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

/// Cached voxel grid for 3D cross-section rendering.
/// Building this is expensive; rotating/ray-marching is cheap.
pub struct VoxelGrid {
    pub data: Vec<f32>,
    pub nx: usize,
    pub ny: usize,
    pub nz: usize,
    pub total_ground_km: f64,
    pub max_altitude_km: f64,
    pub depth_km: f64,
    // Cache key: invalidate when these change
    pub start: (f64, f64),
    pub end: (f64, f64),
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

    /// Build the voxel grid for 3D cross-section (expensive, should be cached).
    pub fn build_voxel_grid(
        file: &Level2File,
        product: RadarProduct,
        color_table: &ColorTable,
        site: &RadarSite,
        start: (f64, f64),
        end: (f64, f64),
    ) -> Option<VoxelGrid> {
        if file.sweeps.is_empty() {
            return None;
        }

        let max_altitude_km: f64 = 20.0;
        let total_ground_km = ground_distance_km(start.0, start.1, end.0, end.1);
        if total_ground_km < 0.01 {
            return None;
        }

        let dlat = end.0 - start.0;
        let dlon = end.1 - start.1;
        let line_len_deg = (dlat * dlat + dlon * dlon).sqrt();
        if line_len_deg < 1e-8 { return None; }
        let perp_lat = -dlon / line_len_deg;
        let perp_lon = dlat / line_len_deg;

        let depth_km = (total_ground_km * 0.20).min(40.0).max(5.0);
        let depth_deg = depth_km / 111.139;

        let vox_x: usize = 160;
        let vox_y: usize = 80;
        let vox_z: usize = 32;
        let vox_total = vox_x * vox_y * vox_z;
        let mut voxels = vec![f32::NAN; vox_total];

        for gz in 0..vox_z {
            let z_t = (gz as f64 / (vox_z - 1) as f64) * 2.0 - 1.0;
            let offset_lat = perp_lat * z_t * depth_deg;
            let offset_lon = perp_lon * z_t * depth_deg;

            for gx in 0..vox_x {
                let x_t = gx as f64 / (vox_x - 1).max(1) as f64;
                let lat = start.0 + x_t * dlat + offset_lat;
                let lon = start.1 + x_t * dlon + offset_lon;
                let ground_dist = ground_distance_km(site.lat, site.lon, lat, lon);
                let az = azimuth_deg(site.lat, site.lon, lat, lon);

                if ground_dist < 0.1 { continue; }

                let mut col_samples: Vec<(f64, f32)> = Vec::new();

                for sweep in &file.sweeps {
                    let radial = match find_nearest_radial(sweep, az) {
                        Some(r) => r, None => continue,
                    };
                    let moment = match radial.moments.iter().find(|m| m.product == product) {
                        Some(m) => m, None => continue,
                    };
                    let elev_rad = (radial.elevation as f64).to_radians();
                    if elev_rad.cos().abs() <= 1e-6 { continue; }
                    let slant_range_km = ground_dist / elev_rad.cos();
                    if slant_range_km < 0.0 { continue; }
                    let value = sample_gate(moment, slant_range_km);
                    if value.is_nan() || value < color_table.min_value { continue; }

                    let alt_km = beam_altitude_km(slant_range_km, elev_rad);
                    if alt_km < 0.0 || alt_km > max_altitude_km { continue; }

                    let half_bw = (BEAMWIDTH_DEG / 2.0).to_radians();
                    let alt_top = beam_altitude_km(slant_range_km, elev_rad + half_bw);
                    let alt_bot = beam_altitude_km(slant_range_km, elev_rad - half_bw);
                    let beam_half = ((alt_top - alt_bot).abs() / 2.0).max(0.4);
                    let gy_top = ((1.0 - (alt_km + beam_half).min(max_altitude_km) / max_altitude_km) * (vox_y - 1) as f64) as usize;
                    let gy_bot = ((1.0 - (alt_km - beam_half).max(0.0) / max_altitude_km) * (vox_y - 1) as f64) as usize;
                    let clamped_val = value.min(color_table.max_value);
                    for gy in gy_top..=gy_bot.min(vox_y - 1) {
                        let vi = (gz * vox_y + gy) * vox_x + gx;
                        if voxels[vi].is_nan() {
                            voxels[vi] = clamped_val;
                        }
                    }
                    col_samples.push((alt_km, value.min(color_table.max_value)));
                }

                // Interpolate between adjacent sweeps
                if col_samples.len() >= 2 {
                    col_samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                    for pair in col_samples.windows(2) {
                        let (alt0, v0) = pair[0];
                        let (alt1, v1) = pair[1];
                        let gy0 = ((1.0 - alt0 / max_altitude_km) * (vox_y - 1) as f64) as usize;
                        let gy1 = ((1.0 - alt1 / max_altitude_km) * (vox_y - 1) as f64) as usize;
                        let (top, bot) = if gy0 < gy1 { (gy0, gy1) } else { (gy1, gy0) };
                        for gy in top..=bot.min(vox_y - 1) {
                            let vi = (gz * vox_y + gy) * vox_x + gx;
                            if !voxels[vi].is_nan() { continue; }
                            let alt = max_altitude_km * (1.0 - gy as f64 / (vox_y - 1) as f64);
                            let t_interp = if (alt1 - alt0).abs() > 0.001 {
                                ((alt - alt0) / (alt1 - alt0)).clamp(0.0, 1.0) as f32
                            } else { 0.5 };
                            voxels[vi] = v0 + (v1 - v0) * t_interp;
                        }
                    }
                }
            }
        }

        Some(VoxelGrid {
            data: voxels,
            nx: vox_x,
            ny: vox_y,
            nz: vox_z,
            total_ground_km,
            max_altitude_km,
            depth_km,
            start,
            end,
        })
    }

    /// Ray-march the cached voxel grid from a given camera angle (fast, uses rayon).
    pub fn render_from_voxels(
        grid: &VoxelGrid,
        color_table: &ColorTable,
        width: u32,
        height: u32,
        view_angle: f64,
        view_pitch: f64,
    ) -> Option<CrossSectionResult> {
        use rayon::prelude::*;

        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 { return None; }

        let angle_rad = view_angle.to_radians();
        let pitch_rad = view_pitch.to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let cos_p = pitch_rad.cos();
        let sin_p = pitch_rad.sin();

        let vert_exag = 2.5;
        let aspect = grid.total_ground_km / grid.max_altitude_km;
        let y_scale = vert_exag / aspect.max(1.0);
        let z_scale = grid.depth_km / grid.total_ground_km;

        let camera_dist = 3.8;

        let box_min = [-1.0f64, -y_scale, -z_scale];
        let box_max = [1.0f64, y_scale, z_scale];

        let voxel_alpha = 0.12_f32;
        let vox_x = grid.nx;
        let vox_y = grid.ny;
        let vox_z = grid.nz;

        // Pre-compute color LUT from voxel values to avoid per-sample color_table lookups
        // Quantize value range into 256 bins
        let lut_size = 256usize;
        let val_range = color_table.max_value - color_table.min_value;
        let lut: Vec<[u8; 4]> = (0..lut_size).map(|i| {
            let v = color_table.min_value + val_range * i as f32 / (lut_size - 1) as f32;
            color_table.color_for_value(v)
        }).collect();

        // Parallel ray march per row
        let row_pixels: Vec<Vec<u8>> = (0..h).into_par_iter().map(|py| {
            let mut row = vec![0u8; w * 4];

            // Background gradient for this row
            let t = py as f64 / h as f64;
            let bg_r = (10.0 + t * 10.0) as u8;
            let bg_g = (10.0 + t * 8.0) as u8;
            let bg_b = (18.0 + t * 14.0) as u8;
            for px in 0..w {
                let idx = px * 4;
                row[idx] = bg_r; row[idx + 1] = bg_g; row[idx + 2] = bg_b; row[idx + 3] = 255;
            }

            let ndc_y = (py as f64 / h as f64 - 0.5) / 2.0;

            for px in 0..w {
                let ndc_x = (px as f64 / w as f64 - 0.5) / 2.0;

                let view_dir_x = ndc_x;
                let view_dir_y = -ndc_y;
                let view_dir_z = 1.0;
                let len = (view_dir_x * view_dir_x + view_dir_y * view_dir_y + view_dir_z * view_dir_z).sqrt();
                let vdx = view_dir_x / len;
                let vdy = view_dir_y / len;
                let vdz = view_dir_z / len;

                // Inverse camera rotation
                let uy = vdy * cos_p + vdz * sin_p;
                let uz = -vdy * sin_p + vdz * cos_p;
                let dx = vdx * cos_a - uz * sin_a;
                let dy = uy;
                let dz = vdx * sin_a + uz * cos_a;

                let oy_c = sin_p * (-camera_dist);
                let oz_c = cos_p * (-camera_dist);
                let ox = -oz_c * sin_a;
                let oy = oy_c;
                let oz = oz_c * cos_a;

                // Ray-box intersection (slab method)
                let mut t_min = f64::NEG_INFINITY;
                let mut t_max = f64::INFINITY;
                let ray_o = [ox, oy, oz];
                let ray_d = [dx, dy, dz];

                let mut miss = false;
                for axis in 0..3 {
                    if ray_d[axis].abs() < 1e-12 {
                        if ray_o[axis] < box_min[axis] || ray_o[axis] > box_max[axis] {
                            miss = true;
                            break;
                        }
                    } else {
                        let t1 = (box_min[axis] - ray_o[axis]) / ray_d[axis];
                        let t2 = (box_max[axis] - ray_o[axis]) / ray_d[axis];
                        let (t_near, t_far) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
                        t_min = t_min.max(t_near);
                        t_max = t_max.min(t_far);
                    }
                }

                if miss || t_min > t_max || t_max < 0.0 { continue; }
                let t_start = t_min.max(0.0);

                let num_steps = 48;
                let step_size = (t_max - t_start) / num_steps as f64;

                let mut acc_r = 0.0_f32;
                let mut acc_g = 0.0_f32;
                let mut acc_b = 0.0_f32;
                let mut acc_a = 0.0_f32;

                for step in 0..num_steps {
                    if acc_a > 0.95 { break; }

                    let t = t_start + (step as f64 + 0.5) * step_size;
                    let sx = ox + dx * t;
                    let sy = oy + dy * t;
                    let sz = oz + dz * t;

                    let vx_f = (sx - box_min[0]) / (box_max[0] - box_min[0]) * (vox_x - 1) as f64;
                    let vy_f = (vox_y - 1) as f64 - (sy - box_min[1]) / (box_max[1] - box_min[1]) * (vox_y - 1) as f64;
                    let vz_f = (sz - box_min[2]) / (box_max[2] - box_min[2]) * (vox_z - 1) as f64;

                    let vxi = vx_f as usize;
                    let vyi = vy_f as usize;
                    let vzi = vz_f as usize;

                    if vxi >= vox_x || vyi >= vox_y || vzi >= vox_z { continue; }

                    let vi = (vzi * vox_y + vyi) * vox_x + vxi;
                    let value = grid.data[vi];
                    if value.is_nan() { continue; }

                    // LUT lookup instead of color_table.color_for_value
                    let lut_idx = ((value - color_table.min_value) / val_range * (lut_size - 1) as f32)
                        .clamp(0.0, (lut_size - 1) as f32) as usize;
                    let color = lut[lut_idx];
                    if color[3] == 0 { continue; }

                    let cr = color[0] as f32 / 255.0;
                    let cg = color[1] as f32 / 255.0;
                    let cb = color[2] as f32 / 255.0;
                    let ca = voxel_alpha;

                    acc_r += cr * ca * (1.0 - acc_a);
                    acc_g += cg * ca * (1.0 - acc_a);
                    acc_b += cb * ca * (1.0 - acc_a);
                    acc_a += ca * (1.0 - acc_a);
                }

                if acc_a > 0.01 {
                    let idx = px * 4;
                    let bg_rf = row[idx] as f32 / 255.0;
                    let bg_gf = row[idx + 1] as f32 / 255.0;
                    let bg_bf = row[idx + 2] as f32 / 255.0;
                    row[idx] = ((acc_r + bg_rf * (1.0 - acc_a)) * 255.0).min(255.0) as u8;
                    row[idx + 1] = ((acc_g + bg_gf * (1.0 - acc_a)) * 255.0).min(255.0) as u8;
                    row[idx + 2] = ((acc_b + bg_bf * (1.0 - acc_a)) * 255.0).min(255.0) as u8;
                }
            }
            row
        }).collect();

        // Flatten rows into final pixel buffer
        let mut pixels: Vec<u8> = Vec::with_capacity(w * h * 4);
        for row in &row_pixels {
            pixels.extend_from_slice(row);
        }

        // Draw wireframe box edges
        let frame_color = [120u8, 180, 230, 140];
        let box_corners: [(f64, f64, f64); 8] = [
            (box_min[0], box_min[1], box_min[2]),
            (box_max[0], box_min[1], box_min[2]),
            (box_max[0], box_max[1], box_min[2]),
            (box_min[0], box_max[1], box_min[2]),
            (box_min[0], box_min[1], box_max[2]),
            (box_max[0], box_min[1], box_max[2]),
            (box_max[0], box_max[1], box_max[2]),
            (box_min[0], box_max[1], box_max[2]),
        ];
        let box_edges: [(usize, usize); 12] = [
            (0,1),(1,2),(2,3),(3,0),
            (4,5),(5,6),(6,7),(7,4),
            (0,4),(1,5),(2,6),(3,7),
        ];
        let project = |x3d: f64, y3d: f64, z3d: f64| -> Option<(f64, f64)> {
            let rx = x3d * cos_a + z3d * sin_a;
            let ry = y3d;
            let rz = -x3d * sin_a + z3d * cos_a;
            let fy = ry * cos_p - rz * sin_p;
            let fz = ry * sin_p + rz * cos_p;
            let depth = fz + camera_dist;
            if depth < 0.1 { return None; }
            Some(((rx / depth * 2.0 + 0.5) * w as f64, (-fy / depth * 2.0 + 0.5) * h as f64))
        };
        for &(a, b) in &box_edges {
            let (ax, ay, az) = box_corners[a];
            let (bx, by, bz) = box_corners[b];
            if let (Some((sx0, sy0)), Some((sx1, sy1))) = (
                project(ax, ay, az), project(bx, by, bz),
            ) {
                draw_line_aa(&mut pixels, w, h, sx0, sy0, sx1, sy1, frame_color);
            }
        }

        Some(CrossSectionResult {
            pixels,
            width,
            height,
            max_range_km: grid.total_ground_km,
            max_altitude_km: grid.max_altitude_km,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Standard WSR-88D beamwidth in degrees.
const BEAMWIDTH_DEG: f64 = 0.95;

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
