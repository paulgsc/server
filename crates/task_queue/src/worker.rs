use crate::config::Config as WorkerConfig;
use crate::error::TaskError;
use crate::redis_queue::{RedisScheduler, Task, TaskResult, TaskStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tokio::time::sleep;

struct Worker {
	id: usize,
	scheduler: Arc<Mutex<RedisScheduler>>,
	config: WorkerConfig,
}

impl Worker {
	fn new(id: usize, scheduler: Arc<Mutex<RedisScheduler>>) -> Self {
		let config = WorkerConfig::new();
		Self { id, scheduler, config }
	}

	async fn run(&self, result_tx: mpsc::Sender<TaskResult>) {
		loop {
			let tasks = {
				let mut scheduler = self.scheduler.lock().unwrap();
				scheduler.dequeue_batch(self.config.prefetch_count).unwrap_or_default()
			};

			for task in tasks {
				let start_time = SystemTime::now();

				// Execute task with timeout
				let status = tokio::select! {
						_ = sleep(self.config.task_timeout) => {
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
					execution_time: start_time.elapsed().unwrap(),
					completed_at: SystemTime::now(),
				};

				if result_tx.send(result).await.is_err() {
					return; // Channel closed, worker should shut down
				}
			}

			// Small delay to prevent tight polling
			sleep(Duration::from_millis(100)).await;
		}
	}

	async fn execute_task(&self, task: &Task) -> Result<(), TaskError> {
		// Actual task execution would go here
		// This is a placeholder that simulates work
		sleep(Duration::from_secs(task.execution_time)).await;
		Ok(())
	}
}
