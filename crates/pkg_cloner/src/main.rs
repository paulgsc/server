use clap::{error::ErrorKind::UnknownArgument, Parser};
use dialoguer::{Input, Select};
use pkg_cloner::{copy_configs, find_closest_match, find_packages, Cli, ConfigFile};
use std::fs;
use std::io::{self, Error, ErrorKind};

fn main() -> io::Result<()> {
	let cli = parse_cli_with_suggestions()?;

	let packages = find_packages(&cli.workspaces)?;

	let package_names: Vec<String> = packages.iter().filter_map(|path| path.file_name()?.to_str().map(String::from)).collect();

	if package_names.is_empty() {
		return Err(Error::new(ErrorKind::NotFound, "No packages found in workspace directory"));
	}

	let selection = Select::new()
		.with_prompt("Select a package to use as template")
		.items(&package_names)
		.interact()
		.map_err(|e| Error::new(ErrorKind::Other, e))?;

	let template_package = &packages[selection];

	let new_package_name: String = Input::new()
		.with_prompt("Enter new package name")
		.interact_text()
		.map_err(|e| Error::new(ErrorKind::Other, e))?;

	let new_package_path = cli.workspaces.join(&new_package_name);
	fs::create_dir_all(&new_package_path)?;
	fs::create_dir_all(new_package_path.join("src"))?;

	let configs = vec![
		ConfigFile::Tsconfig,
		ConfigFile::PackageJson,
		ConfigFile::RollupConfig,
		ConfigFile::EslintConfig,
		ConfigFile::TsconfigBuildConfig,
	];

	copy_configs(template_package, &new_package_path, &configs, &new_package_name)?;

	println!("\nSuccessfully created new package: {new_package_name}");
	println!("Location: {}", new_package_path.display());

	Ok(())
}

fn parse_cli_with_suggestions() -> io::Result<Cli> {
	Cli::try_parse().or_else(|e| {
		if e.kind() != UnknownArgument {
			return Err(Error::new(ErrorKind::InvalidInput, e.to_string()));
		}

		let error_msg = e.to_string(); // String stays alive
		let invalid_arg = error_msg.split('\'').nth(1).ok_or_else(|| Error::new(ErrorKind::InvalidInput, error_msg.clone()))?;
		let valid_args = vec!["--workspaces", "-w"];
		let suggestion = find_closest_match(invalid_arg, &valid_args).ok_or_else(|| Error::new(ErrorKind::InvalidInput, e.to_string()))?;

		eprintln!("Warning: Unknown argument '{}'. Using '{}' instead?", invalid_arg, suggestion);

		// Replace invalid arg with suggestion and retry
		let args: Vec<String> = std::env::args().map(|arg| if arg == invalid_arg { suggestion.to_string() } else { arg }).collect();

		Cli::try_parse_from(&args).map_err(|e| Error::new(ErrorKind::InvalidInput, e.to_string()))
	})
}
