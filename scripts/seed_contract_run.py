#!/usr/bin/env python3
"""Seed the throwaway database `routes.yml`'s contract run signs in to.

The `Routes / Check some-ui against it` job runs some-ui's contract harness
against this PR's `file_host`, on a copy of the database the route snapshot
was built with. That database is migrated but empty, so this writes what the
per-person contracts need in it, the same rows the server would write:

- **The owner's account and a browser session.** A session is stored as the
  SHA-256 of the cookie's 32 bytes (`auth/cookie.rs`), so writing that row is
  signing in. That account is the only one, so it is the owner's and runs the
  operator routes with no OPERATOR_SUBJECTS (#510). No other account is ever
  written here: a second one would leave the box with no owner.
- **A paired phone's session for the same subject.** The rust_rounds routes
  answer a paired phone only (`subject::Paired`): the token must arrive as
  `Authorization: Bearer`, and a phone's session is stored under a
  domain-separated digest, `SHA-256(PHONE_DOMAIN || token)`
  (`SessionToken::hash` with `Carrier::Bearer`), so the browser's cookie
  cannot stand in for it. some-ui's harness sends it for its `paired: true`
  contracts and skips them without it.
- **A held LeetType session file** (#590). Only the `mailbox` poller writes
  `mailbox_item`, from the Jobs mailbox, so without this the
  `/leetype/session-files` list is empty and no file can be fetched. It holds
  `HELD_FILE` for the subject, a valid `some-ui.leetype-session/1`, so a
  contract can fetch it, check its body against `SessionFileSchema`, its
  `ETag` (`HELD_SHA256`) and the `304`; and `OTHER_FILE` for `OTHER_SUBJECT`,
  which has no account, so fetching that name as the subject is the
  cross-subject `404`. Every value is fixed, so a contract may name them
  literally; they are also exported (below).

The two credentials, the browser's cookie and the phone's token, are made
and masked by the workflow step and handed in as `CONTRACT_SESSION_COOKIE`
and `CONTRACT_PHONE_TOKEN` (base64url, as a client holds them). This only
stores their digests, so it never prints, logs or writes either one. What
it exports, to `$GITHUB_ENV` (or `--github-env`), for the harness step is
what is not a secret: `CONTRACT_LEETYPE_FILE`, `CONTRACT_LEETYPE_FILE_SHA256`
and `CONTRACT_LEETYPE_OTHER_FILE`.

`--self-test` seeds a database built from `migrations/*.up.sql` and checks
each row reads back the way the server reads it (the same SQL for the
listing, the digests `auth/cookie.rs` computes with its `PHONE_DOMAIN`, the
schema name `handlers/leetype_files.rs` accepts, both read from those files), so an edit here,
or there, that would leave a contract signed out or fetching nothing fails
before the harness runs.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import sqlite3
import sys
import tempfile
import time
from collections.abc import Callable
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MIGRATIONS = REPO_ROOT / "migrations"
COOKIE_SOURCE = REPO_ROOT / "apps" / "servers" / "file_host" / "src" / "auth" / "cookie.rs"
LEETYPE_HANDLER = REPO_ROOT / "apps" / "servers" / "file_host" / "src" / "handlers" / "leetype_files.rs"

# How long each seeded session lives: far longer than the job.
SESSION_SECONDS = 3600

# `auth::cookie::PHONE_DOMAIN`, prefixed to a phone's token before hashing;
# the self-test reads it there.
PHONE_DOMAIN = b"some-ui paired phone session\0"

# `handlers::leetype_files::SESSION_SCHEMA`; the self-test reads it there.
SESSION_SCHEMA = "some-ui.leetype-session/1"

# Names in the mailbox's own form (`t<thread>-<NN>-<slug>.json`), in a thread
# no routine writes.
HELD_FILE = "t900-01-contract-harness-held.json"
OTHER_FILE = "t900-02-contract-harness-other-subject.json"

# A subject id in the server's own form, with no account: only its file is
# written.
OTHER_SUBJECT = "subject-0000000000000000000000000000beef"

# When "home read" both files: a fixed instant, so every value is fixed.
PULLED_AT = "2026-10-31T00:00:00.000Z"


def session_file(title: str) -> dict[str, object]:
	"""A session file the phone can play: an info screen leading to the end."""
	return {
		"title": title,
		"oneLine": "Seeded by paulgsc/server's routes.yml for some-ui's contract harness.",
		"session": {"kind": "incident", "budgetMinutes": 5},
		"start": "intro",
		"screens": {
			"intro": {
				"type": "info",
				"title": title,
				"body": "Nothing to play: this file exists so a contract can fetch one.",
				"actions": [{"label": "Done", "next": "end"}],
			},
			"end": {"type": "end"},
		},
	}


def wrapped(file: dict[str, object]) -> bytes:
	"""A held mailbox item: the session file wrapped with its schema."""
	return json.dumps({"schema": SESSION_SCHEMA, "file": file}, separators=(",", ":")).encode()


HELD_BODY = wrapped(session_file("Contract harness: a held session file"))
OTHER_BODY = wrapped(session_file("Contract harness: another subject's file"))

# The item's version as the mailbox lists it, the lowercase hex SHA-256 of its
# body, which the server answers as the `ETag`.
HELD_SHA256 = hashlib.sha256(HELD_BODY).hexdigest()


def decoded(token: str) -> bytes:
	"""A 32-byte token from the base64url a client holds (`SessionToken::encoded`)."""
	raw = base64.urlsafe_b64decode(token + "=" * (-len(token) % 4))
	if len(raw) != 32:
		raise ValueError("a session token is 32 bytes")
	return raw


def browser_hash(token: bytes) -> bytes:
	"""What a browser's session is stored under (`Carrier::Cookie`)."""
	return hashlib.sha256(token).digest()


def phone_hash(token: bytes) -> bytes:
	"""What a paired phone's session is stored under (`Carrier::Bearer`)."""
	return hashlib.sha256(PHONE_DOMAIN + token).digest()


# What the harness step reads besides the two credentials: none is a secret.
EXPORTS = {
	"CONTRACT_LEETYPE_FILE": HELD_FILE,
	"CONTRACT_LEETYPE_FILE_SHA256": HELD_SHA256,
	"CONTRACT_LEETYPE_OTHER_FILE": OTHER_FILE,
}


def hold(db: sqlite3.Connection, subject: str, name: str, seq: int, body: bytes) -> None:
	# A plain INSERT: `mailbox_item` is append-only, and a seed that met a row
	# already there should fail, not replace it.
	db.execute(
		"INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?, 'leetype', ?, ?, ?, ?, ?, ?)",
		(subject, name, seq, hashlib.sha256(body).hexdigest(), len(body), body, PULLED_AT),
	)


def seed(db: sqlite3.Connection, subject: str, now: int, cookie: bytes, phone: bytes, random: Callable[[int], bytes] = os.urandom) -> None:
	"""Write every seeded row for `subject` in one transaction."""
	if subject == OTHER_SUBJECT:
		raise ValueError("the subject must not be OTHER_SUBJECT, whose file is the cross-subject 404")
	if cookie == phone:
		raise ValueError("the cookie and the phone's token must differ")
	expires_at = now + SESSION_SECONDS
	with db:
		db.execute("INSERT INTO account (subject_id, user_handle) VALUES (?, ?)", (subject, random(16)))
		db.execute(
			"INSERT INTO auth_session (token_hash, subject_id, expires_at) VALUES (?, ?, ?)",
			(browser_hash(cookie), subject, expires_at),
		)
		db.execute(
			"INSERT INTO auth_session (token_hash, subject_id, expires_at) VALUES (?, ?, ?)",
			(phone_hash(phone), subject, expires_at),
		)
		hold(db, subject, HELD_FILE, 1, HELD_BODY)
		hold(db, OTHER_SUBJECT, OTHER_FILE, 1, OTHER_BODY)


def export(github_env: Path) -> None:
	"""Append `EXPORTS` to `github_env`."""
	with github_env.open("a", encoding="utf-8") as env:
		for key, value in EXPORTS.items():
			env.write(key + "=" + value + "\n")


# --- self-test ---------------------------------------------------------------

# `mailbox_repo::leetype_files`, whose rule the list applies: what of each
# held file it shows.
LIST_SQL = """
SELECT name, sha256, pulled_at, json_valid(text) AS json,
  CASE WHEN json_valid(text) THEN
    CASE WHEN json_extract(text, '$.schema') = 'some-ui.leetype-session/1' AND json_type(text, '$.file.title') = 'text'
    THEN json_extract(text, '$.file.title') END
  END AS title,
  CASE WHEN json_valid(text) THEN
    CASE WHEN json_extract(text, '$.schema') = 'some-ui.leetype-session/1' AND json_type(text, '$.file.oneLine') = 'text'
    THEN json_extract(text, '$.file.oneLine') END
  END AS one_line
FROM (SELECT name, sha256, pulled_at, seq, CAST(body AS TEXT) AS text FROM mailbox_item WHERE subject_id = ? AND folder = 'leetype')
ORDER BY seq DESC
"""

# `mailbox_repo::leetype_file`: what one fetch reads.
FILE_SQL = "SELECT sha256, body FROM mailbox_item WHERE subject_id = ? AND folder = 'leetype' AND name = ?"

# The keys some-ui's `SessionFileSchema` (a strict object) allows.
FILE_KEYS = {"variant", "title", "oneLine", "session", "glossary", "start", "screens"}


def rust_bytes_literal(source: Path, name: str) -> bytes:
	"""The value of `const NAME: &[u8] = b"...";` in `source`."""
	found = re.search(r'const ' + name + r': &\[u8\] = b"((?:[^"\\]|\\.)*)";', source.read_text(encoding="utf-8"))
	if found is None:
		raise AssertionError(f"{source.relative_to(REPO_ROOT)} no longer declares {name}")
	return found.group(1).encode().decode("unicode_escape").encode("latin-1")


def rust_str_literal(source: Path, name: str) -> str:
	"""The value of `pub const NAME: &str = "...";` in `source`."""
	found = re.search(r'const ' + name + r': &str = "([^"\\]*)";', source.read_text(encoding="utf-8"))
	if found is None:
		raise AssertionError(f"{source.relative_to(REPO_ROOT)} no longer declares {name}")
	return found.group(1)


def migrated(path: Path) -> sqlite3.Connection:
	db = sqlite3.connect(path)
	for migration in sorted(MIGRATIONS.glob("*.up.sql")):
		db.executescript(migration.read_text(encoding="utf-8"))
	return db


def check_session_file(file: dict[str, object]) -> None:
	"""The wiring some-ui's `SessionFileSchema` checks, for what this seeds."""
	assert set(file) <= FILE_KEYS, set(file) - FILE_KEYS
	screens = file["screens"]
	assert isinstance(screens, dict) and file["start"] in screens
	for screen in screens.values():
		for action in screen.get("actions", []):
			assert action.get("next") is None or action["next"] in screens, action


def counter(start: int = 0) -> Callable[[int], bytes]:
	"""Distinct, fixed "random" bytes, so the self-test's tokens are known."""
	state = [start]

	def take(n: int) -> bytes:
		state[0] += 1
		return bytes([state[0] % 256]) * n

	return take


def self_test() -> None:
	assert rust_bytes_literal(COOKIE_SOURCE, "PHONE_DOMAIN") == PHONE_DOMAIN, "auth/cookie.rs hashes a phone's token under another domain"
	assert rust_str_literal(LEETYPE_HANDLER, "SESSION_SCHEMA") == SESSION_SCHEMA, "handlers/leetype_files.rs serves another schema"
	subject = "subject-00000000000000000000000000c0ffee"
	now = 1_800_000_000
	with tempfile.TemporaryDirectory() as tmp:
		db = migrated(Path(tmp) / "seed.db")
		# As the workflow hands them in: base64url, as a client holds them.
		cookie_text = base64.urlsafe_b64encode(bytes(range(32))).rstrip(b"=").decode()
		phone_text = base64.urlsafe_b64encode(bytes(range(32, 64))).rstrip(b"=").decode()
		cookie, phone = decoded(cookie_text), decoded(phone_text)
		seed(db, subject, now, cookie, phone, counter())

		# The only account is the subject's: the owner, and nobody else.
		assert db.execute("SELECT subject_id FROM account").fetchall() == [(subject,)]

		# The browser session opens with the cookie, as `live_session` reads it.
		live = "SELECT subject_id FROM auth_session WHERE token_hash = ? AND expires_at > ?"
		assert db.execute(live, (hashlib.sha256(cookie).digest(), now)).fetchall() == [(subject,)]

		# The phone's session opens with its token as a bearer, for the same
		# subject, and its token as a cookie opens nothing: what lets
		# `subject::Paired` tell a phone from a browser.
		assert db.execute(live, (hashlib.sha256(b"some-ui paired phone session\x00" + phone).digest(), now)).fetchall() == [(subject,)]
		assert db.execute(live, (hashlib.sha256(phone).digest(), now)).fetchall() == []
		assert db.execute(live, (phone_hash(cookie), now)).fetchall() == []

		# The list shows the subject's file, and only it, as the server would.
		listed = db.execute(LIST_SQL, (subject,)).fetchall()
		assert listed == [(HELD_FILE, HELD_SHA256, PULLED_AT, 1, "Contract harness: a held session file", session_file("")["oneLine"])], listed
		assert [row[0] for row in db.execute(LIST_SQL, (OTHER_SUBJECT,))] == [OTHER_FILE]

		# The fetch: the held body, whose `file` is what is served, under a
		# version that is a valid `ETag`; and nothing under the other
		# subject's name, which is the cross-subject 404.
		sha256, body = db.execute(FILE_SQL, (subject, HELD_FILE)).fetchone()
		assert sha256 == HELD_SHA256 == hashlib.sha256(body).hexdigest() and re.fullmatch(r"[0-9a-f]{64}", sha256)
		envelope = json.loads(body)
		assert envelope["schema"] == SESSION_SCHEMA
		check_session_file(envelope["file"])
		assert db.execute(FILE_SQL, (subject, OTHER_FILE)).fetchone() is None
		assert db.execute(FILE_SQL, (OTHER_SUBJECT, OTHER_FILE)).fetchone() is not None

		# The listing rule is the one that would catch a bad seed: another
		# schema shows no title.
		hold(db, subject, "t900-03-wrong-schema.json", 2, json.dumps({"schema": "x", "file": session_file("t")}).encode())
		assert db.execute(LIST_SQL, (subject,)).fetchone()[4] is None

		# A second seed meets the first's rows and fails, never replaces them.
		try:
			seed(db, subject, now, cookie, phone, counter(100))
		except sqlite3.IntegrityError:
			pass
		else:
			raise AssertionError("a second seed replaced the first")

		# A token that is not 32 bytes is refused before anything is written.
		try:
			decoded("c2hvcnQ")
		except ValueError:
			pass
		else:
			raise AssertionError("a short token was taken")

		# Exports: only what is not a secret, never a credential.
		env_file = Path(tmp) / "github_env"
		export(env_file)
		written = env_file.read_text(encoding="utf-8")
		assert written.splitlines() == [key + "=" + value for key, value in EXPORTS.items()]
		assert cookie_text not in written and phone_text not in written
		db.close()
	print("self-test passed")


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	parser.add_argument("--self-test", action="store_true", help="check the seed against a freshly migrated database, then exit")
	parser.add_argument("db", nargs="?", type=Path, help="the migrated SQLite database to seed")
	parser.add_argument("--subject", help="the signed-in subject")
	parser.add_argument("--github-env", type=Path, default=os.environ.get("GITHUB_ENV"), help="where to append the exports (default $GITHUB_ENV)")
	args = parser.parse_args()
	if args.self_test:
		self_test()
		return 0
	if args.db is None or args.subject is None:
		parser.error("give the database and --subject, or --self-test")
	if args.github_env is None:
		parser.error("no $GITHUB_ENV: pass --github-env")
	if not args.db.is_file():
		parser.error(f"{args.db} does not exist")
	tokens = [os.environ.get(name) for name in ("CONTRACT_SESSION_COOKIE", "CONTRACT_PHONE_TOKEN")]
	if None in tokens:
		parser.error("set CONTRACT_SESSION_COOKIE and CONTRACT_PHONE_TOKEN (the workflow step makes and masks them)")
	cookie, phone = (decoded(token) for token in tokens if token is not None)
	db = sqlite3.connect(args.db)
	seed(db, args.subject, int(time.time()), cookie, phone)
	db.close()
	export(args.github_env)
	print(f"seeded {args.subject} with a browser and a phone session and {HELD_FILE}, and {OTHER_SUBJECT} with {OTHER_FILE}")
	return 0


if __name__ == "__main__":
	sys.exit(main())
