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
	runs: Mutex<HashMap<String, Result<Vec<Run>, &'static str>>>,
	/// A run's bundle: its zip's bytes, `None` for one gone, or a failure.
	bundles: Mutex<HashMap<i64, Answer>>,
	asked: Mutex<Vec<i64>>,
}

impl Fake {
	fn run(&self, run: Run, bundle: Answer) {
		self.bundles.lock().unwrap().insert(run.id, bundle);
		if let Ok(held) = self.runs.lock().unwrap().entry(run.head_sha.clone()).or_insert_with(|| Ok(Vec::new())) {
			held.push(run);
		}
	}
}

impl Capture for Fake {
	async fn runs(&self, commit: &str) -> Result<Vec<Run>, &'static str> {
		self.runs.lock().unwrap().get(commit).cloned().unwrap_or(Ok(Vec::new()))
	}

	async fn bundle(&self, run: &Run, into: &Path) -> Result<Bundle, &'static str> {
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
}

fn dirs() -> Dirs {
	let root = tempfile::tempdir().unwrap();
	let (essays, cache) = (root.path().join("essays"), root.path().join("snips"));
	std::fs::create_dir_all(&essays).unwrap();
	Dirs { _root: root, essays, cache }
}

/// What the hand-off makes of `snips` with the cache in `dirs`.
async fn handed(pool: &SqlitePool, dirs: &Dirs, snips: &[(String, String)]) -> essay_media::Handed {
	let sources = essay_media::Sources {
		subject: None,
		frames: dirs.cache.join("no-frames"),
		snips: Some(dirs.cache.clone()),
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
	assert_eq!(handed(&pool, &dirs, &wanted).await, essay_media::Handed::Waiting, "not fetched yet: it waits");

	fake.run(
		run(10, 0),
		Ok(Some(bundle(&commit(0), &[(b"settings-png", "/settings"), (b"studio-png", "/studio")], |_| {}))),
	);
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap();
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

	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap(), Picked::default(), "fetched once");
	std::fs::write(dirs.essays.join("shown-2026-11-20.json"), b"{}").unwrap();
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap();
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
		let picked = run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap();
		assert_eq!((picked.fetched, picked.gone), (0, 1), "{case}: {picked:?}");
		assert!(!bundle_of(&dirs.cache, &commit(1)).exists(), "{case}: nothing of it is kept");
		assert!(!dirs.cache.parent().unwrap().join("escape.png").exists());
		assert!(std::fs::read_to_string(gone_of(&dirs.cache, &commit(1))).unwrap().starts_with("reason=refused\n"), "{case}");
		let essay_media::Handed::Refused(refusal) = handed(&pool, &dirs, &wanted).await else {
			panic!("{case}: refused");
		};
		assert_eq!(refusal.reason(), "snip-gone", "{case}");
		assert_eq!(
			run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap(),
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
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap();
	assert_eq!(picked.gone, 1, "no run of its commit");
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.cache, &commit(0))).unwrap(), "reason=no-run\nrun=\n");
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap(), Picked::default(), "said once");

	// A run whose artifact has expired.
	fake.run(run(7, 0), Ok(None));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap().gone, 1);
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.cache, &commit(0))).unwrap(), "reason=gone\nrun=7.1\n");
	assert_eq!(
		run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap(),
		Picked::default(),
		"not asked again for that run"
	);
	assert_eq!(*fake.asked.lock().unwrap(), [7]);

	// The owner runs it again: GitHub's re-run keeps the id, a new attempt.
	let again = Run { run_attempt: 2, ..run(7, 0) };
	fake.runs.lock().unwrap().insert(commit(0), Ok(vec![run(7, 0), again]));
	fake.bundles.lock().unwrap().insert(7, Ok(Some(bundle(&commit(0), &[(b"a", "/a")], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap().fetched, 1);
	assert!(!gone_of(&dirs.cache, &commit(0)).exists(), "its mark goes");
}

#[tokio::test]
async fn one_bundle_a_pass_and_a_failure_records_nothing() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (_, a, _) = record(b"a", "/a");
	let (_, b, _) = record(b"b", "/b");
	hold_essay(&pool, "one-2026-11-20", &[(commit(0), a)]).await;
	hold_essay(&pool, "two-2026-11-21", &[(commit(1), b)]).await;
	fake.run(run(1, 0), Err("timeout"));
	fake.run(run(2, 1), Ok(Some(bundle(&commit(1), &[(b"b", "/b")], |_| {}))));
	let picked = run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap();
	assert_eq!(
		(picked.fetched, picked.gone, picked.failures.as_slice()),
		(0, 0, &["timeout"][..]),
		"a failure ends the pass"
	);
	assert!(!gone_of(&dirs.cache, &commit(0)).exists(), "and says nothing of it");
	fake.bundles.lock().unwrap().insert(1, Ok(Some(bundle(&commit(0), &[(b"a", "/a")], |_| {}))));
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap().fetched, 1, "asked again");
	assert_eq!(*fake.asked.lock().unwrap(), [1, 1], "one bundle a pass");
	assert_eq!(run_once(&pool, &fake, &dirs.essays, &dirs.cache).await.unwrap().fetched, 1);
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
