#!/usr/bin/env python3
"""Erasures as a ratchet: no new silent erasure, and the old ones only go.

The owner's rule (epic #1174): a failure is like energy, never destroyed. A
`Result` turned into an ordinary value with no count and no error log is a
failure destroyed, and about 140 of them hid real ones (#1172: a refused
handing stored as an answer, then read as "waits"). The `Failure` token
(`apps/servers/file_host/src/failure.rs`, #1193) makes the right thing easy;
this makes the wrong thing visible where it is written:

  * `clippy.toml`'s `disallowed-methods` names `Result::ok`, `unwrap_or`,
    `unwrap_or_default` and `unwrap_or_else`, as calls and as paths
    (`filter_map(Result::ok)`, `filter_map(|r| r.ok())`);
  * `.cargo/config.toml` warns `clippy::let_underscore_must_use` (`let _ =`
    of anything `#[must_use]`: a `Result`, a `Failure`, and also iterators
    and futures) and `clippy::map_err_ignore` (`.map_err(|_| ...)`);
  * and, read here from the source with comments and literals masked: an `#[allow]` or
    `#[expect]` (or one inside `cfg_attr`) of any of those lints, or of a
    group holding one (`clippy::style`, `clippy::all`,
    `clippy::restriction`, `warnings`), that gives no non-empty `reason = "..."`.
    Clippy reports nothing in such a scope, so each erasure-shaped call in it
    (`.ok(`, `.unwrap_or*(`, `Result::ok`, `let _ =`, `.map_err(|_|`) is a
    finding here too, read lexically to the end of the statement, the item's
    braces, or the file for `#![...]`: a new one inside an old bare `#[allow]`
    is new. Such an attribute silences the
    erasures in its whole scope, and the 45 bare `#[allow(clippy::disallowed_methods)]`
    already here for other policies (`serde_json::to_*`, `std::env::var`,
    `Router::route`) would otherwise hide erasures from this check entirely.
    A suppression with a reason says why, and review reads it.

Most existing sites are benign (an `Option` default, an untrusted parse that
becomes a 4xx, a send to a closed socket), so these findings are recorded in
their own baseline, `scripts/erasure_baseline.json`, not in
`scripts/clippy_baseline.json`: the owner's decision on #1189, option (a). The
clippy baseline's rule (only a toolchain bump grows it) stays as written, and
`check_clippy_baseline.py` leaves these findings to this script. A
`disallowed_methods` message in a shape neither script recognises stops both
(`check_clippy_baseline.is_erasure`), so a reworded clippy cannot move
erasures into the baseline a toolchain bump may grow.

This baseline only shrinks. There is no `--allow-growth`: a new erasure is
either counted (`Failure::record`, or an `error!` that names it), or it is
benign and says so where it is written, in
`#[expect(clippy::disallowed_methods, reason = "...")]` (or the lint it is).
An `#[expect]` of `disallowed_methods` also silences that lint's other
policies in its scope, so keep it on the one statement. Three comparisons,
as the clippy ratchet makes them:

  * the findings vs the committed baseline: a new one fails, and so does a
    fixed one until `--update` records the fix;
  * the committed baseline vs the base branch's (`--base`): it may not record
    anything the base's does not. lint.yml fails, rather than skips, when the
    base runs this check and its baseline cannot be read;
  * `--update` refuses to record growth, once a baseline exists.

A finding's identity is the clippy ratchet's own (file, lint, enclosing item,
source text), counted, from the same `cargo clippy --message-format=json`
output lint.yml already makes. What it does not see, and review covers:
`map_err(|_e| ...)` (a named, unused binding), `is_ok()`/`is_err()` as the
only use, `map_or`/`map_or_else`, `if let Ok(..)` and `let Ok(x) = .. else`,
`Err(_) => default` arms, `.flatten()` over `Result`s, and `drop(result)`.
The suppression scan is lexical: string and char literals are read past, but
a raw string (`r#"..."#`) holding a `"` or a `/*` can still mislead it.

    python3 scripts/check_erasures.py clippy.json [--base base.json] [--update]
    python3 scripts/check_erasures.py --self-test
"""

from __future__ import annotations

import argparse
import io
import json
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_clippy_baseline as clippy  # noqa: E402

BASELINE = clippy.REPO_ROOT / "scripts" / "erasure_baseline.json"
SUPPRESSED = "erasure::suppressed_without_reason"
# The erasure lints, and the groups that hold them: `disallowed_methods` is in
# `clippy::style` (so `clippy::all`), the other two in `clippy::restriction`.
ERASURE_LINT = re.compile(r"\bclippy::(?:disallowed_methods|let_underscore_must_use|map_err_ignore|style|all|restriction)\b|\bwarnings\b")
ATTRIBUTE = re.compile(r"#!?\[(?P<inner>(?:[^\[\]]|\[[^\[\]]*\])*)\]", re.DOTALL)
SUPPRESSES = re.compile(r"\b(?:allow|expect)\s*\(")
# A reason that says something: a non-empty string.
REASON = re.compile(r'\breason\s*=\s*"[^"]*\S[^"]*"')
# Strings and char literals first, so a `/*` or `//` inside one is not a comment.
LITERAL_OR_COMMENT = re.compile(r"""(?P<lit>"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])')|//[^\n]*|/\*.*?\*/""", re.DOTALL)
SPACE = re.compile(r"\s*")


HIDDEN = "erasure::in_unexplained_scope"
# What each lint's erasure looks like in source, for the scopes clippy is told
# not to look in; a group, or `warnings`, hides all three.
ERASURE_SHAPES = {
	"disallowed_methods": r"\.(?:ok|unwrap_or|unwrap_or_default|unwrap_or_else)\s*\(|\bResult::ok\b",
	"let_underscore_must_use": r"\blet\s+_\s*=",
	"map_err_ignore": r"\.map_err\s*\(\s*\|_\|",
}
NAMED = re.compile(r"\bclippy::(disallowed_methods|let_underscore_must_use|map_err_ignore)\b")


def _hidden_shapes(inner: str) -> re.Pattern[str]:
	"""The erasures a suppression with these arguments hides."""
	names = set(NAMED.findall(inner))
	if not names or re.search(r"\bclippy::(?:style|all|restriction)\b|\bwarnings\b", inner):
		names = set(ERASURE_SHAPES)
	return re.compile("|".join(ERASURE_SHAPES[name] for name in sorted(names)))


def _mask(text: str) -> str:
	"""Comments as spaces, and each literal's contents as `x`s (its whitespace and quotes
	kept), newlines kept throughout: an attribute shown in a comment, a string or a
	`#[doc = "..."]` is no finding, and an empty `reason = ""` still reads as empty."""

	def masked(match: re.Match[str]) -> str:
		if not match.group("lit"):
			return re.sub(r"[^\n]", " ", match.group(0))
		quoted = match.group(0)
		return quoted[0] + re.sub(r"\S", "x", quoted[1:-1]) + quoted[-1]

	return LITERAL_OR_COMMENT.sub(masked, text)


def _scope_end(text: str, start: int, inner: bool) -> int:
	"""Where the scope a suppression covers ends: the file, for `#![...]`; else the
	statement (its `;`) or the item (its closing brace), whichever opens first."""
	if inner:
		return len(text)
	depth = 0
	for index in range(start, len(text)):
		char = text[index]
		if char == ";" and depth == 0:
			return index + 1
		if char == "{":
			depth += 1
		elif char == "}":
			depth -= 1
			if depth <= 0:
				return index + 1
	return len(text)


def suppressions(files: list[str]) -> Counter:
	"""Each `#[allow]`/`#[expect]` (or one inside `cfg_attr`) of an erasure lint, or a group
	holding one, that gives no reason, keyed as a finding; and each erasure in the scope
	such a suppression hides that no suppression with a reason covers."""
	found: Counter = Counter()
	for file in files:
		path = clippy.REPO_ROOT / file
		if not path.is_file():
			continue
		source = path.read_text(encoding="utf-8")
		text, lines = _mask(source), source.splitlines()
		# (attribute, where its scope starts and ends, what it hides, whether it says why)
		scopes = []
		for match in ATTRIBUTE.finditer(text):
			inner = match.group("inner")
			if not SUPPRESSES.search(inner) or not ERASURE_LINT.search(inner):
				continue
			# Past any attributes stacked under it, to what it decorates.
			end = SPACE.match(text, match.end()).end()
			while (stacked := ATTRIBUTE.match(text, end)) is not None:
				end = SPACE.match(text, stacked.end()).end()
			scope_end = _scope_end(text, end, match.group(0).startswith("#!"))
			scopes.append((match, end, scope_end, _hidden_shapes(inner), REASON.search(inner) is not None))
		explained = [(start, stop, shapes) for _, start, stop, shapes, reasoned in scopes if reasoned]
		for match, start, stop, shapes, reasoned in scopes:
			if reasoned:
				continue
			# Keyed to the enclosing item of the first line it decorates (the
			# item itself, or the statement inside a fn).
			after = text.count("\n", 0, start) + 1
			found[(file, SUPPRESSED, clippy.enclosing_item(file, after), clippy.WHITESPACE.sub(" ", match.group(0)).strip())] += 1
			# Clippy reports nothing in that scope, so each erasure in it is
			# read here, and a new one is new however old the attribute is,
			# unless a suppression that says why covers it.
			for call in shapes.finditer(text, start, stop):
				if any(begin <= call.start() < end and covers.fullmatch(call.group(0)) for begin, end, covers in explained):
					continue
				line = text.count("\n", 0, call.start()) + 1
				found[(file, HIDDEN, clippy.enclosing_item(file, line), clippy.WHITESPACE.sub(" ", lines[line - 1]).strip())] += 1
	return found


def rust_files() -> list[str]:
	listed = subprocess.run(["git", "ls-files", "-z", "--", "*.rs"], cwd=clippy.REPO_ROOT, capture_output=True, check=True)
	return sorted(name for name in listed.stdout.decode().split("\0") if name)


def read(stream, files: list[str]) -> tuple[Counter, list[str], bool]:
	findings, errors, succeeded = clippy.read(stream, keep=clippy.is_erasure)
	return findings + suppressions(files), errors, succeeded


def check(findings: Counter, committed: Counter, base: Counter | None) -> list[str]:
	"""Every way the findings break the ratchet, as lines to print; none if it holds."""
	problems: list[str] = []
	grew = clippy.exceeding(findings, committed)
	if grew:
		problems.append(
			"::error::a new silent erasure: count it with Failure::record (or log it at error), "
			'or say why it is benign in #[expect(<the lint>, reason = "...")]:'
		)
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


def main(argv: list[str] | None = None, files: list[str] | None = None) -> int:
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
		findings, errors, succeeded = read(stream, rust_files() if files is None else files)
	if errors or not succeeded:
		print("::error::clippy did not build the workspace; check_clippy_baseline.py says why")
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
		print(f"wrote {sum(findings.values())} erasures across {count} files to {BASELINE.relative_to(clippy.REPO_ROOT)}")
		return 0

	problems = check(findings, committed, clippy.load(args.base) if args.base is not None else None)
	if problems:
		print("\n".join(problems))
		return 1
	print(f"erasures: {sum(findings.values())}, all recorded in the baseline; nothing new")
	return 0


# --- self-test -------------------------------------------------------------


def _message(code: str, text: str, message: str, line: int, file: str = "apps/x/src/lib.rs") -> str:
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
OK = _message("clippy::disallowed_methods", "let n = s.parse().ok();", "use of a disallowed method `core::result::Result::ok`", 3)
OR = _message("clippy::disallowed_methods", "x.unwrap_or_default()", "use of a disallowed method `core::result::Result::unwrap_or_default`", 5)
LET = _message("clippy::let_underscore_must_use", "let _ = send();", "non-binding `let` on a result of a `#[must_use]` function", 7)
MAP = _message("clippy::map_err_ignore", ".map_err(|_| E)", "`map_err(|_|...` wildcard pattern discards the original error", 9)
ENV = _message("clippy::disallowed_methods", 'std::env::var("X")', "use of a disallowed method `std::env::var`", 11)
DOC = _message("clippy::doc_markdown", "/// a FooBar", "item in documentation is missing backticks", 1)
REWORDED = _message("clippy::disallowed_methods", "s.ok()", "`core::result::Result::ok` is disallowed", 3)

SOURCE = "\n".join(
	[
		"fn a() {}",
		"#[allow(clippy::disallowed_methods)]",
		"fn hidden() {}",
		'#[expect(clippy::disallowed_methods, reason = "a probe; its failure is the answer")]',
		"fn said() {}",
		"#[allow(clippy::too_many_lines)]",
		"fn other() {}",
		"#![allow(",
		"    clippy::map_err_ignore",
		")]",
		"fn b() {}",
		"/// #[allow(clippy::disallowed_methods)] in a comment is no finding",
		"#[cfg_attr(test, allow(clippy::disallowed_methods))]",
		"#[tokio::test]",
		"async fn stacked() {}",
		'#[allow(clippy::disallowed_methods, reason = "")]',
		"fn empty_reason() {}",
		"#[allow(clippy::all)]",
		"fn grouped() {}",
		'fn r() { route("/*path"); }',
		"#[allow(warnings)]",
		"#[cfg_attr(",
		"    test,",
		"    derive(Debug)",
		")]",
		"struct Warned;",
		"/* note */",
		'const SHOWN: &str = "#[allow(clippy::disallowed_methods)]";',
		'#[doc = "#[allow(clippy::disallowed_methods)] in a doc attribute"]',
		"fn documented() {}",
		"#[allow(clippy::disallowed_methods)]",
		"fn hides() {",
		"    let n = s.parse::<u8>().ok();",
		"    let _ = send();",
		'    #[expect(clippy::disallowed_methods, reason = "a probe; its failure is the answer")]',
		"    let said = s.parse::<u8>().ok();",
		"}",
		"fn outside() { let m = s.parse::<u8>().ok(); }",
		"fn c() {}",
	]
)


def _stream(*lines: str) -> io.StringIO:
	return io.StringIO("\n".join([*lines, FINISHED]) + "\n")


def _run(*argv: str) -> tuple[int, str]:
	out = io.StringIO()
	saved, sys.stdout = sys.stdout, out
	try:
		code = main(list(argv), files=["apps/x/src/lib.rs"])
	finally:
		sys.stdout = saved
	return code, out.getvalue()


def self_test() -> int:
	global BASELINE
	saved_root, saved_baseline = clippy.REPO_ROOT, BASELINE
	with tempfile.TemporaryDirectory() as tmp:
		root = Path(tmp)
		clippy.REPO_ROOT, BASELINE = root, root / "erasure_baseline.json"
		clippy._sources.clear()
		try:
			(root / "apps/x/src").mkdir(parents=True)
			(root / "apps/x/src/lib.rs").write_text(SOURCE + "\n", encoding="utf-8")
			files = ["apps/x/src/lib.rs"]
			everything = (OK, OR, LET, MAP, ENV, DOC)

			# Each ratchet reads only its own findings, the clippy one by default.
			found, errors, succeeded = read(_stream(*everything), files)
			assert not errors and succeeded
			lints = Counter(key[1] for key in found for _ in range(found[key]))
			assert lints == Counter({"clippy::disallowed_methods": 2, "clippy::let_underscore_must_use": 1, "clippy::map_err_ignore": 1, SUPPRESSED: 7, HIDDEN: 1}), lints
			rest, _, _ = clippy.read(_stream(*everything))
			assert sum(rest.values()) == 2, "the clippy ratchet keeps the env read and the doc lint, and only those"
			assert not set(rest) & set(found), "no finding is in both ratchets"

			# A suppression with no reason is a finding; one with a reason, or of another lint, is not.
			hidden = {key[2]: key[3] for key in found if key[1] == SUPPRESSED}
			scoped = sorted(key[3] for key in found if key[1] == HIDDEN)
			assert scoped == ["let n = s.parse::<u8>().ok();"], "the erasure the bare allow hides; not its `let _ =`, which clippy still sees, nor the one outside it: " + repr(scoped)
			assert set(hidden) == {"fn hidden", "fn b", "fn stacked", "fn empty_reason", "fn grouped", "struct Warned", "fn hides"}, hidden

			# A message clippy reworded stops both ratchets.
			for reader in (lambda s: clippy.read(s), lambda s: read(s, files)):
				try:
					reader(_stream(REWORDED))
				except clippy.UnknownMessage:
					pass
				else:
					raise AssertionError("a reworded disallowed_methods message was read")

			# The comparisons.
			fewer, _, _ = read(_stream(OK, OR, LET), files)
			assert check(found, found, found) == [], "the recorded set holds"
			assert any("a new silent erasure" in line for line in check(found, fewer, None)), "a new one fails"
			assert any("were fixed" in line for line in check(fewer, found, None)), "a fixed one fails until recorded"
			assert any("only shrinks" in line for line in check(found, found, fewer)), "the baseline may not grow past the base's"
			assert check(fewer, fewer, found) == [], "it may shrink"

			# main(), end to end, against the redirected baseline.
			messages = root / "clippy.json"
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
		finally:
			clippy.REPO_ROOT, BASELINE = saved_root, saved_baseline
			clippy._sources.clear()
	print("check_erasures self-test: ok")
	return 0


if __name__ == "__main__":
	sys.exit(main())
