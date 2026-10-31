//! `GET /ws/game-week`: the paired phone's live game day.
//!
//! Game Week Bites, paulgsc/some-ui#2074, story GWB4. The recorder shows what can be
//! happening now in the game the owner is on; this socket is how home's
//! offers reach it while home has them, and how the phone learns home's
//! clock, so each bite keeps the offset between the two (GW5).
//!
//! **Which layer carries it.** `file_host`'s own WebSocket module, on the
//! same axum upgrade and the same [`ws_conn_manager::ConnectionGuard`]
//! admission as `/ws` ([`super::admit`]). Not `/ws`'s event bus: that bus is
//! unauthenticated and broadcasts each event type to every subscriber over
//! NATS, while an offer is made per connection, from the mode and watching
//! flags that phone sent, and nothing else in the system publishes one. So
//! there is no actor, no NATS subject and no spawned task: one loop per
//! socket, ended with it.
//!
//! **Push and pull** (the owner's choice, 2026-10-25).
//!
//! - *Pull:* the phone opens with a `hello` (the last `seq` it saw, its mode,
//!   the game it is on and its watching flags) and home answers with what it
//!   has now, every time. So a reconnect lands on the current state, and the
//!   phone shows no offer as live until home has answered.
//! - *Push:* the game feed tells [`GameLive::fed`] after each pass that kept
//!   something, and every socket makes its offer again at once and sends it
//!   if the phone would see anything new. A [`TICK`] does the same for what
//!   moves with the clock alone (the kickoff countdown, a play leaving "just
//!   happened", pregame opening).
//! - *Clock:* each `ping` carries the phone's clock and is answered with
//!   home's, so the phone keeps the offset with each bite.
//!
//! Every offer's `seq` is larger than any before it on this home, across
//! sockets and restarts: [`GameLive`] starts it at the boot time in
//! milliseconds, and never below the phone's own `lastSeq`.
//!
//! **Who may open it** (docs/identity.md, "Pairing a phone"): a paired
//! phone's session only ([`crate::subject::Paired`]); a browser's is `403`
//! and none `401`. A page cannot set `Authorization` on a WebSocket, so the
//! phone's app offers its token as a subprotocol, `bearer.<token>`, beside
//! [`PROTOCOL`]; home reads it as that header ([`LivePhone`]) and answers
//! [`PROTOCOL`] alone, never the token. The session is looked up again every
//! [`RECHECK`], so unpairing closes an open socket within a minute.
//!
//! **What it holds of the person: nothing.** The hello's mode, game,
//! watching flags and UTC offset decide one offer and are dropped with the
//! socket; none is logged or stored. Its log lines say a socket opened and
//! why it closed. What it sends is ESPN's public game data, read through the
//! engine. LAN only, as the rest of home is.

pub mod offer;
pub mod wire;

use crate::{
	auth::AuthContext,
	net::{AdmissionKey, Peer},
	subject::Paired,
	AppState, FileHostError,
};
use axum::{
	extract::{
		ws::{CloseFrame, Message, WebSocketUpgrade},
		FromRef, FromRequestParts, State,
	},
	http::{
		header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL},
		request::Parts,
		HeaderMap, HeaderValue,
	},
	response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use futures::{Sink, SinkExt, Stream, StreamExt};
use sqlx::SqlitePool;
use std::sync::{
	atomic::{AtomicU64, Ordering},
	Arc,
};
use tokio::{sync::watch, time::Duration};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use wire::{Close, Hello, Inbound, Outbound, State as Offered};
use ws_conn_manager::ConnectionGuard;

/// The subprotocol home answers: version 1 of [`wire`].
pub const PROTOCOL: &str = "game-week.v1";

/// What precedes the phone's token among the subprotocols it offers.
pub const BEARER_PROTOCOL: &str = "bearer.";

/// How often each socket makes its offer again with nothing fed: what moves
/// with the clock alone moves by the minute, and a play leaves "just
/// happened" a minute or more after home saw it.
pub const TICK: Duration = Duration::from_secs(5);

/// How often an open socket's session is looked up again.
pub const RECHECK: Duration = Duration::from_secs(60);

/// How long a socket may hear nothing from the phone, which pings every
/// 15 s: `/ws`'s own stale window.
pub const QUIET: Duration = super::STALE_TIMEOUT;

/// The largest frame the phone may send: a hello is a few hundred bytes.
const MAX_MESSAGE: usize = 16 * 1024;

/// The game day's shared half: who to tell when the feed kept something,
/// and the next `seq`. Cheap to clone.
#[derive(Clone)]
pub struct GameLive {
	inner: Arc<Inner>,
}

struct Inner {
	fed: watch::Sender<u64>,
	seq: AtomicU64,
	following: bool,
	guard: ConnectionGuard,
	cancel: CancellationToken,
	clock: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl GameLive {
	/// `following` is whether the game feed runs (`GAME_FEED_ENABLED`): with
	/// it off, every offer says so and lists no games.
	#[must_use]
	pub fn new(following: bool, guard: ConnectionGuard, cancel: CancellationToken) -> Self {
		Self::with_clock(following, guard, cancel, Box::new(Utc::now))
	}

	fn with_clock(following: bool, guard: ConnectionGuard, cancel: CancellationToken, clock: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>) -> Self {
		let boot = u64::try_from(clock().timestamp_millis()).unwrap_or(0);
		Self {
			inner: Arc::new(Inner {
				fed: watch::Sender::new(0),
				seq: AtomicU64::new(boot),
				following,
				guard,
				cancel,
				clock,
			}),
		}
	}

	/// The game feed kept something: every open socket makes its offer again
	/// now.
	pub fn fed(&self) {
		self.inner.fed.send_modify(|count| *count = count.wrapping_add(1));
	}

	/// The next `seq`: larger than every one before it, and than `seen`.
	fn next_seq(&self, seen: Option<u64>) -> u64 {
		let floor = seen.unwrap_or(0);
		let previous = self
			.inner
			.seq
			.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |seq| Some(seq.max(floor).saturating_add(1)))
			.unwrap_or_else(|seq| seq);
		previous.max(floor).saturating_add(1)
	}

	fn now(&self) -> DateTime<Utc> {
		(self.inner.clock)()
	}
}

/// A paired phone opening the game day socket, its token read from the
/// `bearer.<token>` subprotocol when it sent no `Authorization`.
///
/// It keeps no `SubjectId`: one holds off account deletion while it lives,
/// and a socket lives for hours. It keeps the phone's `Authorization` (a
/// sensitive header value, never logged) to look the session up again
/// every [`RECHECK`].
pub struct LivePhone {
	session: Session,
}

/// How an open socket asks whether its phone is still paired.
#[derive(Clone)]
pub struct Session {
	auth: AuthContext,
	headers: HeaderMap,
}

impl Session {
	/// Whether the phone's session is still live. A storage failure is not
	/// an unpairing: the next look decides.
	async fn ended(&self) -> bool {
		matches!(self.auth.session_subject(&self.headers).await, Ok(None))
	}
}

/// The `Authorization` the phone's offered `bearer.<token>` subprotocol
/// stands for, marked sensitive.
fn offered_bearer(headers: &HeaderMap) -> Option<HeaderValue> {
	let token = headers
		.get_all(SEC_WEBSOCKET_PROTOCOL)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(','))
		.find_map(|protocol| protocol.trim().strip_prefix(BEARER_PROTOCOL))?;
	let mut value = HeaderValue::from_str(&(String::from("Bearer ") + token)).ok()?;
	value.set_sensitive(true);
	Some(value)
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for LivePhone
where
	AuthContext: FromRef<S>,
	S: Send + Sync,
{
	type Rejection = FileHostError;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		if !parts.headers.contains_key(AUTHORIZATION) {
			if let Some(bearer) = offered_bearer(&parts.headers) {
				parts.headers.insert(AUTHORIZATION, bearer);
			}
		}
		// Released at once: see the type's doc.
		let Paired { subject } = Paired::from_request_parts(parts, state).await?;
		drop(subject);
		let mut headers = HeaderMap::new();
		if let Some(bearer) = parts.headers.get(AUTHORIZATION) {
			headers.insert(AUTHORIZATION, bearer.clone());
		}
		Ok(Self {
			session: Session {
				auth: AuthContext::from_ref(state),
				headers,
			},
		})
	}
}

/// The route's handler. The phone is checked first, so a request with no
/// session is `401` whether or not it asks to upgrade.
pub async fn upgrade(phone: LivePhone, Peer { admission, .. }: Peer, State(app): State<AppState>, ws: WebSocketUpgrade) -> Response {
	connect(ws, app.realtime.game_live.clone(), app.core.shared_db.clone(), phone, admission).await
}

/// Admit the socket under the shared guard, then serve it.
async fn connect(ws: WebSocketUpgrade, live: GameLive, db: SqlitePool, phone: LivePhone, admission: AdmissionKey) -> Response {
	let permit = match super::admit(&live.inner.guard, admission).await {
		Ok(permit) => permit,
		Err(refusal) => {
			error!("refusing a game day socket: {}", refusal.reason);
			return refusal.into_response();
		}
	};
	ws.protocols([PROTOCOL]).max_message_size(MAX_MESSAGE).on_upgrade(move |socket| async move {
		info!("game day socket opened");
		let (outgoing, incoming) = socket.split();
		let ended = serve(incoming, outgoing, live, db, phone.session).await;
		info!(reason = ended, "game day socket closed");
		permit.release();
	})
}

/// One socket, from its first frame to its close. Returns why it ended, in a
/// word for the log.
async fn serve<I, O>(mut incoming: I, mut outgoing: O, live: GameLive, db: SqlitePool, session: Session) -> &'static str
where
	I: Stream<Item = Result<Message, axum::Error>> + Unpin,
	O: Sink<Message> + Unpin,
{
	let mut fed = live.inner.fed.subscribe();
	let mut hello: Option<Hello> = None;
	let mut last: Option<Offered> = None;
	let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
	let mut recheck = tokio::time::interval_at(tokio::time::Instant::now() + RECHECK, RECHECK);
	let quiet = tokio::time::sleep(QUIET);
	tokio::pin!(quiet);

	let close = loop {
		// What to send, and whether to send it even with nothing new.
		let mut make = None;
		tokio::select! {
			frame = incoming.next() => {
				quiet.as_mut().reset(tokio::time::Instant::now() + QUIET);
				match frame {
					None | Some(Err(_) | Ok(Message::Close(_))) => return "the phone left",
					Some(Ok(Message::Text(text))) => match wire::read(&text) {
						Ok(Inbound::Hello(said)) => {
							hello = Some(said);
							make = Some(true);
						}
						Ok(Inbound::Ping { phone_at }) => {
							let home_at = u64::try_from(live.now().timestamp_millis()).unwrap_or(0);
							if send(&mut outgoing, &Outbound::Pong { phone_at, home_at }).await.is_err() {
								return "the phone left";
							}
						}
						Err(close) => break close,
					},
					Some(Ok(Message::Binary(_))) => break Close::Unreadable,
					Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
				}
			}
			changed = fed.changed() => {
				if changed.is_ok() && hello.is_some() {
					make = Some(false);
				}
			}
			_ = tick.tick(), if hello.is_some() => make = Some(false),
			_ = recheck.tick() => {
				if session.ended().await {
					break Close::Unpaired;
				}
			}
			() = &mut quiet => break Close::Quiet,
			() = live.inner.cancel.cancelled() => break Close::GoingAway,
		}
		let (Some(force), Some(said)) = (make, hello.as_ref()) else { continue };
		let Ok(state) = offer::state(&db, said, live.now(), live.inner.following).await else {
			error!(error = "storage", "game day offer failed");
			continue;
		};
		if !force && last.as_ref().is_some_and(|sent| sent.same_as(&state)) {
			continue;
		}
		let seq = live.next_seq(said.last_seq);
		if send(&mut outgoing, &Outbound::Offer { seq, state: state.clone() }).await.is_err() {
			return "the phone left";
		}
		last = Some(state);
	};
	let frame = CloseFrame {
		code: close.code(),
		reason: close.reason().into(),
	};
	let _ = outgoing.send(Message::Close(Some(frame))).await;
	close.reason()
}

async fn send<O: Sink<Message> + Unpin>(outgoing: &mut O, message: &Outbound) -> Result<(), O::Error> {
	outgoing.send(Message::Text(message.to_text())).await
}

#[cfg(test)]
mod tests;
