use file_reader::core::Path as ValidatedPath;
use noobgit::NoobGit;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let user_input_path = "/demo";

	match ValidatedPath::parse(user_input_path) {
		Ok(valid_path) => {
			let path_buf = PathBuf::from(valid_path.as_ref());
			watch_directory(path_buf).await?;
		}
		Err(e) => {
			eprintln!("invalid path: {:?}", e);
			return Err(Box::new(e) as Box<dyn std::error::Error>);
		}
	}

	Ok(())
}

async fn watch_directory(root_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
	println!("watching directory: {:?}", root_path);

	let noob_git = Arc::new(Mutex::new(NoobGit::new(&root_path).await.unwrap()));

	let (stop_tx, stop_rx) = mpsc::channel(1);

	let noobgit_clone = Arc::clone(&noob_git);
	let watching_handle = tokio::spawn(async move {
		println!("Step 2: Starting watcher");
		let mut noobgit = noobgit_clone.lock().await;
		if let Err(e) = noobgit.start_watching(stop_rx).await {
			eprintln!("Error starting watcher: {:?}", e);
		}
	});

	loop {
		tokio::time::sleep(Duration::from_secs(1)).await;
		println!("Watcher is running...");
	}

	watching_handle.await.unwrap();

	Ok(())
}
