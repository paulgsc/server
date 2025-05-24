//! WASM bindings for the layout library

// lib.rs
mod layout;

use wasm_bindgen::prelude::*;

// Re-export types needed for the public API
pub use layout::{LayoutMode, NormalizedPosition, StringLayoutEngine};

// Setup for wasm-pack
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
	// Enable more detailed error messages in WASM
	console_error_panic_hook::set_once();
	Ok(())
}

// Entry point and utility functions for JS
#[wasm_bindgen]
pub fn create_layout_engine(spacing_ratio: f64, is_pyramid: bool, max_sort: bool) -> StringLayoutEngine {
	StringLayoutEngine::new(spacing_ratio, is_pyramid, max_sort)
}
