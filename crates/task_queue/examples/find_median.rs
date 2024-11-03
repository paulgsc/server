use task_queue::trees::splay_tree::SplayTree;

pub struct MedianFinder {
	lower_half: SplayTree<i32, usize>, // max heap with count as value
	upper_half: SplayTree<i32, usize>, // min heap with count as value
	lower_count: usize,
	upper_count: usize,
}

impl MedianFinder {
	pub fn new() -> Self {
		Self {
			lower_half: SplayTree::new(),
			upper_half: SplayTree::new(),
			lower_count: 0,
			upper_count: 0,
		}
	}

	pub fn add_num(&mut self, num: i32) {
		// First element goes to lower half
		if self.lower_count == 0 && self.upper_count == 0 {
			self.lower_half.insert(num, 1);
			self.lower_count = 1;
			return;
		}

		// Decide which half to put the number in
		if let Some((&max_lower, _)) = self.lower_half.get_max() {
			if num <= max_lower {
				self.lower_half.insert(num, 1);
				self.lower_count += 1;
			} else {
				self.upper_half.insert(num, 1);
				self.upper_count += 1;
			}
		} else {
			self.lower_half.insert(num, 1);
			self.lower_count += 1;
		}

		// Rebalance if necessary
		self.rebalance();
	}

	fn rebalance(&mut self) {
		while self.lower_count < self.upper_count {
			if let Some((min_val, _)) = self.upper_half.remove_min() {
				self.upper_count -= 1;
				self.lower_half.insert(min_val, 1);
				self.lower_count += 1;
			}
		}

		while self.lower_count > self.upper_count + 1 {
			if let Some((max_val, _)) = self.lower_half.remove_max() {
				self.lower_count -= 1;
				self.upper_half.insert(max_val, 1);
				self.upper_count += 1;
			}
		}
	}

	pub fn find_median(&mut self) -> f64 {
		let total = self.lower_count + self.upper_count;
		if total == 0 {
			return 0.0;
		}

		if total % 2 == 0 {
			let max_lower = self.lower_half.get_max().map(|(&k, _)| k).unwrap_or(0);
			let min_upper = self.upper_half.get_min().map(|(&k, _)| k).unwrap_or(0);
			(max_lower as f64 + min_upper as f64) / 2.0
		} else {
			self.lower_half.get_max().map(|(&k, _)| k as f64).unwrap_or(0.0)
		}
	}
}

fn main() {
	let mut mf = MedianFinder::new();
	mf.add_num(1);
	mf.add_num(2);
	println!("Median after [1,2]: {}", mf.find_median()); // Should print 1.5
	mf.add_num(3);
	println!("Median after [1,2,3]: {}", mf.find_median()); // Should print 2.0
	mf.add_num(4);
	println!("Median after [1,2,3,4]: {}", mf.find_median()); // Should print 2.5
}

#[cfg(test)]
mod tests {
	use super::*;
	use approx::assert_relative_eq;
	use rand::Rng;
	use std::time::Instant;

	#[test]
	fn test_empty() {
		let mut finder = MedianFinder::new();
		finder.add_num(1);
		assert_eq!(finder.find_median(), 1.0);
	}

	#[test]
	fn test_basic_operations() {
		let mut finder = MedianFinder::new();

		// Test single element
		finder.add_num(5);
		assert_eq!(finder.find_median(), 5.0);

		// Test two elements
		finder.add_num(3);
		assert_eq!(finder.find_median(), 4.0);

		// Test three elements
		finder.add_num(7);
		assert_eq!(finder.find_median(), 5.0);

		// Test four elements
		finder.add_num(4);
		assert_eq!(finder.find_median(), 4.5);
	}

	#[test]
	fn test_sorted_sequence() {
		let mut finder = MedianFinder::new();
		for i in 1..=5 {
			finder.add_num(i);
			match i {
				1 => assert_eq!(finder.find_median(), 1.0),
				2 => assert_eq!(finder.find_median(), 1.5),
				3 => assert_eq!(finder.find_median(), 2.0),
				4 => assert_eq!(finder.find_median(), 2.5),
				5 => assert_eq!(finder.find_median(), 3.0),
				_ => unreachable!(),
			}
		}
	}

	#[test]
	fn test_reverse_sorted_sequence() {
		let mut finder = MedianFinder::new();
		for i in (1..=5).rev() {
			finder.add_num(i);
			let median = finder.find_median();
			print!("Adding {} - Median: {}\n", i, median);
		}
		assert_eq!(finder.find_median(), 3.0);
	}

	#[test]
	fn test_duplicate_numbers() {
		let mut finder = MedianFinder::new();
		finder.add_num(1);
		finder.add_num(1);
		finder.add_num(1);
		assert_eq!(finder.find_median(), 1.0);

		finder.add_num(2);
		finder.add_num(2);
		assert_eq!(finder.find_median(), 1.0);
	}

	#[test]
	fn test_alternating_sequence() {
		let mut finder = MedianFinder::new();
		let nums = vec![1, 10, 2, 9, 3, 8, 4, 7, 5, 6];
		let expected = vec![1.0, 5.5, 2.0, 5.5, 3.0, 5.5, 4.0, 5.5, 5.0, 5.5];

		for (i, &num) in nums.iter().enumerate() {
			finder.add_num(num);
			assert_relative_eq!(finder.find_median(), expected[i], epsilon = 1e-10);
		}
	}

	#[test]
	fn test_negative_numbers() {
		let mut finder = MedianFinder::new();
		finder.add_num(-5);
		finder.add_num(-2);
		finder.add_num(-1);
		assert_eq!(finder.find_median(), -2.0);

		finder.add_num(-3);
		finder.add_num(-4);
		assert_eq!(finder.find_median(), -3.0);
	}

	#[test]
	fn test_mixed_positive_negative() {
		let mut finder = MedianFinder::new();
		finder.add_num(-1);
		finder.add_num(1);
		assert_eq!(finder.find_median(), 0.0);

		finder.add_num(0);
		assert_eq!(finder.find_median(), 0.0);
	}

	// Stress tests
	#[test]
	fn stress_test_large_sequence() {
		let mut finder = MedianFinder::new();
		let mut rng = rand::thread_rng();
		let n = 10000;

		let start = Instant::now();

		// Add numbers and verify each operation completes in reasonable time
		for _ in 0..n {
			let num = rng.gen_range(-1000..1000);
			finder.add_num(num);
			let _ = finder.find_median();
		}

		let duration = start.elapsed();
		println!("Stress test with {} numbers completed in {:?}", n, duration);

		// Verify the time complexity is roughly O(n log n) by checking if
		// average operation time is reasonable
		let avg_operation_time = duration.as_secs_f64() / (n as f64);
		assert!(avg_operation_time < 0.001); // Should be well under 1ms per operation
	}

	#[test]
	fn stress_test_sorted_sequence() {
		let mut finder = MedianFinder::new();
		let n = 10000;

		let start = Instant::now();

		// Add numbers in sorted order
		for i in 0..n {
			finder.add_num(i);
			let _ = finder.find_median();
		}

		let duration = start.elapsed();
		println!("Sorted sequence stress test with {} numbers completed in {:?}", n, duration);
	}

	#[test]
	fn stress_test_reverse_sorted_sequence() {
		let mut finder = MedianFinder::new();
		let n = 10000;

		let start = Instant::now();

		// Add numbers in reverse sorted order
		for i in (0..n).rev() {
			finder.add_num(i);
			let _ = finder.find_median();
		}

		let duration = start.elapsed();
		println!("Reverse sorted sequence stress test with {} numbers completed in {:?}", n, duration);
	}

	#[test]
	fn stress_test_repeated_numbers() {
		let mut finder = MedianFinder::new();
		let n = 10000;
		let mut rng = rand::thread_rng();

		let start = Instant::now();

		// Add many duplicate numbers
		for _ in 0..n {
			let num = rng.gen_range(0..10); // Only 10 different numbers
			finder.add_num(num);
			let _ = finder.find_median();
		}

		let duration = start.elapsed();
		println!("Repeated numbers stress test with {} numbers completed in {:?}", n, duration);
	}

	#[test]
	fn stress_test_alternating_small_large() {
		let mut finder = MedianFinder::new();
		let n = 10000;

		let start = Instant::now();

		// Add alternating small and large numbers
		for i in 0..n {
			if i % 2 == 0 {
				finder.add_num(i);
			} else {
				finder.add_num(n - i);
			}
			let _ = finder.find_median();
		}

		let duration = start.elapsed();
		println!("Alternating small/large stress test with {} numbers completed in {:?}", n, duration);
	}

	// Helper function for verifying median calculation
	fn calculate_true_median(numbers: &[i32]) -> f64 {
		let mut sorted = numbers.to_vec();
		sorted.sort_unstable();
		let len = sorted.len();
		if len % 2 == 0 {
			(sorted[len / 2 - 1] as f64 + sorted[len / 2] as f64) / 2.0
		} else {
			sorted[len / 2] as f64
		}
	}

	#[test]
	fn test_against_sorted_calculation() {
		let mut finder = MedianFinder::new();
		let mut numbers = Vec::new();
		let mut rng = rand::thread_rng();

		// Test 100 random additions
		for _ in 0..100 {
			let num = rng.gen_range(-1000..1000);
			numbers.push(num);
			finder.add_num(num);

			let expected = calculate_true_median(&numbers);
			let actual = finder.find_median();
			assert_relative_eq!(actual, expected, epsilon = 1e-10);
		}
	}
}
