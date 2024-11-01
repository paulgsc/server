#[derive(Clone, Copy, Debug)]
pub enum SchedulerType {
	RoundRobin(RoundRobinConfig),
	EDF,
}

#[derive(Clone, Copy, Debug)]
pub struct RoundRobinConfig {
	high_priority_weight: usize,
	medium_priority_weight: usize,
	low_priority_weight: usize,
}

impl Default for RoundRobinConfig {
	fn default() -> Self {
		Self {
			high_priority_weight: 4,
			medium_priority_weight: 2,
			low_priority_weight: 1,
		}
	}
}

impl RoundRobinConfig {
	pub fn get_weights(&self) -> Vec<usize> {
		vec![self.high_priority_weight, self.medium_priority_weight, self.low_priority_weight]
	}

	pub fn get_queue_keys() -> Vec<String> {
		vec!["scheduler:high".to_string(), "scheduler:medium".to_string(), "scheduler:low".to_string()]
	}
}
