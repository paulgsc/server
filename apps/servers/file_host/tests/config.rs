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

/// #580: a requirement clap cannot see stops the binary at start too, with
/// every one that does not hold named at once (`file_host::RULES`).
#[test]
fn every_unmet_rule_stops_the_binary_at_start_by_name() {
	let output = Command::new(env!("CARGO_BIN_EXE_file_host"))
		.current_dir(std::env::temp_dir())
		.env_clear()
		.env("OBS_WEBSOCKET_IP", "127.0.0.1")
		.env("OBS_WEBSOCKET_PWD", "")
		.env("DATABASE_URL", "sqlite::memory:")
		.env("AUTH_ENABLED", "false")
		.env("MAILBOX_ENABLED", "true")
		.env("MAILBOX_SECONDS", "30")
		.output()
		.unwrap();
	assert_eq!(output.status.code(), Some(2), "{output:?}");
	let stderr = String::from_utf8_lossy(&output.stderr);
	for gap in [
		"MAILBOX_URL is required while MAILBOX_ENABLED is true",
		"MAILBOX_TOKEN is required",
		"MAILBOX_SECONDS must be at least 60",
	] {
		assert!(stderr.contains(gap), "{gap}: {output:?}");
	}
}

/// Codex on #600: `--health-check` is a separate process the container's
/// healthcheck runs, inheriting none of the server's flags, so the start-up
/// rules are not its to enforce. Here auth is on by default and unconfigured,
/// which a server would refuse; the health check only asks, and fails as a
/// health check does with nothing listening.
#[test]
fn a_health_check_is_not_held_to_the_start_up_rules() {
	let output = Command::new(env!("CARGO_BIN_EXE_file_host"))
		.current_dir(std::env::temp_dir())
		.env_clear()
		.env("OBS_WEBSOCKET_IP", "127.0.0.1")
		.env("OBS_WEBSOCKET_PWD", "")
		.env("DATABASE_URL", "sqlite::memory:")
		.args(["--health-check", "--health-check-port", "9"])
		.output()
		.unwrap();
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(!stderr.contains("is required while"), "{output:?}");
	assert_ne!(output.status.code(), Some(0), "nothing listens on 9: {output:?}");
}
