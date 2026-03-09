use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

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

/// A complete sounding profile with computed severe weather indices.
#[derive(Debug, Clone)]
pub struct SoundingProfile {
    pub levels: Vec<SoundingLevel>,
    pub station: String,
    pub valid_time: String,
    // Computed indices
    pub cape: f32,
    pub cin: f32,
    pub lcl_m: f32,
    pub srh_0_1: f32,
    pub srh_0_3: f32,
    pub bulk_shear_0_6: f32,
    pub sig_tornado: f32,
}

/// Fetches and parses model soundings from the RUC/RAP sounding server.
pub struct SoundingFetcher {
    http: Arc<reqwest::Client>,
    runtime: Handle,
    pub result: Arc<Mutex<Option<SoundingProfile>>>,
    pub fetching: Arc<Mutex<bool>>,
}

// ── Physical constants ──────────────────────────────────────────────

const LV: f64 = 2.501e6;   // Latent heat of vaporisation (J/kg)
const RD: f64 = 287.04;    // Specific gas constant for dry air (J/(kg·K))
const RV: f64 = 461.5;     // Specific gas constant for water vapour (J/(kg·K))
const CP: f64 = 1004.0;    // Specific heat of dry air at const pressure (J/(kg·K))
const G: f64 = 9.80665;    // Gravitational acceleration (m/s²)
const GAMMA_D: f64 = G / CP; // Dry adiabatic lapse rate (K/m)

// ── Thermodynamic helpers ───────────────────────────────────────────

/// Saturation vapour pressure (hPa) via the Bolton (1980) formula.
fn sat_vapor_pressure(temp_c: f64) -> f64 {
    6.112 * ((17.67 * temp_c) / (temp_c + 243.5)).exp()
}

/// Saturation mixing ratio (kg/kg) given temperature (°C) and pressure (hPa).
fn sat_mixing_ratio(temp_c: f64, pres_mb: f64) -> f64 {
    let es = sat_vapor_pressure(temp_c);
    0.622 * es / (pres_mb - es).max(0.1)
}

/// Virtual temperature (K) given temperature (°C) and mixing ratio (kg/kg).
fn virtual_temp_k(temp_c: f64, w: f64) -> f64 {
    (temp_c + 273.15) * (1.0 + 0.61 * w)
}

/// Moist adiabatic lapse rate (K/m) at given T (°C) and P (hPa).
fn moist_lapse_rate(temp_c: f64, pres_mb: f64) -> f64 {
    let t_k = temp_c + 273.15;
    let ws = sat_mixing_ratio(temp_c, pres_mb);
    let numer = 1.0 + LV * ws / (RD * t_k);
    let denom = 1.0 + LV * LV * ws / (CP * RV * t_k * t_k);
    GAMMA_D * numer / denom
}

/// Pressure at a given height using the hypsometric equation, iteratively.
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

/// Linearly interpolate a value at a given height from the profile.
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

/// Wind components (u, v) in knots from direction (degrees) and speed.
fn wind_components(wdir: f64, wspd: f64) -> (f64, f64) {
    let rad = wdir.to_radians();
    let u = -wspd * rad.sin();
    let v = -wspd * rad.cos();
    (u, v)
}

// ── Index computation ───────────────────────────────────────────────

fn compute_indices(levels: &[SoundingLevel]) -> (f32, f32, f32, f32, f32, f32, f32) {
    if levels.len() < 3 {
        return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    }

    let sfc = &levels[0];
    let sfc_h = sfc.height_m as f64;
    let sfc_t = sfc.temp_c as f64;
    let sfc_td = sfc.dewpoint_c as f64;

    // ── LCL ─────────────────────────────────────────────────────────
    // Lift dry-adiabatically until temp reaches dewpoint (iterative).
    let mut parcel_t = sfc_t;
    let mut parcel_td = sfc_td;
    let mut lcl_h = sfc_h;
    let dz = 10.0; // 10 m steps
    loop {
        if parcel_t <= parcel_td || lcl_h > 20_000.0 + sfc_h {
            break;
        }
        parcel_t -= GAMMA_D * dz;
        // Dewpoint decreases ~0.0018°C/m during dry ascent (mixing ratio conserved approximation).
        parcel_td -= 0.0018 * dz;
        lcl_h += dz;
    }
    let lcl_m_agl = lcl_h - sfc_h;

    // ── Parcel ascent above LCL (moist adiabat) + CAPE/CIN ─────────
    let max_h = levels.last().unwrap().height_m as f64;
    let mut cape: f64 = 0.0;
    let mut cin: f64 = 0.0;
    let mut h = sfc_h;
    let mut p_t = sfc_t; // parcel temperature during ascent

    while h < max_h {
        let pres = pressure_at_height(h, levels);
        let env_t = interp_at_height(h, levels, |l| l.temp_c as f64);
        let env_td = interp_at_height(h, levels, |l| l.dewpoint_c as f64);

        // Parcel virtual temperature
        let p_ws = if h >= lcl_h {
            sat_mixing_ratio(p_t, pres)
        } else {
            sat_mixing_ratio(sfc_td, pres) // conserve mixing ratio below LCL
        };
        let tv_parcel = virtual_temp_k(p_t, p_ws);

        // Environment virtual temperature
        let env_ws = sat_mixing_ratio(env_td, pres);
        let tv_env = virtual_temp_k(env_t, env_ws);

        let buoy = G * (tv_parcel - tv_env) / tv_env * dz;

        if tv_parcel > tv_env {
            cape += buoy;
        } else if h < lcl_h + 3000.0 {
            // CIN only counted below ~3 km above LCL
            cin += buoy; // buoy is negative here
        }

        // Step parcel temperature
        if h < lcl_h {
            p_t -= GAMMA_D * dz;
        } else {
            let gamma_m = moist_lapse_rate(p_t, pres);
            p_t -= gamma_m * dz;
        }

        h += dz;
    }

    let cape = cape.max(0.0) as f32;
    let cin = cin.min(0.0) as f32;
    let lcl_m = lcl_m_agl as f32;

    // ── Bulk wind shear 0-6 km ──────────────────────────────────────
    let h6 = sfc_h + 6000.0;
    let u_sfc;
    let v_sfc;
    {
        let wd = interp_at_height(sfc_h, levels, |l| l.wind_dir as f64);
        let ws = interp_at_height(sfc_h, levels, |l| l.wind_speed_kts as f64);
        let (u, v) = wind_components(wd, ws);
        u_sfc = u;
        v_sfc = v;
    }
    let u_6k;
    let v_6k;
    {
        let wd = interp_at_height(h6, levels, |l| l.wind_dir as f64);
        let ws = interp_at_height(h6, levels, |l| l.wind_speed_kts as f64);
        let (u, v) = wind_components(wd, ws);
        u_6k = u;
        v_6k = v;
    }
    let bulk_shear_0_6 = ((u_6k - u_sfc).powi(2) + (v_6k - v_sfc).powi(2)).sqrt() as f32;

    // ── Bunkers right-mover storm motion ────────────────────────────
    // Mean wind 0-6 km and deviation.
    let n_steps = 60; // every 100 m from 0 to 6 km
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
    u_mean /= (n_steps + 1) as f64;
    v_mean /= (n_steps + 1) as f64;

    // Shear vector 0-6km
    let du = u_6k - u_sfc;
    let dv = v_6k - v_sfc;
    let shear_mag = (du * du + dv * dv).sqrt().max(0.001);
    // Deviation magnitude = 7.5 m/s ≈ 14.6 kt
    let d = 7.5 * 1.944; // convert m/s to knots
    let u_storm = u_mean + d * dv / shear_mag;
    let v_storm = v_mean - d * du / shear_mag;

    // ── SRH calculation ─────────────────────────────────────────────
    let compute_srh = |depth_m: f64| -> f64 {
        let mut srh = 0.0;
        let step = 100.0;
        let mut prev_u = u_sfc;
        let mut prev_v = v_sfc;
        let mut hh = sfc_h + step;
        while hh <= sfc_h + depth_m {
            let wd = interp_at_height(hh, levels, |l| l.wind_dir as f64);
            let ws = interp_at_height(hh, levels, |l| l.wind_speed_kts as f64);
            let (u, v) = wind_components(wd, ws);
            srh += (u - prev_u) * (v - v_storm) - (v - prev_v) * (u - u_storm);
            prev_u = u;
            prev_v = v;
            hh += step;
        }
        srh
    };

    let srh_0_1 = compute_srh(1000.0) as f32;
    let srh_0_3 = compute_srh(3000.0) as f32;

    // ── Significant Tornado Parameter ───────────────────────────────
    let lcl_term = ((2000.0 - lcl_m_agl) / 1000.0).clamp(0.0, 1.0);
    let stp = ((cape as f64) / 1500.0)
        * ((srh_0_1 as f64) / 150.0)
        * ((bulk_shear_0_6 as f64) / 20.0)
        * lcl_term;
    let sig_tornado = stp.max(0.0) as f32;

    (cape, cin, lcl_m, srh_0_1, srh_0_3, bulk_shear_0_6, sig_tornado)
}

// ── GSD format parser ───────────────────────────────────────────────

/// Parse the GSD text format returned by rucsoundings.noaa.gov.
///
/// The format has header/metadata lines starting with type codes, followed by
/// data lines. Data lines contain 7 whitespace-separated numeric columns:
///   TYPE  PRESSURE  HEIGHT  TEMP  DEWPT  WDIR  WSPD
/// where TYPE is an integer code (e.g. 4=mandatory, 5=significant, etc.).
/// Temperatures are in tenths of °C, winds in knots.
fn parse_gsd(body: &str) -> Option<SoundingProfile> {
    let mut station = String::new();
    let mut valid_time = String::new();
    let mut levels: Vec<SoundingLevel> = Vec::new();

    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Station header line (type 1): contains station id
        if line.starts_with("1") && station.is_empty() {
            // Try to extract station from fields
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                station = parts.get(2).unwrap_or(&"UNKNOWN").to_string();
            }
        }

        // Check for valid time info (type 2 or 3 header)
        if line.starts_with("2") && valid_time.is_empty() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                valid_time = format!("{} {}", parts.get(1).unwrap_or(&""), parts.get(2).unwrap_or(&""));
            }
        }

        // Data lines: first field is type code (4,5,6,7,8,9), remaining are data.
        // We accept types 4-9 as valid data lines.
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 7 {
            if let Ok(type_code) = parts[0].parse::<i32>() {
                if (4..=9).contains(&type_code) {
                    // Parse columns: TYPE PRES HGHT TEMP DWPT WDIR WSPD
                    let pres: f32 = parts[1].parse().unwrap_or(-9999.0);
                    let hght: f32 = parts[2].parse().unwrap_or(-9999.0);
                    let temp_raw: f32 = parts[3].parse().unwrap_or(-9999.0);
                    let dwpt_raw: f32 = parts[4].parse().unwrap_or(-9999.0);
                    let wdir: f32 = parts[5].parse().unwrap_or(-9999.0);
                    let wspd: f32 = parts[6].parse().unwrap_or(-9999.0);

                    // GSD format: temps in tenths of °C
                    let temp_c = temp_raw / 10.0;
                    let dewpt_c = dwpt_raw / 10.0;

                    // Skip levels with missing data (flagged as 99999 or negative pressure).
                    if pres > 0.0
                        && pres < 1100.0
                        && hght > -1000.0
                        && temp_raw > -9990.0
                        && dwpt_raw > -9990.0
                    {
                        levels.push(SoundingLevel {
                            pressure_mb: pres,
                            height_m: hght,
                            temp_c,
                            dewpoint_c: dewpt_c,
                            wind_dir: if wdir >= 0.0 && wdir <= 360.0 { wdir } else { 0.0 },
                            wind_speed_kts: if wspd >= 0.0 { wspd } else { 0.0 },
                        });
                    }
                }
            }
        }

        i += 1;
    }

    if levels.len() < 3 {
        log::warn!("Sounding parse: insufficient levels ({})", levels.len());
        return None;
    }

    // Sort by decreasing pressure (surface first, top last).
    levels.sort_by(|a, b| b.pressure_mb.partial_cmp(&a.pressure_mb).unwrap());

    let (cape, cin, lcl_m, srh_0_1, srh_0_3, bulk_shear_0_6, sig_tornado) =
        compute_indices(&levels);

    Some(SoundingProfile {
        levels,
        station,
        valid_time,
        cape,
        cin,
        lcl_m,
        srh_0_1,
        srh_0_3,
        bulk_shear_0_6,
        sig_tornado,
    })
}

// ── Fetcher ─────────────────────────────────────────────────────────

impl SoundingFetcher {
    pub fn new(runtime: Handle) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("NexView/0.3 (github.com/FahrenheitResearch/nexview)")
            .build()
            .expect("Failed to build HTTP client for sounding fetcher");

        Self {
            http: Arc::new(client),
            runtime,
            result: Arc::new(Mutex::new(None)),
            fetching: Arc::new(Mutex::new(false)),
        }
    }

    /// Returns a reference to the latest fetched profile.
    pub fn profile(&self) -> Option<SoundingProfile> {
        self.result.lock().unwrap().clone()
    }

    /// Returns true if a fetch is currently in progress.
    pub fn is_fetching(&self) -> bool {
        *self.fetching.lock().unwrap()
    }

    /// Kick off an async fetch of a sounding at the given lat/lon.
    pub fn fetch_sounding(&self, lat: f64, lon: f64) {
        // Prevent concurrent fetches.
        {
            let mut f = self.fetching.lock().unwrap();
            if *f {
                return;
            }
            *f = true;
        }

        let http = Arc::clone(&self.http);
        let result = Arc::clone(&self.result);
        let fetching = Arc::clone(&self.fetching);

        let url = format!(
            "https://rucsoundings.noaa.gov/get_soundings.cgi?\
             data_source=Op40&latest=latest&start_sounding=latest&\
             n_hrs=1.0&fcst_len=shortest&airport={lat}%2C{lon}&\
             text=Ascii%20text%20%28GSD%20format%29"
        );

        self.runtime.spawn(async move {
            log::info!("Fetching sounding for ({lat:.2}, {lon:.2})");
            match http.get(&url).send().await {
                Ok(resp) => match resp.text().await {
                    Ok(body) => {
                        let profile = parse_gsd(&body);
                        if profile.is_none() {
                            log::warn!("Failed to parse sounding GSD response");
                        }
                        *result.lock().unwrap() = profile;
                    }
                    Err(e) => log::error!("Sounding body read error: {e}"),
                },
                Err(e) => log::error!("Sounding fetch error: {e}"),
            }
            *fetching.lock().unwrap() = false;
        });
    }
}
