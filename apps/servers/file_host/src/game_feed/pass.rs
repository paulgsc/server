//! One pass of the game feed: fetch what is due, keep every response, then
//! read it.
//!
//! - **What is due** is [`Schedule`]'s call, in memory: the scoreboard every
//!   [`Cadence::scoreboard`]; a game's summary every [`Cadence::live`] while
//!   the game is live, or from its kickoff until a summary says it started
//!   (the scoreboard may be minutes stale at kickoff); once more after it goes
//!   final; and never again once a final summary is kept.
//! - **Kept first.** Each response is stored raw, then read, then its plays
//!   appended, in one transaction (`game_feed_repo::keep`). A fetch that got
//!   no response at all stores nothing and is logged by kind.
//! - **Backs off.** A 429, any other non-200, an unreadable body or no
//!   response pushes that target out exponentially from its own cadence, to
//!   at most [`Cadence::backoff_cap`]; its next good response resets it.
//! - **No network in tests.** Fetching is the [`Fetch`] port.

use chrono::{DateTime, Duration as Span, SecondsFormat, Utc};
use game_feed_repo::{Fetched, Read, Response};
use nfl_feed::{scoreboard::Scoreboard, GameState};
use sqlx::SqlitePool;
use std::{collections::BTreeMap, future::Future, time::Duration};

/// How long after kickoff a game the feed still calls `pre` is polled at the
/// live cadence. Past it, the game waits for the scoreboard to say it began
/// (a delayed or postponed game).
pub const PRE_GRACE: Span = Span::hours(4);

/// How long after kickoff a `post` game that is not yet final is fetched.
///
/// Its summary may lag the scoreboard, or the game was postponed (`post`,
/// not completed) and is then left. Longer than any game runs.
pub const POST_GRACE: Span = Span::hours(8);

/// What a fetch answered.
#[derive(Debug, Clone)]
pub struct Answer {
	pub status: u16,
	pub body: Vec<u8>,
}

/// A fetch that got no answer, by kind: never a URL or a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoAnswer(pub &'static str);

/// The network, as the pass sees it.
pub trait Fetch: Send + Sync {
	fn get(&self, url: &str) -> impl Future<Output = Result<Answer, NoAnswer>> + Send;
}

/// [`Fetch`] over `reqwest`.
#[derive(Debug, Clone)]
pub struct ReqwestFetch {
	client: reqwest::Client,
}

impl ReqwestFetch {
	#[must_use]
	pub const fn new(client: reqwest::Client) -> Self {
		Self { client }
	}
}

impl Fetch for ReqwestFetch {
	async fn get(&self, url: &str) -> Result<Answer, NoAnswer> {
		let kind = |err: &reqwest::Error| {
			NoAnswer(if err.is_timeout() {
				"timeout"
			} else if err.is_connect() {
				"connect"
			} else {
				"transport"
			})
		};
		let response = self.client.get(url).send().await.map_err(|err| kind(&err))?;
		let status = response.status().as_u16();
		let body = response.bytes().await.map_err(|err| kind(&err))?;
		Ok(Answer { status, body: body.to_vec() })
	}
}

/// How often each thing is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cadence {
	pub scoreboard: Duration,
	pub live: Duration,
	pub backoff_cap: Duration,
}

/// When one thing is next fetched, and how many fetches of it in a row went
/// wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
	next_at: DateTime<Utc>,
	failures: u32,
}

impl Target {
	const fn now(now: DateTime<Utc>) -> Self {
		Self { next_at: now, failures: 0 }
	}

	fn succeeded(&mut self, now: DateTime<Utc>, every: Duration) {
		self.failures = 0;
		self.next_at = now + span(every);
	}

	/// `every` doubled per failure in a row, to at most `cap`.
	fn failed(&mut self, now: DateTime<Utc>, every: Duration, cap: Duration) {
		self.failures = self.failures.saturating_add(1);
		let factor = 2_u32.saturating_pow(self.failures.min(16));
		self.next_at = now + span(every.saturating_mul(factor).min(cap));
	}
}

fn span(duration: Duration) -> Span {
	Span::from_std(duration).unwrap_or(Span::MAX)
}

/// One game the feed follows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Track {
	kickoff: Option<DateTime<Utc>>,
	state: Option<GameState>,
	/// A summary saying the game is final is kept: nothing more to fetch.
	done: bool,
	target: Target,
}

impl Track {
	/// Whether this game's summary is fetched at all, now.
	fn followed(&self, now: DateTime<Utc>) -> bool {
		match self.state {
			_ if self.done => false,
			Some(GameState::In) => true,
			Some(GameState::Post) => self.kickoff.is_some_and(|kickoff| now - kickoff < POST_GRACE),
			Some(GameState::Pre) | None => self.kickoff.is_some_and(|kickoff| kickoff <= now && now - kickoff < PRE_GRACE),
		}
	}
}

/// The later of two states: a scoreboard read before a summary never moves
/// a game back.
fn later(held: Option<GameState>, seen: Option<GameState>) -> Option<GameState> {
	let rank = |state: Option<GameState>| match state {
		None => 0,
		Some(GameState::Pre) => 1,
		Some(GameState::In) => 2,
		Some(GameState::Post) => 3,
	};
	if rank(seen) >= rank(held) {
		seen
	} else {
		held
	}
}

/// What the worker knows between passes: when to fetch what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
	scoreboard: Target,
	games: BTreeMap<String, Track>,
}

impl Schedule {
	/// Nothing known yet: the scoreboard is due at once.
	#[must_use]
	pub const fn new(now: DateTime<Utc>) -> Self {
		Self {
			scoreboard: Target::now(now),
			games: BTreeMap::new(),
		}
	}

	/// Games being followed now: live, or past kickoff and not yet final.
	#[must_use]
	pub fn followed(&self, now: DateTime<Utc>) -> usize {
		self.games.values().filter(|track| track.followed(now)).count()
	}

	/// Learn the week from a scoreboard. `done` names the games already kept
	/// final. A game no longer listed is dropped unless it is still followed.
	fn learn(&mut self, board: &Scoreboard, done: &[String], now: DateTime<Utc>) {
		for game in &board.games {
			let track = self.games.entry(game.id.clone()).or_insert_with(|| Track {
				kickoff: None,
				state: None,
				done: done.contains(&game.id),
				target: Target::now(now),
			});
			track.kickoff = game.kickoff.or(track.kickoff);
			// `pre` again after `post` is a postponed game rescheduled under
			// the same event id: follow it again from its new kickoff.
			track.state = if game.status.state == Some(GameState::Pre) && !track.done {
				game.status.state
			} else {
				later(track.state, game.status.state)
			};
		}
		self.games.retain(|id, track| board.games.iter().any(|game| &game.id == id) || track.followed(now));
	}

	fn due(&self, now: DateTime<Utc>) -> Vec<String> {
		self
			.games
			.iter()
			.filter(|(_, track)| track.followed(now) && track.target.next_at <= now)
			.map(|(id, _)| id.clone())
			.collect()
	}
}

/// What a pass did, by counts and kinds only.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// The kind of each fetch: what the response read as
	/// (`game_feed_repo::Read::kind`), or why there was none.
	pub fetched: Vec<&'static str>,
	/// Play-log rows added.
	pub added: usize,
}

/// The clock, as an ISO-8601 UTC string `fetched_at` is kept in.
fn stamp(at: DateTime<Utc>) -> String {
	at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Fetch, keep and read whatever is due at `clock()`.
///
/// # Errors
/// A storage failure, which ends the pass. What was kept before it stays
/// kept; the target it failed on is fetched again next pass.
pub async fn run_once<F: Fetch>(
	db: &SqlitePool,
	fetch: &F,
	schedule: &mut Schedule,
	cadence: Cadence,
	clock: &(dyn Fn() -> DateTime<Utc> + Sync),
) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	let now = clock();
	if schedule.scoreboard.next_at <= now {
		let url = nfl_feed::scoreboard_url();
		let read = fetch_and_keep(db, fetch, Fetched::Scoreboard, &url, clock, &mut pass).await;
		if read.is_err() {
			// A storage failure ends the pass; the target still waits, so
			// a failing store is not asked again every wake.
			schedule.scoreboard.failed(now, cadence.scoreboard, cadence.backoff_cap);
		}
		match read? {
			Some(Read::Scoreboard(board)) => {
				let mut done = Vec::new();
				let mut conn = db.acquire().await?;
				for game in board.games.iter().filter(|game| !schedule.games.contains_key(&game.id)) {
					if game_feed_repo::holds_final(&mut conn, &game.id).await? {
						done.push(game.id.clone());
					}
				}
				schedule.learn(&board, &done, now);
				schedule.scoreboard.succeeded(now, cadence.scoreboard);
			}
			_ => schedule.scoreboard.failed(now, cadence.scoreboard, cadence.backoff_cap),
		}
	}
	for game in schedule.due(now) {
		let url = nfl_feed::summary_url(&game);
		let read = fetch_and_keep(db, fetch, Fetched::Summary(&game), &url, clock, &mut pass).await;
		let Some(track) = schedule.games.get_mut(&game) else { continue };
		if read.is_err() {
			track.target.failed(now, cadence.live, cadence.backoff_cap);
		}
		match read? {
			Some(Read::Summary(summary)) => {
				track.state = later(track.state, summary.status.state);
				// The summary's own word, not the scoreboard's: the
				// scoreboard can say `post` before the summary has the
				// last plays.
				track.done = summary.status.is_final();
				track.target.succeeded(now, cadence.live);
			}
			_ => track.target.failed(now, cadence.live, cadence.backoff_cap),
		}
	}
	Ok(pass)
}

/// One fetch: kept, if anything answered, and read. `None` when nothing
/// answered.
async fn fetch_and_keep<F: Fetch>(
	db: &SqlitePool,
	fetch: &F,
	fetched: Fetched<'_>,
	url: &str,
	clock: &(dyn Fn() -> DateTime<Utc> + Sync),
	pass: &mut Pass,
) -> Result<Option<Read>, sqlx::Error> {
	let answer = match fetch.get(url).await {
		Ok(answer) => answer,
		Err(NoAnswer(kind)) => {
			pass.fetched.push(kind);
			return Ok(None);
		}
	};
	let fetched_at = stamp(clock());
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let kept = game_feed_repo::keep(
		&mut tx,
		&Response {
			fetched,
			url,
			fetched_at: &fetched_at,
			status: answer.status,
			body: &answer.body,
		},
	)
	.await?;
	tx.commit().await?;
	pass.fetched.push(kept.read.kind());
	pass.added += kept.added;
	Ok(Some(kept.read))
}

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::sqlite::SqlitePoolOptions;
	use std::sync::Mutex;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	const BOARD: &[u8] = include_bytes!("../../../../../crates/nfl_feed/testdata/scoreboard-2026-week8.json");
	const FINAL: &[u8] = include_bytes!("../../../../../crates/nfl_feed/testdata/summary-401873010-final.json");
	const PRE: &[u8] = include_bytes!("../../../../../crates/nfl_feed/testdata/summary-401873023-pre.json");

	/// The week-8 game the tests follow, and its kickoff.
	const GAME: &str = "401873023";
	const KICKOFF: &str = "2026-10-30T00:15:00Z";

	const CADENCE: Cadence = Cadence {
		scoreboard: Duration::from_secs(600),
		live: Duration::from_secs(15),
		backoff_cap: Duration::from_secs(900),
	};

	/// Canned answers by URL, each used once, in order; the URLs asked are
	/// recorded.
	#[derive(Default)]
	struct Canned {
		answers: Mutex<Vec<(String, Result<Answer, NoAnswer>)>>,
		asked: Mutex<Vec<String>>,
	}

	impl Canned {
		fn answer(&self, url: &str, status: u16, body: &[u8]) {
			self.answers.lock().unwrap().push((url.to_owned(), Ok(Answer { status, body: body.to_vec() })));
		}

		fn fail(&self, url: &str, kind: &'static str) {
			self.answers.lock().unwrap().push((url.to_owned(), Err(NoAnswer(kind))));
		}

		fn asked(&self) -> Vec<String> {
			std::mem::take(&mut *self.asked.lock().unwrap())
		}
	}

	impl Fetch for Canned {
		async fn get(&self, url: &str) -> Result<Answer, NoAnswer> {
			self.asked.lock().unwrap().push(url.to_owned());
			let mut answers = self.answers.lock().unwrap();
			let at = answers.iter().position(|(canned, _)| canned == url).unwrap_or_else(|| panic!("nothing canned for {url}"));
			answers.remove(at).1
		}
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn at(text: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
	}

	fn kickoff_plus(seconds: i64) -> DateTime<Utc> {
		at(KICKOFF) + Span::seconds(seconds)
	}

	/// The real final summary, relabelled as `GAME`'s, so one game can be
	/// followed from before kickoff to the end.
	fn final_of_game() -> Vec<u8> {
		std::str::from_utf8(FINAL).unwrap().replace("401873010", GAME).into_bytes()
	}

	/// The final summary with its status set back to live and the last
	/// `drop` plays not yet made.
	fn live_of_game(drop: usize) -> Vec<u8> {
		let mut page: serde_json::Value = serde_json::from_slice(&final_of_game()).unwrap();
		page["header"]["competitions"][0]["status"]["type"]["state"] = serde_json::json!("in");
		page["header"]["competitions"][0]["status"]["type"]["completed"] = serde_json::json!(false);
		for _ in 0..drop {
			let drives = page["drives"]["previous"].as_array_mut().unwrap();
			let plays = drives.last_mut().unwrap()["plays"].as_array_mut().unwrap();
			plays.pop();
			if plays.is_empty() {
				drives.pop();
			}
		}
		page.to_string().into_bytes()
	}

	async fn pass_at(db: &SqlitePool, fetch: &Canned, schedule: &mut Schedule, now: DateTime<Utc>) -> Pass {
		run_once(db, fetch, schedule, CADENCE, &move || now).await.unwrap()
	}

	#[tokio::test]
	async fn a_game_is_followed_from_kickoff_to_its_final_fetch_and_then_left() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let game_url = nfl_feed::summary_url(GAME);

		// An hour before kickoff: the scoreboard, and no summary.
		let mut schedule = Schedule::new(kickoff_plus(-3600));
		fetch.answer(&board_url, 200, BOARD);
		let pass = pass_at(&db, &fetch, &mut schedule, kickoff_plus(-3600)).await;
		assert_eq!(pass.fetched, ["scoreboard"]);
		assert_eq!(fetch.asked(), std::slice::from_ref(&board_url));
		assert_eq!(schedule.followed(kickoff_plus(-3600)), 0);
		// Nothing is due until the scoreboard's cadence comes round.
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff_plus(-3599)).await, Pass::default());

		// At kickoff the scoreboard still says `pre`, but the game is past
		// its kickoff: its summary is fetched, at the live cadence.
		fetch.answer(&board_url, 200, BOARD);
		fetch.answer(&game_url, 200, PRE);
		let pass = pass_at(&db, &fetch, &mut schedule, kickoff_plus(1)).await;
		assert_eq!(pass.fetched, ["scoreboard", "summary"]);
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff_plus(10)).await, Pass::default(), "not before 15 s");

		fetch.answer(&game_url, 200, &live_of_game(30));
		let pass = pass_at(&db, &fetch, &mut schedule, kickoff_plus(16)).await;
		assert_eq!((pass.fetched.as_slice(), pass.added), (&["summary"][..], 150));
		fetch.answer(&game_url, 200, &live_of_game(10));
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff_plus(31)).await.added, 20);

		// Final: one last fetch, then nothing more for this game.
		fetch.answer(&game_url, 200, &final_of_game());
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff_plus(46)).await.added, 10);
		assert_eq!(schedule.followed(kickoff_plus(61)), 0);
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff_plus(61)).await, Pass::default());
		assert!(fetch.asked().iter().all(|url| url == &board_url || url == &game_url));

		// The log the worker kept replays from its responses.
		let mut conn = db.acquire().await.unwrap();
		let stored = game_feed_repo::log(&mut conn).await.unwrap();
		assert_eq!(stored.len(), 180);
		assert_eq!(game_feed_repo::replay(&mut conn).await.unwrap(), stored);
	}

	#[tokio::test]
	async fn a_restart_does_not_fetch_a_game_it_already_holds_final() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let mut page: serde_json::Value = serde_json::from_slice(BOARD).unwrap();
		page["events"][0]["status"]["type"]["state"] = serde_json::json!("post");
		page["events"][0]["status"]["type"]["completed"] = serde_json::json!(true);
		let final_board = page.to_string();

		let mut first = Schedule::new(kickoff_plus(0));
		fetch.answer(&board_url, 200, final_board.as_bytes());
		fetch.answer(&nfl_feed::summary_url(GAME), 200, &final_of_game());
		assert_eq!(pass_at(&db, &fetch, &mut first, kickoff_plus(0)).await.fetched, ["scoreboard", "summary"]);

		let mut restarted = Schedule::new(kickoff_plus(60));
		fetch.answer(&board_url, 200, final_board.as_bytes());
		assert_eq!(pass_at(&db, &fetch, &mut restarted, kickoff_plus(60)).await.fetched, ["scoreboard"]);
	}

	#[tokio::test]
	async fn a_refusal_or_no_answer_backs_off_to_the_cap_and_a_good_answer_resets_it() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let start = kickoff_plus(-86_400);
		let mut schedule = Schedule::new(start);

		fetch.answer(&board_url, 429, b"Too Many Requests");
		assert_eq!(pass_at(&db, &fetch, &mut schedule, start).await.fetched, ["rate_limited"]);
		// 600 s doubled once is past the 900 s cap.
		assert_eq!(schedule.scoreboard.next_at, start + Span::seconds(900));
		fetch.fail(&board_url, "timeout");
		assert_eq!(pass_at(&db, &fetch, &mut schedule, start + Span::seconds(900)).await.fetched, ["timeout"]);
		assert_eq!(schedule.scoreboard.next_at, start + Span::seconds(1800));
		fetch.answer(&board_url, 200, b"<html>maintenance</html>");
		assert_eq!(pass_at(&db, &fetch, &mut schedule, start + Span::seconds(1800)).await.fetched, ["not_json"]);
		assert_eq!(schedule.scoreboard.failures, 3);
		fetch.answer(&board_url, 200, BOARD);
		pass_at(&db, &fetch, &mut schedule, start + Span::seconds(2700)).await;
		assert_eq!((schedule.scoreboard.failures, schedule.scoreboard.next_at), (0, start + Span::seconds(3300)));

		// A live summary backs off from the live cadence: 30 s, then 60 s.
		let mut target = Target::now(start);
		target.failed(start, CADENCE.live, CADENCE.backoff_cap);
		assert_eq!(target.next_at, start + Span::seconds(30));
		target.failed(start, CADENCE.live, CADENCE.backoff_cap);
		assert_eq!(target.next_at, start + Span::seconds(60));

		// Every answer was kept, the refusals included; the timeout had none.
		let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_feed_response").fetch_one(&db).await.unwrap();
		assert_eq!(kept, 3);
	}

	#[tokio::test]
	async fn a_game_is_left_only_once_its_own_summary_is_final() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let game_url = nfl_feed::summary_url(GAME);
		let mut page: serde_json::Value = serde_json::from_slice(BOARD).unwrap();
		page["events"][0]["status"]["type"]["state"] = serde_json::json!("post");
		page["events"][0]["status"]["type"]["completed"] = serde_json::json!(true);
		let final_board = page.to_string();

		// The scoreboard says final while the summary is still live: the
		// game is fetched again, and left only once its summary is final.
		let mut schedule = Schedule::new(kickoff_plus(10_000));
		fetch.answer(&board_url, 200, final_board.as_bytes());
		fetch.answer(&game_url, 200, &live_of_game(10));
		pass_at(&db, &fetch, &mut schedule, kickoff_plus(10_000)).await;
		assert_eq!(schedule.followed(kickoff_plus(10_015)), 1);
		let mut conn = db.acquire().await.unwrap();
		assert!(!game_feed_repo::holds_final(&mut conn, GAME).await.unwrap(), "a live summary is not final");
		drop(conn);
		fetch.answer(&game_url, 200, &final_of_game());
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff_plus(10_015)).await.added, 10);
		assert_eq!(schedule.followed(kickoff_plus(10_030)), 0);
	}

	#[tokio::test]
	async fn a_postponed_game_is_not_final_and_is_followed_again_once_rescheduled() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let game_url = nfl_feed::summary_url(GAME);
		let board_with = |state: &str, date: &str| {
			let mut page: serde_json::Value = serde_json::from_slice(BOARD).unwrap();
			page["events"][0]["status"]["type"]["state"] = serde_json::json!(state);
			page["events"][0]["status"]["type"]["completed"] = serde_json::json!(false);
			page["events"][0]["date"] = serde_json::json!(date);
			page.to_string()
		};
		let mut postponed: serde_json::Value = serde_json::from_slice(PRE).unwrap();
		postponed["header"]["competitions"][0]["status"]["type"]["state"] = serde_json::json!("post");

		let mut schedule = Schedule::new(kickoff_plus(60));
		fetch.answer(&board_url, 200, board_with("post", "2026-10-30T00:15Z").as_bytes());
		fetch.answer(&game_url, 200, postponed.to_string().as_bytes());
		pass_at(&db, &fetch, &mut schedule, kickoff_plus(60)).await;
		let mut conn = db.acquire().await.unwrap();
		assert!(!game_feed_repo::holds_final(&mut conn, GAME).await.unwrap(), "postponed is not final");
		drop(conn);

		// Two days on, the scoreboard has it again, before a new kickoff.
		let later_kickoff = at(KICKOFF) + Span::days(2);
		fetch.answer(&board_url, 200, board_with("pre", "2026-11-01T00:15Z").as_bytes());
		pass_at(&db, &fetch, &mut schedule, later_kickoff - Span::hours(1)).await;
		assert_eq!(schedule.followed(later_kickoff - Span::hours(1)), 0, "not before its new kickoff");
		assert_eq!(schedule.followed(later_kickoff + Span::seconds(1)), 1);
	}

	#[test]
	fn a_postponed_game_is_left_after_the_post_grace() {
		let track = Track {
			kickoff: Some(at(KICKOFF)),
			state: Some(GameState::Post),
			done: false,
			target: Target::now(at(KICKOFF)),
		};
		assert!(track.followed(kickoff_plus(60)));
		assert!(!track.followed(at(KICKOFF) + POST_GRACE));
	}

	#[test]
	fn a_game_long_past_kickoff_still_called_pre_waits_for_the_scoreboard() {
		let track = Track {
			kickoff: Some(at(KICKOFF)),
			state: Some(GameState::Pre),
			done: false,
			target: Target::now(at(KICKOFF)),
		};
		assert!(!track.followed(kickoff_plus(-1)));
		assert!(track.followed(kickoff_plus(0)));
		assert!(!track.followed(at(KICKOFF) + PRE_GRACE));
		assert_eq!(later(Some(GameState::In), Some(GameState::Pre)), Some(GameState::In));
	}
}
