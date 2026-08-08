//! `GET /ready` — can this process serve, not just answer a socket?
//!
//! See #213 (F1). `/health` (`handlers::health`) is a liveness probe: it
//! consults nothing and is correct as long as the runtime is scheduling.
//! This is the readiness half of that split — it checks every dependency
//! `AppState` actually holds a handle to (`SQLite`, NATS, Redis) and answers
//! 503 with a per-dependency breakdown when any of them is unreachable.
//!
//! Every check that can actually block is bounded by
//! [`DEPENDENCY_CHECK_TIMEOUT`] and run concurrently: a readiness probe
//! that hangs waiting on one dependency is worse than one that answers
//! "not ready" promptly, and there is no reason a slow `SQLite` lock should
//! delay finding out Redis is also down. NATS's check is a non-blocking
//! state read (see `check_nats`) with nothing to bound or join.

use crate::metrics::build_info;
use crate::AppState;
use axum::{extract::State, http::StatusCode, response::Json};
use metrics::gauge;
use serde::Serialize;
use sqlx::SqlitePool;
use std::time::Duration;
use tracing::instrument;

/// Bounds each of the two checks that can actually block (`SQLite`, Redis).
/// Well under the 15s `TASK_TIMEOUT_MS` request timeout even run
/// sequentially — run concurrently (as they are here) the whole handler
/// resolves in one dependency's worth of this budget, not the sum of two.
const DEPENDENCY_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize)]
struct DependencyStatus {
	name: &'static str,
	healthy: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	error: Option<String>,
}

#[derive(Serialize)]
pub struct ReadinessResponse {
	ready: bool,
	service: &'static str,
	version: &'static str,
	// #213 (F4): the same identity `service_info` carries, duplicated here
	// so the answer survives Prometheus being down — the dashboard must not
	// be the only source of truth for which build is running.
	git_sha: String,
	built_at: &'static str,
	rust: &'static str,
	dependencies: Vec<DependencyStatus>,
}

#[axum::debug_handler]
#[instrument(name = "readiness", skip_all)]
pub async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
	// `check_nats` is synchronous (a non-blocking state read, not a
	// round-trip — see its own doc comment), so it isn't part of the join;
	// only the two checks that can actually await something are run
	// concurrently.
	let nats = check_nats(&state);
	let (sqlite, redis) = tokio::join!(check_sqlite(&state.core.shared_db), check_redis(&state));

	// A gauge per dependency, not just the aggregate `ready` bool — #225's
	// DEPS panel needs to name which one is down, not just that something is.
	gauge!("dependency_up", "dependency" => "sqlite").set(f64::from(sqlite.healthy));
	gauge!("dependency_up", "dependency" => "nats").set(f64::from(nats.healthy));
	gauge!("dependency_up", "dependency" => "redis").set(f64::from(redis.healthy));

	let ready = sqlite.healthy && nats.healthy && redis.healthy;
	let status = if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

	(
		status,
		Json(ReadinessResponse {
			ready,
			service: build_info::SERVICE,
			version: env!("CARGO_PKG_VERSION"),
			git_sha: build_info::git_sha_with_dirty_marker(),
			built_at: build_info::BUILT_AT,
			rust: build_info::RUSTC_VERSION,
			dependencies: vec![sqlite, nats, redis],
		}),
	)
}

async fn check_sqlite(pool: &SqlitePool) -> DependencyStatus {
	let result = tokio::time::timeout(DEPENDENCY_CHECK_TIMEOUT, sqlx::query("SELECT 1").fetch_one(pool)).await;
	match result {
		Ok(Ok(_)) => DependencyStatus { name: "sqlite", healthy: true, error: None },
		Ok(Err(e)) => DependencyStatus {
			name: "sqlite",
			healthy: false,
			error: Some(e.to_string()),
		},
		Err(_) => DependencyStatus {
			name: "sqlite",
			healthy: false,
			error: Some("check did not complete within ".to_string() + &DEPENDENCY_CHECK_TIMEOUT.as_secs().to_string() + "s"),
		},
	}
}

/// A non-blocking connection-state read, not a round-trip — see
/// `NatsTransport::is_connected`'s own doc comment for why that is the
/// correct trade-off for a readiness probe. Nothing here can hang, so it
/// does not need `DEPENDENCY_CHECK_TIMEOUT`.
fn check_nats(state: &AppState) -> DependencyStatus {
	if state.realtime.transport.is_connected() {
		DependencyStatus { name: "nats", healthy: true, error: None }
	} else {
		DependencyStatus {
			name: "nats",
			healthy: false,
			error: Some("connection not established".to_string()),
		}
	}
}

async fn check_redis(state: &AppState) -> DependencyStatus {
	let client = state.realtime.dedup_cache.store().redis_client().clone();
	let result = tokio::time::timeout(DEPENDENCY_CHECK_TIMEOUT, async move {
		let mut conn = client.get_multiplexed_async_connection().await?;
		redis::cmd("PING").query_async::<String>(&mut conn).await
	})
	.await;

	match result {
		Ok(Ok(_)) => DependencyStatus { name: "redis", healthy: true, error: None },
		Ok(Err(e)) => DependencyStatus {
			name: "redis",
			healthy: false,
			error: Some(e.to_string()),
		},
		Err(_) => DependencyStatus {
			name: "redis",
			healthy: false,
			error: Some("check did not complete within ".to_string() + &DEPENDENCY_CHECK_TIMEOUT.as_secs().to_string() + "s"),
		},
	}
}
