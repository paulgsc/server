#!/usr/bin/env python3
"""The server's half of paulgsc/some-ui's assurance ledger holds (AS10, #526; GC4).

some-ui keeps one record of what the system promises about outcomes and work,
`packages/assurance/src/ledger.json` (grammar: some-ui's
`docs/operational-assurance.md`). Its own check (`pnpm check:assurance`) holds
everything it can see, but it cannot see this repository, so a `server:` path
it names is taken as written. This script reads that ledger, as
`.github/workflows/assurance.yml` checks it out from some-ui's `main`, and
holds four rules against this repository as the PR leaves it:

1. **A proof that holds exists.** Every `server:<file>#<name>` proof of an
   objective whose verdict is `holds` (and that has not retired) names a
   file git tracks here that contains `<name>`, at least three characters,
   as written. This is some-ui's own rule for `some-ui:` proofs. A renamed
   test, a moved file or a deleted one fails it.
2. **A path it names exists.** Every `server:` path in an objective's
   `evidence` refs or `watch` ref, and in a family's `paths`, is a tracked
   file or a directory holding one (anything after a `#` is not the path).
3. **Every background task belongs to a family.** A background task is a
   spawn site in file_host's own source (`apps/servers/file_host/src`,
   without `src/bin`, which holds one-shot tools, and without test code):
   `tokio::spawn(`, `tokio::task::spawn_blocking(`, `std::thread::spawn(`,
   `runtime.spawn(`, and any other `::spawn(`, `.spawn(`,
   `::spawn_blocking(` or `::spawn_local(` call. The task belongs where its
   module does: a family's `server:` path covers a file when the file is that
   path, lies under it, is the module root of a directory path
   (`src/mailbox.rs` for `src/mailbox`), or is a child module of a file path
   (`src/mailbox/pass/x.rs` for `src/mailbox/pass.rs`). A file with spawn
   sites that no family covers fails, unless it is in NOT_BACKGROUND (below,
   spawns that owe no later outcome, with their exact count) or KNOWN_UNPLACED
   (a real background task the ledger has not placed yet; a warning).
4. **Every table is a store, and every store a table** (STORAGE5,
   some-ui#2226). The tables `migrations/*.up.sql` leave standing, applied
   in file-name order (`CREATE TABLE` adds, `DROP TABLE` removes,
   `ALTER TABLE x RENAME TO y` renames; `--` and `/* */` comments and string
   literals are read out first; `TEMP` tables, sqlx's `_sqlx_migrations` and
   `sqlite_*` are not stores), each have a `server:<table>` entry in the
   ledger's `stores`, and every `server:` store names one of them. SQLite
   itself then applies the same files to an empty database, and a table set
   that differs from this reading fails, so a statement shape the reading
   misses is seen. A ledger with no `stores` list warns.

Why file_host, and why by module: the ledger's families describe home, and
home is file_host. Its families already name the modules their work lives in
(`server:apps/servers/file_host/src/mailbox`), so "the task belongs to the
family whose path holds the code that spawns it" asks nothing new of the
ledger. Library crates file_host links (`some-transport`, `ws-connection`)
spawn connection plumbing of their own; they are outside this rule, and so
is any other app here.

Test code is read out lexically: comments and string literals are blanked,
and an item under `#[cfg(test)]`, `#[test]` or `#[tokio::test]` is skipped
to its closing brace (an inline `mod tests { .. }`) or, for an out-of-line
`#[cfg(test)] mod name;`, the module's file is skipped. A `use` that imports
`spawn` itself (`use tokio::spawn;`) fails, since its bare `spawn(` calls
read the same as a local function's: write the path at the call site.

NOT_BACKGROUND is a server-side judgement, so it is exact: an entry whose
count no longer matches fails, so a poller added to a listed file is seen.
KNOWN_UNPLACED waits on a some-ui change: its entries warn, and one the
ledger has since placed warns too, asking for the entry to go. Neither
fails, because some-ui's `main` moves on its own, and a server PR that did
not touch a task would otherwise turn red the day the ledger placed it.

The ledger is read for what these rules need and nothing else: fields the
rules do not read (a store's `growth`, `synchronous`, a field added later)
are ignored, never rejected; some-ui's own check holds the ledger's shape. A
field these rules do need that is missing or of the wrong kind fails, naming
the field.

    python3 scripts/check_assurance_ledger.py --some-ui <checkout of paulgsc/some-ui>
    python3 scripts/check_assurance_ledger.py --ledger <ledger.json>
    python3 scripts/check_assurance_ledger.py --self-test

With `--some-ui`, the ledger is found by its `schema` among the files that
checkout tracks named `ledger.json`, so moving it is a change to that repo
alone. `--self-test` runs every rule on fixed inputs first, so an edit that
stops one seeing a violation fails instead of passing every run. The design
and its boundaries: docs/assurance.md.
"""

from __future__ import annotations

import argparse
import json
import re
import sqlite3
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "some-ui.assurance-ledger/1"
TASK_ROOT = "apps/servers/file_host/src"
TASK_SKIP = ("apps/servers/file_host/src/bin/",)

# Spawn sites that owe no later outcome, so belong to no family. Exact counts:
# a spawn added to a listed file changes the count and fails until placed.
NOT_BACKGROUND: dict[str, tuple[int, str]] = {
	"apps/servers/file_host/src/main.rs": (1, "forwards the shutdown signal to the shared cancel token; it does no work of its own"),
	"apps/servers/file_host/src/websocket.rs": (
		1,
		"removes a closed connection from the store; per connection, and the ledger places `websocket` in `synchronous` (owed only while open)",
	),
	"apps/servers/file_host/src/websocket/broadcast/handlers.rs": (2, "a connection's event forwarder and its NATS subscriptions; per connection, ended with it"),
	"apps/servers/file_host/src/websocket/message/handlers.rs": (1, "a connection's incoming-message loop; per connection, ended with it"),
}

# Real background tasks the ledger has not placed yet: warned, not failed
# (see the docstring). Each says what some-ui's ledger needs.
KNOWN_UNPLACED: dict[str, str] = {
	"apps/servers/file_host/src/storage.rs": (
		"the daily storage sampler (STORAGE1, #542). It fits family `what-is-kept`: add "
		"`server:apps/servers/file_host/src/storage.rs` to its `paths` in some-ui's ledger"
	),
	"apps/servers/file_host/src/metrics/periodic.rs": (
		"the 10 s operational-gauge sweep (connections, pool, rate limiter, storage footprint). No family "
		"covers it: some-ui's ledger needs a placement the owner chooses (a family whose paths include "
		"`server:apps/servers/file_host/src/metrics/periodic.rs`)"
	),
}

PROOF = re.compile(r"^server:([^#\s]+)#(.{3,})$")
SPAWN = re.compile(r"(?:\b(?:tokio|task|thread)\s*::\s*|\.\s*)spawn(?:_blocking|_local)?\s*\(")
RAW_STRING = re.compile(r'b?r(#*)"')
ATTRIBUTE_START = re.compile(r"\s*#\[")
BARE_SPAWN_IMPORT = re.compile(r"\buse\s+(?:tokio|std::thread|tokio::task|tokio::runtime)\b[^;]*\bspawn(?:_blocking|_local)?\b(?!\s*::)[^;]*;")
TEST_ATTRIBUTE = re.compile(r"#\[\s*(?:cfg\s*\(\s*test\s*\)|test|tokio::test\b[^\]]*)\s*\]")
OUT_OF_LINE_MOD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")


@dataclass
class Report:
	errors: list[tuple[str, str]] = field(default_factory=list)
	warnings: list[tuple[str, str]] = field(default_factory=list)
	notes: list[str] = field(default_factory=list)

	def error(self, title: str, message: str) -> None:
		self.errors.append((title, message))

	def warn(self, title: str, message: str) -> None:
		self.warnings.append((title, message))


@dataclass(frozen=True)
class Repo:
	"""What the rules need from this repository: its tracked files, and their text."""

	files: frozenset[str]
	read: Callable[[str], str]

	def is_file(self, path: str) -> bool:
		return path in self.files

	def exists(self, path: str) -> bool:
		path = path.rstrip("/")
		return path in self.files or any(f.startswith(path + "/") for f in self.files)


# --- reading the ledger: only what the rules need --------------------------------


class LedgerShape(Exception):
	pass


def need(value: Any, kind: type, where: str) -> Any:
	if not isinstance(value, kind):
		raise LedgerShape(f"{where} is {'missing' if value is None else type(value).__name__}, not a {kind.__name__}")
	return value


def optional_str(value: Any, where: str) -> str | None:
	if value is None or isinstance(value, str):
		return value
	raise LedgerShape(f"{where} is {type(value).__name__}, not a string")


def server_path(ref: str | None) -> str | None:
	"""`server:<path>[#...]` -> `<path>`; anything else (some-ui:, an issue, a URL) -> None."""
	if not ref or not ref.startswith("server:"):
		return None
	return ref[len("server:") :].split("#", 1)[0]


@dataclass(frozen=True)
class Objective:
	id: str
	verdict: str
	retired: bool
	proofs: tuple[str, ...]
	refs: tuple[tuple[str, str], ...]  # (what, ref)


@dataclass(frozen=True)
class Family:
	id: str
	paths: tuple[str, ...]
	objectives: tuple[Objective, ...]


def read_stores(data: dict[str, Any]) -> list[str] | None:
	"""Each `stores` entry's `store` name; None when the ledger has no `stores` list."""
	if "stores" not in data:
		return None
	return [need(need(e, dict, f"stores[{i}]").get("store"), str, f"stores[{i}].store") for i, e in enumerate(need(data["stores"], list, "stores"))]


def read_ledger(data: Any) -> list[Family]:
	need(data, dict, "the ledger")
	schema = data.get("schema")
	if schema != SCHEMA:
		raise LedgerShape(f"schema is {schema!r}, not {SCHEMA!r}: a new grammar, so this reader needs updating to it")
	families = []
	for i, raw in enumerate(need(data.get("families"), list, "families")):
		need(raw, dict, f"families[{i}]")
		fid = need(raw.get("id"), str, f"families[{i}].id")
		paths = tuple(need(p, str, f"{fid}.paths[{j}]") for j, p in enumerate(need(raw.get("paths"), list, f"{fid}.paths")))
		objectives = []
		for k, o in enumerate(need(raw.get("objectives"), list, f"{fid}.objectives")):
			need(o, dict, f"{fid}.objectives[{k}]")
			oid = need(o.get("id"), str, f"{fid}.objectives[{k}].id")
			verdict = need(o.get("verdict"), str, f"{oid}.verdict")
			proofs = tuple(need(p, str, f"{oid}.proof[{j}]") for j, p in enumerate(o.get("proof") or []))
			refs: list[tuple[str, str]] = []
			for j, e in enumerate(need(o.get("evidence"), list, f"{oid}.evidence")):
				need(e, dict, f"{oid}.evidence[{j}]")
				ref = optional_str(e.get("ref"), f"{oid}.evidence[{j}].ref")
				if ref:
					refs.append(("evidence", ref))
			watch = need(o.get("watch"), dict, f"{oid}.watch")
			ref = optional_str(watch.get("ref"), f"{oid}.watch.ref")
			if ref:
				refs.append(("watch", ref))
			objectives.append(Objective(oid, verdict, bool(o.get("retired")), proofs, tuple(refs)))
		families.append(Family(fid, paths, tuple(objectives)))
	return families


# --- rules 1 and 2: proofs and paths ------------------------------------------------


def check_proofs_and_paths(families: list[Family], repo: Repo, report: Report) -> int:
	"""Returns how many server proofs were checked."""
	proofs = 0
	for family in families:
		for path in family.paths:
			where = server_path(path)
			if where is not None and not repo.exists(where):
				report.error(f"Ledger path missing ({family.id})", f"family {family.id} names path {where}, which is not in this repository")
		for o in family.objectives:
			for what, ref in o.refs:
				where = server_path(ref)
				if where is not None and not repo.exists(where):
					report.error(f"Ledger path missing ({o.id})", f"{o.id}'s {what} names {where}, which is not in this repository")
			if o.retired or o.verdict != "holds":
				continue
			for proof in o.proofs:
				if not proof.startswith("server:"):
					continue
				proofs += 1
				match = PROOF.match(proof)
				if not match:
					report.error(f"Proof unreadable ({o.id})", f"{o.id} holds by {proof!r}, which is not server:<file>#<name> with a name of at least 3 characters")
					continue
				path, name = match.group(1), match.group(2)
				if not repo.is_file(path):
					report.error(f"Proof missing ({o.id})", f"{o.id} holds by {path}#{name}, and {path} is not a tracked file here")
				elif name not in repo.read(path):
					report.error(f"Proof missing ({o.id})", f"{o.id} holds by {path}#{name}, and {path} does not contain {name!r}")
	return proofs


# --- rule 3: background tasks --------------------------------------------------------


def mask(source: str) -> str:
	"""Comments and the insides of string and char literals as spaces; newlines and offsets kept."""
	out = list(source)
	i, n = 0, len(source)

	def blank(start: int, end: int) -> None:
		for j in range(start, min(end, n)):
			if out[j] != "\n":
				out[j] = " "

	while i < n:
		c = source[i]
		if source.startswith("//", i):
			end = source.find("\n", i)
			end = n if end == -1 else end
			blank(i, end)
			i = end
		elif source.startswith("/*", i):
			depth, j = 1, i + 2
			while j < n and depth:
				if source.startswith("/*", j):
					depth, j = depth + 1, j + 2
				elif source.startswith("*/", j):
					depth, j = depth - 1, j + 2
				else:
					j += 1
			blank(i, j)
			i = j
		elif c in "br" and (raw := RAW_STRING.match(source, i)) and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")):
			close = '"' + raw.group(1)
			end = source.find(close, raw.end())
			end = n if end == -1 else end + len(close)
			blank(raw.end(), end - len(close))
			i = end
		elif c == '"':
			j = i + 1
			while j < n and source[j] != '"':
				j += 2 if source[j] == "\\" else 1
			blank(i + 1, j)
			i = j + 1
		elif c == "'":
			if source.startswith("\\", i + 1):
				end = source.find("'", i + 3)
				end = n if end == -1 else end
				blank(i + 1, end)
				i = end + 1
			elif i + 2 < n and source[i + 2] == "'":
				blank(i + 1, i + 2)
				i += 3
			else:
				i += 1  # a lifetime
		else:
			i += 1
	return "".join(out)


def item_end(masked: str, start: int) -> tuple[int, str | None]:
	"""From just after an attribute: (end of the item, the out-of-line module's name if it is one)."""
	j = start
	while True:
		attr = ATTRIBUTE_START.match(masked, j)
		if not attr:
			break
		depth, k = 1, attr.end()
		while k < len(masked) and depth:
			depth += {"[": 1, "]": -1}.get(masked[k], 0)
			k += 1
		j = k
	brace, semi = masked.find("{", j), masked.find(";", j)
	if semi != -1 and (brace == -1 or semi < brace):
		line_end = masked.find("\n", semi)
		module = OUT_OF_LINE_MOD.match(masked[j : semi + 1])
		return (semi + 1 if line_end == -1 else line_end, module.group(1) if module else None)
	if brace == -1:
		return (len(masked), None)
	depth, k = 1, brace + 1
	while k < len(masked) and depth:
		depth += {"{": 1, "}": -1}.get(masked[k], 0)
		k += 1
	return (k, None)


def without_tests(masked: str) -> tuple[str, list[str]]:
	"""The masked source with test items blanked, and the out-of-line test modules it declares."""
	out = list(masked)
	modules: list[str] = []
	for attr in reversed(list(TEST_ATTRIBUTE.finditer(masked))):
		end, module = item_end(masked, attr.end())
		if module:
			modules.append(module)
		for j in range(attr.start(), end):
			if out[j] != "\n":
				out[j] = " "
	return "".join(out), modules


def module_files(declaring: str, name: str) -> tuple[str, ...]:
	"""Where `mod name;` in `declaring` lives: the file and the directory of its children."""
	parent = declaring.rsplit("/", 1)[0]
	stem = declaring.rsplit("/", 1)[1][: -len(".rs")]
	base = parent if stem in ("lib", "main", "mod") else f"{parent}/{stem}"
	return (f"{base}/{name}.rs", f"{base}/{name}/")


@dataclass(frozen=True)
class SpawnScan:
	sites: dict[str, list[int]]  # file -> 1-based lines
	unreadable: list[tuple[str, int]]


def scan_spawns(repo: Repo) -> SpawnScan:
	files = sorted(f for f in repo.files if f.startswith(TASK_ROOT + "/") and f.endswith(".rs") and not f.startswith(TASK_SKIP))
	test_only: list[str] = []
	stripped: dict[str, str] = {}
	for path in files:
		text, modules = without_tests(mask(repo.read(path)))
		stripped[path] = text
		test_only += [where for module in modules for where in module_files(path, module)]
	sites: dict[str, list[int]] = {}
	unreadable: list[tuple[str, int]] = []
	for path, text in stripped.items():
		if any(path == t or (t.endswith("/") and path.startswith(t)) for t in test_only):
			continue
		lines = [text.count("\n", 0, m.start()) + 1 for m in SPAWN.finditer(text)]
		if lines:
			sites[path] = lines
		unreadable += [(path, text.count("\n", 0, m.start()) + 1) for m in BARE_SPAWN_IMPORT.finditer(text)]
	return SpawnScan(sites, unreadable)


def covers(family_path: str, file: str) -> bool:
	"""Whether a family's server path holds `file`, by Rust's module layout."""
	path = family_path.rstrip("/")
	if file == path or file.startswith(path + "/"):
		return True
	if file == path + ".rs":
		return True
	return path.endswith(".rs") and file.startswith(path[: -len(".rs")] + "/")


def check_tasks(families: list[Family], repo: Repo, report: Report) -> None:
	scan = scan_spawns(repo)
	for path, line in scan.unreadable:
		report.error("Spawn this check cannot see", f"{path}:{line} imports `spawn` by itself, so its calls read as a local function's: call it by its path (`tokio::spawn(`)")
	owners: dict[str, list[str]] = {}
	for family in families:
		for raw in family.paths:
			where = server_path(raw)
			if where is None:
				continue
			for file in scan.sites:
				if covers(where, file):
					owners.setdefault(file, []).append(family.id)
	placed = unplaced = 0
	for file, lines in sorted(scan.sites.items()):
		at = ", ".join(str(n) for n in lines)
		if file in owners:
			placed += 1
			report.notes.append(f"{file} (lines {at}): family {', '.join(sorted(set(owners[file])))}")
			if file in KNOWN_UNPLACED:
				report.warn("Known unplaced task is placed", f"{file} is now in family {', '.join(sorted(set(owners[file])))}: delete its KNOWN_UNPLACED entry in scripts/check_assurance_ledger.py")
			continue
		if file in NOT_BACKGROUND:
			count, why = NOT_BACKGROUND[file]
			if len(lines) == count:
				report.notes.append(f"{file} (lines {at}): not a background task, {why}")
			else:
				report.error(
					"Spawn in a file listed as no background task",
					f"{file} has {len(lines)} spawn sites (lines {at}), and NOT_BACKGROUND allows {count} ({why}). A new background task belongs to a family in some-ui's ledger; if it owes no later outcome either, change the count",
				)
			continue
		unplaced += 1
		if file in KNOWN_UNPLACED:
			report.warn("Background task in no family (known)", f"{file} (lines {at}) is {KNOWN_UNPLACED[file]}")
		else:
			report.error(
				"Background task in no family",
				f"{file} spawns a task (lines {at}) that no family's server paths cover. Place it in some-ui's ledger (a family's `paths`) before this merges, or, if it owes no later outcome, list it in NOT_BACKGROUND with why",
			)
	for file in NOT_BACKGROUND:
		if file not in scan.sites:
			report.error("Stale NOT_BACKGROUND entry", f"{file} no longer spawns anything: delete its NOT_BACKGROUND entry in scripts/check_assurance_ledger.py")
	report.notes.append(f"{sum(len(v) for v in scan.sites.values())} spawn sites in {len(scan.sites)} files: {placed} files in a family, {unplaced} in none")


# --- rule 4: tables and stores ----------------------------------------------------------


def sql_name(group: str) -> str:
	"""A table name, perhaps quoted, perhaps after `schema.`; the name itself captured as `group`."""
	quoted = r"""(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+)"""
	return rf"(?:{quoted}\s*\.\s*)?(?P<{group}>{quoted})"


SQL_STATEMENT = re.compile(
	rf"\bCREATE\s+(?P<temp>(?:TEMP|TEMPORARY)\s+)?(?:VIRTUAL\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?{sql_name('created')}"
	rf"|\bDROP\s+TABLE\s+(?:IF\s+EXISTS\s+)?{sql_name('dropped')}"
	rf"|\bALTER\s+TABLE\s+{sql_name('renamed')}\s+RENAME\s+TO\s+{sql_name('to')}",
	re.IGNORECASE,
)
NOT_A_STORE = re.compile(r"^(?:_sqlx_migrations|sqlite_.*)$")


def sql_code(sql: str) -> str:
	"""The SQL with `--` and `/* */` comments and the insides of '...' literals blanked."""
	out, i, n = [], 0, len(sql)
	while i < n:
		if sql.startswith("--", i):
			end = sql.find("\n", i)
			i = n if end == -1 else end
		elif sql.startswith("/*", i):
			end = sql.find("*/", i + 2)
			i = n if end == -1 else end + 2
			out.append(" ")
		elif sql[i] == "'":
			j = i + 1
			while j < n and not (sql[j] == "'" and not sql.startswith("''", j)):
				j += 2 if sql.startswith("''", j) else 1
			out.append("''")
			i = j + 1
		else:
			out.append(sql[i])
			i += 1
	return "".join(out)


def unquote(name: str) -> str:
	return (name[1:-1] if name[:1] in "\"`[" else name).lower()


def standing_tables(repo: Repo) -> set[str]:
	"""The tables `migrations/*.up.sql`, applied in file-name order, leave standing (temp ones aside)."""
	standing: set[str] = set()
	for path in sorted(f for f in repo.files if re.fullmatch(r"migrations/[^/]+\.up\.sql", f)):
		temp: set[str] = set()
		for m in SQL_STATEMENT.finditer(sql_code(repo.read(path))):
			if m.group("created"):
				(temp if m.group("temp") else standing).add(unquote(m.group("created")))
			elif m.group("dropped"):
				name = unquote(m.group("dropped"))
				(temp if name in temp else standing).discard(name)
			else:
				old, new = unquote(m.group("renamed")), unquote(m.group("to"))
				where = temp if old in temp else standing
				if old in where:
					where.discard(old)
					where.add(new)
	return {t for t in standing if not NOT_A_STORE.match(t)}


def applied_tables(repo: Repo) -> set[str] | str:
	"""The same set, from SQLite itself applying the migrations to an empty database; why not, if it cannot."""
	db = sqlite3.connect(":memory:")
	try:
		for path in sorted(f for f in repo.files if re.fullmatch(r"migrations/[^/]+\.up\.sql", f)):
			db.executescript(repo.read(path))
		names = {row[0].lower() for row in db.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
	except sqlite3.Error as err:
		return f"{path}: {err}"
	finally:
		db.close()
	return {t for t in names if not NOT_A_STORE.match(t)}


def check_stores(stores: list[str] | None, repo: Repo, report: Report) -> None:
	if stores is None:
		report.warn("No stores list", "the ledger has no `stores` list, so no table was checked against it")
		return
	listed = {s[len("server:") :].lower() for s in stores if s.startswith("server:")}
	standing = standing_tables(repo)
	# The rule is the lexical reading; SQLite applying the same files checks it,
	# so a statement shape the reader misses fails instead of passing silently.
	applied = applied_tables(repo)
	if isinstance(applied, str):
		report.warn("Migrations not applied", f"SQLite could not apply the migrations ({applied}), so the reading of their tables is not cross-checked")
	elif applied != standing:
		report.error("Migrations read wrongly", f"SQLite leaves {sorted(applied - standing)} standing and the reader does not; the reader leaves {sorted(standing - applied)} and SQLite does not: fix sql_code or SQL_STATEMENT")
	for table in sorted(standing - listed):
		report.error("Table in no store", f"migrations leave table {table} standing, and the ledger's `stores` has no server:{table}: add it in some-ui's ledger, with how it grows")
	for table in sorted(listed - standing):
		report.error("Store names no table", f"the ledger's `stores` lists server:{table}, and no migration here leaves that table standing")
	report.notes.append(f"{len(standing)} tables the migrations leave standing, {len(listed)} server stores listed")


# --- running it --------------------------------------------------------------------------


def check(data: Any, repo: Repo) -> Report:
	report = Report()
	try:
		families = read_ledger(data)
		stores = read_stores(data)
	except LedgerShape as err:
		report.error("Ledger unreadable", str(err))
		return report
	proofs = check_proofs_and_paths(families, repo, report)
	report.notes.insert(0, f"{len(families)} families, {proofs} server proofs of a holds checked")
	check_tasks(families, repo, report)
	check_stores(stores, repo, report)
	return report


def tracked_repo(root: Path) -> Repo:
	listed = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], check=True, capture_output=True, text=True).stdout
	return Repo(frozenset(p for p in listed.split("\0") if p), lambda path: (root / path).read_text(encoding="utf-8", errors="replace"))


def find_ledger(some_ui: Path) -> Path:
	listed = subprocess.run(["git", "-C", str(some_ui), "ls-files", "-z", "--", "*ledger.json"], check=True, capture_output=True, text=True).stdout
	matches = []
	for path in (p for p in listed.split("\0") if Path(p).name == "ledger.json"):
		try:
			schema = json.loads((some_ui / path).read_text(encoding="utf-8")).get("schema", "")
		except (ValueError, AttributeError):
			continue
		if isinstance(schema, str) and schema.startswith("some-ui.assurance-ledger/"):
			matches.append(path)
	if len(matches) != 1:
		raise LookupError(f"expected one assurance ledger (ledger.json with an assurance-ledger schema) in some-ui, found {len(matches)}: {matches}")
	return some_ui / matches[0]


def emit(report: Report, where: str) -> int:
	for note in report.notes:
		print(note)
	for title, message in report.warnings:
		print(f"::warning title={title}::{message}")
	for title, message in report.errors:
		print(f"::error title={title}::{message}")
	if report.errors:
		print(f"{len(report.errors)} finding(s) against {where}. See docs/assurance.md.")
		return 1
	print(f"the server's half of {where} holds ({len(report.warnings)} warning(s))")
	return 0


# --- self-test -------------------------------------------------------------------------------


def fake_repo(files: dict[str, str]) -> Repo:
	return Repo(frozenset(files), lambda path: files[path])


def objective(oid: str, verdict: str = "holds", proof: list[str] | None = None, refs: list[str] | None = None, watch: str | None = None, **extra: Any) -> dict[str, Any]:
	o: dict[str, Any] = {"id": oid, "verdict": verdict, "evidence": [{"class": "structural", "says": "x", **({"ref": r} if r else {})} for r in (refs or [None])], "watch": {"by": "none", **({"ref": watch} if watch else {})}}
	if proof is not None:
		o["proof"] = proof
	o.update(extra)
	return o


STORES = [{"store": f"server:{t}", "class": "record", "growth": "x", "family": "f"} for t in ("account", "notes", "game_body")] + [{"store": "mailbox:item", "family": "f"}]


def ledger(*families: dict[str, Any], **extra: Any) -> dict[str, Any]:
	return {"schema": SCHEMA, "families": list(families), "synchronous": [], "stores": STORES, **extra}


def fam(fid: str, paths: list[str], *objectives: dict[str, Any], **extra: Any) -> dict[str, Any]:
	return {"id": fid, "name": fid, "paths": paths, "objectives": list(objectives), **extra}


SRC = TASK_ROOT
BASE_FILES = {
	f"{SRC}/main.rs": "fn main() {\n\tnudge::waker::spawn(&s);\n\tfile_host::storage::spawn(&s);\n\ttokio::spawn(async move { shutdown.await; });\n}\n",
	f"{SRC}/websocket.rs": "fn drop() { runtime.spawn(async move {}); }\nfn f() { spawn_event_forwarder(a); }\n",
	f"{SRC}/websocket/broadcast/handlers.rs": "fn a() { tokio::spawn(async {}); }\nfn b() { tokio::spawn(async {}); }\n",
	f"{SRC}/websocket/message/handlers.rs": "fn a() { tokio::spawn(async {}) }\n#[cfg(test)]\nmod tests {\n\tfn t() { tokio::spawn(async {}); }\n}\n",
	f"{SRC}/mailbox.rs": "fn spawn(s: &S) { tokio::spawn(async move { pass().await }); }\nfn start() { spawn(s); }\n",
	f"{SRC}/mailbox/pass.rs": "/// `tokio::spawn(` in a doc comment\nfn every_failed_phase_is_reported() {}\n",
	f"{SRC}/storage.rs": "fn spawn() { tokio::spawn(async {}); }\n",
	f"{SRC}/metrics/periodic.rs": "fn spawn() { tokio::spawn(async {}); }\n",
	f"{SRC}/transcribe/engine.rs": 'fn f() { let s = "tokio::spawn("; let c = \'{\'; let child = ffmpeg().spawn(); }\n',
	f"{SRC}/lib.rs": "#[cfg(test)]\nmod privacy;\npub mod mailbox;\n",
	f"{SRC}/privacy.rs": "fn t() { std::thread::spawn(|| {}); }\n",
	f"{SRC}/bin/dump.rs": "fn main() { tokio::spawn(async {}); }\n",
	f"{SRC}/handlers/auth.rs": "fn h() {}\n#[cfg(test)]\nmod tests {\n\t#[tokio::test(flavor = \"multi_thread\")]\n\tasync fn t() { let x = tokio::spawn(async { '{' }); }\n}\n",
	".github/workflows/lint.yml": "",
	"docs/study-nudge.md": "",
	"migrations/0001_init.up.sql": "-- CREATE TABLE ghost (a);\nCREATE TABLE IF NOT EXISTS \"Account\" (id TEXT DEFAULT '-- DROP TABLE account');\n/* DROP TABLE account; */\nCREATE TABLE notes_old (a);\nCREATE TABLE _sqlx_migrations (v);\n",
	"migrations/0001_init.down.sql": "DROP TABLE account;\n",
	"migrations/0002_rename.up.sql": "ALTER TABLE notes_old RENAME TO notes;\nCREATE TABLE game_response (a);\n",
	"migrations/0003_rebuild.up.sql": "CREATE TABLE game_body (a);\nCREATE TEMP TABLE game_check (a);\nDROP TABLE game_check;\nCREATE TEMP TABLE scratch AS SELECT 1;\nDROP TABLE game_response;\n",
}
GOOD_FAMILIES = [
	fam("routine-to-home", ["some-ui:apps/mailbox", f"server:{SRC}/mailbox"], objective("MI1", proof=[f"server:{SRC}/mailbox/pass.rs#every_failed_phase_is_reported"], refs=[f"server:{SRC}/mailbox/pass.rs", "server#523", None])),
	fam("voice", [f"server:{SRC}/transcribe"], objective("VH1", "broken", tracked="server#1", refs=[f"server:{SRC}/transcribe/engine.rs"])),
	fam("checks", ["server:.github/workflows/"], objective("GC1", proof=["some-ui:packages/x.ts#readBranchRules", "server:docs/study-nudge.md#xx"], verdict="unproven"), objective("GC9", watch="server:docs/study-nudge.md#Scaling")),
]


def run(data: Any, files: dict[str, str] | None = None) -> Report:
	return check(data, fake_repo(BASE_FILES if files is None else files))


def titles(report: Report) -> list[str]:
	return sorted(t for t, _ in report.errors)


def self_test() -> int:
	failures: list[str] = []

	def expect(name: str, report: Report, errors: list[str], warnings: int | None = None) -> None:
		if titles(report) != sorted(errors):
			failures.append(f"{name}: errors {report.errors}, expected titles {errors}")
		if warnings is not None and len(report.warnings) != warnings:
			failures.append(f"{name}: {len(report.warnings)} warnings, expected {warnings}: {report.warnings}")

	# The baseline: proofs and paths present, tasks placed, known and exempt.
	good = run(ledger(*GOOD_FAMILIES))
	expect("compliant ledger", good, [], warnings=2)
	spawning = {line.split(" ")[0] for line in good.notes if line.startswith(SRC)}
	if spawning != {f"{SRC}/{f}" for f in ("main.rs", "websocket.rs", "websocket/broadcast/handlers.rs", "websocket/message/handlers.rs", "mailbox.rs", "storage.rs", "metrics/periodic.rs", "transcribe/engine.rs")} - {f"{SRC}/storage.rs", f"{SRC}/metrics/periodic.rs"}:
		failures.append(f"the files read as spawning (placed or exempt) are {sorted(spawning)}")
	# Fields the rules do not read are ignored, never rejected.
	expect("unknown fields", run(ledger(*GOOD_FAMILIES, stores=[{**e, "later": [1]} for e in STORES], future={"a": 1})), [])
	expect("unknown field in an objective", run(ledger(fam("f", [f"server:{SRC}/mailbox"], objective("MI1", proof=[f"server:{SRC}/mailbox/pass.rs#every_failed"], newField=[1])), *GOOD_FAMILIES[1:])), [])

	# Rule 1: a holds' server proof.
	def with_proof(proof: str, **extra: Any) -> dict[str, Any]:
		return ledger(*GOOD_FAMILIES, fam("p", [], objective("PX1", proof=[proof], **extra)))

	expect("proof file missing", run(with_proof(f"server:{SRC}/mailbox/renamed.rs#every_failed")), ["Proof missing (PX1)"])
	expect("proof name missing", run(with_proof(f"server:{SRC}/mailbox/pass.rs#a_renamed_test")), ["Proof missing (PX1)"])
	expect("proof name too short", run(with_proof(f"server:{SRC}/mailbox/pass.rs#ev")), ["Proof unreadable (PX1)"])
	expect("proof is a directory", run(with_proof(f"server:{SRC}/mailbox#every_failed")), ["Proof missing (PX1)"])
	expect("retired proof not read", run(with_proof(f"server:{SRC}/gone.rs#every_failed", retired={"on": "x"})), [])
	expect("broken verdict's proof not read", run(ledger(*GOOD_FAMILIES, fam("p", [], objective("PX1", "broken", proof=[f"server:{SRC}/gone.rs#xyz"])))), [])
	expect("some-ui proof not read", run(with_proof("some-ui:nowhere.ts#xyz")), [])

	# Rule 2: paths.
	expect("family path missing", run(ledger(*GOOD_FAMILIES, fam("p", ["server:apps/gone"]))), ["Ledger path missing (p)"])
	expect("evidence path missing", run(ledger(*GOOD_FAMILIES, fam("p", [], objective("PX1", "broken", refs=["server:docs/gone.md"])))), ["Ledger path missing (PX1)"])
	expect("watch path missing", run(ledger(*GOOD_FAMILIES, fam("p", [], objective("PX1", "broken", watch="server:infra/gone.jsonnet")))), ["Ledger path missing (PX1)"])
	expect("retired path still read", run(ledger(*GOOD_FAMILIES, fam("p", [], objective("PX1", "broken", refs=["server:gone"], retired={"on": "x"})))), ["Ledger path missing (PX1)"])
	expect("prefix is not a directory", run(ledger(*GOOD_FAMILIES, fam("p", [f"server:{SRC}/mail"]))), ["Ledger path missing (p)"])

	# Ledger shape: only what the rules need, and a new schema fails.
	expect("schema moved", run({**ledger(*GOOD_FAMILIES), "schema": "some-ui.assurance-ledger/2"}), ["Ledger unreadable"])
	expect("paths not a list", run(ledger(fam("f", f"server:{SRC}/mailbox"))), ["Ledger unreadable"])  # type: ignore[arg-type]
	expect("evidence missing", run(ledger(fam("f", [], {"id": "X1", "verdict": "holds", "watch": {"by": "none"}}))), ["Ledger unreadable"])

	# Rule 4: tables and stores.
	def stores_less(*tables: str, more: tuple[str, ...] = ()) -> list[dict[str, Any]]:
		return [e for e in STORES if e["store"] not in {f"server:{t}" for t in tables}] + [{"store": s} for s in more]

	expect("table in no store", run(ledger(*GOOD_FAMILIES, stores=stores_less("notes"))), ["Table in no store"])
	expect("store names a dropped table", run(ledger(*GOOD_FAMILIES, stores=stores_less(more=("server:game_response",)))), ["Store names no table"])
	expect("store names a temp table", run(ledger(*GOOD_FAMILIES, stores=stores_less(more=("server:scratch",)))), ["Store names no table"])
	expect("store names a commented table", run(ledger(*GOOD_FAMILIES, stores=stores_less(more=("server:ghost",)))), ["Store names no table"])
	expect("store names a renamed table's old name", run(ledger(*GOOD_FAMILIES, stores=stores_less("notes", more=("server:notes_old",)))), ["Store names no table", "Table in no store"])
	expect("store for sqlx's own table", run(ledger(*GOOD_FAMILIES, stores=stores_less(more=("server:_sqlx_migrations",)))), ["Store names no table"])
	expect("a new migration's table", run(ledger(*GOOD_FAMILIES), {**BASE_FILES, "migrations/0004_more.up.sql": "create table `More` (a);"}), ["Table in no store"])
	expect("a later drop", run(ledger(*GOOD_FAMILIES), {**BASE_FILES, "migrations/0004_more.up.sql": "DROP TABLE IF EXISTS main.notes;"}), ["Store names no table"])
	expect("reader and SQLite disagree", run(ledger(*GOOD_FAMILIES), {**BASE_FILES, "migrations/0004_more.up.sql": 'CREATE TABLE "a""b" (x);'}), ["Migrations read wrongly", "Table in no store"])
	expect("SQLite cannot apply them", run(ledger(*GOOD_FAMILIES), {**BASE_FILES, "migrations/0004_more.up.sql": "NOT SQL AT ALL;"}), [], warnings=3)
	expect("no stores list warns", run({k: v for k, v in ledger(*GOOD_FAMILIES).items() if k != "stores"}), [], warnings=3)
	expect("stores not a list", run(ledger(*GOOD_FAMILIES, stores={"server:x": 1})), ["Ledger unreadable"])
	expect("store without a name", run(ledger(*GOOD_FAMILIES, stores=[{"class": "x"}])), ["Ledger unreadable"])

	# Rule 3: tasks.
	def files_with(**changes: str | None) -> dict[str, str]:
		out = dict(BASE_FILES)
		for name, text in changes.items():
			path = f"{SRC}/{name.replace('__', '/')}.rs"
			if text is None:
				out.pop(path, None)
			else:
				out[path] = text
		return out

	expect("new poller in no family", run(ledger(*GOOD_FAMILIES), files_with(routine_watch="fn spawn() { tokio::spawn(async {}); }\n")), ["Background task in no family"])
	expect("new thread in no family", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="fn f() { std::thread::spawn(|| {}); }\n")), ["Background task in no family"])
	expect("spawn_blocking in no family", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="fn f() { tokio::task::spawn_blocking(|| {}); }\n")), ["Background task in no family"])
	expect("JoinSet spawn in no family", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="fn f() { set . spawn (async {}); }\n")), ["Background task in no family"])
	expect("a second spawn in main.rs", run(ledger(*GOOD_FAMILIES), files_with(main="fn main() { tokio::spawn(a); tokio::spawn(poller()); }\n")), ["Spawn in a file listed as no background task"])
	expect("exempt file stops spawning", run(ledger(*GOOD_FAMILIES), files_with(websocket="fn drop() {}\n")), ["Stale NOT_BACKGROUND entry"])
	expect("placed task's family loses the path", run(ledger(fam("routine-to-home", ["server:crates/x"]), *GOOD_FAMILIES[1:]), {**BASE_FILES, "crates/x/a.rs": ""}), ["Background task in no family"])
	expect("bare spawn import", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="use tokio::spawn;\nfn f() { spawn(async {}); }\n")), ["Spawn this check cannot see"])
	expect("braced bare spawn import", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="use tokio::{select, spawn};\n")), ["Spawn this check cannot see"])
	expect("path import is fine", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="use tokio::task::JoinSet;\nuse std::thread;\n")), [])
	expect("test module after a raw string", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice='const Q: &str = r#"}{"#;\n#[cfg(test)]\nmod tests { fn t() { if x { tokio::spawn(a); } } }\n')), [])
	expect("code after a test module still read", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="#[cfg(test)]\nmod tests { fn t() {} }\nfn f() { tokio::spawn(a); }\n")), ["Background task in no family"])
	expect("lifetimes are not chars", run(ledger(*GOOD_FAMILIES), files_with(jobs_notice="fn f<'a>(x: &'a str) {}\nfn g() { tokio::spawn(a); }\n")), ["Background task in no family"])
	expect("a placed known task warns", run(ledger(*GOOD_FAMILIES, fam("what-is-kept", [f"server:{SRC}/storage.rs"]))), [], warnings=2)
	expect("child module of a file path", run(ledger(fam("p", [f"server:{SRC}/jobs_notice.rs"]), *GOOD_FAMILIES), files_with(jobs_notice__send="fn f() { tokio::spawn(a); }\n", jobs_notice="")), [])

	for name, path, file, want in (
		("same file", "a/b.rs", "a/b.rs", True),
		("under a directory", "a/b", "a/b/c.rs", True),
		("module root of a directory", "a/b", "a/b.rs", True),
		("trailing slash", "a/b/", "a/b/c/d.rs", True),
		("child of a file module", "a/b.rs", "a/b/c.rs", True),
		("a sibling with a longer name", "a/b", "a/bc.rs", False),
		("a parent", "a/b/c.rs", "a/b.rs", False),
	):
		if covers(path, file) != want:
			failures.append(f"covers({path!r}, {file!r}) is not {want} ({name})")
	if module_files(f"{SRC}/lib.rs", "privacy") != (f"{SRC}/privacy.rs", f"{SRC}/privacy/") or module_files(f"{SRC}/nudge.rs", "t") != (f"{SRC}/nudge/t.rs", f"{SRC}/nudge/t/"):
		failures.append("module_files resolves out-of-line modules wrongly")

	if failures:
		print("::error::check_assurance_ledger.py's own rule tests failed:")
		for failure in failures:
			print(f"  {failure}")
		return 1
	print("check_assurance_ledger.py rule tests passed")
	return 0


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	source = parser.add_mutually_exclusive_group()
	source.add_argument("--some-ui", type=Path, help="a checkout of paulgsc/some-ui; the ledger is found in it")
	source.add_argument("--ledger", type=Path, help="the ledger file itself")
	parser.add_argument("--self-test", action="store_true")
	args = parser.parse_args()
	if args.self_test:
		return self_test()
	try:
		path = args.ledger or (find_ledger(args.some_ui.resolve()) if args.some_ui else None)
	except LookupError as err:
		print(f"::error title=Ledger not found::{err}")
		return 1
	if path is None:
		parser.error("--some-ui or --ledger is required")
	data = json.loads(path.read_text(encoding="utf-8"))
	return emit(check(data, tracked_repo(REPO_ROOT)), f"the assurance ledger ({path.name})")


if __name__ == "__main__":
	sys.exit(main())
