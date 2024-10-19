use noobgit::NoobGit;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let temp_dir = tempfile::tempdir()?;
	let root_path = temp_dir.path().to_path_buf();
	println!("Watching directory: {:?}", root_path);

	let noob_git = Arc::new(Mutex::new(NoobGit::new(&root_path).await?));

	let watch_task = {
		let noob_git = Arc::clone(&noob_git);
		tokio::spawn(async move {
			if let Err(e) = noob_git.lock().await.start_watching().await {
				eprintln!("Error watching directory: {:?}", e);
			}
		})
	};

	simulate_changes(&root_path).await?;

	sleep(Duration::from_secs(3)).await;

	let notifications = noob_git.lock().await.get_notifications();
	println!("Notifications:");
	for notification in notifications {
		println!("  {}", notification);
	}

	temp_dir.close()?;

	Ok(())
}

async fn simulate_changes(root_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
	let file_path = root_path.join("test.txt");
	tokio::fs::File::create(&file_path).await?;
	println!("Created file: {:?}", file_path);

	sleep(Duration::from_secs(1)).await;

	let dir_path = root_path.join("test_dir");
	tokio::fs::create_dir(&dir_path).await?;
	println!("Created directory: {:?}", dir_path);

	sleep(Duration::from_secs(1)).await;

	tokio::fs::remove_file(&file_path).await?;
	println!("Removed file: {:?}", file_path);

	Ok(())
}
