use app_config::schema::{Bound, RequiredWhen, Rules, When, SET};
pub use app_config::{LogArgs, NatsArgs, OtelArgs};
use clap::Parser;
use serde::{Deserialize, Serialize};

/// What `Config` requires only while another variable is on.
///
/// `RULES` holds it at start, before any feature reads it, naming every gap at
/// once (`app_config::schema::parse`), and the schema
/// (`infra/env/file_host.json`) carries it to `make preflight` (#504, #580).
/// The feature code checks the same again as its own guard
/// (`auth::configured_relying_party`, `mailbox::Mailbox::from_config`,
/// `build_nudge`). `MAILBOX_SUBJECT` is not here: home knows its owner (#595).
pub const REQUIRED_WHEN: &[RequiredWhen] = &[
	RequiredWhen {
		variable: "WEBAUTHN_RP_ID",
		when: "AUTH_ENABLED",
		is: "true",
	},
	RequiredWhen {
		variable: "WEBAUTHN_ORIGINS",
		when: "AUTH_ENABLED",
		is: "true",
	},
	RequiredWhen {
		variable: "MAILBOX_URL",
		when: "MAILBOX_ENABLED",
		is: "true",
	},
	RequiredWhen {
		variable: "MAILBOX_TOKEN",
		when: "MAILBOX_ENABLED",
		is: "true",
	},
	RequiredWhen {
		variable: "VAPID_PUBLIC_KEY",
		when: "NUDGE_ENABLED",
		is: "true",
	},
	RequiredWhen {
		variable: "VAPID_PRIVATE_KEY",
		when: "NUDGE_ENABLED",
		is: "true",
	},
];

/// The limits `Config`'s numbers are held to at start, while their feature is on.
///
/// Built from the same constants the feature code checks, so the two cannot
/// disagree (#580). The relation `game_feed::CADENCE.backoff_cap` has to
/// the cadences, and the mailbox's URL shape, stay the feature code's alone.
// The limits are small whole numbers, which an f64 holds exactly.
#[allow(clippy::cast_precision_loss)]
pub const BOUNDS: &[Bound] = &[Bound {
	variable: "MAILBOX_ARCHIVE_DAYS",
	min: Some(crate::mailbox::MIN_ARCHIVE_DAYS as f64),
	max: Some(crate::mailbox::MAX_ARCHIVE_DAYS as f64),
	when: &[
		When {
			variable: "MAILBOX_ENABLED",
			is: "true",
		},
		When {
			variable: "MAILBOX_ARCHIVE_TOKEN",
			is: SET,
		},
	],
}];

/// What `file_host` holds its `Config` to at start beyond what clap parses.
pub const RULES: Rules = Rules {
	required_when: REQUIRED_WHEN,
	bounds: BOUNDS,
};

/// The routines' own schedules (paulgsc/some-ui `packages/job-desk/ROUTINE.md`
/// and `packages/ui/topik/ROUTINE.md`).
pub const DEFAULT_ROUTINE_SCHEDULE: &str = "jobs 07:52 America/Los_Angeles; reflections 09:00 America/Los_Angeles";

// One bool per command-line switch is what a clap `Parser` is; grouping them
// into enums would change every flag and env var the deployment sets.
#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
// Its own filter, which compose used to supply (#617): `async_nats` logs
// every reconnect attempt at info.
#[command(mut_arg("rust_log", |arg| arg.default_value("info,async_nats=warn")))]
pub struct Config {
	/// Dev only: when the port is taken, listen on the next free one instead
	/// of failing (`listen::bind`). A flag, not a variable (#617): `make dev`
	/// passes it, so a server run from source can sit beside the production
	/// container, and no deployment file can set it. Its published port and
	/// healthcheck assume `listen::PORT`.
	#[arg(long, default_value_t = false, action = clap::ArgAction::Set, help_heading = "Server")]
	pub port_fallback: bool,

	/// Where to record the port actually listened on, as `{ port, pid }`, for
	/// `paulgsc/some-ui`'s `vite dev` to find (`listen::PortFile`). Removed on
	/// shutdown. A flag `make dev` passes, like `--port-fallback`.
	#[arg(long, help_heading = "Server")]
	pub port_file: Option<std::path::PathBuf>,

	/// Rate limit, in requests per minute, per client (`main.rs`'s
	/// `rate_limit_middleware` partitions by peer IP). See #215/#231: this
	/// field previously declared the right unit but was unread — the
	/// limiter's capacity was `max_request_size` (megabytes of upload
	/// payload) instead.
	#[arg(long, env = "RATE_LIMIT", default_value = "60", help_heading = "Server")]
	pub rate_limit: u32,

	/// Comma-separated list of allowed CORS origins for browser-facing read routes
	/// (push and sessions). Other routes deliberately allow any origin
	/// (extensions, etc.) and are unaffected by this setting.
	///
	/// Required: no list is right on every host (#406). Include the HTTPS origin
	/// the study app is served from, not only Storybook's `:6006`: service
	/// workers, `Notification`, and `PushManager` are all gated on a secure
	/// context, so a subscription is posted from an HTTPS origin.
	#[arg(long, env = "ALLOWED_ORIGINS", value_delimiter = ',', required = true, help_heading = "Server")]
	pub allowed_origins: Vec<String>,

	/// Redis URL for caching.
	///
	/// Required: no address is right on every host (#617).
	#[arg(long, env = "REDIS_URL", required = true, help_heading = "Server")]
	pub redis_url: String,

	/// The `SQLite` database, as `sqlite://<path>`.
	///
	/// The container sets its own (`infra/compose/file_host.yml`), so in
	/// `.env` this is `make dev`'s. Pointed at the directory the container
	/// mounts, a dev run opens the container's live database.
	#[arg(long, env = "DATABASE_URL", hide_env_values = true, help_heading = "Server")]
	pub database_url: String,

	// ── Study nudge ──────────────────────────────────────────────────────────
	//
	// See `docs/study-nudge.md`. Everything below is off by default: this is a
	// feature that speaks without being spoken to, and one that turns itself on
	// because a container happened to have a database is worse than one that
	// never runs.
	/// Whether the daily nudge tick runs at all.
	///
	/// With this off, the `/push` routes still work — a browser can subscribe and
	/// a manual send can be triggered — but nothing fires on a schedule. On,
	/// the server refuses to start without a usable VAPID pair. A dev run
	/// beside the container keeps it off, or a learner is nudged twice.
	#[arg(long, env = "NUDGE_ENABLED", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Study nudge")]
	pub nudge_enabled: bool,

	/// VAPID public key, base64url, uncompressed P-256 point.
	///
	/// Handed to the browser as `applicationServerKey`. It is baked into every
	/// subscription made with it, so changing it does not re-key them — it
	/// silently invalidates all of them. Treat rotation as a migration.
	/// Generate a pair with `npx web-push generate-vapid-keys`.
	#[arg(long, env = "VAPID_PUBLIC_KEY", help_heading = "Study nudge")]
	pub vapid_public_key: Option<String>,

	/// VAPID private key, base64url, the raw 32-byte P-256 scalar. Secret.
	#[arg(long, env = "VAPID_PRIVATE_KEY", hide_env_values = true, help_heading = "Study nudge")]
	pub vapid_private_key: Option<String>,

	/// IANA timezone name deciding when a day starts and when quiet hours apply.
	///
	/// Deliberately an `Option` with no default. A container is UTC unless told
	/// otherwise, and a silent UTC fallback is how an evening reminder arrives at
	/// 3am; leaving this unset is allowed but is logged as the choice it is. A
	/// name that is not an IANA zone (`America/Denvr`) stops the server at
	/// start (#407); it used to fall back to UTC.
	#[arg(long, env = "NUDGE_TIMEZONE", value_parser = parse_timezone, help_heading = "Study nudge")]
	pub nudge_timezone: Option<String>,

	/// Local hour, 0-23, at which nudges stop.
	#[arg(long, env = "NUDGE_QUIET_HOURS_START", default_value = "22", help_heading = "Study nudge")]
	pub nudge_quiet_hours_start: u32,

	/// Local hour, 0-23, at which nudges resume. May be less than the start hour;
	/// the range wraps midnight. Equal bounds mean no quiet hours at all.
	#[arg(long, env = "NUDGE_QUIET_HOURS_END", default_value = "8", help_heading = "Study nudge")]
	pub nudge_quiet_hours_end: u32,

	/// Whether the recommender ranks a provisioned session with this subject's
	/// outcome history (#289, TEL4). **Off by default, and meant to stay off
	/// until someone has looked at real data** — see `docs/study-nudge.md`,
	/// "Outcome stats and the flag that gates them", for what would justify
	/// turning it on. Off, the recommender sees every activity as unplayed,
	/// exactly as it did before `activity_outcome` existed.
	#[arg(long, env = "RECOMMENDER_USES_OUTCOMES", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Study nudge")]
	pub recommender_uses_outcomes: bool,

	/// Base URL of the study app, used to build the notification's deep link.
	/// Must end with a slash: the client builds `${BASE_URL}sessions/${id}` and
	/// the server has to produce the same shape or clicks land on the dashboard.
	///
	/// Required: no URL is right on every host (#406). The dev host sets
	/// `https://nixos.local:5173/`.
	#[arg(long, env = "APP_BASE_URL", help_heading = "Study nudge")]
	pub app_base_url: String,

	// ── Passkey auth ──────────────────────────────────────────────────────────
	//
	// See docs/identity.md, "Passkey auth". A passkey is the only way in, so
	// there is nothing else to configure: no mail server, no token signing key.
	// It is on unless `AUTH_ENABLED` says otherwise, and on, the server refuses
	// to start without `WEBAUTHN_RP_ID` and `WEBAUTHN_ORIGINS`: a server that
	// boots with nobody able to sign in looks up and serves no one.
	/// Whether passkey sign-in is on.
	///
	/// On by default. On, `WEBAUTHN_RP_ID` and `WEBAUTHN_ORIGINS` are required
	/// and the server will not start without them. `false` is for a run that
	/// needs nobody signed in: every ceremony answers `503` and every
	/// subject-scoped route `401`. `false` with `WEBAUTHN_RP_ID` set is refused
	/// as a contradiction.
	#[arg(long, env = "AUTH_ENABLED", default_value_t = true, action = clap::ArgAction::Set, help_heading = "Passkey auth")]
	pub auth_enabled: bool,

	/// The WebAuthn relying party id: the registrable domain the app is served
	/// from (`nixos.local`, `example.com`), with no scheme or port. A passkey is
	/// bound to it for good, so changing it strands every passkey made under
	/// the old one, which also makes it HOST-SPECIFIC in the strongest sense:
	/// moving to a new domain strands every passkey made on this one.
	/// Required while `AUTH_ENABLED` is on. For local development on
	/// `https://localhost:5173`, `localhost`.
	#[arg(long, env = "WEBAUTHN_RP_ID", help_heading = "Passkey auth")]
	pub webauthn_rp_id: Option<String>,

	/// Comma-separated origins the app is served from, scheme and port
	/// included (`https://nixos.local:5173`). A passkey ceremony from any
	/// other origin fails verification. Required when `WEBAUTHN_RP_ID` is set.
	#[arg(long, env = "WEBAUTHN_ORIGINS", value_delimiter = ',', help_heading = "Passkey auth")]
	pub webauthn_origins: Vec<String>,

	/// How long a sign-in lasts, in days. It slides: opening the app past the
	/// halfway point renews it to a full term.
	#[arg(long, env = "AUTH_SESSION_DAYS", default_value = "30", help_heading = "Passkey auth")]
	pub auth_session_days: u32,

	/// Whether an account may be created while one already exists. Off (the
	/// default), the first account closes sign-up: the box is one person's,
	/// and that account is the owner's, which runs the operator routes and
	/// is whose copy the Jobs mailbox is kept as, with no subject id set
	/// anywhere here. On, for a product where anyone who can reach the
	/// server may make an account; then no account is the owner's by itself,
	/// and `OPERATOR_SUBJECTS` and `MAILBOX_SUBJECT` say who is who. Sign-in
	/// is open either way.
	#[arg(long, env = "AUTH_OPEN_SIGNUP", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Passkey auth")]
	pub auth_open_signup: bool,

	/// How many accounts may be created per UTC day, across the server, with
	/// `AUTH_OPEN_SIGNUP` on. A backstop against a flood of empty accounts;
	/// the per-client rate limit already bounds any one client.
	#[arg(long, env = "AUTH_NEW_ACCOUNTS_PER_DAY", default_value = "100", help_heading = "Passkey auth")]
	pub auth_new_accounts_per_day: u32,

	/// Comma-separated subject ids allowed to use the operator routes: the
	/// ones that rewrite, retire and restore what everyone is served (lessons,
	/// `leetype` rounds). Whitespace around an entry is trimmed and empty
	/// entries are ignored. Unset or empty, the owner's account (the only
	/// one) is the operator, and nobody is while the box has no account or
	/// several. Set, only the listed subjects are: an override for a
	/// deployment with `AUTH_OPEN_SIGNUP` on; see `auth::operator`.
	///
	/// To find a subject id to list, sign in where you operate from and call
	/// any operator route: the refusal is logged at info with your `subject=`.
	#[arg(long, env = "OPERATOR_SUBJECTS", value_delimiter = ',', help_heading = "Passkey auth")]
	pub operator_subjects: Vec<String>,

	// ── The Jobs mailbox ──────────────────────────────────────────────────────
	//
	// See `mailbox`. Home reads the owner's daily digest from the mailbox (a
	// Cloudflare Worker, paulgsc/some-ui `apps/mailbox`) so the phone can take
	// it from home. Off unless `MAILBOX_ENABLED` says otherwise; on, the server
	// refuses to start unless the URL and token are set and usable.
	/// Whether home reads the Jobs mailbox at all. On, the server refuses to
	/// start unless `MAILBOX_URL` and `MAILBOX_TOKEN` are set and usable.
	#[arg(long, env = "MAILBOX_ENABLED", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Jobs mailbox")]
	pub mailbox_enabled: bool,

	/// The mailbox's base URL: `https://` and a host, no path
	/// (`https://some-ui-mailbox.<account>.workers.dev`).
	#[arg(long, env = "MAILBOX_URL", help_heading = "Jobs mailbox")]
	pub mailbox_url: Option<String>,

	/// Home's own mailbox token: it reads `brief` and puts `decisions` and
	/// `scout`. Never logged.
	#[arg(long, env = "MAILBOX_TOKEN", hide_env_values = true, help_heading = "Jobs mailbox")]
	pub mailbox_token: Option<String>,

	/// The subject whose copy the mailbox's files are kept as. Unset, the
	/// owner's account (the only one), read at each pass; while the box has
	/// no account or several, a pass keeps nothing and says `no_account`. Set,
	/// that subject: an override for a deployment with `AUTH_OPEN_SIGNUP` on,
	/// its id found as for `OPERATOR_SUBJECTS`.
	#[arg(long, env = "MAILBOX_SUBJECT", help_heading = "Jobs mailbox")]
	pub mailbox_subject: Option<String>,

	/// When each routine that leaves run records in the mailbox is due:
	/// `<routine> <HH:MM> <IANA zone>` entries separated by `;`, the routine
	/// `jobs` or `reflections`. Empty expects no run. Read only with the
	/// mailbox on (`routine_watch`).
	#[arg(long, env = "ROUTINE_SCHEDULE", default_value = DEFAULT_ROUTINE_SCHEDULE, help_heading = "Jobs mailbox")]
	pub routine_schedule: String,

	/// Home's second mailbox token (the Worker's `ARCHIVE_TOKEN`): it moves a
	/// file live there long enough home, and puts one back when the routine
	/// asks. Unset, nothing is archived. Never logged.
	#[arg(long, env = "MAILBOX_ARCHIVE_TOKEN", hide_env_values = true, help_heading = "Jobs mailbox")]
	pub mailbox_archive_token: Option<String>,

	/// Days a file stays live in the mailbox before home archives it. At
	/// least 30, the mailbox's own floor.
	#[arg(long, env = "MAILBOX_ARCHIVE_DAYS", default_value = "90", help_heading = "Jobs mailbox")]
	pub mailbox_archive_days: u64,

	// ── The Jobs arrival notice ───────────────────────────────────────────────
	//
	// See `jobs_notice`. With the mailbox on, home tells each phone that asked
	// (`PUT /jobs/notice`), through Firebase Cloud Messaging, when a morning
	// run's record arrives. Off unless this is set; set, the server refuses to
	// start unless the file is a service account's key.
	/// A Google service account's JSON key, allowed to send through FCM for
	/// the app's Firebase project, as the container sees it: the repo's
	/// `config/` is mounted at `/app/config`, and a name with `secret` in it
	/// stays out of git (`.gitignore`'s `*secret*`), e.g.
	/// `/app/config/fcm-service-account-secret.json`. Unset, home tells no
	/// phone anything; set to a file that is not such a key, the server
	/// refuses to start.
	#[arg(long, env = "FCM_SERVICE_ACCOUNT_FILE", help_heading = "Jobs arrival notice")]
	pub fcm_service_account_file: Option<std::path::PathBuf>,

	// ── Voice-note transcription ──────────────────────────────────────────────
	//
	// See `transcribe`. Home hears each voice note the phone sends it with
	// whisper.cpp on its CPU. Off unless `TRANSCRIBE_ENABLED`; on, the server
	// refuses to start unless both models load and ffmpeg runs. Not the
	// `WHISPER_*` variables an older `.env` set, which named an
	// English-only model and nothing read.
	/// Whether home transcribes voice notes at all. On, the server refuses to
	/// start unless both models load and ffmpeg runs. Get the models with
	/// `nix develop .#whisper -c whisper fetch`.
	#[arg(long, env = "TRANSCRIBE_ENABLED", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Voice-note transcription")]
	pub transcribe_enabled: bool,

	// ── Rust Rounds recording ─────────────────────────────────────────────────
	//
	// See `rust_rounds`. Home hands each program of each round the phone puts
	// to the rust-runner, a sandboxed container that compiles and runs it, over
	// a Unix socket on a volume both mount, and keeps how each ended. Off
	// unless `RUST_ROUNDS_ENABLED`.
	/// Whether home records Rust Rounds' programs. On, the rust-runner's
	/// socket must be mounted at `rust_rounds::SOCKET`; a runner that is not
	/// there is reported each pass, and nothing is recorded until it is.
	#[arg(long, env = "RUST_ROUNDS_ENABLED", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Rust Rounds recording")]
	pub rust_rounds_enabled: bool,

	// ── The NFL game feed ─────────────────────────────────────────────────────
	//
	// See `game_feed`. Home follows the games of the week through ESPN's public
	// site API and keeps every response and the play log read from them. Off
	// unless `GAME_FEED_ENABLED`; on, the server refuses to start with a cadence
	// out of range.
	/// Whether home follows the games of the week at all.
	#[arg(long, env = "GAME_FEED_ENABLED", default_value_t = false, action = clap::ArgAction::Set, help_heading = "NFL game feed")]
	pub game_feed_enabled: bool,

	// ── Shared with the other binaries (`app_config`) ─────────────────────────
	/// `NATS_URL`. The container sets it (`infra/compose/file_host.yml`).
	#[command(flatten)]
	pub nats: NatsArgs,

	#[command(flatten)]
	pub log: LogArgs,

	#[command(flatten)]
	pub otel: OtelArgs,

	/// Perform health check and exit
	#[arg(long, help = "Perform health check against running server", help_heading = "Health check")]
	pub health_check: bool,

	/// Port for health check (defaults to `listen::PORT`)
	#[arg(long, default_value_t = crate::listen::PORT, help = "Port to check for health endpoint", help_heading = "Health check")]
	pub health_check_port: u16,

	/// Loopback host for the health check (defaults to 127.0.0.1); any other
	/// is refused, since it asks over plain HTTP (`health`).
	#[arg(long, default_value = "127.0.0.1", help = "Loopback host to check for the readiness endpoint", help_heading = "Health check")]
	pub health_check_host: String,
}

#[derive(clap::ValueEnum, Clone, Debug, Serialize, Deserialize)]
pub enum ProgramMode {
	Run,
	PopulateGameClocks,
}

impl Config {
	pub fn new() -> Self {
		Self::parse()
	}

	pub fn as_cache_config(&self) -> some_cache::CacheConfig {
		some_cache::CacheConfig::new(self.redis_url.clone()).with_ttl(crate::cache::CACHE_TTL)
	}
}

/// Refuses a name `chrono-tz` does not know, so a misspelled `NUDGE_TIMEZONE`
/// stops the server at start, naming the variable, instead of running the
/// nudge on UTC (#407).
fn parse_timezone(name: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
	name
		.parse::<chrono_tz::Tz>()
		.map(|_| name.to_owned())
		.map_err(|_| "not an IANA zone name, e.g. America/Denver".into())
}

/// Variables a retired setup left set that nothing reads any more.
///
/// The retired transcriber's `WHISPER_*` three (#407), and
/// `OTEL_SERVICE_NAME`, which the binary's own name replaced (#617). Startup
/// warns with their names, so a stale deployment file is never silent
/// (#476).
///
/// The one read around clap (#484): these are declared nowhere, and are
/// looked for only so a stale file is named.
#[must_use]
#[allow(clippy::disallowed_methods)]
pub fn retired_variables_set() -> Vec<&'static str> {
	retired_variables_in(|name| std::env::var(name).ok())
}

/// [`retired_variables_set`] over `var`, which reads one variable.
fn retired_variables_in(var: impl Fn(&str) -> Option<String>) -> Vec<&'static str> {
	["WHISPER_MODELS_PATH", "WHISPER_MODEL", "WHISPER_THREADS", "OTEL_SERVICE_NAME"]
		.into_iter()
		.filter(|name| var(name).is_some())
		.collect()
}

#[cfg(test)]
mod tests {
	use super::{retired_variables_in, Config};
	use clap::{CommandFactory, Parser};

	#[test]
	fn config_definition_is_valid() {
		Config::command().debug_assert();
	}

	const REFUSED: [(&str, &str); 16] = [
		("RUST_LOG", "info,=[bad"),
		("RATE_LIMIT", "-1"),
		("NUDGE_ENABLED", "on"),
		("NUDGE_QUIET_HOURS_START", "10pm"),
		("NUDGE_QUIET_HOURS_END", "8am"),
		("RECOMMENDER_USES_OUTCOMES", "on"),
		("AUTH_ENABLED", "off"),
		("AUTH_OPEN_SIGNUP", "on"),
		("AUTH_SESSION_DAYS", "a month"),
		("AUTH_NEW_ACCOUNTS_PER_DAY", "-1"),
		("MAILBOX_ENABLED", "on"),
		("MAILBOX_ARCHIVE_DAYS", "90d"),
		("TRANSCRIBE_ENABLED", "on"),
		("RUST_ROUNDS_ENABLED", "on"),
		("GAME_FEED_ENABLED", "on"),
		("NUDGE_TIMEZONE", "America/Denvr"),
	];
	// The URLs, keys and paths are checked by what uses them, which refuses
	// to start on a bad one, as the mailbox does `ROUTINE_SCHEDULE`.
	const FREE_TEXT: [&str; 18] = [
		"ALLOWED_ORIGINS",
		"REDIS_URL",
		"NATS_URL",
		"DATABASE_URL",
		"VAPID_PUBLIC_KEY",
		"VAPID_PRIVATE_KEY",
		"APP_BASE_URL",
		"WEBAUTHN_RP_ID",
		"WEBAUTHN_ORIGINS",
		"OPERATOR_SUBJECTS",
		"MAILBOX_URL",
		"MAILBOX_TOKEN",
		"MAILBOX_SUBJECT",
		"MAILBOX_ARCHIVE_TOKEN",
		"ROUTINE_SCHEDULE",
		"FCM_SERVICE_ACCOUNT_FILE",
		"OTEL_EXPORTER_OTLP_ENDPOINT",
		"ENVIRONMENT",
	];

	/// ENV1 (README.md, "Configuration"); see `app_config::env1`.
	#[test]
	fn every_variable_that_does_not_parse_is_refused_by_name() {
		app_config::env1::assert_every_variable_is_refused_by_name::<Config>("file_host", &REFUSED, &FREE_TEXT);
	}

	/// CFG7 (#503): `infra/env/file_host.json` is this `Config`.
	#[test]
	fn the_checked_in_schema_is_this_config() {
		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../infra/env/file_host.json");
		app_config::schema::assert_checked_in("file_host", &Config::command(), &super::RULES, &path);
		app_config::schema::assert_groups_hold_only_their_own(&Config::command());
	}

	/// The `.env` #476 met: every retired variable left set is named.
	#[test]
	fn a_retired_transcriber_variable_left_set_is_named() {
		let stale = |name: &str| matches!(name, "WHISPER_MODELS_PATH" | "WHISPER_MODEL" | "WHISPER_THREADS" | "OTEL_SERVICE_NAME").then(String::new);
		assert_eq!(
			retired_variables_in(stale),
			["WHISPER_MODELS_PATH", "WHISPER_MODEL", "WHISPER_THREADS", "OTEL_SERVICE_NAME"]
		);
	}

	#[test]
	fn a_current_env_names_no_retired_variable() {
		assert!(retired_variables_in(|_| None).is_empty());
	}

	/// `AUTH_ENABLED` is on by default, so turning it off has to be possible by
	/// value, which a plain `bool` flag (`SetTrue`) would not allow.
	#[test]
	fn auth_enabled_can_be_turned_off_by_value() {
		let required = [
			"file_host",
			"--database-url",
			"sqlite::memory:",
			"--allowed-origins",
			"https://app.example",
			"--app-base-url",
			"https://app.example/",
			"--nats-url",
			"nats://nats:4222",
			"--redis-url",
			"redis://redis:6379",
		];
		let off = Config::try_parse_from(required.iter().copied().chain(["--auth-enabled", "false"])).unwrap();
		assert!(!off.auth_enabled);
		let on = Config::try_parse_from(required.iter().copied().chain(["--auth-enabled", "true"])).unwrap();
		assert!(on.auth_enabled);
	}
}
