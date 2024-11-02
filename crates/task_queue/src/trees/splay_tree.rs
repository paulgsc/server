use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};

pub struct SplayTree<K: Ord + Debug, V> {
	root: Option<Box<SplayNode<K, V>>>,
}

impl<K: Ord + Debug, V: Debug> Display for SplayTree<K, V> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		if let Some(root) = &self.root {
			write!(f, "{}", root)
		} else {
			write!(f, "Empty Tree")
		}
	}
}

struct SplayNode<K, V> {
	key: K,
	value: V,
	left: Option<Box<SplayNode<K, V>>>,
	right: Option<Box<SplayNode<K, V>>>,
}

impl<K: Ord + Debug, V> SplayNode<K, V> {
	fn new(key: K, value: V) -> Self {
		SplayNode {
			key,
			value,
			left: None,
			right: None,
		}
	}
}

impl<K: Debug, V: Debug> Display for SplayNode<K, V> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "Node(Key: {:?}, Value: {:?})", self.key, self.value)?;

		if self.left.is_some() || self.right.is_some() {
			write!(f, " [")?;
			if let Some(left) = &self.left {
				write!(f, " Left: {}", left)?;
			}
			if let Some(right) = &self.right {
				write!(f, " Right: {}", right)?;
			}
			write!(f, "]")?;
		}

		Ok(())
	}
}

impl<K: Ord + Debug, V> SplayTree<K, V> {
	pub fn new() -> Self {
		Self { root: None }
	}

	pub fn is_empty(&self) -> bool {
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
					Some(old_value)
				}
				Ordering::Less => {
					// New key is greater than root
					let mut new_node = Box::new(SplayNode::new(key, value));
					let root = self.root.take().unwrap();
					new_node.left = Some(root);
					self.root = Some(new_node);
					None
				}
				Ordering::Greater => {
					// New key is less than root
					let mut new_node = Box::new(SplayNode::new(key, value));
					let mut root = self.root.take().unwrap();
					new_node.right = root.right.take();
					root.right = Some(new_node);
					self.root = Some(root);
					None
				}
			}
		} else {
			self.root = Some(Box::new(SplayNode::new(key, value)));
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
			while let Some(left) = current.left.take() {
				let mut new_root = left;
				current.left = new_root.right.take();
				new_root.right = Some(current);
				current = new_root;
			}
			self.root = Some(current);
		}
	}

	fn splay_max(&mut self) {
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
impl<K: Ord + Debug, V> IntoIterator for SplayTree<K, V> {
	type Item = (K, V);
	type IntoIter = IntoIter<K, V>;

	fn into_iter(self) -> Self::IntoIter {
		IntoIter { tree: self }
	}
}

pub struct IntoIter<K: Ord + Debug, V> {
	tree: SplayTree<K, V>,
}

impl<K: Ord + Debug, V> Iterator for IntoIter<K, V> {
	type Item = (K, V);

	fn next(&mut self) -> Option<Self::Item> {
		self.tree.remove_min()
	}
}
