use super::types::{SceneConfig, SceneId, TimeMs};
use serde::{Deserialize, Serialize};

/// A scheduled element representing when a scene should be active
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledElement {
	pub id: SceneId,
	pub scene_name: String,
	pub start_time: TimeMs,
	pub end_time: TimeMs,
	pub duration: TimeMs,
	pub is_active: bool,
}

impl ScheduledElement {
	pub fn new(scene: &SceneConfig, start_time: TimeMs) -> Self {
		let duration = scene.duration;
		Self {
			id: scene.id(),
			scene_name: scene.scene_name.clone(),
			start_time,
			end_time: start_time + duration,
			duration,
			is_active: false,
		}
	}

	pub fn is_active_at(&self, time: TimeMs) -> bool {
		time >= self.start_time && time < self.end_time
	}

	pub fn time_until_start(&self, current_time: TimeMs) -> Option<TimeMs> {
		if current_time < self.start_time {
			Some(self.start_time - current_time)
		} else {
			None
		}
	}

	pub fn time_remaining(&self, current_time: TimeMs) -> Option<TimeMs> {
		if current_time >= self.start_time && current_time < self.end_time {
			Some(self.end_time - current_time)
		} else {
			None
		}
	}
}

/// Manages the schedule of scenes throughout the stream
#[derive(Debug, Clone)]
pub struct SceneSchedule {
	elements: Vec<ScheduledElement>,
	total_duration: TimeMs,
}

impl SceneSchedule {
	pub fn new() -> Self {
		Self {
			elements: Vec::new(),
			total_duration: 0,
		}
	}

	pub fn from_scenes(scenes: &[SceneConfig]) -> Self {
		let mut schedule = Self::new();
		let mut accumulated_time = 0;

		for scene in scenes {
			schedule.add_element(scene, accumulated_time);
			accumulated_time += scene.duration;
		}

		schedule.total_duration = accumulated_time;
		schedule
	}

	pub fn add_element(&mut self, scene: &SceneConfig, start_time: TimeMs) {
		let element = ScheduledElement::new(scene, start_time);
		self.total_duration = self.total_duration.max(element.end_time);
		self.elements.push(element);
	}

	pub fn get_active_elements(&self, current_time: TimeMs) -> Vec<&ScheduledElement> {
		self.elements.iter().filter(|e| e.is_active_at(current_time)).collect()
	}

	pub fn get_current_scene(&self, current_time: TimeMs) -> Option<&ScheduledElement> {
		self.elements.iter().find(|e| e.is_active_at(current_time))
	}

	pub fn get_next_scene(&self, current_time: TimeMs) -> Option<&ScheduledElement> {
		self.elements.iter().filter(|e| e.start_time > current_time).min_by_key(|e| e.start_time)
	}

	pub fn get_scene_by_name(&self, scene_name: &str) -> Option<&ScheduledElement> {
		self.elements.iter().find(|e| e.scene_name == scene_name)
	}

	pub fn get_scene_index(&self, scene_name: &str) -> Option<usize> {
		self.elements.iter().position(|e| e.scene_name == scene_name)
	}

	pub fn elements(&self) -> &[ScheduledElement] {
		&self.elements
	}

	pub fn total_duration(&self) -> TimeMs {
		self.total_duration
	}

	pub fn is_complete(&self, current_time: TimeMs) -> bool {
		current_time >= self.total_duration
	}

	pub fn clear(&mut self) {
		self.elements.clear();
		self.total_duration = 0;
	}

	pub fn len(&self) -> usize {
		self.elements.len()
	}

	pub fn is_empty(&self) -> bool {
		self.elements.is_empty()
	}
}

impl Default for SceneSchedule {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn create_test_scenes() -> Vec<SceneConfig> {
		vec![SceneConfig::new("intro", 5000), SceneConfig::new("main", 10000), SceneConfig::new("outro", 3000)]
	}

	#[test]
	fn test_schedule_creation() {
		let scenes = create_test_scenes();
		let schedule = SceneSchedule::from_scenes(&scenes);

		assert_eq!(schedule.len(), 3);
		assert_eq!(schedule.total_duration(), 18000);
	}

	#[test]
	fn test_active_elements() {
		let scenes = create_test_scenes();
		let schedule = SceneSchedule::from_scenes(&scenes);

		// At 0ms, intro should be active
		let active = schedule.get_current_scene(0);
		assert_eq!(active.map(|s| s.scene_name.as_str()), Some("intro"));

		// At 6000ms, main should be active
		let active = schedule.get_current_scene(6000);
		assert_eq!(active.map(|s| s.scene_name.as_str()), Some("main"));

		// At 16000ms, outro should be active
		let active = schedule.get_current_scene(16000);
		assert_eq!(active.map(|s| s.scene_name.as_str()), Some("outro"));

		// At 20000ms, nothing should be active
		let active = schedule.get_current_scene(20000);
		assert!(active.is_none());
	}

	#[test]
	fn test_next_scene() {
		let scenes = create_test_scenes();
		let schedule = SceneSchedule::from_scenes(&scenes);

		let next = schedule.get_next_scene(0);
		assert_eq!(next.map(|s| s.scene_name.as_str()), Some("main"));

		let next = schedule.get_next_scene(6000);
		assert_eq!(next.map(|s| s.scene_name.as_str()), Some("outro"));

		let next = schedule.get_next_scene(20000);
		assert!(next.is_none());
	}

	#[test]
	fn test_is_complete() {
		let scenes = create_test_scenes();
		let schedule = SceneSchedule::from_scenes(&scenes);

		assert!(!schedule.is_complete(0));
		assert!(!schedule.is_complete(10000));
		assert!(schedule.is_complete(18000));
		assert!(schedule.is_complete(20000));
	}
}
