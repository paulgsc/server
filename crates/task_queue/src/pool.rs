use crate::config::Config as WorkerConfig;
use crate::error::KnownError as WorkerPoolError;
use crate::redis_queue::{RedisScheduler, TaskResult, TaskStatus};
use crate::worker::Worker;
use prometheus::{Counter, Gauge, Registry};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct WorkerPool {
	config: WorkerConfig,
	scheduler: Arc<RedisScheduler>,
	#[allow(dead_code)]
	/// TODO: remove once obsolete
	registry: Registry,
	active_workers: Counter,
	#[allow(dead_code)]
	/// TODO: remove once obsolete
	queue_size: Gauge,
	task_counter: Counter,
	error_counter: Counter,
}

impl WorkerPool {
	pub fn new(scheduler: RedisScheduler, registry: Registry) -> Result<Self, WorkerPoolError> {
		let active_workers = Counter::new("worker_pool_active_workers", "Number of active workers")?;
		let queue_size = Gauge::new("worker_pool_queue_size", "Current queue size")?;
		let task_counter = Counter::new("worker_pool_tasks_processed", "Total tasks processed")?;
		let config = WorkerConfig::new();
		let error_counter = Counter::new("worker_pool_task_errors", "Total task errors")?;

		registry.register(Box::new(active_workers.clone()))?;
		registry.register(Box::new(queue_size.clone()))?;
		registry.register(Box::new(task_counter.clone()))?;
		registry.register(Box::new(error_counter.clone()))?;

		Ok(Self {
			config,
			scheduler: Arc::new(scheduler),
			registry,
			active_workers,
			queue_size,
			task_counter,
			error_counter,
		})
	}

	pub async fn start(&self, num_workers: usize) -> Result<(), WorkerPoolError> {
		let (tx, mut rx) = mpsc::channel(100);

		// Start worker threads
		for id in 0..num_workers {
			let worker_tx = tx.clone();
			let scheduler = Arc::clone(&self.scheduler);

			tokio::spawn(async move {
				let worker = Worker::new(id, scheduler);
				let _ = worker.run(worker_tx).await;
			});

			self.active_workers.inc();
		}

		// Start supervisor thread
		// let scheduler = Arc::clone(&self.scheduler);
		// let config = self.config.clone();
		// tokio::spawn(async move {
		//     let supervisor = Supervisor::new(scheduler, config);
		//     supervisor.run().await;
		// });

		// Result handler
		while let Some(result) = rx.recv().await {
			self.handle_task_result(result).await?;
		}

		Ok(())
	}

	async fn handle_task_result(&self, result: TaskResult) -> Result<(), WorkerPoolError> {
		self.task_counter.inc();

		match result.status {
			TaskStatus::Success => {
				// Store successful result
				self.scheduler.set_expiration(&result.task_id, Duration::from_secs(3600)).await?;
			}
			TaskStatus::Failed { error: _, retry_count } => {
				self.error_counter.inc();

				if retry_count < self.config.max_retries {
					// Requeue for retry
					let task = self
						.scheduler
						.get_tasks_by_pattern(&format!("task:{}", result.task_id))
						.await?
						.pop()
						.ok_or_else(|| WorkerPoolError::QueueError("Task not found".into()))?;

					tokio::time::sleep(self.config.retry_delay).await;
					self.scheduler.enqueue(task).await?;
				} else {
					// Move to dead letter queue
					// Implementation depends on your dead letter queue strategy
				}
			}
			TaskStatus::Cancelled | TaskStatus::TimedOut => {
				self.error_counter.inc();
				// Handle cancelled or timed out tasks
			}
		}

		Ok(())
	}
}
