//! Emits the server's HTTP surface, in one of two shapes, on stdout.
//!
//! ```sh
//! cargo run -q --bin dump-routes > routes.server.json
//! cargo run -q --bin dump-routes -- --ts > routes.server.ts
//! ```
//!
//! The client repo checks both files in (`packages/contract-harness/`): the
//! JSON one is diffed against a live drift check, and the TypeScript one is
//! imported directly so a typo in a route path is a `tsc` error instead of a
//! 404 found in a browser three days later (#266). See
//! `file_host::routes::ts_emitter` for why one flag rather than a second
//! `[[bin]]`, and why the two outputs can never disagree with each other.
//!
//! Deliberately does not touch [`file_host::Config`]: the real server requires
//! `HMAC_KEY` and a database before it will start, and needing a provisioned
//! environment just to ask "what paths do you serve?" would make this too
//! annoying to run, which would make the snapshot stale, which would make the
//! whole check worthless.

use clap::Parser;
use file_host::routes::{inventory, ts_emitter};
use std::io::{self, Write};

#[derive(Parser, Debug)]
#[command(author, version, about = "Emits file_host's HTTP surface as JSON (default) or TypeScript (--ts)")]
struct Args {
	/// Emit the `ServerRoute` / `UnversionedRoute` TypeScript module instead
	/// of the default JSON snapshot.
	#[arg(long)]
	ts: bool,
}

fn main() -> io::Result<()> {
	let args = Args::parse();
	let stdout = io::stdout();
	let mut out = stdout.lock();

	if args.ts {
		out.write_all(ts_emitter::render_ts(&inventory::snapshot()).as_bytes())?;
	} else {
		// `to_writer_pretty` rather than `to_string`: `clippy.toml` disallows the
		// latter, and streaming avoids materialising the document twice.
		serde_json::to_writer_pretty(&mut out, &inventory::snapshot()).map_err(io::Error::other)?;
		out.write_all(b"\n")?;
	}
	out.flush()
}
