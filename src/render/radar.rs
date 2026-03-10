use crate::nexrad::{Level2Sweep, RadarProduct, RadarSite};
use crate::render::ColorTable;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

/// Rendering mode for radar sweeps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Classic inverse-mapping (per-pixel lookup)
    Classic,
    /// Smooth forward-mapped triangulation with barycentric interpolation
    Smooth,
}

/// Renders radar sweep data into a pixel buffer for display
pub struct RadarRenderer;

/// Output of rendering a sweep
pub struct RenderedSweep {
    pub pixels: Vec<u8>, // RGBA
    pub width: u32,
    pub height: u32,
    pub center_lat: f64,
    pub center_lon: f64,
    pub range_km: f64,
}

impl RadarRenderer {
    /// Render a sweep for a given product into an RGBA image.
    pub fn render_sweep(
        sweep: &Level2Sweep,
        product: RadarProduct,
        site: &RadarSite,
        image_size: u32,
    ) -> Option<RenderedSweep> {
        Self::render_sweep_with_table(sweep, product, site, image_size, &ColorTable::for_product(product))
    }

    pub fn render_sweep_with_table(
        sweep: &Level2Sweep,
        product: RadarProduct,
        site: &RadarSite,
        image_size: u32,
        color_table: &ColorTable,
    ) -> Option<RenderedSweep> {

        // Find max range from the data
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

        let range_km = max_range_m / 1000.0;
        let size = image_size as usize;
        let center = size as f64 / 2.0;
        let scale = center / max_range_m;

        // Build a sorted azimuth lookup for the sweep so we can map any angle
        // to the correct radial via binary search
        let mut radial_indices: Vec<(f32, usize)> = sweep.radials.iter()
            .enumerate()
            .map(|(i, r)| (r.azimuth, i))
            .collect();
        radial_indices.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let azimuths: Vec<f32> = radial_indices.iter().map(|(az, _)| *az).collect();
        let indices: Vec<usize> = radial_indices.iter().map(|(_, i)| *i).collect();
        let n_az = azimuths.len();

        // Inverse rendering with bilinear interpolation between adjacent
        // gates and azimuths for GR2-quality smooth output.
        // Parallelized per-row with rayon for multi-core rendering.
        #[cfg(not(target_arch = "wasm32"))]
        let iter = (0..size).into_par_iter();
        #[cfg(target_arch = "wasm32")]
        let iter = 0..size;
        let row_chunks: Vec<Vec<u8>> = iter.map(|py| {
            let mut row = vec![0u8; size * 4];
            let dy = center - py as f64;

            for px in 0..size {
                let dx = px as f64 - center;

                let range_m = (dx * dx + dy * dy).sqrt() / scale;
                if range_m <= 0.0 || range_m > max_range_m {
                    continue;
                }

                // Azimuth: 0° = north, clockwise
                let mut az_deg = (dx.atan2(dy)).to_degrees();
                if az_deg < 0.0 {
                    az_deg += 360.0;
                }
                let az_f32 = az_deg as f32;

                // Find the two bracketing radials for bilinear interpolation
                let insert_pos = match azimuths.binary_search_by(|a| {
                    a.partial_cmp(&az_f32).unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    Ok(i) => i,
                    Err(i) => i,
                };

                // Get the two neighboring sorted indices with wrapping
                let lo_sorted = if insert_pos == 0 { n_az - 1 } else { insert_pos - 1 };
                let hi_sorted = if insert_pos >= n_az { 0 } else { insert_pos };

                let lo_idx = indices[lo_sorted];
                let hi_idx = indices[hi_sorted];
                let az_lo = azimuths[lo_sorted];
                let az_hi = azimuths[hi_sorted];

                // Compute azimuthal interpolation weight
                let mut az_span = az_hi - az_lo;
                if az_span < 0.0 { az_span += 360.0; }
                let mut az_off = az_f32 - az_lo;
                if az_off < 0.0 { az_off += 360.0; }

                // Skip if gap too large (missing radials)
                if az_span > 5.0 { continue; }
                let az_t = if az_span > 0.001 { (az_off / az_span) as f64 } else { 0.0 };

                // Sample both radials at the pixel's range with gate interpolation
                let val_lo = Self::sample_radial_interp(&sweep.radials[lo_idx], product, range_m);
                let val_hi = Self::sample_radial_interp(&sweep.radials[hi_idx], product, range_m);

                // Bilinear blend
                let value = match (val_lo, val_hi) {
                    (Some(v0), Some(v1)) => v0 + (v1 - v0) * az_t as f32,
                    (Some(v), None) => v,
                    (None, Some(v)) => v,
                    (None, None) => continue,
                };

                if value.is_nan() { continue; }

                let color = color_table.color_for_value(value);
                if color[3] == 0 {
                    continue;
                }

                let idx = px * 4;
                row[idx] = color[0];
                row[idx + 1] = color[1];
                row[idx + 2] = color[2];
                row[idx + 3] = color[3];
            }
            row
        }).collect();

        // Flatten rows into the pixel buffer
        let mut pixels = vec![0u8; size * size * 4];
        for (py, row) in row_chunks.into_iter().enumerate() {
            let start = py * size * 4;
            pixels[start..start + size * 4].copy_from_slice(&row);
        }

        Some(RenderedSweep {
            pixels,
            width: image_size,
            height: image_size,
            center_lat: site.lat,
            center_lon: site.lon,
            range_km,
        })
    }

    /// Sample a single radial at a given range with linear interpolation between gates.
    #[inline]
    fn sample_radial_interp(
        radial: &crate::nexrad::level2::RadialData,
        product: RadarProduct,
        range_m: f64,
    ) -> Option<f32> {
        let moment = radial.moments.iter().find(|m| m.product == product)?;
        let gate_offset = range_m - moment.first_gate_range as f64;
        if gate_offset < 0.0 { return None; }
        let gate_f = gate_offset / moment.gate_size as f64;
        let gate_lo = gate_f as usize;
        if gate_lo >= moment.data.len() { return None; }
        let v0 = moment.data[gate_lo];
        if v0.is_nan() { return None; }

        // Interpolate with next gate if available
        let gate_hi = gate_lo + 1;
        if gate_hi < moment.data.len() {
            let v1 = moment.data[gate_hi];
            if !v1.is_nan() {
                let t = (gate_f - gate_lo as f64) as f32;
                return Some(v0 + (v1 - v0) * t);
            }
        }
        Some(v0)
    }

    /// Smooth forward-mapped renderer using structured polar grid triangulation.
    ///
    /// Instead of per-pixel inverse lookups, this maps each gate quad
    /// (adjacent radials × adjacent gates) to screen space, splits into
    /// two triangles, and rasterizes with barycentric interpolation of
    /// the radar data values before applying the color table.
    pub fn render_sweep_smooth(
        sweep: &Level2Sweep,
        product: RadarProduct,
        site: &RadarSite,
        image_size: u32,
        color_table: &ColorTable,
    ) -> Option<RenderedSweep> {
        // Find max range
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

        let range_km = max_range_m / 1000.0;
        let size = image_size as usize;
        let center = size as f64 / 2.0;
        let scale = center / max_range_m;

        // Sort radials by azimuth for structured traversal
        let mut sorted_radials: Vec<usize> = (0..sweep.radials.len()).collect();
        sorted_radials.sort_by(|&a, &b| {
            sweep.radials[a].azimuth
                .partial_cmp(&sweep.radials[b].azimuth)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Build pixel buffer — use atomics for thread-safe writes, or just single-thread
        // the triangle rasterization (forward mapping is fast enough)
        let mut pixels = vec![0u8; size * size * 4];

        // For each pair of adjacent radials, for each pair of adjacent gates,
        // form a quad → two triangles → rasterize with interpolation
        let n_radials = sorted_radials.len();
        if n_radials < 2 {
            return None;
        }

        for ri in 0..n_radials {
            let ri_next = (ri + 1) % n_radials;
            let r0_idx = sorted_radials[ri];
            let r1_idx = sorted_radials[ri_next];

            let r0 = &sweep.radials[r0_idx];
            let r1 = &sweep.radials[r1_idx];

            let m0 = match r0.moments.iter().find(|m| m.product == product) {
                Some(m) => m,
                None => continue,
            };
            let m1 = match r1.moments.iter().find(|m| m.product == product) {
                Some(m) => m,
                None => continue,
            };

            // Check for azimuth wrap-around — skip if gap > 10° (missing radials)
            let mut az_gap = (r1.azimuth - r0.azimuth).abs();
            if az_gap > 180.0 { az_gap = 360.0 - az_gap; }
            if az_gap > 10.0 { continue; }

            let az0 = (r0.azimuth as f64).to_radians();
            let az1 = (r1.azimuth as f64).to_radians();
            let sin0 = az0.sin();
            let cos0 = az0.cos();
            let sin1 = az1.sin();
            let cos1 = az1.cos();

            let gate_count = m0.data.len().min(m1.data.len());
            if gate_count < 2 { continue; }

            let first_range0 = m0.first_gate_range as f64;
            let gate_size0 = m0.gate_size as f64;
            let first_range1 = m1.first_gate_range as f64;
            let gate_size1 = m1.gate_size as f64;

            for gi in 0..gate_count - 1 {
                // Get the 4 corner values
                let v00 = m0.data[gi];
                let v01 = m0.data[gi + 1];
                let v10 = m1.data[gi];
                let v11 = m1.data[gi + 1];

                // Skip if all corners are NaN (no data)
                let has_data = !v00.is_nan() || !v01.is_nan() || !v10.is_nan() || !v11.is_nan();
                if !has_data { continue; }

                // Compute screen coordinates for the 4 corners
                let range_inner0 = first_range0 + gi as f64 * gate_size0;
                let range_outer0 = first_range0 + (gi + 1) as f64 * gate_size0;
                let range_inner1 = first_range1 + gi as f64 * gate_size1;
                let range_outer1 = first_range1 + (gi + 1) as f64 * gate_size1;

                // Screen coords: x = center + range * sin(az) * scale
                //                 y = center - range * cos(az) * scale
                let p00 = (
                    center + range_inner0 * sin0 * scale,
                    center - range_inner0 * cos0 * scale,
                );
                let p01 = (
                    center + range_outer0 * sin0 * scale,
                    center - range_outer0 * cos0 * scale,
                );
                let p10 = (
                    center + range_inner1 * sin1 * scale,
                    center - range_inner1 * cos1 * scale,
                );
                let p11 = (
                    center + range_outer1 * sin1 * scale,
                    center - range_outer1 * cos1 * scale,
                );

                // Rasterize two triangles: (p00, p01, p10) and (p01, p11, p10)
                Self::rasterize_tri(
                    &mut pixels, size,
                    p00, p01, p10,
                    v00, v01, v10,
                    color_table,
                );
                Self::rasterize_tri(
                    &mut pixels, size,
                    p01, p11, p10,
                    v01, v11, v10,
                    color_table,
                );
            }
        }

        Some(RenderedSweep {
            pixels,
            width: image_size,
            height: image_size,
            center_lat: site.lat,
            center_lon: site.lon,
            range_km,
        })
    }

    /// Rasterize a single triangle with barycentric interpolation of data values.
    /// NaN values are treated as "no data" — if a vertex is NaN, use the average
    /// of the valid vertices instead (graceful degradation at data edges).
    fn rasterize_tri(
        pixels: &mut [u8],
        size: usize,
        p0: (f64, f64),
        p1: (f64, f64),
        p2: (f64, f64),
        v0: f32,
        v1: f32,
        v2: f32,
        color_table: &ColorTable,
    ) {
        // Replace NaN values with average of valid neighbors for edge smoothing
        let valid_sum: f32;
        let valid_count: u32;
        {
            let mut s = 0.0f32;
            let mut c = 0u32;
            if !v0.is_nan() { s += v0; c += 1; }
            if !v1.is_nan() { s += v1; c += 1; }
            if !v2.is_nan() { s += v2; c += 1; }
            valid_sum = s;
            valid_count = c;
        }
        if valid_count == 0 { return; }
        let fill = valid_sum / valid_count as f32;
        let v0 = if v0.is_nan() { fill } else { v0 };
        let v1 = if v1.is_nan() { fill } else { v1 };
        let v2 = if v2.is_nan() { fill } else { v2 };

        // Bounding box (clipped to image)
        let min_x = p0.0.min(p1.0).min(p2.0).floor().max(0.0) as usize;
        let max_x = p0.0.max(p1.0).max(p2.0).ceil().min(size as f64 - 1.0) as usize;
        let min_y = p0.1.min(p1.1).min(p2.1).floor().max(0.0) as usize;
        let max_y = p0.1.max(p1.1).max(p2.1).ceil().min(size as f64 - 1.0) as usize;

        // Precompute barycentric denominator
        let denom = (p1.1 - p2.1) * (p0.0 - p2.0) + (p2.0 - p1.0) * (p0.1 - p2.1);
        if denom.abs() < 1e-10 { return; } // degenerate triangle
        let inv_denom = 1.0 / denom;

        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let x = px as f64 + 0.5;
                let y = py as f64 + 0.5;

                // Barycentric coordinates
                let w0 = ((p1.1 - p2.1) * (x - p2.0) + (p2.0 - p1.0) * (y - p2.1)) * inv_denom;
                let w1 = ((p2.1 - p0.1) * (x - p2.0) + (p0.0 - p2.0) * (y - p2.1)) * inv_denom;
                let w2 = 1.0 - w0 - w1;

                // Point is inside triangle if all weights are non-negative
                if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 { continue; }

                // Interpolate data value
                let val = w0 as f32 * v0 + w1 as f32 * v1 + w2 as f32 * v2;

                let color = color_table.color_for_value(val);
                if color[3] == 0 { continue; }

                let idx = (py * size + px) * 4;
                pixels[idx] = color[0];
                pixels[idx + 1] = color[1];
                pixels[idx + 2] = color[2];
                pixels[idx + 3] = color[3];
            }
        }
    }
}
