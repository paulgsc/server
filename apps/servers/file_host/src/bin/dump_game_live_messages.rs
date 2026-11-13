//! Emits one of every message the game day socket (`GET /ws/game-week`,
//! `websocket::game_live`, GWB4) sends and reads, and every close it ends
//! with, as JSON on stdout.
//!
//! ```sh
//! cargo run -q --bin dump-game-live-messages > game-live-messages.snapshot.json
//! ```
//!
//! paulgsc/some-ui keeps the output as `game-live-messages.snapshot.json`
//! and reads every `home` example with the phone's own decoder, and checks
//! every `phone` example is what its encoder writes. WebSocket messages are
//! not routes, so the route snapshot cannot carry them; this is their
//! contract. Regenerate the copy there in the same change as whatever moved
//! it here.

use std::io::{self, Write};

fn main() -> io::Result<()> {
	let stdout = io::stdout();
	let mut out = stdout.lock();
	// `to_writer_pretty` rather than `to_string`, which `clippy.toml`
	// disallows, as `dump_routes.rs` does.
	serde_json::to_writer_pretty(&mut out, &file_host::websocket::game_live::wire::examples()).map_err(io::Error::other)?;
	out.write_all(b"\n")?;
	out.flush()
}
