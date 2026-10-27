//! ENV1 end to end (README.md, "Configuration"): a variable that does not
//! parse, set in the environment, stops the binary before it does anything
//! else, with an error naming the variable.

use std::process::Command;

#[test]
fn a_variable_that_does_not_parse_stops_the_binary_at_start() {
	let output = Command::new(env!("CARGO_BIN_EXE_file_host"))
		// Away from the checkout, so `dotenvy` finds no `.env` to add.
		.current_dir(std::env::temp_dir())
		.env_clear()
		.env("OTEL_TRACES_SAMPLER_ARG", "half")
		.output()
		.unwrap();
	assert_eq!(output.status.code(), Some(2), "{output:?}");
	assert!(String::from_utf8_lossy(&output.stderr).contains("OTEL_TRACES_SAMPLER_ARG"), "{output:?}");
}
