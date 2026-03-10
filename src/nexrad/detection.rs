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
        if shear >= 0.015 {
            RotationStrength::Strong
        } else if shear >= 0.010 {
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
const MESO_SHEAR_THRESHOLD: f32 = 0.005;

/// Minimum gate-to-gate delta-V for TVS detection (m/s).
const TVS_DELTA_V_THRESHOLD: f32 = 40.0;

/// Maximum range from radar for TVS detection (km).
const TVS_MAX_RANGE_KM: f32 = 100.0;

/// Maximum range from radar for mesocyclone detection (km).
const MESO_MAX_RANGE_KM: f32 = 120.0;

/// Maximum number of lowest elevation tilts to check for TVS.
const TVS_MAX_TILTS: usize = 2;

/// Minimum reflectivity (dBZ) at the candidate location to report a mesocyclone.
const MIN_REFLECTIVITY_DBZ: f32 = 20.0;

/// Number of lowest tilts to consider for vertical continuity check.
const VERTICAL_CONTINUITY_TILTS: usize = 4;

/// Maximum horizontal offset (km) for matching meso detections across tilts.
const VERTICAL_MATCH_DISTANCE_KM: f32 = 10.0;

/// Minimum number of tilts a mesocyclone must appear on to be reported.
const MIN_TILT_COUNT: usize = 2;

/// Minimum cluster size (gates) to form a mesocyclone candidate.
const MIN_CLUSTER_SIZE: usize = 5;

/// Convert azimuth/range relative to a radar site into lat/lon.
fn azimuth_range_to_latlon(site: &RadarSite, azimuth_deg: f32, range_km: f32) -> (f64, f64) {
    let az_rad = (azimuth_deg as f64).to_radians();
    let lat = site.lat + (range_km as f64 * az_rad.cos()) / 111.139;
    let lon = site.lon + (range_km as f64 * az_rad.sin()) / (111.139 * site.lat.to_radians().cos());
    (lat, lon)
}

/// Dealias a velocity difference using the Nyquist interval.
/// Applies iterative correction until the value is within [-nyquist, nyquist].
fn dealias(delta_v: f32, nyquist: f32) -> f32 {
    let interval = 2.0 * nyquist;
    let mut dv = delta_v;
    while dv > nyquist {
        dv -= interval;
    }
    while dv < -nyquist {
        dv += interval;
    }
    dv
}

/// Get the effective Nyquist velocity for a sweep.
fn sweep_nyquist(sweep: &Level2Sweep) -> f32 {
    // Prefer the Nyquist parsed from the radial 'R' data block.
    if let Some(nv) = sweep.nyquist_velocity {
        if nv > 0.0 {
            return nv;
        }
    }
    // Fallback: check individual radials.
    for radial in &sweep.radials {
        if let Some(nv) = radial.nyquist_velocity {
            if nv > 0.0 {
                return nv;
            }
        }
    }
    DEFAULT_NYQUIST
}

/// A flagged rotation candidate gate before grouping.
struct RotationCandidate {
    azimuth_idx: usize,   // index into sorted radials array
    gate_idx: usize,      // gate index
    azimuth_deg: f32,
    range_km: f32,
    shear: f32,
    delta_v: f32,
    elevation_angle: f32,
}

/// A single-tilt mesocyclone detection before vertical continuity filtering.
struct SingleTiltMeso {
    lat: f64,
    lon: f64,
    azimuth_deg: f32,
    range_km: f32,
    max_shear: f32,
    max_delta_v: f32,
    elevation_angle: f32,
    diameter_km: f32,
    tilt_index: usize,
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

        // Also collect reflectivity sweeps for cross-checking.
        let reflectivity_sweeps: Vec<&Level2Sweep> = file
            .sweeps
            .iter()
            .filter(|s| {
                s.radials
                    .iter()
                    .any(|r| r.moments.iter().any(|m| m.product == RadarProduct::Reflectivity))
            })
            .collect();

        let mut tvs_detections: Vec<TVSDetection> = Vec::new();
        let mut single_tilt_mesos: Vec<SingleTiltMeso> = Vec::new();

        // Sort sweeps by elevation number so we can limit TVS to lowest tilts.
        let mut sorted_sweeps = velocity_sweeps.clone();
        sorted_sweeps.sort_by_key(|s| s.elevation_number);

        for (sweep_idx, sweep) in sorted_sweeps.iter().enumerate() {
            let is_tvs_eligible = sweep_idx < TVS_MAX_TILTS;
            let nyquist = sweep_nyquist(sweep);

            let (candidates, tvs) = Self::scan_sweep(sweep, site, is_tvs_eligible, nyquist);
            tvs_detections.extend(tvs);

            // Group adjacent rotation candidates into mesocyclone detections using
            // connected-components clustering.
            let grouped = Self::group_candidates(&candidates, site, sweep.elevation_angle, sweep_idx);
            single_tilt_mesos.extend(grouped);
        }

        // --- Reflectivity filter ---
        // Remove mesocyclone candidates where the coincident reflectivity is too low.
        let single_tilt_mesos = Self::filter_by_reflectivity(
            single_tilt_mesos,
            &reflectivity_sweeps,
            &sorted_sweeps,
        );

        // --- Vertical continuity filter ---
        // Only keep mesocyclones detected on >= MIN_TILT_COUNT of the lowest
        // VERTICAL_CONTINUITY_TILTS tilts.
        let meso_detections = Self::apply_vertical_continuity(single_tilt_mesos, site);

        (meso_detections, tvs_detections)
    }

    /// Scan a single sweep for rotation candidates and TVS signatures.
    fn scan_sweep(
        sweep: &Level2Sweep,
        site: &RadarSite,
        check_tvs: bool,
        nyquist: f32,
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

                // Skip near-zero velocities (likely ground clutter / clear air).
                if va.abs() < 1.0 && vb.abs() < 1.0 {
                    continue;
                }

                let range_km = first_gate_km + g as f32 * gate_size_km;
                if range_km < 1.0 {
                    continue; // skip unreasonably close gates
                }

                // --- Range filter for mesocyclone candidates ---
                if range_km > MESO_MAX_RANGE_KM {
                    continue;
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
                if shear >= MESO_SHEAR_THRESHOLD {
                    candidates.push(RotationCandidate {
                        azimuth_idx: i,
                        gate_idx: g,
                        azimuth_deg: mid_azimuth,
                        range_km,
                        shear,
                        delta_v: delta_v.abs(),
                        elevation_angle: sweep.elevation_angle,
                    });
                }

                // TVS check: gate-to-gate on adjacent radials, same range.
                if check_tvs
                    && delta_v.abs() >= TVS_DELTA_V_THRESHOLD
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

    /// Group adjacent flagged gates into contiguous mesocyclone regions using
    /// connected-components clustering.
    ///
    /// Two gates are considered connected if they are within 2 range gates and
    /// 2 azimuth bins of each other.
    fn group_candidates(
        candidates: &[RotationCandidate],
        site: &RadarSite,
        elevation_angle: f32,
        tilt_index: usize,
    ) -> Vec<SingleTiltMeso> {
        if candidates.is_empty() {
            return Vec::new();
        }

        // Build a spatial index: (azimuth_idx, gate_idx) -> candidate index.
        // Use connected-components via union-find.
        let n = candidates.len();
        let mut parent: Vec<usize> = (0..n).collect();
        let mut rank: Vec<usize> = vec![0; n];

        fn find(parent: &mut [usize], x: usize) -> usize {
            if parent[x] != x {
                parent[x] = find(parent, parent[x]);
            }
            parent[x]
        }

        fn union(parent: &mut [usize], rank: &mut [usize], a: usize, b: usize) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra == rb {
                return;
            }
            if rank[ra] < rank[rb] {
                parent[ra] = rb;
            } else if rank[ra] > rank[rb] {
                parent[rb] = ra;
            } else {
                parent[rb] = ra;
                rank[ra] += 1;
            }
        }

        // For efficient neighbor lookup, build a hashmap from (azimuth_idx, gate_idx).
        use std::collections::HashMap;
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (ci, c) in candidates.iter().enumerate() {
            let key = (c.azimuth_idx as i32, c.gate_idx as i32);
            grid.entry(key).or_default().push(ci);
        }

        // For each candidate, check neighbors within 2 azimuth bins and 2 range gates.
        for (ci, c) in candidates.iter().enumerate() {
            let ai = c.azimuth_idx as i32;
            let gi = c.gate_idx as i32;
            for da in -2i32..=2 {
                for dg in -2i32..=2 {
                    if da == 0 && dg == 0 {
                        continue;
                    }
                    let key = (ai + da, gi + dg);
                    if let Some(neighbors) = grid.get(&key) {
                        for &ni in neighbors {
                            union(&mut parent, &mut rank, ci, ni);
                        }
                    }
                }
            }
        }

        // Collect clusters.
        let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..n {
            let root = find(&mut parent, i);
            clusters.entry(root).or_default().push(i);
        }

        // Convert qualifying clusters into SingleTiltMeso detections.
        clusters
            .values()
            .filter(|group| group.len() >= MIN_CLUSTER_SIZE)
            .map(|group| {
                let mut sum_az = 0.0_f64;
                let mut sum_range = 0.0_f64;
                let mut max_shear: f32 = 0.0;
                let mut max_dv: f32 = 0.0;
                let mut min_range: f32 = f32::MAX;
                let mut max_range: f32 = f32::MIN;

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
                }

                let count = group.len() as f64;
                let center_az = (sum_az / count) as f32;
                let center_range = (sum_range / count) as f32;
                let diameter_km = max_range - min_range;

                let (lat, lon) = azimuth_range_to_latlon(site, center_az, center_range);

                SingleTiltMeso {
                    lat,
                    lon,
                    azimuth_deg: center_az,
                    range_km: center_range,
                    max_shear,
                    max_delta_v: max_dv,
                    elevation_angle,
                    diameter_km,
                    tilt_index,
                }
            })
            .collect()
    }

    /// Filter out mesocyclone candidates where the coincident reflectivity is
    /// below MIN_REFLECTIVITY_DBZ. Finds the reflectivity sweep closest in
    /// elevation to the velocity sweep and samples the reflectivity at the
    /// candidate's azimuth/range.
    fn filter_by_reflectivity(
        mesos: Vec<SingleTiltMeso>,
        reflectivity_sweeps: &[&Level2Sweep],
        _velocity_sweeps: &[&Level2Sweep],
    ) -> Vec<SingleTiltMeso> {
        if reflectivity_sweeps.is_empty() {
            // Can't filter without reflectivity data — let them through.
            return mesos;
        }

        mesos
            .into_iter()
            .filter(|m| {
                // Find the closest reflectivity sweep by elevation angle.
                let ref_sweep = reflectivity_sweeps
                    .iter()
                    .min_by(|a, b| {
                        let da = (a.elevation_angle - m.elevation_angle).abs();
                        let db = (b.elevation_angle - m.elevation_angle).abs();
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    });
                let ref_sweep = match ref_sweep {
                    Some(s) => s,
                    None => return true, // no ref sweep, keep it
                };

                // Find the radial closest in azimuth.
                let radial = ref_sweep.radials.iter().min_by(|a, b| {
                    let da = azimuth_diff(a.azimuth, m.azimuth_deg);
                    let db = azimuth_diff(b.azimuth, m.azimuth_deg);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                });
                let radial = match radial {
                    Some(r) => r,
                    None => return true,
                };

                // Find the reflectivity moment.
                let ref_moment = match radial
                    .moments
                    .iter()
                    .find(|mo| mo.product == RadarProduct::Reflectivity)
                {
                    Some(mo) => mo,
                    None => return true,
                };

                // Compute gate index for the candidate's range.
                let gate_size_km = ref_moment.gate_size as f32 / 1000.0;
                let first_gate_km = ref_moment.first_gate_range as f32 / 1000.0;
                if gate_size_km <= 0.0 {
                    return true;
                }
                let gate_idx = ((m.range_km - first_gate_km) / gate_size_km).round() as i32;
                if gate_idx < 0 || gate_idx as usize >= ref_moment.data.len() {
                    return false; // out of range
                }
                let ref_val = ref_moment.data[gate_idx as usize];
                if ref_val.is_nan() {
                    return false; // no reflectivity = likely no precip
                }
                ref_val >= MIN_REFLECTIVITY_DBZ
            })
            .collect()
    }

    /// Apply vertical continuity: only keep mesocyclones detected on at least
    /// MIN_TILT_COUNT of the lowest VERTICAL_CONTINUITY_TILTS tilts.
    /// Matching is done by horizontal distance < VERTICAL_MATCH_DISTANCE_KM.
    fn apply_vertical_continuity(
        single_tilt_mesos: Vec<SingleTiltMeso>,
        _site: &RadarSite,
    ) -> Vec<MesocycloneDetection> {
        if single_tilt_mesos.is_empty() {
            return Vec::new();
        }

        // Only consider mesos from the lowest VERTICAL_CONTINUITY_TILTS tilts.
        let relevant: Vec<&SingleTiltMeso> = single_tilt_mesos
            .iter()
            .filter(|m| m.tilt_index < VERTICAL_CONTINUITY_TILTS)
            .collect();

        if relevant.is_empty() {
            return Vec::new();
        }

        // Greedy clustering across tilts: for each meso on tilt 0, find matches
        // on other tilts within VERTICAL_MATCH_DISTANCE_KM.
        let mut used: Vec<bool> = vec![false; relevant.len()];
        let mut results: Vec<MesocycloneDetection> = Vec::new();

        for (i, m) in relevant.iter().enumerate() {
            if used[i] {
                continue;
            }

            // Collect matching detections across tilts.
            let mut matched_indices = vec![i];
            let mut matched_tilts = std::collections::HashSet::new();
            matched_tilts.insert(m.tilt_index);

            for (j, other) in relevant.iter().enumerate() {
                if j == i || used[j] {
                    continue;
                }
                if matched_tilts.contains(&other.tilt_index) {
                    continue; // already have a match for this tilt
                }
                let dist = horizontal_distance_km(m.lat, m.lon, other.lat, other.lon);
                if dist < VERTICAL_MATCH_DISTANCE_KM {
                    matched_indices.push(j);
                    matched_tilts.insert(other.tilt_index);
                }
            }

            if matched_tilts.len() < MIN_TILT_COUNT {
                continue; // Fails vertical continuity
            }

            // Mark all matched as used.
            for &idx in &matched_indices {
                used[idx] = true;
            }

            // Build the final detection from the strongest match.
            let best = matched_indices
                .iter()
                .map(|&idx| &relevant[idx])
                .max_by(|a, b| {
                    a.max_shear
                        .partial_cmp(&b.max_shear)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap();

            // Use the lowest-tilt detection for position (most representative).
            let lowest = matched_indices
                .iter()
                .map(|&idx| &relevant[idx])
                .min_by_key(|m| m.tilt_index)
                .unwrap();

            // Estimate base height using standard beam propagation (simplified).
            let elev_rad = (lowest.elevation_angle as f64).to_radians();
            let base_height_km = (lowest.range_km as f64 * elev_rad.sin()
                + (lowest.range_km as f64).powi(2) / (2.0 * 6371.0 * 1.21))
                as f32;

            results.push(MesocycloneDetection {
                lat: lowest.lat,
                lon: lowest.lon,
                azimuth_deg: lowest.azimuth_deg,
                range_km: lowest.range_km,
                max_shear: best.max_shear,
                max_delta_v: best.max_delta_v,
                strength: RotationStrength::from_shear(best.max_shear),
                base_height_km,
                diameter_km: best.diameter_km,
            });
        }

        results
    }
}

/// Compute the absolute azimuth difference in degrees, handling 0/360 wrap.
fn azimuth_diff(a: f32, b: f32) -> f32 {
    let d = (a - b).abs();
    if d > 180.0 { 360.0 - d } else { d }
}

/// Approximate horizontal distance (km) between two lat/lon points.
fn horizontal_distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f32 {
    let dlat = (lat2 - lat1) * 111.139;
    let dlon = (lon2 - lon1) * 111.139 * lat1.to_radians().cos();
    ((dlat * dlat + dlon * dlon).sqrt()) as f32
}
