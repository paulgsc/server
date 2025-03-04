use crate::error::GameClockError;
use core::fmt;
use regex::Regex;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub enum Quarter {
	First,
	Second,
	Third,
	Fourth,
	OT,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Minutes(u8);

impl Minutes {
	pub fn new(value: u8) -> Result<Self, GameClockError> {
		if value > 15 {
			Err(GameClockError::invalid_minutes_error(value))
		} else {
			Ok(Minutes(value))
		}
	}
}

impl FromStr for Minutes {
	type Err = GameClockError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let value = s.parse::<u8>()?;
		Minutes::new(value)
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seconds(u8);

impl Seconds {
	pub fn new(value: u8) -> Result<Self, GameClockError> {
		if value >= 60 {
			Err(GameClockError::invalid_seconds_error(value))
		} else {
			Ok(Seconds(value))
		}
	}
}

impl FromStr for Seconds {
	type Err = GameClockError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let value = s.parse::<u8>()?;
		Seconds::new(value)
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct GameClock {
	minutes: Minutes,
	seconds: Seconds,
	quarter: Quarter,
}

impl GameClock {
	pub fn new(minutes: Minutes, seconds: Seconds, quarter: Quarter) -> Self {
		GameClock { minutes, seconds, quarter }
	}
}

impl FromStr for GameClock {
	type Err = GameClockError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		// Define a regex pattern to match the game clock format
		let re = Regex::new(r"\((\d{1,2}:\d{2})\s+-\s+(\w+)\)").unwrap();

		// Attempt to find a match for the game clock
		if let Some(captures) = re.captures(s) {
			let time_str = captures.get(1).map_or("", |m| m.as_str());
			let quarter_str = captures.get(2).map_or("", |m| m.as_str());

			// Validate time format (MM:SS)
			let (minutes_str, seconds_str) = time_str.split_once(':').ok_or_else(|| GameClockError::invalid_time_format_error(time_str))?;

			// Parse minutes and seconds
			let minutes = minutes_str.parse::<Minutes>().map_err(|_| GameClockError::InvalidMinutes {
				minutes: minutes_str.parse::<u8>().unwrap_or(0),
			})?;
			let seconds = seconds_str.parse::<Seconds>().map_err(|_| GameClockError::InvalidSeconds {
				seconds: seconds_str.parse::<u8>().unwrap_or(0),
			})?;

			// Parse quarter
			let quarter = Quarter::from_str(quarter_str).map_err(|_| GameClockError::InvalidQuarter { quarter: quarter_str.to_string() })?;

			Ok(GameClock::new(minutes, seconds, quarter))
		} else {
			Err(GameClockError::InvalidFormat("Invalid game clock format".to_string()))
		}
	}
}

impl FromStr for Quarter {
	type Err = GameClockError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"1st" => Ok(Quarter::First),
			"2nd" => Ok(Quarter::Second),
			"3rd" => Ok(Quarter::Third),
			"4th" => Ok(Quarter::Fourth),
			"OT" => Ok(Quarter::OT),
			_ => Err(GameClockError::invalid_quarter_error(s)),
		}
	}
}

impl fmt::Display for Quarter {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Quarter::First => write!(f, "1st"),
			Quarter::Second => write!(f, "2nd"),
			Quarter::Third => write!(f, "3rd"),
			Quarter::Fourth => write!(f, "4th"),
			Quarter::OT => write!(f, "OT"),
		}
	}
}

impl fmt::Display for Minutes {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:02}", self.0)
	}
}

impl fmt::Display for Seconds {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:02}", self.0)
	}
}

impl fmt::Display for GameClock {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "({:02}:{:02} - {})", self.minutes, self.seconds, self.quarter)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_quarter_from_str() {
		assert_eq!(Quarter::from_str("1st"), Ok(Quarter::First));
		assert_eq!(Quarter::from_str("2nd"), Ok(Quarter::Second));
		assert_eq!(Quarter::from_str("3rd"), Ok(Quarter::Third));
		assert_eq!(Quarter::from_str("4th"), Ok(Quarter::Fourth));
		assert_eq!(Quarter::from_str("OT"), Ok(Quarter::OT));
		assert_eq!(Quarter::from_str("5th"), Err(GameClockError::InvalidQuarter { quarter: "5th".to_string() }));
	}

	#[test]
	fn test_game_clock_from_str() {
		let test_cases = vec![
			(
				"(14:32 - 1st)",
				Ok(GameClock {
					minutes: Minutes(14),
					seconds: Seconds(32),
					quarter: Quarter::First,
				}),
			),
			(
				"(0:05 - 2nd)",
				Ok(GameClock {
					minutes: Minutes(0),
					seconds: Seconds(5),
					quarter: Quarter::Second,
				}),
			),
			(
				"(7:15 - 3rd)",
				Ok(GameClock {
					minutes: Minutes(7),
					seconds: Seconds(15),
					quarter: Quarter::Third,
				}),
			),
			(
				"(2:00 - 4th)",
				Ok(GameClock {
					minutes: Minutes(2),
					seconds: Seconds(0),
					quarter: Quarter::Fourth,
				}),
			),
			(
				"(10:00 - OT)",
				Ok(GameClock {
					minutes: Minutes(10),
					seconds: Seconds(0),
					quarter: Quarter::OT,
				}),
			),
			(
				"(15:00 - 1st)",
				Ok(GameClock {
					minutes: Minutes(15),
					seconds: Seconds(0),
					quarter: Quarter::First,
				}),
			),
			("(14:32 - 5th)", Err(GameClockError::InvalidQuarter { quarter: "5th".to_string() })),
			("(60:00 - 1st)", Err(GameClockError::InvalidMinutes { minutes: 60 })),
			("(14:60 - 1st)", Err(GameClockError::InvalidSeconds { seconds: 60 })),
			("14:32 - 1st", Err(GameClockError::InvalidFormat("Invalid game clock format".to_string()))),
			("(14:32 1st)", Err(GameClockError::InvalidFormat("Invalid game clock format".to_string()))),
		];

		for (input, expected) in test_cases {
			assert_eq!(GameClock::from_str(input), expected);
		}
	}

	#[test]
	fn test_game_clock_from_play_description() {
		let input = "(14:32 - 1st) (No Huddle, Shotgun) K.Cousins pass deep right to K.Pitts to TB 36 for 32 yards (Z.McCollum).";
		let expected = Ok(GameClock {
			minutes: Minutes(14),
			seconds: Seconds(32),
			quarter: Quarter::First,
		});

		let result = GameClock::from_str(input);
		assert_eq!(result, expected);
	}
}
