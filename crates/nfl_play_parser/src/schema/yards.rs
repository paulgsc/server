use crate::error::YardsError;
use regex::Regex;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub enum YardType {
	Gain,
	Loss,
	NoGain,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Yards {
	pub value: u8,
	pub yard_type: YardType,
}

impl Yards {
	pub fn new(value: u8, yard_type: YardType) -> Result<Self, YardsError> {
		if value <= 100 {
			Ok(Self { value, yard_type })
		} else {
			Err(YardsError::InvalidYards { value })
		}
	}
}

impl FromStr for Yards {
	type Err = YardsError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		// Regular expression to match yard gains/losses
		let re = Regex::new(r"for (-?\d+) yards?|for no gain").unwrap();

		if let Some(caps) = re.captures(s) {
			if let Some(yards_match) = caps.get(1) {
				// Yards gained or lost
				let value: i8 = yards_match.as_str().parse().map_err(|_| YardsError::InvalidYardsFormat(s.to_string()))?;
				let (value, yard_type) = if value > 0 {
					(value as u8, YardType::Gain)
				} else if value < 0 {
					(value.abs() as u8, YardType::Loss)
				} else {
					(0, YardType::NoGain)
				};
				Yards::new(value, yard_type)
			} else {
				// "for no gain" case
				Yards::new(0, YardType::NoGain)
			}
		} else if s.contains("pass incomplete") {
			// Treat incomplete passes as no gain
			Yards::new(0, YardType::NoGain)
		} else {
			Err(YardsError::NoYardsInfo)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_yards_parsing() {
		assert_eq!(
			"(15:00 - 1st) (Shotgun) B.Robinson right tackle to ATL 32 for 2 yards (T.Smith, L.David)."
				.parse::<Yards>()
				.unwrap(),
			Yards {
				value: 2,
				yard_type: YardType::Gain
			}
		);
		assert_eq!(
			"(12:39 - 1st) (Shotgun) B.Robinson right end to TB 18 for no gain (T.Smith).".parse::<Yards>().unwrap(),
			Yards {
				value: 0,
				yard_type: YardType::NoGain
			}
		);
		assert_eq!(
			"(1:30 - 1st) (Shotgun) B.Mayfield pass incomplete short left to C.Godwin.".parse::<Yards>().unwrap(),
			Yards {
				value: 0,
				yard_type: YardType::NoGain
			}
		);
		assert_eq!(
			"(11:05 - 2nd) R.White right end to ATL 23 for -1 yards (K.Elliss, Z.Harrison).".parse::<Yards>().unwrap(),
			Yards {
				value: 1,
				yard_type: YardType::Loss
			}
		);
	}

	#[test]
	fn test_invalid_play_description() {
		assert_eq!("Invalid play description".parse::<Yards>().unwrap_err(), YardsError::NoYardsInfo);
	}
}
