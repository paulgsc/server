use crate::error::PlayTypeError;
use core::fmt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum PlayType {
	Kickoff,
	Run,
	Pass,
	Punt,
	ExtraPoint,
	FieldGoal,
	Penalty,
	Timeout,
	Sack,
	Kneel,
	Spike,
}

#[derive(Debug, Clone, PartialEq)]
enum PlayTypeCandidates {
	Kickoff,
	Punt,
	ExtraPoint,
	FieldGoal,
	Penalty,
	Timeout,
	Sack,
	Kneel,
	Spike,
	WeakRun,
	WeakPass,
	Unknown,
}

impl FromStr for PlayType {
	type Err = PlayTypeError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let lowercase = s.to_lowercase();

		// First, match weakly to candidates
		let candidate = if lowercase.contains("kickoff") {
			PlayTypeCandidates::Kickoff
		} else if lowercase.contains("punt") {
			PlayTypeCandidates::Punt
		} else if lowercase.contains("extra point") || lowercase.contains("pat") {
			PlayTypeCandidates::ExtraPoint
		} else if lowercase.contains("field goal") {
			PlayTypeCandidates::FieldGoal
		} else if lowercase.contains("penalty") {
			PlayTypeCandidates::Penalty
		} else if lowercase.contains("timeout") {
			PlayTypeCandidates::Timeout
		} else if lowercase.contains("sacked") {
			PlayTypeCandidates::Sack
		} else if lowercase.contains("kneels") || lowercase.contains("kneel down") {
			PlayTypeCandidates::Kneel
		} else if lowercase.contains("spiked") || lowercase.contains("spike") {
			PlayTypeCandidates::Spike
		} else if lowercase.contains("pass") {
			PlayTypeCandidates::WeakPass
		} else {
			// Weak match for run (using regex)
			let run_regex = Regex::new(r"\b(?:up the middle|left end|right end|left tackle|right tackle|left guard|right guard|scrambles)\b").unwrap();
			if run_regex.is_match(&lowercase) {
				PlayTypeCandidates::WeakRun
			} else {
				PlayTypeCandidates::Unknown
			}
		};

		// Now, refine based on candidate
		match candidate {
			// Strong matches we can return immediately
			PlayTypeCandidates::Kickoff => Ok(PlayType::Kickoff),
			PlayTypeCandidates::Punt => Ok(PlayType::Punt),
			PlayTypeCandidates::ExtraPoint => Ok(PlayType::ExtraPoint),
			PlayTypeCandidates::FieldGoal => Ok(PlayType::FieldGoal),
			PlayTypeCandidates::Penalty => Ok(PlayType::Penalty),
			PlayTypeCandidates::Timeout => Ok(PlayType::Timeout),
			PlayTypeCandidates::Sack => Ok(PlayType::Sack),
			PlayTypeCandidates::Kneel => Ok(PlayType::Kneel),
			PlayTypeCandidates::Spike => Ok(PlayType::Spike),

			// Weaker candidates need further checks
			PlayTypeCandidates::WeakPass => {
				if lowercase.contains("pass") {
					Ok(PlayType::Pass)
				} else {
					Err(PlayTypeError::UnknownPlayType { input: s.to_string() })
				}
			}
			PlayTypeCandidates::WeakRun => {
				if !lowercase.contains("pass") {
					Ok(PlayType::Run)
				} else {
					Err(PlayTypeError::UnknownPlayType { input: s.to_string() })
				}
			}

			// If none matched, return an error
			PlayTypeCandidates::Unknown => Err(PlayTypeError::UnknownPlayType { input: s.to_string() }),
		}
	}
}

impl From<String> for PlayType {
	fn from(s: String) -> Self {
		Self::from_str(&s).unwrap()
	}
}

impl fmt::Display for PlayType {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let display_str = match self {
			PlayType::Kickoff => "Kickoff",
			PlayType::Run => "Run",
			PlayType::Pass => "Pass",
			PlayType::Punt => "Punt",
			PlayType::ExtraPoint => "Extra Point",
			PlayType::FieldGoal => "Field Goal",
			PlayType::Penalty => "Penalty",
			PlayType::Timeout => "Timeout",
			PlayType::Sack => "Sack",
			PlayType::Kneel => "Kneel",
			PlayType::Spike => "Spike",
		};
		write!(f, "{}", display_str)
	}
}

// Implement the Display trait for PlayTypeCandidates
impl fmt::Display for PlayTypeCandidates {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let display_str = match self {
			PlayTypeCandidates::Kickoff => "Kickoff",
			PlayTypeCandidates::Punt => "Punt",
			PlayTypeCandidates::ExtraPoint => "Extra Point",
			PlayTypeCandidates::FieldGoal => "Field Goal",
			PlayTypeCandidates::Penalty => "Penalty",
			PlayTypeCandidates::Timeout => "Timeout",
			PlayTypeCandidates::Sack => "Sack",
			PlayTypeCandidates::Kneel => "Kneel",
			PlayTypeCandidates::Spike => "Spike",
			PlayTypeCandidates::WeakRun => "Weak Run",
			PlayTypeCandidates::WeakPass => "Weak Pass",
			PlayTypeCandidates::Unknown => "Unknown Play Type",
		};
		write!(f, "{}", display_str)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_run_play() {
		assert_eq!(PlayType::from_str("K.Cousins up the middle to TB 38 for 8 yards (L.David)."), Ok(PlayType::Run));
		assert_eq!(PlayType::from_str("D.Cook left end to MIN 27 for 2 yards (L.David)."), Ok(PlayType::Run));
		assert_eq!(
			PlayType::from_str("B.Mayfield scrambles right end pushed ob at TB 35 for 8 yards (A.Hamilton)."),
			Ok(PlayType::Run)
		);
	}

	#[test]
	fn test_parse_pass_play() {
		assert_eq!(
			PlayType::from_str("(8:22 - 2nd) (No Huddle, Shotgun) K.Cousins pass short right to K.Pitts to TB 38 for 8 yards (L.David)."),
			Ok(PlayType::Pass)
		);
	}

	#[test]
	fn test_parse_sack() {
		assert_eq!(
			PlayType::from_str("(15:00 - 2nd) K.Cousins sacked at MIN 25 for -7 yards (W.Gholston)."),
			Ok(PlayType::Sack)
		);
	}

	#[test]
	fn test_parse_kneel() {
		assert_eq!(PlayType::from_str("(:38) K.Cousins kneels to MIN 39 for -1 yards."), Ok(PlayType::Kneel));
	}

	#[test]
	fn test_parse_spike() {
		assert_eq!(PlayType::from_str("(:07) K.Cousins spiked the ball to stop the clock."), Ok(PlayType::Spike));
	}
}
