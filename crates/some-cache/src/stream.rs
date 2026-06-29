//! Redis Streams support for `some-cache`.
//!
//! Exposes a narrow API over `XADD` / `XREADGROUP` / `XACK` sufficient for
//! the pipeline → CLI notification channel.
//!
//! ## Stream entry schema
//!
//! Each entry has a single field `session_id`.  The pipeline daemon appends
//! one entry per successfully completed job.  Consumers (the CLI) read
//! entries in order, fetch the corresponding `toml` artifact from Redis, and
//! apply it.
//!
//! ## Consumer group
//!
//! Group name: `ts-cli`.  Consumer name: per-process (hostname or a UUID).
//! Using a group means multiple CLI invocations on different machines can
//! all consume independently — each maintains its own position in the stream.
//! Single-machine deployments work identically; the group just tracks the
//! last-read ID in Redis rather than in a local file.

use tracing::{debug, instrument};

use crate::{error::CacheError, store::CacheStore};

// ── Constants ─────────────────────────────────────────────────────────────

/// Stream key written by bin Y and read by the CLI.
pub const PIPELINE_COMPLETED_STREAM: &str = "ts:pipeline:completed";

/// Consumer group name used by the CLI bin.
pub const CLI_CONSUMER_GROUP: &str = "ts-cli";

/// Field name inside each stream entry.
pub const FIELD_SESSION_ID: &str = "session_id";

/// Maximum number of entries to read per `read_pending` call.
pub const DEFAULT_BATCH: usize = 16;

/// Stream max length (approx).  Keeps the stream from growing unbounded.
/// 10 000 completed jobs ≈ a few hundred KB.
pub const STREAM_MAX_LEN: usize = 10_000;

// ── StreamHandle ──────────────────────────────────────────────────────────

/// Thin wrapper around `CacheStore` that exposes stream operations.
///
/// Clone is cheap — `CacheStore` is `Clone` and internally reference-counted.
#[derive(Clone)]
pub struct StreamHandle {
	store: CacheStore,
	stream_key: String,
	group: String,
	consumer: String,
}

impl StreamHandle {
	/// Construct from an existing `CacheStore`.
	///
	/// `consumer` should be stable across restarts for the same host so that
	/// pending-entry recovery (`XAUTOCLAIM` / re-read of PEL) works correctly.
	/// Typically: hostname or a fixed string per deployment.
	pub fn new(store: CacheStore, stream_key: impl Into<String>, group: impl Into<String>, consumer: impl Into<String>) -> Self {
		Self {
			store,
			stream_key: stream_key.into(),
			group: group.into(),
			consumer: consumer.into(),
		}
	}

	/// Convenience: use the well-known pipeline completed stream + CLI group.
	pub fn pipeline_completed(store: CacheStore, consumer: impl Into<String>) -> Self {
		Self::new(store, PIPELINE_COMPLETED_STREAM, CLI_CONSUMER_GROUP, consumer)
	}

	// ── Writer API (used by bin Y) ────────────────────────────────────────

	/// Append a `session_id` to the stream.
	///
	/// Uses `XADD <key> MAXLEN ~ <STREAM_MAX_LEN> * session_id <id>`.
	/// The `~` makes trimming approximate (faster; avoids rebalancing on
	/// every write).
	///
	/// Non-fatal from the caller's perspective: a failure here must not
	/// block the pipeline ACK.  Log and discard at the call site.
	#[instrument(skip(self), fields(session_id = %session_id))]
	pub async fn publish_completed(&self, session_id: &str) -> Result<String, CacheError> {
		let stream_key = self.stream_key.clone();
		let session_id = session_id.to_string();

		let entry_id: String = self
			.store
			.with_retry("xadd", || {
				let con = self.store.redis_client().clone();
				let stream_key = stream_key.clone();
				let session_id = session_id.clone();

				Box::pin(async move {
					let mut con = con.get_multiplexed_async_connection().await?;
					let id: String = redis::cmd("XADD")
						.arg(&stream_key)
						.arg("MAXLEN")
						.arg("~")
						.arg(STREAM_MAX_LEN)
						.arg("*") // auto-generated ID
						.arg(FIELD_SESSION_ID)
						.arg(&session_id)
						.query_async(&mut con)
						.await?;
					Result::<_, CacheError>::Ok(id)
				})
			})
			.await?;

		debug!("stream published session_id={} entry_id={}", session_id, entry_id);
		Ok(entry_id)
	}

	// ── Reader API (used by CLI bin) ──────────────────────────────────────

	/// Ensure the consumer group exists.
	///
	/// `XGROUP CREATE … $ MKSTREAM` — creates the group at the current tail
	/// so the consumer only sees *new* entries from now on.  Pass
	/// `start_from_beginning = true` to replay from the start of the stream
	/// (useful for recovery / first-time init).
	///
	/// Idempotent: `BUSYGROUP` error is silently ignored.
	pub async fn ensure_group(&self, start_from_beginning: bool) -> Result<(), CacheError> {
		let stream_key = self.stream_key.clone();
		let group = self.group.clone();
		let start_id = if start_from_beginning { "0" } else { "$" };

		let result: redis::RedisResult<()> = async {
			let mut con = self.store.redis_client().get_multiplexed_async_connection().await?;
			redis::cmd("XGROUP")
				.arg("CREATE")
				.arg(&stream_key)
				.arg(&group)
				.arg(start_id)
				.arg("MKSTREAM")
				.query_async(&mut con)
				.await
		}
		.await;

		match result {
			Ok(()) => Ok(()),
			Err(e) if e.to_string().contains("BUSYGROUP") => {
				debug!("consumer group '{}' already exists", group);
				Ok(())
			}
			Err(e) => Err(CacheError::from(e)),
		}
	}

	/// Read up to `batch` undelivered entries from the stream.
	///
	/// Uses `XREADGROUP GROUP <group> <consumer> COUNT <batch> BLOCK <block_ms> STREAMS <key> >`.
	/// The `>` special ID means "give me entries not yet delivered to any
	/// consumer in this group."
	///
	/// Returns a vec of `(entry_id, session_id)` pairs.  The caller must
	/// call `ack` for each entry it successfully processes.
	///
	/// `block_ms = 0` blocks indefinitely; use a finite value for CLI loops
	/// that also need to handle Ctrl-C.
	pub async fn read_new(&self, batch: usize, block_ms: u64) -> Result<Vec<(String, String)>, CacheError> {
		let stream_key = self.stream_key.clone();
		let group = self.group.clone();
		let consumer = self.consumer.clone();

		let mut con = self.store.redis_client().get_multiplexed_async_connection().await?;

		let raw: redis::Value = redis::cmd("XREADGROUP")
			.arg("GROUP")
			.arg(&group)
			.arg(&consumer)
			.arg("COUNT")
			.arg(batch)
			.arg("BLOCK")
			.arg(block_ms)
			.arg("STREAMS")
			.arg(&stream_key)
			.arg(">")
			.query_async(&mut con)
			.await?;

		Ok(parse_xread_response(raw))
	}

	/// Re-read entries in the Pending Entry List (PEL) — i.e. delivered but
	/// not yet ACKed by this consumer.
	///
	/// Called at startup to recover from a crash mid-processing.  Any entry
	/// in the PEL was delivered before the crash but never ACKed; re-reading
	/// it here lets the caller re-process it idempotently.
	pub async fn read_pending(&self, batch: usize) -> Result<Vec<(String, String)>, CacheError> {
		let stream_key = self.stream_key.clone();
		let group = self.group.clone();
		let consumer = self.consumer.clone();

		let mut con = self.store.redis_client().get_multiplexed_async_connection().await?;

		// ID "0" means "re-deliver all PEL entries for this consumer".
		let raw: redis::Value = redis::cmd("XREADGROUP")
			.arg("GROUP")
			.arg(&group)
			.arg(&consumer)
			.arg("COUNT")
			.arg(batch)
			.arg("STREAMS")
			.arg(&stream_key)
			.arg("0")
			.query_async(&mut con)
			.await?;

		Ok(parse_xread_response(raw))
	}

	/// Acknowledge processed entries.
	///
	/// `XACK <stream> <group> <id> …`
	/// Removes the entries from this consumer's PEL; they will not be
	/// redelivered.
	pub async fn ack(&self, entry_ids: &[String]) -> Result<(), CacheError> {
		if entry_ids.is_empty() {
			return Ok(());
		}

		let stream_key = self.stream_key.clone();
		let group = self.group.clone();
		let ids = entry_ids.to_vec();

		let mut con = self.store.redis_client().get_multiplexed_async_connection().await?;
		let mut cmd = redis::cmd("XACK");
		cmd.arg(&stream_key).arg(&group);
		for id in &ids {
			cmd.arg(id);
		}
		let _: i64 = cmd.query_async(&mut con).await?;

		debug!("xack {} entries", ids.len());
		Ok(())
	}
}

// ── Parse helpers ─────────────────────────────────────────────────────────

/// Parse the nested `redis::Value` returned by XREADGROUP into a flat vec of
/// `(entry_id, session_id)` pairs.
///
/// XREADGROUP response shape:
/// ```text
/// Array [
///   Array [
///     Bulk("stream_key"),
///     Array [
///       Array [
///         Bulk("1234567890123-0"),        // entry id
///         Array [ Bulk("session_id"), Bulk("abc-123") ]
///       ],
///       …
///     ]
///   ]
/// ]
/// ```
/// Returns an empty vec on nil response (no new messages / timeout).
fn parse_xread_response(val: redis::Value) -> Vec<(String, String)> {
	let mut out = Vec::new();

	let streams = match val {
		redis::Value::Array(v) => v,
		_ => return out,
	};

	for stream in streams {
		let parts = match stream {
			redis::Value::Array(v) => v,
			_ => continue,
		};
		// parts[0] = stream name, parts[1] = entries array
		let entries = match parts.into_iter().nth(1) {
			Some(redis::Value::Array(e)) => e,
			_ => continue,
		};
		for entry in entries {
			let entry_parts = match entry {
				redis::Value::Array(v) => v,
				_ => continue,
			};
			let entry_id = match entry_parts.first() {
				Some(redis::Value::BulkString(b)) => String::from_utf8_lossy(b).into_owned(),
				_ => continue,
			};
			let fields = match entry_parts.into_iter().nth(1) {
				Some(redis::Value::Array(f)) => f,
				_ => continue,
			};
			// fields = [key, value, key, value, …]
			let mut it = fields.into_iter();
			while let (Some(k), Some(v)) = (it.next(), it.next()) {
				let key = match k {
					redis::Value::BulkString(b) => String::from_utf8_lossy(&b).into_owned(),
					_ => continue,
				};
				if key == FIELD_SESSION_ID {
					let val = match v {
						redis::Value::BulkString(b) => String::from_utf8_lossy(&b).into_owned(),
						_ => continue,
					};
					out.push((entry_id.clone(), val));
				}
			}
		}
	}

	out
}
