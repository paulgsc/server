use std::str::FromStr;
use crate::error::PlayTypeError;

#[derive(Debug, Clone, PartialEq)]
pub enum PlayType {
    Kickoff,
    Run,
    Pass,
    Sack,
    Kneel,
    Spike,
    Punt,
    ExtraPoint,
    Penalty,
    Timeout,
}

impl FromStr for PlayType {
    type Err = PlayTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lowercase = s.to_lowercase();
        
        match lowercase {
            s if s.contains("kickoff") => Ok(PlayType::Kickoff),
            s if s.contains("pass") => Ok(PlayType::Pass),
            s if s.contains("run") || s.contains("rush") => Ok(PlayType::Run),
            s if s.contains("punt") => Ok(PlayType::Punt),
            s if s.contains("extra point") || s.contains("pat") => Ok(PlayType::ExtraPoint),
            s if s.contains("penalty") => Ok(PlayType::Penalty),
            s if s.contains("timeout") => Ok(PlayType::Timeout),
            _ => Err(PlayTypeError::UnknownPlayType { input: s.to_string() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_play_type_from_str() {
        let test_cases = vec![
            ("J.Tucker kicks 65 yards from BAL 35 to end zone, Touchback.", PlayType::Kickoff),
            ("(Shotgun) L.Jackson pass short middle to M.Andrews to BAL 45 for 10 yards (M.Fitzpatrick).", PlayType::Pass),
            ("J.Dobbins right end to BAL 40 for 5 yards (T.Edmunds).", PlayType::Run),
            ("S.Koch punts 45 yards to PIT 15, Center-N.Moore. R.McCloud to PIT 22 for 7 yards (C.Board).", PlayType::Punt),
            ("J.Tucker extra point is GOOD, Center-N.Moore, Holder-S.Koch.", PlayType::ExtraPoint),
            ("PENALTY on PIT-C.Sutton, Defensive Pass Interference, 33 yards, enforced at BAL 32 - No Play.", PlayType::Penalty),
            ("Timeout #2 by BAL at 02:36.", PlayType::Timeout),
            ("L.Jackson scrambles left end to BAL 48 for 3 yards (V.Williams).", PlayType::Run),
            ("J.Tucker PAT attempt is No Good, hit right upright.", PlayType::ExtraPoint),
            ("2-10-BAL 25 (15:00) (Shotgun) L.Jackson sacked at BAL 18 for -7 yards (T.Watt).", PlayType::Pass),
        ];

        for (input, expected) in test_cases {
            assert_eq!(PlayType::from_str(input), Ok(expected), "Failed for input: {}", input);
        }
    }

    #[test]
    fn test_play_type_errors() {
        let error_cases = vec![
            "Coin toss won by BAL.",
            "End of the first quarter.",
            "Two-minute warning.",
        ];

        for input in error_cases {
            assert!(PlayType::from_str(input).is_err(), "Expected error for input: {}", input);
        }
    }

    #[test]
    fn test_play_type_case_insensitivity() {
        assert_eq!(PlayType::from_str("PENALTY on PIT-C.Sutton"), Ok(PlayType::Penalty));
        assert_eq!(PlayType::from_str("timeout #1 by BAL"), Ok(PlayType::Timeout));
        assert_eq!(PlayType::from_str("J.Tucker EXTRA POINT is GOOD"), Ok(PlayType::ExtraPoint));
    }
}

fn main() {
    // Example usage
    let play_desc = "(14:32 - 1st) (No Huddle, Shotgun) K.Cousins pass deep right to K.Pitts to TB 36 for 32 yards (Z.McCollum).";
    match PlayType::from_str(play_desc) {
        Ok(play_type) => println!("Parsed PlayType: {:?}", play_type),
        Err(e) => println!("Error parsing PlayType: {}", e),
    }
}
