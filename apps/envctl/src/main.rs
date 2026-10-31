//! Every variable the stack reads, from where each one is declared (#403).
//!
//! Each binary's clap `Config` is its schema; its tests check a copy into
//! `infra/env/<binary>.json` (`app_config::schema`, CFG7 #503). The compose
//! files declare the rest as `${NAME...}`. `envctl` reads both and never a
//! second list (CFG10, #606; it replaces `scripts/env_schema.py` and
//! `scripts/nixos_options.py`).

// A command-line tool whose whole job is text: reports, generated files and
// a terminal screen for a person or a CI log. It runs no tracing subscriber,
// and nothing here is a hot path the tracing-only convention protects.
#![allow(clippy::disallowed_macros)]

mod compose;
mod deploy;
mod dotenv;
mod example;
mod import;
mod nix;
mod preflight;
mod resolve;
mod schema;
mod values;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use indexmap::IndexMap;
use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
	process::ExitCode,
};

#[derive(Parser)]
#[command(
	version,
	about = "Check, generate and migrate the stack's configuration: schemas, deployment files, example.env, NixOS options"
)]
struct Cli {
	/// The checkout. Default: the nearest directory up from here holding
	/// `infra/env` and `docker-compose.yml`.
	#[arg(long, global = true)]
	repo: Option<PathBuf>,
	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand)]
enum Command {
	/// Hold every deploy/<role>/<binary>.env to its schema, and compose to
	/// reading it (CFG9, #604; CI's Env schema job, `make deploy-check`).
	Check,
	/// Before `docker compose up` (CFG8, #504): every missing or refused
	/// variable, and every "ours" value .env would override, at once.
	Preflight {
		/// The .env to check (default: the repo's, which compose reads).
		#[arg(long)]
		env_file: Option<PathBuf>,
	},
	/// List every variable the stack reads, its default, and whether .env or
	/// the environment sets it (`make env`).
	List {
		#[arg(long)]
		env_file: Option<PathBuf>,
	},
	/// Write example.env, or with --check fail when it differs.
	Example {
		#[arg(long)]
		check: bool,
	},
	/// Write nix/modules/generated/*.nix, or with --check fail when one
	/// differs (#408).
	Nix {
		#[arg(long)]
		check: bool,
	},
	/// Print a deployment file as shell exports, skipping each variable the
	/// environment already sets (`make dev`).
	Export { file: PathBuf },
	/// Fill deploy/<role>/<binary>.env from what the running containers use:
	/// every "ours" value as it runs, its default where it is unset, `#unset`
	/// where it has none. Reads no secret or private value.
	Import {
		/// The role the containers run (DEPLOY).
		#[arg(long, default_value = "home")]
		role: String,
		/// The containers to read; default every running one labelled
		/// env-schema.
		#[arg(long = "container")]
		containers: Vec<String>,
		/// Say what would change, and write nothing.
		#[arg(long)]
		dry_run: bool,
	},
	/// Decide each "ours" value of one deployment file in a terminal UI,
	/// beside the running container's value and the variable's full help.
	Resolve {
		#[arg(long, default_value = "home")]
		role: String,
		#[arg(long, default_value = "file_host")]
		binary: String,
		/// The running container to compare with; default the one labelled
		/// env-schema=<binary>, if any.
		#[arg(long)]
		container: Option<String>,
	},
}

fn main() -> ExitCode {
	match run(Cli::parse()) {
		Ok(code) => code,
		Err(err) => {
			eprintln!("envctl: {err:#}");
			ExitCode::from(2)
		}
	}
}

fn find_repo(given: Option<PathBuf>) -> Result<PathBuf> {
	if let Some(repo) = given {
		return Ok(repo);
	}
	let start = std::env::current_dir().context("reading the current directory")?;
	start
		.ancestors()
		.find(|dir| dir.join("infra/env").is_dir() && dir.join("docker-compose.yml").is_file())
		.map(Path::to_path_buf)
		.with_context(|| format!("no checkout (infra/env and docker-compose.yml) at or above {}", start.display()))
}

/// The variables this process was started with: what compose would see from
/// the shell. Reading them is the point of a tool that checks what compose
/// starts on, not configuration of envctl itself (#484).
#[allow(clippy::disallowed_methods)]
fn shell() -> IndexMap<String, String> {
	std::env::vars().collect()
}

const fn exit(failed: bool) -> ExitCode {
	if failed {
		ExitCode::FAILURE
	} else {
		ExitCode::SUCCESS
	}
}

fn run(cli: Cli) -> Result<ExitCode> {
	let repo = find_repo(cli.repo)?;
	let schemas = schema::load(&repo.join("infra/env"))?;
	match cli.command {
		Command::Check => {
			let raw = compose::config(&repo).context("docker compose config failed, so no service was checked")?;
			let files = deploy::read_all(&repo.join("deploy"))?;
			let problems = deploy::check(&schemas, &raw, &files);
			for problem in &problems {
				println!("{problem}");
			}
			if problems.is_empty() {
				let checked: Vec<String> = files.keys().map(|(role, binary)| format!("{role}/{binary}")).collect();
				println!("every deployment file states every \"ours\" value of its binary ({}).", checked.join(", "));
			} else {
				println!(
					"{} problem(s): every \"ours\" variable is decided in every deploy/<role>/<binary>.env (CFG9, #604).",
					problems.len()
				);
			}
			Ok(exit(!problems.is_empty()))
		}
		Command::Preflight { env_file } => {
			let env_file = env_file.unwrap_or_else(|| repo.join(".env"));
			let report = preflight::run(&repo, &schemas, &compose::read(&repo)?, &env_file, &shell());
			print!("{}", preflight::render(&report, &env_file));
			Ok(exit(report.failed()))
		}
		Command::List { env_file } => {
			let env_file = env_file.unwrap_or_else(|| repo.join(".env"));
			let env = dotenv::parse(&std::fs::read_to_string(&env_file).unwrap_or_default());
			let shell = shell();
			print!(
				"{}",
				preflight::render_list(&schemas, &compose::read(&repo)?, &|name| env.contains_key(name) || shell.contains_key(name))
			);
			Ok(ExitCode::SUCCESS)
		}
		Command::Example { check } => {
			let head = std::fs::read_to_string(repo.join("infra/env/example.env.head")).context("reading infra/env/example.env.head")?;
			let deployed: BTreeSet<String> = deploy::read_all(&repo.join("deploy"))?.into_keys().map(|(_, binary)| binary).collect();
			let rendered = example::render(&schemas, &compose::read(&repo)?, &head, &deployed);
			let path = repo.join("example.env");
			if check {
				if std::fs::read_to_string(&path).unwrap_or_default() != rendered {
					println!("example.env is not what `make env-schema` writes from infra/env/*.json and the compose files.");
					println!("Change the variable where it is declared (its Config, or the compose file), then run `make env-schema`.");
					return Ok(ExitCode::FAILURE);
				}
				println!("example.env matches its declarations.");
			} else {
				std::fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
			}
			Ok(ExitCode::SUCCESS)
		}
		Command::Nix { check } => {
			let out = repo.join("nix/modules/generated");
			let mut stale = Vec::new();
			for schema in &schemas {
				let path = out.join(nix::file_name(&schema.binary));
				let rendered = nix::render(schema);
				if check {
					if std::fs::read_to_string(&path).ok().as_deref() != Some(rendered.as_str()) {
						stale.push(path.strip_prefix(&repo).unwrap_or(&path).display().to_string());
					}
				} else {
					std::fs::create_dir_all(&out)?;
					std::fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
				}
			}
			if !stale.is_empty() {
				println!("not what infra/env/*.json generate: {}. Run `make env-schema` and commit what it writes.", stale.join(", "));
				return Ok(ExitCode::FAILURE);
			}
			if check {
				println!("nix/modules/generated matches infra/env.");
			}
			Ok(ExitCode::SUCCESS)
		}
		Command::Export { file } => {
			let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
			let shell = shell();
			print!("{}", deploy::render_exports(&text, &|name| shell.contains_key(name)));
			Ok(ExitCode::SUCCESS)
		}
		Command::Import { role, containers, dry_run } => {
			let changed = import::run(&repo, &schemas, &role, &containers, dry_run)?;
			Ok(exit(dry_run && changed))
		}
		Command::Resolve { role, binary, container } => {
			let Some(schema) = schemas.iter().find(|schema| schema.binary == binary) else {
				bail!("infra/env/ has no {binary}.json");
			};
			resolve::run(&repo, schema, &role, container.as_deref())?;
			Ok(ExitCode::SUCCESS)
		}
	}
}
