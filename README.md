# NexView

Open-source NEXRAD Level 2 weather radar viewer written in Rust. Downloads real-time radar data directly from AWS and renders it with map overlays.

## Features

- **Real-time NEXRAD data** — streams directly from NOAA's public AWS S3 bucket (no API key needed)
- **All dual-pol products** — REF, VEL, SW, ZDR, CC, KDP
- **Quad view** — display 4 products simultaneously
- **Interactive map** — OpenStreetMap tiles with Web Mercator projection, pan/zoom
- **130+ radar sites** — click to switch, or search by name/state
- **Fast** — parallel bzip2 decompression (rayon), line-drawing renderer, GPU-accelerated display (wgpu)
- **Keyboard shortcuts** — arrow keys for tilt/product cycling, Q for quad toggle
- **Persistent settings** — save your default station, zoom level, and view preferences

## Performance

On a typical machine:
- Download: ~700ms (6MB file from S3)
- Parse: ~120ms (parallel bzip2 decompression)
- Render: ~60ms (4 products in quad view)

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Up/Down | Tilt up/down (elevation sweeps) |
| Left/Right | Cycle through products |
| Q | Toggle quad view |

## Building

```bash
cargo build --release
```

The binary is fully self-contained with no runtime dependencies.

## How It Works

NexView parses NEXRAD Level 2 Archive II files (ICD 2620010H) directly from binary. The parser handles:
- Volume header extraction
- Message Type 31 (Digital Radar Data) parsing
- Data moment block decoding with proper scale/offset
- bzip2 block decompression (parallelized across CPU cores)

Radar data is rendered using a fast line-drawing algorithm that traces radial lines across each gate's azimuth span, avoiding expensive per-pixel trigonometry.

## Data Source

All radar data comes from NOAA's public NEXRAD Level 2 archive on AWS S3 (`unidata-nexrad-level2` bucket). No authentication or API keys required.

## License

MIT
