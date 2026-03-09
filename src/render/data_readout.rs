use crate::nexrad::{Level2File, RadarProduct};

/// Result of looking up data at the cursor position
pub struct CursorReadout {
    pub value: f32,
    pub range_km: f64,
    pub azimuth_deg: f64,
    pub height_agl_km: f64,
    pub elevation_angle: f32,
    pub product: RadarProduct,
}

/// Compute the great-circle distance (km) between two lat/lon points using haversine.
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0; // Earth radius in km
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    r * c
}

/// Compute azimuth (degrees, 0=N clockwise) from point 1 to point 2.
fn azimuth_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();
    let dlon = (lon2 - lon1).to_radians();

    let x = dlon.sin() * lat2.cos();
    let y = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    let bearing = x.atan2(y).to_degrees();
    (bearing + 360.0) % 360.0
}

/// Compute beam height using 4/3 Earth refraction model.
/// h = sqrt(r^2 + Re'^2 + 2*r*Re'*sin(elev)) - Re'
/// where Re' = 8495 km (4/3 * 6371 km)
fn beam_height_km(slant_range_km: f64, elevation_deg: f64) -> f64 {
    let re_prime = 8495.0; // 4/3 Earth radius in km
    let elev_rad = elevation_deg.to_radians();
    let r = slant_range_km;
    (r * r + re_prime * re_prime + 2.0 * r * re_prime * elev_rad.sin()).sqrt() - re_prime
}

/// Format a data value with appropriate precision and units.
pub fn format_value(value: f32, product: RadarProduct) -> String {
    let unit = product.unit();
    match product {
        RadarProduct::CorrelationCoefficient => format!("{:.3} {}", value, unit).trim().to_string(),
        RadarProduct::SpecificDiffPhase => format!("{:.2} {}", value, unit),
        RadarProduct::DifferentialReflectivity => format!("{:.2} {}", value, unit),
        RadarProduct::Velocity | RadarProduct::SpectrumWidth => {
            // Convert m/s to knots for display
            let kts = value * 1.94384;
            format!("{:.1} {}", kts, unit)
        }
        _ => format!("{:.1} {}", value, unit),
    }
}

/// Look up the radar data value at a given cursor lat/lon.
///
/// Returns None if no data is found at that location.
pub fn lookup_cursor_data(
    cursor_lat: f64,
    cursor_lon: f64,
    site_lat: f64,
    site_lon: f64,
    file: &Level2File,
    sweep_index: usize,
    product: RadarProduct,
) -> Option<CursorReadout> {
    let sweep = file.sweeps.get(sweep_index)?;

    let range_km = haversine_km(site_lat, site_lon, cursor_lat, cursor_lon);
    let az = azimuth_deg(site_lat, site_lon, cursor_lat, cursor_lon);

    // Find the closest radial by azimuth using binary search
    // First, we need the radials sorted by azimuth (they usually are)
    let radial_idx = find_closest_radial(sweep, az)?;
    let radial = &sweep.radials[radial_idx];

    // Check that the azimuth is within a reasonable tolerance (half the azimuth spacing)
    let az_diff = angular_diff(radial.azimuth as f64, az);
    let max_az_diff = radial.azimuth_spacing as f64 * 0.75;
    if az_diff > max_az_diff {
        return None;
    }

    // Find the moment data for the requested product
    let moment = radial.moments.iter().find(|m| m.product == product)?;

    // Compute gate index from range
    let range_m = range_km * 1000.0;
    let first_gate_m = moment.first_gate_range as f64;
    let gate_size_m = moment.gate_size as f64;

    if range_m < first_gate_m {
        return None;
    }

    let gate_idx = ((range_m - first_gate_m) / gate_size_m) as usize;
    if gate_idx >= moment.data.len() {
        return None;
    }

    let value = moment.data[gate_idx];
    if value.is_nan() {
        return None;
    }

    let elevation_angle = radial.elevation;
    let height_agl_km = beam_height_km(range_km, elevation_angle as f64);

    Some(CursorReadout {
        value,
        range_km,
        azimuth_deg: az,
        height_agl_km,
        elevation_angle,
        product,
    })
}

/// Find the index of the closest radial to target azimuth.
fn find_closest_radial(sweep: &crate::nexrad::Level2Sweep, target_az: f64) -> Option<usize> {
    if sweep.radials.is_empty() {
        return None;
    }

    let mut best_idx = 0;
    let mut best_diff = f64::MAX;

    for (i, radial) in sweep.radials.iter().enumerate() {
        let diff = angular_diff(radial.azimuth as f64, target_az);
        if diff < best_diff {
            best_diff = diff;
            best_idx = i;
        }
    }

    Some(best_idx)
}

/// Compute the smallest angular difference between two angles in degrees.
fn angular_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}
