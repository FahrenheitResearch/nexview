use crate::nexrad::RadarProduct;
use std::path::Path;

/// Named preset styles for each product
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ColorTablePreset {
    Default,
    Dark,
    Colorblind,
}

impl ColorTablePreset {
    pub fn all() -> &'static [Self] {
        &[Self::Default, Self::Dark, Self::Colorblind]
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Default => "Default",
            Self::Dark => "Dark",
            Self::Colorblind => "Colorblind",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColorTable {
    pub name: String,
    pub entries: Vec<ColorEntry>,
    pub min_value: f32,
    pub max_value: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ColorEntry {
    pub value: f32,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorTable {
    pub fn for_product(product: RadarProduct) -> Self {
        match product {
            RadarProduct::Reflectivity => Self::reflectivity_table(),
            RadarProduct::Velocity | RadarProduct::StormRelativeVelocity => Self::velocity_table(),
            RadarProduct::SpectrumWidth => Self::spectrum_width_table(),
            RadarProduct::DifferentialReflectivity => Self::zdr_table(),
            RadarProduct::CorrelationCoefficient => Self::cc_table(),
            RadarProduct::SpecificDiffPhase => Self::kdp_table(),
            RadarProduct::VIL => Self::vil_table(),
            RadarProduct::EchoTops => Self::echo_tops_table(),
            _ => Self::reflectivity_table(),
        }
    }

    pub fn color_for_value(&self, value: f32) -> [u8; 4] {
        if value.is_nan() || value < self.min_value {
            return [0, 0, 0, 0]; // transparent
        }

        // Find the two entries that bracket this value
        let mut lower = &self.entries[0];
        let mut upper = &self.entries[0];

        for entry in &self.entries {
            if entry.value <= value {
                lower = entry;
            }
            if entry.value >= value {
                upper = entry;
                break;
            }
        }

        // Interpolate between lower and upper
        if (upper.value - lower.value).abs() < 0.001 {
            return [lower.r, lower.g, lower.b, lower.a];
        }

        let t = ((value - lower.value) / (upper.value - lower.value)).clamp(0.0, 1.0);
        let r = (lower.r as f32 + t * (upper.r as f32 - lower.r as f32)) as u8;
        let g = (lower.g as f32 + t * (upper.g as f32 - lower.g as f32)) as u8;
        let b = (lower.b as f32 + t * (upper.b as f32 - lower.b as f32)) as u8;
        let a = (lower.a as f32 + t * (upper.a as f32 - lower.a as f32)) as u8;

        [r, g, b, a]
    }

    fn reflectivity_table() -> Self {
        // Standard NWS reflectivity color table
        ColorTable {
            name: "Reflectivity".into(),
            min_value: -30.0,
            max_value: 80.0,
            entries: vec![
                ColorEntry { value: -30.0, r: 0, g: 0, b: 0, a: 0 },
                ColorEntry { value: -20.0, r: 100, g: 100, b: 100, a: 180 },
                ColorEntry { value: -10.0, r: 150, g: 150, b: 150, a: 200 },
                ColorEntry { value: 0.0, r: 118, g: 118, b: 118, a: 220 },
                ColorEntry { value: 5.0, r: 0, g: 236, b: 236, a: 255 },
                ColorEntry { value: 10.0, r: 1, g: 160, b: 246, a: 255 },
                ColorEntry { value: 15.0, r: 0, g: 0, b: 246, a: 255 },
                ColorEntry { value: 20.0, r: 0, g: 255, b: 0, a: 255 },
                ColorEntry { value: 25.0, r: 0, g: 200, b: 0, a: 255 },
                ColorEntry { value: 30.0, r: 0, g: 144, b: 0, a: 255 },
                ColorEntry { value: 35.0, r: 255, g: 255, b: 0, a: 255 },
                ColorEntry { value: 40.0, r: 231, g: 192, b: 0, a: 255 },
                ColorEntry { value: 45.0, r: 255, g: 144, b: 0, a: 255 },
                ColorEntry { value: 50.0, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: 55.0, r: 214, g: 0, b: 0, a: 255 },
                ColorEntry { value: 60.0, r: 192, g: 0, b: 0, a: 255 },
                ColorEntry { value: 65.0, r: 255, g: 0, b: 255, a: 255 },
                ColorEntry { value: 70.0, r: 153, g: 85, b: 201, a: 255 },
                ColorEntry { value: 75.0, r: 255, g: 255, b: 255, a: 255 },
                ColorEntry { value: 80.0, r: 255, g: 255, b: 255, a: 255 },
            ],
        }
    }

    fn velocity_table() -> Self {
        // Standard velocity color table (-64 to +64 kts)
        ColorTable {
            name: "Velocity".into(),
            min_value: -120.0,
            max_value: 120.0,
            entries: vec![
                ColorEntry { value: -120.0, r: 255, g: 0, b: 255, a: 255 },
                ColorEntry { value: -100.0, r: 200, g: 0, b: 200, a: 255 },
                ColorEntry { value: -80.0, r: 128, g: 0, b: 0, a: 255 },
                ColorEntry { value: -64.0, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: -50.0, r: 192, g: 0, b: 0, a: 255 },
                ColorEntry { value: -36.0, r: 255, g: 127, b: 0, a: 255 },
                ColorEntry { value: -26.0, r: 255, g: 200, b: 0, a: 255 },
                ColorEntry { value: -20.0, r: 255, g: 230, b: 137, a: 255 },
                ColorEntry { value: -10.0, r: 141, g: 0, b: 0, a: 255 },
                ColorEntry { value: -1.0, r: 100, g: 55, b: 55, a: 200 },
                ColorEntry { value: 0.0, r: 0, g: 0, b: 0, a: 0 },
                ColorEntry { value: 1.0, r: 55, g: 100, b: 55, a: 200 },
                ColorEntry { value: 10.0, r: 0, g: 141, b: 0, a: 255 },
                ColorEntry { value: 20.0, r: 137, g: 230, b: 137, a: 255 },
                ColorEntry { value: 26.0, r: 0, g: 200, b: 0, a: 255 },
                ColorEntry { value: 36.0, r: 0, g: 255, b: 127, a: 255 },
                ColorEntry { value: 50.0, r: 0, g: 192, b: 0, a: 255 },
                ColorEntry { value: 64.0, r: 0, g: 0, b: 255, a: 255 },
                ColorEntry { value: 80.0, r: 0, g: 0, b: 128, a: 255 },
                ColorEntry { value: 100.0, r: 0, g: 200, b: 200, a: 255 },
                ColorEntry { value: 120.0, r: 0, g: 255, b: 255, a: 255 },
            ],
        }
    }

    fn spectrum_width_table() -> Self {
        ColorTable {
            name: "Spectrum Width".into(),
            min_value: 0.0,
            max_value: 40.0,
            entries: vec![
                ColorEntry { value: 0.0, r: 0, g: 0, b: 0, a: 0 },
                ColorEntry { value: 2.0, r: 100, g: 100, b: 100, a: 200 },
                ColorEntry { value: 5.0, r: 0, g: 150, b: 0, a: 255 },
                ColorEntry { value: 10.0, r: 0, g: 255, b: 0, a: 255 },
                ColorEntry { value: 15.0, r: 255, g: 255, b: 0, a: 255 },
                ColorEntry { value: 20.0, r: 255, g: 150, b: 0, a: 255 },
                ColorEntry { value: 25.0, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: 30.0, r: 200, g: 0, b: 0, a: 255 },
                ColorEntry { value: 40.0, r: 255, g: 255, b: 255, a: 255 },
            ],
        }
    }

    fn zdr_table() -> Self {
        ColorTable {
            name: "Differential Reflectivity".into(),
            min_value: -8.0,
            max_value: 8.0,
            entries: vec![
                ColorEntry { value: -8.0, r: 0, g: 0, b: 128, a: 255 },
                ColorEntry { value: -4.0, r: 0, g: 0, b: 255, a: 255 },
                ColorEntry { value: -2.0, r: 0, g: 150, b: 255, a: 255 },
                ColorEntry { value: -1.0, r: 0, g: 200, b: 200, a: 255 },
                ColorEntry { value: 0.0, r: 100, g: 100, b: 100, a: 200 },
                ColorEntry { value: 1.0, r: 0, g: 200, b: 0, a: 255 },
                ColorEntry { value: 2.0, r: 255, g: 255, b: 0, a: 255 },
                ColorEntry { value: 4.0, r: 255, g: 128, b: 0, a: 255 },
                ColorEntry { value: 6.0, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: 8.0, r: 200, g: 0, b: 200, a: 255 },
            ],
        }
    }

    fn cc_table() -> Self {
        ColorTable {
            name: "Correlation Coefficient".into(),
            min_value: 0.2,
            max_value: 1.05,
            entries: vec![
                ColorEntry { value: 0.2, r: 0, g: 0, b: 0, a: 0 },
                ColorEntry { value: 0.5, r: 128, g: 0, b: 128, a: 255 },
                ColorEntry { value: 0.7, r: 0, g: 0, b: 200, a: 255 },
                ColorEntry { value: 0.8, r: 0, g: 150, b: 255, a: 255 },
                ColorEntry { value: 0.85, r: 0, g: 200, b: 200, a: 255 },
                ColorEntry { value: 0.90, r: 0, g: 200, b: 0, a: 255 },
                ColorEntry { value: 0.93, r: 255, g: 255, b: 0, a: 255 },
                ColorEntry { value: 0.95, r: 255, g: 128, b: 0, a: 255 },
                ColorEntry { value: 0.97, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: 0.99, r: 200, g: 0, b: 200, a: 255 },
                ColorEntry { value: 1.05, r: 255, g: 255, b: 255, a: 255 },
            ],
        }
    }

    fn kdp_table() -> Self {
        ColorTable {
            name: "Specific Differential Phase".into(),
            min_value: -2.0,
            max_value: 10.0,
            entries: vec![
                ColorEntry { value: -2.0, r: 128, g: 0, b: 128, a: 255 },
                ColorEntry { value: -1.0, r: 0, g: 0, b: 200, a: 255 },
                ColorEntry { value: 0.0, r: 100, g: 100, b: 100, a: 200 },
                ColorEntry { value: 0.5, r: 0, g: 200, b: 0, a: 255 },
                ColorEntry { value: 1.0, r: 0, g: 255, b: 0, a: 255 },
                ColorEntry { value: 2.0, r: 255, g: 255, b: 0, a: 255 },
                ColorEntry { value: 3.0, r: 255, g: 200, b: 0, a: 255 },
                ColorEntry { value: 5.0, r: 255, g: 128, b: 0, a: 255 },
                ColorEntry { value: 7.0, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: 10.0, r: 200, g: 0, b: 200, a: 255 },
            ],
        }
    }

    fn vil_table() -> Self {
        ColorTable {
            name: "VIL".into(),
            min_value: 0.0,
            max_value: 80.0,
            entries: vec![
                ColorEntry { value: 0.0, r: 0, g: 0, b: 0, a: 0 },
                ColorEntry { value: 1.0, r: 0, g: 130, b: 0, a: 200 },
                ColorEntry { value: 5.0, r: 0, g: 200, b: 0, a: 255 },
                ColorEntry { value: 10.0, r: 0, g: 255, b: 0, a: 255 },
                ColorEntry { value: 15.0, r: 200, g: 255, b: 0, a: 255 },
                ColorEntry { value: 25.0, r: 255, g: 255, b: 0, a: 255 },
                ColorEntry { value: 30.0, r: 255, g: 200, b: 0, a: 255 },
                ColorEntry { value: 40.0, r: 255, g: 128, b: 0, a: 255 },
                ColorEntry { value: 50.0, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: 60.0, r: 200, g: 0, b: 0, a: 255 },
                ColorEntry { value: 70.0, r: 180, g: 0, b: 180, a: 255 },
                ColorEntry { value: 80.0, r: 255, g: 0, b: 255, a: 255 },
            ],
        }
    }

    fn echo_tops_table() -> Self {
        ColorTable {
            name: "Echo Tops".into(),
            min_value: 0.0,
            max_value: 20.0,
            entries: vec![
                ColorEntry { value: 0.0, r: 0, g: 0, b: 0, a: 0 },
                ColorEntry { value: 1.0, r: 0, g: 0, b: 180, a: 200 },
                ColorEntry { value: 3.0, r: 0, g: 100, b: 255, a: 255 },
                ColorEntry { value: 5.0, r: 0, g: 200, b: 255, a: 255 },
                ColorEntry { value: 7.0, r: 0, g: 200, b: 0, a: 255 },
                ColorEntry { value: 10.0, r: 0, g: 255, b: 0, a: 255 },
                ColorEntry { value: 12.0, r: 255, g: 255, b: 0, a: 255 },
                ColorEntry { value: 15.0, r: 255, g: 200, b: 0, a: 255 },
                ColorEntry { value: 17.0, r: 255, g: 0, b: 0, a: 255 },
                ColorEntry { value: 20.0, r: 200, g: 0, b: 0, a: 255 },
            ],
        }
    }

    /// Generate a color bar image for the legend (vertical strip)
    pub fn generate_legend_pixels(&self, height: usize) -> Vec<[u8; 4]> {
        let mut pixels = Vec::with_capacity(height);
        for i in 0..height {
            let t = 1.0 - (i as f32 / height as f32); // top = max, bottom = min
            let value = self.min_value + t * (self.max_value - self.min_value);
            pixels.push(self.color_for_value(value));
        }
        pixels
    }

    /// Load from a .pal file (GR2Analyst format: lines of "value R G B [A]")
    pub fn from_pal_file(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        Self::from_pal_string(&content, path.file_stem()?.to_str()?)
    }

    pub fn from_pal_string(content: &str, name: &str) -> Option<Self> {
        let mut entries = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with("//") {
                continue;
            }
            // Try "Color: VALUE R G B [A]" format or plain "VALUE R G B [A]"
            let line = line.strip_prefix("Color:").unwrap_or(line).trim();
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                if let (Ok(v), Ok(r), Ok(g), Ok(b)) = (
                    parts[0].parse::<f32>(),
                    parts[1].parse::<u8>(),
                    parts[2].parse::<u8>(),
                    parts[3].parse::<u8>(),
                ) {
                    let a = parts.get(4).and_then(|s| s.parse::<u8>().ok()).unwrap_or(255);
                    entries.push(ColorEntry { value: v, r, g, b, a });
                }
            }
        }
        if entries.len() < 2 {
            return None;
        }
        entries.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
        let min = entries.first().unwrap().value;
        let max = entries.last().unwrap().value;
        Some(ColorTable {
            name: name.to_string(),
            min_value: min,
            max_value: max,
            entries,
        })
    }

    /// Get a color table for a product with a given preset style
    pub fn for_product_preset(product: RadarProduct, preset: ColorTablePreset) -> Self {
        match preset {
            ColorTablePreset::Default => Self::for_product(product),
            ColorTablePreset::Dark => Self::dark_table(product),
            ColorTablePreset::Colorblind => Self::colorblind_table(product),
        }
    }

    fn dark_table(product: RadarProduct) -> Self {
        // Dark theme: same structure but darker/more saturated colors
        let mut table = Self::for_product(product);
        table.name = format!("{} (Dark)", table.name);
        // Boost saturation and darken backgrounds
        for entry in &mut table.entries {
            if entry.a < 200 {
                entry.a = (entry.a as f32 * 0.7) as u8;
            }
            // Slightly increase color intensity
            entry.r = (entry.r as f32 * 1.1).min(255.0) as u8;
            entry.g = (entry.g as f32 * 1.1).min(255.0) as u8;
            entry.b = (entry.b as f32 * 1.1).min(255.0) as u8;
        }
        table
    }

    fn colorblind_table(product: RadarProduct) -> Self {
        match product {
            RadarProduct::Reflectivity => ColorTable {
                name: "Reflectivity (Colorblind)".into(),
                min_value: -30.0,
                max_value: 80.0,
                entries: vec![
                    ColorEntry { value: -30.0, r: 0, g: 0, b: 0, a: 0 },
                    ColorEntry { value: 0.0, r: 230, g: 230, b: 230, a: 200 },
                    ColorEntry { value: 10.0, r: 171, g: 217, b: 233, a: 255 },
                    ColorEntry { value: 20.0, r: 44, g: 123, b: 182, a: 255 },
                    ColorEntry { value: 30.0, r: 255, g: 255, b: 191, a: 255 },
                    ColorEntry { value: 40.0, r: 253, g: 174, b: 97, a: 255 },
                    ColorEntry { value: 50.0, r: 215, g: 25, b: 28, a: 255 },
                    ColorEntry { value: 60.0, r: 128, g: 0, b: 38, a: 255 },
                    ColorEntry { value: 70.0, r: 77, g: 0, b: 75, a: 255 },
                    ColorEntry { value: 80.0, r: 255, g: 255, b: 255, a: 255 },
                ],
            },
            _ => {
                let mut t = Self::for_product(product);
                t.name = format!("{} (Colorblind)", t.name);
                t
            }
        }
    }
}
