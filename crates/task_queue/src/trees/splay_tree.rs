///
/// Portions of this software are adapted from the work of Takeru Ohta, available under the MIT
/// License.
/// Copyright (c) 2016 Takeru Ohta <phjgt308@gmail.com>
/// @see https://github.com/sile/splay_tree/blob/master/src/tree_core.rs
///
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt::{self, Debug, Display};
use std::mem;

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
				root.format_tree(f, 0, true)
			}
			None => writeln!(f, "SplayTree: <empty>"),
		}
	}
}

impl<K: Ord + Debug, V: Debug> SplayNode<K, V> {
	fn format_tree(&self, f: &mut fmt::Formatter<'_>, depth: usize, is_root: bool) -> fmt::Result {
		let indent = "    ".repeat(depth);

		// Print left child first for left-to-right structure
		if let Some(left) = &self.left {
			left.format_tree(f, depth + 1, false)?;
		}

		// Print current node
		if is_root {
			writeln!(f, "{}({:?}, {:?})", indent, self.key, self.value)?;
		} else {
			writeln!(
				f,
				"{}{} ({:?}, {:?})",
				indent,
				if depth == 0 {
					""
				} else if self.left.is_some() {
					"┌──"
				} else {
					"└──"
				},
				self.key,
				self.value
			)?;
		}

		// Print right child
		if let Some(right) = &self.right {
			right.format_tree(f, depth + 1, false)?;
		}

		Ok(())
	}

	const fn new(key: K, value: V) -> Self {
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
		Self::new()
	}
}

impl<K: Ord + Debug, V: Debug> SplayTree<K, V> {
	pub const fn new() -> Self {
		Self { root: None }
	}

	pub const fn is_empty(&self) -> bool {
		self.root.is_none()
	}

	fn rotate_right(mut node: Box<SplayNode<K, V>>) -> Box<SplayNode<K, V>> {
		let mut new_root = node.left.take().unwrap();
		node.left = new_root.right.take();
		new_root.right = Some(node);
		new_root
	}

	fn rotate_left(mut node: Box<SplayNode<K, V>>) -> Box<SplayNode<K, V>> {
		let mut new_root = node.right.take().unwrap();
		node.right = new_root.left.take();
		new_root.left = Some(node);
		new_root
	}

	// Handle reassembly of the tree after splaying
	fn reassemble_tree(mut root: Box<SplayNode<K, V>>, left_tree: Option<Box<SplayNode<K, V>>>, right_tree: Option<Box<SplayNode<K, V>>>) -> Box<SplayNode<K, V>> {
		if let Some(mut l) = left_tree {
			l.right = root.left.take();
			root.left = Some(l);
		}
		if let Some(mut r) = right_tree {
			r.left = root.right.take();
			root.right = Some(r);
		}
		root
	}

	// Handle zig step (single rotation) to the right
	fn handle_zig_right(root: Box<SplayNode<K, V>>, right_tree: Option<Box<SplayNode<K, V>>>) -> (Box<SplayNode<K, V>>, Option<Box<SplayNode<K, V>>>) {
		let mut old_root = root;
		let mut new_root = old_root.left.take().unwrap();
		old_root.left = new_root.right.take();

		let existing_right = old_root.right.take();

		let updated_right_tree = match (existing_right, right_tree) {
			(None, right) => {
				old_root.right = right;
				Some(old_root)
			}
			(Some(existing), None) => {
				old_root.right = Some(existing);
				Some(old_root)
			}
			(Some(existing), Some(right)) => {
				old_root.right = Some(existing);
				let mut current = &mut old_root.right;
				while let Some(ref mut node) = *current {
					if node.right.is_none() {
						node.right = Some(right);
						break;
					}
					current = &mut node.right;
				}
				Some(old_root)
			}
		};

		(new_root, updated_right_tree)
	}

	// Handle zig step (single rotation) to the left
	fn handle_zig_left(root: Box<SplayNode<K, V>>, left_tree: Option<Box<SplayNode<K, V>>>) -> (Box<SplayNode<K, V>>, Option<Box<SplayNode<K, V>>>) {
		let mut old_root = root;
		let mut new_root = old_root.right.take().unwrap();
		old_root.right = new_root.left.take();

		let existing_left = old_root.left.take();

		let updated_left_tree = match (existing_left, left_tree) {
			(None, left) => {
				old_root.left = left;
				Some(old_root)
			}
			(Some(existing), None) => {
				old_root.left = Some(existing);
				Some(old_root)
			}
			(Some(existing), Some(left)) => {
				old_root.left = Some(existing);
				let mut current = &mut old_root.left;
				while let Some(ref mut node) = *current {
					if node.left.is_none() {
						node.left = Some(left);
						break;
					}
					current = &mut node.left;
				}
				Some(old_root)
			}
		};

		(new_root, updated_left_tree)
	}

	// Main splay operation
	pub fn splay<Q: ?Sized>(&mut self, key: &Q) -> Option<Ordering>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if self.root.is_none() {
			return None;
		}

		let mut root = self.root.take().unwrap();
		let mut left_tree: Option<Box<SplayNode<K, V>>> = None;
		let mut right_tree: Option<Box<SplayNode<K, V>>> = None;

		loop {
			match key.cmp(root.key.borrow()) {
				Ordering::Equal => {
					let reassembled = Self::reassemble_tree(root, left_tree, right_tree);
					self.root = Some(reassembled);
					return Some(Ordering::Equal);
				}
				Ordering::Less => {
					if root.left.is_none() {
						let reassembled = Self::reassemble_tree(root, left_tree, right_tree);
						self.root = Some(reassembled);
						return Some(Ordering::Less);
					}

					// Check for zig-zig case
					if let Some(ref left) = root.left {
						if key.cmp(left.key.borrow()) == Ordering::Less {
							if left.left.is_some() {
								// Handle zig-zig case
								root = Self::rotate_right(root);
								let (new_root, new_right_tree) = Self::handle_zig_right(root, right_tree);
								root = new_root;
								right_tree = new_right_tree;
								continue;
							}
						}
					}

					// Handle regular zig case
					let (new_root, new_right_tree) = Self::handle_zig_right(root, right_tree);
					root = new_root;
					right_tree = new_right_tree;
				}
				Ordering::Greater => {
					if root.right.is_none() {
						let reassembled = Self::reassemble_tree(root, left_tree, right_tree);
						self.root = Some(reassembled);
						return Some(Ordering::Greater);
					}

					// Check for zag-zag case
					if let Some(ref right) = root.right {
						if key.cmp(right.key.borrow()) == Ordering::Greater {
							if right.right.is_some() {
								// Handle zag-zag case
								root = Self::rotate_left(root);
								let (new_root, new_left_tree) = Self::handle_zig_left(root, left_tree);
								root = new_root;
								left_tree = new_left_tree;
								continue;
							}
						}
					}

					// Handle regular zag case
					let (new_root, new_left_tree) = Self::handle_zig_left(root, left_tree);
					root = new_root;
					left_tree = new_left_tree;
				}
			}
		}
	}
	pub fn contains_key<Q: ?Sized>(&mut self, key: &Q) -> bool
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		matches!(self.splay(key), Some(Ordering::Equal))
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
			Ordering::Equal => Some(mem::replace(&mut root.value, value)),
			Ordering::Less => {
				let mut new_node = Box::new(SplayNode::new(key, value));
				mem::swap(&mut new_node.right, &mut self.root);
				if let Some(old_root) = new_node.right.as_mut() {
					mem::swap(&mut new_node.left, &mut old_root.left);
				}
				self.root = Some(new_node);
				None
			}
			Ordering::Greater => {
				let mut new_node = Box::new(SplayNode::new(key, value));
				mem::swap(&mut new_node.left, &mut self.root);
				if let Some(old_root) = new_node.left.as_mut() {
					mem::swap(&mut new_node.right, &mut old_root.right);
				}
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
				(Some(left), right) => {
					self.root = Some(left);
					self.splay(key);
					self.root.as_mut().unwrap().right = right;
					Some(root.value)
				}
			}
		} else {
			None
		}
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
		// Find the minimum value in the tree
		if self.tree.root.is_none() {
			return None;
		}

		// Helper function to find and remove minimum node
		fn remove_min<K: Ord + Debug, V: Debug>(node: &mut Option<Box<SplayNode<K, V>>>) -> Option<(K, V)> {
			if let Some(mut root) = node.take() {
				if root.left.is_none() {
					// This is the minimum node
					*node = root.right.take();
					Some((root.key, root.value))
				} else {
					// Keep searching in left subtree
					let result = remove_min(&mut root.left);
					*node = Some(root);
					result
				}
			} else {
				None
			}
		}

		remove_min(&mut self.tree.root)
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
		let mut tree = SplayTree::<i32, &str>::new();

		// Test single insert
		assert_eq!(tree.insert(1, "one"), None);
		assert!(!tree.is_empty());

		// Test get after insert
		assert_eq!(tree.get(&1), Some(&"one"));
		assert_eq!(tree.get(&2), None);

		// Verify splaying occurred - root should be 2 after failed get
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 1);
		}

		// Test value replacement
		assert_eq!(tree.insert(1, "new_one"), Some("one"));
		assert_eq!(tree.get(&1), Some(&"new_one"));
	}

	#[test]
	fn test_rotate_right() {
		let mut root = Box::new(SplayNode::new(2, "b"));
		let mut left = Box::new(SplayNode::new(1, "a"));
		let right_child = Box::new(SplayNode::new(3, "c"));

		left.right = Some(Box::new(SplayNode::new(4, "d")));
		root.left = Some(left);
		root.right = Some(right_child);

		let rotated = SplayTree::<i32, &str>::rotate_right(root);
		assert_eq!(rotated.key, 1);
		assert_eq!(rotated.right.as_ref().unwrap().key, 2);
		assert_eq!(rotated.right.as_ref().unwrap().right.as_ref().unwrap().key, 3);
	}

	#[test]
	fn test_rotate_left() {
		let mut root = Box::new(SplayNode::new(2, "b"));
		let mut right = Box::new(SplayNode::new(3, "c"));
		let left_child = Box::new(SplayNode::new(1, "a"));

		right.left = Some(Box::new(SplayNode::new(4, "d")));
		root.right = Some(right);
		root.left = Some(left_child);

		let rotated = SplayTree::<i32, &str>::rotate_left(root);
		assert_eq!(rotated.key, 3);
		assert_eq!(rotated.left.as_ref().unwrap().key, 2);
		assert_eq!(rotated.left.as_ref().unwrap().left.as_ref().unwrap().key, 1);
	}

	#[test]
	fn test_reassemble_tree() {
		let root = Box::new(SplayNode::new(2, "b"));
		let left_tree = Some(Box::new(SplayNode::new(1, "a")));
		let right_tree = Some(Box::new(SplayNode::new(3, "c")));

		let reassembled = SplayTree::<i32, &str>::reassemble_tree(root, left_tree, right_tree);
		assert_eq!(reassembled.key, 2);
		assert_eq!(reassembled.left.as_ref().unwrap().key, 1);
		assert_eq!(reassembled.right.as_ref().unwrap().key, 3);
	}

	#[test]
	fn test_handle_zig_right() {
		let mut root = Box::new(SplayNode::new(2, "b"));
		root.left = Some(Box::new(SplayNode::new(1, "a")));
		let right_tree = Some(Box::new(SplayNode::new(3, "c")));

		let (new_root, new_right_tree) = SplayTree::<i32, &str>::handle_zig_right(root, right_tree);
		assert_eq!(new_root.key, 1);
		assert_eq!(new_right_tree.as_ref().unwrap().key, 2);
		assert_eq!(new_right_tree.as_ref().unwrap().right.as_ref().unwrap().key, 3);
	}

	#[test]
	fn test_handle_zig_left() {
		let mut root = Box::new(SplayNode::new(2, "b"));
		root.right = Some(Box::new(SplayNode::new(3, "c")));
		let left_tree = Some(Box::new(SplayNode::new(1, "a")));

		let (new_root, new_left_tree) = SplayTree::<i32, &str>::handle_zig_left(root, left_tree);
		assert_eq!(new_root.key, 3);
		assert_eq!(new_left_tree.as_ref().unwrap().key, 2);
		assert_eq!(new_left_tree.as_ref().unwrap().left.as_ref().unwrap().key, 1);
	}

	#[test]
	fn test_zig_operation() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Insert two nodes to test simple zig (single rotation)
		tree.insert(2, "two");
		tree.insert(1, "one");

		// Access 1 which should trigger a zig rotation
		tree.get(&1);

		// Verify 1 is now at root
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 1);
			// Verify 2 is right child
			if let Some(right) = &root.right {
				assert_eq!(right.key, 2);
			} else {
				panic!("Expected 2 as right child after zig rotation");
			}
		}
	}

	#[test]
	fn test_zig_zig_operation() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Create a right-leaning path of three nodes
		tree.insert(3, "three");
		tree.insert(2, "two");
		tree.insert(1, "one");

		// Access 1 which should trigger a zig-zig rotation
		tree.get(&1);

		// Verify 1 is at root and structure is correct
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 1);
			if let Some(right) = &root.right {
				assert_eq!(right.key, 2);
				if let Some(right_right) = &right.right {
					assert_eq!(right_right.key, 3);
				} else {
					panic!("Expected 3 as right-right child after zig-zig rotation");
				}
			}
		}
	}

	#[test]
	fn test_zig_zag_operation() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Create a structure for zig-zag case
		tree.insert(3, "three");
		tree.insert(1, "one");
		tree.insert(2, "two"); // This creates a zig-zag pattern

		// Access 2 which should trigger a zig-zag rotation
		tree.get(&2);

		// Verify 2 is at root with 1 as left child and 3 as right child
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 2);
			// Check left subtree
			if let Some(left) = &root.left {
				assert_eq!(left.key, 1);
			} else {
				panic!("Expected 1 as left child after zig-zag rotation");
			}
			// Check right subtree
			if let Some(right) = &root.right {
				assert_eq!(right.key, 3);
			} else {
				panic!("Expected 3 as right child after zig-zag rotation");
			}
		}
	}

	#[test]
	fn test_multiple_zig_operations() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Setup initial balanced structure
		tree.insert(4, "four");
		tree.insert(2, "two");
		tree.insert(6, "six");
		tree.insert(1, "one");
		tree.insert(3, "three");
		tree.insert(5, "five");
		tree.insert(7, "seven");

		// Test first zig operation - access leftmost node
		tree.get(&1);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 1);
			if let Some(right) = &root.right {
				assert_eq!(right.key, 2);
			}
		}

		// Test second zig operation - access new rightmost node
		tree.get(&7);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 7);
			if let Some(left) = &root.left {
				assert_eq!(left.key, 6);
			}
		}

		// Test third zig operation - access middle node
		tree.get(&4);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 4);
			if let Some(left) = &root.left {
				assert_eq!(left.key, 3);
			}
		}
	}

	#[test]
	fn test_multiple_zig_zig_operations() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Setup for first zig-zig
		tree.insert(8, "eight");
		tree.insert(6, "six");
		tree.insert(4, "four");
		tree.insert(2, "two");

		// First zig-zig operation
		tree.get(&2);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 2);
			if let Some(right) = &root.right {
				assert_eq!(right.key, 4);
				if let Some(right_right) = &right.right {
					assert_eq!(right_right.key, 6);
				}
			}
		}

		// Setup for second zig-zig
		tree.insert(1, "one");
		tree.insert(0, "zero");

		// Second zig-zig operation
		tree.get(&0);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 0);
			if let Some(right) = &root.right {
				assert_eq!(right.key, 1);
				if let Some(right_right) = &right.right {
					assert_eq!(right_right.key, 2);
				}
			}
		}

		// Setup for third zig-zig
		tree.insert(10, "ten");
		tree.insert(12, "twelve");
		tree.insert(14, "fourteen");

		// Third zig-zig operation
		tree.get(&14);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 14);
			if let Some(left) = &root.left {
				assert_eq!(left.key, 12);
				if let Some(left_left) = &left.left {
					assert_eq!(left_left.key, 10);
				}
			}
		}
	}

	#[test]
	fn test_multiple_zig_zag_operations() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Setup for first zig-zag
		tree.insert(6, "six");
		tree.insert(2, "two");
		tree.insert(4, "four");

		// First zig-zag operation
		tree.get(&4);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 4);
			if let Some(left) = &root.left {
				assert_eq!(left.key, 2);
			}
			if let Some(right) = &root.right {
				assert_eq!(right.key, 6);
			}
		}

		// Setup for second zig-zag
		tree.insert(1, "one");
		tree.insert(3, "three");

		// Second zig-zag operation
		tree.get(&3);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 3);
			if let Some(left) = &root.left {
				assert_eq!(left.key, 1);
			}
			if let Some(right) = &root.right {
				assert_eq!(right.key, 4);
			}
		}

		// Setup for third zig-zag
		tree.insert(8, "eight");
		tree.insert(7, "seven");

		// Third zig-zag operation
		tree.get(&7);
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 7);
			if let Some(left) = &root.left {
				assert_eq!(left.key, 4);
			}
			if let Some(right) = &root.right {
				assert_eq!(right.key, 8);
			}
		}
	}

	#[test]
	fn test_complex_insert_remove() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Test sequence of inserts
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");

		// Verify structure after inserts
		assert_eq!(tree.get(&3), Some(&"three"));
		assert_eq!(tree.get(&5), Some(&"five"));
		assert_eq!(tree.get(&7), Some(&"seven"));

		// Test remove with different cases
		assert_eq!(tree.remove(&3), Some("three")); // Remove with two children
		assert_eq!(tree.get(&3), None);

		assert_eq!(tree.remove(&7), Some("seven")); // Remove leaf
		assert_eq!(tree.get(&7), None);

		assert_eq!(tree.remove(&5), Some("five")); // Remove root
		assert!(tree.is_empty());
	}

	#[test]
	fn test_basic_operations() {
		let mut tree = SplayTree::new();

		// Test empty tree
		assert!(tree.is_empty());
		assert_eq!(tree.get(&5), None);

		// Test insertions
		assert_eq!(tree.insert(5, "five"), None);
		assert_eq!(tree.insert(3, "three"), None);
		assert_eq!(tree.insert(7, "seven"), None);

		// Test successful gets
		assert_eq!(tree.get(&5), Some(&"five"));
		assert_eq!(tree.get(&3), Some(&"three"));
		assert_eq!(tree.get(&7), Some(&"seven"));

		// Test unsuccessful get
		assert_eq!(tree.get(&4), None);

		// Test value replacement
		assert_eq!(tree.insert(5, "new_five"), Some("five"));
		assert_eq!(tree.get(&5), Some(&"new_five"));

		// Test removal
		assert_eq!(tree.remove(&3), Some("three"));
		assert_eq!(tree.get(&3), None);
		assert_eq!(tree.remove(&3), None); // Try removing non-existent key
	}

	#[test]
	fn test_splay_behavior() {
		let mut tree = SplayTree::new();

		// Insert elements
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");
		tree.insert(2, "two");
		tree.insert(4, "four");
		tree.insert(6, "six");
		tree.insert(8, "eight");

		// Access element - should move to root
		tree.get(&4);
		assert_eq!(tree.root.as_ref().unwrap().key, 4);

		// Access another element - should become new root
		tree.get(&7);
		assert_eq!(tree.root.as_ref().unwrap().key, 7);

		// Even unsuccessful searches should splay the last accessed node
		tree.get(&9);
		assert_eq!(tree.root.as_ref().unwrap().key, 8);
	}

	#[test]
	fn test_complex_operations() {
		let mut tree = SplayTree::new();

		// Insert a sequence of numbers
		for i in 0..10 {
			tree.insert(i, i.to_string());
		}

		// Verify all elements can be accessed
		for i in 0..10 {
			assert_eq!(tree.get(&i), Some(&i.to_string()));
			// After each access, the accessed element should be at the root
			assert_eq!(tree.root.as_ref().unwrap().key, i);
		}

		// Remove elements in random order
		let remove_sequence = [4, 1, 8, 3, 6, 0, 9, 2, 5, 7];
		for &i in &remove_sequence {
			assert_eq!(tree.remove(&i), Some(i.to_string()));
			assert_eq!(tree.get(&i), None);
		}

		// Tree should be empty
		assert!(tree.is_empty());
	}
	#[test]
	fn test_iterator() {
		let tree = SplayTree::<i32, &str>::default();

		// Test empty tree iterator
		assert_eq!(tree.into_iter().count(), 0);

		// Create new tree with values
		let mut tree = SplayTree::<i32, &str>::default();
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");
		tree.insert(1, "one");
		tree.insert(9, "nine");

		// Collect and verify in-order traversal
		let items: Vec<_> = tree.into_iter().collect();

		// Check keys are in order
		let keys: Vec<_> = items.iter().map(|(k, _)| k).collect();
		let mut sorted_keys = keys.clone();
		sorted_keys.sort();
		assert_eq!(keys, sorted_keys);
	}

	#[test]
	fn test_string_keys() {
		let mut tree = SplayTree::<String, i32>::default();

		// Test with string keys
		tree.insert(String::from("apple"), 1);
		tree.insert(String::from("banana"), 2);
		tree.insert(String::from("cherry"), 3);

		assert_eq!(tree.get("apple"), Some(&1));
		assert_eq!(tree.get("banana"), Some(&2));
		assert_eq!(tree.get("cherry"), Some(&3));
		assert_eq!(tree.get("date"), None);

		// Test removal with string keys
		assert_eq!(tree.remove("banana"), Some(2));
		assert_eq!(tree.get("banana"), None);

		// Verify tree structure remains valid
		assert_eq!(tree.get("apple"), Some(&1));
		assert_eq!(tree.get("cherry"), Some(&3));
	}

	#[test]
	fn test_edge_cases() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Test operations on empty tree
		assert_eq!(tree.get(&1), None);
		assert_eq!(tree.remove(&1), None);

		// Test single node operations
		tree.insert(1, "one");
		assert_eq!(tree.remove(&1), Some("one"));
		assert!(tree.is_empty());

		// Test repeated insertions and removals
		tree.insert(1, "one");
		tree.insert(1, "new_one");
		assert_eq!(tree.get(&1), Some(&"new_one"));
		assert_eq!(tree.remove(&1), Some("new_one"));

		// Test with nodes having one child
		tree.insert(2, "two");
		tree.insert(1, "one");
		assert_eq!(tree.remove(&2), Some("two"));
		assert_eq!(tree.get(&1), Some(&"one"));

		// Test removing root with complex subtrees
		tree.insert(3, "three");
		tree.insert(2, "two");
		assert_eq!(tree.remove(&1), Some("one"));
		assert_eq!(tree.get(&2), Some(&"two"));
		assert_eq!(tree.get(&3), Some(&"three"));
	}

	#[test]
	fn test_borrow_trait() {
		let mut tree = SplayTree::<String, i32>::default();

		tree.insert(String::from("test"), 1);

		// Test different types of borrows
		assert!(tree.contains_key("test")); // &str
		assert!(tree.contains_key(&String::from("test"))); // &String
		assert!(tree.contains_key(&"test".to_owned())); // &String from owned

		// Test get with different borrow types
		assert_eq!(tree.get("test"), Some(&1));
		assert_eq!(tree.get(&String::from("test")), Some(&1));
	}
}
