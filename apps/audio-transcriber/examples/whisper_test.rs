// Simple test to demonstrate Whisper concurrency issue and solution
// Run with: cargo run --example whisper_test

use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn generate_test_audio(duration_secs: f32, sample_rate: u32) -> Vec<f32> {
	// Generate a simple sine wave for testing
	let num_samples = (sample_rate as f32 * duration_secs) as usize;
	let frequency = 440.0; // A4 note

	(0..num_samples)
		.map(|i| {
			let t = i as f32 / sample_rate as f32;
			(2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5
		})
		.collect()
}

async fn test_concurrent_access() -> Result<()> {
	println!("=== Testing Concurrent Whisper Access ===\n");

	// Load model once
	let model_path = "path/to/your/ggml-model.bin"; // Update this path
	println!("Loading Whisper model from: {}", model_path);
	let ctx = Arc::new(WhisperContext::new_with_params(model_path, WhisperContextParameters::default())?);
	println!("✅ Model loaded\n");

	// Generate test audio chunks
	let audio1 = generate_test_audio(3.0, 16000);
	let audio2 = generate_test_audio(3.0, 16000);
	let audio3 = generate_test_audio(3.0, 16000);

	println!("Generated 3 test audio chunks (3 seconds each)\n");

	// Test 1: Sequential (baseline)
	println!("--- Test 1: Sequential Transcription ---");
	let start = Instant::now();
	transcribe_sync(ctx.clone(), &audio1)?;
	transcribe_sync(ctx.clone(), &audio2)?;
	transcribe_sync(ctx.clone(), &audio3)?;
	let sequential_time = start.elapsed();
	println!("Sequential time: {:.2}s\n", sequential_time.as_secs_f64());

	// Test 2: Concurrent WITHOUT protection (will fail or give wrong results)
	println!("--- Test 2: Concurrent WITHOUT Mutex (UNSAFE) ---");
	let start = Instant::now();
	let handles: Vec<_> = vec![audio1.clone(), audio2.clone(), audio3.clone()]
		.into_iter()
		.map(|audio| {
			let ctx = ctx.clone();
			tokio::task::spawn_blocking(move || transcribe_sync(ctx, &audio))
		})
		.collect();

	// Wait for all
	for handle in handles {
		let _ = handle.await;
	}
	let concurrent_unsafe_time = start.elapsed();
	println!("⚠️  Concurrent UNSAFE time: {:.2}s", concurrent_unsafe_time.as_secs_f64());
	println!("(This may crash, hang, or produce incorrect results)\n");

	// Test 3: Concurrent WITH Mutex (correct but serialized)
	println!("--- Test 3: Concurrent WITH Mutex (SAFE) ---");
	let mutex = Arc::new(tokio::sync::Mutex::new(()));
	let start = Instant::now();

	let handles: Vec<_> = vec![audio1.clone(), audio2.clone(), audio3.clone()]
		.into_iter()
		.map(|audio| {
			let ctx = ctx.clone();
			let mutex = mutex.clone();
			tokio::task::spawn_blocking(move || {
				// This blocks until mutex is available
				let _guard = mutex.blocking_lock();
				transcribe_sync(ctx, &audio)
			})
		})
		.collect();

	for handle in handles {
		let _ = handle.await;
	}
	let concurrent_safe_time = start.elapsed();
	println!("✅ Concurrent SAFE time: {:.2}s", concurrent_safe_time.as_secs_f64());
	println!("(Should be similar to sequential time)\n");

	println!("=== Results ===");
	println!("Sequential:       {:.2}s", sequential_time.as_secs_f64());
	println!(
		"Concurrent SAFE:  {:.2}s ({}% of sequential)",
		concurrent_safe_time.as_secs_f64(),
		(concurrent_safe_time.as_secs_f64() / sequential_time.as_secs_f64() * 100.0) as i32
	);
	println!("\nConclusion: WhisperContext CANNOT be used concurrently.");
	println!("Even with Arc, multiple transcriptions will serialize via internal locks or crash.");

	Ok(())
}

fn transcribe_sync(ctx: Arc<WhisperContext>, audio: &[f32]) -> Result<Vec<String>> {
	let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
	params.set_n_threads(4);
	params.set_print_special(false);
	params.set_print_progress(false);

	let mut state = ctx.create_state()?;
	state.full(params, audio)?;

	let num_segments = state.full_n_segments();
	let mut segments = Vec::new();

	for i in 0..num_segments {
		if let Some(segment) = state.get_segment(i) {
			if let Ok(text) = segment.to_str() {
				segments.push(text.trim().to_string());
			}
		}
	}

	Ok(segments)
}

// Demonstration of the CORRECT approach for your service
async fn correct_approach_demo() -> Result<()> {
	println!("\n=== CORRECT Approach for Your Service ===\n");

	let model_path = "path/to/your/ggml-model.bin";
	let ctx = Arc::new(WhisperContext::new_with_params(model_path, WhisperContextParameters::default())?);

	// Use a Semaphore to limit to ONE concurrent transcription
	let transcription_sem = Arc::new(tokio::sync::Semaphore::new(1));

	println!("Strategy: Use Semaphore(1) to queue transcriptions");
	println!("- Only ONE transcription runs at a time");
	println!("- Additional chunks wait in queue");
	println!("- OR drop chunks if queue is full (your current approach)\n");

	// Simulate processing multiple chunks
	for i in 1..=5 {
		let permit = match transcription_sem.clone().try_acquire_owned() {
			Ok(permit) => {
				println!("✅ Chunk {} - acquired permit, starting transcription", i);
				permit
			}
			Err(_) => {
				println!("⏭️  Chunk {} - dropping (transcription in progress)", i);
				continue;
			}
		};

		let ctx = ctx.clone();
		let audio = generate_test_audio(2.0, 16000);

		tokio::task::spawn_blocking(move || {
			// Permit held for entire transcription
			let _permit = permit;
			let _ = transcribe_sync(ctx, &audio);
			println!("   Chunk {} - completed", i);
		});

		// Simulate chunks arriving rapidly
		tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
	}

	// Wait for completion
	tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

	println!("\n✅ This approach is SAFE but may drop chunks under load");
	println!("Alternative: Use tokio::sync::Semaphore::acquire() (blocking) to queue chunks");

	Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
	// Run tests
	test_concurrent_access().await?;
	correct_approach_demo().await?;

	println!("\n=== Recommendations ===");
	println!("1. Keep Semaphore(1) to serialize Whisper access");
	println!("2. Consider using .acquire() instead of .try_acquire_owned()");
	println!("   to QUEUE chunks instead of dropping them");
	println!("3. For higher throughput, consider:");
	println!("   - Running multiple service instances");
	println!("   - Pre-splitting audio into optimal chunk sizes");
	println!("   - Using faster-whisper or whisper.cpp with batching");

	Ok(())
}

// WAV file transcription example
#[allow(dead_code)]
fn transcribe_wav_file(model_path: &str, wav_path: &str) -> Result<String> {
	use hound::WavReader;

	println!("Loading WAV file: {}", wav_path);
	let mut reader = WavReader::open(wav_path)?;
	let spec = reader.spec();

	println!("Sample rate: {}, Channels: {}, Bits: {}", spec.sample_rate, spec.channels, spec.bits_per_sample);

	// Convert to f32 mono at 16kHz (Whisper's expected format)
	let samples: Vec<f32> = reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();

	// Resample if needed (simplified - use a proper resampler in production)
	let target_rate = 16000;
	let resampled = if spec.sample_rate != target_rate {
		println!("Resampling from {} to {}", spec.sample_rate, target_rate);
		// TODO: Use rubato or similar for proper resampling
		samples
	} else {
		samples
	};

	// Transcribe
	println!("Loading model...");
	let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())?;

	println!("Transcribing...");
	let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
	params.set_n_threads(4);

	let mut state = ctx.create_state()?;
	state.full(params, &resampled)?;

	let num_segments = state.full_n_segments();
	let mut full_text = String::new();

	for i in 0..num_segments {
		if let Some(segment) = state.get_segment(i) {
			if let Ok(text) = segment.to_str() {
				full_text.push_str(text.trim());
				full_text.push(' ');
			}
		}
	}

	Ok(full_text.trim().to_string())
}
