use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Represents a simple musical note
#[derive(Debug, Clone)]
pub struct Note {
	pub name: String,
	pub duration_ms: u64,
}

/// Parse a simple music sheet format
/// Each line has format: NOTE DURATION_MS
/// Example:
/// A4 250
/// C5 500
/// REST 100
/// G4 250
pub fn parse_music_sheet(sheet_path: &Path) -> Result<Vec<Note>> {
	let content = fs::read_to_string(sheet_path).context(format!("Failed to read sheet file: {}", sheet_path.display()))?;

	let mut notes = Vec::new();

	for (line_num, line) in content.lines().enumerate() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue; // Skip empty lines and comments
		}

		let parts: Vec<&str> = line.split_whitespace().collect();
		if parts.len() != 2 {
			return Err(anyhow::anyhow!("Invalid sheet format at line {}: expected 'NOTE DURATION'", line_num + 1));
		}

		let note_name = parts[0].to_string();
		let duration: u64 = parts[1].parse().context(format!("Invalid duration at line {}", line_num + 1))?;

		notes.push(Note {
			name: note_name,
			duration_ms: duration,
		});
	}

	Ok(notes)
}

/// Process a music sheet and generate WAV files
pub fn process_music_sheet(sheet_path: &Path, output_dir: &Path, output_name: &str) -> Result<PathBuf> {
	let notes = parse_music_sheet(sheet_path)?;

	// Create output directory if it doesn't exist
	fs::create_dir_all(output_dir)?;

	// Generate individual note files and combine them
	let mut all_samples = Vec::new();

	for (_, note) in notes.iter().enumerate() {
		if note.name == "REST" {
			// For rests, generate silence
			let silence_samples = generate_silence(Duration::from_millis(note.duration_ms))?;
			all_samples.extend_from_slice(&silence_samples);
		} else {
			// For notes, generate the tone
			let frequency = note_to_frequency(&note.name)?;
			let samples = generate_sine_wave(frequency, Duration::from_millis(note.duration_ms))?;
			all_samples.extend_from_slice(&samples);
		}
	}

	// Write the combined samples to a WAV file
	let output_path = output_dir.join(format!("{}.wav", output_name));
	write_wav_file(&all_samples, &output_path)?;

	println!("Music sheet processed and saved to: {}", output_path.display());

	Ok(output_path)
}

fn generate_silence(duration: Duration) -> Result<Vec<i16>> {
	let sample_rate = 44100;
	let num_samples = (duration.as_secs_f32() * sample_rate as f32) as usize;
	Ok(vec![0; num_samples])
}

fn generate_sine_wave(frequency: f32, duration: Duration) -> Result<Vec<i16>> {
	use std::f32::consts::PI;

	let sample_rate = 44100;
	let num_samples = (duration.as_secs_f32() * sample_rate as f32) as usize;
	let mut samples = Vec::with_capacity(num_samples);

	let amplitude = 0.5; // Adjust volume

	for t in 0..num_samples {
		let sample = (t as f32 * frequency * 2.0 * PI / sample_rate as f32).sin();
		let sample_i16 = (sample * amplitude * i16::MAX as f32) as i16;
		samples.push(sample_i16);
	}

	Ok(samples)
}

fn write_wav_file(samples: &[i16], path: &Path) -> Result<()> {
	use hound::{SampleFormat, WavSpec, WavWriter};

	let spec = WavSpec {
		channels: 1,
		sample_rate: 44100,
		bits_per_sample: 16,
		sample_format: SampleFormat::Int,
	};

	let mut writer = WavWriter::create(path, spec)?;

	for &sample in samples {
		writer.write_sample(sample)?;
	}

	writer.finalize()?;

	Ok(())
}

/// Convert a musical note (e.g., "A4", "C#5") to its frequency in Hz
fn note_to_frequency(note: &str) -> Result<f32> {
	let note = note.to_uppercase();

	// Basic parsing of note format
	let (note_name, octave) = note.split_at(note.len() - 1);
	let octave: i32 = octave.parse().context("Invalid octave")?;

	// A4 = 440 Hz
	let base_frequency = 440.0;

	// Calculate semitones from A4
	let semitones = match note_name {
		"C" => -9,
		"C#" | "DB" => -8,
		"D" => -7,
		"D#" | "EB" => -6,
		"E" => -5,
		"F" => -4,
		"F#" | "GB" => -3,
		"G" => -2,
		"G#" | "AB" => -1,
		"A" => 0,
		"A#" | "BB" => 1,
		"B" => 2,
		_ => return Err(anyhow::anyhow!("Unsupported note: {}", note_name)),
	} + (octave - 4) * 12;

	// Calculate frequency: f = base_frequency * 2^(semitones/12)
	let frequency = base_frequency * 2.0_f32.powf(semitones as f32 / 12.0);

	Ok(frequency)
}
