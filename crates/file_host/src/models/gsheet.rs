use crate::{
	models::utils::{deserialize_color, deserialize_url},
	GSheetDeriveError,
};
use enum_name_derive::EnumFilename;
use gsheet_derive::FromGSheet;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct RangeQuery {
	pub range: Option<String>,
}

#[derive(EnumFilename, Serialize, std::fmt::Debug, Deserialize)]
pub enum SourceType {
	#[filename = "book"]
	Book,
	#[filename = "website"]
	Website,
	#[filename = "satellite data"]
	SatelliteData,
	#[filename = "data"]
	Data,
	#[filename = "document"]
	Document,
	#[filename = "image"]
	Image,
	#[filename = "music"]
	Music,
	#[filename = "video"]
	Video,
	#[filename = "code"]
	Code,
}

#[derive(Debug, Serialize, Deserialize, FromGSheet)]
pub struct Attribution {
	#[gsheet(column = "A")]
	source_type: SourceType,
	#[gsheet(column = "B")]
	title: String,
	#[gsheet(column = "C")]
	author: String,
	#[gsheet(column = "D")]
	#[serde(deserialize_with = "deserialize_url")]
	url: String,
	#[gsheet(column = "E")]
	thanks: String,
	#[gsheet(column = "F")]
	license: String,
	#[gsheet(column = "G")]
	thumbnail: String,
}

#[derive(Debug, Serialize, Deserialize, FromGSheet)]
pub struct VideoChapters {
	#[gsheet(column = "A")]
	id: u32,
	#[gsheet(column = "B")]
	title: String,
	#[gsheet(column = "C")]
	#[serde(deserialize_with = "deserialize_color")]
	color: String,
	#[gsheet(column = "D")]
	details: String,
}

pub trait FromGSheet: Sized {
	#[allow(dead_code)]
	fn column_mapping() -> Vec<(String, String, bool)>; // (field_name, column_letter, required)

	fn from_gsheet_row(row: &[String], header_map: &HashMap<String, usize>) -> Result<Self, GSheetDeriveError>;

	fn from_gsheet(data: &Vec<Vec<String>>, has_header: bool) -> Result<Vec<Self>, GSheetDeriveError> {
		if data.is_empty() {
			return Ok(Vec::new());
		}

		let header_map = if has_header {
			// Create mapping from column letters to indices
			let header = &data[0];
			let mut map = HashMap::new();
			for (idx, _) in header.iter().enumerate() {
				map.insert(index_to_column(idx), idx);
			}
			map
		} else {
			// No header, use indices directly
			let mut map = HashMap::new();
			for idx in 0..data[0].len() {
				map.insert(index_to_column(idx), idx);
			}
			map
		};

		let start_idx = if has_header { 1 } else { 0 };

		let mut results = Vec::new();
		for row_idx in start_idx..data.len() {
			let row = &data[row_idx];
			let item = Self::from_gsheet_row(row, &header_map)?;
			results.push(item);
		}

		Ok(results)
	}
}

// Helper function to convert column index to letter
pub fn index_to_column(idx: usize) -> String {
	let mut result = String::new();
	let mut n = idx + 1;

	while n > 0 {
		let remainder = (n - 1) % 26;
		result.insert(0, (remainder as u8 + b'A') as char);
		n = (n - 1) / 26;
	}

	result
}

// Helper function to convert column letter to index
#[allow(dead_code)]
pub fn column_to_index(column: &str) -> usize {
	column.chars().fold(0, |acc, c| acc * 26 + (c as usize - 'A' as usize + 1)) - 1
}

// Generic parsing helper
pub fn parse_cell<T>(value: &str, field_name: &str, column: &str) -> Result<T, GSheetDeriveError>
where
	T: FromStr,
	<T as FromStr>::Err: std::fmt::Display,
{
	let cleaned_value = value.trim_matches('"');
	cleaned_value
		.parse::<T>()
		.map_err(|e| GSheetDeriveError::ParseError(field_name.to_string(), column.to_string(), e.to_string()))
}

// Helper to get cell value with proper error handling
pub fn get_cell_value<'a>(
	row: &'a [String],
	column: &str,
	header_map: &HashMap<String, usize>,
	field_name: &str,
	required: bool,
) -> Result<Option<&'a str>, GSheetDeriveError> {
	let col_idx = header_map.get(column).ok_or_else(|| GSheetDeriveError::ColumnNotFound(column.to_string()))?;

	if *col_idx >= row.len() {
		if required {
			return Err(GSheetDeriveError::MissingRequiredField(field_name.to_string(), column.to_string()));
		}
		return Ok(None);
	}

	let value = &row[*col_idx];
	if value.is_empty() {
		if required {
			return Err(GSheetDeriveError::MissingRequiredField(field_name.to_string(), column.to_string()));
		}
		return Ok(None);
	}

	Ok(Some(value))
}

pub fn validate_range(range: &str) -> bool {
	let re = Regex::new(r"^(?:[a-zA-Z0-9_]+!)?([A-Z]+)(\d+):([A-Z]+)(\d+)$").unwrap();
	re.is_match(range)
}

#[derive(Serialize, Deserialize)]
pub struct GanttChapter {
	pub id: Box<str>,
	pub title: Box<str>,
	#[serde(rename = "startTime")]
	pub start_time: Box<str>,
	#[serde(rename = "endTime")]
	pub end_time: Box<str>,
	pub description: Box<str>,
	pub color: Box<str>,
	#[serde(rename = "subChapters")]
	pub sub_chapters: Vec<GanttSubChapter>,
}

#[derive(Serialize, Deserialize)]
pub struct GanttSubChapter {
	pub id: Box<str>,
	pub title: Box<str>,
	#[serde(rename = "startTime")]
	pub start_time: Box<str>,
	#[serde(rename = "endTime")]
	pub end_time: Box<str>,
	pub description: Box<str>,
	pub color: Box<str>,
}

#[cfg(test)]
mod tests {
	use super::*;

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

	#[derive(Debug, Deserialize, FromGSheet)]
	struct Person {
		#[gsheet(column = "A", required)]
		name: String,

		#[gsheet(column = "B")]
		age: Option<u32>,

		#[gsheet(column = "C")]
		email: Option<String>,
	}

	#[test]
	fn test_person_from_gsheet() {
		log::debug!("running foo foo!");
		// Sample data with quotes around some values
		let data = vec![
			vec!["Name".to_string(), "Age".to_string(), "Email".to_string()],
			vec!["\"John Doe\"".to_string(), "\"30\"".to_string(), "\"john@example.com\"".to_string()],
			vec!["\"Jane Smith\"".to_string(), "".to_string(), "".to_string()],
		];

		let people = Person::from_gsheet(&data, true).unwrap();
		assert_eq!(people.len(), 2);
		assert_eq!(people[0].name, "John Doe");
		assert_eq!(people[0].age, Some(30));
		assert_eq!(people[1].name, "Jane Smith");
		assert_eq!(people[1].email, None);
	}
}
