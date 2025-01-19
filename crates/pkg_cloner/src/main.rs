use clap::Parser;
use dialoguer::{Input, Select};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
	/// Path to the workspaces directory
	#[arg(short, long)]
	workspaces: PathBuf,
}

#[derive(Clone, PartialEq)]
enum ConfigFile {
	Tsconfig,
	PackageJson,
	RollupConfig,
	EslintConfig,
	EslintBuildConfig,
}

impl ConfigFile {
	const fn filename(&self) -> &str {
		match self {
			Self::Tsconfig => "tsconfig.json",
			Self::PackageJson => "package.json",
			Self::RollupConfig => "rollup.config.js",
			Self::EslintConfig => "eslint.config.js",
			Self::EslintBuildConfig => "eslint.build.config.js",
		}
	}
}

fn main() -> std::io::Result<()> {
	let cli = Cli::parse();

	// Get all package directories
	let packages = fs::read_dir(&cli.workspaces)?
		.filter_map(Result::ok)
		.filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
		.collect::<Vec<_>>();

	// Create selection menu for template package
	let package_names: Vec<String> = packages.iter().map(|entry| entry.file_name().to_string_lossy().to_string()).collect();

	let selection = Select::new().with_prompt("Select a package to use as template").items(&package_names).interact().unwrap();

	let template_package = &packages[selection];

	// Get new package name and path
	let new_package_name: String = Input::new().with_prompt("Enter new package name").interact_text().unwrap();

	// Create new package directory
	let new_package_path = cli.workspaces.join(&new_package_name);
	fs::create_dir_all(&new_package_path)?;

	// Create src directory
	fs::create_dir_all(new_package_path.join("src"))?;

	// Copy config files
	let configs = vec![ConfigFile::Tsconfig, ConfigFile::PackageJson, ConfigFile::RollupConfig, ConfigFile::EslintConfig];

	for config in configs {
		let source = template_package.path().join(config.filename());
		let dest = new_package_path.join(config.filename());

		if source.exists() {
			if config == ConfigFile::PackageJson {
				// Read package.json and update the name
				let content = fs::read_to_string(&source)?;
				let mut json: serde_json::Value = serde_json::from_str(&content).expect("Failed to parse package.json");

				if let Some(obj) = json.as_object_mut() {
					obj["name"] = serde_json::Value::String(new_package_name.clone());
				}

				fs::write(dest, serde_json::to_string_pretty(&json)?)?;
			} else {
				fs::copy(source, dest)?;
			}
			println!("Copied {}", config.filename());
		}
	}

	println!("\nSuccessfully created new package: {}", new_package_name);
	println!("Location: {}", new_package_path.display());

	Ok(())
}
