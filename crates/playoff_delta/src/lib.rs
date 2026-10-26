//! Evaluates an NFL regular season into the Playoff Delta story, week by
//! week: how bad can a team be, and still make the playoffs?
//!
//! Pure (PD2): a checked [`season::Season`] in, the client contract's
//! [`story::StorySeason`] out, from [`evaluate::evaluate`]. The one hard
//! question is [`solver::best`] (is there a way in?); [`count::tally`] asks
//! how many of the ways the season can go get in. Ties go to the team asked
//! about in both (PD1). The README has the field definitions and the
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
