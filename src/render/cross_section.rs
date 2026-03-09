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
