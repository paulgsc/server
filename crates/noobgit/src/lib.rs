use std::path::{Path, PathBuf};
use std::collections::VecDeque;
use notify::{Watcher, RecursiveMode, watcher, DebouncedEvent};
use std::sync::mpsc::channel;
use std::time::Duration;

mod file_system;
mod registry;

use file_system::FileSystem;
use registry::{Registry, Change, ChangeType};

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
        let mut watcher = watcher(tx, Duration::from_secs(2))?;
        watcher.watch(&self.root, RecursiveMode::Recursive)?;

        loop {
            match rx.recv() {
                Ok(event) => self.handle_event(event),
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
            _ => {},
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
