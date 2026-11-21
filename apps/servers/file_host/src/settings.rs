//! The operator's levers: what each admin setting may hold, and what it is
//! when nobody has set it (paulgsc/server#501, ADMIN1; paulgsc/some-ui#2357).
//!
//! A lever is a policy home follows while it runs, changed from the web app
//! by the operator, never by restarting `file_host` with another environment
//! (#2357, "Decouple, don't collapse"). Each one is a [`Namespace`]: a
//! struct whose fields are the lever's, whose `Default` is its value until
//! the operator sets one, and whose `Deserialize` is its schema (unknown
//! fields refused). `admin_setting_repo` keeps the values set, append-only;
//! this module is the only place that knows what they mean.
//!
//! **Read once, where it is used.** Whatever follows a lever reads it when
//! it acts: a background pass once per pass ([`read`]), never per item, and
//! never cached across passes, so a change takes effect at the next pass with
//! no restart. A stored value its namespace no longer parses is an error, not
//! its default: a lever never quietly reverts.
//!
//! **Adding a lever** is a struct here, its line in [`NAMESPACES`], and the
//! code that reads it; the routes (`routes::admin_settings`) serve every
//! namespace listed. Keep a namespace's fields backward compatible (a new
//! field carries `#[serde(default)]`), since every revision ever stored must
//! still read.

use admin_setting_repo::Revision;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;

/// One lever: its name in the store and the route, its fields, its default.
pub trait Namespace: Serialize + DeserializeOwned + Default + PartialEq {
	/// The namespace's name: a path segment, lowercase with dashes.
	const NAME: &'static str;
}

/// Rust Rounds recording (the first lever, paulgsc/some-ui#2357).
///
/// On, the recorder hands each waiting program to the rust-runner every
/// pass; paused, it skips its passes, rounds and weeks are still kept and
/// their programs wait, and recording resumes from the first waiting program
/// when it is turned back on. **On by default**, as D1 of Rust Rounds has it
/// (every program recorded on arrival): with nothing set, home records.
///
/// It replaced `RUST_ROUNDS_ENABLED` (owner, 2026-11-08, on #2357): with the
/// runner in home's stack on every deploy, the variable only said whether
/// to record, which is policy. Whether the runner is there is detected each
/// pass, not declared (`rust_rounds`, "No runner on this machine").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RustRounds {
	pub recording: bool,
}

impl Default for RustRounds {
	fn default() -> Self {
		Self { recording: true }
	}
}

impl Namespace for RustRounds {
	const NAME: &'static str = "rust-rounds";
}

/// Following the NFL games of the week (paulgsc/some-ui#2076, GWB2).
///
/// On, the game feed (`game_feed`) fetches what its schedule says is due
/// each pass, and the phone's live game day (`/ws/game-week`) offers what it
/// kept; paused, the feed fetches nothing and keeps what it has, and every
/// offer says `following: false` with no games. **On by default**: the owner
/// wants the games followed unless paused, so with nothing set, home follows.
///
/// It replaced `GAME_FEED_ENABLED` (owner, 2026-11-13): whether home follows
/// the games is policy, not a deployment constant, as `RUST_ROUNDS_ENABLED`
/// was before it ([`RustRounds`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameFeed {
	pub following: bool,
}

impl Default for GameFeed {
	fn default() -> Self {
		Self { following: true }
	}
}

impl Namespace for GameFeed {
	const NAME: &'static str = "game-feed";
}

/// The most teams [`Teams`] may name: every NFL team is 32.
pub const MAX_TEAMS: usize = 32;

/// The teams whose games the game feed follows live (paulgsc/some-ui#3329,
/// GWB15), each as ESPN spells its abbreviation on the scoreboard (`SF`,
/// `WSH`, `LAR`: `nfl_feed::scoreboard::Side::team`), the same names the
/// phone's game day labels a game by (`SF vs SEA`). Empty means every game.
///
/// Each is 2 to 4 capital letters, at most [`MAX_TEAMS`]; held sorted and
/// once each, so the same choice is always stored the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct Teams(Vec<String>);

impl Teams {
	/// Whether a game between `home` and `away` is followed: always, when no
	/// team is named; else when either side is named.
	#[must_use]
	pub fn follows(&self, home: Option<&str>, away: Option<&str>) -> bool {
		self.0.is_empty() || [home, away].into_iter().flatten().any(|team| self.0.iter().any(|named| named == team))
	}

	/// How many teams are named: 0 is every game.
	#[must_use]
	pub fn len(&self) -> usize {
		self.0.len()
	}

	/// Whether no team is named, so every game is followed.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}
}

impl TryFrom<Vec<String>> for Teams {
	type Error = String;

	fn try_from(mut teams: Vec<String>) -> Result<Self, Self::Error> {
		if let Some(bad) = teams
			.iter()
			.find(|team| !(2..=4).contains(&team.len()) || !team.bytes().all(|byte| byte.is_ascii_uppercase()))
		{
			return Err(format!("a team is 2 to 4 capital letters, as ESPN abbreviates it, not {bad:?}"));
		}
		teams.sort_unstable();
		teams.dedup();
		if teams.len() > MAX_TEAMS {
			return Err(format!("at most {MAX_TEAMS} teams"));
		}
		Ok(Self(teams))
	}
}

impl From<Teams> for Vec<String> {
	fn from(teams: Teams) -> Self {
		teams.0
	}
}

/// Which teams' games the game feed follows (paulgsc/some-ui#3329, GWB15).
///
/// The game feed fetches a game's summary only when one of its sides is
/// named, and sleeps the scoreboard until a named team's next kickoff
/// (`game_feed::pass::Watch`). **Every game by default** (no team named), as
/// before the setting existed.
///
/// A namespace of its own beside [`GameFeed`], not a field of it: the web
/// app sets the switch with `{ following }` alone, and a field there would
/// be reset by every flip of the switch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameTeams {
	pub teams: Teams,
}

impl Namespace for GameTeams {
	const NAME: &'static str = "game-teams";
}

/// A namespace the routes serve, erased to JSON.
pub struct Entry {
	pub name: &'static str,
	/// The default, as JSON.
	pub default: fn() -> Value,
	/// The value `body` holds, re-serialised, or why the namespace refuses it.
	pub accept: fn(&[u8]) -> Result<String, String>,
}

const fn entry<N: Namespace>() -> Entry {
	Entry {
		name: N::NAME,
		default: default_json::<N>,
		accept: accept::<N>,
	}
}

fn default_json<N: Namespace>() -> Value {
	serde_json::to_value(N::default()).unwrap_or(Value::Null)
}

fn accept<N: Namespace>(body: &[u8]) -> Result<String, String> {
	let value: N = serde_json::from_slice(body).map_err(|err| err.to_string())?;
	serde_json::to_value(&value).map(|json| json.to_string()).map_err(|err| err.to_string())
}

/// Every lever home has. A namespace not here is a `404`.
pub const NAMESPACES: &[Entry] = &[entry::<RustRounds>(), entry::<GameFeed>(), entry::<GameTeams>()];

/// The namespace named `name`, if home has one.
#[must_use]
pub fn namespace(name: &str) -> Option<&'static Entry> {
	NAMESPACES.iter().find(|entry| entry.name == name)
}

/// A lever as it stands: its value, and the revision that set it, or `None`
/// while it reads as its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing<N> {
	pub value: N,
	pub set: Option<Revision>,
}

/// Why a lever could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
	#[error(transparent)]
	Storage(#[from] sqlx::Error),
	/// The standing revision no longer parses under its namespace: a schema
	/// change that broke an old value. Never read as the default.
	#[error("the standing value of {0} does not parse: {1}")]
	Unreadable(&'static str, serde_json::Error),
}

/// `N` as it stands: one indexed read of its highest revision.
///
/// # Errors
/// A storage failure, or a standing value `N` refuses ([`ReadError::Unreadable`]).
pub async fn read<N: Namespace>(db: &SqlitePool) -> Result<Standing<N>, ReadError> {
	let mut conn = db.acquire().await?;
	let Some(revision) = admin_setting_repo::current(&mut conn, N::NAME).await? else {
		return Ok(Standing { value: N::default(), set: None });
	};
	let value = serde_json::from_str(&revision.body).map_err(|err| ReadError::Unreadable(N::NAME, err))?;
	Ok(Standing { value, set: Some(revision) })
}

#[cfg(test)]
mod tests {
	use super::{namespace, read, GameFeed, GameTeams, ReadError, RustRounds, Teams, MAX_TEAMS, NAMESPACES};
	use admin_setting_repo::SetBy;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	#[test]
	fn rust_rounds_records_by_default() {
		assert!(RustRounds::default().recording);
		let entry = namespace("rust-rounds").unwrap();
		assert_eq!((entry.default)(), serde_json::json!({ "recording": true }));
	}

	#[test]
	fn the_game_feed_follows_by_default() {
		assert!(GameFeed::default().following);
		let entry = namespace("game-feed").unwrap();
		assert_eq!((entry.default)(), serde_json::json!({ "following": true }));
		assert_eq!((entry.accept)(br#"{"following":false}"#).unwrap(), r#"{"following":false}"#);
		for refused in [&br#"{"following":"no"}"#[..], br#"{"recording":false}"#, b"{}"] {
			assert!((entry.accept)(refused).is_err(), "{}", String::from_utf8_lossy(refused));
		}
	}

	#[test]
	fn the_game_feed_follows_every_team_by_default() {
		assert!(GameTeams::default().teams.is_empty());
		let entry = namespace("game-teams").unwrap();
		assert_eq!((entry.default)(), serde_json::json!({ "teams": [] }));
		// Held sorted and once each.
		assert_eq!((entry.accept)(br#"{"teams":["SF","KC","SF"]}"#).unwrap(), r#"{"teams":["KC","SF"]}"#);
		let letter = |n: usize| char::from(b'A' + u8::try_from(n).unwrap());
		let too_many: Vec<String> = (0..=MAX_TEAMS).map(|n| format!("T{}{}", letter(n / 26), letter(n % 26))).collect();
		let too_many = serde_json::json!({ "teams": too_many }).to_string();
		for refused in [
			&br#"{"teams":["sf"]}"#[..],
			br#"{"teams":["S"]}"#,
			br#"{"teams":["SFSFS"]}"#,
			br#"{"teams":"SF"}"#,
			b"{}",
			too_many.as_bytes(),
		] {
			assert!((entry.accept)(refused).is_err(), "{}", String::from_utf8_lossy(refused));
		}
	}

	#[test]
	fn a_game_is_followed_when_either_side_is_named_or_none_is() {
		let every = Teams::default();
		assert!(every.follows(Some("GB"), Some("CAR")));
		assert!(every.follows(None, None));
		let sf = Teams::try_from(vec![String::from("SF")]).unwrap();
		assert!(sf.follows(Some("SF"), Some("SEA")));
		assert!(sf.follows(Some("SEA"), Some("SF")));
		assert!(!sf.follows(Some("GB"), Some("CAR")));
		assert!(!sf.follows(None, None), "a game with no teams read is not a named team's");
	}

	#[test]
	fn every_namespace_is_a_plain_path_segment_and_named_once() {
		for (index, entry) in NAMESPACES.iter().enumerate() {
			assert!(entry.name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-'), "{}", entry.name);
			assert!(NAMESPACES[..index].iter().all(|earlier| earlier.name != entry.name), "{}", entry.name);
		}
	}

	#[test]
	fn a_value_is_held_to_its_schema() {
		let accept = namespace("rust-rounds").unwrap().accept;
		assert_eq!(accept(br#"{ "recording" : false }"#).unwrap(), r#"{"recording":false}"#);
		for refused in [&br#"{"recording":"off"}"#[..], br#"{"recording":false,"extra":1}"#, b"{}", b"not json"] {
			assert!(accept(refused).is_err(), "{}", String::from_utf8_lossy(refused));
		}
	}

	#[tokio::test]
	async fn an_unset_lever_reads_as_its_default_and_says_so() {
		let pool = pool().await;
		let standing = read::<RustRounds>(&pool).await.unwrap();
		assert_eq!((standing.value, standing.set), (RustRounds::default(), None));
	}

	#[tokio::test]
	async fn a_set_lever_reads_as_set() {
		let pool = pool().await;
		admin_setting_repo::append(
			&mut pool.acquire().await.unwrap(),
			"rust-rounds",
			r#"{"recording":false}"#,
			SetBy::Owner,
			"2026-11-08T00:00:00Z",
		)
		.await
		.unwrap();
		let standing = read::<RustRounds>(&pool).await.unwrap();
		assert!(!standing.value.recording);
		assert_eq!(standing.set.map(|set| (set.revision, set.set_by)), Some((1, SetBy::Owner)));
	}

	#[tokio::test]
	async fn a_value_that_no_longer_parses_is_an_error_not_the_default() {
		let pool = pool().await;
		admin_setting_repo::append(
			&mut pool.acquire().await.unwrap(),
			"rust-rounds",
			r#"{"paused":true}"#,
			SetBy::Owner,
			"2026-11-08T00:00:00Z",
		)
		.await
		.unwrap();
		assert!(matches!(read::<RustRounds>(&pool).await, Err(ReadError::Unreadable("rust-rounds", _))));
	}
}
