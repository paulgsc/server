//! Handlers for `routes::probes`: home's probes, read and run by the
//! operator's web page.
//!
//! The story is paulgsc/some-ui#2383 (CK4), the page paulgsc/some-ui#2388
//! (CK8). What a run is and how it is judged is `transcribe::probe`'s; what
//! is kept is `voice_repo::probe`'s.
//!
//! Every handler takes an [`Operator`]: a run puts a note through home's
//! transcriber and its results describe home, so they are the operator's,
//! never a subject's. Nothing here carries what a probe heard.

use crate::auth::operator::Operator;
use crate::transcribe::probe::{self, Probing};
use crate::FileHostError;
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::{info, instrument};
use voice_repo::probe::{self as kept, AskedBy, Kept, Run, Started, TRANSCRIPTION};
use voice_repo::VoiceError;

/// The most runs of a probe `GET /probes` answers with, newest first.
pub const RECENT: i64 = 30;

/// `GET /probes`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Probes {
	pub probes: Vec<ProbeView>,
}

/// One probe: whether it can run here, how often it runs by itself, and its
/// newest runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeView {
	/// `transcription`.
	pub probe: &'static str,
	/// Whether this deployment runs it: the transcription probe runs only
	/// while transcription is on.
	pub enabled: bool,
	/// How often a run is due by itself, in seconds.
	pub every_seconds: u64,
	/// How long a run waits for its ending before it fails as `timed_out`.
	pub bound_seconds: u64,
	/// Newest first, at most [`RECENT`]; an open run has no `result` yet.
	pub runs: Vec<RunView>,
}

/// A run, and its result once it has ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
	pub run_id: String,
	/// `schedule` or `page`.
	pub asked_by: String,
	pub started_at: String,
	pub result: Option<ResultView>,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultView {
	/// `pass` or `fail`.
	pub verdict: String,
	/// The transcript's outcome (`heard`, `silent`, `unreadable`,
	/// `too_long`, `deadline`, `failed`), or `timed_out`.
	pub outcome: String,
	/// For `heard`: the word error rate against the clip's words.
	pub word_error: Option<f64>,
	/// From the run's start to its ending: the time to transcript.
	pub waited_ms: i64,
	pub took_ms: Option<i64>,
	pub model: Option<String>,
	pub engine: Option<String>,
	pub ended_at: String,
}

/// `POST /probes/transcription`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunAsked {
	/// The run started, or the one already open.
	pub run: RunView,
	/// Whether it was already open: nothing new was started.
	pub joined: bool,
}

fn run_view(run: Run, result: Option<kept::Ending>) -> RunView {
	RunView {
		run_id: run.run_id,
		asked_by: run.asked_by,
		started_at: run.started_at,
		result: result.map(|ending| ResultView {
			verdict: ending.verdict,
			outcome: ending.outcome,
			word_error: ending.word_error,
			waited_ms: ending.waited_ms,
			took_ms: ending.took_ms,
			model: ending.model,
			engine: ending.engine,
			ended_at: ending.ended_at,
		}),
	}
}

fn storage(err: VoiceError) -> FileHostError {
	match err {
		VoiceError::Storage(err) => FileHostError::Sqlite(err),
		VoiceError::Rewritten | VoiceError::Unreadable => FileHostError::OperationError("a probe run does not read back".to_owned()),
	}
}

/// `GET /probes`: each probe and its newest [`RECENT`] runs.
///
/// # Errors
/// `401`/`403` from [`Operator`], and a storage failure.
#[instrument(name = "probes", skip_all)]
pub async fn list(_operator: Operator, State(db): State<SqlitePool>, State(probing): State<Probing>) -> Result<Json<Probes>, FileHostError> {
	let mut conn = db.acquire().await?;
	let runs = kept::recent(&mut conn, TRANSCRIPTION, RECENT).await.map_err(storage)?;
	Ok(Json(Probes {
		probes: vec![ProbeView {
			probe: TRANSCRIPTION,
			enabled: probing.transcription,
			every_seconds: probe::EVERY.as_secs(),
			bound_seconds: probe::BOUND.as_secs(),
			runs: runs.into_iter().map(|Kept { run, result }| run_view(run, result)).collect(),
		}],
	}))
}

/// `POST /probes/transcription`: start a transcription run now, or join the
/// one already open. `202`: the transcriber hears its note on its next pass,
/// and `GET /probes` shows the result once it has one.
///
/// # Errors
/// `401`/`403` from [`Operator`]; `503` (`feature_not_configured`) while
/// transcription is off, since nothing would hear the note; and a storage
/// failure.
#[instrument(name = "probe_transcription", skip_all)]
pub async fn run_transcription(_operator: Operator, State(db): State<SqlitePool>, State(probing): State<Probing>) -> Result<(StatusCode, Json<RunAsked>), FileHostError> {
	if !probing.transcription {
		return Err(FileHostError::FeatureNotConfigured("transcription (TRANSCRIBE_ENABLED unset)"));
	}
	let started = probe::start(&db, AskedBy::Page, Utc::now()).await.map_err(storage)?;
	let joined = matches!(started, Started::Joined(_));
	if !joined {
		info!(probe = TRANSCRIPTION, asked_by = "page", "probe run started");
	}
	let run = match started {
		Started::New(run) | Started::Joined(run) => run,
	};
	Ok((StatusCode::ACCEPTED, Json(RunAsked { run: run_view(run, None), joined })))
}
