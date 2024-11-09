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
		Node {
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

impl<K: Ord + Debug + Clone, V: Debug> SplayTree<K, V> {
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

	pub fn left(&self) -> Option<&Box<Node<K, V>>> {
		self.root.as_ref().and_then(|node| node.left.as_ref())
	}

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

		dbg!(&path);

		// Second phase: splay operations bottom-up
		while let Some((mut parent, went_right)) = path.pop() {
			match went_right {
				true => {
					// Current node is parent's right child
					let mut current = root.take().unwrap();
					parent.right = current.left.take();
					current.left = Some(parent);
					root = Some(current);
				}
				false => {
					// Current node is parent's left child
					let mut current = root.take().unwrap();
					parent.left = current.right.take();
					current.right = Some(parent);
					root = Some(current);
				}
			}
		}

		root
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_multiple_zig_zag_operations() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Setup for first zig-zag
		tree.insert(6, "six");

		tree.insert(2, "two");

		tree.insert(4, "four");

		// First zig-zag operation
		tree.get(&4);
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 4),
			None => panic!("Root node not found!"),
		}

		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 2),
			None => panic!("Left node not found!"),
		}

		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 6),
			None => panic!("Right node not found!"),
		}

		// Setup for second zig-zag
		tree.insert(1, "one");
		println!("After inserting 1:\n{}", tree);

		tree.insert(3, "three");
		println!("After inserting 3:\n{}", tree);

		// Second zig-zag operation
		tree.get(&3);
		println!("After accessing 3:\n{}", tree);

		if let Some(root) = &tree.root {
			assert_eq!(root.key, 3);
			if let Some(left) = &root.left {
				assert_eq!(left.key, 1);
			}
			if let Some(right) = &root.right {
				assert_eq!(right.key, 4);
			}
		}

		tree.get(&2);
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 2),
			None => panic!("Root not found!"),
		}

		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 1),
			None => panic!("Left node not found!"),
		}
		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 3),
			None => panic!("Right node not found!"),
		}

		tree.get(&6);
		match tree.root {
			Some(ref value) => assert_eq!(value.key, 6),
			None => panic!("Root not found!"),
		}

		match tree.left() {
			Some(left_node) => assert_eq!(left_node.key, 2),
			None => panic!("Left node not found!"),
		}
		assert!(tree.right().is_none(), "Expected right node to be None");
	}

	#[test]
	fn test_single_insert() {
		let mut tree = SplayTree::<i32, &str>::default();
		tree.insert(10, "ten");

		// Check that the root is the inserted node
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 10);
			assert_eq!(root.value, "ten");
		} else {
			panic!("Root not found!");
		}

		// Check that the tree is not empty
		assert_eq!(tree.size(), 1);
		assert!(!tree.is_empty());
	}

	#[test]
	fn test_empty_tree() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Check that the tree is empty
		assert!(tree.is_empty());

		// Check that calling left or right returns None
		assert!(tree.left().is_none());
		assert!(tree.right().is_none());

		// Ensure get returns None when tree is empty
		assert!(tree.get(&1).is_none(), "panicked!");
	}

	#[test]
	fn test_duplicate_insert() {
		let mut tree = SplayTree::<i32, &str>::default();
		tree.insert(10, "ten");
		tree.insert(10, "TEN"); // Insert duplicate with different value

		// Ensure that the value is updated
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 10);
			assert_eq!(root.value, "TEN"); // Value should be updated
		} else {
			panic!("Root not found!");
		}

		assert_eq!(tree.size(), 1); // Only one node should exist
	}

	#[test]
	fn test_splay_behavior() {
		let mut tree = SplayTree::<i32, &str>::default();
		tree.insert(10, "ten");
		println!("After inserting 10:\n{tree}");

		tree.insert(5, "five");
		println!("After inserting 5:\n{tree}");

		tree.insert(15, "fifteen");
		println!("After inserting 15:\n{tree}");

		// The tree should splay the node when accessed
		tree.get(&5); // This will splay the node with key 5 to the root
		println!("After accessing 5:\n{tree}");

		match tree.root {
			Some(ref value) => assert_eq!(value.key, 5),
			None => panic!("Root is None, should be 5!"),
		}

		assert!(tree.left().is_none(), "Expected left node to be None");

		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 10),
			None => panic!("Right node is None, should be 10!"),
		}
	}

	#[test]
	fn test_right_none() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Insert a few nodes to form a basic tree structure
		tree.insert(10, "ten");
		tree.insert(5, "five");
		tree.insert(15, "fifteen");
		assert!(tree.right().is_none(), "Expected right node to be None");

		// Access node with key 10, which should move it to the root
		tree.get(&10);
		match tree.right() {
			Some(right_node) => assert_eq!(right_node.key, 15),
			None => panic!("right node not found!"),
		}
	}

	#[test]
	fn test_left_and_right_nodes() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Insert nodes
		tree.insert(10, "ten");
		tree.insert(5, "five");
		tree.insert(15, "fifteen");

		// Access node with key 5, which should splay it to the root
		tree.get(&5);

		// Check that the root is 5
		if let Some(root) = &tree.root {
			assert_eq!(root.key, 5);
			assert_eq!(root.value, "five");

			// Left node should be 10
			if let Some(left) = &root.left {
				assert_eq!(left.key, 10);
			} else {
				panic!("Left node not found!");
			}

			// Right node should be 15
			if let Some(right) = &root.right {
				assert_eq!(right.key, 15);
			} else {
				panic!("Right node not found!");
			}
		} else {
			panic!("Root not found!");
		}
	}

	#[test]
	fn test_access_non_existing_node() {
		let mut tree = SplayTree::<i32, &str>::default();

		// Insert a single node
		tree.insert(10, "ten");

		// Try to access a non-existing node (should return None)
		assert!(tree.get(&100).is_none(), "Non-existing node should return None");
	}
}
