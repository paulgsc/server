use axum::{routing::post, Router};
use std::error::Error;
use std::sync::Arc;
use task_queue::{run_scheduler, schedule_task, Scheduler};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
	// Initialize scheduler
	let scheduler = Arc::new(Scheduler::new());
	let scheduler_clone = scheduler.clone();

	// Start background scheduler
	tokio::spawn(async move {
		run_scheduler(scheduler_clone).await;
	});

	// Setup Axum router
	let app = Router::new().route("/tasks/schedule", post(schedule_task)).with_state(scheduler);

	// Start server
	let listener = TcpListener::bind("127.0.0.1:8000").await?;
	println!("Server running on {}", listener.local_addr()?);
	axum::serve(listener, app).await?;
	Ok(())
}
