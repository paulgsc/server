pub mod retry;

use std::process;
use std::time::{SystemTime, UNIX_EPOCH}; // For writing into buffer

pub fn generate_uuid() -> [u8; 32] {
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
	let pid = process::id();

	let mut buf = [0u8; 32]; // Fixed-size buffer
	let mut cursor = 0;

	let pid_str = format!("{:x}", pid);
	let now_str = format!("{:x}", now);

	let pid_bytes = pid_str.as_bytes();
	let now_bytes = now_str.as_bytes();

	let dash = b"-";

	// Copy parts into buffer
	let end = cursor + pid_bytes.len();
	buf[cursor..end].copy_from_slice(pid_bytes);
	cursor = end;

	if cursor < buf.len() {
		buf[cursor] = dash[0];
		cursor += 1;
	}

	let end = cursor + now_bytes.len();
	if end <= buf.len() {
		buf[cursor..end].copy_from_slice(now_bytes);
	}

	buf
}

pub fn string_to_buffer(input: &str) -> [u8; 32] {
	let mut buffer = [0u8; 32]; // Fixed-size buffer
	let bytes = input.as_bytes();
	let len = bytes.len().min(buffer.len()); // Avoid overflow

	buffer[..len].copy_from_slice(&bytes[..len]); // Copy into buffer

	buffer
}

/// Represents a range from start to end (inclusive).
#[derive(Debug, Clone)]
pub struct Range2D {
	pub start: u64,
	pub end: u64,
}

/// Build prefix sum to allow flat indexing into the total row space
fn build_prefix_sum(ranges: &[Range2D]) -> Vec<u64> {
	let mut prefix_sum = Vec::with_capacity(ranges.len());
	let mut total = 0u64;
	for r in ranges {
		total += r.end - r.start + 1;
		prefix_sum.push(total);
	}
	prefix_sum
}

/// Binary search to find which range a flat index falls into
fn find_range(index: u64, prefix_sum: &[u64]) -> usize {
	let mut low = 0;
	let mut high = prefix_sum.len();
	while low < high {
		let mid = (low + high) / 2;
		if prefix_sum[mid] <= index {
			low = mid + 1;
		} else {
			high = mid;
		}
	}
	low
}

/// Linear Congruential Generator
fn lcg(seed: &mut u64) -> u64 {
	// Constants for the LCG (values commonly used and known to have good properties)
	const A: u64 = 63689;
	const C: u64 = 95070899;
	const M: u64 = 4294967296; // 2^32

	*seed = (A.wrapping_mul(*seed).wrapping_add(C)) % M;
	*seed
}

/// Sample k random rows from the union of all ranges and return them in [row, row] format
pub fn sample_random_rows(ranges: &[Range2D], k: usize) -> Vec<[u64; 2]> {
	let prefix_sum = build_prefix_sum(ranges);
	let total_rows = *prefix_sum.last().unwrap_or(&0);

	assert!(k as u64 <= total_rows, "Requested more samples than available rows");

	let mut result = Vec::with_capacity(k);
	let mut rng_seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;

	let mut chosen_indices = Vec::with_capacity(k); // To track already chosen indices
	while chosen_indices.len() < k {
		let random_value = lcg(&mut rng_seed);
		println!("random_value: {}", random_value);
		let idx = (random_value % total_rows) as u64; // Get a value in range 0..total_rows

		if !chosen_indices.contains(&idx) {
			// Ensure uniqueness
			chosen_indices.push(idx); // Add to the list of chosen indices
			let range_index = find_range(idx, &prefix_sum);
			let base = if range_index == 0 { 0 } else { prefix_sum[range_index - 1] };
			let offset = idx - base;
			let row = ranges[range_index].start + offset;
			result.push([row, row]); // same structure as input, but single-row range
		}
	}
	result
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_sample_random_rows() {
		let ranges = vec![Range2D { start: 1, end: 3 }, Range2D { start: 5, end: 7 }, Range2D { start: 9, end: 11 }];
		let k = 3;
		let result = sample_random_rows(&ranges, k);
		assert_eq!(result.len(), k);

		// Check that the rows are within the given ranges and are unique
		let mut seen = std::collections::HashSet::new();
		for [row, _] in result {
			assert!((1..=3).contains(&row) || (5..=7).contains(&row) || (9..=11).contains(&row));
			assert!(seen.insert(row), "Duplicate row found: {}", row);
		}
	}

	#[test]
	fn test_sample_all_rows() {
		let ranges = vec![Range2D { start: 1, end: 3 }, Range2D { start: 5, end: 7 }];
		let k = 5;
		let result = sample_random_rows(&ranges, k);
		assert_eq!(result.len(), k);

		let expected_rows: std::collections::HashSet<u64> = [1, 2, 3, 5, 6, 7].iter().cloned().collect();
		let actual_rows: std::collections::HashSet<u64> = result.into_iter().map(|[row, _]| row).collect();

		assert_eq!(expected_rows, actual_rows);
	}

	#[test]
	fn test_empty_ranges() {
		let ranges: Vec<Range2D> = vec![];
		let k = 0;
		let result = sample_random_rows(&ranges, k);
		assert_eq!(result.len(), k);
	}

	#[test]
	#[should_panic]
	fn test_k_greater_than_total_rows() {
		let ranges = vec![Range2D { start: 1, end: 5 }];
		let k = 6;
		sample_random_rows(&ranges, k);
	}
}
