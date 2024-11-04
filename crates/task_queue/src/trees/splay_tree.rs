///
/// Portions of this software are adapted from the work of Takeru Ohta, available under the MIT
/// License.
/// Copyright (c) 2016 Takeru Ohta <phjgt308@gmail.com>
/// @see https://github.com/sile/splay_tree/blob/master/src/tree_core.rs
///
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt::{self, Debug, Display};

#[derive(Debug)]
pub struct SplayTree<K: Ord + Debug, V: Debug> {
	pub root: Option<Box<SplayNode<K, V>>>,
}

#[derive(Debug)]
pub struct SplayNode<K, V> {
	key: K,
	value: V,
	left: Option<Box<SplayNode<K, V>>>,
	right: Option<Box<SplayNode<K, V>>>,
}

impl<K: Ord + Debug, V: Debug> Display for SplayTree<K, V> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match &self.root {
			Some(root) => {
				writeln!(f, "SplayTree:")?;
				root.format_tree(f, "", false, true)
			}
			None => writeln!(f, "SplayTree: <empty>"),
		}
	}
}

impl<K: Ord + Debug, V: Debug> SplayNode<K, V> {
	fn format_tree(&self, f: &mut fmt::Formatter<'_>, prefix: &str, is_left: bool, is_root: bool) -> fmt::Result {
		if is_root {
			writeln!(f, "├── ({:?}, {:?})", self.key, self.value)?;
		} else if is_left {
			writeln!(f, "{}├── ({:?}, {:?})", prefix, self.key, self.value)?;
		} else {
			writeln!(f, "{}└── ({:?}, {:?})", prefix, self.key, self.value)?;
		}

		let child_prefix = if is_root {
			"    ".to_string()
		} else {
			format!("{}{}", prefix, if is_left { "│   " } else { "    " })
		};

		if let Some(left) = &self.left {
			left.format_tree(f, &child_prefix, true, false)?;
		}
		if let Some(right) = &self.right {
			right.format_tree(f, &child_prefix, false, false)?;
		}

		Ok(())
	}

	fn new(key: K, value: V) -> Self {
		Self {
			key,
			value,
			left: None,
			right: None,
		}
	}
}

impl<K: Ord + Debug + Default, V: Default + Debug> Default for SplayTree<K, V> {
	fn default() -> Self {
		SplayTree::new()
	}
}

impl<K: Ord + Debug, V: Debug> SplayTree<K, V> {
	pub const fn new() -> Self {
		Self { root: None }
	}

	pub const fn is_empty(&self) -> bool {
		self.root.is_none()
	}

	pub fn contains_key<Q: ?Sized>(&mut self, key: &Q) -> bool
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		match self.splay(key) {
			Some(Ordering::Equal) => true,
			_ => false,
		}
	}

	pub fn get<Q: ?Sized>(&mut self, key: &Q) -> Option<&V>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if self.contains_key(key) {
			self.root.as_ref().map(|node| &node.value)
		} else {
			None
		}
	}

	pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if self.contains_key(key) {
			self.root.as_mut().map(|node| &mut node.value)
		} else {
			None
		}
	}

	pub fn insert(&mut self, key: K, value: V) -> Option<V> {
		if self.root.is_none() {
			self.root = Some(Box::new(SplayNode::new(key, value)));
			return None;
		}

		let _ = self.splay(key.borrow());

		let root = self.root.as_mut().unwrap();
		match key.cmp(&root.key) {
			Ordering::Equal => Some(std::mem::replace(&mut root.value, value)),
			Ordering::Less => {
				let mut new_node = Box::new(SplayNode::new(key, value));
				let old_root = self.root.take().unwrap();
				new_node.right = Some(old_root);
				new_node.left = new_node.right.as_mut().unwrap().left.take();
				self.root = Some(new_node);
				None
			}
			Ordering::Greater => {
				let mut new_node = Box::new(SplayNode::new(key, value));
				let old_root = self.root.take().unwrap();
				new_node.left = Some(old_root);
				new_node.right = new_node.left.as_mut().unwrap().right.take();
				self.root = Some(new_node);
				None
			}
		}
	}

	pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if let Some(Ordering::Equal) = self.splay(key) {
			let root = self.root.take().unwrap();
			match (root.left, root.right) {
				(None, right) => {
					self.root = right;
					Some(root.value)
				}
				(left, None) => {
					self.root = left;
					Some(root.value)
				}
				(left, right) => {
					let mut current = left.unwrap();
					let mut stack = Vec::new();

					// Find maximum in left subtree
					while current.right.is_some() {
						let next = current.right.take().unwrap();
						stack.push(current);
						current = next;
					}

					// Restructure the tree
					let mut new_root = current;
					if let Some(mut last) = stack.pop() {
						last.right = new_root.left.take();
						new_root.left = Some(last);

						// Reattach the rest of the stack
						while let Some(node) = stack.pop() {
							let old_new_root = std::mem::replace(&mut new_root, node);
							new_root.right = Some(old_new_root);
						}
					}

					new_root.right = right;
					self.root = Some(new_root);
					Some(root.value)
				}
			}
		} else {
			None
		}
	}

	fn splay<Q: ?Sized>(&mut self, key: &Q) -> Option<Ordering>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if self.root.is_none() {
			return None;
		}

		let mut stack = Vec::new();
		let mut current = self.root.take().unwrap();
		let found_order: Option<Ordering>;

		// Phase 1: Search and build stack
		loop {
			match key.cmp(current.key.borrow()) {
				Ordering::Equal => {
					found_order = Some(Ordering::Equal);
					break;
				}
				Ordering::Less => {
					if let Some(left) = current.left.take() {
						stack.push((current, true));
						current = left;
					} else {
						found_order = Some(Ordering::Less);
						break;
					}
				}
				Ordering::Greater => {
					if let Some(right) = current.right.take() {
						stack.push((current, false));
						current = right;
					} else {
						found_order = Some(Ordering::Greater);
						break;
					}
				}
			}
		}

		// Phase 2: Splay rotations
		while let Some((parent, is_left_child)) = stack.pop() {
			let mut grandparent = self.root.take().unwrap();

			if is_left_child {
				let right = current.right.take();
				current.right = Some(parent);
				grandparent.left = right;
				self.root = Some(current);
			} else {
				let left = current.left.take();
				current.left = Some(parent);
				grandparent.right = left;
				self.root = Some(current);
			}

			current = grandparent; // Update current to the new grandparent for the next iteration
		}

		self.root = Some(current);
		// Return the found order
		found_order
	}

	pub fn get_min(&mut self) -> Option<(&K, &V)> {
		if self.root.is_none() {
			return None;
		}

		while let Some(_left) = self.root.as_ref().unwrap().left.as_ref() {
			let mut current = self.root.take().unwrap();
			let mut new_root = current.left.take().unwrap();
			current.left = new_root.right.take();
			new_root.right = Some(current);
			self.root = Some(new_root);
		}

		self.root.as_ref().map(|node| (&node.key, &node.value))
	}

	pub fn get_max(&mut self) -> Option<(&K, &V)> {
		if self.root.is_none() {
			return None;
		}

		while let Some(_right) = self.root.as_ref().unwrap().right.as_ref() {
			let mut current = self.root.take().unwrap();
			let mut new_root = current.right.take().unwrap();
			current.right = new_root.left.take();
			new_root.left = Some(current);
			self.root = Some(new_root);
		}

		self.root.as_ref().map(|node| (&node.key, &node.value))
	}

	pub fn remove_min(&mut self) -> Option<(K, V)> {
		if self.get_min().is_none() {
			return None;
		}

		let root = self.root.take().unwrap();
		self.root = root.right;
		Some((root.key, root.value))
	}

	pub fn remove_max(&mut self) -> Option<(K, V)> {
		if self.get_max().is_none() {
			return None;
		}

		let root = self.root.take().unwrap();
		self.root = root.left;
		Some((root.key, root.value))
	}
}

impl<K: Ord + Debug, V: Debug> IntoIterator for SplayTree<K, V> {
	type Item = (K, V);
	type IntoIter = IntoIter<K, V>;

	fn into_iter(self) -> Self::IntoIter {
		IntoIter { tree: self }
	}
}

pub struct IntoIter<K: Ord + Debug, V: Debug> {
	tree: SplayTree<K, V>,
}

impl<K: Ord + Debug, V: Debug> Iterator for IntoIter<K, V> {
	type Item = (K, V);

	fn next(&mut self) -> Option<Self::Item> {
		self.tree.remove_min()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_new_and_empty() {
		let tree: SplayTree<i32, &str> = SplayTree::new();
		assert!(tree.is_empty());
	}

	#[test]
	fn test_basic_insert_and_get() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Test single insert
		assert_eq!(tree.insert(1, "one"), None);
		assert!(!tree.is_empty());

		// Test get after insert
		assert_eq!(tree.get(&1), Some(&"one"));
		assert_eq!(tree.get(&2), None);

		// Test value replacement
		assert_eq!(tree.insert(1, "new_one"), Some("one"));
		assert_eq!(tree.get(&1), Some(&"new_one"));
	}

	#[test]
	fn test_multiple_inserts() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Insert multiple values
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");
		tree.insert(1, "one");
		tree.insert(9, "nine");

		// Verify all values
		assert_eq!(tree.get(&1), Some(&"one"));
		assert_eq!(tree.get(&3), Some(&"three"));
		assert_eq!(tree.get(&5), Some(&"five"));
		assert_eq!(tree.get(&7), Some(&"seven"));
		assert_eq!(tree.get(&9), Some(&"nine"));

		// Verify non-existent values
		assert_eq!(tree.get(&0), None);
		assert_eq!(tree.get(&2), None);
		assert_eq!(tree.get(&10), None);
	}

	#[test]
	fn test_contains_key() {
		let mut tree = SplayTree::<i32, &str>::default();

		tree.insert(1, "one");
		tree.insert(2, "two");

		assert!(tree.contains_key(&1));
		assert!(tree.contains_key(&2));
		assert!(!tree.contains_key(&3));
	}

	#[test]
	fn test_get_mut() {
		let mut tree = SplayTree::<i32, String>::default();

		tree.insert(1, String::from("one"));

		if let Some(value) = tree.get_mut(&1) {
			*value = String::from("modified");
		}

		assert_eq!(tree.get(&1), Some(&String::from("modified")));
	}

	#[test]
	fn test_remove() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Test remove on empty tree
		assert_eq!(tree.remove(&1), None);

		// Insert and remove single element
		tree.insert(1, "one");
		assert_eq!(tree.remove(&1), Some("one"));
		assert!(tree.is_empty());

		// Test multiple inserts and removes
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");

		assert_eq!(tree.remove(&3), Some("three"));
		assert_eq!(tree.get(&3), None);
		assert_eq!(tree.get(&5), Some(&"five"));
		assert_eq!(tree.get(&7), Some(&"seven"));

		// Test removing non-existent key
		assert_eq!(tree.remove(&3), None);
	}

	#[test]
	fn test_min_max_operations() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Test on empty tree
		assert_eq!(tree.get_min(), None);
		assert_eq!(tree.get_max(), None);
		assert_eq!(tree.remove_min(), None);
		assert_eq!(tree.remove_max(), None);

		// Insert values
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");
		tree.insert(1, "one");
		tree.insert(9, "nine");

		// Test get_min and get_max
		println!("Current splay: {}", tree);
		assert_eq!(tree.get_min(), Some((&1, &"one")));
		assert_eq!(tree.get_max(), Some((&9, &"nine")));

		// Test remove_min
		assert_eq!(tree.remove_min(), Some((1, "one")));
		assert_eq!(tree.get_min(), Some((&3, &"three")));

		// Test remove_max
		assert_eq!(tree.remove_max(), Some((9, "nine")));
		assert_eq!(tree.get_max(), Some((&7, &"seven")));
	}

	#[test]
	fn test_iterator() {
		let tree = SplayTree::<i32, &str>::default();

		// Test iterator on empty tree
		assert_eq!(tree.into_iter().count(), 0);

		// Create new tree with values
		let mut tree = SplayTree::<i32, &str>::default();
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");

		// Collect into vector and verify order
		let items: Vec<_> = tree.into_iter().collect();
		assert_eq!(items, vec![(3, "three"), (5, "five"), (7, "seven")]);
	}

	#[test]
	fn test_complex_operations() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Mix of operations
		tree.insert(5, "five");
		assert_eq!(tree.get_max(), Some((&5, &"five")));

		tree.insert(3, "three");
		assert_eq!(tree.remove_min(), Some((3, "three")));

		tree.insert(7, "seven");
		assert_eq!(tree.get_min(), Some((&5, &"five")));

		tree.insert(1, "one");
		assert_eq!(tree.remove_max(), Some((7, "seven")));

		assert_eq!(tree.get(&5), Some(&"five"));
		assert_eq!(tree.get(&1), Some(&"one"));
	}

	#[test]
	fn test_string_keys() {
		let mut tree = SplayTree::<String, i32>::default();

		tree.insert(String::from("apple"), 1);
		tree.insert(String::from("banana"), 2);
		tree.insert(String::from("cherry"), 3);

		assert_eq!(tree.get("apple"), Some(&1));
		assert_eq!(tree.get("banana"), Some(&2));
		assert_eq!(tree.get("cherry"), Some(&3));
		assert_eq!(tree.get("date"), None);

		assert_eq!(tree.remove("banana"), Some(2));
		assert_eq!(tree.get("banana"), None);
	}

	#[test]
	fn test_custom_type() {
		#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
		struct CustomKey(i32);

		let mut tree = SplayTree::<CustomKey, &str>::new();

		tree.insert(CustomKey(1), "one");
		tree.insert(CustomKey(2), "two");

		assert_eq!(tree.get(&CustomKey(1)), Some(&"one"));
		assert_eq!(tree.get(&CustomKey(2)), Some(&"two"));
		assert_eq!(tree.remove(&CustomKey(1)), Some("one"));
	}

	#[test]
	fn test_edge_cases() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Test inserting and removing single node repeatedly
		tree.insert(1, "one");
		assert_eq!(tree.remove(&1), Some("one"));
		assert!(tree.is_empty());

		tree.insert(1, "one");
		assert_eq!(tree.get(&1), Some(&"one"));

		// Test replacing root multiple times
		tree.insert(2, "two");
		tree.insert(1, "new_one");
		assert_eq!(tree.get(&1), Some(&"new_one"));

		// Test removing root with different child configurations
		tree.insert(3, "three");
		assert_eq!(tree.remove(&2), Some("two")); // Remove node with two children
		assert_eq!(tree.remove(&1), Some("new_one")); // Remove node with one child
		assert_eq!(tree.remove(&3), Some("three")); // Remove leaf node
		assert!(tree.is_empty());
	}
}
