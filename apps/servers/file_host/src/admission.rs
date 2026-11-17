//! How many requests home serves at once, and how long any of them may take.
//!
//! **AD4: home answers every request within [`ANSWER_WITHIN`], and answers
//! the ones it has no room for at once with `503` and `Retry-After`.**
//!
//! - _Claim:_ every request the inventory's modules serve, but the health
//!   checks ([`EXEMPT`]), gets an answer within
//!   [`ANSWER_WITHIN`] (8 s, under the phone's 10 s deadline), and a request
//!   home cannot serve now (all [`MAX_CONCURRENT_REQUESTS`] taken, no
//!   database connection within [`POOL_ACQUIRE_TIMEOUT`], or the deadline
//!   reached) is answered `503`, `service_overloaded`, with `Retry-After`,
//!   rather than kept waiting.
//! - _Family:_ how many requests arrive at once (up to and past the limit),
//!   how long each one's work takes (under and past the deadline), and
//!   whether the database pool has a connection free (free, all held past
//!   the acquire timeout).
//! - _Proof:_ `tests::a_storm_past_every_limit_is_answered_within_the_bound`
//!   saturates the concurrency limit and the pool together and checks every
//!   request is answered within [`ANSWER_WITHIN`], each either served or
//!   `503` with `Retry-After`; the other tests here take each limit alone.
//!   A handler that maps its own database error keeps a pool timeout busy
//!   through `FileHostError::operation`; in `handlers::db::tab`, the steps
//!   of `delete_tab`, `batch_delete_tabs`, `prune_tabs` and `reconcile_tabs`
//!   each have a test that a pool timeout is answered busy (#946).
//! - _Tolerance:_ the bound starts once the server has accepted the
//!   connection and parsed the request head; time on the network and in
//!   home's proxy before that is outside it. A WebSocket's bound is its
//!   upgrade, not the connection's life. A pool timeout a handler turns into
//!   a string before it reaches `FileHostError` (the dedup cache's reads in
//!   `handlers::db::tab` and `hopium`, `MoodEventError`) is answered within
//!   the bound, but as that handler's `500` or `422`, not busy. A request body
//!   still arriving at 8 s is answered busy (C530).
//!
//! Why the order of [`Admission::bound`]'s layers matters: `tower`'s concurrency
//! limit waits for a free slot, with no bound of its own. Under the
//! deadline, that wait counts against it; under the load shed, a full limit
//! is answered at once instead of waited on. Before AD4 the limit sat
//! outside both, so a request past it waited as long as the requests ahead
//! of it held their slots, and the load shed sat below everything that
//! could be full, so it never shed. And `Router::layer` gives each route its
//! own copy of a layer, so the limit is the global kind, one count shared by
//! every route and every module. Each module applies it inside its own
//! CORS, as it does the rate limiter, so a browser can read a busy answer.
//!
//! All of this is interactive work (docs/home-compute.md, "Classes"):
//! `file_host`'s own background loops share the database pool but not this
//! limit, and a bounded acquire makes them retry on their next pass rather
//! than hold a person's request behind them.

use crate::error::{FileHostError, Shed};
use crate::metrics::refusals;
use axum::{error_handling::HandleErrorLayer, Router};
use std::time::Duration;
use tower::{limit::GlobalConcurrencyLimitLayer, load_shed::LoadShedLayer, timeout::TimeoutLayer, BoxError, ServiceBuilder};

/// The longest home takes to answer any request.
///
/// Under the phone's 10 s deadline (paulgsc/some-ui
/// `file-host-config/client.ts`), with room for the network, so the phone
/// hears home busy rather than home away.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(8);

/// How many requests are served at once, across every route, before more
/// are answered busy. A source constant (#617).
pub const MAX_CONCURRENT_REQUESTS: usize = 100;

/// How long a request waits for a pool connection before it is answered busy.
///
/// Under [`ANSWER_WITHIN`] with room for the query itself,
/// whose own wait on a lock is `db::connect_options`' `busy_timeout` (5 s).
/// The two together can reach the deadline, which then answers busy: the
/// deadline, not their sum, is the bound.
pub const POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);

/// How many connections the server's pool holds. `SQLite` takes one writer
/// at a time, so more would only wait on its lock.
pub const POOL_CONNECTIONS: u32 = 5;

// The bound fits under the phone's 10 s deadline, and a request's wait for a
// connection fits under the bound; a change that breaks either fails to build.
const _: () = assert!(ANSWER_WITHIN.as_millis() < 10_000);
const _: () = assert!(POOL_ACQUIRE_TIMEOUT.as_millis() < ANSWER_WITHIN.as_millis());

/// The modules AD4's admission leaves out: the health checks.
///
/// The container's healthcheck asks `/ready`, and a busy answer there would
/// mark home unhealthy for being busy, and a deploy could roll back a good
/// image for it. Neither waits on the count; `/ready` bounds its own checks
/// at 2 s each (`handlers::readiness`).
const EXEMPT: [&str; 2] = ["health", "readiness"];

/// Whether module `name` is left out of AD4's admission ([`EXEMPT`]).
#[must_use]
pub fn exempt(name: &str) -> bool {
	EXEMPT.contains(&name)
}

/// AD4's limits, applied to each module inside its CORS.
///
/// The deadline sits outermost, then the load shed, then the concurrency
/// limit. Every clone shares one limit: `GlobalConcurrencyLimitLayer` holds
/// its semaphore behind an `Arc`, so every module bounded by clones of one
/// `Admission` draws on the same count.
#[derive(Clone)]
pub struct Admission {
	limit: GlobalConcurrencyLimitLayer,
	within: Duration,
}

impl Admission {
	#[must_use]
	pub fn new(limit: usize, within: Duration) -> Self {
		Self {
			limit: GlobalConcurrencyLimitLayer::new(limit),
			within,
		}
	}

	/// `router` with these limits. Applied inside the module's CORS
	/// (`Module::into_router`), so a busy answer carries the module's CORS
	/// headers and a browser can read its `Retry-After`: outside them, it
	/// reads a network error, which is what the busy answer exists to
	/// prevent.
	pub fn bound<S>(&self, router: Router<S>) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		router.layer(
			ServiceBuilder::new()
				.layer(HandleErrorLayer::new(|error: BoxError| async move { answer(&error) }))
				.layer(TimeoutLayer::new(self.within))
				.layer(LoadShedLayer::new())
				.layer(self.limit.clone()),
		)
	}
}

/// What a request the layers above refused is answered with.
fn answer(error: &BoxError) -> FileHostError {
	if error.is::<tower::timeout::error::Elapsed>() {
		tracing::warn!("Request timeout: {}", error);
		refusals::record_http("timeout");
		FileHostError::Busy(Shed::Timeout)
	} else if error.is::<tower::load_shed::error::Overloaded>() {
		tracing::warn!("Service overloaded: {}", error);
		refusals::record_http("load_shed");
		FileHostError::Busy(Shed::Concurrency)
	} else {
		tracing::error!("Unhandled tower error: {}", error);
		FileHostError::OperationError(error.to_string())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::routes::table::{Module, RouteTable};
	use axum::{
		body::Body,
		extract::State,
		http::{header::RETRY_AFTER, Request, StatusCode},
	};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use std::time::Instant;
	use tower::ServiceExt;

	/// A deadline far shorter than the real one, so the tests run in
	/// seconds; the layers do not know which they were given.
	const WITHIN: Duration = Duration::from_millis(800);

	/// One request through `app`, and how long its answer took.
	async fn ask(app: Router, path: &str) -> (StatusCode, Option<String>, Duration) {
		let started = Instant::now();
		let response = app.oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
		let retry_after = response.headers().get(RETRY_AFTER).map(|value| value.to_str().unwrap().to_owned());
		(response.status(), retry_after, started.elapsed())
	}

	/// Every answer is in time, and each is served or busy with
	/// `Retry-After`; returns how many were busy.
	fn check(answers: &[(StatusCode, Option<String>, Duration)], within: Duration) -> usize {
		let mut busy = 0;
		for (status, retry_after, took) in answers {
			assert!(*took < within, "an answer took {took:?}, past {within:?}");
			if *status == StatusCode::SERVICE_UNAVAILABLE {
				assert_eq!(retry_after.as_deref(), Some("5"), "a busy answer says when to come back");
				busy += 1;
			} else {
				assert_eq!(*status, StatusCode::OK);
			}
		}
		busy
	}

	/// A pool as the server builds it, `size` connections, acquire bounded.
	async fn pool(size: u32, acquire: Duration) -> SqlitePool {
		SqlitePoolOptions::new()
			.max_connections(size)
			.acquire_timeout(acquire)
			.connect("sqlite::memory:")
			.await
			.unwrap()
	}

	/// Until the request in flight holds `pool`'s one connection, and so its
	/// slot, rather than a sleep that a loaded runner can outlast.
	async fn until_held(pool: &SqlitePool) {
		while !(pool.size() == 1 && pool.num_idle() == 0) {
			tokio::time::sleep(Duration::from_millis(5)).await;
		}
	}

	/// A handler that holds a connection for `hold`, as a slow query would.
	async fn holds_a_connection(State((pool, hold)): State<(SqlitePool, Duration)>) -> Result<&'static str, FileHostError> {
		let connection = pool.acquire().await?;
		tokio::time::sleep(hold).await;
		drop(connection);
		Ok("served")
	}

	/// `/db` and `/quick` as two modules, each bounded on its own by clones
	/// of one `Admission`, as `main.rs` bounds every module.
	fn app(pool: SqlitePool, hold: Duration, limit: usize) -> Router {
		let admission = Admission::new(limit, WITHIN);
		let db = Module::unversioned("db", RouteTable::new().get("/db", holds_a_connection)).into_table_router();
		let quick = Module::unversioned("quick", RouteTable::new().get("/quick", || async { "served" })).into_table_router();
		admission.bound(db).merge(admission.bound(quick)).with_state((pool, hold))
	}

	/// AD4: more requests than the limit and the pool together, each
	/// holding a connection, past every bound at once.
	#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
	async fn a_storm_past_every_limit_is_answered_within_the_bound() {
		let limit = 8;
		let app = app(pool(2, Duration::from_millis(200)).await, Duration::from_millis(300), limit);
		let storm: Vec<_> = (0..40).map(|i| tokio::spawn(ask(app.clone(), if i % 4 == 0 { "/quick" } else { "/db" }))).collect();
		let mut answers = Vec::new();
		for request in storm {
			answers.push(request.await.unwrap());
		}
		let busy = check(&answers, WITHIN + Duration::from_millis(200));
		assert!(busy > 0, "a storm past every limit is shed, not all served");
		assert!(busy < answers.len(), "the ones with room are served");
	}

	/// The limit is one count across modules: a second module is shed while
	/// the first holds every slot, at once rather than at the deadline.
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn a_full_limit_is_shed_at_once_on_every_module() {
		let pool = pool(1, Duration::from_secs(5)).await;
		let app = app(pool.clone(), Duration::from_millis(600), 1);
		let holding = tokio::spawn(ask(app.clone(), "/db"));
		until_held(&pool).await;
		let (status, retry_after, took) = ask(app.clone(), "/quick").await;
		assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(retry_after.as_deref(), Some("5"));
		assert!(took < Duration::from_millis(300), "shed at once, not at the deadline: took {took:?}");
		assert_eq!(holding.await.unwrap().0, StatusCode::OK);
	}

	/// A request whose work runs past the deadline is answered busy at it.
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn work_past_the_deadline_is_answered_busy_at_it() {
		let app = app(pool(1, Duration::from_secs(5)).await, Duration::from_secs(5), 4);
		let (status, retry_after, took) = ask(app, "/db").await;
		assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(retry_after.as_deref(), Some("5"));
		assert!(took >= WITHIN && took < WITHIN + Duration::from_millis(200), "answered at the deadline, took {took:?}");
	}

	/// No connection within the acquire timeout is busy, not a `500`.
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn a_pool_with_no_connection_free_is_answered_busy() {
		let pool = pool(1, Duration::from_millis(100)).await;
		let app = app(pool.clone(), Duration::from_millis(500), 4);
		let holding = tokio::spawn(ask(app.clone(), "/db"));
		until_held(&pool).await;
		let (status, retry_after, took) = ask(app, "/db").await;
		assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(retry_after.as_deref(), Some("5"));
		assert!(took < Duration::from_millis(400), "answered at the acquire timeout, took {took:?}");
		assert_eq!(holding.await.unwrap().0, StatusCode::OK);
	}

	/// Each exempt name is a module the server registers, so a rename cannot
	/// put the health checks back under the count unnoticed.
	#[test]
	fn the_exempt_modules_are_the_health_checks() {
		let names: Vec<&str> = crate::routes::inventory::modules::<crate::AppState>().iter().map(Module::name).collect();
		for name in EXEMPT {
			assert!(names.contains(&name), "no module named {name}");
		}
		let paths: Vec<String> = crate::routes::inventory::routes()
			.into_iter()
			.filter(|route| exempt(route.module))
			.map(|route| route.path.to_owned())
			.collect();
		assert_eq!(paths, ["/health", "/ready"]);
	}

	/// A browser on an allowed origin can read a busy answer and its wait:
	/// the limits sit inside the module's CORS (Codex review on #934).
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn a_cross_origin_busy_answer_is_readable() {
		use axum::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ORIGIN};
		use clap::Parser;

		fn module_cors(config: &crate::Config) -> tower_http::cors::CorsLayer {
			crate::routes::cors::allowlisted_cors_with_credentials(config, vec![axum::http::Method::GET], vec![])
		}
		let config = crate::Config::try_parse_from([
			"file_host",
			"--database-url",
			"sqlite::memory:",
			"--auth-enabled",
			"false",
			"--allowed-origins",
			"https://app.example",
			"--app-base-url",
			"https://app.example/",
			"--nats-url",
			"nats://nats:4222",
			"--redis-url",
			"redis://redis:6379",
		])
		.unwrap();
		let admission = Admission::new(4, WITHIN);
		let module = Module::versioned("probe", RouteTable::new().get("/", || async { tokio::time::sleep(Duration::from_secs(5)).await })).with_cors(module_cors);
		let app: Router = module.into_router(&config, |router| admission.bound(router));

		let started = Instant::now();
		let refused = app
			.oneshot(Request::get("/").header(ORIGIN, "https://app.example").body(Body::empty()).unwrap())
			.await
			.unwrap();
		assert!(started.elapsed() < WITHIN + Duration::from_millis(200));
		assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(refused.headers()[RETRY_AFTER], "5");
		assert_eq!(refused.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "https://app.example");
		assert!(refused.headers()[ACCESS_CONTROL_EXPOSE_HEADERS]
			.to_str()
			.unwrap()
			.to_ascii_lowercase()
			.contains("retry-after"));
	}
}
