use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tokio::time::sleep;
use metrics::{counter, gauge};
use prometheus::{Registry, Counter, Gauge};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct WorkerConfig {
    pub prefetch_count: usize,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub task_timeout: Duration,
    pub heartbeat_interval: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            prefetch_count: 10,
            max_retries: 3,
            retry_delay: Duration::from_secs(60),
            task_timeout: Duration::from_secs(300),
            heartbeat_interval: Duration::from_secs(30),
        }
    }
}


struct Worker {
    id: usize,
    scheduler: Arc<Mutex<RedisScheduler>>,
    config: WorkerConfig,
}

impl Worker {
    fn new(id: usize, scheduler: Arc<Mutex<RedisScheduler>>, config: WorkerConfig) -> Self {
        Self { id, scheduler, config }
    }

    async fn run(&self, result_tx: mpsc::Sender<TaskResult>) {
        loop {
            // Fetch batch of tasks
            let tasks = {
                let mut scheduler = self.scheduler.lock().unwrap();
                scheduler.dequeue_batch(self.config.prefetch_count)
                    .unwrap_or_default()
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

