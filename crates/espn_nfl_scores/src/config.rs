use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "Score Parser")]
#[command(about = "Parses HTML scores and outputs to CSV", long_about = None)]
pub struct Cli {
    /// Path to the env.toml file
    #[arg(short, long, value_name = "FILE")]
    pub config: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Path to the input .tx file with HTML content
    pub input_file: String,
    /// Path to the output CSV file
    pub output_file: String,
}

impl Config {
    pub fn from_toml(file_path: &PathBuf) -> Result<Self, config::ConfigError> {
        let mut settings = config::Config::default();
        settings.merge(config::File::from(file_path.clone()))?;
        settings.try_into()
    }
}

