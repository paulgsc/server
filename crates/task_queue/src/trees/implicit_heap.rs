use std::fmt::{self, Debug, Display, Formatter};

/// A node in the heap storing a key-value pair
#[derive(Debug)]
struct HeapNode<K, V> {
	key: K,
	value: V,
}

impl<K, V> HeapNode<K, V> {
	fn new(key: K, value: V) -> Self {
		Self { key, value }
	}
}

/// An implicit heap implementation with generic key and value types
pub struct ImplicitHeap<K: Ord + Debug, V> {
	nodes: Vec<HeapNode<K, V>>,
}

impl<K: Ord + Debug, V> ImplicitHeap<K, V> {
	/// Creates a new empty heap
	pub fn new() -> Self {
		Self { nodes: Vec::new() }
	}

	/// Returns true if the heap is empty
	pub fn is_empty(&self) -> bool {
		self.nodes.is_empty()
	}

	/// Returns the current size of the heap
	pub fn len(&self) -> usize {
		self.nodes.len()
	}

	/// Inserts a new key-value pair
	pub fn insert(&mut self, key: K, value: V) -> Option<V> {
		let index = self.find_key_index(&key);
		if let Some(idx) = index {
			// Key exists, update value
			let old_value = std::mem::replace(&mut self.nodes[idx].value, value);
			self.sift_up(idx);
			self.sift_down(idx);
			Some(old_value)
		} else {
			// New key
			self.nodes.push(HeapNode::new(key, value));
			self.sift_up(self.len() - 1);
			None
		}
	}

	/// Returns a reference to the maximum key-value pair without removing it
	pub fn peek(&self) -> Option<(&K, &V)> {
		self.nodes.first().map(|node| (&node.key, &node.value))
	}

	/// Returns a mutable reference to the value associated with the maximum key
	pub fn peek_mut(&mut self) -> Option<&mut V> {
		self.nodes.first_mut().map(|node| &mut node.value)
	}

	/// Removes and returns the maximum key-value pair
	pub fn extract_max(&mut self) -> Option<(K, V)> {
		if self.is_empty() {
			return None;
		}

		let last_idx = self.len() - 1;
		self.nodes.swap(0, last_idx);
		let HeapNode { key, value } = self.nodes.pop().unwrap();

		if !self.is_empty() {
			self.sift_down(0);
		}

		Some((key, value))
	}

	/// Returns the value associated with the given key, if it exists
	pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
	where
		K: std::borrow::Borrow<Q>,
		Q: Ord + Debug,
	{
		self.find_key_index(key).map(|idx| &self.nodes[idx].value)
	}

	/// Returns a mutable reference to the value associated with the given key
	pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
	where
		K: std::borrow::Borrow<Q>,
		Q: Ord + Debug,
	{
		self.find_key_index(key).map(move |idx| &mut self.nodes[idx].value)
	}

	/// Removes the key-value pair with the given key
	pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
	where
		K: std::borrow::Borrow<Q>,
		Q: Ord + Debug,
	{
		if let Some(index) = self.find_key_index(key) {
			let last_idx = self.len() - 1;
			self.nodes.swap(index, last_idx);
			let HeapNode { value, .. } = self.nodes.pop().unwrap();

			if index != last_idx && !self.is_empty() {
				self.sift_up(index);
				self.sift_down(index);
			}

			Some(value)
		} else {
			None
		}
	}

	/// Returns true if the heap contains the given key
	pub fn contains_key<Q: ?Sized>(&self, key: &Q) -> bool
	where
		K: std::borrow::Borrow<Q>,
		Q: Ord + Debug,
	{
		self.find_key_index(key).is_some()
	}

	/// Returns the height of the heap
	pub fn height(&self) -> usize {
		if self.is_empty() {
			0
		} else {
			(self.len() as f64).log2().floor() as usize + 1
		}
	}

	/// Returns true if the heap property is satisfied
	pub fn is_valid(&self) -> bool {
		for i in 0..self.len() {
			let left = Self::left_child(i);
			let right = Self::right_child(i);

			if left < self.len() && self.nodes[i].key < self.nodes[left].key {
				return false;
			}
			if right < self.len() && self.nodes[i].key < self.nodes[right].key {
				return false;
			}
		}
		true
	}

	// Helper methods
	const fn parent(index: usize) -> usize {
		(index.saturating_sub(1)) / 2
	}

	const fn left_child(index: usize) -> usize {
		2 * index + 1
	}

	const fn right_child(index: usize) -> usize {
		2 * index + 2
	}

	fn find_key_index<Q: ?Sized>(&self, key: &Q) -> Option<usize>
	where
		K: std::borrow::Borrow<Q>,
		Q: Ord + Debug,
	{
		self.nodes.iter().position(|node| node.key.borrow() == key)
	}

	fn sift_up(&mut self, mut index: usize) {
		while index > 0 {
			let parent = Self::parent(index);
			if self.nodes[parent].key >= self.nodes[index].key {
				break;
			}
			self.nodes.swap(parent, index);
			index = parent;
		}
	}

	fn sift_down(&mut self, mut index: usize) {
		loop {
			let left = Self::left_child(index);
			let right = Self::right_child(index);
			let mut largest = index;

			if left < self.len() && self.nodes[left].key > self.nodes[largest].key {
				largest = left;
			}
			if right < self.len() && self.nodes[right].key > self.nodes[largest].key {
				largest = right;
			}

			if largest == index {
				break;
			}

			self.nodes.swap(index, largest);
			index = largest;
		}
	}
}

// Implementation of common traits
impl<K: Ord + Debug, V> Default for ImplicitHeap<K, V> {
	fn default() -> Self {
		Self::new()
	}
}

impl<K: Ord + Debug + Display, V: Display> Display for ImplicitHeap<K, V> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		if self.is_empty() {
			return writeln!(f, "ImplicitHeap: <empty>");
		}

		writeln!(f, "ImplicitHeap:")?;
		self.format_level(f, 0, 0, "")?;
		Ok(())
	}
}

// Helper methods for display
impl<K: Ord + Debug + Display, V: Display> ImplicitHeap<K, V> {
	fn format_level(&self, f: &mut Formatter<'_>, index: usize, level: usize, prefix: &str) -> fmt::Result {
		if index >= self.len() {
			return Ok(());
		}

		let indent = "    ".repeat(level);
		writeln!(f, "{}{}└── ({}, {})", prefix, indent, self.nodes[index].key, self.nodes[index].value)?;

		let left = Self::left_child(index);
		let right = Self::right_child(index);

		if left < self.len() {
			self.format_level(f, left, level + 1, prefix)?;
		}
		if right < self.len() {
			self.format_level(f, right, level + 1, prefix)?;
		}

		Ok(())
	}
}

// Iterator implementation
impl<K: Ord + Debug, V> IntoIterator for ImplicitHeap<K, V> {
	type Item = (K, V);
	type IntoIter = IntoIter<K, V>;

	fn into_iter(self) -> Self::IntoIter {
		IntoIter { heap: self }
	}
}

pub struct IntoIter<K: Ord + Debug, V> {
	heap: ImplicitHeap<K, V>,
}

impl<K: Ord + Debug, V> Iterator for IntoIter<K, V> {
	type Item = (K, V);

	fn next(&mut self) -> Option<Self::Item> {
		self.heap.extract_max()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_new_heap_is_empty() {
		let heap: ImplicitHeap<i32, &str> = ImplicitHeap::new();
		assert!(heap.is_empty());
		assert_eq!(heap.len(), 0);
	}

	#[test]
	fn test_insert_single_element() {
		let mut heap = ImplicitHeap::new();
		heap.insert(10, "ten");
		assert_eq!(heap.len(), 1);
		assert!(!heap.is_empty());
		assert_eq!(heap.peek(), Some((&10, &"ten")));
	}

	#[test]
	fn test_insert_multiple_elements() {
		let mut heap = ImplicitHeap::new();
		heap.insert(30, "thirty");
		heap.insert(20, "twenty");
		heap.insert(40, "forty");

		assert_eq!(heap.len(), 3);
		assert_eq!(heap.peek(), Some((&40, &"forty")));
	}

	#[test]
	fn test_extract_max() {
		let mut heap = ImplicitHeap::new();
		heap.insert(30, "thirty");
		heap.insert(20, "twenty");
		heap.insert(40, "forty");

		let max = heap.extract_max().unwrap();
		assert_eq!(max, (40, "forty"));
		assert_eq!(heap.len(), 2);
		assert_eq!(heap.peek(), Some((&30, &"thirty")));
	}

	#[test]
	fn test_extract_max_empty_heap() {
		let mut heap: ImplicitHeap<i32, &str> = ImplicitHeap::new();
		assert!(heap.extract_max().is_none());
	}

	#[test]
	fn test_insert_existing_key() {
		let mut heap = ImplicitHeap::new();
		heap.insert(30, "thirty");
		heap.insert(30, "new thirty");

		assert_eq!(heap.len(), 1);
		assert_eq!(heap.peek(), Some((&30, &"new thirty")));
	}

	#[test]
	fn test_remove_key() {
		let mut heap = ImplicitHeap::new();
		heap.insert(10, "ten");
		heap.insert(20, "twenty");
		heap.insert(30, "thirty");

		assert_eq!(heap.remove(&20), Some("twenty"));
		assert_eq!(heap.len(), 2);
		assert_eq!(heap.peek(), Some((&30, &"thirty")));
	}

	#[test]
	fn test_remove_non_existent_key() {
		let mut heap = ImplicitHeap::new();
		heap.insert(10, "ten");
		heap.insert(20, "twenty");

		assert_eq!(heap.remove(&30), None);
		assert_eq!(heap.len(), 2);
	}

	#[test]
	fn test_contains_key() {
		let mut heap = ImplicitHeap::new();
		heap.insert(10, "ten");
		heap.insert(20, "twenty");

		assert!(heap.contains_key(&10));
		assert!(!heap.contains_key(&30));
	}

	#[test]
	fn test_peek() {
		let mut heap = ImplicitHeap::new();
		heap.insert(10, "ten");
		heap.insert(20, "twenty");

		assert_eq!(heap.peek(), Some((&20, &"twenty")));
		heap.insert(30, "thirty");
		assert_eq!(heap.peek(), Some((&30, &"thirty")));
	}

	#[test]
	fn test_height() {
		let mut heap = ImplicitHeap::new();
		assert_eq!(heap.height(), 0);

		heap.insert(10, "ten");
		assert_eq!(heap.height(), 1);

		heap.insert(20, "twenty");
		heap.insert(30, "thirty");
		assert_eq!(heap.height(), 2);
	}

	#[test]
	fn test_is_valid_heap() {
		let mut heap = ImplicitHeap::new();
		assert!(heap.is_valid());

		heap.insert(10, "ten");
		assert!(heap.is_valid());

		heap.insert(20, "twenty");
		heap.insert(30, "thirty");
		assert!(heap.is_valid());

		heap.insert(5, "five");
		assert!(heap.is_valid());

		// Manually create an invalid state
		heap.nodes.swap(1, 2); // Swap to make it invalid
		assert!(!heap.is_valid());
	}

	#[test]
	fn test_display_empty_heap() {
		let heap: ImplicitHeap<i32, &str> = ImplicitHeap::new();
		let display = format!("{}", heap);
		assert_eq!(display, "ImplicitHeap: <empty>\n");
	}

	#[test]
	fn test_display_non_empty_heap() {
		let mut heap = ImplicitHeap::new();
		heap.insert(30, "thirty");
		heap.insert(20, "twenty");
		heap.insert(40, "forty");

		let display = format!("{}", heap);
		assert!(display.contains("ImplicitHeap:"));
		assert!(display.contains("└── (40, forty)"));
		assert!(display.contains("    └── (30, thirty)"));
		assert!(display.contains("    └── (20, twenty)"));
	}

	#[test]
	fn test_iterator() {
		let mut heap = ImplicitHeap::new();
		heap.insert(30, "thirty");
		heap.insert(20, "twenty");
		heap.insert(40, "forty");

		let mut values: Vec<(i32, &str)> = heap.into_iter().collect();
		assert_eq!(values.len(), 3);
		values.sort_by(|a, b| a.0.cmp(&b.0));
		assert_eq!(values, vec![(20, "twenty"), (30, "thirty"), (40, "forty")]);
	}
}
