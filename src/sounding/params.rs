//! Severe weather parameters computed from sounding profiles.
//!
//! Implements SHARPpy-equivalent calculations for CAPE/CIN, storm motion,
//! helicity, composite parameters, and stability indices.
//!
//! References:
//! - Thompson et al. (2003, 2007): STP, SCP, effective layer
//! - Bunkers et al. (2000): Storm motion
//! - Esterheld & Giuliano (2008): Critical angle

use super::profile::{SoundingParams, SoundingProfile};
use super::thermo;
use super::interp;

// ── Parcel types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum ParcelType {
    /// Surface-based: uses surface temperature and dewpoint
    SurfaceBased,
    /// Most-unstable: finds max theta-e in lowest 300mb
    MostUnstable,
    /// Mixed-layer: averages lowest 100mb
    MixedLayer,
}

/// CAPE/CIN result for a lifted parcel.
#[derive(Debug, Clone)]
pub struct ParcelResult {
    pub cape: f64,
    pub cin: f64,
    pub lfc: f64,  // LFC pressure (hPa)
    pub el: f64,   // EL pressure (hPa)
    pub li: f64,   // Lifted Index (C)
    pub lcl_p: f64, // LCL pressure (hPa)
    pub lcl_t: f64, // LCL temperature (C)
    /// Starting parcel temperature (C)
    pub parcel_t: f64,
    /// Starting parcel dewpoint (C)
    pub parcel_td: f64,
    /// Starting parcel pressure (hPa)
    pub parcel_p: f64,
}

impl Default for ParcelResult {
    fn default() -> Self {
        Self {
            cape: 0.0, cin: 0.0, lfc: 0.0, el: 0.0, li: 0.0,
            lcl_p: 0.0, lcl_t: 0.0,
            parcel_t: 0.0, parcel_td: 0.0, parcel_p: 0.0,
        }
    }
}

// ── Master computation ──────────────────────────────────────────────

/// Compute all severe weather parameters for a profile.
pub fn compute_all(profile: &SoundingProfile) -> SoundingParams {
    let mut p = SoundingParams::default();

    if profile.levels.len() < 3 {
        return p;
    }

    // CAPE/CIN for all three parcel types
    let sb = cape_cin(profile, ParcelType::SurfaceBased);
    let mu = cape_cin(profile, ParcelType::MostUnstable);
    let ml = cape_cin(profile, ParcelType::MixedLayer);

    p.sb_cape = sb.cape as f32;
    p.sb_cin = sb.cin as f32;
    p.sb_lfc = sb.lfc as f32;
    p.sb_el = sb.el as f32;
    p.sb_li = sb.li as f32;

    p.mu_cape = mu.cape as f32;
    p.mu_cin = mu.cin as f32;
    p.mu_lfc = mu.lfc as f32;
    p.mu_el = mu.el as f32;

    p.ml_cape = ml.cape as f32;
    p.ml_cin = ml.cin as f32;
    p.ml_lfc = ml.lfc as f32;
    p.ml_el = ml.el as f32;

    // LCL height (AGL) from ML parcel
    let lcl_h = interp::interp_height(profile, ml.lcl_p) - profile.sfc_height() as f64;
    p.lcl_hgt = lcl_h.max(0.0) as f32;

    // Lapse rates
    let (lr03, lr36, lr_700_500) = lapse_rates_all(profile);
    p.lapse_03 = lr03 as f32;
    p.lapse_36 = lr36 as f32;
    p.lapse_700_500 = lr_700_500 as f32;

    // Storm motion (Bunkers)
    let (rm, lm) = storm_motion_bunkers(profile);
    let (rm_dir, rm_spd) = thermo::wind_from_components(rm.0, rm.1);
    let (lm_dir, lm_spd) = thermo::wind_from_components(lm.0, lm.1);
    p.storm_motion_rm = (rm_dir as f32, rm_spd as f32);
    p.storm_motion_lm = (lm_dir as f32, lm_spd as f32);

    // Bulk shear
    p.bulk_shear_01 = bulk_shear(profile, 1000.0) as f32;
    p.bulk_shear_03 = bulk_shear(profile, 3000.0) as f32;
    p.bulk_shear_06 = bulk_shear(profile, 6000.0) as f32;

    // SRH
    p.srh_500 = srh(profile, 500.0, rm) as f32;
    p.srh_01 = srh(profile, 1000.0, rm) as f32;
    p.srh_03 = srh(profile, 3000.0, rm) as f32;

    // Effective layer
    let eff_layer = effective_inflow_layer(profile);
    if let Some((eff_bot, eff_top)) = eff_layer {
        p.eff_shear = effective_bulk_shear_from_layer(profile, eff_bot, eff_top, mu.el) as f32;
        p.eff_srh = srh_layer(profile, eff_bot, eff_top, rm) as f32;
    }

    // Composite parameters
    p.stp_fixed = stp_fixed(profile, &sb, p.bulk_shear_06 as f64, p.srh_01 as f64, lcl_h);
    p.stp_eff = stp_eff(profile, &ml, p.eff_shear as f64, p.eff_srh as f64, lcl_h);
    p.scp = scp(profile, &mu, p.eff_shear as f64, p.eff_srh as f64);
    p.ship = ship(profile, &mu, lr_700_500, p.bulk_shear_06 as f64);

    // Moisture
    p.pwat = precipitable_water(profile) as f32;

    // Stability indices
    p.k_index = k_index(profile) as f32;
    p.totals = totals_totals(profile) as f32;
    p.sweat = sweat_index(profile) as f32;

    // Downdraft CAPE
    p.dcape = dcape(profile) as f32;

    // Critical angle
    p.critical_angle = critical_angle(profile, rm) as f32;

    p
}

// ── CAPE / CIN ──────────────────────────────────────────────────────

/// Compute CAPE/CIN for a given parcel type.
///
/// Uses virtual temperature correction and integrates in height coordinates
/// matching the SHARPpy methodology.
pub fn cape_cin(profile: &SoundingProfile, parcel_type: ParcelType) -> ParcelResult {
    if profile.levels.len() < 3 {
        return ParcelResult::default();
    }

    let (parcel_t, parcel_td, parcel_p) = match parcel_type {
        ParcelType::SurfaceBased => {
            let sfc = &profile.levels[0];
            (sfc.temp_c as f64, sfc.dewpoint_c as f64, sfc.pressure_mb as f64)
        }
        ParcelType::MostUnstable => find_mu_parcel(profile),
        ParcelType::MixedLayer => find_ml_parcel(profile),
    };

    // Find LCL
    let (lcl_p, lcl_t) = thermo::lcl_pressure(parcel_t, parcel_td, parcel_p);

    // Integration
    let sfc_h = profile.sfc_height() as f64;
    let max_h = profile.levels.last().unwrap().height_m as f64;
    let dz = 10.0; // 10m steps

    let mut cape = 0.0_f64;
    let mut cin = 0.0_f64;
    let mut lfc_p = 0.0_f64;
    let mut el_p = 0.0_f64;
    let mut found_lfc = false;

    // Parcel starts at its initial conditions
    let parcel_w = thermo::mixing_ratio(parcel_td, parcel_p); // conserved below LCL

    let mut h = sfc_h;
    let mut p_t = parcel_t; // parcel temperature during ascent

    while h < max_h {
        let pres = interp::interp_pressure(profile, h);

        // Environment virtual temperature
        let env_t = interp::interp_temp(profile, pres);
        let env_td = interp::interp_dwpt(profile, pres);
        let env_w = thermo::mixing_ratio(env_td.min(env_t), pres);
        let tv_env = thermo::virtual_temp(env_t, env_w);

        // Parcel virtual temperature
        let p_w = if pres >= lcl_p {
            parcel_w // conserve mixing ratio below LCL
        } else {
            thermo::mixing_ratio(p_t, pres)
        };
        let tv_parcel = thermo::virtual_temp(p_t, p_w);

        let buoy = thermo::G * (tv_parcel - tv_env) / tv_env * dz;

        if tv_parcel > tv_env {
            cape += buoy;
            if !found_lfc {
                lfc_p = pres;
                found_lfc = true;
            }
            el_p = pres;
        } else if found_lfc {
            // Above EL — stop counting CAPE
            // but keep going to find highest EL
        } else {
            // Below LFC — count as CIN
            cin += buoy; // buoy is negative
        }

        // Step parcel temperature
        if pres >= lcl_p {
            // Dry adiabatic
            let next_pres = interp::interp_pressure(profile, h + dz);
            p_t = thermo::dry_lapse(p_t, pres, next_pres);
        } else {
            // Moist adiabatic
            let gamma_m = thermo::moist_lapse_rate(p_t, pres);
            p_t -= gamma_m * dz;
        }

        h += dz;
    }

    // Lifted Index: environment temp minus parcel temp at 500mb
    let parcel_500 = lift_parcel_to_pressure(parcel_t, parcel_td, parcel_p, lcl_p, 500.0);
    let env_500 = interp::interp_temp(profile, 500.0);
    let li = env_500 - parcel_500;

    ParcelResult {
        cape: cape.max(0.0),
        cin: cin.min(0.0),
        lfc: lfc_p,
        el: el_p,
        li,
        lcl_p,
        lcl_t,
        parcel_t,
        parcel_td,
        parcel_p,
    }
}

/// Find the most-unstable parcel: max theta-e in the lowest 300mb.
fn find_mu_parcel(profile: &SoundingProfile) -> (f64, f64, f64) {
    let sfc_p = profile.levels[0].pressure_mb as f64;
    let ptop = sfc_p - 300.0;

    let mut max_theta_e = f64::NEG_INFINITY;
    let mut best = (
        profile.levels[0].temp_c as f64,
        profile.levels[0].dewpoint_c as f64,
        profile.levels[0].pressure_mb as f64,
    );

    for level in &profile.levels {
        let p = level.pressure_mb as f64;
        if p < ptop {
            break;
        }
        let te = thermo::theta_e(level.temp_c as f64, level.dewpoint_c as f64, p);
        if te > max_theta_e {
            max_theta_e = te;
            best = (level.temp_c as f64, level.dewpoint_c as f64, p);
        }
    }
    best
}

/// Find the mixed-layer parcel: average T and Td in the lowest 100mb.
fn find_ml_parcel(profile: &SoundingProfile) -> (f64, f64, f64) {
    let sfc_p = profile.levels[0].pressure_mb as f64;
    let ptop = sfc_p - 100.0;

    let mut t_sum = 0.0;
    let mut td_sum = 0.0;
    let mut count = 0.0;

    // Sample every 10 hPa in the layer
    let mut p = sfc_p;
    while p >= ptop {
        let t = interp::interp_temp(profile, p);
        let td = interp::interp_dwpt(profile, p);
        t_sum += t;
        td_sum += td;
        count += 1.0;
        p -= 10.0;
    }

    if count > 0.0 {
        (t_sum / count, td_sum / count, sfc_p)
    } else {
        (profile.levels[0].temp_c as f64, profile.levels[0].dewpoint_c as f64, sfc_p)
    }
}

/// Lift a parcel to a target pressure, returning the parcel temperature at that level.
fn lift_parcel_to_pressure(t: f64, _td: f64, p_start: f64, lcl_p: f64, p_target: f64) -> f64 {
    if p_target >= lcl_p {
        // Below LCL: dry adiabatic
        return thermo::dry_lapse(t, p_start, p_target);
    }
    // Dry to LCL, then moist
    let t_lcl = thermo::dry_lapse(t, p_start, lcl_p);
    thermo::moist_lapse(t_lcl, lcl_p, p_target)
}

// ── Effective inflow layer ──────────────────────────────────────────

/// Thompson et al. (2007) effective inflow layer.
///
/// Finds the layer where CAPE >= 100 J/kg and CIN >= -250 J/kg.
/// Returns (bottom_pressure, top_pressure) or None.
pub fn effective_inflow_layer(profile: &SoundingProfile) -> Option<(f64, f64)> {
    if profile.levels.len() < 3 {
        return None;
    }

    let sfc_p = profile.levels[0].pressure_mb as f64;
    let min_p = sfc_p - 300.0;
    let mut eff_bot: Option<f64> = None;
    let mut eff_top: Option<f64> = None;

    // Sample every 50 hPa
    let mut p = sfc_p;
    while p >= min_p {
        let t = interp::interp_temp(profile, p);
        let td = interp::interp_dwpt(profile, p);

        // Quick CAPE/CIN check for this parcel
        let result = cape_cin_quick(profile, t, td, p);

        if result.cape >= 100.0 && result.cin >= -250.0 {
            if eff_bot.is_none() {
                eff_bot = Some(p);
            }
            eff_top = Some(p);
        } else if eff_bot.is_some() {
            break; // Exit once we leave the effective layer
        }

        p -= 50.0;
    }

    match (eff_bot, eff_top) {
        (Some(bot), Some(top)) => Some((bot, top)),
        _ => None,
    }
}

/// Quick CAPE/CIN computation for effective layer testing.
/// Simplified version that only needs approximate values.
fn cape_cin_quick(profile: &SoundingProfile, t: f64, td: f64, p: f64) -> ParcelResult {
    if profile.levels.len() < 3 {
        return ParcelResult::default();
    }

    let (lcl_p, _lcl_t) = thermo::lcl_pressure(t, td, p);
    let parcel_w = thermo::mixing_ratio(td, p);

    let sfc_h = interp::interp_height(profile, p);
    let max_h = profile.levels.last().unwrap().height_m as f64;
    let dz = 50.0; // coarser steps for speed

    let mut cape = 0.0_f64;
    let mut cin = 0.0_f64;
    let mut h = sfc_h;
    let mut p_t = t;
    let mut found_pos = false;

    while h < max_h {
        let pres = interp::interp_pressure(profile, h);
        let env_t = interp::interp_temp(profile, pres);
        let env_td = interp::interp_dwpt(profile, pres);
        let env_w = thermo::mixing_ratio(env_td.min(env_t), pres);
        let tv_env = thermo::virtual_temp(env_t, env_w);

        let p_w = if pres >= lcl_p { parcel_w } else { thermo::mixing_ratio(p_t, pres) };
        let tv_parcel = thermo::virtual_temp(p_t, p_w);
        let buoy = thermo::G * (tv_parcel - tv_env) / tv_env * dz;

        if tv_parcel > tv_env {
            cape += buoy;
            found_pos = true;
        } else if !found_pos {
            cin += buoy;
        }

        if pres >= lcl_p {
            let next_pres = interp::interp_pressure(profile, h + dz);
            p_t = thermo::dry_lapse(p_t, pres, next_pres);
        } else {
            let gamma_m = thermo::moist_lapse_rate(p_t, pres);
            p_t -= gamma_m * dz;
        }

        h += dz;
    }

    ParcelResult {
        cape: cape.max(0.0),
        cin: cin.min(0.0),
        ..ParcelResult::default()
    }
}

// ── Wind shear ──────────────────────────────────────────────────────

/// Bulk wind shear magnitude (kts) over a depth in meters AGL.
pub fn bulk_shear(profile: &SoundingProfile, depth_m: f64) -> f64 {
    let (u_sfc, v_sfc) = interp::interp_wind_at_height_agl(profile, 0.0);
    let (u_top, v_top) = interp::interp_wind_at_height_agl(profile, depth_m);
    ((u_top - u_sfc).powi(2) + (v_top - v_sfc).powi(2)).sqrt()
}

/// Effective-layer bulk shear (Thompson 2007).
///
/// Shear between the effective inflow base and 50% of the equilibrium level height.
pub fn effective_bulk_shear(profile: &SoundingProfile) -> f64 {
    let mu = cape_cin(profile, ParcelType::MostUnstable);
    if let Some((eff_bot, eff_top)) = effective_inflow_layer(profile) {
        effective_bulk_shear_from_layer(profile, eff_bot, eff_top, mu.el)
    } else {
        0.0
    }
}

fn effective_bulk_shear_from_layer(profile: &SoundingProfile, eff_bot: f64, _eff_top: f64, el_p: f64) -> f64 {
    if el_p <= 0.0 {
        return 0.0;
    }

    // Bottom of effective layer
    let (u_bot, v_bot) = interp::interp_wind_components(profile, eff_bot);

    // Top: 50% of distance from eff_bot to EL (in height space)
    let h_bot = interp::interp_height(profile, eff_bot);
    let h_el = interp::interp_height(profile, el_p);
    let h_top = h_bot + 0.5 * (h_el - h_bot);
    let p_top = interp::interp_pressure(profile, h_top);
    let (u_top, v_top) = interp::interp_wind_components(profile, p_top);

    ((u_top - u_bot).powi(2) + (v_top - v_bot).powi(2)).sqrt()
}

// ── Storm motion ────────────────────────────────────────────────────

/// Bunkers (2000) right-mover and left-mover storm motion.
///
/// Returns ((u_rm, v_rm), (u_lm, v_lm)) in knots.
pub fn storm_motion_bunkers(profile: &SoundingProfile) -> ((f64, f64), (f64, f64)) {
    // Mean wind 0-6 km
    let sfc_p = profile.sfc_pressure() as f64;
    let p_6km = interp::interp_pressure_at_height_agl(profile, 6000.0);
    let (u_mean, v_mean) = interp::mean_wind(profile, sfc_p, p_6km);

    // Shear vector: 0-6 km
    let (u_sfc, v_sfc) = interp::interp_wind_at_height_agl(profile, 0.0);
    let (u_6k, v_6k) = interp::interp_wind_at_height_agl(profile, 6000.0);
    let du = u_6k - u_sfc;
    let dv = v_6k - v_sfc;
    let shear_mag = (du * du + dv * dv).sqrt().max(0.001);

    // Deviation magnitude: 7.5 m/s converted to knots
    let d = thermo::ms_to_kts(7.5);

    // Perpendicular deviation (right-mover deviates to the right of shear)
    let u_rm = u_mean + d * dv / shear_mag;
    let v_rm = v_mean - d * du / shear_mag;
    let u_lm = u_mean - d * dv / shear_mag;
    let v_lm = v_mean + d * du / shear_mag;

    ((u_rm, v_rm), (u_lm, v_lm))
}

// ── Storm-relative helicity ─────────────────────────────────────────

/// Storm-relative helicity (m2/s2) for a fixed depth AGL.
///
/// storm_motion is (u, v) in knots.
pub fn srh(profile: &SoundingProfile, depth_m: f64, storm_motion: (f64, f64)) -> f64 {
    let sfc_h = profile.sfc_height() as f64;
    srh_height_range(profile, sfc_h, sfc_h + depth_m, storm_motion)
}

/// SRH for the effective inflow layer.
fn srh_layer(profile: &SoundingProfile, p_bot: f64, p_top: f64, storm_motion: (f64, f64)) -> f64 {
    let h_bot = interp::interp_height(profile, p_bot);
    let h_top = interp::interp_height(profile, p_top);
    // Extend effective layer top by some depth (use actual height range)
    srh_height_range(profile, h_bot, h_top.max(h_bot + 3000.0), storm_motion)
}

/// SRH computation over a height range (ASL).
fn srh_height_range(profile: &SoundingProfile, h_bot: f64, h_top: f64, storm_motion: (f64, f64)) -> f64 {
    // Convert storm motion from kts to m/s for SRH computation
    let u_storm = thermo::kts_to_ms(storm_motion.0);
    let v_storm = thermo::kts_to_ms(storm_motion.1);

    let step = 100.0; // 100m steps
    let mut h = h_bot;

    // Get initial wind
    let p_init = interp::interp_pressure(profile, h);
    let (u0_kt, v0_kt) = interp::interp_wind_components(profile, p_init);
    let mut prev_u = thermo::kts_to_ms(u0_kt);
    let mut prev_v = thermo::kts_to_ms(v0_kt);

    let mut total_srh = 0.0;
    h += step;

    while h <= h_top {
        let p = interp::interp_pressure(profile, h);
        let (u_kt, v_kt) = interp::interp_wind_components(profile, p);
        let u = thermo::kts_to_ms(u_kt);
        let v = thermo::kts_to_ms(v_kt);

        // Cross product: SRH += (u_i - u_{i-1})(v_i - v_storm) - (v_i - v_{i-1})(u_i - u_storm)
        total_srh += (u - prev_u) * (v - v_storm) - (v - prev_v) * (u - u_storm);

        prev_u = u;
        prev_v = v;
        h += step;
    }

    total_srh
}

// ── Composite parameters ────────────────────────────────────────────

/// Significant Tornado Parameter — fixed layer version (Thompson 2003).
///
/// STP = (CAPE/1500) * ((2000-LCL)/1000) * (SRH_01/150) * (EBWD/20) * ((200+CIN)/150)
fn stp_fixed(
    _profile: &SoundingProfile,
    sb: &ParcelResult,
    shear_06: f64,
    srh_01: f64,
    lcl_agl: f64,
) -> f32 {
    if sb.cape <= 0.0 {
        return 0.0;
    }

    let cape_term = sb.cape / 1500.0;
    let lcl_term = ((2000.0 - lcl_agl) / 1000.0).clamp(0.0, 1.0);
    let srh_term = srh_01 / 150.0;
    let shear_term = (shear_06 / 20.0).min(1.5); // cap at 1.5
    let cin_term = ((200.0 + sb.cin) / 150.0).clamp(0.0, 1.0);

    let stp = cape_term * lcl_term * srh_term * shear_term * cin_term;
    stp.max(0.0) as f32
}

/// Significant Tornado Parameter — effective layer version (Thompson 2007).
fn stp_eff(
    _profile: &SoundingProfile,
    ml: &ParcelResult,
    eff_shear: f64,
    eff_srh: f64,
    lcl_agl: f64,
) -> f32 {
    if ml.cape <= 0.0 {
        return 0.0;
    }

    let cape_term = ml.cape / 1500.0;
    let lcl_term = ((2000.0 - lcl_agl) / 1000.0).clamp(0.0, 1.0);
    let srh_term = eff_srh / 150.0;
    let shear_term = (eff_shear / 20.0).min(1.5);
    let cin_term = ((200.0 + ml.cin) / 150.0).clamp(0.0, 1.0);

    let stp = cape_term * lcl_term * srh_term * shear_term * cin_term;
    stp.max(0.0) as f32
}

/// Supercell Composite Parameter (Thompson et al. 2003).
///
/// SCP = (muCAPE/1000) * (effSRH/50) * (effShear/20)
pub fn scp(
    _profile: &SoundingProfile,
    mu: &ParcelResult,
    eff_shear: f64,
    eff_srh: f64,
) -> f32 {
    if mu.cape <= 0.0 {
        return 0.0;
    }

    let cape_term = mu.cape / 1000.0;
    let srh_term = eff_srh / 50.0;
    let shear_term = (eff_shear / 20.0).min(1.5);

    let val = cape_term * srh_term * shear_term;
    val.max(0.0) as f32
}

/// Significant Hail Parameter (SHIP).
///
/// SHIP = (muCAPE * w_mu * LR_700_500 * -500T * SHEAR_06) / 42_000_000
/// Modified from SPC formulation.
pub fn ship(
    profile: &SoundingProfile,
    mu: &ParcelResult,
    lr_700_500: f64,
    shear_06: f64,
) -> f32 {
    if mu.cape <= 0.0 {
        return 0.0;
    }

    let w_mu = thermo::mixing_ratio(mu.parcel_td, mu.parcel_p) * 1000.0; // g/kg
    let t500 = interp::interp_temp(profile, 500.0);

    // SHIP formula
    let numer = mu.cape * w_mu * lr_700_500 * (-t500).max(0.0) * shear_06;
    let val = numer / 42_000_000.0;
    val.max(0.0).min(5.0) as f32
}

// ── Critical angle ──────────────────────────────────────────────────

/// Critical angle (Esterheld & Giuliano 2008).
///
/// Angle between the low-level wind shear vector (0-500m)
/// and the storm-relative inflow vector.
pub fn critical_angle(profile: &SoundingProfile, storm_motion: (f64, f64)) -> f64 {
    let (u_sfc, v_sfc) = interp::interp_wind_at_height_agl(profile, 0.0);
    let (u_500, v_500) = interp::interp_wind_at_height_agl(profile, 500.0);

    // Shear vector (0-500m)
    let shr_u = u_500 - u_sfc;
    let shr_v = v_500 - v_sfc;

    // Storm-relative inflow: surface wind minus storm motion
    let sri_u = u_sfc - storm_motion.0;
    let sri_v = v_sfc - storm_motion.1;

    let shr_mag = (shr_u * shr_u + shr_v * shr_v).sqrt();
    let sri_mag = (sri_u * sri_u + sri_v * sri_v).sqrt();

    if shr_mag < 0.01 || sri_mag < 0.01 {
        return 0.0;
    }

    let dot = shr_u * sri_u + shr_v * sri_v;
    let cos_angle = (dot / (shr_mag * sri_mag)).clamp(-1.0, 1.0);
    cos_angle.acos().to_degrees()
}

// ── Lapse rates ─────────────────────────────────────────────────────

/// Compute lapse rates (C/km) for 0-3km, 3-6km, and 700-500mb layers.
fn lapse_rates_all(profile: &SoundingProfile) -> (f64, f64, f64) {
    let lr_03 = lapse_rate_height(profile, 0.0, 3000.0);
    let lr_36 = lapse_rate_height(profile, 3000.0, 6000.0);
    let lr_700_500 = lapse_rate_pressure(profile, 700.0, 500.0);
    (lr_03, lr_36, lr_700_500)
}

/// Lapse rate (C/km, positive = decreasing with height) for a height-based layer AGL.
pub fn lapse_rate_height(profile: &SoundingProfile, h_bot_agl: f64, h_top_agl: f64) -> f64 {
    let t_bot = interp::interp_temp_at_height_agl(profile, h_bot_agl);
    let t_top = interp::interp_temp_at_height_agl(profile, h_top_agl);
    let depth_km = (h_top_agl - h_bot_agl) / 1000.0;
    if depth_km.abs() < 0.01 {
        return 0.0;
    }
    (t_bot - t_top) / depth_km
}

/// Lapse rate (C/km, positive = decreasing with height) for a pressure-based layer.
pub fn lapse_rate_pressure(profile: &SoundingProfile, p_bot: f64, p_top: f64) -> f64 {
    let t_bot = interp::interp_temp(profile, p_bot);
    let t_top = interp::interp_temp(profile, p_top);
    let h_bot = interp::interp_height(profile, p_bot);
    let h_top = interp::interp_height(profile, p_top);
    let depth_km = (h_top - h_bot) / 1000.0;
    if depth_km.abs() < 0.01 {
        return 0.0;
    }
    (t_bot - t_top) / depth_km
}

// ── Moisture ────────────────────────────────────────────────────────

/// Precipitable water (mm) — integrated column moisture.
///
/// PW = (1/g) * integral(q dp) from surface to top
pub fn precipitable_water(profile: &SoundingProfile) -> f64 {
    if profile.levels.len() < 2 {
        return 0.0;
    }

    let mut pw = 0.0;

    for i in 0..profile.levels.len() - 1 {
        let p0 = profile.levels[i].pressure_mb as f64;
        let p1 = profile.levels[i + 1].pressure_mb as f64;
        let t0 = profile.levels[i].temp_c as f64;
        let td0 = profile.levels[i].dewpoint_c as f64;
        let t1 = profile.levels[i + 1].temp_c as f64;
        let td1 = profile.levels[i + 1].dewpoint_c as f64;

        let w0 = thermo::mixing_ratio(td0.min(t0), p0);
        let w1 = thermo::mixing_ratio(td1.min(t1), p1);
        let w_avg = (w0 + w1) / 2.0;

        let dp = (p0 - p1).abs() * 100.0; // convert hPa to Pa
        pw += w_avg * dp / thermo::G;
    }

    pw // kg/m2 = mm
}

// ── Stability indices ───────────────────────────────────────────────

/// K-Index = (T850 - T500) + Td850 - (T700 - Td700)
pub fn k_index(profile: &SoundingProfile) -> f64 {
    let t850 = interp::interp_temp(profile, 850.0);
    let t700 = interp::interp_temp(profile, 700.0);
    let t500 = interp::interp_temp(profile, 500.0);
    let td850 = interp::interp_dwpt(profile, 850.0);
    let td700 = interp::interp_dwpt(profile, 700.0);

    (t850 - t500) + td850 - (t700 - td700)
}

/// Total Totals = Vertical Totals + Cross Totals
///   VT = T850 - T500
///   CT = Td850 - T500
///   TT = VT + CT = T850 + Td850 - 2*T500
pub fn totals_totals(profile: &SoundingProfile) -> f64 {
    let t850 = interp::interp_temp(profile, 850.0);
    let t500 = interp::interp_temp(profile, 500.0);
    let td850 = interp::interp_dwpt(profile, 850.0);

    t850 + td850 - 2.0 * t500
}

/// SWEAT Index (Severe Weather Threat Index).
///
/// SWEAT = 12*Td850 + 20*(TT-49) + 2*f850 + f500 + 125*(sin(d500-d850) + 0.2)
pub fn sweat_index(profile: &SoundingProfile) -> f64 {
    let td850 = interp::interp_dwpt(profile, 850.0);
    let tt = totals_totals(profile);

    let (d850, s850) = interp::interp_wind(profile, 850.0);
    let (d500, s500) = interp::interp_wind(profile, 500.0);

    let mut sweat = 0.0;

    // Term 1: 12 * Td850 (only if >= 0)
    sweat += 12.0 * td850.max(0.0);

    // Term 2: 20 * (TT - 49) (only if TT > 49)
    if tt > 49.0 {
        sweat += 20.0 * (tt - 49.0);
    }

    // Term 3: 2 * f850
    sweat += 2.0 * s850;

    // Term 4: f500
    sweat += s500;

    // Term 5: shear term (only if specific wind direction criteria met)
    if d850 >= 130.0 && d850 <= 250.0 && d500 >= 210.0 && d500 <= 310.0
        && d500 > d850 && s850 >= 15.0 && s500 >= 15.0
    {
        let diff_rad = (d500 - d850).to_radians();
        sweat += 125.0 * (diff_rad.sin() + 0.2);
    }

    sweat.max(0.0)
}

// ── Downdraft CAPE ──────────────────────────────────────────────────

/// Downdraft CAPE (DCAPE).
///
/// Finds the minimum theta-e in the 400-700 mb layer, then descends
/// a saturated parcel from that level to the surface.
pub fn dcape(profile: &SoundingProfile) -> f64 {
    if profile.levels.len() < 3 {
        return 0.0;
    }

    // Find minimum theta-e in 700-400 mb
    let mut min_te = f64::INFINITY;
    let mut min_p = 600.0;
    let mut min_t = 0.0;

    let mut p = 700.0;
    while p >= 400.0 {
        let t = interp::interp_temp(profile, p);
        let td = interp::interp_dwpt(profile, p);
        let te = thermo::theta_e(t, td, p);
        if te < min_te {
            min_te = te;
            min_p = p;
            min_t = td; // Use dewpoint (saturated descent)
        }
        p -= 10.0;
    }

    // Descend the parcel moist-adiabatically to the surface
    let sfc_h = profile.sfc_height() as f64;
    let start_h = interp::interp_height(profile, min_p);

    let dz = 50.0;
    let mut h = start_h;
    let mut p_t = min_t; // start at dewpoint (saturated)
    let mut dcape_val = 0.0_f64;

    while h > sfc_h {
        let pres = interp::interp_pressure(profile, h);
        let env_t = interp::interp_temp(profile, pres);

        if p_t < env_t {
            let env_tv = thermo::virtual_temp(env_t, thermo::mixing_ratio(interp::interp_dwpt(profile, pres).min(env_t), pres));
            let p_tv = thermo::virtual_temp(p_t, thermo::mixing_ratio(p_t, pres));
            let buoy = thermo::G * (p_tv - env_tv) / env_tv * dz;
            dcape_val += buoy; // negative buoyancy
        }

        // Moist descent (warming)
        let gamma_m = thermo::moist_lapse_rate(p_t, pres);
        p_t += gamma_m * dz;

        h -= dz;
    }

    (-dcape_val).max(0.0)
}

// ── Microburst composite ────────────────────────────────────────────

/// Microburst composite index.
///
/// Combination of DCAPE, lapse rates, and low-level moisture deficit.
pub fn mburst(profile: &SoundingProfile) -> f64 {
    let dc = dcape(profile);
    let lr_36 = lapse_rate_height(profile, 3000.0, 6000.0);
    let t_sfc = interp::interp_temp(profile, profile.sfc_pressure() as f64);
    let td_sfc = interp::interp_dwpt(profile, profile.sfc_pressure() as f64);
    let dd = t_sfc - td_sfc; // dewpoint depression

    // Simple composite: high DCAPE + steep mid-level lapse rates + large sfc DD
    let score = (dc / 1000.0) * (lr_36 / 7.0) * (dd / 15.0).min(2.0);
    score.max(0.0)
}

// ── Comfort indices ─────────────────────────────────────────────────

/// Wind chill temperature (C) given temperature (C) and wind speed (kts).
///
/// Uses the NWS wind chill formula (2001 revision).
pub fn wind_chill(temp_c: f64, wind_kts: f64) -> f64 {
    let wind_kmh = wind_kts * 1.852;
    if temp_c > 10.0 || wind_kmh <= 4.8 {
        return temp_c;
    }
    13.12 + 0.6215 * temp_c - 11.37 * wind_kmh.powf(0.16) + 0.3965 * temp_c * wind_kmh.powf(0.16)
}

/// Heat index (C) given temperature (C) and relative humidity (%).
///
/// Uses the Rothfusz regression equation (NWS formulation).
pub fn heat_index(temp_c: f64, rh_pct: f64) -> f64 {
    let t_f = temp_c * 9.0 / 5.0 + 32.0;
    if t_f < 80.0 {
        // Simple formula for lower temperatures
        let hi_f = 0.5 * (t_f + 61.0 + (t_f - 68.0) * 1.2 + rh_pct * 0.094);
        return (hi_f - 32.0) * 5.0 / 9.0;
    }

    // Rothfusz regression
    let mut hi = -42.379
        + 2.04901523 * t_f
        + 10.14333127 * rh_pct
        - 0.22475541 * t_f * rh_pct
        - 0.00683783 * t_f * t_f
        - 0.05481717 * rh_pct * rh_pct
        + 0.00122874 * t_f * t_f * rh_pct
        + 0.00085282 * t_f * rh_pct * rh_pct
        - 0.00000199 * t_f * t_f * rh_pct * rh_pct;

    // Adjustments
    if rh_pct < 13.0 && t_f >= 80.0 && t_f <= 112.0 {
        hi -= ((13.0 - rh_pct) / 4.0) * ((17.0 - (t_f - 95.0).abs()) / 17.0).sqrt();
    } else if rh_pct > 85.0 && t_f >= 80.0 && t_f <= 87.0 {
        hi += ((rh_pct - 85.0) / 10.0) * ((87.0 - t_f) / 5.0);
    }

    (hi - 32.0) * 5.0 / 9.0
}
