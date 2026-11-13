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
//!   attempt and how many, and how many takes it gave a vector, never a
//!   note's text, audio, vector, id or subject.
//! - **Interview takes are heard verbatim** (`verbatim`, paulgsc/some-ui#2933,
//!   IW2): with a prompt that keeps their fillers, kept under the model's
//!   verbatim key. Same model, same worker, same class: nothing new runs on
//!   home.
//! - **The take library's meaning index is written here** (`meaning`,
//!   docs/take-library.md), when `MEANING_ENABLED`: a take's vector in the
//!   transaction that keeps its hearing, and the takes heard before the
//!   index existed a bounded batch a pass (`pass`).
//!
//! **What it costs.** A multilingual `small` model at `q5_1` holds about
//! 500 MB while loaded, for as long as the server runs, and hears a minute
//! of speech in about a minute on two threads. Off unless
//! `TRANSCRIBE_ENABLED`; on, the server refuses to start without a model,
//! the VAD model and ffmpeg.

pub mod engine;
pub mod pass;
pub mod verbatim;

use crate::auth::DeletionLock;
use crate::call_weeks::lane::Held;
use crate::meaning::{MeaningContext, StaticModel};
use crate::{AppState, Config};
use engine::{Cancelled, Decode, Hear, Heard, Whisper};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use voice_repo::meaning::Embed;

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
	/// The take library's meaning model, when `MEANING_ENABLED`: each pass
	/// embeds the words of each take it hears, in the transaction that keeps
	/// the hearing, and catches up on takes heard before (`pass`).
	pub meaning: Option<Arc<StaticModel>>,
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
	pub fn from_config(config: &Config, deletion: DeletionLock, meaning: Option<&MeaningContext>) -> anyhow::Result<Option<Self>> {
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
			meaning: meaning.map(|meaning| Arc::clone(&meaning.model)),
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

/// The engine, holding interactive work while it hears a note, so the batch
/// lane pauses a render for it (`call_weeks::lane`, LA1). A pass with
/// nothing to hear holds nothing.
impl<H: Hear> Hear for Held<H> {
	fn model(&self) -> &str {
		self.inner.model()
	}

	fn engine(&self) -> &str {
		self.inner.engine()
	}

	fn hear(&self, audio: &[u8], decode: Decode, deadline: Instant, cancel: &CancellationToken) -> Result<Heard, Cancelled> {
		let _hold = self.interactive.hold();
		self.inner.hear(audio, decode, deadline, cancel)
	}
}

/// Spawn the worker, cancelled through the shared token. Its first pass runs
/// at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: TranscribeContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	let engine = Arc::new(Held {
		inner: Arc::clone(&context.engine),
		interactive: state.interactive.clone(),
	});
	crate::metrics::transcribe::record_started();
	crate::metrics::contention::record("transcribe", crate::metrics::contention::Counts::default());
	// A note heard is a transcript on its way to the mailbox: wake its pass
	// as each is kept, rather than leave it for the backstop (MBP2).
	let outbox = <crate::mailbox::Outbox as axum::extract::FromRef<AppState>>::from_ref(state);

	tokio::spawn(async move {
		info!(interval_secs = context.interval.as_secs(), "voice-note transcriber started");
		let mut ticker = tokio::time::interval(context.interval);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut guard = crate::contention::Guard::host();
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("voice-note transcriber cancelled");
					return;
				}
				_ = ticker.tick() => {
					crate::metrics::transcribe::record_pass_started();
					let meaning = context.meaning.as_deref().map(|model| model as &dyn Embed);
					// Each transcript heard wakes the mailbox's pass as soon as
					// it is kept, not when the batch ends (MBP2).
					let filed = || outbox.filed();
					let result = pass::run_once_telling(&db, &engine, meaning, &context.deletion, context.deadline, &mut guard, &cancel, &crate::mailbox::pass::now, &filed).await;
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
	if let Ok(done) = result {
		let guard = crate::metrics::contention::Counts {
			excused: done.excused,
			judged_anyway: done.judged_anyway,
			marked: 0,
			unreadable: done.unreadable,
		};
		crate::metrics::contention::record("transcribe", guard);
		if done.excused > 0 {
			info!("voice-note transcription gave way: a note ran to its deadline while the box was contended, so it is nobody's and heard again next pass");
		}
		if done.judged_anyway > 0 {
			warn!(
				judged_anyway = done.judged_anyway,
				"voice-note transcription kept a deadline reached on a contended box: its note was excused as many times in a row as the guard allows"
			);
		}
	}
	match result {
		Ok(done) if done.kept.is_empty() && done.indexed == 0 => {}
		Ok(done) => {
			let count = |kind: &str| done.kept.iter().filter(|kept| **kept == kind).count();
			info!(
				heard = count("heard"),
				silent = count("silent"),
				unreadable = count("unreadable"),
				too_long = count("too_long"),
				deadline = count("deadline"),
				failed = count("failed"),
				indexed = done.indexed,
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

#[cfg(test)]
mod tests {
	use super::{Cancelled, Decode, Hear, Heard, Held};
	use crate::call_weeks::lane::Interactive;
	use std::sync::{Arc, Mutex};
	use std::time::Instant;
	use tokio_util::sync::CancellationToken;
	use voice_repo::transcript::Outcome;

	/// An engine that notes whether interactive work was held as it heard.
	struct Watch {
		interactive: Interactive,
		held: Mutex<Vec<bool>>,
	}

	impl Hear for Watch {
		fn model(&self) -> &'static str {
			"model"
		}

		fn engine(&self) -> &'static str {
			"engine"
		}

		fn hear(&self, _: &[u8], _: Decode, _: Instant, _: &CancellationToken) -> Result<Heard, Cancelled> {
			self.held.lock().unwrap().push(self.interactive.is_held());
			Ok(Heard {
				outcome: Outcome::Silent,
				duration_ms: None,
			})
		}
	}

	#[test]
	fn a_note_heard_is_interactive_work_until_it_is_heard() {
		let interactive = Interactive::default();
		let watch = Arc::new(Watch {
			interactive: interactive.clone(),
			held: Mutex::new(Vec::new()),
		});
		let engine = Held {
			inner: Arc::clone(&watch),
			interactive: interactive.clone(),
		};
		assert_eq!((engine.model(), engine.engine()), ("model", "engine"));
		assert!(!interactive.is_held());
		assert!(engine.hear(b"note", Decode::Plain, Instant::now(), &CancellationToken::new()).is_ok());
		assert_eq!(*watch.held.lock().unwrap(), [true]);
		assert!(!interactive.is_held(), "let go once heard");
	}
}
