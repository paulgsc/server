//! How long the transcriber may hold the lane in a row (TR4, C899).
//!
//! While a note is heard the transcriber holds `Interactive`, and the lane
//! pauses a render for it (LA1). A backlog of notes, each heard by its
//! deadline, would hold the lane for hours, and every render with it: the
//! two cards essays waited on just that (2026-11-20). So a run of hearing
//! is a turn: once the transcriber has heard for [`TURN`] in a row while
//! batch work waits, it hears nothing for [`GIVE`], or until no batch work
//! waits, whichever comes first, and then takes a fresh turn. It never stops
//! a note midway: the turn is checked before each note, so a turn runs over
//! by at most one note's deadline.
//!
//! - **Only time while batch work waits counts.** The run is timed from
//!   the first note checked while batch work waits, and ends whenever none
//!   does; it includes the gaps between passes and a pass's own catch-up,
//!   which hold no lane but are part of the backlog's run.
//! - **A run ends when the queue does.** A pass that finds nothing to hear
//!   ends the run ([`Turn::rested`]), so notes heard as they arrive never add
//!   up to a turn.
//! - **Nothing is given to no one.** With no batch work waiting the
//!   transcriber hears on, however long its run (docs/home-compute.md
//!   invariant 2).

use crate::call_weeks::lane::Interactive;
use std::time::{Duration, Instant};

/// The longest run of hearing before the transcriber gives the lane a turn,
/// while batch work waits: two notes at their deadline, or many takes heard.
pub const TURN: Duration = Duration::from_secs(20 * 60);

/// How long a turn given lasts, at most: long enough for the lane's
/// hold-down and a render's next steps.
pub const GIVE: Duration = Duration::from_secs(10 * 60);

/// The transcriber's run of hearing, and whether it is giving the lane a
/// turn.
#[derive(Debug)]
pub struct Turn {
	length: Duration,
	give: Duration,
	/// What says whether batch work waits; none for a turn that never ends.
	batch: Option<Interactive>,
	/// When the run of hearing began.
	since: Option<Instant>,
	/// When the turn given ends.
	giving_until: Option<Instant>,
}

impl Turn {
	/// Home's turn: [`TURN`] of hearing, then [`GIVE`] while `batch` says
	/// batch work waits.
	#[must_use]
	pub const fn new(batch: Interactive) -> Self {
		Self::with(TURN, GIVE, Some(batch))
	}

	/// A run that never gives way: for passes with no lane, as in tests.
	#[must_use]
	pub const fn unbounded() -> Self {
		Self::with(Duration::MAX, Duration::ZERO, None)
	}

	#[must_use]
	pub const fn with(length: Duration, give: Duration, batch: Option<Interactive>) -> Self {
		Self {
			length,
			give,
			batch,
			since: None,
			giving_until: None,
		}
	}

	/// Whether the transcriber may hear its next note at `now`: not while it
	/// gives a turn, and not once its run has lasted the turn while batch
	/// work waits, when the turn given begins.
	pub fn may_hear(&mut self, now: Instant) -> bool {
		let waits = self.batch.as_ref().is_some_and(Interactive::batch_waits);
		if let Some(until) = self.giving_until {
			if waits && now < until {
				return false;
			}
			// Given, or nobody to give to any more: a fresh run.
			self.giving_until = None;
			self.since = None;
		}
		// The run counts only while batch work waits: hearing with nobody to
		// give to is no part of a turn.
		if !waits {
			self.since = None;
			return true;
		}
		let since = *self.since.get_or_insert(now);
		if now.saturating_duration_since(since) >= self.length {
			self.giving_until = now.checked_add(self.give);
			self.since = None;
			return false;
		}
		true
	}

	/// The queue was empty: the run of hearing is over.
	pub const fn rested(&mut self) {
		self.since = None;
	}

	/// Whether a turn is being given.
	#[must_use]
	pub const fn giving(&self) -> bool {
		self.giving_until.is_some()
	}
}

#[cfg(test)]
mod tests {
	use super::Turn;
	use crate::call_weeks::lane::Interactive;
	use std::time::{Duration, Instant};

	const MIN: Duration = Duration::from_secs(60);

	/// TR4 (C899): generated over every note length from one to fifteen
	/// minutes and every window in which batch work waits, on a clock of
	/// minutes. Whenever batch work waits, a note starts only before the run
	/// has lasted the turn (so no run lasts past the turn and one note); a
	/// turn given lasts its whole length unless no batch work waits; and the
	/// transcriber never gives way to nothing.
	#[test]
	fn a_run_of_hearing_gives_batch_work_its_turn() {
		let start = Instant::now();
		for note_min in 1..=15_u32 {
			for waits_from in [0_u32, 5, 30] {
				for waits_for in [10_u32, 60, 600] {
					let interactive = Interactive::default();
					let mut turn = Turn::with(20 * MIN, 10 * MIN, Some(interactive.clone()));
					let (mut run_from, mut given_from): (Option<u32>, Option<u32>) = (None, None);
					let (mut minute, mut gave) = (0_u32, 0);
					while minute < 720 {
						let waits = minute >= waits_from && minute < waits_from + waits_for;
						interactive.set_batch_waits(waits);
						let case = format_args!("{note_min} min notes, waiting from {waits_from} for {waits_for}, minute {minute}").to_string();
						if !waits {
							run_from = None;
						}
						if turn.may_hear(start + MIN * minute) {
							if let Some(from) = given_from.take() {
								assert!(!waits || minute >= from + 10, "a turn cut short while batch work waits: {case}");
							}
							if waits {
								let from = *run_from.get_or_insert(minute);
								assert!(minute - from < 20, "a note started past the turn: {case}");
							}
							minute += note_min;
						} else {
							assert!(waits, "gave way to nothing: {case}");
							run_from = None;
							let from = *given_from.get_or_insert_with(|| {
								gave += 1;
								minute
							});
							assert!(minute < from + 10, "a turn given that does not end: {case}");
							minute += 1;
						}
					}
					if waits_for >= 60 && note_min <= 10 {
						assert!(gave > 0, "{note_min} min notes, waiting from {waits_from} for {waits_for}: a long wait got no turn");
					}
				}
			}
		}
	}

	/// A run ends with the queue, so takes heard as they arrive never add up
	/// to a turn; and a turn that never ends never gives way.
	#[test]
	fn a_rested_run_starts_again_and_an_unbounded_one_never_gives_way() {
		let interactive = Interactive::default();
		interactive.set_batch_waits(true);
		let start = Instant::now();
		let mut turn = Turn::with(20 * MIN, 10 * MIN, Some(interactive));
		for hour in 0..10 {
			let at = start + MIN * (60 * hour);
			assert!(turn.may_hear(at), "a take heard as it arrives, hour {hour}");
			assert!(turn.may_hear(at + 5 * MIN));
			turn.rested();
		}
		assert!(!turn.giving());
		let mut unbounded = Turn::unbounded();
		assert!((0..100).all(|hour| unbounded.may_hear(start + MIN * (60 * hour))));
	}
}
