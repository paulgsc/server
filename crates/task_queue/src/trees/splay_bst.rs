use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt::{self, Debug, Display};

#[derive(Debug)]
pub struct Node<K, V> {
	pub key: K,
	pub value: V,
	pub left: Option<Box<Node<K, V>>>,
	pub right: Option<Box<Node<K, V>>>,
}

impl<K, V> Node<K, V> {
	pub fn new(key: K, value: V) -> Self {
		Node {
			key,
			value,
			left: None,
			right: None,
		}
	}
}

type Tree<K, V> = Option<Box<Node<K, V>>>;

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

impl<K: Ord + Debug + Clone, V> SplayTree<K, V> {
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

	pub fn insert(&mut self, key: K, value: V) {
		if self.root.is_none() {
			self.root = Some(Box::new(Node::new(key, value)));
			return;
		}

		self.root = Some(Self::insert_into(self.root.take(), key.clone(), value));
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

	fn insert_into<Q: ?Sized>(node: Option<Box<Node<K, V>>>, key: K, value: V) -> Box<Node<K, V>>
	where
		K: Borrow<Q>,
		Q: Ord + Debug,
	{
		match node {
			Some(mut n) => {
				match key.borrow().cmp(n.key.borrow()) {
					Ordering::Less => {
						n.left = Some(Self::insert_into(n.left.take(), key, value));
					}
					Ordering::Greater => {
						n.right = Some(Self::insert_into(n.right.take(), key, value));
					}
					Ordering::Equal => {
						n.value = value;
					}
				}
				n
			}
			None => Box::new(Node::new(key, value)),
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
		println!("After inserting 6:\n{}", tree);

		tree.insert(2, "two");
		println!("After inserting 2:\n{}", tree);

		tree.insert(4, "four");
		println!("After inserting 4:\n{}", tree);

		// First zig-zag operation
		tree.get(&4);
		println!("After accessing 4:\n{}", tree);

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
	}
}
