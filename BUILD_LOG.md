# NexView Build Log

## Build 6: (uncommitted) - Multi-radar synced animation + composite GIF
- **Branch**: perf-operational
- **Date**: 2026-03-10
- **Status**: ✅ Builds clean (ML + non-ML)
- **Changes**:
  - RadarInstance expanded: anim_frames, anim_textures, anim_timestamps_ms, per-radar opacity
  - RadarApp: sync_timeline, sync_frame_map for unified multi-radar animation
  - load_multi_radar_animation(): parallel S3 download for all active radars
  - build_sync_timeline(): matches secondary frames to primary by closest timestamp (5-min window)
  - sync_secondary_to_frame(): swaps all secondary radar files/textures on frame advance
  - export_composite_gif(): renders all radars onto 1920x1080 canvas with alpha compositing
  - composite_radar_to_canvas(): geographic projection + alpha blending per radar
  - Timeline: multi-radar loading progress, radar count indicator, frame scrub syncs all
  - Sidebar: per-radar opacity sliders, group radar list with remove buttons, loading progress bars
  - Toolbar: "Group GIF" button label when multi-radar active
  - Level2File::unix_timestamp_ms() for timestamp matching

## Build 5: (uncommitted) - Custom color table loading UI + GR2/RadarScope format
- **Branch**: all
- **Date**: 2026-03-10
- **Status**: ✅ Builds clean (ML + non-ML)
- **Changes**:
  - rfd file dialog added for native file picking
  - from_pal_string: full GRLevelX/RadarScope .pal parser (Color:, Color4:, SolidColor:, SolidColor4:, Scale:, Offset:, gradients, semicolons)
  - Sidebar: "Load Custom..." button with file dialog, auto-selects loaded table, "Delete" for custom
  - Supports .pal, .pal3, .wctpal, .csv, .txt extensions

## Build 4: a8f0f63 - Custom color tables with LUT, presets, manager
- **Branch**: all (ml-bugfixes, ml-prediction, perf-operational, master)
- **Date**: 2026-03-10
- **Status**: ✅ Builds clean (ML + non-ML)
- **Changes** (cherry-picked from agent work on perf-feat-colortables):
  - color_table.rs: Complete rewrite with O(1) LUT-based lookup (4096 entries)
  - 6 built-in presets: NWS Default, GR2Analyst, NSSL, Classic, Dark, Colorblind
  - ColorTableManager with JSON persistence (%APPDATA%/nexview/colortables.json)
  - .pal and .csv custom color table import
  - Sidebar UI: preset selector dropdown, custom file loader, delete custom tables
  - panels.rs + sidebar.rs updated to use ColorTableManager

## Build 3: f3972cb - Color table + PHI→KDP fixes
- **Branch**: ml-bugfixes, ml-prediction, perf-operational, master
- **Date**: 2026-03-10
- **Status**: ✅ Builds clean (ML + non-ML)
- **Changes**:
  - color_table.rs: O(log n) binary search for color lookup
  - convert.rs: reusable PHI scratch buffer, KDP edge backward difference
- **Build commands**:
  - ML: `cargo build --release --features tornado-predict`
  - Perf: `cargo build --release`

## Build 2: f19f8a2 - Parser safety + UX bugfixes
- **Branch**: ml-bugfixes
- **Date**: 2026-03-10
- **Status**: ✅ Builds clean
- **Changes**:
  - level2.rs: byte comparison for block type (avoids String alloc per radial)
  - level2.rs: scale != 0 guard prevents division by zero
  - app.rs: drain() instead of remove(0) for prediction history
  - app.rs: Shift+M for measure mode (M alone = mosaic, fixes key conflict)
  - app.rs: clear prediction state on station switch

## Build 1: 332c85c - ML tornado prediction platform
- **Branch**: master (original)
- **Date**: 2026-03-09
- **Status**: ✅ Builds clean
- **Features**: Swin3D support, auto-infer, Ctrl+Click targeting, 8-frame buffer

## Fork Architecture
- **ml-prediction**: ML tornado prediction platform for model testing (Swin3D, ResNet3D, future models)
  - Build: `cargo build --release --features tornado-predict`
- **perf-operational**: Performance-focused pure radar for operational meteorologists
  - Build: `cargo build --release` (no ML features)
- Both forks share the same codebase; ML is behind `tornado-predict` feature flag

## Known Issues (from audit, not yet fixed)
- Level2File cloning (~25-50MB each) in animation/prediction paths - biggest perf win would be Arc/index-based approach
- Smooth rendering algorithm not yet defaulted on
- Multi-radar loading needs hardening

## Agent work available but not yet integrated
- **ml-feat-uipolish (a257210)**: Model name overlay, risk gauge, inference time, Escape dismiss, "Clear All" button
- **perf-feat-optimize (dd9f938, 1ebc88b, 1adbc5f)**: Parallel smooth render (rayon par_iter + AtomicU32), smooth default on, #[cfg] guards on ML UI, LRU cache with 2GB budget, save-on-switch, station status tracking
- **ml-feat-multiradar (f43eb35)**: Per-station prediction buffers (HashMap), preserves prediction state across station switches, merged auto-infer results
