use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use termion::{color, style};
use tokio::sync::RwLock;

// Theme configuration for progress bars
#[derive(Debug, Clone)]
pub struct ProgressTheme {
	pub start_cap: String,
	pub end_cap: String,
	pub bar_filled: String,
	pub bar_empty: String,
	pub spinner_frames: Vec<String>,
	pub colors: ProgressColors,
}

#[derive(Debug, Clone)]
pub struct ProgressColors {
	pub progress: color::Rgb,
	pub background: color::Rgb,
	pub text: color::Rgb,
	pub spinner: color::Rgb,
	pub percentage: color::Rgb,
}

// Built-in themes
impl ProgressTheme {
	pub fn default() -> Self {
		Self {
			start_cap: "[".to_string(),
			end_cap: "]".to_string(),
			bar_filled: "█".to_string(),
			bar_empty: "░".to_string(),
			spinner_frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"].into_iter().map(String::from).collect(),
			colors: ProgressColors {
				progress: color::Rgb(0, 255, 0),
				background: color::Rgb(100, 100, 100),
				text: color::Rgb(200, 200, 200),
				spinner: color::Rgb(255, 255, 0),
				percentage: color::Rgb(255, 255, 255),
			},
		}
	}

	pub fn minimal() -> Self {
		Self {
			start_cap: "|".to_string(),
			end_cap: "|".to_string(),
			bar_filled: "=".to_string(),
			bar_empty: " ".to_string(),
			spinner_frames: vec!["-", "\\", "|", "/"].into_iter().map(String::from).collect(),
			colors: ProgressColors {
				progress: color::Rgb(255, 255, 255),
				background: color::Rgb(100, 100, 100),
				text: color::Rgb(200, 200, 200),
				spinner: color::Rgb(255, 255, 255),
				percentage: color::Rgb(255, 255, 255),
			},
		}
	}

	pub fn fancy() -> Self {
		Self {
			start_cap: "『".to_string(),
			end_cap: "』".to_string(),
			bar_filled: "■".to_string(),
			bar_empty: "□".to_string(),
			spinner_frames: vec!["🌑", "🌒", "🌓", "🌔", "🌕", "🌖", "🌗", "🌘"].into_iter().map(String::from).collect(),
			colors: ProgressColors {
				progress: color::Rgb(100, 255, 100),
				background: color::Rgb(50, 50, 50),
				text: color::Rgb(255, 255, 255),
				spinner: color::Rgb(255, 200, 0),
				percentage: color::Rgb(0, 255, 255),
			},
		}
	}
}

// Progress statistics tracking
#[derive(Debug, Clone)]
pub struct ProgressStats {
	pub start_time: Instant,
	pub last_update: Instant,
	pub current: u64,
	pub total: u64,
	pub bytes_per_sec: f64,
	pub eta: Duration,
	pub message: String,
}

impl ProgressStats {
	fn new(total: u64) -> Self {
		let now = Instant::now();
		Self {
			start_time: now,
			last_update: now,
			current: 0,
			total,
			bytes_per_sec: 0.0,
			eta: Duration::from_secs(0),
			message: String::new(),
		}
	}

	fn update(&mut self, current: u64) {
		let now = Instant::now();
		let elapsed = now.duration_since(self.last_update);

		// Update speed calculations every 100ms
		if elapsed >= Duration::from_millis(100) {
			let delta = current - self.current;
			self.bytes_per_sec = delta as f64 / elapsed.as_secs_f64();

			// Calculate ETA
			if self.bytes_per_sec > 0.0 {
				let remaining = self.total - current;
				self.eta = Duration::from_secs_f64(remaining as f64 / self.bytes_per_sec);
			}

			self.last_update = now;
		}

		self.current = current;
	}
}

// Main progress bar struct
pub struct ProgressBar {
	stats: Arc<RwLock<ProgressStats>>,
	theme: ProgressTheme,
	width: u16,
	message: String,
	spinner_idx: AtomicU64,
	prefix: String,
	suffix: String,
	hide_cursor: bool,
	enable_colors: bool,
}

impl ProgressBar {
	pub fn new(total: u64) -> Self {
		Self {
			stats: Arc::new(RwLock::new(ProgressStats::new(total))),
			theme: ProgressTheme::default(),
			width: 40,
			message: String::new(),
			spinner_idx: AtomicU64::new(0),
			prefix: String::new(),
			suffix: String::new(),
			hide_cursor: true,
			enable_colors: true,
		}
	}

	// Builder methods
	pub fn with_theme(mut self, theme: ProgressTheme) -> Self {
		self.theme = theme;
		self
	}

	pub fn with_width(mut self, width: u16) -> Self {
		self.width = width;
		self
	}

	pub fn with_message<S: Into<String>>(mut self, message: S) -> Self {
		self.message = message.into();
		self
	}

	pub fn with_prefix<S: Into<String>>(mut self, prefix: S) -> Self {
		self.prefix = prefix.into();
		self
	}

	pub fn with_suffix<S: Into<String>>(mut self, suffix: S) -> Self {
		self.suffix = suffix.into();
		self
	}

	pub fn without_cursor(mut self) -> Self {
		self.hide_cursor = false;
		self
	}

	pub fn without_colors(mut self) -> Self {
		self.enable_colors = false;
		self
	}

	// Progress updates
	pub async fn set_progress(&self, n: u64) -> io::Result<()> {
		let mut stats = self.stats.write().await;
		let current = n;
		stats.update(current);
		self.draw(&stats).await?;
		Ok(())
	}

	pub async fn increment(&self, delta: u64) -> io::Result<()> {
		let mut stats = self.stats.write().await;
		let current = stats.current + delta;
		stats.update(current);
		self.draw(&stats).await?;
		Ok(())
	}

	pub async fn finish(&self) -> io::Result<()> {
		let mut stats = self.stats.write().await;
		stats.current = stats.total;
		self.draw(&stats).await?;
		println!();

		if self.hide_cursor {
			print!("{}", termion::cursor::Show);
			io::stdout().flush().unwrap();
		}
		Ok(())
	}
	pub async fn finish_with_message<S: Into<String>>(&self, msg: S) -> io::Result<()> {
		let mut stats = self.stats.write().await;
		stats.current = stats.total;
		stats.message = msg.into();
		self.draw(&stats).await?;
		println!();

		if self.hide_cursor {
			print!("{}", termion::cursor::Show);
			io::stdout().flush()?;
		}
		Ok(())
	}

	// Drawing methods
	async fn draw(&self, stats: &ProgressStats) -> io::Result<()> {
		let percent = stats.current as f32 / stats.total as f32;
		let filled_width = (self.width as f32 * percent) as usize;
		let empty_width = self.width as usize - filled_width;

		// Prepare spinner
		let spinner_frame = &self.theme.spinner_frames[(self.spinner_idx.fetch_add(1, Ordering::SeqCst) as usize) % self.theme.spinner_frames.len()];

		// Format progress bar
		let mut bar = String::new();

		if self.enable_colors {
			// Add colors
			bar.push_str(&format!("{}{}", color::Fg(self.theme.colors.progress), self.theme.bar_filled.repeat(filled_width)));
			bar.push_str(&format!("{}{}", color::Fg(self.theme.colors.background), self.theme.bar_empty.repeat(empty_width)));
		} else {
			bar.push_str(&self.theme.bar_filled.repeat(filled_width));
			bar.push_str(&self.theme.bar_empty.repeat(empty_width));
		}

		// Format stats
		let stats_str = format!(
			"{:.1}% [{}/{}] {:.1} MB/s ETA: {}",
			percent * 100.0,
			self.format_bytes(stats.current),
			self.format_bytes(stats.total),
			stats.bytes_per_sec / 1_000_000.0,
			self.format_duration(stats.eta)
		);

		// Construct full progress line
		let line = format!(
			"\r{prefix}{spinner} {start_cap}{bar}{end_cap} {stats} {msg}{suffix}",
			prefix = self.prefix,
			spinner = if self.enable_colors {
				format!("{}{}", color::Fg(self.theme.colors.spinner), spinner_frame)
			} else {
				spinner_frame.to_string()
			},
			start_cap = self.theme.start_cap,
			bar = bar,
			end_cap = self.theme.end_cap,
			stats = if self.enable_colors {
				format!("{}{}", color::Fg(self.theme.colors.percentage), stats_str)
			} else {
				stats_str
			},
			msg = if self.enable_colors {
				format!("{}{}", color::Fg(self.theme.colors.text), self.message)
			} else {
				self.message.clone()
			},
			suffix = self.suffix
		);

		// Reset colors and print
		if self.enable_colors {
			print!("{}{}", line, style::Reset);
		} else {
			print!("{}", line);
		}

		io::stdout().flush()?;
		Ok(())
	}

	// Utility methods
	fn format_bytes(&self, bytes: u64) -> String {
		const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
		let mut size = bytes as f64;
		let mut unit_index = 0;

		while size >= 1024.0 && unit_index < UNITS.len() - 1 {
			size /= 1024.0;
			unit_index += 1;
		}

		if unit_index == 0 {
			format!("{} {}", size as u64, UNITS[unit_index])
		} else {
			format!("{:.2} {}", size, UNITS[unit_index])
		}
	}

	fn format_duration(&self, duration: Duration) -> String {
		let secs = duration.as_secs();
		if secs < 60 {
			format!("{}s", secs)
		} else if secs < 3600 {
			format!("{}m {}s", secs / 60, secs % 60)
		} else {
			format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
		}
	}
}

// Examples and tests
#[cfg(test)]
mod tests {
	use super::*;
	use tokio::time::sleep;

	#[tokio::test]
	async fn test_progress_bar() {
		let total_size = 1024 * 1024 * 100; // 100MB
		let bar = ProgressBar::new(total_size).with_message("Downloading file.rs").with_prefix("🚀 ").with_suffix(" 📦");

		for i in 0..100 {
			bar.set_progress((total_size as f64 * (i as f64 / 100.0)) as u64).await;
			sleep(Duration::from_millis(50)).await;
		}

		bar.finish_with_message("Download complete!").await;
	}

	#[tokio::test]
	async fn test_themes() {
		async fn demo_theme(theme: ProgressTheme, name: &str) {
			let bar = ProgressBar::new(100).with_theme(theme).with_message(format!("Testing {} theme", name));

			for i in 0..=100 {
				bar.set_progress(i).await;
				sleep(Duration::from_millis(20)).await;
			}

			bar.finish().await;
		}

		demo_theme(ProgressTheme::default(), "default").await;
		demo_theme(ProgressTheme::minimal(), "minimal").await;
		demo_theme(ProgressTheme::fancy(), "fancy").await;
	}
}
