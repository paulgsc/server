use std::path::{Path, PathBuf};
use notify::{Watcher, RecursiveMode, Event};
use std::sync::mpsc::channel;
use std::time::Duration;

mod file_system;
mod registry;
mod debouncer;

use file_system::FileSystem;
use registry::{Registry, Change, ChangeType};
use debouncer::{Debouncer, DebouncedEvent};

pub struct MiniGit {
    root: PathBuf,
    file_system: FileSystem,
    registry: Registry,
}

impl MiniGit {
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self, Box<dyn std::error::Error>> {
        let root = root.as_ref().to_path_buf();
        let file_system = FileSystem::new(&root)?;
        let registry = Registry::new();

        Ok(Self {
            root,
            file_system,
            registry,
        })
    }

    pub fn start_watching(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })?;

        watcher.watch(&self.root, RecursiveMode::Recursive)?;

        let mut debouncer = Debouncer::new(Duration::from_secs(2));

        loop {
            match rx.recv() {
                Ok(event) => {
                    let debounced_events = debouncer.debounce(event);
                    for event in debounced_events {
                        self.handle_event(event);
                    }
                },
                Err(e) => println!("Watch error: {:?}", e),
            }
        }
    }

    fn handle_event(&mut self, event: DebouncedEvent) {
        match event {
            DebouncedEvent::Create(path) => {
                if let Err(e) = self.file_system.add(&path) {
                    println!("Error adding file: {:?}", e);
                }
                self.registry.add_change(Change::new(ChangeType::Create, path));
            },
            DebouncedEvent::Remove(path) => {
                if let Err(e) = self.file_system.remove(&path) {
                    println!("Error removing file: {:?}", e);
                }
                self.registry.add_change(Change::new(ChangeType::Delete, path));
            },
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
