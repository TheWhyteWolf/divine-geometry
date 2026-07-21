pub mod gpu;
pub mod pipeline;

/// Texture readback to PNG. Native-only: it needs a filesystem and a blocking
/// device poll, neither of which exists in the browser.
#[cfg(not(target_arch = "wasm32"))]
pub mod readback;
