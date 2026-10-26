//! ENV1 end to end (README.md, "Configuration"): a variable that does not
//! parse, set in the environment, stops the binary before it does anything
//! else, with an error naming the variable.

use std::process::Command;

#[test]
fn a_variable_that_does_not_parse_stops_the_binary_at_start() {
	let output = Command::new(env!("CARGO_BIN_EXE_orchestrator")).env_clear().env("METRICS_ADDR", "nope").output().unwrap();
	assert_eq!(output.status.code(), Some(2), "{output:?}");
	assert!(String::from_utf8_lossy(&output.stderr).contains("METRICS_ADDR"), "{output:?}");
}
