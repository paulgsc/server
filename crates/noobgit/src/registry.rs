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
