use std::sync::atomic::{AtomicUsize, Ordering};
use std::str::FromStr;
use crate::schema::{DownAndDistance, GameClock, PlayType, ScoringEvent, Yards};



static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone)]
struct Play {
    id: usize,
    game_clock: GameClock,
    play_type: PlayType,
    scoring_event: Option<ScoringEvent>,
    description: String,
    line: DownAndDistance,
    yards: Option<Yards>,
}

impl Play {
    fn next_id() -> usize {
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    }

    // Constructor for a non-scoring play
    pub fn new(description: String, game_clock: GameClock, play_type: PlayType, line: DownAndDistance) -> Self {
        Play {
            id: Play::next_id(),
            description,
            game_clock,
            play_type,
            line,
            yards: None,
            scoring_event: None,
        }
    }

    // Constructor for a scoring play
    pub fn new_scoring(description: String, game_clock: GameClock, play_type: PlayType, line: DownAndDistance, scoring_event: ScoringEvent, yards: Yards) -> Self {
        Play {
            id: Play::next_id(),
            description,
            game_clock,
            play_type,
            line,
            yards: Some(yards),
            scoring_event: Some(scoring_event),
        }
    }
}
