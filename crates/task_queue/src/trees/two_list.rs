use std::fmt::{self, Debug, Display, Formatter};

#[derive(Debug, Clone)]
struct TwoListNode<K, V> {
	key: K,
	value: V,
}

pub struct TwoListHeap<K: Ord + Debug, V> {
	sorted: Vec<TwoListNode<K, V>>,
	unsorted: Vec<TwoListNode<K, V>>,
	max_unsorted: usize,
}

impl<K: Ord + Debug, V> TwoListHeap<K, V> {
	pub fn new(max_unsorted: usize) -> Self {
		Self {
			sorted: Vec::new(),
			unsorted: Vec::new(),
			max_unsorted,
		}
	}

	pub fn is_empty(&self) -> bool {
		self.sorted.is_empty() && self.unsorted.is_empty()
	}

	pub fn len(&self) -> usize {
		self.sorted.len() + self.unsorted.len()
	}

	pub fn insert(&mut self, key: K, value: V) -> Option<V> {
		if let Some(pos) = self.sorted.iter().position(|node| node.key == key) {
			return Some(std::mem::replace(&mut self.sorted[pos].value, value));
		}

		if let Some(pos) = self.unsorted.iter().position(|node| node.key == key) {
			return Some(std::mem::replace(&mut self.unsorted[pos].value, value));
		}

		self.unsorted.push(TwoListNode { key, value });

		if self.unsorted.len() >= self.max_unsorted {
			self.merge_lists();
		}

		None
	}

	pub fn peek(&self) -> Option<(&K, &V)> {
		let sorted_max = self.sorted.last().map(|node| (&node.key, &node.value));
		let unsorted_max = self.unsorted.iter().map(|node| (&node.key, &node.value)).max_by_key(|&(k, _)| k);

		match (sorted_max, unsorted_max) {
			(Some(s), Some(u)) => Some(if s.0 >= u.0 { s } else { u }),
			(Some(s), None) => Some(s),
			(None, Some(u)) => Some(u),
			(None, None) => None,
		}
	}

	pub fn extract_max(&mut self) -> Option<(K, V)> {
		if self.is_empty() {
			return None;
		}

		if !self.unsorted.is_empty() {
			self.merge_lists();
		}

		// Remove and return the maximum element
		self.sorted.pop().map(|node| (node.key, node.value))
	}

	// Helper method to merge unsorted list into sorted list
	fn merge_lists(&mut self) {
		// Sort unsorted list
		self.unsorted.sort_by(|a, b| b.key.cmp(&a.key));

		// Merge sorted lists
		let mut merged = Vec::with_capacity(self.sorted.len() + self.unsorted.len());
		let mut i = 0;
		let mut j = 0;

		while i < self.sorted.len() && j < self.unsorted.len() {
			if self.sorted[i].key >= self.unsorted[j].key {
				merged.push(std::mem::replace(&mut self.sorted[i], unsafe { std::mem::zeroed() }));
				i += 1;
			} else {
				merged.push(std::mem::replace(&mut self.unsorted[j], unsafe { std::mem::zeroed() }));
				j += 1;
			}
		}

		// Add remaining elements
		merged.extend(self.sorted.drain(i..));
		merged.extend(self.unsorted.drain(j..));

		self.sorted = merged;
		self.unsorted.clear();
	}

	/// Returns true if the heap contains the given key
	pub fn contains_key(&self, key: &K) -> bool {
		self.sorted.iter().any(|node| &node.key == key) || self.unsorted.iter().any(|node| &node.key == key)
	}

	/// Returns a reference to the value associated with the given key
	pub fn get(&self, key: &K) -> Option<&V> {
		self
			.sorted
			.iter()
			.find(|node| &node.key == key)
			.map(|node| &node.value)
			.or_else(|| self.unsorted.iter().find(|node| &node.key == key).map(|node| &node.value))
	}

	/// Returns a mutable reference to the value associated with the given key
	pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
		if let Some(pos) = self.sorted.iter().position(|node| &node.key == key) {
			Some(&mut self.sorted[pos].value)
		} else if let Some(pos) = self.unsorted.iter().position(|node| &node.key == key) {
			Some(&mut self.unsorted[pos].value)
		} else {
			None
		}
	}

	/// Removes and returns the value associated with the given key
	pub fn remove(&mut self, key: &K) -> Option<V> {
		if let Some(pos) = self.sorted.iter().position(|node| &node.key == key) {
			Some(self.sorted.remove(pos).value)
		} else if let Some(pos) = self.unsorted.iter().position(|node| &node.key == key) {
			Some(self.unsorted.remove(pos).value)
		} else {
			None
		}
	}
}

// Display implementation for TwoListHeap
impl<K: Ord + Debug + Display, V: Display> Display for TwoListHeap<K, V> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		if self.is_empty() {
			return writeln!(f, "TwoListHeap: <empty>");
		}

		writeln!(f, "TwoListHeap:")?;
		writeln!(f, "Sorted list:")?;
		for node in self.sorted.iter().rev() {
			writeln!(f, "  ({}, {})", node.key, node.value)?;
		}

		if !self.unsorted.is_empty() {
			writeln!(f, "Unsorted list:")?;
			for node in &self.unsorted {
				writeln!(f, "  ({}, {})", node.key, node.value)?;
			}
		}
		Ok(())
	}
}

// Iterator implementation for TwoListHeap
impl<K: Ord + Debug + Clone, V: Clone> IntoIterator for TwoListHeap<K, V> {
	type Item = (K, V);
	type IntoIter = TwoListIntoIter<K, V>;

	fn into_iter(self) -> Self::IntoIter {
		// Merge lists before iteration to ensure correct order
		let mut heap = self;
		if !heap.unsorted.is_empty() {
			heap.merge_lists();
		}
		TwoListIntoIter { heap }
	}
}

pub struct TwoListIntoIter<K: Ord + Debug, V> {
	heap: TwoListHeap<K, V>,
}

impl<K: Ord + Debug + Clone, V: Clone> Iterator for TwoListIntoIter<K, V> {
	type Item = (K, V);

	fn next(&mut self) -> Option<Self::Item> {
		self.heap.extract_max()
	}
}

impl<K: Ord + Debug, V> Default for TwoListHeap<K, V> {
	fn default() -> Self {
		Self::new(100) // Default max_unsorted size
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	mod two_list_heap {
		use super::*;

		#[test]
		fn test_basic_operations() {
			let mut heap = TwoListHeap::new(3);
			assert!(heap.is_empty());
			assert_eq!(heap.len(), 0);

			heap.insert(5, "five");
			assert!(!heap.is_empty());
			assert_eq!(heap.len(), 1);
			assert_eq!(heap.peek(), Some((&5, &"five")));

			heap.insert(3, "three");
			heap.insert(7, "seven");
			assert_eq!(heap.len(), 3);
			assert_eq!(heap.peek(), Some((&7, &"seven")));
		}

		#[test]
		fn test_merge_behavior() {
			let mut heap = TwoListHeap::new(2); // Small max_unsorted to trigger merges

			// Add elements to trigger merge
			heap.insert(5, "five");
			heap.insert(3, "three");
			heap.insert(7, "seven"); // This should trigger merge

			// Verify proper ordering after merge
			assert_eq!(heap.peek(), Some((&7, &"seven")));
			assert_eq!(heap.extract_max(), Some((7, "seven")));
			assert_eq!(heap.extract_max(), Some((5, "five")));
			assert_eq!(heap.extract_max(), Some((3, "three")));
		}

		#[test]
		fn test_extract_max() {
			let mut heap = TwoListHeap::new(5);
			assert_eq!(heap.extract_max(), None);

			let data = vec![(5, "five"), (3, "three"), (7, "seven"), (1, "one")];
			for (k, v) in data {
				heap.insert(k, v);
			}

			assert_eq!(heap.extract_max(), Some((7, "seven")));
			assert_eq!(heap.extract_max(), Some((5, "five")));
			assert_eq!(heap.extract_max(), Some((3, "three")));
			assert_eq!(heap.extract_max(), Some((1, "one")));
			assert_eq!(heap.extract_max(), None);
		}

		#[test]
		fn test_contains_key() {
			let mut heap = TwoListHeap::new(5);
			heap.insert(5, "five");
			heap.insert(3, "three");
			heap.insert(7, "seven");

			assert!(heap.contains_key(&5));
			assert!(heap.contains_key(&3));
			assert!(heap.contains_key(&7));
			assert!(!heap.contains_key(&1));
		}

		#[test]
		fn test_get_and_get_mut() {
			let mut heap = TwoListHeap::new(5);
			heap.insert(5, String::from("five"));
			heap.insert(3, String::from("three"));

			assert_eq!(heap.get(&5), Some(&String::from("five")));
			assert_eq!(heap.get(&3), Some(&String::from("three")));
			assert_eq!(heap.get(&1), None);

			if let Some(value) = heap.get_mut(&5) {
				*value = String::from("FIVE");
			}
			assert_eq!(heap.get(&5), Some(&String::from("FIVE")));
		}

		#[test]
		fn test_remove() {
			let mut heap = TwoListHeap::new(5);
			heap.insert(5, "five");
			heap.insert(3, "three");
			heap.insert(7, "seven");

			assert_eq!(heap.remove(&3), Some("three"));
			assert!(!heap.contains_key(&3));
			assert_eq!(heap.len(), 2);

			assert_eq!(heap.remove(&10), None);
			assert_eq!(heap.remove(&7), Some("seven"));
			assert_eq!(heap.remove(&5), Some("five"));
			assert!(heap.is_empty());
		}

		#[test]
		fn test_iterator() {
			let mut heap = TwoListHeap::new(5);
			let data = vec![(5, "five"), (3, "three"), (7, "seven"), (1, "one")];
			for (k, v) in &data {
				heap.insert(*k, *v);
			}

			let mut collected: Vec<_> = heap.into_iter().collect();
			collected.sort_by_key(|(k, _)| std::cmp::Reverse(*k));

			let mut expected = data.clone();
			expected.sort_by_key(|(k, _)| std::cmp::Reverse(*k));

			assert_eq!(collected, expected);
		}

		#[test]
		fn test_unsorted_buffer_behavior() {
			let mut heap = TwoListHeap::new(3);

			// Fill unsorted buffer
			heap.insert(5, "five");
			heap.insert(3, "three");
			heap.insert(7, "seven"); // Should trigger merge

			// Insert after merge
			heap.insert(6, "six");
			heap.insert(4, "four");

			// Verify correct ordering through extraction
			assert_eq!(heap.extract_max(), Some((7, "seven")));
			assert_eq!(heap.extract_max(), Some((6, "six")));
			assert_eq!(heap.extract_max(), Some((5, "five")));
			assert_eq!(heap.extract_max(), Some((4, "four")));
			assert_eq!(heap.extract_max(), Some((3, "three")));
		}

		#[test]
		fn test_edge_cases() {
			let mut heap = TwoListHeap::new(1); // Minimal unsorted buffer

			// Test empty heap
			assert_eq!(heap.peek(), None);
			assert_eq!(heap.extract_max(), None);
			assert_eq!(heap.remove(&5), None);

			// Test single element
			heap.insert(1, "one");
			assert_eq!(heap.peek(), Some((&1, &"one")));

			// Test immediate merge
			heap.insert(2, "two"); // Should trigger immediate merge
			assert_eq!(heap.peek(), Some((&2, &"two")));
		}

		#[test]
		fn test_duplicate_keys() {
			let mut heap = TwoListHeap::new(5);

			// Insert duplicate keys
			assert_eq!(heap.insert(5, "five"), None);
			assert_eq!(heap.insert(5, "new_five"), Some("five"));
			assert_eq!(heap.get(&5), Some(&"new_five"));

			// Verify only one instance exists
			assert_eq!(heap.len(), 1);
			assert_eq!(heap.extract_max(), Some((5, "new_five")));
			assert!(heap.is_empty());
		}
	}

	// =============== Common Property Tests ===============
	mod property_tests {
		use super::*;

		fn verify_heap_property<H>(heap: &H) -> bool
		where
			H: Iterator<Item = (i32, &'static str)>,
		{
			let elements: Vec<_> = heap.collect();
			for i in 1..elements.len() {
				if elements[i - 1].0 < elements[i].0 {
					return false;
				}
			}
			true
		}

		#[test]
		fn test_heap_property_linked_list() {
			let mut heap = LinkedListHeap::new();
			let data = vec![(5, "five"), (3, "three"), (7, "seven"), (1, "one"), (6, "six"), (4, "four")];
			for (k, v) in data {
				heap.insert(k, v);
			}
			assert!(verify_heap_property(&mut heap.into_iter()));
		}

		#[test]
		fn test_heap_property_two_list() {
			let mut heap = TwoListHeap::new(3);
			let data = vec![(5, "five"), (3, "three"), (7, "seven"), (1, "one"), (6, "six"), (4, "four")];
			for (k, v) in data {
				heap.insert(k, v);
			}
			assert!(verify_heap_property(&mut heap.into_iter()));
		}
	}
}
