use crate::WebSocketFsm;
use some_transport::{NatsTransport, Transport};
use ws_events::{events::Event, UnifiedEvent};

mod errors;
mod handlers;
mod types;

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
