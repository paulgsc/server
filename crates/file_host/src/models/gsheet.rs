use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RangeQuery {
	pub range: Option<String>,
}

pub fn validate_range(range: &str) -> bool {
	let re = Regex::new(r"^(?:[a-zA-Z0-9_]+!)?([A-Z]+)(\d+):([A-Z]+)(\d+)$").unwrap();
	re.is_match(range)
}

#[cfg(test)]
mod tests {
	use super::validate_range;

	#[test]
	fn test_valid_range() {
		assert!(validate_range("default!A1:B4"));
	}

	#[test]
	fn test_invalid_range_incorrect_column_format() {
		assert!(!validate_range("defaultA1:B4"));
	}

	#[test]
	fn test_valid_range_multiple_columns() {
		assert!(validate_range("AA1:BB10"));
	}

	#[test]
	fn test_invalid_range_missing_colon() {
		assert!(!validate_range("A1B2"));
	}
}
