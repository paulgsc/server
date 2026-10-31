//! Rust Rounds on home (paulgsc/some-ui#2234, RR3): each round as the phone
//! pasted it, its programs, and how the rust-runner's recording of each
//! ended. `migrations/20261031000100_create_rust_round.up.sql` records the
//! design; in short:
//!
//! - **A round is its putter's** (docs/identity.md invariant 28).
//!   `rust_round` names the subject that put each round, and every read and
//!   write of it takes that subject and filters on it: another subject's
//!   round is not there. Ids are unique per subject.
//! - **Append-only, and kept until the account goes.** [`put_round`] stores
//!   a round under its subject and id, or nothing when that id already holds
//!   the same bytes; another body under a held id is refused
//!   ([`RoundError::Rewritten`]), never stored as a revision and never
//!   written over, since a round's id is never reused. No row of any table
//!   here is updated (the migration's triggers abort it). A round is deleted
//!   only with its account (a delete while the account stands is aborted);
//!   nothing else here is ever deleted.
//! - **Checked for what home needs, not more** ([`round::read`]): its size,
//!   its tag, its id, and every program to run.
//! - **One row per distinct program** (`rust_round_source`), stored with the
//!   round that first holds it, in the same transaction. It is the recording
//!   pass's queue, in the order programs arrived.
//! - **Runs are keyed to the bytes and the toolchain** (EX2, RX4):
//!   `rust_round_run` holds one row per program and toolchain, written only
//!   by [`keep_run`].
//! - **Recorded in arrival order, so what waits is one range read.** For a
//!   toolchain, the recorded programs are always the first ones to arrive:
//!   [`keep_run`] and [`keep_drop`] write only for the next program
//!   [`waiting`] would give, and refuse ([`Kept::OutOfOrder`]) anything else.
//!   So what still waits is every program after the toolchain's last run,
//!   which [`waiting`] reads with two indexed reads, however many programs
//!   are already recorded (RP2 in
//!   `apps/servers/file_host/docs/leetype-execution.md`).
//! - **The code is nobody's.** `rust_round_source`, `rust_round_run` and
//!   `rust_round_drop` hold a program's bytes and what rustc did with it, no
//!   subject (`privacy.rs`'s `NOT_SUBJECT_SCOPED`), and are shared by every
//!   round that holds the same program, whoever put it.

pub mod round;

use crate::round::Round;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use sqlx::SqliteConnection;

/// The most rounds [`rounds`] lists: the newest.
pub const LIST_LIMIT: i64 = 500;

/// The hex SHA-256 of `bytes`.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// Why a round was not stored.
#[derive(Debug)]
pub enum RoundError {
	/// Another body under an id home holds. Nothing was written.
	Rewritten,
	Storage(sqlx::Error),
}

impl From<sqlx::Error> for RoundError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

impl std::fmt::Display for RoundError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::Storage(err) => write!(f, "storage: {err}"),
		}
	}
}

impl std::error::Error for RoundError {}

/// [`RoundError::Rewritten`]'s words, for the `409`.
pub const REWRITTEN: &str = "home holds another round under this id; a round's id is never reused, so send it under a new one";

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The round, and each of its programs home did not hold, are stored.
	Added,
	/// Home held these bytes under this id already; nothing was written.
	Unchanged,
}

/// One round, as [`rounds`] lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundEntry {
	pub id: String,
	pub content_hash: String,
	pub received_at: String,
}

/// One round as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRound {
	pub id: String,
	/// The round's JSON, byte for byte as stored.
	pub text: String,
	pub content_hash: String,
	pub received_at: String,
}

/// How one program's recording under one toolchain ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRun {
	pub toolchain: String,
	/// `recorded` or `runner-restarted`.
	pub ended: String,
	/// What the phone reads (`RustcRun`), as JSON.
	pub rustc: String,
	pub recorded_at: String,
}

/// A program the pass has not recorded under its toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
	/// Its place in arrival order (`rust_round_source.seq`).
	pub seq: i64,
	pub source: String,
}

/// How a recording ended, to keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
	/// The runner answered: its whole recording, and the `RustcRun` derived
	/// from it, each as JSON.
	Recorded { recording: String, rustc: String },
	/// The runner went away while holding the program, as many times as the
	/// pass allows: a `stopped` `RustcRun`, as JSON.
	RunnerRestarted { rustc: String },
}

impl Ended {
	const fn kind(&self) -> &'static str {
		match self {
			Self::Recorded { .. } => "recorded",
			Self::RunnerRestarted { .. } => "runner-restarted",
		}
	}
}

/// What a write for a waiting program did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// Written; for [`keep_drop`], the drop's number (1 for the first).
	Added(i64),
	/// The program is not the next one waiting under this toolchain (it is
	/// recorded, or an earlier one is not): nothing was written.
	OutOfOrder,
}

/// Store `round` for `subject`, received `now`, and each of its programs
/// home does not hold yet. Run it in a `BEGIN IMMEDIATE` transaction, so the read that
/// decides and the writes see one snapshot.
///
/// # Errors
/// [`RoundError::Rewritten`] for another body under an id `subject` holds,
/// with nothing
/// written; [`RoundError::Storage`] for any `sqlx` failure.
pub async fn put_round(conn: &mut SqliteConnection, subject: &str, round: &Round, now: &str) -> Result<Change, RoundError> {
	let hash = content_hash(round.text.as_bytes());
	let held = sqlx::query_scalar!("SELECT content_hash FROM rust_round WHERE subject_id = ?1 AND id = ?2", subject, round.id)
		.fetch_optional(&mut *conn)
		.await?;
	match held {
		Some(held) if held == hash => return Ok(Change::Unchanged),
		Some(_) => return Err(RoundError::Rewritten),
		None => {}
	}
	sqlx::query!(
		"INSERT INTO rust_round (subject_id, id, body, content_hash, received_at) VALUES (?1, ?2, ?3, ?4, ?5)",
		subject,
		round.id,
		round.text,
		hash,
		now
	)
	.execute(&mut *conn)
	.await?;
	for program in &round.programs {
		let source_hash = content_hash(program.source.as_bytes());
		sqlx::query!(
			"INSERT INTO rust_round_source (source_hash, source, queued_at) VALUES (?1, ?2, ?3) ON CONFLICT (source_hash) DO NOTHING",
			source_hash,
			program.source,
			now
		)
		.execute(&mut *conn)
		.await?;
	}
	Ok(Change::Added)
}

/// `subject`'s newest `limit` rounds, newest first.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn rounds(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<RoundEntry>, sqlx::Error> {
	sqlx::query_as!(
		RoundEntry,
		"SELECT id, content_hash, received_at FROM rust_round WHERE subject_id = ?1 ORDER BY seq DESC LIMIT ?2",
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await
}

/// The round `subject` stored under `id`, if any. Another subject's round
/// under the same id is not there.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn round(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<Option<StoredRound>, sqlx::Error> {
	sqlx::query_as!(
		StoredRound,
		r#"SELECT id, body AS "text", content_hash, received_at FROM rust_round WHERE subject_id = ?1 AND id = ?2"#,
		subject,
		id
	)
	.fetch_optional(&mut *conn)
	.await
}

/// Every run kept for the program whose bytes hash to `source_hash`, under
/// any toolchain, in the order kept.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn runs(conn: &mut SqliteConnection, source_hash: &str) -> Result<Vec<StoredRun>, sqlx::Error> {
	sqlx::query_as!(
		StoredRun,
		"SELECT run.toolchain, run.ended, run.rustc, run.recorded_at
		 FROM rust_round_source AS source
		 JOIN rust_round_run AS run ON run.source_seq = source.seq
		 WHERE source.source_hash = ?1
		 ORDER BY run.recorded_at, run.rowid",
		source_hash
	)
	.fetch_all(&mut *conn)
	.await
}

/// Up to `limit` programs not yet recorded under `toolchain`, in arrival
/// order.
///
/// Every program after the toolchain's last run. Two indexed reads,
/// the toolchain's last run (`rust_round_run`'s primary key) and the range
/// after it (`rust_round_source`'s), so a pass with nothing to do reads
/// nothing.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn waiting(conn: &mut SqliteConnection, toolchain: &str, limit: i64) -> Result<Vec<Waiting>, sqlx::Error> {
	sqlx::query_as!(
		Waiting,
		r#"SELECT seq AS "seq!: i64", source FROM rust_round_source
		 WHERE seq > COALESCE((SELECT source_seq FROM rust_round_run WHERE toolchain = ?1 ORDER BY source_seq DESC LIMIT 1), 0)
		 ORDER BY seq LIMIT ?2"#,
		toolchain,
		limit
	)
	.fetch_all(&mut *conn)
	.await
}

/// Whether `seq` is the next program waiting under `toolchain`.
async fn is_next(conn: &mut SqliteConnection, toolchain: &str, seq: i64) -> Result<bool, sqlx::Error> {
	Ok(waiting(conn, toolchain, 1).await?.first().is_some_and(|next| next.seq == seq))
}

/// Keep how the recording of program `seq` under `toolchain` ended.
///
/// It is kept under `key` (`rust_rounds_wire::recording::key` of the
/// toolchain and the source the pass sent). Run it in a `BEGIN IMMEDIATE`
/// transaction.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn keep_run(conn: &mut SqliteConnection, toolchain: &str, seq: i64, key: &str, ended: &Ended, now: &str) -> Result<Kept, sqlx::Error> {
	if !is_next(conn, toolchain, seq).await? {
		return Ok(Kept::OutOfOrder);
	}
	let kind = ended.kind();
	let (recording, rustc) = match ended {
		Ended::Recorded { recording, rustc } => (Some(recording.as_str()), rustc.as_str()),
		Ended::RunnerRestarted { rustc } => (None, rustc.as_str()),
	};
	sqlx::query!(
		"INSERT INTO rust_round_run (toolchain, source_seq, key, ended, recording, rustc, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		toolchain,
		seq,
		key,
		kind,
		recording,
		rustc,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(Kept::Added(1))
}

/// Keep that the runner went away while it held program `seq` under
/// `toolchain`.
///
/// Says which drop this is (1 for the first). Run it in a
/// `BEGIN IMMEDIATE` transaction, with the [`keep_run`] that the last
/// allowed drop calls for.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn keep_drop(conn: &mut SqliteConnection, toolchain: &str, seq: i64, now: &str) -> Result<Kept, sqlx::Error> {
	if !is_next(conn, toolchain, seq).await? {
		return Ok(Kept::OutOfOrder);
	}
	let attempt = sqlx::query_scalar!(
		r#"SELECT COUNT(*) + 1 AS "attempt!: i64" FROM rust_round_drop WHERE toolchain = ?1 AND source_seq = ?2"#,
		toolchain,
		seq
	)
	.fetch_one(&mut *conn)
	.await?;
	sqlx::query!(
		"INSERT INTO rust_round_drop (toolchain, source_seq, attempt, dropped_at) VALUES (?1, ?2, ?3, ?4)",
		toolchain,
		seq,
		attempt,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(Kept::Added(attempt))
}

#[cfg(test)]
mod tests {
	use super::round::read;
	use super::{content_hash, keep_drop, keep_run, put_round, round, rounds, runs, waiting, Change, Ended, Kept, RoundError};
	use serde_json::json;
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T: &str = "rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn body(id: &str, programs: &[&str]) -> Vec<u8> {
		let cards: Vec<_> = programs
			.iter()
			.enumerate()
			.map(|(n, program)| json!({ "kind": "read", "id": String::from("c") + &n.to_string(), "program": program }))
			.collect();
		json!({ "schema": "some-ui.rust-round/1", "id": id, "cards": cards }).to_string().into_bytes()
	}

	/// Put `body` for `subject`, whose account stands.
	async fn put(pool: &SqlitePool, subject: &str, body: &[u8]) -> Result<Change, RoundError> {
		let mut tx = pool.begin().await.unwrap();
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(subject)
			.execute(&mut *tx)
			.await
			.unwrap();
		let change = put_round(&mut tx, subject, &read(body).unwrap(), "2026-10-31T00:00:00.000Z").await;
		tx.commit().await.unwrap();
		change
	}

	fn recorded(n: u8) -> Ended {
		Ended::Recorded {
			recording: String::from("{}"),
			rustc: String::from("{\"outcome\":\"prints\",\"n\":") + &n.to_string() + "}",
		}
	}

	/// A round is stored once: the same bytes again change nothing, other
	/// bytes under its id are refused, and the first stays as it was.
	#[tokio::test]
	async fn a_round_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = body("r-01", &["fn main() {}\n"]);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Unchanged);
		assert!(matches!(put(&pool, A, &body("r-01", &["fn main() { }\n"])).await, Err(RoundError::Rewritten)));
		let mut conn = pool.acquire().await.unwrap();
		let stored = round(&mut conn, A, "r-01").await.unwrap().unwrap();
		assert_eq!(stored.text.as_bytes(), first.as_slice());
		assert_eq!(stored.content_hash, content_hash(&first));
		assert_eq!(rounds(&mut conn, A, 10).await.unwrap().len(), 1);
		let sources: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_round_source").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(sources, 1, "the refused round's program was not queued");
	}

	/// No row of any table is ever updated, and none deleted while its
	/// account stands (a program, a run and a drop never are).
	#[tokio::test]
	async fn every_table_refuses_an_update_and_a_delete() {
		let pool = pool().await;
		put(&pool, A, &body("r-01", &["fn main() {}\n"])).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		keep_drop(&mut conn, T, 1, "now").await.unwrap();
		keep_run(&mut conn, T, 1, "k", &recorded(1), "now").await.unwrap();
		for statement in [
			"UPDATE rust_round SET body = '{}'",
			"DELETE FROM rust_round",
			"DELETE FROM rust_round WHERE subject_id = 'subject-a'",
			"UPDATE rust_round_source SET source = 'x'",
			"DELETE FROM rust_round_source",
			"UPDATE rust_round_run SET rustc = 'x'",
			"DELETE FROM rust_round_run",
			"UPDATE rust_round_drop SET attempt = 9",
			"DELETE FROM rust_round_drop",
		] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}
	}

	/// Programs wait in arrival order, each distinct program once; a run
	/// takes the next off the front, a write out of that order is refused,
	/// and another toolchain finds every program waiting again.
	#[tokio::test]
	async fn programs_wait_in_arrival_order_until_recorded_under_the_toolchain() {
		let pool = pool().await;
		put(&pool, A, &body("r-01", &["one", "two"])).await.unwrap();
		put(&pool, A, &body("r-02", &["two", "three"])).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let sources = |waiting: Vec<super::Waiting>| waiting.into_iter().map(|each| each.source).collect::<Vec<_>>();
		assert_eq!(sources(waiting(&mut conn, T, 10).await.unwrap()), ["one", "two", "three"]);
		assert_eq!(sources(waiting(&mut conn, T, 2).await.unwrap()), ["one", "two"], "a batch at most");

		assert_eq!(keep_run(&mut conn, T, 2, "k2", &recorded(2), "now").await.unwrap(), Kept::OutOfOrder);
		assert_eq!(keep_drop(&mut conn, T, 2, "now").await.unwrap(), Kept::OutOfOrder);
		assert_eq!(keep_run(&mut conn, T, 1, "k1", &recorded(1), "now").await.unwrap(), Kept::Added(1));
		assert_eq!(keep_run(&mut conn, T, 1, "k1", &recorded(1), "now").await.unwrap(), Kept::OutOfOrder, "once");
		assert_eq!(sources(waiting(&mut conn, T, 10).await.unwrap()), ["two", "three"]);
		assert_eq!(sources(waiting(&mut conn, "rustc 1.98.0", 10).await.unwrap()), ["one", "two", "three"]);

		assert_eq!(keep_drop(&mut conn, T, 2, "now").await.unwrap(), Kept::Added(1));
		assert_eq!(keep_drop(&mut conn, T, 2, "now").await.unwrap(), Kept::Added(2));
		let restarted = Ended::RunnerRestarted {
			rustc: String::from("{\"outcome\":\"stopped\"}"),
		};
		assert_eq!(keep_run(&mut conn, T, 2, "k2", &restarted, "now").await.unwrap(), Kept::Added(1));
		assert_eq!(sources(waiting(&mut conn, T, 10).await.unwrap()), ["three"]);

		let two = runs(&mut conn, &content_hash(b"two")).await.unwrap();
		assert_eq!(two.len(), 1);
		assert_eq!((two[0].ended.as_str(), two[0].rustc.as_str()), ("runner-restarted", "{\"outcome\":\"stopped\"}"));
		assert_eq!(runs(&mut conn, &content_hash(b"one")).await.unwrap()[0].ended, "recorded");
		assert!(runs(&mut conn, &content_hash(b"three")).await.unwrap().is_empty());
	}

	/// What waits is read by index, not by a scan of what is recorded.
	#[tokio::test]
	async fn what_waits_is_an_indexed_range_read() {
		let pool = pool().await;
		let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(
			"EXPLAIN QUERY PLAN SELECT seq, source FROM rust_round_source
			 WHERE seq > COALESCE((SELECT source_seq FROM rust_round_run WHERE toolchain = ?1 ORDER BY source_seq DESC LIMIT 1), 0)
			 ORDER BY seq LIMIT ?2",
		)
		.bind(T)
		.bind(20)
		.fetch_all(&pool)
		.await
		.unwrap();
		let plan = rows.into_iter().map(|row| row.3).collect::<Vec<_>>().join("\n");
		assert!(!plan.contains("SCAN"), "{plan}");
		assert!(plan.contains("SEARCH rust_round_source USING INTEGER PRIMARY KEY"), "{plan}");
		assert!(plan.contains("SEARCH rust_round_run USING"), "{plan}");
	}

	/// A round is its putter's: the same id under another subject is that
	/// subject's own round, each reads only its own, and a program both hold
	/// is one program, recorded once.
	#[tokio::test]
	async fn each_subject_reads_only_its_own_rounds() {
		let pool = pool().await;
		let mine = body("r-01", &["shared", "mine"]);
		let theirs = body("r-01", &["shared", "theirs"]);
		assert_eq!(put(&pool, A, &mine).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, B, &theirs).await.unwrap(), Change::Added, "ids are unique per subject");
		assert_eq!(put(&pool, B, &theirs).await.unwrap(), Change::Unchanged);
		assert!(matches!(put(&pool, B, &mine).await, Err(RoundError::Rewritten)));

		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(round(&mut conn, A, "r-01").await.unwrap().unwrap().text.as_bytes(), mine.as_slice());
		assert_eq!(round(&mut conn, B, "r-01").await.unwrap().unwrap().text.as_bytes(), theirs.as_slice());
		assert!(round(&mut conn, "subject-c", "r-01").await.unwrap().is_none());
		assert_eq!(rounds(&mut conn, A, 10).await.unwrap()[0].content_hash, content_hash(&mine));
		assert_eq!(rounds(&mut conn, B, 10).await.unwrap()[0].content_hash, content_hash(&theirs));
		assert!(rounds(&mut conn, "subject-c", 10).await.unwrap().is_empty());
		let sources: Vec<String> = sqlx::query_scalar("SELECT source FROM rust_round_source ORDER BY seq").fetch_all(&mut *conn).await.unwrap();
		assert_eq!(sources, ["shared", "mine", "theirs"]);
	}

	/// Deleting an account takes its rounds, and nothing of anyone else's;
	/// the programs and their runs, which are nobody's, stay.
	#[tokio::test]
	async fn an_account_takes_its_rounds_and_leaves_the_runs() {
		let pool = pool().await;
		put(&pool, A, &body("r-01", &["shared"])).await.unwrap();
		put(&pool, B, &body("r-01", &["shared"])).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		keep_run(&mut conn, T, 1, "k", &recorded(1), "now").await.unwrap();

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		assert!(round(&mut conn, A, "r-01").await.unwrap().is_none());
		assert!(round(&mut conn, B, "r-01").await.unwrap().is_some());
		assert_eq!(runs(&mut conn, &content_hash(b"shared")).await.unwrap().len(), 1, "the run stays");

		// The sweep of the subject-scoped tables, after the account is gone,
		// finds nothing left and is not refused.
		sqlx::query("DELETE FROM rust_round WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_round").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(left, 1);
	}
}
