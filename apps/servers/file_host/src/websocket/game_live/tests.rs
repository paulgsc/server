use super::*;
use crate::auth::{
	cookie::{Carrier, SessionToken},
	now as auth_now,
};
use axum::{body::Body, extract::ws::Message, http::Request, http::StatusCode, routing::get, Router};
use chrono::{SecondsFormat, TimeDelta};
use futures::channel::mpsc;
use game_feed_repo::{Fetched, Response as Kept};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Mutex;
use tokio::time::timeout;
use tower::ServiceExt;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

/// NE at CHI, 2026-10-22, as GWB2 recorded it from ESPN.
const FINAL: &[u8] = include_bytes!("../../../../../../crates/nfl_feed/testdata/summary-401873010-final.json");
const GAME: &str = "401873010";

/// The beta's ask (GW9): a new offer on the phone within 2 s of home having it.
const ASK: Duration = Duration::from_secs(2);

async fn pool() -> SqlitePool {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	pool
}

fn at(time: &str) -> DateTime<Utc> {
	DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&Utc)
}

/// The week: the recorded game, live, and SF's next one.
fn board() -> Vec<u8> {
	let side = |home_away: &str, team: &str| json!({ "homeAway": home_away, "team": { "abbreviation": team }, "score": "0" });
	json!({
		"season": { "year": 2026, "type": 2 },
		"week": { "number": 8 },
		"events": [
			{
				"id": GAME, "date": "2026-10-23T00:15Z", "shortName": "NE @ CHI",
				"status": { "type": { "state": "in", "completed": false }, "period": 1, "displayClock": "15:00" },
				"competitions": [{ "competitors": [side("home", "CHI"), side("away", "NE")] }],
			},
			{
				"id": "401873099", "date": "2026-10-26T20:25Z",
				"status": { "type": { "state": "pre", "completed": false } },
				"competitions": [{ "competitors": [side("home", "SF"), side("away", "SEA")] }],
			},
		],
	})
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

/// A paired phone's session: its `Authorization`, and the token.
async fn paired(auth: &AuthContext, db: &SqlitePool) -> (HeaderMap, String) {
	sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('subject-a', x'01')")
		.execute(db)
		.await
		.unwrap();
	let token = SessionToken::mint();
	let now = auth_now();
	assert!(auth.repository().create_session(&token.hash(Carrier::Bearer), "subject-a", now + 3_600, now).await.unwrap());
	let mut headers = HeaderMap::new();
	headers.insert(AUTHORIZATION, HeaderValue::from_str(&(String::from("Bearer ") + &token.encoded())).unwrap());
	(headers, token.encoded())
}

struct Phone {
	to_home: mpsc::UnboundedSender<Result<Message, axum::Error>>,
	from_home: mpsc::UnboundedReceiver<Message>,
	served: tokio::task::JoinHandle<&'static str>,
}

impl Phone {
	fn open(live: &GameLive, db: &SqlitePool, session: &Session) -> Self {
		let (to_home, incoming) = mpsc::unbounded();
		let (outgoing, from_home) = mpsc::unbounded();
		let served = tokio::spawn(serve(incoming, outgoing, live.clone(), db.clone(), session.clone()));
		Self { to_home, from_home, served }
	}

	fn say(&self, message: &Value) {
		self.to_home.unbounded_send(Ok(Message::Text(message.to_string()))).unwrap();
	}

	/// The next frame from home, within the beta's ask.
	async fn hear(&mut self) -> Message {
		timeout(ASK, self.from_home.next()).await.unwrap().unwrap()
	}

	async fn hear_json(&mut self) -> Value {
		match self.hear().await {
			Message::Text(text) => serde_json::from_str(&text).unwrap(),
			other => panic!("{other:?}"),
		}
	}
}

fn hello(last_seq: Option<u64>) -> Value {
	json!({
		"type": "hello", "v": 1, "lastSeq": last_seq, "mode": "standard", "gameId": GAME,
		"watching": { GAME: true }, "utcOffsetMinutes": -420,
	})
}

struct Home {
	db: SqlitePool,
	live: GameLive,
	clock: Arc<Mutex<DateTime<Utc>>>,
	session: Session,
}

impl Home {
	async fn new(at: DateTime<Utc>) -> Self {
		let db = pool().await;
		let auth = AuthContext::for_tests(db.clone(), 100);
		let (headers, _) = paired(&auth, &db).await;
		let clock = Arc::new(Mutex::new(at));
		let read = clock.clone();
		let live = GameLive::with_clock(true, ConnectionGuard::new(), CancellationToken::new(), Box::new(move || *read.lock().unwrap()));
		Self {
			db,
			live,
			clock,
			session: Session { auth, headers },
		}
	}

	fn set_clock(&self, to: DateTime<Utc>) {
		*self.clock.lock().unwrap() = to;
	}
}

/// GWB4's "done when": stepping a replayed game at home moves the phone's
/// offer within 2 s, and a reconnect lands on the current state.
#[tokio::test]
async fn stepping_a_replayed_game_moves_the_offer_and_a_reconnect_lands_on_the_current_state() {
	let (early, early_at) = live_after(12);
	let (later, later_at) = live_after(40);
	let home = Home::new(early_at + TimeDelta::seconds(5)).await;
	keep(&home.db, Fetched::Scoreboard, &board(), early_at - TimeDelta::minutes(30)).await;
	keep(&home.db, Fetched::Summary(GAME), &early, early_at + TimeDelta::seconds(5)).await;

	let mut phone = Phone::open(&home.live, &home.db, &home.session);
	phone.say(&hello(None));
	let first = phone.hear_json().await;
	assert_eq!((first["type"].as_str(), first["v"].as_u64()), (Some("offer"), Some(1)));
	assert_eq!(first["following"], json!(true));
	assert_eq!(first["engineVersion"], json!(scenario_engine::ENGINE_VERSION));
	assert_eq!(first["catalogVersion"], json!(game_scenarios::CATALOG_VERSION));
	let games = first["games"].as_array().unwrap();
	assert_eq!(games.len(), 2);
	assert_eq!(
		games[0],
		json!({ "id": GAME, "home": "CHI", "away": "NE", "kickoff": "2026-10-23T00:15:00Z", "want": null })
	);
	assert_eq!(games[1]["want"], json!("SF"), "SF wants itself to win");
	assert_eq!(first["offer"]["gameId"], json!(GAME));
	let first_strip = first["offer"]["strip"].clone();
	assert!(first_strip["clock"].as_str().unwrap().contains("1st"), "{first_strip}");

	// Nothing new: the feed's word alone sends nothing.
	home.live.fed();
	assert!(
		timeout(Duration::from_millis(300), phone.from_home.next()).await.is_err(),
		"the same offer is not sent again"
	);

	// The replay steps: home keeps the next answer and the feed says so.
	home.set_clock(later_at + TimeDelta::seconds(5));
	keep(&home.db, Fetched::Summary(GAME), &later, later_at + TimeDelta::seconds(5)).await;
	home.live.fed();
	let second = phone.hear_json().await;
	assert_ne!(second["offer"]["strip"], first_strip, "the offer moved with the game");
	let first_seq = first["seq"].as_u64().unwrap();
	let second_seq = second["seq"].as_u64().unwrap();
	assert!(second_seq > first_seq);

	// A ping is answered with both clocks.
	phone.say(&json!({ "type": "ping", "v": 1, "phoneAt": 1_000 }));
	let pong = phone.hear_json().await;
	let home_at = u64::try_from((later_at + TimeDelta::seconds(5)).timestamp_millis()).unwrap();
	assert_eq!(pong, json!({ "type": "pong", "v": 1, "phoneAt": 1_000, "homeAt": home_at }));

	// The phone goes away and comes back: it pulls, and lands on now.
	drop(phone.to_home);
	assert_eq!(timeout(ASK, phone.served).await.unwrap().unwrap(), "the phone left");
	let mut again = Phone::open(&home.live, &home.db, &home.session);
	again.say(&hello(Some(second_seq)));
	let current = again.hear_json().await;
	assert!(current["seq"].as_u64().unwrap() > second_seq);
	assert_eq!(current["offer"], second["offer"]);
}

#[tokio::test]
async fn a_seq_never_falls_below_what_the_phone_saw() {
	let home = Home::new(at("2026-10-23T00:20:00Z")).await;
	let mut phone = Phone::open(&home.live, &home.db, &home.session);
	let ahead = 9_000_000_000_000;
	phone.say(&hello(Some(ahead)));
	let offer = phone.hear_json().await;
	assert_eq!(offer["seq"], json!(ahead + 1), "a home whose clock went back still numbers past the phone");
	assert_eq!((offer["games"].clone(), offer["offer"].clone()), (json!([]), Value::Null), "no week kept yet");
}

#[tokio::test]
async fn a_frame_home_cannot_read_closes_the_socket_saying_why() {
	let home = Home::new(at("2026-10-23T00:20:00Z")).await;
	let mut phone = Phone::open(&home.live, &home.db, &home.session);
	phone.say(&json!({ "type": "hello", "v": 2 }));
	let Message::Close(Some(frame)) = phone.hear().await else { panic!() };
	assert_eq!((frame.code, frame.reason.as_ref()), (4426, "version"));
	assert_eq!(phone.served.await.unwrap(), "version");

	let mut phone = Phone::open(&home.live, &home.db, &home.session);
	phone.to_home.unbounded_send(Ok(Message::Binary(vec![1]))).unwrap();
	let Message::Close(Some(frame)) = phone.hear().await else { panic!() };
	assert_eq!((frame.code, frame.reason.as_ref()), (4400, "unreadable"));
}

#[tokio::test]
async fn home_shutting_down_closes_the_socket() {
	let home = Home::new(at("2026-10-23T00:20:00Z")).await;
	let mut phone = Phone::open(&home.live, &home.db, &home.session);
	home.live.inner.cancel.cancel();
	let Message::Close(Some(frame)) = phone.hear().await else { panic!() };
	assert_eq!(frame.code, 1001);
}

#[tokio::test]
async fn an_unpaired_phones_session_has_ended() {
	let home = Home::new(at("2026-10-23T00:20:00Z")).await;
	assert!(!home.session.ended().await);
	sqlx::query("DELETE FROM auth_session").execute(&home.db).await.unwrap();
	assert!(home.session.ended().await);
}

#[test]
fn the_token_is_read_from_the_offered_subprotocols_only() {
	let mut headers = HeaderMap::new();
	headers.insert(SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static("game-week.v1, bearer.abc_DEF-123"));
	let bearer = offered_bearer(&headers).unwrap();
	assert_eq!(bearer.to_str().unwrap(), "Bearer abc_DEF-123");
	assert!(bearer.is_sensitive(), "never printed by a header dump");
	assert!(offered_bearer(&HeaderMap::new()).is_none());
	headers.insert(SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static("game-week.v1"));
	assert!(offered_bearer(&headers).is_none());
}

#[derive(Clone)]
struct TestState {
	auth: AuthContext,
}

impl FromRef<TestState> for AuthContext {
	fn from_ref(state: &TestState) -> Self {
		state.auth.clone()
	}
}

/// Only a paired phone opens the socket: no session is `401`, a browser's
/// `403`, and a phone's token offered as a subprotocol or sent as the header
/// is let through.
#[tokio::test]
#[allow(clippy::disallowed_methods)] // a throwaway test router for the extractor alone, not a served route
async fn only_a_paired_phone_may_open_it() {
	let db = pool().await;
	let auth = AuthContext::for_tests(db.clone(), 100);
	let (bearer, token) = paired(&auth, &db).await;
	let cookie_token = SessionToken::mint();
	let now = auth_now();
	assert!(auth
		.repository()
		.create_session(&cookie_token.hash(Carrier::Cookie), "subject-a", now + 3_600, now)
		.await
		.unwrap());
	let cookie = cookie_token.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned();
	let app = Router::new()
		.route("/ws/game-week", get(|_phone: LivePhone| async { "paired" }))
		.with_state(TestState { auth });
	let status = |headers: Vec<(&'static str, String)>| {
		let app = app.clone();
		async move {
			let mut request = Request::builder().uri("/ws/game-week");
			for (name, value) in headers {
				request = request.header(name, value);
			}
			app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status()
		}
	};
	assert_eq!(status(vec![]).await, StatusCode::UNAUTHORIZED);
	assert_eq!(status(vec![("cookie", cookie)]).await, StatusCode::FORBIDDEN, "a browser's session");
	assert_eq!(
		status(vec![("sec-websocket-protocol", String::from("game-week.v1, bearer.") + &token)]).await,
		StatusCode::OK
	);
	assert_eq!(status(vec![("authorization", bearer[AUTHORIZATION].to_str().unwrap().to_owned())]).await, StatusCode::OK);
	assert_eq!(
		status(vec![("sec-websocket-protocol", String::from("game-week.v1, bearer.not-a-token"))]).await,
		StatusCode::UNAUTHORIZED
	);
}
