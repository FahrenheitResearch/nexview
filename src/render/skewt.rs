use crate::data::sounding::{SoundingProfile, SoundingLevel};

/// Renders a Skew-T / Log-P diagram to an RGBA pixel buffer.
pub struct SkewTRenderer;

// ── Diagram constants ───────────────────────────────────────────────

const P_TOP: f64 = 100.0;    // Top pressure (hPa)
const P_BOT: f64 = 1050.0;   // Bottom pressure (hPa)
const T_MIN: f64 = -40.0;    // Minimum temperature (°C) at P_BOT
const T_MAX: f64 = 50.0;     // Maximum temperature (°C) at P_BOT
const SKEW: f64 = 1.0;       // Skew factor (radians-ish, ~45°)

// Physical constants (duplicated here to keep modules independent).
const LV: f64 = 2.501e6;
const RD: f64 = 287.04;
const RV: f64 = 461.5;
const CP: f64 = 1004.0;
const G: f64 = 9.80665;
const GAMMA_D: f64 = G / CP;

// ── Colours ─────────────────────────────────────────────────────────

const COL_BG: [u8; 4] = [10, 10, 20, 255];
const COL_GRID: [u8; 4] = [60, 60, 70, 255];
const COL_GRID_ZERO: [u8; 4] = [90, 90, 100, 255];
const COL_DRY_ADIABAT: [u8; 4] = [100, 70, 40, 180];
const COL_TEMP: [u8; 4] = [255, 40, 40, 255];
const COL_DEWP: [u8; 4] = [40, 220, 40, 255];
const COL_PARCEL: [u8; 4] = [80, 140, 255, 255];
const COL_CAPE_FILL: [u8; 4] = [255, 60, 60, 60];
const COL_CIN_FILL: [u8; 4] = [60, 60, 255, 40];
const COL_WIND_BARB: [u8; 4] = [220, 220, 220, 255];
const COL_TEXT_BG: [u8; 4] = [20, 20, 30, 220];
const COL_TEXT: [u8; 4] = [240, 240, 240, 255];

// ── Coordinate transforms ───────────────────────────────────────────

/// Normalised Y from pressure (0 = bottom, 1 = top).
fn y_from_p(p: f64) -> f64 {
    (P_BOT.ln() - p.ln()) / (P_BOT.ln() - P_TOP.ln())
}

/// Screen (x, y) from (temperature °C, pressure hPa).
fn tp_to_screen(t: f64, p: f64, w: u32, h: u32) -> (f64, f64) {
    let yn = y_from_p(p);
    let margin_left = 50.0;
    let margin_right = 60.0; // room for wind barbs
    let margin_top = 20.0;
    let margin_bot = 30.0;
    let plot_w = w as f64 - margin_left - margin_right;
    let plot_h = h as f64 - margin_top - margin_bot;

    // Skewed x: shift temperature right as we go up.
    let t_shifted = t + SKEW * (P_BOT.ln() - p.ln()) * 25.0;
    let xn = (t_shifted - T_MIN) / (T_MAX - T_MIN);

    let sx = margin_left + xn * plot_w;
    let sy = margin_top + (1.0 - yn) * plot_h;
    (sx, sy)
}

/// X position for wind barbs (right margin).
fn barb_x(w: u32) -> f64 {
    w as f64 - 35.0
}

// ── Thermo helpers ──────────────────────────────────────────────────

fn sat_vapor_pressure(temp_c: f64) -> f64 {
    6.112 * ((17.67 * temp_c) / (temp_c + 243.5)).exp()
}

fn sat_mixing_ratio(temp_c: f64, pres_mb: f64) -> f64 {
    let es = sat_vapor_pressure(temp_c);
    0.622 * es / (pres_mb - es).max(0.1)
}

fn moist_lapse_rate(temp_c: f64, pres_mb: f64) -> f64 {
    let t_k = temp_c + 273.15;
    let ws = sat_mixing_ratio(temp_c, pres_mb);
    let numer = 1.0 + LV * ws / (RD * t_k);
    let denom = 1.0 + LV * LV * ws / (CP * RV * t_k * t_k);
    GAMMA_D * numer / denom
}

/// Pressure at a given height, linearly interpolated from levels.
fn pressure_at_h(h: f64, levels: &[SoundingLevel]) -> f64 {
    for i in 0..levels.len().saturating_sub(1) {
        let h0 = levels[i].height_m as f64;
        let h1 = levels[i + 1].height_m as f64;
        if h >= h0 && h <= h1 {
            let f = (h - h0) / (h1 - h0).max(1.0);
            let p0 = levels[i].pressure_mb as f64;
            let p1 = levels[i + 1].pressure_mb as f64;
            return p0 + f * (p1 - p0);
        }
    }
    if let Some(l) = levels.last() { l.pressure_mb as f64 } else { 500.0 }
}

// ── Drawing primitives ──────────────────────────────────────────────

struct Canvas {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        // Fill background.
        for i in 0..(w * h) as usize {
            pixels[i * 4] = COL_BG[0];
            pixels[i * 4 + 1] = COL_BG[1];
            pixels[i * 4 + 2] = COL_BG[2];
            pixels[i * 4 + 3] = COL_BG[3];
        }
        Self { pixels, w, h }
    }

    fn put_pixel_blend(&mut self, x: i32, y: i32, col: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let idx = (y as u32 * self.w + x as u32) as usize * 4;
        let alpha = col[3] as f32 / 255.0;
        let inv = 1.0 - alpha;
        self.pixels[idx] = (col[0] as f32 * alpha + self.pixels[idx] as f32 * inv) as u8;
        self.pixels[idx + 1] = (col[1] as f32 * alpha + self.pixels[idx + 1] as f32 * inv) as u8;
        self.pixels[idx + 2] = (col[2] as f32 * alpha + self.pixels[idx + 2] as f32 * inv) as u8;
        self.pixels[idx + 3] = 255;
    }

    /// Bresenham line.
    fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 4]) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;
        loop {
            self.put_pixel_blend(cx, cy, col);
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    /// Thick line (draw parallel offsets).
    fn draw_thick_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 4], thickness: i32) {
        for d in -(thickness / 2)..=(thickness / 2) {
            // Offset perpendicular to the line direction.
            let dx = (x1 - x0) as f64;
            let dy = (y1 - y0) as f64;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let nx = (-dy / len * d as f64) as i32;
            let ny = (dx / len * d as f64) as i32;
            self.draw_line(x0 + nx, y0 + ny, x1 + nx, y1 + ny, col);
        }
    }

    /// Dashed line.
    fn draw_dashed_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 4], dash_len: i32) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = ((dx * dx + dy * dy) as f64).sqrt();
        let steps = len as i32;
        let mut on = true;
        let mut count = 0;
        for i in 0..=steps {
            let t = i as f64 / steps.max(1) as f64;
            let x = x0 as f64 + t * dx as f64;
            let y = y0 as f64 + t * dy as f64;
            if on {
                self.put_pixel_blend(x as i32, y as i32, col);
            }
            count += 1;
            if count >= dash_len {
                on = !on;
                count = 0;
            }
        }
    }

    /// Fill a horizontal span between two x positions at row y.
    fn fill_span(&mut self, y: i32, x_left: i32, x_right: i32, col: [u8; 4]) {
        if y < 0 || y >= self.h as i32 {
            return;
        }
        let l = x_left.max(0);
        let r = x_right.min(self.w as i32 - 1);
        for x in l..=r {
            self.put_pixel_blend(x, y, col);
        }
    }

    /// Draw a tiny 5×7 glyph character. Very minimal built-in font.
    fn draw_char(&mut self, ch: char, px: i32, py: i32, col: [u8; 4]) {
        let bitmap = char_bitmap(ch);
        for (row, bits) in bitmap.iter().enumerate() {
            for col_idx in 0..5 {
                if bits & (1 << (4 - col_idx)) != 0 {
                    self.put_pixel_blend(px + col_idx, py + row as i32, col);
                }
            }
        }
    }

    /// Draw a string of characters.
    fn draw_text(&mut self, text: &str, px: i32, py: i32, col: [u8; 4]) {
        for (i, ch) in text.chars().enumerate() {
            self.draw_char(ch, px + i as i32 * 6, py, col);
        }
    }

    /// Draw text with a background box.
    fn draw_text_box(&mut self, text: &str, px: i32, py: i32, fg: [u8; 4], bg: [u8; 4]) {
        let tw = text.len() as i32 * 6 + 4;
        let th = 10;
        for y in py - 1..py + th {
            self.fill_span(y, px - 2, px + tw, bg);
        }
        self.draw_text(text, px, py, fg);
    }
}

/// Minimal 5×7 bitmap font for digits, letters, and a few symbols.
fn char_bitmap(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        ' ' => [0; 7],
        ':' => [0b00000, 0b00100, 0b00000, 0b00000, 0b00100, 0b00000, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        ',' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b01000],
        'a'..='z' => char_bitmap((ch as u8 - b'a' + b'A') as char),
        _ => [0; 7],
    }
}

// ── Main renderer ───────────────────────────────────────────────────

impl SkewTRenderer {
    /// Render a Skew-T/Log-P diagram. Returns an RGBA pixel buffer of size `width * height * 4`.
    pub fn render(profile: &SoundingProfile, width: u32, height: u32) -> Vec<u8> {
        let mut c = Canvas::new(width, height);

        // ── 1. Background grid ──────────────────────────────────────
        Self::draw_isobars(&mut c, width, height);
        Self::draw_isotherms(&mut c, width, height);
        Self::draw_dry_adiabats(&mut c, width, height);

        // ── 2-6. Profiles and shaded areas ──────────────────────────
        let parcel_path = Self::compute_parcel_path(profile);
        Self::draw_cape_cin_fills(&mut c, profile, &parcel_path, width, height);
        Self::draw_profile_line(&mut c, profile, true, COL_DEWP, width, height);
        Self::draw_profile_line(&mut c, profile, false, COL_TEMP, width, height);
        Self::draw_parcel_path(&mut c, &parcel_path, width, height);

        // ── 7. Wind barbs ───────────────────────────────────────────
        Self::draw_wind_barbs(&mut c, profile, width, height);

        // ── 8. Index text box ───────────────────────────────────────
        Self::draw_index_box(&mut c, profile, width, height);

        // ── Pressure labels ─────────────────────────────────────────
        Self::draw_pressure_labels(&mut c, width, height);

        c.pixels
    }

    fn draw_isobars(c: &mut Canvas, w: u32, h: u32) {
        let pressures = [1000.0, 900.0, 800.0, 700.0, 600.0, 500.0, 400.0, 300.0, 200.0];
        for &p in &pressures {
            let (_, y) = tp_to_screen(0.0, p, w, h);
            let yi = y as i32;
            c.draw_line(50, yi, w as i32 - 60, yi, COL_GRID);
        }
    }

    fn draw_pressure_labels(c: &mut Canvas, w: u32, h: u32) {
        let pressures = [1000, 850, 700, 500, 300, 200];
        for &p in &pressures {
            let (_, y) = tp_to_screen(0.0, p as f64, w, h);
            let label = format!("{p}");
            c.draw_text(&label, 4, y as i32 - 3, COL_GRID);
        }
    }

    fn draw_isotherms(c: &mut Canvas, w: u32, h: u32) {
        let t_start = -80i32;
        let t_end = 60i32;
        let step = 10;
        for t in (t_start..=t_end).step_by(step as usize) {
            let col = if t == 0 { COL_GRID_ZERO } else { COL_GRID };
            let (x0, y0) = tp_to_screen(t as f64, P_BOT, w, h);
            let (x1, y1) = tp_to_screen(t as f64, P_TOP, w, h);
            c.draw_line(x0 as i32, y0 as i32, x1 as i32, y1 as i32, col);
        }
    }

    fn draw_dry_adiabats(c: &mut Canvas, w: u32, h: u32) {
        // Draw dry adiabats from various starting temperatures at 1000 hPa.
        for start_t in (-40..=60).step_by(10) {
            let mut t = start_t as f64;
            let mut p = P_BOT;
            let dp = -10.0; // pressure step
            let mut prev: Option<(i32, i32)> = None;
            while p >= P_TOP {
                let (sx, sy) = tp_to_screen(t, p, w, h);
                if let Some((px, py)) = prev {
                    c.draw_line(px, py, sx as i32, sy as i32, COL_DRY_ADIABAT);
                }
                prev = Some((sx as i32, sy as i32));
                // Poisson relation: T decreases as pressure drops.
                // dT = (Rd*T)/(Cp*P) * dP  ... simplified for plotting:
                let t_k = t + 273.15;
                let new_p = p + dp;
                let new_t_k = t_k * (new_p / p).powf(RD / CP);
                t = new_t_k - 273.15;
                p = new_p;
            }
        }
    }

    /// Build the lifted parcel path as a sequence of (temperature, pressure) points.
    fn compute_parcel_path(profile: &SoundingProfile) -> Vec<(f64, f64)> {
        if profile.levels.is_empty() {
            return vec![];
        }
        let sfc = &profile.levels[0];
        let sfc_t = sfc.temp_c as f64;
        let sfc_td = sfc.dewpoint_c as f64;
        let sfc_p = sfc.pressure_mb as f64;

        let mut path: Vec<(f64, f64)> = Vec::new();
        let mut t = sfc_t;
        let mut td = sfc_td;
        let mut p = sfc_p;
        let mut found_lcl = false;
        let dp = -5.0;

        path.push((t, p));

        while p > P_TOP {
            let new_p = (p + dp).max(P_TOP);
            if !found_lcl {
                // Dry adiabatic ascent via Poisson relation.
                let t_k = t + 273.15;
                let new_t_k = t_k * (new_p / p).powf(RD / CP);
                t = new_t_k - 273.15;
                // Dewpoint changes slowly (conserve mixing ratio → approximate).
                td -= 0.0018 * (RD * (t + 273.15) / G) * (p - new_p) / p * 0.5;
                if t <= td {
                    found_lcl = true;
                }
            } else {
                // Moist adiabat: step height and use lapse rate.
                // Approximate height step from hypsometric equation.
                let t_k = t + 273.15;
                let dz = -(RD * t_k / G) * (dp / p);
                let gamma_m = moist_lapse_rate(t, p);
                t -= gamma_m * dz;
            }
            p = new_p;
            path.push((t, p));
        }
        path
    }

    fn draw_parcel_path(c: &mut Canvas, path: &[(f64, f64)], w: u32, h: u32) {
        for i in 1..path.len() {
            let (x0, y0) = tp_to_screen(path[i - 1].0, path[i - 1].1, w, h);
            let (x1, y1) = tp_to_screen(path[i].0, path[i].1, w, h);
            c.draw_dashed_line(x0 as i32, y0 as i32, x1 as i32, y1 as i32, COL_PARCEL, 6);
        }
    }

    fn draw_profile_line(
        c: &mut Canvas,
        profile: &SoundingProfile,
        dewpoint: bool,
        col: [u8; 4],
        w: u32,
        h: u32,
    ) {
        let levels = &profile.levels;
        for i in 1..levels.len() {
            let t0 = if dewpoint { levels[i - 1].dewpoint_c } else { levels[i - 1].temp_c } as f64;
            let t1 = if dewpoint { levels[i].dewpoint_c } else { levels[i].temp_c } as f64;
            let p0 = levels[i - 1].pressure_mb as f64;
            let p1 = levels[i].pressure_mb as f64;
            let (x0, y0) = tp_to_screen(t0, p0, w, h);
            let (x1, y1) = tp_to_screen(t1, p1, w, h);
            c.draw_thick_line(x0 as i32, y0 as i32, x1 as i32, y1 as i32, col, 2);
        }
    }

    fn draw_cape_cin_fills(
        c: &mut Canvas,
        profile: &SoundingProfile,
        parcel_path: &[(f64, f64)],
        w: u32,
        h: u32,
    ) {
        if parcel_path.is_empty() || profile.levels.is_empty() {
            return;
        }

        // For each parcel path segment, compare parcel T to environment T at that pressure.
        for &(pt, pp) in parcel_path.iter() {
            // Interpolate environment temperature at this pressure.
            let env_t = interp_env_temp_at_p(pp, &profile.levels);
            if env_t.is_none() {
                continue;
            }
            let env_t = env_t.unwrap();

            let (parcel_sx, sy) = tp_to_screen(pt, pp, w, h);
            let (env_sx, _) = tp_to_screen(env_t, pp, w, h);
            let yi = sy as i32;

            if pt > env_t {
                // CAPE region: parcel warmer than environment.
                c.fill_span(yi, env_sx as i32, parcel_sx as i32, COL_CAPE_FILL);
            } else {
                // CIN region: parcel cooler than environment.
                c.fill_span(yi, parcel_sx as i32, env_sx as i32, COL_CIN_FILL);
            }
        }
    }

    fn draw_wind_barbs(c: &mut Canvas, profile: &SoundingProfile, w: u32, h: u32) {
        let bx = barb_x(w) as i32;
        // Draw barbs at a subset of levels to avoid clutter.
        let step = (profile.levels.len() / 30).max(1);
        for (idx, level) in profile.levels.iter().enumerate() {
            if idx % step != 0 {
                continue;
            }
            let p = level.pressure_mb as f64;
            if p < P_TOP || p > P_BOT {
                continue;
            }
            let (_, sy) = tp_to_screen(0.0, p, w, h);
            let yi = sy as i32;
            Self::draw_single_barb(c, bx, yi, level.wind_dir, level.wind_speed_kts);
        }
    }

    /// Draw one wind barb at (cx, cy).
    fn draw_single_barb(c: &mut Canvas, cx: i32, cy: i32, wdir: f32, wspd: f32) {
        let staff_len = 20;
        let dir_rad = (wdir as f64).to_radians();
        // Staff points "into" the wind: from (cx,cy) in the direction the wind comes from.
        let dx = -(dir_rad.sin());
        let dy = dir_rad.cos();
        let ex = cx + (dx * staff_len as f64) as i32;
        let ey = cy + (dy * staff_len as f64) as i32;
        c.draw_line(cx, cy, ex, ey, COL_WIND_BARB);

        // Add barbs from the end of the staff.
        let mut remaining = wspd;
        let mut pos = 0; // distance from end
        let barb_len = 8.0;
        // Perpendicular direction for barbs (to the right of the staff).
        let px = -dy;
        let py = dx;

        // Pennants (50 kt).
        while remaining >= 50.0 {
            let bx0 = ex - (dx * pos as f64) as i32;
            let by0 = ey - (dy * pos as f64) as i32;
            let bx1 = bx0 + (px * barb_len) as i32;
            let by1 = by0 + (py * barb_len) as i32;
            let bx2 = ex - (dx * (pos + 4) as f64) as i32;
            let by2 = ey - (dy * (pos + 4) as f64) as i32;
            // Filled triangle: just draw multiple lines.
            for t in 0..=4 {
                let f = t as f64 / 4.0;
                let mx = bx0 as f64 + f * (bx2 - bx0) as f64;
                let my = by0 as f64 + f * (by2 - by0) as f64;
                let tx = bx1 as f64 + f * (bx2 - bx1) as f64;
                let ty = by1 as f64 + f * (by2 - by1) as f64;
                c.draw_line(mx as i32, my as i32, tx as i32, ty as i32, COL_WIND_BARB);
            }
            remaining -= 50.0;
            pos += 5;
        }

        // Full barbs (10 kt).
        while remaining >= 10.0 {
            let bx0 = ex - (dx * pos as f64) as i32;
            let by0 = ey - (dy * pos as f64) as i32;
            let bx1 = bx0 + (px * barb_len) as i32;
            let by1 = by0 + (py * barb_len) as i32;
            c.draw_line(bx0, by0, bx1, by1, COL_WIND_BARB);
            remaining -= 10.0;
            pos += 3;
        }

        // Half barb (5 kt).
        if remaining >= 5.0 {
            let bx0 = ex - (dx * pos as f64) as i32;
            let by0 = ey - (dy * pos as f64) as i32;
            let bx1 = bx0 + (px * barb_len * 0.5) as i32;
            let by1 = by0 + (py * barb_len * 0.5) as i32;
            c.draw_line(bx0, by0, bx1, by1, COL_WIND_BARB);
        }
    }

    fn draw_index_box(c: &mut Canvas, profile: &SoundingProfile, _w: u32, _h: u32) {
        let x = 55;
        let mut y = 26;
        let lines = [
            format!("CAPE: {:.0} J/KG", profile.cape),
            format!("CIN:  {:.0} J/KG", profile.cin),
            format!("LCL:  {:.0} M AGL", profile.lcl_m),
            format!("0-6 SHEAR: {:.0} KT", profile.bulk_shear_0_6),
            format!("0-1 SRH: {:.0}", profile.srh_0_1),
            format!("0-3 SRH: {:.0}", profile.srh_0_3),
            format!("STP: {:.1}", profile.sig_tornado),
        ];
        for line in &lines {
            c.draw_text_box(line, x, y, COL_TEXT, COL_TEXT_BG);
            y += 12;
        }
    }
}

/// Interpolate environment temperature at a given pressure from sounding levels.
fn interp_env_temp_at_p(p: f64, levels: &[SoundingLevel]) -> Option<f64> {
    if levels.is_empty() {
        return None;
    }
    // Levels are sorted surface-first (decreasing pressure).
    if p >= levels[0].pressure_mb as f64 {
        return Some(levels[0].temp_c as f64);
    }
    if p <= levels.last().unwrap().pressure_mb as f64 {
        return Some(levels.last().unwrap().temp_c as f64);
    }
    for i in 0..levels.len() - 1 {
        let p0 = levels[i].pressure_mb as f64;
        let p1 = levels[i + 1].pressure_mb as f64;
        if p <= p0 && p >= p1 {
            let f = (p0 - p) / (p0 - p1).max(0.01);
            let t = levels[i].temp_c as f64 + f * (levels[i + 1].temp_c as f64 - levels[i].temp_c as f64);
            return Some(t);
        }
    }
    None
}
