use std::thread;
use std::time::{Duration, SystemTime};
use task_queue::error::KnownError as Error;
use task_queue::redis_queue::{RedisScheduler, SchedulerType, Task};
use tokio;

#[tokio::main]
async fn main() -> Result<(), Error> {
	println!("Running Redis Scheduler Examples...\n");

	println!("Example 1: Round Robin Scheduling");
	round_robin_example().await?;

	//	println!("\nExample 2: EDF (Earliest Deadline First) Scheduling");
	//	edf_example()?;
	//
	//	println!("\nExample 3: Batch Processing");
	//	batch_processing_example()?;
	//
	//	println!("\nExample 4: Blocking Dequeue");
	//	blocking_dequeue_example()?;
	//
	//	println!("\nExample 5: Advanced Example");
	//	advanced_examples()?;

	Ok(())
}

async fn round_robin_example() -> Result<(), Error> {
	let scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;

	// Create tasks with different priorities
	let tasks = vec![
		Task::new(
			"task1".to_string(),
			9, // High priority
			SystemTime::now() + Duration::from_secs(3600),
			Duration::from_secs(10),
		)?,
		Task::new(
			"task2".to_string(),
			5, // Medium priority
			SystemTime::now() + Duration::from_secs(3600),
			Duration::from_secs(5),
		)?,
		Task::new(
			"task3".to_string(),
			2, // Low priority
			SystemTime::now() + Duration::from_secs(3600),
			Duration::from_secs(3),
		)?,
	];

	// Enqueue tasks
	for task in tasks {
		scheduler.enqueue(task).await?;
	}

	// Print queue lengths
	let lengths = scheduler.get_queue_lengths().await?;
	println!("Queue lengths - High: {}, Medium: {}, Low: {}", lengths[0], lengths[1], lengths[2]);

	// Dequeue and process tasks
	//	while let Some(task) = scheduler.dequeue_blocking(1.0)? {
	//		println!("Processing task {} with priority {}", task.id, task.priority);
	//	}

	Ok(())
}

#[allow(dead_code)]
async fn edf_example() -> Result<(), Error> {
	let scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::EDF)?;

	// Create tasks with different deadlines
	let tasks = vec![
		Task::new(
			"urgent_task".to_string(),
			5,
			SystemTime::now() + Duration::from_secs(60), // 1 minute deadline
			Duration::from_secs(10),
		)?,
		Task::new(
			"normal_task".to_string(),
			5,
			SystemTime::now() + Duration::from_secs(300), // 5 minutes deadline
			Duration::from_secs(20),
		)?,
		Task::new(
			"relaxed_task".to_string(),
			5,
			SystemTime::now() + Duration::from_secs(3600), // 1 hour deadline
			Duration::from_secs(30),
		)?,
	];

	// Enqueue tasks
	for task in tasks {
		scheduler.enqueue(task).await?;
	}

	// Dequeue tasks - they should come out in deadline order
	while let Some(task) = scheduler.dequeue_blocking(1.0).await? {
		println!("Dequeued task {} with deadline {}", task.id, task.deadline);
	}

	Ok(())
}

#[allow(dead_code)]
async fn batch_processing_example() -> Result<(), Error> {
	let scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;

	// Create a batch of tasks
	let tasks: Vec<Task> = (0..10)
		.map(|i| {
			Task::new(
				format!("batch_task_{}", i),
				(i % 3 * 4) as u8, // Distribute priorities (0, 4, 8)
				SystemTime::now() + Duration::from_secs(3600),
				Duration::from_secs(5),
			)
		})
		.collect::<Result<Vec<_>, _>>()?;

	// Enqueue all tasks
	for task in tasks {
		scheduler.enqueue(task).await?;
	}

	// Process tasks in batches of 3
	loop {
		let batch = scheduler.dequeue_batch(3).await?;
		if batch.is_empty() {
			break;
		}

		println!("Processing batch of {} tasks:", batch.len());
		for task in batch {
			println!("- Task {} (Priority: {})", task.id, task.priority);
		}
	}

	Ok(())
}

#[allow(dead_code)]
async fn blocking_dequeue_example() -> Result<(), Error> {
	let scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;
	let scheduler_clone = scheduler.clone();

	// Spawn a thread that will add tasks after a delay
	tokio::spawn(async move {
		tokio::time::sleep(Duration::from_secs(2)).await;

		match Task::new("delayed_task".to_string(), 8, SystemTime::now() + Duration::from_secs(3600), Duration::from_secs(5)) {
			Ok(delayed_task) => {
				if let Err(e) = scheduler_clone.enqueue(delayed_task).await {
					eprintln!("Failed to enqueue delayed task: {}", e);
				}
			}
			Err(e) => {
				eprintln!("Failed to create delayed task: {}", e);
			}
		}
	});

	// Try to dequeue with a timeout of 3 seconds
	println!("Waiting for tasks...");
	match scheduler.dequeue_blocking(3.0).await? {
		Some(task) => println!("Received delayed task: {}", task.id),
		None => println!("Timeout reached, no tasks received"),
	}

	Ok(())
}

// Function to demonstrate task expiration and pattern matching
#[allow(dead_code)]
async fn advanced_examples() -> Result<(), Error> {
	println!("\nRunning Advanced Examples...");

	let scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;

	// Create a task with expiration
	let task = Task::new("expiring_task".to_string(), 8, SystemTime::now() + Duration::from_secs(3600), Duration::from_secs(10))?;

	// Set task to expire after 5 seconds
	scheduler.enqueue(task.clone()).await?;
	scheduler.set_expiration(&task.id, Duration::from_secs(5)).await?;

	// Wait for expiration
	thread::sleep(Duration::from_secs(6));

	// Try to retrieve expired task
	let tasks = scheduler.get_tasks_by_pattern(&format!("*{}*", task.id)).await?;
	println!("Found {} tasks matching pattern", tasks.len());

	Ok(())
}
