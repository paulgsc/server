//! The mailbox's bell (MBP2, paulgsc/server#500): home holds a WebSocket open
//! to the mailbox, and the mailbox rings it when a file lands.
//!
//! The pass loop in [`super`] runs a pass for any of four causes ([`Cause`]):
//! a ring, a (re)connect, something home filed for the mailbox itself, and
//! the backstop timer. Each is raised on one [`Wake`], and every cause raised
//! while a pass runs folds into at most one more pass, so passes never
//! overlap and never queue.
//!
//! **The protocol** (paulgsc/some-ui `apps/mailbox`, MBP1 #2142):
//!
//! - `GET {MAILBOX_URL}/v1/bell` with `Upgrade: websocket` and home's mailbox
//!   token as the `Authorization: Bearer` header, as every route takes it.
//!   The token never goes in the URL.
//! - The mailbox sends `{"ring":"<folder>"}` and nothing else of a file: no
//!   name, bytes or hash. Home ignores fields it doesn't know, and any other
//!   text. A ring for any folder starts a pass, which lists what's new the
//!   same way the timer's pass does.
//! - Home sends `ping` every [`Timings::ping`], and the bell answers `pong`
//!   without waking (`setWebSocketAutoResponse`). Hearing nothing for
//!   [`Timings::silence`] means the connection is dead.
//! - 404 means the mailbox has no bell yet (before MBP1 deploys). Home asks
//!   again at the backstop's pace and says so once, not on every attempt.
//!
//! **Home stays outbound-only.** This module opens a connection and listens
//! on it. It never binds a port.
//!
//! **A dead bell is visible, never silent.** `mailbox_bell_connected` reads
//! 0, `mailbox_bell_failures_total{why}` counts each failed or dropped
//! connection, and the first failure of each kind is logged. Passes go on
//! by the timer whatever the bell does.
//!
//! The token is a `mailbox_kit::Token`, which never prints, and failures are
//! logged by kind: no token, URL or frame body reaches a log line.

use mailbox_kit::Token;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header::AUTHORIZATION, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Why a pass runs. The lowest value wins when several fold into one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Cause {
	/// The mailbox rang: a file landed in a folder home reads, or the routine
	/// asked for an archived file back.
	Ring = 0,
	/// Home (re)connected to the bell, so it catches up on whatever was put
	/// while it wasn't listening.
	Connect = 1,
	/// Home filed something for the mailbox: a file the phone sent, or a
	/// voice note the transcriber heard.
	Local = 2,
	/// The backstop timer, `mailbox::INTERVAL`.
	Timer = 3,
}

impl Cause {
	/// Every cause, in the order the loop prefers them.
	pub const ALL: [Self; 4] = [Self::Ring, Self::Connect, Self::Local, Self::Timer];

	/// The `cause` label and the log word.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Ring => "ring",
			Self::Connect => "connect",
			Self::Local => "local",
			Self::Timer => "timer",
		}
	}

	const fn bit(self) -> u8 {
		1 << (self as u8)
	}
}

/// What wakes the pass loop: one permit, and the causes raised since the
/// loop last took them.
///
/// Raising stores at most one permit, however many times it is raised, so
/// causes raised during a pass fold into exactly one more.
#[derive(Debug, Default)]
pub struct Wake {
	notify: Notify,
	pending: AtomicU8,
	/// Whether home holds the bell, which sets the backstop's pace.
	held: AtomicBool,
	changed: Notify,
}

impl Wake {
	/// Say whether home holds the bell now.
	pub fn set_held(&self, held: bool) {
		if self.held.swap(held, Ordering::SeqCst) != held {
			self.changed.notify_one();
		}
	}

	/// Whether home holds the bell.
	pub fn held(&self) -> bool {
		self.held.load(Ordering::SeqCst)
	}

	/// Wait until the bell is taken or lost. Cancel-safe: a change while
	/// nobody waits is kept for the next call.
	pub async fn held_changed(&self) {
		self.changed.notified().await;
	}

	/// Ask for a pass, for `cause`.
	pub fn raise(&self, cause: Cause) {
		self.pending.fetch_or(cause.bit(), Ordering::SeqCst);
		self.notify.notify_one();
	}

	/// Wait until a cause is raised, then take every cause raised so far and
	/// return the one the pass is counted under.
	///
	/// Cancel-safe: the causes stay pending until a call returns them.
	pub async fn wait(&self) -> Cause {
		loop {
			if let Some(cause) = self.take() {
				return cause;
			}
			self.notify.notified().await;
		}
	}

	/// Take every pending cause, or `None` when nothing was raised. The
	/// stored permit, if any, stays: the next [`Self::wait`] finds no cause
	/// and waits again.
	pub fn take(&self) -> Option<Cause> {
		let bits = self.pending.swap(0, Ordering::SeqCst);
		Cause::ALL.into_iter().find(|cause| bits & cause.bit() != 0)
	}
}

/// How the bell paces itself. [`Timings::new`] is production's; tests shrink
/// them.
#[derive(Debug, Clone, Copy)]
pub struct Timings {
	/// How often home sends `ping`.
	pub ping: Duration,
	/// How long without hearing anything (a `pong`, a ring, a frame) before
	/// the connection counts as dead.
	pub silence: Duration,
	/// The first retry's delay, doubled on each failure after it.
	pub backoff_floor: Duration,
	/// The longest delay between attempts.
	pub backoff_cap: Duration,
	/// A connection held this long starts the backoff over, so a bell that
	/// connects and drops at once backs off rather than reconnecting (and
	/// running a catch-up pass) every second.
	pub steady: Duration,
	/// The delay while the mailbox has no bell (404): the backstop's pace.
	pub not_deployed: Duration,
	/// How long the connect and its handshake may take.
	pub connect_timeout: Duration,
}

impl Timings {
	/// Production's pacing, retrying a missing bell every `backstop`.
	#[must_use]
	pub const fn new(backstop: Duration) -> Self {
		Self {
			ping: Duration::from_secs(30),
			silence: Duration::from_secs(90),
			backoff_floor: Duration::from_secs(1),
			backoff_cap: Duration::from_secs(300),
			steady: Duration::from_secs(600),
			not_deployed: backstop,
			connect_timeout: Duration::from_secs(30),
		}
	}

	/// The delay before attempt `failures + 1`, after `failures` in a row:
	/// the floor doubled per failure, capped, less up to a quarter of it as
	/// jitter (`jitter` in `0.0..1.0`), so a restarted Worker isn't met by a
	/// reconnect at the same instant every time.
	#[must_use]
	pub fn backoff(&self, failures: u32, jitter: f64) -> Duration {
		let doubled = self.backoff_floor.saturating_mul(1_u32.checked_shl(failures.min(31)).unwrap_or(u32::MAX));
		let capped = doubled.min(self.backoff_cap);
		capped.mul_f64(1.0 - jitter.clamp(0.0, 1.0) / 4.0)
	}
}

/// Where the bell is and who asks for it.
pub struct Bell {
	url: String,
	token: Token,
	timings: Timings,
}

// Hand-written: neither the URL (which names the owner's Worker) nor the token
// belongs in a log line.
impl std::fmt::Debug for Bell {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Bell").finish_non_exhaustive()
	}
}

impl Bell {
	/// The bell of the mailbox at `base`, an `https://` host as
	/// `MailboxContext::from_config` has already checked, reached as `wss://`.
	///
	/// # Errors
	/// A `base` that is not `https://`.
	pub fn new(base: &str, token: Token, timings: Timings) -> Result<Self, &'static str> {
		let Some(host) = base.trim_end_matches('/').strip_prefix("https://") else {
			return Err("the mailbox URL must start with https://");
		};
		Ok(Self::at(String::from("wss://") + host + "/v1/bell", token, timings))
	}

	/// A bell at exactly `url`, `ws://` allowed: for tests, which run a bell
	/// on the loopback without TLS.
	#[must_use]
	pub const fn at(url: String, token: Token, timings: Timings) -> Self {
		Self { url, token, timings }
	}
}

/// How one connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
	/// The server is shutting down.
	Cancelled,
	/// 404: the mailbox has no bell yet.
	NotDeployed,
	/// The attempt failed before a connection was held, by kind.
	Failed(&'static str),
	/// A held connection ended, by kind, after `held`.
	Dropped(&'static str, Duration),
}

/// Every value `mailbox_bell_failures_total`'s `why` label can take.
///
/// An attempt refused (`unauthorized`, `forbidden`, `not_found`, `http`), one
/// that never connected (`transport`, `timeout`), or a held connection that
/// ended (`closed`, `silent`, `dropped`).
pub const FAILURES: [&str; 9] = ["unauthorized", "forbidden", "not_found", "http", "transport", "timeout", "closed", "silent", "dropped"];

/// The ring's one field. Anything else in it is ignored.
#[derive(Deserialize)]
struct Ring {
	ring: String,
}

/// Hold the bell until `cancel`, raising [`Cause::Connect`] on each connect
/// and [`Cause::Ring`] on each ring.
pub async fn hold(bell: Arc<Bell>, wake: Arc<Wake>, cancel: CancellationToken) {
	crate::metrics::mailbox::record_bell_started();
	let timings = bell.timings;
	let mut failures: u32 = 0;
	let mut last_said: Option<&'static str> = None;
	loop {
		let ended = connect_once(&bell, &wake, &cancel).await;
		wake.set_held(false);
		crate::metrics::mailbox::record_bell_connected(false);
		let delay = match ended {
			Ended::Cancelled => return,
			Ended::NotDeployed => {
				crate::metrics::mailbox::record_bell_failure("not_found");
				if last_said != Some("not_found") {
					info!(
						retry_secs = timings.not_deployed.as_secs(),
						"the mailbox has no bell yet (404); home reads it on the timer and asks again at that pace"
					);
					last_said = Some("not_found");
				}
				failures = 0;
				timings.not_deployed
			}
			Ended::Failed(why) => {
				crate::metrics::mailbox::record_bell_failure(why);
				let delay = timings.backoff(failures, rand::random::<f64>());
				if last_said != Some(why) {
					warn!(
						why,
						retry_secs = delay.as_secs(),
						"the mailbox's bell did not connect; home reads it on the timer until it does"
					);
					last_said = Some(why);
				}
				failures = failures.saturating_add(1);
				delay
			}
			Ended::Dropped(why, held) => {
				crate::metrics::mailbox::record_bell_failure(why);
				if held >= timings.steady {
					failures = 0;
				}
				let delay = timings.backoff(failures, rand::random::<f64>());
				// A drop after a steady connection is said every time; a flapping
				// one, once per kind until it holds again.
				if held >= timings.steady || last_said != Some(why) {
					warn!(
						why,
						held_secs = held.as_secs(),
						retry_secs = delay.as_secs(),
						"the mailbox's bell dropped; home reconnects and reads it on the timer meanwhile"
					);
					last_said = Some(why);
				}
				failures = failures.saturating_add(1);
				delay
			}
		};
		tokio::select! {
			() = cancel.cancelled() => return,
			() = tokio::time::sleep(delay) => {}
		}
	}
}

/// One attempt: connect, then listen until the connection ends.
async fn connect_once(bell: &Bell, wake: &Wake, cancel: &CancellationToken) -> Ended {
	let timings = bell.timings;
	let Ok(mut request) = bell.url.as_str().into_client_request() else {
		return Ended::Failed("http");
	};
	let Ok(mut authorization) = HeaderValue::from_str(&(String::from("Bearer ") + bell.token.expose())) else {
		return Ended::Failed("http");
	};
	authorization.set_sensitive(true);
	request.headers_mut().insert(AUTHORIZATION, authorization);
	// A ring is a few bytes; nothing the bell sends is large.
	let config = WebSocketConfig {
		max_message_size: Some(4096),
		max_frame_size: Some(4096),
		..WebSocketConfig::default()
	};

	let attempt = tokio::time::timeout(timings.connect_timeout, tokio_tungstenite::connect_async_with_config(request, Some(config), false));
	let connected = tokio::select! {
		() = cancel.cancelled() => return Ended::Cancelled,
		connected = attempt => connected,
	};
	let mut socket = match connected {
		Err(_) => return Ended::Failed("timeout"),
		Ok(Err(WsError::Http(response))) => {
			return match response.status().as_u16() {
				404 => Ended::NotDeployed,
				401 => Ended::Failed("unauthorized"),
				403 => Ended::Failed("forbidden"),
				_ => Ended::Failed("http"),
			}
		}
		Ok(Err(_)) => return Ended::Failed("transport"),
		Ok(Ok((socket, _))) => socket,
	};

	let since = Instant::now();
	crate::metrics::mailbox::record_bell_connected(true);
	info!("the mailbox's bell is connected");
	wake.set_held(true);
	wake.raise(Cause::Connect);

	let mut ping = tokio::time::interval_at(since + timings.ping, timings.ping);
	ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	let mut heard = since;
	loop {
		use futures::{SinkExt, StreamExt};
		tokio::select! {
			() = cancel.cancelled() => {
				// Best effort: the process is ending either way.
				let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
				return Ended::Cancelled;
			}
			() = tokio::time::sleep_until(heard + timings.silence) => return Ended::Dropped("silent", since.elapsed()),
			_ = ping.tick() => {
				if socket.send(Message::text("ping")).await.is_err() {
					return Ended::Dropped("dropped", since.elapsed());
				}
			}
			frame = socket.next() => match frame {
				Some(Ok(Message::Text(text))) => {
					heard = Instant::now();
					if let Ok(ring) = serde_json::from_str::<Ring>(&text) {
						crate::metrics::mailbox::record_ring(&ring.ring);
						wake.raise(Cause::Ring);
					}
				}
				Some(Ok(Message::Close(_))) | None => return Ended::Dropped("closed", since.elapsed()),
				Some(Ok(Message::Ping(_))) => {
					heard = Instant::now();
					// tungstenite queued the pong; send it now.
					if socket.flush().await.is_err() {
						return Ended::Dropped("dropped", since.elapsed());
					}
				}
				Some(Ok(_)) => heard = Instant::now(),
				Some(Err(_)) => return Ended::Dropped("dropped", since.elapsed()),
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use futures::{SinkExt, StreamExt};
	use tokio::net::TcpListener;
	use tokio::sync::mpsc;
	use tokio_tungstenite::tungstenite::handshake::server::{Callback, ErrorResponse, Request, Response};

	fn quick() -> Timings {
		Timings {
			ping: Duration::from_millis(50),
			silence: Duration::from_millis(400),
			backoff_floor: Duration::from_millis(20),
			backoff_cap: Duration::from_millis(80),
			steady: Duration::from_secs(60),
			not_deployed: Duration::from_millis(200),
			connect_timeout: Duration::from_secs(2),
		}
	}

	async fn next_cause(wake: &Wake) -> Cause {
		tokio::time::timeout(Duration::from_secs(5), wake.wait()).await.unwrap()
	}

	#[tokio::test]
	async fn causes_raised_during_a_pass_fold_into_one_and_the_ring_wins() {
		let wake = Wake::default();
		wake.raise(Cause::Local);
		wake.raise(Cause::Ring);
		wake.raise(Cause::Connect);
		assert_eq!(next_cause(&wake).await, Cause::Ring);
		// One stored permit, no causes: nothing more is owed.
		assert!(tokio::time::timeout(Duration::from_millis(50), wake.wait()).await.is_err());
		wake.raise(Cause::Local);
		assert_eq!(next_cause(&wake).await, Cause::Local);
	}

	#[test]
	fn the_backoff_doubles_to_its_cap_and_jitter_only_shortens_it() {
		let timings = Timings::new(Duration::from_secs(900));
		assert_eq!(timings.backoff(0, 0.0), Duration::from_secs(1));
		assert_eq!(timings.backoff(3, 0.0), Duration::from_secs(8));
		assert_eq!(timings.backoff(40, 0.0), Duration::from_secs(300));
		assert_eq!(timings.backoff(40, 1.0), Duration::from_secs(225));
		assert!(timings.backoff(2, 0.5) < timings.backoff(2, 0.0));
	}

	#[test]
	fn the_bell_is_the_mailbox_host_over_wss_and_never_prints() {
		let bell = Bell::new("https://mailbox.example.test/", Token::new("secret-token".to_owned()), quick()).unwrap();
		assert_eq!(bell.url, "wss://mailbox.example.test/v1/bell");
		let mut shown = String::new();
		std::fmt::Write::write_fmt(&mut shown, format_args!("{bell:?}")).unwrap();
		assert!(!shown.contains("secret-token") && !shown.contains("example.test"), "{shown}");
		assert!(Bell::new("http://mailbox.example.test", Token::new("t".to_owned()), quick()).is_err());
	}

	/// What the fake bell saw of each connection.
	#[derive(Debug)]
	enum Seen {
		Connected(Option<String>),
		Text(String),
	}

	fn loopback(listener: &TcpListener) -> String {
		String::from("ws://") + &listener.local_addr().unwrap().to_string() + "/v1/bell"
	}

	/// What the fake bell's handshake saw of `Authorization`.
	#[derive(Clone, Default)]
	struct Capture(Arc<std::sync::Mutex<Option<String>>>);

	impl Callback for Capture {
		fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
			*self.0.lock().unwrap() = request.headers().get(AUTHORIZATION).and_then(|value| value.to_str().ok()).map(str::to_owned);
			Ok(response)
		}
	}

	/// A bell on the loopback. Each connection gets `script(n)` for the n-th
	/// connection: the frames to send, then whether to drop it.
	async fn fake_bell(script: fn(usize) -> (Vec<&'static str>, bool)) -> (String, mpsc::UnboundedReceiver<Seen>) {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = loopback(&listener);
		let (seen, saw) = mpsc::unbounded_channel();
		tokio::spawn(async move {
			let mut n = 0;
			loop {
				let (stream, _) = listener.accept().await.unwrap();
				let seen = seen.clone();
				let (frames, drop_it) = script(n);
				n += 1;
				tokio::spawn(async move {
					let capture = Capture::default();
					let mut socket = tokio_tungstenite::accept_hdr_async(stream, capture.clone()).await.unwrap();
					let authorization = capture.0.lock().unwrap().take();
					let _ = seen.send(Seen::Connected(authorization));
					for frame in frames {
						socket.send(Message::text(frame)).await.unwrap();
					}
					if drop_it {
						let _ = socket.close(None).await;
						return;
					}
					while let Some(Ok(message)) = socket.next().await {
						if let Message::Text(text) = message {
							if text == "ping" {
								let _ = socket.send(Message::text("pong")).await;
							}
							let _ = seen.send(Seen::Text(text));
						}
					}
				});
			}
		});
		(url, saw)
	}

	#[tokio::test]
	async fn a_ring_and_each_reconnect_wake_a_pass_and_the_token_is_the_bearer() {
		// The first connection rings `brief`, says something home doesn't
		// know, and drops; the second holds.
		let (url, mut saw) = fake_bell(|n| {
			if n == 0 {
				(vec![r#"{"ring":"brief","extra":1}"#, "hello"], true)
			} else {
				(vec![], false)
			}
		})
		.await;
		let wake = Arc::new(Wake::default());
		let cancel = CancellationToken::new();
		let task = tokio::spawn(hold(
			Arc::new(Bell::at(url, Token::new("home-token".to_owned()), quick())),
			Arc::clone(&wake),
			cancel.clone(),
		));

		let Some(Seen::Connected(authorization)) = saw.recv().await else {
			panic!("no connection")
		};
		assert_eq!(authorization.as_deref(), Some("Bearer home-token"));
		// The connect and the ring may fold into one pass; the ring wins.
		let first = next_cause(&wake).await;
		assert!(matches!(first, Cause::Connect | Cause::Ring), "{first:?}");
		if first == Cause::Connect {
			assert_eq!(next_cause(&wake).await, Cause::Ring);
		}
		// The drop is followed by a reconnect, and the reconnect by a pass.
		let Some(Seen::Connected(_)) = saw.recv().await else { panic!("no reconnect") };
		assert_eq!(next_cause(&wake).await, Cause::Connect);
		// The held connection is pinged, and answered, so it stays held.
		let Some(Seen::Text(ping)) = saw.recv().await else { panic!("no ping") };
		assert_eq!(ping, "ping");
		tokio::time::sleep(quick().silence * 2).await;
		assert!(wake.take().is_none(), "a pinged bell stays connected: no further pass");

		cancel.cancel();
		tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
	}

	/// The pass loop with a recording pass: each pass's cause, in order.
	/// No timer pass happens within a test's few seconds.
	fn recording(wake: &Arc<Wake>, cancel: &CancellationToken, takes: Duration) -> (tokio::task::JoinHandle<()>, mpsc::UnboundedReceiver<Cause>) {
		recording_failing(wake, cancel, takes, 0, Duration::from_secs(3600))
	}

	/// As [`recording`], with the first `failing` passes failing, and the
	/// timer `unheld` after a failed pass or while the bell isn't held (an
	/// hour while it is).
	fn recording_failing(
		wake: &Arc<Wake>,
		cancel: &CancellationToken,
		takes: Duration,
		failing: usize,
		unheld: Duration,
	) -> (tokio::task::JoinHandle<()>, mpsc::UnboundedReceiver<Cause>) {
		let ran_so_far = Arc::new(std::sync::atomic::AtomicUsize::new(0));
		let (ran, passes) = mpsc::unbounded_channel();
		let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
		let (wake, cancel) = (Arc::clone(wake), cancel.clone());
		let task = tokio::spawn(async move {
			super::super::run_passes(
				&wake,
				Duration::from_secs(3600),
				unheld,
				&cancel,
				|cause| {
					let (ran, busy) = (ran.clone(), Arc::clone(&busy));
					let nth = ran_so_far.fetch_add(1, Ordering::SeqCst);
					async move {
						assert!(!busy.swap(true, Ordering::SeqCst), "two passes at once");
						let _ = ran.send(cause);
						tokio::time::sleep(takes).await;
						busy.store(false, Ordering::SeqCst);
						nth >= failing
					}
				},
				|| async {},
			)
			.await;
		});
		(task, passes)
	}

	async fn next_pass(passes: &mut mpsc::UnboundedReceiver<Cause>) -> Cause {
		tokio::time::timeout(Duration::from_secs(5), passes.recv()).await.unwrap().unwrap()
	}

	#[tokio::test]
	async fn passes_never_overlap_and_causes_during_one_fold_into_one_more() {
		let wake = Arc::new(Wake::default());
		let cancel = CancellationToken::new();
		let (task, mut passes) = recording(&wake, &cancel, Duration::from_millis(200));
		// The first pass runs at once, on the timer's first tick.
		assert_eq!(next_pass(&mut passes).await, Cause::Timer);
		for cause in [Cause::Local, Cause::Ring, Cause::Ring, Cause::Connect] {
			wake.raise(cause);
		}
		assert_eq!(next_pass(&mut passes).await, Cause::Ring, "four causes during a pass: one more pass, counted as the ring");
		assert!(tokio::time::timeout(Duration::from_millis(500), passes.recv()).await.is_err(), "and no third");
		cancel.cancel();
		task.await.unwrap();
	}

	/// A failed pass is tried again by the timer after the retry, not after
	/// the hour-long backstop, so three failures in a row take 45 minutes on
	/// home, as `MailboxPollerFailing` counts them.
	#[tokio::test]
	async fn a_failed_pass_is_retried_soon_and_a_good_one_waits_the_backstop() {
		let wake = Arc::new(Wake::default());
		wake.set_held(true);
		let cancel = CancellationToken::new();
		let (task, mut passes) = recording_failing(&wake, &cancel, Duration::from_millis(1), 1, Duration::from_millis(300));
		assert_eq!(next_pass(&mut passes).await, Cause::Timer);
		let started = Instant::now();
		assert_eq!(next_pass(&mut passes).await, Cause::Timer, "the retry");
		assert!(started.elapsed() >= Duration::from_millis(250), "after the retry's delay");
		assert!(tokio::time::timeout(Duration::from_millis(800), passes.recv()).await.is_err(), "a good pass waits the hour");
		cancel.cancel();
		task.await.unwrap();
	}

	/// With the bell held and nothing ringing, the loop passes once an hour
	/// and stamps the routine watch every 5 minutes between, so RR2's due
	/// time moves on time.
	#[tokio::test(start_paused = true)]
	async fn an_idle_loop_passes_hourly_and_watches_every_five_minutes() {
		let wake = Wake::default();
		wake.set_held(true);
		let cancel = CancellationToken::new();
		let (passes, watches) = (std::cell::Cell::new(0_u32), std::cell::Cell::new(0_u32));
		let run = super::super::run_passes(
			&wake,
			super::super::INTERVAL,
			super::super::UNHELD,
			&cancel,
			|_| {
				passes.set(passes.get() + 1);
				async { true }
			},
			|| {
				watches.set(watches.get() + 1);
				async {}
			},
		);
		let stop = async {
			tokio::time::sleep(Duration::from_secs(61 * 60)).await;
			cancel.cancel();
		};
		tokio::join!(run, stop);
		assert_eq!(passes.get(), 2, "at once, then an hour later");
		assert_eq!(watches.get(), 12, "every 5 minutes for 61 minutes");
	}

	/// The backstop follows the bell (C64): every 15 minutes while it isn't
	/// held; when it drops, 15 minutes from the last pass, at once if that is
	/// past; and back to hourly after a reconnect's catch-up pass.
	#[tokio::test(start_paused = true)]
	async fn the_backstop_is_hourly_while_the_bell_is_held_and_15_minutes_while_it_is_not() {
		let wake = Wake::default();
		let cancel = CancellationToken::new();
		let start = tokio::time::Instant::now();
		let passes = std::cell::RefCell::new(Vec::new());
		let minute = |m: u64| start + Duration::from_secs(m * 60);
		let run = super::super::run_passes(
			&wake,
			super::super::INTERVAL,
			super::super::UNHELD,
			&cancel,
			|cause| {
				passes.borrow_mut().push((start.elapsed().as_secs() / 60, cause));
				async { true }
			},
			|| async {},
		);
		let script = async {
			// Not held: 0, 15, 30. Held at 31: the next is an hour after 30.
			tokio::time::sleep_until(minute(31)).await;
			wake.set_held(true);
			// Dropped at 50, 20 minutes after the last pass: one at once.
			tokio::time::sleep_until(minute(50)).await;
			wake.set_held(false);
			// Then 65. Taken again at 70, with its catch-up pass, then hourly: 130.
			tokio::time::sleep_until(minute(70)).await;
			wake.set_held(true);
			wake.raise(Cause::Connect);
			// Dropped at 135, 5 minutes after the last: 15 minutes after it, 145.
			tokio::time::sleep_until(minute(135)).await;
			wake.set_held(false);
			tokio::time::sleep_until(minute(146)).await;
			cancel.cancel();
		};
		tokio::join!(run, script);
		let timer = Cause::Timer;
		assert_eq!(
			passes.into_inner(),
			[
				(0, timer),
				(15, timer),
				(30, timer),
				(50, timer),
				(65, timer),
				(70, Cause::Connect),
				(130, timer),
				(145, timer)
			]
		);
	}

	/// #500's "done when": with a bell that drops, the reconnect runs a pass.
	#[tokio::test]
	async fn a_bell_that_drops_runs_a_pass_on_reconnect() {
		let (url, mut saw) = fake_bell(|n| (vec![], n == 0)).await;
		let wake = Arc::new(Wake::default());
		let cancel = CancellationToken::new();
		let (passes_task, mut passes) = recording(&wake, &cancel, Duration::from_millis(1));
		assert_eq!(next_pass(&mut passes).await, Cause::Timer);
		let bell = tokio::spawn(hold(Arc::new(Bell::at(url, Token::new("t".to_owned()), quick())), Arc::clone(&wake), cancel.clone()));
		let Some(Seen::Connected(_)) = saw.recv().await else { panic!("no connection") };
		assert_eq!(next_pass(&mut passes).await, Cause::Connect);
		let Some(Seen::Connected(_)) = saw.recv().await else { panic!("no reconnect") };
		assert_eq!(next_pass(&mut passes).await, Cause::Connect, "the reconnect's catch-up pass");
		cancel.cancel();
		bell.await.unwrap();
		passes_task.await.unwrap();
	}

	#[tokio::test]
	async fn a_bell_that_falls_silent_is_dropped_and_reconnected() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = loopback(&listener);
		let (seen, mut saw) = mpsc::unbounded_channel::<()>();
		tokio::spawn(async move {
			loop {
				let (stream, _) = listener.accept().await.unwrap();
				// Accept, hold, and never answer: no pong, no frame.
				let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
				tokio::spawn(async move {
					let _held = socket;
					std::future::pending::<()>().await;
				});
				let _ = seen.send(());
			}
		});
		let wake = Arc::new(Wake::default());
		let cancel = CancellationToken::new();
		let task = tokio::spawn(hold(Arc::new(Bell::at(url, Token::new("t".to_owned()), quick())), Arc::clone(&wake), cancel.clone()));
		saw.recv().await.unwrap();
		assert_eq!(next_cause(&wake).await, Cause::Connect);
		tokio::time::timeout(Duration::from_secs(5), saw.recv()).await.unwrap().unwrap();
		assert_eq!(next_cause(&wake).await, Cause::Connect);
		cancel.cancel();
		tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
	}

	#[tokio::test]
	async fn a_mailbox_with_no_bell_is_asked_again_at_the_backstops_pace() {
		// Answers every request 404, as the Worker does before MBP1.
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = loopback(&listener);
		let (seen, mut saw) = mpsc::unbounded_channel::<Instant>();
		tokio::spawn(async move {
			loop {
				let (mut stream, _) = listener.accept().await.unwrap();
				let _ = seen.send(Instant::now());
				tokio::spawn(async move {
					use tokio::io::{AsyncReadExt, AsyncWriteExt};
					let mut buffer = [0_u8; 2048];
					let _ = stream.read(&mut buffer).await;
					let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
				});
			}
		});
		let wake = Arc::new(Wake::default());
		let cancel = CancellationToken::new();
		let task = tokio::spawn(hold(Arc::new(Bell::at(url, Token::new("t".to_owned()), quick())), Arc::clone(&wake), cancel.clone()));
		let first = saw.recv().await.unwrap();
		let second = tokio::time::timeout(Duration::from_secs(5), saw.recv()).await.unwrap().unwrap();
		assert!(second - first >= quick().not_deployed, "a 404 waits the backstop's pace, not the backoff's");
		assert!(wake.take().is_none(), "a bell that never connected wakes no pass");
		cancel.cancel();
		tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
	}
}
