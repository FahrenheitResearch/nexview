use std::collections::HashMap;
use std::time::Instant;

use crate::nexrad::Level2File;

/// A single cached radar site with parsed data and optional thumbnail
pub struct CachedSite {
    pub station_id: String,
    pub file: Level2File,
    pub fetched_at: Instant,
    /// 256x256 RGBA pre-rendered reflectivity thumbnail
    pub thumbnail_pixels: Option<Vec<u8>>,
    pub stale: bool,
}

/// Thread-safe cache of parsed NEXRAD Level2 data keyed by station ID
pub struct SiteCache {
    cache: HashMap<String, CachedSite>,
}

impl SiteCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Retrieve a cached site by station ID (e.g. "KTLX")
    pub fn get(&self, station_id: &str) -> Option<&CachedSite> {
        self.cache.get(&station_id.to_uppercase())
    }

    /// Insert or replace a cached site
    pub fn insert(&mut self, entry: CachedSite) {
        self.cache.insert(entry.station_id.to_uppercase(), entry);
    }

    /// Check whether a station is already cached
    pub fn has(&self, station_id: &str) -> bool {
        self.cache.contains_key(&station_id.to_uppercase())
    }

    /// Return the list of station IDs that are currently loaded
    pub fn stations_loaded(&self) -> Vec<String> {
        self.cache.keys().cloned().collect()
    }

    /// Mark all entries older than `max_age` as stale
    pub fn mark_stale(&mut self, max_age: std::time::Duration) {
        let now = Instant::now();
        for entry in self.cache.values_mut() {
            if now.duration_since(entry.fetched_at) > max_age {
                entry.stale = true;
            }
        }
    }

    /// Remove all entries that have been marked stale
    pub fn remove_stale(&mut self) {
        self.cache.retain(|_, v| !v.stale);
    }
}

impl Default for SiteCache {
    fn default() -> Self {
        Self::new()
    }
}
