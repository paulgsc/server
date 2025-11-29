// Simple standalone WAV file transcription test
//
// Add to Cargo.toml:
// [dependencies]
// whisper-rs = "0.12"
// hound = "3.5"
// anyhow = "1.0"
//
// Usage: cargo run --example simple_wav_test -- /path/to/model.bin /path/to/audio.wav

use anyhow::{Context, Result};
use hound::WavReader;
use std::env;
use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn main() -> Result<()> {
	let args: Vec<String> = env::args().collect();

	if args.len() != 3 {
		eprintln!("Usage: {} <model_path> <wav_path>", args[0]);
		eprintln!("Example: {} ./models/ggml-base.en.bin ./test.wav", args[0]);
		std::process::exit(1);
	}

	let model_path = &args[1];
	let wav_path = &args[2];

	println!("=== Whisper WAV Transcription Test ===\n");

	// Step 1: Load WAV file
	println!("📂 Loading WAV file: {}", wav_path);
	let audio_data = load_wav(wav_path)?;
	println!("✅ Loaded {} samples ({:.2}s at 16kHz)\n", audio_data.len(), audio_data.len() as f32 / 16000.0);

	// Step 2: Load model
	println!("🔄 Loading Whisper model: {}", model_path);
	let load_start = Instant::now();
	let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default()).context("Failed to load Whisper model")?;
	println!("✅ Model loaded in {:.2}s\n", load_start.elapsed().as_secs_f32());

	// Step 3: Transcribe
	println!("🎤 Transcribing...");
	let transcribe_start = Instant::now();
	let transcription = transcribe(&ctx, &audio_data)?;
	let transcribe_time = transcribe_start.elapsed();

	println!("✅ Transcription completed in {:.2}s", transcribe_time.as_secs_f32());
	println!("   Real-time factor: {:.2}x\n", transcribe_time.as_secs_f32() / (audio_data.len() as f32 / 16000.0));

	// Step 4: Display results
	println!("=== TRANSCRIPTION ===");
	println!("{}", transcription);
	println!("\n=== END ===");

	Ok(())
}

fn load_wav(path: &str) -> Result<Vec<f32>> {
	let mut reader = WavReader::open(path).context("Failed to open WAV file")?;

	let spec = reader.spec();

	println!("   Sample rate: {} Hz", spec.sample_rate);
	println!("   Channels: {}", spec.channels);
	println!("   Bits per sample: {}", spec.bits_per_sample);
	println!("   Format: {:?}", spec.sample_format);

	// Read samples and convert to f32
	let samples: Result<Vec<f32>> = match spec.bits_per_sample {
		16 => reader
			.samples::<i16>()
			.map(|s| s.map(|sample| sample as f32 / 32768.0))
			.collect::<Result<Vec<f32>, _>>()
			.context("Failed to read i16 samples"),
		32 => reader
			.samples::<i32>()
			.map(|s| s.map(|sample| sample as f32 / 2147483648.0))
			.collect::<Result<Vec<f32>, _>>()
			.context("Failed to read i32 samples"),
		_ => anyhow::bail!("Unsupported bit depth: {}", spec.bits_per_sample),
	};

	let mut samples = samples?;

	// Convert stereo to mono if needed
	if spec.channels == 2 {
		println!("   Converting stereo to mono...");
		samples = samples.chunks_exact(2).map(|chunk| (chunk[0] + chunk[1]) / 2.0).collect();
	}

	// Resample to 16kHz if needed
	if spec.sample_rate != 16000 {
		println!("   ⚠️  Sample rate is {}, but Whisper expects 16000 Hz", spec.sample_rate);
		println!("   For best results, resample your audio to 16kHz before processing");
		println!("   (Continuing anyway, but results may be poor)");
	}

	Ok(samples)
}

fn transcribe(ctx: &WhisperContext, audio: &[f32]) -> Result<String> {
	// Configure transcription parameters
	let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

	// Performance settings
	params.set_n_threads(4); // Adjust based on your CPU

	// Output settings
	params.set_print_special(false);
	params.set_print_progress(false);
	params.set_print_realtime(false);
	params.set_print_timestamps(false);

	// Language settings
	params.set_language(Some("en")); // or None for auto-detect
	params.set_translate(false);

	// Create state and run transcription
	let mut state = ctx.create_state().context("Failed to create Whisper state")?;

	state.full(params, audio).context("Transcription failed")?;

	// Extract text segments
	let num_segments = state.full_n_segments();

	if num_segments == 0 {
		return Ok("[No speech detected]".to_string());
	}

	let mut full_text = String::new();

	for i in 0..num_segments {
		if let Some(segment) = state.get_segment(i) {
			if let Ok(text) = segment.to_str() {
				let trimmed = text.trim();
				if !trimmed.is_empty() {
					full_text.push_str(trimmed);
					full_text.push(' ');
				}
			}
		}
	}

	Ok(full_text.trim().to_string())
}

// Helper function to generate a test WAV file with speech-like signal
#[allow(dead_code)]
fn generate_test_wav(output_path: &str, duration_secs: f32) -> Result<()> {
	let sample_rate = 16000;
	let num_samples = (sample_rate as f32 * duration_secs) as usize;

	let spec = hound::WavSpec {
		channels: 1,
		sample_rate,
		bits_per_sample: 16,
		sample_format: hound::SampleFormat::Int,
	};

	let mut writer = hound::WavWriter::create(output_path, spec)?;

	// Generate a more speech-like signal (modulated sine waves)
	for i in 0..num_samples {
		let t = i as f32 / sample_rate as f32;

		// Mix of frequencies to simulate speech
		let f1 = 200.0 + 100.0 * (t * 2.0).sin(); // Varying fundamental
		let f2 = 800.0 + 200.0 * (t * 3.0).cos(); // First formant
		let f3 = 2500.0; // Second formant

		let sample = 0.3 * (2.0 * std::f32::consts::PI * f1 * t).sin() + 0.2 * (2.0 * std::f32::consts::PI * f2 * t).sin() + 0.1 * (2.0 * std::f32::consts::PI * f3 * t).sin();

		// Add amplitude envelope (syllable-like)
		let envelope = (t * 4.0).sin().abs();
		let amplitude = (sample * envelope * 32767.0) as i16;

		writer.write_sample(amplitude)?;
	}

	writer.finalize()?;
	println!("Generated test WAV: {}", output_path);

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_generate_wav() {
		generate_test_wav("test_output.wav", 3.0).unwrap();
	}
}
