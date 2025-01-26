use crate::common::nfl_server_error::NflServerError as Error;
use chrono::{Datelike, NaiveDate};
use nest::http::Error as NestError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{error::BoxDynError, sqlite::SqliteTypeInfo, Decode, Encode, Type};
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;

/// Constants for date encoding
pub const BASE_YEAR: i32 = 1970;
const YEAR_MASK: u16 = 0b1111111_0000_00000; // 7 bits for year
const MONTH_MASK: u16 = 0b0000000_1111_00000; // 4 bits for month
const DAY_MASK: u16 = 0b0000000_0000_11111; // 5 bits for day

/// Represents a compact, encoded date
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedDate {
	pub value: u16,
}

impl fmt::Display for EncodedDate {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.value)
	}
}

impl From<i64> for EncodedDate {
	fn from(value: i64) -> Self {
		if value >= 0 && value <= u16::MAX as i64 {
			Self { value: value as u16 }
		} else {
			panic!("Invalid EncodedDate: {value}");
		}
	}
}

impl Type<sqlx::Sqlite> for EncodedDate {
	fn type_info() -> SqliteTypeInfo {
		<i64 as Type<sqlx::Sqlite>>::type_info()
	}

	fn compatible(ty: &SqliteTypeInfo) -> bool {
		<i64 as Type<sqlx::Sqlite>>::compatible(ty)
	}
}
impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for EncodedDate {
	fn encode_by_ref(&self, args: &mut <sqlx::Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer) -> sqlx::encode::IsNull {
		let encoded_value = self.value as i64;
		<i64 as Encode<sqlx::Sqlite>>::encode_by_ref(&encoded_value, args)
	}
}

impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for EncodedDate {
	fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
		let decoded_value = <i64 as Decode<sqlx::Sqlite>>::decode(value)?;
		if decoded_value >= 0 && decoded_value <= u16::MAX as i64 {
			Ok(Self { value: decoded_value as u16 })
		} else {
			Err(Box::new(std::io::Error::new(
				std::io::ErrorKind::InvalidData,
				format!("Invalid EncodedDate value: {decoded_value}"),
			)))
		}
	}
}

/// A struct for creating dates with validation
#[derive(Debug, Deserialize)]
pub struct CreateDate {
	pub year: i32,
	pub month: u32,
	pub day: u32,
}

impl FromStr for CreateDate {
	type Err = Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let re = Regex::new(r"NFL (\d{4}).*Date: \w+, (\w+) (\d{1,2})(?:st|nd|rd|th)?").unwrap();

		if let Some(captures) = re.captures(s) {
			let year: i32 = captures[1].parse()?;

			let month_str = &captures[2];
			let month = match month_str.to_lowercase().as_str() {
				"january" => 1,
				"february" => 2,
				"march" => 3,
				"april" => 4,
				"may" => 5,
				"june" => 6,
				"july" => 7,
				"august" => 8,
				"september" => 9,
				"october" => 10,
				"november" => 11,
				"december" => 12,
				_ => return Err(Error::NestError(NestError::InvalidEncodedDate("Invalid month name".to_string()))),
			};

			let day: u32 = captures[3].parse()?;

			Ok(CreateDate { year, month, day })
		} else {
			Err(Error::NestError(NestError::InvalidEncodedDate("Failed to parse the date".to_string())))
		}
	}
}

impl EncodedDate {
	/// Encode a NaiveDate into a compact u16
	pub fn encode(date: NaiveDate) -> Option<Self> {
		// Validate year is within the supported range
		if date.year() < BASE_YEAR || date.year() > (BASE_YEAR + 127) {
			return None;
		}

		let year_offset = (date.year() - BASE_YEAR) as u16;
		let month = date.month() as u16;
		let day = date.day() as u16;

		Some(Self {
			value: (year_offset << 9) | (month << 5) | day,
		})
	}

	/// Decode the stored date
	pub fn decode(&self) -> Option<NaiveDate> {
		let year = ((self.value & YEAR_MASK) >> 9) as i32 + BASE_YEAR;
		let month = ((self.value & MONTH_MASK) >> 5) as u32;
		let day = (self.value & DAY_MASK) as u32;

		NaiveDate::from_ymd_opt(year, month, day)
	}

	/// Check if the encoded date is valid
	pub fn is_valid(&self) -> bool {
		self.decode().is_some()
	}
}

impl CreateDate {
	/// Validate the date
	pub fn is_valid(&self) -> bool {
		if let Some(date) = NaiveDate::from_ymd_opt(self.year, self.month, self.day) {
			return date.year() >= BASE_YEAR && date.year() <= (BASE_YEAR + 127);
		}
		false
	}

	/// Convert to an encoded date
	pub fn to_encoded(&self) -> Option<EncodedDate> {
		NaiveDate::from_ymd_opt(self.year, self.month, self.day).and_then(EncodedDate::encode)
	}
}

// Conversion implementations
impl TryFrom<NaiveDate> for EncodedDate {
	type Error = &'static str;

	fn try_from(date: NaiveDate) -> Result<Self, Self::Error> {
		Self::encode(date).ok_or("Date out of supported range")
	}
}

// Example usage
#[cfg(test)]
mod tests {
	use super::*;
	use chrono::NaiveDate;

	#[test]
	fn test_date_encoding_decoding() {
		let original_date = NaiveDate::from_ymd_opt(2000, 5, 15).unwrap();

		// Encode the date
		let encoded = EncodedDate::encode(original_date).unwrap();

		// Decode the date
		let decoded_date = encoded.decode().unwrap();

		// Verify the date is the same
		assert_eq!(original_date, decoded_date);
	}

	#[test]
	fn test_date_validation() {
		// Valid date
		let valid_date = CreateDate { year: 2000, month: 5, day: 15 };
		assert!(valid_date.is_valid());

		// Invalid date (out of range)
		let invalid_date = CreateDate { year: 1969, month: 1, day: 1 };
		assert!(!invalid_date.is_valid());
	}

	#[test]
	fn test_parse_valid_dates() {
		let test_cases = vec![
			(
				"NFL 2022 - WEEK 13 Schedule | NFL.com Date: Thursday, December 1st",
				CreateDate { year: 2022, month: 12, day: 1 },
			),
			(
				"NFL 2023 - WEEK 1 Schedule | NFL.com Date: Sunday, September 10th",
				CreateDate { year: 2023, month: 9, day: 10 },
			),
			(
				"NFL 2024 - WEEK 5 Schedule  NFL.com Date: Monday, October 2nd",
				CreateDate { year: 2024, month: 10, day: 2 },
			),
		];

		for (input, expected) in test_cases {
			let parsed_date = CreateDate::from_str(input).expect("Should parse successfully");
			assert_eq!(parsed_date.year, expected.year);
			assert_eq!(parsed_date.month, expected.month);
			assert_eq!(parsed_date.day, expected.day);
		}
	}
}
