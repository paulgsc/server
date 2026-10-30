//! `GET /ready` — can this process serve, and how much of it?
//!
//! See #213 (F1). `/health` (`handlers::health`) is a liveness probe: it
//! consults nothing and is correct as long as the runtime is scheduling.
//! This is the readiness half of that split — it checks every dependency
//! `AppState` actually holds a handle to (`SQLite`, NATS, Redis) and answers
//! 503 with a per-dependency breakdown when any of them is unreachable.
//!
//! `schema` is the fourth, and the only one that isn't a connection: a
//! reachable `SQLite` file that is missing migrations this binary's queries
//! were compiled against. Only the queries that touch what those migrations
//! add fail, on whichever one happens to touch it first rather than here.
//! See `crate::schema` for why neither `cargo check` nor CI can catch it.
//!
//! The answer has three levels, not one bit ([`Verdict`]). A dependency is
//! `critical` when losing it leaves nothing to serve; losing one that is not
//! leaves the process **degraded**: it serves, minus what that dependency
//! carries. `ready` and the 503 still mean "every dependency is healthy",
//! for anyone reading only the status code; the container healthcheck
//! (`crate::health`) reads `status` and fails only when the process is
//! **unavailable**, so a degraded one stays up, and so does everything that
//! starts after it. On 2026-10-30 an image ahead of its database's
//! migrations served the Jobs flow normally while its 503 held Prometheus
//! back and blanked every dashboard (some-ui#2199).
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
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::time::Duration;
use tracing::instrument;

/// Bounds each check that can actually block (`SQLite`, schema, Redis).
/// Well under the 15s `TASK_TIMEOUT_MS` request timeout even run
/// sequentially — run concurrently (as they are here) the whole handler
/// resolves in one dependency's worth of this budget, not the sum of three.
const DEPENDENCY_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether losing `dependency` leaves this process nothing to serve.
///
/// Critical unless shown otherwise, so a dependency added later gates the
/// healthcheck until someone argues it doesn't. `schema` is shown: a
/// database behind this build fails only the queries that touch what the
/// pending migrations add (`main.rs` starts and serves on it for the same
/// reason), and on 2026-10-30 the Jobs flow worked with five pending. NATS
/// and Redis stay critical, as they were before this split.
fn critical(dependency: &str) -> bool {
	dependency != "schema"
}

#[derive(Serialize)]
struct DependencyStatus {
	name: &'static str,
	healthy: bool,
	/// See [`critical`].
	critical: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	error: Option<String>,
}

impl DependencyStatus {
	fn up(name: &'static str) -> Self {
		Self {
			name,
			healthy: true,
			critical: critical(name),
			error: None,
		}
	}

	fn down(name: &'static str, error: String) -> Self {
		Self {
			name,
			healthy: false,
			critical: critical(name),
			error: Some(error),
		}
	}
}

/// How much of this process can serve: every dependency healthy, only
/// non-critical ones down, or a critical one down.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
	Ready,
	Degraded,
	Unavailable,
}

impl Verdict {
	fn of(dependencies: &[DependencyStatus]) -> Self {
		if dependencies.iter().any(|d| !d.healthy && d.critical) {
			Self::Unavailable
		} else if dependencies.iter().any(|d| !d.healthy) {
			Self::Degraded
		} else {
			Self::Ready
		}
	}
}

#[derive(Serialize)]
pub struct ReadinessResponse {
	/// Every dependency healthy: [`Verdict::Ready`], and the only 200.
	ready: bool,
	status: Verdict,
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
	// only the three checks that can actually await something are run
	// concurrently.
	let nats = check_nats(&state);
	let (sqlite, mut schema, redis) = tokio::join!(check_sqlite(&state.core.shared_db), check_schema(&state.core.shared_db), check_redis(&state));

	// An unreadable database can't have its migrations read either, and
	// "schema: disk I/O error" next to "sqlite: disk I/O error" points at
	// `sqlx migrate run` for what is really a missing volume. Unhealthy even
	// if the schema read itself succeeded — the two checks can land on
	// different pooled connections, and one database that answers one query
	// and fails the next hasn't verified anything — but the message says
	// whose failure it is; DEPS hides the schema tile while sqlite's is
	// showing for the same reason.
	if !sqlite.healthy {
		schema = DependencyStatus::down("schema", "not checked: sqlite is unreachable".to_string());
	}

	// A gauge per dependency, not just the aggregate `ready` bool — #225's
	// DEPS panel needs to name which one is down, not just that something is.
	gauge!("dependency_up", "dependency" => "sqlite").set(f64::from(sqlite.healthy));
	gauge!("dependency_up", "dependency" => "nats").set(f64::from(nats.healthy));
	gauge!("dependency_up", "dependency" => "redis").set(f64::from(redis.healthy));
	gauge!("dependency_up", "dependency" => "schema").set(f64::from(schema.healthy));

	let dependencies = vec![sqlite, nats, redis, schema];
	let verdict = Verdict::of(&dependencies);
	let ready = verdict == Verdict::Ready;
	let status = if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

	(
		status,
		Json(ReadinessResponse {
			ready,
			status: verdict,
			service: build_info::SERVICE,
			version: env!("CARGO_PKG_VERSION"),
			git_sha: build_info::git_sha_with_dirty_marker(),
			built_at: build_info::BUILT_AT,
			rust: build_info::RUSTC_VERSION,
			dependencies,
		}),
	)
}

async fn check_sqlite(pool: &SqlitePool) -> DependencyStatus {
	let result = tokio::time::timeout(DEPENDENCY_CHECK_TIMEOUT, sqlx::query("SELECT 1").fetch_one(pool)).await;
	match result {
		Ok(Ok(_)) => DependencyStatus::up("sqlite"),
		Ok(Err(e)) => DependencyStatus::down("sqlite", e.to_string()),
		Err(_) => DependencyStatus::down(
			"sqlite",
			"check did not complete within ".to_string() + &DEPENDENCY_CHECK_TIMEOUT.as_secs().to_string() + "s",
		),
	}
}

/// Healthy only when every migration embedded in this binary is applied —
/// the error names the pending ones and the command that applies them. A
/// database that can't be read at all is reported here too rather than
/// guessed about, though `sqlite` above is the check that owns that failure.
async fn check_schema(pool: &SqlitePool) -> DependencyStatus {
	let result = tokio::time::timeout(DEPENDENCY_CHECK_TIMEOUT, crate::schema::drift(pool, &crate::schema::MIGRATOR)).await;
	match result {
		Ok(Ok(drift)) if drift.is_current() => DependencyStatus::up("schema"),
		Ok(Ok(drift)) => DependencyStatus::down("schema", drift.to_string()),
		Ok(Err(e)) => DependencyStatus::down("schema", "could not read _sqlx_migrations: ".to_string() + &e.to_string()),
		Err(_) => DependencyStatus::down(
			"schema",
			"check did not complete within ".to_string() + &DEPENDENCY_CHECK_TIMEOUT.as_secs().to_string() + "s",
		),
	}
}

/// A non-blocking connection-state read, not a round-trip — see
/// `NatsTransport::is_connected`'s own doc comment for why that is the
/// correct trade-off for a readiness probe. Nothing here can hang, so it
/// does not need `DEPENDENCY_CHECK_TIMEOUT`.
fn check_nats(state: &AppState) -> DependencyStatus {
	if state.realtime.transport.is_connected() {
		DependencyStatus::up("nats")
	} else {
		DependencyStatus::down("nats", "connection not established".to_string())
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
		Ok(Ok(_)) => DependencyStatus::up("redis"),
		Ok(Err(e)) => DependencyStatus::down("redis", e.to_string()),
		Err(_) => DependencyStatus::down(
			"redis",
			"check did not complete within ".to_string() + &DEPENDENCY_CHECK_TIMEOUT.as_secs().to_string() + "s",
		),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn deps(down: &[&'static str]) -> Vec<DependencyStatus> {
		["sqlite", "nats", "redis", "schema"]
			.into_iter()
			.map(|name| {
				if down.contains(&name) {
					DependencyStatus::down(name, "down".to_string())
				} else {
					DependencyStatus::up(name)
				}
			})
			.collect()
	}

	#[test]
	fn every_dependency_healthy_is_ready() {
		assert_eq!(Verdict::of(&deps(&[])), Verdict::Ready);
	}

	#[test]
	fn a_database_behind_this_build_is_degraded_not_unavailable() {
		assert_eq!(Verdict::of(&deps(&["schema"])), Verdict::Degraded);
	}

	#[test]
	fn a_critical_dependency_down_is_unavailable_whatever_else_holds() {
		for down in [&["sqlite"][..], &["nats"], &["redis"], &["sqlite", "schema"], &["redis", "schema"]] {
			assert_eq!(Verdict::of(&deps(down)), Verdict::Unavailable, "{down:?}");
		}
	}

	#[test]
	fn a_dependency_is_critical_unless_shown_otherwise() {
		assert!(critical("sqlite") && critical("nats") && critical("redis") && critical("a dependency added later"));
		assert!(!critical("schema"));
	}

	#[test]
	fn the_status_reads_as_a_lowercase_word() {
		let words: Vec<String> = [Verdict::Ready, Verdict::Degraded, Verdict::Unavailable]
			.iter()
			.map(|v| serde_json::to_value(v).unwrap().to_string())
			.collect();
		assert_eq!(words, ["\"ready\"", "\"degraded\"", "\"unavailable\""]);
	}
}
