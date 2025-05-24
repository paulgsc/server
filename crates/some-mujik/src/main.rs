mod music_sheet;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::f32::consts::PI;
use std::path::PathBuf;
use std::time::Duration;

/// Simple sound generator for React overlays
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
	/// Generate a single note
	Note {
		/// The note to generate (e.g., "A4", "C#5")
		#[arg(short, long)]
		note: String,

		/// Duration in milliseconds
		#[arg(short, long, default_value = "500")]
		duration: u64,

		/// Output WAV file
		#[arg(short, long)]
		output: PathBuf,
	},

	/// Process a music sheet file
	Sheet {
		/// Path to the music sheet file
		#[arg(short, long)]
		input: PathBuf,

		/// Output directory for generated WAV files
		#[arg(short, long)]
		output_dir: PathBuf,

		/// Base name for the output file
		#[arg(short, long, default_value = "output")]
		name: String,
	},
}

fn main() -> Result<()> {
	let args = Args::parse();

	match args.command {
		Command::Note { note, duration, output } => {
			// Generate a single note
			let frequency = note_to_frequency(&note)?;
			let duration = Duration::from_millis(duration);

			println!("Generating {} Hz tone for {} ms", frequency, duration.as_millis());
			generate_wav_file(frequency, duration, &output)?;
			println!("Sound saved to: {}", output.display());
		}

		Command::Sheet { input, output_dir, name } => {
			// Process a music sheet file
			println!("Processing music sheet: {}", input.display());
			let output_path = music_sheet::process_music_sheet(&input, &output_dir, &name)?;
			println!("Sound effect created: {}", output_path.display());
		}
	}

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

/// Generate a WAV file with a sine wave at the specified frequency
fn generate_wav_file(frequency: f32, duration: Duration, path: &PathBuf) -> Result<()> {
	// Audio settings
	let sample_rate = 44100;
	let spec = WavSpec {
		channels: 1,
		sample_rate,
		bits_per_sample: 16,
		sample_format: SampleFormat::Int,
	};

	let mut writer = WavWriter::create(path, spec)?;

	// Generate sine wave
	let num_samples = (duration.as_secs_f32() * sample_rate as f32) as usize;

	for t in 0..num_samples {
		let sample = (t as f32 * frequency * 2.0 * PI / sample_rate as f32).sin();
		// Convert to i16 for WAV format
		let amplitude = 0.5; // Adjust volume
		let sample_i16 = (sample * amplitude * i16::MAX as f32) as i16;
		writer.write_sample(sample_i16)?;
	}

	writer.finalize()?;

	Ok(())
}
