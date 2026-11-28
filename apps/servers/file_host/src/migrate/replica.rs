//! Whether Litestream's replica holds the database as it is now, at a TXID it
//! can restore for as long as its retention: the backup a migrating run on
//! home takes (`migrate`, step 3).
//!
//! Two things, in order (C1621):
//!
//! 1. **Caught up.** Litestream (`infra/compose/litestream.yml`) listens on a
//!    control socket (`socket:` in `infra/litestream/litestream.yml`) that
//!    speaks HTTP. `POST /sync` with `wait` makes it read what the
//!    database's WAL holds that it has not, upload everything still pending
//!    to the replica, and answer the database's TXID (`txid`) and the
//!    replica's (`replicated_txid`). The replica holds the database as it is
//!    once the second has reached the first.
//! 2. **Sealed at level 1.** That alone is not a backup that lasts. Each sync
//!    uploads a level-0 file; every 30 s Litestream compacts the new ones
//!    into one level-1 file, and deletes level 0 five minutes later. A TXID
//!    restores (`litestream restore -txid`) only where a file ends, so once
//!    level 0 is gone, a TXID inside a level-1 file is gone with it: verified
//!    on 0.5.18 on 2026-11-28, where the TXID a sync had answered, merged
//!    with the writes that followed it into one level-1 file, answered "no
//!    matching backup files available" six minutes later. Level-1 files stay
//!    until the snapshot before them leaves the retention. So the run also
//!    waits, writing nothing, until a level-1 file ends at or after the TXID
//!    it caught up to, read from the replica's `ltx/1/` (`<min>-<max>.ltx`,
//!    each TXID 16 hex digits: the LTX file client's layout), and that end is
//!    the backup. A write the old server makes meanwhile only moves the end
//!    later; the migration's own writes come after it, so they are never in
//!    the file it ends.
//!
//! [`sealed`] asks again, a second apart, until both hold, or until its
//! bound. HTTP/1.0, so the answer is one body with its length, and the
//! socket closes after it: no client library for one request on one socket.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::Instant;

/// The pause between two asks.
const AGAIN: Duration = Duration::from_secs(1);

/// What `POST /sync` answers when it synced.
#[derive(Debug, Deserialize)]
struct Synced {
	txid: u64,
	replicated_txid: u64,
}

/// What it answers when it did not.
#[derive(Debug, Deserialize)]
struct Failed {
	error: String,
}

/// What one ask found.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
	/// The replica holds the database's newest transaction, this TXID.
	CaughtUp(u64),
	/// Not yet, or Litestream not up yet: ask again while the bound allows.
	NotYet(String),
	/// An answer asking again cannot change: Litestream replicates no
	/// database at this path, or answered something this cannot read.
	Never(String),
}

/// The TXID the replica can restore the database at as it is now: the end
/// of a level-1 file at or after the TXID Litestream's replica caught up
/// to, asked at `socket` for `database` (the path as Litestream's
/// configuration names it, which is the path the database is opened at) and
/// read from `replica`'s `ltx/1/`, for up to `within`.
///
/// # Errors
///
/// Why it had not, when the bound passed or Litestream gave an answer that
/// asking again cannot change.
pub(super) async fn sealed(socket: &Path, database: &Path, replica: &Path, within: Duration) -> Result<u64, String> {
	let deadline = Instant::now() + within;
	let level_one = replica.join("ltx").join("1");
	loop {
		let left = deadline.saturating_duration_since(Instant::now());
		let why = match tokio::time::timeout(left, ask(socket, database, left)).await {
			Ok(Answer::CaughtUp(txid)) => match level_one_end(&level_one) {
				Ok(Some(end)) if end >= txid => return Ok(end),
				Ok(end) => format!(
					"the replica caught up to TXID {txid:016x}, and its level 1 ({}) ends {}: its next compaction seals it",
					level_one.display(),
					end.map_or_else(|| String::from("nowhere yet"), |end| format!("at {end:016x}"))
				),
				Err(why) => why,
			},
			Ok(Answer::Never(why)) => return Err(why),
			Ok(Answer::NotYet(why)) => why,
			Err(elapsed) => format!("it had not answered when the wait ended ({elapsed})"),
		};
		if Instant::now() + AGAIN >= deadline {
			return Err(format!("not within {} s; its last answer: {why}", within.as_secs()));
		}
		tokio::time::sleep(AGAIN).await;
	}
}

/// The highest TXID a level-1 file in `dir` ends at, by the files' names
/// (`<min>-<max>.ltx`); `None` while there is none. A name of another shape
/// is not Litestream's and is passed over.
fn level_one_end(dir: &Path) -> Result<Option<u64>, String> {
	let entries = match std::fs::read_dir(dir) {
		Ok(entries) => entries,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(err) => return Err(format!("the replica's level 1 ({}) cannot be read: {err}", dir.display())),
	};
	let mut end = None;
	for entry in entries {
		let entry = match entry {
			Ok(entry) => entry,
			Err(err) => return Err(format!("the replica's level 1 ({}) cannot be read: {err}", dir.display())),
		};
		let name = entry.file_name();
		let Some((min, max)) = name.to_str().and_then(|name| name.strip_suffix(".ltx")).and_then(|name| name.split_once('-')) else {
			continue;
		};
		if min.len() != 16 || max.len() != 16 {
			continue;
		}
		if let Ok(max) = u64::from_str_radix(max, 16) {
			end = Some(end.map_or(max, |end: u64| end.max(max)));
		}
	}
	Ok(end)
}

/// One `POST /sync`, waiting at most `left` (Litestream's own `timeout`, in
/// whole seconds, at least one).
async fn ask(socket: &Path, database: &Path, left: Duration) -> Answer {
	let mut stream = match UnixStream::connect(socket).await {
		Ok(stream) => stream,
		Err(err) => return Answer::NotYet(format!("its control socket {} does not answer: {err}", socket.display())),
	};
	let body = serde_json::json!({
		"path": database.to_string_lossy(),
		"wait": true,
		"timeout": left.as_secs().max(1),
	})
	.to_string();
	let request = format!(
		"POST /sync HTTP/1.0\r\nHost: litestream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
		body.len()
	);
	if let Err(err) = stream.write_all(request.as_bytes()).await {
		return Answer::NotYet(format!("asking it failed: {err}"));
	}
	let mut response = Vec::new();
	if let Err(err) = stream.read_to_end(&mut response).await {
		return Answer::NotYet(format!("reading its answer failed: {err}"));
	}
	read(&response, database)
}

/// What an answer says, for `database`.
fn read(response: &[u8], database: &Path) -> Answer {
	let text = String::from_utf8_lossy(response);
	let Some((head, body)) = text.split_once("\r\n\r\n") else {
		return Answer::Never(format!("an answer that is not HTTP: {text}"));
	};
	let status = head.split(' ').nth(1).unwrap_or("");
	let error = || match serde_json::from_str::<Failed>(body) {
		Ok(failed) => failed.error,
		Err(_unreadable) => body.trim().to_owned(),
	};
	match status {
		"200" => match serde_json::from_str::<Synced>(body) {
			Ok(synced) if synced.txid > 0 && synced.replicated_txid >= synced.txid => Answer::CaughtUp(synced.txid),
			Ok(synced) => Answer::NotYet(format!("the database is at TXID {:016x} and the replica at {:016x}", synced.txid, synced.replicated_txid)),
			Err(err) => Answer::Never(format!("an answer it could not read ({err}): {body}")),
		},
		"404" => Answer::Never(format!(
			"Litestream replicates no database at {} ({}): its configuration names another path",
			database.display(),
			error()
		)),
		_ => Answer::NotYet(format!("it answered {status}: {}", error())),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::net::UnixListener;

	fn http(status: &str, body: &str) -> Vec<u8> {
		format!("HTTP/1.0 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
	}

	/// The answers Litestream 0.5.18 gave on 2026-11-28, read as they were.
	#[test]
	fn its_answers_are_read_as_litestream_gives_them() {
		let db = Path::new("/app/dbs/hopium.db");
		let synced = r#"{"status":"synced","path":"/app/dbs/hopium.db","txid":2,"replicated_txid":2}"#;
		assert_eq!(read(&http("200 OK", synced), db), Answer::CaughtUp(2));
		let unchanged = r#"{"status":"no_change","path":"/app/dbs/hopium.db","txid":2,"replicated_txid":2}"#;
		assert_eq!(read(&http("200 OK", unchanged), db), Answer::CaughtUp(2));

		let behind = r#"{"status":"synced_local","path":"/app/dbs/hopium.db","txid":9,"replicated_txid":7}"#;
		assert!(matches!(read(&http("200 OK", behind), db), Answer::NotYet(why) if why.contains("0000000000000009")));
		let empty = r#"{"status":"no_change","path":"/app/dbs/hopium.db","txid":0,"replicated_txid":0}"#;
		assert!(matches!(read(&http("200 OK", empty), db), Answer::NotYet(_)));

		let waiting = r#"{"error":"sync database: replica sync: no position, waiting for data"}"#;
		assert!(matches!(read(&http("500 Internal Server Error", waiting), db), Answer::NotYet(why) if why.contains("waiting for data")));
		let elsewhere = r#"{"error":"database not found: /app/dbs/hopium.db"}"#;
		assert!(matches!(read(&http("404 Not Found", elsewhere), db), Answer::Never(why) if why.contains("another path")));
		assert!(matches!(read(b"garbage", db), Answer::Never(_)));
	}

	/// A stand-in for Litestream's socket: answers each ask with the next of
	/// `answers`, the last one again once they run out, and keeps the body of
	/// the first ask.
	fn litestream(socket: &Path, answers: Vec<Vec<u8>>) -> tokio::task::JoinHandle<String> {
		let listener = UnixListener::bind(socket).unwrap();
		tokio::spawn(async move {
			let mut first = None;
			for at in 0.. {
				let (mut stream, _) = listener.accept().await.unwrap();
				let mut asked = vec![0; 4096];
				let read = stream.read(&mut asked).await.unwrap();
				first.get_or_insert_with(|| String::from_utf8_lossy(&asked[..read]).into_owned());
				stream.write_all(&answers[at.min(answers.len() - 1)]).await.unwrap();
				stream.shutdown().await.unwrap();
				if at + 1 >= answers.len() {
					break;
				}
			}
			first.unwrap()
		})
	}

	/// A replica directory whose level 1 holds files ending at `ends`.
	fn replica(dir: &Path, ends: &[(u64, u64)]) -> std::path::PathBuf {
		let replica = dir.join("replica");
		let level_one = replica.join("ltx").join("1");
		std::fs::create_dir_all(&level_one).unwrap();
		for (min, max) in ends {
			std::fs::write(level_one.join(format!("{min:016x}-{max:016x}.ltx")), "").unwrap();
		}
		std::fs::write(level_one.join("not-litestreams.txt"), "").unwrap();
		replica
	}

	#[test]
	fn level_one_ends_at_its_highest_file() {
		let dir = tempfile::tempdir().unwrap();
		assert_eq!(level_one_end(&dir.path().join("none")), Ok(None));
		let replica = replica(dir.path(), &[(1, 1), (2, 0x3f), (0x40, 0x41)]);
		assert_eq!(level_one_end(&replica.join("ltx").join("1")), Ok(Some(0x41)));
	}

	#[tokio::test]
	async fn it_asks_again_until_the_replica_has_caught_up() {
		let dir = tempfile::tempdir().unwrap();
		let socket = dir.path().join("control.sock");
		let db = Path::new("/app/dbs/hopium.db");
		let replica = replica(dir.path(), &[(1, 40)]);
		let server = litestream(
			&socket,
			vec![
				http("500 Internal Server Error", r#"{"error":"no position, waiting for data"}"#),
				http("200 OK", r#"{"status":"synced","txid":40,"replicated_txid":40}"#),
			],
		);
		assert_eq!(sealed(&socket, db, &replica, Duration::from_secs(10)).await, Ok(40));
		let asked = server.await.unwrap();
		assert!(asked.starts_with("POST /sync HTTP/1.0\r\n"), "{asked}");
		assert!(asked.contains(r#""path":"/app/dbs/hopium.db""#) && asked.contains(r#""wait":true"#), "{asked}");
	}

	#[tokio::test]
	async fn a_replica_that_never_catches_up_is_refused_at_the_bound() {
		let dir = tempfile::tempdir().unwrap();
		let socket = dir.path().join("control.sock");
		let replica = replica(dir.path(), &[(1, 5)]);
		let behind = http("200 OK", r#"{"status":"synced_local","txid":5,"replicated_txid":4}"#);
		let _server = litestream(&socket, vec![behind; 100]);
		let refused = sealed(&socket, Path::new("/app/dbs/hopium.db"), &replica, Duration::from_millis(1500)).await.unwrap_err();
		assert!(refused.starts_with("not within 1 s") && refused.contains("the replica at 0000000000000004"), "{refused}");
	}

	/// Caught up is not enough: the TXID must end a level-1 file, or it is
	/// gone once level 0 is. It waits for the compaction that seals it, and
	/// a later end (a write after the sync) is the backup.
	#[tokio::test]
	async fn caught_up_waits_for_level_one_to_seal_it() {
		let dir = tempfile::tempdir().unwrap();
		let socket = dir.path().join("control.sock");
		let db = Path::new("/app/dbs/hopium.db");
		let replica = replica(dir.path(), &[(1, 4)]);
		let caught_up = http("200 OK", r#"{"status":"no_change","txid":6,"replicated_txid":6}"#);
		let _server = litestream(&socket, vec![caught_up; 100]);

		let refused = sealed(&socket, db, &replica, Duration::from_millis(1500)).await.unwrap_err();
		assert!(refused.contains("its level 1") && refused.contains("ends at 0000000000000004"), "{refused}");

		let level_one = replica.join("ltx").join("1");
		let compaction = tokio::spawn(async move {
			tokio::time::sleep(Duration::from_millis(1500)).await;
			std::fs::write(level_one.join(format!("{:016x}-{:016x}.ltx", 5, 7)), "").unwrap();
		});
		assert_eq!(sealed(&socket, db, &replica, Duration::from_secs(10)).await, Ok(7));
		compaction.await.unwrap();
	}

	#[tokio::test]
	async fn no_litestream_is_refused_at_the_bound_and_another_path_at_once() {
		let dir = tempfile::tempdir().unwrap();
		let socket = dir.path().join("control.sock");
		let db = Path::new("/app/dbs/hopium.db");
		let replica = replica(dir.path(), &[]);
		let refused = sealed(&socket, db, &replica, Duration::from_millis(1200)).await.unwrap_err();
		assert!(refused.contains("does not answer"), "{refused}");

		let _server = litestream(&socket, vec![http("404 Not Found", r#"{"error":"database not found: /app/dbs/hopium.db"}"#)]);
		let started = Instant::now();
		let refused = sealed(&socket, db, &replica, Duration::from_secs(60)).await.unwrap_err();
		assert!(refused.contains("another path"), "{refused}");
		assert!(started.elapsed() < Duration::from_secs(5), "a wrong path is not waited on");
	}
}
