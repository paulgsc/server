use crate::config::Config as WorkerConfig;
use crate::error::KnownError as WorkerError;
use crate::redis_queue::{RedisScheduler, Task, TaskResult, TaskStatus};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tokio::time::sleep;

pub struct Worker {
	#[allow(dead_code)]
	/// TODO: remove once obsolete
	id: usize,
	scheduler: Arc<RedisScheduler>,
	config: WorkerConfig,
}

impl Worker {
	#[must_use]
	pub fn new(id: usize, scheduler: Arc<RedisScheduler>) -> Self {
		let config = WorkerConfig::new();
		Self { id, scheduler, config }
	}

	pub async fn run(&self, result_tx: mpsc::Sender<TaskResult>) -> Result<(), WorkerError> {
		loop {
			let tasks = self.scheduler.dequeue_batch(self.config.prefetch_count).await?;

			for task in tasks {
				let start_time = SystemTime::now();

				let status = tokio::select! {
						() = sleep(self.config.task_timeout) => {
								TaskStatus::TimedOut
						}
						result = self.execute_task(&task) => {
								match result {
										Ok(()) => TaskStatus::Success,
										Err(e) => TaskStatus::Failed {
												error: e.to_string(),
												retry_count: 0,
										},
								}
						}
				};

				let result = TaskResult {
					task_id: task.id,
					status,
					execution_time: start_time.elapsed()?,
					completed_at: SystemTime::now(),
				};

				if result_tx.send(result).await.is_err() {
					return Err(WorkerError::InternalError("Result channel closed".to_string()));
				}
			}

			// Small delay to prevent tight polling
			sleep(Duration::from_millis(100)).await;
		}
	}

	async fn execute_task(&self, task: &Task) -> Result<(), WorkerError> {
		// Actual task execution would go here
		// This is a placeholder that simulates work
		println!("executing some task: {}", task);
		sleep(Duration::from_secs(task.execution_time)).await;
		Ok(())
	}
}
