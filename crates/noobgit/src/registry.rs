use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum ChangeType {
	Create,
	Delete,
}

#[derive(Debug, Clone)]
pub struct Change {
	change_type: ChangeType,
	path: PathBuf,
}

impl Change {
	pub fn new(change_type: ChangeType, path: PathBuf) -> Self {
		Self { change_type, path }
	}
}

pub struct Registry {
	unstaged_changes: VecDeque<Change>,
	staged_changes: Vec<Change>,
	max_changes: usize,
}

impl Registry {
	pub fn new() -> Self {
		Self {
			unstaged_changes: VecDeque::new(),
			staged_changes: Vec::new(),
			max_changes: 5,
		}
	}

	pub fn add_change(&mut self, change: Change) {
		if self.unstaged_changes.len() >= self.max_changes {
			self.unstaged_changes.pop_front();
		}
		self.unstaged_changes.push_back(change);
	}

	pub fn stage_changes(&mut self) {
		self.staged_changes.extend(self.unstaged_changes.drain(..));
	}

	pub fn unstage_changes(&mut self) {
		self.unstaged_changes.extend(self.staged_changes.drain(..));
	}

	pub fn get_notifications(&self) -> Vec<String> {
		self
			.unstaged_changes
			.iter()
			.map(|change| {
				let action = match change.change_type {
					ChangeType::Create => "Created",
					ChangeType::Delete => "Deleted",
				};
				format!("{} {}", action, change.path.display())
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_new_registry() {
		let registry = Registry::new();
		assert_eq!(registry.unstaged_changes.len(), 0);
		assert_eq!(registry.staged_changes.len(), 0);
		assert_eq!(registry.max_changes, 5);
	}

	#[test]
	fn test_add_change() {
		let mut registry = Registry::new();
		let change = Change::new(ChangeType::Create, PathBuf::from("/test/file.txt"));
		registry.add_change(change);
		assert_eq!(registry.unstaged_changes.len(), 1);
	}

	#[test]
	fn test_add_change_max_limit() {
		let mut registry = Registry::new();
		for i in 0..6 {
			let change = Change::new(ChangeType::Create, PathBuf::from(format!("/test/file{}.txt", i)));
			registry.add_change(change);
		}
		assert_eq!(registry.unstaged_changes.len(), 5);
		assert_eq!(registry.unstaged_changes.front().unwrap().path, PathBuf::from("/test/file1.txt"));
	}

	#[test]
	fn test_stage_changes() {
		let mut registry = Registry::new();
		registry.add_change(Change::new(ChangeType::Create, PathBuf::from("/test/file1.txt")));
		registry.add_change(Change::new(ChangeType::Delete, PathBuf::from("/test/file2.txt")));
		registry.stage_changes();
		assert_eq!(registry.unstaged_changes.len(), 0);
		assert_eq!(registry.staged_changes.len(), 2);
	}

	#[test]
	fn test_unstage_changes() {
		let mut registry = Registry::new();
		registry.add_change(Change::new(ChangeType::Create, PathBuf::from("/test/file1.txt")));
		registry.add_change(Change::new(ChangeType::Delete, PathBuf::from("/test/file2.txt")));
		registry.stage_changes();
		registry.unstage_changes();
		assert_eq!(registry.unstaged_changes.len(), 2);
		assert_eq!(registry.staged_changes.len(), 0);
	}

	#[test]
	fn test_get_notifications() {
		let mut registry = Registry::new();
		registry.add_change(Change::new(ChangeType::Create, PathBuf::from("/test/file1.txt")));
		registry.add_change(Change::new(ChangeType::Delete, PathBuf::from("/test/file2.txt")));
		let notifications = registry.get_notifications();
		assert_eq!(notifications.len(), 2);
		assert_eq!(notifications[0], "Created /test/file1.txt");
		assert_eq!(notifications[1], "Deleted /test/file2.txt");
	}
}
