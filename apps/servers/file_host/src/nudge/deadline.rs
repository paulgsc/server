//! A pass's deadline and the clock it reads (#264, SLI3): `NudgeContext::pass_deadline`
//! after the pass began, checked between subjects and between deliveries, never inside one.
//! Split out of `waker` (S9, #489); the waker's own doc says why the bound is where it is.

use std::time::Duration;

/// What a pass reads its deadline against (#575): the time since it began,
/// and a bound on one piece of work. `TokioClock` in production; a test can
/// step one forward only when a delivery's budget runs out, so where the
/// deadline falls is exact, not a race with how fast the machine is.
///
/// Storage work takes none of a stepped clock's time. That is the point:
/// what the deadline tests check is the pass's accounting at the boundary,
/// and on a real clock the boundary lands wherever a slow runner puts it.
/// (A paused tokio clock can't do this here: sqlx pings a `SQLite`
/// connection on its own thread as it returns to the pool, and a paused
/// runtime waiting on that skips ahead to the next pool timeout.)
pub(crate) trait PassClock: Sync {
	/// Time since the pass began.
	fn elapsed(&self) -> Duration;

	/// `work`, or `None` once `budget` has passed without it finishing.
	fn timeout<F: std::future::Future + Send>(&self, budget: Duration, work: F) -> impl std::future::Future<Output = Option<F::Output>> + Send;
}

/// The wall clock, through tokio's timer.
pub(crate) struct TokioClock {
	started: tokio::time::Instant,
}

impl TokioClock {
	pub(crate) fn start() -> Self {
		Self {
			started: tokio::time::Instant::now(),
		}
	}
}

impl PassClock for TokioClock {
	fn elapsed(&self) -> Duration {
		self.started.elapsed()
	}

	async fn timeout<F: std::future::Future + Send>(&self, budget: Duration, work: F) -> Option<F::Output> {
		tokio::time::timeout(budget, work).await.ok()
	}
}

/// A pass's deadline: `NudgeContext::pass_deadline` after its clock began,
/// and what the pass learned of whether home reached the push services.
pub(crate) struct Deadline<C> {
	pub(super) clock: C,
	pub(super) after: Duration,
	pub(super) reach: Reach,
}

impl<C: PassClock> Deadline<C> {
	pub(crate) fn new(clock: C, after: Duration) -> Self {
		Self {
			clock,
			after,
			reach: Reach::default(),
		}
	}

	pub(super) fn passed(&self) -> bool {
		self.clock.elapsed() >= self.after
	}

	/// What the pass has left; zero once it has passed.
	pub(super) fn left(&self) -> Duration {
		self.after.saturating_sub(self.clock.elapsed())
	}
}

/// Whether home reached the internet in one pass, and the strikes waiting on
/// it (#1180, the coordinator's call C1513).
///
/// A browser that timed out, or got no answer, failed on its own only if
/// home could reach push services at all: an uplink or DNS outage makes every
/// browser fail alike, and that is home's. So such a strike is held until the
/// pass ends and taken only if some push service, for any subject, answered
/// in the pass ([`Reach::heard`]); otherwise none is. Bound: a pass whose only
/// send is to one dead browser takes no strike, so a lone browser is set aside
/// only across passes that also reached another push service.
#[derive(Default)]
pub(crate) struct Reach {
	answered: std::sync::atomic::AtomicBool,
	held: tokio::sync::Mutex<Vec<(String, &'static str)>>,
}

impl Reach {
	/// A push service answered (anything: accepted, refused, gone).
	pub(super) fn heard(&self) {
		self.answered.store(true, std::sync::atomic::Ordering::Relaxed);
	}

	/// A strike for `endpoint`, `why`, taken at [`Reach::settle`] if home was
	/// heard.
	pub(super) async fn hold(&self, endpoint: &str, why: &'static str) {
		self.held.lock().await.push((endpoint.to_owned(), why));
	}

	/// The strikes this pass held, if a push service answered in it; none
	/// otherwise. Each is taken once.
	pub(super) async fn settle(&self) -> Vec<(String, &'static str)> {
		let held = std::mem::take(&mut *self.held.lock().await);
		if self.answered.load(std::sync::atomic::Ordering::Relaxed) {
			held
		} else {
			if !held.is_empty() {
				tracing::warn!(held = held.len(), "no push service answered this pass: home's, so no browser takes a strike");
			}
			Vec::new()
		}
	}
}
