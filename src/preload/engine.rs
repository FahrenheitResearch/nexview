use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use std::sync::RwLock;
use tokio::sync::Semaphore;

use crate::data::alerts::WeatherAlert;
use crate::nexrad::level2::Level2File;
use crate::nexrad::products::RadarProduct;
use crate::nexrad::sites::{self, RADAR_SITES};
use crate::render::RadarRenderer;

use super::cache::{CachedSite, SiteCache};

const NEXRAD_BASE_URL: &str = "https://unidata-nexrad-level2.s3.amazonaws.com";
const MAX_CONCURRENT_DOWNLOADS: usize = 20;

/// Background preload engine that fetches and caches NEXRAD data from
/// multiple sites concurrently.  Shares an `Arc<RwLock<SiteCache>>` with
/// the UI thread so cached data can be read without blocking downloads.
pub struct PreloadEngine {
    runtime: Handle,
    http: Arc<reqwest::Client>,
    cache: Arc<RwLock<SiteCache>>,
}

impl PreloadEngine {
    pub fn new(runtime: Handle) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("NexView/0.3 Radar Preloader")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            runtime,
            http: Arc::new(http),
            cache: Arc::new(RwLock::new(SiteCache::new())),
        }
    }

    /// Get a handle to the shared cache for the UI thread to read.
    pub fn get_cache(&self) -> Arc<RwLock<SiteCache>> {
        Arc::clone(&self.cache)
    }

    /// Kick off background fetches for the given stations.
    /// `priority_sites` are fetched first, then any remaining NEXRAD sites
    /// can be added later via another call.
    pub fn start_preload(&self, priority_sites: Vec<String>) {
        let http = Arc::clone(&self.http);
        let cache = Arc::clone(&self.cache);

        self.runtime.spawn(async move {
            let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS));
            let mut handles = Vec::new();

            for station_id in priority_sites {
                let station = station_id.to_uppercase();

                // Skip if already cached and fresh
                {
                    let c = cache.read().unwrap();
                    if c.has(&station) {
                        if let Some(entry) = c.get(&station) {
                            if !entry.stale {
                                continue;
                            }
                        }
                    }
                }

                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let http = Arc::clone(&http);
                let cache = Arc::clone(&cache);

                let handle = tokio::spawn(async move {
                    match Self::fetch_and_parse(&http, &station).await {
                        Ok(entry) => {
                            log::info!(
                                "Preload: cached {} ({} sweeps)",
                                station,
                                entry.file.sweeps.len()
                            );
                            cache.write().unwrap().insert(entry);
                        }
                        Err(e) => {
                            log::warn!("Preload: failed to fetch {}: {}", station, e);
                        }
                    }
                    drop(permit);
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }
            log::info!("Preload batch complete");
        });
    }

    /// Preload ALL CONUS NEXRAD sites for national mosaic mode.
    pub fn preload_all_conus(&self) {
        let all_stations: Vec<String> = RADAR_SITES.iter()
            .map(|s| s.id.to_string())
            .collect();
        log::info!("Preload: starting CONUS mosaic fetch of {} sites", all_stations.len());
        self.start_preload(all_stations);
    }

    /// Extract stations near active weather alert polygons and preload them.
    pub fn preload_active_weather(&self, alerts: &[WeatherAlert]) {
        let mut stations_to_fetch: Vec<String> = Vec::new();

        for alert in alerts {
            if alert.polygon.is_empty() {
                continue;
            }

            // Compute centroid of the alert polygon
            let (sum_lat, sum_lon) = alert
                .polygon
                .iter()
                .fold((0.0_f64, 0.0_f64), |(alat, alon), &(lat, lon)| {
                    (alat + lat, alon + lon)
                });
            let n = alert.polygon.len() as f64;
            let center_lat = sum_lat / n;
            let center_lon = sum_lon / n;

            // Find the nearest NEXRAD site to this alert centroid
            if let Some(site) = sites::find_nearest_site(center_lat, center_lon) {
                let id = site.id.to_string();
                if !stations_to_fetch.contains(&id) {
                    stations_to_fetch.push(id);
                }
            }

            // Also grab any sites whose coverage area overlaps the polygon
            // (rough check: site within ~250 km of any polygon vertex)
            for vertex in &alert.polygon {
                for site in RADAR_SITES.iter() {
                    let dist_deg = ((site.lat - vertex.0).powi(2)
                        + (site.lon - vertex.1).powi(2))
                    .sqrt();
                    // ~2.25 degrees ≈ 250 km at mid-latitudes
                    if dist_deg < 2.25 {
                        let id = site.id.to_string();
                        if !stations_to_fetch.contains(&id) {
                            stations_to_fetch.push(id);
                        }
                    }
                }
            }
        }

        if !stations_to_fetch.is_empty() {
            log::info!(
                "Preload: queueing {} sites near weather alerts",
                stations_to_fetch.len()
            );
            self.start_preload(stations_to_fetch);
        }
    }

    /// Re-fetch any cached sites older than `max_age_secs`.
    pub fn refresh_stale(&self, max_age_secs: u64) {
        let cache = Arc::clone(&self.cache);
        let http = Arc::clone(&self.http);

        self.runtime.spawn(async move {
            let max_age = Duration::from_secs(max_age_secs);
            let stale_stations: Vec<String>;

            {
                let mut c = cache.write().unwrap();
                c.mark_stale(max_age);
                stale_stations = c
                    .stations_loaded()
                    .into_iter()
                    .filter(|id| c.get(id).map_or(false, |e| e.stale))
                    .collect();
            }

            if stale_stations.is_empty() {
                return;
            }

            log::info!("Preload: refreshing {} stale sites", stale_stations.len());

            let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS));
            let mut handles = Vec::new();

            for station in stale_stations {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let http = Arc::clone(&http);
                let cache = Arc::clone(&cache);

                let handle = tokio::spawn(async move {
                    match Self::fetch_and_parse(&http, &station).await {
                        Ok(entry) => {
                            log::info!("Preload: refreshed {}", station);
                            cache.write().unwrap().insert(entry);
                        }
                        Err(e) => {
                            log::warn!("Preload: refresh failed for {}: {}", station, e);
                        }
                    }
                    drop(permit);
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }
        });
    }

    // ── internal helpers ──────────────────────────────────────────────

    /// Fetch the latest file for `station` from S3, parse it, render a
    /// 256x256 reflectivity thumbnail, and return a ready-to-cache entry.
    async fn fetch_and_parse(
        http: &reqwest::Client,
        station: &str,
    ) -> Result<CachedSite, String> {
        let now = chrono::Utc::now();
        let today = now.date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let mut latest_key: Option<String> = None;

        // Try today, then yesterday
        for date in &[today, yesterday] {
            let prefix = format!(
                "{:04}/{:02}/{:02}/{}/",
                date.year(),
                date.month(),
                date.day(),
                station.to_uppercase()
            );

            let url = format!("{}?list-type=2&prefix={}", NEXRAD_BASE_URL, prefix);

            let resp = http
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("S3 list failed for {}: {}", station, e))?;

            let body = resp
                .text()
                .await
                .map_err(|e| format!("S3 list body read failed: {}", e))?;

            let files = parse_s3_list_xml(&body);
            if let Some(last) = files.last() {
                latest_key = Some(last.clone());
                break;
            }
        }

        let key = latest_key
            .ok_or_else(|| format!("No NEXRAD files found for {}", station))?;

        // Download
        let url = format!("{}/{}", NEXRAD_BASE_URL, key);
        let resp = http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Download failed for {}: {}", station, e))?;

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Body read failed for {}: {}", station, e))?;

        let raw = maybe_decompress_gz(bytes.to_vec());

        // Parse
        let file = Level2File::parse(&raw)
            .map_err(|e| format!("Parse failed for {}: {}", station, e))?;

        // Compute max reflectivity from the lowest REF sweep
        let max_reflectivity = file.sweeps.iter()
            .find(|s| s.radials.iter().any(|r| {
                r.moments.iter().any(|m| m.product == RadarProduct::Reflectivity)
            }))
            .map(|sweep| {
                sweep.radials.iter()
                    .flat_map(|r| r.moments.iter()
                        .filter(|m| m.product == RadarProduct::Reflectivity)
                        .flat_map(|m| m.data.iter().copied()))
                    .fold(f32::NEG_INFINITY, f32::max)
            })
            .unwrap_or(f32::NEG_INFINITY);

        // Render 256x256 REF thumbnail from the lowest tilt
        let thumbnail_pixels = render_thumbnail(&file, station);

        Ok(CachedSite {
            station_id: station.to_uppercase(),
            file,
            fetched_at: Instant::now(),
            thumbnail_pixels,
            max_reflectivity,
            stale: false,
        })
    }
}

// ── free-standing helpers (mirrors logic from aws.rs) ─────────────────

/// Parse S3 ListObjectsV2 XML to extract file keys, sorted ascending.
fn parse_s3_list_xml(xml: &str) -> Vec<String> {
    let mut keys = Vec::new();

    for contents in xml.split("<Contents>").skip(1) {
        let end = contents.find("</Contents>").unwrap_or(contents.len());
        let block = &contents[..end];

        if let Some(key) = extract_xml_tag(block, "Key") {
            let display = key.rsplit('/').next().unwrap_or(&key);
            // Skip MDM metadata files and empty keys
            if key.is_empty() || display.ends_with("_MDM") || display.ends_with(".md") {
                continue;
            }
            keys.push(key);
        }
    }

    keys.sort();
    keys
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml.find(&close)?;
    Some(xml[start..end].to_string())
}

/// Decompress gzip if the data starts with the gzip magic bytes.
fn maybe_decompress_gz(data: Vec<u8>) -> Vec<u8> {
    if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(&data[..]);
        let mut decompressed = Vec::new();
        match decoder.read_to_end(&mut decompressed) {
            Ok(_) => decompressed,
            Err(_) => data,
        }
    } else {
        data
    }
}

/// Render a 256x256 reflectivity thumbnail from the lowest tilt.
/// Returns `None` if the file has no sweeps or rendering fails.
fn render_thumbnail(file: &Level2File, station_id: &str) -> Option<Vec<u8>> {
    let site = sites::find_site(station_id)?;
    let sweep = file.sweeps.first()?;
    let rendered =
        RadarRenderer::render_sweep(sweep, RadarProduct::Reflectivity, site, 256)?;
    Some(rendered.pixels)
}

use chrono::Datelike;
