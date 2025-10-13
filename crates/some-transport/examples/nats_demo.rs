//! Examples showcasing how to use nats-transport

use nats_transport::{EventKey, JetStreamTransport, NatsConfig, NatsEvent, NatsTransport};
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Duration};
use tracing_subscriber::FmtSubscriber;

/// Define your event key type (domain-specific)
#[derive(Debug, Clone, Serialize, Deserialize)]
enum MyEventKey {
	UserCreated,
	UserDeleted,
}

impl EventKey for MyEventKey {
	fn to_subject(&self) -> String {
		match self {
			MyEventKey::UserCreated => "user.created".into(),
			MyEventKey::UserDeleted => "user.deleted".into(),
		}
	}

	fn from_subject(subject: &str) -> Option<Self> {
		match subject {
			"user.created" => Some(MyEventKey::UserCreated),
			"user.deleted" => Some(MyEventKey::UserDeleted),
			_ => None,
		}
	}
}

/// Basic NATS usage
pub async fn basic_example() -> nats_transport::Result<()> {
	// Logging
	let _ = FmtSubscriber::builder().with_max_level(tracing::Level::INFO).try_init();

	let config = NatsConfig::default();
	let transport = NatsTransport::<MyEventKey>::connect(config).await?;

	// Subscribe to "user.created"
	let mut subscriber = transport.subscribe(vec![MyEventKey::UserCreated]).await?;

	// Publisher task
	let publisher = {
		let transport = transport.clone();
		tokio::spawn(async move {
			let event = NatsEvent::new(MyEventKey::UserCreated, serde_json::json!({ "id": 123, "name": "Alice" }));
			transport.publish(event).await.unwrap();
		})
	};

	// Listener task
	let listener = tokio::spawn(async move {
		if let Some(event) = subscriber.next().await {
			println!("Received event: {:?}, payload = {}", event.event_type, event.payload);
		}
	});

	publisher.await.unwrap();
	listener.await.unwrap();

	Ok(())
}

/// Entrypoint if run as standalone (not as `cargo test`)
#[tokio::main]
async fn main() -> nats_transport::Result<()> {
	// Run basic example
	println!("--- Running basic example ---");
	basic_example().await?;

	Ok(())
}
