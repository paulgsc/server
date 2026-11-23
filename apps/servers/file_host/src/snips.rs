//! Home fetches the snips bundle an essay names (SNIPS2, paulgsc/some-ui#3116;
//! epic paulgsc/some-ui#3114).
//!
//! some-ui's `Snips Capture` workflow photographs every screen of the app on
//! sample data into one bundle per run, the artifact `snips-bundle-<commit>`,
//! kept one day on GitHub (some-ui's `docs/snips.md` is the contract). A cards
//! essay shows a snip by its id and the commit its bundle was captured at
//! (`{"kind": "screen", "of": {"from": "snips", "id", "commit", …}}`, C868).
//!
//! - **No picture library** (the owner, 2026-11-21: nothing a CI run can make
//!   again is kept twice on home). Home fetches a commit's bundle only while
//!   an essay not yet given names it, into `<FOLDER>/<commit>/` ([`FOLDER`]), a cache the
//!   hand-off reads (`essay_media::snips`) and copies into `<key>.snips/`. A
//!   commit no waiting essay names leaves the cache at the next pass, and the
//!   copies go with the video. Home keeps no table of snips: which snip and
//!   commit each essay used is its cut list, which `cards_essay` keeps.
//! - **It rides the mailbox pass** (the coordinator for the owner, 2026-11-17;
//!   home-compute invariant 6): no timer of its own. [`run_once`] runs after
//!   the mailbox's own work.
//! - **Bounded.** A pass looks up at most [`COMMITS_LOOKED`] commits (one list
//!   call each), commits never looked at first and those marked gone last,
//!   and downloads at most one bundle, of at most [`MAX_BUNDLE`] bytes,
//!   streamed to the drive and never held in memory whole, each file in it at
//!   most [`MAX_FILE`] and all of them [`MAX_UNPACKED`]; the whole pass ends
//!   at [`PASS_DEADLINE`] (the mailbox's caller holds it to that). A commit
//!   whose fetch failed goes to the back (`<commit>.retry`), so none holds
//!   another back.
//! - **Checked whole, or refused whole.** The manifest must be
//!   `some-ui.snips/1` of the run's own commit; `SHA256SUMS` must name the
//!   manifest and every file its records name, and nothing else, and the
//!   bundle hold exactly those and `SHA256SUMS`; every file must be its sum,
//!   and every record its file (bytes, sum, the id's hash and app version).
//!   One mismatch refuses the bundle.
//! - **Gone is said, not retried** (C875, `docs/snips.md`). A commit with no successful run,
//!   whose newest run's artifact has expired, or whose bundle was refused,
//!   gets `<commit>.gone`, naming that run: the essay is refused
//!   `snip-gone`, and the owner re-runs `Snips Capture` at that commit. Home
//!   never dispatches a run itself: its token is Actions: read only. A newer
//!   run, or a newer attempt of the same run, of that commit clears the mark
//!   and is fetched.
//! - **A failure that may clear** (a timeout, the drive) gets no `.gone`
//!   mark: only its `.retry` one, and it is asked again next pass.
//! - **Off without its token.** `SNIPS_GITHUB_TOKEN`, a fine-grained token
//!   with Actions: read on paulgsc/some-ui only. Unset, nothing is fetched,
//!   and an essay that shows a snip is refused `snip-missing`.
//! - **Counts and kinds, never bytes.** No token, URL or record reaches a log.
//! - **No network in tests.** GitHub is the [`Capture`] port.
//!
//! **Public pages are the family's second member** (paulgsc/some-ui#3395,
//! C1420, C1440): [`Family::Pages`]. A scene names a page by the run of
//! some-ui's dispatch-only `Public Pages Capture` that took it, and that
//! run's `pages-bundle-<run>` is fetched, checked ([`pages`]) and cached in
//! `<PAGES_FOLDER>/<run>/` by every rule above, the run in the commit's
//! place. What differs is said by [`Family`] alone: what names a bundle,
//! how its run is found (a run of another workflow, or one that did not
//! succeed, is no run), the artifact's name, its check and its folder. One
//! queue holds both families' keys, so the bounds and the deadline above are
//! the whole pickup's, never one per family; pages go first among keys of
//! the same standing, since their bundle can't be made again (C1443). A page can't be taken again the
//! same, so a page gone is the owner's to capture anew and name in a new cut
//! list; a newer attempt of the same run still clears its mark.
//!
//! Not yet: putting each record in the mailbox for the routine (SNIPS2's
//! fourth step) waits on the mailbox Worker gaining a folder for it.

pub mod pages;

use regex::Regex;
use serde::Deserialize;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Where home keeps the bundles waiting essays name, as this server sees it:
/// `<commit>/` (its `manifest.json` and pictures) or `<commit>.gone`.
pub const FOLDER: &str = "/app/dbs/renders/snips";

/// Where home keeps the bundles of the public pages waiting essays name:
/// `<run>/` (its `pages.json` and pictures), `<run>.gone` or `<run>.retry`.
pub const PAGES_FOLDER: &str = "/app/dbs/renders/pages";

/// The one workflow whose runs home takes public pages from: a run of any
/// other is no run of the pages (C1440).
pub const PAGES_WORKFLOW: &str = ".github/workflows/public-pages-capture.yml";

/// The capture's repository on GitHub's API.
pub const REPOSITORY: &str = "https://api.github.com/repos/paulgsc/some-ui";

/// The capture's workflow file.
pub const WORKFLOW: &str = "snips-capture.yml";

/// What the manifest must be (`docs/snips.md`); home refuses any other.
pub const MANIFEST_SCHEMA: &str = "some-ui.snips/1";

/// The most commits a pass looks up.
pub const COMMITS_LOOKED: usize = 5;

/// The most bytes home downloads of one bundle. The first green run's 101
/// snips were well under this.
pub const MAX_BUNDLE: u64 = 256 * 1024 * 1024;

/// The most bytes of one file in a bundle.
pub const MAX_FILE: u64 = 16 * 1024 * 1024;

/// The most bytes a bundle's files may come to, unpacked.
pub const MAX_UNPACKED: u64 = 512 * 1024 * 1024;

/// How long one pickup pass may take, all its requests together: so long at
/// most it holds up the mailbox pass it rides.
pub const PASS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// How long a download may take, below [`PASS_DEADLINE`].
pub const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);

/// How long one list call (runs, artifacts) may take.
pub const LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The most files a bundle may hold.
pub const MAX_FILES: usize = 4096;

/// A snip's id, as the capture names it (`@some-ui/video-render`'s
/// `SNIP_ID`, the same pattern): `<surface>:<path>:<viewport>:<appVersion>:<hash12>`.
static SNIP_ID: LazyLock<Regex> = LazyLock::new(|| {
	#[allow(clippy::unwrap_used)]
	Regex::new(r"^(apk|web):(/\S*|@[a-z0-9-]+(?:/[a-z0-9-]+)?):([a-z][a-z0-9-]*):([0-9A-Za-z.+-]+):([0-9a-f]{12})$").unwrap()
});

/// A file a record names (`SnipFileSchema`'s `name`).
static SNIP_FILE: LazyLock<Regex> = LazyLock::new(|| {
	#[allow(clippy::unwrap_used)]
	Regex::new(r"^(apk|web)/[a-z0-9-]+/[A-Za-z0-9._~-]+\.(png|webm|mp4)$").unwrap()
});

/// Whether `id` is a snip's id.
#[must_use]
pub fn is_snip_id(id: &str) -> bool {
	SNIP_ID.is_match(id)
}

/// Whether `run` is a capture run's id as a scene names it (some-ui's
/// `RUN_ID`, `^[1-9][0-9]{0,19}$`) that a run on GitHub can have.
#[must_use]
pub fn is_run(run: &str) -> bool {
	(1..=20).contains(&run.len()) && !run.starts_with('0') && run.bytes().all(|b| b.is_ascii_digit()) && run.parse::<i64>().is_ok()
}

/// Whether `commit` is a commit's 40 lowercase hex digits.
#[must_use]
pub fn is_commit(commit: &str) -> bool {
	commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A snip's id's app version and picture hash, when it is one.
fn id_parts(id: &str) -> Option<(&str, &str)> {
	let parts = SNIP_ID.captures(id)?;
	Some((parts.get(4)?.as_str(), parts.get(5)?.as_str()))
}

/// `<key>/`, a fetched bundle in `folder`: a commit's, or a run's.
#[must_use]
pub fn bundle_of(folder: &Path, key: &str) -> PathBuf {
	folder.join(key)
}

/// `<key>.gone`, the mark of a bundle home cannot fetch.
#[must_use]
pub fn gone_of(folder: &Path, key: &str) -> PathBuf {
	folder.join(String::from(key) + ".gone")
}

/// One kind of bundle home fetches for waiting essays: a closed set, as
/// [`crate::essay_media::Kind`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Family {
	/// `snips-bundle-<commit>`, of `Snips Capture`, named by its commit.
	Snips,
	/// `pages-bundle-<run>`, of `Public Pages Capture`, named by its run.
	Pages,
}

impl Family {
	/// Every family.
	pub const ALL: [Self; 2] = [Self::Snips, Self::Pages];

	/// Its place in a pass's queue among keys of the same standing: pages
	/// first. A pages bundle lives one day and can't be made again, while a
	/// snips bundle is a re-run away; and at one bundle a pass, hourly, a
	/// day's worth of snips commits ahead of a run would outlive it (C1443).
	const fn first(self) -> u8 {
		match self {
			Self::Pages => 0,
			Self::Snips => 1,
		}
	}

	/// The essay media kind whose items name its bundles.
	#[must_use]
	pub const fn kind(self) -> crate::essay_media::Kind {
		match self {
			Self::Snips => crate::essay_media::Kind::Snips,
			Self::Pages => crate::essay_media::Kind::Pages,
		}
	}

	/// Whether `key` names one of its bundles: a commit, or a run.
	#[must_use]
	pub fn is_key(self, key: &str) -> bool {
		match self {
			Self::Snips => is_commit(key),
			Self::Pages => is_run(key),
		}
	}

	/// Whether `run` is the run that made `key`'s bundle.
	fn owns(self, run: &Run, key: &str) -> bool {
		match self {
			Self::Snips => run.head_sha == key,
			Self::Pages => run.id.to_string() == key,
		}
	}

	/// The name of `run`'s artifact.
	#[must_use]
	pub fn artifact(self, run: &Run) -> String {
		match self {
			Self::Snips => String::from("snips-bundle-") + &run.head_sha,
			Self::Pages => String::from("pages-bundle-") + &run.id.to_string(),
		}
	}

	/// Checks the bundle at `zip` against `run`, as the family's contract
	/// says, and stages what is handed in `stage`.
	fn open(self, zip: &Path, stage: &Path, run: &Run) -> Result<(), Why> {
		match self {
			Self::Snips => open(zip, stage, run),
			Self::Pages => pages::open(zip, stage),
		}
	}
}

/// Where each family's bundles are cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
	pub snips: PathBuf,
	pub pages: PathBuf,
}

impl Folders {
	/// The server's: [`FOLDER`] and [`PAGES_FOLDER`].
	#[must_use]
	pub fn server() -> Self {
		Self {
			snips: PathBuf::from(FOLDER),
			pages: PathBuf::from(PAGES_FOLDER),
		}
	}

	/// `family`'s.
	#[must_use]
	pub fn of(&self, family: Family) -> &Path {
		match family {
			Family::Snips => &self.snips,
			Family::Pages => &self.pages,
		}
	}
}

/// One successful run of the capture.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Run {
	pub id: i64,
	/// The commit it captured (a page's run's is not its key).
	pub head_sha: String,
	/// Which attempt: a re-run keeps its id and makes a new attempt.
	pub run_attempt: i64,
}

impl Run {
	/// How a `.gone` mark names it.
	fn mark(&self) -> String {
		self.id.to_string() + "." + &self.run_attempt.to_string()
	}
}

/// What became of a bundle asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bundle {
	/// Saved whole to the path asked.
	Saved,
	/// Its artifact is gone: expired, or the run made none.
	Gone,
	/// Past [`MAX_BUNDLE`].
	TooBig,
}

/// GitHub, as the pickup sees it. A failure is a kind, never a URL or body.
pub trait Capture: Send + Sync {
	/// `family`'s capture's successful runs that made `key`'s bundle, in
	/// any order: none when there is no such run, or it is of another
	/// workflow, or did not succeed.
	fn runs(&self, family: Family, key: &str) -> impl Future<Output = Result<Vec<Run>, &'static str>> + Send;

	/// `run`'s bundle of `family`, saved to `into`, at most [`MAX_BUNDLE`]
	/// bytes.
	fn bundle(&self, family: Family, run: &Run, into: &Path) -> impl Future<Output = Result<Bundle, &'static str>> + Send;
}

/// [`Capture`] over GitHub's REST API with `SNIPS_GITHUB_TOKEN`.
#[derive(Clone)]
pub struct GitHub {
	client: outbound::Client,
	token: String,
}

impl std::fmt::Debug for GitHub {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("GitHub").finish_non_exhaustive()
	}
}

/// A request's failure, by kind.
fn kind_of(err: &reqwest::Error) -> &'static str {
	if err.is_timeout() {
		"timeout"
	} else if err.is_connect() {
		"connect"
	} else {
		"transport"
	}
}

impl GitHub {
	/// The pickup with `token`, or `None` when it is unset or empty.
	///
	/// # Errors
	/// A client that cannot be built.
	pub fn new(token: Option<&str>) -> anyhow::Result<Option<Self>> {
		let Some(token) = token.map(str::trim).filter(|token| !token.is_empty()) else {
			return Ok(None);
		};
		// A download is bounded in time as in bytes, so the mailbox pass it
		// rides waits at most that ([`PASS_DEADLINE`] bounds the whole pass).
		// Under the pass's deadline, so a stalled download usually fails on
		// its own clock and is said, rather than being cut off unrecorded.
		let client = outbound::client(outbound::Caller::Snips).timeout(DOWNLOAD_TIMEOUT).build()?;
		Ok(Some(Self { client, token: token.to_owned() }))
	}

	fn get(&self, url: &str) -> outbound::Request {
		// GitHub's API refuses a request with no user agent. A generic name:
		// nothing of the owner goes to GitHub.
		self
			.client
			.get(url)
			.bearer_auth(&self.token)
			.header(reqwest::header::USER_AGENT, "home-file-host")
			.header(reqwest::header::ACCEPT, "application/vnd.github+json")
			.header("X-GitHub-Api-Version", "2022-11-28")
	}

	async fn json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, &'static str> {
		self.json_or_none(url).await?.ok_or("refused")
	}

	/// As [`Self::json`], with `None` for a 404: what GitHub answers for a
	/// run it has no such id of.
	async fn json_or_none<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<Option<T>, &'static str> {
		// A list is small: a stalled one fails here, well inside the pass's
		// deadline, and is said on OUTBOUND as any request's failure.
		let answer = self.get(url).timeout(LIST_TIMEOUT).send().await.map_err(|err| kind_of(&err))?;
		if answer.status() == reqwest::StatusCode::NOT_FOUND {
			return Ok(None);
		}
		if !answer.status().is_success() {
			return Err("refused");
		}
		let body = answer.bytes().await.map_err(|err| kind_of(&err))?;
		serde_json::from_slice(&body).map(Some).map_err(|_| "unreadable")
	}
}

#[derive(Deserialize)]
struct Runs {
	workflow_runs: Vec<Run>,
}

/// One run, as `GET /actions/runs/<run>` answers: its latest attempt.
#[derive(Deserialize)]
struct OneRun {
	#[serde(flatten)]
	run: Run,
	path: String,
	conclusion: Option<String>,
}

impl OneRun {
	/// The run, when it is a successful run of [`PAGES_WORKFLOW`]: a run of
	/// another workflow, or one that did not succeed (or has not yet), is no
	/// run of the pages.
	fn of_pages(self) -> Option<Run> {
		(self.path == PAGES_WORKFLOW && self.conclusion.as_deref() == Some("success")).then_some(self.run)
	}
}

#[derive(Deserialize)]
struct Artifacts {
	artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
	id: i64,
	name: String,
	expired: bool,
	size_in_bytes: u64,
	archive_download_url: String,
}

impl Capture for GitHub {
	async fn runs(&self, family: Family, key: &str) -> Result<Vec<Run>, &'static str> {
		// Only a key of its family reaches a URL: digits, or hex.
		if !family.is_key(key) {
			return Ok(Vec::new());
		}
		match family {
			Family::Snips => {
				let url = String::from(REPOSITORY) + "/actions/workflows/" + WORKFLOW + "/runs?status=success&per_page=20&head_sha=" + key;
				Ok(self.json::<Runs>(&url).await?.workflow_runs)
			}
			Family::Pages => {
				let url = String::from(REPOSITORY) + "/actions/runs/" + key;
				Ok(self.json_or_none::<OneRun>(&url).await?.into_iter().filter_map(OneRun::of_pages).collect())
			}
		}
	}

	async fn bundle(&self, family: Family, run: &Run, into: &Path) -> Result<Bundle, &'static str> {
		let wanted = family.artifact(run);
		let url = String::from(REPOSITORY) + "/actions/runs/" + &run.id.to_string() + "/artifacts?per_page=100&name=" + &wanted;
		// An earlier attempt's copy may be listed beside the newest: the
		// newest that has not expired is the run's.
		let artifacts = self.json::<Artifacts>(&url).await?.artifacts;
		let Some(artifact) = artifacts
			.into_iter()
			.filter(|artifact| artifact.name == wanted && !artifact.expired)
			.max_by_key(|artifact| artifact.id)
		else {
			return Ok(Bundle::Gone);
		};
		if artifact.size_in_bytes > MAX_BUNDLE {
			return Ok(Bundle::TooBig);
		}
		// GitHub answers with a redirect to storage, which reqwest follows
		// without the token.
		let mut answer = self.get(&artifact.archive_download_url).send().await.map_err(|err| kind_of(&err))?;
		match answer.status().as_u16() {
			200 => {}
			404 | 410 => return Ok(Bundle::Gone),
			_ => return Err("refused"),
		}
		let mut file = tokio::fs::File::create(into).await.map_err(|_| "drive")?;
		let mut written: u64 = 0;
		while let Some(chunk) = answer.chunk().await.map_err(|err| kind_of(&err))? {
			written += chunk.len() as u64;
			if written > MAX_BUNDLE {
				return Ok(Bundle::TooBig);
			}
			tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await.map_err(|_| "drive")?;
		}
		tokio::io::AsyncWriteExt::flush(&mut file).await.map_err(|_| "drive")?;
		Ok(Bundle::Saved)
	}
}

/// What a pass of the pickup did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Picked {
	/// Bundles fetched, checked and put in the cache.
	pub fetched: usize,
	/// Commits marked gone: no run, an expired artifact, or a refused bundle.
	pub gone: usize,
	/// Cached bundles and marks no waiting essay names, removed.
	pub dropped: usize,
	/// Each failure, by kind (`timeout`, `connect`, `transport`, `refused`,
	/// `unreadable`, `drive`): asked again next pass.
	pub failures: Vec<&'static str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
	schema: String,
	app_version: String,
	commit: String,
	#[serde(borrow)]
	snips: Vec<&'a RawValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
	id: String,
	app_version: String,
	image: FileOf,
	recording: Option<FileOf>,
}

#[derive(Deserialize)]
struct FileOf {
	name: String,
	kind: String,
	bytes: u64,
	sha256: String,
}

/// Each line of `SHA256SUMS` (`<sum>  <name>`), by name; `None` when one is
/// not such a line, or a name is listed twice.
fn sums(text: &str) -> Option<BTreeMap<String, String>> {
	let mut sums = BTreeMap::new();
	for line in text.lines() {
		let (sum, name) = line.split_once("  ")?;
		let plain = sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
		if !plain || sums.insert(name.to_owned(), sum.to_owned()).is_some() {
			return None;
		}
	}
	Some(sums)
}

/// `name`'s bytes in `archive`, at most [`MAX_FILE`].
fn read(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>, Why> {
	let entry = archive.by_name(name).map_err(zip_error)?;
	let mut bytes = Vec::new();
	// A corrupt entry reads as an I/O error too, and the zip itself is on
	// the drive: either is asked again, never said gone.
	entry.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|_| Why::Drive)?;
	if bytes.len() as u64 > MAX_FILE {
		return Err(Why::Refused);
	}
	Ok(bytes)
}

/// Why a bundle was not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
	/// It does not check out: never to be opened.
	Refused,
	/// The drive failed: asked again.
	Drive,
}

/// A zip's failure: the drive's when it is I/O, else the bundle's.
#[allow(clippy::needless_pass_by_value)]
fn zip_error(err: zip::result::ZipError) -> Why {
	if matches!(err, zip::result::ZipError::Io(_)) {
		Why::Drive
	} else {
		Why::Refused
	}
}

/// Writes `bytes` to `name` under `stage`.
fn stage_file(stage: &Path, name: &str, bytes: &[u8]) -> Result<(), Why> {
	let to = stage.join(name);
	if let Some(parent) = to.parent() {
		std::fs::create_dir_all(parent).map_err(|_| Why::Drive)?;
	}
	std::fs::write(to, bytes).map_err(|_| Why::Drive)
}

/// Checks the bundle at `zip` against `run`, as the module doc says, and
/// stages its manifest and pictures in `stage` under their names.
fn open(zip: &Path, stage: &Path, run: &Run) -> Result<(), Why> {
	let file = std::fs::File::open(zip).map_err(|_| Why::Drive)?;
	let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
	if archive.len() > MAX_FILES {
		return Err(Why::Refused);
	}
	let sums = sums(std::str::from_utf8(&read(&mut archive, "SHA256SUMS")?).map_err(|_| Why::Refused)?).ok_or(Why::Refused)?;
	let manifest_bytes = read(&mut archive, "manifest.json")?;
	if sums.get("manifest.json") != Some(&hex::encode(Sha256::digest(&manifest_bytes))) {
		return Err(Why::Refused);
	}
	let manifest: Manifest<'_> = serde_json::from_slice(&manifest_bytes).map_err(|_| Why::Refused)?;
	if manifest.schema != MANIFEST_SCHEMA || manifest.commit != run.head_sha {
		return Err(Why::Refused);
	}
	// Every file the records name, and the manifest: exactly what the sums list.
	let mut named = BTreeSet::from([String::from("manifest.json")]);
	let mut ids = BTreeSet::new();
	let mut records = Vec::with_capacity(manifest.snips.len());
	for raw in &manifest.snips {
		let record: Record = serde_json::from_str(raw.get()).map_err(|_| Why::Refused)?;
		let (version, hash) = id_parts(&record.id).ok_or(Why::Refused)?;
		let files = std::iter::once(&record.image).chain(record.recording.as_ref());
		let plain = files.clone().all(|file| {
			SNIP_FILE.is_match(&file.name)
				&& file.name.rsplit_once('.').is_some_and(|(_, extension)| extension == file.kind)
				&& sums.get(&file.name) == Some(&file.sha256)
				&& (1..=MAX_FILE).contains(&file.bytes)
		});
		let ok = plain
			&& version == manifest.app_version
			&& record.app_version == manifest.app_version
			&& record.image.name.rsplit_once('.').is_some_and(|(_, extension)| extension == "png")
			&& record.image.sha256.starts_with(hash)
			&& ids.insert(record.id.clone());
		if !ok {
			return Err(Why::Refused);
		}
		for file in files {
			if !named.insert(file.name.clone()) {
				return Err(Why::Refused);
			}
		}
		records.push(record);
	}
	if named.iter().ne(sums.keys()) {
		return Err(Why::Refused);
	}
	let unpacked: u64 = records
		.iter()
		.flat_map(|record| std::iter::once(&record.image).chain(record.recording.as_ref()))
		.map(|file| file.bytes)
		.sum();
	if unpacked > MAX_UNPACKED {
		return Err(Why::Refused);
	}
	let listed: Result<Vec<String>, _> = archive.file_names().map(|entry| entry.map(std::borrow::Cow::into_owned)).collect();
	let held: BTreeSet<String> = listed.map_err(|_| Why::Refused)?.into_iter().filter(|name| !name.ends_with('/')).collect();
	if held.len() != named.len() + 1 || !held.contains("SHA256SUMS") || !named.iter().all(|name| held.contains(name)) {
		return Err(Why::Refused);
	}
	for record in &records {
		// Only the pictures are handed; a recording is checked, not kept.
		for (file, kept) in std::iter::once((&record.image, true)).chain(record.recording.as_ref().map(|file| (file, false))) {
			let bytes = read(&mut archive, &file.name)?;
			if bytes.len() as u64 != file.bytes || hex::encode(Sha256::digest(&bytes)) != file.sha256 {
				return Err(Why::Refused);
			}
			if kept {
				stage_file(stage, &file.name, &bytes)?;
			}
		}
	}
	stage_file(stage, "manifest.json", &manifest_bytes)
}

/// Every bundle, by family and key, an item names in an essay home holds
/// that the essays' folder has not been given yet: one read of each body.
async fn wanted(db: &SqlitePool, essays: &Path) -> Result<BTreeSet<(Family, String)>, sqlx::Error> {
	let mut conn = db.acquire().await?;
	let mut keys = BTreeSet::new();
	for essay in cards_essay_repo::held(&mut conn).await? {
		if essays.join(String::from(&essay.key) + ".json").exists() {
			continue;
		}
		let Some(body) = cards_essay_repo::body(&mut conn, &essay.key).await? else {
			continue;
		};
		let essay = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
		for family in Family::ALL {
			for item in crate::essay_media::named(family.kind(), &essay) {
				// An item is `<key> …`: its bundle's key first.
				if let Some(key) = item.split(' ').next().filter(|key| family.is_key(key)) {
					keys.insert((family, key.to_owned()));
				}
			}
		}
	}
	Ok(keys)
}

/// The run a `.gone` mark names, or `None` without one.
async fn marked(folder: &Path, key: &str) -> Option<String> {
	let text = tokio::fs::read_to_string(gone_of(folder, key)).await.ok()?;
	Some(text.lines().find_map(|line| line.strip_prefix("run=")).unwrap_or_default().to_owned())
}

/// `<key>.retry`, touched when a key's fetch failed in a way that may
/// clear: when, by its time, never why.
#[must_use]
pub fn retry_of(folder: &Path, key: &str) -> PathBuf {
	folder.join(String::from(key) + ".retry")
}

/// Touches `key`'s retry mark.
async fn mark_retry(folder: &Path, key: &str) -> std::io::Result<()> {
	tokio::fs::write(retry_of(folder, key), b"").await
}

/// Marks `key` gone, naming `run` and why.
async fn mark_gone(folder: &Path, key: &str, run: Option<&Run>, reason: &str) -> std::io::Result<()> {
	let partial = folder.join(String::from(".") + key + ".gone.partial");
	let text = String::from("reason=") + reason + "\nrun=" + &run.map_or_else(String::new, Run::mark) + "\n";
	tokio::fs::write(&partial, text).await?;
	tokio::fs::rename(&partial, gone_of(folder, key)).await
}

/// Removes the cache's entries no waiting essay names, and what a stopped
/// pass left: the pickup is the cache's one writer, and runs in one task.
async fn drop_unwanted(folder: &Path, family: Family, wanted: &BTreeSet<(Family, String)>, picked: &mut Picked) {
	let Ok(mut entries) = tokio::fs::read_dir(folder).await else {
		return;
	};
	while let Ok(Some(entry)) = entries.next_entry().await {
		let name = entry.file_name().to_string_lossy().into_owned();
		let key = name.strip_suffix(".gone").or_else(|| name.strip_suffix(".retry")).unwrap_or(&name);
		if !name.starts_with('.') && wanted.contains(&(family, key.to_owned())) {
			continue;
		}
		let path = entry.path();
		let removed = if path.is_dir() {
			tokio::fs::remove_dir_all(&path).await
		} else {
			tokio::fs::remove_file(&path).await
		};
		if removed.is_ok() && !name.starts_with('.') {
			picked.dropped += 1;
		}
	}
}

/// One pass of the pickup, as the module doc says: the bundles of every
/// family the essays home holds and has not given to `essays` name, each
/// cached in its family's folder of `folders`.
///
/// # Errors
/// A storage failure.
pub async fn run_once<C: Capture>(db: &SqlitePool, capture: &C, essays: &Path, folders: &Folders) -> Result<Picked, sqlx::Error> {
	let mut picked = Picked::default();
	let wanted = wanted(db, essays).await?;
	for family in Family::ALL {
		drop_unwanted(folders.of(family), family, &wanted, &mut picked).await;
	}
	let mut missing: Vec<(u8, u8, std::time::SystemTime, Family, &String)> = Vec::new();
	for (family, key) in &wanted {
		let folder = folders.of(*family);
		if bundle_of(folder, key).is_dir() {
			continue;
		}
		// Never looked at first, then those whose fetch failed, longest ago
		// first, then those marked gone (only a newer run clears one): so no
		// key, of either family, holds another back for good.
		let retried = tokio::fs::metadata(retry_of(folder, key)).await.and_then(|meta| meta.modified()).ok();
		let rank = if gone_of(folder, key).exists() { 2 } else { u8::from(retried.is_some()) };
		missing.push((rank, family.first(), retried.unwrap_or(std::time::UNIX_EPOCH), *family, key));
	}
	missing.sort();
	for (_, _, _, family, key) in missing.into_iter().take(COMMITS_LOOKED) {
		let folder = folders.of(family);
		if tokio::fs::create_dir_all(folder).await.is_err() {
			picked.failures.push("drive");
			continue;
		}
		// Marked before it is tried, and cleared once it comes to an outcome:
		// a pass the deadline cuts short mid-request leaves the key at the
		// back of the queue, never at its head.
		let _ = mark_retry(folder, key).await;
		let runs = match capture.runs(family, key).await {
			Ok(runs) => runs,
			Err(kind) => {
				picked.failures.push(kind);
				continue;
			}
		};
		let newest = runs.into_iter().filter(|run| family.owns(run, key)).max_by_key(|run| (run.id, run.run_attempt));
		let mark = marked(folder, key).await;
		let Some(run) = newest else {
			if mark.is_none() {
				picked.gone += 1;
				if mark_gone(folder, key, None, "no-run").await.is_err() {
					picked.failures.push("drive");
					continue;
				}
			}
			// Only once it is marked gone: a mark the drive refused leaves it
			// at the back, never at the head.
			let _ = tokio::fs::remove_file(retry_of(folder, key)).await;
			continue;
		};
		if mark.as_deref() == Some(run.mark().as_str()) {
			// Gone as this run, and no newer one: said, not asked again.
			let _ = tokio::fs::remove_file(retry_of(folder, key)).await;
			continue;
		}
		let zip = folder.join(String::from(".") + key + ".zip.partial");
		let stage = folder.join(String::from(".") + key + ".partial");
		let _ = tokio::fs::remove_dir_all(&stage).await;
		let outcome = match capture.bundle(family, &run, &zip).await {
			Err(kind) => Err(kind),
			Ok(Bundle::Gone) => Ok(Some("gone")),
			Ok(Bundle::TooBig) => Ok(Some("refused")),
			Ok(Bundle::Saved) => {
				let (zip_at, stage_at, asked) = (zip.clone(), stage.clone(), run.clone());
				match tokio::task::spawn_blocking(move || family.open(&zip_at, &stage_at, &asked)).await {
					Ok(Ok(())) => match tokio::fs::rename(&stage, bundle_of(folder, key)).await {
						Ok(()) => Ok(None),
						Err(_) => Err("drive"),
					},
					Ok(Err(Why::Refused)) => Ok(Some("refused")),
					// A panic is as a drive's failure: nothing is said of it.
					Ok(Err(Why::Drive)) | Err(_) => Err("drive"),
				}
			}
		};
		let _ = tokio::fs::remove_file(&zip).await;
		let _ = tokio::fs::remove_dir_all(&stage).await;
		match outcome {
			Ok(None) => {
				let _ = tokio::fs::remove_file(gone_of(folder, key)).await;
				let _ = tokio::fs::remove_file(retry_of(folder, key)).await;
				picked.fetched += 1;
			}
			Ok(Some(reason)) => {
				picked.gone += 1;
				if mark_gone(folder, key, Some(&run), reason).await.is_err() {
					picked.failures.push("drive");
				} else {
					let _ = tokio::fs::remove_file(retry_of(folder, key)).await;
				}
			}
			// Its retry mark stays: to the back of the queue, so it holds no
			// other key back.
			Err(kind) => picked.failures.push(kind),
		}
		// One bundle a pass, of either family.
		break;
	}
	Ok(picked)
}

#[cfg(test)]
mod tests;
