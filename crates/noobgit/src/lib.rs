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
	pub async fn new<P: AsRef<Path>>(root: P) -> Result<Self, Box<dyn std::error::Error>> {
		let root = root.as_ref().to_path_buf();
		let file_system = FileSystem::new(&root).await?;
		let registry = Registry::new();

		Ok(Self { root, file_system, registry })
	}

	pub async fn start_watching(noob_git: Arc<Mutex<NoobGit>>, mut stop_receiver: mpsc::Receiver<()>) -> Result<(), Box<dyn std::error::Error>> {
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

			// Keep the watcher alive
			loop {
				std::thread::sleep(Duration::from_secs(1));
			}
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
			while let Some(event) = rx.recv().await {
				println!("Raw event: {:?}", event);

				// Lock NoobGit and process the event if it's debounced
				let mut noob_git = noob_git_clone.lock().await;
				if debouncer.bump() {
					noob_git.handle_event(&event).await;
				}
			}
		});

		// Main loop to listen for stop signals
		loop {
			tokio::select! {
					_ = stop_receiver.recv() => {
							println!("NoobGit: Stop signal received. Exiting watch...");
							break;
					}
			}
		}

		// Clean up tasks
		watcher_handle.abort();
		debouncer_handle.abort();
		event_handle.abort();

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
	async fn test_handle_create_event() {
		let temp_dir = setup_test_dir().await;
		let mut noobgit = NoobGit::new(temp_dir.path()).await.unwrap();

		let test_file = temp_dir.path().join("test_file.txt");
		fs::write(&test_file, "test content").await.unwrap();

		let event = Event {
			kind: EventKind::Create(CreateKind::File),
			paths: vec![test_file.clone()],
			..Default::default() // Fill in other fields as necessary
		};
		noobgit.handle_event(&event).await;

		assert_eq!(noobgit.registry.unstaged_changes.len(), 1);
		assert_eq!(noobgit.registry.unstaged_changes[0].change_type, ChangeType::Create);
		assert_eq!(noobgit.registry.unstaged_changes[0].path, test_file);
	}

	#[tokio::test]
	async fn test_handle_remove_event() {
		let temp_dir = setup_test_dir().await;
		let mut noobgit = NoobGit::new(temp_dir.path()).await.unwrap();

		let test_file = temp_dir.path().join("test_file.txt");
		fs::write(&test_file, "test content").await.unwrap();
		noobgit.file_system.add(&test_file).await.unwrap();

		fs::remove_file(&test_file).await.unwrap();

		let event = Event {
			kind: EventKind::Remove(RemoveKind::File),
			paths: vec![test_file.clone()],
			..Default::default() // Fill in other fields as necessary
		};
		noobgit.handle_event(&event).await;

		assert_eq!(noobgit.registry.unstaged_changes.len(), 1);
		assert_eq!(noobgit.registry.unstaged_changes[0].change_type, ChangeType::Delete);
		assert_eq!(noobgit.registry.unstaged_changes[0].path, test_file);
	}

	#[tokio::test]
	async fn test_stage_and_unstage_changes() {
		let temp_dir = setup_test_dir().await;
		let mut noobgit = NoobGit::new(temp_dir.path()).await.unwrap();

		let test_file = temp_dir.path().join("test_file.txt");
		fs::write(&test_file, "test content").await.unwrap();

		let event = Event {
			kind: EventKind::Create(CreateKind::File),
			paths: vec![test_file],
			..Default::default() // Fill in other fields as necessary
		};
		noobgit.handle_event(&event).await;

		assert_eq!(noobgit.registry.unstaged_changes.len(), 1);
		assert_eq!(noobgit.registry.staged_changes.len(), 0);

		noobgit.stage_changes();
		assert_eq!(noobgit.registry.unstaged_changes.len(), 0);
		assert_eq!(noobgit.registry.staged_changes.len(), 1);

		noobgit.unstage_changes();
		assert_eq!(noobgit.registry.unstaged_changes.len(), 1);
		assert_eq!(noobgit.registry.staged_changes.len(), 0);
	}

	#[tokio::test]
	async fn test_get_notifications() {
		let temp_dir = setup_test_dir().await;
		let mut noobgit = NoobGit::new(temp_dir.path()).await.unwrap();

		let test_file = temp_dir.path().join("test_file.txt");
		fs::write(&test_file, "test content").await.unwrap();

		let event = Event {
			kind: EventKind::Create(CreateKind::File),
			paths: vec![test_file],
			..Default::default() // Fill in other fields as necessary
		};
		noobgit.handle_event(&event).await;

		let notifications = noobgit.get_notifications();
		assert_eq!(notifications.len(), 1);
		assert!(notifications[0].contains("Created"));
	}
}
