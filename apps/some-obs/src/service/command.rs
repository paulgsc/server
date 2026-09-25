use crate::{Error, ObsNatsService};
use obs_websocket::{ObsEvent, UnknownEventData};
use some_transport::{Transport, TransportError};
use std::sync::Arc;
use ws_events::{
	events::{Event, EventType, ObsCommandMessage},
	unified_event, UnifiedEvent,
};

impl ObsNatsService {
	/// Spawn task to handle incoming commands from NATS
	///
	/// # Panics
	///
	/// The spawned task panics if a non-command event arrives on the OBS command
	/// subject, which is a protocol violation.
	#[must_use]
	pub fn spawn_command_handler(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			tracing::info!("🎮 Starting command handler");
			let command_subject = EventType::ObsCommand.subject();
			let mut command_rx = self.transport.subscribe_to_subject(command_subject).await;

			loop {
				tokio::select! {
					() = self.cancel_token.cancelled() => {
						tracing::info!("🛑 Command handler shutting down");
						break;
					}
					result = command_rx.recv() => {
						match result {
							Ok(unified) => {
								match unified.event {
									Some(unified_event::Event::ObsCommand(cmd_msg)) => {
										if let Err(e) = self.handle_command(cmd_msg).await {
											tracing::error!("❌ Failed to handle command: {}", e);
										}
									}
									other => {
										tracing::error!(
											"🚨 FATAL: Unexpected event type on OBS command subject: {:?}",
											other
										);
										panic!("Invalid event delivered to OBS command subject — protocol violation");
									}
								}
							},
							Err(TransportError::Closed) => break,
							Err(_) => {}
						}
					}
				}
			}

			tracing::info!("✅ Command handler stopped");
		})
	}

	/// Handle a command received from NATS, replying on `reply_to` (when set)
	/// with OBS's response data or the reason it failed. A command that doesn't
	/// parse gets an error reply too, so a caller generating commands (e.g. from
	/// speech) learns what was wrong instead of hearing nothing.
	async fn handle_command(&self, cmd_msg: ObsCommandMessage) -> Result<(), Error> {
		let result = match cmd_msg.to_obs_command() {
			Ok(obs_command) => self.obs_manager.execute_command(obs_command).await.map_err(Error::from),
			Err(e) => Err(Error::from(e)),
		};

		let reply = match &result {
			Ok(response_data) => serde_json::json!({
				"request_id": cmd_msg.request_id,
				"status": "success",
				"data": response_data
			}),
			Err(e) => {
				tracing::error!("❌ Command failed: {} - {}", cmd_msg.request_id, e);
				serde_json::json!({
					"request_id": cmd_msg.request_id,
					"status": "error",
					"error": e.to_string()
				})
			}
		};

		if let Some(reply_subject) = cmd_msg.reply_to {
			let event_type = if result.is_ok() { "command_ack" } else { "command_error" };
			let reply = UnifiedEvent::try_from(Event::ObsStatus {
				status: ObsEvent::UnknownEvent(UnknownEventData {
					event_type: event_type.to_string(),
					data: reply,
				}),
			})?;

			if let Err(e) = self.transport.send_to_subject(&reply_subject, reply).await {
				tracing::warn!("⚠️ Failed to send command reply: {}", e);
			}
		}

		result.map(|_| ())
	}
}
