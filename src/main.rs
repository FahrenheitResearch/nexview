use nexview::app;
use eframe::egui;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("NexView - Weather Radar Viewer"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "NexView",
        native_options,
        Box::new(|cc| Ok(Box::new(app::RadarApp::new(cc)))),
    )
}
