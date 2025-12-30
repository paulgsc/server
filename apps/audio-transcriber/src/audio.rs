use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

use crate::observability::{Heartbeat, TranscriberMetrics};
use crate::state::TranscriberState;

pub struct AudioProcessor {
	buffer: Vec<f32>,
	buffer_capacity: usize,
	target_sample_rate: u32,
	known_sample_rate: Option<u32>,
	known_channels: Option<u32>,
	state: Arc<TranscriberState>,
	metrics: TranscriberMetrics,
	heartbeat: Heartbeat,
}

impl AudioProcessor {
	pub fn new(buffer_capacity: usize, target_sample_rate: u32, state: Arc<TranscriberState>, metrics: TranscriberMetrics) -> Self {
		Self {
			buffer: Vec::with_capacity(buffer_capacity * 2),
			buffer_capacity,
			target_sample_rate,
			known_sample_rate: None,
			known_channels: None,
			state,
			metrics,
			heartbeat: Heartbeat::new(30),
		}
	}

	pub async fn process_chunk(&mut self, sample_rate: u32, channels: u32, samples: Vec<f32>) -> Result<()> {
		let chunk_start = Instant::now();
		let chunk_bytes = samples.len() * std::mem::size_of::<f32>();

		// Update metrics
		self.state.chunks_received.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		self.state.bytes_received.fetch_add(chunk_bytes as u64, std::sync::atomic::Ordering::Relaxed);
		self.metrics.chunks_received.add(1, &[]);
		self.metrics.bytes_received.add(chunk_bytes as u64, &[]);

		// Detect audio format
		if sample_rate > 0 {
			if self.known_sample_rate.is_none() {
				info!(sample_rate, channels, "📊 Audio format detected");
				self.state.update_sample_rate(sample_rate);
			}
			self.known_sample_rate = Some(sample_rate);
			self.known_channels = Some(channels);
		}

		let actual_sample_rate = self.known_sample_rate.unwrap_or(sample_rate);
		let actual_channels = self.known_channels.unwrap_or(channels);

		// Process audio
		let processed_samples = self.process_samples(samples, actual_sample_rate, actual_channels).await;

		self.state.samples_processed.fetch_add(processed_samples as u64, std::sync::atomic::Ordering::Relaxed);
		self.metrics.samples_processed.add(processed_samples as u64, &[]);

		// Record latency
		let chunk_latency = chunk_start.elapsed().as_secs_f64() * 1000.0;
		self.metrics.chunk_processing_latency.record(chunk_latency, &[]);

		// Update buffer size
		self.state.update_buffer_size(self.buffer.len());

		// Periodic stats
		let chunks = self.state.chunks_received.load(std::sync::atomic::Ordering::Relaxed);
		if chunks % 50 == 0 {
			debug!(
				chunks,
				buffer_size = self.buffer.len(),
				is_transcribing = self.state.is_transcribing(),
				"📦 Processing stats"
			);
		}

		Ok(())
	}

	async fn process_samples(&mut self, samples: Vec<f32>, sample_rate: u32, channels: u32) -> usize {
		// Convert to mono if stereo
		let mono_samples: Vec<f32> = if channels == 2 {
			samples.chunks(2).map(|stereo| (stereo[0] + stereo[1]) / 2.0).collect()
		} else {
			samples
		};

		// Resample if needed
		let resampled = if sample_rate != self.target_sample_rate {
			let resample_start = Instant::now();
			let resampled = resample_simple(&mono_samples, sample_rate, self.target_sample_rate);
			let resample_latency = resample_start.elapsed().as_secs_f64() * 1000.0;
			self.metrics.resampling_latency.record(resample_latency, &[]);
			resampled
		} else {
			mono_samples
		};

		let sample_count = resampled.len();
		self.buffer.extend(resampled);
		sample_count
	}

	pub fn take_buffer_if_ready(&mut self) -> Option<Vec<f32>> {
		if self.buffer.len() >= self.buffer_capacity {
			let audio = self.buffer.clone();
			self.buffer.clear();
			self.state.update_buffer_size(0);
			Some(audio)
		} else {
			None
		}
	}

	pub fn heartbeat_check(&mut self) {
		if self.heartbeat.maybe_log(
			self.state.chunks_received.load(std::sync::atomic::Ordering::Relaxed),
			self.state.bytes_received.load(std::sync::atomic::Ordering::Relaxed),
			self.state.samples_processed.load(std::sync::atomic::Ordering::Relaxed),
			self.buffer.len(),
			self.state.transcriptions_completed.load(std::sync::atomic::Ordering::Relaxed),
		) {
			if self.state.is_transcribing() {
				info!("⚙️  Transcription in progress (CPU busy)");
			}
		}
	}
}

fn resample_simple(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
	if from_rate == to_rate {
		return samples.to_vec();
	}

	let ratio = from_rate as f32 / to_rate as f32;
	let output_len = (samples.len() as f32 / ratio) as usize;

	(0..output_len)
		.map(|i| {
			let src_idx = (i as f32 * ratio) as usize;
			samples.get(src_idx).copied().unwrap_or(0.0)
		})
		.collect()
}
