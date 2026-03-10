//! Data Packs — pre-download all NEXRAD files for a historic event
//! so they load instantly without hitting S3 every time.
//!
//! Pack layout:  data_packs/{STATION}_{YYYYMMDD}/*.nexrad

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

/// Status of a data pack download
#[derive(Debug, Clone)]
pub enum PackStatus {
    /// Not downloaded
    None,
    /// Currently downloading
    Downloading { done: usize, total: usize },
    /// Finished downloading
    Ready(usize), // number of files
    /// Error
    Error(String),
}

/// Manages data pack storage
pub struct DataPackManager {
    pack_dir: PathBuf,
    runtime: Handle,
    http: Arc<reqwest::Client>,
    pub download_status: Arc<Mutex<PackStatus>>,
}

impl DataPackManager {
    pub fn new(runtime: Handle) -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let pack_dir = exe_dir.join("data_packs");

        let http = reqwest::Client::builder()
            .user_agent("NexView/1.5 Weather Radar Viewer")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            pack_dir,
            runtime,
            http: Arc::new(http),
            download_status: Arc::new(Mutex::new(PackStatus::None)),
        }
    }

    /// Get the directory for a specific event pack
    fn pack_path(&self, station: &str, year: i32, month: u32, day: u32) -> PathBuf {
        self.pack_dir.join(format!("{}_{:04}{:02}{:02}", station, year, month, day))
    }

    /// Check if a pack is already downloaded, return file count
    pub fn pack_exists(&self, station: &str, year: i32, month: u32, day: u32) -> Option<usize> {
        let dir = self.pack_path(station, year, month, day);
        if !dir.exists() {
            return None;
        }
        let count = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().and_then(|ext| ext.to_str()) == Some("nexrad")
            })
            .count();
        if count > 0 { Some(count) } else { None }
    }

    /// Load all files from a pack as raw bytes, sorted by filename
    pub fn load_pack(&self, station: &str, year: i32, month: u32, day: u32) -> Option<Vec<(String, Vec<u8>)>> {
        let dir = self.pack_path(station, year, month, day);
        if !dir.exists() {
            return None;
        }

        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("nexrad"))
            .collect();

        entries.sort();

        let files: Vec<(String, Vec<u8>)> = entries.iter()
            .filter_map(|p| {
                let name = p.file_stem()?.to_string_lossy().to_string();
                let data = std::fs::read(p).ok()?;
                Some((name, data))
            })
            .collect();

        if files.is_empty() { None } else { Some(files) }
    }

    /// Download all NEXRAD files for a station/date and save as a pack
    pub fn download_pack(&self, station: &str, year: i32, month: u32, day: u32) {
        let station = station.to_uppercase();
        let dir = self.pack_path(&station, year, month, day);
        let http = Arc::clone(&self.http);
        let status = Arc::clone(&self.download_status);

        *status.lock().unwrap() = PackStatus::Downloading { done: 0, total: 0 };

        let base_url = "https://unidata-nexrad-level2.s3.amazonaws.com";

        self.runtime.spawn(async move {
            // List files
            let prefix = format!("{:04}/{:02}/{:02}/{}/", year, month, day, station);
            let list_url = format!("{}?list-type=2&prefix={}", base_url, prefix);

            let resp = match http.get(&list_url).send().await {
                Ok(r) => match r.text().await {
                    Ok(t) => t,
                    Err(e) => {
                        *status.lock().unwrap() = PackStatus::Error(format!("List error: {}", e));
                        return;
                    }
                },
                Err(e) => {
                    *status.lock().unwrap() = PackStatus::Error(format!("List error: {}", e));
                    return;
                }
            };

            // Parse keys
            let mut keys: Vec<String> = Vec::new();
            for chunk in resp.split("<Key>").skip(1) {
                if let Some(end) = chunk.find("</Key>") {
                    let key = &chunk[..end];
                    let name = key.rsplit('/').next().unwrap_or(key);
                    if !name.ends_with("_MDM") && !name.ends_with(".md") && !key.is_empty() {
                        keys.push(key.to_string());
                    }
                }
            }
            keys.sort();

            if keys.is_empty() {
                *status.lock().unwrap() = PackStatus::Error("No files found for this date".into());
                return;
            }

            let total = keys.len();
            *status.lock().unwrap() = PackStatus::Downloading { done: 0, total };
            log::info!("Data pack: downloading {} files for {}_{:04}{:02}{:02}",
                total, station, year, month, day);

            // Create directory
            if let Err(e) = std::fs::create_dir_all(&dir) {
                *status.lock().unwrap() = PackStatus::Error(format!("mkdir error: {}", e));
                return;
            }

            // Download with concurrency limit
            use tokio::sync::Semaphore;
            let semaphore = Arc::new(Semaphore::new(6));
            let done_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let error_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

            let mut handles = Vec::new();

            for key in &keys {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let http = Arc::clone(&http);
                let status = Arc::clone(&status);
                let done_count = Arc::clone(&done_count);
                let error_count = Arc::clone(&error_count);
                let key = key.clone();
                let dir = dir.clone();

                let handle = tokio::spawn(async move {
                    let filename = key.rsplit('/').next().unwrap_or(&key).to_string();
                    let file_path = dir.join(format!("{}.nexrad", filename));

                    // Skip if already exists
                    if file_path.exists() {
                        let done = done_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        *status.lock().unwrap() = PackStatus::Downloading { done, total };
                        drop(permit);
                        return;
                    }

                    let url = format!("{}/{}", "https://unidata-nexrad-level2.s3.amazonaws.com", key);
                    match http.get(&url).send().await {
                        Ok(resp) => {
                            match resp.bytes().await {
                                Ok(bytes) => {
                                    if let Err(e) = std::fs::write(&file_path, &bytes) {
                                        log::error!("Failed to write {}: {}", filename, e);
                                        error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to read body for {}: {}", filename, e);
                                    error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to download {}: {}", filename, e);
                            error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }

                    let done = done_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    *status.lock().unwrap() = PackStatus::Downloading { done, total };
                    drop(permit);
                });
                handles.push(handle);
            }

            for h in handles {
                let _ = h.await;
            }

            let errors = error_count.load(std::sync::atomic::Ordering::Relaxed);
            let downloaded = total - errors;
            log::info!("Data pack complete: {}/{} files saved to {:?}", downloaded, total, dir);
            *status.lock().unwrap() = PackStatus::Ready(downloaded);
        });
    }

    /// Delete a pack
    pub fn delete_pack(&self, station: &str, year: i32, month: u32, day: u32) -> bool {
        let dir = self.pack_path(station, year, month, day);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).is_ok()
        } else {
            false
        }
    }
}
