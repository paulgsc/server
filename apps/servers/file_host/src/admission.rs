//! How many requests home serves at once, and how long any of them may take.
//!
//! **AD4: home answers every request within [`ANSWER_WITHIN`], and answers
//! the ones it has no room for at once with `503` and `Retry-After`.**
//!
//! - _Claim:_ every request the router serves gets an answer within
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
//! - _Tolerance:_ the bound starts once the server has accepted the
//!   connection and parsed the request head; time on the network and in
//!   home's proxy before that is outside it. A WebSocket's bound is its
//!   upgrade, not the connection's life.
//!
//! Why the order of [`bounded`]'s layers matters: `tower`'s concurrency
//! limit waits for a free slot, with no bound of its own. Under the
//! deadline, that wait counts against it; under the load shed, a full limit
//! is answered at once instead of waited on. Before AD4 the limit sat
//! outside both, so a request past it waited as long as the requests ahead
//! of it held their slots, and the load shed sat below everything that
//! could be full, so it never shed. And `Router::layer` gives each route its
//! own copy of a layer, so the limit is the global kind, one count shared by
//! every route.
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
/// whose own wait on a lock is `db::connect_options`' `busy_timeout`.
pub const POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);

/// How many connections the server's pool holds. `SQLite` takes one writer
/// at a time, so more would only wait on its lock.
pub const POOL_CONNECTIONS: u32 = 5;

// The bound fits under the phone's 10 s deadline, and a request's wait for a
// connection fits under the bound; a change that breaks either fails to build.
const _: () = assert!(ANSWER_WITHIN.as_millis() < 10_000);
const _: () = assert!(POOL_ACQUIRE_TIMEOUT.as_millis() < ANSWER_WITHIN.as_millis());

/// `router` with AD4's limits: the deadline outermost, then the load shed,
/// then the one concurrency limit every route shares.
pub fn bounded<S>(router: Router<S>, limit: usize, within: Duration) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	router.layer(
		ServiceBuilder::new()
			.layer(HandleErrorLayer::new(|error: BoxError| async move { answer(&error) }))
			.layer(TimeoutLayer::new(within))
			.layer(LoadShedLayer::new())
			.layer(GlobalConcurrencyLimitLayer::new(limit)),
	)
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

	/// A handler that holds a connection for `hold`, as a slow query would.
	async fn holds_a_connection(State((pool, hold)): State<(SqlitePool, Duration)>) -> Result<&'static str, FileHostError> {
		let connection = pool.acquire().await?;
		tokio::time::sleep(hold).await;
		drop(connection);
		Ok("served")
	}

	fn app(pool: SqlitePool, hold: Duration, limit: usize) -> Router {
		let table = RouteTable::new().get("/db", holds_a_connection).get("/quick", || async { "served" });
		bounded(Module::unversioned("probe", table).into_table_router(), limit, WITHIN).with_state((pool, hold))
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

	/// The limit is one count across routes: a second route is shed while
	/// the first holds every slot, at once rather than at the deadline.
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn a_full_limit_is_shed_at_once_on_every_route() {
		let app = app(pool(1, Duration::from_secs(5)).await, Duration::from_millis(600), 1);
		let holding = tokio::spawn(ask(app.clone(), "/db"));
		tokio::time::sleep(Duration::from_millis(100)).await;
		let (status, retry_after, took) = ask(app.clone(), "/quick").await;
		assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(retry_after.as_deref(), Some("5"));
		assert!(took < Duration::from_millis(100), "shed at once, took {took:?}");
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
		let app = app(pool(1, Duration::from_millis(100)).await, Duration::from_millis(500), 4);
		let holding = tokio::spawn(ask(app.clone(), "/db"));
		tokio::time::sleep(Duration::from_millis(50)).await;
		let (status, retry_after, took) = ask(app, "/db").await;
		assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(retry_after.as_deref(), Some("5"));
		assert!(took < Duration::from_millis(400), "answered at the acquire timeout, took {took:?}");
		assert_eq!(holding.await.unwrap().0, StatusCode::OK);
	}
}
