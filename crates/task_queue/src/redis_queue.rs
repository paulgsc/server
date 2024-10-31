use redis::{cmd, Client, Commands, Connection, RedisError};
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::time::{Duration, SystemTime};

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
	pub fn new(id: String, priority: u8, deadline: SystemTime, execution_time: Duration) -> Self {
		let now = SystemTime::now();
		let deadline_secs = deadline.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
		let now_secs = now.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();

		Self {
			id,
			priority,
			deadline: deadline_secs,
			execution_time: execution_time.as_secs(),
			arrival_time: now_secs,
			remaining_time: execution_time.as_secs(),
		}
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

#[derive(Debug, Clone)]
pub struct RedisScheduler {
	conn: Connection,
	scheduler_type: SchedulerType,
	queue_keys: Vec<String>,
}

#[derive(Clone, Copy)]
pub enum SchedulerType {
	RoundRobin,
	EDF,
}

impl RedisScheduler {
	pub fn new(redis_url: &str, scheduler_type: SchedulerType) -> Result<Self, RedisError> {
		let client = Client::open(redis_url)?;
		let conn = client.get_connection()?;

		// For RR, we'll create multiple queues based on priority
		let queue_keys = match scheduler_type {
			SchedulerType::RoundRobin => vec!["scheduler:high".to_string(), "scheduler:medium".to_string(), "scheduler:low".to_string()],
			SchedulerType::EDF => vec!["scheduler:edf".to_string()],
		};

		Ok(Self { conn, scheduler_type, queue_keys })
	}

	pub fn enqueue(&mut self, task: Task) -> Result<(), RedisError> {
		let serialized = serde_json::to_string(&task).unwrap();

		match self.scheduler_type {
			SchedulerType::RoundRobin => {
				let queue_idx = match task.priority {
					8..=u8::MAX => 0,
					4..=7 => 1,
					0..=3 => 2,
				};

				self.conn.rpush(&self.queue_keys[queue_idx], serialized)?;
			}
			SchedulerType::EDF => {
				self.conn.zadd(&self.queue_keys[0], serialized, task.deadline as f64)?;
			}
		}

		Ok(())
	}

	pub fn dequeue_batch(&mut self, count: usize) -> Result<Vec<Task>, RedisError> {
		match self.scheduler_type {
			SchedulerType::RoundRobin => {
				let mut tasks = Vec::new();
				for key in &self.queue_keys {
					if tasks.len() >= count {
						break;
					}
					let remaining = count - tasks.len();
					if let Some(count_nz) = NonZeroUsize::new(remaining) {
						let serialized_items: Vec<String> = self.conn.lpop(key, Some(count_nz))?;
						tasks.extend(serialized_items.into_iter().map(|s| serde_json::from_str(&s).unwrap()));
					}
				}
				Ok(tasks)
			}
			SchedulerType::EDF => {
				let results: Vec<(String, f64)> = self.conn.zpopmin(&self.queue_keys[0], count as isize)?;
				Ok(results.into_iter().map(|(serialized, _)| serde_json::from_str(&serialized).unwrap()).collect())
			}
		}
	}

	pub fn dequeue_blocking(&mut self, timeout: f64) -> Result<Option<Task>, RedisError> {
		match self.scheduler_type {
			SchedulerType::RoundRobin => {
				let result: Option<(String, String)> = self.conn.blpop(&self.queue_keys, timeout)?;
				Ok(result.map(|(_, serialized)| serde_json::from_str(&serialized).unwrap()))
			}
			SchedulerType::EDF => {
				let result: Option<(String, String, f64)> = cmd("BZPOPMIN").arg(&self.queue_keys[0]).arg(timeout).query(&mut self.conn)?;

				Ok(result.map(|(_, serialized, _)| serde_json::from_str(&serialized).unwrap()))
			}
		}
	}

	pub fn get_queue_lengths(&mut self) -> Result<Vec<usize>, RedisError> {
		let mut lengths = Vec::new();

		for key in &self.queue_keys {
			let len = match self.scheduler_type {
				SchedulerType::RoundRobin => self.conn.llen(key)?,
				SchedulerType::EDF => self.conn.zcard(key)?,
			};
			lengths.push(len);
		}

		Ok(lengths)
	}

	// Set task expiration
	pub fn set_expiration(&mut self, task_id: &str, ttl: Duration) -> Result<(), RedisError> {
		self.conn.expire(task_id, ttl.as_secs().try_into().unwrap())
	}

	// Get tasks by pattern (e.g., all high priority tasks)
	pub fn get_tasks_by_pattern(&mut self, pattern: &str) -> Result<Vec<Task>, RedisError> {
		let keys: Vec<String> = self.conn.keys(pattern)?;
		let mut tasks = Vec::new();

		for key in keys {
			if let Some(serialized) = self.conn.get::<_, Option<String>>(&key)? {
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
	fn clear_redis_queues(conn: &mut Connection) -> Result<(), RedisError> {
		let patterns = ["scheduler:high", "scheduler:medium", "scheduler:low", "scheduler:edf"];
		for key in patterns.iter() {
			let _: () = redis::cmd("FLUSHDB").query(conn)?;
		}
		Ok(())
	}

	#[test]
	fn test_redis_scheduler_initialization() -> Result<(), RedisError> {
		let mut round_robin_scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut round_robin_scheduler.conn)?;

		assert!(matches!(round_robin_scheduler.scheduler_type, SchedulerType::RoundRobin));

		let mut edf_scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::EDF)?;
		clear_redis_queues(&mut edf_scheduler.conn)?;

		assert!(matches!(edf_scheduler.scheduler_type, SchedulerType::EDF));
		Ok(())
	}

	#[test]
	fn test_enqueue_dequeue_round_robin() -> Result<(), RedisError> {
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
	fn test_enqueue_dequeue_edf() -> Result<(), RedisError> {
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
	fn test_task_expiration() -> Result<(), RedisError> {
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
	fn test_dequeue_batch() -> Result<(), RedisError> {
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
	fn test_blocking_dequeue_timeout() -> Result<(), RedisError> {
		let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
		clear_redis_queues(&mut scheduler.conn)?;

		let result = scheduler.dequeue_blocking(1.0)?;
		assert!(result.is_none(), "Expected no task to be dequeued after timeout");

		Ok(())
	}

	#[test]
	fn test_queue_lengths() -> Result<(), RedisError> {
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
