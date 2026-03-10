use egui::{self, Color32, RichText};
use crate::app::RadarApp;

pub struct TimelineBar;

impl TimelineBar {
    pub fn show(app: &mut RadarApp, ctx: &egui::Context) {
        let bg_panel = Color32::from_rgb(0x25, 0x25, 0x35);
        let accent = Color32::from_rgb(0x00, 0xE5, 0xFF);
        let text_primary = Color32::from_rgb(0xE0, 0xE0, 0xE0);
        let text_secondary = Color32::from_rgb(0x80, 0x80, 0x90);
        let border = Color32::from_rgb(0x35, 0x35, 0x45);

        egui::TopBottomPanel::bottom("timeline_bottom")
            .exact_height(40.0)
            .frame(egui::Frame::new()
                .fill(bg_panel)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .stroke(egui::Stroke::new(1.0, border)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;

                    if app.anim_frames.is_empty() {
                        // No animation loaded — show load button
                        Self::show_empty_state(app, ui, accent, text_secondary);
                    } else {
                        // Animation controls
                        Self::show_controls(app, ui, accent, text_primary, text_secondary);
                    }
                });
            });
    }

    fn show_empty_state(
        app: &mut RadarApp,
        ui: &mut egui::Ui,
        accent: Color32,
        text_secondary: Color32,
    ) {
        if app.anim_loading || (app.multi_radar_anim && !app.multi_anim_ready) {
            ui.spinner();
            if app.multi_radar_anim {
                // Show per-radar progress
                let total_received: usize = app.multi_anim_progress.iter().map(|(_, r, _)| *r).sum();
                let total_expected: usize = app.multi_anim_progress.iter().map(|(_, _, t)| *t).sum();
                ui.label(
                    RichText::new(format!(
                        "Loading {} radars: {}/{}...",
                        app.multi_anim_progress.len(),
                        total_received,
                        total_expected,
                    ))
                    .color(text_secondary)
                    .size(12.0),
                );
            } else {
                ui.label(
                    RichText::new(format!(
                        "Loading {}/{}...",
                        app.anim_received_count,
                        app.anim_download_queue.len()
                    ))
                    .color(text_secondary)
                    .size(12.0),
                );
            }
        } else {
            ui.label(
                RichText::new("No animation loaded")
                    .color(text_secondary)
                    .size(12.0),
            );

            let load_btn = egui::Button::new(
                RichText::new("Load Loop").color(accent).size(12.0),
            );
            if ui.add(load_btn).clicked() {
                app.load_animation_frames();
            }
        }
    }

    fn show_controls(
        app: &mut RadarApp,
        ui: &mut egui::Ui,
        accent: Color32,
        text_primary: Color32,
        text_secondary: Color32,
    ) {
        // 1. Play/Pause button
        let play_icon = if app.anim_playing { "\u{23F8}" } else { "\u{25B6}" };
        let play_btn = egui::Button::new(
            RichText::new(play_icon).size(16.0).color(accent),
        )
        .min_size(egui::vec2(28.0, 28.0));

        if ui.add(play_btn).clicked() {
            app.anim_playing = !app.anim_playing;
            if app.anim_playing {
                app.anim_last_advance = Some(std::time::Instant::now());
            }
        }

        // 2. Speed control
        ui.label(RichText::new("ms:").color(text_secondary).size(11.0));
        ui.add(
            egui::DragValue::new(&mut app.anim_speed_ms)
                .range(50..=1000)
                .speed(5),
        );

        // 3. Frame scrub slider — fill remaining width
        let mut frame_idx = app.anim_index as i32;
        let max_idx = (app.anim_frames.len() as i32 - 1).max(0);

        let slider = egui::Slider::new(&mut frame_idx, 0..=max_idx)
            .show_value(false);

        let response = ui.add_sized(
            egui::vec2(ui.available_width() - 200.0, 20.0),
            slider,
        );

        if response.changed() {
            app.anim_playing = false;
            app.anim_index = frame_idx as usize;
            app.current_file = Some(app.anim_frames[app.anim_index].clone());
            app.needs_render = true;
            // Sync secondary radars when scrubbing
            if app.multi_radar_anim && app.multi_anim_ready {
                app.sync_secondary_to_frame(app.anim_index);
            }
        }

        // 4. Frame counter
        ui.label(
            RichText::new(format!(
                "{}/{}",
                app.anim_index + 1,
                app.anim_frames.len()
            ))
            .color(text_primary)
            .size(12.0)
            .monospace(),
        );

        // 5. Timestamp of current frame
        let timestamp = app
            .anim_frame_names
            .get(app.anim_index)
            .map(|s| s.as_str())
            .unwrap_or("--:--");

        ui.label(
            RichText::new(timestamp)
                .color(text_secondary)
                .size(11.0)
                .monospace(),
        );

        // 6. Multi-radar sync indicator
        if app.multi_radar_anim && app.multi_anim_ready {
            ui.label(
                RichText::new(format!("{} radars", 1 + app.secondary_radars.len()))
                    .color(accent)
                    .size(11.0),
            );
        }
    }
}
