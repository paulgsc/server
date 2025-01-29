use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

fn main() -> Result<(), anyhow::Error> {
	// Get the default host (audio system).
	let host = cpal::default_host();

	// Get the default input device (microphone).
	let input_device = host.default_input_device().expect("No input device found");

	println!("Using input device: {}", input_device.name()?);

	// Configure the audio stream format.
	let config = input_device.default_input_config().expect("Failed to get default input config");

	println!("Input format: {:?}", config);

	// Define the callback function to process captured audio data.
	let err_fn = |err| eprintln!("An error occurred on the input audio stream: {}", err);
	let _ = config.sample_rate().0;

	let stream = input_device.build_input_stream(
		&config.into(),
		move |data: &[f32], _: &cpal::InputCallbackInfo| {
			// Process the audio data here.
			println!("Captured {} samples", data.len());
			// Example: Print the first few samples.
			for &sample in data.iter().take(10) {
				println!("Sample: {}", sample);
			}
		},
		err_fn,
		None,
	)?;

	// Start the audio stream.
	stream.play()?;

	// Keep the program running to capture audio.
	std::thread::sleep(std::time::Duration::from_secs(10));

	Ok(())
}
