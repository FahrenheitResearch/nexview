//! Automatic nationwide tornado risk scanning.
//!
//! Two-phase approach for efficiency:
//! 1. Download latest scan from each NEXRAD site, run cheap meso/TVS detection
//! 2. Only run expensive ML inference on radars with significant rotation
//!
//! This makes scanning all ~160 radars feasible in ~1-2 minutes.

use chrono::Datelike;
use crate::nexrad::{Level2File, sites, detection::RotationDetector};
use crate::predict::convert::{RadarSequence, TornadoPrediction};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

const NEXRAD_BASE_URL: &str = "https://unidata-nexrad-level2.s3.amazonaws.com";

/// A single scan result from one radar/storm cell.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub station: String,
    pub station_lat: f64,
    pub station_lon: f64,
    pub storm_lat: f64,
    pub storm_lon: f64,
    pub meso_count: usize,
    pub tvs_count: usize,
    pub max_shear: f32,
    /// ML prediction (only if model available and rotation detected)
    pub prediction: Option<TornadoPrediction>,
    pub timestamp: Instant,
}

impl ScanResult {
    /// Primary sort score: ML prediction if available, otherwise shear-based heuristic
    pub fn risk_score(&self) -> f32 {
        if let Some(ref p) = self.prediction {
            p.detection_prob.max(p.prediction_prob)
        } else {
            // Heuristic: TVS > strong meso > moderate meso
            let tvs_score = self.tvs_count as f32 * 0.7;
            let meso_score = self.meso_count as f32 * 0.3;
            (tvs_score + meso_score + self.max_shear * 5.0).min(1.0)
        }
    }

    pub fn risk_level(&self) -> &'static str {
        if let Some(ref p) = self.prediction {
            p.risk_level()
        } else {
            let s = self.risk_score();
            if s >= 0.9 { "EXTREME" }
            else if s >= 0.7 { "HIGH" }
            else if s >= 0.5 { "MODERATE" }
            else if s >= 0.3 { "LOW" }
            else { "MINIMAL" }
        }
    }
}

/// Manages background scanning of all NEXRAD sites.
pub struct AutoScanManager {
    runtime: Handle,
    /// Top results, sorted by risk score descending
    pub results: Arc<Mutex<Vec<ScanResult>>>,
    /// Whether a scan is currently running
    pub scanning: Arc<AtomicBool>,
    /// Progress: how many radars scanned so far
    pub radars_scanned: Arc<AtomicUsize>,
    /// Total radars to scan
    pub radars_total: Arc<AtomicUsize>,
    /// Whether auto-scan is enabled
    pub active: bool,
    /// Scan interval
    pub scan_interval: Duration,
    /// Last scan completion time
    pub last_scan: Option<Instant>,
    /// Whether to run ML inference (requires model)
    pub run_inference: bool,
    /// Path to ONNX model (if found)
    #[cfg(feature = "tornado-predict")]
    pub model_path: Option<std::path::PathBuf>,
}

impl AutoScanManager {
    pub fn new(runtime: Handle) -> Self {
        #[cfg(feature = "tornado-predict")]
        let model_path = crate::predict::TornadoPredictor::find_model();

        Self {
            runtime,
            results: Arc::new(Mutex::new(Vec::new())),
            scanning: Arc::new(AtomicBool::new(false)),
            radars_scanned: Arc::new(AtomicUsize::new(0)),
            radars_total: Arc::new(AtomicUsize::new(0)),
            active: false,
            scan_interval: Duration::from_secs(300), // 5 minutes
            last_scan: None,
            run_inference: true,
            #[cfg(feature = "tornado-predict")]
            model_path,
        }
    }

    /// Check if it's time to run a scan and kick one off.
    pub fn update(&mut self) {
        if !self.active {
            return;
        }
        if self.scanning.load(Ordering::Relaxed) {
            return;
        }
        let should_scan = match self.last_scan {
            None => true,
            Some(t) => t.elapsed() >= self.scan_interval,
        };
        if should_scan {
            self.start_scan();
        }
    }

    /// Get top N results.
    pub fn top_results(&self, n: usize) -> Vec<ScanResult> {
        let results = self.results.lock().unwrap();
        results.iter().take(n).cloned().collect()
    }

    /// Kick off a background scan of all NEXRAD sites.
    pub fn start_scan(&mut self) {
        if self.scanning.load(Ordering::Relaxed) {
            return;
        }

        let all_sites: Vec<_> = sites::RADAR_SITES.iter().collect();
        let total = all_sites.len();

        self.scanning.store(true, Ordering::Relaxed);
        self.radars_scanned.store(0, Ordering::Relaxed);
        self.radars_total.store(total, Ordering::Relaxed);

        let scanning = Arc::clone(&self.scanning);
        let scanned = Arc::clone(&self.radars_scanned);
        let results_arc = Arc::clone(&self.results);
        let run_inference = self.run_inference;

        #[cfg(feature = "tornado-predict")]
        let model_path = self.model_path.clone();

        self.runtime.spawn(async move {
            let http = reqwest::Client::builder()
                .user_agent("NexView/autoscan")
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap();

            let semaphore = Arc::new(tokio::sync::Semaphore::new(12));
            let scan_results: Arc<Mutex<Vec<ScanResult>>> = Arc::new(Mutex::new(Vec::new()));

            let today = chrono::Utc::now();
            let date_prefix = format!("{:04}/{:02}/{:02}", today.year(), today.month(), today.day());

            let mut handles = Vec::new();

            for site in &all_sites {
                let sem = Arc::clone(&semaphore);
                let http = http.clone();
                let date_prefix = date_prefix.clone();
                let station = site.id.to_string();
                let site_lat = site.lat;
                let site_lon = site.lon;
                let scanned = Arc::clone(&scanned);
                let scan_results = Arc::clone(&scan_results);

                #[cfg(feature = "tornado-predict")]
                let model_path = model_path.clone();

                let handle = tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();

                    let result = scan_one_station(
                        &http, &station, site_lat, site_lon, &date_prefix,
                        run_inference,
                        #[cfg(feature = "tornado-predict")]
                        &model_path,
                    ).await;

                    scanned.fetch_add(1, Ordering::Relaxed);

                    if let Some(r) = result {
                        scan_results.lock().unwrap().push(r);
                    }
                });
                handles.push(handle);
            }

            // Wait for all to complete
            for h in handles {
                let _ = h.await;
            }

            // Sort by risk score descending
            {
                let mut results = scan_results.lock().unwrap();
                results.sort_by(|a, b| {
                    b.risk_score().partial_cmp(&a.risk_score()).unwrap_or(std::cmp::Ordering::Equal)
                });
            }

            // Move to shared results
            let final_results = scan_results.lock().unwrap().clone();
            *results_arc.lock().unwrap() = final_results;

            scanning.store(false, Ordering::Relaxed);
            log::info!("Auto-scan complete: {} stations scanned", scanned.load(Ordering::Relaxed));
        });

        self.last_scan = Some(Instant::now());
    }
}

/// Scan a single station: download latest file, detect rotation, optionally run ML.
async fn scan_one_station(
    http: &reqwest::Client,
    station: &str,
    site_lat: f64,
    site_lon: f64,
    date_prefix: &str,
    run_inference: bool,
    #[cfg(feature = "tornado-predict")]
    model_path: &Option<std::path::PathBuf>,
) -> Option<ScanResult> {
    // List files for this station today
    let prefix = format!("{}/{}/", date_prefix, station);
    let url = format!("{}?list-type=2&prefix={}", NEXRAD_BASE_URL, prefix);

    let resp = http.get(&url).send().await.ok()?.text().await.ok()?;

    // Parse S3 XML for keys
    let mut keys: Vec<String> = Vec::new();
    for chunk in resp.split("<Key>").skip(1) {
        if let Some(end) = chunk.find("</Key>") {
            let key = &chunk[..end];
            let name = key.rsplit('/').next().unwrap_or(key);
            if !name.ends_with("_MDM") && !name.contains("NXL2") {
                keys.push(key.to_string());
            }
        }
    }

    if keys.is_empty() {
        return None;
    }

    // Download the latest file
    let latest_key = keys.last()?;
    let file_url = format!("{}/{}", NEXRAD_BASE_URL, latest_key);
    let file_bytes = http.get(&file_url).send().await.ok()?.bytes().await.ok()?;

    // Decompress gzip if needed
    let raw = file_bytes.to_vec();
    let data = if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        use std::io::Read;
        let mut decoder = flate2::read::GzDecoder::new(&raw[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).ok()?;
        decompressed
    } else {
        raw
    };

    let file = Level2File::parse(&data).ok()?;
    if file.sweeps.is_empty() {
        return None;
    }

    let site = sites::find_site(station)?;
    let (mesos, tvs) = RotationDetector::detect(&file, site);

    // Only report if there's any rotation
    if mesos.is_empty() && tvs.is_empty() {
        return None;
    }

    let max_shear = mesos.iter().map(|m| m.max_shear).fold(0.0f32, f32::max);

    // Use strongest rotation point as storm center
    let (storm_lat, storm_lon) = if let Some(t) = tvs.first() {
        (t.lat, t.lon)
    } else if let Some(m) = mesos.first() {
        (m.lat, m.lon)
    } else {
        return None;
    };

    let mut result = ScanResult {
        station: station.to_string(),
        station_lat: site_lat,
        station_lon: site_lon,
        storm_lat,
        storm_lon,
        meso_count: mesos.len(),
        tvs_count: tvs.len(),
        max_shear,
        prediction: None,
        timestamp: Instant::now(),
    };

    // Phase 2: ML inference if available and requested
    #[cfg(feature = "tornado-predict")]
    if run_inference {
        if let Some(ref mp) = model_path {
            // Only run ML on significant rotation (TVS or strong meso)
            let has_significant = !tvs.is_empty() || mesos.iter().any(|m| {
                matches!(m.strength, crate::nexrad::detection::RotationStrength::Strong |
                         crate::nexrad::detection::RotationStrength::Moderate)
            });

            if has_significant {
                // Single frame is enough for a risk estimate
                let files = vec![file];
                if let Some(seq) = RadarSequence::from_files(&files, storm_lat, storm_lon, site) {
                    if let Ok(mut predictor) = crate::predict::TornadoPredictor::load(mp) {
                        if let Ok(pred) = predictor.predict(&seq) {
                            result.prediction = Some(pred);
                        }
                    }
                }
            }
        }
    }

    Some(result)
}
