//! `ts history [--last N]` — view recent session log.

use anyhow::Result;

use crate::{ctx::Ctx, ui};

#[derive(clap::Args)]
pub struct Args {
	/// Number of sessions to show (default: 20).
	#[arg(long, default_value = "20")]
	pub last: usize,
}

pub fn run(ctx: &Ctx, args: &Args) -> Result<()> {
	let history = &ctx.engine.state().history;
	let start = history.len().saturating_sub(args.last);
	let tail = &history[start..];

	let rows: Vec<ui::HistoryRow> = tail
		.iter()
		.map(|s| ui::HistoryRow {
			slot: s.slot_index.as_u64(),
			track: ctx.index.track_label(s.track).to_owned(),
			resource: ctx.index.resource_label(s.resource).to_owned(),
			outcome: ui::format_outcome(&s.outcome).to_owned(),
		})
		.collect();

	ui::print_history_table(&rows);
	Ok(())
}
