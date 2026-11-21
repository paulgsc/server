//! The one way home reaches outside (paulgsc/some-ui#3330).
//!
//! Every HTTP client in `file_host` and its crates is built here, by
//! [`client`], for one [`Caller`]: a closed enum with one case per worker
//! that makes requests to the outside. A new worker that fetches anything
//! adds a case, or it has no client: `clippy.toml` refuses
//! `reqwest::Client::new` and `reqwest::Client::builder` anywhere else.
//!
//! Every request sent through a [`Client`] is recorded once, on the
//! `metrics` facade (`some-metrics` exposes it):
//!
//! - [`REQUESTS_METRIC`], by caller, host and outcome;
//! - [`DURATION_METRIC`], by caller and host, from send to the answer's headers;
//! - [`SENT_METRIC`] and [`RECEIVED_METRIC`], by caller and host.
//!
//! Every label is a closed set the compiler holds. `host` is a fixed name
//! per caller ([`Caller::host`]), never read from a URL, so no URL, query,
//! body or address reaches a label.
//!
//! What is measured ends at the answer's headers: the caller reads the body
//! afterwards. So a request whose body stalls or fails after the headers
//! counts as its status's outcome, and the bytes received are the answer's
//! declared `Content-Length`, whether or not the caller then reads it (an
//! answer that declares none, a chunked one, adds nothing). A worker's own
//! series (the game feed's `game_feed_fetches_total{kind}`) say what the body
//! came to; the dashboard's cadvisor row is the wire's own count.
//!
//! A client's counters are set to zero when it is built ([`Builder::build`]),
//! so its first request after a restart is an increase, and "nothing
//! failed" reads 0 rather than no data.

use std::time::{Duration, Instant};

/// `outbound_requests_total{caller, host, outcome}`.
pub const REQUESTS_METRIC: &str = "outbound_requests_total";
/// `outbound_request_duration_seconds{caller, host}`: from send to the
/// answer's headers, or to the failure.
pub const DURATION_METRIC: &str = "outbound_request_duration_seconds";
/// `outbound_sent_bytes_total{caller, host}`: request bodies.
pub const SENT_METRIC: &str = "outbound_sent_bytes_total";
/// `outbound_received_bytes_total{caller, host}`: declared answer lengths.
pub const RECEIVED_METRIC: &str = "outbound_received_bytes_total";

/// [`DURATION_METRIC`]'s buckets, for the binary's `PrometheusBuilder`.
///
/// The exporter's defaults top out at 10 s; the pollers' timeouts run to
/// 30 s, so a timed-out request needs a bucket above it or it is only `+Inf`.
/// The nudge's client has no timeout of its own (as before this crate), so a
/// push that hangs past 60 s lands in `+Inf`.
pub const DURATION_BUCKETS: &[f64] = &[0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 20.0, 30.0, 60.0];

/// Who is asking: one case per worker that reaches outside home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Caller {
	/// `file_host::game_feed`: ESPN's scoreboard and game summaries.
	GameFeed,
	/// `file_host::call_weeks`: the weekly call's folder in `paulgsc/testing`.
	CallWeeks,
	/// `file_host::cards_essays`: the cards' essays in `paulgsc/testing`.
	CardsEssays,
	/// `file_host::mailbox`: the Jobs mailbox Worker's files, and its archive.
	Mailbox,
	/// `file_host::jobs_notice`: FCM's token and send, for a phone's notice.
	Notice,
	/// `file_host::nudge`: Web Push to each browser's push service.
	Nudge,
	/// `file_host::snips`: the snips bundle of a commit a waiting cards essay
	/// names, from GitHub's Actions API (SNIPS2), with `SNIPS_GITHUB_TOKEN`.
	Snips,
	/// `file_host --health-check`: its own `/ready`, on loopback. The probe
	/// exits straight after, so nothing scrapes it; it is a case so that the
	/// probe's client is built here like every other.
	HealthProbe,
}

impl Caller {
	/// Every caller, for a test or a check that walks the family.
	pub const ALL: [Self; 8] = [
		Self::GameFeed,
		Self::CallWeeks,
		Self::CardsEssays,
		Self::Mailbox,
		Self::Notice,
		Self::Nudge,
		Self::Snips,
		Self::HealthProbe,
	];

	/// The `caller` label.
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::GameFeed => "game_feed",
			Self::CallWeeks => "call_weeks",
			Self::CardsEssays => "cards_essays",
			Self::Mailbox => "mailbox",
			Self::Notice => "notice",
			Self::Nudge => "nudge",
			Self::Snips => "snips",
			Self::HealthProbe => "health_probe",
		}
	}

	/// The one host this caller reaches.
	#[must_use]
	pub const fn host(self) -> Host {
		match self {
			Self::GameFeed => Host::Espn,
			Self::CallWeeks | Self::CardsEssays => Host::GithubRaw,
			Self::Mailbox => Host::MailboxWorker,
			Self::Notice => Host::Googleapis,
			Self::Nudge => Host::WebPush,
			Self::Snips => Host::GithubApi,
			Self::HealthProbe => Host::Loopback,
		}
	}
}

/// Where a caller's requests go, named rather than read from the URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Host {
	/// `site.api.espn.com`.
	Espn,
	/// `raw.githubusercontent.com`.
	GithubRaw,
	/// `api.github.com`, and the storage an artifact's download redirects
	/// to: one name, since the redirect is GitHub's to choose.
	GithubApi,
	/// The owner's Jobs mailbox Worker on `workers.dev` (`MAILBOX_URL`).
	MailboxWorker,
	/// `oauth2.googleapis.com` and `fcm.googleapis.com`.
	Googleapis,
	/// Each subscribed browser's push service (FCM, Mozilla's, Apple's):
	/// one name for all of them, since the endpoint is per browser.
	WebPush,
	/// `127.0.0.1`: home asking itself.
	Loopback,
}

impl Host {
	/// The `host` label.
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Espn => "espn",
			Self::GithubRaw => "github_raw",
			Self::GithubApi => "github_api",
			Self::MailboxWorker => "mailbox_worker",
			Self::Googleapis => "googleapis",
			Self::WebPush => "web_push",
			Self::Loopback => "loopback",
		}
	}
}

/// How a request ended, by kind: the `outcome` label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
	/// An answer below 400.
	Ok,
	/// A 4xx answer.
	Refused4xx,
	/// A 5xx answer, or any status above.
	Refused5xx,
	/// No answer's headers within the client's or the request's timeout.
	Timeout,
	/// No answer's headers for another reason: no connection, a reset, TLS.
	NoAnswer,
}

impl Outcome {
	/// Every outcome.
	pub const ALL: [Self; 5] = [Self::Ok, Self::Refused4xx, Self::Refused5xx, Self::Timeout, Self::NoAnswer];

	/// The `outcome` label.
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Ok => "ok",
			Self::Refused4xx => "refused_4xx",
			Self::Refused5xx => "refused_5xx",
			Self::Timeout => "timeout",
			Self::NoAnswer => "no_answer",
		}
	}

	/// The outcome of an answer with `status`.
	#[must_use]
	pub const fn of_status(status: u16) -> Self {
		match status {
			0..=399 => Self::Ok,
			400..=499 => Self::Refused4xx,
			_ => Self::Refused5xx,
		}
	}

	/// The outcome of a send that got no answer.
	#[must_use]
	pub fn of_error(err: &reqwest::Error) -> Self {
		if err.is_timeout() {
			Self::Timeout
		} else {
			Self::NoAnswer
		}
	}
}

/// The one constructor: a client for `caller`.
///
/// This is the only place a `reqwest::Client` is built (`clippy.toml`'s
/// `disallowed-methods` refuses it anywhere else).
pub fn client(caller: Caller) -> Builder {
	// The one sanctioned construction: see the module doc.
	#[allow(clippy::disallowed_methods)]
	let inner = reqwest::Client::builder();
	Builder { caller, inner }
}

/// [`client`]'s settings, before it is built.
#[derive(Debug)]
#[must_use]
pub struct Builder {
	caller: Caller,
	inner: reqwest::ClientBuilder,
}

impl Builder {
	/// Every request's timeout, connecting through reading the body.
	pub fn timeout(self, timeout: Duration) -> Self {
		Self {
			inner: self.inner.timeout(timeout),
			..self
		}
	}

	/// Ignore the environment's proxy (a test's own loopback server).
	pub fn no_proxy(self) -> Self {
		Self {
			inner: self.inner.no_proxy(),
			..self
		}
	}

	/// The client, with its caller's counters set to zero.
	///
	/// # Errors
	/// The TLS backend could not be set up.
	pub fn build(self) -> Result<Client, reqwest::Error> {
		// The one sanctioned construction: see the module doc.
		#[allow(clippy::disallowed_methods)]
		let inner = self.inner.build()?;
		zero(self.caller);
		Ok(Client { caller: self.caller, inner })
	}
}

/// An HTTP client that records every request it sends as its [`Caller`]'s.
///
/// Cheap to clone: the connection pool is shared, as `reqwest::Client`'s is.
#[derive(Debug, Clone)]
pub struct Client {
	caller: Caller,
	inner: reqwest::Client,
}

impl Client {
	/// Whose requests this client sends.
	#[must_use]
	pub const fn caller(&self) -> Caller {
		self.caller
	}

	/// A `GET` to `url`.
	pub fn get<U: reqwest::IntoUrl>(&self, url: U) -> Request {
		self.request(reqwest::Method::GET, url)
	}

	/// A `POST` to `url`.
	pub fn post<U: reqwest::IntoUrl>(&self, url: U) -> Request {
		self.request(reqwest::Method::POST, url)
	}

	/// A `PUT` to `url`.
	pub fn put<U: reqwest::IntoUrl>(&self, url: U) -> Request {
		self.request(reqwest::Method::PUT, url)
	}

	/// A `DELETE` to `url`.
	pub fn delete<U: reqwest::IntoUrl>(&self, url: U) -> Request {
		self.request(reqwest::Method::DELETE, url)
	}

	/// A request with `method` to `url`.
	pub fn request<U: reqwest::IntoUrl>(&self, method: reqwest::Method, url: U) -> Request {
		Request {
			client: self.clone(),
			builder: self.inner.request(method, url),
		}
	}
}

/// A request being assembled. Unlike `reqwest::RequestBuilder`, its only way
/// onto the wire is [`Request::send`], which records it.
#[derive(Debug)]
#[must_use]
pub struct Request {
	client: Client,
	builder: reqwest::RequestBuilder,
}

impl Request {
	/// Add a header.
	pub fn header<K, V>(self, key: K, value: V) -> Self
	where
		reqwest::header::HeaderName: TryFrom<K>,
		<reqwest::header::HeaderName as TryFrom<K>>::Error: Into<http::Error>,
		reqwest::header::HeaderValue: TryFrom<V>,
		<reqwest::header::HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
	{
		Self {
			builder: self.builder.header(key, value),
			..self
		}
	}

	/// Add `Authorization: Bearer <token>`.
	pub fn bearer_auth<T: std::fmt::Display>(self, token: T) -> Self {
		Self {
			builder: self.builder.bearer_auth(token),
			..self
		}
	}

	/// Set the body.
	pub fn body<T: Into<reqwest::Body>>(self, body: T) -> Self {
		Self {
			builder: self.builder.body(body),
			..self
		}
	}

	/// This request's own timeout, in place of the client's.
	pub fn timeout(self, timeout: Duration) -> Self {
		Self {
			builder: self.builder.timeout(timeout),
			..self
		}
	}

	/// Send it, and record it once as its caller's.
	///
	/// A request that does not build (a URL that does not parse, a header
	/// that is not one) never reaches the wire and is not recorded.
	///
	/// # Errors
	/// As `reqwest::RequestBuilder::send`: the request did not build, or got
	/// no answer.
	pub async fn send(self) -> Result<reqwest::Response, reqwest::Error> {
		let Self { client, builder } = self;
		let request = builder.build()?;
		let sent = request.body().and_then(reqwest::Body::as_bytes).map_or(0, <[u8]>::len);
		let started = Instant::now();
		let answer = client.inner.execute(request).await;
		let (outcome, received) = match &answer {
			Ok(response) => (Outcome::of_status(response.status().as_u16()), response.content_length().unwrap_or(0)),
			Err(err) => (Outcome::of_error(err), 0),
		};
		record(client.caller, outcome, started.elapsed(), sent, received);
		answer
	}
}

/// `caller`'s counters at zero, so a first request is an increase.
fn zero(caller: Caller) {
	let (caller_label, host_label) = (caller.label(), caller.host().label());
	for outcome in Outcome::ALL {
		metrics::counter!(REQUESTS_METRIC, "caller" => caller_label, "host" => host_label, "outcome" => outcome.label()).increment(0);
	}
	metrics::counter!(SENT_METRIC, "caller" => caller_label, "host" => host_label).increment(0);
	metrics::counter!(RECEIVED_METRIC, "caller" => caller_label, "host" => host_label).increment(0);
}

/// One request, on every series.
fn record(caller: Caller, outcome: Outcome, took: Duration, sent: usize, received: u64) {
	let (caller_label, host_label) = (caller.label(), caller.host().label());
	metrics::counter!(REQUESTS_METRIC, "caller" => caller_label, "host" => host_label, "outcome" => outcome.label()).increment(1);
	metrics::histogram!(DURATION_METRIC, "caller" => caller_label, "host" => host_label).record(took.as_secs_f64());
	metrics::counter!(SENT_METRIC, "caller" => caller_label, "host" => host_label).increment(u64::try_from(sent).unwrap_or(u64::MAX));
	metrics::counter!(RECEIVED_METRIC, "caller" => caller_label, "host" => host_label).increment(received);
}

#[cfg(test)]
mod tests {
	use super::{Caller, Outcome};
	use std::collections::HashSet;

	#[test]
	fn every_label_is_distinct() {
		let callers: HashSet<_> = Caller::ALL.iter().map(|caller| caller.label()).collect();
		assert_eq!(callers.len(), Caller::ALL.len());
		let outcomes: HashSet<_> = Outcome::ALL.iter().map(|outcome| outcome.label()).collect();
		assert_eq!(outcomes.len(), Outcome::ALL.len());
	}

	#[test]
	fn a_status_is_ok_below_400_and_refused_by_class_above() {
		assert_eq!(Outcome::of_status(200), Outcome::Ok);
		assert_eq!(Outcome::of_status(304), Outcome::Ok);
		assert_eq!(Outcome::of_status(404), Outcome::Refused4xx);
		assert_eq!(Outcome::of_status(410), Outcome::Refused4xx);
		assert_eq!(Outcome::of_status(503), Outcome::Refused5xx);
	}
}
