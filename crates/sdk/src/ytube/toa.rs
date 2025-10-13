use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Thresholds {
	pub views_impression_ratio: f64,
	pub duration_ratio: f64,
}

#[derive(Debug, Clone)]
pub struct VideoMetrics {
	pub video_id: String,
	pub impressions: u64,
	pub views: u64,
	pub view_duration: u64,
	pub video_length: u64,
	pub current_title: String,
	pub is_private: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum VideoAction {
	DoNothing,
	MakePrivate,
	UpdateTitle(String),
}

pub struct RatioCalculator;

impl RatioCalculator {
	pub fn calculate_view_impression_ratio(metrics: &VideoMetrics) -> Option<f64> {
		if metrics.impressions == 0 {
			return None;
		}
		Some(metrics.views as f64 / metrics.impressions as f64)
	}

	pub fn calculate_duration_ratio(metrics: &VideoMetrics) -> Option<f64> {
		if metrics.video_length == 0 {
			return None;
		}
		Some(metrics.view_duration as f64 / metrics.video_length as f64)
	}
}

pub struct Decision {
	thresholds: Thresholds,
}

impl Decision {
	pub fn new(thresholds: Thresholds) -> Self {
		Self { thresholds }
	}

	pub fn should_make_private(&self, metrics: &VideoMetrics) -> bool {
		if metrics.is_private {
			return false;
		}

		if let Some(views_ratio) = RatioCalculator::calculate_view_impression_ratio(metrics) {
			if views_ratio < self.thresholds.views_impression_ratio {
				return true;
			}
		}

		if let Some(duration_ratio) = RatioCalculator::calculate_duration_ratio(metrics) {
			if duration_ratio < self.thresholds.duration_ratio {
				return true;
			}
		}

		false
	}

	pub fn generate_updated_title(&self, metrics: &VideoMetrics) -> String {
		let original_title = self.extract_original_title(&metrics.current_title);

		format!("{} [{}+ impressions]", original_title, metrics.impressions)
	}

	fn extract_original_title(&self, current_title: &str) -> String {
		let pattern = regex::Regex::new(r"\s*\[\d+\+?\s*impressions?\]$").unwrap();
		pattern.replace(current_title, "").trim().to_string()
	}

	pub fn decide_action(&self, metrics: &VideoMetrics) -> VideoAction {
		if self.should_make_private(metrics) {
			return VideoAction::MakePrivate;
		}

		if !metrics.is_private {
			let new_title = self.generate_updated_title(metrics);
			if new_title != metrics.current_title {
				return VideoAction::UpdateTitle(new_title);
			}
		}

		VideoAction::DoNothing
	}
}

pub struct VideoProcessor {
	decision: Decision,
}

impl VideoProcessor {
	pub fn new(thresholds: Thresholds) -> Self {
		Self {
			decision: Decision::new(thresholds),
		}
	}

	pub fn process_video(&self, metrics: &VideoMetrics) -> VideoAction {
		self.decision.decide_action(metrics)
	}

	pub fn process_batch(&self, videos: &[VideoMetrics]) -> HashMap<String, VideoAction> {
		videos
			.iter()
			.map(|video| {
				let action = self.process_video(video);
				(video.video_id.clone(), action)
			})
			.collect()
	}

	pub fn categorize_actions(&self, actions: &HashMap<String, VideoAction>) -> ActionCategories {
		let mut categories = ActionCategories::default();

		for (video_id, action) in actions {
			match action {
				VideoAction::DoNothing => categories.no_action.push(video_id.clone()),
				VideoAction::MakePrivate => categories.make_private.push(video_id.clone()),
				VideoAction::UpdateTitle(title) => {
					categories.update_title.push((video_id.clone(), title.clone()));
				}
			}
		}

		categories
	}
}

#[derive(Debug, Default)]
pub struct ActionCategories {
	pub no_action: Vec<String>,
	pub make_private: Vec<String>,
	pub update_title: Vec<(String, String)>,
}
