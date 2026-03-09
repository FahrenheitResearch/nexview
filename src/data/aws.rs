use chrono::{Datelike, Utc, NaiveDate};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

const NEXRAD_BASE_URL: &str = "https://unidata-nexrad-level2.s3.amazonaws.com";

/// Fetches NEXRAD Level 2 data from AWS S3 (public, no auth needed)
pub struct NexradFetcher {
    http: Arc<reqwest::Client>,
    runtime: Handle,
    pub available_files: Arc<Mutex<Vec<NexradFileInfo>>>,
    pub fetching: Arc<Mutex<bool>>,
    pub downloaded_data: Arc<Mutex<Option<Vec<u8>>>>,
    pub download_progress: Arc<Mutex<Option<String>>>,
    pub download_duration: Arc<Mutex<Option<Duration>>>,
}

#[derive(Debug, Clone)]
pub struct NexradFileInfo {
    pub key: String,
    pub size: i64,
    pub last_modified: String,
    pub display_name: String,
}

impl NexradFetcher {
    pub fn new(runtime: Handle) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("NexView/0.1 Weather Radar Viewer")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http: Arc::new(http),
            runtime,
            available_files: Arc::new(Mutex::new(Vec::new())),
            fetching: Arc::new(Mutex::new(false)),
            downloaded_data: Arc::new(Mutex::new(None)),
            download_progress: Arc::new(Mutex::new(None)),
            download_duration: Arc::new(Mutex::new(None)),
        }
    }

    /// List available radar files for a given station and date
    pub fn list_files(&self, station: &str, date: NaiveDate) {
        let station = station.to_uppercase();

        // S3 prefix format: YYYY/MM/DD/KXXX/
        let prefix = format!(
            "{:04}/{:02}/{:02}/{}/",
            date.year(),
            date.month(),
            date.day(),
            station
        );

        let http = Arc::clone(&self.http);
        let files = Arc::clone(&self.available_files);
        let fetching = Arc::clone(&self.fetching);

        *fetching.lock().unwrap() = true;

        self.runtime.spawn(async move {
            log::info!("Listing NEXRAD files with prefix: {}", prefix);

            // Use S3 REST API directly (no auth needed for public bucket)
            let url = format!(
                "{}?list-type=2&prefix={}",
                NEXRAD_BASE_URL, prefix
            );

            let mut all_files = Vec::new();

            match http.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(body) = resp.text().await {
                        // Parse the XML response
                        all_files = Self::parse_s3_list_xml(&body);
                    }
                }
                Err(e) => {
                    log::error!("Failed to list NEXRAD files: {}", e);
                }
            }

            all_files.sort_by(|a, b| a.key.cmp(&b.key));
            *files.lock().unwrap() = all_files;
            *fetching.lock().unwrap() = false;
        });
    }

    /// Parse S3 ListObjectsV2 XML response
    fn parse_s3_list_xml(xml: &str) -> Vec<NexradFileInfo> {
        let mut files = Vec::new();

        // Simple XML parsing - extract <Contents> elements
        for contents in xml.split("<Contents>").skip(1) {
            let end = contents.find("</Contents>").unwrap_or(contents.len());
            let block = &contents[..end];

            let key = Self::extract_xml_tag(block, "Key").unwrap_or_default();
            let size_str = Self::extract_xml_tag(block, "Size").unwrap_or_default();
            let modified = Self::extract_xml_tag(block, "LastModified").unwrap_or_default();

            let size: i64 = size_str.parse().unwrap_or(0);
            let display = key.rsplit('/').next().unwrap_or(&key).to_string();

            // Skip MDM files (metadata) and empty keys
            if key.is_empty() || display.ends_with("_MDM") || display.ends_with(".md") {
                continue;
            }

            files.push(NexradFileInfo {
                key,
                size,
                last_modified: modified,
                display_name: display,
            });
        }

        files
    }

    fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        let start = xml.find(&open)? + open.len();
        let end = xml.find(&close)?;
        Some(xml[start..end].to_string())
    }

    /// List recent files and auto-download the latest one.
    /// Tries today first, falls back to yesterday if no files found.
    pub fn list_recent_files(&self, station: &str) {
        let station = station.to_uppercase();
        let today = Utc::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let http = Arc::clone(&self.http);
        let files = Arc::clone(&self.available_files);
        let fetching = Arc::clone(&self.fetching);
        let data = Arc::clone(&self.downloaded_data);
        let progress = Arc::clone(&self.download_progress);
        let dl_duration = Arc::clone(&self.download_duration);

        *fetching.lock().unwrap() = true;

        self.runtime.spawn(async move {
            let mut all_files = Vec::new();

            // Try today first
            for date in &[today, yesterday] {
                let prefix = format!(
                    "{:04}/{:02}/{:02}/{}/",
                    date.year(), date.month(), date.day(), station
                );
                let url = format!("{}?list-type=2&prefix={}", NEXRAD_BASE_URL, prefix);
                log::info!("Listing NEXRAD files: {}", url);

                match http.get(&url).send().await {
                    Ok(resp) => {
                        if let Ok(body) = resp.text().await {
                            all_files = Self::parse_s3_list_xml(&body);
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to list NEXRAD files: {}", e);
                    }
                }

                if !all_files.is_empty() {
                    break; // Found files, no need to try yesterday
                }
                log::info!("No files for {:?}, trying previous day", date);
            }

            all_files.sort_by(|a, b| a.key.cmp(&b.key));

            // Auto-download the latest (last) file
            if let Some(latest) = all_files.last() {
                let key = latest.key.clone();
                let display = latest.display_name.clone();
                log::info!("Auto-downloading latest: {}", key);
                *progress.lock().unwrap() = Some(format!("Downloading {}...", display));

                let dl_start = Instant::now();
                let url = format!("{}/{}", NEXRAD_BASE_URL, key);
                match http.get(&url).send().await {
                    Ok(resp) => {
                        match resp.bytes().await {
                            Ok(bytes) => {
                                let elapsed = dl_start.elapsed();
                                let raw = bytes.to_vec();
                                let mb_s = raw.len() as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64();
                                log::info!("Downloaded {} bytes in {:.0}ms ({:.1} MB/s)", raw.len(), elapsed.as_secs_f64() * 1000.0, mb_s);
                                *data.lock().unwrap() = Some(raw);
                                *progress.lock().unwrap() = None;
                                *dl_duration.lock().unwrap() = Some(elapsed);
                            }
                            Err(e) => {
                                log::error!("Failed to read body: {}", e);
                                *progress.lock().unwrap() = Some(format!("Error: {}", e));
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to download: {}", e);
                        *progress.lock().unwrap() = Some(format!("Error: {}", e));
                    }
                }
            } else {
                log::warn!("No NEXRAD files found for {}", station);
                *progress.lock().unwrap() = Some("No files found".into());
            }

            *files.lock().unwrap() = all_files;
            *fetching.lock().unwrap() = false;
        });
    }

    /// Download a specific NEXRAD file
    pub fn download_file(&self, key: &str) {
        let http = Arc::clone(&self.http);
        let data = Arc::clone(&self.downloaded_data);
        let progress = Arc::clone(&self.download_progress);
        let dl_duration = Arc::clone(&self.download_duration);
        let key = key.to_string();

        let display_name = key.rsplit('/').next().unwrap_or(&key).to_string();
        *progress.lock().unwrap() = Some(format!("Downloading {}...", display_name));

        self.runtime.spawn(async move {
            let dl_start = Instant::now();
            let url = format!("{}/{}", NEXRAD_BASE_URL, key);

            match http.get(&url).send().await {
                Ok(resp) => {
                    match resp.bytes().await {
                        Ok(bytes) => {
                            let elapsed = dl_start.elapsed();
                            let raw = bytes.to_vec();
                            let mb_s = raw.len() as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64();
                            log::info!("Downloaded {} bytes in {:.0}ms ({:.1} MB/s)", raw.len(), elapsed.as_secs_f64() * 1000.0, mb_s);
                            *data.lock().unwrap() = Some(raw);
                            *progress.lock().unwrap() = None;
                            *dl_duration.lock().unwrap() = Some(elapsed);
                        }
                        Err(e) => {
                            log::error!("Failed to read body: {}", e);
                            *progress.lock().unwrap() = Some(format!("Error: {}", e));
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to download {}: {}", key, e);
                    *progress.lock().unwrap() = Some(format!("Error: {}", e));
                }
            }
        });
    }

    pub fn is_fetching(&self) -> bool {
        *self.fetching.lock().unwrap()
    }

    pub fn take_downloaded_data(&self) -> Option<Vec<u8>> {
        self.downloaded_data.lock().unwrap().take()
    }

    pub fn get_progress(&self) -> Option<String> {
        self.download_progress.lock().unwrap().clone()
    }

    pub fn take_download_time(&self) -> Option<Duration> {
        self.download_duration.lock().unwrap().take()
    }
}
