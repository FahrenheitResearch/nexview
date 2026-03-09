use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TileProvider {
    OpenStreetMap,
    Dark,
    Topographic,
    Satellite,
}

impl TileProvider {
    pub fn label(&self) -> &str {
        match self {
            Self::OpenStreetMap => "Standard",
            Self::Dark => "Dark",
            Self::Topographic => "Topographic",
            Self::Satellite => "Satellite",
        }
    }

    pub fn all() -> &'static [TileProvider] {
        &[Self::OpenStreetMap, Self::Dark, Self::Topographic, Self::Satellite]
    }

    fn url(&self, z: u8, x: u32, y: u32) -> String {
        match self {
            Self::OpenStreetMap => format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png"),
            Self::Dark => format!("https://basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png"),
            Self::Topographic => format!("https://tile.opentopomap.org/{z}/{x}/{y}.png"),
            Self::Satellite => format!("https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"),
        }
    }
}

/// Manages downloading and caching of map tiles
pub struct MapTileManager {
    cache: Arc<Mutex<HashMap<TileKey, TileData>>>,
    pending: Arc<Mutex<Vec<TileKey>>>,
    runtime: tokio::runtime::Handle,
    provider: Arc<Mutex<TileProvider>>,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct TileKey {
    pub x: u32,
    pub y: u32,
    pub z: u8,
}

#[derive(Clone)]
pub struct TileData {
    pub pixels: Vec<u8>, // RGBA
    pub width: u32,
    pub height: u32,
}

/// Map view state
#[derive(Debug, Clone)]
pub struct MapView {
    pub center_lat: f64,
    pub center_lon: f64,
    pub zoom: f64,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            center_lat: 38.0,
            center_lon: -97.0,
            zoom: 7.0,
        }
    }
}

impl MapView {
    /// Convert lat/lon to pixel coordinates at the current zoom
    pub fn lat_lon_to_pixel(&self, lat: f64, lon: f64, screen_w: f64, screen_h: f64) -> (f64, f64) {
        let scale = 256.0 * 2.0f64.powf(self.zoom);

        let center_x = (self.center_lon + 180.0) / 360.0 * scale;
        let center_y_rad = self.center_lat.to_radians();
        let center_y = (1.0 - center_y_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * scale;

        let x = (lon + 180.0) / 360.0 * scale;
        let y_rad = lat.to_radians();
        let y = (1.0 - y_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * scale;

        let px = (x - center_x) + screen_w / 2.0;
        let py = (y - center_y) + screen_h / 2.0;

        (px, py)
    }

    /// Convert pixel coordinates to lat/lon
    pub fn pixel_to_lat_lon(&self, px: f64, py: f64, screen_w: f64, screen_h: f64) -> (f64, f64) {
        let scale = 256.0 * 2.0f64.powf(self.zoom);

        let center_x = (self.center_lon + 180.0) / 360.0 * scale;
        let center_y_rad = self.center_lat.to_radians();
        let center_y = (1.0 - center_y_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * scale;

        let x = center_x + (px - screen_w / 2.0);
        let y = center_y + (py - screen_h / 2.0);

        let lon = x / scale * 360.0 - 180.0;
        let lat_rad = (std::f64::consts::PI * (1.0 - 2.0 * y / scale)).sinh().atan();
        let lat = lat_rad.to_degrees();

        (lat, lon)
    }

    /// Get tile coordinates visible in the current view
    pub fn visible_tiles(&self, screen_w: f64, screen_h: f64) -> Vec<TileKey> {
        let z = self.zoom.floor() as u8;
        let n = 2u32.pow(z as u32);
        let scale = 256.0 * n as f64;

        let center_x = (self.center_lon + 180.0) / 360.0 * scale;
        let center_y_rad = self.center_lat.to_radians();
        let center_y = (1.0 - center_y_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * scale;

        let left = center_x - screen_w / 2.0;
        let right = center_x + screen_w / 2.0;
        let top = center_y - screen_h / 2.0;
        let bottom = center_y + screen_h / 2.0;

        let tile_x_start = (left / 256.0).floor().max(0.0) as u32;
        let tile_x_end = (right / 256.0).ceil().min(n as f64) as u32;
        let tile_y_start = (top / 256.0).floor().max(0.0) as u32;
        let tile_y_end = (bottom / 256.0).ceil().min(n as f64) as u32;

        let mut tiles = Vec::new();
        for y in tile_y_start..tile_y_end {
            for x in tile_x_start..tile_x_end {
                tiles.push(TileKey { x, y, z });
            }
        }
        tiles
    }

    /// Get tiles for a slightly expanded viewport (1 tile extra in each direction)
    /// plus tiles at one zoom level up (zoom - 1) covering the current view.
    /// Used for prefetching to make panning and zooming out smoother.
    pub fn prefetch_tiles(&self, screen_w: f64, screen_h: f64) -> Vec<TileKey> {
        let mut tiles = Vec::new();

        // 1) Expanded viewport at current zoom: 1 extra tile in each direction
        let z = self.zoom.floor() as u8;
        let n = 2u32.pow(z as u32);
        let scale = 256.0 * n as f64;

        let center_x = (self.center_lon + 180.0) / 360.0 * scale;
        let center_y_rad = self.center_lat.to_radians();
        let center_y = (1.0 - center_y_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * scale;

        let left = center_x - screen_w / 2.0 - 256.0;
        let right = center_x + screen_w / 2.0 + 256.0;
        let top = center_y - screen_h / 2.0 - 256.0;
        let bottom = center_y + screen_h / 2.0 + 256.0;

        let tile_x_start = (left / 256.0).floor().max(0.0) as u32;
        let tile_x_end = (right / 256.0).ceil().min(n as f64) as u32;
        let tile_y_start = (top / 256.0).floor().max(0.0) as u32;
        let tile_y_end = (bottom / 256.0).ceil().min(n as f64) as u32;

        // Collect the visible tile set so we can exclude them (they're already requested)
        let visible_set: std::collections::HashSet<(u32, u32, u8)> = self
            .visible_tiles(screen_w, screen_h)
            .iter()
            .map(|k| (k.x, k.y, k.z))
            .collect();

        for y in tile_y_start..tile_y_end {
            for x in tile_x_start..tile_x_end {
                if !visible_set.contains(&(x, y, z)) {
                    tiles.push(TileKey { x, y, z });
                }
            }
        }

        // 2) One zoom level up (zoom - 1) covering the current view area
        if z > 2 {
            let z_up = z - 1;
            let n_up = 2u32.pow(z_up as u32);
            let scale_up = 256.0 * n_up as f64;

            let cx_up = (self.center_lon + 180.0) / 360.0 * scale_up;
            let cy_rad = self.center_lat.to_radians();
            let cy_up = (1.0 - cy_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * scale_up;

            // Map current screen extent to the zoom-1 coordinate space
            let ratio = scale_up / scale;
            let left_up = cx_up - (screen_w / 2.0) * ratio;
            let right_up = cx_up + (screen_w / 2.0) * ratio;
            let top_up = cy_up - (screen_h / 2.0) * ratio;
            let bottom_up = cy_up + (screen_h / 2.0) * ratio;

            let tx_start = (left_up / 256.0).floor().max(0.0) as u32;
            let tx_end = (right_up / 256.0).ceil().min(n_up as f64) as u32;
            let ty_start = (top_up / 256.0).floor().max(0.0) as u32;
            let ty_end = (bottom_up / 256.0).ceil().min(n_up as f64) as u32;

            for y in ty_start..ty_end {
                for x in tx_start..tx_end {
                    tiles.push(TileKey { x, y, z: z_up });
                }
            }
        }

        tiles
    }

    /// Get the pixel position of a tile's top-left corner on screen
    pub fn tile_screen_pos(&self, key: &TileKey, screen_w: f64, screen_h: f64) -> (f64, f64) {
        let scale = 256.0 * 2.0f64.powf(self.zoom);
        let z_scale = 256.0 * 2u32.pow(key.z as u32) as f64;

        let center_x = (self.center_lon + 180.0) / 360.0 * scale;
        let center_y_rad = self.center_lat.to_radians();
        let center_y = (1.0 - center_y_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * scale;

        let tile_x = key.x as f64 * 256.0 * (scale / z_scale);
        let tile_y = key.y as f64 * 256.0 * (scale / z_scale);

        let px = tile_x - center_x + screen_w / 2.0;
        let py = tile_y - center_y + screen_h / 2.0;

        (px, py)
    }

    pub fn tile_size_on_screen(&self, z: u8) -> f64 {
        let ratio = 2.0f64.powf(self.zoom - z as f64);
        256.0 * ratio
    }

    /// Zoom in/out centered on a point
    pub fn zoom_at(&mut self, delta: f64, screen_x: f64, screen_y: f64, screen_w: f64, screen_h: f64) {
        let (lat, lon) = self.pixel_to_lat_lon(screen_x, screen_y, screen_w, screen_h);
        self.zoom = (self.zoom + delta).clamp(2.0, 18.0);
        // Re-center so the point under cursor stays put
        let (new_px, new_py) = self.lat_lon_to_pixel(lat, lon, screen_w, screen_h);
        let (cur_cx, cur_cy) = self.lat_lon_to_pixel(self.center_lat, self.center_lon, screen_w, screen_h);
        let offset_x = new_px - screen_x;
        let offset_y = new_py - screen_y;
        let (new_lat, new_lon) = self.pixel_to_lat_lon(cur_cx + offset_x, cur_cy + offset_y, screen_w, screen_h);
        self.center_lat = new_lat;
        self.center_lon = new_lon;
    }

    /// Pan the map by pixel delta
    pub fn pan(&mut self, dx: f64, dy: f64, screen_w: f64, screen_h: f64) {
        let (new_lat, new_lon) = self.pixel_to_lat_lon(
            screen_w / 2.0 - dx,
            screen_h / 2.0 - dy,
            screen_w,
            screen_h,
        );
        self.center_lat = new_lat;
        self.center_lon = new_lon;
    }
}

impl MapTileManager {
    pub fn new(runtime: tokio::runtime::Handle) -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(Vec::new())),
            runtime,
            provider: Arc::new(Mutex::new(TileProvider::OpenStreetMap)),
        }
    }

    pub fn provider(&self) -> TileProvider {
        *self.provider.lock().unwrap()
    }

    pub fn set_provider(&self, provider: TileProvider) {
        let mut current = self.provider.lock().unwrap();
        if *current != provider {
            *current = provider;
            // Clear cache — new provider means new tiles
            self.cache.lock().unwrap().clear();
            self.pending.lock().unwrap().clear();
        }
    }

    pub fn get_tile(&self, key: &TileKey) -> Option<TileData> {
        let cache = self.cache.lock().ok()?;
        cache.get(key).cloned()
    }

    pub fn request_tile(&self, key: TileKey) {
        // Check if already cached or pending
        {
            let cache = self.cache.lock().unwrap();
            if cache.contains_key(&key) {
                return;
            }
        }
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.contains(&key) {
                return;
            }
            pending.push(key);
        }

        let cache = Arc::clone(&self.cache);
        let pending = Arc::clone(&self.pending);
        let provider = *self.provider.lock().unwrap();

        self.runtime.spawn(async move {
            let url = provider.url(key.z, key.x, key.y);

            let client = reqwest::Client::builder()
                .user_agent("NexView/0.1 Weather Radar Viewer")
                .build();

            let client = match client {
                Ok(c) => c,
                Err(_) => {
                    pending.lock().unwrap().retain(|k| k != &key);
                    return;
                }
            };

            match client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(bytes) = resp.bytes().await {
                        if let Ok(img) = image::load_from_memory(&bytes) {
                            let rgba = img.to_rgba8();
                            let tile = TileData {
                                width: rgba.width(),
                                height: rgba.height(),
                                pixels: rgba.into_raw(),
                            };
                            cache.lock().unwrap().insert(key, tile);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to fetch tile {}/{}/{}: {}", key.z, key.x, key.y, e);
                }
            }

            pending.lock().unwrap().retain(|k| k != &key);
        });
    }

    pub fn cache_size(&self) -> usize {
        self.cache.lock().map(|c| c.len()).unwrap_or(0)
    }
}
