//! Home's record of ESPN's NFL feed (paulgsc/some-ui#2076, GWB2):
//! `game_feed_response` and `game_feed_play`.
//!
//! `20261028000200_create_game_feed.up.sql` records the design; in short:
//!
//! - **Every response is kept, first.** [`keep`] stores the bytes as they
//!   arrived (a 429 and an HTML error page included), then reads them, then
//!   appends what they add to the play log, in one transaction the caller
//!   opens. Nothing else writes these tables.
//! - **Append-only.** No row is updated or deleted (the triggers abort both).
//!   A play ESPN revises is a new row superseding the one before
//!   ([`nfl_feed::log::changes`]); a play seen again unchanged adds nothing.
//! - **Replayable** (GW10). Which responses are read, and how, is one rule
//!   ([`read`]), so [`replay`] over the kept responses gives the log [`log`]
//!   reads back, `seen_at` included: it is the response's `fetched_at`.
//! - **Nobody's record.** Public game data, not subject-scoped.

use nfl_feed::{
	log::{changes, Entry, PlayLog},
	scoreboard::{self, Scoreboard},
	summary::{self, Play, Summary},
	FeedError,
};
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;
use std::collections::HashMap;

/// What was fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched<'a> {
	Scoreboard,
	/// One game's summary, by its ESPN event id.
	Summary(&'a str),
}

/// A response as it arrived.
#[derive(Debug, Clone, Copy)]
pub struct Response<'a> {
	pub fetched: Fetched<'a>,
	pub url: &'a str,
	/// ISO-8601 UTC.
	pub fetched_at: &'a str,
	pub status: u16,
	pub body: &'a [u8],
}

/// What a response said, by [`read`]'s rule.
#[derive(Debug)]
pub enum Read {
	/// A status other than 200: kept, not read.
	Status(u16),
	/// A 200 whose body this crate cannot read: kept, not read.
	Unreadable(FeedError),
	Scoreboard(Scoreboard),
	Summary(Summary),
}

impl Read {
	/// The kind, which is all a log line or a metric carries.
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::Status(429) => "rate_limited",
			Self::Status(_) => "status",
			Self::Unreadable(err) => err.kind(),
			Self::Scoreboard(_) => "scoreboard",
			Self::Summary(_) => "summary",
		}
	}
}

/// The one rule for which responses are read and how: a 200 is parsed as
/// what was fetched; anything else is not read.
#[must_use]
pub fn read(fetched: Fetched<'_>, status: u16, body: &[u8]) -> Read {
	if status != 200 {
		return Read::Status(status);
	}
	let parsed = match fetched {
		Fetched::Scoreboard => scoreboard::parse(body).map(Read::Scoreboard),
		Fetched::Summary(_) => summary::parse(body).map(Read::Summary),
	};
	parsed.unwrap_or_else(Read::Unreadable)
}

/// What [`keep`] did.
#[derive(Debug)]
pub struct Kept {
	pub response_id: i64,
	pub read: Read,
	/// Play-log rows the response added.
	pub added: usize,
}

/// Keep `response`, then read it, then append the plays it adds.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the response and the rows
/// read from it land together, and the latest rows it compares against are
/// the ones it appends after.
///
/// # Errors
/// Any `sqlx` failure; nothing is written then, once the caller rolls back.
pub async fn keep(conn: &mut SqliteConnection, response: &Response<'_>) -> Result<Kept, sqlx::Error> {
	let (kind, game_id) = match response.fetched {
		Fetched::Scoreboard => ("scoreboard", None),
		Fetched::Summary(game) => ("summary", Some(game)),
	};
	let sha256 = hex::encode(Sha256::digest(response.body));
	let size = i64::try_from(response.body.len()).unwrap_or(i64::MAX);
	let body = response.body;
	let response_id = sqlx::query!(
		"INSERT INTO game_feed_response (kind, game_id, url, fetched_at, status, sha256, size, body)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
		kind,
		game_id,
		response.url,
		response.fetched_at,
		response.status,
		sha256,
		size,
		body
	)
	.execute(&mut *conn)
	.await?
	.last_insert_rowid();
	let read = read(response.fetched, response.status, response.body);
	let added = match (&read, game_id) {
		(Read::Summary(summary), Some(game)) => append(conn, game, response_id, response.fetched_at, &summary.plays).await?,
		_ => 0,
	};
	Ok(Kept { response_id, read, added })
}

/// One stored play row, in the columns' order.
struct PlayRow {
	row_id: i64,
	game_id: String,
	espn_play_id: String,
	sequence: Option<i64>,
	wallclock: Option<String>,
	period: Option<i64>,
	clock: Option<String>,
	down: Option<i64>,
	distance: Option<i64>,
	yard_line: Option<i64>,
	yards_to_endzone: Option<i64>,
	possession: Option<String>,
	play_type: Option<String>,
	scoring: Option<bool>,
	away_score: Option<i64>,
	home_score: Option<i64>,
	text: Option<String>,
	response_id: i64,
	supersedes: Option<i64>,
	seen_at: String,
}

impl PlayRow {
	fn play(&self) -> Play {
		Play {
			id: self.espn_play_id.clone(),
			sequence: self.sequence,
			wallclock: self.wallclock.clone(),
			period: self.period,
			clock: self.clock.clone(),
			down: self.down,
			distance: self.distance,
			yard_line: self.yard_line,
			yards_to_endzone: self.yards_to_endzone,
			possession: self.possession.clone(),
			play_type: self.play_type.clone(),
			scoring: self.scoring,
			away_score: self.away_score,
			home_score: self.home_score,
			text: self.text.clone(),
		}
	}
}

/// Append what `plays` add to `game`'s log; returns how many rows.
async fn append(conn: &mut SqliteConnection, game: &str, response_id: i64, seen_at: &str, plays: &[Play]) -> Result<usize, sqlx::Error> {
	// The latest version of each of the game's plays: the rows nothing
	// supersedes.
	let rows = sqlx::query_as!(
		PlayRow,
		r#"SELECT row_id AS "row_id!", game_id, espn_play_id, sequence, wallclock, period, clock, down, distance, yard_line,
		          yards_to_endzone, possession, play_type, scoring AS "scoring: bool", away_score, home_score, text,
		          response_id, supersedes, seen_at
		   FROM game_feed_play p
		   WHERE p.game_id = ?1
		     AND NOT EXISTS (SELECT 1 FROM game_feed_play s WHERE s.supersedes = p.row_id)"#,
		game
	)
	.fetch_all(&mut *conn)
	.await?;
	let latest: HashMap<String, (i64, Play)> = rows.iter().map(|row| (row.espn_play_id.clone(), (row.row_id, row.play()))).collect();
	let added = changes(plays, |id| latest.get(id).map(|(row, play)| (*row, play)));
	for change in &added {
		let play = change.play;
		sqlx::query!(
			"INSERT INTO game_feed_play
			   (game_id, espn_play_id, sequence, wallclock, period, clock, down, distance, yard_line, yards_to_endzone,
			    possession, play_type, scoring, away_score, home_score, text, response_id, supersedes, seen_at)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
			game,
			play.id,
			play.sequence,
			play.wallclock,
			play.period,
			play.clock,
			play.down,
			play.distance,
			play.yard_line,
			play.yards_to_endzone,
			play.possession,
			play.play_type,
			play.scoring,
			play.away_score,
			play.home_score,
			play.text,
			response_id,
			change.supersedes,
			seen_at
		)
		.execute(&mut *conn)
		.await?;
	}
	Ok(added.len())
}

/// Whether the newest summary kept for `game` with a 200 says the game is
/// over, so a restarted worker need not fetch it again.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn holds_final(conn: &mut SqliteConnection, game: &str) -> Result<bool, sqlx::Error> {
	let body = sqlx::query_scalar!(
		"SELECT body FROM game_feed_response
		 WHERE game_id = ?1 AND kind = 'summary' AND status = 200
		 ORDER BY response_id DESC LIMIT 1",
		game
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(body.is_some_and(|body| match read(Fetched::Summary(game), 200, &body) {
		Read::Summary(summary) => summary.status.is_final(),
		_ => false,
	}))
}

/// The play log as stored, oldest row first, `supersedes` as a position in
/// it.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn log(conn: &mut SqliteConnection) -> Result<Vec<Entry>, sqlx::Error> {
	let rows = sqlx::query_as!(
		PlayRow,
		r#"SELECT row_id AS "row_id!", game_id, espn_play_id, sequence, wallclock, period, clock, down, distance, yard_line,
		          yards_to_endzone, possession, play_type, scoring AS "scoring: bool", away_score, home_score, text,
		          response_id, supersedes, seen_at
		   FROM game_feed_play ORDER BY row_id"#
	)
	.fetch_all(&mut *conn)
	.await?;
	let position: HashMap<i64, usize> = rows.iter().enumerate().map(|(at, row)| (row.row_id, at)).collect();
	Ok(
		rows
			.iter()
			.map(|row| Entry {
				game_id: row.game_id.clone(),
				play: row.play(),
				response_id: row.response_id,
				seen_at: row.seen_at.clone(),
				supersedes: row.supersedes.and_then(|row| position.get(&row).copied()),
			})
			.collect(),
	)
}

/// The play log rebuilt from the kept responses alone, by the rule [`keep`]
/// follows. Equal to [`log`] while both hold (GW10).
///
/// # Errors
/// Any `sqlx` failure.
pub async fn replay(conn: &mut SqliteConnection) -> Result<Vec<Entry>, sqlx::Error> {
	let responses = sqlx::query!(
		r#"SELECT response_id AS "response_id!", game_id AS "game_id!", fetched_at, status, body
		   FROM game_feed_response WHERE kind = 'summary' ORDER BY response_id"#
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut log = PlayLog::default();
	for response in responses {
		let status = u16::try_from(response.status).unwrap_or(0);
		if let Read::Summary(summary) = read(Fetched::Summary(&response.game_id), status, &response.body) {
			log.apply(&response.game_id, response.response_id, &response.fetched_at, &summary.plays);
		}
	}
	Ok(log.entries().to_vec())
}

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	const FINAL: &[u8] = include_bytes!("../../../nfl_feed/testdata/summary-401873010-final.json");
	const PRE: &[u8] = include_bytes!("../../../nfl_feed/testdata/summary-401873023-pre.json");
	const BOARD: &[u8] = include_bytes!("../../../nfl_feed/testdata/scoreboard-2026-week8.json");
	const GAME: &str = "401873010";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn summary<'a>(body: &'a [u8], at: &'static str) -> Response<'a> {
		Response {
			fetched: Fetched::Summary(GAME),
			url: "https://site.api.espn.com/apis/site/v2/sports/football/nfl/summary?event=401873010",
			fetched_at: at,
			status: 200,
			body,
		}
	}

	/// The real summary with one play revised, as ESPN does after a review.
	fn revised() -> Vec<u8> {
		let text = std::str::from_utf8(FINAL).unwrap();
		let from = "C.Santos kicks 60 yards from CHI 35 to NE 5. C.Kiner to NE 31 for 26 yards";
		assert!(text.contains(from));
		text
			.replacen(from, "C.Santos kicks 60 yards from CHI 35 to NE 5. C.Kiner to NE 33 for 28 yards", 1)
			.into_bytes()
	}

	async fn keep_one(pool: &SqlitePool, response: &Response<'_>) -> Kept {
		let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
		let kept = keep(&mut tx, response).await.unwrap();
		tx.commit().await.unwrap();
		kept
	}

	#[tokio::test]
	async fn a_game_is_logged_once_revised_by_a_new_row_and_replays_to_the_same_log() {
		let pool = pool().await;
		let revised = revised();
		let board = Response {
			fetched: Fetched::Scoreboard,
			url: "https://site.api.espn.com/apis/site/v2/sports/football/nfl/scoreboard",
			fetched_at: "2026-10-23T00:00:00.000Z",
			status: 200,
			body: BOARD,
		};
		assert!(matches!(keep_one(&pool, &board).await.read, Read::Scoreboard(_)));
		assert_eq!(keep_one(&pool, &summary(PRE, "2026-10-23T00:10:00.000Z")).await.added, 0);
		assert_eq!(keep_one(&pool, &summary(FINAL, "2026-10-23T03:30:00.000Z")).await.added, 180);
		assert_eq!(keep_one(&pool, &summary(FINAL, "2026-10-23T03:30:15.000Z")).await.added, 0);
		let limited = Response {
			status: 429,
			..summary(b"Too Many Requests", "2026-10-23T03:30:30.000Z")
		};
		assert_eq!(keep_one(&pool, &limited).await.read.kind(), "rate_limited");
		let html = summary(b"<html>busy</html>", "2026-10-23T03:30:45.000Z");
		assert_eq!(keep_one(&pool, &html).await.read.kind(), "not_json");
		assert_eq!(keep_one(&pool, &summary(&revised, "2026-10-23T03:31:00.000Z")).await.added, 1);

		let mut conn = pool.acquire().await.unwrap();
		let stored = log(&mut conn).await.unwrap();
		assert_eq!(stored.len(), 181);
		let revision = stored.last().unwrap();
		assert_eq!((revision.supersedes, revision.seen_at.as_str()), (Some(0), "2026-10-23T03:31:00.000Z"));
		assert_eq!(revision.play.id, stored[0].play.id);
		assert_eq!(stored[0].seen_at, "2026-10-23T03:30:00.000Z");
		assert_eq!(replay(&mut conn).await.unwrap(), stored);

		let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_feed_response").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(kept, 7, "every response is kept, the 429 and the HTML page included");
		assert!(holds_final(&mut conn, GAME).await.unwrap());
		assert!(!holds_final(&mut conn, "401873023").await.unwrap());
	}

	#[tokio::test]
	async fn nothing_kept_is_rewritten_or_deleted() {
		let pool = pool().await;
		keep_one(&pool, &summary(FINAL, "2026-10-23T03:30:00.000Z")).await;
		let mut conn = pool.acquire().await.unwrap();
		for statement in [
			"UPDATE game_feed_response SET status = 500",
			"DELETE FROM game_feed_response",
			"UPDATE game_feed_play SET text = 'x'",
			"DELETE FROM game_feed_play",
		] {
			let err = sqlx::query(statement).execute(&mut *conn).await.unwrap_err();
			assert!(err.to_string().contains("append-only"), "{statement}: {err}");
		}
		let foreign = sqlx::query("INSERT INTO game_feed_play (game_id, espn_play_id, response_id, supersedes, seen_at) VALUES ('401873010', 'another', 1, 1, 'now')")
			.execute(&mut *conn)
			.await
			.unwrap_err();
		assert!(foreign.to_string().contains("same play"), "{foreign}");
	}
}
