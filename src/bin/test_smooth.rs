/// Test binary: renders a radar sweep in both classic and smooth modes,
/// saves side-by-side comparison PNGs.
use nexview::nexrad::{Level2File, RadarProduct};
use nexview::nexrad::sites::RADAR_SITES;
use nexview::render::radar::RadarRenderer;
use nexview::render::ColorTable;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Fetch latest radar data for KTLX (Oklahoma City)
    let station = "KTLX";
    let site = RADAR_SITES.iter().find(|s| s.id == station)
        .expect("Station not found");

    println!("Fetching latest {} data from S3...", station);

    let data = rt.block_on(async {
        let http = reqwest::Client::builder()
            .user_agent("NexView/test")
            .build()
            .unwrap();

        let now = chrono::Utc::now();
        let prefix = format!(
            "{:04}/{:02}/{:02}/{}/",
            chrono::Datelike::year(&now),
            chrono::Datelike::month(&now),
            chrono::Datelike::day(&now),
            station
        );

        let url = format!(
            "https://unidata-nexrad-level2.s3.amazonaws.com?list-type=2&prefix={}",
            prefix
        );

        let resp = http.get(&url).send().await.unwrap().text().await.unwrap();

        // Parse XML to find the last key
        let mut keys: Vec<String> = Vec::new();
        for line in resp.split("<Key>") {
            if let Some(end) = line.find("</Key>") {
                let key = &line[..end];
                if !key.ends_with("_MDM") && !key.contains("NXL2") {
                    keys.push(key.to_string());
                }
            }
        }

        keys.sort();
        let last_key = keys.last().expect("No files found for today");
        println!("Downloading: {}", last_key);

        let file_url = format!(
            "https://unidata-nexrad-level2.s3.amazonaws.com/{}",
            last_key
        );
        let bytes = http.get(&file_url).send().await.unwrap().bytes().await.unwrap();
        bytes.to_vec()
    });

    println!("Parsing Level2 file ({} bytes)...", data.len());
    let file = Level2File::parse(&data).expect("Failed to parse Level2 file");
    println!("Parsed {} sweeps", file.sweeps.len());

    // Find the lowest REF sweep
    let product = RadarProduct::SuperResReflectivity;
    let sweep_idx = file.sweeps.iter().position(|s| {
        s.radials.iter().any(|r| r.moments.iter().any(|m| m.product == product))
    });

    // Fall back to regular reflectivity
    let (sweep_idx, product) = match sweep_idx {
        Some(i) => (i, product),
        None => {
            let p = RadarProduct::Reflectivity;
            let i = file.sweeps.iter().position(|s| {
                s.radials.iter().any(|r| r.moments.iter().any(|m| m.product == p))
            }).expect("No reflectivity data found");
            (i, p)
        }
    };

    let sweep = &file.sweeps[sweep_idx];
    let image_size = 1024u32;
    let color_table = ColorTable::for_product(product);

    println!("Rendering sweep {} ({:.1} deg, {} radials) at {}x{}...",
        sweep_idx, sweep.elevation_angle, sweep.radials.len(), image_size, image_size);

    // Render classic mode
    let t0 = std::time::Instant::now();
    let classic = RadarRenderer::render_sweep_with_table(
        sweep, product, site, image_size, &color_table,
    ).expect("Classic render failed");
    let classic_ms = t0.elapsed().as_millis();
    println!("Classic render: {}ms", classic_ms);

    // Render smooth mode
    let t0 = std::time::Instant::now();
    let smooth = RadarRenderer::render_sweep_smooth(
        sweep, product, site, image_size, &color_table,
    ).expect("Smooth render failed");
    let smooth_ms = t0.elapsed().as_millis();
    println!("Smooth render: {}ms", smooth_ms);

    // Save as PNGs
    let out_dir = std::path::Path::new("test_output");
    std::fs::create_dir_all(out_dir).ok();

    save_png(&classic.pixels, image_size, &out_dir.join("classic.png"));
    save_png(&smooth.pixels, image_size, &out_dir.join("smooth.png"));

    // Create a side-by-side comparison
    let combined_width = image_size * 2 + 4; // 4px gap
    let mut combined = vec![0u8; (combined_width * image_size * 4) as usize];
    for y in 0..image_size as usize {
        // Left: classic
        let src_start = y * image_size as usize * 4;
        let dst_start = y * combined_width as usize * 4;
        combined[dst_start..dst_start + image_size as usize * 4]
            .copy_from_slice(&classic.pixels[src_start..src_start + image_size as usize * 4]);

        // Gap: white line
        for g in 0..4 {
            let gi = dst_start + (image_size as usize + g) * 4;
            combined[gi] = 255;
            combined[gi + 1] = 255;
            combined[gi + 2] = 255;
            combined[gi + 3] = 255;
        }

        // Right: smooth
        let right_start = dst_start + (image_size as usize + 4) * 4;
        combined[right_start..right_start + image_size as usize * 4]
            .copy_from_slice(&smooth.pixels[src_start..src_start + image_size as usize * 4]);
    }

    save_png(&combined, combined_width, &out_dir.join("comparison.png"));
    println!("\nSaved to test_output/:");
    println!("  classic.png    - inverse-mapped (current renderer)");
    println!("  smooth.png     - forward-mapped triangulation");
    println!("  comparison.png - side by side (classic | smooth)");
}

fn save_png(pixels: &[u8], width: u32, path: &std::path::Path) {
    let height = pixels.len() as u32 / (width * 4);
    let img: image::RgbaImage = image::ImageBuffer::from_raw(width, height, pixels.to_vec())
        .expect("Failed to create image buffer");
    img.save(path).expect("Failed to save PNG");
}
