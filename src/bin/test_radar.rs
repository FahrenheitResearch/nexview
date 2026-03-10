//! Quick diagnostic tool - downloads a NEXRAD file, parses it, renders to PNG
use nexview::nexrad::{self, Level2File, RadarProduct};
use nexview::render::{RadarRenderer, ColorTable};

fn main() {
    env_logger::init();

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Download a real NEXRAD file
    println!("Downloading NEXRAD file...");
    let raw = rt.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent("NexView/0.1 test")
            .build()
            .unwrap();

        // List files
        let url = "https://unidata-nexrad-level2.s3.amazonaws.com/?list-type=2&prefix=2026/03/09/KTLX/&max-keys=3";
        let resp = client.get(url).send().await.unwrap();
        let body = resp.text().await.unwrap();

        let key = extract_key(&body).expect("No files found!");
        println!("Found file: {}", key);

        let file_url = format!("https://unidata-nexrad-level2.s3.amazonaws.com/{}", key);
        let resp = client.get(&file_url).send().await.unwrap();
        let bytes = resp.bytes().await.unwrap();
        println!("Downloaded {} bytes", bytes.len());
        bytes.to_vec()
    });

    // Parse
    println!("\n=== PARSING ===");
    println!("First 24 bytes: {:02X?}", &raw[..24.min(raw.len())]);
    println!("Header: '{}'", String::from_utf8_lossy(&raw[..12.min(raw.len())]));

    let file = match Level2File::parse(&raw) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("PARSE ERROR: {}", e);
            return;
        }
    };

    println!("Station: {}", file.station_id);
    println!("Sweeps: {}", file.sweeps.len());

    for (i, sweep) in file.sweeps.iter().enumerate() {
        println!("\n  Sweep {} (elev #{}):", i, sweep.elevation_number);
        println!("    Elevation angle: {:.2}°", sweep.elevation_angle);
        println!("    Radials: {}", sweep.radials.len());

        if let Some(r) = sweep.radials.first() {
            println!("    First radial: az={:.2}°, el={:.2}°, spacing={:.2}°",
                r.azimuth, r.elevation, r.azimuth_spacing);
            println!("    Moments: {}", r.moments.len());
            for m in &r.moments {
                let valid = m.data.iter().filter(|v| !v.is_nan()).count();
                let (min, max) = m.data.iter()
                    .filter(|v| !v.is_nan())
                    .fold((f32::MAX, f32::MIN), |(mn, mx), &v| (mn.min(v), mx.max(v)));
                println!("      {:?}: {} gates, first_range={}m, gate_size={}m, valid={}/{}, range=[{:.1}, {:.1}]",
                    m.product, m.gate_count, m.first_gate_range, m.gate_size, valid, m.data.len(), min, max);
            }
        }

        if sweep.radials.len() > 1 {
            let azimuths: Vec<f32> = sweep.radials.iter().map(|r| r.azimuth).collect();
            println!("    Azimuth range: {:.1}° - {:.1}°",
                azimuths.iter().cloned().fold(f32::MAX, f32::min),
                azimuths.iter().cloned().fold(f32::MIN, f32::max));
        }
    }

    // Render first sweep reflectivity
    println!("\n=== RENDERING ===");
    let sweep = &file.sweeps[0];
    let site = nexrad::sites::find_site(&file.station_id)
        .unwrap_or_else(|| {
            println!("Site not found for '{}', using KTLX", file.station_id);
            nexrad::sites::find_site("KTLX").unwrap()
        });

    for product in &[RadarProduct::Reflectivity, RadarProduct::Velocity] {
        let rendered = RadarRenderer::render_sweep(sweep, *product, site, 1024);
        match rendered {
            Some(img) => {
                let non_zero = img.pixels.chunks(4).filter(|p| p[3] > 0).count();
                let total = (img.width * img.height) as usize;
                println!("{:?}: {}x{}, filled pixels: {}/{} ({:.1}%)",
                    product, img.width, img.height, non_zero, total,
                    100.0 * non_zero as f64 / total as f64);

                let name = format!("test_{:?}.png", product);
                let path = format!("C:\\Users\\drew\\gr2rust\\{}", name);
                let img_buf = image::RgbaImage::from_raw(img.width, img.height, img.pixels).unwrap();
                img_buf.save(&path).unwrap();
                println!("  Saved to {}", path);
            }
            None => println!("{:?}: render returned None", product),
        }
    }
}

fn extract_key(xml: &str) -> Option<String> {
    let start = xml.find("<Key>")? + 5;
    let end = xml[start..].find("</Key>")? + start;
    Some(xml[start..end].to_string())
}
