//! Gauges that must not be sampled from the request or scrape path — see
//! #227 (C2) and #228 (C3).
//!
//! `WebSocketFsm::connection_gauges` awaits every connection actor to answer
//! `live`/`subscribed`, which is fine on a periodic sweep and not fine on
//! every `/metrics` scrape at scale (#227's own task list says so). The
//! SQLite pool gauges are cheap synchronous reads, but sampling them here
//! too keeps this one place responsible for every gauge that isn't updated
//! from its own mutation site, rather than growing a second spawned loop
//! for a handful of `gauge!` calls that cost nothing extra to batch in.

use crate::websocket::connection::instrument;
use crate::AppState;
use std::time::Duration;
use tracing::info;

/// Spawn the periodic gauge refresh, cancelled through the shared token.
/// Unconditional — unlike the nudge waker, there is no configuration under
/// which these gauges shouldn't run.
pub fn spawn(state: AppState, freshness: Duration, interval: Duration) {
	let cancel = state.core.cancel_token.clone();

	tokio::spawn(async move {
		info!(interval_secs = interval.as_secs(), "operational gauge refresh started");
		let mut tick = tokio::time::interval(interval);

		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("operational gauge refresh cancelled");
					return;
				}
				_ = tick.tick() => {
					let snapshot = state.realtime.ws.connection_gauges(freshness).await;
					instrument::set_presence_gauges(snapshot.connected, snapshot.live, snapshot.subscribed);
					instrument::set_guard_occupancy(state.core.connection_guard.active_global());
					crate::metrics::pool::record(&state.core.shared_db);
				}
			}
		}
	});
}
