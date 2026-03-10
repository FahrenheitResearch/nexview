#[cfg(target_arch = "wasm32")]
use eframe::wasm_bindgen::{self, prelude::*};

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start(canvas_id: &str) -> Result<(), wasm_bindgen::JsValue> {
    // Redirect panics to console.error
    console_error_panic_hook::set_once();

    // Redirect tracing/log to console.log
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();
    eframe::WebRunner::new()
        .start(
            canvas_id,
            web_options,
            Box::new(|cc| Ok(Box::new(crate::app::RadarApp::new(cc)))),
        )
        .expect("failed to start eframe");
    Ok(())
}
