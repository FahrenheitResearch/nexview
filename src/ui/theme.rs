use egui::{self, Color32, Visuals};

/// NexView color theme for consistent styling across the app.
pub struct NexViewTheme {
    pub bg_primary: Color32,
    pub bg_secondary: Color32,
    pub bg_panel: Color32,
    pub accent: Color32,
    pub accent_warm: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_dim: Color32,
    pub border: Color32,
    pub success: Color32,
    pub warning_color: Color32,
    pub danger: Color32,
    pub product_active: Color32,
}

impl NexViewTheme {
    pub fn dark() -> Self {
        Self {
            bg_primary: Color32::from_rgb(0x14, 0x14, 0x20),
            bg_secondary: Color32::from_rgb(0x1E, 0x1E, 0x2E),
            bg_panel: Color32::from_rgb(0x25, 0x25, 0x35),
            accent: Color32::from_rgb(0x00, 0xE5, 0xFF),
            accent_warm: Color32::from_rgb(0xFF, 0x6B, 0x35),
            text_primary: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            text_secondary: Color32::from_rgb(0x80, 0x80, 0x90),
            text_dim: Color32::from_rgb(0x50, 0x50, 0x60),
            border: Color32::from_rgb(0x35, 0x35, 0x45),
            success: Color32::from_rgb(0x00, 0xFF, 0x88),
            warning_color: Color32::from_rgb(0xFF, 0xB8, 0x00),
            danger: Color32::from_rgb(0xFF, 0x44, 0x44),
            product_active: Color32::from_rgb(0x00, 0xFF, 0xFF),
        }
    }

    pub fn light() -> Self {
        Self {
            bg_primary: Color32::from_rgb(0xF0, 0xF0, 0xF4),
            bg_secondary: Color32::from_rgb(0xE8, 0xE8, 0xF0),
            bg_panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            accent: Color32::from_rgb(0x00, 0x8C, 0xB4),
            accent_warm: Color32::from_rgb(0xD4, 0x50, 0x20),
            text_primary: Color32::from_rgb(0x1A, 0x1A, 0x2E),
            text_secondary: Color32::from_rgb(0x60, 0x60, 0x70),
            text_dim: Color32::from_rgb(0x90, 0x90, 0xA0),
            border: Color32::from_rgb(0xC8, 0xC8, 0xD4),
            success: Color32::from_rgb(0x00, 0xA0, 0x50),
            warning_color: Color32::from_rgb(0xCC, 0x8C, 0x00),
            danger: Color32::from_rgb(0xCC, 0x22, 0x22),
            product_active: Color32::from_rgb(0x00, 0xA0, 0xD0),
        }
    }

    /// Apply this theme to egui's visuals and style.
    pub fn apply_to_egui(&self, ctx: &egui::Context) {
        let mut visuals = if self.bg_primary.r() < 0x80 {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        // Panel / window backgrounds
        visuals.panel_fill = self.bg_secondary;
        visuals.window_fill = self.bg_panel;
        visuals.extreme_bg_color = self.bg_primary;
        visuals.faint_bg_color = self.bg_panel;

        // Selection
        visuals.selection.bg_fill = self.accent.linear_multiply(0.3);
        visuals.selection.stroke = egui::Stroke::new(1.0, self.accent);

        // Hyperlink
        visuals.hyperlink_color = self.accent;

        // Text colors
        visuals.override_text_color = Some(self.text_primary);

        // Widget visuals — noninteractive
        visuals.widgets.noninteractive.bg_fill = self.bg_panel;
        visuals.widgets.noninteractive.weak_bg_fill = self.bg_panel;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.border);
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, self.text_secondary);

        // Widget visuals — inactive (hoverable but not hovered)
        visuals.widgets.inactive.bg_fill = self.bg_panel;
        visuals.widgets.inactive.weak_bg_fill = self.bg_panel;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.border);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, self.text_primary);

        // Widget visuals — hovered
        visuals.widgets.hovered.bg_fill = self.accent.linear_multiply(0.15);
        visuals.widgets.hovered.weak_bg_fill = self.accent.linear_multiply(0.10);
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, self.accent);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, self.text_primary);

        // Widget visuals — active (being clicked)
        visuals.widgets.active.bg_fill = self.accent.linear_multiply(0.25);
        visuals.widgets.active.weak_bg_fill = self.accent.linear_multiply(0.20);
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, self.accent);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0, self.text_primary);

        // Widget visuals — open (e.g., open combo box)
        visuals.widgets.open.bg_fill = self.bg_panel;
        visuals.widgets.open.weak_bg_fill = self.bg_panel;
        visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, self.accent);
        visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, self.text_primary);

        // Window shadow
        visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(60),
        };

        // Separators
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.border);

        ctx.set_visuals(visuals);

        // Tweak spacing
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        ctx.set_style(style);
    }
}
