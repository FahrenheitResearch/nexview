use std::collections::HashMap;

use egui::{self, Color32, CornerRadius, Order, TextureHandle, Vec2};

use crate::nexrad::sites::{RadarSite, RADAR_SITES};
use crate::render::map::MapView;

pub struct HoverPreview {
    pub hovered_site: Option<String>,
    pub preview_textures: HashMap<String, TextureHandle>,
    last_hover_pos: Option<egui::Pos2>,
}

impl HoverPreview {
    pub fn new() -> Self {
        Self {
            hovered_site: None,
            preview_textures: HashMap::new(),
            last_hover_pos: None,
        }
    }

    /// Check which site (if any) the cursor is hovering over.
    /// Returns the site if within 12px of a site marker.
    pub fn detect_hover(
        &mut self,
        cursor_pos: Option<egui::Pos2>,
        map_view: &MapView,
        rect: egui::Rect,
    ) -> Option<&'static RadarSite> {
        let cursor = match cursor_pos {
            Some(p) if rect.contains(p) => p,
            _ => {
                self.clear();
                return None;
            }
        };

        self.last_hover_pos = Some(cursor);

        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;
        let hover_radius = 12.0f32;

        let mut closest: Option<(&'static RadarSite, f32)> = None;

        for site in RADAR_SITES.iter() {
            let (px, py) = map_view.lat_lon_to_pixel(site.lat, site.lon, screen_w, screen_h);
            let site_pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);

            if !rect.contains(site_pos) {
                continue;
            }

            let dist = cursor.distance(site_pos);
            if dist <= hover_radius {
                match closest {
                    Some((_, best_dist)) if dist < best_dist => {
                        closest = Some((site, dist));
                    }
                    None => {
                        closest = Some((site, dist));
                    }
                    _ => {}
                }
            }
        }

        if let Some((site, _)) = closest {
            self.hovered_site = Some(site.id.to_string());
            Some(site)
        } else {
            self.hovered_site = None;
            None
        }
    }

    /// Draw the hover preview popup near the cursor.
    /// `thumbnail_pixels` is an optional 256x256 RGBA buffer from the preload cache.
    /// If None, shows a "Loading..." indicator.
    pub fn draw_preview(
        &mut self,
        ctx: &egui::Context,
        _ui: &mut egui::Ui,
        site: &RadarSite,
        cursor_pos: egui::Pos2,
        thumbnail_pixels: Option<&[u8]>,
        timestamp: Option<&str>,
    ) {
        let popup_w = 272.0f32;
        let popup_h = if thumbnail_pixels.is_some() { 310.0 } else { 70.0 };

        // Position: 20px right, 20px above cursor
        let mut popup_pos = egui::pos2(cursor_pos.x + 20.0, cursor_pos.y - 20.0 - popup_h);

        // Clamp to screen rect
        let screen = ctx.screen_rect();
        if popup_pos.x + popup_w > screen.right() {
            popup_pos.x = cursor_pos.x - 20.0 - popup_w;
        }
        if popup_pos.y < screen.top() {
            popup_pos.y = screen.top() + 4.0;
        }
        if popup_pos.y + popup_h > screen.bottom() {
            popup_pos.y = screen.bottom() - popup_h - 4.0;
        }
        if popup_pos.x < screen.left() {
            popup_pos.x = screen.left() + 4.0;
        }

        egui::Area::new(egui::Id::new("hover_preview_popup"))
            .order(Order::Tooltip)
            .fixed_pos(popup_pos)
            .interactable(false)
            .show(ctx, |ui| {
                let bg = Color32::from_rgba_premultiplied(30, 30, 40, 230);
                egui::Frame::new()
                    .fill(bg)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(256.0);

                        // Site ID - bold, 14px
                        ui.label(
                            egui::RichText::new(site.id)
                                .strong()
                                .size(14.0)
                                .color(Color32::WHITE),
                        );

                        // Site name + state - dim, 11px
                        ui.label(
                            egui::RichText::new(format!("{}, {}", site.name, site.state))
                                .size(11.0)
                                .color(Color32::from_rgb(160, 160, 170)),
                        );

                        ui.add_space(4.0);

                        if let Some(pixels) = thumbnail_pixels {
                            // Create or update texture
                            let tex = self
                                .preview_textures
                                .entry(site.id.to_string())
                                .or_insert_with(|| {
                                    let image = egui::ColorImage::from_rgba_unmultiplied(
                                        [256, 256],
                                        pixels,
                                    );
                                    ctx.load_texture(
                                        format!("hover_preview_{}", site.id),
                                        image,
                                        egui::TextureOptions::LINEAR,
                                    )
                                });

                            // Update texture if pixels changed (always update)
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [256, 256],
                                pixels,
                            );
                            tex.set(image, egui::TextureOptions::LINEAR);

                            ui.image(egui::load::SizedTexture::new(
                                tex.id(),
                                Vec2::new(256.0, 256.0),
                            ));

                            if let Some(ts) = timestamp {
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new(ts)
                                        .size(10.0)
                                        .color(Color32::from_rgb(130, 130, 140)),
                                );
                            }
                        } else {
                            // Loading indicator
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new("Loading...")
                                        .size(11.0)
                                        .color(Color32::from_rgb(160, 160, 170)),
                                );
                            });
                        }
                    });
            });
    }

    /// Clear hover state
    pub fn clear(&mut self) {
        self.hovered_site = None;
        self.last_hover_pos = None;
    }
}
