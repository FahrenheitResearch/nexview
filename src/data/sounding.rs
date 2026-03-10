use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

// Re-export from the new sounding module for backward compatibility.
pub use crate::sounding::{SoundingProfile, SoundingLevel, SoundingParams};

// ── Upper-air station database ──────────────────────────────────────

/// A RAOB (radiosonde) station with its coordinates.
struct RaobStation {
    id: &'static str,
    lat: f64,
    lon: f64,
}

/// US upper-air network stations (subset covering CONUS + key sites).
/// These launch radiosondes at 00Z and 12Z daily.
const RAOB_STATIONS: &[RaobStation] = &[
    RaobStation { id: "KOUN", lat: 35.18, lon: -97.44 },
    RaobStation { id: "KDDC", lat: 37.77, lon: -99.97 },
    RaobStation { id: "KTOP", lat: 39.07, lon: -95.62 },
    RaobStation { id: "KSGF", lat: 37.24, lon: -93.40 },
    RaobStation { id: "KLZK", lat: 34.83, lon: -92.26 },
    RaobStation { id: "KSHV", lat: 32.45, lon: -93.84 },
    RaobStation { id: "KFWD", lat: 32.83, lon: -97.30 },
    RaobStation { id: "KAMA", lat: 35.23, lon: -101.71 },
    RaobStation { id: "KMAF", lat: 31.94, lon: -102.19 },
    RaobStation { id: "KEPZ", lat: 31.87, lon: -106.70 },
    RaobStation { id: "KABQ", lat: 35.04, lon: -106.62 },
    RaobStation { id: "KDNR", lat: 39.77, lon: -104.88 },
    RaobStation { id: "KGJT", lat: 39.12, lon: -108.53 },
    RaobStation { id: "KRIW", lat: 43.06, lon: -108.48 },
    RaobStation { id: "KUNR", lat: 44.07, lon: -103.21 },
    RaobStation { id: "KBIS", lat: 46.77, lon: -100.75 },
    RaobStation { id: "KABR", lat: 45.45, lon: -98.41 },
    RaobStation { id: "KOAX", lat: 41.32, lon: -96.37 },
    RaobStation { id: "KDVN", lat: 41.61, lon: -90.58 },
    RaobStation { id: "KILX", lat: 40.15, lon: -89.34 },
    RaobStation { id: "KGRB", lat: 44.48, lon: -88.13 },
    RaobStation { id: "KMPX", lat: 44.85, lon: -93.57 },
    RaobStation { id: "KINX", lat: 36.17, lon: -95.78 },
    RaobStation { id: "KBMX", lat: 33.17, lon: -86.77 },
    RaobStation { id: "KJAN", lat: 32.32, lon: -90.08 },
    RaobStation { id: "KLIX", lat: 30.34, lon: -89.83 },
    RaobStation { id: "KLCH", lat: 30.13, lon: -93.22 },
    RaobStation { id: "KCRP", lat: 27.77, lon: -97.50 },
    RaobStation { id: "KBRO", lat: 25.91, lon: -97.42 },
    RaobStation { id: "KDRT", lat: 29.37, lon: -100.92 },
    RaobStation { id: "KJAX", lat: 30.50, lon: -81.70 },
    RaobStation { id: "KTBW", lat: 27.70, lon: -82.40 },
    RaobStation { id: "KMFL", lat: 25.75, lon: -80.38 },
    RaobStation { id: "KXMR", lat: 28.47, lon: -80.57 },
    RaobStation { id: "KTLH", lat: 30.40, lon: -84.35 },
    RaobStation { id: "KFFC", lat: 33.36, lon: -84.57 },
    RaobStation { id: "KGSO", lat: 36.10, lon: -79.94 },
    RaobStation { id: "KMHX", lat: 34.78, lon: -76.88 },
    RaobStation { id: "KRNK", lat: 37.20, lon: -80.41 },
    RaobStation { id: "KIAD", lat: 38.95, lon: -77.46 },
    RaobStation { id: "KWAL", lat: 37.94, lon: -75.47 },
    RaobStation { id: "KOKX", lat: 40.87, lon: -72.87 },
    RaobStation { id: "KALB", lat: 42.75, lon: -73.80 },
    RaobStation { id: "KBUF", lat: 42.93, lon: -78.73 },
    RaobStation { id: "KPIT", lat: 40.53, lon: -80.23 },
    RaobStation { id: "KDTX", lat: 42.70, lon: -83.47 },
    RaobStation { id: "KAPX", lat: 44.91, lon: -84.72 },
    RaobStation { id: "KCAR", lat: 46.87, lon: -68.02 },
    RaobStation { id: "KCHH", lat: 42.05, lon: -70.02 },
    RaobStation { id: "KGYX", lat: 43.89, lon: -70.26 },
    RaobStation { id: "KBNA", lat: 36.25, lon: -86.57 },
    RaobStation { id: "KILN", lat: 39.42, lon: -83.82 },
    RaobStation { id: "KSLC", lat: 40.77, lon: -111.97 },
    RaobStation { id: "KBOI", lat: 43.57, lon: -116.22 },
    RaobStation { id: "KGGW", lat: 48.21, lon: -106.62 },
    RaobStation { id: "KTFX", lat: 47.46, lon: -111.38 },
    RaobStation { id: "KOTX", lat: 47.68, lon: -117.63 },
    RaobStation { id: "KSLE", lat: 44.92, lon: -123.00 },
    RaobStation { id: "KREV", lat: 39.57, lon: -119.80 },
    RaobStation { id: "KVEF", lat: 36.05, lon: -115.18 },
    RaobStation { id: "KFGZ", lat: 35.23, lon: -111.82 },
    RaobStation { id: "KNKX", lat: 32.87, lon: -117.15 },
    RaobStation { id: "KVBG", lat: 34.75, lon: -120.57 },
    RaobStation { id: "KOAK", lat: 37.75, lon: -122.22 },
    RaobStation { id: "KPAH", lat: 37.07, lon: -88.77 },
    RaobStation { id: "KLBF", lat: 41.13, lon: -100.68 },
];

/// Find the nearest RAOB station to the given lat/lon.
fn nearest_station(lat: f64, lon: f64) -> &'static str {
    let mut best = RAOB_STATIONS[0].id;
    let mut best_dist = f64::MAX;

    for stn in RAOB_STATIONS {
        let dlat = stn.lat - lat;
        let dlon = (stn.lon - lon) * (lat.to_radians().cos());
        let dist = dlat * dlat + dlon * dlon;
        if dist < best_dist {
            best_dist = dist;
            best = stn.id;
        }
    }
    best
}

/// Get station lat/lon for a given station ID.
fn station_coords(id: &str) -> (f64, f64) {
    for stn in RAOB_STATIONS {
        if stn.id == id {
            return (stn.lat, stn.lon);
        }
    }
    (0.0, 0.0)
}

// ── Fetcher ─────────────────────────────────────────────────────────

/// Fetches and parses sounding data from multiple sources with fallback.
pub struct SoundingFetcher {
    http: Arc<reqwest::Client>,
    runtime: Handle,
    pub result: Arc<Mutex<Option<SoundingProfile>>>,
    pub fetching: Arc<Mutex<bool>>,
}

impl SoundingFetcher {
    pub fn new(runtime: Handle) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("NexView/0.3 (github.com/FahrenheitResearch/nexview)")
            .timeout(std::time::Duration::from_secs(15))
            .danger_accept_invalid_certs(true) // Handle expired certs (rucsoundings)
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
    ///
    /// Tries multiple data sources in order:
    /// 1. HRRR model sounding (works at any lat/lon within CONUS)
    /// 2. Iowa Environmental Mesonet (IEM) JSON API — observed soundings
    /// 3. rucsoundings.noaa.gov GSD format — RAP model soundings
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

        self.runtime.spawn(async move {
            log::info!("Fetching sounding for ({lat:.2}, {lon:.2})");

            // Guard that clears `fetching` on drop — even if the task panics.
            struct FetchGuard(Arc<Mutex<bool>>);
            impl Drop for FetchGuard {
                fn drop(&mut self) {
                    *self.0.lock().unwrap() = false;
                }
            }
            let _guard = FetchGuard(Arc::clone(&fetching));

            let profile = fetch_sounding_inner(&http, lat, lon).await;

            if profile.is_none() {
                log::error!("All sounding sources failed for ({lat:.2}, {lon:.2})");
            }

            *result.lock().unwrap() = profile;
            // _guard drops here and clears fetching = false
        });
    }
}

/// Actual sounding fetch logic, separated so the caller can guarantee
/// cleanup (resetting the `fetching` flag) even if this function panics.
async fn fetch_sounding_inner(
    http: &reqwest::Client,
    lat: f64,
    lon: f64,
) -> Option<SoundingProfile> {
    // ── Source 1: HRRR model sounding (any lat/lon in CONUS) ──────
    log::info!("Trying HRRR model sounding at ({lat:.2}, {lon:.2})");
    match fetch_hrrr_sounding(lat, lon).await {
        Ok(profile) => {
            log::info!("HRRR sounding: {} levels at ({:.2}, {:.2})",
                profile.levels.len(), lat, lon);
            return Some(profile);
        }
        Err(e) => {
            log::warn!("HRRR sounding failed: {e}");
        }
    }

    let station = nearest_station(lat, lon);
    let (stn_lat, stn_lon) = station_coords(station);

    // Try latest 00Z and 12Z
    let now = chrono::Utc::now();
    let today = now.format("%Y%m%d").to_string();
    let yesterday = (now - chrono::Duration::hours(24)).format("%Y%m%d").to_string();

    // Build candidate timestamps: try recent synoptic times
    let hour = now.hour();
    let mut timestamps = Vec::new();
    if hour >= 12 {
        timestamps.push(format!("{today}1200"));
        timestamps.push(format!("{today}0000"));
    } else {
        timestamps.push(format!("{today}0000"));
        timestamps.push(format!("{yesterday}1200"));
    }
    timestamps.push(format!("{yesterday}0000"));

    // ── Source 2: IEM JSON API (observed soundings) ──────────
    for ts in &timestamps {
        let url = format!(
            "https://mesonet.agron.iastate.edu/json/raob.py?station={station}&ts={ts}"
        );
        log::info!("Trying IEM: {url}");

        match http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.text().await {
                    Ok(body) => {
                        if let Some(p) = parse_iem_json(&body, stn_lat, stn_lon) {
                            log::info!("IEM sounding parsed: {} levels from {station} at {ts}",
                                p.levels.len());
                            return Some(p);
                        } else {
                            log::warn!("IEM parse failed for {station} at {ts}");
                        }
                    }
                    Err(e) => log::warn!("IEM body read error: {e}"),
                }
            }
            Ok(resp) => log::warn!("IEM returned status {}", resp.status()),
            Err(e) => log::warn!("IEM fetch error: {e}"),
        }
    }

    // ── Source 3: rucsoundings.noaa.gov (RAP model) ─────────
    let url = format!(
        "https://rucsoundings.noaa.gov/get_soundings.cgi?\
         data_source=Op40&latest=latest&start_sounding=latest&\
         n_hrs=1.0&fcst_len=shortest&airport={lat}%2C{lon}&\
         text=Ascii%20text%20%28GSD%20format%29"
    );
    log::info!("Trying rucsoundings: {url}");

    match http.get(&url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(body) => {
                if let Some(p) = parse_gsd(&body, lat, lon) {
                    log::info!("GSD sounding parsed: {} levels", p.levels.len());
                    return Some(p);
                } else {
                    log::warn!("Failed to parse GSD response");
                }
            }
            Err(e) => log::warn!("GSD body read error: {e}"),
        },
        Err(e) => log::warn!("GSD fetch error: {e}"),
    }

    None
}

use chrono::Timelike;

// ── HRRR model sounding ────────────────────────────────────────────

/// Fetch a model sounding from HRRR pressure level data.
/// Runs the blocking hrrr-render fetch on a dedicated thread via spawn_blocking.
async fn fetch_hrrr_sounding(lat: f64, lon: f64) -> Result<SoundingProfile, String> {
    // Wrap spawn_blocking in a timeout to prevent indefinite hangs
    // HRRR sounding downloads ~150 fields; cap total time at 30 seconds
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::task::spawn_blocking(move || {
            let status_fn = |msg: &str| {
                log::info!("HRRR sounding: {}", msg);
            };
            hrrr_render::sounding::fetch_model_sounding("latest", 0, lat, lon, &status_fn)
        })
    ).await
        .map_err(|_| "HRRR sounding timed out after 30s".to_string())?
        .map_err(|e| format!("spawn_blocking error: {e}"))?;

    let sounding = result.map_err(|e| format!("HRRR fetch error: {e}"))?;

    // Convert hrrr-render ModelSounding to our SoundingProfile
    let levels: Vec<SoundingLevel> = sounding.levels.iter()
        .filter(|l| l.pressure_mb.is_finite() && l.height_m.is_finite()
            && l.temp_c.is_finite() && l.dewpoint_c.is_finite())
        .map(|l| SoundingLevel {
            pressure_mb: l.pressure_mb as f32,
            height_m: l.height_m as f32,
            temp_c: l.temp_c as f32,
            dewpoint_c: l.dewpoint_c as f32,
            wind_dir: l.wind_dir as f32,
            wind_speed_kts: l.wind_speed_kts as f32,
        })
        .collect();

    if levels.len() < 3 {
        return Err(format!("HRRR sounding: only {} valid levels", levels.len()));
    }

    let station = format!("HRRR {:.1},{:.1}", lat, lon);
    let valid_time = format!("{}z f{:02}",
        sounding.run_date.get(4..).unwrap_or(&sounding.run_date),
        sounding.forecast_hour);

    Ok(SoundingProfile::new(levels, station, valid_time, lat, lon))
}

// ── IEM JSON parser ─────────────────────────────────────────────────

/// Parse the IEM JSON sounding format.
///
/// Expected format:
/// ```json
/// {
///   "profiles": [{
///     "station": "KOUN",
///     "valid": "2026-03-09T00:00:00Z",
///     "profile": [
///       {"pres": 972.0, "hght": 357.0, "tmpc": 22.4, "dwpc": 2.4, "drct": 170.0, "sknt": 7.0},
///       ...
///     ]
///   }]
/// }
/// ```
fn parse_iem_json(body: &str, lat: f64, lon: f64) -> Option<SoundingProfile> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;

    let profiles = json.get("profiles")?.as_array()?;
    if profiles.is_empty() {
        return None;
    }

    let first = &profiles[0];
    let station = first.get("station")?.as_str().unwrap_or("UNKNOWN").to_string();
    let valid_time = first.get("valid")?.as_str().unwrap_or("").to_string();
    let profile_arr = first.get("profile")?.as_array()?;

    let mut levels: Vec<SoundingLevel> = Vec::new();

    for entry in profile_arr {
        let pres = entry.get("pres").and_then(|v| v.as_f64());
        let hght = entry.get("hght").and_then(|v| v.as_f64());
        let tmpc = entry.get("tmpc").and_then(|v| v.as_f64());
        let dwpc = entry.get("dwpc").and_then(|v| v.as_f64());
        let drct = entry.get("drct").and_then(|v| v.as_f64());
        let sknt = entry.get("sknt").and_then(|v| v.as_f64());

        // Skip levels with missing essential data
        if let (Some(p), Some(h), Some(t), Some(td)) = (pres, hght, tmpc, dwpc) {
            if p > 0.0 && p < 1100.0 && h > -1000.0 {
                levels.push(SoundingLevel {
                    pressure_mb: p as f32,
                    height_m: h as f32,
                    temp_c: t as f32,
                    dewpoint_c: td as f32,
                    wind_dir: drct.unwrap_or(0.0) as f32,
                    wind_speed_kts: sknt.unwrap_or(0.0) as f32,
                });
            }
        }
    }

    if levels.len() < 3 {
        log::warn!("IEM parse: insufficient levels ({})", levels.len());
        return None;
    }

    // Filter out any NaN/Inf values that could cause panics downstream.
    levels.retain(|l| l.pressure_mb.is_finite() && l.height_m.is_finite()
        && l.temp_c.is_finite() && l.dewpoint_c.is_finite());

    if levels.len() < 3 {
        log::warn!("IEM parse: insufficient finite levels after filtering");
        return None;
    }

    // Sort by decreasing pressure (surface first, top last).
    levels.sort_by(|a, b| b.pressure_mb.partial_cmp(&a.pressure_mb)
        .unwrap_or(std::cmp::Ordering::Equal));

    Some(SoundingProfile::new(levels, station, valid_time, lat, lon))
}

// ── GSD format parser ───────────────────────────────────────────────

/// Parse the GSD text format returned by rucsoundings.noaa.gov.
///
/// The format has header/metadata lines starting with type codes, followed by
/// data lines. Data lines contain 7 whitespace-separated numeric columns:
///   TYPE  PRESSURE  HEIGHT  TEMP  DEWPT  WDIR  WSPD
/// where TYPE is an integer code (e.g. 4=mandatory, 5=significant, etc.).
/// Temperatures are in tenths of C, winds in knots.
fn parse_gsd(body: &str, lat: f64, lon: f64) -> Option<SoundingProfile> {
    let mut station = String::new();
    let mut valid_time = String::new();
    let mut levels: Vec<SoundingLevel> = Vec::new();

    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Station header line (type 1): contains station id
        if line.starts_with("1") && station.is_empty() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                station = parts.get(2).unwrap_or(&"UNKNOWN").to_string();
            }
        }

        // Check for valid time info (type 2 header)
        if line.starts_with("2") && valid_time.is_empty() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                valid_time = format!("{} {}", parts.get(1).unwrap_or(&""), parts.get(2).unwrap_or(&""));
            }
        }

        // Data lines: first field is type code (4,5,6,7,8,9), remaining are data.
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 7 {
            if let Ok(type_code) = parts[0].parse::<i32>() {
                if (4..=9).contains(&type_code) {
                    let pres: f32 = parts[1].parse().unwrap_or(-9999.0);
                    let hght: f32 = parts[2].parse().unwrap_or(-9999.0);
                    let temp_raw: f32 = parts[3].parse().unwrap_or(-9999.0);
                    let dwpt_raw: f32 = parts[4].parse().unwrap_or(-9999.0);
                    let wdir: f32 = parts[5].parse().unwrap_or(-9999.0);
                    let wspd: f32 = parts[6].parse().unwrap_or(-9999.0);

                    // GSD format: temps in tenths of C
                    let temp_c = temp_raw / 10.0;
                    let dewpt_c = dwpt_raw / 10.0;

                    // Skip levels with missing data
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
        log::warn!("GSD parse: insufficient levels ({})", levels.len());
        return None;
    }

    // Filter out any NaN/Inf values that could cause panics downstream.
    levels.retain(|l| l.pressure_mb.is_finite() && l.height_m.is_finite()
        && l.temp_c.is_finite() && l.dewpoint_c.is_finite());

    if levels.len() < 3 {
        log::warn!("GSD parse: insufficient finite levels after filtering");
        return None;
    }

    // Sort by decreasing pressure (surface first, top last).
    levels.sort_by(|a, b| b.pressure_mb.partial_cmp(&a.pressure_mb)
        .unwrap_or(std::cmp::Ordering::Equal));

    Some(SoundingProfile::new(levels, station, valid_time, lat, lon))
}
