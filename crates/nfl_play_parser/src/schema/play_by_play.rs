use crate::error::PlayByPlayError;
use crate::query_selectors::PlayDescription;
use crate::schema::{DownAndDistance, GameClock, PlayType, ScoringEvent, Yards};
use core::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone)]
pub struct Play {
	id: usize,
	game_clock: GameClock,
	play_type: PlayType,
	line: DownAndDistance,
	scoring_event: Option<ScoringEvent>,
	yards: Option<Yards>,
	team_on_offense: Option<String>,
}

impl Play {
	fn next_id() -> usize {
		NEXT_ID.fetch_add(1, Ordering::SeqCst)
	}

	pub fn new(game_clock: GameClock, play_type: PlayType, line: DownAndDistance, scoring_event: Option<ScoringEvent>, yards: Option<Yards>, team: Option<String>) -> Self {
		Play {
			id: Self::next_id(),
			game_clock,
			play_type,
			line,
			scoring_event,
			yards,
			team_on_offense: team,
		}
	}
}

impl TryFrom<PlayDescription> for Play {
	type Error = PlayByPlayError;

	fn try_from(desc: PlayDescription) -> Result<Self, Self::Error> {
		let game_clock_str = desc.description.as_deref().ok_or(PlayByPlayError::MissingDescription)?;
		let headline_str = desc.headline.as_deref().ok_or(PlayByPlayError::MissingHeadline)?;
		let team_str = desc.team_name;

		let game_clock = GameClock::from_str(game_clock_str)?;
		let line = DownAndDistance::from_str(headline_str).map_err(PlayByPlayError::DownAndDistance)?;
		let play_type = PlayType::from_str(game_clock_str)?;
		let yards = Yards::from_str(game_clock_str).ok();
		let scoring_event = ScoringEvent::from_str(game_clock_str).ok();

		Ok(Self::new(game_clock, play_type, line, scoring_event, yards, team_str))
	}
}

impl fmt::Display for Play {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let scoring_event_str = if let Some(ref scoring_event) = self.scoring_event {
			format!(" Scoring Event: {}", scoring_event)
		} else {
			String::from("")
		};

		let yards_str = if let Some(ref yards) = self.yards {
			format!(" Yards: {}", yards)
		} else {
			String::from("")
		};

		let team_on_offense_str = if let Some(ref team) = self.team_on_offense {
			format!(" | Team on Offense: {}", team)
		} else {
			String::from(" | Team on Offense: Unknown")
		};

		write!(
			f,
			"Play ID: {} | Game Clock: {} | Play Type: {} | Line: {}{}{}{}",
			self.id, self.game_clock, self.play_type, self.line, team_on_offense_str, scoring_event_str, yards_str
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::schema::{Points, ScoringEventType, YardType};

	#[test]
	fn test_play_from_play_description() {
		let play_description = PlayDescription {
			team_name: Some("Atlanta Falcons".to_string()),
			headline: Some("1st & 10 at ATL 48".to_string()),
			description: Some("(12:39 - 1st) (Shotgun) B.Robinson right end to TB 18 for no gain (T.Smith).".to_string()),
		};

		let play = Play::try_from(play_description).unwrap();

		assert_eq!(play.line, DownAndDistance::from_str("1st & 10 at ATL 48").unwrap());
		assert_eq!(play.game_clock, GameClock::from_str("(12:39 - 1st)").unwrap());
		assert_eq!(play.play_type, PlayType::Run);
		assert_eq!(play.yards, Some(Yards::new(0, YardType::NoGain).unwrap()));
		assert_eq!(play.scoring_event, None);
		assert_eq!(play.team_on_offense, Some("Atlanta Falcons".to_string()));
	}

	#[test]
	fn test_scoring_play_from_play_description() {
		let play_description = PlayDescription {
			team_name: Some("Tampa Bay Buccaneers".to_string()),
			headline: Some("3rd & Goal at TB 2".to_string()),
			description: Some("(10:15 - 2nd) (Pass) T.Brady pass short right to M.Evans for 2 yards, TOUCHDOWN.".to_string()),
		};

		let play = Play::try_from(play_description).unwrap();

		assert_eq!(play.line, DownAndDistance::from_str("3rd & Goal at TB 2").unwrap());
		assert_eq!(play.game_clock, GameClock::from_str("(10:15 - 2nd)").unwrap());
		assert_eq!(play.play_type, PlayType::Pass);
		assert_eq!(play.yards, Some(Yards::new(2, YardType::Gain).unwrap()));
		assert_eq!(play.team_on_offense, Some("Tampa Bay Buccaneers".to_string()));
		assert_eq!(
			play.scoring_event,
			Some(ScoringEvent {
				event_type: ScoringEventType::Touchdown,
				points: Points::Six,
			})
		);
	}

	#[test]
	fn test_play_from_play_description_missing_headline() {
		let play_description = PlayDescription {
			team_name: Some("Green Bay Packers".to_string()),
			headline: None,
			description: Some("(5:30 - 3rd) A.Rodgers pass incomplete short left to D.Adams.".to_string()),
		};

		let result = Play::try_from(play_description);
		assert!(matches!(result, Err(PlayByPlayError::MissingHeadline)));
	}

	#[test]
	fn test_play_from_play_description_missing_description() {
		let play_description = PlayDescription {
			team_name: Some("New England Patriots".to_string()),
			headline: Some("2nd & 5 at NE 30".to_string()),
			description: None,
		};

		let result = Play::try_from(play_description);
		assert!(matches!(result, Err(PlayByPlayError::MissingDescription)));
	}

	#[test]
	fn test_play_from_play_description_missing_drive_id() {
		let play_description = PlayDescription {
			team_name: Some("Seattle Seahawks".to_string()),
			headline: Some("1st & 10 at SEA 25".to_string()),
			description: Some("(15:00 - 1st) R.Wilson pass deep left to DK.Metcalf for 35 yards (J.Ramsey).".to_string()),
		};

		let result = Play::try_from(play_description);
		assert!(matches!(result, Err(PlayByPlayError::MissingDriveId)));
	}

	#[test]
	fn test_play_from_play_description_missing_team_name() {
		let play_description = PlayDescription {
			team_name: None,
			headline: Some("4th & 1 at KC 45".to_string()),
			description: Some("(2:00 - 4th) P.Mahomes pass short middle to T.Kelce for 15 yards (M.Edwards).".to_string()),
		};

		let result = Play::try_from(play_description);
		assert!(matches!(result, Err(PlayByPlayError::MissingTeamName)));
	}
}
