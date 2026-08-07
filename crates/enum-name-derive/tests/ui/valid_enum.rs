use enum_name_derive::EnumFilenameAndFromString;
use std::str::FromStr;

#[derive(EnumFilenameAndFromString)]
pub enum ConfigFile {
	#[filename = "tsconfig.json"]
	Tsconfig,
	#[filename = "package.json"]
	PackageJson,
}

fn main() {
	assert_eq!(ConfigFile::Tsconfig.filename(), "tsconfig.json");
	assert_eq!(ConfigFile::PackageJson.filename(), "package.json");

	assert!(matches!(ConfigFile::from_str("tsconfig.json"), Ok(ConfigFile::Tsconfig)));
	assert!(matches!(ConfigFile::from_str("package.json"), Ok(ConfigFile::PackageJson)));
	assert!(ConfigFile::from_str("Cargo.toml").is_err());
}
