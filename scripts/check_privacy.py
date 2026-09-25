#!/usr/bin/env python3
"""Fingerprinting headers are read in one place: file_host's `net` module.

docs/identity.md, "Privacy invariants": the server keeps no record that could
single a person out, and the request headers that exist mostly to do that —
`User-Agent`, and the forwarded-address family (`X-Forwarded-For`,
`X-Real-IP`, `Forwarded`, and the CDN variants) — are exactly such records.
The one legitimate reader is `apps/servers/file_host/src/net.rs`, which turns
a forwarded hop into a keyed, daily-rotating `PeerKey` before anything else
sees it. `X-Client-ID` is on the list for the opposite reason: it is a client
asserting an identity, and #372 removed the code that believed it.

So this fails on any *read* of those headers — `.get(...)`, `.get_all(...)`,
`.contains_key(...)`, `.remove(...)` with the name as a string literal or as
an `http::header` constant — in Rust source under `apps/` or `crates/`,
outside the allowlist below. Writes are fine (tests build requests carrying
these headers to prove they are ignored), so `.insert(...)` is not matched.

The clippy half of the same boundary — `axum::extract::ConnectInfo`, the raw
peer address — is a `disallowed-types` entry in clippy.toml, enforced by
lint.yml's clippy ratchet. This half is lexical because clippy's disallowed-*
lints match paths, never arguments, and `HeaderMap::get` is fine with most
arguments.

Deliberately lexical, like scripts/check_instrument_skip.py. `--self-test`
runs the rule against known-good and known-bad snippets (this repo's
equivalent of ESLint's RuleTester) and runs in lint.yml alongside the real
scan, so a regex edit that stops matching fails CI instead of passing
silently.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_ROOTS = [REPO_ROOT / "apps", REPO_ROOT / "crates"]

# Every exemption names why. Adding one is a design decision, not a fix.
ALLOWED = {
	"apps/servers/file_host/src/net.rs": "the sanctioned reader: turns a forwarded hop into a PeerKey",
}

HEADER_NAMES = ["user-agent", "x-forwarded-for", "x-real-ip", "forwarded", "cf-connecting-ip", "true-client-ip", "x-client-id"]
HEADER_CONSTANTS = ["USER_AGENT", "FORWARDED"]

READ = re.compile(
	r"\.(?:get|get_all|contains_key|remove)\(\s*"
	r"(?:"
	r'"(?P<literal>' + "|".join(re.escape(name) for name in HEADER_NAMES) + r')"'
	r"|(?:[A-Za-z_][A-Za-z0-9_]*::)*(?P<constant>" + "|".join(HEADER_CONSTANTS) + r")\b"
	r")",
	re.IGNORECASE,
)


def reads(source: str) -> list[tuple[int, str]]:
	"""(line, header) for every fingerprinting-header read in `source`."""
	found = []
	for match in READ.finditer(source):
		header = match.group("literal") or match.group("constant")
		found.append((source.count("\n", 0, match.start()) + 1, header))
	return found


SHOULD_FLAG = [
	'headers.get("user-agent")',
	'headers.get( "User-Agent" )',
	'req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok())',
	'headers.get_all("x-real-ip")',
	'headers.contains_key("x-client-id")',
	"headers.get(USER_AGENT)",
	"headers.get(header::USER_AGENT)",
	"headers.get(axum::http::header::FORWARDED)",
	'headers.remove("cf-connecting-ip")',
]
SHOULD_PASS = [
	'headers.insert("x-forwarded-for", "10.0.0.9".parse().unwrap())',
	'headers.get("x-probe-source")',
	'headers.get("content-type")',
	"headers.get(CONTENT_TYPE)",
	'headers.get("x-forwarded-for-ish")',
	'let user_agent = "user-agent";',
]


def self_test() -> int:
	failures = [f"not flagged: {snippet}" for snippet in SHOULD_FLAG if not reads(snippet)]
	failures += [f"flagged: {snippet}" for snippet in SHOULD_PASS if reads(snippet)]
	if failures:
		print("::error::check_privacy.py's own rule tests failed:")
		for failure in failures:
			print(f"  {failure}")
		return 1
	print(f"check_privacy.py rule tests: {len(SHOULD_FLAG)} flagged, {len(SHOULD_PASS)} passed, as expected")
	return 0


def scan() -> int:
	offenders = []
	for root in RUST_ROOTS:
		for path in sorted(root.rglob("*.rs")):
			relative = path.relative_to(REPO_ROOT)
			if "target" in relative.parts or relative.as_posix() in ALLOWED:
				continue
			for line, header in reads(path.read_text(encoding="utf-8")):
				offenders.append(f"{relative}:{line}: reads {header}")
	if offenders:
		print("::error::fingerprinting headers read outside file_host's net module (docs/identity.md):")
		for offender in offenders:
			print(f"  {offender}")
		return 1
	print("no fingerprinting headers read outside the allowlist")
	return 0


if __name__ == "__main__":
	sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
