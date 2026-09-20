#!/usr/bin/env bash
# Emits hostdir_usage_bytes{target="..."} and
# hostdir_usage_last_run_timestamp_seconds for node_exporter's textfile
# collector — see infra/grafana/dashboards/lib/forensic-panels.libsonnet's
# hostDirDiskUsage panel. cadvisor's container_fs_usage_bytes only sees a
# *running* container's own writable layer: it never sees the cargo
# registry/git caches or the workspace target/ dir on the host, and never
# the parts of Docker's own data-root (build cache, dangling images, unused
# volumes) that no running container's own usage would show either. This
# script closes that gap with a plain `du`.
#
# Run by the disk-usage-exporter sidecar in infra/compose/monitoring.yml, on
# a loop — this script itself is single-shot and idempotent, so it's also
# safe to invoke by hand, or from a systemd timer, if this ever moves off
# docker-compose.
set -euo pipefail

OUTPUT_DIR="${TEXTFILE_COLLECTOR_DIR:-/textfile-collector}"
OUTPUT_FILE="$OUTPUT_DIR/disk_usage.prom"
TMP_FILE="$OUTPUT_DIR/.disk_usage.prom.tmp.$$"

# Covers every early-exit path, not just the happy one — after a successful
# `mv` below this is already gone and `rm -f` is a no-op, but a `du` failure
# (see dir_size_bytes) used to abort mid-script under `set -e` and leave
# this behind forever.
trap 'rm -f "$TMP_FILE"' EXIT

# name:path pairs — bind-mounted read-only by the disk-usage-exporter
# service in infra/compose/monitoring.yml.
TARGETS=(
	"cargo_registry:/mnt/cargo/registry"
	"cargo_git:/mnt/cargo/git"
	"cargo_target:/mnt/workspace-target"
	"docker_data_root:/mnt/docker"
)

dir_size_bytes() {
	local path="$1"
	local size
	if [ -d "$path" ]; then
		# `-B1` (block-size 1, no `--apparent-size`), not `-b` — `-b` is GNU
		# du's shorthand for `--apparent-size --block-size=1`, which reports
		# a sparse file's logical length rather than the disk blocks it
		# actually occupies. A disk-usage panel should track the same
		# "space consumed" a filesystem would run out of, not a number that
		# can overstate it.
		#
		# The `||` guards the *assignment*, not a bare pipeline followed by
		# its own `echo 0` — `du` can observe a file vanish mid-traversal
		# (Docker actively writing to docker_data_root is the realistic
		# case here) and exit nonzero under pipefail despite awk already
		# having printed a usable total; a fallback appended as a separate
		# statement would land *after* that already-emitted line instead of
		# replacing it, producing two lines for one metric and corrupting
		# the whole textfile. Capturing into a variable first means a
		# failed assignment simply reassigns `size` to 0 — nothing printed
		# by the failed attempt survives into the final value. The blank
		# check covers `du` succeeding but printing nothing, same reason.
		size="$(du -s -B1 "$path" 2>/dev/null | awk '{print $1}')" || size=0
		[ -n "$size" ] || size=0
	else
		size=0
	fi
	echo "$size"
}

# Written to a per-run tmp file and renamed into place — a `mv` on the same
# filesystem is atomic, so node_exporter's textfile collector never reads a
# half-written scrape.
{
	echo "# HELP hostdir_usage_bytes Bytes used by a tracked host directory (du -s -B1), refreshed periodically by scripts/disk-usage-textfile.sh."
	echo "# TYPE hostdir_usage_bytes gauge"
	for entry in "${TARGETS[@]}"; do
		name="${entry%%:*}"
		path="${entry#*:}"
		size="$(dir_size_bytes "$path")"
		echo "hostdir_usage_bytes{target=\"${name}\"} ${size}"
	done
	echo "# HELP hostdir_usage_last_run_timestamp_seconds Unix time this script last completed a full pass."
	echo "# TYPE hostdir_usage_last_run_timestamp_seconds gauge"
	echo "hostdir_usage_last_run_timestamp_seconds $(date +%s)"
	# Read from the same env var the compose service's own sleep loop
	# uses (infra/compose/monitoring.yml), not hardcoded here too — so
	# hostDirUsageStaleness can judge staleness relative to whatever
	# interval is actually configured instead of a threshold hardcoded
	# against this default, which an operator raising the interval to
	# reduce du's traversal cost would silently invalidate.
	echo "# HELP hostdir_usage_scan_interval_seconds The configured interval between scans (DISK_USAGE_SCAN_INTERVAL)."
	echo "# TYPE hostdir_usage_scan_interval_seconds gauge"
	echo "hostdir_usage_scan_interval_seconds ${DISK_USAGE_SCAN_INTERVAL:-300}"
} >"$TMP_FILE"

mv "$TMP_FILE" "$OUTPUT_FILE"
