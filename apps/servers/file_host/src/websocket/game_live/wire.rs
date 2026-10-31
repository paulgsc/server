//! The game day socket's messages, version 1.
//!
//! JSON text frames, each an object with a `type` and a `v`. paulgsc/some-ui keeps a copy of
//! [`examples`] (`game-live-messages.snapshot.json`, written by
//! `dump-game-live-messages`) and reads every example with the phone's own
//! decoder, so a change here that the phone cannot read fails there.
//!
//! ```text
//! phone → home  hello  { lastSeq, mode, gameId, watching, utcOffsetMinutes }
//!               ping   { phoneAt }
//! home → phone  offer  { seq, engineVersion, catalogVersion, following,
//!                        games, offer }
//!               pong   { phoneAt, homeAt }
//! ```
//!
//! What the phone sends is read strictly: an unknown `type`, a field out of
//! its bounds or a frame that is not JSON closes the socket with
//! [`Close::Unreadable`], and a `v` this build does not speak with
//! [`Close::Version`]. Fields it does not know are ignored, so a later phone
//! may add one without a new version.

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use scenario_engine::{Mode, Watching};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The one version this build speaks.
pub const VERSION: u32 = 1;

/// The largest number JavaScript holds exactly (2^53 - 1): no `seq` or
/// clock reading the phone sends or is sent may pass it.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// The most games a hello may say the owner is or is not watching.
pub const MAX_WATCHING: usize = 64;

/// The longest game id: ESPN's are nine digits.
const MAX_GAME_ID: usize = 32;

/// The widest UTC offset a hello may give, in minutes (UTC-18 to UTC+18,
/// the range `FixedOffset` takes).
const MAX_OFFSET_MINUTES: i32 = 18 * 60;

/// What the phone says it is looking at, as of its last hello.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
	/// The newest `seq` the phone has seen, if any. Home's next offer has a
	/// larger one, even across a restart of home.
	pub last_seq: Option<u64>,
	pub mode: Mode,
	/// The game the phone is on; `None` before one is picked.
	pub game_id: Option<String>,
	/// The owner's flag per game: watching (`true`) or not.
	pub watching: BTreeMap<String, bool>,
	/// The owner's offset from UTC, which places Full mode's 8 AM.
	pub offset: FixedOffset,
}

impl Hello {
	/// The owner's flag for `game`, as the engine reads it.
	#[must_use]
	pub fn watching(&self, game: &str) -> Watching {
		match self.watching.get(game) {
			Some(true) => Watching::Watched,
			Some(false) => Watching::Skipped,
			None => Watching::Unknown,
		}
	}
}

/// A message from the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
	/// What the phone is looking at; home answers with an offer at once.
	Hello(Hello),
	/// The phone's clock when it sent this, in ms since the epoch; home
	/// answers a pong at once.
	Ping { phone_at: u64 },
}

/// Why home closes the socket, as the close frame's code and reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
	/// A frame home cannot read: not JSON, an unknown type, a field out of
	/// its bounds, or binary.
	Unreadable,
	/// A `v` this build does not speak: the phone needs another build.
	Version,
	/// The phone's pairing ended (unpaired, signed out everywhere, or the
	/// account deleted) while the socket was open.
	Unpaired,
	/// Nothing came from the phone for [`super::QUIET`].
	Quiet,
	/// Home is shutting down.
	GoingAway,
}

impl Close {
	#[must_use]
	pub const fn code(self) -> u16 {
		match self {
			Self::Unreadable => 4400,
			Self::Unpaired => 4401,
			Self::Quiet => 4408,
			Self::Version => 4426,
			Self::GoingAway => 1001,
		}
	}

	#[must_use]
	pub const fn reason(self) -> &'static str {
		match self {
			Self::Unreadable => "unreadable",
			Self::Version => "version",
			Self::Unpaired => "unpaired",
			Self::Quiet => "quiet",
			Self::GoingAway => "going away",
		}
	}
}

#[derive(Deserialize)]
struct Envelope {
	#[serde(rename = "type")]
	kind: String,
	v: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloFields {
	last_seq: Option<u64>,
	mode: String,
	game_id: Option<String>,
	watching: BTreeMap<String, bool>,
	utc_offset_minutes: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PingFields {
	phone_at: u64,
}

fn game_id(id: &str) -> bool {
	!id.is_empty() && id.len() <= MAX_GAME_ID && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn mode(word: &str) -> Option<Mode> {
	match word {
		"light" => Some(Mode::Light),
		"standard" => Some(Mode::Standard),
		"full" => Some(Mode::Full),
		_ => None,
	}
}

/// Read one text frame from the phone.
///
/// # Errors
/// The [`Close`] the socket ends with: [`Close::Version`] for another `v`,
/// [`Close::Unreadable`] for anything else this does not read.
pub fn read(text: &str) -> Result<Inbound, Close> {
	let envelope: Envelope = serde_json::from_str(text).map_err(|_| Close::Unreadable)?;
	if envelope.v != VERSION {
		return Err(Close::Version);
	}
	match envelope.kind.as_str() {
		"hello" => {
			let fields: HelloFields = serde_json::from_str(text).map_err(|_| Close::Unreadable)?;
			let offset = (fields.utc_offset_minutes.abs() <= MAX_OFFSET_MINUTES)
				.then(|| FixedOffset::east_opt(fields.utc_offset_minutes * 60))
				.flatten()
				.ok_or(Close::Unreadable)?;
			let readable = fields.last_seq.is_none_or(|seq| seq <= MAX_SAFE_INTEGER)
				&& fields.game_id.as_deref().is_none_or(game_id)
				&& fields.watching.len() <= MAX_WATCHING
				&& fields.watching.keys().all(|id| game_id(id));
			if !readable {
				return Err(Close::Unreadable);
			}
			Ok(Inbound::Hello(Hello {
				last_seq: fields.last_seq,
				mode: mode(&fields.mode).ok_or(Close::Unreadable)?,
				game_id: fields.game_id,
				watching: fields.watching,
				offset,
			}))
		}
		"ping" => {
			let fields: PingFields = serde_json::from_str(text).map_err(|_| Close::Unreadable)?;
			if fields.phone_at > MAX_SAFE_INTEGER {
				return Err(Close::Unreadable);
			}
			Ok(Inbound::Ping { phone_at: fields.phone_at })
		}
		_ => Err(Close::Unreadable),
	}
}

/// One game of the week, as the phone's picker lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
	/// ESPN's event id: what each bite says it is about.
	pub id: String,
	pub home: String,
	pub away: String,
	/// ISO 8601, to the second.
	pub kickoff: String,
	/// The side SF wants to win, or `None` for a game that cannot move it.
	pub want: Option<String>,
}

/// The live strip, in words (`scenario_engine::Strip`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Strip {
	pub clock: String,
	pub possession: Option<String>,
	pub down: Option<String>,
	pub score: Option<String>,
}

/// A scenario on offer, with the line that says why now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offered {
	/// The catalog's id (`in.third-long`).
	pub id: String,
	pub line: String,
}

/// A play that just happened, as a scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Happened {
	pub id: String,
	/// ESPN's play id.
	pub play_id: String,
	pub line: String,
	/// When home first saw the play, ISO 8601 to the second: the phone says
	/// how long ago from its own clock and the offset.
	pub at: String,
}

/// The offer for the game the phone is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
	pub game_id: String,
	pub strip: Strip,
	/// Most specific first, at most the mode's cards; the first shows large.
	pub scenarios: Vec<Offered>,
	/// Newest first, at most the mode's count.
	pub just_happened: Vec<Happened>,
}

impl Offer {
	/// The engine's offer at `now`, as the phone is sent it.
	#[must_use]
	pub fn from_engine(offer: scenario_engine::Offer, now: DateTime<Utc>) -> Self {
		Self {
			game_id: offer.game_id,
			strip: Strip {
				clock: offer.strip.clock,
				possession: offer.strip.possession,
				down: offer.strip.down,
				score: offer.strip.score,
			},
			scenarios: offer
				.scenarios
				.into_iter()
				.map(|offered| Offered {
					id: offered.scenario.id().to_owned(),
					line: offered.line,
				})
				.collect(),
			just_happened: offer
				.just_happened
				.into_iter()
				.map(|happened| Happened {
					id: happened.scenario.id().to_owned(),
					play_id: happened.play_id,
					line: happened.line,
					at: stamp(now - chrono::TimeDelta::seconds(happened.ago_seconds)),
				})
				.collect(),
		}
	}
}

/// What home has for the phone now, before it is numbered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
	/// Whether home follows games at all (`GAME_FEED_ENABLED`).
	pub following: bool,
	/// The week's games, as the newest scoreboard home kept says.
	pub games: Vec<Game>,
	/// The offer for the hello's game, or `None` when it named none, or one
	/// not in the week.
	pub offer: Option<Offer>,
}

impl State {
	/// Whether the phone would see anything new in `other`. When each play
	/// happened (`at`) is left out: it is read again from the clock each
	/// time, so it can move by a second with nothing new.
	#[must_use]
	pub fn same_as(&self, other: &Self) -> bool {
		let key = |state: &Self| {
			let mut state = state.clone();
			for happened in state.offer.iter_mut().flat_map(|offer| offer.just_happened.iter_mut()) {
				happened.at.clear();
			}
			state
		};
		key(self) == key(other)
	}
}

/// A message from home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
	Offer { seq: u64, state: State },
	Pong { phone_at: u64, home_at: u64 },
}

impl Outbound {
	/// The message as the JSON the phone reads.
	#[must_use]
	pub fn to_json(&self) -> Value {
		match self {
			Self::Offer { seq, state } => json!({
				"type": "offer",
				"v": VERSION,
				"seq": seq,
				"engineVersion": scenario_engine::ENGINE_VERSION,
				"catalogVersion": game_scenarios::CATALOG_VERSION,
				"following": state.following,
				"games": state.games,
				"offer": state.offer,
			}),
			Self::Pong { phone_at, home_at } => json!({
				"type": "pong",
				"v": VERSION,
				"phoneAt": phone_at,
				"homeAt": home_at,
			}),
		}
	}

	/// The message as one text frame.
	#[must_use]
	pub fn to_text(&self) -> String {
		self.to_json().to_string()
	}
}

/// `at` as an ISO 8601 UTC time to the second.
#[must_use]
pub fn stamp(at: DateTime<Utc>) -> String {
	at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// One of every message, with every optional field set and unset at least
/// once, and every close: what `dump-game-live-messages` writes for the
/// phone's copy.
#[must_use]
pub fn examples() -> Value {
	let phone = [
		json!({
			"type": "hello", "v": VERSION, "lastSeq": null, "mode": "standard", "gameId": null,
			"watching": {}, "utcOffsetMinutes": -420,
		}),
		json!({
			"type": "hello", "v": VERSION, "lastSeq": 1_761_178_000_041_u64, "mode": "full", "gameId": "401873010",
			"watching": { "401873010": true, "401873023": false }, "utcOffsetMinutes": -420,
		}),
		json!({ "type": "ping", "v": VERSION, "phoneAt": 1_761_178_500_000_u64 }),
	];
	let home = home_examples();
	let closes = [Close::Unreadable, Close::Version, Close::Unpaired, Close::Quiet, Close::GoingAway].map(|close| json!({ "code": close.code(), "reason": close.reason() }));
	json!({
		"v": VERSION,
		"protocol": super::PROTOCOL,
		"path": "/ws/game-week",
		"phone": phone,
		"home": home.iter().map(Outbound::to_json).collect::<Vec<_>>(),
		"closes": closes,
	})
}

/// Home's half of [`examples`]: a live offer, one before kickoff, one from a
/// home that follows no games, and a pong.
fn home_examples() -> [Outbound; 4] {
	let games = vec![
		Game {
			id: "401873010".to_owned(),
			home: "CHI".to_owned(),
			away: "NE".to_owned(),
			kickoff: "2026-10-23T00:15:00Z".to_owned(),
			want: None,
		},
		Game {
			id: "401873023".to_owned(),
			home: "SF".to_owned(),
			away: "SEA".to_owned(),
			kickoff: "2026-10-26T20:25:00Z".to_owned(),
			want: Some("SF".to_owned()),
		},
	];
	let live = Offer {
		game_id: "401873010".to_owned(),
		strip: Strip {
			clock: "8:42 left in the 3rd".to_owned(),
			possession: Some("NE has the ball".to_owned()),
			down: Some("3rd and 8 at the CHI 35".to_owned()),
			score: Some("CHI 13, NE 7".to_owned()),
		},
		scenarios: vec![
			Offered {
				id: "in.third-long".to_owned(),
				line: "3rd and 8, down 6".to_owned(),
			},
			Offered {
				id: "in.play-call".to_owned(),
				line: "NE on 3rd and 8".to_owned(),
			},
		],
		just_happened: vec![Happened {
			id: "in.sack".to_owned(),
			play_id: "4018730101234".to_owned(),
			line: "NE sacked on 2nd and 5".to_owned(),
			at: "2026-10-23T01:52:20Z".to_owned(),
		}],
	};
	let before = Offer {
		game_id: "401873023".to_owned(),
		strip: Strip {
			clock: "Kickoff in 2 h 10 min".to_owned(),
			possession: None,
			down: None,
			score: None,
		},
		scenarios: Vec::new(),
		just_happened: Vec::new(),
	};
	[
		Outbound::Offer {
			seq: 1_761_178_000_042,
			state: State {
				following: true,
				games: games.clone(),
				offer: Some(live),
			},
		},
		Outbound::Offer {
			seq: 1_761_178_000_043,
			state: State {
				following: true,
				games,
				offer: Some(before),
			},
		},
		Outbound::Offer {
			seq: 1_761_178_000_044,
			state: State {
				following: false,
				games: Vec::new(),
				offer: None,
			},
		},
		Outbound::Pong {
			phone_at: 1_761_178_500_000,
			home_at: 1_761_178_500_012,
		},
	]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_phone_example_reads() {
		let examples = examples();
		let read: Vec<Inbound> = examples["phone"].as_array().unwrap().iter().map(|example| read(&example.to_string()).unwrap()).collect();
		let Inbound::Hello(first) = &read[0] else { panic!("{read:?}") };
		assert_eq!((first.last_seq, first.mode, first.game_id.as_deref()), (None, Mode::Standard, None));
		assert_eq!(first.offset, FixedOffset::west_opt(7 * 3600).unwrap());
		let Inbound::Hello(second) = &read[1] else { panic!("{read:?}") };
		assert_eq!(second.watching("401873010"), Watching::Watched);
		assert_eq!(second.watching("401873023"), Watching::Skipped);
		assert_eq!(second.watching("401873099"), Watching::Unknown);
		assert_eq!(read[2], Inbound::Ping { phone_at: 1_761_178_500_000 });
	}

	#[test]
	fn what_the_phone_sends_is_read_strictly() {
		let hello = |change: &dyn Fn(&mut Value)| {
			let mut message = examples()["phone"][1].clone();
			change(&mut message);
			read(&message.to_string())
		};
		assert_eq!(hello(&|m| m["v"] = json!(2)), Err(Close::Version));
		assert_eq!(read("not json"), Err(Close::Unreadable));
		assert_eq!(read(r#"{"type":"offer","v":1}"#), Err(Close::Unreadable), "home's own message");
		assert_eq!(hello(&|m| m["mode"] = json!("loud")), Err(Close::Unreadable));
		assert_eq!(hello(&|m| m["gameId"] = json!("4018/73010")), Err(Close::Unreadable));
		assert_eq!(hello(&|m| m["gameId"] = json!("4".repeat(33))), Err(Close::Unreadable));
		assert_eq!(hello(&|m| m["lastSeq"] = json!(MAX_SAFE_INTEGER + 1)), Err(Close::Unreadable));
		assert_eq!(hello(&|m| m["utcOffsetMinutes"] = json!(18 * 60 + 1)), Err(Close::Unreadable));
		assert_eq!(hello(&|m| m["watching"]["x y"] = json!(true)), Err(Close::Unreadable));
		let many: serde_json::Map<String, Value> = (0..=MAX_WATCHING).map(|n| (n.to_string(), json!(true))).collect();
		assert_eq!(hello(&|m| m["watching"] = Value::Object(many.clone())), Err(Close::Unreadable));
		assert_eq!(hello(&|m| m.as_object_mut().unwrap().remove("watching").map(drop).unwrap()), Err(Close::Unreadable));
		assert!(hello(&|m| m["addedLater"] = json!("ignored")).is_ok(), "a field this build does not know is ignored");
		assert_eq!(read(&json!({"type": "ping", "v": 1, "phoneAt": -1}).to_string()), Err(Close::Unreadable));
	}

	#[test]
	fn a_play_moving_by_a_second_is_nothing_new() {
		let state = State {
			following: true,
			games: Vec::new(),
			offer: Some(Offer {
				game_id: "401873010".to_owned(),
				strip: Strip {
					clock: "8:42 left in the 3rd".to_owned(),
					possession: None,
					down: None,
					score: None,
				},
				scenarios: Vec::new(),
				just_happened: vec![Happened {
					id: "in.sack".to_owned(),
					play_id: "1".to_owned(),
					line: "NE sacked on 2nd and 5".to_owned(),
					at: "2026-10-23T01:52:20Z".to_owned(),
				}],
			}),
		};
		let mut later = state.clone();
		later.offer.as_mut().unwrap().just_happened[0].at = "2026-10-23T01:52:21Z".to_owned();
		assert!(state.same_as(&later));
		later.offer.as_mut().unwrap().strip.clock = "8:01 left in the 3rd".to_owned();
		assert!(!state.same_as(&later));
	}
}
