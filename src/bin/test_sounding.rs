/// Quick test: fetch a sounding and render it, save as PNG.
use nexview::sounding::profile::SoundingProfile;
use nexview::sounding::profile::SoundingLevel;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Test with Oklahoma City coordinates
    let lat = 35.18;
    let lon = -97.44;

    println!("=== Testing sounding fetch at ({}, {}) ===", lat, lon);

    // Build HTTP client matching SoundingFetcher
    let http = reqwest::Client::builder()
        .user_agent("NexView/test")
        .timeout(std::time::Duration::from_secs(15))
        .danger_accept_invalid_certs(true)
        .build()
        .expect("Failed to build HTTP client");

    let result = rt.block_on(async {
        // Try IEM first (same as the app does now)
        let now = chrono::Utc::now();
        let today = now.format("%Y%m%d").to_string();
        let yesterday = (now - chrono::Duration::hours(24)).format("%Y%m%d").to_string();
        let hour = chrono::Timelike::hour(&now);

        let mut timestamps = Vec::new();
        if hour >= 12 {
            timestamps.push(format!("{today}1200"));
            timestamps.push(format!("{today}0000"));
        } else {
            timestamps.push(format!("{today}0000"));
            timestamps.push(format!("{yesterday}1200"));
        }
        timestamps.push(format!("{yesterday}0000"));

        // Try IEM
        for ts in &timestamps {
            let station = "KOUN";
            let url = format!(
                "https://mesonet.agron.iastate.edu/json/raob.py?station={station}&ts={ts}"
            );
            println!("Trying IEM: {url}");

            match http.get(&url).send().await {
                Ok(resp) => {
                    println!("  HTTP {}", resp.status());
                    if resp.status().is_success() {
                        match resp.text().await {
                            Ok(body) => {
                                println!("  Body length: {} bytes", body.len());
                                // Try to parse
                                let json: serde_json::Value = match serde_json::from_str(&body) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        println!("  JSON parse error: {e}");
                                        continue;
                                    }
                                };
                                let profiles = json.get("profiles").and_then(|p| p.as_array());
                                if let Some(profiles) = profiles {
                                    println!("  Found {} profiles", profiles.len());
                                    if let Some(first) = profiles.first() {
                                        let profile_arr = first.get("profile").and_then(|p| p.as_array());
                                        if let Some(arr) = profile_arr {
                                            println!("  Profile has {} levels", arr.len());

                                            // Parse levels
                                            let mut levels: Vec<SoundingLevel> = Vec::new();
                                            for entry in arr {
                                                let pres = entry.get("pres").and_then(|v| v.as_f64());
                                                let hght = entry.get("hght").and_then(|v| v.as_f64());
                                                let tmpc = entry.get("tmpc").and_then(|v| v.as_f64());
                                                let dwpc = entry.get("dwpc").and_then(|v| v.as_f64());
                                                let drct = entry.get("drct").and_then(|v| v.as_f64());
                                                let sknt = entry.get("sknt").and_then(|v| v.as_f64());

                                                if let (Some(p), Some(h), Some(t), Some(td)) = (pres, hght, tmpc, dwpc) {
                                                    if p > 0.0 && p < 1100.0 && h > -1000.0
                                                        && t.is_finite() && td.is_finite()
                                                    {
                                                        levels.push(SoundingLevel {
                                                            pressure_mb: p as f32,
                                                            height_m: h as f32,
                                                            temp_c: t as f32,
                                                            dewpoint_c: td as f32,
                                                            wind_dir: drct.unwrap_or(0.0) as f32,
                                                            wind_speed_kts: sknt.unwrap_or(0.0) as f32,
                                                        });
                                                    }
                                                }
                                            }

                                            levels.sort_by(|a, b| b.pressure_mb.partial_cmp(&a.pressure_mb)
                                                .unwrap_or(std::cmp::Ordering::Equal));

                                            println!("  Parsed {} valid levels", levels.len());
                                            if levels.len() >= 3 {
                                                // Print first/last few
                                                for l in levels.iter().take(3) {
                                                    println!("    {:.0} mb  {:.0} m  T={:.1}  Td={:.1}",
                                                        l.pressure_mb, l.height_m, l.temp_c, l.dewpoint_c);
                                                }
                                                println!("    ...");
                                                for l in levels.iter().rev().take(2).collect::<Vec<_>>().iter().rev() {
                                                    println!("    {:.0} mb  {:.0} m  T={:.1}  Td={:.1}",
                                                        l.pressure_mb, l.height_m, l.temp_c, l.dewpoint_c);
                                                }

                                                println!("\n=== Creating SoundingProfile (calls compute_all) ===");
                                                let t0 = std::time::Instant::now();
                                                let profile = SoundingProfile::new(
                                                    levels,
                                                    station.to_string(),
                                                    ts.clone(),
                                                    lat, lon,
                                                );
                                                println!("  compute_all took {:.0}ms", t0.elapsed().as_millis());
                                                println!("  SBCAPE={:.0} MLCAPE={:.0} MUCAPE={:.0}",
                                                    profile.params.sb_cape, profile.params.ml_cape, profile.params.mu_cape);
                                                println!("  SRH01={:.0} SRH03={:.0} STP={:.2}",
                                                    profile.params.srh_01, profile.params.srh_03, profile.params.stp_fixed);

                                                println!("\n=== Rendering Skew-T ===");
                                                let t1 = std::time::Instant::now();
                                                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                                    nexview::render::skewt::SkewTRenderer::render(&profile, 900, 700)
                                                })) {
                                                    Ok(pixels) => {
                                                        println!("  Render took {:.0}ms, {} bytes",
                                                            t1.elapsed().as_millis(), pixels.len());
                                                        // Save as PNG
                                                        let img = image::RgbaImage::from_raw(900, 700, pixels)
                                                            .expect("Failed to create image");
                                                        img.save("test_sounding.png").expect("Failed to save PNG");
                                                        println!("  Saved to test_sounding.png");
                                                        return Some(profile);
                                                    }
                                                    Err(e) => {
                                                        println!("  RENDER PANICKED: {:?}", e);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => println!("  Body read error: {e}"),
                        }
                    }
                }
                Err(e) => println!("  Fetch error: {e}"),
            }
        }

        // Try rucsoundings
        let url = format!(
            "https://rucsoundings.noaa.gov/get_soundings.cgi?\
             data_source=Op40&latest=latest&start_sounding=latest&\
             n_hrs=1.0&fcst_len=shortest&airport={lat}%2C{lon}&\
             text=Ascii%20text%20%28GSD%20format%29"
        );
        println!("\nTrying rucsoundings: {url}");
        match http.get(&url).send().await {
            Ok(resp) => {
                println!("  HTTP {}", resp.status());
                match resp.text().await {
                    Ok(body) => println!("  Body length: {} bytes\n  First 200 chars: {}", body.len(), &body[..body.len().min(200)]),
                    Err(e) => println!("  Body error: {e}"),
                }
            }
            Err(e) => println!("  Fetch error: {e}"),
        }

        None::<SoundingProfile>
    });

    if result.is_some() {
        println!("\n=== SUCCESS ===");
    } else {
        println!("\n=== FAILED — no sounding data retrieved ===");
    }
}
