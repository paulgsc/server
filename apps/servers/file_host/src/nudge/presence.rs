//! Is someone looking at the *thing this notification is about*, right now?
//!
//! ## The decision, recorded
//!
//! **Presence is a lease, not truth, and it is scoped to a context, never to
//! "the site."** The previous design asked whether a WebSocket was connected,
//! which turned out wrong two independent ways: the client this feature is
//! for never opens one, so the signal almost never fired for a real user; and
//! this deployment's own synthetic WS health-check prober opens real
//! connections against the same endpoint every 15 seconds, which the old
//! count could not tell apart from a person. Both failures trace to the same
//! root cause — treating "a socket exists" as "someone is present" — so the
//! fix is not a better socket count, it is a different signal entirely.
//!
//! The right question was never "is this person on the site at all", it is
//! "are they looking at the specific thing this notification is about right
//! now". A lease answers that directly: the client writes `{ subject_id,
//! context_key, observed_at }` on meaningful transitions — the tab becoming
//! visible, the route changing — plus a sparse renewal while visible. This
//! module reads that lease at the one moment it matters: when a candidate
//! notification already exists and is about to be sent.
//!
//! ## The invariant this must never regrow past
//!
//! **Presence must never cause work; it can only modify work that was already
//! about to happen.** Nothing here scans every subject's leases on a timer.
//! The only call site is inside admission (`StudyConstraints::admit`), at the
//! moment a due subject's candidate action is already known — the same
//! demand-driven shape `nudge::waker` already has everywhere else. Freshness
//! is derived lazily, `now - observed_at < ttl`, computed at the moment it is
//! asked rather than maintained: a lease six months stale is indistinguishable
//! from no lease at all, so there is nothing to garbage-collect for
//! correctness, only for storage hygiene, which is not this module's problem.
//!
//! ## The asymmetry, by design
//!
//! Uncertain or missing presence sends. Only a fresh, context-matching lease
//! suppresses. A storage failure while fetching leases is therefore read as
//! "no lease" rather than propagated — the same posture `WebSocketFsm::
//! connection_gauges` took for a connection actor that could not answer: a
//! signal that can fail *closed* is worse than the rare redundant nudge that
//! arrives while someone is already looking.

use chrono::{DateTime, Utc};
use presence_repo::PresenceLeaseRepository;
use sqlx::SqlitePool;
use std::time::Duration;
use tracing::warn;

/// Every lease one subject holds, plus the TTL freshness is judged against.
///
/// A snapshot rather than a live query, for the same reason the old
/// `Presence` struct was one: `StudyConstraints::admit` is a synchronous,
/// pure function of its inputs, called from inside `intervention::Engine`
/// where there is no `.await`. Fetching every lease for the subject once,
/// before the engine runs, and answering "is *this* context fresh" out of
/// that fixed snapshot is what keeps `admit` pure while still letting it
/// answer a question — "fresh for *this* action's context" — that depends on
/// an `action` the snapshot's caller does not yet know.
#[derive(Debug, Clone)]
pub struct PresenceLeases {
	leases: Vec<(String, DateTime<Utc>)>,
	ttl: Duration,
}

impl PresenceLeases {
	/// A snapshot with no leases at all — the honest answer for a subject who
	/// has never written one, and the safe answer when the read that would
	/// have populated this failed. Either way, every context reads as absent
	/// rather than fresh, which is the side this asymmetry is supposed to
	/// fail toward.
	#[must_use]
	pub const fn empty(ttl: Duration) -> Self {
		Self { leases: Vec::new(), ttl }
	}

	/// Is there a fresh lease on exactly this context, as of `at`?
	///
	/// `context_key: None` — `StudyAction::GetStarted`, the one variant with
	/// no session to point at — always answers `false`: there is nothing for
	/// a lease to match, so plain absence is never suppressible by presence.
	/// That is a deliberate decision, not a default: a person with no session
	/// prepared cannot be "looking at" the thing this notification invites
	/// them to start, so no lease should ever be able to silence it.
	#[must_use]
	pub fn is_fresh_for(&self, context_key: Option<&str>, at: DateTime<Utc>) -> bool {
		let Some(context_key) = context_key else {
			return false;
		};

		self
			.leases
			.iter()
			.any(|(key, observed_at)| key == context_key && at.signed_duration_since(*observed_at).to_std().is_ok_and(|elapsed| elapsed < self.ttl))
	}

	/// Build a snapshot directly from `(context_key, observed_at)` pairs,
	/// bypassing storage. Test-only: `observe` is the one real constructor,
	/// and this exists so `constraints`'s and `waker`'s test modules can
	/// exercise context-scoped admission without standing up a database.
	#[cfg(test)]
	pub(crate) fn for_test(rows: Vec<(&str, DateTime<Utc>)>, ttl: Duration) -> Self {
		Self {
			leases: rows.into_iter().map(|(key, observed_at)| (key.to_owned(), observed_at)).collect(),
			ttl,
		}
	}
}

/// Fetch every lease a subject currently holds.
///
/// Never fails outward: a storage error is logged and answered as though the
/// subject held no leases at all, per this module's asymmetric-error-cost
/// design — see the module doc comment. Presence must never be able to
/// silence a notification just because reading its own signal broke.
pub async fn observe(db: &SqlitePool, subject_id: &str, ttl: Duration) -> PresenceLeases {
	let rows = match PresenceLeaseRepository::new(db.clone()).for_subject(subject_id).await {
		Ok(rows) => rows,
		Err(err) => {
			warn!(subject = subject_id, error = %err, "could not read presence leases; treating this subject as absent rather than failing closed");
			return PresenceLeases::empty(ttl);
		}
	};

	let leases = rows
		.into_iter()
		.filter_map(|row| {
			let observed_at = crate::nudge::clock::parse_timestamp(&row.observed_at)?;
			Some((row.context_key, observed_at))
		})
		.collect();

	PresenceLeases { leases, ttl }
}

#[cfg(test)]
mod tests {
	use super::{observe, PresenceLeases};
	use chrono::{TimeZone, Utc};
	use std::time::Duration;

	fn at(seconds: i64) -> chrono::DateTime<Utc> {
		Utc.timestamp_opt(1_800_000_000 + seconds, 0).unwrap()
	}

	fn leases(rows: Vec<(&str, chrono::DateTime<Utc>)>, ttl: Duration) -> PresenceLeases {
		PresenceLeases::for_test(rows, ttl)
	}

	#[test]
	fn an_empty_snapshot_is_fresh_for_nothing() {
		let snapshot = PresenceLeases::empty(Duration::from_secs(75));
		assert!(!snapshot.is_fresh_for(Some("session-1"), at(0)));
	}

	#[test]
	fn get_started_has_no_context_key_and_is_never_suppressible() {
		let snapshot = leases(vec![("session-1", at(0))], Duration::from_secs(75));
		// `GetStarted` maps to `None` — there is no session for a lease to match.
		assert!(!snapshot.is_fresh_for(None, at(0)));
	}

	#[test]
	fn a_fresh_lease_on_the_matching_context_is_present() {
		let snapshot = leases(vec![("session-1", at(0))], Duration::from_secs(75));
		assert!(snapshot.is_fresh_for(Some("session-1"), at(30)));
	}

	#[test]
	fn a_stale_lease_is_not_present() {
		let snapshot = leases(vec![("session-1", at(0))], Duration::from_secs(75));
		assert!(!snapshot.is_fresh_for(Some("session-1"), at(76)));
	}

	/// The whole point of a context-scoped lease: a fresh lease on a
	/// *different* session must not suppress a notification about this one. A
	/// site-wide "present" bit could not express this at all.
	#[test]
	fn a_fresh_lease_on_a_different_context_does_not_suppress() {
		let snapshot = leases(vec![("session-other", at(0))], Duration::from_secs(75));
		assert!(!snapshot.is_fresh_for(Some("session-1"), at(0)));
	}

	/// The definition-of-done case: the blackbox WS prober (`infra/blackbox.yml`)
	/// completes a real WebSocket upgrade against `/ws` every 15 seconds and is
	/// tagged `probe:blackbox` for exactly that reason (see
	/// `websocket::connection::client_id_from_request`). The old presence
	/// mechanism counted that connection like any other and could suppress a
	/// real notification because of it. This one cannot, and not because it
	/// happens to filter the right client type — `observe` never looks at the
	/// WebSocket layer at all, so a probe connection existing has no path by
	/// which it could reach this result.
	///
	/// Exercised against a real, migrated database with zero rows in
	/// `presence_leases`, and a real probe connection sitting in the store
	/// alongside it, rather than only reasoned about in prose.
	#[tokio::test]
	async fn the_blackbox_prober_firing_with_zero_real_users_present_yields_no_fresh_lease() {
		use crate::websocket::{connection::client_type_label, WebSocketFsm};
		use axum::http::HeaderMap;
		use sqlx::sqlite::SqlitePoolOptions;
		use std::net::SocketAddr;
		use tokio_util::sync::CancellationToken;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let pool = SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await
			.expect("open an in-memory sqlite database");
		MIGRATOR.run(&pool).await.expect("run the workspace migration history");

		// The probe, exactly as `infra/blackbox.yml` sends it: a real
		// connection lands in the store, tagged `probe`, on every scrape.
		let ws = WebSocketFsm::new();
		let mut headers = HeaderMap::new();
		headers.insert("x-probe-source", "blackbox-exporter".parse().unwrap());
		let addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();
		let client_id = ws.client_id_from_request(&headers, &addr);
		assert_eq!(
			client_type_label(&client_id),
			"probe",
			"sanity check: this is the same probe tagging presence used to trust"
		);
		ws.add_connection(&headers, &addr, &CancellationToken::new())
			.await
			.expect("the probe's connection is accepted like any other");

		// No presence lease was ever written for this subject — nobody real
		// is looking at anything. `observe` has no `ws` argument to consult,
		// so the live probe connection above cannot influence what it finds.
		let leases = observe(&pool, "subject-1", Duration::from_secs(75)).await;
		assert!(
			!leases.is_fresh_for(Some("session-1"), Utc::now()),
			"a WS prober connection must never manufacture a presence lease that was never written"
		);
	}
}
