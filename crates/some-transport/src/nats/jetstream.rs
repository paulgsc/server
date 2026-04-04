#![cfg(feature = "nats")]

use super::pool::NatsConnectionPool;
use crate::error::{Result, TransportError};
use async_nats::jetstream::{
	self,
	consumer::{pull::Config as PullConfig, AckPolicy, DeliverPolicy},
	stream::Config as StreamConfig,
	AckKind, Context as JsContext, Message,
};
use async_nats::Client;
use prost::Message as ProstMessage;
use std::sync::Arc;
use std::time::Duration;

// ── Subject constants ──────────────────────────────────────────────────────

pub struct PipelineSubjects;

impl PipelineSubjects {
	pub const JOBS: &'static str = "pipeline.jobs";
	pub const DLQ: &'static str = "pipeline.dlq";
}

// ── Config ─────────────────────────────────────────────────────────────────

/// Invariant: `ack_wait` must exceed the worst-case duration of a single
/// pipeline stage. If a stage may exceed `ack_wait`, the caller must drive
/// `AckHandle::heartbeat` periodically to extend the lease.
#[derive(Debug, Clone)]
pub struct JetStreamConfig {
	pub consumer_name: String,
	pub ack_wait: Duration,
	/// Server terminates after this many deliveries; routes remainder to DLQ.
	pub max_deliver: i64,
	pub fetch_batch: usize,
}

impl Default for JetStreamConfig {
	fn default() -> Self {
		Self {
			consumer_name: "pipeline-worker".into(),
			ack_wait: Duration::from_secs(600),
			max_deliver: 5,
			fetch_batch: 1,
		}
	}
}

// ── AckHandle ──────────────────────────────────────────────────────────────

/// Invariant: exactly one of `ack`, `nak`, `term` must be called per handle.
/// For stages that may exceed `ack_wait`, call `heartbeat` at intervals
/// shorter than `ack_wait` to prevent server-side lease expiration and
/// concurrent redelivery.
///
/// Failure: dropping without settling is treated as NAK by the server
/// after `ack_wait` expires.
pub struct AckHandle {
	msg: Message,
}

impl AckHandle {
	/// Extend the server-side lease without consuming the message.
	///
	/// Precondition: called at intervals < `ack_wait` for long-running stages.
	/// Postcondition: server resets its redelivery timer.
	pub async fn heartbeat(&self) -> Result<()> {
		self.msg.ack_with(AckKind::Progress).await.map_err(|e| TransportError::NatsError(e.to_string()))
	}

	pub async fn ack(self) -> Result<()> {
		self.msg.ack().await.map_err(|e| TransportError::NatsError(e.to_string()))
	}

	pub async fn nak(self) -> Result<()> {
		self.msg.ack_with(AckKind::Nak(None)).await.map_err(|e| TransportError::NatsError(e.to_string()))
	}

	pub async fn term(self) -> Result<()> {
		self.msg.ack_with(AckKind::Term).await.map_err(|e| TransportError::NatsError(e.to_string()))
	}
}

// ── DurableConsumer<T> ─────────────────────────────────────────────────────

/// Pull-based JetStream consumer generic over `T: prost::Message + Default`.
///
/// Invariant: `AckHandle` returned by `next_msg` must be settled before
/// the next call to `next_msg`. Enforced structurally by the daemon loop.
pub struct DurableConsumer<T>
where
	T: ProstMessage + Default,
{
	consumer: jetstream::consumer::Consumer<PullConfig>,
	config: JetStreamConfig,
	_marker: std::marker::PhantomData<T>,
}

impl<T> DurableConsumer<T>
where
	T: ProstMessage + Default,
{
	/// Bind to the durable consumer using a shared `Arc<Client>` from the pool.
	///
	/// Postcondition: stream `pipeline` exists with WorkQueue retention.
	pub async fn bind(client: Arc<Client>, config: JetStreamConfig) -> Result<Self> {
		let js = jetstream::new((*client).clone());

		let stream = js
			.get_or_create_stream(StreamConfig {
				name: "pipeline".into(),
				subjects: vec![PipelineSubjects::JOBS.into(), PipelineSubjects::DLQ.into()],
				retention: jetstream::stream::RetentionPolicy::WorkQueue,
				max_age: Duration::from_secs(86_400),
				max_bytes: 512 * 1024 * 1024,
				max_message_size: 8 * 1024 * 1024,
				num_replicas: 1,
				..Default::default()
			})
			.await
			.map_err(|e| TransportError::NatsError(e.to_string()))?;

		let consumer = stream
			.get_or_create_consumer(
				"pipeline-consumer",
				PullConfig {
					durable_name: Some(config.consumer_name.clone()),
					ack_policy: AckPolicy::Explicit,
					ack_wait: config.ack_wait,
					max_deliver: config.max_deliver,
					deliver_policy: DeliverPolicy::All,
					filter_subject: PipelineSubjects::JOBS.into(),
					..Default::default()
				},
			)
			.await
			.map_err(|e| TransportError::NatsError(e.to_string()))?;

		Ok(Self {
			consumer,
			config,
			_marker: std::marker::PhantomData,
		})
	}

	/// Resolves the client from `NatsConnectionPool::global()`.
	pub async fn connect(nats_url: &str, config: JetStreamConfig) -> Result<Self> {
		let client = NatsConnectionPool::global().get_or_connect(nats_url).await?;
		Self::bind(client, config).await
	}

	/// Fetch the next message from the stream.
	///
	/// Returns `None` on clean timeout (no messages within `deadline`).
	///
	/// Failure: payload exceeding `max_payload_bytes` is immediately terminated
	/// and routed to DLQ; returns `Err`.
	///
	/// Note: for stages whose duration may exceed `JetStreamConfig::ack_wait`,
	/// drive `AckHandle::heartbeat` at sub-`ack_wait` intervals. Without it,
	/// the server will redeliver concurrently before `max_deliver` is reached.
	pub async fn next_msg(&mut self, deadline: Duration, max_payload_bytes: usize) -> Result<Option<(T, AckHandle)>> {
		use futures::StreamExt;

		let mut batch = self
			.consumer
			.fetch()
			.max_messages(self.config.fetch_batch)
			.expires(deadline)
			.messages()
			.await
			.map_err(|e| TransportError::NatsError(e.to_string()))?;

		let msg = match batch.next().await {
			Some(Ok(m)) => m,
			Some(Err(e)) => return Err(TransportError::NatsError(e.to_string())),
			None => return Ok(None),
		};

		if msg.payload.len() > max_payload_bytes {
			msg.ack_with(AckKind::Term).await.map_err(|e| TransportError::NatsError(e.to_string()))?;
			return Err(TransportError::NatsError(format!("payload too large: {} bytes", msg.payload.len())));
		}

		let value = T::decode(&msg.payload[..]).map_err(|e| TransportError::DeserializationError(e.to_string()))?;

		Ok(Some((value, AckHandle { msg })))
	}
}

// ── JetStreamPublisher<T> ──────────────────────────────────────────────────

/// Publishes prost-encoded `T` to the pipeline stream.
///
/// Shares the `Arc<Client>` from `NatsConnectionPool`; no second TCP
/// connection is opened.
pub struct JetStreamPublisher<T>
where
	T: ProstMessage,
{
	js: JsContext,
	_marker: std::marker::PhantomData<T>,
}

impl<T> JetStreamPublisher<T>
where
	T: ProstMessage,
{
	pub fn from_client(client: Arc<Client>) -> Self {
		let js = jetstream::new((*client).clone());
		Self {
			js,
			_marker: std::marker::PhantomData,
		}
	}

	pub async fn connect(nats_url: &str) -> Result<Self> {
		let client = NatsConnectionPool::global().get_or_connect(nats_url).await?;
		Ok(Self::from_client(client))
	}

	pub async fn publish(&self, msg: &T) -> Result<()> {
		let mut buf = Vec::new();
		msg.encode(&mut buf).map_err(|e| TransportError::SerializationError(e.to_string()))?;

		self
			.js
			.publish(PipelineSubjects::JOBS, buf.into())
			.await
			.map_err(|e| TransportError::NatsError(e.to_string()))?
			.await
			.map_err(|e| TransportError::NatsError(e.to_string()))?;

		Ok(())
	}

	/// Publish raw bytes to the DLQ subject.
	///
	/// Note: structured DLQ envelopes require a caller-defined proto type.
	/// Failure here must not block the main pipeline; log and discard at
	/// the call site.
	pub async fn publish_dlq_raw(&self, payload: prost::bytes::Bytes) -> Result<()> {
		self
			.js
			.publish(PipelineSubjects::DLQ, payload)
			.await
			.map_err(|e| TransportError::NatsError(e.to_string()))?
			.await
			.map_err(|e| TransportError::NatsError(e.to_string()))?;
		Ok(())
	}
}
