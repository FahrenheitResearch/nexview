use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

/// Severity level of a weather alert
#[derive(Debug, Clone, PartialEq)]
pub enum AlertSeverity {
    Extreme,
    Severe,
    Moderate,
    Minor,
    Unknown,
}

/// A weather alert with polygon geometry from the NWS API
#[derive(Debug, Clone)]
pub struct WeatherAlert {
    pub event: String,
    pub headline: String,
    pub severity: AlertSeverity,
    pub expires: String,
    pub polygon: Vec<(f64, f64)>, // (lat, lon) pairs
}

/// Fetches active weather alerts from the NWS API
pub struct AlertFetcher {
    runtime: Handle,
    client: reqwest::Client,
    alerts: Arc<Mutex<Vec<WeatherAlert>>>,
    last_fetch: Arc<Mutex<Option<Instant>>>,
    fetching: Arc<Mutex<bool>>,
}

impl AlertFetcher {
    const API_URL: &'static str =
        "https://api.weather.gov/alerts/active?status=actual&message_type=alert";
    const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

    pub fn new(runtime: Handle) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("NexView/0.3 (github.com/FahrenheitResearch/nexview)")
            .build()
            .expect("Failed to build HTTP client");

        Self {
            runtime,
            client,
            alerts: Arc::new(Mutex::new(Vec::new())),
            last_fetch: Arc::new(Mutex::new(None)),
            fetching: Arc::new(Mutex::new(false)),
        }
    }

    /// Returns true if more than 60 seconds have passed since the last fetch
    pub fn should_refresh(&self) -> bool {
        let last = self.last_fetch.lock().unwrap();
        match *last {
            None => true,
            Some(t) => t.elapsed() > Self::REFRESH_INTERVAL,
        }
    }

    /// Kick off an async fetch of alerts. Non-blocking.
    pub fn fetch_alerts(&self) {
        {
            let mut fetching = self.fetching.lock().unwrap();
            if *fetching {
                return;
            }
            *fetching = true;
        }

        let client = self.client.clone();
        let alerts_ref = Arc::clone(&self.alerts);
        let last_fetch_ref = Arc::clone(&self.last_fetch);
        let fetching_ref = Arc::clone(&self.fetching);

        self.runtime.spawn(async move {
            let result = Self::do_fetch(&client).await;

            match result {
                Ok(new_alerts) => {
                    log::info!("Fetched {} weather alerts with polygons", new_alerts.len());
                    *alerts_ref.lock().unwrap() = new_alerts;
                    *last_fetch_ref.lock().unwrap() = Some(Instant::now());
                }
                Err(e) => {
                    log::warn!("Failed to fetch weather alerts: {}", e);
                }
            }

            *fetching_ref.lock().unwrap() = false;
        });
    }

    async fn do_fetch(client: &reqwest::Client) -> Result<Vec<WeatherAlert>, String> {
        let resp = client
            .get(Self::API_URL)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("JSON parse error: {}", e))?;

        let features = json
            .get("features")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "Missing features array".to_string())?;

        let mut alerts = Vec::new();

        for feature in features {
            let properties = match feature.get("properties") {
                Some(p) => p,
                None => continue,
            };

            let event = properties
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let headline = properties
                .get("headline")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let severity_str = properties
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let severity = match severity_str {
                "Extreme" => AlertSeverity::Extreme,
                "Severe" => AlertSeverity::Severe,
                "Moderate" => AlertSeverity::Moderate,
                "Minor" => AlertSeverity::Minor,
                _ => AlertSeverity::Unknown,
            };

            let expires = properties
                .get("expires")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Extract polygons from geometry
            let geometry = match feature.get("geometry") {
                Some(g) if !g.is_null() => g,
                _ => continue, // Skip alerts without geometry
            };

            let geo_type = geometry
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let polygons = match geo_type {
                "Polygon" => {
                    // coordinates is [[[lon, lat], ...]]
                    Self::parse_polygon_coords(geometry.get("coordinates"))
                }
                "MultiPolygon" => {
                    // coordinates is [[[[lon, lat], ...]], ...]
                    Self::parse_multi_polygon_coords(geometry.get("coordinates"))
                }
                _ => continue,
            };

            for polygon in polygons {
                if polygon.is_empty() {
                    continue;
                }
                alerts.push(WeatherAlert {
                    event: event.clone(),
                    headline: headline.clone(),
                    severity: severity.clone(),
                    expires: expires.clone(),
                    polygon,
                });
            }
        }

        Ok(alerts)
    }

    /// Parse a Polygon geometry's coordinates: [[[lon, lat], ...]]
    /// Returns a vec with one polygon (vec of (lat, lon) tuples)
    fn parse_polygon_coords(coords: Option<&serde_json::Value>) -> Vec<Vec<(f64, f64)>> {
        let coords = match coords.and_then(|v| v.as_array()) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        for ring in coords {
            if let Some(points) = ring.as_array() {
                let polygon: Vec<(f64, f64)> = points
                    .iter()
                    .filter_map(|pt| {
                        let arr = pt.as_array()?;
                        let lon = arr.first()?.as_f64()?;
                        let lat = arr.get(1)?.as_f64()?;
                        // GeoJSON is [lon, lat], we store as (lat, lon)
                        Some((lat, lon))
                    })
                    .collect();

                if !polygon.is_empty() {
                    result.push(polygon);
                }
            }
        }
        result
    }

    /// Parse a MultiPolygon geometry's coordinates: [[[[lon, lat], ...]], ...]
    fn parse_multi_polygon_coords(coords: Option<&serde_json::Value>) -> Vec<Vec<(f64, f64)>> {
        let coords = match coords.and_then(|v| v.as_array()) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        for polygon_coords in coords {
            // Each element is a Polygon's coordinate array
            let mut sub = Self::parse_polygon_coords(Some(polygon_coords));
            result.append(&mut sub);
        }
        result
    }

    /// Get the current list of alerts (non-blocking)
    pub fn get_alerts(&self) -> Vec<WeatherAlert> {
        self.alerts.lock().unwrap().clone()
    }

    /// Returns true if a fetch is currently in progress
    pub fn is_fetching(&self) -> bool {
        *self.fetching.lock().unwrap()
    }
}
