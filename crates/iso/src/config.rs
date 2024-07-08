use clap::Parser;

#[derive(Parser, Debug)]
pub struct Config {
	#[arg(env = "DATABASE_URL")]
	pub database_url: String,
}
