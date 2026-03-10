use eframe::egui;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use chrono::{Datelike, NaiveDate};
use crate::nexrad::{Level2File, RadarProduct, sites};
use crate::render::{RadarRenderer, MapTileManager, GpuRadarRenderer, ColorTableManager};
use crate::render::map::{MapView, TileProvider};
use crate::data::{NexradFetcher, AlertFetcher, SoundingFetcher, DataPackManager, PackStatus};
use crate::ui::{SidePanel, ControlBar};
use crate::ui::toolbar::Toolbar;
use crate::ui::timeline::TimelineBar;
use crate::ui::sidebar::CollapsibleSidebar;
use crate::ui::hover_preview::HoverPreview;
use crate::ui::national_view::NationalView;
use crate::ui::minimap::Minimap;

const SETTINGS_PATH: &str = "nexview_settings.json";

/// Quad-view panel products
pub const QUAD_PRODUCTS: [RadarProduct; 4] = [
    RadarProduct::Reflectivity,
    RadarProduct::Velocity,
    RadarProduct::DifferentialReflectivity,
    RadarProduct::CorrelationCoefficient,
];

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AppSettings {
    pub default_station: String,
    pub default_zoom: f64,
    pub quad_view: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_station: "KTLX".into(),
            default_zoom: 7.0,
            quad_view: false,
        }
    }
}

impl AppSettings {
    fn load() -> Self {
        let path = std::path::Path::new(SETTINGS_PATH);
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(settings) = serde_json::from_str(&data) {
                    return settings;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(SETTINGS_PATH, data);
        }
    }
}

/// HRRR rendered frame data for the overlay.
pub struct HrrrFrame {
    pub pixels: Vec<u8>,  // flat RGBA
    pub width: u32,
    pub height: u32,
}

pub struct RadarApp {
    // Data
    pub current_file: Option<Level2File>,
    pub selected_station: String,
    pub selected_product: RadarProduct,
    pub selected_elevation: usize,
    pub station_filter: String,

    // Rendering - quad view textures
    pub quad_textures: [Option<egui::TextureHandle>; 4],
    pub single_texture: Option<egui::TextureHandle>,
    pub needs_render: bool,
    pub quad_view: bool,
    pub map_view: MapView,
    pub tile_manager: MapTileManager,
    pub tile_textures: std::collections::HashMap<crate::render::map::TileKey, egui::TextureHandle>,

    // Data fetching
    pub fetcher: NexradFetcher,
    pub pack_manager: DataPackManager,

    // Date picker
    pub date_year: i32,
    pub date_month: u32,
    pub date_day: u32,

    // Cross section
    pub cross_section_mode: bool,
    pub cross_section_start: Option<(f64, f64)>,
    pub cross_section_end: Option<(f64, f64)>,
    pub cross_section_texture: Option<egui::TextureHandle>,
    pub cross_section_result: Option<crate::render::cross_section::CrossSectionResult>,
    pub cross_section_max_alt_km: f64,
    pub cross_section_3d: bool,
    pub cross_section_view_angle: f64,
    pub cross_section_view_pitch: f64,

    // Animation / looping
    pub anim_frames: Vec<Level2File>,
    pub anim_frame_names: Vec<String>,
    pub anim_index: usize,
    pub anim_playing: bool,
    pub anim_speed_ms: u64,
    pub anim_last_advance: Option<Instant>,
    pub anim_loading: bool,
    pub anim_download_queue: Vec<String>, // keys to download (kept for reference)
    pub anim_frame_count: usize, // how many frames to load
    #[cfg(not(target_arch = "wasm32"))]
    pub anim_download_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(usize, Vec<u8>)>>,
    pub anim_pending_frames: Vec<Option<Level2File>>, // sparse vec filled as downloads complete
    pub anim_received_count: usize, // how many frames received so far
    pub anim_download_index_map: Vec<usize>, // maps download index -> anim frame index (for partial preload)
    pub pending_auto_anim: bool, // auto-load animation when file list arrives
    pub pending_anim_prerender: bool, // trigger pre-render on next update

    // Pre-rendered animation textures
    pub anim_textures: Vec<Option<egui::TextureHandle>>,
    pub anim_quad_textures: Vec<[Option<egui::TextureHandle>; 4]>,

    // Background preloading
    #[cfg(not(target_arch = "wasm32"))]
    pub preload_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(usize, Vec<u8>)>>,
    pub preloaded_data: Vec<Option<Vec<u8>>>,
    pub preload_keys: Vec<String>,
    pub preload_count: usize,

    // Wall mode (multi-radar)
    pub wall_mode: bool,
    pub wall_panels: Vec<WallPanel>,
    pub wall_loading_index: usize,
    pub wall_fetcher: Option<NexradFetcher>,

    // Multi-radar: secondary radars displayed simultaneously on the map
    pub secondary_radars: Vec<RadarInstance>,

    // Multi-radar sync timeline
    pub sync_timeline: Vec<i64>,
    pub sync_frame_map: Vec<Vec<Option<usize>>>,
    pub multi_radar_anim: bool,
    pub multi_anim_progress: Vec<(String, usize, usize)>,
    pub multi_anim_ready: bool,

    // Interaction
    pub cursor_lat: f64,
    pub cursor_lon: f64,

    // Color table preset
    pub color_preset: crate::render::color_table::ColorTablePreset,
    pub color_table_manager: ColorTableManager,

    // Settings
    pub settings: AppSettings,

    // GPU rendering
    pub gpu_renderer: Option<GpuRadarRenderer>,
    pub gpu_rendering: bool,

    // Dynamic render resolution — scales with zoom for sharp detail when zoomed in
    pub render_resolution: u32,
    pub last_render_zoom: f64,

    // Rendering state — range_km from last render, used for consistent overlay positioning
    pub last_render_range_km: Option<f64>,
    pub quad_render_range_km: [Option<f64>; 4],

    // Weather alerts
    pub alert_fetcher: AlertFetcher,
    pub show_warnings: bool,
    pub warning_opacity: f32,

    // Rotation detection
    pub meso_detections: Vec<crate::nexrad::detection::MesocycloneDetection>,
    pub tvs_detections: Vec<crate::nexrad::detection::TVSDetection>,
    pub show_detections: bool,

    // Tornado prediction (DeepGuess)
    pub prediction_active: bool,
    pub prediction_target: Option<(f64, f64)>, // (lat, lon) of storm center
    pub prediction_buffer: std::collections::VecDeque<Level2File>, // latest 8 frames
    pub prediction_result: Option<crate::predict::convert::TornadoPrediction>,
    pub prediction_history: Vec<(std::time::Instant, f32)>, // (time, prediction_prob) for trend
    #[cfg(feature = "tornado-predict")]
    pub tornado_predictor: Option<crate::predict::TornadoPredictor>,
    pub prediction_last_run: Option<std::time::Instant>,

    // Auto-infer: run prediction on every active radar automatically
    pub auto_infer: bool,
    pub auto_infer_was_off: bool, // track toggle for backfill trigger
    pub auto_infer_results: Vec<crate::predict::convert::TornadoPrediction>,

    // Demo mode
    pub prediction_demo_pending: bool,

    // Prediction backfill: download previous frames for immediate inference
    #[cfg(not(target_arch = "wasm32"))]
    pub prediction_backfill_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(usize, Vec<u8>)>>,
    pub prediction_backfill_count: usize,
    pub prediction_backfill_received: usize,

    // Auto-scan: nationwide tornado risk scanning
    pub autoscan: crate::predict::AutoScanManager,

    // Dual pane mode (side-by-side comparison)
    pub dual_pane: bool,
    pub dual_pane_product: RadarProduct,
    pub dual_pane_texture: Option<egui::TextureHandle>,
    pub dual_pane_range_km: Option<f64>,

    // Overlays
    pub show_range_rings: bool,
    pub show_azimuth_lines: bool,
    pub show_cities: bool,

    // Storm motion (for SRV)
    pub storm_motion_dir: f32,
    pub storm_motion_speed: f32,

    // Sounding
    pub sounding_fetcher: SoundingFetcher,
    pub sounding_mode: bool,
    pub sounding_texture: Option<egui::TextureHandle>,
    pub sounding_pending: bool,

    // HRRR model overlay
    pub hrrr_mode: bool,
    pub hrrr_field_idx: usize,        // index into hrrr_render::fields::FIELDS
    pub hrrr_composite: Option<String>, // Some("stp") for composite fields
    pub hrrr_forecast_hour: u8,
    pub hrrr_texture: Option<egui::TextureHandle>,
    pub hrrr_tex_size: [u32; 2],
    pub hrrr_fetching: Arc<Mutex<bool>>,
    pub hrrr_result: Arc<Mutex<Option<HrrrFrame>>>,
    pub hrrr_status: Arc<Mutex<String>>,

    // Measurement tool
    pub measure_mode: bool,
    pub measure_start: Option<(f64, f64)>,
    pub measure_end: Option<(f64, f64)>,

    // Performance stats
    pub perf: PerfStats,

    // GIF export
    pub gif_export_status: Option<String>,

    // Help overlay
    pub show_help: bool,
    pub show_settings: bool,

    // Render mode
    pub smooth_rendering: bool,

    // Opacity & appearance
    pub radar_opacity: f32,
    pub map_opacity: f32,
    pub dark_mode: bool,

    // Collapsible sidebar state
    pub sidebar_expanded: bool,
    pub sidebar_section: crate::ui::SidebarSection,

    // New UI components
    pub hover_preview: HoverPreview,
    pub national_view: NationalView,
    pub minimap: Minimap,
    pub use_new_ui: bool,

    // National mosaic mode
    pub mosaic_mode: bool,
    pub mosaic_threshold_dbz: f32,
    pub mosaic_loading: bool,
    pub mosaic_site_count: usize,
    pub mosaic_loaded_count: Arc<std::sync::atomic::AtomicUsize>,

    // Preload engine
    #[cfg(not(target_arch = "wasm32"))]
    pub preload_engine: Option<crate::preload::PreloadEngine>,
    #[cfg(not(target_arch = "wasm32"))]
    pub last_preload_sync: Option<Instant>,

    // Runtime (native only — wasm uses browser event loop)
    #[cfg(not(target_arch = "wasm32"))]
    runtime: tokio::runtime::Runtime,
}

pub struct WallPanel {
    pub station_id: String,
    pub file: Option<Level2File>,
    pub texture: Option<egui::TextureHandle>,
    pub status: WallPanelStatus,
}

#[derive(Clone, PartialEq)]
pub enum WallPanelStatus {
    Pending,
    Downloading,
    Loaded,
    Error,
}

/// A secondary radar instance loaded alongside the primary radar.
/// Each has its own fetcher and rendering state.
pub struct RadarInstance {
    pub station_id: String,
    pub file: Option<Level2File>,
    pub texture: Option<egui::TextureHandle>,
    pub range_km: Option<f64>,
    pub fetcher: NexradFetcher,
    pub needs_render: bool,

    // Animation frame storage for multi-radar sync
    pub anim_frames: Vec<crate::nexrad::Level2File>,
    pub anim_textures: Vec<Option<egui::TextureHandle>>,
    pub anim_timestamps_ms: Vec<i64>,

    // Per-radar loading state
    pub anim_loading: bool,
    pub anim_pending_frames: Vec<Option<crate::nexrad::Level2File>>,
    pub anim_received_count: usize,
    pub anim_total_count: usize,
    #[cfg(not(target_arch = "wasm32"))]
    pub anim_download_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(usize, Vec<u8>)>>,
    pub anim_download_index_map: Vec<usize>,

    // Per-radar appearance
    pub opacity: f32,
}

impl RadarInstance {
    pub fn new(station_id: &str, runtime: &tokio::runtime::Handle) -> Self {
        let inst = Self {
            station_id: station_id.to_string(),
            file: None,
            texture: None,
            range_km: None,
            fetcher: NexradFetcher::new(runtime.clone()),
            needs_render: false,
            anim_frames: Vec::new(),
            anim_textures: Vec::new(),
            anim_timestamps_ms: Vec::new(),
            anim_loading: false,
            anim_pending_frames: Vec::new(),
            anim_received_count: 0,
            anim_total_count: 0,
            #[cfg(not(target_arch = "wasm32"))]
            anim_download_rx: None,
            anim_download_index_map: Vec::new(),
            opacity: 1.0,
        };
        inst.fetcher.list_recent_files(station_id);
        inst
    }

    /// Create a RadarInstance from an already-parsed Level2File (used by mosaic mode
    /// to avoid re-fetching data that is already in the preload cache).
    pub fn from_file(station_id: &str, file: Level2File, runtime: &tokio::runtime::Handle) -> Self {
        Self {
            station_id: station_id.to_string(),
            file: Some(file),
            texture: None,
            range_km: None,
            fetcher: NexradFetcher::new(runtime.clone()),
            needs_render: true,
            anim_frames: Vec::new(),
            anim_textures: Vec::new(),
            anim_timestamps_ms: Vec::new(),
            anim_loading: false,
            anim_pending_frames: Vec::new(),
            anim_received_count: 0,
            anim_total_count: 0,
            #[cfg(not(target_arch = "wasm32"))]
            anim_download_rx: None,
            anim_download_index_map: Vec::new(),
            opacity: 1.0,
        }
    }
}

/// Default wall stations — major metro area radars across the US
pub const WALL_STATIONS: &[&str] = &[
    "KTLX", "KFWS", "KAMA", "KHGX", "KLZK",
    "KBMX", "KHTX", "KMRX", "KJAX", "KMFL",
    "KLSX", "KIND", "KCLE", "KOKX", "KDIX",
    "KPUX", "KFTG", "KFSD", "KMPX", "KSOX",
];

#[derive(Default)]
pub struct PerfStats {
    pub parse_time: Option<Duration>,
    pub parse_file_size: usize,
    pub render_time: Option<Duration>,
    pub render_quad_times: [Option<Duration>; 4],
    pub download_time: Option<Duration>,
    pub decompress_time: Option<Duration>,
    pub total_radials: usize,
    pub total_gates: usize,
    pub frame_times: VecDeque<Duration>,
    pub last_frame_start: Option<Instant>,
    pub fps: f64,
}

impl RadarApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        #[cfg(not(target_arch = "wasm32"))]
        let handle = runtime.handle().clone();

        let settings = AppSettings::load();

        let fetcher = NexradFetcher::new(handle.clone());
        let tile_manager = MapTileManager::new(handle.clone());

        // Try to initialize GPU compute renderer (uses its own dedicated wgpu device)
        let gpu_renderer = cc.wgpu_render_state.as_ref().and_then(|rs| {
            log::info!("Initializing GPU compute radar renderer");
            GpuRadarRenderer::new(rs)
        });
        if gpu_renderer.is_some() {
            log::info!("GPU radar renderer initialized successfully");
        } else {
            log::warn!("No wgpu render state available; GPU rendering disabled");
        }

        let station = settings.default_station.clone();
        let zoom = settings.default_zoom;
        let quad = settings.quad_view;

        let (center_lat, center_lon) = sites::find_site(&station)
            .map(|s| (s.lat, s.lon))
            .unwrap_or((35.333, -97.278));

        let mut app = Self {
            current_file: None,
            selected_station: station,
            selected_product: RadarProduct::Reflectivity,
            selected_elevation: 0,
            station_filter: String::new(),

            quad_textures: [None, None, None, None],
            single_texture: None,
            needs_render: false,
            quad_view: quad,
            map_view: MapView {
                center_lat,
                center_lon,
                zoom,
            },
            tile_manager,
            tile_textures: std::collections::HashMap::new(),

            fetcher,
            pack_manager: DataPackManager::new(handle.clone()),

            date_year: chrono::Utc::now().year(),
            date_month: chrono::Utc::now().month(),
            date_day: chrono::Utc::now().day(),

            #[cfg(not(target_arch = "wasm32"))]
            preload_rx: None,
            preloaded_data: Vec::new(),
            preload_keys: Vec::new(),
            preload_count: 10,

            wall_mode: false,
            wall_panels: Vec::new(),
            wall_loading_index: 0,
            wall_fetcher: None,

            anim_frames: Vec::new(),
            anim_frame_names: Vec::new(),
            anim_index: 0,
            anim_playing: false,
            anim_speed_ms: 200,
            anim_last_advance: None,
            anim_loading: false,
            anim_download_queue: Vec::new(),
            anim_frame_count: 10,
            #[cfg(not(target_arch = "wasm32"))]
            anim_download_rx: None,
            anim_pending_frames: Vec::new(),
            anim_received_count: 0,
            anim_download_index_map: Vec::new(),
            pending_auto_anim: false,
            pending_anim_prerender: false,

            anim_textures: Vec::new(),
            anim_quad_textures: Vec::new(),

            cross_section_mode: false,
            cross_section_start: None,
            cross_section_end: None,
            cross_section_texture: None,
            cross_section_result: None,
            cross_section_max_alt_km: 20.0,
            cross_section_3d: false,
            cross_section_view_angle: 25.0,
            cross_section_view_pitch: 20.0,

            secondary_radars: Vec::new(),

            sync_timeline: Vec::new(),
            sync_frame_map: Vec::new(),
            multi_radar_anim: false,
            multi_anim_progress: Vec::new(),
            multi_anim_ready: false,

            cursor_lat: 0.0,
            cursor_lon: 0.0,

            color_preset: crate::render::color_table::ColorTablePreset::Default,
            color_table_manager: ColorTableManager::load_persisted(),

            gpu_rendering: false, // GPU path has readback overhead; CPU+Rayon is faster
            gpu_renderer,
            render_resolution: 1024,
            last_render_zoom: zoom,
            last_render_range_km: None,
            quad_render_range_km: [None; 4],

            alert_fetcher: AlertFetcher::new(handle.clone()),
            show_warnings: true,
            warning_opacity: 1.0,

            meso_detections: Vec::new(),
            tvs_detections: Vec::new(),
            show_detections: false,

            prediction_active: false,
            prediction_target: None,
            prediction_buffer: std::collections::VecDeque::with_capacity(20),
            prediction_result: None,
            prediction_history: Vec::new(),
            #[cfg(feature = "tornado-predict")]
            tornado_predictor: crate::predict::TornadoPredictor::find_model()
                .and_then(|p| {
                    log::info!("Loading tornado prediction model...");
                    crate::predict::TornadoPredictor::load(&p).ok()
                }),
            prediction_last_run: None,

            auto_infer: false,
            auto_infer_was_off: true,
            auto_infer_results: Vec::new(),
            prediction_demo_pending: false,
            #[cfg(not(target_arch = "wasm32"))]
            prediction_backfill_rx: None,
            prediction_backfill_count: 0,
            prediction_backfill_received: 0,

            autoscan: crate::predict::AutoScanManager::new(handle.clone()),

            dual_pane: false,
            dual_pane_product: RadarProduct::Velocity,
            dual_pane_texture: None,
            dual_pane_range_km: None,

            show_range_rings: true,
            show_azimuth_lines: false,
            show_cities: true,

            storm_motion_dir: 240.0,
            storm_motion_speed: 30.0,

            sounding_fetcher: SoundingFetcher::new(handle.clone()),
            sounding_mode: false,
            sounding_texture: None,
            sounding_pending: false,

            hrrr_mode: false,
            hrrr_field_idx: 0,
            hrrr_composite: None,
            hrrr_forecast_hour: 0,
            hrrr_texture: None,
            hrrr_tex_size: [0, 0],
            hrrr_fetching: Arc::new(Mutex::new(false)),
            hrrr_result: Arc::new(Mutex::new(None)),
            hrrr_status: Arc::new(Mutex::new("Ready".to_string())),

            measure_mode: false,
            measure_start: None,
            measure_end: None,

            settings: AppSettings::load(),

            gif_export_status: None,

            perf: PerfStats::default(),

            show_help: false,
            show_settings: false,

            smooth_rendering: false,

            radar_opacity: 0.85,
            map_opacity: 1.0,
            dark_mode: true,

            sidebar_expanded: false,
            sidebar_section: crate::ui::SidebarSection::Station,

            hover_preview: HoverPreview::new(),
            national_view: NationalView::new(),
            minimap: Minimap::new(),
            use_new_ui: true,

            mosaic_mode: false,
            mosaic_threshold_dbz: 30.0,
            mosaic_loading: false,
            mosaic_site_count: 0,
            mosaic_loaded_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),

            #[cfg(not(target_arch = "wasm32"))]
            preload_engine: None,
            #[cfg(not(target_arch = "wasm32"))]
            last_preload_sync: None,

            #[cfg(not(target_arch = "wasm32"))]
            runtime,
        };

        // Initialize preload engine
        #[cfg(not(target_arch = "wasm32"))]
        {
            let preload = crate::preload::PreloadEngine::new(app.runtime.handle().clone());
            app.preload_engine = Some(preload);
        }

        // Auto-load latest data on startup
        app.fetch_latest();
        // Fetch weather alerts
        app.alert_fetcher.fetch_alerts();

        // Start preloading active weather sites
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(ref engine) = app.preload_engine {
                let alerts = app.alert_fetcher.get_alerts();
                engine.preload_active_weather(&alerts);
            }
        }

        // Apply theme
        crate::ui::theme::NexViewTheme::dark().apply_to_egui(&cc.egui_ctx);

        app
    }

    pub fn select_station(&mut self, station_id: &str) {
        // If the station was a secondary radar, remove it (it's now the primary)
        self.secondary_radars.retain(|r| r.station_id != station_id);

        // Clear prediction/auto-infer state for the old station
        self.prediction_buffer.clear();
        self.prediction_result = None;
        self.prediction_history.clear();
        self.auto_infer_results.clear();
        self.prediction_backfill_count = 0;
        self.prediction_backfill_received = 0;
        #[cfg(not(target_arch = "wasm32"))]
        { self.prediction_backfill_rx = None; }

        self.selected_station = station_id.to_string();
        if let Some(site) = sites::find_site(station_id) {
            self.map_view.center_lat = site.lat;
            self.map_view.center_lon = site.lon;
        }

        // Try to load from preload cache first (instant switch)
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cached_file = self.preload_engine.as_ref().and_then(|engine| {
                let cache = engine.get_cache();
                let guard = cache.try_read().ok()?;
                guard.get(station_id).map(|cached| cached.file.clone())
            });
            if let Some(file) = cached_file {
                self.current_file = Some(file);
                self.selected_elevation = 0;
                self.needs_render = true;
                log::info!("Loaded {} from preload cache (instant)", station_id);
                // Still fetch in background to get the absolute latest,
                // but user sees data immediately
                self.fetch_latest();
                return;
            }
        }

        // Fall back to S3 fetch
        self.fetch_latest();
    }

    /// Mark the primary and all secondary radars as needing re-render.
    /// Call this when a shared setting changes (product, color table, etc.).
    pub fn mark_all_needs_render(&mut self) {
        self.needs_render = true;
        // Invalidate ALL cached animation textures so product/color changes take effect
        for tex in &mut self.anim_textures {
            *tex = None;
        }
        self.anim_quad_textures.clear();
        // Schedule re-render of animation textures on next update
        if !self.anim_frames.is_empty() {
            self.pending_anim_prerender = true;
        }
        for inst in &mut self.secondary_radars {
            inst.needs_render = true;
            for tex in &mut inst.anim_textures {
                *tex = None;
            }
        }
    }

    /// Add a secondary radar to display simultaneously on the map.
    /// Does nothing if the station is already loaded (primary or secondary).
    pub fn add_secondary_radar(&mut self, station_id: &str) {
        // Don't add if it's the primary station
        if self.selected_station == station_id {
            return;
        }
        // Don't add duplicates
        if self.secondary_radars.iter().any(|r| r.station_id == station_id) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let handle = self.runtime.handle().clone();
            let instance = RadarInstance::new(station_id, &handle);
            log::info!("Added secondary radar: {}", station_id);
            self.secondary_radars.push(instance);
        }
    }

    /// Remove a secondary radar by station ID.
    pub fn remove_secondary_radar(&mut self, station_id: &str) {
        self.secondary_radars.retain(|r| r.station_id != station_id);

        // Clear prediction/auto-infer state for the old station
        self.prediction_buffer.clear();
        self.prediction_result = None;
        self.prediction_history.clear();
        self.auto_infer_results.clear();
        self.prediction_backfill_count = 0;
        self.prediction_backfill_received = 0;
        #[cfg(not(target_arch = "wasm32"))]
        { self.prediction_backfill_rx = None; }
        log::info!("Removed secondary radar: {}", station_id);
    }

    /// Activate national mosaic mode: fetch all CONUS NEXRAD sites and display
    /// those with storms (reflectivity above threshold) simultaneously.
    pub fn activate_mosaic_mode(&mut self) {
        self.mosaic_mode = true;
        self.mosaic_loading = true;
        self.mosaic_loaded_count.store(0, std::sync::atomic::Ordering::Relaxed);

        // Get all CONUS station IDs
        let all_stations: Vec<String> = sites::RADAR_SITES.iter()
            .map(|s| s.id.to_string())
            .collect();
        self.mosaic_site_count = all_stations.len();

        // Zoom out to national view (CONUS center)
        self.map_view.center_lat = 39.0;
        self.map_view.center_lon = -98.0;
        self.map_view.zoom = 4.5;

        // Start preloading all sites
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(engine) = &self.preload_engine {
            engine.preload_all_conus();
        }
        log::info!("Mosaic mode activated: fetching {} NEXRAD sites", self.mosaic_site_count);
    }

    /// Deactivate national mosaic mode and clear mosaic secondary radars.
    pub fn deactivate_mosaic_mode(&mut self) {
        self.mosaic_mode = false;
        self.mosaic_loading = false;
        // Clear secondary radars that were added by mosaic
        self.secondary_radars.clear();
        log::info!("Mosaic mode deactivated");
    }

    /// Sync secondary radars from the preload cache for mosaic mode.
    /// Adds radars above the reflectivity threshold and removes those below it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn sync_mosaic_from_cache(&mut self, _ctx: &egui::Context) {
        let cache_arc = match &self.preload_engine {
            Some(engine) => engine.get_cache(),
            None => return,
        };

        let cache = match cache_arc.read() {
            Ok(c) => c,
            Err(_) => return,
        };

        let handle = self.runtime.handle().clone();
        let threshold = self.mosaic_threshold_dbz;

        // Collect stations that meet the threshold
        let mut qualifying_stations: Vec<(String, Level2File)> = Vec::new();
        let mut total_loaded: usize = 0;

        for (station_id, entry) in cache.iter() {
            total_loaded += 1;
            if entry.max_reflectivity >= threshold {
                // Only add if not already present as a secondary radar
                if !self.secondary_radars.iter().any(|r| r.station_id == *station_id) {
                    qualifying_stations.push((station_id.clone(), entry.file.clone()));
                }
            }
        }

        // Update loaded count
        self.mosaic_loaded_count.store(total_loaded, std::sync::atomic::Ordering::Relaxed);

        // Check if loading is complete
        if total_loaded >= self.mosaic_site_count && self.mosaic_site_count > 0 {
            self.mosaic_loading = false;
        }

        // Remove secondary radars that are now below threshold
        self.secondary_radars.retain(|r| {
            cache.get(&r.station_id)
                .map_or(true, |entry| entry.max_reflectivity >= threshold)
        });

        // Add new qualifying stations
        for (station_id, file) in qualifying_stations {
            let instance = RadarInstance::from_file(&station_id, file, &handle);
            log::info!("Mosaic: added {} (max_ref={:.1} dBZ)",
                station_id,
                cache.get(&station_id).map_or(0.0, |e| e.max_reflectivity));
            self.secondary_radars.push(instance);
        }
    }

    pub fn save_as_default(&mut self) {
        self.settings.default_station = self.selected_station.clone();
        self.settings.default_zoom = self.map_view.zoom;
        self.settings.quad_view = self.quad_view;
        self.settings.save();
        log::info!("Saved default settings: station={}", self.selected_station);
    }

    pub fn fetch_latest(&mut self) {
        self.fetcher.list_recent_files(&self.selected_station);
    }

    pub fn fetch_for_date(&mut self) {
        if let Some(date) = NaiveDate::from_ymd_opt(self.date_year, self.date_month, self.date_day) {
            self.fetcher.list_files(&self.selected_station, date);
        }
    }

    pub fn start_wall_mode(&mut self) {
        self.wall_mode = true;
        self.wall_panels = WALL_STATIONS.iter().map(|&s| WallPanel {
            station_id: s.to_string(),
            file: None,
            texture: None,
            status: WallPanelStatus::Pending,
        }).collect();
        self.wall_loading_index = 0;

        // Create a dedicated fetcher for wall mode
        #[cfg(not(target_arch = "wasm32"))]
        {
            let handle = self.runtime.handle().clone();
            self.wall_fetcher = Some(NexradFetcher::new(handle));
        }

        // Start loading the first station
        if let Some(ref fetcher) = self.wall_fetcher {
            if let Some(panel) = self.wall_panels.first_mut() {
                panel.status = WallPanelStatus::Downloading;
                fetcher.list_recent_files(&panel.station_id);
            }
        }
    }

    fn check_wall_downloads(&mut self, ctx: &egui::Context) {
        if !self.wall_mode {
            return;
        }

        let fetcher = match &self.wall_fetcher {
            Some(f) => f,
            None => return,
        };

        let idx = self.wall_loading_index;
        if idx >= self.wall_panels.len() {
            return;
        }

        if let Some(data) = fetcher.take_downloaded_data() {
            match Level2File::parse(&data) {
                Ok(file) => {
                    let product = self.selected_product;
                    let base = product.base_product();
                    let require_sr = product.is_super_res();
                    let station_id = self.wall_panels[idx].station_id.clone();
                    let site = sites::find_site(&station_id);

                    if let Some(site) = site {
                        let sweep_idx = file.sweeps.iter().position(|s| {
                            Self::sweep_matches(s, base, require_sr)
                        }).unwrap_or(0);

                        if let Some(sweep) = file.sweeps.get(sweep_idx) {
                            let rendered = RadarRenderer::render_sweep(sweep, base, site, 256);
                            if let Some(rendered) = rendered {
                                let image = egui::ColorImage::from_rgba_unmultiplied(
                                    [rendered.width as usize, rendered.height as usize],
                                    &rendered.pixels,
                                );
                                self.wall_panels[idx].texture = Some(ctx.load_texture(
                                    format!("wall_{}", station_id),
                                    image,
                                    egui::TextureOptions::NEAREST,
                                ));
                            }
                        }
                    }

                    self.wall_panels[idx].file = Some(file);
                    self.wall_panels[idx].status = WallPanelStatus::Loaded;
                    log::info!("Wall: loaded {} ({}/{})", station_id, idx + 1, self.wall_panels.len());
                }
                Err(e) => {
                    log::error!("Wall: failed to parse {}: {}", self.wall_panels[idx].station_id, e);
                    self.wall_panels[idx].status = WallPanelStatus::Error;
                }
            }

            // Move to next station
            self.wall_loading_index += 1;
            if self.wall_loading_index < self.wall_panels.len() {
                let next_station = self.wall_panels[self.wall_loading_index].station_id.clone();
                self.wall_panels[self.wall_loading_index].status = WallPanelStatus::Downloading;
                fetcher.list_recent_files(&next_station);
            }
        } else if !fetcher.is_fetching() {
            // Fetcher finished but no data — station had no files, skip it
            log::warn!("Wall: no data for {} — skipping", self.wall_panels[idx].station_id);
            self.wall_panels[idx].status = WallPanelStatus::Error;
            self.wall_loading_index += 1;
            if self.wall_loading_index < self.wall_panels.len() {
                let next_station = self.wall_panels[self.wall_loading_index].station_id.clone();
                self.wall_panels[self.wall_loading_index].status = WallPanelStatus::Downloading;
                fetcher.list_recent_files(&next_station);
            }
        }
    }

    fn draw_wall_mode(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let count = self.wall_panels.len();
        if count == 0 {
            return;
        }

        // Calculate grid layout
        let cols = (count as f32).sqrt().ceil() as usize;
        let rows = (count + cols - 1) / cols;
        let cell_w = rect.width() / cols as f32;
        let cell_h = rect.height() / rows as f32;

        for (i, panel) in self.wall_panels.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + col as f32 * cell_w, rect.top() + row as f32 * cell_h),
                egui::vec2(cell_w, cell_h),
            );

            // Dark background
            ui.painter().rect_filled(cell_rect, 0.0, egui::Color32::from_rgb(15, 15, 25));

            // Radar texture
            if let Some(tex) = &panel.texture {
                let margin = 2.0;
                let img_rect = cell_rect.shrink(margin);
                // Keep square, centered
                let side = img_rect.width().min(img_rect.height());
                let centered = egui::Rect::from_center_size(img_rect.center(), egui::vec2(side, side));
                ui.painter().image(
                    tex.id(),
                    centered,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            // Station label
            let label_bg = egui::Rect::from_min_size(cell_rect.min, egui::vec2(60.0, 18.0));
            ui.painter().rect_filled(label_bg, 2.0, egui::Color32::from_black_alpha(200));

            let status_color = match panel.status {
                WallPanelStatus::Loaded => egui::Color32::from_rgb(100, 255, 100),
                WallPanelStatus::Downloading => egui::Color32::YELLOW,
                WallPanelStatus::Error => egui::Color32::RED,
                WallPanelStatus::Pending => egui::Color32::GRAY,
            };

            ui.painter().text(
                cell_rect.min + egui::vec2(4.0, 2.0),
                egui::Align2::LEFT_TOP,
                &panel.station_id,
                egui::FontId::proportional(12.0),
                status_color,
            );

            // Border
            ui.painter().rect_stroke(cell_rect, 0.0, egui::Stroke::new(0.5, egui::Color32::from_gray(40)), egui::StrokeKind::Outside);
        }
    }

    pub fn set_tile_provider(&mut self, provider: TileProvider) {
        self.tile_manager.set_provider(provider);
        self.tile_textures.clear();
    }

    /// Try to load animation from a local data pack first, then fall back to S3
    pub fn load_animation_frames(&mut self) {
        // If we have secondary radars, load animation for all of them
        if !self.secondary_radars.is_empty() {
            self.load_multi_radar_animation();
            return;
        }
        self.multi_radar_anim = false;
        self.multi_anim_ready = false;

        // Check for local data pack
        if let Some(pack_files) = self.pack_manager.load_pack(
            &self.selected_station, self.date_year, self.date_month, self.date_day
        ) {
            log::info!("Loading animation from data pack: {} files", pack_files.len());
            self.anim_frames.clear();
            self.anim_textures.clear();
            self.anim_quad_textures.clear();
            self.anim_frame_names.clear();

            let count = self.anim_frame_count.min(pack_files.len());
            let start = pack_files.len() - count;

            for (name, data) in &pack_files[start..] {
                match Level2File::parse(data) {
                    Ok(file) => {
                        self.anim_frame_names.push(name.clone());
                        self.anim_frames.push(file);
                    }
                    Err(e) => {
                        log::warn!("Failed to parse pack file {}: {}", name, e);
                    }
                }
            }

            if !self.anim_frames.is_empty() {
                self.current_file = Some(self.anim_frames[0].clone());
                self.anim_index = 0;
                self.anim_playing = true;
                self.anim_loading = false;
                self.anim_last_advance = Some(std::time::Instant::now());
                self.needs_render = true;
                self.pending_anim_prerender = true;
                log::info!("Animation loaded from data pack: {} frames", self.anim_frames.len());
            }
            return;
        }

        // Fall back to S3 download
        let files = self.fetcher.available_files.lock().unwrap().clone();
        if files.is_empty() {
            return;
        }

        // Take the last N files
        let count = self.anim_frame_count.min(files.len());
        let start = files.len() - count;
        let keys: Vec<String> = files[start..].iter().map(|f| f.key.clone()).collect();
        let names: Vec<String> = files[start..].iter().map(|f| f.display_name.clone()).collect();

        let num_keys = keys.len();
        self.anim_frames.clear();
        self.anim_textures.clear();
        self.anim_quad_textures.clear();
        self.anim_frame_names = names;
        self.anim_download_queue = keys.clone();
        self.anim_loading = true;
        self.anim_index = 0;
        self.anim_playing = false;
        self.anim_pending_frames = vec![None; num_keys];
        self.anim_received_count = 0;

        // Check which keys are already preloaded
        let mut keys_to_download = Vec::new();
        let mut download_index_map = Vec::new(); // maps download index -> anim frame index
        let mut preload_hits = 0;

        for (anim_idx, key) in keys.iter().enumerate() {
            // Look up this key in preload_keys
            let preload_match = self.preload_keys.iter().position(|pk| pk == key);
            if let Some(preload_idx) = preload_match {
                if let Some(Some(data)) = self.preloaded_data.get(preload_idx) {
                    // Parse the preloaded data directly
                    match Level2File::parse(data) {
                        Ok(file) => {
                            self.anim_pending_frames[anim_idx] = Some(file);
                            self.anim_received_count += 1;
                            preload_hits += 1;
                            continue;
                        }
                        Err(e) => {
                            log::warn!("Failed to parse preloaded frame {}: {}", key, e);
                            // Fall through to download
                        }
                    }
                }
            }
            keys_to_download.push(key.clone());
            download_index_map.push(anim_idx);
        }

        if preload_hits > 0 {
            log::info!("Animation: {} of {} frames from preload cache, {} to download",
                preload_hits, num_keys, keys_to_download.len());
        }

        // Clear preload data since we've consumed it
        self.preloaded_data.clear();
        self.preload_keys.clear();
        self.preload_rx = None;

        if keys_to_download.is_empty() {
            // All frames were preloaded — finalize immediately
            self.anim_frames = self.anim_pending_frames
                .iter_mut()
                .filter_map(|slot| slot.take())
                .collect();
            self.anim_pending_frames.clear();
            self.anim_loading = false;
            self.anim_playing = true;
            self.anim_last_advance = Some(Instant::now());
            if !self.anim_frames.is_empty() {
                self.current_file = Some(self.anim_frames[0].clone());
                self.needs_render = true;
            }
            log::info!("Animation loaded instantly from preload: {} frames", self.anim_frames.len());
            // Schedule pre-rendering for next update() when we have ctx
            self.pending_anim_prerender = true;
        } else {
            // Download remaining frames; we need to remap indices
            // The download_files_parallel returns (download_idx, data), but we need anim_idx
            // We'll store the mapping and handle it in check_animation_downloads
            // For simplicity, download all remaining keys and use the index mapping
            self.anim_download_index_map = download_index_map;
            let rx = self.fetcher.download_files_parallel(keys_to_download);
            self.anim_download_rx = Some(rx);
        }
    }

    fn check_animation_downloads(&mut self, ctx: &egui::Context) {
        if !self.anim_loading {
            return;
        }

        ctx.request_repaint();

        // Poll the parallel download receiver (drain all available to avoid stalling)
        let mut processed = 0;
        if let Some(rx) = &mut self.anim_download_rx {
            loop {
                match rx.try_recv() {
                    Ok((dl_idx, data)) => {
                        processed += 1;
                        // Map download index to animation frame index
                        let anim_idx = if !self.anim_download_index_map.is_empty() {
                            self.anim_download_index_map.get(dl_idx).copied().unwrap_or(dl_idx)
                        } else {
                            dl_idx
                        };
                        match Level2File::parse(&data) {
                            Ok(file) => {
                                if anim_idx < self.anim_pending_frames.len() {
                                    self.anim_pending_frames[anim_idx] = Some(file);
                                }
                                self.anim_received_count += 1;
                                log::info!("Animation frame {}/{} loaded (index {})",
                                    self.anim_received_count, self.anim_download_queue.len(), anim_idx);
                            }
                            Err(e) => {
                                self.anim_received_count += 1;
                                log::error!("Failed to parse animation frame {}: {}", anim_idx, e);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        // Check if all frames are received
        let total = self.anim_download_queue.len();
        if self.anim_received_count >= total && total > 0 {
            // Compact: collect non-None frames in order
            self.anim_frames = self.anim_pending_frames
                .iter_mut()
                .filter_map(|slot| slot.take())
                .collect();
            self.anim_pending_frames.clear();
            self.anim_download_rx = None;

            self.anim_loading = false;
            self.anim_playing = true;
            self.anim_last_advance = Some(Instant::now());
            if !self.anim_frames.is_empty() {
                self.current_file = Some(self.anim_frames[0].clone());
                self.needs_render = true;
            }
            log::info!("Animation loaded: {} frames (parallel)", self.anim_frames.len());
            // Pre-render all animation frame textures for smooth playback
            self.pre_render_animation_textures(ctx);
        }
    }

    /// Pre-render ALL animation frame textures eagerly for smooth playback.
    /// This avoids expensive Level2File clones during animation and prevents stutter.
    fn pre_render_animation_textures(&mut self, ctx: &egui::Context) {
        let num_frames = self.anim_frames.len();
        self.anim_textures = vec![None; num_frames];
        self.anim_quad_textures = vec![[None, None, None, None]; num_frames];

        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => {
                log::info!("Animation ready: {} frames (no site, textures on demand)", num_frames);
                return;
            }
        };

        let product = self.selected_product.base_product();
        let color_table = self.color_table_manager.resolve(product);

        // Lock elevation: use frame 0's elevation as target
        let target_elevation = self.anim_frames.first().and_then(|f0| {
            Self::find_sweep_for_product_in_frame(f0, self.selected_product, None)
                .and_then(|idx| f0.sweeps.get(idx))
                .and_then(|s| s.radials.first())
                .map(|r| r.elevation)
        });

        let render_start = Instant::now();
        for fi in 0..num_frames {
            let frame = &self.anim_frames[fi];
            let sweep_idx = Self::find_sweep_for_product_in_frame(frame, self.selected_product, target_elevation)
                .unwrap_or(0);
            if let Some(sweep) = frame.sweeps.get(sweep_idx) {
                let rendered = if self.smooth_rendering {
                    RadarRenderer::render_sweep_smooth(sweep, product, site, 1024, &color_table)
                } else {
                    RadarRenderer::render_sweep_with_table(sweep, product, site, 1024, &color_table)
                };
                if let Some(r) = rendered {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [r.width as usize, r.height as usize], &r.pixels,
                    );
                    self.anim_textures[fi] = Some(ctx.load_texture(
                        format!("anim_single_{}", fi), image, egui::TextureOptions::NEAREST,
                    ));
                }
            }
        }
        log::info!("Animation pre-rendered: {} frames in {:?}", num_frames, render_start.elapsed());
    }

    /// Ensure the texture for a specific animation frame is rendered.
    /// Called lazily when the frame is actually displayed.
    /// Find the best sweep index for a product in a specific frame,
    /// locked to the same elevation angle as the first frame to prevent tilt flicker.
    fn find_sweep_for_product_in_frame(frame: &Level2File, product: RadarProduct, target_elevation: Option<f32>) -> Option<usize> {
        let base = product.base_product();
        let require_sr = product.is_super_res();

        // Find all matching sweeps
        let matches: Vec<(usize, f32)> = frame.sweeps.iter().enumerate()
            .filter(|(_, s)| Self::sweep_matches(s, base, require_sr) || Self::sweep_matches(s, base, false))
            .map(|(i, s)| {
                let elev = s.radials.first().map(|r| r.elevation).unwrap_or(0.0);
                (i, elev)
            })
            .collect();

        if matches.is_empty() { return None; }

        if let Some(target) = target_elevation {
            // Pick the sweep closest to the target elevation
            matches.iter()
                .min_by(|a, b| (a.1 - target).abs().partial_cmp(&(b.1 - target).abs()).unwrap_or(std::cmp::Ordering::Equal))
                .map(|&(idx, _)| idx)
        } else {
            // No target — use the first matching sweep (lowest tilt)
            Some(matches[0].0)
        }
    }

    fn ensure_anim_texture(&mut self, frame_idx: usize, ctx: &egui::Context) {
        if frame_idx >= self.anim_frames.len() { return; }
        if frame_idx < self.anim_textures.len() && self.anim_textures[frame_idx].is_some() {
            return; // already rendered
        }

        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        let product = self.selected_product.base_product();
        let color_table = self.color_table_manager.resolve(product);

        // Lock elevation angle: use frame 0's elevation as the target
        let target_elevation = self.anim_frames.first().and_then(|f0| {
            Self::find_sweep_for_product_in_frame(f0, self.selected_product, None)
                .and_then(|idx| f0.sweeps.get(idx))
                .and_then(|s| s.radials.first())
                .map(|r| r.elevation)
        });

        let frame = &self.anim_frames[frame_idx];
        let sweep_idx = Self::find_sweep_for_product_in_frame(frame, self.selected_product, target_elevation)
            .unwrap_or(0);

        if let Some(sweep) = frame.sweeps.get(sweep_idx) {
            let rendered = if self.smooth_rendering {
                RadarRenderer::render_sweep_smooth(sweep, product, site, 1024, &color_table)
            } else {
                RadarRenderer::render_sweep_with_table(sweep, product, site, 1024, &color_table)
            };
            if let Some(r) = rendered {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [r.width as usize, r.height as usize],
                    &r.pixels,
                );
                let tex = ctx.load_texture(
                    format!("anim_single_{}", frame_idx),
                    image,
                    egui::TextureOptions::NEAREST,
                );
                if frame_idx < self.anim_textures.len() {
                    self.anim_textures[frame_idx] = Some(tex);
                }
            }
        }

        // Render quad textures if in quad view
        if self.quad_view && frame_idx < self.anim_quad_textures.len() {
            for (qi, &qproduct) in QUAD_PRODUCTS.iter().enumerate() {
                if self.anim_quad_textures[frame_idx][qi].is_some() { continue; }
                let qsweep_idx = Self::find_sweep_for_product_in_frame(frame, qproduct, None)
                    .unwrap_or(0);
                let qsweep = frame.sweeps.get(qsweep_idx);
                if let Some(sweep) = qsweep {
                    let qcolor_table = self.color_table_manager.resolve(qproduct);
                    let rendered = RadarRenderer::render_sweep_with_table(sweep, qproduct, site, 512, &qcolor_table);
                    if let Some(r) = rendered {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [r.width as usize, r.height as usize],
                            &r.pixels,
                        );
                        self.anim_quad_textures[frame_idx][qi] = Some(ctx.load_texture(
                            format!("anim_quad_{}_{}", frame_idx, qi),
                            image,
                            egui::TextureOptions::NEAREST,
                        ));
                    }
                }
            }
        }
    }

    /// Export the current animation loop as a GIF file.
    pub fn export_loop_gif(&mut self) {
        // Use composite export when multi-radar animation is active
        if self.multi_radar_anim && !self.secondary_radars.is_empty() {
            self.export_composite_gif();
            return;
        }

        use crate::render::RadarRenderer;

        if self.anim_frames.is_empty() {
            self.gif_export_status = Some("No animation frames loaded".into());
            log::warn!("GIF export: no animation frames loaded");
            return;
        }

        let site = match crate::nexrad::sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => {
                self.gif_export_status = Some("Unknown station".into());
                log::error!("GIF export: unknown station {}", self.selected_station);
                return;
            }
        };

        log::info!("Exporting {} animation frames as GIF...", self.anim_frames.len());

        let orig_file = self.current_file.take();
        let mut color_images: Vec<egui::ColorImage> = Vec::with_capacity(self.anim_frames.len());

        for frame in &self.anim_frames {
            self.current_file = Some(frame.clone());

            let sweep_idx = self
                .find_sweep_for_product(self.selected_product)
                .unwrap_or(self.selected_elevation);

            if let Some(sweep) = frame.sweeps.get(sweep_idx) {
                if let Some(rendered) = RadarRenderer::render_sweep(
                    sweep,
                    self.selected_product.base_product(),
                    site,
                    1024,
                ) {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [rendered.width as usize, rendered.height as usize],
                        &rendered.pixels,
                    );
                    color_images.push(image);
                }
            }
        }

        self.current_file = orig_file;

        if color_images.is_empty() {
            self.gif_export_status = Some("No frames could be rendered".into());
            log::error!("GIF export: no frames could be rendered");
            return;
        }

        // Build a descriptive filename and save to exports/ next to exe
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let product_name = self.selected_product.display_name().replace(' ', "_");
        let filename = format!("{}_{}_{}frames_{}.gif",
            self.selected_station, product_name,
            color_images.len(), timestamp);

        let export_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("exports");

        if let Err(e) = std::fs::create_dir_all(&export_dir) {
            self.gif_export_status = Some(format!("Failed to create exports dir: {}", e));
            return;
        }

        let path = export_dir.join(&filename);

        match crate::export::export_animation_gif(&color_images, self.anim_speed_ms as u16,
            path.to_str().unwrap_or("nexview_loop.gif"))
        {
            Ok(()) => {
                let msg = format!("Exported {} frames to {}", color_images.len(), path.display());
                log::info!("GIF export: {}", msg);
                self.gif_export_status = Some(msg);
                // Open the exports folder
                #[cfg(target_os = "windows")]
                { let _ = std::process::Command::new("explorer").arg(&export_dir).spawn(); }
            }
            Err(e) => {
                let msg = format!("Export failed: {}", e);
                log::error!("GIF export: {}", msg);
                self.gif_export_status = Some(msg);
            }
        }
    }

    fn advance_animation(&mut self) {
        if !self.anim_playing || self.anim_frames.is_empty() {
            return;
        }

        let should_advance = match self.anim_last_advance {
            Some(last) => last.elapsed().as_millis() >= self.anim_speed_ms as u128,
            None => true,
        };

        if should_advance {
            self.anim_index = (self.anim_index + 1) % self.anim_frames.len();
            self.anim_last_advance = Some(Instant::now());

            // Try to use pre-rendered cached textures
            let has_cached_single = self.anim_textures.get(self.anim_index)
                .and_then(|t| t.as_ref()).is_some();
            let has_cached_quad = self.anim_quad_textures.get(self.anim_index).is_some();

            if has_cached_single || has_cached_quad {
                // Swap in cached textures directly — no re-render needed
                if let Some(cached) = self.anim_textures.get(self.anim_index) {
                    self.single_texture = cached.clone();
                }
                if self.quad_view {
                    if let Some(cached_quad) = self.anim_quad_textures.get(self.anim_index) {
                        self.quad_textures = cached_quad.clone();
                    }
                }
                // Skip expensive Level2File clone — UI reads anim_frame_names for timestamp
                // Only set current_file if it's actually needed (e.g., cursor readout)
            } else {
                // No cached textures — need full render, which requires current_file
                self.current_file = Some(self.anim_frames[self.anim_index].clone());
                self.needs_render = true;
            }

            // Sync secondary radars to the current timeline slot
            if self.multi_radar_anim && self.multi_anim_ready {
                self.sync_secondary_to_frame(self.anim_index);
            }
        }
    }

    // ========================================================================
    // Multi-radar synced animation
    // ========================================================================

    /// Load animation frames for ALL active radars (primary + secondaries) in parallel.
    fn load_multi_radar_animation(&mut self) {
        // Start primary animation download (reuse existing logic inline)
        let files = self.fetcher.available_files.lock().unwrap().clone();
        if files.is_empty() {
            return;
        }

        let count = self.anim_frame_count.min(files.len());
        let start = files.len() - count;
        let keys: Vec<String> = files[start..].iter().map(|f| f.key.clone()).collect();
        let names: Vec<String> = files[start..].iter().map(|f| f.display_name.clone()).collect();
        let num_keys = keys.len();

        self.anim_frames.clear();
        self.anim_textures.clear();
        self.anim_quad_textures.clear();
        self.anim_frame_names = names;
        self.anim_download_queue = keys.clone();
        self.anim_loading = true;
        self.anim_index = 0;
        self.anim_playing = false;
        self.anim_pending_frames = vec![None; num_keys];
        self.anim_received_count = 0;

        // Check preload cache for primary
        let mut keys_to_download = Vec::new();
        let mut download_index_map = Vec::new();
        for (anim_idx, key) in keys.iter().enumerate() {
            let preload_match = self.preload_keys.iter().position(|pk| pk == key);
            if let Some(preload_idx) = preload_match {
                if let Some(Some(data)) = self.preloaded_data.get(preload_idx) {
                    if let Ok(file) = Level2File::parse(data) {
                        self.anim_pending_frames[anim_idx] = Some(file);
                        self.anim_received_count += 1;
                        continue;
                    }
                }
            }
            keys_to_download.push(key.clone());
            download_index_map.push(anim_idx);
        }
        self.preloaded_data.clear();
        self.preload_keys.clear();
        self.preload_rx = None;
        self.anim_download_index_map = download_index_map;

        if keys_to_download.is_empty() {
            self.anim_frames = self.anim_pending_frames.iter_mut().filter_map(|s| s.take()).collect();
            self.anim_pending_frames.clear();
            self.anim_loading = false;
            self.anim_playing = true;
            self.anim_last_advance = Some(Instant::now());
            if !self.anim_frames.is_empty() {
                self.current_file = Some(self.anim_frames[0].clone());
                self.needs_render = true;
            }
            self.pending_anim_prerender = true;
        } else {
            let rx = self.fetcher.download_files_parallel(keys_to_download);
            self.anim_download_rx = Some(rx);
        }

        // Kick off downloads for each secondary radar
        for inst in &mut self.secondary_radars {
            let sec_files = inst.fetcher.available_files.lock().unwrap().clone();
            if sec_files.is_empty() {
                log::warn!("Secondary {} has no files yet, refreshing file list", inst.station_id);
                inst.fetcher.list_recent_files(&inst.station_id);
                inst.anim_loading = false;
                continue;
            }
            let sec_count = self.anim_frame_count.min(sec_files.len());
            let sec_start = sec_files.len() - sec_count;
            let sec_keys: Vec<String> = sec_files[sec_start..].iter().map(|f| f.key.clone()).collect();
            let sec_num = sec_keys.len();

            inst.anim_frames.clear();
            inst.anim_textures.clear();
            inst.anim_timestamps_ms.clear();
            inst.anim_loading = true;
            inst.anim_pending_frames = vec![None; sec_num];
            inst.anim_received_count = 0;
            inst.anim_total_count = sec_num;
            inst.anim_download_index_map = (0..sec_num).collect();

            let rx = inst.fetcher.download_files_parallel(sec_keys);
            inst.anim_download_rx = Some(rx);
        }

        self.multi_radar_anim = true;
        self.multi_anim_ready = false;
        self.update_multi_anim_progress();
        log::info!("Multi-radar animation: loading {} frames for {} radars",
            num_keys, 1 + self.secondary_radars.len());
    }

    /// Poll secondary radar animation download channels.
    fn check_secondary_anim_downloads(&mut self, ctx: &egui::Context) {
        if !self.multi_radar_anim || self.multi_anim_ready { return; }

        let mut any_loading = self.anim_loading; // primary still loading?

        for inst in &mut self.secondary_radars {
            if !inst.anim_loading { continue; }
            any_loading = true;

            if let Some(rx) = &mut inst.anim_download_rx {
                loop {
                    match rx.try_recv() {
                        Ok((dl_idx, data)) => {
                            let anim_idx = inst.anim_download_index_map
                                .get(dl_idx).copied().unwrap_or(dl_idx);
                            match Level2File::parse(&data) {
                                Ok(file) => {
                                    if anim_idx < inst.anim_pending_frames.len() {
                                        inst.anim_pending_frames[anim_idx] = Some(file);
                                    }
                                    inst.anim_received_count += 1;
                                }
                                Err(e) => {
                                    inst.anim_received_count += 1;
                                    log::error!("Secondary {} anim frame {} parse error: {}",
                                        inst.station_id, anim_idx, e);
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            }

            // Finalize when all received
            let total = inst.anim_total_count;
            if inst.anim_received_count >= total && total > 0 {
                inst.anim_frames = inst.anim_pending_frames
                    .drain(..).filter_map(|s| s).collect();
                inst.anim_timestamps_ms = inst.anim_frames.iter()
                    .map(|f| f.unix_timestamp_ms()).collect();
                inst.anim_textures = vec![None; inst.anim_frames.len()];
                inst.anim_loading = false;
                inst.anim_download_rx = None;
                log::info!("Secondary {} animation loaded: {} frames",
                    inst.station_id, inst.anim_frames.len());
            }
        }

        self.update_multi_anim_progress();

        // Check if ALL radars are done
        if !any_loading && self.secondary_radars.iter().all(|r| !r.anim_loading) {
            if !self.multi_anim_ready {
                self.build_sync_timeline();
                self.multi_anim_ready = true;
                // Pre-render secondary animation textures
                self.pre_render_secondary_anim_textures(ctx);
                log::info!("Multi-radar animation ready: {} timeline slots", self.sync_timeline.len());
            }
        }

        ctx.request_repaint();
    }

    /// Build unified sync timeline from primary timestamps.
    /// Each secondary radar's frames are matched to the closest primary timestamp.
    pub fn build_sync_timeline(&mut self) {
        // Primary timestamps are the canonical timeline
        let primary_ts: Vec<i64> = self.anim_frames.iter()
            .map(|f| f.unix_timestamp_ms()).collect();
        self.sync_timeline = primary_ts.clone();

        let mut frame_map = Vec::new();

        // Index 0: primary (identity mapping)
        frame_map.push(primary_ts.iter().enumerate().map(|(i, _)| Some(i)).collect());

        // Index 1+: each secondary, matched by closest timestamp within 5-minute window
        const TOLERANCE_MS: i64 = 5 * 60 * 1000;
        for inst in &self.secondary_radars {
            let mapping: Vec<Option<usize>> = self.sync_timeline.iter().map(|&target_ts| {
                inst.anim_timestamps_ms.iter().enumerate()
                    .min_by_key(|(_, &ts)| (ts - target_ts).abs())
                    .and_then(|(idx, &ts)| {
                        if (ts - target_ts).abs() <= TOLERANCE_MS { Some(idx) } else { None }
                    })
            }).collect();
            frame_map.push(mapping);
        }

        self.sync_frame_map = frame_map;
    }

    /// Swap secondary radar files/textures to match the given timeline index.
    pub fn sync_secondary_to_frame(&mut self, timeline_idx: usize) {
        let frame_map = self.sync_frame_map.clone();
        for (sec_idx, inst) in self.secondary_radars.iter_mut().enumerate() {
            let map_idx = sec_idx + 1; // +1 because index 0 is primary
            if let Some(mapping) = frame_map.get(map_idx) {
                if let Some(Some(frame_idx)) = mapping.get(timeline_idx) {
                    let frame_idx = *frame_idx;
                    if frame_idx < inst.anim_frames.len() {
                        if let Some(Some(tex)) = inst.anim_textures.get(frame_idx) {
                            // Have cached texture — use it directly, skip file clone
                            inst.texture = Some(tex.clone());
                        } else {
                            // No cached texture — need to render, which requires file
                            inst.file = Some(inst.anim_frames[frame_idx].clone());
                            inst.needs_render = true;
                        }
                    }
                } else {
                    inst.texture = None;
                }
            }
        }
    }

    /// Pre-render animation textures for secondary radars.
    /// Renders ALL frames eagerly so animation doesn't need to clone Level2File.
    fn pre_render_secondary_anim_textures(&mut self, ctx: &egui::Context) {
        let product = self.selected_product.base_product();
        let color_table = self.color_table_manager.resolve(product);
        let render_size: u32 = 1024;
        let selected_product = self.selected_product;
        let smooth = self.smooth_rendering;

        for inst in &mut self.secondary_radars {
            let site = match sites::find_site(&inst.station_id) {
                Some(s) => s,
                None => continue,
            };

            // Lock elevation: use frame 0's elevation as target
            let target_elevation = inst.anim_frames.first().and_then(|f0| {
                Self::find_sweep_for_product_in_frame(f0, selected_product, None)
                    .and_then(|idx| f0.sweeps.get(idx))
                    .and_then(|s| s.radials.first())
                    .map(|r| r.elevation)
            });

            inst.anim_textures = vec![None; inst.anim_frames.len()];
            for fi in 0..inst.anim_frames.len() {
                let frame = &inst.anim_frames[fi];
                let sweep_idx = Self::find_sweep_for_product_in_frame(frame, selected_product, target_elevation)
                    .unwrap_or(0);
                if let Some(sweep) = frame.sweeps.get(sweep_idx) {
                    let rendered = if smooth {
                        RadarRenderer::render_sweep_smooth(sweep, product, site, render_size, &color_table)
                    } else {
                        RadarRenderer::render_sweep_with_table(sweep, product, site, render_size, &color_table)
                    };
                    if let Some(r) = rendered {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [r.width as usize, r.height as usize], &r.pixels,
                        );
                        inst.anim_textures[fi] = Some(ctx.load_texture(
                            format!("sec_anim_{}_{}", inst.station_id, fi),
                            image, egui::TextureOptions::NEAREST,
                        ));
                        inst.range_km = Some(r.range_km);
                    }
                }
            }
        }
    }

    /// Update multi_anim_progress for UI display.
    fn update_multi_anim_progress(&mut self) {
        let mut progress = Vec::new();
        let primary_total = self.anim_download_queue.len();
        progress.push((
            self.selected_station.clone(),
            self.anim_received_count,
            primary_total,
        ));
        for inst in &self.secondary_radars {
            progress.push((
                inst.station_id.clone(),
                inst.anim_received_count,
                inst.anim_total_count,
            ));
        }
        self.multi_anim_progress = progress;
    }

    /// Export composite GIF with ALL radars rendered on one canvas per frame.
    pub fn export_composite_gif(&mut self) {
        use crate::render::RadarRenderer;

        if self.anim_frames.is_empty() {
            self.gif_export_status = Some("No animation frames loaded".into());
            return;
        }

        let canvas_w: u32 = 1920;
        let canvas_h: u32 = 1080;
        let num_frames = self.anim_frames.len();
        let mut color_images: Vec<egui::ColorImage> = Vec::with_capacity(num_frames);
        let product = self.selected_product.base_product();
        let color_table = self.color_table_manager.resolve(product);
        let frame_map = self.sync_frame_map.clone();

        log::info!("Exporting composite GIF: {} frames, {} radars, {}x{}",
            num_frames, 1 + self.secondary_radars.len(), canvas_w, canvas_h);

        for timeline_idx in 0..num_frames {
            // Dark background
            let mut canvas = vec![0u8; (canvas_w * canvas_h * 4) as usize];
            for chunk in canvas.chunks_exact_mut(4) {
                chunk[0] = 0x1A; chunk[1] = 0x1A; chunk[2] = 0x2E; chunk[3] = 0xFF;
            }

            // Render secondary radars (back to front)
            for (sec_idx, inst) in self.secondary_radars.iter().enumerate() {
                let map_idx = sec_idx + 1;
                let frame_idx = frame_map.get(map_idx)
                    .and_then(|m| m.get(timeline_idx))
                    .and_then(|opt| *opt);
                if let Some(fi) = frame_idx {
                    if let Some(frame) = inst.anim_frames.get(fi) {
                        Self::composite_radar_to_canvas(
                            &mut canvas, canvas_w, canvas_h,
                            frame, &inst.station_id, inst.opacity,
                            product, &color_table, &self.map_view,
                            self.selected_elevation, self.smooth_rendering,
                        );
                    }
                }
            }

            // Render primary radar on top
            let primary_frame = &self.anim_frames[timeline_idx];
            Self::composite_radar_to_canvas(
                &mut canvas, canvas_w, canvas_h,
                primary_frame, &self.selected_station, self.radar_opacity,
                product, &color_table, &self.map_view,
                self.selected_elevation, self.smooth_rendering,
            );

            let image = egui::ColorImage::from_rgba_unmultiplied(
                [canvas_w as usize, canvas_h as usize], &canvas,
            );
            color_images.push(image);
        }

        if color_images.is_empty() {
            self.gif_export_status = Some("No frames could be rendered".into());
            return;
        }

        // Build filename
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let stations: Vec<&str> = std::iter::once(self.selected_station.as_str())
            .chain(self.secondary_radars.iter().map(|r| r.station_id.as_str()))
            .collect();
        let product_name = self.selected_product.display_name().replace(' ', "_");
        let filename = format!("composite_{}_{}_{}_{}frames_{}.gif",
            stations.join("-"), product_name,
            canvas_w, color_images.len(), timestamp);

        let export_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("exports");

        if let Err(e) = std::fs::create_dir_all(&export_dir) {
            self.gif_export_status = Some(format!("Failed to create exports dir: {}", e));
            return;
        }

        let path = export_dir.join(&filename);
        match crate::export::export_animation_gif(&color_images, self.anim_speed_ms as u16,
            path.to_str().unwrap_or("composite_loop.gif"))
        {
            Ok(()) => {
                let msg = format!("Exported {} composite frames to {}", color_images.len(), path.display());
                log::info!("GIF export: {}", msg);
                self.gif_export_status = Some(msg);
                #[cfg(target_os = "windows")]
                { let _ = std::process::Command::new("explorer").arg(&export_dir).spawn(); }
            }
            Err(e) => {
                let msg = format!("Export failed: {}", e);
                log::error!("GIF export: {}", msg);
                self.gif_export_status = Some(msg);
            }
        }
    }

    /// Render a single radar frame onto a canvas at the correct geographic position.
    fn composite_radar_to_canvas(
        canvas: &mut [u8],
        canvas_w: u32,
        canvas_h: u32,
        frame: &Level2File,
        station_id: &str,
        opacity: f32,
        product: RadarProduct,
        color_table: &crate::render::ColorTable,
        map_view: &MapView,
        selected_elevation: usize,
        smooth: bool,
    ) {
        use crate::render::RadarRenderer;

        let site = match sites::find_site(station_id) { Some(s) => s, None => return };

        let require_sr = product.is_super_res();
        let sweep_idx = frame.sweeps.iter()
            .position(|s| Self::sweep_matches(s, product, require_sr))
            .or_else(|| frame.sweeps.iter().position(|s| Self::sweep_matches(s, product, false)))
            .unwrap_or(selected_elevation.min(frame.sweeps.len().saturating_sub(1)));

        let sweep = match frame.sweeps.get(sweep_idx) { Some(s) => s, None => return };
        let rendered = if smooth {
            RadarRenderer::render_sweep_smooth(sweep, product, site, 1024, color_table)
        } else {
            RadarRenderer::render_sweep_with_table(sweep, product, site, 1024, color_table)
        };
        let rendered = match rendered { Some(r) => r, None => return };

        let cw = canvas_w as f64;
        let ch = canvas_h as f64;
        let range_km = rendered.range_km;

        // Compute radar bounding box on canvas using map projection
        let range_deg_lat = range_km / 111.139;
        let range_deg_lon = range_km / (111.139 * site.lat.to_radians().cos());

        let (cx, cy) = map_view.lat_lon_to_pixel(site.lat, site.lon, cw, ch);
        let (_, north_y) = map_view.lat_lon_to_pixel(site.lat + range_deg_lat, site.lon, cw, ch);
        let (east_x, _) = map_view.lat_lon_to_pixel(site.lat, site.lon + range_deg_lon, cw, ch);

        let half_h = (cy - north_y).abs();
        let half_w = (east_x - cx).abs();

        let x0 = ((cx - half_w) as i32).max(0) as u32;
        let y0 = ((cy - half_h) as i32).max(0) as u32;
        let x1 = ((cx + half_w) as i32).min(canvas_w as i32 - 1) as u32;
        let y1 = ((cy + half_h) as i32).min(canvas_h as i32 - 1) as u32;

        let rw = rendered.width as f64;
        let rh = rendered.height as f64;
        let box_w = half_w * 2.0;
        let box_h = half_h * 2.0;
        if box_w < 1.0 || box_h < 1.0 { return; }

        for py in y0..=y1 {
            for px in x0..=x1 {
                // Map canvas pixel to radar image pixel
                let rx = ((px as f64 - (cx - half_w)) / box_w * rw) as i32;
                let ry = ((py as f64 - (cy - half_h)) / box_h * rh) as i32;
                if rx < 0 || ry < 0 || rx >= rendered.width as i32 || ry >= rendered.height as i32 {
                    continue;
                }
                let src_idx = ((ry as u32 * rendered.width + rx as u32) * 4) as usize;
                if src_idx + 3 >= rendered.pixels.len() { continue; }

                let sa = rendered.pixels[src_idx + 3] as f32 / 255.0 * opacity;
                if sa < 0.01 { continue; }

                let dst_idx = ((py * canvas_w + px) * 4) as usize;
                if dst_idx + 3 >= canvas.len() { continue; }

                for c in 0..3 {
                    let src_c = rendered.pixels[src_idx + c] as f32;
                    let dst_c = canvas[dst_idx + c] as f32;
                    canvas[dst_idx + c] = (src_c * sa + dst_c * (1.0 - sa)) as u8;
                }
                canvas[dst_idx + 3] = ((sa + canvas[dst_idx + 3] as f32 / 255.0 * (1.0 - sa)) * 255.0).min(255.0) as u8;
            }
        }
    }

    /// Estimate storm motion from the current velocity data and update the
    /// storm_motion_dir / storm_motion_speed fields.
    pub fn estimate_storm_motion(&mut self) {
        if let Some(ref file) = self.current_file {
            // Collect all velocity sweeps for multi-elevation estimation
            let vel_sweeps: Vec<&crate::nexrad::level2::Level2Sweep> = file
                .sweeps
                .iter()
                .filter(|s| {
                    s.radials.iter().any(|r| {
                        r.moments
                            .iter()
                            .any(|m| m.product == RadarProduct::Velocity)
                    })
                })
                .collect();
            if !vel_sweeps.is_empty() {
                let (dir, speed) =
                    crate::nexrad::srv::SRVComputer::estimate_storm_motion(&vel_sweeps);
                self.storm_motion_dir = dir;
                self.storm_motion_speed = speed;
            }
        }
    }

    /// Find the best sweep for a given product.
    /// NEXRAD splits products across different sweeps at the same elevation.
    /// For super-res products, only sweeps with azimuth_spacing <= 0.5 are considered.
    pub fn find_sweep_for_product(&self, product: RadarProduct) -> Option<usize> {
        let file = self.current_file.as_ref()?;
        let base = product.base_product();
        let require_super_res = product.is_super_res();

        // First, try to find a sweep at the selected elevation that has this product
        if let Some(sweep) = file.sweeps.get(self.selected_elevation) {
            if Self::sweep_matches(sweep, base, require_super_res) {
                return Some(self.selected_elevation);
            }
        }

        // Otherwise, find the lowest elevation sweep that has this product
        for (i, sweep) in file.sweeps.iter().enumerate() {
            if Self::sweep_matches(sweep, base, require_super_res) {
                return Some(i);
            }
        }
        None
    }

    /// Check if a sweep contains the given product and matches super-res requirements.
    pub fn sweep_matches(sweep: &crate::nexrad::Level2Sweep, product: RadarProduct, require_super_res: bool) -> bool {
        let has_product = sweep.radials.iter().any(|r| {
            r.moments.iter().any(|m| m.product == product)
        });
        if !has_product {
            return false;
        }
        if require_super_res {
            // Super-res: azimuth_spacing must be <= 0.5 degrees
            sweep.radials.first().map_or(false, |r| r.azimuth_spacing <= 0.5)
        } else {
            true
        }
    }

    /// Returns the sweep indices that are valid for the given product.
    /// For super-res products, only sweeps with 0.5° azimuth spacing are included.
    pub fn valid_sweep_indices(&self, product: RadarProduct) -> Vec<usize> {
        let file = match self.current_file.as_ref() {
            Some(f) => f,
            None => return vec![],
        };
        let base = product.base_product();
        let require_super_res = product.is_super_res();
        file.sweeps.iter().enumerate()
            .filter(|(_, sweep)| Self::sweep_matches(sweep, base, require_super_res))
            .map(|(i, _)| i)
            .collect()
    }

    fn check_downloads(&mut self, ctx: &egui::Context) {
        if let Some(data) = self.fetcher.take_downloaded_data() {
            let file_size = data.len();
            let parse_start = Instant::now();
            match Level2File::parse(&data) {
                Ok(file) => {
                    let parse_time = parse_start.elapsed();
                    log::info!(
                        "Parsed Level2 file: station={}, sweeps={}, parse={:.1}ms, size={}KB",
                        file.station_id,
                        file.sweeps.len(),
                        parse_time.as_secs_f64() * 1000.0,
                        file_size / 1024,
                    );

                    // Collect stats
                    let total_radials: usize = file.sweeps.iter().map(|s| s.radials.len()).sum();
                    let total_gates: usize = file.sweeps.iter()
                        .flat_map(|s| s.radials.iter())
                        .flat_map(|r| r.moments.iter())
                        .map(|m| m.gate_count as usize)
                        .sum();

                    self.perf.parse_time = Some(parse_time);
                    self.perf.parse_file_size = file_size;
                    self.perf.total_radials = total_radials;
                    self.perf.total_gates = total_gates;

                    // Run rotation detection on the new file
                    if self.show_detections || self.auto_infer {
                        if let Some(site) = sites::find_site(&self.selected_station) {
                            let (mesos, tvs) = crate::nexrad::detection::RotationDetector::detect(&file, site);
                            log::info!("Detection: {} mesocyclones, {} TVS", mesos.len(), tvs.len());
                            self.meso_detections = mesos;
                            self.tvs_detections = tvs;
                        }
                    }

                    // Push to prediction buffer if prediction or auto-infer is active
                    if self.prediction_active || self.auto_infer {
                        self.prediction_buffer.push_back(file.clone());
                        if self.prediction_buffer.len() > 8 {
                            self.prediction_buffer.pop_front();
                        }
                        if self.prediction_active {
                            self.run_prediction();
                        }
                        if self.auto_infer {
                            self.run_auto_infer_all();
                        }
                    }

                    self.current_file = Some(file);
                    self.selected_elevation = 0;
                    self.needs_render = true;

                    // Auto-estimate storm motion from fresh velocity data
                    self.estimate_storm_motion();

                    // Start background preloading if available_files is populated and no preload in progress
                    if self.preload_rx.is_none() {
                        let files = self.fetcher.available_files.lock().unwrap().clone();
                        if files.len() > 1 {
                            let count = self.preload_count.min(files.len() - 1);
                            let start = files.len() - 1 - count; // exclude the last (already downloaded)
                            let keys: Vec<String> = files[start..files.len() - 1].iter().map(|f| f.key.clone()).collect();
                            log::info!("Starting background preload of {} frames", keys.len());
                            self.preload_keys = keys.clone();
                            self.preloaded_data = vec![None; keys.len()];
                            #[cfg(not(target_arch = "wasm32"))]
                            let preload_fetcher = NexradFetcher::new(self.runtime.handle().clone());
                            let rx = preload_fetcher.download_files_parallel(keys);
                            self.preload_rx = Some(rx);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to parse Level2 file: {}", e);
                }
            }
        }

        // Track download time from fetcher
        if let Some(dl_time) = self.fetcher.take_download_time() {
            self.perf.download_time = Some(dl_time);
        }

        // Auto-load animation after file list arrives (e.g., from historic event)
        if self.pending_auto_anim && !self.fetcher.is_fetching() {
            let files = self.fetcher.available_files.lock().unwrap();
            if !files.is_empty() {
                drop(files);
                self.pending_auto_anim = false;
                self.load_animation_frames();
            }
        }

        ctx.request_repaint();
    }

    /// Poll secondary radar fetchers for downloaded data and parse it.
    fn check_secondary_downloads(&mut self, ctx: &egui::Context) {
        for inst in &mut self.secondary_radars {
            if let Some(data) = inst.fetcher.take_downloaded_data() {
                match Level2File::parse(&data) {
                    Ok(file) => {
                        log::info!(
                            "Secondary radar parsed: station={}, sweeps={}",
                            file.station_id,
                            file.sweeps.len(),
                        );
                        inst.file = Some(file);
                        inst.needs_render = true;
                    }
                    Err(e) => {
                        log::error!("Failed to parse secondary radar {}: {}", inst.station_id, e);
                    }
                }
            }
        }

        // Render textures for secondary radars that need it
        self.render_secondary_radars(ctx);
    }

    fn check_preload_downloads(&mut self) {
        if self.preload_rx.is_none() {
            return;
        }

        let mut processed = 0;
        let total = self.preload_keys.len();
        if let Some(rx) = &mut self.preload_rx {
            while processed < 2 {
                match rx.try_recv() {
                    Ok((idx, data)) => {
                        processed += 1;
                        if idx < self.preloaded_data.len() {
                            log::info!("Preloaded frame {}/{} ({} bytes)",
                                idx + 1, total, data.len());
                            self.preloaded_data[idx] = Some(data);
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        // Check if all preloads are done
        let received = self.preloaded_data.iter().filter(|d| d.is_some()).count();
        if received >= total && total > 0 {
            log::info!("Background preload complete: {}/{} frames cached", received, total);
            self.preload_rx = None;
        }
    }

    /// Compute ideal render resolution based on zoom level.
    /// At low zoom (zoomed out), 1024 is fine. At high zoom (close up), go to 2048 or 4096.
    fn compute_render_resolution(&self) -> u32 {
        let zoom = self.map_view.zoom;
        if zoom >= 11.0 {
            4096
        } else if zoom >= 9.0 {
            2048
        } else {
            1024
        }
    }

    /// Check if zoom has changed enough to warrant a re-render at different resolution.
    fn check_zoom_rerender(&mut self) {
        let new_res = self.compute_render_resolution();
        if new_res != self.render_resolution {
            self.render_resolution = new_res;
            self.last_render_zoom = self.map_view.zoom;
            self.needs_render = true;
            self.mark_all_needs_render();
            log::info!("Dynamic resolution: zoom={:.1} → {}px", self.map_view.zoom, new_res);
        }
    }

    fn render_radar(&mut self, ctx: &egui::Context) {
        if !self.needs_render {
            return;
        }
        self.needs_render = false;

        let render_start = Instant::now();

        let file = match &self.current_file {
            Some(f) => f,
            None => return,
        };

        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        let use_gpu = self.gpu_rendering && self.gpu_renderer.is_some();

        if self.quad_view {
            // Render all 4 quad products — half primary resolution to save VRAM
            let quad_res = (self.render_resolution / 2).max(512);
            for (i, &product) in QUAD_PRODUCTS.iter().enumerate() {
                let sweep_idx = self.find_sweep_for_product(product);
                let sweep = sweep_idx.and_then(|idx| file.sweeps.get(idx));

                if let Some(sweep) = sweep {
                    let t0 = Instant::now();
                    let color_table = self.color_table_manager.resolve(product);
                    let mut rendered = if use_gpu {
                        self.gpu_renderer.as_ref().unwrap()
                            .render_sweep_gpu(sweep, product, site, quad_res, &color_table)
                    } else {
                        None
                    };
                    if rendered.is_none() {
                        rendered = if self.smooth_rendering {
                            RadarRenderer::render_sweep_smooth(sweep, product, site, quad_res, &color_table)
                        } else {
                            RadarRenderer::render_sweep_with_table(sweep, product, site, quad_res, &color_table)
                        };
                    }
                    self.perf.render_quad_times[i] = Some(t0.elapsed());

                    if let Some(rendered) = rendered {
                        self.quad_render_range_km[i] = Some(rendered.range_km);
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [rendered.width as usize, rendered.height as usize],
                            &rendered.pixels,
                        );
                        self.quad_textures[i] = Some(ctx.load_texture(
                            format!("radar_quad_{}", i),
                            image,
                            egui::TextureOptions::NEAREST,
                        ));
                    } else {
                        self.quad_textures[i] = None;
                        self.quad_render_range_km[i] = None;
                    }
                } else {
                    self.quad_textures[i] = None;
                    self.perf.render_quad_times[i] = None;
                }
            }
        }

        // Always render the selected single product too (for single view / overlay)
        // Handle derived products that need special computation
        let derived_sweep;
        let (render_product, render_sweep) = match self.selected_product {
            RadarProduct::VIL => {
                derived_sweep = crate::nexrad::derived::DerivedProducts::compute_vil(file);
                (RadarProduct::VIL, Some(&derived_sweep))
            }
            RadarProduct::EchoTops => {
                derived_sweep = crate::nexrad::derived::DerivedProducts::compute_echo_tops(file, 18.0);
                (RadarProduct::EchoTops, Some(&derived_sweep))
            }
            RadarProduct::StormRelativeVelocity => {
                let vel_idx = self.find_sweep_for_product(RadarProduct::Velocity);
                if let Some(idx) = vel_idx {
                    if let Some(vel_sweep) = file.sweeps.get(idx) {
                        derived_sweep = crate::nexrad::srv::SRVComputer::compute(
                            vel_sweep, self.storm_motion_dir, self.storm_motion_speed,
                        );
                        (RadarProduct::StormRelativeVelocity, Some(&derived_sweep))
                    } else {
                        (self.selected_product, None)
                    }
                } else {
                    (self.selected_product, None)
                }
            }
            _ => {
                let sweep_idx = self.find_sweep_for_product(self.selected_product)
                    .unwrap_or(self.selected_elevation);
                // For super-res products, use the base product for moment lookup
                let render_prod = self.selected_product.base_product();
                (render_prod, file.sweeps.get(sweep_idx))
            }
        };

        if let Some(sweep) = render_sweep {
            let res = self.render_resolution;
            let color_table = self.color_table_manager.resolve(render_product);
            let mut rendered = if use_gpu {
                self.gpu_renderer.as_ref().unwrap()
                    .render_sweep_gpu(sweep, render_product, site, res, &color_table)
            } else {
                None
            };
            // Fall back to CPU if GPU returned None (empty output or error)
            if rendered.is_none() {
                rendered = if self.smooth_rendering {
                    RadarRenderer::render_sweep_smooth(sweep, render_product, site, res, &color_table)
                } else {
                    RadarRenderer::render_sweep_with_table(sweep, render_product, site, res, &color_table)
                };
            }
            if let Some(rendered) = rendered {
                self.last_render_range_km = Some(rendered.range_km);
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [rendered.width as usize, rendered.height as usize],
                    &rendered.pixels,
                );
                self.single_texture = Some(ctx.load_texture(
                    "radar_single",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            } else {
                self.single_texture = None;
            }
        }
        // Render dual pane product (right pane) if dual_pane is active
        if self.dual_pane {
            let dp = self.dual_pane_product;
            let dp_derived_sweep;
            let (dp_product, dp_sweep) = match dp {
                RadarProduct::VIL => {
                    dp_derived_sweep = crate::nexrad::derived::DerivedProducts::compute_vil(file);
                    (RadarProduct::VIL, Some(&dp_derived_sweep))
                }
                RadarProduct::EchoTops => {
                    dp_derived_sweep = crate::nexrad::derived::DerivedProducts::compute_echo_tops(file, 18.0);
                    (RadarProduct::EchoTops, Some(&dp_derived_sweep))
                }
                RadarProduct::StormRelativeVelocity => {
                    let vel_idx = self.find_sweep_for_product(RadarProduct::Velocity);
                    if let Some(idx) = vel_idx {
                        if let Some(vel_sweep) = file.sweeps.get(idx) {
                            dp_derived_sweep = crate::nexrad::srv::SRVComputer::compute(
                                vel_sweep, self.storm_motion_dir, self.storm_motion_speed,
                            );
                            (RadarProduct::StormRelativeVelocity, Some(&dp_derived_sweep))
                        } else {
                            (dp, None)
                        }
                    } else {
                        (dp, None)
                    }
                }
                _ => {
                    let sweep_idx = self.find_sweep_for_product(dp)
                        .unwrap_or(self.selected_elevation);
                    (dp.base_product(), file.sweeps.get(sweep_idx))
                }
            };

            if let Some(sweep) = dp_sweep {
                let color_table = self.color_table_manager.resolve(dp_product);
                let dp_res = self.render_resolution;
                let mut rendered = if use_gpu {
                    self.gpu_renderer.as_ref().unwrap()
                        .render_sweep_gpu(sweep, dp_product, site, dp_res, &color_table)
                } else {
                    None
                };
                if rendered.is_none() {
                    rendered = if self.smooth_rendering {
                        RadarRenderer::render_sweep_smooth(sweep, dp_product, site, dp_res, &color_table)
                    } else {
                        RadarRenderer::render_sweep_with_table(sweep, dp_product, site, dp_res, &color_table)
                    };
                }
                if let Some(rendered) = rendered {
                    self.dual_pane_range_km = Some(rendered.range_km);
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [rendered.width as usize, rendered.height as usize],
                        &rendered.pixels,
                    );
                    self.dual_pane_texture = Some(ctx.load_texture(
                        "radar_dual_pane",
                        image,
                        egui::TextureOptions::NEAREST,
                    ));
                } else {
                    self.dual_pane_texture = None;
                    self.dual_pane_range_km = None;
                }
            } else {
                self.dual_pane_texture = None;
                self.dual_pane_range_km = None;
            }
        }

        let render_method = if use_gpu { "GPU" } else { "CPU" };
        self.perf.render_time = Some(render_start.elapsed());
        log::info!("Render total ({}): {:.1}ms", render_method, render_start.elapsed().as_secs_f64() * 1000.0);
    }

    /// Render textures for secondary radars that have new data.
    fn render_secondary_radars(&mut self, ctx: &egui::Context) {
        let product = self.selected_product.base_product();
        let color_table = self.color_table_manager.resolve(product);
        let use_gpu = self.gpu_rendering && self.gpu_renderer.is_some();
        // Scale secondary radar resolution with primary; halve in mosaic mode to save VRAM
        let render_size: u32 = if self.mosaic_mode {
            (self.render_resolution / 2).max(512)
        } else {
            self.render_resolution
        };

        for inst in &mut self.secondary_radars {
            if !inst.needs_render {
                continue;
            }
            inst.needs_render = false;

            let file = match &inst.file {
                Some(f) => f,
                None => continue,
            };
            let site = match sites::find_site(&inst.station_id) {
                Some(s) => s,
                None => continue,
            };

            // Find the best sweep: try selected_elevation first, then fallback
            let base = product;
            let require_sr = self.selected_product.is_super_res();
            let sweep_idx = if let Some(sweep) = file.sweeps.get(self.selected_elevation) {
                if Self::sweep_matches(sweep, base, require_sr) || Self::sweep_matches(sweep, base, false) {
                    self.selected_elevation
                } else {
                    file.sweeps.iter().position(|s| Self::sweep_matches(s, base, require_sr))
                        .or_else(|| file.sweeps.iter().position(|s| Self::sweep_matches(s, base, false)))
                        .unwrap_or(0)
                }
            } else {
                file.sweeps.iter().position(|s| Self::sweep_matches(s, base, require_sr))
                    .or_else(|| file.sweeps.iter().position(|s| Self::sweep_matches(s, base, false)))
                    .unwrap_or(0)
            };

            if let Some(sweep) = file.sweeps.get(sweep_idx) {
                let mut rendered = if use_gpu {
                    self.gpu_renderer.as_ref().unwrap()
                        .render_sweep_gpu(sweep, product, site, render_size, &color_table)
                } else {
                    None
                };
                if rendered.is_none() {
                    rendered = if self.smooth_rendering {
                        RadarRenderer::render_sweep_smooth(sweep, product, site, render_size, &color_table)
                    } else {
                        RadarRenderer::render_sweep_with_table(sweep, product, site, render_size, &color_table)
                    };
                }
                if let Some(rendered) = rendered {
                    inst.range_km = Some(rendered.range_km);
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [rendered.width as usize, rendered.height as usize],
                        &rendered.pixels,
                    );
                    inst.texture = Some(ctx.load_texture(
                        format!("radar_secondary_{}", inst.station_id),
                        image,
                        egui::TextureOptions::NEAREST,
                    ));
                } else {
                    inst.texture = None;
                    inst.range_km = None;
                }
            }
        }
    }

    fn draw_map(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let visible = self.map_view.visible_tiles(screen_w, screen_h);
        for key in &visible {
            self.tile_manager.request_tile(*key);

            if let Some(tile_data) = self.tile_manager.get_tile(key) {
                let tex = self.tile_textures.entry(*key).or_insert_with(|| {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [tile_data.width as usize, tile_data.height as usize],
                        &tile_data.pixels,
                    );
                    ui.ctx().load_texture(
                        format!("tile_{}_{}_{}", key.z, key.x, key.y),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                });

                let (px, py) = self.map_view.tile_screen_pos(key, screen_w, screen_h);
                let tile_size = self.map_view.tile_size_on_screen(key.z) as f32;

                let tile_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + px as f32, rect.top() + py as f32),
                    egui::vec2(tile_size, tile_size),
                );

                if tile_rect.intersects(rect) {
                    ui.painter().image(
                        tex.id(),
                        tile_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::from_white_alpha((self.map_opacity * 255.0) as u8),
                    );
                }
            }
        }

        // Prefetch tiles for expanded viewport and one zoom level up
        let prefetch = self.map_view.prefetch_tiles(screen_w, screen_h);
        for key in &prefetch {
            self.tile_manager.request_tile(*key);
        }

        // Retain textures for both visible and prefetched tiles
        let retain_set: std::collections::HashSet<_> = visible.iter().chain(prefetch.iter()).copied().collect();
        self.tile_textures.retain(|k, _| retain_set.contains(k));
    }

    fn get_radar_rect_for_range(&self, rect: egui::Rect, range_km: f64) -> Option<egui::Rect> {
        let site = sites::find_site(&self.selected_station)?;
        self.get_radar_rect_for_site(rect, range_km, site)
    }

    /// Compute the screen rect for a radar overlay centered on the given site.
    /// The rect is centered on the station's Mercator-projected position so
    /// that the radar image center (which represents the station) aligns
    /// correctly with other Mercator-projected overlays (warnings, map tiles).
    fn get_radar_rect_for_site(&self, rect: egui::Rect, range_km: f64, site: &crate::nexrad::RadarSite) -> Option<egui::Rect> {
        let max_range_m = range_km * 1000.0;

        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        // Project the station center to screen coordinates
        let (cx, cy) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);

        // Compute pixel extents by projecting the range edges and measuring
        // the distance from center. Use the maximum extent in each direction
        // to keep the radar image square in screen space.
        let range_deg_lat = max_range_m / 111139.0;
        let range_deg_lon = max_range_m / (111139.0 * site.lat.to_radians().cos());

        let (_, north_y) = self.map_view.lat_lon_to_pixel(
            site.lat + range_deg_lat, site.lon,
            screen_w, screen_h,
        );
        let (east_x, _) = self.map_view.lat_lon_to_pixel(
            site.lat, site.lon + range_deg_lon,
            screen_w, screen_h,
        );

        let half_h = (cy - north_y).abs();
        let half_w = (east_x - cx).abs();

        Some(egui::Rect::from_min_max(
            egui::pos2(rect.left() + (cx - half_w) as f32, rect.top() + (cy - half_h) as f32),
            egui::pos2(rect.left() + (cx + half_w) as f32, rect.top() + (cy + half_h) as f32),
        ))
    }

    fn draw_radar_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let opacity = egui::Color32::from_white_alpha((self.radar_opacity * 255.0) as u8);
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        // Draw secondary radars first (underneath the primary)
        for inst in &self.secondary_radars {
            let tex = match &inst.texture {
                Some(t) => t,
                None => continue,
            };
            let site = match sites::find_site(&inst.station_id) {
                Some(s) => s,
                None => continue,
            };
            let radar_rect = match inst.range_km.and_then(|r| self.get_radar_rect_for_site(rect, r, site)) {
                Some(r) => r,
                None => continue,
            };
            let sec_opacity = egui::Color32::from_white_alpha((inst.opacity * 255.0) as u8);
            ui.painter().image(tex.id(), radar_rect, uv, sec_opacity);

            // Draw station marker for secondary radar
            let (cx, cy) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
            let marker_pos = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);
            let secondary_color = egui::Color32::from_rgb(100, 200, 255);
            ui.painter().circle_filled(marker_pos, 4.0, secondary_color);
            ui.painter().circle_stroke(marker_pos, 4.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
            ui.painter().text(
                marker_pos + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                &inst.station_id,
                egui::FontId::proportional(12.0),
                secondary_color,
            );
        }

        // Draw primary radar
        if let Some(tex) = &self.single_texture {
            let radar_rect = match self.last_render_range_km.and_then(|r| self.get_radar_rect_for_range(rect, r)) {
                Some(r) => r,
                None => return,
            };

            ui.painter().image(tex.id(), radar_rect, uv, opacity);

            // Draw radar site marker for primary
            if let Some(site) = sites::find_site(&self.selected_station) {
                let (cx, cy) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
                let marker_pos = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);
                ui.painter().circle_filled(marker_pos, 4.0, egui::Color32::WHITE);
                ui.painter().circle_stroke(marker_pos, 4.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
                ui.painter().text(
                    marker_pos + egui::vec2(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    &self.selected_station,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }
    }

    /// Draw a timestamp overlay in the top-right corner of the given rect.
    /// Shows station ID, elevation, product, and volume scan time.
    fn draw_timestamp_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        // During animation use the current animation frame, otherwise use current_file.
        let file = if self.anim_playing && !self.anim_frames.is_empty() {
            Some(&self.anim_frames[self.anim_index])
        } else {
            self.current_file.as_ref()
        };
        let file = match file {
            Some(f) => f,
            None => return,
        };

        // Build the overlay text lines
        let elevation_angle = file
            .sweeps
            .get(self.selected_elevation)
            .map(|s| s.elevation_angle)
            .unwrap_or(0.0);
        let line1 = format!(
            "{} {:.1}\u{00b0} {}",
            file.station_id,
            elevation_angle,
            self.selected_product.short_name(),
        );
        let line2 = file.timestamp_string();
        let text = format!("{}\n{}", line1, line2);

        let font = egui::FontId::proportional(14.0);
        let galley = ui.painter().layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
        let text_size = galley.size();

        let padding = egui::vec2(8.0, 6.0);
        let bg_size = text_size + padding * 2.0;
        let bg_pos = egui::pos2(rect.right() - bg_size.x - 6.0, rect.top() + 6.0);
        let bg_rect = egui::Rect::from_min_size(bg_pos, bg_size);

        // Semi-transparent dark background
        ui.painter().rect_filled(bg_rect, 4.0, egui::Color32::from_black_alpha(180));

        // White text
        ui.painter().text(
            bg_rect.min + padding,
            egui::Align2::LEFT_TOP,
            text,
            font,
            egui::Color32::WHITE,
        );
    }

    fn draw_quad_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let half_w = rect.width() / 2.0;
        let half_h = rect.height() / 2.0;

        let rects = [
            egui::Rect::from_min_size(rect.min, egui::vec2(half_w, half_h)),
            egui::Rect::from_min_size(egui::pos2(rect.left() + half_w, rect.top()), egui::vec2(half_w, half_h)),
            egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + half_h), egui::vec2(half_w, half_h)),
            egui::Rect::from_min_size(egui::pos2(rect.left() + half_w, rect.top() + half_h), egui::vec2(half_w, half_h)),
        ];

        for (i, &product) in QUAD_PRODUCTS.iter().enumerate() {
            let quad_rect = rects[i];

            // Draw map tiles in this quadrant
            self.draw_map_in_rect(ui, quad_rect);

            // Draw radar overlay (clipped to quadrant)
            if let Some(tex) = &self.quad_textures[i] {
                if let Some(radar_rect) = self.quad_render_range_km[i].and_then(|r| self.get_radar_rect_for_range(quad_rect, r)) {
                    let clipped = ui.painter_at(quad_rect);
                    clipped.image(
                        tex.id(),
                        radar_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::from_white_alpha((self.radar_opacity * 255.0) as u8),
                    );
                }
            }

            // Overlays (range rings, site marker, warnings, etc.)
            self.draw_overlays_in_rect(ui, quad_rect);

            // Label
            ui.painter().rect_filled(
                egui::Rect::from_min_size(quad_rect.min, egui::vec2(80.0, 22.0)),
                4.0,
                egui::Color32::from_black_alpha(180),
            );
            ui.painter().text(
                quad_rect.min + egui::vec2(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                product.short_name(),
                egui::FontId::proportional(14.0),
                egui::Color32::WHITE,
            );

            // Border between quadrants
            ui.painter().rect_stroke(quad_rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_gray(60)), egui::StrokeKind::Outside);
        }
    }


    fn draw_dual_pane(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let half_w = rect.width() / 2.0;

        let left_rect = egui::Rect::from_min_size(rect.min, egui::vec2(half_w, rect.height()));
        let right_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + half_w, rect.top()),
            egui::vec2(rect.width() - half_w, rect.height()),
        );

        // Ensure map tiles are loaded for the half-pane size
        let screen_w = half_w as f64;
        let screen_h = rect.height() as f64;
        let visible = self.map_view.visible_tiles(screen_w, screen_h);
        for key in &visible {
            self.tile_manager.request_tile(*key);
            if let Some(tile_data) = self.tile_manager.get_tile(key) {
                self.tile_textures.entry(*key).or_insert_with(|| {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [tile_data.width as usize, tile_data.height as usize],
                        &tile_data.pixels,
                    );
                    ui.ctx().load_texture(
                        format!("tile_{}_{}_{}", key.z, key.x, key.y),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                });
            }
        }
        let prefetch = self.map_view.prefetch_tiles(screen_w, screen_h);
        for key in &prefetch {
            self.tile_manager.request_tile(*key);
        }

        // -- Left pane: selected_product --
        self.draw_map_in_rect(ui, left_rect);
        // Draw radar overlay (left = single_texture)
        if let Some(tex) = &self.single_texture {
            if let Some(radar_rect) = self.last_render_range_km.and_then(|r| self.get_radar_rect_for_range(left_rect, r)) {
                let clipped = ui.painter_at(left_rect);
                clipped.image(
                    tex.id(),
                    radar_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha((self.radar_opacity * 255.0) as u8),
                );
            }
        }
        // Overlays on left pane
        self.draw_overlays_in_rect(ui, left_rect);
        self.draw_timestamp_overlay(ui, left_rect);
        // Label
        ui.painter().rect_filled(
            egui::Rect::from_min_size(left_rect.min, egui::vec2(80.0, 22.0)),
            4.0,
            egui::Color32::from_black_alpha(180),
        );
        ui.painter().text(
            left_rect.min + egui::vec2(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            self.selected_product.short_name(),
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );

        // -- Right pane: dual_pane_product --
        self.draw_map_in_rect(ui, right_rect);
        // Draw radar overlay (right = dual_pane_texture)
        if let Some(tex) = &self.dual_pane_texture {
            if let Some(radar_rect) = self.dual_pane_range_km.and_then(|r| self.get_radar_rect_for_range(right_rect, r)) {
                let clipped = ui.painter_at(right_rect);
                clipped.image(
                    tex.id(),
                    radar_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha((self.radar_opacity * 255.0) as u8),
                );
            }
        }
        // Overlays on right pane
        self.draw_overlays_in_rect(ui, right_rect);
        self.draw_timestamp_overlay(ui, right_rect);
        // Label
        ui.painter().rect_filled(
            egui::Rect::from_min_size(right_rect.min, egui::vec2(80.0, 22.0)),
            4.0,
            egui::Color32::from_black_alpha(180),
        );
        ui.painter().text(
            right_rect.min + egui::vec2(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            self.dual_pane_product.short_name(),
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );

        // Divider line
        ui.painter().line_segment(
            [egui::pos2(left_rect.right(), rect.top()), egui::pos2(left_rect.right(), rect.bottom())],
            egui::Stroke::new(1.5, egui::Color32::from_gray(80)),
        );
    }

    fn draw_overlays_in_rect(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };
        let clipped = ui.painter_at(rect);

        if self.show_range_rings {
            crate::render::overlays::MapOverlays::draw_range_rings(
                &clipped, site.lat, site.lon, &self.map_view, rect,
            );
        }
        if self.show_azimuth_lines {
            let max_range = self.last_render_range_km.unwrap_or(230.0);
            crate::render::overlays::MapOverlays::draw_azimuth_lines(
                &clipped, site.lat, site.lon, &self.map_view, rect, max_range,
            );
        }
        crate::render::overlays::MapOverlays::draw_site_marker(
            &clipped, site.lat, site.lon, &self.selected_station, &self.map_view, rect,
        );
        if self.show_warnings {
            let alerts = self.alert_fetcher.get_alerts();
            crate::render::warnings::WarningRenderer::draw_warnings_with_opacity(
                &alerts, &clipped, &self.map_view, rect, self.warning_opacity,
            );
        }
    }

    fn draw_map_in_rect(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        // Clip to quadrant bounds so tiles don't bleed across quadrants
        let painter = ui.painter_at(rect);

        let visible = self.map_view.visible_tiles(screen_w, screen_h);
        for key in &visible {
            if let Some(tex) = self.tile_textures.get(key) {
                let (px, py) = self.map_view.tile_screen_pos(key, screen_w, screen_h);
                let tile_size = self.map_view.tile_size_on_screen(key.z) as f32;

                let tile_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + px as f32, rect.top() + py as f32),
                    egui::vec2(tile_size, tile_size),
                );

                if tile_rect.intersects(rect) {
                    painter.image(
                        tex.id(),
                        tile_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::from_white_alpha((self.map_opacity * 255.0) as u8),
                    );
                }
            }
        }
    }

    fn draw_radar_sites(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        if self.map_view.zoom < 5.0 {
            return;
        }

        // Get cursor position for hover highlight
        let cursor_screen = ui.ctx().input(|i| i.pointer.hover_pos());

        for site in sites::RADAR_SITES.iter() {
            let (px, py) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
            let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);

            if !rect.contains(pos) {
                continue;
            }

            let is_selected = site.id == self.selected_station;
            let is_secondary = self.secondary_radars.iter().any(|r| r.station_id == site.id);
            let is_hovered = cursor_screen
                .map(|c| c.distance(pos) < 15.0)
                .unwrap_or(false);

            let color = if is_selected {
                egui::Color32::from_rgb(255, 255, 0)
            } else if is_secondary {
                egui::Color32::from_rgb(100, 200, 255)
            } else if is_hovered {
                egui::Color32::from_rgb(100, 200, 255)
            } else {
                egui::Color32::from_rgb(200, 200, 200)
            };

            let radius = if is_selected {
                6.0
            } else if is_secondary {
                5.5
            } else if is_hovered {
                5.5
            } else {
                4.0
            };

            // Draw outer glow ring on hover for clickability hint
            if is_hovered && !is_selected {
                ui.painter().circle_stroke(
                    pos,
                    8.0,
                    egui::Stroke::new(1.5, egui::Color32::from_rgba_premultiplied(100, 200, 255, 120)),
                );
            }

            ui.painter().circle_filled(pos, radius, color);

            if self.map_view.zoom >= 7.0 || is_selected || is_secondary || is_hovered {
                let label_color = if is_hovered && !is_selected && !is_secondary {
                    egui::Color32::from_rgb(100, 200, 255)
                } else {
                    color
                };
                ui.painter().text(
                    pos + egui::vec2(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    site.id,
                    egui::FontId::proportional(if is_hovered || is_secondary { 12.0 } else { 10.0 }),
                    label_color,
                );
            }
        }
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let products = RadarProduct::all_products();

        let mut toggle_help = false;
        let mut toggle_range_rings = false;
        let mut toggle_detections = false;
        let mut toggle_sounding = false;
        let mut toggle_hrrr = false;
        let mut toggle_measure = false;
        let mut fetch_latest = false;
        let mut zoom_in = false;
        let mut zoom_out = false;
        let mut num_product: Option<usize> = None;

        ctx.input(|i| {
            // Up/Down arrows: tilt up/down (syncs all radars)
            if i.key_pressed(egui::Key::ArrowUp) {
                if let Some(ref file) = self.current_file {
                    if self.selected_elevation + 1 < file.sweeps.len() {
                        self.selected_elevation += 1;
                        self.mark_all_needs_render();
                    }
                }
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                if self.selected_elevation > 0 {
                    self.selected_elevation -= 1;
                    self.mark_all_needs_render();
                }
            }

            // Left/Right arrows: cycle products
            if i.key_pressed(egui::Key::ArrowRight) {
                if let Some(idx) = products.iter().position(|&p| p == self.selected_product) {
                    let next = (idx + 1) % products.len();
                    let new_product = products[next];
                    if new_product == RadarProduct::StormRelativeVelocity
                        && self.selected_product != RadarProduct::StormRelativeVelocity
                    {
                        self.estimate_storm_motion();
                    }
                    self.selected_product = new_product;
                    self.mark_all_needs_render();
                }
            }
            if i.key_pressed(egui::Key::ArrowLeft) {
                if let Some(idx) = products.iter().position(|&p| p == self.selected_product) {
                    let prev = if idx == 0 { products.len() - 1 } else { idx - 1 };
                    let new_product = products[prev];
                    if new_product == RadarProduct::StormRelativeVelocity
                        && self.selected_product != RadarProduct::StormRelativeVelocity
                    {
                        self.estimate_storm_motion();
                    }
                    self.selected_product = new_product;
                    self.mark_all_needs_render();
                }
            }

            // Q: toggle quad view
            if i.key_pressed(egui::Key::Q) {
                self.quad_view = !self.quad_view;
                if self.quad_view {
                    self.dual_pane = false; // mutually exclusive
                }
                self.needs_render = true;
            }

            // W: toggle wall mode
            if i.key_pressed(egui::Key::W) {
                if self.wall_mode {
                    self.wall_mode = false;
                } else {
                    self.start_wall_mode();
                }
            }

            // M: toggle mosaic mode
            if i.key_pressed(egui::Key::M) {
                if self.mosaic_mode {
                    self.deactivate_mosaic_mode();
                } else {
                    self.activate_mosaic_mode();
                }
            }

            // Space: play/pause animation
            if i.key_pressed(egui::Key::Space) {
                if !self.anim_frames.is_empty() {
                    self.anim_playing = !self.anim_playing;
                    if self.anim_playing {
                        self.anim_last_advance = Some(Instant::now());
                    }
                }
            }

            // Comma/Period: step backward/forward through animation frames
            if i.key_pressed(egui::Key::Period) && !self.anim_frames.is_empty() {
                self.anim_playing = false;
                self.anim_index = (self.anim_index + 1) % self.anim_frames.len();
                self.current_file = Some(self.anim_frames[self.anim_index].clone());
                self.needs_render = true;
            }
            if i.key_pressed(egui::Key::Comma) && !self.anim_frames.is_empty() {
                self.anim_playing = false;
                self.anim_index = if self.anim_index == 0 { self.anim_frames.len() - 1 } else { self.anim_index - 1 };
                self.current_file = Some(self.anim_frames[self.anim_index].clone());
                self.needs_render = true;
            }

            // M: toggle measure mode
            if i.key_pressed(egui::Key::M) {
                toggle_measure = true;
            }

            // H / F1: toggle help overlay
            if i.key_pressed(egui::Key::H) || i.key_pressed(egui::Key::F1) {
                toggle_help = true;
            }

            // R: toggle range rings
            if i.key_pressed(egui::Key::R) {
                toggle_range_rings = true;
            }

            // D: toggle meso/TVS detection
            if i.key_pressed(egui::Key::D) {
                toggle_detections = true;
            }

            // S: toggle sounding mode
            if i.key_pressed(egui::Key::S) {
                toggle_sounding = true;
            }

            // Y: toggle HRRR model overlay
            if i.key_pressed(egui::Key::Y) {
                toggle_hrrr = true;
            }

            // +/=: zoom in, -: zoom out
            if i.key_pressed(egui::Key::Plus) {
                zoom_in = true;
            }
            if i.key_pressed(egui::Key::Minus) {
                zoom_out = true;
            }

            // L: load latest data
            if i.key_pressed(egui::Key::L) {
                fetch_latest = true;
            }

            // 1-9: select product directly
            let num_keys = [
                egui::Key::Num1, egui::Key::Num2, egui::Key::Num3,
                egui::Key::Num4, egui::Key::Num5, egui::Key::Num6,
                egui::Key::Num7, egui::Key::Num8, egui::Key::Num9,
            ];
            for (idx, key) in num_keys.iter().enumerate() {
                if i.key_pressed(*key) {
                    num_product = Some(idx);
                }
            }
        });

        // Apply state changes outside ctx.input closure
        if toggle_help {
            self.show_help = !self.show_help;
        }
        if toggle_range_rings {
            self.show_range_rings = !self.show_range_rings;
        }
        if toggle_detections {
            self.show_detections = !self.show_detections;
        }
        if toggle_sounding {
            self.sounding_mode = !self.sounding_mode;
        }
        if toggle_hrrr {
            self.hrrr_mode = !self.hrrr_mode;
            if self.hrrr_mode {
                self.sounding_mode = false;
            }
        }
        if toggle_measure {
            self.measure_mode = !self.measure_mode;
            if self.measure_mode {
                self.measure_start = None;
                self.measure_end = None;
            }
        }
        if zoom_in {
            self.map_view.zoom = (self.map_view.zoom + 0.5).min(18.0);
            self.needs_render = true;
        }
        if zoom_out {
            self.map_view.zoom = (self.map_view.zoom - 0.5).max(2.0);
            self.needs_render = true;
        }
        if fetch_latest {
            self.fetch_latest();
        }
        // Number key product selection: 1=REF, 2=VEL, 3=SW, 4=ZDR, 5=CC, 6=KDP, 7=SRV
        if let Some(idx) = num_product {
            let products = RadarProduct::all_products();
            if idx < products.len() {
                let new_product = products[idx];
                // Auto-estimate storm motion when switching to SRV
                if new_product == RadarProduct::StormRelativeVelocity
                    && self.selected_product != RadarProduct::StormRelativeVelocity
                {
                    self.estimate_storm_motion();
                }
                self.selected_product = new_product;
                self.mark_all_needs_render();
            }
        }
    }

    fn handle_interaction(&mut self, response: &egui::Response, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        if let Some(pos) = response.hover_pos() {
            let rel_x = (pos.x - rect.left()) as f64;
            let rel_y = (pos.y - rect.top()) as f64;
            let (lat, lon) = self.map_view.pixel_to_lat_lon(rel_x, rel_y, screen_w, screen_h);
            self.cursor_lat = lat;
            self.cursor_lon = lon;
        }

        let scroll = response.ctx.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 && response.hovered() {
            if let Some(pos) = response.hover_pos() {
                let rel_x = (pos.x - rect.left()) as f64;
                let rel_y = (pos.y - rect.top()) as f64;
                let delta = scroll as f64 * 0.003;
                self.map_view.zoom_at(delta, rel_x, rel_y, screen_w, screen_h);
            }
        }

        if response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            self.map_view.pan(delta.x as f64, delta.y as f64, screen_w, screen_h);
        }

        // Ctrl+Click: set tornado prediction target
        // Use button_released for reliability; fall back to latest_pos if hover_pos is None
        let ctrl_clicked = response.ctx.input(|i| {
            i.modifiers.ctrl && !i.modifiers.alt &&
            i.pointer.button_released(egui::PointerButton::Primary)
        });
        let ctrl_click_pos = if ctrl_clicked && response.hovered() {
            response.hover_pos().or_else(|| response.ctx.input(|i| i.pointer.latest_pos()))
        } else {
            None
        };
        if let Some(pos) = ctrl_click_pos {
            if rect.contains(pos) {
                let click_x = (pos.x - rect.left()) as f64;
                let click_y = (pos.y - rect.top()) as f64;
                let (lat, lon) = self.map_view.pixel_to_lat_lon(click_x, click_y, screen_w, screen_h);
                log::info!("Prediction target set: ({:.4}, {:.4})", lat, lon);
                self.prediction_target = Some((lat, lon));
                self.prediction_active = true;
                self.prediction_result = None;
                self.prediction_history.clear();

                // If buffer already has frames (from auto-infer or previous target),
                // just re-run prediction at the new location without re-downloading
                if self.prediction_buffer.len() >= 3 {
                    self.prediction_last_run = None; // bypass throttle for explicit click
                    self.run_prediction();
                } else {
                    self.prediction_buffer.clear();
                    if let Some(ref file) = self.current_file {
                        self.prediction_buffer.push_back(file.clone());
                    }
                    self.start_prediction_backfill();
                }
            }
        }

        // For tool modes (cross-section, measure, sounding), use button_released
        // instead of response.clicked() so that slight mouse movement during click
        // doesn't prevent the tool from registering (common issue with map panning).
        let tool_mode_active = self.cross_section_mode || self.measure_mode || self.sounding_mode;
        let tool_clicked = if tool_mode_active {
            let released = response.ctx.input(|i| {
                !i.modifiers.ctrl && i.pointer.button_released(egui::PointerButton::Primary)
            });
            released && response.hovered()
        } else {
            false
        };

        if tool_clicked {
            let pos = response.hover_pos()
                .or_else(|| response.ctx.input(|i| i.pointer.latest_pos()));
            if let Some(pos) = pos {
                if rect.contains(pos) {
                    let click_x = (pos.x - rect.left()) as f64;
                    let click_y = (pos.y - rect.top()) as f64;
                    let (lat, lon) = self.map_view.pixel_to_lat_lon(click_x, click_y, screen_w, screen_h);

                    if self.measure_mode {
                        if self.measure_start.is_none() {
                            self.measure_start = Some((lat, lon));
                        } else {
                            self.measure_end = Some((lat, lon));
                            self.measure_mode = false;
                        }
                    } else if self.cross_section_mode {
                        if self.cross_section_start.is_none() {
                            self.cross_section_start = Some((lat, lon));
                        } else {
                            self.cross_section_end = Some((lat, lon));
                            self.cross_section_mode = false;
                            self.render_cross_section_image();
                        }
                    } else if self.sounding_mode {
                        self.sounding_fetcher.fetch_sounding(lat, lon);
                        self.sounding_mode = false;
                        self.sounding_texture = None;
                        self.sounding_pending = true;
                    }
                }
            }
        } else if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let click_x = (pos.x - rect.left()) as f64;
                let click_y = (pos.y - rect.top()) as f64;
                let (lat, lon) = self.map_view.pixel_to_lat_lon(click_x, click_y, screen_w, screen_h);

                if false { // tool modes handled above
                } else {
                    let multi_select = response.ctx.input(|i| i.modifiers.shift || i.modifiers.ctrl);
                    for site in sites::RADAR_SITES.iter() {
                        let (sx, sy) = self.map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
                        let dist = ((click_x - sx).powi(2) + (click_y - sy).powi(2)).sqrt();
                        if dist < 15.0 {
                            if multi_select {
                                // Ctrl+Click or Shift+Click: toggle as secondary radar
                                if self.secondary_radars.iter().any(|r| r.station_id == site.id) {
                                    self.remove_secondary_radar(site.id);
                                } else {
                                    self.add_secondary_radar(site.id);
                                }
                            } else {
                                self.select_station(site.id);
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    fn render_cross_section_image(&mut self) {
        let start = match self.cross_section_start {
            Some(s) => s,
            None => return,
        };
        let end = match self.cross_section_end {
            Some(e) => e,
            None => return,
        };
        let file = match &self.current_file {
            Some(f) => f,
            None => return,
        };
        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        let render_prod = self.selected_product.base_product();
        let color_table = self.color_table_manager.resolve(render_prod);
        let result = if self.cross_section_3d {
            crate::render::CrossSectionRenderer::render_cross_section_3d(
                file, render_prod, &color_table, site, start, end, 800, 400,
                self.cross_section_view_angle, self.cross_section_view_pitch,
            )
        } else {
            crate::render::CrossSectionRenderer::render_cross_section(
                file, render_prod, &color_table, site, start, end, 800, 300,
            )
        };

        // Texture will be created in the update loop since we need ctx
        // Store the result temporarily
        if let Some(res) = result {
            self.cross_section_result = Some(res);
        }
    }

    fn draw_cursor_readout(&self, ui: &mut egui::Ui, rect: egui::Rect, product: RadarProduct) {
        // Need cursor position, radar data, and site info
        let file = match &self.current_file {
            Some(f) => f,
            None => return,
        };
        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        // Check if cursor is within the rect
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;
        let (cx, cy) = self.map_view.lat_lon_to_pixel(self.cursor_lat, self.cursor_lon, screen_w, screen_h);
        let cursor_screen = egui::pos2(rect.left() + cx as f32, rect.top() + cy as f32);
        if !rect.contains(cursor_screen) {
            return;
        }

        // For super-res variants, use the base product for moment data lookup
        let lookup_product = product.base_product();

        // Find the sweep for this product (respects super-res filtering)
        let sweep_idx = match self.find_sweep_for_product_in(file, product) {
            Some(i) => i,
            None => return,
        };

        // Look up the data
        let readout = match crate::render::data_readout::lookup_cursor_data(
            self.cursor_lat, self.cursor_lon,
            site.lat, site.lon,
            file, sweep_idx, lookup_product,
        ) {
            Some(r) => r,
            None => return,
        };

        // Format the readout lines
        let value_str = crate::render::data_readout::format_value(readout.value, readout.product);
        let lines = [
            value_str,
            format!("{:.1} km  {:.1}\u{00B0}", readout.range_km, readout.azimuth_deg),
            format!("{:.2} km AGL", readout.height_agl_km),
        ];

        // Compute tooltip position (offset from cursor)
        let offset = egui::vec2(16.0, -60.0);
        let tooltip_pos = cursor_screen + offset;

        // Measure text to size the background box
        let font = egui::FontId::monospace(12.0);
        let line_height = 16.0_f32;
        let padding = 6.0_f32;

        let max_width = lines.iter()
            .map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), egui::Color32::WHITE).rect.width())
            .fold(0.0_f32, f32::max);

        let box_width = max_width + padding * 2.0;
        let box_height = lines.len() as f32 * line_height + padding * 2.0;

        // Keep tooltip within bounds
        let mut box_min = tooltip_pos;
        if box_min.x + box_width > rect.right() {
            box_min.x = cursor_screen.x - box_width - 8.0;
        }
        if box_min.y < rect.top() {
            box_min.y = cursor_screen.y + 16.0;
        }

        let box_rect = egui::Rect::from_min_size(box_min, egui::vec2(box_width, box_height));

        // Draw background
        ui.painter().rect_filled(box_rect, 4.0, egui::Color32::from_black_alpha(200));
        ui.painter().rect_stroke(box_rect, 4.0, egui::Stroke::new(1.0, egui::Color32::from_gray(80)), egui::StrokeKind::Outside);

        // Draw text lines
        for (i, line) in lines.iter().enumerate() {
            let text_pos = egui::pos2(
                box_min.x + padding,
                box_min.y + padding + i as f32 * line_height,
            );
            ui.painter().text(
                text_pos,
                egui::Align2::LEFT_TOP,
                line,
                font.clone(),
                if i == 0 { egui::Color32::from_rgb(100, 255, 100) } else { egui::Color32::from_gray(220) },
            );
        }
    }

    /// Find the best sweep for a given product in a specific file (non-mutating helper).
    fn find_sweep_for_product_in(&self, file: &Level2File, product: RadarProduct) -> Option<usize> {
        let base = product.base_product();
        let require_super_res = product.is_super_res();

        // First, try the selected elevation
        if let Some(sweep) = file.sweeps.get(self.selected_elevation) {
            if Self::sweep_matches(sweep, base, require_super_res) {
                return Some(self.selected_elevation);
            }
        }
        // Fallback to lowest elevation with this product
        for (i, sweep) in file.sweeps.iter().enumerate() {
            if Self::sweep_matches(sweep, base, require_super_res) {
                return Some(i);
            }
        }
        None
    }

    fn draw_cross_section_line(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        if let Some(start) = self.cross_section_start {
            let (sx, sy) = self.map_view.lat_lon_to_pixel(start.0, start.1, screen_w, screen_h);
            let start_pos = egui::pos2(rect.left() + sx as f32, rect.top() + sy as f32);

            // Draw start marker
            ui.painter().circle_filled(start_pos, 6.0, egui::Color32::from_rgb(255, 100, 100));

            let end_point = self.cross_section_end.unwrap_or((self.cursor_lat, self.cursor_lon));
            let (ex, ey) = self.map_view.lat_lon_to_pixel(end_point.0, end_point.1, screen_w, screen_h);
            let end_pos = egui::pos2(rect.left() + ex as f32, rect.top() + ey as f32);

            // Draw line
            ui.painter().line_segment(
                [start_pos, end_pos],
                egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 100, 100)),
            );

            // Draw end marker
            if self.cross_section_end.is_some() {
                ui.painter().circle_filled(end_pos, 6.0, egui::Color32::from_rgb(255, 100, 100));
            }
        }
    }

    fn draw_measurement(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        // Show mode indicator when actively measuring (even before first click)
        if self.measure_mode && self.measure_start.is_none() {
            let yellow = egui::Color32::from_rgb(255, 255, 0);
            ui.painter().text(
                egui::pos2(rect.left() + 10.0, rect.top() + 10.0),
                egui::Align2::LEFT_TOP,
                "Measure: click start point",
                egui::FontId::proportional(14.0),
                yellow,
            );
            return;
        }

        let start = match self.measure_start {
            Some(s) => s,
            None => return,
        };

        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let (sx, sy) = self.map_view.lat_lon_to_pixel(start.0, start.1, screen_w, screen_h);
        let start_pos = egui::pos2(rect.left() + sx as f32, rect.top() + sy as f32);

        // Use end point if set, otherwise use cursor position (live preview)
        let end = self.measure_end.unwrap_or((self.cursor_lat, self.cursor_lon));
        let (ex, ey) = self.map_view.lat_lon_to_pixel(end.0, end.1, screen_w, screen_h);
        let end_pos = egui::pos2(rect.left() + ex as f32, rect.top() + ey as f32);

        let yellow = egui::Color32::from_rgb(255, 255, 0);

        // Draw start marker
        ui.painter().circle_filled(start_pos, 5.0, yellow);

        // Draw dashed line from start to end
        let dx = end_pos.x - start_pos.x;
        let dy = end_pos.y - start_pos.y;
        let line_len = (dx * dx + dy * dy).sqrt();
        let dash_len = 8.0_f32;
        let gap_len = 5.0_f32;
        let segment = dash_len + gap_len;

        if line_len > 1.0 {
            let nx = dx / line_len;
            let ny = dy / line_len;
            let mut t = 0.0_f32;
            while t < line_len {
                let t_end = (t + dash_len).min(line_len);
                let p0 = egui::pos2(start_pos.x + nx * t, start_pos.y + ny * t);
                let p1 = egui::pos2(start_pos.x + nx * t_end, start_pos.y + ny * t_end);
                ui.painter().line_segment([p0, p1], egui::Stroke::new(2.5, yellow));
                t += segment;
            }
        }

        // Draw end marker
        if self.measure_end.is_some() {
            ui.painter().circle_filled(end_pos, 5.0, yellow);
        }

        // Haversine distance and bearing
        let (dist_km, bearing) = haversine_distance_bearing(start.0, start.1, end.0, end.1);
        let dist_mi = dist_km * 0.621371;

        // Draw label at midpoint
        let mid_pos = egui::pos2(
            (start_pos.x + end_pos.x) / 2.0,
            (start_pos.y + end_pos.y) / 2.0,
        );

        let label = format!("{:.1} km / {:.1} mi\n{:.0}\u{00B0} bearing", dist_km, dist_mi, bearing);

        // Background rect for readability
        let font = egui::FontId::proportional(13.0);
        let galley = ui.painter().layout_no_wrap(label.clone(), font.clone(), yellow);
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(mid_pos.x + 10.0, mid_pos.y - galley.size().y / 2.0),
            galley.size(),
        ).expand(4.0);
        ui.painter().rect_filled(text_rect, 4.0, egui::Color32::from_black_alpha(180));
        ui.painter().galley(
            egui::pos2(mid_pos.x + 10.0, mid_pos.y - galley.size().y / 2.0),
            galley,
            yellow,
        );

        // Show mode indicator when measuring (waiting for second click)
        if self.measure_mode {
            ui.painter().text(
                egui::pos2(rect.left() + 10.0, rect.top() + 10.0),
                egui::Align2::LEFT_TOP,
                "Measure: click end point",
                egui::FontId::proportional(14.0),
                yellow,
            );
        }
    }
}

/// Haversine formula for distance (km) and initial bearing (degrees) between two lat/lon points.
fn haversine_distance_bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> (f64, f64) {
    let r = 6371.0; // Earth radius in km
    let lat1_r = lat1.to_radians();
    let lat2_r = lat2.to_radians();
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();

    let a = (dlat / 2.0).sin().powi(2)
        + lat1_r.cos() * lat2_r.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    let distance = r * c;

    let y = dlon.sin() * lat2_r.cos();
    let x = lat1_r.cos() * lat2_r.sin() - lat1_r.sin() * lat2_r.cos() * dlon.cos();
    let bearing = y.atan2(x).to_degrees();
    let bearing = (bearing + 360.0) % 360.0;

    (distance, bearing)
}

impl eframe::App for RadarApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // FPS tracking
        let frame_start = Instant::now();
        if let Some(last) = self.perf.last_frame_start {
            let dt = last.elapsed();
            self.perf.frame_times.push_back(dt);
            if self.perf.frame_times.len() > 60 {
                self.perf.frame_times.pop_front();
            }
            if !self.perf.frame_times.is_empty() {
                let avg: f64 = self.perf.frame_times.iter()
                    .map(|d| d.as_secs_f64())
                    .sum::<f64>() / self.perf.frame_times.len() as f64;
                self.perf.fps = if avg > 0.0 { 1.0 / avg } else { 0.0 };
            }
        }
        self.perf.last_frame_start = Some(frame_start);

        // Animation downloads use the same fetcher — don't let check_downloads steal data
        if self.anim_loading {
            self.check_animation_downloads(ctx);
        } else {
            self.check_downloads(ctx);
        }
        self.check_preload_downloads();
        self.check_wall_downloads(ctx);
        self.check_secondary_downloads(ctx);
        self.check_secondary_anim_downloads(ctx);
        // Refresh weather alerts every 60 seconds
        if self.show_warnings && self.alert_fetcher.should_refresh() {
            self.alert_fetcher.fetch_alerts();
        }
        // Check for sounding results
        self.check_sounding_result(ctx);
        // Auto-scan for tornado risk
        self.autoscan.update();
        // Prediction backfill downloads
        self.check_prediction_backfill();
        // Auto-infer toggle: start backfill when turned on
        if self.auto_infer && self.auto_infer_was_off {
            self.auto_infer_was_off = false;
            log::info!("Auto-infer enabled — downloading previous frames for inference");
            // Run detection on current file first
            if let Some(ref file) = self.current_file.clone() {
                if let Some(site) = sites::find_site(&self.selected_station) {
                    let (mesos, tvs) = crate::nexrad::detection::RotationDetector::detect(file, site);
                    self.meso_detections = mesos;
                    self.tvs_detections = tvs;
                }
                // Seed buffer with current file if empty
                if self.prediction_buffer.is_empty() {
                    self.prediction_buffer.push_back(file.clone());
                }
            }
            self.start_prediction_backfill();
        } else if !self.auto_infer && !self.auto_infer_was_off {
            self.auto_infer_was_off = true;
            self.auto_infer_results.clear();
        }
        // Prediction demo
        if self.prediction_demo_pending {
            self.prediction_demo_pending = false;
            self.run_prediction_demo();
        }
        // Sync preload cache → national view thumbnails (throttled to once per 2 seconds)
        #[cfg(not(target_arch = "wasm32"))]
        {
            let now = Instant::now();
            let should_sync = self.last_preload_sync
                .map(|t| now.duration_since(t) > Duration::from_secs(2))
                .unwrap_or(true);
            if should_sync {
                self.last_preload_sync = Some(now);
                if let Some(ref engine) = self.preload_engine {
                    let cache = engine.get_cache();
                    if let Ok(guard) = cache.try_read() {
                        let loaded = guard.stations_loaded();
                        if loaded.len() > self.national_view.loaded_count() {
                            for station_id in &loaded {
                                if let Some(cached) = guard.get(station_id) {
                                    if let Some(ref pixels) = cached.thumbnail_pixels {
                                        self.national_view.update_thumbnail(ctx, station_id, pixels);
                                    }
                                }
                            }
                        }
                    }
                    // Periodically refresh alerts and re-trigger preload
                    if self.show_warnings && self.alert_fetcher.should_refresh() {
                        let alerts = self.alert_fetcher.get_alerts();
                        engine.preload_active_weather(&alerts);
                    }

                    // When zoomed to single-radar view, preload current + neighbors within ~300km
                    if self.map_view.zoom >= 7.0 {
                        if let Some(site) = sites::find_site(&self.selected_station) {
                            let neighbors = sites::find_nearest_sites(site.lat, site.lon, 6);
                            let mut to_preload = Vec::new();
                            let cache_ref = engine.get_cache();
                            if let Ok(guard) = cache_ref.try_read() {
                                for neighbor in &neighbors {
                                    let dist = sites::haversine_km(site.lat, site.lon, neighbor.lat, neighbor.lon);
                                    if dist <= 300.0 && !guard.has(neighbor.id) {
                                        to_preload.push(neighbor.id.to_string());
                                    }
                                }
                            }
                            if !to_preload.is_empty() {
                                log::info!("Preloading {} neighbors of {}", to_preload.len(), self.selected_station);
                                engine.start_preload(to_preload);
                            }
                        }
                    }
                }
            }
        }
        // Sync mosaic mode from preload cache (every 500ms)
        #[cfg(not(target_arch = "wasm32"))]
        if self.mosaic_mode {
            let should_sync = self.last_preload_sync
                .map_or(true, |t| t.elapsed() > Duration::from_millis(500));
            if should_sync {
                self.last_preload_sync = Some(Instant::now());
                self.sync_mosaic_from_cache(ctx);
            }
        }

        if self.pending_anim_prerender {
            self.pending_anim_prerender = false;
            self.pre_render_animation_textures(ctx);
            if self.multi_radar_anim && self.multi_anim_ready {
                self.pre_render_secondary_anim_textures(ctx);
            }
        }
        self.advance_animation();
        // Lazy-render the current animation frame's texture if needed
        if self.anim_playing && !self.anim_frames.is_empty() {
            let idx = self.anim_index;
            self.ensure_anim_texture(idx, ctx);
            // Also use the freshly-rendered texture
            let has_cached = self.anim_textures.get(idx).and_then(|t| t.as_ref()).is_some();
            if has_cached {
                if let Some(cached) = self.anim_textures.get(idx) {
                    self.single_texture = cached.clone();
                }
                if self.quad_view {
                    if let Some(cached_quad) = self.anim_quad_textures.get(idx) {
                        self.quad_textures = cached_quad.clone();
                    }
                }
            }
        }
        self.handle_keyboard(ctx);
        self.check_zoom_rerender();
        self.render_radar(ctx);

        // Create cross-section texture if result is pending
        if let Some(res) = self.cross_section_result.take() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [res.width as usize, res.height as usize],
                &res.pixels,
            );
            self.cross_section_texture = Some(ctx.load_texture(
                "cross_section",
                image,
                egui::TextureOptions::NEAREST,
            ));
            self.cross_section_max_alt_km = res.max_altitude_km;
        }

        if self.use_new_ui {
            Toolbar::show(self, ctx);
            TimelineBar::show(self, ctx);
            CollapsibleSidebar::show(self, ctx);
        } else {
            ControlBar::show(self, ctx);
            SidePanel::show(self, ctx);
        }

        // Cross-section window (bottom panel)
        if self.cross_section_texture.is_some() {
            egui::TopBottomPanel::bottom("cross_section_panel")
                .resizable(true)
                .default_height(250.0)
                .show(ctx, |ui| {
                    let mut rerender = false;
                    ui.horizontal(|ui| {
                        ui.strong("Cross Section");
                        ui.label(format!("| Max alt: {:.0} km", self.cross_section_max_alt_km));
                        ui.separator();
                        if ui.selectable_label(!self.cross_section_3d, "2D").clicked() && self.cross_section_3d {
                            self.cross_section_3d = false;
                            rerender = true;
                        }
                        if ui.selectable_label(self.cross_section_3d, "3D").clicked() && !self.cross_section_3d {
                            self.cross_section_3d = true;
                            rerender = true;
                        }
                        if self.cross_section_3d {
                            ui.separator();
                            ui.label("Angle:");
                            let angle_resp = ui.add(egui::DragValue::new(&mut self.cross_section_view_angle)
                                .range(-180.0..=180.0).speed(1.0).suffix("°"));
                            if angle_resp.changed() { rerender = true; }
                            ui.label("Pitch:");
                            let pitch_resp = ui.add(egui::DragValue::new(&mut self.cross_section_view_pitch)
                                .range(0.0..=60.0).speed(0.5).suffix("°"));
                            if pitch_resp.changed() { rerender = true; }
                        }
                        ui.separator();
                        if ui.button("Close").clicked() {
                            self.cross_section_texture = None;
                            self.cross_section_start = None;
                            self.cross_section_end = None;
                        }
                    });
                    // Get texture info without holding borrow during drag handling
                    let tex_info = self.cross_section_texture.as_ref().map(|tex| {
                        (tex.id(), tex.size())
                    });
                    if let Some((tex_id, tex_size)) = tex_info {
                        let available_w = ui.available_width();
                        let aspect = tex_size[0] as f32 / tex_size[1] as f32;
                        let max_h = if self.cross_section_3d { 320.0 } else { 220.0 };
                        let h = (available_w / aspect).min(max_h).max(100.0);

                        // Allocate with drag sense for 3D rotation
                        let sense = if self.cross_section_3d {
                            egui::Sense::click_and_drag()
                        } else {
                            egui::Sense::hover()
                        };
                        let (img_rect, img_resp) = ui.allocate_exact_size(
                            egui::vec2(available_w, h),
                            sense,
                        );
                        // Drag to rotate in 3D mode
                        if self.cross_section_3d && img_resp.dragged() {
                            let delta = img_resp.drag_delta();
                            self.cross_section_view_angle += delta.x as f64 * 0.5;
                            self.cross_section_view_pitch = (self.cross_section_view_pitch - delta.y as f64 * 0.3).clamp(0.0, 60.0);
                            rerender = true;
                        }

                        // Dark background
                        ui.painter().rect_filled(img_rect, 0.0, egui::Color32::from_rgb(10, 10, 20));

                        ui.painter().image(
                            tex_id,
                            img_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );

                        // Altitude axis labels (2D mode only — 3D has tick marks baked in)
                        if !self.cross_section_3d {
                            for i in 0..=4 {
                                let t = i as f32 / 4.0;
                                let alt = self.cross_section_max_alt_km * (1.0 - t as f64);
                                let y = img_rect.top() + t * h;
                                ui.painter().text(
                                    egui::pos2(img_rect.left() + 4.0, y),
                                    egui::Align2::LEFT_TOP,
                                    format!("{:.0}km", alt),
                                    egui::FontId::proportional(10.0),
                                    egui::Color32::from_gray(200),
                                );
                            }
                        }
                    }
                    if rerender {
                        self.render_cross_section_image();
                    }
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_rgb(20, 20, 30)))
            .show(ctx, |ui| {
                let available_rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(available_rect, egui::Sense::click_and_drag());

                let bg_color = if self.dark_mode {
                    egui::Color32::from_rgb(20, 20, 30)
                } else {
                    egui::Color32::from_rgb(220, 225, 230)
                };
                ui.painter().rect_filled(available_rect, 0.0, bg_color);

                if self.wall_mode {
                    self.draw_wall_mode(ui, available_rect);
                } else if self.quad_view {
                    let screen_w = (available_rect.width() / 2.0) as f64;
                    let screen_h = (available_rect.height() / 2.0) as f64;
                    let visible = self.map_view.visible_tiles(screen_w, screen_h);
                    for key in &visible {
                        self.tile_manager.request_tile(*key);
                        if let Some(tile_data) = self.tile_manager.get_tile(key) {
                            self.tile_textures.entry(*key).or_insert_with(|| {
                                let image = egui::ColorImage::from_rgba_unmultiplied(
                                    [tile_data.width as usize, tile_data.height as usize],
                                    &tile_data.pixels,
                                );
                                ui.ctx().load_texture(
                                    format!("tile_{}_{}_{}", key.z, key.x, key.y),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                )
                            });
                        }
                    }
                    // Prefetch tiles for expanded viewport and zoom-out
                    let prefetch = self.map_view.prefetch_tiles(screen_w, screen_h);
                    for key in &prefetch {
                        self.tile_manager.request_tile(*key);
                    }
                    self.draw_quad_overlay(ui, available_rect);
                    self.draw_radar_sites(ui, available_rect);
                } else if self.dual_pane {
                    self.draw_dual_pane(ui, available_rect);
                    self.draw_radar_sites(ui, available_rect);
                } else {
                    self.draw_map(ui, available_rect);
                    self.draw_radar_overlay(ui, available_rect);
                    self.draw_timestamp_overlay(ui, available_rect);
                    self.draw_overlays(ui, available_rect);
                    self.draw_radar_sites(ui, available_rect);
                    self.draw_cursor_readout(ui, available_rect, self.selected_product);
                }

                // Draw cross-section line on map
                if self.cross_section_start.is_some() {
                    self.draw_cross_section_line(ui, available_rect);
                }

                // Draw measurement line/label
                if self.measure_mode || self.measure_start.is_some() {
                    self.draw_measurement(ui, available_rect);
                }

                // Hover preview on radar sites
                if self.use_new_ui {
                    let cursor_pos = response.hover_pos();
                    if let Some(site) = self.hover_preview.detect_hover(cursor_pos, &self.map_view, available_rect) {
                        let site_id = site.id.to_string();
                        let thumbnail: Option<Vec<u8>> = {
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                self.preload_engine.as_ref().and_then(|e| {
                                    let cache = e.get_cache();
                                    let guard = cache.try_read().ok()?;
                                    guard.get(&site_id).and_then(|c| c.thumbnail_pixels.clone())
                                })
                            }
                            #[cfg(target_arch = "wasm32")]
                            { None }
                        };
                        if let Some(pos) = cursor_pos {
                            self.hover_preview.draw_preview(
                                ctx, ui, site, pos,
                                thumbnail.as_deref(),
                                None,
                            );
                        }
                    }

                    // Minimap in corner when zoomed in
                    if Minimap::should_show(self.map_view.zoom) {
                        let loaded: Vec<String> = {
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                self.preload_engine.as_ref()
                                    .and_then(|e| {
                                        let cache = e.get_cache();
                                        cache.try_read().ok().map(|g| g.stations_loaded())
                                    })
                                    .unwrap_or_default()
                            }
                            #[cfg(target_arch = "wasm32")]
                            { Vec::new() }
                        };
                        if let Some((lat, lon)) = self.minimap.draw(
                            ui, available_rect, &self.map_view,
                            &self.selected_station, &loaded,
                        ) {
                            self.map_view.center_lat = lat;
                            self.map_view.center_lon = lon;
                        }
                    }
                }

                self.handle_interaction(&response, available_rect);
            });

        // Sounding window
        if self.sounding_texture.is_some() || self.sounding_pending {
            let mut open = true;
            egui::Window::new("Sounding - Skew-T/Log-P")
                .open(&mut open)
                .resizable(true)
                .default_size([600.0, 800.0])
                .show(ctx, |ui| {
                    if self.sounding_fetcher.is_fetching() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.spinner();
                            ui.label("Fetching sounding data...");
                            ui.add_space(8.0);
                            ui.label("(Trying multiple data sources — may take up to 30s)");
                        });
                    } else if !self.sounding_fetcher.is_fetching() && self.sounding_pending && self.sounding_texture.is_none() {
                        // Fetch finished but no texture = parse/render failed
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.colored_label(egui::Color32::YELLOW, "No sounding data available for this location.");
                            ui.label("Try clicking closer to a reporting station.");
                        });
                    } else if let Some(tex) = &self.sounding_texture {
                        let available = ui.available_size();
                        let tex_aspect = tex.size()[0] as f32 / tex.size()[1] as f32;
                        let (w, h) = if available.x / available.y > tex_aspect {
                            (available.y * tex_aspect, available.y)
                        } else {
                            (available.x, available.x / tex_aspect)
                        };
                        let img_size = egui::vec2(w, h);
                        ui.image(egui::load::SizedTexture::new(tex.id(), img_size));
                    }
                });
            if !open {
                self.sounding_texture = None;
                self.sounding_pending = false;
            }
        }

        // ── HRRR Model Overlay ──────────────────────────────────
        if self.hrrr_mode {
            // Check for incoming HRRR frame
            {
                let mut result = self.hrrr_result.lock().unwrap();
                if let Some(frame) = result.take() {
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &frame.pixels,
                    );
                    self.hrrr_texture = Some(ctx.load_texture(
                        "hrrr_overlay", color_image, egui::TextureOptions::LINEAR,
                    ));
                    self.hrrr_tex_size = [frame.width, frame.height];
                }
            }

            egui::Window::new("HRRR Model Data")
                .default_size([900.0, 700.0])
                .resizable(true)
                .collapsible(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let current_label = if let Some(ref comp) = self.hrrr_composite {
                            hrrr_render::composite::COMPOSITE_FIELDS.iter()
                                .find(|c| c.name == comp.as_str())
                                .map(|c| c.label)
                                .unwrap_or("Unknown")
                        } else {
                            hrrr_render::fields::FIELDS[self.hrrr_field_idx].label
                        };

                        egui::ComboBox::from_label("Field")
                            .selected_text(current_label)
                            .show_ui(ui, |ui| {
                                // Regular fields
                                for (i, field) in hrrr_render::fields::FIELDS.iter().enumerate() {
                                    if ui.selectable_label(
                                        self.hrrr_composite.is_none() && self.hrrr_field_idx == i,
                                        field.label
                                    ).clicked() {
                                        self.hrrr_field_idx = i;
                                        self.hrrr_composite = None;
                                    }
                                }
                                ui.separator();
                                // Composite fields
                                for comp in hrrr_render::composite::COMPOSITE_FIELDS.iter() {
                                    if ui.selectable_label(
                                        self.hrrr_composite.as_deref() == Some(comp.name),
                                        comp.label
                                    ).clicked() {
                                        self.hrrr_composite = Some(comp.name.to_string());
                                    }
                                }
                            });

                        ui.add(egui::DragValue::new(&mut self.hrrr_forecast_hour)
                            .range(0..=48)
                            .prefix("f")
                            .speed(0.2));

                        let fetching = *self.hrrr_fetching.lock().unwrap();
                        if ui.add_enabled(!fetching,
                            egui::Button::new(if fetching { "Loading..." } else { "Fetch" })
                        ).clicked() {
                            self.fetch_hrrr_frame(ctx);
                        }

                        let status = self.hrrr_status.lock().unwrap().clone();
                        ui.label(&status);
                    });

                    // Display the rendered HRRR map
                    if let Some(ref tex) = self.hrrr_texture {
                        let available = ui.available_size();
                        let img_w = self.hrrr_tex_size[0] as f32;
                        let img_h = self.hrrr_tex_size[1] as f32;
                        let scale = (available.x / img_w).min(available.y / img_h).min(1.0);
                        let size = egui::vec2(img_w * scale, img_h * scale);
                        ui.image(egui::load::SizedTexture::new(tex.id(), size));
                    }
                });
        }

        // Help overlay window
        if self.show_help {
            let mut open = self.show_help;
            egui::Window::new("Keyboard Shortcuts")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_width(420.0)
                .show(ctx, |ui| {
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);

                    let shortcuts: &[(&str, &str)] = &[
                        ("H / F1",         "Toggle this help overlay"),
                        ("Space",          "Play / pause animation"),
                        ("Left / Right",   "Step animation frames"),
                        (",  /  .",        "Step animation frames (alt)"),
                        ("Up / Down",      "Change elevation angle"),
                        ("1-7",            "Select product (1=REF 2=VEL 3=SW 4=ZDR 5=CC 6=KDP 7=SRV)"),
                        ("Q",              "Toggle quad view"),
                        ("R",              "Toggle range rings"),
                        ("W",              "Toggle NWS warnings"),
                        ("D",              "Toggle meso/TVS detection"),
                        ("S",              "Toggle sounding mode"),
                        ("Y",              "Toggle HRRR model overlay"),
                        ("M",              "Toggle measure mode"),
                        ("L",              "Load latest data"),
                        ("Shift+Click",    "Add/remove secondary radar"),
                        ("+ / -",          "Zoom in / out"),
                        ("Scroll",         "Zoom at cursor"),
                        ("Drag",           "Pan map"),
                    ];

                    egui::Grid::new("help_shortcuts_grid")
                        .num_columns(2)
                        .spacing([20.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for (key, desc) in shortcuts {
                                ui.strong(*key);
                                ui.label(*desc);
                                ui.end_row();
                            }
                        });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.weak("Press H or F1 to close");
                });
            if !open {
                self.show_help = false;
            }
        }

        // Settings window
        if self.show_settings {
            let mut open = self.show_settings;
            egui::Window::new("Settings")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(320.0)
                .show(ctx, |ui| {
                    ui.heading("Display");
                    ui.horizontal(|ui| {
                        ui.label("Radar opacity:");
                        ui.add(egui::Slider::new(&mut self.radar_opacity, 0.0..=1.0).step_by(0.01));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Map opacity:");
                        ui.add(egui::Slider::new(&mut self.map_opacity, 0.0..=1.0).step_by(0.01));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Warning opacity:");
                        ui.add(egui::Slider::new(&mut self.warning_opacity, 0.0..=1.0).step_by(0.01));
                    });
                    ui.checkbox(&mut self.dark_mode, "Dark Mode");

                    ui.separator();
                    ui.heading("Overlays");
                    ui.checkbox(&mut self.show_warnings, "NWS Warnings");
                    ui.checkbox(&mut self.show_range_rings, "Range Rings");

                    ui.separator();
                    ui.heading("Rendering");
                    if self.gpu_renderer.is_some() {
                        ui.checkbox(&mut self.gpu_rendering, "GPU Rendering");
                    } else {
                        ui.label("GPU rendering: not available");
                    }
                    if ui.checkbox(&mut self.smooth_rendering, "Smooth Triangulation").changed() {
                        self.mark_all_needs_render();
                    }

                    ui.separator();
                    ui.heading("Defaults");
                    ui.horizontal(|ui| {
                        ui.label("Default station:");
                        ui.text_edit_singleline(&mut self.settings.default_station);
                    });
                    if ui.button("Save defaults").clicked() {
                        self.settings.default_zoom = self.map_view.zoom;
                        self.settings.quad_view = self.quad_view;
                        self.settings.save();
                    }

                    ui.separator();
                    ui.heading("Export");
                    if !self.anim_frames.is_empty() {
                        if ui.button(format!("Export GIF ({} frames)", self.anim_frames.len())).clicked() {
                            self.export_loop_gif();
                        }
                        if let Some(status) = &self.gif_export_status {
                            ui.label(status.as_str());
                        }
                    } else {
                        ui.weak("Load an animation loop to export GIF");
                    }
                });
            if !open {
                self.show_settings = false;
            }
        }

        // Only request continuous repaint when actually needed
        let any_secondary_fetching = self.secondary_radars.iter().any(|r| r.fetcher.is_fetching());
        if self.anim_playing || self.anim_loading || self.fetcher.is_fetching()
            || any_secondary_fetching
        {
            ctx.request_repaint();
        } else if self.sounding_fetcher.is_fetching() {
            // Sounding fetch only needs ~4fps for the spinner animation
            ctx.request_repaint_after(Duration::from_millis(250));
        } else {
            // Otherwise repaint at a low rate for background updates
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl RadarApp {
    fn draw_overlays(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        // Range rings
        if self.show_range_rings {
            crate::render::overlays::MapOverlays::draw_range_rings(
                ui.painter(), site.lat, site.lon, &self.map_view, rect,
            );
        }

        // Azimuth lines
        if self.show_azimuth_lines {
            let max_range = self.last_render_range_km.unwrap_or(230.0);
            crate::render::overlays::MapOverlays::draw_azimuth_lines(
                ui.painter(), site.lat, site.lon, &self.map_view, rect, max_range,
            );
        }

        // Site marker
        crate::render::overlays::MapOverlays::draw_site_marker(
            ui.painter(), site.lat, site.lon, &self.selected_station, &self.map_view, rect,
        );

        // City labels
        if self.show_cities {
            crate::render::geo_overlays::GeoOverlays::draw_cities(
                ui.painter(), &self.map_view, rect, self.map_view.zoom,
            );
        }

        // Weather warnings
        if self.show_warnings {
            let alerts = self.alert_fetcher.get_alerts();
            crate::render::warnings::WarningRenderer::draw_warnings_with_opacity(
                &alerts, ui.painter(), &self.map_view, rect, self.warning_opacity,
            );
        }

        // Mesocyclone/TVS detections
        if self.show_detections {
            self.draw_detections(ui, rect);
        }

        // Tornado prediction overlay
        if self.prediction_active {
            self.draw_prediction_overlay(ui, rect);
        }

        // Auto-scan risk markers
        if self.autoscan.active {
            self.draw_autoscan_overlay(ui, rect);
        }

        // Auto-infer results on active radars
        if self.auto_infer && !self.auto_infer_results.is_empty() {
            self.draw_auto_infer_overlay(ui, rect);
        }
    }

    fn draw_detections(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        // Draw mesocyclone markers
        for meso in &self.meso_detections {
            let (px, py) = self.map_view.lat_lon_to_pixel(meso.lat, meso.lon, screen_w, screen_h);
            let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);
            if !rect.contains(pos) { continue; }

            let (color, radius) = match meso.strength {
                crate::nexrad::detection::RotationStrength::Weak =>
                    (egui::Color32::YELLOW, 6.0),
                crate::nexrad::detection::RotationStrength::Moderate =>
                    (egui::Color32::from_rgb(255, 165, 0), 8.0),
                crate::nexrad::detection::RotationStrength::Strong =>
                    (egui::Color32::RED, 10.0),
            };
            ui.painter().circle_stroke(pos, radius, egui::Stroke::new(2.0, color));
            ui.painter().circle_stroke(pos, radius - 3.0, egui::Stroke::new(1.0, color));
        }

        // Draw TVS markers (inverted triangle)
        for tvs in &self.tvs_detections {
            let (px, py) = self.map_view.lat_lon_to_pixel(tvs.lat, tvs.lon, screen_w, screen_h);
            let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);
            if !rect.contains(pos) { continue; }

            let size = 10.0;
            let color = egui::Color32::from_rgb(255, 0, 0);
            // Inverted triangle
            let points = vec![
                egui::pos2(pos.x - size, pos.y - size),
                egui::pos2(pos.x + size, pos.y - size),
                egui::pos2(pos.x, pos.y + size),
            ];
            ui.painter().add(egui::Shape::convex_polygon(points, color.linear_multiply(0.3), egui::Stroke::new(2.5, color)));
            ui.painter().text(pos + egui::vec2(12.0, -4.0), egui::Align2::LEFT_CENTER,
                "TVS", egui::FontId::proportional(10.0), color);
        }
    }

    fn start_prediction_backfill(&mut self) {
        let files = self.fetcher.available_files.lock().unwrap().clone();
        if files.len() < 2 {
            log::info!("Prediction backfill: not enough files in list");
            return;
        }
        // Take up to 7 previous files (current frame is already in buffer)
        // This gives us 8 total for a single inference window
        let end = files.len().saturating_sub(1); // exclude the latest (already loaded)
        let start = end.saturating_sub(7);
        let keys: Vec<String> = files[start..end].iter().map(|f| f.key.clone()).collect();

        if keys.is_empty() {
            return;
        }

        log::info!("Prediction backfill: downloading {} previous frames", keys.len());
        self.prediction_backfill_count = keys.len();
        self.prediction_backfill_received = 0;

        let backfill_fetcher = NexradFetcher::new(self.runtime.handle().clone());
        let rx = backfill_fetcher.download_files_parallel(keys);
        self.prediction_backfill_rx = Some(rx);
    }

    fn check_prediction_backfill(&mut self) {
        let rx = match self.prediction_backfill_rx.as_mut() {
            Some(rx) => rx,
            None => return,
        };

        let mut new_frames: Vec<(usize, Level2File)> = Vec::new();
        while let Ok((idx, data)) = rx.try_recv() {
            self.prediction_backfill_received += 1;
            match Level2File::parse(&data) {
                Ok(file) if !file.sweeps.is_empty() => {
                    new_frames.push((idx, file));
                }
                Ok(_) => log::warn!("Backfill frame {} parsed but 0 sweeps", idx),
                Err(e) => log::warn!("Backfill frame {} parse error: {}", idx, e),
            }
        }

        // Insert frames sorted by index at the front of the buffer
        if !new_frames.is_empty() {
            new_frames.sort_by_key(|(idx, _)| *idx);
            // Current buffer has the latest frame; prepend backfill frames
            let current: Vec<Level2File> = self.prediction_buffer.drain(..).collect();
            for (_, file) in new_frames {
                self.prediction_buffer.push_back(file);
            }
            for file in current {
                self.prediction_buffer.push_back(file);
            }
            // Trim to 16
            while self.prediction_buffer.len() > 8 {
                self.prediction_buffer.pop_front();
            }
        }

        // Done?
        if self.prediction_backfill_received >= self.prediction_backfill_count {
            self.prediction_backfill_rx = None;
            log::info!(
                "Prediction backfill complete: {} frames in buffer",
                self.prediction_buffer.len()
            );
            // Run prediction on target if active
            self.run_prediction();
            // Run auto-infer on all rotation points if enabled
            if self.auto_infer {
                self.run_auto_infer_all();
            }
        }
    }

    fn run_prediction_demo(&mut self) {
        // Greenfield 2024 EF4 - KDMX (closest radar, 81km), April 26, 2024
        let station = "KDMX";
        let storm_lat = 41.3;
        let storm_lon = -94.5;

        // Try loading from data pack
        let pack_data = self.pack_manager.load_pack(station, 2024, 4, 26);
        let raw_files = match pack_data {
            Some(files) => files,
            None => {
                // Download the pack first
                log::info!("Demo: downloading Greenfield data pack...");
                self.pack_manager.download_pack(station, 2024, 4, 26);
                return; // Will retry on next update when pack is ready
            }
        };

        // Parse last 16 files for rolling inference
        let mut parsed: Vec<crate::nexrad::Level2File> = Vec::new();
        let start = raw_files.len().saturating_sub(16);
        for (_name, data) in &raw_files[start..] {
            if let Ok(file) = crate::nexrad::Level2File::parse(data) {
                if !file.sweeps.is_empty() {
                    parsed.push(file);
                }
            }
        }

        if parsed.is_empty() {
            log::error!("Demo: no valid files parsed");
            return;
        }

        log::info!("Demo: {} frames parsed, running prediction...", parsed.len());

        let site = match sites::find_site(station) {
            Some(s) => s,
            None => return,
        };

        // Set up prediction state
        self.prediction_active = true;
        self.prediction_target = Some((storm_lat, storm_lon));
        self.prediction_buffer.clear();
        for f in &parsed {
            self.prediction_buffer.push_back(f.clone());
        }

        // Switch to this station and load the last frame
        self.selected_station = station.to_string();
        if let Some(last) = parsed.last() {
            self.current_file = Some(last.clone());
            self.needs_render = true;
        }

        // Center map on storm
        self.map_view.center_lat = storm_lat;
        self.map_view.center_lon = storm_lon;
        self.map_view.zoom = 8.0;

        // Run rolling inference across all windows
        self.prediction_last_run = None; // Reset throttle so run_prediction fires
        self.run_prediction();
    }

    fn run_prediction(&mut self) {
        if !self.prediction_active || self.prediction_buffer.len() < 3 {
            return;
        }
        let (storm_lat, storm_lon) = match self.prediction_target {
            Some(t) => t,
            None => return,
        };
        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        // Throttle: at most once every 2 seconds
        if let Some(last) = self.prediction_last_run {
            if last.elapsed() < Duration::from_secs(2) {
                return;
            }
        }

        // Use the latest 8 frames (or fewer if not enough yet)
        let files: Vec<Level2File> = self.prediction_buffer.iter().cloned().collect();

        #[cfg(feature = "tornado-predict")]
        {
            if let Some(ref mut predictor) = self.tornado_predictor {
                let sequence = crate::predict::convert::RadarSequence::from_files(
                    &files, storm_lat, storm_lon, site,
                );

                if let Some(seq) = sequence {
                    match predictor.predict(&seq) {
                        Ok(result) => {
                            log::info!(
                                "Prediction: det={:.1}% pred={:.1}% risk={} ({} frames)",
                                result.detection_prob * 100.0,
                                result.prediction_prob * 100.0,
                                result.risk_level(),
                                files.len(),
                            );
                            self.prediction_history.push((
                                std::time::Instant::now(),
                                result.prediction_prob,
                            ));
                            self.prediction_result = Some(result);
                        }
                        Err(e) => {
                            log::error!("Prediction failed: {}", e);
                        }
                    }
                }
            }
        }
        #[cfg(not(feature = "tornado-predict"))]
        {
            log::warn!("Tornado prediction requires 'tornado-predict' feature");
        }

        // Keep last 50 history entries
        while self.prediction_history.len() > 50 {
            self.prediction_history.remove(0);
        }

        self.prediction_last_run = Some(std::time::Instant::now());
    }

    /// Run inference on all detected rotation points using prediction_buffer.
    /// Called after backfill completes or when new data arrives with auto_infer on.
    fn run_auto_infer_all(&mut self) {
        if self.prediction_buffer.len() < 3 {
            log::info!("Auto-infer: only {} frames, need at least 3", self.prediction_buffer.len());
            return;
        }

        let site = match sites::find_site(&self.selected_station) {
            Some(s) => s,
            None => return,
        };

        // Collect all TVS and strong/moderate meso detections
        let targets: Vec<(f64, f64)> = self.tvs_detections.iter()
            .map(|t| (t.lat, t.lon))
            .chain(self.meso_detections.iter()
                .filter(|m| matches!(m.strength,
                    crate::nexrad::detection::RotationStrength::Strong |
                    crate::nexrad::detection::RotationStrength::Moderate))
                .map(|m| (m.lat, m.lon)))
            .collect();

        if targets.is_empty() {
            log::info!("Auto-infer: no rotation detected");
            self.auto_infer_results.clear();
            return;
        }

        let mut results: Vec<crate::predict::convert::TornadoPrediction> = Vec::new();

        #[cfg(feature = "tornado-predict")]
        {
            if let Some(ref mut predictor) = self.tornado_predictor {
                let files: Vec<Level2File> = self.prediction_buffer.iter().cloned().collect();
                for (lat, lon) in targets.iter().take(10) {
                    if let Some(seq) = crate::predict::convert::RadarSequence::from_files(
                        &files, *lat, *lon, site,
                    ) {
                        match predictor.predict(&seq) {
                            Ok(pred) => {
                                log::info!(
                                    "Auto-infer ({:.2},{:.2}): det={:.1}% pred={:.1}%",
                                    lat, lon, pred.detection_prob * 100.0, pred.prediction_prob * 100.0,
                                );
                                results.push(pred);
                            }
                            Err(e) => log::warn!("Auto-infer failed for ({:.2},{:.2}): {}", lat, lon, e),
                        }
                    }
                }
            }
        }

        // Sort by risk descending
        results.sort_by(|a, b| {
            let sa = a.detection_prob.max(a.prediction_prob);
            let sb = b.detection_prob.max(b.prediction_prob);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        self.auto_infer_results = results;
    }

    fn draw_prediction_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let (lat, lon) = match self.prediction_target {
            Some(t) => t,
            None => return,
        };

        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;
        let (px, py) = self.map_view.lat_lon_to_pixel(lat, lon, screen_w, screen_h);
        let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);

        // Draw crosshair at target
        let crosshair_color = egui::Color32::from_rgb(0, 200, 255);
        let arm = 12.0;
        ui.painter().line_segment(
            [pos - egui::vec2(arm, 0.0), pos + egui::vec2(arm, 0.0)],
            egui::Stroke::new(2.0, crosshair_color),
        );
        ui.painter().line_segment(
            [pos - egui::vec2(0.0, arm), pos + egui::vec2(0.0, arm)],
            egui::Stroke::new(2.0, crosshair_color),
        );
        ui.painter().circle_stroke(pos, 8.0, egui::Stroke::new(1.5, crosshair_color));

        // Draw risk badge if we have a result
        if let Some(ref result) = self.prediction_result {
            let risk = result.risk_level();
            let max_prob = result.detection_prob.max(result.prediction_prob);

            let badge_color = match risk {
                "EXTREME" => egui::Color32::from_rgb(200, 0, 0),
                "HIGH" => egui::Color32::from_rgb(255, 100, 0),
                "MODERATE" => egui::Color32::from_rgb(255, 200, 0),
                "LOW" => egui::Color32::from_rgb(100, 200, 0),
                _ => egui::Color32::from_rgb(100, 100, 100),
            };

            let text = format!("{} {:.0}%", risk, max_prob * 100.0);
            let badge_pos = pos + egui::vec2(16.0, -16.0);

            // Background pill
            let galley = ui.painter().layout_no_wrap(
                text.clone(),
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
            let text_rect = egui::Rect::from_min_size(
                badge_pos,
                galley.size() + egui::vec2(12.0, 4.0),
            );
            ui.painter().rect_filled(
                text_rect,
                4.0,
                badge_color,
            );
            ui.painter().text(
                text_rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }
    }

    fn draw_auto_infer_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        for (i, result) in self.auto_infer_results.iter().enumerate() {
            let (px, py) = self.map_view.lat_lon_to_pixel(
                result.storm_lat, result.storm_lon, screen_w, screen_h,
            );
            let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);
            if !rect.contains(pos) { continue; }

            let risk = result.risk_level();
            let color = match risk {
                "EXTREME" => egui::Color32::from_rgb(255, 0, 0),
                "HIGH" => egui::Color32::from_rgb(255, 120, 0),
                "MODERATE" => egui::Color32::from_rgb(255, 200, 0),
                "LOW" => egui::Color32::from_rgb(100, 200, 0),
                _ => egui::Color32::GRAY,
            };

            let max_prob = result.detection_prob.max(result.prediction_prob);
            let label = format!("{} {:.0}%", risk, max_prob * 100.0);

            // Diamond marker
            let s = 8.0;
            let diamond = vec![
                egui::pos2(pos.x, pos.y - s),
                egui::pos2(pos.x + s, pos.y),
                egui::pos2(pos.x, pos.y + s),
                egui::pos2(pos.x - s, pos.y),
            ];
            ui.painter().add(egui::Shape::convex_polygon(
                diamond, color.linear_multiply(0.3), egui::Stroke::new(2.0, color),
            ));

            ui.painter().text(
                pos + egui::vec2(s + 4.0, -2.0),
                egui::Align2::LEFT_CENTER,
                &label,
                egui::FontId::proportional(11.0),
                color,
            );
        }
    }

    fn draw_autoscan_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;
        let results = self.autoscan.top_results(10);

        for (i, result) in results.iter().enumerate() {
            let (px, py) = self.map_view.lat_lon_to_pixel(
                result.storm_lat, result.storm_lon, screen_w, screen_h,
            );
            let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);
            if !rect.contains(pos) { continue; }

            let risk = result.risk_level();
            let color = match risk {
                "EXTREME" => egui::Color32::from_rgb(255, 0, 0),
                "HIGH" => egui::Color32::from_rgb(255, 120, 0),
                "MODERATE" => egui::Color32::from_rgb(255, 200, 0),
                "LOW" => egui::Color32::from_rgb(100, 200, 0),
                _ => egui::Color32::GRAY,
            };

            // Pulsing ring for top results
            let radius = if i < 5 { 14.0 } else { 10.0 };
            ui.painter().circle_stroke(pos, radius, egui::Stroke::new(2.5, color));
            if i < 5 {
                ui.painter().circle_stroke(pos, radius + 4.0, egui::Stroke::new(1.0, color.linear_multiply(0.5)));
            }

            // Label
            let score = result.risk_score();
            let label = format!("#{} {} {:.0}%", i + 1, result.station, score * 100.0);
            ui.painter().text(
                pos + egui::vec2(radius + 4.0, -6.0),
                egui::Align2::LEFT_CENTER,
                &label,
                egui::FontId::proportional(11.0),
                color,
            );

            // Sub-label with details
            let detail = if let Some(ref p) = result.prediction {
                format!("ML: {}", p.risk_level())
            } else {
                format!("{}M {}T", result.meso_count, result.tvs_count)
            };
            ui.painter().text(
                pos + egui::vec2(radius + 4.0, 6.0),
                egui::Align2::LEFT_CENTER,
                &detail,
                egui::FontId::proportional(9.0),
                color.linear_multiply(0.7),
            );
        }

        // Scanning progress indicator
        if self.autoscan.scanning.load(std::sync::atomic::Ordering::Relaxed) {
            let scanned = self.autoscan.radars_scanned.load(std::sync::atomic::Ordering::Relaxed);
            let total = self.autoscan.radars_total.load(std::sync::atomic::Ordering::Relaxed);
            let text = format!("Scanning {}/{}...", scanned, total);
            ui.painter().text(
                egui::pos2(rect.right() - 10.0, rect.top() + 20.0),
                egui::Align2::RIGHT_TOP,
                text,
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(0, 200, 255),
            );
        }
    }

    fn check_sounding_result(&mut self, ctx: &egui::Context) {
        // Only check when we don't already have a texture and a fetch was started
        if self.sounding_texture.is_some() || !self.sounding_pending {
            return;
        }

        // If the fetch is still in progress, nothing to do yet.
        if self.sounding_fetcher.is_fetching() {
            return;
        }

        // Fetch is complete. Check if we got a profile.
        if let Some(profile) = self.sounding_fetcher.profile() {
            // Clear the result so we don't re-render every frame
            *self.sounding_fetcher.result.lock().unwrap() = None;
            // Render Skew-T diagram — wrap in catch_unwind to prevent crash
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::render::skewt::SkewTRenderer::render(&profile, 900, 700)
            })) {
                Ok(pixels) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([900, 700], &pixels);
                    self.sounding_texture = Some(ctx.load_texture(
                        "sounding", image, egui::TextureOptions::LINEAR,
                    ));
                    self.sounding_pending = false;
                    log::info!("Sounding loaded: CAPE={:.0} CIN={:.0} SRH={:.0}",
                        profile.params.sb_cape, profile.params.sb_cin, profile.params.srh_01);
                }
                Err(_) => {
                    log::error!("Skew-T render panicked — showing error to user");
                    self.sounding_pending = true; // keep window open to show error
                }
            }
        } else {
            // Fetch completed but no data — the sounding window will show the "no data" message.
            // Keep sounding_pending true so the window stays open.
        }
    }

    fn fetch_hrrr_frame(&self, ctx: &egui::Context) {
        let fetching = Arc::clone(&self.hrrr_fetching);
        {
            let mut f = fetching.lock().unwrap();
            if *f { return; }
            *f = true;
        }

        let result = Arc::clone(&self.hrrr_result);
        let status = Arc::clone(&self.hrrr_status);
        let field_idx = self.hrrr_field_idx;
        let composite = self.hrrr_composite.clone();
        let fhour = self.hrrr_forecast_hour;
        let ctx = ctx.clone();

        std::thread::spawn(move || {
            *status.lock().unwrap() = "Fetching...".to_string();
            ctx.request_repaint();

            let render_result = if let Some(ref comp_name) = composite {
                // Composite field
                match hrrr_render::fetch::parse_run("latest") {
                    Ok((date, run_hour)) => {
                        let status_fn = |msg: &str| {
                            *status.lock().unwrap() = msg.to_string();
                            ctx.request_repaint();
                        };
                        hrrr_render::composite::compute_composite(
                            comp_name, &date, run_hour, fhour, &status_fn
                        ).and_then(|(values, _nx, _ny)| {
                            let comp_def = hrrr_render::composite::COMPOSITE_FIELDS.iter()
                                .find(|c| c.name == comp_name.as_str())
                                .ok_or_else(|| std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    format!("Unknown composite: {}", comp_name)
                                ))?;
                            let tmp_field = hrrr_render::fields::FieldDef {
                                name: comp_def.name,
                                label: comp_def.label,
                                unit: comp_def.unit,
                                discipline: 0, category: 0, number: 0,
                                idx_name: "", level: "",
                                value_range: comp_def.value_range,
                                kelvin_to_fahrenheit: false,
                                group: comp_def.group,
                            };
                            let proj = hrrr_render::render::projection::LambertProjection::hrrr_default();
                            Ok(hrrr_render::render::render_to_pixels(&values, &tmp_field, &proj, 1799, 1059))
                        })
                    }
                    Err(e) => Err(e),
                }
            } else {
                // Regular field
                let field = hrrr_render::fields::FIELDS[field_idx].clone();
                match hrrr_render::fetch::parse_run("latest") {
                    Ok((date, run_hour)) => {
                        *status.lock().unwrap() = format!("Fetching {} f{:02}...", field.label, fhour);
                        ctx.request_repaint();

                        hrrr_render::fetch::fetch_field(&date, run_hour, fhour, field.idx_name, field.level)
                            .and_then(|data| hrrr_render::parse_grib2_field(&data))
                            .map(|(mut values, nx, ny)| {
                                hrrr_render::fields::convert_values(&field, &mut values);
                                let proj = hrrr_render::render::projection::LambertProjection::new(
                                    38.5, 38.5, -97.5, 21.138, -122.72,
                                    3000.0, 3000.0, nx as u32, ny as u32,
                                );
                                hrrr_render::render::render_to_pixels(&values, &field, &proj, 1799, 1059)
                            })
                    }
                    Err(e) => Err(e),
                }
            };

            match render_result {
                Ok((pixel_buf, img_width, img_height)) => {
                    let flat: Vec<u8> = pixel_buf.iter()
                        .flat_map(|c| c.iter().copied()).collect();
                    *result.lock().unwrap() = Some(HrrrFrame {
                        pixels: flat,
                        width: img_width,
                        height: img_height,
                    });
                    *status.lock().unwrap() = "Done".to_string();
                }
                Err(e) => {
                    *status.lock().unwrap() = format!("Error: {}", e);
                }
            }
            *fetching.lock().unwrap() = false;
            ctx.request_repaint();
        });
    }
}
