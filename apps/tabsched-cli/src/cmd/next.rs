//! `ts next` — show the next scheduled session without logging it.
//!
//! Use this when you want to see what's coming before committing. Does
//! not mutate state. Call `ts step` to actually execute and log.

use anyhow::Result;
use tabsched::next_session;

use crate::{ctx::Ctx, ui};

pub fn run(ctx: &Ctx) -> Result<()> {
	let session = next_session(ctx.engine.state(), ctx.topology);
	ui::print_session_card(&session, &ctx.index);
	Ok(())
}
