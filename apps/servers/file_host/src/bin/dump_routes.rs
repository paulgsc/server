//! Emits the server's HTTP surface as JSON on stdout.
//!
//! ```sh
//! cargo run -q --bin dump-routes > routes.server.json
//! ```
//!
//! The client repo checks that file in (`packages/contract-harness/`) and
//! diffs its contracts against it, so a route rename on this side shows up
//! there as a failing drift check rather than a 404 in the browser.
//!
//! Deliberately does not touch [`file_host::Config`]: the real server requires
//! `HMAC_KEY` and a database before it will start, and needing a provisioned
//! environment just to ask "what paths do you serve?" would make this too
//! annoying to run, which would make the snapshot stale, which would make the
//! whole check worthless.

use file_host::routes::inventory;
use std::io::{self, Write};

fn main() -> io::Result<()> {
	let stdout = io::stdout();
	let mut out = stdout.lock();

	// `to_writer_pretty` rather than `to_string`: `clippy.toml` disallows the
	// latter, and streaming avoids materialising the document twice.
	serde_json::to_writer_pretty(&mut out, &inventory::snapshot()).map_err(io::Error::other)?;
	out.write_all(b"\n")?;
	out.flush()
}
