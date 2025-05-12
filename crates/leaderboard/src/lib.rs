use float_utils::{AsFloatWrapper, FloatWrapper};

mod float_utils;

/// The layout style for the leader board chart
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ChartStyle {
	/// Pyramid style - bricks are centered relative to bricks below
	Pyramid,
	/// Wall style - bricks are right-aligned
	BrickWall,
}

/// Sort direction for the leader board
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SortDirection {
	/// Highest weights at the top (max heap)
	MaxOnTop,
	/// Lowest weights at the top (min heap)
	MinOnTop,
}

/// A single element on the leader board
#[derive(Debug, Clone)]
pub struct Element<K> {
	/// Unique identifier
	pub key: K,
	/// Weight value
	pub weight: FloatWrapper,
	/// Position in the chart (layer, index)
	pub position: (usize, usize),
}

/// The leader board chart
#[derive(Debug)]
pub struct LeaderBoard<K> {
	/// Elements in the chart
	elements: Vec<Element<K>>,
	/// Organized elements by layer
	layers: Vec<Vec<usize>>,
	/// Chart display style
	style: ChartStyle,
	/// Sort direction
	sort_direction: SortDirection,
}

impl<K> LeaderBoard<K>
where
	K: Clone + Eq + std::hash::Hash,
{
	/// Create a new leader board with the specified style and sort direction
	pub fn new(style: ChartStyle, sort_direction: SortDirection) -> Self {
		Self {
			elements: Vec::new(),
			layers: Vec::new(),
			style,
			sort_direction,
		}
	}

	pub fn build_leaderboard_data<T, W>(&self, data: Vec<(T, W)>) -> Vec<(T, FloatWrapper)>
	where
		W: AsFloatWrapper,
	{
		data.into_iter().map(|(key, weight)| (key, weight.as_float_wrapper())).collect()
	}

	/// Build the leader board from a collection of (key, weight) tuples
	pub fn build<I>(&mut self, elements: I) -> &mut Self
	where
		I: IntoIterator<Item = (K, FloatWrapper)>,
	{
		// Clear any existing data
		self.elements.clear();
		self.layers.clear();

		// Create elements from input tuples
		self.elements = elements
			.into_iter()
			.map(|(key, weight)| Element {
				key,
				weight,
				position: (0, 0), // Initial position will be updated during organization
			})
			.collect::<Vec<Element<K>>>();

		// Sort elements by weight according to sort direction
		self.elements.sort_by(|a, b| {
			let ordering = a.weight.cmp(&b.weight);
			match self.sort_direction {
				SortDirection::MaxOnTop => ordering.reverse(),
				SortDirection::MinOnTop => ordering,
			}
		});

		// Organize elements into layers
		self.organize_layers();

		// Calculate positions based on style
		self.calculate_positions();

		self
	}

	/// Organize elements into layers
	fn organize_layers(&mut self) {
		if self.elements.is_empty() {
			return;
		}

		// Group elements by weight
		// We'll use the element's index in the sorted array instead of the weight as key
		// to avoid floating-point hash key issues
		let mut weight_groups = Vec::new();

		if !self.elements.is_empty() {
			let mut current_weight = &self.elements[0].weight;
			let mut current_group = Vec::new();

			for (idx, element) in self.elements.iter().enumerate() {
				if &element.weight != current_weight {
					weight_groups.push((current_weight, current_group));
					current_weight = &element.weight;
					current_group = Vec::new();
				}
				current_group.push(idx);
			}

			// Don't forget to add the last group
			if !current_group.is_empty() {
				weight_groups.push((current_weight, current_group));
			}
		}

		// Create layers
		self.layers = Vec::new();
		let mut current_layer = Vec::new();
		let mut prev_weight_size = 0;

		// The weight_groups vector is already sorted by weight due to how we created it
		// from the sorted elements array
		for (_, elements_with_weight) in weight_groups {
			// Create a new layer if needed
			if !current_layer.is_empty() && current_layer.len() < prev_weight_size + elements_with_weight.len() {
				self.layers.push(current_layer);
				current_layer = Vec::new();
			}

			// Add elements to the current layer
			prev_weight_size = elements_with_weight.len();
			current_layer.extend(elements_with_weight);
		}

		// Add the final layer if not empty
		if !current_layer.is_empty() {
			self.layers.push(current_layer);
		}

		// Ensure lower layers have at least as many elements as upper layers
		self.ensure_layer_constraints();
	}

	/// Ensure that each layer has at least as many elements as the layer above it
	fn ensure_layer_constraints(&mut self) {
		if self.layers.len() <= 1 {
			return;
		}

		// Work from top to bottom
		for i in (0..self.layers.len() - 1).rev() {
			let upper_layer_size = self.layers[i].len();
			let lower_layer_size = self.layers[i + 1].len();

			if lower_layer_size < upper_layer_size {
				// Need to move some elements from the upper layer to the lower layer
				let elements_to_move = upper_layer_size - lower_layer_size;
				let moved_elements: Vec<usize> = self.layers[i].drain(upper_layer_size - elements_to_move..upper_layer_size).collect();

				self.layers[i + 1].extend(moved_elements);
			}
		}
	}

	fn calculate_positions(&mut self) {
		let mut position_updates = Vec::new();

		// First pass: collect all position updates
		for (layer_idx, layer) in self.layers.iter().enumerate() {
			for (idx, &element_idx) in layer.iter().enumerate() {
				let position = match self.style {
					ChartStyle::Pyramid => {
						// Calculate pyramid position logic
						let layer_size = layer.len();
						let layer_above_size = if layer_idx > 0 { self.layers[layer_idx - 1].len() } else { 0 };

						let position_index = if layer_above_size > 0 {
							let offset = (layer_size - layer_above_size) / 2;
							idx + offset
						} else {
							idx
						};
						(layer_idx, position_index)
					}
					ChartStyle::BrickWall => (layer_idx, idx),
				};
				position_updates.push((element_idx, position));
			}
		}

		// Second pass: apply all position updates
		for (element_idx, position) in position_updates {
			self.elements[element_idx].position = position;
		}
	}

	/// Get all elements in the leader board
	pub fn get_elements(&self) -> &[Element<K>] {
		&self.elements
	}

	/// Get the organized layers
	pub fn get_layers(&self) -> &[Vec<usize>] {
		&self.layers
	}

	/// Get elements in a specific layer
	pub fn get_layer_elements(&self, layer_idx: usize) -> Option<Vec<&Element<K>>> {
		self.layers.get(layer_idx).map(|layer| layer.iter().map(|&idx| &self.elements[idx]).collect())
	}

	/// Get the total number of layers
	pub fn layer_count(&self) -> usize {
		self.layers.len()
	}

	/// Get the maximum width across all layers
	pub fn max_width(&self) -> usize {
		self.layers.iter().map(|layer| layer.len()).max().unwrap_or(0)
	}

	/// Change the chart style
	pub fn set_style(&mut self, style: ChartStyle) -> &mut Self {
		self.style = style;
		if !self.elements.is_empty() {
			self.calculate_positions();
		}
		self
	}

	/// Change the sort direction
	pub fn set_sort_direction(&mut self, direction: SortDirection) -> &mut Self {
		self.sort_direction = direction;
		if !self.elements.is_empty() {
			// Rebuild the chart with the new sort direction
			let elements: Vec<(K, FloatWrapper)> = self.elements.iter().map(|e| (e.key.clone(), e.weight)).collect();
			self.build(elements);
		}
		self
	}
}

// Implementation for display formatting
impl<K> std::fmt::Display for LeaderBoard<K>
where
	K: std::fmt::Display + Clone + Eq + std::hash::Hash,
{
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		let max_width = self.max_width();

		for layer_idx in 0..self.layers.len() {
			let layer_elements = self.get_layer_elements(layer_idx).unwrap();
			let layer_size = layer_elements.len();

			match self.style {
				ChartStyle::Pyramid => {
					// Center the layer
					let padding = (max_width - layer_size) / 2;
					write!(f, "{}", " ".repeat(padding))?;
				}
				ChartStyle::BrickWall => {
					// Right align, so padding is on the left
					let padding = max_width - layer_size;
					write!(f, "{}", " ".repeat(padding))?;
				}
			}

			// Print the layer elements
			for element in layer_elements {
				write!(f, "[{}: {}] ", element.key, element.weight)?;
			}
			writeln!(f)?;
		}

		Ok(())
	}
}

// Test module
#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_empty_leader_board() {
		let mut board: LeaderBoard<&str> = LeaderBoard::new(ChartStyle::Pyramid, SortDirection::MaxOnTop);
		board.build(vec![]);
		assert_eq!(board.layer_count(), 0);
		assert_eq!(board.max_width(), 0);
	}

	#[test]
	fn test_single_element() {
		let mut board = LeaderBoard::new(ChartStyle::Pyramid, SortDirection::MaxOnTop);
		let data = vec![("A", 1.0)];
		let elements = board.build_leaderboard_data(data);
		board.build(elements);
		assert_eq!(board.layer_count(), 1);
		assert_eq!(board.max_width(), 1);

		let elements = board.get_elements();
		assert_eq!(elements.len(), 1);
		assert_eq!(elements[0].position, (0, 0));
	}

	#[test]
	fn test_multiple_layers() {
		let mut board = LeaderBoard::new(ChartStyle::Pyramid, SortDirection::MaxOnTop);
		let data = vec![("A", 3.0), ("B", 2.0), ("C", 2.0), ("D", 1.0), ("E", 1.0), ("F", 1.0)];
		let elements = board.build_leaderboard_data(data);
		board.build(elements);

		assert_eq!(board.layer_count(), 3);

		// Check the top layer has A
		let top_layer = board.get_layer_elements(0).unwrap();
		assert_eq!(top_layer.len(), 1);
		assert_eq!(top_layer[0].key, "A");

		// Check the middle layer has B and C
		let middle_layer = board.get_layer_elements(1).unwrap();
		assert_eq!(middle_layer.len(), 2);
		assert!(middle_layer.iter().any(|e| e.key == "B"));
		assert!(middle_layer.iter().any(|e| e.key == "C"));

		// Check the bottom layer has D, E, and F
		let bottom_layer = board.get_layer_elements(2).unwrap();
		assert_eq!(bottom_layer.len(), 3);
		assert!(bottom_layer.iter().any(|e| e.key == "D"));
		assert!(bottom_layer.iter().any(|e| e.key == "E"));
		assert!(bottom_layer.iter().any(|e| e.key == "F"));
	}

	#[test]
	fn test_layer_constraint() {
		let mut board = LeaderBoard::new(ChartStyle::Pyramid, SortDirection::MaxOnTop);
		let data = vec![("A", 3.0), ("B", 2.0), ("C", 1.0), ("D", 1.0)];
		let elements = board.build_leaderboard_data(data);
		board.build(elements);

		// With the constraints, we should have 2 layers, not 3
		assert_eq!(board.layer_count(), 2);

		// Check the top layer has A
		let top_layer = board.get_layer_elements(0).unwrap();
		assert_eq!(top_layer.len(), 1);
		assert_eq!(top_layer[0].key, "A");

		// Check the bottom layer has the rest
		let bottom_layer = board.get_layer_elements(1).unwrap();
		assert_eq!(bottom_layer.len(), 3);
		assert!(bottom_layer.iter().any(|e| e.key == "B"));
		assert!(bottom_layer.iter().any(|e| e.key == "C"));
		assert!(bottom_layer.iter().any(|e| e.key == "D"));
	}

	#[test]
	fn test_sort_direction() {
		// Test max on top
		let mut board = LeaderBoard::new(ChartStyle::Pyramid, SortDirection::MaxOnTop);
		let data = vec![("A", 1.0), ("B", 2.0), ("C", 3.0)];
		let elements = board.build_leaderboard_data(data);
		board.build(elements);

		let top_layer = board.get_layer_elements(0).unwrap();
		assert_eq!(top_layer[0].key, "C");

		// Test min on top
		board.set_sort_direction(SortDirection::MinOnTop);
		let top_layer = board.get_layer_elements(0).unwrap();
		assert_eq!(top_layer[0].key, "A");
	}

	#[test]
	fn test_chart_styles() {
		let mut board = LeaderBoard::new(ChartStyle::Pyramid, SortDirection::MaxOnTop);
		let data = vec![("A", 3.0), ("B", 2.0), ("C", 2.0), ("D", 1.0), ("E", 1.0), ("F", 1.0)];
		let elements = board.build_leaderboard_data(data);
		board.build(elements);

		// In pyramid style, middle layer elements should be centered
		let middle_elements = board.get_layer_elements(1).unwrap();
		let positions: Vec<(usize, usize)> = middle_elements.iter().map(|e| e.position).collect();
		assert!(positions.contains(&(1, 0)));
		assert!(positions.contains(&(1, 1)));

		// Switch to brick wall style
		board.set_style(ChartStyle::BrickWall);

		// In brick wall style, all elements should be right-aligned
		let middle_elements = board.get_layer_elements(1).unwrap();
		let positions: Vec<(usize, usize)> = middle_elements.iter().map(|e| e.position).collect();
		assert!(positions.contains(&(1, 0)));
		assert!(positions.contains(&(1, 1)));
	}
}
