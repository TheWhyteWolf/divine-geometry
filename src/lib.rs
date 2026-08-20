//! divine — sacred geometry drawn by compass and straightedge.
//!
//! Library crate shared by the native binary (`main.rs`) and the wasm build
//! (`web.rs`). Everything except the offscreen-still tooling compiles for both.

pub mod anim;
pub mod app;
pub mod color;
pub mod figures;
pub mod fx;
pub mod geom;
pub mod hud;
pub mod infinite;
pub mod params;
pub mod render;
pub mod scene;
pub mod snow;
pub mod tess;

/// Offscreen PNG stills — the native verification path. Needs a filesystem and
/// a blocking device poll, so it has no wasm counterpart.
#[cfg(not(target_arch = "wasm32"))]
pub mod shot;

#[cfg(target_arch = "wasm32")]
pub mod web;
