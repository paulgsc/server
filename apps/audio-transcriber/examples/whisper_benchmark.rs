// Whisper performance benchmarking tool
//
// Build with:
//   RUSTFLAGS="-C target-cpu=native" cargo build --release --example whisper_benchmark
//
// Run with:
//   ./target/release/examples/whisper_benchmark <model_path> <audio_file>

use anyhow::{Context, Result};
use hound::WavReader;
use std::env;
use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn main() -> Result<()> {
	let args: Vec<String> = env::args().collect();

	if args.len() < 3 {
		print_usage(&args[0]);
		std::process::exit(1);
	}

	let model_path = &args[1];
	let wav_path = &args[2];
	let iterations = args.get(3).and_then(|s| s.parse::<usize>().ok()).unwrap_or(3);

	println!("╔═══════════════════════════════════════════════════════════╗");
	println!("║          Whisper CPU Performance Benchmark            ║");
	println!("╚═══════════════════════════════════════════════════════════╝\n");

	// Check if we're in debug or release mode
	#[cfg(debug_assertions)]
	{
		println!("⚠️  WARNING: Running in DEBUG mode!");
		println!("   Performance will be 10-20x slower than release.");
		println!("   Build with: cargo build --release\n");
	}

	#[cfg(not(debug_assertions))]
	{
		println!("✅ Running in RELEASE mode\n");
	}

	// System info
	println!("📊 System Information:");
	println!("   CPU cores: {}", num_cpus::get());
	println!("   Model: {}", model_path);
	println!("   Audio: {}\n", wav_path);

	// Load audio
	println!("📂 Loading audio file...");
	let audio_data = load_wav(wav_path)?;
	let audio_duration = audio_data.len() as f64 / 16000.0;
	println!("   Duration: {:.2}s", audio_duration);
	println!("   Samples: {}\n", audio_data.len());

	// Load model with timing
	println!("🔄 Loading Whisper model...");
	let model_load_start = Instant::now();
	let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default()).context("Failed to load model")?;
	let model_load_time = model_load_start.elapsed();
	println!("   Load time: {:.2}s\n", model_load_time.as_secs_f64());

	// Run benchmark
	println!("🎤 Running transcription benchmark ({} iterations)...\n", iterations);

	let mut times = Vec::new();
	let mut transcriptions = Vec::new();

	for i in 1..=iterations {
		println!("   Iteration {}/{}...", i, iterations);

		let start = Instant::now();
		let transcription = transcribe(&ctx, &audio_data, num_cpus::get() as i32)?;
		let duration = start.elapsed();

		times.push(duration.as_secs_f64());
		transcriptions.push(transcription);

		let rtf = duration.as_secs_f64() / audio_duration;
		println!("      Time: {:.2}s", duration.as_secs_f64());
		println!("      RTF:  {:.3}x", rtf);

		if rtf < 1.0 {
			println!("      ✅ Faster than realtime!");
		} else {
			println!("      ⚠️  Slower than realtime");
		}
		println!();
	}

	// Calculate statistics
	let avg_time = times.iter().sum::<f64>() / times.len() as f64;
	let min_time = times.iter().fold(f64::INFINITY, |a, &b| a.min(b));
	let max_time = times.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));

	let avg_rtf = avg_time / audio_duration;
	let min_rtf = min_time / audio_duration;
	let max_rtf = max_time / audio_duration;

	// Results
	println!("╔═══════════════════════════════════════════════════════════╗");
	println!("║                    Benchmark Results                      ║");
	println!("╚═══════════════════════════════════════════════════════════╝\n");

	println!("⏱️  Timing Statistics:");
	println!("   Average: {:.2}s (RTF: {:.3}x)", avg_time, avg_rtf);
	println!("   Minimum: {:.2}s (RTF: {:.3}x)", min_time, min_rtf);
	println!("   Maximum: {:.2}s (RTF: {:.3}x)", max_time, max_rtf);
	println!();

	println!("📈 Performance Rating:");
	if avg_rtf < 0.3 {
		println!("   🚀 EXCELLENT - 3x+ faster than realtime");
		println!("   Can handle 3+ concurrent streams");
	} else if avg_rtf < 0.7 {
		println!("   ✅ GOOD - Faster than realtime");
		println!("   Can handle 1-2 concurrent streams");
	} else if avg_rtf < 1.0 {
		println!("   ⚡ ACCEPTABLE - Near realtime");
		println!("   Can handle 1 stream with buffering");
	} else if avg_rtf < 2.0 {
		println!("   ⚠️  SLOW - 2x slower than realtime");
		println!("   Consider faster model or GPU");
	} else {
		println!("   ❌ TOO SLOW - {}x slower than realtime", avg_rtf.round() as i32);
		println!("   Strongly recommend GPU or tiny model");
	}
	println!();

	println!("💬 Transcription:");
	println!("   \"{}\"", transcriptions[0]);
	println!();

	// Recommendations
	print_recommendations(avg_rtf);

	Ok(())
}

fn transcribe(ctx: &WhisperContext, audio: &[f32], threads: i32) -> Result<String> {
	let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

	params.set_n_threads(threads);
	params.set_print_special(false);
	params.set_print_progress(false);
	params.set_print_realtime(false);
	params.set_print_timestamps(false);
	params.set_language(Some("en"));
	params.set_translate(false);

	let mut state = ctx.create_state()?;
	state.full(params, audio)?;

	let num_segments = state.full_n_segments();
	if num_segments == 0 {
		return Ok(String::new());
	}

	let mut text = String::new();
	for i in 0..num_segments {
		if let Some(segment) = state.get_segment(i) {
			if let Ok(seg_text) = segment.to_str() {
				text.push_str(seg_text.trim());
				text.push(' ');
			}
		}
	}

	Ok(text.trim().to_string())
}

fn load_wav(path: &str) -> Result<Vec<f32>> {
	let mut reader = WavReader::open(path)?;
	let spec = reader.spec();

	let samples: Vec<f32> = match spec.bits_per_sample {
		16 => reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect(),
		32 => reader.samples::<i32>().map(|s| s.unwrap() as f32 / 2147483648.0).collect(),
		_ => anyhow::bail!("Unsupported bit depth"),
	};

	let mono = if spec.channels == 2 {
		samples.chunks_exact(2).map(|c| (c[0] + c[1]) / 2.0).collect()
	} else {
		samples
	};

	if spec.sample_rate != 16000 {
		eprintln!("⚠️  Sample rate is {}, not 16000 Hz - results may be poor", spec.sample_rate);
	}

	Ok(mono)
}

fn print_recommendations(rtf: f64) {
	println!("💡 Recommendations:");

	if rtf > 1.0 {
		println!("\n   To improve performance:");
		println!("   1. ✅ Switch to a smaller model:");
		println!("      • tiny.en: 3-10x faster");
		println!("      • base.en: 2-3x faster than small/medium");
		println!();
		println!("   2. 🔧 Optimize build flags:");
		println!("      RUSTFLAGS=\"-C target-cpu=native\" cargo build --release");
		println!();
		println!("   3. ⚙️  Check Cargo.toml:");
		println!("      [profile.release]");
		println!("      opt-level = 3");
		println!("      lto = \"fat\"");
		println!();
		println!("   4. 🖥️  Consider GPU deployment:");
		println!("      • 10-50x faster than CPU");
		println!("      • AWS g4dn, GCP T4 instances");
	} else if rtf > 0.7 {
		println!("\n   Performance is good, but could be better:");
		println!("   • Ensure you're using all CPU cores");
		println!("   • Try tiny.en for even faster processing");
		println!("   • Current setup can handle ~1 concurrent stream");
	} else {
		println!("\n   ✅ Performance is excellent for CPU!");
		println!("   • Can handle {} concurrent streams", (1.0 / rtf).floor() as i32);
		println!("   • Consider this setup for production");
	}
}

fn print_usage(program: &str) {
	eprintln!("Usage: {} <model_path> <wav_path> [iterations]", program);
	eprintln!();
	eprintln!("Examples:");
	eprintln!("  {} ggml-tiny.en.bin audio.wav", program);
	eprintln!("  {} ggml-base.en.bin audio.wav 5", program);
	eprintln!();
	eprintln!("Models (download from https://huggingface.co/ggerganov/whisper.cpp):");
	eprintln!("  • ggml-tiny.en.bin   (75 MB)  - Fastest, good quality");
	eprintln!("  • ggml-base.en.bin   (142 MB) - Balanced speed/quality");
	eprintln!("  • ggml-small.en.bin  (466 MB) - Better quality, slower");
	eprintln!("  • *-q5_1.bin variants         - Quantized (smaller, may be slower on CPU)");
}
