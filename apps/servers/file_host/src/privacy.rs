//! `docs/identity.md`'s privacy invariants, as tests.
//!
//! Two of them cannot be enforced statically, so they are enforced here
//! against the real thing:
//!
//! - **Nothing at rest can single a person out.** Checked against the schema
//!   the migrations actually produce, not a list someone remembered to update:
//!   no column named for an address, a contact detail or a fingerprint, and
//!   every table classified, in writing, as subject-scoped or not. A new table
//!   fails here until someone decides which it is.
//! - **No address reaches a log line.** A capturing `tracing` layer records
//!   every field of every event and span on the paths that used to log one —
//!   the rate limiter's rejection, a WebSocket connection's admission (#372)
//!   and `ConnectionGuard`'s permit accounting — and the test fails if an
//!   address appears in any of them, however it got there (a field, a
//!   formatted message, a `Debug` of a struct).
//!
//! The static half of the same boundary lives in clippy.toml
//! (`disallowed-types`: `ConnectInfo`) and `scripts/check_privacy.py`
//! (fingerprinting headers).

use crate::{net::peer_key, rate_limiter::token_bucket::rate_limit_middleware, schema::MIGRATOR, WebSocketFsm};
use axum::{
	body::Body,
	extract::connect_info::MockConnectInfo,
	http::{HeaderMap, Request, StatusCode},
	middleware::from_fn_with_state,
	routing::get,
	Router,
};
use some_services::rate_limiter::{PartitionedTokenBucketLimiter, DEFAULT_REFILL_PERIOD_MS};
use sqlx::{sqlite::SqlitePoolOptions, Row};
use std::{
	collections::BTreeSet,
	fmt::{self, Write},
	net::SocketAddr,
	sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tracing::{
	field::{Field, Visit},
	span, Event, Subscriber,
};
use tracing_subscriber::{
	layer::{Context, SubscriberExt},
	registry::LookupSpan,
	Layer, Registry,
};

/// Tables whose rows belong to one subject. A new one is a decision: what it
/// stores about a person, and why that is not identifying.
const SUBJECT_SCOPED: &[(&str, &str)] = &[
	("activity_outcome", "per-block study outcomes"),
	("engagement_charge", "engagement model state"),
	("engagement_gate", "the waker's due index"),
	("intervention_log", "what the waker decided and when"),
	("presence_leases", "which session a subject is looking at, right now"),
	(
		"push_subscriptions",
		"a browser's push endpoint — stable per browser, and known to its push service; see docs/identity.md",
	),
	("sessions", "study sessions"),
];

/// Tables with no subject at all, and why.
const NOT_SUBJECT_SCOPED: &[(&str, &str)] = &[
	("_sqlx_migrations", "sqlx's own migration ledger"),
	("activities", "the activity catalogue — corpus-wide"),
	("curriculum", "lesson content — corpus-wide"),
	("curriculum_publication", "curriculum release ledger"),
	("mood_events", "editorial content, not per-person"),
	(
		"tabs",
		"browser-extension page captures (URL, title, content), keyed by URL hash — no subject column, but the content itself can identify whoever captured it; see docs/identity.md",
	),
];

/// Column-name tokens (split on `_`) that name something identifying.
const FORBIDDEN_TOKENS: &[&str] = &["ip", "ips", "addr", "address", "email", "phone", "fingerprint", "useragent", "forwarded"];

fn forbidden(column: &str) -> bool {
	let lower = column.to_ascii_lowercase();
	lower.contains("user_agent") || lower.split('_').any(|token| FORBIDDEN_TOKENS.contains(&token))
}

#[test]
fn the_column_rule_catches_what_it_is_for_and_nothing_it_is_not() {
	for column in ["ip", "client_ip", "ip_address", "remote_addr", "email", "user_agent", "phone_number", "device_fingerprint"] {
		assert!(forbidden(column), "{column} should be forbidden");
	}
	for column in ["subject_id", "endpoint", "description", "tab_title", "shipped_at", "zip_code", "skip"] {
		assert!(!forbidden(column), "{column} should be allowed");
	}
}

#[tokio::test]
async fn the_migrated_schema_stores_nothing_that_singles_a_person_out() {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();

	let tables: Vec<String> = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
		.fetch_all(&pool)
		.await
		.unwrap()
		.iter()
		.map(|row| row.get("name"))
		.collect();

	let scoped: BTreeSet<&str> = SUBJECT_SCOPED.iter().map(|(table, _)| *table).collect();
	let unscoped: BTreeSet<&str> = NOT_SUBJECT_SCOPED.iter().map(|(table, _)| *table).collect();
	let mut problems: Vec<String> = Vec::new();

	for table in &tables {
		let columns: Vec<String> = sqlx::query("SELECT name FROM pragma_table_info(?)")
			.bind(table)
			.fetch_all(&pool)
			.await
			.unwrap()
			.iter()
			.map(|row| row.get("name"))
			.collect();

		for column in columns.iter().filter(|column| forbidden(column)) {
			problems.push(String::new() + table + "." + column + " is named for something identifying");
		}
		let has_subject = columns.iter().any(|column| column == "subject_id");
		match (scoped.contains(table.as_str()), unscoped.contains(table.as_str())) {
			(false, false) => {
				problems.push(String::new() + table + " is not classified: add it to SUBJECT_SCOPED or NOT_SUBJECT_SCOPED, with why");
			}
			(false, true) if has_subject => {
				problems.push(String::new() + table + " has a subject_id but is classified NOT_SUBJECT_SCOPED");
			}
			(true, _) if !has_subject => {
				problems.push(String::new() + table + " is classified SUBJECT_SCOPED but has no subject_id");
			}
			_ => {}
		}
	}
	for table in scoped.union(&unscoped) {
		if !tables.iter().any(|existing| existing == table) {
			problems.push(String::from(*table) + " is classified but no migration creates it");
		}
	}

	assert!(problems.is_empty(), "{problems:#?}");
}

/// Every field of every event and span, rendered.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
	fn lines(&self) -> Vec<String> {
		self.0.lock().unwrap().clone()
	}

	fn assert_no_address(&self, addresses: &[&str]) {
		let lines = self.lines();
		assert!(!lines.is_empty(), "nothing was captured, so nothing was checked");
		for line in &lines {
			for address in addresses {
				assert!(!line.contains(address), "{address} reached a log line: {line}");
			}
		}
	}
}

struct Render<'a>(&'a mut Vec<String>);

impl Visit for Render<'_> {
	fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
		let mut line = String::new();
		let _ = write!(line, "{}={value:?}", field.name());
		self.0.push(line);
	}

	fn record_str(&mut self, field: &Field, value: &str) {
		self.0.push(String::from(field.name()) + "=" + value);
	}
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Captured {
	fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
		let mut lines = Vec::new();
		event.record(&mut Render(&mut lines));
		self.0.lock().unwrap().extend(lines);
	}

	fn on_new_span(&self, attrs: &span::Attributes<'_>, _id: &span::Id, _ctx: Context<'_, S>) {
		let mut lines = Vec::new();
		attrs.record(&mut Render(&mut lines));
		self.0.lock().unwrap().extend(lines);
	}

	fn on_record(&self, _span: &span::Id, values: &span::Record<'_>, _ctx: Context<'_, S>) {
		let mut lines = Vec::new();
		values.record(&mut Render(&mut lines));
		self.0.lock().unwrap().extend(lines);
	}
}

fn capture() -> (Captured, tracing::subscriber::DefaultGuard) {
	let captured = Captured::default();
	let guard = tracing::subscriber::set_default(Registry::default().with(captured.clone()));
	(captured, guard)
}

/// #372: this middleware logged `client_id = <ip>` on every rejection.
#[tokio::test]
async fn a_rate_limited_request_logs_no_address() {
	let (captured, _guard) = capture();
	let limiter = Arc::new(PartitionedTokenBucketLimiter::new(1, DEFAULT_REFILL_PERIOD_MS));
	let app = Router::new()
		.route("/", get(|| async { "ok" }))
		.layer(from_fn_with_state(limiter, rate_limit_middleware))
		.layer(MockConnectInfo(SocketAddr::from(([10, 1, 2, 3], 5555))));

	let first = app.clone().oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();
	let second = app.oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();

	assert_eq!(first.status(), StatusCode::OK);
	assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS, "sanity check: the rejection path ran");
	let lines = captured.lines();
	assert!(lines.iter().any(|line| line.contains("rate limit exceeded")), "{lines:#?}");
	// Without this the test passes vacuously: a harness whose connection
	// info never reaches the extractor logs `unknown-peer`, which contains no
	// address because it was never given one.
	assert!(
		lines.iter().any(|line| line.starts_with("peer=") && !line.contains(crate::net::UNKNOWN_PEER)),
		"the peer was not keyed from the connection info: {lines:#?}"
	);
	captured.assert_no_address(&["10.1.2.3"]);
}

/// #372: admission logged `addr = <ip:port>` and a `client_id` built from the
/// forwarded address.
#[tokio::test]
async fn admitting_a_websocket_connection_logs_no_address() {
	let (captured, _guard) = capture();
	let fsm = WebSocketFsm::new();
	let mut headers = HeaderMap::new();
	headers.insert("x-forwarded-for", "10.9.8.7".parse().unwrap());

	fsm
		.add_connection(&headers, &peer_key(SocketAddr::from(([10, 1, 2, 3], 5555))), &CancellationToken::new())
		.await
		.unwrap();

	assert!(
		captured.lines().iter().any(|line| line.contains("Connection added successfully")),
		"{:#?}",
		captured.lines()
	);
	captured.assert_no_address(&["10.1.2.3", "10.9.8.7"]);
}

/// Codex P2 on #374: admission is counted under the process-stable
/// `AdmissionKey` so a connection held across midnight still counts, which
/// makes that key linkable across days — so `ConnectionGuard` must never log
/// it. Acquire and release both log; neither may name the key or the address.
#[tokio::test]
async fn connection_admission_logs_neither_the_admission_key_nor_the_address() {
	let (captured, _guard) = capture();
	let guard = ws_conn_manager::ConnectionGuard::new();
	let addr = SocketAddr::from(([10, 1, 2, 3], 5555));
	let key = crate::net::admission_key(addr).into_string();

	let permits = [
		guard.acquire(key.clone()).await.unwrap(),
		guard.acquire(crate::net::admission_key(addr).into_string()).await.unwrap(),
	];
	assert_eq!(guard.active_per_client(&key), 2, "sanity check: both counted against one client");
	permits.into_iter().for_each(ws_conn_manager::ConnectionPermit::release);

	let lines = captured.lines();
	assert!(lines.iter().any(|line| line.contains("acquired active slot")), "{lines:#?}");
	for line in &lines {
		assert!(!line.contains(&key), "the admission key reached a log line: {line}");
	}
	captured.assert_no_address(&["10.1.2.3"]);
}
