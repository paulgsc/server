use crate::error::DownAndDistanceError;
use crate::schema::TeamAbbreviation;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub struct YardLine(u8);

impl YardLine {
	pub fn new(value: u8) -> Result<Self, DownAndDistanceError> {
		if (1..=100).contains(&value) {
			Ok(Self(value))
		} else {
			Err(DownAndDistanceError::InvalidYardLine)
		}
	}
}

#[derive(Debug, Clone, PartialEq)]
pub enum Down {
	First,
	Second,
	Third,
	Fourth,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Distance {
	Yards(u8),
	Goal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownAndDistance {
	down: Down,
	distance: Distance,
	yard_line: YardLine,
	side_of_ball: TeamAbbreviation,
}

impl FromStr for DownAndDistance {
	type Err = DownAndDistanceError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let parts: Vec<&str> = s.split_whitespace().collect();
		if parts.len() != 6 {
			return Err(DownAndDistanceError::InvalidDownDistanceFormat);
		}

		// Parse down
		let down = match parts[0] {
			"1st" => Down::First,
			"2nd" => Down::Second,
			"3rd" => Down::Third,
			"4th" => Down::Fourth,
			_ => return Err(DownAndDistanceError::InvalidDown),
		};

		let distance = if parts[2] == "Goal" {
			Distance::Goal
		} else {
			let distance_value = parts[2].parse::<u8>().map_err(|_| DownAndDistanceError::InvalidDownDistance)?;
			Distance::Yards(distance_value)
		};

		let yard_line_value = parts[5].parse::<u8>().map_err(|_| DownAndDistanceError::InvalidYardLine)?;
		let yard_line = YardLine::new(yard_line_value)?;
		let side_of_ball = TeamAbbreviation::from_str(parts[4])?;

		Ok(DownAndDistance {
			down,
			distance,
			yard_line,
			side_of_ball,
		})
	}
}

impl fmt::Display for Down {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Down::First => write!(f, "1st"),
			Down::Second => write!(f, "2nd"),
			Down::Third => write!(f, "3rd"),
			Down::Fourth => write!(f, "4th"),
		}
	}
}

impl fmt::Display for Distance {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Distance::Yards(yards) => write!(f, "{}", yards),
			Distance::Goal => write!(f, "Goal"),
		}
	}
}

impl fmt::Display for YardLine {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl fmt::Display for DownAndDistance {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} & {} at {} {}", self.down, self.distance, self.side_of_ball, self.yard_line)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::error::{DownAndDistanceError, TeamAbbreviationError};
	use std::str::FromStr;

	#[test]
	fn test_valid_down_and_distance_parsing() {
		let input = "1st & 10 at ATL 30";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_ok());
		let down_and_distance = result.unwrap();
		assert_eq!(down_and_distance.down, Down::First);
		assert_eq!(down_and_distance.distance, Distance::Yards(10));
		assert_eq!(down_and_distance.yard_line, YardLine::new(30).unwrap());
		assert_eq!(down_and_distance.side_of_ball, TeamAbbreviation::ATL);
	}

	#[test]
	fn test_invalid_down_and_distance_parsing_invalid_format() {
		let input = "Invalid format";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidDownDistanceFormat);
	}

	#[test]
	fn test_invalid_down_and_distance_parsing_invalid_distance() {
		let input = "1st & XX at ATL 30";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidDownDistance);
	}

	#[test]
	fn test_invalid_down_and_distance_parsing_invalid_yard_line() {
		let input = "1st & 10 at ATL XX";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidYardLine);
	}

	#[test]
	fn test_invalid_down_parsing() {
		let input = "5st & 10 at TB 18";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidDown);
	}

	#[test]
	fn test_invalid_team_abbreviation() {
		let input = "1st & 10 at XYZ 30";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_err());

		match result.unwrap_err() {
			DownAndDistanceError::TeamAbbreviationError(TeamAbbreviationError::InvalidTeamAbbreviation(_)) => {}
			_ => panic!("Expected InvalidTeamAbbreviation error"),
		}
	}

	#[test]
	fn test_valid_down_and_distance_parsing_goal_line() {
		let input = "3rd & Goal at ATL 4";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_ok());
		let down_and_distance = result.unwrap();
		assert_eq!(down_and_distance.down, Down::Third);
		assert_eq!(down_and_distance.distance, Distance::Goal); // Assuming Distance(4) represents goal-to-go
		assert_eq!(down_and_distance.yard_line, YardLine::new(4).unwrap());
		assert_eq!(down_and_distance.side_of_ball, TeamAbbreviation::ATL);
	}

	#[test]
	fn test_invalid_yard_line_out_of_range() {
		let input = "1st & 10 at ATL 1000";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidYardLine);
	}

	#[test]
	fn test_invalid_yard_line_zero() {
		let input = "1st & 10 at ATL 0";
		let result = DownAndDistance::from_str(input);
		assert!(result.is_err());
		assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidYardLine);
	}
}
