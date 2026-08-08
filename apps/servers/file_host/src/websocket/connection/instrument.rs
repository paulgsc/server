//! WebSocket connection metrics, recorded through the `metrics` facade per
//! [#139][issue-139] (P1) — see #226 (C1) and #227 (C2).
//!
//! This file used to declare eight `prometheus` `lazy_static!` families and
//! eight recording macros, none of which had a call site. Because a
//! `lazy_static!` registers on first dereference, that meant the families
//! were never registered at all — not zero, absent — and one macro
//! (`connection_health_check!`) referenced a module path
//! (`$crate::connection::instrument`) that doesn't resolve, so it could not
//! have compiled at any call site either. #226 picked, per family, whether
//! to wire it at its real call site or delete it:
//!
//!   - `ws_connection_lifecycle_total{event}` — kept, `created`/`removed`
//!     only. `marked_stale`/`disconnected` are gone: nothing in the live
//!     code path ever drives `ConnectionState` through those transitions
//!     (`ConnectionCommand::MarkStale`/`Disconnect` have no caller — the
//!     only "monitor" that would have sent them, `ws-connection`'s
//!     `core/monitor.rs`, references a `crate::websocket` module tree that
//!     doesn't exist in this crate and isn't declared in `lib.rs`, so it
//!     has never compiled into anything). Keeping those event values would
//!     have meant a counter that reads real but is structurally unable to
//!     move.
//!   - `ws_client_connections{client_type}` — kept, paired with lifecycle.
//!   - `ws_connection_duration_seconds{end_reason}` — kept, recorded once
//!     per socket at the `ConnectionCleanup::drop` choke point
//!     (`websocket.rs`) rather than at store-level removal: `Drop` is the
//!     one place every connection's teardown provably passes through
//!     exactly once, including the server-shutdown path where the store
//!     entry is reclaimed directly by `ConnectionStore`'s own cancellation
//!     handling rather than through `WebSocketFsm::remove_connection`.
//!   - `ws_connection_messages_total{message_type}` — kept, relabelled onto
//!     the raw WebSocket frame kind (`text`/`ping`/`pong`/`close`/`binary`)
//!     recorded in `message/handlers.rs::handle_websocket_message`, the one
//!     site every inbound frame passes through regardless of payload.
//!   - `ws_connection_errors_total{error_type,phase}` — kept, wired at the
//!     real failure points in connection setup and message handling.
//!   - `ws_connection_states{state}` — deleted. `state="stale"` could never
//!     move for the same reason `marked_stale` above couldn't: nothing
//!     drives a connection into `ConnectionState::is_stale`. Superseded by
//!     `ws_connections{state}` below, which answers the question this
//!     dashboard row actually needs (connected/live/subscribed) instead of
//!     a state the code never reaches.
//!   - `ws_connection_subscriptions{event_type}` — deleted per #226's own
//!     note that it overlaps with #227's needs. A per-event-type breakdown
//!     has no reader: the CLIENTS row (#229) asks "does this connection
//!     want anything", not "which event types are popular".
//!   - `ws_timeout_monitor_operations_total{operation,result}` — deleted.
//!     Its `operation` label values (`mark_stale`/`cleanup`/`health_check`)
//!     describe `ws-connection::core::monitor::TimeoutMonitor`, which — see
//!     above — is not part of this workspace's compiled graph. There is no
//!     monitor loop to instrument.
//!
//! `ws_connections{state}` (connected/live/subscribed) and the
//! `ConnectionGuard` occupancy/capacity gauges are #227's new families —
//! see `websocket.rs::WebSocketFsm::connection_gauges` and
//! `metrics::periodic` for where they're computed and refreshed.
//!
//! [issue-139]: https://github.com/paulgsc/server/issues/139

use metrics::{counter, gauge, histogram};

/// A connection was added to the store. Paired with [`record_removed`].
pub fn record_created(client_type: &'static str) {
	counter!("ws_connection_lifecycle_total", "event" => "created").increment(1);
	gauge!("ws_client_connections", "client_type" => client_type).increment(1.0);
}

/// A connection's socket-level resources (tasks, permit) were torn down —
/// called exactly once per socket, from `ConnectionCleanup::drop`.
///
/// `end_reason` is one of `timeout` | `client_disconnect` | `error` |
/// `cleanup`, decided by the caller from why the teardown happened, not
/// inferred here from a free-text reason string the way the deleted macro
/// did it.
pub fn record_removed(client_type: &'static str, end_reason: &'static str, duration_secs: f64) {
	counter!("ws_connection_lifecycle_total", "event" => "removed").increment(1);
	gauge!("ws_client_connections", "client_type" => client_type).decrement(1.0);
	histogram!("ws_connection_duration_seconds", "end_reason" => end_reason).record(duration_secs);
}

/// A raw WebSocket frame was processed, labelled by its frame kind
/// (`text`/`ping`/`pong`/`close`/`binary`) rather than by parsed message
/// content — see this module's header comment for why.
pub fn record_message(message_type: &'static str) {
	counter!("ws_connection_messages_total", "message_type" => message_type).increment(1);
}

/// A connection-related failure, at one of the real failure points in
/// connection setup (`phase = "creation"`) or message handling
/// (`phase = "operation"`).
pub fn record_error(error_type: &'static str, phase: &'static str) {
	counter!("ws_connection_errors_total", "error_type" => error_type, "phase" => phase).increment(1);
}

/// `connected` / `live` / `subscribed` — see #227. `connected` is cheap
/// (a `DashMap::len()`) and is also set synchronously from
/// `add_connection`/`remove_connection`; `live` and `subscribed` require
/// awaiting every connection actor and are refreshed only by
/// `metrics::periodic`'s background sweep, never from the request or scrape
/// path.
pub fn set_presence_gauges(connected: usize, live: usize, subscribed: usize) {
	set_connected(connected);
	#[allow(clippy::cast_precision_loss)]
	gauge!("ws_connections", "state" => "live").set(live as f64);
	#[allow(clippy::cast_precision_loss)]
	gauge!("ws_connections", "state" => "subscribed").set(subscribed as f64);
}

/// Just `connected` — cheap enough to call from `add_connection` and
/// `remove_connection` directly, so it moves within one scrape interval
/// instead of waiting for the next periodic sweep.
pub fn set_connected(connected: usize) {
	#[allow(clippy::cast_precision_loss)]
	gauge!("ws_connections", "state" => "connected").set(connected as f64);
}

/// `ConnectionGuard`'s global semaphore occupancy — permits held right now.
/// Diverging from `connected` (held steady while `connected` drops toward
/// zero) is the signature of a leaked permit; see #227.
pub fn set_guard_occupancy(occupancy: usize) {
	#[allow(clippy::cast_precision_loss)]
	gauge!("ws_connection_guard_occupancy").set(occupancy as f64);
}

/// `ConnectionGuard`'s configured global capacity (`ws_conn_manager::MAX_GLOBAL`).
/// Set once at startup, the same "fixed value, dashboarded so it doesn't
/// have to be memorised" shape as `metrics::build_info::record`.
pub fn set_guard_capacity(capacity: usize) {
	#[allow(clippy::cast_precision_loss)]
	gauge!("ws_connection_guard_capacity").set(capacity as f64);
}
