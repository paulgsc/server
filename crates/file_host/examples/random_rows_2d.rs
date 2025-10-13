use file_host::utils::{sample_random_rows, Range2D};

fn main() {
	let ranges = vec![Range2D { start: 0, end: 9 }, Range2D { start: 20, end: 29 }, Range2D { start: 100, end: 105 }];

	let k = 5;
	let result = sample_random_rows(&ranges, k);

	println!("Sampled rows (2D): {:?}", result);
}
