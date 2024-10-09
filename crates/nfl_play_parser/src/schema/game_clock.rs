use std::str::FromStr;
use crate::error::GameClockError;

#[derive(Debug, Clone, PartialEq)]
pub enum Quarter {
    First,
    Second,
    Third,
    Fourth,
    OT,
}

/// Struct to represent minutes (valid range: 0-15)
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

/// Struct to represent seconds (valid range: 0-59)
#[derive(Debug, Clone, PartialEq)]
pub struct Seconds(u8);

impl Seconds {
    pub fn new(value: u8) -> Result<Self, GameClockError> {
        if value >= 60 {
            Err(GameClockError::invalid_minutes_error(value))
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
        GameClock {
            minutes,
            seconds,
            quarter,
        }
    }
}

impl FromStr for GameClock {
    type Err = GameClockError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Split the string into time and quarter parts
        let (time_str, quarter_str) = s.trim_matches(|c| c == '(' || c == ')')
                                       .split_once(" - ")
                                       .ok_or_else(|| GameClockError::invalid_format_error(s))?;

        // Split time into minutes and seconds
        let (minutes_str, seconds_str) = time_str.split_once(':')
            .ok_or_else(|| GameClockError::invalid_format_error(time_str))?;

        // Parse minutes and seconds
        let minutes = minutes_str.parse::<Minutes>()?;
        let seconds = seconds_str.parse::<Seconds>()?;

        // Parse quarter
        let quarter = Quarter::from_str(quarter_str)
            .map_err(|_| GameClockError::invalid_quarter_error(quarter_str))?;

        Ok(GameClock::new(minutes, seconds, quarter))
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
            "OT"  => Ok(Quarter::OT),
            _ => Err(GameClockError::invalid_quarter_error(s)),
        }
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
        assert!(Quarter::from_str("5th").is_err());
    }

    #[test]
    fn test_game_clock_from_str() {
        let test_cases = vec![
            ("(14:32 - 1st)", Ok(GameClock { minutes: Minutes(14), seconds: Seconds(32), quarter: Quarter::First })),
            ("(0:05 - 2nd)", Ok(GameClock { minutes: Minutes(0), seconds: Seconds(5), quarter: Quarter::Second })),
            ("(7:15 - 3rd)", Ok(GameClock { minutes: Minutes(7), seconds: Seconds(15), quarter: Quarter::Third })),
            ("(2:00 - 4th)", Ok(GameClock { minutes: Minutes(2), seconds: Seconds(0), quarter: Quarter::Fourth })),
            ("(10:00 - OT)", Ok(GameClock { minutes: Minutes(10), seconds: Seconds(0), quarter: Quarter::OT })),
            ("(15:00 - 1st)", Ok(GameClock { minutes: Minutes(15), seconds: Seconds(0), quarter: Quarter::First })),
            ("(14:32 - 5th)", Err("Invalid quarter: 5th".to_string())),
            ("(60:00 - 1st)", Err("Invalid minutes: must be between 0 and 15".to_string())),
            ("(14:60 - 1st)", Err("Invalid seconds".to_string())),
            ("14:32 - 1st", Err("Invalid game clock format".to_string())),
            ("(14:32 1st)", Err("Invalid game clock format".to_string())),
        ];

        for (input, expected) in test_cases {
            assert_eq!(GameClock::from_str(input), expected);
        }
    }

    #[test]
    fn test_game_clock_from_play_description() {
        let play_desc = "(14:32 - 1st) (No Huddle, Shotgun) K.Cousins pass deep right to K.Pitts to TB 36 for 32 yards (Z.McCollum).";
        let clock_str = play_desc.split_whitespace().take(2).collect::<Vec<&str>>().join(" ");
        let game_clock = GameClock::from_str(&clock_str).unwrap();

        assert_eq!(game_clock, GameClock {
            minutes: Minutes(14),
            seconds: Seconds(32), 
            quarter: Quarter::First,
        });
    }

}

