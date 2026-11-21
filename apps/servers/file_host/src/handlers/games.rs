//! Handlers for `routes::games`: a game's moment log.
//!
//! Game Week Bites, paulgsc/some-ui#2074. The log is every play home kept of
//! the game, each as the scenarios it was, in the game's order
//! (`scenario_engine::moment_log`): the family "just happened" on
//! `/ws/game-week` is the newest few of. Each entry is sent through the same
//! [`wire::Happened::from_engine`] as "just happened", with where it sits in
//! the game beside it, so the two cannot drift. Which entries are big, or have
//! a bite beside them, is the phone's reading: home serves the log, not a
//! policy.
//!
//! **Cost:** one read of the game's play log (`game_feed_repo::game_log`,
//! every revision of its ~200 plays) and of the scoreboard and summary that
//! list it, then one replay through the engine. Nothing is cached: the
//! answer is a function of the log and the time, and the engine's version is
//! in it.
//!
//! **What it holds of the person: nothing.** Only ESPN's public game data
//! read through the engine; no take. It takes a [`Paired`] phone all the
//! same, as `/ws/game-week` does (docs/identity.md, "Pairing a phone"): a
//! browser's session is `403`, none `401`. Nothing of the request is logged
//! or stored.

use crate::subject::Paired;
use crate::websocket::game_live::{offer, wire};
use crate::FileHostError;
use axum::{
	extract::{Path, State},
	Json,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// `GET /games/:id/moments`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomentLog {
	/// ESPN's event id.
	pub game_id: String,
	/// The engine's version the log was read under
	/// (`scenario_engine::ENGINE_VERSION`).
	pub engine_version: u32,
	/// The scenario catalog's, whose ids, units, sizes and kinds it names.
	pub catalog_version: u32,
	/// Whether the newest summary home kept of the game by now says it is
	/// final (`post` and completed), so the log is whole.
	#[serde(rename = "final")]
	pub is_final: bool,
	/// Every play in the game's order, each as the scenarios it was, most
	/// specific first within a play.
	pub moments: Vec<Moment>,
}

/// One entry of the log: the "just happened" entry, field for field, and
/// where its play sits in the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
	#[serde(flatten)]
	pub entry: wire::Happened,
	/// The quarter of the play's snap; 5 is overtime.
	pub period: Option<i64>,
	/// The game clock at the snap (`4:05`).
	pub clock: Option<String>,
}

/// The moment log of game `id` as of `now`.
///
/// The game is read as the socket reads it (`offer::game_at`): from the newest
/// scoreboard that lists it, now, or else from the one home had when it last
/// kept a play of it by `now` (a game of an earlier week). Either way its
/// status is the newest summary's kept by `now`, so both paths read a game
/// the same, and `final` is that summary's. `None` for an id that cannot be
/// ESPN's, a game home kept no play of by `now`, or one no kept scoreboard
/// lists.
///
/// Unlike the socket, it does not read the operator's switch
/// (`settings::GameFeed`): the log is the game's record, not a live offer, so
/// a paused feed only stops it growing.
///
/// # Errors
/// Any storage failure.
pub async fn moment_log(db: &SqlitePool, id: &str, now: DateTime<Utc>) -> Result<Option<MomentLog>, sqlx::Error> {
	if !wire::game_id(id) {
		return Ok(None);
	}
	let mut conn = db.acquire().await?;
	let log = game_feed_repo::game_log(&mut conn, id).await?;
	let Some(last) = log
		.iter()
		.filter_map(|entry| DateTime::parse_from_rfc3339(&entry.seen_at).ok())
		.map(|seen| seen.with_timezone(&Utc))
		.filter(|seen| *seen <= now)
		.max()
	else {
		return Ok(None);
	};
	let mut game = match offer::game_at(&mut conn, id, now).await? {
		Some(game) => game,
		None => match offer::game_at(&mut conn, id, last).await? {
			Some(game) => game,
			None => return Ok(None),
		},
	};
	let as_of = now.to_rfc3339_opts(SecondsFormat::Millis, true);
	let summary = game_feed_repo::summary(&mut conn, id, &as_of).await?;
	if let Some(summary) = &summary {
		game.status = summary.status.clone();
	}
	let is_final = summary.is_some_and(|summary| summary.status.is_final());
	let moments = scenario_engine::moment_log(&game, &log, now)
		.into_iter()
		.map(|logged| Moment {
			entry: wire::Happened::from_engine(logged.happened, now),
			period: logged.period,
			clock: logged.clock,
		})
		.collect();
	Ok(Some(MomentLog {
		game_id: game.id,
		engine_version: scenario_engine::ENGINE_VERSION,
		catalog_version: game_scenarios::CATALOG_VERSION,
		is_final,
		moments,
	}))
}

/// `GET /games/:id/moments`: the game's moment log, to a paired phone.
///
/// # Errors
/// `404` for a game home has no log of; any storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "game_moments", skip_all, fields(otel.kind = "server"))]
pub async fn get_moments(_paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<MomentLog>, FileHostError> {
	Ok(Json(moment_log(&db, &id, Utc::now()).await?.ok_or(FileHostError::NotFound)?))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now as auth_now, AuthContext,
	};
	use axum::{
		body::Body,
		extract::FromRef,
		http::{header::AUTHORIZATION, header::COOKIE, Request, StatusCode},
		Router,
	};
	use chrono::TimeDelta;
	use game_feed_repo::{Fetched, Response as Kept};
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	/// NE at CHI, 2026-10-22, as GWB2 recorded it from ESPN.
	const FINAL: &[u8] = include_bytes!("../../../../../crates/nfl_feed/testdata/summary-401873010-final.json");
	const GAME: &str = "401873010";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn at(time: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&Utc)
	}

	/// A week's scoreboard listing `games` (`(id, home, away, date)`).
	fn board(games: &[(&str, &str, &str, &str)]) -> Vec<u8> {
		let side = |home_away: &str, team: &str| json!({ "homeAway": home_away, "team": { "abbreviation": team }, "score": "0" });
		let events: Vec<Value> = games
			.iter()
			.map(|(id, home, away, date)| {
				json!({
					"id": id, "date": date,
					"status": { "type": { "state": "in", "completed": false } },
					"competitions": [{ "competitors": [side("home", home), side("away", away)] }],
				})
			})
			.collect();
		json!({ "season": { "year": 2026, "type": 2 }, "week": { "number": 8 }, "events": events })
			.to_string()
			.into_bytes()
	}

	/// The recorded summary as ESPN served it live after its first `count`
	/// plays, and when that play happened.
	fn live_after(count: usize) -> (Vec<u8>, DateTime<Utc>) {
		let plays = nfl_feed::summary::parse(FINAL).unwrap().plays;
		let last = &plays[count - 1];
		let upto = last.sequence.unwrap();
		let mut page: Value = serde_json::from_slice(FINAL).unwrap();
		for drive in page["drives"]["previous"].as_array_mut().unwrap() {
			drive["plays"].as_array_mut().unwrap().retain(|play| {
				play["sequenceNumber"]
					.as_str()
					.and_then(|n| n.parse::<i64>().ok())
					.or_else(|| play["sequenceNumber"].as_i64())
					.is_some_and(|n| n <= upto)
			});
		}
		let competition = &mut page["header"]["competitions"][0];
		competition["status"]["type"]["state"] = json!("in");
		competition["status"]["type"]["completed"] = json!(false);
		(page.to_string().into_bytes(), at(last.wallclock.as_deref().unwrap()))
	}

	async fn keep(db: &SqlitePool, fetched: Fetched<'_>, body: &[u8], fetched_at: DateTime<Utc>) {
		let fetched_at = fetched_at.to_rfc3339_opts(SecondsFormat::Millis, true);
		let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
		game_feed_repo::keep(
			&mut tx,
			&Kept {
				fetched,
				url: "https://site.api.espn.com/apis/site/v2/sports/football/nfl/",
				fetched_at: &fetched_at,
				status: 200,
				body,
			},
		)
		.await
		.unwrap();
		tx.commit().await.unwrap();
	}

	/// NE at CHI kept as the poller would: the week, then the game live in
	/// steps, then its final. Returns when the final was kept.
	async fn recorded(db: &SqlitePool) -> DateTime<Utc> {
		let (_, first_at) = live_after(1);
		keep(
			db,
			Fetched::Scoreboard,
			&board(&[(GAME, "CHI", "NE", "2026-10-23T00:15Z")]),
			first_at - TimeDelta::minutes(30),
		)
		.await;
		let plays = nfl_feed::summary::parse(FINAL).unwrap().plays.len();
		for count in (10..plays).step_by(10) {
			let (page, played) = live_after(count);
			keep(db, Fetched::Summary(GAME), &page, played + TimeDelta::seconds(5)).await;
		}
		let (_, last_at) = live_after(plays);
		let final_at = last_at + TimeDelta::minutes(2);
		keep(db, Fetched::Summary(GAME), FINAL, final_at).await;
		final_at
	}

	/// The whole game, in its order, each entry the shape `justHappened`
	/// sends with its quarter and clock; and whatever "just happened" offered
	/// at a time is an entry of the log at that time, field for field.
	#[tokio::test]
	async fn the_log_is_the_whole_game_and_just_happened_is_its_newest() {
		let db = pool().await;
		let final_at = recorded(&db).await;
		let now = final_at + TimeDelta::hours(1);
		let read = moment_log(&db, GAME, now).await.unwrap().unwrap();
		assert!(read.is_final);
		assert_eq!(read.engine_version, scenario_engine::ENGINE_VERSION);
		assert!(read.moments.len() > 50, "{} moments", read.moments.len());
		let json = serde_json::to_value(&read).unwrap();
		assert_eq!(json["gameId"], json!(GAME));
		assert_eq!(json["final"], json!(true));
		let keys = |entry: &Value| {
			let mut keys: Vec<String> = entry.as_object().unwrap().keys().cloned().collect();
			keys.sort();
			keys
		};
		let big = json["moments"].as_array().unwrap().iter().find(|m| m["id"] == json!("in.big-play")).unwrap();
		assert_eq!(keys(big), ["at", "bigPlay", "clock", "id", "line", "period", "playId", "unit", "unitSide"]);
		let plain = json["moments"].as_array().unwrap().iter().find(|m| m["id"] != json!("in.big-play")).unwrap();
		assert_eq!(keys(plain), ["at", "clock", "id", "line", "period", "playId", "unit", "unitSide"]);

		// Mid-game: the socket's "just happened" in Full mode, as it would
		// have sent it, is the newest of the log read then.
		let (_, played) = live_after(60);
		let then = played + TimeDelta::seconds(25);
		let mut conn = db.acquire().await.unwrap();
		let game = offer::game_at(&mut conn, GAME, then).await.unwrap().unwrap();
		let log = game_feed_repo::game_log(&mut conn, GAME).await.unwrap();
		drop(conn);
		let offered = scenario_engine::offer(&game, &log, then.fixed_offset(), scenario_engine::Mode::Full, scenario_engine::Watching::Unknown);
		let sent = wire::Offer::from_engine(offered, then);
		assert!(!sent.just_happened.is_empty());
		let then_log = moment_log(&db, GAME, then).await.unwrap().unwrap();
		assert!(!then_log.is_final);
		// An entry carries the unit read off its play, live and after the
		// final alike (C1020).
		assert!(then_log.moments.iter().any(|m| m.entry.unit.is_some()));
		assert!(read.moments.iter().any(|m| m.entry.unit.is_some()));
		for happened in &sent.just_happened {
			assert!(then_log.moments.iter().any(|m| m.entry == *happened), "{happened:?}");
		}
	}

	/// A game of an earlier week, no longer on the newest scoreboard, is read
	/// from the one that listed it while home kept its plays; a game home
	/// kept no play of, or an id ESPN could not have given, is none.
	#[tokio::test]
	async fn an_earlier_weeks_game_is_still_read_and_an_unknown_one_is_none() {
		let db = pool().await;
		let final_at = recorded(&db).await;
		let next_week = final_at + TimeDelta::days(4);
		keep(&db, Fetched::Scoreboard, &board(&[("401873099", "SF", "SEA", "2026-10-30T20:25Z")]), next_week).await;
		let later = moment_log(&db, GAME, next_week + TimeDelta::hours(1)).await.unwrap().unwrap();
		let earlier = moment_log(&db, GAME, final_at + TimeDelta::hours(1)).await.unwrap().unwrap();
		assert_eq!(later.moments, earlier.moments, "both paths read the game the same");
		assert_eq!(later.is_final, earlier.is_final);
		assert!(moment_log(&db, "401873099", next_week).await.unwrap().is_none(), "no play kept");
		assert!(moment_log(&db, "../etc", next_week).await.unwrap().is_none());
	}

	#[derive(Clone)]
	struct TestState {
		pool: SqlitePool,
		auth: AuthContext,
	}

	impl FromRef<TestState> for SqlitePool {
		fn from_ref(state: &TestState) -> Self {
			state.pool.clone()
		}
	}

	impl FromRef<TestState> for AuthContext {
		fn from_ref(state: &TestState) -> Self {
			state.auth.clone()
		}
	}

	/// Only a paired phone reads it: none is `401`, a browser's session
	/// `403`; a game home has no log of is `404`.
	#[tokio::test]
	async fn only_a_paired_phone_reads_it() {
		let db = pool().await;
		recorded(&db).await;
		let auth = AuthContext::for_tests(db.clone(), 100);
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('subject-a', x'01')")
			.execute(&db)
			.await
			.unwrap();
		let now = auth_now();
		let bearer = SessionToken::mint();
		assert!(auth
			.repository()
			.create_session(&bearer.hash(Carrier::Bearer), "subject-a", now + 3_600, now)
			.await
			.unwrap());
		let cookie = SessionToken::mint();
		assert!(auth
			.repository()
			.create_session(&cookie.hash(Carrier::Cookie), "subject-a", now + 3_600, now)
			.await
			.unwrap());
		let app: Router = crate::routes::games::games().into_table_router().with_state(TestState { pool: db, auth });
		let status = |path: &str, header: Option<(axum::http::HeaderName, String)>| {
			let app = app.clone();
			let mut request = Request::builder().uri(path);
			if let Some((name, value)) = header {
				request = request.header(name, value);
			}
			let request = request.body(Body::empty()).unwrap();
			async move { app.oneshot(request).await.unwrap().status() }
		};
		let path = String::from("/games/") + GAME + "/moments";
		let phone = (AUTHORIZATION, String::from("Bearer ") + &bearer.encoded());
		let browser = (COOKIE, cookie.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned());
		assert_eq!(status(&path, None).await, StatusCode::UNAUTHORIZED);
		assert_eq!(status(&path, Some(browser)).await, StatusCode::FORBIDDEN);
		assert_eq!(status(&path, Some(phone.clone())).await, StatusCode::OK);
		assert_eq!(status("/games/401873099/moments", Some(phone)).await, StatusCode::NOT_FOUND);
	}
}
