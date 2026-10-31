//! docs/schema.md's rules for how a table lives, as tests (S13, #511).
//!
//! Every table the migrations leave standing has one lifecycle, written down
//! here beside the subject classification `privacy.rs` holds
//! (`subject::SUBJECT_SCOPED_TABLES` and `NOT_SUBJECT_SCOPED`). Four rules
//! follow from it, each checked against the real thing:
//!
//! - **Every table has a lifecycle.** Read from `sqlite_master` after the
//!   migrations run, so a new table fails until someone says how it lives.
//! - **An append-only table is append-only by construction.** It carries a
//!   trigger that aborts every `UPDATE` unconditionally, and a table with
//!   such a trigger is classified append-only. A rebuild that forgets to
//!   recreate the trigger fails here, whatever the table's own crate tests.
//!   *This proves the trigger exists, not what deletes it allows:* most
//!   append-only tables let their rows go with the account, by design.
//! - **No production SQL rewrites an append-only table.** The string
//!   literals of every non-test source file in `crates/db` and `file_host`'s
//!   `src` are read for `UPDATE`, `DELETE FROM`, `REPLACE INTO` and an
//!   upsert's `DO UPDATE` against one, and each site found fails unless
//!   `APPEND_ONLY_REWRITES` names it with why. *Literals only:* SQL built at
//!   run time from a table name (account deletion's sweep over
//!   `SUBJECT_SCOPED_TABLES`) is not seen, and the trigger is what holds it.
//! - **A declared reference is listed, and touches no subject.** Every
//!   `REFERENCES` the schema holds is in `DECLARED_REFERENCES`, neither end is
//!   subject-scoped, and its parent is append-only or the child goes with it
//!   (`ON DELETE CASCADE`). The references the schema only implies are
//!   docs/schema.md's to list; nothing here checks them.
//!
//! What a table is worth, how it grows and when it is let go are the
//! assurance ledger's `stores` (paulgsc/some-ui, held here by
//! `scripts/check_assurance_ledger.py`); where a kept row is deleted or
//! rewritten is paulgsc/some-ui's `docs/discard-sites.md`. This file says
//! only whether a row can change once written.

use crate::{schema::MIGRATOR, subject::SUBJECT_SCOPED_TABLES};
use sqlx::{
	sqlite::{SqlitePool, SqlitePoolOptions},
	Row,
};
use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
};

/// How a table's rows live once written. docs/schema.md, "Lifecycles".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lifecycle {
	/// Written once and never updated: a trigger aborts every `UPDATE`. A row
	/// leaves only as its trigger allows, with its account or never. #511
	/// calls these "records"; the ledger's `class: record` is another
	/// question (what a store is worth), so the name here says the mechanism.
	AppendOnly,
	/// Current state: a row is rewritten or deleted in place, and the value
	/// before is not kept.
	State,
	/// Content whose source lives outside this database (a seed migration,
	/// git, some-ui's corpus), and what is derived from it.
	Corpus,
	/// Rows that leave by design after a horizon or beyond a cap, not by
	/// anyone's act.
	Ephemeral,
	/// Kept from an earlier era of the server; whether it stays is S12's
	/// call 2 (#492), not this list's.
	Legacy,
}

use Lifecycle::{AppendOnly, Corpus, Ephemeral, Legacy, State};

/// Every table the migrations leave standing, its lifecycle, and why. Each
/// describes the code as it is; where an owner call pending on S12 (#492)
/// would change it, the reason says so.
const TABLE_LIFECYCLE: &[(&str, Lifecycle, &str)] = &[
	("_sqlx_migrations", State, "sqlx's own ledger of applied migrations"),
	(
		"account",
		State,
		"one row per account, never updated; deleted when its owner deletes the account (docs/identity.md invariant 9)",
	),
	(
		"activities",
		Corpus,
		"the activity catalogue, written by the seed migrations; what it should seed is S12 (#492) call 5",
	),
	(
		"activity_outcome",
		Ephemeral,
		"per-block outcomes, appended and never updated, swept after 365 days (ACTIVITY_OUTCOME_RETENTION_DAYS; the owner kept that horizon, #541); whether a durable copy comes first is S12 (#492) call 1",
	),
	("aph_entry", AppendOnly, "every revision of every aph entry"),
	("aph_settings", AppendOnly, "every revision of the aph log's settings"),
	(
		"auth_session",
		Ephemeral,
		"a sign-in session: expires (slid on use), deleted on sign-out, and expired or beyond 32 per account deleted at sign-in",
	),
	(
		"curriculum",
		Corpus,
		"TOPIK lessons imported from git by import-curriculum; a changed lesson's body is overwritten in place (version + 1)",
	),
	(
		"curriculum_publication",
		Corpus,
		"one row per publication of corpus content (the nudge's epoch); only appended today, but no trigger holds it",
	),
	(
		"engagement_charge",
		State,
		"one row per (subject, engagement class), upserted with each signal",
	),
	("engagement_gate", State, "one row per subject, the waker's due index, rewritten each pass"),
	("game_bite", AppendOnly, "each game bite, as said"),
	("game_feed_body", AppendOnly, "each distinct ESPN body, once by hash; never deleted"),
	("game_feed_play", AppendOnly, "each version of a play read from the feed; a revision is a new row; never deleted"),
	("game_feed_response", AppendOnly, "each fetch from ESPN's feed; never deleted"),
	(
		"intervention_log",
		Ephemeral,
		"what the waker decided: actuated_at filled in once, a claim no subscription received deleted (release), swept after 90 days; keep, export or serve is S12 (#492) call 1",
	),
	("jobs_event", AppendOnly, "every Jobs event the phone recorded"),
	(
		"jobs_notice_device",
		State,
		"a phone's standing request to be told, not a record (migration 20261025000400); deleted when it withdraws or FCM says the token is gone",
	),
	("jobs_notice_sent", AppendOnly, "which morning runs home told the phones of"),
	("korean_entry", AppendOnly, "every revision of every Korean entry"),
	(
		"learner_shelf",
		State,
		"a kept item: keeping it again with other bytes overwrites its body and saved_at, and removing it deletes it (a convenience copy, #387)",
	),
	(
		"leetype_round",
		Corpus,
		"rounds imported from some-ui's corpus; a changed body is overwritten in place (version + 1), a retired round stays",
	),
	("leetype_round_run", Corpus, "a round's recorded runs, replaced by each new recording"),
	("leetype_round_witness", Corpus, "derived from its round's body and rewritten with it"),
	("leetype_session_event", AppendOnly, "every LeetType session event sent home"),
	("mailbox_archive", AppendOnly, "every file home archived from the Jobs mailbox"),
	("mailbox_handed", AppendOnly, "each answer the mailbox gave a hand-off"),
	("mailbox_item", AppendOnly, "every file home read from the Jobs mailbox"),
	("mailbox_leetype_log", AppendOnly, "each decision on a finished LeetType session"),
	("mailbox_outgoing", AppendOnly, "every file filed with home for the mailbox"),
	("mailbox_transcript", AppendOnly, "which heard transcripts home filed for the mailbox"),
	(
		"mood_events",
		Legacy,
		"NFL-era editorial events, edited and deleted in place; whether it stays is S12 (#492) call 2",
	),
	("passkey", State, "an account's passkeys; the counter and serialised passkey are rewritten on each sign-in"),
	(
		"presence_leases",
		Ephemeral,
		"which context a subject is on right now: renewed in place, the oldest beyond 16 per subject deleted",
	),
	(
		"push_subscriptions",
		State,
		"a browser's push endpoint: keys and counters rewritten on resubscribe and each send, deleted on 410 Gone or unsubscribe",
	),
	("rust_round", AppendOnly, "each Rust Rounds round as the phone put it; deleted only with its account"),
	("rust_round_drop", AppendOnly, "each time the runner went away holding a program; never deleted"),
	("rust_round_run", AppendOnly, "what rustc did with each program, per toolchain; never deleted"),
	("rust_round_source", AppendOnly, "each distinct program, once by hash; never deleted"),
	(
		"sessions",
		State,
		"a study session: rewritten in place (upsert, status, refresh of an unstarted proposal) and deleted by its owner",
	),
	("storage_sample", AppendOnly, "one row set per UTC day; never replaced or deleted"),
	("studio_friction", AppendOnly, "how each Studio sitting went"),
	("studio_sitting_event", AppendOnly, "every Studio sitting event sent home"),
	(
		"tabs",
		Legacy,
		"browser-extension captures, upserted and pruned; whether it stays is S12 (#492) call 2",
	),
	("voice_note", AppendOnly, "each voice note sent home, described"),
	("voice_note_audio", AppendOnly, "each voice note's audio bytes"),
	("voice_transcript", AppendOnly, "each transcription attempt's ending"),
	("voice_transcript_word", AppendOnly, "each word of a heard transcript"),
];

/// Every `REFERENCES` the schema declares: (child, column, parent, why).
/// docs/schema.md, "References".
const DECLARED_REFERENCES: &[(&str, &str, &str, &str)] = &[
	(
		"game_feed_play",
		"response_id",
		"game_feed_response",
		"a play version names the fetch it was read from; neither is ever deleted",
	),
	(
		"game_feed_play",
		"supersedes",
		"game_feed_play",
		"a revision names the version it revises, in the same append-only table",
	),
	("game_feed_response", "sha256", "game_feed_body", "a fetch names its body by hash; a body is never deleted"),
	(
		"leetype_round_run",
		"round_id",
		"leetype_round",
		"runs are derived from their round and go with it (ON DELETE CASCADE)",
	),
	(
		"leetype_round_witness",
		"round_id",
		"leetype_round",
		"a witness is derived from its round's body and goes with it (ON DELETE CASCADE)",
	),
	(
		"rust_round_drop",
		"source_seq",
		"rust_round_source",
		"a drop names the program it held; a program is never deleted",
	),
	(
		"rust_round_run",
		"source_seq",
		"rust_round_source",
		"a run names the program it ran; a program is never deleted",
	),
];

/// Production SQL that rewrites an append-only table, allowed: (file, what,
/// table, why), `what` as `rewrite_sites` names it. Empty: no such site
/// exists. One added here says why the row may change, as a `Lifetime:`
/// line does in paulgsc/some-ui; most such sites belong in a new row
/// instead.
const APPEND_ONLY_REWRITES: &[(&str, &str, &str, &str)] = &[];

fn lifecycle_of(table: &str) -> Option<Lifecycle> {
	TABLE_LIFECYCLE.iter().find(|(name, _, _)| *name == table).map(|(_, lifecycle, _)| *lifecycle)
}

async fn migrated() -> SqlitePool {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	pool
}

/// Whether `sql`, a `CREATE TRIGGER` statement, aborts every `UPDATE` of its
/// table: `BEFORE UPDATE ON`, no `WHEN`, and a body that is exactly
/// `SELECT RAISE(ABORT, '...');`. A trigger on some columns only
/// (`UPDATE OF`), or one that aborts only some updates, is not one.
fn aborts_every_update(sql: &str) -> bool {
	let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
	let upper = collapsed.to_ascii_uppercase();
	let Some(begin) = upper.find(" BEGIN ") else {
		return false;
	};
	let (head, body) = (&upper[..begin], collapsed[begin + " BEGIN ".len()..].trim_start());
	if !head.contains(" BEFORE UPDATE ON ") || head.contains(" WHEN ") {
		return false;
	}
	let mut rest = body.chars().filter(|c| !c.is_whitespace()).collect::<String>();
	rest.make_ascii_uppercase();
	let Some(after) = rest.strip_prefix("SELECTRAISE(ABORT,'") else {
		return false;
	};
	// The message: up to the closing quote, a doubled quote being one quote.
	let mut chars = after.chars().peekable();
	while let Some(c) = chars.next() {
		if c == '\'' {
			if chars.peek() == Some(&'\'') {
				chars.next();
				continue;
			}
			return chars.collect::<String>() == ");END";
		}
	}
	false
}

/// Why the migrated schema in `pool` breaks the lifecycle rules, or nothing.
async fn lifecycle_problems(pool: &SqlitePool) -> Vec<String> {
	let tables: Vec<String> = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
		.fetch_all(pool)
		.await
		.unwrap()
		.iter()
		.map(|row| row.get("name"))
		.collect();
	let held: BTreeSet<String> = sqlx::query("SELECT tbl_name, sql FROM sqlite_master WHERE type = 'trigger'")
		.fetch_all(pool)
		.await
		.unwrap()
		.iter()
		.filter(|row| aborts_every_update(&row.get::<String, _>("sql")))
		.map(|row| row.get::<String, _>("tbl_name").to_ascii_lowercase())
		.collect();

	let mut problems = Vec::new();
	let mut seen = BTreeSet::new();
	for (table, _, _) in TABLE_LIFECYCLE {
		if !seen.insert(*table) {
			problems.push(String::from(*table) + " has more than one lifecycle in TABLE_LIFECYCLE");
		}
		if !tables.iter().any(|existing| existing == table) {
			problems.push(String::from(*table) + " has a lifecycle but no migration creates it");
		}
	}
	for table in &tables {
		match (lifecycle_of(table), held.contains(&table.to_ascii_lowercase())) {
			(None, _) => problems.push(table.clone() + " has no lifecycle: add it to lifecycle::TABLE_LIFECYCLE, with why (docs/schema.md)"),
			(Some(AppendOnly), false) => {
				problems.push(table.clone() + " is append-only but no trigger aborts every UPDATE of it: a rebuild must recreate its `" + table + "_is_append_only`");
			}
			(Some(other), true) if other != AppendOnly => {
				let mut problem = table.clone() + " has a trigger that aborts every UPDATE, so it is append-only, not ";
				problem.push_str(name_of(other));
				problems.push(problem);
			}
			_ => {}
		}
	}
	problems
}

const fn name_of(lifecycle: Lifecycle) -> &'static str {
	match lifecycle {
		AppendOnly => "AppendOnly",
		State => "State",
		Corpus => "Corpus",
		Ephemeral => "Ephemeral",
		Legacy => "Legacy",
	}
}

#[tokio::test]
async fn every_table_has_a_lifecycle_and_every_append_only_table_is_held_by_its_trigger() {
	let pool = migrated().await;
	let problems = lifecycle_problems(&pool).await;
	assert!(problems.is_empty(), "{problems:#?}");
}

/// The rule above is only worth having if a rebuild that drops the trigger
/// fails it, as `sharing_choice`'s rebuilds could have (#511).
#[tokio::test]
async fn a_table_rebuilt_without_its_trigger_fails_the_lifecycle_check() {
	let pool = migrated().await;
	sqlx::query("DROP TRIGGER jobs_event_is_append_only").execute(&pool).await.unwrap();
	sqlx::query("CREATE TABLE made_up (id INTEGER PRIMARY KEY)").execute(&pool).await.unwrap();
	let problems = lifecycle_problems(&pool).await;
	assert_eq!(problems.len(), 2, "{problems:#?}");
	assert!(problems.iter().any(|problem| problem.starts_with("jobs_event is append-only but")), "{problems:#?}");
	assert!(problems.iter().any(|problem| problem.starts_with("made_up has no lifecycle")), "{problems:#?}");
}

#[test]
fn the_trigger_rule_accepts_only_an_unconditional_abort_of_every_update() {
	let holds = [
		"CREATE TRIGGER t_is_append_only\nBEFORE UPDATE ON t\nBEGIN\n    SELECT RAISE(ABORT, 't is append-only: write a new row');\nEND",
		"create trigger t_no_update before update on t begin select raise ( abort , 'a day''s sample; is kept' ) ; end",
	];
	let does_not = [
		"CREATE TRIGGER t_is_kept BEFORE DELETE ON t BEGIN SELECT RAISE(ABORT, 'kept'); END",
		"CREATE TRIGGER t_some BEFORE UPDATE OF body ON t BEGIN SELECT RAISE(ABORT, 'x'); END",
		"CREATE TRIGGER t_when BEFORE UPDATE ON t WHEN OLD.x IS NOT NULL BEGIN SELECT RAISE(ABORT, 'x'); END",
		"CREATE TRIGGER t_where BEFORE UPDATE ON t BEGIN SELECT RAISE(ABORT, 'x') WHERE NEW.x <> OLD.x; END",
		"CREATE TRIGGER t_ignore BEFORE UPDATE ON t BEGIN SELECT RAISE(IGNORE); END",
		"CREATE TRIGGER t_leaves AFTER DELETE ON account BEGIN DELETE FROM t WHERE subject_id = OLD.subject_id; END",
	];
	for sql in holds {
		assert!(aborts_every_update(sql), "{sql}");
	}
	for sql in does_not {
		assert!(!aborts_every_update(sql), "{sql}");
	}
}

#[tokio::test]
async fn every_declared_reference_is_listed_and_touches_no_subject() {
	let pool = migrated().await;
	let scoped: BTreeSet<&str> = SUBJECT_SCOPED_TABLES.iter().map(|(table, _)| *table).collect();
	let mut problems = Vec::new();
	let mut declared = BTreeSet::new();
	for (table, _, _) in TABLE_LIFECYCLE {
		for row in sqlx::query("SELECT \"from\", \"table\", on_delete FROM pragma_foreign_key_list(?)")
			.bind(table)
			.fetch_all(&pool)
			.await
			.unwrap()
		{
			let (column, parent): (String, String) = (row.get("from"), row.get("table"));
			let cascades = row.get::<String, _>("on_delete").eq_ignore_ascii_case("CASCADE");
			let what = String::from(*table) + "." + &column + " REFERENCES " + &parent;
			if !DECLARED_REFERENCES.iter().any(|(child, from, to, _)| child == table && *from == column && *to == parent) {
				problems.push(what.clone() + " is not in DECLARED_REFERENCES: a table here names another by value (docs/schema.md, \"References\")");
			}
			if !cascades && lifecycle_of(&parent) != Some(AppendOnly) {
				problems.push(what + " can block deleting its parent: the parent is not append-only and the reference does not cascade");
			}
			declared.insert((String::from(*table), column, parent));
		}
	}
	for (child, from, to, _) in DECLARED_REFERENCES {
		let what = String::from(*child) + "." + from + " REFERENCES " + to;
		if scoped.contains(child) || scoped.contains(to) {
			problems.push(what.clone() + " touches a subject-scoped table, which leaves by subject, never through a reference");
		}
		if !declared.contains(&(String::from(*child), String::from(*from), String::from(*to))) {
			problems.push(what + " is listed but the schema does not declare it");
		}
	}
	assert!(problems.is_empty(), "{problems:#?}");
}

// --- the rewrite scan ------------------------------------------------------------

/// A string literal outside test code: the 1-based line it starts on, and
/// its text as written between the quotes.
struct Literal {
	line: usize,
	text: String,
}

/// What reading one source file found: its string literals outside test
/// code, the out-of-line test modules it declares (`#[cfg(test)] mod x;`),
/// and whether the whole file is test code (`#![cfg(test)]`).
struct Read {
	literals: Vec<Literal>,
	test_mods: Vec<String>,
	all_test: bool,
}

const fn is_ident(c: char) -> bool {
	c.is_ascii_alphanumeric() || c == '_'
}

/// `src` with every comment and string literal blanked to spaces (newlines
/// kept, so offsets and lines still match), and each literal's start and
/// text. A `'` starts a char literal only when one character (or an escape)
/// and a closing `'` follow; otherwise it is a lifetime.
fn blank(src: &[char]) -> (Vec<char>, Vec<(usize, String)>) {
	let mut code = src.to_vec();
	let mut literals = Vec::new();
	let at = |i: usize| src.get(i).copied().unwrap_or('\0');
	let wipe = |code: &mut [char], from: usize, to: usize| {
		for c in code.iter_mut().take(to.min(src.len())).skip(from) {
			if *c != '\n' {
				*c = ' ';
			}
		}
	};
	let mut i = 0;
	while i < src.len() {
		let c = at(i);
		let after_ident = i > 0 && is_ident(at(i - 1));
		if c == '/' && at(i + 1) == '/' {
			let end = (i..src.len()).find(|&j| at(j) == '\n').unwrap_or(src.len());
			wipe(&mut code, i, end);
			i = end;
		} else if c == '/' && at(i + 1) == '*' {
			let (mut depth, mut j) = (0usize, i);
			while j < src.len() {
				if at(j) == '/' && at(j + 1) == '*' {
					depth += 1;
					j += 2;
				} else if at(j) == '*' && at(j + 1) == '/' {
					depth -= 1;
					j += 2;
					if depth == 0 {
						break;
					}
				} else {
					j += 1;
				}
			}
			wipe(&mut code, i, j);
			i = j;
		} else if !after_ident && ((c == 'r' && (at(i + 1) == '"' || at(i + 1) == '#')) || (c == 'b' && at(i + 1) == 'r')) {
			let open = if c == 'b' { i + 2 } else { i + 1 };
			let hashes = (open..src.len()).take_while(|&j| at(j) == '#').count();
			if at(open + hashes) != '"' {
				i += 1;
				continue;
			}
			let start = open + hashes + 1;
			let end = (start..src.len()).find(|&j| at(j) == '"' && (1..=hashes).all(|k| at(j + k) == '#')).unwrap_or(src.len());
			literals.push((i, src[start..end].iter().collect()));
			wipe(&mut code, i, end + 1 + hashes);
			i = end + 1 + hashes;
		} else if c == '"' {
			let mut j = i + 1;
			while j < src.len() && at(j) != '"' {
				j += if at(j) == '\\' { 2 } else { 1 };
			}
			let end = j.min(src.len());
			literals.push((i, src[i + 1..end].iter().collect()));
			wipe(&mut code, i, end + 1);
			i = end + 1;
		} else if c == '\'' && at(i + 1) == '\\' {
			let end = (i + 2..src.len()).find(|&j| at(j) == '\'').unwrap_or(src.len());
			wipe(&mut code, i, end + 1);
			i = end + 1;
		} else if c == '\'' && at(i + 2) == '\'' {
			wipe(&mut code, i, i + 3);
			i += 3;
		} else {
			i += 1;
		}
	}
	(code, literals)
}

/// Whether an attribute's inside (`cfg(test)`, `tokio::test(flavor = ..)`)
/// marks test code.
fn is_test_attribute(inside: &str) -> bool {
	let inside: String = inside.chars().filter(|c| !c.is_whitespace()).collect();
	inside == "cfg(test)"
		|| inside == "test"
		|| ["tokio::test", "sqlx::test"]
			.iter()
			.any(|attr| inside == *attr || inside.starts_with(&(String::from(*attr) + "(")))
}

/// The ranges of `code` (comments and literals already blanked) that are
/// test items: from a test attribute to the end of the item it marks, its
/// closing brace or its `;`. A `;` item that declares a module names a file
/// to skip as well.
fn test_items(code: &[char]) -> (Vec<(usize, usize)>, Vec<String>) {
	let (mut ranges, mut mods) = (Vec::new(), Vec::new());
	let mut i = 0;
	while i + 1 < code.len() {
		if code[i] != '#' || code[i + 1] != '[' {
			i += 1;
			continue;
		}
		let Some(close) = (i + 2..code.len()).find(|&j| code[j] == ']') else {
			break;
		};
		if !is_test_attribute(&code[i + 2..close].iter().collect::<String>()) {
			i = close + 1;
			continue;
		}
		let (mut depth, mut j) = (0usize, close + 1);
		while j < code.len() {
			match code[j] {
				'{' => depth += 1,
				'}' if depth > 0 => {
					depth -= 1;
					if depth == 0 {
						break;
					}
				}
				';' if depth == 0 => {
					let item: String = code[close + 1..j].iter().collect();
					let words: Vec<&str> = item.split(|c: char| !is_ident(c)).filter(|word| !word.is_empty()).collect();
					if let [.., "mod", name] = words.as_slice() {
						mods.push(String::from(*name));
					}
					break;
				}
				_ => {}
			}
			j += 1;
		}
		ranges.push((i, j + 1));
		i = j + 1;
	}
	(ranges, mods)
}

fn read_source(src: &str) -> Read {
	let chars: Vec<char> = src.chars().collect();
	let (code, literals) = blank(&chars);
	let flat: String = code.iter().filter(|c| !c.is_whitespace()).collect();
	let (ranges, test_mods) = test_items(&code);
	let newlines: Vec<usize> = chars.iter().enumerate().filter(|(_, c)| **c == '\n').map(|(at, _)| at).collect();
	let line_of = |at: usize| newlines.partition_point(|newline| *newline < at) + 1;
	let literals = literals
		.into_iter()
		.filter(|(at, _)| !ranges.iter().any(|(from, to)| (*from..*to).contains(at)))
		.map(|(at, text)| Literal { line: line_of(at), text })
		.collect();
	Read {
		literals,
		test_mods,
		all_test: flat.contains("#![cfg(test)]"),
	}
}

/// The statements in `sql` that rewrite or delete rows: (what, table), what
/// being `UPDATE`, `DELETE`, `REPLACE` (which deletes the row it replaces)
/// or `UPSERT` (an `INSERT` with `DO UPDATE`).
fn rewrite_sites(sql: &str) -> Vec<(&'static str, String)> {
	let words: Vec<&str> = sql.split(|c: char| !is_ident(c)).filter(|word| !word.is_empty()).collect();
	let upper: Vec<String> = words.iter().map(|word| word.to_ascii_uppercase()).collect();
	let word = |i: usize| upper.get(i).map_or("", String::as_str);
	let upserts = (0..upper.len()).any(|i| word(i) == "DO" && word(i + 1) == "UPDATE");
	let mut sites = Vec::new();
	for i in 0..words.len() {
		let table = |at: usize| words.get(at).map(|table| table.to_ascii_lowercase());
		let found = match word(i) {
			"UPDATE" if i == 0 || word(i - 1) != "DO" => table(if word(i + 1) == "OR" { i + 3 } else { i + 1 }).map(|table| ("UPDATE", table)),
			"DELETE" if word(i + 1) == "FROM" => table(i + 2).map(|table| ("DELETE", table)),
			"REPLACE" if word(i + 1) == "INTO" => table(i + 2).map(|table| ("REPLACE", table)),
			"INTO" if upserts && i > 0 && word(i - 1) != "REPLACE" => table(i + 1).map(|table| ("UPSERT", table)),
			_ => None,
		};
		sites.extend(found);
	}
	sites
}

/// Every `.rs` file under `dir`, skipping `tests`, `benches`, `examples` and
/// `target` directories, which hold no production code.
fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return;
	};
	let mut entries: Vec<PathBuf> = entries.map(|entry| entry.unwrap().path()).collect();
	entries.sort();
	for path in entries {
		let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
		if path.is_dir() {
			if !["tests", "benches", "examples", "target"].contains(&name) {
				rust_files(&path, into);
			}
		} else if path.extension().is_some_and(|ext| ext == "rs") {
			into.push(path);
		}
	}
}

/// Where an out-of-line module `name` declared in `file` lives: beside a
/// `lib.rs`, `main.rs` or `mod.rs`, under a directory named for any other.
fn module_path(file: &Path, name: &str) -> PathBuf {
	let dir = file.parent().unwrap_or_else(|| Path::new(""));
	let stem = file.file_stem().and_then(|stem| stem.to_str()).unwrap_or("");
	let dir = if ["lib", "main", "mod"].contains(&stem) { dir.to_path_buf() } else { dir.join(stem) };
	dir.join(name)
}

/// Every site in the production code under `roots` (relative to `repo`)
/// that rewrites a table in `tables`: (file relative to `repo`, line, what,
/// table).
fn production_rewrites(repo: &Path, roots: &[&str], tables: &BTreeSet<&str>) -> Vec<(String, usize, &'static str, String)> {
	let mut files = Vec::new();
	for root in roots {
		rust_files(&repo.join(root), &mut files);
	}
	let mut skipped: Vec<PathBuf> = Vec::new();
	let mut found = Vec::new();
	for file in &files {
		let read = read_source(&std::fs::read_to_string(file).unwrap());
		for name in &read.test_mods {
			skipped.push(module_path(file, name));
		}
		if read.all_test {
			skipped.push(file.with_extension(""));
		}
		for literal in &read.literals {
			for (what, table) in rewrite_sites(&literal.text) {
				if tables.contains(table.as_str()) {
					found.push((file.clone(), literal.line, what, table));
				}
			}
		}
	}
	// A skipped module is its `name.rs` and everything under `name/`.
	let in_test_module = |file: &Path| skipped.iter().any(|module| file.with_extension("") == *module || file.starts_with(module));
	found
		.into_iter()
		.filter(|(file, _, _, _)| !in_test_module(file))
		.map(|(file, line, what, table)| {
			let relative = file.strip_prefix(repo).unwrap_or(&file).to_string_lossy().replace('\\', "/");
			(relative, line, what, table)
		})
		.collect()
}

#[test]
fn no_production_sql_rewrites_an_append_only_table() {
	let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
	let append_only: BTreeSet<&str> = TABLE_LIFECYCLE
		.iter()
		.filter(|(_, lifecycle, _)| *lifecycle == AppendOnly)
		.map(|(table, _, _)| *table)
		.collect();
	let roots = ["crates/db", "apps/servers/file_host/src"];
	let found = production_rewrites(repo, &roots, &append_only);
	// A scan that reads nothing passes too: it must see what crates/db/session does.
	let sessions = production_rewrites(repo, &roots, &BTreeSet::from(["sessions"]));
	assert!(sessions.len() >= 4, "the scan sees too few rewrites of sessions to be reading the source: {sessions:#?}");

	let mut problems = Vec::new();
	for (file, line, what, table) in &found {
		if !APPEND_ONLY_REWRITES.iter().any(|(at, kind, name, _)| at == file && kind == what && name == table) {
			problems.push(
				file.clone()
					+ ":" + &line.to_string()
					+ " " + what
					+ " " + table
					+ ": "
					+ table
					+ " is append-only; write a new row, or name the site in APPEND_ONLY_REWRITES with why",
			);
		}
	}
	for (at, kind, name, _) in APPEND_ONLY_REWRITES {
		if !found.iter().any(|(file, _, what, table)| file == at && what == kind && table == name) {
			problems.push(String::from(*at) + " " + kind + " " + name + " is in APPEND_ONLY_REWRITES but no such site exists: remove it");
		}
	}
	assert!(problems.is_empty(), "{problems:#?}");
}

/// The scan above is only worth having if it can fail: it sees each kind of
/// rewrite in plain and raw literals, and skips comments and every shape of
/// test code.
#[test]
fn the_rewrite_scan_sees_each_rewrite_and_skips_test_code() {
	let src = r##"
// "UPDATE jobs_event SET x = 1" in a comment is not SQL.
/* "DELETE FROM jobs_event" /* nested */ either */
fn live<'a>(x: &'a str) -> char {
	sqlx::query!("UPDATE OR IGNORE jobs_event SET a = ? WHERE b = ?");
	sqlx::query(r#"
		DELETE FROM voice_note WHERE seq = ?
	"#);
	let _ = "INSERT OR REPLACE INTO storage_sample (day) VALUES (?)";
	let _ = "INSERT INTO mailbox_item (seq) VALUES (?) ON CONFLICT (seq) DO UPDATE SET seq = excluded.seq";
	let _ = "INSERT INTO sessions (id) VALUES (?) ON CONFLICT (id) DO NOTHING";
	let _ = "say \"hi\" then DELETE FROM jobs_event";
	'"'
}
#[cfg(test)]
mod tests {
	fn inner() { let _ = "UPDATE jobs_event SET x = '}'"; if true { } }
}
#[tokio::test(flavor = "multi_thread")]
async fn refused() { let _ = "DELETE FROM jobs_event"; }
#[cfg(test)]
mod elsewhere;
"##;
	let read = read_source(src);
	let sites: Vec<(usize, &str, String)> = read
		.literals
		.iter()
		.flat_map(|literal| rewrite_sites(&literal.text).into_iter().map(move |(what, table)| (literal.line, what, table)))
		.collect();
	let expected = [
		(5, "UPDATE", "jobs_event"),
		(6, "DELETE", "voice_note"),
		(9, "REPLACE", "storage_sample"),
		(10, "UPSERT", "mailbox_item"),
		(12, "DELETE", "jobs_event"),
	];
	assert_eq!(
		sites,
		expected.iter().map(|(line, what, table)| (*line, *what, String::from(*table))).collect::<Vec<_>>(),
		"{:#?}",
		read.literals.iter().map(|literal| &literal.text).collect::<Vec<_>>()
	);
	assert_eq!(read.test_mods, ["elsewhere"]);
	assert!(!read.all_test);
	assert!(read_source("#![cfg(test)]\nfn x() {}").all_test);
	assert_eq!(module_path(Path::new("a/src/lib.rs"), "tests"), Path::new("a/src/tests"));
	assert_eq!(module_path(Path::new("a/src/repo.rs"), "tests"), Path::new("a/src/repo/tests"));
}
