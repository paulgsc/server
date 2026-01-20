use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Global state for transcriber metrics and status
pub struct TranscriberState {
	// Audio ingestion metrics
	pub chunks_received: AtomicU64,
	pub bytes_received: AtomicU64,
	pub samples_processed: AtomicU64,

	// Transcription metrics
	pub transcriptions_completed: AtomicU64,
	pub transcriptions_failed: AtomicU64,
	pub subtitles_published: AtomicU64,

	// Queue metrics (new)
	pub jobs_enqueued: AtomicU64,
	pub jobs_dropped: AtomicU64,
	pub queue_depth: AtomicUsize,

	// Buffer state
	pub buffer_size: AtomicUsize,
	pub current_sample_rate: AtomicU64,

	// Worker state
	pub is_transcribing: AtomicBool,
}

impl Default for TranscriberState {
	fn default() -> Self {
		Self {
			chunks_received: AtomicU64::new(0),
			bytes_received: AtomicU64::new(0),
			samples_processed: AtomicU64::new(0),
			transcriptions_completed: AtomicU64::new(0),
			transcriptions_failed: AtomicU64::new(0),
			subtitles_published: AtomicU64::new(0),
			jobs_enqueued: AtomicU64::new(0),
			jobs_dropped: AtomicU64::new(0),
			queue_depth: AtomicUsize::new(0),
			buffer_size: AtomicUsize::new(0),
			current_sample_rate: AtomicU64::new(0),
			is_transcribing: AtomicBool::new(false),
		}
	}
}

impl TranscriberState {
	pub fn new() -> Arc<Self> {
		Arc::new(Self::default())
	}

	/// Register OpenTelemetry gauge callbacks
	pub fn register_gauges(self: &Arc<Self>) -> Result<()> {
		let meter = opentelemetry::global::meter("transcriber");

		// Buffer size gauge
		let state_clone = Arc::clone(self);
		let _buffer_size_reg = meter
			.u64_observable_gauge("transcriber.buffer.size")
			.with_callback(move |observer| {
				let size = state_clone.buffer_size.load(Ordering::Relaxed) as u64;
				observer.observe(size, &[]);
			})
			.build();

		// Sample rate gauge
		let state_clone = Arc::clone(self);
		let _sample_rate_reg = meter
			.u64_observable_gauge("transcriber.sample_rate")
			.with_callback(move |observer| {
				let rate = state_clone.current_sample_rate.load(Ordering::Relaxed);
				observer.observe(rate, &[]);
			})
			.build();

		// Queue depth gauge (new)
		let state_clone = Arc::clone(self);
		let _queue_depth_reg = meter
			.u64_observable_gauge("transcriber.queue.depth")
			.with_callback(move |observer| {
				let depth = state_clone.queue_depth.load(Ordering::Relaxed) as u64;
				observer.observe(depth, &[]);
			})
			.build();

		// Worker busy gauge (new)
		let state_clone = Arc::clone(self);
		let _worker_busy_reg = meter
			.u64_observable_gauge("transcriber.worker.busy")
			.with_callback(move |observer| {
				let busy = if state_clone.is_transcribing.load(Ordering::Relaxed) { 1 } else { 0 };
				observer.observe(busy, &[]);
			})
			.build();

		// Heartbeat gauge
		let _heartbeat_reg = meter
			.u64_observable_gauge("transcriber.heartbeat")
			.with_callback(move |observer| {
				let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
				observer.observe(timestamp, &[]);
			})
			.build();

		Ok(())
	}

	// Convenience methods
	pub fn set_transcribing(&self, value: bool) {
		self.is_transcribing.store(value, Ordering::Relaxed);
	}

	pub fn is_transcribing(&self) -> bool {
		self.is_transcribing.load(Ordering::Relaxed)
	}

	pub fn update_buffer_size(&self, size: usize) {
		self.buffer_size.store(size, Ordering::Relaxed);
	}

	pub fn update_sample_rate(&self, rate: u32) {
		self.current_sample_rate.store(rate as u64, Ordering::Relaxed);
	}

	// Queue management methods (new)
	pub fn increment_jobs_enqueued(&self) {
		self.jobs_enqueued.fetch_add(1, Ordering::Relaxed);
	}

	pub fn increment_jobs_dropped(&self) {
		self.jobs_dropped.fetch_add(1, Ordering::Relaxed);
	}

	pub fn update_queue_depth(&self, depth: usize) {
		self.queue_depth.store(depth, Ordering::Relaxed);
	}
}
