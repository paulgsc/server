//! Studio takes on a conveyor (paulgsc/some-ui epic #2872, CONVEYOR1
//! paulgsc/some-ui#2873): where each take is, and every move that put it
//! there. `studio_take_move`.
//!
//! `20261113000600_create_studio_take_move.up.sql` records the design; in
//! short:
//!
//! - **The owner's stations** (paulgsc/some-ui#2876, 2026-11-11): a take is
//!   [`Station::InTheOven`] when recorded, then [`Station::Cooking`] while
//!   the owner sorts it, [`Station::ReadyForPickup`] (the only station the
//!   routine files from), [`Station::PickedUp`] once filed, with its issue,
//!   or [`Station::Canned`]: set aside and kept.
//! - **Each move is a row; the newest says where the take is.** A take with
//!   no row is in the oven. Nothing rewrites a move (the trigger aborts an
//!   `UPDATE`); a later move is a new row.
//! - **A move names where it starts.** [`advance`] refuses a move from a
//!   station the take is not at ([`Advanced::Stale`]), so a phone that has
//!   not seen a later move cannot skip it. A retry of the take's newest move
//!   is [`Advanced::Unchanged`] and writes nothing.
//! - **Which moves there are** ([`ask`]): to cooking, ready for pickup or
//!   canned from any station but itself; to picked up only from ready for
//!   pickup, and then always with its issue; never back to the oven, and
//!   never on from picked up.
//! - **Labels carry forward.** A move's labels are the take's from then on;
//!   a move without labels leaves them as they were.
//! - **Only the subject's own Studio takes.** A `studio` note the subject
//!   holds that is no idea's answer, as [`crate::delete::deletable`] tells a
//!   take from an answer. Every query takes the subject and filters on it.
//! - **Deleted with the take, never alone.** A take's moves go only with the
//!   account, or with the take when the owner deletes it for good from My
//!   data ([`crate::delete`], whose receipt then lists
//!   [`crate::delete::CONVEYOR_HISTORY`] among what went).
//! - **Written only by `POST /voice-notes/:id/station`** (`file_host`'s
//!   `handlers::voice`).

use crate::{
	delete::{deletable, Deletable},
	instant, VoiceError,
};
use serde::Serialize;
use sqlx::SqliteConnection;

/// Where a take is on the conveyor, in the owner's words (paulgsc/some-ui#2876).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Station {
	/// New: every take is here when recorded.
	InTheOven,
	/// Being sorted.
	Cooking,
	/// The only station the routine files from.
	ReadyForPickup,
	/// Filed, with its issue.
	PickedUp,
	/// Set aside and kept.
	Canned,
}

impl Station {
	/// Every station, in the conveyor's order.
	pub const ALL: [Self; 5] = [Self::InTheOven, Self::Cooking, Self::ReadyForPickup, Self::PickedUp, Self::Canned];

	/// The station's name, as routes and the table spell it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::InTheOven => "in-the-oven",
			Self::Cooking => "cooking",
			Self::ReadyForPickup => "ready-for-pickup",
			Self::PickedUp => "picked-up",
			Self::Canned => "canned",
		}
	}

	/// The station `name` spells, if it is one.
	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|station| station.as_str() == name)
	}

	/// The station a stored name spells.
	fn stored(name: &str) -> Result<Self, VoiceError> {
		Self::parse(name).ok_or(VoiceError::Unreadable)
	}
}

/// The most labels a move carries.
pub const MAX_LABELS: usize = 8;
/// The most characters of a label.
pub const MAX_LABEL_CHARS: usize = 32;
/// The most characters of a move's comment.
pub const MAX_COMMENT_CHARS: usize = 280;

/// A move as asked for, checked by [`ask`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
	from: Station,
	to: Station,
	at: String,
	labels: Option<Vec<String>>,
	comment: Option<String>,
	issue: Option<String>,
}

/// What a request said a move is, unchecked.
#[derive(Debug, Clone, Default)]
pub struct Said<'a> {
	/// Where the mover saw the take.
	pub from: &'a str,
	/// Where it goes.
	pub to: &'a str,
	/// When, as an RFC 3339 instant.
	pub at: &'a str,
	/// The take's labels from now on, or `None` to keep them.
	pub labels: Option<&'a [String]>,
	/// A short comment on the move.
	pub comment: Option<&'a str>,
	/// The issue a pickup filed the take as, `<owner>/<repo>#<n>`.
	pub issue: Option<&'a str>,
}

/// Whether `text` is printable: no control characters.
fn printable(text: &str) -> bool {
	!text.chars().any(char::is_control)
}

/// Whether `issue` reads `<owner>/<repo>#<n>`.
fn is_issue(issue: &str) -> bool {
	let name = |part: &str| !part.is_empty() && part.len() <= 100 && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
	let Some((repo, number)) = issue.split_once('#') else {
		return false;
	};
	let Some((owner, repo)) = repo.split_once('/') else {
		return false;
	};
	name(owner) && name(repo) && !number.is_empty() && number.len() <= 9 && number.bytes().all(|b| b.is_ascii_digit()) && !number.starts_with('0')
}

/// `said`, checked against the rules that hold whatever the take's state:
/// or each refused field, with why, in fixed words.
///
/// # Errors
/// Each refused field (`from`, `to`, `at`, `labels`, `comment`, `issue`) and
/// why.
pub fn ask(said: &Said<'_>) -> Result<Asked, Vec<(&'static str, &'static str)>> {
	let mut refused = Vec::new();
	let from = Station::parse(said.from);
	let to = Station::parse(said.to);
	if from.is_none() {
		refused.push(("from", "not a station"));
	}
	match to {
		None => refused.push(("to", "not a station")),
		Some(Station::InTheOven) => refused.push(("to", "nothing goes back to the oven")),
		Some(to) if from == Some(to) => refused.push(("to", "the take is already there")),
		Some(Station::PickedUp) if from.is_some() && from != Some(Station::ReadyForPickup) => {
			refused.push(("to", "only a take ready for pickup is picked up"));
		}
		Some(_) => {}
	}
	if from == Some(Station::PickedUp) {
		refused.push(("from", "a take picked up stays picked up"));
	}
	let at = instant(said.at);
	if at.is_none() {
		refused.push(("at", "not an RFC 3339 instant"));
	}
	let labels = said.labels.map(|labels| labels.iter().map(|label| label.trim().to_owned()).collect::<Vec<_>>());
	if let Some(labels) = &labels {
		if labels.len() > MAX_LABELS {
			refused.push(("labels", "too many labels"));
		} else if labels.iter().any(|label| label.is_empty() || label.chars().count() > MAX_LABEL_CHARS || !printable(label)) {
			refused.push(("labels", "a label is empty, too long or not printable"));
		}
	}
	let comment = said.comment.map(str::trim).filter(|comment| !comment.is_empty());
	if comment.is_some_and(|comment| comment.chars().count() > MAX_COMMENT_CHARS || !printable(comment)) {
		refused.push(("comment", "too long or not printable"));
	}
	match (to, said.issue) {
		(Some(Station::PickedUp), None) => refused.push(("issue", "a pickup names its issue")),
		(Some(Station::PickedUp), Some(issue)) if !is_issue(issue) => refused.push(("issue", "not <owner>/<repo>#<n>")),
		(Some(to), Some(_)) if to != Station::PickedUp => refused.push(("issue", "only a pickup names an issue")),
		_ => {}
	}
	match (from, to, at) {
		(Some(from), Some(to), Some(at)) if refused.is_empty() => Ok(Asked {
			from,
			to,
			at,
			labels,
			comment: comment.map(str::to_owned),
			issue: said.issue.map(str::to_owned),
		}),
		_ => Err(refused),
	}
}

/// One move of a take, as home keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Move {
	/// Its place among the take's moves, from 1.
	pub seq: i64,
	/// Where the take was.
	pub from: Station,
	/// Where it went.
	pub to: Station,
	/// When the owner moved it.
	pub at: String,
	/// The take's labels from this move on, or `None` when it left them.
	pub labels: Option<Vec<String>>,
	/// The owner's comment on the move.
	pub comment: Option<String>,
	/// The issue a pickup filed the take as.
	pub issue: Option<String>,
	/// When home recorded it.
	pub stored_at: String,
}

/// Where a take is, its labels, and every move, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TakeStation {
	/// The take's id.
	pub id: String,
	/// Where it is: its newest move's `to`, or the oven.
	pub station: Station,
	/// Its labels: the newest move's that set any, or none.
	pub labels: Vec<String>,
	/// Every move, oldest first.
	pub history: Vec<Move>,
}

impl TakeStation {
	fn of(id: &str, history: Vec<Move>) -> Self {
		Self {
			id: id.to_owned(),
			station: history.last().map_or(Station::InTheOven, |newest| newest.to),
			labels: history.iter().rev().find_map(|step| step.labels.clone()).unwrap_or_default(),
			history,
		}
	}
}

/// What a move did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Advanced {
	/// The take moved; where it is now.
	Moved(TakeStation),
	/// The take's newest move is this one already: nothing written.
	Unchanged(TakeStation),
	/// The take is not where the move starts, but here: nothing written.
	Stale(Station),
	/// The subject holds no such take: nothing written.
	NotHeld,
}

fn labels_of(stored: Option<&str>) -> Result<Option<Vec<String>>, VoiceError> {
	stored.map(|text| serde_json::from_str::<Vec<String>>(text).map_err(|_| VoiceError::Unreadable)).transpose()
}

/// Whether `subject` holds `note_id` as a Studio take: a `studio` note that
/// no idea's answer names and whose `ref` does not answer one.
async fn holds_take(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<bool, VoiceError> {
	let Some(note) = sqlx::query!(
		r#"SELECT note.of_kind, note.of_ref,
		          EXISTS (SELECT 1 FROM idea_answer answer WHERE answer.subject_id = note.subject_id AND answer.note_id = note.note_id) AS "answers!: bool"
		   FROM voice_note note WHERE note.subject_id = ?1 AND note.note_id = ?2"#,
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?
	else {
		return Ok(false);
	};
	Ok(deletable(&note.of_kind, &note.of_ref, note.answers) == Some(Deletable::StudioTake))
}

async fn history(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Vec<Move>, VoiceError> {
	sqlx::query!(
		"SELECT seq, from_station, to_station, moved_at, labels, comment, issue, stored_at
		 FROM studio_take_move WHERE subject_id = ?1 AND note_id = ?2 ORDER BY seq",
		subject,
		note_id
	)
	.fetch_all(&mut *conn)
	.await?
	.into_iter()
	.map(|row| {
		Ok(Move {
			seq: row.seq,
			from: Station::stored(&row.from_station)?,
			to: Station::stored(&row.to_station)?,
			at: row.moved_at,
			labels: labels_of(row.labels.as_deref())?,
			comment: row.comment,
			issue: row.issue,
			stored_at: row.stored_at,
		})
	})
	.collect()
}

/// Where `subject`'s take `note_id` is, and every move that put it there, if
/// the subject holds it as a Studio take.
///
/// # Errors
/// Any `sqlx` failure, or a stored move that does not read back.
pub async fn station(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<TakeStation>, VoiceError> {
	if !holds_take(conn, subject, note_id).await? {
		return Ok(None);
	}
	Ok(Some(TakeStation::of(note_id, history(conn, subject, note_id).await?)))
}

/// Moves `subject`'s take `note_id` as `asked` says, recorded by home at
/// `now`, if the take is where the move starts.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that finds where
/// the take is and the write see one snapshot.
///
/// # Errors
/// Any `sqlx` failure, or a stored move that does not read back; nothing is
/// written then.
pub async fn advance(conn: &mut SqliteConnection, subject: &str, note_id: &str, asked: &Asked, now: &str) -> Result<Advanced, VoiceError> {
	if !holds_take(conn, subject, note_id).await? {
		return Ok(Advanced::NotHeld);
	}
	let mut moves = history(conn, subject, note_id).await?;
	if let Some(newest) = moves.last() {
		let same =
			(newest.from, newest.to, &newest.at, &newest.labels, &newest.comment, &newest.issue) == (asked.from, asked.to, &asked.at, &asked.labels, &asked.comment, &asked.issue);
		if same {
			return Ok(Advanced::Unchanged(TakeStation::of(note_id, moves)));
		}
	}
	let at = moves.last().map_or(Station::InTheOven, |newest| newest.to);
	if at != asked.from {
		return Ok(Advanced::Stale(at));
	}
	let seq = moves.last().map_or(1, |newest| newest.seq + 1);
	let labels = asked.labels.as_ref().map(|labels| serde_json::Value::from(labels.clone()).to_string());
	let (from, to) = (asked.from.as_str(), asked.to.as_str());
	sqlx::query!(
		"INSERT INTO studio_take_move (subject_id, note_id, seq, from_station, to_station, moved_at, labels, comment, issue, stored_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
		subject,
		note_id,
		seq,
		from,
		to,
		asked.at,
		labels,
		asked.comment,
		asked.issue,
		now
	)
	.execute(&mut *conn)
	.await?;
	moves.push(Move {
		seq,
		from: asked.from,
		to: asked.to,
		at: asked.at.clone(),
		labels: asked.labels.clone(),
		comment: asked.comment.clone(),
		issue: asked.issue.clone(),
		stored_at: now.to_owned(),
	});
	Ok(Advanced::Moved(TakeStation::of(note_id, moves)))
}

/// A take at a station, as the conveyor page and the routine's pickup list
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtStation {
	/// The take's id.
	pub id: String,
	/// The take, `<project>:<shot>:<take>`.
	#[serde(rename = "ref")]
	pub of_ref: String,
	/// When the phone recorded it.
	pub recorded_at: String,
	/// When it was moved here, or `None` in the oven.
	pub moved_at: Option<String>,
	/// Its labels.
	pub labels: Vec<String>,
	/// The issue it was filed as, once picked up.
	pub issue: Option<String>,
}

/// The most takes [`at_station`] answers.
pub const AT_STATION: i64 = 100;

/// `subject`'s takes at `station`, newest move first (in the oven, newest
/// recorded first; the take's id breaking a tie), at most [`AT_STATION`] of
/// them, and how many are there in all.
///
/// # Errors
/// Any `sqlx` failure, or stored labels that do not read back.
pub async fn at_station(conn: &mut SqliteConnection, subject: &str, station: Station) -> Result<(Vec<AtStation>, i64), VoiceError> {
	if station == Station::InTheOven {
		return in_the_oven(conn, subject).await;
	}
	let name = station.as_str();
	let total = sqlx::query_scalar!(
		r#"SELECT COUNT(*) AS "total!: i64" FROM studio_take_move newest
		   WHERE newest.subject_id = ?1 AND newest.to_station = ?2
		     AND newest.seq = (SELECT MAX(later.seq) FROM studio_take_move later WHERE later.subject_id = newest.subject_id AND later.note_id = newest.note_id)
		     AND EXISTS (SELECT 1 FROM voice_note note WHERE note.subject_id = newest.subject_id AND note.note_id = newest.note_id)"#,
		subject,
		name
	)
	.fetch_one(&mut *conn)
	.await?;
	let rows = sqlx::query!(
		r#"SELECT newest.note_id AS "note_id!", note.of_ref AS "of_ref!", note.recorded_at AS "recorded_at!", newest.moved_at AS "moved_at!", newest.issue,
		          (SELECT labelled.labels FROM studio_take_move labelled
		           WHERE labelled.subject_id = newest.subject_id AND labelled.note_id = newest.note_id AND labelled.labels IS NOT NULL
		           ORDER BY labelled.seq DESC LIMIT 1) AS "labels?: String"
		   FROM studio_take_move newest
		   JOIN voice_note note ON note.subject_id = newest.subject_id AND note.note_id = newest.note_id
		   WHERE newest.subject_id = ?1 AND newest.to_station = ?2
		     AND newest.seq = (SELECT MAX(later.seq) FROM studio_take_move later WHERE later.subject_id = newest.subject_id AND later.note_id = newest.note_id)
		   ORDER BY newest.moved_at DESC, newest.note_id DESC
		   LIMIT ?3"#,
		subject,
		name,
		AT_STATION
	)
	.fetch_all(&mut *conn)
	.await?;
	let takes = rows
		.into_iter()
		.map(|row| {
			Ok(AtStation {
				labels: labels_of(row.labels.as_deref())?.unwrap_or_default(),
				id: row.note_id,
				of_ref: row.of_ref,
				recorded_at: row.recorded_at,
				moved_at: Some(row.moved_at),
				issue: row.issue,
			})
		})
		.collect::<Result<Vec<_>, VoiceError>>()?;
	Ok((takes, total))
}

/// The takes no move has touched: every Studio take is in the oven until
/// its first.
async fn in_the_oven(conn: &mut SqliteConnection, subject: &str) -> Result<(Vec<AtStation>, i64), VoiceError> {
	let total = sqlx::query_scalar!(
		r#"SELECT COUNT(*) AS "total!: i64" FROM voice_note note
		   WHERE note.subject_id = ?1 AND note.of_kind = 'studio' AND substr(note.of_ref, 1, 5) <> 'idea:'
		     AND NOT EXISTS (SELECT 1 FROM idea_answer answer WHERE answer.subject_id = note.subject_id AND answer.note_id = note.note_id)
		     AND NOT EXISTS (SELECT 1 FROM studio_take_move moved WHERE moved.subject_id = note.subject_id AND moved.note_id = note.note_id)"#,
		subject
	)
	.fetch_one(&mut *conn)
	.await?;
	let takes = sqlx::query!(
		r#"SELECT note.note_id, note.of_ref, note.recorded_at FROM voice_note note
		   WHERE note.subject_id = ?1 AND note.of_kind = 'studio' AND substr(note.of_ref, 1, 5) <> 'idea:'
		     AND NOT EXISTS (SELECT 1 FROM idea_answer answer WHERE answer.subject_id = note.subject_id AND answer.note_id = note.note_id)
		     AND NOT EXISTS (SELECT 1 FROM studio_take_move moved WHERE moved.subject_id = note.subject_id AND moved.note_id = note.note_id)
		   ORDER BY note.recorded_at DESC, note.note_id DESC
		   LIMIT ?2"#,
		subject,
		AT_STATION
	)
	.fetch_all(&mut *conn)
	.await?
	.into_iter()
	.map(|row| AtStation {
		id: row.note_id,
		of_ref: row.of_ref,
		recorded_at: row.recorded_at,
		moved_at: None,
		labels: Vec::new(),
		issue: None,
	})
	.collect();
	Ok((takes, total))
}

#[cfg(test)]
mod tests {
	use super::{advance, ask, at_station, station, Advanced, Asked, Said, Station, AT_STATION};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-13T09:00:00.000Z";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str, of_ref: &str, recorded_at: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, ?3, ?4, ?5, 'audio/webm', 1, 's', 'now')",
		)
		.bind(subject)
		.bind(id)
		.bind(of)
		.bind(of_ref)
		.bind(recorded_at)
		.execute(pool)
		.await
		.unwrap();
	}

	async fn take(pool: &SqlitePool, subject: &str, id: &str, recorded_at: &str) {
		note(pool, subject, id, "studio", "ep05:s1:1", recorded_at).await;
	}

	fn asked(from: &str, to: &str, at: &str) -> Asked {
		ask(&Said { from, to, at, ..Said::default() }).unwrap()
	}

	async fn go(pool: &SqlitePool, subject: &str, id: &str, asked: &Asked) -> Advanced {
		let mut conn = pool.acquire().await.unwrap();
		advance(&mut conn, subject, id, asked, NOW).await.unwrap()
	}

	async fn listed(pool: &SqlitePool, subject: &str, at: Station) -> (Vec<String>, i64) {
		let mut conn = pool.acquire().await.unwrap();
		let (takes, total) = at_station(&mut conn, subject, at).await.unwrap();
		(takes.into_iter().map(|take| take.id).collect(), total)
	}

	#[test]
	fn a_station_reads_back_as_it_is_spelled() {
		for station in Station::ALL {
			assert_eq!(Station::parse(station.as_str()), Some(station));
			assert_eq!(serde_json::to_value(station).unwrap(), station.as_str());
		}
		assert_eq!(Station::parse("oven"), None);
	}

	/// The moves that are refused whatever the take's state, each naming its
	/// field.
	#[test]
	fn a_move_is_one_the_conveyor_has() {
		let refused = |said: Said<'_>| ask(&said).unwrap_err().into_iter().map(|(field, _)| field).collect::<Vec<_>>();
		let at = "2026-11-13T01:00:00-08:00";
		let move_ = |from, to| Said { from, to, at, ..Said::default() };
		assert_eq!(asked("in-the-oven", "cooking", at).at, NOW, "kept as UTC to the millisecond");
		assert!(ask(&move_("canned", "ready-for-pickup")).is_ok(), "a canned take can come back");
		assert_eq!(refused(move_("cooking", "in-the-oven")), ["to"], "nothing goes back to the oven");
		assert_eq!(refused(move_("cooking", "cooking")), ["to"]);
		assert_eq!(refused(move_("cooking", "picked-up")), ["to", "issue"], "only from ready, with its issue");
		assert_eq!(refused(move_("picked-up", "canned")), ["from"], "picked up stays picked up");
		assert_eq!(refused(move_("oven", "fridge")), ["from", "to"]);
		assert_eq!(
			refused(Said {
				at: "soon",
				..move_("cooking", "canned")
			}),
			["at"]
		);
		let pickup = |issue| Said {
			issue: Some(issue),
			..move_("ready-for-pickup", "picked-up")
		};
		assert!(ask(&pickup("paulgsc/some-ui#2873")).is_ok());
		for bad in ["some-ui#2873", "paulgsc/some-ui#", "paulgsc/some-ui#0", "paulgsc/some ui#1", "paulgsc/some-ui#12a"] {
			assert_eq!(refused(pickup(bad)), ["issue"], "{bad}");
		}
		assert_eq!(
			refused(Said {
				issue: Some("paulgsc/server#1"),
				..move_("cooking", "canned")
			}),
			["issue"],
			"only a pickup names one"
		);

		let nine: Vec<String> = (0..9).map(|n| n.to_string()).collect();
		let long = ["x".repeat(33)];
		let blank = [String::from("  ")];
		for labels in [&nine[..], &long[..], &blank[..]] {
			assert_eq!(
				refused(Said {
					labels: Some(labels),
					..move_("cooking", "canned")
				}),
				["labels"]
			);
		}
		let spaced = [String::from(" hook ")];
		let checked = ask(&Said {
			labels: Some(&spaced),
			comment: Some("  "),
			..move_("cooking", "canned")
		})
		.unwrap();
		assert_eq!(
			(checked.labels, checked.comment),
			(Some(vec![String::from("hook")]), None),
			"trimmed, and a blank comment is none"
		);
		let long_comment = "y".repeat(281);
		assert_eq!(
			refused(Said {
				comment: Some(&long_comment),
				..move_("cooking", "canned")
			}),
			["comment"]
		);
		assert_eq!(
			refused(Said {
				comment: Some("a\u{7}b"),
				..move_("cooking", "canned")
			}),
			["comment"]
		);
	}

	/// A take starts in the oven, moves only from where it is, keeps every
	/// move, carries its labels forward, and a retry writes nothing.
	#[tokio::test]
	async fn a_take_moves_along_from_where_it_is_and_keeps_every_move() {
		let pool = pool().await;
		take(&pool, A, "t1", "2026-11-12T10:00:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		let fresh = station(&mut conn, A, "t1").await.unwrap().unwrap();
		drop(conn);
		assert_eq!((fresh.station, fresh.history.len()), (Station::InTheOven, 0));

		let labels = [String::from("hook"), String::from("ep05")];
		let to_cooking = ask(&Said {
			from: "in-the-oven",
			to: "cooking",
			at: "2026-11-13T08:00:00Z",
			labels: Some(&labels),
			comment: Some("the opener?"),
			issue: None,
		})
		.unwrap();
		let Advanced::Moved(cooking) = go(&pool, A, "t1", &to_cooking).await else {
			panic!("moved");
		};
		assert_eq!((cooking.station, cooking.labels.as_slice()), (Station::Cooking, &labels[..]));
		assert_eq!(go(&pool, A, "t1", &to_cooking).await, Advanced::Unchanged(cooking.clone()), "a retry writes nothing");

		let stale = asked("in-the-oven", "canned", "2026-11-13T08:01:00Z");
		assert_eq!(go(&pool, A, "t1", &stale).await, Advanced::Stale(Station::Cooking), "a stale phone cannot skip a move");

		assert!(matches!(
			go(&pool, A, "t1", &asked("cooking", "ready-for-pickup", "2026-11-13T08:02:00Z")).await,
			Advanced::Moved(_)
		));
		let pickup = ask(&Said {
			from: "ready-for-pickup",
			to: "picked-up",
			at: "2026-11-13T08:03:00Z",
			issue: Some("paulgsc/some-ui#3000"),
			..Said::default()
		})
		.unwrap();
		let Advanced::Moved(picked) = go(&pool, A, "t1", &pickup).await else {
			panic!("picked up");
		};
		assert_eq!(picked.station, Station::PickedUp);
		assert_eq!(picked.labels, labels, "a move without labels leaves them");
		let moves: Vec<_> = picked.history.iter().map(|step| (step.seq, step.from, step.to)).collect();
		assert_eq!(
			moves,
			[
				(1, Station::InTheOven, Station::Cooking),
				(2, Station::Cooking, Station::ReadyForPickup),
				(3, Station::ReadyForPickup, Station::PickedUp)
			]
		);
		assert_eq!(picked.history[0].comment.as_deref(), Some("the opener?"));
		assert_eq!(picked.history[2].issue.as_deref(), Some("paulgsc/some-ui#3000"));
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(station(&mut conn, A, "t1").await.unwrap(), Some(picked), "the read is what the move answered");
	}

	/// Only the subject's own Studio take is on the conveyor: not another
	/// kind, not an idea's answer, not another subject's take.
	#[tokio::test]
	async fn only_the_subjects_own_studio_take_moves() {
		let pool = pool().await;
		take(&pool, B, "b-take", "then").await;
		note(&pool, A, "reflection", "reflection", "session-1", "then").await;
		note(&pool, A, "spoken", "studio", "idea:i1:why", "then").await;
		let first = asked("in-the-oven", "cooking", NOW);
		for id in ["b-take", "reflection", "spoken", "never-sent"] {
			assert_eq!(go(&pool, A, id, &first).await, Advanced::NotHeld, "{id}");
			let mut conn = pool.acquire().await.unwrap();
			assert_eq!(station(&mut conn, A, id).await.unwrap(), None, "{id}");
		}
		assert_eq!(listed(&pool, A, Station::InTheOven).await, (vec![], 0));
		assert_eq!(listed(&pool, B, Station::InTheOven).await, (vec![String::from("b-take")], 1));
	}

	/// A station lists the takes whose newest move put them there, newest
	/// move first; the oven lists the takes no move has touched.
	#[tokio::test]
	async fn a_station_lists_the_takes_it_holds_now() {
		let pool = pool().await;
		for (id, recorded) in [("t1", "2026-11-10T00:00:00.000Z"), ("t2", "2026-11-11T00:00:00.000Z"), ("t3", "2026-11-12T00:00:00.000Z")] {
			take(&pool, A, id, recorded).await;
		}
		take(&pool, B, "t1", "2026-11-12T00:00:00.000Z").await;
		assert_eq!(
			listed(&pool, A, Station::InTheOven).await,
			(vec![String::from("t3"), String::from("t2"), String::from("t1")], 3)
		);

		go(&pool, A, "t1", &asked("in-the-oven", "cooking", "2026-11-13T01:00:00Z")).await;
		go(&pool, A, "t2", &asked("in-the-oven", "cooking", "2026-11-13T02:00:00Z")).await;
		go(&pool, A, "t1", &asked("cooking", "ready-for-pickup", "2026-11-13T03:00:00Z")).await;
		go(&pool, B, "t1", &asked("in-the-oven", "ready-for-pickup", "2026-11-13T04:00:00Z")).await;

		assert_eq!(listed(&pool, A, Station::InTheOven).await, (vec![String::from("t3")], 1));
		assert_eq!(listed(&pool, A, Station::Cooking).await, (vec![String::from("t2")], 1), "t1 has moved on");
		assert_eq!(listed(&pool, A, Station::ReadyForPickup).await, (vec![String::from("t1")], 1), "A's, not B's");
		let mut conn = pool.acquire().await.unwrap();
		let (ready, _) = at_station(&mut conn, A, Station::ReadyForPickup).await.unwrap();
		assert_eq!(
			(ready[0].of_ref.as_str(), ready[0].moved_at.as_deref(), ready[0].recorded_at.as_str()),
			("ep05:s1:1", Some("2026-11-13T03:00:00.000Z"), "2026-11-10T00:00:00.000Z")
		);
	}

	#[tokio::test]
	async fn a_station_answers_at_most_its_bound_and_counts_them_all() {
		let pool = pool().await;
		let over = AT_STATION + 2;
		for n in 0..over {
			let n = n.to_string();
			take(&pool, A, &(String::from("t") + &n), &(String::from("2026-11-12T00:00:00.000Z#") + &n)).await;
		}
		let (takes, total) = listed(&pool, A, Station::InTheOven).await;
		assert_eq!((i64::try_from(takes.len()).unwrap(), total), (AT_STATION, over));
	}

	#[tokio::test]
	async fn no_move_is_ever_updated_and_moves_leave_with_the_account() {
		let pool = pool().await;
		take(&pool, A, "t1", "then").await;
		take(&pool, B, "t1", "then").await;
		for subject in [A, B] {
			go(&pool, subject, "t1", &asked("in-the-oven", "canned", NOW)).await;
		}
		let refused = sqlx::query("UPDATE studio_take_move SET to_station = 'cooking'").execute(&pool).await;
		assert!(refused.unwrap_err().to_string().contains("append-only"));
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM studio_take_move").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B]);
	}

	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('studio_take_move') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			columns,
			[
				"subject_id",
				"note_id",
				"seq",
				"from_station",
				"to_station",
				"moved_at",
				"labels",
				"comment",
				"issue",
				"stored_at"
			]
		);
	}
}
