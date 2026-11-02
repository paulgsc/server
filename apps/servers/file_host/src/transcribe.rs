//! Home hears the owner's voice notes (paulgsc/server#462).
//!
//! The phone sends each note home (`PUT /api/v1/voice-notes/:id`, kept in
//! `voice_note` and `voice_note_audio`), and this worker transcribes each on
//! home's CPU with whisper.cpp, keeping what it heard in `voice_transcript`,
//! and when each word was said in `voice_transcript_word`
//! (`voice_repo::transcript`). The routines that used to download notes from
//! Drive and install Whisper every run will read the text instead (VOICE4,
//! VOICE6 in #460).
//!
//! **Why this is a poll.** A note can arrive while the worker is busy, and a
//! restart must pick up whatever was left, so the worker asks the database
//! every [`INTERVAL`] (a minute) for the notes its model has not
//! heard. An idle pass is that one indexed read. A note waits at most an
//! interval before it is heard, which nothing reading transcripts notices.
//!
//! What this module promises, as docs/identity.md invariant 23 states:
//!
//! - **Never on a request's path.** Notes are heard one at a time, on a
//!   blocking thread with one fewer threads than the container's cores,
//!   each by [`DEADLINE`].
//! - **Append-only.** Each attempt is a new row; nothing is updated, and a
//!   note's audio is read, never written or deleted (`pass`).
//! - **Nothing heard reaches a log line.** A pass logs the kind of each
//!   attempt and how many, never a note's text, audio, id or subject.
//!
//! **What it costs.** A multilingual `small` model at `q5_1` holds about
//! 500 MB while loaded, for as long as the server runs, and hears a minute
//! of speech in about a minute on two threads. Off unless
//! `TRANSCRIBE_ENABLED`; on, the server refuses to start without a model,
//! the VAD model and ffmpeg.

pub mod engine;
pub mod pass;

use crate::auth::DeletionLock;
use crate::{AppState, Config};
use engine::Whisper;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

// The transcriber's engineering values: source constants, not variables
// (#617). Each was its variable's default.

/// The shortest interval a pass may run at.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);

/// How often the worker looks for notes to hear (`TRANSCRIBE_SECONDS`).
pub const INTERVAL: Duration = Duration::from_secs(60);

/// The shortest deadline one note may get: a minute-long note needs about a
/// minute on two threads.
pub const MIN_DEADLINE: Duration = Duration::from_secs(60);

/// How long one note may take, decoding included: `engine::MAX_SECONDS`'
/// worth (`TRANSCRIBE_DEADLINE_SECONDS`).
pub const DEADLINE: Duration = Duration::from_secs(600);

const _: () = assert!(INTERVAL.as_secs() >= MIN_INTERVAL.as_secs() && DEADLINE.as_secs() >= MIN_DEADLINE.as_secs());

/// The whisper model, where compose mounts `TRANSCRIBE_MODELS_DIR`
/// (`infra/compose/file_host.yml`) and `nix/whisper-manager` downloads it
/// (`TRANSCRIBE_MODEL`).
pub const MODEL: &str = "/models/ggml-small-q5_1.bin";

/// The Silero VAD model that finds a note's speech, mounted beside it
/// (`TRANSCRIBE_VAD_MODEL`).
pub const VAD_MODEL: &str = "/models/ggml-silero-v5.1.2.bin";

/// Everything the worker needs, or `None` when the deployment has not turned
/// transcription on.
#[derive(Clone)]
pub struct TranscribeContext {
	pub engine: Arc<Whisper>,
	pub interval: Duration,
	/// How long one note may take, decoding included.
	pub deadline: Duration,
	/// Account deletion's lock, shared with `AppState::auth`.
	pub deletion: DeletionLock,
}

impl TranscribeContext {
	/// Validate the transcriber's configuration and load its model, once, at
	/// startup.
	///
	/// Off is fine and logged. On with anything missing or unusable is a hard
	/// error, as `MAILBOX_ENABLED` without its keys is: the only symptom of a
	/// transcriber that never runs is a routine that never gets its text.
	///
	/// # Errors
	/// `TRANSCRIBE_ENABLED` with an engine that does not load
	/// (`engine::Whisper::load`).
	pub fn from_config(config: &Config, deletion: DeletionLock) -> anyhow::Result<Option<Self>> {
		if !config.transcribe_enabled {
			info!("voice-note transcription is not enabled; home keeps notes without hearing them");
			return Ok(None);
		}
		let engine = Whisper::load(&PathBuf::from(MODEL), &PathBuf::from(VAD_MODEL), default_threads())?;
		Ok(Some(Self {
			engine: Arc::new(engine),
			interval: INTERVAL,
			deadline: DEADLINE,
			deletion,
		}))
	}
}

/// One fewer than the cores this process may use, and at least one, so
/// hearing a note leaves a core for requests. Counts the container's CPU
/// quota, not the host's cores.
fn default_threads() -> usize {
	std::thread::available_parallelism().map_or(1, |cores| cores.get().saturating_sub(1).max(1))
}

/// Start the worker if the deployment turned it on, and say which on
/// `transcribe_enabled` either way.
pub fn start(state: &AppState) {
	crate::metrics::transcribe::record_enabled(state.transcribe.is_some());
	if let Some(context) = state.transcribe.clone() {
		spawn(state, context);
	}
}

/// Spawn the worker, cancelled through the shared token. Its first pass runs
/// at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: TranscribeContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	crate::metrics::transcribe::record_started();

	tokio::spawn(async move {
		info!(interval_secs = context.interval.as_secs(), "voice-note transcriber started");
		let mut ticker = tokio::time::interval(context.interval);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("voice-note transcriber cancelled");
					return;
				}
				_ = ticker.tick() => {
					crate::metrics::transcribe::record_pass_started();
					let result = pass::run_once(&db, &context.engine, &context.deletion, context.deadline, &cancel, &crate::mailbox::pass::now).await;
					crate::metrics::transcribe::record_pass(result.is_ok());
					log_pass(&result);
				}
			}
		}
	});
}

/// The pass's one log line, when it did anything: the kind of each attempt,
/// counted. Never a note's text, audio, id or subject.
pub(crate) fn log_pass(result: &Result<pass::Pass, voice_repo::VoiceError>) {
	match result {
		Ok(done) if done.kept.is_empty() => {}
		Ok(done) => {
			let count = |kind: &str| done.kept.iter().filter(|kept| **kept == kind).count();
			info!(
				heard = count("heard"),
				silent = count("silent"),
				unreadable = count("unreadable"),
				too_long = count("too_long"),
				deadline = count("deadline"),
				failed = count("failed"),
				cancelled = done.cancelled,
				"voice-note transcription pass"
			);
		}
		Err(err) => error!(error = kind(err), "voice-note transcription pass failed"),
	}
}

/// What failed, without the database's own message.
const fn kind(err: &voice_repo::VoiceError) -> &'static str {
	match err {
		voice_repo::VoiceError::Storage(_) => "storage",
		voice_repo::VoiceError::Rewritten | voice_repo::VoiceError::Unreadable => "unexpected",
	}
}
