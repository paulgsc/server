#!/usr/bin/env python3
"""Erasures as a ratchet: no new silent erasure of a `Result`, and the old ones only go.

The owner's rule (epic #1174): a failure is like energy, never destroyed. A
`Result` turned into an ordinary value with no count and no error log is a
failure destroyed, and about 140 of them hid real ones (#1172: a refused
handing stored as an answer, then read as "waits"). The `Failure` token
(`apps/servers/file_host/src/failure.rs`) makes the right thing easy; this
makes the wrong thing visible, at the place it is written:

  * `clippy.toml`'s `disallowed-methods` names `Result::ok`, `unwrap_or`,
    `unwrap_or_default` and `unwrap_or_else`;
  * `.cargo/config.toml` warns `clippy::let_underscore_must_use` (`let _ =`
    of a `Result` or a `Failure`) and `clippy::map_err_ignore`
    (`.map_err(|_| ...)`).

Most existing sites are benign (an `Option` default, an untrusted parse that
becomes a 4xx, a send to a closed socket), so these findings are recorded in
their own baseline, `scripts/erasure_baseline.json`, not in
`scripts/clippy_baseline.json`: the owner's decision on #1189, option (a). The
clippy baseline's rule (only a toolchain bump grows it) stays as written, and
`check_clippy_baseline.py` leaves these findings to this script.

This baseline only shrinks. There is no `--allow-growth`: a new erasure is
either counted (`Failure::record`, or an `error!` that names it), or it is
benign and says so where it is written, in
`#[expect(clippy::disallowed_methods, reason = "...")]` (or the lint it is).
Three comparisons, as the clippy ratchet makes them:

  * the findings vs the committed baseline: a new one fails, and so does a
    fixed one until `--update` records the fix;
  * the committed baseline vs the base branch's (`--base`): it may not record
    anything the base's does not;
  * `--update` refuses to record growth.

A finding's identity is the clippy ratchet's own (file, lint, enclosing item,
source text), counted, from the same `cargo clippy --message-format=json`
output lint.yml already makes. What it does not see: `filter_map(|r| r.ok())`
and `.flatten()` over `Result`s, and a `match` arm `Err(_) => default`.
`filter_map(Result::ok)` is seen, as a path. Review covers the rest.

    python3 scripts/check_erasures.py clippy.json [--base base.json] [--update]
    python3 scripts/check_erasures.py --self-test
"""

from __future__ import annotations

import argparse
import io
import json
import sys
import tempfile
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_clippy_baseline as clippy  # noqa: E402

BASELINE = clippy.REPO_ROOT / "scripts" / "erasure_baseline.json"


def read(stream) -> tuple[Counter, list[str], bool]:
	return clippy.read(stream, keep=clippy.is_erasure)


def check(findings: Counter, committed: Counter, base: Counter | None) -> list[str]:
	"""Every way the findings break the ratchet, as lines to print; none if it holds."""
	problems: list[str] = []
	grew = clippy.exceeding(findings, committed)
	if grew:
		problems.append("::error::a new silent erasure of a Result: count it with Failure::record, or say why it is benign in #[expect(..., reason)]:")
		problems.extend(clippy.rows(grew))
	stale = [(key, recorded, found) for key, found, recorded in clippy.exceeding(committed, findings)]
	if stale:
		problems.append("::error::erasures were fixed but the baseline still allows them: run")
		problems.append("  python3 scripts/check_erasures.py <messages.json> --update")
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
	parser.add_argument("messages", type=Path, nargs="?", help="cargo clippy --message-format=json output")
	parser.add_argument("--update", action="store_true", help="rewrite the baseline from these findings (refuses growth)")
	parser.add_argument("--base", type=Path, help="the base branch's erasure baseline; the committed one may not exceed it")
	parser.add_argument("--self-test", action="store_true", help="run the rule's own cases")
	args = parser.parse_args(argv)

	if args.self_test:
		return self_test()
	if args.messages is None:
		parser.error("the clippy messages are required")

	with args.messages.open(encoding="utf-8") as stream:
		findings, errors, succeeded = read(stream)
	if errors or not succeeded:
		print("::error::clippy did not build the workspace; check_clippy_baseline.py says why")
		return 1

	committed = clippy.load(BASELINE) if BASELINE.exists() else Counter()

	if args.update:
		grew = clippy.exceeding(findings, committed)
		# The first baseline records what is there (#1189); after it, the
		# base branch's copy is what CI's `--base` holds any later one to, so
		# deleting the file is no way to record growth either.
		if grew and BASELINE.exists():
			print("::error::refusing to record new erasures in the baseline: it only shrinks.")
			print("\n".join(clippy.rows(grew)))
			return 1
		BASELINE.write_text(json.dumps(clippy.to_json(findings), indent=2, sort_keys=True) + "\n", encoding="utf-8")
		files = len({file for file, *_ in findings})
		print(f"wrote {sum(findings.values())} erasures across {files} files to {BASELINE.relative_to(clippy.REPO_ROOT)}")
		return 0

	problems = check(findings, committed, clippy.load(args.base) if args.base is not None else None)
	if problems:
		print("\n".join(problems))
		return 1
	print(f"erasures: {sum(findings.values())}, all recorded in the baseline; nothing new")
	return 0


# --- self-test -------------------------------------------------------------


def _message(code: str, text: str, message: str, file: str = "apps/x/src/lib.rs", line: int = 3) -> str:
	return json.dumps(
		{
			"reason": "compiler-message",
			"message": {
				"code": {"code": code},
				"level": "warning",
				"message": message,
				"spans": [{"is_primary": True, "file_name": file, "line_start": line, "column_start": 1, "text": [{"text": text}]}],
			},
		}
	)


FINISHED = json.dumps({"reason": "build-finished", "success": True})
OK = _message("clippy::disallowed_methods", "let n = s.parse().ok();", "use of a disallowed method `core::result::Result::ok`")
OR = _message("clippy::disallowed_methods", "x.unwrap_or_default()", "use of a disallowed method `core::result::Result::unwrap_or_default`", line=5)
LET = _message("clippy::let_underscore_must_use", "let _ = send();", "non-binding `let` on a result of a `#[must_use]` function", line=7)
MAP = _message("clippy::map_err_ignore", ".map_err(|_| E)", "`map_err(|_|...` wildcard pattern discards the original error", line=9)
ENV = _message("clippy::disallowed_methods", "std::env::var(\"X\")", "use of a disallowed method `std::env::var`", line=11)
DOC = _message("clippy::doc_markdown", "/// a FooBar", "item in documentation is missing backticks", line=1)


def _read(*lines: str, keep=None) -> Counter:
	stream = io.StringIO("\n".join([*lines, FINISHED]) + "\n")
	findings, errors, succeeded = clippy.read(stream, keep=keep) if keep else read(stream)
	assert not errors and succeeded
	return findings


def self_test() -> int:
	with tempfile.TemporaryDirectory() as tmp:
		source = Path(tmp)
		clippy.REPO_ROOT, saved = source, clippy.REPO_ROOT
		clippy._sources.clear()
		try:
			(source / "apps/x/src").mkdir(parents=True)
			(source / "apps/x/src/lib.rs").write_text("\n".join(["fn a() {"] * 12) + "\n", encoding="utf-8")

			everything = (OK, OR, LET, MAP, ENV, DOC)
			erasures = _read(*everything)
			assert {key[1] for key in erasures} == {"clippy::disallowed_methods", "clippy::let_underscore_must_use", "clippy::map_err_ignore"}, erasures
			assert sum(erasures.values()) == 4, "the four erasures, not the env read or the doc lint"

			rest = _read(*everything, keep=clippy.not_erasure)
			assert sum(rest.values()) == 2, "the clippy ratchet keeps the env read and the doc lint, and only those"
			assert not set(rest) & set(erasures), "no finding is in both ratchets"

			assert check(erasures, erasures, erasures) == [], "the recorded set holds"
			fewer = _read(OK, OR, LET)
			assert any("a new silent erasure" in line for line in check(erasures, fewer, None)), "a new one fails"
			assert any("were fixed" in line for line in check(fewer, erasures, None)), "a fixed one fails until recorded"
			assert any("only shrinks" in line for line in check(erasures, erasures, fewer)), "the baseline may not grow past the base's"
			assert check(fewer, fewer, erasures) == [], "it may shrink"
		finally:
			clippy.REPO_ROOT = saved
			clippy._sources.clear()
	print("check_erasures self-test: ok")
	return 0


if __name__ == "__main__":
	sys.exit(main())
