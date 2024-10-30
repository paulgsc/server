use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tokio::time::sleep;
use metrics::{counter, gauge};
use prometheus::{Registry, Counter, Gauge};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub struct WorkerPool {
    config: WorkerConfig,
    scheduler: Arc<Mutex<RedisScheduler>>,
    registry: Registry,
    active_workers: Counter,
    queue_size: Gauge,
    task_counter: Counter,
    error_counter: Counter,
}

impl WorkerPool {
    pub fn new(
        scheduler: RedisScheduler,
        config: WorkerConfig,
        registry: Registry,
    ) -> Self {
        let active_workers = Counter::new("worker_pool_active_workers", "Number of active workers").unwrap();
        let queue_size = Gauge::new("worker_pool_queue_size", "Current queue size").unwrap();
        let task_counter = Counter::new("worker_pool_tasks_processed", "Total tasks processed").unwrap();
        let error_counter = Counter::new("worker_pool_task_errors", "Total task errors").unwrap();

        registry.register(Box::new(active_workers.clone())).unwrap();
        registry.register(Box::new(queue_size.clone())).unwrap();
        registry.register(Box::new(task_counter.clone())).unwrap();
        registry.register(Box::new(error_counter.clone())).unwrap();

        Self {
            config,
            scheduler: Arc::new(Mutex::new(scheduler)),
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
            let config = self.config.clone();
            
            tokio::spawn(async move {
                let worker = Worker::new(id, scheduler, config);
                worker.run(worker_tx).await;
            });
            
            self.active_workers.inc();
        }

        // Start supervisor thread
        let scheduler = Arc::clone(&self.scheduler);
        let config = self.config.clone();
        
        tokio::spawn(async move {
            let supervisor = Supervisor::new(scheduler, config);
            supervisor.run().await;
        });

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
                let mut scheduler = self.scheduler.lock().unwrap();
                scheduler.set_expiration(&result.task_id, Duration::from_secs(3600))?;
            }
            TaskStatus::Failed { error, retry_count } => {
                self.error_counter.inc();
                
                if retry_count < self.config.max_retries {
                    // Requeue for retry
                    let mut scheduler = self.scheduler.lock().unwrap();
                    let task = scheduler.get_tasks_by_pattern(&format!("task:{}", result.task_id))?
                        .pop()
                        .ok_or_else(|| TaskError::QueueError("Task not found".into()))?;
                    
                    tokio::time::sleep(self.config.retry_delay).await;
                    scheduler.enqueue(task)?;
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

