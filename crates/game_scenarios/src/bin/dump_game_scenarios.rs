//! Emits the scenario set as JSON on stdout: the copy paulgsc/some-ui keeps
//! as `game-scenarios.snapshot.json`.
//!
//! ```sh
//! cargo run -q --bin dump-game-scenarios > game-scenarios.snapshot.json
//! ```

use std::io::{self, Write};

fn main() -> io::Result<()> {
	let stdout = io::stdout();
	let mut out = stdout.lock();
	// `to_writer_pretty` rather than `to_string`, which `clippy.toml` disallows.
	serde_json::to_writer_pretty(&mut out, &game_scenarios::catalog()).map_err(io::Error::other)?;
	out.write_all(b"\n")?;
	out.flush()
}
