use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt::{self, Debug, Display};

type Tree<K, V> = Option<Box<Node<K, V>>>;

#[derive(Debug)]
pub struct Node<K, V> {
	pub key: K,
	pub value: V,
	pub left: Tree<K, V>,
	pub right: Tree<K, V>,
}

impl<K: Debug, V: Debug> Node<K, V> {
	pub const fn new(key: K, value: V) -> Self {
		Self {
			key,
			value,
			left: None,
			right: None,
		}
	}
}

impl<K: Display, V: Display> Display for Node<K, V> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Node({}, {})", self.key, self.value)
	}
}

#[derive(Debug, Default)]
pub struct SplayTree<K, V> {
	pub root: Option<Box<Node<K, V>>>,
	pub size: usize,
}

impl<K: Ord + Debug + Clone + Display, V: Clone + Display> Display for SplayTree<K, V> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		fn print_tree<K: Display, V: Display>(node: &Tree<K, V>, prefix: &str, is_left: bool, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			if let Some(node) = node {
				writeln!(f, "{}{}({}, {})", prefix, if is_left { "└──" } else { "┌──" }, node.key, node.value)?;
				let new_prefix = format!("{}{}", prefix, if is_left { "    " } else { "│   " });
				print_tree(&node.right, &new_prefix, false, f)?;
				print_tree(&node.left, &new_prefix, true, f)?;
			}
			Ok(())
		}

		writeln!(f, "SplayTree (size: {}):", self.size)?;
		print_tree(&self.root, "", true, f)
	}
}

impl<K: Ord + Debug + Clone, V: Debug + Clone> SplayTree<K, V> {
	#[must_use]
	pub const fn new() -> Self {
		Self { root: None, size: 0 }
	}

	#[must_use]
	pub const fn size(&self) -> usize {
		self.size
	}

	#[must_use]
	pub const fn is_empty(&self) -> bool {
		self.size == 0
	}

	#[must_use]
	pub fn left(&self) -> Option<&Box<Node<K, V>>> {
		self.root.as_ref().and_then(|node| node.left.as_ref())
	}

	#[must_use]
	pub fn right(&self) -> Option<&Box<Node<K, V>>> {
		self.root.as_ref().and_then(|node| node.right.as_ref())
	}

	pub fn insert(&mut self, key: K, value: V) {
		if self.root.is_none() {
			self.root = Some(Box::new(Node::new(key, value)));
			self.size += 1;
			return;
		}

		self.root = Some(Self::insert_recursive(self.root.take(), &key, value));
		self.size += 1;
		match &self.root {
			Some(node) => println!("{:?}", node),
			None => println!("None"),
		}
		self.root = Self::splay(self.root.take(), &key);
	}

	pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		// First splay the node to remove (or its closest parent) to the root
		self.root = Self::splay(self.root.take(), key);

		match self.root.take() {
			Some(mut root) => {
				if root.key.borrow().cmp(key) != Ordering::Equal {
					// Key not found, put the root back and return None
					self.root = Some(root);
					return None;
				}

				// Key found, need to remove root
				let value = root.value.clone();

				match (root.left.take(), root.right.take()) {
					(None, None) => {
						// No children, tree is now empty
						self.root = None;
					}
					(Some(left), None) => {
						// Only left child
						self.root = Some(left);
					}
					(None, Some(right)) => {
						// Only right child
						self.root = Some(right);
					}
					(Some(left), Some(right)) => {
						// Both children exist
						// 1. Set root to left subtree
						self.root = Some(left);
						// 2. Find the maximum element in the left subtree
						self.root = Self::splay(self.root.take(), root.key.borrow());
						// 3. Make the right subtree the right child of the new root
						self.root.as_mut().unwrap().right = Some(right);
					}
				}

				self.size -= 1;
				Some(value)
			}
			None => None,
		}
	}

	pub fn get<Q: ?Sized>(&mut self, key: &Q) -> Option<&V>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		self.root = Self::splay(self.root.take(), key);
		match self.root.as_ref() {
			Some(node) if node.key.borrow().cmp(key) == Ordering::Equal => Some(&node.value),
			_ => None,
		}
	}

	pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		self.root = Self::splay(self.root.take(), key);
		match self.root.as_mut() {
			Some(node) if node.key.borrow().cmp(key) == Ordering::Equal => Some(&mut node.value),
			_ => None,
		}
	}

	fn insert_recursive<Q: ?Sized>(node: Option<Box<Node<K, V>>>, key: &Q, value: V) -> Box<Node<K, V>>
	where
		K: Borrow<Q> + From<Q::Owned>,
		Q: Ord + Debug + ToOwned,
	{
		match node {
			Some(mut n) => {
				match key.cmp(n.key.borrow()) {
					Ordering::Less => {
						n.left = Some(Self::insert_recursive(n.left.take(), key, value));
					}
					Ordering::Greater => {
						n.right = Some(Self::insert_recursive(n.right.take(), key, value));
					}
					Ordering::Equal => {
						n.value = value;
					}
				}
				n
			}
			None => Box::new(Node::new(K::from(key.to_owned()), value)),
		}
	}

	fn splay<Q: ?Sized>(mut root: Option<Box<Node<K, V>>>, key: &Q) -> Option<Box<Node<K, V>>>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		if root.is_none() {
			return None;
		}

		let mut path = Vec::new();
		// First phase: find the node and build the path
		loop {
			let current_key = root.as_ref().unwrap().key.borrow();
			match key.cmp(current_key) {
				Ordering::Equal => {
					break;
				}
				Ordering::Less => {
					if let Some(mut node) = root {
						if node.left.is_none() {
							root = Some(node);
							break;
						}
						let next = node.left.take();
						path.push((node, false)); // false means we went left
						root = next;
					} else {
						break;
					}
				}
				Ordering::Greater => {
					if let Some(mut node) = root {
						if node.right.is_none() {
							root = Some(node);
							break;
						}
						let next = node.right.take();
						path.push((node, true)); // true means we went right
						root = next;
					} else {
						break;
					}
				}
			}
		}

		// Second phase: splay operations bottom-up
		while !path.is_empty() {
			if path.len() >= 2 {
				// We have a grandparent - do a double rotation
				let (mut parent, went_right_to_parent) = path.pop().unwrap();
				let (mut grandparent, went_right_to_current) = path.pop().unwrap();
				let mut current = root.take().unwrap();

				match (went_right_to_parent, went_right_to_current) {
					(true, true) => {
						// Zig-zig case (right-right)
						parent.right = current.left.take();
						grandparent.right = parent.left.take();
						parent.left = Some(grandparent);
						current.left = Some(parent);
					}
					(false, false) => {
						// Zig-zig case (left-left)
						parent.left = current.right.take();
						grandparent.left = parent.right.take();
						parent.right = Some(grandparent);
						current.right = Some(parent);
					}
					(false, true) => {
						// Zig-zag case (right-left)
						parent.left = current.right.take();
						grandparent.right = current.left.take();
						current.left = Some(grandparent);
						current.right = Some(parent);
					}
					(true, false) => {
						// Zig-zag case (left-right)
						parent.right = current.left.take();
						grandparent.left = current.right.take();
						current.right = Some(grandparent);
						current.left = Some(parent);
					}
				}
				root = Some(current);
			} else {
				// Single rotation when we only have a parent
				let (mut parent, went_right) = path.pop().unwrap();
				let mut current = root.take().unwrap();

				match went_right {
					true => {
						// Current node is parent's right child
						parent.right = current.left.take();
						current.left = Some(parent);
					}
					false => {
						// Current node is parent's left child
						parent.left = current.right.take();
						current.right = Some(parent);
					}
				}
				root = Some(current);
			}
		}
		root
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Helper function to verify basic Binary Search Tree properties
	fn verify_bst_properties<K: Ord + Debug + Clone, V: Debug>(node: &Option<Box<Node<K, V>>>, min: Option<&K>, max: Option<&K>) -> bool {
		match node {
			None => true,
			Some(n) => {
				// Check current node's value against bounds
				if let Some(min_val) = min {
					if n.key <= *min_val {
						return false;
					}
				}
				if let Some(max_val) = max {
					if n.key >= *max_val {
						return false;
					}
				}

				// Recursively check children
				verify_bst_properties(&n.left, min, Some(&n.key)) && verify_bst_properties(&n.right, Some(&n.key), max)
			}
		}
	}

	#[test]
	fn test_basic_operations() {
		let mut tree = SplayTree::new();
		assert!(tree.is_empty());
		assert_eq!(tree.size(), 0);

		// Test single insertion
		tree.insert(5, "five");
		assert_eq!(tree.size(), 1);
		assert!(!tree.is_empty());
		assert_eq!(tree.get(&5), Some(&"five"));

		// Test multiple insertions
		tree.insert(3, "three");
		tree.insert(7, "seven");
		assert_eq!(tree.size(), 3);
		assert_eq!(tree.get(&3), Some(&"three"));
		assert_eq!(tree.get(&7), Some(&"seven"));

		// Test updating existing key
		tree.insert(5, "new_five");
		assert_eq!(tree.size(), 3);
		assert_eq!(tree.get(&5), Some(&"new_five"));
	}

	#[test]
	fn test_splay_operations() {
		let mut tree = SplayTree::new();

		// Insert elements to create a specific tree structure
		tree.insert(5, "five");
		tree.insert(3, "three");
		tree.insert(7, "seven");
		tree.insert(2, "two");
		tree.insert(4, "four");
		tree.insert(6, "six");
		tree.insert(8, "eight");

		// Test zig-zig (left-left) case
		tree.get(&2);
		assert_eq!(tree.root.as_ref().unwrap().key, 2);

		// Test zig-zag (right-left) case
		tree.insert(6, "six");
		tree.get(&4);
		assert_eq!(tree.root.as_ref().unwrap().key, 4);

		// Verify BST properties are maintained after splay operations
		assert!(verify_bst_properties(&tree.root, None, None));
	}

	#[test]
	fn test_edge_cases() {
		let mut tree = SplayTree::new();

		// Test getting from empty tree
		assert_eq!(tree.get(&1), None);

		// Test single node operations
		tree.insert(1, "one");
		assert_eq!(tree.get(&1), Some(&"one"));
		assert_eq!(tree.size(), 1);

		// Test getting non-existent keys
		assert_eq!(tree.get(&2), None);
		assert_eq!(tree.get(&0), None);

		// Test multiple insertions and gets with same key
		tree.insert(1, "one_new");
		assert_eq!(tree.get(&1), Some(&"one_new"));
		assert_eq!(tree.size(), 1);
	}

	#[test]
	fn test_large_tree() {
		let mut tree = SplayTree::new();
		let values: Vec<i32> = (1..100).collect();

		// Insert values in a way that could create an unbalanced tree
		for &value in &values {
			tree.insert(value, value.to_string());
		}

		// Verify all values can be retrieved
		for &value in &values {
			assert_eq!(tree.get(&value), Some(&value.to_string()));
		}

		// Verify BST properties are maintained
		assert!(verify_bst_properties(&tree.root, None, None));
	}

	#[test]
	fn test_complex_splay_sequences() {
		let mut tree = SplayTree::new();

		// Create a tree with multiple levels
		let values = vec![50, 25, 75, 12, 37, 62, 87];
		for &value in &values {
			tree.insert(value, value.to_string());
		}

		// Test sequence of operations that trigger different splay cases
		tree.get(&12); // Should trigger zig-zig
		assert_eq!(tree.root.as_ref().unwrap().key, 12);

		tree.get(&62); // Should trigger zig-zag
		assert_eq!(tree.root.as_ref().unwrap().key, 62);

		tree.get(&87); // Should trigger zig
		assert_eq!(tree.root.as_ref().unwrap().key, 87);

		// Verify tree maintains BST properties after complex operations
		assert!(verify_bst_properties(&tree.root, None, None));
	}

	#[test]
	fn test_get_mut() {
		let mut tree = SplayTree::new();

		// Insert some initial values
		tree.insert(1, String::from("one"));
		tree.insert(2, String::from("two"));
		tree.insert(3, String::from("three"));

		// Modify value using get_mut
		if let Some(value) = tree.get_mut(&2) {
			*value = String::from("TWO");
		}

		// Verify the modification
		assert_eq!(tree.get(&2), Some(&String::from("TWO")));

		// Attempt to modify non-existent key
		assert_eq!(tree.get_mut(&4), None);
	}

	#[test]
	fn test_tree_structure() {
		let mut tree = SplayTree::new();

		// Insert values in a specific order to test tree structure
		let values = vec![3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5];
		for &value in &values {
			tree.insert(value, value.to_string());
		}

		// After each get operation, verify:
		// 1. The accessed node is at the root
		// 2. BST properties are maintained
		// 3. All values are still accessible

		tree.get(&5);
		assert_eq!(tree.root.as_ref().unwrap().key, 5);
		assert!(verify_bst_properties(&tree.root, None, None));

		tree.get(&1);
		assert_eq!(tree.root.as_ref().unwrap().key, 1);
		assert!(verify_bst_properties(&tree.root, None, None));

		tree.get(&9);
		assert_eq!(tree.root.as_ref().unwrap().key, 9);
		assert!(verify_bst_properties(&tree.root, None, None));
	}
	#[test]
	fn test_splay_behavior() {
		let mut tree = SplayTree::<i32, &str>::default();
		// Frist testing zig-zig-left (right right case)
		tree.insert(10, "ten");

		tree.insert(5, "five");

		tree.insert(15, "fifteen");

		// Second Test zig-zig-right
		tree.get(&5); // This will splay the node with key 5 to the root

		match tree.root {
			Some(ref value) => assert_eq!(value.key, 5),
			None => panic!("Root is None, should be 5!"),
		}

		assert!(tree.left().is_none(), "Expected left node to be None");

		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 10),
			None => panic!("Right node is None, should be 10!"),
		}

		// Third Test  zig-zag-left
		tree.insert(9, "nine");

		match tree.root {
			Some(ref value) => assert_eq!(value.key, 9),
			None => panic!("Root is None, should be 9!"),
		}

		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 5),
			None => panic!("Right node is None, should be 5!"),
		}

		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 10),
			None => panic!("Right node is None, should be 10!"),
		}

		// Fourth Test zig-zag-right
		tree.insert(6, "six");
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 6),
			None => panic!("Root is None, should be 6!"),
		}

		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 5),
			None => panic!("Right node is None, should be 5!"),
		}

		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 9),
			None => panic!("Right node is None, should be 9!"),
		}

		// Fifth Test zig-left
		tree.get(&9);
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 9),
			None => panic!("Root is None, should be 9!"),
		}

		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 6),
			None => panic!("Right node is None, should be 6!"),
		}

		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 10),
			None => panic!("Right node is None, should be 10!"),
		}

		// Sixth Test zig-right
		tree.get(&6);
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 6),
			None => panic!("Root is None, should be 6!"),
		}

		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 5),
			None => panic!("Right node is None, should be 5!"),
		}

		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 9),
			None => panic!("Right node is None, should be 9!"),
		}
	}
	#[test]
	fn test_remove_basic() {
		let mut tree = SplayTree::new();

		// Test removing from empty tree
		assert_eq!(tree.remove(&1), None);

		// Test removing single element
		tree.insert(1, "one");
		assert_eq!(tree.remove(&1), Some("one"));
		assert!(tree.is_empty());
		assert_eq!(tree.size(), 0);

		// Test removing non-existent element
		tree.insert(1, "one");
		assert_eq!(tree.remove(&2), None);
		assert_eq!(tree.size(), 1);
	}

	#[test]
	fn test_remove_complex() {
		let mut tree = SplayTree::new();

		// Insert several elements
		let values = vec![5, 3, 7, 2, 4, 6, 8];
		for &value in &values {
			tree.insert(value, value.to_string());
		}

		// Test removing leaf node
		assert_eq!(tree.remove(&2), Some("2".to_string()));
		assert_eq!(tree.size(), 6);
		assert!(verify_bst_properties(&tree.root, None, None));

		// Test removing node with one child
		assert_eq!(tree.remove(&3), Some("3".to_string()));
		assert_eq!(tree.size(), 5);
		assert!(verify_bst_properties(&tree.root, None, None));

		// Test removing node with two children
		assert_eq!(tree.remove(&7), Some("7".to_string()));
		assert_eq!(tree.size(), 4);
		assert!(verify_bst_properties(&tree.root, None, None));

		// Verify remaining structure
		assert_eq!(tree.get(&4), Some(&"4".to_string()));
		assert_eq!(tree.get(&5), Some(&"5".to_string()));
		assert_eq!(tree.get(&6), Some(&"6".to_string()));
		assert_eq!(tree.get(&8), Some(&"8".to_string()));
	}

	#[test]
	fn test_remove_root() {
		let mut tree = SplayTree::new();

		// Test removing root when it's the only node
		tree.insert(1, "one");
		assert_eq!(tree.remove(&1), Some("one"));
		assert!(tree.is_empty());

		// Test removing root with one child
		tree.insert(2, "two");
		tree.insert(1, "one");
		assert_eq!(tree.remove(&2), Some("two"));
		assert_eq!(tree.root.as_ref().unwrap().key, 1);

		// Test removing root with two children
		tree.insert(2, "two");
		tree.insert(3, "three");
		assert_eq!(tree.remove(&2), Some("two"));
		assert!(verify_bst_properties(&tree.root, None, None));
	}

	#[test]
	fn test_remove_sequence() {
		let mut tree = SplayTree::new();

		// Insert values in sequence
		for i in 1..=10 {
			tree.insert(i, i.to_string());
		}

		// Remove values in different order
		let remove_sequence = vec![5, 3, 7, 1, 9, 2, 8, 4, 6, 10];
		for &value in &remove_sequence {
			assert_eq!(tree.remove(&value), Some(value.to_string()));
			if !tree.is_empty() {
				assert!(verify_bst_properties(&tree.root, None, None));
			}
		}

		assert!(tree.is_empty());
	}

	#[test]
	fn test_remove_rebalancing() {
		let mut tree = SplayTree::new();

		// Create a specific tree structure
		let values = vec![50, 25, 75, 12, 37, 62, 87];
		for &value in &values {
			tree.insert(value, value.to_string());
		}

		// Remove nodes and verify splaying occurs correctly
		assert_eq!(tree.remove(&25), Some("25".to_string()));
		assert!(verify_bst_properties(&tree.root, None, None));

		// Verify the structure after removal
		assert_eq!(tree.get(&37), Some(&"37".to_string()));
		assert_eq!(tree.root.as_ref().unwrap().key, 37);

		// Remove more nodes and verify structure
		assert_eq!(tree.remove(&75), Some("75".to_string()));
		assert!(verify_bst_properties(&tree.root, None, None));
		assert_eq!(tree.remove(&50), Some("50".to_string()));
		assert!(verify_bst_properties(&tree.root, None, None));
	}

	#[test]
	fn test_remove_stress() {
		let mut tree = SplayTree::new();
		let mut values: Vec<i32> = (1..100).collect();

		// Insert all values
		for &value in &values {
			tree.insert(value, value.to_string());
		}

		// Remove random values
		use rand::seq::SliceRandom;
		let mut rng = rand::thread_rng();
		values.shuffle(&mut rng);

		for &value in &values {
			assert_eq!(tree.remove(&value), Some(value.to_string()));
			if !tree.is_empty() {
				assert!(verify_bst_properties(&tree.root, None, None));
			}
		}

		assert!(tree.is_empty());
	}

	#[test]
	fn test_remove_splay_behavior() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Initial tree setup
		tree.insert(10, "ten");
		tree.insert(5, "five");
		tree.insert(15, "fifteen");
		tree.insert(3, "three");
		tree.insert(7, "seven");
		tree.insert(12, "twelve");
		tree.insert(17, "seventeen");

		// Test Case 1: Remove leaf node (zig-zig case)
		tree.remove(&3);
		// After removing 3, 5 should be at root due to splaying
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 15),
			None => panic!("Root is None, should be 15!"),
		}
		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 17),
			None => panic!("Right node is None, should be 17!"),
		}

		// Test Case 2: Remove node with one child (zig-zag case)
		tree.remove(&15);
		// After removing 15, 12 should be splayed to root
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 12),
			None => panic!("Root is None, should be 12!"),
		}
		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 17),
			None => panic!("Right node is None, should be 17!"),
		}

		// Test Case 3: Remove node with two children (complex case)
		tree.remove(&10);
		// After removing 10, the largest element in left subtree should be splayed to root
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 7),
			None => panic!("Root is None, should be 7!"),
		}
		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 5),
			None => panic!("Left node is None, should be 5!"),
		}
		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 12),
			None => panic!("Right node is None, should be 12!"),
		}

		// Test Case 4: Remove root node
		tree.remove(&7);
		// After removing root, the largest element in left subtree should become new root
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 5),
			None => panic!("Root is None, should be 5!"),
		}
		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 12),
			None => panic!("Right node is None, should be 12!"),
		}

		// Test Case 5: Remove last element in a branch
		tree.remove(&17);
		// After removing 17, 12 should be splayed to root
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 12),
			None => panic!("Root is None, should be 12!"),
		}
		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 5),
			None => panic!("Left node is None, should be 5!"),
		}
		assert!(tree.right().is_none(), "Expected right node to be None after removing 17");

		// Test Case 6: Remove non-existent key
		tree.remove(&20);
		// Tree structure should remain unchanged after failed removal
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 12),
			None => panic!("Root is None, should be 12!"),
		}
		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 5),
			None => panic!("Left node is None, should be 5!"),
		}

		// Test Case 7: Remove remaining nodes in specific order
		tree.remove(&5);
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 12),
			None => panic!("Root is None, should be 12!"),
		}
		assert!(tree.left().is_none(), "Expected left node to be None after removing 5");
		assert!(tree.right().is_none(), "Expected right node to be None after removing 5");

		// Remove final node
		tree.remove(&12);
		assert!(tree.root.is_none(), "Expected empty tree after removing all nodes");
		assert_eq!(tree.size(), 0, "Tree size should be 0 after removing all nodes");
	}
}
