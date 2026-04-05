//! `ts done <outcome>` — record the outcome of the last session.
//!
//! Intended for the common pattern:
//!
//!   $ ts step          # open the resource, start the timer
//!   ... 30 minutes ...
//!   $ ts done stuck    # record how it went
//!
//! Outcome is stored as metadata only. It does not affect scheduling.
//! Calling `ts done` with no prior `ts step` is a no-op with a warning.

use anyhow::Result;
use tabsched::Outcome;

use crate::{cmd::step::OutcomeArg, ctx::Ctx, ui};

#[derive(clap::Args)]
pub struct Args {
	/// The session outcome.
	#[arg(value_enum)]
	pub outcome: OutcomeArg,
}

pub fn run(ctx: &mut Ctx, args: &Args) -> Result<()> {
	let outcome: Outcome = args.outcome.clone().into();

	if ctx.engine.state().history.is_empty() {
		ui::print_warn("no sessions recorded yet — nothing to annotate");
		return Ok(());
	}

	ctx.engine.record_outcome(outcome);
	ctx.save()?;
	ui::print_ok("outcome recorded");
	Ok(())
}
