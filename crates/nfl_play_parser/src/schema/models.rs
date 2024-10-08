use std::str::FromStr;

#[derive(Debug, Clone)]
struct GameClock {
    minutes: u8,
    seconds: u8,
    quarter: u8,
}

#[derive(Debug, Clone)]
enum PlayType {
    Kickoff,
    Run,
    Pass,
    Punt,
    FieldGoal,
    ExtraPoint,
    Penalty,
    Timeout,
    TwoPointConversion,
}

#[derive(Debug, Clone)]
struct Player {
    name: String,
    team: String,
}

#[derive(Debug, Clone)]
struct Play {
    game_clock: GameClock,
    play_type: PlayType,
    description: String,
    yards: i32,
    players_involved: Vec<Player>,
}

#[derive(Debug, Clone)]
struct DownAndDistance {
    down: u8,
    distance: u8,
    yard_line: u8,
    possession: String,
}

impl FromStr for GameClock {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err("Invalid game clock format".to_string());
        }

        let minutes = parts[0].parse::<u8>().map_err(|_| "Invalid minutes")?;
        let seconds = parts[1].parse::<u8>().map_err(|_| "Invalid seconds")?;

        Ok(GameClock {
            minutes,
            seconds,
            quarter: 1, // Default to 1st quarter, update later
        })
    }
}

impl FromStr for DownAndDistance {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 6 {
            return Err("Invalid down and distance format".to_string());
        }

        let down = match parts[0] {
            "1st" => 1,
            "2nd" => 2,
            "3rd" => 3,
            "4th" => 4,
            _ => return Err("Invalid down".to_string()),
        };

        let distance = parts[2].parse::<u8>().map_err(|_| "Invalid distance")?;
        let yard_line = parts[5].parse::<u8>().map_err(|_| "Invalid yard line")?;
        let possession = parts[4].to_string();

        Ok(DownAndDistance {
            down,
            distance,
            yard_line,
            possession,
        })
    }
}


