use clap::Parser;
use enum_name_derive::EnumFilename;
use serde_json::{json, Value};
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
	/// Path to the workspaces directory
	#[arg(short, long)]
	pub workspaces: PathBuf,
}

#[derive(Clone, Eq, PartialEq, EnumFilename)]
pub enum ConfigFile {
	#[filename = "tsconfig.json"]
	Tsconfig,
	#[filename = "package.json"]
	PackageJson,
	#[filename = "rollup.config.js"]
	RollupConfig,
	#[filename = "eslint.config.js"]
	EslintConfig,
	#[filename = "tsconfig.build.json"]
	TsconfigBuildConfig,
}

pub fn find_packages(workspaces: &Path) -> io::Result<Vec<PathBuf>> {
	Ok(
		fs::read_dir(workspaces)?
			.filter_map(Result::ok)
			.filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
			.map(|entry| entry.path())
			.collect(),
	)
}

pub fn update_package_json(content: &str, new_package_name: &str) -> io::Result<String> {
	let mut json: Value = serde_json::from_str(content).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

	if let Some(obj) = json.as_object_mut() {
		obj.insert("name".to_string(), json!(new_package_name));
	}

	serde_json::to_string_pretty(&json).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

pub fn copy_configs(template_package: &Path, new_package_path: &Path, configs: &[ConfigFile], new_package_name: &str) -> io::Result<()> {
	for config in configs {
		let source = template_package.join(config.filename());
		let dest = new_package_path.join(config.filename());

		if source.exists() {
			if *config == ConfigFile::PackageJson {
				let content = fs::read_to_string(&source)?;
				let updated_content = update_package_json(&content, new_package_name)?;
				fs::write(&dest, updated_content)?;
			} else {
				fs::copy(&source, &dest)?;
			}
			println!("Copied {}", config.filename());
		}
	}

	Ok(())
}

pub fn find_closest_match<'a>(input: &'a str, candidates: &[&'a str]) -> Option<&'a str> {
	let input_len = input.chars().count();

	candidates
		.iter()
		.min_by_key(|&&candidate| levenshtein(input, candidate))
		.filter(|&&candidate| {
			let distance = levenshtein(input, candidate);
			let max_len = input_len.max(candidate.chars().count()); // Use the longest word length
			let similarity = 1.0 - (distance as f64 / max_len as f64);
			similarity >= 0.75
		})
		.copied()
}

pub fn levenshtein(a: &str, b: &str) -> usize {
	let a_len = a.chars().count();
	let b_len = b.chars().count();

	let mut dp = vec![vec![0; b_len + 1]; a_len + 1];

	for i in 0..=a_len {
		dp[i][0] = i;
	}
	for j in 0..=b_len {
		dp[0][j] = j;
	}

	for (i, a_char) in a.chars().enumerate() {
		for (j, b_char) in b.chars().enumerate() {
			let cost = if a_char == b_char { 0 } else { 1 };
			dp[i + 1][j + 1] = (dp[i][j + 1] + 1) // Deletion
				.min(dp[i + 1][j] + 1) // Insertion
				.min(dp[i][j] + cost); // Substitution
		}
	}

	dp[a_len][b_len]
}
