use clap::Parser;
use serde::{Deserialize, Serialize};


#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Config {

    /// File with HTML soup
    #[arg(long, env = "HTML_FILE")]
    pub input_file: String,
    /// Path to the output CSV file
    #[arg(long, env = "CSV_OUTPUT", default_value = "data.csv")]
    pub output_file: String,
}


impl Config {
    pub fn new() -> Self {
        Self::parse()
    }
}

