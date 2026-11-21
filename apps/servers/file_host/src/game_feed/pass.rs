//! One pass of the game feed: fetch what is due, keep every response, then
//! read it.
//!
//! - **What is due** is [`Schedule`]'s call, in memory: the scoreboard as
//!   its [`Watch`] says (every [`Cadence::scoreboard`] while a chosen game is
//!   near, else asleep until the next chosen kickoff less
//!   [`Cadence::margin`], or [`Cadence::refresh`] after the last read,
//!   whichever is sooner); a chosen game's summary every [`Cadence::live`] while
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

use crate::settings::Teams;
use chrono::{DateTime, Duration as Span, SecondsFormat, Utc};
use game_feed_repo::{Fetched, Read, Response};
use nfl_feed::{scoreboard::Scoreboard, GameState};
use sqlx::SqlitePool;
use std::{collections::BTreeMap, future::Future, time::Duration};

/// How long after kickoff a game the feed still calls `pre` is polled at the
/// live cadence. Past it, the game waits for the scoreboard to say it began
/// (a delayed or postponed game).
pub const PRE_GRACE: Span = Span::hours(4);

/// How long after kickoff a `post` game the scoreboard does not call
/// completed is fetched.
///
/// Such a game was postponed or cancelled; it is left after this, and
/// followed again if the scoreboard says `pre` once more.
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

	/// [`Fetch::get`] for a reader that wants only a 200's body: a 200 whose
	/// body passes `max` bytes is refused as `too_big`, and another status's
	/// body may be left unread.
	///
	/// This default reads the whole answer first, which suits a fake.
	/// [`ReqwestFetch`] stops reading once the body passes `max`, so an answer
	/// over the cap costs home at most `max` bytes and one chunk.
	fn get_at_most(&self, url: &str, max: usize) -> impl Future<Output = Result<Answer, NoAnswer>> + Send {
		async move {
			let answer = self.get(url).await?;
			if answer.status == 200 && answer.body.len() > max {
				return Err(NoAnswer(TOO_BIG));
			}
			Ok(answer)
		}
	}
}

/// The kind of an answer refused for passing its cap.
const TOO_BIG: &str = "too_big";

/// Fetches `url`, at most `max` bytes.
///
/// Its body when it answered 200 within `max`, else the failure's kind:
/// `too_big`, `refused` (a status other than 200), or the [`NoAnswer`]'s.
/// The keepers of `paulgsc/testing`'s files share it (`call_weeks::pass`,
/// `pd_fills::pass`).
///
/// # Errors
/// The failure's kind, never a URL or a body.
pub async fn fetch_capped<F: Fetch>(fetch: &F, url: &str, max: usize) -> Result<Vec<u8>, &'static str> {
	match fetch.get_at_most(url, max).await {
		Ok(Answer { status: 200, body }) => Ok(body),
		Ok(_) => Err("refused"),
		Err(no_answer) => Err(no_answer.0),
	}
}

/// [`Fetch`] over `reqwest`. [`Fetch::get`] reads a whole answer, whatever
/// its status: the game feed keeps every response raw, uncapped (GA3).
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

/// A `reqwest` failure's kind.
fn no_answer(err: &reqwest::Error) -> NoAnswer {
	NoAnswer(if err.is_timeout() {
		"timeout"
	} else if err.is_connect() {
		"connect"
	} else {
		"transport"
	})
}

impl ReqwestFetch {
	/// `url`'s answer, read as [`read_body`] does.
	async fn read(&self, url: &str, max: usize, only_ok: bool) -> Result<Answer, NoAnswer> {
		let response = self.client.get(url).send().await.map_err(|err| no_answer(&err))?;
		read_body(response, max, only_ok).await
	}
}

/// An answer with its body read in chunks to at most `max` bytes; with
/// `only_ok`, another status's body is left unread.
async fn read_body(mut response: reqwest::Response, max: usize, only_ok: bool) -> Result<Answer, NoAnswer> {
	let status = response.status().as_u16();
	if only_ok && status != 200 {
		return Ok(Answer { status, body: Vec::new() });
	}
	// A declared length over the cap is refused before any of the body.
	if response.content_length().is_some_and(|length| usize::try_from(length).map_or(true, |length| length > max)) {
		return Err(NoAnswer(TOO_BIG));
	}
	let mut body = Vec::new();
	while let Some(chunk) = response.chunk().await.map_err(|err| no_answer(&err))? {
		if body.len().saturating_add(chunk.len()) > max {
			return Err(NoAnswer(TOO_BIG));
		}
		body.extend_from_slice(&chunk);
	}
	Ok(Answer { status, body })
}

impl Fetch for ReqwestFetch {
	async fn get(&self, url: &str) -> Result<Answer, NoAnswer> {
		self.read(url, usize::MAX, false).await
	}

	async fn get_at_most(&self, url: &str, max: usize) -> Result<Answer, NoAnswer> {
		self.read(url, max, true).await
	}
}

/// How often each thing is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cadence {
	/// The scoreboard while a chosen game is near ([`Watch::Awake`]).
	pub scoreboard: Duration,
	pub live: Duration,
	pub backoff_cap: Duration,
	/// How long before a chosen game's kickoff the scoreboard wakes.
	pub margin: Duration,
	/// The longest the scoreboard sleeps between reads, so a game added,
	/// moved or flexed into the week is learnt within it.
	pub refresh: Duration,
}

/// Where the scoreboard stands between reads: the feed's states, one of
/// which holds after every pass ([`Schedule::watch`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watch {
	/// Not read yet, or a chosen game is live, past kickoff and not final, or
	/// within [`Cadence::margin`] of kickoff: next read at `at`, one
	/// [`Cadence::scoreboard`] after the last (at once before the first).
	Awake { at: DateTime<Utc> },
	/// No chosen game near: next read at `until`, the next chosen kickoff
	/// less [`Cadence::margin`] or [`Cadence::refresh`] after the last read,
	/// whichever is sooner, and never sooner than [`Watch::Awake`] would be.
	Asleep { until: DateTime<Utc> },
	/// The last read failed, the first included: next read at `at`, its
	/// backoff, whatever the games.
	BackingOff { at: DateTime<Utc> },
}

impl Watch {
	/// When the scoreboard is next read.
	#[must_use]
	pub const fn due_at(self) -> DateTime<Utc> {
		match self {
			Self::Awake { at } | Self::BackingOff { at } => at,
			Self::Asleep { until } => until,
		}
	}
}

/// When one thing is next fetched, and how many fetches of it in a row went
/// wrong. `call_weeks` schedules its pass with it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Target {
	next_at: DateTime<Utc>,
	failures: u32,
}

impl Target {
	pub(crate) const fn now(now: DateTime<Utc>) -> Self {
		Self { next_at: now, failures: 0 }
	}

	/// Whether it is due at `now`.
	pub(crate) fn due(&self, now: DateTime<Utc>) -> bool {
		self.next_at <= now
	}

	pub(crate) fn succeeded(&mut self, now: DateTime<Utc>, every: Duration) {
		self.failures = 0;
		self.next_at = now + span(every);
	}

	/// `every` doubled per failure in a row, to at most `cap`.
	pub(crate) fn failed(&mut self, now: DateTime<Utc>, every: Duration, cap: Duration) {
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
	/// Its sides' teams, home first, as the scoreboard spells them.
	teams: [Option<String>; 2],
	state: Option<GameState>,
	/// The scoreboard says the game is final (`post` and completed).
	completed: bool,
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
			// A completed game is fetched until its final summary is kept,
			// however late home first sees it; one not completed
			// (postponed or cancelled) only for the grace.
			// A game called off before its kickoff waits for it, so one
			// postponed days ahead does not keep the feed awake until then.
			Some(GameState::Post) => self.completed || self.kickoff.is_some_and(|kickoff| kickoff <= now && now - kickoff < POST_GRACE),
			Some(GameState::Pre) | None => self.kickoff.is_some_and(|kickoff| kickoff <= now && now - kickoff < PRE_GRACE),
		}
	}

	/// Whether `teams` choose this game.
	fn chosen(&self, teams: &Teams) -> bool {
		let [home, away] = &self.teams;
		teams.follows(home.as_deref(), away.as_deref())
	}

	/// Whether the scoreboard stays awake for it: followed now, or not yet
	/// final and within `margin` of its kickoff.
	fn near(&self, now: DateTime<Utc>, margin: Span) -> bool {
		self.followed(now) || (!self.done && self.kickoff.is_some_and(|kickoff| now < kickoff && kickoff - margin <= now))
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
	/// When the scoreboard was last read well: `None` before the first.
	read_at: Option<DateTime<Utc>>,
	games: BTreeMap<String, Track>,
}

impl Schedule {
	/// Nothing known yet: the scoreboard is due at once.
	#[must_use]
	pub const fn new(now: DateTime<Utc>) -> Self {
		Self {
			scoreboard: Target::now(now),
			read_at: None,
			games: BTreeMap::new(),
		}
	}

	/// Chosen games being followed now: live, or past kickoff and not yet
	/// final.
	#[must_use]
	pub fn followed(&self, now: DateTime<Utc>, teams: &Teams) -> usize {
		self.chosen(teams).filter(|track| track.followed(now)).count()
	}

	fn chosen<'a>(&'a self, teams: &'a Teams) -> impl Iterator<Item = &'a Track> + 'a {
		self.games.values().filter(move |track| track.chosen(teams))
	}

	/// Where the scoreboard stands at `now`, for the games `teams` choose.
	///
	/// Worked out from what the schedule holds each time it is asked, never
	/// stored, so a change of `teams` moves it at the next pass.
	#[must_use]
	pub fn watch(&self, now: DateTime<Utc>, teams: &Teams, cadence: Cadence) -> Watch {
		let at = self.scoreboard.next_at;
		if self.scoreboard.failures > 0 {
			return Watch::BackingOff { at };
		}
		let Some(read_at) = self.read_at else { return Watch::Awake { at } };
		let margin = span(cadence.margin);
		if self.chosen(teams).any(|track| track.near(now, margin)) {
			return Watch::Awake { at };
		}
		let next_kickoff = self
			.chosen(teams)
			.filter(|track| !track.done)
			.filter_map(|track| track.kickoff)
			.filter(|kickoff| now < *kickoff)
			.min();
		let refresh = read_at + span(cadence.refresh);
		let until = next_kickoff.map_or(refresh, |kickoff| (kickoff - margin).min(refresh));
		Watch::Asleep { until: until.max(at) }
	}

	/// Learn the week from a scoreboard. `done` names the games already kept
	/// final. A game no longer listed is dropped unless it is still followed
	/// and chosen: a game no team chose is never fetched, so never `done`, and
	/// would otherwise be kept, and fetched all at once if every game were
	/// chosen later.
	fn learn(&mut self, board: &Scoreboard, done: &[String], now: DateTime<Utc>, teams: &Teams) {
		for game in &board.games {
			let track = self.games.entry(game.id.clone()).or_insert_with(|| Track {
				kickoff: None,
				teams: [None, None],
				state: None,
				completed: false,
				done: done.contains(&game.id),
				target: Target::now(now),
			});
			track.kickoff = game.kickoff.or(track.kickoff);
			track.teams = [game.home.team.clone(), game.away.team.clone()];
			track.completed = game.status.is_final();
			// `pre` again after `post` is a postponed game rescheduled under
			// the same event id: follow it again from its new kickoff.
			track.state = if game.status.state == Some(GameState::Pre) && !track.done {
				game.status.state
			} else {
				later(track.state, game.status.state)
			};
		}
		self
			.games
			.retain(|id, track| board.games.iter().any(|game| &game.id == id) || (track.chosen(teams) && track.followed(now)));
	}

	fn due(&self, now: DateTime<Utc>, teams: &Teams) -> Vec<String> {
		self
			.games
			.iter()
			.filter(|(_, track)| track.chosen(teams) && track.followed(now) && track.target.next_at <= now)
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

/// Fetch, keep and read whatever is due at `clock()`, for the games `teams`
/// choose.
///
/// # Errors
/// A storage failure, which ends the pass. What was kept before it stays
/// kept; the target it failed on is fetched again next pass.
pub async fn run_once<F: Fetch>(
	db: &SqlitePool,
	fetch: &F,
	schedule: &mut Schedule,
	cadence: Cadence,
	teams: &Teams,
	clock: &(dyn Fn() -> DateTime<Utc> + Sync),
) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	let now = clock();
	if schedule.watch(now, teams, cadence).due_at() <= now {
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
				schedule.learn(&board, &done, now, teams);
				schedule.scoreboard.succeeded(now, cadence.scoreboard);
				schedule.read_at = Some(now);
			}
			_ => schedule.scoreboard.failed(now, cadence.scoreboard, cadence.backoff_cap),
		}
	}
	for game in schedule.due(now, teams) {
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
		margin: Duration::from_secs(3600),
		refresh: Duration::from_secs(86_400),
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

	/// An answer of `status` whose body is `body`, its length known.
	fn answer(status: u16, body: Vec<u8>) -> reqwest::Response {
		reqwest::Response::from(axum::http::Response::builder().status(status).body(body).unwrap())
	}

	/// A body that never ends and has no length: 64 KiB chunks, each counted
	/// in `pulled` as it is read.
	struct Unending {
		pulled: std::sync::Arc<std::sync::atomic::AtomicUsize>,
		/// A length it declares, though it never ends.
		declared: Option<u64>,
	}

	impl http_body::Body for Unending {
		type Data = axum::body::Bytes;
		type Error = std::convert::Infallible;

		fn poll_frame(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
			self.pulled.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
			std::task::Poll::Ready(Some(Ok(http_body::Frame::data(axum::body::Bytes::from(vec![b' '; 0x10000])))))
		}

		fn size_hint(&self) -> http_body::SizeHint {
			self.declared.map_or_else(http_body::SizeHint::default, http_body::SizeHint::with_exact)
		}
	}

	fn unending(status: u16, pulled: &std::sync::Arc<std::sync::atomic::AtomicUsize>, declared: Option<u64>) -> reqwest::Response {
		let body = reqwest::Body::wrap(Unending {
			pulled: std::sync::Arc::clone(pulled),
			declared,
		});
		reqwest::Response::from(axum::http::Response::builder().status(status).body(body).unwrap())
	}

	const MIB: usize = 1_048_576;

	#[tokio::test]
	async fn an_answer_within_the_cap_is_read_whole() {
		let read = read_body(answer(200, b"hello".to_vec()), 5, true).await;
		assert_eq!(read.map(|a| a.body), Ok(b"hello".to_vec()));
	}

	#[tokio::test]
	async fn a_declared_length_over_the_cap_is_refused_before_its_body() {
		let pulled = std::sync::Arc::default();
		let read = read_body(unending(200, &pulled, Some(u64::try_from(MIB).unwrap() + 1)), MIB, true).await;
		assert_eq!(read.map(|a| a.status), Err(NoAnswer("too_big")));
		assert_eq!(pulled.load(std::sync::atomic::Ordering::SeqCst), 0, "none of its body is read");
	}

	/// An answer with no length that never ends: the read stops at the chunk
	/// that passes the cap, 1 MiB and one chunk in.
	#[tokio::test]
	async fn an_unending_answer_is_refused_once_it_passes_the_cap() {
		let pulled = std::sync::Arc::default();
		let read = tokio::time::timeout(Duration::from_secs(10), read_body(unending(200, &pulled, None), MIB, true)).await;
		assert_eq!(read.map(|r| r.map(|a| a.status)), Ok(Err(NoAnswer("too_big"))));
		assert_eq!(pulled.load(std::sync::atomic::Ordering::SeqCst), MIB / 0x10000 + 1);
	}

	/// Another status is a refusal whatever its size, and its body is not read.
	#[tokio::test]
	async fn a_refusal_over_the_cap_is_still_a_refusal() {
		let declared = read_body(answer(503, vec![b' '; MIB + 1]), MIB, true).await;
		assert_eq!(declared.map(|a| (a.status, a.body.len())), Ok((503, 0)));
		let pulled = std::sync::Arc::default();
		let unread = read_body(unending(404, &pulled, None), MIB, true).await;
		assert_eq!(unread.map(|a| (a.status, a.body.len())), Ok((404, 0)));
		assert_eq!(pulled.load(std::sync::atomic::Ordering::SeqCst), 0, "its body is never read");
		let refused = Canned::default();
		refused.answers.lock().unwrap().push((
			String::from("x"),
			Ok(Answer {
				status: 503,
				body: vec![b' '; MIB + 1],
			}),
		));
		assert_eq!(fetch_capped(&refused, "x", MIB).await, Err("refused"));
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
		run_once(db, fetch, schedule, CADENCE, &Teams::default(), &move || now).await.unwrap()
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
		assert_eq!(schedule.followed(kickoff_plus(-3600), &Teams::default()), 0);
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
		assert_eq!(schedule.followed(kickoff_plus(61), &Teams::default()), 0);
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
		assert_eq!(schedule.followed(kickoff_plus(10_015), &Teams::default()), 1);
		let mut conn = db.acquire().await.unwrap();
		assert!(!game_feed_repo::holds_final(&mut conn, GAME).await.unwrap(), "a live summary is not final");
		drop(conn);
		fetch.answer(&game_url, 200, &final_of_game());
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff_plus(10_015)).await.added, 10);
		assert_eq!(schedule.followed(kickoff_plus(10_030), &Teams::default()), 0);
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
		assert_eq!(schedule.followed(later_kickoff - Span::hours(1), &Teams::default()), 0, "not before its new kickoff");
		assert_eq!(schedule.followed(later_kickoff + Span::seconds(1), &Teams::default()), 1);
	}

	#[tokio::test]
	async fn a_completed_game_first_seen_long_after_is_still_fetched() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let mut page: serde_json::Value = serde_json::from_slice(BOARD).unwrap();
		page["events"][0]["status"]["type"]["state"] = serde_json::json!("post");
		page["events"][0]["status"]["type"]["completed"] = serde_json::json!(true);
		let a_day_on = at(KICKOFF) + Span::days(1);
		let mut schedule = Schedule::new(a_day_on);
		fetch.answer(&board_url, 200, page.to_string().as_bytes());
		fetch.answer(&nfl_feed::summary_url(GAME), 200, &final_of_game());
		assert_eq!(pass_at(&db, &fetch, &mut schedule, a_day_on).await.fetched, ["scoreboard", "summary"]);
		assert_eq!(schedule.followed(a_day_on + Span::seconds(15), &Teams::default()), 0);
	}

	/// A scoreboard of the games given, each `(id, kickoff, state, home,
	/// away)`, none completed.
	fn board(games: &[(&str, &str, &str, &str, &str)]) -> Vec<u8> {
		let events: Vec<serde_json::Value> = games
			.iter()
			.map(|(id, date, state, home, away)| {
				serde_json::json!({
					"id": id,
					"date": date,
					"status": { "type": { "state": state, "completed": false } },
					"competitions": [{ "competitors": [
						{ "homeAway": "home", "team": { "abbreviation": home } },
						{ "homeAway": "away", "team": { "abbreviation": away } },
					] }],
				})
			})
			.collect();
		serde_json::json!({ "events": events }).to_string().into_bytes()
	}

	fn teams(named: &[&str]) -> Teams {
		Teams::try_from(named.iter().map(|team| (*team).to_owned()).collect::<Vec<_>>()).unwrap()
	}

	async fn pass_for(db: &SqlitePool, fetch: &Canned, schedule: &mut Schedule, now: DateTime<Utc>, chosen: &Teams) -> Pass {
		run_once(db, fetch, schedule, CADENCE, chosen, &move || now).await.unwrap()
	}

	/// A start on a Tuesday with no game in the week.
	const TUESDAY: &str = "2026-11-03T15:00:00Z";

	#[tokio::test]
	async fn a_week_with_no_game_reads_the_scoreboard_once_a_day_and_nothing_else() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let start = at(TUESDAY);
		for _ in 0..8 {
			fetch.answer(&board_url, 200, &board(&[]));
		}
		let mut schedule = Schedule::new(start);
		let mut asked = 0;
		// A wake every 5 minutes for a week: one read a day.
		let mut now = start;
		while now < start + Span::days(7) {
			let pass = pass_at(&db, &fetch, &mut schedule, now).await;
			asked += pass.fetched.len();
			if !pass.fetched.is_empty() {
				assert_eq!(pass.fetched, ["scoreboard"]);
				assert_eq!(schedule.watch(now, &Teams::default(), CADENCE), Watch::Asleep { until: now + Span::days(1) });
			}
			now += Span::minutes(5);
		}
		assert_eq!(asked, 7);
		assert!(fetch.asked().iter().all(|url| url == &board_url));
	}

	#[tokio::test]
	async fn a_game_tomorrow_wakes_the_scoreboard_an_hour_before_its_kickoff() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let start = at(TUESDAY);
		let kickoff = start + Span::hours(20);
		let week = board(&[(GAME, &stamp(kickoff), "pre", "SF", "SEA")]);
		let mut schedule = Schedule::new(start);
		fetch.answer(&board_url, 200, &week);
		pass_at(&db, &fetch, &mut schedule, start).await;
		let asleep = Watch::Asleep { until: kickoff - Span::hours(1) };
		assert_eq!(schedule.watch(start, &Teams::default(), CADENCE), asleep);
		assert_eq!(asleep.due_at(), kickoff - Span::hours(1));
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff - Span::hours(2)).await, Pass::default());

		// Awake from an hour before: the scoreboard at its cadence, then the
		// summary from kickoff.
		fetch.answer(&board_url, 200, &week);
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff - Span::hours(1)).await.fetched, ["scoreboard"]);
		let next = kickoff - Span::hours(1) + Span::minutes(10);
		assert_eq!(schedule.watch(next, &Teams::default(), CADENCE), Watch::Awake { at: next });
		fetch.answer(&board_url, 200, &week);
		fetch.answer(&nfl_feed::summary_url(GAME), 200, PRE);
		assert_eq!(pass_at(&db, &fetch, &mut schedule, kickoff).await.fetched, ["scoreboard", "summary"]);
	}

	#[tokio::test]
	async fn a_flexed_kickoff_is_learnt_at_the_read_before_the_old_one_or_the_daily_read() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let start = at(TUESDAY);

		// Flexed later: read an hour before the old kickoff, which says the
		// new one, so nothing is fetched at the old kickoff.
		let old = start + Span::hours(10);
		let new = old + Span::hours(7);
		let mut schedule = Schedule::new(start);
		fetch.answer(&board_url, 200, &board(&[(GAME, &stamp(old), "pre", "SF", "SEA")]));
		pass_at(&db, &fetch, &mut schedule, start).await;
		fetch.answer(&board_url, 200, &board(&[(GAME, &stamp(new), "pre", "SF", "SEA")]));
		assert_eq!(pass_at(&db, &fetch, &mut schedule, old - Span::hours(1)).await.fetched, ["scoreboard"]);
		assert_eq!(schedule.watch(old, &Teams::default(), CADENCE), Watch::Asleep { until: new - Span::hours(1) });
		assert_eq!(pass_at(&db, &fetch, &mut schedule, old).await, Pass::default());

		// Moved sooner: five days out at first, then the daily read says
		// tomorrow.
		let far = start + Span::days(5);
		let sooner = start + Span::days(1) + Span::hours(10);
		let mut schedule = Schedule::new(start);
		fetch.answer(&board_url, 200, &board(&[(GAME, &stamp(far), "pre", "SF", "SEA")]));
		pass_at(&db, &fetch, &mut schedule, start).await;
		assert_eq!(schedule.watch(start, &Teams::default(), CADENCE), Watch::Asleep { until: start + Span::days(1) });
		fetch.answer(&board_url, 200, &board(&[(GAME, &stamp(sooner), "pre", "SF", "SEA")]));
		pass_at(&db, &fetch, &mut schedule, start + Span::days(1)).await;
		let read = start + Span::days(1);
		assert_eq!(schedule.watch(read, &Teams::default(), CADENCE), Watch::Asleep { until: sooner - Span::hours(1) });
	}

	#[tokio::test]
	async fn a_postponed_game_sleeps_the_scoreboard_until_the_daily_read_learns_its_new_kickoff() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let kickoff = at(TUESDAY) + Span::days(4);
		let stuck = board(&[(GAME, &stamp(kickoff), "pre", "SF", "SEA")]);

		// Still `pre` past its grace: no summary, and the scoreboard asleep
		// until the day after its last read.
		let past = kickoff + PRE_GRACE;
		let mut schedule = Schedule::new(past);
		fetch.answer(&board_url, 200, &stuck);
		assert_eq!(pass_at(&db, &fetch, &mut schedule, past).await.fetched, ["scoreboard"]);
		assert_eq!(schedule.watch(past, &Teams::default(), CADENCE), Watch::Asleep { until: past + Span::days(1) });

		// The daily read has it rescheduled two days on.
		let again = kickoff + Span::days(2);
		fetch.answer(&board_url, 200, &board(&[(GAME, &stamp(again), "pre", "SF", "SEA")]));
		pass_at(&db, &fetch, &mut schedule, past + Span::days(1)).await;
		let read = past + Span::days(1);
		assert_eq!(schedule.watch(read, &Teams::default(), CADENCE), Watch::Asleep { until: again - Span::hours(1) });
	}

	#[tokio::test]
	async fn only_a_chosen_teams_games_are_fetched_or_wake_the_scoreboard() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let start = at(TUESDAY);
		let kickoff = start + Span::hours(5);
		let week = board(&[
			(GAME, &stamp(kickoff), "pre", "SF", "SEA"),
			("401873099", &stamp(kickoff - Span::hours(3)), "pre", "GB", "CAR"),
		]);
		let (sf, dallas) = (teams(&["SF"]), teams(&["DAL"]));

		// A team with no game this week: asleep until the daily read, though
		// two games kick off today.
		let mut schedule = Schedule::new(start);
		fetch.answer(&board_url, 200, &week);
		pass_for(&db, &fetch, &mut schedule, start, &dallas).await;
		assert_eq!(schedule.watch(start, &dallas, CADENCE), Watch::Asleep { until: start + Span::days(1) });
		// Every game, or SF: the earlier GB game, or SF's, wakes it.
		assert_eq!(schedule.watch(start, &Teams::default(), CADENCE), Watch::Asleep { until: kickoff - Span::hours(4) });
		assert_eq!(schedule.watch(start, &sf, CADENCE), Watch::Asleep { until: kickoff - Span::hours(1) });

		// Choosing SF takes effect at the next pass, and at SF's kickoff only
		// SF's summary is fetched, the GB game live or not.
		fetch.answer(&board_url, 200, &week);
		fetch.answer(&nfl_feed::summary_url(GAME), 200, PRE);
		assert_eq!(pass_for(&db, &fetch, &mut schedule, kickoff, &sf).await.fetched, ["scoreboard", "summary"]);
		assert_eq!(fetch.asked(), [board_url.clone(), board_url.clone(), nfl_feed::summary_url(GAME)]);
		assert_eq!(schedule.followed(kickoff, &sf), 1);
		assert_eq!(schedule.followed(kickoff, &Teams::default()), 2);
		assert_eq!(schedule.followed(kickoff, &dallas), 0);
	}

	#[tokio::test]
	async fn a_failing_read_backs_off_as_before_while_asleep() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let start = at(TUESDAY);
		let mut schedule = Schedule::new(start);
		fetch.answer(&board_url, 200, &board(&[]));
		pass_at(&db, &fetch, &mut schedule, start).await;
		let day = start + Span::days(1);
		fetch.answer(&board_url, 429, b"Too Many Requests");
		pass_at(&db, &fetch, &mut schedule, day).await;
		assert_eq!(schedule.watch(day, &Teams::default(), CADENCE), Watch::BackingOff { at: day + Span::seconds(900) });
		fetch.answer(&board_url, 200, &board(&[]));
		pass_at(&db, &fetch, &mut schedule, day + Span::seconds(900)).await;
		let read = day + Span::seconds(900);
		assert_eq!(schedule.watch(read, &Teams::default(), CADENCE), Watch::Asleep { until: read + Span::days(1) });
	}

	#[test]
	fn a_game_called_off_before_its_kickoff_does_not_keep_the_feed_awake() {
		let track = Track {
			kickoff: Some(at(KICKOFF)),
			teams: [None, None],
			state: Some(GameState::Post),
			completed: false,
			done: false,
			target: Target::now(at(KICKOFF)),
		};
		let days_ahead = at(KICKOFF) - Span::days(3);
		assert!(!track.followed(days_ahead));
		assert!(!track.near(days_ahead, Span::hours(1)));
	}

	#[tokio::test]
	async fn a_game_no_team_chose_is_dropped_once_it_leaves_the_scoreboard() {
		let db = pool().await;
		let fetch = Canned::default();
		let board_url = nfl_feed::scoreboard_url();
		let start = at(TUESDAY);
		let mut over: serde_json::Value = serde_json::from_slice(&board(&[(GAME, &stamp(start - Span::hours(3)), "post", "GB", "CAR")])).unwrap();
		over["events"][0]["status"]["type"]["completed"] = serde_json::json!(true);
		let sf = teams(&["SF"]);
		let mut schedule = Schedule::new(start);
		fetch.answer(&board_url, 200, over.to_string().as_bytes());
		pass_for(&db, &fetch, &mut schedule, start, &sf).await;
		assert_eq!(schedule.followed(start, &Teams::default()), 1, "held while listed");
		fetch.answer(&board_url, 200, &board(&[]));
		pass_for(&db, &fetch, &mut schedule, start + Span::days(1), &sf).await;
		assert!(schedule.games.is_empty(), "never fetched, so never done: dropped once unlisted");
	}

	#[test]
	fn a_postponed_game_is_left_after_the_post_grace() {
		let track = Track {
			kickoff: Some(at(KICKOFF)),
			state: Some(GameState::Post),
			completed: false,
			done: false,
			teams: [None, None],
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
			completed: false,
			done: false,
			teams: [None, None],
			target: Target::now(at(KICKOFF)),
		};
		assert!(!track.followed(kickoff_plus(-1)));
		assert!(track.followed(kickoff_plus(0)));
		assert!(!track.followed(at(KICKOFF) + PRE_GRACE));
		assert_eq!(later(Some(GameState::In), Some(GameState::Pre)), Some(GameState::In));
	}
}
