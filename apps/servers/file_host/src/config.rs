use app_config::schema::RequiredWhen;
pub use app_config::{LogArgs, NatsArgs, OtelArgs, TraceSampler, DEV_HOST_NATS_URL};
use clap::Parser;
use serde::{Deserialize, Serialize};

// HOST-SPECIFIC defaults (#406). Each one is right on the dev host this server
// grew up on (`nixos.local`, with NATS and Redis on the same machine) and wrong
// on any other. They stay as defaults so that host keeps working with an
// unchanged `.env`, but they are named, marked, and announced at startup
// (`Config::host_specific_defaults_in_use`), so a new host finds every value
// it must set instead of inheriting this one's silently.

/// `ALLOWED_ORIGINS`' default. HOST-SPECIFIC: the `nixos.local` dev host.
pub const DEV_HOST_ALLOWED_ORIGINS: &str = "https://nixos.local:5173,https://localhost:5173,http://nixos.local:6006";
/// `APP_BASE_URL`'s default. HOST-SPECIFIC: the `nixos.local` dev host.
pub const DEV_HOST_APP_BASE_URL: &str = "https://nixos.local:5173/";
/// Used when `REDIS_URL` is unset. HOST-SPECIFIC: a Redis on this machine.
pub const DEV_HOST_REDIS_URL: &str = "redis://127.0.0.1:6379";

/// What `Config` requires only while another variable is on.
///
/// Each is checked where the server refuses to start without it
/// (`auth::configured_relying_party`, `mailbox::Mailbox::from_config`,
/// `build_nudge`), and declared here so the schema
/// (`infra/env/file_host.json`) and `make preflight` know it (#504).
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

/// The routines' own schedules (paulgsc/some-ui `packages/job-desk/ROUTINE.md`
/// and `packages/ui/topik/ROUTINE.md`).
pub const DEFAULT_ROUTINE_SCHEDULE: &str = "jobs 07:52 America/Los_Angeles; reflections 09:00 America/Los_Angeles";

// One bool per command-line switch is what a clap `Parser` is; grouping them
// into enums would change every flag and env var the deployment sets.
#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
#[command(mut_arg("otel_service_name", app_config::default_service_name("file_host")))]
pub struct Config {
	/// Database connection timeout in seconds
	#[arg(long, env = "CONNECTION_TIMEOUT", default_value = "30", help_heading = "Server")]
	pub connection_timeout: u64,

	/// Max number of concurrent fetches to deduplicate. Older entries are evicted when exceeded.
	#[arg(long, env = "MAX_IN_FLIGHT", default_value = "256", help_heading = "Server")]
	pub max_in_flight: u64,

	/// Address to listen on.
	///
	/// `FILE_HOST_BIND`, not `HOST`: zsh exports `HOST` as the machine's name.
	/// These two were once `HOST`/`PORT` and read by nothing (the listener was
	/// hard-coded to `0.0.0.0:3000`); the defaults are that address, so honouring
	/// them changes nothing for a server started without them.
	#[arg(long = "bind", env = "FILE_HOST_BIND", value_name = "FILE_HOST_BIND", default_value = "0.0.0.0", help_heading = "Server")]
	pub host: String,

	/// Port to listen on.
	///
	/// `FILE_HOST_PORT`, not `PORT`: the container reads the repo's `.env`, and
	/// a bare `PORT` there meant for another service would move this one.
	#[arg(long, env = "FILE_HOST_PORT", value_name = "FILE_HOST_PORT", default_value = "3000", help_heading = "Server")]
	pub port: u16,

	/// Dev only: when the port is taken, listen on the next free one instead
	/// of failing (`listen::bind`). `make dev` sets it, so a server run from
	/// source can sit beside the production container. Never set it in
	/// production: its published port and healthcheck assume `FILE_HOST_PORT`.
	#[arg(long, env = "FILE_HOST_PORT_FALLBACK", value_name = "FILE_HOST_PORT_FALLBACK", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Server")]
	pub port_fallback: bool,

	/// Where to record the port actually listened on, as `{ port, pid }`, for
	/// `paulgsc/some-ui`'s `vite dev` to find (`listen::PortFile`). Removed on
	/// shutdown. `make dev` sets it.
	#[arg(long, env = "FILE_HOST_PORT_FILE", value_name = "FILE_HOST_PORT_FILE", help_heading = "Server")]
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
	/// The default carries three entries rather than one because the study app is
	/// not on `:6006` — that is Storybook. Service workers, `Notification`, and
	/// `PushManager` are all gated on a secure context, so the origin a
	/// subscription is actually posted from is an HTTPS one, and a list that only
	/// admits the Storybook origin blocks it.
	///
	/// HOST-SPECIFIC default: the `nixos.local` dev host. Set it on any other.
	#[arg(long, env = "ALLOWED_ORIGINS", value_delimiter = ',', default_value = DEV_HOST_ALLOWED_ORIGINS, help_heading = "Server")]
	pub allowed_origins: Vec<String>,

	/// Redis URL for caching.
	///
	/// Unset, a Redis on this machine (HOST-SPECIFIC; see `DEV_HOST_REDIS_URL`).
	#[arg(long, env = "REDIS_URL", help_heading = "Server")]
	pub redis_url: Option<String>,

	/// Cache TTL in seconds
	#[arg(long, env = "CACHE_TTL", default_value = "600", help_heading = "Server")]
	pub cache_ttl: u64,

	/// Prometheus metrics port
	#[arg(long, env = "PROMETHEUS_PORT", default_value = "9090", help_heading = "Server")]
	pub prometheus_port: u16,

	/// Streaming Chunk Size
	#[arg(long, env = "BUFFER_SIZE", value_name = "BUFFER_SIZE", default_value = "65536", help_heading = "Server")]
	pub chunk_size: usize,

	/// Individual request payload lmit in MB
	#[arg(long, env = "MAX_REQUEST_SIZE_MB", value_name = "MAX_REQUEST_SIZE_MB", default_value = "10", help_heading = "Server")]
	pub max_request_size: usize,

	/// Active request limit
	#[arg(long, env = "MAX_CONCURRENT_REQUESTS", value_name = "MAX_CONCURRENT_REQUESTS", default_value = "100", help_heading = "Server")]
	pub max_concurrent_req: usize,

	/// Hard timeout for any operation
	#[arg(long, env = "TASK_TIMEOUT_MS", default_value = "15000", help_heading = "Server")]
	pub task_timeout_ms: u64,

	/// Streaming Max Chunk Size
	#[arg(long, env = "MAX_CHUNKS_IN_FLIGHT", value_name = "MAX_CHUNKS_IN_FLIGHT", default_value = "5", help_heading = "Server")]
	pub max_chunks: usize,

	/// OBS Websocket Server IP Address
	#[arg(long, env = "OBS_WEBSOCKET_IP", value_name = "OBS_WEBSOCKET_IP", help_heading = "Server")]
	pub obs_host: String,

	/// OBS Websocket Server Password
	#[arg(long, env = "OBS_WEBSOCKET_PWD", value_name = "OBS_WEBSOCKET_PWD", hide_env_values = true, help_heading = "Server")]
	pub obs_password: String,

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

	/// The VAPID `sub` claim: a `mailto:` or an origin URL identifying the sender.
	/// Push services reject or deprioritise senders without one.
	#[arg(long, env = "VAPID_SUBJECT", default_value = "mailto:admin@maishatu.com", help_heading = "Study nudge")]
	pub vapid_subject: String,

	/// IANA timezone name deciding when a day starts and when quiet hours apply.
	///
	/// Deliberately an `Option` with no default. A container is UTC unless told
	/// otherwise, and a silent UTC fallback is how an evening reminder arrives at
	/// 3am; leaving this unset is allowed but is logged as the choice it is.
	#[arg(long, env = "NUDGE_TIMEZONE", help_heading = "Study nudge")]
	pub nudge_timezone: Option<String>,

	/// Local hour, 0-23, at which nudges stop.
	#[arg(long, env = "NUDGE_QUIET_HOURS_START", default_value = "22", help_heading = "Study nudge")]
	pub nudge_quiet_hours_start: u32,

	/// Local hour, 0-23, at which nudges resume. May be less than the start hour;
	/// the range wraps midnight. Equal bounds mean no quiet hours at all.
	#[arg(long, env = "NUDGE_QUIET_HOURS_END", default_value = "8", help_heading = "Study nudge")]
	pub nudge_quiet_hours_end: u32,

	/// How often the waker picks up work the engine already decided.
	///
	/// Not a schedule. Eligibility instants are solved when signals arrive and
	/// written to an index; this is the resolution at which they are noticed, so
	/// shortening it lands interventions closer to their computed instant
	/// without changing which ones happen.
	#[arg(long, env = "NUDGE_WAKER_SECONDS", default_value = "300", help_heading = "Study nudge")]
	pub nudge_waker_seconds: u64,

	/// How long one push delivery may take before it counts as a failure
	/// (#264, SLI3).
	///
	/// A push service that accepts answers in well under a second; one that
	/// has not answered in ten is not going to, and every second past that is
	/// a second the rest of the batch waits in the serial loop. Long enough
	/// that a slow-but-working provider on a bad day is not misread as down,
	/// short enough that one black-holed endpoint costs a pass seconds rather
	/// than the transport's own (unbounded) patience. A timed-out delivery is
	/// recorded as a failure — never a prune, never `Accepted` — and does not
	/// hand the claim back: see `nudge::waker::actuate`.
	#[arg(long, env = "PUSH_DELIVERY_TIMEOUT_MS", default_value = "10000", help_heading = "Study nudge")]
	pub push_delivery_timeout_ms: u64,

	/// How long one waker pass may run before it stops taking on new subjects
	/// (#264, SLI3).
	///
	/// Two minutes: well inside `NUDGE_WAKER_SECONDS`' default of five, so a
	/// pass that hits it still leaves the next tick to start on time rather
	/// than be delayed by `MissedTickBehavior::Delay`, and long enough for a
	/// full `BATCH` of 32 subjects at typical sub-second delivery latency to
	/// finish many times over. A pass that runs out starts no further subject
	/// and tries no further device — each delivery's timeout is also capped at
	/// what the pass has left — but never cancels a subject part-way, so a
	/// claim already written stands. Subjects it did not reach are still due on
	/// the next pass, the property `BATCH` already relies on.
	#[arg(long, env = "WAKER_PASS_DEADLINE_MS", default_value = "120000", help_heading = "Study nudge")]
	pub waker_pass_deadline_ms: u64,

	/// Whether the recommender ranks a provisioned session with this subject's
	/// outcome history (#289, TEL4). **Off by default, and meant to stay off
	/// until someone has looked at real data** — see `docs/study-nudge.md`,
	/// "Outcome stats and the flag that gates them", for what would justify
	/// turning it on. Off, the recommender sees every activity as unplayed,
	/// exactly as it did before `activity_outcome` existed.
	#[arg(long, env = "RECOMMENDER_USES_OUTCOMES", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Study nudge")]
	pub recommender_uses_outcomes: bool,

	/// How long a client-written presence lease stays fresh before a due
	/// subject's notification is sent as though nobody were looking.
	///
	/// Renamed from `NUDGE_PRESENCE_FRESHNESS_SECONDS` (default `120`) when
	/// presence stopped being a WebSocket connection's staleness window and
	/// became a lease TTL: same shape of question, a different mechanism
	/// underneath, and a smaller default — a client renewing roughly every
	/// 45s while visible only needs a TTL wide enough to absorb timer
	/// throttling and scheduling jitter, not two minutes of slack.
	#[arg(long, env = "NUDGE_PRESENCE_LEASE_TTL_SECONDS", default_value = "75", help_heading = "Study nudge")]
	pub nudge_presence_lease_ttl_seconds: u64,

	/// Base URL of the study app, used to build the notification's deep link.
	/// Must end with a slash: the client builds `${BASE_URL}sessions/${id}` and
	/// the server has to produce the same shape or clicks land on the dashboard.
	///
	/// HOST-SPECIFIC default: the `nixos.local` dev host. Set it on any other.
	#[arg(long, env = "APP_BASE_URL", default_value = DEV_HOST_APP_BASE_URL, help_heading = "Study nudge")]
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

	/// Seconds between reads of the mailbox. At least 60.
	#[arg(long, env = "MAILBOX_SECONDS", default_value = "900", help_heading = "Jobs mailbox")]
	pub mailbox_seconds: u64,

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
	// `WHISPER_*` variables an older `example.env` set, which named an
	// English-only model and nothing read.
	/// Whether home transcribes voice notes at all. On, the server refuses to
	/// start unless both models load and ffmpeg runs. Get the models with
	/// `nix develop .#whisper -c whisper fetch`.
	#[arg(long, env = "TRANSCRIBE_ENABLED", default_value_t = false, action = clap::ArgAction::Set, help_heading = "Voice-note transcription")]
	pub transcribe_enabled: bool,

	/// The multilingual whisper.cpp model, as a path inside the container.
	/// The default is where `infra/compose/file_host.yml` mounts the host's
	/// models directory (HOST-SPECIFIC there), with the model `nix/whisper-manager`
	/// downloads.
	#[arg(long, env = "TRANSCRIBE_MODEL", default_value = "/models/ggml-small-q5_1.bin", help_heading = "Voice-note transcription")]
	pub transcribe_model: String,

	/// The Silero VAD model that finds a note's speech, mounted beside it.
	#[arg(
		long,
		env = "TRANSCRIBE_VAD_MODEL",
		default_value = "/models/ggml-silero-v5.1.2.bin",
		help_heading = "Voice-note transcription"
	)]
	pub transcribe_vad_model: String,

	/// Threads one note is heard with. Unset, one fewer than the cores the
	/// container may use, and at least one.
	#[arg(long, env = "TRANSCRIBE_THREADS", help_heading = "Voice-note transcription")]
	pub transcribe_threads: Option<usize>,

	/// Seconds one note may take, decoding included. At least 60.
	#[arg(long, env = "TRANSCRIBE_DEADLINE_SECONDS", default_value = "600", help_heading = "Voice-note transcription")]
	pub transcribe_deadline_seconds: u64,

	/// Seconds between looks for notes to hear. At least 10.
	#[arg(long, env = "TRANSCRIBE_SECONDS", default_value = "60", help_heading = "Voice-note transcription")]
	pub transcribe_seconds: u64,

	// ── The NFL game feed ─────────────────────────────────────────────────────
	//
	// See `game_feed`. Home follows the games of the week through ESPN's public
	// site API and keeps every response and the play log read from them. Off
	// unless `GAME_FEED_ENABLED`; on, the server refuses to start with a cadence
	// out of range.
	/// Whether home follows the games of the week at all.
	#[arg(long, env = "GAME_FEED_ENABLED", default_value_t = false, action = clap::ArgAction::Set, help_heading = "NFL game feed")]
	pub game_feed_enabled: bool,

	/// Seconds between reads of the week's scoreboard. At least 60.
	#[arg(long, env = "GAME_FEED_SCOREBOARD_SECONDS", default_value = "600", help_heading = "NFL game feed")]
	pub game_feed_scoreboard_seconds: u64,

	/// Seconds between reads of a live game's play-by-play. At least 5.
	#[arg(long, env = "GAME_FEED_LIVE_SECONDS", default_value = "15", help_heading = "NFL game feed")]
	pub game_feed_live_seconds: u64,

	/// The longest a refused or failed fetch waits before the next try. At
	/// least both cadences above, and at most 86400 (a day).
	#[arg(long, env = "GAME_FEED_BACKOFF_CAP_SECONDS", default_value = "900", help_heading = "NFL game feed")]
	pub game_feed_backoff_cap_seconds: u64,

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

	/// Port for health check (defaults to 3000)
	#[arg(long, default_value = "3000", help = "Port to check for health endpoint", help_heading = "Health check")]
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
		some_cache::CacheConfig::new(self.redis_url.clone().unwrap_or_else(|| DEV_HOST_REDIS_URL.into())).with_ttl(self.cache_ttl)
	}

	/// The variables this run is taking from a HOST-SPECIFIC default (#406):
	/// values that are right on the `nixos.local` dev host and wrong anywhere
	/// else. Startup logs them, so running on them is never silent.
	#[must_use]
	pub fn host_specific_defaults_in_use(&self) -> Vec<&'static str> {
		host_specific_defaults_in_use(&self.allowed_origins, &self.app_base_url, &self.nats.nats_url, self.redis_url.as_deref())
	}
}

/// Variables the retired transcriber's setup (#407) left set in `.env`.
///
/// The `WHISPER_*` three, which nothing reads, and `OTEL_SERVICE_NAME` set to
/// `transcriber`, which renames `file_host`'s traces. Startup warns with
/// their names, so a stale `.env` is never silent (#476).
///
/// The one read around clap (#484): the `WHISPER_*` three are declared
/// nowhere, and are looked for only so a stale `.env` is named.
/// `otel_service_name` is the `Config`'s.
#[must_use]
#[allow(clippy::disallowed_methods)]
pub fn retired_variables_set(otel_service_name: &str) -> Vec<&'static str> {
	retired_variables_in(|name| std::env::var(name).ok(), otel_service_name)
}

/// [`retired_variables_set`] over `var`, which reads one variable.
fn retired_variables_in(var: impl Fn(&str) -> Option<String>, otel_service_name: &str) -> Vec<&'static str> {
	let mut set: Vec<&'static str> = ["WHISPER_MODELS_PATH", "WHISPER_MODEL", "WHISPER_THREADS"]
		.into_iter()
		.filter(|name| var(name).is_some())
		.collect();
	if otel_service_name == "transcriber" {
		set.push("OTEL_SERVICE_NAME");
	}
	set
}

/// Compares values, not where they came from: setting a variable to the dev
/// host's value explicitly is still running on the dev host's value.
fn host_specific_defaults_in_use(allowed_origins: &[String], app_base_url: &str, nats_url: &str, redis_url: Option<&str>) -> Vec<&'static str> {
	let mut in_use = Vec::new();
	if allowed_origins.join(",") == DEV_HOST_ALLOWED_ORIGINS {
		in_use.push("ALLOWED_ORIGINS");
	}
	if app_base_url == DEV_HOST_APP_BASE_URL {
		in_use.push("APP_BASE_URL");
	}
	if nats_url == DEV_HOST_NATS_URL {
		in_use.push("NATS_URL");
	}
	// Unset falls back to the dev-host value; set to it is the same value.
	if redis_url.is_none_or(|url| url == DEV_HOST_REDIS_URL) {
		in_use.push("REDIS_URL");
	}
	in_use
}

#[cfg(test)]
mod tests {
	use super::{host_specific_defaults_in_use, retired_variables_in, Config, DEV_HOST_ALLOWED_ORIGINS, DEV_HOST_APP_BASE_URL, DEV_HOST_NATS_URL, DEV_HOST_REDIS_URL};
	use clap::{CommandFactory, Parser};

	#[test]
	fn config_definition_is_valid() {
		Config::command().debug_assert();
	}

	const REFUSED: [(&str, &str); 39] = [
		("RUST_LOG", "info,=[bad"),
		("CONNECTION_TIMEOUT", "soon"),
		("MAX_IN_FLIGHT", "-1"),
		("FILE_HOST_PORT", "70000"),
		("FILE_HOST_PORT_FALLBACK", "1"),
		("RATE_LIMIT", "-1"),
		("CACHE_TTL", "1.5"),
		("PROMETHEUS_PORT", "70000"),
		("BUFFER_SIZE", "64k"),
		("MAX_REQUEST_SIZE_MB", "10MB"),
		("MAX_CONCURRENT_REQUESTS", "many"),
		("TASK_TIMEOUT_MS", "15s"),
		("MAX_CHUNKS_IN_FLIGHT", "-5"),
		("NUDGE_ENABLED", "on"),
		("NUDGE_QUIET_HOURS_START", "10pm"),
		("NUDGE_QUIET_HOURS_END", "8am"),
		("NUDGE_WAKER_SECONDS", "5m"),
		("PUSH_DELIVERY_TIMEOUT_MS", "10s"),
		("WAKER_PASS_DEADLINE_MS", "2m"),
		("RECOMMENDER_USES_OUTCOMES", "on"),
		("NUDGE_PRESENCE_LEASE_TTL_SECONDS", "75s"),
		("AUTH_ENABLED", "off"),
		("AUTH_OPEN_SIGNUP", "on"),
		("AUTH_SESSION_DAYS", "a month"),
		("AUTH_NEW_ACCOUNTS_PER_DAY", "-1"),
		("MAILBOX_ENABLED", "on"),
		("MAILBOX_SECONDS", "15m"),
		("MAILBOX_ARCHIVE_DAYS", "90d"),
		("TRANSCRIBE_ENABLED", "on"),
		("TRANSCRIBE_THREADS", "all"),
		("TRANSCRIBE_DEADLINE_SECONDS", "10m"),
		("TRANSCRIBE_SECONDS", "1m"),
		("GAME_FEED_ENABLED", "on"),
		("GAME_FEED_SCOREBOARD_SECONDS", "10m"),
		("GAME_FEED_LIVE_SECONDS", "15s"),
		("GAME_FEED_BACKOFF_CAP_SECONDS", "-1"),
		("OTEL_TRACES_SAMPLER", "parentbased_always_on"),
		("OTEL_TRACES_SAMPLER_ARG", "half"),
		("OTEL_TRACES_SAMPLER_ARG", "2"),
	];
	// `NUDGE_TIMEZONE` is parsed once the nudge starts, and a name it does
	// not know falls back to UTC with an error and a metric, by design
	// (`nudge::clock::NudgeClock::resolve`). The URLs, keys and paths are
	// checked by what uses them, which refuses to start on a bad one, as the
	// mailbox does `ROUTINE_SCHEDULE`.
	const FREE_TEXT: [&str; 27] = [
		"FILE_HOST_BIND",
		"FILE_HOST_PORT_FILE",
		"ALLOWED_ORIGINS",
		"REDIS_URL",
		"NATS_URL",
		"OBS_WEBSOCKET_IP",
		"OBS_WEBSOCKET_PWD",
		"DATABASE_URL",
		"VAPID_PUBLIC_KEY",
		"VAPID_PRIVATE_KEY",
		"VAPID_SUBJECT",
		"NUDGE_TIMEZONE",
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
		"TRANSCRIBE_MODEL",
		"TRANSCRIBE_VAD_MODEL",
		"OTEL_SERVICE_NAME",
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
		app_config::schema::assert_checked_in("file_host", &Config::command(), super::REQUIRED_WHEN, &path);
		app_config::schema::assert_groups_hold_only_their_own(&Config::command());
	}

	#[test]
	fn every_dev_host_default_in_use_is_named() {
		let dev_origins: Vec<String> = DEV_HOST_ALLOWED_ORIGINS.split(',').map(String::from).collect();
		assert_eq!(
			host_specific_defaults_in_use(&dev_origins, DEV_HOST_APP_BASE_URL, DEV_HOST_NATS_URL, None),
			["ALLOWED_ORIGINS", "APP_BASE_URL", "NATS_URL", "REDIS_URL"]
		);
	}

	/// `example.env` sets `NATS_URL` to the dev host's value explicitly, for
	/// `make dev`; that is still running on it.
	#[test]
	fn a_dev_host_value_set_explicitly_is_still_named() {
		let origins = [String::from("https://app.example.com")];
		assert_eq!(
			host_specific_defaults_in_use(&origins, "https://app.example.com/", DEV_HOST_NATS_URL, Some(DEV_HOST_REDIS_URL)),
			["NATS_URL", "REDIS_URL"]
		);
	}

	#[test]
	fn a_host_that_sets_its_own_values_runs_on_none() {
		let origins = [String::from("https://app.example.com")];
		assert!(host_specific_defaults_in_use(&origins, "https://app.example.com/", "nats://nats:4222", Some("redis://redis:6379")).is_empty());
	}

	/// The `.env` #476 met: every retired variable left set is named.
	#[test]
	fn a_retired_transcriber_variable_left_set_is_named() {
		let stale = |name: &str| matches!(name, "WHISPER_MODELS_PATH" | "WHISPER_MODEL" | "WHISPER_THREADS").then(String::new);
		assert_eq!(
			retired_variables_in(stale, "transcriber"),
			["WHISPER_MODELS_PATH", "WHISPER_MODEL", "WHISPER_THREADS", "OTEL_SERVICE_NAME"]
		);
	}

	/// `OTEL_SERVICE_NAME` is still read, so only the retired value is named.
	#[test]
	fn a_current_env_names_no_retired_variable() {
		assert!(retired_variables_in(|_| None, "file_host").is_empty());
	}

	/// `AUTH_ENABLED` is on by default, so turning it off has to be possible by
	/// value, which a plain `bool` flag (`SetTrue`) would not allow.
	#[test]
	fn auth_enabled_can_be_turned_off_by_value() {
		let required = ["file_host", "--obs-host", "h", "--obs-password", "p", "--database-url", "sqlite::memory:"];
		let off = Config::try_parse_from(required.iter().copied().chain(["--auth-enabled", "false"])).unwrap();
		assert!(!off.auth_enabled);
		let on = Config::try_parse_from(required.iter().copied().chain(["--auth-enabled", "true"])).unwrap();
		assert!(on.auth_enabled);
	}
}
