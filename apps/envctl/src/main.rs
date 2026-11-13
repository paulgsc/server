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

mod caps;
mod classes;
mod compose;
mod conform;
mod deliver;
mod deploy;
mod dotenv;
mod import;
mod nix;
mod preflight;
mod prune;
mod release;
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
	/// decrypted file, every file under deploy/ is encrypted, and every
	/// variable is a deployment constant or on its way out (ENV-CLASS).
	Check {
		/// The base branch's `infra/env-classes.json`, which the checkout's
		/// may not outgrow (CI passes it).
		#[arg(long)]
		base_classes: Option<PathBuf>,
	},
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
	/// Home's deploy of `file_host` (AS18, #564; `make home-deploy`): pull
	/// pgathondu/server:latest and, only when it changed, `make up`, wait for
	/// it to be ready, and put the previous image back when it is not. With
	/// --on-demand, only when `file_host`'s marker asks for a newer contract
	/// epoch (`nix/modules/home-deploy.nix`). Exit 0 deployed or nothing to
	/// do, 1 not deployed, 2 rolled back, 3 nothing ready.
	Deploy {
		/// Use the images already here.
		#[arg(long = "no-pull")]
		no_pull: bool,
		/// Seconds to wait for each start to be ready.
		#[arg(long, default_value_t = deploy::DEFAULT_READY_TIMEOUT)]
		ready_timeout: f64,
		/// Where the failed images are remembered (default
		/// `$XDG_STATE_HOME/server-deploy`).
		#[arg(long)]
		state_dir: Option<PathBuf>,
		/// The image file-host runs, as `infra/compose/file_host.yml` names it.
		#[arg(long, default_value = deploy::IMAGE)]
		image: String,
		/// Run only if `DEPLOY_DIR/wanted` asks (`${STORAGE}/deploy`).
		#[arg(long, value_name = "DEPLOY_DIR")]
		on_demand: Option<PathBuf>,
		/// Run make as this user (the on-demand unit runs as root).
		#[arg(long)]
		as_user: Option<String>,
		/// With --as-user: a tmpfs directory for envctl, chowned to the user.
		#[arg(long, requires = "as_user")]
		runtime_dir: Option<PathBuf>,
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

/// Each `<app>.sops.env` in deploy/<role>/ but compose's own and the tools
/// file, which no service reads (#634).
fn files_of(repo: &Path, role: &str) -> Vec<String> {
	let mut apps: Vec<String> = std::fs::read_dir(repo.join(sops::DEPLOY).join(role))
		.into_iter()
		.flatten()
		.filter_map(|entry| entry.ok()?.file_name().into_string().ok())
		.filter_map(|name| name.strip_suffix(sops::SUFFIX).map(str::to_owned))
		.filter(|app| app != sops::COMPOSE && app != sops::TOOLS)
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
	// Every image compose starts, from the role's release (#788), never from
	// a deployment file or the shell.
	if let Some(release) = release::read(repo, role)? {
		compose.extend(release.variables());
	}
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
	let shell = shell();
	// The deploy reads no schema: it pulls, runs `make up` and waits.
	if matches!(cli.command, Command::Deploy { .. }) {
		return run_deploy(repo, &shell, cli.command);
	}
	let schemas = schema::load(&repo.join("infra/env"))?;
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
			// Only `up` runs on the box it deploys, so only its MemTotal is home's.
			let meminfo = std::fs::read_to_string("/proc/meminfo").ok();
			// The profiles `up` starts with: the deployment's, or else the shell's,
			// which reaches compose unless a compose file reads the variable.
			let profiles = deployment.compose.get("COMPOSE_PROFILES").cloned().or_else(|| shell.get("COMPOSE_PROFILES").cloned());
			for warning in caps::warnings(&caps::caps(&raw, &caps::profiles(profiles.as_deref())), meminfo.as_deref().and_then(caps::mem_total)) {
				println!("warning: {warning}");
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
		Command::Check { base_classes } => check(&repo, &schemas, base_classes.as_deref()),
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
		Command::Deploy { .. } => unreachable!("handled above, before the schemas are read"),
	}
}

/// `envctl deploy`: one pass, or with `on_demand` the marker's.
fn run_deploy(repo: PathBuf, shell: &IndexMap<String, String>, command: Command) -> Result<ExitCode> {
	let Command::Deploy {
		no_pull,
		ready_timeout,
		state_dir,
		image,
		on_demand,
		as_user,
		runtime_dir,
	} = command
	else {
		bail!("not a deploy");
	};
	let options = deploy::Options {
		pull: !no_pull,
		image,
		ready_timeout,
		poll: deploy::POLL_SECONDS,
	};
	if options.ready_timeout <= 0.0 || !options.ready_timeout.is_finite() {
		bail!("--ready-timeout must be positive");
	}
	let as_user = match as_user.as_deref() {
		Some(user) => deploy::runuser_prefix(user, runtime_dir.as_deref())?,
		None => Vec::new(),
	};
	let state_dir = state_dir.unwrap_or_else(|| deploy::default_state_dir(shell.get("XDG_STATE_HOME").map(String::as_str), shell.get("HOME").map(String::as_str)));
	let mut host = deploy::RealHost::new(repo, &state_dir, as_user);
	let clock = deploy::Monotonic::new();
	let mut say = |line: &str| println!("{line}");
	let code = if let Some(folder) = on_demand {
		let mut demand = deploy::RealDemand::new(folder);
		let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			deploy::on_demand(&mut host, &mut demand, &options, &deploy::wall_now, &clock, &mut say)
		}));
		match ran {
			Ok(Ok(code)) => code,
			failed => {
				// Never leave the marker behind a crash: the path unit would
				// start this again at once. The failure is recorded for the
				// alert, and the reason goes to the journal.
				deploy::record_crash(&mut demand, deploy::wall_now());
				if let Ok(Err(err)) = failed {
					eprintln!("envctl deploy: {err:#}");
				}
				deploy::EXIT_NOT_DEPLOYED
			}
		}
	} else {
		deploy::deploy(&mut host, &options, &clock, &mut say)
	};
	Ok(ExitCode::from(code))
}

/// Conformance without a key (ENV3, #616): every problem in the deployment
/// files, compose's wiring, the binaries' schemas and the tracked files, by
/// name. Nothing is decrypted, and no value is read.
fn check(repo: &Path, schemas: &[Schema], base_classes: Option<&Path>) -> Result<ExitCode> {
	let raw = compose::config(repo).context("docker compose config failed, so no service was checked")?;
	let mut roles = Vec::new();
	let mut problems = Vec::new();
	let mut entries: Vec<PathBuf> = std::fs::read_dir(repo.join(sops::DEPLOY))
		.into_iter()
		.flatten()
		.filter_map(|entry| Some(entry.ok()?.path()))
		.collect();
	entries.sort();
	for dir in entries {
		let shown = dir.strip_prefix(repo).unwrap_or(&dir).display().to_string();
		let Some(role) = dir.file_name().and_then(|name| name.to_str()).filter(|_| dir.is_dir()).map(str::to_owned) else {
			problems.push(format!("{shown} is not a role directory; deploy/ holds only deploy/<role>/<app>{}", sops::SUFFIX));
			continue;
		};
		let mut files = Vec::new();
		let mut release_text = None;
		let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
			.with_context(|| format!("reading {shown}"))?
			.filter_map(|entry| Some(entry.ok()?.path()))
			.collect();
		paths.sort();
		for path in paths {
			let shown = path.strip_prefix(repo).unwrap_or(&path).display().to_string();
			let app = path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_suffix(sops::SUFFIX));
			match app {
				// The release, the one plain file: image references are not
				// secret (#788).
				None if path.is_file() && path.file_name().is_some_and(|name| name == release::FILE) => {
					release_text = Some(std::fs::read_to_string(&path).with_context(|| format!("reading {shown}"))?);
				}
				Some(app) if path.is_file() => {
					let text = std::fs::read_to_string(&path).with_context(|| format!("reading {shown}"))?;
					files.push((app.to_owned(), text));
				}
				_ => problems.push(format!(
					"{shown} is not a <app>{} file; deploy/ holds only encrypted ones and the role's {}",
					sops::SUFFIX,
					release::FILE
				)),
			}
		}
		let compose_file = files.iter().find(|(app, _)| app == sops::COMPOSE).map(|(_, text)| text.as_str());
		problems.extend(release::role_problems(&role, &raw, release_text.as_deref(), compose_file));
		roles.push(conform::Role { name: role, files });
	}
	problems.extend(release::line_problems(&raw));
	let compose_variables = compose::read(repo)?;
	let images = compose::services(&raw).into_iter().map(|(service, _)| release::variable(service)).collect();
	problems.extend(conform::problems(schemas, &compose_variables, &raw, &roles));
	let base = base_classes
		.map(|path| {
			std::fs::read_to_string(path)
				.with_context(|| format!("reading {}", path.display()))
				.and_then(|text| classes::Exceptions::parse(&text))
		})
		.transpose()?;
	problems.extend(classes::problems(schemas, &compose_variables, &images, &classes::Exceptions::load(repo)?, base.as_ref()));
	let tracked = Process::new("git").args(["ls-files", "-z"]).current_dir(repo).output().context("running git ls-files")?;
	if !tracked.status.success() {
		bail!("git ls-files failed: {}", String::from_utf8_lossy(&tracked.stderr).trim());
	}
	for path in String::from_utf8_lossy(&tracked.stdout).split('\0').filter(|path| conform::plain_env_file(path)) {
		problems.push(format!(
			"{path} is tracked and is a plain-text env file; every value is encrypted (deploy/<role>/<app>{})",
			sops::SUFFIX
		));
	}
	let metadata = Process::new("cargo")
		.args(["metadata", "--format-version", "1", "--no-deps", "--offline"])
		.current_dir(repo)
		.output()
		.context("running cargo metadata")?;
	if !metadata.status.success() {
		bail!("cargo metadata failed: {}", String::from_utf8_lossy(&metadata.stderr).trim());
	}
	let metadata: Value = serde_json::from_slice(&metadata.stdout).context("cargo metadata printed no JSON")?;
	for binary in conform::configured_binaries(&metadata) {
		if !schemas.iter().any(|schema| schema.binary == binary) {
			problems.push(format!(
				"{binary} reads its environment through app-config and has no infra/env/{binary}.json; check its schema in (app_config::schema::assert_checked_in)"
			));
		}
	}
	for problem in &problems {
		println!("{problem}");
	}
	if problems.is_empty() {
		println!("every deployment file is sops-encrypted and names only what its binary or compose reads; each labelled service reads only its own file; every configured binary has its schema; no plain-text env file is tracked; every variable is a deployment constant or on its way out.");
	} else {
		println!("{} problem(s), named by variable and file only (ENV3, #616).", problems.len());
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
