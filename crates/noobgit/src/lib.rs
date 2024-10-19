use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;

mod debouncer;
pub mod error;
pub mod file_system;
pub mod registry;

use debouncer::Debouncer;
use file_system::FileSystem;
use registry::{Change, ChangeType, Registry};

pub struct NoobGit {
	root: PathBuf,
	file_system: FileSystem,
	registry: Registry,
}

impl NoobGit {
	pub async fn new<P: AsRef<Path>>(root: P) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
		let root = root.as_ref().to_path_buf();
		let file_system = FileSystem::new(&root).await?;
		let registry = Registry::new();

		Ok(Self { root, file_system, registry })
	}

	pub async fn start_watching(noob_git: Arc<Mutex<NoobGit>>, mut stop_receiver: mpsc::Receiver<()>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		println!("NoobGit: Watch began!");

		let (tx, mut rx) = mpsc::channel(100);

		// Clone necessary references
		let root_clone = noob_git.lock().await.root.clone();
		let noob_git_clone = Arc::clone(&noob_git);

		// Watcher task: Spawn in a blocking thread
		let watcher_handle = tokio::task::spawn_blocking(move || {
			let mut watcher = RecommendedWatcher::new(
				move |res: Result<Event, notify::Error>| match res {
					Ok(event) => {
						println!("Received event: {:?}", event);
						if let Err(e) = tx.blocking_send(event) {
							eprintln!("Error sending event: {:?}", e);
						}
					}
					Err(e) => eprintln!("Error watching directory: {:?}", e),
				},
				notify::Config::default(),
			)
			.unwrap();

			// Start watching the directory (recursive)
			if let Err(e) = watcher.watch(&root_clone, RecursiveMode::Recursive) {
				eprintln!("Error starting watcher: {:?}", e);
			}

			watcher
		});

		// Debouncer setup
		let debouncer = Arc::new(Debouncer::new(Duration::from_millis(500)));
		let debouncer_clone = Arc::clone(&debouncer);

		// Debouncer task
		let debouncer_handle = tokio::spawn(async move {
			debouncer_clone.debounce().await;
		});

		// Event handling task
		let event_handle = tokio::spawn(async move {
			loop {
				tokio::select! {
						Some(event) = rx.recv() => {
								println!("Raw event: {:?}", event);

								// Lock NoobGit and process the event if it's debounced
								let mut noob_git = noob_git_clone.lock().await;
								if debouncer.bump() {
										noob_git.handle_event(&event).await;
								}
						}
						_ = stop_receiver.recv() => {
								println!("NoobGit: Stop signal received. Exiting watch...");
								break;
						}
				}
			}
		});

		// Wait for the event handling task to complete
		event_handle.await?;

		// Clean up tasks
		watcher_handle.abort();
		debouncer_handle.abort();

		println!("NoobGit: Watch ended");
		Ok(())
	}

	// Handle the event based on the event kind
	async fn handle_event(&mut self, event: &Event) {
		for path in &event.paths {
			match event.kind {
				notify::EventKind::Create(_) => {
					if let Err(e) = self.file_system.add(path).await {
						println!("Error adding file: {:?}", e);
					}
					self.registry.add_change(Change::new(ChangeType::Create, path.clone()));
					println!("NoobGit: Create event handled for {:?}", path);
				}
				notify::EventKind::Modify(_) => {
					if let Err(e) = self.file_system.add(path).await {
						println!("Error updating file: {:?}", e);
					}
					self.registry.add_change(Change::new(ChangeType::Modify, path.clone()));
					println!("NoobGit: Modify event handled for {:?}", path);
				}
				notify::EventKind::Remove(_) => {
					if let Err(e) = self.file_system.remove(path).await {
						println!("Error removing file: {:?}", e);
					}
					self.registry.add_change(Change::new(ChangeType::Delete, path.clone()));
					println!("NoobGit: Delete event handled for {:?}", path);
				}
				_ => {
					println!("NoobGit: Unhandled event kind: {:?}", event.kind);
				}
			}
		}
	}

	// Other methods for staging/unstaging changes and notifications
	pub fn stage_changes(&mut self) {
		self.registry.stage_changes();
	}

	pub fn unstage_changes(&mut self) {
		self.registry.unstage_changes();
	}

	pub fn get_notifications(&self) -> Vec<String> {
		self.registry.get_notifications()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use notify::{
		event::{CreateKind, RemoveKind},
		Event, EventKind,
	};
	use tempfile::TempDir;
	use tokio::fs;

	async fn setup_test_dir() -> TempDir {
		TempDir::new().expect("Failed to create temp directory")
	}

	#[tokio::test]
	async fn test_noobgit_new() {
		let temp_dir = setup_test_dir().await;
		let result = NoobGit::new(temp_dir.path()).await;
		assert!(result.is_ok());

		let noobgit = result.unwrap();
		assert_eq!(noobgit.root, temp_dir.path());
	}

	#[tokio::test]
	async fn test_start_watching_terminates_on_stop_signal() {
		let temp_dir = setup_test_dir().await;
		let root = temp_dir.path().to_path_buf();

		let noob_git = Arc::new(Mutex::new(NoobGit::new(&root).await.unwrap()));
		let (stop_tx, stop_rx) = mpsc::channel(1);

		// Start watching in a separate task
		let watch_handle = tokio::spawn({
			let noob_git = Arc::clone(&noob_git);
			async move { NoobGit::start_watching(noob_git, stop_rx).await }
		});

		// Wait a short time to ensure watching has started
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Send stop signal
		stop_tx.send(()).await.unwrap();

		// Wait for the watch task to finish
		let result = tokio::time::timeout(Duration::from_secs(5), watch_handle).await;

		assert!(result.is_ok(), "start_watching did not terminate within 5 seconds after stop signal");
		assert!(result.unwrap().is_ok(), "start_watching returned an error");
	}

	#[tokio::test]
	async fn test_watcher_handles_file_creation() {
		let temp_dir = setup_test_dir().await;
		let root = temp_dir.path().to_path_buf();

		let noob_git = Arc::new(Mutex::new(NoobGit::new(&root).await.unwrap()));
		let (stop_tx, stop_rx) = mpsc::channel(1);

		// Start watching in a separate task
		let watch_handle = tokio::spawn({
			let noob_git = Arc::clone(&noob_git);
			async move { NoobGit::start_watching(noob_git, stop_rx).await }
		});

		// Wait a short time to ensure watching has started
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Create a new file
		let file_path = root.join("test_file.txt");
		tokio::fs::write(&file_path, "Hello, NoobGit!").await.unwrap();

		// Wait for the event to be processed
		tokio::time::sleep(Duration::from_secs(1)).await;

		// Stop watching
		stop_tx.send(()).await.unwrap();
		watch_handle.await.unwrap().unwrap();

		// Check if the file creation was detected
		let notifications = noob_git.lock().await.get_notifications();
		assert!(
			notifications.iter().any(|n| n.contains("Create") && n.contains("test_file.txt")),
			"File creation was not detected"
		);
	}

	#[tokio::test]
	async fn test_watcher_handles_file_modification() {
		let temp_dir = setup_test_dir().await;
		let root = temp_dir.path().to_path_buf();

		let noob_git = Arc::new(Mutex::new(NoobGit::new(&root).await.unwrap()));
		let (stop_tx, stop_rx) = mpsc::channel(1);

		// Create a file before starting the watcher
		let file_path = root.join("test_file.txt");
		tokio::fs::write(&file_path, "Initial content").await.unwrap();

		// Start watching in a separate task
		let watch_handle = tokio::spawn({
			let noob_git = Arc::clone(&noob_git);
			async move { NoobGit::start_watching(noob_git, stop_rx).await }
		});

		// Wait a short time to ensure watching has started
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Modify the file
		tokio::fs::write(&file_path, "Modified content").await.unwrap();

		// Wait for the event to be processed
		tokio::time::sleep(Duration::from_secs(1)).await;

		// Stop watching
		stop_tx.send(()).await.unwrap();
		watch_handle.await.unwrap().unwrap();

		// Check if the file modification was detected
		let notifications = noob_git.lock().await.get_notifications();
		assert!(
			notifications.iter().any(|n| n.contains("Modify") && n.contains("test_file.txt")),
			"File modification was not detected"
		);
	}

	#[tokio::test]
	async fn test_watcher_handles_file_deletion() {
		let temp_dir = setup_test_dir().await;
		let root = temp_dir.path().to_path_buf();

		let noob_git = Arc::new(Mutex::new(NoobGit::new(&root).await.unwrap()));
		let (stop_tx, stop_rx) = mpsc::channel(1);

		// Create a file before starting the watcher
		let file_path = root.join("test_file.txt");
		tokio::fs::write(&file_path, "Content to be deleted").await.unwrap();

		// Start watching in a separate task
		let watch_handle = tokio::spawn({
			let noob_git = Arc::clone(&noob_git);
			async move { NoobGit::start_watching(noob_git, stop_rx).await }
		});

		// Wait a short time to ensure watching has started
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Delete the file
		tokio::fs::remove_file(&file_path).await.unwrap();

		// Wait for the event to be processed
		tokio::time::sleep(Duration::from_secs(1)).await;

		// Stop watching
		stop_tx.send(()).await.unwrap();
		watch_handle.await.unwrap().unwrap();

		// Check if the file deletion was detected
		let notifications = noob_git.lock().await.get_notifications();
		assert!(
			notifications.iter().any(|n| n.contains("Delete") && n.contains("test_file.txt")),
			"File deletion was not detected"
		);
	}

	#[tokio::test]
	async fn test_multiple_file_operations() {
		let temp_dir = setup_test_dir().await;
		let root = temp_dir.path().to_path_buf();

		let noob_git = Arc::new(Mutex::new(NoobGit::new(&root).await.unwrap()));
		let (stop_tx, stop_rx) = mpsc::channel(1);

		// Start watching in a separate task
		let watch_handle = tokio::spawn({
			let noob_git = Arc::clone(&noob_git);
			async move { NoobGit::start_watching(noob_git, stop_rx).await }
		});

		// Wait a short time to ensure watching has started
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Perform multiple file operations
		let file1_path = root.join("file1.txt");
		let file2_path = root.join("file2.txt");

		tokio::fs::write(&file1_path, "File 1 content").await.unwrap();
		tokio::fs::write(&file2_path, "File 2 content").await.unwrap();
		tokio::fs::write(&file1_path, "File 1 modified").await.unwrap();
		tokio::fs::remove_file(&file2_path).await.unwrap();

		// Wait for events to be processed
		tokio::time::sleep(Duration::from_secs(2)).await;

		// Stop watching
		stop_tx.send(()).await.unwrap();
		watch_handle.await.unwrap().unwrap();

		// Check if all operations were detected
		let notifications = noob_git.lock().await.get_notifications();
		assert!(
			notifications.iter().any(|n| n.contains("Create") && n.contains("file1.txt")),
			"File1 creation was not detected"
		);
		assert!(
			notifications.iter().any(|n| n.contains("Create") && n.contains("file2.txt")),
			"File2 creation was not detected"
		);
		assert!(
			notifications.iter().any(|n| n.contains("Modify") && n.contains("file1.txt")),
			"File1 modification was not detected"
		);
		assert!(
			notifications.iter().any(|n| n.contains("Delete") && n.contains("file2.txt")),
			"File2 deletion was not detected"
		);
	}
}
