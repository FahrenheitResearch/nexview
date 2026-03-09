use std::fs::File;
use std::io::BufWriter;

/// Export a sequence of RGBA frames as an animated GIF.
///
/// `frames` - slices of egui::ColorImage (all must be the same dimensions)
/// `delay_ms` - delay between frames in milliseconds
/// `path` - output file path
pub fn export_animation_gif(
    frames: &[egui::ColorImage],
    delay_ms: u16,
    path: &str,
) -> Result<(), String> {
    if frames.is_empty() {
        return Err("No frames to export".into());
    }

    let width = frames[0].width() as u16;
    let height = frames[0].height() as u16;

    let file = File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;
    let writer = BufWriter::new(file);

    let mut encoder = gif::Encoder::new(writer, width, height, &[])
        .map_err(|e| format!("Failed to create GIF encoder: {}", e))?;

    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| format!("Failed to set repeat: {}", e))?;

    // GIF delay is in centiseconds (10ms units)
    let delay_cs = (delay_ms + 5) / 10;

    for (i, frame) in frames.iter().enumerate() {
        if frame.width() as u16 != width || frame.height() as u16 != height {
            return Err(format!(
                "Frame {} has different dimensions ({}x{}) than first frame ({}x{})",
                i,
                frame.width(),
                frame.height(),
                width,
                height
            ));
        }

        // Convert RGBA pixels to indexed color using NeuQuant-style quantization
        // The gif crate expects indexed pixels with a color palette
        let rgba_pixels = &frame.pixels;

        // Build a simple palette by collecting unique colors, clamping to 256
        let mut palette_colors: Vec<[u8; 3]> = Vec::with_capacity(256);
        let mut color_map: std::collections::HashMap<[u8; 3], u8> =
            std::collections::HashMap::new();

        let mut indexed_pixels: Vec<u8> = Vec::with_capacity(rgba_pixels.len());
        // We'll also track transparency
        let mut transparent_index: Option<u8> = None;

        for pixel in rgba_pixels.iter() {
            let r = pixel.r();
            let g = pixel.g();
            let b = pixel.b();
            let a = pixel.a();

            if a < 128 {
                // Transparent pixel
                if transparent_index.is_none() {
                    if palette_colors.len() < 256 {
                        transparent_index = Some(palette_colors.len() as u8);
                        palette_colors.push([0, 0, 0]);
                    } else {
                        transparent_index = Some(0);
                    }
                }
                indexed_pixels.push(transparent_index.unwrap());
            } else {
                let rgb = [r, g, b];
                if let Some(&idx) = color_map.get(&rgb) {
                    indexed_pixels.push(idx);
                } else if palette_colors.len() < 256 {
                    let idx = palette_colors.len() as u8;
                    palette_colors.push(rgb);
                    color_map.insert(rgb, idx);
                    indexed_pixels.push(idx);
                } else {
                    // Palette full - find nearest color
                    let nearest = find_nearest_color(&palette_colors, rgb);
                    indexed_pixels.push(nearest);
                }
            }
        }

        // Pad palette to power of 2 (GIF requirement)
        while palette_colors.len() < 2 {
            palette_colors.push([0, 0, 0]);
        }
        let next_pow2 = palette_colors.len().next_power_of_two();
        while palette_colors.len() < next_pow2 {
            palette_colors.push([0, 0, 0]);
        }

        // Flatten palette to [r, g, b, r, g, b, ...]
        let flat_palette: Vec<u8> = palette_colors.iter().flat_map(|c| c.iter().copied()).collect();

        let mut gif_frame = gif::Frame::default();
        gif_frame.width = width;
        gif_frame.height = height;
        gif_frame.delay = delay_cs;
        gif_frame.dispose = gif::DisposalMethod::Background;
        gif_frame.buffer = std::borrow::Cow::Borrowed(&indexed_pixels);
        gif_frame.palette = Some(flat_palette);

        if let Some(ti) = transparent_index {
            gif_frame.transparent = Some(ti);
        }

        encoder
            .write_frame(&gif_frame)
            .map_err(|e| format!("Failed to write frame {}: {}", i, e))?;
    }

    Ok(())
}

/// Find the index of the nearest color in the palette using simple Euclidean distance.
fn find_nearest_color(palette: &[[u8; 3]], target: [u8; 3]) -> u8 {
    let mut best_idx = 0u8;
    let mut best_dist = u32::MAX;

    for (i, color) in palette.iter().enumerate() {
        let dr = (color[0] as i32 - target[0] as i32).unsigned_abs();
        let dg = (color[1] as i32 - target[1] as i32).unsigned_abs();
        let db = (color[2] as i32 - target[2] as i32).unsigned_abs();
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as u8;
        }
    }

    best_idx
}
