use crate::nexrad::{Level2Sweep, RadarProduct, RadarSite};
use crate::render::ColorTable;

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
    /// Uses fast line-drawing per radial instead of per-pixel azimuth testing.
    pub fn render_sweep(
        sweep: &Level2Sweep,
        product: RadarProduct,
        site: &RadarSite,
        image_size: u32,
    ) -> Option<RenderedSweep> {
        let color_table = ColorTable::for_product(product);

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

        // Fast rendering: for each radial, draw a wedge by stepping along the
        // azimuth range and painting pixels along radial lines
        for radial in &sweep.radials {
            let moment = match radial.moments.iter().find(|m| m.product == product) {
                Some(m) => m,
                None => continue,
            };

            let az_rad = (radial.azimuth as f64).to_radians();
            let half_spacing = (radial.azimuth_spacing as f64 / 2.0).to_radians();

            // Draw multiple lines across the azimuth span to fill the wedge
            let num_lines = if radial.azimuth_spacing <= 0.5 { 3 } else { 5 };

            for line in 0..num_lines {
                let t = line as f64 / (num_lines - 1) as f64;
                let az = az_rad - half_spacing + t * 2.0 * half_spacing;
                let sin_az = az.sin();
                let cos_az = az.cos();

                for (gate_idx, &value) in moment.data.iter().enumerate() {
                    if value.is_nan() {
                        continue;
                    }

                    let color = color_table.color_for_value(value);
                    if color[3] == 0 {
                        continue;
                    }

                    let gate_start_m = moment.first_gate_range as f64 + gate_idx as f64 * moment.gate_size as f64;
                    let gate_end_m = gate_start_m + moment.gate_size as f64;

                    // Draw pixels along this radial line for the gate extent
                    let r_start = (gate_start_m * scale) as i32;
                    let r_end = (gate_end_m * scale) as i32;

                    // Step through the radial at ~1px intervals
                    let step = 1.max((r_end - r_start) / 3);
                    let mut r = r_start;
                    while r <= r_end {
                        let px = (center + r as f64 * sin_az) as i32;
                        let py = (center - r as f64 * cos_az) as i32;

                        if px >= 0 && px < size as i32 && py >= 0 && py < size as i32 {
                            let idx = (py as usize * size + px as usize) * 4;
                            pixels[idx] = color[0];
                            pixels[idx + 1] = color[1];
                            pixels[idx + 2] = color[2];
                            pixels[idx + 3] = color[3];
                        }
                        r += step;
                    }
                }
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
}
