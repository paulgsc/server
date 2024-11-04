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

#[derive(Debug)]
pub struct SplayNode<K, V> {
	key: K,
	value: V,
	left: Option<Box<SplayNode<K, V>>>,
	right: Option<Box<SplayNode<K, V>>>,
}

impl<K: Ord + Debug, V: Debug> SplayNode<K, V> {
	fn format_tree(&self, f: &mut fmt::Formatter<'_>, prefix: &str, is_left: bool, is_root: bool) -> fmt::Result {
		// Print current node
		if is_root {
			writeln!(f, "└── ({:?}, {:?})", self.key, self.value)?;
		} else if is_left {
			writeln!(f, "{}├── ({:?}, {:?})", prefix, self.key, self.value)?;
		} else {
			writeln!(f, "{}└── ({:?}, {:?})", prefix, self.key, self.value)?;
		}
		// Prepare the prefix for children
		let child_prefix = if is_root {
			"    ".to_string()
		} else {
			format!("{}{}", prefix, if is_left { "│   " } else { "    " })
		};

		// Recursively print left and right subtrees
		if let Some(left) = &self.left {
			left.format_tree(f, &child_prefix, true, false)?;
		}
		if let Some(right) = &self.right {
			right.format_tree(f, &child_prefix, false, false)?;
		}

		Ok(())
	}
}

impl<K: Ord + Debug, V> SplayNode<K, V> {
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
		SplayTree::new()
	}
}

impl<K: Ord + Debug, V: Debug> SplayTree<K, V> {
	#[must_use]
	pub const fn new() -> Self {
		Self { root: None }
	}

	#[must_use]
	pub const fn is_empty(&self) -> bool {
		self.root.is_none()
	}

	pub fn contains_key<Q: ?Sized>(&mut self, key: &Q) -> bool
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if let Some(found) = self.splay(key) {
			matches!(found, Ordering::Equal)
		} else {
			false
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

		if let Some(found) = self.splay(key.borrow()) {
			match found {
				Ordering::Equal => {
					// Key exists, update value and return old value
					let old_value = std::mem::replace(&mut self.root.as_mut().unwrap().value, value);
					println!("Tree after insert: {}", self);
					Some(old_value)
				}
				Ordering::Less => {
					// New key is greater than root
					let mut new_node = Box::new(SplayNode::new(key, value));
					let root = self.root.take().unwrap();
					new_node.left = Some(root);
					self.root = Some(new_node);
					println!("Tree after insert: {}", self);
					None
				}
				Ordering::Greater => {
					// New key is less than root
					let mut new_node = Box::new(SplayNode::new(key, value));
					let mut root = self.root.take().unwrap();
					new_node.right = root.right.take();
					root.right = Some(new_node);
					self.root = Some(root);
					println!("Tree after insert: {}", self);
					None
				}
			}
		} else {
			self.root = Some(Box::new(SplayNode::new(key, value)));
			println!("Tree after insert: {}", self);
			None
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
				(None, None) => Some(root.value),
				(Some(left), None) => {
					self.root = Some(left);
					Some(root.value)
				}
				(None, Some(right)) => {
					self.root = Some(right);
					Some(root.value)
				}
				(Some(left), Some(right)) => {
					// Find the maximum key in the left subtree
					let (new_left, max_node) = self.remove_max_node(left);
					let mut new_root = max_node;
					new_root.left = new_left;
					new_root.right = Some(right);
					self.root = Some(new_root);
					Some(root.value)
				}
			}
		} else {
			None
		}
	}

	pub fn get_min(&mut self) -> Option<(&K, &V)> {
		if self.root.is_none() {
			return None;
		}

		self.splay_min();
		self.root.as_ref().map(|node| (&node.key, &node.value))
	}

	pub fn get_max(&mut self) -> Option<(&K, &V)> {
		if self.root.is_none() {
			return None;
		}

		self.splay_max();
		self.root.as_ref().map(|node| (&node.key, &node.value))
	}

	pub fn remove_min(&mut self) -> Option<(K, V)> {
		if self.root.is_none() {
			return None;
		}

		self.splay_min();
		let root = self.root.take().unwrap();
		self.root = root.right;
		Some((root.key, root.value))
	}

	pub fn remove_max(&mut self) -> Option<(K, V)> {
		if self.root.is_none() {
			return None;
		}

		self.splay_max();
		let root = self.root.take().unwrap();
		self.root = root.left;
		Some((root.key, root.value))
	}

	// Helper methods
	fn splay<Q: ?Sized>(&mut self, key: &Q) -> Option<Ordering>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if self.root.is_none() {
			return None;
		}

		let root = self.root.take().unwrap();
		let (new_root, order) = self.splay_step(*root, key);
		self.root = Some(Box::new(new_root));
		println!("Tree after splay: {}", self);
		Some(order)
	}

	fn splay_step<Q: ?Sized>(&mut self, mut node: SplayNode<K, V>, key: &Q) -> (SplayNode<K, V>, Ordering)
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		match key.cmp(node.key.borrow()) {
			Ordering::Equal => (node, Ordering::Equal),
			Ordering::Less => {
				if let Some(left) = node.left.take() {
					let (new_left, order) = self.splay_step(*left, key);
					if matches!(order, Ordering::Equal) {
						let mut new_root = new_left;
						node.left = new_root.right.take();
						new_root.right = Some(Box::new(node));
						(new_root, Ordering::Equal)
					} else {
						node.left = Some(Box::new(new_left));
						(node, Ordering::Less)
					}
				} else {
					(node, Ordering::Less)
				}
			}
			Ordering::Greater => {
				if let Some(right) = node.right.take() {
					let (new_right, order) = self.splay_step(*right, key);
					if matches!(order, Ordering::Equal) {
						let mut new_root = new_right;
						node.right = new_root.left.take();
						new_root.left = Some(Box::new(node));
						(new_root, Ordering::Equal)
					} else {
						node.right = Some(Box::new(new_right));
						(node, Ordering::Greater)
					}
				} else {
					(node, Ordering::Greater)
				}
			}
		}
	}

	fn splay_min(&mut self) {
		if let Some(mut current) = self.root.take() {
			while let Some(mut left) = current.left.take() {
				let left_right = left.right.take();
				current.left = left_right;
				left.right = Some(current);
				current = left;
			}
			self.root = Some(current);
		}
	}

	fn splay_max(&mut self) {
		// println!("current splay: {:?}", self);
		if let Some(mut current) = self.root.take() {
			while let Some(right) = current.right.take() {
				let mut new_root = right;
				current.right = new_root.left.take();
				new_root.left = Some(current);
				current = new_root;
			}
			self.root = Some(current);
		}
	}

	fn remove_max_node(&mut self, mut node: Box<SplayNode<K, V>>) -> (Option<Box<SplayNode<K, V>>>, Box<SplayNode<K, V>>) {
		if node.right.is_none() {
			(node.left.take(), node)
		} else {
			let (new_right, max_node) = self.remove_max_node(node.right.take().unwrap());
			node.right = new_right;
			(Some(node), max_node)
		}
	}
}

// Implement iterator support
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
