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
pub const NAMESPACES: &[Entry] = &[entry::<RustRounds>()];

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
	use super::{namespace, read, ReadError, RustRounds, NAMESPACES};
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
