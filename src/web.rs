//! Web (wasm32) entry point and browser glue.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

/// Canvas id expected in the host page.
const CANVAS_ID: &str = "divine";

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();

    crate::app::run_web().map_err(|e| JsValue::from_str(&format!("{e:#}")))?;
    Ok(())
}

/// The render canvas from the host page (winit attaches to it).
pub fn get_canvas() -> Option<HtmlCanvasElement> {
    let doc = web_sys::window()?.document()?;
    doc.get_element_by_id(CANVAS_ID)?.dyn_into::<HtmlCanvasElement>().ok()
}

/// The canvas's CSS size in physical pixels.
///
/// The canvas is laid out at 100vw/100vh, so its *backing buffer* has to be
/// driven from the CSS box times the device pixel ratio — otherwise the figure
/// renders at a default 300×150 and gets stretched, which on a HiDPI screen
/// looks like someone smeared the whole construction sideways.
pub fn canvas_size() -> Option<(u32, u32)> {
    let win = web_sys::window()?;
    let canvas = get_canvas()?;
    let dpr = win.device_pixel_ratio().max(1.0);
    let w = (canvas.client_width() as f64 * dpr).round() as u32;
    let h = (canvas.client_height() as f64 * dpr).round() as u32;
    (w > 0 && h > 0).then_some((w, h))
}

/// Show the host page's #fatal overlay (e.g. WebGPU unavailable).
pub fn show_fatal(msg: &str) {
    tracing::error!("{msg}");
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    if let Some(el) = doc.get_element_by_id("fatal") {
        el.set_text_content(Some(msg));
        let _ = el.set_attribute("style", "display:flex");
    }
}

/// Remove the loading overlay once the first frame is on screen.
pub fn hide_loading() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    if let Some(el) = doc.get_element_by_id("loading") {
        el.remove();
    }
}
