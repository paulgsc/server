//! `ts step [--outcome <outcome>]` — execute and log one session.
//!
//! This is the primary command in the loop:
//!
//!   free time appears → `ts step` → open the resource → work → `ts done <outcome>`
//!
//! Optionally, if you already know the outcome (e.g. calling from a
//! script), pass `--outcome` to record it in one shot.

use anyhow::Result;

use crate::{ctx::Ctx, ui};

#[derive(clap::Args)]
pub struct Args {
	/// Record the session outcome immediately.
	#[arg(long, value_enum)]
	pub outcome: Option<OutcomeArg>,
}

/// clap-facing outcome enum. Kept separate from `tabsched::Outcome` so
/// the lib type doesn't need to derive `clap::ValueEnum`.
#[derive(clap::ValueEnum, Clone, Debug)]
pub enum OutcomeArg {
	Progress,
	Stuck,
	Review,
}

impl From<OutcomeArg> for tabsched::Outcome {
	fn from(o: OutcomeArg) -> Self {
		match o {
			OutcomeArg::Progress => tabsched::Outcome::Progress,
			OutcomeArg::Stuck => tabsched::Outcome::Stuck,
			OutcomeArg::Review => tabsched::Outcome::Review,
		}
	}
}

pub fn run(ctx: &mut Ctx, args: &Args) -> Result<()> {
	let session = ctx.engine.step();

	if let Some(ref o) = args.outcome {
		ctx.engine.record_outcome(o.clone().into());
	}

	ctx.save()?;
	ui::print_session_card(&session, &ctx.index);

	if args.outcome.is_none() {
		ui::print_step_hint();
	}

	Ok(())
}
