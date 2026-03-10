//! Convert NEXRAD Level2 radar data to the format expected by the
//! DeepGuess ResNet3D tornado prediction model.
//!
//! Expected input shape: (B, 24, 8, 128, 128)
//!   - 24 channels = 6 products × 4 elevation angles
//!   - 8 temporal frames (consecutive volume scans, ~5 min apart)
//!   - 128×128 spatial grid, storm-centered at 1 km/pixel
//!
//! Products (in order): REF, VEL, SW, ZDR, CC, KDP
//! Elevations (in order): 0.5°, 0.9°, 1.3°, 1.8°

use crate::nexrad::{Level2File, Level2Sweep, RadarProduct, RadarSite};

/// The 6 dual-pol products in the order the model expects them.
/// KDP channel uses DifferentialPhase (PHI) from Level2, then we derive KDP
/// by computing the range gradient of PhiDP on the Cartesian grid.
pub const MODEL_PRODUCTS: [RadarProduct; 6] = [
    RadarProduct::Reflectivity,
    RadarProduct::Velocity,
    RadarProduct::SpectrumWidth,
    RadarProduct::DifferentialReflectivity,
    RadarProduct::CorrelationCoefficient,
    RadarProduct::DifferentialPhase, // raw PHI — converted to KDP in post-processing
];

/// Target elevation angles (degrees). Model expects data at these 4 tilts.
pub const MODEL_ELEVATIONS: [f32; 4] = [0.5, 0.9, 1.3, 1.8];

/// Normalization ranges for each product (min, max).
/// Values are clamped to [min, max] then mapped to [0, 1].
/// Derived from the tornet-temporal dataset value distributions.
const NORM_RANGES: [(f32, f32); 6] = [
    (-20.0, 80.0),   // REF (dBZ)
    (-36.0, 36.0),   // VEL (m/s)  — matched to Nyquist
    (0.0, 30.0),     // SW  (m/s)
    (-4.0, 8.0),     // ZDR (dB)
    (0.0, 1.05),     // CC  (unitless)
    (-2.0, 10.0),    // KDP (°/km)
];

/// Normalize a raw value for a given product index (0-5) to [0, 1].
fn normalize_value(val: f32, product_idx: usize) -> f32 {
    let (vmin, vmax) = NORM_RANGES[product_idx];
    let clamped = val.clamp(vmin, vmax);
    (clamped - vmin) / (vmax - vmin)
}

/// Fill value for missing/no-data pixels.
/// The tornet dataset uses 0.0 for all missing data regardless of product.
fn fill_value(_product_idx: usize) -> f32 {
    0.0
}

/// Spatial grid size
pub const GRID_SIZE: usize = 128;

/// Spatial resolution in meters per pixel
pub const GRID_RESOLUTION_M: f64 = 1000.0;

/// A converted radar sequence ready for model inference.
#[derive(Debug)]
pub struct RadarSequence {
    /// Flat buffer: [frames][channels][y][x] in row-major order
    /// Shape: (num_frames, 24, 128, 128)
    pub data: Vec<f32>,
    /// Number of temporal frames
    pub num_frames: usize,
    /// Center lat/lon of the storm crop
    pub center_lat: f64,
    pub center_lon: f64,
    /// Station used
    pub station: String,
}

impl RadarSequence {
    /// Total elements per frame = 24 channels × 128 × 128
    pub const FRAME_SIZE: usize = 24 * GRID_SIZE * GRID_SIZE;

    /// Convert a sequence of Level2Files into model input.
    ///
    /// `files` - consecutive volume scans (ideally 8, ~40 min)
    /// `storm_lat`, `storm_lon` - center of the storm cell to analyze
    /// `site` - radar site location
    pub fn from_files(
        files: &[Level2File],
        storm_lat: f64,
        storm_lon: f64,
        site: &RadarSite,
    ) -> Option<Self> {
        if files.is_empty() {
            return None;
        }

        let num_frames = files.len().min(8);
        let mut data = vec![f32::NAN; num_frames * Self::FRAME_SIZE];

        for (fi, file) in files.iter().take(num_frames).enumerate() {
            Self::extract_frame(file, site, storm_lat, storm_lon, fi, &mut data);
        }

        // Convert PHI (ch20-23) to KDP via spatial gradient, then normalize all
        let mut phi_scratch = vec![0.0f32; GRID_SIZE * GRID_SIZE];
        for fi in 0..num_frames {
            // PHI → KDP: compute gradient of PhiDP to get specific diff phase
            for ei in 0..4 {
                let phi_ch = 5 * 4 + ei; // product 5 (PHI/KDP), elevation ei
                let offset = fi * Self::FRAME_SIZE + phi_ch * GRID_SIZE * GRID_SIZE;
                phi_scratch.copy_from_slice(&data[offset..offset + GRID_SIZE * GRID_SIZE]);
                let phi_grid = &phi_scratch;

                // Compute KDP as spatial gradient magnitude (°/km at 1km/pixel)
                for y in 0..GRID_SIZE {
                    for x in 0..GRID_SIZE {
                        let idx = y * GRID_SIZE + x;
                        let phi = phi_grid[idx];
                        if phi.is_nan() {
                            data[offset + idx] = f32::NAN;
                            continue;
                        }
                        // Use forward difference in range (radial) direction
                        // Approximate with x-gradient as a simple estimate
                        let kdp = if x + 1 < GRID_SIZE {
                            let phi_next = phi_grid[idx + 1];
                            if phi_next.is_nan() {
                                f32::NAN
                            } else {
                                let mut diff = phi_next - phi;
                                // Handle phase wrapping
                                if diff > 180.0 { diff -= 360.0; }
                                if diff < -180.0 { diff += 360.0; }
                                // KDP should be positive (one-way), divide by 2
                                // and per km (1 pixel = 1 km)
                                (diff / 2.0).clamp(-2.0, 10.0)
                            }
                        } else {
                            f32::NAN
                        };
                        data[offset + idx] = kdp;
                    }
                }
            }

            // Now normalize all channels
            for pi in 0..6 {
                for ei in 0..4 {
                    let ch = pi * 4 + ei;
                    let offset = fi * Self::FRAME_SIZE + ch * GRID_SIZE * GRID_SIZE;
                    for i in 0..GRID_SIZE * GRID_SIZE {
                        let v = &mut data[offset + i];
                        if v.is_nan() {
                            *v = 0.0; // missing data
                        } else {
                            *v = normalize_value(*v, pi);
                        }
                    }
                }
            }
        }

        Some(RadarSequence {
            data,
            num_frames,
            center_lat: storm_lat,
            center_lon: storm_lon,
            station: site.id.to_string(),
        })
    }

    /// Extract one frame (one volume scan) into the flat data buffer.
    fn extract_frame(
        file: &Level2File,
        site: &RadarSite,
        storm_lat: f64,
        storm_lon: f64,
        frame_idx: usize,
        data: &mut [f32],
    ) {
        let frame_offset = frame_idx * Self::FRAME_SIZE;

        for (pi, &product) in MODEL_PRODUCTS.iter().enumerate() {
            for (ei, &target_elev) in MODEL_ELEVATIONS.iter().enumerate() {
                let channel_idx = pi * 4 + ei;
                let channel_offset = frame_offset + channel_idx * GRID_SIZE * GRID_SIZE;

                // Find the sweep closest to the target elevation that has this product
                let sweep = Self::find_sweep(file, product, target_elev);

                if let Some(sweep) = sweep {
                    Self::sample_sweep_to_grid(
                        sweep, product, site, storm_lat, storm_lon,
                        &mut data[channel_offset..channel_offset + GRID_SIZE * GRID_SIZE],
                    );
                }
                // If no sweep found, data remains NaN → will be zeroed later
            }
        }
    }

    /// Find the sweep closest to a target elevation angle that contains the given product.
    fn find_sweep<'a>(
        file: &'a Level2File,
        product: RadarProduct,
        target_elevation: f32,
    ) -> Option<&'a Level2Sweep> {
        // Also check super-res variants
        let products_to_check = match product {
            RadarProduct::Reflectivity => vec![RadarProduct::SuperResReflectivity, RadarProduct::Reflectivity],
            RadarProduct::Velocity => vec![RadarProduct::SuperResVelocity, RadarProduct::Velocity],
            _ => vec![product],
        };

        file.sweeps.iter()
            .filter(|s| {
                s.radials.iter().any(|r| {
                    r.moments.iter().any(|m| products_to_check.contains(&m.product))
                })
            })
            .min_by(|a, b| {
                let da = (a.elevation_angle - target_elevation).abs();
                let db = (b.elevation_angle - target_elevation).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Sample a sweep onto a 128×128 Cartesian grid centered on (storm_lat, storm_lon).
    fn sample_sweep_to_grid(
        sweep: &Level2Sweep,
        product: RadarProduct,
        site: &RadarSite,
        storm_lat: f64,
        storm_lon: f64,
        grid: &mut [f32],
    ) {
        // Compute storm position relative to radar in meters
        let (storm_x_m, storm_y_m) = latlon_to_xy(
            site.lat, site.lon, storm_lat, storm_lon,
        );

        // Grid covers GRID_SIZE × GRID_RESOLUTION_M centered on the storm
        let half_extent = (GRID_SIZE as f64 / 2.0) * GRID_RESOLUTION_M;

        // Build sorted azimuth lookup
        let mut radial_info: Vec<(f32, usize)> = sweep.radials.iter()
            .enumerate()
            .map(|(i, r)| (r.azimuth, i))
            .collect();
        radial_info.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let azimuths: Vec<f32> = radial_info.iter().map(|(az, _)| *az).collect();
        let indices: Vec<usize> = radial_info.iter().map(|(_, i)| *i).collect();

        // Also check super-res variants
        let products_to_check = match product {
            RadarProduct::Reflectivity => vec![RadarProduct::SuperResReflectivity, RadarProduct::Reflectivity],
            RadarProduct::Velocity => vec![RadarProduct::SuperResVelocity, RadarProduct::Velocity],
            _ => vec![product],
        };

        for gy in 0..GRID_SIZE {
            for gx in 0..GRID_SIZE {
                // World position of this grid cell (meters from radar)
                let world_x = storm_x_m - half_extent + (gx as f64 + 0.5) * GRID_RESOLUTION_M;
                let world_y = storm_y_m + half_extent - (gy as f64 + 0.5) * GRID_RESOLUTION_M;

                // Convert to polar (range, azimuth) from radar
                let range_m = (world_x * world_x + world_y * world_y).sqrt();
                let mut az_deg = world_x.atan2(world_y).to_degrees();
                if az_deg < 0.0 { az_deg += 360.0; }

                // Find nearest radial
                let ridx = match azimuths.binary_search_by(|a| {
                    a.partial_cmp(&(az_deg as f32)).unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    Ok(i) => indices[i],
                    Err(i) => {
                        if i == 0 {
                            indices[0]
                        } else if i >= azimuths.len() {
                            indices[azimuths.len() - 1]
                        } else {
                            let d_prev = (az_deg as f32 - azimuths[i - 1]).abs();
                            let d_next = (azimuths[i] - az_deg as f32).abs();
                            if d_prev <= d_next { indices[i - 1] } else { indices[i] }
                        }
                    }
                };

                let radial = &sweep.radials[ridx];
                let moment = match radial.moments.iter().find(|m| products_to_check.contains(&m.product)) {
                    Some(m) => m,
                    None => continue,
                };

                let gate_offset = range_m - moment.first_gate_range as f64;
                if gate_offset < 0.0 { continue; }
                let gate_idx = (gate_offset / moment.gate_size as f64) as usize;
                if gate_idx >= moment.data.len() { continue; }

                let value = moment.data[gate_idx];
                grid[gy * GRID_SIZE + gx] = value;
            }
        }
    }

    /// Reshape the data into the ResNet3D model's expected format: (1, 24, T, 128, 128)
    /// returned as a flat Vec<f32> in row-major (C) order.
    pub fn to_model_input(&self) -> Vec<f32> {
        self.to_model_input_nch(24)
    }

    /// Reshape the data into the Swin3D model's expected format: (1, 12, T, 128, 128)
    /// Only uses the first 3 products (REF, VEL, SW) × 4 elevations = 12 channels.
    pub fn to_model_input_12ch(&self) -> Vec<f32> {
        self.to_model_input_nch(12)
    }

    /// Internal: reshape data for a model expecting `num_ch` channels.
    /// Channels are taken in order from the 24-channel layout:
    ///   0-3: REF×4elev, 4-7: VEL×4elev, 8-11: SW×4elev, 12+: dual-pol
    fn to_model_input_nch(&self, num_ch: usize) -> Vec<f32> {
        let t = self.num_frames.min(8);
        let mut output = vec![0.0f32; num_ch * 8 * GRID_SIZE * GRID_SIZE];

        for c in 0..num_ch {
            for ti in 0..8 {
                let src_frame = if ti < t { ti } else { t - 1 }; // pad with last frame
                for y in 0..GRID_SIZE {
                    for x in 0..GRID_SIZE {
                        let src_idx = src_frame * Self::FRAME_SIZE
                            + c * GRID_SIZE * GRID_SIZE
                            + y * GRID_SIZE + x;
                        let dst_idx = c * 8 * GRID_SIZE * GRID_SIZE
                            + ti * GRID_SIZE * GRID_SIZE
                            + y * GRID_SIZE + x;
                        output[dst_idx] = self.data[src_idx];
                    }
                }
            }
        }

        // Debug: log per-channel stats
        let products = ["REF", "VEL", "SW", "ZDR", "CC", "KDP"];
        for c in 0..num_ch {
            let start = c * 8 * GRID_SIZE * GRID_SIZE;
            let end = start + 8 * GRID_SIZE * GRID_SIZE;
            let slice = &output[start..end];
            let non_zero: Vec<f32> = slice.iter().filter(|&&v| v != 0.0).copied().collect();
            if !non_zero.is_empty() {
                let min = non_zero.iter().cloned().fold(f32::INFINITY, f32::min);
                let max = non_zero.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                let mean = non_zero.iter().sum::<f32>() / non_zero.len() as f32;
                let pi = c / 4;
                let pname = if pi < products.len() { products[pi] } else { "?" };
                log::debug!(
                    "  ch{:02} {} {:.1}°: min={:.1} max={:.1} mean={:.1} fill={:.0}%",
                    c, pname, [0.5, 0.9, 1.3, 1.8][c % 4],
                    min, max, mean,
                    (1.0 - non_zero.len() as f32 / (8.0 * GRID_SIZE as f32 * GRID_SIZE as f32)) * 100.0,
                );
            }
        }

        output
    }

    /// Save the sequence as NPZ for debugging/validation against the Python pipeline.
    pub fn save_npz(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        // Simple NPY format for the data array
        let t = self.num_frames;
        let shape = [t, 24, GRID_SIZE, GRID_SIZE];
        let npy_data = self.to_npy_bytes(&shape);
        std::fs::write(path, &npy_data)?;
        Ok(())
    }

    /// Encode as raw NPY (NumPy) format bytes.
    fn to_npy_bytes(&self, shape: &[usize]) -> Vec<u8> {
        let mut buf = Vec::new();
        // Magic: \x93NUMPY
        buf.extend_from_slice(b"\x93NUMPY");
        // Version 1.0
        buf.push(1);
        buf.push(0);
        // Header
        let shape_str: Vec<String> = shape.iter().map(|s| s.to_string()).collect();
        let header = format!(
            "{{'descr': '<f4', 'fortran_order': False, 'shape': ({},), }}",
            shape_str.join(", ")
        );
        // Pad to multiple of 64
        let pad_len = 64 - ((10 + header.len()) % 64);
        let padded_header = format!("{}{}\n", header, " ".repeat(pad_len - 1));
        let header_len = padded_header.len() as u16;
        buf.extend_from_slice(&header_len.to_le_bytes());
        buf.extend_from_slice(padded_header.as_bytes());
        // Data
        for &v in &self.data[..shape.iter().product::<usize>()] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf
    }
}

/// Convert lat/lon to x/y meters relative to an origin point.
/// Uses simple equirectangular approximation (accurate enough at storm scale).
fn latlon_to_xy(origin_lat: f64, origin_lon: f64, lat: f64, lon: f64) -> (f64, f64) {
    let m_per_deg_lat = 111_320.0;
    let m_per_deg_lon = 111_320.0 * (origin_lat.to_radians().cos());
    let x = (lon - origin_lon) * m_per_deg_lon;
    let y = (lat - origin_lat) * m_per_deg_lat;
    (x, y)
}

/// Result of tornado prediction inference
#[derive(Debug, Clone)]
pub struct TornadoPrediction {
    /// Probability that a tornado is occurring right now (0.0–1.0)
    pub detection_prob: f32,
    /// Probability that a tornado will occur (0.0–1.0)
    pub prediction_prob: f32,
    /// Combined score
    pub combined_score: f32,
    /// Center of analyzed storm cell
    pub storm_lat: f64,
    pub storm_lon: f64,
    /// Station ID
    pub station: String,
    /// Number of frames used
    pub num_frames: usize,
    /// True if model has separate detection/prediction heads (ResNet3D)
    pub dual_head: bool,
}

impl TornadoPrediction {
    /// Recommended threshold for actionable alerts (from model card, optimized for CSI)
    pub const THRESHOLD: f32 = 0.66;

    pub fn is_significant(&self) -> bool {
        self.prediction_prob >= Self::THRESHOLD || self.detection_prob >= Self::THRESHOLD
    }

    pub fn risk_level(&self) -> &'static str {
        let max_prob = self.prediction_prob.max(self.detection_prob);
        if max_prob >= 0.9 { "EXTREME" }
        else if max_prob >= 0.75 { "HIGH" }
        else if max_prob >= Self::THRESHOLD { "MODERATE" }
        else if max_prob >= 0.4 { "LOW" }
        else { "MINIMAL" }
    }
}
