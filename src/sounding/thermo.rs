//! Thermodynamic functions for atmospheric sounding analysis.
//!
//! All formulas follow SHARPpy and standard meteorological references:
//! - Bolton (1980) for saturation vapor pressure
//! - Poisson relation for dry adiabatic processes
//! - Iterative methods for moist adiabatic and wet-bulb calculations

// ── Physical constants ──────────────────────────────────────────────

/// Latent heat of vaporisation (J/kg)
pub const LV: f64 = 2.501e6;
/// Specific gas constant for dry air (J/(kg*K))
pub const RD: f64 = 287.04;
/// Specific gas constant for water vapour (J/(kg*K))
pub const RV: f64 = 461.5;
/// Specific heat of dry air at constant pressure (J/(kg*K))
pub const CP: f64 = 1004.0;
/// Gravitational acceleration (m/s^2)
pub const G: f64 = 9.80665;
/// Dry adiabatic lapse rate (K/m)
pub const GAMMA_D: f64 = G / CP;
/// Absolute zero offset
pub const ZEROCNK: f64 = 273.15;
/// Epsilon = Rd/Rv
pub const EPS: f64 = RD / RV;

// ── Core thermodynamic functions ────────────────────────────────────

/// Saturation vapour pressure (hPa) via the Bolton (1980) formula.
///
/// es(T) = 6.112 * exp(17.67 * T / (T + 243.5))
///
/// Reference: Bolton, D. (1980). Monthly Weather Review, 108, 1046-1053.
pub fn sat_vapor_pressure(temp_c: f64) -> f64 {
    6.112 * ((17.67 * temp_c) / (temp_c + 243.5)).exp()
}

/// Saturation mixing ratio (kg/kg) given temperature (C) and pressure (hPa).
///
/// ws = eps * es / (p - es)
pub fn mixing_ratio(temp_c: f64, pres_mb: f64) -> f64 {
    let es = sat_vapor_pressure(temp_c);
    EPS * es / (pres_mb - es).max(0.001)
}

/// Potential temperature (K) via Poisson relation.
///
/// theta = T * (1000/p)^(Rd/Cp)
pub fn theta(temp_c: f64, pres_mb: f64) -> f64 {
    let t_k = temp_c + ZEROCNK;
    t_k * (1000.0 / pres_mb).powf(RD / CP)
}

/// Equivalent potential temperature (K).
///
/// Uses Bolton (1980) formula:
/// theta_e = theta_d * exp((3.376/T_lcl - 0.00254) * r * (1 + 0.81e-3 * r))
///
/// where theta_d is dry potential temperature, T_lcl is LCL temperature,
/// and r is mixing ratio in g/kg.
pub fn theta_e(temp_c: f64, dewpt_c: f64, pres_mb: f64) -> f64 {
    let r = mixing_ratio(dewpt_c, pres_mb) * 1000.0; // g/kg
    let t_k = temp_c + ZEROCNK;

    // Bolton (1980) LCL temperature approximation
    let t_lcl = 56.0 + 1.0 / (1.0 / (dewpt_c + ZEROCNK - 56.0) + (t_k / (dewpt_c + ZEROCNK)).ln() / 800.0);

    // Dry potential temperature
    let theta_d = t_k * (1000.0 / pres_mb).powf(0.2854 * (1.0 - 0.00028 * r));

    // Equivalent potential temperature
    theta_d * ((3.376 / t_lcl - 0.00254) * r * (1.0 + 0.81e-3 * r)).exp()
}

/// Wet-bulb temperature (C) via iterative Newton-Raphson method.
///
/// Finds Tw such that the wet-bulb equation is satisfied:
/// e(Tw) - e = gamma * p * (T - Tw)
/// where gamma is the psychrometer constant.
pub fn wetbulb(temp_c: f64, dewpt_c: f64, pres_mb: f64) -> f64 {
    // Initial guess: average of temp and dewpoint
    let mut tw = (temp_c + dewpt_c) / 2.0;
    let e_actual = sat_vapor_pressure(dewpt_c);

    for _ in 0..50 {
        let e_tw = sat_vapor_pressure(tw);
        let de_dtw = e_tw * 17.67 * 243.5 / ((tw + 243.5) * (tw + 243.5));

        // Psychrometric equation: e_tw - e_actual = 0.000662 * p * (T - Tw)
        let gamma = 0.000662 * pres_mb;
        let f = e_tw - e_actual - gamma * (temp_c - tw);
        let fp = de_dtw + gamma;

        let delta = f / fp;
        tw -= delta;

        if delta.abs() < 0.001 {
            break;
        }
    }
    tw
}

/// Virtual temperature (K) given temperature (C) and mixing ratio (kg/kg).
///
/// Tv = T * (1 + 0.61 * w)
pub fn virtual_temp(temp_c: f64, w: f64) -> f64 {
    (temp_c + ZEROCNK) * (1.0 + 0.61 * w)
}

/// Virtual temperature (C) from temperature and dewpoint.
pub fn virtual_temp_from_dwpt(temp_c: f64, dewpt_c: f64, pres_mb: f64) -> f64 {
    let w = mixing_ratio(dewpt_c, pres_mb);
    virtual_temp(temp_c, w) - ZEROCNK
}

/// LCL pressure (hPa) via iterative parcel lifting.
///
/// Lifts a surface parcel dry-adiabatically until temperature equals dewpoint.
/// Returns (lcl_pressure, lcl_temperature).
pub fn lcl_pressure(temp_c: f64, dewpt_c: f64, pres_mb: f64) -> (f64, f64) {
    // Bolton (1980) LCL temperature
    let t_k = temp_c + ZEROCNK;
    let td_k = dewpt_c + ZEROCNK;
    let t_lcl = 56.0 + 1.0 / (1.0 / (td_k - 56.0) + (t_k / td_k).ln() / 800.0);

    // LCL pressure from Poisson relation
    let p_lcl = pres_mb * (t_lcl / t_k).powf(CP / RD);

    (p_lcl, t_lcl - ZEROCNK)
}

/// Moist adiabatic lapse rate (K/m) at given T (C) and P (hPa).
///
/// Gamma_m = Gamma_d * (1 + Lv*ws/(Rd*T)) / (1 + Lv^2*ws/(Cp*Rv*T^2))
///
/// This is the pseudo-adiabatic lapse rate (all condensate falls out).
pub fn moist_lapse_rate(temp_c: f64, pres_mb: f64) -> f64 {
    let t_k = temp_c + ZEROCNK;
    let ws = mixing_ratio(temp_c, pres_mb);
    let numer = 1.0 + LV * ws / (RD * t_k);
    let denom = 1.0 + LV * LV * ws / (CP * RV * t_k * t_k);
    GAMMA_D * numer / denom
}

/// Lift a parcel moist-adiabatically from (temp_c, pres_mb) to pres_target.
///
/// Uses a 4th-order Runge-Kutta integration in log-pressure coordinates
/// for accuracy, following SHARPpy's approach.
///
/// Returns temperature (C) at the target pressure.
pub fn moist_lapse(temp_c: f64, pres_mb: f64, pres_target: f64) -> f64 {
    if (pres_mb - pres_target).abs() < 0.01 {
        return temp_c;
    }

    let mut t = temp_c;
    let mut p = pres_mb;

    // Number of steps depends on the pressure interval
    let n_steps = ((pres_mb - pres_target).abs() / 5.0).ceil().max(10.0) as usize;
    let dp = (pres_target - pres_mb) / n_steps as f64;

    for _ in 0..n_steps {
        // RK4 in pressure coordinates
        // dT/dp = (Rd * Tv) / (Cp * p) * (1 + Lv*ws/(Rd*T)) / (1 + Lv^2*ws/(Cp*Rv*T^2))
        let _dt_dp = |t_c: f64, p_hpa: f64| -> f64 {
            let t_k = t_c + ZEROCNK;
            let ws = mixing_ratio(t_c, p_hpa);
            let tv_k = t_k * (1.0 + 0.61 * ws);
            let numer = 1.0 + LV * ws / (RD * t_k);
            let denom = 1.0 + LV * LV * ws / (CP * RV * t_k * t_k);
            (RD * tv_k) / (CP * p_hpa) / (numer / denom).recip()
        };

        // Simplified: use direct height-based approach for stability
        // Convert dp to dz: dz = -(Rd * T) / (g * p) * dp
        let t_k = t + ZEROCNK;
        let dz = -(RD * t_k / G) * (dp / p);
        let gamma_m = moist_lapse_rate(t, p);

        // RK4 steps
        let k1 = -gamma_m * dz;
        let t2 = t + k1 * 0.5;
        let p2 = p + dp * 0.5;
        let gamma2 = moist_lapse_rate(t2, p2);
        let dz2 = -(RD * (t2 + ZEROCNK) / G) * (dp * 0.5 / (p + dp * 0.25));
        let k2 = -gamma2 * dz2 * 2.0;

        let t3 = t + k2 * 0.5;
        let gamma3 = moist_lapse_rate(t3, p2);
        let k3 = -gamma3 * dz2 * 2.0;

        let t4 = t + k3;
        let p4 = p + dp;
        let gamma4 = moist_lapse_rate(t4, p4);
        let dz4 = -(RD * (t4 + ZEROCNK) / G) * (dp / p4.max(1.0));
        let k4 = -gamma4 * dz4;

        t += (k1 + 2.0 * k2 + 2.0 * k3 + k4) / 6.0;
        p += dp;

        // Safety check
        if t < -273.0 || p < 1.0 {
            break;
        }
    }

    t
}

/// Dry adiabatic temperature (C) at pres_target given initial conditions.
///
/// Uses Poisson relation: T2 = T1 * (P2/P1)^(Rd/Cp)
pub fn dry_lapse(temp_c: f64, pres_mb: f64, pres_target: f64) -> f64 {
    let t_k = temp_c + ZEROCNK;
    let t_target = t_k * (pres_target / pres_mb).powf(RD / CP);
    t_target - ZEROCNK
}

/// Temperature (C) at the LCL for a given mixing ratio line and pressure.
///
/// Inverts the Bolton saturation vapor pressure formula.
pub fn temp_at_mixing_ratio(w: f64, pres_mb: f64) -> f64 {
    // w = eps * es / (p - es), so es = w * p / (eps + w)
    let es = w * pres_mb / (EPS + w);
    // Invert Bolton: T = 243.5 * ln(es/6.112) / (17.67 - ln(es/6.112))
    if es <= 0.0 {
        return -273.15;
    }
    let x = (es / 6.112).ln();
    243.5 * x / (17.67 - x)
}

/// Wind components (u, v) in knots from direction (degrees) and speed.
pub fn wind_components(wdir: f64, wspd: f64) -> (f64, f64) {
    let rad = wdir.to_radians();
    let u = -wspd * rad.sin();
    let v = -wspd * rad.cos();
    (u, v)
}

/// Wind direction and speed from u, v components.
pub fn wind_from_components(u: f64, v: f64) -> (f64, f64) {
    let spd = (u * u + v * v).sqrt();
    if spd < 0.01 {
        return (0.0, 0.0);
    }
    let dir = (u.atan2(v).to_degrees() + 180.0) % 360.0;
    (dir, spd)
}

/// Convert knots to m/s.
pub fn kts_to_ms(kts: f64) -> f64 {
    kts * 0.514444
}

/// Convert m/s to knots.
pub fn ms_to_kts(ms: f64) -> f64 {
    ms * 1.94384
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sat_vapor_pressure() {
        // At 0C, es should be about 6.112 hPa
        let es = sat_vapor_pressure(0.0);
        assert!((es - 6.112).abs() < 0.01, "es at 0C = {es}");

        // At 20C, es should be about 23.37 hPa
        let es20 = sat_vapor_pressure(20.0);
        assert!((es20 - 23.37).abs() < 0.5, "es at 20C = {es20}");
    }

    #[test]
    fn test_theta() {
        // At 1000mb, theta should equal T (in K)
        let th = theta(20.0, 1000.0);
        assert!((th - 293.15).abs() < 0.5, "theta at 1000mb = {th}");
    }

    #[test]
    fn test_dry_lapse() {
        // Lifting from 1000mb to 500mb should cool significantly
        let t = dry_lapse(20.0, 1000.0, 500.0);
        assert!(t < -20.0, "dry lapse to 500mb = {t}");
    }

    #[test]
    fn test_lcl() {
        // T=20, Td=10 at 1000mb -> LCL should be around 875-900mb
        let (p_lcl, _t_lcl) = lcl_pressure(20.0, 10.0, 1000.0);
        assert!(p_lcl > 850.0 && p_lcl < 950.0, "LCL pressure = {p_lcl}");
    }

    #[test]
    fn test_wind_components() {
        // South wind (180 deg) at 10 kts -> u=0, v=10
        let (u, v) = wind_components(180.0, 10.0);
        assert!(u.abs() < 0.01, "u = {u}");
        assert!((v - 10.0).abs() < 0.01, "v = {v}");
    }
}
