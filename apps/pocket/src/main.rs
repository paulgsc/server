#![allow(clippy::disallowed_macros)]
#![allow(clippy::multiple_crate_versions)]
use clap::Parser;
use pocket::{
	cli::{handlers, Cli, Command},
	error::PocketError,
	store::RegistryPath,
};

fn main() {
	let cli = Cli::parse();

	let result = run(cli);

	if let Err(e) = result {
		// Distinguish user-facing (wrong input) from system (I/O) errors
		// so callers can differentiate via exit code.
		let code = match &e {
			// Escape / no selection is not an error — compose pattern (e.g. pipe)
			PocketError::PickerCancelled => 0,
			// Bad input — user-correctable
			PocketError::LabelNotFound { .. }
			| PocketError::EmptyLabel
			| PocketError::EmptyValue
			| PocketError::DuplicateLabel { .. }
			| PocketError::RegistryFull { .. }
			| PocketError::PickerNoSelection => 1,
			// System / serialisation errors
			_ => 2,
		};
		eprintln!("pocket: {e}");
		std::process::exit(code);
	}
}

fn run(cli: Cli) -> pocket::error::Result<()> {
	let path = RegistryPath::resolve()?;

	match cli.command {
		Command::Add { label, value } => handlers::add(&path, label, value),
		Command::Query => handlers::query(&path),
		Command::Ls => handlers::ls(&path),
		Command::Rm { label } => handlers::rm(&path, &label),
		Command::Edit { label } => handlers::edit(&path, &label),
		Command::Path => {
			handlers::print_path(&path);
			Ok(())
		}
	}
}
