//! `ts status` — current deficit and session counts per track.
//!
//! Shows the scheduler's view: how many sessions each leaf track has
//! received in the current window, its target, and the deficit.
//! Positive deficit = under-served (will be prioritised).

use anyhow::Result;

use crate::{ctx::Ctx, ui};

pub fn run(ctx: &Ctx) -> Result<()> {
	let state = ctx.engine.state();
	let window = state.window_size();
	let history = &state.history;
	let total_slots = history.len();

	// Count recent sessions per leaf track.
	let window_start = total_slots.saturating_sub(window);
	let recent_window = &history[window_start..];

	let mut rows: Vec<ui::StatusRow> = ctx
		.topology
		.leaf_tracks()
		.map(|track| {
			let recent = recent_window.iter().filter(|s| s.track == track.id()).count() as u32;
			let total = history.iter().filter(|s| s.track == track.id()).count();
			let deficit = track.base_target() as i32 - recent as i32;

			ui::StatusRow {
				track: ctx.index.track_label(track.id()).to_owned(),
				target: track.base_target(),
				recent,
				deficit,
				total,
			}
		})
		.collect();

	// Sort by descending deficit so most-starved tracks appear first.
	rows.sort_by(|a, b| b.deficit.cmp(&a.deficit));

	println!("\n  window: {} slots   total: {} sessions", window, total_slots);
	ui::print_status_table(&rows);
	Ok(())
}
