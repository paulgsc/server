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
	pub fn spawn_command_handler(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			tracing::info!("🎮 Starting command handler");
			let command_subject = EventType::ObsCommand.subject();
			let mut command_rx = self.transport.subscribe_to_subject(command_subject).await;

			loop {
				tokio::select! {
					_ = self.cancel_token.cancelled() => {
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
							Err(e) => match e {
								TransportError::Closed => break,
								_ => continue,
							}
						}
					}
				}
			}

			tracing::info!("✅ Command handler stopped");
		})
	}

	/// Handle a command received from NATS
	async fn handle_command(&self, cmd_msg: ObsCommandMessage) -> Result<(), Error> {
		// Deserialize the command from the protobuf message
		let obs_command = cmd_msg.to_obs_command()?;

		match self.obs_manager.execute_command(obs_command).await {
			Ok(()) => {
				// Send acknowledgment if reply_to is specified
				if let Some(reply_subject) = cmd_msg.reply_to {
					let ack = UnifiedEvent::try_from(Event::ObsStatus {
						status: ObsEvent::UnknownEvent(UnknownEventData {
							event_type: "command_ack".to_string(),
							data: serde_json::json!({
								"request_id": cmd_msg.request_id,
								"status": "success"
							}),
						}),
					})?;

					if let Err(e) = self.transport.send_to_subject(&reply_subject, ack).await {
						tracing::warn!("⚠️ Failed to send acknowledgment: {}", e);
					}
				}
				Ok(())
			}
			Err(e) => {
				tracing::error!("❌ Command failed: {} - {}", cmd_msg.request_id, e);

				// Send error response if reply_to is specified
				if let Some(reply_subject) = cmd_msg.reply_to {
					let error_msg = UnifiedEvent::try_from(Event::ObsStatus {
						status: ObsEvent::UnknownEvent(UnknownEventData {
							event_type: "command_error".to_string(),
							data: serde_json::json!({
								"request_id": cmd_msg.request_id,
								"status": "error",
								"error": e.to_string()
							}),
						}),
					})?;

					if let Err(send_err) = self.transport.send_to_subject(&reply_subject, error_msg).await {
						tracing::warn!("⚠️ Failed to send error response: {}", send_err);
					}
				}
				Err(e.into())
			}
		}
	}
}
