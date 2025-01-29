use enum_name_derive::EnumFilename;

#[derive(EnumFilename)]
pub enum ConfigFile {
	#[filename = "tsconfig.json"]
	Tsconfig,
	#[filename = "package.json"]
	PackageJson,
}

fn main() {
	assert_eq!(ConfigFile::Tsconfig.filename(), "tsconfig.json");
	assert_eq!(ConfigFile::PackageJson.filename(), "package.json");
}
