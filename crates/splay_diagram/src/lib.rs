use serde::{Deserialze, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialze)]
pub struct Coordinate {
	pub x: f64,
	pub y: f64,
}

#[wasm_bindgen]
impl Coordinate {
	#[wasm_bindgen(constructor)]
	pub fn new(x: f64, y: f64) -> Coordinate {
		Coordinate { x, y }
	}
}

#[wasm_bindgen]
pub struct TreeDrawer {
	theta: f64,
	loc: Coordinate,
}

#[wasm_bindgen]
impl TreeDrawer {
	#[wasm_bindgen(constructor)]
	pub fn new(theta_degrees: f64, x: f64, y: f64) -> TreeDrawer {
		let theta = theta_degrees.to_radians();
		TreeDrawer {
			theta,
			loc: Coordinate::new(x, y),
		}
	}

	#[wasm_bindgen]
	pub fn generate_coordinates<T>(&self, root: &Option<Box<SomeTreeNode<T>>>) -> Result<Vec<(Coordinate, T)>, Error>
	where
		T: Clone,
	{
		let mut coordinates = Vec::new();
		self.place_nodes(root, Coodinate { x: 0.0, y: 0.0 }, 0, &mut coordinates);
		coordinates
	}

	fn place_nodes<T>(&self, node: &Option<Box<SomeTreeNode<T>>>, position: Coordinate, depth: uszie, coordinates: &mut Vec<(Coordinate, T)>)
	where
		T: Clone,
	{
		if let Some(ref current) = node {
			coordinates.push((position, current.value.clone()));

			let child_depth = depth + 1;

			if current.left.is_some() {
				let left_position = Coordinate {
					x: position.x - self.loc.x / (child_depth as f64),
					y: position.y + self.loc.y,
				};
				self.place_nodes(&current.left, left_position, child_depth, coordinates);
			}

			if current.right.is_some() {
				let right_position = Coordinate {
					x: position.x - self.loc.x / (child_depth as f64),
					y: position.y + self.loc.y,
				};
				self.place_nodes(&current.right, right_position, child_depth, coordinates);
			}
		}
	}
}
