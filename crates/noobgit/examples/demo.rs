use file_reader::core::Path as ValidatedPath;
use lazy_static::lazy_static;
use noobgit::NoobGit;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;

lazy_static! {
	static ref DEBOUNCER_DURATION: Duration = Duration::from_millis(500);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
	let user_input_path = "/demo";

	match ValidatedPath::parse(user_input_path) {
		Ok(valid_path) => {
			let path_buf = PathBuf::from(valid_path.as_ref());
			watch_directory(path_buf).await?;
		}
		Err(e) => {
			eprintln!("invalid path: {:?}", e);
			return Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
		}
	}

	Ok(())
}

async fn watch_directory(root_path: PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
	println!("watching directory: {:?}", root_path);

	let noob_git = Arc::new(Mutex::new(NoobGit::new(&root_path, *DEBOUNCER_DURATION).await.unwrap()));

	let (stop_tx, stop_rx) = mpsc::channel(1); // Unused stop_tx, but can be used to stop watcher.

	let noob_git_clone = Arc::clone(&noob_git);

	let watching_handle = tokio::spawn(async move {
		println!("Step 2: Starting watcher");
		if let Err(e) = NoobGit::start_watching(noob_git_clone, stop_rx).await {
			eprintln!("Error starting watcher: {:?}", e);
		}
	});

	let work_duration = Duration::from_secs(60);
	tokio::time::sleep(work_duration).await;
	stop_tx.send(()).await?;
	let _ = watching_handle.await;

	let notifications = noob_git.lock().await.get_notifications();
	println!("Final notifications:");
	for notification in notifications {
		println!("  {}", notification);
	}

	Ok(())
}
