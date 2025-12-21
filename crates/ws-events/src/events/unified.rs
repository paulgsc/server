use super::common::{Event, EventType, SystemEvent};
use prost::Message;

mod audio;
mod now_playing;
mod obs;
mod orchestrator;
mod system;
mod utterance;

pub use audio::{AudioChunkMessage, SubtitleMessage};
use now_playing::TabMetaDataMessage;
pub use obs::{ObsCommandMessage, ObsStatusMessage};
use orchestrator::{OrchestratorStateMessage, TickCommandMessage};
use system::{ClientCountMessage, ErrorMessage, SystemEventMessage};
use utterance::UtteranceMessage;

/// Unified event type for NATS transport (Prost-compatible)
/// Contains only events that should be transported via NATS
#[derive(Clone, Message)]
pub struct UnifiedEvent {
	#[prost(oneof = "unified_event::Event", tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11")]
	pub event: Option<unified_event::Event>,
}

pub mod unified_event {
	use super::*;

	#[derive(Clone, PartialEq, prost::Oneof)]
	pub enum Event {
		#[prost(message, tag = "1")]
		ObsStatus(ObsStatusMessage),
		#[prost(message, tag = "2")]
		ObsCommand(ObsCommandMessage),
		#[prost(message, tag = "3")]
		TabMetaData(TabMetaDataMessage),
		#[prost(message, tag = "4")]
		ClientCount(ClientCountMessage),
		#[prost(message, tag = "5")]
		Error(ErrorMessage),
		#[prost(message, tag = "6")]
		Utterance(UtteranceMessage),
		#[prost(message, tag = "7")]
		SystemEvent(SystemEventMessage),
		#[prost(message, tag = "8")]
		OrchestratorCommandData(TickCommandMessage),
		#[prost(message, tag = "9")]
		OrchestratorState(OrchestratorStateMessage),
		#[prost(message, tag = "10")]
		AudioChunk(AudioChunkMessage),
		#[prost(message, tag = "11")]
		Subtitle(SubtitleMessage),
	}
}

/// TryFrom<Event> for UnifiedEvent (fallible): transportable events -> Ok(UnifiedEvent), otherwise Err(String)
impl From<Event> for Option<UnifiedEvent> {
	fn from(event: Event) -> Self {
		match event {
			Event::ObsStatus { status } => ObsStatusMessage::new(status).ok().map(|msg| UnifiedEvent {
				event: Some(unified_event::Event::ObsStatus(msg)),
			}),
			Event::ObsCmd { cmd } => ObsCommandMessage::new(uuid::Uuid::new_v4().to_string(), cmd).ok().map(|msg| UnifiedEvent {
				event: Some(unified_event::Event::ObsCommand(msg)),
			}),
			Event::TabMetaData { data } => Some(UnifiedEvent {
				event: Some(unified_event::Event::TabMetaData(TabMetaDataMessage::from_now_playing(data))),
			}),
			Event::ClientCount { count } => Some(UnifiedEvent {
				event: Some(unified_event::Event::ClientCount(ClientCountMessage { count: count as u64 })),
			}),
			Event::Error { message } => Some(UnifiedEvent {
				event: Some(unified_event::Event::Error(ErrorMessage { message })),
			}),
			Event::Utterance { text, metadata } => UtteranceMessage::new(text, metadata).ok().map(|msg| UnifiedEvent {
				event: Some(unified_event::Event::Utterance(msg)),
			}),
			Event::System(sys_event) => serde_json::to_vec(&sys_event).ok().map(|payload| {
				let event_type = match &sys_event {
					SystemEvent::ConnectionStateChanged { .. } => "ConnectionStateChanged",
					SystemEvent::MessageProcessed { .. } => "MessageProcessed",
					SystemEvent::BroadcastFailed { .. } => "BroadcastFailed",
					SystemEvent::ConnectionCleanup { .. } => "ConnectionCleanup",
					SystemEvent::Other { name, .. } => name.as_str(),
				};

				UnifiedEvent {
					event: Some(unified_event::Event::SystemEvent(SystemEventMessage {
						event_type: event_type.to_string(),
						payload,
						timestamp: chrono::Utc::now().timestamp(),
					})),
				}
			}),
			Event::OrchestratorCommandData { stream_id, command } => TickCommandMessage::from_tick_command(stream_id, command).ok().map(|msg| UnifiedEvent {
				event: Some(unified_event::Event::OrchestratorCommandData(msg)),
			}),
			Event::OrchestratorState { stream_id, state } => OrchestratorStateMessage::from_orchestrator_state(stream_id, &state).ok().map(|msg| UnifiedEvent {
				event: Some(unified_event::Event::OrchestratorState(msg)),
			}),
			Event::AudioChunk { sample_rate, channels, samples } => Some(UnifiedEvent {
				event: Some(unified_event::Event::AudioChunk(AudioChunkMessage::new(sample_rate, channels, samples))),
			}),
			Event::Subtitle { text, timestamp, confidence } => Some(UnifiedEvent {
				event: Some(unified_event::Event::Subtitle(if let Some(conf) = confidence {
					SubtitleMessage::with_confidence(text, timestamp, conf)
				} else {
					SubtitleMessage::new(text, timestamp)
				})),
			}),

			// Non-transportable events -> error
			Event::Ping | Event::Pong | Event::Subscribe { .. } | Event::Unsubscribe { .. } => None,
		}
	}
}

/// TryFrom implementation for conversation with error handling
impl TryFrom<Event> for UnifiedEvent {
	type Error = String;

	fn try_from(event: Event) -> Result<Self, Self::Error> {
		let event_type = event.get_type().map(|et| et.to_string()).unwrap_or("SystemEvent".to_string());

		Option::<UnifiedEvent>::from(event).ok_or_else(|| format!("Event type '{}' cannot be converted to UnifiedEvent (should not be sent to nats)", event_type))
	}
}

/// Convert UnifiedEvent to Result<Event, String>
impl From<UnifiedEvent> for Result<Event, String> {
	fn from(unified: UnifiedEvent) -> Self {
		match unified.event {
			Some(unified_event::Event::ObsStatus(msg)) => msg.to_obs_event().map_err(|e| e.to_string()).map(|status| Event::ObsStatus { status }),
			Some(unified_event::Event::ObsCommand(msg)) => msg.to_obs_command().map_err(|e| e.to_string()).map(|cmd| Event::ObsCmd { cmd }),
			Some(unified_event::Event::TabMetaData(msg)) => Ok(Event::TabMetaData { data: msg.to_now_playing() }),
			Some(unified_event::Event::ClientCount(msg)) => Ok(Event::ClientCount { count: msg.count as usize }),
			Some(unified_event::Event::Error(msg)) => Ok(Event::Error { message: msg.message }),
			Some(unified_event::Event::Utterance(msg)) => msg
				.get_metadata()
				.map_err(|e| e.to_string())
				.map(|metadata| Event::Utterance { text: msg.text.clone(), metadata }),
			Some(unified_event::Event::SystemEvent(msg)) => serde_json::from_slice::<SystemEvent>(&msg.payload)
				.map(Event::System)
				.map_err(|e| format!("Failed to deserialize SystemEvent: {}", e)),
			Some(unified_event::Event::OrchestratorCommandData(msg)) => msg.to_tick_command().map(|(stream_id, command)| Event::OrchestratorCommandData { stream_id, command }),
			Some(unified_event::Event::OrchestratorState(msg)) => msg.to_orchestrator_state().map(|(stream_id, state)| Event::OrchestratorState { stream_id, state }),
			Some(unified_event::Event::AudioChunk(msg)) => {
				// Decode bytes back to f32 samples
				let samples = msg.decode_samples().map_err(|e| format!("Failed to decode audio samples: {}", e))?;

				Ok(Event::AudioChunk {
					sample_rate: msg.sample_rate.unwrap_or(48000), // Default to 48kHz if not present
					channels: msg.channels.unwrap_or(2),           // Default to stereo if not present
					samples,
				})
			}
			Some(unified_event::Event::Subtitle(msg)) => Ok(Event::Subtitle {
				text: msg.text,
				timestamp: msg.timestamp,
				confidence: msg.confidence,
			}),

			None => Err("UnifiedEvent has no event variant".to_string()),
		}
	}
}

impl UnifiedEvent {
	/// Get the EventType for this unified event
	pub fn event_type(&self) -> Option<EventType> {
		match &self.event {
			Some(unified_event::Event::ObsStatus(_)) => Some(EventType::ObsStatus),
			Some(unified_event::Event::ObsCommand(_)) => Some(EventType::ObsCommand),
			Some(unified_event::Event::TabMetaData(_)) => Some(EventType::TabMetaData),
			Some(unified_event::Event::ClientCount(_)) => Some(EventType::ClientCount),
			Some(unified_event::Event::Error(_)) => Some(EventType::Error),
			Some(unified_event::Event::Utterance(_)) => Some(EventType::Utterance),
			Some(unified_event::Event::SystemEvent(_)) => Some(EventType::SystemEvent),
			Some(unified_event::Event::OrchestratorCommandData(_)) => Some(EventType::OrchestratorCommandData),
			Some(unified_event::Event::OrchestratorState(_)) => Some(EventType::OrchestratorState),
			Some(unified_event::Event::AudioChunk(_)) => Some(EventType::AudioChunk),
			Some(unified_event::Event::Subtitle(_)) => Some(EventType::Subtitle),
			None => None,
		}
	}

	/// Get the NATS subject for this event
	pub fn subject(&self) -> Option<String> {
		self.event_type().map(|et| et.subject().to_string())
	}

	/// Get the connection-specific subject for this event
	pub fn connection_subject(&self, connection_id: &str) -> Option<String> {
		self.event_type().map(|et| et.connection_subject(connection_id))
	}
}
