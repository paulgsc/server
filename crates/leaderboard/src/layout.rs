use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

/// The layout mode for the visualization
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutMode {
	Pyramid,
	Wall,
}

/// Position of a brick in normalized coordinates [0,1]²
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NormalizedPosition {
	pub x: f64,
	pub y: f64,
	pub width: f64,
	pub height: f64,
}

/// A complete layout configuration.  This is now public for use in the WASM API.
#[wasm_bindgen]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
	/// Spacing ratio between bricks (0 = touching, 1 = large gaps)
	pub spacing_ratio: f64,
	/// Layout mode (pyramid or wall)
	pub mode: LayoutMode,
	/// Whether to use min heap (false) or max heap (true)
	pub max_sort: bool,
}

impl LayoutConfig {
	/// Creates a new LayoutConfig with default values.  Useful for testing.
	pub fn new(spacing_ratio: f64, mode: LayoutMode, max_sort: bool) -> Self {
		LayoutConfig { spacing_ratio, mode, max_sort }
	}
}

/// Core element structure containing key and weight.  Made generic but kept private.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Element<K> {
	key: K,
	weight: f64,
}

/// Internal representation of a brick with its computed position.  Kept private.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrickPosition<K> {
	key: K,
	weight: f64,
	layer: usize,
	index_in_layer: usize,
	position: NormalizedPosition,
}

/// Internal layer info for calculations.  Kept private.
#[derive(Debug, Clone)]
struct LayerInfo {
	width: usize,               // Number of elements in this layer
	elements_per_weight: usize, // Number of elements with this weight
	start_x: f64,               // Starting x position for this layer
}

/// Layout engine that processes elements into positions.  Made generic and kept private.
struct LayoutEngine<K: Clone + Eq + std::hash::Hash> {
	config: LayoutConfig,
	elements: Vec<Element<K>>,
	layers: Vec<LayerInfo>,
	positions: HashMap<K, NormalizedPosition>,
	max_width: usize,
	layer_count: usize,
	unit_x: f64,
	unit_y: f64,
}

impl<K: Clone + Eq + std::hash::Hash> LayoutEngine<K> {
	/// Create a new layout engine with the specified configuration
	pub fn new(config: LayoutConfig) -> Self {
		Self {
			config,
			elements: Vec::new(),
			layers: Vec::new(),
			positions: HashMap::new(),
			max_width: 0,
			layer_count: 0,
			unit_x: 0.0,
			unit_y: 0.0,
		}
	}

	/// Add an element to the layout
	pub fn add_element(&mut self, key: K, weight: f64) {
		self.elements.push(Element { key, weight });
	}

	/// Clear all elements from the layout
	pub fn clear(&mut self) {
		self.elements.clear();
		self.layers.clear();
		self.positions.clear();
		self.max_width = 0;
		self.layer_count = 0;
	}

	/// Sort elements by weight and compute their layers
	fn stratify_by_weight(&mut self) -> Vec<Vec<Element<K>>> {
		// Create a mapping from unique weights to elements with that weight
		let mut weight_map: HashMap<f64, Vec<Element<K>>> = HashMap::new();

		for element in &self.elements {
			weight_map.entry(element.weight).or_insert_with(Vec::new).push(element.clone());
		}

		// Get unique weights and sort them
		let mut unique_weights: Vec<f64> = weight_map.keys().cloned().collect();

		if self.config.max_sort {
			unique_weights.sort_by(|a, b| b.partial_cmp(a).unwrap()); // Max heap (descending)
		} else {
			unique_weights.sort_by(|a, b| a.partial_cmp(b).unwrap()); // Min heap (ascending)
		}

		// Group elements by their weight layers
		let mut layers: Vec<Vec<Element<K>>> = Vec::new();
		for weight in unique_weights {
			if let Some(elements) = weight_map.get(&weight) {
				layers.push(elements.clone());
			}
		}

		// Check and enforce layering constraint: each layer must have at least as many elements as layer above
		let mut max_elements = 0;
		for i in (0..layers.len()).rev() {
			max_elements = std::cmp::max(max_elements, layers[i].len());
			if i > 0 && layers[i - 1].len() < max_elements {
				// Need to pad the layer below with duplicates of the last element
				let pad_amount = max_elements - layers[i - 1].len();
				let last_element = layers[i - 1].last().unwrap().clone();
				for _ in 0..pad_amount {
					layers[i - 1].push(last_element.clone());
				}
			}
		}

		layers
	}

	/// Compute layout positions for all elements
	pub fn compute_layout(&mut self) {
		let stratified_layers = self.stratify_by_weight();

		// Determine max width and layer count
		self.max_width = stratified_layers.iter().map(|layer| layer.len()).max().unwrap_or(0);
		self.layer_count = stratified_layers.len();

		// Compute unit sizes
		let r = self.config.spacing_ratio;
		self.unit_x = 1.0 / (self.max_width as f64 + r * (self.max_width as f64 - 1.0));
		self.unit_y = 1.0 / (self.layer_count as f64 + r * (self.layer_count as f64 - 1.0));

		let spacing_x = r * self.unit_x;
		let spacing_y = r * self.unit_y;

		self.layers = Vec::with_capacity(self.layer_count);
		self.positions.clear();

		// Compute start position for each layer based on layout mode
		for (layer_idx, layer_elements) in stratified_layers.iter().enumerate() {
			let layer_width = layer_elements.len();

			let start_x = match self.config.mode {
				LayoutMode::Pyramid => {
					// Center the layer within max width
					(self.max_width as f64 - layer_width as f64) / 2.0 * (self.unit_x + spacing_x)
				}
				LayoutMode::Wall => {
					// Right align the layer
					(self.max_width as f64 - layer_width as f64) * (self.unit_x + spacing_x)
				}
			};

			self.layers.push(LayerInfo {
				width: layer_width,
				elements_per_weight: layer_elements.iter().filter(|e| e.weight == layer_elements[0].weight).count(),
				start_x,
			});

			// Calculate positions for each element in the layer
			for (idx_in_layer, element) in layer_elements.iter().enumerate() {
				let x = start_x + idx_in_layer as f64 * (self.unit_x + spacing_x);
				let y = layer_idx as f64 * (self.unit_y + spacing_y);

				self.positions.insert(
					element.key.clone(),
					NormalizedPosition {
						x,
						y,
						width: self.unit_x,
						height: self.unit_y,
					},
				);
			}
		}
	}

	/// Get the computed position for an element by key
	pub fn get_position(&self, key: &K) -> Option<NormalizedPosition> {
		self.positions.get(key).cloned()
	}

	/// Get layout dimensions
	pub fn get_dimensions(&self) -> (usize, usize) {
		(self.max_width, self.layer_count)
	}

	/// Scale a normalized position to actual coordinates
	pub fn scale_position(&self, pos: &NormalizedPosition, width: f64, height: f64) -> NormalizedPosition {
		NormalizedPosition {
			x: pos.x * width,
			y: pos.y * height,
			width: pos.width * width,
			height: pos.height * height,
		}
	}

	pub fn get_unit_sizes(&self) -> (f64, f64) {
		(self.unit_x, self.unit_y)
	}
}

// WASM bindings for string keys
#[wasm_bindgen]
pub struct StringLayoutEngine {
	inner: LayoutEngine<String>,
}

#[wasm_bindgen]
impl StringLayoutEngine {
	#[wasm_bindgen(constructor)]
	pub fn new(config: LayoutConfig) -> Self {
		Self { inner: LayoutEngine::new(config) }
	}

	#[wasm_bindgen]
	pub fn add_element(&mut self, key: String, weight: f64) {
		self.inner.add_element(key, weight);
	}

	#[wasm_bindgen]
	pub fn clear(&mut self) {
		self.inner.clear();
	}

	#[wasm_bindgen]
	pub fn compute_layout(&mut self) {
		self.inner.compute_layout();
	}

	#[wasm_bindgen]
	pub fn get_position(&self, key: String) -> Option<NormalizedPosition> {
		self.inner.get_position(&key)
	}

	#[wasm_bindgen]
	pub fn get_dimensions(&self) -> JsValue {
		let (width, height) = self.inner.get_dimensions();
		JsValue::from_serde(&(width, height)).unwrap()
	}

	#[wasm_bindgen]
	pub fn scale_position(&self, position: &NormalizedPosition, width: f64, height: f64) -> NormalizedPosition {
		self.inner.scale_position(position, width, height)
	}

	#[wasm_bindgen]
	pub fn get_unit_sizes(&self) -> JsValue {
		let (unit_x, unit_y) = self.inner.get_unit_sizes();
		JsValue::from_serde(&(unit_x, unit_y)).unwrap()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_layout_engine_pyramid() {
		let config = LayoutConfig::new(0.1, LayoutMode::Pyramid, false);
		let mut engine = LayoutEngine::new(config);

		engine.add_element("a".to_string(), 1.0);
		engine.add_element("b".to_string(), 2.0);
		engine.add_element("c".to_string(), 1.0);
		engine.add_element("d".to_string(), 3.0);
		engine.add_element("e".to_string(), 2.0);
		engine.add_element("f".to_string(), 1.0);

		engine.compute_layout();

		let pos_a = engine.get_position(&"a".to_string()).unwrap();
		let pos_b = engine.get_position(&"b".to_string()).unwrap();
		let pos_c = engine.get_position(&"c".to_string()).unwrap();
		let pos_d = engine.get_position(&"d".to_string()).unwrap();
		let pos_e = engine.get_position(&"e".to_string()).unwrap();
		let pos_f = engine.get_position(&"f".to_string()).unwrap();

		assert_eq!(engine.max_width, 3);
		assert_eq!(engine.layer_count, 3);
		assert_eq!(pos_a.x, 0.0);
		assert_eq!(pos_a.y, 0.0);
		assert_eq!(pos_b.x, 0.3666666666666667);
		assert_eq!(pos_b.y, 0.3666666666666667);
		assert_eq!(pos_c.x, 0.7333333333333333);
		assert_eq!(pos_c.y, 0.0);
		assert_eq!(pos_d.x, 0.3666666666666667);
		assert_eq!(pos_d.y, 0.7333333333333333);
		assert_eq!(pos_e.x, 0.7333333333333333);
		assert_eq!(pos_e.y, 0.3666666666666667);
		assert_eq!(pos_f.x, 0.0);
		assert_eq!(pos_f.y, 0.0);
	}

	#[test]
	fn test_layout_engine_wall() {
		let config = LayoutConfig::new(0.2, LayoutMode::Wall, true);
		let mut engine = LayoutEngine::new(config);
		engine.add_element("a".to_string(), 5.0);
		engine.add_element("b".to_string(), 1.0);
		engine.add_element("c".to_string(), 4.0);
		engine.add_element("d".to_string(), 2.0);
		engine.add_element("e".to_string(), 3.0);
		engine.add_element("f".to_string(), 1.0);

		engine.compute_layout();

		let pos_a = engine.get_position(&"a".to_string()).unwrap();
		let pos_b = engine.get_position(&"b".to_string()).unwrap();
		let pos_c = engine.get_position(&"c".to_string()).unwrap();
		let pos_d = engine.get_position(&"d".to_string()).unwrap();
		let pos_e = engine.get_position(&"e".to_string()).unwrap();
		let pos_f = engine.get_position(&"f".to_string()).unwrap();

		assert_eq!(engine.max_width, 3);
		assert_eq!(engine.layer_count, 3);
		assert_eq!(pos_a.x, 0.68);
		assert_eq!(pos_a.y, 0.0);
		assert_eq!(pos_b.x, 0.0);
		assert_eq!(pos_b.y, 0.68);
		assert_eq!(pos_c.x, 0.36);
		assert_eq!(pos_c.y, 0.36);
		assert_eq!(pos_d.x, 0.36);
		assert_eq!(pos_d.y, 0.68);
		assert_eq!(pos_e.x, 0.0);
		assert_eq!(pos_e.y, 0.36);
		assert_eq!(pos_f.x, 0.0);
		assert_eq!(pos_f.y, 0.68);
	}

	#[test]
	fn test_layout_engine_mixed_weights() {
		let config = LayoutConfig::new(0.0, LayoutMode::Pyramid, false);
		let mut engine = LayoutEngine::new(config);
		engine.add_element("a".to_string(), 1.0);
		engine.add_element("b".to_string(), 2.0);
		engine.add_element("c".to_string(), 1.0);
		engine.add_element("d".to_string(), 2.0);
		engine.add_element("e".to_string(), 1.0);

		engine.compute_layout();

		let pos_a = engine.get_position(&"a".to_string()).unwrap();
		let pos_b = engine.get_position(&"b".to_string()).unwrap();
		let pos_c = engine.get_position(&"c".to_string()).unwrap();
		let pos_d = engine.get_position(&"d".to_string()).unwrap();
		let pos_e = engine.get_position(&"e".to_string()).unwrap();

		assert_eq!(pos_a.x, 0.0);
		assert_eq!(pos_a.y, 0.0);
		assert_eq!(pos_b.x, 0.3333333333333333);
		assert_eq!(pos_b.y, 0.3333333333333333);
		assert_eq!(pos_c.x, 0.6666666666666666);
		assert_eq!(pos_c.y, 0.0);
		assert_eq!(pos_d.x, 0.3333333333333333);
		assert_eq!(pos_d.y, 0.3333333333333333);
		assert_eq!(pos_e.x, 0.0);
		assert_eq!(pos_e.y, 0.0);
	}

	#[test]
	fn test_scale_position() {
		let config = LayoutConfig::new(0.2, LayoutMode::Wall, true);
		let engine = LayoutEngine::new(config);
		let pos = NormalizedPosition {
			x: 0.2,
			y: 0.5,
			width: 0.3,
			height: 0.1,
		};
		let scaled_pos = engine.scale_position(&pos, 100.0, 200.0);
		assert_eq!(scaled_pos.x, 20.0);
		assert_eq!(scaled_pos.y, 100.0);
		assert_eq!(scaled_pos.width, 30.0);
		assert_eq!(scaled_pos.height, 20.0);
	}

	#[test]
	fn test_get_dimensions() {
		let config = LayoutConfig::new(0.2, LayoutMode::Wall, true);
		let mut engine = LayoutEngine::new(config);
		engine.add_element("a".to_string(), 5.0);
		engine.add_element("b".to_string(), 1.0);
		engine.add_element("c".to_string(), 4.0);
		engine.compute_layout();
		let (width, height) = engine.get_dimensions();
		assert_eq!(width, 3);
		assert_eq!(height, 3);
	}

	#[test]
	fn test_clear() {
		let config = LayoutConfig::new(0.2, LayoutMode::Wall, true);
		let mut engine = LayoutEngine::new(config);
		engine.add_element("a".to_string(), 5.0);
		engine.add_element("b".to_string(), 1.0);
		engine.compute_layout();
		engine.clear();
		assert_eq!(engine.elements.len(), 0);
		assert_eq!(engine.layers.len(), 0);
		assert_eq!(engine.positions.len(), 0);
		assert_eq!(engine.max_width, 0);
		assert_eq!(engine.layer_count, 0);
	}
}
