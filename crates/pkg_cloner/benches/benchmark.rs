use criterion::{black_box, criterion_group, criterion_main, Criterion};
use pkg_cloner::{copy_configs, find_packages, ConfigFile};
use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;

fn benchmark_find_packages(c: &mut Criterion) {
	c.bench_function("find_packages", |b| {
		let workspaces_dir = tempdir().unwrap();
		let workspaces = workspaces_dir.path();

		for i in 0..3 {
			let package_path = workspaces.join(format!("package_{}", i));
			fs::create_dir_all(&package_path).unwrap();
		}

		b.iter(|| black_box(find_packages(workspaces)));
	});
}

fn benchmark_copy_configs(c: &mut Criterion) {
	c.bench_function("copy_configs", |b| {
		let template_dir = tempdir().unwrap();
		let new_package_dir = tempdir().unwrap();
		let template_package = template_dir.path();
		let new_package_path = new_package_dir.path();

		let configs = vec![
			ConfigFile::Tsconfig,
			ConfigFile::PackageJson,
			ConfigFile::RollupConfig,
			ConfigFile::EslintConfig,
			ConfigFile::EslintBuildConfig,
		];

		for config in &configs {
			let file_path = template_package.join(config.filename());
			let mut file = File::create(&file_path).unwrap();
			write!(file, "{{}}").unwrap();
		}

		b.iter(|| black_box(copy_configs(template_package, new_package_path, &configs, "new_package_name")));
	});
}

criterion_group!(benches, benchmark_find_packages, benchmark_copy_configs);

criterion_main!(benches);
