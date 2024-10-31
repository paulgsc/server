use crate::error::KnownError as TaskQueueError;
use redis::{cmd, Client, Commands, Connection};
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Task {
	pub id: String,
	pub priority: u8,
	pub deadline: u64, // Stored as timestamp for Redis compatibility
	pub execution_time: u64,
	pub arrival_time: u64,
	pub remaining_time: u64,
}

impl Task {
	///
	/// # Errors
	/// Returns error if system time operations fail or if there are conversion errors
	pub fn new(id: String, priority: u8, deadline: SystemTime, execution_time: Duration) -> Result<Self, TaskQueueError> {
		let now = SystemTime::now();
		let deadline_secs = deadline.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
		let now_secs = now.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();

		Ok(Self {
			id,
			priority,
			deadline: deadline_secs,
			execution_time: execution_time.as_secs(),
			arrival_time: now_secs,
			remaining_time: execution_time.as_secs(),
		})
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
	Success,
	Failed { error: String, retry_count: u32 },
	Cancelled,
	TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
	pub task_id: String,
	pub status: TaskStatus,
	pub execution_time: Duration,
	pub completed_at: SystemTime,
}

#[derive(Clone)]
pub struct RedisScheduler {
	conn: Arc<Mutex<Connection>>,
	scheduler_type: SchedulerType,
	queue_keys: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum SchedulerType {
	RoundRobin,
	EDF,
}

impl RedisScheduler {
	///
	/// # Errors
	/// This function returns an error if:
	/// - Redis connection fails
	/// - Invalid Configuration provided
	pub fn new(redis_url: &str, scheduler_type: SchedulerType) -> Result<Self, TaskQueueError> {
		let client = Client::open(redis_url)?;
		let conn = client.get_connection()?;

		// For RR, we'll create multiple queues based on priority
		let queue_keys = match scheduler_type {
			SchedulerType::RoundRobin => vec!["scheduler:high".to_string(), "scheduler:medium".to_string(), "scheduler:low".to_string()],
			SchedulerType::EDF => vec!["scheduler:edf".to_string()],
		};

		Ok(Self {
			conn: Arc::new(Mutex::new(conn)),
			scheduler_type,
			queue_keys,
		})
	}

	///
	/// # Errors
	/// This function returns an error if:
	/// - Redis operations fails
	/// - Serialization fails
	pub async fn enqueue(&self, task: Task) -> Result<(), TaskQueueError> {
		let serialized = serde_json::to_string(&task)?;
		let mut conn = self.conn.lock().await;

		match self.scheduler_type {
			SchedulerType::RoundRobin => {
				let queue_idx = match task.priority {
					8..=u8::MAX => 0,
					4..=7 => 1,
					0..=3 => 2,
				};

				conn.rpush(&self.queue_keys[queue_idx], serialized)?;
			}
			SchedulerType::EDF => {
				conn.zadd(&self.queue_keys[0], serialized, task.deadline as f64)?;
			}
		}

		drop(conn);
		Ok(())
	}

	///
	/// # Errors
	/// This function returns an error if:
	/// - Redis operations fails
	/// - Serialization fails
	pub async fn dequeue_batch(&self, count: usize) -> Result<Vec<Task>, TaskQueueError> {
		let mut conn = self.conn.lock().await;

		match self.scheduler_type {
			SchedulerType::RoundRobin => {
				let mut tasks = Vec::new();
				for key in &self.queue_keys {
					if tasks.len() >= count {
						break;
					}
					let remaining = count - tasks.len();
					if let Some(count_nz) = NonZeroUsize::new(remaining) {
						let serialized_items: Vec<String> = conn.lpop(key, Some(count_nz))?;
						tasks.extend(serialized_items.into_iter().map(|s| serde_json::from_str(&s)).collect::<Result<Vec<Task>, _>>()?);
					}
				}
				drop(conn);
				Ok(tasks)
			}
			SchedulerType::EDF => {
				let results: Vec<(String, f64)> = conn.zpopmin(&self.queue_keys[0], count as isize)?;
				drop(conn);
				Ok(
					results
						.into_iter()
						.map(|(serialized, _)| serde_json::from_str::<Task>(&serialized))
						.collect::<Result<Vec<Task>, _>>()?,
				)
			}
		}
	}

	///
	/// # Errors
	/// This function returns an error if:
	/// - Redis operations fails
	/// - Serialization fails
	pub async fn dequeue_blocking(&self, timeout: f64) -> Result<Option<Task>, TaskQueueError> {
		let mut conn = self.conn.lock().await;

		match self.scheduler_type {
			SchedulerType::RoundRobin => {
				let result: Option<(String, String)> = conn.blpop(&self.queue_keys, timeout)?;
				drop(conn);
				Ok(result.and_then(|(_, serialized)| serde_json::from_str::<Task>(&serialized).ok()))
			}
			SchedulerType::EDF => {
				let result: Option<(String, String, f64)> = cmd("BZPOPMIN").arg(&self.queue_keys[0]).arg(timeout).query(&mut *conn)?;

				drop(conn);
				Ok(result.and_then(|(_, serialized, _)| serde_json::from_str::<Task>(&serialized).ok()))
			}
		}
	}

	///
	/// # Errors
	/// This function returns an error if:
	/// - Redis operations fails
	pub async fn get_queue_lengths(&self) -> Result<Vec<usize>, TaskQueueError> {
		let mut queue_lengths = Vec::with_capacity(self.queue_keys.len());
		let mut conn = self.conn.lock().await;

		for key in &self.queue_keys {
			let len = match self.scheduler_type {
				SchedulerType::RoundRobin => conn.llen(key)?,
				SchedulerType::EDF => conn.zcard(key)?,
			};
			queue_lengths.push(len);
		}

		drop(conn);
		Ok(queue_lengths)
	}

	///
	/// # Errors
	/// This function returns an error if:
	/// - Redis operations fails
	/// - Conversion fails
	pub async fn set_expiration(&self, task_id: &str, ttl: Duration) -> Result<(), TaskQueueError> {
		let mut conn = self.conn.lock().await;
		let _: bool = conn.expire(task_id, ttl.as_secs().try_into()?)?;
		drop(conn);
		Ok(())
	}

	///
	/// # Errors
	/// This function returns an error if:
	/// - Redis operations fails
	/// - Deserialization fails
	pub async fn get_tasks_by_pattern(&self, pattern: &str) -> Result<Vec<Task>, TaskQueueError> {
		let mut conn = self.conn.lock().await;
		let keys: Vec<String> = conn.keys(pattern)?;
		let mut tasks = Vec::new();

		for key in keys {
			if let Some(serialized) = conn.get::<_, Option<String>>(&key)? {
				if let Ok(task) = serde_json::from_str(&serialized) {
					tasks.push(task);
				}
			}
		}

		Ok(tasks)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::thread::sleep;

	// Helper function to clear Redis queues
	fn clear_redis_queues(conn: &mut Connection) -> Result<(), TaskQueueError> {
		let patterns = ["scheduler:high", "scheduler:medium", "scheduler:low", "scheduler:edf"];
		for key in patterns.iter() {
			let _: () = redis::cmd("FLUSHDB").query(conn)?;
		}
		Ok(())
	}

	#[test]
	fn test_redis_scheduler_initialization() -> Result<(), TaskQueueError> {
		let mut round_robin_scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut round_robin_scheduler.conn)?;

		assert!(matches!(round_robin_scheduler.scheduler_type, SchedulerType::RoundRobin));

		let mut edf_scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::EDF)?;
		clear_redis_queues(&mut edf_scheduler.conn)?;

		assert!(matches!(edf_scheduler.scheduler_type, SchedulerType::EDF));
		Ok(())
	}

	#[test]
	fn test_enqueue_dequeue_round_robin() -> Result<(), TaskQueueError> {
		let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut scheduler.conn)?;

		let task1 = Task::new("task1".to_string(), 9, SystemTime::now() + Duration::from_secs(60), Duration::from_secs(10));
		let task2 = Task::new("task2".to_string(), 5, SystemTime::now() + Duration::from_secs(120), Duration::from_secs(20));

		scheduler.enqueue(task1)?;
		scheduler.enqueue(task2)?;

		let first = scheduler.dequeue_batch(1)?.pop().unwrap();
		let second = scheduler.dequeue_batch(1)?.pop().unwrap();

		assert_eq!(first.id, "task1");
		assert_eq!(second.id, "task2");
		Ok(())
	}

	#[test]
	fn test_enqueue_dequeue_edf() -> Result<(), TaskQueueError> {
		let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::EDF)?;
		clear_redis_queues(&mut scheduler.conn)?;

		let task1 = Task::new("task1".to_string(), 5, SystemTime::now() + Duration::from_secs(50), Duration::from_secs(10));
		let task2 = Task::new("task2".to_string(), 7, SystemTime::now() + Duration::from_secs(100), Duration::from_secs(20));

		scheduler.enqueue(task1)?;
		scheduler.enqueue(task2)?;

		let first = scheduler.dequeue_batch(1)?.pop().unwrap();
		let second = scheduler.dequeue_batch(1)?.pop().unwrap();

		assert_eq!(first.id, "task1");
		assert_eq!(second.id, "task2");

		Ok(())
	}

	#[test]
	fn test_task_expiration() -> Result<(), TaskQueueError> {
		let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut scheduler.conn)?;

		let task = Task::new("task_expire".to_string(), 5, SystemTime::now() + Duration::from_secs(120), Duration::from_secs(10));
		scheduler.enqueue(task.clone())?;

		// Set task to expire immediately
		scheduler.set_expiration(&task.id, Duration::from_secs(1))?;
		sleep(Duration::from_secs(2));

		// Try to dequeue the expired task
		let result = scheduler.get_tasks_by_pattern("scheduler:*")?;
		assert!(result.iter().all(|t| t.id != task.id), "Expired task was not removed");

		Ok(())
	}

	#[test]
	fn test_dequeue_batch() -> Result<(), TaskQueueError> {
		let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut scheduler.conn)?;

		let task1 = Task::new("task_batch1".to_string(), 9, SystemTime::now() + Duration::from_secs(60), Duration::from_secs(10));
		let task2 = Task::new("task_batch2".to_string(), 5, SystemTime::now() + Duration::from_secs(120), Duration::from_secs(20));
		let task3 = Task::new("task_batch3".to_string(), 2, SystemTime::now() + Duration::from_secs(150), Duration::from_secs(30));

		scheduler.enqueue(task1)?;
		scheduler.enqueue(task2)?;
		scheduler.enqueue(task3)?;

		let batch = scheduler.dequeue_batch(2)?;
		assert_eq!(batch.len(), 2);

		let ids: Vec<String> = batch.into_iter().map(|task| task.id).collect();
		assert_eq!(ids, vec!["task_batch1", "task_batch2"]);

		Ok(())
	}

	#[test]
	fn test_blocking_dequeue_timeout() -> Result<(), TaskQueueError> {
		let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut scheduler.conn)?;

		let result = scheduler.dequeue_blocking(1.0)?;
		assert!(result.is_none(), "Expected no task to be dequeued after timeout");

		Ok(())
	}

	#[test]
	fn test_queue_lengths() -> Result<(), TaskQueueError> {
		let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut scheduler.conn)?;

		let task1 = Task::new("task_len1".to_string(), 9, SystemTime::now() + Duration::from_secs(60), Duration::from_secs(10));
		let task2 = Task::new("task_len2".to_string(), 5, SystemTime::now() + Duration::from_secs(120), Duration::from_secs(20));

		scheduler.enqueue(task1)?;
		scheduler.enqueue(task2)?;

		let lengths = scheduler.get_queue_lengths()?;
		assert_eq!(lengths.iter().sum::<usize>(), 2, "Total queue length mismatch");

		Ok(())
	}
}
