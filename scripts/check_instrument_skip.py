#!/usr/bin/env python3
"""Every `#[instrument]` must say what it skips.

`#[tracing::instrument]` records every argument of the function it wraps as a
span field unless told otherwise. On a handler that is the request body, the
extracted state, the headers — log bloat at best, and at worst a field that
carries something this deployment has promised not to record. So the policy
is that an `#[instrument]` names its arguments explicitly: `skip(...)` for
the ones it drops, or `skip_all` and an explicit `fields(...)` list.

This replaces a grep step in lint.yml that had two blind spots, both silent:

  * it searched `crates/ src/`, and there is no root `src/` — every handler
    lives under `apps/`, so the directory the policy mattered most for was
    never scanned;
  * it read one line at a time, so an attribute written across several
    lines was judged by its first line alone (`#[instrument(` with the
    `skip_all` on the next line was reported, and one with no skip at all
    passed as long as its first line mentioned one elsewhere).

Here an attribute is read to its matching `]`, however many lines that is,
and `skip(...)`/`skip_all` must appear as a top-level argument outside any
string literal (`name = "skip"` and `fields(skip_all = true)` both record every
argument all the same). `--self-test` runs the
rule against known-compliant and known-noncompliant attributes, and runs in
lint.yml next to the real scan.

A second rule closes the gap the first leaves: `skip(state)` is "naming a
skip", and it still records `Json(payload)`. So a parameter that carries
request content (`Json`, `Form`, `Bytes`, `Multipart`, `Query`, `Path`) or the
request itself (`HeaderMap`, `Uri`, `OriginalUri`, `Request`, `Parts`, `CookieJar`,
`TypedHeader`, `Host`, `RawQuery`, `RawForm`: cookies, authorization, the full
path and query) must itself be skipped, with `skip_all` or by its binding. A value worth recording
is named in `fields(...)`, which is a choice somebody made rather than a
side effect. At the default `info` level those span fields reach the log and
the OpenTelemetry exporter alike (docs/identity.md, invariant 6).

Deliberately lexical, like scripts/check_metric_contract.py: a real parse
would need the whole workspace to build, which would make the cheapest check
in the repo the most expensive one to run.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_ROOTS = [REPO_ROOT / "apps", REPO_ROOT / "crates"]

ATTRIBUTE = re.compile(r"#\[\s*(?:tracing\s*::\s*)?instrument\b")
# `skip(...)` or `skip_all` as a *top-level* argument of the attribute. String
# literals are blanked first (so `name = "skip"` isn't one), and arguments
# nested inside another argument don't count (so `fields(skip_all = true)`,
# which tracing reads as a field name, isn't one either).
SKIP_DIRECTIVE = re.compile(r"^(?:skip_all|skip\s*\(.*\))$", re.DOTALL)
STRING = re.compile(r'"(?:\\.|[^"\\])*"')


def top_level_arguments(attribute: str) -> list[str]:
	"""`#[instrument(a, b(c, d), e)]` -> `["a", "b(c, d)", "e"]`."""
	arguments, current, depth = [], [], 0
	for char in STRING.sub('""', attribute):
		if char == "(":
			depth += 1
			if depth == 1:
				continue
		elif char == ")":
			depth -= 1
			if depth == 0:
				break
		if depth == 1 and char == ",":
			arguments.append("".join(current).strip())
			current = []
		elif depth >= 1:
			current.append(char)
	arguments.append("".join(current).strip())
	return [argument for argument in arguments if argument]


def names_its_skips(attribute: str) -> bool:
	return any(SKIP_DIRECTIVE.match(argument) for argument in top_level_arguments(attribute))


# Extractors whose value is whatever the caller sent, and the request itself:
# its headers carry cookies and authorization, its URI the path and query.
# `State`, `Extension`, `MatchedPath` (the route template) and the auth
# extractors (`SubjectId`, `Operator`) are the server's own.
CONTENT_EXTRACTORS = {
	"Json",
	"Form",
	"Query",
	"Path",
	"Bytes",
	"Multipart",
	"HeaderMap",
	"Uri",
	"OriginalUri",
	"Request",
	"Parts",
	"CookieJar",
	"TypedHeader",
	"Host",
	"RawQuery",
	"RawForm",
}
FN_NAME = re.compile(r"\bfn\s+\w+")
PATH_PREFIX = re.compile(r"(?:\w+::)+")
SINGLE_COLON = re.compile(r"(?<!:):(?!:)")


def parameters(source: str, start: int) -> list[str]:
	"""The parameters, as written, of the first `fn` at or after `start`."""
	named = FN_NAME.search(source, start)
	if not named:
		return []
	index, angle = named.end(), 0
	while index < len(source):  # generics sit between the name and the list
		char = source[index]
		if char == "<":
			angle += 1
		elif char == ">" and source[index - 1] != "-":
			angle -= 1
		elif char == "(" and angle == 0:
			break
		index += 1
	found, current, depth = [], [], 0
	for char in source[index + 1 :]:
		if char in "([{<":
			depth += 1
		elif char in ")]}>" and not (char == ">" and current and current[-1] == "-"):
			if depth == 0:
				break
			depth -= 1
		if char == "," and depth == 0:
			found.append("".join(current).strip())
			current = []
		else:
			current.append(char)
	found.append("".join(current).strip())
	return [parameter for parameter in found if parameter]


def unskipped_content(attribute: str, source: str, attribute_end: int) -> list[str]:
	"""Parameters carrying request content that `attribute` still records."""
	arguments = top_level_arguments(attribute)
	if any(argument == "skip_all" for argument in arguments):
		return []
	skipped: set[str] = set()
	for argument in arguments:
		if re.match(r"^skip\s*\(", argument):
			skipped |= set(re.findall(r"[A-Za-z_]\w*", argument[argument.index("(") + 1 :]))
	recorded = []
	for parameter in parameters(source, attribute_end):
		halves = SINGLE_COLON.split(parameter, maxsplit=1)
		if len(halves) != 2:
			continue  # `self`
		pattern, kind = halves
		extractor = re.match(r"\s*&?\s*(?:\w+::)*(\w+)", kind)
		if not extractor or extractor.group(1) not in CONTENT_EXTRACTORS:
			continue
		bindings = set(re.findall(r"\b[a-z_]\w*\b", PATH_PREFIX.sub("", pattern))) - {"mut", "ref"}
		if not bindings & skipped:
			recorded.append(" ".join(parameter.split()))
	return recorded


def attribute_text(source: str, start: int) -> str:
	"""The attribute beginning at `start` (its `#`), through its matching `]`."""
	depth = 0
	for index in range(start + 1, len(source)):
		char = source[index]
		if char == "[":
			depth += 1
		elif char == "]":
			depth -= 1
			if depth == 0:
				return source[start : index + 1]
	return source[start:]


def offenders() -> list[str]:
	found = []
	for root in RUST_ROOTS:
		for path in sorted(root.rglob("*.rs")):
			if "target" in path.relative_to(REPO_ROOT).parts:
				continue
			source = path.read_text(encoding="utf-8")
			for match in ATTRIBUTE.finditer(source):
				attribute = attribute_text(source, match.start())
				line = source.count("\n", 0, match.start()) + 1
				where = f"{path.relative_to(REPO_ROOT)}:{line}"
				if not names_its_skips(attribute):
					found.append(f"{where}: {' '.join(attribute.split())}")
					continue
				for parameter in unskipped_content(attribute, source, match.start() + len(attribute)):
					found.append(f"{where}: records request content `{parameter}`; skip_all, or name it in skip(...)")
	return found


COMPLIANT = [
	"#[instrument(skip_all)]",
	'#[instrument(name = "health", skip_all)]',
	"#[instrument(skip(state, body))]",
	"#[tracing::instrument(skip (state))]",
	'#[instrument(\n\tname = "x",\n\tskip_all,\n\tfields(otel.kind = "server")\n)]',
]
NONCOMPLIANT = [
	"#[instrument]",
	'#[instrument(name = "health")]',
	'#[instrument(name = "skip")]',
	'#[instrument(fields(note = "skip_all"))]',
	'#[instrument(name = "skip(x)")]',
	"#[instrument(fields(skipped = true))]",
	"#[instrument(fields(skip_all = true))]",
	"#[instrument(fields(skip(x)))]",
	"#[instrument(skip_all_the_things)]",
]


# (attribute + the function it sits on) that record no request content.
CONTENT_CLEAN = [
	"#[instrument(skip_all)]\nasync fn h(State(state): State<S>, Json(p): Json<T>) {}",
	"#[instrument(skip(state, p))]\nasync fn h(State(state): State<S>, Json(p): Json<T>) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, subject: SubjectId) {}",
	"#[instrument(skip(state, q))]\nasync fn h(State(state): State<S>, axum::extract::Query(q): axum::extract::Query<Q>) {}",
	'#[instrument(name = "x", skip_all, fields(id = %id))]\npub async fn h(Path(id): Path<i64>) {}',
	"#[instrument(skip(db))]\nasync fn h<T: Fn(u8) -> u8>(db: &Pool, f: T) {}",
	"#[instrument(skip(state, headers))]\nasync fn h(State(state): State<S>, headers: HeaderMap) {}",
	"#[instrument(skip_all)]\nasync fn h(uri: Uri, axum::extract::Request { .. }: axum::extract::Request) {}",
	'#[instrument(skip(state), fields(route = %matched.as_str()))]\nasync fn h(State(state): State<S>, matched: MatchedPath) {}',
]
# ...and ones that still record it.
CONTENT_RECORDED = [
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, Json(p): Json<T>) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, Path(id): Path<i64>) {}",
	"#[instrument(skip(state))]\nasync fn h(axum::extract::Query(q): axum::extract::Query<Q>) {}",
	"#[instrument(skip(state, other))]\nasync fn h(Json(p): Json<T>) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, body: Bytes) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, mut form: axum::Form<F>) {}",
	"#[tracing::instrument(skip(state))]\npub async fn h<T>(State(state): State<S>, Json(p): Json<T>) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, headers: HeaderMap) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, uri: Uri) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, OriginalUri(uri): OriginalUri) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, req: axum::extract::Request) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, jar: axum_extra::extract::CookieJar) {}",
	"#[instrument(skip(state))]\nasync fn h(State(state): State<S>, TypedHeader(auth): TypedHeader<Authorization<Bearer>>) {}",
	"#[instrument(skip(state, uri))]\nasync fn h(State(state): State<S>, headers: HeaderMap, uri: Uri) {}",
]


def content_recorded(case: str) -> list[str]:
	match = ATTRIBUTE.search(case)
	attribute = attribute_text(case, match.start())
	return unskipped_content(attribute, case, match.start() + len(attribute))


def self_test() -> int:
	failures = [f"not compliant: {case!r}" for case in COMPLIANT if not names_its_skips(case)]
	failures += [f"compliant: {case!r}" for case in NONCOMPLIANT if names_its_skips(case)]
	failures += [f"records content, should not: {case!r}" for case in CONTENT_CLEAN if content_recorded(case)]
	failures += [f"content not caught: {case!r}" for case in CONTENT_RECORDED if not content_recorded(case)]
	if failures:
		print("::error::check_instrument_skip.py's own rule tests failed:")
		for failure in failures:
			print(f"  {failure}")
		return 1
	print(
		f"check_instrument_skip.py rule tests: {len(COMPLIANT)} compliant, {len(NONCOMPLIANT)} not; "
		f"{len(CONTENT_CLEAN)} record no request content, {len(CONTENT_RECORDED)} do, as expected"
	)
	return 0


def main() -> int:
	if "--self-test" in sys.argv[1:]:
		return self_test()
	found = offenders()
	if not found:
		print("every #[instrument] names what it skips, and none records request content")
		return 0
	print("::error::#[instrument] without skip(...) or skip_all, or one that still records request content:")
	for offender in found:
		print(f"  {offender}")
	return 1


if __name__ == "__main__":
	sys.exit(main())
