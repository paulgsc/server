use super::{JobRecord, JobState, Store};
use crate::{engine::process_job, error::StageError, llm::LlmBackend, stages::EmbedProvider};
use some_transport::nats::{AckHandle, DurableConsumer, JetStreamConfig, JetStreamPublisher, NatsTransport};
use std::sync::Arc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_events::tabsched::JobEnvelope;
// ── Shared worker context ─────────────────────────────────────────────────

/// Everything a worker needs, cheap to clone (Arc internals).
#[derive(Clone)]
pub struct WorkerCtx {
	pub http: reqwest::Client,
	pub embed_provider: Arc<EmbedProvider>,
	pub llm: Arc<dyn LlmBackend>,
	pub store: Store,
	pub transport: NatsTransport<JobEnvelope>,
	pub publisher: Arc<JetStreamPublisher<JobEnvelope>>,
	pub edge_template: Arc<String>,
	pub track_template: Arc<String>,
	pub similarity_threshold: f32,
	pub window_size: u32,
}

// ── Worker loop ───────────────────────────────────────────────────────────

/// Single worker task.  Runs until the cancellation token fires.
pub async fn worker(worker_id: usize, ctx: WorkerCtx, js_config: JetStreamConfig, token: CancellationToken) {
	info!(worker_id, "worker starting");

	let mut consumer = match DurableConsumer::<JobEnvelope>::bind(ctx.transport.client().clone(), js_config).await {
		Ok(c) => c,
		Err(e) => {
			error!(worker_id, error = %e, "worker failed to connect to JetStream");
			return;
		}
	};

	loop {
		// Honour shutdown signal.
		tokio::select! {
				biased;
				_ = token.cancelled() => {
						info!(worker_id, "worker shutting down");
						return;
				}
				result = consumer.next_msg(Duration::from_secs(5)) => {
						match result {
								// No messages available — tight-loop back to select.
								Ok(None) => continue,

								Err(e) => {
										// JetStream fetch error — log and back off briefly.
										// Do NOT exit; transient NATS issues should not kill workers.
										warn!(worker_id, error = %e, "JetStream fetch error, backing off");
										tokio::time::sleep(Duration::from_secs(2)).await;
								}

								Ok(Some((envelope, handle))) => {
										let session_id = envelope.session_id.clone();
										info!(worker_id, session_id = %session_id, attempt = envelope.attempt, "job received");

										let result = process_job(&ctx, &session_id, token.clone()).await;
										settle(result, handle, &ctx, &session_id, worker_id).await;
								}
						}
				}
		}
	}
}

/// Dispatch ACK / NAK / TERM based on the stage result.
async fn settle(result: Result<(), StageError>, handle: AckHandle, ctx: &WorkerCtx, session_id: &str, worker_id: usize) {
	match result {
		Ok(()) => {
			if let Err(e) = handle.ack().await {
				error!(worker_id, session_id, error = %e, "ACK failed");
			}
		}

		Err(StageError::Retryable(e)) => {
			warn!(worker_id, session_id, error = %e, "transient failure — NAK");
			if let Err(ne) = handle.nak().await {
				error!(worker_id, session_id, error = %ne, "NAK failed");
			}
		}

		Err(StageError::Permanent(e)) | Err(StageError::Poison(e)) => {
			let reason = e.to_string();
			error!(worker_id, session_id, error = %reason, "permanent/poison failure — TERM + DLQ");
			// we'll format the payload as a string for now
			let payload = format!("session_id: {}: failed with reason: {}", session_id, reason);

			// Push to Redis DLQ list for manual inspection.
			if let Err(de) = ctx.store.clone().push_dlq(session_id, &reason).await {
				error!(worker_id, session_id, error = %de, "DLQ push failed");
			}

			// Publish to NATS DLQ subject for external consumers.
			if let Err(pe) = ctx.publisher.publish_dlq_raw(payload.into()).await {
				error!(worker_id, session_id, error = %pe, "NATS DLQ publish failed");
			}

			// TERM: server will never redeliver.
			if let Err(te) = handle.term().await {
				error!(worker_id, session_id, error = %te, "TERM failed");
			}

			// Write Failed state to Redis.
			let mut record = JobRecord::new(session_id);
			record.transition(JobState::Failed { reason });
			ctx.store.clone().write_state(&record).await.ok();
		}
	}
}
