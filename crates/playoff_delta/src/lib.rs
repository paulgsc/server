//! Evaluates an NFL regular season into the Playoff Delta story, week by
//! week: did following the team pay, and is it still worth following?
//!
//! Pure (PD2): a checked [`season::Season`] in, the client contract's
//! [`story::StorySeason`] out, from [`evaluate::evaluate`]. [`count::tally`]
//! asks how many of the ways the season can go get the team in, and
//! [`count::slates`] how far a week can move that, every way it can go (read
//! by [`ways`]); [`solver::best`] asks
//! whether any way in is left, and [`solver::clinched`] whether every way is.
//! Ties go to the team asked about throughout (PD1). The README has the field definitions and the
//! invariants.

pub mod count;
pub mod evaluate;
pub mod flow;
pub mod league;
pub mod rank;
pub mod season;
pub mod solver;
pub mod story;
pub mod table;
#[cfg(test)]
mod testing;
pub mod ways;
