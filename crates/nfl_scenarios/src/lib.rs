#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::must_use_candidate)]
pub fn add(left: usize, right: usize) -> usize {
	left + right
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn it_works() {
		let result = add(2, 2);
		assert_eq!(result, 4);
	}
}
