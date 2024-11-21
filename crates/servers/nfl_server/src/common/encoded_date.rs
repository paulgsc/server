use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

/// Constants for date encoding
pub const BASE_YEAR: i32 = 1970;
const YEAR_MASK: u16 = 0b1111111_0000_00000; // 7 bits for year
const MONTH_MASK: u16 = 0b0000000_1111_00000; // 4 bits for month
const DAY_MASK: u16 = 0b0000000_0000_11111;   // 5 bits for day

/// Represents a compact, encoded date
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedDate {
    pub value: u16,
}

/// A struct for creating dates with validation
#[derive(Debug, Deserialize)]
pub struct CreateDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
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
            value: (year_offset << 9) | (month << 5) | day
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
        NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .and_then(EncodedDate::encode)
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
        let valid_date = CreateDate { 
            year: 2000, 
            month: 5, 
            day: 15 
        };
        assert!(valid_date.is_valid());

        // Invalid date (out of range)
        let invalid_date = CreateDate { 
            year: 1969, 
            month: 1, 
            day: 1 
        };
        assert!(!invalid_date.is_valid());
    }
}
