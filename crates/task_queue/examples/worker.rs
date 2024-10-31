use prometheus::Registry;
use std::time::{Duration, SystemTime};
use task_queue::pool::WorkerPool;
use task_queue::redis_queue::{RedisScheduler, SchedulerType};
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Initialize Prometheus registry for metrics
	let registry = Registry::new();

	// Configure Redis scheduler
	let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
	let scheduler = RedisScheduler::new(&redis_url, SchedulerType::EDF)?;

	// Create worker pool
	let pool = WorkerPool::new(scheduler.clone(), registry);

	// Create some example tasks
	let example_tasks = create_example_tasks();

	// Enqueue the example tasks
	{
		let mut scheduler = scheduler;
		for task in example_tasks {
			scheduler.enqueue(task)?;
		}
	}

	// Start the worker pool with 3 workers
	println!("Starting worker pool with 3 workers...");
	pool.start(3).await?;

	Ok(())
}

fn create_example_tasks() -> Vec<Task> {
	let now = SystemTime::now();
	let mut tasks = Vec::new();

	// Create tasks with different priorities and deadlines
	for i in 0..10 {
		let priority = match i % 3 {
			0 => 8, // High priority
			1 => 5, // Medium priority
			_ => 2, // Low priority
		};

		let deadline = now + Duration::from_secs(300 + i * 60); // Staggered deadlines
		let execution_time = Duration::from_secs(10 + i * 5); // Varying execution times

		let task = Task::new(format!("task-{}", i), priority, deadline, execution_time);

		tasks.push(task);
	}

	tasks
}
