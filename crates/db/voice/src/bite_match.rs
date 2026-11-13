//! The reconciler's record (paulgsc/some-ui#2081, GWB7): each game bite
//! matched to the play it was about (`bite_match`), and the scenarios home
//! mapped to bites said first (`bite_mapping`).
//!
//! Both are append-only, as the bite is: a later pass adds a row, and a
//! reader takes a bite's newest. A pass whose match for a bite is the one
//! already newest adds nothing, so passes repeated over a finished game do not
//! grow the table. Both leave with the account (`bite_match_leaves_with_account`).

use crate::{Bite, How, Mode, VoiceError};
use sqlx::SqliteConnection;

/// A game bite, with the scenario home last mapped to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameBite {
	pub subject: String,
	pub note_id: String,
	pub bite: Bite,
	/// The scenario of the bite's newest `bite_mapping`, if any.
	pub mapped: Option<String>,
}

impl GameBite {
	/// The scenario the bite is matched under: its own, or else the one
	/// mapped to it.
	#[must_use]
	pub fn scenario(&self) -> Option<&str> {
		self.bite.scenario.as_deref().or(self.mapped.as_deref())
	}
}

/// Every bite said about `game` (ESPN's event id, the note's `ref`), oldest
/// note first.
///
/// # Errors
/// [`VoiceError::Unreadable`] for a bite that does not read back as this
/// crate wrote it, [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn bites_of_game(conn: &mut SqliteConnection, game: &str) -> Result<Vec<GameBite>, VoiceError> {
	let rows = sqlx::query!(
		r#"SELECT b.subject_id AS "subject_id!", b.note_id AS "note_id!", b.scenario_id, b.how, b.offer_seq, b.live_strip,
		          b.context_line, b.watching, b.started_at, b.clock_offset_ms, b.mode, b.catalog_version,
		          (SELECT m.scenario_id FROM bite_mapping m WHERE m.subject_id = b.subject_id AND m.note_id = b.note_id
		           ORDER BY m.mapping_id DESC LIMIT 1) AS "mapped?: String"
		   FROM voice_note n JOIN game_bite b ON b.subject_id = n.subject_id AND b.note_id = n.note_id
		   WHERE n.of_kind = 'game' AND n.of_ref = ?1
		   ORDER BY n.recorded_at, n.subject_id, n.note_id"#,
		game
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			Ok(GameBite {
				subject: row.subject_id,
				note_id: row.note_id,
				bite: Bite {
					scenario: row.scenario_id,
					how: How::named(&row.how).ok_or(VoiceError::Unreadable)?,
					seq: row.offer_seq,
					strip: row.live_strip,
					context: row.context_line,
					watching: row.watching != 0,
					started: row.started_at,
					offset: row.clock_offset_ms,
					mode: Mode::named(&row.mode).ok_or(VoiceError::Unreadable)?,
					catalog: row.catalog_version,
				},
				mapped: row.mapped,
			})
		})
		.collect()
}

/// How many bites were said about `game`: what the reconciler's worker
/// compares to see whether a finished game has new ones.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn bites_in_game(conn: &mut SqliteConnection, game: &str) -> Result<i64, sqlx::Error> {
	sqlx::query_scalar!(
		r#"SELECT COUNT(*) AS "n!: i64" FROM voice_note n JOIN game_bite b ON b.subject_id = n.subject_id AND b.note_id = n.note_id
		   WHERE n.of_kind = 'game' AND n.of_ref = ?1"#,
		game
	)
	.fetch_one(&mut *conn)
	.await
}

/// How a bite was matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
	/// The nearest play the scenario's rule accepts, at or before the bite,
	/// within the window.
	Nearest,
	/// No such play: kept, and marked (GW15).
	Unmatched,
	/// The scenario is about a stretch of the game, not one play.
	NotAPlay,
	/// Said first, and nothing mapped to it yet.
	NoScenario,
}

impl Method {
	const ALL: [Self; 4] = [Self::Nearest, Self::Unmatched, Self::NotAPlay, Self::NoScenario];

	/// The stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Nearest => "nearest",
			Self::Unmatched => "unmatched",
			Self::NotAPlay => "not-a-play",
			Self::NoScenario => "no-scenario",
		}
	}

	/// The method stored as `name`.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|method| method.as_str() == name)
	}
}

/// What one pass matched a bite to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
	/// The scenario matched under; `None` exactly for [`Method::NoScenario`].
	pub scenario: Option<String>,
	/// ESPN's play id; `Some` exactly for [`Method::Nearest`].
	pub play: Option<String>,
	pub method: Method,
	/// The bite's start on home's clock minus the play's time; `Some`
	/// exactly with a play.
	pub gap_seconds: Option<i64>,
	/// The scenario engine's rules it was made under.
	pub engine_version: i64,
}

/// The bite's newest match, if any pass has made one.
///
/// # Errors
/// [`VoiceError::Unreadable`] for a row this crate did not write,
/// [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn newest_match(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Match>, VoiceError> {
	let row = sqlx::query!(
		"SELECT scenario_id, play_id, method, gap_seconds, engine_version FROM bite_match
		 WHERE subject_id = ?1 AND note_id = ?2 ORDER BY match_id DESC LIMIT 1",
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?;
	row
		.map(|row| {
			Ok(Match {
				scenario: row.scenario_id,
				play: row.play_id,
				method: Method::named(&row.method).ok_or(VoiceError::Unreadable)?,
				gap_seconds: row.gap_seconds,
				engine_version: row.engine_version,
			})
		})
		.transpose()
}

/// Add `matched` as the bite's newest match, unless it already is. Returns
/// whether a row was added.
///
/// # Errors
/// [`VoiceError::Unreadable`] for a stored match that does not read back,
/// [`VoiceError::Storage`] for any `sqlx` failure, a `CHECK` refusing an
/// inconsistent match included.
pub async fn add_match(conn: &mut SqliteConnection, subject: &str, note_id: &str, matched: &Match, now: &str) -> Result<bool, VoiceError> {
	if newest_match(conn, subject, note_id).await?.as_ref() == Some(matched) {
		return Ok(false);
	}
	let method = matched.method.as_str();
	sqlx::query!(
		"INSERT INTO bite_match (subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
		subject,
		note_id,
		matched.scenario,
		matched.play,
		method,
		matched.gap_seconds,
		matched.engine_version,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(true)
}

/// A scenario home accepted for a bite said first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
	pub scenario: String,
	/// What proposed it: `shot-writer` (GWB9), the only proposer.
	pub proposed_by: &'static str,
	/// The scenario set it was checked against.
	pub catalog_version: i64,
	/// The engine's rules the replay ran under.
	pub engine_version: i64,
}

/// Add `mapping` as the bite's newest mapping. Only an accepted proposal is
/// ever added; the caller has checked it.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure, a `CHECK` refusing an
/// unknown proposer included.
pub async fn add_mapping(conn: &mut SqliteConnection, subject: &str, note_id: &str, mapping: &Mapping, now: &str) -> Result<(), VoiceError> {
	sqlx::query!(
		"INSERT INTO bite_mapping (subject_id, note_id, scenario_id, proposed_by, catalog_version, engine_version, mapped_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		subject,
		note_id,
		mapping.scenario,
		mapping.proposed_by,
		mapping.catalog_version,
		mapping.engine_version,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{add_mapping, add_match, bites_of_game, newest_match, Mapping, Match, Method};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const NOW: &str = "2026-11-16T00:00:00.000Z";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in ["a", "b"] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, randomblob(16))")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	async fn bite(pool: &SqlitePool, subject: &str, note: &str, game: &str, scenario: Option<&str>, how: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'game', ?3, ?2, 'audio/webm', 1, 's', 'now')",
		)
		.bind(subject)
		.bind(note)
		.bind(game)
		.execute(pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO game_bite (subject_id, note_id, scenario_id, how, live_strip, watching, started_at, clock_offset_ms, mode, catalog_version)
			 VALUES (?1, ?2, ?3, ?4, '', 1, '2026-11-16T00:00:00.000Z', 250, 'standard', 1)",
		)
		.bind(subject)
		.bind(note)
		.bind(scenario)
		.bind(how)
		.execute(pool)
		.await
		.unwrap();
	}

	fn nearest(play: &str, gap: i64) -> Match {
		Match {
			scenario: Some("in.third-long".to_owned()),
			play: Some(play.to_owned()),
			method: Method::Nearest,
			gap_seconds: Some(gap),
			engine_version: 2,
		}
	}

	fn mapping(scenario: &str) -> Mapping {
		Mapping {
			scenario: scenario.to_owned(),
			proposed_by: "shot-writer",
			catalog_version: 1,
			engine_version: 2,
		}
	}

	async fn rows(pool: &SqlitePool, table: &str) -> i64 {
		sqlx::query_scalar(&["SELECT COUNT(*) FROM ", table].concat()).fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn a_games_bites_are_its_own_with_their_newest_mapping() {
		let pool = pool().await;
		bite(&pool, "a", "n1", "401", Some("in.third-long"), "offered").await;
		bite(&pool, "a", "n2", "401", None, "said-first").await;
		bite(&pool, "b", "n3", "402", Some("in.sack"), "just-happened").await;
		let mut conn = pool.acquire().await.unwrap();
		add_mapping(&mut conn, "a", "n2", &mapping("in.sack"), NOW).await.unwrap();
		add_mapping(&mut conn, "a", "n2", &mapping("in.turnover"), NOW).await.unwrap();
		let bites = bites_of_game(&mut conn, "401").await.unwrap();
		assert_eq!(bites.iter().map(|b| b.note_id.as_str()).collect::<Vec<_>>(), ["n1", "n2"]);
		assert_eq!(bites[0].scenario(), Some("in.third-long"));
		assert_eq!(bites[1].bite.scenario, None, "the bite is never rewritten");
		assert_eq!(bites[1].scenario(), Some("in.turnover"), "the newest mapping");
		assert_eq!(bites[0].bite.offset, Some(250));
	}

	#[tokio::test]
	async fn a_match_is_added_once_and_a_later_one_is_a_new_row() {
		let pool = pool().await;
		bite(&pool, "a", "n1", "401", Some("in.third-long"), "offered").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(newest_match(&mut conn, "a", "n1").await.unwrap(), None);
		assert!(add_match(&mut conn, "a", "n1", &nearest("p1", 12), NOW).await.unwrap());
		assert!(
			!add_match(&mut conn, "a", "n1", &nearest("p1", 12), NOW).await.unwrap(),
			"the same match again adds nothing"
		);
		assert!(add_match(&mut conn, "a", "n1", &nearest("p2", 40), NOW).await.unwrap());
		assert_eq!(newest_match(&mut conn, "a", "n1").await.unwrap(), Some(nearest("p2", 40)));
		drop(conn);
		assert_eq!(rows(&pool, "bite_match").await, 2);
		let refused = sqlx::query("UPDATE bite_match SET play_id = 'p3'").execute(&pool).await;
		assert!(refused.is_err(), "append-only");
	}

	#[tokio::test]
	async fn an_inconsistent_match_is_refused() {
		let pool = pool().await;
		bite(&pool, "a", "n1", "401", Some("in.third-long"), "offered").await;
		let mut conn = pool.acquire().await.unwrap();
		let mut no_play = nearest("p1", 1);
		no_play.play = None;
		no_play.gap_seconds = None;
		assert!(add_match(&mut conn, "a", "n1", &no_play, NOW).await.is_err(), "nearest names a play");
		let unmatched_with_play = Match {
			method: Method::Unmatched,
			..nearest("p1", 1)
		};
		assert!(add_match(&mut conn, "a", "n1", &unmatched_with_play, NOW).await.is_err());
		let no_scenario = Match {
			scenario: Some("in.sack".to_owned()),
			play: None,
			method: Method::NoScenario,
			gap_seconds: None,
			engine_version: 2,
		};
		assert!(add_match(&mut conn, "a", "n1", &no_scenario, NOW).await.is_err());
		let unknown = Mapping {
			proposed_by: "someone",
			..mapping("in.sack")
		};
		assert!(add_mapping(&mut conn, "a", "n1", &unknown, NOW).await.is_err());
		drop(conn);
		assert_eq!(rows(&pool, "bite_match").await + rows(&pool, "bite_mapping").await, 0);
	}

	#[tokio::test]
	async fn matches_and_mappings_leave_with_the_account() {
		let pool = pool().await;
		bite(&pool, "a", "n1", "401", None, "said-first").await;
		bite(&pool, "b", "n2", "401", Some("in.third-long"), "offered").await;
		let mut conn = pool.acquire().await.unwrap();
		add_mapping(&mut conn, "a", "n1", &mapping("in.sack"), NOW).await.unwrap();
		add_match(&mut conn, "a", "n1", &nearest("p1", 3), NOW).await.unwrap();
		add_match(&mut conn, "b", "n2", &nearest("p1", 3), NOW).await.unwrap();
		drop(conn);
		sqlx::query("DELETE FROM account WHERE subject_id = 'a'").execute(&pool).await.unwrap();
		assert_eq!((rows(&pool, "bite_match").await, rows(&pool, "bite_mapping").await), (1, 0));
	}
}
