use std::str::FromStr;
use crate::error::ScoringEventError;

#[derive(Debug, Clone, PartialEq)]
pub enum Points {
    One,
    Two,
    Three,
    Six,
}

impl Points {
    fn value(&self) -> u8 {
        match self {
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
    FieldGoal,
    ExtraPoint,
    TwoPointConversion,
    Safety,
    DefensiveTouchdown,
}

impl FromStr for ScoringEventType {
    type Err = ScoringEventError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lowercase = s.to_lowercase();
        match lowercase {
            s if s.contains("touchdown") => Ok(ScoringEventType::Touchdown),
            s if s.contains("field goal") => Ok(ScoringEventType::FieldGoal),
            s if s.contains("extra point") || s.contains("pat") => Ok(ScoringEventType::ExtraPoint),
            s if s.contains("two-point") || s.contains("2-point") => Ok(ScoringEventType::TwoPointConversion),
            s if s.contains("safety") => Ok(ScoringEventType::Safety),
            s if s.contains("defensive") && s.contains("touchdown") => Ok(ScoringEventType::DefensiveTouchdown),
            _ => Err(ScoringEventError::UnknownScoringEventType { input: s.to_string() }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScoringEvent {
    pub player: Option<String>,
    pub event_type: ScoringEventType,
    pub points: Points,
    pub description: String,
}

impl FromStr for ScoringEvent {
    type Err = ScoringEventError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let event_type = ScoringEventType::from_str(s)?;
        let player = extract_player(s);
        
        let points = match event_type {
            ScoringEventType::Touchdown | ScoringEventType::DefensiveTouchdown => Points::Six,
            ScoringEventType::FieldGoal => Points::Three,
            ScoringEventType::ExtraPoint => Points::One,
            ScoringEventType::TwoPointConversion | ScoringEventType::Safety => Points::Two,
        };

        Ok(ScoringEvent {
            player,
            event_type,
            points,
            description: s.to_string(),
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
                event_type: ScoringEventType::FieldGoal,
                points: Points::Three,
                description: "J.Tucker kicks 28 yard field goal is GOOD, Center-N.Moore, Holder-S.Koch.".to_string(),
             }),
            ("J.Tucker extra point is GOOD, Center-N.Moore, Holder-S.Koch.", 
             ScoringEvent {
                player: Some("J.Tucker".to_string()),
                event_type: ScoringEventType::ExtraPoint,
                points: Points::One,
                description: "J.Tucker extra point is GOOD, Center-N.Moore, Holder-S.Koch.".to_string(),
             }),
            ("L.Jackson pass short right to M.Brown for 11 yards, TOUCHDOWN.", 
             ScoringEvent {
                player: Some("L.Jackson".to_string()),
                event_type: ScoringEventType::Touchdown,
                points: Points::Six,
                description: "L.Jackson pass short right to M.Brown for 11 yards, TOUCHDOWN.".to_string(),
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
