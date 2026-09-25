//! Connection lifecycle and command dispatch over one `obws` client.

use crate::commands;
use crate::config::ObsConfig;
use crate::events::to_obs_event;
use crate::polling::{self, PollingConfig};
use crate::types::ObsEvent;
use crate::ObsCommand;
use futures_util::future::BoxFuture;
use futures_util::StreamExt;
use obws::client::ConnectConfig;
use obws::requests::EventSubscription;
use obws::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::task::JoinHandle;

/// How long [`ObsWebSocketManager::execute_command`] waits for OBS to answer.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// Buffered events per subscriber before the slowest one starts losing the oldest.
const EVENT_CAPACITY: usize = 256;

/// Every category a stream operator acts on, so each [`ObsCommand`]'s effect
/// comes back as an event: the ones [`to_obs_event`] maps, plus General,
/// Filters and media playback (forwarded as `UnknownEvent`). Leaves out OBS's
/// high-volume categories such as input meters.
const EVENT_SUBSCRIPTIONS: EventSubscription = EventSubscription::GENERAL
	.union(EventSubscription::SCENES)
	.union(EventSubscription::INPUTS)
	.union(EventSubscription::TRANSITIONS)
	.union(EventSubscription::FILTERS)
	.union(EventSubscription::OUTPUTS)
	.union(EventSubscription::SCENE_ITEMS)
	.union(EventSubscription::MEDIA_INPUTS)
	.union(EventSubscription::UI);

#[derive(Debug, Error)]
pub enum ObsWebsocketError {
	/// Includes OBS being older than `obws` supports (OBS Studio 30.2,
	/// obs-websocket 5.5), which it checks right after connecting.
	#[error("Failed to connect to OBS: {0}")]
	Connect(#[source] obws::error::Error),
	#[error(transparent)]
	Command(#[from] CommandError),
}

/// Why a command did not complete successfully.
#[derive(Debug, Clone, Error)]
pub enum CommandError {
	/// OBS received the request and refused it. `comment` is OBS's own reason,
	/// e.g. "The stream output is already running."
	#[error("OBS rejected {request_type} (code {code}): {comment}")]
	Rejected { request_type: String, code: u64, comment: String },

	#[error("Not connected to OBS")]
	NotConnected,

	#[error("OBS did not respond within {0:?}")]
	Timeout(Duration),

	#[error("Connection to OBS closed before it responded")]
	ConnectionClosed,

	#[error("{request_type} failed: {reason}")]
	Transport { request_type: String, reason: String },
}

impl CommandError {
	pub(crate) fn from_obws(request_type: &str, error: obws::error::Error) -> Self {
		match error {
			obws::error::Error::Api { code, message } => Self::Rejected {
				request_type: request_type.to_owned(),
				code: u16::from(code).into(),
				comment: message.unwrap_or_else(|| "no comment from OBS".to_owned()),
			},
			obws::error::Error::Disconnected | obws::error::Error::ReceiveMessage(_) => Self::ConnectionClosed,
			other => Self::Transport {
				request_type: request_type.to_owned(),
				reason: other.to_string(),
			},
		}
	}
}

#[derive(Debug, Clone)]
pub enum ConnectionState {
	Disconnected,
	Connecting,
	Connected { since: Instant },
	Failed { error: String },
}

/// One live connection: the client plus the tasks feeding events from it.
struct Connection {
	client: Arc<Client>,
	tasks: [JoinHandle<()>; 2],
}

/// Connects to OBS, publishes its events and status snapshots as [`ObsEvent`]s,
/// and runs [`ObsCommand`]s against it.
///
/// One manager holds at most one connection; [`Self::connect`] replaces any
/// existing one. After the socket drops, the state reads
/// [`ConnectionState::Disconnected`] and [`Self::stream_events`] returns, so a
/// caller can loop connect → stream → reconnect.
pub struct ObsWebSocketManager {
	config: ObsConfig,
	connection: Mutex<Option<Connection>>,
	state: Arc<watch::Sender<ConnectionState>>,
	events: broadcast::Sender<ObsEvent>,
}

impl ObsWebSocketManager {
	#[must_use]
	pub fn new(config: ObsConfig) -> Self {
		let (events, _) = broadcast::channel(EVENT_CAPACITY);
		Self {
			config,
			connection: Mutex::new(None),
			state: Arc::new(watch::Sender::new(ConnectionState::Disconnected)),
			events,
		}
	}

	/// Connects to OBS and starts publishing its events, plus the status
	/// snapshots `polling` asks for.
	///
	/// # Errors
	///
	/// [`ObsWebsocketError::Connect`] when OBS is unreachable, rejects the
	/// password, or is older than `obws` supports.
	pub async fn connect(&self, polling: PollingConfig) -> Result<(), ObsWebsocketError> {
		let mut slot = self.connection.lock().await;
		if let Some(previous) = slot.take() {
			close(previous).await;
		}

		self.state.send_replace(ConnectionState::Connecting);
		let client = match Client::connect_with_config(self.connect_config()).await {
			Ok(client) => Arc::new(client),
			Err(e) => {
				self.state.send_replace(ConnectionState::Failed { error: e.to_string() });
				return Err(ObsWebsocketError::Connect(e));
			}
		};
		let obs_events = match client.events() {
			Ok(stream) => stream,
			Err(e) => {
				self.state.send_replace(ConnectionState::Failed { error: e.to_string() });
				return Err(ObsWebsocketError::Connect(e));
			}
		};

		let events = self.events.clone();
		let state = Arc::clone(&self.state);
		let pump = tokio::spawn(async move {
			let mut obs_events = obs_events;
			while let Some(event) = obs_events.next().await {
				if let Some(event) = to_obs_event(event) {
					// No subscribers is not an error; nobody is listening yet.
					drop(events.send(event));
				}
			}
			// obws ends the stream when the socket closes.
			tracing::info!("OBS connection closed");
			state.send_replace(ConnectionState::Disconnected);
		});
		let poller = tokio::spawn(polling::run(Arc::clone(&client), polling, self.events.clone()));

		*slot = Some(Connection { client, tasks: [pump, poller] });
		drop(slot);
		self.state.send_replace(ConnectionState::Connected { since: Instant::now() });
		tracing::info!(host = %self.config.host, port = self.config.port, "connected to OBS");
		Ok(())
	}

	/// Closes the connection, if any.
	pub async fn disconnect(&self) {
		let previous = self.connection.lock().await.take();
		if let Some(previous) = previous {
			close(previous).await;
		}
		self.state.send_replace(ConnectionState::Disconnected);
	}

	#[must_use]
	pub fn connection_state(&self) -> ConnectionState {
		self.state.borrow().clone()
	}

	#[must_use]
	pub fn is_healthy(&self) -> bool {
		matches!(*self.state.borrow(), ConnectionState::Connected { .. })
	}

	/// Sends `command` and waits for OBS's response to it.
	///
	/// Returns what OBS's `responseData` holds for it (`None` for requests that
	/// return nothing). Success means OBS accepted the request, not that its
	/// effect finished: after `StartStream`, watch for a `StreamStateChanged`
	/// whose `output_state` is `OBS_WEBSOCKET_OUTPUT_STARTED`.
	///
	/// # Errors
	///
	/// [`CommandError::Rejected`] carries OBS's own code and reason when it
	/// refuses the request. Also fails when not connected, when the connection
	/// drops first, or after [`COMMAND_TIMEOUT`] with no response.
	pub async fn execute_command(&self, command: ObsCommand) -> Result<Option<Value>, ObsWebsocketError> {
		let client = self.client().await.ok_or(CommandError::NotConnected)?;

		match tokio::time::timeout(COMMAND_TIMEOUT, commands::execute(&client, command)).await {
			Ok(result) => Ok(result?),
			Err(_) => Err(CommandError::Timeout(COMMAND_TIMEOUT).into()),
		}
	}

	/// Subscribes to published events. Subscribe before sending a command whose
	/// outcome you want to observe, so its events can't slip past.
	#[must_use]
	pub fn events(&self) -> broadcast::Receiver<ObsEvent> {
		self.events.subscribe()
	}

	/// Calls `handler` for every published event until the connection ends.
	/// A handler slower than OBS's event rate loses the oldest events, logged.
	pub async fn stream_events<F>(&self, mut handler: F)
	where
		F: FnMut(ObsEvent) -> BoxFuture<'static, ()>,
	{
		let mut events = self.events.subscribe();
		let mut state = self.state.subscribe();

		while matches!(*state.borrow_and_update(), ConnectionState::Connected { .. }) {
			tokio::select! {
				event = events.recv() => match event {
					Ok(event) => handler(event).await,
					Err(RecvError::Lagged(skipped)) => tracing::warn!(skipped, "event handler fell behind; oldest events dropped"),
					Err(RecvError::Closed) => return,
				},
				changed = state.changed() => {
					if changed.is_err() {
						return;
					}
				}
			}
		}
	}

	async fn client(&self) -> Option<Arc<Client>> {
		self.connection.lock().await.as_ref().map(|c| Arc::clone(&c.client))
	}

	fn connect_config(&self) -> ConnectConfig<&str, &str> {
		ConnectConfig {
			host: self.config.host.as_str(),
			port: self.config.port,
			password: Some(self.config.password.as_str()).filter(|p| !p.is_empty()),
			event_subscriptions: Some(EVENT_SUBSCRIPTIONS),
			broadcast_capacity: EVENT_CAPACITY,
			connect_timeout: obws::client::DEFAULT_CONNECT_TIMEOUT,
			dangerous: None,
		}
	}
}

async fn close(connection: Connection) {
	for task in connection.tasks {
		task.abort();
		drop(task.await);
	}
	// The poller held the only other long-lived clone; an in-flight command may
	// still hold one, in which case dropping ours lets obws shut down on its own.
	if let Ok(mut client) = Arc::try_unwrap(connection.client) {
		client.disconnect().await;
	}
}
