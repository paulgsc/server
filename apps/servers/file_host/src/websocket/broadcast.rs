use crate::WebSocketFsm;
use some_transport::{NatsTransport, Transport};
use ws_events::{events::Event, UnifiedEvent};

mod errors;
mod handlers;

pub(crate) use errors::BroadcastError;
pub(crate) use handlers::spawn_event_forwarder;

impl WebSocketFsm {
	/// Broadcast an event to all subscribers of its type
	pub async fn broadcast_event(&self, transport: NatsTransport<UnifiedEvent>, event: Event) -> Result<(), BroadcastError> {
		let unified_event = UnifiedEvent::try_from(event.clone())?;
		let event_type = event.get_type().ok_or(BroadcastError::NoEventType)?;
		let subject = event_type.subject();

		transport.send_to_subject(subject, unified_event).await?;

		Ok(())
	}
}

/// A transport whose client never reached a server, so every send fails the
/// way it does while NATS is down. For tests of what a failed broadcast is
/// answered with; `retry_on_initial_connect` hands back the client without
/// waiting for the connection.
#[cfg(test)]
pub(crate) async fn disconnected_transport() -> NatsTransport<UnifiedEvent> {
	let client = async_nats::ConnectOptions::new().retry_on_initial_connect().connect("nats://127.0.0.1:1").await.unwrap();
	NatsTransport::new(client)
}
