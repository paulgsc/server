use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Cli {
	#[clap(subcommand)]
	pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
	PopulateGameClocks,
}
