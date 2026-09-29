//! The parity test between the round corpus `@some-ui/leetype` exports and
//! what this server stores and serves (#326, LTY-SRV2).
//!
//! ## The user story
//!
//! "As the person who authors a round in `paulgsc/some-ui`, I want to find
//! out at build time if the server would store it differently from how I
//! wrote it, or refuse it." This file is that tripwire, in the shape
//! `crates/db/activity/tests/catalog_parity.rs` set for the activity
//! catalogue: a checked-in fixture exported by the client, compared against
//! what the server does with it, no server process required.
//!
//! ## The fixture
//!
//! `testdata/rounds/` is a **verbatim copy** of the client's export,
//! `packages/ui/leetype/corpus/rounds/` in `paulgsc/some-ui`: `manifest.json`
//! (`{ "rounds": ["<id>", ...] }`) and one `<id>.json` per round, each
//! written by `JSON.stringify(round, null, 2) + "\n"`. Copy the directory
//! across whole when the corpus changes; never edit a file here by hand. The
//! test is generic over whatever the directory holds, so adding a round
//! needs no change below.
//!
//! ## What it proves
//!
//! - The directory is exactly the manifest's rounds, so a round exported but
//!   not listed (or listed but not exported) fails here, not at import.
//! - Every round passes [`parse_round`], the only validation the server does.
//! - An import stores every body **byte for byte** — the server's `ETag` and
//!   idempotence are over exact bytes, so a re-serialisation anywhere would
//!   show up here first.
//! - Every round's witness rows equal `μ` as read from the file itself, member
//!   by member. The file is read here with `serde_json` alone, not through
//!   the crate's parser, so the two readings are independent.
//! - A second import of the same directory changes nothing.

use leetype_round_repo::{import_dir, parse_round, RoundRepository, MANIFEST_CEILING};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::BTreeSet;
use std::path::PathBuf;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

fn corpus() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("rounds")
}

/// A fixture that is not what the export writes is a failure of the test,
/// not a panic inside a helper.
type Fixture<T> = Result<T, Box<dyn std::error::Error>>;

fn manifest_ids() -> Fixture<Vec<String>> {
	let manifest: Value = serde_json::from_slice(&std::fs::read(corpus().join("manifest.json"))?)?;
	let rounds = manifest["rounds"].as_array().ok_or("manifest.json has no `rounds` array")?;
	rounds
		.iter()
		.map(|id| id.as_str().map(str::to_owned).ok_or_else(|| "a round id is not a string".into()))
		.collect()
}

fn file(id: &str) -> Fixture<Vec<u8>> {
	let mut name = id.to_owned();
	name.push_str(".json");
	Ok(std::fs::read(corpus().join(name))?)
}

/// `μ` and admissibility per member, read from the file with no help from
/// the crate under test.
fn mu(body: &[u8]) -> Fixture<Vec<(String, bool)>> {
	let round: Value = serde_json::from_slice(body)?;
	let options = round["diffOptions"].as_array().ok_or("no `diffOptions` array")?;
	options
		.iter()
		.map(|option| {
			let member = &option["member"];
			match (member["propositionId"].as_str(), member["admissible"].as_bool()) {
				(Some(proposition), Some(admissible)) => Ok((proposition.to_owned(), admissible)),
				_ => Err("a member without a string `propositionId` and a boolean `admissible`".into()),
			}
		})
		.collect()
}

#[test]
fn the_directory_is_exactly_the_manifests_rounds() {
	let ids = manifest_ids().unwrap();
	assert!(!ids.is_empty(), "an empty export proves nothing");
	let listed: BTreeSet<String> = ids.iter().map(|id| id.clone() + ".json").chain(["manifest.json".to_owned()]).collect();
	assert_eq!(listed.len(), ids.len() + 1, "no round is listed twice");
	let present: BTreeSet<String> = std::fs::read_dir(corpus())
		.unwrap()
		.map(|entry| entry.unwrap().file_name().into_string().unwrap())
		.collect();
	assert_eq!(present, listed);
	#[allow(clippy::cast_possible_wrap)] // a handful of files
	let count = ids.len() as i64;
	assert!(count <= MANIFEST_CEILING);
}

#[test]
fn every_exported_round_passes_the_servers_only_validation() {
	for id in manifest_ids().unwrap() {
		let parsed = parse_round(&file(&id).unwrap()).unwrap_or_else(|problems| panic!("`{id}` is refused: {problems:?}"));
		assert_eq!(parsed.id, id, "`{id}`.id is its file name");
	}
}

#[tokio::test]
async fn an_import_stores_every_round_verbatim_with_its_mu_and_a_reimport_changes_nothing() {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	let ids = manifest_ids().unwrap();

	let first = import_dir(&pool, &corpus(), "2026-09-29T00:00:00+00:00", false).await.unwrap();
	assert!(first.failed.is_empty(), "{:?}", first.failed);
	assert_eq!(first.inserted, ids);

	let repository = RoundRepository::new(pool.clone());
	let entries = repository.entries(MANIFEST_CEILING).await.unwrap();
	for id in &ids {
		let bytes = file(id).unwrap();
		let (hash, body) = repository.body(id).await.unwrap().unwrap();
		assert_eq!(body.as_bytes(), bytes.as_slice(), "`{id}` is stored byte for byte");
		assert_eq!(hash, leetype_round_repo::content_hash(&bytes), "`{id}`'s hash is over the file's bytes");

		let entry = entries.iter().find(|entry| &entry.id == id).unwrap();
		let stored: Vec<(String, bool)> = entry.witnesses.iter().map(|w| (w.proposition_id.clone(), w.admissible)).collect();
		assert_eq!(stored, mu(&bytes).unwrap(), "`{id}`'s witness rows are its members' μ, in order");
	}

	let second = import_dir(&pool, &corpus(), "2026-09-30T00:00:00+00:00", false).await.unwrap();
	assert!(second.failed.is_empty() && second.writes() == 0, "{second:?}");
	assert_eq!(second.unchanged, ids);
}
