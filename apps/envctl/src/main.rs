//! Every variable the stack reads, from where each one is declared (#403),
//! and every value a deployment sets, from its encrypted files (#613).
//!
//! Each binary's clap `Config` is its schema; its tests check a copy into
//! `infra/env/<binary>.json` (`app_config::schema`, CFG7 #503). The compose
//! files declare the rest as `${NAME...}`. A deployment's values are in
//! `deploy/<role>/<app>.sops.env` (ENV1, #614), encrypted with sops to the
//! recipients in `.sops.yaml`: envctl decrypts them into memory, gives each
//! compose service its own on tmpfs only, and prints names, never a value.

// A command-line tool whose whole job is text: reports and generated files
// for a person or a CI log. It runs no tracing subscriber, and nothing here
// is a hot path the tracing-only convention protects.
#![allow(clippy::disallowed_macros)]

mod compose;
mod deliver;
mod dotenv;
mod import;
mod nix;
mod preflight;
mod prune;
mod schema;
mod sops;
mod values;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use indexmap::IndexMap;
use preflight::Deployment;
use schema::Schema;
use serde_json::Value;
use std::{
	path::{Path, PathBuf},
	process::{Command as Process, ExitCode},
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
	time::Duration,
};

#[derive(Parser)]
#[command(version, about = "Deliver, check and migrate the stack's configuration: encrypted deployment files, schemas, NixOS options")]
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
	/// Before `docker compose up` (CFG8, #504): decrypt deploy/<role>/ into
	/// memory and name every missing or refused variable, at once.
	Preflight {
		/// The role this host runs (DEPLOY).
		#[arg(long, default_value = "home")]
		role: String,
	},
	/// The preflight, then `docker compose up -d` with each service's file
	/// decrypted to tmpfs, removed once compose has started it.
	Up {
		#[arg(long, default_value = "home")]
		role: String,
		/// Where the decrypted files go (default `$XDG_RUNTIME_DIR/server`);
		/// it must be on tmpfs.
		#[arg(long)]
		env_dir: Option<PathBuf>,
	},
	/// Any `docker compose` command, with the role's files decrypted for it
	/// as for `up` (`make compose ARGS="logs -f file-host"`).
	Compose {
		#[arg(long, default_value = "home")]
		role: String,
		#[arg(long)]
		env_dir: Option<PathBuf>,
		/// What follows `docker compose`.
		#[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
		args: Vec<String>,
	},
	/// Run a command with one app's file decrypted into its environment, the
	/// shell's own variables winning (`make dev`).
	Run {
		#[arg(long, default_value = "dev")]
		role: String,
		#[arg(long, default_value = "file_host")]
		app: String,
		#[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
		command: Vec<String>,
	},
	/// Every variable the stack reads, its default, and whether the role's
	/// files set it, read by name without decrypting (`make env`).
	List {
		#[arg(long, default_value = "home")]
		role: String,
	},
	/// Without a key: each labelled compose service reads only its own
	/// decrypted file, and every file under deploy/ is encrypted.
	Check,
	/// Write nix/modules/generated/*.nix, or with --check fail when one
	/// differs (#408).
	Nix {
		#[arg(long)]
		check: bool,
	},
	/// Prune variables of one app's file, in a checklist grouped by the
	/// schema's sections, a page at a time; written through sops (`ENV2b`, #628).
	Prune {
		#[arg(long, default_value = "home")]
		role: String,
		#[arg(long, default_value = "file_host")]
		app: String,
		/// Rows per page.
		#[arg(long, default_value_t = 10)]
		page: usize,
		/// Prune these, without the checklist (comma-separated).
		#[arg(long = "prune", value_delimiter = ',')]
		names: Vec<String>,
	},
	/// Write deploy/<role>/ from what the running containers use, through
	/// sops: no plain text reaches a disk (ENV2, #615).
	Import {
		#[arg(long, default_value = "home")]
		role: String,
		/// The containers to read; default every running one labelled
		/// env-schema.
		#[arg(long = "container")]
		containers: Vec<String>,
		/// Say which names would change, and write nothing.
		#[arg(long)]
		dry_run: bool,
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
	let found = if let Some(repo) = given {
		repo
	} else {
		let start = std::env::current_dir().context("reading the current directory")?;
		start
			.ancestors()
			.find(|dir| dir.join("infra/env").is_dir() && dir.join("docker-compose.yml").is_file())
			.map(Path::to_path_buf)
			.with_context(|| format!("no checkout (infra/env and docker-compose.yml) at or above {}", start.display()))?
	};
	std::fs::canonicalize(&found).with_context(|| format!("no checkout at {}", found.display()))
}

/// The variables this process was started with. Reading them is the point
/// of a tool that starts compose and a dev server on them, not configuration
/// of envctl itself (#484).
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

/// The binaries compose's labelled services run, in service order.
fn labelled(raw: &Value) -> Vec<String> {
	let mut binaries: Vec<String> = Vec::new();
	for (_, service) in compose::services(raw) {
		if let Some(binary) = compose::labels(service).get(compose::LABEL) {
			if !binaries.contains(binary) {
				binaries.push(binary.clone());
			}
		}
	}
	binaries
}

/// Each `<app>.sops.env` in deploy/<role>/ but compose's own.
fn files_of(repo: &Path, role: &str) -> Vec<String> {
	let mut apps: Vec<String> = std::fs::read_dir(repo.join(sops::DEPLOY).join(role))
		.into_iter()
		.flatten()
		.filter_map(|entry| entry.ok()?.file_name().into_string().ok())
		.filter_map(|name| name.strip_suffix(sops::SUFFIX).map(str::to_owned))
		.filter(|app| app != sops::COMPOSE)
		.collect();
	apps.sort();
	apps
}

/// One role's files, decrypted into memory, and what make gives compose.
fn deployment(repo: &Path, role: &str, raw: &Value, env_dir: &Path, sops_program: &[String]) -> Result<Deployment> {
	// An empty value is "decided: unset" (#616): the binary sees it unset.
	let read = |app: &str| -> Result<Option<sops::Plain>> {
		let relative = sops::relative(role, app);
		if repo.join(&relative).exists() {
			sops::decrypt(sops_program, repo, &relative).map(|file| Some(sops::set_only(file)))
		} else {
			Ok(None)
		}
	};
	let mut compose = read(sops::COMPOSE)?.unwrap_or_default();
	compose.insert("REPO_ROOT".to_owned(), repo.display().to_string());
	compose.insert("ENV_DIR".to_owned(), env_dir.display().to_string());
	let mut apps = IndexMap::new();
	for app in labelled(raw).into_iter().chain(files_of(repo, role)) {
		if !apps.contains_key(&app) {
			let file = read(&app)?;
			apps.insert(app, file);
		}
	}
	Ok(Deployment {
		role: role.to_owned(),
		compose,
		apps,
	})
}

fn check_preflight(repo: &Path, schemas: &[Schema], raw: &Value, deployment: &Deployment) -> Result<bool> {
	let report = preflight::preflight(schemas, &compose::read(repo)?, raw, deployment);
	print!("{}", preflight::render(&report, &deployment.role));
	if repo.join(".env").exists() {
		println!(
			"warning: {} is read by nothing (compose runs with --env-file /dev/null); delete it once `make up` has run from deploy/{}/.",
			repo.join(".env").display(),
			deployment.role
		);
	}
	Ok(!report.failed())
}

/// `docker compose <args>` on `deployment`, its apps' files on tmpfs in
/// `env_dir` for as long as compose runs.
///
/// Every labelled app's file must exist: an empty stand-in would start its
/// service on nothing, which `env_file`'s `required` is there to prevent.
/// While compose runs, envctl outlives the signals that end it (Ctrl-C on
/// `logs -f` is the usual way out): SIGINT reaches compose from the terminal,
/// SIGTERM and SIGHUP are passed on, and the files are removed either way.
fn compose_on(
	repo: &Path,
	deployment: &Deployment,
	env_dir: &Path,
	args: &[String],
	shell: &IndexMap<String, String>,
	compose_variables: &compose::ComposeVariables,
) -> Result<ExitCode> {
	let missing: Vec<String> = deployment
		.apps
		.iter()
		.filter(|(_, file)| file.is_none())
		.map(|(app, _)| sops::relative(&deployment.role, app).display().to_string())
		.collect();
	if !missing.is_empty() {
		bail!(
			"{} does not exist, so its service would start on none of its variables; `make import` or `sops edit` writes it",
			missing.join(", ")
		);
	}
	let apps: IndexMap<String, sops::Plain> = deployment.apps.iter().map(|(app, file)| (app.clone(), file.clone().unwrap_or_default())).collect();
	let interrupted = Arc::new(AtomicBool::new(false));
	let ended = Arc::new(AtomicBool::new(false));
	signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted)).context("handling SIGINT")?;
	for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
		signal_hook::flag::register(signal, Arc::clone(&ended)).context("handling SIGTERM and SIGHUP")?;
	}
	let delivered = deliver::Delivered::write(env_dir, &apps)?;
	let mut process = Process::new("docker");
	process.args(["compose", "--env-file", "/dev/null"]).args(args).current_dir(repo);
	// Only the role's files set compose's variables: one the shell happens to
	// export is not a second source.
	for name in compose_variables.keys() {
		if shell.contains_key(name) && !deployment.compose.contains_key(name) {
			process.env_remove(name);
		}
	}
	process.envs(&deployment.compose);
	process.env("ENV_DIR", delivered.dir());
	let mut child = process.spawn().context("running docker compose")?;
	let status = loop {
		if let Some(status) = child.try_wait().context("waiting for docker compose")? {
			break status;
		}
		if ended.swap(false, Ordering::SeqCst) {
			if let Some(pid) = i32::try_from(child.id()).ok().and_then(rustix::process::Pid::from_raw) {
				let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
			}
		}
		std::thread::sleep(Duration::from_millis(50));
	};
	drop(delivered);
	Ok(status.code().map_or(ExitCode::FAILURE, |code| ExitCode::from(u8::try_from(code).unwrap_or(1))))
}

fn run(cli: Cli) -> Result<ExitCode> {
	let repo = find_repo(cli.repo)?;
	let schemas = schema::load(&repo.join("infra/env"))?;
	let shell = shell();
	let sops_program = sops::program(&shell);
	match cli.command {
		Command::Preflight { role } => {
			let raw = compose::config(&repo).context("docker compose config failed, so no service was checked")?;
			let deployment = deployment(&repo, &role, &raw, Path::new("$XDG_RUNTIME_DIR/server"), &sops_program)?;
			Ok(exit(!check_preflight(&repo, &schemas, &raw, &deployment)?))
		}
		Command::Up { role, env_dir } => {
			let env_dir = deliver::env_dir(env_dir.as_deref(), &shell)?;
			let raw = compose::config(&repo).context("docker compose config failed, so no service was checked")?;
			let deployment = deployment(&repo, &role, &raw, &env_dir, &sops_program)?;
			if !check_preflight(&repo, &schemas, &raw, &deployment)? {
				println!("Nothing was started.");
				return Ok(ExitCode::FAILURE);
			}
			compose_on(&repo, &deployment, &env_dir, &["up".to_owned(), "-d".to_owned()], &shell, &compose::read(&repo)?)
		}
		Command::Compose { role, env_dir, args } => {
			let env_dir = deliver::env_dir(env_dir.as_deref(), &shell)?;
			let raw = compose::config(&repo).context("docker compose config failed")?;
			let deployment = deployment(&repo, &role, &raw, &env_dir, &sops_program)?;
			compose_on(&repo, &deployment, &env_dir, &args, &shell, &compose::read(&repo)?)
		}
		Command::Run { role, app, command } => {
			let relative = sops::relative(&role, &app);
			if !repo.join(&relative).exists() {
				bail!("{} does not exist; `sops edit {}` writes it", relative.display(), relative.display());
			}
			let file = sops::set_only(sops::decrypt(&sops_program, &repo, &relative)?);
			let Some((program, args)) = command.split_first() else {
				bail!("no command to run");
			};
			let mut process = Process::new(program);
			process.args(args).current_dir(&repo);
			process.envs(file.iter().filter(|(name, _)| !shell.contains_key(*name)));
			let status = process.status().with_context(|| format!("running {program}"))?;
			Ok(status.code().map_or(ExitCode::FAILURE, |code| ExitCode::from(u8::try_from(code).unwrap_or(1))))
		}
		Command::List { role } => {
			let read = |app: &str| std::fs::read_to_string(repo.join(sops::relative(&role, app))).unwrap_or_default();
			let mut set: IndexMap<String, Vec<String>> = IndexMap::new();
			let mut unset: IndexMap<String, Vec<String>> = IndexMap::new();
			for app in schemas.iter().map(|schema| schema.binary.as_str()).chain([sops::COMPOSE]) {
				let text = read(app);
				let empty = sops::unset_names(&text);
				set.insert(app.to_owned(), sops::names(&text).into_iter().filter(|name| !empty.contains(name)).collect());
				unset.insert(app.to_owned(), empty);
			}
			// What compose's own environment: gives a labelled service is set
			// too, by compose. Without compose, the files alone say.
			if let Ok(raw) = compose::config(&repo) {
				for (_, service) in compose::services(&raw) {
					if let Some(binary) = compose::labels(service).get(compose::LABEL) {
						set.entry(binary.clone()).or_default().extend(compose::own_environment(service).into_keys());
					}
				}
			}
			let has = |map: &IndexMap<String, Vec<String>>, reader: &str, name: &str| map.get(reader).is_some_and(|names| names.iter().any(|known| known == name));
			let stated = |reader: &str, name: &str| {
				if has(&set, reader, name) {
					preflight::Stated::Set
				} else if has(&unset, reader, name) {
					preflight::Stated::Unset
				} else {
					preflight::Stated::Absent
				}
			};
			print!("{}", preflight::render_list(&schemas, &compose::read(&repo)?, &stated));
			Ok(ExitCode::SUCCESS)
		}
		Command::Check => check(&repo),
		Command::Nix { check } => nix(&repo, &schemas, check),
		Command::Prune { role, app, page, names } => {
			let Some(schema) = schemas.iter().find(|schema| schema.binary == app) else {
				bail!("infra/env/ has no {app}.json");
			};
			prune::prune(&repo, schema, &role, &names, page, &sops_program)?;
			Ok(ExitCode::SUCCESS)
		}
		Command::Import { role, containers, dry_run } => {
			import::run(&repo, &schemas, &role, &containers, dry_run, &sops_program)?;
			Ok(ExitCode::SUCCESS)
		}
	}
}

/// Without a key: each labelled service reads only its own decrypted file,
/// and every file under deploy/ is one sops wrote.
fn check(repo: &Path) -> Result<ExitCode> {
	let raw = compose::config(repo).context("docker compose config failed, so no service was checked")?;
	let mut problems = Vec::new();
	for (name, service) in compose::services(&raw) {
		if let Some(binary) = compose::labels(service).get(compose::LABEL) {
			problems.extend(compose::wired(service, binary).map(|problem| format!("service {name} {problem}")));
		} else if !compose::env_file_paths(service).is_empty() {
			problems.push(format!(
				"service {name} reads an env_file and carries no {} label, so nothing holds it to a schema",
				compose::LABEL
			));
		}
	}
	let deploy = repo.join(sops::DEPLOY);
	for role in std::fs::read_dir(&deploy).into_iter().flatten().filter_map(Result::ok) {
		if !role.path().is_dir() {
			let path = role.path();
			problems.push(format!(
				"{} is not a role directory; deploy/ holds only deploy/<role>/<app>{}",
				path.strip_prefix(repo).unwrap_or(&path).display(),
				sops::SUFFIX
			));
			continue;
		}
		for file in std::fs::read_dir(role.path()).into_iter().flatten().filter_map(Result::ok) {
			let path = file.path();
			let shown = path.strip_prefix(repo).unwrap_or(&path).display().to_string();
			if !shown.ends_with(sops::SUFFIX) {
				problems.push(format!("{shown} is not a <app>{} file; deploy/ holds only encrypted ones", sops::SUFFIX));
				continue;
			}
			let text = std::fs::read_to_string(&path).with_context(|| format!("reading {shown}"))?;
			if let Err(plain) = sops::plain_values(&text) {
				problems.push(format!("{shown} is not encrypted by sops: {} in the clear", plain.join(", ")));
			}
		}
	}
	for problem in &problems {
		println!("{problem}");
	}
	if problems.is_empty() {
		println!("each labelled service reads only its own decrypted file, and every file under deploy/ is encrypted.");
	}
	Ok(exit(!problems.is_empty()))
}

/// `nix/modules/generated/*.nix` from the schemas, or with `check` whether
/// each is what they generate.
fn nix(repo: &Path, schemas: &[Schema], check: bool) -> Result<ExitCode> {
	let out = repo.join("nix/modules/generated");
	let mut stale = Vec::new();
	for schema in schemas {
		let path = out.join(nix::file_name(&schema.binary));
		let rendered = nix::render(schema);
		if check {
			if std::fs::read_to_string(&path).ok().as_deref() != Some(rendered.as_str()) {
				stale.push(path.strip_prefix(repo).unwrap_or(&path).display().to_string());
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
