use super::*;
use crate::essay_media;
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

/// A change to a bundle's files before it is zipped.
type Edit = Box<dyn FnOnce(&mut BTreeMap<String, Vec<u8>>)>;

/// A run's bundle: its zip's bytes, `None` for one gone, or a failure.
type Answer = Result<Option<Vec<u8>>, &'static str>;

/// Answers runs and bundles from tables, and records each bundle asked for.
#[derive(Default)]
struct Fake {
	runs: Mutex<HashMap<(Family, String), Result<Vec<Run>, &'static str>>>,
	/// A run's bundle: its zip's bytes, `None` for one gone, or a failure.
	bundles: Mutex<HashMap<i64, Answer>>,
	asked: Mutex<Vec<i64>>,
}

impl Fake {
	/// A snips run of its commit.
	fn run(&self, run: Run, bundle: Answer) {
		let key = (Family::Snips, run.head_sha.clone());
		self.held(key, run, bundle);
	}

	/// A run of the pages capture, by its id.
	fn page_run(&self, run: Run, bundle: Answer) {
		let key = (Family::Pages, run.id.to_string());
		self.held(key, run, bundle);
	}

	fn held(&self, key: (Family, String), run: Run, bundle: Answer) {
		self.bundles.lock().unwrap().insert(run.id, bundle);
		if let Ok(held) = self.runs.lock().unwrap().entry(key).or_insert_with(|| Ok(Vec::new())) {
			held.push(run);
		}
	}
}

impl Capture for Fake {
	async fn runs(&self, family: Family, key: &str) -> Result<Vec<Run>, &'static str> {
		self.runs.lock().unwrap().get(&(family, String::from(key))).cloned().unwrap_or(Ok(Vec::new()))
	}

	async fn bundle(&self, _family: Family, run: &Run, into: &Path) -> Result<Bundle, &'static str> {
		self.asked.lock().unwrap().push(run.id);
		let answer = self.bundles.lock().unwrap().get(&run.id).cloned().unwrap_or(Ok(None));
		match answer? {
			None => Ok(Bundle::Gone),
			Some(bytes) => {
				std::fs::write(into, bytes).map_err(|_| "drive")?;
				Ok(Bundle::Saved)
			}
		}
	}
}

async fn pool() -> SqlitePool {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	pool
}

fn commit(n: u8) -> String {
	char::from(b'a' + n).to_string().repeat(40)
}

fn run(id: i64, n: u8) -> Run {
	Run {
		id,
		head_sha: commit(n),
		run_attempt: 1,
	}
}

fn sha(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// A picture's bundle name, id and record, its bytes `png`.
fn record(png: &[u8], route: &str) -> (String, String, serde_json::Value) {
	let hash = sha(png);
	let name = String::from("web/phone/") + route.trim_start_matches('/') + "-" + &hash[..12] + ".png";
	let id = String::from("web:") + route + ":phone:1.2.3:" + &hash[..12];
	let record = serde_json::json!({
		"id": id, "surface": "web", "build": "release", "route": route, "path": route, "viewport": "phone",
		"width": 824, "height": 1830, "scale": 2, "regions": [], "appVersion": "1.2.3", "module": "settings",
		"shows": "Settings", "description": "Settings, on the web app, with sample data.",
		"image": { "name": name, "kind": "png", "bytes": png.len(), "sha256": hash },
	});
	(name, id, record)
}

/// A bundle of `pictures` for `commit`, as the capture writes it, then `edit`
/// applied to its files by name before it is zipped.
fn bundle(commit: &str, pictures: &[(&[u8], &str)], edit: impl FnOnce(&mut BTreeMap<String, Vec<u8>>)) -> Vec<u8> {
	let mut files = BTreeMap::new();
	let mut records = Vec::new();
	for (png, route) in pictures {
		let (name, _, record) = record(png, route);
		files.insert(name, png.to_vec());
		records.push(record);
	}
	let manifest = serde_json::json!({
		"schema": MANIFEST_SCHEMA, "appVersion": "1.2.3", "commit": commit, "run": "https://example.invalid/run",
		"frozenAt": "2026-11-02T09:30:00.000Z", "data": "sample", "snips": records, "unreached": [],
	});
	files.insert(String::from("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap());
	let sums: String = files.iter().map(|(name, bytes)| sha(bytes) + "  " + name + "\n").collect();
	files.insert(String::from("SHA256SUMS"), sums.into_bytes());
	edit(&mut files);
	let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
	let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
	for (name, bytes) in files {
		zip.start_file(name, options).unwrap();
		zip.write_all(&bytes).unwrap();
	}
	zip.finish().unwrap().into_inner()
}

/// Keeps a cards essay `key` whose scenes show `snips`, each `(commit, id)`.
async fn hold_essay(pool: &SqlitePool, key: &str, snips: &[(String, String)]) {
	let shots: Vec<serde_json::Value> = snips
		.iter()
		.map(|(commit, id)| serde_json::json!({ "picture": { "kind": "screen", "of": { "from": "snips", "id": id, "commit": commit, "at": [] } } }))
		.collect();
	let day = &key[key.len() - 10..];
	let body = serde_json::json!({ "schema": "some-ui.cut-list/2", "stamp": { "date": day }, "shots": shots }).to_string();
	let mut conn = pool.acquire().await.unwrap();
	let essay = cards_essay_repo::Essay {
		key,
		day,
		body: body.as_bytes(),
		taken_at: "2026-11-21T08:00:00.000Z",
	};
	cards_essay_repo::keep(&mut conn, &essay).await.unwrap();
}

struct Dirs {
	_root: tempfile::TempDir,
	essays: PathBuf,
	cache: PathBuf,
	pages: PathBuf,
}

impl Dirs {
	/// Both families' caches.
	fn folders(&self) -> Folders {
		Folders {
			snips: self.cache.clone(),
			pages: self.pages.clone(),
		}
	}

	/// The cache, made.
	fn cache_ready(&self) -> PathBuf {
		std::fs::create_dir_all(&self.cache).unwrap();
		self.cache.clone()
	}
}

fn dirs() -> Dirs {
	let root = tempfile::tempdir().unwrap();
	let (essays, cache, pages) = (root.path().join("essays"), root.path().join("snips"), root.path().join("pages"));
	std::fs::create_dir_all(&essays).unwrap();
	Dirs {
		_root: root,
		essays,
		cache,
		pages,
	}
}

/// What the hand-off makes of `snips` with the cache in `dirs`.
async fn handed(pool: &SqlitePool, dirs: &Dirs, snips: &[(String, String)]) -> essay_media::Handed {
	let sources = essay_media::Sources {
		subject: None,
		frames: dirs.cache.join("no-frames"),
		snips: Some(dirs.cache.clone()),
		pages: Some(dirs.pages.clone()),
	};
	let items: Vec<String> = snips.iter().map(|(commit, id)| commit.clone() + " " + id).collect();
	let mut conn = pool.acquire().await.unwrap();
	essay_media::hand(&mut conn, &sources, essay_media::Kind::Snips, &items).await.unwrap()
}

#[tokio::test]
async fn a_waiting_essays_bundle_is_fetched_checked_and_handed_then_dropped_once_given() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (name, id, record) = record(b"settings-png", "/settings");
	let wanted = [(commit(0), id.clone())];
	hold_essay(&pool, "shown-2026-11-20", &wanted).await;
	assert_eq!(
		handed(&pool, &dirs, &wanted).await,
		essay_media::Handed::Waiting(vec![commit(0) + " " + &id]),
		"not fetched yet: it waits"
	);

	fake.run(
		run(10, 0),
		Ok(Some(bundle(&commit(0), &[(b"settings-png", "/settings"), (b"studio-png", "/studio")], |_| {}))),
	);
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
	assert_eq!((picked.fetched, picked.gone), (1, 0), "{picked:?}");
	let names: Vec<_> = std::fs::read_dir(&dirs.cache).unwrap().map(|entry| entry.unwrap().file_name()).collect();
	assert_eq!(names, [commit(0).as_str()], "no partial is left");

	let essay_media::Handed::Ready(files) = handed(&pool, &dirs, &wanted).await else {
		panic!("handed once fetched");
	};
	let given: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
	assert_eq!(files[0].0, "snips.json");
	assert_eq!(
		given,
		serde_json::json!({ "schema": "some-ui.snips-given/1", "bundles": [{ "commit": commit(0), "snips": [record] }] })
	);
	assert_eq!(files[1], (name, b"settings-png".to_vec()), "the picture under its bundle name");
	assert_eq!(files.len(), 2, "only what the essay names");

	let other = [(commit(0), String::from("web:/gone:phone:1.2.3:0123456789ab"))];
	let essay_media::Handed::Refused(refusal) = handed(&pool, &dirs, &other).await else {
		panic!("an id its bundle doesn't hold");
	};
	assert_eq!(refusal.reason(), "snip-missing");

	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap(), Picked::default(), "fetched once");
	std::fs::write(dirs.essays.join("shown-2026-11-20.json"), b"{}").unwrap();
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
	assert_eq!(picked.dropped, 1, "once the essay is given, no copy is kept");
	assert!(!bundle_of(&dirs.cache, &commit(0)).exists());
}

#[tokio::test]
async fn a_bundle_that_does_not_check_out_is_refused_whole_and_said_gone() {
	let pictures: [(&[u8], &str); 2] = [(b"settings-png", "/settings"), (b"studio-png", "/studio")];
	let (settings, id, _) = record(b"settings-png", "/settings");
	let cases: Vec<(&str, String, Edit)> = vec![
		("a picture not its sum", commit(1), Box::new(move |files| drop(files.insert(settings, b"other".to_vec())))),
		(
			"a file the sums don't list",
			commit(1),
			Box::new(|files| drop(files.insert(String::from("web/phone/extra.png"), b"x".to_vec()))),
		),
		("a picture missing", commit(1), Box::new(|files| files.retain(|name, _| !name.contains("studio")))),
		("another commit's", commit(2), Box::new(|_| {})),
		(
			"another schema",
			commit(1),
			Box::new(|files| {
				let manifest = String::from_utf8(files["manifest.json"].clone()).unwrap().replace(MANIFEST_SCHEMA, "some-ui.snips/2");
				let sums = String::from_utf8(files["SHA256SUMS"].clone())
					.unwrap()
					.replace(&sha(&files["manifest.json"]), &sha(manifest.as_bytes()));
				files.insert(String::from("manifest.json"), manifest.into_bytes());
				files.insert(String::from("SHA256SUMS"), sums.into_bytes());
			}),
		),
		(
			"an escaping name",
			commit(1),
			Box::new(|files| drop(files.insert(String::from("../escape.png"), b"x".to_vec()))),
		),
	];
	for (case, made_for, edit) in cases {
		let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
		let wanted = [(commit(1), id.clone())];
		hold_essay(&pool, "shown-2026-11-20", &wanted).await;
		fake.run(run(20, 1), Ok(Some(bundle(&made_for, &pictures, edit))));
		let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
		assert_eq!((picked.fetched, picked.gone), (0, 1), "{case}: {picked:?}");
		assert!(!bundle_of(&dirs.cache, &commit(1)).exists(), "{case}: nothing of it is kept");
		assert!(!dirs.cache.parent().unwrap().join("escape.png").exists());
		assert!(std::fs::read_to_string(gone_of(&dirs.cache, &commit(1))).unwrap().starts_with("reason=refused\n"), "{case}");
		let essay_media::Handed::Refused(refusal) = handed(&pool, &dirs, &wanted).await else {
			panic!("{case}: refused");
		};
		assert_eq!(refusal.reason(), "snip-gone", "{case}");
		assert_eq!(
			run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap(),
			Picked::default(),
			"{case}: not downloaded again"
		);
	}
}

#[tokio::test]
async fn a_commit_gone_is_said_until_a_newer_run_of_it_and_never_dispatched() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (_, id, _) = record(b"a", "/a");
	hold_essay(&pool, "shown-2026-11-20", &[(commit(0), id.clone())]).await;
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
	assert_eq!(picked.gone, 1, "no run of its commit");
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.cache, &commit(0))).unwrap(), "reason=no-run\nrun=\n");
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap(), Picked::default(), "said once");

	// A run whose artifact has expired.
	fake.run(run(7, 0), Ok(None));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().gone, 1);
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.cache, &commit(0))).unwrap(), "reason=gone\nrun=7.1\n");
	assert_eq!(
		run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap(),
		Picked::default(),
		"not asked again for that run"
	);
	assert_eq!(*fake.asked.lock().unwrap(), [7]);

	// The owner runs it again: GitHub's re-run keeps the id, a new attempt.
	let again = Run { run_attempt: 2, ..run(7, 0) };
	fake.runs.lock().unwrap().insert((Family::Snips, commit(0)), Ok(vec![run(7, 0), again]));
	fake.bundles.lock().unwrap().insert(7, Ok(Some(bundle(&commit(0), &[(b"a", "/a")], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
	assert!(!gone_of(&dirs.cache, &commit(0)).exists(), "its mark goes");
}

#[tokio::test]
async fn one_bundle_a_pass_and_a_failure_says_nothing_and_goes_to_the_back() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (_, a, _) = record(b"a", "/a");
	let (_, b, _) = record(b"b", "/b");
	hold_essay(&pool, "one-2026-11-20", &[(commit(0), a)]).await;
	hold_essay(&pool, "two-2026-11-21", &[(commit(1), b)]).await;
	fake.run(run(1, 0), Err("timeout"));
	fake.run(run(2, 1), Ok(Some(bundle(&commit(1), &[(b"b", "/b")], |_| {}))));
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
	assert_eq!((picked.fetched, picked.gone, picked.failures.as_slice()), (0, 0, &["timeout"][..]), "one download a pass");
	assert!(!gone_of(&dirs.cache, &commit(0)).exists(), "a failure says nothing of the commit");
	assert_eq!(
		run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched,
		1,
		"the failed commit waits behind the other"
	);
	fake.bundles.lock().unwrap().insert(1, Ok(Some(bundle(&commit(0), &[(b"a", "/a")], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1, "then it is asked again");
	assert_eq!(*fake.asked.lock().unwrap(), [1, 2, 1]);
	assert!(!retry_of(&dirs.cache, &commit(0)).exists(), "fetched: its retry mark goes");
}

#[tokio::test]
async fn commits_marked_gone_never_hold_a_new_one_back() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (_, id, _) = record(b"a", "/a");
	for n in 0..u8::try_from(COMMITS_LOOKED).unwrap() {
		hold_essay(&pool, &(String::from("gone-2026-11-0") + &(n + 1).to_string()), &[(commit(n), id.clone())]).await;
		mark_gone(&dirs.cache_ready(), &commit(n), None, "no-run").await.unwrap();
	}
	// The last commit by name, after every gone one.
	hold_essay(&pool, "new-2026-11-20", &[(commit(5), id.clone())]).await;
	fake.run(run(9, 5), Ok(Some(bundle(&commit(5), &[(b"a", "/a")], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
}

#[test]
fn a_snip_id_is_the_captures_own() {
	assert!(is_snip_id("web:/settings/appearance:phone:1.2.3:0123456789ab"));
	assert!(is_snip_id("web:/a:b:phone:1.2.3:0123456789ab"), "a path may hold a colon, as some-ui's SNIP_ID lets it");
	assert!(is_snip_id("apk:@studio/takes:android:0.9.0-rc.1:0123456789ab"));
	for bad in [
		"web:/a:phone:1.2.3:0123",
		"other:/a:phone:1.2.3:0123456789ab",
		"web:/a:phone:1/2:0123456789ab",
		"web:/a b:phone:1.2.3:0123456789ab",
	] {
		assert!(!is_snip_id(bad), "{bad}");
	}
	assert_eq!(id_parts("web:/a:phone:1.2.3:0123456789ab"), Some(("1.2.3", "0123456789ab")));
	assert!(is_commit(&commit(0)) && !is_commit("ABC") && !is_commit(&"A".repeat(40)));
}

/// GitHub as a connection that never answers.
struct Stalled;

impl Capture for Stalled {
	async fn runs(&self, _family: Family, _key: &str) -> Result<Vec<Run>, &'static str> {
		std::future::pending().await
	}

	async fn bundle(&self, _family: Family, _run: &Run, _into: &Path) -> Result<Bundle, &'static str> {
		std::future::pending().await
	}
}

#[tokio::test]
async fn a_pass_cut_off_mid_request_leaves_its_commit_at_the_back() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (_, a, _) = record(b"a", "/a");
	let (_, b, _) = record(b"b", "/b");
	hold_essay(&pool, "one-2026-11-20", &[(commit(0), a)]).await;
	hold_essay(&pool, "two-2026-11-21", &[(commit(1), b)]).await;
	let cut = tokio::time::timeout(std::time::Duration::from_millis(50), run_once(&pool, &Stalled, &dirs.essays, &dirs.folders())).await;
	assert!(cut.is_err(), "the pass's deadline cuts it off");
	assert!(retry_of(&dirs.cache, &commit(0)).exists(), "the commit it was on is marked to retry");
	// Both commits can be fetched now: the one cut off waits behind the other.
	fake.run(run(1, 0), Ok(Some(bundle(&commit(0), &[(b"a", "/a")], |_| {}))));
	fake.run(run(2, 1), Ok(Some(bundle(&commit(1), &[(b"b", "/b")], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
	assert_eq!(*fake.asked.lock().unwrap(), [2], "the next pass asks the other commit first");
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
	assert_eq!(*fake.asked.lock().unwrap(), [2, 1], "then the one cut off");
}

// Public pages, the family's second member (paulgsc/some-ui#3395).

const URL: &str = "https://github.com/paulgsc/some-ui/actions";
const AT: &str = "2026-11-23T09:30:00.000Z";

/// A page's picture's file and record, its bytes `png`, of `url` at `at`.
fn page(png: &[u8], slug: &str, url: &str, at: &str) -> (String, serde_json::Value) {
	let hash = sha(png);
	let name = String::from(slug) + "-" + &hash[..12] + ".png";
	let record = serde_json::json!({
		"url": url, "capturedAt": at, "title": "Actions", "width": 2880, "height": 1800, "scale": 2,
		"regions": [{ "name": "graph", "box": { "x": 0, "y": 0, "width": 10, "height": 10 } }],
		"image": { "name": name, "bytes": png.len(), "sha256": hash },
	});
	(name, record)
}

/// A pages bundle of `records` (each with its picture's bytes), as the
/// capture writes it, then `edit` applied before it is zipped.
fn pages_bundle(records: &[(&[u8], serde_json::Value)], edit: impl FnOnce(&mut BTreeMap<String, Vec<u8>>)) -> Vec<u8> {
	let mut files = BTreeMap::new();
	for (png, record) in records {
		files.insert(record["image"]["name"].as_str().unwrap().to_owned(), png.to_vec());
	}
	let body = serde_json::json!({ "schema": pages::SCHEMA, "pages": records.iter().map(|(_, record)| record.clone()).collect::<Vec<_>>() });
	files.insert(String::from("pages.json"), serde_json::to_vec_pretty(&body).unwrap());
	let sums: String = files.iter().map(|(name, bytes)| sha(bytes) + "  " + name + "\n").collect();
	files.insert(String::from("SHA256SUMS"), sums.into_bytes());
	edit(&mut files);
	let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
	let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
	for (name, bytes) in files {
		zip.start_file(name, options).unwrap();
		zip.write_all(&bytes).unwrap();
	}
	zip.finish().unwrap().into_inner()
}

/// A page scene's `of`: `run`'s `file` of `url` at `at`.
fn scene(run: i64, file: &str, url: &str, at: &str) -> serde_json::Value {
	serde_json::json!({ "from": "pages", "url": url, "capturedAt": at, "file": file, "run": run.to_string(), "at": ["graph"] })
}

/// Keeps a cards essay `key` whose scenes show `scenes`, each a page's `of`.
async fn hold_page_essay(pool: &SqlitePool, key: &str, scenes: &[serde_json::Value]) {
	let shots: Vec<serde_json::Value> = scenes.iter().map(|of| serde_json::json!({ "picture": { "kind": "screen", "of": of } })).collect();
	let day = &key[key.len() - 10..];
	let body = serde_json::json!({ "schema": "some-ui.cut-list/2", "stamp": { "date": day }, "shots": shots }).to_string();
	let mut conn = pool.acquire().await.unwrap();
	let essay = cards_essay_repo::Essay {
		key,
		day,
		body: body.as_bytes(),
		taken_at: "2026-11-23T08:00:00.000Z",
	};
	cards_essay_repo::keep(&mut conn, &essay).await.unwrap();
}

fn page_run(id: i64) -> Run {
	Run {
		id,
		head_sha: commit(0),
		run_attempt: 1,
	}
}

/// What the hand-off makes of `scenes` with the caches in `dirs`.
async fn pages_handed(pool: &SqlitePool, dirs: &Dirs, scenes: &[serde_json::Value]) -> essay_media::Handed {
	let essay = serde_json::json!({ "shots": scenes.iter().map(|of| serde_json::json!({ "picture": { "kind": "screen", "of": of } })).collect::<Vec<_>>() });
	let sources = essay_media::Sources {
		subject: None,
		frames: dirs.cache.join("no-frames"),
		snips: Some(dirs.cache.clone()),
		pages: Some(dirs.pages.clone()),
	};
	let items = essay_media::named(essay_media::Kind::Pages, &essay);
	let mut conn = pool.acquire().await.unwrap();
	essay_media::hand(&mut conn, &sources, essay_media::Kind::Pages, &items).await.unwrap()
}

fn refused(handed: essay_media::Handed) -> essay_media::Refusal {
	let essay_media::Handed::Refused(refusal) = handed else {
		panic!("refused: {handed:?}");
	};
	refusal
}

#[tokio::test]
async fn a_page_essays_runs_are_fetched_checked_and_handed_one_record_a_file_then_dropped_once_given() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (actions, actions_record) = page(b"actions-png", "github-com-paulgsc-some-ui-actions", URL, AT);
	let (docs, docs_record) = page(b"docs-png", "docs-github-com", "https://docs.github.com/", AT);
	let (unnamed, unnamed_record) = page(b"other-png", "example-com", "https://example.com/", AT);
	// The same picture in two runs is one; the essay names a scene of each run.
	let scenes = [
		scene(101, &actions, URL, AT),
		scene(202, &docs, "https://docs.github.com/", "2026-11-23T09:30:00Z"),
		scene(202, &actions, URL, AT),
	];
	hold_page_essay(&pool, "pages-2026-11-23", &scenes).await;
	assert_eq!(
		pages_handed(&pool, &dirs, &scenes).await,
		essay_media::Handed::Waiting(vec![String::from("101 ") + &actions, String::from("202 ") + &docs, String::from("202 ") + &actions]),
		"not fetched yet: it waits, each named by its run and file alone"
	);
	fake.page_run(
		page_run(101),
		Ok(Some(pages_bundle(&[(b"actions-png", actions_record.clone()), (b"other-png", unnamed_record)], |_| {}))),
	);
	fake.page_run(
		page_run(202),
		Ok(Some(pages_bundle(&[(b"docs-png", docs_record.clone()), (b"actions-png", actions_record.clone())], |_| {}))),
	);
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1, "one bundle a pass");
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
	let names: BTreeSet<_> = std::fs::read_dir(&dirs.pages)
		.unwrap()
		.map(|entry| entry.unwrap().file_name().into_string().unwrap())
		.collect();
	assert_eq!(names, BTreeSet::from([String::from("101"), String::from("202")]), "no partial is left");

	let essay_media::Handed::Ready(files) = pages_handed(&pool, &dirs, &scenes).await else {
		panic!("handed once fetched");
	};
	assert_eq!(files[0].0, "pages.json");
	let given: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
	assert_eq!(
		given,
		serde_json::json!({ "schema": "some-ui.pages/1", "pages": [actions_record, docs_record] }),
		"only what the essay names, one record a file, its moment compared as an instant"
	);
	let pictures: Vec<_> = files[1..].iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
	assert_eq!(pictures, [(actions.as_str(), &b"actions-png"[..]), (docs.as_str(), &b"docs-png"[..])]);
	assert!(!files.iter().any(|(name, _)| *name == unnamed));

	std::fs::write(dirs.essays.join("pages-2026-11-23.json"), b"{}").unwrap();
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
	assert_eq!(picked.dropped, 2, "once the essay is given, no cache is kept");
	assert!(!bundle_of(&dirs.pages, "101").exists() && !bundle_of(&dirs.pages, "202").exists());
}

#[tokio::test]
async fn a_scene_its_runs_bundle_does_not_give_as_named_is_gone_naming_the_run_and_file() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (actions, record) = page(b"actions-png", "github-com-paulgsc-some-ui-actions", URL, AT);
	let (absent, _) = page(b"absent-png", "github-com-absent", URL, AT);
	hold_page_essay(&pool, "pages-2026-11-23", &[scene(101, &actions, URL, AT)]).await;
	fake.page_run(page_run(101), Ok(Some(pages_bundle(&[(b"actions-png", record)], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
	for (case, of) in [
		("a file its run doesn't hold", scene(101, &absent, URL, AT)),
		("another address", scene(101, &actions, "https://github.com/other", AT)),
		("another moment", scene(101, &actions, URL, "2026-11-23T09:30:01.000Z")),
		("a moment that doesn't read", scene(101, &actions, URL, "yesterday")),
	] {
		let refusal = refused(pages_handed(&pool, &dirs, &[of.clone()]).await);
		assert_eq!(refusal.reason(), "page-gone", "{case}");
		let file = of["file"].as_str().unwrap();
		assert_eq!(refusal.items, [String::from("101 ") + file], "{case}: by its run and file, never its address");
	}
	for (case, of) in [
		("no run", serde_json::json!({ "from": "pages", "url": URL, "capturedAt": AT, "file": actions })),
		("a run that is no run's id", scene(0, &actions, URL, AT)),
		("a file that is a path", scene(101, "../101/pages.json", URL, AT)),
	] {
		assert_eq!(refused(pages_handed(&pool, &dirs, &[of]).await).reason(), "page-missing", "{case}");
	}
	let mut none = essay_media::Sources {
		subject: None,
		frames: dirs.cache.join("no-frames"),
		snips: None,
		pages: None,
	};
	let items = essay_media::named(
		essay_media::Kind::Pages,
		&serde_json::json!({ "shots": [{ "picture": { "kind": "screen", "of": scene(101, &actions, URL, AT) } }] }),
	);
	let mut conn = pool.acquire().await.unwrap();
	let handed = essay_media::hand(&mut conn, &none, essay_media::Kind::Pages, &items).await.unwrap();
	assert_eq!(refused(handed).reason(), "page-missing", "no token: home fetches no pages");
	none.pages = Some(dirs.pages.clone());
	assert!(matches!(
		essay_media::hand(&mut conn, &none, essay_media::Kind::Pages, &items).await.unwrap(),
		essay_media::Handed::Ready(_)
	));
}

#[tokio::test]
async fn one_file_two_runs_give_unlike_is_gone() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (actions, record) = page(b"actions-png", "github-com-paulgsc-some-ui-actions", URL, AT);
	let mut later = record.clone();
	later["capturedAt"] = serde_json::json!("2026-11-24T09:30:00.000Z");
	let scenes = [scene(101, &actions, URL, AT), scene(202, &actions, URL, "2026-11-24T09:30:00.000Z")];
	hold_page_essay(&pool, "pages-2026-11-23", &scenes).await;
	fake.page_run(page_run(101), Ok(Some(pages_bundle(&[(b"actions-png", record)], |_| {}))));
	fake.page_run(page_run(202), Ok(Some(pages_bundle(&[(b"actions-png", later)], |_| {}))));
	for _ in 0..2 {
		run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
	}
	let refusal = refused(pages_handed(&pool, &dirs, &scenes).await);
	assert_eq!(refusal.reason(), "page-gone");
	assert_eq!(refusal.items, [String::from("101 ") + &actions, String::from("202 ") + &actions]);
}

#[tokio::test]
async fn a_pages_bundle_that_does_not_check_out_is_refused_whole_and_said_gone() {
	let (actions, record) = page(b"actions-png", "github-com-paulgsc-some-ui-actions", URL, AT);
	let (docs, docs_record) = page(b"docs-png", "docs-github-com", "https://docs.github.com/", AT);
	let records = [(&b"actions-png"[..], record.clone()), (&b"docs-png"[..], docs_record.clone())];
	let resummed = |files: &mut BTreeMap<String, Vec<u8>>| {
		files.remove("SHA256SUMS");
		let sums: String = files.iter().map(|(name, bytes)| sha(bytes) + "  " + name + "\n").collect();
		files.insert(String::from("SHA256SUMS"), sums.into_bytes());
	};
	let rewrite = move |body: serde_json::Value| -> Edit {
		Box::new(move |files: &mut BTreeMap<String, Vec<u8>>| {
			files.insert(String::from("pages.json"), serde_json::to_vec(&body).unwrap());
			resummed(files);
		})
	};
	let swapped = actions.clone();
	let cases: Vec<(&str, Edit)> = vec![
		("a picture swapped for another", Box::new(move |files| drop(files.insert(swapped, b"other".to_vec())))),
		("a file the sums don't list", Box::new(|files| drop(files.insert(String::from("extra.png"), b"x".to_vec())))),
		("a picture missing", Box::new(|files| files.retain(|name, _| !name.starts_with("docs")))),
		("an escaping name", Box::new(|files| drop(files.insert(String::from("../escape.png"), b"x".to_vec())))),
		(
			"another schema",
			rewrite(serde_json::json!({ "schema": "some-ui.pages/2", "pages": [record.clone(), docs_record.clone()] })),
		),
		(
			"a picture named twice",
			rewrite(serde_json::json!({ "schema": pages::SCHEMA, "pages": [record.clone(), record.clone(), docs_record.clone()] })),
		),
		("a record's bytes not its file's", {
			let mut wrong = record.clone();
			wrong["image"]["bytes"] = serde_json::json!(3);
			rewrite(serde_json::json!({ "schema": pages::SCHEMA, "pages": [wrong, docs_record.clone()] }))
		}),
		("a name whose hash is not its sum's", {
			let renamed = String::from("github-com-0123456789ab.png");
			let mut wrong = record.clone();
			wrong["image"]["name"] = serde_json::json!(renamed.clone());
			let body = serde_json::json!({ "schema": pages::SCHEMA, "pages": [wrong, docs_record.clone()] });
			let (from, to) = (actions.clone(), renamed);
			Box::new(move |files: &mut BTreeMap<String, Vec<u8>>| {
				let png = files.remove(&from).unwrap();
				files.insert(to, png);
				files.insert(String::from("pages.json"), serde_json::to_vec(&body).unwrap());
				resummed(files);
			})
		}),
		("a name that is a path", {
			let mut wrong = record.clone();
			wrong["image"]["name"] = serde_json::json!(String::from("../") + &actions);
			rewrite(serde_json::json!({ "schema": pages::SCHEMA, "pages": [wrong, docs_record.clone()] }))
		}),
	];
	for (case, edit) in cases {
		let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
		let scenes = [scene(101, &docs, "https://docs.github.com/", AT)];
		hold_page_essay(&pool, "pages-2026-11-23", &scenes).await;
		fake.page_run(page_run(101), Ok(Some(pages_bundle(&records, edit))));
		let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
		assert_eq!((picked.fetched, picked.gone), (0, 1), "{case}: {picked:?}");
		assert!(!bundle_of(&dirs.pages, "101").exists(), "{case}: nothing of it is kept");
		assert!(!dirs.pages.parent().unwrap().join("escape.png").exists(), "{case}");
		assert_eq!(std::fs::read_to_string(gone_of(&dirs.pages, "101")).unwrap(), "reason=refused\nrun=101.1\n", "{case}");
		assert_eq!(refused(pages_handed(&pool, &dirs, &scenes).await).reason(), "page-gone", "{case}");
		assert_eq!(
			run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap(),
			Picked::default(),
			"{case}: not downloaded again"
		);
	}
}

#[tokio::test]
async fn a_page_run_gone_is_said_never_retried_until_a_newer_attempt_and_a_failure_only_retries() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (actions, record) = page(b"actions-png", "github-com-paulgsc-some-ui-actions", URL, AT);
	let scenes = [scene(101, &actions, URL, AT)];
	hold_page_essay(&pool, "pages-2026-11-23", &scenes).await;
	// No such run (or one of another workflow, or that did not succeed).
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().gone, 1);
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.pages, "101")).unwrap(), "reason=no-run\nrun=\n");
	assert_eq!(refused(pages_handed(&pool, &dirs, &scenes).await).reason(), "page-gone");

	// Its artifact expired: gone as that attempt, and never asked again.
	fake.page_run(page_run(101), Ok(None));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().gone, 1);
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.pages, "101")).unwrap(), "reason=gone\nrun=101.1\n");
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap(), Picked::default());
	assert_eq!(*fake.asked.lock().unwrap(), [101]);

	// A newer attempt that times out: only its retry mark, the gone one stays.
	let again = Run { run_attempt: 2, ..page_run(101) };
	fake.runs.lock().unwrap().insert((Family::Pages, String::from("101")), Ok(vec![again]));
	fake.bundles.lock().unwrap().insert(101, Err("timeout"));
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap();
	assert_eq!((picked.gone, picked.failures.as_slice()), (0, &["timeout"][..]));
	assert!(retry_of(&dirs.pages, "101").exists());
	fake.bundles.lock().unwrap().insert(101, Ok(Some(pages_bundle(&[(b"actions-png", record)], |_| {}))));
	assert_eq!(
		run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched,
		1,
		"the newer attempt clears it"
	);
	assert!(!gone_of(&dirs.pages, "101").exists() && !retry_of(&dirs.pages, "101").exists());
	assert!(matches!(pages_handed(&pool, &dirs, &scenes).await, essay_media::Handed::Ready(_)));
}

#[tokio::test]
async fn both_families_share_one_queue_and_one_bundle_a_pass() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (_, id, _) = record(b"a", "/a");
	let (actions, page_record) = page(b"actions-png", "github-com-paulgsc-some-ui-actions", URL, AT);
	hold_essay(&pool, "snipped-2026-11-22", &[(commit(0), id)]).await;
	// A page essay still waiting on something else is fetched all the same.
	hold_page_essay(&pool, "paged-2026-11-23", &[scene(101, &actions, URL, AT)]).await;
	fake.run(run(1, 0), Ok(Some(bundle(&commit(0), &[(b"a", "/a")], |_| {}))));
	fake.page_run(page_run(101), Ok(Some(pages_bundle(&[(b"actions-png", page_record)], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().fetched, 1);
	assert_eq!(*fake.asked.lock().unwrap(), [101, 1], "a page first: its bundle lives a day and can't be made again");
	assert!(bundle_of(&dirs.cache, &commit(0)).is_dir() && bundle_of(&dirs.pages, "101").is_dir());
	// A run's key in the snips' cache is no snips key: never kept there.
	std::fs::create_dir_all(bundle_of(&dirs.cache, "101")).unwrap();
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.folders()).await.unwrap().dropped, 1);
	assert!(bundle_of(&dirs.pages, "101").is_dir());
}

#[test]
fn a_run_is_a_pages_run_only_when_of_its_workflow_and_a_success() {
	let one = |path: &str, conclusion: serde_json::Value| -> Option<Run> {
		let answer = serde_json::json!({ "id": 101, "head_sha": commit(0), "run_attempt": 2, "path": path, "conclusion": conclusion, "status": "completed" });
		serde_json::from_value::<OneRun>(answer).unwrap().of_pages()
	};
	assert_eq!(one(PAGES_WORKFLOW, serde_json::json!("success")), Some(Run { run_attempt: 2, ..page_run(101) }));
	assert_eq!(one(".github/workflows/snips-capture.yml", serde_json::json!("success")), None, "another workflow's");
	assert_eq!(one(PAGES_WORKFLOW, serde_json::json!("failure")), None);
	assert_eq!(one(PAGES_WORKFLOW, serde_json::Value::Null), None, "not finished");
	assert!(is_run("101") && is_run("9223372036854775807"));
	for bad in ["", "0", "012", "1a", "99999999999999999999", "-1", "1 2"] {
		assert!(!is_run(bad), "{bad:?}");
	}
	assert_eq!(Family::Pages.artifact(&page_run(101)), "pages-bundle-101");
	assert_eq!(Family::Snips.artifact(&run(1, 0)), String::from("snips-bundle-") + &commit(0));
}
