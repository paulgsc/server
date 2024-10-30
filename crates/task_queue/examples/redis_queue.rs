use redis::RedisError;
use std::thread;
use std::time::{Duration, SystemTime};
use task_queue::redis_queue::{RedisScheduler, SchedulerType, Task};

pub fn main() -> Result<(), RedisError> {
	println!("Running Redis Scheduler Examples...\n");

	println!("Example 1: Round Robin Scheduling");
	round_robin_example()?;

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

fn round_robin_example() -> Result<(), RedisError> {
	let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;

	// Create tasks with different priorities
	let tasks = vec![
		Task::new(
			"task1".to_string(),
			9, // High priority
			SystemTime::now() + Duration::from_secs(3600),
			Duration::from_secs(10),
		),
		Task::new(
			"task2".to_string(),
			5, // Medium priority
			SystemTime::now() + Duration::from_secs(3600),
			Duration::from_secs(5),
		),
		Task::new(
			"task3".to_string(),
			2, // Low priority
			SystemTime::now() + Duration::from_secs(3600),
			Duration::from_secs(3),
		),
	];

	// Enqueue tasks
	for task in tasks {
		scheduler.enqueue(task)?;
	}

	// Print queue lengths
	let lengths = scheduler.get_queue_lengths()?;
	println!("Queue lengths - High: {}, Medium: {}, Low: {}", lengths[0], lengths[1], lengths[2]);

	// Dequeue and process tasks
	//	while let Some(task) = scheduler.dequeue_blocking(1.0)? {
	//		println!("Processing task {} with priority {}", task.id, task.priority);
	//	}

	Ok(())
}

fn edf_example() -> Result<(), RedisError> {
	let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::EDF)?;

	// Create tasks with different deadlines
	let tasks = vec![
		Task::new(
			"urgent_task".to_string(),
			5,
			SystemTime::now() + Duration::from_secs(60), // 1 minute deadline
			Duration::from_secs(10),
		),
		Task::new(
			"normal_task".to_string(),
			5,
			SystemTime::now() + Duration::from_secs(300), // 5 minutes deadline
			Duration::from_secs(20),
		),
		Task::new(
			"relaxed_task".to_string(),
			5,
			SystemTime::now() + Duration::from_secs(3600), // 1 hour deadline
			Duration::from_secs(30),
		),
	];

	// Enqueue tasks
	for task in tasks {
		scheduler.enqueue(task)?;
	}

	// Dequeue tasks - they should come out in deadline order
	while let Some(task) = scheduler.dequeue_blocking(1.0)? {
		println!("Dequeued task {} with deadline {}", task.id, task.deadline);
	}

	Ok(())
}

fn batch_processing_example() -> Result<(), RedisError> {
	let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;

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
		.collect();

	// Enqueue all tasks
	for task in tasks {
		scheduler.enqueue(task)?;
	}

	// Process tasks in batches of 3
	loop {
		let batch = scheduler.dequeue_batch(3)?;
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

fn blocking_dequeue_example() -> Result<(), RedisError> {
	let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;

	// Spawn a thread that will add tasks after a delay
	thread::spawn(move || {
		thread::sleep(Duration::from_secs(2));

		let mut delayed_scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin).expect("Failed to create scheduler");

		let delayed_task = Task::new("delayed_task".to_string(), 8, SystemTime::now() + Duration::from_secs(3600), Duration::from_secs(5));

		delayed_scheduler.enqueue(delayed_task).expect("Failed to enqueue delayed task");
	});

	// Try to dequeue with a timeout of 3 seconds
	println!("Waiting for tasks...");
	match scheduler.dequeue_blocking(3.0)? {
		Some(task) => println!("Received delayed task: {}", task.id),
		None => println!("Timeout reached, no tasks received"),
	}

	Ok(())
}

// Function to demonstrate task expiration and pattern matching
pub fn advanced_examples() -> Result<(), RedisError> {
	println!("\nRunning Advanced Examples...");

	let mut scheduler = RedisScheduler::new("redis://127.0.0.1/", SchedulerType::RoundRobin)?;

	// Create a task with expiration
	let task = Task::new("expiring_task".to_string(), 8, SystemTime::now() + Duration::from_secs(3600), Duration::from_secs(10));

	// Set task to expire after 5 seconds
	scheduler.enqueue(task.clone())?;
	scheduler.set_expiration(&task.id, Duration::from_secs(5))?;

	// Wait for expiration
	thread::sleep(Duration::from_secs(6));

	// Try to retrieve expired task
	let tasks = scheduler.get_tasks_by_pattern(&format!("*{}*", task.id))?;
	println!("Found {} tasks matching pattern", tasks.len());

	Ok(())
}
