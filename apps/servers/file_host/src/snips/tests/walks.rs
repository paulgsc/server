// Walks, the family's third member (paulgsc/some-ui#3476).

use super::*;
use crate::snips::walks as check;

/// The commit the walks' runs walked at.
fn walked() -> String {
	commit(2)
}

fn walk_run(id: i64) -> Run {
	Run {
		id,
		head_sha: walked(),
		run_attempt: 1,
	}
}

/// A step's picture's bytes: `app`'s step `n`.
fn png(app: &str, n: usize) -> Vec<u8> {
	(String::from(app) + "-step-" + &n.to_string()).into_bytes()
}

/// `app`'s walk of `steps` steps, as `walks.json` lists it.
fn entry(app: &str, steps: usize) -> serde_json::Value {
	let snips: Vec<_> = (1..=steps)
		.map(|n| serde_json::json!({ "file": check::step_file(n), "label": "step", "atMs": n * 1000 }))
		.collect();
	let pictures: Vec<_> = (1..=steps)
		.map(|n| {
			let bytes = png(app, n);
			serde_json::json!({ "name": check::picture_of(app, n), "bytes": bytes.len(), "sha256": sha(&bytes) })
		})
		.collect();
	serde_json::json!({
		"app": app,
		"walk": {
			"schema": check::WALK_SCHEMA, "app": app, "version": "1.2.3",
			"viewport": { "width": 412, "height": 915, "scale": 2 },
			"snips": snips, "taps": [{ "label": "open", "atMs": 600, "x": 10, "y": 20 }], "lengthMs": steps * 1000 + 1000,
		},
		"pictures": pictures,
	})
}

/// `run`'s walks bundle of `apps`, each `(app, steps)`, as the capture
/// writes it, then `edit` applied before it is zipped.
fn walks_bundle(run: i64, apps: &[(&str, usize)], edit: impl FnOnce(&mut BTreeMap<String, Vec<u8>>)) -> Vec<u8> {
	let walks: Vec<_> = apps.iter().map(|(app, steps)| entry(app, *steps)).collect();
	let body = serde_json::json!({ "schema": check::SCHEMA, "commit": walked(), "run": run.to_string(), "walks": walks });
	zipped(apps, &body, edit)
}

/// The bundle of `body` and the pictures of `apps`, summed, then `edit`.
fn zipped(apps: &[(&str, usize)], body: &serde_json::Value, edit: impl FnOnce(&mut BTreeMap<String, Vec<u8>>)) -> Vec<u8> {
	let mut files = BTreeMap::new();
	for (app, steps) in apps {
		for n in 1..=*steps {
			files.insert(check::picture_of(app, n), png(app, n));
		}
	}
	files.insert(String::from(check::RECORD), serde_json::to_vec_pretty(body).unwrap());
	resum(&mut files);
	edit(&mut files);
	let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
	let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
	for (name, bytes) in files {
		zip.start_file(name, options).unwrap();
		zip.write_all(&bytes).unwrap();
	}
	zip.finish().unwrap().into_inner()
}

/// `SHA256SUMS` written again over every other file.
fn resum(files: &mut BTreeMap<String, Vec<u8>>) {
	files.remove("SHA256SUMS");
	let sums: String = files.iter().map(|(name, bytes)| sha(bytes) + "  " + name + "\n").collect();
	files.insert(String::from("SHA256SUMS"), sums.into_bytes());
}

/// A walk scene's `of`: `run`'s walk of `app`, and its steps when named.
fn scene(run: i64, app: &str, steps: Option<[u64; 2]>) -> serde_json::Value {
	let mut of = serde_json::json!({ "from": "walks", "run": run.to_string(), "app": app });
	if let Some(steps) = steps {
		of["steps"] = serde_json::json!(steps);
	}
	of
}

fn essay_of(scenes: &[serde_json::Value]) -> serde_json::Value {
	serde_json::json!({ "shots": scenes.iter().map(|of| serde_json::json!({ "picture": { "kind": "screen", "of": of } })).collect::<Vec<_>>() })
}

/// Keeps a cards essay `key` whose scenes show `scenes`, each a walk's `of`.
async fn hold_walk_essay(pool: &SqlitePool, key: &str, scenes: &[serde_json::Value]) {
	hold_page_essay(pool, key, scenes).await;
}

/// What the hand-off makes of `scenes` with `sources`.
async fn walks_handed_from(pool: &SqlitePool, sources: &essay_media::Sources, scenes: &[serde_json::Value]) -> essay_media::Handed {
	let items = essay_media::named(essay_media::Kind::Walks, &essay_of(scenes));
	let mut conn = pool.acquire().await.unwrap();
	essay_media::hand(&mut conn, sources, essay_media::Kind::Walks, &items).await.unwrap()
}

/// What the hand-off makes of `scenes` with the caches in `dirs`.
async fn walks_handed(pool: &SqlitePool, dirs: &Dirs, scenes: &[serde_json::Value]) -> essay_media::Handed {
	walks_handed_from(pool, &dirs.sources(), scenes).await
}

async fn pass(pool: &SqlitePool, fake: &Fake, dirs: &Dirs) -> Picked {
	run_once(pool, fake, &dirs.essays, &dirs.folders()).await.unwrap()
}

#[tokio::test]
async fn a_walk_essays_runs_are_fetched_checked_and_handed_only_the_named_apps_then_dropped_once_given() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let scenes = [scene(301, "jobs", Some([2, 3])), scene(301, "jobs", None), scene(402, "studio", Some([1, 1]))];
	hold_walk_essay(&pool, "walks-2026-11-24", &scenes).await;
	assert_eq!(
		walks_handed(&pool, &dirs, &scenes).await,
		essay_media::Handed::Waiting(vec![String::from("301 jobs"), String::from("402 studio")]),
		"not fetched yet: it waits, each named by its run and app alone"
	);
	fake.walk_run(walk_run(301), Ok(Some(walks_bundle(301, &[("jobs", 3), ("bites", 2)], |_| {}))));
	fake.walk_run(walk_run(402), Ok(Some(walks_bundle(402, &[("studio", 1)], |_| {}))));
	assert_eq!(pass(&pool, &fake, &dirs).await.fetched, 1, "one bundle a pass");
	assert_eq!(pass(&pool, &fake, &dirs).await.fetched, 1);
	let names: BTreeSet<_> = std::fs::read_dir(&dirs.walks)
		.unwrap()
		.map(|entry| entry.unwrap().file_name().into_string().unwrap())
		.collect();
	assert_eq!(names, BTreeSet::from([String::from("301"), String::from("402")]), "no partial is left");

	let essay_media::Handed::Ready(files) = walks_handed(&pool, &dirs, &scenes).await else {
		panic!("handed once fetched");
	};
	assert_eq!(files[0].0, "walks.json");
	let given: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
	assert_eq!(
		given,
		serde_json::json!({ "schema": "some-ui.walks-given/1", "runs": [
			{ "run": "301", "commit": walked(), "walks": [entry("jobs", 3)] },
			{ "run": "402", "commit": walked(), "walks": [entry("studio", 1)] },
		]}),
		"only the apps the essay names, each once, as the bundle has them"
	);
	let pictures: Vec<_> = files[1..].iter().map(|(name, bytes)| (name.clone(), bytes.clone())).collect();
	assert_eq!(
		pictures,
		[
			(String::from("301/jobs/snips/01.png"), png("jobs", 1)),
			(String::from("301/jobs/snips/02.png"), png("jobs", 2)),
			(String::from("301/jobs/snips/03.png"), png("jobs", 3)),
			(String::from("402/studio/snips/01.png"), png("studio", 1)),
		],
		"each named walk's every picture, under its run"
	);
	// What the render re-checks: every picture listed is its sum.
	for run in given["runs"].as_array().unwrap() {
		for walk in run["walks"].as_array().unwrap() {
			for picture in walk["pictures"].as_array().unwrap() {
				let name = run["run"].as_str().unwrap().to_owned() + "/" + picture["name"].as_str().unwrap();
				let (_, bytes) = files.iter().find(|(held, _)| *held == name).unwrap();
				assert_eq!(sha(bytes), picture["sha256"].as_str().unwrap());
			}
		}
	}

	std::fs::write(dirs.essays.join("walks-2026-11-24.json"), b"{}").unwrap();
	assert_eq!(pass(&pool, &fake, &dirs).await.dropped, 2, "once the essay is given, no cache is kept");
	assert!(!bundle_of(&dirs.walks, "301").exists() && !bundle_of(&dirs.walks, "402").exists());
}

#[tokio::test]
async fn an_app_or_step_its_run_does_not_have_is_gone_and_a_scene_naming_nothing_is_missing() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	hold_walk_essay(&pool, "walks-2026-11-24", &[scene(301, "jobs", None)]).await;
	fake.walk_run(walk_run(301), Ok(Some(walks_bundle(301, &[("jobs", 3)], |_| {}))));
	assert_eq!(pass(&pool, &fake, &dirs).await.fetched, 1);
	for steps in [None, Some([1, 3]), Some([3, 3])] {
		assert!(
			matches!(walks_handed(&pool, &dirs, &[scene(301, "jobs", steps)]).await, essay_media::Handed::Ready(_)),
			"{steps:?}: within its walk"
		);
	}
	for (case, of) in [
		("an app the run left out", scene(301, "studio", None)),
		("a last step past the walk's", scene(301, "jobs", Some([2, 4]))),
		("steps all past it", scene(301, "jobs", Some([9, 12]))),
	] {
		let refusal = refused(walks_handed(&pool, &dirs, std::slice::from_ref(&of)).await);
		assert_eq!(refusal.reason(), "walk-gone", "{case}");
		assert_eq!(refusal.items, [String::from("301 ") + of["app"].as_str().unwrap()], "{case}: by its run and app");
	}
	for (case, of) in [
		("no run", serde_json::json!({ "from": "walks", "app": "jobs" })),
		("a run that is no run's id", scene(0, "jobs", None)),
		("a run as a number", serde_json::json!({ "from": "walks", "run": 301, "app": "jobs" })),
		("an app that is a path", scene(301, "../301", None)),
		("an app that is no slug", scene(301, "Jobs", None)),
		("steps from 0", scene(301, "jobs", Some([0, 1]))),
		("steps backwards", scene(301, "jobs", Some([3, 2]))),
		("one step", serde_json::json!({ "from": "walks", "run": "301", "app": "jobs", "steps": [1] })),
		(
			"steps not numbers",
			serde_json::json!({ "from": "walks", "run": "301", "app": "jobs", "steps": ["1", "2"] }),
		),
		("steps not whole", serde_json::json!({ "from": "walks", "run": "301", "app": "jobs", "steps": [1.5, 2] })),
	] {
		assert_eq!(refused(walks_handed(&pool, &dirs, &[of]).await).reason(), "walk-missing", "{case}");
	}
	// A refusal outranks a wait: an essay that can never be given says so.
	let refusal = refused(walks_handed(&pool, &dirs, &[scene(999, "jobs", None), scene(301, "jobs", Some([2, 4]))]).await);
	assert_eq!(refusal.reason(), "walk-gone");
	let mut none = dirs.sources();
	none.walks = None;
	assert_eq!(
		refused(walks_handed_from(&pool, &none, &[scene(301, "jobs", None)]).await).reason(),
		"walk-missing",
		"no token: home fetches no walks"
	);
}

/// The walks a refused bundle is of.
const APPS: &[(&str, usize)] = &[("jobs", 2), ("studio", 1)];

/// [`APPS`]' `walks.json` of run 301, then `edit` applied.
fn walks_body(edit: &dyn Fn(&mut serde_json::Value)) -> serde_json::Value {
	let walks: Vec<_> = APPS.iter().map(|(app, steps)| entry(app, *steps)).collect();
	let mut body = serde_json::json!({ "schema": check::SCHEMA, "commit": walked(), "run": "301", "walks": walks });
	edit(&mut body);
	body
}

/// `walks.json` replaced by `edit`'s, summed again.
fn rewrite(edit: &dyn Fn(&mut serde_json::Value)) -> Edit {
	let body = walks_body(edit);
	Box::new(move |files: &mut BTreeMap<String, Vec<u8>>| {
		files.insert(String::from(check::RECORD), serde_json::to_vec_pretty(&body).unwrap());
		resum(files);
	})
}

/// Bundles whose files don't check out.
fn broken_files() -> Vec<(&'static str, Edit)> {
	vec![
		(
			"a picture swapped for another",
			Box::new(|files| drop(files.insert(String::from("jobs/snips/02.png"), b"other".to_vec()))),
		),
		("a picture missing: half a walk", Box::new(|files| drop(files.remove("jobs/snips/02.png")))),
		(
			"a picture missing, summed again",
			Box::new(|files| {
				files.remove("jobs/snips/02.png");
				resum(files);
			}),
		),
		(
			"a file the sums don't list",
			Box::new(|files| drop(files.insert(String::from("jobs/snips/03.png"), b"x".to_vec()))),
		),
		(
			"a file listed and summed but no walk's",
			Box::new(|files| {
				files.insert(String::from("notes.txt"), b"x".to_vec());
				resum(files);
			}),
		),
		("no sums", Box::new(|files| drop(files.remove("SHA256SUMS")))),
		("an escaping name", Box::new(|files| drop(files.insert(String::from("../escape.png"), b"x".to_vec())))),
	]
}

/// Bundles whose `walks.json` doesn't.
fn broken_records() -> Vec<(&'static str, Edit)> {
	vec![
		("another schema", rewrite(&|body| body["schema"] = serde_json::json!("some-ui.walks/2"))),
		("another commit", rewrite(&|body| body["commit"] = serde_json::json!(commit(3)))),
		("another run", rewrite(&|body| body["run"] = serde_json::json!("302"))),
		("the run as a number", rewrite(&|body| body["run"] = serde_json::json!(301))),
		(
			"an app listed twice",
			rewrite(&|body| {
				let first = body["walks"][0].clone();
				body["walks"].as_array_mut().unwrap().push(first);
			}),
		),
		("a walk of another app", rewrite(&|body| body["walks"][0]["walk"]["app"] = serde_json::json!("studio"))),
		(
			"a walk of another schema",
			rewrite(&|body| body["walks"][0]["walk"]["schema"] = serde_json::json!("some-ui.walk/2")),
		),
		(
			"fewer pictures than steps",
			rewrite(&|body| drop(body["walks"][0]["pictures"].as_array_mut().unwrap().pop())),
		),
		(
			"more steps than pictures",
			rewrite(&|body| {
				body["walks"][0]["walk"]["snips"]
					.as_array_mut()
					.unwrap()
					.push(serde_json::json!({ "file": "snips/03.png", "label": "x", "atMs": 1 }));
			}),
		),
		("steps out of order", rewrite(&|body| body["walks"][0]["walk"]["snips"].as_array_mut().unwrap().swap(0, 1))),
		("pictures out of order", rewrite(&|body| body["walks"][0]["pictures"].as_array_mut().unwrap().swap(0, 1))),
		(
			"a step's file not two digits",
			rewrite(&|body| body["walks"][0]["walk"]["snips"][0]["file"] = serde_json::json!("snips/1.png")),
		),
		(
			"a walk of no steps",
			rewrite(&|body| {
				body["walks"][1]["walk"]["snips"] = serde_json::json!([]);
				body["walks"][1]["pictures"] = serde_json::json!([]);
			}),
		),
		(
			"an app that is a path",
			rewrite(&|body| {
				body["walks"][1]["app"] = serde_json::json!("..");
				body["walks"][1]["walk"]["app"] = serde_json::json!("..");
			}),
		),
		(
			"a picture's bytes not its file's",
			rewrite(&|body| body["walks"][0]["pictures"][0]["bytes"] = serde_json::json!(3)),
		),
		(
			"a picture's sum not its file's",
			rewrite(&|body| body["walks"][0]["pictures"][0]["sha256"] = serde_json::json!(sha(b"other"))),
		),
	]
}

#[tokio::test]
async fn a_walks_bundle_that_does_not_check_out_is_refused_whole_and_said_gone() {
	for (case, edit) in broken_files().into_iter().chain(broken_records()) {
		let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
		let scenes = [scene(301, "studio", None)];
		hold_walk_essay(&pool, "walks-2026-11-24", &scenes).await;
		fake.walk_run(walk_run(301), Ok(Some(zipped(APPS, &walks_body(&|_| {}), edit))));
		let picked = pass(&pool, &fake, &dirs).await;
		assert_eq!((picked.fetched, picked.gone), (0, 1), "{case}: {picked:?}");
		assert!(!bundle_of(&dirs.walks, "301").exists(), "{case}: nothing of it is kept, not even the walk that checks out");
		assert!(!dirs.walks.parent().unwrap().join("escape.png").exists(), "{case}");
		assert_eq!(std::fs::read_to_string(gone_of(&dirs.walks, "301")).unwrap(), "reason=refused\nrun=301.1\n", "{case}");
		let refusal = refused(walks_handed(&pool, &dirs, &scenes).await);
		assert_eq!((refusal.reason(), refusal.items.as_slice()), ("walk-gone", &[String::from("301 studio")][..]), "{case}");
		assert_eq!(pass(&pool, &fake, &dirs).await, Picked::default(), "{case}: not downloaded again");
	}
	// And the bundle as the capture writes it checks out.
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	hold_walk_essay(&pool, "walks-2026-11-24", &[scene(301, "studio", None)]).await;
	fake.walk_run(walk_run(301), Ok(Some(zipped(APPS, &walks_body(&|_| {}), |_| {}))));
	assert_eq!(pass(&pool, &fake, &dirs).await.fetched, 1);
}

#[tokio::test]
async fn an_expired_walks_bundle_is_gone_until_a_newer_attempt_and_one_under_way_is_waited_on() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let scenes = [scene(301, "jobs", None)];
	hold_walk_essay(&pool, "walks-2026-11-24", &scenes).await;
	// No such run (or one of another workflow, or that did not succeed).
	assert_eq!(pass(&pool, &fake, &dirs).await.gone, 1);
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.walks, "301")).unwrap(), "reason=no-run\nrun=\n");
	assert_eq!(refused(walks_handed(&pool, &dirs, &scenes).await).reason(), "walk-gone");

	// Its artifact expired: gone as that attempt, and never asked again.
	fake.walk_run(walk_run(301), Ok(None));
	assert_eq!(pass(&pool, &fake, &dirs).await.gone, 1);
	assert_eq!(std::fs::read_to_string(gone_of(&dirs.walks, "301")).unwrap(), "reason=gone\nrun=301.1\n");
	assert_eq!(pass(&pool, &fake, &dirs).await, Picked::default());
	assert_eq!(*fake.asked.lock().unwrap(), [301]);

	// The owner re-runs it, and the new attempt is under way: it waits.
	fake.runs.lock().unwrap().insert((Family::Walks, String::from("301")), Err(UNFINISHED));
	let picked = pass(&pool, &fake, &dirs).await;
	assert_eq!((picked.gone, picked.failures.as_slice()), (0, &[UNFINISHED][..]));
	assert!(!gone_of(&dirs.walks, "301").exists() && retry_of(&dirs.walks, "301").exists());
	assert!(matches!(walks_handed(&pool, &dirs, &scenes).await, essay_media::Handed::Waiting(_)));

	// The newer attempt's artifact is taken.
	fake.runs.lock().unwrap().remove(&(Family::Walks, String::from("301")));
	fake.walk_run(Run { run_attempt: 2, ..walk_run(301) }, Ok(Some(walks_bundle(301, &[("jobs", 2)], |_| {}))));
	assert_eq!(pass(&pool, &fake, &dirs).await.fetched, 1, "the newer attempt clears it");
	assert!(!gone_of(&dirs.walks, "301").exists() && !retry_of(&dirs.walks, "301").exists());
	assert!(matches!(walks_handed(&pool, &dirs, &scenes).await, essay_media::Handed::Ready(_)));
}

#[tokio::test]
async fn walks_share_the_one_queue_after_pages_and_before_snips_and_a_failing_one_takes_turns() {
	let (pool, dirs, fake) = (pool().await, dirs(), Fake::default());
	let (_, id, _) = record(b"a", "/a");
	let (actions, page_record) = page(b"actions-png", "github-com-paulgsc-some-ui-actions", URL, AT);
	hold_essay(&pool, "snipped-2026-11-22", &[(commit(0), id)]).await;
	hold_page_essay(&pool, "paged-2026-11-23", &[super::scene(101, &actions, URL, AT)]).await;
	hold_walk_essay(&pool, "walked-2026-11-24", &[scene(301, "jobs", None)]).await;
	fake.run(run(1, 0), Ok(Some(bundle(&commit(0), &[(b"a", "/a")], |_| {}))));
	fake.page_run(page_run(101), Ok(Some(pages_bundle(&[(b"actions-png", page_record)], |_| {}))));
	fake.walk_run(walk_run(301), Ok(Some(walks_bundle(301, &[("jobs", 1)], |_| {}))));
	for _ in 0..3 {
		assert_eq!(pass(&pool, &fake, &dirs).await.fetched, 1, "one bundle a pass, of any family");
	}
	assert_eq!(*fake.asked.lock().unwrap(), [101, 301, 1], "pages, then walks, then snips");
	// A run's key cached as another family's is never kept there.
	std::fs::create_dir_all(bundle_of(&dirs.pages, "301")).unwrap();
	assert_eq!(pass(&pool, &fake, &dirs).await.dropped, 1);
	assert!(bundle_of(&dirs.walks, "301").is_dir());

	// Failed keys take turns, whatever their family: none starves another.
	let (pool, dirs, fake) = (super::pool().await, super::dirs(), Fake::default());
	let (_, id, _) = record(b"a", "/a");
	hold_essay(&pool, "snipped-2026-11-22", &[(commit(0), id)]).await;
	hold_page_essay(&pool, "paged-2026-11-23", &[super::scene(101, &actions, URL, AT)]).await;
	hold_walk_essay(&pool, "walked-2026-11-24", &[scene(301, "jobs", None)]).await;
	fake.run(run(1, 0), Err("timeout"));
	fake.page_run(page_run(101), Err("timeout"));
	fake.walk_run(walk_run(301), Err("timeout"));
	for _ in 0..6 {
		assert_eq!(pass(&pool, &fake, &dirs).await.failures, ["timeout"]);
		// Apart past the drive's coarse clock, so two retry marks never share a time.
		tokio::time::sleep(std::time::Duration::from_millis(50)).await;
	}
	assert_eq!(*fake.asked.lock().unwrap(), [101, 301, 1, 101, 301, 1]);
}

#[test]
fn a_run_is_a_walks_run_only_when_of_snips_capture_and_a_success() {
	let one = |path: &str, conclusion: serde_json::Value| -> Result<Option<Run>, &'static str> {
		let answer = serde_json::json!({ "id": 301, "head_sha": walked(), "run_attempt": 2, "path": path, "conclusion": conclusion, "status": "completed" });
		serde_json::from_value::<OneRun>(answer).unwrap().of(WALKS_WORKFLOW)
	};
	assert_eq!(one(WALKS_WORKFLOW, serde_json::json!("success")), Ok(Some(Run { run_attempt: 2, ..walk_run(301) })));
	assert_eq!(one(PAGES_WORKFLOW, serde_json::json!("success")), Ok(None), "another workflow's");
	assert_eq!(one(WALKS_WORKFLOW, serde_json::json!("cancelled")), Ok(None));
	assert_eq!(one(WALKS_WORKFLOW, serde_json::Value::Null), Err(UNFINISHED), "not finished: only retried");
	assert_eq!(WALKS_WORKFLOW, String::from(".github/workflows/") + WORKFLOW, "the snips' own workflow");
	assert_eq!(Family::Walks.artifact(&walk_run(301)), "walks-bundle-301");
	assert!(Family::Walks.is_key("301") && !Family::Walks.is_key(&walked()));
}
