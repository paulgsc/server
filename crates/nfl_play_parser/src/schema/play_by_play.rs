use std::sync::atomic::{AtomicUsize, Ordering};
use std::str::FromStr;
use crate::schema::{DownAndDistance, GameClock, PlayType, ScoringEvent, Yards};
use crate::error::{PlayByPlayError, YardsError};


const PLAY_DELIMITER: &str = " ||| ";

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone)]
struct Play {
    id: usize,
    game_clock: GameClock,
    play_type: PlayType,
    line: DownAndDistance,
    scoring_event: Option<ScoringEvent>,
    yards: Option<Yards>,
}

impl Play {
    fn next_id() -> usize {
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    }

    // Unified constructor with optional parameters for scoring plays
    pub fn new(
        game_clock: GameClock,
        play_type: PlayType,
        line: DownAndDistance,
        scoring_event: Option<ScoringEvent>,
        yards: Option<Yards>,
    ) -> Self {
        Play {
            id: Play::next_id(),
            game_clock,
            play_type,
            line,
            scoring_event,
            yards,
        }
    }
}


impl FromStr for Play {
    type Err = PlayByPlayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(PLAY_DELIMITER).collect();

        if parts.len() != 2 {
            return Err(PlayByPlayError::InvalidFormat);
        }

        let game_clock = GameClock::from_str(parts[1])?;
        let line = DownAndDistance::from_str(parts[0])
            .map_err(|e| PlayByPlayError::DownAndDistance(e))?;
        let play_type = PlayType::from_str(parts[1])?;
        let yards = Yards::from_str(parts[1]).ok();
        let scoring_event = ScoringEvent::from_str(parts[1]).ok();

        if let Some(scoring_event) = scoring_event {
            Ok(Play::new(game_clock, play_type, line, Some(scoring_event), yards))
        } else {
            Ok(Play::new(game_clock, play_type, line, None, None))
        }

    }
}

// Example implementations for PlayType and ScoringEvent (you may need to adjust these)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_play_from_str() {
        let play_description = "(15:00 - 1st) (1st & 10 at ATL 25) B.Robinson right tackle to ATL 32 for 7 yards (T.Smith, L.David).";
        let play = Play::from_str(play_description).unwrap();

        assert_eq!(play.game_clock, GameClock::from_str("(15:00 - 1st)").unwrap());
        assert_eq!(play.line, DownAndDistance::from_str("1st & 10 at ATL 25").unwrap());
        assert_eq!(play.play_type, PlayType::Run);
        assert_eq!(play.yards, Some(Yards::new(7, YardType::Gain).unwrap()));
        assert_eq!(play.scoring_event, None);
        assert_eq!(play.description, play_description);
    }

    #[test]
    fn test_scoring_play_from_str() {
        let play_description = "(10:15 - 2nd) (3rd & Goal at TB 2) T.Brady pass short right to M.Evans for 2 yards, TOUCHDOWN.";
        let play = Play::from_str(play_description).unwrap();

        assert_eq!(play.game_clock, GameClock::from_str("(10:15 - 2nd)").unwrap());
        assert_eq!(play.line, DownAndDistance::from_str("3rd & Goal at TB 2").unwrap());
        assert_eq!(play.play_type, PlayType::Pass);
        assert_eq!(play.yards, Some(Yards::new(2, YardType::Gain).unwrap()));
        assert_eq!(play.scoring_event, Some(ScoringEvent::Touchdown));
        assert_eq!(play.description, play_description);
    }

    // Add more tests for different play scenarios
}
