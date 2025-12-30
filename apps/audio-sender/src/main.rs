use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use some_transport::{NatsTransport, Transport};
use std::io::{self, Write};
use tokio::sync::mpsc;
use ws_events::events::{Event, UnifiedEvent};

#[tokio::main]
async fn main() -> Result<()> {
	// Health check mode: just verify we can enumerate devices and exit
	if std::env::args().any(|arg| arg == "--health-check") {
		let host = cpal::default_host();
		let device_count = host.input_devices()?.count();
		if device_count > 0 {
			std::process::exit(0); // Success
		} else {
			std::process::exit(1); // Failure
		}
	}

	println!("🎤 Starting audio sender...");

	// Connect to NATS
	let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
	let transport = NatsTransport::<UnifiedEvent>::connect_pooled(&nats_url).await?;
	println!("✅ Connected to NATS at {}", nats_url);

	// Get audio host and devices
	let host = cpal::default_host();
	let devices: Vec<_> = host.input_devices().context("Failed to enumerate input devices")?.collect();

	if devices.is_empty() {
		eprintln!("❌ No input devices found!");
		return Ok(());
	}

	// Determine which device to use
	let device = select_audio_device(&devices)?;
	println!("✅ Selected: {}", device.name()?);

	let config = device.default_input_config().context("Failed to get default input config")?;
	println!("📊 Config: {:?}", config);

	let sample_rate = config.sample_rate().0;
	let channels = config.channels();

	// Channel to send audio from callback to async task
	let (tx, mut rx) = mpsc::channel::<Vec<f32>>(100);

	// Build input stream
	let stream = device
		.build_input_stream(
			&config.into(),
			move |data: &[f32], _: &cpal::InputCallbackInfo| {
				// Send audio chunk (non-blocking)
				let _ = tx.try_send(data.to_vec());
			},
			move |err| eprintln!("❌ Stream error: {}", err),
			None,
		)
		.context("Failed to build input stream")?;

	stream.play().context("Failed to start audio stream")?;
	println!("🎵 Audio stream started, publishing to 'audio.stream'...");
	println!("💡 Press Ctrl+C to stop\n");

	// Publish audio chunks to NATS
	let mut chunk_count = 0;
	while let Some(samples) = rx.recv().await {
		let event = Event::AudioChunk {
			sample_rate,
			channels: channels as u32,
			samples,
		};

		let unified: UnifiedEvent = event.try_into().expect("AudioChunk should always convert to UnifiedEvent");

		let subject = unified.subject().unwrap_or("audio.chunk".to_owned());
		transport.send_to_subject(&subject, unified).await?;

		chunk_count += 1;
		if chunk_count % 100 == 0 {
			print!(".");
			io::stdout().flush()?;
		}
	}

	Ok(())
}

/// Select audio device based on environment variables or user input
fn select_audio_device(devices: &[cpal::Device]) -> Result<&cpal::Device> {
	// Check for AUDIO_DEVICE_INDEX env var (0-based index)
	if let Ok(index_str) = std::env::var("AUDIO_DEVICE_INDEX") {
		let index: usize = index_str.parse().context("AUDIO_DEVICE_INDEX must be a valid number")?;
		return devices.get(index).ok_or_else(|| anyhow::anyhow!("Device index {} out of range", index));
	}

	// Check for AUDIO_DEVICE_NAME env var (matches by name)
	if let Ok(device_name) = std::env::var("AUDIO_DEVICE_NAME") {
		for device in devices {
			if let Ok(name) = device.name() {
				if name.to_lowercase().contains(&device_name.to_lowercase()) {
					println!("🎯 Found device matching '{}': {}", device_name, name);
					return Ok(device);
				}
			}
		}
		eprintln!("⚠️  Device matching '{}' not found, falling back...", device_name);
	}

	// Check if running in non-interactive environment (Docker, CI, etc.)
	if !atty::is(atty::Stream::Stdin) {
		println!("🤖 Non-interactive mode detected, using default device");
		return devices.first().ok_or_else(|| anyhow::anyhow!("No devices available"));
	}

	// Interactive mode: list devices and prompt user
	println!("\n📋 Available input devices:");
	println!("───────────────────────────────────────");
	for (idx, device) in devices.iter().enumerate() {
		let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
		println!("  [{}] {}", idx, name);
	}
	println!("───────────────────────────────────────");

	print!("\n🎯 Select device number (or press Enter for default): ");
	io::stdout().flush()?;

	let mut input = String::new();
	io::stdin().read_line(&mut input)?;
	let input = input.trim();

	if input.is_empty() {
		println!("Using default device");
		return devices.first().ok_or_else(|| anyhow::anyhow!("No devices available"));
	}

	let device_idx: usize = input.parse().context("Please enter a valid number or press Enter for default")?;

	devices.get(device_idx).ok_or_else(|| anyhow::anyhow!("Invalid device index: {}", device_idx))
}
