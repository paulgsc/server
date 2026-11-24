#!/usr/bin/env python3
"""Erasures as a ratchet: no new silent erasure, and the old ones only go.

The owner's rule (epic #1174): a failure is like energy, never destroyed. A
`Result` turned into an ordinary value with no count and no error log is a
failure destroyed, and about 140 of them hid real ones (#1172: a refused
handing stored as an answer, then read as "waits"). The `Failure` token
(`apps/servers/file_host/src/failure.rs`, #1193) makes the right thing easy;
this makes the wrong thing visible where it is written. The erasure lints:

  * `clippy.toml`'s `disallowed-methods` names `Result::ok`, `unwrap_or`,
    `unwrap_or_default` and `unwrap_or_else`, in every form clippy resolves:
    a method call, an associated-function call (`Result::unwrap_or_default(x)`)
    and a function reference (`filter_map(Result::<u8, E>::ok)`);
  * `clippy::let_underscore_must_use` (`let _ =` of anything `#[must_use]`: a
    `Result`, a `Failure`, and also iterators and futures);
  * `clippy::map_err_ignore` (`.map_err(|_| ...)`).

No excuses (the owner, 2026-11-24, #1202): lint.yml runs a second clippy pass
with each erasure lint under `--force-warn` (`--clippy-args` prints its
flags), and a forced lint is reported through any `#[allow]` or `#[expect]`,
with or without a `reason`, by a lint group or `warnings`, on an item, a
statement or a whole module. Clippy reads the scopes, so nothing here does.
A new erasure is fixed (returned, or counted with `Failure::record`), or it
fails: a stated reason does not excuse one. `--probe` holds that against the
real clippy, on a scratch crate with this repo's `clippy.toml`: the
associated-function form, a reasoned `#[expect]` on an `if` with no `;`, a
function reference with generics, `#[allow(warnings)]`, `#![allow]` and
`cfg_attr` all report, and `std::env::var` under a bare allow does not.

Clippy names a forced lint's source in a note (`requested on the command line
with --force-warn ...`, once per lint per crate). Reading a run whose erasure
findings carry any other source (`on by default`, a `-W`), or that carries no
forced note at all, fails: a run without the flags would silently lose every
erasure under an allow, and with it the "fixed" findings this ratchet asks
to be recorded.

The other ratchet. `check_clippy_baseline.py` reads lint.yml's first, unforced
clippy pass and leaves every erasure finding to this script
(`check_clippy_baseline.is_erasure`); this script reads the forced pass and
keeps only those. So no finding is in both, and the forced pass's other
`disallowed_methods` findings (`std::env::var`, `Router::route`, ...) under
their bare allows never reach the clippy baseline. A `disallowed_methods`
message in a shape neither script recognises stops both, so a reworded clippy
cannot move erasures into the baseline a toolchain bump may grow.

Most existing sites are benign (an `Option` default, an untrusted parse that
becomes a 4xx, a send to a closed socket), so they are recorded in their own
baseline, `scripts/erasure_baseline.json`, not the clippy one (the owner's
decision on #1189, option (a)). It only shrinks, with no toolchain exception.
Three comparisons, as the clippy ratchet makes them:

  * the findings vs the committed baseline: a new one fails, and so does a
    fixed one until `--update` records the fix;
  * the committed baseline vs the base branch's (`--base`): it may not record
    anything the base's does not. lint.yml fails, rather than skips, when the
    base runs this check and its baseline cannot be read;
  * `--update` refuses to record growth, once a baseline exists.

A finding's identity is the clippy ratchet's own (file, lint, enclosing item,
source text), counted. What the lints do not see, and review covers:
`map_err(|_e| ...)` (a named, unused binding), `is_ok()`/`is_err()` as the
only use, `map_or`/`map_or_else`, `if let Ok(..)` and `let Ok(x) = .. else`,
`Err(_) => default` arms, `.flatten()` over `Result`s, and `drop(result)`.

    cargo clippy <lint.yml's flags> -- $(python3 scripts/check_erasures.py --clippy-args) > erasures.json
    python3 scripts/check_erasures.py erasures.json [--base base.json] [--update]
    python3 scripts/check_erasures.py --self-test
    python3 scripts/check_erasures.py --probe        # needs cargo with clippy
"""

from __future__ import annotations

import argparse
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_clippy_baseline as clippy  # noqa: E402

BASELINE = clippy.REPO_ROOT / "scripts" / "erasure_baseline.json"
ERASURE_LINTS = ("clippy::disallowed_methods", "clippy::let_underscore_must_use", "clippy::map_err_ignore")
# What follows `--` in the forced pass: lint.yml's own clippy flags, then each
# erasure lint forced through any allow.
CLIPPY_ARGS = ["--cap-lints=warn", "-A", "unknown-lints"] + [flag for lint in ERASURE_LINTS for flag in ("--force-warn", lint)]
# The note rustc adds (once per lint per crate) saying where a lint's level came from.
LEVEL_SOURCE = re.compile(r"on by default$|^requested on the command line with |implied by ")
FORCED = re.compile(r"^requested on the command line with `--force-warn ")


class NotForced(ValueError):
	"""The clippy output was not made with the erasure lints forced."""


def read(stream) -> tuple[Counter, list[str], bool]:
	"""The erasure findings of a forced clippy pass, hard compiler errors, and whether the build
	finished. Raises `NotForced` if any erasure finding's level came from anywhere but
	`--force-warn`, or if no finding says it was forced."""
	sources: list[str] = []

	def keep(code: str, message: dict) -> bool:
		if not clippy.is_erasure(code, message):
			return False
		sources.extend(child.get("message", "") for child in message.get("children", []) if LEVEL_SOURCE.search(child.get("message", "")))
		return True

	findings, errors, succeeded = clippy.read(stream, keep=keep)
	if errors or not succeeded:
		return findings, errors, succeeded
	unforced = sorted({source for source in sources if not FORCED.match(source)})
	if unforced or not sources:
		raise NotForced(
			"the erasure lints were not forced in this clippy run ("
			+ ("; ".join(unforced) if unforced else "no finding says --force-warn")
			+ "): pass `-- $(python3 scripts/check_erasures.py --clippy-args)`"
		)
	return findings, errors, succeeded


def check(findings: Counter, committed: Counter, base: Counter | None) -> list[str]:
	"""Every way the findings break the ratchet, as lines to print; none if it holds."""
	problems: list[str] = []
	grew = clippy.exceeding(findings, committed)
	if grew:
		problems.append(
			"::error::a new silent erasure: return the error, or count it with Failure::record "
			"(#1174). No #[allow] or #[expect] hides one, with a reason or without (#1202):"
		)
		problems.extend(clippy.rows(grew))
	stale = [(key, recorded, found) for key, found, recorded in clippy.exceeding(committed, findings)]
	if stale:
		problems.append("::error::erasures were fixed but the baseline still allows them: run")
		problems.append("  python3 scripts/check_erasures.py <forced clippy messages.json> --update")
		problems.append("and commit scripts/erasure_baseline.json so the fix can't be undone silently:")
		problems.extend(clippy.rows(stale))
	if base is not None:
		recorded = clippy.exceeding(committed, base)
		if recorded:
			problems.append("::error::the committed erasure baseline records erasures the base branch's does not: it only shrinks:")
			problems.extend(clippy.rows(recorded))
	return problems


def main(argv: list[str] | None = None) -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	parser.add_argument("messages", type=Path, nargs="?", help="the forced pass's cargo clippy --message-format=json output")
	parser.add_argument("--update", action="store_true", help="rewrite the baseline from these findings (refuses growth)")
	parser.add_argument("--base", type=Path, help="the base branch's erasure baseline; the committed one may not exceed it")
	parser.add_argument("--self-test", action="store_true", help="run the rule's own cases")
	parser.add_argument("--probe", action="store_true", help="run the real clippy on a scratch crate and check what it reports")
	parser.add_argument("--clippy-args", action="store_true", help="print the flags the forced pass passes after `--`")
	args = parser.parse_args(argv)

	if args.clippy_args:
		print(" ".join(CLIPPY_ARGS))
		return 0
	if args.self_test:
		return self_test()
	if args.probe:
		return probe()
	if args.messages is None:
		parser.error("the clippy messages are required")

	with args.messages.open(encoding="utf-8") as stream:
		try:
			findings, errors, succeeded = read(stream)
		except NotForced as error:
			print(f"::error::{error}")
			return 1
	if errors or not succeeded:
		print("::error::clippy did not build the workspace:")
		print("\n".join(errors[:20]) or "  the build did not finish")
		return 1

	committed = clippy.load(BASELINE) if BASELINE.exists() else Counter()

	if args.update:
		grew = clippy.exceeding(findings, committed)
		# The first baseline records what is there (#1189). After it, `--update`
		# refuses growth, and CI's `--base` holds the committed file to the
		# base branch's, so deleting it is no way to record growth either.
		if grew and BASELINE.exists():
			print("::error::refusing to record new erasures in the baseline: it only shrinks.")
			print("\n".join(clippy.rows(grew)))
			return 1
		BASELINE.write_text(json.dumps(clippy.to_json(findings), indent=2, sort_keys=True) + "\n", encoding="utf-8")
		count = len({file for file, *_ in findings})
		print(f"wrote {sum(findings.values())} erasures across {count} files to {os.path.relpath(BASELINE, clippy.REPO_ROOT)}")
		return 0

	problems = check(findings, committed, clippy.load(args.base) if args.base is not None else None)
	if problems:
		print("\n".join(problems))
		return 1
	print(f"erasures: {sum(findings.values())}, all recorded in the baseline; nothing new")
	return 0


# --- probe: the real clippy, on the cases a lexical reading missed ----------

PROBE_SOURCE = """\
#![allow(dead_code)]
#[derive(Debug)]
pub struct E;
pub fn send() -> Result<(), E> { Err(E) }

// The associated-function form, inside a bare allow (#1194, Codex round 4).
#[allow(clippy::disallowed_methods)]
pub fn call_form(s: &str) -> u8 {
    Result::unwrap_or_default(s.parse::<u8>())
}

// A reasoned expect on an `if` with no `;`, and the erasure after it (#1194, round 4).
#[allow(clippy::disallowed_methods)]
pub fn if_without_semicolon(s: &str) -> Option<u8> {
    #[expect(clippy::disallowed_methods, reason = "a probe")]
    if !s.is_empty() {
        let _inside_if = s.parse::<u8>().ok();
    }
    let after_if = s.parse::<u8>().ok();
    after_if
}

// A function reference with generic arguments, under a group.
#[allow(clippy::all)]
pub fn reference(v: Vec<Result<u8, E>>) -> Vec<u8> {
    v.into_iter().filter_map(Result::<u8, E>::ok).collect()
}

#[allow(warnings)]
pub fn warnings_allowed() -> u8 {
    let _ = send();
    let mapped = send().map_err(|_| 1u8);
    "1".parse::<u8>().unwrap_or_else(|_| 0) + u8::from(mapped.is_ok())
}

#[cfg_attr(all(), allow(clippy::disallowed_methods))]
pub fn cfg_attr_allowed(s: &str) -> u8 {
    s.parse::<u8>().unwrap_or(7)
}

pub mod inner {
    #![allow(clippy::disallowed_methods, reason = "a module-wide reason excuses nothing")]
    pub fn module_allowed(s: &str) -> Option<u8> {
        s.parse::<u8>().ok()
    }
}

// Another disallowed_methods policy under a bare allow: not this ratchet's.
#[allow(clippy::disallowed_methods)]
pub fn other_policy() -> bool {
    std::env::var_os("X").is_some()
}
"""

# Each line the probe must report, with its lint: and nothing else.
PROBE_EXPECTED = Counter(
	{
		("clippy::disallowed_methods", "Result::unwrap_or_default(s.parse::<u8>())"): 1,
		("clippy::disallowed_methods", "let _inside_if = s.parse::<u8>().ok();"): 1,
		("clippy::disallowed_methods", "let after_if = s.parse::<u8>().ok();"): 1,
		("clippy::disallowed_methods", "v.into_iter().filter_map(Result::<u8, E>::ok).collect()"): 1,
		("clippy::let_underscore_must_use", "let _ = send();"): 1,
		("clippy::map_err_ignore", "let mapped = send().map_err(|_| 1u8);"): 1,
		("clippy::disallowed_methods", '"1".parse::<u8>().unwrap_or_else(|_| 0) + u8::from(mapped.is_ok())'): 1,
		("clippy::disallowed_methods", "s.parse::<u8>().unwrap_or(7)"): 1,
		("clippy::disallowed_methods", "s.parse::<u8>().ok()"): 1,
	}
)


def probe() -> int:
	saved_root = clippy.REPO_ROOT
	with tempfile.TemporaryDirectory() as tmp:
		crate = Path(tmp)
		shutil.copy(saved_root / "clippy.toml", crate / "clippy.toml")
		(crate / "Cargo.toml").write_text('[package]\nname = "erasure-probe"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n\n[workspace]\n', encoding="utf-8")
		(crate / "src").mkdir()
		(crate / "src/lib.rs").write_text(PROBE_SOURCE, encoding="utf-8")
		env = {key: value for key, value in os.environ.items() if key not in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS")}
		run = subprocess.run(
			["cargo", "clippy", "--quiet", "--message-format=json", "--", *CLIPPY_ARGS],
			cwd=crate,
			env={**env, "CARGO_TARGET_DIR": str(crate / "target")},
			capture_output=True,
			text=True,
			check=False,
		)
		clippy.REPO_ROOT = crate.resolve()
		clippy._sources.clear()
		try:
			findings, errors, succeeded = read(io.StringIO(run.stdout))
		except NotForced as error:
			print(f"::error::the erasure probe: {error}")
			return 1
		finally:
			clippy.REPO_ROOT = saved_root
			clippy._sources.clear()
	if errors or not succeeded:
		print("::error::the erasure probe did not build:\n" + "\n".join(errors) + run.stderr[-2000:])
		return 1
	found = Counter({(code, text): count for (_, code, _, text), count in findings.items()})
	if found != PROBE_EXPECTED:
		print("::error::clippy under --force-warn did not report exactly the probe's erasures:")
		for key in sorted(set(found) | set(PROBE_EXPECTED)):
			if found[key] != PROBE_EXPECTED[key]:
				print(f"  {key[0]}: expected {PROBE_EXPECTED[key]}, found {found[key]}: {key[1]}")
		return 1
	print(f"check_erasures probe: ok ({sum(found.values())} erasures, each through its allow)")
	return 0


# --- self-test -------------------------------------------------------------

FORCED_NOTE = "requested on the command line with `--force-warn clippy::disallowed-methods`"


def _message(code: str, text: str, message: str, line: int, note: str | None = None, file: str = "apps/x/src/lib.rs") -> str:
	return json.dumps(
		{
			"reason": "compiler-message",
			"message": {
				"code": {"code": code},
				"level": "warning",
				"message": message,
				"children": [{"message": note, "level": "note"}] if note else [],
				"spans": [{"is_primary": True, "file_name": file, "line_start": line, "column_start": 1, "text": [{"text": text}]}],
			},
		}
	)


FINISHED = json.dumps({"reason": "build-finished", "success": True})
OK = _message("clippy::disallowed_methods", "let n = s.parse().ok();", "use of a disallowed method `core::result::Result::ok`", 3, FORCED_NOTE)
OR = _message("clippy::disallowed_methods", "x.unwrap_or_default()", "use of a disallowed method `core::result::Result::unwrap_or_default`", 5)
LET = _message("clippy::let_underscore_must_use", "let _ = send();", "non-binding `let` on a result of a `#[must_use]` function", 7, "requested on the command line with `--force-warn clippy::let-underscore-must-use`")
MAP = _message("clippy::map_err_ignore", ".map_err(|_| E)", "`map_err(|_|...` wildcard pattern discards the original error", 9)
ENV = _message("clippy::disallowed_methods", 'std::env::var("X")', "use of a disallowed method `std::env::var`", 11, "`#[warn(clippy::disallowed_methods)]` on by default")
DOC = _message("clippy::doc_markdown", "/// a FooBar", "item in documentation is missing backticks", 1, "`-W clippy::doc-markdown` implied by `-W clippy::pedantic`")
REWORDED = _message("clippy::disallowed_methods", "s.ok()", "`core::result::Result::ok` is disallowed", 3, FORCED_NOTE)
DEFAULT = _message("clippy::disallowed_methods", "s.ok()", "use of a disallowed method `core::result::Result::ok`", 3, "`#[warn(clippy::disallowed_methods)]` on by default")
WARNED = _message("clippy::let_underscore_must_use", "let _ = send();", "non-binding `let` on a result of a `#[must_use]` function", 7, "requested on the command line with `-W clippy::let-underscore-must-use`")


def _stream(*lines: str) -> io.StringIO:
	return io.StringIO("\n".join([*lines, FINISHED]) + "\n")


def _run(*argv: str) -> tuple[int, str]:
	out = io.StringIO()
	saved, sys.stdout = sys.stdout, out
	try:
		code = main(list(argv))
	finally:
		sys.stdout = saved
	return code, out.getvalue()


def _not_forced(*lines: str) -> bool:
	try:
		read(_stream(*lines))
	except NotForced:
		return True
	return False


def self_test() -> int:
	global BASELINE
	saved_baseline = BASELINE
	with tempfile.TemporaryDirectory() as tmp:
		root = Path(tmp)
		BASELINE = root / "erasure_baseline.json"
		try:
			everything = (OK, OR, LET, MAP, ENV, DOC)

			# Each ratchet reads only its own findings.
			found, errors, succeeded = read(_stream(*everything))
			assert not errors and succeeded
			lints = Counter(key[1] for key in found for _ in range(found[key]))
			assert lints == Counter({"clippy::disallowed_methods": 2, "clippy::let_underscore_must_use": 1, "clippy::map_err_ignore": 1}), lints
			rest, _, _ = clippy.read(_stream(*everything))
			assert sum(rest.values()) == 2, "the clippy ratchet keeps the env read and the doc lint, and only those"
			assert not set(rest) & set(found), "no finding is in both ratchets"

			# Only a forced run is read: an erasure from a default or `-W` level, or no forced note at all, fails.
			assert _not_forced(OK, DEFAULT), "an erasure whose level is the default"
			assert _not_forced(OK, WARNED), "an erasure whose level is a plain -W"
			assert _not_forced(OR, MAP), "a run with no note saying it was forced"
			assert not _not_forced(OK, OR, ENV, DOC), "other lints' default levels are not this ratchet's"
			assert "--force-warn clippy::disallowed_methods" in " ".join(CLIPPY_ARGS)
			messages = root / "clippy.json"
			messages.write_text(_stream(OK, DEFAULT).getvalue(), encoding="utf-8")
			code, out = _run(str(messages))
			assert code == 1 and "not forced" in out, out

			# A message clippy reworded stops both ratchets.
			for reader in (clippy.read, read):
				try:
					reader(_stream(REWORDED))
				except clippy.UnknownMessage:
					pass
				else:
					raise AssertionError("a reworded disallowed_methods message was read")

			# The comparisons.
			fewer, _, _ = read(_stream(OK, OR, LET))
			assert check(found, found, found) == [], "the recorded set holds"
			assert any("a new silent erasure" in line for line in check(found, fewer, None)), "a new one fails"
			assert any("were fixed" in line for line in check(fewer, found, None)), "a fixed one fails until recorded"
			assert any("only shrinks" in line for line in check(found, found, fewer)), "the baseline may not grow past the base's"
			assert check(fewer, fewer, found) == [], "it may shrink"

			# main(), end to end, against the redirected baseline.
			messages.write_text(_stream(OK, OR, LET).getvalue(), encoding="utf-8")
			assert _run(str(messages), "--update")[0] == 0 and BASELINE.exists(), "the first --update records what is there"
			assert _run(str(messages))[0] == 0, "and then holds"
			messages.write_text(_stream(*everything).getvalue(), encoding="utf-8")
			code, out = _run(str(messages), "--update")
			assert code == 1 and "refusing" in out, "--update refuses growth once a baseline exists"
			code, out = _run(str(messages))
			assert code == 1 and "a new silent erasure" in out, out
			base = root / "base.json"
			base.write_text(json.dumps({}), encoding="utf-8")
			messages.write_text(_stream(OK, OR, LET).getvalue(), encoding="utf-8")
			code, out = _run(str(messages), "--base", str(base))
			assert code == 1 and "only shrinks" in out, "a baseline past an empty base's fails"
			messages.write_text("", encoding="utf-8")
			code, out = _run(str(messages))
			assert code == 1 and "did not build" in out, "an unfinished build fails"
		finally:
			BASELINE = saved_baseline
	print("check_erasures self-test: ok")
	return 0


if __name__ == "__main__":
	sys.exit(main())
