//! Recording the corpus.
//!
//! [`Runner::record`] over the stored rounds, each transcript stored with
//! [`RoundRepository::replace_runs`] against the body it was recorded from.
//! What `record-leetype-runs` does, as a library, so it is tested end to end.

use crate::{RecordError, Runner};
use leetype_round_repo::{RecordedRun, RoundRepository, MANIFEST_CEILING};

/// Which rounds to record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
	/// Re-record rounds whose current version already has runs.
	pub all: bool,
	/// Only this round (listed or retired), instead of every listed one.
	pub only: Option<String>,
}

/// What happened to one round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundOutcome {
	/// Recorded and stored: its transcript.
	Recorded(Vec<RecordedRun>),
	/// Its current version already has runs, and `all` was not asked for.
	UpToDate,
	/// It has no harness, so it cannot be run. Not a failure: a round
	/// authored before harnesses existed simply has no transcript.
	NoHarness,
	/// Its body changed while it was being recorded, so the transcript was
	/// not stored; the next run records the new version.
	Changed,
	/// The round cannot be run (a hunk that does not apply, say), and why.
	Failed(String),
}

/// Every round considered, in id order, and what happened to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordReport {
	pub rounds: Vec<(String, RoundOutcome)>,
}

impl RecordReport {
	/// Whether any round failed or changed under the recording.
	#[must_use]
	pub fn incomplete(&self) -> bool {
		self.rounds.iter().any(|(_, outcome)| matches!(outcome, RoundOutcome::Failed(_) | RoundOutcome::Changed))
	}
}

/// Why a recording stopped before considering every round.
#[derive(Debug)]
pub enum RecorderError {
	/// `Selection::only` names a round the table does not hold.
	UnknownRound(String),
	/// The compiler or the temporary directory failed: no round would fare
	/// better, so the run stops.
	Toolchain(RecordError),
	Storage(sqlx::Error),
}

impl std::fmt::Display for RecorderError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::UnknownRound(id) => write!(f, "no round {id}"),
			Self::Toolchain(err) => err.fmt(f),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for RecorderError {}

impl From<sqlx::Error> for RecorderError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// Record every selected round, as of `now` (ISO-8601 UTC, stored as
/// `recorded_at` and never served).
///
/// Compiling and running block the calling thread: this is an offline
/// command's work, done one program at a time on purpose, so that no two
/// runs share the machine while they are timed.
///
/// # Errors
/// See [`RecorderError`]. A round that cannot be run is a
/// [`RoundOutcome::Failed`] in the report, not an error.
pub async fn record_listed(repository: &RoundRepository, runner: &Runner, selection: &Selection, now: &str) -> Result<RecordReport, RecorderError> {
	let ids: Vec<String> = match &selection.only {
		Some(id) => vec![id.clone()],
		None => repository.entries(MANIFEST_CEILING).await?.into_iter().map(|entry| entry.id).collect(),
	};
	let mut report = RecordReport::default();
	for id in ids {
		let Some((hash, body)) = repository.body(&id).await? else {
			return Err(RecorderError::UnknownRound(id));
		};
		if !selection.all && repository.has_runs(&id, &hash).await? {
			report.rounds.push((id, RoundOutcome::UpToDate));
			continue;
		}
		let outcome = match runner.record(body.as_bytes()) {
			Ok(runs) => {
				if repository.replace_runs(&id, &hash, &runs, now).await? {
					RoundOutcome::Recorded(runs)
				} else {
					RoundOutcome::Changed
				}
			}
			Err(RecordError::NoHarness) => RoundOutcome::NoHarness,
			Err(err) if err.is_toolchain() => return Err(RecorderError::Toolchain(err)),
			Err(err) => RoundOutcome::Failed(err.to_string()),
		};
		report.rounds.push((id, outcome));
	}
	Ok(report)
}
