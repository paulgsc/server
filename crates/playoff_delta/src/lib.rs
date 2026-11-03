//! Evaluates an NFL regular season into the Playoff Delta story, week by
//! week: how few and how many wins our rivals can still finish with, and
//! which way each week's results moved us.
//!
//! Pure (PD2): a checked [`season::Season`] in, the client contract's
//! [`story::StorySeason`] out, from [`evaluate::evaluate`]. [`poles`] reads
//! the extremes off the games left: a group's floor and the bloc that forces
//! it, its ceiling, and the worst seed; [`solver::best`] the best seed. Ties
//! go to the team asked about throughout (PD1). The README has the field
//! definitions and the invariants.

pub mod evaluate;
pub mod flow;
pub mod league;
pub mod poles;
pub mod rank;
pub mod season;
pub mod solver;
pub mod story;
pub mod table;
#[cfg(test)]
mod testing;
