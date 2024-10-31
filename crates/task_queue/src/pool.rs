use crate::config::Config as WorkerConfig;
use crate::error::TaskError;
use crate::redis_queue::{RedisScheduler, TaskResult, TaskStatus};
use crate::worker::Worker;
use prometheus::{Counter, Gauge, Registry};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct WorkerPool {
	config: WorkerConfig,
    scheduler: Arc<RedisScheduler>,
	registry: Registry,
	active_workers: Counter,
	queue_size: Gauge,
	task_counter: Counter,
	error_counter: Counter,
}

impl WorkerPool {
	pub fn new(scheduler: RedisScheduler, registry: Registry) -> Self {
		let active_workers = Counter::new("worker_pool_active_workers", "Number of active workers").unwrap();
		let queue_size = Gauge::new("worker_pool_queue_size", "Current queue size").unwrap();
		let task_counter = Counter::new("worker_pool_tasks_processed", "Total tasks processed").unwrap();
		let config = WorkerConfig::new();
		let error_counter = Counter::new("worker_pool_task_errors", "Total task errors").unwrap();

		registry.register(Box::new(active_workers.clone())).unwrap();
		registry.register(Box::new(queue_size.clone())).unwrap();
		registry.register(Box::new(task_counter.clone())).unwrap();
		registry.register(Box::new(error_counter.clone())).unwrap();

		Self {
			config,
			scheduler: Arc::new(scheduler),
			registry,
			active_workers,
			queue_size,
			task_counter,
			error_counter,
		}
	}

	pub async fn start(&self, num_workers: usize) -> Result<(), TaskError> {
		let (tx, mut rx) = mpsc::channel(100);

		// Start worker threads
		for id in 0..num_workers {
			let worker_tx = tx.clone();
			let scheduler = Arc::clone(&self.scheduler);

			tokio::spawn(async move {
				let worker = Worker::new(id, scheduler);
				worker.run(worker_tx).await;
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

	async fn handle_task_result(&self, result: TaskResult) -> Result<(), TaskError> {
		self.task_counter.inc();

		match result.status {
			TaskStatus::Success => {
				// Store successful result
				self.scheduler.set_expiration(&result.task_id, Duration::from_secs(3600)).await?;
			}
			TaskStatus::Failed { error, retry_count } => {
				self.error_counter.inc();

				if retry_count < self.config.max_retries {
					// Requeue for retry
					let task = self.scheduler
						.get_tasks_by_pattern(&format!("task:{}", result.task_id))
						.await?
						.pop()
						.ok_or_else(|| TaskError::QueueError("Task not found".into()))?;

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
