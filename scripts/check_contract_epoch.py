#!/usr/bin/env python3
"""The contract epoch only rises, and rises with every route change.

`apps/servers/file_host/src/contract.rs` holds `CONTRACT_EPOCH`,
`OLDEST_SUPPORTED` and `ROUTE_FINGERPRINT`. Its own test holds the
fingerprint to the routes the build serves, so a route change cannot land
without the fingerprint moving. This check holds the epoch to the base
branch's, so the fingerprint cannot move without the epoch rising:

- `CONTRACT_EPOCH` is never below the base's, and is above it whenever
  `ROUTE_FINGERPRINT` differs;
- `OLDEST_SUPPORTED` is never below the base's, and never above
  `CONTRACT_EPOCH`.

A client's build bundles the epoch and home pulls a newer image only when a
client needs one above its own (`handlers::compat`), so an epoch that stood
still across a route change would leave a client that needs the change
served by a build without it.

    python3 scripts/check_contract_epoch.py --base-file base_contract.rs
    python3 scripts/check_contract_epoch.py --self-test

With no `--base-file` (or an empty one: the base branch has no contract.rs
yet) only the head's own rule is checked. Stdlib only.
"""

from __future__ import annotations

import argparse
import re
import sys
import unittest
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CONTRACT = REPO_ROOT / "apps" / "servers" / "file_host" / "src" / "contract.rs"


@dataclass(frozen=True)
class Contract:
    epoch: int
    oldest: int
    fingerprint: str


def parse(text: str) -> Contract:
    def find(pattern: str) -> str:
        found = re.search(pattern, text, re.MULTILINE)
        if found is None:
            raise ValueError(f"contract.rs: no match for {pattern!r}")
        return found.group(1)

    return Contract(
        int(find(r"^pub const CONTRACT_EPOCH: u32 = (\d+);")),
        int(find(r"^pub const OLDEST_SUPPORTED: u32 = (\d+);")),
        find(r'^pub const ROUTE_FINGERPRINT: &str = "([^"]*)";'),
    )


def problems(head: Contract, base: Contract | None) -> list[str]:
    found = []
    if not 1 <= head.oldest <= head.epoch:
        found.append(f"OLDEST_SUPPORTED ({head.oldest}) must be between 1 and CONTRACT_EPOCH ({head.epoch})")
    if base is None:
        return found
    if head.epoch < base.epoch:
        found.append(f"CONTRACT_EPOCH went down ({base.epoch} -> {head.epoch}): it only rises")
    if head.oldest < base.oldest:
        found.append(f"OLDEST_SUPPORTED went down ({base.oldest} -> {head.oldest}): it only rises")
    if head.fingerprint != base.fingerprint and head.epoch <= base.epoch:
        found.append(
            f"the route surface changed (ROUTE_FINGERPRINT {base.fingerprint} -> {head.fingerprint}) "
            f"but CONTRACT_EPOCH stayed at {head.epoch}: raise it to {base.epoch + 1} "
            "(apps/servers/file_host/src/contract.rs, \"When to bump\")"
        )
    return found


class Tests(unittest.TestCase):
    SOURCE = 'pub const CONTRACT_EPOCH: u32 = {e};\npub const OLDEST_SUPPORTED: u32 = {o};\npub const ROUTE_FINGERPRINT: &str = "{f}";\n'

    def contract(self, e: int, o: int, f: str) -> Contract:
        return parse(self.SOURCE.format(e=e, o=o, f=f))

    def test_reads_the_three_constants(self) -> None:
        self.assertEqual(self.contract(3, 2, "ab"), Contract(3, 2, "ab"))

    def test_a_route_change_needs_the_epoch_to_rise(self) -> None:
        self.assertTrue(problems(self.contract(3, 1, "new"), self.contract(3, 1, "old")))
        self.assertEqual(problems(self.contract(4, 1, "new"), self.contract(3, 1, "old")), [])

    def test_a_bump_by_hand_with_no_route_change_is_fine(self) -> None:
        self.assertEqual(problems(self.contract(4, 1, "same"), self.contract(3, 1, "same")), [])

    def test_nothing_goes_down(self) -> None:
        self.assertTrue(problems(self.contract(2, 1, "x"), self.contract(3, 1, "x")))
        self.assertTrue(problems(self.contract(3, 1, "x"), self.contract(3, 2, "x")))

    def test_oldest_supported_stays_within_the_epoch(self) -> None:
        self.assertTrue(problems(self.contract(2, 3, "x"), None))
        self.assertTrue(problems(self.contract(2, 0, "x"), None))
        self.assertEqual(problems(self.contract(2, 2, "x"), None), [])

    def test_the_checked_in_file_parses(self) -> None:
        parse(CONTRACT.read_text())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--base-file", type=Path, help="the base branch's contract.rs (empty or missing: none yet)")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=1).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
        return 0 if result.wasSuccessful() else 1
    head = parse(CONTRACT.read_text())
    base = None
    if args.base_file is not None and args.base_file.exists() and args.base_file.read_text().strip():
        base = parse(args.base_file.read_text())
    found = problems(head, base)
    for problem in found:
        print(f"::error file=apps/servers/file_host/src/contract.rs::{problem}")
    if not found:
        print(f"contract epoch {head.epoch} (oldest supported {head.oldest})" + (f", base {base.epoch}" if base else ", no base contract yet"))
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
