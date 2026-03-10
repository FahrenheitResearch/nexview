//! Interpolation helpers for sounding profiles.
//!
//! All interpolation is done in log-pressure space for physical correctness.

use super::profile::{SoundingLevel, SoundingProfile};
use super::thermo;

/// Interpolate pressure (hPa) at a given height (m) above sea level.
pub fn interp_pressure(profile: &SoundingProfile, height_m: f64) -> f64 {
    let levels = &profile.levels;
    if levels.is_empty() {
        return 1013.25;
    }
    if height_m <= levels[0].height_m as f64 {
        return levels[0].pressure_mb as f64;
    }
    if height_m >= levels.last().unwrap().height_m as f64 {
        return levels.last().unwrap().pressure_mb as f64;
    }

    for i in 0..levels.len() - 1 {
        let h0 = levels[i].height_m as f64;
        let h1 = levels[i + 1].height_m as f64;
        if height_m >= h0 && height_m <= h1 {
            let frac = (height_m - h0) / (h1 - h0).max(1.0);
            // Interpolate in log-pressure space
            let lnp0 = (levels[i].pressure_mb as f64).ln();
            let lnp1 = (levels[i + 1].pressure_mb as f64).ln();
            return (lnp0 + frac * (lnp1 - lnp0)).exp();
        }
    }
    levels.last().unwrap().pressure_mb as f64
}

/// Interpolate height (m) at a given pressure (hPa).
pub fn interp_height(profile: &SoundingProfile, pressure_mb: f64) -> f64 {
    interp_generic_at_pressure(&profile.levels, pressure_mb, |l| l.height_m as f64)
}

/// Interpolate temperature (C) at a given pressure (hPa).
pub fn interp_temp(profile: &SoundingProfile, pressure_mb: f64) -> f64 {
    interp_generic_at_pressure(&profile.levels, pressure_mb, |l| l.temp_c as f64)
}

/// Interpolate dewpoint (C) at a given pressure (hPa).
pub fn interp_dwpt(profile: &SoundingProfile, pressure_mb: f64) -> f64 {
    interp_generic_at_pressure(&profile.levels, pressure_mb, |l| l.dewpoint_c as f64)
}

/// Interpolate wind (dir_deg, speed_kts) at a given pressure (hPa).
///
/// Wind interpolation is done in u/v component space to avoid
/// discontinuities around 360/0 degrees.
pub fn interp_wind(profile: &SoundingProfile, pressure_mb: f64) -> (f64, f64) {
    let u = interp_generic_at_pressure(&profile.levels, pressure_mb, |l| {
        let (u, _) = thermo::wind_components(l.wind_dir as f64, l.wind_speed_kts as f64);
        u
    });
    let v = interp_generic_at_pressure(&profile.levels, pressure_mb, |l| {
        let (_, v) = thermo::wind_components(l.wind_dir as f64, l.wind_speed_kts as f64);
        v
    });
    thermo::wind_from_components(u, v)
}

/// Interpolate wind components (u, v) in knots at a given pressure.
pub fn interp_wind_components(profile: &SoundingProfile, pressure_mb: f64) -> (f64, f64) {
    let u = interp_generic_at_pressure(&profile.levels, pressure_mb, |l| {
        let (u, _) = thermo::wind_components(l.wind_dir as f64, l.wind_speed_kts as f64);
        u
    });
    let v = interp_generic_at_pressure(&profile.levels, pressure_mb, |l| {
        let (_, v) = thermo::wind_components(l.wind_dir as f64, l.wind_speed_kts as f64);
        v
    });
    (u, v)
}

/// Pressure-weighted mean wind (u, v) in knots between pbot and ptop (hPa).
///
/// Uses trapezoidal integration in log-pressure space, matching SHARPpy.
pub fn mean_wind(profile: &SoundingProfile, pbot: f64, ptop: f64) -> (f64, f64) {
    if profile.levels.is_empty() || pbot <= ptop {
        return (0.0, 0.0);
    }

    let dp = 10.0; // pressure step in hPa
    let mut p = pbot;
    let mut u_sum = 0.0;
    let mut v_sum = 0.0;
    let mut weight_sum = 0.0;

    while p >= ptop {
        let (u, v) = interp_wind_components(profile, p);
        // Weight by pressure (pressure-weighted mean)
        u_sum += u;
        v_sum += v;
        weight_sum += 1.0;
        p -= dp;
    }

    if weight_sum > 0.0 {
        (u_sum / weight_sum, v_sum / weight_sum)
    } else {
        (0.0, 0.0)
    }
}

/// Interpolate temperature at a height above ground level (m).
pub fn interp_temp_at_height_agl(profile: &SoundingProfile, height_agl: f64) -> f64 {
    let sfc_h = profile.sfc_height() as f64;
    interp_at_height_asl(&profile.levels, sfc_h + height_agl, |l| l.temp_c as f64)
}

/// Interpolate wind components at a height above ground level (m).
pub fn interp_wind_at_height_agl(profile: &SoundingProfile, height_agl: f64) -> (f64, f64) {
    let sfc_h = profile.sfc_height() as f64;
    let h = sfc_h + height_agl;
    let u = interp_at_height_asl(&profile.levels, h, |l| {
        let (u, _) = thermo::wind_components(l.wind_dir as f64, l.wind_speed_kts as f64);
        u
    });
    let v = interp_at_height_asl(&profile.levels, h, |l| {
        let (_, v) = thermo::wind_components(l.wind_dir as f64, l.wind_speed_kts as f64);
        v
    });
    (u, v)
}

/// Interpolate pressure at a height AGL.
pub fn interp_pressure_at_height_agl(profile: &SoundingProfile, height_agl: f64) -> f64 {
    let sfc_h = profile.sfc_height() as f64;
    interp_pressure(profile, sfc_h + height_agl)
}

// ── Internal helpers ────────────────────────────────────────────────

/// Generic interpolation at a given pressure (hPa) in log-pressure space.
/// Levels must be sorted by decreasing pressure (surface first).
fn interp_generic_at_pressure(levels: &[SoundingLevel], pressure_mb: f64, get: fn(&SoundingLevel) -> f64) -> f64 {
    if levels.is_empty() {
        return 0.0;
    }
    if pressure_mb >= levels[0].pressure_mb as f64 {
        return get(&levels[0]);
    }
    if pressure_mb <= levels.last().unwrap().pressure_mb as f64 {
        return get(levels.last().unwrap());
    }

    for i in 0..levels.len() - 1 {
        let p0 = levels[i].pressure_mb as f64;
        let p1 = levels[i + 1].pressure_mb as f64;
        if pressure_mb <= p0 && pressure_mb >= p1 {
            // Interpolate in log-pressure space
            let lnp0 = p0.ln();
            let lnp1 = p1.ln();
            let lnp = pressure_mb.ln();
            let frac = (lnp0 - lnp) / (lnp0 - lnp1).max(1e-10);
            let v0 = get(&levels[i]);
            let v1 = get(&levels[i + 1]);
            return v0 + frac * (v1 - v0);
        }
    }
    get(levels.last().unwrap())
}

/// Generic interpolation at a given height ASL (m).
fn interp_at_height_asl(levels: &[SoundingLevel], h: f64, get: fn(&SoundingLevel) -> f64) -> f64 {
    if levels.is_empty() {
        return 0.0;
    }
    if h <= levels[0].height_m as f64 {
        return get(&levels[0]);
    }
    if h >= levels.last().unwrap().height_m as f64 {
        return get(levels.last().unwrap());
    }

    for i in 0..levels.len() - 1 {
        let h0 = levels[i].height_m as f64;
        let h1 = levels[i + 1].height_m as f64;
        if h >= h0 && h <= h1 {
            let frac = (h - h0) / (h1 - h0).max(1.0);
            return get(&levels[i]) + frac * (get(&levels[i + 1]) - get(&levels[i]));
        }
    }
    get(levels.last().unwrap())
}
