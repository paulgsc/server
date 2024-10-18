use futures::StreamExt;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

mod debouncer;
pub mod error;
mod file_system;
mod registry;

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

	pub async fn start_watching(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let (tx, mut rx) = mpsc::channel(100);
		let mut watcher = RecommendedWatcher::new(
			move |res: Result<Event, notify::Error>| {
				if let Ok(event) = res {
					let _ = futures::executor::block_on(tx.send(event));
				}
			},
			notify::Config::default(),
		)?;

		watcher.watch(&self.root, RecursiveMode::Recursive)?;

		let debouncer = Arc::new(Debouncer::new(std::time::Duration::from_secs(2)));
		let debouncer_clone = debouncer.clone();

		tokio::spawn(async move {
			debouncer_clone.debounce().await;
		});

		while let Some(event) = rx.recv().await {
			if debouncer.bump() {
				self.handle_event(&event).await;
			} else {
				break;
			}
		}

		Ok(())
	}

	async fn handle_event(&mut self, event: &Event) {
		for path in &event.paths {
			match event.kind {
				notify::EventKind::Create(_) => {
					if let Err(e) = self.file_system.add(path).await {
						println!("Error adding file: {:?}", e);
					}
					self.registry.add_change(Change::new(ChangeType::Create, path.clone()));
				}
				notify::EventKind::Remove(_) => {
					if let Err(e) = self.file_system.remove(path).await {
						println!("Error removing file: {:?}", e);
					}
					self.registry.add_change(Change::new(ChangeType::Delete, path.clone()));
				}
				_ => {}
			}
		}
	}

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