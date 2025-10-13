use async_nats::{Client, ConnectOptions, Subscriber};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct NatsTransport<K: EventKey> {
	client: Client,
	config: NatsConfig,
	_phantom: std::marker::PhantomData<K>,
}

impl<K: EventKey> NatsTransport<K> {
	pub async fn connect(config: NatsConfig) -> Result<Self> {
		let mut opts = ConnectOptions::new();

		if let Some(name) = &config.name {
			opts = opts.name(name);
		}

		if let Some(token) = &config.token {
			opts = opts.token(token);
		} else if let (Some(user), Some(pass)) = (&config.username, &config.password) {
			opts = opts.user_and_password(user.to_string(), pass.to_string());
		}

		let client = opts.connect(&config.servers[0]).await?;
		info!("Connected to NATS at {}", config.servers[0]);

		Ok(Self {
			client,
			config,
			_phantom: std::marker::PhantomData,
		})
	}

	pub async fn publish(&self, event: NatsEvent<K>) -> Result<()> {
		let subject = self.event_subject(&event.event_type);
		let payload = serde_json::to_vec(&event)?;

		self.client.publish(subject.clone(), payload.into()).await?;
		info!("Published event to subject: {}", subject);
		Ok(())
	}

	pub async fn subscribe(&self, event_types: Vec<K>) -> Result<NatsSubscriber<K>> {
		let subjects: Vec<String> = event_types.iter().map(|et| self.event_subject(et)).collect();

		let mut subscribers = Vec::new();

		for subject in subjects {
			let sub = self.client.subscribe(subject.clone()).await.map_err(|e| NatsError::ConnectionError(e))?;
			info!("Subscribed to subject: {}", subject);
			subscribers.push(sub);
		}

		Ok(NatsSubscriber {
			subscribers,
			_phantom: std::marker::PhantomData,
		})
	}

	pub async fn subscribe_all(&self) -> Result<NatsSubscriber<K>> {
		let subject = format!("{}.*", self.config.subject_prefix);
		let sub = self.client.subscribe(subject.clone()).await.map_err(|e| NatsError::ConnectionError(e))?;

		info!("Subscribed to all events: {}", subject);

		Ok(NatsSubscriber {
			subscribers: vec![sub],
			_phantom: std::marker::PhantomData,
		})
	}

	pub async fn event_stream(&self, event_types: Vec<K>, buffer_size: usize) -> Result<mpsc::Receiver<NatsEvent<K>>> {
		let mut subscriber = self.subscribe(event_types).await?;
		let (tx, rx) = mpsc::channel(buffer_size);

		tokio::spawn(async move {
			while let Some(event) = subscriber.next().await {
				if tx.send(event).await.is_err() {
					warn!("Event stream receiver dropped");
					break;
				}
			}
		});

		Ok(rx)
	}

	fn event_subject(&self, event_type: &K) -> String {
		format!("{}.{}", self.config.subject_prefix, event_type.to_subject())
	}

	pub fn client(&self) -> &Client {
		&self.client
	}
}
