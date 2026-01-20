use std::time::Instant;
use tokio::sync::mpsc;

/// A single unit of CPU work for Whisper
///
/// Once created, this job is immutable and self-describing.
/// It represents one irreversible transcription task.
#[derive(Debug, Clone)]
pub struct TranscriptionJob {
	/// Monotonic sequence number (global, not per-stream)
	/// Establishes total ordering and enables:
	/// - Detecting dropped jobs
	/// - Matching transcript order downstream
	/// - Debugging out-of-order arrivals
	pub seq: u64,

	/// Raw PCM samples, already resampled to target rate & mono
	/// Ownership transfers to worker - no shared references
	pub audio: Vec<f32>,

	/// Sample rate Whisper expects (usually 16000)
	/// Kept for self-documentation and future multi-source support
	pub sample_rate: u32,

	/// When this job became eligible for transcription
	/// Used to compute queue latency and end-to-end latency
	pub created_at: Instant,

	/// Optional correlation id (session / speaker / stream)
	/// Enables multi-session/multi-speaker tracking
	#[allow(dead_code)]
	pub stream_id: Option<String>,
}

impl TranscriptionJob {
	pub fn new(seq: u64, audio: Vec<f32>, sample_rate: u32, stream_id: Option<String>) -> Self {
		Self {
			seq,
			audio,
			sample_rate,
			created_at: Instant::now(),
			stream_id,
		}
	}

	/// Compute how long this job has been waiting
	pub fn queue_latency(&self) -> std::time::Duration {
		self.created_at.elapsed()
	}

	/// Audio duration in seconds
	pub fn audio_duration_secs(&self) -> f64 {
		self.audio.len() as f64 / self.sample_rate as f64
	}
}

/// Bounded queue for transcription jobs
///
/// Uses bounded MPSC to enforce backpressure instead of hiding overload
pub struct TranscriptionQueue {
	tx: mpsc::Sender<TranscriptionJob>,
	rx: Option<mpsc::Receiver<TranscriptionJob>>,
	#[allow(dead_code)]
	capacity: usize,
	#[allow(dead_code)]
	seq_counter: std::sync::atomic::AtomicU64,
}

impl TranscriptionQueue {
	pub fn new(capacity: usize) -> Self {
		let (tx, rx) = mpsc::channel(capacity);

		Self {
			tx,
			rx: Some(rx),
			capacity,
			seq_counter: std::sync::atomic::AtomicU64::new(0),
		}
	}

	/// Get the sender handle (for producers)
	pub fn sender(&self) -> mpsc::Sender<TranscriptionJob> {
		self.tx.clone()
	}

	/// Take the receiver (for the worker - can only be called once)
	pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<TranscriptionJob>> {
		self.rx.take()
	}

	/// Get next sequence number
	#[allow(dead_code)]
	pub fn next_seq(&self) -> u64 {
		self.seq_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
	}

	/// Get queue capacity
	#[allow(dead_code)]
	pub fn capacity(&self) -> usize {
		self.capacity
	}

	/// Try to enqueue a job (non-blocking)
	/// Returns Ok(seq) if enqueued, Err(job) if queue is full
	#[allow(dead_code)]
	pub fn try_enqueue(&self, mut job: TranscriptionJob) -> Result<u64, TranscriptionJob> {
		// Assign sequence number
		job.seq = self.next_seq();
		let seq = job.seq;

		match self.tx.try_send(job) {
			Ok(_) => Ok(seq),
			Err(mpsc::error::TrySendError::Full(job)) => Err(job),
			Err(mpsc::error::TrySendError::Closed(job)) => {
				// Receiver dropped - system is shutting down
				Err(job)
			}
		}
	}
}
