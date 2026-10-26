//! What hears a note: ffmpeg decodes it to 16 kHz mono, whisper.cpp's Silero
//! VAD finds its speech, and Whisper hears that speech.
//!
//! **In-process whisper.cpp, through `whisper-rs`, rather than a sidecar.**
//! whisper.cpp builds from source with the binary (cmake, no GPU), so the
//! image carries no second program, and the bindings give what a sidecar's
//! printed line does not: the detected language's probability, and an abort
//! callback that ends a run at its deadline. ffmpeg stays a separate program,
//! because no Rust decoder covers opus in webm and AAC in mp4 together.
//!
//! **Silence never comes back as words.** Whisper hears "you" or "Thank you."
//! in silence and glyph runs in hiss (the routines met both with
//! faster-whisper, which is why theirs run `vad_filter`). Here the VAD runs
//! first and Whisper hears only the speech it found: none is
//! [`Outcome::Silent`], and Whisper is not run at all.
//!
//! **The audio touches disk once, privately.** ffmpeg cannot read an mp4
//! whose index comes last (as the phone's `m4a` does) from a pipe, so each
//! note is written to a fresh `0700` temporary directory, decoded beside it,
//! and both are removed when the attempt ends, whatever its outcome
//! (docs/identity.md invariant 23).
//!
//! **A deadline is kept to within one encoder pass.** ffmpeg is killed at
//! the deadline; whisper.cpp checks its abort callback between passes, not
//! inside one, so a run ends up to one pass late (about 9 seconds on two
//! threads), and language detection, one pass, is not interruptible at all.
//! Against `TRANSCRIBE_DEADLINE_SECONDS`' default of ten minutes, that is
//! noise.
//!
//! **Nothing heard is logged.** whisper.cpp's own log goes nowhere
//! (`whisper_rs::install_logging_hooks` with no log backend), ffmpeg's output
//! is discarded, and this module returns an outcome, never a message.

use std::ffi::{c_void, OsStr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use voice_repo::transcript::Outcome;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

/// Whisper hears 16 kHz mono.
pub const SAMPLE_RATE: usize = 16_000;

/// The longest note the worker hears: ten minutes.
///
/// That is well past the phone's one-minute takes and the four minutes
/// `voice_repo::NOTE_CEILING` holds at its bitrate. Longer is
/// [`Outcome::TooLong`], so a note encoded at a tiny bitrate cannot decode to
/// hours of samples.
pub const MAX_SECONDS: usize = 600;

/// [`MAX_SECONDS`] and one more, as ffmpeg's `-t` takes it: a note decodes
/// that far at most, so one longer reads as too long without decoding all of
/// it.
const DECODE_SECONDS: &str = "601";

/// The program that decodes, found on `PATH` (the image installs it).
const FFMPEG: &str = "ffmpeg";

/// How often a running decode is checked against its deadline.
const DECODE_POLL: Duration = Duration::from_millis(20);

/// What fetches the models, as example.env says it.
const FETCH: &str = "nix develop .#whisper -c whisper fetch";

/// How a note was heard, and how long its audio is when it decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
	pub outcome: Outcome,
	pub duration_ms: Option<i64>,
}

impl Heard {
	const fn without_audio(outcome: Outcome) -> Self {
		Self { outcome, duration_ms: None }
	}
}

/// The server is shutting down: the attempt is abandoned, and nothing about
/// it is kept, since it says nothing about the note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// Something that hears notes. [`Whisper`] in the server; a fake in tests.
pub trait Hear: Send + Sync + 'static {
	/// The model file it hears with (`ggml-small-q5_1.bin`).
	fn model(&self) -> &str;

	/// What runs it (`whisper.cpp 1.8.3`).
	fn engine(&self) -> &str;

	/// Hears `audio`, a note's bytes as the phone sent them, by `deadline`.
	/// Blocking: call it off the async runtime.
	///
	/// # Errors
	/// [`Cancelled`] once `cancel` fires.
	fn hear(&self, audio: &[u8], deadline: Instant, cancel: &CancellationToken) -> Result<Heard, Cancelled>;
}

/// whisper.cpp with a loaded model, and the VAD model it finds speech with.
pub struct Whisper {
	context: WhisperContext,
	vad_model: String,
	model: String,
	engine: String,
	threads: usize,
}

impl Whisper {
	/// Loads `model` and checks that `vad_model` loads and ffmpeg runs, so a
	/// deployment that cannot hear fails at startup rather than at its first
	/// note. Errors name the variable, never its value.
	///
	/// # Errors
	/// A model or VAD model that is not there, is not a file or cannot be
	/// looked at, a model that does not load or
	/// is English-only, a VAD model that does not load, a path that is not
	/// UTF-8, no ffmpeg, or no threads.
	pub fn load(model: &Path, vad_model: &Path, threads: usize) -> anyhow::Result<Self> {
		whisper_rs::install_logging_hooks();
		anyhow::ensure!(threads > 0, "TRANSCRIBE_THREADS must be at least 1");
		let Some(model_path) = model.to_str() else {
			anyhow::bail!("TRANSCRIBE_MODEL is not a UTF-8 path");
		};
		let Some(vad_path) = vad_model.to_str() else {
			anyhow::bail!("TRANSCRIBE_VAD_MODEL is not a UTF-8 path");
		};
		// Both, before either loads: whisper.cpp refuses a missing file as it
		// does a corrupt one, and a box that never fetched the models should
		// hear how to get them (#476).
		model_file(model, "TRANSCRIBE_MODEL")?;
		model_file(vad_model, "TRANSCRIBE_VAD_MODEL")?;
		let mut parameters = WhisperContextParameters::default();
		parameters.use_gpu(false);
		let context = WhisperContext::new_with_params(model_path, parameters).map_err(|_| anyhow::anyhow!("TRANSCRIBE_MODEL does not load as a whisper.cpp model"))?;
		// The notes mix Korean into English, and an `.en` model garbles it.
		anyhow::ensure!(context.is_multilingual(), "TRANSCRIBE_MODEL is an English-only model; it must be multilingual (no `.en`)");
		WhisperVadContext::new(vad_path, vad_context(threads)?).map_err(|_| anyhow::anyhow!("TRANSCRIBE_VAD_MODEL does not load as a whisper.cpp VAD model"))?;
		let ffmpeg = Command::new(FFMPEG)
			.arg("-version")
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status();
		anyhow::ensure!(ffmpeg.is_ok_and(|status| status.success()), "ffmpeg is not on PATH; the transcriber decodes with it");
		Ok(Self {
			context,
			vad_model: vad_path.to_owned(),
			model: model.file_name().and_then(OsStr::to_str).unwrap_or(model_path).to_owned(),
			engine: String::from("whisper.cpp ") + whisper_rs::get_whisper_version(),
			threads,
		})
	}

	/// The speech in `pcm`, as the VAD finds it, run end to end; empty when
	/// it finds none.
	fn speech(&self, pcm: &[f32]) -> Option<Vec<f32>> {
		let mut vad = WhisperVadContext::new(&self.vad_model, vad_context(self.threads).ok()?).ok()?;
		let segments = vad.segments_from_samples(WhisperVadParams::new(), pcm).ok()?;
		let mut speech = Vec::new();
		for segment in segments {
			// Centiseconds to samples.
			let at = |centiseconds: f32| sample_at(centiseconds, pcm.len());
			let (start, end) = (at(segment.start), at(segment.end));
			if start < end {
				speech.extend_from_slice(&pcm[start..end]);
			}
		}
		Some(speech)
	}

	/// What Whisper hears in `speech`, by `deadline`.
	fn transcribe(&self, speech: &[f32], deadline: Instant, cancel: &CancellationToken) -> Result<Outcome, Cancelled> {
		let failed = |cancel: &CancellationToken| {
			if cancel.is_cancelled() {
				Err(Cancelled)
			} else if Instant::now() >= deadline {
				Ok(Outcome::Deadline)
			} else {
				Ok(Outcome::Failed)
			}
		};
		let Ok(threads) = i32::try_from(self.threads) else {
			return Ok(Outcome::Failed);
		};
		let Ok(mut state) = self.context.create_state() else {
			return failed(cancel);
		};
		// The language once, from the first 30 seconds of speech, with its
		// probability, which `full` does not report; then Whisper hears the
		// whole note in it. The probability costs an encoder pass of its own,
		// about 40% of a 20-second note's time on two threads.
		if state.pcm_to_mel(speech, self.threads).is_err() {
			return failed(cancel);
		}
		let Ok((id, probabilities)) = state.lang_detect(0, self.threads) else {
			return failed(cancel);
		};
		let (Some(language), Some(probability)) = (whisper_rs::get_lang_str(id), usize::try_from(id).ok().and_then(|id| probabilities.get(id))) else {
			return Ok(Outcome::Failed);
		};
		if cancel.is_cancelled() {
			return Err(Cancelled);
		}
		if Instant::now() >= deadline {
			return Ok(Outcome::Deadline);
		}
		let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
		params.set_n_threads(threads);
		params.set_language(Some(language));
		params.set_print_special(false);
		params.set_print_progress(false);
		params.set_print_realtime(false);
		params.set_print_timestamps(false);
		// Non-speech tokens (music notes, bracketed sounds) are never text, as
		// faster-whisper's defaults have it; without this, a note's last words
		// drift into stock phrases.
		params.set_suppress_nst(true);
		let watch = Watch { deadline, cancel };
		// SAFETY: `should_abort` reads its data as a `Watch`, and `watch` lives
		// on this stack, unmoved, until `full` below returns; `full` is the one
		// call that runs the callback, and it consumes `params`.
		unsafe {
			params.set_abort_callback(Some(should_abort));
			params.set_abort_callback_user_data(std::ptr::from_ref(&watch).cast_mut().cast());
		}
		if state.full(params, speech).is_err() {
			return failed(cancel);
		}
		let mut text = String::new();
		for segment in state.as_iter() {
			let Ok(said) = segment.to_str_lossy() else {
				return Ok(Outcome::Failed);
			};
			let said = said.trim();
			if !said.is_empty() {
				if !text.is_empty() {
					text.push(' ');
				}
				text.push_str(said);
			}
		}
		if text.is_empty() {
			return Ok(Outcome::Silent);
		}
		Ok(Outcome::Heard {
			language: language.to_owned(),
			language_p: f64::from(*probability),
			text,
		})
	}
}

impl Hear for Whisper {
	fn model(&self) -> &str {
		&self.model
	}

	fn engine(&self) -> &str {
		&self.engine
	}

	fn hear(&self, audio: &[u8], deadline: Instant, cancel: &CancellationToken) -> Result<Heard, Cancelled> {
		let pcm = match decode(audio, deadline, cancel)? {
			Decoded::Pcm(pcm) => pcm,
			Decoded::Unreadable => return Ok(Heard::without_audio(Outcome::Unreadable)),
			Decoded::TooLong => return Ok(Heard::without_audio(Outcome::TooLong)),
			Decoded::Deadline => return Ok(Heard::without_audio(Outcome::Deadline)),
		};
		let duration_ms = i64::try_from(pcm.len() * 1000 / SAMPLE_RATE).ok();
		let heard = |outcome| Heard { outcome, duration_ms };
		let Some(speech) = self.speech(&pcm) else {
			return Ok(heard(Outcome::Failed));
		};
		if speech.is_empty() {
			return Ok(heard(Outcome::Silent));
		}
		if cancel.is_cancelled() {
			return Err(Cancelled);
		}
		if Instant::now() >= deadline {
			return Ok(heard(Outcome::Deadline));
		}
		self.transcribe(&speech, deadline, cancel).map(heard)
	}
}

/// What whisper.cpp's abort callback reads while Whisper hears a note: the
/// attempt's deadline, and the server's shutdown.
struct Watch<'a> {
	deadline: Instant,
	cancel: &'a CancellationToken,
}

/// whisper.cpp's abort callback, checked as it computes: true ends the run.
///
/// Not `whisper-rs`'s `set_abort_callback_safe`: in 0.16.0 it hands C a
/// pointer to a `Box<dyn FnMut>` and reads it back as the closure's own type,
/// which hung the first run on real notes. This one reads back exactly what
/// it was given.
unsafe extern "C" fn should_abort(data: *mut c_void) -> bool {
	// SAFETY: `data` is the `Watch` `Whisper::transcribe` registered with this
	// callback, alive until the run that calls this returns, and only read.
	let watch = unsafe { &*data.cast::<Watch<'_>>() };
	watch.cancel.is_cancelled() || Instant::now() >= watch.deadline
}

/// Refuses `path`, which `variable` names, unless a file is there: a path
/// with nothing at it, with a directory at it, or that cannot be looked at
/// (no permission on a directory above it) each says which. Errors name the
/// variable, never the path or the filesystem's error.
fn model_file(path: &Path, variable: &str) -> anyhow::Result<()> {
	match path.metadata() {
		Ok(metadata) if metadata.is_file() => Ok(()),
		Ok(_) => anyhow::bail!("{variable} is not a file: it must name the model file itself"),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => anyhow::bail!("{variable} is not there: run `{FETCH}`"),
		Err(_) => anyhow::bail!("{variable} cannot be looked at: check its directory's permissions"),
	}
}

fn vad_context(threads: usize) -> anyhow::Result<WhisperVadContextParams> {
	let mut params = WhisperVadContextParams::new();
	params.set_n_threads(i32::try_from(threads)?);
	params.set_use_gpu(false);
	Ok(params)
}

/// The sample `centiseconds` into a note of `len` samples (160 samples a
/// centisecond at 16 kHz), held within it.
fn sample_at(centiseconds: f32, len: usize) -> usize {
	// Rounded to a whole sample; a VAD timestamp is never negative or past
	// the note, and both ends are clamped in case it were.
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let sample = (centiseconds.max(0.0) * 160.0).round() as usize;
	sample.min(len)
}

/// A note decoded, or why not.
#[derive(Debug, PartialEq)]
enum Decoded {
	Pcm(Vec<f32>),
	Unreadable,
	TooLong,
	Deadline,
}

/// `audio` as 16 kHz mono samples, decoded by ffmpeg in a private temporary
/// directory that is removed on return.
fn decode(audio: &[u8], deadline: Instant, cancel: &CancellationToken) -> Result<Decoded, Cancelled> {
	let Ok(scratch) = scratch() else {
		return Ok(Decoded::Unreadable);
	};
	let (note, pcm) = (scratch.path().join("note"), scratch.path().join("pcm"));
	if std::fs::write(&note, audio).is_err() {
		return Ok(Decoded::Unreadable);
	}
	let Ok(child) = ffmpeg(&note, &pcm).spawn() else {
		return Ok(Decoded::Unreadable);
	};
	match finish(child, deadline, cancel)? {
		Finished::Succeeded => {}
		Finished::Failed => return Ok(Decoded::Unreadable),
		Finished::Deadline => return Ok(Decoded::Deadline),
	}
	let Ok(bytes) = std::fs::read(&pcm) else {
		return Ok(Decoded::Unreadable);
	};
	let samples: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
	Ok(if samples.is_empty() {
		Decoded::Unreadable
	} else if samples.len() > MAX_SECONDS * SAMPLE_RATE {
		Decoded::TooLong
	} else {
		Decoded::Pcm(samples)
	})
}

/// A fresh directory only this process's user can enter (`0700`), removed
/// when dropped. tempfile's own default is `0777` less the umask, `0755`
/// under the usual one, which would let any local user read a note while it
/// decodes.
fn scratch() -> std::io::Result<tempfile::TempDir> {
	use std::os::unix::fs::PermissionsExt as _;
	tempfile::Builder::new().prefix("voice-note-").permissions(std::fs::Permissions::from_mode(0o700)).tempdir()
}

/// ffmpeg decoding `note` into raw little-endian f32 at `pcm`, at most
/// [`DECODE_SECONDS`] of it. Silent: its output would name the file and
/// codec.
fn ffmpeg(note: &PathBuf, pcm: &PathBuf) -> Command {
	let mut command = Command::new(FFMPEG);
	command
		.args(["-nostdin", "-loglevel", "quiet", "-i"])
		.arg(note)
		.args(["-t", DECODE_SECONDS, "-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-y"])
		.arg(pcm)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null());
	command
}

enum Finished {
	Succeeded,
	Failed,
	Deadline,
}

/// Waits for `child`, killing it at `deadline` or once `cancel` fires.
fn finish(mut child: Child, deadline: Instant, cancel: &CancellationToken) -> Result<Finished, Cancelled> {
	loop {
		match child.try_wait() {
			Ok(Some(status)) => return Ok(if status.success() { Finished::Succeeded } else { Finished::Failed }),
			Ok(None) => {}
			Err(_) => {
				let _ = child.kill();
				let _ = child.wait();
				return Ok(Finished::Failed);
			}
		}
		let cancelled = cancel.is_cancelled();
		if cancelled || Instant::now() >= deadline {
			let _ = child.kill();
			let _ = child.wait();
			return if cancelled { Err(Cancelled) } else { Ok(Finished::Deadline) };
		}
		std::thread::sleep(DECODE_POLL);
	}
}

#[cfg(test)]
mod tests {
	//! The decode half runs against a real ffmpeg, which CI's runners and the
	//! image both have; a test skips when it is missing. Whisper itself needs
	//! a model file, so it is exercised through `pass`'s fake and by hand;
	//! what `Whisper::load` refuses, short of a real model, is tested here.

	use super::{decode, sample_at, Decoded, Whisper, DECODE_SECONDS, MAX_SECONDS, SAMPLE_RATE};
	use std::path::Path;
	use std::process::{Command, Stdio};
	use std::time::{Duration, Instant};
	use tokio_util::sync::CancellationToken;

	fn have_ffmpeg() -> bool {
		Command::new("ffmpeg")
			.arg("-version")
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.is_ok_and(|status| status.success())
	}

	/// `seconds` of a tone, encoded by ffmpeg as `format` (`webm`, `ogg` or
	/// `ipod`, the m4a muxer, whose index comes last as the phone's does).
	fn encoded(seconds: u32, format: &str, codec: &str) -> Vec<u8> {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("take");
		let status = Command::new("ffmpeg")
			.args(["-nostdin", "-loglevel", "quiet", "-f", "lavfi", "-i"])
			.arg(String::from("sine=frequency=440:sample_rate=48000:duration=") + &seconds.to_string())
			.args(["-c:a", codec, "-f", format, "-y"])
			.arg(&path)
			.status()
			.unwrap();
		assert!(status.success());
		std::fs::read(path).unwrap()
	}

	fn later() -> Instant {
		Instant::now() + Duration::from_secs(60)
	}

	#[test]
	fn the_phones_containers_decode_to_16_khz_mono() {
		if !have_ffmpeg() {
			return;
		}
		for (format, codec) in [("webm", "libopus"), ("ogg", "libopus"), ("ipod", "aac")] {
			let Ok(Decoded::Pcm(pcm)) = decode(&encoded(3, format, codec), later(), &CancellationToken::new()) else {
				panic!("{format} did not decode");
			};
			let seconds = pcm.len() / SAMPLE_RATE;
			assert!((2..=3).contains(&seconds), "{format}: {seconds} s");
		}
	}

	#[test]
	fn what_is_not_audio_is_unreadable_and_a_passed_deadline_stops_the_decode() {
		if !have_ffmpeg() {
			return;
		}
		assert_eq!(decode(b"not audio at all", later(), &CancellationToken::new()), Ok(Decoded::Unreadable));
		let take = encoded(3, "webm", "libopus");
		assert_eq!(decode(&take, Instant::now(), &CancellationToken::new()), Ok(Decoded::Deadline));
		let cancel = CancellationToken::new();
		cancel.cancel();
		assert_eq!(decode(&take, later(), &cancel), Err(super::Cancelled));
	}

	#[test]
	fn a_note_past_the_longest_is_too_long() {
		if !have_ffmpeg() {
			return;
		}
		let long = encoded(u32::try_from(MAX_SECONDS).unwrap() + 5, "ogg", "libopus");
		assert_eq!(decode(&long, later(), &CancellationToken::new()), Ok(Decoded::TooLong));
	}

	/// docs/identity.md invariant 23: the one place audio sits outside the
	/// tables is a directory no other user can enter.
	#[test]
	fn a_note_decodes_in_a_directory_only_its_owner_can_enter() {
		use std::os::unix::fs::PermissionsExt as _;
		let dir = super::scratch().unwrap();
		assert_eq!(dir.path().metadata().unwrap().permissions().mode() & 0o777, 0o700);
	}

	#[test]
	fn the_decode_stops_one_second_past_the_longest_note() {
		assert_eq!(DECODE_SECONDS.parse::<usize>().unwrap(), MAX_SECONDS + 1);
	}

	/// Why `Whisper::load` refused `model` and `vad_model`.
	fn refusal(model: &Path, vad_model: &Path) -> String {
		let Err(err) = Whisper::load(model, vad_model, 1) else {
			panic!("loaded");
		};
		err.to_string()
	}

	/// #476: a box that never fetched the models hears that they are not
	/// there, and how to get them, not that they do not load.
	#[test]
	fn a_model_that_is_not_there_says_so_and_how_to_get_it() {
		let dir = tempfile::tempdir().unwrap();
		let (model, vad) = (dir.path().join("ggml-small-q5_1.bin"), dir.path().join("ggml-silero-v5.1.2.bin"));
		assert_eq!(refusal(&model, &vad), "TRANSCRIBE_MODEL is not there: run `nix develop .#whisper -c whisper fetch`");
		std::fs::write(&model, b"not a model").unwrap();
		assert_eq!(refusal(&model, &vad), "TRANSCRIBE_VAD_MODEL is not there: run `nix develop .#whisper -c whisper fetch`");
	}

	#[test]
	fn a_directory_where_a_model_goes_is_not_a_model_file() {
		let dir = tempfile::tempdir().unwrap();
		let (model, vad) = (dir.path().join("ggml-small-q5_1.bin"), dir.path().join("ggml-silero-v5.1.2.bin"));
		std::fs::create_dir(&model).unwrap();
		assert_eq!(refusal(&model, &vad), "TRANSCRIBE_MODEL is not a file: it must name the model file itself");
	}

	/// A model behind a directory nobody may enter is not called missing, and
	/// the same for the VAD model.
	#[test]
	fn a_model_that_cannot_be_looked_at_says_so() {
		use std::os::unix::fs::PermissionsExt as _;
		let dir = tempfile::tempdir().unwrap();
		let locked = dir.path().join("locked");
		std::fs::create_dir(&locked).unwrap();
		// A file that is there, and a path nobody may look past.
		let (there, hidden) = (dir.path().join("ggml-small-q5_1.bin"), locked.join("ggml-silero-v5.1.2.bin"));
		std::fs::write(&there, b"not a model").unwrap();
		std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
		// Root ignores modes, so where the locked directory still reads (a
		// test run as root), there is nothing to show and the test skips.
		let refused = std::fs::read_dir(&locked).is_err().then(|| (refusal(&hidden, &there), refusal(&there, &hidden)));
		std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
		let Some((model_refused, vad_refused)) = refused else {
			return;
		};
		assert_eq!(model_refused, "TRANSCRIBE_MODEL cannot be looked at: check its directory's permissions");
		assert_eq!(vad_refused, "TRANSCRIBE_VAD_MODEL cannot be looked at: check its directory's permissions");
	}

	#[test]
	fn a_model_that_is_there_and_does_not_load_says_it_does_not_load() {
		let dir = tempfile::tempdir().unwrap();
		let (model, vad) = (dir.path().join("ggml-small-q5_1.bin"), dir.path().join("ggml-silero-v5.1.2.bin"));
		std::fs::write(&model, b"not a model").unwrap();
		std::fs::write(&vad, b"not a model either").unwrap();
		assert_eq!(refusal(&model, &vad), "TRANSCRIBE_MODEL does not load as a whisper.cpp model");
	}

	#[test]
	fn a_vad_timestamp_lands_inside_the_note() {
		assert_eq!(sample_at(0.0, 100), 0);
		assert_eq!(sample_at(0.5, 1_000), 80);
		assert_eq!(sample_at(-3.0, 1_000), 0);
		assert_eq!(sample_at(1e9, 1_000), 1_000);
	}
}
