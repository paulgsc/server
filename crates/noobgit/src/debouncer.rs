use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use notify::Event;

#[derive(Debug, Clone)]
pub enum DebouncedEvent {
    Create(PathBuf),
    Remove(PathBuf),
}

pub struct Debouncer {
    delay: Duration,
    last_events: HashMap<PathBuf, (Instant, Event)>,
}

impl Debouncer {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            last_events: HashMap::new(),
        }
    }

    pub fn debounce(&mut self, event: Event) -> Vec<DebouncedEvent> {
        let now = Instant::now();
        let mut debounced_events = Vec::new();

        for path in &event.paths {
            let entry = self
                .last_events
                .entry(path.clone())
                .or_insert_with(|| (now, event.clone()));

            if now.duration_since(entry.0) >= self.delay {
                match &event.kind {
                    notify::EventKind::Create(_) => {
                        debounced_events.push(DebouncedEvent::Create(path.clone()));
                    }
                    notify::EventKind::Remove(_) => {
                        debounced_events.push(DebouncedEvent::Remove(path.clone()));
                    }
                    _ => {}
                }
                entry.0 = now;
                entry.1 = event.clone();
            }
        }

        debounced_events
    }
}

