//! Is someone looking at the dashboard right now?
//!
//! ## The decision, recorded
//!
//! **Presence is an input, not a veto.** The policy weighs it exactly where the
//! client weighed `document.visibilityState`, and the reasoning for keeping it
//! weighable rather than absolute is this:
//!
//! What the server can observe is not what the client observes. The client
//! knows whether its tab is *visible*. The server knows only whether a
//! WebSocket is *connected* — and a pinned background tab holds its socket open
//! all day. Treating connection as presence would suppress every nudge for
//! exactly the person this feature was built for: someone who keeps the
//! dashboard open on a second monitor and forgets about it. The failure mode is
//! total silence that looks like a broken feature, which is the worst kind of
//! bug to have in a reminder.
//!
//! So the veto is bounded twice over:
//!
//! 1. A connection counts only if the actor reports it active, not stale, and
//!    with activity inside a freshness window (`NUDGE_PRESENCE_FRESHNESS_SECONDS`).
//!    A zombie half-open socket therefore cannot silence the nudge indefinitely.
//! 2. Every decision logs which way presence went, so a missing nudge is
//!    diagnosable rather than mysterious.
//!
//! The stronger signal — the client reporting genuine visibility, which it
//! already knows — is the escape hatch if the bound turns out not to be enough.
//! That is a `some-ui` change and is deliberately not built speculatively.

use crate::websocket::WebSocketFsm;
use std::time::Duration;

/// What the tick learned about presence, kept as a struct rather than a bool so
/// the log line can say *why* it concluded what it concluded.
#[derive(Debug, Clone, Copy)]
pub struct Presence {
	/// Connections the store still holds handles for, fresh or not.
	pub connected: usize,
	/// Of those, the ones active, unstale, and recently heard from.
	pub live: usize,
}

impl Presence {
	/// The signal the policy consumes.
	#[must_use]
	pub const fn is_present(self) -> bool {
		self.live > 0
	}

	/// True when sockets are held but none of them count — the zombie case,
	/// worth its own log line because it is the one that used to cause silence.
	#[must_use]
	pub const fn has_only_stale_connections(self) -> bool {
		self.connected > 0 && self.live == 0
	}
}

/// Ask the WebSocket layer about presence.
///
/// Goes through [`WebSocketFsm::live_connection_count`] rather than reaching
/// into the connection store, so the sender does not depend on WS internals and
/// the freshness rule has one home.
pub async fn observe(ws: &WebSocketFsm, freshness: Duration) -> Presence {
	let (connected, live) = ws.live_connection_count(freshness).await;
	Presence { connected, live }
}
