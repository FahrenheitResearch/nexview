use crate::nexrad::{Level2Sweep, RadarProduct, RadarSite};
use crate::render::ColorTable;
use rayon::prelude::*;

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
        let mut pixels = vec![0u8; size * size * 4];
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

        // Inverse rendering: for each pixel, compute azimuth and range,
        // look up the correct radial and gate, and assign the color.
        // This guarantees every pixel is filled with no gaps.
        // Parallelized per-row with rayon for multi-core rendering.
        let row_chunks: Vec<Vec<u8>> = (0..size).into_par_iter().map(|py| {
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

                // Find the closest radial by azimuth using binary search
                let radial_idx = match azimuths.binary_search_by(|a| {
                    a.partial_cmp(&(az_deg as f32)).unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    Ok(i) => indices[i],
                    Err(i) => {
                        if i == 0 {
                            let dist_first = az_deg as f32 - azimuths[0];
                            let dist_last = 360.0 - azimuths[azimuths.len() - 1] + az_deg as f32;
                            if dist_last.abs() < dist_first.abs() {
                                indices[azimuths.len() - 1]
                            } else {
                                indices[0]
                            }
                        } else if i >= azimuths.len() {
                            let dist_last = az_deg as f32 - azimuths[azimuths.len() - 1];
                            let dist_first = 360.0 - az_deg as f32 + azimuths[0];
                            if dist_first.abs() < dist_last.abs() {
                                indices[0]
                            } else {
                                indices[azimuths.len() - 1]
                            }
                        } else {
                            let d_prev = (az_deg as f32 - azimuths[i - 1]).abs();
                            let d_next = (azimuths[i] - az_deg as f32).abs();
                            if d_prev <= d_next { indices[i - 1] } else { indices[i] }
                        }
                    }
                };

                // Check azimuth is within this radial's beam width
                let radial = &sweep.radials[radial_idx];
                let half_spacing = radial.azimuth_spacing as f64 / 2.0;
                let mut az_diff = (az_deg - radial.azimuth as f64).abs();
                if az_diff > 180.0 {
                    az_diff = 360.0 - az_diff;
                }
                if az_diff > half_spacing + 0.1 {
                    continue;
                }

                let moment = match radial.moments.iter().find(|m| m.product == product) {
                    Some(m) => m,
                    None => continue,
                };

                let gate_offset = range_m - moment.first_gate_range as f64;
                if gate_offset < 0.0 {
                    continue;
                }
                let gate_idx = (gate_offset / moment.gate_size as f64) as usize;
                if gate_idx >= moment.data.len() {
                    continue;
                }

                let value = moment.data[gate_idx];
                if value.is_nan() {
                    continue;
                }

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
}
