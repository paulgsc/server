//! `/api/v1/games` — a game's record, to a paired phone (Game Week Bites,
//! paulgsc/some-ui#2074)
//!
//! GET /games/:id/moments  → { gameId, engineVersion, catalogVersion, final,
//!                              moments: [{ id, playId, line, at, bigPlay?,
//!                              unit, unitSide, period, clock }] }: every
//!                              play home kept of the game, in its order, as
//!                              the scenarios it was; each entry the shape
//!                              `/ws/game-week`'s `justHappened` sends, with
//!                              the quarter and clock of its snap. `404` for
//!                              a game home has no log of.
//!
//! **Trust model:** a [`crate::subject::Paired`] phone, as `/ws/game-week`;
//! it serves only ESPN's public game data read through the engine. See
//! `handlers::games`.

use crate::auth::AuthContext;
use crate::handlers::games as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn games<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	Module::versioned("games", RouteTable::new().get("/games/:id/moments", handlers::get_moments)).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::OPTIONS], vec![ACCEPT, AUTHORIZATION])
}
