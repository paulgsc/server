use std::str::FromStr;
use crate::error::ScoringEventError;

#[derive(Debug, Clone, PartialEq)]
pub enum Points {
    Zero,
    One,
    Two,
    Three,
    Six,
}

impl Points {
    fn value(&self) -> u8 {
        match self {
            Points::Zero => 0,
            Points::One => 1,
            Points::Two => 2,
            Points::Three => 3,
            Points::Six => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScoringEventType {
    Touchdown,
    FieldGoalAttempt(bool),
    ExtraPointAttempt(bool),
    TwoPointConversionAttempt(bool),
    Safety,
    DefensiveTouchdown,
}

impl FromStr for ScoringEventType {
    type Err = ScoringEventError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lowercase = s.to_lowercase();

        match lowercase.as_str() {
            s if s.contains("touchdown") => {
                if s.contains("defensive") {
                    Ok(ScoringEventType::DefensiveTouchdown)
                } else {
                    Ok(ScoringEventType::Touchdown)
                }
            }
            s if s.contains("field goal") => {
                let is_good = s.contains("is good");
                Ok(ScoringEventType::FieldGoalAttempt(is_good))
            }
            s if s.contains("extra point") || s.contains("pat") => {
                let is_good = s.contains("is good");
                Ok(ScoringEventType::ExtraPointAttempt(is_good))
            }
            s if s.contains("two-point") || s.contains("2-point") => {
                let is_good = !s.contains("failed") && !s.contains("no good");
                Ok(ScoringEventType::TwoPointConversionAttempt(is_good))
            }
            s if s.contains("safety") => Ok(ScoringEventType::Safety),
            _ => Err(ScoringEventError::UnknownScoringEventType {
                input: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoringEvent {
    pub player: Option<String>,
    pub event_type: ScoringEventType,
    pub points: Points,
}

impl FromStr for ScoringEvent {
    type Err = ScoringEventError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let event_type = ScoringEventType::from_str(s)?;
        let player = extract_player(s);
        
        let points = match event_type {
            ScoringEventType::Touchdown | ScoringEventType::DefensiveTouchdown => Points::Six,
            ScoringEventType::FieldGoalAttempt(true) => Points::Three,
            ScoringEventType::FieldGoalAttempt(false) => Points::Zero,
            ScoringEventType::ExtraPointAttempt(true) => Points::One,
            ScoringEventType::ExtraPointAttempt(false) => Points::Zero,
            ScoringEventType::TwoPointConversionAttempt(true) => Points::Two,
            ScoringEventType::TwoPointConversionAttempt(false) => Points::Zero,
            ScoringEventType::Safety => Points::Two,
        };

        Ok(ScoringEvent {
            player,
            event_type,
            points,
        })
    }
}

fn extract_player(s: &str) -> Option<String> {
    // This is a simplistic approach. You might need a more sophisticated method
    // to extract player names based on your actual data format.
    s.split_whitespace()
        .next()
        .map(|name| name.trim_end_matches('.').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scoring_event_from_str() {
        let test_cases = vec![
            ("J.Tucker kicks 28 yard field goal is GOOD, Center-N.Moore, Holder-S.Koch.", 
             ScoringEvent {
                player: Some("J.Tucker".to_string()),
                event_type: ScoringEventType::FieldGoalAttempt(true),
                points: Points::Three,
             }),
            ("J.Tucker extra point is GOOD, Center-N.Moore, Holder-S.Koch.", 
             ScoringEvent {
                player: Some("J.Tucker".to_string()),
                event_type: ScoringEventType::ExtraPointAttempt(true),
                points: Points::One,
             }),
            ("L.Jackson pass short right to M.Brown for 11 yards, TOUCHDOWN.", 
             ScoringEvent {
                player: Some("L.Jackson".to_string()),
                event_type: ScoringEventType::Touchdown,
                points: Points::Six,
             }),
            ("(10:17 - 3rd) H.Butker 51 yard field goal is No Good, Hit Right Upright, Center-J.Winchester, Holder-M.Araiza.",
             ScoringEvent {
                player: Some("H.Butker".to_string()),
                event_type: ScoringEventType::FieldGoalAttempt(false),
                points: Points::Zero,
             }),
        ];

        for (input, expected) in test_cases {
            assert_eq!(ScoringEvent::from_str(input), Ok(expected), "Failed for input: {}", input);
        }
    }

    #[test]
    fn test_scoring_event_errors() {
        let error_cases = vec![
            "Coin toss won by BAL.",
            "End of the first quarter.",
            "Two-minute warning.",
        ];

        for input in error_cases {
            assert!(ScoringEvent::from_str(input).is_err(), "Expected error for input: {}", input);
        }
    }
}
