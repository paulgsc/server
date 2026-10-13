//! Evaluates an NFL regular season into the Playoff Delta story, week by
//! week: how bad can a team be, and still make the playoffs?
//!
//! Pure (PD2): a checked [`season::Season`] in, the client contract's
//! [`story::StorySeason`] out, from [`evaluate::evaluate`]. The one hard
//! question is [`solver::best`]; ties go to the team asked about (PD1). The
//! README has the field definitions and the invariants.

pub mod evaluate;
pub mod flow;
pub mod league;
pub mod season;
pub mod solver;
pub mod story;
pub mod table;
