use clap::Parser;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Config {
	#[arg(long, env = "PREFETCH_COUNT", default_value = "10", help = "Number of tasks to prefetch from queue")]
	pub prefetch_count: usize,

	#[arg(long, env = "MAX_RETRIES", default_value = "3", help = "Maximum number of retry attempts for failed tasks")]
	pub max_retries: u32,

	#[arg(
        long,
        env = "RETRY_DELAY_SECS",
        default_value = "60",
        value_parser = parse_duration,
        help = "Delay between retry attempts in seconds"
    )]
	pub retry_delay: Duration,

	#[arg(
        long,
        env = "TASK_TIMEOUT_SECS",
        default_value = "300",
        value_parser = parse_duration,
        help = "Task execution timeout in seconds"
    )]
	pub task_timeout: Duration,

	#[arg(
        long,
        env = "HEARTBEAT_INTERVAL_SECS",
        default_value = "30",
        value_parser = parse_duration,
        help = "Worker heartbeat interval in seconds"
    )]
	pub heartbeat_interval: Duration,
}

impl Config {
	pub fn new() -> Self {
		Self::parse()
	}

	pub fn default() -> Self {
		Self {
			prefetch_count: 5,
			max_retries: 3,
			retry_delay: Duration::from_secs(60),
			task_timeout: Duration::from_secs(300),
			heartbeat_interval: Duration::from_secs(30),
		}
	}

	#[cfg(test)]
	pub fn test() -> Self {
		Self {
			prefetch_count: 1,
			max_retries: 1,
			retry_delay: Duration::from_secs(1),
			task_timeout: Duration::from_secs(5),
			heartbeat_interval: Duration::from_secs(1),
		}
	}
}

fn parse_duration(s: &str) -> Result<Duration, std::num::ParseIntError> {
	s.parse::<u64>().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_default_config() {
		let config = Config::default();
		assert_eq!(config.prefetch_count, 10);
		assert_eq!(config.max_retries, 3);
		assert_eq!(config.retry_delay, Duration::from_secs(60));
		assert_eq!(config.task_timeout, Duration::from_secs(300));
		assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
	}

	#[test]
	fn test_parse_duration() {
		assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
		assert!(parse_duration("invalid").is_err());
	}

	#[test]
	fn test_config_parser() {
		let args = vec![
			"program",
			"--prefetch-count",
			"20",
			"--max-retries",
			"5",
			"--retry-delay-secs",
			"120",
			"--task-timeout-secs",
			"600",
			"--heartbeat-interval-secs",
			"45",
		];

		let config = Config::try_parse_from(args).unwrap();
		assert_eq!(config.prefetch_count, 20);
		assert_eq!(config.max_retries, 5);
		assert_eq!(config.retry_delay, Duration::from_secs(120));
		assert_eq!(config.task_timeout, Duration::from_secs(600));
		assert_eq!(config.heartbeat_interval, Duration::from_secs(45));
	}
}
