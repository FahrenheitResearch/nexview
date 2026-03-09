use super::{Level2File, Level2Sweep, RadarProduct, RadarSite};

/// Detected mesocyclone signature from velocity data.
pub struct MesocycloneDetection {
    pub lat: f64,
    pub lon: f64,
    pub azimuth_deg: f32,
    pub range_km: f32,
    pub max_shear: f32,        // s^-1
    pub max_delta_v: f32,      // m/s
    pub strength: RotationStrength,
    pub base_height_km: f32,
    pub diameter_km: f32,
}

/// Detected Tornadic Vortex Signature (gate-to-gate shear).
pub struct TVSDetection {
    pub lat: f64,
    pub lon: f64,
    pub azimuth_deg: f32,
    pub range_km: f32,
    pub max_delta_v: f32,      // m/s
    pub gate_to_gate_shear: f32, // s^-1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationStrength {
    Weak,
    Moderate,
    Strong,
}

impl RotationStrength {
    fn from_shear(shear: f32) -> Self {
        if shear > 0.012 {
            RotationStrength::Strong
        } else if shear > 0.008 {
            RotationStrength::Moderate
        } else {
            RotationStrength::Weak
        }
    }
}

impl std::fmt::Display for RotationStrength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RotationStrength::Weak => write!(f, "Weak"),
            RotationStrength::Moderate => write!(f, "Moderate"),
            RotationStrength::Strong => write!(f, "Strong"),
        }
    }
}

/// Default Nyquist velocity estimate (m/s) when not available from data.
const DEFAULT_NYQUIST: f32 = 30.0;

/// Minimum azimuthal shear threshold for mesocyclone candidacy (s^-1).
const MESO_SHEAR_THRESHOLD: f32 = 0.004;

/// Minimum gate-to-gate delta-V for TVS detection (m/s).
const TVS_DELTA_V_THRESHOLD: f32 = 36.0;

/// Maximum range from radar for TVS detection (km).
const TVS_MAX_RANGE_KM: f32 = 100.0;

/// Maximum number of lowest elevation tilts to check for TVS.
const TVS_MAX_TILTS: usize = 2;

/// Convert azimuth/range relative to a radar site into lat/lon.
fn azimuth_range_to_latlon(site: &RadarSite, azimuth_deg: f32, range_km: f32) -> (f64, f64) {
    let az_rad = (azimuth_deg as f64).to_radians();
    let lat = site.lat + (range_km as f64 * az_rad.cos()) / 111.139;
    let lon = site.lon + (range_km as f64 * az_rad.sin()) / (111.139 * site.lat.to_radians().cos());
    (lat, lon)
}

/// Dealias a velocity difference using the Nyquist interval.
fn dealias(delta_v: f32, nyquist: f32) -> f32 {
    let mut dv = delta_v;
    if dv > nyquist {
        dv -= 2.0 * nyquist;
    } else if dv < -nyquist {
        dv += 2.0 * nyquist;
    }
    dv
}

/// A flagged rotation candidate gate before grouping.
struct RotationCandidate {
    azimuth_deg: f32,
    range_km: f32,
    shear: f32,
    delta_v: f32,
    elevation_angle: f32,
}

/// Automated rotation detection from NEXRAD Level 2 velocity data.
pub struct RotationDetector;

impl RotationDetector {
    /// Detect mesocyclones and TVS from velocity sweeps in a Level2 file.
    ///
    /// Returns `(mesocyclones, tvs_detections)`.
    pub fn detect(
        file: &Level2File,
        site: &RadarSite,
    ) -> (Vec<MesocycloneDetection>, Vec<TVSDetection>) {
        let velocity_sweeps: Vec<&Level2Sweep> = file
            .sweeps
            .iter()
            .filter(|s| {
                s.radials
                    .iter()
                    .any(|r| r.moments.iter().any(|m| m.product == RadarProduct::Velocity))
            })
            .collect();

        let mut meso_detections: Vec<MesocycloneDetection> = Vec::new();
        let mut tvs_detections: Vec<TVSDetection> = Vec::new();

        // Sort sweeps by elevation number so we can limit TVS to lowest tilts.
        let mut sorted_sweeps = velocity_sweeps.clone();
        sorted_sweeps.sort_by_key(|s| s.elevation_number);

        for (sweep_idx, sweep) in sorted_sweeps.iter().enumerate() {
            let is_tvs_eligible = sweep_idx < TVS_MAX_TILTS;

            let (candidates, tvs) = Self::scan_sweep(sweep, site, is_tvs_eligible);

            tvs_detections.extend(tvs);

            // Group adjacent rotation candidates into mesocyclone detections.
            let grouped = Self::group_candidates(&candidates, site);
            meso_detections.extend(grouped);
        }

        (meso_detections, tvs_detections)
    }

    /// Scan a single sweep for rotation candidates and TVS signatures.
    fn scan_sweep(
        sweep: &Level2Sweep,
        site: &RadarSite,
        check_tvs: bool,
    ) -> (Vec<RotationCandidate>, Vec<TVSDetection>) {
        let mut candidates = Vec::new();
        let mut tvs_list = Vec::new();

        if sweep.radials.is_empty() {
            return (candidates, tvs_list);
        }

        // Sort radials by azimuth.
        let mut radials: Vec<&super::level2::RadialData> = sweep.radials.iter().collect();
        radials.sort_by(|a, b| {
            a.azimuth
                .partial_cmp(&b.azimuth)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let nyquist = DEFAULT_NYQUIST;

        for i in 0..radials.len() {
            let next_i = (i + 1) % radials.len();
            let rad_a = radials[i];
            let rad_b = radials[next_i];

            // Find velocity moment data for each radial.
            let vel_a = match rad_a
                .moments
                .iter()
                .find(|m| m.product == RadarProduct::Velocity)
            {
                Some(v) => v,
                None => continue,
            };
            let vel_b = match rad_b
                .moments
                .iter()
                .find(|m| m.product == RadarProduct::Velocity)
            {
                Some(v) => v,
                None => continue,
            };

            // Compute angular distance between the two radials.
            let mut delta_az = (rad_b.azimuth - rad_a.azimuth).abs();
            if delta_az > 180.0 {
                delta_az = 360.0 - delta_az;
            }
            let delta_az_rad = (delta_az as f64).to_radians();

            // Use the smaller gate count so we stay in bounds.
            let gate_count = vel_a.data.len().min(vel_b.data.len());
            let gate_size_km = vel_a.gate_size as f32 / 1000.0;
            let first_gate_km = vel_a.first_gate_range as f32 / 1000.0;

            let mid_azimuth = if (rad_b.azimuth - rad_a.azimuth).abs() > 180.0 {
                // Wrap around 0/360.
                let sum = rad_a.azimuth + rad_b.azimuth + 360.0;
                (sum / 2.0) % 360.0
            } else {
                (rad_a.azimuth + rad_b.azimuth) / 2.0
            };

            for g in 0..gate_count {
                let va = vel_a.data[g];
                let vb = vel_b.data[g];

                // Skip missing/range-folded gates.
                if va.is_nan() || vb.is_nan() {
                    continue;
                }

                let range_km = first_gate_km + g as f32 * gate_size_km;
                if range_km < 1.0 {
                    continue; // skip unreasonably close gates
                }

                let raw_delta = vb - va;
                let delta_v = dealias(raw_delta, nyquist);

                // Azimuthal shear: delta_v / angular_distance_km.
                let angular_distance_km = range_km * delta_az_rad as f32;
                if angular_distance_km < 0.001 {
                    continue;
                }
                let shear = delta_v.abs() / angular_distance_km;

                // Mesocyclone candidate check.
                if shear > MESO_SHEAR_THRESHOLD {
                    candidates.push(RotationCandidate {
                        azimuth_deg: mid_azimuth,
                        range_km,
                        shear,
                        delta_v: delta_v.abs(),
                        elevation_angle: sweep.elevation_angle,
                    });
                }

                // TVS check: gate-to-gate on adjacent radials, same range.
                if check_tvs
                    && delta_v.abs() > TVS_DELTA_V_THRESHOLD
                    && range_km <= TVS_MAX_RANGE_KM
                {
                    let (lat, lon) = azimuth_range_to_latlon(site, mid_azimuth, range_km);
                    tvs_list.push(TVSDetection {
                        lat,
                        lon,
                        azimuth_deg: mid_azimuth,
                        range_km,
                        max_delta_v: delta_v.abs(),
                        gate_to_gate_shear: shear,
                    });
                }
            }
        }

        (candidates, tvs_list)
    }

    /// Group adjacent flagged gates into contiguous mesocyclone regions.
    fn group_candidates(
        candidates: &[RotationCandidate],
        site: &RadarSite,
    ) -> Vec<MesocycloneDetection> {
        if candidates.is_empty() {
            return Vec::new();
        }

        // Simple spatial clustering: sort by (azimuth, range) and merge nearby gates.
        let mut sorted: Vec<usize> = (0..candidates.len()).collect();
        sorted.sort_by(|&a, &b| {
            let ca = &candidates[a];
            let cb = &candidates[b];
            ca.azimuth_deg
                .partial_cmp(&cb.azimuth_deg)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    ca.range_km
                        .partial_cmp(&cb.range_km)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });

        // Maximum gap (km) to consider gates as part of the same region.
        const MAX_GAP_KM: f32 = 5.0;
        // Maximum azimuth gap (degrees) for same cluster.
        const MAX_AZ_GAP_DEG: f32 = 3.0;
        // Minimum cluster size to report.
        const MIN_CLUSTER_SIZE: usize = 3;

        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut current_group: Vec<usize> = vec![sorted[0]];

        for w in sorted.windows(2) {
            let ca = &candidates[w[0]];
            let cb = &candidates[w[1]];

            let range_gap = (cb.range_km - ca.range_km).abs();
            let mut az_gap = (cb.azimuth_deg - ca.azimuth_deg).abs();
            if az_gap > 180.0 {
                az_gap = 360.0 - az_gap;
            }

            if range_gap <= MAX_GAP_KM && az_gap <= MAX_AZ_GAP_DEG {
                current_group.push(w[1]);
            } else {
                if current_group.len() >= MIN_CLUSTER_SIZE {
                    groups.push(current_group.clone());
                }
                current_group = vec![w[1]];
            }
        }
        if current_group.len() >= MIN_CLUSTER_SIZE {
            groups.push(current_group);
        }

        // Convert each group into a MesocycloneDetection.
        groups
            .iter()
            .map(|group| {
                let mut sum_az = 0.0_f64;
                let mut sum_range = 0.0_f64;
                let mut max_shear: f32 = 0.0;
                let mut max_dv: f32 = 0.0;
                let mut min_range: f32 = f32::MAX;
                let mut max_range: f32 = f32::MIN;
                let mut elev_angle: f32 = 0.0;

                for &idx in group {
                    let c = &candidates[idx];
                    sum_az += c.azimuth_deg as f64;
                    sum_range += c.range_km as f64;
                    if c.shear > max_shear {
                        max_shear = c.shear;
                    }
                    if c.delta_v > max_dv {
                        max_dv = c.delta_v;
                    }
                    if c.range_km < min_range {
                        min_range = c.range_km;
                    }
                    if c.range_km > max_range {
                        max_range = c.range_km;
                    }
                    elev_angle = c.elevation_angle;
                }

                let n = group.len() as f64;
                let center_az = (sum_az / n) as f32;
                let center_range = (sum_range / n) as f32;
                let diameter_km = max_range - min_range;

                let (lat, lon) = azimuth_range_to_latlon(site, center_az, center_range);

                // Estimate base height using standard beam propagation (simplified).
                let elev_rad = (elev_angle as f64).to_radians();
                let base_height_km =
                    (center_range as f64 * elev_rad.sin() + (center_range as f64).powi(2) / (2.0 * 6371.0 * 1.21))
                        as f32;

                MesocycloneDetection {
                    lat,
                    lon,
                    azimuth_deg: center_az,
                    range_km: center_range,
                    max_shear,
                    max_delta_v: max_dv,
                    strength: RotationStrength::from_shear(max_shear),
                    base_height_km,
                    diameter_km,
                }
            })
            .collect()
    }
}
