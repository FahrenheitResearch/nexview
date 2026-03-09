use crate::data::sounding::{SoundingLevel, SoundingProfile};

/// Professional-quality Skew-T/Log-P renderer inspired by SHARPpy.
/// Outputs an RGBA pixel buffer for display as an egui texture.
pub struct SkewTRenderer;

// ── Layout constants ────────────────────────────────────────────────

const SKEWT_FRAC: f64 = 0.70; // Main diagram = 70% of width

// Skew-T margins (within the left 70%)
const MARGIN_LEFT: f64 = 52.0;
const MARGIN_RIGHT: f64 = 50.0; // room for wind barbs
const MARGIN_TOP: f64 = 24.0;
const MARGIN_BOT: f64 = 28.0;

// Pressure range
const P_TOP: f64 = 100.0;
const P_BOT: f64 = 1050.0;

// Temperature range at the bottom of the diagram
const T_MIN: f64 = -40.0;
const T_MAX: f64 = 50.0;
const SKEW: f64 = 1.0; // ~45 degree skew factor

// Physical constants
const LV: f64 = 2.501e6;
const RD: f64 = 287.04;
const RV: f64 = 461.5;
const CP: f64 = 1004.0;
const G: f64 = 9.80665;
const GAMMA_D: f64 = G / CP;
const KELVIN: f64 = 273.15;

// Standard pressure levels for isobars
const STD_PRESSURES: &[f64] = &[
    1000.0, 925.0, 850.0, 700.0, 500.0, 400.0, 300.0, 250.0, 200.0, 150.0, 100.0,
];

// ── Colour palette (SHARPpy-inspired dark background) ───────────────

const COL_BG: [u8; 4] = [10, 10, 22, 255];
const COL_GRID: [u8; 4] = [45, 45, 55, 255];
const COL_GRID_ZERO: [u8; 4] = [70, 100, 180, 200];
const COL_ISOBAR: [u8; 4] = [50, 50, 60, 255];
const COL_DRY_AD: [u8; 4] = [120, 75, 40, 130];
const COL_MOIST_AD: [u8; 4] = [40, 120, 60, 130];
const COL_MIX_RATIO: [u8; 4] = [110, 60, 140, 100];
const COL_TEMP: [u8; 4] = [255, 40, 40, 255];
const COL_DEWP: [u8; 4] = [40, 220, 40, 255];
const COL_PARCEL: [u8; 4] = [80, 160, 255, 255];
const COL_CAPE_FILL: [u8; 4] = [255, 60, 60, 55];
const COL_CIN_FILL: [u8; 4] = [60, 60, 255, 40];
const COL_WIND_BARB: [u8; 4] = [230, 230, 230, 255];
const COL_LABEL: [u8; 4] = [180, 180, 190, 255];
const COL_TEXT: [u8; 4] = [230, 230, 230, 255];
const COL_TEXT_DIM: [u8; 4] = [140, 140, 150, 255];
const COL_TEXT_HEADER: [u8; 4] = [100, 180, 255, 255];
const COL_PANEL_BG: [u8; 4] = [18, 18, 32, 255];
const COL_PANEL_BORDER: [u8; 4] = [50, 50, 70, 255];

// Hodograph height-band colours
const COL_HODO_0_3: [u8; 4] = [255, 60, 60, 255];
const COL_HODO_3_6: [u8; 4] = [60, 220, 60, 255];
const COL_HODO_6_9: [u8; 4] = [60, 120, 255, 255];
const COL_HODO_9_12: [u8; 4] = [180, 80, 220, 255];
const COL_HODO_RING: [u8; 4] = [55, 55, 65, 255];
const COL_HODO_BUNKERS: [u8; 4] = [255, 200, 60, 255];
const COL_HODO_MEAN: [u8; 4] = [200, 200, 200, 255];

// ── 7x10 bitmap font for improved readability ──────────────────────

const FONT_W: i32 = 7;
const FONT_H: i32 = 10;

/// Return a 10-row bitmap for a character (7 bits wide per row).
fn char_bitmap(ch: char) -> [u16; 10] {
    match ch {
        '0' => [
            0b0011100, 0b0100010, 0b1000001, 0b1000101, 0b1001001, 0b1010001, 0b1000001,
            0b0100010, 0b0011100, 0b0000000,
        ],
        '1' => [
            0b0001000, 0b0011000, 0b0101000, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
            0b0001000, 0b0111110, 0b0000000,
        ],
        '2' => [
            0b0111100, 0b1000010, 0b0000010, 0b0000100, 0b0001000, 0b0010000, 0b0100000,
            0b1000000, 0b1111110, 0b0000000,
        ],
        '3' => [
            0b0111100, 0b1000010, 0b0000010, 0b0011100, 0b0000010, 0b0000010, 0b0000010,
            0b1000010, 0b0111100, 0b0000000,
        ],
        '4' => [
            0b0000100, 0b0001100, 0b0010100, 0b0100100, 0b1000100, 0b1111110, 0b0000100,
            0b0000100, 0b0000100, 0b0000000,
        ],
        '5' => [
            0b1111110, 0b1000000, 0b1000000, 0b1111100, 0b0000010, 0b0000010, 0b0000010,
            0b1000010, 0b0111100, 0b0000000,
        ],
        '6' => [
            0b0011100, 0b0100000, 0b1000000, 0b1111100, 0b1000010, 0b1000010, 0b1000010,
            0b0100010, 0b0011100, 0b0000000,
        ],
        '7' => [
            0b1111110, 0b0000010, 0b0000100, 0b0001000, 0b0010000, 0b0010000, 0b0010000,
            0b0010000, 0b0010000, 0b0000000,
        ],
        '8' => [
            0b0111100, 0b1000010, 0b1000010, 0b0111100, 0b1000010, 0b1000010, 0b1000010,
            0b1000010, 0b0111100, 0b0000000,
        ],
        '9' => [
            0b0111100, 0b1000010, 0b1000010, 0b0111110, 0b0000010, 0b0000010, 0b0000100,
            0b0001000, 0b0110000, 0b0000000,
        ],
        'A' => [
            0b0011100, 0b0100010, 0b1000001, 0b1000001, 0b1111111, 0b1000001, 0b1000001,
            0b1000001, 0b1000001, 0b0000000,
        ],
        'B' => [
            0b1111100, 0b1000010, 0b1000010, 0b1111100, 0b1000010, 0b1000010, 0b1000010,
            0b1000010, 0b1111100, 0b0000000,
        ],
        'C' => [
            0b0011110, 0b0100001, 0b1000000, 0b1000000, 0b1000000, 0b1000000, 0b1000000,
            0b0100001, 0b0011110, 0b0000000,
        ],
        'D' => [
            0b1111100, 0b1000010, 0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
            0b1000010, 0b1111100, 0b0000000,
        ],
        'E' => [
            0b1111110, 0b1000000, 0b1000000, 0b1111100, 0b1000000, 0b1000000, 0b1000000,
            0b1000000, 0b1111110, 0b0000000,
        ],
        'F' => [
            0b1111110, 0b1000000, 0b1000000, 0b1111100, 0b1000000, 0b1000000, 0b1000000,
            0b1000000, 0b1000000, 0b0000000,
        ],
        'G' => [
            0b0011110, 0b0100001, 0b1000000, 0b1000000, 0b1001111, 0b1000001, 0b1000001,
            0b0100001, 0b0011110, 0b0000000,
        ],
        'H' => [
            0b1000001, 0b1000001, 0b1000001, 0b1111111, 0b1000001, 0b1000001, 0b1000001,
            0b1000001, 0b1000001, 0b0000000,
        ],
        'I' => [
            0b0111110, 0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
            0b0001000, 0b0111110, 0b0000000,
        ],
        'J' => [
            0b0001111, 0b0000010, 0b0000010, 0b0000010, 0b0000010, 0b0000010, 0b1000010,
            0b0100010, 0b0011100, 0b0000000,
        ],
        'K' => [
            0b1000010, 0b1000100, 0b1001000, 0b1010000, 0b1100000, 0b1010000, 0b1001000,
            0b1000100, 0b1000010, 0b0000000,
        ],
        'L' => [
            0b1000000, 0b1000000, 0b1000000, 0b1000000, 0b1000000, 0b1000000, 0b1000000,
            0b1000000, 0b1111110, 0b0000000,
        ],
        'M' => [
            0b1000001, 0b1100011, 0b1010101, 0b1001001, 0b1000001, 0b1000001, 0b1000001,
            0b1000001, 0b1000001, 0b0000000,
        ],
        'N' => [
            0b1000001, 0b1100001, 0b1010001, 0b1001001, 0b1000101, 0b1000011, 0b1000001,
            0b1000001, 0b1000001, 0b0000000,
        ],
        'O' => [
            0b0011100, 0b0100010, 0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
            0b0100010, 0b0011100, 0b0000000,
        ],
        'P' => [
            0b1111100, 0b1000010, 0b1000010, 0b1111100, 0b1000000, 0b1000000, 0b1000000,
            0b1000000, 0b1000000, 0b0000000,
        ],
        'Q' => [
            0b0011100, 0b0100010, 0b1000001, 0b1000001, 0b1000001, 0b1000101, 0b1000010,
            0b0100010, 0b0011101, 0b0000000,
        ],
        'R' => [
            0b1111100, 0b1000010, 0b1000010, 0b1111100, 0b1010000, 0b1001000, 0b1000100,
            0b1000010, 0b1000001, 0b0000000,
        ],
        'S' => [
            0b0111110, 0b1000001, 0b1000000, 0b0111100, 0b0000010, 0b0000001, 0b0000001,
            0b1000010, 0b0111100, 0b0000000,
        ],
        'T' => [
            0b1111111, 0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
            0b0001000, 0b0001000, 0b0000000,
        ],
        'U' => [
            0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
            0b0100010, 0b0011100, 0b0000000,
        ],
        'V' => [
            0b1000001, 0b1000001, 0b1000001, 0b0100010, 0b0100010, 0b0010100, 0b0010100,
            0b0001000, 0b0001000, 0b0000000,
        ],
        'W' => [
            0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1001001, 0b1010101, 0b1010101,
            0b0100010, 0b0100010, 0b0000000,
        ],
        'X' => [
            0b1000001, 0b0100010, 0b0010100, 0b0001000, 0b0001000, 0b0010100, 0b0100010,
            0b1000001, 0b1000001, 0b0000000,
        ],
        'Y' => [
            0b1000001, 0b0100010, 0b0010100, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
            0b0001000, 0b0001000, 0b0000000,
        ],
        'Z' => [
            0b1111111, 0b0000010, 0b0000100, 0b0001000, 0b0010000, 0b0100000, 0b1000000,
            0b1000000, 0b1111111, 0b0000000,
        ],
        ' ' => [0; 10],
        ':' => [
            0b0000000, 0b0000000, 0b0001000, 0b0001000, 0b0000000, 0b0000000, 0b0001000,
            0b0001000, 0b0000000, 0b0000000,
        ],
        '.' => [
            0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000,
            0b0001100, 0b0001100, 0b0000000,
        ],
        '-' => [
            0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0111110, 0b0000000, 0b0000000,
            0b0000000, 0b0000000, 0b0000000,
        ],
        '+' => [
            0b0000000, 0b0000000, 0b0001000, 0b0001000, 0b0111110, 0b0001000, 0b0001000,
            0b0000000, 0b0000000, 0b0000000,
        ],
        '/' => [
            0b0000001, 0b0000010, 0b0000100, 0b0001000, 0b0010000, 0b0100000, 0b1000000,
            0b0000000, 0b0000000, 0b0000000,
        ],
        ',' => [
            0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0001100,
            0b0001100, 0b0000100, 0b0001000,
        ],
        '(' => [
            0b0000100, 0b0001000, 0b0010000, 0b0010000, 0b0010000, 0b0010000, 0b0010000,
            0b0001000, 0b0000100, 0b0000000,
        ],
        ')' => [
            0b0100000, 0b0010000, 0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
            0b0010000, 0b0100000, 0b0000000,
        ],
        '%' => [
            0b1100001, 0b1100010, 0b0000100, 0b0001000, 0b0010000, 0b0100000, 0b0100110,
            0b1000110, 0b0000000, 0b0000000,
        ],
        '=' => [
            0b0000000, 0b0000000, 0b0111110, 0b0000000, 0b0000000, 0b0111110, 0b0000000,
            0b0000000, 0b0000000, 0b0000000,
        ],
        '*' => [
            0b0000000, 0b0001000, 0b0101010, 0b0011100, 0b0101010, 0b0001000, 0b0000000,
            0b0000000, 0b0000000, 0b0000000,
        ],
        '~' => [
            0b0000000, 0b0000000, 0b0110000, 0b1001001, 0b0000110, 0b0000000, 0b0000000,
            0b0000000, 0b0000000, 0b0000000,
        ],
        'a'..='z' => char_bitmap((ch as u8 - b'a' + b'A') as char),
        _ => [0; 10],
    }
}

// ── Coordinate transforms ───────────────────────────────────────────

/// Normalised Y from pressure (0 = bottom, 1 = top).
fn y_from_p(p: f64) -> f64 {
    (P_BOT.ln() - p.ln()) / (P_BOT.ln() - P_TOP.ln())
}

/// Screen coords within the Skew-T plot area.
fn tp_to_screen(t: f64, p: f64, plot_w: f64, plot_h: f64) -> (f64, f64) {
    let yn = y_from_p(p);
    let t_shifted = t + SKEW * (P_BOT.ln() - p.ln()) * 25.0;
    let xn = (t_shifted - T_MIN) / (T_MAX - T_MIN);
    let sx = MARGIN_LEFT + xn * plot_w;
    let sy = MARGIN_TOP + (1.0 - yn) * plot_h;
    (sx, sy)
}

// ── Thermodynamic helpers ───────────────────────────────────────────

fn sat_vapor_pressure(temp_c: f64) -> f64 {
    6.112 * ((17.67 * temp_c) / (temp_c + 243.5)).exp()
}

fn sat_mixing_ratio(temp_c: f64, pres_mb: f64) -> f64 {
    let es = sat_vapor_pressure(temp_c);
    0.622 * es / (pres_mb - es).max(0.1)
}

fn moist_lapse_rate(temp_c: f64, pres_mb: f64) -> f64 {
    let t_k = temp_c + KELVIN;
    let ws = sat_mixing_ratio(temp_c, pres_mb);
    let numer = 1.0 + LV * ws / (RD * t_k);
    let denom = 1.0 + LV * LV * ws / (CP * RV * t_k * t_k);
    GAMMA_D * numer / denom
}

/// Potential temperature (K) from T (C) and P (hPa).
fn theta(temp_c: f64, pres_mb: f64) -> f64 {
    (temp_c + KELVIN) * (1000.0 / pres_mb).powf(RD / CP)
}

/// Temperature from potential temperature at a given pressure.
fn temp_from_theta(theta_k: f64, pres_mb: f64) -> f64 {
    theta_k * (pres_mb / 1000.0).powf(RD / CP) - KELVIN
}

/// Dewpoint (C) from mixing ratio (kg/kg) and pressure (hPa).
fn dewpoint_from_mixing_ratio(w: f64, pres_mb: f64) -> f64 {
    let es = w * pres_mb / (0.622 + w);
    let es = es.max(0.001);
    let val = es.ln() - 6.112_f64.ln();
    let denom = 17.67 - val;
    if denom.abs() < 0.001 {
        return -40.0;
    }
    243.5 * val / denom
}

/// Wet-bulb potential temperature (approximation): follow moist adiabat from T,P.
fn theta_w(temp_c: f64, pres_mb: f64) -> f64 {
    let mut t = temp_c;
    let mut p = pres_mb;
    let dp = -5.0;
    while p > 200.0 {
        let t_k = t + KELVIN;
        let new_p = (p + dp).max(200.0);
        let dz = -(RD * t_k / G) * (dp / p);
        let gamma_m = moist_lapse_rate(t, p);
        t -= gamma_m * dz;
        p = new_p;
    }
    t + KELVIN
}

/// Interpolate environment temperature at a given pressure from sounding levels.
fn interp_env_at_p(p: f64, levels: &[SoundingLevel], get_val: fn(&SoundingLevel) -> f64) -> Option<f64> {
    if levels.is_empty() {
        return None;
    }
    if p >= levels[0].pressure_mb as f64 {
        return Some(get_val(&levels[0]));
    }
    if p <= levels.last().unwrap().pressure_mb as f64 {
        return Some(get_val(levels.last().unwrap()));
    }
    for i in 0..levels.len() - 1 {
        let p0 = levels[i].pressure_mb as f64;
        let p1 = levels[i + 1].pressure_mb as f64;
        if p <= p0 && p >= p1 {
            let f = (p0 - p) / (p0 - p1).max(0.01);
            return Some(get_val(&levels[i]) + f * (get_val(&levels[i + 1]) - get_val(&levels[i])));
        }
    }
    None
}

fn interp_env_temp_at_p(p: f64, levels: &[SoundingLevel]) -> Option<f64> {
    interp_env_at_p(p, levels, |l| l.temp_c as f64)
}

fn interp_env_td_at_p(p: f64, levels: &[SoundingLevel]) -> Option<f64> {
    interp_env_at_p(p, levels, |l| l.dewpoint_c as f64)
}

fn interp_height_at_p(p: f64, levels: &[SoundingLevel]) -> f64 {
    interp_env_at_p(p, levels, |l| l.height_m as f64).unwrap_or(0.0)
}

fn interp_wind_at_p(p: f64, levels: &[SoundingLevel]) -> (f64, f64) {
    let dir = interp_env_at_p(p, levels, |l| l.wind_dir as f64).unwrap_or(0.0);
    let spd = interp_env_at_p(p, levels, |l| l.wind_speed_kts as f64).unwrap_or(0.0);
    (dir, spd)
}

/// Interpolate at a given height AGL
fn interp_at_height(h: f64, levels: &[SoundingLevel], get: fn(&SoundingLevel) -> f64) -> f64 {
    if levels.is_empty() {
        return 0.0;
    }
    if h <= levels[0].height_m as f64 {
        return get(&levels[0]);
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

fn wind_components(wdir: f64, wspd: f64) -> (f64, f64) {
    let rad = wdir.to_radians();
    (-wspd * rad.sin(), -wspd * rad.cos())
}

fn pressure_at_height(h_target: f64, levels: &[SoundingLevel]) -> f64 {
    if levels.is_empty() {
        return 1013.25;
    }
    for i in 0..levels.len() - 1 {
        let h0 = levels[i].height_m as f64;
        let h1 = levels[i + 1].height_m as f64;
        if h_target >= h0 && h_target <= h1 {
            let frac = (h_target - h0) / (h1 - h0).max(1.0);
            let p0 = levels[i].pressure_mb as f64;
            let p1 = levels[i + 1].pressure_mb as f64;
            return p0 + frac * (p1 - p0);
        }
    }
    levels.last().unwrap().pressure_mb as f64
}

// ── Computed sounding parameters ────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct SoundingParams {
    // CAPE/CIN variants
    sb_cape: f64,
    sb_cin: f64,
    ml_cape: f64,
    ml_cin: f64,
    mu_cape: f64,
    // Heights
    lcl_hgt: f64,
    lfc_hgt: f64,
    el_hgt: f64,
    // Lapse rates
    lr_0_3: f64,
    lr_3_6: f64,
    lr_700_500: f64,
    // Shear
    shear_0_1: f64,
    shear_0_3: f64,
    shear_0_6: f64,
    eff_shear: f64,
    // SRH
    srh_500m: f64,
    srh_0_1: f64,
    srh_0_3: f64,
    eff_srh: f64,
    // Composite params
    stp_fixed: f64,
    stp_eff: f64,
    scp: f64,
    ship: f64,
    // Other
    pwat: f64,
    k_index: f64,
    total_totals: f64,
    // Storm motion (Bunkers)
    bunkers_rm: (f64, f64), // (u, v) in knots
    bunkers_lm: (f64, f64),
    mean_wind: (f64, f64),
}

/// Convert from the sounding module's precomputed params to the renderer's internal struct.
fn compute_params(profile: &SoundingProfile) -> SoundingParams {
    let pp = &profile.params;

    // Compute Bunkers storm motion u,v components for hodograph rendering.
    // storm_motion_rm/lm are stored as (dir, spd) — convert to (u, v).
    let rm_uv = wind_components(pp.storm_motion_rm.0 as f64, pp.storm_motion_rm.1 as f64);
    let lm_uv = wind_components(pp.storm_motion_lm.0 as f64, pp.storm_motion_lm.1 as f64);

    // Compute mean wind for hodograph
    let levels = &profile.levels;
    let sfc_h = if levels.is_empty() { 0.0 } else { levels[0].height_m as f64 };
    let n_steps = 60;
    let mut u_mean = 0.0;
    let mut v_mean = 0.0;
    for i in 0..=n_steps {
        let hh = sfc_h + (i as f64 / n_steps as f64) * 6000.0;
        let wd = interp_at_height(hh, levels, |l| l.wind_dir as f64);
        let ws = interp_at_height(hh, levels, |l| l.wind_speed_kts as f64);
        let (u, v) = wind_components(wd, ws);
        u_mean += u;
        v_mean += v;
    }
    if n_steps > 0 {
        u_mean /= (n_steps + 1) as f64;
        v_mean /= (n_steps + 1) as f64;
    }

    // Compute LFC and EL heights AGL for display
    let lfc_hgt = if pp.sb_lfc > 0.0 {
        interp_height_at_p(pp.sb_lfc as f64, levels) - sfc_h
    } else { 0.0 };
    let el_hgt = if pp.sb_el > 0.0 {
        interp_height_at_p(pp.sb_el as f64, levels) - sfc_h
    } else { 0.0 };

    SoundingParams {
        sb_cape: pp.sb_cape as f64,
        sb_cin: pp.sb_cin as f64,
        ml_cape: pp.ml_cape as f64,
        ml_cin: pp.ml_cin as f64,
        mu_cape: pp.mu_cape as f64,
        lcl_hgt: pp.lcl_hgt as f64,
        lfc_hgt: lfc_hgt.max(0.0),
        el_hgt: el_hgt.max(0.0),
        lr_0_3: pp.lapse_03 as f64,
        lr_3_6: pp.lapse_36 as f64,
        lr_700_500: pp.lapse_700_500 as f64,
        shear_0_1: pp.bulk_shear_01 as f64,
        shear_0_3: pp.bulk_shear_03 as f64,
        shear_0_6: pp.bulk_shear_06 as f64,
        eff_shear: pp.eff_shear as f64,
        srh_500m: pp.srh_500 as f64,
        srh_0_1: pp.srh_01 as f64,
        srh_0_3: pp.srh_03 as f64,
        eff_srh: pp.eff_srh as f64,
        stp_fixed: pp.stp_fixed as f64,
        stp_eff: pp.stp_eff as f64,
        scp: pp.scp as f64,
        ship: pp.ship as f64,
        pwat: pp.pwat as f64,
        k_index: pp.k_index as f64,
        total_totals: pp.totals as f64,
        bunkers_rm: rm_uv,
        bunkers_lm: lm_uv,
        mean_wind: (u_mean, v_mean),
    }
}

fn theta_e_approx(temp_c: f64, td_c: f64, pres_mb: f64) -> f64 {
    let t_k = temp_c + KELVIN;
    let ws = sat_mixing_ratio(td_c, pres_mb);
    let th = theta(temp_c, pres_mb);
    th * (LV * ws / (CP * t_k)).exp()
}

fn compute_cape_cin(
    levels: &[SoundingLevel],
    sfc_t: f64,
    sfc_td: f64,
    _sfc_p: f64,
) -> (f64, f64, f64, f64, f64) {
    // Returns (CAPE, CIN, LCL height, LFC height, EL height)
    if levels.len() < 3 {
        return (0.0, 0.0, 0.0, 0.0, 0.0);
    }
    let sfc_h = levels[0].height_m as f64;
    let dz = 10.0;

    // Find LCL
    let mut parcel_t = sfc_t;
    let mut parcel_td = sfc_td;
    let mut lcl_h = sfc_h;
    loop {
        if parcel_t <= parcel_td || lcl_h > sfc_h + 20_000.0 {
            break;
        }
        parcel_t -= GAMMA_D * dz;
        parcel_td -= 0.0018 * dz;
        lcl_h += dz;
    }

    // Ascent
    let max_h = levels.last().unwrap().height_m as f64;
    let mut cape = 0.0_f64;
    let mut cin = 0.0_f64;
    let mut h = sfc_h;
    let mut p_t = sfc_t;
    let mut lfc_h = max_h;
    let mut el_h = sfc_h;
    let mut prev_buoy = 0.0_f64;

    while h < max_h {
        let pres = pressure_at_height(h, levels);
        let env_t = interp_at_height(h, levels, |l| l.temp_c as f64);
        let env_td = interp_at_height(h, levels, |l| l.dewpoint_c as f64);

        let p_ws = if h >= lcl_h {
            sat_mixing_ratio(p_t, pres)
        } else {
            sat_mixing_ratio(sfc_td, pres)
        };
        let tv_p = (p_t + KELVIN) * (1.0 + 0.61 * p_ws);
        let env_ws = sat_mixing_ratio(env_td, pres);
        let tv_e = (env_t + KELVIN) * (1.0 + 0.61 * env_ws);

        let buoy = G * (tv_p - tv_e) / tv_e * dz;

        if buoy > 0.0 && prev_buoy <= 0.0 && h > lcl_h {
            lfc_h = h;
        }
        if buoy < 0.0 && prev_buoy > 0.0 && cape > 0.0 {
            el_h = h;
        }

        if tv_p > tv_e {
            cape += buoy;
            if el_h < lfc_h {
                el_h = h;
            }
        } else if h < lcl_h + 3000.0 {
            cin += buoy;
        }

        prev_buoy = buoy;

        if h < lcl_h {
            p_t -= GAMMA_D * dz;
        } else {
            let gamma_m = moist_lapse_rate(p_t, pres);
            p_t -= gamma_m * dz;
        }
        h += dz;
    }

    (cape.max(0.0), cin.min(0.0), lcl_h, lfc_h, el_h)
}

// ── Canvas ──────────────────────────────────────────────────────────

struct Canvas {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            pixels[i * 4] = COL_BG[0];
            pixels[i * 4 + 1] = COL_BG[1];
            pixels[i * 4 + 2] = COL_BG[2];
            pixels[i * 4 + 3] = COL_BG[3];
        }
        Self { pixels, w, h }
    }

    #[inline]
    fn put_pixel_blend(&mut self, x: i32, y: i32, col: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let idx = (y as u32 * self.w + x as u32) as usize * 4;
        let alpha = col[3] as f32 / 255.0;
        let inv = 1.0 - alpha;
        self.pixels[idx] = (col[0] as f32 * alpha + self.pixels[idx] as f32 * inv) as u8;
        self.pixels[idx + 1] =
            (col[1] as f32 * alpha + self.pixels[idx + 1] as f32 * inv) as u8;
        self.pixels[idx + 2] =
            (col[2] as f32 * alpha + self.pixels[idx + 2] as f32 * inv) as u8;
        self.pixels[idx + 3] = 255;
    }

    /// Wu's antialiased line drawing.
    fn draw_line_aa(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, col: [u8; 4]) {
        let steep = (y1 - y0).abs() > (x1 - x0).abs();
        let (mut x0, mut y0, mut x1, mut y1) = if steep {
            (y0, x0, y1, x1)
        } else {
            (x0, y0, x1, y1)
        };
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
        }
        let dx = x1 - x0;
        let dy = y1 - y0;
        let gradient = if dx.abs() < 0.001 { 1.0 } else { dy / dx };

        // First endpoint
        let xend = x0.round();
        let yend = y0 + gradient * (xend - x0);
        let xpxl1 = xend as i32;
        let mut intery = yend + gradient;

        // Second endpoint
        let xend2 = x1.round();
        let xpxl2 = xend2 as i32;

        for x in xpxl1..=xpxl2 {
            let fpart = intery - intery.floor();
            let y = intery as i32;
            let a1 = ((1.0 - fpart) * col[3] as f64) as u8;
            let a2 = (fpart * col[3] as f64) as u8;
            if steep {
                self.put_pixel_blend(y, x, [col[0], col[1], col[2], a1]);
                self.put_pixel_blend(y + 1, x, [col[0], col[1], col[2], a2]);
            } else {
                self.put_pixel_blend(x, y, [col[0], col[1], col[2], a1]);
                self.put_pixel_blend(x, y + 1, [col[0], col[1], col[2], a2]);
            }
            intery += gradient;
        }
    }

    /// Bresenham line (for grid/utility lines).
    fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 4]) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;
        loop {
            self.put_pixel_blend(cx, cy, col);
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    /// Thick antialiased line (draw AA line with parallel offsets).
    fn draw_thick_line_aa(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, col: [u8; 4], thickness: i32) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let nx = -dy / len;
        let ny = dx / len;
        for d in -(thickness / 2)..=(thickness / 2) {
            let off = d as f64;
            self.draw_line_aa(
                x0 + nx * off, y0 + ny * off,
                x1 + nx * off, y1 + ny * off,
                col,
            );
        }
    }

    /// Dashed line.
    fn draw_dashed_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, col: [u8; 4], dash: f64, gap: f64) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1.0 {
            return;
        }
        let ux = dx / len;
        let uy = dy / len;
        let mut dist = 0.0;
        let mut on = true;
        while dist < len {
            let seg = if on { dash } else { gap };
            let end = (dist + seg).min(len);
            if on {
                let sx = x0 + ux * dist;
                let sy = y0 + uy * dist;
                let ex = x0 + ux * end;
                let ey = y0 + uy * end;
                self.draw_line_aa(sx, sy, ex, ey, col);
            }
            dist = end;
            on = !on;
        }
    }

    /// Thick dashed line.
    fn draw_thick_dashed_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, col: [u8; 4], thickness: i32, dash: f64, gap: f64) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let nx = -dy / len;
        let ny = dx / len;
        for d in -(thickness / 2)..=(thickness / 2) {
            let off = d as f64;
            self.draw_dashed_line(
                x0 + nx * off, y0 + ny * off,
                x1 + nx * off, y1 + ny * off,
                col, dash, gap,
            );
        }
    }

    /// Fill a horizontal span with alpha blending.
    fn fill_span(&mut self, y: i32, x_left: i32, x_right: i32, col: [u8; 4]) {
        if y < 0 || y >= self.h as i32 {
            return;
        }
        let l = x_left.max(0);
        let r = x_right.min(self.w as i32 - 1);
        for x in l..=r {
            self.put_pixel_blend(x, y, col);
        }
    }

    /// Fill a rectangle with alpha blending.
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, col: [u8; 4]) {
        for row in y..y + h {
            self.fill_span(row, x, x + w - 1, col);
        }
    }

    /// Draw a rectangle outline.
    fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, col: [u8; 4]) {
        self.draw_line(x, y, x + w - 1, y, col);
        self.draw_line(x, y + h - 1, x + w - 1, y + h - 1, col);
        self.draw_line(x, y, x, y + h - 1, col);
        self.draw_line(x + w - 1, y, x + w - 1, y + h - 1, col);
    }

    /// Draw a circle outline (midpoint algorithm).
    fn draw_circle(&mut self, cx: i32, cy: i32, r: i32, col: [u8; 4]) {
        let mut x = r;
        let mut y = 0;
        let mut err = 1 - r;
        while x >= y {
            self.put_pixel_blend(cx + x, cy + y, col);
            self.put_pixel_blend(cx - x, cy + y, col);
            self.put_pixel_blend(cx + x, cy - y, col);
            self.put_pixel_blend(cx - x, cy - y, col);
            self.put_pixel_blend(cx + y, cy + x, col);
            self.put_pixel_blend(cx - y, cy + x, col);
            self.put_pixel_blend(cx + y, cy - x, col);
            self.put_pixel_blend(cx - y, cy - x, col);
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    /// Draw a 7x10 character.
    fn draw_char(&mut self, ch: char, px: i32, py: i32, col: [u8; 4]) {
        let bitmap = char_bitmap(ch);
        for (row, &bits) in bitmap.iter().enumerate() {
            for col_idx in 0..FONT_W {
                if bits & (1 << (FONT_W - 1 - col_idx)) != 0 {
                    self.put_pixel_blend(px + col_idx, py + row as i32, col);
                }
            }
        }
    }

    /// Draw a string.
    fn draw_text(&mut self, text: &str, px: i32, py: i32, col: [u8; 4]) {
        let mut x = px;
        for ch in text.chars() {
            self.draw_char(ch, x, py, col);
            x += FONT_W + 1; // 1px spacing
        }
    }

    /// Width of a text string in pixels.
    fn text_width(text: &str) -> i32 {
        let n = text.len() as i32;
        if n == 0 { 0 } else { n * (FONT_W + 1) - 1 }
    }

    /// Draw text right-aligned.
    fn draw_text_right(&mut self, text: &str, right_x: i32, py: i32, col: [u8; 4]) {
        let w = Self::text_width(text);
        self.draw_text(text, right_x - w, py, col);
    }

    /// Set a clip region: all drawing outside this rect is suppressed.
    /// Returns the old clip, or None to restore full canvas.
    fn clip_rect(&self) -> (i32, i32, i32, i32) {
        (0, 0, self.w as i32, self.h as i32)
    }
}

// ── Clipping helper ─────────────────────────────────────────────────
/// A canvas wrapper that clips to a sub-rectangle.
struct ClippedCanvas<'a> {
    canvas: &'a mut Canvas,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl<'a> ClippedCanvas<'a> {
    fn new(canvas: &'a mut Canvas, x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            x0: x.max(0),
            y0: y.max(0),
            x1: (x + w).min(canvas.w as i32),
            y1: (y + h).min(canvas.h as i32),
            canvas,
        }
    }

    fn put_pixel_blend(&mut self, x: i32, y: i32, col: [u8; 4]) {
        if x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1 {
            self.canvas.put_pixel_blend(x, y, col);
        }
    }

    fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 4]) {
        // Simple: use Bresenham with clip check per pixel
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;
        loop {
            self.put_pixel_blend(cx, cy, col);
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    fn draw_line_aa(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, col: [u8; 4]) {
        let steep = (y1 - y0).abs() > (x1 - x0).abs();
        let (mut x0, mut y0, mut x1, mut y1) = if steep {
            (y0, x0, y1, x1)
        } else {
            (x0, y0, x1, y1)
        };
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
        }
        let dx = x1 - x0;
        let dy = y1 - y0;
        let gradient = if dx.abs() < 0.001 { 1.0 } else { dy / dx };
        let xpxl1 = x0.round() as i32;
        let yend = y0 + gradient * (x0.round() - x0);
        let mut intery = yend + gradient;
        let xpxl2 = x1.round() as i32;
        for x in xpxl1..=xpxl2 {
            let fpart = intery - intery.floor();
            let y = intery as i32;
            let a1 = ((1.0 - fpart) * col[3] as f64) as u8;
            let a2 = (fpart * col[3] as f64) as u8;
            if steep {
                self.put_pixel_blend(y, x, [col[0], col[1], col[2], a1]);
                self.put_pixel_blend(y + 1, x, [col[0], col[1], col[2], a2]);
            } else {
                self.put_pixel_blend(x, y, [col[0], col[1], col[2], a1]);
                self.put_pixel_blend(x, y + 1, [col[0], col[1], col[2], a2]);
            }
            intery += gradient;
        }
    }

    fn draw_thick_line_aa(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, col: [u8; 4], thickness: i32) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let nx = -dy / len;
        let ny = dx / len;
        for d in -(thickness / 2)..=(thickness / 2) {
            let off = d as f64;
            self.draw_line_aa(
                x0 + nx * off, y0 + ny * off,
                x1 + nx * off, y1 + ny * off,
                col,
            );
        }
    }

    fn fill_span(&mut self, y: i32, x_left: i32, x_right: i32, col: [u8; 4]) {
        if y < self.y0 || y >= self.y1 {
            return;
        }
        let l = x_left.max(self.x0);
        let r = x_right.min(self.x1 - 1);
        for x in l..=r {
            self.canvas.put_pixel_blend(x, y, col);
        }
    }
}

// ── Main Renderer ───────────────────────────────────────────────────

impl SkewTRenderer {
    /// Render a professional Skew-T/Log-P diagram with hodograph and text indices.
    /// Returns an RGBA pixel buffer of size `width * height * 4`.
    pub fn render(profile: &SoundingProfile, width: u32, height: u32) -> Vec<u8> {
        let mut c = Canvas::new(width, height);
        let params = compute_params(profile);

        let skewt_w = (width as f64 * SKEWT_FRAC) as u32;
        let right_x = skewt_w as i32;
        let right_w = width - skewt_w;

        let plot_w = skewt_w as f64 - MARGIN_LEFT - MARGIN_RIGHT;
        let plot_h = height as f64 - MARGIN_TOP - MARGIN_BOT;

        // ── 1. Skew-T background ────────────────────────────────────
        Self::draw_mixing_ratio_lines(&mut c, plot_w, plot_h);
        Self::draw_moist_adiabats(&mut c, plot_w, plot_h);
        Self::draw_dry_adiabats(&mut c, plot_w, plot_h);
        Self::draw_isobars(&mut c, plot_w, plot_h, skewt_w);
        Self::draw_isotherms(&mut c, plot_w, plot_h);

        // ── 2. CAPE/CIN shading ─────────────────────────────────────
        let parcel_path = Self::compute_parcel_path(profile);
        Self::draw_cape_cin_fills(&mut c, profile, &parcel_path, plot_w, plot_h);

        // ── 3. Profiles ─────────────────────────────────────────────
        Self::draw_profile_line(&mut c, profile, true, COL_DEWP, plot_w, plot_h);
        Self::draw_profile_line(&mut c, profile, false, COL_TEMP, plot_w, plot_h);
        Self::draw_parcel_path(&mut c, &parcel_path, plot_w, plot_h);

        // ── 4. Wind barbs ───────────────────────────────────────────
        Self::draw_wind_barbs(&mut c, profile, plot_w, plot_h, skewt_w);

        // ── 5. Pressure & temperature labels ────────────────────────
        Self::draw_pressure_labels(&mut c, plot_w, plot_h);
        Self::draw_temp_labels(&mut c, plot_w, plot_h);

        // ── 6. Station/time label ───────────────────────────────────
        let title = format!("{} {}", profile.station, profile.valid_time);
        c.draw_text(&title.to_uppercase(), MARGIN_LEFT as i32, 4, COL_TEXT);

        // ── 7. Right panel: hodograph ───────────────────────────────
        let hodo_h = (height as i32) / 2;
        Self::draw_hodograph(&mut c, profile, &params, right_x, 0, right_w as i32, hodo_h);

        // ── 8. Right panel: text indices ────────────────────────────
        Self::draw_text_panel(&mut c, &params, right_x, hodo_h, right_w as i32, height as i32 - hodo_h);

        c.pixels
    }

    // ── Isobars ─────────────────────────────────────────────────────

    fn draw_isobars(c: &mut Canvas, plot_w: f64, plot_h: f64, skewt_w: u32) {
        for &p in STD_PRESSURES {
            let (_, y) = tp_to_screen(0.0, p, plot_w, plot_h);
            let yi = y as i32;
            c.draw_line(MARGIN_LEFT as i32, yi, (skewt_w as f64 - MARGIN_RIGHT) as i32, yi, COL_ISOBAR);
        }
    }

    fn draw_pressure_labels(c: &mut Canvas, plot_w: f64, plot_h: f64) {
        for &p in STD_PRESSURES {
            let (_, y) = tp_to_screen(0.0, p, plot_w, plot_h);
            let label = format!("{}", p as i32);
            c.draw_text(&label, 2, y as i32 - FONT_H / 2, COL_LABEL);
        }
    }

    // ── Isotherms ───────────────────────────────────────────────────

    fn draw_isotherms(c: &mut Canvas, plot_w: f64, plot_h: f64) {
        for t in (-80..=60).step_by(10) {
            let col = if t == 0 { COL_GRID_ZERO } else { COL_GRID };
            let thick = t == 0;
            let (x0, y0) = tp_to_screen(t as f64, P_BOT, plot_w, plot_h);
            let (x1, y1) = tp_to_screen(t as f64, P_TOP, plot_w, plot_h);
            if thick {
                c.draw_thick_line_aa(x0, y0, x1, y1, col, 2);
            } else {
                c.draw_line(x0 as i32, y0 as i32, x1 as i32, y1 as i32, col);
            }
        }
    }

    fn draw_temp_labels(c: &mut Canvas, plot_w: f64, plot_h: f64) {
        let y_bot = (MARGIN_TOP + plot_h) as i32 + 4;
        for t in (-30..=40).step_by(10) {
            let (x, _) = tp_to_screen(t as f64, P_BOT, plot_w, plot_h);
            let label = format!("{}", t);
            let tw = Canvas::text_width(&label);
            c.draw_text(&label, x as i32 - tw / 2, y_bot, COL_LABEL);
        }
    }

    // ── Dry adiabats ────────────────────────────────────────────────

    fn draw_dry_adiabats(c: &mut Canvas, plot_w: f64, plot_h: f64) {
        for start_t in (-40..=80).step_by(10) {
            let th = (start_t as f64) + KELVIN; // potential temperature
            let mut prev: Option<(i32, i32)> = None;
            let mut p = P_BOT;
            while p >= P_TOP {
                let t = temp_from_theta(th, p);
                let (sx, sy) = tp_to_screen(t, p, plot_w, plot_h);
                if let Some((px, py)) = prev {
                    c.draw_line(px, py, sx as i32, sy as i32, COL_DRY_AD);
                }
                prev = Some((sx as i32, sy as i32));
                p -= 10.0;
            }
        }
    }

    // ── Moist adiabats ──────────────────────────────────────────────

    fn draw_moist_adiabats(c: &mut Canvas, plot_w: f64, plot_h: f64) {
        for start_t in (-30..=32).step_by(4) {
            let mut t = start_t as f64;
            let mut p = 1050.0;
            let dp = -5.0;
            let mut prev: Option<(i32, i32)> = None;
            while p >= P_TOP {
                let (sx, sy) = tp_to_screen(t, p, plot_w, plot_h);
                if let Some((px, py)) = prev {
                    c.draw_line(px, py, sx as i32, sy as i32, COL_MOIST_AD);
                }
                prev = Some((sx as i32, sy as i32));
                let t_k = t + KELVIN;
                let new_p = (p + dp).max(P_TOP);
                let dz = -(RD * t_k / G) * (dp / p);
                let gamma_m = moist_lapse_rate(t, p);
                t -= gamma_m * dz;
                p = new_p;
            }
        }
    }

    // ── Mixing ratio lines ──────────────────────────────────────────

    fn draw_mixing_ratio_lines(c: &mut Canvas, plot_w: f64, plot_h: f64) {
        let ratios = [1.0, 2.0, 4.0, 7.0, 10.0, 16.0, 24.0];
        for &w in &ratios {
            let w_kg = w / 1000.0;
            let mut prev: Option<(f64, f64)> = None;
            let mut p = P_BOT;
            while p >= 400.0 {
                let td = dewpoint_from_mixing_ratio(w_kg, p);
                let (sx, sy) = tp_to_screen(td, p, plot_w, plot_h);
                if let Some((px, py)) = prev {
                    c.draw_dashed_line(px, py, sx, sy, COL_MIX_RATIO, 4.0, 4.0);
                }
                prev = Some((sx, sy));
                p -= 20.0;
            }
        }
    }

    // ── Parcel path ─────────────────────────────────────────────────

    fn compute_parcel_path(profile: &SoundingProfile) -> Vec<(f64, f64)> {
        if profile.levels.is_empty() {
            return vec![];
        }
        let sfc = &profile.levels[0];
        let sfc_t = sfc.temp_c as f64;
        let sfc_td = sfc.dewpoint_c as f64;
        let sfc_p = sfc.pressure_mb as f64;

        let mut path: Vec<(f64, f64)> = Vec::new();
        let mut t = sfc_t;
        let mut td = sfc_td;
        let mut p = sfc_p;
        let mut found_lcl = false;
        let dp = -3.0;

        path.push((t, p));

        while p > P_TOP {
            let new_p = (p + dp).max(P_TOP);
            if !found_lcl {
                let t_k = t + KELVIN;
                let new_t_k = t_k * (new_p / p).powf(RD / CP);
                t = new_t_k - KELVIN;
                td -= 0.0018 * (RD * (t + KELVIN) / G) * (p - new_p) / p * 0.5;
                if t <= td {
                    found_lcl = true;
                }
            } else {
                let t_k = t + KELVIN;
                let dz = -(RD * t_k / G) * (dp / p);
                let gamma_m = moist_lapse_rate(t, p);
                t -= gamma_m * dz;
            }
            p = new_p;
            path.push((t, p));
        }
        path
    }

    fn draw_parcel_path(c: &mut Canvas, path: &[(f64, f64)], plot_w: f64, plot_h: f64) {
        for i in 1..path.len() {
            let (x0, y0) = tp_to_screen(path[i - 1].0, path[i - 1].1, plot_w, plot_h);
            let (x1, y1) = tp_to_screen(path[i].0, path[i].1, plot_w, plot_h);
            c.draw_thick_dashed_line(x0, y0, x1, y1, COL_PARCEL, 2, 8.0, 5.0);
        }
    }

    // ── Profile lines (temp & dewpoint) ─────────────────────────────

    fn draw_profile_line(
        c: &mut Canvas,
        profile: &SoundingProfile,
        dewpoint: bool,
        col: [u8; 4],
        plot_w: f64,
        plot_h: f64,
    ) {
        let levels = &profile.levels;
        for i in 1..levels.len() {
            let t0 = if dewpoint { levels[i - 1].dewpoint_c } else { levels[i - 1].temp_c } as f64;
            let t1 = if dewpoint { levels[i].dewpoint_c } else { levels[i].temp_c } as f64;
            let p0 = levels[i - 1].pressure_mb as f64;
            let p1 = levels[i].pressure_mb as f64;
            if p0 < P_TOP || p1 > P_BOT {
                continue;
            }
            let (x0, y0) = tp_to_screen(t0, p0, plot_w, plot_h);
            let (x1, y1) = tp_to_screen(t1, p1, plot_w, plot_h);
            c.draw_thick_line_aa(x0, y0, x1, y1, col, 3);
        }
    }

    // ── CAPE/CIN fills ──────────────────────────────────────────────

    fn draw_cape_cin_fills(
        c: &mut Canvas,
        profile: &SoundingProfile,
        parcel_path: &[(f64, f64)],
        plot_w: f64,
        plot_h: f64,
    ) {
        if parcel_path.is_empty() || profile.levels.is_empty() {
            return;
        }
        for &(pt, pp) in parcel_path.iter() {
            let env_t = match interp_env_temp_at_p(pp, &profile.levels) {
                Some(t) => t,
                None => continue,
            };
            let (parcel_sx, sy) = tp_to_screen(pt, pp, plot_w, plot_h);
            let (env_sx, _) = tp_to_screen(env_t, pp, plot_w, plot_h);
            let yi = sy as i32;
            if pt > env_t {
                c.fill_span(yi, env_sx as i32, parcel_sx as i32, COL_CAPE_FILL);
            } else {
                c.fill_span(yi, parcel_sx as i32, env_sx as i32, COL_CIN_FILL);
            }
        }
    }

    // ── Wind barbs ──────────────────────────────────────────────────

    fn draw_wind_barbs(c: &mut Canvas, profile: &SoundingProfile, plot_w: f64, plot_h: f64, skewt_w: u32) {
        let bx = (skewt_w as f64 - MARGIN_RIGHT / 2.0) as i32;
        let barb_pressures = [1000.0, 975.0, 950.0, 925.0, 900.0, 875.0, 850.0, 825.0,
            800.0, 775.0, 750.0, 700.0, 650.0, 600.0, 550.0, 500.0,
            450.0, 400.0, 350.0, 300.0, 250.0, 200.0, 150.0];

        for &p in &barb_pressures {
            if p < P_TOP || p > P_BOT {
                continue;
            }
            let (dir, spd) = interp_wind_at_p(p, &profile.levels);
            if spd < 0.5 {
                continue;
            }
            let (_, sy) = tp_to_screen(0.0, p, plot_w, plot_h);
            Self::draw_single_barb(c, bx, sy as i32, dir as f32, spd as f32);
        }
    }

    fn draw_single_barb(c: &mut Canvas, cx: i32, cy: i32, wdir: f32, wspd: f32) {
        let staff_len = 22;
        let dir_rad = (wdir as f64).to_radians();
        let dx = -(dir_rad.sin());
        let dy = dir_rad.cos();
        let ex = cx + (dx * staff_len as f64) as i32;
        let ey = cy + (dy * staff_len as f64) as i32;
        c.draw_line(cx, cy, ex, ey, COL_WIND_BARB);

        let mut remaining = wspd;
        let mut pos = 0;
        let barb_len = 10.0;
        let px = -dy;
        let py = dx;

        // Pennants (50 kt)
        while remaining >= 47.5 {
            let bx0 = ex - (dx * pos as f64) as i32;
            let by0 = ey - (dy * pos as f64) as i32;
            let bx1 = bx0 + (px * barb_len) as i32;
            let by1 = by0 + (py * barb_len) as i32;
            let bx2 = ex - (dx * (pos + 5) as f64) as i32;
            let by2 = ey - (dy * (pos + 5) as f64) as i32;
            for t in 0..=5 {
                let f = t as f64 / 5.0;
                let mx = bx0 as f64 + f * (bx2 - bx0) as f64;
                let my = by0 as f64 + f * (by2 - by0) as f64;
                let tx = bx1 as f64 + f * (bx2 - bx1) as f64;
                let ty = by1 as f64 + f * (by2 - by1) as f64;
                c.draw_line(mx as i32, my as i32, tx as i32, ty as i32, COL_WIND_BARB);
            }
            remaining -= 50.0;
            pos += 6;
        }

        // Full barbs (10 kt)
        while remaining >= 7.5 {
            let bx0 = ex - (dx * pos as f64) as i32;
            let by0 = ey - (dy * pos as f64) as i32;
            let bx1 = bx0 + (px * barb_len) as i32;
            let by1 = by0 + (py * barb_len) as i32;
            c.draw_line(bx0, by0, bx1, by1, COL_WIND_BARB);
            remaining -= 10.0;
            pos += 3;
        }

        // Half barb (5 kt)
        if remaining >= 2.5 {
            let bx0 = ex - (dx * pos as f64) as i32;
            let by0 = ey - (dy * pos as f64) as i32;
            let bx1 = bx0 + (px * barb_len * 0.5) as i32;
            let by1 = by0 + (py * barb_len * 0.5) as i32;
            c.draw_line(bx0, by0, bx1, by1, COL_WIND_BARB);
        }
    }

    // ── Hodograph ───────────────────────────────────────────────────

    fn draw_hodograph(
        c: &mut Canvas,
        profile: &SoundingProfile,
        params: &SoundingParams,
        rx: i32,
        ry: i32,
        rw: i32,
        rh: i32,
    ) {
        // Panel background
        c.fill_rect(rx, ry, rw, rh, COL_PANEL_BG);
        c.draw_rect(rx, ry, rw, rh, COL_PANEL_BORDER);

        let cx = rx + rw / 2;
        let cy = ry + rh / 2 + 8;
        let max_radius = (rw.min(rh) / 2 - 20).max(20);

        // Scale: max ring = 60 kt
        let scale = max_radius as f64 / 60.0;

        // Title
        c.draw_text("HODOGRAPH", rx + 4, ry + 3, COL_TEXT_HEADER);

        // Concentric rings at 20, 40, 60 kt
        for &kt in &[20, 40, 60] {
            let r = (kt as f64 * scale) as i32;
            c.draw_circle(cx, cy, r, COL_HODO_RING);
            // Label
            let label = format!("{}", kt);
            c.draw_text(&label, cx + r + 2, cy - FONT_H / 2, COL_TEXT_DIM);
        }

        // Cross-hairs
        let r60 = (60.0 * scale) as i32;
        c.draw_line(cx - r60, cy, cx + r60, cy, COL_HODO_RING);
        c.draw_line(cx, cy - r60, cx, cy + r60, COL_HODO_RING);

        if profile.levels.is_empty() {
            return;
        }

        let sfc_h = profile.levels[0].height_m as f64;

        // Helper: get screen coords from (u, v) knots
        let uv_to_screen = |u: f64, v: f64| -> (i32, i32) {
            (cx + (u * scale) as i32, cy - (v * scale) as i32)
        };

        // Plot wind vectors by height, color-coded
        let height_color = |h_agl: f64| -> [u8; 4] {
            if h_agl < 3000.0 {
                COL_HODO_0_3
            } else if h_agl < 6000.0 {
                COL_HODO_3_6
            } else if h_agl < 9000.0 {
                COL_HODO_6_9
            } else {
                COL_HODO_9_12
            }
        };

        let max_h = sfc_h + 12000.0;
        let dh = 100.0;
        let mut h = sfc_h;
        let mut prev: Option<(i32, i32, f64)> = None;

        while h <= max_h && h <= profile.levels.last().unwrap().height_m as f64 {
            let wd = interp_at_height(h, &profile.levels, |l| l.wind_dir as f64);
            let ws = interp_at_height(h, &profile.levels, |l| l.wind_speed_kts as f64);
            let (u, v) = wind_components(wd, ws);
            let (sx, sy) = uv_to_screen(u, v);
            let h_agl = h - sfc_h;
            let col = height_color(h_agl);

            if let Some((px, py, _)) = prev {
                // Clip to panel
                if sx >= rx && sx < rx + rw && sy >= ry && sy < ry + rh {
                    c.draw_line_aa(px as f64, py as f64, sx as f64, sy as f64, col);
                }
            }
            prev = Some((sx, sy, h_agl));
            h += dh;
        }

        // Mark Bunkers RM with +
        {
            let (u, v) = params.bunkers_rm;
            let (sx, sy) = uv_to_screen(u, v);
            c.draw_line(sx - 4, sy, sx + 4, sy, COL_HODO_BUNKERS);
            c.draw_line(sx, sy - 4, sx, sy + 4, COL_HODO_BUNKERS);
        }

        // Mark mean wind with circle
        {
            let (u, v) = params.mean_wind;
            let (sx, sy) = uv_to_screen(u, v);
            c.draw_circle(sx, sy, 4, COL_HODO_MEAN);
        }

        // Height legend
        let legend_items: &[(&str, [u8; 4])] = &[
            ("0-3KM", COL_HODO_0_3),
            ("3-6KM", COL_HODO_3_6),
            ("6-9KM", COL_HODO_6_9),
            ("9-12KM", COL_HODO_9_12),
        ];
        let lx = rx + 4;
        let mut ly = ry + rh - (legend_items.len() as i32) * (FONT_H + 2) - 4;
        for &(label, col) in legend_items {
            // Color swatch
            c.fill_rect(lx, ly + 2, 10, FONT_H - 4, col);
            c.draw_text(label, lx + 14, ly, COL_TEXT_DIM);
            ly += FONT_H + 2;
        }
    }

    // ── Text panel ──────────────────────────────────────────────────

    fn draw_text_panel(
        c: &mut Canvas,
        params: &SoundingParams,
        rx: i32,
        ry: i32,
        rw: i32,
        rh: i32,
    ) {
        c.fill_rect(rx, ry, rw, rh, COL_PANEL_BG);
        c.draw_rect(rx, ry, rw, rh, COL_PANEL_BORDER);

        let lx = rx + 6;
        let vx = rx + rw - 8; // right-align values
        let mut y = ry + 6;
        let line_h = FONT_H + 3;

        let section = |c: &mut Canvas, y: &mut i32, title: &str| {
            c.draw_text(title, lx, *y, COL_TEXT_HEADER);
            *y += line_h;
        };

        let row = |c: &mut Canvas, y: &mut i32, label: &str, value: &str| {
            c.draw_text(label, lx, *y, COL_TEXT_DIM);
            c.draw_text_right(value, vx, *y, COL_TEXT);
            *y += line_h;
        };

        // CAPE / CIN
        section(c, &mut y, "CAPE / CIN");
        row(c, &mut y, "SB CAPE", &format!("{:.0} J/KG", params.sb_cape));
        row(c, &mut y, "ML CAPE", &format!("{:.0} J/KG", params.ml_cape));
        row(c, &mut y, "MU CAPE", &format!("{:.0} J/KG", params.mu_cape));
        row(c, &mut y, "SB CIN", &format!("{:.0} J/KG", params.sb_cin));
        row(c, &mut y, "ML CIN", &format!("{:.0} J/KG", params.ml_cin));
        y += 2;

        // Heights
        section(c, &mut y, "HEIGHTS");
        row(c, &mut y, "LCL", &format!("{:.0} M", params.lcl_hgt));
        row(c, &mut y, "LFC", &format!("{:.0} M", params.lfc_hgt));
        row(c, &mut y, "EL", &format!("{:.0} M", params.el_hgt));
        y += 2;

        // Lapse rates
        section(c, &mut y, "LAPSE RATES");
        row(c, &mut y, "0-3 KM", &format!("{:.1} C/KM", params.lr_0_3));
        row(c, &mut y, "3-6 KM", &format!("{:.1} C/KM", params.lr_3_6));
        row(c, &mut y, "700-500", &format!("{:.1} C/KM", params.lr_700_500));
        y += 2;

        // Shear
        section(c, &mut y, "BULK SHEAR");
        row(c, &mut y, "0-1 KM", &format!("{:.0} KT", params.shear_0_1));
        row(c, &mut y, "0-3 KM", &format!("{:.0} KT", params.shear_0_3));
        row(c, &mut y, "0-6 KM", &format!("{:.0} KT", params.shear_0_6));
        row(c, &mut y, "EFF", &format!("{:.0} KT", params.eff_shear));
        y += 2;

        // SRH
        section(c, &mut y, "SRH");
        row(c, &mut y, "0-500M", &format!("{:.0}", params.srh_500m));
        row(c, &mut y, "0-1 KM", &format!("{:.0}", params.srh_0_1));
        row(c, &mut y, "0-3 KM", &format!("{:.0}", params.srh_0_3));
        row(c, &mut y, "EFF", &format!("{:.0}", params.eff_srh));
        y += 2;

        // Composite params
        section(c, &mut y, "COMPOSITES");
        row(c, &mut y, "STP FXD", &format!("{:.1}", params.stp_fixed));
        row(c, &mut y, "STP EFF", &format!("{:.1}", params.stp_eff));
        row(c, &mut y, "SCP", &format!("{:.1}", params.scp));
        row(c, &mut y, "SHIP", &format!("{:.1}", params.ship));
        y += 2;

        // Other
        section(c, &mut y, "OTHER");
        row(c, &mut y, "PWAT", &format!("{:.1} MM", params.pwat));
        row(c, &mut y, "K-INDEX", &format!("{:.0}", params.k_index));
        row(c, &mut y, "TT", &format!("{:.0}", params.total_totals));
        y += 2;

        // Storm motion
        section(c, &mut y, "STORM MOTION");
        let rm_spd = (params.bunkers_rm.0.powi(2) + params.bunkers_rm.1.powi(2)).sqrt();
        let rm_dir = (params.bunkers_rm.0.atan2(params.bunkers_rm.1).to_degrees() + 180.0) % 360.0;
        row(c, &mut y, "BNK RM", &format!("{:.0}/{:.0} KT", rm_dir, rm_spd));
        let lm_spd = (params.bunkers_lm.0.powi(2) + params.bunkers_lm.1.powi(2)).sqrt();
        let lm_dir = (params.bunkers_lm.0.atan2(params.bunkers_lm.1).to_degrees() + 180.0) % 360.0;
        row(c, &mut y, "BNK LM", &format!("{:.0}/{:.0} KT", lm_dir, lm_spd));
    }
}
