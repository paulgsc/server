#![allow(clippy::borrowed_box)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::disallowed_macros)]
#![allow(clippy::disallowed_methods)]
#![allow(clippy::match_bool)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::multiple_bound_locations)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::question_mark)]
#![allow(clippy::self_only_used_in_recursion)]
#![allow(clippy::single_match_else)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::unnecessary_box_returns)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::new_without_default)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::struct_field_names)]
#![allow(clippy::type_repetition_in_bounds)]
#![allow(clippy::use_self)]
#![allow(clippy::unwrap_used)]
pub mod config;
pub mod error;
pub mod pool;
pub mod priority_queue;
pub mod redis_queue;
pub mod scheduler_types;
pub mod worker;

pub mod trees;

use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{broadcast, RwLock};
use tokio::time;
use uuid::Uuid;

// Task status enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum TaskStatus {
	Scheduled,
	Running,
	Completed,
	Failed,
}

// Task definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
	id: Uuid,
	name: String,
	schedule_time: DateTime<Utc>,
	status: TaskStatus,
	payload: serde_json::Value,
}

// Task request for API
#[derive(Debug, Deserialize)]
pub struct ScheduleTaskRequest {
	name: String,
	schedule_time: DateTime<Utc>,
	payload: serde_json::Value,
}

// Scheduler state
pub struct Scheduler {
	pub tasks: RwLock<HashMap<Uuid, Task>>,
	pub task_tx: broadcast::Sender<Task>,
}

impl Scheduler {
	pub fn new() -> Self {
		let (task_tx, _) = broadcast::channel(100);
		Self {
			tasks: RwLock::new(HashMap::new()),
			task_tx,
		}
	}
}

// API handlers
pub async fn schedule_task(State(scheduler): State<Arc<Scheduler>>, Json(request): Json<ScheduleTaskRequest>) -> Json<Task> {
	let task = Task {
		id: Uuid::new_v4(),
		name: request.name,
		schedule_time: request.schedule_time,
		status: TaskStatus::Scheduled,
		payload: request.payload,
	};

	// Store task
	scheduler.tasks.write().await.insert(task.id, task.clone());

	// Notify scheduler
	let _ = scheduler.task_tx.send(task.clone());

	Json(task)
}

// Task processor function
async fn process_task(task: &mut Task) {
	// Simulate task processing
	println!("Processing task: {}", task.name);
	time::sleep(Duration::from_secs(2)).await;
	task.status = TaskStatus::Completed;
}

// Background scheduler
pub async fn run_scheduler(scheduler: Arc<Scheduler>) {
	let mut task_rx = scheduler.task_tx.subscribe();

	loop {
		tokio::select! {
				Ok(task) = task_rx.recv() => {
						let scheduler_clone = scheduler.clone();

						// Spawn a new task for handling the scheduled job
						tokio::spawn(async move {
								let now = Utc::now();
								if task.schedule_time <= now {
										// Process immediately if schedule time has passed
										let mut tasks = scheduler_clone.tasks.write().await;
										if let Some(task) = tasks.get_mut(&task.id) {
												task.status = TaskStatus::Running;
												process_task(task).await;
										}
								} else {
										// Schedule for future execution
										let delay = task.schedule_time.timestamp() - now.timestamp();
										if delay > 0 {
												time::sleep(Duration::from_secs(delay as u64)).await;
												let mut tasks = scheduler_clone.tasks.write().await;
												if let Some(task) = tasks.get_mut(&task.id) {
														task.status = TaskStatus::Running;
														process_task(task).await;
												}
										}
								}
						});
				}
		}
	}
}
