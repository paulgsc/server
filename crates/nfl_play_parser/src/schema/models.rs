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
    ExtraPoint,
    Penalty,
    Timeout,
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

#[derive(Debug, Clone)]
pub enum ScoringEventType {
    Touchdown,
    FieldGoal,
    ExtraPoint,
    TwoPointConversion,
    Safety,
    DefensiveTouchdown, // For when the defense scores (interception return, fumble return, etc.)
}

#[derive(Debug, Clone)]
pub struct ScoringEvent {
    pub team: String,           // Name of the team that scored
    pub player: Option<String>, // Player involved in the score, if applicable (e.g., for TDs or FGs)
    pub event_type: ScoringEventType, // Type of scoring event
    pub points: u8,             // Number of points awarded for the score
    pub description: String,    // Original description of the scoring event
}

impl ScoringEvent {
    // Constructor for creating a new scoring event
    pub fn new(team: String, player: Option<String>, event_type: ScoringEventType, points: u8, description: String) -> Self {
        ScoringEvent {
            team,
            player,
            event_type,
            points,
            description,
        }
    }

    // A function to create a touchdown event
    pub fn touchdown(team: String, player: String, description: String) -> Self {
        ScoringEvent::new(team, Some(player), ScoringEventType::Touchdown, 6, description)
    }

    // A function to create a field goal event
    pub fn field_goal(team: String, player: String, description: String) -> Self {
        ScoringEvent::new(team, Some(player), ScoringEventType::FieldGoal, 3, description)
    }

    // A function to create an extra point event
    pub fn extra_point(team: String, player: String, description: String) -> Self {
        ScoringEvent::new(team, Some(player), ScoringEventType::ExtraPoint, 1, description)
    }

    // A function to create a two-point conversion event
    pub fn two_point_conversion(team: String, player: String, description: String) -> Self {
        ScoringEvent::new(team, Some(player), ScoringEventType::TwoPointConversion, 2, description)
    }

    // A function to create a safety event
    pub fn safety(team: String, description: String) -> Self {
        ScoringEvent::new(team, None, ScoringEventType::Safety, 2, description)
    }

    // A function to create a defensive touchdown event
    pub fn defensive_touchdown(team: String, player: String, description: String) -> Self {
        ScoringEvent::new(team, Some(player), ScoringEventType::DefensiveTouchdown, 6, description)
    }
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


