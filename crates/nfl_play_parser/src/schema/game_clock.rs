use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub enum Quarter {
    First,
    Second,
    Third,
    Fourth,
    OT
}

impl FromStr for Quarter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1st" => Ok(Quarter::First),
            "2nd" => Ok(Quarter::Second),
            "3rd" => Ok(Quarter::Third),
            "4th" => Ok(Quarter::Fourth),
            "OT" => Ok(Quarter::OT),
            _ => Err(format!("Invalid quarter: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct GameClock {
    minutes: u8,
    seconds: u8,
    quarter: Quarter,
}

impl FromStr for GameClock {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Remove parentheses and split by '-'
        let parts: Vec<&str> = s.trim_matches(|c| c == '(' || c == ')')
            .split('-')
            .map(str::trim)
            .collect();

        if parts.len() != 2 {
            return Err("Invalid game clock format".to_string());
        }

        let time_parts: Vec<&str> = parts[0].split(':').collect();
        if time_parts.len() != 2 {
            return Err("Invalid time format".to_string());
        }

        let minutes = time_parts[0].parse::<u8>()
            .map_err(|_| "Invalid minutes".to_string())?;
        let seconds = time_parts[1].parse::<u8>()
            .map_err(|_| "Invalid seconds".to_string())?;
        let quarter = Quarter::from_str(parts[1])?;

        Ok(GameClock {
            minutes,
            seconds,
            quarter,
        })
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
            ("(14:32 - 1st)", Ok(GameClock { minutes: 14, seconds: 32, quarter: Quarter::First })),
            ("(0:05 - 2nd)", Ok(GameClock { minutes: 0, seconds: 5, quarter: Quarter::Second })),
            ("(7:15 - 3rd)", Ok(GameClock { minutes: 7, seconds: 15, quarter: Quarter::Third })),
            ("(2:00 - 4th)", Ok(GameClock { minutes: 2, seconds: 0, quarter: Quarter::Fourth })),
            ("(10:00 - OT)", Ok(GameClock { minutes: 10, seconds: 0, quarter: Quarter::OT })),
            ("(15:00 - 1st)", Ok(GameClock { minutes: 15, seconds: 0, quarter: Quarter::First })),
            ("(14:32 - 5th)", Err("Invalid quarter: 5th".to_string())),
            ("(60:00 - 1st)", Err("Invalid minutes".to_string())),
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
            minutes: 14,
            seconds: 32,
            quarter: Quarter::First,
        });
    }
}

