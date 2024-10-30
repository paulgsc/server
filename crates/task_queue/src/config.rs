use clap::Parser;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Config {
	#[arg(long, env = "PREFETCH_COUNT", help = "Default prefetch count")]
    pub prefetch_count: usize,

	#[arg(long, env = "MAX_RETRIES", default_value = "3", help = "Max no. of retries for failed task")]
    pub max_retries: u32,

    #[arg(long, env = "RETRY_DELAY", default_value = "3", help = "Duration between retries")]
    pub retry_delay: Duration,

    #[arg(long, env = "TASK_TIMEOUT", default_value = "3", help = "Duration between retries")]
    pub task_timeout: Duration,

    #[arg(long, env = "TASK_TIMEOUT", default_value = "3", help = "Duration between retries")]
    pub heartbeat_interval: Duration,

}

impl Config {
	pub fn new() -> Self {
		Self::parse()
	}
}
