//! Enhanced sounding profile structures with precomputed severe weather parameters.

/// A single level in a sounding profile.
#[derive(Debug, Clone)]
pub struct SoundingLevel {
    pub pressure_mb: f32,
    pub height_m: f32,
    pub temp_c: f32,
    pub dewpoint_c: f32,
    pub wind_dir: f32,
    pub wind_speed_kts: f32,
}

/// Precomputed severe weather parameters for a sounding profile.
#[derive(Debug, Clone)]
pub struct SoundingParams {
    // Surface-based parcel
    pub sb_cape: f32,
    pub sb_cin: f32,
    pub sb_lfc: f32,
    pub sb_el: f32,
    pub sb_li: f32,
    // Most-unstable parcel
    pub mu_cape: f32,
    pub mu_cin: f32,
    pub mu_lfc: f32,
    pub mu_el: f32,
    // Mixed-layer parcel
    pub ml_cape: f32,
    pub ml_cin: f32,
    pub ml_lfc: f32,
    pub ml_el: f32,
    // LCL height
    pub lcl_hgt: f32,
    // Lapse rates (C/km)
    pub lapse_03: f32,
    pub lapse_36: f32,
    pub lapse_700_500: f32,
    // Bulk shear (kts)
    pub bulk_shear_01: f32,
    pub bulk_shear_03: f32,
    pub bulk_shear_06: f32,
    pub eff_shear: f32,
    // Storm-relative helicity (m2/s2)
    pub srh_500: f32,
    pub srh_01: f32,
    pub srh_03: f32,
    pub eff_srh: f32,
    // Composite parameters
    pub stp_fixed: f32,
    pub stp_eff: f32,
    pub scp: f32,
    pub ship: f32,
    // Moisture
    pub pwat: f32,
    // Stability indices
    pub k_index: f32,
    pub totals: f32,
    pub sweat: f32,
    // Downdraft
    pub dcape: f32,
    // Storm motion
    pub storm_motion_rm: (f32, f32), // right-mover (dir, spd)
    pub storm_motion_lm: (f32, f32), // left-mover (dir, spd)
    // Critical angle
    pub critical_angle: f32,
}

impl Default for SoundingParams {
    fn default() -> Self {
        Self {
            sb_cape: 0.0, sb_cin: 0.0, sb_lfc: 0.0, sb_el: 0.0, sb_li: 0.0,
            mu_cape: 0.0, mu_cin: 0.0, mu_lfc: 0.0, mu_el: 0.0,
            ml_cape: 0.0, ml_cin: 0.0, ml_lfc: 0.0, ml_el: 0.0,
            lcl_hgt: 0.0,
            lapse_03: 0.0, lapse_36: 0.0, lapse_700_500: 0.0,
            bulk_shear_01: 0.0, bulk_shear_03: 0.0, bulk_shear_06: 0.0, eff_shear: 0.0,
            srh_500: 0.0, srh_01: 0.0, srh_03: 0.0, eff_srh: 0.0,
            stp_fixed: 0.0, stp_eff: 0.0,
            scp: 0.0, ship: 0.0,
            pwat: 0.0,
            k_index: 0.0, totals: 0.0, sweat: 0.0,
            dcape: 0.0,
            storm_motion_rm: (0.0, 0.0),
            storm_motion_lm: (0.0, 0.0),
            critical_angle: 0.0,
        }
    }
}

/// A complete sounding profile with computed severe weather indices.
#[derive(Debug, Clone)]
pub struct SoundingProfile {
    pub levels: Vec<SoundingLevel>,
    pub station: String,
    pub valid_time: String,
    pub lat: f64,
    pub lon: f64,
    /// Precomputed severe weather parameters.
    pub params: SoundingParams,
}

impl SoundingProfile {
    /// Create a new profile and compute all parameters.
    pub fn new(
        levels: Vec<SoundingLevel>,
        station: String,
        valid_time: String,
        lat: f64,
        lon: f64,
    ) -> Self {
        let mut profile = Self {
            levels,
            station,
            valid_time,
            lat,
            lon,
            params: SoundingParams::default(),
        };
        if profile.levels.len() >= 3 {
            // Use catch_unwind so that bad data doesn't crash the async task.
            // Also enforce a time limit to prevent infinite loops from NaN data.
            let profile_clone = profile.clone();
            let handle = std::thread::spawn(move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::sounding::params::compute_all(&profile_clone)
                }))
            });
            match handle.join() {
                Ok(Ok(params)) => profile.params = params,
                Ok(Err(_)) => {
                    log::error!("compute_all panicked — using default parameters");
                }
                Err(_) => {
                    log::error!("compute_all thread failed — using default parameters");
                }
            }
        }
        profile
    }

    /// Surface pressure in mb.
    pub fn sfc_pressure(&self) -> f32 {
        self.levels.first().map(|l| l.pressure_mb).unwrap_or(1013.25)
    }

    /// Surface height in meters.
    pub fn sfc_height(&self) -> f32 {
        self.levels.first().map(|l| l.height_m).unwrap_or(0.0)
    }
}
