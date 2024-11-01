use std::cmp::Ordering;
use std::fmt::Debug;

pub trait PriorityQueue<T: Debug> {
	fn insert(&mut self, item: T, priority: i32);
	fn peek_max(&self) -> Option<&T>;
	fn extract_max(&mut self) -> Option<T>;
	fn is_empty(&self) -> bool;
}

#[derive(Debug, Clone, Copy)]
struct QueueNode<T> {
	item: T,
	priority: i32,
}

struct ListNode<T> {
	data: QueueNode<T>,
	next: Option<Box<ListNode<T>>>,
}

pub struct LinkedListPQ<T: Debug> {
	head: Option<Box<ListNode<T>>>,
}

impl<T: Debug> Default for LinkedListPQ<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Debug> LinkedListPQ<T> {
	#[must_use]
	pub const fn new() -> Self {
		Self { head: None }
	}
}

impl<T: Debug> PriorityQueue<T> for LinkedListPQ<T> {
	fn insert(&mut self, item: T, priority: i32) {
		let new_node = Box::new(ListNode {
			data: QueueNode { item, priority },
			next: None,
		});

		let head = &mut self.head;

		match head {
			None => {
				self.head = Some(new_node);
			}
			Some(head) if head.data.priority < priority => {
				let mut new_node = new_node;
				new_node.next = self.head.take();
				self.head = Some(new_node);
			}
			Some(head) => {
				let mut current = head;
				while current.next.is_some() && current.next.as_ref().unwrap().data.priority >= priority {
					current = current.next.as_mut().unwrap();
				}

				let mut new_node = new_node;
				new_node.next = current.next.take();
				current.next = Some(new_node);
			}
		}
	}

	fn peek_max(&self) -> Option<&T> {
		self.head.as_ref().map(|node| &node.data.item)
	}

	fn extract_max(&mut self) -> Option<T> {
		self.head.take().map(|node| {
			self.head = node.next;
			node.data.item
		})
	}

	fn is_empty(&self) -> bool {
		self.head.is_none()
	}
}

pub struct ImplicitHeap<T: Debug> {
	heap: Vec<QueueNode<T>>,
}

impl<T: Debug> Default for ImplicitHeap<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Debug> ImplicitHeap<T> {
	#[must_use]
	pub const fn new() -> Self {
		Self { heap: Vec::new() }
	}

	const fn parent(index: usize) -> usize {
		(index - 1) / 2
	}

	const fn left_child(index: usize) -> usize {
		2 * index + 1
	}

	const fn right_child(index: usize) -> usize {
		2 * index + 2
	}

	fn sift_up(&mut self, mut index: usize) {
		while index > 0 {
			let parent = Self::parent(index);
			if self.heap[parent].priority >= self.heap[index].priority {
				break;
			}
			self.heap.swap(parent, index);
			index = parent;
		}
	}

	fn sift_down(&mut self, mut index: usize) {
		loop {
			let left = Self::left_child(index);
			let right = Self::right_child(index);
			let mut largest = index;

			if left < self.heap.len() && self.heap[left].priority > self.heap[largest].priority {
				largest = left;
			}
			if right < self.heap.len() && self.heap[right].priority > self.heap[largest].priority {
				largest = right;
			}

			if largest == index {
				break;
			}

			self.heap.swap(index, largest);
			index = largest;
		}
	}
}

impl<T: Debug> PriorityQueue<T> for ImplicitHeap<T> {
	fn insert(&mut self, item: T, priority: i32) {
		let node = QueueNode { item, priority };
		self.heap.push(node);
		let new_index = self.heap.len() - 1;
		self.sift_up(new_index);
	}

	fn peek_max(&self) -> Option<&T> {
		self.heap.first().map(|node| &node.item)
	}

	fn extract_max(&mut self) -> Option<T> {
		if self.heap.is_empty() {
			return None;
		}
		let last_idx = self.heap.len() - 1;
		self.heap.swap(0, last_idx);
		let node = self.heap.pop().unwrap();
		if !self.heap.is_empty() {
			self.sift_down(0);
		}
		Some(node.item)
	}

	fn is_empty(&self) -> bool {
		self.heap.is_empty()
	}
}

// 3. Two-List Implementation
pub struct TwoListPQ<T: Debug> {
	small: Vec<QueueNode<T>>,
	large: Vec<QueueNode<T>>,
	threshold: usize,
}

impl<T: Debug> TwoListPQ<T> {
	#[must_use]
	pub fn new(threshold: usize) -> Self {
		Self {
			small: Vec::new(),
			large: Vec::new(),
			threshold,
		}
	}

	fn rebalance(&mut self) {
		if self.large.len() > self.threshold {
			self.large.sort_by_key(|node| -node.priority);
			while self.large.len() > self.threshold / 2 {
				if let Some(node) = self.large.pop() {
					self.small.push(node);
				}
			}
			self.small.sort_by_key(|node| -node.priority);
		}
	}
}

impl<T: Debug> PriorityQueue<T> for TwoListPQ<T> {
	fn insert(&mut self, item: T, priority: i32) {
		let node = QueueNode { item, priority };
		self.large.push(node);
		self.rebalance();
	}

	fn peek_max(&self) -> Option<&T> {
		self
			.small
			.first()
			.map(|node| &node.item)
			.or_else(|| self.large.iter().max_by_key(|node| node.priority).map(|node| &node.item))
	}

	fn extract_max(&mut self) -> Option<T> {
		if self.small.is_empty() {
			let max_idx = self.large.iter().enumerate().max_by_key(|(_, node)| node.priority).map(|(i, _)| i);
			max_idx.map(|i| self.large.remove(i).item)
		} else {
			Some(self.small.remove(0).item)
		}
	}

	fn is_empty(&self) -> bool {
		self.small.is_empty() && self.large.is_empty()
	}
}

// 4. Henriksen's Splay Tree Implementation
pub struct SplayTree<T: Debug> {
	root: Option<Box<SplayNode<T>>>,
}

struct SplayNode<T> {
	item: T,
	priority: i32,
	left: Option<Box<SplayNode<T>>>,
	right: Option<Box<SplayNode<T>>>,
}

impl<T: Debug> Default for SplayTree<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Debug> SplayTree<T> {
	#[must_use]
	pub const fn new() -> Self {
		Self { root: None }
	}

	fn splay(&mut self, priority: i32) {
		if self.root.is_none() {
			return;
		}

		let root = self.root.take().unwrap();
		let (new_root, _found) = Self::splay_step(*root, priority);
		self.root = Some(Box::new(new_root));
	}

	fn splay_step(mut node: SplayNode<T>, priority: i32) -> (SplayNode<T>, bool) {
		match node.priority.cmp(&priority) {
			Ordering::Equal => (node, true),
			Ordering::Less => {
				if let Some(right) = node.right {
					let (new_right, found) = Self::splay_step(*right, priority);
					node.right = Some(Box::new(new_right));
					if found {
						let mut new_root = *node.right.take().unwrap();
						node.right = new_root.left.take();
						new_root.left = Some(Box::new(node));
						(new_root, true)
					} else {
						(node, false)
					}
				} else {
					(node, false)
				}
			}
			Ordering::Greater => {
				if let Some(left) = node.left {
					let (new_left, found) = Self::splay_step(*left, priority);
					node.left = Some(Box::new(new_left));
					if found {
						let mut new_root = *node.left.take().unwrap();
						node.left = new_root.right.take();
						new_root.right = Some(Box::new(node));
						(new_root, true)
					} else {
						(node, false)
					}
				} else {
					(node, false)
				}
			}
		}
	}
}

impl<T: Debug> PriorityQueue<T> for SplayTree<T> {
	fn insert(&mut self, item: T, priority: i32) {
		let new_node = SplayNode {
			item,
			priority,
			left: None,
			right: None,
		};

		if self.root.is_none() {
			self.root = Some(Box::new(new_node));
			return;
		}

		self.splay(priority);
		let mut current = self.root.take().unwrap();

		if priority > current.priority {
			let mut new_root = Box::new(new_node);
			new_root.left = Some(current);
			self.root = Some(new_root);
		} else {
			let mut new_node = Box::new(new_node);
			new_node.right = current.right.take();
			current.right = Some(new_node);
			self.root = Some(current);
		}
	}

	fn peek_max(&self) -> Option<&T> {
		self.root.as_ref().map(|node| &node.item)
	}

	fn extract_max(&mut self) -> Option<T> {
		if let Some(root) = self.root.take() {
			let result = root.item;
			if let Some(left) = root.left {
				self.root = Some(left);
			}
			Some(result)
		} else {
			None
		}
	}

	fn is_empty(&self) -> bool {
		self.root.is_none()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Helper function to test basic operations on any PQ implementation
	fn test_basic_operations<T>(mut pq: T)
	where
		T: PriorityQueue<i32>,
	{
		assert!(pq.is_empty());
		assert_eq!(pq.peek_max(), None);

		// Test single insertion
		pq.insert(42, 42);
		assert!(!pq.is_empty());
		assert_eq!(pq.peek_max(), Some(&42));

		// Test extraction
		assert_eq!(pq.extract_max(), Some(42));
		assert!(pq.is_empty());
	}

	// Helper function to test ordering
	fn test_ordering<T>(mut pq: T)
	where
		T: PriorityQueue<i32>,
	{
		// Insert items in arbitrary order
		let inputs = vec![(5, 5), (3, 3), (7, 7), (1, 1), (6, 6), (4, 4), (2, 2)];
		for (item, priority) in inputs {
			pq.insert(item, priority);
		}

		// Should extract in descending order
		for expected in (1..=7).rev() {
			assert_eq!(pq.extract_max(), Some(expected));
		}
		assert!(pq.is_empty());
	}

	// Helper function to test duplicate priorities
	fn test_duplicates<T>(mut pq: T)
	where
		T: PriorityQueue<i32>,
	{
		pq.insert(1, 5);
		pq.insert(2, 5);
		pq.insert(3, 5);

		// All items with priority 5 should be extractable
		assert!(pq.extract_max().is_some());
		assert!(pq.extract_max().is_some());
		assert!(pq.extract_max().is_some());
		assert!(pq.is_empty());
	}

	// Helper function to test mixed operations
	fn test_mixed_operations<T>(mut pq: T)
	where
		T: PriorityQueue<i32>,
	{
		pq.insert(1, 1);
		pq.insert(3, 3);
		assert_eq!(pq.peek_max(), Some(&3));

		pq.insert(2, 2);
		assert_eq!(pq.extract_max(), Some(3));

		pq.insert(4, 4);
		assert_eq!(pq.peek_max(), Some(&4));

		assert_eq!(pq.extract_max(), Some(4));
		assert_eq!(pq.extract_max(), Some(2));
		assert_eq!(pq.extract_max(), Some(1));
		assert!(pq.is_empty());
	}

	// Tests for LinkedListPQ
	#[test]
	fn test_linked_list_pq() {
		let pq = LinkedListPQ::new();
		test_basic_operations(pq);

		let pq = LinkedListPQ::new();
		test_ordering(pq);

		let pq = LinkedListPQ::new();
		test_duplicates(pq);

		let pq = LinkedListPQ::new();
		test_mixed_operations(pq);
	}

	// Tests for ImplicitHeap
	#[test]
	fn test_implicit_heap() {
		let pq = ImplicitHeap::new();
		test_basic_operations(pq);

		let pq = ImplicitHeap::new();
		test_ordering(pq);

		let pq = ImplicitHeap::new();
		test_duplicates(pq);

		let pq = ImplicitHeap::new();
		test_mixed_operations(pq);
	}

	// Tests for TwoListPQ
	#[test]
	fn test_two_list_pq() {
		let pq = TwoListPQ::new(4); // Using small threshold for testing
		test_basic_operations(pq);

		let pq = TwoListPQ::new(4);
		test_ordering(pq);

		let pq = TwoListPQ::new(4);
		test_duplicates(pq);

		let pq = TwoListPQ::new(4);
		test_mixed_operations(pq);
	}

	// Additional test specific to TwoListPQ
	#[test]
	fn test_two_list_rebalancing() {
		let mut pq = TwoListPQ::new(4);

		// Insert enough items to trigger rebalancing
		for i in 0..6 {
			pq.insert(i, i);
		}

		// Verify all items can still be extracted in order
		for i in (0..6).rev() {
			assert_eq!(pq.extract_max(), Some(i));
		}
	}

	// Tests for SplayTree
	#[test]
	fn test_splay_tree() {
		let pq = SplayTree::new();
		test_basic_operations(pq);

		let pq = SplayTree::new();
		test_ordering(pq);

		let pq = SplayTree::new();
		test_duplicates(pq);

		let pq = SplayTree::new();
		test_mixed_operations(pq);
	}

	// Additional test specific to SplayTree
	#[test]
	fn test_splay_operations() {
		let mut pq = SplayTree::new();

		// Insert some items
		pq.insert(3, 3);
		pq.insert(1, 1);
		pq.insert(4, 4);
		pq.insert(2, 2);

		// Splaying to 4 should make it the root
		pq.splay(4);
		assert_eq!(pq.peek_max(), Some(&4));

		// Splaying to 1 should reorganize the tree
		pq.splay(1);
		assert_eq!(pq.extract_max(), Some(4));
	}

	// Property-based tests (if quickcheck is available)
	#[test]
	fn test_pq_properties() {
		fn is_sorted_descending(vec: &[i32]) -> bool {
			vec.windows(2).all(|w| w[0] >= w[1])
		}

		fn test_random_sequence<T>(mut pq: T, items: Vec<(i32, i32)>)
		where
			T: PriorityQueue<i32>,
		{
			// Insert all items
			for (item, priority) in items {
				pq.insert(item, priority);
			}

			// Extract all items and verify they're in descending order
			let mut extracted = Vec::new();
			while let Some(item) = pq.extract_max() {
				extracted.push(item);
			}

			assert!(is_sorted_descending(&extracted));
		}

		// Test with some random sequences
		let sequences = vec![vec![(1, 1), (2, 2), (3, 3)], vec![(3, 3), (2, 2), (1, 1)], vec![(2, 2), (3, 3), (1, 1)]];

		for seq in sequences {
			test_random_sequence(LinkedListPQ::new(), seq.clone());
			test_random_sequence(ImplicitHeap::new(), seq.clone());
			test_random_sequence(TwoListPQ::new(4), seq.clone());
			test_random_sequence(SplayTree::new(), seq.clone());
		}
	}
}
