#!/usr/bin/env bash
# Thin looping wrapper around disk-usage-textfile.sh — kept separate so
# that script stays single-shot/idempotent (its own header comment's own
# contract, and safe to invoke by hand or from a systemd timer).
#
# Lives in its own file rather than inline in
# infra/compose/monitoring.yml's `command:` specifically so
# $DISK_USAGE_SCAN_INTERVAL is resolved by *this script's own* bash at
# container runtime. Docker Compose's own variable interpolation runs over
# compose YAML text — inline `command:` strings included — before the
# container ever starts, so a bare `$VAR` referenced there is substituted
# by Compose itself (from its invoking shell's environment, not the
# container's) unless escaped, and `$$` for that escape reads as a
# literal shell PID substitution if it round-trips into the container
# unresolved (verified directly: `docker compose config` preserved
# `$$DISK_USAGE_SCAN_INTERVAL` as two literal dollar signs rather than
# collapsing to one). A bind-mounted script's own contents are never
# touched by Compose's interpolation pass at all — this sidesteps the
# question rather than getting it right.
set -euo pipefail

while true; do
	bash /usr/local/bin/disk-usage-textfile.sh
	sleep "${DISK_USAGE_SCAN_INTERVAL:-300}"
done
